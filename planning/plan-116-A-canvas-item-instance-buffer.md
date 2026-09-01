# plan-116-A: The item block moves from push constants into a per-frame instance buffer

Last updated: 2026-08-31
Overall Effort: huge (>3d)
Effort: x-large (1d–3d) — revised 2026-09-01; see Corrections C1
Depends on: nothing

Both GPU backends today send one item's parameters *per draw* — Metal through
`setVertexBytes:`/`setFragmentBytes:`, Vulkan through `vkCmdPushConstants` — and the
block is sized to Vulkan's guaranteed 128-byte push-constant range with 112 bytes
used. That ceiling is what makes every other letter of plan-116 impossible: the
fields still to be added (transform 6 words, clip 4, gradient base 1, cap 1, blend 1,
ellipse angle 1) total 14 extra words = 56 bytes, which puts the block at 168 bytes,
past the guarantee.

This letter moves the item block into a **per-frame buffer of `ITEM_BLOCK_SIZE`-byte
records indexed by instance id**, and turns each item's draw into an instanced draw.
**On Metal it also moves the polygon edge payload into the same frame buffer**,
because an instanced run cannot rebind a per-item `setFragmentBytes:` payload
between instances — without this, every polygon would end the run and letters F
(per-item gradient stops) and H (one instanced draw per group) would each rediscover
the same conflict. It adds no feature and, with one named exception (§Compatibility),
changes no pixel.

Behavioral outcome: every existing canvas scene renders **byte-identically** on the
software, Metal and Vulkan paths before and after this letter, while the per-item
parameters travel in a buffer rather than in push constants — so a later letter can
widen the block without a push-constant limit to negotiate.

## The plan-116 letters, in implementation order

Letter order is implementation order: every phase of a letter lands before any phase of
the next. Each is a complete plan in its own right.

| Letter | What it lands | Effort |
|---|---|---|
| **A** (this) | The item block moves into a per-frame instance buffer, both backends, and Metal's polygon edges move into the same buffer. No feature; no pixel change except one named decline (§Compatibility). **The enabler**: the block is at 112 of Vulkan's guaranteed 128 push-constant bytes, B–F need 56 more, and instanced draws cannot coexist with per-item side payloads. | x-large |
| **B** | `Paint.blend` and `Paint.clip` become real — four blend modes as four pipelines, a clip that antialiases its own edge. | large |
| **C** | `Paint.transform` becomes real — the SDF is evaluated at the inverse-transformed query point. | large |
| **D** | `cap AS CapStyle` on `Line` and `Arc` (`Butt`/`Round`). | medium |
| **E** | `canvas::Ellipse`, the ninth `DrawItem` variant, with a fixed-iteration exact SDF. | large |
| **F** | Gradient fills — `GradientKind`/`GradientStop`/`Gradient` and `Paint.fillGradient`. | large |
| **G** | Named groups: `setGroup`/`removeGroup`/`canvas::Group`, storage, lifetime, resolution, software rendering. | large |
| **H** | Groups on the GPU — one instanced draw per group node, per-draw offset bound to both stages. | large |
| **I** | `canvas::Picture` and `canvas::Text` hold `RES canvas::Image` / `RES canvas::Font` directly; the `ImageRef`/`FontRef` records and `canvas::imageRef`/`fontRef` are **removed**. plan-114 A–E landed 2026-09-01 (`ls planning/completed/plan-114-*` → 5), so `RES` record fields are legal source. | large |
| **J** | `setGroup` takes ownership of the resources in its list. **Hard prerequisite: plan-116-I complete** — before I, nothing ownable can be in a `DrawItem`. | medium |

B and C together close the defect that `Paint.transform`, `Paint.clip` and
`Paint.blend` are documented but never read
(`grep -rn "\.transform\|\.clip\|\.blend" src/codegen/builtins/canvas/` → 3 hits, all
doc strings).

References:

- `.ai/canvas-threading.md` §10 — the renderer branch, the two `*Renderable`
  predicates, and why they differ between backends.
- `.ai/testing-gates.md` — the canvas reference-image gate and `Tolerance::GPU_DEFAULT`.
- `.ai/arch-abi.md` — macOS AArch64 and the Metal object-send path.
- `src/codegen/runtime/canvas/mod.rs:246-282` — the item block layout, spelled once
  for both backends.
- `scripts/regen-spirv.sh` — the only way the Vulkan GLSL becomes SPIR-V.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| A Linux box with `glslang-tools` reachable for SPIR-V regen | `scripts/regen-spirv.sh` (ships GLSL to box 2228, compiles against a dpkg-extracted glslang — its own header documents this) | MET (2026-09-01: box 2228 reachable; the script needs no preinstalled glslang) |
| The Metal box (macOS host) can run `tests/rt_canvas_metal.rs` | `cargo test --test rt_canvas_metal -- --no-fail-fast` | MET (2026-09-01: in the default local suite on this host; re-run at Phase 2 start) |
| A Vulkan-capable Linux box (ICD present, not just loader) | `ssh -p 2228 test@127.0.0.1 'ls /usr/share/vulkan/icd.d/'`; then `scripts/test-canvas-vulkan.sh` end to end | MET (2026-09-01: 7 ICDs on 2228 including `lvp_icd.json` — lavapipe, a software ICD, so `vulkanReady` does not depend on GPU hardware); run the script before relying on it |

Everything below is written against the world where these hold. The SPIR-V one is
hard: the `.spv` blobs are checked in and there is no build-time shader compiler
(`scripts/regen-spirv.sh` header), so a GLSL edit that cannot be compiled cannot land.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again
> before you decide to stop. If you stop, report the status of *all* prerequisites.

## 1. Goal

- The per-item parameter block for both GPU backends lives in a buffer written once
  per frame, one `ITEM_BLOCK_SIZE` record per drawn item, and the shaders index it by
  instance id (`[[instance_id]]` / `gl_InstanceIndex`) rather than reading a push
  constant.
- `ITEM_BLOCK_SIZE` is no longer bounded by 128 bytes; the constant's doc comment
  says so and names the buffer as the new bound.
- Every existing canvas scene renders byte-identically on all three paths.

### Non-goals (explicit constraints)

- **No new `canvas::` surface.** No new record, enum, union variant, function or
  `Paint` field lands in this letter. `mfb man canvas --all` output is unchanged.
- **No pixel change, on any backend.** This is the provably-neutral class; see §3.
- **No change to the software rasteriser.** `__canvas_drawGeometry` and the geometry
  cache are untouched — they never had a push-constant limit.
- **No change to the 22-float geometry header** (`HEADER_SLOTS`). The header is the
  CPU-side cache format; only its GPU-side *transport* changes.
- **The two `*Renderable` predicates keep declining exactly what they decline today.**
  Widening what the GPU accepts is a later letter's job, and doing it here would hide
  a regression inside a change whose whole gate is "nothing moved".
- **The per-item `MAX_EDGES` polygon cap and both glyph caps are unchanged.** The
  Metal predicate keeps declining a >`MAX_EDGES` polygon even though the frame
  buffer no longer forces the cap — decline parity with today is this letter's
  gate; unifying the caps is later work, taken deliberately or not at all.
- **Glyph runs keep their per-draw `setFragmentBytes:` coverage payload.** A text
  item is already N separate draws and stays so (§4.3), so its per-glyph payload
  never has to coexist with instancing and does NOT move in this letter. Only the
  polygon edge payload moves, because polygons are ordinary one-draw items inside
  instanced runs.

## 2. Current State

### The block and its two emitters

`src/codegen/runtime/canvas/mod.rs:246` declares `ITEM_BLOCK_SIZE = 112` with the
layout spelled once for both backends: six `ivec4`s at 0/16/32/48/64/80 and the
surface size at 96. The doc comment states the reason for 112 outright — *"112 bytes
fits Vulkan's guaranteed 128-byte push-constant range, which is why neither backend
needs descriptor sets or uniform buffers."*

- **Metal** (`src/target/macos_aarch64/app/metal.rs`) sends it with
  `setVertexBytes:`/`setFragmentBytes:` and draws with
  `drawPrimitives:vertexStart:vertexCount:` (`metal.rs:314`), one draw per item.
  Classified by emission site
  (`grep -n 'SEL_SET_VERTEX_BYTES\|SEL_SET_FRAGMENT_BYTES\|SEL_DRAW_PRIMITIVES'
  src/target/macos_aarch64/app/metal.rs`): the **item block** is sent twice, each
  to both stages — the shape path's setter loop at `:996` and the glyph path's at
  `:1539`; the **polygon edge payload** is a separate per-item
  `setFragmentBytes:` at `:1017` ("always bound even when empty"); the **glyph
  coverage bitmap** is a per-glyph `setFragmentBytes:` at `:1560`/`:1571`; the
  draws are `:1043` (shape) and `:1596` (glyph). The distinction matters: the
  item-block sends become buffer writes, the edge send becomes a buffer region,
  and the glyph sends stay (non-goals).
- **Vulkan** (`src/codegen/runtime/canvas/vulkan.rs`) sends it with
  `vkCmdPushConstants` and draws with `vkCmdDraw` (`vulkan.rs:144-145`, resolved once
  and kept at `:4284`), one draw per item.

Both shaders declare the block and read it directly: `mfb_canvas.vert:14`
(`layout(push_constant) uniform Item`), `mfb_canvas.frag:15`, and the MSL
`struct MfbItem` at `metal.rs:107` with `constant MfbItem &item [[buffer(0)]]` in both
stages (`metal.rs:118`, `metal.rs:197`).

### The precedent this mirrors

Vulkan **already has** a descriptor-bound storage buffer and already indexes it
per-item by an offset carried in the item block: `GRAPHICS_OFFSET_VULKAN_EDGE_BUFFER`
(`runtime/canvas/mod.rs:216`), read through `item.arc.z` in
`mfb_canvas.frag:edgeDistance`. The design here is that same shape applied to the
block itself, so it is a generalisation of a mechanism already proven on this path,
not a new one.

### Measured populations

| What | Count | Command |
|---|---|---|
| Metal byte-push emission sites | 2 item-block setter loops (`:996`, `:1539`), 1 edge send (`:1017`), 2 glyph sends (`:1560`, `:1571`) | `grep -n 'SEL_SET_VERTEX_BYTES\|SEL_SET_FRAGMENT_BYTES' src/target/macos_aarch64/app/metal.rs` |
| Vulkan push/draw fn slots resolved once | 2 | `grep -n 'vkCmdPushConstants\|vkCmdDraw' src/codegen/runtime/canvas/vulkan.rs` → `:4284-4285` |
| `metal.rs` LOC | 2073 | `wc -l src/target/macos_aarch64/app/metal.rs` |
| `vulkan.rs` LOC | 5073 | `wc -l src/codegen/runtime/canvas/vulkan.rs` |
| Canvas test files / LOC | 12 / 4391 | `wc -l tests/*canvas*.rs` |
| Canvas reference-image goldens | 1 (`smiley.png`) | `ls tests/golden/canvas/` |
| Checked-in SPIR-V blobs | 2 | `ls src/codegen/runtime/canvas/shaders/*.spv` |

### Verified properties

- **The block is byte-identical between backends today.** Read both declarations:
  `metal.rs:107-116` (`struct MfbItem`, six `int4` + `int2 surface`) and
  `mfb_canvas.vert:14-23` / `mfb_canvas.frag:15-23` (seven `ivec4`). The Vulkan side
  declares the surface as a full `ivec4` rather than an `ivec2` *specifically* so
  trailing padding cannot differ — stated at `mfb_canvas.vert:17`. There is a unit
  test pinning the size against the guaranteed range (`vulkan.rs:5018`).
- **Three words of the block are already unused**: `arc.w` (offset 92) and
  `surface.z`/`surface.w` (104, 108). Read from the layout comments at
  `runtime/canvas/mod.rs:268` ("then one unused word") and `mfb_canvas.vert:22`
  ("width, height, unused, unused"). This is why the widening in later letters starts
  from 3 free words, not 0.
- **Vulkan's descriptor set already exists and is already indexed per-item.** Read
  `mfb_canvas.frag:44` (`readonly buffer Edges`) and `edgeDistance(item.arc.z, …)`.
  So on the Vulkan side this letter adds a *second binding*, not the first.
- **UNVERIFIED: whether an instanced draw reproduces the current output bit-for-bit.**
  The shading maths is unchanged and the vertex stage already synthesises corners from
  `gl_VertexIndex`/`[[vertex_id]]` with no vertex buffer, so there is no interpolation
  or rasterisation-rule change in prospect — but this is the plan's central premise and
  Phase 1 exists to test it before the rest is built on it.

## 3. Design Overview

Four pieces, layered:

1. **A per-frame item buffer** in the graphics state, host-visible and persistently
   mapped, holding N `ITEM_BLOCK_SIZE` records. Sized once at device/pipeline creation
   like the edge buffer, not per frame.
2. **The shaders read `items[instanceOrDrawIndex]`** instead of a push constant. On
   Metal the block becomes `constant MfbItem *items [[buffer(0)]]` plus
   `uint iid [[instance_id]]`; on Vulkan a second `readonly buffer` binding plus
   `gl_InstanceIndex`.
3. **Metal's polygon edges move into an edge region of the same frame buffer**,
   with each polygon's first-edge index carried in `ITEM_ARC_EDGE_BASE` — the word
   that already exists for exactly this and that Metal currently leaves zero
   (`runtime/canvas/mod.rs`, `ITEM_OFFSET_ARC` doc: "Metal leaves the edge base
   zero — its `setFragmentBytes:` copies each item's edges into the command
   buffer"). After this letter the two backends carry edges identically.
4. **The emitters write the buffer, then draw.** Each backend fills its slice of the
   buffer at record time and issues one instanced draw per run of consecutive
   non-text items (§4.3).

**Where the correctness risk concentrates:** the shaders. A mis-indexed instance is a
scene drawn with another item's parameters — plausible-looking output, not a crash.
The reference-image gate is what catches it, which is why Phase 1 proves the mechanism
on the simplest possible scene before Phases 2–3 convert the real paths.

**Where the design uncertainty concentrates:** whether an instanced draw is
bit-identical to N separate draws. Phase 1 is the cheapest experiment that settles it.

**Byte-identity IS this letter's acceptance gate, and legitimately so.** This is the
provably-neutral class: same geometry, same distance fields, same coverage
quantization, same blend state — only the transport of the parameters changes. So the
gate is "the Metal and Vulkan frames match the software oracle exactly as well as they
did before, and the software oracle itself is unchanged byte-for-byte".

**If a byte-identity check fails, that is a bug in this letter's plumbing — most
likely an instance-index or buffer-stride mistake — to be root-caused by dumping one
frame (`MFB_CANVAS_DUMP`) and diffing it against the oracle. It is never evidence the
design is unworkable, and it is never a reason to stop.** The one class of diff that
would be *expected* is none: no target should diff.

### Rejected alternatives

- **Raise the push-constant block past 128 bytes and query
  `maxPushConstantsSize` at run time.** Rejected: 128 is the only guaranteed value,
  so this makes the feature set conditional on the device and gives two code paths
  that must agree pixel-for-pixel. The buffer has no such ceiling.
- **Keep push constants and put only the *new* fields in a buffer.** Rejected: it
  splits one logical item block across two transports with different lifetimes, and
  every later letter would have to decide, per field, which half it lands in.
- **Keep Metal's per-item edge payload and end the instanced run at every
  polygon.** Rejected: it works for this letter alone, but plan-116-F's gradient
  stops and plan-116-H's one-draw-per-group each add another per-item payload, so
  each would re-split the runs and H's outcome would be false for any group
  containing a polygon. The frame buffer is the same mechanism this letter already
  builds for the item blocks — the edge region is one more range in it, and it is
  the shape Vulkan has already proven on this exact payload.
- **One uniform buffer per item rather than one buffer for the frame.** Rejected for
  the reason already recorded for Vulkan's edge buffer at `runtime/canvas/mod.rs:216`:
  a command buffer is recorded once and executed once, so per-item rebinding gives
  every item the last one's data.

## 4. Detailed Design

### 4.1 The buffer

One allocation per backend, created with the device (not the render target — its size
does not depend on the surface), host-visible, persistently mapped, mirroring
`GRAPHICS_OFFSET_VULKAN_EDGE_BUFFER`'s lifecycle exactly.

- New graphics-state slots for the Vulkan side, appended after
  `GRAPHICS_OFFSET_RESIZES_SEEN` (currently 648, `GRAPHICS_STATE_SIZE` 656):
  `…_VULKAN_ITEM_BUFFER`, `…_VULKAN_ITEM_MEMORY`, `…_VULKAN_ITEM_MAPPED`. Metal gets
  `GRAPHICS_OFFSET_MTL_ITEM_BUFFER`. `GRAPHICS_STATE_SIZE` grows accordingly.
- Capacity: `CANVAS_MAX_FRAME_ITEMS`, a new constant beside `VULKAN_MAX_FRAME_EDGES`.
  Set it to **4096** items = 448 KiB at the current block size. Both `*Renderable`
  predicates gain a frame-item-count check against it, declining to software past it —
  the same honesty-gate shape `VULKAN_MAX_FRAME_EDGES` already has, and for the same
  reason: a truncated scene is a *different scene*.
- **Metal's buffer carries two regions**: the item blocks at offset 0, then an edge
  region of `METAL_MAX_FRAME_EDGES` (**16384**, mirroring `VULKAN_MAX_FRAME_EDGES`)
  × 16 bytes. The Metal shape emitter writes each polygon's edges there and stores
  the first-edge index in `ITEM_ARC_EDGE_BASE`, exactly as the Vulkan emitter
  already does. `__canvas_metalRenderable` gains the same frame-total edge sum
  `__canvas_vulkanRenderable` has (`helper_render.rs:232-248`) — see
  §Compatibility for the one scene class this newly declines.

### 4.2 The shader change

Metal (`METAL_SHADER_SOURCE`, `metal.rs:102`):

```
vertex VOut mfbVertex(uint vid [[vertex_id]], uint iid [[instance_id]],
                      constant MfbItem *items [[buffer(0)]]) {
  constant MfbItem &item = items[iid];
  …unchanged…
}
fragment float4 mfbFragment(VOut in [[stage_in]], …)
```

The fragment stage needs the same index, so `VOut` gains a flat-interpolated
`uint iid [[flat]]` passed from the vertex stage. That is the standard way to get an
instance id into a fragment shader in MSL and it costs one varying.

Vulkan (`mfb_canvas.vert` / `.frag`): the push-constant block becomes

```
layout(std430, set = 0, binding = 1) readonly buffer Items { ItemBlock items[]; } itemBuf;
```

with the vertex stage reading `itemBuf.items[gl_InstanceIndex]` and passing the index
to the fragment stage as `layout(location = 1) flat out int vItem;`.

**`std430` layout of an `ivec4[7]` struct is exactly the 112 bytes the block already
is** — `ivec4` has 16-byte alignment and size in std430, so the array stride is 112
with no padding. Phase 3 must confirm this against glslang's reflection rather than
assume it; if glslang reports a different stride, the fix is to pad the struct to the
reported stride and record the number in `ITEM_BLOCK_SIZE`'s doc comment.

### 4.3 The draw

- Metal: one `drawPrimitives:instanceCount:` per *run of consecutive non-text
  items* — a run ends only at a `Text` item (which is its own N glyph draws, below)
  or at the end of the scene; there is still exactly one pipeline. Polygons ride
  inside runs, their edges reached through the edge region. The selector string at
  `metal.rs:314-316` gains its instanced sibling
  `drawPrimitives:vertexStart:vertexCount:instanceCount:`.
- Vulkan: `vkCmdDraw(cmd, 4, instanceCount, 0, firstInstance)`.

Glyph runs stay N draws, not N instances, in this letter — a text item is already
"not one draw" (`runtime/canvas/mod.rs`, `GEO_KIND_TEXT` doc) and folding it into the
instancing scheme is a change of shape, not of transport. It keeps its current
per-glyph path with the item block read from the buffer at its own index.

## Compatibility / Format Impact

- **No externally observable change**, with ONE named exception. No API, no
  `mfb man` output, no scene format, no golden.
- **The exception:** a Metal scene whose polygons sum past `METAL_MAX_FRAME_EDGES`
  (16384) previously rendered on the GPU via unbounded per-item `setFragmentBytes:`
  and now **declines to software** — the honest cap the frame edge region forces,
  the same one Vulkan has always had. Software is the oracle, so the picture is at
  least as correct; assert the decline via `MFB_CANVAS_STATS`, and name this class
  in the Phase 2 commit message. Every other scene renders byte-identically per
  path.
- **`GRAPHICS_STATE_SIZE` changes** — internal to the runtime, not a stable ABI.
- **The two `.spv` blobs are regenerated.** They are checked in
  (`src/codegen/runtime/canvas/shaders/*.spv`), so the commit contains binary churn;
  `scripts/regen-spirv.sh` is the only sanctioned producer.
- **`.ncodesum` churn is expected on every target that emits the canvas runtime**,
  because the Vulkan emitter and the Metal emitter both change instruction sequences.
  Regenerate with `scripts/regen-ncodesum.sh` and prove the delta is only this
  letter's, per `.ai/testing-gates.md`.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit
> as the work it describes; use `- [~]` for partial with a one-line remainder; mark a
> task moot with `- [x] ~~text~~ — moot: <evidence>`; fill each `Commit:` the moment
> the phase lands. **An unticked box means NOT DONE.**

### Phase 1 — Prove instancing is bit-identical, on Vulkan only, smallest scene

Vulkan first because its shader change is compiled by a tool with reflection output,
so the stride question gets a measured answer rather than an assumed one.

- [ ] Add `CANVAS_MAX_FRAME_ITEMS` and the three `…_VULKAN_ITEM_*` graphics-state
      slots to `src/codegen/runtime/canvas/mod.rs`; grow `GRAPHICS_STATE_SIZE`.
- [ ] Allocate, bind and persistently map the item buffer in `vulkan.rs` beside the
      edge buffer's creation; add it as `binding = 1` on the existing set layout.
- [ ] Rewrite `mfb_canvas.vert`/`.frag` to read `itemBuf.items[gl_InstanceIndex]`,
      passing the index to the fragment stage as a `flat` varying.
- [ ] Run `scripts/regen-spirv.sh` and **record glslang's reported array stride for
      the item struct** in `ITEM_BLOCK_SIZE`'s doc comment. If it is not 112, pad to
      the reported value and say so here in Corrections.
- [ ] Convert the Vulkan shape emitter to write the buffer and issue
      `vkCmdDraw(…, instanceCount, …)`.
- [ ] Add the frame-item-count check to `__canvas_vulkanRenderable`
      (`helper_render.rs`), declining past `CANVAS_MAX_FRAME_ITEMS`.
- [ ] Tests: extend `tests/rt_canvas_golden.rs` with a Vulkan-vs-oracle exact-match
      case over the existing smiley scene; keep `tests/rt_canvas_damage.rs` green.

Acceptance: on a Vulkan-capable Linux box, `MFB_CANVAS_GPU=1` renders the smiley
scene to a frame **byte-identical** to the one the same commit's software oracle
produces for it, and `MFB_CANVAS_STATS` reports `vulkanReady=TRUE` (proving the GPU
path actually ran — a frame identical to the oracle on a box where Vulkan declined is
the false pass `.ai/canvas-threading.md` §10 and the GPU-backend memory both warn
about).
Commit: —

### Phase 2 — Metal to the same mechanism

- [ ] Add `GRAPHICS_OFFSET_MTL_ITEM_BUFFER`; create the buffer with the device in
      `_mfb_macapp_metal_init`, sized for `CANVAS_MAX_FRAME_ITEMS` blocks plus the
      `METAL_MAX_FRAME_EDGES` × 16-byte edge region.
- [ ] Convert the Metal edge payload (`metal.rs:1017`) to writes into the edge
      region, storing each polygon's first-edge index in `ITEM_ARC_EDGE_BASE`;
      delete the edge-base-is-always-zero comment at `runtime/canvas/mod.rs`
      (`ITEM_OFFSET_ARC`) and rewrite it for the shared shape.
- [ ] Add the frame-total edge sum to `__canvas_metalRenderable`
      (`METAL_MAX_FRAME_EDGES`), keeping the per-item `MAX_EDGES` decline
      unchanged.
- [ ] Rewrite `METAL_SHADER_SOURCE` to take `constant MfbItem *items [[buffer(0)]]`
      with `[[instance_id]]`, and add the flat `iid` varying to `VOut`.
- [ ] Replace the 9 `setVertexBytes:`/`setFragmentBytes:` item-block sites with buffer
      writes; add the `drawPrimitives:vertexStart:vertexCount:instanceCount:`
      selector and use it.
- [ ] Add the frame-item-count check to `__canvas_metalRenderable`.
- [ ] Tests: `tests/rt_canvas_metal.rs` gains the same exact-match case, plus a
      polygon scene (edges through the region) and the negative case: a scene
      summing past `METAL_MAX_FRAME_EDGES` declines (assert via
      `MFB_CANVAS_STATS`, never by pixel equality).

Acceptance: on the macOS host, `cargo test --test rt_canvas_metal -- --no-fail-fast`
passes and the Metal frame for the smiley scene matches the oracle at least as
closely as it did at this letter's base commit (record both pixel-difference counts
in the commit message; the number must not increase).
Commit: —

### Phase 3 — Lift the 128-byte ceiling in the contract, and prove the whole suite

- [ ] Rewrite `ITEM_BLOCK_SIZE`'s doc comment in
      `src/codegen/runtime/canvas/mod.rs`: the bound is now the buffer, not the
      push-constant range. Say what the new bound is and what enforces it.
- [ ] Update the unit test at `vulkan.rs:5018` that pins the block against the
      guaranteed push-constant range — it is now asserting a constraint that no longer
      applies. Replace it with one pinning the block against the *buffer stride*
      glslang reports, so the two-language agreement stays gated.
- [ ] Update `.ai/canvas-threading.md` §10 to describe the buffer transport. It
      currently explains the predicate asymmetry by Metal's edges crossing as a
      per-item `setFragmentBytes:` payload — after this letter both backends carry
      edges in a frame buffer, Metal is frame-capped too, and only the per-item
      `MAX_EDGES` decline (kept by policy, §Non-goals) and the per-glyph vs
      per-frame glyph caps still differ. Rewrite the section to say exactly that.
- [ ] Run `scripts/regen-ncodesum.sh` and prove every `.ncodesum` diff is this
      letter's.
- [ ] Tests: full `cargo test --no-fail-fast`.

Acceptance: `cargo test --no-fail-fast` is green on the macOS host **and** on the
Linux CI axis (`.ai` memory: CI is linux + DEBUG, local gates are mac + RELEASE — a
green local run proves neither axis alone), `scripts/artifact-gate.sh all` reports 0
diffs, and `tests/golden/canvas/smiley.png` is unchanged on disk.
Commit: —

## Validation Plan

- **Tests:** `tests/rt_canvas_golden.rs`, `tests/rt_canvas_metal.rs`,
  `tests/rt_canvas_rasteriser.rs`, `tests/rt_canvas_damage.rs`,
  `tests/rt_canvas_graphics_thread.rs`, and on the Vulkan side
  `scripts/test-canvas-vulkan.sh target/release/mfb` (the oracle-vs-GPU diff
  harness — `.ai/testing-gates.md` §"The GPU comparison is against the ORACLE").
  Negative cases: a scene with more than `CANVAS_MAX_FRAME_ITEMS` items and a Metal
  scene past `METAL_MAX_FRAME_EDGES` must each **decline to software**, not
  truncate — assert via `MFB_CANVAS_STATS`.
- **Coverage check:** the new buffer code is in the *emitters*, which are compiler
  code reached only when a canvas program is built. Confirm the new lines are in the
  denominator with `cargo llvm-cov --bin mfb` per `.ai` memory — a green
  `cargo test` here can mean the emitter never ran.
- **Runtime proof:** `MFB_CANVAS_GPU=1 MFB_CANVAS_DUMP=/tmp/f.rgba` on the smiley
  scene, on both a Metal host and a Vulkan box, diffed byte-for-byte against the
  software dump of the same scene.
- **Doc sync:** `.ai/canvas-threading.md` §10; `ITEM_BLOCK_SIZE` doc comment. No
  `mfb spec` change — nothing observable moves.
- **Acceptance:** `cargo test --no-fail-fast`, `scripts/test-accept.sh`,
  `scripts/artifact-gate.sh all`, and
  `rustup run 1.96.0 cargo fmt --all && (cd repository && rustup run 1.96.0 cargo fmt)`.

## Open Decisions

- **`CANVAS_MAX_FRAME_ITEMS` = 4096.** Recommended as a starting value (448 KiB).
  The alternative is to size the buffer from the scene at frame start, which removes
  the cap entirely but adds an allocation on the graphics thread — a thread that today
  allocates nothing (`.ai/canvas-threading.md` §3, "the graphics thread never returns
  memory"). Recommend the fixed cap; revisit only if a real scene hits it. (§4.1)
- **Glyph runs stay N draws.** Recommended. Folding them into instancing is a
  separate, larger change and this letter's gate is "nothing moved". (§4.3)
- **`METAL_MAX_FRAME_EDGES` = 16384, mirroring Vulkan.** Recommended: one number to
  reason about across both backends, and the region costs 256 KiB. The cost is the
  §Compatibility decline for >16384-edge Metal scenes; raising both caps together
  later is one-line-per-backend. (§4.1)

## Corrections

- **C1 (2026-09-01, review — pre-execution).** As first written, this letter moved
  only the item block and left Metal's polygon edge payload as a per-item
  `setFragmentBytes:` — which an instanced draw cannot rebind between instances, so
  every polygon would have silently ended the run, and plan-116-F (gradient stops)
  and plan-116-H (one draw per group) each collided with the same fact. The edge
  region moved into this letter; effort re-estimated large → x-large. The "9 call
  sites" population was also a miscount (it counted selector-constant declarations
  and comments); replaced with the classified emission sites in §2.

## Summary

The real engineering risk is the instance index reaching the fragment stage correctly
on both languages: get it wrong and every item draws with a neighbour's parameters,
which looks like a plausible picture rather than a failure. The reference-image
exact-match gate is what makes that visible, and Phase 1 buys the answer on the
cheapest possible scene before either real emitter is converted. Untouched: the
software rasteriser, the geometry cache and header, the scene ring, and every
`canvas::` surface a program can see.
