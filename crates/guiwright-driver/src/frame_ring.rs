//! The session-side publisher behind the §15.6 shared-frame ring: every
//! presented frame is offered to a [`guiwright_proto::shmring`]
//! [`FrameRingWriter`](guiwright_proto::shmring::FrameRingWriter) so a
//! same-host client reads pixels outside the door (no socket hop, no
//! framing, no LZ4 — the §15.6 optimization beside the blob side-channel).
//!
//! The publisher never disturbs the session (§18): declines and ring
//! errors are counted (the honest [`RingStats::skipped`]) and logged at
//! `debug!`, never propagated — a broken or taken ring path degrades to a
//! session without the ring, exactly the §15.6 backpressure posture ("the
//! UI thread's only costs are the tap hooks"). The store materializes on
//! the first publishable frame (the adapter's declared geometry is the
//! contract), so pixel-less adapters leave no file behind.

use std::path::{Path, PathBuf};

use guiwright_proto::shmring::{FrameRingWriter, RingConfig, RingStats};
use tracing::debug;

use crate::adapter::FrameRef;

/// Publishes presented frames into the shared ring at a fixed path. Owned
/// by the [`Session`](crate::Session) (single-writer by construction — the
/// SPSC contract holds: one session thread offers, readers only read).
pub struct FrameRingPublisher {
    path: PathBuf,
    config: RingConfig,
    /// Created on the first publishable frame — the ring's declared
    /// geometry is the adapter's frame geometry, which the session only
    /// knows once a frame carries pixels.
    writer: Option<FrameRingWriter>,
    /// Writer stats once publishing, else the skip ledger alone.
    stats: RingStats,
}

impl std::fmt::Debug for FrameRingPublisher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FrameRingPublisher")
            .field("path", &self.path)
            .field("config", &self.config)
            .field("stats", &self.stats)
            .finish_non_exhaustive()
    }
}

impl FrameRingPublisher {
    /// A publisher for the ring at `path`, default geometry (the first
    /// frame's dimensions decide the slot size).
    pub fn new(path: PathBuf) -> Self {
        Self::with_config(path, RingConfig::new())
    }

    /// A publisher with an explicit slot budget (`slots`); the slot size
    /// still follows the first frame's geometry.
    pub fn with_config(path: PathBuf, config: RingConfig) -> Self {
        Self {
            path,
            config,
            writer: None,
            stats: RingStats::default(),
        }
    }

    /// The path the ring publishes at (or would publish at).
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Publication counters (published / wrapped / skipped).
    pub fn stats(&self) -> RingStats {
        self.stats
    }

    /// Offer one presented frame. `frame = None` (or a pixel-less frame —
    /// the §6.2 "hash-only `FrameRef`" shape) is a counted skip, not a
    /// silent drop.
    pub fn offer(&mut self, seq: u64, frame: Option<&FrameRef>) {
        let Some(frame) = frame else {
            self.skip(seq, "no frame from the adapter");
            return;
        };
        let Some(pixels) = frame.pixels.as_deref() else {
            self.skip(seq, "hash-only frame (no pixel path, §6.2)");
            return;
        };
        let (w, h) = frame.size;
        if self.writer.is_none() {
            // First publishable frame decides the declared geometry; the
            // caller's slot budget caps the slot count only.
            let config = RingConfig {
                slots: self.config.slots,
                slot_bytes: guiwright_proto::shmring::rgba_len(w, h),
            };
            match FrameRingWriter::create_with(&self.path, w, h, config) {
                Ok(writer) => {
                    debug!(path = %self.path.display(), w, h, "frame ring publishing");
                    self.writer = Some(writer);
                }
                Err(err) => {
                    debug!(
                        path = %self.path.display(),
                        %err,
                        "frame ring did not start; frames will not publish"
                    );
                    self.skip(seq, "ring creation failed");
                    return;
                }
            }
        }
        let writer = self
            .writer
            .as_mut()
            .expect("checked above: writer created or returned");
        match writer.publish(seq, pixels) {
            Ok(_outcome) => {
                self.stats = writer.stats();
            }
            Err(err) => {
                debug!(seq, %err, "frame ring publish failed; frame skipped");
                writer.note_skipped();
                self.stats = writer.stats();
            }
        }
    }

    fn skip(&mut self, seq: u64, why: &str) {
        self.stats.skipped += 1;
        debug!(seq, why, "frame ring skipped a presented frame");
        if let Some(writer) = self.writer.as_mut() {
            writer.note_skipped();
        }
    }
}
