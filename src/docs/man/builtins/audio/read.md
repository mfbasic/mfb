# read

Capture PCM frames from an input stream as raw `s16le` bytes.

## Synopsis

```
audio::read(input AS AudioInput, frames AS Integer) AS List OF Byte
audio::read(input AS AudioInput, frames AS Integer, timeoutMs AS Integer) AS List OF Byte
```

## Package

audio

## Imports

```
IMPORT audio
```

`audio` is a built-in package, so no manifest dependency is required. A program
that does not `IMPORT audio` gains no audio symbol and no dynamic-library
dependency. [[src/builtins/audio.rs:augmented_project]]

## Description

`audio::read` captures PCM from an open `AudioInput` and returns it as a
`List OF Byte` of raw interleaved signed 16-bit little-endian (`s16le`) samples:
one frame is `channels * 2` bytes, so a successful blocking read yields exactly
`frames * channels * 2` bytes. `read` is defined only over `AudioInput`; passing
an `AudioOutput` is a compile-time overload-resolution error, never a runtime
check. [[src/builtins/audio.rs:resolve_call]] The stream is borrowed, not
consumed — the handle stays open and must still be closed with `audio::close`
or by lexical drop. [[src/builtins/audio.rs:consumes_argument]]

`frames` must be in `1..=1048576`: a value below `1` or above `1048576` raises
`ErrInvalidArgument` before capture begins. The timed form additionally caps
`timeoutMs` at `86400000` (24 hours) — a larger value raises
`ErrInvalidArgument`. Only that upper bound is enforced; a `timeoutMs` of `0`
(or any non-positive value) is accepted and returns immediately with whatever
whole frames are already buffered, exactly like a poll.
[[src/target/shared/code/audio/macos.rs:READ_FRAMES_MAX]][[src/target/shared/code/audio/alsa.rs:TIMEOUT_MAX]]

The two-argument form blocks until exactly `frames` frames are captured. The
three-argument form returns early when `timeoutMs` elapses, yielding only whole
frames gathered so far — possibly an empty list, never a partial frame, and
never more than `frames`; the result is right-sized to the frames actually
returned. A `timeoutMs` of `0` polls: it returns whatever whole frames are
already buffered without blocking. [[src/target/shared/code/audio/macos.rs:lower_read]][[src/target/shared/code/audio/alsa.rs:lower_read]]

On macOS, capture is drained from an internal ring filled by the Core Audio
callback thread, so the ring may be small relative to a large `frames` request.
On Linux, capture reads directly through `snd_pcm_readi` on the calling thread
via a `libasound.so.2` resolved at runtime with `dlopen`; a binary that imports
`audio` still starts on a host without alsa-lib, but a `read` there raises
`ErrAudioUnavailable` when the library or a required symbol cannot be resolved.
[[src/target/shared/code/audio/alsa.rs:emit_dlopen]] macOS drives Core Audio
directly and has no such runtime-library failure. [[src/target/shared/code/audio/macos.rs:lower_read]]

Reading a stream that has been closed, or one whose device has failed, raises
`ErrAudioDevice`. [[src/target/shared/code/audio/macos.rs:lower_read]]

## Overloads

**`audio::read(input, frames)`**

Block until exactly `frames` frames are captured, then return
`frames * channels * 2` bytes.

**`audio::read(input, frames, timeoutMs)`**

Return early once `timeoutMs` elapses, with only the whole frames captured so
far (possibly none). A `timeoutMs` of `0` (or any non-positive value) returns
immediately with whatever whole frames are already buffered, without blocking.
[[src/builtins/audio.rs:implementation_name]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `input` | `AudioInput` | An open capture stream, from `audio::openInput`. Borrowed, not consumed. Reading after close raises `ErrAudioDevice`. [[src/builtins/audio.rs:consumes_argument]] |
| `frames` | `Integer` | Number of frames to capture. Must be in `1..=1048576`. [[src/target/shared/code/audio/macos.rs:READ_FRAMES_MAX]] |
| `timeoutMs` | `Integer` | Maximum wait in milliseconds (timed overload only). Must not exceed `86400000` (24 hours); `0` or any non-positive value returns immediately with whatever is buffered. [[src/target/shared/code/audio/macos.rs:TIMEOUT_MAX]] |

## Return value

| Type | Description |
| --- | --- |
| `List OF Byte` | Interleaved `s16le` PCM. The blocking form returns exactly `frames * channels * 2` bytes; the timed form returns a whole-frame-aligned list of at most that size, possibly empty. [[src/builtins/audio.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `frames` is below `1` or above `1048576`, or (timed form) `timeoutMs` exceeds `86400000`. [[src/target/shared/code/audio/macos.rs:READ_FRAMES_MAX]][[src/target/shared/code/audio/alsa.rs:TIMEOUT_MAX]] |
| `77050018` | `ErrAudioDevice` | The stream is already closed, or the device failed during capture. [[src/target/shared/code/audio/macos.rs:lower_read]][[src/target/shared/code/audio/alsa.rs:lower_read]] |
| `77050017` | `ErrAudioUnavailable` | Linux only: `libasound.so.2` (or a required symbol) could not be resolved at runtime. [[src/target/shared/code/audio/alsa.rs:emit_dlopen]] |
| `77010001` | `ErrOutOfMemory` | Allocation of the result byte list failed. [[src/target/shared/code/audio/macos.rs:lower_read]][[src/target/shared/code/audio/alsa.rs:lower_read]] |

## Examples

Capture 100 ms of mono audio at 48 kHz, blocking until the full buffer is ready:

```
IMPORT audio

RES mic AS AudioInput = audio::openInput(48000, 1, 512)
LET pcm = audio::read(mic, 4800)
audio::close(mic)
```

Poll for whatever whole frames are already buffered, without blocking:

```
IMPORT audio

RES mic AS AudioInput = audio::openInput(48000, 1, 512)
LET now = audio::read(mic, 4800, 0)
audio::close(mic)
```

## See also

- `mfb man audio openInput`
- `mfb man audio poll`
- `mfb man audio available`
- `mfb man audio close`
- `mfb man audio types`
