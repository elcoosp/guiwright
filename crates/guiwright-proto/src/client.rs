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
use std::time::{Duration, Instant};

use guiwright_core::Selector;

use crate::GwpError;
use crate::schema::{
    AdapterInfoV1, DescribeAllV1, DescribeV1, DescribedV1, EventKindV1, EventV1, GwpEvent,
    NodeKeyV1, RequestV1, ResolvedMatchV1, ResponseV1, SelectorV1, WindowInfoV1, WindowV1,
};
use crate::wait::{Eval, WaitState, WaitTracker};

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

    /// The §8.2 set read over the wire (M30 — the m22/m26/m29 records'
    /// named protocol half): every current match of `selector`, document
    /// order — the kit's `count()`/`all()` walk, non-strict and never
    /// waiting, so an empty set answers with an empty list. Rows are
    /// directly usable as `RequestV1::Action` handles (the dump-row
    /// rule, M11). The connection's window scope (§16 v0.4) narrows the
    /// walk exactly like the resolves [`Self::window_use`] aims.
    fn resolve_all(
        &mut self,
        selector: SelectorV1,
    ) -> impl Future<Output = Result<Vec<ResolvedMatchV1>, GwpError>> + Send {
        async move {
            match self.request(RequestV1::ResolveAll { selector }).await? {
                ResponseV1::ResolvedAll { matches } => Ok(matches),
                other => Err(GwpError::Protocol(format!(
                    "ResolveAll answered by {other:?}"
                ))),
            }
        }
    }

    /// The hit-test read over the wire (M32): which node occupies the
    /// point — window-relative physical px, the same space the tree's
    /// bounds speak (the dump rows' `bounds`, the resolve rects), so a
    /// client can hit inside a rect it just resolved. The connection's
    /// window scope (§16 v0.4) narrows the answer exactly like the
    /// resolves [`Self::window_use`] aims; empty space answers `None` —
    /// the read is total, never an error. The hit row is directly usable
    /// as an [`RequestV1::Action`] handle (the dump-row rule, M11).
    fn hit_test(
        &mut self,
        x: f64,
        y: f64,
    ) -> impl Future<Output = Result<Option<ResolvedMatchV1>, GwpError>> + Send {
        async move {
            match self.request(RequestV1::HitTest { x, y }).await? {
                ResponseV1::HitTest { hit } => Ok(hit),
                other => Err(GwpError::Protocol(format!("HitTest answered by {other:?}"))),
            }
        }
    }

    /// The node-describe read over the wire (M36 — the m32 record's
    /// standing gap): one node's identity facts — role, accessible name,
    /// value, bounds — for a handle the client already holds (a
    /// [`Self::resolve_all`] row, a hit row, a dump row: the dump-row
    /// rule, M11). Handle-addressed, so no window scope applies (the
    /// [`RequestV1::Action`] boundary); the generation guard rejects a
    /// recycled id (`ErrorCode::StaleGeneration`) and a dead one
    /// (`ErrorCode::UnknownNode`) as the typed `Error` — the same
    /// honesty an [`RequestV1::Action`] on the row would meet.
    fn describe(
        &mut self,
        node: NodeKeyV1,
        generation: u32,
    ) -> impl Future<Output = Result<DescribedV1, GwpError>> + Send {
        async move {
            match self
                .request(RequestV1::Describe(DescribeV1 { node, generation }))
                .await?
            {
                ResponseV1::Described(described) => Ok(described),
                other => Err(GwpError::Protocol(format!(
                    "Describe answered by {other:?}"
                ))),
            }
        }
    }

    /// The describe-all read over the wire (M39 — the m37 record's
    /// standing gap): every current match of `selector` carrying its
    /// identity facts — the [`Self::resolve_all`] walk's rows plus the
    /// [`Self::describe`] facts, one round trip instead of N. Non-strict
    /// and never waiting, so an empty set answers with an empty list.
    /// Rows are directly usable as `RequestV1::Action` handles (the
    /// dump-row rule, M11). The connection's window scope (§16 v0.4)
    /// narrows the walk exactly like [`Self::resolve_all`].
    fn describe_all(
        &mut self,
        selector: SelectorV1,
    ) -> impl Future<Output = Result<Vec<DescribedV1>, GwpError>> + Send {
        async move {
            match self
                .request(RequestV1::DescribeAll(DescribeAllV1 { selector }))
                .await?
            {
                ResponseV1::DescribedAll { rows } => Ok(rows),
                other => Err(GwpError::Protocol(format!(
                    "DescribeAll answered by {other:?}"
                ))),
            }
        }
    }

    /// The client-side wait (m42 — the m21 record's named follow-up):
    /// Playwright's `locator.waitFor(state)` for remote suites. Composes
    /// the §7 event channel with the M39 describe-all re-reads — subscribe
    /// to `TreeChanged`/`FramePresented`, evaluate the state against the
    /// rows (the [`crate::wait::WaitTracker`] rules), and re-read on every
    /// wake until the state holds or `timeout` expires. No wire change:
    /// the wait rides existing domains byte-for-byte; every
    /// [`ClientConn`] impl inherits it.
    ///
    /// **Answers.** The positive states answer with the row that
    /// satisfied them — directly usable as an `RequestV1::Action` handle
    /// (the dump-row rule, M11). `Hidden` answers `Ok(None)`: absence is
    /// the satisfying fact. The timeout is the typed
    /// [`GwpError::WaitTimeout`], naming the state, the locator and the
    /// last evaluation — never a wire answer.
    ///
    /// **Subscription ownership.** The wait installs the
    /// `TreeChanged` + `FramePresented` filter for its duration and
    /// drops it on return, both on success and on expiry (a consumer
    /// that never drains its events must not stay subscribed — the
    /// driver only pumps at frame cadence while subscribers exist). A
    /// connection running its own long-lived subscription (the viewer's
    /// live feed) should not hand the same connection to a wait: the
    /// one-event-consumer rule the §7 channel already states. Events
    /// that interleave the wait's own requests are buffered by the
    /// connection state machine and consumed by the wait itself, so no
    /// wake is lost mid-wait.
    ///
    /// # Errors
    ///
    /// Protocol failures (`subscribe`, the describe-all round trips,
    /// `next_event`) propagate as-is — a driver that predates M39
    /// answers `DescribeAll` with the typed `Unsupported` and the wait
    /// surfaces that rejection rather than degrading (the m41 fail-fast
    /// rule). A spent budget answers [`GwpError::WaitTimeout`].
    fn wait_for(
        &mut self,
        selector: SelectorV1,
        state: WaitState,
        timeout: Duration,
    ) -> impl Future<Output = Result<Option<DescribedV1>, GwpError>> + Send {
        async move {
            // The locator string for the timeout line (the kit's render
            // vocabulary — `getByRole(Button, "Save")` — one conversion,
            // not one per re-read).
            let rendered = Selector::from(selector.clone()).render();

            // The wait owns the event filter for its duration. A driver
            // that rejects Log-less subscriptions answers the same
            // `Subscribed` ack every door speaks.
            self.subscribe(&[EventKindV1::TreeChanged, EventKindV1::FramePresented])
                .await?;

            let deadline = tokio::time::Instant::from_std(Instant::now() + timeout);
            let mut tracker = WaitTracker::new(state);
            // The entry read is a tree-time evaluation; frame wakes re-pin
            // and — only for `Stable` — confirm.
            let mut wake = Wake::Tree;
            loop {
                let rows = self.describe_all(selector.clone()).await?;
                let eval = match wake {
                    Wake::Tree => tracker.evaluate(&rows),
                    Wake::Frame => tracker.observe_frame(&rows),
                };
                match eval {
                    Eval::Satisfied(row) => {
                        self.unsubscribe().await?;
                        return Ok(row);
                    }
                    Eval::Pending { reason } => {
                        if tokio::time::Instant::now() >= deadline {
                            return Err(expire(self, state, rendered, timeout, reason).await);
                        }
                        // The event wait: a tree wake re-decides the state,
                        // a frame wake may confirm `Stable`, `Log` records
                        // are never wakes, and a spent budget answers
                        // `None` — the expiry path below.
                        let next = loop {
                            tokio::select! {
                                event = self.next_event() => match event? {
                                    GwpEvent::V1(EventV1::TreeChanged { .. }) => {
                                        break Some(Wake::Tree)
                                    }
                                    GwpEvent::V1(EventV1::FramePresented { .. }) => {
                                        break Some(Wake::Frame)
                                    }
                                    _ => continue,
                                },
                                _ = tokio::time::sleep_until(deadline) => break None,
                            }
                        };
                        let Some(next_wake) = next else {
                            return Err(expire(self, state, rendered, timeout, reason).await);
                        };
                        wake = next_wake;
                    }
                }
            }
        }
    }
}

/// Which event ended the last event-wait: a tree change re-decides the
/// state, a presented frame may confirm `Stable`. `Log` records are
/// never wakes; expiry is `None` and ends the wait.
enum Wake {
    Tree,
    Frame,
}

/// Build the typed timeout and drop the wait's subscription (best
/// effort — the timeout outcome is the answer; a failed cleanup leaves
/// the filter on, surfaced by the next conversation).
async fn expire<C: ClientConn + ?Sized>(
    client: &mut C,
    state: WaitState,
    selector: String,
    timeout: Duration,
    last: String,
) -> GwpError {
    let outcome = GwpError::WaitTimeout {
        state: state.describe(),
        selector,
        timeout_ms: u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX),
        last,
    };
    let _ = client.unsubscribe().await;
    outcome
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
