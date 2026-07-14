# plan-33-B: ALSA Backend for Audio Builtins

Last updated: 2026-07-08
Effort: medium (1h-2h)
Depends on: plan-33-A

This sub-plan adds the Linux implementation for the `audio` package introduced
by plan-33-A. A correct implementation lets `linux-aarch64`, `linux-x86_64`,
and `linux-riscv64` native binaries open the default ALSA PCM capture/playback
device and move raw interleaved signed 16-bit little-endian PCM through the same
`audio::read` and `audio::write` API as macOS.

References:

- `planning/plan-33-A-audio-macos.md` - public API, resource layout, validation
  rules, and shared runtime helper family.
- `.ai/compiler.md` - runtime proof and acceptance requirements.
- `src/target/linux_x86_64/plan.rs:runtime_imports`,
  `src/target/linux_aarch64/plan.rs:runtime_imports`, and
  `src/target/linux_riscv64/plan.rs:runtime_imports` - Linux platform imports.
- `src/os/linux/object.rs` and `src/os/linux/link/elf.rs` - dynamic library
  import and needed-library emission.
- `src/target/shared/code/net/mod.rs` and `src/target/shared/code/tls/openssl.rs`
  - precedents for Linux helper code that drives C libraries from runtime
  helpers.

## 1. Goal

- On each supported Linux native target, an `IMPORT audio` program can open
  `audio::openOutput(48000, 2, 512)`, write a whole-frame `List OF Byte`, close
  successfully, and link against ALSA only when an audio helper is used.
- `audio::openInput(...); audio::read(input, frames)` returns real bytes from
  ALSA capture on hosts with a default input PCM device.

### Non-goals (explicit constraints)

- No PulseAudio, PipeWire, JACK, OSS, or platform abstraction layer in v1. ALSA
  is the Linux backend for this plan.
- No virtual/test audio backend and no silent success when `libasound` or the
  default device is unavailable.
- No change to the public `audio` API defined in plan-33-A.
- No Linux-only sample formats, channel layouts, or partial-frame writes.
- No ALSA dependency for programs that do not call `audio::` helpers.

## 2. Current State

Linux runtime imports are selected per target in
`src/target/linux_x86_64/plan.rs`, `src/target/linux_aarch64/plan.rs`, and
`src/target/linux_riscv64/plan.rs`. The existing pattern imports libc and pthread
symbols directly and pulls in other runtime dependencies only when their helper
is present. Linux object/link code already supports additional needed libraries
through `PlatformImport.library` in `src/os/linux/object.rs` and
`src/os/linux/link/elf.rs`.

No ALSA symbols are imported today, and no `audio` helper family exists before
plan-33-A. Existing network and TLS helper code demonstrates the two viable
approaches: direct libc-style imports for known symbols, or `dlopen`/`dlsym` for
optional external libraries.

## 3. Design Overview

Implement the same resource contract as plan-33-A using ALSA PCM handles:

- `AudioOutput` wraps an `snd_pcm_t*` opened with `SND_PCM_STREAM_PLAYBACK`.
- `AudioInput` wraps an `snd_pcm_t*` opened with `SND_PCM_STREAM_CAPTURE`.
- Both are configured for `SND_PCM_ACCESS_RW_INTERLEAVED`,
  `SND_PCM_FORMAT_S16_LE`, requested channels, requested sample rate, and the
  requested period/buffer size derived from `bufferFrames`.

Linux does not need the Core Audio callback/ring-buffer split for v1. ALSA's
blocking `snd_pcm_readi` and `snd_pcm_writei` APIs can be called from the
runtime helper thread directly:

- `audio::read` allocates the result `List OF Byte`, calls `snd_pcm_readi` into
  its data buffer, handles recoverable `EPIPE`/`ESTRPIPE` with
  `snd_pcm_recover`, and returns the actual frame count read.
- `audio::write` validates whole-frame length and loops on `snd_pcm_writei`
  until every frame is accepted or a nonrecoverable ALSA error occurs.
- `audio::close` drains playback handles when appropriate, drops capture handles,
  closes `snd_pcm_t*`, marks the resource closed, and frees native state.

Correctness risk concentrates in recovery paths and dynamic linking. ALSA can
return short writes/reads, underrun/overrun, suspend, and configuration rounding.
Every one of those paths must either recover transparently or raise a documented
runtime error.

Rejected alternatives:

- Use raw `/dev/snd/pcm*` device files: rejected because ALSA's userspace API
  owns device selection, plugins, software conversion policy, and recovery.
- Use nonblocking ALSA first: rejected because the public v1 API is blocking and
  simpler to validate.
- Require exact requested sample rate only: rejected because ALSA may choose the
  nearest supported rate. The implementation may accept ALSA's exact configured
  rate only if it verifies the final rate equals the request; otherwise it raises
  a device error. Silent resampling policy is not part of v1.

## 4. Linux Runtime Imports

Recommended implementation: direct imports from `libasound.so.2` for the ALSA
symbols needed by the helper, plus libc errno/error support already used by
other helpers.

Expected ALSA symbol set:

- `snd_pcm_open`
- `snd_pcm_close`
- `snd_pcm_hw_params_malloc`
- `snd_pcm_hw_params_any`
- `snd_pcm_hw_params_set_access`
- `snd_pcm_hw_params_set_format`
- `snd_pcm_hw_params_set_channels`
- `snd_pcm_hw_params_set_rate_near`
- `snd_pcm_hw_params_set_period_size_near`
- `snd_pcm_hw_params`
- `snd_pcm_hw_params_free`
- `snd_pcm_prepare`
- `snd_pcm_readi`
- `snd_pcm_writei`
- `snd_pcm_drain`
- `snd_pcm_drop`
- `snd_pcm_recover`
- `snd_strerror`

Add these imports only for `audio::` helper specs in:

- `src/target/linux_x86_64/plan.rs:runtime_imports`
- `src/target/linux_aarch64/plan.rs:runtime_imports`
- `src/target/linux_riscv64/plan.rs:runtime_imports`

If direct imports make `libasound.so.2` absence a load-time failure, that is an
acceptable v1 contract only if documented. If runtime-friendly missing-library
errors are required, use `dlopen`/`dlsym` and keep all ALSA symbol resolution in
the helper with a precise `ErrAudioUnavailable` failure.

## 5. ALSA Helper Design

Extend `src/target/shared/code/audio.rs` with Linux emit routines selected by
`CodegenPlatform::target()` or by platform-specific helper methods.

For `openOutput` / `openInput`:

- Validate frontend-independent arguments.
- Allocate the `AudioHandle` resource record.
- Open ALSA PCM device `"default"` for playback/capture.
- Configure interleaved `s16le`, channels, rate, and period size.
- Verify ALSA returned the requested rate and a usable period/buffer size.
- Prepare the device and return the resource handle.

For `write`:

- Reject closed handles and `AudioInput` handles.
- Reject empty byte lists and lengths not divisible by `bytesPerFrame`.
- Loop until all frames have been written.
- On `-EPIPE` or recoverable suspend, call `snd_pcm_recover` and continue.
- On nonrecoverable errors, raise the audio device error with `snd_strerror`
  text included in the message.

For `read`:

- Reject closed handles and `AudioOutput` handles.
- Allocate a `List OF Byte` sized to `frames * bytesPerFrame`.
- Call `snd_pcm_readi`; recover from overrun/suspend when ALSA reports it.
- Set the list length to actual bytes read if ALSA returns fewer frames than
  requested.

For `close`:

- Mark closed before calling OS close routines so cleanup is idempotent across
  explicit close and drop cleanup.
- Use `snd_pcm_drain` for output and `snd_pcm_drop` for input.
- Always call `snd_pcm_close` once for a successfully opened handle.

## Compatibility / Format Impact

Externally observable additions are exactly those from plan-33-A, now available
on Linux native targets. Linux binaries that use `audio` gain a dynamic
dependency on `libasound.so.2` if direct imports are used. Binaries that do not
use `audio` must not gain that dependency.

## Phases

### Phase 1 - Linux import planning

This makes Linux native plans describe ALSA dependencies correctly.

- [ ] Add ALSA runtime imports for audio helper specs in all three Linux plan
      files.
- [ ] Add plan/object/link tests proving `libasound.so.2` appears only for
      programs that call `audio::` helpers.
- [ ] Confirm no Linux app-mode imports are affected.

Acceptance: native plan JSON for an audio program on each Linux target contains
the expected ALSA imports, and a non-audio program contains none.
Commit: -

### Phase 2 - ALSA open/close helpers

This lands real resource acquisition and cleanup before read/write.

- [ ] Implement ALSA open/configure/prepare for `openInput` and `openOutput` in
      `src/target/shared/code/audio.rs`.
- [ ] Implement `audio::close` for ALSA handles.
- [ ] Tests: add runtime open/close smoke tests for Linux hosts with ALSA and
      runtime error tests for invalid rate/channel/buffer arguments.

Acceptance: on a Linux host with `libasound.so.2` and a default PCM device,
generated native programs open and close input/output handles without leaks or
crashes; invalid parameter programs raise documented errors.
Commit: -

### Phase 3 - ALSA read/write helpers

This completes Linux functionality.

- [ ] Implement blocking `audio::write` with short-write and recoverable-error
      handling.
- [ ] Implement blocking `audio::read` with actual byte-count adjustment and
      recoverable overrun handling.
- [ ] Tests: add runtime valid tests for writing silence, whole-frame validation,
      wrong-resource-kind errors, read smoke, and write-after-close/read-after-
      close errors.

Acceptance: Linux generated programs can write a short `s16le` buffer to the
default playback device and read real capture bytes from the default capture
device when present; all invalid cases raise documented errors.
Commit: -

## Validation Plan

- Tests: ALSA import tests for each Linux target; runtime valid/error tests under
  `tests/rt-behavior/audio/` and `tests/rt-error/audio/`; syntax invalid tests
  shared with plan-33-A.
- Runtime proof: run generated binaries on at least one glibc Linux target with
  `libasound.so.2` and a default PCM device; if CI lacks hardware, record that as
  a documented external validation blocker and run the import/error tests in CI.
- Doc sync: update the `audio` spec/man pages from plan-33-A with Linux ALSA
  availability and `libasound.so.2` dependency notes.
- Acceptance: `scripts/test-accept.sh target/debug/mfb target/accept-actual`.

## Open Decisions

- Linking strategy - recommended: direct `libasound.so.2` imports for transparent
  dependency plans; alternative: `dlopen`/`dlsym` for cleaner missing-library
  runtime errors.
- Hardware CI - recommended: keep deterministic compiler/import/error tests in
  normal CI and document real-device runtime proof as host-gated; alternative:
  provision virtual ALSA loopback devices in remote test machines.
- Rate negotiation - recommended: require ALSA's final configured rate to equal
  the requested rate in v1; alternative: expose actual rate later through a
  query function.

## Summary

This sub-plan keeps Linux behavior aligned with the macOS surface while using
ALSA's blocking PCM API instead of a callback ring. The main risks are
recoverable ALSA errors, dynamic dependency handling, and verifying real audio
hardware behavior without introducing a fake backend.
