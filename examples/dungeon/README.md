# dungeon

A nethack-style dungeon of rooms and halls, drawn isometrically from tiles, on
a windowed `--app` build.

## Build and run

Canvas needs a window, so this is a `--app` build:

```sh
mfb build examples/dungeon
```

- macOS — `open examples/dungeon/build/dungeon.app`
- Linux — `./examples/dungeon/build/dungeon.AppImage`
- Windows — cross-compile with `mfb build --target windows-x86_64 examples/dungeon`,
  then run `examples/dungeon/build/dungeon.exe`

## Controls

- **W A S D** — move up-right, up-left, down-left, down-right on the screen
  (relative to the current view rotation)
- **Q** — rotate the view counterclockwise
- **E** — rotate the view clockwise
- **Esc** — quit

## Layout

The program is split across `map.mfb` (map format, directions, movement),
`generate.mfb` (dungeon generation from a seed), `render.mfb` (the isometric
view and tile scene), `input.mfb` (raw key reading), and `main.mfb` (the key
→ action mapping and the draw/move loop).

## Tiles

The isometric tile set under `resources/dungeon/` is from Kenney (kenney.nl),
CC0-licensed — see [`CREDITS.md`](CREDITS.md) for the source pack and the
license.
