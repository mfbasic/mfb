# Move `src/builtins/` into `src/codegen/` Plan

Last updated: 2026-08-23
Effort: large (3h–1d)

The top-level crate module `crate::builtins` is not a package of builtin
lowerings — the plan-95 migration already moved every package's lowering into
`src/codegen/builtins/<pkg>/`. What remains at `src/builtins/` is the **builtin
dispatch facade** (`mod.rs`), the **resource-type registry** (`resource.rs`),
and the **test-desugar metadata** (`testing.rs`). All three are live and belong
under the codegen layer they front. This plan relocates the whole module into
`src/codegen/` and deletes `src/builtins/`, repointing every reference. It is
pure code motion: **no behavior change, and every codegen golden stays
byte-identical.**

The single checkable outcome: `src/builtins/` no longer exists, `mod builtins;`
is gone from `src/main.rs`, and `cargo build`/`cargo test --no-fail-fast` plus
the acceptance and artifact-gate suites pass with byte-identical `.ncode` across
all targets.

References:

- `AGENTS.md` — "Never edit a test/golden to pass" (byte-identity is the gate
  for pure code motion), citation-sweep and acceptance-harness obligations.
- `.ai/testing-gates.md` — artifact-gate, byte-identity, acceptance golden
  harness, citation sweeps.
- `.ai/build-tooling.md` — the two-path `cargo fmt` invocation (root `--all`
  does not reach `repository/`).
- `src/codegen/mod.rs`, `src/codegen/builtins/mod.rs` — the destination modules.

## Prerequisites

None. This is self-contained code motion against `main`; it depends on no other
plan and no bug fix.

| Must be true | Command | Status |
|---|---|---|
| Working tree builds clean on `main` | `cargo build 2>&1 \| tail -1` | MET (assumed; re-run before starting) |
| No in-flight rename of `src/builtins` | `git status --porcelain src/builtins` → empty | MET |

Everything below is written against the current `main`.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again
> before you decide to stop.

## 1. Goal

- `src/builtins/` is deleted; `mod builtins;` is removed from `src/main.rs`.
- Its three files live under `src/codegen/`:
  - `src/builtins/resource.rs` → `src/codegen/resource.rs` (`crate::codegen::resource`)
  - `src/builtins/testing.rs` → `src/codegen/builtins_testing.rs` (`crate::codegen::builtins_testing`)
  - `src/builtins/mod.rs` (the facade functions/tests) → **merged into** the
    existing `src/codegen/builtins/mod.rs` (`crate::codegen::builtins`)
- Every reference is repointed; the tree compiles and all suites pass.
- Every codegen artifact is **byte-identical** to pre-change (`.ncode`,
  `.ncodesum`, objdump) — proof this was pure code motion.

### Non-goals (explicit constraints)

- **No behavior change.** No function body is altered, no logic simplified, no
  delegation collapsed. Bodies move verbatim; only module paths change.
- **No API-shape change.** The facade functions keep their names, signatures,
  and `pub(crate)` visibility. Only their fully-qualified path changes
  (`crate::builtins::X` → `crate::codegen::builtins::X`).
- **No golden churn.** If any `.ncode`/`.ncodesum`/objdump byte changes, that is
  a bug introduced by this motion (an accidental logic edit) — root-cause and
  fix it; do not re-baseline.
- **Do not merge `testing.rs` into `codegen::builtins::testing`** — that path is
  already taken by the testing *package's lowering* (`src/codegen/builtins/testing/`).
  Keep the front-end test-desugar metadata a separate module.

## 2. Current State

`src/builtins/` has exactly three files:

- `src/builtins/mod.rs` (1129 lines) — the aggregate builtin dispatch facade.
  Most functions delegate to `crate::codegen::registry` / `crate::codegen::builtins::*`
  (e.g. `is_package_constant`, `builtin_package_name`, `call_return_type_name`),
  but several carry genuine dispatch logic (`resolve_call_return_type` special-cases
  `general`/`vector`/`strings`; the inline-TRAP census trio
  `inline_trap_unsupported` / `inline_builtin_raw_supported` /
  `inline_builtin_is_infallible`) and pure utilities with no other home
  (`split_top_level_commas`, `split_func_params_and_return`, `split_top_level_types`,
  `select_param_name_overload`, `exact`, `is_builtin_import`). It ends with a
  `#[cfg(test)] mod tests` block (~15 tests), one of which
  (`spec_section_18_package_list_matches_is_builtin_import`) reads a **literal
  citation string** `[[src/builtins/mod.rs:is_builtin_import]]` out of the spec.
- `src/builtins/resource.rs` — the data-driven RES resource-type registry
  (`ResourceInfo`, `ResourceKind`, `ResourceRegistry`, `state_type_name`,
  `base_resource_name`, `builtin_resource_close_function`, …).
- `src/builtins/testing.rs` — test-desugar metadata: `is_testing_call`,
  `TEST_ABORT_CODE`, `EXPECT_EQUAL`/`EXPECT_NEQUAL`/`EXPECT_TRAP`/`EXPECT_NTRAP`,
  `expect_arity`.

Wired in at `src/main.rs:5` (`mod builtins;`). The parent codegen module already
declares `pub(crate) mod builtins;` at `src/codegen/mod.rs:10` — so
`crate::codegen::builtins` already exists (the per-package lowering registry);
merging the facade into its `mod.rs` unifies the two.

Note the existing back-reference: `src/codegen/builtins/general/mod.rs:460` does
`use crate::builtins::exact;`. After the merge, `exact` lives in the same
`codegen::builtins` module, so that becomes `use super::exact;` (or
`crate::codegen::builtins::exact`).

### Measured populations

| What | Count | Command |
|---|---|---|
| `crate::builtins` references (src + repository) | 172 | `grep -rn "crate::builtins" src/ repository/ \| wc -l` |
| Bare `builtins::` module-relative refs in src (excl. `codegen::builtins`) | 146 | `grep -rn "[^:_a-zA-Z]builtins::" src/ \| grep -v "crate::builtins" \| grep -v "codegen::builtins" \| wc -l` |
| Distinct files touching `crate::builtins` | 59 | `grep -rl "crate::builtins" src/ repository/ \| wc -l` |
| `builtins::resource` references | 105 | `grep -rn "builtins::resource" src/ repository/ \| wc -l` |
| `builtins::testing` references | 13 | `grep -rn "builtins::testing" src/ \| wc -l` |
| Doc/spec/man citation lines to `src/builtins` | 17 | `grep -rn "src/builtins" src/docs/ \| wc -l` |

The `builtins::resource` (105) and `builtins::testing` (13) subsets partition
cleanly; the remaining `crate::builtins::` refs (172 − the resource/testing
subset within them) plus bare `builtins::` facade refs are the facade population.
The exact per-file split is discovered per file during execution (each file's
`use crate::builtins;` line and its bare `builtins::` uses move together).

### Verified properties

- **The facade is live, not dead.** Read `src/builtins/mod.rs` in full: every
  `pub(crate)` function is reachable from the 59 dependent files (monomorph, ir,
  syntaxcheck, resolver, target, types, codegen, testing). Deleting it breaks the
  build — hence *relocate*, not *remove*. Verified by reading the file and the
  reference census above.
- **`testing.rs` cannot occupy `codegen::builtins::testing`.** That path is the
  testing package's lowering directory (`src/codegen/builtins/testing/mod.rs`
  exists). Verified: `ls src/codegen/builtins/testing/`. Hence the separate
  `codegen::builtins_testing` home.
- **Doc citations are gate-checked.** The test
  `spec_section_18_package_list_matches_is_builtin_import` in `mod.rs` reads the
  literal `[[src/builtins/mod.rs:is_builtin_import]]` from
  `src/docs/spec/language/18_builtin-functions.md`. Moving the file obligates
  updating both the 17 doc citation lines AND that test's `.find(...)` literal, or
  the test fails. Verified by reading the test body.

## 3. Design Overview

Three independent moves, each a leaf relocation that compiles and passes green on
its own, ordered lowest-coupling first:

1. **`resource.rs`** (Phase 1) — a self-contained submodule with no dependency on
   the facade. Moves to `src/codegen/resource.rs`. 105 refs repoint
   `builtins::resource::` → `codegen::resource::`. Lowest coupling → first.
2. **`testing.rs`** (Phase 2) — self-contained metadata, 13 refs. Moves to
   `src/codegen/builtins_testing.rs`.
3. **`mod.rs` facade** (Phase 3) — the remaining facade functions/tests merge
   into `src/codegen/builtins/mod.rs`; `src/builtins/` is deleted and
   `mod builtins;` removed from `main.rs`. Largest blast radius → last.

**Byte-identity IS this plan's correctness gate.** This is provably-neutral code
motion — bodies move verbatim, only paths change. "Byte-identical `.ncode`,
`.ncodesum`, and objdump across all targets" is the acceptance check for every
phase. A diff means an accidental logic edit slipped in during the move:
root-cause it (objdump one fixture), fix it, and the gate passes. A diff never
means "the design is dead."

**Rejected alternative — a separate `codegen::builtins_registry` module for the
facade.** Keeps the facade visibly distinct from per-package lowering, but leaves
two sibling `builtins`-named modules under codegen and a `crate::builtins::exact`
→ cross-module import from `codegen::builtins::general`. Merging into the existing
`codegen::builtins::mod` gives one unified module the user asked for ("into
codegen where it already should have been") and turns that cross-import into a
`super::` reference. Chosen.

**Rejected alternative — keep `crate::builtins` as a `pub use` alias.** Leaves the
old path resolvable and the 318 call sites untouched, but does not delete
`src/builtins/`, which is the explicit goal. Rejected.

## Compatibility / Format Impact

None externally observable. This is an internal module rename. No public API,
file/wire format, layout, or ABI changes. All codegen output is byte-identical.

## Phases

> **NOTE — keep the checkboxes current as you go**, tick `- [x]` in the same
> commit as the work, and fill each `Commit:` line the moment the phase lands.

### Phase 1 — relocate `resource.rs`

Move the resource registry to `src/codegen/resource.rs`; it has no facade
dependency, so it lands cleanly first.

- [ ] `git mv src/builtins/resource.rs src/codegen/resource.rs`.
- [ ] Add `pub(crate) mod resource;` to `src/codegen/mod.rs` (near the existing
      `pub(crate) mod builtins;`).
- [ ] In `src/builtins/mod.rs`, replace `pub(crate) mod resource;` +
      `pub(crate) use resource::{...}` with `pub(crate) use crate::codegen::resource;`
      and repoint its internal `resource::` uses — keeps `crate::builtins::resource`
      resolving for the not-yet-moved facade during this phase.
- [ ] Repoint the 105 `builtins::resource::` references to
      `crate::codegen::resource::` across the dependent files (incl.
      `src/cli/man.rs:683`, `repository/` refs). For a file with
      `use crate::builtins;` and bare `builtins::resource::X`, either add
      `use crate::codegen::resource;` and drop the `builtins::` qualifier, or
      fully-qualify — match the file's existing style.
- [ ] Update the 2 doc citations to `src/builtins/resource.rs`
      (`src/docs/spec/architecture/21_type-name-encoding.md:137`,
      `src/docs/spec/architecture/09_modules.md:39`) → `src/codegen/resource.rs`.
- [ ] Tests: no new tests; the existing resource unit tests move with the file.

Acceptance: `cargo build` and `cargo test --no-fail-fast` green; `artifact-gate.sh all`
reports **byte-identical** `.ncode`/`.ncodesum` for every target (no diff).
Commit: —

### Phase 2 — relocate `testing.rs`

Move the test-desugar metadata to `src/codegen/builtins_testing.rs`.

- [ ] `git mv src/builtins/testing.rs src/codegen/builtins_testing.rs`.
- [ ] Add `pub(crate) mod builtins_testing;` to `src/codegen/mod.rs`.
- [ ] In `src/builtins/mod.rs`, replace `pub(crate) mod testing;` with
      `pub(crate) use crate::codegen::builtins_testing as testing;` (keeps
      `crate::builtins::testing` resolving during this phase) — or repoint the 13
      refs directly (below) and drop the re-export.
- [ ] Repoint the 13 `builtins::testing::` references to
      `crate::codegen::builtins_testing::` (`src/testing/desugar/mod.rs`,
      `src/testing/desugar/expect.rs`, `src/syntaxcheck/inference.rs`).
- [ ] Tests: the `testing.rs` unit tests (if any) move with the file.

Acceptance: `cargo build` and `cargo test --no-fail-fast` green; `artifact-gate.sh all`
byte-identical for every target.
Commit: —

### Phase 3 — merge the facade, delete `src/builtins/` (largest blast radius)

Fold `src/builtins/mod.rs` into `src/codegen/builtins/mod.rs`, delete the old
directory, and repoint every remaining reference.

- [ ] Append the facade functions, constants, and the `#[cfg(test)] mod tests`
      block from `src/builtins/mod.rs` into `src/codegen/builtins/mod.rs`
      **verbatim**. Resolve the now-local references: `general::` is already a
      child module here (drop the `use crate::codegen::builtins::general;`
      aliasing comment); `resource` / `builtins_testing` referenced via
      `crate::codegen::…`.
- [ ] Update `src/codegen/builtins/general/mod.rs:460`
      `use crate::builtins::exact;` → `use super::exact;`.
- [ ] Repoint all remaining `crate::builtins::<fn>` and bare `builtins::<fn>`
      references (facade functions: `is_builtin_call`, `resolve_call_return_type`,
      `native_builtin_target`, `arity`, `expected_arguments`, `argument_types`,
      `is_builtin_import`, `is_builtin_type`, `is_package_constant`,
      `split_top_level_commas`, `split_func_params_and_return`,
      `split_top_level_types`, `select_param_name_overload`, `general_override_target`,
      `qualified_builtin_type`, `resource_close_function`, `is_resource_type`,
      `is_thread_sendable_resource_type`, `inline_*`, `call_param_names`,
      `call_param_name_overloads`, `call_return_type_name`, `builtin_package_name`,
      `is_nonescaping_callback_arg`, `is_internal_only_call`, `is_builtin_member`,
      `package_constant_type_name`, `package_constant_value`) →
      `crate::codegen::builtins::<fn>`. Update the `use crate::builtins;` import
      lines (`src/ir/mod.rs:24`, `src/ir/verify/mod.rs:46`,
      `src/codegen/memory/value/builder_value_semantics.rs:2`,
      `src/codegen/memory/data/data_objects.rs:2`,
      `src/codegen/engine/types/type_utils.rs:2`,
      `src/codegen/engine/validation/validation.rs:4`,
      `src/codegen/engine/value/builder_values.rs:2`, `src/binary_repr/mod.rs:1`,
      `src/resolver/mod.rs:7`, `src/syntaxcheck/mod.rs:10`,
      `src/binary_repr/tests/writer_tests.rs:81`) to
      `use crate::codegen::builtins;`.
- [ ] `git rm -r src/builtins/` (now empty of source) and remove `mod builtins;`
      from `src/main.rs:5`.
- [ ] Update the remaining doc/man citations (facade functions and the bare
      `src/builtins/` directory references) — the ~15 lines in
      `src/docs/man/lambda/package.md`, `src/docs/spec/memory/07_runtime-helper-abi.md`,
      `src/docs/spec/language/06_functions.md`, `…/18_builtin-functions.md`,
      `…/13_modules-and-packages.md`, `src/docs/spec/architecture/19_internal-naming.md`,
      `…/21_type-name-encoding.md`, `…/04_ir.md`, `…/09_modules.md`,
      `src/docs/spec/stdlib/06_url.md`, `…/12_bits.md` — repointing
      `[[src/builtins/mod.rs:...]]` → `[[src/codegen/builtins/mod.rs:...]]` and the
      bare `src/builtins/` → `src/codegen/` where it names the directory.
- [ ] Update the moved `spec_section_18_package_list_matches_is_builtin_import`
      test's `.find("[[src/builtins/mod.rs:is_builtin_import]]")` literal to the new
      `[[src/codegen/builtins/mod.rs:is_builtin_import]]` (must match the doc edit
      above).
- [ ] Tests: the ~15 facade unit tests move into `codegen/builtins/mod.rs`'s test
      module and must still pass under their new path.

Acceptance: `src/builtins/` is gone (`test ! -d src/builtins`), `grep -rn "crate::builtins\|mod builtins" src/` returns nothing; `cargo test --no-fail-fast` green; `artifact-gate.sh all` byte-identical for every target.
Commit: —

## Validation Plan

- Tests: `cargo test --no-fail-fast` (per memory: plain `cargo test` fail-fast
  skips the `rt_*` tests that sort after `golden.rs`).
- Coverage check: the moved unit tests (resource, testing, facade) still run —
  confirm by name in the test output, not just an overall green.
- Runtime proof: the acceptance golden harness (`test-accept.sh` with a `/tmp`
  scratch dir per memory — never a real dir as arg 2) passes unchanged; a couple
  of pre-existing harness mismatches noted in memory are not ours.
- Doc sync: all 17 `src/builtins` citations updated; the §18 spec-list test
  passes against the new citation literal; no dangling `[[src/builtins/...]]`
  citation remains (`grep -rn "\[\[src/builtins" src/`).
- Acceptance: `artifact-gate.sh all` shows **zero** `.ncode`/`.ncodesum` diffs
  across all targets (byte-identity is the gate); then
  `rustup run 1.96.0 cargo fmt --all && (cd repository && rustup run 1.96.0 cargo fmt)`.

## Open Decisions

- **Facade home**: merge into `codegen::builtins::mod` (recommended) vs. a
  separate `codegen::builtins_registry` module. Merging is chosen (§3); revisit
  only if `codegen/builtins/mod.rs` becomes unwieldy.
- **`testing.rs` name**: `codegen::builtins_testing` (recommended) vs.
  `codegen::test_desugar`. The former keeps it lexically adjacent to the builtins
  it describes; either is acceptable and touches only 13 refs. (§1)

## Corrections

<Filled in during execution.>

## Summary

Pure code motion with a large but mechanical reference count (172
`crate::builtins` + 146 bare `builtins::` refs across 59 files, plus 17 doc
citations). The only real risk is an accidental logic edit sneaking in while
moving bodies — which the byte-identity artifact gate catches deterministically.
No behavior, API, or format changes; `src/builtins/` ceases to exist and its
three files live under `src/codegen/`.
