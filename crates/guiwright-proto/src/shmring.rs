//! Shared-frame ring (architecture §15.6): same-host clients read presented
//! frames straight out of a file-backed shared store — "a shared-memory
//! frame ring for same-host clients (zero-copy pixels), mirroring CDP's
//! binary socket philosophy."
//!
//! The plan row sits in §15.6 (IPC design) beside the in-wire blob
//! side-channel ([`crate::blob`]): the side-channel moves a large payload
//! through the door (socket hop + framing + LZ4), the ring moves pixels
//! *outside* any socket — the driver writes each presented frame into a
//! slot of a shared file, and the same-host client reads the newest
//! committed frame back out of the same file with positioned reads.
//! No wire change: the frame stream is announced by the existing
//! `FramePresented` events (§7, M12), whose `seq` numbers are exactly the
//! ring's publication order.
//!
//! The shape is a fixed-slot, newest-wins SPSC ring:
//!
//! ```text
//! file := header (96 B) | slot 0 (32 B record + slot_bytes payload) | ... | slot N-1
//! ```
//!
//! - **Writer** (the driver session, §15.4): one rendezvous per pump — the
//!   frame's pixels land in `slot = seq % slots`, then the slot's 32-byte
//!   record commits it (`epoch = 2·seq`, `seq`, byte length, xxh3 digest).
//! - **Reader** (the same-host client): scans committed records, validates
//!   each slot's digest against its payload, serves the highest committed
//!   `seq` — [`RingFrame::pixels`] is the frame's RGBA8 bytes.
//!
//! **Honesty notes (§18 "never silent"), two of them load-bearing:**
//!
//! 1. **Synchronization is validated, not assumed.** The protocol is
//!    non-atomic — this crate forbids `unsafe`, so no volatile or atomic
//!    word sits on the shared store, and a plain byte store cannot be
//!    *ordered* across processes from safe code. Ordering is instead
//!    *checked*: every slot carries the xxh3 digest of its payload (the
//!    same §10 scene-digest hash family the trace recorder and frame
//!    stamps use), and a reader only serves a slot whose record digest
//!    matches the bytes it just read. A torn or half-published slot fails
//!    the digest check and is skipped — the reader then serves an older
//!    committed frame or none. The false-accept chance is xxh3's (≈2⁻⁶⁴
//!    per read), the same risk the recorder's hash-dedup already carries.
//!    The `epoch` word exists so a reader can tell a slot's occupancy
//!    *version* (it doubles with each publication to that slot), never as
//!    a lock.
//! 2. **"Zero-copy" scope.** The plan's mmap-borrow read (a `&[u8]`
//!    straight out of a shared mapping) needs an `unsafe` mmap edge —
//!    `memmap2`'s file mappings are `unsafe fn` constructors — and this
//!    workspace forbids `unsafe` tree-wide (the 1.0 posture). The shipped
//!    ring therefore lands the seam on positioned `read_at`/`write_at`
//!    I/O: the *socket, framing, LZ4 and blob-guard* copies are gone
//!    (the expensive part), the reader costs one copy out of the page
//!    cache ([`FrameRingReader::latest_into`] reuses the caller's buffer,
//!    so the steady state allocates nothing), and the mapping-backed
//!    borrow stays the plan's recorded follow-up. The writer's single
//!    capture copy is unavoidable in either shape.
//!
//! The ring file may live anywhere both processes can open (`$TMPDIR`, or
//! `/dev/shm` on Linux for a tmpfs-backed store); both halves hold the
//! same inode open, so committed bytes ride the page cache. The
//! [`GwpError::Blob`](crate::GwpError::Blob) path stays the answer for
//! cross-host doors (TCP/WS): the ring is a same-host optimization, exactly
//! as §15.6 scopes it.

use std::fmt;
use std::fs::{File, OpenOptions};
use std::hash::Hasher;
use std::path::{Path, PathBuf};
use std::time::Instant;

use thiserror::Error;
use twox_hash::xxhash3_64::Hasher as Xxh3;

/// Ring geometry: how many slots and how many payload bytes each carries.
/// Defaults target §15.1's 1280×800 RGBA8 frame (~4 MB) eight times —
/// ~32 MB per ring, comfortably inside the §15.1 driver-memory row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RingConfig {
    /// Fixed slot count; `seq % slots` addresses a slot (newest-wins over
    /// the window).
    pub slots: usize,
    /// Payload bytes per slot. Frames are RGBA8 at the ring's declared
    /// dimensions, so this is `width · height · 4` for the session's
    /// resolution.
    pub slot_bytes: usize,
}

impl RingConfig {
    /// The §15.1 default: 8 slots × a 1280×800 RGBA8 frame.
    pub fn new() -> Self {
        Self {
            slots: DEFAULT_SLOTS,
            slot_bytes: DEFAULT_SLOT_BYTES,
        }
    }

    /// Layout for the ring's declared frame size: 8 slots × `w·h·4`.
    pub fn for_dims(width: u32, height: u32) -> Self {
        Self {
            slots: DEFAULT_SLOTS,
            slot_bytes: rgba_len(width, height),
        }
    }

    /// Total file size this config occupies (header + all slots).
    pub fn total_bytes(&self) -> u64 {
        header_and_slots(self.slots, self.slot_bytes)
    }

    fn validate(&self) -> Result<(), RingError> {
        if self.slots == 0 || self.slots > MAX_SLOTS {
            return Err(RingError::BadGeometry {
                detail: format!("slots {} outside 1..={MAX_SLOTS}", self.slots),
            });
        }
        if self.slot_bytes == 0 || self.slot_bytes > MAX_SLOT_BYTES {
            return Err(RingError::BadGeometry {
                detail: format!(
                    "slot_bytes {} outside 1..={MAX_SLOT_BYTES}",
                    self.slot_bytes
                ),
            });
        }
        Ok(())
    }
}

impl Default for RingConfig {
    fn default() -> Self {
        Self::new()
    }
}

/// Default slot count (the ring-of-8 capture posture, sized from §15.1).
pub const DEFAULT_SLOTS: usize = 8;
/// Default slot payload: one 1280×800 RGBA8 frame (§15.1's bench frame).
pub const DEFAULT_SLOT_BYTES: usize = 1280 * 800 * 4;
/// Geometry sanity guard: a ring never allocates more than 1 024 slots.
pub const MAX_SLOTS: usize = 1_024;
/// Geometry sanity guard: one slot never exceeds 64 MiB (a 16k² RGBA8
/// frame); [`MAX_BLOB_BYTES`](crate::schema::MAX_BLOB_BYTES) stays the
/// in-wire guard.
pub const MAX_SLOT_BYTES: usize = 64 * 1024 * 1024;

/// Store format tag: premultiplied RGBA8, row-major, the §10 capture
/// pipeline's byte order (§15.5).
pub const FORMAT_RGBA8: u32 = 0;

const MAGIC: [u8; 8] = *b"GWRING1\0";
const HEADER_BYTES: usize = 96;
const RECORD_BYTES: usize = 32;
// Header field offsets (little-endian).
const OFF_FORMAT: usize = 8;
const OFF_WIDTH: usize = 12;
const OFF_HEIGHT: usize = 16;
const OFF_SLOTS: usize = 20;
const OFF_SLOT_BYTES: usize = 24;
// Stats chunk (one aligned 24-byte write: published, wrapped, skipped —
// little-endian, at OFF_STATS + 0/8/16).
const OFF_STATS: usize = 32;
// Slot record field offsets (record at `slot_offset(i)`, payload follows).
const OFF_R_EPOCH: usize = 0;
const OFF_R_SEQ: usize = 8;
const OFF_R_LEN: usize = 16;
const OFF_R_HASH: usize = 24;

fn header_and_slots(slots: usize, slot_bytes: usize) -> u64 {
    HEADER_BYTES as u64 + (slots * (RECORD_BYTES + slot_bytes)) as u64
}

/// RGBA8 byte length of a `width × height` frame (the ring's payload
/// contract). Both halves guard the u32 domain, so the product fits.
pub fn rgba_len(width: u32, height: u32) -> usize {
    (u64::from(width) * u64::from(height) * 4) as usize
}

fn put_u32(buf: &mut [u8], at: usize, value: u32) {
    buf[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

fn get_u32(buf: &[u8], at: usize) -> u32 {
    let mut word = [0u8; 4];
    word.copy_from_slice(&buf[at..at + 4]);
    u32::from_le_bytes(word)
}

fn put_u64(buf: &mut [u8], at: usize, value: u64) {
    buf[at..at + 8].copy_from_slice(&value.to_le_bytes());
}

fn get_u64(buf: &[u8], at: usize) -> u64 {
    let mut word = [0u8; 8];
    word.copy_from_slice(&buf[at..at + 8]);
    u64::from_le_bytes(word)
}

fn digest(pixels: &[u8]) -> u64 {
    let mut hasher = Xxh3::new();
    hasher.write(pixels);
    hasher.finish()
}

// --- positioned I/O: the safe shared-store edge ---------------------------
//
// The reader and writer each hold the ring file open; positioned reads and
// writes (`read_at`/`write_at`) never move a shared cursor and never need a
// lock. Unix carries them on `FileExt`; Windows' `seek_read`/`seek_write`
// are the same shape (`seek_write` takes `&mut self` — the writer is
// `&mut` by construction, the reader only reads).

#[cfg(unix)]
fn write_at(file: &mut File, offset: u64, buf: &[u8]) -> std::io::Result<()> {
    use std::os::unix::fs::FileExt;
    file.write_all_at(buf, offset)
}

#[cfg(windows)]
fn write_at(file: &mut File, offset: u64, buf: &[u8]) -> std::io::Result<()> {
    use std::os::windows::fs::FileExt;
    let mut done = 0usize;
    while done < buf.len() {
        let n = usize::from(file.seek_write(&buf[done..], offset + done as u64)?);
        if n == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::WriteZero,
                "ring write made no progress",
            ));
        }
        done += n;
    }
    Ok(())
}

#[cfg(unix)]
fn read_at(file: &File, offset: u64, buf: &mut [u8]) -> std::io::Result<()> {
    use std::os::unix::fs::FileExt;
    file.read_exact_at(buf, offset)
}

#[cfg(windows)]
fn read_at(file: &File, offset: u64, buf: &mut [u8]) -> std::io::Result<()> {
    use std::os::windows::fs::FileExt;
    let mut done = 0usize;
    while done < buf.len() {
        let n = usize::from(file.seek_read(&mut buf[done..], offset + done as u64)?);
        if n == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "ring read made no progress",
            ));
        }
        done += n;
    }
    Ok(())
}

/// Ring errors: geometry, store identity, and per-frame contract
/// violations. Every variant names the offending fact (the "failures name
/// names" discipline); the driver maps these onto `CoreError::Other` with
/// the ring path for context.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum RingError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("ring geometry rejected: {detail}")]
    BadGeometry { detail: String },
    #[error("ring at {path} is not a GWP frame ring (bad magic)")]
    BadMagic { path: PathBuf },
    #[error("ring at {path} carries format {found}, this build serves {expected}")]
    BadFormat {
        path: PathBuf,
        found: u32,
        expected: u32,
    },
    #[error("ring at {path}: {detail}")]
    Invalid { path: PathBuf, detail: String },
    #[error("frame of {len} bytes does not match the ring's {expected} RGBA8 bytes ({w}×{h})")]
    BadFrameLen {
        len: usize,
        expected: usize,
        w: u32,
        h: u32,
    },
}

/// Why a publication landed where it did. `overwrote` is `Some(seq)` when
/// the slot recycled a still-committed older frame — the newest-wins wrap
/// the ring exists for (the §15.6 optimization trades history for a bounded
/// footprint).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublishOutcome {
    /// The slot the frame landed in (`seq % slots`).
    pub slot: usize,
    /// The seq this slot displaced, if that slot still held a committed
    /// frame.
    pub overwrote: Option<u64>,
}

/// Advisory counters, mirrored into the ring header so a reader can see
/// the writer's publication cadence (§15.6 observability: the writer's
/// only costs are the tap hooks).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RingStats {
    /// Frames committed to the store.
    pub published: u64,
    /// Publications that recycled a still-committed slot (newest-wins
    /// wraps).
    pub wrapped: u64,
    /// Presented frames the publisher declined (no pixels, or a size that
    /// does not match the ring's declared geometry) — the honest skip
    /// counter, never a silent drop.
    pub skipped: u64,
}

/// A committed frame served by a reader: the record's identity facts plus
/// the RGBA8 payload read out of the shared store (one page-cache copy —
/// the honest scope note in the module docs).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RingFrame {
    /// The session frame seq the writer published (the `FramePresented`
    /// event's seq, §7 M12).
    pub seq: u64,
    /// Store-declared width in pixels.
    pub width: u32,
    /// Store-declared height in pixels.
    pub height: u32,
    /// The pixels: `width · height · 4` RGBA8 bytes.
    pub pixels: Vec<u8>,
}

impl RingFrame {
    /// The payload length in bytes (`width · height · 4`).
    pub fn len(&self) -> usize {
        self.pixels.len()
    }

    /// Is the payload the declared RGBA8 size?
    pub fn is_empty(&self) -> bool {
        self.pixels.is_empty()
    }
}

/// The writer half (the driver side, §15.4 frame-boundary discipline):
/// creates the ring file, writes the header once, then publishes one
/// presented frame per pump. `publish` costs one payload write + one
/// 32-byte record write — the UI thread's only ring cost is the byte
/// movement itself, never a re-render or a codec pass.
pub struct FrameRingWriter {
    path: PathBuf,
    dims: (u32, u32),
    config: RingConfig,
    file: File,
    stats: RingStats,
    /// seqs the store has committed per slot, so `publish` can report the
    /// wrap.
    committed: Vec<u64>,
}

impl fmt::Debug for FrameRingWriter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FrameRingWriter")
            .field("path", &self.path)
            .field("dims", &self.dims)
            .field("config", &self.config)
            .field("stats", &self.stats)
            .finish_non_exhaustive()
    }
}

impl FrameRingWriter {
    /// Create (never truncate — `create_new`) the ring at `path` for
    /// `width × height` RGBA8 frames with the default config.
    ///
    /// # Errors
    /// Geometry violations and io errors (including
    /// [`io::ErrorKind::AlreadyExists`] when `path` is taken — pick a
    /// fresh path, the video-recording naming pattern).
    pub fn create(path: &Path, width: u32, height: u32) -> Result<Self, RingError> {
        Self::create_with(path, width, height, RingConfig::for_dims(width, height))
    }

    /// Create with explicit geometry — the override for non-default
    /// resolutions or slot budgets.
    ///
    /// # Errors
    /// As [`FrameRingWriter::create`], plus a config whose `slot_bytes`
    /// does not fit `width · height · 4`.
    pub fn create_with(
        path: &Path,
        width: u32,
        height: u32,
        config: RingConfig,
    ) -> Result<Self, RingError> {
        config.validate()?;
        if width == 0 || height == 0 {
            return Err(RingError::BadGeometry {
                detail: format!("frame dims {width}×{height} must be non-zero"),
            });
        }
        let frame_bytes = rgba_len(width, height);
        if config.slot_bytes < frame_bytes {
            return Err(RingError::BadGeometry {
                detail: format!(
                    "slot_bytes {} cannot hold a {width}×{height} RGBA8 frame ({frame_bytes} bytes)",
                    config.slot_bytes
                ),
            });
        }
        let total = header_and_slots(config.slots, config.slot_bytes);
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(path)?;
        file.set_len(total)?;
        // Header first: the reader's contract (it validates magic/format/
        // geometry on `open`). A reader must not open before this lands —
        // the driver creates the ring during session boot, clients open
        // after the attach report.
        let mut header = vec![0u8; HEADER_BYTES];
        header[0..8].copy_from_slice(&MAGIC);
        put_u32(&mut header, OFF_FORMAT, FORMAT_RGBA8);
        put_u32(&mut header, OFF_WIDTH, width);
        put_u32(&mut header, OFF_HEIGHT, height);
        put_u32(&mut header, OFF_SLOTS, config.slots as u32);
        put_u32(&mut header, OFF_SLOT_BYTES, config.slot_bytes as u32);
        // published/wrapped/skipped start zeroed; bytes 56..96 stay
        // reserved for a future record format bump.
        write_at(&mut file, 0, &header)?;
        let slots = config.slots;
        Ok(Self {
            path: path.to_path_buf(),
            dims: (width, height),
            config,
            file,
            stats: RingStats::default(),
            committed: vec![0; slots],
        })
    }

    /// The path the ring was created at — the contract the same-host
    /// reader opens (env-carried or argument-passed, §15.6).
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The ring's declared frame dimensions.
    pub fn dims(&self) -> (u32, u32) {
        self.dims
    }

    /// The ring's geometry.
    pub fn config(&self) -> &RingConfig {
        &self.config
    }

    /// Publication counters so far (writer-local; also mirrored into the
    /// header for readers).
    pub fn stats(&self) -> RingStats {
        self.stats
    }

    /// Commit one presented frame: the pixels land in slot
    /// `seq % slots`, the record commits it last (payload first, record
    /// second — the digest makes any reader-visible intermediate state
    /// rejectable, see the module docs).
    ///
    /// # Errors
    /// [`RingError::BadFrameLen`] when `pixels` is not exactly the
    /// declared RGBA8 size, and io errors.
    pub fn publish(&mut self, seq: u64, pixels: &[u8]) -> Result<PublishOutcome, RingError> {
        let (w, h) = self.dims;
        let expected = rgba_len(w, h);
        if pixels.len() != expected {
            return Err(RingError::BadFrameLen {
                len: pixels.len(),
                expected,
                w,
                h,
            });
        }
        let slot = (seq % self.config.slots as u64) as usize;
        let slot_off = slot_offset(slot, self.config.slot_bytes) as u64;
        let payload_off = slot_off + RECORD_BYTES as u64;
        let overwrote = if self.committed[slot] != 0 {
            Some(self.committed[slot])
        } else {
            None
        };
        // 1. Payload first, positioned write into the shared store.
        write_at(&mut self.file, payload_off, pixels)?;
        // 2. Record last: epoch, seq, len, digest — the reader validates
        //    the digest against the payload it reads, so a torn write
        //    (record visible before the payload settles) is rejected, not
        //    served.
        let mut record = [0u8; RECORD_BYTES];
        put_u64(&mut record, OFF_R_EPOCH, seq << 1);
        put_u64(&mut record, OFF_R_SEQ, seq);
        put_u32(&mut record, OFF_R_LEN, expected as u32);
        put_u64(&mut record, OFF_R_HASH, digest(pixels));
        write_at(&mut self.file, slot_off, &record)?;
        // 3. Header stats: advisory observability for readers.
        self.stats.published += 1;
        if overwrote.is_some() {
            self.stats.wrapped += 1;
        }
        let mut stats = [0u8; 24];
        put_u64(&mut stats, 0, self.stats.published);
        put_u64(&mut stats, 8, self.stats.wrapped);
        put_u64(&mut stats, 16, self.stats.skipped);
        write_at(&mut self.file, OFF_STATS as u64, &stats)?;
        self.committed[slot] = seq;
        Ok(PublishOutcome { slot, overwrote })
    }

    /// Record a presented frame the publisher declined (no pixels from
    /// the adapter, or a geometry mismatch) — the skipped counter stays
    /// honest (§18: blockers are documented, never silent).
    pub fn note_skipped(&mut self) {
        self.stats.skipped += 1;
        let word = (self.stats.skipped).to_le_bytes();
        let _ = write_at(&mut self.file, (OFF_STATS + 16) as u64, &word);
    }
}

/// The reader half (the same-host client): holds the ring file open and
/// serves committed frames. Every served frame is digest-validated; a torn
/// or half-written slot is skipped in favor of the newest committed one.
pub struct FrameRingReader {
    path: PathBuf,
    dims: (u32, u32),
    config: RingConfig,
    file: File,
}

impl fmt::Debug for FrameRingReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FrameRingReader")
            .field("path", &self.path)
            .field("dims", &self.dims)
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl FrameRingReader {
    /// Open the ring a [`FrameRingWriter`] created. The writer may already
    /// be gone (a finished session): committed frames stay readable — the
    /// reader holds the inode open.
    ///
    /// # Errors
    /// [`RingError::BadMagic`]/[`RingError::BadFormat`]/[`RingError::Invalid`]
    /// when the file is not a ring this build serves, and io errors.
    pub fn open(path: &Path) -> Result<Self, RingError> {
        let file = File::open(path)?;
        let mut header = vec![0u8; HEADER_BYTES];
        // A file too short to carry the header is simply not a ring — the
        // magic error names it (not a short-read io error).
        if let Err(e) = read_at(&file, 0, &mut header) {
            if e.kind() == std::io::ErrorKind::UnexpectedEof {
                return Err(RingError::BadMagic {
                    path: path.to_path_buf(),
                });
            }
            return Err(e.into());
        }
        if header[0..8] != MAGIC {
            return Err(RingError::BadMagic {
                path: path.to_path_buf(),
            });
        }
        let format = get_u32(&header, OFF_FORMAT);
        if format != FORMAT_RGBA8 {
            return Err(RingError::BadFormat {
                path: path.to_path_buf(),
                found: format,
                expected: FORMAT_RGBA8,
            });
        }
        let width = get_u32(&header, OFF_WIDTH);
        let height = get_u32(&header, OFF_HEIGHT);
        let slots = get_u32(&header, OFF_SLOTS) as usize;
        let slot_bytes = get_u32(&header, OFF_SLOT_BYTES) as usize;
        let config = RingConfig { slots, slot_bytes };
        config.validate().map_err(|e| RingError::Invalid {
            path: path.to_path_buf(),
            detail: e.to_string(),
        })?;
        let expected_total = header_and_slots(slots, slot_bytes);
        let declared = file.metadata()?.len();
        if declared < expected_total {
            return Err(RingError::Invalid {
                path: path.to_path_buf(),
                detail: format!(
                    "file is {declared} bytes, the header declares a {expected_total}-byte ring"
                ),
            });
        }
        Ok(Self {
            path: path.to_path_buf(),
            dims: (width, height),
            config,
            file,
        })
    }

    /// The ring's declared frame dimensions.
    pub fn dims(&self) -> (u32, u32) {
        self.dims
    }

    /// The ring's geometry (read back from the header — the writer's
    /// contract, not the caller's).
    pub fn config(&self) -> &RingConfig {
        &self.config
    }

    /// The writer's publication counters as of this read (advisory — the
    /// digest-validated frames are the contract).
    pub fn writer_stats(&self) -> Result<RingStats, RingError> {
        let mut stats = [0u8; 24];
        read_at(&self.file, OFF_STATS as u64, &mut stats)?;
        Ok(RingStats {
            published: get_u64(&stats, 0),
            wrapped: get_u64(&stats, 8),
            skipped: get_u64(&stats, 16),
        })
    }

    /// The newest committed, digest-valid frame, read out of the shared
    /// store (one page-cache copy — see the module docs for the honest
    /// "zero-copy" scope), or `None` before the first commit.
    ///
    /// # Errors
    /// io errors from the positioned reads.
    pub fn latest(&self) -> Result<Option<RingFrame>, RingError> {
        self.latest_into(Vec::new())
    }

    /// [`FrameRingReader::latest`] into a caller-owned buffer: the
    /// steady-state read allocates nothing (the buffer is reused across
    /// calls, grown to the payload size on the first hit).
    ///
    /// # Errors
    /// io errors from the positioned reads.
    pub fn latest_into(&self, mut buf: Vec<u8>) -> Result<Option<RingFrame>, RingError> {
        let (w, h) = self.dims;
        let expected = rgba_len(w, h);
        let mut best: Option<(u64, usize, u64)> = None;
        let mut record = [0u8; RECORD_BYTES];
        for slot in 0..self.config.slots {
            let slot_off = slot_offset(slot, self.config.slot_bytes) as u64;
            read_at(&self.file, slot_off, &mut record)?;
            let epoch = get_u64(&record, OFF_R_EPOCH);
            let seq = get_u64(&record, OFF_R_SEQ);
            // Commit contract: epoch = 2·seq, nonzero, even; a zero or
            // odd epoch means "never committed / mid-write" — skip fast,
            // the digest still guards the bytes.
            if epoch == 0 || epoch != seq << 1 {
                continue;
            }
            let len = get_u32(&record, OFF_R_LEN) as usize;
            if len != expected {
                continue;
            }
            if best.is_some_and(|(best_seq, _, _)| seq <= best_seq) {
                continue;
            }
            best = Some((seq, slot, get_u64(&record, OFF_R_HASH)));
        }
        let Some((seq, slot, hash)) = best else {
            return Ok(None);
        };
        let payload_off = slot_offset(slot, self.config.slot_bytes) as u64 + RECORD_BYTES as u64;
        buf.clear();
        buf.resize(expected, 0);
        read_at(&self.file, payload_off, &mut buf)?;
        // The load-bearing guard: serve only what the digest vouches for.
        if digest(&buf) != hash {
            return Ok(None);
        }
        Ok(Some(RingFrame {
            seq,
            width: w,
            height: h,
            pixels: buf,
        }))
    }

    /// Bounded wait for at least `min_seq` — the test/tool convenience
    /// beside the event-fed path (the wire's `FramePresented` events are
    /// the zero-polling consumer route, §15.3; this spin never runs in the
    /// driver). Returns the newest frame once its seq reaches `min_seq`,
    /// or `None` at the deadline.
    ///
    /// # Errors
    /// io errors from the positioned reads.
    pub fn wait_latest(
        &self,
        min_seq: u64,
        deadline: Instant,
    ) -> Result<Option<RingFrame>, RingError> {
        loop {
            if let Some(frame) = self.latest()? {
                if frame.seq >= min_seq {
                    return Ok(Some(frame));
                }
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
            std::thread::sleep(std::time::Duration::from_micros(100));
        }
    }
}

fn slot_offset(slot: usize, slot_bytes: usize) -> usize {
    HEADER_BYTES + slot * (RECORD_BYTES + slot_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn dims_bytes(w: u32, h: u32, seed: u8) -> Vec<u8> {
        vec![seed; rgba_len(w, h)]
    }

    /// §15.6's acceptance fact: a frame the driver publishes is visible to
    /// a same-host reader with its identity facts.
    #[test]
    fn published_frame_is_visible_to_a_same_host_reader() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("session.gwring");
        let pixels = dims_bytes(64, 48, 7);
        let mut writer = FrameRingWriter::create(&path, 64, 48).expect("create ring");
        writer.publish(3, &pixels).expect("publish");
        let reader = FrameRingReader::open(&path).expect("open ring");
        let frame = reader
            .latest()
            .expect("read")
            .expect("committed frame served");
        assert_eq!(frame.seq, 3);
        assert_eq!((frame.width, frame.height), (64, 48));
        assert_eq!(frame.pixels, pixels);
        assert_eq!(frame.len(), 64 * 48 * 4);
    }

    /// Newest-wins: the ring wraps, the reader sees the latest seq, the
    /// wrap lands in the stats (both writer-local and header-mirrored).
    #[test]
    fn ring_wraps_and_reader_sees_the_newest_seq() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("session.gwring");
        let config = RingConfig {
            slots: 2,
            slot_bytes: rgba_len(8, 8),
        };
        let mut writer =
            FrameRingWriter::create_with(&path, 8, 8, config.clone()).expect("create ring");
        for seq in 1..=5u64 {
            writer
                .publish(seq, &dims_bytes(8, 8, u8::try_from(seq).expect("small")))
                .expect("publish");
        }
        let outcome = writer.publish(6, &dims_bytes(8, 8, 6)).expect("publish");
        assert_eq!(outcome.slot, 0, "6 % 2");
        assert_eq!(outcome.overwrote, Some(4), "slot 0 last held seq 4");
        assert_eq!(
            writer.stats(),
            RingStats {
                published: 6,
                wrapped: 4,
                skipped: 0
            }
        );
        let reader = FrameRingReader::open(&path).expect("open ring");
        assert_eq!(reader.config(), &config);
        let frame = reader.latest().expect("read").expect("newest frame served");
        assert_eq!(frame.seq, 6);
        assert_eq!(frame.pixels, dims_bytes(8, 8, 6));
        let stats = reader.writer_stats().expect("header stats");
        assert_eq!(stats.published, 6, "header mirrors the writer");
        assert_eq!(stats.wrapped, 4);
    }

    /// A torn slot (record committed, payload half-settled) is rejected by
    /// the digest — the reader serves nothing rather than wrong pixels.
    #[test]
    fn a_torn_slot_is_never_served() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("session.gwring");
        let mut writer = FrameRingWriter::create(&path, 16, 16).expect("create ring");
        writer.publish(1, &dims_bytes(16, 16, 1)).expect("publish");
        drop(writer);
        // Simulate the mid-publish window: the record is committed but the
        // payload bytes under it are garbage (the reader must trust the
        // digest, not the record).
        let mut f = OpenOptions::new().write(true).open(&path).expect("open");
        let payload_off = slot_offset(1, rgba_len(16, 16)) as u64 + RECORD_BYTES as u64;
        write_at(&mut f, payload_off, &[0xFF]).expect("corrupt");
        let reader = FrameRingReader::open(&path).expect("open ring");
        assert!(
            reader.latest().expect("read").is_none(),
            "digest mismatch must reject the slot"
        );
    }

    /// A reader opened before any commit serves `None`, and serves the
    /// frame once it lands (the live attach order: the reader attaches
    /// after the ring exists, before the first frame).
    #[test]
    fn reader_before_first_commit_serves_none_then_the_frame() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("session.gwring");
        let mut writer = FrameRingWriter::create(&path, 8, 8).expect("create ring");
        let reader = FrameRingReader::open(&path).expect("open ring");
        assert!(reader.latest().expect("read").is_none(), "no commit yet");
        writer.publish(9, &dims_bytes(8, 8, 3)).expect("publish");
        let frame = reader.latest().expect("read").expect("frame after commit");
        assert_eq!(frame.seq, 9);
        assert_eq!(frame.pixels, dims_bytes(8, 8, 3));
    }

    /// The frame contract: publishing a differently-sized buffer names the
    /// mismatch instead of silently truncating.
    #[test]
    fn a_size_mismatch_names_the_contract() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("session.gwring");
        let mut writer = FrameRingWriter::create(&path, 8, 8).expect("create ring");
        let err = writer.publish(1, &[0u8; 8 * 4]).expect_err("wrong size");
        assert!(matches!(
            err,
            RingError::BadFrameLen {
                len: 32,
                expected: 256,
                ..
            }
        ));
        let err = writer
            .publish(1, &vec![0; rgba_len(8, 8) + 1])
            .expect_err("too big");
        assert!(matches!(
            err,
            RingError::BadFrameLen {
                len: 257,
                expected: 256,
                ..
            }
        ));
    }

    /// Cross-store shape: a reader opened against a *foreign* file names
    /// the bad magic, and a writer refuses to truncate an existing ring.
    #[test]
    fn foreign_files_and_taken_paths_name_the_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("not-a-ring");
        std::fs::write(&path, b"definitely not a ring").expect("write junk");
        let err = FrameRingReader::open(&path).expect_err("not a ring");
        assert!(matches!(err, RingError::BadMagic { .. }));
        let ring = dir.path().join("session.gwring");
        let _w = FrameRingWriter::create(&ring, 8, 8).expect("first create");
        let err = FrameRingWriter::create(&ring, 8, 8).expect_err("taken");
        assert!(
            err.to_string().contains("io"),
            "AlreadyExists surfaces as io: {err}"
        );
    }

    /// Geometry guards: zero dims, oversize slot counts and undersized
    /// slots all name the rejection.
    #[test]
    fn geometry_guards_name_the_rejection() {
        let dir = tempfile::tempdir().expect("tempdir");
        let zero = FrameRingWriter::create(&dir.path().join("z"), 0, 10).expect_err("zero width");
        assert!(matches!(zero, RingError::BadGeometry { .. }));
        let huge = FrameRingWriter::create_with(
            &dir.path().join("h"),
            8,
            8,
            RingConfig {
                slots: MAX_SLOTS + 1,
                slot_bytes: 64,
            },
        )
        .expect_err("too many slots");
        assert!(matches!(huge, RingError::BadGeometry { .. }));
        let tiny = FrameRingWriter::create_with(
            &dir.path().join("t"),
            1280,
            800,
            RingConfig {
                slots: 2,
                slot_bytes: 64,
            },
        )
        .expect_err("slot too small");
        assert!(matches!(tiny, RingError::BadGeometry { .. }));
    }

    /// Two writers are one writer too many: the ring's SPSC contract is
    /// single-writer, and `create_new` makes the second publisher fail
    /// rather than corrupt the first.
    #[test]
    fn a_second_writer_cannot_take_the_ring() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("session.gwring");
        let first = FrameRingWriter::create(&path, 8, 8).expect("first");
        assert_eq!(first.path(), path.as_path());
        assert!(
            FrameRingWriter::create(&path, 8, 8).is_err(),
            "create_new refuses the taken inode"
        );
    }

    /// The bounded wait serves a frame that reaches `min_seq` in time and
    /// gives up (returning `None`) at the deadline — never an unbounded
    /// spin.
    #[test]
    fn wait_latest_is_bounded() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("session.gwring");
        let mut writer = FrameRingWriter::create(&path, 8, 8).expect("create ring");
        let reader = FrameRingReader::open(&path).expect("open ring");
        let deadline = Instant::now() + Duration::from_millis(10);
        assert!(
            reader.wait_latest(1, deadline).expect("read").is_none(),
            "nothing publishes"
        );
        writer.publish(5, &dims_bytes(8, 8, 1)).expect("publish");
        let deadline = Instant::now() + Duration::from_secs(2);
        let frame = reader
            .wait_latest(4, deadline)
            .expect("read")
            .expect("seq 5 ≥ 4 in time");
        assert_eq!(frame.seq, 5);
        let deadline = Instant::now() + Duration::from_millis(10);
        assert!(
            reader.wait_latest(6, deadline).expect("read").is_none(),
            "seq 6 never lands"
        );
    }

    /// The skipped counter is writer-side and header-mirrored
    /// (`note_skipped`), so a reader can see the publisher's honest
    /// decline count.
    #[test]
    fn skipped_frames_are_counted_not_silently_dropped() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("session.gwring");
        let mut writer = FrameRingWriter::create(&path, 8, 8).expect("create ring");
        writer.note_skipped();
        writer.note_skipped();
        writer.publish(1, &dims_bytes(8, 8, 1)).expect("publish");
        let reader = FrameRingReader::open(&path).expect("open ring");
        assert_eq!(
            reader.writer_stats().expect("header stats"),
            RingStats {
                published: 1,
                wrapped: 0,
                skipped: 2
            }
        );
    }

    /// Multi-slot publication order: frames land in `seq % slots`, and the
    /// reader always serves the highest committed seq regardless of slot
    /// order.
    #[test]
    fn the_reader_serves_the_highest_committed_seq() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("session.gwring");
        let config = RingConfig {
            slots: 8,
            slot_bytes: rgba_len(8, 8),
        };
        let mut writer = FrameRingWriter::create_with(&path, 8, 8, config).expect("ring");
        for seq in [7u64, 3, 5] {
            writer
                .publish(seq, &dims_bytes(8, 8, u8::try_from(seq).expect("small")))
                .expect("publish");
        }
        let reader = FrameRingReader::open(&path).expect("open ring");
        let frame = reader.latest().expect("read").expect("served");
        assert_eq!(frame.seq, 7, "highest committed seq wins");
        assert_eq!(frame.pixels, dims_bytes(8, 8, 7));
    }

    /// `latest_into` reuses the caller's buffer: same answer as
    /// `latest`, no fresh allocation per poll.
    #[test]
    fn latest_into_reuses_the_callers_buffer() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("session.gwring");
        let mut writer = FrameRingWriter::create(&path, 8, 8).expect("create ring");
        writer.publish(2, &dims_bytes(8, 8, 4)).expect("publish");
        let reader = FrameRingReader::open(&path).expect("open ring");
        let frame = reader
            .latest_into(Vec::new())
            .expect("read")
            .expect("served");
        assert_eq!(frame.pixels, dims_bytes(8, 8, 4));
        let frame = reader
            .latest_into(frame.pixels)
            .expect("read")
            .expect("served");
        assert_eq!(frame.seq, 2);
        assert_eq!(frame.pixels, dims_bytes(8, 8, 4));
    }
}
