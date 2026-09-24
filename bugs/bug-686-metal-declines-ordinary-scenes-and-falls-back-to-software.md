# bug-686: Metal declines ordinary scenes to software — an obsolete per-polygon cap, frame caps a real scene exceeds, and a pipeline that cannot feed the GPU

Last updated: 2026-09-23
Effort: huge (>3d)
Severity: HIGH
Class: Correctness

Status: Open
Regression Test: `examples/gpu` (Phase 4) + `tests/canvas/` rows added per phase

A `Mode.Canvas` program draws a map with a few thousand items, some of them
polygons of a few hundred points, and gets **one frame every several seconds**
with `gpuSelected=TRUE` and the GPU idle. Three separate things cause it, and
each one alone is enough to sink the frame:

1. **A per-polygon edge cap of 256 that the code itself documents as obsolete.**
   One oversized polygon makes `__canvas_metalRenderable` decline the **whole
   frame**, and the MFBASIC software rasteriser draws it instead.
2. **Frame caps an ordinary scene exceeds** — 4096 quads and 16384 edges — with
   the same whole-frame decline past either.
3. **A per-item pipeline that cannot feed a GPU even when it accepts the
   scene.** With Metal drawing, the submit costs ~10 ms/frame while the MFBASIC
   geometry work in front of it costs ~200 ms/frame.

**The single correct behavior a fix produces:** a scene of a few thousand items,
including polygons of a few hundred edges, renders on Metal at an interactive
frame rate, and the software rasteriser is a fallback for a *capability* the
backend genuinely lacks, not for ordinary geometry.

References:

- `src/codegen/builtins/canvas/helper_render.rs:590` `__CANVAS_METAL_MAX_EDGES`,
  `:588` `__CANVAS_MAX_FRAME_ITEMS`, `:597` `__CANVAS_METAL_MAX_FRAME_EDGES`,
  `:696` `__CANVAS_VULKAN_MAX_FRAME_EDGES` — the MFBASIC copies, which are what
  the `*Renderable` predicates actually test.
- `src/codegen/runtime/canvas/mod.rs:681` `CANVAS_MAX_FRAME_ITEMS`, `:723`
  `METAL_MAX_FRAME_EDGES`, `:662` `VULKAN_MAX_FRAME_EDGES`, `:763`
  `METAL_BUFFER_BYTES` — the Rust constants that size the buffer.
- `src/target/macos_aarch64/app/metal.rs:133-135` — `METAL_EDGE_BASE`,
  `METAL_GRADIENT_BASE`, `METAL_GLYPH_BASE`, hand-maintained literals inside a
  `concat!` shader source.
- `planning/completed/plan-116-A-canvas-item-instance-buffer.md` — the letter
  that moved edges off `setFragmentBytes:` into the frame buffer, which is what
  makes the 256 cap obsolete.
- `bugs/completed/bug-682-*`, `bugs/completed/bug-683-*` — the arena and present
  leaks on this path, both fixed; neither explains this.
- Commit `5cf168dd6` — the O(n²) draw-list walk, fixed separately today. Also
  not this.

## Failing Reproduction

`examples/wind` renders an Equal Earth coastline (93 rings, the largest ~500
points) with ~5,700 streamline segments over it. On Metal, `MFB_CANVAS_GPU=1`:

```
gpuSelected=TRUE  metalReady=TRUE  frames=2   (in 24 seconds)
```

- Observed: ~1 frame per 5–30 s. Keyboard input is read and the view state
  changes, but the repaint never lands, so the controls appear dead.
- Expected: an interactive frame rate.

Reducing the largest ring below 256 points is enough to change `frames=2` into
`frames=114` over the same 24 s, with `gpuFrames=114` — i.e. **the entire
difference is whether Metal accepts the scene.**

Measured with a synthetic probe (N polygons of P points, new coordinates each
frame, 15 s runs, `gpuSelected=TRUE` throughout):

| items | points | edges | over which cap | frames/15s |
| --- | --- | --- | --- | --- |
| 95 | 8 | 760 | — | 708 |
| 1,000 | 8 | 8,000 | — | 287 |
| 2,000 | 8 | 16,000 | — | 114 |
| 2,500 | 8 | 20,000 | frame edges | **7** |
| 3,000 | 3 | 9,000 | — | 67 |
| 5,000 | 3 | 15,000 | frame items | **3** |
| 1,200 | 16 | 19,200 | frame edges | **11** |

The collapse tracks the caps exactly. A **static** 5,000-item scene — every
geometry probe of which should hit the cache — is just as slow as a moving one,
because the decline is about the scene's shape, not its novelty.

And the per-polygon cap, measured directly by rendering one polygon of N points
twice, software versus Metal, and comparing the frames byte for byte:

| polygon points | differing bytes (of 2.3 MB) |
| --- | --- |
| 16 | 2 |
| 64 | 4 |
| 200 | 4 |
| 256 | 4 |
| 300 | **363** |
| 400 | **363** |

A handful of bytes is antialiasing noise; 363 is a real divergence. So **256
edges is a genuine limit today** — 256 × 16 bytes is exactly the 4 KB
`setFragmentBytes:` payload — even though `plan-116-A` moved edges into the
frame buffer and the cap's own comment says it is no longer forced by the
transport. *Something still honours that boundary, and finding what is Phase 1.*

| Environment | | Result |
| --- | --- | --- |
| macos-aarch64, Metal | `gpuSelected=TRUE` | fails ✗ |
| macos-aarch64, software | default | same scenes, ~100× slower, correct picture |
| Linux / Vulkan | not measured | shares the MFBASIC predicates and caps; expect ✗ |

## Root Cause

**(1) The per-polygon cap.** `__canvas_metalRenderable` declines a frame
carrying any polygon past `__CANVAS_METAL_MAX_EDGES` (256). Its comment:

> The PER-ITEM cap, kept exactly as it was. plan-116-A moved Metal's edges into
> a frame buffer, so this one is no longer forced by the transport — but
> declining the same scenes Metal declined before is that letter's gate, and
> unifying the two backends' caps is later work, taken deliberately or not at
> all.

So it was knowingly retained as a behaviour-freeze. But the byte comparison
above shows Metal *does* diverge past 256 today, so the cap is currently load
bearing and removing it alone renders wrong pictures (confirmed: large polygons
draw as bounding boxes). Either the edge data is not fully on the frame-buffer
path, or a second 4 KB-shaped constraint remains. **Unresolved — Phase 1.**

**(2) The frame caps are duplicated, and the MFBASIC copies are authoritative.**
`CANVAS_MAX_FRAME_ITEMS` / `METAL_MAX_FRAME_EDGES` exist in Rust (sizing
`METAL_BUFFER_BYTES`) *and* as `LET __CANVAS_*` in MFBASIC helper source. The
`*Renderable` predicates read the MFBASIC ones. Raising only the Rust side
allocates a bigger buffer the predicate still refuses to fill — a silent no-op.

**(3) Raising them moves the buffer's regions, and the shader's region bases are
hand-maintained literals.** `METAL_EDGE_BASE = 212992` is
`CANVAS_MAX_FRAME_ITEMS × ITEM_BLOCK_SIZE / 4`, spelled out because
`METAL_SHADER_SOURCE` is a `concat!` that cannot interpolate. Change a cap
without recomputing all three and the shader reads edges from the middle of the
item region — valid memory, wrong numbers, plausible-looking wrong shapes.
`the_metal_shader_region_bases_match_the_buffer_layout` is the only thing
holding them together.

**(4) Even when Metal accepts the scene, the pipeline in front of it dominates.**
Per frame at 5,723 items, with Metal drawing every frame:

| phase | ms/frame |
| --- | --- |
| `__canvas_sceneOffsets` (MFBASIC geometry build) | **168** |
| `__canvas_sceneDraws` | 37 |
| Metal submit | **10** |

~29 µs per item, in MFBASIC, on the graphics thread, over arena-allocated
`List OF Float` with a `collections::getOr` per element. `generations` sits at
**1.9 per item per frame** — the second walk misses the geometry cache and
rebuilds everything, despite the render loop's comment asserting it hits. The
cache is 256 entries (`__CANVAS_GEO_CAPACITY`) against scenes of thousands.

## Goal

- **(1)** `__CANVAS_METAL_MAX_EDGES` is gone, or set to the frame-edge budget,
  and a polygon of several thousand edges renders identically on Metal and
  software.
- **(2)** `CANVAS_MAX_FRAME_ITEMS` and the edge caps admit a scene of tens of
  thousands of quads; the MFBASIC and Rust copies are **one** definition, not
  two that can drift.
- **(3)** The shader's region bases derive from the buffer-layout constants
  rather than being literals, so a cap change cannot silently desynchronise.
- **(4)** Metal sustains 60 fps on a scene of millions of triangles, reached by
  moving the geometry cache and the draw batching native and off the arena —
  Phases 5-8.
- **(5)** `examples/gpu` exists and measures all of the above.

### Non-goals (must NOT change)

- **The honesty gate itself.** Declining a scene the backend cannot reproduce is
  correct; the bug is declining ones it *can*. Never clamp a polygon's edges to
  fit — that renders a different shape, which is worse than falling back.
- **The software rasteriser as the oracle.** It stays the reference the GPU is
  measured against.
- **Do not "fix" this by simplifying the geometry that provokes it.** An earlier
  attempt regenerated `examples/wind`'s coastline under 256 points per ring; that
  hides a renderer defect behind the example's data and is explicitly rejected.
- **Metal must draw what software draws.** That is the property, and it is what
  every phase below is accepted against: render the same scene on both paths and
  compare the frames. The software rasteriser is the oracle; a GPU frame that
  disagrees with it is wrong however plausible it looks. The canvas goldens are
  a regression net against *reference scenes* and must keep passing, but a golden
  file changing is not by itself a failure and its byte-identity is not the
  criterion — an arithmetic change that moves a low bit is fine, a Metal frame
  that stops matching software is not.

## Blast Radius

- `helper_render.rs` `__canvas_metalRenderable` / `__canvas_vulkanRenderable`
  and the four `LET __CANVAS_*MAX*` — **the caps**.
- `runtime/canvas/mod.rs` `CANVAS_MAX_FRAME_ITEMS`, `METAL/VULKAN_MAX_FRAME_EDGES`,
  `METAL_BUFFER_BYTES`, `METAL_*_BASE_WORDS` — **the buffer layout**.
- `metal.rs:133-135` and the Vulkan equivalent (`vulkan.rs:6589` checks a
  `GRADIENT_BASE` literal the same way) — **the shader bases**.
- `__canvas_sceneOffsets` / `__canvas_sceneDraws` / `__canvas_geometryFor` and
  `__CANVAS_GEO_CAPACITY` — **the per-item pipeline**, goal (4).
- `examples/wind` — the discovery site; it must NOT be altered to dodge this.
- Every `Mode.Canvas` program with a detailed polygon or more than ~4000 quads.
- `planning/plan-150-*` `canvas::ParticleSystem` — expansion produces ordinary
  draw entries, so N particles inherit every cap and every per-item cost here.

## Fix Design

Phases 1–3 are constants and plumbing and are tractable. Phase 4 is not, and the
document should say so rather than imply a cap change buys it.

**On goal (4): the geometry cache and the draw batching move native.** That is
what the graphics thread was for, and it is not what it does. Today
`__canvas_renderLoop`, `__canvas_renderFrame`, `__canvas_sceneOffsets`,
`__canvas_sceneDraws`, `__canvas_geometryFor` and the whole software rasteriser
are MFBASIC helper source (`RegistryHelper` string constants in
`src/codegen/builtins/canvas/helper_*.rs`), compiled and run on that thread over
arena-allocated `List OF Float` / `List OF Integer`, a `collections::getOr` call
per element. The graphics trampoline pins an arena-state register specifically so
they can (`emit_graphics_trampoline`, `runtime/canvas/mod.rs:846` — its own
comment says this "is what gives the loop its own geometry cache").

5,000,000 triangles at 60 fps is ~3 ns of CPU per triangle. The pipeline spends
~29,000 ns per item and rebuilds each one twice a frame. That gap is not a cap,
a cache size or a constant — it is the cost model of the language the renderer
is written in, applied per element, in the hot loop.

The migration, in dependency order:

* **The ring boundary first — by reading what is already there.** A `List` is
  *already* a flat block: a header, then a contiguous data region at
  `header + capacity * ENTRY`, fixed stride per element. Nothing needs
  serialising into a second representation. What the graphics thread needs is to
  walk that layout in native code instead of through `collections::get` per
  element — which is the only reason it pins an arena at all, since reading a
  collection *as an MFBASIC value* is what requires one.

  The repo already has hand-rolled native collection readers
  (`_mfb_rt_fs_path_join`, `_mfb_rt_sort_string_list`), and the rule for writing
  another is in `.ai/collections.md`: the data base is `capacity`, never `count`
  — go through `emit_collection_data_pointer`, because a count-based base reads
  garbage the moment a list has grown. A `Polygon`'s points are a nested `List`
  reached by pointer, flat on the same terms.
* **The geometry cache.** A native open-addressed `hash -> offset` table over a
  flat `Vec<f32>`, process-global rather than arena-resident, replacing
  `__CANVAS_GEO_HASHES/OFFSETS/COUNTS/LASTUSED/DATA` and the 256-entry
  `__CANVAS_GEO_CAPACITY` that makes every real scene thrash to a 100% miss rate.
* **Geometry generation.** `__canvas_headerFor` / `__canvas_tailFor` per
  `DrawItem` kind, natively, writing straight into that buffer.
* **The draw batching.** `__canvas_sceneDraws`, `__canvas_pushOneDraw`,
  `__canvas_drawsJoin` and the instance assignment: run-building natively, and —
  the actual throughput win — geometry that did not change is **not rebuilt**.
  `generations` must fall from ~1.9 per item per frame to ~0 for a static scene.
* **The software rasteriser** last, since it is the oracle and only runs when a
  backend declines.

**The acceptance test is mechanical.** Delete the arena-state pinning from
`emit_graphics_trampoline`. If the graphics thread still runs, nothing on it is
MFBASIC any more. Today it would fault immediately.

## Phases

### Phase 1 — find what still honours 4 KB (no behavior change)

- [ ] Instrument or read the Metal edge path end to end and establish why a
      polygon past 256 edges diverges, given `plan-116-A`. Candidates: an edge
      slice still travelling as `setFragmentBytes:`; a per-item cap in the
      emitter's publish loop; an `int4` field that cannot address past 256.
- [ ] Add the byte-comparison harness from the reproduction (one polygon of N
      points, software vs Metal) as a test, failing at N=300.

Acceptance: the divergence is attributed to a named line; the test fails there.
Commit: —

### Phase 2 — remove the per-polygon cap

- [ ] Fix what Phase 1 found, then delete `__CANVAS_METAL_MAX_EDGES` (or set it
      to the frame-edge budget) and the Vulkan equivalent.

Acceptance: a polygon of 300, 1000 and 4000 edges draws the same picture on
Metal as on software — the byte comparison from the reproduction, now passing at
sizes where it failed. The canvas suite still passes.
Commit: —

### Phase 3 — one definition of the caps, and derived shader bases

- [ ] Make the shader's three region bases derive from the buffer-layout
      constants instead of being literals (`METAL_SHADER_SOURCE` becomes a
      substituted template, or the literals are generated).
- [ ] Give the MFBASIC predicates the Rust constants rather than their own
      `LET` copies, so the two cannot drift.
- [ ] Raise `CANVAS_MAX_FRAME_ITEMS` and the edge budgets to admit tens of
      thousands of quads; size `METAL_BUFFER_BYTES` accordingly.

Acceptance: the probe table above has no cliff, and every row of it agrees with
software — this is the phase that moves buffer regions, so a row that renders
fast and wrong is the exact failure to catch.
`the_metal_shader_region_bases_match_the_buffer_layout` passes by construction.
Commit: —

### Phase 4 — `examples/gpu`

- [ ] A canvas program that sweeps item count, points per item, static vs
      moving, and primitive kind; reports frames rendered, the per-phase frame
      breakdown, `gpuSelected`/`gpuFrames`, and peak RSS.
- [ ] A `--compare` mode that renders the same scene on software and on the GPU
      and reports differing bytes — the oracle check, so a throughput win that
      draws the wrong picture cannot be mistaken for a win.
- [ ] A README stating what each knob probes and what a healthy number is.

Acceptance: one command reproduces every row in this document.
Commit: —

### Phase 5 — the graphics thread reads the published scene natively

- [ ] Walk the published `List OF DrawItem` (and a `Polygon`'s nested points
      list) from native code against the collection layout, replacing the
      `collections::` element reads in the scene walk. No new representation and
      no second copy of the scene — the block is already flat.
- [ ] Follow `.ai/collections.md`: base the data region on `capacity` via
      `emit_collection_data_pointer`, never on `count`.

Acceptance: `examples/gpu --compare` clean across the sweep; the frame breakdown
shows the scene walk no longer touching the arena; the canvas suite passes.
Commit: —

### Phase 6 — the geometry cache and generation go native

- [ ] Native `hash -> offset` table plus a flat float arena, replacing the five
      `__CANVAS_GEO_*` globals and `__CANVAS_GEO_CAPACITY`.
- [ ] `headerFor`/`tailFor` per `DrawItem` kind, natively.

Acceptance: `examples/gpu --compare` clean — the native generator and the
software one must still agree, which is the whole risk of rewriting geometry
twice. Per-item geometry cost drops by an order of magnitude; a static scene
reports `generations` 0 after warm-up; the canvas suite passes.
Commit: —

### Phase 7 — native batching, and stop rebuilding what did not change

- [ ] `sceneDraws` / `pushOneDraw` / `drawsJoin` and instance assignment native.
- [ ] An item whose content hash is unchanged since the last frame keeps its
      geometry and its instance slot; only the changed ones are rebuilt and
      re-uploaded.

Acceptance: `generations` ~0 for a static scene and proportional to the *changed*
item count for a moving one; `examples/gpu` sustains 60 fps at a scene size named
in Phase 4's README.
Commit: —

### Phase 8 — the graphics thread is native

- [ ] Port the software rasteriser, then delete the arena-state pinning from
      `emit_graphics_trampoline`.

Acceptance: the graphics thread runs with no arena; `examples/gpu --compare`
clean; full canvas suite green.
Commit: —

## Validation Plan

- Regression tests: the software-vs-Metal byte comparison at several polygon
  sizes; a no-cliff frame-rate row; `the_metal_shader_region_bases_match_the_buffer_layout`.
- Runtime proof: `examples/wind` interactive with its coastline **unmodified**,
  and `examples/gpu` reproducing the tables here.
- Doc sync: `.ai/canvas-threading.md` and the plan-116-A notes, once the
  per-polygon cap goes.
- Suite: `cargo test --test 'rt_canvas_*'`. The goldens are the regression net;
  `examples/gpu --compare` is the correctness check, because it compares the two
  backends against each other rather than either against a stored file.

## Open Decisions

- Whether Phase 3 raises the caps to a fixed larger number or sizes the buffer
  from the scene. Recommended: **fixed and generous** — the buffer is a few MiB
  and a dynamic size adds a resize path to the hottest code in the renderer.

## Summary

Phases 1–3 are well-understood and bounded: one obsolete gate, one duplicated
constant, one hand-maintained offset. The risk in them is entirely that a cap
and a shader offset must move together — get that wrong and nothing fails, the
picture is just quietly incorrect, which is how this document's author produced
two wrong frames before finding it. That is why every acceptance below is
'Metal agrees with software', not 'a file did not change': the wrong frames in
question would have passed a file-identity check on every scene that still
declined. Phase 4 is the instrument that should have
existed before any of this was touched. Phases 5-8 are the architecture change
goal (4) actually needs — the geometry cache and the draw batching move off
MFBASIC and off the arena onto the graphics thread's own native code, which is
what that thread exists for. The risk there is the one `.ai/collections.md`
names: a hand-rolled reader that bases the data region on `count` instead of
`capacity` reads garbage from any list that has grown, and does it silently.
