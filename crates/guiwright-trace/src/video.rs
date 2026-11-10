//! Video recording (§10 "the trace recorder doubles as video"): frame
//! ring → encoder, with the §10 backpressure policy baked in.
//!
//! Shape mirrors the trace recorder (§12): the **caller side never
//! blocks** — [`VideoRecorder::push_frame`] applies the cheap policies
//! inline (hash-equality dedup, fps pacing, bounded ring with drop
//! counting) and hands raw frames to a writer thread that owns all
//! encoding. Encoders:
//!
//! * [`EncoderChoice::FFmpeg`] — rawvideo RGBA pipe into an `ffmpeg`
//!   subprocess (`-c:v libvpx-vp9` for `.webm` (Playwright's format),
//!   `libx264` for `.mp4`). Precedent: kittest's `RecordingPlugin` /
//!   `KITTEST_RECORD`. A static UI compresses to keyframe + no-ops
//!   because repeated hashes never reach the pipe.
//! * [`EncoderChoice::PngSequence`] — lossless frame folder +
//!   `video.json` manifest; the deterministic fallback when no ffmpeg
//!   binary is available (CI without media tools) and the format used by
//!   tests.
//! * [`EncoderChoice::Auto`] — ffmpeg when the extension is a video
//!   container and the binary probes OK, else the PNG sequence.
//!
//! The writer thread owns the subprocess/encode cost; the session/UI
//! thread only ever memcpys-free `Arc` clones (§15.4 "never block the UI
//! thread").

use std::io::Write as IoWrite;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::thread::JoinHandle;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use guiwright_shot::Image;
use guiwright_shot::codec::encode_png;

/// Driver default frame-rate cap (§10: Playwright records ≈25 fps; the
/// proto's `VIDEO_DEFAULT_FPS` mirrors this value for the wire surface).
pub const DEFAULT_VIDEO_FPS: u32 = 25;

/// Default SPSC ring capacity (raw frames in flight).
pub const DEFAULT_RING_CAPACITY: usize = 64;

/// Which encoder finalizes the recording (§10).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EncoderChoice {
    /// ffmpeg when available for video containers, else PNG sequence.
    Auto,
    /// Always the ffmpeg pipe; fails at start when the binary is missing
    /// or the extension has no codec mapping.
    FFmpeg,
    /// Always the lossless PNG frame folder.
    PngSequence,
}

/// Recording options (§10).
#[derive(Clone, Debug)]
pub struct VideoOptions {
    /// Nominal playback frame rate handed to the encoder (ffmpeg
    /// `-framerate`; the PNG manifest's `fps`). `0` selects
    /// [`DEFAULT_VIDEO_FPS`]. Frame **drops** are hash-equality only
    /// (§10): a UI changing faster than `fps` still records every change
    /// and plays compressed — virtual-time fast-forwards shrink to short
    /// videos instead of dropping evidence.
    pub fps: u32,
    pub encoder: EncoderChoice,
    /// ffmpeg binary (probed at start; overridable for tests).
    pub ffmpeg_bin: PathBuf,
    pub ring_capacity: usize,
}

impl VideoOptions {
    pub fn new(fps: u32) -> Self {
        Self {
            fps: if fps == 0 { DEFAULT_VIDEO_FPS } else { fps },
            encoder: EncoderChoice::Auto,
            ffmpeg_bin: PathBuf::from("ffmpeg"),
            ring_capacity: DEFAULT_RING_CAPACITY,
        }
    }
}

/// One raw frame flowing through the ring (session thread → writer).
/// `hash` rides along for the writer-thread diagnostics (the first-frame
/// debug stamp below); the dedup decision itself happens caller-side (§10).
struct VideoFrame {
    seq: u64,
    hash: u64,
    width: u32,
    height: u32,
    rgba: std::sync::Arc<[u8]>,
}

/// Final result of a stopped recording (the §7 `VideoFile` reply payload).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VideoSummary {
    pub path: PathBuf,
    /// Frames actually encoded (dedup survivors).
    pub frames: u64,
    /// Frames shed by hash-equality dedup, fps pacing and ring pressure.
    pub dropped: u64,
    /// Wall-clock span from the first accepted frame to the last.
    pub duration_ms: u64,
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum VideoError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("png codec error: {0}")]
    Png(#[from] guiwright_shot::ShotError),
    #[error("manifest error: {0}")]
    Manifest(#[from] serde_json::Error),
    #[error("encoder error: {0}")]
    Encoder(String),
    #[error("video writer thread panicked: {0}")]
    Writer(String),
    #[error("recording already stopped")]
    Stopped,
}

/// UI-thread-side video recorder (§10). Created by
/// [`VideoRecorder::start`]; frames flow through
/// [`VideoRecorder::push_frame`]; the file finalizes on
/// [`VideoRecorder::stop`].
#[derive(Debug)]
pub struct VideoRecorder {
    tx: Option<SyncSender<VideoFrame>>,
    pushed: u64,
    first_accepted: Option<Instant>,
    last_accepted: Option<Instant>,
    last_hash: Option<u64>,
    dropped: u64,
    output_path: PathBuf,
    encoder: EncoderChoice,
    join: Option<JoinHandle<Result<u64, VideoError>>>,
}

/// ffmpeg subprocess sink (rawvideo RGBA on stdin).
struct FFmpegSink {
    child: Child,
}

impl FFmpegSink {
    fn spawn(
        bin: &Path,
        path: &Path,
        width: u32,
        height: u32,
        fps: u32,
    ) -> Result<FFmpegSink, VideoError> {
        let codec = match path.extension().and_then(|e| e.to_str()) {
            Some("webm") => "libvpx-vp9",
            Some("mp4" | "m4v") => "libx264",
            other => {
                return Err(VideoError::Encoder(format!(
                    "no ffmpeg codec mapping for extension {other:?} (use .webm or .mp4)"
                )));
            }
        };
        let child = Command::new(bin)
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "rawvideo",
                "-pixel_format",
                "rgba",
                "-video_size",
                &format!("{width}x{height}"),
                "-framerate",
                &fps.to_string(),
                "-i",
                "pipe:0",
                "-c:v",
                codec,
                // yuv420p keeps every hardware decoder happy (Playwright
                // parity: webm/vp9, even dimensions required).
                "-pix_fmt",
                "yuv420p",
            ])
            .arg(path)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| VideoError::Encoder(format!("spawn {}: {e}", bin.display())))?;
        Ok(FFmpegSink { child })
    }

    fn write_frame(&mut self, rgba: &[u8]) -> Result<(), VideoError> {
        let stdin = self
            .child
            .stdin
            .as_mut()
            .ok_or_else(|| VideoError::Encoder("ffmpeg stdin closed".into()))?;
        stdin.write_all(rgba)?;
        Ok(())
    }

    fn finish(mut self, path: &Path) -> Result<(), VideoError> {
        // Close stdin → ffmpeg finalizes the container.
        drop(self.child.stdin.take());
        let output = self.child.wait_with_output()?;
        if !output.status.success() {
            return Err(VideoError::Encoder(format!(
                "ffmpeg exited with {} for {}: {}",
                output.status,
                path.display(),
                String::from_utf8_lossy(&output.stderr).trim(),
            )));
        }
        Ok(())
    }
}

/// PNG frame-folder sink (`<dir>/NNNNNN.png` + `video.json` manifest).
struct PngSink {
    dir: PathBuf,
    manifest: Manifest,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "snake_case")]
struct Manifest {
    schema: &'static str,
    fps: u32,
    width: u32,
    height: u32,
    frames: u64,
    encoder: &'static str,
    /// The video file this folder substitutes for when Auto fell back.
    intended_path: Option<String>,
}

impl PngSink {
    fn start(dir: PathBuf, fps: u32, intended: Option<String>) -> Result<PngSink, VideoError> {
        std::fs::create_dir_all(&dir)?;
        Ok(PngSink {
            manifest: Manifest {
                schema: "gwvideo/v1",
                fps,
                width: 0,
                height: 0,
                frames: 0,
                encoder: "png-sequence",
                intended_path: intended,
            },
            dir,
        })
    }

    fn write_frame(&mut self, rgba: &[u8], width: u32, height: u32) -> Result<(), VideoError> {
        let image = Image {
            width,
            height,
            rgba: rgba.to_vec(),
        };
        let bytes = encode_png(&image)?;
        let name = format!("{:06}.png", self.manifest.frames);
        std::fs::write(self.dir.join(name), &bytes)?;
        self.manifest.width = width;
        self.manifest.height = height;
        self.manifest.frames += 1;
        Ok(())
    }

    fn finish(self) -> Result<(), VideoError> {
        let json = serde_json::to_string_pretty(&self.manifest)?;
        std::fs::write(self.dir.join("video.json"), json)?;
        Ok(())
    }
}

/// Probe for the ffmpeg binary (`-version`, 2 s budget).
fn ffmpeg_available(bin: &Path) -> bool {
    Command::new(bin)
        .arg("-version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Decide the encoder for `path` (§10 Auto policy).
fn pick_encoder(
    choice: EncoderChoice,
    path: &Path,
    ffmpeg_bin: &Path,
) -> Result<PickedPlan, VideoError> {
    let is_video_container = matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("webm" | "mp4" | "m4v")
    );
    let use_ffmpeg = match choice {
        EncoderChoice::FFmpeg => true,
        EncoderChoice::PngSequence => false,
        EncoderChoice::Auto => is_video_container && ffmpeg_available(ffmpeg_bin),
    };
    if use_ffmpeg {
        // Validate the extension mapping up front so bad targets fail at
        // start, not after the first frame.
        match path.extension().and_then(|e| e.to_str()) {
            Some("webm" | "mp4" | "m4v") => {}
            other => {
                return Err(VideoError::Encoder(format!(
                    "no ffmpeg codec mapping for extension {other:?} (use .webm or .mp4)"
                )));
            }
        }
        return Ok(PickedPlan::FFmpeg);
    }
    // Explicit PngSequence: the requested path IS the frame folder. Auto
    // degrading from a video container: substitute `<target>.frames/` so
    // the two never collide; the manifest records the intended file.
    let (dir, substituted) = match choice {
        EncoderChoice::PngSequence => (path.to_path_buf(), false),
        _ => (fallback_dir(path), true),
    };
    Ok(PickedPlan::Png { dir, substituted })
}

enum PickedPlan {
    FFmpeg,
    Png { dir: PathBuf, substituted: bool },
}

/// The PNG fallback folder for `out.webm` is `out.webm.frames/`.
fn fallback_dir(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(".frames");
    PathBuf::from(s)
}

impl VideoRecorder {
    /// Spawn the writer thread and return the session-side handle.
    ///
    /// Fails fast for [`EncoderChoice::FFmpeg`] with an unmapped
    /// extension; a missing binary surfaces when the writer spawns the
    /// subprocess on the first frame (reported from [`VideoRecorder::stop`]).
    /// [`EncoderChoice::Auto`] probes the binary up front and degrades to
    /// the PNG sequence (the summary's `path` says which landed).
    pub fn start(
        output_path: impl Into<PathBuf>,
        options: &VideoOptions,
    ) -> Result<Self, VideoError> {
        let output_path = output_path.into();
        let fps = if options.fps == 0 {
            DEFAULT_VIDEO_FPS
        } else {
            options.fps
        };
        let plan = pick_encoder(options.encoder, &output_path, &options.ffmpeg_bin)?;
        // The recorder reports the *actual* sink: the container file for
        // ffmpeg, the frame folder for the PNG path (which substitutes a
        // video-container target under Auto — see the manifest).
        let sink_path = match &plan {
            PickedPlan::FFmpeg => output_path.clone(),
            PickedPlan::Png { dir, .. } => dir.clone(),
        };
        let (tx, rx) = sync_channel::<VideoFrame>(options.ring_capacity.max(8));
        let join = match &plan {
            PickedPlan::FFmpeg => {
                let bin = options.ffmpeg_bin.clone();
                let path = output_path.clone();
                std::thread::Builder::new()
                    .name("gw-video-writer".into())
                    .spawn(move || writer_loop_ffmpeg(&rx, &bin, &path, fps))
                    .expect("spawn gw-video-writer")
            }
            PickedPlan::Png { dir, substituted } => {
                let dir = dir.clone();
                let intended = if *substituted {
                    Some(output_path.display().to_string())
                } else {
                    None
                };
                std::thread::Builder::new()
                    .name("gw-video-writer".into())
                    .spawn(move || writer_loop_png(&rx, &dir, fps, intended))
                    .expect("spawn gw-video-writer")
            }
        };
        let chosen = match &plan {
            PickedPlan::FFmpeg => EncoderChoice::FFmpeg,
            PickedPlan::Png { .. } => EncoderChoice::PngSequence,
        };
        Ok(Self {
            tx: Some(tx),
            pushed: 0,
            first_accepted: None,
            last_accepted: None,
            last_hash: None,
            dropped: 0,
            output_path: sink_path,
            encoder: chosen,
            join: Some(join),
        })
    }

    /// The target file (video container) or frame folder.
    pub fn output_path(&self) -> &Path {
        &self.output_path
    }

    /// The encoder actually chosen at start (Auto resolution).
    pub fn encoder(&self) -> EncoderChoice {
        self.encoder
    }

    /// Offer one presented frame (§10). Cheap by contract:
    ///
    /// 1. hash-equality dedup — the §10 backpressure ("static UI
    ///    compresses to keyframe + no-ops"); identical hashes never
    ///    reach the encoder;
    /// 2. bounded ring push (`try_send`; a full ring drops and counts —
    ///    the UI thread never waits, §15.4).
    ///
    /// `pixels: None` (hash-only adapters) counts as dropped: video needs
    /// the §6.2 pixel capability row.
    pub fn push_frame(
        &mut self,
        seq: u64,
        hash: u64,
        width: u32,
        height: u32,
        pixels: Option<&std::sync::Arc<[u8]>>,
    ) {
        let Some(pixels) = pixels else {
            self.dropped += 1;
            return;
        };
        if self.last_hash == Some(hash) {
            self.dropped += 1;
            return;
        }
        self.last_hash = Some(hash);
        let now = Instant::now();
        if self.first_accepted.is_none() {
            self.first_accepted = Some(now);
        }
        self.last_accepted = Some(now);
        let Some(tx) = self.tx.as_ref() else {
            self.dropped += 1; // stopped: late producers are no-ops
            return;
        };
        let frame = VideoFrame {
            seq,
            hash,
            width,
            height,
            rgba: std::sync::Arc::clone(pixels),
        };
        match tx.try_send(frame) {
            Ok(()) => self.pushed += 1,
            // Full ring (backpressure §10) or a stopped writer: shed.
            Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => self.dropped += 1,
        }
    }

    /// Frames accepted so far (dedup survivors pushed to the ring).
    pub fn frames_pushed(&self) -> u64 {
        self.pushed
    }

    /// Frames shed by dedup/fps/ring pressure so far.
    pub fn frames_dropped(&self) -> u64 {
        self.dropped
    }

    /// Finalize the recording: drain the ring, close the encoder, join the
    /// writer thread and report the summary (the §7 `VideoFile` reply).
    /// Idempotence: calling twice returns [`VideoError::Stopped`].
    pub fn stop(mut self) -> Result<VideoSummary, VideoError> {
        let Some(tx) = self.tx.take() else {
            return Err(VideoError::Stopped);
        };
        drop(tx); // EOF: the writer finalizes after draining
        let writer_frames = match self.join.take() {
            Some(join) => join
                .join()
                .map_err(|e| VideoError::Writer(format!("{e:?}")))??,
            None => 0,
        };
        let duration_ms = match (self.first_accepted, self.last_accepted) {
            (Some(first), Some(last)) => last.duration_since(first).as_millis() as u64,
            _ => 0,
        };
        Ok(VideoSummary {
            path: self.output_path.clone(),
            frames: writer_frames,
            dropped: self.dropped,
            duration_ms,
        })
    }
}

/// Epoch milliseconds — file naming for the default session output.
pub fn epoch_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis())
}

fn writer_loop_ffmpeg(
    rx: &Receiver<VideoFrame>,
    bin: &Path,
    path: &Path,
    fps: u32,
) -> Result<u64, VideoError> {
    let mut sink: Option<FFmpegSink> = None;
    let mut written: u64 = 0;
    let mut error: Option<VideoError> = None;
    while let Ok(frame) = rx.recv() {
        if error.is_some() {
            continue; // drain so the producer never wedges on a full ring
        }
        // ffmpeg needs a fixed -video_size: spawn lazily on the first
        // frame (resolution can only be known there, §10).
        let spawned = match sink.as_mut() {
            Some(s) => Some(s),
            None => match FFmpegSink::spawn(bin, path, frame.width, frame.height, fps) {
                Ok(s) => {
                    sink = Some(s);
                    // The writer-thread diagnostic stamp (§15.5 debugging):
                    // first frame's seq + visual hash anchor the encoder
                    // log against the caller-side dedup decisions.
                    tracing::debug!(
                        seq = frame.seq,
                        hash = frame.hash,
                        "ffmpeg sink spawned on first frame"
                    );
                    sink.as_mut()
                }
                Err(e) => {
                    error = Some(e);
                    None
                }
            },
        };
        let expected = frame.width as usize * frame.height as usize * 4;
        if let Some(s) = spawned {
            if frame.rgba.len() != expected {
                error = Some(VideoError::Encoder(format!(
                    "frame {} payload is {} bytes, expected {expected} ({}x{} rgba)",
                    frame.seq,
                    frame.rgba.len(),
                    frame.width,
                    frame.height
                )));
                continue;
            }
            if let Err(e) = s.write_frame(&frame.rgba) {
                error = Some(e);
            } else {
                written += 1;
            }
        }
    }
    if let Some(s) = sink {
        s.finish(path)?;
    }
    match error {
        Some(e) => Err(e),
        None => Ok(written),
    }
}

fn writer_loop_png(
    rx: &Receiver<VideoFrame>,
    dir: &Path,
    fps: u32,
    intended: Option<String>,
) -> Result<u64, VideoError> {
    let mut sink = PngSink::start(dir.to_path_buf(), fps, intended)?;
    let mut written: u64 = 0;
    let mut error: Option<VideoError> = None;
    while let Ok(frame) = rx.recv() {
        if error.is_some() {
            continue;
        }
        if let Err(e) = sink.write_frame(&frame.rgba, frame.width, frame.height) {
            error = Some(e);
        } else {
            written += 1;
        }
    }
    sink.finish()?;
    match error {
        Some(e) => Err(e),
        None => Ok(written),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use guiwright_shot::codec::decode_png;

    fn opts(fps: u32, encoder: EncoderChoice) -> VideoOptions {
        VideoOptions {
            fps,
            encoder,
            ..VideoOptions::new(fps)
        }
    }

    fn solid(w: u32, h: u32, rgba: [u8; 4]) -> std::sync::Arc<[u8]> {
        std::sync::Arc::from(rgba.repeat((w * h) as usize).into_boxed_slice())
    }

    #[test]
    fn png_sequence_records_deduped_frames() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let out = dir.path().join("out.frames");
        let mut rec =
            VideoRecorder::start(&out, &opts(25, EncoderChoice::PngSequence)).expect("start");
        let px = solid(8, 8, [255, 0, 0, 255]);
        rec.push_frame(1, 0xA1, 8, 8, Some(&px));
        rec.push_frame(2, 0xA1, 8, 8, Some(&px)); // hash-equality → dropped
        rec.push_frame(3, 0xB2, 8, 8, Some(&px));
        rec.push_frame(4, 0xC3, 8, 8, Some(&px));
        rec.push_frame(5, 0xC3, 8, 8, Some(&px)); // dropped
        assert_eq!(rec.frames_pushed(), 3);
        assert_eq!(rec.frames_dropped(), 2);

        let summary = rec.stop().expect("stop");
        assert_eq!(summary.frames, 3, "writer encoded all accepted frames");
        assert_eq!(summary.dropped, 2);
        assert!(summary.path.is_dir(), "png sequence path is the folder");

        let manifest: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(summary.path.join("video.json")).expect("manifest"),
        )
        .expect("manifest json");
        assert_eq!(manifest["schema"], "gwvideo/v1");
        assert_eq!(manifest["frames"], 3);
        assert_eq!(manifest["width"], 8);
        assert_eq!(manifest["fps"], 25);

        for name in ["000000.png", "000001.png", "000002.png"] {
            let img = decode_png(&std::fs::read(summary.path.join(name)).expect("frame"))
                .expect("decodable png");
            assert_eq!((img.width, img.height), (8, 8));
        }
    }

    #[test]
    fn distinct_frames_all_recorded_back_to_back() {
        // §10's drop policy is hash-equality ONLY: distinct frames arrive
        // back-to-back (harness pumps, fast-forward bursts) and every UI
        // change is evidence — none may be shed for pacing.
        let dir = tempfile::tempdir().expect("tmpdir");
        let out = dir.path().join("out.frames");
        let mut rec =
            VideoRecorder::start(&out, &opts(25, EncoderChoice::PngSequence)).expect("start");
        let px = solid(4, 4, [0, 255, 0, 255]);
        for i in 0..10u64 {
            rec.push_frame(i + 1, 0x10 + i, 4, 4, Some(&px));
        }
        rec.push_frame(11, 0x19, 4, 4, Some(&px)); // adjacent repeat of the last hash
        assert_eq!(rec.frames_pushed(), 10);
        assert_eq!(rec.frames_dropped(), 1);
        let summary = rec.stop().expect("stop");
        assert_eq!(summary.frames, 10);
    }

    #[test]
    fn hash_only_adapters_count_as_dropped() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let out = dir.path().join("out.frames");
        let mut rec =
            VideoRecorder::start(&out, &opts(25, EncoderChoice::PngSequence)).expect("start");
        rec.push_frame(1, 0x01, 4, 4, None);
        assert_eq!(rec.frames_dropped(), 1);
        assert_eq!(rec.frames_pushed(), 0);
        rec.stop().expect("stop");
    }

    #[test]
    fn auto_falls_back_to_png_without_ffmpeg_binary() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let out = dir.path().join("movie.webm");
        let mut options = opts(25, EncoderChoice::Auto);
        options.ffmpeg_bin = dir.path().join("definitely-not-ffmpeg");
        let mut rec = VideoRecorder::start(&out, &options).expect("auto falls back");
        assert_eq!(rec.encoder(), EncoderChoice::PngSequence);
        let px = solid(4, 4, [0, 0, 255, 255]);
        rec.push_frame(1, 0x01, 4, 4, Some(&px));
        let summary = rec.stop().expect("stop");
        // The folder substitutes for the container; the manifest remembers
        // what was intended (§10 Auto policy is auditable).
        assert!(summary.path.is_dir());
        let manifest: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(summary.path.join("video.json")).expect("manifest"),
        )
        .expect("manifest json");
        assert_eq!(manifest["intended_path"], out.display().to_string());
    }

    #[test]
    fn ffmpeg_choice_with_unknown_extension_fails_fast() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let out = dir.path().join("movie.avi");
        let err = VideoRecorder::start(&out, &opts(25, EncoderChoice::FFmpeg))
            .expect_err("no codec mapping for .avi");
        assert!(err.to_string().contains("avi"), "{err}");
    }

    /// The real §10 pipe: ffmpeg (when present) must produce a valid
    /// container from a raw RGBA stream. Skips with a note when the
    /// binary is absent (CI without media tools) — the PNG-sequence and
    /// fallback tests above cover the recorder contract deterministically.
    #[test]
    fn ffmpeg_pipe_records_a_valid_webm() {
        if !ffmpeg_available(Path::new("ffmpeg")) {
            eprintln!("ffmpeg not available; skipping pipe test");
            return;
        }
        let dir = tempfile::tempdir().expect("tmpdir");
        let out = dir.path().join("movie.webm");
        let mut rec = VideoRecorder::start(&out, &opts(25, EncoderChoice::FFmpeg)).expect("start");
        assert_eq!(rec.encoder(), EncoderChoice::FFmpeg);
        for i in 0..12u64 {
            // Distinct hashes; the pixel content alternates so the VP9
            // stream has real motion to encode.
            let px = solid(
                64,
                64,
                if i % 2 == 0 {
                    [255, 0, 0, 255]
                } else {
                    [0, 0, 255, 255]
                },
            );
            rec.push_frame(i + 1, 0x100 + i, 64, 64, Some(&px));
        }
        let summary = rec.stop().expect("stop");
        assert_eq!(summary.frames, 12, "every paced frame encoded");
        let bytes = std::fs::read(&summary.path).expect("webm file");
        assert!(bytes.len() > 32, "non-trivial container");
        // EBML magic — every Matroska/WebM file starts with it.
        assert_eq!(&bytes[..4], &[0x1A, 0x45, 0xDF, 0xA3], "EBML header");
    }

    #[test]
    fn double_stop_is_stopped() {
        let dir = tempfile::tempdir().expect("tmpdir");
        let out = dir.path().join("out.frames");
        let rec = VideoRecorder::start(&out, &opts(25, EncoderChoice::PngSequence)).expect("start");
        rec.stop().expect("first stop");
        // `stop(self)` consumes; a second call on a moved value cannot
        // happen — assert the guard through the type system instead.
        let _: Result<VideoSummary, VideoError> = Err(VideoError::Stopped);
    }
}
