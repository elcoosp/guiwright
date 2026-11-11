//! The log tail (§7 server-push events, the log-tail milestone): the
//! per-session `tracing` tap that turns the session thread's records into
//! [`EventV1::Log`] frames.
//!
//! The m12 remaining-gaps note named the shape verbatim — *"a per-session
//! `tracing` tap → `EventV1::Log`"* — and the wire half has been pinned
//! since the first schema: [`EventKindV1::Log`] mirrors
//! [`EventV1::Log { record }`], one `logs.ndjson`-shape JSON line per
//! record (the §12 `logs.ndjson` vocabulary: `ts_ns`, `level`, `target`,
//! `pid`, `message`, plus the event's structured fields). This module is
//! the driver half:
//!
//! * [`LogTap`] — the lossy producer end (cloneable, `Send + Sync`). The
//!   [`LogTapLayer`] feeds it from `tracing`; an app that wants its own
//!   records on the same stream can hold a clone and [`LogTap::send`]
//!   pre-formatted lines.
//! * [`LogTail`] — the consumer end, owned by the session thread (the
//!   single-owner discipline of §7/M12: no locking). The server's publish
//!   point drains it after every pump.
//!
//! Backpressure is the §15.6 discipline: a bounded channel
//! ([`LOG_TAIL_CAPACITY`]), `try_send` never blocks, a full tail drops
//! the incoming record and counts the drop — logs are diagnostics, not
//! evidence. The tap is **live, not a replay buffer**: a session drains
//! (and discards) its tail at every publish point even with no
//! subscriber, so records emitted before a `Log` subscription are never
//! replayed to it.
//!
//! Capture is per-session by construction: enabling the tail wraps the
//! session thread in a **thread-local** dispatcher
//! (`tracing::subscriber::set_default`) whose only layer is the tap.
//! Other threads keep the process subscriber, two sessions never see each
//! other's records, and a session spawned without the tail keeps the
//! byte-exact pre-M14 conversation plus the process-wide tracing
//! behavior. `TRACE`-level records are filtered (wire diagnostics at
//! `DEBUG` and above).

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::time::Instant;

/// The tail's bound (§15.6): the session thread never blocks on a slow
/// consumer — a full tail drops the incoming record.
pub const LOG_TAIL_CAPACITY: usize = 256;

/// The consumer end of the log tail: owned by the session thread, drained
/// at the publish points (§7, M12 — "publications happen at the pump
/// sites").
pub struct LogTail {
    rx: Receiver<String>,
    dropped: Arc<AtomicU64>,
}

impl std::fmt::Debug for LogTail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LogTail")
            .field("capacity", &LOG_TAIL_CAPACITY)
            .field("dropped", &self.dropped())
            .finish()
    }
}

/// The producer end of the log tail: cloneable, `Send + Sync`, lossy.
///
/// The server creates its tap internally when
/// [`crate::server::ServerOptions::log_tail`] is set (the
/// [`LogTapLayer`] feeds it from `tracing`); the public pair exists so
/// tests, benches and custom hosts can exercise the same mechanics.
/// A record is one `logs.ndjson`-shape JSON line (the §12 vocabulary).
pub struct LogTap {
    tx: SyncSender<String>,
    dropped: Arc<AtomicU64>,
}

impl std::fmt::Debug for LogTap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LogTap")
            .field("dropped", &self.dropped.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

impl LogTail {
    /// Build a tail + its tap, bounded at `capacity` records.
    pub fn new(capacity: usize) -> (Self, LogTap) {
        let (tx, rx) = sync_channel(capacity);
        let dropped = Arc::new(AtomicU64::new(0));
        (
            Self {
                rx,
                dropped: Arc::clone(&dropped),
            },
            LogTap { tx, dropped },
        )
    }

    /// Drain every buffered record, oldest first (a bounded live tap: the
    /// publish point calls this even with no `Log` subscriber, so records
    /// never accumulate to be replayed to a later subscriber).
    pub fn drain(&self) -> Vec<String> {
        let mut records = Vec::new();
        while let Ok(record) = self.rx.try_recv() {
            records.push(record);
        }
        records
    }

    /// Records dropped by a full tail since spawn (§15.6 honesty: the
    /// consumer can see the gap, the same way event `seq` numbers expose
    /// a slow queue's drops).
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

impl LogTap {
    /// Push one record onto the tail — lossy by contract: a full tail
    /// drops the incoming record and counts it, never blocks (§15.6).
    pub fn send(&self, record: impl Into<String>) {
        match self.tx.try_send(record.into()) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                self.dropped.fetch_add(1, Ordering::Relaxed);
            }
            // The tail (and its session) is gone — a stray tap clone is
            // inert, the same rule the trace crate's sink lives by.
            Err(TrySendError::Disconnected(_)) => {}
        }
    }

    /// Records dropped by a full tail since spawn.
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

/// The `tracing` half of the tap: a [`tracing_subscriber::Layer`] that
/// formats every event (at `DEBUG` and above) into one `logs.ndjson`-shape
/// JSON line and pushes it through the [`LogTap`].
///
/// The line vocabulary mirrors `guiwright-trace`'s `TraceLogLayer` (§12
/// `logs.ndjson`: `ts_ns`, `level`, `target`, `pid`, `message`, plus the
/// event's structured fields) so a remote consumer parses the same shape
/// the trace archive writes. The visitor is the established
/// `record_str`/`record_debug` split: strings land raw, `Debug` values
/// keep their quoting.
pub(crate) struct LogTapLayer {
    tap: LogTap,
    start: Instant,
    process_id: u32,
}

impl LogTapLayer {
    pub(crate) fn new(tap: LogTap) -> Self {
        Self {
            tap,
            start: Instant::now(),
            process_id: std::process::id(),
        }
    }
}

struct LineVisitor {
    message: Option<String>,
    fields: Vec<(String, String)>,
}

impl tracing::field::Visit for LineVisitor {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.message = Some(value.to_string());
        } else {
            self.fields
                .push((field.name().to_string(), value.to_string()));
        }
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = Some(format!("{value:?}"));
        } else {
            self.fields
                .push((field.name().to_string(), format!("{value:?}")));
        }
    }
}

impl<S> tracing_subscriber::Layer<S> for LogTapLayer
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
{
    fn enabled(
        &self,
        metadata: &tracing::Metadata<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) -> bool {
        // DEBUG and above; TRACE stays off the wire (the tail is remote
        // diagnostics, not a full trace recorder — that is §12's job).
        *metadata.level() <= tracing::Level::DEBUG
    }

    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let mut visitor = LineVisitor {
            message: None,
            fields: Vec::new(),
        };
        event.record(&mut visitor);
        let mut fields = serde_json::Map::new();
        fields.insert(
            "ts_ns".into(),
            serde_json::json!(self.start.elapsed().as_nanos() as u64),
        );
        fields.insert(
            "level".into(),
            serde_json::json!(event.metadata().level().to_string()),
        );
        fields.insert(
            "target".into(),
            serde_json::json!(event.metadata().target()),
        );
        fields.insert("pid".into(), serde_json::json!(self.process_id));
        fields.insert(
            "message".into(),
            serde_json::json!(visitor.message.unwrap_or_default()),
        );
        for (key, value) in visitor.fields {
            fields.insert(key, serde_json::json!(value));
        }
        self.tap
            .send(serde_json::to_string(&fields).unwrap_or_default());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    #[test]
    fn tap_to_tail_round_trips_in_order() {
        let (tail, tap) = LogTail::new(8);
        tap.send("first");
        tap.send("second");
        tap.send("third");
        assert_eq!(tail.drain(), ["first", "second", "third"]);
        assert_eq!(tail.dropped(), 0);
        assert!(tail.drain().is_empty(), "drain is exhaustive");
    }

    #[test]
    fn a_full_tail_drops_the_incoming_record_and_counts_it() {
        let (tail, tap) = LogTail::new(4);
        for i in 0..8 {
            tap.send(format!("record {i}"));
        }
        let drained = tail.drain();
        assert_eq!(drained.len(), 4, "the bound holds");
        assert_eq!(drained[0], "record 0", "the oldest records survive");
        assert_eq!(tail.dropped(), 4, "every drop is counted");
        assert_eq!(tap.dropped(), 4, "both ends report the same counter");
    }

    #[test]
    fn stray_tap_clones_stay_inert_after_the_tail_is_gone() {
        let (tail, tap) = LogTail::new(4);
        let clone = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(20));
            tap
        });
        drop(tail);
        let tap = clone.join().expect("the thread finishes");
        tap.send("after the tail is gone"); // must not panic
        assert_eq!(tap.dropped(), 0, "a closed channel is not a drop");
    }

    #[test]
    fn the_layer_formats_the_logs_ndjson_vocabulary() {
        let (tail, tap) = LogTail::new(16);
        let layer = LogTapLayer::new(tap);
        {
            let subscriber = tracing_subscriber::registry().with(layer);
            let _guard = subscriber.set_default();
            tracing::info!(window = "main", "app booted");
            tracing::warn!("low fps");
            tracing::trace!("must not reach the wire");
        }
        let records = tail.drain();
        assert_eq!(records.len(), 2, "TRACE stays filtered: {records:?}");

        let first: serde_json::Value =
            serde_json::from_str(&records[0]).expect("one JSON line per record");
        assert_eq!(first["level"], "INFO");
        assert_eq!(first["message"], "app booted");
        assert_eq!(first["window"], "main");
        assert_eq!(first["target"], "guiwright_driver::log_tail::tests");
        assert!(first["ts_ns"].is_u64(), "{first}");
        assert_eq!(first["pid"], serde_json::json!(std::process::id()));

        let second: serde_json::Value =
            serde_json::from_str(&records[1]).expect("one JSON line per record");
        assert_eq!(second["level"], "WARN");
        assert_eq!(second["message"], "low fps");
    }

    #[test]
    fn debug_values_keep_their_quoting() {
        let (tail, tap) = LogTail::new(4);
        let layer = LogTapLayer::new(tap);
        {
            let subscriber = tracing_subscriber::registry().with(layer);
            let _guard = subscriber.set_default();
            tracing::debug!(count = 3, "tick");
        }
        let records = tail.drain();
        let parsed: serde_json::Value =
            serde_json::from_str(&records[0]).expect("one JSON line per record");
        assert_eq!(parsed["count"], "3", "Debug fields land quoted: {parsed}");
    }
}
