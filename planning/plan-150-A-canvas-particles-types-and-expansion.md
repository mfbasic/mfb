# plan-150-A: Particle systems — types, the analytic model, and expansion

Last updated: 2026-10-11 (re-derived against the completed plan-172; see "Re-derivation
record" below)
Overall Effort: x-large (1d–3d)
Effort: large (3h–1d)
Depends on: plan-172 (all of A–N; complete and archived — see Prerequisites)

`canvas::ParticleSystem` becomes the **twelfth `DrawItem` variant**: one scene item
that expands into N drawn particles, each an instance of an arbitrary template
`DrawItem` — a `Picture` sprite, a `Circle`, a `Polygon`, anything the union already
holds — placed, rotated, scaled and tinted from a **closed-form** function of its slot
index and the system's clock.

```
canvas::ParticleSystem
    position   AS vector::Float2   ' the emitter's reference point
    item       AS DrawItem         ' the template, drawn once per live particle
    emitter    AS Shape            ' where particles are born
    emission   AS Emission         ' how many, how often, how long they live
    motion     AS Motion           ' how they move
    appearance AS Appearance       ' how they look across their life
    seed       AS Integer          ' fixes every per-particle random draw
    time       AS Float            ' the program's clock, in seconds
```

Behavioral outcome: a program presents a scene containing one `ParticleSystem` with
`time` advanced each frame, and sees a fire / fog / fireworks effect that is
**deterministic** — the same `(seed, time)` produces the same picture on every run, on
every backend, on every platform.

The design's load-bearing decision is **expansion, not a particle shader**. A
`ParticleSystem` is expanded on the graphics thread into N ordinary draw entries, the
way `Group` already expands (`helper_render.rs:__canvas_appendDraw`). Because the
per-particle state is expressed entirely in fields the item model *already has* — each
shape's own `position` and `transform` (a full affine, plan-172-C/D/F) and its
`paint.fill`/`paint.stroke` (tint) — the expansion emits ordinary items, and **both GPU
backends and the software rasteriser draw them with no change to any shader, pipeline,
binding, or buffer layout.**

References:

- `planning/new_canvas.md` — the canvas shape model plan-172 built. Its "Transform
  order, spelled out" is the rule every particle is placed by, and its "Particles -
  plan 150" section is the owner's statement of this letter's type set.
- `.ai/canvas-threading.md` — §2 (arena state is per-thread), §3 (the scene ring),
  §4 (the **seven** redraw triggers), §13 "A group node is expanded before any consumer
  sees it", §14 (the geometry cache), §16 (`AutoPaint` instances), §17 (a `Picture` is
  one or two draw entries). **Read §4 before touching this letter**: the system is
  advanced by the program re-presenting with a new `time`, never by a timer. Trigger 7
  is the one clock in the renderer and it belongs to `AutoPaint` alone.
- `src/codegen/builtins/canvas/mod.rs` — the registry declarations and the frozen-set
  tests this letter must amend.
- `src/codegen/builtins/canvas/helper_render.rs` — `__canvas_appendDraw` and
  `__canvas_composeGroup`, the `Group` expansion this design mirrors, and
  `__canvas_groupSignature`, the worker-side walk the raises sit beside.
- `src/codegen/runtime/canvas/mod.rs` — `ITEM_BLOCK_SIZE` and the `ITEM_OFFSET_*`
  layout, the evidence that no new transport is needed.
- `AGENTS.md` — the never-edit-a-test-to-pass rule. Two frozen-set tests are amended by
  this letter; each amendment is an addition, never a loosening.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-172 (the canvas shape model) is complete and archived, every letter A–N | `ls planning/plan-172-*` → no matches, and `ls planning/completed/plan-172-M-*` → present | MET (2026-10-13 at `29f5500cb`: no matches; `planning/completed/plan-172-M-canvas-closeout.md` present) |
| The `DrawItem` union is plan-172's eleven variants, ending `AutoPaint`, `Picture`, `Group` | `cargo test --bin mfb draw_item_variant_set_is_frozen` → pass against that list | MET (2026-10-13: "1 passed") — against a list of **thirteen**, not eleven: plan-164-A put `PolyCurve` and `CurvePolygon` in `Shape`. It still ends `AutoPaint`, `Picture`, `Group`. See Corrections 1 |
| Every shape carries its own `transform`, and `Paint` has none | `grep -c 'transform_prop()' src/codegen/builtins/canvas/mod.rs` → ≥ 8, and `mfb man canvas types` shows no `transform` under `Paint` | MET (2026-10-13: the grep → 11, the helper plus ten uses; `target/debug/mfb man canvas types`, built at `29f5500cb`, lists no `transform` row in the `canvas::Paint` table) |
| `canvas::Shape` exists, for `emitter AS canvas::Shape` | `grep -c 'name: "Shape"' src/codegen/builtins/canvas/mod.rs` → 1 | MET (2026-10-13: 1) |
| No other plan has appended a `DrawItem` variant since | `grep -c '"ParticleSystem"\|"Path"' src/codegen/builtins/canvas/mod.rs` → 0 | MET (2026-10-13: 0). plan-164 DID land first, as two members of `Shape` rather than a `Path` variant of `DrawItem` — Corrections 1. If plan-164 lands first, `Path` is the twelfth variant and `ParticleSystem` the thirteenth: re-read every "twelfth"/"tag 11" below |
| The canvas suite is green before any edit | `cargo test --test rt_canvas_golden --test rt_canvas_rasteriser` → pass | MET (2026-10-13: "26 passed; 0 failed" and "106 passed; 0 failed; 2 ignored") |

plan-150-B and plan-150-C inherit this gate through their dependency on this letter.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again
> before you decide to stop. If you stop, report the status of *all* prerequisites.

### Re-derivation record (2026-10-11)

This document was written before plan-172 and was re-derived against the tree at
`af940da94` once plan-172 was archived. What changed, so a reader of the old text knows
what not to trust from memory:

- **Twelfth variant, not eleventh.** `AutoPaint` is in the union. `DrawItem` is now three
  nested unions (`Shape`, `AutoShape INCLUDES Shape`, `DrawItem INCLUDES AutoShape`), and
  `ParticleSystem` is appended to `DrawItem`'s **own** members, after `Picture` and
  `Group`.
- **`Paint.transform` is gone.** The per-particle affine goes into the template item's
  own `position` and `transform` (§6). `canvas::Point` is gone; every coordinate is a
  `vector::Float2`.
- **The emitter is `canvas::Shape`.** The `Emitter` union and the four `Emit*` records
  are deleted from this plan, and with them `emitter_variant_set_is_frozen`. §5's spawn
  table is rewritten per shape.
- **The rotation pivot is settled by plan-172**: a shape's transform pivots about its
  reference point. The bounding-box-centre computation and its Open Decision are gone.
- **A `Picture` has no paint of its own** and holds a shape; an `AutoPaint` holds a
  shape. §6 says what a particle does with each.
- **Particles are never hit-tested**, and an `AutoPaint` in a template draws its default
  paint (`planning/new_canvas.md`, "Particles - plan 150"). Both are new work: §6.
- **Error codes 77050025–77050027 are taken** (`ErrInvalidSurrogate`,
  `ErrCanvasGroupLimit`, `ErrTooLarge`). The two raises move to the next free codes.
- **Every measured number moved**: see "Measured populations". The frame cap is no
  longer 4096 and no longer one constant; the item block is 368 bytes; the draw entry
  is 16 words; the geometry cache is no longer a 256-entry LRU.

## 1. Goal

- `canvas::ParticleSystem` exists as the twelfth `DrawItem` variant and renders in
  software.
- The same `(seed, time)` renders byte-identically across runs — the system is a pure
  function of its fields, holding no state between frames.
- The template may be any other `DrawItem` variant: the eight shapes, an `AutoPaint`
  (drawn with its default paint), a `Picture`, or a `Group`.
- The emitter may be any of the eight `Shape` variants.
- Both GPU backends render a particle scene within `Tolerance::GPU_DEFAULT` of the
  software oracle **without any shader, pipeline or buffer-layout change** — this is
  a consequence of the design, and Phase 5 is the measurement that proves it.
- A particle is never hit: it adds nothing to `MouseEvent.overID` and blocks nothing
  beneath it.

### Non-goals (explicit constraints)

- **No field advection / wind streamlines.** Path-dependent motion needs per-particle
  state carried across frames; every quantity in this letter is closed-form in
  `(slot, time)`. Out of scope, deliberately, and not a later phase of this plan.
- **No user-supplied shaders.** The `DrawItem` set stays closed; nothing in this plan
  compiles user source.
- **No change to `ITEM_BLOCK_SIZE` (368), `VULKAN_MAX_FRAME_ITEMS` /
  `METAL_MAX_FRAME_ITEMS` (65,536 each), the `ItemBlock` layout, the draw-entry width
  (16 words), or either shader.** A particle's quads count against the existing frame
  caps exactly as any other item's do. If a system exceeds one, the frame declines to
  software — correct, and slow, and measured in letter B rather than pre-empted by a
  guessed new constant.
- **No new redraw trigger.** `time` is a field the program sets; advancing it changes
  the item's hash and fires trigger 1 (`present`). A timer-driven repaint would be an
  eighth trigger, and `.ai/canvas-threading.md` §4 admits exactly one clock, trigger 7.
- **No GPU-side simulation and no persistent GPU state.** Every frame is computed from
  `(seed, time)` alone.
- **No hover or press state on a particle.** An `AutoPaint` template is drawn as its
  shape with its default paint; nothing is noted in the `AutoPaint` instance table.
- **`Paint`, `Transform`, `BlendMode`, `CapStyle`, the `Shape` and `AutoShape` unions and
  the existing eleven variants' records are unchanged.** This letter only *adds*.

## 2. Current State

A scene is a flat `List OF DrawItem` deep-copied by `present`
(`tests/canvas/rt_canvas_present_deep_copy.rs`). On the graphics thread,
`helper_render.rs:__canvas_sceneOffsets` resolves the frame in three passes: a native
pass (`canvas::sceneResolve`) that builds what it can, an MFBASIC pass that sends every
index the native pass left through `__canvas_appendDraw`, and a native layout pass
(`canvas::sceneLayout`). `__canvas_appendDraw` resolves an item's geometry through the
cache and appends one offset per drawn entry, noting each entry's composed offset,
matrix number and draw hash in `__CANVAS_DRAW_DX` / `_DY` / `_MAT` / `_HASHES`
(`__canvas_noteEntry`).

**`Group` is the precedent this design mirrors.** `__canvas_appendDraw`'s `CASE Group(g)`
arm composes the node with its parents (`__canvas_composeGroup` — a full affine since
plan-172-G, answering a matrix number and leaving the composed translation in
`__CANVAS_COMPOSED_X` / `_Y`) and recurses over `canvas::groupItems(...)`. It is bounded
by `__CANVAS_GROUP_MAX_DEPTH = 64`, and the depth *raise* happens on the worker inside
`present` (`__canvas_groupSignature`) — never on the graphics thread, where a `FAIL` has
no user frame to unwind to. The arm's own stop is silent, and its comment says why.

**Three variants already draw through something they hold.** `Group` is a container
(no `paint`); `AutoPaint` and `Picture` are decorators (no `paint` of their own; they
draw a `shape`). The test `every_draw_item_variant_carries_a_paint` names both kinds of
exemption explicitly — `CONTAINERS` and `DECORATORS` — rather than dropping the check.

**One item can already be several draw entries.** A `Picture` is one, two or no entries
(`__canvas_pictureAppend`, `.ai/canvas-threading.md` §17), and `__canvas_sceneOffsets`
handles an index whose entry count is not one (`several`, the `-2` offset that sends the
frame to `__canvas_sceneDrawsWalk`). A `ParticleSystem` is the first item whose count is
unbounded by its type.

**The GPU walk is not the software walk.** `__canvas_sceneDrawsWalk` /
`__canvas_drawGroup` / `__canvas_memoGroup` lay out blocks and draw entries; a group's
blocks are recorded once and shared by every use of it. Both walks call
`__canvas_composeGroup`, which is what keeps them agreeing.

**The worker has a walk of its own** for hit-testing (`helper_hit.rs:__canvas_hitAppend`,
plan-172-I), into the worker's own geometry cache.

### Measured populations

All measured 2026-10-11 at `af940da94`.

| What | Count | Command |
|---|---|---|
| `DrawItem` variants today | 11 | `draw_item_variant_set_is_frozen` pins the list: `Rectangle`, `Line`, `Polygon`, `Circle`, `Arc`, `Text`, `Ellipse`, `PolyLine`, `AutoPaint`, `Picture`, `Group` |
| Records in the `canvas` registry | 20 | `grep -c 'pkg.add_record(RegistryRecord {' src/codegen/builtins/canvas/mod.rs` → 20 |
| Unions in the `canvas` registry | 3 | `grep -c 'pkg.add_union(RegistryUnion {' src/codegen/builtins/canvas/mod.rs` → 3 (`Shape`, `AutoShape`, `DrawItem`) |
| Enums in the `canvas` registry | 13 | `grep -c 'pkg.add_enum(' src/codegen/builtins/canvas/mod.rs` → 13 |
| Registry functions in `canvas` | 95 | `grep -c 'pkg.add_function' src/codegen/builtins/canvas/*.rs \| awk -F: '{s+=$2} END {print s}'` → 95 |
| Canvas runtime test files | 21 | `ls tests/canvas/*.rs \| wc -l` → 21 |
| `Paint` fields | 13 | the `Paint` record in `mod.rs`: `fill`, `stroke`, `strokeWidth`, `blend`, `clip`, `clipRule`, `gradient`, `cap`, `join`, `miterLimit`, `dashes`, `dashOffset`, `transparency` — no `transform` |
| MFBASIC `MATCH` arms naming `Group` / `Picture` | 13 / 12 | `grep -c 'CASE Group(' src/codegen/builtins/canvas/*.rs` → `helper_render.rs` 6, `helper_hit.rs` 3, `helper_geometry.rs` 4; `grep -c 'CASE Picture('` → 5, 3, 4. Every one is a walk that must decide what a `ParticleSystem` is to it |
| Item block size / draw-entry width | 368 B / 16 words | `grep -n 'const ITEM_BLOCK_SIZE\|const CANVAS_DRAW_ENTRY_WORDS' src/codegen/runtime/canvas/mod.rs` |
| Frame block cap, per backend | 65,536 | `grep -n 'const VULKAN_MAX_FRAME_ITEMS\|const METAL_MAX_FRAME_ITEMS' src/codegen/runtime/canvas/mod.rs`. `CANVAS_MAX_FRAME_ITEMS` no longer exists as a constant (`grep -rn CANVAS_MAX_FRAME_ITEMS src` → 2 hits, both prose) |
| Geometry header size | 67 floats | `grep -n 'LET __CANVAS_GEO_HEADER' src/codegen/builtins/canvas/helper_geometry.rs` |
| Geometry cache capacity | none fixed | `grep -rn 'GEO_CAPACITY\|__CANVAS_GEO_LIVE' src` → 0. The 256-slot LRU is gone (bug-686): slots are stamped per frame and dropped at `canvas::geoBeginFrame` (`.ai/canvas-threading.md` §14) |
| Highest error code in use | 77050027 | `grep -rhoE '7705[0-9]{4}' src build.rs \| sort -u \| tail -1` → 77050027, so the next free is 77050028 |

### Verified properties

- **A particle's full state is expressible in existing item fields.** VERIFIED by
  reading the registry: each of the eight shapes carries `position AS vector::Float2`
  and `transform AS Transform` (`transform_prop()`, a full affine `a b c d tx ty`), and
  a `paint` with `fill` and `stroke`. `helper_paint_defaults.rs` states the rule the
  renderer applies: a local point `p` reaches the surface at `position + transform(p)`.
  Position goes in `position`; rotation and scale go in `transform`; colour goes in
  `paint.fill`/`paint.stroke`.
- **A shape's transform pivots about its reference point.** VERIFIED from
  `planning/new_canvas.md` ("The transform pivots about the reference point") and the
  plan-172-D doc comment in `helper_paint_defaults.rs`. A particle therefore spins
  about the particle point with no bounds computation.
- **A transform with no area is treated as the identity, on a shape — and that is a
  trap here.** VERIFIED by reading `__canvas_transformActive`: all-zero, or
  `|a*d - b*c| < 1e-12`, answers FALSE and the shape is drawn untransformed. A particle
  whose scale reaches zero would therefore draw at **full size**. §6 makes such a
  particle dead instead.
- **Tint-by-multiply is already the documented rule for `Picture`.** VERIFIED by
  reading the `Picture` record's description in `mod.rs`: the shape's paint is the
  picture's, and "its fill colour tints the image". The particle tint reuses that rule
  and works for sprite particles and shape particles identically.
- **The GPU transport already carries everything a particle needs.** VERIFIED by
  reading `src/codegen/runtime/canvas/mod.rs`: the `ItemBlock` has
  `ITEM_OFFSET_TRANSFORM` (128) and `ITEM_OFFSET_FILL`/`ITEM_OFFSET_STROKE` (32/48). No
  new region, no new binding, no widened block.
- **UNVERIFIED — a recursive variant.** `item AS DrawItem` inside a `DrawItem` variant
  is a type cycle. `AutoPaint.shape AS Shape` and `Picture.shape AS AutoShape` are
  unions inside variant records, which is precedent for the nesting but **not** for the
  cycle: neither can hold itself. `.ai/collections.md` says a value "whose type reaches
  a type cycle" is copied with the graph walker (`needs_graph_copy`, plan-134-D).
  **This is the letter's highest-risk premise** — see Phase 1, which falsifies it first
  and cheaply, in the registry, in `present`'s deep copy, and in the two native walks
  that fold a scene (`func_item_hash.rs`, `func_carried_hashes.rs`).
- **UNVERIFIED — that the `several` path takes an index of N entries.** It was built
  for a `Picture`'s two. Phase 4 measures it at N = 1,000 before relying on it.
- **UNVERIFIED — what N fresh geometry records per frame cost.** N particles have N
  distinct hashes, so each is built and dropped every frame. Nothing is evicted to make
  room for them any more, so the old "thrash" question is now a build-and-compact cost.
  It is **letter B's opening measurement**, not an assumption here. Phase 4 emits
  particles through the ordinary `__canvas_geometryFor` path precisely so that B
  measures the real thing.

## 3. Design Overview

Five pieces, layered:

1. **The type set** — four new records, one new enum, and the twelfth `DrawItem`
   variant. Registry data only; no behaviour.
2. **The analytic model** — a pure function `slot → (alive, position, rotation, scale,
   colour)` from `(seed, time, position, emitter, emission, motion, appearance)`.
   Written once, in MFBASIC, on the graphics thread.
3. **The expansion** — a `CASE ParticleSystem(ps)` arm in `__canvas_appendDraw` that
   loops slots, skips dead ones, composes each live one's item, and appends it; and its
   twin in the GPU walk.
4. **Validation** — the raises, on the worker inside `present`, never on the graphics
   thread.
5. **Rendering** — nothing. Falls out of 3.

**Where design uncertainty concentrates: the recursive type.** `item AS DrawItem`
inside a `DrawItem` variant is a type cycle, and `.ai/collections.md` records four
distinct bugs in that machinery (bug-601, bug-536 shape C, plan-134-E, plan-134-G).
If the registry or the copy path cannot express it, the whole shape of the type
changes. **Phase 1 is therefore the cheapest experiment that could falsify it** — declare
the type, present a scene containing one, and watch it survive `present`'s deep copy.
Nothing else is built until that holds.

**Where correctness risk concentrates: the expansion loop and the walks beside it.** The
loop runs on the graphics thread, where a `FAIL` has nowhere to unwind. Every raise must
therefore happen on the worker, in `present`. And plan-172 left the package with more
walks over a scene than it had — the software walk, the GPU walk, the hit walk, the
group signature, two native folds — each of which must give a `ParticleSystem` a
deliberate answer. Scheduled last, behind tests, in Phase 4.

**Byte-identity is NOT this plan's gate.** This plan adds a new variant and new
rendering; codegen for every existing program is expected to be unchanged, but that
is a side-condition, not the acceptance check. The gates are runtime behaviour and
goldens. Programs that use no `ParticleSystem` are expected to produce byte-identical
`.ncode`; a diff there is a bug in this letter to root-cause, never a signal the
design is dead.

### Rejected alternatives

- **A particle shader deriving positions from `gl_InstanceIndex`.** Rejected because
  `item AS DrawItem` admits any template — `Picture`, `Text`, `Polygon`. A shader would
  have to reproduce every primitive's SDF *and* a derived transform, in MSL and GLSL,
  and the software rasteriser would need a third copy for the oracle. Expansion gets
  arbitrary templates for free. Revisit only if letter B measures the expansion as the
  bottleneck **and** a single-primitive fast path is worth a fourth implementation.
- **A per-particle compact record in a new frame-buffer region.** Rejected: it is the
  right answer only if the per-particle payload does not fit existing fields, and it
  does fit (see Verified properties). A new region means a new binding, a new base
  constant, new caps, and changes to both hand-written emitters, for no new capability.
- **The per-particle affine in the draw entry, as a group's is.** plan-172-G gave every
  draw entry a composed matrix, so a particle could be drawn as "the template under an
  unnamed group node": one geometry record for the whole system instead of N. Not chosen
  for this letter, for two reasons. The draw entry carries no colour, so a tinted
  particle still needs its own item block; and one entry is one draw call, so N
  particles become N calls where composing into the item gives one instanced call over N
  blocks. It is letter B's first candidate if the measurement blames the N records, and
  §6 defines a shape particle so that the two give the same pixels.
- **Stateful simulation (integrate velocity each frame).** Rejected: errors compound,
  so software and GPU diverge over time and "render the same program twice and diff"
  — the method `tests/canvas/rt_canvas_metal.rs` is built on — stops meaning anything.
  Closed-form recomputation keeps every discrepancy sub-pixel and non-accumulating.
- **Continuous and burst emission simultaneously in one system.** Rejected: two
  overlapping birth schedules over one slot pool make `age(slot)` ambiguous. `mode` is
  an enum; layering two systems composes cleanly and keeps each schedule a single
  unambiguous formula.
- **A dedicated `Emitter` union (`EmitPoint` / `EmitLine` / `EmitRect` / `EmitCircle`).**
  This plan's first draft. Superseded by `canvas::Shape` (`planning/new_canvas.md`): the
  shapes already say "a line", "a box", "a disc" with a position and a transform, and a
  second closed union saying the same things would need its own freeze test and its own
  manual pages.

## 4. The type set

All registry data in `src/codegen/builtins/canvas/mod.rs`. This is
`planning/new_canvas.md`'s listing, unchanged.

```
canvas::ParticleSystem                    ' the 12th DrawItem variant — a CONTAINER, no paint
    position    AS vector::Float2         ' emitter reference point
    item        AS canvas::DrawItem       ' the template drawn once per live particle
    emitter     AS canvas::Shape
    emission    AS canvas::Emission
    motion      AS canvas::Motion
    appearance  AS canvas::Appearance
    seed        AS Integer                ' fixes every per-particle random draw
    time        AS Float                  ' the program's clock in seconds; the program advances it

canvas::EmissionMode                      ' enum
    Continuous | Burst

canvas::Emission
    mode          AS canvas::EmissionMode
    maxParticles  AS Integer   ' the slot pool; the hard bound on this system's quads
    rate          AS Float     ' Continuous: particles per second
    burst         AS Integer   ' Burst: particles per burst, clamped to maxParticles
    burstPeriod   AS Float     ' Burst: seconds between bursts; 0 = a single burst at time 0
    lifetime      AS Float     ' seconds a particle lives
    lifetimeVar   AS Float     ' ± fraction of lifetime, 0..1

canvas::Motion
    speed         AS Float     ' initial speed, px/s
    speedVar      AS Float     ' ± fraction, 0..1
    angle         AS Float     ' initial direction in radians; 0 = +X, matching Arc
    spread        AS Float     ' ± radians around angle; PI = omnidirectional
    gravityX      AS Float     ' px/s²
    gravityY      AS Float     ' px/s² — positive is DOWN, matching canvas' top-left origin
    drag          AS Float     ' exponential damping, 1/s; 0 = none
    rotation      AS Float     ' initial rotation, radians
    rotationVar   AS Float     ' ± radians
    spin          AS Float     ' radians/s
    spinVar       AS Float     ' ± fraction
    turbulence    AS Float     ' px amplitude of the wander
    turbulenceHz  AS Float     ' cycles/sec of the wander

canvas::Appearance
    startColor    AS color::Color   ' multiplied into the template's fill AND stroke
    endColor      AS color::Color
    startScale    AS Float
    endScale      AS Float
    scaleVar      AS Float     ' ± fraction, applied to both endpoints per particle
```

**Where it goes in the unions.** `ParticleSystem` is appended to `DrawItem`'s own
`variants`, after `Picture` and `Group`. It is **not** a `Shape` or an `AutoShape`: an
included union's members come first, so adding it there would renumber `AutoPaint`,
`Picture` and `Group`, and it would also make a `ParticleSystem` something an `AutoPaint`
could wrap and a `Picture` could be cut to. Appended last it takes the next tag (11) and
moves no other. Nothing may depend on that number: the native walks ask the type model
for a tag by name (`func_geo_build.rs:variant`).

**No `id`, no `transform`.** Every other variant leads with `id_prop()`. A
`ParticleSystem` has none because nothing about it is ever hit (§6), so an `id` would be
a field `overID` could never report. It has no `transform` because `new_canvas.md` gives
it none: the emitter shape carries its own.

`ParticleSystem` carries **no `paint`**, exactly as `Group` does not: the template
carries it. It joins `Group` in `CONTAINERS` in
`every_draw_item_variant_carries_a_paint`. It is not a `DECORATORS` entry: that list is
for a variant with a field named `shape`, and it checks that field's union; a
`ParticleSystem` holds an `item`, which may itself be a container.

**The `enum` zero value.** `EmissionMode`'s first variant is `Continuous`, which is what
a `ParticleSystem[...]` built without naming `mode` gets (plan-172-N: an enum field
defaults to its first variant). The order is therefore part of the surface.

**No constructor functions.** The owner's answer to plan-172-A Open Decision 1 was no
`canvas::rectangle(...)`-style constructors for the shapes; the same holds here. A
`ParticleSystem` is built by name and refined with `WITH`.

## 5. The analytic model

One MFBASIC function per stage, in a new `src/codegen/builtins/canvas/helper_particles.rs`.
Every quantity below is a closed form in `(slot, time)`; nothing integrates, and nothing
is remembered between frames.

**Per-slot randoms.** `__canvas_particleHash(seed, slot, cycle, stream)` returns a
`Float` in `[0,1)`. It reuses the integer mixing already proved in this module
(`helper_geometry.rs:__canvas_hashFloat`). `stream` separates the independent draws
so that changing `speedVar` cannot move a particle's spawn point. Draws used:
`r0` lifetime, `r1`/`r2` emitter sample, `r3` direction, `r4` speed, `r5` rotation,
`r6` spin, `r7` scale, `r8`/`r9` turbulence phase. A `Polygon` emitter's retries (below)
take streams from 16 up, so they cannot collide with these.

**Birth schedule and age.**

```
Continuous:  period P   = maxParticles / rate          (rate > 0)
             birth_i    = i / rate
             cycle      = floor((time - birth_i) / P)
             age        = (time - birth_i) - cycle * P

Burst, repeating (burstPeriod > 0):
             cycle      = floor(time / burstPeriod)
             age        = time - cycle * burstPeriod
             slot live only if i < burst

Burst, one-shot (burstPeriod = 0):
             cycle      = 0
             age        = time
             slot live only if i < burst
```

`cycle` feeds the hash, so a slot re-spawning gets fresh randoms rather than repeating
its previous life.

**Life.** `life_i = lifetime * (1 + lifetimeVar * (2*r0 - 1))`. The slot is **dead —
and contributes no draw entry and no quad — if `age < 0` or `age >= life_i`.**
Normalized age `u = age / life_i`, in `[0,1)`.

**Spawn point.** The emitter is a `canvas::Shape`. A sample `e` is drawn in the
emitter's **local geometry** — the space plan-172 gives every shape, with its reference
point at `(0, 0)` — and carried to the surface the way a point of that shape would be:

```
p0 = ps.position + emitter.position + emitter.transform(e)
```

`emitter.transform` is applied when `__canvas_transformActive` says it is in effect, and
is the identity otherwise; its `tx`/`ty` add, as they do when the shape is drawn. So an
emitter is turned, stretched and offset with the fields a user already knows, and
`ps.position` moves the whole system. The emitter's `id` and `paint` are never read: it
is never drawn and never hit, and its `paint.clip` does not cut the emitting region.

A shape with an interior emits **uniformly over its area**; a shape drawn from its
stroke alone emits **uniformly along its length**. That is the same split hit-testing
makes (`planning/new_canvas.md`, "Hit-testing").

| Emitter | The local sample `e` |
|---|---|
| `Circle` | `r = radius * sqrt(r1)`, `φ = 2π*r2` → `(r*cos φ, r*sin φ)`. `radius := 0.0` is a point emitter |
| `Ellipse` | the unit-disc sample above, scaled: `(radiusX*sqrt(r1)*cos φ, radiusY*sqrt(r1)*sin φ)` |
| `Rectangle` | `bind = TopLeft`: `(r1*size.width, r2*size.height)`; `bind = Center`: the same, less half the size. The corner radii are ignored (Open Decisions) |
| `Polygon` | uniform over the interior under `rule`, by bounded rejection: up to 32 candidates drawn uniformly in the contours' bounding box, the first that the fill test accepts (`helper_draw.rs:__canvas_edgeDistanceRule`'s crossing rule). If none is accepted the slot is dead for that cycle |
| `Line` | `lerp(startAt, endAt, r1)` |
| `Arc` | `θ = lerp(startAngle, endAngle, r1)` → `(radius*cos θ, radius*sin θ)`. A full turn is a ring emitter |
| `PolyLine` | the point at distance `r1 * L` along `points`, `L` their total length. `colors` is ignored |
| `Text` | uniform over the layout box — advance width by ascent plus descent — the box hit-testing uses (Open Decisions) |

The `sqrt` in the disc sample is what makes it *uniform by area*; sampling `r` linearly
clusters particles at the centre and is the classic mistake here. An affine map keeps a
uniform area density uniform, so `Ellipse` and any transformed area emitter stay uniform.
A stroke emitter under a non-uniform scale is uniform in its local space, not along its
drawn length; that is accepted and documented.

The rejection loop is bounded and deterministic: its candidates come from the hash, not
from a clock. It does mean a `Polygon` emitter costs up to 32 fill tests per newborn
slot per frame, which letter B measures.

**Velocity.** `θ = angle + spread*(2*r3 - 1)`, `s = speed * (1 + speedVar*(2*r4 - 1))`,
`v0 = (s*cos θ, s*sin θ)`. Velocity is in surface axes: the emitter's transform turns
where particles are born, not which way they fly.

**Position.** With `g = (gravityX, gravityY)`, `k = drag`, `t = age`:

```
k = 0:  p = p0 + v0*t + 0.5*g*t²
k > 0:  p = p0 + (v0 - g/k) * (1 - e^(-k*t)) / k  +  (g/k)*t
```

The second is the exact solution of linear drag, not an approximation, so the two agree
as `k → 0`.

**Turbulence**, added to `p`:

```
p.x += turbulence * sin(2π*turbulenceHz*t + 2π*r8)
p.y += turbulence * sin(2π*turbulenceHz*t + 2π*r9)
```

Two independent phases rather than a noise field, deliberately: a value-noise function
would eventually need to agree across MFBASIC, MSL and GLSL if any later letter moves
the model into a shader, and two sines have no table to keep in step.

**Rotation.** `θr = rotation + rotationVar*(2*r5 - 1) + spin*(1 + spinVar*(2*r6 - 1))*t`.

**Scale.** `sc = lerp(startScale, endScale, u) * (1 + scaleVar*(2*r7 - 1))`.

**Colour.** `c = lerp(startColor, endColor, u)`, interpolated in **linear** space via
`color::toLinear` / `color::fromLinear`, because that is the space the rasteriser and
both shaders composite in. Interpolating in sRGB would make a red→transparent fade pass
through a visibly wrong midpoint.

## 6. The expansion

A `CASE ParticleSystem(ps)` arm in `helper_render.rs:__canvas_appendDraw`, structurally
the same as `CASE Group(g)`. The signature is the one plan-172 left:
`(offsets, item, hash, gdx, gdy, mat, depth, path, index)`.

```
CASE ParticleSystem(ps)
  IF depth >= __CANVAS_GROUP_MAX_DEPTH THEN RETURN out      ' silent, as Group's is
  FOR i = 0 TO ps.emission.maxParticles - 1
    LET st AS ParticleState = __canvas_particleState(ps, i)
    IF st.alive THEN
      LET child AS DrawItem = __canvas_particleItem(ps.item, st)
      out = __canvas_appendDraw(out, child, __canvas_hashItem(child), gdx, gdy, mat, depth + 1, path, index)
    END IF
  NEXT
  RETURN out
END CASE
```

The enclosing groups' offset and matrix (`gdx`, `gdy`, `mat`) pass straight through, so
a system inside a turned or scaled group is turned and scaled with it, like any item.

### What a particle is

**A particle is the template drawn as if it sat alone in a group node at the particle
point `P` with the transform `M = R(θr) · S(sc)`.** That is the definition; it uses the
one composition rule plan-172 has (`world = P + M(local)`), and it says what the
template's own `position` and `transform` mean: they are offsets inside the particle,
turned and scaled with it. A template with `position := (0, 0)` sits on the particle
point.

`__canvas_particleItem(template, st)` flattens that node into the template's own fields,
so the result is one ordinary item and one item block. With the template's linear part
`A` and translation `t` (the identity and zero when its transform is not in effect):

| Template | What is rewritten |
|---|---|
| any of the eight shapes | `position := P`; `transform := [ M·A  \|  M·(template.position + t) ]`; `paint.fill` and `paint.stroke` each multiplied by `st.color`, per channel including alpha |
| `AutoPaint` | its `shape` is taken as a plain shape and rewritten as above. Its default paint is the one drawn; the four state paints are never read and no instance is noted |
| `Picture` | its `shape` is rewritten as above (an `AutoPaint` in it unwrapped first). The tint lands in the shape's `paint.fill`, which is what tints the image. `image`, `clip` and `fit` are carried through |
| `Group` | the node becomes `Group[position := P, transform := [ M·A | M·(g.position + t) ], name := g.name]` and goes through `CASE Group` as any node does. **It is not tinted** (Open Decisions) |
| `ParticleSystem` | nothing is drawn. `present` has already raised (§7); this arm is the silent stop |

`blend`, `clip`, `clipRule`, `strokeWidth`, `gradient`, the stroke styling and
`transparency` are carried through unchanged. A white, opaque `startColor`/`endColor`
leaves the template's own colours untouched.

Three consequences, each a rule the renderer already has rather than one this letter
invents:

- **A particle with no area is dead.** If `|det(M·A)| < 1e-12` — `sc` is zero, or the
  template's own transform has collapsed — the slot contributes no entry. Without this
  the shape rule would treat the transform as the identity and draw the particle at
  full size at the moment it was meant to vanish. A `Group` template needs no special
  case: `__canvas_composeGroup` already answers "draws nothing" for a node with no area.
- **A stroke is scaled with its particle**, because a stroke is transformed with its
  shape. A particle at `sc = 2` draws a 4-pixel outline 8 pixels wide.
- **A `Picture` with `clip := ClipOnly` keeps its image upright.** That mode fits the
  image to the transformed shape's bounding box and draws it unturned; only the outline
  turns. A sprite that should spin is a `ClipAndTransform` picture. This is the one
  place the flattened item differs from a real group node, which would turn the image in
  both modes (Open Decisions).

### What a particle is not

- **Never hit.** `helper_hit.rs:__canvas_hitAppend` gets a `CASE ParticleSystem` arm
  that appends nothing, so a system contributes no hit entry on the worker, adds nothing
  to `MouseEvent.overID`, and blocks nothing beneath it. The worker never runs the model.
- **Never an `AutoPaint` instance.** An `AutoPaint` template is unwrapped before the
  recursion, so `__canvas_autoAppend` is not reached. An `AutoPaint` *inside a `Group`
  template* would be reached by the group walk, once per particle, all at the same
  `(path, index)` place: Phase 4 gives the walk a way to say "no instances below here"
  and tests that such an `AutoPaint` draws its default paint.

### The other walks

Every walk over a scene must give the variant an answer. The ones known today, each a
Phase 4 task:

| Walk | What a `ParticleSystem` is to it |
|---|---|
| `__canvas_appendDraw` (software, damage) | the expansion above |
| `__canvas_sceneOffsets` | an index of N entries: the `several` path, as a two-entry `Picture` |
| `__canvas_sceneDrawsWalk` / `__canvas_memoGroup` / `__canvas_drawGroup` (GPU) | at the top level, the frame's entries; as a group's child, N blocks in that group's run; with a `Group` template, one `__canvas_drawGroup` per live particle at its composed affine |
| `__canvas_hitAppend` (worker) | nothing |
| `__canvas_groupSignature` (worker, in `present`) | recurse into `ps.item`, so a `setGroup` under a template group's name is seen; and the two raises (§7) |
| `__canvas_hashItem` | folds every field, the template's hash and `time` included |
| `__canvas_geometryFor` / `__canvas_geoReference` | never reached with one: it is expanded first. The arm builds the empty header, as a `Group`'s does |
| `canvas::sceneResolve` / `canvas::sceneDrawsFlat` (native) | left to the MFBASIC walk; a frame holding one is not flat |
| `func_item_hash.rs`, `func_carried_hashes.rs` (native) | hashed structurally through `item`; never carried when the template names a font or an image, as `Text` and `Picture` are never carried |
| the resource walks — who names an image or a font (`canvas::setBytes`'s trigger 5, a group's ownership of what its items name) | a `Picture` or `Text` template names its resource through `item` |

The depth guard is inherited from `Group` and shares its constant: a `ParticleSystem`
whose template is a `Group` is bounded by the same 64.

Because the software arm recurses through `__canvas_appendDraw`, every particle lands in
`__CANVAS_DRAW_HASHES` / `_DX` / `_DY` / `_MAT` the way any other entry does — so damage,
the geometry cache, both GPU emitters and the software walk all see ordinary items.

## 7. Validation and raises

Raised on the **worker**, inside `canvas::present` and `canvas::presentLayers`, in the
walk that already raises the group depth (`__canvas_groupSignature`, called from
`func_present.rs:__canvas_present`) — never on the graphics thread.

New error codes continue from the highest in use. On 2026-10-11 that is 77050027, so
the two below are **77050028 and 77050029; re-measure before taking them** (the command
is in "Measured populations"). Each is a named constant in
`src/codegen/builtins/errorcode/mod.rs` and a row in
`src/docs/spec/diagnostics/02_error-codes.md`, as `ErrCanvasGroupLimit` is.

| Condition | Behaviour |
|---|---|
| `maxParticles <= 0` | draws nothing; **not** an error |
| `rate <= 0` with `mode = Continuous` | draws nothing; not an error |
| `burst <= 0` with `mode = Burst` | draws nothing; not an error |
| `lifetime <= 0` | `FAIL error(77050028, …)` — no closed form for `u` exists |
| `item` is a `ParticleSystem` | `FAIL error(77050029, …)` — a system of systems multiplies quads without bound |
| `lifetimeVar`, `speedVar`, `spinVar`, `scaleVar` outside `[0,1]` | clamped; not an error |
| `time` negative | draws nothing; not an error |
| a scale of zero, or a template transform with no area | that particle draws nothing; not an error (§6) |

`burst > maxParticles` is clamped to `maxParticles`, not an error: the pool is the
bound and clamping is the honest reading of "release this many from this pool".

The walk looks only at the template the scene holds. A `ParticleSystem` stored inside a
named group that a template names is checked when that group's items are walked, by the
same recursion.

## Compatibility / Format Impact

- **Added, externally observable:** one `DrawItem` variant, four records, one enum, two
  error codes. `mfb man canvas types` grows accordingly (letter C).
- **Unchanged:** `ITEM_BLOCK_SIZE` (368), the `ItemBlock` layout, both frame block caps
  (65,536), the draw-entry width (16 words), both shaders, both `.spv` blobs, the
  geometry header layout, `Paint`, `Transform`, the `Shape` and `AutoShape` unions, and
  every existing variant's record and tag.
- `scripts/regen-spirv.sh` is **not** run by this plan; if a diff appears in either
  `.spv`, that is a bug in this letter.
- Existing programs: no source change, and `.ncode` expected byte-identical. A program
  with a `MATCH` over `DrawItem` and no `CASE ELSE` stops being exhaustive — the one
  source-visible effect, and the reason the set is frozen by a test.
- `tests/syntax/app/app-mouse-surface/golden/*` holds the union and type surface and is
  **expected** to change by the added types and variant only.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit as
> the work it describes. Use `- [~]` for partially done plus one line on what remains.
> Mark a task moot with `- [x] ~~text~~ — moot: <evidence>`. Fill each `Commit:` line
> the moment the phase lands. **An unticked box means NOT DONE.**

### Phase 1 — Falsify the recursive type (uncertainty first)

The one premise that would reshape the whole design. Nothing else is built until it
holds. Deliberately the smallest possible version: a one-field record, no model, no
expansion.

- [ ] Declare a minimal `ParticleSystem` record in `src/codegen/builtins/canvas/mod.rs`
      with a single prop `item AS canvas::DrawItem`, and append it to `DrawItem`'s own
      `variants`, after `Group`.
- [ ] Amend `draw_item_variant_set_is_frozen` to the twelve-name list, **appending**
      `ParticleSystem` last so no existing variant's tag moves. Keep the assertion's
      warning message; record the amendment in the doc comment beside the plan-116 and
      plan-172-K entries, per the rule that the list grows by one visible entry and
      never gets laxer. `shape_variant_set_is_frozen` and
      `auto_shape_variant_set_is_frozen` must pass **unedited**.
- [ ] Add `ParticleSystem` to `CONTAINERS` in `every_draw_item_variant_carries_a_paint`,
      beside `Group`.
- [ ] Give every MFBASIC `MATCH` over `DrawItem` that has no `CASE ELSE` the arm it
      needs to keep compiling, drawing nothing. The compiler's exhaustiveness check
      finds them; the list in §6 is what they should number.
- [ ] Confirm, or teach, the two native folds to walk a `DrawItem` inside a variant
      record: `func_item_hash.rs` and `func_carried_hashes.rs`. Two systems differing
      only in a field of their template must hash differently.
- [ ] Write a runtime probe that presents a scene containing
      `ParticleSystem[item := Circle[…]]` and survives `present`'s deep copy.
- [ ] Tests: `tests/canvas/rt_canvas_particles.rs`, new — the probe above, and the
      differing-template hash case.

Acceptance: a scene holding a `ParticleSystem` whose `item` is a `Circle` is presented,
deep-copied and dropped without a leak, a double free, or a SIGSEGV — i.e. the type
cycle survives the graph-copy path `.ai/collections.md` describes — and re-presenting it
with a changed template is not skipped as an identical scene.
  Check: `cargo test --test rt_canvas_particles` → pass (est. 3 min, guess), and
  `cargo test --bin mfb canvas` → pass (est. 3 min, guess).
Commit: —

### Phase 2 — The full type set

Registry data only; still no behaviour, so nothing can regress.

- [ ] Expand `ParticleSystem` to all eight props (§4), in `new_canvas.md`'s order.
- [ ] Add records `Emission`, `Motion`, `Appearance`, and the `EmissionMode` enum with
      `Continuous` first.
- [ ] Write per-field descriptions to the standard the existing records set
      (`.ai/man-content.md`) — every unit (px, px/s, px/s², radians, seconds) named in
      the text, and what an emitter of each shape emits from.
- [ ] Tests: extend `tests/canvas/rt_canvas_particles.rs` to construct every record, and
      a `ParticleSystem` with an emitter of each of the eight shapes.

Acceptance: `mfb man canvas types` lists `ParticleSystem`, `Emission`, `Motion`,
`Appearance` and `EmissionMode` with every field documented, and a program constructing
a system with each of the eight emitter shapes compiles and runs.
  Check: `cargo test --bin mfb canvas` → pass, and
  `cargo run -- man canvas types | grep -c 'ParticleSystem\|Emission\|Motion\|Appearance'`
  → ≥ 5 (est. 5 min, guess).
Commit: —

### Phase 3 — The analytic model, as a pure function

Testable in isolation, before anything draws. The whole model is a function of its
arguments, so this phase is where its correctness is actually established.

- [ ] New `src/codegen/builtins/canvas/helper_particles.rs`: `__canvas_particleHash`,
      `__canvas_particleAge`, `__canvas_particleSpawn`, `__canvas_particlePosition`,
      `__canvas_particleState`, per §5.
- [ ] Register the helper in `src/codegen/builtins/canvas/mod.rs` alongside the other
      `helper_*` registrations, gated on the program constructing a `ParticleSystem`
      the way `helper_autopaint.rs` is gated on an `AutoPaint`
      (`.ai/canvas-threading.md` §16, "The code is gated on the program constructing
      one"), so a program without one carries none of it.
- [ ] Unit-test the module the way the sibling helpers are tested, in
      `src/codegen/builtins/canvas/helper_particles/tests/mod.rs`.
- [ ] Tests, each a named case in `tests/canvas/rt_canvas_particles.rs`:
      **determinism** (same `(seed, time)` → identical state, twice in one run);
      **liveness** (a slot is dead before birth and after `lifetime`);
      **drag limit** (`drag = 1e-6` agrees with `drag = 0` to 1e-4 px over 1 s);
      **disc uniformity** (10,000 `Circle`-emitter samples: the inner-half-radius disc
      holds within 2% of a quarter of them — the check that catches a missing `sqrt`);
      **emitter placement** (a `Rectangle` emitter with `bind := Center`, a quarter-turn
      `transform` and a non-zero `position` puts every sample inside the turned box, and
      some outside the unturned one);
      **stroke emitters** (every `Line`, `Arc` and `PolyLine` sample lies on its curve
      to 1e-9 px);
      **polygon emitter** (an L-shaped `Polygon`: no sample falls in the notch);
      **burst schedule** (with `burstPeriod = 2.0`, slots are alive at `t = 0.1` and
      `t = 2.1` and dead at `t = 1.9` for `lifetime = 1.0`).

Acceptance: every case above passes, and the disc-uniformity case **fails** if the
`sqrt` is removed from the `Circle` sample — verified once by removing it, observing the
failure, and restoring it.
  Check: `cargo test --test rt_canvas_particles` → pass (est. 5 min, guess).
Commit: —

### Phase 4 — Expansion and rendering (largest blast radius)

Touches the draw walk every scene goes through, so it lands last and behind Phase 3's
tests.

- [ ] Add the `CASE ParticleSystem(ps)` arm to `__canvas_appendDraw` per §6, including
      the silent depth guard.
- [ ] Add `__canvas_particleItem`: the flattened affine and the tint for each template
      kind in §6's table, and the no-area rule.
- [ ] Measure the `several` path in `__canvas_sceneOffsets` with one index of 1,000
      entries, software and GPU. Fix what it finds; record the result in Corrections.
- [ ] The GPU walk: a system at the top level, a system as a group's child
      (`__canvas_memoGroup`), and a system whose template is a `Group`
      (`__canvas_drawGroup` per live particle).
- [ ] `__canvas_hitAppend`: the arm that appends nothing.
- [ ] `AutoPaint` inside a `Group` template: no instance is noted and the default paint
      is drawn.
- [ ] `__canvas_groupSignature`: recurse into `ps.item`; add the two raises per §7, with
      their `errorcode` constants and spec rows. Confirm `presentLayers` takes the same
      walk.
- [ ] The resource walks: a `Picture` or `Text` template's image or font is seen through
      `item` wherever a scene's resources are collected. Find the sites with
      `grep -n 'CASE Picture(\|CASE Text(' src/codegen/builtins/canvas/*.rs`, and read
      `func_set_bytes.rs` and `gen_group.rs`; list them in Corrections.
- [ ] Confirm the consumers of the per-entry arrays need no change, because a particle
      is an ordinary entry (`grep -n '__CANVAS_DRAW_DX\|__CANVAS_DRAW_MAT'
      src/codegen/builtins/canvas/*.rs` — `helper_render.rs`, `helper_damage.rs`,
      `helper_hit.rs`, `helper_autopaint.rs`, `func_geo_cache.rs`) — and say so in a
      comment at the expansion site so the next reader does not re-derive it.
- [ ] Tests: a software golden `tests/golden/canvas/particles.png` via the existing
      `render()` harness in `tests/canvas/rt_canvas_golden.rs`, from a fixed
      `(seed, time)` fireworks scene — a `Burst` system with a `Circle[radius := 0.0]`
      emitter, gravity, a colour ramp and a `Circle` template.
- [ ] Tests, in `rt_canvas_particles.rs`: a particle equals its definition — a system of
      one live particle renders pixel-identically to the same template inside a `Group`
      at `(P, M)`; a scale that reaches zero draws nothing rather than a full-size
      shape; a system inside a turned group; a `Picture` template; a `Group` template; a
      `Text` template.
- [ ] Tests, in `tests/canvas/rt_canvas_hit.rs`: a system drawn over a named rectangle
      leaves `overID` naming the rectangle.
- [ ] Tests: a negative case per raise, matching how the group-depth raise is tested.

Acceptance: the fireworks golden renders identically on two consecutive runs; one live
particle is pixel-identical to its group-node definition; a template that is itself a
`ParticleSystem` raises `77050029` from `present` on the worker — not on the graphics
thread; and a system blocks no hit.
  Check: `cargo test --test rt_canvas_golden --test rt_canvas_particles --test
  rt_canvas_hit` → pass, and run the golden twice comparing with `compare_exact`
  (est. 8 min, guess).
Commit: —

### Phase 5 — Prove the GPU backends need no change

The design's central claim, measured rather than asserted. If it fails, the failure is
a bug in Phase 4's composition to root-cause — not evidence the design is dead.

- [ ] Add a Metal parity case to `tests/canvas/rt_canvas_metal.rs` rendering the
      Phase 4 fireworks scene twice — `MFB_CANVAS_GPU=1` and software — and diffing
      within `Tolerance::GPU_DEFAULT`, the pattern the file already uses. Add a second
      row with the system inside a turned, scaled group.
- [ ] Add the fireworks scene to `tests/canvas/scenes/`, so
      `scripts/test-canvas-gpu-rows.sh` carries it to the Linux and Windows lanes
      letter C runs.
- [ ] Confirm `git status` shows **no** diff in `src/codegen/runtime/canvas/shaders/`
      — neither `.frag`/`.vert` nor either `.spv` — and none in the MSL
      (`src/target/macos_aarch64/app/metal.rs:METAL_SHADER_BODY`).
- [ ] Record in the plan's Corrections section whether the no-shader-change claim held.

Acceptance: the Metal parity cases pass within `Tolerance::GPU_DEFAULT` on a host
reporting a Metal device, skip cleanly on one that does not, and no shader source or
SPIR-V blob has changed.
  Check: `cargo test --test rt_canvas_metal` → pass or documented skip, and
  `git status --porcelain src/codegen/runtime/canvas/shaders/` → empty (est. 5 min,
  guess).
Commit: —

## Validation Plan

- **Tests:** `tests/canvas/rt_canvas_particles.rs` (new — determinism, liveness, drag
  limit, the emitter cases, burst schedule, the group-node equivalence, both raises); a
  hit case in `tests/canvas/rt_canvas_hit.rs`; a new golden
  `tests/golden/canvas/particles.png`; Metal parity cases in
  `tests/canvas/rt_canvas_metal.rs`; registry tests in
  `src/codegen/builtins/canvas/mod.rs` (`draw_item_variant_set_is_frozen` and
  `every_draw_item_variant_carries_a_paint` amended; the two `Shape` freeze tests
  untouched); helper unit tests in `helper_particles/tests/mod.rs`.
- **Coverage check:** `scripts/coverage-check.sh` — confirm `helper_particles.rs` is in
  the denominator. A green gate means "nothing covered changed"; a new file absent from
  coverage would make that claim vacuous for exactly the code this letter adds.
- **Runtime proof:** a program presenting a `Burst` system with a `Picture` template,
  `time` advanced over 60 frames, read through `canvas::getFrameStats` (and
  `MFB_CANVAS_STATS`) — GPU frames on a Metal host, and a block count that tracks the
  live particle count rather than `maxParticles`.
- **Doc sync:** `mfb man canvas` gains the new types (letter C does the prose);
  `src/docs/spec/app/06_canvas.md` gains the variant in "The item unions" and a
  "Particle systems" section in the same change as Phase 4 (the Hard Completion Gate in
  `.ai/specifications.md`; letter C reviews it); `02_error-codes.md` gains the two
  codes; `.ai/canvas-threading.md` §4 gains a sentence stating that a particle system is
  advanced by the program re-presenting with a new `time`, and that this is trigger 1
  and **not** an eighth trigger.
- **Final gate (run ONCE, after Phase 5):** `scripts/test-accept.sh <mfb-exe> <actual-output-dir>` (both arguments are required; the script exits 2 without them) (est. per its own
  runtime). Per-phase checks stay scoped; do not repeat this per phase.

## Open Decisions

Each has a recommendation, and the plan above is written to it. They are the places
where plan-172 opened a choice that `planning/new_canvas.md` does not settle.

1. **Where the per-particle affine lives** — composed into the template item's own
   `position` and `transform` (recommended, §6), or carried in the draw entry as an
   unnamed group node's matrix. Composing gives one instanced draw call over N blocks
   and needs no GPU-walk mechanism that does not already exist; the draw-entry route
   gives one geometry record for the whole system but one draw call per particle, and
   still needs an item block per tinted particle. Letter B measures the first and may
   argue for the second.
2. **A `Picture` template with `clip := ClipOnly`** — its image stays upright while the
   particle turns (recommended: accept it, and say so in the manual), or the expansion
   draws every `Picture` particle as `ClipAndTransform`. The first is the mode meaning
   what it means everywhere else; the second quietly overrides a field the user set.
3. **A `Group` template's tint** — not tinted (recommended), or a tint carried down the
   group walk. A group's blocks are recorded once and shared by every use of the group,
   so a per-particle tint cannot be written into them; carrying it would mean a block
   per particle per item and a new argument on every walk.
4. **A `Rectangle` emitter's corner radii** — ignored (recommended; the box is the
   emitter), or honoured by the rejection loop `Polygon` uses.
5. **A `Polygon` emitter** — bounded rejection in the bounding box, 32 tries
   (recommended), or emit along its outline only. Rejection is uniform over the interior
   under `rule`, holes included, at a per-slot cost letter B measures.
6. **A `Text` emitter** — its layout box (recommended; the box hit-testing uses), or a
   raise. It costs a font measure per frame on the graphics thread.
7. **The two raises' codes** — two new codes with named constants (recommended, and what
   this plan always specified), or `ErrInvalidArgument` (`77050002`) for both.
8. **A template that fills with a gradient** — the tint does not reach the ramp, so such
   a particle does not change colour or fade (recommended: document it; `startColor`
   and `endColor` tint flat fills, strokes and pictures). The alternative is to put the
   tint's alpha in `paint.transparency`, which fades every template alike but is a
   second tint rule.
9. **`turbulence` as two sines vs. a shared value-noise function** — two sines
   recommended (§5). Noise looks better for fog specifically; it becomes worth its cost
   only if a later letter moves the model into shaders, where a table would then have to
   agree across three languages.
10. **Whether `Text` should be allowed as a template** — allowed as specified. One text
    particle is one quad per glyph against the 65,536-block frame cap. Letter B measures
    it; if the cliff is sharp, a documented warning in `mfb man canvas` may be the right
    answer rather than a restriction.

Settled, and no longer open: **the rotation origin** is the template's reference point,
by plan-172's transform rule.

## Corrections

1. **`DrawItem` has thirteen variants and `Shape` ten, not eleven and eight**
   (2026-10-13, `cargo test --bin mfb draw_item_variant_set_is_frozen` at `29f5500cb`;
   the list is `Rectangle`, `Line`, `Polygon`, `Circle`, `Arc`, `Text`, `Ellipse`,
   `PolyLine`, `PolyCurve`, `CurvePolygon`, `AutoPaint`, `Picture`, `Group`). plan-164-A
   landed between this plan's re-derivation and its execution and appended `PolyCurve`
   and `CurvePolygon` to `Shape`. Every "twelfth variant" / "tag 11" / "twelve-name list"
   / "eight shapes" above reads **fourteenth / tag 13 / fourteen-name list / ten shapes**.
   Re-scoped in place, not re-split:
   - §5's emitter table gains two rows: a `PolyCurve` emits uniformly along its length
     (a stroke emitter, like `PolyLine`), a `CurvePolygon` uniformly over its interior
     under `rule` (bounded rejection, like `Polygon`). Both are sampled on the outline
     flattened to straight edges, the reduction a `Picture` already cuts to
     (`.ai/canvas-threading.md` §17).
   - §6's template table: both curve shapes are "any of the shapes" — rewritten like
     the other eight.
   - Phase 2's "an emitter of each of the eight shapes" is each of the ten; Phase 3's
     stroke-emitter and polygon-emitter cases gain a curve twin each.
   - Letter C's "interior for a `Rectangle`, `Circle`, `Ellipse`, `Polygon` and `Text`,
     its length for a `Line`, `Arc` and `PolyLine`" gains `CurvePolygon` and `PolyCurve`.
2. **Other populations re-measured 2026-10-13 at `29f5500cb`**, same commands as
   "Measured populations": records 23 (was 20), unions 3, enums 13, registry functions
   98 (was 95), canvas runtime test files 22 (was 21), highest error code 77050027
   (unchanged, so the two raises are still 77050028 and 77050029).

## Summary

The engineering risk is concentrated in two places, and they are scheduled at opposite
ends. **Phase 1** falsifies the recursive-type premise — the only thing that would
reshape the design — for the cost of one record and one probe. **Phase 4** touches the
draw walk that every scene goes through, and lands last, behind the model's own tests.

Everything between them is additive registry data and a pure function.

What plan-172 changed is where the per-particle state goes — into each shape's own
`position` and `transform` rather than a `Paint.transform` that no longer exists — and
how many walks there are to teach: the hit walk, the `AutoPaint` instance table and the
GPU's group walk did not exist in the form this plan was first written against.

What this letter still deliberately leaves untouched is the entire GPU transport: no
shader, no pipeline, no binding, no buffer layout, no cap. That is the direct
consequence of particles being expressible in fields every shape already has, and Phase
5 exists to prove the claim rather than assume it.
