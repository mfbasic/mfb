# wide-demo

The plan-70 Unicode-wide-character parity proof: draws a bordered box around
CJK, astral (emoji), and multi-scalar (NFD combining, ZWJ family) content. On
a Unicode-correct backend the box borders stay square around the content —
every wide grapheme occupies two columns, every combining/ZWJ cluster one —
and nothing straddles a cell or tears across the border. That alignment is
the proof; it runs once and exits.

## Build and run

Builds as either a console program or a windowed app:

```sh
mfb build examples/wide-demo         # console
mfb build --app examples/wide-demo   # windowed (macOS / Linux GTK / Windows GDI)
```

- macOS / Linux (console) — `./examples/wide-demo/build/wide-demo.out`
- macOS (app) — `open examples/wide-demo/build/wide-demo.app`
- Linux (app) — `./examples/wide-demo/build/wide-demo.AppImage`
- Windows — cross-compile with `mfb build --target windows-x86_64 examples/wide-demo`,
  then run `examples/wide-demo/build/wide-demo.exe`

## Controls

- any key — dismiss the panel; the program then prints the measured display
  width of every line (in columns) and exits

No dependencies beyond `term`, `color`, `strings`, `collections`, `io`.
