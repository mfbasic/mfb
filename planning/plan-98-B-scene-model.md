# plan-98-B: Canvas scene model — union, deep copy, arena, hashing, RES resources

Last updated: 2026-08-15
Effort: large (3h–1d)
Depends on: plan-98-A (mode plumbing + reconcile build/teardown must be complete)

This sub-plan builds the **worker-thread-side scene model** with no GPU and no
graphics thread: the `DrawItem` UNION (closed variant set), the `canvas::present` /
`canvas::presentLayers` calls, transitive deep-copy into a runtime-owned scene arena,
per-item content hashing, the runtime-side geometry cache keyed on that hash, the
zero-work frame-skip, and `Image`/`Font` as **RES resources** (closed-flag lifetime,
no refcounting). The single checkable outcome: `canvas::present(items)` in `Mode.Canvas`
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
- `Image`/`Font` are a new **native RES backend** (the existing resource record
  `tag@0 / handle@8 / closed@16 / STATE@24`, `handle@8` = OS-side texture id), owned by
  MFB scope like a file — **no refcount, no generation table** (invariant 4).
  `canvas::loadImage`/`createImage`/`loadFont` allocate a resource; `canvas::destroyImage`/
  `destroyFont` (or scope-drop of the owner) set `closed@16` and mark the OS texture
  pending-free. `present()` copies the resource **id** (an integer) into the scene; it
  performs **no** refcount work. Using a closed resource is the existing
  `ERR_RESOURCE_CLOSED` no-op (the stale-id safety net). Actual OS free is D's job,
  gated on `closed AND lastUsedFrame < lastCompletedFrame` — a monotonic compare, not a
  count.
- **Image content is mutable independently of the scene.** Each image RES resource keeps
  a **CPU-side shadow** of its RGBA8 pixels (also the re-upload source + device-lost
  recovery copy). `canvas::getBytes(image)` returns the shadow (cheap — no GPU readback);
  `canvas::setBytes(image, pixels)` deep-copies into the shadow and marks the texture
  dirty — **fallible**, `ErrBadPixelCount` if `len(pixels) != width*height*4`;
  `canvas::getSize(image)` returns its dimensions. Mutating content does **not** go
  through `present()` (the scene layout is unchanged); the actual GPU upload + the redraw
  trigger are D's job. `createImage` takes `List OF Byte` RGBA8. `canvas::rgb`/`rgba`
  build `Color` values.

### Non-goals (explicit constraints)

- **No graphics thread, no scene ring, no GPU, no rendering.** `present()` publishes
  into a single runtime-owned "live scene" slot read only by tests in this sub-plan.
  The triple-buffer ring and the OS-side texture free (the `closed AND
  lastUsedFrame < lastCompletedFrame` gate) are D.
- **No geometry generation.** Tessellation, stroke expansion, and text shaping are
  stubbed (cache miss allocates a zero-length vertex range). C/G fill them. The cache
  *contract* (probe/insert/evict, hash keying) is real and tested now.
- **The `DrawItem` variant set is frozen here and is a breaking change to extend
  later** (invariant 6): Image, Rectangle, Line, Polygon, Circle, Arc, Text, RoundedRect.
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
- **RES resource system** is the model for `Image`/`Font` (`.ai/resources-packages.md`):
  every resource is a 96-byte record `tag@0 / handle@8 / closed@16 / STATE@24`
  (`RESOURCE_OFFSET_CLOSED = 16`, `RESOURCE_RECORD_SIZE_BYTES = 96`); `close ≠ drop`
  (close releases the OS handle, scope-drop reclaims the record); a double-close/use is
  a defined `ERR_RESOURCE_CLOSED` no-op. Canvas resources follow the doc's "Adding a NEW
  native backend" recipe (reserve `tag@0/handle@8/closed@16/STATE@24`, put the texture id
  in `handle@8`). This is scope-ownership, NOT refcounting — consistent with MFB having
  no refs/GC.
- **Scene arena:** there is no runtime-owned arena that outlives a call today
  (`term::` grids live in the program-entry frame). The scene arena is **new**,
  runtime-owned (not caller-frame-scoped), because scenes outlive `present()`.

### Measured populations

| What | Count | Command |
|---|---|---|
| `DrawItem` variants to define | 8 | Image, Rectangle, Line, Polygon, Circle, Arc, Text, RoundedRect (see plan-98-api.md) |
| Existing builtin packages in REGISTRY | UNMEASURED | `rg -c '&[a-z_]*::[A-Z_]*,' src/builtins/descriptor.rs` (run Phase 1) |
| RES record offsets to reserve for the canvas backend | 4 (`tag@0/handle@8/closed@16/STATE@24`) | `.ai/resources-packages.md` "Adding a NEW native backend" |

### Verified properties

- **A `List OF DrawItem` is a plain language array** (invariant: retained scene is a
  language value, cache is runtime-side keyed on content hash). VERIFIED against the
  design summary's central claim; the array carries no opaque handle.
- **Deep copy is mandatory** because the render thread reads at arbitrary times.
  VERIFIED from the design's `present()` semantics; enforced here even though the
  reader is only a test until D.
- **The RES closed flag is sufficient for lifetime; no refcount is needed.** VERIFIED
  from the model: `close ≠ drop` and scope-ownership floats to the outermost scope, so a
  single owner ends the resource's life via `closed@16`. Scenes reference the id (an
  integer) but do **not** own it — the only thing keeping the OS texture alive after
  close is the GPU still reading it, which D gates with `lastUsedFrame`. No count is ever
  required.

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
4. **`Image`/`Font` as a native RES backend.** Reserve `tag@0/handle@8/closed@16/STATE@24`;
   `handle@8` = OS texture id. `loadImage`/`createImage`/`loadFont` allocate a resource;
   `destroyImage`/`destroyFont` (and scope-drop) close it via the existing resource
   cleanup wiring. `present()` copies the **id** only — no refcount work at all.

**Where correctness risk concentrates:** wiring `Image`/`Font` into the existing RES
cleanup paths correctly (scope-drop reclaim, thread-transfer, the LINK-close-thunk /
`resource_close_function` gating described in `.ai/resources-packages.md`) so a canvas
resource closes and reclaims exactly like a file. There is **no** cross-thread refcount
race to design here — the OS free is deferred to D and gated purely on the closed flag +
frame drain, so B never frees an OS texture. Land the RES backend last in this sub-plan,
behind tests that drive load→present→destroy and scope-drop reclaim on a single thread.

**Where design uncertainty concentrates:** the exact `params[]` encoding for
variable-length payloads (polygon points, strings). Resolve the encoding before hashing,
since the hash spans the copied bytes. (The resource model is settled: RES closed-flag,
no table decision to make.)

**Byte-identity is not this sub-plan's gate.** This is new behavior; it is verified by
unit tests over the copy/hash/cache/resource-close logic (pure worker-thread, GPU-free — the
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
  `loadImage`/`createImage`/`destroyImage`, `loadFont`/`destroyFont`,
  `getBytes`/`setBytes`, `getSize` (canvas- and image-size overloads), `rgb`/`rgba`,
  `measureText`), new user-visible
  types (`DrawItem` union with 8 variants, `DrawLayer`, `Paint`, `Color`, `Point`, `Size`,
  `Bounds`, `TextMetrics`, `Image`, `Font`; `Image`/`Font` are RES resources). New runtime
  scene arena + a native RES backend for canvas textures with a CPU pixel shadow (internal,
  not a wire format). New per-backend advertised `runtime_calls`.
- **Unchanged:** everything non-canvas; `Mode` discriminants; presentation slot;
  `term::`/`io::` semantics.
- **Frozen forever after this lands:** the `DrawItem` variant set (invariant 6).

## Phases

### Phase 1 — `canvas::` package, closed types, present signatures

- [ ] Read `.ai/resources-packages.md` "Adding a NEW native backend"; confirm the
      canvas resource record reserves `tag@0/handle@8/closed@16/STATE@24` and how close
      dispatch / scope-drop reclaim are wired (`resource_close_function`, LINK thunks).
      Record the offsets/wiring in Corrections.
- [ ] Create `src/builtins/canvas_package.mfb` (`EXPORT UNION DrawItem` with the 8
      frozen variants — Image, Rectangle, Line, Polygon, Circle, Arc, Text, RoundedRect;
      `DrawLayer`, `Paint`, `Color`, `Point`, `Size`, `Bounds`, `TextMetrics` records;
      `Image`/`Font` RES types; `present`/`presentLayers`/`rgb`/`rgba` signatures; helper
      bodies stay in the `.mfb`, member codegen in `canvas.rs` per the migration pattern in
      `.ai/resources-packages.md`). Field shapes per plan-98-api.md.
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

### Phase 4 — `Image`/`Font` as a native RES backend (largest blast radius last)

- [ ] Add the canvas resource record per the "Adding a NEW native backend" recipe
      (`tag@0/handle@8/closed@16/STATE@24`, texture id in `handle@8`, tag in
      `error_constants.rs`, zero STATE at construction, the `== RESOURCE_OFFSET_*`
      asserts).
- [ ] `canvas::loadImage`/`createImage`/`loadFont` allocate a resource (fallible — return
      per the result ABI: tag in x0, value in x1); the OS-side texture is created by the
      backend (software now; Metal/Vulkan in E/F). `createImage` takes `List OF Byte`
      RGBA8; store the pixels in the image's **CPU shadow** (in STATE).
- [ ] `canvas::destroyImage`/`destroyFont` close the resource (set `closed@16` + release
      path); wire scope-drop reclaim, thread-transfer, and the `resource_close_function`
      / LINK-thunk gating exactly like a file resource.
- [ ] Image-content ops: `canvas::getBytes(image)` returns the CPU shadow (no GPU);
      `canvas::setBytes(image, pixels)` deep-copies into the shadow + marks the texture
      dirty, **fallible** `ErrBadPixelCount` when `len(pixels) != width*height*4`;
      `canvas::getSize(image)` returns the dimensions. The GPU upload of a dirty texture
      and the "in current scene → redraw" trigger are D's job; B only updates the shadow +
      dirty flag.
- [ ] Color helpers `canvas::rgb`/`rgba` build `Color` (clamp components 0..255).
- [ ] `present()` copies only the resource **id** into the scene — **no** refcount work.
- [ ] Mark-pending-free on close (a runtime-side flag the graphics thread reads in D);
      B does **not** free the OS texture — that is D's `closed AND lastUsedFrame <
      lastCompletedFrame` gate.
- [ ] Tests: load→present→destroy closes the resource and marks the texture pending-free;
      scope-drop of an un-destroyed image closes + reclaims the record; using a closed
      image is `ERR_RESOURCE_CLOSED`; double-close is the defined no-op; `setBytes` with a
      wrong-length list returns `ErrBadPixelCount`; `getBytes` round-trips the CPU shadow;
      `getSize` matches `createImage`.

Acceptance: canvas `Image`/`Font` load/close/scope-drop exactly like a file resource
(reclaim, transfer, double-close no-op all correct); `present()` does zero refcount work;
a closed image marks its texture pending-free without B freeing it; `setBytes` rejects a
wrong pixel count and `getBytes`/`getSize` reflect the shadow. Full `cargo test` green.
Commit: —

## Validation Plan

- Tests: unit tests over deep copy, hashing, cache probe/insert/evict, frame-skip,
  positional diff, the RES resource lifecycle (load/close/scope-drop/double-close), and the
  image-content ops (`getBytes`/`setBytes` round-trip, `setBytes` `ErrBadPixelCount`,
  `getSize`, `rgb`/`rgba` clamping) — all in-process (`--bin mfb` denominator per
  `.ai/build-tooling.md`), GPU-free.
- Coverage check: confirm the scene/cache/resource code is measured by in-process unit
  tests (const-fn table builders need `black_box` runtime tests per the coverage memo).
- Runtime proof: a headless `--app` program builds a scene, `present()`s it, mutates
  one item, re-`present()`s — observable via the published `revision` and cache-hit
  counters exposed to the test harness.
- Doc sync: `src/docs/spec/app/` new `canvas` section (scene model, deep-copy rule,
  the 8-variant frozen set, image-content-vs-scene orthogonality, RES resource lifetime —
  closed-flag, no refcount); man pages for `canvas::present`, `presentLayers`, `loadImage`,
  `createImage`, `destroyImage`, `loadFont`, `destroyFont`, `getBytes`, `setBytes`,
  `getSize`, `rgb`, `rgba`, `measureText` per the man templates.
- Acceptance: full `cargo test`; non-canvas byte-identity corpus unchanged; fmt.

## Open Decisions

- **`params[]` encoding for variable-length payloads** — recommended: length-prefixed
  contiguous blobs per item so the hash spans a stable byte range. (§Design 2)
- **Compute damage in B or defer to G** — recommended: **defer the bounds-union damage
  to G** (it has no consumer until damage-rect present); keep only the cheap
  whole-sequence frame-skip in B. Note the deferral in Phase 3 rather than computing
  unused work (invariant against per-frame waste). (§Phase 3)
- **`DrawItem` variant constructor qualification** — bare `Circle[…]` (like the exported
  `Mode` enum, verified in `tests/syntax/app/app_mode_surface_valid`) vs qualified
  `canvas::Circle[…]` (the spec's *included* union-member rule shows `extras::Circle[…]`).
  Recommended: confirm which applies to a directly-exported union variant against the
  package/union addressing checker in Phase 1 and make the man-page examples match.
  (surfaced by the plan-98-api.md smiley example)

## Corrections

<Filled in during execution — especially the RES record wiring and payload encoding.>

## Summary

Risk in B concentrates in Phase 4's RES-backend wiring — getting `Image`/`Font` to
close, reclaim, transfer, and double-close exactly like a file resource through the
existing cleanup paths. There is deliberately **no** cross-thread refcount to design:
MFB is not refcounted, `Image`/`Font` are plain RES values with closed-flag lifetime, and
the only OS-side rule (defer the texture free past the GPU frame-drain) lives in D.
Everything else (package, arena, copy, hash, cache) is pure worker-thread logic, fully
testable without a GPU, and is the shippable foundation C renders and D threads.
