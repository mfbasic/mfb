# bug-416: Windows WASAPI audio backend — `audio::available` returns bytes not frames, capture drops the tail of every non-aligned read, plus a latent shared-mix OOB read and a stale module doc

Last updated: 2026-07-28
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness (cross-platform divergence + capture data loss) + Memory-safety
(LOW) + wrong-comment (LOW)

STATUS: FIXED (2d369db4f)
Regression Test: `src/target/shared/code/audio/windows_tests.rs` (emit-inspection,
`cargo test --bin mfb windows_tests`) — (1) `audio::available` emits no `mul` writing
the result register (frames, not bytes); (2) `audio::read`/`readTimeout` drain a
carry-over tail (a second `emit_read_fill`) and save the unconsumed packet tail
(`*_carry_tail` labels) before `ReleaseBuffer`; (3) `openInput` loads `W_MIX_CH` to
reject `mixCh < userCh`. Runtime proof of the WASAPI paths is Windows-only (box 2230).

Fix summary (all four defects, Windows-only codegen):
- (1) Dropped the `* BPF` scale in the `Query::Available` arm (`windows_io.rs`).
- (2) Added a per-stream capture carry-over stash (`W_CARRY_PTR/FRAMES/HEAD`, one
  device buffer wide, input streams only; `windows.rs`/`windows_open.rs`). The read
  loop copies the unconsumed tail out before the (mandatory whole-packet)
  `ReleaseBuffer`; the next read drains it first — no captured frame is dropped.
- (3) Reject `mixCh < userCh` at SHARED open (`windows_open.rs`) — the read
  converter cannot synthesize channels the device lacks; the write path is already
  bounded on `mixCh`.
- (4) Rewrote the stale module header and removed the dangling `branch_if_hr`
  comment (`windows.rs`) — no buffer-alignment retry exists, AUTOCONVERTPCM is
  deliberately unused.
- Byte-identity: `windows-x86_64.ncodesum` regenerated; the four non-Windows audio
  goldens are byte-identical (change is confined to the `windows` module).

Four defects in the Windows WASAPI audio backend, batched (same subsystem):

### (1) `audio/windows_io.rs:706` — `audio::available` returns bytes, not frames (MED)
`audio::available(stream)` is specified (man `audio/available.md`) to return **whole
frames**, and both other backends do (macOS `free_top*bufferFrames`/`fill/bpf`; ALSA
the raw `snd_pcm_avail_update` frame count). The Windows `Query::Available` arm
computes `RESULT = avail_frames * BPF` (`load BPF_OFF`, `multiply_registers`),
returning **bytes** — 2× (mono) or 4× (stereo) too large. The man example
`LET n = audio::available(mic); audio::read(mic, n)` then requests bpf× too many
frames on Windows (over-read / block / exceed `READ_FRAMES_MAX` → ErrInvalidArgument).
The sibling `Query::Poll` arm one block below correctly uses the raw frame count,
confirming frames is intended.
- Fix: drop the `* BPF` in the `Query::Available` arm.

### (2) `audio/windows_io.rs:557` — capture drops unconsumed frames (MED, data loss)
In `lower_read`, each `IAudioCaptureClient::GetBuffer` returns `numFrames`; the code
copies `min(numFrames, frames-got)` then calls `ReleaseBuffer(numFrames)` — releasing
the whole packet (WASAPI requires `NumFramesRead == GetBuffer count` or 0; you cannot
partially consume). When `numFrames > frames-got` (the final packet of essentially
any read whose length isn't packet/period-aligned — the common case), the
`numFrames - copyFrames` unconsumed frames are permanently discarded and the next
`audio::read` starts on a fresh packet → a gap in captured audio. ALSA leaves the
remainder in the kernel buffer and macOS in the ring; only Windows drops it.
- Fix: buffer the unconsumed tail (a small carry-over ring) so the next read
  continues it, or size reads to whole packets.

### (3) `audio/windows_io.rs:382` — shared-mix read OOB when mixCh < userCh (LOW, latent)
In `emit_read_fill` (SHARED mix), the inner loop reads `pData + f*mixBpf + c*4` for
`c ∈ [0, userCh)` asserting "userCh<=mixCh", but `windows_open.rs`'s GetMixFormat
path never checks `mixCh >= userCh` (only rate and 32-bit). A mono shared-mix device
(`mixCh=1`) with a stereo `openInput` reads 4 bytes past the WASAPI capture buffer on
the last frame. Requires EXCLUSIVE-refused + mono-mix capture + stereo open, hence
LOW/latent. (The write path bounds `c` on `mixCh`, safe.)
- Fix: reject `mixCh < userCh` at open, or clamp the read `c` loop to `mixCh`.

### (4) `audio/windows.rs:23` — stale module doc describes non-existent mechanism (LOW)
The module header claims EXCLUSIVE is tried first "with the buffer-alignment retry …
falls back to SHARED with AUTOCONVERTPCM" and references `AUDCLNT_E_BUFFER_SIZE_NOT_
ALIGNED … built by shift+add in `branch_if_hr``. None exists: there is no
`branch_if_hr`, the alignment error is never handled (`lower_open` branches straight
to `use_shared`), and SHARED uses manual s16↔f32 conversion (`windows_open.rs:10-11`
explicitly: "AUTOCONVERTPCM is deliberately NOT used").
- Fix: rewrite the header to describe the actual fallback.

References: `src/target/shared/code/audio/windows_io.rs:706`/`:557`/`:382`,
`audio/windows.rs:23`; man `audio/available.md`. ALSA/macOS backends verified clean.
Found during goal-07.

## Failing Reproduction

Windows-only; not reproducible on the macOS host. Confirmed statically: the
`* BPF` multiply in `Query::Available` vs the raw count in `Query::Poll`; the
`ReleaseBuffer(numFrames)` after a `min(...)` copy; the absent `mixCh>=userCh` guard;
`grep branch_if_hr` returns only the comment.

- Observed: (1) `available` = bytes; (2) captured audio gaps on non-aligned reads;
  (3) 4-byte OOB read on a mono-mix stereo-open device; (4) doc contradicts code.
- Expected: (1) frames; (2) no dropped frames; (3) in-bounds; (4) accurate doc.
- Confirmed statically before the fix (worktree-B-416): `* BPF` in `Query::Available`
  vs the raw count in `Query::Poll`; `ReleaseBuffer(numFrames)` after a `min(...)`
  copy; the absent `mixCh >= userCh` guard; `grep branch_if_hr` = comment only. All
  now hold in the emit-inspection tests (RED→GREEN) + a clean windows-x86_64 encode.

## Root Cause

The WASAPI backend diverges from the frame-unit contract, mishandles partial packet
consumption, lacks a mix-channel bound, and carries a stale header.

## Goal

- Windows `audio::available` returns frames; capture loses no frames; shared-mix
  reads stay in-bounds; the module doc matches the code.

### Non-goals

- The ALSA/macOS backends (correct). The EXCLUSIVE/SHARED selection logic itself.

## Blast Radius

- `windows_io.rs:706`/`:557`/`:382`, `windows.rs:23` — all Windows-WASAPI-only.
