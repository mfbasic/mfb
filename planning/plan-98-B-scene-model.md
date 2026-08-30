# plan-98-B: Canvas scene model — union, deep copy, arena, hashing, RES resources

Last updated: 2026-08-30
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

This is **build step 2** of the A–G sequence.

References:

- **plan-98-A** — the feature's top-level design lives in its "Cross-cutting
  invariants" section; invariants 1–4, 6 and 8 are binding here. There is no separate
  design document (plan-98-A … plan-98-G + plan-98-api.md are the whole corpus).
- `planning/plan-98-api.md` — the exact field shapes for every type declared here.
- `.ai/resources-packages.md` — RES resource system + package authoring seams.
- `.ai/collections.md` — `List OF` representation (the scene is a language array).
- `.ai/codegen-invariants.md` — record layout, monomorph, deep-copy patterns.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-98-A complete (Canvas mode enters/tears down) | `ls planning/completed/plan-98-A-*` → hit | NOT MET (A precedes B) |
| Canvas surface handle retrievable in canvas mode | plan-98-A Phase 3 acceptance met | NOT MET |
| The registry can declare unions/records/resources as data | `rg -n "fn add_union\|fn add_record\|fn add_resource" src/codegen/registry/mod.rs` → 3 hits | MET (2026-08-30) |
| Working tree builds | `cargo build` → pass | UNVERIFIED (run before starting) |

> plan-98-A is a precondition, not scope. If A is incomplete, B cannot start, full stop.
> Per A's invariant 8 there is no "full suite green at HEAD" row and no byte-identity
> obligation in this letter; the full suite runs once, at the end of the plan (G).

## 1. Goal

- Register the `canvas::` builtin package on the clean-room registry
  (`src/codegen/builtins/canvas/mod.rs:register` + a `register` call in
  `src/codegen/registry/mod.rs` + `mod.rs:ALL_BUILTIN_PACKAGES` + per-backend
  `runtime_calls`), exposing the closed `DrawItem` UNION type and the
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

- **Package registration seams** (from A's research, re-cited; all verified
  2026-08-30): every builtin package registers itself on the clean-room registry via
  `crate::codegen::builtins::<pkg>::register(&mut r)` in
  `src/codegen/registry/mod.rs:1550-1576` (28 calls today), with the test mirror
  `src/codegen/builtins/mod.rs:921:ALL_BUILTIN_PACKAGES` (26 user-visible names), and
  per-backend `BackendCapabilities.runtime_calls` (`src/target.rs:106`) enforced by
  `src/target/shared/validate/capabilities.rs:7:validate_capabilities`. A new call not
  advertised is a hard compile error. **`canvas` is greenfield on this model** — a new
  `src/codegen/builtins/canvas/` directory with `mod.rs` + one `func_*.rs` per member,
  following `planning/migrate.md`. The registry already carries types as data:
  `add_union`, `add_record`, `add_enum`, `add_resource`
  (`src/codegen/registry/mod.rs:1100-1145`), so `DrawItem`/`Paint`/`Image`/`Font` need
  no new registry machinery. The old `.mfb` package file + `BuiltinModule` +
  `descriptor.rs:REGISTRY` authoring model this plan was first written against no
  longer exists (deleted by `4ed7d60de` / `0bf877510`).
- **`term::` as the present precedent:** `src/codegen/term/core/term.rs:158:lower_term_helper`,
  `term::sync` → `term_grid::emit_grid_present`. The `term::` model is *ambient
  mutation + present-diffs*; canvas deliberately differs (retained scene, content-hash
  cache — a deliberate divergence, decided here); but the deep-copy-before-handoff
  discipline is the same lesson as the live→snapshot copy
  (`term_draw.rs:emit_term_snapshot_copy`).
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
| Existing builtin packages on the registry | 28 registered / 26 user-visible | `rg -c 'crate::codegen::builtins::[a-z_]*::register' src/codegen/registry/mod.rs` → 28; `ALL_BUILTIN_PACKAGES` (`src/codegen/builtins/mod.rs:921`) → 26 (2026-08-30) |
| RES record offsets to reserve for the canvas backend | 4 (`tag@0/handle@8/closed@16/STATE@24`) | `.ai/resources-packages.md` "Adding a NEW native backend" |

### Verified properties

- **A `List OF DrawItem` is a plain language array** — the retained scene is an
  ordinary language value and the geometry cache is runtime-side, keyed on content
  hash, so the array carries no opaque handle. This is a **design decision of this
  plan**, not an external finding; it is what makes `Circle[…]` in a list literal work
  (plan-98-api.md) and it is what forces the content-hash cache (A invariant 2).
  Phase 1 proves the type checks and Phase 3 proves the cache; until then it is a
  decision, not a verified property.
- **Deep copy is mandatory** because the render thread reads the scene at arbitrary
  times after `present()` returns — A invariant 3. Enforced here even though the reader
  is only a test until D. Phase 2's caller-frame-drop test is what actually proves it.
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
   (`mfb.runtime.canvas_scene.v1`, laid out as: `revision`, `itemCount`,
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

**There is no byte-identity gate here** (A's invariant 8). This is new behavior; it is
verified by unit tests over the copy/hash/cache/resource-close logic (pure
worker-thread, GPU-free — which is the point of sequencing this as step 2).

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
- [ ] Read `planning/migrate.md` (the canonical builtin-package authoring procedure)
      before writing any of it. `canvas` is a **new** clean-room package, so it starts
      at the migration playbook's end state — there is no `.mfb` package file and no
      `BuiltinModule`.
- [ ] Create `src/codegen/builtins/canvas/mod.rs` with `pub(crate) fn register(r: &mut
      Registry)` building a `RegistryPackage::new("canvas", …)`, declaring the types as
      registry data: `add_union` for `DrawItem` with the 8 frozen variants (Image,
      Rectangle, Line, Polygon, Circle, Arc, Text, RoundedRect); `add_record` for
      `DrawLayer`, `Paint`, `Color`, `Point`, `Size`, `Bounds`, `TextMetrics`;
      `add_resource` for `Image`/`Font`. Field shapes per plan-98-api.md. Declare union
      variants with **bare** ids (no `pkg::Type` normalization) per
      `.ai/resources-packages.md:24` — this also settles the Open Decision below.
- [ ] One `func_*.rs` per member (`func_present.rs`, `func_present_layers.rs`,
      `func_rgb.rs`, …), each with its own `register(pkg: &mut RegistryPackage)` and a
      `Body`. **Only `Body::abi_inline` and `Body::abi_function` are sanctioned**
      lowering shapes — do not invent a variant. `present`/`presentLayers` and the
      resource calls are OS-seam/heavy work → `abi_function`; `rgb`/`rgba` are pure
      arithmetic on pre-lowered args → `abi_inline`.
- [ ] Add `crate::codegen::builtins::canvas::register(&mut r);` to
      `src/codegen/registry/mod.rs`, add `"canvas"` to
      `src/codegen/builtins/mod.rs:ALL_BUILTIN_PACKAGES`, and advertise the new calls in
      each `--app` backend's `runtime_calls` (`src/target/macos_aarch64/mod.rs:33`,
      `src/target/linux_common/mod.rs`, `src/target/win_x86_64/`).
- [ ] Gate every `canvas::` call on `Mode.Canvas` (reuse the mode-gate seam;
      `canvas::present` in `Console`/`None` traps `ErrWrongMode`, per the mode gate
      in plan-98-api.md).
- [ ] Tests: package imports only in `--app` builds; `canvas::present` traps
      `ErrWrongMode` outside canvas mode; the union type is exhaustively matchable.

Acceptance: a `--app` program imports `canvas::`, and `canvas::present` compiles in
canvas mode and traps `ErrWrongMode` elsewhere; `validate_capabilities` passes on all
backends. Run only `cargo test --bin mfb codegen::builtins::canvas`, the registry
consistency tests (`cargo test --bin mfb codegen::registry`), and the new syntax
fixtures — the registry's own `catalog_is_consistent` / `ALL_BUILTIN_PACKAGES` tests are
the ones a new package can break.
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
wrong pixel count and `getBytes`/`getSize` reflect the shadow. Run only the new
resource tests plus the existing RES-lifecycle targets this touches
(`cargo test --bin mfb resource`, `rg -rln "resource" tests/ | head`).
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
- Acceptance: the per-phase targeted tests above — **no full-suite run and no
  byte-identity check in this letter** (A's invariant 8); fmt.

## Open Decisions

- **`params[]` encoding for variable-length payloads** — recommended: length-prefixed
  contiguous blobs per item so the hash spans a stable byte range. (§Design 2)
- **Compute damage in B or defer to G** — recommended: **defer the bounds-union damage
  to G** (it has no consumer until damage-rect present); keep only the cheap
  whole-sequence frame-skip in B. Note the deferral in Phase 3 rather than computing
  unused work (invariant against per-frame waste). (§Phase 3)
- ~~**`DrawItem` variant constructor qualification**~~ — **RESOLVED 2026-08-30: bare
  `Circle[…]`.** `.ai/resources-packages.md:24` states the rule directly for a new
  native backend: "Declare union variants with BARE ids (no `pkg::Type`
  normalization)." The spec's qualified `extras::Circle[…]` form applies to *included*
  union members, not directly-exported variants. Phase 1 declares them bare and the man
  examples match. (surfaced by the plan-98-api.md smiley example)

## Corrections

**2026-08-30 — pre-execution revision (no code written yet).** See plan-98-A's
Corrections for the full account; applied here:

1. **Package-authoring model replaced.** The `.mfb` package file + `canvas.rs
   BuiltinModule` + `descriptor.rs:REGISTRY` + `mod.rs:ALL_BUILTIN_PACKAGES` recipe this
   plan was written against was deleted by `0bf877510` (2026-08-10) / `4ed7d60de`
   (2026-08-16). `canvas` is now a greenfield clean-room package under
   `src/codegen/builtins/canvas/`, authored per `planning/migrate.md`, with types as
   registry data (`add_union`/`add_record`/`add_resource`, already present at
   `src/codegen/registry/mod.rs:1100-1145`) and `Body::abi_inline`/`Body::abi_function`
   as the only sanctioned lowering shapes. Net effect: **less** work than the plan
   assumed, not more.
2. **Citation remap.** 6 stale `src/builtins/*` / `src/target/shared/code/*` mentions
   repointed; `lower_term_helper` is now `src/codegen/term/core/term.rs:158`. The RES
   record layout claim (`tag@0/handle@8/closed@16/STATE@24`, 96-byte envelope) was
   re-verified against `.ai/resources-packages.md:203` and **still holds** — Phase 4 is
   unaffected.
3. **Byte-identity and full-suite acceptance removed** per A's invariant 8.
4. **Open Decision on variant qualification resolved** (bare ids) from
   `.ai/resources-packages.md:24`.

<Further corrections filled in during execution — especially the RES record wiring and
payload encoding.>

## Summary

Risk in B concentrates in Phase 4's RES-backend wiring — getting `Image`/`Font` to
close, reclaim, transfer, and double-close exactly like a file resource through the
existing cleanup paths. There is deliberately **no** cross-thread refcount to design:
MFB is not refcounted, `Image`/`Font` are plain RES values with closed-flag lifetime, and
the only OS-side rule (defer the texture free past the GPU frame-drain) lives in D.
Everything else (package, arena, copy, hash, cache) is pure worker-thread logic, fully
testable without a GPU, and is the shippable foundation C renders and D threads.
