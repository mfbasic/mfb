# plan-116-B: `Paint.blend` and `Paint.clip` become real

Last updated: 2026-08-31
Effort: large (3h–1d)
Depends on: plan-116-A

`canvas::Paint` declares `blend AS BlendMode` and `clip AS Bounds`, `mfb man canvas
types` documents both, `mfb spec` §"Paint is a value, not ambient state" promises
both — and **neither is read by any renderer**. `grep -rn "\.transform\|\.clip\|\.blend"
src/codegen/builtins/canvas/` returns exactly three hits, all `description:` string
literals (`mod.rs:254`, `:322`, `:350`). The geometry builder reads only the two
colours and `strokeWidth`; the software rasteriser has no clip test and no blend
switch; and the Metal pipeline fixes its blend state at `One`/`OneMinusSourceAlpha`
for the whole frame (`metal.rs:38`), so `BlendMode.Multiply`/`Screen`/`Add` could not
take effect even if an item asked for them.

This is a **defect, not a gap**: the documentation states a behaviour the code does
not implement.

Behavioral outcome: a scene in which one item carries
`blend := BlendMode.Add` and another carries a non-zero `clip` renders with the first
item's colour added to what is beneath it and the second item's drawing confined to
its clip rectangle — identically on the software, Metal and Vulkan paths — while an
item carrying the zero `BlendMode.Normal` and a zero-area clip renders exactly as it
does today.

References:

- `src/docs/spec/app/06_canvas.md` §"Paint is a value, not ambient state" and
  §"Rendering conventions" — the promises this letter makes true.
- `.ai/canvas-threading.md` §10 — the `*Renderable` honesty gates.
- `src/codegen/builtins/canvas/helper_items.rs:83` — `__canvas_drawGeometry`, the
  software per-pixel loop where the clip test and the blend switch land.
- plan-116-A §4.2 — the item block now travels in a buffer, which is what makes room
  for the clip rectangle.

## Prerequisites

See plan-116-A §Prerequisites for the three environment gates (SPIR-V regen box,
Metal host, Vulkan box); they apply unchanged here.

| Must be true | Command | Status |
|---|---|---|
| plan-116-A complete and archived | `ls planning/completed/plan-116-A-*` → one match | NOT MET |

If plan-116-A is not complete, this letter cannot start, full stop. The clip
rectangle is four words and the blend mode one, and the pre-A item block has three
free words — so without A this letter would have to re-open the push-constant-limit
question that A exists to close.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command before you continue and again before you stop, and report the
> status of *all* prerequisites if you stop.

## 1. Goal

- `Paint.clip`: a non-zero-area `Bounds` restricts an item's drawing to that
  rectangle on all three paths. A zero-area `Bounds` (either extent `0.0`) means no
  clipping, exactly as `mod.rs:249` already documents.
- `Paint.blend`: all four `BlendMode` variants composite correctly on all three
  paths. `Normal` is unchanged from today's output, byte for byte.
- The software rasteriser remains the oracle: it defines each mode, and the GPU
  backends match it within `Tolerance::GPU_DEFAULT`.

### Non-goals (explicit constraints)

- **`Paint.transform` stays unread.** It is plan-116-C. Doing it here would put three
  independent behaviour changes behind one gate.
- **No new `canvas::` type or function.** `BlendMode` and `Bounds` already exist with
  the right shapes; this letter adds no surface. `Paint`'s field list is unchanged.
- **The clip is an axis-aligned rectangle in surface pixels**, not a path and not a
  transformed rectangle. `Bounds` cannot express anything else.
- **`BlendMode.Normal` output must not move by one byte** — it is what every existing
  golden and every existing scene renders with.
- **Blending stays in linear light**, per `06_canvas.md` §"Rendering conventions".
  The four modes are defined on linear values, not on sRGB bytes.

## 2. Current State

### The software path

`__canvas_drawGeometry` (`helper_items.rs:83`) reads the header by slot: fill RGBA
from slots 8–11, stroke RGBA from 12–15, `strokeHalf` from 7, bounds from 16–19. It
writes each covered pixel with `__canvas_blendChannel` (`helper_color.rs:60`), which
implements exactly one equation — source-over. The per-pixel loop is bounded by the
header's bounds clamped to the surface (`:184-187`), and there is no second
rectangle test.

The module comment on `helper_items.rs` records why the blend block is written out
twice rather than factored into a helper: the surface `List OF Byte` cannot cross a
function boundary without reintroducing a whole-surface copy per call
(`.ai/collections.md`, and the `collection-set-in-place-only-for-same-function-local`
memory — 290× slower). **That constraint governs this letter's software design**: a
blend-mode switch must not become a helper call per pixel.

### The GPU paths

Both fragment shaders end with source-over composed in-shader for stroke-over-fill,
then hand the result to fixed-function blending
(`mfb_canvas.frag:main`, `metal.rs:197`). The Metal pipeline's blend state is set
once at `metal.rs:38` to `One`/`OneMinusSourceAlpha`.

The in-shader composition rests on a stated identity: *"Stroke over fill, then the
hardware puts that over the destination — which is what makes this one fragment
equal to the software path's two sequential writes, since `over` is associative"*
(`mfb_canvas.frag:170-174`). **That identity is `Normal`-only.** The software oracle
applies the mode twice per pixel — fill into the surface, then stroke into the
result (`helper_items.rs`, the two `__canvas_blendChannel` runs) — and
`M(M(D, fill), stroke) = M(D, over(stroke, fill))` holds for `over` but for none of
`Multiply`/`Screen`/`Add` wherever the stroke band covers filled pixels. §4.3's
two-instance rule is what closes this.

A glyph is fill-only (`mfb_canvas.frag:160-165` — a text item's stroke was turned
into an outline polygon by the geometry builder), so text draws are always a single
source and need no such treatment.

### Measured populations

| What | Count | Command |
|---|---|---|
| Reads of `.blend`/`.clip`/`.transform` in canvas builtins | 3, all doc strings | `grep -rn "\.transform\|\.clip\|\.blend" src/codegen/builtins/canvas/` |
| `BlendMode` variants | 4 | `mod.rs:315-341` (`Normal`, `Multiply`, `Screen`, `Add`) |
| Free words in the item block after plan-116-A | 3 pre-A (`arc.w`, `surface.z`, `surface.w`); unbounded after | `runtime/canvas/mod.rs:268`, `mfb_canvas.vert:22` |
| Words this letter needs | 5 (clip ×4, blend ×1) | §4.1 |
| Canvas reference-image goldens | 1 | `ls tests/golden/canvas/` |

### Verified properties

- **All four blend modes are expressible with fixed-function blend factors**, so
  neither backend needs programmable blending, a read-back, or a subpass input. Worked
  through on premultiplied linear source `S` and destination `D`:
  - `Normal` — `One` / `OneMinusSrcAlpha` (today's state).
  - `Add` — `One` / `One`.
  - `Multiply` — `DstColor` / `OneMinusSrcAlpha`.
  - `Screen` — `One` / `OneMinusSrcColor`.
  This is the property the whole GPU design rests on; it is arithmetic, verified by
  expanding each equation, and Phase 3's per-mode reference images are what confirm it
  empirically. **Two bounds on that verification.** First, `Multiply`'s factor pair
  is exact only for an **opaque destination** (the premultiplied form has a
  `+Cs·(1-Ad)` term the factors drop): true here, because every surface pixel's
  alpha is written 255 (`helper_items.rs`, both blend arms store `toByte(255)`; the
  damage clear at `helper_render.rs:53` likewise), so state the assumption in the
  code. Second, the factors blend ONE source; a stroked+filled item under a
  non-`Normal` mode is TWO sequential sources in the oracle, so it cannot ride the
  in-shader stroke-over-fill composition — see §"The GPU paths" and §4.3.
- **`Paint.clip` and `Paint.blend` are vacuous for `Picture` today** — a `Picture`
  has no renderer at all: `__canvas_headerFor` gives it an empty `NONE` header and
  no draw path exists (bug-484). This letter changes nothing for `Picture`; when
  bug-484 lands the picture path, its blend/clip handling is that fix's design
  load, against the semantics this letter pins.
- **The blend mode is a per-*pipeline* state on both APIs, not a per-draw one.** Read
  `MTLRenderPipelineDescriptor`'s colour-attachment blend fields (set at
  `metal.rs:38`) and `VkPipelineColorBlendAttachmentState` (baked into
  `GRAPHICS_OFFSET_VULKAN_PIPELINE`, `runtime/canvas/mod.rs:186`). So per-item blend
  means **four pipelines**, selected per draw — not a shader branch.
- **UNVERIFIED: whether four pipelines can be built on a device that built one.**
  Nothing suggests otherwise, but it is a device-level fact and Phase 3 measures it
  before Phase 4 depends on it.

## 3. Design Overview

Three pieces:

1. **Carry the fields.** The geometry header gains a clip rectangle and a blend mode;
   the item block gains the same. This is pure plumbing and lands first.
2. **The clip.** A rectangle intersection — cheap, exact, and identical on all three
   paths because it is integer/float min-max on the *bounds*, not a per-pixel test,
   for the fill; and a per-pixel test only where the clip cuts a partially covered
   pixel. See §4.2.
3. **The blend.** Four equations in the software rasteriser, four pipelines on each
   GPU backend, selected by the item's mode — and a non-`Normal` stroked+filled
   item is emitted as **two instances**, so each reaches the fixed-function unit as
   a single source (§4.3).

**Where the correctness risk concentrates:** `BlendMode.Normal` regressing. Every
existing golden renders with it, so a mistake in the "unchanged" arm is a mass
reference-image failure — which is *good*, it is caught immediately. The subtler risk
is the three new modes disagreeing between the oracle and the GPUs in a way no
existing golden covers, which is why Phase 3 adds a per-mode reference scene before
the GPU work starts.

**Where the design uncertainty concentrates:** the four-pipeline claim (verified as
arithmetic, unverified as a device fact). Phase 3 builds the pipelines first and
proves them on a single quad before the emitters select between them.

**Byte-identity is NOT this letter's gate** — behaviour legitimately changes. The gate
is rt-behavioural: per-mode reference images plus an exact-match assertion on the
`Normal` path. **Expected to diff:** every `.ncodesum` for a target that emits the
canvas runtime, and the software rasteriser's emitted code. **Expected NOT to diff:**
`tests/golden/canvas/smiley.png`, because that scene uses `Normal` and no clip. A diff
there is a regression in the unchanged arm, to be root-caused — not a re-baseline.

### Rejected alternatives

- **Per-pixel clip test for every pixel.** Rejected: the fill loop is already bounded
  by the header's bounds, so intersecting the *bounds* with the clip does the same job
  for every interior pixel at zero per-pixel cost. Only the boundary needs care, and
  §4.2 handles it by clamping the loop rather than testing inside it.
- **A blend-mode branch inside the fragment shader, reading the destination.**
  Rejected: reading the destination requires programmable blending (Metal, Apple GPUs
  only) or an input attachment (Vulkan, a render-pass change). Fixed-function factors
  express all four modes exactly, on every device.
- **Clipping by shrinking the vertex quad only.** Rejected on its own: it is
  necessary but not sufficient, because a quad clipped to whole pixels cannot express
  a fractional clip edge. The shader keeps a clip test for the boundary pixels; the
  quad shrink is the optimisation, not the mechanism.

## 4. Detailed Design

### 4.1 Carrying the fields

**Geometry header** (`HEADER_SLOTS`, currently 22 — `runtime/canvas/mod.rs:338`):
grows to **27**. New slots:

| Slot | Meaning |
|---|---|
| 22 | clip `x` (px) |
| 23 | clip `y` |
| 24 | clip `x + w` |
| 25 | clip `y + h` |
| 26 | blend mode (0..3, the enum tag) |

Stored as the *resolved* rectangle (`x`, `y`, `x+w`, `y+h`) rather than `x/y/w/h`, so
neither the rasteriser nor either shader repeats the addition. A zero-area clip is
stored as all four zeros and is recognised by `x >= z OR y >= w`.

`HEADER_SLOTS` is read in five places that must all move together — this is the
change with the widest mechanical reach in the letter, and Phase 1 is exactly it.

**Item block:** the clip rectangle takes a new `ivec4` (16.16 px) at offset 112, and
the blend mode takes `surface.z` (offset 104), which plan-116-A's audit confirmed is
free. `ITEM_BLOCK_SIZE` becomes 128.

### 4.2 The clip

**Software** (`__canvas_drawGeometry`): the loop bounds at `helper_items.rs:184-187`
already clamp the header's bounds to the surface with `__canvas_maxI`/`__canvas_minI`.
The clip folds in as two more terms in the same four expressions — `firstX` takes
`max(…, ceil(clipX0))`, `lastX` takes `min(…, floor(clipX1))`, and so on. **No new
per-pixel work for the interior.**

Fractional clip edges: a clip at `x = 10.3` must leave the pixel at `x = 10` 70%
covered. That is a coverage multiply, and it happens only on the at most two columns
and two rows the clip edge crosses. Implement it as a per-pixel coverage factor
computed from the clip rectangle — the same
`clamp(0.5 - d, 0, 1)` form §"Rendering conventions" already specifies, with `d` the
signed distance to the clip rectangle. Folding it into the existing `coverage`
multiply costs one `__canvas_rectDistance` call per pixel and **must be gated on the
item having a clip at all**, so an unclipped item pays nothing.

**GPU:** the vertex stage intersects the item quad with the clip before mapping to
clip space (this is the "clamp the quad to the surface" step, now also clamping to the
clip). The fragment stage multiplies coverage by the clip's own coverage using the
same `rectDistance` already in both shaders — so a fractional clip edge is antialiased
identically to a shape edge, which is what keeps the oracle and the GPUs in agreement.

**Text:** a glyph run is a coverage-bitmap blit, not an SDF (`helper_items.rs`, the
`__CANVAS_GEO_TEXT` arm), but its blit loop has the same clamped-bounds shape as the
fill loop, so the clip folds into its bounds identically, and the boundary columns'
coverage multiply applies to the glyph's own coverage value. On the GPU the glyph
quad is intersected with the clip like any other quad, and the glyph fragment path
takes the same clip-coverage multiply. **Picture:** vacuous — no renderer exists
(bug-484, §2); nothing to do here.

### 4.3 The blend

**Software.** `__canvas_blendChannel` (`helper_color.rs:60`) gains three siblings —
`__canvas_blendChannelMultiply`, `…Screen`, `…Add` — each taking the same
`(dst, src, alpha)` and returning a `Byte`, all operating on linear values through the
existing `__CANVAS_SRGB` table so the transfer function is unchanged.

The per-pixel dispatch is the sharp edge. `helper_items.rs`'s module comment forbids
moving the surface across a function boundary, but these helpers take *channels*, not
the surface, so they are safe to call. The dispatch must be **hoisted out of the pixel
loop**: read the blend mode once before `WHILE y`, and branch to one of four copies of
the inner loop. That is four copies of an already-duplicated block, which is a real
cost in generated code size — see Open Decisions.

**GPU.** Build four pipelines at init, differing only in their colour-attachment blend
factors (§2 "Verified properties"). Store them in four graphics-state slots. Sort each
frame's draws by blend mode so the pipeline is set once per mode rather than once per
item — with `Normal` first, so an ordinary scene issues exactly one
`setRenderPipelineState:` / `vkCmdBindPipeline` as it does today.

**Sorting changes draw order, and draw order is semantics** — later items paint over
earlier ones (`func_present.rs` DESC). So the sort must be **stable within a mode and
must not reorder across modes when the scenes' bounds overlap**. The safe rule, and
the one to implement: batch only *adjacent runs* of the same mode. A scene that
alternates modes issues more pipeline binds; a scene that groups them issues few. This
preserves paint order exactly.

**A non-`Normal` stroked+filled item is two instances, not one.** The oracle blends
fill into the surface and then stroke into the result — the mode applied twice —
while the shaders compose stroke-over-fill in-shader and blend once, an identity
that holds only for `over` (§2). So for an item whose mode is not `Normal` and which
both fills and strokes, the emitter writes **two adjacent item records** into
plan-116-A's instance buffer: the first with `strokeHalf` zeroed (fill only), the
second with the fill alpha zeroed (stroke only), in that order. The existing
fragment shader needs no change for this — a zero `strokeHalf` skips the stroke
arm, and a zero fill alpha premultiplies to nothing — and paint order is exactly
the oracle's. `Normal` items, fill-only items, and stroke-only items stay one
instance, byte-preserving today's path. Cost: one extra `ITEM_BLOCK_SIZE` record
per non-`Normal` stroked+filled item, and the frame-item count in the
`CANVAS_MAX_FRAME_ITEMS` predicate check counts the split records.

## Compatibility / Format Impact

- **`canvas::` surface: unchanged.** No new type, function, field or variant. What
  changes is that two existing fields start being obeyed.
- **Observable rendering changes** for any scene that already sets a non-zero `clip`
  or a non-`Normal` `blend` — which today draws as if they were unset. There is no
  deprecation path and none is owed: the current behaviour contradicts the
  documentation.
- **`HEADER_SLOTS` 22 → 27** and **`ITEM_BLOCK_SIZE` 112 → 128** — both internal.
- **`.ncodesum` churn on every canvas-emitting target**; SPIR-V blobs regenerate.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick in the same commit as the
> work; `- [~]` for partial with a one-line remainder; fill `Commit:` on landing.
> **An unticked box means NOT DONE.**

### Phase 1 — Widen the header and the block, carrying zeros

Pure plumbing, no behaviour. Lands alone and is provably neutral.

- [ ] `HEADER_SLOTS` 22 → 27 in `src/codegen/runtime/canvas/mod.rs`; add the five
      slot constants with the meanings in §4.1.
- [ ] Find and update **every** reader of the header length — `__CANVAS_GEO_HEADER`
      in `helper_geometry.rs:53`, the tail-offset arithmetic at `:298` and `:451`,
      `__canvas_headerMatches` at `:489`, and both GPU emitters' `HEADER_SLOTS` uses.
      Grep for `HEADER_SLOTS` and `__CANVAS_GEO_HEADER` and fix all hits; a missed one
      reads a polygon's first edge as a header slot.
- [ ] `__canvas_paintHeader` writes the resolved clip and the blend tag into slots
      22–26 from the `Paint` it is already given.
- [ ] `ITEM_BLOCK_SIZE` 112 → 128; add the clip `ivec4` and the blend word; extend the
      MSL struct and both GLSL blocks to match; `scripts/regen-spirv.sh`.
- [ ] Tests: the existing suite must stay green with no golden change.

Acceptance: `cargo test --no-fail-fast` green, `tests/golden/canvas/smiley.png`
unchanged on disk, and a Metal/Vulkan frame for the smiley scene still matches the
oracle to the same pixel count as at this letter's base commit. Nothing reads the new
slots yet, so **any** rendering change here is a plumbing bug — most likely a missed
`__CANVAS_GEO_HEADER` site — to be root-caused, not re-baselined.
Commit: —

### Phase 2 — The clip, software path only

- [ ] Fold the clip into `__canvas_drawGeometry`'s four loop-bound expressions
      (`helper_items.rs:184-187`).
- [ ] Add the fractional-edge coverage multiply, gated on the item having a clip.
- [ ] Add a `__canvas_hasClip(offset)` slot read so the gate is one compare.
- [ ] Tests: `tests/rt_canvas_rasteriser.rs` gains cases for — a clip that cuts a
      circle in half; a clip on a fractional pixel boundary; a zero-area clip (must be
      identical to no clip); a clip entirely outside the item (must draw nothing); a
      clip larger than the item (must be identical to no clip).

Acceptance: the five new rasteriser cases pass, and the pre-existing rasteriser and
golden cases are unchanged byte for byte.
Commit: —

### Phase 3 — The four blend modes, software path only, and the reference scene

The oracle defines the modes, so it lands before either GPU sees them.

- [ ] Add `__canvas_blendChannelMultiply/Screen/Add` to `helper_color.rs`, on linear
      values through `__CANVAS_SRGB`.
- [ ] Hoist a blend-mode branch outside `__canvas_drawGeometry`'s pixel loop, with one
      inner-loop copy per mode.
- [ ] Add a **new reference-image golden** `tests/golden/canvas/blendmodes.png`: four
      overlapping pairs, one per mode, on a mid-grey ground so `Multiply` and `Screen`
      are distinguishable from `Normal`.
- [ ] Tests: `tests/rt_canvas_rasteriser.rs` asserts the four modes' exact channel
      values for a known overlap (e.g. half-opaque white over red, whose `Normal`
      answer `(255, 188, 188)` `06_canvas.md` already pins).

Acceptance: the new golden renders and is committed; the four per-mode channel
assertions pass; `smiley.png` is unchanged.
Commit: —

### Phase 4 — Four pipelines on Metal and Vulkan

Largest blast radius, behind Phase 3's oracle.

- [ ] Build four pipelines per backend, differing only in blend factors; four
      graphics-state slots each.
- [ ] Batch adjacent same-mode runs and bind per run, preserving paint order (§4.3).
- [ ] Emit a non-`Normal` stroked+filled item as two adjacent instances (fill record
      with `strokeHalf` zeroed, then stroke record with fill alpha zeroed), per
      §4.3; count the split records against `CANVAS_MAX_FRAME_ITEMS`.
- [ ] Tests: a stroked+filled `Multiply` item over a mid-grey ground, GPU vs oracle
      within `Tolerance::GPU_DEFAULT` — the case the one-pass composition gets
      wrong; and the same scene with `Normal`, byte-matching the pixel counts at
      this letter's base commit.
- [ ] Both fragment shaders multiply coverage by the clip's coverage; both vertex
      shaders intersect the quad with the clip. `scripts/regen-spirv.sh`.
- [ ] Both `*Renderable` predicates: no change needed — every mode and every clip is
      now reproducible. **Confirm this by test rather than by assertion**; if a mode
      turns out not to be, decline it explicitly rather than drawing it wrongly.
- [ ] Tests: `tests/rt_canvas_metal.rs` and the Vulkan golden case render
      `blendmodes.png`'s scene and match the oracle within `Tolerance::GPU_DEFAULT`.

Acceptance: on a Metal host and a Vulkan box, the blend-modes scene and a clipped
scene both match the software oracle within `Tolerance::GPU_DEFAULT`, with
`MFB_CANVAS_STATS` confirming the GPU path ran (`metalReady=TRUE` / `vulkanReady=TRUE`
— an oracle-identical frame from a declined backend is the false pass).
Commit: —

### Phase 5 — Docs, and the promise closed

- [ ] `mod.rs` — the `Paint.blend` and `Paint.clip` descriptions currently describe
      intent; make them describe behaviour, and say the clip is axis-aligned in
      surface pixels and unaffected by `transform` (which is still unread until
      plan-116-C — say nothing about transform here).
- [ ] `src/docs/spec/app/06_canvas.md` §"Rendering conventions" gains the four blend
      equations on linear premultiplied values, and the clip's coverage rule.
- [ ] `scripts/man-census.sh --memory-scope` reports 0 unclassified hits.
- [ ] `scripts/man-run-examples.sh canvas --run` passes.
- [ ] `scripts/regen-ncodesum.sh`; prove the delta is this letter's.

Acceptance: `cargo test --no-fail-fast` green on mac+RELEASE and linux+DEBUG,
`scripts/test-accept.sh` green, `scripts/artifact-gate.sh all` 0 diffs, and
`mfb man canvas types` describes behaviour that the tests in Phases 2–4 prove.
Commit: —

## Validation Plan

- **Tests:** `tests/rt_canvas_rasteriser.rs` (clip ×5, blend ×4),
  `tests/rt_canvas_golden.rs` (+`blendmodes.png`), `tests/rt_canvas_metal.rs`,
  `tests/cli_canvas_package.rs`. Negative cases: zero-area clip ≡ no clip; clip
  entirely outside ≡ nothing drawn; `BlendMode.Normal` ≡ today's bytes.
- **Coverage check:** the four blend helpers are MFBASIC source strings compiled into
  emitted programs, so `cargo llvm-cov --bin mfb` will *not* show them. Their coverage
  is the rt tests; confirm each of the four modes is exercised by a distinct assertion
  rather than by one scene that happens to include them.
- **Runtime proof:** render `blendmodes.png`'s scene three ways — software,
  `MFB_CANVAS_GPU=1` on Metal, `MFB_CANVAS_GPU=1` on Vulkan — and diff all three.
- **Doc sync:** `src/docs/spec/app/06_canvas.md`; the `Paint` field descriptions in
  `mod.rs`; `.ai/canvas-threading.md` §10 if the pipeline count changes what the
  renderer branch documents.
- **Acceptance:** `cargo test --no-fail-fast`, `scripts/test-accept.sh`,
  `scripts/artifact-gate.sh all`, `rustup run 1.96.0 cargo fmt --all &&
  (cd repository && rustup run 1.96.0 cargo fmt)`.

## Open Decisions

- **Four copies of the software inner loop (§4.3).** Recommended, because the
  alternative — a helper call per pixel — is the exact pattern
  `helper_items.rs`'s module comment forbids. The cost is generated-program size for
  every canvas program, including ones that only ever use `Normal`. The alternative
  worth measuring if that cost bites: keep one loop, and make the blend a
  four-way branch *inside* it on a value hoisted to a local — one predictable branch
  per pixel rather than four loop bodies. **Measure before choosing**; do not assume.
- **Whether `clip` should be affected by `transform`.** Recommend **no** — the clip is
  in surface pixels — and say so explicitly in the docs, because plan-116-C makes
  `transform` real and the question will be asked. A transformed clip cannot be
  expressed by `Bounds` anyway.

## Corrections

- **C1 (2026-09-01, review — pre-execution).** As first written, Phase 4 kept the
  shaders' in-shader stroke-over-fill composition for every mode; that composition
  equals the oracle's two sequential blends only because `over` is associative
  (`mfb_canvas.frag:170-174`), so `Multiply`/`Screen`/`Add` would have diverged from
  the oracle on every stroked+filled item. Replaced with the §4.3 two-instance
  rule. The `Multiply` factor pair's opaque-destination assumption was also made
  explicit, and the `Picture` variant was discovered to have no renderer at all
  (filed as bug-484) — blend and clip are vacuous for it.

## Summary

The risk is concentrated in the `Normal` arm: every existing scene and every existing
golden goes through it, so a mistake there is loud and immediate. The genuinely new
surface is the three other modes and the fractional clip edge, neither of which any
current golden covers — hence a new reference image in Phase 3, authored against the
oracle, before either GPU backend is asked to match it. Untouched: `Paint.transform`
(plan-116-C), the scene ring, the geometry cache's eviction policy, and every
`canvas::` type declaration.
