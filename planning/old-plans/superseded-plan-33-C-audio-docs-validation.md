# plan-33-C: Audio Docs, Spec, and Cross-Target Validation

Last updated: 2026-07-08
Effort: medium (1h-2h)
Depends on: plan-33-A, plan-33-B

This sub-plan finishes the `audio` feature by making the public contract
auditable in the embedded spec and man pages, then proving the macOS Core Audio
and Linux ALSA implementations behave consistently for raw PCM input/output.

References:

- `planning/plan-33-A-audio-macos.md` - public API and macOS backend.
- `planning/plan-33-B-audio-alsa.md` - Linux ALSA backend.
- `.ai/man_template.md`, `.ai/man_type_template.md`,
  `.ai/man_package_template.md` - required man-page structure.
- `.ai/specifications.md` - embedded spec requirements.
- `scripts/update_man.sh` and `scripts/update_man_package.sh` - man page driver
  scripts and authoring rules.
- `src/docs/spec/stdlib/spec.md` and
  `src/docs/spec/language/18_builtin-functions.md` - spec integration points.
- `src/docs/man/builtins/fs/**` and `src/docs/man/builtins/net/**` - package and
  resource-function documentation precedents.

## 1. Goal

- `./mfb man audio` and `./mfb spec stdlib audio` document the exact `audio`
  package contract: resources, raw `s16le` frame layout, blocking behavior,
  device selection, errors, platform backends, and non-goals.
- The full acceptance suite passes after macOS and Linux audio support lands.
- Runtime proofs demonstrate real OS audio input/output on macOS and Linux hosts
  with default devices.

### Non-goals (explicit constraints)

- No new API beyond plan-33-A/33-B while documenting. Do not add convenience
  codecs, file helpers, mixers, volume controls, device enumeration, or format
  negotiation in this finishing pass.
- No undocumented test skips. Hardware-gated runtime proofs must report a clear
  device/library availability reason.
- No stale or duplicate documentation. The per-function API belongs in `mfb man
  audio`; behavioral model details belong in the `stdlib audio` spec topic with
  links instead of repeated full text.

## 2. Current State

The embedded standard-library spec currently has topics for regex, datetime,
csv, json, http, url, math-rng, encoding, vector, and crypto under
`src/docs/spec/stdlib/**`; there is no audio topic. The built-in package
orientation in `src/docs/spec/language/18_builtin-functions.md` does not list
`audio`.

Man pages live under `src/docs/man/builtins/<package>/`. Existing resource
packages such as `fs` and `net` provide a package page, per-function pages, and
type/resource descriptions. The AGENTS instructions require the exact templates
and driver scripts for new man pages.

Function test coverage is split by behavior in the current tree: valid runtime
tests for resource packages live under `tests/rt-behavior/<package>/`, invalid
frontend tests live under `tests/syntax/<package>/`, and runtime error tests live
under `tests/rt-error/<package>/`. `.ai/compiler.md` additionally requires
function valid and invalid coverage for every created or modified function.

## 3. Design Overview

Document `audio` as a live-device raw PCM package:

- Package: `audio`.
- Resources: `AudioInput`, `AudioOutput`, both move-only and non-sendable in v1.
- Format: interleaved signed 16-bit little-endian PCM, exactly `channels * 2`
  bytes per frame.
- Device selection: default OS input/output device only.
- Open parameters: `sampleRate`, `channels`, `bufferFrames`.
- Read/write semantics: blocking, whole-frame, no EOF, no file paths.
- Platform backend: Core Audio on macOS, ALSA on Linux.
- Error model: invalid parameters, no device/library, OS configuration failure,
  closed handle, wrong handle kind, underrun/overrun or nonrecoverable stream
  failure.

Validation has two layers:

1. Deterministic compiler/runtime tests that can run without relying on audible
   output, such as metadata, invalid calls, import planning, parameter
   validation, closed-handle errors, and writing silence when a device exists.
2. Hardware-gated runtime proofs that use real default devices for output and
   input and record exact host preconditions.

Correctness risk concentrates in claiming runtime support when CI only proved
compiler plumbing. The completion gate is real-device execution on both backend
families, or a plainly recorded blocker for unavailable hardware/library in a
specific environment.

## 4. Documentation Design

Add man pages under `src/docs/man/builtins/audio/`:

- `package.md` - package overview and platform availability.
- `types.md` - `AudioInput`, `AudioOutput`, raw PCM frame layout.
- `openInput.md`
- `openOutput.md`
- `read.md`
- `write.md`
- `close.md`

Follow the templates exactly:

- `.ai/man_package_template.md` for `package.md`.
- `.ai/man_type_template.md` for `types.md`.
- `.ai/man_template.md` for each function page.

Add or update spec files:

- `src/docs/spec/stdlib/11_audio.md` - behavioral model for live audio streams.
- `src/docs/spec/stdlib/spec.md` - reading-order bullet and see-also entry.
- `src/docs/spec/language/18_builtin-functions.md` - add `audio` to the fixed
  import-gated package set and the orientation list.

Spec claims that cite implementation details must use invisible `[[path:Symbol]]`
citations per `.ai/specifications.md`, after grep-confirming the symbol exists.

## 5. Validation Design

Required deterministic tests:

- `tests/syntax/audio/func_audio_openInput_invalid`
- `tests/syntax/audio/func_audio_openOutput_invalid`
- `tests/syntax/audio/func_audio_read_invalid`
- `tests/syntax/audio/func_audio_write_invalid`
- `tests/syntax/audio/func_audio_close_invalid`
- `tests/rt-error/audio/func_audio_openInput_invalid_runtime`
- `tests/rt-error/audio/func_audio_openOutput_invalid_runtime`
- `tests/rt-error/audio/func_audio_read_invalid_runtime`
- `tests/rt-error/audio/func_audio_write_invalid_runtime`
- `tests/rt-error/audio/func_audio_close_invalid_runtime`
- `tests/rt-behavior/audio/func_audio_openOutput_valid`
- `tests/rt-behavior/audio/func_audio_write_valid`
- `tests/rt-behavior/audio/func_audio_close_valid`

Hardware-gated tests/proofs:

- macOS output: write silence and a short tone buffer through Core Audio.
- macOS input: open input and read at least one frame from a default input
  device.
- Linux output: write silence and a short tone buffer through ALSA.
- Linux input: open capture and read at least one frame from a default capture
  PCM device.

The runtime proof commands should record:

- target triple/flavor,
- whether default input/output devices were detected,
- whether `libasound.so.2` was present on Linux,
- program exit status and stderr/stdout,
- for input, the returned byte length and whole-frame alignment.

## Compatibility / Format Impact

No new compatibility impact beyond plan-33-A and plan-33-B. This sub-plan only
documents and verifies the public contract already introduced there.

## Phases

### Phase 1 - Man pages

This lands the user-facing API reference after the functions exist.

- [ ] Read the three man templates and the relevant driver scripts.
- [ ] Add `src/docs/man/builtins/audio/package.md`,
      `types.md`, and one page per public function.
- [ ] Run the man update/check commands required by the driver scripts.

Acceptance: `./mfb man audio`, `./mfb man audio openInput`, and every other
audio page render without placeholder text, missing required sections, or broken
links.
Commit: -

### Phase 2 - Embedded spec

This records the behavioral contract in the version-locked spec.

- [ ] Add `src/docs/spec/stdlib/11_audio.md` with cited implementation claims.
- [ ] Update `src/docs/spec/stdlib/spec.md` reading order and see-also list.
- [ ] Update `src/docs/spec/language/18_builtin-functions.md` package set and
      orientation list.
- [ ] Run `cargo test --bin mfb spec` and render `./mfb spec stdlib --all` to
      verify no leaked `[[` citations or broken links.

Acceptance: `./mfb spec stdlib audio` documents raw PCM/device semantics and
`cargo test --bin mfb spec` passes.
Commit: -

### Phase 3 - Cross-target validation

This proves the feature is real, not just documented.

- [ ] Run audio metadata/unit tests.
- [ ] Run all syntax, runtime behavior, and runtime error audio tests.
- [ ] Run macOS Core Audio output and input proofs on a host with default
      devices.
- [ ] Run Linux ALSA output and input proofs on a host with `libasound.so.2` and
      default PCM devices.
- [ ] Run `scripts/test-accept.sh target/debug/mfb target/accept-actual`.

Acceptance: deterministic tests and acceptance pass; real-device proofs pass on
macOS and Linux, or each unavailable proof has a concrete external blocker such
as "no default capture device" or "libasound.so.2 missing" and the feature is not
declared fully verified for that platform until rerun.
Commit: -

## Validation Plan

- Tests: every public `audio::` function has valid and invalid coverage; runtime
  error tests cover parameter validation, closed handles, wrong resource kind,
  and unavailable device/library where deterministic.
- Runtime proof: execute generated native programs that read/write real OS audio
  on macOS and Linux. Compiler output or native plan goldens are not sufficient.
- Doc sync: `mfb man audio` and `mfb spec stdlib audio` must match the landed API
  exactly.
- Acceptance: `cargo build`, `cargo test --bin mfb spec`, and
  `scripts/test-accept.sh target/debug/mfb target/accept-actual`.

## Open Decisions

- Audibility assertion - recommended: runtime tests verify successful OS writes
  and byte/frame invariants, while manual/hardware proof can use a short tone;
  alternative: add external loopback capture, which is stronger but requires CI
  device setup.
- Skip representation - recommended: hardware-gated proofs are separate from
  deterministic acceptance tests and documented with exact blockers; alternative:
  encode them as skipped tests, which risks hiding missing validation.

## Summary

This finishing sub-plan keeps documentation and verification at the same quality
bar as the implementation. The main risk is overstating support on hosts without
real audio devices; the validation language requires real-device proof before
calling a platform done.
