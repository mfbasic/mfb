# plan-98-B: Canvas scene model — union, deep copy, arena, hashing, resource table

Last updated: 2026-08-15
Effort: large (3h–1d)
Depends on: plan-98-A (mode plumbing + reconcile build/teardown must be complete)

This sub-plan builds the **worker-thread-side scene model** with no GPU and no
graphics thread: the `DrawItem` UNION (closed variant set), the `canvas::present` /
`canvas::presentLayers` calls, transitive deep-copy into a runtime-owned scene arena,
per-item content hashing, the runtime-side geometry cache keyed on that hash, the
zero-work frame-skip, and the `Image`/`Font` resource handle table with atomic
refcounts. The single checkable outcome: `canvas::present(items)` in `Mode.Canvas`
deep-copies a scene into runtime-owned storage (nothing points at caller memory),
hashes each item, populates/reuses the geometry cache, and — this is all pure
worker-thread logic — is fully unit-testable without a GPU or a graphics thread.

This is design-doc **build step 2**.

References:

- The design summary — "Type model", "present() Semantics", "Scene Arena Layout",
  "Geometry Cache", "Resource Lifetime", and the work-split table.
- plan-98-A cross-cutting invariants 1–4 and 6 (binding here).
- `.ai/resources-packages.md` — RES resource system + package authoring seams.
- `.ai/collections.md` — `List OF` representation (the scene is a language array).
- `.ai/codegen-invariants.md` — record layout, monomorph, deep-copy patterns.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-98-A complete (Canvas mode enters/tears down) | `ls planning/completed/plan-98-A-*` → hit | NOT MET (A precedes B) |
| Canvas surface handle retrievable in canvas mode | plan-98-A Phase 3 acceptance met | NOT MET |
| Full suite green at HEAD | `cargo test` → pass | UNVERIFIED |

> plan-98-A is a precondition, not scope. If A is incomplete, B cannot start, full stop.

## 1. Goal

- Register the `canvas::` builtin package (`canvas_package.mfb` + `canvas.rs`
  `BuiltinModule` + `descriptor.rs:REGISTRY` + `mod.rs:ALL_BUILTIN_PACKAGES` +
  per-backend `runtime_calls`), exposing the closed `DrawItem` UNION type and the
  calls `canvas::present(items AS List OF DrawItem)` and
  `canvas::presentLayers(layers AS List OF DrawLayer)`, both requiring `Mode.Canvas`.
- `canvas::present` **deep-copies the scene transitively** (item params, polygon
  point arrays, text strings, `Paint` records, referenced `Image`/`Font` handles)
  into a runtime-owned scene arena; after `present()` returns, nothing in the
  published scene points at caller-owned memory.
- Each item is content-hashed; a runtime-side geometry cache keyed on the hash is
  probed (hit → reuse cached vertex range; miss → allocate an entry — geometry
  *generation* itself is a stub returning empty vertices until C, but the cache
  mechanics, hashing, and eviction are real).
- **Zero-work frame skip:** if the incoming item-hash sequence is identical and
  same-length to the live scene's, `present()` returns without publishing.
- `Image`/`Font` handles are `{u32 index; u32 generation;}` into a runtime table with
  an **atomic refcount**; `present()` increments refcounts as it copies, using the
  normative *increment-then-recheck-generation* sequence (invariant 4). `image::create`
  / `font::load` allocate entries; `image::destroy` marks dead + bumps generation and
  frees only at refcount zero.

### Non-goals (explicit constraints)

- **No graphics thread, no scene ring, no GPU, no rendering.** `present()` publishes
  into a single runtime-owned "live scene" slot read only by tests in this sub-plan.
  The triple-buffer ring and fence-gated retirement are D.
- **No geometry generation.** Tessellation, stroke expansion, and text shaping are
  stubbed (cache miss allocates a zero-length vertex range). C/G fill them. The cache
  *contract* (probe/insert/evict, hash keying) is real and tested now.
- **The `DrawItem` variant set is frozen here and is a breaking change to extend
  later** (invariant 6): Image, Rectangle, Line, Polygon, Text, RoundedRect (SDF).
  No variant is added after this sub-plan without a deliberate breaking-change plan.
- No change to `Mode` discriminants, the presentation slot, or non-canvas codegen.

## 2. Current State

- **Package registration seams** (from A's research, re-cited):
  `src/builtins/descriptor.rs:643:REGISTRY`, `src/builtins/mod.rs:1074:ALL_BUILTIN_PACKAGES`,
  per-backend `BackendCapabilities.runtime_calls` (`src/target.rs:106`), enforced by
  `src/target/shared/validate/capabilities.rs:validate_capabilities`. A new call not
  advertised is a hard compile error.
- **`term::` as the present precedent:** `src/target/shared/code/term.rs:lower_term_helper`
  (152), `term::sync` → `term_grid::emit_grid_present`. The `term::` model is *ambient
  mutation + present-diffs*; canvas deliberately differs (retained scene, content-hash
  cache) per the design summary, but the deep-copy-before-handoff discipline is the
  same lesson as the live→snapshot copy (`term_draw.rs:emit_term_snapshot_copy`).
- **RES resource system** for `Image`/`Font` backing (see `.ai/resources-packages.md`)
  — UNVERIFIED how closely the existing RES table matches the `{index, generation,
  refcount}` shape needed; Phase 1 task reads it to decide reuse vs new table.
- **Scene arena:** there is no runtime-owned arena that outlives a call today
  (`term::` grids live in the program-entry frame). The scene arena is **new**,
  runtime-owned (not caller-frame-scoped), because scenes outlive `present()`.

### Measured populations

| What | Count | Command |
|---|---|---|
| `DrawItem` variants to define | 6 | design summary variant set (Image, Rectangle, Line, Polygon, Text, RoundedRect) |
| Existing builtin packages in REGISTRY | UNMEASURED | `rg -c '&[a-z_]*::[A-Z_]*,' src/builtins/descriptor.rs` (run Phase 1) |
| RES table shape vs needed handle shape | UNVERIFIED | read `.ai/resources-packages.md` + the RES table struct (Phase 1) |

### Verified properties

- **A `List OF DrawItem` is a plain language array** (invariant: retained scene is a
  language value, cache is runtime-side keyed on content hash). VERIFIED against the
  design summary's central claim; the array carries no opaque handle.
- **Deep copy is mandatory** because the render thread reads at arbitrary times.
  VERIFIED from the design's `present()` semantics; enforced here even though the
  reader is only a test until D.
- UNVERIFIED: whether the existing RES refcount (if any) is atomic. Phase 1 reads it;
  if not atomic or not generation-tagged, this sub-plan adds the canvas resource table
  rather than bending RES.

## 3. Design Overview

Four layered pieces, worker-thread only:

1. **`canvas::` package + `DrawItem`/`DrawLayer`/`Paint` types + `present`/
   `presentLayers` signatures.** Registration and the closed union. Lowest risk.
2. **Scene arena + transitive deep copy.** A runtime-owned arena
   (`mfb.runtime.canvas_scene.v1` per the design's layout: `revision`, `itemCount`,
   `generation`, `damage`, `hashes[]`, `bounds[]`, `vtxOffset[]`, `vtxCount[]`,
   `params[]`). `present()` walks the incoming list and copies every reachable byte
   into `params[]`.
3. **Content hashing + geometry cache + frame-skip.** Hash each item (flat-union
   `memcmp`-speed); probe `GeoCacheEntry{hash, vtxOffset, vtxCount, bounds, lastUsedRev}`;
   hit reuses, miss inserts (geometry stubbed empty), LRU-evict by `lastUsedRev` under
   arena pressure; identical whole-sequence hash → return without publishing.
4. **Resource handle table.** `{index, generation}` handles, atomic refcount;
   `image::create`/`font::load` allocate; `present()` increments via the normative
   increment-then-recheck sequence; `image::destroy` marks dead + bumps generation,
   frees at zero.

**Where correctness risk concentrates:** the resource refcount sequence (invariant 4).
Even without the graphics thread, the *copy path* must already use
increment-then-recheck-generation so that D can add a concurrent releaser without
reworking `present()`. This is the piece the design doc says to write on paper first;
land it last in this sub-plan, behind a test that drives create→present→destroy→
retire ordering on a single thread (the multi-thread race is exercised in D).

**Where design uncertainty concentrates:** RES-table reuse vs a new canvas table
(Phase 1 resolves), and the exact `params[]` encoding for variable-length payloads
(polygon points, strings). Resolve the encoding before hashing, since the hash spans
the copied bytes.

**Byte-identity is not this sub-plan's gate.** This is new behavior; it is verified by
unit tests over the copy/hash/cache/refcount logic (pure worker-thread, GPU-free — the
design's step-2 selling point). Non-canvas programs remain byte-identical (verified via
the corpus), but that is a guardrail, not the acceptance criterion.

**Rejected alternatives:**
- *Store geometry in the user-visible `DrawItem`.* Rejected: it would force opaque
  handles and break the "plain language array" ergonomics; the runtime-side content-hash
  cache is what makes the array a plain value.
- *Ambient `Paint` current-state (like a canvas 2D context).* Rejected in the design:
  ambient state interacts badly with retained scenes ("which fill was current when item
  47 was appended?"). `Paint` is a flat value record threaded through items.

## Compatibility / Format Impact

- **Changes:** new `canvas::` package surface (`present`, `presentLayers`,
  `image::create`/`load`/`destroy`, `font::load`, `canvas::size`), new user-visible
  types (`DrawItem` union, `DrawLayer`, `Paint`, `Image`, `Font`, `Bounds`,
  `TextMetrics`). New runtime scene arena + resource table (internal, not a wire
  format). New per-backend advertised `runtime_calls`.
- **Unchanged:** everything non-canvas; `Mode` discriminants; presentation slot;
  `term::`/`io::` semantics.
- **Frozen forever after this lands:** the `DrawItem` variant set (invariant 6).

## Phases

### Phase 1 — `canvas::` package, closed types, present signatures, RES decision

- [ ] Read `.ai/resources-packages.md` + the RES table struct; decide RES-reuse vs
      a new canvas resource table for `{index, generation, atomic refcount}`. Record
      the decision and evidence in Corrections.
- [ ] Create `src/builtins/canvas_package.mfb` (`EXPORT UNION DrawItem` with the 6
      frozen variants; `DrawLayer`, `Paint`, `Image`, `Font`, `Bounds`, `TextMetrics`
      records; `present`/`presentLayers` signatures; helper bodies stay in the `.mfb`,
      member codegen in `canvas.rs` per the migration pattern in
      `.ai/resources-packages.md`).
- [ ] Create `src/builtins/canvas.rs` `CANVAS: BuiltinModule`; register in
      `descriptor.rs:REGISTRY` and `mod.rs:ALL_BUILTIN_PACKAGES`; advertise the new
      calls in each `--app` backend's `runtime_calls`; add runtime helper specs.
- [ ] Gate every `canvas::` call on `Mode.Canvas` (reuse the mode-gate seam;
      `canvas::present` in `Console`/`None` traps `ErrWrongMode`, per the design's I/O
      matrix).
- [ ] Tests: package imports only in `--app` builds; `canvas::present` traps
      `ErrWrongMode` outside canvas mode; the union type is exhaustively matchable.

Acceptance: a `--app` program imports `canvas::`, and `canvas::present` compiles in
canvas mode and traps `ErrWrongMode` elsewhere; `validate_capabilities` passes on all
backends; full `cargo test` green.
Commit: —

### Phase 2 — Scene arena + transitive deep copy

- [ ] Define the runtime scene arena `mfb.runtime.canvas_scene.v1` (fields per the
      design layout) as runtime-owned storage (not caller-frame-scoped).
- [ ] Implement `present()` deep copy: walk the `List OF DrawItem`, copy every
      reachable payload (params, polygon point arrays, text strings, `Paint`) into
      `params[]`. After copy, assert (in tests) no field points into caller memory.
- [ ] Publish into a single "live scene" slot (no ring yet); bump `revision`.
- [ ] Tests: build a scene referencing caller-frame arrays/strings, `present()`, drop
      the caller frame, and read the published scene back intact — proves the copy is
      transitive and self-contained.

Acceptance: a scene whose sources go out of scope is fully readable from runtime
storage after `present()`; no dangling pointer into caller memory (test-verified).
Commit: —

### Phase 3 — Content hashing, geometry cache, frame-skip

- [ ] Hash each copied item (flat-union byte hash over `params[]`); store into
      `hashes[]`.
- [ ] Implement the geometry cache: `GeoCacheEntry{hash, vtxOffset, vtxCount, bounds,
      lastUsedRev}`; probe on hash; hit reuses vtx range; miss inserts with a **stub
      empty geometry** (real generation is C); LRU-evict by `lastUsedRev` under arena
      pressure.
- [ ] Implement zero-work frame skip: if incoming hash sequence == live scene's hash
      sequence and same length, return without publishing.
- [ ] Implement positional diff/damage: per-index hash compare, union old∪new bounds
      on divergence, length-differ → dirty from divergence point. (Damage is *computed*
      only if cheap to keep; if it adds work with no consumer yet, defer to G and note
      it — see Open Decisions.)
- [ ] Tests: identical re-present publishes nothing; one changed item regenerates one
      cache entry; LRU eviction fires under a forced small arena; hash sequence compare
      is O(n) and correct across length changes.

Acceptance: re-`present()` of an identical list publishes no new revision; changing one
item invalidates exactly one cache entry; eviction is `lastUsedRev`-ordered — all
test-proven, no GPU.
Commit: —

### Phase 4 — Resource handle table + refcount protocol (largest blast radius last)

- [ ] Implement the `{index, generation}` handle table with atomic refcount (RES-backed
      or new per Phase 1).
- [ ] `image::create`/`image::load`/`font::load` allocate an entry (fallible — return
      per the result ABI: tag in x0, value in x1).
- [ ] `present()` increments refcounts for referenced resources using the **normative
      increment-then-recheck-generation sequence** (invariant 4): increment
      unconditionally, re-read generation, if changed decrement and treat as dead handle.
- [ ] `image::destroy` marks the entry dead + bumps generation; frees the backing only
      at refcount zero. Live scene keeps it alive.
- [ ] Single-thread retirement stub: publishing a new scene decrements the old scene's
      resource refs (the *fence-gated* multi-thread version is D; here it is immediate,
      single-thread, and correct for the no-graphics-thread world).
- [ ] Tests: create→present→destroy leaves the resource alive while the live scene
      references it, freed after the scene is replaced; a stale handle (old generation)
      is rejected; the increment-then-recheck sequence rejects a resurrected slot
      (drive it deterministically by bumping generation between increment and recheck).

Acceptance: destroying a referenced image mid-scene never frees it while live; a
stale-generation handle is caught; the copy path uses increment-then-recheck (proven by
a deterministic generation-bump test). Full `cargo test` green.
Commit: —

## Validation Plan

- Tests: unit tests over deep copy, hashing, cache probe/insert/evict, frame-skip,
  positional diff, and the resource refcount sequence — all in-process (`--bin mfb`
  denominator per `.ai/build-tooling.md`), GPU-free.
- Coverage check: confirm the scene/cache/refcount code is measured by in-process unit
  tests (const-fn table builders need `black_box` runtime tests per the coverage memo).
- Runtime proof: a headless `--app` program builds a scene, `present()`s it, mutates
  one item, re-`present()`s — observable via the published `revision` and cache-hit
  counters exposed to the test harness.
- Doc sync: `src/docs/spec/app/` new `canvas` section (scene model, deep-copy rule,
  the frozen variant set, resource lifetime); man pages for `canvas::present`,
  `presentLayers`, `image::*`, `font::load`, `canvas::size` per the man templates.
- Acceptance: full `cargo test`; non-canvas byte-identity corpus unchanged; fmt.

## Open Decisions

- **RES table reuse vs new canvas resource table** — recommended: reuse RES if its
  refcount is already atomic and generation-tagged; else new table. Resolve in Phase 1.
- **`params[]` encoding for variable-length payloads** — recommended: length-prefixed
  contiguous blobs per item so the hash spans a stable byte range. (§Design 2)
- **Compute damage in B or defer to G** — recommended: **defer the bounds-union damage
  to G** (it has no consumer until damage-rect present); keep only the cheap
  whole-sequence frame-skip in B. Note the deferral in Phase 3 rather than computing
  unused work (invariant against per-frame waste). (§Phase 3)

## Corrections

<Filled in during execution — especially the RES decision and payload encoding.>

## Summary

Risk in B concentrates in Phase 4's refcount sequence, which must be written correct
now (increment-then-recheck) even though the concurrent releaser arrives in D — getting
it wrong here is the design doc's predicted source of intermittent crashes. Everything
else (package, arena, copy, hash, cache) is pure worker-thread logic, fully testable
without a GPU, and is the shippable foundation C renders and D threads.
