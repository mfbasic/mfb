# plan-176-B: Split tests/runtime, part 1 (38 integration-test files)

Last updated: 2026-10-17
Effort: large (3h–1d)
Depends on: plan-176-A

This sub-plan splits the first 38 over-budget integration-test files under `tests/runtime/`
(`rt_big_int.rs` through `rt_package_dependency_closure.rs`, plus `rt_foreign_entry_callee_saved.rs`
and `rt_inplace_self_update.rs`; the list is `/tmp/loc/groups.json["B"]`). Together they hold
20,661 test LOC and 367 `#[test]` fns. Each file keeps its root as the `[[test]]` target and gains
topic and support modules under `tests/runtime/<name>/`, declared with `#[path]`, the layout
plan-176-A verified. It is pure code motion: no test is added, removed or renamed, no `[[test]]`
target changes, and no assertion text changes. The exceptions are a single `include_str!`
path that gains a `../` because its file moves one directory deeper, and one natural extraction:
`word_boundaries_match_an_independent_oracle`'s inline MFBASIC program moves verbatim to
`tests/runtime/rt_big_int/word_boundaries.mfb` and is read with `include_str!`.

**Single outcome:** after B lands, `cargo test --test file_size_budget` prints no row for any
path under `tests/runtime/` that this letter owns, and `tests/guards/file_size_ceilings.txt`
has no `ceiling` row for them. The only rows left are the two `exempt` rows B adds:
`tests/runtime/rt_big_int/powers.rs` and `tests/runtime/rt_closure_single_owner/cases.rs`.

References:

- `planning/plan-176-A-file-size-guard.md`: the rule, the counting rules, the guard, the
  ceiling-file format, the `MFB_LOC_TIGHTEN` ratchet, and the integration-test layout
  (`#[path = "<name>/<topic>.rs"] mod <topic>;`, verified there in §2 "Verified properties").
- `.ai/build-tooling.md` § "Splitting an over-budget file (plan-176)" (written by A Phase 3):
  the split conventions this letter applies.
- `.ai/testing-gates.md`: test-target and acceptance-gate hazards. It names
  `rt_debug_report` (:348), `rt_debug_soak`'s `BLOCK_BOUND` (:865), `rt_list_append_growth_bounds` (:871) and
  `rt_inplace_self_update`'s ignored grid tests (:77). All of these stay valid (§4.0).
- `.ai/collections.md` (:12, :92, :97, :577, :588) and `.ai/codegen-invariants.md` (:537)
  name root paths or targets of this letter. `.ai/arch-abi.md` (:309, :522) does the same. All
  stay valid because roots do not move.
- Spec topics: none. No `[[tests/runtime/<file>.rs:…]]` citation names a B file
  (`grep -rnoE '\[\[tests/runtime/[a-z_0-9]+\.rs[^]]*\]\]' src/docs .ai` → 0 hits).
  `src/docs/spec/architecture/18_math-kernels.md:65` and `src/docs/spec/app/04_term-backend.md:342` name root
  paths in prose and stay valid.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-176-A is complete and archived | `ls planning/plan-176-A-* planning/completed/plan-176-A-*` → only `planning/completed/plan-176-A-file-size-guard.md` | NOT MET (2026-10-14) |
| The guard and the oracle exist (A Phases 1–2) | `cargo test --test file_size_budget` → passes; `python3 -I tools/loc-census/census.py --file tests/runtime/rt_big_int.rs` → `0 2584` | NOT MET (2026-10-14; lands with A) |
| This letter's files have no uncommitted changes | `git status --porcelain -- tests/runtime .ai/testing-gates.md .ai/collections.md tests/guards/file_size_ceilings.txt tools/loc-census/census.py` → empty | MET (2026-10-14: empty output) |

Everything below is written against the world where these hold.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again
> before you decide to stop. Never act on a status you did not just verify — a
> prerequisite recorded NOT MET may have landed since, and one recorded MET may
> have regressed.
>
> **If you stop, report the current status of *all* prerequisites** — not only
> the one that blocked you. The reader needs to know how far off the gate is,
> not just which row you hit first.

## 1. Goal

- Every file of this letter, and every file it creates, is ≤ 250 test LOC by
  `tools/loc-census/census.py --file`. The two exceptions are the exempt files named above,
  each carrying an `exempt` row with its reason in both `tests/guards/file_size_ceilings.txt`
  and `tools/loc-census/census.py`'s `EXEMPT` table.
- For every one of the 38 targets, the sorted list of test names from `cargo test --test <t> -- --list`,
  with module prefixes stripped, is identical before and after. Every target passes
  `cargo test --test <t>`.

### Non-goals (explicit constraints)

- No behavior change: no test body, assertion, expected-output literal, MFBASIC program text
  or `#[ignore]` reason changes. The only text edits inside moved code are the decided ones:
  `pub(super)` on support items (§4 "Surface"), the one `include_str!` repoint (§4.38), and
  the word_boundaries program extraction (§4.28), whose string value is byte-identical.
- No `[[test]]` target is added, renamed or re-pathed. `Cargo.toml` is not edited.
- No test fn is renamed. The test path gains a module prefix (`representation::a_defaulted_int_is_canonical_zero`),
  and that is the only visible change. Substring filters such as `cargo test --test rt_big_int
  defaulted` and `--ignored` selection keep working.
- No test moves between targets, and no helper is promoted into `tests/common/`.

## 2. Current State

### Files and measured sizes

Test LOC per file are from the planning oracle on the working tree of `17a9b5799`
(`python3 -I /tmp/loc/census.py "$PWD"`; after A it is
`python3 -I tools/loc-census/census.py --file <path>`). Test counts are
`grep -cE '#\[test\]' <file>`; no file uses another test attribute.

| File | Test LOC | Tests | New files | Largest part after |
|---|---|---|---|---|
| `tests/runtime/rt_big_int.rs` | 2584 | 17 | 10 (one `.mfb`) | 1272 (exempt) |
| `tests/runtime/rt_callback_failure_frees.rs` | 285 | 1 | 1 | 169 |
| `tests/runtime/rt_closure_single_owner.rs` | 574 | 1 | 1 | 497 (exempt) |
| `tests/runtime/rt_debug_arena.rs` | 418 | 8 | 3 | 173 |
| `tests/runtime/rt_debug_report.rs` | 500 | 9 | 3 | 188 |
| `tests/runtime/rt_debug_soak.rs` | 1164 | 49 | 10 | 145 |
| `tests/runtime/rt_domain_error_frees_locals.rs` | 306 | 1 | 1 | 157 |
| `tests/runtime/rt_error_model_recovery.rs` | 663 | 4 | 4 | 224 |
| `tests/runtime/rt_foreign_entry_callee_saved.rs` | 480 | 6 | 3 | 145 |
| `tests/runtime/rt_foreign_type_reexport.rs` | 270 | 3 | 2 | 104 |
| `tests/runtime/rt_global_argument_reassigned_by_callee.rs` | 263 | 7 | 2 | 218 |
| `tests/runtime/rt_hof_transient_bytes.rs` | 293 | 2 | 2 | 204 |
| `tests/runtime/rt_imported_package_global.rs` | 291 | 10 | 2 | 141 |
| `tests/runtime/rt_imported_type_name_collision.rs` | 304 | 10 | 2 | 147 |
| `tests/runtime/rt_imported_union_enum_members.rs` | 315 | 9 | 2 | 122 |
| `tests/runtime/rt_inline_trap_expression_boundary.rs` | 406 | 9 | 3 | 132 |
| `tests/runtime/rt_inline_trap_nested_call.rs` | 503 | 20 | 4 | 143 |
| `tests/runtime/rt_inline_trap_raising_operator.rs` | 556 | 21 | 3 | 191 |
| `tests/runtime/rt_inline_trap_short_circuit.rs` | 477 | 11 | 3 | 188 |
| `tests/runtime/rt_inline_trap_success_alloc.rs` | 437 | 2 | 3 | 142 |
| `tests/runtime/rt_inline_trap_union_bind.rs` | 251 | 6 | 2 | 117 |
| `tests/runtime/rt_inplace_failure_atomic.rs` | 494 | 3 | 3 | 143 |
| `tests/runtime/rt_inplace_self_update.rs` | 1572 | 4 | 11 | 215 |
| `tests/runtime/rt_io_key_readers.rs` | 563 | 13 | 4 | 170 |
| `tests/runtime/rt_json_bounds.rs` | 308 | 4 | 3 | 165 |
| `tests/runtime/rt_list_append_growth_bounds.rs` | 284 | 8 | 2 | 165 |
| `tests/runtime/rt_list_element_record_borrow.rs` | 645 | 21 | 4 | 171 |
| `tests/runtime/rt_list_take_drop_in_place.rs` | 502 | 5 | 4 | 192 |
| `tests/runtime/rt_map_integer_key_probe_cost.rs` | 355 | 4 | 2 | 175 |
| `tests/runtime/rt_math_fixed_trig_accuracy.rs` | 617 | 3 | 4 | 174 |
| `tests/runtime/rt_math_float_trig_large_angles.rs` | 384 | 3 | 3 | 210 |
| `tests/runtime/rt_moved_handle_guards.rs` | 307 | 2 | 2 | 203 |
| `tests/runtime/rt_native_io_runtime.rs` | 581 | 15 | 4 | 170 |
| `tests/runtime/rt_native_term_runtime.rs` | 1038 | 20 | 6 | 210 |
| `tests/runtime/rt_nothing_collections.rs` | 279 | 10 | 2 | 110 |
| `tests/runtime/rt_overloaded_function_values.rs` | 454 | 27 | 3 | 153 |
| `tests/runtime/rt_owned_argument.rs` | 643 | 13 | 4 | 166 |
| `tests/runtime/rt_package_dependency_closure.rs` | 295 | 6 | 2 | 124 |
| **Total (38 files)** | **20,661** | **367** | **130** (129 `.rs` + 1 `.mfb`) | |

Totals: summed from the per-file map headers in `/tmp/loc/maps/B.md`. Part sizes are
`python3 -I /tmp/loc/span.py <file> <a> <b>` summed over each part's line ranges, plus the
lines the split adds (§3).

### Verified properties

- **The `#[path]` layout works for integration-test targets and keeps name filters.**
  plan-176-A §2 probe: `cargo test --test rt_demo -- --list` listed `lists::list_a: test`, and
  `cargo test --test rt_demo list_a` ran 1 test.
- **A topic reaches the root's imports, `common` helpers and support globs through
  `use super::*;` with no warnings.** Probe `/tmp/loc_bcd_probe` (a `[[test]]` root with
  `mod common; use common::*; use std::collections::HashMap;`, a topic using `HashMap`, a bare
  `helper()` from `common` and `CASES` from a sibling support file, and a root `use data::*;`):
  `cargo test --test t1` → `test result: ok. 1 passed` and no warning. A root import used only
  by a topic is not reported unused. Neither is a root support glob used only by a topic.
- **Glob cycles compile without warnings.** Probe `/tmp/loc/probe2`
  (`mod.rs: use a::*; use b::*;`, `a.rs`/`b.rs: use super::*;`) printed `6` with no warning.
- **`use super::*;` in a file that references no parent item warns.** Same probe, with a
  data-only file starting with `use super::*;`: `warning: unused import: super::*`. Such files
  therefore omit the line (§3). Which support files reference no parent item was measured by
  listing the identifiers outside string literals in each item's line range
  (`python3 -I /tmp/loc_bcd_probe/s/idents.py <file> <a-b>…`): each of the eight files marked "no `use super::*;`" in §4 names
  only its own items and std prelude names.
- **`tests/guards/test_targets_registered.rs` ignores `tests/runtime/<name>/*.rs`.**
  `test_files_in_subdirectories` reads only the direct `.rs` children of each `tests/<dir>/`
  (plan-176-A §2). No topic file is named `main.rs`.
- **Every B test compiles on the macOS development host,** so `-- --list` there counts every
  test. No B file gates a test or a moved item to a non-macOS OS:
  `grep -nE 'cfg\((target_os = "(linux|windows)"|windows|not\(unix\))' <the 38 files>` finds only
  `#[cfg(any(target_os = "macos", target_os = "linux"))]` (`rt_moved_handle_guards.rs`, 1 hit)
  and one `#[cfg(not(unix))]` statement inside the body of
  `native_term_did_resize_reports_false_without_a_resize` (`rt_native_term_runtime.rs:787`).
  That statement moves verbatim inside its fn. No remote box is needed.
- **Nothing reads these files by path except two self-contained cases.**
  `rt_owned_argument.rs` reads its own source (`include_str!("rt_owned_argument.rs")`, line 430),
  and its guard plus the seven case tests it scans stay in the root (§4.27).
  `rt_inplace_self_update.rs`'s `scratch_arms_match_the_compiler` includes a compiler source file,
  and its path gains one `../` (§4.38). `grep -rn -- --exact .github scripts` finds no use on any B test.
  `.github/workflows/coverage.yml:180,185` runs `cargo test --release --test rt_inplace_self_update -- --ignored`;
  the target name and the `--ignored` selection are unchanged.
- **No B file is in `scripts/coverage-exceptions.txt` or a `tests/guards` allowlist.**
  `grep -n <name> scripts/coverage-exceptions.txt tests/guards/{architecture_guards,no_type_strings,no_operator_strings,inplace_self_update_census}.rs`
  → only the `//!` doc of `inplace_self_update_census.rs:6`, which names the root path that stays.
- **No new directory collides.** `tests/runtime/<name>/` exists for none of the 38 stems.
  The existing `tests/runtime/inplace_self_update/` (TSV data) is a different name from the new
  `tests/runtime/rt_inplace_self_update/`.

## 3. Design Overview

**Conventions (every file).**

- The root `tests/runtime/<name>.rs` stays the `[[test]]` target. It keeps its module doc,
  `#![cfg(…)]`, `#[path = "../common/mod.rs"] mod common;` (unchanged: the root does not move),
  its `use` lines, and the shared helpers listed under it in §4. It gains one
  `#[path = "<name>/<m>.rs"] mod <m>;` pair per new `.rs` file (2 LOC each).
- Every topic file (one holding `#[test]` fns) starts with `use super::*;`. Through that glob it
  reaches `common`, the root's imports and helpers, and the support globs. Private parent items
  are visible to child modules, so no root item changes visibility. Items a topic owns are used
  only inside it (measured by identifier search over code, including `{IDENT}` format captures),
  so topics need no visibility edits.
- Every support file (data or helper modules with no `#[test]`) gets `pub(super)` on each item,
  struct field and inherent method named under its "Surface" line. The root adds one
  `use <m>::*;` per support file (1 LOC, counted). A support file starts with `use super::*;`
  only when its items reference a parent item. The eight data-only files marked
  "no `use super::*;`" in §4 omit it, because the line would warn.
- Moved code is copied verbatim, including doc comments and attributes (`#[ignore = "…"]`,
  `#[cfg(…)]`), in source order within each file. Line ranges are as of `17a9b5799`. Item and
  test NAMES are authoritative if lines drift.
- An `exempt` row is added by hand to `tests/guards/file_size_ceilings.txt`
  (`exempt test <path> # <reason>`, §4.2 format of plan-176-A). The same path, kind and reason go
  into `tools/loc-census/census.py`'s `EXEMPT` table, in the same commit as the file that needs it.

**Gate class: test-motion.** The proof that nothing was lost is the per-target list of test names,
stripped of module prefixes and sorted, compared before and after, plus a passing run of exactly
the moved targets. Byte-identity does not apply: no compiler code changes. The gate kit used by
every phase follows. `$T` is the phase's target list and `$RE` its path regex, both given in each
phase.

- **K1 list before** (run on the tree before the phase's first edit):
  `for t in $T; do cargo test -q --test $t -- --list 2>/dev/null | grep ': test$' | sed -E 's/^([a-z0-9_]+::)+//' | sort > /tmp/176B-$t.list; done; cat $(for t in $T; do echo /tmp/176B-$t.list; done) | wc -l`
  → the phase's test total.
- **K2 list after**:
  `for t in $T; do cargo test -q --test $t -- --list 2>/dev/null | grep ': test$' | sed -E 's/^([a-z0-9_]+::)+//' | sort | diff -q /tmp/176B-$t.list - >/dev/null || echo "DIFF $t"; done`
  → no output.
- **K3 run**: `cargo test $(for t in $T; do printf -- '--test %s ' $t; done)` → every target
  `test result: ok`, `0 failed`.
- **K4 warnings**: `cargo check --all-targets 2>&1 | grep -E -- "--> tests/runtime/$RE(\.rs|/)"` → no output.
- **K5 budget**: `cargo test --test file_size_budget 2>&1 | grep -E "tests/runtime/$RE(\.rs|/)"` → only
  `TIGHTEN` rows, one per root in the phase, and no `NEW`/`GREW`/`STALE` row. Then
  `MFB_LOC_TIGHTEN=1 cargo test --test file_size_budget`. Then
  `git diff -U0 tests/guards/file_size_ceilings.txt | grep -E '^[-+](ceiling|exempt)' | grep -vE "tests/runtime/$RE(\.rs|/)"`
  → no output (the rewrite touched only this phase's rows). Re-running the first grep → no output.

**Where the risk concentrates.** The main risk is a test that silently stops running. That
happens if a `#[path]` is missing, because a file that is not declared is not compiled. K1/K2
catch it. The next risk is a test that reads its own source or a sibling by path: rt_owned_argument
reads its own source (that code stays in the root), and rt_inplace_self_update includes a compiler
source file (path repointed). A wrong `include_str!` path is a compile error, so it cannot pass
silently. The rt_owned_argument guard can pass silently, which is why §4.27 pins the guard and its
seven scanned tests to the root. `rt_inplace_self_update` (11 new files, 36 `pub(super)` items) is
the largest split and comes last.

**Rejected alternatives.**

- *Promote shared helpers into `tests/common/`*. That changes 338 includers' compile surface
  (letter E owns `tests/common/`) and is not motion.
- *One `[[test]]` target per topic*. That changes target names, breaks `cargo test --test <name>`
  in docs, CI (`coverage.yml`) and scripts, and adds link time per binary.
- *`foo/mod.rs` with the root moved into the directory*. Integration-test roots must stay
  `tests/<dir>/<name>.rs` for `test_targets_registered.rs`
  (`a_registered_target_is_named_after_its_file`) and for the unchanged `[[test]]` stanza.
- *Splitting the exempt units* (§4.28, §4.11). That would be a logic edit of a single test or
  table. The owner and author decided exemption instead (§Decisions).

## 4. Detailed Design

### 4.0 Sweeps common to every file

- Spec citations: `grep -rnoE "\[\[tests/runtime/<name>.rs:[^]]+\]\]" src/docs/spec` → 0 for all
  38 files. `spec_citations_resolve` is not affected because no cited path moves.
- `.ai/`, `scripts/`, `.github/`, `Cargo.toml` references name root paths or target names only.
  Each one is listed under its file's "Sweeps" line, and all stay valid because roots do not move.
- Format of each entry: the layout (new path, LOC, contents, source line ranges as of `17a9b5799`),
  then "Surface" (visibility edits and root globs), then "Sweeps".

#### 4.1 `tests/runtime/rt_foreign_type_reexport.rs` — 270 test LOC, 3 tests

- `tests/runtime/rt_foreign_type_reexport.rs` (104): header; helpers unique_root, mfb, write_project, build_ok, build_expect_failure, install, PB_SRC, PC_SRC [lines 32-37, 39-46, 48-88, 90-108, 110-126, 128-136, 144-145, 146-147]; mods reexport, abi_versions
- `tests/runtime/rt_foreign_type_reexport/reexport.rs` (93): tests (2) foreign_type_reexport_round_trips_through_two_packages, consumer_cannot_name_a_dependencys_private_type; items PA_SRC [lines 138-141, 149-225, 227-255]
- `tests/runtime/rt_foreign_type_reexport/abi_versions.rs` (79): tests (1) abi_incompatible_dependency_versions_are_rejected [lines 257-343]
Surface: none (no support file; root keeps every shared helper).
Sweeps: none (§4.0 commands: 0 hits).

#### 4.2 `tests/runtime/rt_global_argument_reassigned_by_callee.rs` — 263 test LOC, 7 tests

- `tests/runtime/rt_global_argument_reassigned_by_callee.rs` (7): header; mods borrowing, argument_copy
- `tests/runtime/rt_global_argument_reassigned_by_callee/borrowing.rs` (218): tests (6) a_user_callee_that_reassigns_the_global_leaves_the_parameter_intact, a_callee_reached_through_a_func_value_fails_closed, a_string_global_argument_survives_its_reassignment, a_higher_order_builtin_walks_the_value_from_the_call, a_field_of_a_global_record_survives_the_records_reassignment, calls_that_cannot_write_the_global_keep_borrowing; items REPRO, FUNC_VALUE, STRING_GLOBAL, HOF_CALLBACKS, RECORD_FIELD, NO_COPY_CONTROLS, snapshot_slots, run, assert_runs [lines 22-41, 43-64, 66-83, 85-112, 114-137, 139-172, 203-221, 223-229, 231-235, 237-245, 247-254, 256-263, 265-275, 277-284, 286-297]
- `tests/runtime/rt_global_argument_reassigned_by_callee/argument_copy.rs` (44): tests (1) the_argument_copy_is_freed_every_call; items leak_source [lines 174-201, 299-316]
Surface: none (no support file; root keeps every shared helper).
Sweeps: none (§4.0 commands: 0 hits).

#### 4.3 `tests/runtime/rt_imported_package_global.rs` — 291 test LOC, 10 tests

- `tests/runtime/rt_imported_package_global.rs` (50): header; helpers importer_project [lines 80-126]; mods access, refusals
- `tests/runtime/rt_imported_package_global/access.rs` (141): tests (5) an_exported_package_constant_is_readable, a_package_global_is_initialized_before_main, an_exported_package_mut_is_one_shared_slot, an_aliased_import_reaches_the_same_globals, a_qualified_comparison_in_expression_position_is_still_a_comparison; items run_importer, build [lines 37-57, 128-145, 147-170, 172-195, 197-221, 223-246, 350-374]
- `tests/runtime/rt_imported_package_global/refusals.rs` (106): tests (5) a_write_to_an_exported_package_constant_is_refused, a_write_of_the_wrong_type_to_a_package_mut_is_refused, a_package_constant_read_is_type_checked, a_private_package_global_is_not_visible, an_unexported_global_name_still_names_its_package; items build_error [lines 59-78, 248-272, 274-291, 293-309, 311-329, 331-348]
Surface: none (no support file; root keeps every shared helper).
Sweeps: none (§4.0 commands: 0 hits).

#### 4.4 `tests/runtime/rt_imported_type_name_collision.rs` — 304 test LOC, 10 tests

- `tests/runtime/rt_imported_type_name_collision.rs` (147): header; helpers unique_root, mfb, write_project, build, build_fails, install, Package, run_app, OV_SRC, LOCAL_A [lines 18-23, 25-29, 31-69, 71-87, 89-106, 108-114, 116-117, 119-165, 168-170, 173-173]; mods local_collisions, package_collisions
- `tests/runtime/rt_imported_type_name_collision/local_collisions.rs` (68): tests (5) consumer_type_and_package_type_share_a_name, consumer_reads_an_imported_types_field_beside_a_local_type_of_the_same_name, overloaded_imported_call_with_imported_field_beside_a_local_type_of_the_same_name, a_bare_imported_user_package_type_is_refused, an_imported_value_is_refused_by_a_local_type_of_the_same_name [lines 175-185, 206-218, 220-237, 239-255, 322-339]
- `tests/runtime/rt_imported_type_name_collision/package_collisions.rs` (95): tests (5) two_packages_export_types_with_the_same_name, an_aliased_import_names_the_same_type, imported_union_variants_and_enum_members_keep_their_package, one_packages_type_is_refused_where_another_packages_same_named_type_is_expected, diamond_import_shares_one_copy_of_a_type [lines 187-204, 257-270, 272-293, 295-320, 341-367]
Surface: none (no support file; root keeps every shared helper).
Sweeps: none (§4.0 commands: 0 hits).

#### 4.5 `tests/runtime/rt_imported_union_enum_members.rs` — 315 test LOC, 9 tests

- `tests/runtime/rt_imported_union_enum_members.rs` (122): header; helpers run_importer, build_error, importer_project, build [lines 43-63, 65-85, 87-147, 149-166]; mods unions, enums
- `tests/runtime/rt_imported_union_enum_members/unions.rs` (95): tests (4) a_qualified_imported_union_matches_exhaustively, a_bare_imported_union_is_refused, an_imported_union_match_missing_a_variant_is_still_refused, a_case_on_a_type_the_union_does_not_include_is_still_refused [lines 168-196, 198-223, 303-330, 376-400]
- `tests/runtime/rt_imported_union_enum_members/enums.rs` (104): tests (5) a_qualified_imported_enum_matches_exhaustively, a_qualified_imported_enum_member_is_a_value, a_bare_imported_enum_is_refused, an_imported_enum_match_missing_a_member_is_still_refused, a_qualified_enum_name_that_is_not_a_member_is_still_refused [lines 225-249, 251-276, 278-301, 332-352, 354-374]
Surface: none (no support file; root keeps every shared helper).
Sweeps: none (§4.0 commands: 0 hits).

#### 4.6 `tests/runtime/rt_package_dependency_closure.rs` — 295 test LOC, 6 tests

- `tests/runtime/rt_package_dependency_closure.rs` (66): header; helpers unique_root, mfb, BASE_SRC, USER_SRC, APP_SRC, write_project, write_packages, run, build [lines 23-28, 30-36, 38-38, 41-42, 43-44, 46-64, 66-78, 80-88, 90-92]; mods refusals, declared_closures
- `tests/runtime/rt_package_dependency_closure/refusals.rs` (111): tests (3) undeclared_transitive_dependency_is_refused_before_lowering, stale_dependency_fields_are_listed_and_refused, incompatible_dependency_version_is_refused_and_verify_explains_it; items BASE_V2_SRC [lines 39-40, 111-145, 268-313, 315-353]
- `tests/runtime/rt_package_dependency_closure/declared_closures.rs` (124): tests (3) pkg_update_declares_the_closure_and_the_app_runs, pkg_add_of_a_compiled_package_declares_its_sibling_dependency, a_declared_closure_builds_and_runs; items built_executable, assert_runs_and_prints_5 [lines 94-101, 103-109, 147-194, 196-245, 247-266]
Surface: none (no support file; root keeps every shared helper).
Sweeps: none (§4.0 commands: 0 hits).

#### 4.7 `tests/runtime/rt_overloaded_function_values.rs` — 454 test LOC, 27 tests

- `tests/runtime/rt_overloaded_function_values.rs` (102): header; helpers unique_root, mfb, write_project, build, run_program, run_built, assert_runs, PICK_INT, PICK_STRING, PICK_RETURN, program [lines 21-26, 28-32, 34-63, 65-77, 79-83, 85-106, 108-112, 135-135, 136-136, 137-137, 139-141]; mods rejection, selection, imports_and_generics
- `tests/runtime/rt_overloaded_function_values/rejection.rs` (76): tests (6) untyped_reference_is_ambiguous, reference_without_a_callable_context_is_ambiguous, no_matching_parameter_types_is_rejected, no_matching_return_type_is_rejected, isolation_mismatch_is_rejected, non_callable_expected_type_is_rejected; items assert_rejects [lines 114-133, 317-325, 327-335, 337-345, 347-355, 357-368, 370-378]
- `tests/runtime/rt_overloaded_function_values/selection.rs` (153): tests (12) annotated_local_selects_parameter_type_overload, annotated_local_selects_return_type_overload, annotated_locals_select_each_return_type_member, single_declaration_control, direct_call_and_lambda_controls, annotated_global_selects_overload, assignment_selects_overload, callback_argument_selects_overload, return_operand_selects_overload, typed_aggregates_select_overload, isolated_annotation_selects_isolated_overload, local_shadowing_the_name_is_not_rewritten [lines 145-155, 157-167, 169-181, 185-195, 197-209, 213-223, 225-236, 238-251, 253-265, 267-286, 288-299, 301-313]
- `tests/runtime/rt_overloaded_function_values/imports_and_generics.rs` (132): tests (9) imported_overload_selected_by_annotation, imported_overload_selected_through_an_alias, imported_overload_without_a_match_is_rejected, imported_overload_untyped_is_ambiguous, generic_body_selects_per_instantiation, call_through_shadowing_local_runs_the_local, call_through_shadowing_parameter_runs_the_parameter, generic_body_selects_return_overload_returning_a_callable, builtin_named_override_selected_by_annotation; items OV_SRC, build_package, consumer [lines 382-384, 386-390, 392-401, 403-415, 417-427, 429-442, 444-453, 455-470, 479-489, 491-503, 505-522, 524-540]
Surface: none (no support file; root keeps every shared helper).
Sweeps: none (§4.0 commands: 0 hits).

#### 4.8 `tests/runtime/rt_nothing_collections.rs` — 279 test LOC, 10 tests

- `tests/runtime/rt_nothing_collections.rs` (73): header; helpers build_and_run, HELPERS, program [lines 23-70, 72-86, 88-90]; mods lists, maps_and_sets
- `tests/runtime/rt_nothing_collections/lists.rs` (110): tests (5) a_list_literal_of_nothing_builds_and_iterates, list_of_nothing_builders_and_slices, list_of_nothing_searches_and_predicates, list_of_nothing_callbacks_and_regrouping, a_mut_list_of_nothing_updates_in_place [lines 92-104, 106-134, 136-163, 165-188, 190-204]
- `tests/runtime/rt_nothing_collections/maps_and_sets.rs` (102): tests (5) a_map_with_nothing_values, a_map_keyed_by_nothing_has_one_entry, a_set_of_nothing_holds_at_most_one_element, nothing_func_calls_fill_collection_literals, collections_of_nothing_nest_and_copy [lines 206-229, 231-249, 251-272, 274-291, 293-317]
Surface: none (no support file; root keeps every shared helper).
Sweeps: none (§4.0 commands: 0 hits).

#### 4.9 `tests/runtime/rt_moved_handle_guards.rs` — 307 test LOC, 2 tests

- `tests/runtime/rt_moved_handle_guards.rs` (8): header; helpers MOVED [lines 22-23]; mods file_socket_thread, tls_listener
- `tests/runtime/rt_moved_handle_guards/file_socket_thread.rs` (203): tests (1) file_socket_and_thread_operations_report_a_moved_handle [lines 25-226]
- `tests/runtime/rt_moved_handle_guards/tls_listener.rs` (102): tests (1) tls_listener_operations_report_a_moved_handle [lines 228-332]
Surface: none (no support file; root keeps every shared helper).
Sweeps: none (§4.0 commands: 0 hits).

#### 4.10 `tests/runtime/rt_callback_failure_frees.rs` — 285 test LOC, 1 tests

- `tests/runtime/rt_callback_failure_frees.rs` (119): header; helpers REPS, program, run; tests (1) a_failing_callback_frees_everything_in_flight [lines 30-30, 211-243, 245-295, 297-325]; mods cases
- `tests/runtime/rt_callback_failure_frees/cases.rs` (support) (169; no `use super::*;`, see §3): items HELPERS, CASES, trapped [lines 32-127, 129-200, 202-209]
Surface: pub(super) on 3 item(s): HELPERS, CASES, trapped; root adds `use cases::*;`.
Sweeps: none (§4.0 commands: 0 hits).

#### 4.11 `tests/runtime/rt_closure_single_owner.rs` — 574 test LOC, 1 tests

- `tests/runtime/rt_closure_single_owner.rs` (80): header; helpers run; tests (1) every_holder_of_a_capturing_closure_frees_it_exactly_once [lines 522-573, 575-597]; mods cases
- `tests/runtime/rt_closure_single_owner/cases.rs` (support) (497; no `use super::*;`, see §3): items CASES [lines 23-520]
Surface: pub(super) on 1 item(s): CASES; root adds `use cases::*;`.
Sweeps: none (§4.0 commands: 0 hits).
Exempt (§Decisions; row added in Phase 2): `tests/runtime/rt_closure_single_owner/cases.rs` (497) — the single `const CASES: &[(&str, &str, &str)]` table (lines 23-520) read by the one `#[test]`; splitting it needs several per-holder consts chained in the test (a logic edit). The root drops to 80.

#### 4.12 `tests/runtime/rt_domain_error_frees_locals.rs` — 306 test LOC, 1 tests

- `tests/runtime/rt_domain_error_frees_locals.rs` (152): header; helpers CASES, arena_sum; tests (1) an_inline_domain_error_frees_the_scope_it_leaves [lines 180-257, 259-267, 269-330]; mods program
- `tests/runtime/rt_domain_error_frees_locals/program.rs` (support) (157; no `use super::*;`, see §3): items PROGRAM [lines 22-178]
Surface: pub(super) on 1 item(s): PROGRAM; root adds `use program::*;`.
Sweeps: none (§4.0 commands: 0 hits).

#### 4.13 `tests/runtime/rt_error_model_recovery.rs` — 663 test LOC, 4 tests

- `tests/runtime/rt_error_model_recovery.rs` (94): header; helpers HELPERS, line_of, project_root, build_output, build, run [lines 31-53, 55-63, 296-307, 309-319, 321-340, 342-353]; mods routes, routing, error_values, memory
- `tests/runtime/rt_error_model_recovery/routes.rs` (support) (100; no `use super::*;`, see §3): items Row, ROWS [lines 65-76, 78-179]
- `tests/runtime/rt_error_model_recovery/routing.rs` (123): tests (1) every_handler_route_reaches_the_documented_handler; items routing_source, routing_expected [lines 181-241, 243-294, 355-373]
- `tests/runtime/rt_error_model_recovery/error_values.rs` (134): tests (2) an_error_returned_from_an_error_function_is_successful_data, returning_an_error_from_a_non_error_function_is_a_type_mismatch [lines 375-465, 467-516]
- `tests/runtime/rt_error_model_recovery/memory.rs` (224): tests (1) nested_recovery_and_propagate_balance_the_arena; items Payload, PAYLOADS, memory_source, Value, impl Value, memory_expected [lines 518-530, 532-560, 562-657, 659-665, 667-675, 677-729, 731-760]
Surface: pub(super) on 1 item(s): ROWS; plus struct Row fields name, body, escapes, recovered; root adds `use routes::*;`.
Sweeps: none (§4.0 commands: 0 hits).

#### 4.14 `tests/runtime/rt_inline_trap_expression_boundary.rs` — 406 test LOC, 9 tests

- `tests/runtime/rt_inline_trap_expression_boundary.rs` (88): header; helpers HEAD, build_output, combined, build, run, expect_main [lines 25-43, 45-52, 54-60, 62-78, 80-90, 92-109]; mods raising_operators, handler_shape, allocation
- `tests/runtime/rt_inline_trap_expression_boundary/raising_operators.rs` (132): tests (4) a_chained_division_failure_anywhere_recovers_the_whole_result, checked_integer_byte_fixed_and_money_operations_are_captured, float_observation_timing_is_unchanged_by_the_trap, comparisons_raise_only_when_they_promote_into_fixed [lines 111-144, 146-181, 183-211, 213-255]
- `tests/runtime/rt_inline_trap_expression_boundary/handler_shape.rs` (106): tests (4) a_known_total_expression_carries_a_dead_handler, every_position_takes_the_whole_value, an_operator_only_trap_is_not_a_function_handler_failure_edge, recover_typing_fallthrough_and_testing_assertions_are_unchanged [lines 257-289, 291-324, 326-336, 338-375]
- `tests/runtime/rt_inline_trap_expression_boundary/allocation.rs` (89): tests (1) operator_rooted_traps_allocate_what_the_plain_expression_does [lines 377-469]
Surface: none (no support file; root keeps every shared helper).
Sweeps: none (§4.0 commands: 0 hits).

#### 4.15 `tests/runtime/rt_inline_trap_nested_call.rs` — 503 test LOC, 20 tests

- `tests/runtime/rt_inline_trap_nested_call.rs` (94): header; helpers unique_root, mfb, PRELUDE, build_output, build, run [lines 29-34, 36-41, 43-68, 70-87, 89-109, 111-123]; mods nesting, roots, handler_forms, builtins_and_conversions
- `tests/runtime/rt_inline_trap_nested_call/nesting.rs` (143): tests (7) nested_fallible_call_reaches_the_inline_handler, outermost_fallible_call_still_reaches_the_handler, deeply_nested_fallible_call_reaches_the_handler, recover_skips_the_rest_of_the_trapped_expression, hoisting_preserves_left_to_right_argument_order, first_failing_argument_short_circuits_the_second, nested_success_path_delivers_the_full_expression_value [lines 125-148, 150-170, 172-192, 194-217, 219-242, 244-263, 265-284]
- `tests/runtime/rt_inline_trap_nested_call/roots.rs` (106): tests (5) nested_fallible_call_under_an_infallible_builtin_root, non_call_root_with_a_nested_fallible_call_is_trapped, a_trapped_expression_with_no_call_delivers_its_value, a_fallible_call_in_a_short_circuited_operand_is_caught_only_when_selected, the_handler_runs_once_per_failure [lines 286-306, 308-329, 331-351, 353-388, 390-410]
- `tests/runtime/rt_inline_trap_nested_call/handler_forms.rs` (59): tests (3) a_diverging_handler_covers_a_nested_failure, the_discard_form_covers_a_nested_failure, the_assign_form_covers_a_nested_failure [lines 412-431, 433-453, 455-475]
- `tests/runtime/rt_inline_trap_nested_call/builtins_and_conversions.rs` (113): tests (5) a_nested_fallible_builtin_reaches_the_handler, a_fallible_user_override_of_an_infallible_builtin_is_still_covered, a_nested_conversion_whose_handler_reads_the_error, a_nested_conversion_whose_handler_ignores_the_error, a_decode_wrapping_a_fallible_read_reaches_the_handler [lines 477-497, 499-532, 534-566, 568-589, 591-632]
Surface: none (no support file; root keeps every shared helper).
Sweeps: none (§4.0 commands: 0 hits).

#### 4.16 `tests/runtime/rt_inline_trap_raising_operator.rs` — 556 test LOC, 21 tests

- `tests/runtime/rt_inline_trap_raising_operator.rs` (95): header; helpers unique_root, mfb, PRELUDE, build_output, build, run [lines 33-38, 40-45, 47-77, 79-96, 98-118, 120-132]; mods operators, evaluation_order, lifting
- `tests/runtime/rt_inline_trap_raising_operator/operators.rs` (137): tests (7) nested_division_by_zero_reaches_the_inline_handler, a_call_after_the_last_fallible_call_under_a_raising_operator_builds_and_runs_in_order, nested_integer_overflow_reaches_the_inline_handler, nested_mod_by_zero_reaches_the_inline_handler, nested_float_overflow_reaches_the_inline_handler, nested_unary_negation_overflow_reaches_the_inline_handler, raising_operator_inside_a_fallible_call_reaches_the_handler [lines 134-154, 156-183, 185-205, 207-226, 228-248, 250-269, 271-291]
- `tests/runtime/rt_inline_trap_raising_operator/evaluation_order.rs` (142): tests (7) recover_skips_the_rest_of_the_trapped_expression, lifting_preserves_left_to_right_argument_order, a_raising_operator_that_succeeds_still_delivers_its_value, the_first_failure_in_evaluation_order_wins, a_preceding_call_failure_still_wins, an_assign_target_recovers_from_a_raising_operator, a_handler_that_ignores_the_error_still_recovers [lines 293-317, 319-339, 341-361, 363-387, 389-408, 410-430, 432-451]
- `tests/runtime/rt_inline_trap_raising_operator/lifting.rs` (191): tests (7) a_callee_that_raises_only_through_an_operator_is_still_checked, bug457s_lift_also_keeps_an_operator_raising_root_checked, a_negative_literal_is_not_lifted_but_a_computed_negation_is, a_short_circuited_raising_operator_is_caught_only_when_selected, a_bare_operator_scrutinee_is_trapped, bug457s_nested_call_shape_still_works, a_raising_operator_outside_a_trap_still_propagates; items build_ir [lines 453-500, 502-526, 528-552, 554-594, 596-624, 626-648, 650-669, 671-693]
Surface: none (no support file; root keeps every shared helper).
Sweeps: none (§4.0 commands: 0 hits).

#### 4.17 `tests/runtime/rt_inline_trap_short_circuit.rs` — 477 test LOC, 11 tests

- `tests/runtime/rt_inline_trap_short_circuit.rs` (102): header; helpers HEAD, project, build_output, build, run, expect_main [lines 23-64, 66-68, 70-77, 79-97, 99-109, 111-122]; mods selection, targets, allocation
- `tests/runtime/rt_inline_trap_short_circuit/selection.rs` (188): tests (6) a_selected_right_call_is_captured_and_an_unselected_one_never_runs, nested_conditions_and_failures_skip_exactly_the_rest, a_conditional_inside_a_collection_or_nested_call_is_captured, an_infallible_side_effect_stays_ordered_before_a_later_failure, raising_operators_and_comparisons_in_a_selected_operand_are_captured, a_float_comparison_in_a_selected_operand_keeps_the_observation_model [lines 124-162, 164-197, 199-223, 225-244, 246-292, 294-329]
- `tests/runtime/rt_inline_trap_short_circuit/targets.rs` (53): tests (3) conditional_results_reach_every_target, a_conditional_result_reaches_a_state_target, a_trapped_conditional_is_not_a_function_handler_failure_edge [lines 331-363, 365-379, 381-392]
- `tests/runtime/rt_inline_trap_short_circuit/allocation.rs` (143): tests (2) right_operand_payloads_balance_the_arena_on_every_path, the_trapped_conditional_success_path_allocates_what_the_plain_one_does; items PAYLOAD_HEAD, PAYLOAD_RIGHTS [lines 394-440, 442-447, 449-486, 488-550]
Surface: none (no support file; root keeps every shared helper).
Sweeps: none (§4.0 commands: 0 hits).

#### 4.18 `tests/runtime/rt_inline_trap_success_alloc.rs` — 437 test LOC, 2 tests

- `tests/runtime/rt_inline_trap_success_alloc.rs` (123): header; helpers HEAD, inputs, build, run [lines 30-83, 206-228, 358-382, 384-398]; mods shapes, success_path, failing_shapes
- `tests/runtime/rt_inline_trap_success_alloc/shapes.rs` (support) (110; no `use super::*;`, see §3): items Shape, SHAPES [lines 85-97, 99-204]
- `tests/runtime/rt_inline_trap_success_alloc/success_path.rs` (71): tests (1) the_trapped_success_path_allocates_what_the_plain_expression_does; items success_source, alloc_delta [lines 230-270, 400-411, 413-437]
- `tests/runtime/rt_inline_trap_success_alloc/failing_shapes.rs` (142): tests (1) every_failing_shape_delivers_the_untrapped_error; items error_source [lines 272-356, 439-504]
Surface: pub(super) on 2 item(s): Shape, SHAPES; plus struct Shape fields name, type_, expr, use_, fail, failing_expr, recover; root adds `use shapes::*;`.
Sweeps: `.ai/codegen-invariants.md:537` names the target — unchanged.

#### 4.19 `tests/runtime/rt_inline_trap_union_bind.rs` — 251 test LOC, 6 tests

- `tests/runtime/rt_inline_trap_union_bind.rs` (58): header; helpers unique_root, build, run [lines 38-43, 45-78, 80-91]; mods data_unions, resource_unions
- `tests/runtime/rt_inline_trap_union_bind/data_unions.rs` (117): tests (4) a_plain_union_bind_from_a_variant_producer_matches_its_variant, a_trap_bound_data_union_matches_its_variant, a_trap_bound_data_union_still_reaches_its_handler, a_recovered_variant_in_a_union_returning_trap_matches_its_variant; items DATA_PRELUDE [lines 93-104, 106-131, 133-167, 169-194, 236-272]
- `tests/runtime/rt_inline_trap_union_bind/resource_unions.rs` (82): tests (2) a_trap_bound_resource_union_matches_its_variant, a_recovered_variant_in_a_resource_union_returning_trap_is_matched_and_closed [lines 196-234, 274-326]
Surface: none (no support file; root keeps every shared helper).
Sweeps: none (§4.0 commands: 0 hits).

#### 4.20 `tests/runtime/rt_hof_transient_bytes.rs` — 293 test LOC, 2 tests

- `tests/runtime/rt_hof_transient_bytes.rs` (8): header; mods partition_group_by, failing_callback
- `tests/runtime/rt_hof_transient_bytes/partition_group_by.rs` (204): tests (1) partition_and_group_by_allocate_about_their_result; items HELPERS, CASES, N, program, control, run, expected [lines 25-69, 71-107, 109-109, 111-127, 129-146, 148-188, 190-200, 202-243]
- `tests/runtime/rt_hof_transient_bytes/failing_callback.rs` (87): tests (1) a_failing_value_callback_frees_the_key_projection [lines 245-334]
Surface: none (no support file; root keeps every shared helper).
Sweeps: `.ai/collections.md:577` names the target (`grep -rn rt_hof_transient_bytes .ai`) — unchanged.

#### 4.21 `tests/runtime/rt_inplace_failure_atomic.rs` — 494 test LOC, 3 tests

- `tests/runtime/rt_inplace_failure_atomic.rs` (89): header; helpers HELPERS [lines 19-97]; mods binding, field, reallocating_field
- `tests/runtime/rt_inplace_failure_atomic/binding.rs` (138): tests (1) a_failing_self_update_leaves_the_binding_unchanged; items CASES, program [lines 99-192, 194-215, 217-238]
- `tests/runtime/rt_inplace_failure_atomic/field.rs` (133): tests (1) a_failing_field_self_update_leaves_the_owner_unchanged [lines 240-381]
- `tests/runtime/rt_inplace_failure_atomic/reallocating_field.rs` (143): tests (1) a_failing_reallocating_field_update_leaves_the_owner_unchanged [lines 383-531]
Surface: none (no support file; root keeps every shared helper).
Sweeps: `.ai/collections.md:97` names the root path — unchanged.

#### 4.22 `tests/runtime/rt_json_bounds.rs` — 308 test LOC, 4 tests

- `tests/runtime/rt_json_bounds.rs` (21): header; helpers run [lines 22-33]; mods grammar, cost, semantics
- `tests/runtime/rt_json_bounds/grammar.rs` (86): tests (2) crlf_formatted_json_parses, number_grammar_verdicts_are_unchanged_over_bytes [lines 35-67, 69-129]
- `tests/runtime/rt_json_bounds/cost.rs` (45): tests (1) a_large_string_costs_a_bounded_multiple_of_its_size; items LARGE_STRING [lines 131-152, 154-182]
- `tests/runtime/rt_json_bounds/semantics.rs` (165): tests (1) parsing_semantics_are_unchanged; items PARSE_CORPUS, PARSE_CORPUS_EXPECTED [lines 184-262, 264-327, 329-351]
Surface: none (no support file; root keeps every shared helper).
Sweeps: Cargo.toml:68 libc dependency comment names the path — unchanged.

#### 4.23 `tests/runtime/rt_list_append_growth_bounds.rs` — 284 test LOC, 8 tests

- `tests/runtime/rt_list_append_growth_bounds.rs` (165): header; helpers HEADER, GROW_INIT, GROW_TAPER, ALLOC_GRANULE, step, generations, round_up, grown_bytes, build_debug, alloc_bytes, Arm, impl Arm, list_program [lines 34-35, 40-41, 42-42, 43-44, 46-55, 57-73, 75-77, 79-85, 87-113, 115-135, 137-145, 147-184, 186-220]; mods map_and_peak, list_arms
- `tests/runtime/rt_list_append_growth_bounds/map_and_peak.rs` (64): tests (2) map_set_reserves_data_by_entry_width, a_16_mib_append_built_byte_list_peaks_near_its_data; items MAP_ENTRY, MAP_BUCKET [lines 36-37, 38-39, 297-319, 321-364]
- `tests/runtime/rt_list_append_growth_bounds/list_arms.rs` (61): tests (6) append_reserves_data_by_element_width, insert_at_the_end_reserves_data_by_element_width, prepend_reserves_data_by_element_width, bulk_append_reserves_data_by_element_width, record_field_append_reserves_data_by_element_width, record_field_bulk_append_reserves_data_by_element_width; items measure_list, BOX_PREFIX, assert_list_arm [lines 222-231, 233-235, 237-264, 266-269, 271-274, 276-280, 282-285, 287-290, 292-295]
Surface: none (no support file; root keeps every shared helper).
Sweeps: `.ai/collections.md:12`, `.ai/testing-gates.md:871` name the root path — unchanged.

#### 4.24 `tests/runtime/rt_list_element_record_borrow.rs` — 645 test LOC, 21 tests

- `tests/runtime/rt_list_element_record_borrow.rs` (122): header; helpers run, TYPES, GLOBAL, GLOBAL_EXIT, AGES, assert_no_leak [lines 24-74, 76-125, 127-129, 130-130, 132-133, 135-144]; mods bounded_reads, copies, element_updates, union_elements
- `tests/runtime/rt_list_element_record_borrow/bounded_reads.rs` (171): tests (10) a_direct_field_read_is_bounded, a_direct_field_read_of_a_scalar_record_is_bounded, a_binding_read_only_through_fields_of_an_immutable_list_is_bounded, a_binding_read_only_through_fields_of_a_mut_list_is_bounded, a_read_of_a_module_level_list_is_bounded, a_read_of_a_map_value_is_bounded, a_get_then_set_back_is_bounded, a_get_with_set_element_update_is_bounded, a_get_with_set_element_update_of_a_module_level_list_is_bounded, a_single_expression_element_update_is_bounded; items bounded [lines 146-178, 180-192, 194-207, 209-222, 224-238, 240-254, 256-271, 273-286, 288-312, 314-331, 333-345]
- `tests/runtime/rt_list_element_record_borrow/copies.rs` (120): tests (5) a_returned_element_is_a_copy, an_element_stored_into_another_list_is_a_copy, a_binding_that_outlives_a_container_write_is_a_copy, a_binding_across_a_call_that_writes_the_module_level_list_is_a_copy, a_binding_whose_index_call_writes_the_module_level_list_is_a_copy [lines 347-370, 372-393, 395-422, 424-450, 452-479]
- `tests/runtime/rt_list_element_record_borrow/element_updates.rs` (153): tests (4) a_match_over_a_get_or_miss_reads_the_default, an_element_read_around_its_write_back_sees_each_value, an_element_update_that_grows_a_field_keeps_every_element, a_failing_element_update_leaves_the_list_unchanged [lines 481-528, 530-555, 557-593, 595-646]
- `tests/runtime/rt_list_element_record_borrow/union_elements.rs` (91): tests (2) a_binding_read_only_through_a_match_of_a_union_element_is_bounded, a_match_extract_that_escapes_is_a_copy; items EXPR [lines 648-682, 684-712, 714-746]
Surface: none (no support file; root keeps every shared helper).
Sweeps: `.ai/collections.md:588` names the target — unchanged.

#### 4.25 `tests/runtime/rt_list_take_drop_in_place.rs` — 502 test LOC, 5 tests

- `tests/runtime/rt_list_take_drop_in_place.rs` (17): header; mods scaling, scale_program, value_program, value_semantics
- `tests/runtime/rt_list_take_drop_in_place/scaling.rs` (93): tests (3) a_take_all_self_update_over_a_variable_width_list_is_linear, a_drop_none_self_update_over_a_variable_width_list_is_linear, the_already_linear_shapes_stay_linear; items ROUNDS, MAX_RATIO, SLACK_SECONDS, scale_exe, cpu_run, assert_linear [lines 41-41, 42-42, 43-43, 391-397, 399-446, 448-479, 481-486, 488-492, 494-502]
- `tests/runtime/rt_list_take_drop_in_place/scale_program.rs` (support) (145; no `use super::*;`, see §3): items SCALE_PROGRAM [lines 45-192]
- `tests/runtime/rt_list_take_drop_in_place/value_program.rs` (support) (192; no `use super::*;`, see §3): items VALUE_PROGRAM, VALUE_OUTPUT [lines 194-357, 359-389]
- `tests/runtime/rt_list_take_drop_in_place/value_semantics.rs` (67): tests (2) take_drop_mid_keep_value_semantics_and_the_right_elements, take_drop_mid_free_every_removed_element_once [lines 504-518, 520-575]
Surface: pub(super) on 3 item(s): SCALE_PROGRAM, VALUE_PROGRAM, VALUE_OUTPUT; root adds `use value_program::*;`, `use scale_program::*;`.
Sweeps: none (§4.0 commands: 0 hits).

#### 4.26 `tests/runtime/rt_map_integer_key_probe_cost.rs` — 355 test LOC, 4 tests

- `tests/runtime/rt_map_integer_key_probe_cost.rs` (43): header; helpers run_program [lines 33-67]; mods cost, hash_sites
- `tests/runtime/rt_map_integer_key_probe_cost/cost.rs` (175): tests (3) an_integer_key_lookup_costs_within_2_5x_a_list_get, building_an_integer_key_map_costs_within_9x_list_appends, a_list_value_read_modify_write_allocates_twice_per_iteration; items SIZES, timing_program, timings [lines 31-31, 69-130, 132-148, 150-183, 185-207, 209-264]
- `tests/runtime/rt_map_integer_key_probe_cost/hash_sites.rs` (143): tests (1) every_hash_site_agrees_across_grows_and_removals [lines 266-411]
Surface: none (no support file; root keeps every shared helper).
Sweeps: none (§4.0 commands: 0 hits).

#### 4.27 `tests/runtime/rt_owned_argument.rs` — 643 test LOC, 13 tests

- `tests/runtime/rt_owned_argument.rs` (114): header; tests (8) local_return, helper_append, helper_map_set, helper_concat, record_field, bind_param, recursive_fill, the_ignored_set_is_exactly_the_five_open_cases [lines 365-372, 374-379, 381-385, 387-394, 396-402, 404-409, 411-419, 421-511]; mods cases, support, failure_route, hand_over
- `tests/runtime/rt_owned_argument/cases.rs` (support) (153; no `use super::*;`, see §3): items Case, Expect, DEEP_N, FLAT_N, cases, case, source [lines 51-65, 67-78, 80-82, 83-84, 86-225, 227-232, 234-240]
- `tests/runtime/rt_owned_argument/support.rs` (support) (110): items build_debug, measure, check [lines 242-269, 271-308, 310-363]
- `tests/runtime/rt_owned_argument/failure_route.rs` (166): tests (2) an_owned_parameter_is_freed_once_on_the_failure_route, a_handed_over_temporary_is_freed_once_on_the_failure_route [lines 513-629, 664-760]
- `tests/runtime/rt_owned_argument/hand_over.rs` (113): tests (3) the_hand_over_really_happens, a_handed_over_temporary_is_claimed_past_a_later_temporary, a_string_literal_argument_is_never_handed_over [lines 631-662, 762-837, 839-884]
Surface: pub(super) on 7 item(s): Case, Expect, cases, case, source, build_debug, check; plus struct Case fields name, letter, n, want, expect; root adds `use cases::*;`, `use support::*;`.
Sweeps: `the_ignored_set_is_exactly_the_five_open_cases` reads ITS OWN SOURCE (`include_str!("rt_owned_argument.rs")`, line 430) and scans it for `#[ignore = "plan-147-…"]` on the case tests → the guard AND the seven case tests (local_return, helper_append, helper_map_set, helper_concat, record_field, bind_param, recursive_fill) MUST stay in the root, or the guard silently stops seeing ignores (it would still pass today because `want` is empty, which is exactly the silent-loss mode it exists to catch). It also calls `cases()` (now in cases.rs, reached via the root glob). Module doc `cargo test --test rt_owned_argument -- --ignored` and the intra-doc link to the guard stay valid (both in root).

#### 4.28 `tests/runtime/rt_big_int.rs` — 2584 test LOC, 17 tests

- `tests/runtime/rt_big_int.rs` (51): header; helpers run, SHOW_BYTES [lines 16-31, 33-47]; mods representation, ordering_and_sign, addition, multiply_and_bits, text, division, powers, random, word_boundaries
- `tests/runtime/rt_big_int/representation.rs` (118): tests (3) a_defaulted_int_is_canonical_zero, integer_round_trip_and_overflow_edges, byte_round_trips_in_both_orders [lines 49-67, 69-118, 120-178]
- `tests/runtime/rt_big_int/ordering_and_sign.rs` (100): tests (2) compare_is_a_total_order_and_the_predicates_agree, negate_abs_and_the_total_decoder [lines 180-239, 241-285]
- `tests/runtime/rt_big_int/addition.rs` (176): tests (3) additive_carry_and_borrow_chains, additive_identities_and_integer_oracle, a_thousand_term_sum_prints_the_independent_total [lines 287-340, 342-423, 730-776]
- `tests/runtime/rt_big_int/multiply_and_bits.rs` (194): tests (2) multiply_matches_an_independent_oracle, bit_operations [lines 425-510, 512-625]
- `tests/runtime/rt_big_int/text.rs` (100): tests (1) text_matches_an_independent_oracle [lines 627-728]
- `tests/runtime/rt_big_int/division.rs` (223): tests (3) division_targeted_cases, division_property_over_ten_thousand_seeded_pairs, division_members_agree_and_raise [lines 778-829, 831-935, 937-1018]
- `tests/runtime/rt_big_int/powers.rs` (1272): tests (1) powers_gcd_factorial_and_mod_pow_match_an_independent_oracle [lines 1020-2294]
- `tests/runtime/rt_big_int/random.rs` (88): tests (1) crypto_random_int_big_overload_is_in_range_and_uniform [lines 2296-2387]
- `tests/runtime/rt_big_int/word_boundaries.rs` (141): tests (1) word_boundaries_match_an_independent_oracle [lines 2389-2686], with the program literal replaced by `include_str!("word_boundaries.mfb")` (288 − 149 literal lines + 1 + `use super::*;`; `python3 -I /tmp/loc/span.py tests/runtime/rt_big_int.rs 2389 2686` → 288, `… 2403 2551` → 149)
- `tests/runtime/rt_big_int/word_boundaries.mfb` (143 MFBASIC LOC; a test file because it is under `tests/`): the bytes of the `r#"…"#` program literal passed as `run`'s second argument, from `IMPORT io` (line 2403, right after `r#"`) through line 2550's `END FUNC\n` (the literal closes with `"#,` at the start of line 2551), verbatim. Measured: the extracted text has 143 non-blank lines, none a `'`/`REM` comment and no `DOC` block. No harness compiles stray `.mfb` files under `tests/runtime/` (`grep -rln 'tests/runtime' scripts tests/guards tests/common | xargs grep -ln '\.mfb'` → no file).
Surface: none (no support file; root keeps every shared helper).
Sweeps: none (§4.0 commands: 0 hits).
Exempt (§Decisions; row added in Phase 4): `tests/runtime/rt_big_int/powers.rs` (1272) — one `#[test]` whose body is a single inline MFBASIC program passed to `run("big_c_powers", r#"…"#)` and the expected output it is asserted against. Moving that program into an `.mfb` under `tests/` leaves a test-kind file of the same size, so no extraction shrinks the unit. `word_boundaries.rs` is NOT exempt: its natural extraction (program → `word_boundaries.mfb`) brings both files under 250 (141 and 143). Every other file of this split is ≤ 223.

#### 4.29 `tests/runtime/rt_math_fixed_trig_accuracy.rs` — 617 test LOC, 3 tests

- `tests/runtime/rt_math_fixed_trig_accuracy.rs` (52): header; helpers W, parse_fixed, error_units, units [lines 27-28, 199-221, 223-231, 422-428]; mods forward, oracle, inputs, inverse
- `tests/runtime/rt_math_fixed_trig_accuracy/forward.rs` (174): tests (2) fixed_sin_cos_tan_are_within_one_unit_and_tan_overflow_raises, the_oracle_agrees_with_host_floating_point_where_both_are_exact_enough; items ERR_OVERFLOW, PROGRAM_HEAD, run, fixed_text [lines 26-26, 333-357, 359-416, 418-420, 430-502, 504-523]
- `tests/runtime/rt_math_fixed_trig_accuracy/oracle.rs` (support) (146): items big, pow2, div_floor, atan_inverse, pi_over_2, sin_cos_small, sqrt_scaled, atan_scaled, asin_scaled, atan2_scaled, Truth, truth [lines 30-32, 34-36, 38-46, 48-65, 67-70, 72-98, 100-103, 105-137, 139-152, 154-174, 176-180, 182-197]
- `tests/runtime/rt_math_fixed_trig_accuracy/inputs.rs` (support) (88): items random_raws, near_pole_raws, corpus [lines 233-249, 251-288, 294-331]
- `tests/runtime/rt_math_fixed_trig_accuracy/inverse.rs` (171): tests (1) fixed_inverse_trig_is_within_one_unit [lines 525-706]
Surface: pub(super) on 9 item(s): big, pow2, pi_over_2, atan_scaled, asin_scaled, atan2_scaled, truth, random_raws, corpus; plus struct Truth fields sin, cos, tan; root adds `use oracle::*;`, `use inputs::*;`.
Sweeps: Cargo.toml:81 dependency comment names the path (`grep -n rt_math_fixed_trig_accuracy Cargo.toml`) — unchanged.

#### 4.30 `tests/runtime/rt_math_float_trig_large_angles.rs` — 384 test LOC, 3 tests

- `tests/runtime/rt_math_float_trig_large_angles.rs` (11): header; mods oracle, large_angles, signed_zero
- `tests/runtime/rt_math_float_trig_large_angles/oracle.rs` (support) (108): items W, big, pow2, div_floor, atan_inverse, pi_over_2, sin_cos_small, scaled, truth, ulp_scaled, within_one_ulp [lines 25-26, 28-30, 32-34, 36-43, 45-61, 63-65, 67-92, 94-116, 118-133, 135-139, 141-146]
- `tests/runtime/rt_math_float_trig_large_angles/large_angles.rs` (210): tests (2) trig_is_within_one_ulp_at_every_large_finite_angle, the_oracle_agrees_with_host_libm_at_ordinary_angles; items literal, corpus, Row, NAMES, run, check [lines 148-156, 158-209, 211-217, 219-219, 221-308, 310-338, 340-353, 355-375]
- `tests/runtime/rt_math_float_trig_large_angles/signed_zero.rs` (65): tests (1) a_zero_argument_keeps_its_sign [lines 377-446]
Surface: pub(super) on 6 item(s): big, pi_over_2, scaled, truth, ulp_scaled, within_one_ulp; root adds `use oracle::*;`.
Sweeps: `src/docs/spec/architecture/18_math-kernels.md:65` names the root path (prose, `grep -rn rt_math_float_trig_large_angles src/docs`) — unchanged. Cargo.toml:81 dependency comment names the path — unchanged.

#### 4.31 `tests/runtime/rt_debug_arena.rs` — 418 test LOC, 8 tests

- `tests/runtime/rt_debug_arena.rs` (11): header; mods registration, counters, series
- `tests/runtime/rt_debug_arena/registration.rs` (150): tests (2) three_workers_register_seven_arenas, the_canvas_graphics_thread_registers_a_graphics_arena; items COUNTERS, registration [lines 19-43, 45-75, 77-129, 131-183]
- `tests/runtime/rt_debug_arena/counters.rs` (173): tests (4) churn_counts_scale_with_iterations_and_partition_by_path, a_mebibyte_read_grows_the_arena, retained_values_raise_peak_live_bytes_and_churn_does_not, fill_counters_match_known_allocations; items churn_source, retain_source, main_arena, assert_partition, record_churn_source [lines 185-190, 192-198, 200-205, 207-224, 226-256, 258-291, 293-314, 316-322, 324-386]
- `tests/runtime/rt_debug_arena/series.rs` (93): tests (2) the_series_rises_in_three_bursts, the_series_stays_bounded_and_ordered; items series [lines 388-399, 401-440, 442-493]
Surface: none (no support file; root keeps every shared helper).
Sweeps: none (§4.0 commands: 0 hits).

#### 4.32 `tests/runtime/rt_debug_report.rs` — 500 test LOC, 9 tests

- `tests/runtime/rt_debug_report.rs` (151): header; helpers host_target, assert_block, build, Run, run, split_report [lines 24-35, 37-111, 113-140, 142-146, 148-155, 157-177]; mods exit_paths, codegen, measurements
- `tests/runtime/rt_debug_report/exit_paths.rs` (188): tests (5) a_normal_return_ends_stderr_with_the_report, exit_program_ends_stderr_with_the_report, an_untrapped_error_ends_stderr_with_the_report, sigterm_mid_sleep_ends_stderr_with_the_report, a_headless_app_finish_ends_stderr_with_the_report; items assert_report_on_exit, run_until_sigterm [lines 179-203, 205-211, 213-230, 232-247, 249-288, 290-327, 484-535]
- `tests/runtime/rt_debug_report/codegen.rs` (109): tests (2) only_a_debug_build_imports_the_peak_rss_call, every_cross_target_calls_the_report_right_after_shutdown_done; items build_ncode, functions [lines 329-355, 357-359, 390-420, 422-482]
- `tests/runtime/rt_debug_report/measurements.rs` (61): tests (2) a_64_mib_string_reports_at_least_64_mib_peak_rss, the_perf_section_times_the_arena; items report_value [lines 361-370, 372-388, 537-576]
Surface: none (no support file; root keeps every shared helper).
Sweeps: `.ai/testing-gates.md:348` names the target (`grep -rn rt_debug_report .ai`) — target name unchanged, still valid. No include/respawn.

#### 4.33 `tests/runtime/rt_debug_soak.rs` — 1164 test LOC, 49 tests

- `tests/runtime/rt_debug_soak.rs` (104): header; helpers FLAT_BOUND, BLOCK_BOUND, main_live_bytes, assert_live_bytes_flat, assert_live_bytes_within, main_arena_calls, debug_run, assert_block_flat [lines 24-26, 28-32, 111-117, 119-129, 131-148, 807-817, 819-828, 992-1021]; mods browser, text_parsing, network, tls, attributed_strings, crypto, resource_ownership, trap_and_state, threads, record_frees
- `tests/runtime/rt_debug_soak/browser.rs` (144): tests (3) a_dom_parse_loop_keeps_live_bytes_constant, a_resolve_styles_loop_keeps_live_bytes_constant, a_paint_loop_keeps_live_bytes_constant; items repo_root, copy_tree, install_browser_package, browser_project [lines 34-36, 38-55, 57-75, 77-109, 229-250, 252-291, 293-312]
- `tests/runtime/rt_debug_soak/text_parsing.rs` (86): tests (3) a_flat_split_loop_keeps_live_bytes_constant, a_json_parse_loop_keeps_live_bytes_constant, a_thread_copy_back_loop_keeps_live_bytes_constant; items write_split_input, write_json_input [lines 150-161, 163-178, 180-202, 204-227, 314-331]
- `tests/runtime/rt_debug_soak/network.rs` (133): tests (5) an_http_read_loop_keeps_live_bytes_constant, a_tcp_read_loop_keeps_live_bytes_constant, a_tcp_connect_close_loop_keeps_live_bytes_constant, a_udp_bind_close_loop_keeps_live_bytes_constant, an_http_read_loop_leaves_no_block_behind; items serve_http, serve_tcp [lines 333-353, 355-376, 378-391, 393-414, 416-436, 438-457, 740-763]
- `tests/runtime/rt_debug_soak/tls.rs` (136): tests (2) a_tls_connect_close_loop_keeps_live_bytes_constant, a_tls_socket_closed_on_another_thread_keeps_live_bytes_constant; items have_openssl_peer, serve_tls [lines 459-469, 471-539, 541-571, 898-934]
- `tests/runtime/rt_debug_soak/attributed_strings.rs` (99): tests (5) a_bound_attributed_string_keeps_live_bytes_constant, a_list_of_attributed_strings_keeps_live_bytes_constant, a_record_of_attributed_strings_keeps_live_bytes_constant, an_attributed_string_with_an_attribute_keeps_live_bytes_constant, a_defaulted_record_keeps_live_bytes_constant; items astrings_project [lines 573-582, 584-603, 605-623, 625-643, 645-663, 665-685]
- `tests/runtime/rt_debug_soak/crypto.rs` (45): tests (2) a_native_ec_generate_loop_keeps_live_bytes_constant, a_software_curve_generate_loop_keeps_live_bytes_constant; items crypto_generate_project [lines 687-703, 705-718, 720-738]
- `tests/runtime/rt_debug_soak/resource_ownership.rs` (138): tests (6) a_resource_union_bound_from_a_producer_keeps_live_bytes_constant, a_resource_union_aliasing_a_binding_keeps_live_bytes_constant, a_resource_passed_through_a_function_is_freed_once, a_returned_union_wrapping_an_owned_local_stays_open_in_the_caller, a_union_alias_in_an_inner_scope_leaves_the_outer_handle_open, a_resource_not_returned_on_a_sibling_path_is_still_closed; items run_with_fd_limit [lines 765-784, 786-805, 830-851, 853-874, 876-896, 936-961, 963-990]
- `tests/runtime/rt_debug_soak/trap_and_state.rs` (126): tests (10) a_trap_bound_resource_union_keeps_live_bytes_constant, a_trap_bound_concrete_resource_keeps_live_bytes_constant, a_trap_bind_whose_producer_fails_keeps_live_bytes_constant, a_trapped_borrowed_get_keeps_live_bytes_constant, a_trapped_borrowed_poll_with_an_owned_recover_keeps_live_bytes_constant, a_stateful_resource_union_keeps_live_bytes_constant, a_stateful_concrete_resource_keeps_live_bytes_constant, an_in_place_state_scalar_update_keeps_live_bytes_constant, an_owned_list_of_resource_unions_keeps_live_bytes_constant, an_owned_list_of_concrete_resources_keeps_live_bytes_constant [lines 1023-1046, 1048-1063, 1065-1088, 1090-1110, 1112-1127, 1129-1154, 1156-1173, 1175-1191, 1193-1209, 1211-1228]
- `tests/runtime/rt_debug_soak/threads.rs` (145): tests (10) a_thread_resource_transfer_loop_keeps_live_bytes_constant, a_thread_string_send_loop_keeps_live_bytes_constant, a_thread_send_of_a_computed_message_keeps_live_bytes_constant, a_worker_send_of_a_computed_message_keeps_live_bytes_constant, an_undelivered_queued_message_is_freed_when_the_thread_is_released, a_stateful_resource_transfer_frees_its_record_and_its_state, a_thread_start_seed_is_freed_when_the_thread_is_released, a_thread_start_with_a_literal_seed_frees_nothing, a_thread_start_with_a_local_seed_leaves_it_to_its_binding, a_failed_thread_start_frees_its_seed [lines 1230-1249, 1251-1267, 1269-1288, 1290-1329, 1392-1418, 1420-1439, 1441-1459, 1461-1476, 1478-1494, 1496-1520]
- `tests/runtime/rt_debug_soak/record_frees.rs` (38): tests (3) an_fs_file_binding_frees_its_resource_record, an_fs_file_bound_through_an_inline_trap_frees_both_records, an_unfloated_list_of_res_frees_its_collection_block [lines 1331-1352, 1354-1372, 1374-1391]
Surface: none (no support file; root keeps every shared helper).
Sweeps: `.ai/testing-gates.md:865` cites `tests/runtime/rt_debug_soak.rs`'s `BLOCK_BOUND` (`grep -rn rt_debug_soak .ai`) → BLOCK_BOUND stays in the root, citation stays valid. `repo_root().join("tests/runtime/data/rt_debug_soak_basic.html")` (line 233, moves to browser.rs) is CARGO_MANIFEST_DIR-relative, unaffected. Module-doc command `cargo test --release --test rt_debug_soak -- --include-ignored` unaffected.

#### 4.34 `tests/runtime/rt_io_key_readers.rs` — 563 test LOC, 13 tests

- `tests/runtime/rt_io_key_readers.rs` (75): header; helpers KEY_DUMP, key_dump, run_with_timed_stdin [lines 18-54, 56-58, 142-165]; mods pipe, timed, terminal, kitty
- `tests/runtime/rt_io_key_readers/pipe.rs` (113): tests (4) read_key_decodes_every_encoding_on_a_pipe, the_text_readers_return_text_only, read_raw_is_byte_exact_on_a_pipe, every_reader_builds_and_reads_in_a_mouse_program [lines 60-92, 94-115, 117-140, 594-637]
- `tests/runtime/rt_io_key_readers/timed.rs` (100): tests (3) a_timed_read_line_keeps_the_part_of_the_line_it_had_read, a_timed_read_waits_its_timeout_and_no_longer, a_timeout_of_zero_is_one_immediate_attempt [lines 167-208, 210-229, 231-273]
- `tests/runtime/rt_io_key_readers/terminal.rs` (117): tests (2) a_terminal_reads_one_key_at_a_time_without_echo, a_timed_read_line_on_a_terminal_leaves_the_half_typed_line; items run_on_terminal [lines 275-330, 332-364, 366-397]
- `tests/runtime/rt_io_key_readers/kitty.rs` (170): tests (4) term_on_switches_a_kitty_terminal_to_key_events_and_off_restores_it, term_on_leaves_a_legacy_terminal_alone_and_keeps_keys_typed_meanwhile, read_line_under_term_on_reads_the_line_in_the_terminals_ordinary_encoding, ctrl_c_under_the_kitty_protocol_still_interrupts_the_program; items run_on_replying_terminal, run_on_replying_terminal_status, TERM_KEYS [lines 399-408, 410-474, 476-496, 498-525, 527-540, 542-572, 574-592]
Surface: none (no support file; root keeps every shared helper).
Sweeps: none (§4.0 commands: 0 hits).

#### 4.35 `tests/runtime/rt_native_io_runtime.rs` — 581 test LOC, 15 tests

- `tests/runtime/rt_native_io_runtime.rs` (13): header; mods stdin_and_flush, resource_cleanup, terminal, failures
- `tests/runtime/rt_native_io_runtime/stdin_and_flush.rs` (113): tests (5) native_io_reads_from_stdin_and_preserves_input_semantics, native_io_readline_grows_buffer_and_reuses_freed_chunks, native_io_flush_reports_standard_stream_failures, native_io_input_flushes_stdout_before_reading, native_io_input_reports_prompt_flush_failure [lines 11-31, 33-60, 62-90, 285-312, 446-472]
- `tests/runtime/rt_native_io_runtime/resource_cleanup.rs` (170): tests (3) native_resource_cleanup_reports_secondary_close_failure_metadata, native_exit_program_runs_caller_resource_cleanup, native_loop_exit_and_continue_run_body_resource_cleanup [lines 92-151, 153-214, 216-283]
- `tests/runtime/rt_native_io_runtime/terminal.rs` (141): tests (5) native_io_input_echoes_terminal_line_input, native_io_readline_suppresses_terminal_echo_until_newline, native_io_readgrapheme_reads_terminal_key_without_newline_or_echo, native_io_readraw_reads_terminal_key_without_newline_or_echo, native_io_terminal_helpers_cover_pipe_and_tty_execution [lines 314-340, 342-374, 376-411, 413-444, 554-591]
- `tests/runtime/rt_native_io_runtime/failures.rs` (156): tests (2) native_io_reports_eof_and_encoding_failures, native_io_short_write_returns_do_not_truncate_output [lines 474-552, 593-684]
Surface: none (no support file; root keeps every shared helper).
Sweeps: `.ai/arch-abi.md:309` names the target — unchanged. The root's `#[cfg(unix)] use std::...` (lines 4-5) stays in the root; topics see it through `use super::*` under the same cfg.

#### 4.36 `tests/runtime/rt_native_term_runtime.rs` — 1038 test LOC, 20 tests

- `tests/runtime/rt_native_term_runtime.rs` (94): header; helpers mouse_decoder_source [lines 87-170]; mods mouse_decoding, mouse_escapes, mouse_ring, lifecycle, console, attributed_text
- `tests/runtime/rt_native_term_runtime/mouse_decoding.rs` (126): tests (3) native_term_mouse_is_opt_in_and_silent_until_asked, native_term_mouse_decodes_every_kind_and_modifier, native_term_mouse_leaves_other_input_untouched [lines 9-85, 172-215, 217-268]
- `tests/runtime/rt_native_term_runtime/mouse_escapes.rs` (210): tests (5) native_term_mouse_poll_input_keeps_the_byte_after_an_escape, native_term_mouse_escape_at_end_of_input_is_delivered, native_term_mouse_lone_escape_is_reported_without_a_following_key, native_term_mouse_line_read_keeps_the_bytes_after_an_escape, native_term_mouse_does_not_echo_during_a_line_read; items mouse_poll_then_read_source, run_with_held_stdin [lines 270-298, 300-331, 333-348, 350-362, 364-417, 1099-1160, 1162-1217]
- `tests/runtime/rt_native_term_runtime/mouse_ring.rs` (193): tests (3) native_term_mouse_ring_overwrites_oldest_and_expires_stale, native_term_mouse_keeps_immediate_reads_honest, native_term_mouse_is_per_thread_and_needs_stdin [lines 419-511, 1030-1097, 1219-1299]
- `tests/runtime/rt_native_term_runtime/lifecycle.rs` (139): tests (3) native_term_off_withdraws_mouse_tracking, native_term_did_resize_reports_false_without_a_resize, native_term_on_resets_state_to_defaults [lines 513-560, 751-804, 806-862]
- `tests/runtime/rt_native_term_runtime/console.rs` (166): tests (3) native_term_size_reports_unsupported_off_and_size_when_active, native_term_console_emits_expected_escape_sequences, native_term_gate_no_ops_while_inactive [lines 562-632, 634-685, 687-749]
- `tests/runtime/rt_native_term_runtime/attributed_text.rs` (128): tests (3) native_term_draw_text_attributed_applies_bold_underline, native_term_draw_text_attributed_applies_colors, native_term_draw_text_attributed_ignores_alpha [lines 864-912, 914-955, 957-1028]
Surface: none (no support file; root keeps every shared helper).
Sweeps: none (§4.0 commands: 0 hits).

#### 4.37 `tests/runtime/rt_foreign_entry_callee_saved.rs` — 480 test LOC, 6 tests

- `tests/runtime/rt_foreign_entry_callee_saved.rs` (121): header; helpers ALL_TARGETS, audit_app, assert_audited; tests (6) macos_app_imps_preserve_callee_saved, gtk_aarch64_callbacks_preserve_callee_saved, gtk_x86_64_callbacks_preserve_callee_saved, win32_window_procedure_preserves_callee_saved, thread_start_routine_preserves_callee_saved, tls_verify_callback_preserves_callee_saved [lines 106-112, 474-486, 488-495, 497-513, 515-532, 534-547, 549-557, 559-568, 570-579]; mods fixtures, audit, escape
- `tests/runtime/rt_foreign_entry_callee_saved/fixtures.rs` (support) (97): items MOUSE_SURFACES, MOUSE_CONSOLE, app_project, copy_dir, fixture_project [lines 43-83, 85-104, 429-443, 445-459, 461-472]
- `tests/runtime/rt_foreign_entry_callee_saved/audit.rs` (support) (129): items MFB_DISPATCHERS, allocator_preserved, callee_saved, is_mfb_function_value, is_app_backend_body, audit [lines 114-118, 120-149, 151-176, 333-343, 345-349, 351-427]
- `tests/runtime/rt_foreign_entry_callee_saved/escape.rs` (support) (145): items Loc, State, field, is_stack_base, offset, immediate, escapes [lines 178-182, 184-190, 192-194, 196-198, 200-204, 206-210, 212-331]
Surface: pub(super) on 8 item(s): MOUSE_SURFACES, MOUSE_CONSOLE, callee_saved, field, escapes, audit, app_project, fixture_project; plus struct State fields values; root adds `use escape::*;`, `use audit::*;`, `use fixtures::*;`.
Sweeps: `.ai/arch-abi.md:522` and `src/docs/spec/app/04_term-backend.md:342` name the file path (`grep -rn rt_foreign_entry_callee_saved .ai src/docs`) — root path stays, still valid (prose, not a `[[path:symbol]]` citation). `fixture_project` (→ fixtures.rs) joins `env!("CARGO_MANIFEST_DIR")`, absolute, unaffected. Tests stay in the root (they are 6 thin calls into the audit; the bulk is the audit machinery, split by responsibility into escape/audit/fixtures).

#### 4.38 `tests/runtime/rt_inplace_self_update.rs` — 1572 test LOC, 4 tests

- `tests/runtime/rt_inplace_self_update.rs` (43): header; helpers DEFAULT_N, EXEMPT_M, VALUE_M, CASES, FIELD_EXPECT, FIELD_KINDS [lines 96-96, 98-100, 101-101, 103-103, 104-104, 105-105]; mods case_grid, build, site_programs, variant_programs, scratch_arms, case_check, field_sites, field_cases, field_programs, field_check, grids
- `tests/runtime/rt_inplace_self_update/case_grid.rs` (support) (160): items PRELUDE, Status, Case, impl Case, len_of, extra_imports, prelude_for, cases [lines 107-176, 178-185, 187-198, 200-207, 209-218, 220-239, 241-245, 273-310]
- `tests/runtime/rt_inplace_self_update/build.rs` (support) (154): items STDIN_LINES, output_with_stdin, run_peak, build_debug, run, Build, build_debug_as, run_lines [lines 247-249, 251-271, 964-994, 1028-1055, 1057-1064, 1066-1072, 1074-1103, 1105-1147]
- `tests/runtime/rt_inplace_self_update/site_programs.rs` (support) (180): items Site, ENABLED_SITES, impl Site, frame, at_site, sized, return_program [lines 312-344, 346-353, 355-390, 392-403, 405-441, 443-446, 448-632]
- `tests/runtime/rt_inplace_self_update/variant_programs.rs` (support) (201): items SCRATCH_ARMS, statement_target, case_needs_scratch, owned_param_program, program, replace_ident, result_program [lines 634-664, 666-672, 674-687, 719-825, 827-849, 851-876, 878-917]
- `tests/runtime/rt_inplace_self_update/scratch_arms.rs` (25): tests (1) scratch_arms_match_the_compiler [lines 689-717]
- `tests/runtime/rt_inplace_self_update/case_check.rs` (support) (122): items exempt_program, exempt_check, check [lines 919-962, 996-1026, 1149-1221]
- `tests/runtime/rt_inplace_self_update/field_sites.rs` (support) (154): items FieldSite, FIELD_SITES, impl FieldSite, FieldExpect, impl FieldExpect, field_expectations [lines 1227-1266, 1268-1284, 1286-1371, 1373-1384, 1386-1405, 1407-1430]
- `tests/runtime/rt_inplace_self_update/field_cases.rs` (support) (151): items FieldCase, impl FieldCase, field_kinds, kind_helpers [lines 1432-1454, 1456-1492, 1494-1551, 1553-1600]
- `tests/runtime/rt_inplace_self_update/field_programs.rs` (support) (215): items field_frame, at_field_site, field_statement_lines, field_prologue, field_program, field_result_program, field_extra [lines 1602-1662, 1664-1699, 1701-1720, 1722-1734, 1736-1769, 1771-1800, 1802-1844]
- `tests/runtime/rt_inplace_self_update/field_check.rs` (support) (111): items check_field, site_filter, label_filter, Work, run_work [lines 1846-1909, 1911-1916, 1918-1923, 1925-1929, 1931-1964]
- `tests/runtime/rt_inplace_self_update/grids.rs` (98): tests (3) every_field_kind_meets_its_expectation, every_self_update_case_meets_its_allocation_bound, every_via_let_field_cell_meets_its_expectation; items VIA_LET_SAMPLE [lines 1966-1988, 1990-2025, 2027-2035, 2037-2075]
Surface: pub(super) on 36 item(s): Status, Case, len_of, prelude_for, cases, Site, ENABLED_SITES, frame, at_site, sized, return_program, SCRATCH_ARMS, case_needs_scratch, program, replace_ident, result_program, run_peak, run, Build, build_debug_as, run_lines, check, FieldSite, FIELD_SITES, FieldExpect, field_expectations, FieldCase, field_kinds, kind_helpers, field_program, field_result_program, field_extra, site_filter, label_filter, Work, run_work; plus struct Case fields signature, status, decl, setup, statements, check, n; impl Case fns ty; impl Site fns applies; impl FieldSite fns code, from_code, field, sibling, statement, statement_with, control, owner_init; impl FieldExpect fns parse; struct FieldCase fields label, build, ty, decl, setup, statements, check, value_bound, has_len, via_let, n, expect; impl FieldCase fns from_case; root adds `use case_grid::*;`, `use site_programs::*;`, `use variant_programs::*;`, `use build::*;`, `use case_check::*;`, `use field_sites::*;`, `use field_cases::*;`, `use field_programs::*;`, `use field_check::*;`.
Sweeps: (1) `include_str!("inplace_self_update/cases.tsv")`, `field_expect.tsv`, `field_kinds.tsv` (consts CASES/FIELD_EXPECT/FIELD_KINDS, lines 103-105) are PINNED in the root so their relative paths stay valid; (2) `scratch_arms_match_the_compiler` (line 696) `include_str!("../../src/codegen/collection/assign/self_update.rs")` moves to `rt_inplace_self_update/scratch_arms.rs` → must become `include_str!("../../../src/codegen/collection/assign/self_update.rs")`. (3) Refs (`grep -rn rt_inplace_self_update tests/guards .ai .github Cargo.toml`): Cargo.toml [[test]] stanza (path unchanged), `tests/guards/inplace_self_update_census.rs:6` doc (path, valid), `.ai/collections.md:92`, `.ai/testing-gates.md:77` ("two grid tests are #[ignore]d" — still true, now in grids.rs), `.github/workflows/coverage.yml:174-185` (`cargo test --release --test rt_inplace_self_update -- --ignored` — target name unchanged, `--ignored` still selects them inside `grids::`). Module doc filter `... rt_inplace_self_update via_let -- --ignored` still matches `grids::every_via_let_field_cell_meets_its_expectation`. Existing dir `tests/runtime/inplace_self_update/` (tsv data) does not collide with the new `tests/runtime/rt_inplace_self_update/`.
Cross-letter note: letter Q turns `src/codegen/collection/assign/self_update.rs` into a directory; Q repoints this `include_str!` (a missing path is a compile error, and a moved `SCRATCH_ARMS` declaration fails the test's `expect("the compiler's SCRATCH_ARMS declaration")`), so neither can pass silently. That repoint is Q's scope, not B's.

## Phases

> **NOTE — keep the checkboxes current as you go. This is not bookkeeping; it is
> the only way anyone can see where the work actually is.**
>
> - Tick `- [x]` **in the same commit as the work it describes** — never in a
>   batch at the end of a phase, never "I'll update the plan after". If it is a
>   separate step it will be skipped, and the list becomes a lie.
> - Use `- [~]` for partially done and say in one line what remains.
> - Mark a task moot with `- [x] ~~text~~ — moot: <evidence>` rather than
>   deleting it. A vanished task is indistinguishable from a forgotten one.
> - Fill the phase's `Commit:` line with the hash(es) the moment it lands.
> - Add any task you discover you needed; an unlisted task that got done is a
>   task the next reader will do twice.
>
> **An unticked box means NOT DONE.** Someone reading this file — including the
> person who has to decide whether to stop — is entitled to trust that.

Every phase follows the same order. First run K1, then split each file per its §4 entry (create
the files, move the items verbatim, add the `#[path]` pairs, the root globs and the `pub(super)`
edits), then run K2–K5, then commit the phase: the new files, the edited roots, and the ceiling
file (plus `census.py` when the phase adds an exempt row). Phases are ordered lowest risk first:
small two-way splits with no hazards, then support-file splits, then the exempt units, then the
files with prose references and the most parts, and finally rt_inplace_self_update.

### Phase 1 — Package, import and overload tests (§4.1–§4.9)

Nine files with no support module and no hazard; this phase proves the gate kit.
`T="rt_foreign_type_reexport rt_global_argument_reassigned_by_callee rt_imported_package_global rt_imported_type_name_collision rt_imported_union_enum_members rt_package_dependency_closure rt_overloaded_function_values rt_nothing_collections rt_moved_handle_guards"`,
`RE="rt_(foreign_type_reexport|global_argument_reassigned_by_callee|imported_package_global|imported_type_name_collision|imported_union_enum_members|package_dependency_closure|overloaded_function_values|nothing_collections|moved_handle_guards)"`.

- [ ] K1 on the unsplit tree → 84 tests.
- [ ] `tests/runtime/rt_foreign_type_reexport.rs` → root + `reexport.rs`, `abi_versions.rs` (§4.1).
- [ ] `tests/runtime/rt_global_argument_reassigned_by_callee.rs` → root + `borrowing.rs`, `argument_copy.rs` (§4.2).
- [ ] `tests/runtime/rt_imported_package_global.rs` → root + `access.rs`, `refusals.rs` (§4.3).
- [ ] `tests/runtime/rt_imported_type_name_collision.rs` → root + `local_collisions.rs`, `package_collisions.rs` (§4.4).
- [ ] `tests/runtime/rt_imported_union_enum_members.rs` → root + `unions.rs`, `enums.rs` (§4.5).
- [ ] `tests/runtime/rt_package_dependency_closure.rs` → root + `refusals.rs`, `declared_closures.rs` (§4.6).
- [ ] `tests/runtime/rt_overloaded_function_values.rs` → root + `rejection.rs`, `selection.rs`, `imports_and_generics.rs` (§4.7).
- [ ] `tests/runtime/rt_nothing_collections.rs` → root + `lists.rs`, `maps_and_sets.rs` (§4.8).
- [ ] `tests/runtime/rt_moved_handle_guards.rs` → root + `file_socket_thread.rs`, `tls_listener.rs` (§4.9). The `#[cfg(any(target_os = "macos", target_os = "linux"))]` attribute moves with its item.
- [ ] Tighten: K5, then commit `tests/guards/file_size_ceilings.txt` with the phase.

Acceptance: 84 tests are listed under the same names and all pass, and the nine paths leave the
budget banner.
  Check: K2 → no output; K3 → 9 × `test result: ok`, 0 failed; K4 → no output; K5 → 9 TIGHTEN
  rows, then none (est. 15 min: nine targets each build and run MFBASIC programs; K3 is the run of
  exactly the moved tests).
Commit: —

### Phase 2 — Inline trap, error model and callback-free tests (§4.10–§4.19)

Ten files, five of them with data-only support modules, and the first exempt row.
`T="rt_callback_failure_frees rt_closure_single_owner rt_domain_error_frees_locals rt_error_model_recovery rt_inline_trap_expression_boundary rt_inline_trap_nested_call rt_inline_trap_raising_operator rt_inline_trap_short_circuit rt_inline_trap_success_alloc rt_inline_trap_union_bind"`,
`RE="rt_(callback_failure_frees|closure_single_owner|domain_error_frees_locals|error_model_recovery|inline_trap_expression_boundary|inline_trap_nested_call|inline_trap_raising_operator|inline_trap_short_circuit|inline_trap_success_alloc|inline_trap_union_bind)"`.

- [ ] K1 → 76 tests.
- [ ] `rt_callback_failure_frees.rs` → root + support `cases.rs` (no `use super::*;`; `pub(super)` HELPERS, CASES, trapped; root `use cases::*;`) (§4.10).
- [ ] `rt_closure_single_owner.rs` → root + support `cases.rs` (no `use super::*;`; `pub(super)` CASES; root `use cases::*;`) (§4.11).
- [ ] Exempt row for `tests/runtime/rt_closure_single_owner/cases.rs`: add to
      `tests/guards/file_size_ceilings.txt`
      `exempt test tests/runtime/rt_closure_single_owner/cases.rs # one const CASES table of (name, program, expected) rows read by the file's one #[test]; splitting it chains per-holder tables in the test (a logic edit)`,
      and add the same path, kind and reason to `EXEMPT` in `tools/loc-census/census.py`.
- [ ] `rt_domain_error_frees_locals.rs` → root + support `program.rs` (no `use super::*;`) (§4.12).
- [ ] `rt_error_model_recovery.rs` → root + support `routes.rs` (no `use super::*;`; `pub(super)` ROWS, struct Row and its fields name, body, escapes, recovered) + `routing.rs`, `error_values.rs`, `memory.rs` (§4.13).
- [ ] `rt_inline_trap_expression_boundary.rs` → root + `raising_operators.rs`, `handler_shape.rs`, `allocation.rs` (§4.14).
- [ ] `rt_inline_trap_nested_call.rs` → root + `nesting.rs`, `roots.rs`, `handler_forms.rs`, `builtins_and_conversions.rs` (§4.15).
- [ ] `rt_inline_trap_raising_operator.rs` → root + `operators.rs`, `evaluation_order.rs`, `lifting.rs` (§4.16).
- [ ] `rt_inline_trap_short_circuit.rs` → root + `selection.rs`, `targets.rs`, `allocation.rs` (§4.17).
- [ ] `rt_inline_trap_success_alloc.rs` → root + support `shapes.rs` (no `use super::*;`; `pub(super)` Shape, SHAPES and Shape's seven fields) + `success_path.rs`, `failing_shapes.rs` (§4.18).
- [ ] `rt_inline_trap_union_bind.rs` → root + `data_unions.rs`, `resource_unions.rs` (§4.19).
- [ ] Tighten: K5 and commit the ceiling file and `census.py`.

Acceptance: 76 tests are unchanged and pass, and the only remaining budget row for these paths
is the `exempt` row.
  Check: K2 → no output; K3 → 10 × ok; K4 → no output; K5 → 10 TIGHTEN rows, then none;
  `grep -E "tests/runtime/$RE" tests/guards/file_size_ceilings.txt` → exactly the one `exempt` line;
  `python3 -I tools/loc-census/census.py --seed | grep rt_closure_single_owner` → the same `exempt`
  line (est. 15 min).
Commit: —

### Phase 3 — Collection and memory-bound tests (§4.20–§4.27)

Eight files, including rt_owned_argument, which reads its own source.
`T="rt_hof_transient_bytes rt_inplace_failure_atomic rt_json_bounds rt_list_append_growth_bounds rt_list_element_record_borrow rt_list_take_drop_in_place rt_map_integer_key_probe_cost rt_owned_argument"`,
`RE="rt_(hof_transient_bytes|inplace_failure_atomic|json_bounds|list_append_growth_bounds|list_element_record_borrow|list_take_drop_in_place|map_integer_key_probe_cost|owned_argument)"`.

- [ ] K1 → 60 tests.
- [ ] `rt_hof_transient_bytes.rs` → root + `partition_group_by.rs`, `failing_callback.rs` (§4.20).
- [ ] `rt_inplace_failure_atomic.rs` → root + `binding.rs`, `field.rs`, `reallocating_field.rs` (§4.21).
- [ ] `rt_json_bounds.rs` → root + `grammar.rs`, `cost.rs`, `semantics.rs` (§4.22).
- [ ] `rt_list_append_growth_bounds.rs` → root + `map_and_peak.rs`, `list_arms.rs` (§4.23).
- [ ] `rt_list_element_record_borrow.rs` → root + `bounded_reads.rs`, `copies.rs`, `element_updates.rs`, `union_elements.rs` (§4.24).
- [ ] `rt_list_take_drop_in_place.rs` → root + `scaling.rs`, support `scale_program.rs` and `value_program.rs` (both without `use super::*;`), `value_semantics.rs` (§4.25).
- [ ] `rt_map_integer_key_probe_cost.rs` → root + `cost.rs`, `hash_sites.rs` (§4.26).
- [ ] `rt_owned_argument.rs` → root keeps `the_ignored_set_is_exactly_the_five_open_cases` and the
      seven case tests it scans (local_return, helper_append, helper_map_set, helper_concat,
      record_field, bind_param, recursive_fill), + support `cases.rs` (no `use super::*;`) and
      `support.rs`, + `failure_route.rs`, `hand_over.rs` (§4.27).
- [ ] rt_owned_argument guard proof: `grep -c '#\[ignore = "plan-147-' tests/runtime/rt_owned_argument.rs`
      gives the same number before and after the split. The ignores the guard scans must still be in
      the file it reads.
- [ ] Tighten: K5 and commit the ceiling file.

Acceptance: 60 tests are unchanged and pass, and the self-reading guard still sees every ignore.
  Check: K2 → no output; K3 → 8 × ok; K4 → no output; K5 → 8 TIGHTEN rows, then none; the
  ignore-count grep is equal before and after (est. 15 min).
Commit: —

### Phase 4 — Big integers and math kernels (§4.28–§4.30)

Three files, the word_boundaries extraction, and the remaining exempt row.
`T="rt_big_int rt_math_fixed_trig_accuracy rt_math_float_trig_large_angles"`,
`RE="rt_(big_int|math_fixed_trig_accuracy|math_float_trig_large_angles)"`.

- [ ] K1 → 23 tests.
- [ ] `rt_big_int.rs` → root + nine topics (§4.28).
- [ ] word_boundaries extraction: write the program literal's bytes (after `r#"` on line 2403 through
      line 2550's `END FUNC\n`) verbatim to `tests/runtime/rt_big_int/word_boundaries.mfb`. In
      `word_boundaries.rs`, replace the literal with `include_str!("word_boundaries.mfb")` as `run`'s
      second argument.
- [ ] Byte check: `git show HEAD:tests/runtime/rt_big_int.rs | python3 -I -c 'import sys; t=sys.stdin.read(); s=t.index("fn word_boundaries_match_an_independent_oracle"); a=t.index("r#\"", s)+3; sys.stdout.write(t[a:t.index("\"#", a)])' | cmp - tests/runtime/rt_big_int/word_boundaries.mfb`
      → no output (`HEAD` is the tree before this phase).
- [ ] Count check: `python3 -I tools/loc-census/census.py --file tests/runtime/rt_big_int/word_boundaries.rs` → `0 141`,
      and `python3 -I tools/loc-census/census.py --file tests/runtime/rt_big_int/word_boundaries.mfb` → `0 143`.
      Both are ≤ 250.
- [ ] Exempt row, in `tests/guards/file_size_ceilings.txt` and `census.py`'s `EXEMPT`:
      `exempt test tests/runtime/rt_big_int/powers.rs # one #[test] whose body is a single inline MFBASIC program and its expected output; the program is the test, and as an .mfb under tests/ it is the same over-limit test file`.
- [ ] `rt_math_fixed_trig_accuracy.rs` → root + `forward.rs`, support `oracle.rs` and `inputs.rs`, `inverse.rs` (§4.29).
- [ ] `rt_math_float_trig_large_angles.rs` → root + support `oracle.rs`, `large_angles.rs`, `signed_zero.rs` (§4.30).
- [ ] Tighten: K5 and commit the ceiling file and `census.py`.

Acceptance: 23 tests are unchanged and pass, the extracted program is byte-identical, and the
single exempt row is present in both the ceiling file and the oracle.
  Check: K2 → no output; K3 → 3 × ok; K4 → no output; K5 → 3 TIGHTEN rows, then none;
  `python3 -I tools/loc-census/census.py --seed | grep 'rt_big_int/'` → exactly the `powers.rs` `exempt` row; the byte `cmp` → no output
  (est. 10 min).
Commit: —

### Phase 5 — Debug report, soak, I/O and terminal runtime tests (§4.31–§4.37)

Seven files with the most parts and the most prose references.
`T="rt_debug_arena rt_debug_report rt_debug_soak rt_io_key_readers rt_native_io_runtime rt_native_term_runtime rt_foreign_entry_callee_saved"`,
`RE="rt_(debug_arena|debug_report|debug_soak|io_key_readers|native_io_runtime|native_term_runtime|foreign_entry_callee_saved)"`.

- [ ] K1 → 120 tests.
- [ ] `rt_debug_arena.rs` → root + `registration.rs`, `counters.rs`, `series.rs` (§4.31).
- [ ] `rt_debug_report.rs` → root + `exit_paths.rs`, `codegen.rs`, `measurements.rs` (§4.32).
- [ ] `rt_debug_soak.rs` → root (keeps `FLAT_BOUND`, `BLOCK_BOUND`, cited by `.ai/testing-gates.md:865`) + ten topics (§4.33).
- [ ] `rt_io_key_readers.rs` → root + `pipe.rs`, `timed.rs`, `terminal.rs`, `kitty.rs` (§4.34).
- [ ] `rt_native_io_runtime.rs` → root (keeps its `#[cfg(unix)] use std::…` lines) + `stdin_and_flush.rs`, `resource_cleanup.rs`, `terminal.rs`, `failures.rs` (§4.35).
- [ ] `rt_native_term_runtime.rs` → root + six topics (§4.36).
- [ ] `rt_foreign_entry_callee_saved.rs` → root keeps the six tests + support `fixtures.rs`, `audit.rs`, `escape.rs` (§4.37).
- [ ] Prose-reference check: `grep -n 'BLOCK_BOUND' tests/runtime/rt_debug_soak.rs` → the const is still declared in the root.
- [ ] Tighten: K5 and commit the ceiling file.

Acceptance: 120 tests are unchanged and pass, and every `.ai`/spec reference still names an
existing path and symbol.
  Check: K2 → no output; K3 → 7 × ok (ignored soak tests stay ignored; they are listed and
  compared by K1/K2); K4 → no output; K5 → 7 TIGHTEN rows, then none;
  `ls tests/runtime/rt_debug_soak.rs tests/runtime/rt_foreign_entry_callee_saved.rs tests/runtime/rt_native_io_runtime.rs`
  → all present (est. 20 min: rt_debug_soak builds the browser package and runs 49 tests).
Commit: —

### Phase 6 — rt_inplace_self_update (§4.38)

The largest split (11 new files, 36 `pub(super)` items) and the one `include_str!` repoint.
`T="rt_inplace_self_update"`, `RE="rt_(inplace_self_update)"`.

- [ ] K1 → 4 tests (two of them `#[ignore]`d grid tests; `--list` includes them).
- [ ] `rt_inplace_self_update.rs` → root (keeps `DEFAULT_N`, `EXEMPT_M`, `VALUE_M` and the three
      `include_str!("inplace_self_update/*.tsv")` consts, whose relative paths stay valid) + nine
      support files, `scratch_arms.rs` and `grids.rs`, with the `pub(super)` surface and the nine
      root globs listed in §4.38.
- [ ] In `tests/runtime/rt_inplace_self_update/scratch_arms.rs`, change
      `include_str!("../../src/codegen/collection/assign/self_update.rs")` to
      `include_str!("../../../src/codegen/collection/assign/self_update.rs")`.
- [ ] Run the ignored grid tests once, because CI runs them (`coverage.yml:180`):
      `cargo test --release --test rt_inplace_self_update -- --ignored` → ok.
- [ ] Tighten: K5 and commit the ceiling file.

Acceptance: 4 tests are unchanged, the plain and ignored runs pass, and the compiler-source include
resolves.
  Check: K2 → no output; K3 → ok (`scratch_arms::scratch_arms_match_the_compiler` passes); the
  `--ignored` run → ok; K4 → no output; K5 → 1 TIGHTEN row, then none (est. 25 min: the ignored
  grids build every case program, and this run is the only check of the 36 moved support items'
  run-time behavior).
Commit: —

## Validation Plan

- Tests: no test is added. Every moved test runs in its phase's K3, and the ignored ones are
  compared by K1/K2 (and the self-update grids run in Phase 6).
- Coverage check: `tests/` is outside the coverage denominator by design
  (`scripts/coverage-common.sh` IGNORE `(^|/)tests/`), and no compiler code changes, so coverage
  cannot move.
- Runtime proof: K3 per phase is the end-to-end proof. Each target compiles and runs its MFBASIC
  programs exactly as before.
- Doc sync: none required. No spec, man or `.ai` reference changes meaning, because roots,
  target names and the cited `BLOCK_BOUND` stay put (§4.0). No `mfb spec` contract changes.
- Final gate (once, after Phase 6):
  `cargo test --test file_size_budget 2>&1 | grep -E 'tests/runtime/rt_(big_int|callback_failure_frees|closure_single_owner|debug_arena|debug_report|debug_soak|domain_error_frees_locals|error_model_recovery|foreign_entry_callee_saved|foreign_type_reexport|global_argument_reassigned_by_callee|hof_transient_bytes|imported_package_global|imported_type_name_collision|imported_union_enum_members|inline_trap_[a-z_]+|inplace_failure_atomic|inplace_self_update|io_key_readers|json_bounds|list_append_growth_bounds|list_element_record_borrow|list_take_drop_in_place|map_integer_key_probe_cost|math_fixed_trig_accuracy|math_float_trig_large_angles|moved_handle_guards|native_io_runtime|native_term_runtime|nothing_collections|overloaded_function_values|owned_argument|package_dependency_closure)(\.rs|/)'`
  → no output. The same regex over `tests/guards/file_size_ceilings.txt` → exactly the two
  `exempt` lines (`rt_big_int/powers.rs`, `rt_closure_single_owner/cases.rs`). `cargo test --test test_targets_registered` → ok (est. 3 min). The full
  workspace suite runs once, in plan-176-V's Validation Plan.

## Decisions

- Rule: code ≤ 1000 and test ≤ 250 non-comment, non-blank LOC, over `.rs`/`.mfb` under `src/`,
  `packages/`, `examples/`, `repository/` and `tests/` — owner, 2026-10-14.
- Inline test code counts against the 250 test budget; generated files are exempt; the guard
  warns with a banner and never fails; a new module is always an acceptable split — owner, 2026-10-14.
- A single test-side unit over 250 (an embedded MFBASIC program, a const table, a single `#[test]`
  fn) gets its own file and an `exempt` row with its reason, in both the ceiling file and
  `census.py`'s `EXEMPT`, in the same commit — owner, 2026-10-14. Natural extractions come first.
- B's exempt files are `tests/runtime/rt_big_int/powers.rs` and
  `tests/runtime/rt_closure_single_owner/cases.rs` — author, 2026-10-14. For powers.rs, an `.mfb`
  extraction leaves a 1,200-plus-LOC test file. cases.rs is one table.
- `tests/runtime/rt_big_int/word_boundaries.rs` is NOT exempt — parent decision, 2026-10-17
  (natural extractions come first). Its inline program moves to `word_boundaries.mfb` via
  `include_str!`, giving 141 + 143 LOC.
- Integration-test splits keep the root as the `[[test]]` target and use `#[path]` children —
  plan-176-A (verified there) and `.ai/build-tooling.md`.
- Data-only support files omit `use super::*;` because an unused glob warns (probe,
  §2) — author, 2026-10-14.
- Glob-cycle imports (`use super::*;` in parts, `use part::*;` in the root) compile without warnings
  (`/tmp/loc/probe2` printed `6`, no warning) — verified, 2026-10-14.
- rt_owned_argument's self-reading guard and the seven tests it scans stay in the root — author,
  2026-10-14, because moving them would let the guard pass while seeing nothing.

## Corrections

## Summary

The risk is a test that silently stops compiling or running. The before/after name list per
target (K1/K2) and the one run per phase (K3) close that. rt_owned_argument's self-scan and
rt_inplace_self_update's compiler-source include are the two path-sensitive spots, and §4 pins both.
The word_boundaries extraction is the one place a string value could change, and a byte `cmp`
closes it. Nothing outside `tests/runtime/`, the ceiling file and `census.py`'s `EXEMPT` table
changes.
