# plan-33-B: macOS Backend — AudioQueue and Core Audio Device Enumeration

Last updated: 2026-07-12
Effort: large (4h-8h)
Depends on: plan-33-A

This sub-plan lands the first real backend. On `macos-aarch64`, `audio::devices`
enumerates the machine's audio hardware, and `openInput`/`openOutput` return a
live `AudioInput` / `AudioOutput` that moves raw interleaved `s16le` PCM through
the OS.

References:

- `planning/plan-33-A-audio-surface.md` — public API, `AudioHandle`/`AudioState`
  layout, parameter validation, the binding concurrency contract in §6, and the
  error-code registry in §7 (`ErrAudioUnavailable = 7-705-0017`,
  `ErrAudioDevice = 7-705-0018`; §3.5 violations reuse `ErrInvalidArgument`).
  This backend adds the matching `ERR_AUDIO_*` triples to
  `src/target/shared/code/error_constants.rs` per plan-33-A §7 and raises with
  them.
- `src/target/shared/code/runtime_helpers_thread.rs:99` — the `pthread_mutex_*`
  / `pthread_cond_*` calling precedent this backend reuses.
- `src/target/shared/code/runtime_helpers.rs:545` — `pthread_create` (symbol
  `_pthread_create` on `macos-aarch64`) with an emitted C-ABI trampoline; the
  precedent for handing the OS a function pointer into generated code.
- `src/target/shared/code/entry_and_arena.rs:1139` — the platform `mmap` hook,
  used here for `AudioState` instead of a `malloc` this runtime does not import.
- `src/target/macos_aarch64/plan.rs:runtime_imports` — per-spec platform imports.
- `src/os/macos/object.rs:dylib_for_library` and
  `src/os/macos/link/mod.rs:dylib_path` — the two closed framework tables that
  must stay in sync.
- `src/target/shared/code/mod.rs:lower_runtime_helper` — where the audio bodies
  get dispatched.

## 1. Goal

On a Mac with a default output device, this runs, plays 200 ms of a 440 Hz tone,
and exits 0:

```basic
IMPORT audio
LET out AS AudioOutput = audio::openOutput(48000, 2, 512)
audio::write(out, tone)
audio::close(out)
```

With a default input device, `audio::openInput(48000, 1, 512)` followed by
`audio::read(in, 4800)` returns exactly 9600 bytes of real microphone PCM.

### Non-goals (explicit constraints)

- No `AudioUnit` / `AURemoteIO` / realtime render callback. See §3.1.
- No app-mode (`mfb build -app`) audio in this sub-plan. The helpers are written
  for console binaries; app-mode reuse waits until its lifecycle is specified.
- No virtual/simulated device, no silent no-op when a device is missing.
- No change to the plan-33-A public API.
- No `dlopen`. macOS frameworks are direct imports; see §5.
- No AVFoundation, no CoreMedia, no `AudioComponent`.

## 2. Current State

`macos-aarch64` links a fixed, closed set of libraries: `libSystem`, `Network`,
`AppKit`, `Foundation`, `libobjc`, and `libz`. Both
`src/os/macos/object.rs:dylib_for_library` and
`src/os/macos/link/mod.rs:dylib_path` hard-code that list and reject anything
else with `does not know dylib for platform library '<name>'`. Adding a
framework means editing both tables and their unit tests.

Handing the OS a pointer to generated code is precedented: `pthread_create`
receives an emitted trampoline (`runtime_helpers.rs:545`), and console binaries
install `signal()` handlers (`entry_and_arena.rs:118`). Cross-thread
synchronization is exclusively `pthread_mutex_*`/`pthread_cond_*` — there are no
atomic instructions in any backend, which is the constraint plan-33-A §6 makes
binding.

`AudioState` (plan-33-A §5.1) is `mmap`'d through the existing platform hook, so
this backend adds no allocator imports.

## 3. Design Overview

### 3.1 Why AudioQueue

Core Audio offers two ways to move PCM:

- **`AudioUnit` / `AURemoteIO`** — the render callback runs on a realtime
  thread. It must not block, must not take a lock, and must not page-fault. The
  only correct producer/consumer structure is a lock-free ring, which needs
  acquire/release ordering. **This compiler emits no atomic or barrier
  instructions on any of its three architectures.** A plain-load/plain-store ring
  is incorrect on AArch64's weak memory model. This path is therefore closed
  until atomics land as a separate plan.
- **`AudioQueue`** — when created with a `NULL` callback run loop, callbacks are
  delivered on an ordinary AudioQueue-internal thread. Taking a `pthread_mutex`
  there is legal and expected. The buffer-based model also matches
  `audio::write`'s "block until queued" contract directly.

AudioQueue is the only option consistent with plan-33-A §6, and it happens to be
the simpler one. It costs a small amount of latency (buffers are handed to the
queue rather than filled in-place at the deadline), which is acceptable for a
raw-PCM API with a `bufferFrames` parameter that the caller controls.

### 3.2 Output stream

`AudioState.osObject` holds an `AudioQueueRef`. The stream owns
`AUDIO_QUEUE_BUFFERS = 4` `AudioQueueBufferRef`s, each of
`bufferFrames * bytesPerFrame` bytes. `AudioState.ring` is unused for output;
the free-buffer list is a small fixed array in the mapping.

- `openOutput` builds the ASBD (§4.1), calls `AudioQueueNewOutput` with the
  emitted output callback and the `AudioHandle*` as `inUserData`, optionally
  sets the device (§4.3), allocates the four buffers via
  `AudioQueueAllocateBuffer`, marks all four free, and calls `AudioQueueStart`.
- `write` takes the mutex; while bytes remain, it waits on the condvar for a
  free buffer, copies up to one buffer's worth into it, sets
  `mAudioDataByteSize`, marks it in-flight, **releases the mutex**, calls
  `AudioQueueEnqueueBuffer`, and reacquires. It returns when every byte is
  enqueued.
- The output callback (`void (*)(void *inUserData, AudioQueueRef,
  AudioQueueBufferRef)`) takes the mutex, marks that buffer free, and if the
  in-flight count just reached zero while the stream is started and not closing,
  increments `xruns` (a starved queue is an underrun). It signals the condvar
  and returns. It touches no arena and calls no MFBASIC helper.
- `available` returns `freeBuffers * bufferFrames`. `poll` is `available > 0`,
  with the timed form waiting on the condvar.

AudioQueue emits silence on its own when starved; we do not enqueue silence.

### 3.3 Input stream

`AudioState.ring` is a byte ring of `bufferFrames * bytesPerFrame *
AUDIO_QUEUE_BUFFERS` bytes, guarded by the mutex.

- `openInput` builds the ASBD, calls `AudioQueueNewInput` with the emitted input
  callback, optionally sets the device, allocates and immediately enqueues all
  four buffers, then calls `AudioQueueStart`.
- The input callback (`void (*)(void *inUserData, AudioQueueRef,
  AudioQueueBufferRef, const AudioTimeStamp *, UInt32, const
  AudioStreamPacketDescription *)`) takes the mutex, copies
  `mAudioDataByteSize` bytes into the ring, and if the ring lacks space,
  advances `ringTail` to discard the **oldest** whole frames and increments
  `xruns` by one (one overrun *event*, per plan-33-A §3.3). It signals the
  condvar, releases the mutex, re-enqueues the buffer with
  `AudioQueueEnqueueBuffer`, and returns.
- `read(stream, frames)` takes the mutex and waits on `pthread_cond_wait` until
  `ringHead - ringTail >= frames * bytesPerFrame`, then copies out exactly that
  many bytes and advances `ringTail`.
- `read(stream, frames, timeoutMs)` is the same loop over
  `pthread_cond_timedwait_relative_np`, recomputing the remaining timeout after
  each spurious wake. On expiry it returns **whole frames only** — every
  complete frame currently in the ring, possibly none. It never returns a
  partial frame and never returns more than `frames`.
- `available` is `(ringHead - ringTail) / bytesPerFrame`.

`read` allocates its `List OF Byte` result *before* taking the mutex, so no
arena allocation happens under a lock that an OS callback thread contends.

### 3.4 Close, and the one deadlock

`close` is idempotent and drop-cleanup routes to it.

1. Take the mutex. If `closed` is already set, unlock and return.
2. For output, wait on the condvar until the in-flight buffer count is zero
   (this is the drain).
3. Set `closed`. **Release the mutex.**
4. `AudioQueueStop(queue, true)` then `AudioQueueDispose(queue, true)`.
5. `pthread_cond_destroy`, `pthread_mutex_destroy`, `munmap` the state page.

Step 3 is not optional. `AudioQueueDispose(queue, true)` blocks until every
in-flight callback has returned, and those callbacks take the mutex. Calling
Dispose while holding it deadlocks. The callbacks must therefore also re-check
`closed` after acquiring the mutex and return immediately if set, so a callback
racing step 4 does not touch a freed ring. Because `closed` is written under the
mutex and read under the mutex, this needs no atomics.

A drained output stream that is never closed leaks the AudioQueue and one page.
Drop cleanup covers the normal path; a `TRAP` that skips cleanup is the existing
resource-drop question, not an audio-specific one.

## 4. Core Audio Details

Every constant below must be re-verified against the SDK headers
(`AudioToolbox/AudioQueue.h`, `CoreAudio/AudioHardware.h`,
`CoreAudioTypes/CoreAudioBaseTypes.h`) before landing. They are recorded here so
the implementer diffs rather than guesses. Offsets assume arm64 / LP64.

### 4.1 `AudioStreamBasicDescription` (40 bytes)

| Offset | Field | Value |
| --- | --- | --- |
| 0 | `mSampleRate` (f64) | requested `sampleRate` |
| 8 | `mFormatID` (u32) | `kAudioFormatLinearPCM` = `'lpcm'` = `0x6C70636D` |
| 12 | `mFormatFlags` (u32) | `kAudioFormatFlagIsSignedInteger \| kAudioFormatFlagIsPacked` = `0x0C` |
| 16 | `mBytesPerPacket` (u32) | `channels * 2` |
| 20 | `mFramesPerPacket` (u32) | `1` |
| 24 | `mBytesPerFrame` (u32) | `channels * 2` |
| 28 | `mChannelsPerFrame` (u32) | `channels` |
| 32 | `mBitsPerChannel` (u32) | `16` |
| 36 | `mReserved` (u32) | `0` |

Omitting `kAudioFormatFlagIsBigEndian` gives native-endian, which on every
supported Mac is little-endian — exactly the `s16le` contract. Omitting
`kAudioFormatFlagIsNonInterleaved` gives interleaved. AudioQueue performs any
conversion between this client format and the device's format, which is why the
program observes the same bytes regardless of hardware.

### 4.2 Device enumeration (`audio::devices`)

Through `CoreAudio`'s `AudioObjectGetPropertyDataSize` /
`AudioObjectGetPropertyData` on `kAudioObjectSystemObject` (= `1`), with an
`AudioObjectPropertyAddress` of three `u32`s (selector, scope, element):

| Constant | FourCC | Value |
| --- | --- | --- |
| `kAudioObjectPropertyScopeGlobal` | `'glob'` | `0x676C6F62` |
| `kAudioObjectPropertyScopeInput` | `'inpt'` | `0x696E7074` |
| `kAudioObjectPropertyScopeOutput` | `'outp'` | `0x6F757470` |
| `kAudioObjectPropertyElementMain` | — | `0` |
| `kAudioHardwarePropertyDevices` | `'dev#'` | `0x64657623` |
| `kAudioHardwarePropertyDefaultInputDevice` | `'dIn '` | `0x64496E20` |
| `kAudioHardwarePropertyDefaultOutputDevice` | `'dOut'` | `0x644F7574` |
| `kAudioObjectPropertyName` | `'lnam'` | `0x6C6E616D` |
| `kAudioDevicePropertyDeviceUID` | `'uid '` | `0x75696420` |
| `kAudioDevicePropertyStreamConfiguration` | `'slay'` | `0x736C6179` |

Per device (`AudioDeviceID` is a `u32`):

- `name` — `kAudioObjectPropertyName` in global scope yields a `CFStringRef`;
  convert with `CFStringGetCString` (encoding `kCFStringEncodingUTF8` = `0x08000100`)
  into a stack buffer, then build the MFBASIC `String`, then `CFRelease`.
- `id` — `kAudioDevicePropertyDeviceUID`, same treatment.
- `canInput` / `canOutput` — `kAudioDevicePropertyStreamConfiguration` in input
  / output scope yields an `AudioBufferList` (`u32 mNumberBuffers`, then that
  many `AudioBuffer { u32 mNumberChannels; u32 mDataByteSize; void *mData; }`).
  Sum `mNumberChannels`; nonzero means the direction is supported. The list is
  variable-length: size it with `AudioObjectGetPropertyDataSize` first.
- `isDefaultInput` / `isDefaultOutput` — compare the `AudioDeviceID` against the
  two default-device properties on `kAudioObjectSystemObject`.

`CFRelease` every `CFStringRef` obtained. Any nonzero `OSStatus` raises
`ErrAudioDevice` with the status embedded in the message; a zero device count
raises `ErrAudioUnavailable`.

### 4.3 Selecting a device

`AudioQueueSetProperty(queue, kAudioQueueProperty_CurrentDevice = 'aqcd' =
0x61716364, &uidCFString, sizeof(CFStringRef))`, called after
`AudioQueueNew{Input,Output}` and **before** `AudioQueueAllocateBuffer` and
`AudioQueueStart`. Build the `CFStringRef` from the `AudioDevice.id` bytes with
`CFStringCreateWithCString`, and `CFRelease` it after the property is set. A
device that has disappeared makes `AudioQueueSetProperty` return nonzero →
`ErrAudioDevice`.

The default-device overloads skip this call entirely; AudioQueue then follows the
system default, including when the user changes it mid-stream.

### 4.4 `AudioQueueBuffer` field offsets

`mAudioDataBytesCapacity` (u32) at 0, `mAudioData` (void\*) at 8,
`mAudioDataByteSize` (u32) at 16, `mUserData` (void\*) at 24. Verify against
`AudioQueue.h` — the struct continues with packet-description fields this
backend never uses, and the emitted code must not assume a total size.

### 4.5 Microphone permission (TCC)

`audio::openInput` triggers macOS microphone authorization. A console binary has
no bundle and no `NSMicrophoneUsageDescription`, so the grant is attributed to
the *responsible process* — usually Terminal or the IDE. Consequences the
implementation and the runtime proof must both respect:

- Interactively, the first `openInput` prompts once for the parent terminal, and
  subsequent runs inherit that grant.
- Under a launchd/CI context with no responsible UI process, authorization is
  denied. `AudioQueueStart` then returns a nonzero `OSStatus`, or the queue runs
  and delivers **buffers of digital silence**.

The second case is dangerous: a denied capture looks like a working capture full
of zeros. `openInput` must therefore query
`kAudioHardwarePropertyDefaultInputDevice` and fail with `ErrAudioUnavailable`
when there is none, and the runtime proof in plan-33-D must assert the captured
buffer is **not** all-zero before declaring macOS input verified. Silence is not
proof of capture.

## 5. Imports and Linking

Add three frameworks to **both** `src/os/macos/object.rs:dylib_for_library` and
`src/os/macos/link/mod.rs:dylib_path`, and to both files' unit tests:

| Library | Path |
| --- | --- |
| `AudioToolbox` | `/System/Library/Frameworks/AudioToolbox.framework/AudioToolbox` |
| `CoreAudio` | `/System/Library/Frameworks/CoreAudio.framework/CoreAudio` |
| `CoreFoundation` | `/System/Library/Frameworks/CoreFoundation.framework/CoreFoundation` |

Symbols, added to `src/target/macos_aarch64/plan.rs:runtime_imports` **per
audio spec**, so that a program importing `audio` but only calling
`audio::devices` pulls in `CoreAudio` and `CoreFoundation` but not
`AudioToolbox`:

- `AudioToolbox`: `_AudioQueueNewOutput`, `_AudioQueueNewInput`,
  `_AudioQueueAllocateBuffer`, `_AudioQueueEnqueueBuffer`, `_AudioQueueStart`,
  `_AudioQueueStop`, `_AudioQueueFlush`, `_AudioQueueDispose`,
  `_AudioQueueSetProperty`.
- `CoreAudio`: `_AudioObjectGetPropertyData`,
  `_AudioObjectGetPropertyDataSize`, `_AudioObjectHasProperty`.
- `CoreFoundation`: `_CFStringCreateWithCString`, `_CFStringGetCString`,
  `_CFRelease`.
- `libSystem` (several already imported for `thread::`):
  `_pthread_mutex_init`, `_pthread_mutex_lock`, `_pthread_mutex_unlock`,
  `_pthread_mutex_destroy`, `_pthread_cond_init`, `_pthread_cond_signal`,
  `_pthread_cond_wait`, `_pthread_cond_timedwait_relative_np`,
  `_pthread_cond_destroy`.

`_pthread_cond_timedwait_relative_np` is new to this tree. It takes a *relative*
`timespec`, which avoids the `clock_gettime`-plus-arithmetic dance that the
POSIX absolute-deadline form requires. Linux has no such call; plan-33-C uses
`pthread_cond_timedwait` with an absolute deadline instead. The two backends
must still produce identical observable timeout behavior.

`mmap`/`munmap` go through the existing platform hook, not through new imports.

## 6. Build-Time Assertions

The `AudioState` reservations in plan-33-A §5.1 are guesses until checked.
Add a Rust-side `const` assertion in the macOS backend that
`size_of::<pthread_mutex_t>() <= 128` and `size_of::<pthread_cond_t>() <= 128`
for the target, sourced from the platform headers rather than from this
document. If either fails, the build breaks rather than corrupting the ring.

## Phases

### Phase 1 - Framework tables and imports

- [ ] Add `AudioToolbox`, `CoreAudio`, `CoreFoundation` to
      `dylib_for_library` and `dylib_path`, with tests in both files.
- [ ] Add per-spec `runtime_imports` arms in
      `src/target/macos_aarch64/plan.rs` mapping each audio symbol to exactly
      the libraries it needs.
- [ ] Tests: a native-plan test that `audio::devices` alone imports `CoreAudio`
      + `CoreFoundation` and *not* `AudioToolbox`; that `audio::openOutput`
      imports `AudioToolbox`; that a non-audio program imports none of the three.

Acceptance: `cargo test --bin mfb os::macos target::macos_aarch64` passes and
the import-minimality tests hold.
Commit: -

### Phase 2 - Device enumeration

- [ ] Add `src/target/shared/code/audio/mod.rs` and `audio/macos.rs`; dispatch
      `builtins::audio` calls from
      `src/target/shared/code/mod.rs:lower_runtime_helper`.
- [ ] Emit `_mfb_rt_audio_audio_devices`: property-size query, device array,
      per-device name/UID/stream-config/default checks, `CFRelease` on every
      `CFStringRef`, `List OF AudioDevice` construction.
- [ ] Error translation: nonzero `OSStatus` → `ErrAudioDevice` with the status;
      zero devices → `ErrAudioUnavailable`.
- [ ] Tests: `tests/rt-behavior/audio/func_audio_devices_valid` asserts a
      nonempty list, whole records, and exactly one `isDefaultOutput` when a
      default output exists.

Acceptance: `mfb run` on a device-listing program prints the same device names
as `system_profiler SPAudioDataType` on the same host.
Commit: -

### Phase 3 - Output: open, write, close

- [ ] Emit the output callback as a C-ABI function following the
      `pthread_create` trampoline precedent, and take its address for
      `AudioQueueNewOutput`.
- [ ] Emit `openOutput` (both overloads), `write`, `available`, `poll` (both
      overloads), `xruns`, and `close` for output streams, including the
      mutex-release-before-Dispose ordering of §3.4.
- [ ] Add the §6 build-time size assertions.
- [ ] Tests: `func_audio_openOutput_valid`, `func_audio_write_valid`,
      `func_audio_close_valid`; `tests/rt-error/audio/` for every plan-33-A §3.5
      parameter violation, non-whole-frame `write`, `write` after `close`, and
      double `close`. (`read` on an `AudioOutput` is a *compile* error covered
      by plan-33-A Phase 1, not a runtime test.)

Acceptance: a native program writes 200 ms of a 440 Hz `s16le` tone to the
default output, closes, and exits 0 — audibly, on a real Mac. `leaks` reports no
leaked AudioQueue and the state page is unmapped.
Commit: -

### Phase 4 - Input: open, read, poll

- [ ] Emit the input callback, the ring copy with oldest-frame discard, and the
      `xruns` event increment.
- [ ] Emit `openInput` (both overloads), `read` (both overloads) with the
      whole-frames-only timeout rule, and input `available`/`poll`.
- [ ] Enforce the §4.5 default-input-device precheck.
- [ ] Tests: `func_audio_read_valid` (gated on a default input device),
      `func_audio_poll_valid`, `func_audio_read_timeout_valid` asserting a
      `timeoutMs = 0` read returns a whole-frame-aligned list, and
      `tests/rt-error/audio/` for `read` after `close`.

Acceptance: `audio::read(in, 4800)` on a 1-channel 48 kHz stream returns exactly
9600 bytes; the bytes are **not all zero** with a live microphone (see §4.5); a
`timeoutMs = 0` read on a just-opened stream returns an empty list rather than
blocking.
Commit: -

## Validation Plan

- Tests: import-minimality tests per audio symbol; `rt-behavior` for devices,
  open/write/close, read, poll, timeout alignment; `rt-error` for every
  parameter violation and both after-close misuses. Wrong-direction misuse is a
  compile error and is tested once, in plan-33-A.
- Runtime proof: on real hardware — (a) tone playback is audible and exits 0;
  (b) a capture of 100 ms from a live microphone contains at least one nonzero
  sample; (c) `xruns` stays 0 across a 5-second continuous playback loop;
  (d) opening a device by `id` obtained from `devices()` reaches that specific
  device. Record host, macOS version, default device names, and TCC state.
- Doc sync: deferred to plan-33-D.
- Acceptance: `scripts/test-accept.sh target/debug/mfb target/accept-actual`.

## Open Decisions

- Buffer count — recommended: 4 buffers of `bufferFrames` each, giving the
  caller latency control through one parameter; alternative: expose the count,
  which widens the API for a knob almost nobody turns.
- Default-device tracking — recommended: the no-device overloads let AudioQueue
  follow the system default live, including mid-stream switches; alternative:
  resolve the default once at open and pin it, which is more predictable but
  surprises a user who unplugs headphones.
- `CFStringRef` conversion buffer — recommended: a 1 KiB stack buffer with a
  `CFStringGetCString` failure treated as `ErrAudioDevice`; alternative: query
  the length first, which is one more call for a name that is never that long.

## Summary

AudioQueue rather than AudioUnit, because plan-33-A §6 forbids a lock-free ring
and this compiler has no atomics to build one with. The buffer-based model maps
cleanly onto "block until queued" for output and a mutex-guarded ring for input.
The two things most likely to go wrong are the Dispose-while-holding-the-mutex
deadlock (§3.4) and mistaking a TCC-denied silent capture for a working one
(§4.5); both have explicit countermeasures above.
