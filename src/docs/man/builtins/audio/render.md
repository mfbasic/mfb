# render

Synthesize one `AudioNote` to mono `s16le` PCM at 48 kHz.

## Synopsis

```
audio::render(note AS AudioNote) AS List OF Byte
```

## Package

audio

## Imports

```
IMPORT audio
```

`audio` is a built-in package, so no manifest dependency is required. The
note/envelope records and `render` itself are supplied by a source companion that
is injected only when a program imports `audio`. [[src/builtins/audio.rs:augmented_project]]

## Description

`audio::render` is a pure MFBASIC tone synthesizer, not a device call: it never
opens hardware and touches no audio stream. It turns one `AudioNote` into raw
single-channel signed 16-bit little-endian (`s16le`) PCM at a fixed 48 kHz sample
rate and returns it as a `List OF Byte` — the same mono frame layout
`audio::write` consumes, so the result can be handed straight to an open
`AudioOutput`. The output is single-channel, so one frame is one sample of two
bytes; the returned list is
`note.noteFrames * 2` bytes long. A `note.noteFrames` of zero or less runs no
iterations and returns an empty list. [[src/builtins/audio_package.mfb:__audio_render]]

For each frame `i` the renderer evaluates a sine oscillator
`sin(2 * pi * note.frequencyHz * (i / 48000))` and shapes it with the note's
`AudioEnvelope`, all in raw s16 amplitude units where the peak is `32767`:

- a **linear attack** rising from silence to peak over the first `attackFrames`,
- a **linear decay** from peak down to `sustainLevel` over the next `decayFrames`,
- a **sustain** held at `sustainLevel` through the middle of the note, and
- a **linear release** falling from `sustainLevel` to silence over the final
  `releaseFrames`.

`holdFrames` is informational; the sustain fills whatever the note length leaves
between decay and release. [[src/builtins/audio_package.mfb:__audio_render]]

Each sample is the oscillator value times the envelope times `note.gainOverall`,
converted to an `Integer`, then clamped to the s16 range `[-32768, 32767]` and
encoded little-endian. The conversion happens **before** the clamp, so a
non-finite or wildly out-of-range product — for example a non-finite
`frequencyHz`/`gainOverall`, or a `gainOverall` large enough to push the product
past the `Integer` range — is rejected by the conversion rather than clamped.
[[src/builtins/audio_package.mfb:__audio_render]][[src/target/shared/code/builder_conversions.rs:emit_float_to_int_value]]

`render` is deterministic and platform-independent: it produces byte-identical
PCM on every target.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `note` | `AudioNote` | The note to synthesize: `frequencyHz` (Float, cycles per second), `noteFrames` (Integer total length in frames at 48 kHz), `envelope` (an `AudioEnvelope`), and `gainOverall` (Float, nominally 0..1). Construct it with `AudioNote[...]`. [[src/builtins/audio.rs:builtin_type_fields]] |

## Return value

| Type | Description |
| --- | --- |
| `List OF Byte` | Single-channel mono `s16le` PCM at 48 kHz, exactly `note.noteFrames * 2` bytes; empty when `note.noteFrames <= 0`. [[src/builtins/audio.rs:resolve_call]][[src/builtins/audio_package.mfb:__audio_render]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050003` | `ErrInvalidFormat` | A shaped sample is NaN or infinite before conversion (e.g. a non-finite `frequencyHz` or `gainOverall`). [[src/target/shared/code/builder_conversions.rs:emit_float_to_int_value]][[src/target/shared/code/error_constants.rs:ERR_INVALID_FORMAT_CODE]] |
| `77050010` | `ErrOverflow` | A shaped sample's magnitude exceeds the `Integer` range before it can be clamped (e.g. an extreme `gainOverall`). [[src/target/shared/code/builder_conversions.rs:emit_float_to_int_value]][[src/target/shared/code/error_constants.rs:ERR_OVERFLOW_CODE]] |

## Examples

Render one second of A4 (440 Hz) and play it on the default output:

```
IMPORT audio

LET env = AudioEnvelope[2400, 4800, 31200, 9600, 12000]
LET note = AudioNote[440.0, 48000, env, 0.8]
LET tone = audio::render(note)

RES out AS AudioOutput = audio::openOutput(48000, 1, 512)
audio::write(out, tone)
audio::close(out)
```

## See also

- `mfb man audio types`
- `mfb man audio write`
- `mfb man audio play`
- `mfb man audio openOutput`
