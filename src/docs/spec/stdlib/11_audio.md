# audio — raw PCM capture and playback

The behavioral model of the `audio` package: the frame layout, the exact-or-
timeout `read` rule, the block-until-queued `write` rule, what `available` means
in each direction, `xruns` as an event count, the non-sendable/no-duplex
consequence, the two platform backends, and the error model. The per-function
API — signatures, parameters, return types — is owned by `./mfb man audio`; this
topic specifies the behavior behind it.

## Frame layout

The only format is raw interleaved signed 16-bit little-endian PCM (`s16le`).
One **frame** is `channels * 2` bytes: for stereo, a frame is a left sample
(2 bytes) followed by a right sample (2 bytes). A byte buffer handed to
`audio::write` [[src/codegen/builtins/audio/func_write.rs:write]] must be a nonzero whole number of
frames; a buffer returned by `audio::read` [[src/codegen/builtins/audio/func_read.rs:read]] is
always whole frames. There is no file, container, codec, mixer, resampler, or
channel-conversion API at any layer.

## `read`: exactly N frames, or a timeout

`audio::read(input, frames)` blocks until exactly `frames` frames are captured
and returns `frames * channels * 2` bytes. `audio::read(input, frames, timeoutMs)`
returns early when `timeoutMs` elapses, yielding **whole frames only** — possibly
an empty list, never a partial frame, and never more than `frames`. A
`timeoutMs` of 0 polls. This rule is identical on both backends: the macOS ring
short-reads naturally and the Linux blocking `snd_pcm_readi` returns exactly the
requested count, so each backend implements the shared exact-or-timeout contract
rather than exposing its own semantics.

## `write`: block until queued

`audio::write(output, bytes)` blocks until every byte is queued for playback.
The length must be a nonzero multiple of `channels * 2`. A starved playback queue
emits silence on its own (the runtime does not enqueue silence) and counts one
underrun event.

## `render`: synthesize a note to PCM

`audio::render(note)` [[src/codegen/builtins/audio/func_render.rs:render]] is a pure MFBASIC tone
synthesizer — not a device call. It renders an `AudioNote` to mono `s16le` PCM at
48 kHz and returns it as the same `List OF Byte` layout `write` consumes, so a
rendered tone plays with no conversion. It opens no hardware and never raises.
Unlike the native surface, `render` and its two value records (`AudioEnvelope`,
`AudioNote`) live in the package's MFBASIC source companion,[[src/codegen/builtins/audio/mod.rs]]
injected on `IMPORT audio` exactly like
`net`'s `Url`. `AudioEnvelope` and `AudioNote` are ordinary value records the
program constructs (`AudioEnvelope[...]`, `AudioNote[...]`) — unlike the
device-owned `AudioDevice`.

A note is a sine at `frequencyHz` held for `noteFrames` frames, shaped by a linear
ADSR `AudioEnvelope` (amplitudes in raw `0..32767` sample units) and scaled by
`gainOverall` (`0..1`): a linear attack to peak (32767), a linear decay to
`sustainLevel`, a sustain across the middle, and a linear release over the final
`releaseFrames`. Every sample is clamped to the `s16` range and encoded
little-endian; the result is `noteFrames * 2` bytes.

## `play`: an MML sequencer

`audio::play(output, mml)` [[src/codegen/builtins/audio/func_play.rs:play]] plays music written in
**MML** (Music Macro Language) — a small source-companion sequencer, overloaded by
its second argument on a single `String` track or a `List OF String` of tracks.
It pre-renders every track to mono `s16le` PCM at 48 kHz, mixes them (summing with
clamping), and writes the audio to `output` — a pointer to an open `AudioOutput`
that the caller's scope owns and closes (open it at 48 kHz mono). Malformed MML raises
`ErrInvalidArgument` (`7-705-0002`) *before* anything is written; the strings are
validated at the call.

A track is a string of **whitespace-separated tokens** — every token must be
separated by whitespace (`C E G`, never `CEG`). Each track is fully isolated: the
tempo, length, octave, volume, and instrument are per-track state that never
carries between tracks. The tokens are notes `A`–`G` (with a `+`/`-` accidental, an
inline length, and trailing dots), `R` (a rest of the current length) and
`P1`–`P64` (a pause of a given length), `O0`–`O6`/`<`/`>` (octave), `L1`–`L64`
(default length), `T32`–`T255` (tempo), `V0`–`V10` (volume), `I <name>`
(instrument — `square`/`triangle`/`sine`/`saw`/`noise`, the name a separate token),
`( … )` legato and `[ … ]` staccato (neither may nest), and `{ … }<count>` repeat
(count `>= 1`, attached to the closing brace, may nest). A track is refused with
`ErrInvalidArgument` when its repeats expand past 65,536 tokens, or when it would
play for longer than ten minutes. `O4` is the octave of A440. Like `render`, `play` and the sequencer live in the MFBASIC source companion.

## `available` and `poll`

`audio::available(stream)` [[src/codegen/builtins/audio/func_available.rs:available]] returns the frames
that can move immediately without blocking: readable frames for an `AudioInput`,
writable frames for an `AudioOutput`. Per the timeout convention, `audio::poll(stream)`
(omitted timeout) **blocks** until that condition holds and then returns `TRUE`;
`audio::poll(stream, 0)` is the immediate `available(stream) > 0` check; and the
timed overload waits up to `timeoutMs` for the condition. Both meanings are
identical across the two backends.

## `xruns`: events, not frames

`audio::xruns(stream)` [[src/codegen/builtins/audio/func_xruns.rs:xruns]] is a monotonic count of
xrun **events** since the stream opened — capture overruns for an `AudioInput`,
playback underruns for an `AudioOutput` — incremented by exactly one per event.
It counts events, not lost frames: `xruns() > 0` means audio was lost and the
amount is unknowable, because ALSA does not report how many frames an overrun or
underrun destroyed. An event count is exact on both platforms; a frame count
would be truthful on macOS and fabricated on Linux.

## Direction is in the type; no duplex, no threads

`AudioInput` and `AudioOutput` are separate move-only resource types
[[src/codegen/builtins/audio/mod.rs:AUDIO_INPUT_TYPE]]. `read` is defined only over
`AudioInput` and `write` only over `AudioOutput`, so a swapped stream is a
**compile** error caught by overload resolution, never a runtime check. This
follows the hardware: no operating system in scope has a duplex stream handle
(`AudioQueueNewInput`/`AudioQueueNewOutput` are separate objects; ALSA's
`snd_pcm_open` takes one direction), so full duplex is always two handles.

Both types are **non-sendable**: neither can cross a thread boundary, so a
program cannot run capture on one thread and playback on another. Single-threaded
duplex is expressible — open one stream of each direction and drive both from one
loop with `poll`, `available`, and timed `read`. That is why those three calls
exist. `audio::devices()` [[src/codegen/builtins/audio/func_devices.rs:devices]] returns no channel
counts or supported rates: a caller discovers a working configuration by
attempting to open the device and handling the error.

## Backends

macOS drives Core Audio's `AudioQueue`, whose callbacks run on an ordinary
internal thread where taking a mutex is legal. Linux drives ALSA's blocking
`snd_pcm_readi`/`snd_pcm_writei` directly on the calling thread, with no callback
thread. Neither uses a lock-free ring, because this compiler emits no atomic or
barrier instructions on any architecture; all cross-thread synchronization is
`pthread_mutex`/`pthread_cond`. On Linux, `libasound.so.2` is resolved lazily
with `dlopen` — never a `DT_NEEDED` — so a binary that imports `audio` still
starts where alsa-lib is absent, and every `audio::` call there raises
`ErrAudioUnavailable`.

## Error model

- `ErrAudioUnavailable` (`7-705-0017`) — the audio backend library or a device is
  unavailable: no `libasound.so.2`, no audio device, or capture authorization
  denied.
- `ErrAudioDevice` (`7-705-0018`) — an audio device open, configuration, or
  stream operation failed.
- `ErrInvalidArgument` (`7-705-0002`) — a parameter is out of range: `sampleRate`
  outside `8000..192000`, `channels` other than 1 or 2, `bufferFrames` outside
  `64..8192`, `read` `frames` outside `1..1048576`, a **negative** `timeoutMs`
  (a too-large `timeoutMs` is clamped to `2147483647`, not rejected — the language
  timeout convention), or a `write` length that is not a nonzero whole number of
  frames.

The two audio codes are registered in the constant registry by the diagnostics
topic; see `mfb spec diagnostics error-codes`.

## See Also

* ./mfb man audio — the per-function API reference
* ./mfb spec diagnostics error-codes — the ErrAudio* constants
* ./mfb spec language builtin-functions — the import-gated package set
