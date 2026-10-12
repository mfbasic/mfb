# bug-1080: a defaulted record updated with `WITH` costs about 27 KB of machine code a statement

Last updated: 2026-10-11
Effort: unknown — not root-caused
Severity: LOW — the program is right; its executable is large and slower to build
Class: Other

Status: Open
Regression Test: none

Building one `canvas::Rectangle` the way `mfb man canvas` recommends — `MUT r AS
canvas::Rectangle`, then `r = WITH r { … }`, then hand it to a scene — compiles to
about 27,000 bytes of code in the function that does it, and the cost per item GROWS
with the number of items in the function. A `main` that builds ten is 371 KB.

**The single correct behavior a fix produces:** the function sizes in the table below
grow linearly in the number of items, at a few hundred bytes each rather than tens of
thousands.

References:

- `bugs/repro/bug-1080/with_5.mfb`, `with_10.mfb` — the programs measured.
- `tests/canvas/scenes/picture_shapes_on_black.mfb` — where it was noticed: a scene of
  fifteen items written this way has a 1,144,024-byte `main`, a third of the
  executable's code.
- `mfb man canvas types`, `canvas::Rectangle` — the description that recommends the
  pattern ("`MUT box AS canvas::Rectangle`, then `WITH box { … }`").
- Found while measuring plan-172-L's code-size cost; not caused by it (the pattern is
  plan-172-E's, and the functions measured hold no picture).

## Repro

Build each program with `--debug`, which writes function names into the symbol table,
and read `main`'s size as the distance to the next symbol:

```
mfb build -app --debug <project>
nm -n <project>/build/<name>.app/Contents/MacOS/<name> | grep -A1 ' _mfb_fn_main$'
```

Each item in the program is these four lines:

```
MUT r0 AS canvas::Rectangle
r0 = WITH r0 { position := vector::Float2[x := 0.0, y := 10.0], size := canvas::Size[width := 20.0, height := 20.0], paint := canvas::fill(color::rgb(255, 255, 255)) }
LET i0 AS canvas::DrawItem = r0
scene = collections::append(scene, i0)
```

| Items in `main` | `_mfb_fn_main`, bytes | Per item |
|---|---|---|
| 1 | 26,664 | 26,664 |
| 2 | 54,480 | 27,240 |
| 5 | 152,260 | 30,452 |
| 10 | 371,080 | 37,108 |

Measured on macOS AArch64 with the compiler at `b83b09637`
(`/tmp/plan-172-A/with_size.sh`, which generates the four programs).

## What is not known

Which of the four lines the bytes belong to — the default construction of a record
that holds a `Paint` (itself two lists and a nested `Gradient`), the `WITH` copy, the
conversion to the `DrawItem` union, or the append — and why the per-item cost rises
with the count, which suggests something proportional to the number of live records in
the function (clean-up paths, most likely) rather than to the statement.

## First steps

1. Drop the lines one at a time from the one-item program and re-measure, to find
   which carries the 26 KB.
2. Compare with the full-literal form (`canvas::Rectangle[id := "", …]`), which most
   of the test scenes use: if it is small, the cost is the default-then-`WITH` pair.
3. `mfb build --ncode` on the one-item program and look at what repeats.
