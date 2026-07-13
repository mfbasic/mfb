# audio

Raw interleaved `s16le` PCM capture and playback

## Synopsis

```
IMPORT audio
LET devices = audio::devices()
RES out AS AudioOutput = audio::openOutput(48000, 2, 512)
audio::write(out, pcmBytes)
audio::close(out)
RES mic AS AudioInput = audio::openInput(48000, 1, 512)
LET frames = audio::read(mic, 4800)
audio::close(mic)
```

## Description

The `audio` package moves raw interleaved signed 16-bit little-endian PCM
(`s16le`) through the operating system's audio hardware. It enumerates devices,
opens a capture or playback stream, moves whole frames of PCM through it, and
closes it. There is no audio file, container, codec, mixing, resampling, or
channel-conversion API at any layer — the only format is `s16le`, and one frame
is `channels * 2` bytes.

Direction is part of the type. `audio::openInput` returns an `AudioInput` and
`audio::openOutput` returns an `AudioOutput`; `audio::read` is defined only over
`AudioInput` and `audio::write` only over `AudioOutput`, so passing the wrong
stream is a compile error rather than a runtime one. This mirrors the hardware:
no operating system in scope has a duplex stream handle, so full duplex means
opening one stream of each direction and driving both from a single loop with
`audio::poll`, `audio::available`, and the timed form of `audio::read`.

Both stream types are move-only, non-sendable resource handles: neither can
cross a thread boundary, so a program cannot run capture on one thread and
playback on another. Each is closed automatically by lexical drop when its
binding leaves scope, or explicitly with `audio::close`. `AudioDevice` is a
plain read-only record obtained only from `audio::devices()`.

`audio::xruns` counts overrun (capture) and underrun (playback) **events**, not
lost frames — `xruns() > 0` means audio was lost and the amount is unknowable,
because ALSA cannot report how many frames an xrun destroyed. `audio::devices()`
reports no channel counts or supported sample rates: a caller discovers a working
configuration by attempting to open a stream and handling the error.

## Platform availability

macOS drives Core Audio's `AudioQueue`; Linux drives ALSA's blocking PCM API
through a `libasound.so.2` resolved at runtime with `dlopen` — so a binary that
imports `audio` still starts on a Linux host without alsa-lib, and every
`audio::` call there raises `ErrAudioUnavailable`. A program that does not
`IMPORT audio` gains no audio symbol and no dynamic-library dependency.

## Members

- `audio::devices` — enumerate the audio devices
- `audio::openInput` — open a capture stream
- `audio::openOutput` — open a playback stream
- `audio::read` — capture PCM frames
- `audio::write` — play PCM frames
- `audio::poll` — test a stream for readiness
- `audio::available` — frames readable/writable without blocking
- `audio::xruns` — cumulative overrun/underrun event count
- `audio::close` — close a stream
- `audio::render` — synthesize an `AudioNote` to raw PCM
- `audio::play` — parse and play MML music

## See also

- `mfb man audio types`
- `mfb man audio devices`
- `mfb man audio openOutput`
- `mfb man audio read`
