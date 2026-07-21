# libsnd playback — a MANUAL hardware fixture

This is **not** part of the acceptance suite, and deliberately has no `golden/`.

It is the plan-58-D Phase 3 deliverable: decode a FLAC with `libsnd::loadSound`
and play it through `audio::write`. Two things make it unsuitable for automated
running:

- **It needs a real audio output device.** The CI/VM hosts have `/dev/snd` but no
  usable PCM device, so `audio::openOutput` fails there — an environment result,
  not a code result.
- **bug-370.** `audio::close` on macOS intermittently never returns (~2 runs in
  6), so an automated runner would hang rather than fail.

## Running it

```
mfb build tests/rt-behavior/native/libsnd-playback-rt
tests/rt-behavior/native/libsnd-playback-rt/build/libsnd_playback_rt.out
```

Expected output, and then a 0.25 s 440 Hz tone:

```
rate=44100
channels=2
bytes=44100
frames=11025
frame_aligned=TRUE
played=TRUE
```

A run that prints through `frame_aligned=TRUE` and then stops without `played=TRUE`
has hit bug-370; the audio still sounded.

For a cross-target run, build with `-target linux-<arch>` and ship the executable,
`build/vendor/libsnd-libsndfile.*`, and `build/tone.flac` together.

## Results as of 2026-07-20

| target | decode | playback |
|---|---|---|
| macos aarch64 | ok | audible |
| linux aarch64 glibc (kali) | ok | `played=TRUE` |
| linux aarch64 glibc (arch) | `dlopen` failed — box lacks libFLAC/libogg/libvorbis/libopus | — |
| linux aarch64 musl (alpine) | ok | no PCM device |
| linux x86_64 musl (alpine) | ok | no PCM device |
| linux riscv64 musl (alpine) | ok | no audio device |
| linux x86_64 glibc | UNTESTED — host unreachable | — |
| linux riscv64 glibc | UNTESTED — host unreachable | — |
