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
use crate::count::{CountEval, CountTracker};
use crate::expect::{ExpectEval, ExpectKind, ExpectTracker};
use crate::schema::{
    AdapterInfoV1, ClockV1, DescribeAllV1, DescribeV1, DescribedV1, EventKindV1, EventV1,
    GestureV1, GwpEvent, InputEventV1, InspectV1, MockV1, NodeKeyV1, RequestV1, ResolvedMatchV1,
    ResponseV1, ScreenshotV1, SelectorV1, SemActionV1, SnapshotNodeV1, SnapshotV1, TextMatchV1,
    VideoV1, WindowInfoV1, WindowV1,
};
use crate::wait::{Eval, WaitState, WaitTracker};
use guiwright_core::wait::CountMatch;

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

/// The client-side carrier for the wire `Shot` reply (m50): field-for-field
/// the M10 payload — the session frame the pixels came from, the pixel
/// dimensions and the PNG bytes. Never serialized itself (the reply rides
/// `ResponseV1::Shot` byte-for-byte; PNG is already losslessly compressed,
/// the §15.6 LZ4 blob stage would double-encode without shrinking it), so
/// the schema stays byte-pinned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shot {
    /// The session frame the pixels came from (the same seq space the
    /// [`ActionReceipt::frame_seq`] reports — clients can correlate).
    pub frame_seq: u64,
    /// Pixel width of `png` (the full frame for a window shot, the crop
    /// width for an element shot).
    pub width: u32,
    /// Pixel height of `png`.
    pub height: u32,
    /// PNG-encoded RGBA8 (lossless, §10 "Output: PNG").
    pub png: Vec<u8>,
}

/// The client-side carrier for the wire `MockDone` reply (m52):
/// field-for-field the §14 payload — the seam id (or fixture scope) the
/// reply names and the detail the hook or canned override produced. Never
/// serialized itself (the reply rides `ResponseV1::MockDone` byte-for-byte),
/// so the schema stays byte-pinned.
///
/// The wire reuses one payload for the whole §14 domain: a `ResetState`
/// reply names the **fixture scope** in the `id` slot (there is no seam id
/// to name — the session ran its configured reset seam), while `Call` /
/// `InstallSeam` / `Clear` name the seam id. The carrier surfaces the field
/// verbatim, so a consumer can render `mock reset_state: …` and
/// `mock seed-42: …` exactly as the REPL's rows do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MockReceipt {
    /// The seam id the reply names — or, for a `ResetState` reply, the
    /// fixture scope that reset (the wire's one payload, field verbatim).
    pub id: String,
    /// The seam result detail (or the canned override response) — the same
    /// line the REPL's `mock` rows render.
    pub detail: String,
}

/// The client-side carrier for the wire `Codegen` reply (m53):
/// field-for-field the §13 payload — the generated kit code and the
/// strict-mode warnings collected while recording. Never serialized itself
/// (the reply rides `ResponseV1::Codegen` byte-for-byte), so the schema
/// stays byte-pinned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodegenOutput {
    /// The generated kit code (one recorded interaction per line, the §13
    /// cost model's minimal stable selectors).
    pub code: String,
    /// Strict-mode warnings collected while recording (§13).
    pub warnings: Vec<String>,
}

/// The reply an [`ClientConn::inspect`] call answers (m53): the §13
/// domain's replies are heterogeneous by wire design — the ack-only
/// commands (recording start, overlay enable/disable, highlight) ride the
/// shared `Pong`, and only the recording stop carries the `Codegen`
/// payload. The union names that shape client-side instead of collapsing
/// it into a bare `Option` that hides which command answered what — the
/// same honesty the carriers above draw (never serialized itself, so the
/// schema stays byte-pinned).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InspectReply {
    /// The command was applied — the `Pong` arms (recording start, overlay
    /// enable/disable, highlight aim/clear).
    Applied,
    /// The recording stop's answer: the generated kit code and its
    /// strict-mode warnings.
    Codegen(CodegenOutput),
}

/// The client-side carrier for the wire `ClockState` reply (m51):
/// field-for-field the §14 payload — the virtual clock's time in
/// milliseconds (`None` while the adapter still runs real time), the
/// paused flag and the frame-sized jumps the servicing command applied.
/// Never serialized itself (the reply rides `ResponseV1::ClockState`
/// byte-for-byte), so the schema stays byte-pinned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClockState {
    /// Virtual time in milliseconds, `None` before the clock is installed
    /// (the probe is total — a read, never a mutation).
    pub virtual_time_ms: Option<u64>,
    /// Whether the virtual clock is paused.
    pub paused: bool,
    /// Frame-sized jumps applied by the servicing command (the §14 pacing
    /// the ledger records, §12 `clock.bin`).
    pub steps: u64,
}

/// The client-side carrier for the wire `VideoFile` reply (m55):
/// field-for-field the §10 payload — the finalized recording file's path
/// (the client-owned artifact dir the `Start` options named, Playwright-style),
/// the frames actually encoded (dedup survivors), the frames shed by the
/// §10 hash-equality backpressure (a static UI compresses to a keyframe +
/// no-ops) and the wall-clock span the recording covers. Never serialized
/// itself (the reply rides `ResponseV1::VideoFile` byte-for-byte), so the
/// schema stays byte-pinned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VideoOutput {
    /// The finalized recording file's path (the `VideoFile` path — the
    /// same line the REPL's `video stop` row renders).
    pub path: String,
    /// Frames actually encoded (the §10 dedup survivors).
    pub frames: u64,
    /// Frames shed by hash-equality dedup, fps pacing and ring pressure
    /// (the §10 backpressure — dropped frames are the design working).
    pub dropped: u64,
    /// Wall-clock span from the first accepted frame to the last.
    pub duration_ms: u64,
}

/// The reply a [`ClientConn::video`] call answers (m55): the §10 domain's
/// replies are heterogeneous by wire design — the `Start` arm rides the
/// shared `Pong` ack (the recorder is live), and only the `Stop` arm
/// carries the finalized `VideoFile` payload. The union names that shape
/// client-side instead of collapsing it into a bare `Option` that hides
/// which command answered what — the same honesty the m53 [`InspectReply`]
/// union draws (never serialized itself, so the schema stays byte-pinned).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VideoReply {
    /// The recorder is live — the `Start` arm's ack (`Pong` on the wire).
    Started,
    /// The `Stop` arm's answer: the finalized recording file.
    File(VideoOutput),
}

/// The receipt a [`ClientConn::input`] answers (m56): the frame the raw
/// batch queued against (the §15.4 rendezvous — clients can await exactly
/// this frame, the same affordance the [`GestureReceipt::frame_seq`] and
/// [`ActionReceipt::frame_seq`] fields carry for their domains). A
/// client-side carrier for the wire `InputQueued` payload — never
/// serialized itself, so the schema stays byte-pinned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InputReceipt {
    /// The frame the batch was applied at.
    pub frame_seq: u64,
}

/// The receipt a [`ClientConn::tree_snapshot`] answers (m57): the §7
/// summary read — the tree generation (`tree_seq`, the seq space the
/// `TreeChanged` events report) and the whole tree's node count, the same
/// two facts the wire arm reports. A client-side carrier for the wire
/// `TreeSnapshot` payload — never serialized itself, so the schema stays
/// byte-pinned (the [`InputReceipt`] rule).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TreeSnapshotSummary {
    /// The tree generation the read observed.
    pub tree_seq: u64,
    /// The whole tree's node count (session-wide — the §16 v0.4 rule:
    /// the connection's window scope narrows locators, not whole-tree
    /// reads).
    pub node_count: usize,
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

    /// The §7 render domain as typed seam sugar (m50 — the m13-style read
    /// half of the M10 wire surface): one `RequestV1::Screenshot(
    /// ScreenshotV1::Window)` round trip — the driver renders the window
    /// frame once (the §10 headless on-demand path, no re-render) and
    /// answers the [`Shot`] carrier: the frame the pixels came from, the
    /// pixel dimensions and the PNG payload. The same shape the TypeScript
    /// client has spoken since M9 (`screenshotWindow()`) and the REPL since
    /// M10 (`screenshot window`). No wire change: the shot rides the
    /// `Screenshot` domain byte-for-byte as M10 blessed it; every
    /// [`ClientConn`] impl inherits the sugar.
    ///
    /// Adapters without a pixel path answer the typed `Unsupported` (§6.2
    /// capability row) — the sugar surfaces that rejection rather than
    /// degrading, the same honesty the embedded `App::screenshot` gives.
    fn screenshot_window(&mut self) -> impl Future<Output = Result<Shot, GwpError>> + Send {
        async move {
            match self
                .request(RequestV1::Screenshot(ScreenshotV1::Window))
                .await?
            {
                ResponseV1::Shot {
                    frame_seq,
                    width,
                    height,
                    png,
                } => Ok(Shot {
                    frame_seq,
                    width,
                    height,
                    png,
                }),
                other => Err(GwpError::Protocol(format!(
                    "Screenshot::Window answered by {other:?}"
                ))),
            }
        }
    }

    /// One element shot as typed seam sugar (m50): resolve `selector` (the
    /// driver's auto-wait `Attached` within `timeout_ms`, honoring the
    /// connection's window scope — the same resolve [`Self::resolve`] and
    /// [`Self::tree_dump_under`] run) and crop the window frame by the
    /// resolved node's clip-aware physical bounds — never a re-render (§10
    /// element rule). The same shape the TypeScript client has spoken since
    /// M9 (`screenshotElement(selector, timeoutMs)`) and the REPL since M10
    /// (`screenshot element`). No wire change: the crop rides the
    /// `Screenshot` domain byte-for-byte as M10 blessed it; every
    /// [`ClientConn`] impl inherits the sugar.
    ///
    /// The resolve's honesty rides the sugar unchanged: a selector that
    /// never matches inside the budget answers `ErrorCode::Timeout` naming
    /// the stuck check; strict mode is the wire's own policy (§8.2), so an
    /// ambiguous locator answers `ErrorCode::StrictModeViolation`. A node
    /// without bounds (laid out virtually) answers the honest error the
    /// M9 wording records — nothing to crop, not an empty crop.
    fn screenshot_element(
        &mut self,
        selector: SelectorV1,
        timeout_ms: u64,
    ) -> impl Future<Output = Result<Shot, GwpError>> + Send {
        async move {
            match self
                .request(RequestV1::Screenshot(ScreenshotV1::Element {
                    selector,
                    timeout_ms,
                }))
                .await?
            {
                ResponseV1::Shot {
                    frame_seq,
                    width,
                    height,
                    png,
                } => Ok(Shot {
                    frame_seq,
                    width,
                    height,
                    png,
                }),
                other => Err(GwpError::Protocol(format!(
                    "Screenshot::Element answered by {other:?}"
                ))),
            }
        }
    }

    /// The §14 virtual-clock domain as typed seam sugar (m51): one
    /// [`RequestV1::Clock`] round trip — install / fast-forward / run-for /
    /// pause / resume / set-time / probe — answering the [`ClockState`]
    /// carrier (virtual time in ms, the paused flag, the frame-sized jumps
    /// the command applied). The whole wire vocabulary in one call taking
    /// the wire's own [`ClockV1`] enum — no parallel client-side vocabulary
    /// to keep in sync, the same rule the m48 sugar set for `GestureV1`.
    /// Every reply carries the full state, so a client chains the next
    /// command off the state it just produced — the affordance the REPL's
    /// `clock` family and the TypeScript `clock()` / `fastForward()` helpers
    /// have spoken since §14/M4. No wire change: the command rides the
    /// `Clock` domain byte-for-byte as §14 blessed it; every [`ClientConn`]
    /// impl inherits the sugar.
    ///
    /// The boundary's honesty carries over unchanged: the probe is total
    /// (a read, never a mutation — `virtual_time_ms` reads `None` before
    /// the clock is installed); a `run_for` on a paused clock answers the
    /// session's typed `ClockPaused` rejection (run-for paces real time);
    /// the jumps ride the §14 ledger so traces replay timing (§12).
    fn clock(&mut self, cmd: ClockV1) -> impl Future<Output = Result<ClockState, GwpError>> + Send {
        async move {
            match self.request(RequestV1::Clock(cmd)).await? {
                ResponseV1::ClockState {
                    virtual_time_ms,
                    paused,
                    steps,
                } => Ok(ClockState {
                    virtual_time_ms,
                    paused,
                    steps,
                }),
                other => Err(GwpError::Protocol(format!("Clock answered by {other:?}"))),
            }
        }
    }

    /// The §14 state-seam domain as typed seam sugar (m52): one
    /// [`RequestV1::Mock`] round trip — install a canned override / clear
    /// it / call a registered seam once / reset app state to a fixture
    /// scope — answering the [`MockReceipt`] carrier (the id the reply
    /// names plus the detail the hook or override produced). The whole
    /// wire vocabulary in one call taking the wire's own [`MockV1`] enum —
    /// no parallel client-side vocabulary to keep in sync, the same rule
    /// the m48 sugar set for `GestureV1` and the m51 sugar set for
    /// [`ClockV1`]. The same shape the TypeScript client has spoken since
    /// M3 (`mock()` / `callSeam()` / `resetState()`) and the REPL since M4
    /// (`mock install` / `clear` / `call` / `reset`). No wire change: the
    /// command rides the `Mock` domain byte-for-byte as §14 blessed it;
    /// every [`ClientConn`] impl inherits the sugar.
    ///
    /// The domain's honesty carries over unchanged: install/clear are
    /// bookkeeping on the session's override table (the ack's detail says
    /// which), a `call` runs the registered hook inside the adapter's
    /// runner context (§14 "state writes need the runner's context") and
    /// pumps a frame so any UI effect settles, and a `reset_state` runs
    /// the session's configured reset seam (the default `"reset_state"`
    /// id) under the caller's fixture scope — the wire names that scope in
    /// the reply's id slot, the carrier surfaces it verbatim. A seam id
    /// nothing registered, and a hook that fails, answer the wire's typed
    /// rejection naming the cause — surfaced, never re-mapped (the canned
    /// override wins over the real hook exactly where §14 put it).
    ///
    /// [`ClockV1`]: crate::schema::ClockV1
    fn mock(&mut self, cmd: MockV1) -> impl Future<Output = Result<MockReceipt, GwpError>> + Send {
        async move {
            match self.request(RequestV1::Mock(cmd)).await? {
                ResponseV1::MockDone { id, detail } => Ok(MockReceipt { id, detail }),
                other => Err(GwpError::Protocol(format!("Mock answered by {other:?}"))),
            }
        }
    }

    /// The §13 inspector domain as typed seam sugar (m53): one
    /// [`RequestV1::Inspect`] round trip — start/stop the interaction →
    /// kit-code recorder, enable/disable the live overlay, aim/clear the
    /// hover outline — answering the [`InspectReply`] union (the ack-only
    /// commands answer [`InspectReply::Applied`], the recording stop
    /// answers [`InspectReply::Codegen`] with the generated kit code and
    /// its strict-mode warnings). The whole wire vocabulary in one call
    /// taking the wire's own [`InspectV1`] enum — no parallel client-side
    /// vocabulary to keep in sync, the same rule the m48 sugar set for
    /// `GestureV1` and the m51/m52 sugars set for [`ClockV1`] /
    /// [`MockV1`]. The same shapes the TypeScript client has spoken since
    /// M3 (`inspectOverlay()` / `inspectHighlight()`) and the REPL since
    /// M4/M8 (`codegen start` / `codegen stop`, `inspect` / `highlight`).
    /// No wire change: the command rides the `Inspect` domain
    /// byte-for-byte as §13 blessed it; every [`ClientConn`] impl inherits
    /// the sugar.
    ///
    /// The domain's honesty carries over unchanged: the highlight takes
    /// the dump-row handle form (a [`Self::resolve`] row — `None` clears
    /// the outline while the overlay session stays on), a handle the tree
    /// does not know answers the M8 boundary's typed
    /// `ErrorCode::UnknownNode`, starting a second recording while one is
    /// active answers the recorder's typed rejection (single-slot, §13),
    /// and a stop with nothing recorded answers the same rejection naming
    /// the cause. Adapters without an overlay surface answer the typed
    /// `Unsupported` (§6.2 capability row) — surfaced, never degraded.
    ///
    /// [`ClockV1`]: crate::schema::ClockV1
    /// [`MockV1`]: crate::schema::MockV1
    fn inspect(
        &mut self,
        cmd: InspectV1,
    ) -> impl Future<Output = Result<InspectReply, GwpError>> + Send {
        async move {
            match self.request(RequestV1::Inspect(cmd)).await? {
                ResponseV1::Pong => Ok(InspectReply::Applied),
                ResponseV1::Codegen { code, warnings } => {
                    Ok(InspectReply::Codegen(CodegenOutput { code, warnings }))
                }
                other => Err(GwpError::Protocol(format!("Inspect answered by {other:?}"))),
            }
        }
    }

    /// The §10 recording domain as typed seam sugar (m55): one
    /// [`RequestV1::Video`] round trip — start the presented-frame recorder
    /// (the client-owned artifact dir, fps cap; `0` selects the driver
    /// default) / stop and finalize it — answering the [`VideoReply`] union
    /// (the start arm acks [`VideoReply::Started`], the stop arm carries the
    /// finalized file as [`VideoOutput`]: path, encoded frames, dropped
    /// frames, wall-clock span). The whole wire vocabulary in one call
    /// taking the wire's own [`VideoV1`] enum — no parallel client-side
    /// vocabulary to keep in sync, the same rule the m48 sugar set for
    /// `GestureV1` and the m51/m52/m53 sugars set for the rest. The same
    /// shapes the TypeScript client has spoken since M3 (`videoStart()` /
    /// `videoStop()`) and the REPL since M3 (`video start` / `video stop`).
    /// No wire change: the command rides the `Video` domain byte-for-byte
    /// as M3 blessed it; every [`ClientConn`] impl inherits the sugar.
    ///
    /// The domain's honesty carries over unchanged: the recorder needs the
    /// adapter's §6.2 pixel capability row, so an adapter without a pixel
    /// path answers the typed `Unsupported`; a second `Start` while a
    /// recording runs answers the session's typed rejection (single-slot,
    /// the M3 rule); a `Stop` with nothing running answers the same
    /// rejection ("idempotence via error", mirroring the `Clock` rules).
    /// Frames presented faster than the fps cap collapse by hash-equality
    /// dedup — the reply's `dropped` count is the §10 backpressure working,
    /// never an anomaly.
    fn video(&mut self, cmd: VideoV1) -> impl Future<Output = Result<VideoReply, GwpError>> + Send {
        async move {
            match self.request(RequestV1::Video(cmd)).await? {
                ResponseV1::Pong => Ok(VideoReply::Started),
                ResponseV1::VideoFile {
                    path,
                    frames,
                    dropped,
                    duration_ms,
                } => Ok(VideoReply::File(VideoOutput {
                    path,
                    frames,
                    dropped,
                    duration_ms,
                })),
                other => Err(GwpError::Protocol(format!("Video answered by {other:?}"))),
            }
        }
    }

    /// The §9 raw-injection half as typed seam sugar (m56): one
    /// [`RequestV1::Input`] round trip — the client hand-builds the
    /// [`InputEventV1`] batch (pointer/keyboard/wheel/IME/window
    /// primitives, §9 vocabulary) and the driver injects it through the
    /// target window's real pipeline, applying the batch inside the next
    /// frame (one rendezvous pump, §15.4). Answers the [`InputReceipt`] —
    /// the frame the batch applied at. No wire change: the batch rides the
    /// `Input` domain byte-for-byte as M0 blessed it; every [`ClientConn`]
    /// impl inherits the sugar, the same shape the TypeScript client has
    /// spoken since M7 (`input(events)`).
    ///
    /// **Un-synthesized, on purpose.** The sugar stays the escape hatch
    /// the m48 gesture row's contract names — gestures synthesize their
    /// batches from the driver's per-adapter pacing table (§9), this row
    /// applies exactly the primitives the client sent, in order (a test
    /// that needs the pacing fidelity asks [`Self::gesture`], a test that
    /// needs raw fidelity sends the batch). The connection's window scope
    /// (§16 v0.4) steers the batch the way it steers gesture coordinates.
    /// An empty batch is an honest no-op that still pumps (the session's
    /// own rule) — the receipt names the frame the pump applied at, never
    /// a synthesized sentinel.
    fn input(
        &mut self,
        events: Vec<InputEventV1>,
    ) -> impl Future<Output = Result<InputReceipt, GwpError>> + Send {
        async move {
            match self.request(RequestV1::Input { events }).await? {
                ResponseV1::InputQueued { frame_seq } => Ok(InputReceipt { frame_seq }),
                other => Err(GwpError::Protocol(format!("Input answered by {other:?}"))),
            }
        }
    }

    /// The §7 summary read as typed seam sugar (m57): one
    /// [`RequestV1::TreeSnapshot`] round trip — the M0-era whole-tree
    /// answer (the tree generation and the node count), the read the M11
    /// dump domain later expanded into the m49 rows. Answers the
    /// [`TreeSnapshotSummary`] carrier. No wire change: the read rides the
    /// `TreeSnapshot` domain byte-for-byte as M0 blessed it; every
    /// [`ClientConn`] impl inherits the sugar, the same shape the
    /// TypeScript client has spoken since its M3 landing
    /// (`treeSnapshot()`).
    ///
    /// The domain's honesty carries over unchanged: the read is
    /// session-wide by design — the connection's window scope narrows
    /// locators, not whole-tree reads (the §16 v0.4 rule the dump rows
    /// carry) — and it is a read, not a mutation pump: the session pumps
    /// once and reports the tree as it is *now*, so back-to-back reads
    /// agree with each other and with the [`Self::tree_dump`] walk's count
    /// at the same generation. A client that needs the rows asks
    /// [`Self::tree_dump`]; a client that needs the two summary facts asks
    /// this row.
    fn tree_snapshot(
        &mut self,
    ) -> impl Future<Output = Result<TreeSnapshotSummary, GwpError>> + Send {
        async move {
            match self.request(RequestV1::TreeSnapshot).await? {
                ResponseV1::TreeSnapshot {
                    tree_seq,
                    node_count,
                } => Ok(TreeSnapshotSummary {
                    tree_seq,
                    node_count,
                }),
                other => Err(GwpError::Protocol(format!(
                    "TreeSnapshot answered by {other:?}"
                ))),
            }
        }
    }

    /// The §7 lifecycle close as typed seam sugar (m58): one
    /// [`RequestV1::Bye`] round trip answering `Ok(())` on the wire's
    /// `Bye` arm — the same round trip the TypeScript client's `close()`
    /// has made since its M3 landing before its own transport teardown.
    /// No wire change: the farewell rides the `Bye` domain byte-for-byte
    /// as M0 blessed it; every [`ClientConn`] impl inherits the sugar.
    ///
    /// **The transport close stays the door's business.** The wire's own
    /// contract: the driver answers the farewell and the conversation
    /// ends — `run_connection` returns right after writing the `Bye`
    /// frame, so a further request on this connection cannot land (the
    /// TS `close()` does exactly this round trip before its own
    /// `transport.close()`). What the farewell does *not* end is the
    /// session under the connection: the driver keeps serving fresh
    /// connections untouched, the door's teardown being the door's own
    /// business (socket drop, the in-process channels). The method
    /// answers before the door closes, so `Ok(())` is the honest
    /// last-word: the driver received the goodbye.
    fn bye(&mut self) -> impl Future<Output = Result<(), GwpError>> + Send {
        async move {
            match self.request(RequestV1::Bye).await? {
                ResponseV1::Bye => Ok(()),
                other => Err(GwpError::Protocol(format!("Bye answered by {other:?}"))),
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
                        let next = next_wake(self, deadline).await?;
                        let Some(next_wake) = next else {
                            return Err(expire(self, state, rendered, timeout, reason).await);
                        };
                        wake = next_wake;
                    }
                }
            }
        }
    }

    /// The content assertion, text order (m54 — the kit's
    /// `expect(locator).to_have_text(...)` over any door): the m29
    /// source order (the accessible name first, the value as the
    /// fallback — the source order the text locators match, the m46
    /// derivation reused verbatim) as a waitable assertion. Composes the
    /// §7 event channel with the M39 describe-all re-reads exactly like
    /// [`Self::wait_for`]: subscribe, evaluate the set's head (the
    /// [`crate::expect::ExpectTracker`] rules), and re-read on every wake
    /// until the content matches or `timeout` expires. **No wire
    /// change**: the assertion rides existing domains byte-for-byte;
    /// every [`ClientConn`] impl inherits it.
    ///
    /// **Answers.** The satisfied assertion hands back the row itself —
    /// directly usable as a `RequestV1::Action` handle (the dump-row
    /// rule, M11). The timeout is the typed [`GwpError::WaitTimeout`],
    /// naming the assertion (`have text`), the locator and the last
    /// evaluation — expected and found — never a wire answer.
    ///
    /// **Subscription ownership.** The assertion installs the
    /// `TreeChanged` + `FramePresented` filter for its duration and
    /// drops it on return, both on success and on expiry — the same
    /// rule [`Self::wait_for`] states: a connection running its own
    /// long-lived subscription (the viewer's live feed) should not hand
    /// the same connection to an assertion.
    ///
    /// # Errors
    ///
    /// Protocol failures (`subscribe`, the describe-all round trips,
    /// `next_event`) propagate as-is — a driver that predates M39
    /// answers `DescribeAll` with the typed `Unsupported` and the
    /// assertion surfaces that rejection rather than degrading (the m41
    /// fail-fast rule). A spent budget answers [`GwpError::WaitTimeout`].
    fn expect_text(
        &mut self,
        selector: SelectorV1,
        expected: TextMatchV1,
        timeout: Duration,
    ) -> impl Future<Output = Result<DescribedV1, GwpError>> + Send {
        self.expect_content(selector, ExpectKind::Text(expected), timeout)
    }

    /// The content assertion, value order (m54 — the kit's
    /// `expect(locator).to_have_value(...)` over any door): the m31
    /// source order (the value first, the accessible name as the
    /// fallback — the form-dump flip, so the two reads can never
    /// silently agree on the wrong datum), the same composition as
    /// [`Self::expect_text`]. **No wire change**; every [`ClientConn`]
    /// impl inherits it.
    ///
    /// # Errors
    ///
    /// The same boundary as [`Self::expect_text`]: protocol failures
    /// propagate as-is; a spent budget answers [`GwpError::WaitTimeout`]
    /// naming `have value`.
    fn expect_value(
        &mut self,
        selector: SelectorV1,
        expected: TextMatchV1,
        timeout: Duration,
    ) -> impl Future<Output = Result<DescribedV1, GwpError>> + Send {
        self.expect_content(selector, ExpectKind::Value(expected), timeout)
    }

    /// The identity assertion, role form (m62 — the kit's
    /// `expect(locator).to_have_role(...)` over any door, M38): the head
    /// row's role answers the exact string — the AccessKit debug name
    /// the describe rows print (`Button`, `TextInput`), the vocabulary
    /// the role locators speak. A structural fact, so the comparison is
    /// exact (no matcher, no fuzz). The same composition as
    /// [`Self::expect_text`]: the m42 wake loop over the M39
    /// describe-all rows, the set's head pinned, the subscription window
    /// owned and dropped. **No wire change**; every [`ClientConn`] impl
    /// inherits it.
    ///
    /// # Errors
    ///
    /// The same boundary as [`Self::expect_text`]: protocol failures
    /// propagate as-is; a spent budget answers [`GwpError::WaitTimeout`]
    /// naming `have role` and the role the last evaluation saw.
    fn expect_role(
        &mut self,
        selector: SelectorV1,
        role: String,
        timeout: Duration,
    ) -> impl Future<Output = Result<DescribedV1, GwpError>> + Send {
        self.expect_content(selector, ExpectKind::Role(role), timeout)
    }

    /// The identity assertion, name form (m62 — the kit's
    /// `expect(locator).to_have_accessible_name(...)` over any door,
    /// M38): the label-only accessible name — narrower than
    /// [`Self::expect_text`]'s m29 order (the value never answers), the
    /// fact the text row cannot pin. A node with no accessible name
    /// stays pending (waitable — the m54 empty-content rule's shape).
    /// The same composition; **no wire change**; every [`ClientConn`]
    /// impl inherits it.
    ///
    /// # Errors
    ///
    /// The same boundary as [`Self::expect_text`]: protocol failures
    /// propagate as-is; a spent budget answers [`GwpError::WaitTimeout`]
    /// naming `have name` and the name the last evaluation saw.
    fn expect_name(
        &mut self,
        selector: SelectorV1,
        expected: TextMatchV1,
        timeout: Duration,
    ) -> impl Future<Output = Result<DescribedV1, GwpError>> + Send {
        self.expect_content(selector, ExpectKind::Name(expected), timeout)
    }

    /// The shared content-assertion composition (m54): the m42 wait's
    /// loop shape with the [`ExpectTracker`] evaluation — one entry read,
    /// event-driven re-reads, the subscription window owned and dropped,
    /// the typed timeout built by [`expire_expect`]. Tree and frame wakes
    /// both re-decide (content has no `Stable`-like frame discipline);
    /// `Log` records are never wakes.
    fn expect_content(
        &mut self,
        selector: SelectorV1,
        kind: ExpectKind,
        timeout: Duration,
    ) -> impl Future<Output = Result<DescribedV1, GwpError>> + Send {
        async move {
            // The locator string for the timeout line (the kit's render
            // vocabulary — one conversion, not one per re-read).
            let rendered = Selector::from(selector.clone()).render();

            // The assertion owns the event filter for its duration, the
            // wait's rule (a driver that rejects Log-less subscriptions
            // answers the same `Subscribed` ack every door speaks).
            self.subscribe(&[EventKindV1::TreeChanged, EventKindV1::FramePresented])
                .await?;

            let deadline = tokio::time::Instant::from_std(Instant::now() + timeout);
            let tracker = ExpectTracker::new(kind.clone());
            loop {
                let rows = self.describe_all(selector.clone()).await?;
                match tracker.evaluate(&rows) {
                    ExpectEval::Satisfied(row) => {
                        self.unsubscribe().await?;
                        return Ok(row);
                    }
                    ExpectEval::Pending { reason } => {
                        if tokio::time::Instant::now() >= deadline {
                            return Err(expire_expect(self, kind, rendered, timeout, reason).await);
                        }
                        if next_wake(self, deadline).await?.is_none() {
                            return Err(expire_expect(self, kind, rendered, timeout, reason).await);
                        }
                    }
                }
            }
        }
    }

    /// The count assertion (m59 — the kit's `expect(locator)
    /// .to_have_count(...)` over any door, M22): the whole match set's
    /// size as a waitable assertion — the M30 resolve-all round trip's
    /// set read (strict mode never consulted, the m22 record's escape
    /// hatch), woken by the §7 event channel exactly like
    /// [`Self::wait_for`] and the m54 content pair. **No wire change**:
    /// the assertion rides existing domains byte-for-byte; every
    /// [`ClientConn`] impl inherits it.
    ///
    /// **Answers.** The satisfied assertion hands back the found count —
    /// the fact the assertion speaks (the m57 summary shape); the set's
    /// rows themselves ride [`Self::resolve_all`] (M30), one read per
    /// question.
    ///
    /// **Absence is an answer.** An empty set is found 0 — satisfying
    /// `count 0` and every less-than row (Playwright's `toHaveCount(0)`
    /// parity), unlike the content pair's empty-set pending.
    ///
    /// **Subscription ownership.** The assertion installs the
    /// `TreeChanged` + `FramePresented` filter for its duration and
    /// drops it on return, both on success and on expiry — the same
    /// rule [`Self::wait_for`] states: a connection running its own
    /// long-lived subscription (the viewer's live feed) should not hand
    /// the same connection to an assertion.
    ///
    /// # Errors
    ///
    /// Protocol failures (`subscribe`, the resolve-all round trips,
    /// `next_event`) propagate as-is — a driver that predates M30
    /// answers `ResolveAll` with the typed `Unsupported` and the
    /// assertion surfaces that rejection rather than degrading (the m41
    /// fail-fast rule). A spent budget answers [`GwpError::WaitTimeout`]
    /// naming `have count`.
    fn expect_count(
        &mut self,
        selector: SelectorV1,
        expected: usize,
        timeout: Duration,
    ) -> impl Future<Output = Result<usize, GwpError>> + Send {
        self.expect_count_with(selector, CountMatch::Exact(expected), timeout)
    }

    /// The one-sided count comparator (m59 — the kit's
    /// `expect(locator).to_have_count_greater_than(...)` over any door,
    /// M25): satisfied while *more* than `min` nodes match, the same
    /// composition as [`Self::expect_count`] with the comparison as the
    /// only difference. **No wire change**; every [`ClientConn`] impl
    /// inherits it.
    ///
    /// # Errors
    ///
    /// The same boundary as [`Self::expect_count`]: protocol failures
    /// propagate as-is; a spent budget answers [`GwpError::WaitTimeout`]
    /// naming `have count`.
    fn expect_count_greater_than(
        &mut self,
        selector: SelectorV1,
        min: usize,
        timeout: Duration,
    ) -> impl Future<Output = Result<usize, GwpError>> + Send {
        self.expect_count_with(selector, CountMatch::GreaterThan(min), timeout)
    }

    /// The one-sided count comparator (m59 — the kit's
    /// `expect(locator).to_have_count_less_than(...)` over any door,
    /// M25): satisfied while *fewer* than `max` nodes match, the mirror
    /// of [`Self::expect_count_greater_than`]. **No wire change**; every
    /// [`ClientConn`] impl inherits it.
    ///
    /// # Errors
    ///
    /// The same boundary as [`Self::expect_count`]: protocol failures
    /// propagate as-is; a spent budget answers [`GwpError::WaitTimeout`]
    /// naming `have count`.
    fn expect_count_less_than(
        &mut self,
        selector: SelectorV1,
        max: usize,
        timeout: Duration,
    ) -> impl Future<Output = Result<usize, GwpError>> + Send {
        self.expect_count_with(selector, CountMatch::LessThan(max), timeout)
    }

    /// The shared count-assertion composition (m59): the m54 loop shape
    /// over the M30 resolve-all round trip — one entry read, event-driven
    /// re-reads, the subscription window owned and dropped, the typed
    /// timeout built by [`expire_count`]. Tree and frame wakes both
    /// re-decide (a count has no `Stable`-like frame discipline); `Log`
    /// records are never wakes.
    fn expect_count_with(
        &mut self,
        selector: SelectorV1,
        comparison: CountMatch,
        timeout: Duration,
    ) -> impl Future<Output = Result<usize, GwpError>> + Send {
        async move {
            // The locator string for the timeout line (the kit's render
            // vocabulary — one conversion, not one per re-read).
            let rendered = Selector::from(selector.clone()).render();

            // The assertion owns the event filter for its duration, the
            // wait's rule (a driver that rejects Log-less subscriptions
            // answers the same `Subscribed` ack every door speaks).
            self.subscribe(&[EventKindV1::TreeChanged, EventKindV1::FramePresented])
                .await?;

            let deadline = tokio::time::Instant::from_std(Instant::now() + timeout);
            let tracker = CountTracker::new(comparison);
            loop {
                let set = self.resolve_all(selector.clone()).await?;
                match tracker.evaluate(&set) {
                    CountEval::Satisfied { found } => {
                        self.unsubscribe().await?;
                        return Ok(found);
                    }
                    CountEval::Pending { reason } => {
                        if tokio::time::Instant::now() >= deadline {
                            return Err(expire_count(self, rendered, timeout, reason).await);
                        }
                        if next_wake(self, deadline).await?.is_none() {
                            return Err(expire_count(self, rendered, timeout, reason).await);
                        }
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

/// The shared event-wait behind the client-side compositions (`wait_for`,
/// m42; the content assertions, m54): block on the §7 event channel or
/// the deadline, whichever lands first. `Log` records are never wakes —
/// the loop keeps waiting through them. `Ok(None)` is the spent budget
/// (the caller decides the expiry); event errors propagate as-is.
async fn next_wake<C: ClientConn + ?Sized>(
    client: &mut C,
    deadline: tokio::time::Instant,
) -> Result<Option<Wake>, GwpError> {
    loop {
        tokio::select! {
            event = client.next_event() => {
                // `GwpEvent` has the single `V1` variant, so this bind is
                // irrefutable (the version check pattern the connection
                // loop uses).
                let GwpEvent::V1(event) = event?;
                match event {
                    EventV1::TreeChanged { .. } => return Ok(Some(Wake::Tree)),
                    EventV1::FramePresented { .. } => return Ok(Some(Wake::Frame)),
                    // `Log` is never a wake: fall through to the next
                    // wake window.
                    EventV1::Log { .. } => {}
                }
            }
            () = tokio::time::sleep_until(deadline) => return Ok(None),
        }
    }
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

/// Build the content assertion's typed timeout (m54) and drop its
/// subscription (the wait's cleanup rule) — the assertion names the
/// state word (`have text` / `have value`), the locator and the last
/// evaluation (expected and found).
async fn expire_expect<C: ClientConn + ?Sized>(
    client: &mut C,
    kind: ExpectKind,
    selector: String,
    timeout: Duration,
    last: String,
) -> GwpError {
    let outcome = GwpError::WaitTimeout {
        state: kind.state(),
        selector,
        timeout_ms: u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX),
        last,
    };
    let _ = client.unsubscribe().await;
    outcome
}

/// Build the count assertion's typed timeout (m59) and drop its
/// subscription (the wait's cleanup rule) — the assertion names the
/// state word (`have count`), the locator and the last evaluation (the
/// demanded comparison and the count the last evaluation found).
async fn expire_count<C: ClientConn + ?Sized>(
    client: &mut C,
    selector: String,
    timeout: Duration,
    last: String,
) -> GwpError {
    let outcome = GwpError::WaitTimeout {
        state: crate::count::HAVE_COUNT_STATE,
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
