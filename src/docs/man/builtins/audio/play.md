# play

Parse MML music text and play it on an open output stream.

## Synopsis

```
audio::play(output AS AudioOutput, mml AS String) AS Nothing
audio::play(output AS AudioOutput, tracks AS List OF String) AS Nothing
```

## Package

audio

## Imports

```
IMPORT audio
```

`audio` is a built-in package, so no manifest dependency is required. `play` and
the MML sequencer it drives are supplied by a source companion that is injected
only when a program imports `audio`. [[src/builtins/audio.rs:augmented_project]]

## Description

`audio::play` is a small MML (Music Macro Language) sequencer. It parses one or
more tracks of MML text, synthesizes each track to mono signed 16-bit
little-endian (`s16le`) PCM at a fixed 48 kHz sample rate, mixes the tracks by
summing (with clamping to the `s16` range), and writes the result to `output` via
`audio::write`. Because the sequencer renders at 48 kHz mono, `output` must be an
`AudioOutput` opened with `sampleRate = 48000` and `channels = 1`.
[[src/builtins/audio_package.mfb:__mml_synth]][[src/builtins/audio_package.mfb:__audio_play_samples]]

`play` parses and synthesizes the entire program before writing, so malformed MML
raises an error and nothing is written. When the rendered PCM is non-empty, the
single `audio::write` call blocks until the audio is queued to the device; an
all-rest or empty program writes nothing. The `output` stream is **borrowed** — it
is not consumed, so the caller keeps ownership and must close it.
[[src/builtins/audio.rs:consumes_argument]][[src/builtins/audio_package.mfb:__audio_play_samples]]

A track is a string of space-separated tokens (`play` splits on the space
character, so every token must be separated by a space — `C E G`, never `CEG`).
Empty tokens from repeated spaces are ignored. Each track is fully isolated: the
tempo, default length, octave, volume, and instrument set in one track never carry
into another. [[src/builtins/audio_package.mfb:__mml_tokens]][[src/builtins/audio_package.mfb:__mml_parse]]

Tokens (all case-sensitive; note letters are upper case, instrument names lower
case): [[src/builtins/audio_package.mfb:__mml_parse]]

- `A` `B` `C` `D` `E` `F` `G` — a note. Append `+` or `-` for a sharp or flat, then
  optional length digits (as in `L`, in range 1..64), then optional trailing dots.
  `C`, `C+`, `D-`, `D16`, and `C+8.` are all notes. [[src/builtins/audio_package.mfb:__mml_note]]
- `R` — a rest for the current default length; trailing dots extend it (`R.`).
- `P1` .. `P64` — a pause (rest) of the given length.
- `O0` .. `O6` — set the octave; `O4` is the octave of A440.
- `<` `>` — shift the octave down / up by one, clamped to `0..6`.
- `L1` .. `L64` — set the default note length (1 = whole, 4 = quarter, …).
- `T32` .. `T255` — set the tempo in beats per minute.
- `V0` .. `V10` — set the volume (0 silent, 10 full).
- `I <name>` — set the instrument to `square`, `triangle`, `sine`, `saw`, or
  `noise`; the name is a separate space-separated token. [[src/builtins/audio_package.mfb:__mml_waveCode]]
- `( .. )` — legato: the enclosed notes are tied, with no attack/release at the
  interior joins. May not be nested inside legato or staccato.
- `[ .. ]` — staccato: the enclosed notes are shortened. May not be nested inside
  legato or staccato.
- `{ .. }<count>` — repeat the enclosed tokens `count` times (`count >= 1`); the
  count is attached to the closing brace (`}2`). May nest. [[src/builtins/audio_package.mfb:__mml_expand]]

`play` is deterministic: the same tracks produce byte-identical audio on every
target (the `noise` instrument uses a fixed-seed LCG).
[[src/builtins/audio_package.mfb:__mml_lcg]]

## Overloads

**`audio::play(output AS AudioOutput, mml AS String) AS Nothing`**

Plays a single MML track. [[src/builtins/audio_package.mfb:__audio_play]]

**`audio::play(output AS AudioOutput, tracks AS List OF String) AS Nothing`**

Plays several MML tracks together on the same stream, mixing them frame-by-frame;
shorter tracks are padded with silence. The overload is selected on the second
argument's type. [[src/builtins/audio.rs:source_implementation_name]][[src/builtins/audio_package.mfb:__audio_play_tracks]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `output` | `AudioOutput` | An open playback stream opened at 48 kHz mono (`audio::openOutput(48000, 1, ...)`). Borrowed — `play` writes to it and leaves it open. [[src/builtins/audio.rs:resolve_call]] |
| `mml` | `String` | A single MML track (single-track overload). [[src/builtins/audio_package.mfb:__audio_play]] |
| `tracks` | `List OF String` | Several MML tracks played together (multi-track overload). [[src/builtins/audio_package.mfb:__audio_play_tracks]] |

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | `play` writes audio for its side effect and returns no value. [[src/builtins/audio.rs:resolve_call]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050002` | `ErrInvalidArgument` | The MML is malformed: an unrecognized token, an out-of-range value (`tempo`, `octave`, length, `volume`, `pause`, note length), an unknown instrument, `I` with no instrument name, unbalanced or illegally nested `( )` / `[ ]`, an unbalanced `{ }`, or a repeat count below 1. [[src/builtins/audio_package.mfb:__mml_parse]][[src/builtins/audio_package.mfb:__mml_reqInt]][[src/builtins/audio_package.mfb:__mml_expand]][[src/target/shared/code/error_constants.rs:ERR_INVALID_ARGUMENT_CODE]] |
| `77050018` | `ErrAudioDevice` | The rendered audio failed to write because `output` is already closed or the device failed while queuing playback. [[src/builtins/audio_package.mfb:__audio_play_samples]][[src/target/shared/code/audio/macos.rs:lower_write]][[src/target/shared/code/audio/alsa.rs:lower_write]][[src/target/shared/code/error_constants.rs:ERR_AUDIO_DEVICE_CODE]] |
| `77050017` | `ErrAudioUnavailable` | Linux only: writing the rendered audio when `libasound.so.2` (or a required symbol such as `snd_pcm_writei`) cannot be resolved at runtime. macOS never raises this, and an all-rest or empty program writes nothing and so cannot raise it. [[src/target/shared/code/audio/alsa.rs:emit_dlopen]][[src/target/shared/code/audio/alsa.rs:lower_write]][[src/target/shared/code/error_constants.rs:ERR_AUDIO_UNAVAILABLE_CODE]] |

## Examples

Play a single track on the default output:

```
IMPORT audio

RES out AS AudioOutput = audio::openOutput(48000, 1, 512)
audio::play(out, "T100 O4 L8 I sine C E G < C > [ C E G ] { C. D16 }2")
audio::close(out)
```

Play a bass line and a lead together on the same stream:

```
IMPORT audio

LET bass = "T100 O2 L4 I triangle { C G }4"
LET lead = "T100 O4 L8 I sine C E G < C > [ C E G ] { C. D16 }2"

RES out AS AudioOutput = audio::openOutput(48000, 1, 512)
audio::play(out, [bass, lead])
audio::close(out)
```

## See also

- `mfb man audio render`
- `mfb man audio write`
- `mfb man audio openOutput`
- `mfb man audio types`
