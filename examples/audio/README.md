# audio

A console menu that plays synthesized MML tunes on the `audio::` package, and
two Ogg files decoded through the `libsnd` package (via `libsnd::openSound` /
`libsnd::loadFrames`) — a single-track tune, a three-part round, and two
recordings, on the output device you pick.

## Build and run

The `libsnd` package (see Dependency, below) is not tracked in git — `mfb
pkg update` resolves and downloads it, writing `examples/audio/mfb.lock` and
`examples/audio/packages/`. On a fresh checkout this step needs network
access to the package registry:

```sh
(cd examples/audio && mfb pkg update)
mfb build examples/audio
```

A checkout that already has `examples/audio/mfb.lock` and
`examples/audio/packages/libsnd.mfp` (e.g. this repo's own working tree,
where the package was resolved once already) can skip straight to `mfb build
examples/audio` — `mfb build` only complains about a missing package, it
does not resolve one itself.

- macOS / Linux — `./examples/audio/build/audio.out`
- Windows — `mfb build --target windows-x86_64 examples/audio`, then copy
  `examples/audio/build/` (`audio.exe`, `music/`, `vendor/`) to the Windows
  machine and run `audio.exe` there. `vendor/libsnd-sndfile.dll` must stay
  beside it.

## Usage

The program is interactive: it lists output devices (auto-selecting when
there is only one) and then loops a menu:

```
1) Twinkle Tinkle Little Star (Play - Single Track)
2) Row-Row-Row You Boat (Play - Multi Track)
3) Mozart 1 (libsnd - Ogg)
4) Mozart 2 (libsnd - Ogg)
Q) Quit
```

Type a number to play that track (each blocks until it finishes) or `Q` to
exit. Each tune opens its own 48 kHz stream and closes it when it ends, and
each Ogg file opens one at the file's own rate: on Windows a stream at a rate
other than the device's takes the device exclusively and ends every other
stream on it, so nothing is held open across menu choices.

## Dependency: the `libsnd` package

`project.json` declares a pinned registry package, `jzaun#libsnd` version
1.6.0. It resolves to a signed source (`mfb pkg update`/`mfb build` report
`uses libsnd - [Verified]`) and ships its own per-target `libsndfile`
dynamic libraries vendored under `packages/libsnd.vendor/` once installed —
no `--unsigned` flag is ever needed for it. The vendor tree covers macOS
(aarch64), Windows (x86_64, `sndfile.dll`) and Linux (aarch64/riscv64/x86_64,
glibc and musl): `mfb pkg update` downloads 8 vendor libraries.
