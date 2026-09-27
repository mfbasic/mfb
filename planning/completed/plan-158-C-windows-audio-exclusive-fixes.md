# plan-158-C: Windows audio — exclusive-mode write and stream preemption

Last updated: 2026-09-26
Effort: large (3h–1d)
Depends on: plan-158-B

Two Windows `audio::` bugs measured on 2026-09-26 on box 192.168.1.206 (Win11 26200):

1. **An EXCLUSIVE output stream never plays.** `audio::openOutput(dev, 44100, 2, 512)`
   and `(dev, 48000, 2, 512)` open in EXCLUSIVE mode, and the first `audio::write`
   never returns and makes no sound. This is why `examples/audio`'s libsnd Ogg tracks
   are silent on Windows. libsnd decoding itself was verified correct on the box: PCM
   matched macOS apart from ±1 decoder rounding.
2. **Opening a second stream invalidates the first.** With a SHARED 48000/1 stream open,
   opening (and closing) another stream on the same device makes the first stream's
   next `GetBuffer` fail with `0x88890004 AUDCLNT_E_DEVICE_INVALIDATED`. In
   `examples/audio`, opening the Ogg stream kills the MML stream `out`.

Also measured: on the "Display Audio" device with no sink attached, the EXCLUSIVE write
hangs the same way.

**Outcome:** on the box, an EXCLUSIVE 44100/2 stream plays audibly and `audio::write`
returns. A write to a stream that cannot progress fails with an error instead of
hanging. The preemption behaviour is either removed or documented, per the Open Decision.

References: the measurements above (session 2026-09-26, throwaway instrumented compiler, since reverted), `.ai/arch-abi.md` §Windows
audio, `src/codegen/builtins/audio/gen_windows_open.rs` (`lower_open`:
EXCLUSIVE-then-SHARED), `gen_windows_io.rs` (`lower_write`, `emit_write_fill`),
`planning/completed/plan-66-windows-parity.md` Open Decision 1 (why EXCLUSIVE is tried
first: the `audio::` no-resampling s16le contract).

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-158-B complete (archived) | `ls planning/completed/plan-158-B-*` → one file | MET (2026-09-26: `planning/completed/plan-158-B-audio-status-macos-alsa.md`) |

If plan-158-B is not complete, this plan cannot start, full stop. (plan-158-A's status
messages are what Phase 1 reads.)

## 1. Goal

- EXCLUSIVE render streams play and `audio::write` completes, for write sizes that are
  not multiples of the device buffer (the example writes 500-frame chunks into a
  512-frame stream).
- No `audio::write` can block forever: a stalled device raises `ErrAudioDevice` with a
  status message.
- The stream-preemption behaviour matches the decision recorded below.

### Non-goals

- The `audio::` API and its no-resampling contract (plan-66 Open Decision 1) are
  unchanged.
- SHARED-mode conversion (`emit_write_fill` f32 path) is unchanged except where a fix
  requires otherwise.
- Capture (`lower_read`) is out of scope unless Phase 1 shows the same defect there.

## 2. Current State

- `lower_open` calls `Initialize(EXCLUSIVE, EVENTCALLBACK, dur, dur, s16 wfx)`. On
  failure it re-activates and tries SHARED at the mix format, and it requires mix rate
  == requested rate, 32-bit float, mix channels >= requested. It calls `Start()` inside
  open, before any data is written.
- `lower_write` loops: `WaitForSingleObject(event, INFINITE)`, `GetCurrentPadding`,
  `avail = W_BUFFER - padding`, `GetBuffer(min(remaining, avail))`, fill,
  `ReleaseBuffer`. It is the same loop for both modes.
- plan-158-A measured (box 206, SHARED 48000/1, a second stream opened and closed on
  the device): the first `write` afterwards fails with
  `(IAudioClient::GetCurrentPadding: 0x88890004)` (`AUDCLNT_E_DEVICE_INVALIDATED`), and
  the next `write` never returns. Its `WaitForSingleObject(event, INFINITE)` waits on
  an event the invalidated stream no longer signals (plan-158-A Corrections C10).
- WASAPI documents that in EXCLUSIVE event-driven mode the client fills the entire
  buffer each pass, and pre-rolls one buffer before `Start()`. The current loop requests
  partial buffers and never pre-rolls. UNVERIFIED which of these causes the hang;
  Phase 1 measures it.

### Measured populations

| What | Count | Command |
|---|---|---|
| Windows audio emit-inspection tests | 5 | `grep -c '#\[test\]' src/codegen/builtins/audio/gen_windows_tests.rs` |
| Windows goldens touching audio | 1 | `ls tests/byte-identity/audio/golden/*windows*` → `audio_codegen_cover_rt.windows-x86_64.ncodesum` |

## 3. Design Overview

Measure first, using plan-158-A's status messages plus a bounded wait, then fix.

Expected design, to be confirmed by Phase 1:

- **Pre-roll:** open no longer calls `Start()` for an EXCLUSIVE render stream.
- **Carry buffer:** allocate a render carry buffer (`W_BUFFER * BPF` bytes, mirroring
  the bug-416 capture carry). `write` appends into it, and each time it holds a full
  buffer it submits: first submission `GetBuffer(W_BUFFER)` → copy → `ReleaseBuffer` →
  `Start()`; later ones wait for the event, then do the same.
- **Tail on close:** `close` pads the carry with silence, submits it, and waits one
  period before `Stop`.
- **Bounded wait:** every wait gets a timeout of 2 × buffer duration (min 200 ms). On
  timeout, raise `ErrAudioDevice` with status `WAIT_TIMEOUT` (0x102) through
  `StatusFailSites` (plan-158-B deleted the unused single-site `emit_fail_status`).

Byte-identity is not the gate. Expected diff: the Windows audio ncodesum.

## Phases

> **NOTE — keep the checkboxes current as you go** (same rules as plan-158-A).

### Phase 1 — Measure the hang and the preemption

- [x] Temporarily bound `lower_write`'s wait (not committed), and run the EXCLUSIVE
      44100/2 probe with a watchdog on the box. Record where it spins (padding == buffer
      forever? `GetBuffer` failure? event never signalled?) and the statuses, in
      §Corrections. — padding == buffer forever (C1).
- [x] Record whether EXCLUSIVE `Initialize` succeeds while a SHARED stream is open, and
      the SHARED stream's status afterwards. — the attempt invalidates the SHARED
      stream whether or not it succeeds (C2).

Acceptance: §Corrections names the root cause of both bugs, with probe output.
  Check: `grep -c 'root cause' planning/plan-158-C-windows-audio-exclusive-fixes.md` → ≥ 2
  (est. 1 min).
Commit: 30d371d83

### Phase 2 — Fix EXCLUSIVE render

- [x] `gen_windows.rs`: state fields for the render carry (`W_RCARRY_PTR`,
      `W_RCARRY_FRAMES`), placed in unused STATE-block offsets or by extending `W_SIZE`.
      — the existing `W_CARRY_PTR`/`W_CARRY_FRAMES` serve (C3); `RENDER_WAIT_FLOOR_MS`
      added.
- [x] `gen_windows_open.rs`: skip `Start()` for EXCLUSIVE render; allocate the carry.
- [x] `gen_windows_io.rs` `lower_write`: the EXCLUSIVE path as designed (§3), plus the
      bounded wait on both paths. — `emit_exclusive_submit`, `emit_render_wait_timeout`.
- [x] `gen_windows_open.rs` `lower_close`: flush the carry tail, then `Stop`.
- [x] `available`/`poll` for an unstarted EXCLUSIVE output (C4): `available` reports
      the carry's room; `poll`/`pollTimeout` answer TRUE before the first submit
      (`emit_unstarted_exclusive_output_ready`).
- [x] `gen_windows_tests.rs`: emit-inspection tests: no `Start` in the EXCLUSIVE open
      path before the first submit; `lower_write` has no INFINITE wait. —
      `exclusive_output_open_does_not_start_the_stream`, `write_has_no_infinite_wait`;
      `windows_audio_failures_report_their_os_status` updated for the new call names
      (C5). `cargo test --release --bin mfb audio` → 24 passed.
- [x] Regenerate the Windows audio ncodesum (expected diff). — base (`54e7a1efa`) vs
      current `-ncode` changes only `_mfb_rt_audio_audio_{available,closeInput,
      closeOutput,openInput,openInputDevice,openOutput,openOutputDevice,poll,
      pollTimeout,write}` and the `_mfb_str_errstatus_*` set (the `mix *` and bare
      `Initialize` prefixes go, `Initialize(SHARED)`/`(EXCLUSIVE)` and
      `WaitForSingleObject` come); `regen-native-goldens.sh` → only the windows sum
      changed.
- [x] Re-run plan-158-A's leak loop on the write path, which its C10 could not run while
      the wait was unbounded: `/tmp/p158/wprobe`-style probe, a SHARED stream
      invalidated by a second open, then 10 000 failing `write`s under TRAP in a
      `--debug` build. `arena.0.live_bytes` equal at N=10 and N=10 000. — the second
      stream is now 44100/2 (only an EXCLUSIVE open invalidates, C2/C6). `wprobe trap 10`
      → `write failures=10`, `live_bytes 96`; `wprobe trap 10000` → `write failures=
      10000`, `alloc_calls 60048`, `free_calls 60044`, `live_bytes 96`. Each failing write
      after the first returns in the 200 ms bound (the run took about 33 min).

Acceptance: on the box, a 2 s 440 Hz tone written in 500-frame chunks to an EXCLUSIVE
44100/2 stream finishes within the watchdog, and the user confirms it is audible.
  Check: box probe prints `wrote N chunks OK` in under 5 s (est. 3 min).
  — met: `cprobe tone 44100 2` → `wrote 176 chunks OK in 1972 ms` (ssh) and three
  console-session runs (`schtasks /it`) → `1968`/`1967`/`1975 ms`; the user confirmed
  the three tones were audible (2026-09-27). `tone 48000 2` → `wrote 192 chunks OK in
  1979 ms`.
Commit: 9caf5a9f7 (code), 64fee39e9 (docs), 7b3b9ff57 (golden + ledger)

### Phase 3 — Preemption

- [x] Implement the Open Decision below. Test: the SHARED-stream + second-stream probe
      either no longer fails, or fails with a documented, named status. — SHARED-first
      (landed with Phase 2's open, C6). `cprobe preempt 48000 1` → `first after: wrote
      192 chunks OK in 1979 ms` (no longer fails); `preempt 44100 2` (the second must be
      EXCLUSIVE) → `second: wrote 176 chunks OK`, then `first after failed: …
      (IAudioClient::GetCurrentPadding: 0x88890004)`, the status `11_audio.md`
      §"Windows share modes and preemption" names.

Acceptance: probe output recorded; `11_audio.md` states the behaviour.
  Check: `grep -n 'exclusive' src/docs/spec/stdlib/11_audio.md` → the preemption
  paragraph (est. 1 min). — met (lines 142–153); the `audio` man description says the
  same for the developer.
Commit: 9caf5a9f7 (with Phase 2, C6), 64fee39e9 (spec)

## Validation Plan

- Runtime proof: the box probes, plus `examples/audio` menu items 1–4 played by the user
  (plan-158-D re-runs this with the final package).
- Doc sync: `.ai/arch-abi.md` §Windows audio (EXCLUSIVE render pre-roll and carry
  invariant); `src/docs/spec/stdlib/11_audio.md`.
- Final gate: `scripts/artifact-gate.sh` (est. 15 min).

## Open Decisions

- **Decided (user, 2026-09-27): SHARED-first.** The measured fact that decided it: an
  EXCLUSIVE attempt invalidates SHARED streams even when it fails (C2).
- **Preemption:** try SHARED first whenever the requested rate equals the mix rate, and
  use EXCLUSIVE only when SHARED cannot serve the format (recommended: MML at 48 kHz then
  never grabs the device, and only a 44.1 kHz file does) vs. keep EXCLUSIVE-first and
  document that opening a stream can invalidate other streams on the device.
  Recommended option changes plan-66 Open Decision 1's order; the user decides.

## Corrections

Phase 1 instruments (never committed): a detached worktree at `c58e7dbab`
(`/tmp/p158/instr`) whose `lower_write` waits at most 2000 ms and raises
`INSTR WaitForSingleObject` on a timeout, raises `INSTR padding stuck` with the padding
after 200 consecutive zero-avail passes, and whose `lower_open` stores
`1000 + W_SHARED` in `W_XRUNS` so `audio::xruns` reports the share mode (1000 =
EXCLUSIVE, 1001 = SHARED). Probe `/tmp/p158/cprobe` on box 206, default output
("Speakers (2- Synaptics HD Audio)", mix format 48000 Hz float).

- **C1 — root cause of bug 1 (EXCLUSIVE write never returns).** `cprobe tone 44100 2`
  → `opened 44100/2 mode=1000`, `tone write failed: … (INSTR padding stuck:
  0x00000200)`. `tone 48000 2` → `mode=1000`, the same. `tone 48000 1` →
  `mode=1001`, `wrote 192 chunks OK in 1969 ms`. The event IS signalled (200 waits,
  none timed out) and `GetBuffer` is never reached: in EXCLUSIVE event-driven mode
  `GetCurrentPadding` reports the whole buffer (512 frames) on every pass, so
  `avail = W_BUFFER - padding` is 0 forever and the loop spins. A 48000/1 open
  works only because EXCLUSIVE refuses mono and the open falls back to SHARED.
- **C2 — root cause of bug 2 (a second open invalidates the first).**
  `cprobe preempt 48000 1` → the second stream ends up SHARED (`mode=1001`), plays
  (`wrote 192 chunks OK`), and still the first fails afterwards with
  `(IAudioClient::GetCurrentPadding: 0x88890004)`. `preempt 44100 2` (second is
  EXCLUSIVE) → the same failure on the first. With the EXCLUSIVE attempt removed
  from `lower_open` (instrument variant, every open SHARED), `preempt 48000 1` →
  `first after: wrote 192 chunks OK in 1980 ms`. So the EXCLUSIVE `Initialize`
  attempt invalidates every SHARED stream on the endpoint, even when it fails and
  the open falls back to SHARED. Under EXCLUSIVE-first, opening ANY stream kills
  the others.
- **C3 (no new state fields).** The render carry reuses `W_CARRY_PTR`/`W_CARRY_FRAMES`:
  the capture carry is input-only and a stream has one direction, both carries are
  one device buffer in the device format (`W_BUFFER * W_MIX_BPF`, with
  `W_MIX_BPF = BPF` in EXCLUSIVE), so `emit_free_wasapi_state` and the failed-open
  release already free it correctly. `W_SIZE` is unchanged.
- **C4 (poll/available would have hung or lied).** Not starting an EXCLUSIVE output at
  open means its event never fires until the first submit, so `poll` (an INFINITE
  wait by the convention) would block forever on a fresh stream, and `available`
  (`W_BUFFER - padding`, padding always the whole buffer, C1) would report 0 forever.
  Both now answer from the carry for an EXCLUSIVE output. Not in the plan's task list;
  added.
- **C5 (the Windows status call names changed).** SHARED-first makes the mix checks
  choose the mode instead of failing, so the `mix rate`/`mix bits`/`mix channels`
  status sites are gone, and `Initialize` is reported as `Initialize(SHARED)` or
  `Initialize(EXCLUSIVE)`. `write` gains `WaitForSingleObject` (the bounded wait) and
  `IAudioClient::Start` (the pre-rolled first submit). plan-158-A's
  `windows_audio_failures_report_their_os_status` pinned the old set; its expectation
  is updated to the decided design (the test was written in this plan family; its old
  set described EXCLUSIVE-first, which the user's decision replaced). The `mix rate`
  examples in `11_audio.md` and the man page are replaced.
- **C6 (Phase 3 landed with Phase 2).** SHARED-first is a rewrite of the same part of
  `lower_open` as Phase 2's pre-roll (the init block, the Start and the carry), so the
  two were written and committed together; `emit_reactivate_client` (the retry after a
  failed EXCLUSIVE `Initialize`) has no caller any more and is deleted.
- **C7 (acceptance measured over ssh and at the console).** The ssh session reaches the
  device (writes pace at real time), but audibility was checked in the logged-in
  console session through `schtasks /it`, as `.ai/remote_systems.md` prescribes for
  box 206.

## Summary

The risk is in the EXCLUSIVE carry/pre-roll protocol, which only runs on the box.
Measure first (Phase 1) so the fix targets the real cause.
