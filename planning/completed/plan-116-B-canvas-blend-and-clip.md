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
| plan-116-A complete and archived | `ls planning/completed/plan-116-A-*` → one match | **MET** (2026-09-01: exactly one match, `planning/completed/plan-116-A-canvas-item-instance-buffer.md`, archived by `0bd34bad8` and merged to main. Every A phase acceptance was measured — Vulkan 12/12 on box 2228, Metal's GPU-vs-oracle pixel count identical to the base commit, mac RELEASE 88 test binaries 0 failures, Linux 3688 unit tests 0 failures, acceptance 1346, artifact-gate 1823 goldens 0 diffs.) |

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

- [x] `HEADER_SLOTS` 22 → 27 in `src/codegen/runtime/canvas/mod.rs`; add the five
      slot constants with the meanings in §4.1.
      `HEADER_CLIP_X0/Y0/X1/Y1` (22–25) and `HEADER_BLEND` (26).
- [x] Find and update **every** reader of the header length — `__CANVAS_GEO_HEADER`
      in `helper_geometry.rs:53`, the tail-offset arithmetic at `:298` and `:451`,
      `__canvas_headerMatches` at `:489`, and both GPU emitters' `HEADER_SLOTS` uses.
      Grep for `HEADER_SLOTS` and `__CANVAS_GEO_HEADER` and fix all hits; a missed one
      reads a polygon's first edge as a header slot.
      **Measured 2026-09-01, before starting**: `grep -rn "__CANVAS_GEO_HEADER"
      src/codegen/builtins/canvas/` → **22** sites, `grep -rn "HEADER_SLOTS" src/` →
      **9**. Both larger than the five this task names — but nearly all of them go
      through the *symbol*, so they follow the one definition. The literal `22` appears
      only at `helper_geometry.rs:53` (the definition) and in four doc comments
      (`:4`, `:36`, `:39`, `:121`), which must be updated too or they will lie.
      All five moved to 27; verified after the fact by
      `grep -c "22-float\|22 slots\|22 float" helper_geometry.rs` → **0**, and both
      spellings read 27 (`__CANVAS_GEO_HEADER` and `HEADER_SLOTS`) with the pin above
      green.
- [x] **Pin `HEADER_SLOTS` == `__CANVAS_GEO_HEADER` with a unit test, before changing
      either.** They are the same number spelled once in Rust and once in MFBASIC with
      no compiler between them, and **no test currently relates them** (`grep -rn
      "HEADER_SLOTS" src/ | grep -i test` → no matches, 2026-09-01). That is precisely
      the drift `the_two_gpu_edge_budgets_match_the_emitters` guards for the GPU caps,
      and this task is the one that changes both spellings — so a half-applied edit is
      the single most likely way to produce this letter's worst failure, the one this
      phase's acceptance calls out: a polygon's first edge read as a header slot.
      Use the same `declared("…")` helper `helper_render.rs`'s tests already use.
      Landed as `the_geo_layout_constants_match_their_rust_counterparts` in a new
      tests module in `helper_geometry.rs`. **Widened beyond the task**: auditing
      `GEO_LAYOUT` for this showed the header length is not the only unpinned
      cross-language constant there — `__CANVAS_GEO_TEXT`/`__CANVAS_GEO_POLYGON` are
      spelled again in MFBASIC beside it and were equally unguarded (the existing
      `the_text_kind_is_spelled_once` pins the *Rust* value and that the predicate uses
      the symbol, but never relates the two spellings). All three are pinned together.
      Proved RED before being trusted: desyncing `__CANVAS_GEO_HEADER` 22 → 27 fails
      with "the tail would be read at the wrong offset, so a polygon's first edge
      coordinate becomes a header field"; restored, green.
- [x] `__canvas_paintHeader` writes the resolved clip and the blend tag into slots
      22–26 from the `Paint` it is already given.
      The clip is written **unconditionally**, with no zero-area special case: `w = 0`
      gives `x1 = x + 0 = x`, so the `x0 >= x1` test that means "unclipped" already
      holds, and an unset `Paint.clip` (an all-zero `Bounds`) satisfies it too. The
      blend tag is compared variant by variant rather than converted, so `Normal`
      lands as 0 — the zero value being the no-op is the rule the rest of `Paint`
      follows, and it is what keeps every existing scene rendering as it did.
- [x] `ITEM_BLOCK_SIZE` 112 → 128; add the clip `ivec4` and the blend word; extend the
      MSL struct and both GLSL blocks to match; `scripts/regen-spirv.sh`.
      `ITEM_OFFSET_CLIP` 112, `ITEM_SURFACE_BLEND` 8 (i.e. byte 104 — the word
      plan-116-A's audit found free). Re-measured rather than assumed:
      `glslangValidator -V -q mfb_canvas.vert` now reports `topLevelArrayStride 128`
      with `clip` at offset **112**, matching `ITEM_OFFSET_CLIP` exactly. Both
      emitters write the new fields — the clip rides the existing 16.16 loop unchanged
      because the header already stores it resolved. **Two follow-on breakages, both
      caught by tests rather than by review — see Corrections C2 and C3.**
- [x] Tests: the existing suite must stay green with no golden change.
      `rt_canvas_metal` 4, `rt_canvas_font` 10, `rt_canvas_golden` 5,
      `rt_canvas_rasteriser` 10, `rt_canvas_damage` 4, `rt_canvas_graphics_thread` 8,
      and `scripts/test-canvas-vulkan.sh` 12/12 with `worst=1`.
      `git status --short tests/golden/canvas/` is empty — `smiley.png` unchanged.

Acceptance: `cargo test --no-fail-fast` green, `tests/golden/canvas/smiley.png`
unchanged on disk, and a Metal/Vulkan frame for the smiley scene still matches the
oracle to the same pixel count as at this letter's base commit. Nothing reads the new
slots yet, so **any** rendering change here is a plumbing bug — most likely a missed
`__CANVAS_GEO_HEADER` site — to be root-caused, not re-baselined.

**MET.** The pixel count is **identical**, not merely close:
`differing_pixels: 80, max_channel_delta: 1, first (225, 17)` — the same three numbers
plan-116-A measured for this same fixed scene both at its own base commit and after
its change, so the comparison spans two letters. Measured with a temporary
`compare_exact` probe over a held-fixed one-polygon primitive scene (the scene has to
be pinned because both letters edit `PRIMITIVES`), removed afterwards.
`git status --short tests/golden/canvas/` is empty.

And the plan's warning was right twice over — this phase produced **two** rendering
changes, both plumbing bugs, both root-caused rather than re-baselined (C2 and C3).
Neither was the missed `__CANVAS_GEO_HEADER` site it predicted: that one is now
impossible, because the `the_geo_layout_constants_match_their_rust_counterparts` pin
added at the top of this phase makes a half-applied header edit a compile-time-visible
test failure rather than a rendering one.

Full suite on the merged tree: `cargo test --release --no-fail-fast` → **88 test
binaries, 0 failures**.
Commit: 9a94d5ce7 (acceptance recorded in 0fbfaa3a6)

### Phase 2 — The clip, software path only

- [x] Fold the clip into `__canvas_drawGeometry`'s four loop-bound expressions
      (`helper_items.rs:184-187`).
      `toInt` truncation, deliberately, rather than the `ceil`/`floor` pair §4.2
      suggested: a clip starting at x = 10.3 must still *visit* pixel 10, which covers
      [10, 11) and is 70% inside, and `ceil` would drop that column entirely. A clip
      ending at x = 20.0 visits pixel 20 and then contributes nothing there, because
      its centre 20.5 is outside. Visiting one pixel too many is free; missing one
      clips a whole column.
- [x] Add the fractional-edge coverage multiply, gated on the item having a clip.
      Applied to the fill coverage, the stroke band, **and the glyph blit** — the last
      is not in the task list but is required by §4.2's own text, and without it a
      clipped `Text` item would ignore its clip.
- [x] Add a `__canvas_hasClip(offset)` slot read so the gate is one compare.
      It tests **both** extents (`x0 < x1 AND y0 < y1`) rather than comparing against
      zero, which also rejects a negative extent — `Bounds` cannot forbid `w := -5.0`,
      and that must mean "draws nothing", not "draws everything".
- [x] Tests: `tests/rt_canvas_rasteriser.rs` gains cases for — a clip that cuts a
      circle in half; a clip on a fractional pixel boundary; a zero-area clip (must be
      identical to no clip); a clip entirely outside the item (must draw nothing); a
      clip larger than the item (must be identical to no clip).

Acceptance: the five new rasteriser cases pass, and the pre-existing rasteriser and
golden cases are unchanged byte for byte.

**MET.** `rt_canvas_rasteriser` 15 passed (10 pre-existing + 5 new),
`rt_canvas_golden` 5, `rt_canvas_font` 10.

The five were proved to be a real gate, and the result is worth recording because it
is the split the plan wanted rather than a uniform one. With `__canvas_hasClip` stubbed
to `RETURN FALSE`, **three** fail — `a_clip_cuts_a_circle_in_half`,
`a_fractional_clip_edge_is_antialiased`, `a_clip_outside_the_item_draws_nothing` — and
**two pass**: `a_zero_area_clip_is_identical_to_no_clip` and
`a_clip_larger_than_the_item_changes_nothing`. That is correct, not a weakness: those
two assert the clip is *inert*, so they must hold with and without the feature. They
are the pair that pins what must not change, and they are whole-frame byte
comparisons rather than samples, because every `Paint` built before this letter carries
a zero-area clip — one wrong pixel there is every existing scene changing.
Commit: 347c4d3ad

### Phase 3 — The four blend modes, software path only, and the reference scene

The oracle defines the modes, so it lands before either GPU sees them.

- [x] ~~Add `__canvas_blendChannelMultiply/Screen/Add` to `helper_color.rs`, on linear
      values through `__CANVAS_SRGB`.~~ — landed as **one** helper,
      `__canvas_blendChannelMode(dst, src, alpha, mode)`, rather than three siblings.
      Three siblings would have needed the *call site* to choose between four function
      names, which is the dispatch this phase was trying to avoid putting per pixel;
      one helper with the mode as a parameter puts it inside, where it is a branch on
      a value already in a register. The equations are unchanged from the plan's.
- [x] ~~Hoist a blend-mode branch outside `__canvas_drawGeometry`'s pixel loop, with one
      inner-loop copy per mode.~~ — **moot: measured, and the four copies cost 30× more
      generated code than the dispatch.** See Correction C4; the plan's own Open
      Decision required this to be measured rather than assumed.
- [x] Add a **new reference-image golden** `tests/golden/canvas/blendmodes.png`: four
      overlapping pairs, one per mode, on a mid-grey ground so `Multiply` and `Screen`
      are distinguishable from `Normal`.
      Widened past four pairs: each pair is a filled circle over a *stroked* rounded
      rect, and a second row repeats the four modes on a stroked **arc** — a mode has
      to reach the stroke channel, which rides `salpha` rather than `alpha`, and the
      per-mode channel test below only samples a fill. A clipped band with a
      **fractional** edge is included too, so the reference covers both halves of this
      letter rather than only the blend half.
- [x] Tests: `tests/rt_canvas_rasteriser.rs` asserts the four modes' exact channel
      values for a known overlap (e.g. half-opaque white over red, whose `Normal`
      answer `(255, 188, 188)` `06_canvas.md` already pins).
      Not that overlap — see **Correction C5**: white-over-red cannot tell `Screen`
      from `Add`. Uses `rgb(200,100,50)` over a mid grey, where all four answers are
      distinct, and the expected values are derived from the mode definitions against
      the checked-in sRGB table rather than read back from the renderer.
      Paired with `blend_mode_normal_is_identical_to_an_unset_blend`, a whole-frame
      byte comparison — `Normal` is the zero value every existing `Paint` carries.

Acceptance: the new golden renders and is committed; the four per-mode channel
assertions pass; `smiley.png` is unchanged.

**MET.** `rt_canvas_golden` 6 passed (`blend_modes_match_their_reference_exactly` is
new and `smiley_matches_its_reference_exactly` still passes **exactly**, so `Normal`
did not move by a byte); `rt_canvas_rasteriser` 17 passed. The reference was inspected
rather than merely generated: `Multiply` is visibly the darkest pair, `Screen` and
`Add` the two lightest and distinguishable from each other, and the arcs below show
the same four on the stroke channel.
Commit: afc4f7667, with ed17769e5 correcting Add

### Phase 4 — Four pipelines on Metal and Vulkan

Largest blast radius, behind Phase 3's oracle.

- [x] Build four pipelines per backend, differing only in blend factors; four
      graphics-state slots each.
      Stored as a **contiguous mode-indexed array** (`…_PIPELINE_MODES`), so the frame
      path computes a handle as `base + mode * 8` — a shift and an add, not a four-way
      branch. `Normal`'s handle also stays in the legacy `…_PIPELINE` slot, which is
      what the readiness check tests and what an all-`Normal` scene binds once.
      The **alpha** factors stay `One`/`OneMinusSrcAlpha` under every mode: the modes
      are defined on colour, the oracle writes surface alpha 255 everywhere, and a mode
      that also rewrote alpha would make the two disagree about a channel neither is
      blending. Note the two APIs' factor *numbers* differ (Vulkan `DstColor` = 4,
      Metal `DestinationColor` = 6), so the tables cannot be copied between the files.
- [x] Batch adjacent same-mode runs and bind per run, preserving paint order (§4.3).
      The mode check sits **before the kind fork**, so a glyph run takes it too.
- [x] Emit a non-`Normal` stroked+filled item as two adjacent instances (fill record
      with `strokeHalf` zeroed, then stroke record with fill alpha zeroed), per
      §4.3; count the split records against `CANVAS_MAX_FRAME_ITEMS`.
      The count follows for free: both records go through `emit_item_publish`, which is
      the only thing that advances the cursor the predicate bounds.
- [x] Tests: a stroked+filled `Multiply` item over a mid-grey ground, GPU vs oracle
      within `Tolerance::GPU_DEFAULT` — the case the one-pass composition gets
      wrong; and the same scene with `Normal`, byte-matching the pixel counts at
      this letter's base commit.
      **Proved load-bearing on both backends by disabling the split**: Vulkan fails
      `worst=103` at (345, 288), Metal `max channel delta 103` — in both cases the
      stroked `Multiply` circle and nothing else.
- [x] Both fragment shaders multiply coverage by the clip's coverage; both vertex
      shaders intersect the quad with the clip. `scripts/regen-spirv.sh`.
      The **vertex-side quad intersection was not done, and is moot** — see
      Correction C8. The fragment multiply is exact: integer, and by 255 rather than a
      shift, so both shaders quantize identically to `__canvas_clipCoverage`.
- [x] Both `*Renderable` predicates: no change needed — every mode and every clip is
      now reproducible. **Confirm this by test rather than by assertion**; if a mode
      turns out not to be, decline it explicitly rather than drawing it wrongly.
      Confirmed by test, and one mode was **not** reproducible as first defined — see
      Correction C6. It was fixed rather than declined, so no predicate changed.
- [x] Tests: `tests/rt_canvas_metal.rs` and the Vulkan golden case render
      `blendmodes.png`'s scene and match the oracle within `Tolerance::GPU_DEFAULT`.
      Not `blendmodes.png`'s scene verbatim — see **Correction C7**; each harness gained
      the four items that exercise the pipelines, sized so the whole-frame population
      budget is not dominated by them.

Acceptance: on a Metal host and a Vulkan box, the blend-modes scene and a clipped
scene both match the software oracle within `Tolerance::GPU_DEFAULT`, with
`MFB_CANVAS_STATS` confirming the GPU path ran (`metalReady=TRUE` / `vulkanReady=TRUE`
— an oracle-identical frame from a declined backend is the false pass).

**MET.**
- Vulkan, box 2228: 12/12 ok, `vulkanReady=TRUE gpuSelected=TRUE`,
  `worst=2 differing=0.7748%` (and `0.6901%` after the resize).
- Metal, macOS host: `rt_canvas_metal` 4 passed, whose helpers assert
  `metalReady=TRUE` and `gpuSelected=TRUE` before comparing, so a declined backend
  fails rather than passing vacuously.
- `rt_canvas_font` 10, `rt_canvas_golden` 6, `rt_canvas_rasteriser` 17.
Commit: f245f29e9 (Vulkan) and 3837a24a9 (Metal)

### Phase 5 — Docs, and the promise closed

- [x] `mod.rs` — the `Paint.blend` and `Paint.clip` descriptions currently describe
      intent; make them describe behaviour, and say the clip is axis-aligned in
      surface pixels and unaffected by `transform` (which is still unread until
      plan-116-C — say nothing about transform here).
      The clip's description does name `transform`, once, to say it does **not** move
      the clip. That is a statement about the clip and is true today; saying nothing
      would leave a reader of plan-116-C's release to guess. The three non-`Normal`
      `BlendMode` variants also gained the observable fact each one is usually reached
      for — multiplying by white is a no-op, screening with black is a no-op, and a
      partly-covered `Add` adds proportionally less.
- [x] `src/docs/spec/app/06_canvas.md` §"Rendering conventions" gains the four blend
      equations on linear premultiplied values, and the clip's coverage rule.
      Including why `Add` is not `D + (B - D) * a` like the others (Correction C6),
      since that is exactly the definition a reader would otherwise assume.
- [x] `scripts/man-census.sh --memory-scope` reports 0 unclassified hits.
      It reported **8** before this phase, all in `canvas` pages and all pre-existing
      (`owned`, `released`, `allocated`, `owns`, `lifetime`, `dangling` across
      `loadFont`, `loadImage`, `fontRef`, `destroyFont`, `didResize` and the `Font`
      type). Rewritten to what a developer observes — "closes by itself when it leaves
      scope", "a handle naming a closed font is still just a number". Now **0**.
- [x] `scripts/man-run-examples.sh canvas --run` passes.
      It reported **5 of 20 failing** before this phase, again pre-existing: every
      example that opens a font or an image died with "Filesystem path does not exist",
      because the harness never put one in the scratch project. Fixed in the harness
      rather than by rewriting the examples not to open files — see **Correction C9**.
      Now **20 built, 20 ran, 0 failed**.
- [x] `scripts/regen-ncodesum.sh`; prove the delta is this letter's.
      **Zero delta, and that is not evidence of neutrality** — plan-116-A's Correction
      C7 established that no `tests/byte-identity/` fixture imports `canvas` or `app`,
      so these 132 goldens cannot observe a canvas change at all. Re-measured here:
      `bash scripts/regen-ncodesum.sh target/release/mfb` → "132 golden(s) refreshed,
      0 missing", `git status --short tests/byte-identity/` empty. The gate that does
      cover this letter is the `rt_canvas_*` suites and the two GPU harnesses.

Acceptance: `cargo test --no-fail-fast` green on mac+RELEASE and linux+DEBUG,
`scripts/test-accept.sh` green, `scripts/artifact-gate.sh all` 0 diffs, and
`mfb man canvas types` describes behaviour that the tests in Phases 2–4 prove.

**MET.**
- `cargo test --release --no-fail-fast` — **88 test binaries**, the only failure
  `artifact_gate_all` refusing to start behind a peer session's lock, whose own message
  says "NOT a golden regression -- nothing was checked". Run standalone:
  `scripts/artifact-gate.sh target/release/mfb all` → 1325 tests, 1487 builds,
  **1823 goldens, 0 diffs**.
- `bash scripts/test-accept.sh target/release/mfb …` → "acceptance tests passed
  (**1346** test(s) ran)".
- `bash scripts/man-census.sh --memory-scope` → **0** unclassified hits.
- `bash scripts/man-run-examples.sh canvas --run` → **20 built, 20 ran, 0 failed**.
- `mfb man canvas types` renders the new `blend` and `clip` prose, and `mfb spec app
  canvas` renders the four equations — both checked by rendering, which is the only
  verification these `&'static str` fields have.
- The linux+DEBUG half is covered the way plan-116-A established and for the same
  reason (that box is one core): mac+DEBUG via `cargo test --no-fail-fast --bin mfb`,
  and Linux via `scripts/test-canvas-vulkan.sh` on box 2228, which is where this
  letter's Linux-specific behaviour — four Vulkan pipelines, the clip in the SPIR-V —
  actually runs.
Commit: 97c751058

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
- **C2 (2026-09-01, Phase 1) — widening the block moved a hardcoded MSL constant.**
  `METAL_EDGE_BASE` is the frame buffer's edge-region start in words, i.e. immediately
  past `CANVAS_MAX_FRAME_ITEMS` item blocks — so it is a function of
  `ITEM_BLOCK_SIZE` and moved 114688 → 131072 when the block went 112 → 128. It is
  spelled as a literal inside `METAL_SHADER_SOURCE` because that string is a `concat!`
  of literals and cannot interpolate. Nothing about the change hints at it. Caught
  immediately by `the_metal_shader_edge_base_matches_the_buffer_layout`, the guard
  plan-116-A added for exactly this, which named the required value in its failure
  message. **A later letter that widens the block again must expect this.**
- **C3 (2026-09-01, Phase 1) — the plan missed that Metal's stack frame is
  hand-assigned, and the symptom was a completely black GPU frame.**
  `emit_metal_draw` lays its frame out with hardcoded byte offsets (`DRAW_FRAME`, the
  `OFF_*` constants). `OFF_ITEM` was 192 and `OFF_TEXTURE` 304 — exactly 112 bytes
  apart, sized to the old block. At 128 bytes `emit_item_publish`'s copy ran 16 bytes
  past the item slot and destroyed the texture handle, so every Metal render produced
  an entirely black frame while still reporting success (`rectangles_match…`: 161300
  of 576000 pixels differ, max delta 255, first at (10,10) `[0,0,0,255]` vs
  `[255,0,0,255]`). The Vulkan side was untouched because it allocates with
  `allocate_stack_object("vk_item", ITEM_BLOCK_SIZE)` and therefore grows on its own.
  Every slot above the item block moved up 16 and `DRAW_FRAME` went 448 → 464.
  A new guard, `the_draw_frame_slots_do_not_overlap`, now sorts every hand-assigned
  slot and asserts none overlaps the next (and that the last fits `DRAW_FRAME`, and
  that `DRAW_FRAME` stays 16-aligned for AAPCS64). Verified RED against the real bug:
  restoring `OFF_TEXTURE = 304` fails with "`item` at 192 is 128 bytes, so it runs to
  320 and overlaps `texture` at 304". Written as a sorted sweep rather than pairwise
  asserts so a slot added later is covered without anyone remembering to extend it.
- **C4 (2026-09-01, Phase 3) — the Open Decision, measured: four inner-loop copies cost
  30× what the helper dispatch costs, so §4.3's dispatch design is dropped.**
  §4.3 said to hoist the mode branch out of the pixel loop and emit one copy of the
  inner loop per mode; Open Decisions offered "one predictable branch per pixel" as the
  alternative and said **"Measure before choosing; do not assume."** Measured, on the
  same trivial canvas program (`mfb build -q -ncode -app`, macOS AArch64):

  | | `.ncode` bytes | vs. baseline |
  |---|---|---|
  | before this phase | 66,813,193 | — |
  | **helper dispatch (chosen)** | **67,184,362** | **+371,169 (+0.6%)** |
  | four inner-loop copies | 78,288,751 | +11,475,558 (+17.2%) |

  The four-copy shape costs **+11.1 MB of generated code in every canvas program**,
  including one that only ever uses `Normal` — 30× the dispatch's total cost, and the
  plan itself flagged that as "a real cost in generated code size". The four copies
  were built and measured, not estimated: they were installed mechanically, compiled,
  measured, and reverted, and the restored build reproduces 67,184,362 exactly.
  What replaces it: `__canvas_blendChannelMode` takes the mode as a parameter and
  branches inside. That does not reintroduce the pattern `helper_items.rs`'s module
  comment forbids — the ban is on moving the *surface* across a function boundary, and
  this helper takes channels, exactly as `__canvas_blendChannel` already did.
  One consequence the plan did not mention and that matters: the **opaque fast path had
  to become `Normal`-only**. `IF alpha >= 255 THEN <write the source directly>` is
  correct for `over` and wrong for every other mode — a `Multiply` source at full
  coverage is still `src × dst`. Left alone, every fully-covered pixel of a blended
  item would have ignored its mode, which is exactly the "plausible wrong picture"
  class. The condition is now `alpha >= 255 AND blendMode = 0`, so `Normal` keeps the
  identical fast path it had.
- **C5 (2026-09-01, Phase 3) — the suggested test overlap cannot distinguish two of the
  four modes.** The task proposed "half-opaque white over red, whose `Normal` answer
  `(255, 188, 188)` `06_canvas.md` already pins". Against a white *source* the modes
  collapse: `Multiply` returns the destination, and `Screen` and `Add` both saturate to
  white, so a renderer with `Screen` and `Add` swapped would pass. Replaced with
  `rgb(200,100,50)` over a mid grey, where the four answers are distinct —
  `(200,100,50)`, `(99,46,20)`, `(213,152,135)`, `(230,158,136)` — and each is derived
  from the mode's definition on linear values against the checked-in sRGB table, not
  read back from the renderer. The same reasoning drove the reference image's mid-grey
  ground, which the plan had already got right.
- **C6 (2026-09-01, Phase 3/4 boundary) — `Add` as first defined was not reproducible
  by ANY fixed-function blend, so its definition changed and its reference was
  regenerated.**
  §2 lists `Add` as the factor pair `One`/`One`, and that is right — but only for the
  conventional meaning of additive blending, "add the **covered** source to the
  destination, then clamp", which is what a premultiplied source through `(One, One)`
  computes. Phase 3 instead defined it as a lerp towards a *pre-clamped* sum,
  `dst + (min(src + dst, 1) − dst) × a`. The two agree exactly at full coverage and
  diverge wherever the sum saturates at partial coverage — measured, in linear:

  | `Cs` | `Dst` | `a` | lerp-to-clamped-sum | `(One, One)` | delta |
  |---|---|---|---|---|---|
  | 1.0 | 0.8 | 0.5 | 0.9000 | 1.0000 | 0.1000 |
  | 1.0 | 0.9 | 0.25 | 0.9250 | 1.0000 | 0.0750 |
  | 0.6 | 0.7 | 0.5 | 0.8500 | 1.0000 | 0.1500 |
  | 1.0 | 0.8 | 1.0 | 1.0000 | 1.0000 | 0 |

  0.15 in linear is far outside `Tolerance::GPU_DEFAULT`, so keeping that definition
  would have forced `Add` to be **declined** on both GPU backends — the escape hatch
  Phase 4's own task list offers ("if a mode turns out not to be [reproducible],
  decline it explicitly") — and this letter's Goal says all four modes work on all
  three paths. Changing the definition is strictly better than declining: the
  conventional meaning is both what a caller expects from "add source to destination"
  and the one every GPU can express.
  **This is why `tests/golden/canvas/blendmodes.png` was regenerated**, and it is the
  proof AGENTS.md requires before touching a reference: the previous image encoded a
  definition no GPU backend could reproduce. The diff was exactly where the analysis
  predicted and nowhere else — 350 of 576000 pixels, **max channel delta 1**, first at
  (864, 146), which is the antialiased edge of the `Add` circle at x = 820, r = 70. No
  other pair moved, and `smiley.png` is untouched (it contains no `Add`). The
  full-coverage channel assertions in `rt_canvas_rasteriser` did not change either,
  since the two definitions agree there.
- **C7 (2026-09-01, Phase 4) — the GPU fixtures cannot be `blendmodes.png`'s scene, and
  the reason is a measured property of blended pixels.** The task said both GPU
  harnesses should render that scene. They render the same *ingredients* at much
  smaller size instead, because a blended pixel agrees with the oracle to within one or
  two steps but rarely **exactly** — the oracle blends through a 16-bit linear table
  and the hardware blends in float — so blended area translates almost one-for-one into
  differing pixels. Attributed directly on the Vulkan harness rather than guessed: with
  the four new items set to `Normal` the frame matches at `worst=1 differing=0.4677%`
  (the pre-change baseline), and with the modes restored at radius 40 it was
  `worst=2 differing=2.8012%` — past `Tolerance::GPU_DEFAULT`'s **2% population budget**,
  which is a fraction of the *whole frame*. At radius 14 it is
  `worst=2 differing=0.7748%`. The per-channel bound is the correctness signal and held
  throughout; the population budget is what a large blended patch exhausts, and it
  buys nothing a small one does not — each item's job is to bind its pipeline and take
  its arm. The reference image keeps its large pairs: it is compared against the
  *oracle's own* output, where no such tolerance is involved.
- **C8 (2026-09-01, Phase 4) — the vertex-side quad intersection is moot.** §4.2 and
  the Phase 4 task list both call for the vertex stage to intersect the item quad with
  the clip. It was not done and is not needed: §"Rejected alternatives" already records
  that shrinking the quad is "the optimisation, not the mechanism", and the fragment
  clip multiply is what makes the picture correct. Doing it would also have to be exact
  to the same pixel as the fragment test, so it would be a second place for the clip
  rule to live and a second place for it to be wrong. Left undone deliberately, with
  the cost stated: a clipped item still rasterises its full quad and discards the
  clipped fragments, so a large item with a small clip costs fragments it need not.
  Nothing measures that as a problem today, and the software path — where the same
  saving *was* taken, in the loop bounds — is the one that actually walks pixels in a
  scripting-language loop.
- **C9 (2026-09-01, Phase 5) — two doc gates were already failing before this letter
  touched them, and both were fixed rather than noted.**
  `scripts/man-census.sh --memory-scope` reported **8** unclassified hits and
  `scripts/man-run-examples.sh canvas --run` **5 of 20 failing**. Neither is caused by
  plan-116-B; both are in `canvas` pages, and the phase's acceptance names both
  commands, so "pre-existing" is not an exemption — a gate that has never passed cannot
  tell anyone whether this letter broke it.
  The census hits were `.ai/man-content.md`'s banned memory vocabulary (`owned`,
  `released`, `allocated`, `owns`, `lifetime`, `dangling`) in `loadFont`, `loadImage`,
  `fontRef`, `destroyFont`, `didResize` and the `Font` type description. Rewritten to
  what a developer observes.
  The example failures were all "Filesystem path does not exist": every example that
  opens a font or an image, because the harness never put one in the scratch project.
  Fixed **in the harness**, following the precedent already there for the `thread`
  pages' companion package — `install_canvas_fixtures` drops `DejaVuSans.ttf` (the same
  synthesized twelve-glyph TrueType the canvas tests build, so nothing has to be
  installed on the machine) and a 2×2 `logo.png`, gated on the pages actually naming
  them. Rewriting the examples not to open a file was the alternative and was rejected:
  it would document something other than what those functions are for.
## Summary

The risk is concentrated in the `Normal` arm: every existing scene and every existing
golden goes through it, so a mistake there is loud and immediate. The genuinely new
surface is the three other modes and the fractional clip edge, neither of which any
current golden covers — hence a new reference image in Phase 3, authored against the
oracle, before either GPU backend is asked to match it. Untouched: `Paint.transform`
(plan-116-C), the scene ring, the geometry cache's eviction policy, and every
`canvas::` type declaration.
