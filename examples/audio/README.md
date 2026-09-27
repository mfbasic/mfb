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
1.5.0. It resolves to a signed source (`mfb pkg update`/`mfb build` report
`uses libsnd - [Verified]`) and ships its own per-target `libsndfile`
dynamic libraries vendored under `packages/libsnd.vendor/` once installed —
no `--unsigned` flag is ever needed for it. The vendor tree covers macOS
(aarch64) and Linux (aarch64/riscv64/x86_64, glibc and musl); there is no
`.dll`, so a Windows cross-build has not been confirmed to work.

Verified end to end from a fresh detached worktree (no prior `mfb.lock` or
`packages/`): `(cd examples/audio && mfb pkg update)` resolved and
downloaded `libsnd 1.5.0` plus its 7 vendor libraries, then `mfb build
examples/audio` built and ran cleanly.
