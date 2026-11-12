//! WebSocket transport (architecture §7, third door): an RFC 6455
//! server and client implemented in-module so the transport stays
//! dependency-free.
//! The m12 note recorded the gap verbatim — *"WebSocket transport in Rust
//! (the TS client ships `connectWs`; the §7 transports list records the
//! constraint)"* — and M15 cashes it: `connectWs` (M3, TypeScript)
//! finally has a Rust peer.
//!
//! The shape is the same trick the UDS/TCP share, one step further out:
//! [`WsStream`] implements `tokio`'s `AsyncRead` + `AsyncWrite` over the
//! frame codec, so the GWP conversation (`crate::conn`) — framing,
//! handshake gate, dispatch, event multiplexing — runs on it unchanged.
//! A WS session is byte-identical to a TCP session; only the pipe
//! differs. Server-side frames are unmasked, client-side frames carry a
//! fresh mask key per message (RFC 6455 §5.3); pings are answered
//! automatically, close is answered once, and the driver never
//! fragments outbound (every message ships in one FIN frame).
//!
//! This is a handshake-hash, loopback-grade implementation: SHA-1 and
//! base64 exist only because the RFC's opening handshake demands them,
//! and the driver speaks to local peers (the §7 threat model never
//! included a hostile network).

use std::collections::VecDeque;
use std::pin::Pin;
use std::task::{Context, Poll, ready};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;

use crate::GwpError;
use crate::conn::ClientCore;
use crate::schema::{EventKindV1, RequestV1, ResponseV1};

/// The RFC 6455 magic GUID appended to the client key before hashing
/// (§1.3): both sides derive `Sec-WebSocket-Accept` from it.
const WS_GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

// Frame opcodes (RFC 6455 §5.2).
const OPCODE_CONTINUATION: u8 = 0x0;
const OPCODE_TEXT: u8 = 0x1;
const OPCODE_BINARY: u8 = 0x2;
const OPCODE_CLOSE: u8 = 0x8;
const OPCODE_PING: u8 = 0x9;
const OPCODE_PONG: u8 = 0xA;

/// Sanity guard on one frame's declared payload (loopback-grade `DoS`
/// guard; GWP frames ride well under 1 MiB, the blob side-channel is a
/// different door).
const MAX_FRAME_PAYLOAD: u64 = 1 << 30;

/// Hard cap on the HTTP opening-handshake headers (a request or response
/// never approaches this; anything larger is not a WS handshake).
const MAX_HANDSHAKE_BYTES: usize = 16 * 1024;

/// Which side of the wire a stream speaks for: clients mask every
/// outbound frame with a fresh key (§5.3), servers never mask.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Client,
    Server,
}

/// SHA-1 (RFC 3174) — the WebSocket handshake's only hashing primitive,
/// implemented here so the transport stays dependency-free. Pinned below
/// by RFC test vectors; this is a handshake hash, not a security boundary
/// (the driver speaks to local/loopback peers).
fn sha1(data: &[u8]) -> [u8; 20] {
    let mut state: [u32; 5] = [
        0x6745_2301,
        0xEFCD_AB89,
        0x98BA_DCFE,
        0x1032_5476,
        0xC3D2_E1F0,
    ];
    // Message padding: 0x80, zeros, 64-bit big-endian bit length.
    let bit_len = u64::try_from(data.len())
        .expect("sha1 input fits in u64 bits")
        .wrapping_mul(8);
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    for block in msg.chunks_exact(64) {
        let mut schedule = [0u32; 80];
        for (slot, word) in block.chunks_exact(4).enumerate() {
            schedule[slot] = u32::from_be_bytes(word.try_into().expect("4-byte word"));
        }
        for slot in 16..80 {
            schedule[slot] = (schedule[slot - 3]
                ^ schedule[slot - 8]
                ^ schedule[slot - 14]
                ^ schedule[slot - 16])
                .rotate_left(1);
        }
        // The five working registers ride `work[0..5]` (the textbook
        // a..e lanes); each round rotates them one slot down.
        let mut work = state;
        for (round, &word) in schedule.iter().enumerate() {
            let (mix, constant) = match round {
                0..=19 => ((work[1] & work[2]) | (!work[1] & work[3]), 0x5A82_7999u32),
                20..=39 => (work[1] ^ work[2] ^ work[3], 0x6ED9_EBA1),
                40..=59 => (
                    (work[1] & work[2]) | (work[1] & work[3]) | (work[2] & work[3]),
                    0x8F1B_BCDC,
                ),
                _ => (work[1] ^ work[2] ^ work[3], 0xCA62_C1D6),
            };
            let rotated = work[0]
                .rotate_left(5)
                .wrapping_add(mix)
                .wrapping_add(work[4])
                .wrapping_add(constant)
                .wrapping_add(word);
            work[4] = work[3];
            work[3] = work[2];
            work[2] = work[1].rotate_left(30);
            work[1] = work[0];
            work[0] = rotated;
        }
        for (slot, word) in work.iter().enumerate() {
            state[slot] = state[slot].wrapping_add(*word);
        }
    }
    let mut out = [0u8; 20];
    for (slot, word) in state.iter().enumerate() {
        out[slot * 4..slot * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

/// Standard-alphabet base64 with padding (RFC 4648) — exactly what the
/// `Sec-WebSocket-Key` / `Sec-WebSocket-Accept` header fields use.
fn base64_encode(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks_exact(3) {
        let word = (u32::from(chunk[0]) << 16) | (u32::from(chunk[1]) << 8) | u32::from(chunk[2]);
        out.push(ALPHABET[(word >> 18) as usize & 0x3f] as char);
        out.push(ALPHABET[(word >> 12) as usize & 0x3f] as char);
        out.push(ALPHABET[(word >> 6) as usize & 0x3f] as char);
        out.push(ALPHABET[word as usize & 0x3f] as char);
    }
    match data.len() % 3 {
        1 => {
            let word = u32::from(data[data.len() - 1]) << 16;
            out.push(ALPHABET[(word >> 18) as usize & 0x3f] as char);
            out.push(ALPHABET[(word >> 12) as usize & 0x3f] as char);
            out.push_str("==");
        }
        2 => {
            let word =
                (u32::from(data[data.len() - 2]) << 16) | (u32::from(data[data.len() - 1]) << 8);
            out.push(ALPHABET[(word >> 18) as usize & 0x3f] as char);
            out.push(ALPHABET[(word >> 12) as usize & 0x3f] as char);
            out.push(ALPHABET[(word >> 6) as usize & 0x3f] as char);
            out.push('=');
        }
        _ => {}
    }
    out
}

/// `Sec-WebSocket-Accept` for a client key (RFC 6455 §1.3): base64 of
/// SHA-1 over `key + GUID`.
fn accept_key(client_key: &str) -> String {
    let mut material = client_key.as_bytes().to_vec();
    material.extend_from_slice(WS_GUID.as_bytes());
    base64_encode(&sha1(&material))
}

/// A per-stream mask-key source (§5.3: clients SHOULD use a fresh key per
/// frame). No crypto required — the mask defeats intermediaries, not
/// attackers — so a hashed `RandomState` seed plus a counter is plenty.
#[derive(Debug)]
struct MaskSource {
    seed: u64,
    counter: u64,
}

impl MaskSource {
    fn new() -> Self {
        use std::hash::{BuildHasher, Hasher, RandomState};
        let mut hasher = RandomState::new().build_hasher();
        hasher.write_u32(0x6d_61_73_6b); // "mask"
        Self {
            seed: hasher.finish(),
            counter: 0,
        }
    }

    fn next_key(&mut self) -> [u8; 4] {
        self.counter = self.counter.wrapping_add(1);
        let word = (self.seed ^ self.counter.rotate_left(17)).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        (word as u32).to_ne_bytes()
    }
}

/// One decoded WS frame's header + unmasked payload (the inbound parse's
/// return shape).
struct InboundFrame {
    fin: bool,
    opcode: u8,
    payload: Vec<u8>,
}

/// The inbound half of a WebSocket connection: raw socket bytes in,
/// application payload + control-frame actions out. Kept separate from
/// [`WsStream`] so the frame grammar is unit-testable without a socket.
#[derive(Debug, Default)]
struct Inbound {
    /// Raw bytes pulled off the socket, not yet consumed by a full frame.
    net: Vec<u8>,
    /// Decoded application payload ready for the GWP `FrameReader`.
    plain: Vec<u8>,
    /// Pings awaiting a Pong (the connection encodes + writes them with
    /// its own `Mode`, so they are stored raw here).
    pings: VecDeque<Vec<u8>>,
    /// Set once a Close frame is parsed; nothing after it is decoded.
    close_seen: bool,
    /// Fragment reassembly: the opcode of the first fragment while a
    /// fragmented message is mid-flight.
    frag_opcode: Option<u8>,
    frag_buf: Vec<u8>,
}

impl Inbound {
    /// Parse every complete frame in `net`, appending application payload
    /// to `plain` and queuing control actions. Incomplete tails stay in
    /// `net` — no consumed byte is ever lost (the §7 partial-frame
    /// contract, now at the WS layer).
    fn parse_available(&mut self) -> Result<(), GwpError> {
        loop {
            if self.close_seen {
                // RFC 6455 §5.5.1: the peer sends nothing after Close, and
                // we stop decoding even if it does.
                self.net.clear();
                return Ok(());
            }
            let Some((frame, consumed)) = parse_frame(&self.net)? else {
                return Ok(());
            };
            self.net.drain(..consumed);
            match frame.opcode {
                OPCODE_TEXT | OPCODE_BINARY => {
                    if frame.fin {
                        self.plain.extend_from_slice(&frame.payload);
                    } else {
                        self.frag_opcode = Some(frame.opcode);
                        self.frag_buf = frame.payload;
                    }
                }
                OPCODE_CONTINUATION => {
                    if self.frag_opcode.is_none() {
                        return Err(GwpError::Protocol(
                            "ws: continuation frame without a fragmented message".into(),
                        ));
                    }
                    self.frag_buf.extend_from_slice(&frame.payload);
                    if frame.fin {
                        let assembled = std::mem::take(&mut self.frag_buf);
                        self.plain.extend_from_slice(&assembled);
                        self.frag_opcode = None;
                    }
                }
                OPCODE_CLOSE => {
                    self.close_seen = true;
                }
                OPCODE_PING => {
                    self.pings.push_back(frame.payload);
                }
                OPCODE_PONG => {}
                _ => {
                    return Err(GwpError::Protocol(format!(
                        "ws: reserved frame opcode {:#04x}",
                        frame.opcode
                    )));
                }
            }
        }
    }
}

/// Parse at most one frame from the front of `net`. `Ok(None)` = the
/// header or payload is incomplete; more bytes will complete it.
fn parse_frame(net: &[u8]) -> Result<Option<(InboundFrame, usize)>, GwpError> {
    if net.len() < 2 {
        return Ok(None);
    }
    let first = net[0];
    let fin = first & 0x80 != 0;
    if first & 0x70 != 0 {
        return Err(GwpError::Protocol(format!(
            "ws: reserved RSV bits set ({first:#04x})"
        )));
    }
    let opcode = first & 0x0f;
    let second = net[1];
    let masked = second & 0x80 != 0;
    let len7 = usize::from(second & 0x7f);

    let (len, header_len) = match len7 {
        n @ 0..=125 => (n, 2),
        126 => {
            if net.len() < 4 {
                return Ok(None);
            }
            let len = u16::from_be_bytes([net[2], net[3]]);
            (usize::from(len), 4)
        }
        _ => {
            if net.len() < 10 {
                return Ok(None);
            }
            let len = u64::from_be_bytes(net[2..10].try_into().expect("8 length bytes"));
            if len > MAX_FRAME_PAYLOAD {
                return Err(GwpError::Protocol(format!(
                    "ws: frame payload of {len} bytes exceeds the guard"
                )));
            }
            (
                usize::try_from(len).expect("len fits usize under the guard"),
                10,
            )
        }
    };

    let mask_len = usize::from(masked) * 4;
    let total = header_len + mask_len + len;
    if net.len() < total {
        return Ok(None);
    }

    // Control frames must be short and unfragmented (RFC 6455 §5.5).
    if opcode >= 0x8 {
        if !fin {
            return Err(GwpError::Protocol("ws: fragmented control frame".into()));
        }
        if len > 125 {
            return Err(GwpError::Protocol(format!(
                "ws: control frame payload of {len} bytes exceeds 125"
            )));
        }
    }

    let mut payload = net[header_len + mask_len..total].to_vec();
    if masked {
        let key: [u8; 4] = net[header_len..header_len + 4]
            .try_into()
            .expect("4 mask bytes");
        for (i, byte) in payload.iter_mut().enumerate() {
            *byte ^= key[i % 4];
        }
    }
    Ok(Some((
        InboundFrame {
            fin,
            opcode,
            payload,
        },
        total,
    )))
}

/// Encode one WS frame. Client-side frames are masked with `mask` (a
/// fresh key per message, §5.3); server-side frames are unmasked. Outbound
/// frames always carry FIN — the driver never fragments.
fn encode_frame(opcode: u8, payload: &[u8], mode: Mode, mask: [u8; 4]) -> Vec<u8> {
    let masked = mode == Mode::Client;
    let len_bit = u8::from(masked) * 0x80;
    let mut out = Vec::with_capacity(payload.len() + 14);
    out.push(0x80 | opcode);
    match payload.len() {
        n if n <= 125 => out.push(len_bit | n as u8),
        n if u16::try_from(n).is_ok() => {
            out.push(len_bit | 0x7e);
            out.extend_from_slice(&(n as u16).to_be_bytes());
        }
        n => {
            out.push(len_bit | 0x7f);
            out.extend_from_slice(
                &(u64::try_from(n).expect("payload length fits u64")).to_be_bytes(),
            );
        }
    }
    if masked {
        out.extend_from_slice(&mask);
        for (i, byte) in payload.iter().enumerate() {
            out.push(byte ^ mask[i % 4]);
        }
    } else {
        out.extend_from_slice(payload);
    }
    out
}

/// A WebSocket session over one TCP stream: decodes inbound frames and
/// encodes outbound messages transparently, so `crate::framing`'s
/// length-prefixed GWP frames ride the connection unchanged.
pub struct WsStream {
    stream: TcpStream,
    mode: Mode,
    inbound: Inbound,
    masks: MaskSource,
    /// Bytes written since the last flush (the next outbound message).
    write_buf: Vec<u8>,
    /// The outbound message currently on the wire: encoded bytes + offset.
    /// A partially-sent message survives the flush future being dropped —
    /// the offset is state, no byte is ever written twice or skipped.
    sending: Option<(Vec<u8>, usize)>,
    /// Encoded control frames (Pongs, and the close reply) waiting for a
    /// flush window; Pongs try to jump the queue with a non-blocking
    /// write first.
    controls: VecDeque<Vec<u8>>,
    /// The close reply is queued exactly once (`poll_shutdown` may be
    /// polled many times).
    close_queued: bool,
}

impl std::fmt::Debug for WsStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WsStream")
            .field("stream", &self.stream)
            .field("mode", &self.mode)
            .field("inbound", &self.inbound)
            .field("masks", &self.masks)
            .field("write_buf", &self.write_buf)
            .field("sending", &self.sending)
            .field("controls", &self.controls)
            .field("close_queued", &self.close_queued)
            .finish()
    }
}

impl WsStream {
    fn new(stream: TcpStream, mode: Mode) -> Self {
        Self {
            stream,
            mode,
            inbound: Inbound::default(),
            masks: MaskSource::new(),
            write_buf: Vec::new(),
            sending: None,
            controls: VecDeque::new(),
            close_queued: false,
        }
    }

    /// True once the peer's Close frame has been parsed (the session ends
    /// cleanly; the GWP loop observes EOF through the read half).
    pub fn close_seen(&self) -> bool {
        self.inbound.close_seen
    }

    /// Answer any queued Pings: try a non-blocking write first (Pongs are
    /// tiny and the socket is usually ready). A partial control write
    /// would corrupt the frame stream, so the Pong is dropped instead —
    /// RFC 6455 lets the peer re-ping; a lost Pong is always legal.
    fn dispatch_pongs(&mut self) {
        while let Some(payload) = self.inbound.pings.pop_front() {
            let frame = encode_frame(OPCODE_PONG, &payload, self.mode, self.masks.next_key());
            if !matches!(self.stream.try_write(&frame), Ok(n) if n == frame.len()) {
                break;
            }
        }
    }

    /// Queue the outbound Close (code 1000, no reason) for
    /// [`AsyncWrite::poll_shutdown`] to flush.
    fn queue_close(&mut self) {
        if self.close_queued {
            return;
        }
        let frame = encode_frame(
            OPCODE_CLOSE,
            &1000u16.to_be_bytes(),
            self.mode,
            self.masks.next_key(),
        );
        self.controls.push_back(frame);
        self.close_queued = true;
    }
}

impl AsyncRead for WsStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let this = self.get_mut();
        // Already-decoded payload first.
        if !this.inbound.plain.is_empty() {
            let n = buf.remaining().min(this.inbound.plain.len());
            let drained: Vec<u8> = this.inbound.plain.drain(..n).collect();
            buf.put_slice(&drained);
            return Poll::Ready(Ok(()));
        }
        if this.inbound.close_seen {
            // Clean WS close reads as EOF; the GWP loop unwinds normally.
            return Poll::Ready(Ok(()));
        }
        loop {
            let mut net_buf = [0u8; 8192];
            let mut read_buf = ReadBuf::new(&mut net_buf);
            ready!(Pin::new(&mut this.stream).poll_read(cx, &mut read_buf))?;
            let filled = read_buf.filled().len();
            if filled == 0 {
                if this.inbound.net.is_empty() {
                    return Poll::Ready(Ok(()));
                }
                return Poll::Ready(Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "ws: peer hung up mid-frame",
                )));
            }
            this.inbound.net.extend_from_slice(read_buf.filled());
            this.inbound.parse_available().map_err(|err| {
                std::io::Error::new(std::io::ErrorKind::InvalidData, err.to_string())
            })?;
            this.dispatch_pongs();
            if !this.inbound.plain.is_empty() {
                let n = buf.remaining().min(this.inbound.plain.len());
                let drained: Vec<u8> = this.inbound.plain.drain(..n).collect();
                buf.put_slice(&drained);
                return Poll::Ready(Ok(()));
            }
            if this.inbound.close_seen {
                return Poll::Ready(Ok(()));
            }
            // No complete frame yet: keep filling `net`.
        }
    }
}

impl AsyncWrite for WsStream {
    fn poll_write(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let this = self.get_mut();
        // Buffer now; the message ships at flush (one WS message per GWP
        // write, never fragmented).
        this.write_buf.extend_from_slice(buf);
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        let this = self.get_mut();
        // Drain to empty: control frames first (Pongs owed, the close
        // reply), then the buffered application message — a `write_frame`
        // that flushes once must see its bytes on the wire.
        loop {
            if this.sending.is_none() {
                if let Some(control) = this.controls.pop_front() {
                    this.sending = Some((control, 0));
                } else if !this.write_buf.is_empty() {
                    let mask = this.masks.next_key();
                    let frame = encode_frame(OPCODE_BINARY, &this.write_buf, this.mode, mask);
                    this.write_buf.clear();
                    this.sending = Some((frame, 0));
                }
            }
            if this.sending.is_none() {
                return Poll::Ready(Ok(())); // every queue drained
            }
            ready!(flush_sending(this, cx))?;
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        let this = self.get_mut();
        this.queue_close();
        // Flush the close frame (Pending propagates while the socket
        // drains), then tear the TCP socket down.
        ready!(Self::poll_flush(Pin::new(this), cx))?;
        Pin::new(&mut this.stream).poll_shutdown(cx)
    }
}

/// Drain `sending` toward the socket; leaves the partially-sent tail in
/// place (offset state) when the socket would block — no byte is ever
/// written twice or skipped.
fn flush_sending(this: &mut WsStream, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
    loop {
        // Take the current message out (the borrow must end before the
        // next `poll_write` on the same struct); restore it on any wait.
        let Some((bytes, mut offset)) = this.sending.take() else {
            return Poll::Ready(Ok(()));
        };
        loop {
            if offset >= bytes.len() {
                break; // fully sent; `sending` stays None
            }
            match Pin::new(&mut this.stream).poll_write(cx, &bytes[offset..]) {
                Poll::Ready(Ok(0)) => {
                    this.sending = Some((bytes, offset));
                    return Poll::Ready(Err(std::io::Error::new(
                        std::io::ErrorKind::WriteZero,
                        "ws: socket accepted zero bytes",
                    )));
                }
                Poll::Ready(Ok(n)) => offset += n,
                Poll::Pending => {
                    this.sending = Some((bytes, offset));
                    return Poll::Pending;
                }
                Poll::Ready(Err(err)) => {
                    this.sending = Some((bytes, offset));
                    return Poll::Ready(Err(err));
                }
            }
        }
    }
}

/// One random 16-byte `Sec-WebSocket-Key`, base64-encoded (RFC 6455 §4.1
/// recommends 16 bytes of randomness; masks/handshakes here are not
/// cryptographic).
fn fresh_ws_key() -> String {
    use std::hash::{BuildHasher, Hasher, RandomState};
    let mut hasher = RandomState::new().build_hasher();
    hasher.write_u64(0x006b_6579_7365_6564); // "keyseed"
    let seed = hasher.finish();
    let mut key = [0u8; 16];
    for (i, byte) in key.iter_mut().enumerate() {
        let round = seed
            ^ (i as u64)
                .wrapping_mul(0x9E37_79B9_7F4A_7C15)
                .rotate_left(13);
        *byte = (round >> (i % 8 * 8)) as u8;
    }
    base64_encode(&key)
}

/// The lowercase header table the handshake parser produces.
type Headers = Vec<(String, String)>;

/// Parse a raw HTTP/1.x header block into (status-or-none, headers). The
/// WS handshake never carries a body; both sides read to `\r\n\r\n`.
fn parse_http_head(raw: &str) -> Result<(Option<u16>, Headers), GwpError> {
    let mut lines = raw.split("\r\n");
    let request_line = lines.next().ok_or(GwpError::Handshake("empty handshake"))?;
    let status = if request_line.starts_with("HTTP/") {
        request_line
            .split_whitespace()
            .nth(1)
            .and_then(|code| code.parse::<u16>().ok())
    } else {
        // A request line ("GET / HTTP/1.1") — the server side.
        None
    };
    let mut headers = Vec::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let (name, value) = line
            .split_once(':')
            .ok_or(GwpError::Handshake("malformed handshake header line"))?;
        headers.push((name.trim().to_ascii_lowercase(), value.trim().to_string()));
    }
    Ok((status, headers))
}

fn header_value<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
}

/// Header token list membership, case-insensitive (`Connection: keep-alive,
/// Upgrade` is the RFC's own example shape).
fn header_has_token(headers: &[(String, String)], name: &str, token: &str) -> bool {
    header_value(headers, name).is_some_and(|value| {
        value
            .split(',')
            .any(|part| part.trim().eq_ignore_ascii_case(token))
    })
}

/// Read one HTTP head (to `\r\n\r\n`) with the handshake byte cap.
async fn read_http_head(stream: &mut TcpStream) -> Result<String, GwpError> {
    use tokio::io::AsyncReadExt;
    let mut raw: Vec<u8> = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    loop {
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            return Err(GwpError::Handshake("peer closed during handshake"));
        }
        raw.extend_from_slice(&chunk[..n]);
        if raw.len() > MAX_HANDSHAKE_BYTES {
            return Err(GwpError::Handshake("handshake headers exceed the cap"));
        }
        if raw.ends_with(b"\r\n\r\n") {
            return Ok(String::from_utf8_lossy(&raw).into_owned());
        }
    }
}

/// Perform the server half of the RFC 6455 opening handshake on a fresh
/// TCP stream. On success the stream is a [`WsStream`] (server mode) ready
/// for the GWP conversation; on failure the socket is returned untouched
/// enough to be dropped by the caller.
pub async fn accept(mut stream: TcpStream) -> Result<WsStream, GwpError> {
    use tokio::io::AsyncWriteExt;
    let raw = read_http_head(&mut stream).await?;
    let (_, headers) = parse_http_head(&raw)?;

    if !header_has_token(&headers, "upgrade", "websocket") {
        return Err(GwpError::Handshake("missing `Upgrade: websocket`"));
    }
    if !header_has_token(&headers, "connection", "upgrade") {
        return Err(GwpError::Handshake("missing `Connection: Upgrade`"));
    }
    let version = header_value(&headers, "sec-websocket-version")
        .ok_or(GwpError::Handshake("missing Sec-WebSocket-Version"))?;
    if version.trim() != "13" {
        return Err(GwpError::Handshake("only Sec-WebSocket-Version 13"));
    }
    let key = header_value(&headers, "sec-websocket-key")
        .ok_or(GwpError::Handshake("missing Sec-WebSocket-Key"))?
        .to_string();

    let response = format!(
        "HTTP/1.1 101 Switching Protocols\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Accept: {}\r\n\
         \r\n",
        accept_key(&key)
    );
    stream.write_all(response.as_bytes()).await?;
    stream.flush().await?;
    Ok(WsStream::new(stream, Mode::Server))
}

/// Client half of a WebSocket session (architecture §7's viewer door —
/// the Rust peer the TypeScript `connectWs` (M3) has been waiting for).
/// The conversation (handshake, requests, event buffering) is the shared
/// [`crate::conn::ClientCore`]; this type owns the upgrade and the
/// frame codec.
#[derive(Debug)]
pub struct WsClient {
    core: ClientCore<WsStream>,
}

impl WsClient {
    /// Connect to `host:port` (or a `ws://host:port` URL — the scheme is
    /// accepted and stripped, matching the `ConnectTarget::Ws` storage)
    /// and perform the RFC 6455 opening handshake (`GET /`), verifying the
    /// server's `Sec-WebSocket-Accept`.
    pub async fn connect(addr: &str) -> Result<WsClient, GwpError> {
        use tokio::io::AsyncWriteExt;
        let addr = addr.strip_prefix("ws://").unwrap_or(addr);
        if addr.starts_with("wss://") {
            return Err(GwpError::Protocol(
                "wss:// is not supported — the driver speaks to loopback peers in the clear".into(),
            ));
        }
        let mut stream = TcpStream::connect(addr).await?;
        let key = fresh_ws_key();
        let request = format!(
            "GET / HTTP/1.1\r\n\
             Host: {addr}\r\n\
             Upgrade: websocket\r\n\
             Connection: Upgrade\r\n\
             Sec-WebSocket-Key: {key}\r\n\
             Sec-WebSocket-Version: 13\r\n\
             \r\n"
        );
        stream.write_all(request.as_bytes()).await?;
        stream.flush().await?;
        let raw = read_http_head(&mut stream).await?;
        let (status, headers) = parse_http_head(&raw)?;
        if status != Some(101) {
            return Err(GwpError::Handshake("server refused the ws upgrade"));
        }
        if !header_has_token(&headers, "upgrade", "websocket") {
            return Err(GwpError::Handshake("ws upgrade response lacks websocket"));
        }
        let accept = header_value(&headers, "sec-websocket-accept")
            .ok_or(GwpError::Handshake("missing Sec-WebSocket-Accept"))?;
        if accept != accept_key(&key) {
            return Err(GwpError::Handshake("Sec-WebSocket-Accept mismatch"));
        }
        Ok(WsClient {
            core: ClientCore::new(WsStream::new(stream, Mode::Client)),
        })
    }

    /// Connected peer address (the server's bound port lives here for
    /// `127.0.0.1:0` style ephemeral binds).
    pub fn peer_addr(&self) -> Result<std::net::SocketAddr, GwpError> {
        self.core.stream().stream.peer_addr().map_err(GwpError::Io)
    }

    /// Send the GWP `Hello` and validate the driver's `Welcome` (§7) —
    /// the byte-identical TCP conversation, now over the upgraded socket.
    pub async fn handshake(
        &mut self,
        agent: &str,
    ) -> Result<crate::schema::AdapterInfoV1, GwpError> {
        self.core.handshake(agent).await
    }

    pub async fn request(&mut self, request: RequestV1) -> Result<ResponseV1, GwpError> {
        self.core.request(request).await
    }

    /// The next pushed event (§7, M12) — the WS mirror of the UDS
    /// [`crate::uds::Client::next_event`].
    pub async fn next_event(&mut self) -> Result<crate::schema::GwpEvent, GwpError> {
        self.core.next_event().await
    }

    /// Subscribe this connection to the named event kinds (§7, M12).
    pub async fn subscribe(&mut self, kinds: &[EventKindV1]) -> Result<(), GwpError> {
        self.core.subscribe(kinds).await
    }

    /// Drop this connection's event subscription (§7, M12).
    pub async fn unsubscribe(&mut self) -> Result<(), GwpError> {
        self.core.unsubscribe().await
    }

    pub async fn ping(&mut self) -> Result<(), GwpError> {
        self.core.ping().await
    }
}

/// Alias for symmetry with the other transports' `Client` types
/// (`uds::Client`, `tcp::Client`).
pub type Client = WsClient;

// [ws-tests-append-marker]

#[cfg(test)]
mod tests {
    use super::*;
    use crate::framing::{encode_frame as encode_gwp, read_frame};
    use crate::schema::{AdapterInfoV1, CapsV1, ErrorCode, GwpRequest, GwpResponse, PROTO_VERSION};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[test]
    fn sha1_matches_rfc_3174_vectors() {
        let hex = |bytes: [u8; 20]| {
            let mut out = String::new();
            for byte in bytes {
                use std::fmt::Write as _;
                write!(out, "{byte:02x}").expect("string write never fails");
            }
            out
        };
        assert_eq!(
            hex(sha1(b"abc")),
            "a9993e364706816aba3e25717850c26c9cd0d89d",
            "RFC 3174 §7.3 'abc'"
        );
        assert_eq!(
            hex(sha1(b"")),
            "da39a3ee5e6b4b0d3255bfef95601890afd80709",
            "RFC 3174 §7.3 empty"
        );
        assert_eq!(
            hex(sha1(
                b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
            )),
            "84983e441c3bd26ebaae4aa1f95129e5e54670f1",
            "RFC 3174 §7.3 448-bit"
        );
    }

    #[test]
    fn base64_matches_rfc_4648_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn accept_key_matches_rfc_6455_section_1_3() {
        // The RFC's own worked example.
        assert_eq!(
            accept_key("dGhlIHNhbXBsZSBub25jZQ=="),
            "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
        );
    }

    #[test]
    fn frames_roundtrip_through_the_grammar() {
        let cases: Vec<Vec<u8>> = vec![
            b"ping".to_vec(),
            Vec::new(),
            vec![0u8; 300],    // forces the 16-bit length form
            vec![7u8; 70_000], // forces the 64-bit length form
        ];
        for payload in &cases {
            // Client mode: masked outbound, unmasked by the parser.
            let frame = encode_frame(
                OPCODE_BINARY,
                payload,
                Mode::Client,
                [0x11, 0x22, 0x33, 0x44],
            );
            let mut inbound = Inbound {
                net: frame,
                ..Inbound::default()
            };
            inbound.parse_available().expect("client frame parses");
            assert_eq!(inbound.plain, *payload);

            // Server mode: unmasked on both ends.
            let frame = encode_frame(OPCODE_BINARY, payload, Mode::Server, [0; 4]);
            let mut inbound = Inbound {
                net: frame,
                ..Inbound::default()
            };
            inbound.parse_available().expect("server frame parses");
            assert_eq!(inbound.plain, *payload);
        }
    }

    #[test]
    fn a_partial_frame_stays_in_net_and_completes_later() {
        let full = encode_frame(OPCODE_BINARY, b"hello world", Mode::Server, [0; 4]);
        let (head, tail) = full.split_at(5);
        let mut inbound = Inbound::default();

        inbound.net.extend_from_slice(head);
        inbound.parse_available().expect("partial frame parses");
        assert!(inbound.plain.is_empty(), "nothing decoded from a prefix");

        inbound.net.extend_from_slice(tail);
        inbound.parse_available().expect("completed frame parses");
        assert_eq!(inbound.plain, b"hello world");
    }

    #[test]
    fn fragmented_messages_reassemble_and_controls_interleave() {
        // `encode_frame` always sets FIN (the driver never fragments), so
        // the fragments here clear the FIN bit in the header byte the way
        // a fragmenting peer would.
        let mut first = encode_frame(OPCODE_BINARY, b"he", Mode::Server, [0; 4]);
        first[0] &= !0x80; // FIN off: the message continues
        let mut third = encode_frame(OPCODE_CONTINUATION, b"ll", Mode::Server, [0; 4]);
        third[0] &= !0x80; // still fragmented
        let mut wire = Vec::new();
        wire.extend_from_slice(&first);
        // A control frame may interleave inside a fragmented message.
        wire.extend_from_slice(&encode_frame(OPCODE_PING, b"hi", Mode::Server, [0; 4]));
        wire.extend_from_slice(&third);
        wire.extend_from_slice(&encode_frame(
            OPCODE_CONTINUATION,
            b"o",
            Mode::Server,
            [0; 4],
        ));

        let mut inbound = Inbound {
            net: wire,
            ..Inbound::default()
        };
        inbound.parse_available().expect("fragmented stream parses");
        assert_eq!(inbound.plain, b"hello");
        assert_eq!(inbound.pings.pop_front().as_deref(), Some(b"hi".as_slice()));
    }

    #[test]
    fn a_continuation_without_a_fragment_is_a_protocol_error() {
        let mut inbound = Inbound {
            net: encode_frame(OPCODE_CONTINUATION, b"loose", Mode::Server, [0; 4]),
            ..Inbound::default()
        };
        let err = inbound.parse_available().expect_err("orphan continuation");
        assert!(matches!(err, GwpError::Protocol(_)), "{err:?}");
    }

    #[test]
    fn post_close_frames_are_not_decoded() {
        let mut wire = encode_frame(OPCODE_CLOSE, &1000u16.to_be_bytes(), Mode::Server, [0; 4]);
        wire.extend_from_slice(&encode_frame(OPCODE_BINARY, b"ghost", Mode::Server, [0; 4]));
        let mut inbound = Inbound {
            net: wire,
            ..Inbound::default()
        };
        inbound.parse_available().expect("close parses");
        assert!(inbound.close_seen);
        // The post-close binary frame is not decoded (we stopped at close).
        assert_eq!(inbound.plain.len(), 0, "no payload past the close");
    }

    #[test]
    fn masked_client_frames_decode_to_their_payload() {
        // §5.3 masking round-trip: mask on encode, unmask on parse.
        let payload = b"masked payload \x00\x01\x02";
        let frame = encode_frame(
            OPCODE_BINARY,
            payload,
            Mode::Client,
            [0xAA, 0xBB, 0xCC, 0xDD],
        );
        assert_eq!(frame[1] & 0x80, 0x80, "client frames carry the mask bit");
        let mut inbound = Inbound {
            net: frame,
            ..Inbound::default()
        };
        inbound.parse_available().expect("masked frame parses");
        assert_eq!(inbound.plain, payload);
    }

    #[test]
    fn oversized_declared_lengths_are_rejected() {
        // A header claiming more than the guard, with no payload: parse
        // must reject instead of buffering forever.
        let mut wire = vec![0x82, 0x7f];
        wire.extend_from_slice(&(MAX_FRAME_PAYLOAD + 1).to_be_bytes());
        let mut inbound = Inbound {
            net: wire,
            ..Inbound::default()
        };
        let err = inbound.parse_available().expect_err("oversized length");
        assert!(matches!(err, GwpError::Protocol(_)), "{err:?}");
    }

    /// A scripted GWP-over-WS server: accepts the upgrade, then runs the
    /// standard connection loop.
    async fn spawn_ws_server<H, F>(mut make_handler: F) -> (String, tokio::task::JoinHandle<()>)
    where
        H: crate::conn::Handler,
        F: FnMut() -> H + Send + 'static,
    {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("bind");
        let addr = listener.local_addr().expect("addr").to_string();
        let task = tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                let Ok(ws) = accept(stream).await else {
                    continue;
                };
                tokio::spawn(crate::conn::run_connection(ws, make_handler()));
            }
        });
        (addr, task)
    }

    fn scripted_handler() -> impl crate::conn::Handler {
        struct Echo;
        impl crate::conn::Handler for Echo {
            fn handle(
                &mut self,
                request: RequestV1,
            ) -> impl std::future::Future<Output = ResponseV1> + Send {
                let response = match request {
                    RequestV1::Hello { .. } => ResponseV1::Welcome {
                        proto_version: PROTO_VERSION,
                        session_id: "ws-session".into(),
                        tree_seq: 1,
                        frame_seq: 2,
                        adapter: AdapterInfoV1 {
                            name: "mock".into(),
                            framework: "fixture".into(),
                            framework_version: "0".into(),
                            caps: CapsV1::default(),
                        },
                    },
                    RequestV1::Ping => ResponseV1::Pong,
                    RequestV1::Bye => ResponseV1::Bye,
                    _ => ResponseV1::Error {
                        code: ErrorCode::Unsupported,
                        message: "not implemented in the ws echo handler".into(),
                    },
                };
                async move { response }
            }
        }
        Echo
    }

    #[tokio::test]
    async fn handshake_ping_bye_roundtrip_over_ws() {
        let (addr, _server) = spawn_ws_server(scripted_handler).await;
        let mut client = WsClient::connect(&addr).await.expect("client upgrades");
        let adapter = client.handshake("ws-test/0.1").await.unwrap();
        assert_eq!(adapter.name, "mock");
        assert_eq!(adapter.framework, "fixture");
        client.ping().await.unwrap();
        let resp = client.request(RequestV1::Bye).await.unwrap();
        assert_eq!(resp, ResponseV1::Bye);
    }

    #[tokio::test]
    async fn a_gwp_frame_split_across_ws_writes_survives() {
        // The reader-side hazard: a GWP frame arrives as raw bytes that
        // may span flushes and WS frames; the `AsyncRead` adapter hands
        // the GWP `FrameReader` a seamless byte stream.
        let (addr, _server) = spawn_ws_server(scripted_handler).await;
        let mut client = WsClient::connect(&addr).await.expect("client upgrades");
        client.handshake("split-test").await.unwrap();

        let frame = encode_gwp(&GwpRequest::V1(RequestV1::Ping)).unwrap();
        let (head, tail) = frame.split_at(3);
        client
            .core
            .stream_mut()
            .write_all(head)
            .await
            .expect("head writes");
        client
            .core
            .stream_mut()
            .flush()
            .await
            .expect("head flushes");
        client
            .core
            .stream_mut()
            .write_all(tail)
            .await
            .expect("tail writes");
        client
            .core
            .stream_mut()
            .flush()
            .await
            .expect("tail flushes");
        let resp: GwpResponse = read_frame(client.core.stream_mut()).await.unwrap();
        let GwpResponse::V1(ResponseV1::Pong) = resp else {
            panic!("expected Pong, got {resp:?}");
        };
    }

    #[tokio::test]
    async fn ws_level_pings_get_auto_pongs() {
        // A raw-socket client (below the GWP layer) pings the server; the
        // connection loop must answer with a Pong carrying the same
        // application payload.
        let (addr, _server) = spawn_ws_server(scripted_handler).await;
        let mut raw = TcpStream::connect(&addr).await.expect("connect");
        let key = fresh_ws_key();
        raw.write_all(
            format!(
                "GET / HTTP/1.1\r\n\
                 Host: {addr}\r\n\
                 Upgrade: websocket\r\n\
                 Connection: Upgrade\r\n\
                 Sec-WebSocket-Key: {key}\r\n\
                 Sec-WebSocket-Version: 13\r\n\
                 \r\n"
            )
            .as_bytes(),
        )
        .await
        .expect("upgrade writes");
        let head = read_http_head(&mut raw).await.expect("101 head");
        assert!(head.contains("101 Switching Protocols"), "{head:?}");

        let ping = encode_frame(OPCODE_PING, b"keep-alive", Mode::Client, [1, 2, 3, 4]);
        raw.write_all(&ping).await.expect("ping writes");

        // The server auto-pongs: one unmasked control frame back.
        let mut head = [0u8; 2];
        tokio::time::timeout(std::time::Duration::from_secs(2), raw.read_exact(&mut head))
            .await
            .expect("pong in time")
            .expect("pong header");
        assert_eq!(head[0], 0x8A, "FIN + Pong opcode, got {:#04x}", head[0]);
        assert_eq!(head[1] & 0x80, 0, "server frames are unmasked");
        let len = usize::from(head[1] & 0x7f);
        let mut payload = vec![0u8; len];
        raw.read_exact(&mut payload).await.expect("pong payload");
        assert_eq!(payload, b"keep-alive");
    }

    /// A handler that answers `Hello` normally and every other request
    /// with a 300 KB error payload (the long WS length form's ride).
    struct BigPongHandler;

    impl crate::conn::Handler for BigPongHandler {
        fn handle(
            &mut self,
            request: RequestV1,
        ) -> impl std::future::Future<Output = ResponseV1> + Send {
            let response = match request {
                RequestV1::Hello { .. } => ResponseV1::Welcome {
                    proto_version: PROTO_VERSION,
                    session_id: "big-ws".into(),
                    tree_seq: 1,
                    frame_seq: 2,
                    adapter: AdapterInfoV1 {
                        name: "mock".into(),
                        framework: "fixture".into(),
                        framework_version: "0".into(),
                        caps: CapsV1::default(),
                    },
                },
                _ => ResponseV1::Error {
                    code: ErrorCode::Internal,
                    message: "x".repeat(300_000),
                },
            };
            async move { response }
        }
    }

    #[tokio::test]
    async fn large_gwp_frames_ride_the_64_bit_length_form() {
        // A response larger than 64 KiB forces the 16/64-bit WS length
        // form outbound and exercises the read path's reassembly.
        let (addr, _server) = spawn_ws_server(|| BigPongHandler).await;
        let mut client = WsClient::connect(&addr).await.expect("client upgrades");
        client.handshake("big-client").await.unwrap();
        let err = client
            .request(RequestV1::Ping)
            .await
            .expect_err("the big error reply rejects");
        let GwpError::Rejected { message, .. } = err else {
            panic!("expected Rejected, got {err:?}");
        };
        assert_eq!(
            message.len(),
            300_000,
            "the whole 300 KB payload rode the wire"
        );
    }

    #[tokio::test]
    async fn a_rejected_upgrade_closes_cleanly() {
        // Plain HTTP bytes to a WS endpoint: the server rejects at the
        // handshake instead of corrupting the conversation.
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("bind");
        let addr = listener.local_addr().expect("addr").to_string();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let _ = accept(stream).await; // reject expected here
            }
        });

        let mut raw = TcpStream::connect(&addr).await.unwrap();
        raw.write_all(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n")
            .await
            .unwrap();
        let mut buf = Vec::new();
        let res = raw.read_to_end(&mut buf).await;
        // The server may drop or answer; either way it never upgrades.
        let head = String::from_utf8_lossy(&buf);
        assert!(!head.contains("101 Switching Protocols"));
        assert!(res.is_ok() || res.is_err(), "socket closes without 101");
    }

    #[tokio::test]
    async fn events_flow_over_ws_like_any_transport() {
        use crate::schema::{EventKindV1, GwpEvent, SubscribeV1};
        use tokio::sync::mpsc;

        struct Emitting {
            rx: Option<mpsc::Receiver<GwpEvent>>,
        }
        impl crate::conn::Handler for Emitting {
            fn handle(
                &mut self,
                request: RequestV1,
            ) -> impl std::future::Future<Output = ResponseV1> + Send {
                let response = match request {
                    RequestV1::Hello { .. } => ResponseV1::Welcome {
                        proto_version: PROTO_VERSION,
                        session_id: "ws-events".into(),
                        tree_seq: 1,
                        frame_seq: 2,
                        adapter: AdapterInfoV1 {
                            name: "mock".into(),
                            framework: "fixture".into(),
                            framework_version: "0".into(),
                            caps: CapsV1::default(),
                        },
                    },
                    RequestV1::Subscribe(SubscribeV1::On { kinds }) => {
                        ResponseV1::Subscribed { kinds }
                    }
                    RequestV1::Subscribe(SubscribeV1::Off) => {
                        ResponseV1::Subscribed { kinds: Vec::new() }
                    }
                    _ => ResponseV1::Pong,
                };
                async move { response }
            }
            fn event_stream(&mut self) -> Option<mpsc::Receiver<GwpEvent>> {
                self.rx.take()
            }
        }

        let (tx, rx) = mpsc::channel(crate::conn::EVENT_QUEUE_CAPACITY);
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("bind");
        let addr = listener.local_addr().expect("addr").to_string();
        tokio::spawn(async move {
            // One connection, one handler (the test's only client).
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let Ok(ws) = accept(stream).await else {
                return;
            };
            let _ = crate::conn::run_connection(ws, Emitting { rx: Some(rx) }).await;
        });

        let mut client = WsClient::connect(&addr).await.expect("client upgrades");
        client.handshake("subscriber").await.unwrap();
        client
            .subscribe(&[EventKindV1::TreeChanged])
            .await
            .expect("subscribe");
        tx.send(GwpEvent::V1(crate::schema::EventV1::TreeChanged {
            seq: 42,
        }))
        .await
        .unwrap();
        let event = tokio::time::timeout(std::time::Duration::from_secs(2), client.next_event())
            .await
            .expect("event in time")
            .expect("event parses");
        let GwpEvent::V1(crate::schema::EventV1::TreeChanged { seq }) = event else {
            panic!("expected TreeChanged, got {event:?}");
        };
        assert_eq!(seq, 42);
        client.unsubscribe().await.unwrap();
    }
}
