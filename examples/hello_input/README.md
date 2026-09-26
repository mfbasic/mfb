# hello_input

Asks for the user's name, then reads single keypresses and echoes an arrow
for each of W/A/S/D until you press Q. The smallest example of a blocking
single-key read.

## Build and run

```sh
mfb build examples/hello_input
```

- macOS / Linux — `./examples/hello_input/build/hello_input.out`
- Windows — cross-compile with `mfb build --target windows-x86_64 examples/hello_input`,
  then run `examples/hello_input/build/hello_input.exe`

Run it from an interactive terminal.

## Controls

- **W / A / S / D** — print ↑ / ← / ↓ / →
- **Q** — say goodbye and exit
- any other key — reprints the usage line

No dependencies beyond `io`.
