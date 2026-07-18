# types

the audio package types

## Synopsis

```
audio::AudioInput
audio::AudioOutput
audio::AudioDevice
audio::AudioEnvelope
audio::AudioNote
```

## Package

audio

## Imports

```
IMPORT audio
```

`audio` is a built-in package, so `IMPORT audio` needs no manifest dependency.

## Description

The `audio` package defines two stream resource types, one plain device record,
and two value records for tone synthesis.

`AudioInput` and `AudioOutput` are opaque, owned, move-only, non-sendable
resource handles — a capture stream and a playback stream respectively. Because
direction is part of the type, `audio::read` accepts only an `AudioInput` and
`audio::write` only an `AudioOutput`; swapping them does not compile. Neither can
cross a thread boundary, be copied, be stored in a collection, or be carried in a
record. Each is closed automatically by lexical drop when its binding leaves
scope, or explicitly with `audio::close`; using a stream after it is closed
raises an error, and closing twice is a no-op. [[src/builtins/audio.rs:resource_close_function]]

`AudioDevice` is a plain read-only record obtained only from `audio::devices()` —
a program cannot construct one, because its `id` is an opaque platform handle
(a Core Audio device UID on macOS, an ALSA PCM hint `NAME` on Linux) that must
be handed back to `audio::openInput`/`audio::openOutput` verbatim. It carries no
channel counts or supported rates: those cannot be reported without opening the
device on Linux, and a field truthful on one platform and zero on the other would
be worse than no field. [[src/target/shared/code/audio/macos.rs:SEL_UID]] [[src/target/shared/code/audio/mod.rs]]

## Types

### AudioInput

An open capture stream. Obtained from `audio::openInput`. Move-only,
non-sendable; closed by `audio::close`.

### AudioOutput

An open playback stream. Obtained from `audio::openOutput`. Move-only,
non-sendable; closed by `audio::close`.

### AudioDevice

A description of one audio device, obtained only from `audio::devices()`. [[src/builtins/audio.rs:builtin_type_fields]]

| Field | Type | Description |
| --- | --- | --- |
| `id` | `String` | Opaque, platform-specific device identifier, stable within one run. Pass it to `openInput`/`openOutput`; never construct it. |
| `name` | `String` | Human-readable device name. |
| `canInput` | `Boolean` | Whether the device supports capture. |
| `canOutput` | `Boolean` | Whether the device supports playback. |
| `isDefaultInput` | `Boolean` | Whether this is the system default capture device. |
| `isDefaultOutput` | `Boolean` | Whether this is the system default playback device. |

### AudioEnvelope

A linear ADSR amplitude envelope in raw s16 sample units (`0..32767`), passed to
`audio::render` through an `AudioNote`. Unlike `AudioDevice`, it is an ordinary
value record: construct it with `AudioEnvelope[...]`. [[src/builtins/audio_package.mfb:AudioEnvelope]] [[src/builtins/audio.rs:builtin_type_fields]]

| Field | Type | Description |
| --- | --- | --- |
| `attackFrames` | `Integer` | Frames of linear rise from silence to `peak` (32767). |
| `decayFrames` | `Integer` | Frames of linear fall from `peak` to `sustainLevel`. |
| `holdFrames` | `Integer` | Informational; the sustain fills whatever the note's length leaves between decay and release. |
| `releaseFrames` | `Integer` | Frames of linear fall from `sustainLevel` to silence at the note's end. |
| `sustainLevel` | `Integer` | Held amplitude between decay and release, in `0..32767`. |

### AudioNote

A single note for `audio::render` to synthesize: a sine at `frequencyHz` for
`noteFrames` frames, shaped by `envelope` and scaled by `gainOverall`. A value
record; construct it with `AudioNote[...]`. [[src/builtins/audio_package.mfb:AudioNote]] [[src/builtins/audio.rs:builtin_type_fields]]

| Field | Type | Description |
| --- | --- | --- |
| `frequencyHz` | `Float` | Sine frequency in hertz. |
| `noteFrames` | `Integer` | Total length in frames (one frame is 2 bytes of mono s16le at 48 kHz). [[src/builtins/audio_package.mfb:__audio_render]] |
| `envelope` | `AudioEnvelope` | The amplitude envelope applied over the note. |
| `gainOverall` | `Float` | Overall gain scaling the whole note, `0..1`. |

## See also

- `mfb man audio`
- `mfb man audio devices`
- `mfb man audio openInput`
- `mfb man audio openOutput`
- `mfb man audio render`
