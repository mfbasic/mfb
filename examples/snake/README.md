# snake

A bounded terminal snake game built on the `term::` TUI module.

## Build and run

```sh
mfb build examples/snake
```

- macOS / Linux — `./examples/snake/build/snake.out`
- Windows — cross-compile with `mfb build --target windows-x86_64 examples/snake`,
  then run `examples/snake/build/snake.exe`

Run it from an interactive terminal.

## Controls

- **Arrow keys** or **W/A/S/D** — turn
- **Space** — pause / resume
- **R** — restart
- **Q** — quit

The board fills the terminal and resizes with it; a resize starts a fresh
game, since the old snake may no longer fit inside the new walls.

No dependencies beyond `term`, `color`, `datetime`, `math`, `strings`,
`collections`, `io`.
