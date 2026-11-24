# M88 — The §15.6 shared-frame ring: same-host pixels land outside the door, digest-guarded, wire-unchanged

**Status:** implemented, green end to end. The architecture's §15.6 (IPC
design) carried one explicit `unshipped` roadmap row: *"a shared-memory
frame ring for same-host clients (zero-copy pixels), mirroring CDP's
binary socket philosophy."* This record pays it. **No wire change**: the
frame stream is announced by the existing `FramePresented` events (§7,
M12) whose `seq` numbers are the ring's publication order; the pixels
themselves travel through the shared store, never through the GWP
conversation.

---

## 1. Why now

Every door (UDS/TCP/WS/duplex) moves large payloads through the blob
side-channel (`blob.rs`): a socket hop, the framing guard, an LZ4 pass.
For a **same-host** client that cost is pure overhead — the bytes already
live in the same kernel page cache. §15.6 scopes the fix precisely: a
shared-memory frame ring for same-host clients, newest-wins, CDP
binary-socket shaped. The seam was the last `unshipped` line in the
architecture doc (the other, §12's WASM viewer, is the "full vision"
beyond the shipped `trace.html` emitter and carries no version promise).

## 2. The mechanism (`guiwright-proto/src/shmring.rs`)

A file-backed fixed-slot newest-wins SPSC ring: `header (96 B) | slot
(record 32 B + payload)`, addressed by `seq % slots`.

* **Writer** (`FrameRingWriter::create_with`) — `create_new` (never
  truncates a taken path), sets the file length, writes the header
  (magic `GWRING1\0`, format tag RGBA8, dims, geometry, stats). `publish`
  writes the payload, then the 32-byte record (`epoch = 2·seq`, `seq`,
  length, xxh3 digest), then the header stats — payload first, commit
  last.
* **Reader** (`FrameRingReader::open`) — validates magic/format/
  geometry/length on open (a too-short foreign file answers
  `BadMagic`, not a bare io error), then serves the highest committed,
  **digest-validated** seq. `latest()` allocates once;
  `latest_into(buf)` reuses the caller's buffer (the steady state
  allocates nothing); `wait_latest(min_seq, deadline)` is the bounded
  tool-poll beside the event-fed route (§15.3's zero-polling discipline
  is untouched — the driver never spins).
* **Two honesty notes, both load-bearing (§18):**
  1. *Synchronization is validated, not assumed.* The crate forbids
     `unsafe` (the workspace-wide 1.0 posture), so no atomic word sits on
     the store and a plain byte store cannot be *ordered* across
     processes from safe code. Ordering is instead *checked*: every slot
     carries the xxh3 digest of its payload (the §10 scene-digest hash
     family the recorder and frame stamps already use), and a reader
     serves only what the digest vouches for — a torn or half-published
     slot is skipped, never served (false-accept ≈ 2⁻⁶⁴ per read, the
     same risk the recorder's hash-dedup carries).
  2. *"Zero-copy" scope.* The plan's mmap-borrow read needs an `unsafe`
     mmap edge (`memmap2`'s file mappings are `unsafe fn` constructors —
     its safe `map_if_safe` serves only sealed memfds/fs-verity files),
     which the workspace forbid rules out. The shipped ring lands the
     seam on positioned `read_at`/`write_at` I/O (std `FileExt`, unix +
     windows): the socket, framing, LZ4 and blob-guard copies are gone;
     the reader costs one page-cache copy; the mapping-backed borrow
     stays the plan's recorded follow-up. The writer's single capture
     copy is unavoidable in either shape.

## 3. Wire-up (`guiwright-driver/src/frame_ring.rs`, `session.rs`, `server.rs`)

* **`FrameRingPublisher`** — the session-side offer loop: every presented
  frame rides the same `last_frame` tap the §10 video recorder uses.
  Pixel-less frames and geometry mismatches are **counted skips** (the
  honest header counter), ring errors log at `debug!` and never disturb
  the session (§15.6 backpressure posture: the UI thread's only cost is
  the write itself). The store materializes on the first publishable
  frame — its declared geometry is that frame's dimensions — so
  pixel-less adapters leave no file behind.
* **`Session::frame_ring_start[_with]` / `frame_ring_stop` /
  `frame_ring_stats`** — the embedded API, mirroring the video service's
  shape (single-writer start errors name the SPSC contract; stop is
  idempotent-via-error like `video_stop`).
* **`ServerOptions::frame_ring`** — the remote-mode knob; the
  `GUIWRIGHT_FRAME_RING` env var is the fallback (`GUIWRIGHT_VIDEO_DIR`
  pattern). A ring that fails to start is a session without the ring,
  never a session that fails.

## 4. The evidence

* **Unit (`guiwright-proto/src/shmring.rs` tests, 12):** the acceptance
  fact (a published frame is visible to a same-host reader with its
  identity facts), wrap/newest-wins with writer+header stats, the torn
  slot never served, reader-before-commit, size-mismatch naming, foreign
  file + taken path, geometry guards, second-writer refusal, bounded
  `wait_latest`, the skipped counter, highest-seq-wins, `latest_into`
  buffer reuse. **Red→green recorded**: with the slot-commit writes
  withheld, the acceptance test fails ("committed frame served" panic);
  restored, all 12 pass.
* **Integration (`guiwright-driver/tests/frame_ring.rs`, 5):** embedded
  session publishes (seq == `frame_seq`, exact pixels), pixel-less skips
  (no file), single-writer rejections, custom slot budget wrap, and the
  UDS door end-to-end (`ServerOptions::frame_ring` → a client drives a
  screenshot → the same-host reader serves the newest frame, exact
  pixels, within the 2 s budget).
* **Example (`wire_frame_ring.rs`):** server + client + reader in one
  binary — 10 frames published across attach + two clicks, wrap counted,
  digest-validated.
* **Budget (`bench/budgets.toml` + the crate copy, `shm_ring_frame_hop`):
  one publish + `latest_into` hop of the §15.1 bench frame
  (1280×800 RGBA8) — the §15.7 row that pins the seam's cost.

## 5. What this does not claim

* Not zero-copy at the mapping level (see the honesty note above) — the
  mmap-borrow variant is the recorded §15.6 follow-up, gated on the
  workspace's `unsafe` forbid.
* Not a cross-host path: TCP/WS clients keep the blob side-channel
  (`blob.rs`), exactly as §15.6 scopes the ring to same-host clients.
* No wire change, no new protocol rows, no schema bump: `schema-golden`
  untouched.
