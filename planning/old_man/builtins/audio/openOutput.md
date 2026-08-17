# openOutput

Open a playback stream and return an `AudioOutput` handle.

## Synopsis

```
audio::openOutput(sampleRate AS Integer, channels AS Integer, bufferFrames AS Integer) AS audio::AudioOutput
audio::openOutput(device AS AudioDevice, sampleRate AS Integer, channels AS Integer, bufferFrames AS Integer) AS audio::AudioOutput
```

## Package

audio

## Imports

```
IMPORT audio
```

`audio` is a built-in package, so no manifest dependency is required. A program
that does not `IMPORT audio` gains no audio symbol and no dynamic-library
dependency. [[src/codegen/builtins/audio/mod.rs:register]]

## Description

`audio::openOutput` opens a PCM playback stream and returns an `AudioOutput`. The
three-argument form opens the system default output device; the four-argument
form opens the specific device named by an `AudioDevice` obtained from
`audio::devices()`. [[src/codegen/builtins/audio/mod.rs:register]]

The stream carries raw interleaved signed 16-bit little-endian (`s16le`) PCM: one
frame is `channels * 2` bytes. `sampleRate` is the playback rate in Hz and must be
in `8000..=192000`; `channels` must be `1` (mono) or `2` (stereo); `bufferFrames`
is the frames per OS buffer and must be in `64..=8192`. Any value outside these
bounds raises `ErrInvalidArgument` before the device is touched.
[[src/codegen/builtins/audio/native/macos.rs:emit_validate_open]][[src/codegen/builtins/audio/native/common.rs:SR_MIN]]

`bufferFrames` sets the per-buffer latency the caller controls; it is not a hard
latency guarantee. `channels` and `sampleRate` are not resampled: on Linux the
committed rate and channel count must match the request exactly or the call raises
`ErrAudioDevice` (no silent resampling).
[[src/codegen/builtins/audio/native/alsa.rs:emit_configure_hw_params]]

The returned `AudioOutput` is a move-only, non-sendable resource: it cannot be
copied or transferred to another thread. Bind it with `RES`; it is closed
automatically by lexical drop, or explicitly with `audio::close`. Feed it with
`audio::write`, which blocks until every byte is queued for playback, or with
`audio::play` to render MML tracks. `audio::write`/`audio::play` are defined only
over `AudioOutput` — passing an `AudioInput` is a compile-time overload-resolution
error, never a runtime check. [[src/codegen/builtins/audio/mod.rs:CLOSE_INPUT]]

macOS drives Core Audio directly through an output `AudioQueue`; the default
(three-argument) form uses the system default output implicitly, so there is no
default-device lookup and no `ErrAudioUnavailable` path there.
[[src/codegen/builtins/audio/native/macos.rs:lower_open_output]] Linux drives ALSA
through a `libasound.so.2` resolved at runtime with `dlopen`, so a binary that
imports `audio` still starts on a host without alsa-lib; both forms raise
`ErrAudioUnavailable` there when the library (or a required symbol) cannot be
resolved. [[src/codegen/builtins/audio/native/alsa.rs:emit_dlopen]]

## Overloads

**`audio::openOutput(sampleRate, channels, bufferFrames)`**

Open the system default output device.
[[src/codegen/builtins/audio/native/macos.rs:lower_open_output]]

**`audio::openOutput(device, sampleRate, channels, bufferFrames)`**

Open the specific output device identified by `device` (from `audio::devices()`),
using the same `sampleRate`/`channels`/`bufferFrames`.
[[src/codegen/builtins/audio/func_open_input.rs:openInputDevice]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `device` | `AudioDevice` | The device to open, from `audio::devices()` with `canOutput` set (four-argument form only). A device whose `id` no longer exists raises `ErrAudioDevice`. |
| `sampleRate` | `Integer` | Playback rate in Hz. Must be in `8000..=192000`. [[src/codegen/builtins/audio/native/common.rs:SR_MIN]] |
| `channels` | `Integer` | Channel count: `1` (mono) or `2` (stereo). [[src/codegen/builtins/audio/native/macos.rs:emit_validate_open]] |
| `bufferFrames` | `Integer` | Frames per OS buffer. Must be in `64..=8192`; need not be a power of two. [[src/codegen/builtins/audio/native/common.rs:BUF_MIN]] |

## Return value

| Type | Description |
| --- | --- |
| `AudioOutput` | An open, move-only playback stream accepting interleaved `s16le` PCM at the requested rate and channel count. [[src/codegen/builtins/audio/mod.rs:register]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `sampleRate` is outside `8000..=192000`, `channels` is not `1` or `2`, or `bufferFrames` is outside `64..=8192`. [[src/codegen/builtins/audio/native/macos.rs:emit_validate_open]][[src/codegen/builtins/audio/native/alsa.rs:emit_validate_open]] |
| `77050017` | `ErrAudioUnavailable` | Linux only: `libasound.so.2` (or a required symbol) could not be resolved at runtime. [[src/codegen/builtins/audio/native/alsa.rs:emit_dlopen]] |
| `77050018` | `ErrAudioDevice` | The device could not be opened, configured, or prepared for the requested rate/channel/buffer settings (includes a named device whose `id` no longer exists, and a Linux device that will not commit the exact requested rate/channels). [[src/codegen/builtins/audio/native/macos.rs:lower_open_output]][[src/codegen/builtins/audio/native/alsa.rs:lower_open]] |
| `77010001` | `ErrOutOfMemory` | Allocation of the stream handle failed. [[src/codegen/builtins/audio/native/macos.rs:lower_open_output]][[src/codegen/builtins/audio/native/alsa.rs:lower_open]] |

## Examples

Open the default mono output at 48 kHz and play a short MML tune:

```
IMPORT audio

SUB main()
  RES out AS audio::AudioOutput = audio::openOutput(48000, 1, 512)
  audio::play(out, "T120 O4 L8 I sine C E G")
  audio::close(out)
END SUB
```

Open a specific output device chosen from the enumerated list:

```
IMPORT audio

SUB main()
  FOR EACH d IN audio::devices()
    IF d.isDefaultOutput THEN
      RES out AS audio::AudioOutput = audio::openOutput(d, 48000, 2, 512)
      audio::play(out, "cde")
      audio::close(out)
    END IF
  NEXT
END SUB
```

## See also

- `mfb man audio write`
- `mfb man audio play`
- `mfb man audio close`
- `mfb man audio devices`
- `mfb man audio openInput`
- `mfb man audio types`
