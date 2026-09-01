# plan-116-I: `Picture` and `Text` hold `RES canvas::Image` / `RES canvas::Font` directly

Last updated: 2026-09-01
Effort: large (3h–1d)
Depends on: plan-116-H (series order); plan-114 A–E (landed 2026-09-01)

plan-114 made a `RES` record field legal source: the field holds a copy of the one
handle pointer, copying the record aliases the same resource, and ownership floats
to the record binding's scope (`mfb spec language resource-management` §15, §15.6).
`canvas` predates that: `Picture` names its image through the `ImageRef` value
handle and `Text` its font through `FontRef`, each a one-field record wrapping the
backend's integer id, minted by `canvas::imageRef` / `canvas::fontRef`
(`mod.rs:398-423`, `func_image_ref.rs`, `func_font_ref.rs`).

This letter migrates both, **by user direction (2026-09-01)**: `Picture.image`
becomes `RES canvas::Image`, `Text.font` becomes `RES canvas::Font`, and the
`ImageRef`/`FontRef` records and the `canvas::imageRef`/`fontRef` members are
**removed**.

Behavioral outcome: a program writes `Picture[…, image := img, …]` with its
`RES canvas::Image` binding directly — no handle-minting call — and the scene
renders exactly as before; destroying an image or font a published scene still
names keeps drawing that item as nothing (today's semantics, §4.2); and
`canvas::imageRef`, `canvas::fontRef`, `canvas::ImageRef` and `canvas::FontRef` no
longer exist anywhere in `mfb man canvas` output.

References:

- `mfb spec language resource-management` §15 ("A record field may hold a
  resource…") and §15.6 — the aliasing, float-up and thread-plane rules this letter
  builds on.
- `planning/completed/plan-114-B/C/D/E` — layout, escape-record edges, the lifted
  ban, and record export.
- `src/codegen/builtins/canvas/gen_image.rs:1-15` — the `Image` resource record:
  `handle@8` **is** the backend id; `closed@16` the destroy flag.
- `.ai/canvas-threading.md` §7 — the closed-flag/deferred-free model this letter
  must preserve, and whose last paragraph it rewrites.
- `.ai/resources-packages.md` — builtin-package authoring seams.
- bugs/bug-484 — `Picture` has no renderer today; this letter changes its field
  type, not its (absent) rendering.

## Prerequisites

See plan-116-A §Prerequisites for the three environment gates.

| Must be true | Command | Status |
|---|---|---|
| plan-116-H complete and archived | `ls planning/completed/plan-116-H-*` → one match | NOT MET |
| plan-114 A–E complete and archived | `ls planning/completed/plan-114-*` → 5 matches | MET (2026-09-01) |
| A union variant record may carry a `RES` field, and `List OF <that union>` compiles | the probe program below (§2) | MET (2026-09-01, probe-compiled) |

If plan-116-H is not complete, this letter cannot start, full stop — the series is
strictly ordered and H is the last renderer letter before the type surface moves.
(Technically this letter touches none of A–H's mechanisms; it sits here so the
breaking type change and plan-116-J's ownership semantics land adjacently and are
tested against the finished renderer.)

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command before you continue and again before you stop.

## 1. Goal

- `Picture.image` is `RES canvas::Image`; `Text.font` is `RES canvas::Font`.
- The `ImageRef` and `FontRef` records, and the `imageRef`/`fontRef` members, are
  removed from the registry, the docs, and every seam that names them.
- Every renderer behavior is preserved: text draws/measures through a live font
  exactly as today, and a destroyed font or image in a still-installed scene
  renders that item as empty/nothing — no raise, no crash.
- `mfb man canvas --all` renders with zero mentions of the removed surface.

### Non-goals (explicit constraints)

- **No ownership change.** `present` and `setGroup` still take ownership of
  nothing; the program's `RES` binding (or whatever scope §15.6's float rule picks)
  owns the resource. Ownership by groups is plan-116-J.
- **No `Picture` rendering.** A `Picture` draws nothing today (bug-484 —
  `__canvas_headerFor` gives it an empty `NONE` header and no draw path exists);
  this letter migrates its field type and leaves bug-484 to its own fix.
- **No `sendable`/`live_slots` change on `Image`/`Font`** (`mod.rs:744-748`,
  `:765`). They stay `sendable: false`; the transfer-audit question belongs to
  plan-116-J Phase 1. Consequence, documented not softened: a `DrawItem` list
  containing a `Picture`/`Text` cannot cross a thread data plane
  (`2-203-0138 TYPE_THREAD_RESOURCE_PLANE_REQUIRED`) — where the old integer
  handles could. §Compatibility.
- **No STATE clause on either field.** `Image`/`Font` carry no STATE; the fields
  are bare `RES` slots.
- **No existing golden may move.**

## 2. Current State

### The two handles and who touches them

- `ImageRef`/`FontRef` records: `mod.rs:398-423`; minting members:
  `func_image_ref.rs` / `func_font_ref.rs`, each reading `handle@8` behind a
  closed-guard that **raises `ErrResourceClosed`** on a destroyed resource.
- The renderer's only resource reads are `t.font.id` — 5 sites, all in
  `helper_geometry.rs` (`grep -n 't\.font\.id' src/codegen/builtins/canvas/` →
  `:367,:401,:422,:646,:717` at last read) feeding `__canvas_fontBlob`/glyph
  lookups by integer id. **Nothing reads `pic.image` anywhere** (bug-484).
- Seam registrations naming the members:
  `src/codegen/memory/data/data_objects.rs:252` (`"canvas.imageRef"`), `:274`
  (`"canvas.fontRef"` in the font force-emit list),
  `src/codegen/engine/analysis/module_analysis.rs:47` (`"canvas.imageRef"`).
  These are the force-emit/analysis pairings the
  `adding-a-call-to-an-existing-native-pkg` memory warns about — on removal they
  must be deleted or `catalog_is_consistent`-class tests fail.
- The pinning test `resource_handles_are_plain_integer_values` (`mod.rs:984`)
  asserts exactly the design this letter retires.

### Measured populations (2026-09-01)

| What | Count | Command |
|---|---|---|
| Files naming `imageRef`/`fontRef`/`ImageRef`/`FontRef` | 22 | `grep -rln 'imageRef\|fontRef\|ImageRef\|FontRef' --include='*.rs' --include='*.mfb' src/ tests/ examples/` |
| `Picture[` construction sites (code + doc examples) | 7 | `grep -rn 'Picture\[' --include='*.rs' --include='*.mfb' src/ tests/ examples/` |
| `Text[` construction sites | 12 | same grep, `Text\[` |
| Renderer reads of `t.font.id` | 5 | `grep -n 't\.font\.id' src/codegen/builtins/canvas/helper_geometry.rs` |
| Renderer reads of `pic.image` | 0 | `grep -rn 'pic\.image' src/codegen/builtins/canvas/` (bug-484) |
| Fabricated zero-handle uses (`ImageRef[id := 0]`, `FontRef[id := …]`) | 3 | `tests/cli_canvas_package.rs:54,55`, `tests/rt_canvas_font.rs:634` |

Re-run every row at Phase 1 start — the series letters before this one add sites
(plan-116-D touched the same fixture files and counts have moved once already).

### Verified properties

- **A union variant record may carry a `RES` field, and a `List OF` that union
  compiles and appends.** Probe-compiled 2026-09-01 (macos-aarch64, `mfb build` of
  an `--app`-less executable):

  ```
  TYPE Holder
    x AS Float
    handle AS RES fs::File
  END TYPE
  UNION Thing
    Holder
    Plain
  END UNION
  ' RES f = fs::openFile(...); Holder[x := 1.0, handle := f]; append to List OF Thing — builds.
  ```

- **`handle@8` is the backend id, and the resource record's address is stable for
  the thread's lifetime.** `gen_image.rs:1-15` (the id), and `mfb spec` §15: *"The
  record itself is retained until the thread's arena is torn down"* — which is what
  makes a graphics-thread read through a published pointer safe (§4.2). The
  published scene blocks are themselves worker-arena memory the graphics thread
  already reads (`.ai/canvas-threading.md` §3), so this adds no new cross-thread
  class.
- **`copy_flat_block` copying a pointer is the CORRECT semantics here.**
  `emit_publish`'s comment (*"a flat block has no internal pointers, the byte copy
  IS a deep copy"*, `gen_present.rs:85`) becomes one word wrong — a `RES` field IS
  an internal pointer — but §15.6 defines record copy as pointer copy (alias), so
  the byte copy implements exactly the language's rule. The comment must be
  rewritten, not the mechanism. The frame-skip data-region compare also still
  works: same resource → same pointer bytes.
- **The registry's type machinery already models `Res`.**
  `ParameterType::Res` exists and is threaded through registry qualification
  (`src/types.rs:64`, `src/codegen/registry/mod.rs:1928`), and is used today for
  *parameters* (`tcp/udp func_poll`: `list_of(Res(socket()))`).
  **UNVERIFIED: whether a `RecordProp` whose `ty` is `Res(...)` flows through
  record validation, construction type-check, and type-export for a BUILTIN
  package record** — plan-114-E proved user-declared records; the registry prop
  path is this letter's Phase 1 experiment.

## 3. Design Overview

Three pieces:

1. **The type swap** — two `RecordProp` types change to
   `ParameterType::res(named(...))`; the two handle records and two members are
   deleted; the three force-emit/analysis seams are cleaned; the pinning test is
   replaced.
2. **The id bridge.** The renderer needs the integer id at draw time. Two new
   **non-exported** members, `canvas::imageHandle(RES Image) AS Integer` and
   `canvas::fontHandle(RES Font) AS Integer`, with `imageRef`'s existing lowering
   minus the record allocation and minus the raise: **a closed resource returns
   `0`** — the id that already means "no image / no font" throughout the renderer
   (the zero-handle idiom the old records documented). `helper_geometry.rs`'s five
   `t.font.id` reads become `canvas::fontHandle(t.font)`.
3. **The docs** — the module comment (`mod.rs:27`, `:138`, `:385-396`, `:731`),
   `func_present.rs:28`, the load/create/measure/get/set member docs, the spec's
   §"Images are named, not embedded", and `.ai/canvas-threading.md` §7's last
   paragraph.

**Where the correctness risk concentrates:** the closed-handle read path. The old
model copied an integer at `imageRef()` time and could never see a later destroy;
the new model chases the pointer at render time, so it reads `closed@16` and
`handle@8` cross-thread while the worker may be destroying. Both words are
single-byte/word flags written once by the worker (`.ai/canvas-threading.md` §7's
model — close sets a flag, nothing is freed), so the race is the same benign class
§7 already documents for textures; the design makes it explicit: **read `closed`
first, then the id; a torn observation yields either the live id or 0, both of
which render a defined picture.**

**Where the design uncertainty concentrates:** the registry `RecordProp` +
`Res` plumbing (§2, UNVERIFIED). Phase 1 proves it on a scratch non-exported
record before the breaking swap.

**Byte-identity is NOT this letter's gate** (surface changes), but every canvas
golden must be unchanged: the id that reaches the renderer is the same integer by
construction. **Expected to diff:** `.ncodesum` on canvas-emitting targets,
`mfb man canvas` (members and types removed), `tests/cli_canvas_package.rs` and
every fixture that minted a handle.

### Rejected alternatives

- **Resolve `RES` → id at publish (a per-variant walk in `emit_publish`).**
  Rejected: it destroys the "byte copy IS the deep copy" property for every scene
  to serve two fields, and the pointer-chase it avoids is one guarded load per
  text run per frame.
- **Keep `imageRef`/`fontRef` as deprecated aliases.** Rejected: the user asked
  for removal; MFBASIC has no deprecation mechanism, and a member that mints a
  now-unnameable record type cannot exist anyway.
- **Return the id raising on closed (today's `imageRef` behavior) in the
  bridge.** Rejected: the bridge runs on the graphics thread at render time — a
  raise there is a crash in the render loop for a program that legally destroyed
  a resource after presenting. `0` = "draws nothing" is today's documented
  render-time semantics for a stale handle; the raise existed only at mint time,
  a moment that no longer exists.

## 4. Detailed Design

### 4.1 The swap

- `Picture.image`: `ty: ParameterType::res(ParameterType::named(IMAGE_TYPE))`,
  description rewritten ("The image to draw. The scene keeps drawing through this
  handle; destroying the image afterwards makes this item draw nothing — the
  handle stays yours to close"). Same shape for `Text.font`.
- Delete the two `add_record` calls (`mod.rs:397-423`), `func_image_ref.rs`,
  `func_font_ref.rs`, their `register` lines, and the seam rows
  (`data_objects.rs:252`, `:274`'s `"canvas.fontRef"`, `module_analysis.rs:47`).
- Replace `resource_handles_are_plain_integer_values` (`mod.rs:984`) with
  `picture_and_text_hold_res_handles`, pinning the NEW shape (field `ty` is
  `Res(Image)`/`Res(Font)`), doc comment citing this plan — the same
  amend-with-the-reason treatment plan-116-E gives the frozen-set test. Under the
  AGENTS.md four-question gate: the old test records a decision
  (handles-as-integers) that the user reversed on 2026-09-01; the decision, not
  the test, is what changed.

### 4.2 The id bridge and the closed read

`imageHandle`/`fontHandle` lower exactly as `lower_image_ref` does today
(`func_image_ref.rs:47-80`) with two deletions: no arena record allocation (the
return is the bare `Integer`), and the closed guard branches to `RETURN 0` instead
of `raise ErrResourceClosed`. Registered non-exported so `mfb man canvas` never
shows them (the same visibility class as the internal draw/publish members).

Renderer changes: the five `t.font.id` sites in `helper_geometry.rs` become
`canvas::fontHandle(t.font)`; there are zero `pic.image` sites to change
(bug-484). The glyph caches, font table (`gen_font_table.rs` — keyed by the same
integer) and every downstream consumer are untouched: they see the same id.

### 4.3 Construction sites and the dead zero-handle idiom

Every `Picture[image := canvas::imageRef(img)]` becomes
`Picture[image := img]`; likewise fonts. The fabricated-handle fixtures
(`ImageRef[id := 0]`, `FontRef[id := 999]`-style) cannot be expressed anymore —
each either takes a real resource or, where the *point* was a stale id
(`rt_canvas_font.rs:634`), becomes destroy-then-present, which is the same
observable ("draws as empty") through the new mechanism and a better test for it.
Ownership at the call sites follows §15.6's float rules automatically; no fixture
needs explicit closes added (scope-drop covers them), but each updated fixture is
re-run, not assumed.

## Compatibility / Format Impact

- **BREAKING: `canvas::ImageRef`, `canvas::FontRef`, `canvas::imageRef`,
  `canvas::fontRef` are removed**, and `Picture`/`Text` construction takes the
  resource directly. Every user program that names any of the four stops
  compiling with ordinary unknown-symbol/type diagnostics.
- **BREAKING (thread plane):** a `DrawItem` list containing `Picture`/`Text` is
  now refused on a thread data plane (`2-203-0138`), where the integer handles
  previously slipped through. No in-tree program does this (census task,
  Phase 2); documented in the spec section this letter rewrites.
- **Behavioral edge:** minting a handle from a destroyed resource used to raise
  `ErrResourceClosed` at the `imageRef()` call; that moment no longer exists —
  constructing a `Picture` with a closed image is legal and the item draws
  nothing. The *render-time* semantics are unchanged.
- **`mfb man canvas` output shrinks**; `.ncodesum` churn on canvas-emitting
  targets; every canvas golden byte-identical.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick in the same commit as the
> work; `- [~]` for partial with a one-line remainder; fill `Commit:` on landing.
> **An unticked box means NOT DONE.**

### Phase 1 — Prove the registry plumbing; land the bridge (no surface change)

- [ ] Re-run every §2 census row; update the tables in place.
- [ ] Unit-prove a builtin `RecordProp` with `ty: ParameterType::res(...)`: give a
      scratch (or the real, still-unwired) record the field in a `#[cfg(test)]`
      registry and drive record validation, construction type-check and
      type-export over it. Fix whatever seams reject it — this is the letter's
      unverified premise and lands first.
- [ ] Add `canvas::imageHandle`/`fontHandle` (non-exported, §4.2) with unit tests:
      live resource → its id; destroyed → 0.
- [ ] Tests: `cargo test --no-fail-fast`; every golden unchanged (nothing visible
      moved yet).

Acceptance: the `Res`-prop probe passes validation/construction/export in tests,
and both bridges return measured ids/zeros at runtime.
Commit: —

### Phase 2 — The breaking swap, in one commit

- [ ] Field types swapped; records/members deleted; seams cleaned
      (`data_objects.rs`, `module_analysis.rs`); pinning test replaced (§4.1).
- [ ] `helper_geometry.rs`'s five reads → `canvas::fontHandle(t.font)` (§4.2).
- [ ] Every construction site updated per §4.3 (re-censused list).
- [ ] Census: no in-tree program sends a `DrawItem` across a thread plane
      (`grep` canvas + `thread::` co-use); record the result here.
- [ ] Tests: `tests/cli_canvas_package.rs` constructs `Picture`/`Text` with real
      resources; `tests/rt_canvas_font.rs` all green including the
      destroy-then-present rewrite; a new negative case pins `2-203-0138` for a
      `DrawItem` on a thread plane.

Acceptance: `cargo test --no-fail-fast` green on mac+RELEASE and linux+DEBUG;
every canvas golden byte-identical on disk; `mfb man canvas --all | grep -ci
'imageRef\|fontRef\|ImageRef\|FontRef'` → 0.
Commit: —

### Phase 3 — Lifetime semantics proven end to end

- [ ] rt test: create image → `Picture` in an installed scene → destroy image →
      present again → frame renders, item contributes nothing, no raise
      (`MFB_CANVAS_SYNC=1`; the software path).
- [ ] rt test: the same for a font: destroyed font's text measures 0 and draws
      empty — today's exact semantics through the new read.
- [ ] rt test: 200 × (open font, put in `Text`, present, drop binding) — glyph
      cache stats and process fd count return to baseline (fonts are
      arena-backed; the loop guards the *pointer-chase* path, not an fd).
- [ ] Tests: `tests/rt_canvas_graphics_thread.rs` — destroy racing a mid-frame
      render (the §3 benign-race claim, asserted, not argued).

Acceptance: all four cases pass; `MFB_CANVAS_STATS` shows no growth across the
200-cycle loop.
Commit: —

### Phase 4 — Docs, spec, and gates

- [ ] `mod.rs` module comment (`:27`, `:138`, `:385-396`, `:731`),
      `func_present.rs` DESC, and the load/create/measure/get/set docs: the scene
      draws *through the handle you still own*; destroying afterwards draws
      nothing. **No memory vocabulary** — copy/mutate/value/alias-for-RES only
      (`.ai/man-content.md`); `scripts/man-census.sh --memory-scope` → 0
      unclassified hits.
- [ ] `src/docs/spec/app/06_canvas.md` §"Images are named, not embedded" —
      rewritten for direct `RES` fields, including the thread-plane consequence.
- [ ] `.ai/canvas-threading.md` §7 last paragraph — the guard moves from
      `imageRef` (which no longer exists) to the render-time closed-read-as-zero
      rule (§4.2).
- [ ] `scripts/man-run-examples.sh canvas --run` passes (every example now names
      resources directly).
- [ ] `scripts/regen-ncodesum.sh`; prove the delta is this letter's.

Acceptance: `cargo test --no-fail-fast` green on both axes;
`scripts/test-accept.sh` green; `scripts/artifact-gate.sh all` 0 diffs;
`mfb man canvas picture`-reachable pages describe the new model with zero banned
vocabulary.
Commit: —

## Validation Plan

- **Tests:** the Phase 1 registry probe; `tests/cli_canvas_package.rs`;
  `tests/rt_canvas_font.rs`; `tests/cli_canvas_image_resource.rs` (rewritten
  around the removed member); `tests/rt_canvas_graphics_thread.rs` (destroy
  race); the `2-203-0138` negative case. Negative cases: destroyed image/font in
  a live scene (draws nothing, no raise); `DrawItem` on a thread plane (refused).
- **Coverage check:** the bridges are codegen lowering — confirm in the
  denominator via `cargo llvm-cov --bin mfb`; the helper-side reads are MFBASIC,
  covered by the rt cases (both the live-id and closed-zero arms must each be
  exercised by a distinct assertion).
- **Runtime proof:** the Phase 3 destroy-while-installed programs, run under
  `MFB_CANVAS_DUMP` and diffed against the same scene never containing the item.
- **Doc sync:** §Phase 4's list; `.ai/specifications.md` discipline for the spec
  edit.
- **Acceptance:** `cargo test --no-fail-fast`, `scripts/test-accept.sh`,
  `scripts/artifact-gate.sh all`, `rustup run 1.96.0 cargo fmt --all &&
  (cd repository && rustup run 1.96.0 cargo fmt)`.

## Open Decisions

- **Closed handle reads as id 0 at render time (§4.2).** Recommended and
  effectively decided: it is the only choice that preserves today's render-time
  semantics and cannot crash the render loop. The alternative (skip the item by
  flag) is the same picture with a second mechanism.
- **Where the bridge members live.** Recommended: non-exported registry members
  beside the other internal canvas machinery, so `mfb man` never shows them and
  the lowering reuses `func_image_ref.rs`'s emitted shape verbatim.

## Corrections

<!-- Filled in during execution. -->

## Summary

The migration is small at the renderer — five integer reads move behind a guarded
pointer chase, and nothing else in the pipeline ever knew the handles existed —
and large at the surface: two records and two members disappear, every
construction site in the tree changes shape, and the "scene holds only integers"
sentence that appears in a dozen doc strings becomes false and must be rewritten
everywhere it appears. The two things to hold onto: the render-time semantics of a
destroyed resource are byte-preserved (closed reads as the zero id), and ownership
still belongs to the program until plan-116-J deliberately takes it for groups.
Untouched: `Image`/`Font` themselves, their `sendable: false`, every renderer
letter A–H, and bug-484's missing picture path.
