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
| Working tree builds clean on `main` | `cargo build 2>&1 \| tail -1` | MET (2026-08-23: `Finished dev profile`) |
| No in-flight rename of `src/builtins` | `git status --porcelain src/builtins` → empty | MET (empty) |

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

- [x] ~~`git mv src/builtins/resource.rs src/codegen/resource.rs`~~ — CORRECTED:
      `src/codegen/resource.rs` path is taken by an existing `src/codegen/resource/`
      **directory** module (wires `cleanup`). Merged the resource-registry body into
      `src/codegen/resource/mod.rs` instead (preserves `crate::codegen::resource::X`
      for the repoint), `git rm src/builtins/resource.rs`. See Corrections.
- [x] ~~Add `pub(crate) mod resource;` to `src/codegen/mod.rs`~~ — moot: already
      present at `src/codegen/mod.rs:20` (the existing directory module).
- [x] In `src/builtins/mod.rs`, replaced `pub(crate) mod resource;` +
      `pub(crate) use resource::{...}` with `use crate::codegen::resource;`
      (the `ResourceInfo/Kind/Registry` re-export consumers were repointed directly
      to `crate::codegen::resource::`, so no facade re-export is needed).
- [x] Repointed all `builtins::resource::` module-path references (82: 75 qualified
      `crate::builtins::resource::` + 7 bare) plus the 6 `builtins::{ResourceInfo,
      ResourceKind,ResourceRegistry}` re-export refs → `crate::codegen::resource::`.
      `grep -rn "builtins::resource::" src/ repository/` → 0. Dropped the now-unused
      `use crate::builtins;` from `validation.rs` (its only facade use was resource).
- [x] Updated the 2 doc citations (`…/21_type-name-encoding.md:137`,
      `…/09_modules.md:39`) → `[[src/codegen/resource/mod.rs…]]`.
- [x] Tests: the 5 resource unit tests moved with the body; all pass under
      `codegen::resource::tests::*`.

Acceptance: `cargo build` green; the 5 `codegen::resource::tests::*` pass. Full
`cargo test --no-fail-fast` + release `artifact-gate.sh all` byte-identity verified
cumulatively at Phase 3 (same worktree, cumulative code motion).
Commit: aebb059bf

### Phase 2 — relocate `testing.rs`

Move the test-desugar metadata to `src/codegen/builtins_testing.rs`.

- [x] `git mv src/builtins/testing.rs src/codegen/builtins_testing.rs`.
- [x] Added `pub(crate) mod builtins_testing;` to `src/codegen/mod.rs`.
- [x] In `src/builtins/mod.rs`, removed `pub(crate) mod testing;` outright (no
      facade fn references it) and repointed every ref directly — chose the
      repoint-and-drop path the plan offered over a transitional re-export.
- [x] Repointed the facade `testing` refs → `crate::codegen::builtins_testing::`:
      the 5 qualified `crate::builtins::testing` files (`ir/lower.rs`,
      `codegen/builtins/testing/mod.rs` doc link, `testing/desugar/{mod,expect}.rs`,
      `syntaxcheck/inference.rs`) via a `crate::builtins::testing`-only sed (safe:
      distinct substring from the `crate::codegen::builtins::testing` *package*
      module — `registry/mod.rs:1517`'s `::register` stayed intact) + the 2 bare
      refs (`resolver/resolution.rs:1222`, `syntaxcheck/inference.rs:261`) by hand.
      `grep crate::builtins::testing` → 0.
- [x] Tests: the 5 `testing.rs` unit tests moved with the file; all pass under
      `codegen::builtins_testing::tests::*`.

Acceptance: `cargo build` green; the 5 `codegen::builtins_testing::tests::*` pass.
Full `cargo test --no-fail-fast` + release `artifact-gate.sh all` byte-identity
verified cumulatively at Phase 3.
Commit: 98bc5a4ab

### Phase 3 — merge the facade, delete `src/builtins/` (largest blast radius)

Fold `src/builtins/mod.rs` into `src/codegen/builtins/mod.rs`, delete the old
directory, and repoint every remaining reference.

- [x] Appended the facade functions, constants, and `#[cfg(test)] mod tests`
      block from `src/builtins/mod.rs` into `src/codegen/builtins/mod.rs`
      **verbatim** (below the `pub(crate) mod X;` package declarations, under a
      section-header comment). Dropped `use crate::codegen::builtins::general;`
      (`general` is the child module — `general::` resolves directly); kept
      `use crate::codegen::resource;`; `builtins_testing` unused by the facade.
- [x] `src/codegen/builtins/general/mod.rs:460` `use crate::builtins::exact;` →
      `use crate::codegen::builtins::exact;` (the uniform `crate::builtins::` sed;
      functionally identical to the plan's suggested `super::exact` — `exact` is
      `pub(super)`, visible to `codegen::builtins::general` either way).
- [x] Repointed all remaining fully-qualified `crate::builtins::<fn>` (32 files, sed
      `crate::builtins::` → `crate::codegen::builtins::`, safe — distinct substring
      from existing `crate::codegen::builtins::`) and the 9 `use crate::builtins;`
      module imports → `use crate::codegen::builtins;`. Bare `builtins::<fn>` refs
      resolve through `use super::*` re-export of those aliases (edition 2021).
      `grep crate::builtins` (src+repository) → 0.
- [x] `git rm -r src/builtins/` and removed `mod builtins;` from `src/main.rs:5`.
- [x] Updated the doc/man citations: `[[src/builtins/mod.rs…]]` →
      `[[src/codegen/builtins/mod.rs…]]` (9 files) and the 2 bare `[[src/builtins/]]`
      directory citations → `[[src/codegen/builtins/]]` (`09_modules.md:40`,
      `19_internal-naming.md:4`), plus the prose `src/builtins/` in
      `07_runtime-helper-abi.md`, `src/target/shared/runtime/mod.rs`,
      `src/internal_name.rs`. `grep "\[\[src/builtins" src/` → 0. (The remaining
      `src/builtins/<pkg>.rs` mentions in code comments are historical "relocated
      from the deleted …" notes from plan-95, referring to files that exist at
      neither path — left as accurate history.)
- [x] Updated the moved `spec_section_18…` test's `.find(…)` literal to
      `[[src/codegen/builtins/mod.rs:is_builtin_import]]` (matches the §18 doc edit);
      the test passes.
- [x] Tests: the 20 facade unit tests run under `codegen::builtins::tests::*` and
      all pass.

Acceptance: `src/builtins/` is gone (`ls src/builtins` → No such file); top-level
`crate::builtins` = 0 and `mod builtins;` appears ONLY at `src/codegen/mod.rs:10`
(the legitimate `codegen::builtins` declaration — the plan's blanket
`grep "mod builtins"` was imprecise, corrected here). Full `cargo test
--no-fail-fast` green (exit 0, 0 failures); release `artifact-gate.sh all`:
**1248 tests, 1716 goldens, 0 diff(s)** — byte-identity holds for every target.
`test-accept.sh` shows 2 mismatches that reproduce identically on the baseline
(commit 50c953142) — pre-existing stdin-EOF environment sensitivity in
`tests/acceptance/src/io.mfb`, not ours (proven via a detached baseline build).
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

- **Phase 1 destination path collision (`src/codegen/resource.rs` already taken).**
  The plan assumed `crate::codegen::resource` was free and prescribed
  `git mv src/builtins/resource.rs src/codegen/resource.rs`. Reality:
  `ls src/codegen/resource/` shows an existing **directory** module
  (`src/codegen/resource/mod.rs` wiring `pub(crate) mod cleanup;`), and
  `src/codegen/mod.rs:20` already declares `pub(crate) mod resource;`. Resolution:
  merged the resource-registry body verbatim into `src/codegen/resource/mod.rs`
  (below the existing `mod cleanup;`), so `crate::codegen::resource::X` resolves
  exactly as the plan's repoint intended and the two `mod`-declaration boxes became
  moot. The `resource.rs` `//!` header folded into the module's own doc block.
  Also: the `crate::builtins::{ResourceInfo,ResourceKind,ResourceRegistry}`
  re-export (6 refs in `syntaxcheck/{mod,link}.rs`) was repointed straight to
  `crate::codegen::resource::` rather than carried as a facade re-export, and
  `validation.rs`'s `use crate::builtins;` (Phase-3 repoint target) became unused
  once its lone resource ref moved, so it was deleted in Phase 1.

- **Phase 3 acceptance grep was imprecise.** The plan wrote the acceptance as
  `grep -rn "crate::builtins\|mod builtins" src/` returns nothing. But
  `src/codegen/mod.rs:10` legitimately declares `pub(crate) mod builtins;` for the
  merged `codegen::builtins` module — that MUST stay. The correct, checkable
  invariant is: top-level `crate::builtins` references = 0, and `mod builtins;`
  appears only at `src/codegen/mod.rs:10`. Verified both.

- **Phase 3 `use crate::builtins;` file list was stale (11 → 9).** The plan listed
  11 import sites including `validation.rs` (deleted in Phase 1) and
  `binary_repr/tests/writer_tests.rs`. At Phase 3 only 9 `use crate::builtins;`
  imports remained (`writer_tests.rs` used `use crate::builtins::split_top_level_types`,
  a fully-qualified item import handled by the `crate::builtins::` sed, not the
  module-import sed). All repointed; `grep crate::builtins` → 0.

- **`test-accept.sh` 2 mismatches are pre-existing, not plan-103.** The acceptance
  runtime harness reports 2 mismatches — the `[1] acceptance` behavioral test's
  `input reads at end of input` group (`tests/acceptance/src/io.mfb:134–158`),
  which `expectTrap(…, ErrEndOfFile)` guarded by `IF io::pollInput(0)`. Under this
  harness's stdin state the reads don't trap. Built the baseline binary at the
  fork commit (50c953142) in a detached worktree and ran the identical harness:
  **same exit 1, same "2 mismatch(es)", same 5 io.mfb X markers** — so this is
  pre-existing stdin-environment sensitivity, independent of the code motion (which
  is byte-identical anyway: 0 `.ncode` diffs across every target).

## Summary

Pure code motion with a large but mechanical reference count (172
`crate::builtins` + 146 bare `builtins::` refs across 59 files, plus 17 doc
citations). The only real risk is an accidental logic edit sneaking in while
moving bodies — which the byte-identity artifact gate catches deterministically.
No behavior, API, or format changes; `src/builtins/` ceases to exist and its
three files live under `src/codegen/`.
