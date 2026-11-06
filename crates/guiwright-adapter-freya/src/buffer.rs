//! Frame-quantized event ordering (§4.3: freya "input is frame-quantized
//! by ragnarok's `EventBuffer` — respect **press→sync→move→release**
//! ordering").
//!
//! Freya applies queued events at frame boundaries; a click that arrives
//! as down+up within one frame must be delivered in the canonical order
//! (press, the frame sync, the pointer move, release). [`SyncEventBuffer`]
//! is the adapter-side scheduler that enforces that shape and keeps the
//! queue bounded (§5.1 "bounded queues").

/// A pending frame-quantized event (the adapter's neutral vocabulary; the
/// runner glue maps these onto `PlatformEvent` + sync flushes).
#[derive(Clone, Debug, PartialEq)]
pub enum PendingEvent {
    /// Pointer press (mouse down) at logical px.
    Press { x: f64, y: f64 },
    /// Pointer move at logical px.
    Move { x: f64, y: f64 },
    /// Pointer release at logical px.
    Release { x: f64, y: f64 },
    /// Wheel delta.
    Wheel { dx: f64, dy: f64 },
    /// Keyboard event (raw key + phase), see [`crate::keys::KeyEvent`].
    Keyboard(crate::keys::KeyEvent),
}

impl PendingEvent {
    /// Canonical rank (§4.3 press→sync→move→release; wheel rides with the
    /// move slot, keyboard events are their own stream and keep order).
    fn rank(&self) -> u8 {
        match self {
            PendingEvent::Press { .. } => 0,
            PendingEvent::Keyboard(_) => 1,
            PendingEvent::Move { .. } | PendingEvent::Wheel { .. } => 2,
            PendingEvent::Release { .. } => 3,
        }
    }
}

/// Bounded, ordering-checked queue feeding the runner's frame loop.
#[derive(Debug)]
pub struct SyncEventBuffer {
    pending: Vec<PendingEvent>,
    capacity: usize,
    dropped: u64,
}

impl Default for SyncEventBuffer {
    fn default() -> Self {
        Self::new(1024)
    }
}

impl SyncEventBuffer {
    /// `capacity` is the §5.1 bound: a runaway producer is dropped and
    /// counted, never allowed to balloon the UI thread's queue.
    pub fn new(capacity: usize) -> Self {
        Self {
            pending: Vec::new(),
            capacity: capacity.max(1),
            dropped: 0,
        }
    }

    /// Queue one event. Full buffer → drop + count (never blocks).
    pub fn push(&mut self, ev: PendingEvent) {
        if self.pending.len() >= self.capacity {
            self.dropped += 1;
            return;
        }
        self.pending.push(ev);
    }

    /// Drain in canonical order (stable: equal ranks keep arrival order —
    /// only cross-rank order is corrected, never intra-rank intent).
    pub fn drain(&mut self) -> Vec<PendingEvent> {
        self.pending.sort_by_key(PendingEvent::rank);
        std::mem::take(&mut self.pending)
    }

    pub fn len(&self) -> usize {
        self.pending.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    pub fn dropped(&self) -> u64 {
        self.dropped
    }
}

/// The canonical single-click sequence (§4.3): press, sync marker slot,
/// move, release — the move lands *between* sync and release exactly as
/// ragnarok's buffer expects.
pub fn click_sequence(x: f64, y: f64) -> [PendingEvent; 3] {
    [
        PendingEvent::Press { x, y },
        PendingEvent::Move { x, y },
        PendingEvent::Release { x, y },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::key_sequence;

    #[test]
    fn canonical_click_order_regardless_of_arrival() {
        let mut buffer = SyncEventBuffer::default();
        // Arrival: release first (a producer race), then press, then move.
        buffer.push(PendingEvent::Release { x: 8.0, y: 8.0 });
        buffer.push(PendingEvent::Press { x: 8.0, y: 8.0 });
        buffer.push(PendingEvent::Move { x: 8.0, y: 8.0 });
        let drained = buffer.drain();
        assert_eq!(
            drained,
            vec![
                PendingEvent::Press { x: 8.0, y: 8.0 },
                PendingEvent::Move { x: 8.0, y: 8.0 },
                PendingEvent::Release { x: 8.0, y: 8.0 },
            ],
            "press → move → release"
        );
    }

    #[test]
    fn equal_ranks_keep_arrival_order() {
        let mut buffer = SyncEventBuffer::default();
        buffer.push(PendingEvent::Move { x: 1.0, y: 1.0 });
        buffer.push(PendingEvent::Move { x: 2.0, y: 2.0 });
        buffer.push(PendingEvent::Move { x: 3.0, y: 3.0 });
        let drained = buffer.drain();
        assert_eq!(drained.len(), 3);
        assert_eq!(
            drained,
            vec![
                PendingEvent::Move { x: 1.0, y: 1.0 },
                PendingEvent::Move { x: 2.0, y: 2.0 },
                PendingEvent::Move { x: 3.0, y: 3.0 },
            ],
            "no intra-rank reordering"
        );
    }

    #[test]
    fn keyboard_events_rank_after_press_before_move() {
        let mut buffer = SyncEventBuffer::default();
        buffer.push(PendingEvent::Move { x: 0.0, y: 0.0 });
        buffer.push(PendingEvent::Keyboard(
            key_sequence("Enter", &[])[0].clone(),
        ));
        buffer.push(PendingEvent::Press { x: 0.0, y: 0.0 });
        let drained = buffer.drain();
        assert_eq!(drained[0].rank(), 0, "press first");
        assert_eq!(drained[1].rank(), 1, "keyboard second");
        assert_eq!(drained[2].rank(), 2, "move last");
    }

    #[test]
    fn bounded_queue_drops_and_counts() {
        let mut buffer = SyncEventBuffer::new(2);
        buffer.push(PendingEvent::Press { x: 0.0, y: 0.0 });
        buffer.push(PendingEvent::Press { x: 1.0, y: 1.0 });
        buffer.push(PendingEvent::Press { x: 2.0, y: 2.0 });
        buffer.push(PendingEvent::Press { x: 3.0, y: 3.0 });
        assert_eq!(buffer.len(), 2, "capacity bound");
        assert_eq!(buffer.dropped(), 2);
        assert!(!buffer.is_empty());
        assert_eq!(buffer.drain().len(), 2, "the queued two still deliver");
        assert_eq!(buffer.len(), 0);
    }

    #[test]
    fn click_sequence_is_the_documented_shape() {
        let seq = click_sequence(10.0, 20.0);
        assert_eq!(
            seq,
            [
                PendingEvent::Press { x: 10.0, y: 20.0 },
                PendingEvent::Move { x: 10.0, y: 20.0 },
                PendingEvent::Release { x: 10.0, y: 20.0 },
            ]
        );
        let mut buffer = SyncEventBuffer::default();
        for ev in seq {
            buffer.push(ev);
        }
        assert_eq!(buffer.drain().len(), 3);
    }

    #[test]
    fn keyboard_sequence_flushes_through_the_buffer() {
        let mut buffer = SyncEventBuffer::default();
        for ev in key_sequence("a", &["Control".into()]) {
            buffer.push(PendingEvent::Keyboard(ev));
        }
        let drained = buffer.drain();
        assert_eq!(drained.len(), 4, "ctrl↓ a↓ a↑ ctrl↑");
        assert_eq!(
            drained.last().expect("last").rank(),
            1,
            "keyboard stays in its rank slot"
        );
    }
}
