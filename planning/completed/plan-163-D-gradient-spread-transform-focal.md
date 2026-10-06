# plan-163-D: gradients — spread, transform, focal point, and stroke painting

Last updated: 2026-09-28
Effort: large (3h–1d)
Depends on: plan-163-C (see Prerequisites)

`canvas::Gradient` is missing four things SVG gradients have:

- `spreadMethod`: what happens past the ends of the ramp;
- `gradientTransform`: a gradient scaled, rotated or sheared, which is how an elliptical
  radial gradient is made;
- a radial focal point (`fx`/`fy`/`fr`);
- painting a stroke. A stroke is always a flat colour today.

On top of that, the ramp lives in **surface pixels**, so rotating an item spins the
shape through a ramp that stays put. That is the opposite of SVG, and of what
plan-163-C made the clip do.

This letter adds `spread`, `transform`, `focalOffset`, `focalRadius` and `strokeApply`
to `Gradient`, and renames `Paint.fillGradient` to `Paint.gradient`. The gradient moves
into the **item's own coordinates**: `gradient.transform` applies first, then
`paint.transform` and the group translation, the same order as SVG and the same
treatment C gave the clip. When `strokeApply` is `FillAndStroke`, the gradient paints
the stroke too, with the stroke colour as a tint over it. The tint rule is the one C
gave pictures.

**No backwards compatibility** (user decision, 2026-09-28): the rename and the ramp's
coordinate space change outright, and every in-tree site is migrated.

**The single behavioral outcome:** a gradient item draws the SVG ramp for its `kind`,
`spread`, `transform` and focal fields, carried by the item's transform and group
offset. With `strokeApply := FillAndStroke` its stroke shows the same ramp, tinted by
`paint.stroke` at strength `paint.stroke.a`. All three renderers agree: the software
renderer exactly, the GPUs within `Tolerance::GPU_DEFAULT`.

References:

- plan-163-C (the item-space clip precedent, the tint rule, the `transparency` fold
  into stop alphas, and the header and item-block growth this letter extends).
- `src/docs/spec/app/06_canvas.md` :438–442 ("a stroke is never a gradient", which this
  letter rewrites) and the transform rule (:378–399).
- `.ai/canvas-threading.md`:
  - :517–574, a per-item payload is a frame region plus an index;
  - :1003–1040, "Growing `ITEM_BLOCK_SIZE` moves six things" (**this letter grows it**);
  - :1042+, shared layout constants need pin tests.
- `.ai/testing-gates.md` :235 and :309.
- SVG 1.1 §13.2 (`spreadMethod`, `gradientTransform`, `fx`/`fy` and the focal clamp)
  and SVG 2 (`fr`) are the external contract.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-163-C complete and archived | `ls planning/completed/plan-163-C-*` → match | MET (2026-10-05: `planning/completed/plan-163-C-clip-path-transparency.md`, archived in 81b47828a) |

A and B are complete by transitivity. Everything below assumes C has landed:

- the clip is in item space, evaluated at the shape-space point;
- `transparency` is folded into the stop alphas at build time;
- a picture's fill colour is a tint;
- the header and item block are at C's sizes.

> **NOTE — the Status column is a snapshot; the Command column is the truth.** Re-run
> the command before you continue and before you stop.

## 1. Goal

- `canvas::Gradient` fields, in this order:
  - `kind`, `startPoint`, `endPoint`, `stops` (unchanged);
  - `spread AS canvas::GradientSpread`;
  - `transform AS canvas::Transform`;
  - `focalOffset AS canvas::Point`;
  - `focalRadius AS Float`;
  - `strokeApply AS canvas::GradientApply`.
- A new enum `canvas::GradientSpread`: `Pad` (ordinal 0, today's behavior), `Reflect`
  and `Repeat`.
- A new enum `canvas::GradientApply`: `FillOnly` (ordinal 0, today's behavior) and
  `FillAndStroke`.
- `Paint.fillGradient` is renamed `Paint.gradient`.
- The gradient's points are in the item's own coordinates. A pixel's gradient point is
  `transform⁻¹ · q`, where `q` is the shape-space point the renderer already computes
  (after undoing `paint.transform` and the group offset).
- All three renderers implement it.

### Non-goals (explicit constraints)

- **No gradient on `Text`, `PolyLine` or `Picture`.**
  - `Text` keeps ignoring it: a glyph is a coverage bitmap.
  - `PolyLine` keeps its per-point `colors` as its ramp.
  - `Picture` keeps its image as its fill.
- **No separate stroke gradient.** A stroke uses the item's one gradient, or none (user
  decision).
- **No SVG 2 "cone" radial**, the case where the focal circle is not inside the end
  circle. The focal point is clamped inside (§4), as SVG 1.1 does.
- **No `gradientUnits="objectBoundingBox"`.** A converter maps bounding-box units into
  `gradient.transform` itself.
- **No change to the stop-interpolation rule** (linear light, clamped offsets).

## 2. Current State

Paths are under `src/codegen/builtins/canvas/` unless written in full. Line numbers are
at HEAD `76d62ee7a`. A, B and C shift some of them, so re-verify before editing.

- **Types.**
  - `GradientKind` is at `mod.rs:527`, `GradientStop` at :717 and `Gradient` at :742.
  - `Paint.fillGradient` is at :862.
  - The default is `helper_paint_defaults.rs:__canvas_noGradient` (:178).
- **Header.**
  - Slots 41 `GRADIENT_COUNT`, 42 `KIND`, and 43–46 `FROMX/FROMY/TOX/TOY`
    (`helper_geometry.rs:GEO_LAYOUT`). PolyLine aliases 43/44.
  - The stop tail is built by `__canvas_gradientTail` (:820) and appended **last** by
    `__canvas_appendGradientTail` (:854). It is located by `__canvas_gradientStopBase`
    (:251).
- **Software ramp** (`helper_items.rs` near :565–588):
  - `t` is computed from the **surface** point `px, py`: radial
    `|p − from| / gradLen`, linear `dot(p − from, axis) / gradLen2`.
  - A PolyLine takes `t` from `__canvas_polylineScan` instead.
  - `__canvas_gradientColor` (`helper_color.rs:123`) clamps `t` to 0..1 (that clamp
    is today's Pad) and interpolates the stops.
- **GPU ramp.**
  - `mfb_canvas.frag:gradientColour(vec2 p)` (:788–803) uses the same formulas on the
    point it is given.
  - Radial is `item.surface.w == 1`.
  - `gradientAt(t)` (:751) reads the stops from the frame region (count and base in
    `item.ellipse.z/w`).
  - It is called for the fill at :940, and for a PolyLine at :946.
  - Metal's twins are in `metal.rs:METAL_SHADER_BODY` (`gradientAt` near :879, its
    caller near :920 and :1050).
- **Stroke colour.** The stroke is drawn as `covered(item.stroke, …)`
  (`mfb_canvas.frag` near :963) and its software twin in `helper_items.rs`. It is
  never a gradient (spec :438–442).

### Measured populations

The command for every counted row is `grep -rn "<pattern>" --exclude-dir=target --exclude-dir=.git --exclude-dir=planning --exclude-dir=bugs . | wc -l`.

| What | Count |
|---|---|
| `fillGradient` references (the rename) | 105 (Corrections 1) |
| `fillGradient :=` sites | 42 (Corrections 1) |
| `canvas::Gradient[` constructions (each gains 5 fields) | 43 (Corrections 1) |
| Gradient items that also have a transform or sit in a translated group (pixels change) | 0 (Corrections 2) |
| Gradient on a Line or an Arc under today's `__canvas_strokeAsFill` | draws nothing — the line is flat (Corrections 3) |

## 3. Design Overview

The work comes in three pieces:

1. **Software, item space.** Rename the field and add the new fields, with the stop
   ramp taking its point from `q` rather than the surface point. A linear gradient
   under any `gradient.transform` stays linear, so it folds into new points at build
   time.
2. **Software radial with transform and focal, spread, and stroke apply.** These are
   the per-pixel parts.
3. **GPU.** The item block grows by three `ivec4`s for the radial inverse transform and
   the focal fields. The spread and apply bits go into `surface.w`.

**Decisions:**

- **Linear folds at build time.** `t = dot(G⁻¹q − s, d)/|d|²` is an affine function of
  `q`, `t = u·q + c`. That is exactly a linear gradient with `from = −c·u/|u|²` and
  `axis = u/|u|²`. Both builders compute these in f64 and write them into slots 43–46
  as today. The renderers see an ordinary linear gradient, **so a linear gradient
  needs no new GPU word and no new shader arithmetic**, only the change from `p` to
  `q`. A degenerate `G` is treated as the identity, the `Transform` rule.
- **Radial needs `G⁻¹` per pixel.** A transformed circle is an ellipse, which the
  circle formula cannot fold into. The inverse is computed once at build, like
  `paint.transform`'s inverse (slots 27–32), and applied to `q` per pixel.
- **Focal (two-circle) radial**, in gradient space:
  - centre `c = startPoint` and radius `r = |endPoint − startPoint|`;
  - focus `f = c + focalOffset` and focal radius `fr = focalRadius`.
  - `t` is the largest root with `fr + t·(r − fr) ≥ 0` of
    `|p − f − t·(c − f)|² = (fr + t·(r − fr))²`, a quadratic using only `+ − × ÷
    sqrt` (the oracle's rule).
  - Clamps, as in SVG 1.1:
    - `fr` is clamped to `[0, r]`;
    - `focalOffset` is scaled so `|c − f| ≤ (r − fr)·(1 − 2⁻¹⁰)`, which keeps the
      quadratic's leading coefficient strictly negative. With `focalOffset = 0` and
      `fr = 0`, `t = |p − c| / r`, today's formula.
  - A zero `r` leaves `t = 0`, as today.
- **Spread** is applied to `t` before the stop lookup:
  - `Pad`: clamp to 0..1 (today's clamp in `__canvas_gradientColor`);
  - `Repeat`: `t − floor(t)`;
  - `Reflect`: `u = t − 2·floor(t/2)`, then `u > 1 → 2 − u`.

  It is the same expression in all three renderers, as one helper each.
- **Stroke apply.** With `FillAndStroke` and a gradient of at least 2 stops, a stroke
  pixel takes `g = ramp(q)` and draws:
  - `tinted = g.rgb × stroke.rgb / 255`;
  - `rgb = g.rgb + (tinted − g.rgb) × stroke.a / 255`;
  - `a = g.a`.

  This is C's picture tint with integer rounding copied from it exactly, so white at
  any alpha, or any colour at alpha 0, draws the plain ramp. The ramp's alpha already
  carries `transparency` (C's fold).
  - On a `Line` or an `Arc`, which have no interior, `FillAndStroke` paints the line
    with the ramp and `FillOnly` leaves it flat. Phase 1 pins whichever of these
    matches today's `FillOnly` behavior after reading `__canvas_strokeAsFill`.
- **Bit packing.** `surface.w` becomes `kind | spread << 1 | strokeApply << 3`. Every
  test `surface.w == 1` becomes `(surface.w & 1) == 1`. The header gets new slots
  `GRADIENT_SPREAD` and `GRADIENT_APPLY`, and the emitters pack them.

**Correctness risk concentrates in the item-block growth** (Phase 3, behind the four
guards), and in changing every `surface.w == 1` test. A missed one silently draws a
spread gradient as linear. Phase 3 greps for every reader and pins the count.

**Design uncertainty concentrates in** the focal quadratic's precision near the focal
edge on fp32 GPUs. Phase 2 measures it in software against a hand-computed ramp, and
Phase 3 against `GPU_DEFAULT`. If fp32 falls outside tolerance, add the double-float
path the shapes use. That would be a task, not a stop.

**Correctness gate: behavioral.**

- **Byte-identical sub-gate:** every software golden and hand test for a gradient with
  **no paint transform, no group offset, no gradient transform, Pad, no focal offset,
  and FillOnly** stays exact. For those, `q = p` and every new term is the identity.
- **Expected to change:**
  - gradients on transformed or group-offset items (measured in Phase 1);
  - `tests/syntax/app/app-mouse-surface/golden/*`;
  - the item-hash values.
  Each changed expectation is re-derived by hand, citing the user's 2026-09-28
  decision.

**Rejected alternatives:**

- *A ramp that stays in surface space, with `gradient.transform` mapping straight to
  the surface.* The user chose item space (Q1 = a).
- *A separate stroke gradient.* Rejected by the user.
- *An absolute `focalPoint`.* Its zero value `(0,0)` would move the focus to the item
  origin, breaking the canvas rule that an unset field is a no-op. `focalOffset` from
  the centre is inert at zero.
- *The SVG 2 cone for an outside focus.* It is rarely used, and it needs a second
  root-selection rule.

## 4. Record, enums, defaults

```
canvas::GradientSpread   Pad (0) | Reflect | Repeat
canvas::GradientApply    FillOnly (0) | FillAndStroke

canvas::Gradient
  kind, startPoint, endPoint, stops      ' unchanged
  spread       AS canvas::GradientSpread
  transform    AS canvas::Transform      ' all-zero = identity
  focalOffset  AS canvas::Point          ' from startPoint; zero = centred
  focalRadius  AS Float                  ' SVG fr; 0 = a point focus
  strokeApply  AS canvas::GradientApply
```

`__canvas_noGradient` sets every new field to its zero value. The zero `Gradient`
remains "no gradient", because it has fewer than two stops.

## 5. Layout

- **Header**: new slots appended after C's, with numbers recorded in Phase 1:
  - `GRADIENT_SPREAD`, `GRADIENT_APPLY`;
  - `GRADIENT_INV0..5` (radial `G⁻¹` as float32 bits, the slots 27–32 convention);
  - `GRADIENT_HAS_INV`;
  - `GRADIENT_FOCALX`, `GRADIENT_FOCALY`, `GRADIENT_FOCALR` (clamped per §3).
  - Linear gradients write the folded points into 43–46 and leave the INV slots at
    zero.
  - Both pin tests pin every new slot.
- **Item block**: grows by 48 bytes (three `ivec4`s):
  - `gradInv0 = (ia, ib, ic, id)` as float32 bits;
  - `gradInv1 = (itx, ity, hasInv, spare)`;
  - `gradFocal = (focalX, focalY, focalR, spare)` in 16.16.
  - Follow `.ai/canvas-threading.md:1003–1040` item by item, with both `.spv`
    regenerated on box 2228. The guards are
    `the_draw_frame_slots_do_not_overlap`,
    `the_metal_shader_region_bases_match_the_buffer_layout`,
    `the_pipeline_specializes_every_region_base` and
    `the_shared_buffer_holds_every_region`.

## Compatibility / Format Impact

- **Source-breaking, no shim:**
  - `Paint.fillGradient` → `Paint.gradient` (101 references);
  - `canvas::Gradient[...]` must name five more fields (39 sites);
  - `GradientSpread` and `GradientApply` are new.
- **Pixel-changing, no shim:** a gradient on a transformed or group-offset item now
  moves with the item.
- **Unchanged:** stop interpolation and every gradient covered by §3's byte-identical
  sub-gate.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit as
> the work. Use `- [~]` for partial work, with what remains. Mark a moot task
> `- [x] ~~text~~ — moot: <evidence>`. Fill `Commit:` when a phase lands. **An unticked
> box means NOT DONE.**

### Phase 1 — Types, rename, item-space linear gradients, spread (software)

- [x] Measure §2's UNMEASURED row and resolve the UNVERIFIED row. Record the file
      lists here; they are the tests whose expectations change. (Every file `grep -rln
      "fillGradient"` lists was read: no gradient item has a `paint.transform`, and a
      group offset already moved the ramp — so the list of tests whose expectations
      change is empty, Corrections 2. A Line's or Arc's gradient draws nothing today,
      Corrections 3.)
- [x] `mod.rs`: the two enums and the new `Gradient` fields per §4. Rename
      `fillGradient` → `gradient`. Write the descriptions, including that
      `gradient.transform`, `paint.transform` and the group translation all apply to
      the ramp, and the `strokeApply` tint rule. Update `__canvas_noGradient`.
- [x] Rename all 101 `fillGradient` references. Add the five fields to the 39
      `canvas::Gradient[` sites.
      Check: `grep -rn "fillGradient" --exclude-dir=target --exclude-dir=.git
      --exclude-dir=planning --exclude-dir=bugs .` → 0. (105 references, 43
      constructions — Corrections 1; the grep → 0 outside the app-mouse-surface goldens,
      which were then regenerated, and → 0 after.)
- [x] Both builders:
      - the linear fold of §3 (f64);
      - the spread and apply slots;
      - the new slots with pin tests.
      `emit_paint_supported` refuses a radial gradient with a non-identity `transform`
      or a focal field until Phase 2. (`__canvas_gradientHeader`/`__canvas_gradientInverse`
      in `helper_geometry.rs`, slots 55–66, header 55 → 67; the native builder already
      refuses every paint with two or more stops, so its share is the header length —
      Corrections 4. Pin: `the_gradient_slots_follow_the_clip_slots_and_fill_the_header`.)
- [x] Software (`helper_items.rs`): take the ramp point from `q` instead of `p`, and
      apply spread before `__canvas_gradientColor`, via one `__canvas_gradientSpread`
      helper. (`__canvas_gradientSpread` in `helper_color.rs`.)
- [x] `__canvas_hashPaint`: fold the new gradient fields. (Spread and apply as one step,
      the transform, the focal offset and radius; every other kind hashes structurally
      in `func_item_hash.rs`, which takes the new fields from the type model.)
- [x] Tests (hand-computed, `rt_canvas_rasteriser.rs`/`rt_canvas_golden.rs`):
      - a linear gradient rotated 90° by `gradient.transform` runs top to bottom;
      - a linear gradient on a rotated item rotates with it;
      - Reflect and Repeat at `t = 1.25` give the colours of `t = 0.75` and `t = 0.25`;
      - Pad at `t = 1.25` gives the last stop.
      Re-derive each changed expectation from the Phase 1 measurement by hand.
      (`a_linear_gradient_turned_by_its_transform_runs_top_to_bottom`,
      `a_linear_gradient_on_a_rotated_item_rotates_with_it`,
      `spread_pads_reflects_and_repeats_past_the_end`. No expectation changed by the
      measurement; one bound changed by the type growth — Corrections 6.)
- [x] Regenerate `tests/syntax/app/app-mouse-surface/golden/*` (expected). Inspect the
      `.ir` diff: it should be the type surface only. (`sync-goldens.sh` → 5 synced,
      `regen-native-goldens.sh` → 6 rewritten, 0 failures. The `.ir` diff is not the type
      surface only — Corrections 5.)
- [x] Discovered: `tests/cli/cli_canvas_package.rs`'s zero-values program still read
      `filled.clip.w`, plan-163-C's removed `Bounds` — Corrections 7.

Acceptance: item-space linear gradients and spread hold in software, and §3's sub-gate
set is byte-identical.
  Check: `cargo test --test rt_canvas_rasteriser --test rt_canvas_golden --test
  rt_canvas_geo_native` → pass. `git diff --stat tests/golden/canvas` must list only
  scenes in the Phase 1 measured list (est. 10 min, guess).
  Result: rasteriser 79 passed, 1 failed (the drain bound, Corrections 6; re-run alone
  after it → passed), 2 ignored; golden 22, geo_native 4; `git diff --stat
  tests/golden/canvas` → empty, the measured list being empty. Also `cargo test --bin mfb
  canvas` → 111; `cli_canvas_package` 7, `inplace_self_update_census` 2, `rt_canvas_font`
  33, `rt_canvas_picture` 11 passed.
Commit: 7cedfaabd

### Phase 2 — Radial transform, focal point, stroke apply (software)

- [x] Both builders: radial `G⁻¹` and the focal fields with the §3 clamps.
      `emit_paint_supported` accepts them once native emits them.
      `MFB_CANVAS_GEO_VERIFY=1` under `--debug` reports no mismatch on the tests below.
      (`__canvas_gradientHeader`: `G⁻¹` as float32 bits in slots 57–62 with the flag in
      63, the clamped focal offset and radius in 64–66, Corrections 8; under
      `FillAndStroke` slot 15 keeps the raw stroke alpha, Corrections 9. The native and
      GEO_VERIFY clauses are moot — the native builder declines every gradient paint, so
      GEO_VERIFY never sees one, Corrections 4.)
- [x] Software radial per §3: `G⁻¹q`, then the focal quadratic. Stroke apply per §3 in
      the stroke path. (`__canvas_focalT` and the tint in `helper_items.rs`; a Line or
      Arc tints its stroke-as-fill; `__canvas_strokeHalf` strokes a zero-alpha stroke
      under `FillAndStroke`.)
- [x] Tests (hand-computed):
      - a radial gradient scaled `(2, 1)` is an ellipse twice as wide as tall;
      - with `focalOffset := (r/2, 0)`, the pixel at the focus is stop 0 and the pixel
        at `c − (r, 0)` is the last stop;
      - `focalOffset` beyond `r` is clamped (no NaN, and a defined colour everywhere);
      - `focalRadius := r/4` puts stop 0 over a disc of radius `r/4` at the focus;
      - `FillAndStroke` with a white stroke at alpha 255 shows the plain ramp on the
        stroke;
      - a red stroke at alpha 128 gives a half-strength red tint;
      - a stroke colour at alpha 0 gives the plain ramp;
      - `FillOnly` leaves the stroke flat.
      (`a_radial_gradient_scaled_two_to_one_is_an_ellipse`,
      `a_focal_offset_moves_the_first_stop_to_the_focus`,
      `a_focal_offset_beyond_the_circle_is_clamped`,
      `a_focal_radius_fills_a_disc_with_the_first_stop`,
      `fill_and_stroke_paints_the_outline_with_the_tinted_ramp` — white, alpha 0, red at
      128 and a Line; `FillOnly` is `a_gradients_stroke_stays_a_flat_colour` and
      `a_gradient_on_a_stroke_only_kind_is_ignored`. The Validation Plan's negative cases:
      `degenerate_gradient_fields_draw_as_their_no_op_forms` — a singular transform, one
      stop under `FillAndStroke`, `focalRadius` above `r`.)
- [x] Discovered: regenerate the app-mouse-surface goldens for the Phase 2 helpers
      (`sync-goldens.sh` → 5, `regen-native-goldens.sh` → 6 rewritten, 0 failures).

Acceptance: per §3 in software, with Phase 1 results unchanged.
  Check: `cargo test --test rt_canvas_rasteriser --test rt_canvas_golden --test
  rt_canvas_geo_native` → pass, goldens unmodified since Phase 1 (est. 10 min, guess).
  Result: rasteriser 86 passed (2 ignored), golden 22, geo_native 4, picture 11, font 33;
  `git status --short tests/golden` → nothing.
Commit: 54e7684d7

### Phase 3 — Item block growth and the GPU (largest blast radius)

- [x] Item block +48 bytes per §5, with every guard passing. (320 → 368:
      `ITEM_OFFSET_GRADIENT_INV` 320, `ITEM_OFFSET_GRADIENT_FOCAL` 352; `gradInv0`,
      `gradInv1`, `gradFocal` in both GLSL structs and the MSL one; `cargo test --bin mfb
      -- the_draw_frame_slots_do_not_overlap the_metal_shader_region_bases_match_the_buffer_layout
      the_pipeline_specializes_every_region_base the_shared_buffer_holds_every_region
      vulkan spirv metal` → 19 passed.)
- [x] Emitters (`metal.rs`, `vulkan.rs`): the new words, and `surface.w` packing.
      (`emit_item_block` in both: header 57–63 word for word, 64–66 in 16.16, `kind + 2 ·
      spread + 8 · apply`.)
- [x] Discovered: the blend split published a gradient's or picture's fill twice —
      Corrections 10. (`emit_split_or_publish` in both emitters, `emit_block_instances`,
      `__canvas_blockInstances`; record two's fill alpha −1, read by both shaders.)
- [x] Shaders (`METAL_SHADER_BODY`, `mfb_canvas.frag`, line for line):
      - `gradientColour` takes `q`;
      - radial `G⁻¹` and the focal quadratic;
      - the spread helper;
      - the stroke tint.
      Replace every `surface.w == 1`: first run `grep -nE "surface\.w *== *1"
      src/codegen/runtime/canvas/shaders/mfb_canvas.frag src/target/macos_aarch64/app/metal.rs`,
      record the count here, and bring it to 0. Regenerate both `.spv` on box 2228.
      (At 54e7684d7: 1 in `mfb_canvas.frag`, and Metal's twin `gradientKind == 1` 1 —
      Metal names the word `gradientKind`; now 0 and 0, every reader unpacking `& 1`.
      `gradientSpread`, `focalT`, `tintRamp`, `gradientColour(q)`. `.spv` regenerated on
      2226 — vert 6208, frag 248628 bytes; Corrections 12.)
- [x] Tests: the Phase 1 and Phase 2 scenes as `rt_canvas_metal.rs` rows.
      (`gradients_match_the_software_oracle_on_metal` over `scenes/gradients_on_black.mfb`
      — every Phase 1 and 2 case plus two stroked gradients under Multiply; the picture
      row gains a blended stroked picture with a clear tint; Vulkan row `gradientsblack`
      — Corrections 13.)

Acceptance: both GPUs match the software renderer within tolerance, and the layout
guards pass.
  Check: `cargo test --bin mfb canvas` → pass (est. 3 min, guess);
  `cargo test --test rt_canvas_metal` → pass with `gpuFrames≠0` (est. 5 min, guess);
  `scripts/test-canvas-vulkan.sh <focal scene>` and `<spread scene>` on box 2228 →
  within `GPU_DEFAULT` (est. 15 min, guess);
  `scripts/test-canvas-gpu-rows.sh <focal scene> --target windows-x86_64` on box 206
  (the only real GPU) → within tolerance (est. 10 min, guess).
  Result: `cargo test --bin mfb canvas` → 111 passed; `cargo test --test rt_canvas_metal
  --test rt_canvas_golden` → 45 and 22 passed (`gpuFrames≠0` asserted per row; the
  gradient and picture rows re-run after Corrections 13 → passed); `test-canvas-gpu-rows.sh
  target/release/mfb --target linux-aarch64 --rows gradientsblack` on 2226 → passed, worst=1
  0.9458%; the same with `--target windows-x86_64` on 206 → passed, worst=1 0.7720%;
  pathclips, groupsblack, xformblack and matrix on both → passed;
  `scripts/test-canvas-vulkan.sh target/release/mfb --box 2226 --target linux-aarch64` →
  passed, main scene worst=2 0.8160%. Focal and spread are both in `gradientsblack`.
  app-mouse-surface goldens regenerated (5 synced, 6 rewritten).
Commit: e28f90a56

### Phase 4 — Docs and spec

- [x] `06_canvas.md`:
      - the gradient is in item space, with the transform order spelled out;
      - the linear fold;
      - the radial inverse;
      - the focal quadratic and its clamps;
      - the spread formulas;
      - rewrite :438–442 for `strokeApply` and the tint;
      - the layout growth.
      (*`Paint.gradient` replaces an item's fill colour with a ramp* rewritten: item space
      and the transform order, the `t` table, the fold, the radial inverse, the focal root
      and clamps, the spread, the stroke tint, the GPU words; the group paragraph and
      bullet; `strokeStyle` "of its 368".)
- [x] `.ai/canvas-threading.md`: the new block words and the `surface.w` bit packing.
      (The block history to 368, the 67-float header, `surface.w`'s packing, and the
      split's fill-off marker.)
- [x] Man prose on `Gradient`, `GradientSpread`, `GradientApply` and `Paint.gradient`
      (`.ai/man-content.md` bans apply). (Written with the types in Phase 1.)

Acceptance: man and spec render and cite.
  Check: `mfb man canvas types` shows the new fields and enums and no `fillGradient`;
  `bash scripts/man-census.sh --memory-scope` → 0 unclassified;
  `bash scripts/man-run-examples.sh canvas --run` → pass;
  `bash scripts/spec-census.sh --citations` and `--links` → 0 dangling;
  `cargo test --bin mfb spec` → pass (est. 8 min, guess).
  Result: `mfb man canvas types` → 0 `fillGradient`, `GradientSpread`, `GradientApply`
  and the five fields present; `man-census.sh --memory-scope` → "unclassified
  memory-vocabulary hits: 0"; `man-run-examples.sh canvas --run` → "examples: 32 built:
  32 ran: 32 not run: 0 failed: 0"; `spec-census.sh --citations` → MISS-PATH 0, MISS-LINE
  0, MISS-SYMBOL 0; `--links` → unresolved=0; `cargo test --bin mfb spec` → 46 passed.
  Runtime proof (Validation Plan): `/tmp/p163/demod` built `--app --debug`, run with
  `MFB_CANVAS_GPU=1 MFB_CANVAS_SYNC=1 MFB_CANVAS_DUMP=…` → `metal=TRUE gpuFrames=90
  frames=90`; frame 90 shows the elliptical radial ramp turned with its rotating item, a
  sphere's off-centre focal highlight, Repeat stripes, and a ramp-painted stroke tinted
  half-way to red.
Commit: 8d083c8c3

## Validation Plan

- Tests: per phase, with hand-computed software pixels. Negative cases: a degenerate
  `gradient.transform`, a zero radius, a focal point outside the circle, a
  `focalRadius` above `r`, and a gradient with fewer than 2 stops under
  `FillAndStroke` (a flat stroke).
- Coverage check: the software paths run in `rt_canvas_rasteriser`/`rt_canvas_golden`.
  The shader paths run only in `rt_canvas_metal` (confirm `gpuFrames≠0`) and the Vulkan
  scripts on 2228 and 206.
- Runtime proof: an `--app` program on macOS with `MFB_CANVAS_GPU=1`, screenshotted.
  It shows:
  - a rotating item carrying an elliptical radial gradient;
  - an off-centre focal highlight on a sphere;
  - Repeat stripes;
  - a gradient-painted stroke tinted red.
- Doc sync: Phase 4.
- Final gate (run ONCE, after Phase 4): `cargo test --no-fail-fast` (est. 40 min,
  from plan-159-D's recorded run) and `bash scripts/artifact-gate.sh target/release/mfb
  all` → 0 diffs outside `tests/syntax/app/app-mouse-surface` (est. 20 min).

## Open Decisions

None. The design was settled with the user on 2026-09-28. Choices not raised with the
user are recorded as decisions in §3.

## Corrections

1. **The populations grew with B and C.** `grep -rn "fillGradient" --exclude-dir=target
   --exclude-dir=.git --exclude-dir=planning --exclude-dir=bugs --exclude-dir=.claude . |
   wc -l` → 105 (not 101), `"fillGradient :="` → 42 (not 38), `"canvas::Gradient\["` →
   43 (not 39), measured at 81b47828a. The scope is the same rename and the same five
   fields at each site; nothing else was derived from those counts.
2. **No test's pixels move.** Every file listing `fillGradient` was read: no gradient item
   carries a `paint.transform`. A group offset already moved the ramp — the software
   ramp read the group-relative point since plan-116-G §4.5
   (`a_gradient_inside_a_group_moves_with_the_group`) — so only `paint.transform` changes
   pixels, and no golden or hand test exercises it. `git diff --stat tests/golden/canvas`
   is empty after Phase 1.
3. **A gradient on a Line or Arc draws nothing today.** `__canvas_paintHeader` zeroes
   their stop count (plan-116-F Correction F17), and
   `a_gradient_on_a_stroke_only_kind_is_ignored` pins it. So `FillOnly` keeps that flat
   line, and only `FillAndStroke` gives the two kinds the stops — through
   `__canvas_strokeRampTail` in both record builders.
4. **The native builder never builds a gradient paint.** `emit_paint_supported` declines
   every paint with two or more stops (`func_geo_build.rs`), so a gradient record is
   always the MFBASIC builder's and "both builders" is the MFBASIC one plus the native
   header length (`GEO_HEADER` 67). The "refuses a radial with a transform until Phase 2"
   clause is already true of every gradient. The linear fold is skipped for the identity
   transform: folding the identity would project the start point onto the axis line
   through the origin, the same `t` in exact arithmetic but rounded differently, which
   would break §3's byte-identical sub-gate.
5. **The app-mouse-surface `.ir` is not the type surface only.** It records every canvas
   helper global with its source line, so any helper edit moves it (the mechanism of
   plan-163-C's Correction 3). Its diff is the new slot globals, the shifted line
   numbers, and the changed helpers.
6. **`the_group_drain_does_not_depend_on_the_scene_changing`'s bound was one buffer's
   size, and the buffer grew.** Written in plan-116-G Phase 5 (G7) as "a bound of one
   outstanding buffer"; it guards that retired group buffers drain while the scene never
   changes; nothing else depends on it. Proof the number is stale: one installed
   eight-rectangle group measures `groupBytes=4082` at 81b47828a (a detached worktree,
   `MFB_CANVAS_STATS`) and 4914 now — 104 bytes an item, the five new `Gradient` words;
   the failure's 4923 is that one buffer with a 9-byte longer name. The bound is now
   8192 (one buffer, not two) and its comment the measured size.
7. **A plan-163-C leftover.** `tests/cli/cli_canvas_package.rs`'s
   `macos_paint_zero_values_are_no_ops` still read `filled.clip.w`, a `Bounds` field C
   removed, so the program did not build — at C's HEAD too. Its contract (the zero clip
   is the no-op) is now `len(filled.clip) <> 0`. C's own checks did not run that suite.
8. **`focalRadius` is clamped to `r (1 − 2^-10)`, not `r`.** §3 said `fr ∈ [0, r]` keeps
   the leading coefficient strictly negative; at `fr = r` the focal offset is clamped to
   0 and `a = 0 − 0 = 0`. The upper clamp `r (1 − 2^-10)` keeps `r − fr ≥ r · 2^-10`, so
   `a < 0` whenever `r > 0`.
9. **Under `FillAndStroke` the stroke alpha is the tint's strength, unfaded, and a
   zero-alpha stroke still strokes.** §3 gives `a = g.a` and a tint at strength
   `stroke.a`, and "any colour at alpha 0 draws the plain ramp" — but the header's stroke
   alpha (slot 15) was faded by `transparency` and `__canvas_strokeHalf` reported a
   zero-alpha stroke as no stroke. So under `FillAndStroke` slot 15 keeps the raw alpha
   (the ramp's stops carry the fade, as a picture's texels do) and `__canvas_strokeHalf`
   strokes a zero-alpha stroke when the paint's gradient has two or more stops. Its
   native twin only sees paints with fewer than two stops, where nothing changed.
10. **A pre-existing bug: the blend split drew a gradient's or picture's fill twice.**
    `emit_split_or_publish` publishes a blended item that fills and strokes as a fill
    record and a stroke record, and made record two stroke-only by zeroing the fill
    alpha — but a gradient's colour (since plan-116-F) and a picture's (since plan-163-C,
    whose fill alpha is a tint) do not come from it, so record two drew the fill again
    under the blend; and the split test, fill alpha > 0, sent a stroke-only paint with a
    ramp down the single path. Measured on Metal over the gradient scene before the fix:
    the Multiply rectangle 12492 pixels off by up to 54, the stroke-only one 4390 by up
    to 43. Fixed in both emitters, the native counter `emit_block_instances` and
    `__canvas_blockInstances` (which must agree, or every later draw base shifts — the
    first attempt changed only the MFBASIC counter and moved a later item's pixels by
    160): "fills" is fill alpha > 0, two or more stops, or a picture, and record two marks
    its fill off with alpha −1, which both shaders skip. After: both within 1.
11. **`the_metal_fragment_clips_and_samples_in_shape_space` pinned the ramp at `p`.** It
    guards that the fragment samples in shape space, not at `in.pos.xy` (plan-116-H); the
    ramp now takes `q`, the inverse-mapped twin of `p` (this plan's item-space decision),
    so the pinned call is `gradientColour(q, edges, item)` and the line defining `q` from
    `p` is pinned beside it.
12. **Box 2228 is down; 2226 stands in, as in plan-163-C (Correction 6).** The `.spv` were
    regenerated with `MFB_SPIRV_PORT=2226 scripts/regen-spirv.sh`; the Linux Vulkan rows
    ran on 2226 (`--target linux-aarch64`) and the same rows on 206's real GPU.
13. **The Vulkan row is `gradientsblack`, and its blended items are small.** A row named
    `gradients` already ran `many_gradients.mfb`; `test-canvas-gpu-rows.sh` runs every
    table row matching a name, so both ran under one label. Renamed. And the first scene
    put 2·180×80 px of Multiply over grey: on 2226's lavapipe 2.27% of the frame drifted
    one step (worst 1; 5452 and 5833 of those pixels in the two blended items), past
    `GPU_DEFAULT`'s 2% — the blend-precision drift the script's header already records.
    The two items exist for the split, so they are 80×56 now.

## Summary

The engineering risk is in three places:

- the item-block growth, with the `surface.w` bit repacking touching every radial test;
- the focal quadratic's fp32 precision on the GPUs;
- the deliberate pixel changes to gradients on transformed items, which are measured
  first and re-derived by hand.

Linear gradients cost nothing new at draw time, because they fold at build.
Untouched: stop interpolation, Text, PolyLine and Picture colouring, and every gradient
in the byte-identical sub-gate.
