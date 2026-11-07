//! Transport-agnostic GWP connection state machine (architecture §7),
//! shared by the UDS and TCP transports: handshake gate, request
//! dispatch, bounded event flush between responses. The transports own
//! only their accept loop + client socket types.
//!
//! The server multiplexes one connection per client; each request is a
//! round-trip. Pushed events land on a shared queue the server flushes
//! between requests (bounded — slow clients degrade, never block the UI
//! thread, §15.6).

use std::future::Future;

use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc;

use crate::GwpError;
use crate::framing::{read_frame, write_frame};
use crate::schema::{
    ErrorCode, GwpEvent, GwpRequest, GwpResponse, PROTO_VERSION, RequestV1, ResponseV1,
};

/// A request handler: application logic behind the transport.
pub trait Handler: Send + 'static {
    fn handle(&mut self, request: RequestV1) -> impl Future<Output = ResponseV1> + Send;
}

/// Bounded outbound event queue (§15.6 backpressure: 64 messages default).
pub const EVENT_QUEUE_CAPACITY: usize = 64;

/// Run one client connection until `Bye` or EOF. Generic over the stream
/// so the UDS and TCP transports share the exact handshake + dispatch
/// semantics (identical bytes in, identical bytes out).
pub async fn run_connection<S, H>(mut stream: S, mut handler: H) -> Result<(), GwpError>
where
    S: AsyncRead + AsyncWrite + Unpin,
    H: Handler,
{
    // Handshake gate: first frame must be Hello with a known version (§7).
    let first: GwpRequest = read_frame(&mut stream).await?;
    let GwpRequest::V1(RequestV1::Hello {
        proto_version,
        agent,
    }) = first
    else {
        write_frame(
            &mut stream,
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
            &mut stream,
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
        &mut stream,
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

    // Event pump: events queued by the app side are flushed between
    // requests (bounded queue, §15.6).
    let (_event_tx, mut event_rx) = mpsc::channel::<GwpEvent>(EVENT_QUEUE_CAPACITY);
    loop {
        tokio::select! {
            event = event_rx.recv() => {
                if let Some(event) = event {
                    write_frame(&mut stream, &event).await?;
                }
            }
            req = read_frame::<GwpRequest, _>(&mut stream) => {
                let req = req?;
                // GwpRequest has a single (versioned) variant, so this is
                // irrefutable; the version check happened at handshake.
                let GwpRequest::V1(inner) = req;
                if matches!(inner, RequestV1::Bye) {
                    write_frame(&mut stream, &GwpResponse::V1(ResponseV1::Bye)).await?;
                    return Ok(());
                }
                let resp = handler.handle(inner).await;
                write_frame(&mut stream, &GwpResponse::V1(resp)).await?;
                // Opportunistically flush queued events after each response.
                while let Ok(event) = event_rx.try_recv() {
                    write_frame(&mut stream, &event).await?;
                }
            }
        }
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
