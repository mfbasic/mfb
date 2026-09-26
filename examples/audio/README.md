# audio

A console menu that plays synthesized MML tunes on the `audio::` package, and
two Ogg files decoded through the `libsnd` package (via `libsnd::openSound` /
`libsnd::loadFrames`) — a single-track tune, a three-part round, and two
recordings, on the output device you pick.

## Build and run

```sh
mfb build examples/audio
```

- macOS / Linux — `./examples/audio/build/audio.out`

No Windows build is documented here — see the vendoring caveat below.

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
exit.

## Dependency: the `libsnd` package

`project.json` declares a pinned registry package, `jzaun#libsnd` version
1.5.0. It resolves to a signed source (`mfb build` reports `uses libsnd -
[Verified]`) and ships its own per-target `libsndfile` dynamic libraries
vendored under `packages/libsnd.vendor/` once the package is installed — a
plain `mfb build examples/audio` resolves and builds with no `--unsigned`
flag needed. The vendor tree covers macOS (aarch64) and Linux
(aarch64/riscv64/x86_64, glibc and musl); there is no `.dll`, so a Windows
cross-build has not been confirmed to work.
