# life

Conway's Game of Life as a full-screen terminal cellular automaton, built on
the `term::` TUI module. The grid wraps at the edges (a torus), so a glider or
spaceship that runs off one side reappears on the other.

## Build and run

```sh
mfb build examples/life
```

- macOS / Linux — `./examples/life/build/life.out`
- Windows — cross-compile with `mfb build --target windows-x86_64 examples/life`,
  then run `examples/life/build/life.exe`

Run it from an interactive terminal.

## Controls

- **1–9** — load one of nine named starting patterns (glider, blinkers, toad,
  beacon, pulsar, glider-gun, spaceship, r-pentomino, acorn)
- **0** — load a random starting screen
- **Space** — pause / resume
- **R** — restart the current pattern
- **Q** — quit

The status bar at the bottom shows the live/dead cell counts, the generation
number, the current pattern, and elapsed time. Resizing the terminal is
handled live.

No dependencies beyond `term`, `color`, `datetime`, `math`, `strings`,
`collections`, `io`.
