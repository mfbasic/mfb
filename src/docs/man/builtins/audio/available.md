# available

Frames an open stream can move immediately without blocking.

## Synopsis

```
audio::available(stream AS AudioInput) AS Integer
audio::available(stream AS AudioOutput) AS Integer
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

`audio::available` returns how many whole frames a stream can move right now
without blocking, as an `Integer`. For an `AudioInput` it is the frames currently
readable; for an `AudioOutput` it is the frames writable before `audio::write`
would block. Use it to size a `audio::read`, or to decide how much to write,
without stalling on the device. [[src/target/shared/code/audio/macos.rs:lower_query]][[src/target/shared/code/audio/alsa.rs:lower_query]]

`available` is defined over both directions and never blocks; it only reports the
current count. The result is never negative: on Linux a device-reported negative
count (an error return from `snd_pcm_avail_update`, such as `-EPIPE`) is clamped
to `0`; on macOS the count comes from unsigned fill/free counters that cannot go
negative. The untimed `audio::poll(stream)` is exactly
`audio::available(stream) > 0`: both read the same fill/free counters.
[[src/target/shared/code/audio/alsa.rs:lower_query]][[src/target/shared/code/audio/macos.rs:lower_query]]
The stream is borrowed, not consumed — the handle stays open and must still be
closed with `audio::close` or by lexical drop. [[src/builtins/audio.rs:consumes_argument]]

A stream that has already been closed (or a defaulted handle) reports `0` rather
than raising an error, so `available` is always safe to call. [[src/target/shared/code/audio/macos.rs:lower_query]][[src/target/shared/code/audio/alsa.rs:lower_query]]

On macOS the counter is maintained by the Core Audio callback thread and read
under the stream mutex; the call never fails on the stream itself.
[[src/target/shared/code/audio/macos.rs:lower_query]]
On Linux the count comes from `snd_pcm_avail_update` in a `libasound.so.2`
resolved at runtime with `dlopen`; a binary that imports `audio` still starts on
a host without alsa-lib, but a call to `available` there raises
`ErrAudioUnavailable` when the library or a required symbol cannot be resolved.
[[src/target/shared/code/audio/alsa.rs:emit_dlopen]][[src/target/shared/code/audio/alsa.rs:lower_query]]

## Overloads

**`audio::available(stream AS AudioInput)`**

Frames currently readable from the capture stream without blocking. [[src/builtins/audio.rs:resolve_call]]

**`audio::available(stream AS AudioOutput)`**

Frames writable to the playback stream before `audio::write` would block. Both
overloads share one internal body; the direction is read from the handle at
runtime. [[src/builtins/audio.rs:resolve_call]][[src/target/shared/code/audio/macos.rs:lower_query]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `stream` | `AudioInput` or `AudioOutput` | An open capture or playback stream, from `audio::openInput`/`audio::openOutput`. Borrowed, not consumed. A closed handle reports `0`. [[src/builtins/audio.rs:resolve_call]][[src/builtins/audio.rs:consumes_argument]] |

## Return value

| Type | Description |
| --- | --- |
| `Integer` | Frames readable (input) or writable (output) without blocking; never negative — a negative device count is clamped to `0`, as is a closed or defaulted handle. [[src/builtins/audio.rs:call_return_type_name]][[src/target/shared/code/audio/alsa.rs:lower_query]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050017` | `ErrAudioUnavailable` | Linux only: `libasound.so.2` (or a required symbol such as `snd_pcm_avail_update`) could not be resolved at runtime. macOS never raises this. [[src/target/shared/code/audio/alsa.rs:lower_query]][[src/target/shared/code/error_constants.rs:ERR_AUDIO_UNAVAILABLE_CODE]] |

## Examples

Read exactly what is available, without blocking:

```
IMPORT audio

SUB main()
  RES mic AS AudioInput = audio::openInput(48000, 1, 512)
  LET n = audio::available(mic)
  IF n > 0 THEN
    LET pcm = audio::read(mic, n)
  END IF
  audio::close(mic)
END SUB
```

Write only as much as the output can accept right now:

```
IMPORT audio

SUB main()
  RES out AS AudioOutput = audio::openOutput(48000, 2, 512)
  LET pcm AS List OF Byte = [0, 0, 0, 0]
  IF audio::available(out) > 0 THEN
    audio::write(out, pcm)
  END IF
  audio::close(out)
END SUB
```

## See also

- `mfb man audio poll`
- `mfb man audio read`
- `mfb man audio write`
- `mfb man audio xruns`
- `mfb man audio close`
- `mfb man audio types`
