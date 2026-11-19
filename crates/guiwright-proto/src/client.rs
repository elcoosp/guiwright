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
    AdapterInfoV1, DescribeAllV1, DescribeV1, DescribedV1, EventKindV1, EventV1, GestureV1,
    GwpEvent, NodeKeyV1, RequestV1, ResolvedMatchV1, ResponseV1, SelectorV1, SemActionV1,
    SnapshotNodeV1, SnapshotV1, WindowInfoV1, WindowV1,
};
use crate::wait::{Eval, WaitState, WaitTracker};

/// The receipt a [`ClientConn::action`] answers (m47): the frame the
/// action applied at (clients can await exactly this frame — the §7
/// multiplexing rule) and the adapter's detail string (`"clicked"`, the
/// same line the REPL's action rows render). A client-side carrier for
/// the wire `ActionDone` payload — never serialized itself, so the
/// schema stays byte-pinned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActionReceipt {
    /// The frame the action was applied at.
    pub frame_seq: u64,
    /// The adapter's detail string (the `ActionDone` detail).
    pub detail: String,
}

/// The receipt a [`ClientConn::gesture`] answers (m48): the frame the
/// synthesized batch applied at (the §15.4 rendezvous — clients can
/// await exactly this frame) and the synthesis summary (`"clicked at
/// (76, 32) — 3 events"`, the same line the REPL's gesture rows render).
/// A client-side carrier for the wire `GestureDone` payload — never
/// serialized itself, so the schema stays byte-pinned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GestureReceipt {
    /// The frame the gesture was applied at.
    pub frame_seq: u64,
    /// The synthesis summary (the `GestureDone` detail).
    pub detail: String,
}

/// The client-side carrier for the wire `TreeDump` reply (m49):
/// field-for-field the M11/M13 payload — the tree generation the dump was
/// taken at, the dumped node count, the focused node and the depth-first
/// rows. Never serialized itself (the reply rides `ResponseV1::TreeDump`
/// byte-for-byte), so the schema stays byte-pinned.
#[derive(Clone, Debug, PartialEq)]
pub struct TreeDump {
    /// The tree generation the dump was taken at (the same seq space the
    /// `ResponseV1::TreeSnapshot` reply reports).
    pub tree_seq: u64,
    /// Dumped node count (== `nodes.len()`, redundancy for readers).
    pub node_count: usize,
    /// The focused node, when the tree has one.
    pub focus: Option<NodeKeyV1>,
    /// Depth-first per window root (§16 v0.4): every window's subtree
    /// appears once, roots in adapter-reported window order.
    pub nodes: Vec<SnapshotNodeV1>,
}

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

    /// The kit's bulk text read over the wire (m46 — the m41 record's
    /// Rust-client half, closed): every current match of `selector` with
    /// its content derived client-side from the [`Self::describe_all`]
    /// rows — the m29 `all_text_contents()` order (accessible name
    /// first, value as the fallback), the read the text locators match.
    /// One round trip instead of one [`Self::describe`] per row: the
    /// walk's rows already carry the name and the value, so the read
    /// costs what [`Self::describe_all`] costs.
    ///
    /// **No wire change and no fallback** (the m41 fail-fast rule, the
    /// [`crate::bulk`] module's boundary): the rows ride `DescribeAll`
    /// byte-for-byte as M39 blessed them; a driver that predates M39
    /// answers the typed `Unsupported` and this read surfaces that
    /// rejection rather than degrading — the REPL's per-row fallback is
    /// the interactive surface's concession, never this one's. Snapshot
    /// semantics, unchanged from the `resolve_all` family: no strict
    /// mode, no waiting, an empty set answers an empty list, hidden
    /// nodes ride the walk, and the connection's window scope narrows
    /// the read exactly like [`Self::describe_all`].
    ///
    /// Every [`ClientConn`] impl inherits the derivation (a default
    /// method over [`Self::describe_all`]), the same way the TypeScript
    /// client carries `allTextContents()` (m41) and the REPL carries
    /// `texts` (m40): one vocabulary, every surface, one round trip.
    fn all_text_contents(
        &mut self,
        selector: SelectorV1,
    ) -> impl Future<Output = Result<Vec<String>, GwpError>> + Send {
        async move {
            let rows = self.describe_all(selector).await?;
            Ok(rows.iter().map(crate::bulk::text_content).collect())
        }
    }

    /// The kit's form-dump read over the wire (m46 — the m31 kit
    /// reader's client half): [`Self::all_text_contents`] with the
    /// source order flipped — the value first, the accessible name as
    /// the fallback (the `getByValue` source order, the m31 rule) — so
    /// the fixture's Email input answers `"ada@example.dev"` here and
    /// `"Email"` there, and the two reads can never silently agree on
    /// the wrong datum. A node with neither fact reads as the empty
    /// string, keeping `Vec` positions 1:1 with the document order the
    /// set read established.
    ///
    /// Everything else is the [`Self::all_text_contents`] contract,
    /// unchanged: the [`Self::describe_all`] round trip as M39 blessed
    /// it (no wire change, no fallback — a pre-M39 driver answers the
    /// typed `Unsupported`), the snapshot-read family rules, and the
    /// connection's window scope.
    fn all_values(
        &mut self,
        selector: SelectorV1,
    ) -> impl Future<Output = Result<Vec<String>, GwpError>> + Send {
        async move {
            let rows = self.describe_all(selector).await?;
            Ok(rows.iter().map(crate::bulk::value_content).collect())
        }
    }

    /// The §7 locate half as typed seam sugar (m47): one strict-mode
    /// [`RequestV1::Resolve`] round trip — the driver resolves with its
    /// own auto-wait (`timeout_ms` budget, the §8.3 attached gate on the
    /// session thread, no polling) and answers the row the kit's `get`
    /// step resolves: handle, generation, bounds. The row is directly
    /// usable as a [`RequestV1::Action`] node (the dump-row rule, M11)
    /// and pairs with [`Self::action`] — the §7 locate-and-act loop in
    /// two typed calls, no raw request.
    ///
    /// Strict mode is the wire's own policy (§8.2): an ambiguous
    /// selector answers the typed `Rejected { code:
    /// ErrorCode::StrictModeViolation }`, a selector that never matches
    /// inside the budget answers `ErrorCode::Timeout` naming the stuck
    /// check. The connection's window scope (§16 v0.4) narrows the
    /// resolve exactly like [`Self::resolve_all`] — one target per
    /// window. No wire change: the row rides the `Resolve` domain
    /// byte-for-byte as M0 blessed it; every [`ClientConn`] impl
    /// inherits the sugar, the same shape the TypeScript client has
    /// spoken since M3 (`resolve()`).
    fn resolve(
        &mut self,
        selector: SelectorV1,
        timeout_ms: u64,
    ) -> impl Future<Output = Result<ResolvedMatchV1, GwpError>> + Send {
        async move {
            match self
                .request(RequestV1::Resolve {
                    selector,
                    timeout_ms,
                })
                .await?
            {
                ResponseV1::Resolved {
                    node,
                    generation,
                    rect,
                } => Ok(ResolvedMatchV1 {
                    node,
                    generation,
                    rect,
                }),
                other => Err(GwpError::Protocol(format!("Resolve answered by {other:?}"))),
            }
        }
    }

    /// The §7 act half as typed seam sugar (m47): one [`RequestV1::Action`]
    /// round trip applying `sem` on a row the client already holds — a
    /// [`Self::resolve`] answer, a [`Self::resolve_all`] / [`Self::describe_all`]
    /// row, a [`Self::hit_test`] hit (the dump-row rule, M11). The
    /// driver re-checks the pinned node at act time (`force` skips
    /// exactly stability + hit-test, the M17 [`RequestV1::Action`]
    /// contract) within `timeout_ms` and applies it at a frame
    /// rendezvous, answering the [`ActionReceipt`] — the frame the
    /// action applied at and the adapter's detail. No wire change: the
    /// act rides the `Action` domain byte-for-byte as M17 blessed it;
    /// every [`ClientConn`] impl inherits the sugar, the same shape the
    /// TypeScript client has spoken since M3 (`action()`).
    ///
    /// The Action boundary's honesty carries over unchanged: a row whose
    /// generation has moved on answers `ErrorCode::StaleGeneration`, a
    /// dead one the boundary's since-M0 `NoMatch` mapping — the typed
    /// `ErrorCode::StrictModeViolation` whose message names the dead
    /// handle (the `Describe` boundary's `ErrorCode::UnknownNode` is
    /// that read's own contract) — and a node failing its act-time gate
    /// answers `ErrorCode::Timeout` naming the stuck check (the m17
    /// record's named error).
    fn action(
        &mut self,
        node: NodeKeyV1,
        generation: u32,
        sem: SemActionV1,
        force: bool,
        timeout_ms: u64,
    ) -> impl Future<Output = Result<ActionReceipt, GwpError>> + Send {
        async move {
            match self
                .request(RequestV1::Action {
                    node,
                    generation,
                    action: sem,
                    force,
                    timeout_ms,
                })
                .await?
            {
                ResponseV1::ActionDone { frame_seq, detail } => {
                    Ok(ActionReceipt { frame_seq, detail })
                }
                other => Err(GwpError::Protocol(format!("Action answered by {other:?}"))),
            }
        }
    }

    /// The §9 input half as typed seam sugar (m48): one [`RequestV1::Gesture`]
    /// round trip — the driver synthesizes the raw `InputEventV1` batch from
    /// its adapter's pacing table (click = move→down→up; drag = the §9
    /// ratchet-interpolated moves; key = down→up; type = one text commit) and
    /// injects it through the target window's pipeline, applying the batch
    /// inside the next frame. Answers the [`GestureReceipt`] — the frame the
    /// batch applied at plus the synthesis summary the REPL's gesture rows
    /// render. No wire change: the synthesis rides the `Gesture` domain
    /// byte-for-byte as M7 blessed it; every [`ClientConn`] impl inherits
    /// the sugar, the same shape the TypeScript client has spoken since M7
    /// (`gesture()`) and the REPL since M7 (`mouse` / `key` / `text`).
    ///
    /// The connection's window scope (§16 v0.4) steers the coordinates (the
    /// M6 handoff the §9 grammar rides) — the batch lands in the aimed
    /// window's pipeline, the same scope [`Self::resolve`] aims the walk
    /// through. The boundary's honesty carries over unchanged: a gesture
    /// that synthesizes nothing (a zero-count click — the session's
    /// client-bug rejection) answers the wire's typed error naming the
    /// cause; the raw `RequestV1::Input` batch stays the un-synthesized
    /// escape hatch, exactly where M7 left it.
    fn gesture(
        &mut self,
        gesture: GestureV1,
    ) -> impl Future<Output = Result<GestureReceipt, GwpError>> + Send {
        async move {
            match self.request(RequestV1::Gesture(gesture)).await? {
                ResponseV1::GestureDone { frame_seq, detail } => {
                    Ok(GestureReceipt { frame_seq, detail })
                }
                other => Err(GwpError::Protocol(format!("Gesture answered by {other:?}"))),
            }
        }
    }

    /// The §7 tree-dump domain as typed seam sugar (m49 — the m13 record's
    /// dump-row half): one `RequestV1::Snapshot(SnapshotV1::Full)` round
    /// trip — the whole logical tree, depth-first per window root (§16
    /// v0.4), rebuilt into the [`TreeDump`] carrier. The walk is the M11
    /// contract the REPL's `snapshot` row prints and the TypeScript
    /// client's `treeDump()` carries: every row carries the identity facts
    /// (role, name, value, bounds) and the `children` counts rebuild the
    /// hierarchy without a parent pointer per row. Not window-scoped by
    /// design — the full walk covers every window target, disjoint by
    /// construction ([`Self::tree_dump_under`] is the scoped read). No
    /// wire change: the walk rides the `Snapshot` domain byte-for-byte as
    /// M11 blessed it; every [`ClientConn`] impl inherits the sugar.
    fn tree_dump(&mut self) -> impl Future<Output = Result<TreeDump, GwpError>> + Send {
        async move {
            match self.request(RequestV1::Snapshot(SnapshotV1::Full)).await? {
                ResponseV1::TreeDump {
                    tree_seq,
                    node_count,
                    focus,
                    nodes,
                } => Ok(TreeDump {
                    tree_seq,
                    node_count,
                    focus,
                    nodes,
                }),
                other => Err(GwpError::Protocol(format!(
                    "TreeDump answered by {other:?}"
                ))),
            }
        }
    }

    /// One subtree as typed seam sugar (m49 — the m13 `Snapshot::Filtered`
    /// arm): resolve `selector` (the driver's auto-wait `Attached` within
    /// `timeout_ms`, honoring the connection's window scope — the same
    /// resolve [`Self::resolve`] runs) and dump the depth-first walk
    /// rooted at the match — a leaf locator answers one row, itself. The
    /// resolve's honesty rides the sugar unchanged: a selector that never
    /// matches inside the budget answers `ErrorCode::Timeout` naming the
    /// stuck check; strict mode is the wire's own policy (§8.2), so an
    /// ambiguous locator answers `ErrorCode::StrictModeViolation`. No wire
    /// change: the subtree walk rides the `Snapshot` domain byte-for-byte
    /// as M13 blessed it; every [`ClientConn`] impl inherits the sugar,
    /// the same shape the TypeScript client's `treeDumpUnder()` has
    /// spoken since M13.
    fn tree_dump_under(
        &mut self,
        selector: SelectorV1,
        timeout_ms: u64,
    ) -> impl Future<Output = Result<TreeDump, GwpError>> + Send {
        async move {
            match self
                .request(RequestV1::Snapshot(SnapshotV1::Filtered {
                    selector,
                    timeout_ms,
                }))
                .await?
            {
                ResponseV1::TreeDump {
                    tree_seq,
                    node_count,
                    focus,
                    nodes,
                } => Ok(TreeDump {
                    tree_seq,
                    node_count,
                    focus,
                    nodes,
                }),
                other => Err(GwpError::Protocol(format!(
                    "TreeDump answered by {other:?}"
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
                                event = self.next_event() => {
                                    // `GwpEvent` has the single `V1`
                                    // variant, so this bind is
                                    // irrefutable (the version check
                                    // pattern the connection loop uses).
                                    let GwpEvent::V1(event) = event?;
                                    match event {
                                        EventV1::TreeChanged { .. } => {
                                            break Some(Wake::Tree)
                                        }
                                        EventV1::FramePresented { .. } => {
                                            break Some(Wake::Frame)
                                        }
                                        // `Log` is never a wake: fall
                                        // through to the next wake
                                        // window (the loop body's tail).
                                        EventV1::Log { .. } => {}
                                    }
                                }
                                () = tokio::time::sleep_until(deadline) => break None,
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
