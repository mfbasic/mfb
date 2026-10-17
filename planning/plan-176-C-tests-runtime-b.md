# plan-176-C: Split tests/runtime, part 2 (32 integration-test files)

Last updated: 2026-10-14
Effort: large (3h–1d)
Depends on: plan-176-B

This sub-plan splits the remaining 32 over-budget integration-test files under `tests/runtime/`
(`rt_package_parameter_defaults.rs` through `rt_top_level_initializer_globals.rs`, plus
`rt_scope_drop_leaks.rs`; the list is `/tmp/loc/groups.json["C"]`). Together they hold 20,249 test
LOC and 457 `#[test]` fns. The method is the one B used. Each root stays the `[[test]]` target and
gains topic and support modules under `tests/runtime/<name>/`, declared with `#[path]`. It is pure
code motion. The only text edits are `pub(super)` on support items and the 23 `include_str!` paths
in rt_resource_model, which gain a `../` because their file moves one directory deeper. C also fixes
one pre-existing stale `.ai` path that it touches.

**Single outcome:** after C lands, `cargo test --test file_size_budget` prints no row for any path
under `tests/runtime/` that this letter owns, and `tests/guards/file_size_ceilings.txt` has no
`ceiling` row for them. The only rows left are the two `exempt` rows C adds:
`tests/runtime/rt_resource_last_holder/map_iteration.rs` and
`tests/runtime/rt_self_update_cost/value_program.rs`.

References:

- `planning/plan-176-A-file-size-guard.md`: the rule, the guard, the ceiling format, the tighten
  ratchet, and the `#[path]` layout verified in its §2.
- `planning/plan-176-B-tests-runtime-a.md`: the same conventions and gate kit (K1–K5) on the
  first half of `tests/runtime/`. C restates them here so it can be executed on its own.
- `.ai/build-tooling.md` § "Splitting an over-budget file (plan-176)".
- `.ai/testing-gates.md` (:22 names `tests/runtime/rt_res_state_inplace_mutation.rs`).
- `.ai/collections.md` (:47, :75 name `rt_self_update_cost`) and `.ai/codegen-invariants.md` (:559
  names `rt_regex_bounds`). These are root paths or targets and stay valid.
- `.ai/resources-packages.md` (:49, :176): stale path, fixed in Phase 3 (§4.19).
- Spec topics: none. No `[[tests/runtime/<file>.rs:…]]` citation names a C file
  (`grep -rnoE '\[\[tests/runtime/[a-z_0-9]+\.rs[^]]*\]\]' src/docs .ai` → 0 hits).

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-176-B is complete and archived | `ls planning/plan-176-B-* planning/completed/plan-176-B-*` → only `planning/completed/plan-176-B-tests-runtime-a.md` | NOT MET (2026-10-14) |
| The guard and the oracle exist (plan-176-A) | `cargo test --test file_size_budget` → passes; `python3 -I tools/loc-census/census.py --file tests/runtime/rt_scope_drop_leaks.rs` → `0 5476` | NOT MET (2026-10-14; lands with A) |
| This letter's files have no uncommitted changes | `git status --porcelain -- tests/runtime .ai/resources-packages.md .ai/testing-gates.md tests/guards/file_size_ceilings.txt tools/loc-census/census.py` → empty | MET (2026-10-14: empty output) |

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
  `tools/loc-census/census.py --file`. The two exceptions are the exempt files named above, each
  with an `exempt` row in both `tests/guards/file_size_ceilings.txt` and `census.py`'s `EXEMPT`.
- For every one of the 32 targets, the sorted list of test names from `-- --list`, with module
  prefixes stripped, is identical before and after. Every target passes `cargo test --test <t>`.
- `.ai/resources-packages.md` names the real path of `rt_res_state_inplace_mutation.rs`.

### Non-goals (explicit constraints)

- No behavior change: no test body, assertion, MFBASIC program text, expected output, `#[cfg]` or
  `#[ignore]` changes. Text edits inside moved code are limited to `pub(super)` (§4 "Surface")
  and the 23 `include_str!` repoints (§4.23).
- No `[[test]]` target added, renamed or re-pathed. `Cargo.toml` is not edited.
- No test fn renamed. Only a module prefix is added to test paths, and substring filters keep working.
- No helper is promoted into `tests/common/`, and no test moves between targets.

## 2. Current State

### Files and measured sizes

Test LOC are from the planning oracle on the working tree of `17a9b5799`
(`python3 -I /tmp/loc/census.py "$PWD"`; after A it is
`python3 -I tools/loc-census/census.py --file <path>`). Test counts are
`grep -cE '#\[test\]' <file>`; no file uses another test attribute.

| File | Test LOC | Tests | New files | Largest part after |
|---|---|---|---|---|
| `tests/runtime/rt_package_parameter_defaults.rs` | 323 | 7 | 2 | 140 |
| `tests/runtime/rt_package_private_type_collision.rs` | 291 | 9 | 2 | 140 |
| `tests/runtime/rt_package_public_export_name_collision.rs` | 307 | 6 | 2 | 116 |
| `tests/runtime/rt_parameter_default_scope.rs` | 302 | 9 | 2 | 125 |
| `tests/runtime/rt_place_forward.rs` | 437 | 19 | 2 | 210 |
| `tests/runtime/rt_process_spawn_ambient_fds.rs` | 396 | 5 | 2 | 177 |
| `tests/runtime/rt_record_nested_update.rs` | 489 | 4 | 3 | 174 |
| `tests/runtime/rt_record_update_forms.rs` | 266 | 1 | 1 | 153 |
| `tests/runtime/rt_recursive_value_collection_drops.rs` | 272 | 9 | 2 | 113 |
| `tests/runtime/rt_recursive_value_construction_copies.rs` | 290 | 4 | 2 | 153 |
| `tests/runtime/rt_recursive_value_copies.rs` | 307 | 3 | 3 | 135 |
| `tests/runtime/rt_recursive_value_drop_symmetry.rs` | 302 | 6 | 2 | 107 |
| `tests/runtime/rt_regex_bounds.rs` | 416 | 5 | 4 | 134 |
| `tests/runtime/rt_regex_span.rs` | 447 | 3 | 3 | 188 |
| `tests/runtime/rt_res_list_from_call_leak.rs` | 343 | 4 | 3 | 160 |
| `tests/runtime/rt_res_state_inplace_mutation.rs` | 651 | 24 | 4 | 174 |
| `tests/runtime/rt_resource_aggregate_temporary.rs` | 616 | 22 | 3 | 189 |
| `tests/runtime/rt_resource_drop_reclaims.rs` | 741 | 20 | 4 | 202 |
| `tests/runtime/rt_resource_last_holder.rs` | 1476 | 14 | 7 | 285 (exempt) |
| `tests/runtime/rt_resource_model.rs` | 292 | 23 | 3 | 91 |
| `tests/runtime/rt_resource_review_805.rs` | 574 | 18 | 4 | 137 |
| `tests/runtime/rt_resource_review_806.rs` | 532 | 17 | 3 | 186 |
| `tests/runtime/rt_resource_store_float.rs` | 499 | 8 | 2 | 204 |
| `tests/runtime/rt_resource_trap_state_free.rs` | 259 | 4 | 2 | 132 |
| `tests/runtime/rt_resource_union_box_owner.rs` | 629 | 16 | 3 | 191 |
| `tests/runtime/rt_resource_union_member_float.rs` | 416 | 6 | 2 | 156 |
| `tests/runtime/rt_resource_union_reclaim.rs` | 561 | 10 | 3 | 182 |
| `tests/runtime/rt_scope_drop_leaks.rs` | 5476 | 154 | 35 | 228 |
| `tests/runtime/rt_self_update_cost.rs` | 1337 | 13 | 7 | 443 (exempt) |
| `tests/runtime/rt_sortby_copying_native.rs` | 385 | 4 | 3 | 113 |
| `tests/runtime/rt_thread_block_arena.rs` | 366 | 5 | 2 | 194 |
| `tests/runtime/rt_top_level_initializer_globals.rs` | 251 | 5 | 2 | 106 |
| **Total (32 files)** | **20,249** | **457** | **125** | |

Part sizes come from `python3 -I /tmp/loc/span.py <file> <a> <b>` summed over each part's ranges,
plus the lines the split adds (§3).

### Verified properties

- **Layout, globs and warnings.** These are the same facts as plan-176-B §2, from the same probes.
  The `#[path]` children list as `<topic>::<test>` (plan-176-A probe). Root imports, `common`
  helpers and support globs used only by a topic produce no warning (`/tmp/loc_bcd_probe`:
  `test result: ok. 1 passed`, no warning). Glob cycles are clean (`/tmp/loc/probe2` printed `6`).
  `use super::*;` in a file that references no parent item warns
  (`warning: unused import: super::*`).
- **Seven C support files reference no parent item** and so omit `use super::*;`:
  `rt_record_nested_update/cases.rs`, `rt_record_update_forms/cases.rs`, `rt_regex_bounds/corpus.rs`,
  `rt_regex_span/crosscheck_corpus.rs`, `rt_self_update_cost/scale_program.rs`,
  `rt_self_update_cost/value_program.rs`, `rt_sortby_copying_native/types.rs`. Measured with
  `python3 -I /tmp/loc_bcd_probe/s/idents.py <file> <ranges>`: every identifier outside string
  literals is the item's own name (each item is one string or string-tuple const).
- **Every C test compiles on the macOS development host.**
  `grep -lE 'cfg\((target_os = "(linux|windows)"|windows|not\(unix\))' <the 32 files>` → no file.
  rt_scope_drop_leaks' 23 `#[cfg(any(target_os = "macos", target_os = "linux"))]` gates include
  macOS. So `-- --list` on the development host counts every test, and no remote box is needed.
- **rt_resource_model includes 23 bug repros by relative path.** `grep -c include_str tests/runtime/rt_resource_model.rs` → `23`.
  22 are on one line, and one (`bug_1078_…`, line 292) spans three lines. All are
  `"../../bugs/repro/bug-NNNN-….mfb"`, resolved relative to the including file.
- **No C file is self-reading, `--exact`-run, in `scripts/coverage-exceptions.txt`, or in a
  `tests/guards` allowlist** (`grep -rn -- --exact .github scripts`; `grep -n <name>
  scripts/coverage-exceptions.txt tests/guards/{architecture_guards,no_type_strings,no_operator_strings,inplace_self_update_census}.rs`
  → no hit). `tests/guards/test_targets_registered.rs` reads only direct `.rs` children of each
  `tests/<dir>/` (plan-176-A §2). No `tests/runtime/<name>/` directory exists for any C stem.
- **The `.ai/resources-packages.md` path is stale today.** Lines 49 and 176 name
  `tests/rt_res_state_inplace_mutation.rs`, and `ls tests/rt_res_state_inplace_mutation.rs` →
  `No such file or directory`. The file is `tests/runtime/rt_res_state_inplace_mutation.rs`.

## 3. Design Overview

**Conventions (every file)** are plan-176-B §3's conventions:

- The root stays the `[[test]]` target. It keeps its doc, `#![cfg]`,
  `#[path = "../common/mod.rs"] mod common;`, its `use` lines and the shared helpers listed in §4.
  It gains one `#[path = "<name>/<m>.rs"] mod <m>;` pair per new file (2 LOC).
- Topic files start with `use super::*;` and need no visibility edits. Support files get
  `pub(super)` on the items, fields and methods named under "Surface", and the root adds one
  `use <m>::*;` per support file. A support file starts with `use super::*;` only when it references
  a parent item. The seven files marked "no `use super::*;`" omit it (§2).
- Code moves verbatim, with doc comments, `#[cfg]` and `#[ignore]` attributes, in source order.
  Ranges are as of `17a9b5799`, and item and test names are authoritative if lines drift.
- Exempt rows go by hand into both `tests/guards/file_size_ceilings.txt`
  (`exempt test <path> # <reason>`) and `census.py`'s `EXEMPT`, in the same commit.

**Gate class: test-motion.** The gate kit is plan-176-B's, with `/tmp/176C-` file prefixes:

- **K1 list before**:
  `for t in $T; do cargo test -q --test $t -- --list 2>/dev/null | grep ': test$' | sed -E 's/^([a-z0-9_]+::)+//' | sort > /tmp/176C-$t.list; done; cat $(for t in $T; do echo /tmp/176C-$t.list; done) | wc -l`
  → the phase's test total.
- **K2 list after**:
  `for t in $T; do cargo test -q --test $t -- --list 2>/dev/null | grep ': test$' | sed -E 's/^([a-z0-9_]+::)+//' | sort | diff -q /tmp/176C-$t.list - >/dev/null || echo "DIFF $t"; done`
  → no output.
- **K3 run**: `cargo test $(for t in $T; do printf -- '--test %s ' $t; done)` → every target `test result: ok`, 0 failed.
- **K4 warnings**: `cargo check --all-targets 2>&1 | grep -E -- "--> tests/runtime/$RE(\.rs|/)"` → no output.
- **K5 budget**: `cargo test --test file_size_budget 2>&1 | grep -E "tests/runtime/$RE(\.rs|/)"` → only
  `TIGHTEN` rows (one per root), with no NEW/GREW/STALE row. Then `MFB_LOC_TIGHTEN=1 cargo test --test file_size_budget`.
  Then `git diff -U0 tests/guards/file_size_ceilings.txt | grep -E '^[-+](ceiling|exempt)' | grep -vE "tests/runtime/$RE(\.rs|/)"`
  → no output. Re-running the first grep → no output.

**Where the risk concentrates.** rt_scope_drop_leaks is the riskiest file. It has 154 tests in 35
topics, so a missed `#[path]` silently drops a whole topic. It goes last, alone, behind K1/K2.
rt_resource_model's 23 include paths fail at compile time if they are wrong, so they cannot pass
silently. Many rt_scope_drop_leaks tests and helpers carry `#[cfg(unix)]` or
`#[cfg(any(target_os = …))]` attributes. Those attributes move with their items, and the cfg-gated
helpers that stay in the root (`peak_rss`, `assert_flat`, `condition_temp_report`, …) keep theirs.

**Rejected alternatives.** These are the same as plan-176-B §3: no helper promotion into
`tests/common/` (letter E owns it), no per-topic `[[test]]` targets (that renames targets used by
docs and CI), the root does not move into its directory (`test_targets_registered.rs` requires root
= target name), and no logic edit to split the two exempt units (§Decisions).

## 4. Detailed Design

### 4.0 Sweeps common to every file

- Spec citations to C files: 0 (§References). No cited path moves, so `spec_citations_resolve`
  is unaffected.
- `Cargo.toml` stanzas and comments (`:69` names `tests/runtime/rt_regex_bounds.rs`), and the
  `.ai/` prose references, name root paths or targets and stay valid. The single stale reference
  (`.ai/resources-packages.md:49,176`) is fixed in Phase 3.
- Each entry gives the layout (path, LOC, contents, source ranges as of `17a9b5799`), then
  "Surface", then "Sweeps".

#### 4.1 `tests/runtime/rt_package_parameter_defaults.rs` — 323 test LOC, 7 tests

- `tests/runtime/rt_package_parameter_defaults.rs` (120): header; helpers Package, package_entries, build_app, run_app, build, lines [lines 18-25, 46-69, 71-115, 117-134, 136-155, 157-159]; mods computed_defaults, literal_defaults
- `tests/runtime/rt_package_parameter_defaults/computed_defaults.rs` (140): tests (4) a_computed_package_default_reads_a_private_global_on_each_call, a_package_default_calling_another_packages_export_is_filled, a_link_wrapper_package_fills_both_defaults, a_package_default_naming_a_parameter_is_refused_at_package_build; items LIBC_LIBRARIES [lines 27-30, 198-232, 259-297, 299-338, 340-372]
- `tests/runtime/rt_package_parameter_defaults/literal_defaults.rs` (69): tests (3) an_omitted_literal_integer_package_default_is_passed, an_omitted_literal_string_package_default_is_passed, a_named_package_call_fills_a_skipped_middle_default; items DEFLIB [lines 32-44, 161-178, 180-196, 234-257]
Surface: none (no support file; root keeps every shared helper).
Sweeps: none (§4.0 commands: 0 hits).

#### 4.2 `tests/runtime/rt_package_private_type_collision.rs` — 291 test LOC, 9 tests

- `tests/runtime/rt_package_private_type_collision.rs` (102): header; helpers Form, combined, project, run [lines 112-119, 121-127, 129-183, 185-214]; mods private_types, export_checks
- `tests/runtime/rt_package_private_type_collision/private_types.rs` (140): tests (6) a_private_flat_package_type_and_a_flat_program_type_are_distinct, a_private_flat_mfp_type_and_a_flat_program_type_are_distinct, a_private_recursive_package_type_and_a_flat_program_type_are_distinct, a_private_recursive_mfp_type_and_a_flat_program_type_are_distinct, a_private_flat_package_type_and_a_recursive_program_type_are_distinct, a_private_flat_mfp_type_and_a_recursive_program_type_are_distinct; items FLAT_PACKAGE, RECURSIVE_PACKAGE, FLAT_PROGRAM, RECURSIVE_PROGRAM [lines 25-43, 45-72, 74-87, 89-110, 216-222, 224-230, 232-243, 245-256, 258-269, 271-282]
- `tests/runtime/rt_package_private_type_collision/export_checks.rs` (55): tests (3) an_exported_type_naming_a_non_exported_type_is_rejected_at_the_package_build, an_exported_signature_naming_a_non_exported_type_is_rejected_at_the_package_build, an_exported_type_naming_an_exported_type_still_builds_and_imports; items build_package [lines 284-303, 305-319, 321-332, 334-347]
Surface: none (no support file; root keeps every shared helper).
Sweeps: none (§4.0 commands: 0 hits).

#### 4.3 `tests/runtime/rt_package_public_export_name_collision.rs` — 307 test LOC, 6 tests

- `tests/runtime/rt_package_public_export_name_collision.rs` (116): header; helpers LIB, PUBLIC_HELPER, RETURN_HELPER, Form, combined, write_package, build_package, build_importer [lines 24-32, 34-37, 69-71, 81-88, 90-96, 98-111, 113-131, 133-185]; mods overloads, importer_view
- `tests/runtime/rt_package_public_export_name_collision/overloads.rs` (111): tests (4) an_export_overloaded_by_a_public_sibling_is_callable_from_source, an_export_overloaded_by_a_public_sibling_is_callable_from_mfp, an_export_with_a_public_return_type_sibling_is_callable, two_export_overloads_still_resolve_by_argument_types; items EXPORT_HELPER, PROGRAM, EXPECTED, RETURN_LIB, RETURN_PROGRAM, run [lines 39-42, 44-53, 55-55, 57-67, 73-79, 187-202, 204-215, 217-228, 230-246, 248-272]
- `tests/runtime/rt_package_public_export_name_collision/importer_view.rs` (86): tests (2) calling_the_public_overload_from_an_importer_is_a_located_error, a_public_sibling_does_not_change_the_package_exports; items exports_of [lines 274-312, 314-332, 334-366]
Surface: none (no support file; root keeps every shared helper).
Sweeps: none (§4.0 commands: 0 hits).

#### 4.4 `tests/runtime/rt_parameter_default_scope.rs` — 302 test LOC, 9 tests

- `tests/runtime/rt_parameter_default_scope.rs` (71): header; helpers build_program, run_program, build, lines [lines 22-46, 48-65, 67-86, 88-90]; mods evaluation, scope
- `tests/runtime/rt_parameter_default_scope/evaluation.rs` (125): tests (5) a_generic_function_default_is_filled_per_instantiation, a_link_function_fills_an_omitted_default, a_default_reading_a_mut_global_sees_its_value_on_each_call, a_default_calling_a_function_runs_on_every_call, a_literal_default_is_unchanged; items LIBC_LIBRARIES [lines 17-20, 219-248, 250-277, 279-301, 303-328, 330-349]
- `tests/runtime/rt_parameter_default_scope/scope.rs` (112): tests (4) a_default_calling_a_function_ignores_a_callers_same_named_local, a_caller_local_named_like_a_defaults_global_is_refused, a_named_call_fills_an_omitted_default_in_declaration_scope, a_sub_default_ignores_a_callers_same_named_local [lines 92-123, 125-159, 161-188, 190-217]
Surface: none (no support file; root keeps every shared helper).
Sweeps: none (§4.0 commands: 0 hits).

#### 4.5 `tests/runtime/rt_top_level_initializer_globals.rs` — 251 test LOC, 5 tests

- `tests/runtime/rt_top_level_initializer_globals.rs` (106): header; helpers Package, package_entries, run_app, build, lines [lines 27-32, 46-69, 71-122, 124-141, 143-145]; mods top_level, package_initializers
- `tests/runtime/rt_top_level_initializer_globals/top_level.rs` (87): tests (3) a_top_level_initializer_reads_the_programs_own_global, a_top_level_initializer_reads_an_imported_package_global, a_top_level_initializer_calling_a_package_sees_its_initialized_globals; items LIMITS [lines 34-44, 147-172, 174-205, 231-254]
- `tests/runtime/rt_top_level_initializer_globals/package_initializers.rs` (64): tests (2) a_package_initializer_reads_the_packages_own_global, a_package_initializer_calling_another_package_sees_its_initialized_globals [lines 207-229, 256-301]
Surface: none (no support file; root keeps every shared helper).
Sweeps: none (§4.0 commands: 0 hits).

#### 4.6 `tests/runtime/rt_place_forward.rs` — 437 test LOC, 19 tests

- `tests/runtime/rt_place_forward.rs` (71): header; helpers build_and_run, assert_same_output, PRELUDE [lines 21-50, 52-61, 63-86]; mods locals, globals_and_state
- `tests/runtime/rt_place_forward/locals.rs` (210): tests (11) the_benchmark_set_shape, the_remove_at_shape, a_nested_path, a_bare_local, a_read_only_window_to_the_block_end, a_read_in_a_nested_for, sibling_reads_in_the_owner_write, w1_an_owner_write_before_a_read_keeps_the_old_value, w2_an_owner_write_nested_in_the_last_op_keeps_the_old_value, a_name_reused_after_the_loop, a_forward_that_makes_the_operand_read_itself [lines 88-105, 107-121, 123-137, 139-155, 157-171, 173-191, 193-213, 215-231, 233-251, 431-457, 459-497]
- `tests/runtime/rt_place_forward/globals_and_state.rs` (162): tests (8) the_global_set_shape, the_global_remove_at_shape, a_bare_global, the_state_set_and_remove_at_shapes, the_state_remove_key_and_remove_shapes, w4_a_user_sub_writing_the_global_keeps_the_old_value, w4_a_user_call_inside_the_owner_write_keeps_the_old_value, w5_passing_the_resource_keeps_the_old_value; items owner_program [lines 255-258, 260-278, 280-295, 297-311, 313-335, 337-360, 362-383, 385-406, 408-429]
Surface: none (no support file; root keeps every shared helper).
Sweeps: none (§4.0 commands: 0 hits).

#### 4.7 `tests/runtime/rt_process_spawn_ambient_fds.rs` — 396 test LOC, 5 tests

- `tests/runtime/rt_process_spawn_ambient_fds.rs` (97): header; helpers build_probe, scratch [lines 50-138, 140-145]; mods inheritance, signals_and_timeouts
- `tests/runtime/rt_process_spawn_ambient_fds/inheritance.rs` (177): tests (2) spawned_child_sees_no_ambient_inherited_fd, spawned_child_still_gets_exactly_its_stdio; items run_with_ambient_fds, SpawnForm, impl SpawnForm, probe_parent_source, assert_child_got_its_stdio, leaked_line [lines 147-212, 214-223, 225-245, 247-273, 275-288, 290-295, 297-329, 331-358]
- `tests/runtime/rt_process_spawn_ambient_fds/signals_and_timeouts.rs` (128): tests (3) spawned_child_gets_default_signal_dispositions, spawned_child_dies_on_a_closed_pipe, a_timed_out_run_leaves_no_descendant_behind [lines 360-401, 403-443, 445-514]
Surface: none (no support file; root keeps every shared helper).
Sweeps: none (§4.0 commands: 0 hits).

#### 4.8 `tests/runtime/rt_record_nested_update.rs` — 489 test LOC, 4 tests

- `tests/runtime/rt_record_nested_update.rs` (174): header; helpers REPS, PRELUDE, run [lines 34-34, 36-148, 295-345]; mods cases, allocation, rebinding
- `tests/runtime/rt_record_nested_update/cases.rs` (support) (106; no `use super::*;`, see §3): items CASES [lines 150-272]
- `tests/runtime/rt_record_nested_update/allocation.rs` (106): tests (2) a_nested_record_update_allocates_only_what_its_size_change_needs, a_later_failure_leaves_the_element_unchanged; items program [lines 274-293, 347-394, 396-435]
- `tests/runtime/rt_record_nested_update/rebinding.rs` (112): tests (2) a_rebound_get_binding_keeps_its_element_update, an_unsafe_rebinding_beside_a_borrowing_one_reads_its_own_copy; items rebound_program [lines 437-478, 480-506, 508-557]
Surface: pub(super) on 1 item(s): CASES; root adds `use cases::*;`.
Sweeps: none (§4.0 commands: 0 hits).

#### 4.9 `tests/runtime/rt_record_update_forms.rs` — 266 test LOC, 1 tests

- `tests/runtime/rt_record_update_forms.rs` (116): header; helpers REPS, program, run; tests (1) a_record_update_allocates_nothing_per_update [lines 24-24, 183-196, 198-248, 250-297]; mods cases
- `tests/runtime/rt_record_update_forms/cases.rs` (support) (153; no `use super::*;`, see §3): items PRELUDE, CASES [lines 26-106, 108-181]
Surface: pub(super) on 2 item(s): PRELUDE, CASES; root adds `use cases::*;`.
Sweeps: none (§4.0 commands: 0 hits).

#### 4.10 `tests/runtime/rt_recursive_value_collection_drops.rs` — 272 test LOC, 9 tests

- `tests/runtime/rt_recursive_value_collection_drops.rs` (113): header; helpers run_debug, assert_discards_free_the_element, PRELUDE, program [lines 24-69, 71-95, 97-133, 135-137]; mods lists, maps_and_types
- `tests/runtime/rt_recursive_value_collection_drops/lists.rs` (93): tests (5) an_in_place_list_set_frees_the_overwritten_element, a_list_remove_at_frees_the_removed_element, a_record_field_list_set_frees_the_overwritten_element, a_record_field_list_remove_at_frees_the_removed_element, a_fetched_recursive_element_survives_an_in_place_remove_and_a_growing_append [lines 139-155, 157-174, 213-229, 231-248, 269-295]
- `tests/runtime/rt_recursive_value_collection_drops/maps_and_types.rs` (72): tests (4) a_map_set_over_an_existing_key_frees_the_old_value, a_map_remove_key_frees_the_removed_value, a_record_field_map_remove_key_frees_the_removed_value, a_collection_of_a_type_that_only_reaches_a_cycle_is_freed [lines 176-192, 194-211, 250-267, 297-317]
Surface: none (no support file; root keeps every shared helper).
Sweeps: none (§4.0 commands: 0 hits).

#### 4.11 `tests/runtime/rt_recursive_value_construction_copies.rs` — 290 test LOC, 4 tests

- `tests/runtime/rt_recursive_value_construction_copies.rs` (7): header; mods construction, copy_counts
- `tests/runtime/rt_recursive_value_construction_copies/construction.rs` (153): tests (2) a_recursive_value_built_into_another_is_independent_of_its_source, a_value_built_from_the_list_it_is_appended_to_holds_the_old_list; items CONSTRUCTION_SOURCE, SELF_REFERENCE_SOURCE, run [lines 31-106, 108-129, 131-145, 147-173, 331-344]
- `tests/runtime/rt_recursive_value_construction_copies/copy_counts.rs` (136): tests (2) a_construction_store_copies_an_aliased_source_once_and_a_fresh_value_never, a_native_collection_builtin_copies_its_result_elements_graphs; items COPY_COUNT_SOURCE, count_calls, BUILTIN_COPIES_SOURCE [lines 175-197, 199-221, 223-252, 254-300, 302-329]
Surface: none (no support file; root keeps every shared helper).
Sweeps: none (§4.0 commands: 0 hits).

#### 4.12 `tests/runtime/rt_recursive_value_copies.rs` — 307 test LOC, 3 tests

- `tests/runtime/rt_recursive_value_copies.rs` (9): header; mods copies, moves, churn
- `tests/runtime/rt_recursive_value_copies/copies.rs` (109): tests (1) a_recursive_value_copy_is_independent_of_its_source; items COPIES_SOURCE [lines 41-117, 119-152]
- `tests/runtime/rt_recursive_value_copies/moves.rs` (63): tests (1) a_moved_recursive_value_prints_what_it_printed_before; items MOVES_SOURCE [lines 154-198, 200-223]
- `tests/runtime/rt_recursive_value_copies/churn.rs` (135): tests (1) every_recursive_store_shape_frees_once_under_churn; items STORE_SHAPE_CHURN_SOURCE [lines 225-321, 323-376]
Surface: none (no support file; root keeps every shared helper).
Sweeps: none (§4.0 commands: 0 hits).

#### 4.13 `tests/runtime/rt_recursive_value_drop_symmetry.rs` — 302 test LOC, 6 tests

- `tests/runtime/rt_recursive_value_drop_symmetry.rs` (107): header; helpers HOOK_ENV, build, run, counter, Pair, assert_copy_then_drop_is_exact [lines 27-27, 29-61, 63-82, 84-92, 94-98, 100-141]; mods user_types, builtin_graphs
- `tests/runtime/rt_recursive_value_drop_symmetry/user_types.rs` (98): tests (2) a_user_recursive_record_drops_exactly_its_copy, a_user_recursive_union_drops_exactly_its_copy; items USER_NODE_SOURCE, USER_UNION_SOURCE [lines 143-174, 176-180, 182-234, 236-245]
- `tests/runtime/rt_recursive_value_drop_symmetry/builtin_graphs.rs` (103): tests (4) a_json_value_drops_exactly_its_copy, the_regex_matcher_graphs_drop_exactly_their_copies, a_million_deep_chain_drops_exactly_its_copy, churn_frees_every_copy_as_it_goes; items JSON_SOURCE, REGEX_SOURCE, DEEP_CHAIN_SOURCE, CHURN_SOURCE [lines 247-261, 263-267, 269-282, 284-295, 297-314, 316-321, 323-350, 352-367]
Surface: none (no support file; root keeps every shared helper).
Sweeps: none (§4.0 commands: 0 hits).

#### 4.14 `tests/runtime/rt_regex_bounds.rs` — 416 test LOC, 5 tests

- `tests/runtime/rt_regex_bounds.rs` (24): header; helpers run [lines 26-38]; mods group_repeats, cost, corpus, semantics
- `tests/runtime/rt_regex_bounds/group_repeats.rs` (105): tests (3) a_group_repeat_over_a_long_input_matches, a_repeat_that_does_not_match_still_fails_cleanly, find_all_cost_is_bounded_for_the_whole_call; items match_program [lines 40-69, 71-101, 103-114, 116-163]
- `tests/runtime/rt_regex_bounds/cost.rs` (49): tests (1) a_large_subject_costs_a_bounded_multiple_of_its_size; items LARGE_SUBJECT [lines 165-191, 193-222]
- `tests/runtime/rt_regex_bounds/corpus.rs` (support) (134; no `use super::*;`, see §3): items MATCHER_CORPUS [lines 224-360]
- `tests/runtime/rt_regex_bounds/semantics.rs` (116): tests (1) matching_semantics_are_unchanged; items MATCHER_CORPUS_EXPECTED [lines 362-453, 455-477]
Surface: pub(super) on 1 item(s): MATCHER_CORPUS; root adds `use corpus::*;`.
Sweeps: Cargo.toml:68 comment and `.ai/codegen-invariants.md:559` name it — unchanged.

#### 4.15 `tests/runtime/rt_regex_span.rs` — 447 test LOC, 3 tests

- `tests/runtime/rt_regex_span.rs` (38): header; helpers run, assert_output [lines 28-40, 42-57]; mods spans, crosscheck_corpus, crosscheck
- `tests/runtime/rt_regex_span/spans.rs` (133): tests (2) spans_text_and_captures_are_reported_for_a_variable_length_pattern, a_program_with_its_own_group_record_still_compiles_alongside_regex; items SPANS, SPANS_EXPECTED, OWN_GROUP_TYPE [lines 59-125, 127-148, 150-165, 167-189, 191-202]
- `tests/runtime/rt_regex_span/crosscheck_corpus.rs` (support) (188; no `use super::*;`, see §3): items CROSSCHECK [lines 204-395]
- `tests/runtime/rt_regex_span/crosscheck.rs` (97): tests (1) the_new_members_agree_with_the_index_only_members_across_the_matcher_corpus; items CROSSCHECK_EXPECTED [lines 397-482, 484-497]
Surface: pub(super) on 1 item(s): CROSSCHECK; root adds `use crosscheck_corpus::*;`.
Sweeps: none (§4.0 commands: 0 hits).

#### 4.16 `tests/runtime/rt_sortby_copying_native.rs` — 385 test LOC, 4 tests

- `tests/runtime/rt_sortby_copying_native.rs` (60): header; helpers run [lines 214-264]; mods cost, types, key_fn
- `tests/runtime/rt_sortby_copying_native/cost.rs` (113): tests (1) a_copying_sort_costs_what_the_self_update_spelling_costs; items N, SLACK, CASES, program [lines 24-24, 25-28, 140-184, 186-212, 266-304]
- `tests/runtime/rt_sortby_copying_native/types.rs` (support) (109; no `use super::*;`, see §3): items TYPES [lines 30-138]
- `tests/runtime/rt_sortby_copying_native/key_fn.rs` (112): tests (3) a_failing_key_fn_in_a_copying_sort_by_frees_everything, a_capturing_key_fn_is_freed_after_the_call, every_sort_by_lowering_calls_key_fn_once_for_one_element [lines 306-341, 343-382, 384-428]
Surface: pub(super) on 1 item(s): TYPES; root adds `use types::*;`.
Sweeps: none (§4.0 commands: 0 hits).

#### 4.17 `tests/runtime/rt_thread_block_arena.rs` — 366 test LOC, 5 tests

- `tests/runtime/rt_thread_block_arena.rs` (8): header; mods round_trips, transfers
- `tests/runtime/rt_thread_block_arena/round_trips.rs` (194): tests (2) seeds_and_results_round_trip_through_the_block_arena, a_long_message_exchange_keeps_the_block_arena_small; items thread_lines, thread_value, block_arenas, assert_block_arenas_unmapped, SHAPES, MESSAGES [lines 19-33, 35-41, 43-53, 55-81, 83-151, 153-181, 183-214, 216-233]
- `tests/runtime/rt_thread_block_arena/transfers.rs` (170): tests (3) an_unaccepted_transfer_is_closed_when_the_thread_is_freed, a_trapped_bind_transfer_frees_the_senders_record, an_inline_trapped_wait_for_of_a_failing_worker_reaches_its_handler; items UNACCEPTED, TRAPPED_TRANSFER, trapped_transfer_live_bytes [lines 235-262, 264-283, 285-319, 321-337, 339-347, 349-424]
Surface: none (no support file; root keeps every shared helper).
Sweeps: none (§4.0 commands: 0 hits).

#### 4.18 `tests/runtime/rt_res_list_from_call_leak.rs` — 343 test LOC, 4 tests

- `tests/runtime/rt_res_list_from_call_leak.rs` (18): header; helpers counter [lines 41-49]; mods take_drop, user_function, native_members
- `tests/runtime/rt_res_list_from_call_leak/take_drop.rs` (74): tests (1) a_res_list_bound_from_take_or_drop_frees_its_block; items SOURCE [lines 17-39, 51-100]
- `tests/runtime/rt_res_list_from_call_leak/user_function.rs` (100): tests (1) a_res_list_bound_from_a_user_function_frees_its_block; items USER_SOURCE [lines 102-150, 152-210]
- `tests/runtime/rt_res_list_from_call_leak/native_members.rs` (160): tests (2) a_res_list_bound_from_a_native_member_frees_its_block, a_nested_res_list_from_chunks_or_window_frees_every_block; items MEMBER_SOURCE, NESTED_SOURCE [lines 212-243, 245-294, 296-326, 328-382]
Surface: none (no support file; root keeps every shared helper).
Sweeps: none (§4.0 commands: 0 hits).

#### 4.19 `tests/runtime/rt_res_state_inplace_mutation.rs` — 651 test LOC, 24 tests

- `tests/runtime/rt_res_state_inplace_mutation.rs` (64): header; helpers TARGET, stack_slot_count, label_count, function, ncode, state_src [lines 35-35, 37-46, 48-62, 64-71, 73-78, 289-307]; mods field_updates, collection_ops, splices, operand_analysis
- `tests/runtime/rt_res_state_inplace_mutation/field_updates.rs` (159): tests (6) scalar_state_field_assign_stores_in_place, collection_state_field_grows_in_place, whole_state_replace_still_rebuilds, string_state_field_still_rebuilds, record_field_append_grows_in_place, record_field_append_not_last_inlined_rebuilds [lines 80-110, 112-147, 149-176, 178-206, 208-239, 241-270]
- `tests/runtime/rt_res_state_inplace_mutation/collection_ops.rs` (174): tests (7) remove_key_on_a_state_field_mutates_in_place, add_on_a_state_field_grows_the_state_block, a_growing_state_arm_reads_the_field_offset_before_the_grow, map_set_on_a_state_field_grows_the_state_block, fixed_width_list_set_on_a_state_field_writes_in_place, variable_width_list_set_on_a_state_field_grows_the_state_block, a_second_updated_state_field_is_stored_beside_the_arm [lines 309-334, 336-359, 361-384, 386-407, 409-432, 434-469, 471-509]
- `tests/runtime/rt_res_state_inplace_mutation/splices.rs` (157): tests (6) remove_at_on_a_state_field_mutates_in_place, remove_at_on_a_recursive_element_declines_in_the_state_container, set_remove_on_a_state_field_mutates_in_place, insert_on_a_state_field_grows_the_state_block, prepend_on_a_state_field_grows_the_state_block, a_plain_local_splice_does_not_use_the_state_grow [lines 522-543, 545-580, 582-603, 605-630, 632-660, 662-692]
- `tests/runtime/rt_res_state_inplace_mutation/operand_analysis.rs` (109): tests (5) state_field_append_whose_operand_reaches_a_state_assign_declines, state_field_append_whose_operand_reaches_a_state_assign_transitively_declines, state_field_append_whose_operand_call_chain_never_assigns_state_grows_in_place, scalar_state_field_whose_operand_reaches_a_state_assign_declines, scalar_state_field_whose_operand_never_assigns_state_stores_in_place; items g25_src [lines 713-744, 746-769, 771-786, 788-813, 815-829, 831-847]
Surface: none (no support file; root keeps every shared helper).
Sweeps: `.ai/testing-gates.md:22` names it; `.ai/resources-packages.md:49,176` cite `tests/rt_res_state_inplace_mutation.rs` — a path that ALREADY does not exist (the file is under tests/runtime/; pre-existing stale reference, `ls tests/rt_res_state_inplace_mutation.rs` → missing) — pre-existing, not caused by this split; Phase 3 repoints both to `tests/runtime/rt_res_state_inplace_mutation.rs` (its own task; AGENTS.md: never leave a bug). The root path stays, so `.ai/testing-gates.md:22` stays valid.

#### 4.20 `tests/runtime/rt_resource_aggregate_temporary.rs` — 616 test LOC, 22 tests

- `tests/runtime/rt_resource_aggregate_temporary.rs` (185): header; helpers PRELUDE, program, Counts, run_at, run_limited, assert_statement_owns_it, POSITIONS [lines 28-102, 104-112, 114-120, 122-133, 135-150, 152-182, 400-432]; mods statement_drops, positions, takers
- `tests/runtime/rt_resource_aggregate_temporary/statement_drops.rs` (189): tests (9) the_reported_loop_runs_out_of_no_descriptors, an_aggregate_given_to_a_builtin_that_only_measures_it_is_dropped_with_the_statement, an_element_listed_twice_is_dropped_once, an_aggregate_given_to_a_function_that_keeps_nothing_is_dropped_with_the_statement, every_statement_kind_drops_its_own_temporaries, a_failing_receiving_call_drops_the_temporary_on_the_error_exit, a_skipped_producer_drops_nothing, an_element_the_receiving_function_stores_at_module_level_stays_open, an_aggregate_a_longer_lived_value_keeps_stays_usable [lines 184-209, 211-221, 223-227, 229-234, 236-272, 274-309, 311-332, 334-372, 374-398]
- `tests/runtime/rt_resource_aggregate_temporary/positions.rs` (77): tests (7) a_constructor_argument_is_released_with_the_record_that_took_it, a_member_read_of_a_call_result_is_released_by_whatever_took_it, a_for_each_iterable_is_released_when_the_loop_ends, an_argument_of_a_call_that_answers_with_a_reference_is_dropped_with_the_statement, a_result_reaching_a_resource_union_is_dropped_with_the_statement, a_receiver_that_reads_a_field_of_its_record_argument_keeps_nothing, an_inline_trap_on_the_receiving_call_releases_the_argument [lines 434-444, 446-462, 464-473, 475-491, 493-497, 499-509, 511-528]
- `tests/runtime/rt_resource_aggregate_temporary/takers.rs` (174): tests (6) an_element_a_loop_body_stores_further_out_takes_that_scope, a_res_binding_takes_the_one_resource_it_names, a_binding_through_a_call_over_a_nested_list_ends_balanced, a_record_built_around_a_call_and_returned_by_name_ends_balanced, an_inline_trap_argument_that_holds_blocks_by_pointer_frees_them, every_other_taker_releases_what_it_took [lines 530-578, 580-596, 598-614, 616-630, 632-662, 664-724]
Surface: none (no support file; root keeps every shared helper).
Sweeps: none (§4.0 commands: 0 hits).

#### 4.21 `tests/runtime/rt_resource_drop_reclaims.rs` — 741 test LOC, 20 tests

- `tests/runtime/rt_resource_drop_reclaims.rs` (140): header; helpers Counts, run_at, assert_balanced, PRELUDE, program, LIST_PRELUDE, list_program, LOOP_MAIN [lines 16-22, 24-41, 43-68, 91-122, 124-126, 260-291, 293-295, 297-305]; mods elements, unions, lists, calls
- `tests/runtime/rt_resource_drop_reclaims/elements.rs` (202): tests (6) a_list_around_a_same_scope_binding_leaves_the_record_to_the_binding, an_element_returned_out_of_the_list_stays_open_for_the_caller, an_element_a_callee_hands_back_is_freed_once, an_element_copied_into_an_outer_list_takes_its_scope, an_element_a_callback_keeps_is_closed_not_freed, a_list_given_to_a_reading_callback_is_still_freed; items assert_no_double_free [lines 70-89, 487-516, 518-543, 545-562, 564-606, 608-671, 673-703]
- `tests/runtime/rt_resource_drop_reclaims/unions.rs` (122): tests (4) a_union_bound_from_a_producer_frees_the_write_buffer, a_union_bound_from_a_producer_frees_the_read_buffer, a_union_wrapping_a_live_binding_stays_balanced, a_stateful_union_bound_from_a_producer_frees_its_buffers [lines 130-156, 158-186, 188-219, 221-256]
- `tests/runtime/rt_resource_drop_reclaims/lists.rs` (162): tests (6) an_adopted_list_read_through_get_frees_its_element, a_floated_element_read_through_get_is_freed, an_iterated_list_frees_its_block, a_floated_element_of_an_iterated_list_is_freed, an_adopted_list_iterated_here_and_in_a_callee_is_freed, a_floated_element_given_to_a_named_callee_is_freed [lines 307-321, 323-341, 343-373, 375-406, 408-443, 445-481]
- `tests/runtime/rt_resource_drop_reclaims/calls.rs` (127): tests (4) a_record_built_from_a_list_of_resources_frees_what_it_holds, a_nested_list_of_resources_bound_from_a_call_frees_its_inner_blocks, a_resource_returned_by_a_call_and_used_unbound_frees_its_record, a_resource_used_unbound_through_a_list_building_call_frees_its_record; items run_limited, assert_balanced_and_closed [lines 708-724, 726-739, 741-771, 773-803, 805-826, 828-851]
Surface: none (no support file; root keeps every shared helper).
Sweeps: none (§4.0 commands: 0 hits).

#### 4.22 `tests/runtime/rt_resource_last_holder.rs` — 1476 test LOC, 14 tests

- `tests/runtime/rt_resource_last_holder.rs` (170): header; helpers prelude, Run, run, measure, check, check_at_both_levels, check_built [lines 23-77, 79-83, 85-102, 104-125, 127-132, 134-140, 142-186]; mods bindings_and_copies, exits, reads, callee_copies, trapped_calls, trapped_element_reads, map_iteration
- `tests/runtime/rt_resource_last_holder/bindings_and_copies.rs` (179): tests (6) a_handle_bound_on_its_own_leaves_the_record_to_its_owner, a_record_returned_after_a_copy_is_freed_by_the_caller, a_record_captured_by_a_reading_lambda_is_freed, a_list_whose_element_a_callee_stores_elsewhere_frees_its_block, a_record_whose_copy_is_appended_to_a_list_is_freed, a_resource_union_and_a_map_follow_the_same_rule [lines 188-205, 207-230, 232-248, 250-285, 287-305, 307-382]
- `tests/runtime/rt_resource_last_holder/exits.rs` (182): tests (2) a_loop_body_frees_on_every_way_out, an_error_exit_frees_what_the_scope_took [lines 384-452, 454-572]
- `tests/runtime/rt_resource_last_holder/reads.rs` (121): tests (2) two_cleanups_that_name_one_record_free_it_once, a_list_read_through_get_or_copied_by_a_callee_is_freed [lines 574-634, 636-702]
- `tests/runtime/rt_resource_last_holder/callee_copies.rs` (183): tests (1) a_copy_of_a_record_returned_by_a_callee_is_freed_at_every_level [lines 704-893]
- `tests/runtime/rt_resource_last_holder/trapped_calls.rs` (131): tests (1) a_trapped_call_that_returns_its_argument_frees_the_default_record [lines 895-1034]
- `tests/runtime/rt_resource_last_holder/trapped_element_reads.rs` (246): tests (1) a_trapped_element_read_does_not_keep_the_list_block [lines 1036-1289]
- `tests/runtime/rt_resource_last_holder/map_iteration.rs` (285): tests (1) a_map_of_resources_iterated_through_its_values_frees_its_block [lines 1291-1583]
Surface: none (no support file; root keeps every shared helper).
Sweeps: none (§4.0 commands: 0 hits).
Exempt (§Decisions; row added in Phase 3): `tests/runtime/rt_resource_last_holder/map_iteration.rs` (285) — the single `#[test]` `a_map_of_resources_iterated_through_its_values_frees_its_block` (284 LOC: nine sequential `check_at_both_levels` calls, lines 1291-1583); splitting it changes the test count. `trapped_element_reads.rs` (246) is one 245-LOC test and is within the limit.

#### 4.23 `tests/runtime/rt_resource_model.rs` — 292 test LOC, 23 tests

- `tests/runtime/rt_resource_model.rs` (89): header; helpers Run, counter, run, holds [lines 24-29, 31-39, 41-63, 65-106]; mods bugs_1018_to_1067, bugs_1069_to_1078, bugs_999
- `tests/runtime/rt_resource_model/bugs_1018_to_1067.rs` (91): tests (10) bug_1018_list_of_res_argument, bug_1018_list_of_res_bound_from_a_call, bug_1018_map_of_res_argument, bug_1063_state_variant_to_union_bind, bug_1063_state_variant_to_union_arg, bug_1063_state_variant_to_union_append, bug_1064_record_from_a_call_under_an_inline_trap, bug_1065_stateful_list_floated_from_a_loop, bug_1066_union_map_for_each_match, bug_1067_list_literal_assigned_in_a_loop [lines 108-116, 118-126, 128-136, 138-146, 148-156, 158-166, 168-176, 178-186, 188-196, 198-206]
- `tests/runtime/rt_resource_model/bugs_1069_to_1078.rs` (84): tests (9) bug_1069_res_in_constructor_field_appended, bug_1070_list_raised_from_a_loop_copied_by_a_callee, bug_1071_inner_list_appended_to_a_list_of_lists, bug_1072_bound_list_of_res_inline_trap, bug_1074_with_update_from_a_call, bug_1075_foreach_lambda_appends_to_captured_outer_list, bug_1076_filter_result_assigned_to_outer_list, bug_1077_function_trap_handler_reads_drained_list, bug_1078_close_only_drain_kept_beside_an_unraised_outward_store [lines 208-216, 218-226, 228-236, 238-246, 248-256, 258-266, 268-276, 278-286, 288-298]
- `tests/runtime/rt_resource_model/bugs_999.rs` (37): tests (4) bug_999_call_through_a_function_value, bug_999_callback_result_unbound, bug_999_match_on_an_element_of_a_call, bug_999_record_copy_source_reassigned [lines 300-308, 310-318, 320-328, 330-338]
Surface: none (no support file; root keeps every shared helper).
Sweeps: Each of the 23 tests embeds `include_str!("../../bugs/repro/bug-NNNN-….mfb")` (lines 112-330, `grep -n include_str tests/runtime/rt_resource_model.rs` → 23). Moving into `rt_resource_model/<m>.rs` → every one must become `include_str!("../../../bugs/repro/…")`.

#### 4.24 `tests/runtime/rt_resource_review_805.rs` — 574 test LOC, 18 tests

- `tests/runtime/rt_resource_review_805.rs` (115): header; helpers PRELUDE, Run, build_and_run, run, double_free_skips [lines 25-63, 65-69, 71-108, 110-123, 125-135]; mods nested_returns, mixed_transfers, ownership_edges, leaks
- `tests/runtime/rt_resource_review_805/nested_returns.rs` (137): tests (5) a_nested_list_of_records_returns_a_live_handle, a_nested_list_of_records_closes_once_at_the_callers_scope, a_list_of_records_transfers_its_handles, a_map_of_records_transfers_its_handles, a_record_holding_a_list_of_records_transfers_its_handles [lines 137-170, 172-203, 205-227, 229-251, 253-282]
- `tests/runtime/rt_resource_review_805/mixed_transfers.rs` (120): tests (3) a_record_with_two_resource_kinds_transfers_both, a_data_union_variant_transfers_its_handle, a_recursive_record_transfers_every_handle_in_the_tree [lines 284-323, 325-367, 369-410]
- `tests/runtime/rt_resource_review_805/ownership_edges.rs` (133): tests (6) a_discarded_nested_result_closes_what_it_carried, an_error_before_the_return_closes_in_the_callee, a_passed_in_handle_returned_inside_a_nested_aggregate_keeps_its_owner, a_repackaged_nested_value_does_not_adopt_the_callers_handles, the_flat_resource_list_return_is_unchanged, a_declared_resource_union_does_not_change_a_plain_file_return [lines 412-431, 433-459, 461-483, 485-511, 513-537, 539-563]
- `tests/runtime/rt_resource_review_805/leaks.rs` (81): tests (4) a_returned_nested_aggregate_leaks_nothing_per_call, a_returned_flat_resource_list_frees_its_float_nodes, a_resource_removed_before_the_return_closes_in_the_callee, a_container_assigned_a_call_result_adopts_its_resources; items live_bytes, live_after_calls [lines 565-573, 575-585, 587-596, 598-606, 608-630, 632-664]
Surface: none (no support file; root keeps every shared helper).
Sweeps: none (§4.0 commands: 0 hits).

#### 4.25 `tests/runtime/rt_resource_review_806.rs` — 532 test LOC, 17 tests

- `tests/runtime/rt_resource_review_806.rs` (121): header; helpers PRELUDE, Run, impl Run, build_and_run, run, double_free_skips [lines 27-66, 68-73, 75-80, 82-120, 122-137, 139-146]; mods globals, teardown, stores
- `tests/runtime/rt_resource_review_806/globals.rs` (186): tests (8) a_resource_appended_to_a_global_stays_readable, a_global_resource_closes_once_at_program_exit, a_parameter_stored_into_a_global_outlives_its_callers_scope, a_record_parameter_stored_into_a_global_outlives_its_callers_scope, a_flat_global_resource_list_keeps_its_handles, a_nested_global_aggregate_keeps_its_handles, removing_the_last_global_view_does_not_close, an_explicit_close_through_the_global_is_honoured [lines 148-179, 181-204, 206-230, 232-255, 257-277, 279-299, 301-323, 325-349]
- `tests/runtime/rt_resource_review_806/teardown.rs` (130): tests (5) exit_program_closes_global_resources, an_untrapped_error_closes_global_resources, a_workers_global_resource_closes_at_worker_exit, a_global_initializer_resource_closes_at_program_exit, repeated_global_stores_close_each_handle_once [lines 351-378, 380-404, 406-441, 443-459, 461-493]
- `tests/runtime/rt_resource_review_806/stores.rs` (104): tests (4) a_local_outer_container_float_is_unchanged, a_union_parameter_stored_into_a_global_outlives_its_callers_scope, a_global_map_of_records_keeps_its_handles, a_call_result_stored_into_a_global_is_raised_after_the_store [lines 495-519, 521-556, 558-578, 580-608]
Surface: none (no support file; root keeps every shared helper).
Sweeps: none (§4.0 commands: 0 hits).

#### 4.26 `tests/runtime/rt_resource_store_float.rs` — 499 test LOC, 8 tests

- `tests/runtime/rt_resource_store_float.rs` (148): header; helpers prelude, Run, run, measure, check [lines 25-94, 96-100, 102-119, 121-142, 144-171]; mods inserting_stores, scopes
- `tests/runtime/rt_resource_store_float/inserting_stores.rs` (153): tests (3) a_record_from_a_loop_body_appended_to_an_outer_list_keeps_its_resource_open, every_inserting_store_raises_what_it_stores, a_resource_union_field_is_raised_like_a_concrete_one [lines 173-213, 215-306, 308-336]
- `tests/runtime/rt_resource_store_float/scopes.rs` (204): tests (5) the_store_may_sit_in_any_inner_scope_and_any_number_of_scopes_in, the_raised_resource_follows_the_container_out, only_what_was_stored_is_raised, an_element_of_an_inner_container_is_raised_out_of_that_container, a_handle_owned_further_out_is_left_with_its_owner [lines 338-375, 377-412, 414-486, 488-533, 535-556]
Surface: none (no support file; root keeps every shared helper).
Sweeps: none (§4.0 commands: 0 hits).

#### 4.27 `tests/runtime/rt_resource_trap_state_free.rs` — 259 test LOC, 4 tests

- `tests/runtime/rt_resource_trap_state_free.rs` (132): header; helpers prelude, Run, run, measure, check [lines 22-74, 76-80, 82-99, 101-122, 124-152]; mods bindings, exits
- `tests/runtime/rt_resource_trap_state_free/bindings.rs` (43): tests (2) a_stateful_binding_through_an_inline_trap_frees_its_state, a_stateful_union_binding_through_an_inline_trap_frees_its_state [lines 154-183, 185-200]
- `tests/runtime/rt_resource_trap_state_free/exits.rs` (90): tests (2) the_state_is_freed_on_every_way_out_of_the_binding_s_scope, the_paths_that_were_right_stay_right [lines 202-250, 252-296]
Surface: none (no support file; root keeps every shared helper).
Sweeps: none (§4.0 commands: 0 hits).

#### 4.28 `tests/runtime/rt_resource_union_box_owner.rs` — 629 test LOC, 16 tests

- `tests/runtime/rt_resource_union_box_owner.rs` (132): header; helpers TOMBSTONE, prelude, Run, run, measure, check [lines 24-25, 27-68, 70-74, 76-93, 95-116, 118-154]; mods wrap_sites, bindings, element_reads
- `tests/runtime/rt_resource_union_box_owner/wrap_sites.rs` (144): tests (6) a_record_field_outlives_the_binding_that_wrapped_its_box, a_box_wrapped_at_a_call_argument_is_freed, a_box_wrapped_at_a_list_element_is_freed, a_box_wrapped_at_a_record_field_is_freed, a_returned_list_of_boxes_is_not_freed_twice, a_returned_list_with_a_floated_box_is_not_freed_twice [lines 156-202, 204-224, 226-248, 250-274, 276-292, 294-314]
- `tests/runtime/rt_resource_union_box_owner/bindings.rs` (171): tests (5) a_binding_that_owns_its_box_stays_balanced, a_binding_of_a_field_outlives_the_record_it_was_read_from, a_record_holding_a_box_is_copied_and_dropped_whole, a_statement_left_early_frees_its_boxes, a_returned_box_is_freed_by_its_caller [lines 316-349, 351-370, 372-427, 429-475, 477-497]
- `tests/runtime/rt_resource_union_box_owner/element_reads.rs` (191): tests (5) a_binding_of_a_list_element_outlives_the_block_it_was_read_from, a_binding_of_a_map_value_outlives_the_block_it_was_read_from, a_binding_of_a_getor_default_outlives_the_statement_that_wrapped_it, a_trapped_binding_of_an_element_outlives_the_block_it_was_read_from, a_trapped_binding_recovered_with_an_alias_owns_its_box [lines 499-550, 552-572, 574-618, 620-685, 687-723]
Surface: none (no support file; root keeps every shared helper).
Sweeps: none (§4.0 commands: 0 hits).

#### 4.29 `tests/runtime/rt_resource_union_member_float.rs` — 416 test LOC, 6 tests

- `tests/runtime/rt_resource_union_member_float.rs` (137): header; helpers prelude, Run, run, measure, check [lines 27-85, 87-91, 93-110, 112-133, 135-162]; mods lists, maps_and_scopes
- `tests/runtime/rt_resource_union_member_float/lists.rs` (129): tests (3) a_member_floated_from_a_loop_into_a_list_is_tagged_closed_and_freed, a_member_floated_into_a_returned_list_is_adopted_by_the_caller, a_member_floated_before_an_early_return_or_through_an_inline_trap [lines 164-229, 231-265, 267-301]
- `tests/runtime/rt_resource_union_member_float/maps_and_scopes.rs` (156): tests (3) a_member_floated_into_a_map_is_tagged_closed_and_freed, a_member_gets_its_own_tag_in_every_union_that_lists_it, a_member_stored_in_the_scope_that_declares_it_stays_balanced [lines 303-351, 353-388, 390-466]
Surface: none (no support file; root keeps every shared helper).
Sweeps: none (§4.0 commands: 0 hits).

#### 4.30 `tests/runtime/rt_resource_union_reclaim.rs` — 561 test LOC, 10 tests

- `tests/runtime/rt_resource_union_reclaim.rs` (144): header; helpers prelude, Run, run, Counters, measure, Expect, check [lines 21-55, 57-61, 63-80, 82-88, 90-110, 112-121, 123-167]; mods iteration, trapped_bindings, record_bindings
- `tests/runtime/rt_resource_union_reclaim/iteration.rs` (182): tests (4) an_iterated_list_of_resource_unions_frees_its_block, a_collection_of_resource_unions_read_through_get_frees_its_block, a_list_of_floated_resource_unions_frees_its_block_and_records, a_case_binding_that_leaves_keeps_its_list_from_freeing_the_record [lines 169-242, 244-288, 290-325, 327-368]
- `tests/runtime/rt_resource_union_reclaim/trapped_bindings.rs` (118): tests (3) a_trapped_resource_union_binding_releases_its_default, a_trapped_resource_union_binding_recovered_releases_its_default, a_trapped_resource_union_binding_in_a_loop_releases_its_default; items MAKERS, check_trap [lines 370-380, 382-389, 391-424, 426-464, 466-503]
- `tests/runtime/rt_resource_union_reclaim/record_bindings.rs` (126): tests (3) a_record_binding_frees_the_resource_record_it_adopted, a_copied_record_binding_frees_the_record_and_its_read_buffer, a_record_binding_whose_handle_leaves_does_not_free_the_record; items HOLDERS, check_holder [lines 505-520, 522-524, 526-573, 575-614, 616-652]
Surface: none (no support file; root keeps every shared helper).
Sweeps: none (§4.0 commands: 0 hits).

#### 4.31 `tests/runtime/rt_self_update_cost.rs` — 1337 test LOC, 13 tests

- `tests/runtime/rt_self_update_cost.rs` (43): header; helpers expected, counter [lines 1076-1093, 1263-1269]; mods linearity, scale_program, value_program, value_semantics, list_churn, map_churn, small_churn
- `tests/runtime/rt_self_update_cost/linearity.rs` (110): tests (7) remove_at_near_the_end_of_a_variable_width_list_is_linear, remove_key_and_set_remove_are_linear, difference_with_a_small_argument_is_linear, symmetric_difference_with_a_small_argument_is_linear, string_window_self_updates_that_keep_the_start_are_linear, truncating_an_out_of_order_variable_width_list_is_linear, append_stays_linear; items ROUNDS, MAX_RATIO, SLACK_SECONDS, scale_exe, cpu_run, assert_linear [lines 47-47, 48-48, 49-49, 1068-1074, 1095-1138, 1140-1171, 1173-1180, 1182-1188, 1190-1194, 1196-1200, 1202-1208, 1210-1216, 1218-1222]
- `tests/runtime/rt_self_update_cost/scale_program.rs` (support) (166; no `use super::*;`, see §3): items SCALE_PROGRAM [lines 51-218]
- `tests/runtime/rt_self_update_cost/value_program.rs` (support) (443; no `use super::*;`, see §3): items VALUE_PROGRAM [lines 220-664]
- `tests/runtime/rt_self_update_cost/value_semantics.rs` (124): tests (2) removals_and_windows_keep_value_semantics, removals_free_every_removed_element_once; items VALUE_OUTPUT, without_globals [lines 666-729, 1224-1238, 1240-1261, 1271-1300]
- `tests/runtime/rt_self_update_cost/list_churn.rs` (131): tests (1) a_churning_list_does_not_grow_without_bound; items CHURN_PROGRAM [lines 731-832, 1302-1338]
- `tests/runtime/rt_self_update_cost/map_churn.rs` (224): tests (2) a_churning_map_or_set_does_not_grow_without_bound, a_map_reclaim_terminates_when_moved_values_pack_tighter; items MAP_CHURN_PROGRAM, MAP_MOVED_VALUES_PROGRAM [lines 834-976, 978-1004, 1340-1391, 1393-1409]
- `tests/runtime/rt_self_update_cost/small_churn.rs` (117): tests (1) a_small_list_under_length_changing_churn_stays_small; items SMALL_CHURN_PROGRAM, run_with_deadline [lines 1006-1066, 1411-1437, 1439-1478]
Surface: pub(super) on 2 item(s): SCALE_PROGRAM, VALUE_PROGRAM; root adds `use scale_program::*;`, `use value_program::*;`.
Sweeps: `.ai/collections.md:47,75` name the root path/target — unchanged.
Exempt (§Decisions; row added in Phase 4): `tests/runtime/rt_self_update_cost/value_program.rs` (443) — the single `const VALUE_PROGRAM: &str = r#"…"#` MFBASIC program (lines 220-664) consumed whole by `value_semantics.rs`; it cannot be split without editing the program or its consumer.

#### 4.32 `tests/runtime/rt_scope_drop_leaks.rs` — 5476 test LOC, 154 tests

- `tests/runtime/rt_scope_drop_leaks.rs` (216): header; helpers peak_rss, assert_flat, B561_TRAP_ALWAYS_FAILS, B573_BUILTIN_RAISE, condition_temp_report, assert_condition_temps_freed, thread_loop_report, thread_loop_growth [lines 28-53, 55-69, 1253-1282, 2703-2730, 6325-6354, 6356-6384, 6680-6699, 6701-6709]; mods return_shapes, string_producers, user_string_returns, callback_callees, string_return_values, string_self_append, trap_bound_results, trap_bound_values, hof_callbacks, hof_callback_frees, for_each_items, for_each_values, closure_arguments, trapped_errors, param_borrows, returned_concats, returned_concat_values, runtime_helper_results, helper_scratch, helper_scratch_values, error_filename_depth, raised_errors, unbound_helper_results, unbound_helper_values, getor_elements, get_contrasts, unbound_collection_values, trapped_error_retention, addresses, recursive_loops, json_regex_trap_loops, branch_exit_temps, loop_condition_temps, thread_results, thread_handle_moves
- `tests/runtime/rt_scope_drop_leaks/return_shapes.rs` (117): tests (4) returning_a_record_constructor_runs_at_constant_rss, returning_a_fresh_call_result_runs_at_constant_rss, returning_an_owned_local_still_runs_at_constant_rss, every_return_shape_still_produces_the_right_value; items SHAPE_A_CONSTRUCTOR, SHAPE_A_CALL, SHAPE_A_CONTRAST_LOCAL, SHAPE_A_BEHAVIOUR [lines 73-88, 90-105, 107-122, 124-128, 130-134, 136-145, 147-181, 183-207]
- `tests/runtime/rt_scope_drop_leaks/string_producers.rs` (187): tests (5) an_unbound_string_call_result_runs_at_constant_rss, an_unbound_string_concat_runs_at_constant_rss, unbound_native_string_producers_run_at_constant_rss, a_bound_string_call_result_still_runs_at_constant_rss, every_string_producer_still_produces_the_right_value; items SHAPE_B_CALL_RESULT, SHAPE_B_CONCAT, SHAPE_B_NATIVE_PRODUCERS, SHAPE_B_CONTRAST_BOUND, SHAPE_B_BEHAVIOUR [lines 211-226, 228-239, 241-270, 272-295, 297-301, 303-307, 309-318, 320-329, 331-394, 396-442]
- `tests/runtime/rt_scope_drop_leaks/user_string_returns.rs` (93): tests (4) an_unbound_user_string_call_result_runs_at_constant_rss, a_transitive_user_string_call_chain_runs_at_constant_rss, a_returned_string_literal_runs_at_constant_rss, tostring_of_a_fresh_string_runs_at_constant_rss; items SHAPE_B2_APPEND_ARGUMENT, SHAPE_B2_TRANSITIVE, SHAPE_B2_RETURNED_LITERAL, SHAPE_B2_TOSTRING_IDENTITY [lines 446-483, 485-503, 505-524, 526-553, 582-591, 593-597, 599-608, 610-619]
- `tests/runtime/rt_scope_drop_leaks/callback_callees.rs` (117): tests (4) a_callback_invoked_through_a_user_function_returns_an_owned_block, a_direct_call_to_a_callback_referenced_string_callee_runs_at_constant_rss, a_direct_call_to_a_plain_string_callee_still_runs_at_constant_rss, a_param_borrow_string_callee_still_runs_at_constant_rss; items SHAPE_B2_PARAM_BORROW_CONTRAST, SHAPE_562_CALLBACK_REFERENCED_DIRECT_CALL, SHAPE_562_PLAIN_DIRECT_CALL_CONTRAST, SHAPE_562_INDIRECT_CALLABLE [lines 555-580, 623-650, 652-668, 670-711, 713-743, 745-754, 756-765, 767-776]
- `tests/runtime/rt_scope_drop_leaks/string_return_values.rs` (108): tests (1) every_string_return_shape_still_produces_the_right_value; items SHAPE_B2_BEHAVIOUR [lines 778-851, 853-901]
- `tests/runtime/rt_scope_drop_leaks/string_self_append.rs` (214): tests (5) a_reassigned_string_self_append_runs_at_constant_rss, a_returned_string_self_append_runs_at_constant_rss, the_utf32_decoder_runs_at_constant_rss, a_plain_string_self_append_still_runs_at_constant_rss, every_string_self_append_shape_still_produces_the_right_value; items B560_REASSIGNED_SELF_APPEND, B560_RETURNED_SELF_APPEND, B560_DECODER, B560_CONTRAST_PLAIN, B560_BEHAVIOUR [lines 905-925, 927-953, 955-976, 978-992, 994-1003, 1005-1009, 1011-1015, 1017-1021, 1023-1114, 1116-1169]
- `tests/runtime/rt_scope_drop_leaks/trap_bound_results.rs` (144): tests (6) a_trap_bound_string_result_runs_at_constant_rss, a_trap_bound_collection_result_runs_at_constant_rss, a_trap_bound_inline_builtin_result_runs_at_constant_rss, a_trap_whose_call_always_fails_still_produces_the_right_value, a_trap_bound_scalar_result_still_runs_at_constant_rss, the_same_producers_without_a_trap_still_run_at_constant_rss; items B561_TRAP_STRING, B561_TRAP_LIST, B561_TRAP_INLINE_BUILTIN, B561_CONTRAST_SCALAR, B561_CONTRAST_NO_TRAP [lines 1173-1199, 1201-1223, 1225-1251, 1284-1304, 1306-1328, 1330-1334, 1336-1340, 1342-1351, 1353-1373, 1375-1379, 1381-1385]
- `tests/runtime/rt_scope_drop_leaks/trap_bound_values.rs` (141): tests (1) every_trap_bound_result_still_produces_the_right_value; items B561_BEHAVIOUR [lines 1387-1474, 1476-1557]
- `tests/runtime/rt_scope_drop_leaks/hof_callbacks.rs` (131): tests (6) transform_runs_at_constant_rss_with_a_string_callback, sort_by_runs_at_constant_rss_with_a_string_key, group_by_runs_at_constant_rss_with_a_string_value, map_values_runs_at_constant_rss_with_a_string_callback, a_bare_parameter_callback_runs_at_constant_rss, a_fixed_width_callback_stays_flat; items SHAPE_569_TRANSFORM, SHAPE_569_SORT_BY, SHAPE_569_GROUP_BY, SHAPE_569_MAP_VALUES, SHAPE_569_BARE_PARAM_CALLBACK, SHAPE_569_FIXED_WIDTH_CONTRAST [lines 1561-1582, 1584-1599, 1601-1616, 1618-1643, 1645-1669, 1671-1687, 1689-1694, 1696-1702, 1704-1708, 1710-1717, 1719-1730, 1732-1741]
- `tests/runtime/rt_scope_drop_leaks/hof_callback_frees.rs` (104): tests (1) every_hof_frees_its_callback_result_exactly_once [lines 1743-1861]
- `tests/runtime/rt_scope_drop_leaks/for_each_items.rs` (216): tests (7) a_for_each_over_a_list_of_string_does_not_leak_its_element, a_for_each_over_a_map_of_string_does_not_leak_either_side, a_for_each_over_a_set_of_string_does_not_leak_its_element, a_for_each_over_fixed_width_elements_stays_flat, an_early_exit_from_a_for_each_does_not_leak_the_item, returning_the_loop_item_neither_leaks_nor_double_frees, a_for_each_body_that_uses_the_item_does_not_leak_it; items SHAPE_571_LIST_OF_STRING, SHAPE_571_MAP_OF_STRING, SHAPE_571_SET_OF_STRING, SHAPE_571_CONTRAST_MAP_OF_INTEGER, SHAPE_571_CONTRAST_LIST_OF_INTEGER, SHAPE_571_EXIT_FOR, SHAPE_571_CONTINUE_FOR, SHAPE_571_RETURN_ITEM, SHAPE_571_USES_THE_ITEM [lines 1865-1882, 1884-1905, 1907-1928, 1930-1953, 1955-1968, 1970-1987, 1989-2006, 2008-2033, 2035-2058, 2060-2070, 2072-2082, 2084-2094, 2096-2113, 2115-2122, 2124-2133, 2135-2140]
- `tests/runtime/rt_scope_drop_leaks/for_each_values.rs` (147): tests (1) every_for_each_body_shape_still_produces_the_right_value [lines 2142-2309]
- `tests/runtime/rt_scope_drop_leaks/closure_arguments.rs` (217): tests (5) a_capturing_lambda_argument_does_not_leak_its_environment, a_captureless_lambda_argument_stays_flat, every_admitted_callback_position_frees_its_closure, a_retained_callback_position_is_freed_by_its_owners, every_closure_argument_shape_still_produces_the_right_value; items SHAPE_572_CAPTURING_FILTER, SHAPE_572_CONTRAST_CAPTURELESS, SHAPE_572_REDUCE, SHAPE_572_FOREACH_BY_REF, SHAPE_572_MAP_VALUES, SHAPE_572_RETAINED_ROUTE [lines 2313-2332, 2334-2350, 2352-2368, 2370-2387, 2389-2410, 2412-2430, 2432-2442, 2444-2454, 2456-2470, 2472-2490, 2492-2598]
- `tests/runtime/rt_scope_drop_leaks/trapped_errors.rs` (218): tests (7) the_same_trap_whose_call_never_fails_runs_at_constant_rss, a_trap_whose_call_always_fails_runs_at_constant_rss, a_trapped_error_read_by_its_handler_runs_at_constant_rss, a_failing_call_inside_a_collection_walk_runs_at_constant_rss, an_auto_propagated_error_out_of_a_loop_body_runs_at_constant_rss, an_inline_builtins_own_domain_error_runs_at_constant_rss, every_trapped_error_shape_still_produces_the_right_value; items B565_TRAP_ALWAYS_FAILS, B565_HANDLER_READS_ERROR, B565_FAILING_CALL_IN_A_WALK, B565_AUTO_PROPAGATED, B565_CONTROL_NEVER_FAILS, B565_ERROR_SHAPES [lines 2602-2616, 2618-2641, 2643-2668, 2670-2701, 2732-2754, 2756-2765, 2767-2776, 2778-2787, 2789-2798, 2800-2804, 2806-2810, 2812-2879, 2881-2923]
- `tests/runtime/rt_scope_drop_leaks/param_borrows.rs` (175): tests (5) a_trap_over_a_param_returning_callee_runs_at_constant_rss, a_trap_over_every_scalar_payload_runs_at_constant_rss, a_trap_over_a_param_returning_string_callee_runs_at_constant_rss, a_param_borrow_without_a_trap_still_runs_at_constant_rss, every_param_borrow_shape_still_produces_the_right_value; items B568_TRAP_PARAM_BORROW, B568_TRAP_EVERY_SCALAR, B568_TRAP_STRING_BORROW, B568_CONTRAST_NO_TRAP, B568_BORROW_SHAPES [lines 2927-2964, 2966-3000, 3002-3022, 3024-3043, 3045-3054, 3056-3060, 3062-3071, 3073-3077, 3079-3134, 3136-3166]
- `tests/runtime/rt_scope_drop_leaks/returned_concats.rs` (178): tests (6) returning_a_nested_concat_runs_at_constant_rss, returning_a_concat_of_a_call_result_runs_at_constant_rss, returning_a_two_operand_concat_still_runs_at_constant_rss, returning_a_collection_built_from_interior_concats_runs_at_constant_rss, returning_an_interior_temp_beside_a_live_local_runs_at_constant_rss, a_failing_trap_does_not_leak_its_interior_temp; items SHAPE_567_NESTED_CONCAT, SHAPE_567_CONCAT_OF_A_CALL, SHAPE_567_CONTRAST_TWO_OPERANDS, SHAPE_567_RETURNED_LIST_OF_CONCATS, SHAPE_567_INTERIOR_TEMP_WITH_A_LIVE_LOCAL, SHAPE_567_FAIL_WITH_AN_INTERIOR_TEMP, SHAPE_567_FAIL_CONTRAST_NO_INTERIOR [lines 3170-3186, 3188-3201, 3203-3218, 3220-3236, 3238-3255, 3257-3278, 3280-3299, 3301-3310, 3312-3321, 3323-3333, 3335-3344, 3346-3358, 3360-3413]
- `tests/runtime/rt_scope_drop_leaks/returned_concat_values.rs` (83): tests (1) every_returned_concat_shape_still_produces_the_right_value [lines 3415-3505]
- `tests/runtime/rt_scope_drop_leaks/runtime_helper_results.rs` (228): tests (4) a_trapped_runtime_helper_string_result_grows_no_faster_than_the_plain_call, a_trapped_runtime_helper_collection_result_grows_no_faster_than_the_plain_call, a_trapped_runtime_helper_scalar_result_was_never_the_leak, every_trapped_runtime_helper_result_still_produces_the_right_value; items assert_no_extra_growth, SHAPE_566_TRAPPED_READ_TEXT, SHAPE_566_PLAIN_READ_TEXT, SHAPE_566_TRAPPED_READ_BYTES, SHAPE_566_PLAIN_READ_BYTES, SHAPE_566_TRAPPED_SCALAR, SHAPE_566_PLAIN_SCALAR [lines 3509-3554, 3556-3576, 3578-3593, 3595-3612, 3614-3627, 3629-3650, 3652-3667, 3669-3679, 3681-3691, 3693-3705, 3707-3789]
- `tests/runtime/rt_scope_drop_leaks/helper_scratch.rs` (194): tests (8) a_short_path_argument_runs_at_constant_rss, a_long_path_argument_runs_at_constant_rss, a_long_path_argument_in_a_local_runs_at_constant_rss, a_long_environment_name_runs_at_constant_rss, a_call_marshalling_two_arguments_runs_at_constant_rss, a_helper_with_no_argument_but_its_own_buffer_runs_at_constant_rss, a_helper_that_allocates_only_its_result_stays_flat, a_tls_connect_host_name_is_not_leaked_in_proportion_to_its_length; items SHAPE_574_SHORT_PATH, SHAPE_574_LONG_PATH, SHAPE_574_LOCAL_PATH, SHAPE_574_ENV_NAME, SHAPE_574_TWO_ARGUMENTS, SHAPE_574_NO_ARGUMENT, SHAPE_574_CONTRAST_ARCH, tls_connect_probe [lines 3805-3819, 3821-3845, 3847-3872, 3874-3896, 3898-3916, 3918-3941, 3943-3958, 3960-3964, 3966-3970, 3972-3976, 3978-3982, 3984-3988, 3990-3994, 3996-4000, 4057-4080, 4082-4100]
- `tests/runtime/rt_scope_drop_leaks/helper_scratch_values.rs` (117): tests (1) every_helper_whose_scratch_is_released_still_produces_the_right_value; items SHAPE_574_VALUES [lines 4102-4181, 4183-4228]
- `tests/runtime/rt_scope_drop_leaks/error_filename_depth.rs` (56): tests (1) a_raised_error_runs_at_constant_rss_however_long_its_filename_is; items peak_rss_at_depth, DEEP_SOURCE_DIRECTORY [lines 4243-4271, 4273-4279, 4281-4308]
- `tests/runtime/rt_scope_drop_leaks/raised_errors.rs` (177): tests (5) a_user_fail_runs_at_constant_rss, a_propagated_error_runs_at_constant_rss, a_runtime_helper_error_runs_at_constant_rss, a_loop_that_raises_nothing_still_runs_at_constant_rss, every_raised_error_still_reports_its_true_origin; items B573_USER_FAIL, B573_PROPAGATED, B573_HELPER_ERROR, B573_CONTRAST_NO_ERROR, B573_ORIGIN_VALUES [lines 4310-4328, 4330-4356, 4358-4375, 4377-4395, 4397-4401, 4403-4407, 4409-4413, 4415-4419, 4421-4485, 4487-4525]
- `tests/runtime/rt_scope_drop_leaks/unbound_helper_results.rs` (201): tests (9) an_unbound_helper_hostname_runs_at_constant_rss, an_unbound_helper_result_that_allocates_only_its_result_runs_at_constant_rss, an_unbound_helper_tempdir_runs_at_constant_rss, an_unbound_helper_result_consumed_by_concat_runs_at_constant_rss, an_unbound_helper_result_passed_to_another_call_runs_at_constant_rss, a_bound_helper_result_still_runs_at_constant_rss, a_bound_directory_helper_result_still_runs_at_constant_rss, a_returned_helper_result_still_runs_at_constant_rss, a_helper_result_moved_into_a_collection_still_runs_at_constant_rss; items SHAPE_576_UNBOUND_HOSTNAME, SHAPE_576_UNBOUND_ARCH, SHAPE_576_UNBOUND_TEMPDIR, SHAPE_576_UNBOUND_IN_CONCAT, SHAPE_576_UNBOUND_AS_ARGUMENT, SHAPE_576_CONTRAST_BOUND_ARCH, SHAPE_576_CONTRAST_BOUND_CWD, SHAPE_576_CONTRAST_RETURNED, SHAPE_576_CONTRAST_INTO_COLLECTION [lines 4529-4556, 4558-4570, 4572-4583, 4585-4599, 4601-4616, 4618-4633, 4635-4647, 4649-4673, 4675-4704, 4706-4715, 4717-4726, 4728-4737, 4739-4748, 4750-4759, 4761-4770, 4772-4781, 4783-4792, 4794-4803]
- `tests/runtime/rt_scope_drop_leaks/unbound_helper_values.rs` (99): tests (1) every_unbound_helper_result_position_still_produces_the_right_value; items SHAPE_576_VALUES [lines 4805-4866, 4868-4924]
- `tests/runtime/rt_scope_drop_leaks/getor_elements.rs` (154): tests (7) an_unbound_list_getor_hit_runs_at_constant_rss, an_unbound_list_getor_miss_runs_at_constant_rss, an_unbound_hash_map_getor_hit_runs_at_constant_rss, an_unbound_hash_map_getor_miss_runs_at_constant_rss, an_unbound_scan_map_getor_hit_runs_at_constant_rss, an_unbound_scan_map_getor_miss_runs_at_constant_rss, a_returned_getor_element_runs_at_constant_rss; items SHAPE_592_LIST_GETOR_HIT, SHAPE_592_LIST_GETOR_MISS, SHAPE_592_HASH_MAP_GETOR_HIT, SHAPE_592_HASH_MAP_GETOR_MISS, SHAPE_592_SCAN_MAP_GETOR_HIT, SHAPE_592_SCAN_MAP_GETOR_MISS, SHAPE_592_RETURNED [lines 4928-4966, 4968-4982, 4984-4996, 4998-5009, 5011-5025, 5027-5039, 5041-5060, 5127-5136, 5138-5147, 5149-5158, 5160-5169, 5171-5180, 5182-5191, 5193-5197]
- `tests/runtime/rt_scope_drop_leaks/get_contrasts.rs` (80): tests (3) a_bound_getor_element_still_runs_at_constant_rss, an_unbound_get_element_still_runs_at_constant_rss, a_borrowed_get_element_still_runs_at_constant_rss; items SHAPE_592_CONTRAST_BOUND_GETOR, SHAPE_592_CONTRAST_UNBOUND_GET, SHAPE_592_CONTRAST_BORROWED_GET [lines 5062-5076, 5078-5095, 5097-5125, 5199-5208, 5210-5219, 5221-5230]
- `tests/runtime/rt_scope_drop_leaks/unbound_collection_values.rs` (152): tests (2) every_unbound_collection_element_position_still_produces_the_right_value, a_borrowed_get_with_a_fresh_key_operand_runs_at_constant_rss; items SHAPE_592_VALUES, SHAPE_592_BORROWED_GET_WITH_A_FRESH_KEY [lines 5232-5302, 5304-5371, 5373-5403, 5405-5414]
- `tests/runtime/rt_scope_drop_leaks/trapped_error_retention.rs` (154): tests (3) a_trapped_error_bound_as_a_resource_is_not_retained, a_trapped_error_bound_as_an_address_list_is_not_retained, every_failing_resource_call_still_reports_its_error_and_origin; items b593_trapped_error_probe, b593_assert_error_not_retained, b593_long_message, b593_resource_probe, b593_address_list_probe, B593_VALUES, B593_VALUES_EXPECTED [lines 5472-5501, 5503-5520, 5522-5525, 5527-5537, 5539-5549, 5551-5559, 5561-5569, 5571-5618, 5620-5631, 5633-5660]
- `tests/runtime/rt_scope_drop_leaks/addresses.rs` (197): tests (5) every_address_reads_back_after_its_builder_scratch_is_freed, a_looped_net_lookup_runs_at_constant_rss, a_looped_local_address_runs_at_constant_rss, a_looped_user_built_address_list_runs_at_constant_rss, a_looped_udp_receive_runs_at_constant_rss; items SHAPE_599_ADDRESS_VALUES, SHAPE_599_LOOKUP_LOOP, SHAPE_599_LOCAL_ADDRESS_LOOP, SHAPE_599_USER_BUILT_LIST_LOOP, SHAPE_599_DATAGRAM_LOOP [lines 5664-5745, 5747-5786, 5790-5808, 5810-5823, 5825-5840, 5842-5858, 5860-5864, 5866-5875, 5877-5886, 5888-5897]
- `tests/runtime/rt_scope_drop_leaks/recursive_loops.rs` (168): tests (7) a_looped_recursive_union_bind_runs_at_constant_rss, a_looped_recursive_record_bind_runs_at_constant_rss, a_looped_recursive_reassignment_runs_at_constant_rss, a_looped_recursive_global_overwrite_runs_at_constant_rss, a_looped_unbound_recursive_temp_runs_at_constant_rss, a_looped_recursive_closure_capture_runs_at_constant_rss, a_looped_recursive_get_result_runs_at_constant_rss; items SHAPE_C_UNION_BIND, SHAPE_C_RECORD_BIND, SHAPE_C_REASSIGN, SHAPE_C_GLOBAL, SHAPE_C_UNBOUND_TEMP, SHAPE_C_CLOSURE, SHAPE_C_GET [lines 5901-5914, 5916-5932, 5934-5949, 5951-5966, 5968-5997, 5999-6017, 6019-6037, 6039-6048, 6050-6059, 6061-6065, 6067-6071, 6073-6082, 6084-6088, 6090-6094]
- `tests/runtime/rt_scope_drop_leaks/json_regex_trap_loops.rs` (193): tests (6) a_looped_inline_trap_union_bind_runs_at_constant_rss, a_looped_failing_call_with_a_temp_argument_runs_at_constant_rss, a_looped_trapped_recursive_json_parse_runs_at_constant_rss, a_repeated_json_parse_of_one_document_runs_at_constant_rss, a_repeated_regex_find_all_runs_at_constant_rss, a_looped_unbound_recursive_json_temp_runs_at_constant_rss; items SHAPE_C_JSON_REPEAT, SHAPE_C_REGEX_REPEAT, SHAPE_C_JSON_UNBOUND_TEMP, SHAPE_C_JSON_TRAP_RECOVER, SHAPE_TRAP_UNION_DEFAULT, SHAPE_FAILING_CALL_TEMP_ARGUMENT [lines 6098-6118, 6120-6140, 6142-6157, 6159-6182, 6184-6226, 6228-6265, 6267-6276, 6278-6287, 6289-6298, 6300-6304, 6306-6310, 6312-6321]
- `tests/runtime/rt_scope_drop_leaks/branch_exit_temps.rs` (101): tests (10) an_if_condition_temp_is_freed_when_the_branch_returns, a_block_if_condition_temp_is_freed_when_the_branch_returns, an_elseif_condition_temp_is_freed_when_the_branch_returns, an_if_condition_temp_is_freed_when_the_branch_returns_a_fresh_string, an_if_condition_record_temp_is_freed_on_exit_do, an_if_condition_temp_is_freed_on_exit_for, an_if_condition_temp_is_freed_on_exit_while, an_if_condition_temp_is_freed_on_continue_while, an_if_condition_temp_is_freed_on_continue_for, a_continue_out_of_a_nested_loop_frees_every_enclosing_condition_temp [lines 6386-6396, 6398-6408, 6410-6420, 6422-6433, 6435-6445, 6447-6457, 6459-6469, 6471-6481, 6483-6493, 6495-6506]
- `tests/runtime/rt_scope_drop_leaks/loop_condition_temps.rs` (143): tests (8) a_do_while_condition_temp_is_freed_on_every_pass, a_while_condition_temp_is_freed_on_every_pass, a_loop_until_condition_temp_is_freed_on_every_pass, a_while_condition_record_temp_is_freed_on_every_pass, condition_temps_that_already_had_an_owner_stay_flat, a_return_from_a_for_each_frees_its_fresh_collection, a_return_from_a_while_with_a_condition_temp_frees_every_block, a_resource_temporary_in_a_container_is_closed [lines 6508-6518, 6520-6530, 6532-6542, 6544-6554, 6556-6586, 6588-6607, 6609-6620, 6624-6676]
- `tests/runtime/rt_scope_drop_leaks/thread_results.rs` (211): tests (3) a_waited_for_thread_leaves_nothing_in_the_parent_arena, a_completed_thread_dropped_without_waitfor_leaves_nothing_in_the_parent_arena, a_thread_result_copy_is_freed_by_its_owner; items B622_INTEGER_RESULT, B622_COMPLETED_DROP, B622_STRING_BOUND, B622_STRING_UNBOUND, B622_STRING_TRAPPED, B622_UNION, B622_RECORD, B622_RECEIVE, B622_TRAPPED_QUERY [lines 6711-6725, 6727-6738, 6740-6752, 6754-6765, 6767-6779, 6781-6800, 6802-6822, 6824-6841, 6843-6860, 6862-6878, 7058-7072, 7074-7125]
- `tests/runtime/rt_scope_drop_leaks/thread_handle_moves.rs` (150): tests (2) a_moved_or_reassigned_thread_handle_is_freed_exactly_once, a_shared_thread_handle_stays_readable_and_is_freed_once; items B622_MOVED_TO_CALLEE, B622_REASSIGNED, B622_ALIAS, B622_MOVED_REUSE, B622_INLINE_TRAP_START, B622_TRAP_ROUTED [lines 6880-6894, 6896-6912, 6914-6936, 6938-6956, 6958-6974, 6976-6992, 6994-7015, 7017-7056]
Surface: none (no support file; root keeps every shared helper).
Sweeps: `a_resource_temporary_in_a_container_is_closed` (→ loop_condition_temps.rs) reads `concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml")` (line 6632) — absolute, unaffected. Many tests carry `#[cfg(unix)]` / `#[cfg(any(target_os = …))]`; the attributes move with them, and helpers with cfgs (peak_rss, assert_flat, condition_temp_report, …) keep theirs.

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

Every phase follows the same order: K1 on the unsplit tree, then split each file per its §4 entry,
then K2–K5, then commit the phase with its new files, its roots, the ceiling file, and `census.py`
when the phase adds an exempt row.

### Phase 1 — Package, parameter-default, place-forward and spawn tests (§4.1–§4.7)

Seven two-way splits with no support module and no hazard.
`T="rt_package_parameter_defaults rt_package_private_type_collision rt_package_public_export_name_collision rt_parameter_default_scope rt_top_level_initializer_globals rt_place_forward rt_process_spawn_ambient_fds"`,
`RE="rt_(package_parameter_defaults|package_private_type_collision|package_public_export_name_collision|parameter_default_scope|top_level_initializer_globals|place_forward|process_spawn_ambient_fds)"`.

- [ ] K1 → 60 tests.
- [ ] `rt_package_parameter_defaults.rs` → root + `computed_defaults.rs`, `literal_defaults.rs` (§4.1).
- [ ] `rt_package_private_type_collision.rs` → root + `private_types.rs`, `export_checks.rs` (§4.2).
- [ ] `rt_package_public_export_name_collision.rs` → root + `overloads.rs`, `importer_view.rs` (§4.3).
- [ ] `rt_parameter_default_scope.rs` → root + `evaluation.rs`, `scope.rs` (§4.4).
- [ ] `rt_top_level_initializer_globals.rs` → root + `top_level.rs`, `package_initializers.rs` (§4.5).
- [ ] `rt_place_forward.rs` → root + `locals.rs`, `globals_and_state.rs` (§4.6).
- [ ] `rt_process_spawn_ambient_fds.rs` → root + `inheritance.rs`, `signals_and_timeouts.rs` (§4.7).
- [ ] Tighten: K5 and commit the ceiling file.

Acceptance: 60 tests are listed unchanged and pass, and the seven paths leave the banner.
  Check: K2 → no output; K3 → 7 × ok; K4 → no output; K5 → 7 TIGHTEN rows, then none (est. 12 min).
Commit: —

### Phase 2 — Record update, recursive value, regex, sort and thread-arena tests (§4.8–§4.17)

Ten files, six of them with data-only support modules.
`T="rt_record_nested_update rt_record_update_forms rt_recursive_value_collection_drops rt_recursive_value_construction_copies rt_recursive_value_copies rt_recursive_value_drop_symmetry rt_regex_bounds rt_regex_span rt_sortby_copying_native rt_thread_block_arena"`,
`RE="rt_(record_nested_update|record_update_forms|recursive_value_collection_drops|recursive_value_construction_copies|recursive_value_copies|recursive_value_drop_symmetry|regex_bounds|regex_span|sortby_copying_native|thread_block_arena)"`.

- [ ] K1 → 44 tests.
- [ ] `rt_record_nested_update.rs` → root + support `cases.rs` (no `use super::*;`) + `allocation.rs`, `rebinding.rs` (§4.8).
- [ ] `rt_record_update_forms.rs` → root + support `cases.rs` (no `use super::*;`) (§4.9).
- [ ] `rt_recursive_value_collection_drops.rs` → root + `lists.rs`, `maps_and_types.rs` (§4.10).
- [ ] `rt_recursive_value_construction_copies.rs` → root + `construction.rs`, `copy_counts.rs` (§4.11).
- [ ] `rt_recursive_value_copies.rs` → root + `copies.rs`, `moves.rs`, `churn.rs` (§4.12).
- [ ] `rt_recursive_value_drop_symmetry.rs` → root + `user_types.rs`, `builtin_graphs.rs` (§4.13).
- [ ] `rt_regex_bounds.rs` → root + `group_repeats.rs`, `cost.rs`, support `corpus.rs` (no `use super::*;`), `semantics.rs` (§4.14).
- [ ] `rt_regex_span.rs` → root + `spans.rs`, support `crosscheck_corpus.rs` (no `use super::*;`), `crosscheck.rs` (§4.15).
- [ ] `rt_sortby_copying_native.rs` → root + `cost.rs`, support `types.rs` (no `use super::*;`), `key_fn.rs` (§4.16).
- [ ] `rt_thread_block_arena.rs` → root + `round_trips.rs`, `transfers.rs` (§4.17).
- [ ] Tighten: K5 and commit the ceiling file.

Acceptance: 44 tests are unchanged and pass.
  Check: K2 → no output; K3 → 10 × ok; K4 → no output (this catches a support file that wrongly
  kept `use super::*;`); K5 → 10 TIGHTEN rows, then none (est. 15 min).
Commit: —

### Phase 3 — Resource ownership tests (§4.18–§4.30)

Thirteen files: the 23 include repoints, one exempt row, and the stale `.ai` path fix.
`T="rt_res_list_from_call_leak rt_res_state_inplace_mutation rt_resource_aggregate_temporary rt_resource_drop_reclaims rt_resource_last_holder rt_resource_model rt_resource_review_805 rt_resource_review_806 rt_resource_store_float rt_resource_trap_state_free rt_resource_union_box_owner rt_resource_union_member_float rt_resource_union_reclaim"`,
`RE="rt_(res_list_from_call_leak|res_state_inplace_mutation|resource_aggregate_temporary|resource_drop_reclaims|resource_last_holder|resource_model|resource_review_805|resource_review_806|resource_store_float|resource_trap_state_free|resource_union_box_owner|resource_union_member_float|resource_union_reclaim)"`.

- [ ] K1 → 186 tests.
- [ ] `rt_res_list_from_call_leak.rs` → root + `take_drop.rs`, `user_function.rs`, `native_members.rs` (§4.18).
- [ ] `rt_res_state_inplace_mutation.rs` → root + `field_updates.rs`, `collection_ops.rs`, `splices.rs`, `operand_analysis.rs` (§4.19).
- [ ] `.ai/resources-packages.md:49` and `:176`: replace `tests/rt_res_state_inplace_mutation.rs`
      with `tests/runtime/rt_res_state_inplace_mutation.rs`. This is a pre-existing stale path
      (evidence: `ls tests/rt_res_state_inplace_mutation.rs` → No such file). Check:
      `grep -n 'tests/rt_res_state' .ai/resources-packages.md` → no output.
- [ ] `rt_resource_aggregate_temporary.rs` → root + `statement_drops.rs`, `positions.rs`, `takers.rs` (§4.20).
- [ ] `rt_resource_drop_reclaims.rs` → root + `elements.rs`, `unions.rs`, `lists.rs`, `calls.rs` (§4.21).
- [ ] `rt_resource_last_holder.rs` → root + seven topics (§4.22).
- [ ] Exempt row in `tests/guards/file_size_ceilings.txt` and `census.py`'s `EXEMPT`:
      `exempt test tests/runtime/rt_resource_last_holder/map_iteration.rs # one #[test]: nine sequential check_at_both_levels calls; splitting it changes the test count`.
- [ ] `rt_resource_model.rs` → root + `bugs_1018_to_1067.rs`, `bugs_1069_to_1078.rs`, `bugs_999.rs`
      (§4.23). Every `include_str!("../../bugs/repro/…")` becomes `include_str!("../../../bugs/repro/…")`,
      including the three-line one in `bug_1078_close_only_drain_kept_beside_an_unraised_outward_store`.
      Check: `grep -c '"\.\./\.\./\.\./bugs/repro/' tests/runtime/rt_resource_model/*.rs` sums to `23`,
      and `grep -rn '"\.\./\.\./bugs/repro/' tests/runtime/rt_resource_model/` → no output.
- [ ] `rt_resource_review_805.rs` → root + `nested_returns.rs`, `mixed_transfers.rs`, `ownership_edges.rs`, `leaks.rs` (§4.24).
- [ ] `rt_resource_review_806.rs` → root + `globals.rs`, `teardown.rs`, `stores.rs` (§4.25).
- [ ] `rt_resource_store_float.rs` → root + `inserting_stores.rs`, `scopes.rs` (§4.26).
- [ ] `rt_resource_trap_state_free.rs` → root + `bindings.rs`, `exits.rs` (§4.27).
- [ ] `rt_resource_union_box_owner.rs` → root + `wrap_sites.rs`, `bindings.rs`, `element_reads.rs` (§4.28).
- [ ] `rt_resource_union_member_float.rs` → root + `lists.rs`, `maps_and_scopes.rs` (§4.29).
- [ ] `rt_resource_union_reclaim.rs` → root + `iteration.rs`, `trapped_bindings.rs`, `record_bindings.rs` (§4.30).
- [ ] Tighten: K5 and commit the ceiling file, `census.py` and `.ai/resources-packages.md`.

Acceptance: 186 tests are unchanged and pass, the repro includes resolve, the one exempt row is
present, and the `.ai` path is real.
  Check: K2 → no output; K3 → 13 × ok; K4 → no output; K5 → 13 TIGHTEN rows, then only
  `exempt test tests/runtime/rt_resource_last_holder/map_iteration.rs …` matches `$RE` in the
  ceiling file (est. 25 min: thirteen targets, each building resource programs).
Commit: —

### Phase 4 — rt_self_update_cost (§4.31)

One file with the second exempt row.
`T="rt_self_update_cost"`, `RE="rt_(self_update_cost)"`.

- [ ] K1 → 13 tests.
- [ ] `rt_self_update_cost.rs` → root + `linearity.rs`, support `scale_program.rs` and
      `value_program.rs` (both without `use super::*;`), `value_semantics.rs`, `list_churn.rs`,
      `map_churn.rs`, `small_churn.rs` (§4.31).
- [ ] Exempt row in the ceiling file and `census.py`'s `EXEMPT`:
      `exempt test tests/runtime/rt_self_update_cost/value_program.rs # one const VALUE_PROGRAM MFBASIC program consumed whole by value_semantics.rs; no split without editing the program or its consumer`.
- [ ] Tighten: K5 and commit the ceiling file and `census.py`.

Acceptance: 13 tests are unchanged and pass.
  Check: K2 → no output; K3 → ok; K4 → no output; K5 → 1 TIGHTEN row, then only the `exempt` row
  matches (est. 10 min).
Commit: —

### Phase 5 — rt_scope_drop_leaks (§4.32)

The largest split in the letter: 154 tests into 35 topics. It goes last because a missed `#[path]`
here drops the most tests.
`T="rt_scope_drop_leaks"`, `RE="rt_(scope_drop_leaks)"`.

- [ ] K1 → 154 tests.
- [ ] `rt_scope_drop_leaks.rs` → root (keeps `peak_rss`, `assert_flat`, `B561_TRAP_ALWAYS_FAILS`,
      `B573_BUILTIN_RAISE`, `condition_temp_report`, `assert_condition_temps_freed`,
      `thread_loop_report` and `thread_loop_growth`, with their cfg attributes) + the 35 topics of
      §4.32. Every test's and constant's `#[cfg(…)]` moves with it.
- [ ] Declaration check: `grep -c '^#\[path = "rt_scope_drop_leaks/' tests/runtime/rt_scope_drop_leaks.rs` → `35`,
      and `ls tests/runtime/rt_scope_drop_leaks/*.rs | wc -l` → `35`.
- [ ] Tighten: K5 and commit the ceiling file.

Acceptance: all 154 tests are listed under the same names and pass.
  Check: K2 → no output; K3 → ok; K4 → no output; K5 → 1 TIGHTEN row, then none (est. 25 min: the
  target runs 154 RSS-flatness programs; this is the run of exactly the moved tests).
Commit: —

## Validation Plan

- Tests: none are added. Every moved test runs in its phase's K3. Ignored tests are covered by the
  K1/K2 name comparison.
- Coverage check: `tests/` is outside the coverage denominator (`scripts/coverage-common.sh`
  IGNORE), and no compiler code changes.
- Runtime proof: each phase's K3.
- Doc sync: `.ai/resources-packages.md` (stale path fixed, Phase 3). No spec or man topic changes,
  and no `mfb spec` contract changes.
- Final gate (once, after Phase 5):
  `cargo test --test file_size_budget 2>&1 | grep -E 'tests/runtime/rt_(package_parameter_defaults|package_private_type_collision|package_public_export_name_collision|parameter_default_scope|place_forward|process_spawn_ambient_fds|record_nested_update|record_update_forms|recursive_value_[a-z_]+|regex_bounds|regex_span|res_list_from_call_leak|res_state_inplace_mutation|resource_[a-z0-9_]+|scope_drop_leaks|self_update_cost|sortby_copying_native|thread_block_arena|top_level_initializer_globals)(\.rs|/)'`
  → no output. The same regex over the ceiling file → exactly the two `exempt` lines of C (B's
  rows do not match it). `cargo test --test test_targets_registered` → ok (est. 3 min). The full
  workspace suite runs once, in plan-176-V.

## Decisions

- Rule: code ≤ 1000, test ≤ 250 non-comment, non-blank LOC, over `.rs`/`.mfb` under `src/`,
  `packages/`, `examples/`, `repository/` and `tests/` — owner, 2026-10-14.
- Inline test code counts against 250; generated files are exempt; the guard warns and never fails;
  a new module is always an acceptable split — owner, 2026-10-14.
- A single test-side unit over 250 gets its own file and an `exempt` row with its reason, in both
  the ceiling file and `census.py`'s `EXEMPT`, in the same commit — owner, 2026-10-14.
- C's exempt files are `tests/runtime/rt_resource_last_holder/map_iteration.rs` (one `#[test]`) and
  `tests/runtime/rt_self_update_cost/value_program.rs` (one program const) — author, 2026-10-14.
- Data-only support files omit `use super::*;` (an unused glob warns) — author, 2026-10-14.
- Glob-cycle imports compile without warnings (`/tmp/loc/probe2` printed `6`, no warning) — verified.
- Pre-existing defects found by the map are fixed in the letter that touches them: the
  `.ai/resources-packages.md` stale path — AGENTS.md "never leave a bug", 2026-10-14.

## Corrections

## Summary

The risk is a topic that is never compiled, chiefly in rt_scope_drop_leaks' 35 topics. The
name-list diff per target closes that, and so do the declaration count and one run of exactly the
moved tests. The include repoints fail at compile time, never silently. Nothing outside
`tests/runtime/`, the ceiling file, `census.py`'s `EXEMPT` and two lines of
`.ai/resources-packages.md` changes.
