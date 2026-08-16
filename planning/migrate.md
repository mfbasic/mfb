# Generic plan: full migration of a builtin package onto the clean-room registry

Last updated: 2026-08-16

This is a **reusable playbook**, not a single-package plan. Substitute `<pkg>`
(e.g. `net`, `strings`, `math`, `fs`) throughout. It describes the **full, finished**
migration of one un-migrated builtin package off the legacy `src/builtins/<pkg>.rs`
+ `src/target/shared/**` machinery and onto the clean-room registry
(`src/codegen/registry`), landing under `src/codegen/builtins/<pkg>/`.

**The bar is "lean and uniform", not "it compiles".** When you are done the package
must look like the packages already migrated — one `register()` in `mod.rs`, one
`func_*.rs` per member, a `package.mfb` for any source, a `native/` dir only if it
lowers to syscalls, and **nothing else**. No per-package resolver, no `IMPL_NAMES`
table, no hand-written runtime-spec file, no `is_<pkg>_call` predicate scattered
across the tree. If the finished package carries any bespoke seam that a migrated
sibling does not, you are not finished — remove it or justify it in review.

Read these before starting: `.ai/resources-packages.md` (registry + package
authoring seams), `.ai/collections.md` (List/Map/Set + fast paths),
`.ai/codegen-invariants.md`, `.ai/testing-gates.md`, and the memory notes
**`mfb-package-rewrite-paths`** and **`adding-a-call-to-an-existing-native-pkg`**.

---

## Byte-identity is a SIGNAL, not a hard gate

`scripts/artifact-gate.sh` byte-identity (`.ast`/`.ir`/`.ncode` diffs) is a **nice-to-have
tripwire, not a pass/fail requirement**. Its value is that for *pure code motion* it should
be neutral — so a diff you did **not** expect is worth investigating. It does **not** mean a
diff is a hard stop.

The real bar for any golden change is: **it is explained.** A migration legitimately changes
some goldens — the member/`.mfb` registration order can shift the injected source (moving
`.ast`/`.ir` line numbers), the descriptor's full parameter list can improve coercion
(better-typed args, added union wraps), a corrected diagnostic can add/reword an error. Those
are fine. When a golden changes:

1. **Understand the diff** — objdump/`.ncode`- or `.ir`-diff ONE fixture and name the cause.
2. **Classify it.** A legitimate consequence of the migration (order shift, better coercion,
   corrected diagnostic — with the program still running correctly, `_rt` fixture green) →
   **regenerate the golden and write the one-line reason** in the commit. An *unexplained* or
   *wrong* diff (broke a program, dropped a symbol, changed emitted behavior) → that's the
   bug; fix the code, do not regenerate.
3. **Never regenerate to HIDE a defect.** "It changed and I don't know why" is not an
   explanation — that's still a bug to hunt (AGENTS.md). The distinction is *explained vs
   unexplained*, not *zero vs non-zero*.

So: keep the gate green **when the change is pure motion** (Phases 1–2), and expect+explain
diffs where the migration genuinely changes output (Phases 3–5). A non-zero gate never blocks
landing on its own.

---

## Two worlds: what you are migrating FROM and TO

There are two parallel descriptor vocabularies. Migration deletes the first for
`<pkg>` and adds the second.

### FROM — the legacy plan-72 world (un-migrated packages: net, fs, io, os, math, …)

- **Descriptor:** a `static <PKG>: BuiltinModule` in `src/builtins/<pkg>.rs`
  (`src/target/shared/registry.rs`'s `BuiltinModule`/`BuiltinFunction`/
  `BuiltinOverload`/`Implementation{Same|Rewrite|Custom|Native|Mfb|Os}`/`Lowering`/
  `BuiltinType`/`BuiltinResolver` vocabulary), listed in the `static REGISTRY`
  (`registry.rs`).
- **Source companion:** `src/builtins/<pkg>_package.mfb`, wired by the
  `super::package_source_glue!(...)` macro (generates `source_file`/`augmented_project`/
  `uses_package`).
- **Native lowering:** hand-written `lower_<pkg>_*_helper` fns under
  `src/target/shared/code/<pkg>/`, dispatched by a hand-written string `match` in
  `src/target/shared/code/mod.rs` (`call if call.starts_with("<pkg>.")`).
- **Runtime specs:** a hand-written `src/target/shared/runtime/<pkg>_specs.rs`
  (`<PKG>_*_SPEC: RuntimeHelperSpec`), listed by hand in `SUPPORTED_HELPER_SPECS`
  (`runtime/catalog.rs`) and routed by an `is_<pkg>_call` arm in
  `runtime/mod.rs::helper_for_call`.
- **Resources:** type-id strings + `resource_close_function` in `<pkg>.rs`, seeded
  into `src/builtins/resource.rs::BUILTIN_RESOURCES` and `src/binary_repr/sections.rs`.
- **Hand-wired hooks** (~14 for net): `is_<pkg>_call` / `expected_arguments` /
  `argument_types` / `call_param_names` / `augmented_project` / `implementation_name`
  threaded into `builtins/mod.rs`, `ir/lower.rs`, `syntaxcheck`, `resolver`,
  per-target `plan.rs`.

### TO — the clean-room registry (migrated packages: csv, json, regex, encoding, collections, datetime, process)

- **Descriptor:** a `RegistryPackage` built in `src/codegen/builtins/<pkg>/mod.rs`
  by a single `register(r: &mut Registry)`, added to the frozen registry by the
  `build()` list in `src/codegen/registry/mod.rs`.
- **Members:** one `func_*.rs` per member, each `add_function`-ing a
  `RegistryFunction` whose `Implementation`s carry a `Body`.
- **Source, native lowering, runtime specs, resources, docs, coercion, rewrite
  targets** are all **owned by the descriptor** — the compiler reads them generically
  through the registry. Nothing package-specific survives in `src/target/`, `src/builtins/`,
  `resource.rs`, `binary_repr`, or the hand-wired hooks above.

The migration is done when `grep -rE '<pkg>::|builtins::<pkg>|__<pkg>_|is_<pkg>_call|<PKG>_.*_SPEC' src/target/ src/builtins/mod.rs` is empty (generic-word hits excepted) and `src/builtins/<pkg>.rs` + `<pkg>_package.mfb` are deleted.

---

## The clean-room registry API (what you build in `mod.rs` / `func_*.rs`)

All in `src/codegen/registry/mod.rs`. This is the whole vocabulary — there is no
per-package constructor wrapper.

### Registration shape

```rust
// src/codegen/builtins/<pkg>/mod.rs
pub(crate) fn register(r: &mut Registry) {
    let mut pkg = RegistryPackage::new("<pkg>", INTRO, DESC);
    // package-level surface (only what applies):
    pkg.add_imports(vec!["strings", "collections"]);      // IMPORTs the companion needs
    pkg.add_source_types(&["Url"]);                        // value types authored in package.mfb
    pkg.add_record(RegistryRecord { .. });                // OR modeled on the registry
    pkg.add_union(RegistryUnion { .. });
    pkg.add_enum(RegistryEnum { .. });                    // rendered into the injected source
    pkg.add_resource(RegistryResource { .. });            // opaque handle (see Resources)
    pkg.add_source_generics(FUNCTIONS);                   // source-generic member names
    pkg.add_source_generic_fast_paths(&[("sort", func_sort::sort_fast_path), ..]);
    pkg.add_helper_functions(vec![include_str!("package.mfb")]); // the source companion
    // one call per member — each func_*.rs registers itself:
    func_parse::register(&mut pkg);
    func_stringify::register(&mut pkg);
    r.add_package(pkg);
}
```

Then add `crate::codegen::builtins::<pkg>::register(&mut r);` to `build()` in
`src/codegen/registry/mod.rs`, and `pub(crate) mod <pkg>;` to
`src/codegen/builtins/mod.rs`.

### `RegistryPackage` builders (only call the ones that apply)

| Builder | Purpose |
|---|---|
| `RegistryPackage::new(import_name, intro, desc)` | package + module docs |
| `add_function(RegistryFunction)` | one public member (call it from the `func_*.rs`) |
| `add_helper_functions(vec![include_str!("package.mfb")])` | the injected `.mfb` companion (private helpers + `Mfb` member bodies + arity bodies + `@@MFB_BODY@@` markers) |
| `add_source_generics(&[&str])` | member names implemented as monomorphized generic `.mfb` bodies (collections) |
| `add_source_generic_fast_paths(&[(member, fn)])` | native fast paths for source-generic members |
| `add_source_types(&[&str])` | value types authored in `package.mfb` (not modeled as records here) |
| `add_record` / `add_union` / `add_enum(Registry{Record,Union,Enum})` | value types modeled on the registry; `get_mfb()` renders enums/records into the injected source |
| `add_resource(RegistryResource)` | opaque, package-scoped handle (`net::Socket`) |
| `add_imports(vec![&str])` | packages the companion `IMPORT`s |

### `RegistryFunction` / `Implementation` / `Body` (the member)

```rust
// src/codegen/builtins/<pkg>/func_<name>.rs
pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "connectTcp",
        intro: INTRO, desc: DESC, example: EX,   // man/help docs live on the member
        expected_arguments: None,                // Some("…") ONLY for a bespoke phrasing
        implementations: vec![Implementation {   // >1 = an overload set
            params: vec![Parameter {
                name: "host",
                desc: "…",
                aliases: &[],
                ty: ParameterType::String,
                default: DefaultValue::None,      // ::Fill{type_name,expr} / ::Optional for trailing optionals
            }, /* … */],
            return_type: ParameterType::Named("net.Socket"),
            errors: vec!["ErrConnRefused"],
            body: /* one of: */
                Body::mfb(FUNC_BODY, "__net_toUrl"),                 // source member; 2nd arg = the rewrite target it declares
                // Body::mfb_with_fast_path(FUNC_BODY, "__x", fast_path),
                // Body::native(Some(posix_lower), Some(win_lower), None),        // OS-seam / native member
                // Body::native_os_seam(Some(posix), Some(win), &["connectTcpAddr"]), // + code-layer overload-split aliases
                // Body::Rewrite("__datetime_instant1"),             // arity overload: body lives in package.mfb
                // Body::Intrinsic,                                  // inline op, no body / no rewrite
        }],
    });
}
```

- `Body::Mfb { body, rewrite, fast_path }` — the rewrite target is a **field**
  (`Body::mfb(body, "__csv_next")`); there is **no `IMPL_NAMES` table and no
  `implementation_name` fn**. The monomorphizer/IR-lowering reads `Body.rewrite_target()`.
- `Body::Native { posix, win, common, os_aliases }` — `posix`/`win` are per-platform
  `OsLower` runtime-helper bodies (built on `abi::` + `CodegenPlatform`, branch on OS
  *family* never per-arch); `common` is a target-generic `NativeLower` call-site
  lowering. `os_aliases` names the `builder_values` overload-split calls that share
  this body (`connectTcpAddr`, `pollList`, `spawnEnv`, `sendTimeout`).
- **Overloads** (arity or type) are just multiple `Implementation`s; `select`/`unify`
  picks one by argument types. datetime's `instant`→`__datetime_instant1..5` is five
  `Implementation`s each `Body::Rewrite("__datetime_instantN")` — **no `Custom`, no
  resolver, no arity-keyed `implementation_name`.**

Constructors `Body::mfb` / `mfb_with_fast_path` / `native` / `native_os_seam` are the
blessed builders. **Fast-path and native-lower fns must be free fns** (the
`MfbFastPath`/`NativeLower` HRTB fn-pointer won't coerce from an `impl` method — E0308).

---

## Target file layout (the finished package)

```
src/codegen/builtins/<pkg>/
  mod.rs            # the single register(); package-level add_*; INTRO/DESC consts;
                    # per-package overload/param helper fns ONLY (super:: from children);
                    # re-exports of native/ entry fns; unit tests.
  package.mfb       # source-backed pkgs: private FUNC __<pkg>_* helpers, arity-member
                    # bodies, and '@@MFB_BODY:<slug>@@ markers where single-body members were.
  func_<name>.rs    # ONE per member, ALWAYS — its INTRO/DESC/EX consts, its BODY const
                    # (Mfb) or native lowering fn (Native), and its RegistryFunction.
                    # A doc-only member still gets its own file.
  native/           # ONLY for native/OS-seam packages: the relocated arch-neutral
    unix.rs         # syscall emission (lower_<pkg>_*_helper). Split by platform family
    windows.rs      # or by concern; re-exported pub(crate) from mod.rs.
  common/           # ONLY if ≥2 members share `impl CodeBuilder` lowering (collections'
                    # native fast paths). Do NOT invent an empty common/.
```

Rules that are **not** optional:
- **One `func_*.rs` per member, always** — even a doc-only member. Do not keep the
  member table inline in `mod.rs`.
- `mod.rs` holds *only* the `register()`, package-level surface, and small
  overload/param helper fns children reach via `super::`. It is NOT a member table.
- **Shared `.mfb` companions used by >1 package** (the Unicode tables: regex + strings)
  live in the neutral `src/codegen/unicode/`, `include_str!`d by each — not nested
  under the first package.
- **No `specs.rs`, no `<pkg>_specs.rs`, no per-package resolver, no `IMPL_NAMES`.** If
  you are writing one, you are copying the legacy shape — stop and derive it instead.

---

## Where the legacy seams GO (derive, don't relocate)

The old world hand-wrote five things per package. The registry derives all five, so
the migration **deletes** them rather than moving them:

| Legacy seam (delete it) | Now derived from | By |
|---|---|---|
| `<pkg>_specs.rs` `RuntimeHelperSpec` consts + `catalog.rs` list | `Body::Native` (`pkg.member` + `os_aliases`, typed by `return_type`) + `RegistryResource.close_function` | `registry::runtime_specs()`, merged into the catalog by `supported_helper_specs()` |
| `resource_close_function` in `<pkg>.rs` + `resource.rs` seeding | `RegistryResource { name, close_function, sendable, close_may_fail, kind }` | `registry::resource_close_function` / `is_qualified_builtin_resource` |
| `is_<pkg>_call` predicate (14 sites) | membership on the registry | `registry().owning_package(call)` / `is_member` / `is_source_generic_member` |
| `expected_arguments` / `argument_types` hand tables | the descriptor's `Parameter`s | `registry::expected_arguments` (diagnostic, full signature) + `registry::argument_types` (machine coercion table — **kept separate**, bug-443) |
| `lower_<pkg>_*_helper` + the `code/mod.rs` string match | `Body::Native` posix/win/os_aliases | `registry::os_helper(call, platform)` (generic OS-seam dispatch) |
| `implementation_name` / `IMPL_NAMES` | `Body::Mfb.rewrite` field + `os_aliases` | `Body.rewrite_target()`; overload-split calls in `monomorph`'s per-package internal-callee map |

The **runtime-family enum variant** (`RuntimeHelper::<Pkg>`) and its
`helper_for_call` arm are shared cross-package infrastructure that map a call to its
family; those two lines stay in `src/target/shared/runtime/`. Everything else in the
"delete it" column goes.

---

## Package-shape taxonomy (classify per MEMBER, not per package)

Measure first: `grep -c 'Implementation::' src/builtins/<pkg>.rs`; `ls src/builtins/<pkg>*.mfb`;
`grep -rlE 'lower_<pkg>_|is_<pkg>_call|__<pkg>_' src/target/`. A real package is a
**mix**; migrate each member by its row.

| Member today | Migrates to | Notes |
|---|---|---|
| `Rewrite("__<pkg>_x")` or source generic, **one** `.mfb` body | `Body::mfb(BODY, "__<pkg>_x")` | body → `#[rustfmt::skip] const BODY` in the `func_*.rs`; a `@@MFB_BODY:<slug>@@` marker replaces it **at its original line** in `package.mfb` |
| `Custom` (resolver-selected) but with **one** body | `Body::mfb(...)` — still! | a single-body member never needs a resolver; return typing is unification (`select`), not a per-package hook |
| source generic **with** a native fast path | `Body::mfb_with_fast_path(BODY, "__x", fast_path)` + `add_source_generic_fast_paths` | fast-path fn → `func_*.rs`; shared lowering → `common/` |
| `Same`/`Inline`/`Helper`, lowered natively (bits/math/io/fs/…) | `Body::native(posix, win, common)` | `impl CodeBuilder { lower_<pkg>_* }` → `native/` (or `common/` if shared); delete the `code/mod.rs` match arm — `os_helper` reaches it |
| `Os { posix, win }` runtime intrinsic (io/fs/net/thread syscalls) | `Body::native_os_seam(posix, win, os_aliases)` | arch-neutral per-platform emission → `native/{unix,windows}.rs`; the `<pkg>_specs.rs` **deletes** (derived) |
| `Custom` **arity one-to-many** (`instant`→`instant1..5`) | N `Implementation`s, each `Body::Rewrite("__<pkg>_slugN")` | the N bodies stay in `package.mfb`; `select` picks by arity — no resolver |
| code-layer overload split (`connectTcp`→`connectTcpAddr`, `poll`→`pollList`) | an `os_aliases` entry on the base member's `Body::Native` | keeps the derived spec + `os_helper` dispatch; the NIR-name rewrite stays in `builder_values.rs` |
| pure data / constant (no lowering) | data on the descriptor | relocate only |

---

## Phases (ordered, each independently landable, each its own commit)

### Phase 0 — Baseline the gate

- `scripts/artifact-gate.sh target/release/mfb <pkg>` (byte-identity of `.ast`/`.ir`/
  `.ncode`) **and** the acceptance suite for `<pkg>` (`scripts/test-accept.sh`) must be
  green at HEAD before any code moves.
- **If a gate is already red on untouched HEAD**, it is a forgotten-regen stale golden,
  not your bug. Prove it benign (build+run the `_rt` fixture: exit 0 + expected output),
  regenerate **only** this package's sums/goldens (`sync-goldens.sh`), and land that as a
  **separate** pre-migration commit. Do not start on a red gate.

### Phase 1 — Scaffold the module + `register()`, relocate the descriptor verbatim

- Create `src/codegen/builtins/<pkg>/mod.rs` with `register(r: &mut Registry)`; move
  `<pkg>_package.mfb` → `<pkg>/package.mfb` and inject it via
  `add_helper_functions(vec![include_str!("package.mfb")])` (keep the `SOURCE_LABEL`/
  `SOURCE_DOC` identity — `"<builtin-<pkg>>"`, `"builtins/<pkg>.mfb"` — byte-identical,
  or `.ast`/`.ir` loc metadata drifts). Enums/records that were `.mfb`-authored move to
  `add_enum`/`add_record` and render through `get_mfb()`.
- Register every member with a `RegistryFunction` translated from the old
  `BuiltinFunction` (name/params/return/errors/docs), still in `mod.rs` for now.
- Add the package to `build()` and `codegen/builtins/mod.rs`; remove `<pkg>` from
  `src/builtins/mod.rs`'s module list and the `REGISTRY` array **once the registry
  serves it** (the compiler dual-paths `registry().X(name).or(old(name))` during the
  transition — see todo.md Phase 2).
- **Acceptance:** `cargo build --bin mfb` clean; `artifact-gate <pkg>` reviewed — this is
  pure code motion so it should be neutral; if the registration order differs from the old
  module's, the injected `.mfb` order (and its `.ast`/`.ir` line numbers) shifts — that's an
  acceptable, explained diff (regenerate + note it), not a bug.

### Phase 2 — Split each member into `func_<name>.rs`

- One file per member: `pub(super) fn register(pkg: &mut RegistryPackage)` +
  `INTRO`/`DESC`/`EX` consts. `mod.rs`'s body becomes `func_<name>::register(&mut pkg)`
  calls. File = `func_<snake(slug)>.rs`.
- **Acceptance:** build clean; `artifact-gate <pkg>` reviewed (pure motion — should be
  neutral; explain any diff).

### Phase 3 — Move each member's implementation onto its `Body` (the real migration)

Per member, by its taxonomy row:
- **Source (`Body::mfb`).** Extract the `FUNC __<pkg>_<slug> … END FUNC` block
  **byte-for-byte** into `#[rustfmt::skip] const BODY: &str = r#"…"#;`; replace it in
  `package.mfb` with a single `@@MFB_BODY:<slug>@@` line **at its original position**.
  `Body::mfb(BODY, "__<pkg>_<slug>")`. Verify the round-trip (`get_mfb()` reassembles the
  original `.mfb` byte-identically) before building. The body's 2-space indentation feeds
  `.ncode` columns; the marker-at-original-line keeps `.ast`/`.ir` line numbers.
- **Native (`Body::native` / `native_os_seam`).** Move the `lower_<pkg>_*_helper`
  emission into `<pkg>/native/{unix,windows}.rs` (`use crate::target::shared::code::*;`,
  promote any `pub(super)` it needs — `HelperResult`/`HelperBody`, `raise_error_into`,
  `finalize_vreg_body_with_locals` — to `pub(crate)`); re-export `pub(crate)` from
  `mod.rs`; delete the member's arm from the `code/mod.rs` `<pkg>.` string match; delete
  the `<pkg>_specs.rs` and its `catalog.rs` rows (now derived by `runtime_specs`).
- **Overloads / arity.** Emit one `Implementation` per overload; arity one-to-many uses
  `Body::Rewrite("__<pkg>_slugN")` with the N bodies kept in `package.mfb`.
- **Transitivity trap.** A helper called *only* by a shared/non-`<pkg>` function is not
  `<pkg>`-only — it stays in `src/target`. Census each helper's callers by *effect*, not
  by one name.
- **Acceptance:** build clean; **review the gate.** Pure `Body::mfb` extraction should be
  neutral, so an unexpected diff there is worth an objdump/`.ncode`-diff of ONE fixture. But
  moving native lowering onto `Body::Native` can legitimately shift output (order, better
  coercion) — understand each diff, keep it if it's an explained consequence (`_rt` fixture
  still green) and regenerate with the reason, fix the code if it's not (byte-identity is a
  signal, not a gate — see the top).

### Phase 4 — Resources, runtime specs, coercion (native packages)

- **Resource:** `add_resource(RegistryResource { name: "<Type>", export, description,
  close_function: "<pkg>.close", sendable, close_may_fail, kind: ResourceKind::Builtin })`
  with `<PKG>_TYPE_ID = "<pkg>.<Type>"` package-scoped (plan-97). Delete the type-id +
  `resource_close_function` in `<pkg>.rs`, the `resource.rs::BUILTIN_RESOURCES` seeding,
  and the `binary_repr/sections.rs` type-id rows once the registry answers
  `is_qualified_builtin_resource`/`resource_close_function` for it.
- **Runtime specs:** delete `<pkg>_specs.rs` entirely — `runtime_specs()` derives each
  `pkg.member` (+ `os_aliases`, typed by `return_type`) and each resource close op.
  Confirm with `catalog::tests::catalog_is_consistent`.
- **Coercion/diagnostics:** the descriptor's `Parameter`s now drive both
  `registry::argument_types` (machine coercion table) and `registry::expected_arguments`
  (diagnostic). Delete the hand `expected_arguments`/`argument_types`/`call_param_names`
  tables in `<pkg>.rs`.
- **Acceptance:** runtime/catalog + coercion tests pass; `artifact-gate` reviewed — the fuller
  descriptor param list can legitimately improve coercion (better-typed args / added union
  wraps): keep those explained diffs (`_rt` green, native `.ncodesum` usually unchanged),
  regenerate with the reason.

### Phase 5 — Docs / man2

- Per-member `intro`/`desc`/`example` on each `RegistryFunction`; module `intro`/`desc`
  on `RegistryPackage::new`. man2 is registry-generic — just verify `mfb man2 <pkg>` and
  `mfb man2 <pkg> <fn>` render intro/params/desc/examples and the overview.
- **Citation repointing rides with the file move, not here.** The
  `man_citations_resolve` / `spec_citations_resolve` tests break the instant a file
  moves; repoint each `[[…]]` in the *same* commit that moves the file. Use `\b` so
  `__<pkg>_add` doesn't match `__<pkg>_addDays`.
- **Acceptance:** citation tests pass; man2 renders; `artifact-gate` reviewed (docs are
  metadata — usually neutral; explain any diff).

### Phase 6 — Delete the legacy surface + land

- `git rm src/builtins/<pkg>.rs src/builtins/<pkg>_package.mfb
  src/target/shared/runtime/<pkg>_specs.rs`; remove the `code/<pkg>/` dir (or fold into
  `native/`); delete the `REGISTRY` entry, the `is_<pkg>_call`/hook arms in
  `builtins/mod.rs` / `ir/lower.rs` / `syntaxcheck` / `resolver` / per-target `plan.rs`,
  the `catalog.rs` `<PKG>_*_SPEC` rows, the `resource.rs`/`binary_repr` seeding.
- `rustup run 1.96.0 cargo fmt --all && (cd repository && rustup run 1.96.0 cargo fmt)`.
- Full `cargo test --bin mfb` (never one module); `cargo clippy --bin mfb` clean on the
  new module (no `#![allow(dead_code)]`); `artifact-gate <pkg>` + a **dependent**
  package's gate (one whose `.mfb` calls `<pkg>::`).
- **Emptiness check:** `grep -rE '<pkg>::|builtins::<pkg>|__<pkg>_|is_<pkg>_call|<PKG>_.*_SPEC' src/target/ src/builtins/mod.rs src/binary_repr/ src/resolver/` → empty (generic-word hits excepted).

---

## Rewire teardown checklist (delete every legacy `<pkg>` site)

Grep `[^:]<pkg>::` and `builtins::<pkg>` across `src/`. For net there were ~14; expect
similar. Delete or repoint each:

- `src/target/shared/registry.rs` — the `&crate::builtins::<pkg>::<PKG>` REGISTRY entry.
- `src/builtins/mod.rs` — `mod <pkg>;`, the import/package-name match arm, and every
  `.or_else(|| <pkg>::expected_arguments/argument_types/call_param_names/…)` dispatch.
- `src/ir/lower.rs` — `<pkg>::augmented_project` injection, `is_<pkg>_call` gate, the
  `.or_else(|| <pkg>::implementation_name(..))` rewrite (now `Body.rewrite_target`).
- `src/target/shared/code/mod.rs` — `mod <pkg>;` + the `<pkg>.` string-match dispatch.
- `src/target/shared/code/builder_values.rs` — the NIR-name overload-split rewrites
  (`<pkg>.x` → `<pkg>.xAddr`) become `os_aliases`; the resource-consume/arity logic stays
  but keys on registry data.
- `src/target/shared/runtime/catalog.rs` — the `<PKG>_*_SPEC` array rows +
  `CODE_LAYER_ONLY_CALLS` entries + family-coverage assertion (keep the enum variant).
- `src/target/shared/runtime/mod.rs` — leave `RuntimeHelper::<Pkg>` + its
  `helper_for_call` arm (shared family map); it may now key on `owning_package`.
- `src/syntaxcheck/{mod,builtins,helpers}.rs` — `augmented_project`, the `BuiltinArgMode`
  entry, any `type_name == <pkg>::<TYPE>` checks.
- `src/resolver/mod.rs` — resource type-id arrays; source-injection ordering.
- `src/builtins/resource.rs` + `src/binary_repr/sections.rs` — resource seeding/type-ids.
- per-target `src/target/<arch>/plan.rs` — `is_<pkg>_call` symbol-planning arms.

---

## Byte-identity & gotchas (hard-won)

- **An UNEXPLAINED gate diff is a bug-hunt trigger; an explained one is fine.** Objdump/
  `.ncode`-diff ONE fixture, localize, name the cause. A diff on a target you *expected* to
  change (order shift, better coercion, corrected diagnostic) is the plan working —
  regenerate + note it. Only "changed for no reason I can name" is the bug. Byte-identity is
  a signal, not the gate.
- **Preserve the synthetic source path/doc labels** exactly (`.ast`/`.ir` loc metadata).
- **Marker substitution restores exact bytes** (`get_mfb()` splices `BODY` with no
  leading/trailing newline; surrounding newlines come from `package.mfb`). Choose
  raw-string hashes so `"#…` can't close early.
- **Fast-path / native-lower fns are free fns** (HRTB coercion; methods → E0308).
- **`super::` in a `func_*.rs` reaches the parent `mod.rs`** (private-item access to an
  ancestor — no `pub(super)` needed) for overload/param helpers; use `crate::…` absolute
  paths for shared codegen imports.
- **A migration can surface a stale golden in a *sibling*** (moving a shared `include_str!`
  Unicode table flips strings' gate red on a byte-identical rename). Prove benign,
  regenerate the sibling's sums in *its own* commit — don't fold it in.
- **Moving a *generated* file is a lockstep edit** (update the generator's output path,
  `scripts/check-generated.sh`, `scripts/list_functions.py`; re-run `check-generated.sh`).
- **Subagent edits can silently vanish** — `git diff --stat` before trusting any
  "tests pass" from delegated file moves.
- **No test/golden re-baseline to HIDE a defect** (AGENTS.md): you may regenerate a golden the
  migration legitimately changed (with the reason in the commit), but never to paper over an
  unexplained/wrong diff. Explained → regenerate; unexplained → hunt the bug.
- **Own the deviation.** If a package tempts you to keep a resolver, an `IMPL_NAMES`
  table, a `specs.rs`, or the member table inline "because it's special" — it is not.
  Do it the uniform way or ask in review before deviating.

---

## Verification checklist (Phase 6 gate)

- [ ] `cargo build --bin mfb` clean, **0 warnings**.
- [ ] `cargo test --bin mfb` fully green (citations, monomorph, syntaxcheck, resolver,
      man2, `catalog_is_consistent`, package unit tests).
- [ ] `scripts/artifact-gate.sh target/release/mfb <pkg>` reviewed (+ a dependent package
      for ripple): every diff is either zero or **explained + regenerated with its reason** —
      no unexplained diff remains.
- [ ] Acceptance (`test-accept.sh` for `<pkg>` + a dependent) green.
- [ ] `cargo clippy --bin mfb` clean on `src/codegen/builtins/<pkg>/**`.
- [ ] `mfb man2 <pkg>` and `mfb man2 <pkg> <fn>` render intro/params/desc/examples.
- [ ] **Leanness:** the finished package is `mod.rs` (one `register`) + `func_*.rs` per
      member + `package.mfb` + optional `native/`/`common/` — **no `specs.rs`, no
      resolver, no `IMPL_NAMES`, no member table in `mod.rs`.**
- [ ] `grep -rE '<pkg>::|builtins::<pkg>|__<pkg>_|is_<pkg>_call|<PKG>_.*_SPEC' src/target/ src/builtins/mod.rs src/binary_repr/ src/resolver/` → empty.
- [ ] `src/builtins/<pkg>.rs`, `<pkg>_package.mfb`, `<pkg>_specs.rs`, `code/<pkg>/` deleted;
      both fmt passes run.

---

## Worked examples (read the finished code)

- **`csv`, `json`** — pure source + value/opaque **types**: `Body::mfb` members; the
  rewrite target is the `Body::mfb` field (`readRow` → `Body::mfb(.., "__csv_next")`); no
  resolver, no `IMPL_NAMES`.
- **`regex`** — the multi-file source case: `get_mfb()` splices member bodies then appends
  the shared Unicode tables from `src/codegen/unicode/` (also `include_str!`d by `strings`).
- **`encoding`** — pure `Body::mfb`, no `common/`; overloaded `utf8Encode`/`utf8Decode` are
  two `Implementation`s, code-layer-mangled in `monomorph`.
- **`collections`** — `add_source_generics(FUNCTIONS)` + `add_source_generic_fast_paths`
  + `Body::mfb_with_fast_path`; shared lowering in `common/`.
- **`datetime`** — hybrid: single-body members on `Body::mfb`; `instant`/`parse` arity
  overloads as N `Implementation`s each `Body::Rewrite("__datetime_slugN")` (bodies in
  `package.mfb`); three OS-seam intrinsics (`nowNanos`/`monotonicNanos`/`localOffset`) as
  `Body::native` with emission in `native.rs`. **No `Custom`, no `DatetimeResolver`** —
  any lingering doc comment saying otherwise is stale.
- **`process`** — the native OS-seam + resource reference: every member
  `Body::native_os_seam(posix, win, os_aliases)` with emission in `native/{unix,windows}`;
  the `Process` handle via `add_resource` (`PROCESS_TYPE_ID = "process.Process"`); runtime
  specs **derived** (`runtime_specs`) — there is no `process/specs.rs`.
