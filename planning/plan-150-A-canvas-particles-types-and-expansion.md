# plan-150-A: Particle systems — types, the analytic model, and expansion

Last updated: 2026-09-22
Overall Effort: x-large (1d–3d)
Effort: large (3h–1d)
Depends on: nothing

`canvas::ParticleSystem` becomes the **eleventh `DrawItem` variant**: one scene item
that expands into N drawn particles, each an instance of an arbitrary template
`DrawItem` — a `Picture` sprite, a `Circle`, a `Polygon`, anything the union already
holds — placed, rotated, scaled and tinted from a **closed-form** function of its slot
index and the system's clock.

```
canvas::ParticleSystem
    item       AS DrawItem     ' the template, drawn once per live particle
    emitter    AS Emitter      ' where particles are born
    emission   AS Emission     ' how many, how often, how long they live
    motion     AS Motion       ' how they move
    appearance AS Appearance   ' how they look across their life
    seed       AS Integer      ' fixes every per-particle random draw
    time       AS Float        ' the program's clock, in seconds
```

Behavioral outcome: a program presents a scene containing one `ParticleSystem` with
`time` advanced each frame, and sees a fire / fog / fireworks effect that is
**deterministic** — the same `(seed, time)` produces the same picture on every run, on
every backend, on every platform.

The design's load-bearing decision is **expansion, not a particle shader**. A
`ParticleSystem` is expanded on the graphics thread into N ordinary draw entries, the
way `Group` already expands (`helper_render.rs:157 __canvas_appendDraw`). Because the
per-particle state is expressed entirely in fields the item model *already has* —
`paint.transform` (a full affine) and `paint.fill`/`paint.stroke` (tint) — the
expansion emits ordinary items, and **both GPU backends and the software rasteriser
draw them with no change to any shader, pipeline, binding, or buffer layout.**

References:

- `.ai/canvas-threading.md` — §2 (arena state is per-thread), §3 (the scene ring),
  §4 (the five redraw triggers; **time is not one**). **Read §4 before touching this
  letter**: the system is advanced by the program re-presenting with a new `time`,
  never by a timer.
- `src/codegen/builtins/canvas/mod.rs` — the registry declarations and the three
  frozen-set tests this letter must amend.
- `src/codegen/builtins/canvas/helper_render.rs` — `__canvas_appendDraw`, the
  `Group` expansion this design mirrors.
- `src/codegen/runtime/canvas/mod.rs:328-439` — `ITEM_BLOCK_SIZE` and the
  `ITEM_OFFSET_*` layout, the evidence that no new transport is needed.
- `AGENTS.md` — the never-edit-a-test-to-pass rule. Three frozen-set tests are
  amended by this letter; each amendment is an addition, never a loosening.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| The `DrawItem` union has exactly ten variants | `cargo test -p mfb draw_item_variant_set_is_frozen` → pass | MET |
| `canvas::Transform` is a field of `Paint` | `grep -n 'ty: ParameterType::named("Transform")' src/codegen/builtins/canvas/mod.rs` → line 711 | MET |
| The canvas suite is green before any edit | `cargo test --test rt_canvas_golden --test rt_canvas_rasteriser` → pass | UNVERIFIED — run first |

Everything below is written against the world where these hold.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again
> before you decide to stop. If you stop, report the status of *all* prerequisites.

## 1. Goal

- `canvas::ParticleSystem` exists as the eleventh `DrawItem` variant and renders in
  software.
- The same `(seed, time)` renders byte-identically across runs — the system is a pure
  function of its fields, holding no state between frames.
- The template may be any of the nine drawable `DrawItem` variants, including
  `Picture` and `Text`.
- Both GPU backends render a particle scene within `Tolerance::GPU_DEFAULT` of the
  software oracle **without any shader, pipeline or buffer-layout change** — this is
  a consequence of the design, and Phase 5 is the measurement that proves it.

### Non-goals (explicit constraints)

- **No field advection / wind streamlines.** Path-dependent motion needs per-particle
  state carried across frames; every quantity in this letter is closed-form in
  `(slot, time)`. Out of scope, deliberately, and not a later phase of this plan.
- **No user-supplied shaders.** The `DrawItem` set stays closed; nothing in this plan
  compiles user source.
- **No change to `ITEM_BLOCK_SIZE` (208), `CANVAS_MAX_FRAME_ITEMS` (4096), the
  `ItemBlock` layout, the draw-entry width (8 words), or either shader.** A particle's
  quads count against the existing frame cap exactly as any other item's do. If a
  system exceeds it, the frame declines to software — correct, and slow, and measured
  in letter B rather than pre-empted by a guessed new constant.
- **No new redraw trigger.** `time` is a field the program sets; advancing it changes
  the item's hash and fires trigger 1 (`present`). A timer-driven repaint would
  violate `.ai/canvas-threading.md` §4.
- **No GPU-side simulation and no persistent GPU state.** Every frame is computed from
  `(seed, time)` alone.
- **`Paint`, `Transform`, `BlendMode`, `CapStyle` and the existing nine variants'
  records are unchanged.** This letter only *adds*.

## 2. Current State

A scene is a flat `List OF DrawItem` deep-copied by `present`
(`tests/canvas/rt_canvas_present_deep_copy.rs:148`). On the graphics thread,
`__canvas_sceneOffsets` (`helper_render.rs:552`) walks the installed items and calls
`__canvas_appendDraw` (`helper_render.rs:157`) per item, which resolves each item's
geometry through the cache and appends one offset per drawn entry.

**`Group` is the precedent this design mirrors.** It is the one existing variant that
draws nothing itself and expands into N entries: `__canvas_appendDraw`'s `CASE Group(g)`
arm recurses over `canvas::groupItems(...)`, accumulating a translation `(gdx, gdy)`
into the parallel arrays `__CANVAS_DRAW_DX`/`__CANVAS_DRAW_DY` and folding it into the
recorded hash. It is bounded by `__CANVAS_GROUP_MAX_DEPTH = 64` (`helper_render.rs:154`),
and the depth *raise* happens on the worker inside `present` — never on the graphics
thread, where a `FAIL` has no user frame to unwind to (`helper_render.rs:231`).

`Group` is also the precedent for a **container variant**: it carries no `paint`, and
the test `every_draw_item_variant_carries_a_paint` names its container exemptions
explicitly rather than dropping the check.

### Measured populations

| What | Count | Command |
|---|---|---|
| `DrawItem` variants today | 10 | `grep -c 'UnionVariant {' src/codegen/builtins/canvas/mod.rs` (within the `DrawItem` union) / `draw_item_variant_set_is_frozen` pins the list |
| Records in the `canvas` registry | 20 | `grep -c 'pkg.add_record(RegistryRecord {' src/codegen/builtins/canvas/mod.rs` → 20 |
| Unions in the `canvas` registry | 1 | `grep -c 'pkg.add_union(RegistryUnion {' src/codegen/builtins/canvas/mod.rs` → 1 |
| Enums in the `canvas` registry | 5 | `grep -c 'pkg.add_enum(' src/codegen/builtins/canvas/mod.rs` → 5 |
| Registry functions in `canvas` | 62 | `grep -c 'pkg.add_function' src/codegen/builtins/canvas/*.rs \| awk -F: '{s+=$2} END {print s}'` → 62 |
| Canvas runtime test files | 12 | `ls tests/canvas/*.rs \| wc -l` → 12 |
| `Paint` fields | 7 | `sed -n '672,760p' src/codegen/builtins/canvas/mod.rs \| grep -c 'name:'` → 8 incl. the record name → 7 props (fill, stroke, strokeWidth, blend, transform, clip, fillGradient) |
| Consumers of the per-entry offset arrays | 3 | `grep -n '__CANVAS_DRAW_DX' src/codegen/builtins/canvas/*.rs` → `helper_render.rs:69` (software walk), `helper_damage.rs:129`, `helper_damage.rs:243` |
| Geometry cache capacity | 256 entries | `grep -n 'GEO_CAPACITY' src/codegen/builtins/canvas/helper_geometry.rs` → `LET __CANVAS_GEO_CAPACITY AS Integer = 256` |
| Highest canvas error code in use | 77050024 | `grep -roE '7705[0-9]+' src/codegen/builtins/canvas/*.rs \| sort -u \| tail -1` → 77050024 |
| Frame quad cap / item block size | 4096 / 208 B | `grep -n 'CANVAS_MAX_FRAME_ITEMS\|ITEM_BLOCK_SIZE: usize' src/codegen/runtime/canvas/mod.rs` → 681, 328 |

### Verified properties

- **A particle's full state is expressible in existing item fields.** VERIFIED by
  reading `mod.rs:672-760`: `Paint` carries `transform AS Transform` (a full affine —
  `mod.rs:538-570` documents `a := cos θ, b := -sin θ, c := sin θ, d := cos θ` for a
  rotation and `a := sx, d := sy` for a scale) plus `fill` and `stroke`. Position,
  rotation and scale go in `paint.transform`; colour goes in `paint.fill`/`paint.stroke`.
- **Tint-by-multiply is already the documented rule for `Picture`.** VERIFIED by
  reading `mod.rs:955-963`: *"The paint's fill colour tints the image — each channel is
  multiplied by the fill's — so a white fill draws it unchanged and the fill's alpha is
  how opaque it is."* The particle tint therefore reuses an existing, documented rule
  rather than inventing a second one, and it works for sprite particles and shape
  particles identically.
- **The GPU transport already carries everything a particle needs.** VERIFIED by
  reading `src/codegen/runtime/canvas/mod.rs:330-439`: the `ItemBlock` has
  `ITEM_OFFSET_TRANSFORM` (128, two `ivec4`s: the inverse affine as float bits plus
  `hasTransform`) and `ITEM_OFFSET_FILL`/`ITEM_OFFSET_STROKE` (32/48, RGBA). No new
  region, no new binding, no widened block.
- **Recursive value types are supported.** VERIFIED by reading `.ai/collections.md`:
  a value "whose type reaches a type cycle" is copied with the graph walker
  (`needs_graph_copy`, plan-134-D) at the five owning stores. `item AS DrawItem` inside
  a `DrawItem` variant is exactly such a cycle. **This is the letter's highest-risk
  premise** — see Phase 1, which falsifies it first and cheaply.
- **UNVERIFIED — the geometry cache's behaviour under N distinct per-particle
  transforms.** The cache holds 256 entries keyed by item hash; N particles have N
  distinct hashes. Whether this thrashes, and what it costs, is **letter B's opening
  measurement**, not an assumption here. Phase 4 below emits particles through the
  ordinary `__canvas_geometryFor` path precisely so that B measures the real thing.

## 3. Design Overview

Five pieces, layered:

1. **The type set** — five new records, one new union, one new enum, and the eleventh
   `DrawItem` variant. Registry data only; no behaviour.
2. **The analytic model** — a pure function `slot → (alive, position, rotation, scale,
   colour)` from `(seed, time, emitter, emission, motion, appearance)`. Written once,
   in MFBASIC, on the graphics thread.
3. **The expansion** — a `CASE ParticleSystem(ps)` arm in `__canvas_appendDraw` that
   loops slots, skips dead ones, composes each live one's item, and appends it.
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

**Where correctness risk concentrates: the expansion loop.** It runs on the graphics
thread, where a `FAIL` has nowhere to unwind (`helper_render.rs:231` records exactly
this for group depth). Every raise must therefore happen on the worker, in `present`.
Scheduled last, behind tests, in Phase 4.

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
- **A per-entry affine+tint, generalizing `__CANVAS_DRAW_DX`/`DY`.** Rejected: the
  draw entry has only 3 reserved words (`helper_render.rs:400-402`) and a full affine
  plus RGBA needs ~7. Composing into the item's own `paint` avoids the transport
  question entirely.
- **Stateful simulation (integrate velocity each frame).** Rejected: errors compound,
  so software and GPU diverge over time and "render the same program twice and diff"
  — the method `tests/canvas/rt_canvas_metal.rs` is built on — stops meaning anything.
  Closed-form recomputation keeps every discrepancy sub-pixel and non-accumulating.
- **Continuous and burst emission simultaneously in one system.** Rejected: two
  overlapping birth schedules over one slot pool make `age(slot)` ambiguous. `mode` is
  an enum; layering two systems composes cleanly and keeps each schedule a single
  unambiguous formula.

## 4. The type set

All registry data in `src/codegen/builtins/canvas/mod.rs`.

```
canvas::ParticleSystem                    ' the 11th DrawItem variant — a CONTAINER, no paint
    item        AS canvas::DrawItem       ' the template drawn once per live particle
    emitter     AS canvas::Emitter
    emission    AS canvas::Emission
    motion      AS canvas::Motion
    appearance  AS canvas::Appearance
    seed        AS Integer                ' fixes every per-particle random draw
    time        AS Float                  ' the program's clock in seconds; the program advances it

canvas::Emitter                           ' a CLOSED union — where particles are born
    EmitPoint   [x, y]
    EmitLine    [x1, y1, x2, y2]
    EmitRect    [x, y, w, h]
    EmitCircle  [x, y, radius, innerRadius]   ' innerRadius 0 = filled disc, >0 = ring

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

`ParticleSystem` carries **no `paint`**, exactly as `Group` does not: the template
carries it. It therefore joins `Group` in the named container exemptions of
`every_draw_item_variant_carries_a_paint`.

`Emitter` is a **second closed union** in the package and gets its own freeze test,
mirroring `draw_item_variant_set_is_frozen`, for the same reason: a user's `MATCH` over
it stops being exhaustive if a variant is added later.

**Variant names are prefixed `Emit*`** because `Line`, `Rectangle` and `Circle` are
already `DrawItem` record names in the same package namespace.

## 5. The analytic model

One MFBASIC function per stage, in a new `src/codegen/builtins/canvas/helper_particles.rs`.
Every quantity below is a closed form in `(slot, time)`; nothing integrates, and nothing
is remembered between frames.

**Per-slot randoms.** `__canvas_particleHash(seed, slot, cycle, stream)` returns a
`Float` in `[0,1)`. It reuses the integer mixing already proved in this module
(`__canvas_hashFloat`, `helper_geometry.rs`). `stream` separates the independent draws
so that changing `speedVar` cannot move a particle's spawn point. Draws used:
`r0` lifetime, `r1`/`r2` emitter sample, `r3` direction, `r4` speed, `r5` rotation,
`r6` spin, `r7` scale, `r8`/`r9` turbulence phase.

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

**Spawn point**, uniform over the emitter, closed form:

```
EmitPoint  → (x, y)
EmitLine   → lerp((x1,y1), (x2,y2), r1)
EmitRect   → (x + r1*w, y + r2*h)
EmitCircle → r = sqrt(innerRadius² + r1*(radius² - innerRadius²));  φ = 2π*r2
             → (x + r*cos φ, y + r*sin φ)
```

The `sqrt` in `EmitCircle` is what makes the disc sample *uniform by area*; sampling
`r` linearly clusters particles at the centre and is the classic mistake here.

**Velocity.** `θ = angle + spread*(2*r3 - 1)`, `s = speed * (1 + speedVar*(2*r4 - 1))`,
`v0 = (s*cos θ, s*sin θ)`.

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
`color::toLinear` / the inverse, because that is the space the rasteriser and both
shaders composite in (`.ai/canvas-threading.md`; the `__COLOR_SRGB` note in
`src/codegen/runtime/canvas/mod.rs:16`). Interpolating in sRGB would make a
red→transparent fade pass through a visibly wrong midpoint.

## 6. The expansion

A `CASE ParticleSystem(ps)` arm in `__canvas_appendDraw` (`helper_render.rs:157`),
structurally the same as `CASE Group(g)`:

```
CASE ParticleSystem(ps)
  IF depth >= __CANVAS_GROUP_MAX_DEPTH THEN RETURN out      ' silent, as Group's is
  FOR i = 0 TO ps.emission.maxParticles - 1
    LET st AS ParticleState = __canvas_particleState(ps, i)
    IF st.alive THEN
      LET child AS DrawItem = __canvas_particleItem(ps.item, st)
      out = __canvas_appendDraw(out, child, __canvas_hashItem(child), gdx, gdy, depth + 1)
    END IF
  NEXT
  RETURN out
END CASE
```

`__canvas_particleItem(template, st)` returns the template with exactly two changes:

- **`paint.transform` = `T(st.px, st.py) ∘ R(st.rot) ∘ S(st.scale) ∘ T(-cx, -cy) ∘
  template.paint.transform`**, where `(cx, cy)` is the template's own bounding-box
  centre, so a particle rotates and scales **about its own centre** rather than about
  the canvas origin. The template's existing transform is composed *innermost* and
  therefore still means what it meant.
- **`paint.fill` = `template.paint.fill × st.color`** and **`paint.stroke` =
  `template.paint.stroke × st.color`**, per channel including alpha — the rule
  `Picture` already documents (`mod.rs:957`). A white, opaque `startColor`/`endColor`
  leaves the template's own colours untouched.

`blend`, `clip`, `strokeWidth` and `fillGradient` are carried through unchanged.

The depth guard is inherited from `Group` and shares its constant: a `ParticleSystem`
whose template is a `Group` containing a `ParticleSystem` is bounded by the same 64.

Because the arm recurses through `__canvas_appendDraw`, a particle whose template is a
`Group` expands correctly with no extra code, and every particle lands in
`__CANVAS_DRAW_HASHES`/`__CANVAS_DRAW_DX`/`__CANVAS_DRAW_DY` the way any other entry
does — so damage, the geometry cache, both GPU emitters and the software walk all see
ordinary items.

## 7. Validation and raises

Raised on the **worker**, inside `canvas::present`, alongside the existing group
signature check — never on the graphics thread (`helper_render.rs:231` records why).
New error codes continue from 77050024:

| Condition | Behaviour |
|---|---|
| `maxParticles <= 0` | draws nothing; **not** an error |
| `rate <= 0` with `mode = Continuous` | draws nothing; not an error |
| `burst <= 0` with `mode = Burst` | draws nothing; not an error |
| `lifetime <= 0` | `FAIL error(77050025, …)` — no closed form for `u` exists |
| `template is ParticleSystem` | `FAIL error(77050026, …)` — a system of systems multiplies quads without bound |
| `lifetimeVar`, `speedVar`, `spinVar`, `scaleVar` outside `[0,1]` | clamped; not an error |
| `time` negative | draws nothing; not an error |

`burst > maxParticles` is clamped to `maxParticles`, not an error: the pool is the
bound and clamping is the honest reading of "release this many from this pool".

## Compatibility / Format Impact

- **Added, externally observable:** one `DrawItem` variant, five records, one union,
  one enum, two error codes. `mfb man canvas types` grows accordingly (letter C).
- **Unchanged:** `ITEM_BLOCK_SIZE` (208), the `ItemBlock` layout, `CANVAS_MAX_FRAME_ITEMS`
  (4096), the draw-entry width (8 words), both shaders, both `.spv` blobs, the geometry
  header layout, `Paint`, `Transform`, and every existing variant's record.
- `scripts/regen-spirv.sh` is **not** run by this plan; if a diff appears in either
  `.spv`, that is a bug in this letter.
- Existing programs: no source change, and `.ncode` expected byte-identical.

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
      with a single prop `item AS canvas::DrawItem`, and append it as the eleventh
      `DrawItem` variant.
- [ ] Amend `draw_item_variant_set_is_frozen` to the eleven-name list, **appending**
      `ParticleSystem` last so no existing variant's tag moves. Keep the assertion's
      warning message; record the amendment in the doc comment beside the `Ellipse`
      and `Group` entries, per the rule that the list grows by one visible entry and
      never gets laxer.
- [ ] Add `ParticleSystem` to the named container exemptions in
      `every_draw_item_variant_carries_a_paint`, beside `Group`.
- [ ] Write a runtime probe that presents a scene containing
      `ParticleSystem[item := Circle[…]]` and survives `present`'s deep copy.
- [ ] Tests: `tests/canvas/rt_canvas_particles.rs`, new — the probe above.

Acceptance: a scene holding a `ParticleSystem` whose `item` is a `Circle` is presented,
deep-copied and dropped without a leak, a double free, or a SIGSEGV — i.e. the type
cycle survives the graph-copy path `.ai/collections.md` describes.
  Check: `cargo test --test rt_canvas_particles` → pass (est. 3 min).
Commit: —

### Phase 2 — The full type set

Registry data only; still no behaviour, so nothing can regress.

- [ ] Expand `ParticleSystem` to all seven props (§4).
- [ ] Add records `Emission`, `Motion`, `Appearance` and the four `Emit*` records.
- [ ] Add the `Emitter` union and the `EmissionMode` enum.
- [ ] Add `emitter_variant_set_is_frozen`, mirroring `draw_item_variant_set_is_frozen`,
      pinning the four `Emit*` names and their order.
- [ ] Write per-field descriptions to the standard the existing records set — every
      unit (px, px/s, px/s², radians, seconds) named in the text.
- [ ] Tests: extend `tests/canvas/rt_canvas_particles.rs` to construct every record and
      every `Emitter` variant.

Acceptance: `mfb man canvas types` lists `ParticleSystem`, `Emitter`, `Emission`,
`Motion`, `Appearance` and `EmissionMode` with every field documented, and a program
constructing all four `Emitter` variants compiles and runs.
  Check: `cargo test -p mfb canvas` → pass, and
  `cargo run -- man canvas types | grep -c 'ParticleSystem\|Emitter\|Emission\|Motion\|Appearance'` → ≥ 6 (est. 5 min).
Commit: —

### Phase 3 — The analytic model, as a pure function

Testable in isolation, before anything draws. The whole model is a function of its
arguments, so this phase is where its correctness is actually established.

- [ ] New `src/codegen/builtins/canvas/helper_particles.rs`: `__canvas_particleHash`,
      `__canvas_particleAge`, `__canvas_particleSpawn`, `__canvas_particlePosition`,
      `__canvas_particleState`, per §5.
- [ ] Register the helper in `src/codegen/builtins/canvas/mod.rs` alongside the other
      `helper_*` registrations.
- [ ] Unit-test the module the way the sibling helpers are tested, in
      `src/codegen/builtins/canvas/helper_particles/tests/mod.rs`.
- [ ] Tests, each a named case in `tests/canvas/rt_canvas_particles.rs`:
      **determinism** (same `(seed, time)` → identical state, twice in one run);
      **liveness** (a slot is dead before birth and after `lifetime`);
      **drag limit** (`drag = 1e-6` agrees with `drag = 0` to 1e-4 px over 1 s);
      **disc uniformity** (10,000 `EmitCircle` samples: the inner-half-radius disc holds
      within 2% of a quarter of them — the check that catches a missing `sqrt`);
      **burst schedule** (with `burstPeriod = 2.0`, slots are alive at `t = 0.1` and
      `t = 2.1` and dead at `t = 1.9` for `lifetime = 1.0`).

Acceptance: every case above passes, and the disc-uniformity case **fails** if the
`sqrt` is removed from `EmitCircle` — verified once by removing it, observing the
failure, and restoring it.
  Check: `cargo test --test rt_canvas_particles` → pass (est. 5 min).
Commit: —

### Phase 4 — Expansion and rendering (largest blast radius)

Touches the draw walk every scene goes through, so it lands last and behind Phase 3's
tests.

- [ ] Add the `CASE ParticleSystem(ps)` arm to `__canvas_appendDraw`
      (`helper_render.rs:157`) per §6, including the silent depth guard.
- [ ] Add `__canvas_particleItem` composing `paint.transform` and multiplying
      `paint.fill`/`paint.stroke` per §6.
- [ ] Add the worker-side validation and the two raises to `func_present.rs`, beside
      the existing group-signature check, per §7.
- [ ] Confirm the three per-entry-offset consumers (`helper_render.rs:69`,
      `helper_damage.rs:129`, `helper_damage.rs:243`) need no change, because a
      particle is an ordinary entry — and say so in a comment at the expansion site so
      the next reader does not re-derive it.
- [ ] Tests: a software golden `tests/golden/canvas/particles.png` via the existing
      `render()` harness in `tests/canvas/rt_canvas_golden.rs`, from a fixed
      `(seed, time)` fireworks scene — a `Burst` `EmitPoint` system with gravity,
      a colour ramp and a `Circle` template.
- [ ] Tests: a negative case per raise in `tests/syntax/canvas/` or as a runtime raise
      case, matching how the group-depth raise is tested.

Acceptance: the fireworks golden renders identically on two consecutive runs, and a
template that is itself a `ParticleSystem` raises `77050026` from `present` on the
worker — not on the graphics thread.
  Check: `cargo test --test rt_canvas_golden --test rt_canvas_particles` → pass, and
  run the golden twice comparing with `compare_exact` (est. 5 min).
Commit: —

### Phase 5 — Prove the GPU backends need no change

The design's central claim, measured rather than asserted. If it fails, the failure is
a bug in Phase 4's composition to root-cause — not evidence the design is dead.

- [ ] Add a Metal parity case to `tests/canvas/rt_canvas_metal.rs` rendering the
      Phase 4 fireworks scene twice — `MFB_CANVAS_GPU=1` and software — and diffing
      within `Tolerance::GPU_DEFAULT`, the pattern the file already uses.
- [ ] Confirm `git status` shows **no** diff in `src/codegen/runtime/canvas/shaders/`
      — neither `.frag`/`.vert` nor either `.spv`.
- [ ] Record in the plan's Corrections section whether the no-shader-change claim held.

Acceptance: the Metal parity case passes within `Tolerance::GPU_DEFAULT` on a host
reporting a Metal device, skips cleanly on one that does not, and no shader source or
SPIR-V blob has changed.
  Check: `cargo test --test rt_canvas_metal` → pass or documented skip, and
  `git status --porcelain src/codegen/runtime/canvas/shaders/` → empty (est. 5 min).
Commit: —

## Validation Plan

- **Tests:** `tests/canvas/rt_canvas_particles.rs` (new — determinism, liveness, drag
  limit, disc uniformity, burst schedule, both raises); a new golden
  `tests/golden/canvas/particles.png`; a Metal parity case in
  `tests/canvas/rt_canvas_metal.rs`; registry tests in
  `src/codegen/builtins/canvas/mod.rs` (`draw_item_variant_set_is_frozen` amended,
  `emitter_variant_set_is_frozen` new, `every_draw_item_variant_carries_a_paint`
  amended); helper unit tests in `helper_particles/tests/mod.rs`.
- **Coverage check:** `scripts/coverage-check.sh` — confirm `helper_particles.rs` is in
  the denominator. A green gate means "nothing covered changed"; a new file absent from
  coverage would make that claim vacuous for exactly the code this letter adds.
- **Runtime proof:** a program presenting a `Burst` `EmitPoint` system with a `Picture`
  template, `time` advanced over 60 frames, run under `MFB_CANVAS_STATS` — the stats
  line must report GPU frames on a Metal host and the quad count must track the live
  particle count rather than `maxParticles`.
- **Doc sync:** `mfb man canvas` gains the new types (letter C does the prose);
  `.ai/canvas-threading.md` §4 gains a sentence stating that a particle system is
  advanced by the program re-presenting with a new `time`, and that this is trigger 1
  and **not** a sixth trigger.
- **Final gate (run ONCE, after Phase 5):** `scripts/test-accept.sh <mfb-exe> <actual-output-dir>` (both arguments are required; the script exits 2 without them) (est. per its own
  runtime). Per-phase checks stay scoped; do not repeat this per phase.

## Open Decisions

- **Rotation origin** — the template's bounding-box centre (recommended, §6) vs. the
  template's own coordinate origin. Centre is what every particle system means by
  "rotate the sprite", but it costs a bounds computation per particle. If letter B
  measures that as significant, revisit.
- **`turbulence` as two sines vs. a shared value-noise function** — two sines
  recommended (§5). Noise looks better for fog specifically; it becomes worth its cost
  only if a later letter moves the model into shaders, where a table would then have to
  agree across three languages.
- **Whether `Text` should be allowed as a template** — allowed as specified. One text
  particle is N quads against the 4096 frame cap, so a 200-particle text system can
  decline a frame to software on its own. Letter B measures it; if the cliff is sharp,
  a documented warning in `mfb man canvas` may be the right answer rather than a
  restriction.

## Corrections

<Filled in DURING execution.>

## Summary

The engineering risk is concentrated in two places, and they are scheduled at opposite
ends. **Phase 1** falsifies the recursive-type premise — the only thing that would
reshape the design — for the cost of one record and one probe. **Phase 4** touches the
draw walk that every scene goes through, and lands last, behind the model's own tests.

Everything between them is additive registry data and a pure function.

What this letter deliberately leaves untouched is the entire GPU transport: no shader,
no pipeline, no binding, no buffer layout, no cap. That is not restraint for its own
sake — it is the direct consequence of particles being expressible in `paint.transform`
and `paint.fill`, and Phase 5 exists to prove the claim rather than assume it.
