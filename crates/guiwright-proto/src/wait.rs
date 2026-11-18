//! The client-side wait (m42 — the m21 record's named follow-up):
//! Playwright's `locator.waitFor(state)` for remote suites. The
//! vocabulary and the row-set evaluation live here — a pure,
//! transport-blind core — while the wire composition (subscribe →
//! describe-all re-reads → wake on the §7 event channel) rides
//! [`crate::client::ClientConn::wait_for`] as a default method, so every
//! shipped client inherits it.
//!
//! Two boundaries keep this honest:
//!
//! * **No wire change.** The states below are a client-side vocabulary
//!   (never serialized): the wait composes the existing `Subscribe`,
//!   `DescribeAll` and event domains byte-for-byte as M12/M39 blessed
//!   them. A driver that predates M39 rejects `DescribeAll` with the
//!   typed `Unsupported` and the wait surfaces that rejection rather
//!   than degrading (the m41 fail-fast rule; the REPL's per-row fallback
//!   is the interactive surface's concession, not this one's).
//! * **The rows are the truth.** Evaluation sees exactly what the M39
//!   `describe_all` walk answers: a *visible* row is one whose bounds
//!   are present with area > 0 — the wire shape of the §8.3 visible
//!   check (hidden nodes ride the walk — the non-strict set semantics —
//!   but carry `bounds: None`, the fixture's "Invisible" button being
//!   the pinned case). `Stable` mirrors the core crate's
//!   `StabilityTracker` exactly: frame-fed only, bounds identical across
//!   two consecutive presented frames, reset on identity change (§8.2)
//!   and on a changed rect.

use crate::schema::{DescribedV1, NodeKeyV1};

/// Playwright's `waitFor(state)` vocabulary (§16 auto-waiting row), the
/// client-side mirror of the kit's `WaitState`. Same names, same
/// semantics, one wire-free surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WaitState {
    /// The node is present in the (consumer) tree.
    Attached,
    /// The node passes the §8.3 visible check's wire shape: a row whose
    /// bounds have area > 0.
    Visible,
    /// The node is hidden — satisfied by absence too (Playwright parity):
    /// no row of the set is visible.
    Hidden,
    /// Visible **and** bounds-identical across two consecutive presented
    /// frames (the §8.3 `Stable` semantics the M17 wire gate ships).
    Stable,
}

impl WaitState {
    /// Stable rendering for errors and traces (the kit's naming).
    pub fn describe(self) -> &'static str {
        match self {
            WaitState::Attached => "attached",
            WaitState::Visible => "visible",
            WaitState::Hidden => "hidden",
            WaitState::Stable => "stable",
        }
    }
}

/// One evaluation step's answer: either the wait is done — carrying the
/// row that satisfied it (a directly usable `RequestV1::Action` handle,
/// the dump-row rule, M11; `None` for the absence-shaped `Hidden`) — or
/// the reason it is still pending, which becomes the timeout error's
/// "last evaluation" line.
#[derive(Clone, Debug)]
pub enum Eval {
    /// The state holds. `Some(row)` for the positive states, `None` for
    /// `Hidden` (absence is the satisfying fact).
    Satisfied(Option<DescribedV1>),
    /// The world may still change so the state can pass; the reason names
    /// what the last evaluation found.
    Pending { reason: String },
}

/// The row-set evaluator behind [`crate::client::ClientConn::wait_for`].
/// Feed it fresh describe-all rows after every wake and it answers the
/// state machine; the wake kind decides which entry point runs (a tree
/// wake re-decides, a presented frame may also confirm `Stable`).
#[derive(Debug)]
pub struct WaitTracker {
    state: WaitState,
    /// The row the `Stable` tracker is watching (the set's first visible
    /// row, re-pinned whenever the set's head changes identity, §8.2).
    pin: Option<NodeKeyV1>,
    /// The last confirmed bounds of the pinned row (the core
    /// `StabilityTracker`'s baseline — identical bounds across two
    /// consecutive presented frames confirm).
    held: Option<[f64; 4]>,
}

/// The wire shape of the §8.3 visible check: bounds present *and* of
/// non-empty area. Degenerate rects (zero width or height) are not laid
/// out, whatever the row says.
fn visible_bounds(row: &DescribedV1) -> Option<[f64; 4]> {
    let bounds = row.bounds?;
    let [x0, y0, x1, y1] = bounds;
    (x1 > x0 && y1 > y0).then_some(bounds)
}

impl WaitTracker {
    /// A tracker for one wait, in the state it is waiting for.
    pub fn new(state: WaitState) -> Self {
        Self {
            state,
            pin: None,
            held: None,
        }
    }

    /// Tree-time evaluation: the entry read and every `TreeChanged` wake.
    /// For `Stable` this re-pins (identity change resets the tracker) but
    /// never confirms — only presented frames do.
    pub fn evaluate(&mut self, rows: &[DescribedV1]) -> Eval {
        self.step(rows, false)
    }

    /// Frame-time evaluation: a `FramePresented` wake. For the positive
    /// states this decides exactly like a tree wake; for `Stable` it is
    /// the only call that can confirm.
    pub fn observe_frame(&mut self, rows: &[DescribedV1]) -> Eval {
        self.step(rows, true)
    }

    fn step(&mut self, rows: &[DescribedV1], frame_wake: bool) -> Eval {
        match self.state {
            WaitState::Attached => self.attached_step(rows),
            WaitState::Visible => self.visible_step(rows),
            WaitState::Hidden => self.hidden_step(rows),
            WaitState::Stable => self.stable_step(rows, frame_wake),
        }
    }

    fn attached_step(&mut self, rows: &[DescribedV1]) -> Eval {
        match rows.first() {
            Some(row) => Eval::Satisfied(Some(row.clone())),
            None => Eval::Pending {
                reason: "no match yet (the set is empty)".into(),
            },
        }
    }

    fn visible_step(&mut self, rows: &[DescribedV1]) -> Eval {
        match rows
            .iter()
            .find_map(|row| visible_bounds(row).map(|bounds| (row, bounds)))
        {
            Some((row, _)) => Eval::Satisfied(Some(row.clone())),
            None if rows.is_empty() => Eval::Pending {
                reason: "no match yet (the set is empty)".into(),
            },
            None => Eval::Pending {
                reason: "the set's rows carry no bounds yet".into(),
            },
        }
    }

    fn hidden_step(&mut self, rows: &[DescribedV1]) -> Eval {
        let visible = rows
            .iter()
            .filter(|row| visible_bounds(row).is_some())
            .count();
        if visible == 0 {
            Eval::Satisfied(None)
        } else {
            Eval::Pending {
                reason: format!("{visible} visible row(s) remain"),
            }
        }
    }

    fn stable_step(&mut self, rows: &[DescribedV1], frame_wake: bool) -> Eval {
        let visible: Vec<(usize, [f64; 4])> = rows
            .iter()
            .enumerate()
            .filter_map(|(i, row)| visible_bounds(row).map(|bounds| (i, bounds)))
            .collect();
        let Some(&(idx, bounds)) = visible.first() else {
            // No laid-out row to watch: the tracker resets with the set.
            self.pin = None;
            self.held = None;
            return Eval::Pending {
                reason:
                    "no visible row — the stable check composes the visible precondition (§8.3)"
                        .into(),
            };
        };
        let node = rows[idx].node.clone();
        // Identity change (§8.2): a waiter resets on the node it tracked —
        // the pin moves to the set's current head, frames start over.
        if let Some(prev) = self.pin.as_ref().filter(|prev| *prev != &node).cloned() {
            self.pin = Some(node);
            self.held = None;
            return Eval::Pending {
                reason: format!("re-pinned — {prev:?} left the set; the frame tracker resets"),
            };
        }
        self.pin = Some(node);
        if !frame_wake {
            // A tree wake re-reads the set; the tracker is frame-fed only
            // (the core rule: "evaluated only on frame events").
            return Eval::Pending {
                reason: "pinned — waiting for presented frames to confirm the rect".into(),
            };
        }
        // Same pin, presented frame: first identical observation sets the
        // baseline, the next confirms — bounds identical across two
        // consecutive presented frames.
        if self.held == Some(bounds) {
            self.held = Some(bounds);
            return Eval::Satisfied(Some(rows[idx].clone()));
        }
        self.held = Some(bounds);
        Eval::Pending {
            reason: "waiting for the rect to hold on the next frame".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The guard the module docs pin: a degenerate rect is not a visible
    /// row, whatever its other fields say.
    #[test]
    fn zero_area_bounds_are_not_visible() {
        let mut row = DescribedV1 {
            node: NodeKeyV1::Logical {
                adapter: "test".into(),
                id: 1,
            },
            generation: 0,
            role: "Button".into(),
            name: Some("Save".into()),
            value: None,
            bounds: Some([16.0, 16.0, 16.0, 48.0]),
        };
        assert!(visible_bounds(&row).is_none());
        row.bounds = Some([16.0, 16.0, 136.0, 48.0]);
        assert!(visible_bounds(&row).is_some());
    }
}
