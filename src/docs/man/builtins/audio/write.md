# write

Queue raw `s16le` PCM to an output stream, blocking until every byte is enqueued.

## Synopsis

```
audio::write(output AS AudioOutput, bytes AS List OF Byte) AS Nothing
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

`audio::write` queues raw interleaved signed 16-bit little-endian (`s16le`) PCM
for playback on an open `AudioOutput` and blocks until every byte has been handed
to the operating system for playback. It returns `Nothing`. `write` is defined
only over `AudioOutput`; passing an `AudioInput` is a compile-time
overload-resolution error, never a runtime check.
[[src/builtins/audio.rs:resolve_call]] The stream is borrowed, not consumed — the
handle stays open and must still be closed with `audio::close` or by lexical
drop. [[src/builtins/audio.rs:consumes_argument]]

`bytes` carries interleaved `s16le` samples: one frame is `channels * 2` bytes.
Its length must be nonzero and an exact whole number of frames (a multiple of the
stream's bytes-per-frame); a zero-length list or a length that is not
frame-aligned raises `ErrInvalidArgument` before any audio is queued. The data
is read from the list's capacity-based data region, so an append-built list plays
back correctly. [[src/target/shared/code/audio/macos.rs:lower_write]][[src/target/shared/code/audio/alsa.rs:lower_write]]

Playback is queued frame by frame, in order, until the whole list is enqueued;
`write` does not resample or reinterpret the bytes. On Linux, if `snd_pcm_writei`
reports an underrun `write` bumps the stream's underrun counter (read with
`audio::xruns`), calls `snd_pcm_recover`, and resumes rather than aborting; only a
recovery that itself fails raises `ErrAudioDevice`. On macOS the underrun counter
is incremented by the Core Audio callback when the queue runs dry, not by `write`.
[[src/target/shared/code/audio/alsa.rs:lower_write]]

Writing to a stream that has already been closed, or one whose device has failed,
raises `ErrAudioDevice`. macOS drives Core Audio directly through the output
`AudioQueue`. [[src/target/shared/code/audio/macos.rs:lower_write]] Linux drives
ALSA through `snd_pcm_writei` on the calling thread via a `libasound.so.2`
resolved at runtime with `dlopen`; a binary that imports `audio` still starts on
a host without alsa-lib, but a `write` there raises `ErrAudioUnavailable` when the
library or a required symbol cannot be resolved.
[[src/target/shared/code/audio/alsa.rs:emit_dlopen]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `output` | `AudioOutput` | An open playback stream, from `audio::openOutput`. Borrowed, not consumed. Writing after close raises `ErrAudioDevice`. [[src/builtins/audio.rs:consumes_argument]] |
| `bytes` | `List OF Byte` | Interleaved `s16le` PCM. Length must be nonzero and a whole multiple of `channels * 2` (one frame). [[src/target/shared/code/audio/macos.rs:lower_write]] |

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | Returns once every byte has been queued for playback. [[src/builtins/audio.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `bytes` is empty, or its length is not a whole multiple of the frame size (`channels * 2`). [[src/target/shared/code/audio/macos.rs:lower_write]][[src/target/shared/code/audio/alsa.rs:lower_write]] |
| `77050018` | `ErrAudioDevice` | The stream is already closed, or the device failed while queuing playback. [[src/target/shared/code/audio/macos.rs:lower_write]][[src/target/shared/code/audio/alsa.rs:lower_write]] |
| `77050017` | `ErrAudioUnavailable` | Linux only: `libasound.so.2` (or a required symbol) could not be resolved at runtime. [[src/target/shared/code/audio/alsa.rs:emit_dlopen]] |

## Examples

Open a stereo output at 48 kHz and play a buffer of PCM:

```
IMPORT audio

RES out AS AudioOutput = audio::openOutput(48000, 2, 512)
audio::write(out, pcm)
audio::close(out)
```

## See also

- `mfb man audio openOutput`
- `mfb man audio play`
- `mfb man audio xruns`
- `mfb man audio available`
- `mfb man audio close`
- `mfb man audio types`
