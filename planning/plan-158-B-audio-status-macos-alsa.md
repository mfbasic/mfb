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
| plan-158-A complete (archived) | `ls planning/completed/plan-158-A-*` → one file | MET (2026-09-26: `planning/completed/plan-158-A-native-error-status-messages.md`) |

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

- Which branches have an OS status at the branch: classified in the Phase 1 table
  (2026-09-26, read at each site with `/tmp/p158/ctx.py`).
- Free frame-slot pairs: macOS stream frame (`F = 512`) 488/496, macOS devices frame
  (`FRAME_SIZE = 1024`) 1008/1016, ALSA (`FRAME = 640`) 536/544. Proof: the highest
  named slots are `MAPSIZE_OFF = 480`, `DEVLIST_OFF = 1000` and `DEVLIST_OFF = 528`
  (`grep -n "const .*_OFF: usize" src/codegen/builtins/audio/gen_{macos,alsa}_shared.rs`),
  and `grep -nE "\b(488|496|504|536|544|1008|1016)\b" src/codegen/builtins/audio/gen_{macos,alsa}*.rs`
  → no match.

### Site classification (Phase 1)

"ret" = the call's result in `abi::return_register()` at the branch (sign-extended
`OSStatus` or negative errno). "value" = the observed device value, reported the way
plan-158-A reports Windows' `mix rate` (moved into the return register at the site).
Rows marked "no status" keep `emit_fail`. The 33 grep rows plus the 8 further calls
behind the shared `check` closure (§Corrections C1) are 41 rows.

| file | function | call | status |
|---|---|---|---|
| gen_macos_io.rs:53 | lower_write | closed-stream check | no status |
| gen_macos_io.rs:222 | lower_write | AudioQueueEnqueueBuffer | ret |
| gen_macos_io.rs:654 | lower_read | closed-stream check | no status |
| gen_macos_devices.rs:97 | lower_devices | AudioObjectGetPropertyData | ret |
| gen_macos_devices.rs:103 | lower_devices | device count == 0 | no status |
| gen_macos_shared.rs:240 | emit_select_device | CFStringCreateWithCString (NULL) | no status |
| gen_macos_shared.rs:271 | emit_select_device | AudioQueueSetProperty | ret (saved across `CFRelease`) |
| gen_macos_shared.rs:427 | emit_cfstring_field | AudioObjectGetPropertyData | ret |
| gen_macos_shared.rs:458 | emit_cfstring_field | CFStringGetCString (false) | no status |
| gen_macos_stream.rs:119 | lower_open_output | mmap (MAP_FAILED) | no status |
| gen_macos_stream.rs:209 | lower_open_output | AudioQueueNewOutput | ret |
| gen_macos_stream.rs:251 | lower_open_output | AudioQueueAllocateBuffer | ret |
| gen_macos_stream.rs:288 | lower_open_output | AudioQueueStart | ret |
| gen_macos_stream.rs:655 | lower_open_input | AudioObjectGetPropertyData (default input) | ret |
| gen_macos_stream.rs:658 | lower_open_input | default input id == 0 | no status |
| gen_macos_stream.rs:724 | lower_open_input | mmap (MAP_FAILED) | no status |
| gen_macos_stream.rs:811 | lower_open_input | AudioQueueNewInput | ret |
| gen_macos_stream.rs:852 | lower_open_input | AudioQueueAllocateBuffer | ret |
| gen_macos_stream.rs:870 | lower_open_input | AudioQueueEnqueueBuffer | ret |
| gen_macos_stream.rs:892 | lower_open_input | AudioQueueStart | ret |
| gen_alsa_devices.rs:77 | lower_devices | snd_device_name_hint | ret |
| gen_alsa_io.rs:47 | lower_write | closed-stream check | no status |
| gen_alsa_io.rs:166 | lower_write | snd_pcm_recover | ret |
| gen_alsa_io.rs:254 | lower_read | closed-stream check | no status |
| gen_alsa_io.rs:522 | lower_read | snd_pcm_recover | ret |
| gen_alsa_shared.rs:261 | emit_dlopen | dlopen (NULL) | no status |
| gen_alsa_shared.rs:293 | emit_dlsym | dlsym (NULL) | no status |
| gen_alsa_shared.rs:551 | emit_configure_hw_params | snd_pcm_hw_params_malloc | ret |
| gen_alsa_shared.rs:551 | emit_configure_hw_params | snd_pcm_hw_params_any | ret |
| gen_alsa_shared.rs:551 | emit_configure_hw_params | snd_pcm_hw_params_set_access | ret |
| gen_alsa_shared.rs:551 | emit_configure_hw_params | snd_pcm_hw_params_set_format | ret |
| gen_alsa_shared.rs:551 | emit_configure_hw_params | snd_pcm_hw_params_set_channels | ret |
| gen_alsa_shared.rs:551 | emit_configure_hw_params | snd_pcm_hw_params_set_rate_near | ret |
| gen_alsa_shared.rs:551 | emit_configure_hw_params | snd_pcm_hw_params_set_period_size_near | ret |
| gen_alsa_shared.rs:551 | emit_configure_hw_params | snd_pcm_hw_params_set_buffer_size_near | ret |
| gen_alsa_shared.rs:551 | emit_configure_hw_params | snd_pcm_hw_params | ret |
| gen_alsa_shared.rs:841 | emit_configure_hw_params | rate (committed ≠ requested) | value |
| gen_alsa_shared.rs:845 | emit_configure_hw_params | channels (committed ≠ requested) | value |
| gen_alsa_stream.rs:114 | lower_open | mmap (MAP_FAILED) | no status |
| gen_alsa_stream.rs:181 | lower_open | snd_pcm_open | ret |
| gen_alsa_stream.rs:217 | lower_open | snd_pcm_prepare | ret |

## 3. Design Overview

Mechanical adoption of plan-158-A's API. Byte-identity is not the gate. Expected diffs:
`audio_codegen_cover_rt.{macos-aarch64,linux-aarch64,linux-riscv64,linux-x86_64}.ncodesum`.
Risk is low: each site keeps its label structure and only swaps its raise tail.

## Phases

> **NOTE — keep the checkboxes current as you go** (same rules as plan-158-A).

### Phase 1 — Classify sites

- [x] For each of the 33 branches, record (file, function, call name, status register or
      "no status") in a table in this plan. Sites without an OS status stay `emit_fail`.
      — §Site classification: 41 rows (C1), 28 with a status, 13 without.
- [x] Pick the frame-slot pair per backend; record the grep that proves it free. —
      §Verified properties.

Acceptance: the table covers every grep row plus the calls behind the shared `check`
closure (strengthened from 33, C1).
  Check: `grep -c '^| gen_' planning/plan-158-B-audio-status-macos-alsa.md` → 41 (est. 1 min).
Commit: —

### Phase 2 — Adopt

- [ ] macOS files: convert the status-bearing sites from Phase 1.
- [ ] ALSA files: convert the status-bearing sites from Phase 1.
- [ ] Regenerate the four `audio_codegen_cover_rt.*.ncodesum` goldens. Confirm with
      `-ncode` that each delta is confined to audio helpers.
- [ ] Failed-open leak, macOS and ALSA: plan-158-A's audit found that neither backend's
      open-failure path frees the handle record (or, on ALSA, the mapped STATE)
      (plan-158-A C6 fixed the Windows twin). Measure with an `openfail N` loop in a
      `--debug` build (N=10 vs N=100, `arena.0.live_bytes`), fix, and re-measure flat.
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

- **C1 (the ALSA count).** The Phase 1 grep counts `gen_alsa_shared.rs:551` once, but
  it is the `check` closure in `emit_configure_hw_params`, called after 9 different
  `snd_pcm_hw_params*` calls
  (`grep -c 'check(ctx.instructions, dev_fail)' src/codegen/builtins/audio/gen_alsa_shared.rs`
  → 9). Each is its own status site with its own call name, so the table has 41 rows,
  not 33, and the acceptance count is corrected to match (a stronger check).

## Summary

Adoption only; the error-path logic (call-site tag-3 re-stamp, OOM arms) is proven in plan-158-A.
