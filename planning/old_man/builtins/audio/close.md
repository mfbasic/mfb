# close

Close an audio stream and release its operating-system resources, consuming the handle.

## Synopsis

```
audio::close(stream AS audio::AudioInput) AS Nothing
audio::close(stream AS audio::AudioOutput) AS Nothing
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

`audio::close` shuts an open capture or playback stream down and releases the
underlying OS objects, returning `Nothing`. It is defined over both directions;
`audio::close` stays the single user-facing name and IR lowering routes each
operand to a distinct per-direction internal body
(`audio.closeInput` / `audio.closeOutput`). [[src/codegen/builtins/audio/mod.rs:runtime_overload_name]][[src/codegen/builtins/audio/mod.rs:CLOSE_INPUT]]
Unlike `audio::available`, `audio::poll`, and `audio::xruns` — which share one
body and read the direction from the handle at runtime — the two `close` forms
are separate helpers with separate symbols (`_mfb_rt_audio_audio_closeInput`,
`_mfb_rt_audio_audio_closeOutput`), because their teardown sequences genuinely
differ.
[[src/codegen/builtins/audio/func_close.rs:closeInput]][[src/codegen/builtins/audio/func_close.rs:closeOutput]]

Unlike every other `audio::` call, `close` **consumes** its stream handle: the
binding is moved into the call and cannot be used afterward.
[[src/syntaxcheck/builtins.rs:audio_consumes_argument]] A stream is also closed automatically
by lexical drop when its binding leaves scope, so an explicit `close` is only
needed to release a stream earlier than the end of its scope; the same
per-direction body backs both paths. [[src/codegen/builtins/audio/mod.rs:CLOSE_INPUT]]

Closing an `AudioOutput` first **drains** queued playback — it waits for every
buffer the operating system still owns to finish before tearing the stream down —
then stops, disposes, and unmaps the stream state. Closing an `AudioInput`
instead **drops** any buffered capture immediately and tears the stream down
without waiting. [[src/codegen/builtins/audio/native/macos.rs:lower_close_output]][[src/codegen/builtins/audio/native/macos.rs:lower_close_input]][[src/codegen/builtins/audio/native/alsa.rs:lower_close]]

**Closing an output can therefore block.** On macOS `close` first pads the buffer
the last `write` left part-filled, if any, with silence up to a whole buffer and
enqueues it, because an `AudioQueue` never finishes a buffer holding less than a
full period and the drain below would otherwise wait forever. The drain itself is
not a device call but a condition-variable wait loop: `close` holds the stream
mutex and waits on the stream condvar until the free-buffer stack holds all four
of the stream's `AudioQueue` buffers, which happens only once the callback thread
has handed back every buffer it was playing. The call returns no sooner than the queued audio
finishes sounding, so the wait is bounded by however much PCM the program has
written but not yet heard — up to four times the `bufferFrames` the stream was
opened with. Closing an input takes no such wait; it stops the queue with the
immediate flag set, discarding whatever the ring still holds.
[[src/codegen/builtins/audio/native/macos.rs:lower_close_output]][[src/codegen/builtins/audio/native/mod.rs:NUM_BUFFERS]][[src/codegen/builtins/audio/native/macos.rs:lower_close_input]]

Teardown then runs in a fixed order in both directions: the stream's shared state
is marked closed (so a callback that fires mid-teardown does nothing), the queue
is stopped and disposed, the condvar and mutex are destroyed, the handle's closed
flag is set, and finally the state page itself is `munmap`ped. Because the state
page is unmapped, nothing survives a close for a later call to read — this is why
`audio::available`, `audio::poll`, and `audio::xruns` answer from the handle's
closed flag alone and report `0`/`FALSE` for a closed stream rather than
consulting state that no longer exists.
[[src/codegen/builtins/audio/native/macos.rs:lower_close_output]][[src/codegen/builtins/audio/native/alsa.rs:lower_close]]

`close` is idempotent. Each handle carries a closed flag that is checked first;
closing a stream that is already closed (or a defaulted handle) is a no-op that
returns successfully, never an error, and does not touch the audio library.
[[src/codegen/builtins/audio/native/alsa.rs:lower_close]][[src/codegen/builtins/audio/native/macos.rs:lower_close_output]]

On macOS the stream is driven directly through Core Audio (`AudioQueue`), which
is linked at load time, so `close` never fails: the drain, stop, dispose,
destroy, and `munmap` steps always run to completion.
[[src/codegen/builtins/audio/native/macos.rs:lower_close_output]][[src/codegen/builtins/audio/native/macos.rs:lower_close_input]]
On Linux the drain/drop and teardown go through `snd_pcm_drain` /
`snd_pcm_drop` and `snd_pcm_close` in a `libasound.so.2` resolved at runtime with
`dlopen`; a binary that imports `audio` still starts on a host without alsa-lib,
but closing an open (not already-closed) stream there raises
`ErrAudioUnavailable` when the library or a required symbol cannot be resolved.
Only that *resolution* failure raises. An error **returned** by `snd_pcm_drain`
or `snd_pcm_drop` is deliberately not propagated: a device that refuses to drain
must not be allowed to skip `snd_pcm_close` and leak the PCM, so teardown
continues regardless and the call still succeeds. The already-closed check runs
before the `dlopen`, so re-closing a closed handle succeeds even on a host with
no alsa-lib at all.
[[src/codegen/builtins/audio/native/alsa.rs:emit_dlopen]][[src/codegen/builtins/audio/native/alsa.rs:lower_close]]

## Overloads

**`audio::close(stream AS audio::AudioInput)`**

Close a capture stream. Any buffered capture is dropped immediately; the stream
is not drained, so the call does not block. Lowers to the internal
`audio.closeInput` body and its own symbol, using `snd_pcm_drop` on Linux.
[[src/codegen/builtins/audio/mod.rs:runtime_overload_name]][[src/codegen/builtins/audio/native/macos.rs:lower_close_input]][[src/codegen/builtins/audio/native/alsa.rs:lower_close]]

**`audio::close(stream AS audio::AudioOutput)`**

Close a playback stream. Queued playback is drained to completion before
teardown, so the call blocks until the audio already written has finished
sounding. Lowers to the internal `audio.closeOutput` body and its own symbol,
using `snd_pcm_drain` on Linux.
[[src/codegen/builtins/audio/mod.rs:runtime_overload_name]][[src/codegen/builtins/audio/native/macos.rs:lower_close_output]][[src/codegen/builtins/audio/native/alsa.rs:lower_close]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `stream` | `AudioInput` or `AudioOutput` | An open capture or playback stream, from `audio::openInput`/`audio::openOutput`. Consumed by the call — the handle is moved and unusable afterward. A closed handle is a no-op. [[src/codegen/builtins/audio/mod.rs:register]][[src/syntaxcheck/builtins.rs:audio_consumes_argument]] |

## Return value

| Type | Description |
| --- | --- |
| `Nothing` | Returns once the stream has been closed — for an `AudioOutput`, not before the queued playback has finished sounding; immediately for an already-closed handle. [[src/codegen/builtins/audio/mod.rs:register]][[src/codegen/builtins/audio/native/macos.rs:lower_close_output]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050017` | `ErrAudioUnavailable` | Linux only: closing an open stream when `libasound.so.2` (or a required symbol such as `snd_pcm_drain` / `snd_pcm_drop` / `snd_pcm_close`) cannot be resolved at runtime. Only resolution failure raises — an error returned by `snd_pcm_drain`/`snd_pcm_drop` does not, so that teardown still completes. macOS never raises this, and an already-closed handle never raises it on either platform. [[src/codegen/builtins/audio/native/alsa.rs:emit_dlopen]][[src/codegen/builtins/audio/native/alsa.rs:lower_close]] |

## Examples

Close an output stream explicitly after playback:

```
IMPORT audio

SUB main()
  RES out AS audio::AudioOutput = audio::openOutput(48000, 2, 512)
  LET pcm AS List OF Byte = [0, 0, 0, 0]
  audio::write(out, pcm)
  audio::close(out)
END SUB
```

Close a capture stream, dropping any buffered audio:

```
IMPORT audio

SUB main()
  RES mic AS audio::AudioInput = audio::openInput(48000, 1, 512)
  LET pcm = audio::read(mic, 480)
  audio::close(mic)
END SUB
```

## See also

- `mfb man audio openOutput`
- `mfb man audio openInput`
- `mfb man audio write`
- `mfb man audio read`
- `mfb man audio types`
