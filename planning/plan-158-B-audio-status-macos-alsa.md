# plan-158-B: macOS and ALSA audio adopt status messages

Last updated: 2026-09-26
Effort: medium (1h–2h)
Depends on: plan-158-A

Extends plan-158-A's `emit_fail_status` to the two remaining `audio::` backends, so a
failed Core Audio or ALSA call reports its call name and OS status the way Windows does.
**Outcome:** on macOS and Linux, an `audio::` failure's `e.message` ends in
`(<call>: 0x<status>)`, and `e.code` is unchanged.

References: plan-158-A (§3 design, §4.2 `_mfb_rt_fail_status`, `StatusFailSites`; its §Prerequisites table is the gate for this
letter too), `src/codegen/builtins/audio/gen_macos_*.rs`,
`src/codegen/builtins/audio/gen_alsa_*.rs`, `src/docs/spec/stdlib/11_audio.md`.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-158-A complete (archived) | `ls planning/completed/plan-158-A-*` → one file | NOT MET |

If plan-158-A is not complete, this plan cannot start, full stop.

## 1. Goal

- Every OS-status failure site in the macOS and ALSA audio backends raises through
  `emit_fail_status` / `StatusFailSites`.

### Non-goals

- No change to Windows code (plan-158-A) or to the status format (decided in plan-158-A).
- No new error codes.

## 2. Current State

- macOS: Core Audio calls return `OSStatus` in the C return register. The emitters check
  it and branch to shared fail labels. See `gen_macos_shared.rs` (the helper whose doc
  says "Leaves the `OSStatus` in the return register") and `gen_macos_stream.rs`.
- ALSA: `snd_pcm_*` return a negative errno in the C return register. Checks live in
  `gen_alsa_shared.rs`, `gen_alsa_stream.rs`, `gen_alsa_io.rs`, `gen_alsa_devices.rs`.

### Measured populations

| What | Count | Command |
|---|---|---|
| OS-status branches, macOS | 20 (devices 2, io 3, shared 4, stream 11) | `grep -cE 'branch_(eq\|ne\|lt\|le\|gt\|ge\|lo\|hs\|hi\|ls)\(&?[a-z_]*(fail\|unavailable)' src/codegen/builtins/audio/gen_macos*.rs` |
| OS-status branches, ALSA | 13 (devices 1, io 4, shared 5, stream 3) | same command on `gen_alsa*.rs` |
| `emit_fail(` calls, macOS / ALSA | 18 / 17 | `grep -c 'emit_fail(' gen_macos*.rs gen_alsa*.rs` |

### Verified properties

- UNVERIFIED: that every branch counted above has the status still in a register at the
  branch (a check that compares a derived value, e.g. a frame count, has no OS status).
  Phase 1 classifies each one.
- UNVERIFIED: a free frame-slot pair in each backend's frame table for `StatusFailSites`.
  Phase 1 task.

## 3. Design Overview

Mechanical adoption of plan-158-A's API. Byte-identity is not the gate. Expected diffs:
`audio_codegen_cover_rt.{macos-aarch64,linux-aarch64,linux-riscv64,linux-x86_64}.ncodesum`.
Risk is low: each site keeps its label structure and only swaps its raise tail.

## Phases

> **NOTE — keep the checkboxes current as you go** (same rules as plan-158-A).

### Phase 1 — Classify sites

- [ ] For each of the 33 branches, record (file, function, call name, status register or
      "no status") in a table in this plan. Sites without an OS status stay `emit_fail`.
- [ ] Pick the frame-slot pair per backend; record the grep that proves it free.

Acceptance: the table covers 33 rows.
  Check: `grep -c '^| gen_' planning/plan-158-B-audio-status-macos-alsa.md` → 33 (est. 1 min).
Commit: —

### Phase 2 — Adopt

- [ ] macOS files: convert the status-bearing sites from Phase 1.
- [ ] ALSA files: convert the status-bearing sites from Phase 1.
- [ ] Regenerate the four `audio_codegen_cover_rt.*.ncodesum` goldens. Confirm with
      `-ncode` that each delta is confined to audio helpers.
- [ ] Runtime proof, macOS (local): open an output on a device id that does not exist →
      `e.message` ends with `: 0x…)`.
- [ ] Runtime proof, Linux: the same probe on box 2223 (native aarch64 glibc), with a
      bogus ALSA device name.

Acceptance: both probes print the status suffix.
  Check: `./build/probe.out` → a line matching `\(.*: 0x[0-9a-f]{8}\)` (est. 2 min per box).
Commit: —

## Validation Plan

- Doc sync: `src/docs/spec/stdlib/11_audio.md` backend sections name the macOS/ALSA calls.
- Final gate: `scripts/artifact-gate.sh` (est. 15 min: audio goldens on four targets).

## Open Decisions

- ALSA: append `snd_strerror(err)` text instead of hex? Recommended **no** for this plan:
  it needs the C string copied into the message, which the plan-158-A primitive does not
  support. File it as a follow-up if wanted.

## Corrections

## Summary

Adoption only; the error-path logic (call-site tag-3 re-stamp, OOM arms) is proven in plan-158-A.
