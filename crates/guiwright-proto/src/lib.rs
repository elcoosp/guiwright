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
//! * [`schemadoc`] — the published, machine-readable wire schema (§16)
//!
//! Anything larger than a frame rides the blob side-channel so command
//! latency stays independent of payload size (§15.6).

#![forbid(unsafe_code)]

pub mod blob;
pub mod conn;
pub mod convert;
pub mod duplex;
pub mod framing;
pub mod schema;
pub mod schemadoc;
pub mod tcp;
pub mod uds;
pub mod ws;

pub use conn::Handler;
pub use duplex::Client as DuplexClient;
pub use schema::{
    AdapterInfoV1, CapsV1, ErrorCode, EventV1, GwpEvent, GwpRequest, GwpResponse, InputEventV1,
    NodeKeyV1, NthV1, PROTO_VERSION, PointerButtonV1, RequestV1, ResponseV1, SelectorV1,
    SemActionV1, TextMatchV1, VIDEO_DEFAULT_FPS, VideoOptsV1, VideoV1,
};
pub use tcp::Client as TcpClient;
pub use uds::Client;
pub use uds::Client as UdsClient;
pub use ws::WsClient;

use thiserror::Error;

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
}
