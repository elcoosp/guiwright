//! The event hub (§7 server-push events, M12): the session thread's
//! subscriber registry and fan-out point. Owned by the session thread —
//! subscriptions arrive through the command queue, publications happen at
//! the pump sites — so the registry needs no locking (the same
//! single-owner discipline as the `Session` itself).
//!
//! Backpressure is the §15.6 bounded queue: each connection holds an
//! `EVENT_QUEUE_CAPACITY` channel; a slow client's queue drops events
//! (`try_send` never blocks the UI thread) and the seq numbers on
//! [`EventV1::TreeChanged`] / [`EventV1::FramePresented`] expose the gap.

use tokio::sync::mpsc;

use guiwright_proto::schema::{EventKindV1, GwpEvent};

/// One subscribed connection: its filter and the queue the connection
/// loop drains (the bridge's `event_stream` receiver).
pub(crate) struct Subscriber {
    /// Stable per-connection id (the bridge's identity across
    /// re-subscribes).
    id: u64,
    /// The deduplicated kind filter; empty = subscribed to nothing.
    kinds: Vec<EventKindV1>,
    tx: mpsc::Sender<GwpEvent>,
}

/// The session thread's subscriber set.
#[derive(Default)]
pub(crate) struct EventHub {
    subs: Vec<Subscriber>,
}

impl Subscriber {
    pub(crate) fn new(id: u64, kinds: Vec<EventKindV1>, tx: mpsc::Sender<GwpEvent>) -> Self {
        Self { id, kinds, tx }
    }
}

impl EventHub {
    /// Register (or re-register — replaces the filter) a connection.
    pub(crate) fn subscribe(&mut self, sub: Subscriber) {
        if let Some(existing) = self.subs.iter_mut().find(|s| s.id == sub.id) {
            *existing = sub;
            return;
        }
        self.subs.push(sub);
    }

    /// Drop a connection's subscription (idempotent).
    pub(crate) fn unsubscribe(&mut self, id: u64) {
        self.subs.retain(|s| s.id != id);
    }

    /// Are there any subscribers at all? The session loop arms its frame
    /// tick only while this is `false`-to-`true` — sessions without
    /// subscribers keep the exact pre-M12 loop (zero cost, §15.3).
    pub(crate) fn is_empty(&self) -> bool {
        self.subs.is_empty()
    }

    /// Does any subscriber want this kind? The publish path drains the log
    /// tail unconditionally (a live tap, never a replay buffer) but builds
    /// [`EventV1::Log`] frames only while a subscriber named the kind.
    pub(crate) fn wants(&self, kind: EventKindV1) -> bool {
        self.subs.iter().any(|sub| sub.kinds.contains(&kind))
    }

    /// Fan one batch of events out to the matching subscribers. Returns
    /// the delivered count; closed connections unsubscribe themselves,
    /// full queues drop (and log once per drop, debug level).
    pub(crate) fn publish(&mut self, events: &[GwpEvent]) -> usize {
        let mut delivered = 0;
        self.subs.retain_mut(|sub| {
            for event in events {
                if !sub.kinds.iter().any(|kind| event_matches(event, *kind)) {
                    continue;
                }
                match sub.tx.try_send(event.clone()) {
                    Ok(()) => delivered += 1,
                    Err(mpsc::error::TrySendError::Full(_)) => {
                        tracing::debug!(
                            subscriber = sub.id,
                            "event queue full — event dropped (§15.6 backpressure)"
                        );
                    }
                    Err(mpsc::error::TrySendError::Closed(_)) => {
                        tracing::debug!(subscriber = sub.id, "subscriber went away");
                        return false; // remove the dead subscriber
                    }
                }
            }
            true
        });
        delivered
    }
}

/// Does `event` match a subscription kind? Mirrors [`EventV1`]'s variant
/// order one-to-one (append a kind → append a row here).
fn event_matches(event: &GwpEvent, kind: EventKindV1) -> bool {
    let GwpEvent::V1(event) = event;
    let event_kind = match event {
        guiwright_proto::schema::EventV1::TreeChanged { .. } => EventKindV1::TreeChanged,
        guiwright_proto::schema::EventV1::FramePresented { .. } => EventKindV1::FramePresented,
        guiwright_proto::schema::EventV1::Log { .. } => EventKindV1::Log,
    };
    event_kind == kind
}

#[cfg(test)]
mod tests {
    use super::*;
    use guiwright_proto::conn::EVENT_QUEUE_CAPACITY;
    use guiwright_proto::schema::EventV1;

    fn sub_with(id: u64, kinds: Vec<EventKindV1>) -> (Subscriber, mpsc::Receiver<GwpEvent>) {
        let (tx, rx) = mpsc::channel(EVENT_QUEUE_CAPACITY);
        (Subscriber { id, kinds, tx }, rx)
    }

    #[tokio::test]
    async fn fan_out_respects_the_kind_filter() {
        let mut hub = EventHub::default();
        let (both, mut both_rx) = sub_with(
            1,
            vec![EventKindV1::TreeChanged, EventKindV1::FramePresented],
        );
        let (frames_only, mut frames_rx) = sub_with(2, vec![EventKindV1::FramePresented]);
        hub.subscribe(both);
        hub.subscribe(frames_only);

        let delivered = hub.publish(&[
            GwpEvent::V1(EventV1::TreeChanged { seq: 3 }),
            GwpEvent::V1(EventV1::FramePresented { seq: 4, hash: 9 }),
        ]);
        assert_eq!(
            delivered, 3,
            "2 events to the both-filter + 1 to frames-only"
        );

        assert!(matches!(
            both_rx.recv().await,
            Some(GwpEvent::V1(EventV1::TreeChanged { seq: 3 }))
        ));
        assert!(matches!(
            both_rx.recv().await,
            Some(GwpEvent::V1(EventV1::FramePresented { .. }))
        ));
        assert!(matches!(
            frames_rx.recv().await,
            Some(GwpEvent::V1(EventV1::FramePresented { .. }))
        ));
    }

    #[tokio::test]
    async fn empty_filter_receives_nothing_and_resubscribe_replaces() {
        let mut hub = EventHub::default();
        let (silent, mut silent_rx) = sub_with(1, Vec::new());
        hub.subscribe(silent);
        assert_eq!(
            hub.publish(&[GwpEvent::V1(EventV1::TreeChanged { seq: 1 })]),
            0
        );

        // Re-subscribe with a real filter: replaces, not appends.
        let (replaced, mut replaced_rx) = sub_with(1, vec![EventKindV1::TreeChanged]);
        hub.subscribe(replaced);
        assert_eq!(
            hub.publish(&[GwpEvent::V1(EventV1::TreeChanged { seq: 2 })]),
            1
        );
        assert!(matches!(
            replaced_rx.recv().await,
            Some(GwpEvent::V1(EventV1::TreeChanged { seq: 2 }))
        ));
        assert!(silent_rx.try_recv().is_err(), "the old filter is gone");
    }

    #[tokio::test]
    async fn closed_subscribers_remove_themselves_on_publish() {
        let mut hub = EventHub::default();
        let (sub, rx) = sub_with(7, vec![EventKindV1::TreeChanged]);
        hub.subscribe(sub);
        drop(rx); // the connection went away

        assert_eq!(
            hub.publish(&[GwpEvent::V1(EventV1::TreeChanged { seq: 1 })]),
            0
        );
        assert!(hub.is_empty(), "the closed subscriber is gone");
        // A later publish does not resurrect or panic.
        assert_eq!(
            hub.publish(&[GwpEvent::V1(EventV1::TreeChanged { seq: 2 })]),
            0
        );
    }

    #[test]
    fn unsubscribe_is_idempotent() {
        let mut hub = EventHub::default();
        let (sub, _rx) = sub_with(3, vec![EventKindV1::TreeChanged]);
        hub.subscribe(sub);
        hub.unsubscribe(3);
        hub.unsubscribe(3);
        assert!(hub.is_empty());
    }
}
