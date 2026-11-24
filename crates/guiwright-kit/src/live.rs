//! The kit's live capture (the m18 record's remaining-gaps list: *"The
//! kit's `export_trace_viewer` still archives first. Wiring `LiveFeed`
//! into the kit's trace flow (live export during a test run) is a
//! kit-side follow-on; M18 ships the viewer-side capability"*).
//!
//! The archive-first trace flow answers for finished runs; the capture
//! answers for runs that are still going: attach the M18
//! [`LiveFeed`](guiwright_viewer::LiveFeed) over the remote-to-self
//! door (the m16 handle's clone-and-connect contract — the feed rides a
//! second connection beside the test body's), pump on its own thread
//! while the test acts over the same door, and `finish` writes M23's
//! `render_live_html` output stamped `gwtrace/v1 (live)` — the m87 CLI
//! shape (`guiwright live-html`), now at kit width, where the run
//! itself is the capture window.
//!
//! The thread boundary mirrors the driver's own: the door's conversation
//! is async, the kit's surface is sync — the capture owns a
//! current-thread runtime for the pump (the same shape the §15.1 bench
//! samples and the m87 emitter ride), the test thread never sees it.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use guiwright_viewer::{LiveChange, LiveFeed};

use crate::app::KitError;
use crate::remote::RemoteToSelf;

/// One live capture over a [`RemoteToSelf`] door: attach, pump during
/// the run, [`LiveCapture::finish`] writes the scrubber.
pub struct LiveCapture {
    /// The stop half; dropped by [`Self::finish`] — or by plain `drop`,
    /// which detaches the pump (the thread breaks on the disconnect)
    /// without touching the door.
    stop: Option<mpsc::Sender<()>>,
    /// The pump thread's one result: the rendered capture.
    result: mpsc::Receiver<Result<Captured, KitError>>,
    /// The dump count, shared so the test body can wait on the fact
    /// (the event-driven shape: the deadline is the budget, the fact is
    /// the release) instead of guessing a sleep.
    dumps: Arc<AtomicUsize>,
    adapter: String,
}

impl std::fmt::Debug for LiveCapture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveCapture")
            .field("adapter", &self.adapter)
            .field("dumps", &self.dumps.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

/// What the pump thread hands back at `finish` — the capture's facts
/// beside the rendered HTML.
struct Captured {
    html: String,
    dumps: usize,
    frames: u64,
    logs: u64,
}

/// A finished capture: the scrubber file plus what it saw.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveExport {
    /// The path the scrubber HTML was written to (the `finish` argument).
    pub path: PathBuf,
    /// The adapter the feed attached to (the `Welcome` facts).
    pub adapter: String,
    /// How many tree dumps landed — the attach dump plus one per
    /// `TreeChanged` auto-sync.
    pub dumps: usize,
    /// How many `FramePresented` events advanced the axis.
    pub frames: u64,
    /// How many log records joined the buffer.
    pub logs: u64,
}

impl RemoteToSelf {
    /// Attach the live feed over the door (a second connection beside
    /// the test body's) and start the pump thread: `TreeChanged`
    /// auto-syncs a fresh dump, `FramePresented` advances the frame
    /// axis, `Log` joins the buffer — the M18 accumulation. The pump
    /// runs until [`LiveCapture::finish`] (or the capture's drop) or
    /// the door closes; the session's end never eats the capture (the
    /// m87 rule — a quiet or closed door still leaves the entry-state
    /// scrubber).
    ///
    /// # Errors
    ///
    /// [`KitError::Io`] names a pump-thread spawn failure (the door
    /// itself is already up — the attach rides the thread, its wire
    /// refusals surface through `finish`).
    pub fn start_live(&self, test: impl Into<String>) -> Result<LiveCapture, KitError> {
        let connect = self.connect_half().clone();
        let agent = self.agent().to_string();
        let test = test.into();
        let adapter = self.adapter().name.clone();
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let (result_tx, result_rx) = mpsc::channel::<Result<Captured, KitError>>();
        let dumps = Arc::new(AtomicUsize::new(0));
        let dumps_in_thread = Arc::clone(&dumps);

        std::thread::Builder::new()
            .name("guiwright-kit-live".into())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(err) => {
                        let _ = result_tx.send(Err(err.into()));
                        return;
                    }
                };
                let captured = runtime.block_on(async {
                    let mut feed =
                        LiveFeed::new(guiwright_proto::DuplexClient::connect(&connect).await?);
                    let _info = feed.attach(&agent).await?;
                    feed.set_test(test);
                    dumps_in_thread.store(1, Ordering::Relaxed);
                    let mut dumps = 1_usize;
                    let mut frames = 0_u64;
                    let mut logs = 0_u64;
                    loop {
                        // Stop on a signal or on the sender's drop — both
                        // mean the test body is done with the capture.
                        match stop_rx.try_recv() {
                            Ok(()) | Err(mpsc::TryRecvError::Disconnected) => break,
                            Err(mpsc::TryRecvError::Empty) => {}
                        }
                        match feed.pump(Duration::from_millis(50)).await {
                            Ok(Some(LiveChange::TreeChanged { .. })) => {
                                dumps += 1;
                                dumps_in_thread.store(dumps, Ordering::Relaxed);
                            }
                            Ok(Some(LiveChange::FramePresented { .. })) => frames += 1,
                            Ok(Some(LiveChange::Log)) => logs += 1,
                            // A quiet window is not a failure — the capture
                            // runs while the test runs; more events may come.
                            Ok(None) => {}
                            // The door closed under the feed: keep what the
                            // capture saw (the m87 rule), never spin.
                            Err(_) => break,
                        }
                    }
                    let _ = feed.bye().await;
                    let html = feed.render_html();
                    Ok(Captured {
                        html,
                        dumps,
                        frames,
                        logs,
                    })
                });
                let _ = result_tx.send(captured);
            })?;

        Ok(LiveCapture {
            stop: Some(stop_tx),
            result: result_rx,
            dumps,
            adapter,
        })
    }
}

impl LiveCapture {
    /// How many tree dumps have landed so far (the attach dump counts).
    /// The fact a test waits on between actions and `finish`.
    pub fn snapshot_count(&self) -> usize {
        self.dumps.load(Ordering::Relaxed)
    }

    /// The adapter the feed attached to (the door's `Welcome` facts).
    pub fn adapter(&self) -> &str {
        &self.adapter
    }

    /// Stop the pump, render the scrubber and write the HTML to `out` —
    /// M23's `render_live_html` output, stamped `gwtrace/v1 (live)` with
    /// the adapter header. The export replays everything the feed saw
    /// up to this moment, offline, no server, no assets.
    ///
    /// # Errors
    ///
    /// [`KitError::Wire`]/[`KitError::Viewer`] carry the attach and pump
    /// refusals (a dead door before the first dump, a wire error
    /// mid-attach), [`KitError::Trace`] names a pump thread that ended
    /// without a capture, and [`KitError::Io`] names the file write.
    pub fn finish(mut self, out: impl Into<PathBuf>) -> Result<LiveExport, KitError> {
        // Drop the stop half first: the pump breaks on the disconnect
        // (or on an earlier signal) and hands the result over.
        drop(self.stop.take());
        let captured = self.result.recv().map_err(|_| {
            KitError::Trace("the live capture thread ended without a capture".into())
        })??;
        let out = out.into();
        std::fs::write(&out, &captured.html)?;
        Ok(LiveExport {
            path: out,
            adapter: self.adapter,
            dumps: captured.dumps,
            frames: captured.frames,
            logs: captured.logs,
        })
    }
}
