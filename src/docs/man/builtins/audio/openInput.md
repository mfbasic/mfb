# openInput

Open a capture stream and return an `AudioInput` handle.

## Synopsis

```
audio::openInput(sampleRate AS Integer, channels AS Integer, bufferFrames AS Integer) AS audio::AudioInput
audio::openInput(device AS AudioDevice, sampleRate AS Integer, channels AS Integer, bufferFrames AS Integer) AS audio::AudioInput
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

`audio::openInput` opens a PCM capture stream and returns an `AudioInput`. The
three-argument form opens the system default input device; the four-argument
form opens the specific device named by an `AudioDevice` obtained from
`audio::devices()`. [[src/codegen/builtins/audio/mod.rs:register]]

The stream delivers raw interleaved signed 16-bit little-endian (`s16le`) PCM:
one frame is `channels * 2` bytes. `sampleRate` is the capture rate in Hz and
must be in `8000..=192000`; `channels` must be `1` (mono) or `2` (stereo);
`bufferFrames` is the frames per OS buffer and must be in `64..=8192`. Any value
outside these bounds raises `ErrInvalidArgument` before the device is touched.
[[src/codegen/builtins/audio/native/macos.rs:emit_validate_open]][[src/codegen/builtins/audio/native/common.rs:SR_MIN]]

The returned `AudioInput` is a move-only, non-sendable resource: it cannot be
copied or transferred to another thread. Bind it with `RES`; it is closed
automatically by lexical drop, or explicitly with `audio::close`. Read captured
frames with `audio::read`, which is defined only over `AudioInput` — passing an
`AudioOutput` is a compile-time overload-resolution error, never a runtime check.
[[src/codegen/builtins/audio/mod.rs:CLOSE_INPUT]]

macOS drives Core Audio directly; opening an input stream triggers microphone
authorization, attributed to the responsible terminal or IDE process. A *denied*
microphone does not raise an error — it delivers buffers of digital silence, so
an all-zero capture is indistinguishable from success by byte count alone; verify
a captured buffer contains a nonzero sample before treating capture as working.
The default (three-argument) form additionally requires a default input device to
exist, and raises `ErrAudioUnavailable` when none does.
[[src/codegen/builtins/audio/native/macos.rs:lower_open_input]] Linux drives ALSA
through a `libasound.so.2` resolved at runtime with `dlopen`, so a binary that
imports `audio` still starts on a host without alsa-lib; both forms raise
`ErrAudioUnavailable` there when the library cannot be resolved.
[[src/codegen/builtins/audio/native/alsa.rs:emit_dlopen]]

## Overloads

**`audio::openInput(sampleRate, channels, bufferFrames)`**

Open the system default input device. On macOS this fails with
`ErrAudioUnavailable` when no default input device is present.
[[src/codegen/builtins/audio/native/macos.rs:lower_open_input]]

**`audio::openInput(device, sampleRate, channels, bufferFrames)`**

Open the specific input device identified by `device` (from `audio::devices()`),
using the same `sampleRate`/`channels`/`bufferFrames`.
[[src/codegen/builtins/audio/func_open_input.rs:openInputDevice]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `device` | `AudioDevice` | The device to open, from `audio::devices()` (four-argument form only). A device whose `id` no longer exists raises `ErrAudioDevice`. |
| `sampleRate` | `Integer` | Capture rate in Hz. Must be in `8000..=192000`. [[src/codegen/builtins/audio/native/common.rs:SR_MIN]] |
| `channels` | `Integer` | Channel count: `1` (mono) or `2` (stereo). [[src/codegen/builtins/audio/native/macos.rs:emit_validate_open]] |
| `bufferFrames` | `Integer` | Frames per OS buffer. Must be in `64..=8192`. [[src/codegen/builtins/audio/native/common.rs:BUF_MIN]] |

## Return value

| Type | Description |
| --- | --- |
| `AudioInput` | An open, move-only capture stream delivering interleaved `s16le` PCM at the requested rate and channel count. [[src/codegen/builtins/audio/mod.rs:register]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | `sampleRate` is outside `8000..=192000`, `channels` is not `1` or `2`, or `bufferFrames` is outside `64..=8192`. [[src/codegen/builtins/audio/native/macos.rs:emit_validate_open]][[src/codegen/builtins/audio/native/alsa.rs:emit_validate_open]] |
| `77050017` | `ErrAudioUnavailable` | macOS: the default (three-argument) form finds no default input device. Linux: `libasound.so.2` (or a required symbol) could not be resolved at runtime. [[src/codegen/builtins/audio/native/macos.rs:lower_open_input]][[src/codegen/builtins/audio/native/alsa.rs:emit_dlopen]] |
| `77050018` | `ErrAudioDevice` | The device could not be opened, configured, or prepared for the requested rate/channel/buffer settings (includes a device whose `id` no longer exists). [[src/codegen/builtins/audio/native/macos.rs:lower_open_input]][[src/codegen/builtins/audio/native/alsa.rs:lower_open]] |
| `77010001` | `ErrOutOfMemory` | Allocation of the stream handle or its state page failed. [[src/codegen/builtins/audio/native/macos.rs:lower_open_input]][[src/codegen/builtins/audio/native/alsa.rs:lower_open]] |

## Examples

Capture 100 ms of mono audio at 48 kHz from the default input:

```
IMPORT audio

SUB main()
  RES mic AS audio::AudioInput = audio::openInput(48000, 1, 512)
  LET pcm = audio::read(mic, 4800)
  audio::close(mic)
END SUB
```

Open a specific input device chosen from the enumerated list:

```
IMPORT audio

SUB main()
  FOR EACH d IN audio::devices()
    IF d.isDefaultInput THEN
      RES mic AS audio::AudioInput = audio::openInput(d, 48000, 2, 512)
      LET pcm = audio::read(mic, 4800)
      audio::close(mic)
    END IF
  NEXT
END SUB
```

## See also

- `mfb man audio read`
- `mfb man audio close`
- `mfb man audio devices`
- `mfb man audio openOutput`
- `mfb man audio types`
