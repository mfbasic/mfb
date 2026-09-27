# plan-158-B: macOS and ALSA audio adopt status messages

Last updated: 2026-09-26
Effort: medium (1h–2h)
Depends on: plan-158-A

Extends plan-158-A's `StatusFailSites` to the two remaining `audio::` backends, so a
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
  `StatusFailSites` (plan-158-A deleted the single-site `emit_fail_status`: no site needed it).

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
Commit: ca414e4c3

### Phase 2 — Adopt

- [x] macOS files: convert the status-bearing sites from Phase 1. — 13 `ret` rows via
      `StatusFailSites` (slots 488/496, devices 1008/1016). The refused-enqueue tail in
      `write` also returns the buffer to the free stack (C4).
- [x] ALSA files: convert the status-bearing sites from Phase 1. — 13 `ret` + 2 `value`
      rows (slots 536/544).
- [x] Regenerate the four `audio_codegen_cover_rt.*.ncodesum` goldens. Confirm with
      `-ncode` that each delta is confined to audio helpers. — `/tmp/p158/cmp_targets.py`
      against a baseline built at `54e7a1efa` (each baseline reproduces its golden sha):
      macOS changes `_mfb_rt_audio_audio_{devices,openInput,openInputDevice,openOutput,
      openOutputDevice,write}`; Linux (3 targets) changes `…_{devices,openInput,
      openInputDevice,openOutput,openOutputDevice,read,readTimeout,write}`; each adds
      `_mfb_rt_fail_status` and `_mfb_str_errstatus_*` data only; windows-x86_64 is
      unchanged. `bash scripts/regen-native-goldens.sh target/release/mfb
      tests/byte-identity/audio` → `5 golden(s) rewritten, 0 failure(s)`, four changed.
- [x] Emit-inspection tests pinning each backend's status call names and its static
      raises (C7): `macos_audio_failures_report_their_os_status`,
      `alsa_audio_failures_report_their_os_status` in `audio/status_tests.rs`
      (`cargo test --release --bin mfb audio` → 22 passed).
- [x] Failed-open leak, macOS and ALSA: plan-158-A's audit found that neither backend's
      open-failure path frees the handle record (or, on ALSA, the mapped STATE)
      (plan-158-A C6 fixed the Windows twin). Measure with an `openfail N` loop in a
      `--debug` build (N=10 vs N=100, `arena.0.live_bytes`), fix, and re-measure flat.
      — Both `emit_open_cleanup`s free the handle record (ALSA already unmapped its
      STATE). Measured absolute rather than as a slope, which is stronger: the macOS
      probe's 3 failed opens and the Linux probe's 12 leave `arena.0.live_bytes 0`
      (every allocation freed), C5.
- [x] Runtime proof, macOS (local): open an output on a device id that does not exist →
      `e.message` ends with `: 0x…)`. — A device id cannot be constructed (C3), so
      `/tmp/p158/macprobe` opens every input-only device as an output and the speakers
      as an input: `output: … (AudioQueueStart: 0xfffefbb3)` (source
      `src/main.mfb:12:39`), `input: … (AudioQueueStart: 0xfffefbbc)` (source
      `src/main.mfb:22:38`), `live_bytes 0`.
- [x] Runtime proof, Linux: the same probe on box 2223 (native aarch64 glibc), with a
      bogus ALSA device name. — Box 2223 refused connections; ran on 2226 (Debian
      aarch64 glibc, HDA card) instead (C2). `/tmp/p158/linprobe` opens every hint:
      `device hw:CARD=Intel,DEV=0: … (snd_pcm_open: 0xfffffff0)` (-EBUSY, pipewire holds
      the card), `device dsnoop:…: … (snd_pcm_open: 0xffffffea)` (-EINVAL), 12 failures,
      `live_bytes 0`.

Acceptance: both probes print the status suffix.
  Check: `./build/probe.out` → a line matching `\(.*: 0x[0-9a-f]{8}\)` (est. 2 min per box).
  — met on both (above).
Commit: d382a92a3 (adoption, fixes, tests), 5ed78e791 (spec), 3e3547727 (goldens + ledger)

## Validation Plan

- Doc sync: `src/docs/spec/stdlib/11_audio.md` backend sections name the macOS/ALSA calls.
  — done: §"The OS status in the message" per backend; the `audio` man description
  gives an example per platform (`mfb man audio` renders it; `man-census.sh
  --memory-scope` → 0 unclassified; `spec-census.sh --citations` → 0 misses).
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
- **C2 (Linux box).** `ssh -p 2223 test@127.0.0.1` → `Connection refused` (2228 too).
  Box 2226 is the same class, native aarch64 glibc, and has `libasound.so.2` and an HDA
  card (`aplay -l`), so the Linux proof ran there.
- **C3 (no bogus device id).** `audio::AudioDevice[...]` is rejected with
  `TYPE_READ_ONLY_RECORD_CONSTRUCTOR`: a program cannot name a device that
  `audio::devices()` did not return. The macOS proof instead points a queue at a real
  device for the wrong direction, which Core Audio refuses at `AudioQueueStart`; the
  Linux proof opens real hints that are busy or wrong-direction.
- **C4 (refused enqueue dropped a buffer).** `lower_write` (macOS) lost the buffer in
  hand when `AudioQueueEnqueueBuffer` failed: not enqueued, not pending, not free. Each
  refusal shrank the free stack by one, and with it empty the next write would wait on
  the condition forever. The status tail now returns it to the free stack under the
  mutex and clears the pending slot (the refused buffer may be the resumed one).
- **C5 (failed-open record leak).** plan-158-A's audit: macOS and ALSA
  `emit_open_cleanup` never freed the handle record. Both now do; every caller of each
  is an open failure tail after the record's allocation. macOS's cleanup labels are
  now suffixed by position, since an open now runs it from two tails (the first build
  failed with a duplicate `…_cleanup_munmap`).
- **C6 (no single-site form).** Every site in all three backends is a
  `StatusFailSites` site, so plan-158-A's `emit_fail_status` had no caller and was
  deleted (no dead code), and this plan's text now names `StatusFailSites`.
- **C7 (guard against an unemitted site).** The first Linux build failed:
  `runtime.audio.devices branches to label '…_status_unavailable_s0', which it does not
  define`. The ALSA `devices` edit registered a site but never emitted the stubs; no
  unit test lowered that helper, and macOS builds never do. The guard is the new
  per-backend emit-inspection tests: they lower every audio helper and read each
  status call name from its prefix relocation, which only an emitted stub creates,
  so a site whose stub is missing drops out of the expected set. (A `Drop` guard on
  `StatusFailSites` was tried and removed in §5: a body that returns `Err` part-way —
  an emit failure — drops its sites unemitted legitimately, and the guard turned that
  error into a panic, failing `an_emit_failure_past_the_first_one_still_propagates`.)

## Summary

Adoption only; the error-path logic (call-site tag-3 re-stamp, OOM arms) is proven in plan-158-A.
