# bug-970: `canvas::didResize()` answers TRUE once at startup in a real window although the surface never changed size (macOS and GTK)

Last updated: 2026-10-12
Effort: small (<1h)
Severity: LOW
Class: Correctness

Status: Fixed (bs23-agent-a)
Coordinator note: measured by the review agent only (Linux runs in podman, or a macOS window screenshot); the coordinator did not re-run it.
Regression Test: `codegen::runtime::canvas::tests::publishing_the_size_the_program_already_sees_counts_no_resize` (the publisher's instructions, executed), `rt_canvas_damage::a_first_publish_of_the_startup_size_is_not_a_resize` and `a_first_publish_of_another_size_is_one_resize` (scripted first publish; GTK and Windows hosts), `scripts/test-macapp.sh` case 3d3 (a real macOS window, `MFB_MACAPP_GUI=1`)

## Cause (coordinator, 2026-10-12, bug-staging-23)

- Rule: `.ai/canvas-threading.md` "`canvas::didResize` is a counter pair": the counter is bumped "only when the size actually changed", and `mfb man canvas didResize` promises a change of the size the program can see.
- Place: `runtime/canvas/mod.rs:emit_publish_surface_size_tagged` compares the new size against the raw stored words, which start at 0, while the size the program sees (`emit_surface_dimension`, used by `func_graphics.rs:lower_dimension`) reads 0 as `DEFAULT_SURFACE_WIDTH`/`HEIGHT`. One word, two meanings of 0.
- Fix: the publisher compares against the EFFECTIVE old size, through the same 0-means-default reading the getter uses (one shared definition, not a second copy of the constants). A first publish of 900x640 is then no change; a first publish of any other size (Windows' 384x261 client) still counts, correctly. Nothing is special-cased as "first".

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

## Resolution (bs23-agent-a, 2026-10-12)

The Cause note is right. `runtime/canvas/mod.rs` now has one reading of a size word,
`SurfaceDimension::emit_load` (load the word; zero reads as the startup size), with the
word's offset and its startup value paired in `SurfaceDimension` and nowhere else.
`emit_surface_dimension` (what `canvas::getSize` and the renderer read) and
`emit_publish_surface_size_tagged` both go through it, so the publisher compares the new
size against the size the program sees. Nothing is special-cased as "first": a first
publish of 900 by 640 is no change, and a first publish of any other size is one.

Register budget: unchanged. The publisher still writes only `base` and `SCRATCH[5]`
(`x14`; `r10` on x86-64), which it already wrote at every site, and reads `width` and
`height`. The five callers pass `LOCAL[1..3]` (macOS `setFrameSize:`), `LOCAL[0..2]`
(GTK resize helper, Windows headless resize) and `SCRATCH[0..2]` (Windows `WM_SIZE` and
the canvas reconcile); none is `SCRATCH[5]`. The getter's instructions are the same as
before (same label, same order).

### Measured

- Real macOS window, `bugs/repro/bug-970/did_resize.mfb`, `mfb build --app`, run without
  `MFB_MACAPP_HEADLESS`: before `start size=900.00x640.00 didResize=TRUE`, after
  `start size=900.00x640.00 didResize=FALSE` (then `later ... FALSE`, `again ... FALSE`
  both times). `scripts/test-macapp.sh` case 3d3 is that run as a row: FAIL with
  `/tmp/main-dbg-target/release/mfb`, `ok` with the fix.
- Scripted first publish, `rt_canvas_damage.rs:RESIZE_POLL` cross-built and run headless
  with `MFB_CANVAS_SYNC=1` and `MFB_CANVAS_RESIZE_W`/`_H`:

  | box | publish | before | after |
  |---|---|---|---|
  | 206, windows-x86_64 | 900 x 640 | `first:TRUE resizes:0` | `first:FALSE resizes:0` |
  | 206, windows-x86_64 | 640 x 480 | `first:TRUE resizes:0` | `first:TRUE resizes:0` |
  | 2226, linux-aarch64 (GTK, glibc AppImage) | 900 x 640 | `first:TRUE resizes:0` | `first:FALSE resizes:0` |
  | 2226, linux-aarch64 | 640 x 480 | `first:TRUE resizes:0` | `first:TRUE resizes:0` |

  So a real resize is still reported once, and the publish of the size already seen is
  not. `--target linux-x86_64` builds; it was not run (boxes 2227 and 2228 are down).
- The unit test runs the publisher's emitted instructions over a zeroed state: 900 x 640
  counts 0, a repeat 0, 640 x 640 one, 640 x 480 two, a repeat two, back to 900 x 640
  three; a first publish of 384 x 261, 900 x 261 or 384 x 640 counts one. Before the fix
  it failed at the first step (`left: 1, right: 0`).

A Windows real window (first client area 384 x 261) was not run.

### Golden impact

`scripts/artifact-gate.sh target/release/mfb all`: 4 diffs, the same four
`syntax/app/app-mouse-surface/app_mouse_surface.<target>.app.ncode` goldens bug-968
moves. Each holds the publisher (`grep -c resize_old_width`: 2 labels on macos-aarch64,
linux-aarch64 and linux-x86_64, 6 on windows-x86_64, which publishes from three sites).
No other fixture moved. Goldens were not regenerated.

