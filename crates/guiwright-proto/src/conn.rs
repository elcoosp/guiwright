//! Transport-agnostic GWP connection state machine (architecture §7),
//! shared by the UDS, TCP and WS transports: the server-side handshake
//! gate + request dispatch, and the client-side conversation core. The
//! transports own only their accept loops and socket types.
//!
//! The server multiplexes one connection per client; each request is a
//! round-trip. Pushed events land on the handler's queue and are
//! flushed concurrently with request reads and between responses
//! (bounded — slow clients degrade, never block the UI thread, §15.6).
//! Events ride the response envelope ([`ResponseV1::Event`]) so a
//! client demultiplexes one enum; a connection whose handler never
//! produces events keeps the byte-exact pre-M12 conversation.

use std::collections::VecDeque;
use std::future::Future;

use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc;

use crate::GwpError;
use crate::framing::{FrameReader, read_frame, write_frame};
use crate::schema::{
    ErrorCode, EventKindV1, EventV1, GwpEvent, GwpRequest, GwpResponse, PROTO_VERSION, RequestV1,
    ResponseV1, SubscribeV1,
};

/// A request handler: application logic behind the transport.
pub trait Handler: Send + 'static {
    fn handle(&mut self, request: RequestV1) -> impl Future<Output = ResponseV1> + Send;

    /// This connection's outbound event queue (§7 server-push events,
    /// M12). Called exactly once, before the request loop: the returned
    /// receiver is drained by the connection loop and flushed to the
    /// client as [`ResponseV1::Event`] frames. `None` keeps the
    /// connection event-free — the byte-identical pre-M12 behavior.
    fn event_stream(&mut self) -> Option<mpsc::Receiver<GwpEvent>> {
        None
    }
}

/// Bounded outbound event queue (§15.6 backpressure: 64 messages default).
pub const EVENT_QUEUE_CAPACITY: usize = 64;

/// Run one client connection until `Bye` or EOF. Generic over the stream
/// so the UDS and TCP transports share the exact handshake + dispatch
/// semantics (identical bytes in, identical bytes out).
///
/// The loop multiplexes request reads against the handler's event queue:
/// reads go through [`FrameReader`], so a partially-read request frame
/// survives the event branch winning the `select!` (no consumed bytes are
/// ever lost — the §7 multiplexing contract).
pub async fn run_connection<S, H>(stream: S, mut handler: H) -> Result<(), GwpError>
where
    S: AsyncRead + AsyncWrite + Unpin,
    H: Handler,
{
    // Split halves: the reader owns the incremental frame state, writes
    // (responses + event flushes) share the write half.
    let (reader_half, mut writer) = tokio::io::split(stream);
    let mut reader = FrameReader::new(reader_half);

    // Handshake gate: first frame must be Hello with a known version (§7).
    let first: GwpRequest = decode_request(&reader.next_frame().await?)?;
    let GwpRequest::V1(RequestV1::Hello {
        proto_version,
        agent,
    }) = first
    else {
        write_frame(
            &mut writer,
            &GwpResponse::V1(ResponseV1::Error {
                code: ErrorCode::NotHandshaked,
                message: "first request must be Hello".into(),
            }),
        )
        .await?;
        return Err(GwpError::Handshake("first request was not Hello"));
    };
    if proto_version != PROTO_VERSION {
        write_frame(
            &mut writer,
            &GwpResponse::V1(ResponseV1::Error {
                code: ErrorCode::UnsupportedVersion,
                message: format!(
                    "client protocol {proto_version}, driver protocol {PROTO_VERSION}"
                ),
            }),
        )
        .await?;
        return Err(GwpError::Handshake("version mismatch"));
    }
    tracing::debug!(agent, "client handshaked");
    write_frame(
        &mut writer,
        &handshake_response(
            &handler
                .handle(RequestV1::Hello {
                    proto_version,
                    agent,
                })
                .await,
        ),
    )
    .await?;

    // Event pump (§7, M12): the handler's queue, flushed while waiting
    // for the next request and after each response (bounded queue,
    // §15.6). `None` = this connection never receives events.
    let mut events = handler.event_stream();
    loop {
        tokio::select! {
            event = next_event(&mut events) => {
                // `None` means the queue closed (the handler side is gone):
                // stop arming the branch, keep serving requests.
                if let Some(event) = event {
                    write_frame(&mut writer, &GwpResponse::V1(ResponseV1::Event(event))).await?;
                } else {
                    events = None;
                }
            }
            frame = reader.next_frame() => {
                let req: GwpRequest = decode_request(&frame?)?;
                // GwpRequest has a single (versioned) variant, so this is
                // irrefutable; the version check happened at handshake.
                let GwpRequest::V1(inner) = req;
                if matches!(inner, RequestV1::Bye) {
                    write_frame(&mut writer, &GwpResponse::V1(ResponseV1::Bye)).await?;
                    return Ok(());
                }
                let resp = handler.handle(inner).await;
                write_frame(&mut writer, &GwpResponse::V1(resp)).await?;
                // Opportunistically flush queued events after each response.
                while let Some(event) = try_next_event(&mut events) {
                    write_frame(&mut writer, &GwpResponse::V1(ResponseV1::Event(event))).await?;
                }
            }
        }
    }
}

/// Decode one request frame body (the incremental reader returns raw
/// postcard bytes).
fn decode_request(body: &[u8]) -> Result<GwpRequest, GwpError> {
    Ok(postcard::from_bytes(body)?)
}

/// Await the handler's queue, treating "no queue" as "never ready".
async fn next_event(events: &mut Option<mpsc::Receiver<GwpEvent>>) -> Option<EventV1> {
    match events {
        Some(rx) => rx.recv().await.map(|GwpEvent::V1(event)| event),
        None => std::future::pending().await,
    }
}

/// Non-blocking drain for the post-response flush: `Some(event)` when one
/// is already queued, `None` when the queue is empty or absent.
fn try_next_event(events: &mut Option<mpsc::Receiver<GwpEvent>>) -> Option<EventV1> {
    let rx = events.as_mut()?;
    rx.try_recv().ok().map(|GwpEvent::V1(event)| event)
}

/// Client-side receive half (§7 conversation): reads one response frame,
/// buffering any pushed [`ResponseV1::Event`] frames that interleave —
/// events flow between responses on a subscribed connection, and a
/// request reply always eventually follows. Shared by both transports'
/// `Client::request`.
pub(crate) async fn read_response_demuxing<R: AsyncRead + Unpin>(
    stream: &mut R,
    events: &mut VecDeque<GwpEvent>,
) -> Result<ResponseV1, GwpError> {
    loop {
        let resp: GwpResponse = crate::framing::read_frame(stream).await?;
        let GwpResponse::V1(inner) = resp;
        match inner {
            ResponseV1::Event(event) => events.push_back(GwpEvent::V1(event)),
            other => return Ok(other),
        }
    }
}

/// Client-side event read (§7, M12): the next pushed event — from the
/// buffer of events that interleaved earlier responses, or straight off
/// the wire. A non-event frame here means the server answered a request
/// the client never made: a protocol violation, never silently swallowed.
pub(crate) async fn read_next_event<R: AsyncRead + Unpin>(
    stream: &mut R,
    events: &mut VecDeque<GwpEvent>,
) -> Result<GwpEvent, GwpError> {
    if let Some(event) = events.pop_front() {
        return Ok(event);
    }
    let resp: GwpResponse = crate::framing::read_frame(stream).await?;
    let GwpResponse::V1(inner) = resp;
    match inner {
        ResponseV1::Event(event) => Ok(GwpEvent::V1(event)),
        other => Err(GwpError::Protocol(format!(
            "expected a pushed event frame, got a response: {other:?}"
        ))),
    }
}

/// Extract a `Welcome` from a handler response, or synthesize an error.
fn handshake_response(resp: &ResponseV1) -> GwpResponse {
    match resp {
        ResponseV1::Welcome { .. } => GwpResponse::V1(resp.clone()),
        other => GwpResponse::V1(ResponseV1::Error {
            code: ErrorCode::Internal,
            message: format!("driver handler did not answer Hello with Welcome: {other:?}"),
        }),
    }
}

/// Client half of a GWP conversation, generic over the byte stream
/// (architecture §7): the handshake, request round-trip, pushed-event
/// buffering and typed convenience wrappers the UDS, TCP and WS clients
/// all share. A session is byte-identical on every door — the transport
/// owns only its `connect`, and this struct owns the conversation, so a
/// bug fixed here is fixed on all three transports at once (the M15
/// extraction that retired the UDS/TCP copy-paste).
#[derive(Debug)]
pub(crate) struct ClientCore<S> {
    stream: S,
    /// Events that interleaved earlier responses (§7, M12): pushed frames
    /// read while waiting for a request reply are buffered here, not lost.
    events: VecDeque<GwpEvent>,
}

impl<S: AsyncRead + AsyncWrite + Unpin> ClientCore<S> {
    pub(crate) fn new(stream: S) -> Self {
        Self {
            stream,
            events: VecDeque::new(),
        }
    }

    /// The underlying byte stream (socket-level queries like the TCP
    /// client's `peer_addr` stay transport-side).
    pub(crate) fn stream(&self) -> &S {
        &self.stream
    }

    /// Mutable access for tests and transport-level plumbing.
    #[cfg(test)]
    pub(crate) fn stream_mut(&mut self) -> &mut S {
        &mut self.stream
    }

    /// Send `Hello` and validate the driver's `Welcome`.
    pub(crate) async fn handshake(
        &mut self,
        agent: &str,
    ) -> Result<crate::schema::AdapterInfoV1, GwpError> {
        write_frame(
            &mut self.stream,
            &GwpRequest::V1(RequestV1::Hello {
                proto_version: PROTO_VERSION,
                agent: agent.to_string(),
            }),
        )
        .await?;
        let resp: GwpResponse = read_frame(&mut self.stream).await?;
        let GwpResponse::V1(ResponseV1::Welcome {
            proto_version,
            adapter,
            ..
        }) = resp
        else {
            return match resp {
                GwpResponse::V1(ResponseV1::Error { code, message }) => {
                    Err(GwpError::Rejected { code, message })
                }
                GwpResponse::V1(_) => Err(GwpError::Handshake("expected Welcome")),
            };
        };
        if proto_version != PROTO_VERSION {
            return Err(GwpError::Handshake("driver protocol mismatch"));
        }
        Ok(adapter)
    }

    pub(crate) async fn request(&mut self, request: RequestV1) -> Result<ResponseV1, GwpError> {
        write_frame(&mut self.stream, &GwpRequest::V1(request)).await?;
        // One response, with pushed events buffered for `next_event` (§7,
        // M12): a subscribed connection receives `Event` frames between
        // responses; a connection that never subscribes never sees one.
        let inner = read_response_demuxing(&mut self.stream, &mut self.events).await?;
        match inner {
            ResponseV1::Error { code, message } => Err(GwpError::Rejected { code, message }),
            other => Ok(other),
        }
    }

    /// The next pushed event (§7, M12): from the buffer of events that
    /// interleaved earlier responses, or straight off the wire. Blocks
    /// until the driver publishes one — pair it with the subscriber's
    /// own pacing, never on a hot path.
    pub(crate) async fn next_event(&mut self) -> Result<GwpEvent, GwpError> {
        read_next_event(&mut self.stream, &mut self.events).await
    }

    /// Subscribe this connection to the named event kinds (§7, M12).
    /// Re-subscribing replaces the filter; the ack echoes the kinds the
    /// driver honored.
    pub(crate) async fn subscribe(&mut self, kinds: &[EventKindV1]) -> Result<(), GwpError> {
        let resp = self
            .request(RequestV1::Subscribe(SubscribeV1::On {
                kinds: kinds.to_vec(),
            }))
            .await?;
        match resp {
            ResponseV1::Subscribed { .. } => Ok(()),
            other => Err(GwpError::Protocol(format!(
                "expected Subscribed, got {other:?}"
            ))),
        }
    }

    /// Drop this connection's event subscription (§7, M12).
    pub(crate) async fn unsubscribe(&mut self) -> Result<(), GwpError> {
        match self.request(RequestV1::Subscribe(SubscribeV1::Off)).await? {
            ResponseV1::Subscribed { kinds } if kinds.is_empty() => Ok(()),
            other => Err(GwpError::Protocol(format!(
                "expected an empty Subscribed ack, got {other:?}"
            ))),
        }
    }

    pub(crate) async fn ping(&mut self) -> Result<(), GwpError> {
        match self.request(RequestV1::Ping).await? {
            ResponseV1::Pong => Ok(()),
            other => Err(GwpError::Protocol(format!("expected Pong, got {other:?}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::framing::read_frame;
    use crate::schema::{AdapterInfoV1, CapsV1};
    use std::sync::{Arc, Mutex};
    use tokio::io::duplex;

    /// A handler with a scriptable request inbox — the connection-loop
    /// test double without events (the pre-M12 shape).
    struct ScriptedHandler {
        inbox: Arc<Mutex<Vec<RequestV1>>>,
    }

    /// The scripted reply table, shared by both handler shapes.
    fn scripted_response(request: &RequestV1) -> ResponseV1 {
        match request {
            RequestV1::Hello { .. } => ResponseV1::Welcome {
                proto_version: PROTO_VERSION,
                session_id: "test-session".into(),
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
                message: "not implemented in the scripted handler".into(),
            },
        }
    }

    impl Handler for ScriptedHandler {
        fn handle(
            &mut self,
            request: RequestV1,
        ) -> impl std::future::Future<Output = ResponseV1> + Send {
            self.inbox.lock().unwrap().push(request.clone());
            async move { scripted_response(&request) }
        }
    }

    /// The event-producing shape: each connection gets its own event
    /// channel (the real bridge's shape); the test holds a sender clone.
    struct EmittingHandler {
        inbox: Arc<Mutex<Vec<RequestV1>>>,
        rx: Option<mpsc::Receiver<GwpEvent>>,
    }

    impl EmittingHandler {
        fn new(inbox: Arc<Mutex<Vec<RequestV1>>>) -> (Self, mpsc::Sender<GwpEvent>) {
            let (tx, rx) = mpsc::channel(EVENT_QUEUE_CAPACITY);
            (
                Self {
                    inbox,
                    rx: Some(rx),
                },
                tx,
            )
        }
    }

    impl Handler for EmittingHandler {
        fn handle(
            &mut self,
            request: RequestV1,
        ) -> impl std::future::Future<Output = ResponseV1> + Send {
            self.inbox.lock().unwrap().push(request.clone());
            async move { scripted_response(&request) }
        }

        fn event_stream(&mut self) -> Option<mpsc::Receiver<GwpEvent>> {
            self.rx.take()
        }
    }

    async fn connect_and_handshake(client: &mut tokio::io::DuplexStream) -> ResponseV1 {
        write_frame(
            client,
            &GwpRequest::V1(RequestV1::Hello {
                proto_version: PROTO_VERSION,
                agent: "conn-test".into(),
            }),
        )
        .await
        .unwrap();
        let resp: GwpResponse = read_frame(client).await.unwrap();
        let GwpResponse::V1(inner) = resp;
        assert!(matches!(inner, ResponseV1::Welcome { .. }));
        inner
    }

    #[tokio::test]
    async fn events_are_flushed_while_the_connection_is_idle() {
        let (server_side, mut client_side) = duplex(4096);
        let inbox = Arc::new(Mutex::new(Vec::new()));
        let (handler, tx) = EmittingHandler::new(Arc::clone(&inbox));
        tokio::spawn(run_connection(server_side, handler));

        connect_and_handshake(&mut client_side).await;

        // Push an event with no request in flight: the §7 multiplexed
        // loop flushes it without any client traffic.
        tx.send(GwpEvent::V1(EventV1::TreeChanged { seq: 5 }))
            .await
            .unwrap();
        let resp: GwpResponse = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            read_frame(&mut client_side).await.unwrap()
        })
        .await
        .expect("event flushed while idle");
        let GwpResponse::V1(ResponseV1::Event(EventV1::TreeChanged { seq })) = resp else {
            panic!("expected a pushed Event frame, got {resp:?}");
        };
        assert_eq!(seq, 5);
    }

    #[tokio::test]
    async fn events_interleave_with_responses_and_requests_survive() {
        // The multiplexing hazard: an event arriving while a request
        // frame is being read must not corrupt the stream. The
        // `FrameReader` keeps partial reads alive across `select!` drops.
        let (server_side, mut client_side) = duplex(4096);
        let inbox = Arc::new(Mutex::new(Vec::new()));
        let (handler, tx) = EmittingHandler::new(Arc::clone(&inbox));
        tokio::spawn(run_connection(server_side, handler));

        connect_and_handshake(&mut client_side).await;

        // Interleave: event, ping — the event may flush before or after
        // the reply (the §7 queue flush races the request loop); clients
        // demultiplex by frame kind, never by position.
        for i in 0..3u64 {
            tx.send(GwpEvent::V1(EventV1::TreeChanged { seq: 10 + i }))
                .await
                .unwrap();
            write_frame(&mut client_side, &GwpRequest::V1(RequestV1::Ping))
                .await
                .unwrap();
            let mut pong = false;
            let mut event_seq = None;
            for _ in 0..2 {
                let resp: GwpResponse = read_frame(&mut client_side).await.unwrap();
                let GwpResponse::V1(inner) = resp;
                match inner {
                    ResponseV1::Pong => pong = true,
                    ResponseV1::Event(EventV1::TreeChanged { seq }) => event_seq = Some(seq),
                    other => panic!("reply {i}: unexpected frame {other:?}"),
                }
            }
            assert!(pong, "reply {i}: the Pong never arrived");
            assert_eq!(event_seq, Some(10 + i), "reply {i}: wrong event");
        }
    }

    #[tokio::test]
    async fn a_half_read_request_survives_a_concurrent_event() {
        // The mid-frame hazard made concrete: deliver the request frame
        // in two halves with an event queued in between. A naive
        // `select!` over `read_exact` futures loses the consumed prefix
        // bytes and desynchronizes the stream; the incremental reader
        // does not.
        use tokio::io::AsyncWriteExt;
        let (server_side, mut client_side) = duplex(4096);
        let inbox = Arc::new(Mutex::new(Vec::new()));
        let (handler, tx) = EmittingHandler::new(Arc::clone(&inbox));
        tokio::spawn(run_connection(server_side, handler));

        connect_and_handshake(&mut client_side).await;

        let frame = crate::framing::encode_frame(&GwpRequest::V1(RequestV1::Ping)).unwrap();
        let (head, tail) = frame.split_at(3); // prefix + 0 body bytes: mid-frame
        client_side.write_all(head).await.unwrap();
        // Give the server a chance to consume the partial frame, then
        // queue the event and finish the request.
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        tx.send(GwpEvent::V1(EventV1::FramePresented { seq: 1, hash: 2 }))
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        client_side.write_all(tail).await.unwrap();

        // Both the event and the (uncorrupted) reply arrive.
        let first: GwpResponse = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            read_frame(&mut client_side).await.unwrap()
        })
        .await
        .expect("frames keep flowing");
        let second: GwpResponse = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            read_frame(&mut client_side).await.unwrap()
        })
        .await
        .expect("the request still completes");
        let got_event = matches!(
            first,
            GwpResponse::V1(ResponseV1::Event(EventV1::FramePresented { .. }))
        ) || matches!(
            second,
            GwpResponse::V1(ResponseV1::Event(EventV1::FramePresented { .. }))
        );
        let got_pong = matches!(first, GwpResponse::V1(ResponseV1::Pong))
            || matches!(second, GwpResponse::V1(ResponseV1::Pong));
        assert!(
            got_event,
            "the event was flushed: {first:?} then {second:?}"
        );
        assert!(
            got_pong,
            "the half-read request was not lost: {first:?} then {second:?}"
        );
    }

    #[tokio::test]
    async fn a_handler_without_event_stream_keeps_the_conversation_event_free() {
        // Pre-M12 byte-compatibility: no event_stream → no Event frames,
        // ever (the flush branch never arms).
        let (server_side, mut client_side) = duplex(4096);
        let inbox = Arc::new(Mutex::new(Vec::new()));
        let handler = ScriptedHandler { inbox };
        tokio::spawn(run_connection(server_side, handler));

        connect_and_handshake(&mut client_side).await;
        write_frame(&mut client_side, &GwpRequest::V1(RequestV1::Ping))
            .await
            .unwrap();
        let resp: GwpResponse = read_frame(&mut client_side).await.unwrap();
        let GwpResponse::V1(inner) = resp;
        assert!(matches!(inner, ResponseV1::Pong));
        // Bye still closes cleanly.
        write_frame(&mut client_side, &GwpRequest::V1(RequestV1::Bye))
            .await
            .unwrap();
        let resp: GwpResponse = read_frame(&mut client_side).await.unwrap();
        let GwpResponse::V1(ResponseV1::Bye) = resp else {
            panic!("expected Bye, got {resp:?}");
        };
    }

    #[tokio::test]
    async fn events_flush_after_each_response_without_waiting_for_idle() {
        // §15.6 latency bound: events queued while the handler was busy
        // flush right after the response, before the next request.
        let (server_side, mut client_side) = duplex(4096);
        let inbox = Arc::new(Mutex::new(Vec::new()));
        let (handler, tx) = EmittingHandler::new(Arc::clone(&inbox));
        tokio::spawn(run_connection(server_side, handler));

        connect_and_handshake(&mut client_side).await;
        tx.send(GwpEvent::V1(EventV1::Log {
            record: "queued before the ping".into(),
        }))
        .await
        .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        write_frame(&mut client_side, &GwpRequest::V1(RequestV1::Ping))
            .await
            .unwrap();

        // The event and the reply may land in either order (idle flush
        // races the post-response drain); both must arrive.
        let mut pong = false;
        let mut record = None;
        for _ in 0..2 {
            let resp: GwpResponse = read_frame(&mut client_side).await.unwrap();
            let GwpResponse::V1(inner) = resp;
            match inner {
                ResponseV1::Pong => pong = true,
                ResponseV1::Event(EventV1::Log { record: r }) => record = Some(r),
                other => panic!("unexpected frame {other:?}"),
            }
        }
        assert!(pong, "the Pong never arrived");
        assert_eq!(record.as_deref(), Some("queued before the ping"));
    }

    #[tokio::test]
    async fn oversized_request_prefix_is_rejected_cleanly() {
        use tokio::io::AsyncWriteExt;
        let (server_side, mut client_side) = duplex(4096);
        let inbox = Arc::new(Mutex::new(Vec::new()));
        let (handler, _tx) = EmittingHandler::new(Arc::clone(&inbox));
        tokio::spawn(run_connection(server_side, handler));
        connect_and_handshake(&mut client_side).await;

        let evil = (MAX_FRAME_BYTES_U32 + 1).to_le_bytes();
        client_side.write_all(&evil).await.unwrap();
        // The connection errors out; the client sees EOF on the next read.
        let res: Result<GwpResponse, _> = read_frame(&mut client_side).await;
        assert!(res.is_err(), "the loop dropped the connection");
    }

    use crate::schema::MAX_FRAME_BYTES as MAX_FRAME_BYTES_U32;
}
