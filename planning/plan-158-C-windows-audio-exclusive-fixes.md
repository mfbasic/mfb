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
| plan-158-B complete (archived) | `ls planning/completed/plan-158-B-*` → one file | NOT MET |

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
  `emit_fail_status`.

Byte-identity is not the gate. Expected diff: the Windows audio ncodesum.

## Phases

> **NOTE — keep the checkboxes current as you go** (same rules as plan-158-A).

### Phase 1 — Measure the hang and the preemption

- [ ] Temporarily bound `lower_write`'s wait (not committed), and run the EXCLUSIVE
      44100/2 probe with a watchdog on the box. Record where it spins (padding == buffer
      forever? `GetBuffer` failure? event never signalled?) and the statuses, in
      §Corrections.
- [ ] Record whether EXCLUSIVE `Initialize` succeeds while a SHARED stream is open, and
      the SHARED stream's status afterwards.

Acceptance: §Corrections names the root cause of both bugs, with probe output.
  Check: `grep -c 'root cause' planning/plan-158-C-windows-audio-exclusive-fixes.md` → ≥ 2
  (est. 1 min).
Commit: —

### Phase 2 — Fix EXCLUSIVE render

- [ ] `gen_windows.rs`: state fields for the render carry (`W_RCARRY_PTR`,
      `W_RCARRY_FRAMES`), placed in unused STATE-block offsets or by extending `W_SIZE`.
- [ ] `gen_windows_open.rs`: skip `Start()` for EXCLUSIVE render; allocate the carry.
- [ ] `gen_windows_io.rs` `lower_write`: the EXCLUSIVE path as designed (§3), plus the
      bounded wait on both paths.
- [ ] `gen_windows_open.rs` `lower_close`: flush the carry tail, then `Stop`.
- [ ] `gen_windows_tests.rs`: emit-inspection tests: no `Start` in the EXCLUSIVE open
      path before the first submit; `lower_write` has no INFINITE wait.
- [ ] Regenerate the Windows audio ncodesum (expected diff).

Acceptance: on the box, a 2 s 440 Hz tone written in 500-frame chunks to an EXCLUSIVE
44100/2 stream finishes within the watchdog, and the user confirms it is audible.
  Check: box probe prints `wrote N chunks OK` in under 5 s (est. 3 min).
Commit: —

### Phase 3 — Preemption

- [ ] Implement the Open Decision below. Test: the SHARED-stream + second-stream probe
      either no longer fails, or fails with a documented, named status.

Acceptance: probe output recorded; `11_audio.md` states the behaviour.
  Check: `grep -n 'exclusive' src/docs/spec/stdlib/11_audio.md` → the preemption
  paragraph (est. 1 min).
Commit: —

## Validation Plan

- Runtime proof: the box probes, plus `examples/audio` menu items 1–4 played by the user
  (plan-158-D re-runs this with the final package).
- Doc sync: `.ai/arch-abi.md` §Windows audio (EXCLUSIVE render pre-roll and carry
  invariant); `src/docs/spec/stdlib/11_audio.md`.
- Final gate: `scripts/artifact-gate.sh` (est. 15 min).

## Open Decisions

- **Preemption:** try SHARED first whenever the requested rate equals the mix rate, and
  use EXCLUSIVE only when SHARED cannot serve the format (recommended: MML at 48 kHz then
  never grabs the device, and only a 44.1 kHz file does) vs. keep EXCLUSIVE-first and
  document that opening a stream can invalidate other streams on the device.
  Recommended option changes plan-66 Open Decision 1's order; the user decides.

## Corrections

## Summary

The risk is in the EXCLUSIVE carry/pre-roll protocol, which only runs on the box.
Measure first (Phase 1) so the fix targets the real cause.
