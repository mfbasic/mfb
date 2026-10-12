# bug-968: canvas frame skip republishes an identical scene when it contains a nested list (Polygon points, DrawLayer items) built by a different allocation history

Last updated: 2026-10-12
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness

Status: Fixed (bs23-agent-a)
Regression Test: `rt_canvas_graphics_thread::content_equal_scenes_with_nested_lists_draw_one_frame` (scene `tests/canvas/scenes/frame_skip_nested_lists.mfb`), `rt_canvas_present_leak::re_presenting_a_rebuilt_unchanged_scene_holds_steady_memory`, and three unit tests in `builder_collection_compare.rs`

## Cause (coordinator, 2026-10-12, bug-staging-23)

- Rule: `mfb spec app canvas` "Retained, not immediate": the frame skip compares "the two scenes' contents, element for element ... CONTENT, not whole blocks" (`.ai/canvas-threading.md` 3.1). `mfb spec memory collections` says why bytes cannot stand in for content: shrink-to-fit is "not recursively" applied, and an embedded block keeps its stored capacities.
- Place: `gen_present.rs:emit_publish` compares `count`, `dataLength` and the outer data region byte for byte, so it reads every nested block's allocation history.
- Measured (`mfb dbg`, `bp _mfb_rt_canvas_canvas_publishScene show mem x0 480` on `bugs/repro/bug-968/frame_skip.mfb`): two `[poly(lit)]` scenes differ in exactly 2 words, the 7 reserved bytes after each fresh list entry's 1-byte `flags` (never written; `keyOffset`/`keyLength` ARE 0, so the comment in `emit_publish` names the wrong words); the `mkpts()` scene also carries the appended list's `capacity` 8 against `count` 3, and its five spare entries.
- Fix: compare content through the entries, element by element, by the scene's TYPE (one value-equality emitter beside `emit_comparable_values_match_branch`, gaining the List and union arms it lacks; bytes first per element, the typed walk only where bytes differ). That deletes the `count`/`dataLength`/data-base arithmetic in `emit_publish` and its wrong comment. No hash; no canvas item names in the comparison.

`canvas::present` / `presentLayers` promise that re-presenting identical content
publishes nothing, so an animation loop that rebuilds an unchanged frame does not
redraw. That holds for a flat list of fixed-size items, but not when an item carries a
nested collection: two scenes with byte-for-byte equal *content* compare unequal and
each present draws a frame.

Measured with `MFB_CANVAS_SYNC=1` (frame counter after each call):

| call | frames |
|---|---|
| `present([poly])` (LET-bound polygon, literal points) ×2 | 1, 1 ✓ |
| `present([Polygon[points := pts]])` (LET pts) ×2 | 1, 1 ✓ |
| `present([Polygon[points := mkpts()]])` (points built by `collections::append`) ×2 | 2, 3 ✗ |
| `present([LET-bound poly from mkpts()])` | 4 ✗ |
| `presentLayers(one)` (LET-bound layers) ×2 | 1, 1 ✓ |
| `presentLayers([DrawLayer[items := [a]]])` inline ×2 (same content as `one`) | 2, 3 ✗ |
| flat `present(scene())` built by `append` ×3 | 1, 1, 1 ✓ |

The single correct behavior: equal scene content ⇒ no publish, independent of how the
nested lists were built.

References:

- `src/docs/spec/app/06_canvas.md` "Retained, not immediate": "Re-presenting identical
  content is free of publication ... The comparison is exact rather than hashed: both
  sides are shrink-to-fit copies, so equal content is equal bytes"; "Both calls share
  one implementation and therefore one set of guarantees".
- `.ai/canvas-threading.md` §3.1 "The frame skip compares CONTENT, not bytes".

## Re-verified 2026-10-12: still reproduces (HEAD `555130a25`, after plan-172 and plan-164)

The program below no longer compiles: plan-172 changed the item records (`Polygon` takes
`contours`, `rule`, `id`, `position` and `transform`; a point is `vector::Float2`). The
same cases in today's syntax are `bugs/repro/bug-968/frame_skip.mfb`. Built with
`mfb build --app` and run with `MFB_MACAPP_HEADLESS=1 MFB_CANVAS_SYNC=1`, the frame counter
after each call:

| call | frames | |
|---|---|---|
| `present([bound])`, one LET-bound polygon, twice | 1, 1 | ✓ |
| `present([Circle[…]])` written inline, twice | 2, 2 | ✓ |
| `present([poly(lit)])`, polygon built by a function from LET-bound literal points, twice | 3, 4 | ✗ |
| `present([poly(mkpts())])`, points built by `collections::append`, three times | 5, 6, 7 | ✗ |
| `presentLayers(one)`, LET-bound layers, twice | 8, 8 | ✓ |
| `presentLayers([DrawLayer[items := [a]]])` inline, twice | 9, 10 | ✗ |

`gen_present.rs:emit_publish` has not changed since bug-686 (`git log -- src/codegen/builtins/canvas/gen_present.rs`:
newest commit `e5e8167ef`, 2026-09-24). The root-cause hypothesis is still not instrumented.
One row is new: a polygon rebuilt by a function from the *same* LET-bound literal points
also redraws, so a different allocation history is enough even with no `append`.

## Failing Reproduction

`/tmp/hardening/B17/fsl` (polygon variant) and the layers variant:

```
FUNC mkpts() AS List OF canvas::Point
  MUT pts AS List OF canvas::Point = []
  pts = collections::append(pts, canvas::Point[x := 1.0, y := 1.0])
  pts = collections::append(pts, canvas::Point[x := 9.0, y := 1.0])
  pts = collections::append(pts, canvas::Point[x := 5.0, y := 9.0])
  RETURN pts
END FUNC
...
  app::setMode(app::Mode.Canvas)
  LET p AS canvas::Paint = canvas::fill(color::rgb(255, 0, 0))
  canvas::present([canvas::Polygon[points := mkpts(), paint := p]])
  io::print(toString(canvas::getFrameStats().frames))     ' 1 expected
  canvas::present([canvas::Polygon[points := mkpts(), paint := p]])
  io::print(toString(canvas::getFrameStats().frames))     ' 1 expected, prints the next count
```

```
mfb build --app fsl && MFB_MACAPP_HEADLESS=1 MFB_CANVAS_SYNC=1 fsl/build/p1.app/Contents/MacOS/p1
```

- Observed: each identical present increments `frames`.
- Expected: the second present is skipped.

## Root Cause

`src/codegen/builtins/canvas/gen_present.rs:emit_publish` compares `count`,
`dataLength` and the outer list's **data region** byte-wise. A nested collection
(the polygon's `points`, a layer's `items`) is inlined *inside* that data region with
its own header and lookup entries. Hypothesis (not instrumented): `copy_flat_block` /
`copy_collection_tight` tightens only the outer block, so the inner block keeps its
allocation-dependent bytes — the unwritten `keyOffset`/`keyLength` words of each list
lookup entry (the exact junk the outer-level comment in `emit_publish` describes) and/or
capacity slack — and the byte compare sees them. Confirm by dumping both copies'
data regions for the `mkpts()` case.

## Goal

- Content-equal scenes with nested lists skip, for both `present` and
  `presentLayers`.

### Non-goals (must NOT change)

- No hashing (no collision reasoning); content-different scenes still publish.

## Blast Radius

- Every variant with a list field: `Polygon.points`, `PolyLine.points`/`colors`,
  `Paint.fillGradient.stops`, `DrawLayer.items` — same hazard; Polygon and DrawLayer
  measured.

## Resolution (bs23-agent-a, 2026-10-12)

The Cause note is right, and the fix is the one it names.

`emit_publish` no longer reads `count`, `dataLength` or the data base. It hands the two
blocks to `CodeBuilder::emit_values_content_equal_branch`
(`src/codegen/collection/compare/builder_collection_compare.rs`), a value-equality
emitter beside `emit_comparable_values_match_branch` that has the `List` and data-union
arms that one lacks:

- a list: `count`, then each element through its entry (`valueOffset`, `valueLength`);
  an entry-free list (fixed-width elements) is one range of `count * width` bytes;
- a record: slot by slot; an inlined field is followed, and its offset is not compared;
- a data union: the tag, then the active variant's record at `+16`;
- a `String` field: length and bytes; a `Float`: its bits, as in the rest of that module.

It reads nothing that says how a block was built (`capacity`, `dataLength`,
`dataCapacity`, a union's `size`, an entry's reserved bytes). For each element of a list
with entries the bytes are tried first, a word at a time, and the typed walk runs only
where they differ.

Each block-shaped type is emitted once, as a routine the places that hold a value of the
type branch to, with the place to return to in a frame slot. Emitting the walk in place
would repeat `Paint` under every item and every item under `AutoPaint` and `Picture`.
The emitter names no canvas item, field or variant. `SceneShape::content_type` resolves
the scene list's element (`canvas.DrawItem`, `canvas.DrawLayer`) in the type model, and
everything below it comes from the model's fields and variants. A `Map`, a `Set`, a
`Result`, a function value and a value that is not flat have no arm: the emitter returns
`Err`, so such a field would be a build error, not a wrong answer.

How collections are built and copied did not change.

### Measured

- The doc's repro (`bugs/repro/bug-968/frame_skip.mfb`, `mfb build --app`, then
  `MFB_MACAPP_HEADLESS=1 MFB_CANVAS_SYNC=1 build/fs968.app/Contents/MacOS/fs968`): frames
  after each call were `1 1 2 2 3 4 5 6 7 8 8 9 10`, and are `1 1 2 2 3 3 3 3 3 4 4 4 4`.
- The matrix scene prints 80 lines. Before: every present drew a frame (1 to 80). After:
  29 frames, each where the content changes. The same executable built with
  `--target windows-x86_64` printed the same 80 lines on box 206
  (`MFB_WINAPP_HEADLESS=1`, `MFB_CANVAS_SYNC=1`).
- The copy is freed on the skip path the typed walk reaches: a 40-polygon scene rebuilt
  with `collections::append` and presented 120 and 240 times under `MFB_CANVAS_SYNC`
  (`mfb build --app --debug`) reported `frames 1`, `arena.0.live_bytes 72720` both times,
  and `alloc_calls - free_calls` = 48 both times (59166/59118 and 118206/118158).
- Cost of one `present` of 10,000 rectangles, release build, no `MFB_CANVAS_SYNC`, 200
  presents (`/tmp/bs23-agent-a/perf`; indicative only, load average 25 to 29, three runs
  each): unchanged scene 6.4 to 10.1 ms before and 3.2 to 4.2 ms after; last item changed
  on every present 6.6 to 10.7 ms before and 3.9 to 5.4 ms after. "Before" is
  `/tmp/main-dbg-target/release/mfb`.
- Code size: `runtime.canvas.publishScene` went from 372 to 3435 lines of `--ncode`
  (labels included) and `runtime.canvas.publishLayers` from 372 to 3544; 31 and 33
  routines. Both functions are in every program that imports `canvas`. That is a guess of
  11 KB of AArch64 code each (about 2,700 instructions), not a measured section size:
  no compiler built from the base commit was at hand to compare `__text` with.

### Golden impact

`scripts/artifact-gate.sh target/release/mfb all`: 2352 goldens checked, 4 diffs, all of
them `syntax/app/app-mouse-surface/app_mouse_surface.<target>.app.ncode` (macos-aarch64,
linux-aarch64, linux-x86_64, windows-x86_64). That fixture imports `canvas`, so it holds
`runtime.canvas.publishScene` and `runtime.canvas.publishLayers`, the two functions this
fix rewrites. No other fixture moved. Goldens were not regenerated.

### A test corrected

`rt_canvas_present_deep_copy::an_identical_re_present_skips_the_publish` asserted that the
publish body holds a label named `canvas_present_same`, the byte loop over the data
region. (1) Written 2026-08-30 in `5e3babc8c` (plan-98-B Phase 3). (2) It protects: an
identical re-present compares before it publishes and writes nothing to the scene region.
(3) No other user of the label (`grep -rn canvas_present_same`). (4) The label is the
data-region byte compare this bug's Cause note says to delete; the test pinned its name,
not the behaviour. Only that assertion changed: it now requires the content comparison's
routines (`content_equal_enter`). The store-order assertions in the same test are
untouched and pass.

