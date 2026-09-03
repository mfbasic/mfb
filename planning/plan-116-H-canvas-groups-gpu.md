# plan-116-H: Groups on the GPU — one instanced draw per group node

Last updated: 2026-08-31
Effort: large (3h–1d)
Depends on: plan-116-G

plan-116-G renders groups on the software path and makes both `*Renderable`
predicates **decline** any scene containing a `Group`, so a GPU-accelerated program
silently falls back to the oracle the moment it uses one. This letter removes that
decline.

The mechanism is the one the feature request specifies, and it is why plan-116-A moved
the item block into a buffer in the first place: a group is **one instance buffer of N
item blocks**, drawn as a single `drawPrimitives:…instanceCount:N` (Metal) /
`vkCmdDraw(…, N, …)` (Vulkan) per group node, with the shader indexing by instance id.
The node's accumulated `(dx, dy)` rides as a per-draw `vec2` **bound to both stages**:
the vertex stage offsets the quad, and the fragment stage computes `p = fragCoord -
offset` before evaluating the distance field, because signed distance fields are
evaluated in absolute pixel coordinates. The quad is clamped to the surface **after**
offsetting.

Behavioral outcome: with `MFB_CANVAS_GPU=1`, a scene containing nested `Group` nodes
renders on Metal and on Vulkan, matching the software oracle within
`Tolerance::GPU_DEFAULT`, with `MFB_CANVAS_STATS` reporting `metalReady=TRUE` /
`vulkanReady=TRUE` — and the same scene drawn as one flat list renders identically.

References:

- plan-116-A §4.2–4.3 — the instance buffer and the instanced draw this letter reuses.
- plan-116-G §4.4–4.5 — the resolved group tree and the accumulated-offset rule.
- `.ai/canvas-threading.md` §10 — the two `*Renderable` predicates, why they differ,
  and the recorded incident where a predicate accepted a kind its shader did not know
  (4,536 pixels wrong, reported as success).
- `src/codegen/builtins/canvas/helper_render.rs:122` — `__canvas_sceneOffsets`, the
  scene walk both backends share.

## Prerequisites

See plan-116-A §Prerequisites for the three environment gates. All three are load-
bearing here: this letter cannot be verified without a Metal host, a Vulkan box, and a
SPIR-V regen box.

| Must be true | Command | Status |
|---|---|---|
| plan-116-G complete and archived | `ls planning/completed/plan-116-G-*` → one match | NOT MET |

If plan-116-G is not complete, this letter cannot start, full stop. G produces the
resolved group tree; without it there is nothing for a backend to walk.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command before you continue and again before you stop.

## 1. Goal

- Both `*Renderable` predicates accept a scene containing `Group` nodes.
- Each group node is one instanced draw over its own item buffer.
- The accumulated offset reaches both shader stages and is applied per §4.2.
- GPU output matches the software oracle within `Tolerance::GPU_DEFAULT` for flat,
  nested, and diamond-shaped group trees.

### Non-goals (explicit constraints)

- **No change to group storage, lifetime, resolution or the depth limit.** All of
  that is plan-116-G and this letter must not touch it.
- **No resource ownership.** plan-116-J (behind the plan-116-I `RES` migration).
- **No change to the software renderer.** It is the oracle; if the GPU disagrees, the
  GPU is wrong.
- **Glyph runs stay N draws.** plan-116-A §4.3 decided this; a group containing text
  issues its glyph draws with the group's offset applied, not a different scheme.
- **No flattening.** A nested group is a separate draw with a composed offset, never
  merged into the parent's buffer — the same rule plan-116-G's design rests on.
- **No existing golden may move.**

## 2. Current State

### What plan-116-A left in place

Every item's parameters live in a per-frame buffer of `ITEM_BLOCK_SIZE` records, and
each item is drawn with an instanced draw indexed by instance id. `ITEM_BLOCK_SIZE` is
**224** bytes after plan-116-F. Crucially for this letter, **Metal's polygon edges
and gradient stops also live in regions of that frame buffer** (plan-116-A §4.1 as
revised; plan-116-F §4.2), with per-item base indices in the block — so a polygon
or gradient item rides *inside* an instanced run on both backends, and "one
instanced draw per group node" is actually achievable. Only a `Text` item still
breaks a run into its own glyph draws. The flat scene is one buffer; the draw
walks it.

### What plan-116-G left in place

`present` publishes a **resolved tree**: each `Group` node carries a slot index and a
revision instead of a name, and the group's own items are a separate published block.
`__canvas_renderScene` walks it with an accumulated `(dx, dy)`.

`__canvas_sceneOffsets` (`helper_render.rs:122`) is the shared scene walk both backends
consume — *"flat items then layers in order, one hash index across both — reduced to
what a backend actually needs: the cache offset of each item's geometry, in draw
order."* It is flat. **A group tree is not flat**, so this letter's central change on
the CPU side is that the walk must yield `(offset, dx, dy)` triples and a draw
grouping, not a flat offset list.

### Measured populations

| What | Count | Command |
|---|---|---|
| `ITEM_BLOCK_SIZE` after plan-116-F | 224 | plan-116-F §4.2 |
| Backends to convert | 2 | Metal (`src/target/macos_aarch64/app/metal.rs`), Vulkan (`src/codegen/runtime/canvas/vulkan.rs`) |
| Shader files to edit | 3 | `metal.rs:METAL_SHADER_SOURCE`, `shaders/mfb_canvas.vert`, `shaders/mfb_canvas.frag` |
| `*Renderable` predicates | 2 | `__canvas_metalRenderable` and `__canvas_vulkanRenderable` in `helper_render.rs` (`grep -n 'FUNC __canvas_.*Renderable' src/codegen/builtins/canvas/helper_render.rs`) |
| Shared scene walk | 1 | `helper_render.rs:122` (`__canvas_sceneOffsets`) |

> **Census re-verified 2026-09-02 (pre-execution).** Still 2 backends, 3 shader
> sources, 2 `*Renderable` predicates and 1 shared scene walk. The
> `ITEM_BLOCK_SIZE after plan-116-F | 224` row depends on plan-116-F landing its own
> corrected figures — see **F1** there, which took F's header from 42→48 to 41→47; the
> block size 192 → 224 is unaffected, since plan-116-E landed 192 as F assumed.

### Verified properties

- **The offset must reach the fragment stage, not just the vertex stage.** Read both
  fragment shaders: `geoDistance` is called with `gl_FragCoord.xy` / `in.pos.xy`, which
  is the absolute framebuffer pixel centre (`metal.rs:76` states this explicitly for
  MSL, `mfb_canvas.frag:12` for GLSL). The shape's parameters in the item block are
  also absolute. So offsetting only the quad would move *where the shape is rasterised*
  without moving *the shape*, drawing a translated window onto an un-translated shape.
  This is the single most likely way to implement the feature wrongly and it would look
  almost right.
- **The quad clamp must follow the offset.** Both vertex shaders map the item's `quad`
  to clip space by dividing by `item.surface`. Clamping before offsetting would clip a
  group against the surface rectangle it has not been moved into yet.
- **plan-116-A already established a flat varying between the stages** (the instance
  index). The offset can ride the same mechanism, so no new interface between the
  stages is invented here.
- **UNVERIFIED: whether a per-draw offset is cheaper as a push constant / `setBytes`
  than as a per-item field in the buffer.** Both work. §3 recommends the per-draw
  route because it is what the feature request specifies and because it keeps a
  group's buffer *offset-independent* — which is what lets one group be drawn at two
  offsets from one buffer. Phase 1 confirms the buffer stays shared.

## 3. Design Overview

Four pieces:

1. **The scene walk yields a draw list**, not a flat offset list: a sequence of
   `(itemBufferBase, count, dx, dy)` draws, produced by the same depth-first walk
   `__canvas_renderScene` does. §4.1.
2. **Per-draw offset**, pushed to both stages — a push constant on Vulkan (the block is
   now small: two words), `setVertexBytes:`/`setFragmentBytes:` on Metal. §4.2.
3. **Shader arithmetic** — vertex offsets and *then* clamps; fragment subtracts before
   `geoDistance`. §4.2.
4. **The predicates accept groups**, with the frame-item-count cap now counting every
   instance across every group node. §4.3.

**Where the correctness risk concentrates:** the fragment-stage offset (§2). Getting it
half-right — vertex only — produces a plausible picture that is wrong everywhere a
group is translated, and no existing golden would catch it because no existing golden
has a group. Phase 2's very first test is a group at a non-zero offset compared
pixel-for-pixel against the oracle.

**Where the design uncertainty concentrates:** whether one group's buffer can serve two
draws at different offsets (the diamond case). It is the property that makes groups
worth having, and §3's per-draw-offset choice is what delivers it. Phase 1 proves it.

**Byte-identity is NOT this letter's gate.** **Expected NOT to diff:** every existing
golden, and — importantly — the *software* output of every group scene plan-116-G
added, since this letter must not touch the oracle. **Expected to diff:** `.ncodesum`
on every canvas-emitting target, and both `.spv` blobs.

### Rejected alternatives

- **Bake the offset into each item block when the buffer is written.** Rejected: it
  makes a group's buffer offset-specific, so the diamond case (one group, two parents,
  two positions) would need two buffers — losing the sharing that is the feature's
  point. It would also mean rewriting the buffer whenever a node moves, turning a
  two-float change into an N-item copy.
- **Carry the offset as a per-item field in the buffer.** Rejected for the same
  reason, one step weaker: the buffer would have to be rewritten per draw.
- **Flatten the tree on the CPU into one draw list with pre-offset items.** Rejected:
  it is `bake the offset` plus the flattening plan-116-G already rejected, and it
  discards the instance-buffer sharing entirely.
- **Apply the offset by translating the viewport per draw.** Rejected: the viewport is
  dynamic state on both APIs, but it also clips, so a group translated partly off its
  parent's area would be clipped by the viewport rather than by the surface — a
  different and wrong behaviour.

## 4. Detailed Design

### 4.1 The draw list

`__canvas_sceneOffsets` gains a sibling — `__canvas_sceneDraws` — that performs the
same depth-first walk `__canvas_renderScene` performs, emitting one entry per
contiguous run of non-group, non-text items at a given offset (a `Text` item also
ends a run: it is its own glyph draws, issued at the current offset, per
plan-116-A §4.3):

```
(itemBase, itemCount, dx, dy)
```

A group node ends the current run, emits the group's own runs recursively at the
composed offset, and starts a new run after it. So a scene of `[rect, Group(A) @ (10,20),
circle]` where A is `[c1, c2]` yields three draws: `(0,1,0,0)`, `(A,2,10,20)`,
`(2,1,0,0)`.

Keeping `__canvas_sceneOffsets` alongside it, rather than replacing it, is deliberate:
it is also what feeds the geometry-cache warm-up (`helper_render.rs:75` — *"Generating
it costs nothing extra when the Metal path declines, because `__canvas_geometryFor` is
the cache and the software walk that follows hits it"*), and that role is unchanged.

### 4.2 The offset in the shaders

**Interface.** A two-word per-draw payload `ivec2 offset` in 16.16, pushed before each
draw:

- **Vulkan** — a push constant. The item block left the push-constant range in
  plan-116-A, so the range is free; two words is trivially inside the 128-byte
  guarantee.
- **Metal** — `setVertexBytes:` and `setFragmentBytes:` at a dedicated buffer
  index. Per-DRAW state through `setBytes:` is fine — the conflict plan-116-A
  removed was per-ITEM payloads inside one instanced draw; the offset changes only
  between draws, which is exactly what `setBytes:` is for (and what the glyph
  draws still use for their bitmaps).

**Vertex stage:**

```
corner = <the item's quad corner, as today>
corner += offset            // offset FIRST
gl_Position = clip_space(clamp_to_surface(corner))   // clamp AFTER
```

**Fragment stage:**

```
vec2 p = gl_FragCoord.xy - offset;   // back into the group's own coordinates
… geoDistance(p) …                   // unchanged
```

Everything downstream of `p` — every distance function, the coverage rule, the stroke
band — is unchanged, because they take `p` and nothing else positional.

**Two things are not downstream of `p` today, and they go opposite ways (H2).**

* **The clip must stay at `gl_FragCoord.xy`.** `Paint.clip` is defined in *surface*
  pixels (plan-116-B §Non-goals), so a group's translation moves the shape and not the
  clip rectangle. It already reads `gl_FragCoord.xy` / `in.pos.xy`; leave it.
* **The glyph arm must MOVE to `p`, and never sees it.** `item.misc.x == 6` returns
  *before* `shapeDistanceAndScale` is ever called
  (`sed -n 419,432p src/codegen/runtime/canvas/shaders/mfb_canvas.frag`), so introducing
  `p` at the `geoDistance` call site leaves `Text` reading `gl_FragCoord.xy` — and it
  indexes the cached bitmap as `int(floor(gp.x)) - item.shape.x`, with `item.shape.x`
  the glyph origin in the item's own coordinates. A `Text` item in a translated group
  therefore samples the wrong texels: shifted by the offset, and blank once the offset
  exceeds the glyph's width. Both the transformed and untransformed branches of that
  line take `p`.
* **The gradient must MOVE to `p`, and does not read it today.** plan-116-F evaluates
  the ramp at the surface point — `gradientColour(gl_FragCoord.xy)`
  (`mfb_canvas.frag:442`), `gradientColour(in.pos.xy, …)` in MSL, and `px`/`py` against
  `gradFX`/`gradFY` in the oracle (`helper_items.rs`) — against an axis authored in the
  item's own coordinates. Under a translation those disagree: the shape moves and the
  ramp does not, so a group drawn at an offset shows its gradient sliding across it,
  and the same group drawn twice at two offsets shows two *different* pictures. Switch
  all three to `p`.

`grep -n gl_FragCoord src/codegen/runtime/canvas/shaders/mfb_canvas.frag` returns
**four** call sites and that is the whole list: `:419` clip (stays), `:428` glyph
(moves), `:436` `shapeDistanceAndScale` (the one §4.2 already describes), `:442`
gradient (moves). Everything else — the arc cap discs at `:217`, the ellipse, the
stroke band — is computed against the point those four pass in, so it follows for free.
Do the same grep on `METAL_SHADER_SOURCE` for the MSL twin.

None of this is a reasoning step — **Phase 2 and Phase 3 must each test the clipped
item, the gradient-filled item and the `Text` item inside a translated group.** The
diamond scene is the sharp one: one group, two offsets, one buffer, and the two draws
must be the same picture translated.

### 4.3 The predicates

Both `*Renderable` predicates stop declining `Group`. Two things change in what they
count:

- The **frame item count** (plan-116-A §4.1, `CANVAS_MAX_FRAME_ITEMS`) must count every
  instance a group tree expands to, not the number of scene nodes — a diamond drawn
  twice costs two draws but the *buffer* is shared, so what is capped is the draw
  count and the total instance count, and both must be summed over the resolved tree.
- **Both backends'** frame-total caps (`VULKAN_MAX_FRAME_EDGES` /
  `METAL_MAX_FRAME_EDGES`, glyph samples, and the gradient-stop caps — Metal gained
  its frame regions in plan-116-A and plan-116-F) must likewise sum over the
  resolved tree, counting a shared group **once per reference** if its payload is
  re-uploaded per draw, or **once** if it is not. Decide this explicitly in Phase 1
  and make each predicate match its emitter; a predicate that counts differently
  from the emitter is the class of bug `.ai/canvas-threading.md` §10 records.

## Compatibility / Format Impact

- **No new `canvas::` surface.** Groups already exist after plan-116-G; this letter
  makes them GPU-renderable.
- **Observable change:** a scene with a `Group` now uses the GPU when one was asked
  for, where it previously fell back to software. Output is expected to move by at most
  `Tolerance::GPU_DEFAULT`, exactly as it does for every other kind.
- **`.ncodesum` churn**; both `.spv` blobs regenerate.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick in the same commit as the
> work; `- [~]` for partial with a one-line remainder; fill `Commit:` on landing.
> **An unticked box means NOT DONE.**

### Phase 1 — The draw list, and the buffer-sharing question settled

CPU-side only; no shader change, no predicate change. Both backends still decline.

- [ ] Add `__canvas_sceneDraws` per §4.1, beside `__canvas_sceneOffsets`.
- [ ] **Decide and record**, in §4.3, whether a shared group's edge/glyph/gradient
      payload is uploaded once or once per reference — and make the Vulkan predicate's
      sum match. Record the choice with the reason.
- [ ] Tests: `tests/rt_canvas_rasteriser.rs` asserts `__canvas_sceneDraws` produces the
      expected `(base, count, dx, dy)` sequence for: a flat scene; one group; a nested
      group; a diamond. Assert the **diamond's two draws name the same item base** —
      that is the buffer-sharing property this whole letter is arranged around.

Acceptance: the four draw-list cases pass, the diamond shares a base, and every
existing golden and every plan-116-G group scene is byte-identical (the software
renderer is untouched).
Commit: —

### Phase 2 — Vulkan: the offset, both stages

Vulkan first, as in plan-116-A, because glslang gives measured reflection.

- [ ] Add the `ivec2 offset` push constant to both GLSL files; vertex offsets then
      clamps; fragment subtracts before `geoDistance`, and **evaluates the clip at
      `gl_FragCoord.xy`** (§4.2).
- [ ] `scripts/regen-spirv.sh`.
- [ ] Convert the Vulkan emitter to walk `__canvas_sceneDraws`, pushing the offset and
      issuing one instanced `vkCmdDraw` per draw entry.
- [ ] Remove the `Group` decline from `__canvas_vulkanRenderable`; update its frame
      caps to sum over the resolved tree per §4.3.
- [ ] Tests: on a Vulkan box, a group at `(0,0)` matches the oracle; a group at
      `(37, 53)` matches the oracle; a nested group matches; a diamond matches; a
      **clipped** item inside a translated group matches (the §4.2 clip case); and a **gradient-filled** item and a **`Text`** item
      inside a translated group match (the §4.2 gradient and glyph cases, **H2** —
      both fail today).

Acceptance: all five scenes match the software oracle within
`Tolerance::GPU_DEFAULT` with `MFB_CANVAS_STATS` reporting `vulkanReady=TRUE`. The
non-zero-offset case is the one that proves the fragment-stage offset landed; a pass
there with a vertex-only implementation is not possible.
Commit: —

### Phase 3 — Metal: the same

- [ ] Add the offset to `METAL_SHADER_SOURCE`, bound via `setVertexBytes:` and
      `setFragmentBytes:` at a dedicated index; same offset-then-clamp and
      subtract-before-`geoDistance` arithmetic; clip at `in.pos.xy`.
- [ ] Convert the Metal emitter to walk `__canvas_sceneDraws`, issuing one
      `drawPrimitives:vertexStart:vertexCount:instanceCount:` per draw entry.
- [ ] Remove the `Group` decline from `__canvas_metalRenderable`; update its caps.
- [ ] Tests: the same seven scenes in `tests/rt_canvas_metal.rs` — five, plus the
      gradient-in-a-translated-group and `Text`-in-a-translated-group cases (**H2**).

Acceptance: all five scenes match the oracle within `Tolerance::GPU_DEFAULT` with
`metalReady=TRUE`.
Commit: —

### Phase 4 — The reference image, docs, and the gates

- [ ] New reference image `tests/golden/canvas/groups.png`: a group at the origin, the
      same group at an offset, a nested group, and a diamond — enough that a
      vertex-only offset, a missing clamp reorder, or a flattened diamond each change
      it visibly.
- [ ] Assert `groups.png` on all three renderers: software exactly, both GPUs within
      `Tolerance::GPU_DEFAULT`.
- [ ] `.ai/canvas-threading.md` §10 — record that a group is one instanced draw per
      node with a per-draw offset bound to both stages, and **why the fragment stage
      needs it** (SDFs are absolute). This is the fact a future reader is most likely
      to get wrong.
- [ ] `src/docs/spec/app/06_canvas.md` — note that a group's translation moves the
      geometry but not `Paint.clip`, which stays in surface pixels.
- [ ] `scripts/man-census.sh --memory-scope` → 0 unclassified hits;
      `scripts/man-run-examples.sh canvas --run` passes.
- [ ] `scripts/regen-ncodesum.sh`; prove the delta is this letter's.

Acceptance: `groups.png` matches on all three renderers; `cargo test --no-fail-fast`
green on mac+RELEASE and linux+DEBUG; `scripts/test-accept.sh` green;
`scripts/artifact-gate.sh all` 0 diffs.
Commit: —

## Validation Plan

- **Tests:** `tests/rt_canvas_rasteriser.rs` (draw-list ×4),
  `tests/rt_canvas_metal.rs` (×5), the Vulkan golden cases (×5),
  `tests/rt_canvas_golden.rs` (+`groups.png`). Negative cases: a scene exceeding the
  frame instance cap must **decline to software** (assert via `MFB_CANVAS_STATS`, never
  by pixel equality — a declined frame equals the oracle by construction, which is the
  false pass `.ai/canvas-threading.md` §10 names); a group naming an absent group draws
  nothing on the GPU too.
- **Coverage check:** the emitters are compiler code — confirm the new lines are in the
  denominator with `cargo llvm-cov --bin mfb`. The predicates and the scene walk are
  MFBASIC source, covered by the rt cases; confirm the group-present and group-absent
  arms of each predicate are both exercised.
- **Runtime proof:** render `groups.png`'s scene three ways and diff. Separately,
  render the diamond scene and the equivalent flat scene (the same shapes written out
  twice at the two positions) and assert the two frames are identical — the strongest
  available check that grouping changes nothing but cost.
- **Doc sync:** `.ai/canvas-threading.md` §10; `src/docs/spec/app/06_canvas.md`.
- **Acceptance:** `cargo test --no-fail-fast`, `scripts/test-accept.sh`,
  `scripts/artifact-gate.sh all`, `rustup run 1.96.0 cargo fmt --all &&
  (cd repository && rustup run 1.96.0 cargo fmt)`.

## Open Decisions

- **Per-draw offset rather than per-item (§3).** Recommended and effectively decided:
  it is what keeps one group's buffer usable at two positions, which is the diamond
  case and the feature's whole economic argument.
- **Whether a shared group's edge/glyph/gradient payload uploads once or per
  reference (§4.3).** Genuinely open; **decide it in Phase 1** and make the predicate
  and the emitter agree. Recommend **per reference** to start, because it is the
  simpler emitter and the caps are generous; revisit if a real scene approaches them.
- **Keeping `__canvas_sceneOffsets` alongside `__canvas_sceneDraws` (§4.1).**
  Recommended: it still feeds the geometry-cache warm-up, which is a separate job from
  the draw list.

## Corrections

**H2 (2026-09-03, pre-execution) — §4.2's list of "everything downstream of `p`" names
the gradient, and the gradient is not downstream of `p`.** The sentence reads *"every
distance function, the coverage rule, the stroke band, the clip test, the gradient
parameter — is unchanged, because they all take `p` and nothing else positional"*, then
flags the clip as the single exception. Two errors in one sentence: the clip is not
"downstream of `p`" either (that is why it is the exception), and neither is the
gradient — but for the opposite reason and with the opposite fix.

Measured. plan-116-F landed the ramp evaluated at the **surface** point on all three
renderers:

* `grep -n 'gradientColour(' src/codegen/runtime/canvas/shaders/mfb_canvas.frag`
  → `:442  ivec4 fillRgba = item.ellipse.z >= 2 ? gradientColour(gl_FragCoord.xy) : item.fill;`
* the MSL twin passes `in.pos.xy`
  (`grep -n 'gradientColour(in.pos.xy' src/target/macos_aarch64/app/metal.rs`)
* the oracle uses the loop's `px`/`py` against `gradFX`/`gradFY`
  (`sed -n 500,510p src/codegen/builtins/canvas/helper_items.rs`)

and the axis itself comes from the item's own geometry record, authored in the item's
coordinates. Untranslated those coincide, which is why plan-116-F is correct and its
goldens pass. Under a group offset they do not: the shape is drawn at `+ (dx, dy)` and
the ramp is not, so the gradient slides across the shape — and a **diamond**, one group
drawn at two offsets, renders two different pictures from one buffer, which is the
property this whole letter is arranged around.

The fix is one line per renderer (evaluate the ramp at `p`), so this is cheap — but
only if it is *done*. Left as §4.2 reads, an executor checks the sentence, sees the
gradient listed as already handled, and ships it. Recorded as a task in Phase 2 and
Phase 3 with the diamond named as the test that can see it.

A third case turned up on the second pass and is the reason the grep above is in the
letter rather than the finding: the **glyph arm returns before `geoDistance` is
reached**, so a fix applied at the `geoDistance` call site — which is where §4.2
puts it — silently misses `Text` entirely. Enumerate the `gl_FragCoord` call sites;
do not reason from the data flow.

Note the asymmetry is real and worth stating in the spec when H lands: a **clip** is a
surface rectangle and does not move with a group; a **gradient** is part of the item's
paint and does. `src/docs/spec/app/06_canvas.md`'s gradient subsection (plan-116-F)
currently says the ramp is "measured in surface pixels", which will need the group
qualification.

**H1 (2026-09-03, pre-execution) — one of this letter's three `helper_render.rs`
citations is stale; the other two are exact.** `:180` is given as the Metal
`*Renderable` predicate and is a line inside `__canvas_runSamples`
(`awk 'NR==180' src/codegen/builtins/canvas/helper_render.rs`); the predicate is
`__canvas_metalRenderable`, currently at `:198`, and plan-116-F moved it again when it
replaced the Phase 3 blanket gradient decline with a frame-total cap.

`:122` (`__canvas_sceneOffsets`) and `:75` (the geometry-cache warm-up comment, quoted
verbatim in §4) both still land exactly, which is worth stating: this is not a reason
to distrust the letter, it is one line to fix. Replaced with the two symbols and the
grep that finds them, per plan-116-G's **G1**.

Noted for Phase 2 and Phase 3, which both remove a `Group` decline from these two
functions: they are the same two functions plan-116-F edited, so expect them to have
moved again by the time H runs — find them by name.

- **C1 (2026-09-01, review — pre-execution).** Aligned with the revised plan-116-A:
  Metal's edges (A) and gradient stops (F) live in frame-buffer regions, which is
  what makes "one instanced draw per group node" true for groups containing
  polygons or gradients — under the original A, every polygon would have split the
  draw. `Text` inside a group is explicitly a run-breaker (§4.1). Ownership is
  plan-116-J behind the plan-116-I `RES` migration.

## Summary

The whole letter turns on one fact that is easy to miss and expensive to miss: a
signed distance field is evaluated in absolute pixel coordinates, so translating a
group means translating the *query point* in the fragment shader as well as the quad in
the vertex shader. An implementation that does only the vertex half produces a picture
that is wrong in a way that reads as a rasterisation quirk rather than a bug, and no
pre-existing golden covers it — which is why the first GPU test in Phases 2 and 3 is a
group at a deliberately awkward non-zero offset, checked against the oracle. The second
risk is quieter: `Paint.clip` is in surface pixels and must **not** move with the
group, so the fragment shader evaluates the shape at `p` and the clip at
`gl_FragCoord.xy`. Untouched: the software oracle, group storage and lifetime, and
resource ownership.
