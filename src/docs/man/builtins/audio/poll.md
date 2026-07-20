# poll

Test an open stream for readiness, optionally waiting up to a deadline.

## Synopsis

```
audio::poll(stream AS AudioInput) AS Boolean
audio::poll(stream AS AudioOutput) AS Boolean
audio::poll(stream AS AudioInput, timeoutMs AS Integer) AS Boolean
audio::poll(stream AS AudioOutput, timeoutMs AS Integer) AS Boolean
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

`audio::poll` reports whether an open stream is ready for its next I/O operation,
returning a `Boolean`. For an `AudioInput`, ready means at least one whole frame
can be read; for an `AudioOutput`, ready means at least one buffer is free to
write. `poll` is defined over both directions, and the untimed form is exactly
`audio::available(stream) > 0`: both read the same mutex-guarded fill/free
counters. [[src/target/shared/code/audio/macos.rs:lower_query]][[src/target/shared/code/audio/alsa.rs:lower_query]]
The stream is borrowed, not consumed — the handle stays open and must still be
closed with `audio::close` or by lexical drop. [[src/builtins/audio.rs:consumes_argument]]

The one-argument form tests readiness immediately and never blocks. The
two-argument form waits up to `timeoutMs` milliseconds for the stream to become
ready, returning `TRUE` the moment it is and `FALSE` at the deadline; a
`timeoutMs` of `0` is a non-blocking test. `timeoutMs` is not range-checked —
any `Integer` is accepted. [[src/target/shared/code/audio/macos.rs:lower_query]][[src/target/shared/code/audio/alsa.rs:lower_query]]

Polling never fails on the stream itself. A stream that has already been closed
(or a defaulted handle) polls as `FALSE` rather than raising an error, so `poll`
is always safe to call. [[src/target/shared/code/audio/macos.rs:lower_query]][[src/target/shared/code/audio/alsa.rs:lower_query]]

On macOS the counters are maintained by the Core Audio callback thread and read
under the stream mutex; the timed form waits on the stream condition variable
until data arrives or the deadline passes. [[src/target/shared/code/audio/macos.rs:lower_query]]
On Linux readiness comes from `snd_pcm_avail_update` (untimed) or `snd_pcm_wait`
(timed) in a `libasound.so.2` resolved at runtime with `dlopen`; a binary that
imports `audio` still starts on a host without alsa-lib, but a `poll` there
raises `ErrAudioUnavailable` when the library or a required symbol cannot be
resolved. [[src/target/shared/code/audio/alsa.rs:emit_dlopen]][[src/target/shared/code/audio/alsa.rs:lower_query]]

## Overloads

**`audio::poll(stream)`**

Test readiness immediately without blocking. Equivalent to
`audio::available(stream) > 0`. [[src/target/shared/code/audio/macos.rs:lower_query]]

**`audio::poll(stream, timeoutMs)`**

Wait up to `timeoutMs` milliseconds for the stream to become ready, returning as
soon as it is. A `timeoutMs` of `0` polls without blocking. This form lowers to a
distinct internal body. [[src/builtins/audio.rs:implementation_name]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `stream` | `AudioInput` or `AudioOutput` | An open capture or playback stream, from `audio::openInput`/`audio::openOutput`. Borrowed, not consumed. A closed handle polls as `FALSE`. [[src/builtins/audio.rs:resolve_call]][[src/builtins/audio.rs:consumes_argument]] |
| `timeoutMs` | `Integer` | Maximum wait in milliseconds (timed overload only). `0` is a non-blocking test; not range-checked. [[src/target/shared/code/audio/macos.rs:lower_query]] |

## Return value

| Type | Description |
| --- | --- |
| `Boolean` | `TRUE` when the stream is ready (at least one frame readable, or one buffer writable), `FALSE` otherwise — including on a closed handle or, for the timed form, at the deadline. [[src/builtins/audio.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050017` | `ErrAudioUnavailable` | Linux only: `libasound.so.2` (or a required symbol such as `snd_pcm_avail_update` / `snd_pcm_wait`) could not be resolved at runtime. macOS never raises this. [[src/target/shared/code/audio/alsa.rs:emit_dlopen]][[src/target/shared/code/audio/alsa.rs:lower_query]][[src/target/shared/code/error_constants.rs:ERR_AUDIO_UNAVAILABLE_CODE]] |

## Examples

Drive a capture stream only when at least one frame is ready, waiting up to 50 ms:

```
IMPORT audio

SUB main()
  RES mic AS AudioInput = audio::openInput(48000, 1, 512)
  IF audio::poll(mic, 50) THEN
    LET pcm = audio::read(mic, 480, 0)
  END IF
  audio::close(mic)
END SUB
```

Non-blocking readiness check on an output stream:

```
IMPORT audio

SUB main()
  RES out AS AudioOutput = audio::openOutput(48000, 2, 512)
  LET pcm AS List OF Byte = [0, 0, 0, 0]
  IF audio::poll(out) THEN
    audio::write(out, pcm)
  END IF
  audio::close(out)
END SUB
```

## See also

- `mfb man audio available`
- `mfb man audio read`
- `mfb man audio write`
- `mfb man audio openInput`
- `mfb man audio close`
- `mfb man audio types`
