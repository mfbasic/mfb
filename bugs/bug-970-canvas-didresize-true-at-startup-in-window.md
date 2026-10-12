# bug-970: `canvas::didResize()` answers TRUE once at startup in a real window although the surface never changed size (macOS and GTK)

Last updated: 2026-10-12
Effort: small (<1h)
Severity: LOW
Class: Correctness

Status: Open
Coordinator note: measured by the review agent only (Linux runs in podman, or a macOS window screenshot); the coordinator did not re-run it.
Regression Test: (proposed) non-headless run (or a scripted first publish of 900×640) of a program polling `didResize` after `setMode(Canvas)` reports FALSE until a real size change

`canvas::didResize` promises TRUE "the first time it is called after the surface's
dimensions change" and none for "a platform event that re-publishes the size it
already had". In a real window the canvas view's first `setFrameSize:` (macOS) / first
allocation (GTK) publishes 900×640 over the graphics state's initial `0×0`, which the
change test counts as a resize — but `canvas::getSize()` already reported 900×640
before it (it falls back to `DEFAULT_SURFACE_WIDTH/HEIGHT` while the word is 0). So the
program sees no size change yet gets a resize. `tests/canvas/rt_canvas_damage.rs`
names this exact hazard ("would make every program lay out twice at startup") but
runs headless only, where the size is never published from 0.

The single correct behavior: the first publish that equals what `getSize` already
reported is not a resize.

References:

- `src/docs/spec/app/06_canvas.md`; `mfb man canvas didResize`.
- `.ai/canvas-threading.md` "`canvas::didResize` is a counter pair, not a flag".

## Re-verified 2026-10-12: still reproduces on macOS (HEAD `555130a25`)

`bugs/repro/bug-970/did_resize.mfb` (the program below, writing to stderr), `mfb build --app`:

```
real window:  start size=900.00x640.00 didResize=TRUE     later … didResize=FALSE
headless:     start size=900.00x640.00 didResize=FALSE    later … didResize=FALSE
```

`emit_publish_surface_size_tagged` still compares the new size against the stored words,
which start at 0, and `func_graphics.rs:lower_dimension` still answers
`DEFAULT_SURFACE_WIDTH`/`HEIGHT` for 0. GTK and Windows were not re-run.

## Failing Reproduction

`/tmp/hardening/B17/dr1`:

```
  app::setMode(app::Mode.Canvas)
  LET s0 AS canvas::Size = canvas::getSize()
  fs::appendText(path, "start size=" & toString(s0.width) & "x" & toString(s0.height) & " didResize=" & toString(canvas::didResize()) & "\n")
```

```
mfb build --app dr1; dr1/build/p1.app/Contents/MacOS/p1; cat dr1.log
MFB_MACAPP_HEADLESS=1 dr1/build/p1.app/Contents/MacOS/p1; cat dr1.log
```

- Observed GUI: `start size=900x640 didResize=TRUE`; headless: `didResize=FALSE`.
- GTK (`/tmp/hardening/B17/cal`, Xvfb, no resizes performed): `resizes=1 ... size=900x640`.
- Expected: FALSE in all three.

## Root Cause

`src/codegen/runtime/canvas/mod.rs:emit_publish_surface_size_tagged` bumps
`GRAPHICS_OFFSET_RESIZES` whenever the published size differs from the stored words,
which start at 0, while `src/codegen/builtins/canvas/func_graphics.rs:lower_dimension`
reports the default for 0. The comparison should be against the *effective* size
(default when 0), or the first publish should seed without counting.

## Goal

- No spurious TRUE at startup; real resizes still counted once each (today: 60
  scripted resizes → `resizes=61` on macOS `/tmp/hardening/B17/ca1`, 40 → 41 on GTK —
  exactly one extra each).

## Blast Radius

- macOS `MFBCanvasView setFrameSize:`, GTK resize handler, Windows `WM_SIZE` — all use
  the shared publisher (Windows not run).
