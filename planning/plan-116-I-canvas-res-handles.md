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
(`mod.rs`'s `ImageRef`/`FontRef` records, `func_image_ref.rs`, `func_font_ref.rs`).

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
| plan-116-H complete and archived | `ls planning/completed/plan-116-H-*` → one match | **MET** (2026-09-04: one match, archived after box 2228 went green) |
| plan-114 A–E complete and archived | `ls planning/completed/plan-114-*` → 5 matches | **MET** (re-measured 2026-09-04: 5 matches, A–E) |
| A union variant record may carry a `RES` field, and `List OF <that union>` compiles | the probe program below (§2) | **MET** (re-probed 2026-09-04) |

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
- **No `sendable`/`live_slots` change on `Image`/`Font`** — the two
  `pkg.add_resource(RegistryResource { … })` calls in `mod.rs`
  (`grep -n 'add_resource\|sendable' src/codegen/builtins/canvas/mod.rs`). They stay
  `sendable: false`; the transfer-audit question belongs to
  plan-116-J Phase 1. Consequence, documented not softened: a `DrawItem` list
  containing a `Picture`/`Text` cannot cross a thread data plane
  (`2-203-0138 TYPE_THREAD_RESOURCE_PLANE_REQUIRED`) — where the old integer
  handles could. §Compatibility.
- **No STATE clause on either field.** `Image`/`Font` carry no STATE; the fields
  are bare `RES` slots.
- **No existing golden may move.**

## 2. Current State

### The two handles and who touches them

- `ImageRef`/`FontRef` records: `mod.rs`'s two `add_record` calls; minting members:
  `func_image_ref.rs` / `func_font_ref.rs`, each reading `handle@8` behind a
  closed-guard that **raises `ErrResourceClosed`** on a destroyed resource.
- The renderer's only resource reads are `t.font.id` — 5 sites, all in
  `helper_geometry.rs` (`grep -n 't\.font\.id' src/codegen/builtins/canvas/` →
  `:626,:660,:681,:934,:942,:1009` as of 2026-09-03) feeding `__canvas_fontBlob`/glyph
  lookups by integer id. **Nothing reads `pic.image` anywhere** (bug-484).
- Seam registrations naming the members:
  `src/codegen/memory/data/data_objects.rs:252` (`"canvas.imageRef"`), `:274`
  (`"canvas.fontRef"` in the font force-emit list),
  `src/codegen/engine/analysis/module_analysis.rs:47` (`"canvas.imageRef"`).
  These are the force-emit/analysis pairings the
  `adding-a-call-to-an-existing-native-pkg` memory warns about — on removal they
  must be deleted or `catalog_is_consistent`-class tests fail.
- The pinning test `resource_handles_are_plain_integer_values`
  (`grep -n resource_handles_are_plain_integer_values src/codegen/builtins/canvas/mod.rs`)
  asserts exactly the design this letter retires.

### Measured populations (2026-09-01)

| What | Count | Command |
|---|---|---|
| Files naming `imageRef`/`fontRef`/`ImageRef`/`FontRef` | ~~22~~ ~~29~~ **30 + 1** | `grep -rln … src/ tests/ examples/` gives 29 and `… src tests` gives 29, but over **different sets** (**I5**). The union is 30; `.ai/canvas-threading.md` is the +1 that no command reaches. |
| `Picture[` construction sites (code + doc examples) | 7 | `grep -rn 'Picture\[' --include='*.rs' --include='*.mfb' src/ tests/ examples/` |
| `canvas::Text[` construction sites | ~~12~~ ~~20~~ ~~22~~ **20** | `grep -rn 'canvas::Text\[' …`. The old figure used the unqualified `Text\[`, which also matches three `astrings::AttrText[` sites in another package (**I5**). Today's unqualified count is 23. |
| Renderer reads of `t.font.id` | ~~5~~ **6** | `grep -n 't\.font\.id' src/codegen/builtins/canvas/helper_geometry.rs` |
| Renderer reads of `pic.image` | 0 | `grep -rn 'pic\.image' src/codegen/builtins/canvas/` (bug-484) |
| `.id` reads that are **not** construction sites | **2** | `tests/cli_canvas_image_resource.rs:104` (`IF handle.id = 0`), `tests/rt_canvas_font.rs:74` (`IF r.id = 0`). Assertions *about* a handle, so the `Picture[`/`Text[` sweep cannot see them and they need a semantic rewrite, not a substitution (**I5**). |
| Fabricated zero-handle uses (`ImageRef[id := 0]`, `FontRef[id := …]`) | ~~3~~ **5** | `tests/cli_canvas_package.rs` `:54`, `:55`, `:233`; `tests/rt_canvas_font.rs:672`; `tests/rt_canvas_present_deep_copy.rs:130` (**not** `:104` — **I5**) |

> **Re-measured 2026-09-02 (I1).** Four of the six rows had drifted, two of them by
> more than half. The growth is this plan's own: letters C, D and E added text and
> scene fixtures, and peers landed more. **Re-run every row again at Phase 1** — this
> letter's whole job is a mechanical sweep over these populations, so a stale count is
> not a scoping detail here, it is the work itself. plan-116-D's D2 and D5 are the two
> ways that goes wrong: a count measured at plan time, and a census whose command
> cannot see every site.

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

  **Re-probed 2026-09-04 and it builds — but not as written.** Two things the sketch
  leaves out, both of which are a compile error rather than a subtlety:

  * `fs::openFile`'s mode is a **`String`**, not an enum: `fs::openFile(p, "write")`.
    There is no `fs::OpenMode` (`2-201-0011 SYMBOL_UNKNOWN_IDENTIFIER`).
  * A variant value cannot be appended to `List OF Thing` directly — neither the
    record variable nor a record literal. It must be **bound through the union type**
    first:

    ```
    LET t AS Thing = h
    things = collections::append(things, t)
    ```

    Passing `h` straight in gives `2-203-0021 TYPE_CALL_ARGUMENT_MISMATCH`:
    *"argument type(s) (List OF Thing, Holder), expected List OF T, T"*. The generic
    binds `T` to the argument's own type rather than widening it to the union.

  That last point matters beyond the probe: §4.3's construction sites build
  `DrawItem` values, and `DrawItem` is a union. Any site that hands a bare `Picture`
  or `Text` to something expecting `List OF DrawItem` has the same shape.

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
  (`grep -n 'Res(' src/types.rs` for the variant; `grep -n 'ParameterType::Res' src/codegen/registry/mod.rs`
  for the qualification arm — *line citations dropped per **I9***), and was used at
  authoring time only for *parameters* (`tcp`/`udp` `func_poll`: `list_of(Res(socket()))`).
  ~~**UNVERIFIED: whether a `RecordProp` whose `ty` is `Res(...)` flows through record
  validation, construction type-check, and type-export for a BUILTIN package record**~~
  — **VERIFIED, and then shipped** (2026-09-05, closing a marker that would otherwise be
  archived still reading as open). Phase 1's experiment answered it and the whole letter
  rests on the answer: `Picture.image` is
  `ty: ParameterType::res(ParameterType::named("canvas.Image"))` in
  `canvas/mod.rs` today, it renders as `RES canvas.Image` in `mfb man canvas types`, and
  programs construct `canvas::Picture[…, image := img, …]` from a `RES` binding across
  the whole test suite. plan-114-E had proved only user-declared records; the registry
  prop path is proved by this letter existing.

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
3. **The docs** — the module comment and the `ImageRef`/`FontRef` prose in `mod.rs`
   (`grep -n 'ImageRef' src/codegen/builtins/canvas/mod.rs`),
   `func_present.rs`'s DESC, the load/create/measure/get/set member docs, the spec's
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
`Res` plumbing (§2 — **UNVERIFIED at authoring time, verified in Phase 1 and shipped**).
Phase 1 proves it on a scratch non-exported
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
- Delete the two `add_record` calls (`ImageRef` and `FontRef` in `mod.rs`), `func_image_ref.rs`,
  `func_font_ref.rs`, their `register` lines, and the seam rows
  (`data_objects.rs:252`, `:274`'s `"canvas.fontRef"`, `module_analysis.rs:47`).
- Replace `resource_handles_are_plain_integer_values` with
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

- [x] Re-run every §2 census row; update the tables in place. — Done, and three rows
      were wrong in ways re-running their own commands does not reveal (**I5**): the
      file count's two commands return 29 over *different* sets (union 30, plus
      `.ai/canvas-threading.md`, which no command reaches); the `Text[` row counted three
      `astrings::AttrText[` sites in another package (20 canvas sites, not 22); and the
      fabricated-handle row's line for `rt_canvas_present_deep_copy.rs` was 26 lines
      stale. A new row records the two `.id` reads that are *assertions about* a handle
      rather than constructions with one, so the sweep driving Phase 2 cannot see them.
      §Summary's third copy of "five integer reads" now says six.
- [x] Unit-prove a builtin `RecordProp` with `ty: ParameterType::res(...)`: give a
      scratch (or the real, still-unwired) record the field in a `#[cfg(test)]`
      registry and drive record validation, construction type-check and
      type-export over it. Fix whatever seams reject it — this is the letter's
      unverified premise and lands first. —
      `a_builtin_record_property_may_carry_a_res_type` in `codegen/registry/mod.rs`.
      **No seam rejected it**, so there was nothing to fix: `source_spelling` is
      `ty.name()`, and `Res(inner).name()` is `RES <inner>`, so the property exports as
      `image AS RES canvas::Image` by construction.
      The test asserts the **variant**, not the spelling, because
      `Named("RES canvas.Image")` and `Res(Named("canvas.Image"))` render identically
      and behave differently — that substitution would satisfy every other assertion
      here while the field was not a resource at all.
- [~] Add `canvas::imageHandle`/`fontHandle` (non-exported, §4.2) with unit tests:
      live resource → its id; destroyed → 0. — Both landed in
      `func_handle_bridge.rs`, forked from `lower_image_ref` with the two deletions §4.2
      names: no arena allocation (the id is returned bare, since a scene can now carry
      the resource) and no raise on the closed path.
      `the_handle_bridges_are_internal_take_a_resource_and_cannot_raise` pins the
      declared contract — `internal_only`, a `RES` parameter, an `Integer` return, and
      an **empty `errors` list**, which is the machine-checkable form of "answers 0,
      does not raise". `mfb man canvas --all` mentions neither.
      **Remaining:** the runtime half (live → id, destroyed → 0) is not asserted yet.
      An `internal_only` member has no caller until Phase 2 wires it into
      `helper_geometry`, so the observation lands with Phase 3's destroyed-font rt test
      rather than here. Recorded rather than claimed.
- [x] Tests: `cargo test --no-fail-fast`; every golden unchanged (nothing visible
      moved yet). — `cargo test --release --no-fail-fast` on macOS: **rc=0, 113 targets,
      no failures**. The artifact gate is one of those targets, so "every golden
      unchanged" is asserted by the same run rather than by a separate claim.

Acceptance: the `Res`-prop probe passes validation/construction/export in tests,
and both bridges return measured ids/zeros at runtime.

**Half met.** The probe passes, and it passed *unmodified* — the premise held, so the
"fix whatever seams reject it" half of that box turned out to be empty. The bridges
are landed and their declared contract is pinned, but "return measured ids/zeros **at
runtime**" is not shown: nothing calls an `internal_only` member until Phase 2, so
there is no runtime to measure. That assertion moves to Phase 3, where a destroyed
font in an installed scene is already a planned rt case — it is the same observation.
Commit: `5a1dd63d1`, `509b72a22`

### Phase 2 — The breaking swap, in one commit

- [x] Field types swapped; records/members deleted; seams cleaned
      (`data_objects.rs`, `module_analysis.rs`); pinning test replaced (§4.1). —
      **Nine seam rows, not the two this box names** (**I5**): `data_objects.rs` ×2,
      `module_analysis.rs`, and a per-target force-emit list in
      `macos_aarch64/mod.rs`, `win_x86_64/mod.rs` and `linux_common/mod.rs`, ×2 each.
      Missing one force-emits a symbol that no longer exists.
      The pin is **inverted rather than replaced** and renamed
      `the_resource_naming_variants_hold_the_resource_itself`: it asserts the *variant*,
      because `Named("canvas.Image")` renders identically to
      `Res(Named("canvas.Image"))` and is a value field that copies the record. It also
      asserts the records and members are **gone**, not merely unused.
- [x] `helper_geometry.rs`'s **six** reads of `t.font.id` → `canvas::fontHandle(t.font)` (§4.2). Six, not five — §2's table was corrected by **I1** and this task was not (**I3**); re-count at Phase 1 anyway. — Re-counted at Phase 1: six, at `:660, 694, 715, 987, 995, 1111`. All six replaced.
- [x] Every construction site updated per §4.3 (re-censused list). — 21 mechanical
      `canvas::imageRef(x)`/`fontRef(x)` → `x`, plus five fabricated handles that could
      not be substituted and were rewritten to what each was asserting: two zero handles
      became a real `createImage`/`loadFont`, the stale-id case became
      **destroy-then-present** (§4.3), and two fixtures now thread a real font through
      their scene helpers.
      Two `.id` reads were **assertions about** a handle rather than constructions with
      one, so no construction census could see them; both were checking that the *mint*
      succeeded, and there is no mint step, so both were reduced rather than translated
      — see the commit for why a `measureText` substitute was tried and rejected.
- [x] Census: no in-tree program sends a `DrawItem` across a thread plane
      (`grep` canvas + `thread::` co-use); record the result here. — **Result: none.**
      No `.mfb` file and no MFBASIC source string in `tests/` or `src/` co-uses `canvas`
      and `thread::`. The narrowing this letter introduces therefore breaks no existing
      program. (The naive grep matches ~30 *compiler* sources that mention both words;
      the census has to be scoped to MFBASIC programs or it answers a different
      question.)
- [x] Tests: `tests/cli_canvas_package.rs` constructs `Picture`/`Text` with real
      resources; `tests/rt_canvas_font.rs` all green including the
      destroy-then-present rewrite; a new negative case pins `2-203-0138` for a
      `DrawItem` on a thread plane. — `cli_canvas_package` 7 passed (its builder now
      writes `fixture.ttf` beside every project, and `run_headless` runs from the
      project directory so `loadFont` can find it); `rt_canvas_font` 13 passed;
      `tests/syntax/threads/canvas-drawitem-thread-plane-invalid` reports `2-203-0138`
      on **both** the message and the output plane.
      That fixture names `RES canvas.Image`, not the font: `DrawItem` is a union and the
      cause walk reports the first resource-carrying variant it reaches. Its comment
      says so, and says not to remove whichever variant looks unused — either alone
      would refuse the plane.

Acceptance: `cargo test --no-fail-fast` green on **mac RELEASE, mac DEBUG (`--bin mfb`) and **box 2228 RELEASE, scoped** (**J16**: a bare `cargo test --release` on that box is one `rustc` per test target on a single core — measured at 2h37m without completing one target, so it is a multi-day run, not a slow one. The row is decomposed to the targets whose behaviour can differ by *platform*, which is what it exists for, and run **once on the final merged tree** for plan-116-I and plan-116-J together)** (plan-116-E **E6**: CI is `--release` on all five platforms, so the `debug_assert!`s run nowhere in it and the debug row has to be run here);
every canvas golden byte-identical on disk; `mfb man canvas --all | grep -ci
'imageRef\|fontRef\|ImageRef\|FontRef'` → 0.

**Met on macOS; the box-2228 row is queued behind plan-116-H's own Linux run on that
one-core machine.**

| gate | result |
|---|---|
| mac RELEASE | `rc=0` |
| mac DEBUG (`--bin mfb`) | `rc=0`, **3790 passed** |
| canvas goldens byte-identical | `git status tests/golden/canvas/` is empty |
| `mfb man canvas --all \| grep -ci …` | **0** |
| box 2228 RELEASE | **folded into the combined final row** (**J16**) — the run started for this letter was validating a snapshot taken before plan-116-J rewrote `gen_group.rs`, `func_present.rs`, `helper_render.rs`, `ir/verify` and five support tables, so finishing it would have proved something about a tree that no longer exists |

`man-run-examples.sh canvas --run`: **25 examples, 25 built, 25 ran, 0 failed** — two
fewer than before because two members are gone, and every remaining example now
constructs with the resource directly.
Commit: `8a9a9f294`, `a274f147b`, `4730b1896`

### Phase 3 — Lifetime semantics proven end to end

- [x] rt test: create image → `Picture` in an installed scene → destroy image →
      present again → frame renders, item contributes nothing, no raise
      (`MFB_CANVAS_SYNC=1`; the software path). —
      `an_image_destroyed_while_a_scene_names_it_renders_a_frame_and_does_not_raise`.
      **"Present again" is not expressible** (**I6**): `destroyImage` consumes the
      binding, so building a second `Picture` from it is `2-203-0055
      TYPE_USE_AFTER_MOVE`. The test presents the item that *already* holds the image,
      which is the reachable shape and the one the lifetime question is about.
- [~] rt test: the same for a font: destroyed font's text measures 0 and draws
      empty — today's exact semantics through the new read. —
      **Draws empty: asserted** (`text_whose_font_was_destroyed_draws_nothing`, rewritten
      as destroy-then-present).
      **Measures 0: unreachable, and that is the finding** (**I6**). `destroyFont`
      consumes its binding, so `measureText` on a destroyed font is a compile error
      rather than a zero. The remainder is not work left undone — there is nothing to
      assert, because the situation cannot be written.
- [x] rt test: 200 × (open font, put in `Text`, present, drop binding) — glyph
      cache stats and process fd count return to baseline (fonts are
      arena-backed; the loop guards the *pointer-chase* path, not an fd). —
      `two_hundred_presents_through_one_font_leave_the_glyph_cache_where_they_found_it`.
      **One font, 200 presents — not 200 loads** (**I6**): each `loadFont` mints a new
      backend identity and the cache is keyed by it, so 200 loads legitimately leave
      `glyphs=200`, measuring the cache's key rather than the pointer chase the box's
      own parenthetical names.
- [x] Tests: `tests/rt_canvas_graphics_thread.rs` — destroy racing a mid-frame
      render (the §3 benign-race claim, asserted, not argued). —
      `destroying_a_font_mid_frame_is_clean`, in `rt_canvas_rasteriser.rs` rather than
      `rt_canvas_graphics_thread.rs` (**I6**): `MFB_CANVAS_FRAME_HOLD_MS` and the two
      sibling mid-frame race tests live there.
      The hold is what makes it a test rather than a hope — it parks the graphics thread
      inside `__canvas_renderFrame` so the destroy lands *while* a frame holding that
      font is drawing.

Acceptance: all four cases pass; `MFB_CANVAS_STATS` shows no growth across the
200-cycle loop.

**Met, with one case reduced rather than passed.** Three of the four assert what the box
asked. The fourth — "destroyed font's text measures 0" — cannot be written: the compiler
refuses it. That is the guarantee being stronger than the plan expected, not the test
being weaker (**I6**).
`MFB_CANVAS_STATS` across 200 presents through one font: no growth.
Commit: `a6ea441b0`

### Phase 4 — Docs, spec, and gates

- [x] `mod.rs` module comment and every `ImageRef`/`FontRef` mention in it
      (`grep -n 'ImageRef\|FontRef' src/codegen/builtins/canvas/mod.rs`),
      `func_present.rs` DESC, and the load/create/measure/get/set docs: the scene
      draws *through the handle you still own*; destroying afterwards draws
      nothing. **No memory vocabulary** — copy/mutate/value/alias-for-RES only
      (`.ai/man-content.md`); `scripts/man-census.sh --memory-scope` → 0
      unclassified hits. — `0 unclassified hits`. The remaining `ImageRef`/`FontRef`
      mentions in `mod.rs` are all in the *inverted pin* and the comment recording what
      used to be declared there — history, not surface. `mfb man canvas --all` mentions
      neither, which is the check that matters.
- [x] `src/docs/spec/app/06_canvas.md` §"Images are named, not embedded" —
      rewritten for direct `RES` fields, including the thread-plane consequence. —
      Including the rule code (`2-203-0138`) and the remedy, so a reader who hits it
      finds the section that explains it. Two sentences in the neighbouring
      "content is orthogonal" subsection also said *"behind the id"* and now say
      "behind the image".
- [x] `.ai/canvas-threading.md` §7 last paragraph — the guard moves from
      `imageRef` (which no longer exists) to the render-time closed-read-as-zero
      rule (§4.2). — And **row R3 of the race matrix**, which the box does not mention
      (**I5**): it asserted `ErrResourceClosed` *at `imageRef`*, so it named the
      mechanism, not just the wording.
      The replacement is stronger than a moved guard: `destroyImage` consumes its
      binding, so "name it again" is a compile error. The render-time rule covers what
      the guard actually protected — a scene built *before* the destroy, which still
      draws, as nothing.
- [x] `scripts/man-run-examples.sh canvas --run` passes (every example now names
      resources directly). — `examples: 25   built: 25   ran: 25   failed: 0`. Two
      fewer than before this letter, because two members are gone.
- [x] `scripts/regen-ncodesum.sh`. Expect **0 diffs, and do not read that as
      evidence** — no `canvas` fixture is hashed (plan-116-F **F11**). —
      `141 golden(s) refreshed, 0 missing`, no `.ncodesum` file changed. Predicted, and
      not evidence: this letter changed the canvas registry, the renderer's font read
      and nine force-emit tables, and none of that is hashed. What the gate proves is
      the narrow thing — nothing *outside* canvas moved.

Acceptance: `cargo test --no-fail-fast` green on **mac RELEASE, mac DEBUG (`--bin mfb`) and **box 2228 RELEASE, scoped** (**J16**: a bare `cargo test --release` on that box is one `rustc` per test target on a single core — measured at 2h37m without completing one target, so it is a multi-day run, not a slow one. The row is decomposed to the targets whose behaviour can differ by *platform*, which is what it exists for, and run **once on the final merged tree** for plan-116-I and plan-116-J together)** (plan-116-E **E6**: CI is `--release` on all five platforms, so the `debug_assert!`s run nowhere in it and the debug row has to be run here);
`scripts/test-accept.sh` green; `scripts/artifact-gate.sh all` 0 diffs;
`mfb man canvas picture`-reachable pages describe the new model with zero banned
vocabulary.

**Met on macOS; box 2228 is running.**

| gate | result |
|---|---|
| mac RELEASE | green — the run reported `rc=101`, and its **only** failing target was `artifact_gate_all`, which refused to start because a peer session held the gate lock. Re-run standalone: 0 diffs. Nothing was checked on the first attempt, so it was never a golden result. **Re-run on the merged tree: 133 targets ok** (same refusal, same resolution — plan-116-J **J21**). |
| mac DEBUG (`--bin mfb`) | `rc=0`, **3790 passed**. **Merged tree: 3832 passed**, 0 failed. |
| `scripts/test-accept.sh` | **1379 tests ran**, passed — one more than before this letter, which is `canvas-drawitem-thread-plane-invalid`. **Merged tree: 1399 ran**, the growth being `main`'s own fixtures plus plan-116-J's `canvas-setgroup-consumes-items`. |
| `scripts/artifact-gate.sh all` | 1357 tests, 1520 builds, **1878 goldens, 0 diffs**. **Merged tree, uncontended: 1377 / 1540 / 1906 goldens, 0 diffs.** |
| banned vocabulary | `man-census.sh --memory-scope` → **0 unclassified hits** (re-run 2026-09-04 on the merged tree: still 0) |
| removed surface absent from `mfb man` | `mfb man canvas --all` → `rc=0`, **2417 lines, 0 mentions** of `imageRef`/`fontRef`/`ImageRef`/`FontRef` (re-run 2026-09-04 on the tree merged with `main`, which had added canvas doc text this letter never saw) |
| box 2228 RELEASE | **folded into the combined final row** (**J16**) — stopped at 2h37m with 0 test targets completed; see that correction for the measurement and the decomposition |

The gate's refusal is worth noting rather than glossing: it exits **98** and says
*"another gate run holds the lock … nothing was checked"*. Read as a failure it would
have sent someone hunting a golden regression that does not exist.
Commit: `f7280678d`

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

**I9 (2026-09-04) — the corrected line numbers **I5** supplied have themselves drifted,
and the fix is to stop supplying them.**

**I5** caught that this letter's three seam citations were stale and gave the right ones:
*"`data_objects.rs:252` → **261**, `data_objects.rs:274` → **291**,
`module_analysis.rs:47` → **58**."* Re-measured today, after plan-116-J added
`canvas.nextReclaimableGroup`, `canvas.retiredItems` and `canvas.groupSlots` to the same
tables:

| I5's corrected number | today |
|---|---|
| `data_objects.rs:261` | **`:264`** |
| `data_objects.rs:291` | **`:297`** |
| `module_analysis.rs:58` | **`:61`** |

Three for three, inside two months, from a sibling letter adding rows *above* them —
nothing about the rows these citations point at changed at all.

**So the correction is not a fourth set of numbers.** The claim these citations support is
*"the members are registered in these three seams, and a removal that misses one fails a
`catalog_is_consistent`-class test"* — and that claim is checked by
`grep -n 'canvas\.\(image\|font\)Handle' src/codegen/memory/data/data_objects.rs
src/codegen/engine/analysis/module_analysis.rs`, which returns the seams by name and
cannot go stale. The **count** (three registrations across two files) is the load-bearing
part; the offsets never were.

Recorded in both letters — plan-116-J hit the identical failure three times
(**J1**, **J6**, **J17**) and swept its own citations for the same reason. Four
independent instances in one series is the argument for the rule rather than for the
fixes: **cite the symbol and the grep, never the line.**

**I7 (2026-09-04) — the box-2228 row is decomposed and folded into a combined final run;
recorded here as well as in plan-116-J **J16**, because a reader of this letter alone must
not conclude the row was skipped.**

A bare `cargo test --release --no-fail-fast` on 2228 is **one `rustc` per test target on a
single core**. Measured: the run started for this letter reached **2h37m with zero test
targets completed** (`grep -c '^test result:' linux_i.log` → `0`, `rustc` at 91.6% of the
one core), the first hour being `mfb` itself. With ~90 targets that is a multi-day run.

Two changes follow, both recorded rather than quietly taken:

1. **Scope.** The row is decomposed to the targets whose behaviour can differ by
   *platform* — glibc and the x86-64 ABI — which is what plan-116-E **E6** put it there
   for. Re-running host-independent logic already green on macOS buys nothing.
2. **Timing.** It runs **once, on the final merged tree**, covering this letter and
   plan-116-J together. The in-flight run was validating a snapshot taken before
   plan-116-J rewrote `gen_group.rs`, `func_present.rs`, `helper_render.rs`, `ir/verify`
   and five support tables, so completing it would have proved something about a tree that
   will never land.

Everything else in this letter's Phase 4 acceptance was met on macOS and is unaffected:
mac RELEASE, mac DEBUG (3790), `test-accept` 1379, artifact-gate 1878 goldens 0 diffs, and
`man-census.sh --memory-scope` 0 unclassified hits.

**I6 (Phase 3) — three of Phase 3's four cases assume a runtime observation the type
system makes unreachable. The guarantee is stronger than the plan asked for.**

`canvas::destroyFont` and `canvas::destroyImage` **consume their binding**. So the
shapes Phase 3 describes — "destroy → present again", "destroyed font's text measures
0" — are not runtime cases at all:

```
canvas::destroyFont(face)
LET after = canvas::measureText(face, 40.0, "AA")
                                ^ 2-203-0055 TYPE_USE_AFTER_MOVE
```

A program cannot measure a destroyed font, and cannot build a *new* item naming one.
That is a better outcome than the runtime answer the box wanted, and it means the box
cannot be ticked as written. What remains expressible — and is what the letter actually
has to guarantee — is:

> build the item while the resource is live, destroy the resource, present the item
> **that already holds it**.

The scene's copy outlives the binding, which is precisely the lifetime question this
letter opened by putting a resource in a record field. Both halves are now asserted:
`text_whose_font_was_destroyed_draws_nothing` and
`an_image_destroyed_while_a_scene_names_it_renders_a_frame_and_does_not_raise`.

**A second shape looked like a failure and was correct behaviour.** Presenting the same
scene again after the destroy leaves the *previous* frame — ink and all — on screen,
because `present` skips a re-present of an unchanged scene. A test written the obvious
way reports "the frame still has ink" and is wrong to. Where a second frame is wanted,
the scene has to differ.

**The 200-cycle churn box measures the cache's key, not the pointer chase, if written as
200 loads.** Each `loadFont` mints a new backend identity and the glyph cache is keyed by
it, so N loads legitimately produce N entries: measured `glyphs=200 glyphBytes=22000
glyphEvictions=0` — bounded by the cache cap rather than leaking, but growing for a
reason that has nothing to do with this letter. Rewritten as **one font presented 200
times**, which is the pointer-chase path the box's own parenthetical names, and which
holds at a handful of entries.

Worth recording separately, because it is a real property and not obviously the intended
one: **`destroyFont` does not purge the destroyed font's glyphs from the cache.** It
unregisters the blob — which is what makes later text draw empty, via the `len(b) = 0`
guard in `__canvas_textGlyphRun` — but entries already rasterised stay until eviction
pressure. Bounded, so not a leak; noted for whoever next reads "return to baseline".

**Placement deviation:** the mid-frame race test is in `rt_canvas_rasteriser.rs` rather
than `rt_canvas_graphics_thread.rs` where the box names it, because
`MFB_CANVAS_FRAME_HOLD_MS` and the two sibling mid-frame race tests live there. A third
race test beside them is easier to keep honest than one that re-derives the timing.

**I5 (2026-09-04, pre-execution) — the §2 census is wrong in three ways that a re-run of
its own commands does not reveal, and §4.1's seam list is 3 rows of 9.**

Measured before starting, because I4 had already corrected these numbers once and the
correction did not hold.

**1. The two census commands return 29 over *different* 29-file sets.**

```
grep -rl 'imageRef\|fontRef\|ImageRef\|FontRef' --include='*.rs' --include='*.mfb' src/ tests/ examples/   → 29
grep -rl 'imageRef\|fontRef\|ImageRef\|FontRef' src tests                                                   → 29
```

The first sees `examples/emoji/src/main.mfb` and misses `src/docs/spec/app/06_canvas.md`;
the second does the reverse. The union is **30**, plus `.ai/canvas-threading.md`, which no
command in the plan looks at. Two commands agreeing on a *count* while disagreeing on a
*set* is the most persuasive way a census can be wrong, and it is why the number survived
I4 unchallenged.

**2. The `Text[` row counts a different package's records.** It reads 23 today, but only
**20** are `canvas::Text[`. The other three are `astrings::AttrText[` —
`astrings/func_font.rs:29`, `astrings/helper_decode_attr.rs:16`,
`tests/acceptance/src/astrings.mfb:182` — matched because the pattern `Text\[` has no
package qualifier. They have been in this row for all three of its measurements. (One of
the 23 is genuinely new: plan-116-H's `groups.png` scene added a `Text` item, so the
canvas figure moved 19 → 20 while the row moved 22 → 23.)

**3. `tests/rt_canvas_present_deep_copy.rs`'s fabricated `FontRef` is at `:130`, not
`:104`.** I1's table gives the old line.

**4. §4.1 and §2 undercount the force-emit seams by six lines in three files the plan
never names.** Beyond `data_objects.rs` and `module_analysis.rs`, each target keeps its
own list:

| file | lines |
|---|---|
| `src/target/macos_aarch64/mod.rs` | 75, 78 |
| `src/target/win_x86_64/mod.rs` | 141, 144 |
| `src/target/linux_common/mod.rs` | 90, 93 |

And all three line numbers the plan *does* give are stale: `data_objects.rs:252` → **261**,
`data_objects.rs:274` → **291**, `module_analysis.rs:47` → **58**. Phase 2's "seams
cleaned" is a 3-of-9 checklist as written. Missing a per-target row does not fail the
build — it force-emits a symbol that no longer exists, which is the shape
`new-error-in-a-package-needs-a-data-object-row` records as a link failure on a
historical symbol.

**5. Two `.id` reads will break and appear in no census.**
`tests/cli_canvas_image_resource.rs:104` (`IF handle.id = 0 THEN RETURN 6`) and
`tests/rt_canvas_font.rs:74` (`IF r.id = 0`) are assertions *about* the handle rather than
constructions *with* it, so the `Picture[`/`Text[` sweep that drives Phase 2 does not see
them. Both need a semantic rewrite, not a substitution: there is no `.id` on a `RES`.

**6. §Summary still says "five integer reads".** I3 corrected that number in §2 and in
Phase 2 and stopped there — which is I3's own closing lesson ("a number in a plan is
usually written down more than once") applied one place short. The count is **six**:
`helper_geometry.rs:660, 694, 715, 987, 995, 1111`.

**7. `.ai/canvas-threading.md` needs more than §7's last paragraph.** Row **R3** of the
table at `:221` asserts `ErrResourceClosed` *at `imageRef`* as a race outcome. That member
ceases to exist, so the row's mechanism is gone, not merely its wording.

**Also confirmed, so Phase 1's probe is genuinely unprecedented:** no builtin record
property anywhere declares a `Res` type. `grep -rn 'ParameterType::res(' src/codegen/builtins/`
→ 0 hits; the only `ParameterType::Res` uses are two *parameters*
(`tcp/func_poll.rs:149`, `udp/func_poll.rs:138`). I4 is right that there is no precedent
to copy, and the registry arms Phase 1 must satisfy are
`registry/mod.rs:1928, 2264, 2267, 2360, 2363, 2586, 2608, 3842`.

**I8 (pre-execution, 2026-09-04; recorded as a second "I4" and renumbered 2026-09-04 —
two corrections shared the number, so a reference to "I4" resolved to whichever the
reader found first) — re-measured all six rows; one had drifted again.**
`Text[` is **22**, up from the 20 recorded by **I1** on 2026-09-02. The two new ones are
plan-116-G's: `text_inside_a_translated_group_draws_at_the_offset` in
`tests/rt_canvas_font.rs`, and the fixture it builds.

The other five are unchanged from I1: 29 files naming the ref types, 7 `Picture[` sites,
6 reads of `t.font.id`, 0 reads of `pic.image`, 5 fabricated zero-handle uses.

This is the third measurement of this table and the third time `Text[` has moved — 12 →
20 → 22 — which is the row to distrust rather than the count to memorise. I1 already
says to re-run every row at Phase 1; the reason is now empirical rather than cautionary,
and the mechanism is worth naming: **every canvas letter that adds a rendering feature
adds a text fixture to prove the feature works on the glyph path**, because the glyph
path is on a different branch from the distance path and is exactly what a fix written
for distances misses (plan-116-G **G5**). So this row grows once per letter, by
construction, and will have grown again by the time I runs.

**I4 (2026-09-03, pre-execution, measured while plan-116-G was gating) — the letter's
"unverified premise" is confirmed unverified, and the reason is sharper than "nothing
has tried it".** Phase 1 says a builtin `RecordProp` carrying `ty:
ParameterType::res(...)` is this letter's unverified premise and must land first. Two
measurements bracket it:

* `ParameterType::res` **exists and is exercised** — `src/types.rs` constructs it in
  `with_vars`, in `parse` (the `RES ` prefix arm), and in its own tests over
  `fs.File`, including nested inside a `List OF`. So the *type* is not the risk.
* **No builtin record field uses one.** `grep -rn "RecordProp" -A 3 src/codegen/builtins/
  | grep -c "ParameterType::res("` → **0** across every package.

So what is untested is specifically the **registry → record-validation →
construction-typecheck → type-export** path for a `Res`-typed *field*, not the type
model underneath it. That narrows Phase 1's probe: it does not need to establish that
`RES` works, it needs to establish that those four seams accept a field whose type is
one. Write the probe against those four by name.

**I3 (2026-09-03, pre-execution) — I1 corrected the census table but not the task that
consumes it.** §2's row reads *"Renderer reads of `t.font.id` — ~~5~~ **6**"*, while
Phase 2's task still says *"`helper_geometry.rs`'s five reads"*. An executor working
the checklist rather than the table sweeps five of six sites and leaves one reading
`.id` off a field that is no longer a record — which fails to compile, so it is a
cheap defect, but only because this particular field change is type-visible. The
general form of that mistake is not.

Re-measured: `grep -c 't\.font\.id' src/codegen/builtins/canvas/helper_geometry.rs`
→ **6**, at `:626, :660, :681, :934, :942, :1009`. The line list in §2 was also from
the pre-C/D/E/F file and has been replaced.

*Re-measured again 2026-09-03 after plan-116-G:* still **6** reads, and **every line
number moved** — now `:660, :694, :715, :987, :995, :1111`. G added the seven `Group`
arms and `__canvas_groupHash` to this file, which shifted everything below them.

That is the argument for not writing the line list down a third time. The count is the
useful fact and it has been stable across F and G; the positions are not, and a list
that must be re-measured to be trusted is a list that should have been a grep. Phase 2
should run `grep -n 't\.font\.id' src/codegen/builtins/canvas/helper_geometry.rs` and
sweep what it prints.

The lesson is narrower than I1's and worth keeping separate: when a correction changes
a count, grep the letter for every *other* place that count is spelled. A number in a
plan is usually written down more than once.

**I2 (2026-09-03, pre-execution) — every `mod.rs` line citation in this letter is
stale, the same defect plan-116-G recorded as G1.** Checked with
`awk 'NR==N {print}' src/codegen/builtins/canvas/mod.rs` for each cited N; not one
lands on what the letter says is there. A sample:

* `mod.rs:398`, given as `Picture`'s image field, is a sentence about
  `canvas::Paint.transform`.
* `mod.rs:397`, given as the `ImageRef` integer id, is a sentence about a
  degenerate transform collapsing points to the origin.
* `mod.rs:398-423`, given as the `ImageRef`/`FontRef` record declarations, spans a
  `Paint.transform` sentence to a `Float` prop type.

Not every citation was wrong, and the ones that hold are worth naming so this
correction is not read as blanket distrust: `mod.rs:27` (the module comment on the
value handles) is exact, and `src/codegen/registry/mod.rs:1928` — a *different*
file — really is the `ParameterType::Res` arm this letter's premise rests on.
An earlier draft of this note claimed that one was past end-of-file; it is not,
and `awk 'NR==1928' src/codegen/registry/mod.rs` shows the `Res` qualify arm in a
5254-line file.

*Both survivors re-checked 2026-09-03, after plan-116-F, plan-116-G and a 38-commit
merge of `main`:* `mod.rs:27` is still the module comment's *"through the
`ImageRef`/`FontRef` value handles, since a record field cannot hold"* line, and
`registry/mod.rs:1928` is still the `ParameterType::Res(inner) =>` qualify arm. Both
held through changes that moved every other canvas citation, which is worth knowing —
they are stable anchors, and this letter's premise rests on the second one.

Prefer the symbol grep regardless: `grep -n "ParameterType::Res(" src/codegen/registry/mod.rs`
finds the arm at 1928 and two more uses at 2264/2267 that a line citation would have
hidden.

The letters were written before plan-116-C, D, E and F each added records and
descriptions to that file. The counts and claims these citations *support* are not in
question — this is a navigation defect — but it is the dangerous kind, because the line
a reader lands on is plausible code they could edit in good faith.

Every one is replaced with the **symbol** and the command that finds it, per G1's
lesson: every letter of this plan edits `mod.rs`, so a line citation into it decays the
moment the letter before it lands. Verify with
`grep -n 'name: \"Picture\"\|live_slots\|keeps the scene from retaining' src/codegen/builtins/canvas/mod.rs`.

- **I1 (2026-09-02, pre-execution) — four of the six measured populations had drifted,
  two by more than half.** Re-measured against the tree after plan-116-C, D and E
  landed:

  | Row | Plan (2026-09-01) | Now |
  |---|---|---|
  | Files naming `imageRef`/`fontRef`/`ImageRef`/`FontRef` | 22 | **30 + `.ai/canvas-threading.md`** |
  | `canvas::Text[` construction sites | 12 | **20** (the unqualified pattern reads 23; three are `astrings::AttrText[`) |
  | Renderer reads of `t.font.id` | 5 | **6** — `helper_geometry.rs:660, 694, 715, 987, 995, 1111` |
  | Fabricated zero-handle uses | 3 | **5** |
  | `Picture[` sites | 7 | 7 |
  | Renderer reads of `pic.image` | 0 | 0 |

  Most of the growth is this plan's own — C added the transformed-text fixtures and a
  golden scene that loads a font, D and E added GPU harness scenes — with the rest from
  peers. **This matters more here than in the letters before it**: D and E were
  *additive* changes whose census only mis-scoped an estimate, whereas this letter is
  a mechanical sweep *over* these populations, so a stale count is not a scoping detail,
  it is the work itself. A missed `Text[` site is a site that keeps the old handle.

  **Re-measured again 2026-09-03, after plan-116-F and G landed — and this time with the
  commands written down, which I1 did not do.** A census row whose command is not
  recorded cannot be re-run by the next reader, only re-invented, and two re-inventions
  of the same row are two different rows:

  ```
  grep -rl 'imageRef\|fontRef\|ImageRef\|FontRef' src tests | wc -l   -> 29
  grep -rn 'Text\[' src tests | wc -l                                  -> 20
  grep -rn 't\.font\.id' src | wc -l                                   ->  6
  grep -rn 'Picture\[' src tests | wc -l                                ->  7
  grep -rn 'pic\.image' src | wc -l                                     ->  0
  ```

  Every total is unchanged from I1 — but **the composition is not**: plan-116-G added a
  `Text[` site (`tests/rt_canvas_font.rs:378`, the text-in-a-translated-group case) and
  the total still reads 20, so something else lost one. Do not read "the same number"
  as "the same set". The sweep is over *sites*, and Phase 1 should diff the file list,
  not the count.

  **The sixth row had no command, and its NAME is wrong** — recovered here by
  measurement. "Fabricated zero-handle uses: 5" is not what it sounds like. The obvious
  reading, `grep -rnE '(FontRef|ImageRef)\[ *id *:= *0'`, yields **2**, both in
  `tests/cli_canvas_package.rs`. The number that reproduces 5 is *every hand-built ref
  record*, zero or not:

  ```
  grep -rn 'FontRef\[\|ImageRef\[' src tests | wc -l   -> 5
  ```

  | site | handle |
  |---|---|
  | `tests/rt_canvas_present_deep_copy.rs:104` | `FontRef[id := 1]` |
  | `tests/rt_canvas_font.rs:672` | `FontRef[id := 12345]` |
  | `tests/cli_canvas_package.rs:54` | `ImageRef[id := 0]` |
  | `tests/cli_canvas_package.rs:55` | `FontRef[id := 0]` |
  | `tests/cli_canvas_package.rs:233` | `FontRef[id := 3]` |

  The row is right about the *population* and wrong about the *predicate*, and the
  distinction is this letter's subject: what the `RES` migration removes is a ref record
  hand-built from an integer — `id := 12345` names no font just as surely as `id := 0`
  does. A Phase 1 that swept only the zero ones would leave three sites constructing a
  handle out of a literal, which is the exact shape this letter exists to delete.

  Re-run every row at Phase 1 rather than trusting the table, and heed plan-116-D's two
  ways this goes wrong: **D2**, a count measured at plan time on a shared checkout, and
  **D5**, a census whose command cannot see every site (there, MFBASIC embedded in a
  shell heredoc, which `--include='*.rs' --include='*.mfb'` cannot match).

## Summary

The migration is small at the renderer — **six** integer reads move behind a guarded
pointer chase, and nothing else in the pipeline ever knew the handles existed —
and large at the surface: two records and two members disappear, every
construction site in the tree changes shape, and the "scene holds only integers"
sentence that appears in a dozen doc strings becomes false and must be rewritten
everywhere it appears. The two things to hold onto: the render-time semantics of a
destroyed resource are byte-preserved (closed reads as the zero id), and ownership
still belongs to the program until plan-116-J deliberately takes it for groups.
Untouched: `Image`/`Font` themselves, their `sendable: false`, every renderer
letter A–H, and bug-484's missing picture path.
