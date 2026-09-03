# plan-116-F: Gradient fills

Last updated: 2026-08-31
Effort: large (3h–1d)
Depends on: plan-116-E

Every item's interior is currently one flat `Color` (`Paint.fill`). This letter adds a
gradient as an alternative interior:

```
canvas::GradientKind        ' enum: Linear (the zero) | Radial
canvas::GradientStop
    offset AS Float         ' 0.0..1.0 along the gradient
    color  AS Color
canvas::Gradient
    kind       AS GradientKind
    startPoint AS Point     ' Linear: the axis start.  Radial: the centre.
    endPoint   AS Point     ' Linear: the axis end.    Radial: a point on the outer circle.
    stops AS List OF GradientStop
```

plus `fillGradient AS Gradient` on `canvas::Paint`.

Behavioral outcome: an item whose `paint.fillGradient` carries two or more stops fills
with that gradient — linear along `from`→`to`, or radial outward from `from` to the
circle through `to` — antialiased at the shape's edge exactly as a flat fill is, and
identically on the software, Metal and Vulkan paths. An item whose `fillGradient` has
fewer than two stops fills with `paint.fill`, exactly as today.

References:

- `src/codegen/builtins/canvas/mod.rs:424` — the `Paint` record and its
  "every field's zero value is that field's no-op" rule.
- `src/codegen/builtins/canvas/func_fill_stroke.rs:43` — the **only** `Paint[…]`
  construction site in the tree.
- `src/codegen/builtins/canvas/helper_geometry.rs:314` — `__canvas_polygonEdges`, the
  precedent for a variable-length geometry tail.
- `src/codegen/runtime/canvas/mod.rs:216` — the Vulkan storage buffer and its
  one-buffer-per-frame, offset-per-item rule.
- `src/docs/spec/app/06_canvas.md` §"Rendering conventions" — linear-light
  compositing.

## Prerequisites

See plan-116-A §Prerequisites for the three environment gates.

| Must be true | Command | Status |
|---|---|---|
| plan-116-E complete and archived | `ls planning/completed/plan-116-E-*` → one match | NOT MET |

If plan-116-E is not complete, this letter cannot start, full stop. E is the last
letter before this one to grow the header and the item block, and E also establishes
the amended `DrawItem` frozen-set language this letter's docs build on.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command before you continue and again before you stop.

## 1. Goal

- Three new exported types (`GradientKind`, `GradientStop`, `Gradient`) and one new
  `Paint` field (`fillGradient`).
- Linear and radial gradients fill any `DrawItem` that has an interior, with stops
  interpolated in linear light.
- A `Gradient` with fewer than two stops is a no-op: the item fills with `paint.fill`,
  byte-identically to today.
- The gradient is evaluated in **surface pixel coordinates**, so it composes with
  `Paint.transform` the same way the shape does.

### Non-goals (explicit constraints)

- **No stroke gradient.** `paint.stroke` stays a flat `Color`. A gradient stroke is a
  second, independent payload per item and was not asked for.
- **No gradient on `Text`.** A glyph is drawn from a cached coverage bitmap
  (`helper_items.rs`, the `GEO_KIND_TEXT` arm) and has no distance field; a gradient
  there is a different code path. `Text` keeps its flat fill. Say so in the docs.
- **No conic/sweep gradient.** Two kinds, as specified.
- **No colour-space option.** One interpolation space, chosen and documented (§4.3).
- **`paint.fill` is not removed** and remains the fallback.
- **No existing golden may move.**

## 2. Current State

### `Paint`, and how cheap it is to extend

`Paint` has six fields (`mod.rs:424-471`). MFBASIC named construction requires every
field, so a seventh breaks every literal — but there is exactly **one** literal in the
tree:

```
$ grep -rn 'Paint\[' --include='*.rs' --include='*.mfb' . | grep -v '/target/'
```
returns 7 rows, of which **6 are prose** in doc strings (`func_fill_stroke.rs:22`,
`mod.rs:431`, `helper_paint_defaults.rs:5`, `func_stroke.rs:22`, `func_fill.rs:23`,
`func_rgba.rs:23`) and **1 is code**: `func_fill_stroke.rs:43`. `canvas::fill` and
`canvas::stroke` both delegate to `__canvas_fillStroke`
(`func_fill.rs:43`, `func_stroke.rs:43`), so the entire `Paint` construction surface
is that one line plus the three default helpers in `helper_paint_defaults.rs`.

This is why adding a `Paint` field is a **medium** change and adding a `DrawItem`
field (plan-116-D) was not.

### The variable-length-payload precedent

A polygon's edges are exactly this problem, already solved three ways:

- **Software** — the geometry cache's tail. `__canvas_polygonHeader`
  (`helper_geometry.rs:277`) sets header slot 1 to `HEADER + count * 5` and
  `__canvas_polygonEdges` (`:314`) appends five floats per edge after the header.
- **Vulkan** — one storage buffer for the whole frame, each item carrying its start
  index in `ITEM_ARC_EDGE_BASE` (`runtime/canvas/mod.rs:274`), because a command buffer
  is recorded once and rebinding per item would give every polygon the last one's data.
- **Metal** — `setFragmentBytes:` copies each item's payload into the command buffer,
  so its base is always zero, and the payload is capped at 4 KiB (`MAX_EDGES`).

Gradient stops take the identical shape, and this letter reuses all three mechanisms
rather than inventing a fourth.

### Measured populations

| What | Count | Command |
|---|---|---|
| `Paint[` rows in the tree | 7 | `grep -rn 'Paint\[' --include='*.rs' --include='*.mfb' . \| grep -v '/target/'` — **re-verified 2026-09-02**, still 7 |
| …that are **code**, not prose | 1 (`func_fill_stroke.rs`, now `:74`) | read all 7 rows; the other 6 are doc prose |
| `Paint` fields today | 6 | `mod.rs` — **re-verified**; letters C–E added none, since plan-116-D's `cap` went on `Line`/`Arc` rather than on `Paint` |
| Paint default helpers | 3 (`__canvas_transparent`, `…noTransform`, `…noClip`) | `helper_paint_defaults.rs:15,25,166` |
| `mfb man canvas` members with compile-gated examples | 13 | `sed -n 23,37p tests/cli_canvas_man_examples_compile.rs` — **re-verified 2026-09-02**, still 13; letters C–E added examples to `fillStroke` and `present`, which were already listed |
| Vulkan buffer regions today | 2 (edges, then glyph coverage) | `runtime/canvas/mod.rs:245` (`VULKAN_GLYPH_BASE_WORDS`) |

### Verified properties

- **`__canvas_fillStroke` is the sole `Paint` constructor.** Read
  `func_fill.rs:42-44` and `func_stroke.rs:42-44`: both `RETURN
  __canvas_fillStroke(…)`. So one line changes and all three public constructors
  follow.
- **The cache's tail blindness was a LIVE bug and is already fixed.** Planning this
  letter surfaced that `__canvas_hashItem` hashed only the 22-slot header and
  `__canvas_headerMatches` compared only the header, while a polygon's point
  coordinates live only in the tail — so two same-box, same-count, same-paint
  polygons deterministically shared one entry and the second drew the first's
  shape. Fixed on main 2026-09-01: the polygon's points are folded into the item
  hash and a hit is confirmed by `__canvas_tailMatches` against the stored edge
  origins (`helper_geometry.rs`; pinned by
  `tests/rt_canvas_rasteriser.rs::polygons_sharing_a_header_keep_their_own_points`).
  **A gradient's stops are the same shape of content** — header-invisible tail —
  so this letter must add a gradient arm to BOTH seams: the hash and
  `__canvas_tailMatches`. §4.2.
- **"Every `__CANVAS_GEO_HEADER` reader updated" costs nothing** (verified 2026-09-02).
  `grep -rn "__CANVAS_GEO_HEADER" src/codegen/builtins/canvas/*.rs` → 25 sites, and
  **every one uses the symbol**, never a literal: `WHILE i < __CANVAS_GEO_HEADER`,
  `offset + __CANVAS_GEO_HEADER + g * 3`, `toFloat(__CANVAS_GEO_HEADER + count * 5)`.
  So growing the header is two edits — the MFBASIC `LET` at `helper_geometry.rs:53` and
  the Rust `HEADER_SLOTS` — and the rest follows, with
  `the_geo_layout_constants_match_their_rust_counterparts` pinning that the two agree.
  That has held for plan-116-C, D and E. The Phase 1 box reads like a sweep; it is not
  one, and looking for readers to edit is wasted effort.

- **UNVERIFIED: whether stop interpolation in linear light matches what a designer
  expects.** It is the choice consistent with `06_canvas.md`'s compositing rule, but
  the two spaces differ visibly on a black→white ramp. §4.3 decides and documents;
  Phase 3's reference image is what makes the choice inspectable.

## 3. Design Overview

Five pieces:

1. **The three types and the `Paint` field** — registry data, one constructor line,
   one new default helper.
2. **The stop payload** — carried in the geometry tail (software), the Vulkan buffer's
   third region, and Metal's per-item bytes. §4.2.
3. **The gradient evaluation** — a parameter `t` from the pixel position, then a stop
   lookup and a lerp. §4.3.
4. **The cache-key fix** — the header-only confirmation compare must not let two
   different gradients share an entry. §4.2.
5. **All three renderers.**

**Where the correctness risk concentrates:** the geometry cache — specifically,
forgetting one of the TWO seams the landed polygon fix established (§2): the stop
colours must be folded into the item hash AND confirmed by a `__canvas_tailMatches`
arm. Missing the first degrades distribution; missing the second turns a 31-bit
hash collision into an item drawn with another item's gradient. Phase 2 adds both
arms and tests them directly with two deliberately-colliding gradients.

**Where the design uncertainty concentrates:** the interpolation space (§4.3) and the
per-item stop cap on Metal. Both are settled in Phase 1 before renderers change.

**Byte-identity is NOT this letter's gate.** **Expected NOT to diff:** every existing
golden, since every existing scene has an empty `fillGradient`. **Expected to diff:**
`.ncodesum` on every canvas-emitting target, both `.spv` blobs, `mfb man canvas types`,
and `mfb man canvas fill`/`stroke`/`fillStroke` (whose rendered signatures gain the
field).

### Rejected alternatives

- **A `Gradient` resource (`RES`) holding a compiled ramp.** Rejected: `Paint` is a
  flat value threaded through items (`mod.rs:424`, and `06_canvas.md` §"Paint is a
  value"). A `RES` record field is *legal* now (plan-114 retired `2-203-0084`,
  2026-09-01), so the ban is no longer the reason — the reason is that a resource in
  `Paint` would make every painted scene carry lifetime, which `func_present.rs`'s
  DESC promises it never does; plan-116-I deliberately confines scene-carried
  resources to the two variants that name external assets.
- **Pre-bake the ramp to a 256-entry texture on the CPU.** Rejected: it quantises the
  gradient to 256 steps, which is visible as banding on a large area, and it would need
  a sampler on both GPU backends where neither has one today
  (`.ai/canvas-threading.md`: `Picture` "draws nothing until it has an atlas").
  Evaluating the stops per pixel is exact and needs no new binding on Metal.
- **A fourth region in the Vulkan buffer with its own binding.** Rejected for the
  reason already recorded at `runtime/canvas/mod.rs:245` for glyph coverage: a second
  buffer would need its own allocation, memory-type search, descriptor binding and
  upload, for data with exactly the same lifetime and access pattern. One buffer,
  three regions, one binding.

## 4. Detailed Design

### 4.1 The types and the no-op

```
GradientKind : enum { Linear (zero), Radial }
GradientStop : { offset AS Float, color AS Color }
Gradient     : { kind AS GradientKind, startPoint AS Point, endPoint AS Point,
                 stops AS List OF GradientStop }
Paint        : … + fillGradient AS Gradient
```

New default helper `__canvas_noGradient()` in `helper_paint_defaults.rs`, returning
`Gradient[kind := GradientKind.Linear, startPoint := Point[0,0], endPoint := Point[0,0], stops := []]`
— the all-zero value, with an empty stop list. `__canvas_fillStroke` names it, which is
the one code line that changes.

**The no-op rule: fewer than two stops means no gradient.** One stop is a flat colour a
user should express with `fill`, and zero stops has no colour at all. Both fall back to
`paint.fill`. This keeps `Paint`'s zero-is-no-op invariant exactly.

Stops are used **in the order given**, with `offset` clamped to `0.0..1.0` and to the
previous stop's offset (monotonic). Sorting them would be a silent reinterpretation of
what the program asked for; clamping is visible and predictable. Document it.

### 4.2 Carrying the stops, and the cache key

**Header** (**41** slots after plan-116-E, 0–40) grows to **47**. Written as offsets
from the constant rather than as literals, which is the rule **F1** draws out of four
letters getting this wrong in a row — take the base from `HEADER_SLOTS`, do not spell
it:

| Slot | Meaning |
|---|---|
| `HEADER_SLOTS`+0 = 41 | stop count (0 = no gradient) |
| +1 = 42 | gradient kind (0 Linear, 1 Radial) |
| +2, +3 = 43, 44 | `startPoint.x`, `startPoint.y` |
| +4, +5 = 45, 46 | `endPoint.x`, `endPoint.y` |

(Corrected 2026-09-02 from 42–47 / 48; see **F1**.)

**Tail**: five floats per stop — `offset, r, g, b, a` — appended after any existing
tail. Header slot 1 (total length) accounts for both, as
`__canvas_polygonHeader:298` already does for edges.

A `Polygon` with a gradient therefore has two tails. **Order matters and must be
fixed**: edges first, then stops, with the stop base derivable as
`HEADER_SLOTS + edgeCount * EDGE_SLOTS`. Write it that way in one helper so no reader
computes it independently.

**The cache-key extension.** The polygon half of this problem was fixed on main
2026-09-01 (§2): `__canvas_hashItem` folds tail-only content into the hash per
kind, and `__canvas_tailMatches` confirms a hit against the stored tail. Gradients
join that landed mechanism, not a new one:

- `__canvas_hashItem` hashes each stop's five values after the header (and after a
  polygon's points, for a polygon carrying a gradient — the fixed edges-then-stops
  tail order above makes the hash order equally fixed).
- `__canvas_tailMatches` gains a stop compare: for any item whose paint carries ≥ 2
  stops, compare the stored stop slots (base = `HEADER_SLOTS + edgeCount *
  EDGE_SLOTS`, the same helper every reader calls) against the item's stops.

Both seams are per-kind `MATCH` arms beside the polygon ones, so the pattern is
already on main to copy.

**Item block.** The gradient's scalars take one new `ivec4` (kind + count) and one for
`from`/`to` in 16.16 — block reaches **224** bytes. The stops ride:

- **Vulkan** — a third region of the shared buffer, after glyph coverage. New constant
  `VULKAN_GRADIENT_BASE_WORDS`, and a per-item start index in a free block word,
  mirroring `ITEM_ARC_EDGE_BASE` exactly.
- **Metal** — a **gradient region in plan-116-A's frame buffer**, after the edge
  region, mirroring how A moved the edges: the emitter writes each item's stops
  there and carries the per-item first-stop index in a free block word. A per-item
  `setFragmentBytes:` is NOT an option — plan-116-A made items instanced, and an
  instanced run cannot rebind a payload between instances (plan-116-A §3, rejected
  alternatives). New constant `METAL_MAX_FRAME_GRADIENT_STOPS`, equal to Vulkan's.
- **Vulkan** — the third region of its shared buffer as described above, capped by
  `VULKAN_MAX_FRAME_GRADIENT_STOPS`.
- **Both predicates sum the frame's stops** against the same number, exactly as
  both now sum polygon edges (plan-116-A gave Metal its frame edge cap). Recommend
  **4096** stops (× 5 floats × 4 bytes = 80 KiB per backend region): generous for
  a hand-authored scene, cheap to raise in one constant per backend.

### 4.3 Evaluating the gradient

Per pixel, in surface coordinates:

- **Linear** — `t = dot(p - startPoint, endPoint - startPoint) / |endPoint - startPoint|²`,
  clamped to `0..1`. A zero-length axis (`startPoint == endPoint`) yields `t = 0`, so
  the first stop's colour fills — defined, not a divide by zero.
- **Radial** — `t = |p - startPoint| / |endPoint - startPoint|`, clamped to `0..1`.
  Same zero-length rule.

Then walk the stops for the bracketing pair and lerp. A linear walk is `O(stops)` per
pixel; with the caps in §4.2 that is bounded, and it matches how
`__canvas_edgeDistance` already walks edges per pixel.

**Interpolation space: linear light.** The stop colours are sRGB-encoded bytes; decode
each through the existing `__CANVAS_SRGB` table (`helper_color.rs:23`), lerp in linear,
and hand the linear value to the same blend the flat fill uses. This is the choice
consistent with `06_canvas.md` §"Rendering conventions" — *"Compositing happens in
linear light"* — and with both shaders' `srgbToLinear`. It is also the choice that
makes a black→white ramp look uniformly bright rather than dark-heavy.

Recorded as a decision because sRGB-space interpolation is what several drawing APIs do
and a user may expect it; the difference is large enough to see. Phase 3's reference
image is what makes it inspectable.

**Coverage is unchanged.** The gradient replaces the fill *colour*; the shape's signed
distance, its coverage, and the stroke are untouched. That is what keeps a gradient-
filled ellipse's edge identical to a flat-filled one's.

## Compatibility / Format Impact

- **BREAKING: `canvas::Paint` gains a seventh required field.** Every user
  `Paint[…]` literal stops compiling. In-tree the cost is one line
  (`func_fill_stroke.rs:43`), because `canvas::fill`/`stroke`/`fillStroke` are the
  documented way to build one (`06_canvas.md`: *"a `Paint` is built with
  `canvas::fill`, `canvas::stroke` or `canvas::fillStroke` and refined with `WITH`"*)
  — so most user code is unaffected.
- **Three new exported types**; `mfb man canvas types` grows.
- **No existing scene changes** — every existing `Paint` gets an empty `fillGradient`.
- **`HEADER_SLOTS` 41 → 47**, **`ITEM_BLOCK_SIZE` 192 → 224** — internal.
  (Corrected 2026-09-02 from 42 → 48; see **F1** in Corrections. `ITEM_BLOCK_SIZE` was
  assumed correctly — plan-116-E landed 192.)
- **`.ncodesum` churn**; both `.spv` blobs regenerate.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick in the same commit as the
> work; `- [~]` for partial with a one-line remainder; fill `Commit:` on landing.
> **An unticked box means NOT DONE.**

### Phase 1 — The types, the `Paint` field, and the no-op path

The whole breaking surface change, with nothing yet reading it.

- [x] Add `GradientKind`, `GradientStop` and `Gradient` to `mod.rs`. → the two
      endpoints are `startPoint`/`endPoint`, not `from`/`to`: **`TO` is a reserved
      keyword** and the record would not parse. See **F3**.
- [x] Add `fillGradient AS Gradient` to `Paint`, **last** in the prop list. The three
      constructors' prose that said "all six `canvas::Paint` fields" now says seven,
      and `fillStroke`'s "blend, transform and clip" is now four no-ops.
- [x] Add `__canvas_noGradient()` to `helper_paint_defaults.rs`; name it in
      `__canvas_fillStroke` (`func_fill_stroke.rs:43`).
- [x] Header slots **41–46** written by `__canvas_paintHeader`; `HEADER_SLOTS` → **47**;
      every `__CANVAS_GEO_HEADER` reader updated. → the last clause is free, not a
      sweep: all 25 sites use the symbol (see Verified properties). The stop *count*
      doubles as the has-a-gradient test, so one comparison decides it rather than a
      flag that could disagree with the list.
- [x] Tests: `tests/cli_canvas_package.rs` builds a `Gradient` and a `Paint` carrying
      one; `mfb man canvas types` lists all three new types. → the scene builds a
      two-stop radial gradient, wraps it in a `Paint` with `WITH`, and asserts the stop
      count survives; all three types render on the types page.

Acceptance: `cargo test --no-fail-fast` green, **every** canvas golden byte-identical,
`scripts/man-run-examples.sh canvas --run` passes. Nothing reads the gradient yet.

**MET.** `rt_canvas_golden` 12/12 with `git status --short tests/golden/canvas/`
empty — the gradient is inert, as intended, since nothing reads the slots yet.
`man-run-examples.sh canvas --run` → 22/22, and
`cargo test --release --no-fail-fast` → 96 test binaries, 0 failures.
Commit: a8d0b9dc3

### Phase 2 — The tail, and the cache-key fix

The correctness-critical piece, landed before any pixel depends on it.

- [x] Append the stop tail (five floats per stop) after any existing tail; one helper
      computes the stop base as ~~`HEADER_SLOTS + edgeCount * EDGE_SLOTS`~~
      **`length − stopCount × 5`** and **every** reader calls it. → the plan's formula
      is right only for a polygon; see **F4**. The record's own length is in slot 1 and
      the stop tail is always last, so the base is derivable without knowing what the
      other tail is. Edges come first, then stops, and that order is what makes it
      derivable.
- [x] Add the gradient arm to `__canvas_hashItem` (stop values after header/points)
      and to `__canvas_tailMatches` (stored-stop compare), beside the polygon arms
      the 2026-09-01 fix landed (§4.2). → both, plus `__canvas_paintHeader` skipping
      the kinds with no interior (`Text`, `NONE`) so a stop tail never lands on a
      record whose tail means something else.
- [x] Tests: two gradients with identical headers and different stop colours get
      different cache entries ~~and draw their own ramps~~ — the gradient sibling of
      `polygons_sharing_a_header_keep_their_own_points`, which already pins the
      polygon case and must stay green. → `gradients_sharing_a_header_keep_their_own_stops`
      asserts `entries=2`. The "draw their own ramps" half is **deliberately not here**:
      nothing evaluates a gradient until Phase 3, so a colour assertion at this phase
      would pass off the two items' flat fills without touching the gradient. It lands
      in Phase 3 with both items sharing one flat fill, where only the stops can
      separate them.
- [x] **Added:** `a_gradient_with_fewer_than_two_stops_is_byte_identical_to_a_flat_fill`.
      The no-op rule checked where it is cheapest to get wrong — the record *length*. A
      one-stop gradient that still appended five floats would leave slot 1 disagreeing
      with what `__canvas_gradientStopBase` derives, and every later reader would index
      off the end.

Acceptance: the two cache-collision cases pass, every existing golden is
byte-identical, and `MFB_CANVAS_STATS`'s `entries=`/`floats=` counters show the
expected entry count for a scene with two near-identical polygons.

**MET.** Both new cases pass; `rt_canvas_rasteriser` is 34 passed / 2 ignored with
`polygons_sharing_a_header_keep_their_own_points` still green;
`rt_canvas_golden` 12/12 and `git status --short tests/golden/canvas/` empty. The
`entries=2` counter is what discriminates the real failure: with neither seam wired,
the two records hash the same, `__canvas_headerMatches` agrees, `__canvas_tailMatches`
agrees, and the second item silently draws the first's record.
Commit: fae99b90d

### Phase 3 — Software evaluation, and the reference image

- [x] `__canvas_gradientColor(offset, t)` in `helper_color.rs`: the stop walk and the
      linear-light lerp per §4.3. See **F5** — the first walk clobbered the index it
      found and read into the next record.
- [x] `__canvas_drawGeometry`: when the stop count is ≥ 2, compute `t` per pixel and
      take the fill colour from the gradient instead of slots 8–11. Hoist the
      has-gradient test outside the pixel loop, as plan-116-B did for blend. → hoisted:
      the count, the stop base, the kind and the axis are read once per item, so an
      item without a gradient — which is every item written before this letter — pays
      nothing per pixel.
- [x] New reference image `tests/golden/canvas/gradients.png`: a linear ramp, a radial
      ramp, a multi-stop ramp, and a black→white ramp (the case that makes the
      interpolation-space choice inspectable). → four rows, and two of them carry an
      argument rather than a demonstration: the radial row puts the same ramp on a
      circle and an **ellipse**, where the ramp visibly stays circular because it is
      measured in surface pixels; and the multi-stop row gives its stops out of order,
      which shows as a hard edge rather than a fourth band.
- [x] Tests: `tests/rt_canvas_rasteriser.rs` — a two-stop linear gradient (assert the
      colour at `t = 0`, `0.5`, `1.0`); a radial gradient (assert centre and rim); a
      zero-stop gradient (byte-identical to `paint.fill`); a one-stop gradient (same);
      a zero-length axis (first stop's colour everywhere); offsets out of order
      (clamped monotonically, per §4.1). → seven cases including the two from Phase 2
      and `gradients_draw_their_own_ramps`, the colour half deferred from there.
- [x] **Added:** both `*Renderable` predicates **decline** a scene carrying a gradient,
      until Phase 4 teaches the shaders to read the stops. Not in the plan and
      necessary: without it a GPU frame accepts the scene and draws the flat `fill`
      underneath — a wrong picture reported as success. plan-116-G/H use the same
      land-then-remove shape for groups, so this is the family's own pattern.

Acceptance: the six cases pass, `gradients.png` is committed, and every pre-existing
golden is byte-identical.

**MET.** Seven gradient cases pass; `rt_canvas_golden` 13/13 including the new
reference; `rt_canvas_metal` 4/4 and `rt_canvas_font` 12/12; no pre-existing golden
moved.

This phase also found that the guard on four GPU tests was inert (**F6**):
`gpuSelected=TRUE` means the *program asked* for the GPU, not that a backend drew, so
the assertion that was supposed to catch "a predicate declined and software drew a
perfect picture" caught nothing. A `gpuFrames=` counter was added and all four now
assert on it; re-run with the real guard they still pass, so the earlier measurements
stand — only the assertion was worthless.
Commit: 444b539d3

### Phase 4 — Metal and Vulkan

- [x] Vulkan: a third buffer region, `VULKAN_GRADIENT_BASE_WORDS`, a per-item start
      index, and the frame-total cap in `__canvas_vulkanRenderable`.
- [x] Metal: the gradient region of the frame buffer, the per-item first-stop index
      in the block, `__CANVAS_MAX_FRAME_GRADIENT_STOPS`, and the frame-sum decline in
      `__canvas_metalRenderable`.
- [x] The stop walk and linear-light lerp in MSL and GLSL; `scripts/regen-spirv.sh`.
- [x] The Phase 3 blanket decline removed from both predicates, replaced by the
      frame-total cap it was standing in for.
- [x] Tests: both GPUs match the oracle on `gradients.png` within
      `Tolerance::GPU_DEFAULT`; a scene whose stops sum past
      `__CANVAS_MAX_FRAME_GRADIENT_STOPS` **declines to software** (asserted via
      `MFB_CANVAS_STATS`, not by pixel equality — a declined frame equals the oracle
      by construction, which is the false pass).
- [x] A gradient-filled **Polygon** in both GPU scenes. Not in the plan and load-
      bearing: it is the only kind whose tail is edges *then* stops, so it is the only
      shape that can tell a correct first-stop base from a lucky one (**F9**).

Acceptance: `gradients.png` matches on both GPUs within `Tolerance::GPU_DEFAULT` with
`metalReady=TRUE`/`vulkanReady=TRUE`, and the over-cap scene is provably declined
rather than truncated.

**MET.** Metal: `rt_canvas_golden` 15/15, including
`the_gpu_draws_the_gradient_scene_the_reference_shows` (gated on `gpuFrames`, so a
decline cannot pass it) and `a_frame_past_the_gradient_stop_cap_declines_to_software`.
Vulkan: `scripts/test-canvas-vulkan.sh target/release/mfb --box 2228 --libc glibc` and
`--box 2227 --libc musl --icd auto` both exit 0 with `vulkanReady=TRUE gpuFrames=1`,
`worst=2 differing=0.8116%`. `rt_canvas_rasteriser` 39/39, `cargo test --bin mfb
canvas` 71/71.

Two defects were found here rather than reasoned about, both of which drew a *plausible
wrong picture* rather than failing — see **F7** and **F8**.
Commit: —

### Phase 5 — Docs and gates

- [ ] `mod.rs` — the three types, their props, and `Paint.fillGradient`. State the
      fewer-than-two-stops no-op, the offset clamping, that stops are **not** sorted,
      that interpolation is in linear light, and that `Text` has no gradient fill.
- [ ] `src/docs/spec/app/06_canvas.md` — a gradient subsection under §"Rendering
      conventions" with the two `t` formulas and the interpolation space.
- [ ] A worked example on `canvas::fill` or `canvas::fillStroke` showing a gradient;
      both are already in `MEMBERS`
      (`sed -n 23,37p tests/cli_canvas_man_examples_compile.rs`).
- [ ] `scripts/man-census.sh --memory-scope` → 0 unclassified hits;
      `scripts/man-run-examples.sh canvas --run` passes.
- [ ] `scripts/regen-ncodesum.sh`; prove the delta is this letter's.

Acceptance: `cargo test --no-fail-fast` green on mac+RELEASE and linux+DEBUG,
`scripts/test-accept.sh` green, `scripts/artifact-gate.sh all` 0 diffs.
Commit: —

## Validation Plan

- **Tests:** `tests/rt_canvas_rasteriser.rs` (6 gradient cases + 2 cache cases),
  `tests/rt_canvas_golden.rs` (+`gradients.png`), `tests/rt_canvas_metal.rs`,
  `tests/cli_canvas_package.rs`. Negative cases: zero stops; one stop; zero-length
  axis; out-of-order offsets; over-cap stop count on each backend.
- **Coverage check:** the evaluation is MFBASIC source in emitted programs, invisible
  to `cargo llvm-cov --bin mfb`. Confirm the has-gradient and no-gradient arms, both
  kinds, and the clamped-offset path are each exercised by a distinct assertion.
- **Runtime proof:** render `gradients.png`'s scene software / Metal / Vulkan and
  diff; separately, render a zero-stop gradient beside a flat fill and diff them
  against each other.
- **Doc sync:** `src/docs/spec/app/06_canvas.md`; the three type descriptions and
  `Paint.fillGradient` in `mod.rs`.
- **Acceptance:** `cargo test --no-fail-fast`, `scripts/test-accept.sh`,
  `scripts/artifact-gate.sh all`, `rustup run 1.96.0 cargo fmt --all &&
  (cd repository && rustup run 1.96.0 cargo fmt)`.

## Open Decisions

- **Interpolation in linear light (§4.3).** Recommended, for consistency with
  `06_canvas.md`'s compositing rule and both shaders' `srgbToLinear`. The alternative
  (sRGB-space) is what some drawing APIs do and the difference is visible on a
  black→white ramp — which is why that ramp is in `gradients.png`. Decide by looking
  at the image, then document the choice; do not leave it implicit.
- **Stops are used in order, offsets clamped monotonically, never sorted (§4.1).**
  Recommended: sorting silently reinterprets the program's request.
- **`METAL_MAX_FRAME_GRADIENT_STOPS = VULKAN_MAX_FRAME_GRADIENT_STOPS = 4096`
  (§4.2).** Recommended as a starting value; raise only with a measured scene.

## Corrections

**F7 — the Metal gradient cursor was zeroed per ITEM, not per frame, and with the
header pointer.** A `store_u64(SCRATCH[0], sp, OFF_GRAD_CURSOR)` landed inside
`emit_edge_buffer`'s body, where `SCRATCH[0]` holds the geometry record's address. So
the frame's stop cursor was never zeroed and read as a huge value; `cursor + count`
then exceeded `MAX_FRAME_GRADIENT_STOPS` on the first gradient, the over-cap arm stored
a stop count of 0, and the shader drew the flat `fill` beneath every ramp.

Measured, not reasoned: `the_gpu_draws_the_gradient_scene_the_reference_shows` reported
`247904 of 576000 pixels differ (max channel delta 255); first at (60, 60): got
[0, 0, 0, 255], want [255, 64, 34, 255]` — solid black, which is exactly what the
test's own failure text names as "the gradient arm never taken". Fixed by deleting the
stray store, zeroing `OFF_GRAD_CURSOR` in the per-frame cursor loop beside
`OFF_EDGE_CURSOR`, and adding the slot to the stack-slot census, which had never
listed it.

**F8 — the shaders lerped continuously; the oracle quantises `t` to 1/4096 by
truncation and lerps in integer 0..65535 linear space.** With F7 fixed the scene was
right in shape and colour but `13312 of 576000 pixels differ (max channel delta 1)` —
2.31% against `Tolerance::GPU_DEFAULT`'s 2% population budget. The per-channel bound
was never in question; the population was, because a gradient makes nearly every pixel
a boundary case, so a systematic sub-step bias converts directly into differing pixels.

The plan said "the stop walk and linear-light lerp", which is true and not sufficient:
`__canvas_gradientColor` computes `num = toInt((tt - loOff) / span * 4096.0)` and
`__canvas_gradientChannel` evaluates `loLin + (hiLin - loLin) * num / 4096` on the
**integer** table entries, then picks the byte whose linear value is nearest by binary
search. Three quantisations the shaders did not have. Both now reproduce all three:
`srgbTable(i)` recomputes the rounded table entry, `num` is truncated to 1/4096, the
lerp uses `trunc`, and the encode picks the nearest entry in LINEAR space by testing
the midpoint either side rather than rounding in sRGB space.

An intermediate fix that changed only the rounding *space* moved the count from 13312
to 13334 — which is what proved the rounding space was not the cause and sent the
search to the quantisation of `t`. Recorded because the near-miss is the useful part:
"the shader and the oracle both interpolate in linear light" was true throughout.

**F9 — a gradient-filled `Polygon` was in neither GPU scene.** A gradient's stops sit
at the END of the geometry record, so both emitters find the first stop by subtracting
`count * 5` from the record's own length. On every kind except `Polygon` the stops
begin directly after the header and a base computed either way agrees — so no scene in
the plan could distinguish a correct base from a lucky one. One was added to `GRADIENTS`
(`tests/rt_canvas_golden.rs`, regenerating `gradients.png`) and one to the scene in
`scripts/test-canvas-vulkan.sh`. Both pass, so the arithmetic was right; it was
untested, which is a different thing.

- **F6 (2026-09-02, Phase 3) — `gpuSelected=TRUE` does not mean the GPU drew, so the
  guard on four GPU tests was inert.** Every GPU comparison this plan family writes has
  the same shape: assert the backend rendered, *then* compare pixels — because a
  predicate that declines the scene makes software draw a perfect picture and the
  comparison passes for the wrong reason. plan-116-C, D and E each used
  `stats.contains("gpuSelected=TRUE")` as that assertion.

  `gpuSelected` is `canvas::useGpu()` — whether the **program asked** for the GPU
  (`helper_surface.rs:42` says so outright: "the program asked for it"). A test that
  sets `MFB_CANVAS_GPU=1` and then asserts `gpuSelected=TRUE` is asserting that it set
  its own environment variable. Four tests carried that guard and none of them guarded
  anything:
  `the_gpu_draws_the_{transform,endcap,ellipse}_scene_the_reference_shows` and
  `a_transformed_text_run_reaches_the_gpu_and_matches_the_oracle`.

  Found here because this phase deliberately makes both predicates **decline** a
  gradient scene, and the existing guard failed to notice — the stats read
  `gpuSelected=TRUE` on a frame software had drawn.

  Fixed by adding a counter that answers the question asked: `gpuFrames=`, incremented
  by each backend when it actually renders. All four tests now assert
  `!stats.contains("gpuFrames=0")`. Re-run with the real guard, the three golden GPU
  tests still pass — so those scenes were genuinely rendering on Metal and the earlier
  measurements stand; only the assertion was worthless. The gradient scene reads
  `gpuSelected=TRUE gpuFrames=0`, which is the discrimination the old guard could not
  make.

  One trap inside the fix, worth keeping: the counter must be incremented **inside**
  each backend before it calls `__canvas_presentSurface`, because that is what writes
  the stats line. Bumped by the caller afterwards it lags a frame and reads 0 on the
  single frame a headless test renders — which looks exactly like a decline.

- **F5 (2026-09-02, Phase 3) — an early exit that clobbers the index it found.** The
  stop walk was written as a `WHILE` that set the loop counter past `count` to leave
  early, which destroys the index the walk exists to produce. The lerp then read five
  slots past the stops into the **next record's header**, and every gradient rendered as
  one flat colour — `(38, 0, 0)` for a red-to-blue ramp — with the radial circle
  vanishing entirely.

  Worth recording for the failure *mode* rather than the slip: reading past a record in
  this buffer does not fault and does not produce anything obviously wrong. The
  neighbouring header is well-formed floats, so it renders as a plausible flat fill. The
  rewrite walks the whole list and records the index in a separate variable.

- **F4 (2026-09-02, Phase 2 design) — the stop base formula is wrong for `Text`, and
  the length accounting has a better home than the plan proposes.** Two findings from
  reading the eight header builders before writing the tail.

  **The formula.** §4.2 says every reader computes the stop base as
  `HEADER_SLOTS + edgeCount * EDGE_SLOTS`. That is right for a `Polygon` and wrong for
  a `Text`, whose tail is three floats per glyph rather than five per edge, and whose
  slot 20 holds a glyph count rather than an edge count. It happens not to bite, because
  §Non-goals rules out gradients on `Text` — but only if the *stops are actually not
  appended* for a text item, which is a thing the code has to do rather than a thing the
  non-goal achieves by itself. `__canvas_paintHeader` runs for every kind including
  `Text`, so it must skip the ones with no interior (kind 6 `__CANVAS_GEO_TEXT`, kind 5
  `__CANVAS_GEO_NONE`) explicitly; slot 0 is set before slot 1 in every builder, so it
  can.

  A base derivable from the *stored record* is better than one derived per kind:
  `stopBase = slot1 − stopCount × 5`. Both terms are already in the header, it needs no
  knowledge of what the other tail is, and it is correct for a polygon, a glyph run and
  a plain shape alike. One helper, as the plan asks — just a different formula.

  **The home.** §4.2 has each header builder account for the stop length in slot 1.
  Reading them shows something better: **all eight set slot 1 immediately before calling
  `__canvas_paintHeader`**, without exception (`grep -n "collections::set(out, 1,\|
  __canvas_paintHeader(out"` → the two alternate, eight times). So `paintHeader` can
  *add* the stop length to slot 1 itself, and no builder clobbers it afterwards. One
  edit rather than eight, in the function that already knows the paint.

  Why slot 1 has to be right *there* and not later, which is the trap: `header` is built
  and compared by `__canvas_headerMatches` before the tail exists, so correcting slot 1
  after `__canvas_tailFor` would make every stored record differ from every freshly
  built one — the cache would never hit again and would grow without bound. That is a
  performance cliff with no failing test, which is exactly the kind this letter's Phase 2
  exists to prevent.

- **F3 (2026-09-02, Phase 1) — `Gradient.to` cannot exist: `TO` is a reserved keyword.**
  The plan names the two endpoints `from` and `to`, which reads best and is what most
  drawing APIs call them. MFBASIC lexes `TO` as a keyword case-insensitively
  (`src/lexer.rs:1225`, `value.eq_ignore_ascii_case("TO")`) for `FOR i = 1 TO 10`, so
  the record declaration fails to parse:
  `error[1-102-0003 MFB_PARSE_INVALID_IDENTIFIER]: Field name must be an identifier.`

  Renamed to **`startPoint`** and **`endPoint`**, which is not a workaround but the
  house style: `canvas::Arc` already carries `startAngle`/`endAngle`, so a reader
  meeting `Gradient` finds the naming they have already seen on the neighbouring
  variant. `from` is *not* reserved and could have been kept, but a `from`/`endPoint`
  pair would be worse than either consistent choice.

  Worth recording beyond the rename: **a plan that names a new field cannot know it is
  legal**, because the reserved-word list is not something a designer consults. The
  check is one build — `mfb build -app` on any canvas program — and it costs seconds at
  Phase 1 versus a rename across the shaders, the emitters and the docs at Phase 4.

- **F2 (2026-09-02, pre-execution) — plan-122-D will delete `canvas::Color`, and this
  letter adds new surface that names it.** Main landed plan-122's six sub-plan documents
  (`d5889c379`) while plan-116-E was finishing. No code yet — the commit is
  `planning/plan-122-*.md` only, 2193 lines and nothing under `src/` — but **plan-122-D
  is a canvas migration** that says outright: "Delete `canvas::Color`, `canvas::rgb` and
  `canvas::rgba`", replacing them with `color::Color`, whose "field names, order and
  types are identical to `canvas::Color`'s by construction".

  This letter's `GradientStop` carries a `color AS Color`, and its man examples will
  name `canvas::rgb`. So the two plans touch the same surface, in one direction:
  whichever lands second inherits the other's rename.

  **Not a blocker and not a reason to change this letter's design.** The field is
  structurally identical by plan-122-D's own construction, so the migration is a
  find-and-replace over a name rather than a change of shape, and plan-122-D's census
  of "man examples naming `canvas::Color`/`rgb`" is exactly the mechanism that will
  sweep whatever this letter adds. Recorded so that whoever runs plan-122-D re-censuses
  rather than working from that list: **it was written before `GradientStop`,
  `Gradient` and this letter's `canvas::fill`/`fillStroke` gradient example existed.**
  The same warning plan-116-D's D2 earned the hard way — a census over a shared
  checkout is measured at the merge, not at plan time.

- **F1 (2026-09-02, pre-execution) — the header slot numbers are one high, for the
  fourth letter running.** This letter puts the gradient's six slots at 42–47 and takes
  `HEADER_SLOTS` to 48. plan-116-E landed **41**
  (`grep -n "^pub(crate) const HEADER_SLOTS" src/codegen/runtime/canvas/mod.rs`;
  `helper_geometry.rs:53` → `LET __CANVAS_GEO_HEADER AS Integer = 41`), so they are
  **41–46** and `HEADER_SLOTS` → **47**. `ITEM_BLOCK_SIZE` 192 → 224 stands; E landed
  192 as assumed.

  The root is plan-116-C's Correction C2 — C measured that it needed no per-axis slot
  and landed one lower than every later letter had predicted — and D, E and now F have
  each inherited it (D1, E1, F1). **Do not write absolute slot numbers in a plan.**
  Take the base from `HEADER_SLOTS` and describe new slots as offsets from it, which is
  what this letter's own §4.2 already does correctly for the *stop* base
  (`HEADER_SLOTS + edgeCount * EDGE_SLOTS`) and what the fixed numbers above should
  have done too.

  Letters G–J were checked at the same time and carry no absolute slot numbers, so this
  is the last of the four. plan-116-H's census row `ITEM_BLOCK_SIZE after plan-116-F |
  224` is consistent with the corrected figures and needs no edit.

- **C1 (2026-09-01, review — pre-execution).** The "latent" polygon cache gap this
  letter planned to close in Phase 2 was reproduced as a LIVE mis-render and fixed
  on main immediately (per `AGENTS.md` "never leave a bug you found"); §4.2 and
  Phase 2 were rewritten to extend the landed mechanism rather than build it. The
  Metal stop transport also changed from a per-item `setFragmentBytes:` (impossible
  under plan-116-A's instancing, as revised) to a region of A's frame buffer, and
  the per-item stop cap became a frame cap on both backends.

## Summary

The visible feature is small — one `t`, one lerp — and the sharp edge is the
geometry cache: planning this letter surfaced that its header-only key was already
a LIVE polygon bug, which was reproduced and fixed on main 2026-09-01
(`__canvas_tailMatches`; see §2). What remains here is to give gradients the same
two arms — hash and tail confirmation — so their header-invisible stops can never
share an entry. The other decision worth a second look is the interpolation
space; it is settled here in favour of linear light for consistency, and
`gradients.png`'s black→white ramp exists so a human can check that call. Untouched:
strokes, `Text` fills, and every existing scene.
