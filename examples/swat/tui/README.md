# swat-tui

Swat — click the bugs, spare the grass. A mouse-driven game built on the
`term::` TUI module, drawn with emoji glyphs. See
[`examples/swat/canvas`](../canvas) for the shape-drawn edition of the same
game.

## Build and run

```sh
mfb build examples/swat/tui
```

- macOS / Linux — `./examples/swat/tui/build/swat-tui.out`
- Windows — cross-compile with `mfb build --target windows-x86_64 examples/swat/tui`,
  then run `examples/swat/tui/build/swat-tui.exe`

Run it from a terminal that supports mouse input.

## Rules

Grass sprouts at random (at most `level * 10` plants at once; the oldest
wilts when a new one would go over that). A bug appears somewhere on the
field and stays for `10 / level + 1` seconds before scurrying off, returning
somewhere new 5–10 seconds later. Clicking the bug scores +1 and splats it;
clicking grass scores −2. A score below zero ends the game; every 15 points
is another level, each with its own bug and plants. After the tenth level the
cycle starts again.

## Controls

- **Mouse click** — swat whatever is under the cursor
- **Esc** — pause (press again to resume)
- **R** — restart
- **Q** — quit

Resizing the terminal clears the field, since what was on it may no longer
fit inside the new border.

No dependencies beyond `term`, `color`, `datetime`, `math`, `strings`,
`collections`.
