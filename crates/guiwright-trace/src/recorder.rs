//! The recorder (§12 "lock-free SPSC ring on the UI thread → writer
//! thread").
//!
//! M1 shape: the UI-thread side never blocks — every call is a `try_send`
//! into a bounded SPSC channel ([`std::sync::mpsc::sync_channel`]; one
//! producer, one consumer, capacity = ring). When the ring is full the
//! record is **dropped and counted** (`meta.records_dropped`), never
//! awaited: tracing must not stall the app. The writer thread owns all
//! buffering, applies the frame byte budget with oldest-first eviction
//! (§12 "ring buffer with sliding-window eviction; cap default 256 MB")
//! and assembles the zip archive on [`TraceRecorder::stop`].

use std::collections::VecDeque;
use std::io::Write as IoWrite;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError, sync_channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::schema::{
    AdapterRecord, ClockJump, FrameIndexEntry, SCHEMA, TraceEvent, TraceMeta, TraceStats,
    TreeSnapshot,
};

/// Default frame byte budget — §12 "cap default 256 MB, oldest-first
/// eviction".
pub const DEFAULT_FRAME_BUDGET_BYTES: usize = 256 * 1024 * 1024;

/// Default SPSC ring capacity (records in flight).
pub const DEFAULT_RING_CAPACITY: usize = 4096;

/// Recorder configuration (§12 archive options).
#[derive(Clone, Debug)]
pub struct TraceOptions {
    /// `guiwright-<test>-<ts>.zip` target path (§12).
    pub output_path: PathBuf,
    pub test: String,
    pub app: String,
    pub adapter: AdapterRecord,
    pub seed: Option<u64>,
    pub resolution: (u32, u32),
    pub scale: f32,
    pub frame_budget_bytes: usize,
    pub ring_capacity: usize,
}

impl TraceOptions {
    pub fn new(output_path: impl Into<PathBuf>, test: &str, adapter: AdapterRecord) -> Self {
        Self {
            output_path: output_path.into(),
            test: test.to_string(),
            app: String::new(),
            adapter,
            seed: None,
            resolution: (1280, 800),
            scale: 1.0,
            frame_budget_bytes: DEFAULT_FRAME_BUDGET_BYTES,
            ring_capacity: DEFAULT_RING_CAPACITY,
        }
    }
}

/// One record flowing through the ring (UI thread → writer thread).
enum Record {
    Event(TraceEvent),
    Snapshot(TreeSnapshot),
    Diff(crate::schema::TreeDiffRecord),
    Frame {
        seq: u64,
        hash: u64,
        png: Vec<u8>,
    },
    Log {
        line: String,
    },
    Clock(ClockJump),
    /// Teardown: the UI thread's final drop count (sent with a bounded
    /// blocking retry — `stop()` may wait, event paths may not).
    DropNotice(u64),
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TraceError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("zip writer error: {0}")]
    Zip(String),
    #[error("trace writer thread panicked: {0}")]
    Writer(String),
    #[error("trace already stopped")]
    Stopped,
}

/// Final result of a stopped trace.
#[derive(Clone, Debug)]
pub struct TraceFile {
    pub path: PathBuf,
    pub stats: TraceStats,
}

/// A side sender for cross-thread producers — the `tracing` layer (§12
/// `logs.ndjson`) and the remote host. Non-blocking by contract: a full
/// ring silently drops log lines (they are diagnostics, not evidence).
#[derive(Clone)]
pub struct TraceSink(SyncSender<Record>);

impl std::fmt::Debug for TraceSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TraceSink").finish_non_exhaustive()
    }
}

impl TraceSink {
    pub fn log_line(&self, line: String) {
        let _ = self.0.try_send(Record::Log { line });
    }
}

/// UI-thread handle (§12). `event`/`frame`/… are non-blocking by contract.
pub struct TraceRecorder {
    tx: Option<SyncSender<Record>>,
    start: Instant,
    seq: u64,
    dropped: u64,
    /// Writer shutdown flag — the writer polls this instead of waiting for
    /// full sender-EOF, so stray [`TraceSink`] clones can never wedge the
    /// archive (a sink may live on any thread for the whole session).
    stop_flag: Arc<AtomicBool>,
    join: Option<JoinHandle<Result<TraceFile, TraceError>>>,
}

impl std::fmt::Debug for TraceRecorder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TraceRecorder")
            .field("seq", &self.seq)
            .field("dropped", &self.dropped)
            .finish_non_exhaustive()
    }
}

impl TraceRecorder {
    /// Spawn the writer thread and return the UI-thread handle.
    pub fn start(options: TraceOptions) -> Self {
        Self::start_with_sink(options).0
    }

    /// Like [`TraceRecorder::start`], plus the [`TraceSink`] for the
    /// `tracing` layer (the subscriber usually lives on another thread).
    pub fn start_with_sink(options: TraceOptions) -> (Self, TraceSink) {
        let (tx, rx) = sync_channel::<Record>(options.ring_capacity.max(16));
        let start = Instant::now();
        let stop_flag = Arc::new(AtomicBool::new(false));
        let writer_flag = Arc::clone(&stop_flag);
        let join = std::thread::Builder::new()
            .name("gw-trace-writer".into())
            .spawn(move || writer_loop(rx, options, writer_flag))
            .expect("spawn gw-trace-writer");
        let sink = TraceSink(tx.clone());
        (
            Self {
                tx: Some(tx),
                start,
                seq: 0,
                dropped: 0,
                stop_flag,
                join: Some(join),
            },
            sink,
        )
    }

    fn push(&mut self, record: Record) {
        let Some(tx) = self.tx.as_ref() else {
            return; // stopped: late producers are no-ops
        };
        match tx.try_send(record) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => self.dropped += 1,
            Err(TrySendError::Disconnected(_)) => {}
        }
    }

    /// Record one `events.ndjson` line (§12: input events, semantic
    /// actions, protocol calls). Returns the event's sequence number.
    pub fn event(&mut self, frame_seq: u64, kind: &'static str, detail: impl Into<String>) -> u64 {
        self.seq += 1;
        let ev = TraceEvent {
            seq: self.seq,
            elapsed_ns: self.start.elapsed().as_nanos() as u64,
            frame_seq,
            kind,
            detail: detail.into(),
        };
        self.push(Record::Event(ev));
        self.seq
    }

    /// Offer a presented frame — hash-deduped by the writer (§12
    /// "identical frames skipped"). `png` is the encoded frame (codec tag
    /// in `meta.frame_codec`).
    pub fn frame(&mut self, seq: u64, hash: u64, png: Vec<u8>) {
        self.push(Record::Frame { seq, hash, png });
    }

    /// Record a logical-tree snapshot (`trees/<seq>.bin`, §12).
    pub fn tree_snapshot(&mut self, snapshot: TreeSnapshot) {
        self.push(Record::Snapshot(snapshot));
    }

    /// Record one incremental diff (`trees/diff.bin`, §12).
    pub fn tree_diff(
        &mut self,
        frame_seq: u64,
        added: Vec<u64>,
        updated: Vec<u64>,
        removed: Vec<u64>,
        focus_changed: bool,
    ) {
        self.push(Record::Diff(crate::schema::TreeDiffRecord {
            frame_seq,
            added,
            updated,
            removed,
            focus_changed,
        }));
    }

    /// Record a virtual-clock jump (`clock.bin`, §14).
    pub fn clock_jump(&mut self, jump: ClockJump) {
        self.push(Record::Clock(jump));
    }

    /// Finish the trace: flush the drop notice, drain the ring, assemble
    /// the archive and join the writer thread (§12). Idempotence: calling
    /// twice returns [`TraceError::Stopped`].
    pub fn stop(mut self) -> Result<TraceFile, TraceError> {
        let Some(tx) = self.tx.take() else {
            return Err(TraceError::Stopped);
        };
        // The recorder-side drop count must reach the writer before EOF so
        // it lands in meta.json. stop() is teardown — bounded blocking is
        // allowed here (event paths never do this).
        let mut notice = Err(TrySendError::Disconnected(Record::DropNotice(self.dropped)));
        for _ in 0..1_000 {
            match tx.try_send(Record::DropNotice(self.dropped)) {
                Ok(()) => {
                    notice = Ok(());
                    break;
                }
                Err(TrySendError::Full(rec)) => {
                    notice = Err(TrySendError::Full(rec));
                    std::thread::sleep(Duration::from_micros(200));
                }
                Err(e) => {
                    notice = Err(e);
                    break;
                }
            }
        }
        drop(tx); // recorder is done producing
        self.stop_flag.store(true, Ordering::Release);
        let join = self.join.take().expect("join handle present until stop");
        let file = match join.join() {
            Ok(Ok(file)) => Ok(file),
            Ok(Err(e)) => Err(e),
            Err(panic) => Err(TraceError::Writer(format!("{panic:?}"))),
        }?;
        let mut stats = file.stats;
        if notice.is_ok() {
            stats.records_dropped = self.dropped;
        }
        Ok(TraceFile {
            path: file.path,
            stats,
        })
    }
}

fn writer_loop(
    rx: Receiver<Record>,
    options: TraceOptions,
    stop_flag: Arc<AtomicBool>,
) -> Result<TraceFile, TraceError> {
    Writer::new(options).run(rx, &stop_flag)
}

struct Writer {
    options: TraceOptions,
    events: Vec<String>,
    logs: Vec<String>,
    snapshots: Vec<(u64, Vec<u8>)>,
    diffs: Vec<Vec<u8>>,
    frames: VecDeque<FrameRecord>,
    frame_bytes: usize,
    index: Vec<FrameIndexEntry>,
    clock: Vec<ClockJump>,
    last_hash: Option<u64>,
    stats: TraceStats,
    dropped: u64,
}

struct FrameRecord {
    seq: u64,
    hash: u64,
    png: Vec<u8>,
}

impl Writer {
    fn new(options: TraceOptions) -> Self {
        Self {
            options,
            events: Vec::new(),
            logs: Vec::new(),
            snapshots: Vec::new(),
            diffs: Vec::new(),
            frames: VecDeque::new(),
            frame_bytes: 0,
            index: Vec::new(),
            clock: Vec::new(),
            last_hash: None,
            stats: TraceStats::default(),
            dropped: 0,
        }
    }

    fn run(
        mut self,
        rx: Receiver<Record>,
        stop_flag: &AtomicBool,
    ) -> Result<TraceFile, TraceError> {
        // Poll-based drain: records until the stop flag (or sender EOF for
        // recorders without sinks), so a long-lived TraceSink clone cannot
        // stall the archive.
        loop {
            if stop_flag.load(Ordering::Acquire) {
                // One final non-blocking sweep for records already queued.
                while let Ok(record) = rx.try_recv() {
                    self.apply(record);
                }
                break;
            }
            match rx.recv_timeout(Duration::from_millis(20)) {
                Ok(record) => self.apply(record),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        self.finish()
    }

    fn apply(&mut self, record: Record) {
        match record {
            Record::Event(ev) => {
                self.stats.events += 1;
                self.events
                    .push(serde_json::to_string(&ev).expect("event json"));
            }
            Record::Log { line } => {
                self.stats.log_lines += 1;
                self.logs.push(line);
            }
            Record::Snapshot(snap) => {
                self.stats.tree_snapshots += 1;
                let bytes = lz4_flex::compress_prepend_size(
                    &postcard::to_allocvec(&snap).expect("snapshot postcard"),
                );
                self.snapshots.push((snap.frame_seq, bytes));
            }
            Record::Diff(diff) => {
                self.stats.tree_diffs += 1;
                self.diffs.push(lz4_flex::compress_prepend_size(
                    &postcard::to_allocvec(&diff).expect("diff postcard"),
                ));
            }
            Record::Clock(jump) => {
                self.stats.clock_jumps += 1;
                self.clock.push(jump);
            }
            Record::DropNotice(count) => self.dropped = count,
            Record::Frame { seq, hash, png } => {
                if self.last_hash == Some(hash) {
                    self.stats.frames_deduped += 1;
                    return;
                }
                self.last_hash = Some(hash);
                self.stats.frames += 1;
                self.frame_bytes += png.len();
                self.frames.push_back(FrameRecord { seq, hash, png });
                // §12: cap default 256 MB, oldest-first eviction.
                while self.frame_bytes > self.options.frame_budget_bytes {
                    match self.frames.pop_front() {
                        Some(evicted) => {
                            self.frame_bytes -= evicted.png.len();
                            self.stats.frames_evicted += 1;
                        }
                        None => break,
                    }
                }
            }
        }
    }

    fn finish(mut self) -> Result<TraceFile, TraceError> {
        self.index = self
            .frames
            .iter()
            .map(|f| FrameIndexEntry {
                seq: f.seq,
                hash: f.hash,
                offset: 0,
                len: f.png.len() as u32,
            })
            .collect();

        let meta = TraceMeta {
            schema: SCHEMA.into(),
            test: self.options.test.clone(),
            app: self.options.app.clone(),
            adapter: self.options.adapter.clone(),
            os: std::env::consts::OS.into(),
            arch: std::env::consts::ARCH.into(),
            seed: self.options.seed,
            resolution: self.options.resolution,
            scale: self.options.scale,
            frame_codec: "png".into(),
            frames_evicted: self.stats.frames_evicted,
            records_dropped: self.dropped,
        };
        let meta_json = serde_json::to_string_pretty(&meta).expect("meta json");

        if let Some(parent) = self.options.output_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = std::fs::File::create(&self.options.output_path)?;
        let mut zip = zip::ZipWriter::new(file);
        write_archive(
            &mut zip,
            &meta_json,
            &self.events,
            &self.logs,
            &self.snapshots,
            &self.diffs,
            &self.frames,
            &self.clock,
            &self.index,
        )?;
        let file = zip.finish().map_err(|e| TraceError::Zip(e.to_string()))?;
        let archive_bytes = file.metadata()?.len();

        self.stats.archive_bytes = archive_bytes;
        self.stats.records_dropped = self.dropped;
        Ok(TraceFile {
            path: self.options.output_path,
            stats: self.stats,
        })
    }
}

#[allow(clippy::too_many_arguments)]
fn write_archive(
    zip: &mut zip::ZipWriter<std::fs::File>,
    meta_json: &str,
    events: &[String],
    logs: &[String],
    snapshots: &[(u64, Vec<u8>)],
    diffs: &[Vec<u8>],
    frames: &VecDeque<FrameRecord>,
    clock: &[ClockJump],
    index: &[FrameIndexEntry],
) -> Result<(), TraceError> {
    // Stored entries — compression lives in the payloads (postcard+LZ4),
    // the writer thread never re-compresses compressed records (§12).
    let options: zip::write::SimpleFileOptions = Default::default();
    let mut put = |name: &str, data: &[u8]| -> Result<(), TraceError> {
        zip.start_file(name, options)
            .map_err(|e| TraceError::Zip(e.to_string()))?;
        zip.write_all(data)
            .map_err(|e| TraceError::Zip(e.to_string()))?;
        Ok(())
    };
    put("meta.json", meta_json.as_bytes())?;
    put("events.ndjson", join_lines(events).as_bytes())?;
    put("logs.ndjson", join_lines(logs).as_bytes())?;
    for (seq, bytes) in snapshots {
        put(&format!("trees/{seq}.bin"), bytes)?;
    }
    put("trees/diff.bin", &diffs.concat())?;
    for frame in frames {
        put(&format!("frames/{}.png", frame.seq), &frame.png)?;
    }
    put(
        "clock.bin",
        &postcard::to_allocvec(clock).expect("clock postcard"),
    )?;
    put(
        "index.bin",
        &postcard::to_allocvec(index).expect("index postcard"),
    )?;
    Ok(())
}

fn join_lines(lines: &[String]) -> String {
    let mut out = String::with_capacity(lines.iter().map(|l| l.len() + 1).sum());
    for line in lines {
        out.push_str(line);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests;
