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
| plan-98-A complete (Canvas mode enters/tears down) | `ls planning/completed/plan-98-A-*` → hit | MET (archived after its Phase 4 landed, `b3bc8e5c2`) |
| Canvas surface handle retrievable in canvas mode | plan-98-A Phase 3 acceptance met | MET — macOS `[view layer]`, GTK `ST_CANVAS_SURFACE` (`gtk_native_get_surface`), Windows `CANVAS_HWND_SYM`; the real GUI enter→exit→re-enter cycle is `scripts/test-macapp.sh` Case 3e, green |
| The registry can declare unions/records/resources as data | `rg -n "fn add_union\|fn add_record\|fn add_resource" src/codegen/registry/mod.rs` → 3 hits | MET (re-run: mod.rs:1100 `add_record`, :1113 `add_union`, :1139 `add_resource`) |
| Working tree builds | `cargo build` → pass | MET (re-run: `Finished `dev` profile`) |

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
- ~~Each item is content-hashed; a runtime-side geometry cache keyed on the hash…~~
  **MOVED to plan-98-C Phase 1** (Correction 18): a cache over a stub generator has
  nothing to cache and its own acceptance is unobservable.
- **Zero-work frame skip:** if the incoming scene is byte-identical to the installed
  one, `present()`/`presentLayers()` return without publishing. Exact rather than
  hashed — both sides are shrink-to-fit copies, so equal content is equal bytes
  (Correction 18).
- `Image`/`Font` are a new **native RES backend** (the existing resource record
  `tag@0 / handle@8 / closed@16 / STATE@24`, `handle@8` = OS-side texture id), owned by
  MFB scope like a file — **no refcount, no generation table** (invariant 4).
  `canvas::createImage` allocates a resource and `canvas::destroyImage` (or scope-drop
  of the owner) sets `closed@16`; there is no separate pending-free flag, because the
  closed flag alone ends a resource's life (Correction 23). `loadImage`, and the whole
  `Font` resource with `loadFont`/`destroyFont`, moved to plan-98-G (Corrections
  20-21). `present()` copies the resource **id** (an integer) into the scene; it
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
- **No geometry generation, and no geometry cache.** Tessellation, stroke expansion
  and text shaping are C/G's. The cache moved with them (Correction 18) rather than
  landing empty here.
- **The `DrawItem` variant set is frozen here and is a breaking change to extend
  later** (invariant 6): **Picture** (renamed from `Image`, Correction 6), Rectangle,
  Line, Polygon, Circle, Arc, Text, RoundedRect.
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

- [x] Read `.ai/resources-packages.md` "Adding a NEW native backend"; confirm the
      canvas resource record reserves `tag@0/handle@8/closed@16/STATE@24` and how close
      dispatch / scope-drop reclaim are wired (`resource_close_function`, LINK thunks).
      Record the offsets/wiring in Corrections. → Read; layout confirmed
      (`RESOURCE_OFFSET_TAG=0 / HANDLE=8 / CLOSED=16 / STATE=24`, 96-byte envelope,
      type-specific tail at 32+, "do NOT store a per-record CLOSE fn ptr at offset 32
      — collides with `FILE_OFFSET_BUF_PTR@32`"). The consequential finding was a
      different sentence in the same doc — resources cannot be record fields — see
      Correction 5.
- [x] Read `planning/migrate.md` (the canonical builtin-package authoring procedure)
      before writing any of it. `canvas` is a **new** clean-room package, so it starts
      at the migration playbook's end state — there is no `.mfb` package file and no
      `BuiltinModule`.
- [x] Create `src/codegen/builtins/canvas/mod.rs` with `pub(crate) fn register(r: &mut
      Registry)` building a `RegistryPackage::new("canvas", …)`, declaring the types as
      registry data: `add_union` for `DrawItem` with the 8 frozen variants (Image,
      Rectangle, Line, Polygon, Circle, Arc, Text, RoundedRect); `add_record` for
      `DrawLayer`, `Paint`, `Color`, `Point`, `Size`, `Bounds`, `TextMetrics`;
      `add_resource` for `Image`/`Font`. Field shapes per plan-98-api.md. Declare union
      variants with **bare** ids (no `pkg::Type` normalization) per
      `.ai/resources-packages.md:24` — this also settles the Open Decision below.
      → Done, with four field-shape corrections the api doc could not have survived
      (Corrections 5–8): the `Image` **variant** is `Picture` (it collided with the
      `Image` resource); `size AS Real` is `Float` (`Real` is not an MFB type);
      `Picture.image`/`Text.font` carry the value handles `ImageRef`/`FontRef`
      (a record field cannot hold a resource); and `Paint`'s three untyped fields are
      pinned as `BlendMode`/`Transform`/`Bounds` under a "**every field's zero value
      is its no-op**" rule. `add_resource` **moved to Phase 4** — see Correction 9.
- [x] One `func_*.rs` per member (`func_present.rs`, `func_present_layers.rs`,
      `func_rgb.rs`, …), each with its own `register(pkg: &mut RegistryPackage)` and a
      `Body`. **Only `Body::abi_inline` and `Body::abi_function` are sanctioned**
      lowering shapes — do not invent a variant. `present`/`presentLayers` and the
      resource calls are OS-seam/heavy work → `abi_function`; `rgb`/`rgba` are pure
      arithmetic on pre-lowered args → `abi_inline`.
      → Phase 1's members are `rgb`, `rgba`, `fill`, `stroke`, `fillStroke`.
      **`Body::mfb`, not `abi_inline`** (Correction 10): they build records from
      records, which is what MFBASIC source expresses directly and what a
      target-generic inline lowering would have to hand-assemble per architecture.
      `Body::mfb` is a sanctioned shape for a public member (it is what puts it in
      `mfb man`); the "only `abi_inline`/`abi_function`" rule is about not inventing a
      *new* lowering variant. `present`/`presentLayers` are Phase 2 (Correction 11).
      The three `fill`/`stroke`/`fillStroke` constructors are **new surface this phase
      had to add** — see Correction 7.
- [x] Add `crate::codegen::builtins::canvas::register(&mut r);` to
      `src/codegen/registry/mod.rs`, add `"canvas"` to
      `src/codegen/builtins/mod.rs:ALL_BUILTIN_PACKAGES`, and advertise the new calls in
      each `--app` backend's `runtime_calls` (`src/target/macos_aarch64/mod.rs:33`,
      `src/target/linux_common/mod.rs`, `src/target/win_x86_64/`).
      → Registered, plus two seams the plan did not list but the consistency tests
      demand: `is_builtin_import` (`src/codegen/builtins/mod.rs:48`) and the spec §18
      package list, which `spec_section_18_package_list_matches_is_builtin_import`
      pins. **No `runtime_calls` advertising yet**: every Phase 1 member is
      `Body::mfb`, which emits no `_mfb_rt_*` call at all, so there is nothing to
      advertise until Phase 2's `present`.
- [x] Gate every `canvas::` call on `Mode.Canvas` (reuse the mode-gate seam;
      `canvas::present` in `Console`/`None` traps `ErrWrongMode`, per the mode gate
      in plan-98-api.md). → The **import** gate lands here: `IMPORT canvas` in a
      console build is now the same compile error `IMPORT app` is
      (`src/cli/build/mod.rs`), which is the stronger of the two gates and the one
      Phase 1's members need. The per-call `ErrWrongMode` gate lands with the first
      surface-touching call in Phase 2 — Phase 1 has none. **`rgb`/`rgba` are
      deliberately exempt from the mode gate** (Correction 12).
- [x] Tests: package imports only in `--app` builds; `canvas::present` traps
      `ErrWrongMode` outside canvas mode; the union type is exhaustively matchable.
      → 6 registry unit tests (frozen variant set incl. order; every variant has a
      record; every variant carries a `paint`; no record/resource name collision;
      handles are plain `Integer`s; the types are builtin types) plus 5 integration
      tests in `tests/cli_canvas_package.rs` that build and *run* a program naming
      every type, every variant, both colour and all three paint constructors, and
      assert the console-build rejection names `canvas`. The `present` trap moves to
      Phase 2 with `present`.

Acceptance: a `--app` program imports `canvas::` and the whole declared type surface
is constructible and runnable; a console build importing it is rejected by name;
`catalog_is_consistent` / `ALL_BUILTIN_PACKAGES` / the spec §18 list all pass.
(**Amended** from "`canvas::present` compiles in canvas mode and traps `ErrWrongMode`
elsewhere" — `present` is Phase 2, Correction 11. Not weakened: the replacement
exercises the full frozen type set at runtime, which the original did not.)
→ MET. `cargo test --bin mfb codegen::builtins::canvas` = 6 passed;
`cargo test --bin mfb codegen::registry` = 32 passed;
`cargo test --bin mfb codegen::builtins::tests` = 20 passed;
`cargo test --bin mfb catalog_is_consistent` = 1 passed;
`cargo test --test cli_canvas_package` = 5 passed.
Rendered: `mfb man canvas`, `mfb man canvas types`.
Commit: d3cd3a0f6

### Phase 2 — Scene arena + transitive deep copy

- [x] Define the runtime scene arena `mfb.runtime.canvas_scene.v1` (fields per the
      design layout) as runtime-owned storage (not caller-frame-scoped).
      → Reserved as a region in the **arena state**, one region past the
      presentation-mode word, on the same pinned arena-state register — the
      established `term_state_offset`/`presentation_mode_offset` pattern, threaded
      through `ArenaLayout` → `AbiCtx` and gated on `uses_canvas` so no program that
      never draws pays for it. Chosen over a writable global because a global must be
      declared per target, while the arena region is already threaded to every
      runtime helper; and it satisfies "outlives the call" because the arena is a
      growing region owned by the execution context, not a frame.
      Layout: `revision@0`, `count@8`, `items@16`, `hashes@24` (the last reserved for
      Phase 3).
- [x] Implement `present()` deep copy: walk the `List OF DrawItem`, copy every
      reachable payload (params, polygon point arrays, text strings, `Paint`) into
      `params[]`. After copy, assert (in tests) no field points into caller memory.
      → **No walk is needed, and writing one would have been a mistake**
      (Correction 13). An MFBASIC collection is already a self-contained flat block —
      strings, records and nested collections are inlined, not referenced — so
      `copy_flat_block` (the codebase's existing deep-copy primitive, shared with
      value-copy semantics and thread transfer) *is* the transitive copy. Its own
      contract says so: "because a flat block has no internal pointers, the byte copy
      **is** a deep copy".
- [x] Publish into a single "live scene" slot (no ring yet); bump `revision`.
      → Published in the order **items → count → revision**, revision last, because
      the revision is what a reader gates on: bumping it first would let a reader see
      a new revision beside the previous frame's pointer. Pinned by test.
- [x] Tests: build a scene referencing caller-frame arrays/strings, `present()`, drop
      the caller frame, and read the published scene back intact — proves the copy is
      transitive and self-contained.
      → **Read-back is not possible yet and the acceptance is corrected accordingly**
      (Correction 14). Nothing can read the published scene until the renderer exists
      (plan-98-D), so a runtime test can only show `present` does not crash — it
      cannot distinguish "copied" from "aliased". Replaced with the checks that do
      discriminate: `tests/rt_canvas_present_deep_copy.rs` asserts on the emitted
      helper that (a) it **allocates**, so it cannot be publishing the caller's own
      block; (b) the publish order is items/count/revision with the revision last;
      (c) the mode gate **precedes** the allocation, so a wrong-mode call cannot
      strand an arena block. Plus the runtime case in `tests/cli_canvas_package.rs`
      presenting a scene built entirely in a dead callee frame.
- [x] **Added task — advertise `canvas.present`.** Phase 1 had no runtime call to
      advertise (every member was `Body::mfb`); `present` is the first, so it is now
      in all three `--app` backends' `runtime_calls`, and `RuntimeHelper::Canvas`
      exists so the helper is `_mfb_rt_canvas_*` rather than falling back to the
      shared `Abi` family (which the `uses_canvas` arena probe would not have seen).
- [x] **Added task — `abi_function` bodies could not raise a message-carrying
      error.** `lower_abi_function_helper` built its `CodeBuilder` with an *empty*
      `string_symbols` map, so any body reaching `raise_error_bare` failed with
      "native code string literal 'Allocation failed.' has no data object". Threaded
      the module's real string table through; see Correction 15.
- [x] **Added task — fixed a pre-existing capability-validation hole this uncovered.**
      A *trapped* runtime call escaped `validate_capabilities` entirely. See
      Correction 16; regression test `tests/rt_trapped_call_capability_gate.rs`.

Acceptance (**corrected, Correction 14** — the original required reading the scene
back, which nothing can do until D): `present` deep-copies the scene into
runtime-owned storage rather than publishing the caller's block, publishes it in an
order no reader can observe half-written, and gates on `Mode.Canvas` before
allocating — all proven on the emitted helper; and a scene built entirely in a frame
that is gone by the time `present` is called installs cleanly at runtime.
→ MET. `cargo test --test rt_canvas_present_deep_copy` = 3 passed;
`cargo test --test rt_trapped_call_capability_gate` = 3 passed (RED-checked);
`cargo test --bin mfb` = 3555 passed. Cross-builds green for `linux-aarch64`,
`linux-x86_64`, `windows-x86_64`.
Commit: 118837f5a (goldens: 7b5330083)

### Phase 3 — Content hashing, geometry cache, frame-skip

- [x] ~~Hash each copied item (flat-union byte hash over `params[]`); store into
      `hashes[]`.~~ — **moved to plan-98-C Phase 1** (Correction 18). The per-item
      hash exists to key the geometry cache; it moves with it. The scene region's
      `hashes` slot is already reserved (`CANVAS_SCENE_HASHES_OFFSET`, B Phase 2), so
      C fills a slot rather than growing the layout.
- [x] ~~Implement the geometry cache … miss inserts with a **stub empty geometry**
      (real generation is C)~~ — **moved to plan-98-C Phase 1** (Correction 18), where
      geometry exists to cache. Its own acceptance ("changing one item invalidates
      exactly one cache entry") is only observable once a miss does work.
- [x] Implement zero-work frame skip: if incoming hash sequence == live scene's hash
      sequence and same length, return without publishing.
      → Landed, and **exactly rather than by hash**: `present` byte-compares the
      shrink-to-fit copy against the installed scene, so equal content is equal bytes
      and there are no collisions. Comparing the *copy* (not the caller's block) is
      what makes it exact — a working buffer carries capacity headroom the installed
      scene does not, so the same content in the two shapes has different bytes.
      The copy still happens on a skipped frame; that is plan-98-A invariant 2, which
      charges the deep copy to the caller's frame budget. What the skip buys is not
      re-publishing, which is what would make the renderer redraw.
- [x] ~~Implement positional diff/damage~~ — **deferred to G**, taking the plan's own
      Open Decision recommendation. It has no consumer until damage-rect present, and
      computing an unused bounds-union every frame is exactly the per-frame waste
      invariant 1 forbids.
- [x] Tests: identical re-present publishes nothing; one changed item regenerates one
      cache entry; LRU eviction fires under a forced small arena; hash sequence compare
      is O(n) and correct across length changes.
      → For what remains in B: `an_identical_re_present_skips_the_publish` asserts on
      the emitted body that the skip path **bypasses the scene-region stores
      entirely** — the substantive claim, since a "skip" that still bumped the
      revision would be a skip in name only. Plus
      `macos_repeated_and_changed_presents_are_sound`, which runs every shape through
      the branch: first present, identical re-present, changed content, back to the
      earlier content, empty both ways, non-empty again. The cache-entry and eviction
      cases move to C with the cache.

Acceptance (**corrected, Correction 18** — the cache half moved to C): re-`present()`
of an identical list takes a path that writes nothing to the scene region, and the
skip/publish branch is sound on first-present, identical, changed, and empty scenes.
→ MET. `cargo test --test rt_canvas_present_deep_copy` = 4 passed;
`cargo test --test cli_canvas_package` = 6 passed.
Commit: 47034488f

### Phase 4 — `Image` as a native RES backend (largest blast radius last)

> **`Font` moved to plan-98-G** (Correction 21). Without `canvas::loadFont` there is
> no way to *construct* a `Font`, so declaring the resource here would have shipped a
> type no program could reach — and `loadFont` needs the font parser G vendors. Tag
> `12` is reserved for it in `error_constants.rs` so the layout does not shift later.

- [x] Add the canvas resource record per the "Adding a NEW native backend" recipe
      (`tag@0/handle@8/closed@16/STATE@24`, texture id in `handle@8`, tag in
      `error_constants.rs`, zero STATE at construction, the `== RESOURCE_OFFSET_*`
      asserts).
      → `src/codegen/builtins/canvas/gen_image.rs`: `RESOURCE_TAG_IMAGE = 11`, tail at
      32+ (`width`, `height`, `pixels`, `dirty`, `lastUsedFrame`), with compile-time
      asserts that the tail starts exactly at `RESOURCE_OFFSET_STATE + 8` and fits the
      96-byte envelope. `lastUsedFrame` is reserved now because D's free gate is
      `closed AND lastUsedFrame < lastCompletedFrame` — a monotonic compare, not a count.
- [x] `canvas::loadImage`/`createImage`/`loadFont` allocate a resource (fallible — return
      per the result ABI: tag in x0, value in x1); the OS-side texture is created by the
      backend (software now; Metal/Vulkan in E/F). `createImage` takes `List OF Byte`
      RGBA8; store the pixels in the image's **CPU shadow** (in STATE).
      → `createImage` landed. **`loadImage` moved to plan-98-G** (Correction 20):
      decoding a real image format needs inflate, which does not exist in the tree
      (`grep -rn "inflate\|deflate" src/codegen/builtins/` → nothing; it is plan-93-A's
      scope), and G already owns the vendored-single-header decision that `stb_image`
      needs. `createImage` is the primitive `loadImage` will feed, so images are usable
      today by any program that generates or parses its own pixels.
      The shadow is its own arena block pointed to from the tail rather than living in
      `STATE@24`: `STATE` is the user-facing `RES … STATE T` slot, and taking it for
      internal storage would collide with a program attaching state to an image.
      `handle@8` is the record's own address — unique, non-zero, and already a real
      identity a backend can adopt as its key, rather than a placeholder id.
- [x] `canvas::destroyImage`/`destroyFont` close the resource (set `closed@16` + release
      path); wire scope-drop reclaim, thread-transfer, and the `resource_close_function`
      / LINK-thunk gating exactly like a file resource.
      → `destroyImage` landed, wired as the resource's `close_function` so scope-drop
      routes to it. Not thread-sendable in v1 (an image belongs to the drawing
      surface's thread), which the descriptor records.
- [x] Image-content ops: `canvas::getBytes(image)` returns the CPU shadow (no GPU);
      `canvas::setBytes(image, pixels)` deep-copies into the shadow + marks the texture
      dirty, **fallible** `ErrBadPixelCount` when `len(pixels) != width*height*4`;
      `canvas::getSize(image)` returns the dimensions. The GPU upload of a dirty texture
      and the "in current scene → redraw" trigger are D's job; B only updates the shadow +
      dirty flag.
      → All three landed, plus `canvas::imageRef` (Correction 5). `getBytes` **copies**
      the shadow rather than returning it: collections are values, so handing back the
      runtime's own block would let a caller mutate the image behind its back and would
      alias storage `setBytes` later replaces. `setBytes` writes the new block pointer
      **before** the dirty flag, so a reader that sees dirty is guaranteed to see the
      new pixels. `ErrBadPixelCount` is a new error code (`7-705-0021`), registered in
      the `errorCode` package and the spec table.
- [x] Color helpers `canvas::rgb`/`rgba` build `Color` (clamp components 0..255).
      → Landed in Phase 1.
- [x] `present()` copies only the resource **id** into the scene — **no** refcount work.
      → True by construction: a `Picture` carries an `ImageRef`, a one-field record
      holding an `Integer`, so there is no resource in a scene to count.
- [x] Mark-pending-free on close (a runtime-side flag the graphics thread reads in D);
      B does **not** free the OS texture — that is D's `closed AND lastUsedFrame <
      lastCompletedFrame` gate.
      → The closed flag **is** that marker — invariant 4 is explicit that there is no
      separate pending-free flag ("the closed flag alone ends a resource's life"), and
      adding one would be a second source of truth for the same fact. `destroyImage`
      sets it; `lastUsedFrame` is reserved in the record for D's half of the compare.
- [x] Tests: load→present→destroy closes the resource and marks the texture pending-free;
      scope-drop of an un-destroyed image closes + reclaims the record; using a closed
      image is `ERR_RESOURCE_CLOSED`; double-close is the defined no-op; `setBytes` with a
      wrong-length list returns `ErrBadPixelCount`; `getBytes` round-trips the CPU shadow;
      `getSize` matches `createImage`.
      → `tests/cli_canvas_image_resource.rs`: one program covering create → getSize →
      getBytes → imageRef → setBytes → getBytes → present-with-handle, plus the three
      error contracts, each with its own exit code. Cross-built for `linux-aarch64`,
      `linux-x86_64` and `windows-x86_64` so every backend's advertising is checked.
      NOTE: `presentLayers` is covered by `cli_canvas_package.rs` — see Correction 27.
      **The double-close and use-after-close cases are NOT written from source**
      (Correction 22): the compiler rejects both statically as `TYPE_USE_AFTER_MOVE`,
      which is stronger than the runtime no-op. `ErrResourceClosed` is reached the way
      it actually occurs — closing through a `RES` parameter, where ownership floats up
      and the checker cannot see it.
- [x] **Added task — verify the man examples compile** (Correction 25). Nothing in the
      tree did, and three of this letter's examples did not.
      `tests/cli_canvas_man_examples_compile.rs` reads each `canvas::` member's example
      back out of `mfb man` (so it cannot drift from what a user is shown) and compiles
      all 12.

Acceptance (**corrected** — `Font` and `loadImage` moved, Corrections 20–21): canvas
`Image` create/close/scope-drop exactly like a file resource; `present` does zero
refcount work; a closed image raises `ErrResourceClosed` on every read; `setBytes`
rejects a wrong pixel count and `getBytes`/`getSize` reflect the shadow; every `--app`
backend advertises the surface.
→ MET. `cargo test --test cli_canvas_image_resource` = 3 passed (incl. the headless
runtime contract and all three cross-target builds);
`cargo test --bin mfb codegen::builtins::canvas` = 6 passed. Rendered: `mfb man canvas`
lists all 12 members, `mfb man canvas types` lists `canvas::Image`.
Commit: 8c2ebb103

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
- Doc sync: **DONE.** `src/docs/spec/app/06_canvas.md` is the new spec topic (retained
  vs immediate and what each consequence buys; the deep-copy rule and *why* it is
  soundness rather than convenience; `Paint` as a value with the zero-is-no-op rule,
  including the non-obvious all-zero-`Transform`-is-identity definition; the
  coordinate/angle convention and why it is stated; images named rather than embedded,
  and content-vs-scene orthogonality; the mode gate and its documented exemption),
  listed in `spec.md`'s reading order and cross-linked both ways. Man pages ship from
  the registry descriptors for all 12 members — and, per Correction 25, every one of
  their examples is now compile-verified (13 members, including `presentLayers` —
  Correction 27 caught it missing). `loadImage`/`loadFont`/`destroyFont`/`measureText`
  are not documented here because they are not in this letter (Corrections 20–21).
  Verified: `mfb spec app canvas`, `mfb man canvas`, `mfb man canvas types`,
  `cargo test --bin mfb docs::` (26 passed — `spec_links_resolve` caught a wrong
  cross-link on the first pass), `cargo test -p mfb --bins citations_resolve`.
- Acceptance: the per-phase targeted tests above — **no full-suite run and no
  byte-identity check in this letter** (A's invariant 8); fmt.

## Open Decisions

- ~~**`params[]` encoding for variable-length payloads**~~ — **MOOT (Correction 13):
  there is no `params[]`.** The concern was that a hash needs a stable, contiguous
  byte range. An MFBASIC collection already *is* one: strings, records and nested
  collections are inlined into the block, not referenced from it, which is why one
  `copy_flat_block` is the whole transitive copy. Phase 3's hash spans those bytes
  directly. (§Design 2)
- ~~**Compute damage in B or defer to G**~~ — **RESOLVED: deferred to G** (the
  recommendation, taken). It has no consumer until damage-rect present, and computing
  an unused bounds-union every `present` is exactly the per-frame waste plan-98-A
  invariant 1 forbids. B keeps the whole-scene frame skip, which is real now.
  (§Phase 3, Correction 19)
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

**2026-08-30 — during execution.** Phase 1 found four places where
`plan-98-api.md`'s field shapes cannot be built as written. None touches the design;
all four are corrected here **and in `plan-98-api.md`**, so the corpus does not keep a
false surface.

5. **A record field cannot hold a resource, so `Text[… font AS Font …]` and
   `Image[… image AS Image …]` are unbuildable.** Verified directly, both spellings:
   `TYPE Holder / handle AS File` → `SYMBOL_UNKNOWN_TYPE` ("Type `File` is not a
   built-in or top-level project type" — a resource is not a value type), and
   `handle AS RES File` → `MFB_PARSE_INVALID_IDENTIFIER` (`RES` does not parse in a
   field position at all). `.ai/resources-packages.md` says the same thing from the
   other side: resources live in collections, not in records.

   **Resolution, which preserves every invariant rather than working around one:**
   the two variants carry a plain value handle — `ImageRef`/`FontRef`, a one-field
   record wrapping the backend's `Integer` id — obtained from the owning resource
   with `canvas::imageRef`/`fontRef` (Phase 4, with the resources). This is *exactly*
   the model plan-98-A invariant 4 and plan-98-api.md already state — "the backend
   owns the one real copy, **MFB holds only the id**", "`present()` copies the
   resource **id** (an integer) into the scene" — so the correction makes the type
   surface match the design instead of contradicting it. `Image`/`Font` stay RES
   resources with closed-flag scope-drop lifetime (invariant 4 intact), and because a
   handle is an `Integer` a published scene provably retains nothing.

6. **The `DrawItem` variant `Image` collided with the `Image` resource.** Two types of
   the same name in one package are unresolvable. The **variant** is renamed
   `Picture` (not the resource: `loadImage`/`createImage`/`destroyImage`/`getBytes`/
   `setBytes`/`getSize` all read on `Image`, and `Picture` sits naturally beside
   `Rectangle`/`Circle`/`Arc` as a shape noun). The frozen set is therefore
   **Picture, Rectangle, Line, Polygon, Circle, Arc, Text, RoundedRect**;
   `no_record_shares_a_name_with_a_resource` pins that no future addition re-collides.

7. **MFBASIC named construction does NOT default unset fields**, so
   `Paint[fill := yellow]` is impossible. Measured:
   `TYPE_CONSTRUCTOR_ARITY_MISMATCH` — "Constructor `Paint` has 1 argument(s),
   expected 6". plan-98-api.md asserted the opposite ("MFB named construction already
   defaults unset fields (spec §4 `Circle[radius := 10.0]`), so this holds") — that
   reading is wrong: the spec's `Circle` has exactly **one** field (`radius AS
   Float`, `04_types.md:150`), so `Circle[radius := 2.0]` is a *complete*
   construction, not a partial one.

   This mattered because the whole `Paint` ergonomic story rested on it.
   **Resolution:** three constructors — `canvas::fill(color)`,
   `canvas::stroke(color, width)`, `canvas::fillStroke(fill, stroke, width)` — which
   write the no-op zeros for the fields the caller did not name, with `WITH` for the
   advanced ones (`WITH canvas::fill(red) { blend := BlendMode.Add }`). This is the
   same reason `rgb`/`rgba` exist rather than raw `Color` field construction, and the
   result reads better than the impossible form did:
   `paint := canvas::stroke(green, 14.0)` vs `paint := Paint[stroke := green,
   strokeWidth := 14.0]`.

8. **`Real` is not an MFBASIC type.** plan-98-api.md types `Text.size` and
   `measureText`'s size as `Real`; `grep -rn "\bReal\b" src/docs/spec/` returns
   **nothing** and `ParameterType` has no such variant. Corrected to `Float`
   throughout.

   Also pinned here, because the api doc left them untyped: `Paint.blend AS
   BlendMode` (a new 4-variant enum: Normal, Multiply, Screen, Add),
   `Paint.transform AS Transform` (a new 2×3 affine record), `Paint.clip AS Bounds`.
   All three obey one stated rule — **every `Paint` field's zero value is that
   field's no-op** — which is what makes the constructors above behave obviously. It
   forces one non-obvious definition, recorded on the type: **the all-zero
   `Transform` means the identity**, not the degenerate matrix that collapses every
   point to the origin. Defining it the other way would make an unset transform erase
   the drawing.

9. **`add_resource` cannot precede its close member, so `Image`/`Font` move to Phase
   4.** `registry::runtime_specs` **derives a runtime call from a resource's
   `close_function`**, so declaring the resources in Phase 1 produced calls nothing
   implements: `catalog_is_consistent` failed with "misrouted calls:
   [canvas.destroyImage: None (expected Some(Canvas)), canvas.destroyFont: …]". The
   resources therefore land in Phase 4 beside `destroyImage`/`destroyFont`, which is
   also where AGENTS.md's no-stubs rule puts them. Nothing in the frozen `DrawItem`
   set depends on this — Correction 5 already replaced the resource-typed fields with
   value handles.

10. **`Body::mfb`, not `abi_inline`, for the value constructors.** The plan said
    `rgb`/`rgba` are "pure arithmetic on pre-lowered args → `abi_inline`". They are
    not arithmetic — they build a **record** from clamped components, and `fill`/
    `stroke`/`fillStroke` build a record containing three more records. A
    target-generic inline lowering would have to hand-assemble record layout per
    architecture; MFBASIC source expresses it directly, and `Body::mfb` is the
    sanctioned shape for a public member (it is what puts the member in `mfb man`).
    The "only `abi_inline`/`abi_function`" rule is about not inventing a *new*
    lowering variant, which this does not.

11. **`present`/`presentLayers` moved from Phase 1 to Phase 2.** Phase 1 has no scene
    arena, so a `present` landed here could only have been a no-op that returns OK —
    a stub, which AGENTS.md forbids shipping. Phase 2 lands it complete, with the
    arena and the deep copy, in one piece. Phase 1's acceptance line was amended
    accordingly (and strengthened: it now requires the whole frozen type set to be
    constructible *and runnable*, which the original did not).

12. **`rgb`/`rgba` are deliberately exempt from the `Mode.Canvas` gate.**
    plan-98-api.md states the gate as "every `canvas::` call requires `Mode.Canvas`".
    Applied literally that would trap a call that touches no surface at all — these
    two only build a `Color` value — and would stop a program computing a palette
    before it presents anything, buying no safety. The gate exists so a call cannot
    "touch (or block on) an absent grid/input pipe"; a value constructor touches
    nothing. There is direct precedent: `io::readByte` sits outside the gated set
    while its three siblings are in it. Documented in `MODULE_DESC` and on both
    members, so the exemption is visible where a user reads it.

**2026-08-30 — Phase 2.**

13. **The transitive deep copy is one call, not a per-variant walk — and the Open
    Decision on `params[]` encoding is moot.** The plan's design §2 has `present`
    walking the list and copying "every reachable payload … into `params[]`", with an
    Open Decision on how to encode variable-length payloads so the hash spans a
    stable byte range. Neither is needed: **an MFBASIC collection is already a
    self-contained flat block.** Strings, records and nested collections are inlined
    into it, not referenced from it, which is exactly why `copy_flat_block` — the
    codebase's existing deep-copy primitive, shared with value-copy semantics and
    `thread::transfer` — states outright that "because a flat block has no internal
    pointers, the byte copy **is** a deep copy".

    So `present` calls it once. Writing a bespoke walk would have duplicated a
    load-bearing primitive with a second copy of its layout knowledge, which is the
    kind of divergence that goes wrong silently. It also means Phase 3's hash can span
    the copied bytes directly, since they are already contiguous and pointer-free —
    the Open Decision's whole concern.

14. **Phase 2's acceptance required reading the published scene back, which nothing
    can do until plan-98-D.** There is no reader — that is the renderer, and D builds
    it. A runtime test can therefore show only that `present` does not crash; it
    cannot tell a copy from an alias, so "no dangling pointer into caller memory
    (test-verified)" was not testable as written.

    Corrected to checks that actually discriminate, on the emitted helper:
    - **It allocates.** Publishing the caller's own pointer would be cheaper, would
      pass every runtime test that exists, and would hand the renderer a pointer into
      storage the program may reuse the instant `present` returns. `_mfb_arena_alloc`
      being called is the discriminating fact.
    - **The publish order is items → count → revision.** The revision is what a
      reader gates on; bumping it first would expose a half-written scene.
    - **The mode gate precedes the allocation.** A gate placed after it would strand
      an arena block on every wrong-mode `present` — and the program would still
      behave correctly, so nothing else would catch it.

    The transitivity of the copy itself is *inherited* from `copy_flat_block`, which
    is already covered by the existing value-copy and thread-transfer tests; what
    Phase 2 newly claims is that `present` uses it and publishes the copy, which is
    what the above proves.

15. **`abi_function` bodies could not raise a message-carrying error.**
    `lower_abi_function_helper` constructed its `CodeBuilder` with
    `string_symbols: HashMap::new()` — an empty table — so any body reaching
    `raise_error_bare` died at codegen with "native code string literal 'Allocation
    failed.' has no data object". `present` is the first `abi_function` body to
    allocate, so it is the first to hit it.

    Fixed at the seam rather than worked around in `present`: the module's real
    string table is now threaded into `lower_abi_function_helper`, so **every**
    `abi_function` body can raise, not just this one. Two further pieces the raise
    path needs and that a runtime helper cannot pull in by itself: `canvas.present`
    now registers `ErrWrongMode`/`ErrOutOfMemory` in the data-object pass, and it
    forces the `_mfb_str_empty` sentinel via `module_requires_empty_string_constant`
    — the same override the recursive-transfer copy functions already use, for the
    same reason (the requirement comes from the *helper*, not from anything in the
    program's own ops).

16. **A trapped runtime call escaped capability validation entirely — pre-existing,
    and the common case.** Found because `canvas::present` built fine in one test
    program and was correctly rejected in another. The difference was `TRAP`.

    The TRAP desugar emits `NirValue::CallResult { target: "canvas.present", … }`,
    not `NirValue::RuntimeCall`, and `collect_runtime_calls_from_value` walked a
    `CallResult`'s **arguments only**, never its target. So on any backend that does
    not advertise a call, the bare form was rejected and the trapped form was
    silently accepted — and since a program almost always traps a fallible call, the
    trapped form is how the code is normally written. The result was a binary emitted
    for a backend with no implementation behind the call: precisely what
    `validate_capabilities` exists to prevent.

    Fixed with the predicate the sibling pass (`runtime::usage::push_value_helpers`)
    already used, so the two agree by construction. **Both halves of the predicate are
    load-bearing**: the first attempt collected any target `helper_for_call`
    recognized, which swept in the bare-named `general` family (`toString`, `toInt`)
    — those appear in no backend's `runtime_calls`, so every program trapping a
    conversion started failing validation (caught by
    `builtin_codegen_corpora_lower_in_process`). Requiring a package-qualified name
    as well fixes that, since every `runtime_calls` entry is `pkg.member`.

    RED-checked: with the collection disabled, the trapped-form test fails while the
    bare-form premise still passes. `tests/rt_trapped_call_capability_gate.rs` covers
    all three cases — supported+trapped builds, general+trapped builds, and
    unsupported+trapped is rejected exactly as unsupported+bare is (with the premise
    asserted, so it cannot pass vacuously if that call is ever advertised).

17. **A full-suite run after Phase 2 found four golden drifts, all mine and all
    intended.** Run early (not at G's closeout) because Phase 2 changed a tree-wide
    validation seam. Attribution was measured, not assumed: a baseline binary built
    from `main` via `git archive` reproduces the committed golden exactly
    (`e91927b1…`) while the current one does not, so the drift is this branch's.
    - **byte-identity fs/http/thread** — a structural diff of the two plans shows the
      *sole* difference is the `_mfb_str_error_wrong_mode` data object growing
      224 → 376 bytes: plan-98-A Phase 2's `ErrWrongMode` message rewrite. No
      instruction changed. Those three fixtures embed the error-message table.
    - **syntax/app/app_mode_surface_valid `.ir`** — two lines: the `Mode` enum gains
      `Canvas` (plan-98-A Phase 1).
    - **syntax/app/macos-app-mode-{io,plumbing,term}** — the `windows-x86_64` golden
      only, from Phase 3/4's new wndproc arms. macOS is unchanged because those
      fixtures are `Console`-default, so no reconcile helpers are emitted for them —
      which is the "a `Console`-default program keeps its exact function set"
      property holding rather than a gap.
    - **`rt_gtk_term_utf8_grid`'s derived GTK state size**, +16 for the two Phase 3
      slots. That test sums the state's enumerated members precisely so adding one is
      a deliberate edit; its comment already records two earlier extensions handled
      the same way, and the bug-203 assertion (char grid at 4 B/cell) is untouched.

    Regenerated per AGENTS.md ("a churn from a correct change means regenerate the
    golden") with `sync-goldens.sh`, `regen-ncodesum.sh` and — the one the first two
    do not sweep — `regen-outside-ncode.sh`. `regen-ncodesum.sh` refreshed all 117
    goldens and exactly the 15 above changed, which independently confirms the blast
    radius.

**2026-08-30 — Phase 3.**

18. **The per-item hashing and the geometry cache moved to plan-98-C Phase 1; the
    frame skip stayed and became exact.** The plan had the cache landing here over a
    "stub empty geometry" generator. That is a cache whose every entry is a
    zero-length vertex range: real code, no content, and — decisively — its own
    acceptance is unobservable. "Changing one item invalidates exactly one cache
    entry" and "eviction is `lastUsedRev`-ordered under arena pressure" cannot be
    demonstrated when a miss does no work and an entry occupies no bytes. AGENTS.md
    forbids shipping a placeholder, and building the cache empty here would mean
    re-shaping its keying and sizing in C when real vertex data arrives.

    They move **one letter**, to the phase that first generates geometry — not out of
    the plan. C's Phase 1 now carries both tasks, its Prerequisites row for the
    "generation hook" is marked N/A (there is no cross-letter hook left to check), and
    its acceptance gained the cache claim. The scene region's `hashes` slot stays
    reserved here, so C fills a slot rather than growing the layout.

    What B keeps is real without them: the **whole-scene** frame skip. And it is
    better than the planned hash comparison — `present` byte-compares the
    shrink-to-fit copy against the installed scene, so equal content is equal bytes
    and there are no collisions to reason about. Comparing the *copy* rather than the
    caller's block is what makes that exact: a working buffer carries capacity
    headroom that the installed scene does not, so identical content in the two shapes
    has different bytes and a naive comparison would never match.

19. **Damage computation deferred to G**, taking the plan's own Open Decision
    recommendation rather than overriding it. It has no consumer until damage-rect
    present, and computing an unused bounds-union on every `present` is precisely the
    per-frame waste plan-98-A invariant 1 exists to prevent.

**2026-08-30 — Phase 4.**

20. **`canvas::loadImage` moved to plan-98-G: decoding needs inflate, which does not
    exist.** `grep -rn "inflate\|deflate" src/codegen/builtins/` returns **nothing**,
    and building one is plan-93-A's scope, not plan-98's. Every real image format
    worth loading (PNG certainly) is compressed, so `loadImage` cannot be honest here.

    It moves to **G**, which already owns the vendored-single-header decision for
    `stb_truetype` — the same decision, and `stb_image` is its sibling. G's Phase 1
    now carries `loadImage` and its policy task says explicitly that the decision
    gates images as well as fonts.

    Nothing is lost meanwhile: `canvas::createImage` — the primitive `loadImage` will
    ultimately feed — landed here, so any program that generates or parses its own
    pixels can use images today.

21. **The `Font` resource moved to plan-98-G with `loadFont`.** Without `loadFont`
    there is no way to *construct* a `Font`, so declaring the resource in B would have
    shipped a type no program could reach — surface that exists only to be listed.
    `destroyFont`/`fontRef` go with it, since they operate on a thing that cannot yet
    exist. `RESOURCE_TAG_FONT = 12` is reserved in `error_constants.rs` (as a comment,
    not a dead constant) so the tag space does not shift when G lands.

    `Image` is unaffected and complete: create, destroy, `imageRef`, `getSize`,
    `getBytes`, `setBytes`.

22. **Two of Phase 4's planned tests are unwritable from source, because the compiler
    is stricter than the runtime contract.** "double-close is the defined no-op" and
    "using a closed image is `ErrResourceClosed`" are both rejected statically as
    `TYPE_USE_AFTER_MOVE` ("Binding `img` was moved and cannot be used again") when the
    close is visible in the same scope — a *stronger* guarantee than the runtime
    behaviour they were meant to check.

    So the runtime guard is tested the way it is actually reached: closing through a
    `RES` parameter, where ownership floats up to the caller's binding and the checker
    cannot see the close. That is the real path, and the test exercises it.

23. **Two design choices worth recording, both about not taking the obvious slot.**
    - The CPU pixel shadow is an arena block pointed to from the record's **tail**,
      not stored in `STATE@24`. `STATE` is the user-facing `RES … STATE T` slot; taking
      it for internal storage would collide with a program attaching its own state to
      an image.
    - There is **no separate pending-free flag**, though Phase 4 listed one. Invariant
      4 is explicit that "the closed flag alone ends a resource's life"; a second flag
      meaning the same thing is a second source of truth to keep in sync.
      `destroyImage` sets `closed@16`, and `lastUsedFrame` is reserved in the record
      for D's half of the `closed AND lastUsedFrame < lastCompletedFrame` compare.

24. **`getBytes` copies the shadow; `setBytes` orders its two stores.** Neither is
    incidental. Returning the runtime's own block would let a caller mutate an image
    behind its back *and* would alias storage `setBytes` later replaces. And `setBytes`
    writes the new pixel-block pointer **before** the dirty flag, so a reader that
    observes dirty is guaranteed to see the new pixels rather than the old ones — the
    same publish-ordering discipline `present` uses for the revision.

25. **Nothing verified that a `mfb man` example compiles, and three of the ones this
    phase wrote did not.** Found by hand: `LET img AS Image` (a resource is bound with
    `RES` and named **package-qualified**, `RES img AS canvas::Image`), and a list
    literal spanning source lines (`MFB_PARSE_UNEXPECTED_STATEMENT`). A man example is
    the recommended way to use a call, so a broken one is worse than none.

    `grep`ing the tree found no existing check, so `tests/cli_canvas_man_examples_compile.rs`
    now reads each `canvas::` member's example back **out of `mfb man`** — so it cannot
    drift from what a user is shown — and compiles it. It immediately caught a third
    (`canvas::rgb`'s, which still used the impossible `Paint[fill := …]` form from
    Correction 7).

    Scoped to `canvas` deliberately: a tree-wide version is obviously desirable but
    the rest of the corpus predates any such check and would need its own audit, which
    is not plan-98's scope. Recorded here so that audit has a starting point.

26. **Adding an error code broke a count assertion, and the fix made the assertion
    stronger.** `table_has_no_duplicate_names_or_codes` ended with
    `assert_eq!(names.len(), 45)` over the comment "the migration reproduces every
    legacy row". A bare total cannot distinguish "45 legacy rows" from "44 legacy rows
    plus one addition", so bumping it to 46 would have silently retired the very
    property it was pinning. It now subtracts an explicit `ADDED_SINCE_MIGRATION` list
    (asserted present) from the total, so the legacy-row claim stays checkable and each
    later addition is a visible, justified line.

27. **`canvas::presentLayers` was in this letter's Goal and very nearly shipped
    missing.** It is named in §1, in the Compatibility list, and in Phase 1's task
    ("`func_present.rs`, `func_present_layers.rs`, …"), but Phase 1 moved `present` to
    Phase 2 and `presentLayers` silently fell out — no unticked box remained to catch
    it, because the box that named it had been ticked for the members that *did* land.
    Caught while writing the doc-sync line, by noticing a reference to a correction
    number that did not exist yet.

    Implemented, and the fix improved `present` rather than duplicating it. Both calls
    now share `gen_present::emit_publish`, parameterised by a `SceneShape`; they differ
    **only** in the element type copied and which pair of scene slots is written. Every
    correctness property — the mode gate before the allocation, the transitive copy,
    the exact frame-skip comparison, the revision-last store ordering — is one piece of
    code, so the two cannot drift apart.

    A scene is exactly one shape at a time: publishing clears the *other* shape's
    pointer and count, so a reader decides with a single test (`layers != 0`) rather
    than carrying a discriminant that could disagree with the pointers. Switching
    shapes therefore always publishes, which is correct — the scene really did change.
    The considered alternative — making a flat `present` wrap its list in a
    one-element layer so there is only one published shape — was rejected: it puts an
    allocation and a copy on the common path to simplify the rarer one.

    The scene region grew two slots (`layers@32`, `layerCount@40`); offset 24 stays
    held open for C's per-item hashes.

    Tested by `macos_present_layers_and_shape_switching_are_sound`, covering exactly
    the transitions the shared body cannot get right by construction: trap outside
    canvas mode, first publish, identical re-publish, changed content, **layered →
    flat → layered**, and an empty layer list both ways.

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
