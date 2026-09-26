# emoji

Two faces — 🙂 and 🙁 — drawn entirely from `canvas` primitives (no image
files) on a windowed `--app` build. Demonstrates a radial and a linear
gradient fill, an ellipse, and a round-capped arc.

## Build and run

Canvas needs a window, so this is a `--app` build:

```sh
mfb build examples/emoji
```

(`"mode": "app"` in `examples/emoji/project.json` is what makes a plain
`mfb build` produce an app; a console build cannot `IMPORT app`/`canvas`.)

- macOS — `open examples/emoji/build/emoji.app`
- Linux — `./examples/emoji/build/emoji.AppImage`
- Windows — cross-compile with `mfb build --target windows-x86_64 examples/emoji`,
  then run `examples/emoji/build/emoji.exe`

## Controls

- any key — switch mood (🙂 ↔ 🙁)
- **Q** — quit

The scene is presented once per change (a mood switch or a resize), not once
per frame — `canvas::present` installs a retained scene the runtime keeps
drawing on its own.

No dependencies beyond `app`, `canvas`, `color`, `collections`, `math`,
`strings`, `io`.
