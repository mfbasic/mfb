# Generic plan: full migration of a builtin package into `src/codegen/builtins`

Last updated: 2026-08-10

This is a **reusable playbook**, not a single-package plan. Substitute `<pkg>`
(e.g. `strings`, `math`, `net`) throughout. It describes the **full** migration —
the same depth `collections` and `encoding` received — so that after it runs, the
package is entirely self-describing from the registry and **no package-specific
code remains in `src/target`**.

Several migrations have run this playbook; read them as worked examples:
- `src/codegen/builtins/collections/` — source generics **and** native fast paths
  (`Implementation::Mfb` + `Mfb.fast_path`, plus `common/` lowering).
- `src/codegen/builtins/encoding/` — pure MFBASIC source (`Implementation::Mfb`,
  every `fast_path` `None`, no `common/`).
- `src/codegen/builtins/{csv,json}/` — pure source with a resolver + record/opaque
  **types**; concrete rewrite via `IMPL_NAMES` (json's `JsonResolver`; csv's
  irregular `readRow` → `__csv_next`).
- `src/codegen/builtins/regex/` — the **multi-file** source case: `assembled_source`
  splices member bodies into the engine, then appends shared Unicode tables that
  live in the neutral `src/codegen/unicode/` (also `include_str!`d by `strings`).
- `src/codegen/builtins/datetime/` — a **hybrid**: `Custom` members whose source
  bodies stay in `package.mfb` (arity one-to-many, so `Mfb` does not fit) **plus**
  three OS-seam runtime intrinsics whose arch-neutral syscall emission moved to
  `native.rs`.

**One rule that overrides convenience: one `func_*.rs` per member, always** — even
a `Custom`, doc-only member with no body or per-member lowering gets its own file
(its intro/description/examples are content, and consistency is the standard). Do
not keep the member table inline in `mod.rs` because "there's nothing but docs to
co-locate"; that is a rationalization, not an exemption.

Read `.ai/collections.md`, `.ai/resources-packages.md`, `.ai/codegen-invariants.md`,
and the memory note **`mfb-package-rewrite-paths`** before starting.

---

## Goal (checkable)

`src/builtins/<pkg>.rs` and `src/builtins/<pkg>*.mfb` no longer exist; the package
lives under `src/codegen/builtins/<pkg>/` with one `func_*.rs` per member; every
member's lowering is owned by its descriptor (`Implementation::Mfb`/`::Native`/
`::Custom`), so **no `<pkg>`-specific symbol remains under `src/target/`**; the
docs (`doc_intro`/`doc_desc`/`doc_example`) are migrated into the descriptors and
`mfb man2 <pkg> [fn]` renders them; and the whole change is **byte-identical**
(`scripts/artifact-gate.sh <pkg>` → 0 diffs) with the full suite green.

## Non-goals / invariants (must NOT change)

- **Runtime behavior / emitted code.** Every phase is provably codegen-neutral.
  The gate is byte-identity of `.ast`/`.ir`/`.ncode` (and friends), not "tests pass".
- **Public surface.** Call names, arity, overload resolution, error codes, and
  return-type resolution are preserved exactly (the plan-72 parity contract).
- **The injected source's identity.** The synthetic path label and doc path passed
  to `parse_source_internal` (e.g. `"<builtin-encoding>"`, `"builtins/encoding.mfb"`)
  are preserved **byte-for-byte**, or `.ast`/`.ir` line/loc metadata drifts.
- **Non-`<pkg>` packages.** Shared helpers you move must keep serving their other
  callers (see "Transitivity trap").

---

## Package-shape taxonomy (decides which mechanisms apply)

Classify the package **first** — the phases below apply per member according to how
that member is implemented today. Measure, don't guess:

```
grep -c 'Implementation::' src/builtins/<pkg>.rs        # how members are declared
ls src/builtins/<pkg>*.mfb                               # source companion?
grep -rlE 'lower_<pkg>_|builder_<pkg>|is_<pkg>_call|__<pkg>_' src/target/  # native payload
```

Classify **per member, not per package** — the decisive fact is often *how many
`.mfb` bodies a member has*, not the package's overall `Implementation`. (datetime
was uniformly `Custom`, but 37 of its 44 members have a single body and belong on
`Mfb`; only 7 truly need `Custom`. Don't lump the whole package by its hardest
members — that is the same mistake as keeping the table inline "because it's just
docs".)

| Member shape today | Migrates to | Mechanism |
|---|---|---|
| `Rewrite("__<pkg>_x")` / source generic, **one** body in `<pkg>*.mfb` | `Implementation::Mfb { body, fast_path: None }` | body → Rust const + `'@@MFB_BODY:<slug>@@` marker in `package.mfb`, spliced by `assembled_source()` |
| `Custom` (resolver-selected) but with **one** `__<pkg>_<slug>` body | **`Implementation::Mfb`** (still!) | same as above. A member is `Mfb`-eligible whenever it has exactly one body — **even in a resolver package**: return-type resolution is module-level (`resolve_call_return_type` always calls `module.resolver`, `src/builtins/mod.rs`), so it fires regardless of a member's `Implementation`. Converting `Custom`→`Mfb` for a single-body member is byte-identical. |
| Source generic **with** a native fast path in `src/target` | `Implementation::Mfb { body, fast_path: Some(..) }` | as above **plus** the fast-path fn moves into the `func_*.rs`; shared lowering → `common/` |
| `Same` + `Inline`/`Helper`, lowered natively in `src/target` (bits/math/io/…) | `Implementation::Native(lower)` | the `impl CodeBuilder { lower_<pkg>_* }` methods move into the package; shared lowering → `common/`; reached via the `try_native_lower` seam |
| `Custom` with a **one-to-many arity** mapping (`instant`→`__<pkg>_instant1..5`) | `Implementation::Custom` (must stay) | `Mfb` holds one body per member; arity bodies **stay in `package.mfb`** verbatim. Resolver + arity-keyed `implementation_name` move with the module. |
| **OS-seam runtime intrinsic** — arch-neutral per-platform syscall emission (datetime `nowNanos`; io/fs/net/thread syscalls) | `Implementation::Custom`, emission → `native.rs` | a **third lowering kind** (not `Mfb`, not `try_native_lower`): a `RuntimeHelperSpec` + a `lower_<pkg>_helper` built on the `abi::`/`CodegenPlatform` seam. See Phase 3's OS-seam bullet. |
| A pure constant / data-only member (no lowering) | stays data in the descriptor | relocate only |

A real package is usually a **mix** (datetime: `Mfb` + arity-`Custom` + OS-seam;
`strings`: native scalar members + an `.mfb` companion + Tier-B transforms). Handle
each member by its row.

### Constructors (use the registry-wide ones — never a per-package `ef`/`mf` wrapper)

`src/codegen/registry.rs`:
- `BuiltinFunction::mfb(name, slug, intro, desc, errors, overloads, body)`
- `BuiltinFunction::mfb_with_fast_path(.., body, fast_path)`
- `BuiltinFunction::native(name, slug, intro, desc, errors, overloads, lower)`
- `BuiltinFunction::custom(name, slug, intro, desc, errors, overloads)`
- `.with_example(EX)` chains `## Examples`; `.with_intro(..)` / `.with_desc(..)` chain
  intro/desc for members declared via a compact `(name, slug, overloads)` helper
  (datetime's `df`) rather than a full constructor.

Package-local helpers stay only for **overload/parameter** construction (collections'
`custom`/`req`/`opt`, encoding's `ov`/`p`, datetime's `req`/`opt`/`optn`/`ov`) — the
descriptor-level `BuiltinFunction` constructor is always the shared one. Child
`func_*.rs` reach these parent helpers via `super::` (private-item access to an
ancestor module — no `pub(super)` needed).

---

## Target file layout

```
src/codegen/builtins/<pkg>/
  mod.rs            # module decls, ENCODING_FUNCTIONS-style table (func:: refs),
                    # resolver (if any), metadata tables, source glue, IMPL_NAMES,
                    # overload/param helpers, runtime specs/predicate, tests
  package.mfb       # source-backed pkgs: private helpers + arity-member bodies +
                    # '@@MFB_BODY:<slug>@@ markers where single-body members were
  func_<name>.rs    # one per member, ALWAYS — its INTRO/DESC/EX consts, its BODY
                    # const (Mfb) or native lowering fn (Native), and its descriptor.
                    # A doc-only Custom member still gets its own file.
  native.rs         # ONLY for OS-seam packages: the relocated arch-neutral syscall
                    # emission (`lower_<pkg>_helper`), re-exported pub(crate) from mod.rs
  common/           # ONLY if members share `impl CodeBuilder` lowering (Native/
    mod.rs          # fast-path packages). Holds the shared lowering methods that
    *.rs            # were `<pkg>`-only in src/target. Pure-source packages have none.
```

`common/` exists **iff** the package has native lowering that multiple members
share (collections has it; encoding does not). Do not invent an empty `common/`.
**Shared `.mfb` companions used by more than one package** (the Unicode tables:
regex + strings) live in the neutral **`src/codegen/unicode/`**, not nested under
the first package to need them.

---

## Phases (ordered, each independently landable, each its own commit)

### Phase 0 — Baseline the gate (prerequisite)

- Run `scripts/artifact-gate.sh target/release/mfb <pkg>`.
- **If it already fails on untouched `main`**, it is a *forgotten-regen* stale
  golden (a prior codegen change drifted output without refreshing sums), not your
  bug. Prove it benign (build+run the `_rt` fixture: exit 0 + expected output; a
  clean rebuild is deterministic), regenerate **only** this package's sums, and
  land it as a **separate** commit *before* touching code. Precedent: the encoding
  and collections golden-regen commits. Do **not** start the migration on a red gate.
- **Acceptance:** `artifact-gate <pkg>` = 0 diffs at HEAD before any code moves.

### Phase 1 — Scaffold the module, relocate the descriptor verbatim

- Create `src/codegen/builtins/<pkg>/mod.rs` from `src/builtins/<pkg>.rs`; move
  `<pkg>*.mfb` → `src/codegen/builtins/<pkg>/package.mfb`.
- Add `pub(crate) mod <pkg>;` to `src/codegen/builtins/mod.rs`; remove it from
  `src/builtins/mod.rs`. Point `REGISTRY` at `crate::codegen::builtins::<pkg>::<PKG>`.
- Replace `super::package_source_glue!` (which resolves `super` = `crate::builtins`)
  with an in-module `source_file`/`uses_package`/`augmented_project` — **and** for a
  source package a marker-substituting `assembled_source()` (copy encoding's). Keep
  `SOURCE_LABEL`/`SOURCE_DOC` **byte-identical** to the macro's old literals.
- Rewire every external reference (see **Rewire checklist**).
- **Acceptance:** `cargo build --bin mfb` clean; `artifact-gate <pkg>` = 0 diffs
  (no `Implementation` changed yet — this is pure code motion).

### Phase 2 — Split each member into `func_<name>.rs`

- One file per registry member exporting `pub(crate) const <NAME>: BuiltinFunction`.
  `mod.rs`'s table becomes `func_<name>::<NAME>` references. Child modules can read
  the parent's private overload/param helpers (`super::ov`/`p`/…) — no `pub(super)`
  needed.
- File name = snake(slug) (`func_base64_url_encode.rs`); const = UPPER_SNAKE(slug).
- **Acceptance:** build clean, `artifact-gate <pkg>` = 0 diffs.

### Phase 3 — Move the implementation into the descriptor (the real migration)

Per member, by its taxonomy row:

- **Source (`Mfb`).** Extract the exact `FUNC __<pkg>_<slug> … END FUNC` block
  **byte-for-byte** into a `#[rustfmt::skip] const BODY: &str = r#"…"#;` in the
  `func_*.rs`; replace it in `package.mfb` with a single `'@@MFB_BODY:<slug>@@`
  line **at its original position**. Descriptor via `BuiltinFunction::mfb(…, BODY)`.
  Verify the round-trip: substituting every marker reproduces the original `.mfb`
  **identically** (a scripted diff — do this before building).
  - **Byte-significance:** the body's 2-space indentation feeds source *columns*
    into `.ncode`; the marker-at-original-position keeps every other line's *number*
    unchanged for `.ast`/`.ir`. Both matter; exact extraction satisfies both.
- **Native (`Native`).** Move the member's `impl CodeBuilder { lower_<pkg>_* }`
  methods into the `func_*.rs` (or `common/` if shared by ≥2 members) as free/impl
  fns; expose a `NativeLower` fn; descriptor via `BuiltinFunction::native(…, lower)`.
  Delete the member's arm from the old `lower_<pkg>_call` dispatch in `src/target`;
  the `try_native_lower` seam (`builder_values.rs`) now reaches it. Promote any
  `pub(super)` helpers the moved code needs to `pub(crate)`; import shared consts
  (`use crate::target::shared::code::*;`) rather than re-pathing each.
  - **Fast-path fns must be free fns** — an `impl` method won't coerce to the
    `MfbFastPath`/`NativeLower` HRTB fn-pointer (E0308).
- **Custom.** Descriptor via `BuiltinFunction::custom(…)`; move the package
  `BuiltinResolver` and its `dispatch_*` helpers into `mod.rs` unchanged. **But
  first check each Custom member's body count** — a single-body Custom member is
  `Mfb`-eligible (see the taxonomy); only arity one-to-many and OS-seam members
  stay `Custom`. A source-backed Custom package therefore usually needs an
  `assembled_source()` too (markers for the `Mfb` members; arity bodies stay).
- **OS-seam runtime intrinsic (`native.rs`).** For a member lowered by the
  *runtime-call* seam (a `RuntimeHelperSpec` + a per-platform `lower_<pkg>_helper`
  in `src/target/shared/code/<pkg>.rs`, dispatched from `code/mod.rs`, recognized in
  `runtime/mod.rs`, catalogued in `runtime/catalog.rs`): the emission is
  **arch-neutral** (built on `abi::` + `CodegenPlatform`; branches on OS *family*,
  never per-arch), so it moves like `Native`. Relocate `lower_<pkg>_helper` →
  `src/codegen/builtins/<pkg>/native.rs` (`use crate::target::shared::code::*;`,
  promote any `pub(super)` it needs — `HelperResult`/`HelperBody`,
  `raise_error_into`, `finalize_vreg_body_with_locals`), re-export it `pub(crate)`
  from `mod.rs`, and repoint the `code/mod.rs` dispatch call. Move the
  `RuntimeHelperSpec` consts + a `is_<pkg>_runtime_call` predicate into `mod.rs`;
  the shared `runtime/mod.rs` recognizer delegates to the predicate and the
  `catalog` imports the specs from codegen. The `RuntimeHelper::<Pkg>` enum variant,
  the catalog array entry, and the dispatch line **stay** in `src/target` — they
  are the runtime-call analogue of the `REGISTRY` module list, not `<pkg>` logic.
- **Transitivity trap.** A helper called *only* by a shared/non-`<pkg>` function is
  **not** `<pkg>`-only; it stays in `src/target`. Verify each helper's callers
  before moving it (census by effect, not by one name — a helper has many spellings).
- **Acceptance:** build clean; **`artifact-gate <pkg>` = 0 diffs** (this is where a
  diff means a real bug — objdump/`.ncode`-diff ONE fixture and fix it, never
  re-baseline).

### Phase 4 — The call-rewrite path (only if the package rewrites)

Two mechanisms exist; pick by whether members are **generics** or **concrete**
(memory: `mfb-package-rewrite-paths`):

- **Generic package** (members `FUNC … OF T`, rewritten in the monomorphizer, e.g.
  collections): add `<pkg>::internal_name(member)` + a `<pkg>_internal_callee`
  binding path in `src/monomorph/lower.rs`. `implementation_name` may return `None`.
- **Concrete package** (members concrete-typed, rewritten in `ir/lower.rs` via the
  `.or_else(|| builtins::<pkg>::implementation_name(..))` chain, e.g. encoding/csv/json/regex):
  `Implementation::Mfb`/`::Native` make the descriptor's `implementation_name`
  `None`, which **breaks that rewrite**. Keep an explicit `IMPL_NAMES:
  &[(&str,&str)]` table (`"<pkg>.slug" → "__<pkg>_slug"`) and have the package's
  `implementation_name` read it. Byte-identical because the rewrite string is
  unchanged. (The table also captures irregular pairings a derivation would miss —
  csv's `readRow` → `__csv_next`.)
- **Arity-keyed resolver variant** (datetime): the package's `implementation_name`
  is `fn(name, argc) -> Option<String>` (not `&'static str`) and is called on its
  own `ir/lower.rs` line, **not** the `.or_else` chain. It routes through the module
  resolver's `dispatch_implementation_name`, which is independent of a member's
  `Implementation` — so it keeps working unchanged for `Mfb` and `Custom` members
  alike. No `IMPL_NAMES` needed; just move the resolver + the two `dispatch_*` fns.
- **Acceptance:** `artifact-gate <pkg>` = 0 diffs; the package's monomorph/overload
  tests pass.

### Phase 5 — Migrate docs, man2

> **Citation repointing is NOT here — it belongs in the Phases 1–4 commit that
> moves the files.** The `man_citations_resolve` / `spec_citations_resolve` tests
> break the instant a file moves, independent of doc *content*. So each time you
> `git mv`/split a file, repoint its citations in the *same* commit and keep the
> suite green. Phase 5 is purely the metadata content.

- **Per-member docs.** Populate each descriptor's `doc_intro` (man `# title`
  sub-line), `doc_desc` (`## Description`, citations stripped), `doc_example`
  (`## Examples`, stripped) from `src/docs/man/builtins/<pkg>/*.md`. Members declared
  with a compact constructor (`datetime`'s `df`) layer docs via `with_intro`/
  `with_desc`/`with_example`; `mfb`/`native`/`custom` take intro/desc as params.
- **Module-level docs (a distinct, easy-to-forget step).** Populate the
  `BuiltinModule`'s own `doc_intro`/`doc_desc` from `<pkg>/package.md`'s title line
  + `## Description`. (Missed on csv the first time — `mfb man2 <pkg>` renders a
  blank overview without it.)
- **Citation-repointing rule** (used in Phases 1–4, restated here as the reference):
  the check is a **loose substring** match. A member body's `__<pkg>_<slug>` lives
  in its `func_*.rs` (Mfb) — repoint member citations there; helpers + arity bodies
  stay in `package.mfb`; `src/builtins/<pkg>.rs` refs → `mod.rs`; a relocated
  `native.rs`/spec → its new path. Use `\b` so `__<pkg>_add` doesn't match
  `__<pkg>_addDays`.
- man2 is already registry-generic (`show_man2` → `REGISTRY.module`), so no wiring
  is needed — just verify `mfb man2 <pkg>` and `mfb man2 <pkg> <fn>` render.
- **Acceptance:** `artifact-gate <pkg>` = 0 diffs (docs are metadata); citation
  tests pass; man2 renders the overview + each function.

### Phase 6 — Land

- `rustup run 1.96.0 cargo fmt --all` **and** `(cd repository && rustup run 1.96.0 cargo fmt)`.
- Full `cargo test --bin mfb` (never a single module); clippy clean on the new
  module (`cargo clippy --bin mfb`, no `#![allow(dead_code)]`); `artifact-gate <pkg>`
  and a spot-check of a **dependent** package's gate (a package whose `.mfb` calls
  `<pkg>::` — its `.ncode` shifts if you regressed `<pkg>`).
- Delete `src/builtins/<pkg>.rs` + `<pkg>*.mfb` (via `git rm`). Confirm the emptying
  goal: `grep -rE '__<pkg>_|<pkg>::|is_<pkg>_call|<Pkg>Resolver|builtins::<pkg>' src/target/`
  returns nothing (generic-word hits excepted).

---

## Rewire checklist (every external reference to the moved module)

Grep `[^:]<pkg>::` and `builtins::<pkg>` across `src/`, then fix by site:

- `src/codegen/registry.rs` — `REGISTRY` entry; any plan-72 parity harness wiring.
- `src/ir/lower.rs` — package-source injection (`augmented_project`) and the
  `implementation_name` rewrite `.or_else` chain.
- `src/monomorph/lower.rs` — overload-target selection / `is_overloaded`; the
  generic `<pkg>_internal_callee` binding path (generic packages only).
- `src/ir/verify/compat.rs` — `is_<pkg>_call` and friends.
- `src/syntaxcheck/mod.rs`, `src/resolver/mod.rs` — `augmented_project(&ast)`.
- `src/builtins/mod.rs` — dispatch helpers (`expected_arguments`, `argument_types`,
  `call_param_names`, `is_<pkg>_call`, `general_override_target`,
  `qualified_builtin_type`, resource-close, type-field lookups).
- Handle `crate::builtins::<pkg>::` vs bare `<pkg>::` (a `use crate::builtins;`
  module) separately so you don't double-prefix into `crate::crate::…`.

---

## Byte-identity & gotchas (hard-won)

- **A gate diff is a bug-hunt trigger, never "the design is dead."** Objdump/`.ncode`-
  diff ONE fixture, localize, fix (or correct a wrong prediction). A diff on a
  target you *expected* to change is the plan working.
- **Preserve the synthetic source path/doc labels** exactly (line/loc metadata).
- **Marker substitution restores exact bytes**: `source.replacen(marker, BODY, 1)`
  with `BODY` carrying no leading/trailing newline; the surrounding newlines come
  from `package.mfb`. Choose raw-string hashes so `"#…` cannot close early.
- **Fast-path / native-lower fns are free fns** (HRTB coercion; methods → E0308).
- **`super` changes meaning** after the move (was `crate::builtins`, now
  `crate::codegen::builtins`): re-path `package_source_glue!`, `map_type_parts`,
  etc.; prefer `crate::…` absolute paths for shared codegen imports.
- **Subagent edits can silently vanish** — `git diff --stat` before trusting any
  "tests pass" from delegated file moves.
- **No test/golden re-baseline** to make it pass (AGENTS.md): a real diff is a bug;
  a stale golden is proven benign + regenerated in its own pre-migration commit.
- **A migration can surface a stale golden in a *sibling* package.** Moving a shared
  `include_str!` file (regex relocating `unicode_gencat.mfb`, which `strings` also
  includes) flips the sibling's gate red even though your change is a byte-identical
  rename. Prove it benign and regenerate the sibling's sums in their **own** commit —
  don't fold it into the migration.
- **Moving a *generated* file is a lockstep edit.** Update its generator's output
  path, `scripts/check-generated.sh`, and `scripts/list_functions.py` together, then
  run `check-generated.sh` (it must still reproduce the file byte-for-byte).
- **Own the deviation.** If a package tempts you to skip the pattern (keep the table
  inline, leave a member `Custom` that could be `Mfb`, put shared data under one
  package), either do it right or *ask* — do not quietly deviate and justify it after.
  Every such shortcut here was caught in review and redone.

---

## Verification checklist (Phase 6 gate)

- [ ] `cargo build --bin mfb` clean, **0 warnings**.
- [ ] `cargo test --bin mfb` fully green (citations, monomorph, syntaxcheck,
      resolver, man2, package unit tests).
- [ ] `scripts/artifact-gate.sh target/release/mfb <pkg>` = **0 diffs**.
- [ ] A dependent package's gate still 0 diffs (ripple check).
- [ ] `cargo clippy --bin mfb` clean on `src/codegen/builtins/<pkg>/**`.
- [ ] `mfb man2 <pkg>` and `mfb man2 <pkg> <fn>` render intro/params/desc/examples.
- [ ] `grep -rE '__<pkg>_|is_<pkg>_call|builtins::<pkg>' src/target/` → empty.
- [ ] `src/builtins/<pkg>.rs` and `<pkg>*.mfb` deleted; both fmt passes run.

---

## Per-shape quick reference

- **Pure source** (csv, json, regex, http): Phases 1→2→3(Mfb)→4(concrete/IMPL_NAMES
  if it rewrites)→5. No `common/`, no `Native`. ~encoding.
- **Source + fast paths** (collections-like): add `Mfb.fast_path` + `common/`.
- **Native/inline** (bits, math, io, fs, os, thread, tls): Phase 3 is the bulk —
  `impl CodeBuilder` lowering → `func_*.rs`/`common/`, `Implementation::Native`,
  delete the `lower_<pkg>_call` dispatch, seam via `try_native_lower`. Largest
  `src/target` payload; land member-group by member-group, gate green each step.
- **Custom/resolver** (encoding's 2 overloaded names, crypto, datetime): move the
  `BuiltinResolver`; but classify **per member** — single-body members go on `Mfb`,
  only arity one-to-many + OS-seam stay `custom`. datetime: 37 `Mfb` + 4 arity-`Custom`
  + 3 OS-seam (`native.rs`). A resolver package still needs `assembled_source()`.
- **Data-only** (errorcode, general, testing): mostly relocate + docs; little to move.

## Commit sequence (what each phase lands)

One commit per phase, each green: (0) sibling+own stale-golden regens — separate
commits; (1) descriptor + `package.mfb` relocation + rewire + **citation repoint**;
(2) `func_*.rs` split; (3) implementation move (`Mfb` bodies / `Native` / `native.rs`
OS-seam) + **citation repoint**; (4) rewrite path; (5) docs (per-member + module).
Split large native/OS-seam packages further (datetime landed descriptor, then
syscall emission, then runtime specs as three commits). Never batch a file move
without its citation repoint.
