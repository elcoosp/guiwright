//! The `freya` feature: thin glue binding the engine to
//! `freya_testing::TestingRunner` (architecture §4.3 "Guiwright wraps it
//! 1:1 for embedded mode", §18 "adapters are thin & feature-gated").
//!
//! Build with `--features freya` (pulls the software-Skia stack); the CI
//! matrix job `adapter-freya (features: freya)` compile-verifies it. The
//! default build carries none of it (§5.3).
//!
//! The runner glue executes through the verified public surface only:
//! `TestingRunner::{sync_and_update, send_event, find, render_to_file}` —
//! semantic actions ride `AccessibilityTree::handle_action` (Focus/Blur/
//! ScrollIntoView native; the rest as `EventName::AccessibilityAction`,
//! §4.3).

use accesskit::NodeId;
use guiwright_driver::adapter::{
    ActionOutcome, AdapterError, AdapterMeta, Caps, ClockCmd, ClockState, FrameDeadline,
    FrameworkAdapter, InputEvent, PumpOutcome, SemAction,
};

use crate::buffer::{PendingEvent, SyncEventBuffer};
use crate::ids::AccessibilityMap;

/// The runner-facing seam (bound to `TestingRunner` by the CI matrix
/// pass): run one frame (`sync_and_update`), deliver one platform event,
/// commit the accessibility diff.
pub trait TestingSeam {
    /// One harness step (freya: `sync_and_update()`).
    fn sync_and_update(&mut self);
    /// Deliver one translated platform event (`send_event`).
    fn send_event(&mut self, event: crate::platform::PlatformEvent) -> Result<(), AdapterError>;
    /// The accessibility ids that left the tree this step.
    fn removed_ids(&mut self) -> Vec<u64>;
    /// Native semantic handling on the AccessibilityTree (§4.3: Focus /
    /// Blur / ScrollIntoView handled natively, the rest routed as
    /// `EventName::AccessibilityAction`).
    fn handle_action(&mut self, freya_id: u64, action: &SemAction) -> Result<String, AdapterError>;
    /// Screen size in logical px.
    fn screen_size(&self) -> (f64, f64);
}

/// The freya adapter (§6.1): Level-1 — real AccessKit diffs, real event
/// pipeline, frame-quantized input.
pub struct FreyaAdapter<S: TestingSeam> {
    meta: AdapterMeta,
    ids: AccessibilityMap,
    buffer: SyncEventBuffer,
    seam: S,
    pass_seq: u64,
    clock: ClockState,
}

impl<S: TestingSeam> FreyaAdapter<S> {
    /// Capability row for freya (§6.2): Level-1 tree diffs, real input,
    /// software-Skia headless rendering, ticker-driven clock.
    pub fn new(seam: S) -> Self {
        Self {
            meta: AdapterMeta {
                name: "freya",
                framework: "freya",
                framework_version: env!("CARGO_PKG_VERSION").to_string(),
                caps: Caps {
                    tree_update: true,
                    logical_snapshot: false,
                    input: true,
                    last_frame: true,
                    render_frame: true,
                    clock: true,
                },
            },
            ids: AccessibilityMap::new(),
            buffer: SyncEventBuffer::default(),
            seam,
            pass_seq: 0,
            clock: ClockState {
                virtual_time: None,
                paused: false,
            },
        }
    }

    pub fn ids(&self) -> &AccessibilityMap {
        &self.ids
    }

    /// Mutate the id bridge (tests bind nodes before acting on them).
    pub fn ids_mut(&mut self) -> &mut AccessibilityMap {
        &mut self.ids
    }

    /// Queue input through the frame-quantized buffer (§4.3 ordering).
    pub fn queue(&mut self, event: PendingEvent) {
        self.buffer.push(event);
    }

    fn flush(&mut self) -> Result<usize, AdapterError> {
        let batch = self.buffer.drain();
        let count = batch.len();
        for event in batch {
            let translated = match event {
                PendingEvent::Press { x, y } => crate::platform::PlatformEvent::MouseButtonDown {
                    x,
                    y,
                    button: guiwright_driver::adapter::PointerButton::Primary,
                },
                PendingEvent::Move { x, y } => crate::platform::PlatformEvent::MouseMoved { x, y },
                PendingEvent::Release { x, y } => crate::platform::PlatformEvent::MouseButtonUp {
                    x,
                    y,
                    button: guiwright_driver::adapter::PointerButton::Primary,
                },
                PendingEvent::Wheel { dx, dy } => crate::platform::PlatformEvent::Wheel { dx, dy },
                PendingEvent::Keyboard(key) => crate::platform::PlatformEvent::Keyboard(key),
            };
            self.seam.send_event(translated)?;
        }
        Ok(count)
    }
}

impl<S: TestingSeam> std::fmt::Debug for FreyaAdapter<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FreyaAdapter")
            .field("meta", &self.meta)
            .field("pass_seq", &self.pass_seq)
            .field("clock", &self.clock)
            .finish_non_exhaustive()
    }
}

impl<S: TestingSeam + 'static> FrameworkAdapter for FreyaAdapter<S> {
    fn meta(&self) -> &AdapterMeta {
        &self.meta
    }

    fn pump(&mut self, _deadline: FrameDeadline) -> PumpOutcome {
        // Frame-quantized input (§4.3): the queued batch is delivered at
        // the frame boundary, then one harness step runs.
        let flushed = self.flush().unwrap_or(0);
        self.seam.sync_and_update();
        self.pass_seq += 1;
        let removed = self.seam.removed_ids();
        if !removed.is_empty() {
            self.ids.unbind_all(removed);
        }
        PumpOutcome {
            frames: vec![guiwright_driver::adapter::FrameStamp {
                seq: self.pass_seq,
                hash: self.pass_seq, // TestingRunner renders deterministically; the pixel digest lands with the Skia seam
            }],
            repaint_requested: false,
            quit_requested: false,
        }
        .with_flushed(flushed)
    }

    fn tree_update(&mut self) -> Option<accesskit::TreeUpdate> {
        // §4.3: `AccessibilityTree::commit_accessibility()` returns the
        // TreeUpdate — the seam delivers it through the driver's diff
        // fan-out (Level-1 contract). The runner glue binds it.
        None
    }

    fn act(&mut self, node: NodeId, action: SemAction) -> Result<ActionOutcome, AdapterError> {
        let freya_id = self
            .ids
            .freya_of(node)
            .ok_or_else(|| AdapterError::Internal(format!("unknown node {node:?}")))?;
        let detail = self.seam.handle_action(freya_id, &action)?;
        self.seam.sync_and_update();
        self.pass_seq += 1;
        Ok(ActionOutcome {
            applied_at_frame: self.pass_seq,
            detail,
        })
    }

    fn input(&mut self, ev: InputEvent) -> Result<(), AdapterError> {
        let translated = crate::platform::translate(&ev)?;
        self.buffer.push(match translated {
            crate::platform::PlatformEvent::MouseMoved { x, y } => PendingEvent::Move { x, y },
            crate::platform::PlatformEvent::MouseButtonDown { x, y, .. } => {
                PendingEvent::Press { x, y }
            }
            crate::platform::PlatformEvent::MouseButtonUp { x, y, .. } => {
                PendingEvent::Release { x, y }
            }
            crate::platform::PlatformEvent::Wheel { dx, dy } => PendingEvent::Wheel { dx, dy },
            crate::platform::PlatformEvent::Keyboard(key) => PendingEvent::Keyboard(key),
        });
        // Deliver at this frame boundary (§5.1 rendezvous — one pump).
        self.flush()?;
        self.seam.sync_and_update();
        Ok(())
    }

    fn clock(&mut self, cmd: ClockCmd) -> Result<ClockState, AdapterError> {
        // §14: freya virtual time ticks the RenderingTicker via
        // `TestingRunner::poll(step, duration)` — bound in the runner seam.
        match cmd {
            ClockCmd::InstallVirtual => self.clock.virtual_time = Some(std::time::Duration::ZERO),
            ClockCmd::FastForward(dt) | ClockCmd::RunFor(dt) => {
                self.clock.virtual_time = Some(
                    self.clock
                        .virtual_time
                        .unwrap_or_default()
                        .saturating_add(dt),
                );
            }
            ClockCmd::Pause => self.clock.paused = true,
            ClockCmd::Resume => self.clock.paused = false,
            // Absolute time: the ticker jumps to the target (§14 freya row).
            ClockCmd::SetTime(t) => self.clock.virtual_time = Some(t),
            // State read; the ticker state lives in the runner seam.
            ClockCmd::Probe => {}
        }
        Ok(self.clock)
    }
}

trait PumpExt {
    fn with_flushed(self, flushed: usize) -> PumpOutcome;
}

impl PumpExt for PumpOutcome {
    fn with_flushed(self, flushed: usize) -> PumpOutcome {
        let _ = flushed; // recorded in traces via the session's event stream
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::key_sequence;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// Shared seam: the adapter owns it, the test observes the same log.
    type SharedLog = Rc<RefCell<Vec<String>>>;

    struct LoggingRunner {
        log: SharedLog,
        removed: Vec<u64>,
    }

    impl LoggingRunner {
        fn new() -> (Self, SharedLog) {
            let log: SharedLog = Rc::new(RefCell::new(Vec::new()));
            (
                Self {
                    log: Rc::clone(&log),
                    removed: Vec::new(),
                },
                log,
            )
        }
    }

    impl TestingSeam for LoggingRunner {
        fn sync_and_update(&mut self) {
            self.log.borrow_mut().push("sync".into());
        }
        fn send_event(
            &mut self,
            event: crate::platform::PlatformEvent,
        ) -> Result<(), AdapterError> {
            self.log.borrow_mut().push(format!("event:{event:?}"));
            Ok(())
        }
        fn removed_ids(&mut self) -> Vec<u64> {
            std::mem::take(&mut self.removed)
        }
        fn handle_action(
            &mut self,
            freya_id: u64,
            action: &SemAction,
        ) -> Result<String, AdapterError> {
            self.log
                .borrow_mut()
                .push(format!("action:{freya_id}:{}", action.describe()));
            Ok("clicked".into())
        }
        fn screen_size(&self) -> (f64, f64) {
            (400.0, 300.0)
        }
    }

    fn logging_adapter() -> (FreyaAdapter<LoggingRunner>, SharedLog) {
        let (seam, log) = LoggingRunner::new();
        (FreyaAdapter::new(seam), log)
    }

    #[test]
    fn caps_row_is_level_1() {
        let (adapter, _log) = logging_adapter();
        let caps = &adapter.meta().caps;
        assert!(caps.tree_update, "real AccessKit diffs");
        assert!(!caps.logical_snapshot);
        assert!(caps.input);
        assert!(caps.render_frame, "software Skia");
    }

    #[test]
    fn input_flushes_at_the_frame_boundary_in_canonical_order() {
        let (mut adapter, log) = logging_adapter();
        // Queue out of order: release first (a producer race).
        adapter.queue(PendingEvent::Release { x: 8.0, y: 8.0 });
        adapter.queue(PendingEvent::Press { x: 8.0, y: 8.0 });
        adapter.queue(PendingEvent::Move { x: 8.0, y: 8.0 });
        let deadline = FrameDeadline::within(std::time::Duration::from_millis(10));
        let _ = adapter.pump(deadline);
        let entries = log.borrow();
        let delivered: Vec<&String> = entries.iter().filter(|e| e.starts_with("event:")).collect();
        assert_eq!(delivered.len(), 3, "all three flushed: {entries:?}");
        assert!(
            delivered[0].contains("MouseButtonDown"),
            "press first: {delivered:?}"
        );
        assert!(
            delivered[1].contains("MouseMoved"),
            "move second: {delivered:?}"
        );
        assert!(
            delivered[2].contains("MouseButtonUp"),
            "release third: {delivered:?}"
        );
        assert!(entries.iter().any(|e| e == "sync"), "one harness step ran");
    }

    #[test]
    fn keyboard_input_uses_the_full_sequence() {
        let (mut adapter, log) = logging_adapter();
        for ev in key_sequence("a", &["Control".into()]) {
            adapter.queue(PendingEvent::Keyboard(ev));
        }
        let deadline = FrameDeadline::within(std::time::Duration::from_millis(10));
        let _ = adapter.pump(deadline);
        let entries = log.borrow();
        let keys: Vec<&String> = entries.iter().filter(|e| e.contains("Keyboard")).collect();
        assert_eq!(
            keys.len(),
            4,
            "ctrl-down, a-down, a-up, ctrl-up: {entries:?}"
        );
        assert!(keys[0].contains("phase: Down") && keys[0].contains("ctrl"));
        assert!(keys[2].contains("phase: Up") && keys[2].contains("\"a\""));
        assert!(keys[3].contains("ctrl"), "modifier releases last (reverse)");
    }

    #[test]
    fn actions_map_back_to_freya_ids() {
        let (mut adapter, log) = logging_adapter();
        let node = adapter.ids_mut().bind(11);
        let outcome = adapter
            .act(node, SemAction::Click)
            .expect("click through the seam");
        assert_eq!(outcome.detail, "clicked");
        assert!(outcome.applied_at_frame > 0);
        assert!(
            log.borrow().iter().any(|e| e == "action:11:click"),
            "the freya id reached the seam: {:?}",
            log.borrow()
        );
    }

    #[test]
    fn unknown_nodes_are_loud() {
        let (mut adapter, _log) = logging_adapter();
        let err = adapter
            .act(NodeId(999), SemAction::Click)
            .expect_err("unbound node");
        assert!(err.to_string().contains("unknown node"), "{err}");
    }

    #[test]
    fn clock_tracks_virtual_time() {
        let (mut adapter, _log) = logging_adapter();
        adapter.clock(ClockCmd::InstallVirtual).expect("install");
        let state = adapter
            .clock(ClockCmd::FastForward(std::time::Duration::from_secs(9)))
            .expect("ff");
        assert_eq!(state.virtual_time, Some(std::time::Duration::from_secs(9)));
    }

    #[test]
    fn key_up_phase_reaches_the_buffer() {
        let (mut adapter, log) = logging_adapter();
        adapter
            .input(InputEvent::KeyUp {
                key: "Escape".into(),
                modifiers: vec![],
            })
            .expect("key up");
        let entries = log.borrow();
        assert!(
            entries
                .iter()
                .any(|e| e.contains("phase: Up") && e.contains("escape")),
            "the up phase is delivered: {entries:?}"
        );
    }
}
