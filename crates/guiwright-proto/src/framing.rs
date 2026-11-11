//! GWP framing: 4-byte little-endian length prefix + postcard-encoded body
//! (architecture §7 "Framing"). Async over any tokio stream; a single
//! `MAX_FRAME_BYTES` guard rejects corrupt or hostile prefixes.

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::GwpError;
use crate::schema::MAX_FRAME_BYTES;

/// Encode one frame into a fresh buffer (used by tests and the blob channel).
pub fn encode_frame<T: serde::Serialize>(payload: &T) -> Result<Vec<u8>, GwpError> {
    let body = postcard::to_allocvec(payload)?;
    if body.len() > MAX_FRAME_BYTES {
        return Err(GwpError::FrameTooLarge(body.len()));
    }
    let mut out = Vec::with_capacity(4 + body.len());
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    Ok(out)
}

/// Write one length-prefixed frame.
pub async fn write_frame<T: serde::Serialize, W: AsyncWrite + Unpin>(
    sink: &mut W,
    payload: &T,
) -> Result<(), GwpError> {
    let frame = encode_frame(payload)?;
    sink.write_all(&frame).await?;
    sink.flush().await?;
    Ok(())
}

/// Read exactly one length-prefixed frame and decode it.
pub async fn read_frame<T: serde::de::DeserializeOwned, R: AsyncRead + Unpin>(
    stream: &mut R,
) -> Result<T, GwpError> {
    let mut len_buf = [0u8; 4];
    stream
        .read_exact(&mut len_buf)
        .await
        .map_err(GwpError::Io)?;
    let len = u32::from_le_bytes(len_buf) as usize;
    if len > MAX_FRAME_BYTES {
        return Err(GwpError::FrameTooLarge(len));
    }
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body).await.map_err(GwpError::Io)?;
    Ok(postcard::from_bytes(&body)?)
}

/// Incremental frame reassembler (§7 multiplexing, M12): the server
/// connection loop reads requests and pushed events **concurrently**
/// (`tokio::select!`), so a partially-read frame must survive the future
/// being dropped when the other branch wins. State lives in the struct —
/// bytes consumed from the stream are never lost, exactly like the
/// TypeScript client's `FrameReader` (clients/typescript/src/framing.ts).
pub struct FrameReader<R> {
    stream: R,
    /// Bytes read from the stream but not yet consumed as a frame.
    buf: Vec<u8>,
}

impl<R: std::fmt::Debug> std::fmt::Debug for FrameReader<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FrameReader")
            .field("stream", &self.stream)
            .field("buf_len", &self.buf.len())
            .finish()
    }
}

impl<R: AsyncRead + Unpin> FrameReader<R> {
    pub fn new(stream: R) -> Self {
        Self {
            stream,
            buf: Vec::new(),
        }
    }

    /// Read the next complete frame body. Cancellation-safe: dropping the
    /// future mid-read keeps partial bytes in the buffer for the next call.
    pub async fn next_frame(&mut self) -> Result<Vec<u8>, GwpError> {
        loop {
            if self.buf.len() >= 4 {
                let len =
                    u32::from_le_bytes(self.buf[..4].try_into().expect("length prefix is 4 bytes"))
                        as usize;
                if len > MAX_FRAME_BYTES {
                    return Err(GwpError::FrameTooLarge(len));
                }
                if self.buf.len() >= 4 + len {
                    let body = self.buf.split_off(4);
                    let body = {
                        // `split_off(4)` moves everything past the prefix
                        // into `body`; the frame is its first `len` bytes.
                        let mut framed = body;
                        let rest = framed.split_off(len);
                        self.buf = rest;
                        framed
                    };
                    return Ok(body);
                }
            }
            // Not enough buffered yet: pull one chunk from the stream.
            let mut chunk = [0u8; 4096];
            let n = self.stream.read(&mut chunk).await.map_err(GwpError::Io)?;
            if n == 0 {
                return Err(GwpError::Io(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "connection closed mid-frame",
                )));
            }
            self.buf.extend_from_slice(&chunk[..n]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{GwpRequest, RequestV1};
    use tokio::io::duplex;

    #[tokio::test]
    async fn frame_reader_survives_split_and_coalesced_frames() {
        // The §7 multiplexing contract: chunk boundaries may split a
        // frame anywhere; the reader reassembles without losing a byte.
        let payloads: Vec<GwpRequest> = vec![
            GwpRequest::V1(RequestV1::Ping),
            GwpRequest::V1(RequestV1::Hello {
                proto_version: 1,
                agent: "reader-test".into(),
            }),
            GwpRequest::V1(RequestV1::Bye),
        ];
        let (mut tx, rx) = duplex(64);
        let mut reader = FrameReader::new(rx);
        let frames: Vec<Vec<u8>> = payloads
            .iter()
            .map(|p| encode_frame(p).expect("encode"))
            .collect();

        // One byte at a time (worst-case split).
        let frame_count = payloads.len();
        let handle = tokio::spawn(async move {
            let mut decoded = Vec::new();
            for _ in 0..frame_count {
                let body = reader.next_frame().await.expect("frame");
                decoded.push(postcard::from_bytes::<GwpRequest>(&body).expect("decode"));
            }
            decoded
        });
        for frame in &frames {
            for byte in frame {
                tx.write_all(std::slice::from_ref(byte)).await.unwrap();
                tokio::task::yield_now().await;
            }
        }
        let decoded = handle.await.unwrap();
        assert_eq!(decoded, payloads);
    }

    #[tokio::test]
    async fn frame_reader_rejects_oversized_prefix_and_eof_is_an_error() {
        let (mut tx, rx) = duplex(64);
        let mut reader = FrameReader::new(rx);
        let evil = (MAX_FRAME_BYTES as u32 + 1).to_le_bytes();
        tx.write_all(&evil).await.unwrap();
        let res = reader.next_frame().await;
        assert!(matches!(res, Err(GwpError::FrameTooLarge(_))));
        drop(tx);
        let res = reader.next_frame().await;
        assert!(res.is_err(), "EOF surfaces as an error, not a hang");
    }

    #[tokio::test]
    async fn frames_survive_roundtrip_and_coalescing() {
        let payloads: Vec<GwpRequest> = vec![
            GwpRequest::V1(RequestV1::Ping),
            GwpRequest::V1(RequestV1::Hello {
                proto_version: 1,
                agent: "test".into(),
            }),
            GwpRequest::V1(RequestV1::Bye),
        ];
        let (mut tx, mut rx) = duplex(64);
        // Write all frames back-to-back (coalesced in the pipe).
        for p in &payloads {
            write_frame(&mut tx, p).await.unwrap();
        }
        drop(tx);

        for expected in &payloads {
            let got: GwpRequest = read_frame(&mut rx).await.unwrap();
            assert_eq!(&got, expected);
        }
        // EOF cleanly.
        let res: Result<GwpRequest, _> = read_frame(&mut rx).await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn oversized_length_prefix_is_rejected() {
        let (mut tx, mut rx) = duplex(64);
        let evil = (MAX_FRAME_BYTES as u32 + 1).to_le_bytes();
        tx.write_all(&evil).await.unwrap();
        let res: Result<GwpRequest, _> = read_frame(&mut rx).await;
        assert!(matches!(res, Err(GwpError::FrameTooLarge(_))));
    }

    #[test]
    fn encode_frame_prefixes_little_endian_length() {
        let req = GwpRequest::V1(RequestV1::Ping);
        let frame = encode_frame(&req).unwrap();
        let len = u32::from_le_bytes(frame[0..4].try_into().unwrap()) as usize;
        assert_eq!(frame.len(), 4 + len);
    }
}
