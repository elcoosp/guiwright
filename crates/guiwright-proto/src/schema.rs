//! Guiwright Protocol (GWP) — wire schema V1 (architecture §7).
//!
//! CDP-inspired: versioned, binary-first (postcard). Every payload enum has
//! an explicit version variant (`GwpRequest::V1`) so old clients get clean
//! errors instead of garbage. Wire types are decoupled from
//! `guiwright-core` types on purpose: the wire schema can evolve
//! independently (AddError-style explicit `V1` variants, §7 "old clients
//! get clean errors").

use serde::{Deserialize, Serialize};

/// Current protocol version. Bumped on breaking schema changes; the `V1`
/// variant stays parseable forever.
pub const PROTO_VERSION: u32 = 1;

/// Maximum encoded frame body size (64 MiB) — guards against corrupt or
/// hostile length prefixes.
pub const MAX_FRAME_BYTES: usize = 64 * 1024 * 1024;

/// Maximum decompressed blob size (256 MiB, the default trace ring cap §12).
pub const MAX_BLOB_BYTES: usize = 256 * 1024 * 1024;

/// Client → driver request.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum GwpRequest {
    V1(RequestV1),
}

/// Driver → client response.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum GwpResponse {
    V1(ResponseV1),
}

/// Server-push events (multiplexed by `seq` per topic, §7).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum GwpEvent {
    V1(EventV1),
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum RequestV1 {
    /// Handshake (§7). Must be the first request on a connection.
    Hello { proto_version: u32, agent: String },
    /// Liveness/latency probe.
    Ping,
    /// Snapshot of the logical tree (summary in M0; the M11
    /// [`RequestV1::Snapshot`] dump carries the full tree as typed
    /// postcard).
    TreeSnapshot,
    /// Resolve a selector with the strict-mode policy (§8.2).
    Resolve {
        selector: SelectorV1,
        timeout_ms: u64,
    },
    /// Apply a semantic action on a resolved node (§8.4).
    Action {
        node: NodeKeyV1,
        generation: u32,
        action: SemActionV1,
        force: bool,
        timeout_ms: u64,
    },
    /// Inject synthetic input through the real pipeline (§9).
    Input { events: Vec<InputEventV1> },
    /// Virtual clock domain (§7 `Clock` + §14): install/fast-forward/pause.
    Clock(ClockV1),
    /// Mock domain (§7 `Mock` + §14 state seams): install/clear/reset.
    Mock(MockV1),
    /// Inspector domain (§7 `Inspect` + §13): overlay + codegen recording.
    Inspect(InspectV1),
    /// Close the session cleanly.
    Bye,
    /// Video domain (§7 `Video` + §10 "the trace recorder doubles as
    /// video"): start/stop a frame recording of the session.
    ///
    /// Wire discipline: request/response variants are **append-only**
    /// within a `V1` envelope — `Video` follows `Bye` (discriminant 10)
    /// so every pre-M3 client keeps its byte-exact encodings; the
    /// pinning tests below fail the build on any reordering.
    Video(VideoV1),
    /// Window domain (§7 + §16 v0.4 multi-window, M6): enumerate the
    /// session's window targets and aim the calling connection at one of
    /// them — Playwright's "targets per window". Wire discipline: appended
    /// after `Video` (discriminant 11); see [`RequestV1::Video`].
    Window(WindowV1),
    /// Gesture domain (§9 input pipeline, M7): high-level synthetic
    /// gestures — the session synthesizes the raw event sequence
    /// (per-adapter pacing, §18) and injects it through the target
    /// window's real pipeline. Wire discipline: appended after `Window`
    /// (discriminant 12); see [`RequestV1::Video`].
    Gesture(GestureV1),
    /// Render domain (§7 `Screenshot { target }`, M10): pull a PNG shot of
    /// the session's current frame — the whole window, or the crop of one
    /// resolved element (§10 "crop from the window frame by the node's
    /// clip-aware rect — never re-render"). Wire discipline: appended
    /// after `Gesture` (discriminant 13); see [`RequestV1::Video`].
    Screenshot(ScreenshotV1),
    /// Tree-dump domain (§7 `TreeSnapshot` full form, M11): read the
    /// session's whole logical tree as data — the CDP "DOM snapshot"
    /// analog, and the M0 promise this file's `TreeSnapshot` doc carried
    /// ("full snapshot rides the wire once traces land" — traces shipped
    /// in M1; the dump rides as typed postcard, see [`SnapshotV1`]).
    /// Wire discipline: appended after `Screenshot` (discriminant 14);
    /// see [`RequestV1::Video`].
    Snapshot(SnapshotV1),
    /// Event-subscription domain (§7 server-push events, M12): turn this
    /// connection into a subscriber — pushed [`ResponseV1::Event`] frames
    /// flow between responses while the subscription is on (bounded queue,
    /// §15.6). Wire discipline: appended after `Snapshot` (discriminant
    /// 15); see [`RequestV1::Video`].
    Subscribe(SubscribeV1),
    /// Resolve-all domain (§8.2's set read, M30): the non-strict
    /// enumeration of a selector's current match set — the kit's
    /// `count()` / `all()` walk over the wire, the m22/m26/m29 records'
    /// named protocol half. Snapshot semantics: the read never waits
    /// (hence no `timeout_ms` — an empty set answers with an empty list;
    /// a client that wants arrivals waits and re-reads, the way the kit's
    /// snapshot reads compose with `wait_for` and the assertions). The
    /// connection's window scope narrows the walk exactly like
    /// [`RequestV1::Resolve`]. The reply is
    /// [`ResponseV1::ResolvedAll`]. Wire discipline: appended after
    /// `Subscribe` (discriminant 16); see [`RequestV1::Video`].
    ResolveAll { selector: SelectorV1 },
    /// Hit-test read (§8.3's "receives events" mechanism as a public
    /// read, M32): which node occupies the point — window-relative
    /// physical px, the same space the tree's bounds speak (the dump
    /// rows' `bounds`, the resolve rects), so a client can hit inside a
    /// rect it just resolved. A pump runs first (consistency, like
    /// [`RequestV1::Resolve`]); the connection's window scope narrows the
    /// answer exactly like the resolve paths (a hit outside the aimed
    /// window reads `None`); empty space answers `None` — the read is
    /// total, never an error. Hidden nodes cannot answer (the same
    /// `common_filter` the resolution pipeline applies). The reply is
    /// [`ResponseV1::HitTest`]. Wire discipline: appended after
    /// `ResolveAll` (discriminant 17); see [`RequestV1::Video`].
    HitTest { x: f64, y: f64 },
    /// Node-describe read (§8.1's identity facts as a public read, M36 —
    /// the m32 record's standing gap: "role/name would ride a
    /// node-describe domain that does not exist yet"): one node's facts —
    /// role, accessible name, value, bounds — for a handle the client
    /// already holds. The handle is the dump-row form (a resolve row, a
    /// hit row, a dump row — each is directly usable as the `Describe`
    /// node, M11's rule), so the read answers "what IS the node this
    /// handle points at" for REPL rows and viewer-style clients without
    /// a whole-subtree dump. A pump runs first (consistency, like
    /// [`RequestV1::Resolve`]). The read is **handle-addressed, not
    /// scoped**: unlike a point (ambiguous — [`RequestV1::HitTest`]
    /// narrows) or a selector (the tree answers — the resolve paths
    /// narrow), the handle IS the node, so no window scope applies — the
    /// same boundary [`RequestV1::Action`] draws. The generation guard is
    /// the Action boundary's too: a handle minted before the node left
    /// and re-entered the tree is the typed
    /// `Error { code: ErrorCode::StaleGeneration }`, a handle the tree
    /// does not know is `ErrorCode::UnknownNode`. The reply is
    /// [`ResponseV1::Described`]. Wire discipline: appended after
    /// `HitTest` (discriminant 18); see [`RequestV1::Video`].
    Describe(DescribeV1),
    /// Describe-all domain (the §8.2 set read carrying facts, M39 — the
    /// m37 record's standing gap: "enumerating a set's facts would open
    /// one `Describe` round trip per row"): the non-strict
    /// enumeration of a selector's current match set, each row carrying
    /// the identity facts the [`RequestV1::Describe`] read returns —
    /// one round trip instead of N. Snapshot semantics: the read never
    /// waits (hence no `timeout_ms` — an empty set answers with an
    /// empty list; a client that wants arrivals waits and re-reads, the
    /// way the kit's snapshot reads compose with `wait_for` and the
    /// assertions). The connection's window scope narrows the walk
    /// exactly like [`RequestV1::ResolveAll`]. The reply is
    /// [`ResponseV1::DescribedAll`]. Wire discipline: appended after
    /// `Describe` (discriminant 19); see [`RequestV1::Video`].
    DescribeAll(DescribeAllV1),
    /// Describe-all identity read (§8.1's identity fact set over the
    /// §8.2 walk, M64 — the m62/m63 records' reserved wire change, the
    /// m63 record's "the wire design is the next milestone's first
    /// decision"): the same non-strict walk as [`RequestV1::DescribeAll`]
    /// answering rows that carry the description fact beside the
    /// identity four — the fact the M38 identity family's third
    /// assertion (`to_have_accessible_description`, kit-side since M38)
    /// reads and the M39 rows left out. [`DescribedV1`]'s encoding is
    /// byte-pinned (older clients keep their exact encodings), so the
    /// widening lands as its own appended domain reusing the
    /// [`DescribeAllV1`] payload — the read costs what
    /// [`RequestV1::DescribeAll`] costs. The reply is
    /// [`ResponseV1::DescribedAllIdentity`]. Wire discipline: appended
    /// after `DescribeAll` (discriminant 20); see [`RequestV1::Video`].
    DescribeAllIdentity(DescribeAllV1),
}

/// The describe-all read's request payload (§8.1/§8.2, M39): the
/// selector the set read walks — the [`RequestV1::ResolveAll`] payload's
/// exact shape, so every client that can enumerate a set can describe it.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DescribeAllV1 {
    /// The selector whose match set is enumerated (non-strict — the
    /// whole set, document order; the m30 rule).
    pub selector: SelectorV1,
}

/// The node-describe read's request payload (§8.1, M36): the handle plus
/// the generation it was minted at — the [`RequestV1::Action`] guard's
/// exact shape, so every client that can act on a row can describe it.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DescribeV1 {
    /// The node identity (a dump-row handle: directly usable as the
    /// [`RequestV1::Action`] node).
    pub node: NodeKeyV1,
    /// The node's resolve generation at mint time (the ABA guard, §8.2
    /// step 2 — the boundary rejects a recycled id).
    pub generation: u32,
}

/// `Snapshot { Full, Filtered }` (§7 tree domain, M11/M13). Extensible on
/// purpose — new dump forms land behind new variants, never a reshape of
/// the existing ones (the append-only discipline).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum SnapshotV1 {
    /// The whole logical tree, depth-first per window root (§16 v0.4
    /// multi-window: one walk per window target, disjoint by
    /// construction) → [`ResponseV1::TreeDump`].
    Full,
    /// One subtree: resolve `selector` (auto-wait `Attached` within
    /// `timeout_ms`, honoring the connection's window scope — the same
    /// resolve the `Resolve`/`Screenshot::Element` arms run) and dump the
    /// depth-first walk rooted at the match → [`ResponseV1::TreeDump`]
    /// with only that subtree's rows (M13; the m11 "filtered dumps land
    /// behind new variants" note, cashed).
    Filtered {
        selector: SelectorV1,
        timeout_ms: u64,
    },
}

/// One node of a [`ResponseV1::TreeDump`] (M11): the identity plus the
/// facts the REPL/viewer render. `children` + depth-first order rebuild
/// the hierarchy without a parent pointer per node.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SnapshotNodeV1 {
    /// The node identity (the same vocabulary [`RequestV1::Action`] and
    /// [`RequestV1::Inspect`] take — a dump row is directly usable as a
    /// handle).
    pub key: NodeKeyV1,
    /// AccessKit role `Debug` name (`Button`, `TextInput`, …) — the same
    /// vocabulary [`SelectorV1::Role`] speaks.
    pub role: String,
    /// Accessible name, when the node has one.
    pub name: Option<String>,
    /// Value (inputs), when the node carries one.
    pub value: Option<String>,
    /// Window-relative bounds, physical px (AccessKit convention).
    pub bounds: Option<[f64; 4]>,
    /// Immediate children — DFS order + this count rebuilds the tree.
    pub children: u32,
}

/// `Screenshot { Window | Element { selector, timeout_ms } }` (§7 render
/// domain, M10). The driver renders the window frame once (the §10
/// "prefer `render_frame` headless on-demand" path) and answers with
/// [`ResponseV1::Shot`]; element shots resolve the selector first (§8.2
/// auto-wait over the wire, `Attached` within `timeout_ms`) and crop the
/// frame by the resolved node's physical bounds.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum ScreenshotV1 {
    /// The full window frame (§10 window screenshot).
    Window,
    /// Resolve `selector` (auto-wait `Attached` within `timeout_ms`, like
    /// [`RequestV1::Resolve`]) and crop the window frame by the resolved
    /// node's physical bounds — never a re-render (§10 element rule).
    Element {
        selector: SelectorV1,
        timeout_ms: u64,
    },
}

/// `Clock { InstallVirtual | FastForward(ms) | RunFor(ms) | Pause | Resume |
/// SetTime(t) }` (§7) plus the state probe the kit needs.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ClockV1 {
    InstallVirtual,
    FastForward {
        ms: u64,
    },
    RunFor {
        ms: u64,
    },
    Pause,
    Resume,
    /// Absolute virtual time (§7 `SetTime(t)`), milliseconds.
    SetTime {
        ms: u64,
    },
    /// Read the current clock state without mutating it.
    State,
}

/// `Mock { InstallSeam { id, response } | Clear(id) | ResetState { scope } }`
/// (§7) plus `Call` — run a registered seam once from the test side.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum MockV1 {
    /// Install (or re-aim) a seam by its registered id. `response` presets a
    /// canned result for the mock domain (None = run the real hook).
    InstallSeam {
        id: String,
        response: Option<String>,
    },
    /// Remove a seam override; the real hook (if any) runs again.
    Clear { id: String },
    /// Reset app state to a fixture scope (§14 "fixture parity with
    /// Playwright contexts") — runs the session's reset seam.
    ResetState { scope: String },
    /// Execute a registered seam once (arg = scope/fixture tag).
    Call { id: String, scope: String },
}

/// `Inspect { EnableOverlay | Highlight(handle) | StartRecording |
/// StopRecording }` (§7). M2 shipped the codegen recorder half (§13);
/// M8 lands the live highlight handles — the interactive overlay session
/// (`EnableOverlay` / `DisableOverlay` / `Highlight`), composited by the
/// adapter at present time (§13 "Overlay: adapter-level composition").
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum InspectV1 {
    /// Start the interaction → code recorder (§13).
    StartRecording,
    /// Stop and fetch the generated kit code.
    StopRecording,
    /// §13 `EnableOverlay` (M8): start the inspector overlay session —
    /// the adapter composites hover outlines + interactive-role badges
    /// from the tree indexes at present time. Wire discipline: appended
    /// after `StopRecording` (discriminant 2); see [`RequestV1::Video`].
    EnableOverlay,
    /// §13 overlay complement (M8): stop compositing and drop the
    /// highlight. Appended after `EnableOverlay` (discriminant 3).
    DisableOverlay,
    /// §13 `Highlight(handle)` (M8): aim the hover outline at a resolved
    /// node (facts ride the caller's own resolution; the reply is
    /// [`ResponseV1::Pong`]). `None` clears the outline and keeps the
    /// overlay session. Wire discipline: appended after `DisableOverlay`
    /// (discriminant 4).
    Highlight { node: Option<NodeKeyV1> },
}

/// `Video { Start(opts) | Stop }` → `VideoFile { path }` (§7, §10). The
/// driver records presented frames between the two calls: hash-equality
/// dedup drops static repetitions (§10 backpressure), fps paces the ring,
/// and the encoder subprocess (ffmpeg) or PNG-sequence fallback finalizes
/// the file at `Stop`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum VideoV1 {
    /// Start recording with these options (§10).
    Start(VideoOptsV1),
    /// Stop and finalize the recording file.
    Stop,
}

/// Video recording options (§10).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct VideoOptsV1 {
    /// Target frame-rate cap. `0` selects the driver default (25 fps).
    /// Frames presented faster than `1/fps` are collapsed by the
    /// hash-equality dedup — a static UI records to keyframe + no-ops.
    pub fps: u32,
    /// Output directory for the recording (Playwright-style: the client
    /// owns the artifact dir). `None` = the driver's default
    /// (`GUIWRIGHT_VIDEO_DIR` or the OS temp dir). Relative paths resolve
    /// against the driver process's cwd.
    pub dir: Option<String>,
}

impl Default for VideoOptsV1 {
    fn default() -> Self {
        Self {
            fps: VIDEO_DEFAULT_FPS,
            dir: None,
        }
    }
}

/// Driver default frame-rate cap (§10: Playwright records ≈25 fps).
pub const VIDEO_DEFAULT_FPS: u32 = 25;

/// `Window { List | Use { handle } }` (§7 + §16 v0.4, M6). A multi-window
/// app exposes one target per window; `Use` aims the **calling connection**
/// at one target — subsequent `Resolve` requests (and the auto-wait they
/// drive) only see that window's subtree. Other connections keep their own
/// target: per-connection state lives on the server bridge, not in the
/// shared session (§11 "one driver session serves all connections").
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum WindowV1 {
    /// List the session's window targets → `Windows`.
    List,
    /// Aim this connection at a window target → `WindowUsed`, or
    /// `Error { UnknownWindow }` when the handle is unknown (never existed
    /// or closed). The node identities of `Action` are already
    /// window-unique (`NodeKey` carries the `TreeId`), so scoping concerns
    /// resolution only.
    Use { handle: String },
}

/// One window target as reported by `Windows` (§16 v0.4, M6). `handle` is
/// the stable target id — the AccessKit subtree `TreeId` for grafted
/// (multi-window) adapters, `"main"` for single-window adapters.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WindowInfoV1 {
    pub handle: String,
    pub title: String,
    pub focused: bool,
}

/// `Gesture { Click | Drag | Wheel | Key | Type }` (§9 synthesis, M7): the
/// wire-level gesture vocabulary. The **driver** synthesizes the raw
/// `InputEventV1` sequence (click = move→down→up; drag = down→
/// ratchet-interpolated moves→up, §9 "ratchet ~5 px/tick"; key = down→up;
/// type = one text commit) from its adapter's pacing table — synthesis is
/// driver-side so each framework's quirks stay encoded at one place
/// (§18 "per-adapter gesture pacing tables"). Coordinates are logical px
/// **relative to the target window** (the sending connection's active
/// target, defaulting to `main`).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum GestureV1 {
    /// Press and release at a window-relative position. `count` follows
    /// Playwright's clickCount: 1 = click, 2 = double-click (down/up
    /// repeated at the same point).
    Click {
        x: f64,
        y: f64,
        button: PointerButtonV1,
        count: u32,
    },
    /// Drag from one window-relative position to another; the driver
    /// interpolates the moves (§9 ratchet pacing) so hover/drag state
    /// machines see a real pointer path.
    Drag {
        from_x: f64,
        from_y: f64,
        to_x: f64,
        to_y: f64,
        button: PointerButtonV1,
    },
    /// Wheel scroll by logical-px deltas.
    Wheel { dx: f64, dy: f64 },
    /// Press and release a key with modifiers (key names are the §8.4
    /// `Press` vocabulary, e.g. "Enter", "a").
    Key { key: String, modifiers: Vec<String> },
    /// Commit text in one step (adapters translate to IME or per-key
    /// events per §4.2 keyboard fidelity).
    Type { text: String },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum ResponseV1 {
    /// Handshake reply (§7 `Welcome`).
    Welcome {
        proto_version: u32,
        session_id: String,
        tree_seq: u64,
        frame_seq: u64,
        adapter: AdapterInfoV1,
    },
    Pong,
    TreeSnapshot {
        tree_seq: u64,
        node_count: usize,
    },
    Resolved {
        node: NodeKeyV1,
        generation: u32,
        /// Window-relative bounds, physical px (AccessKit convention).
        rect: Option<[f64; 4]>,
    },
    ActionDone {
        /// The frame the action was applied at — clients can await exactly
        /// this frame, no polling (§7 multiplexing).
        frame_seq: u64,
        detail: String,
    },
    InputQueued {
        frame_seq: u64,
    },
    /// Clock state reply (§14 `ClockState`), milliseconds on the wire.
    ClockState {
        virtual_time_ms: Option<u64>,
        paused: bool,
        /// Jumps applied by the servicing command (frame-sized steps).
        steps: u64,
    },
    /// Mock domain reply — seam executed or bookkeeping applied.
    MockDone {
        id: String,
        /// Seam result detail (or the canned override response).
        detail: String,
    },
    /// Inspector reply — the generated kit code (§13).
    Codegen {
        code: String,
        /// Strict-mode warnings collected while recording (§13).
        warnings: Vec<String>,
    },
    /// Rejected request: unknown version, bad sequence, adapter error…
    Error {
        code: ErrorCode,
        message: String,
    },
    Bye,
    /// Video domain reply (§10): the finalized recording file. Append-only
    /// wire discipline — see [`RequestV1::Video`]. `dropped` counts frames
    /// shed by the §10 hash-equality backpressure (static UI compresses to
    /// a keyframe + no-ops), `duration_ms` is the wall-clock span the
    /// recording covers.
    VideoFile {
        path: String,
        frames: u64,
        dropped: u64,
        duration_ms: u64,
    },
    /// Window domain reply (§16 v0.4, M6): the session's window targets.
    /// Append-only wire discipline — see [`RequestV1::Video`].
    Windows {
        windows: Vec<WindowInfoV1>,
    },
    /// Window domain reply: this connection is now aimed at the window
    /// target (§16 v0.4). The server bridge records the active handle and
    /// stamps subsequent requests with it.
    WindowUsed {
        handle: String,
    },
    /// Gesture domain reply (§9, M7): the synthesized event batch was
    /// injected through the target window's pipeline and applied inside
    /// the next frame. Append-only wire discipline — see
    /// [`RequestV1::Video`].
    GestureDone {
        /// The frame the gesture was applied at (rendezvous, §15.4).
        frame_seq: u64,
        /// Human-readable synthesis summary (event count, positions).
        detail: String,
    },
    /// Render domain reply (§7 `Shot`, M10): the PNG bytes of the captured
    /// frame — the whole window, or the element crop — with the session
    /// frame they came from and their pixel dimensions. The payload rides
    /// the response frame directly: PNG is already losslessly compressed
    /// (DEFLATE), so the §15.6 LZ4 blob stage would double-encode without
    /// shrinking it; [`MAX_FRAME_BYTES`] bounds hostile sizes. Append-only
    /// wire discipline — see [`RequestV1::Video`].
    Shot {
        /// The session frame the pixels came from (the same seq space
        /// [`ResponseV1::ActionDone`] reports — clients can correlate).
        frame_seq: u64,
        /// Pixel width of `png` (the full frame for `Window`, the crop
        /// width for `Element`).
        width: u32,
        /// Pixel height of `png`.
        height: u32,
        /// PNG-encoded RGBA8 (lossless, §10 "Output: PNG").
        png: Vec<u8>,
    },
    /// Tree-dump domain reply (§7 full `TreeSnapshot`, M11): the whole
    /// logical tree as typed postcard — depth-first per window root, so
    /// `children` counts rebuild the hierarchy. The dump rides the frame
    /// directly (a debugging payload, bounded by [`MAX_FRAME_BYTES`]); the
    /// LZ4 blob stage stays with the trace-frame traffic it was sized for
    /// (§15.6), keeping other-language clients dependency-free.
    /// Append-only wire discipline — see [`RequestV1::Video`].
    TreeDump {
        /// The tree generation the dump was taken at (the same seq space
        /// [`ResponseV1::TreeSnapshot`] reports).
        tree_seq: u64,
        /// Dumped node count (== `nodes.len()`, redundancy for readers).
        node_count: usize,
        /// The focused node, when the tree has one.
        focus: Option<NodeKeyV1>,
        /// Depth-first per window root (§16 v0.4): every window's subtree
        /// appears once, roots in adapter-reported window order.
        nodes: Vec<SnapshotNodeV1>,
    },
    /// Server-push event (§7, M12): an [`EventV1`] the connection is
    /// subscribed to, flushed between responses (bounded queue, §15.6).
    /// Events ride the response envelope so every client demultiplexes
    /// one enum — a connection that never subscribes never sees one
    /// (byte-identical conversations for pre-M12 clients). Append-only
    /// wire discipline — see [`RequestV1::Video`].
    Event(EventV1),
    /// Event-subscription ack (§7, M12): the kinds now active on this
    /// connection (empty after `Off`). Append-only wire discipline — see
    /// [`RequestV1::Video`].
    Subscribed {
        /// The deduplicated, server-honored subscription (a named-but-
        /// unsupported kind fails the whole request instead — the
        /// all-or-nothing filter contract).
        kinds: Vec<EventKindV1>,
    },
    /// Resolve-all reply (§8.2's set read, M30): every current match of
    /// the selector, document order — the same non-strict walk
    /// `resolve_all` runs embedded (no strict mode, no waiting, hidden
    /// nodes included — the `count()`/`all()` set semantics). Each row
    /// is directly usable as an [`RequestV1::Action`] handle (the
    /// dump-row rule, M11). An empty set answers with an empty list —
    /// the set read is total, never an error. Append-only wire
    /// discipline — see [`RequestV1::Video`].
    ResolvedAll {
        /// One row per match, document order (the `all()`/`count()`
        /// ordering).
        matches: Vec<ResolvedMatchV1>,
    },
    /// Hit-test reply (§8.3's mechanism as a public read, M32): the node
    /// occupying the requested point as one [`ResolvedMatchV1`] row —
    /// identity + generation + bounds, directly usable as an
    /// [`RequestV1::Action`] handle (the dump-row rule, M11) — or `None`
    /// for empty space (and for a hit outside the connection's window
    /// scope, §16 v0.4). The read is total: a miss is a valid answer,
    /// never an error. Append-only wire discipline — see
    /// [`RequestV1::Video`].
    HitTest {
        /// The node under the point, `None` on empty space / outside the
        /// aimed window's subtree.
        hit: Option<ResolvedMatchV1>,
    },
    /// Node-describe reply (§8.1's identity facts, M36): the handle's
    /// role, accessible name, value and bounds in one row — the same
    /// facts a [`SnapshotNodeV1`] dump row carries, one node instead of a
    /// walk. The handle echoes the request's (a row is directly usable
    /// as the [`RequestV1::Action`] node, the dump-row rule). Stale and
    /// unknown handles ride the typed `Error` codes, never a partial
    /// row. Append-only wire discipline — see [`RequestV1::Video`].
    Described(DescribedV1),
    /// Describe-all reply (the §8.2 set read carrying facts, M39): one
    /// [`DescribedV1`] row per match, document order (the `all()` /
    /// `count()` ordering) — the resolve-all walk's rows plus the facts
    /// the [`ResponseV1::Described`] read returns, one round trip
    /// instead of N. An empty set answers with an empty list — the set
    /// read is total, never an error. Append-only wire discipline — see
    /// [`RequestV1::Video`].
    DescribedAll {
        /// One row per match, document order (the `all()`/`count()`
        /// ordering).
        rows: Vec<DescribedV1>,
    },
    /// Describe-all identity reply (§8.1's identity fact set over the
    /// walk, M64): one [`DescribedIdentityV1`] row per match, document
    /// order — the [`ResponseV1::DescribedAll`] rows plus the
    /// description fact, the m62/m63 records' reserved wire change. An
    /// empty set answers with an empty list — the set read is total,
    /// never an error. Append-only wire discipline — see
    /// [`RequestV1::Video`].
    DescribedAllIdentity {
        /// One row per match, document order (the `all()`/`count()`
        /// ordering).
        rows: Vec<DescribedIdentityV1>,
    },
}

/// One node's identity facts ([`ResponseV1::Described`], M36) — the
/// one-row read beside the dump's whole-tree walk: role (the
/// [`SelectorV1::Role`] vocabulary), accessible name, value (inputs) and
/// window-relative physical bounds (the resolve rects' space).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DescribedV1 {
    /// The described node — the request's handle, echoed.
    pub node: NodeKeyV1,
    /// The node's current resolve generation (the handle is live at this
    /// generation — the guard the boundary enforced).
    pub generation: u32,
    /// AccessKit role `Debug` name (`Button`, `TextInput`, …).
    pub role: String,
    /// Accessible name, when the node has one.
    pub name: Option<String>,
    /// Value (inputs), when the node carries one.
    pub value: Option<String>,
    /// Window-relative bounds, physical px (AccessKit convention) — the
    /// same rect [`ResponseV1::Resolved`] reports.
    pub bounds: Option<[f64; 4]>,
}

/// One node's identity facts, description included
/// ([`ResponseV1::DescribedAllIdentity`], M64) — the M39
/// [`DescribedV1`] row's fields plus the accessible description, the
/// fact the M38 identity family's third assertion reads (the m62/m63
/// records' reserved wire change; `DescribedV1` keeps its byte-exact
/// M39 encoding, the append-only rule — the widening lands as its own
/// row rather than a reshape).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DescribedIdentityV1 {
    /// The described node — the request's handle, echoed (a row is
    /// directly usable as the [`RequestV1::Action`] node, the dump-row
    /// rule).
    pub node: NodeKeyV1,
    /// The node's current resolve generation (the handle is live at this
    /// generation — the guard the boundary enforced).
    pub generation: u32,
    /// AccessKit role `Debug` name (`Button`, `TextInput`, …).
    pub role: String,
    /// Accessible name, when the node has one.
    pub name: Option<String>,
    /// Value (inputs), when the node carries one.
    pub value: Option<String>,
    /// Window-relative bounds, physical px (AccessKit convention) — the
    /// same rect [`ResponseV1::Resolved`] reports.
    pub bounds: Option<[f64; 4]>,
    /// Accessible description, when the node carries one — the m38
    /// read's fact (`description()` only, never the name or the value:
    /// a value never satisfies a description).
    pub description: Option<String>,
}

impl From<&DescribedIdentityV1> for DescribedV1 {
    /// The M39 row shape: the identity four projected off the m64 row —
    /// the projection the content and identity kinds read when they
    /// evaluate over the wider rows (the description fact rides beside
    /// them, never instead of them).
    fn from(row: &DescribedIdentityV1) -> Self {
        Self {
            node: row.node.clone(),
            generation: row.generation,
            role: row.role.clone(),
            name: row.name.clone(),
            value: row.value.clone(),
            bounds: row.bounds,
        }
    }
}

/// One row of a [`ResponseV1::ResolvedAll`] (M30): the identity facts the
/// set read returns per match — the same three fields
/// [`ResponseV1::Resolved`] carries, one per node of the set.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ResolvedMatchV1 {
    /// The node identity (a dump-row handle: directly usable as the
    /// [`RequestV1::Action`] node).
    pub node: NodeKeyV1,
    /// The node's resolve generation (the ABA guard, §8.2 step 2).
    pub generation: u32,
    /// Window-relative bounds, physical px (AccessKit convention) — the
    /// same rect [`ResponseV1::Resolved`] reports.
    pub rect: Option<[f64; 4]>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum EventV1 {
    TreeChanged { seq: u64 },
    FramePresented { seq: u64, hash: u64 },
    Log { record: String },
}

/// Event-subscription vocabulary (§7 server-push events, M12). The kinds
/// mirror [`EventV1`]'s variant order — appending a future event kind
/// appends here too, never reorders.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum EventKindV1 {
    /// The logical tree changed (the same seq space
    /// [`ResponseV1::TreeSnapshot`] reports).
    TreeChanged,
    /// A frame was presented (seq + the visual-change hash, the same
    /// vocabulary the trace recorder dedups on, §15.5).
    FramePresented,
    /// A log record (the `tracing`-fed channel, the log-tail milestone):
    /// one `logs.ndjson`-shape JSON line per record (`ts_ns`, `level`,
    /// `target`, `pid`, `message`, plus the event's structured fields —
    /// the §12 vocabulary), streamed from the session thread's per-session
    /// tracing tap. The tap is opt-in: a session spawned without it still
    /// rejects a `Log` subscription with [`ErrorCode::Unsupported`] — the
    /// honest capability answer, never a silent no-op filter.
    Log,
}

/// `Subscribe { On, Off }` (§7 event channel, M12). Re-subscribing
/// replaces the connection's filter; `Off` clears it. Both answer
/// [`ResponseV1::Subscribed`], echoing the kinds now active (empty after
/// `Off`).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum SubscribeV1 {
    /// Subscribe (or re-subscribe) to the named event kinds. An empty
    /// list subscribes to nothing (an acknowledged no-op filter).
    On { kinds: Vec<EventKindV1> },
    /// Drop the connection's subscription.
    Off,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ErrorCode {
    UnsupportedVersion,
    NotHandshaked,
    UnknownNode,
    StaleGeneration,
    StrictModeViolation,
    Timeout,
    Unsupported,
    Internal,
    /// M6 (§16 v0.4): `Window::Use` named a handle the session does not
    /// know (never existed or closed). Append-only — see the pinning test.
    UnknownWindow,
}

/// Adapter self-description reported in `Welcome` (§6.1 `AdapterMeta`).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AdapterInfoV1 {
    pub name: String,
    pub framework: String,
    pub framework_version: String,
    /// Capability flags (§6.2 capability matrix).
    pub caps: CapsV1,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CapsV1 {
    pub tree_update: bool,
    pub logical_snapshot: bool,
    pub input: bool,
    pub last_frame: bool,
    pub render_frame: bool,
    pub clock: bool,
}

/// Wire form of [`guiwright_core::NodeKey`] (§6.1, normalized identities).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum NodeKeyV1 {
    AccessKit {
        /// AccessKit `TreeId` is a Uuid on the wire.
        tree: [u8; 16],
        node: u64,
    },
    Logical {
        adapter: String,
        id: u64,
    },
}

/// Wire form of [`guiwright_core::Selector`] (§8.1 `SelectorV1`).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum SelectorV1 {
    Role {
        /// `accesskit::Role` as u8 discriminant string for forward
        /// compatibility we serialize the Debug-ish name.
        role: String,
        name: Option<TextMatchV1>,
    },
    Label(TextMatchV1),
    Text(TextMatchV1),
    TestId(String),
    Value(TextMatchV1),
    All(Vec<SelectorV1>),
    Filter {
        source: Box<SelectorV1>,
        has_text: Option<TextMatchV1>,
        has_not: Option<Box<SelectorV1>>,
        nth: Option<NthV1>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TextMatchV1 {
    pub text: String,
    pub exact: bool,
    pub ignore_case: bool,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum NthV1 {
    First,
    Last,
    Index(usize),
}

/// Wire form of [`guiwright_driver::SemAction`] (§8.4).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum SemActionV1 {
    Click,
    DoubleClick,
    SetValue { text: String },
    Focus,
    Blur,
    Hover,
    Press { key: String, modifiers: Vec<String> },
    Scroll { dx: f64, dy: f64 },
    Check,
    Uncheck,
}

/// Wire form of [`guiwright_driver::InputEvent`] (§9).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum InputEventV1 {
    PointerDown {
        x: f64,
        y: f64,
        button: PointerButtonV1,
    },
    PointerUp {
        x: f64,
        y: f64,
        button: PointerButtonV1,
    },
    PointerMove {
        x: f64,
        y: f64,
    },
    Wheel {
        dx: f64,
        dy: f64,
    },
    KeyDown {
        key: String,
        modifiers: Vec<String>,
    },
    KeyUp {
        key: String,
        modifiers: Vec<String>,
    },
    /// Text commit (IME-backed on some adapters, §4.2 keyboard fidelity).
    TextInput {
        text: String,
    },
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum PointerButtonV1 {
    Primary,
    Secondary,
    Middle,
}

impl From<guiwright_core::NodeKey> for NodeKeyV1 {
    fn from(key: guiwright_core::NodeKey) -> Self {
        use guiwright_core::NodeKey;
        match key {
            NodeKey::AccessKit { tree, node } => NodeKeyV1::AccessKit {
                tree: tree.0.as_bytes().to_owned(),
                node: node.0,
            },
            NodeKey::Logical { adapter, id } => NodeKeyV1::Logical {
                adapter: adapter.to_string(),
                id,
            },
        }
    }
}

impl From<NodeKeyV1> for guiwright_core::NodeKey {
    fn from(v: NodeKeyV1) -> Self {
        use guiwright_core::NodeKey;
        match v {
            NodeKeyV1::AccessKit { tree, node } => NodeKey::AccessKit {
                tree: accesskit::TreeId(uuid::Uuid::from_bytes(tree)),
                node: accesskit::NodeId(node),
            },
            NodeKeyV1::Logical { adapter, id } => {
                // Box-leak to a &'static str: adapter names are static
                // registry keys, and NodeKey is Copy. The count of distinct
                // adapters is bounded and tiny.
                NodeKey::Logical {
                    adapter: leak_adapter_name(adapter),
                    id,
                }
            }
        }
    }
}

use std::collections::HashSet;
use std::sync::Mutex;
use std::sync::OnceLock;

fn adapter_registry() -> &'static Mutex<HashSet<&'static str>> {
    static REGISTRY: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashSet::new()))
}

fn leak_adapter_name(name: String) -> &'static str {
    // Poison recovery, deliberately: the registry holds plain leaked
    // strings, so a poisoned lock (a panic in some other decoder thread
    // while holding the guard) leaves the data valid — the set is only
    // ever grown with dedup-checked names. Abandoning the conversation
    // over an already-consistent table would trade a recoverable state
    // for a panic in a decode path; errors carry context, panics carry
    // nothing.
    let mut registry = match adapter_registry().lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    if let Some(existing) = registry.get(name.as_str()) {
        return existing;
    }
    let leaked: &'static str = Box::leak(name.into_boxed_str());
    registry.insert(leaked);
    leaked
}

#[cfg(test)]
mod tests {
    use super::*;
    use guiwright_core::{NodeKey, Selector, TextMatch};

    #[test]
    fn node_key_roundtrip_accesskit() {
        let key = NodeKey::AccessKit {
            tree: accesskit::TreeId::ROOT,
            node: accesskit::NodeId(42),
        };
        let wire: NodeKeyV1 = key.into();
        let back: NodeKey = wire.into();
        assert_eq!(key, back);
    }

    #[test]
    fn node_key_roundtrip_logical() {
        let key = NodeKey::Logical {
            adapter: "iced",
            id: 7,
        };
        let wire: NodeKeyV1 = key.into();
        let back: NodeKey = wire.into();
        assert_eq!(key, back);
        // Different payload, same adapter name → identical &'static.
        let again: NodeKey = NodeKeyV1::Logical {
            adapter: "iced".into(),
            id: 9,
        }
        .into();
        let NodeKey::Logical { adapter, .. } = again else {
            panic!("logical");
        };
        let NodeKey::Logical { adapter: first, .. } = back else {
            panic!("logical");
        };
        assert!(std::ptr::eq(adapter, first), "interned adapter name");
    }

    #[test]
    fn selector_roundtrip_preserves_matching_semantics() {
        let selector = Selector::role_named(accesskit::Role::Button, "Save").first();
        let wire: SelectorV1 = selector.clone().into();
        let back: Selector = wire.into();
        assert_eq!(selector, back);
    }

    #[test]
    fn text_match_v1_roundtrip() {
        let m = TextMatch::exact("Ada");
        let wire: TextMatchV1 = m.clone().into();
        let back: TextMatch = wire.into();
        assert_eq!(m, back);
    }

    #[test]
    fn postcard_roundtrip_all_requests() {
        let requests = vec![
            GwpRequest::V1(RequestV1::Hello {
                proto_version: PROTO_VERSION,
                agent: "guiwright-kit/0.1".into(),
            }),
            GwpRequest::V1(RequestV1::Ping),
            GwpRequest::V1(RequestV1::TreeSnapshot),
            GwpRequest::V1(RequestV1::Resolve {
                selector: SelectorV1::TestId("save".into()),
                timeout_ms: 5000,
            }),
            GwpRequest::V1(RequestV1::Action {
                node: NodeKeyV1::AccessKit {
                    tree: [0u8; 16],
                    node: 2,
                },
                generation: 0,
                action: SemActionV1::Click,
                force: false,
                timeout_ms: 5000,
            }),
            GwpRequest::V1(RequestV1::Input {
                events: vec![InputEventV1::PointerDown {
                    x: 10.0,
                    y: 20.0,
                    button: PointerButtonV1::Primary,
                }],
            }),
            GwpRequest::V1(RequestV1::Clock(ClockV1::FastForward { ms: 30_000 })),
            GwpRequest::V1(RequestV1::Mock(MockV1::ResetState {
                scope: "seed-42".into(),
            })),
            GwpRequest::V1(RequestV1::Mock(MockV1::InstallSeam {
                id: "reset_db".into(),
                response: None,
            })),
            GwpRequest::V1(RequestV1::Inspect(InspectV1::StartRecording)),
            GwpRequest::V1(RequestV1::Bye),
        ];
        for req in requests {
            let bytes = postcard::to_allocvec(&req).expect("encode");
            let back: GwpRequest = postcard::from_bytes(&bytes).expect("decode");
            assert_eq!(req, back);
        }
    }

    #[test]
    fn postcard_roundtrip_all_responses() {
        let responses = vec![
            GwpResponse::V1(ResponseV1::Welcome {
                proto_version: PROTO_VERSION,
                session_id: "s-1".into(),
                tree_seq: 3,
                frame_seq: 17,
                adapter: AdapterInfoV1 {
                    name: "egui".into(),
                    framework: "egui".into(),
                    framework_version: "0.37.0".into(),
                    caps: CapsV1 {
                        tree_update: true,
                        input: true,
                        clock: true,
                        ..Default::default()
                    },
                },
            }),
            GwpResponse::V1(ResponseV1::Pong),
            GwpResponse::V1(ResponseV1::Resolved {
                node: NodeKeyV1::Logical {
                    adapter: "iced".into(),
                    id: 5,
                },
                generation: 1,
                rect: Some([0.0, 0.0, 100.0, 40.0]),
            }),
            GwpResponse::V1(ResponseV1::ActionDone {
                frame_seq: 18,
                detail: "clicked".into(),
            }),
            GwpResponse::V1(ResponseV1::ClockState {
                virtual_time_ms: Some(30_000),
                paused: false,
                steps: 2,
            }),
            GwpResponse::V1(ResponseV1::MockDone {
                id: "reset_db".into(),
                detail: "reset to seed-42".into(),
            }),
            GwpResponse::V1(ResponseV1::Codegen {
                code: "app.get_by_test_id(\"save\").click()".into(),
                warnings: vec![],
            }),
            GwpResponse::V1(ResponseV1::Error {
                code: ErrorCode::UnsupportedVersion,
                message: "client sent proto 0".into(),
            }),
        ];
        for resp in responses {
            let bytes = postcard::to_allocvec(&resp).expect("encode");
            let back: GwpResponse = postcard::from_bytes(&bytes).expect("decode");
            assert_eq!(resp, back);
        }
    }

    #[test]
    fn events_roundtrip() {
        let events = vec![
            GwpEvent::V1(EventV1::TreeChanged { seq: 9 }),
            GwpEvent::V1(EventV1::FramePresented {
                seq: 10,
                hash: 0xAB,
            }),
            GwpEvent::V1(EventV1::Log {
                record: "hello".into(),
            }),
        ];
        for ev in events {
            let bytes = postcard::to_allocvec(&ev).expect("encode");
            let back: GwpEvent = postcard::from_bytes(&bytes).expect("decode");
            assert_eq!(ev, back);
        }
    }

    /// §7 event-subscription domain round-trip (M12): the `Subscribe`
    /// request vocabulary, the pushed-event reply, and the subscription
    /// acknowledgement survive postcard both ways.
    #[test]
    fn subscribe_and_event_replies_roundtrip() {
        let requests = [
            RequestV1::Subscribe(SubscribeV1::On {
                kinds: vec![EventKindV1::TreeChanged, EventKindV1::FramePresented],
            }),
            RequestV1::Subscribe(SubscribeV1::On {
                kinds: vec![EventKindV1::Log],
            }),
            RequestV1::Subscribe(SubscribeV1::On { kinds: Vec::new() }),
            RequestV1::Subscribe(SubscribeV1::Off),
        ];
        for req in requests {
            let bytes = postcard::to_allocvec(&req).expect("encode");
            let back: RequestV1 = postcard::from_bytes(&bytes).expect("decode");
            assert_eq!(req, back);
        }
        let responses = [
            GwpResponse::V1(ResponseV1::Event(EventV1::TreeChanged { seq: 4 })),
            GwpResponse::V1(ResponseV1::Event(EventV1::FramePresented {
                seq: 7,
                hash: 0xDEAD_BEEF,
            })),
            GwpResponse::V1(ResponseV1::Subscribed {
                kinds: vec![EventKindV1::TreeChanged, EventKindV1::FramePresented],
            }),
            GwpResponse::V1(ResponseV1::Subscribed { kinds: Vec::new() }),
        ];
        for resp in responses {
            let bytes = postcard::to_allocvec(&resp).expect("encode");
            let back: GwpResponse = postcard::from_bytes(&bytes).expect("decode");
            assert_eq!(resp, back);
        }
    }

    /// §7 clock & mock domains round-trip (M2).
    #[test]
    fn clock_and_mock_domains_roundtrip() {
        let clocks = [
            ClockV1::InstallVirtual,
            ClockV1::FastForward { ms: 16 },
            ClockV1::RunFor { ms: 500 },
            ClockV1::Pause,
            ClockV1::Resume,
            ClockV1::SetTime { ms: 120_000 },
            ClockV1::State,
        ];
        for cmd in clocks {
            let bytes = postcard::to_allocvec(&cmd).expect("encode");
            let back: ClockV1 = postcard::from_bytes(&bytes).expect("decode");
            assert_eq!(cmd, back);
        }
        let mocks = [
            MockV1::InstallSeam {
                id: "reset_db".into(),
                response: Some("fixture-ok".into()),
            },
            MockV1::Clear {
                id: "reset_db".into(),
            },
            MockV1::ResetState {
                scope: "seed-42".into(),
            },
            MockV1::Call {
                id: "seed_users".into(),
                scope: "smoke".into(),
            },
        ];
        for cmd in mocks {
            let bytes = postcard::to_allocvec(&cmd).expect("encode");
            let back: MockV1 = postcard::from_bytes(&bytes).expect("decode");
            assert_eq!(cmd, back);
        }
    }

    /// §7 `Video` domain round-trip (M3, §10).
    #[test]
    fn video_domain_roundtrip() {
        let requests = [
            GwpRequest::V1(RequestV1::Video(VideoV1::Start(VideoOptsV1 {
                fps: 30,
                dir: Some("test-results/videos".into()),
            }))),
            GwpRequest::V1(RequestV1::Video(VideoV1::Start(VideoOptsV1::default()))),
            GwpRequest::V1(RequestV1::Video(VideoV1::Stop)),
        ];
        for req in requests {
            let bytes = postcard::to_allocvec(&req).expect("encode");
            let back: GwpRequest = postcard::from_bytes(&bytes).expect("decode");
            assert_eq!(req, back);
        }
        let resp = GwpResponse::V1(ResponseV1::VideoFile {
            path: "guiwright-smoke-1730889600.webm".into(),
            frames: 42,
            dropped: 7,
            duration_ms: 1680,
        });
        let bytes = postcard::to_allocvec(&resp).expect("encode");
        let back: GwpResponse = postcard::from_bytes(&bytes).expect("decode");
        assert_eq!(resp, back);
    }

    /// §7 `Window` domain round-trip (M6, §16 v0.4 multi-window).
    #[test]
    fn window_domain_roundtrip() {
        let requests = [
            GwpRequest::V1(RequestV1::Window(WindowV1::List)),
            GwpRequest::V1(RequestV1::Window(WindowV1::Use {
                handle: "a6000000-0000-0000-0000-000000000001".into(),
            })),
        ];
        for req in requests {
            let bytes = postcard::to_allocvec(&req).expect("encode");
            let back: GwpRequest = postcard::from_bytes(&bytes).expect("decode");
            assert_eq!(req, back);
        }
        let responses = [
            GwpResponse::V1(ResponseV1::Windows {
                windows: vec![WindowInfoV1 {
                    handle: "a6000000-0000-0000-0000-000000000001".into(),
                    title: "Editor".into(),
                    focused: true,
                }],
            }),
            GwpResponse::V1(ResponseV1::WindowUsed {
                handle: "a6000000-0000-0000-0000-000000000001".into(),
            }),
        ];
        for resp in responses {
            let bytes = postcard::to_allocvec(&resp).expect("encode");
            let back: GwpResponse = postcard::from_bytes(&bytes).expect("decode");
            assert_eq!(resp, back);
        }
    }

    /// §7 `Gesture` domain round-trip (M7, §9 input pipeline): the
    /// high-level synthetic gestures and the `GestureDone` reply.
    #[test]
    fn gesture_domain_roundtrip() {
        let requests = [
            GwpRequest::V1(RequestV1::Gesture(GestureV1::Click {
                x: 12.5,
                y: 30.0,
                button: PointerButtonV1::Primary,
                count: 1,
            })),
            GwpRequest::V1(RequestV1::Gesture(GestureV1::Click {
                x: 4.0,
                y: 8.0,
                button: PointerButtonV1::Secondary,
                count: 2,
            })),
            GwpRequest::V1(RequestV1::Gesture(GestureV1::Drag {
                from_x: 10.0,
                from_y: 10.0,
                to_x: 110.0,
                to_y: 60.0,
                button: PointerButtonV1::Primary,
            })),
            GwpRequest::V1(RequestV1::Gesture(GestureV1::Wheel {
                dx: 0.0,
                dy: -120.0,
            })),
            GwpRequest::V1(RequestV1::Gesture(GestureV1::Key {
                key: "Enter".into(),
                modifiers: vec!["Shift".into()],
            })),
            GwpRequest::V1(RequestV1::Gesture(GestureV1::Type {
                text: "héllo wörld".into(),
            })),
        ];
        for req in requests {
            let bytes = postcard::to_allocvec(&req).expect("encode");
            let back: GwpRequest = postcard::from_bytes(&bytes).expect("decode");
            assert_eq!(req, back);
        }
        let responses = [GwpResponse::V1(ResponseV1::GestureDone {
            frame_seq: 20,
            detail: "clicked at (12.5, 30) — 3 events".into(),
        })];
        for resp in responses {
            let bytes = postcard::to_allocvec(&resp).expect("encode");
            let back: GwpResponse = postcard::from_bytes(&bytes).expect("decode");
            assert_eq!(resp, back);
        }
    }

    /// §7 render domain round-trip (M10): the `Screenshot` requests and
    /// the `Shot` reply (PNG payload survives postcard verbatim).
    #[test]
    fn screenshot_domain_roundtrip() {
        let requests = [
            GwpRequest::V1(RequestV1::Screenshot(ScreenshotV1::Window)),
            GwpRequest::V1(RequestV1::Screenshot(ScreenshotV1::Element {
                selector: SelectorV1::Role {
                    role: "Button".into(),
                    name: Some(TextMatchV1 {
                        text: "Save".into(),
                        exact: false,
                        ignore_case: true,
                    }),
                },
                timeout_ms: 5000,
            })),
        ];
        for req in requests {
            let bytes = postcard::to_allocvec(&req).expect("encode");
            let back: GwpRequest = postcard::from_bytes(&bytes).expect("decode");
            assert_eq!(req, back);
        }
        let responses = [GwpResponse::V1(ResponseV1::Shot {
            frame_seq: 7,
            width: 120,
            height: 32,
            png: vec![0x89, b'P', b'N', b'G', 0, 1, 2, 3],
        })];
        for resp in responses {
            let bytes = postcard::to_allocvec(&resp).expect("encode");
            let back: GwpResponse = postcard::from_bytes(&bytes).expect("decode");
            assert_eq!(resp, back);
        }
    }

    /// §7 tree-dump domain round-trip (M11): the `Snapshot::Full` request
    /// and the `TreeDump` reply (nested `SnapshotNodeV1` rows survive
    /// postcard verbatim).
    #[test]
    fn tree_dump_domain_roundtrip() {
        let requests = [GwpRequest::V1(RequestV1::Snapshot(SnapshotV1::Full))];
        for req in requests {
            let bytes = postcard::to_allocvec(&req).expect("encode");
            let back: GwpRequest = postcard::from_bytes(&bytes).expect("decode");
            assert_eq!(req, back);
        }
        let responses = [GwpResponse::V1(ResponseV1::TreeDump {
            tree_seq: 3,
            node_count: 2,
            focus: Some(NodeKeyV1::AccessKit {
                tree: [0u8; 16],
                node: 0,
            }),
            nodes: vec![SnapshotNodeV1 {
                key: NodeKeyV1::AccessKit {
                    tree: [0u8; 16],
                    node: 0,
                },
                role: "Window".into(),
                name: Some("Test App".into()),
                value: None,
                bounds: Some([0.0, 0.0, 480.0, 240.0]),
                children: 1,
            }],
        })];
        for resp in responses {
            let bytes = postcard::to_allocvec(&resp).expect("encode");
            let back: GwpResponse = postcard::from_bytes(&bytes).expect("decode");
            assert_eq!(resp, back);
        }
    }

    /// §7 hit-test read round-trip (M32): the point request and the
    /// `HitTest` reply (a `ResolvedMatchV1` row on a hit, `None` on a
    /// miss) survive postcard both ways.
    #[test]
    fn hit_test_domain_roundtrip() {
        let requests = [GwpRequest::V1(RequestV1::HitTest { x: 76.0, y: 32.0 })];
        for req in requests {
            let bytes = postcard::to_allocvec(&req).expect("encode");
            let back: GwpRequest = postcard::from_bytes(&bytes).expect("decode");
            assert_eq!(req, back);
        }
        let responses = [
            GwpResponse::V1(ResponseV1::HitTest {
                hit: Some(ResolvedMatchV1 {
                    node: NodeKeyV1::AccessKit {
                        tree: [0u8; 16],
                        node: 2,
                    },
                    generation: 0,
                    rect: Some([16.0, 16.0, 136.0, 48.0]),
                }),
            }),
            GwpResponse::V1(ResponseV1::HitTest { hit: None }),
        ];
        for resp in responses {
            let bytes = postcard::to_allocvec(&resp).expect("encode");
            let back: GwpResponse = postcard::from_bytes(&bytes).expect("decode");
            assert_eq!(resp, back);
        }
    }

    /// postcard encodes enum variants as a varint of the variant index, so
    /// **variant order is the wire format**. §7 "old clients get clean
    /// errors" → within a `V1` envelope variants are append-only: pre-M3
    /// requests keep discriminants 0–9 (`Bye` = 9), `Video` appends at
    /// 10, the M6 `Window` domain at 11, the M7 `Gesture` domain at 12,
    /// the M10 render domain at 13, the M11 tree-dump domain at 14 and
    /// the M12 event-subscription domain at 15; responses keep 0–10
    /// (`Bye` = 10), `VideoFile` appends at 11, the M6 window replies at
    /// 12–13, the M7 `GestureDone` reply at 14, the M10 `Shot` reply at
    /// 15, the M11 `TreeDump` reply at 16 and the M12 event replies
    /// (`Event`, `Subscribed`) at 17–18.
    /// Reordering or inserting mid-enum fails this test — add new variants
    /// at the end and extend the pin list instead.
    #[test]
    fn wire_discriminants_are_append_only() {
        request_pins();
        response_pins();
        service_pins();
        nested_domain_pins();
    }

    /// Request variant indices, pinned for other-language clients.
    fn request_pins() {
        fn discriminant<T: serde::Serialize>(value: &T) -> u64 {
            // The first byte(s) of a postcard enum are the varint variant
            // index; every variant index pinned here is < 128 (single byte).
            let bytes = postcard::to_allocvec(value).expect("encode");
            u64::from(bytes[0])
        }
        assert_eq!(
            discriminant(&RequestV1::Hello {
                proto_version: 1,
                agent: String::new()
            }),
            0
        );
        assert_eq!(discriminant(&RequestV1::Ping), 1);
        assert_eq!(discriminant(&RequestV1::TreeSnapshot), 2);
        assert_eq!(
            discriminant(&RequestV1::Resolve {
                selector: SelectorV1::TestId(String::new()),
                timeout_ms: 0
            }),
            3
        );
        assert_eq!(
            discriminant(&RequestV1::Action {
                node: NodeKeyV1::Logical {
                    adapter: String::new(),
                    id: 0
                },
                generation: 0,
                action: SemActionV1::Click,
                force: false,
                timeout_ms: 0
            }),
            4
        );
        assert_eq!(discriminant(&RequestV1::Input { events: Vec::new() }), 5);
        assert_eq!(discriminant(&RequestV1::Clock(ClockV1::State)), 6);
        assert_eq!(
            discriminant(&RequestV1::Mock(MockV1::Clear { id: String::new() })),
            7
        );
        assert_eq!(
            discriminant(&RequestV1::Inspect(InspectV1::StartRecording)),
            8
        );
        assert_eq!(discriminant(&RequestV1::Bye), 9);
        assert_eq!(discriminant(&RequestV1::Video(VideoV1::Stop)), 10);
        assert_eq!(discriminant(&RequestV1::Window(WindowV1::List)), 11);
        // M7 (§9): the gesture domain appends after Window.
        assert_eq!(
            discriminant(&RequestV1::Gesture(GestureV1::Wheel { dx: 0.0, dy: 0.0 })),
            12
        );
        // M10 (§7 render domain): the screenshot domain appends after Gesture.
        assert_eq!(
            discriminant(&RequestV1::Screenshot(ScreenshotV1::Window)),
            13
        );
        // M11 (§7 tree domain): the tree-dump domain appends after Screenshot.
        assert_eq!(discriminant(&RequestV1::Snapshot(SnapshotV1::Full)), 14);
        // M12 (§7 event channel): the subscription domain appends after Snapshot.
        assert_eq!(
            discriminant(&RequestV1::Subscribe(SubscribeV1::On { kinds: Vec::new() })),
            15
        );
        // M30 (§8.2's set read): the resolve-all domain appends after Subscribe.
        assert_eq!(
            discriminant(&RequestV1::ResolveAll {
                selector: SelectorV1::TestId(String::new()),
            }),
            16
        );
        // M32 (the hit-test read): the hit-test domain appends after ResolveAll.
        assert_eq!(discriminant(&RequestV1::HitTest { x: 0.0, y: 0.0 }), 17);
        // M36 (the node-describe read): the describe request appends after HitTest.
        assert_eq!(
            discriminant(&RequestV1::Describe(DescribeV1 {
                node: NodeKeyV1::Logical {
                    adapter: String::new(),
                    id: 0
                },
                generation: 0
            })),
            18
        );
    }

    /// Response variant indices, pinned for other-language clients.
    fn response_pins() {
        fn discriminant<T: serde::Serialize>(value: &T) -> u64 {
            // The first byte(s) of a postcard enum are the varint variant
            // index; every variant index pinned here is < 128 (single byte).
            let bytes = postcard::to_allocvec(value).expect("encode");
            u64::from(bytes[0])
        }
        assert_eq!(
            discriminant(&ResponseV1::Welcome {
                proto_version: 1,
                session_id: String::new(),
                tree_seq: 0,
                frame_seq: 0,
                adapter: AdapterInfoV1 {
                    name: String::new(),
                    framework: String::new(),
                    framework_version: String::new(),
                    caps: CapsV1::default()
                }
            }),
            0
        );
        assert_eq!(discriminant(&ResponseV1::Pong), 1);
        assert_eq!(
            discriminant(&ResponseV1::TreeSnapshot {
                tree_seq: 0,
                node_count: 0
            }),
            2
        );
        assert_eq!(
            discriminant(&ResponseV1::Resolved {
                node: NodeKeyV1::Logical {
                    adapter: String::new(),
                    id: 0
                },
                generation: 0,
                rect: None
            }),
            3
        );
        assert_eq!(
            discriminant(&ResponseV1::ActionDone {
                frame_seq: 0,
                detail: String::new()
            }),
            4
        );
    }

    /// Clock / mock / codegen / error / video service replies, in wire order.
    fn service_pins() {
        fn discriminant<T: serde::Serialize>(value: &T) -> u64 {
            // The first byte(s) of a postcard enum are the varint variant
            // index; every variant index pinned here is < 128 (single byte).
            let bytes = postcard::to_allocvec(value).expect("encode");
            u64::from(bytes[0])
        }
        assert_eq!(discriminant(&ResponseV1::InputQueued { frame_seq: 0 }), 5);
        assert_eq!(
            discriminant(&ResponseV1::ClockState {
                virtual_time_ms: None,
                paused: false,
                steps: 0
            }),
            6
        );
        assert_eq!(
            discriminant(&ResponseV1::MockDone {
                id: String::new(),
                detail: String::new()
            }),
            7
        );
        assert_eq!(
            discriminant(&ResponseV1::Codegen {
                code: String::new(),
                warnings: Vec::new()
            }),
            8
        );
        assert_eq!(
            discriminant(&ResponseV1::Error {
                code: ErrorCode::Internal,
                message: String::new()
            }),
            9
        );
        assert_eq!(discriminant(&ResponseV1::Bye), 10);
        assert_eq!(
            discriminant(&ResponseV1::VideoFile {
                path: String::new(),
                frames: 0,
                dropped: 0,
                duration_ms: 0
            }),
            11
        );
        assert_eq!(
            discriminant(&ResponseV1::Windows {
                windows: Vec::new()
            }),
            12
        );
        assert_eq!(
            discriminant(&ResponseV1::WindowUsed {
                handle: String::new()
            }),
            13
        );
        assert_eq!(
            discriminant(&ResponseV1::GestureDone {
                frame_seq: 0,
                detail: String::new()
            }),
            14
        );
        // M10 (§7 render domain): the Shot reply appends after GestureDone.
        assert_eq!(
            discriminant(&ResponseV1::Shot {
                frame_seq: 0,
                width: 0,
                height: 0,
                png: Vec::new()
            }),
            15
        );
        // M11 (§7 tree domain): the TreeDump reply appends after Shot.
        assert_eq!(
            discriminant(&ResponseV1::TreeDump {
                tree_seq: 0,
                node_count: 0,
                focus: None,
                nodes: Vec::new()
            }),
            16
        );
        // M12 (§7 event channel): pushed events ride the response envelope
        // (`Event`), the subscription ack appends after it.
        assert_eq!(
            discriminant(&ResponseV1::Event(EventV1::TreeChanged { seq: 0 })),
            17
        );
        assert_eq!(
            discriminant(&ResponseV1::Subscribed { kinds: Vec::new() }),
            18
        );
        // M30 (§8.2's set read): the resolve-all reply appends after Subscribed.
        assert_eq!(
            discriminant(&ResponseV1::ResolvedAll {
                matches: Vec::new()
            }),
            19
        );
        // M32 (the hit-test read): the hit-test reply appends after ResolvedAll.
        assert_eq!(discriminant(&ResponseV1::HitTest { hit: None }), 20);
    }

    /// The M39 describe-all pins — one statement per milestone keeps the
    /// append-only sequence a readable table (the m36 pin split's rule).
    #[test]
    fn m39_describe_all_pins() {
        fn discriminant<T: serde::Serialize>(value: &T) -> u64 {
            let bytes = postcard::to_allocvec(value).expect("encode");
            u64::from(bytes[0])
        }
        // M39 (the describe-all read): the domain appends after Describe.
        assert_eq!(
            discriminant(&RequestV1::DescribeAll(DescribeAllV1 {
                selector: SelectorV1::TestId(String::new()),
            })),
            19
        );
        assert_eq!(
            discriminant(&ResponseV1::DescribedAll { rows: Vec::new() }),
            22
        );
        assert_eq!(discriminant(&ResponseV1::Described(described_shell())), 21);
    }

    /// The M64 describe-all identity pins — the m62/m63 records'
    /// reserved wire change, appended after the M39 domain (one
    /// statement per milestone keeps the append-only sequence a readable
    /// table, the m36 pin split's rule).
    #[test]
    fn m64_describe_all_identity_pins() {
        fn discriminant<T: serde::Serialize>(value: &T) -> u64 {
            let bytes = postcard::to_allocvec(value).expect("encode");
            u64::from(bytes[0])
        }
        // M64 (the identity-fact widening): the domain appends after
        // DescribeAll, reusing its payload.
        assert_eq!(
            discriminant(&RequestV1::DescribeAllIdentity(DescribeAllV1 {
                selector: SelectorV1::TestId(String::new()),
            })),
            20
        );
        assert_eq!(
            discriminant(&ResponseV1::DescribedAllIdentity { rows: Vec::new() }),
            23
        );
    }

    /// The M64 identity row round-trips (the widening fact rides the
    /// struct, `postcard` both ways — the m11 pin shape at row width).
    #[test]
    fn described_identity_rows_round_trip() {
        let row = DescribedIdentityV1 {
            node: NodeKeyV1::Logical {
                adapter: "test".into(),
                id: 9,
            },
            generation: 4,
            role: "Button".into(),
            name: Some("Install".into()),
            value: None,
            bounds: Some([16.0, 204.0, 136.0, 236.0]),
            description: Some("Installs the latest build".into()),
        };
        let bytes = postcard::to_allocvec(&row).expect("encode");
        let back: DescribedIdentityV1 = postcard::from_bytes(&bytes).expect("decode");
        assert_eq!(back, row);
        // The M39 projection: the identity four, the description dropped
        // (the wider row's view beside the byte-pinned M39 encoding).
        let projected = DescribedV1::from(&row);
        assert_eq!(projected.name.as_deref(), Some("Install"));
        assert_eq!(projected.value, None);
    }

    /// The M36 describe pin's payload — a `DescribedV1` shell whose fields
    /// carry no information (the discriminant is the pinned fact; the
    /// payload shape rides the struct itself). Split out of the pin list
    /// so the append-only sequence stays one statement per milestone.
    fn described_shell() -> DescribedV1 {
        DescribedV1 {
            node: NodeKeyV1::Logical {
                adapter: String::new(),
                id: 0,
            },
            generation: 0,
            role: String::new(),
            name: None,
            value: None,
            bounds: None,
        }
    }

    /// Nested domain + `ErrorCode` indices, pinned for other-language clients.
    fn nested_domain_pins() {
        fn discriminant<T: serde::Serialize>(value: &T) -> u64 {
            // The first byte(s) of a postcard enum are the varint variant
            // index; every variant index pinned here is < 128 (single byte).
            let bytes = postcard::to_allocvec(value).expect("encode");
            u64::from(bytes[0])
        }
        // hard-code these tables (clients/typescript).
        assert_eq!(discriminant(&VideoV1::Start(VideoOptsV1::default())), 0);
        assert_eq!(discriminant(&VideoV1::Stop), 1);
        assert_eq!(discriminant(&ClockV1::InstallVirtual), 0);
        assert_eq!(discriminant(&ClockV1::State), 6);
        assert_eq!(
            discriminant(&MockV1::InstallSeam {
                id: String::new(),
                response: None
            }),
            0
        );
        assert_eq!(
            discriminant(&MockV1::Call {
                id: String::new(),
                scope: String::new()
            }),
            3
        );
        assert_eq!(discriminant(&InspectV1::StartRecording), 0);
        assert_eq!(discriminant(&InspectV1::StopRecording), 1);
        // M8 (§13): the overlay/highlight half of the Inspect domain —
        // appended, never reordered (clients/typescript/src/schema.ts).
        assert_eq!(discriminant(&InspectV1::EnableOverlay), 2);
        assert_eq!(discriminant(&InspectV1::DisableOverlay), 3);
        assert_eq!(
            discriminant(&InspectV1::Highlight {
                node: Some(NodeKeyV1::Logical {
                    adapter: String::new(),
                    id: 0
                })
            }),
            4
        );
        assert_eq!(discriminant(&WindowV1::List), 0);
        assert_eq!(
            discriminant(&WindowV1::Use {
                handle: String::new()
            }),
            1
        );
        assert_eq!(
            discriminant(&GestureV1::Click {
                x: 0.0,
                y: 0.0,
                button: PointerButtonV1::Primary,
                count: 1
            }),
            0
        );
        assert_eq!(
            discriminant(&GestureV1::Drag {
                from_x: 0.0,
                from_y: 0.0,
                to_x: 0.0,
                to_y: 0.0,
                button: PointerButtonV1::Primary
            }),
            1
        );
        assert_eq!(discriminant(&GestureV1::Wheel { dx: 0.0, dy: 0.0 }), 2);
        assert_eq!(
            discriminant(&GestureV1::Key {
                key: String::new(),
                modifiers: Vec::new()
            }),
            3
        );
        assert_eq!(
            discriminant(&GestureV1::Type {
                text: String::new()
            }),
            4
        );
        // M10 (§7 render domain): the Screenshot element arm appends after
        // the Window arm (clients/typescript/src/schema.ts).
        assert_eq!(
            discriminant(&RequestV1::Screenshot(ScreenshotV1::Element {
                selector: SelectorV1::TestId(String::new()),
                timeout_ms: 0
            })),
            13
        );
        assert_eq!(
            discriminant(&ScreenshotV1::Element {
                selector: SelectorV1::TestId(String::new()),
                timeout_ms: 0
            }),
            1
        );
        subscribe_pins();
    }

    /// The M12 subscription + M13 filtered-dump vocabulary, pinned for
    /// other-language clients (appended after the M11 snapshot table,
    /// `clients/typescript/src/schema.ts`).
    fn subscribe_pins() {
        fn discriminant<T: serde::Serialize>(value: &T) -> u64 {
            // The first byte(s) of a postcard enum are the varint variant
            // index; every variant index pinned here is < 128 (single byte).
            let bytes = postcard::to_allocvec(value).expect("encode");
            u64::from(bytes[0])
        }
        assert_eq!(
            discriminant(&SubscribeV1::On {
                kinds: vec![EventKindV1::TreeChanged]
            }),
            0
        );
        assert_eq!(discriminant(&SubscribeV1::Off), 1);
        // The event-kind vocabulary mirrors `EventV1`'s variant order —
        // appended, never reordered.
        assert_eq!(discriminant(&EventKindV1::TreeChanged), 0);
        assert_eq!(discriminant(&EventKindV1::FramePresented), 1);
        assert_eq!(discriminant(&EventKindV1::Log), 2);
        // M13 (§7 tree domain): the filtered-dump arm appends after `Full`.
        assert_eq!(
            discriminant(&SnapshotV1::Filtered {
                selector: SelectorV1::TestId(String::new()),
                timeout_ms: 0
            }),
            1
        );
        // ErrorCode is flat on the wire (u8 in Error payloads) — appending
        // UnknownWindow keeps the pre-M6 codes byte-identical.
        assert_eq!(discriminant(&ErrorCode::Internal), 7);
        assert_eq!(discriminant(&ErrorCode::UnknownWindow), 8);
    }
}
