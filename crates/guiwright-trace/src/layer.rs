//! `logs.ndjson` capture (§12: "`tracing` subscriber output — frameworks
//! already use tracing").
//!
//! Install with [`TraceLogLayer`]: every `tracing` event emitted while the
//! layer is registered becomes one NDJSON line on the writer thread
//! (dropped, never blocking, when the ring is full — logs are diagnostics).

use std::time::Instant;

use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;

use crate::recorder::TraceSink;

/// A `tracing_subscriber` layer feeding the recorder's `logs.ndjson`.
pub struct TraceLogLayer {
    sink: TraceSink,
    start: Instant,
    process_id: u32,
}

impl std::fmt::Debug for TraceLogLayer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TraceLogLayer")
            .field("process_id", &self.process_id)
            .finish_non_exhaustive()
    }
}

impl TraceLogLayer {
    pub fn new(sink: TraceSink) -> Self {
        Self {
            sink,
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
        // record_str delivers strings raw (no Debug quoting) — messages and
        // string fields land as plain text in the NDJSON line.
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

impl<S> tracing_subscriber::Layer<S> for TraceLogLayer
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = LineVisitor {
            message: None,
            fields: Vec::new(),
        };
        event.record(&mut visitor);
        // One NDJSON line per event; serde_json on the hot-ish path is fine
        // at trace volumes (diagnostics only, capped by the ring).
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
        for (k, v) in visitor.fields {
            fields.insert(k, serde_json::json!(v));
        }
        self.sink
            .log_line(serde_json::to_string(&fields).expect("log line json"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    #[test]
    fn layer_captures_events_as_ndjson() {
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("trace.zip");
        let options = crate::recorder::TraceOptions {
            output_path: path,
            test: "log_test".into(),
            app: "app".into(),
            adapter: crate::schema::AdapterRecord {
                name: "mock".into(),
                framework: "fixture".into(),
                framework_version: "0".into(),
            },
            seed: None,
            resolution: (10, 10),
            scale: 1.0,
            frame_budget_bytes: 1 << 20,
            ring_capacity: 256,
        };
        let (recorder, sink) = crate::recorder::TraceRecorder::start_with_sink(options);
        let layer = TraceLogLayer::new(sink);

        {
            let subscriber = tracing_subscriber::registry().with(layer);
            let _guard = subscriber.set_default();
            tracing::info!(window = "main", "app booted");
            tracing::warn!("low fps");
        }

        let file = recorder.stop().expect("stop");
        assert!(file.stats.log_lines >= 2, "{:?}", file.stats);
        // Reopen the archive and check the lines parse.
        let bytes = std::fs::read(&file.path).expect("zip");
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("zip");
        let logs = archive.by_name("logs.ndjson").expect("logs.ndjson");
        let text = std::io::read_to_string(logs).expect("utf8");
        let mut lines = text.lines().filter(|l| !l.is_empty());
        let first: serde_json::Value =
            serde_json::from_str(lines.next().expect("first line")).expect("ndjson");
        assert_eq!(first["level"], "INFO");
        assert_eq!(first["message"], "app booted");
        assert_eq!(first["window"], "main");
        let second: serde_json::Value =
            serde_json::from_str(lines.next().expect("second line")).expect("ndjson");
        assert_eq!(second["level"], "WARN");
        assert_eq!(second["message"], "low fps");
    }
}
