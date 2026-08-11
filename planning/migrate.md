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

| Member shape today | Migrates to | Mechanism |
|---|---|---|
| `Rewrite("__<pkg>_x")` / source generic, body in `<pkg>*.mfb` | `Implementation::Mfb { body, fast_path: None }` | body → Rust const + `'@@MFB_BODY:<slug>@@` marker in `package.mfb`, spliced by `assembled_source()` |
| Source generic **with** a native fast path in `src/target` | `Implementation::Mfb { body, fast_path: Some(..) }` | as above **plus** the fast-path fn moves into the `func_*.rs`; shared lowering → `common/` |
| `Same` + `Inline`/`Helper`, lowered natively in `src/target` (bits/math/io/…) | `Implementation::Native(lower)` | the `impl CodeBuilder { lower_<pkg>_* }` methods move into the package; shared lowering → `common/`; reached via the `try_native_lower` seam |
| `Custom` (argument-dependent, resolver-selected) | `Implementation::Custom` + package `BuiltinResolver` | descriptor via `BuiltinFunction::custom(...)`; resolver moves with the module |
| A pure constant / data-only member (no lowering) | stays data in the descriptor | relocate only |

A real package is usually a **mix** (e.g. `strings` has native scalar members + an
`.mfb` companion + Tier-B transforms). Handle each member by its row.

### Constructors (use the registry-wide ones — never a per-package `ef`/`mf` wrapper)

`src/codegen/registry.rs`:
- `BuiltinFunction::mfb(name, slug, intro, desc, errors, overloads, body)`
- `BuiltinFunction::mfb_with_fast_path(.., body, fast_path)`
- `BuiltinFunction::native(name, slug, intro, desc, errors, overloads, lower)`
- `BuiltinFunction::custom(name, slug, intro, desc, errors, overloads)`
- `.with_example(EX)` chains the `## Examples` block.

Package-local helpers stay only for **overload/parameter** construction (collections'
`custom`/`req`/`opt`, encoding's `ov`/`p`) — the descriptor-level `BuiltinFunction`
constructor is always the shared one.

---

## Target file layout

```
src/codegen/builtins/<pkg>/
  mod.rs            # module decls, ENCODING_FUNCTIONS-style table (func:: refs),
                    # resolver (if any), metadata tables, source glue, IMPL_NAMES,
                    # overload/param helpers, tests
  package.mfb       # ONLY for source-backed packages: shared private helpers +
                    # '@@MFB_BODY:<slug>@@ markers where member bodies were
  func_<name>.rs    # one per member: its INTRO/DESC/EX consts, its BODY const
                    # (Mfb) or native lowering fn (Native), and its descriptor
  common/           # ONLY if members share `impl CodeBuilder` lowering (Native/
    mod.rs          # fast-path packages). Holds the shared lowering methods that
    *.rs            # were `<pkg>`-only in src/target. Pure-source packages have none.
```

`common/` exists **iff** the package has native lowering that multiple members
share (collections has it; encoding does not). Do not invent an empty `common/`.

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
  `BuiltinResolver` and its `dispatch_*` helpers into `mod.rs` unchanged.
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
  `.or_else(|| builtins::<pkg>::implementation_name(..))` chain, e.g. encoding):
  `Implementation::Mfb`/`::Native` make the descriptor's `implementation_name`
  `None`, which **breaks that rewrite**. Keep an explicit `IMPL_NAMES:
  &[(&str,&str)]` table (`"<pkg>.slug" → "__<pkg>_slug"`) and have the package's
  `implementation_name` read it. Byte-identical because the rewrite string is
  unchanged.
- **Acceptance:** `artifact-gate <pkg>` = 0 diffs; the package's monomorph/overload
  tests pass.

### Phase 5 — Migrate docs, repoint citations, man2

- Populate each descriptor's `doc_intro` (man `# title` sub-line), `doc_desc`
  (`## Description`, citations stripped), `doc_example` (`## Examples`, stripped)
  from `src/docs/man/builtins/<pkg>/*.md`. Metadata only.
- **Repoint doc citations** (the `man_citations_resolve` / `spec_citations_resolve`
  tests are a **loose substring** check): a member body's `__<pkg>_<slug>` now lives
  in its `func_*.rs`/`mod.rs`, **not** `package.mfb` (which holds only helpers +
  markers). Repoint member citations → the Rust module; helper citations →
  `package.mfb`; old `src/builtins/<pkg>.rs` refs → the new `mod.rs`.
- man2 is already registry-generic (`show_man2` → `REGISTRY.module`), so no wiring
  is needed — just verify `mfb man2 <pkg>` and `mfb man2 <pkg> <fn>` render.
- **Acceptance:** `artifact-gate <pkg>` = 0 diffs (docs are metadata); citation
  tests pass; man2 renders intro/params/description/examples.

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
- **Custom/resolver** (parts of encoding, crypto, datetime): `BuiltinFunction::custom`
  + move the `BuiltinResolver`.
- **Data-only** (errorcode, general, testing): mostly relocate + docs; little to move.
