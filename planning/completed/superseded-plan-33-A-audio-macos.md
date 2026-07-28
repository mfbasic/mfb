# plan-33-A: Audio Builtin Surface and macOS Core Audio

Last updated: 2026-07-08
Overall Effort: x-large (1d-3d)
Effort: medium (1h-2h)
Depends on: nothing

This sub-plan introduces the `audio` built-in package surface and lands the first
real backend on macOS. A correct implementation lets a console MFBASIC program
open the default OS audio input or output device as a resource and move raw PCM
frames through it with `audio::read` and `audio::write`, without any audio file
loading, decoding, encoding, or virtual/simulated audio path.

References:

- `.ai/compiler.md` - runtime completion gate, mandatory function tests, runtime
  proof, acceptance command.
- `.ai/specifications.md` - embedded spec sync rules and citation requirements.
- `src/builtins/mod.rs:is_builtin_import` - fixed import-gated builtin package
  registry.
- `src/builtins/fs.rs` and `src/builtins/net.rs` - resource package metadata
  precedents.
- `src/builtins/resource.rs:BUILTIN_RESOURCES` - single source of truth for
  built-in resources and close operations.
- `src/target/shared/runtime/mod.rs:RuntimeHelper` and
  `src/target/shared/runtime/catalog.rs:supported_helper_specs` - runtime helper
  routing and ABI catalog.
- `src/target/shared/code/mod.rs:lower_runtime_helper` - shared runtime helper
  dispatch.
- `src/target/macos_aarch64/plan.rs:runtime_imports` - macOS platform imports.
- `src/os/macos/object.rs:dylib_for_library` and
  `src/os/macos/link/mod.rs:dylib_path` - Mach-O framework/library mapping.
- `src/docs/spec/language/18_builtin-functions.md` and
  `src/docs/spec/stdlib/spec.md` - built-in package orientation and standard
  package reading order.

## 1. Goal

- On `macos-aarch64`, this program shape works against the real default audio
  devices:
  - `IMPORT audio`
  - `LET out AS AudioOut = audio::openOutput(48000, 2, 512)`
  - `audio::write(out, pcmBytes)`
  - `audio::close(out)`
  and an input variant can `audio::read(input, frames)` to obtain raw PCM bytes
  from the default microphone/input device.

### Non-goals (explicit constraints)

- No audio file APIs. This plan does not add loading, saving, container parsing,
  codecs, metadata, sample libraries, or path-based functions.
- No implicit decode/encode or format guessing. The initial public format is raw
  interleaved signed 16-bit little-endian PCM (`s16le`) only.
- No silent fallback device, dummy stream, generated tones, loopback, or default
  byte result. If the OS device cannot be opened or used, the call raises a real
  runtime error.
- No change to existing `io::`, `fs::`, `net::`, `term::`, app-mode console IO,
  collection layout, scalar storage, or resource ABI.
- No app-mode audio special case in this sub-plan. The macOS implementation is
  valid for native console binaries first; app-mode support can reuse the same
  helpers later only after its lifecycle is specified.
- No cross-thread handle transfer in v1. `AudioInput` and `AudioOutput` are not
  sendable resources until callback/ring-buffer ownership is explicitly audited.

## 2. Current State

MFBASIC has no `audio` import-gated package today. The recognized built-in
package set in `src/builtins/mod.rs:is_builtin_import` excludes `audio`, and
`src/docs/spec/language/18_builtin-functions.md` lists the current packages as
`collections`, `csv`, `datetime`, `errorCode`, `fs`, `http`, `io`, `json`,
`math`, `net`, `os`, `regex`, `strings`, `term`, `thread`, `tls`, and `vector`.

Resource packages follow a clear shape. `src/builtins/fs.rs` defines the `File`
resource, `fs.close`, metadata helpers (`is_fs_call`, `resolve_call`,
`call_return_type_name`, `arity`), and a close mapping via
`resource_close_function`. `src/builtins/net.rs` does the same for `Socket`,
`Listener`, and `UdpSocket`, including raw byte operations such as `net.read`
and `net.write`. The cross-package resource registry is centralized in
`src/builtins/resource.rs:BUILTIN_RESOURCES`; resource storage is a pointer-sized
reference in `src/target/shared/plan/lower.rs:storage_for_type`.

Runtime helper plumbing is data-driven but still needs explicit rows. A helper
family is represented in `src/target/shared/runtime/mod.rs:RuntimeHelper`,
each call needs a `RuntimeHelperSpec` in `src/target/shared/runtime/catalog.rs`,
platform imports are selected by `src/target/macos_aarch64/plan.rs:runtime_imports`,
and native helper bodies are selected in `src/target/shared/code/mod.rs`.

macOS already links platform frameworks, but only those registered in both plan
and object/link mappings. `src/os/macos/object.rs:dylib_for_library` and
`src/os/macos/link/mod.rs:dylib_path` know `Network`, `AppKit`, `Foundation`,
`libobjc`, `libSystem`, and generic `/usr/lib/*.dylib` names. Audio frameworks
must be added there if direct framework imports are used.

## 3. Design Overview

Add a new import-gated built-in package, `audio`, with two move-only resources:

- `AudioInput` - owns one OS capture stream and a runtime input ring buffer.
- `AudioOutput` - owns one OS playback stream and a runtime output ring buffer.

The v1 public API is deliberately narrow:

| Function | Signature | Behavior |
| --- | --- | --- |
| `audio::openInput(sampleRate, channels, bufferFrames)` | `(Integer, Integer, Integer) -> AudioInput` | Opens the default input device for interleaved `s16le` PCM. |
| `audio::openOutput(sampleRate, channels, bufferFrames)` | `(Integer, Integer, Integer) -> AudioOutput` | Opens the default output device for interleaved `s16le` PCM. |
| `audio::read(input, frames)` | `(AudioInput, Integer) -> List OF Byte` | Blocks until at least one frame is available or an OS error occurs; returns up to `frames * channels * 2` bytes. |
| `audio::write(output, bytes)` | `(AudioOutput, List OF Byte) -> Nothing` | Blocks until all bytes are queued/played by the OS stream. Byte length must be a whole frame. |
| `audio::close(resource)` | `(AudioInput) -> Nothing`, `(AudioOutput) -> Nothing` | Stops and releases the OS stream. Drop cleanup routes here. |

Correctness risk concentrates in the runtime callback boundary. Core Audio pulls
output and pushes input on OS-managed realtime callbacks; MFBASIC code runs on
normal threads and allocates through arenas. The callback must never allocate in
an arena, never call MFBASIC helpers, never hold a mutex while calling user code,
and must only move bytes through a fixed native ring buffer owned by the audio
resource. All conversion between MFBASIC `List OF Byte` and the native ring
buffer happens outside the Core Audio callback.

Rejected alternatives:

- `Float` samples or platform-native formats first: rejected because different
  OS default stream formats would make the same program observe different byte
  layouts. `s16le` is a stable byte contract.
- A single duplex `AudioDevice` resource first: rejected because many hosts have
  independent input/output devices and Core Audio/ALSA duplex setup has more
  synchronization risk. Separate resources keep v1 smaller.
- File-like `readBytes`/`writeBytes` names: rejected to avoid implying seekable
  EOF/file behavior. Audio streams are live resources.

## 4. Frontend and Resource Design

Add `src/builtins/audio.rs` modeled after `fs.rs` and `net.rs`.

- Constants: `AUDIO_INPUT_TYPE = "AudioInput"`, `AUDIO_OUTPUT_TYPE =
  "AudioOutput"`.
- Calls: `audio.openInput`, `audio.openOutput`, `audio.read`, `audio.write`,
  `audio.close`.
- Metadata: `is_audio_call`, `is_builtin_type`, `resource_close_function`,
  `call_param_names`, `call_return_type_name`, `resolve_call`,
  `expected_arguments`, `arity`.
- Builtin registration: update `src/builtins/mod.rs` to include `audio` in
  `mod`, `is_builtin_import`, `is_builtin_type`, and any metadata dispatch that
  enumerates packages.
- Resource registry: add both resources to
  `src/builtins/resource.rs:BUILTIN_RESOURCES`, with `sendable: false` and
  `close_may_fail: true`.

Input validation is part of the runtime helper, not just frontend arity:

- `sampleRate` must be positive and in a supported practical range, initially
  `8000..=192000`.
- `channels` must be `1` or `2` in v1.
- `bufferFrames` must be positive and bounded, initially `64..=8192`.
- `read` frame count must be positive and bounded so the returned byte list size
  cannot overflow.
- `write` byte length must be nonzero and divisible by `channels * 2`.

## 5. Runtime Helper and macOS Design

Add a new helper family in `src/target/shared/runtime/mod.rs`:
`RuntimeHelper::Audio`, and a new `audio_specs.rs` catalog file with fixed ABI
rows for the five public calls. `helper_for_call` routes `builtins::audio`
calls to `RuntimeHelper::Audio`.

Add shared lowering module `src/target/shared/code/audio.rs` with helper entry
points selected by `src/target/shared/code/mod.rs:lower_runtime_helper`.

Resource layout is an internal native record allocated with `_mfb_arena_alloc`
for the owning MFBASIC handle plus C-heap/native storage for callback-safe
buffers:

```
AudioHandle {
  u64 kind              // 1=input, 2=output
  u64 closed
  u64 sampleRate
  u64 channels
  u64 bytesPerFrame     // channels * 2
  u64 bufferFrames
  void* osObject        // AudioUnit/AudioComponentInstance or backend object
  Ring* ring            // native malloc/calloc storage, not arena memory
  u64 lastError
}
```

The macOS backend uses Core Audio through AudioUnit HAL/default-device APIs:

- Resolve/open the default input or output component.
- Configure client stream format to interleaved signed 16-bit little-endian PCM
  at the requested sample rate and channels.
- Install an input callback that writes captured frames into the native ring.
- Install an output callback that reads queued frames from the native ring and
  writes silence only for underrun after reporting/recording an underrun state
  visible to the next `audio::write` or `audio::close`.
- Start the AudioUnit after successful configuration; stop/uninitialize/dispose
  it in `audio::close`.

The generated helper must call only OS functions and internal runtime utilities.
Core Audio callback state cannot point into a MFBASIC arena except for immutable
metadata copied into native storage. If any OS call fails, translate the OSStatus
into a runtime error using existing error machinery; if no existing error code is
specific enough, add a new diagnostics registry entry in the same implementation
change.

Direct framework imports require adding `AudioToolbox` and `CoreAudio` to:

- `src/target/macos_aarch64/plan.rs:runtime_imports`
- `src/os/macos/object.rs:dylib_for_library`
- `src/os/macos/link/mod.rs:dylib_path`

If the implementation chooses `dlopen`/`dlsym` instead, this phase must still
add deterministic missing-framework error handling and tests that prove an
unresolved symbol path raises, not crashes.

## Compatibility / Format Impact

Externally observable additions:

- New import-gated built-in package: `audio`.
- New resource types: `AudioInput`, `AudioOutput`.
- New raw PCM contract: interleaved `s16le`; one frame is `channels * 2` bytes.
- New runtime helper family and symbols of the form `_mfb_rt_audio_audio_*`.

Unchanged:

- Existing package names, resource ABI, `List OF Byte` layout, and native calling
  convention.
- Existing `io` standard streams and `fs` file semantics.
- Existing object/binary formats except for additional platform imports when an
  audio helper is used.

## Phases

### Phase 1 - Frontend package and metadata

This lands the source-visible `audio` package shape without emitting helper
bodies yet.

- [ ] Add `src/builtins/audio.rs` with metadata and unit tests covering every
      function and both resource types.
- [ ] Register `audio` in `src/builtins/mod.rs`.
- [ ] Register `AudioInput` and `AudioOutput` in
      `src/builtins/resource.rs:BUILTIN_RESOURCES` as non-sendable resources.
- [ ] Add runtime helper enum/spec plumbing in
      `src/target/shared/runtime/mod.rs`, `catalog.rs`, and a new
      `audio_specs.rs`.
- [ ] Tests: add syntax invalid coverage under `tests/syntax/audio/` for wrong
      arity, wrong argument types, and use without `IMPORT audio`.

Acceptance: `cargo test --bin mfb builtins::audio target::shared::runtime` passes,
and invalid audio package calls produce diagnostics in the new syntax tests.
Commit: -

### Phase 2 - macOS helper imports and object/link mapping

This makes macOS plans able to name the real OS audio APIs.

- [ ] Update `src/target/macos_aarch64/plan.rs:runtime_imports` for the audio
      helper symbols.
- [ ] Update `src/os/macos/object.rs:dylib_for_library` and
      `src/os/macos/link/mod.rs:dylib_path` for any direct Core Audio framework
      libraries used.
- [ ] Add native-plan/object-plan tests that prove an `audio::openOutput`
      program imports only the audio libraries it actually needs.

Acceptance: a tiny `IMPORT audio` program that opens and closes output produces
native plan/object plan JSON with the expected audio runtime symbol and platform
imports, and a program without `IMPORT audio` has no audio imports.
Commit: -

### Phase 3 - Core Audio runtime helpers

This is the first end-to-end functional backend.

- [ ] Add `src/target/shared/code/audio.rs` with `lower_audio_helper` and
      macOS-specific emit routines for open/read/write/close.
- [ ] Wire `lower_runtime_helper` in `src/target/shared/code/mod.rs` to dispatch
      `builtins::audio` calls to the audio lowering.
- [ ] Implement callback-safe native ring buffers for input and output using
      native allocation and OS-safe synchronization.
- [ ] Implement error translation for invalid parameters, OS open/configure
      failures, underrun/overrun, write-after-close, and read/write using the
      wrong resource kind.
- [ ] Tests: add runtime valid tests under
      `tests/rt-behavior/audio/func_audio_openOutput_valid`,
      `func_audio_close_valid`, and parameter invalid/runtime-error tests under
      `tests/syntax/audio/` or `tests/rt-error/audio/` as appropriate.

Acceptance: on macOS with an available default output device, a generated native
program opens output, writes a short known PCM buffer, closes successfully, and
exits 0; invalid byte/frame arguments raise the documented errors rather than
crashing or silently succeeding.
Commit: -

## Validation Plan

- Tests: unit tests for metadata/spec parity; syntax invalid tests for each
  overload; runtime valid/error tests for open, close, read/write parameter
  validation, and wrong-resource-kind handling.
- Runtime proof: run a macOS native program that writes 100 ms of `s16le` silence
  and a 440 Hz tone buffer to the default output, then closes; run an input smoke
  program that opens input and reads at least one frame when a default input
  device is present.
- Doc sync: update `src/docs/spec/language/18_builtin-functions.md`,
  `src/docs/spec/stdlib/spec.md`, add `src/docs/spec/stdlib/11_audio.md`, and add
  `src/docs/man/builtins/audio/**` pages following the man templates.
- Acceptance: `scripts/test-accept.sh target/debug/mfb target/accept-actual`.

## Open Decisions

- Error code - recommended: add `ErrAudioUnavailable` and `ErrAudioDevice`
  if existing `ErrUnsupported`/`ErrInvalidArgument`/`ErrIo` coverage is not
  precise enough; alternative: reuse broad IO errors, which is less useful for
  device failures.
- Callback import strategy - recommended: direct framework imports for
  AudioToolbox/CoreAudio so object plans stay transparent; alternative:
  `dlopen`/`dlsym`, which avoids linker table edits but hides symbol dependency
  detail in runtime code.
- Input runtime test - recommended: make it a smoke test gated on default input
  availability and report a clear skip/blocker in CI without audio hardware;
  alternative: require virtual audio devices, which is stronger but adds CI
  infrastructure.

## Summary

The real risk in plan-33-A is the Core Audio callback boundary and resource
lifetime. The public API stays intentionally small: two resources, raw `s16le`
frames, blocking read/write, and close. Linux/ALSA and broad cross-target
validation are left to the dependent sub-plans.
