# xruns

Cumulative count of overrun/underrun events on a stream since it was opened.

## Synopsis

```
audio::xruns(stream AS AudioInput) AS Integer
audio::xruns(stream AS AudioOutput) AS Integer
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

`audio::xruns` returns, as an `Integer`, the number of xrun events recorded on
an open stream since it was opened: capture overruns for an `AudioInput`,
playback underruns for an `AudioOutput`. The stream is borrowed, not consumed —
the handle stays open and must still be closed with `audio::close` or by lexical
drop. [[src/builtins/audio.rs:consumes_argument]]

The value is a monotonic counter maintained in the stream's shared state
(`S_XRUNS`); each xrun event increments it by exactly one. It counts events, not
lost frames: `audio::xruns(stream) > 0` means audio was lost, but the number of
frames destroyed is not reported by the platform, so an event count is the only
value that is exact everywhere. A stream that has never dropped audio reports
`0`. [[src/target/shared/code/audio/mod.rs:S_XRUNS]]

`xruns` cannot fail on the stream itself. Reading the counter takes no library
call — unlike `audio::available` and `audio::poll`, the xrun query does not open
`libasound.so.2`, so it never raises `ErrAudioUnavailable` even on a Linux host
without ALSA. [[src/target/shared/code/audio/alsa.rs:lower_query]] A stream that
has already been closed (or a defaulted handle) reports `0` rather than raising
an error, so `xruns` is always safe to call.
[[src/target/shared/code/audio/macos.rs:lower_query]]

On macOS the counter is bumped under the stream mutex by the Core Audio callback
threads — the input callback on a capture overrun and the output callback when a
started playback stream runs its buffers empty — and is read back here under the
same mutex; on Linux it is bumped when `snd_pcm_recover` recovers a stream after
an overrun or underrun.
[[src/target/shared/code/audio/macos.rs:lower_audio_input_callback]][[src/target/shared/code/audio/macos.rs:lower_audio_output_callback]][[src/target/shared/code/audio/macos.rs:lower_query]][[src/target/shared/code/audio/alsa.rs:lower_query]]

## Overloads

**`audio::xruns(stream AS AudioInput)`**

Capture overruns: the number of times the input ring dropped whole frames
because it filled before the program read from it.

**`audio::xruns(stream AS AudioOutput)`**

Playback underruns: the number of times the output starved because the program
did not supply data fast enough. Both overloads read the same counter and return
an `Integer`. [[src/builtins/audio.rs:resolve_call]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `stream` | `AudioInput` or `AudioOutput` | An open capture or playback stream, from `audio::openInput`/`audio::openOutput`. Borrowed, not consumed. A closed handle reports `0`. [[src/builtins/audio.rs:resolve_call]][[src/builtins/audio.rs:consumes_argument]] |

## Return value

| Type | Description |
| --- | --- |
| `Integer` | The cumulative xrun event count since the stream was opened; `0` when no audio has been lost, and `0` for a closed or defaulted handle. [[src/builtins/audio.rs:call_return_type_name]] |

## Errors

No errors.

## Examples

Check for lost audio after a playback loop:

```
IMPORT audio
IMPORT io

RES out AS AudioOutput = audio::openOutput(48000, 2, 512)
audio::write(out, pcm)
io::print("underruns: " & toString(audio::xruns(out)))
audio::close(out)
```

Detect a capture overrun and report the delta across a read:

```
IMPORT audio

RES mic AS AudioInput = audio::openInput(48000, 1, 512)
LET before = audio::xruns(mic)
LET pcm = audio::read(mic, 480, 0)
IF audio::xruns(mic) > before THEN
  io::print("dropped capture audio")
END IF
audio::close(mic)
```

## See also

- `mfb man audio available`
- `mfb man audio poll`
- `mfb man audio read`
- `mfb man audio write`
- `mfb man audio close`
- `mfb man audio types`
