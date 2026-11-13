//! The client-side connection seam (architecture §7, M18): one trait for
//! the client half of a GWP conversation, implemented by every shipped
//! client — the UDS [`crate::uds::Client`], the TCP [`crate::tcp::Client`],
//! the WebSocket [`crate::ws::WsClient`] and the embedded
//! [`crate::duplex::Client`].
//!
//! The server-side mirror is [`crate::Handler`]: one seam per
//! conversation half. A consumer written against [`ClientConn`] — the
//! viewer's M18 live feed, a downstream recorder — runs over any door
//! unchanged, exactly the way a [`Handler`] runs behind any listener.
//!
//! The trait covers the conversation vocabulary only: handshake,
//! request, `next_event`, subscribe/unsubscribe, ping. `connect` stays
//! transport-owned (a socket path, an address, a duplex handle) — the §7
//! "the transport owns only its `connect`" rule, mirrored client-side.
//! Domain sugar rides the trait as default methods in terms of
//! [`Self::request`]: the `Window` targets (§16 v0.4) so a Rust consumer
//! gets the same typed surface the REPL (`window list` / `window use`)
//! and the TypeScript client (`windowList()` / `windowUse()`) speak —
//! every impl, shipped or downstream, inherits them for free.

use std::future::Future;

use crate::GwpError;
use crate::schema::{
    AdapterInfoV1, EventKindV1, GwpEvent, RequestV1, ResponseV1, WindowInfoV1, WindowV1,
};

/// The client half of a GWP conversation, transport-blind (§7, M18).
///
/// The four shipped clients implement this one-for-one over the shared
/// [`crate::conn::ClientCore`]; downstream transports that wrap a
/// connected stream (TLS, named pipes, an in-memory mesh) implement it
/// the same way — the conversation never changes.
pub trait ClientConn: Send + 'static {
    /// Send `Hello` and validate the driver's `Welcome` (§7).
    fn handshake(
        &mut self,
        agent: &str,
    ) -> impl Future<Output = Result<AdapterInfoV1, GwpError>> + Send;

    /// One request round-trip (§7): the response to `request`, with any
    /// interleaved event frames buffered for [`Self::next_event`].
    fn request(
        &mut self,
        request: RequestV1,
    ) -> impl Future<Output = Result<ResponseV1, GwpError>> + Send;

    /// The next pushed event (§7, M12): from the buffer of events that
    /// interleaved earlier responses, or straight off the wire. Blocks
    /// until the driver publishes one.
    fn next_event(&mut self) -> impl Future<Output = Result<GwpEvent, GwpError>> + Send;

    /// Subscribe this connection to the named event kinds (§7, M12).
    /// Re-subscribing replaces the filter; the ack echoes the kinds the
    /// driver honored.
    fn subscribe(
        &mut self,
        kinds: &[EventKindV1],
    ) -> impl Future<Output = Result<(), GwpError>> + Send;

    /// Drop this connection's event subscription (§7, M12).
    fn unsubscribe(&mut self) -> impl Future<Output = Result<(), GwpError>> + Send;

    /// One `Ping` round-trip — the liveness probe (§7).
    fn ping(&mut self) -> impl Future<Output = Result<(), GwpError>> + Send;

    /// The session's window targets (§16 v0.4, M6): adapter-reported
    /// metadata (handle / title / focus), one entry per window — the
    /// same list `window list` prints and `windowList()` returns.
    /// Single-window adapters report exactly one `main` target.
    fn window_list(&mut self) -> impl Future<Output = Result<Vec<WindowInfoV1>, GwpError>> + Send {
        async move {
            match self.request(RequestV1::Window(WindowV1::List)).await? {
                ResponseV1::Windows { windows } => Ok(windows),
                other => Err(GwpError::Protocol(format!(
                    "Window::List answered by {other:?}"
                ))),
            }
        }
    }

    /// Aim this connection at a window target (§16 v0.4, M6): subsequent
    /// resolves on this connection see only that window's subtree — the
    /// per-connection state the M6 row records. The ack echoes the aimed
    /// handle; an unknown handle (never existed or closed) is the typed
    /// `Rejected { code: ErrorCode::UnknownWindow }`.
    fn window_use(
        &mut self,
        handle: impl Into<String> + Send,
    ) -> impl Future<Output = Result<String, GwpError>> + Send {
        async move {
            let handle = handle.into();
            match self
                .request(RequestV1::Window(WindowV1::Use {
                    handle: handle.clone(),
                }))
                .await?
            {
                ResponseV1::WindowUsed { handle: aimed } => Ok(aimed),
                other => Err(GwpError::Protocol(format!(
                    "Window::Use answered by {other:?}"
                ))),
            }
        }
    }
}

impl ClientConn for crate::uds::Client {
    fn handshake(
        &mut self,
        agent: &str,
    ) -> impl Future<Output = Result<AdapterInfoV1, GwpError>> + Send {
        self.handshake(agent)
    }

    fn request(
        &mut self,
        request: RequestV1,
    ) -> impl Future<Output = Result<ResponseV1, GwpError>> + Send {
        self.request(request)
    }

    fn next_event(&mut self) -> impl Future<Output = Result<GwpEvent, GwpError>> + Send {
        self.next_event()
    }

    fn subscribe(
        &mut self,
        kinds: &[EventKindV1],
    ) -> impl Future<Output = Result<(), GwpError>> + Send {
        self.subscribe(kinds)
    }

    fn unsubscribe(&mut self) -> impl Future<Output = Result<(), GwpError>> + Send {
        self.unsubscribe()
    }

    fn ping(&mut self) -> impl Future<Output = Result<(), GwpError>> + Send {
        self.ping()
    }
}

impl ClientConn for crate::tcp::Client {
    fn handshake(
        &mut self,
        agent: &str,
    ) -> impl Future<Output = Result<AdapterInfoV1, GwpError>> + Send {
        self.handshake(agent)
    }

    fn request(
        &mut self,
        request: RequestV1,
    ) -> impl Future<Output = Result<ResponseV1, GwpError>> + Send {
        self.request(request)
    }

    fn next_event(&mut self) -> impl Future<Output = Result<GwpEvent, GwpError>> + Send {
        self.next_event()
    }

    fn subscribe(
        &mut self,
        kinds: &[EventKindV1],
    ) -> impl Future<Output = Result<(), GwpError>> + Send {
        self.subscribe(kinds)
    }

    fn unsubscribe(&mut self) -> impl Future<Output = Result<(), GwpError>> + Send {
        self.unsubscribe()
    }

    fn ping(&mut self) -> impl Future<Output = Result<(), GwpError>> + Send {
        self.ping()
    }
}

impl ClientConn for crate::ws::WsClient {
    fn handshake(
        &mut self,
        agent: &str,
    ) -> impl Future<Output = Result<AdapterInfoV1, GwpError>> + Send {
        self.handshake(agent)
    }

    fn request(
        &mut self,
        request: RequestV1,
    ) -> impl Future<Output = Result<ResponseV1, GwpError>> + Send {
        self.request(request)
    }

    fn next_event(&mut self) -> impl Future<Output = Result<GwpEvent, GwpError>> + Send {
        self.next_event()
    }

    fn subscribe(
        &mut self,
        kinds: &[EventKindV1],
    ) -> impl Future<Output = Result<(), GwpError>> + Send {
        self.subscribe(kinds)
    }

    fn unsubscribe(&mut self) -> impl Future<Output = Result<(), GwpError>> + Send {
        self.unsubscribe()
    }

    fn ping(&mut self) -> impl Future<Output = Result<(), GwpError>> + Send {
        self.ping()
    }
}

impl ClientConn for crate::duplex::Client {
    fn handshake(
        &mut self,
        agent: &str,
    ) -> impl Future<Output = Result<AdapterInfoV1, GwpError>> + Send {
        self.handshake(agent)
    }

    fn request(
        &mut self,
        request: RequestV1,
    ) -> impl Future<Output = Result<ResponseV1, GwpError>> + Send {
        self.request(request)
    }

    fn next_event(&mut self) -> impl Future<Output = Result<GwpEvent, GwpError>> + Send {
        self.next_event()
    }

    fn subscribe(
        &mut self,
        kinds: &[EventKindV1],
    ) -> impl Future<Output = Result<(), GwpError>> + Send {
        self.subscribe(kinds)
    }

    fn unsubscribe(&mut self) -> impl Future<Output = Result<(), GwpError>> + Send {
        self.unsubscribe()
    }

    fn ping(&mut self) -> impl Future<Output = Result<(), GwpError>> + Send {
        self.ping()
    }
}
