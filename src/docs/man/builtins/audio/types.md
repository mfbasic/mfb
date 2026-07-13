# types

the audio package types

## Synopsis

```
audio::AudioInput
audio::AudioOutput
audio::AudioDevice
```

## Package

audio

## Imports

```
IMPORT audio
```

`audio` is a built-in package, so `IMPORT audio` needs no manifest dependency.

## Description

The `audio` package defines two stream resource types and one plain record.

`AudioInput` and `AudioOutput` are opaque, owned, move-only, non-sendable
resource handles — a capture stream and a playback stream respectively. Because
direction is part of the type, `audio::read` accepts only an `AudioInput` and
`audio::write` only an `AudioOutput`; swapping them does not compile. Neither can
cross a thread boundary, be copied, be stored in a collection, or be carried in a
record. Each is closed automatically by lexical drop when its binding leaves
scope, or explicitly with `audio::close`; using a stream after it is closed
raises an error, and closing twice is a no-op.

`AudioDevice` is a plain read-only record obtained only from `audio::devices()` —
a program cannot construct one, because its `id` is an opaque platform handle
(a Core Audio device UID on macOS, an ALSA PCM hint `NAME` on Linux) that must
be handed back to `audio::openInput`/`audio::openOutput` verbatim. It carries no
channel counts or supported rates: those cannot be reported without opening the
device on Linux, and a field truthful on one platform and zero on the other would
be worse than no field.

## Types

### AudioInput

An open capture stream. Obtained from `audio::openInput`. Move-only,
non-sendable; closed by `audio::close`.

### AudioOutput

An open playback stream. Obtained from `audio::openOutput`. Move-only,
non-sendable; closed by `audio::close`.

### AudioDevice

A description of one audio device, obtained only from `audio::devices()`.

| Field | Type | Description |
| --- | --- | --- |
| `id` | `String` | Opaque, platform-specific device identifier, stable within one run. Pass it to `openInput`/`openOutput`; never construct it. |
| `name` | `String` | Human-readable device name. |
| `canInput` | `Boolean` | Whether the device supports capture. |
| `canOutput` | `Boolean` | Whether the device supports playback. |
| `isDefaultInput` | `Boolean` | Whether this is the system default capture device. |
| `isDefaultOutput` | `Boolean` | Whether this is the system default playback device. |

## See also

- `mfb man audio`
- `mfb man audio devices`
- `mfb man audio openInput`
- `mfb man audio openOutput`
