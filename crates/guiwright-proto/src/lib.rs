//! Guiwright Protocol (GWP) — the wire between apps and clients (§7).
//!
//! * [`schema`] — versioned postcard schema (`GwpRequest::V1`, …)
//! * [`framing`] — 4-byte LE length prefix + postcard body, size-guarded
//! * [`blob`] — LZ4 side-channel for large payloads (frames/trees/traces)
//! * [`uds`] — default transport: server loop + client
//! * [`tcp`] — loopback/container transport for other-language clients
//! * [`ws`] — the viewer live-mode door: RFC 6455 server + client (M15)
//! * [`duplex`] — the embedded door: in-process pipes + client (M16), the
//!   reference shape for downstream [`Handler`]-based transports
//! * [`conn`] — the transport-agnostic connection state machine all doors share
//! * [`client`] — the client-side connection seam: one [`ClientConn`]
//!   trait every shipped client implements (M18), the mirror of [`Handler`]
//! * [`wait`] — the client-side wait (m42): the `waitFor(state)` row-set
//!   evaluator behind [`ClientConn::wait_for`], no wire change
//! * [`bulk`] — the bulk-content derivations (m46): the kit's m29/m31
//!   source orders over the describe-all rows, behind
//!   [`ClientConn::all_text_contents`] / [`ClientConn::all_values`]
//! * [`expect`] — the client-side content assertions (m54): the kit's
//!   `to_have_text` / `to_have_value` rows over any door, behind
//!   [`ClientConn::expect_text`] / [`ClientConn::expect_value`], no wire
//!   change
//! * [`count`] — the client-side count assertion (m59): the kit's
//!   `to_have_count` family (the M22 exact form + the M25 comparators)
//!   over any door, behind [`ClientConn::expect_count`] /
//!   [`ClientConn::expect_count_greater_than`] /
//!   [`ClientConn::expect_count_less_than`], no wire change
//! * [`schemadoc`] — the published, machine-readable wire schema (§16)
//!
//! Anything larger than a frame rides the blob side-channel so command
//! latency stays independent of payload size (§15.6).

#![forbid(unsafe_code)]

pub mod blob;
pub mod bulk;
pub mod client;
pub mod conn;
pub mod convert;
pub mod count;
pub mod duplex;
pub mod expect;
pub mod framing;
pub mod schema;
pub mod schemadoc;
pub mod tcp;
pub mod uds;
pub mod wait;
pub mod ws;

pub use client::ClientConn;
pub use conn::Handler;
pub use duplex::Client as DuplexClient;
pub use expect::ExpectKind;
pub use schema::{
    AdapterInfoV1, CapsV1, ErrorCode, EventV1, GwpEvent, GwpRequest, GwpResponse, InputEventV1,
    NodeKeyV1, NthV1, PROTO_VERSION, PointerButtonV1, RequestV1, ResponseV1, SelectorV1,
    SemActionV1, TextMatchV1, VIDEO_DEFAULT_FPS, VideoOptsV1, VideoV1,
};
pub use tcp::Client as TcpClient;
use thiserror::Error;
pub use uds::Client;
pub use wait::WaitState;
pub use ws::WsClient;

/// Transport and protocol errors.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum GwpError {
    #[error("postcard codec: {0}")]
    Codec(#[from] postcard::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("frame body of {0} bytes exceeds the guard")]
    FrameTooLarge(usize),
    #[error("blob: {0}")]
    Blob(String),
    #[error("handshake failed: {0}")]
    Handshake(&'static str),
    #[error("driver rejected the request ({code:?}): {message}")]
    Rejected { code: ErrorCode, message: String },
    #[error("protocol misuse: {0}")]
    Protocol(String),
    /// The client-side wait expired (m42): the state it waited for, the
    /// locator, the budget it had and the last evaluation — the kit's
    /// "failures name names" discipline (§8.3) carried over the wire.
    /// Never a wire answer: the timeout is decided client-side (the wire
    /// `ErrorCode::Timeout` stays the driver's resolve/action answer), so
    /// the vocabulary is a client-side addition, not a schema change.
    #[error("wait {state} timed out after {timeout_ms} ms — {selector} ({last})")]
    WaitTimeout {
        state: &'static str,
        selector: String,
        timeout_ms: u64,
        last: String,
    },
}
