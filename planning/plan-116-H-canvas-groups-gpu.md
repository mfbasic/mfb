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
| plan-116-G complete and archived | `ls planning/completed/plan-116-G-*` → one match | **MET** (2026-09-04: exactly one match, `planning/completed/plan-116-G-canvas-groups-storage.md`, archived by `155d32238`. Every G box resolved, 43 corrections. Final gates: unit 3782/0, nine canvas suites 0 failed, box 2228 RELEASE 3773/0 with the source hash-verified, test-accept 1377 ran, artifact-gate 1876 goldens 0 diffs, Vulkan 12/12 on 2228 glibc and 2227 musl. **Note the row asks for archived, not landed** — G's push to `main` is blocked by another session's uncommitted work in the shared checkout, recorded as G43; the work is committed on `worktree-P-116` and every gate stands.) |
| SPIR-V regen reachable (A's gate 1) | `scripts/regen-spirv.sh` | **MET** (2026-09-04: exit 0, "Glslang Version: 11:15.2.0", vert→4420 B, frag→36648 B, and `git status --short src/codegen/runtime/canvas/shaders/` **empty** — the checked-in blobs reproduce byte-identically, which this letter needs, since it edits both shaders.) |
| The Metal box runs `rt_canvas_metal` (A's gate 2) | `cargo test --release --test rt_canvas_metal --no-fail-fast` | **MET** (2026-09-04: **4 passed, 0 failed**, 205.05s.) |
| A Vulkan-capable Linux box (A's gate 3) | `ssh -p 2228 test@127.0.0.1 'ls /usr/share/vulkan/icd.d/'` | **MET** (2026-09-04: 7 ICDs including `lvp_icd.json`; `scripts/test-canvas-vulkan.sh` ran 12/12 on both 2228 glibc and 2227 musl at G's close.) |

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
| `ITEM_BLOCK_SIZE` after plan-116-F | **208** | `grep -n 'ITEM_BLOCK_SIZE: usize' src/codegen/runtime/canvas/mod.rs` → `= 208` (2026-09-04). The plan's 224 was read from F's §4.2 *design* text, not from the landed constant; F ended at 208 — see **H7**. |
| Backends to convert | 2 | Metal (`src/target/macos_aarch64/app/metal.rs`), Vulkan (`src/codegen/runtime/canvas/vulkan.rs`) |
| Shader files to edit | 3 | `metal.rs:METAL_SHADER_SOURCE`, `shaders/mfb_canvas.vert`, `shaders/mfb_canvas.frag` |
| `*Renderable` predicates | 2 | `__canvas_metalRenderable` and `__canvas_vulkanRenderable` in `helper_render.rs` (`grep -n 'FUNC __canvas_.*Renderable' src/codegen/builtins/canvas/helper_render.rs`) |
| Shared scene walk | 1 | `__canvas_sceneOffsets`, now `helper_render.rs:251` (`grep -n 'FUNC __canvas_sceneOffsets' src/codegen/builtins/canvas/helper_render.rs`, 2026-09-04) — plan-116-G inserted `__canvas_appendDraw` and the group globals above it. Cite the symbol, not the line. |

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
circle]` where A is `[c1, c2]` yields three draws — and **what the third one's base is
depends on §4.3's undecided question, which is the point (H3)**:

* If a shared group's blocks are written **once per reference**, inline, the flattened
  block order is `rect=0, c1=1, c2=2, circle=3` and the draws are
  `(0,1,0,0)`, `(1,2,10,20)`, `(3,1,0,0)`.
* If they are written **once** and referenced, the scene's own blocks are `rect=0,
  circle=1`, A's live at its own base `a`, and the draws are
  `(0,1,0,0)`, `(a,2,10,20)`, `(1,1,0,0)`.

The second is what makes a diamond share a buffer, which is what this letter is arranged
around — so it is the likely answer, but Phase 1 decides it and **the Phase 1 test's
expected sequence must be written from that decision**, not copied from here.

Keeping `__canvas_sceneOffsets` alongside it, rather than replacing it, is deliberate:
it is also what feeds the geometry-cache warm-up (`helper_render.rs:75` — *"Generating
it costs nothing extra when the Metal path declines, because `__canvas_geometryFor` is
the cache and the software walk that follows hits it"*), and that role is unchanged.

### 4.2 The offset in the shaders

**Interface.** A two-word per-draw payload `ivec2 offset` in 16.16, pushed before each
draw:

- **Vulkan** — a push constant. Two words is trivially inside the 128-byte guarantee,
  but the range is not "free" so much as **gone (H4)**: plan-116-A did not vacate it,
  it deleted it. `VkPipelineLayoutCreateInfo` is built with `rangeCount` 0 and a null
  `pPushConstantRanges`, and the comment there
  (`grep -n 'NO push-constant range' src/codegen/runtime/canvas/vulkan.rs`) records why
  that was deliberate: *"a layout that declares bytes no stage consumes is a layout the
  validation layers flag"*. So this phase re-adds the range **and** the two shader
  declarations in one step, with both stages consuming it — vertex offsets, fragment
  subtracts — or the layout is invalid in a way that only shows on a box with the
  validation layers on.
- **Metal** — `setVertexBytes:` and `setFragmentBytes:` at a dedicated buffer index.
  **`setVertexBytes:length:atIndex:` no longer exists and must be re-added (H5)**:
  plan-116-A *deleted* the selector from `metal_data_objects` rather than leaving it
  unused, because *"every selector in `metal_data_objects` is a C string emitted into
  every canvas binary and registered with the ObjC runtime at startup, so an unsent one
  is not free"* (`sed -n 570,588p src/target/macos_aarch64/app/metal.rs`).
  `setFragmentBytes:` survives — the glyph bitmaps still ride it — so only the vertex
  one is missing. Per-DRAW state through `setBytes:` is fine — the conflict plan-116-A
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
* **The gradient goes wherever plan-116-G decided, and it is not `p` today.**
  plan-116-F evaluates the ramp at the surface point —
  `gradientColour(gl_FragCoord.xy)` (`mfb_canvas.frag:442`),
  `gradientColour(in.pos.xy, …)` in MSL — against an axis read from the record with no
  transform applied, so a gradient is surface-anchored and `Paint.transform` does not
  drag it. Whether a *group* offset should is a semantic question plan-116-G **G5**
  settles for the oracle; this letter's job is to match whatever the oracle does, on
  both backends. Read G's answer before touching this line, and do not infer it from
  the clip — the clip is surface-anchored for a documented reason of its own.

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

  **DECIDED (Phase 1): once, referenced — not once per reference.** Three reasons, the
  first of which settles it on its own:

  1. **It is the only shape in which a per-draw offset earns its place.** The whole
     apparatus this letter adds — a Vulkan push constant, Metal `setVertexBytes:` — exists
     so two draws of one group differ *only* by a translation. If the blocks were
     duplicated per reference, the offset could simply be baked into each copy as it is
     written, and none of that machinery would be needed. Choosing duplication would be
     choosing to build the offset plumbing and then not need it.
  2. **It bounds what reuse costs.** `CANVAS_MAX_FRAME_ITEMS` is 4096. A UI drawing one
     200-item panel at thirty positions costs 200 blocks under sharing and 6000 under
     duplication — over the cap, so the frame would decline to software and the feature
     would be slowest precisely where it is used most.
  3. It is this letter's stated goal (1), reuse, expressed in the buffer rather than only
     in the source.

  **The consequence the predicates must encode: two different quantities are capped.**
  The number of **blocks** (which a diamond does *not* double) against
  `CANVAS_MAX_FRAME_ITEMS`, and the number of **draws** (which it does). The edge, glyph
  and gradient payloads follow the blocks, so they are summed **once per distinct
  resolved group**, not once per reference.

  Those sums land in **Phase 2 and Phase 3, with the decline removals**, not here. A
  predicate that summed the resolved tree while still declining every group scene would
  be unreachable code, and the plan's own instruction is to make each predicate match its
  emitter — the emitter does not exist until Phase 2. Recorded rather than deferred:
  the decision is made, and the phase that acts on it is named.

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

- [x] Add `__canvas_sceneDraws` per §4.1, beside `__canvas_sceneOffsets`.
- [x] **Decide and record**, in §4.3, whether a shared group's edge/glyph/gradient
      payload is uploaded once or once per reference — and make the Vulkan predicate's
      sum match. Record the choice with the reason.
- [x] Tests: `tests/rt_canvas_rasteriser.rs` asserts `__canvas_sceneDraws` produces the
      expected `(base, count, dx, dy)` sequence for: a flat scene; one group; a nested
      group; a diamond. Assert the **diamond's two draws name the same item base** —
      that is the buffer-sharing property this whole letter is arranged around.
      Derive the expected bases from the decision above, not from §4.1's example
      (**H3**): the example predates the decision and is arithmetically wrong under
      either answer.

Acceptance: the four draw-list cases pass, the diamond shares a base, and every
existing golden and every plan-116-G group scene is byte-identical (the software
renderer is untouched).

**MET.** `scene_draws_shares_one_base_between_a_diamonds_two_draws`
(`rt_canvas_rasteriser`) covers all four cases in one test, deliberately: the failures
are *relative*, so a walk emitting per-reference blocks passes the flat and single-group
cases unchanged and diverges only on the diamond, while a walk dropping the composed
offset passes everything except the nested case.

Measured, from `MFB_CANVAS_STATS` — the only window onto a structure built on the
graphics thread and handed straight to an emitter:

| case | `blocks=` | `draws=` |
|---|---|---|
| flat scene, two items | 2 | `0:2:0:0` |
| one group at (100,100) | 2 | `0:2:6553600:6553600` |
| nested, (500,100)+(0,200) | 1 | run drawn at `32768000:19660800` = (500,300) |
| **diamond, two references** | **2** | `0:2:6553600:6553600｜0:2:19660800:6553600` |

The diamond is the row that matters: **two draws, both base 0**, one set of blocks.

The software renderer is untouched — `__canvas_sceneOffsets` is kept alongside rather
than replaced, since it also feeds the geometry-cache warm-up. `rt_canvas_rasteriser`
58, `rt_canvas_golden` 13, `rt_canvas_font` 17, `rt_canvas_damage` 6, all 0 failed, so
no golden and no plan-116-G group scene moved.
Commit: a4c3357fa

### Phase 2 — Vulkan: the offset, both stages

Vulkan first, as in plan-116-A, because glslang gives measured reflection.

- [x] Add the `ivec2 offset` push constant to both GLSL files; vertex offsets then
      clamps; fragment subtracts before `geoDistance`, and **evaluates the clip at
      `gl_FragCoord.xy`** (§4.2). — `3ded46db6`. The range and both stage declarations
      had to land together (**H4**): a layout declaring bytes no stage consumes is what
      the validation layers flag.
- [x] `scripts/regen-spirv.sh`. — `3ded46db6`; both blobs grew (vert 4420→4828,
      frag 36648→37028) and both now carry a `PushConstant` storage class, checked by
      decoding the committed words rather than by trusting the script ran.
- [x] Convert the Vulkan emitter to walk `__canvas_sceneDraws`, pushing the offset and
      issuing one instanced `vkCmdDraw` per draw entry. — `c6dbc0524` (the conversion,
      plus the two halves of it that a stash had dropped) and `2c6809dcd` (the
      `strokeHalf` sign bug the conversion exposed). `emit_run_flush` is gone; every
      block is published first and `emit_draw_list_pass` issues the draws.
      **12/12 on box 2228, `worst=2 differing=0.8116%`** — the pre-conversion control's
      numbers exactly. Four corrections came out of this one box: **H12** (the ad-hoc
      loop was not the harness), **H13** (draw-entry width), **H14** (the blend-split
      instance rule), **H15** (the unsigned `strokeHalf` read).
- [x] Remove the `Group` decline from `__canvas_vulkanRenderable`; update its frame
      caps to sum over the resolved tree per §4.3. — `__CANVAS_DRAW_HAS_GROUP` is kept,
      because `__canvas_metalRenderable` still reads it until Phase 3.
      **§4.3 resolved by measurement, not by the plan's guess**: a group's blocks are
      recorded **once** and referenced by base, in *both* walks. A diamond referencing
      one leaf twice reports `entries=1 blocks=1` with two draw entries sharing base 0,
      so the caps already sum over the resolved tree and need no per-reference
      multiplier — the plan's second bullet was right.
      The item cap was wrong for a different reason and is fixed here: it counted `1`
      per non-text item, but a blended item that both strokes and fills publishes
      **two** records (**H15**), so it now asks `__canvas_blockInstances` — the same
      function the draw list asks.
- [x] Tests: on a Vulkan box, a group at `(0,0)` matches the oracle; a group at
      `(37, 53)` matches the oracle; a nested group matches; a diamond matches; a
      **clipped** item inside a translated group matches (the §4.2 clip case); and a **gradient-filled** item and a **`Text`** item
      inside a translated group match (the §4.2 gradient and glyph cases, **H2** —
      both fail today). — All seven, added to `scripts/test-canvas-vulkan.sh` as a
      second program rather than run by hand, so they are a gate and not an anecdote.
      **`worst=0 differing=0.0000%`** on box 2228 with `gpuFrames=1`.
      The emitted draw list reads the cases back: `2:1` at `(395,65)` is the nested
      group composing `(380,40)+(15,25)`; the two `3:1` entries at `(600,40)` and
      `(700,140)` are the diamond sharing one base; `6:2` is the two glyphs of the text
      run inside its group.
      The frame count is asserted **before** the pixels: a declined group scene compares
      the software renderer against itself and passes on pixels alone, which is exactly
      what plan-116-G's decline used to do.
      `tests/rt_canvas_golden.rs`'s decline test is now per-backend and renamed
      `a_group_reaches_only_the_backend_that_knows_the_per_draw_offset` — it asserts
      Vulkan **accepts** and Metal still declines, and Phase 3 flips the Metal arm.

Acceptance: all five scenes match the software oracle within
`Tolerance::GPU_DEFAULT` with `MFB_CANVAS_STATS` reporting `vulkanReady=TRUE`. The
non-zero-offset case is the one that proves the fragment-stage offset landed; a pass
there with a vertex-only implementation is not possible.

**MET, and above the bar: seven scenes rather than five, at `worst=0
differing=0.0000%` rather than merely inside `Tolerance::GPU_DEFAULT`,** with
`vulkanReady=TRUE gpuFrames=1`. `scripts/test-canvas-vulkan.sh` reports 14/14.
The whole-suite number is unchanged from before the conversion — `worst=2
differing=0.8116%` on the primitive scene — so the two-pass emitter costs nothing in
agreement with the oracle.
Commit: `3ded46db6`, `240fdf6ee`, `c29046626`, `c6dbc0524`, `2c6809dcd`, `e33b9e8f5`

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
      it visibly. **The group's item list must include a gradient-filled item, a
      `Text` item and a clipped item** (**H2**): those are the three positional reads
      that do not follow `p` for free, and a scene of plain shapes cannot see any of
      them go wrong.
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
- [ ] `scripts/regen-ncodesum.sh`. Expect **0 diffs, and do not read that as
      evidence** — no `canvas` fixture is hashed (plan-116-F **F11**).

Acceptance: `groups.png` matches on all three renderers; `cargo test --no-fail-fast`
green on **mac RELEASE, mac DEBUG (`--bin mfb`) and box 2228 RELEASE** (plan-116-E **E6**: CI is `--release` on all five platforms, so the `debug_assert!`s run nowhere in it and the debug row has to be run here); `scripts/test-accept.sh` green;
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

**H15 (Phase 2) — the residual 4.27% was a pre-existing emitter bug that only the draw
list could expose: the blend-split test read `strokeHalf` UNSIGNED.**

`emit_split_or_publish` splits an item into two published records when the blend mode is
not Normal **and** it strokes **and** it fills. The middle test was:

```rust
builder.emit(abi::load_u32(SCRATCH[0], sp, off_item + ITEM_OFFSET_MISC + 8));
builder.emit(abi::compare_immediate(SCRATCH[0], "0"));
builder.emit(abi::branch_le(&single));
```

`__canvas_strokeHalf` (`helper_items.rs`) reports "does not stroke" as **`-1.0`**, which is
`0xFFFF0000` in 16.16 — and `load_u32` **zero-extends**, so the 64-bit compare saw
4294901760 and took the split. **Every blended fill-only item was published as two records
instead of one.** The function's own comment already asserted the opposite ("so it is
fill-only and takes the single path"); the load simply did not implement it. Fixed with
`abi::sign_extend_word`, whose doc-comment cites bug-04 for this exact shape.

**Why nothing caught it before.** While draws came from `emit_run_flush`, the instance
count was `cursor - run_start` — *whatever had actually been published* — so the spurious
second record was drawn and painted nothing. The bug was invisible by construction. The
moment the draw list began predicting instance counts independently
(`__canvas_blockInstances`), one extra record shifted every later draw base by one and the
scene lost its tail. **That is the general hazard: replacing a self-consistent count with
an independently predicted one turns every latent publish/predict disagreement into a
visible defect.**

**How it was found — by bisection on the box, not by reading.** The reasoning that looked
right was wrong three times (the shaders, the ABI, the gradients), so the scene was shrunk
until it was two rectangles:

| scene | result |
|---|---|
| one gradient | 0 differing |
| 7 items, all Normal, one entry | 184 (edges only) |
| gradient **first**, blended second | 0 differing |
| blended **first**, gradient second | **9000 — exactly the rect** |

Then emitter probes on that two-item scene, each one harness run:

* per-entry pipeline bind disabled → still wrong (**not the bind**)
* per-entry push constants disabled → still wrong (**not the push**)
* `firstInstance=0, instanceCount=8` → **0 differing**: every item *is* published
* `firstInstance=3, instanceCount=1` → drew the **blue** rect, i.e. item **2**

That last probe is the one that named it: the item buffer was shifted by one, so something
published an extra record. Note the fill colour had to be changed from black first —
the gradient items' fallback fill is `rgb(0,0,0)`, which is also the background, so
"drawn with the fill" and "not drawn at all" were indistinguishable and I had read the
symptom as a gradient failure for three rounds.

**Gates.** `a_paint_that_does_not_stroke_reports_a_negative_stroke_half` in
`helper_items.rs` pins the sentinel as negative, because the tempting fix from the other
side — returning `0.0` — would hide a signed-read bug and leave the next consumer to
rediscover this. The sibling test at `ITEM_OFFSET_FILL + 12` was checked and is correct
unsigned: fill alpha is 0..255 and never negative.

**Metal (Phase 3) has no split path yet**, so it does not carry this bug — but it must not
reproduce it when it gains one.

**Result: `scripts/test-canvas-vulkan.sh` is 12/12, `worst=2 differing=0.8116%` — byte-for-
byte the pre-conversion control's numbers.**

**H14 (Phase 2) — the SECOND lost half, and a misread tuple that pointed H10 at the
harness instead of at the renderer.**

H13 found `helper_render.rs` short of its eight-word draw entry. The same file was also
missing the blend-split case of `__canvas_blockInstances`, which had decayed to:

```basic
FUNC __canvas_blockInstances(offset AS Integer) AS Integer
  IF toInt(__canvas_geoAt(offset, 0)) = __CANVAS_GEO_TEXT THEN
    RETURN toInt(__canvas_geoAt(offset, 20))
  END IF
  RETURN 1
END FUNC
```

`emit_split_or_publish` writes **two** records for an item that both strokes and fills
under a non-Normal blend mode (fill with the stroke off, then stroke with the fill made
transparent). Predicting 1 there shifts every draw-list base after the scene's
`blendStroke` item by one, so the final entry draws instances that were never published —
uninitialised item buffer, which reaches the screen as opaque black. **The symptom lands
at the end of the scene, nowhere near the item that actually disagreed**, which is what
made it read as "the gradients are broken".

Restoring the case moved the draw list from

    0:10|10:4|14:2|16:1|17:1|18:1|19:1|20:3|23:2|25:7      (32 instances, shifted)

to

    0:10|10:4|14:2|16:1|17:1|18:1|19:2|21:3|24:2|26:7      (33 instances)

which is exactly the list H10 recorded and mapped to the scene by hand, and the harness
from 6.93% differing back to H10's 4.27%.

**Both gates now exist**, in `helper_render.rs`'s test module, because neither half was
catchable by anything in `cargo test` before: `the_draw_entry_width_agrees_with_the_emitter`
counts the appends in `__canvas_pushOneDraw` against `CANVAS_DRAW_ENTRY_WORDS`, and
`block_instances_keeps_the_blend_split_case` pins the split by the three slots
`emit_split_or_publish` tests (26 blend mode, 7 strokeHalf, 11 fill alpha). The width is
now one constant in `src/codegen/runtime/canvas/mod.rs` that the emitter's byte stride,
element-count shift and mode offset all derive from, rather than three literals.

**H10's pixel claim is withdrawn.** It reported that "the GPU's disputed pixel is correct
— at (430,100) it produces `ff4321` — and it is the *oracle* the harness reports as
`000000`". That is the tuple read backwards. The harness builds it as

```python
first = (pixel % width, pixel // width, a.hex(), b.hex())   # a = software, b = gpu
```

so `first-beyond-tolerance=(430, 100, 'ff4321ff', '000000ff')` means **software `ff4321`,
GPU `000000`**: the GPU is the black one. H10 spent its closing paragraphs asking whether
the harness measured the right thing; it does, and the remaining 4.27% is a real defect in
the GPU render.

**H13 (Phase 2) — the Phase 2 failure was a draw-entry WIDTH disagreement, not an ABI
problem. H11 is withdrawn.**

The draw entry's width is agreed in three places and **no gate checks that they agree**:

| site | file |
|---|---|
| `__canvas_pushOneDraw` — how many words it appends | `src/codegen/builtins/canvas/helper_render.rs` |
| `__canvas_drawsText` — the stride it prints with | `src/codegen/builtins/canvas/helper_surface.rs` |
| `emit_draw_list_pass` — `shift_left_immediate(.., 6)`, mode at `+32` | `src/codegen/runtime/canvas/vulkan.rs` |

The working tree had the last two at eight words and **the first still at four**, so the
emitter strode 64 bytes through a 32-byte-per-entry array. Two consequences, and between
them they account for every symptom Phase 2 was chasing:

1. It visited **every other entry** and its loop bound (`len / 8`) was half the real count.
   So the draw list *looked* short — `blocks=28` against `draws=0:10|14:2|17:1|19:1|23:2` —
   which is the "the tail is missing / the last items render as their fill" reading.
2. It took the **following entry's `base` as the blend mode**, which indexes the pipeline
   table out of range and hands Vulkan a junk `VkPipeline`.

(2) is why it SIGSEGVs rather than drawing wrongly, and why the fault looks like it has
nothing to do with this code: `Thread 4 received SIGSEGV`, `rip` in JIT-compiled lavapipe
code at `mov 0x208(%rax),%eax`, **no MFBASIC frame in the backtrace at all**, `info symbol`
matching nothing and `info sharedlibrary` listing only `ld-linux`. A bad pipeline handle
surfaces exclusively as a crash inside the driver's generated code.

**How it got there:** the conversion was carried across contexts as stash entries, and the
apply restored the `vulkan.rs` and `helper_surface.rs` halves while the `helper_render.rs`
half was never in the entry. A `--stat` showed two files where three were required, and I
did not check that count against what the change needed.

**H11 is withdrawn in full.** Its claim — that `draws`, as the eighth argument, is clobbered
by staging that writes the C argument bank — is disproved by the emitted prologue, which
this correction finally read instead of reasoning about:

```
add_imm rbx, rbp, 40      ; draws data pointer, parked at rsp+120
ldr_u64 rbx, [rbp+8]      ; the collection count
lsr_imm rbx, rbx, 3       ; -> entries, parked at rsp+816
```

`draws` **is** in `rbp`, it **is** read correctly, and nothing between entry and that read
writes `rbp`. G31's "widening past eight arguments is safe" needed no second half. The
hoist H11 proposed is unnecessary; what made it look like it "moved the failure" was H12's
missing-display artifact.

**The reusable lesson: a constant shared between MFBASIC builtin source and the native
emitter that lowers it is a silent coupling** — the MFBASIC side is a string the Rust
compiler never type-checks, so a width change applied to one side and not the other builds
clean, passes every Rust test, and fails only as a driver-internal segfault on one remote
box. Phase 3 should express the entry width as one named constant with an assertion rather
than three independent literals.

**H12 (Phase 2) — the ad-hoc repro loop was not the harness, and three findings recorded
from it are measurement artifacts rather than defects.**

To iterate faster than the ~20-minute `scripts/test-canvas-vulkan.sh`, I built a hand-rolled
loop: build the scene, `scp` the AppDir to box 2228, run it with and without
`MFB_CANVAS_GPU=1`, diff the dumps. It was wrong in three ways, each of which produced a
confident reading:

| omitted | what the box did instead | what I recorded |
|---|---|---|
| `MFB_GTKAPP_HEADLESS=1` | `Gtk-WARNING: Failed to open display`, exit 1, no dump | "the GPU path produces no frame" |
| `MFB_CANVAS_SYNC=1` | dumped whichever frame happened to be current | "two *software* runs differ by 3.7%" |
| `MFB_CANVAS_STATS=<path>` (I wrote `=1`) | wrote no stats file at all | a mis-grep for `gpuFrames` |

The harness does all three (`scripts/test-canvas-vulkan.sh:294-297`), and the middle one is
the important one: **`MFB_CANVAS_DUMP` overwrites, so without `MFB_CANVAS_SYNC=1` the file
left behind is whichever frame the renderer last finished** — the script's own comment at
line 430 says exactly this. Two runs differing is then expected, not a defect, and
`rendering_is_byte_reproducible` was never in tension with it.

So **H10's closing paragraph is withdrawn**: "two software runs of that scene differ by 3.7%
with `gpuFrames=0` in both" is not evidence of a frame/damage defect and is not where the
next session should start. **H11's conclusion is also weakened**: the "hoist produces no
frame" observation that made the arg-8 fix look wrong was this same missing-display failure,
so the hoist was never shown to be incorrect — it was never actually exercised.

What survives unaffected is the part measured *through the harness*: the 21418-pixel
constant across five bisected variants, and the pixel dumps showing the trailing gradient
items rendering as their `fill`. Those came from harness runs, not from this loop.

**The lesson, which is the reusable part: a hand-rolled substitute for an existing test
harness must be diffed against that harness's invocation before anything is concluded from
it.** Three environment variables separated "the renderer is broken" from "the program never
opened a window". Re-measure through `scripts/test-canvas-vulkan.sh` and treat any ad-hoc
loop's disagreement with it as the loop being wrong.

**H11 (Phase 2) — the fourth bug in the conversion, and the one worth carrying forward:
`draws` is argument EIGHT, and reading it after other argument staging returns garbage.**

Bisected rather than guessed, which is what makes it worth recording. Five variants were
built and run on box 2228 against the software render of the same scene:

| variant | differing |
|---|---|
| fragment-stage offset neutralised | **21418** |
| both shader offsets neutralised (shaders behave exactly as HEAD) | **21418** |
| draw list emitted one entry per block, no run merging | **21418** |
| the software `offsets` list passed instead of the block list | **21418** |

**Identical every time**, which is the finding: the difference does not depend on the draw
list, the shaders, or the block list. And the pixels say what is missing — the three
gradient items render as `000000`, which is their `fill`, while everything before them is
correct to the byte:

```
(430,100) sw=ff4321 gpu=000000      (30,30) sw=ff0000 gpu=ff0000
(500,130) sw=fbd955 gpu=000000     (450,250) sw=ffff00 gpu=ffff00
```

The gradient items are **last** in that scene. A draw walk that stops early loses exactly
the tail — and the entry count is read from `draws`, the **eighth** parameter, which
MFBASIC's convention places in `rbp` (bug-296), *after* the staging below it has written
`SCRATCH` registers that alias the C argument bank on x86-64. That is
`.ai/arch-abi.md`'s "stage ABI args via temporaries" rule, and it bites here rather than
earlier because no previous argument to this function was the eighth. plan-116-G's **G31**
established that widening past eight is safe; it did not say the eighth argument must be
read before anything else is staged, and that is the missing half.

**The fix — hoisting the `draws` parking above all other staging — is written and
preserved as stash `59b7ae27a`, and is not yet correct**: with it the GPU path exits 0
without producing a frame, while the software path is unaffected. So the hoist moved the
failure rather than removing it, and the next step is to read the emitted prologue for
that function and confirm where `draws` actually lands before trusting any further
reasoning about it.

Stashes on `worktree-P-116`, newest first: `59b7ae27a` (conversion + the arg hoist),
`2c21706ae` (conversion as measured above). The branch itself stays at the last green
commit.

**H10 (Phase 2) — the emitter's two-pass conversion is written and preserved in a stash,
not on the branch, because it makes the Vulkan harness worse than HEAD and the remaining
failure is not yet understood.** What is on the branch (`3ded46db6`, `240fdf6ee`,
`c29046626`) is everything that stands on its own: the push-constant range with both
stages consuming it, both shader declarations, the pipeline split, and instance counting.
The emitter conversion is `git stash` entry `2c21706ae` on `worktree-P-116`.

**What the conversion contains, all of it verified as far as it goes:**

* `emit_draw_list_pass` — walks `__CANVAS_DRAWS`, binds each entry's pipeline, pushes its
  offset, issues one instanced `vkCmdDraw`. The emitted `.ncode` was read instruction by
  instruction and is correct: stride `lsl 6` (8 elements × 8 bytes), fields at 0/8/16/24/32
  loaded with `ldr_u64`, pipeline via the same shift-and-add the publish walk uses.
* `emit_run_flush` deleted (74 lines, one function, guarded by an assertion on the cut).
* `emit_glyph_draws` → `emit_glyph_publish`: its `vkCmdDraw` moved to the draw pass.

**Three real bugs were found and fixed inside it**, each worth keeping:

1. **The entry count was the element count.** Each entry is 8 elements, so the walk ran
   8× too long over garbage; a nonsense `instanceCount` *hung the GPU* rather than
   failing — box 2228 timed out with no output.
2. **Fields were read as 32-bit.** An MFBASIC `Integer` is 64-bit, so `base` and `count`
   were the low halves of the wrong words. 35.1% of the frame wrong, worst=255, with a
   draw list every assertion called correct.
3. **The glyph draw stayed in the publish walk**, so glyphs were drawn twice — the second
   time with the push constant undefined for that command.

**Where it stands: 10/12 on box 2228 against HEAD's 12/12**, `differing=4.27%`. The
evidence gathered rules out the obvious causes and does not yet name the real one:

* The draw list is **provably right**. Every entry maps to the scene by hand:
  `0:10` the ten leading shapes, `10:4` the four-glyph label, `19:2` the `blendStroke`
  blend split, `24:2` the two-glyph rotated text, `26:7` the tail. 33 instances from 28
  blocks, which is 28 + 4 text + 1 split.
* The GPU's disputed pixel is **correct**. At (430,100) it produces `ff4321`, and both the
  macOS *and* Linux software renders produce `ff4321` there — measured by shipping the
  scene to the box and dumping. It is the value the harness reports as the *oracle* that
  is `000000`.
* Two **software** runs of that scene on the box differ by 3.7% in rows 100–341, the
  gradient bands, with `gpuFrames=0` in both — so a difference appears between two runs
  that never touched the GPU at all.

**That last point was then tested and the hypothesis it suggested is wrong.** Running the
same program twice on box 2228 with no `MFB_CANVAS_GPU` produces **byte-identical** dumps
(`identical: True`, 2,304,000 bytes each). The software renderer is deterministic there,
exactly as `rendering_is_byte_reproducible` asserts, so there is no oracle defect to find.

Which means the 3.7% was between a software run and a run that had `MFB_CANVAS_GPU=1` —
and the `gpuFrames=0` read from that run's stats came from a `grep` over a *multi-line*
stats file, one line per frame, so it may well have been an earlier frame's value rather
than the drawing frame's. **The next step is to re-read that measurement per frame rather
than per file**, and to establish whether the disputed run drew on the GPU at all before
concluding anything from the comparison.

The narrowing that survives: the draw list is right, the emitted code is right, the GPU's
disputed pixel is right, and the oracle is deterministic. What has not been established is
what the harness's `sw.rgba` actually contains for that scene.

Stashed rather than committed because HEAD is green on that harness and the conversion is
not: leaving the branch red while the cause is unknown would make every later gate
ambiguous. The stash is not a deferral — Phase 2's boxes stay unticked, which is what
"not done" looks like in this ledger.

**H9 (Phase 2) — sharing a group's blocks forces the emitter into two passes, and §4.1
does not say so.** The current Vulkan emitter is one forward walk: for each item it
publishes a block and, at a run boundary, issues the draw for everything published since
the last one. That works because *publish order and draw order are the same sequence*.

The Phase 1 decision breaks that identity. With a shared group written **once**, a diamond
draws blocks `0..1` twice — so the draw sequence visits a range the publish walk has
already passed. No single forward pass can issue both draws, and the shape of the fix is
not a tweak to the run tracking; it is that **publishing and drawing become separate
passes**: walk the block list to fill the item buffer, then walk `__CANVAS_DRAWS` to issue
the calls.

That is a bigger change than "convert the emitter to walk `__canvas_sceneDraws`" sounds,
and the reason to write it down is the consequence for **text**. Today `emit_glyph_draws`
runs *inside* the item walk, where `off_item` holds the block just staged and `off_header`
the geometry record it came from. In a two-pass emitter the glyph draws move to the second
pass, which has neither — it has a block index. Both are recoverable (the header is the
geometry base plus `blocks[i] * 8`, and the block is already in the item buffer), but they
have to be *recovered* rather than inherited, and a text run drawn from a stale `off_item`
would render the previous item's glyphs: a plausible wrong picture again.

The ordering constraint that makes this non-optional: draws must be issued in scene order,
and text is interleaved with shapes. So the second pass must issue *both* kinds, in draw-
list order — text cannot be left in the first pass without putting every shape that
follows a text run on top of it.

**Phase 2's box list should read: publish pass, draw pass, glyph re-staging, then the
push-constant per entry.** The first two increments are landed (`3ded46db6`, `240fdf6ee`);
this correction is what the third has to build.

**H8 (Phase 2) — a draw entry is not "a run of non-group, non-text items"; it is a run of
everything that must not change within one `vkCmdDraw`, and §4.1 names only two of the
three things that force a split.** §4.1 says a run ends at a group node or a `Text` item.
Implementing the emitter shows a third: **`Paint.blend`**. The existing Vulkan emitter
flushes its run on a blend-mode change and binds that mode's pipeline
(`emit_run_flush` at `vulkan.rs:5360`, under `branch_ne(&same_mode)`), because each mode
is a separate `VkPipeline` and a pipeline is bound per draw, not per instance.

So a draw entry carries an implicit "and all of these share a pipeline" that §4.1 never
states, and a draw list built to §4.1's rule would issue one `vkCmdDraw` spanning items
with different blend modes — every one of them drawn with whichever pipeline happened to
be bound. That is a *plausible wrong picture*: a `Multiply` circle rendered `Normal` looks
like a colour mistake, not like a missing feature.

**The split belongs in the draw list, not in the emitter.** The list should already encode
everything that forces a separate draw call, which is what makes it a *draw* list rather
than a group list — and it is the only place both backends can share the decision. The
alternative, having each emitter sub-split entries it is handed, puts the same rule in two
assemblers and invites them to disagree, which is the `.ai/canvas-threading.md` §10 class
this letter is otherwise careful about.

Note the split can be computed at draw-list build time *or* read back from the blocks: a
block's kind is geometry slot 0 and its blend mode slot 26, so a walk over a memoised
block range can find the boundaries without re-visiting the scene. That matters for a
**group**, whose blocks are laid out once and referenced many times — the runs within it
are a property of the group, not of the reference, so they are computed once with the memo
and reused.

§4.1's example draws are unaffected: they use one blend mode throughout, which is why the
gap did not show in Phase 1's four cases. A fifth case — a group containing two items with
different blend modes — is added to that test, and it is the case that fails against the
§4.1 rule as written.

**H7 (pre-execution, 2026-09-04) — two §2 census rows had drifted; re-measured.**

* **`ITEM_BLOCK_SIZE` is 208, not 224.** The row cited plan-116-F §4.2 rather than the
  constant, and §4.2 is F's *design* text — F landed at 208.
  `grep -n 'ITEM_BLOCK_SIZE: usize' src/codegen/runtime/canvas/mod.rs` → `= 208`. This
  matters to H specifically: the letter adds a per-draw offset, and if it grows the block
  again then the Metal frame-slot constants shift by the same amount, which is the
  `METAL_EDGE_BASE` class of breakage F hit three times.
* **The shared scene walk is at `helper_render.rs:251`, not 122.** plan-116-G inserted
  `__canvas_appendDraw`, the `__CANVAS_DRAW_*` globals and `__canvas_groupSignature`
  above `__canvas_sceneOffsets`. Now cited by symbol with its grep, per the project's own
  rule that a `file.rs:NNN` into a file the plan series edits is stale before it is read.

The other four rows re-measured correct: 2 backends, 3 shader files, 2 `*Renderable`
predicates (`grep -c 'FUNC __canvas_.*Renderable'` → 2).

**One thing H should know that its §4.1 predates:** plan-116-G already flattens the group
tree where the draw list is built, and publishes each entry's accumulated offset in
`__CANVAS_DRAW_DX`/`__CANVAS_DRAW_DY` beside the offsets list. So `__canvas_sceneDraws`
does not need to re-walk the scene — a *run* is a maximal span of consecutive entries
sharing a `(dx, dy)` and not ending on a `Text`, which is a grouping pass over data that
already exists rather than a second traversal that could disagree with the first.

**H7 (2026-09-03, pre-execution) — the `rbp` invariant this letter must hold, stated the
way that is checkable.** MFBASIC stages parameter 8 in `rbp`, which is callee-saved under
SysV, and the callee-saved set is computed from *allocated* registers so the staging is
invisible to it (bug-296). The consequence is **not** an arity limit —
`__canvas_geoDistance` takes 22 parameters and `__canvas_drawGeometry` calls it six times
per item — it is that **every point where foreign code calls into MFB code must save
`rbp`**: the thread trampolines and the `_mfb_gtkapp_*` callbacks.

For H the check is: does this letter add an **entry point reached from GTK or pthread**?
Adding a per-draw offset, widening a helper, or introducing a new MFB→MFB function is
safe at any arity. A new callback or trampoline is not, at any arity.

Worth carrying because H is the letter most likely to widen `__canvas_drawGeometry` again
and to conclude something from the parameter count. plan-116-G's **G31** did exactly
that, was wrong, and is corrected in place with the wrong reading left visible.

**H6 (2026-09-03, pre-execution) — `GEO_KIND_GROUP` is `#[cfg(test)]` and this is the
letter that takes it off.** plan-116-G added the Rust-side kind constant so the
MFBASIC/Rust pair could be pinned by
`the_geo_layout_constants_match_their_rust_counterparts`, and marked it `#[cfg(test)]`
because that guard was genuinely its only reader: nothing writes kind 8 into a header,
since a group node carries `__canvas_emptyHeader()` whose kind is `NONE` (plan-116-G
**G16**).

That was chosen to be self-correcting rather than to need remembering — an emitter that
starts dispatching on kind 8 will not compile until the attribute comes off — but the
compile error appears in a file H is editing anyway, so it is worth knowing what it is
rather than diagnosing it. `grep -n "GEO_KIND_GROUP" src/codegen/runtime/canvas/mod.rs`.

Note also that H may not need it at all. G expands every group away before an emitter
sees the draw list, so no *item block* ever carries kind 8; if H's per-draw offset
arrives as a push constant keyed off the draw entry rather than off the item, the
emitters never test the kind and the constant stays test-only. Which of those is true is
decided by Phase 1's draw-list shape, not by this correction.

**H5 (2026-09-03, pre-execution, written from plan-116-G as landed) — §4.1's
`__canvas_sceneDraws` describes a walk that plan-116-G already performs, and §4.3's
"undecided question" is already decided in the direction §4.1 calls unlikely.**

This section was written against a world where `__canvas_sceneOffsets` returns one entry
per *scene* item with `Group` nodes still in the list, so H would add a second walk to
flatten them. That is not what G built. `__canvas_appendDraw` (`helper_render.rs`)
expands every group where the draw list is assembled, so the list H inherits is:

* **already flat** — leaf items only, no `Group` to end a run;
* **already carrying the offset** — `__CANVAS_DRAW_DX` / `__CANVAS_DRAW_DY`, parallel to
  the offsets list, one entry each;
* **already telling a caller whether groups were involved** —
  `__CANVAS_DRAW_HAS_GROUP`, which is what both `*Renderable` predicates read to decline
  today (and which H removes).

So Phase 1's real work is not "perform the same depth-first walk"; it is **group the
existing flat list into runs of equal `(dx, dy)`**, which is a scan rather than a
traversal, and emit `(itemBase, itemCount, dx, dy)` per run. Re-measure before writing
it: `grep -n "FUNC __canvas_appendDraw" -A 30 src/codegen/builtins/canvas/helper_render.rs`.

**And §4.3's question is settled by that same code, the other way.** G emits one entry
per *reference*, so `[rect, Group(A)@(10,20), circle]` with `A = [c1, c2]` gives four
offsets entries — `rect, c1, c2, circle` — which is §4.1's **first** bullet, the one it
calls less likely. A diamond therefore produces two entries per child, not one shared
run.

That is not the loss it looks like, and the distinction H should keep hold of is
*which* buffer is being shared:

* The **geometry cache** already shares, completely. Measured directly rather than
  inferred: a scene of two groups each naming a two-item group — four drawn instances of
  one rectangle — reports `generations=1 entries=1 floats=47`. One 47-slot record, one
  cache entry, four draws. Both references resolve to the same `__canvas_geometryFor`
  offset, which is also why plan-116-G's
  `a_group_renders_at_its_offset_nested_diamond_and_absent` measures `entries=1` for one
  shape drawn at three offsets. That is the sharing the feature's speed goal named, and
  it is already won.

  Note what that same measurement says about the draw list: the four instances *are*
  drawn, and they carry different offsets, so the offsets list holds four entries all
  pointing at the one cache offset. Repetition in the draw list and sharing in the cache
  are not in tension — they are the two different buffers this correction is about.
* The **GPU item block** does not share, because a block is per draw instance and two
  references differ in exactly the field this letter adds — the offset. Two references
  cannot share one block *unless* the offset moves out of the block and into a per-draw
  push constant, which is precisely what §4.2 does.

So the answer to §4.3 is available without re-deciding it: once the offset is a push
constant rather than a block field, two references to a group have byte-identical blocks
and *may* share them — the entries in the offsets list already point at one geometry
entry, so the emitter can key its item-block cache on that offset. Whether it is worth
doing is a measurement Phase 1 should take rather than a design question, and the honest
default is **no**: writing a block per reference is a memcpy into a mapped buffer, while
sharing needs a per-frame map from geometry offset to block index, and
`CANVAS_MAX_FRAME_ITEMS` is 4096 either way.

**H5 (2026-09-03, pre-execution) — `setVertexBytes:` is not available to bind the Metal
offset; plan-116-A deleted it.** §4.2 names `setVertexBytes:` and `setFragmentBytes:` as
though both were on hand. `grep -n 'setVertexBytes' src/target/macos_aarch64/app/metal.rs`
finds it only inside a doc comment explaining its removal: plan-116-A replaced
`drawPrimitives:vertexStart:vertexCount:` and `setVertexBytes:length:atIndex:` with the
instanced draw and *deleted* both selectors, on the stated grounds that an unsent
selector still costs a C string and a runtime registration in every canvas binary.
`setFragmentBytes:` is still there, carrying the glyph bitmaps.

So Phase 3 re-adds one selector to `metal_data_objects` alongside the shader change. The
failure if it is missed is not a compile error — the emitter would send to a selector
that was never registered.

Two things next to it that are *not* problems, checked while here: the instanced draw
already carries `baseInstance:`, which is exactly the "name your own blocks" mechanism a
per-group draw needs, and MSL's `[[instance_id]]` already includes it (plan-116-A
Correction C5 measured that, having predicted the opposite). And the comment's rejected
alternative — binding the buffer at `base * ITEM_BLOCK_SIZE` — is still rejected: it
cites a "112-byte stride", which is stale (plan-116-A/C/D/E/F have grown
`ITEM_BLOCK_SIZE` to 208), but 208 no more meets the `MTLBuffer` offset alignment than
112 did, so the conclusion survives its own arithmetic. The stale number is being
corrected by plan-116-F's close-out.

**H4 (2026-09-03, pre-execution) — the Vulkan push-constant range was deleted, not
vacated.** §4.2 says *"the item block left the push-constant range in plan-116-A, so the
range is free"*, which reads as "a declared range is sitting there unused". It is not:
`emit_struct` zeroes `VkPipelineLayoutCreateInfo` and the emitter leaves `rangeCount` at
0 with `pPushConstantRanges` null, and both the struct comment (`:501-503`) and the
pipeline-layout comment (`:1799-1804`) say the absence is the point — *"a layout that
declares bytes no stage consumes is a layout the validation layers flag."*

The work is therefore re-adding a range, not filling one, and the range and the shader
declarations have to land together: a range with no consumer is flagged, and a shader
declaration with no range is a layout mismatch. Both stages must consume it, which
§4.2's own arithmetic already has them doing.

Worth writing down because the failure is invisible on a box without validation layers —
`scripts/test-canvas-vulkan.sh` runs against lavapipe on 2227 and whatever ICD 2228 has,
and a layout error that the driver tolerates is exactly the kind that reaches a user's
machine and not ours.

*Re-verified against `main` after plan-116-G's merge (38 commits, including two canvas
changes), because a line citation into a file a plan series edits is stale before it is
read:* `grep -n "pPushConstantRanges" src/codegen/runtime/canvas/vulkan.rs` → **501**,
still the "deliberately absent … with `rangeCount` 0 the pointer must be null" comment;
`grep -n "declares bytes no stage consumes" …` → **1802**, inside the range the
correction cites. Both claims hold and both citations still land.

**H3 (2026-09-03, pre-execution) — §4.1's worked example does not add up under either
answer to §4.3's open question.** It gives `[rect, Group(A) @ (10,20), circle]` with A =
`[c1, c2]` as `(0,1,0,0)`, `(A,2,10,20)`, `(2,1,0,0)`.

Work the third entry both ways. Inline-per-reference: blocks are `rect=0, c1=1, c2=2,
circle=3`, so `circle` is base **3**. Uploaded-once: A's blocks are not in the scene's
sequence at all, the scene holds `rect=0, circle=1`, so `circle` is base **1**. The
example says 2, which is the base `circle` would have if A contributed exactly one block
— and A has two, which the same line says.

Small, and it matters because Phase 1's acceptance is *"asserts `__canvas_sceneDraws`
produces the expected `(base, count, dx, dy)` sequence"*. The natural way to write that
test is to copy the numbers from the design section, and those numbers encode an
expectation no correct implementation can satisfy — so the test fails, gets "fixed" by
reading them off the implementation, and stops being an independent check of anything.

Rewritten to give both sequences explicitly and to say which question picks between
them.

**The recommendation is withdrawn — see H5.** §4.3 reasoned that uploaded-once "is what
makes a diamond share a buffer, which is the property this letter exists for". Measured
after plan-116-G, that sentence conflates two buffers: the **geometry cache** already
shares a diamond completely (`entries=1` for four drawn instances of one rectangle), and
that is the property the feature's speed goal named. What repeats per reference is the
**draw list**, and a GPU **item block** cannot be shared between two references while the
offset lives in the block — which is precisely what §4.2 moves out of it.

So Phase 1 inherits G's inline-per-reference list and the first sequence above is the
live one: `(0,1,0,0)`, `(1,2,10,20)`, `(3,1,0,0)`. Whether to *additionally* share item
blocks once the offset is a push constant is a measurement Phase 1 may take, not a design
question it must answer first — and H5 gives the honest default.

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

**Update (post-plan-116-G, measured):** G settled §4.5's open decision — the gradient
*follows* the group (G23) — and implemented it in the oracle only. So the three renderers
now **disagree**, which is a sharper statement of this correction than it originally
carried:

* The **oracle** is fixed. `helper_items.rs` still reads `gradFX`/`gradFY` from the
  record and still evaluates at the loop's `px`/`py` (`:544-545`), but G redefined
  `px`/`py` as the **shape-space** point (`px = spx - gdx`), so the ramp translates with
  the shape for free and no gradient code changed.
* **Both shaders still evaluate at the surface point** — `gradientColour(gl_FragCoord.xy)`
  at `mfb_canvas.frag:442`, `gradientColour(in.pos.xy, …)` at `metal.rs:470`.

That is not a live bug today: both `*Renderable` predicates decline any scene that
contained a group, so a gradient inside one is never drawn on a GPU. It becomes one the
moment this letter removes that decline. **So the gradient is not a separate task here —
it falls out of §4.2 done correctly.** Once the per-draw offset reaches the fragment
stage, the shaders must subtract it before computing `t`, exactly as `px` does; the same
subtraction that fixes the distance field fixes the ramp, provided the gradient is
written in terms of the offset-corrected point rather than `gl_FragCoord`/`in.pos`
directly.

The clip is the one that must keep the **raw** surface point in both shaders, for the
reason §4.2 already gives — and plan-116-G's
`a_clip_inside_a_translated_group_stays_on_the_surface` and
`a_gradient_inside_a_group_moves_with_the_group` are the two oracle tests that pin the
pair. H's GPU tests should assert against those same two scenes, since agreement with the
oracle is the whole acceptance.

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

**They did, and by more than a line shift** (re-measured after plan-116-G landed):

| symbol | now at |
|---|---|
| `__canvas_appendDraw` | `:163` — **new in G**, the group-expanding walk |
| `__canvas_sceneOffsets` | `:251` (was `:122`) |
| `__canvas_metalRenderable` | `:327` (was `:198`, given as `:180`) |
| `__canvas_vulkanRenderable` | `:424` |

`grep -n "FUNC __canvas_metalRenderable\|FUNC __canvas_vulkanRenderable\|FUNC
__canvas_sceneOffsets\|FUNC __canvas_appendDraw"
src/codegen/builtins/canvas/helper_render.rs` finds all four.

The substantive change for H is not the offsets: it is that the decline the two
predicates now carry is **not a `Group` check**. G expands every group away before a
predicate sees the list, so there is no `Group` item left to find — both read
`__CANVAS_DRAW_HAS_GROUP`, a flag the walk sets. Phase 2 and Phase 3 remove *that*, and a
phase that went looking for a `CASE Group` arm to delete would find nothing and leave the
decline in place.

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
