# plan-176-C: Split tests/runtime, part 2 (3 integration-test files)

Last updated: 2026-10-18
Effort: medium (1h–2h)
Depends on: plan-176-B

This sub-plan splits three test-only integration-test files under `tests/runtime/` that are over
the 1000-LOC test-only-file limit: `rt_resource_last_holder.rs`, `rt_self_update_cost.rs` and
`rt_scope_drop_leaks.rs`. Together they hold 8,289 test LOC and 181 `#[test]` fns
(`python3 -I /tmp/loc/census.py "$PWD"` at `ce7a995ca`, field `test`; test counts by
`grep -cE '#\[test\]' <file>`). The method is the one B uses. Each root stays the `[[test]]` target
and gains parts under `tests/runtime/<name>/`, declared with `#[path]`. It is pure code motion: no
test is added, removed or renamed, no assertion changes, and no file is read by a path that moves.
C also fixes one pre-existing stale `.ai` path (§4.0).

**Single outcome:** after C lands, `cargo test --test file_size_budget` prints no row for any path
under `tests/runtime/` that this letter owns. `tests/guards/file_size_ceilings.txt` has no row for
them, and every file C creates or leaves is ≤ 900 test LOC.

References:

- `planning/plan-176-A-file-size-guard.md`: the rule (three kinds: `code` ≤ 1000, `inline` ≤ 250,
  `test` ≤ 1000), the guard, the `ceiling <code|inline|test> <n> <path>` format, the tighten
  ratchet, and the `#[path]` layout verified in its §2.
- `planning/plan-176-B-tests-runtime-a.md`: the same conventions and gate kit (K1–K5) on the
  first half of `tests/runtime/`. C restates them here so it can be executed on its own.
- `.ai/build-tooling.md` § "Splitting an over-budget file (plan-176)".
- `.ai/collections.md`: `:47` and `:75` name `rt_self_update_cost`, and `:542` names the test
  `rt_self_update_cost::a_churning_map_or_set_does_not_grow_without_bound`. All stay valid, because
  that test stays in the root (§4.2).
- `.ai/resources-packages.md` (:49, :176): stale path, fixed in Phase 1 (§4.0).
- Spec topics: none. No `[[tests/runtime/<file>.rs:…]]` citation names a C file
  (`grep -rnoE '\[\[tests/runtime/(rt_resource_last_holder|rt_self_update_cost|rt_scope_drop_leaks)\.rs[^]]*\]\]' src/docs .ai` → 0 hits).

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-176-B is complete and archived | `ls planning/plan-176-B-* planning/completed/plan-176-B-*` → only `planning/completed/plan-176-B-tests-runtime-a.md` | MET (2026-10-18 at `bdabb4517`: `planning/plan-176-B-*` no match; `planning/completed/plan-176-B-tests-runtime-a.md` present) |
| The guard and the oracle exist (plan-176-A) | `cargo test --test file_size_budget` → passes; `python3 -I tools/loc-census/census.py --file tests/runtime/rt_scope_drop_leaks.rs` → `0 5476` | MET (2026-10-18: `test result: ok. 35 passed`; `0 5476`) |
| This letter's files have no uncommitted changes | `git status --porcelain -- tests/runtime .ai/resources-packages.md tests/guards/file_size_ceilings.txt` → empty | MET (2026-10-18: empty output) |

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

- Every file of this letter, and every file it creates, is ≤ 1000 test LOC by
  `python3 -I tools/loc-census/census.py --file <path>`, and each new part is ≤ 900 (the headroom
  target). No exemption exists.
- For each of the 3 targets, the sorted list of test names from `-- --list`, with module prefixes
  stripped, is identical before and after. Every target passes `cargo test --test <t>`.
- `.ai/resources-packages.md` names the real path of `rt_res_state_inplace_mutation.rs`.

### Non-goals (explicit constraints)

- No behavior change. No test body, assertion, MFBASIC program text, expected output, `#[cfg]` or
  `#[ignore]` changes. C adds no `pub(super)` (§3), so moved code has no text edits.
- No `[[test]]` target added, renamed or re-pathed. `Cargo.toml` is not edited.
- No test fn renamed. Only a module prefix is added to test paths, and substring filters keep working.
- No helper is promoted into `tests/common/`, and no test moves between targets.

## 2. Current State

### Files and measured sizes

Test LOC are from the planning oracle at `ce7a995ca` (`python3 -I /tmp/loc/census.py "$PWD"` →
`/tmp/loc/census.json`, field `test`; after A it is
`python3 -I tools/loc-census/census.py --file <path>`). Test counts are `grep -cE '#\[test\]' <file>`;
no file uses another test attribute, and none has an `#[ignore]` (`grep -c '^#\[ignore' <file>` → `0`
for all three). The three files are unchanged since `17a9b5799`
(`git diff --stat 17a9b5799 HEAD -- <the 3 files>` → no output), so the line ranges below hold
at both commits.

| File | Test LOC | Tests | New files | Largest file after |
|---|---|---|---|---|
| `tests/runtime/rt_resource_last_holder.rs` | 1476 | 14 | 1 | 819 (root) |
| `tests/runtime/rt_self_update_cost.rs` | 1337 | 13 | 1 | 773 (root) |
| `tests/runtime/rt_scope_drop_leaks.rs` | 5476 | 154 | 6 | 878 (`trap_binds_hofs_and_for_each.rs`) |
| **Total (3 files)** | **8,289** | **181** | **8** | |

Part sizes come from `python3 -I /tmp/loc/span.py <file> <a> <b>` summed over each part's ranges,
plus the lines the split adds (§3). The per-part sums were re-run in `/tmp/loc/rw-B/` (`c.py`,
`sdl.py`) and each equals the sum of the topic sizes in `/tmp/loc/maps/C.md`, minus one
`use super::*;` per merged topic.

### Verified properties

- **Layout, globs and warnings.** These are the same facts as plan-176-B §2, from the same probes.
  The `#[path]` children list as `<part>::<test>` (plan-176-A probe). Root imports, `common`
  helpers and private root items used only by a part produce no warning (`/tmp/loc_bcd_probe`:
  `test result: ok. 1 passed`, no warning). Glob cycles are clean (`/tmp/loc/probe2` printed `6`).
- **Every C test compiles on the macOS development host.**
  `grep -nE 'cfg\((target_os = "(linux|windows)"|windows|not\(unix\))' <the 3 files>` → no hit.
  rt_scope_drop_leaks' 23 `#[cfg(any(target_os = "macos", target_os = "linux"))]` gates
  (`grep -c`) include macOS, and its 133 `#[cfg(unix)]` lines and rt_self_update_cost's
  `#![cfg(unix)]` hold on macOS. So `-- --list` on the development host counts every test, and no
  remote box is needed.
- **No C file reads a file by a path that moves.** `grep -n 'include_str\|CARGO_MANIFEST_DIR' <the 3 files>`
  → only `rt_scope_drop_leaks.rs:6632`, `concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml")`.
  That path is absolute, and the line moves into `loops_and_condition_temps.rs` unchanged.
- **No C file is self-reading, `--exact`-run, in `scripts/coverage-exceptions.txt`, or in a
  `tests/guards` allowlist** (`grep -rn -- --exact .github scripts`; `grep -n <name>
  scripts/coverage-exceptions.txt tests/guards/{architecture_guards,no_type_strings,no_operator_strings,inplace_self_update_census}.rs`
  → no hit). `tests/guards/test_targets_registered.rs` reads only direct `.rs` children of each
  `tests/<dir>/` (plan-176-A §2). No `tests/runtime/<name>/` directory exists for any C stem.
- **The `.ai/resources-packages.md` path is stale today.** Lines 49 and 176 name
  `tests/rt_res_state_inplace_mutation.rs`, and `ls tests/rt_res_state_inplace_mutation.rs` →
  `No such file or directory`. The file is `tests/runtime/rt_res_state_inplace_mutation.rs`
  (`grep -n 'rt_res_state' .ai/resources-packages.md` → :49, :176, re-run 2026-10-18).
- **Section banners are comment-only.** Each gap between two parts' code ranges in
  rt_scope_drop_leaks holds 0 counted LOC (`span.py` over 3790-3804, 4001-4056, 4229-4242,
  5415-5471, … → 0). Those gaps are `// ---- bug-NNN …` banners and blank lines. A banner moves
  with the code that follows it, and the §4 ranges are written to include it.

## 3. Design Overview

**Conventions (every file)** are plan-176-B §3's conventions:

- The root stays the `[[test]]` target. It keeps its doc, `#![cfg]`,
  `#[path = "../common/mod.rs"] mod common;`, its `use` lines, the shared helpers, and the topics
  §4 leaves in it. It gains one `#[path = "<name>/<m>.rs"] mod <m>;` pair per new file (2 LOC).
- Parts are regrouped for the ≤ 900 headroom target with as few files as possible. Neighbouring
  topics are merged in source order, and the root keeps the topics nearest its shared helpers.
- Every part starts with `use super::*;` and needs no visibility edits. Every item a part owns is
  used only inside it, and everything it borrows is a root item, which children can see. C has no
  support file, so there is no `pub(super)` and no root glob. In particular, `SCALE_PROGRAM` and
  `VALUE_PROGRAM` now live in the files of their only consumers (§4.2).
- Code moves verbatim, with doc comments, `#[cfg]` and `#[ignore]` attributes, in source order.
  Ranges are as of `ce7a995ca` (identical at `17a9b5799`), and item and test names are
  authoritative if lines drift.

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
  `TIGHTEN` rows (one per root, each reporting its `ceiling test` entry as removable), with no
  NEW/GREW/STALE row. Then `MFB_LOC_TIGHTEN=1 cargo test --test file_size_budget`.
  Then `git diff -U0 tests/guards/file_size_ceilings.txt | grep -E '^[-+]ceiling' | grep -vE "tests/runtime/$RE(\.rs|/)"`
  → no output. Re-running the first grep → no output.

**Where the risk concentrates.** rt_scope_drop_leaks is the riskiest file. It has 154 tests in 6
parts plus the root, so a missed `#[path]` silently drops up to 36 tests. It goes last, alone,
behind K1/K2 and a declaration count. Many rt_scope_drop_leaks tests and helpers carry
`#[cfg(unix)]` or `#[cfg(any(target_os = …))]` attributes. Those attributes move with their
items. The cfg-gated helpers that stay in the root keep theirs: `peak_rss`, `assert_flat`,
`condition_temp_report`, `assert_condition_temps_freed`, `thread_loop_report` and
`thread_loop_growth`.

**Rejected alternatives.** These are the same as plan-176-B §3:
- no helper promotion into `tests/common/` (letter E owns it);
- no per-topic `[[test]]` targets (that renames targets used by docs and CI);
- the root does not move into its directory (`test_targets_registered.rs` requires root = target
  name);
- no one-file-per-topic layout (it adds `#[path]` pairs and buys nothing under a 1000-LOC limit).

## 4. Detailed Design

### 4.0 Sweeps common to every file

- Spec citations to C files: 0 (§References). No cited path moves, so `spec_citations_resolve`
  is unaffected.
- `Cargo.toml` stanzas (`:1148`, `:1284`, `:1536`) and the `.ai/collections.md` references name
  root paths, targets or root-resident tests, and all stay valid.
- Pre-existing stale path, fixed in Phase 1: `.ai/resources-packages.md:49` and `:176` name
  `tests/rt_res_state_inplace_mutation.rs`, a path that does not exist (§2). Both become
  `tests/runtime/rt_res_state_inplace_mutation.rs`. That file needs no split under the current
  rule (651 test LOC, `/tmp/loc/census.json`), but the doc defect stands on its own. AGENTS.md
  says never leave a bug you found, and this letter is the one that found it.
- Each entry gives the layout (path, LOC, contents, source ranges), then "Surface", then "Sweeps".

#### 4.1 `tests/runtime/rt_resource_last_holder.rs` — 1476 test LOC, 14 tests

- `tests/runtime/rt_resource_last_holder.rs` (819): header; helpers prelude, Run, run, measure,
  check, check_at_both_levels, check_built; and 11 tests, as follows:
  - bindings and copies (6): a_handle_bound_on_its_own_leaves_the_record_to_its_owner, a_record_returned_after_a_copy_is_freed_by_the_caller, a_record_captured_by_a_reading_lambda_is_freed, a_list_whose_element_a_callee_stores_elsewhere_frees_its_block, a_record_whose_copy_is_appended_to_a_list_is_freed, a_resource_union_and_a_map_follow_the_same_rule.
  - exits (2): a_loop_body_frees_on_every_way_out, an_error_exit_frees_what_the_scope_took.
  - reads (2): two_cleanups_that_name_one_record_free_it_once, a_list_read_through_get_or_copied_by_a_callee_is_freed.
  - callee copies (1): a_copy_of_a_record_returned_by_a_callee_is_freed_at_every_level.

  Lines: 1-893. Mod: trapped_and_maps.
- `tests/runtime/rt_resource_last_holder/trapped_and_maps.rs` (660): tests (3)
  a_trapped_call_that_returns_its_argument_frees_the_default_record,
  a_trapped_element_read_does_not_keep_the_list_block,
  a_map_of_resources_iterated_through_its_values_frees_its_block [lines 894-1583].

Surface: none (no support file; root keeps every shared helper).
Sweeps: none (§4.0 commands: 0 hits).
Indivisible-unit disposition: `a_map_of_resources_iterated_through_its_values_frees_its_block`
(one 284-LOC `#[test]`) is far under the 1000 test-only-file limit. It is not split and needs no
exemption.

#### 4.2 `tests/runtime/rt_self_update_cost.rs` — 1337 test LOC, 13 tests

- `tests/runtime/rt_self_update_cost.rs` (773): header; consts ROUNDS, MAX_RATIO, SLACK_SECONDS;
  helpers expected, counter; and 11 tests, as follows:
  - linearity (7): remove_at_near_the_end_of_a_variable_width_list_is_linear, remove_key_and_set_remove_are_linear, difference_with_a_small_argument_is_linear, symmetric_difference_with_a_small_argument_is_linear, string_window_self_updates_that_keep_the_start_are_linear, truncating_an_out_of_order_variable_width_list_is_linear, append_stays_linear; items SCALE_PROGRAM, scale_exe, cpu_run, assert_linear.
  - list churn (1): a_churning_list_does_not_grow_without_bound; item CHURN_PROGRAM.
  - map churn (2): a_churning_map_or_set_does_not_grow_without_bound, a_map_reclaim_terminates_when_moved_values_pack_tighter; items MAP_CHURN_PROGRAM, MAP_MOVED_VALUES_PROGRAM.
  - small churn (1): a_small_list_under_length_changing_churn_stays_small; items SMALL_CHURN_PROGRAM, run_with_deadline.

  Lines: 1-219, 730-1223, 1262-1270, 1301-1478. Mod: value_semantics.
- `tests/runtime/rt_self_update_cost/value_semantics.rs` (567): tests (2)
  removals_and_windows_keep_value_semantics, removals_free_every_removed_element_once; items
  VALUE_PROGRAM, VALUE_OUTPUT, without_globals [lines 220-729, 1224-1261, 1271-1300].

Surface: none. `SCALE_PROGRAM`'s only reader is `scale_exe` (line 1071, root), and
`VALUE_PROGRAM`'s only readers are the two value tests (lines 1227 and 1278, part)
(`grep -n 'SCALE_PROGRAM\|VALUE_PROGRAM' tests/runtime/rt_self_update_cost.rs`). Each const sits
beside its readers and keeps its private visibility. `counter` stays in the root, because both the
root's churn tests and the part's value test use it.
Sweeps:
- `.ai/collections.md:47,75` name the root path and target, which are unchanged.
- `.ai/collections.md:542` names `rt_self_update_cost::a_churning_map_or_set_does_not_grow_without_bound`.
  That test stays in the root, so its path has no module prefix and the citation stays exact.

Indivisible-unit disposition: `VALUE_PROGRAM` (one 444-LOC MFBASIC program const) is under the
limit. It moves whole, beside its consumer, with no exemption.

#### 4.3 `tests/runtime/rt_scope_drop_leaks.rs` — 5476 test LOC, 154 tests

The 35 topics of the map are merged, in source order, into six parts and the root. Each part's
LOC is the `span.py` sum over its ranges + 1 (`use super::*;`), and equals the sum of its topics
in `/tmp/loc/maps/C.md` minus one per merged topic.

- `tests/runtime/rt_scope_drop_leaks.rs` (517): header; helpers peak_rss, assert_flat,
  B561_TRAP_ALWAYS_FAILS, B573_BUILTIN_RAISE, condition_temp_report,
  assert_condition_temps_freed, thread_loop_report, thread_loop_growth. These keep their cfg
  attributes. The root also keeps 5 tests [lines 1-69, 1253-1282, 2703-2730, 6325-6384,
  6677-7125]:
  - thread results (3): a_waited_for_thread_leaves_nothing_in_the_parent_arena, a_completed_thread_dropped_without_waitfor_leaves_nothing_in_the_parent_arena, a_thread_result_copy_is_freed_by_its_owner; items B622_INTEGER_RESULT, B622_COMPLETED_DROP, B622_STRING_BOUND, B622_STRING_UNBOUND, B622_STRING_TRAPPED, B622_UNION, B622_RECORD, B622_RECEIVE, B622_TRAPPED_QUERY.
  - thread handle moves (2): a_moved_or_reassigned_thread_handle_is_freed_exactly_once, a_shared_thread_handle_stays_readable_and_is_freed_once; items B622_MOVED_TO_CALLEE, B622_REASSIGNED, B622_ALIAS, B622_MOVED_REUSE, B622_INLINE_TRAP_START, B622_TRAP_ROUTED.

  Mods: return_and_string_shapes, trap_binds_hofs_and_for_each, closures_errors_and_concats,
  helper_results_and_raised_errors, unbound_results_and_gets, loops_and_condition_temps.
- `tests/runtime/rt_scope_drop_leaks/return_and_string_shapes.rs` (831): 23 tests from 6 topics [lines 70-1169]:
  - return shapes (4): returning_a_record_constructor_runs_at_constant_rss, returning_a_fresh_call_result_runs_at_constant_rss, returning_an_owned_local_still_runs_at_constant_rss, every_return_shape_still_produces_the_right_value; items SHAPE_A_CONSTRUCTOR, SHAPE_A_CALL, SHAPE_A_CONTRAST_LOCAL, SHAPE_A_BEHAVIOUR.
  - string producers (5): an_unbound_string_call_result_runs_at_constant_rss, an_unbound_string_concat_runs_at_constant_rss, unbound_native_string_producers_run_at_constant_rss, a_bound_string_call_result_still_runs_at_constant_rss, every_string_producer_still_produces_the_right_value; items SHAPE_B_CALL_RESULT, SHAPE_B_CONCAT, SHAPE_B_NATIVE_PRODUCERS, SHAPE_B_CONTRAST_BOUND, SHAPE_B_BEHAVIOUR.
  - user string returns (4): an_unbound_user_string_call_result_runs_at_constant_rss, a_transitive_user_string_call_chain_runs_at_constant_rss, a_returned_string_literal_runs_at_constant_rss, tostring_of_a_fresh_string_runs_at_constant_rss; items SHAPE_B2_APPEND_ARGUMENT, SHAPE_B2_TRANSITIVE, SHAPE_B2_RETURNED_LITERAL, SHAPE_B2_TOSTRING_IDENTITY.
  - callback callees (4): a_callback_invoked_through_a_user_function_returns_an_owned_block, a_direct_call_to_a_callback_referenced_string_callee_runs_at_constant_rss, a_direct_call_to_a_plain_string_callee_still_runs_at_constant_rss, a_param_borrow_string_callee_still_runs_at_constant_rss; items SHAPE_B2_PARAM_BORROW_CONTRAST, SHAPE_562_CALLBACK_REFERENCED_DIRECT_CALL, SHAPE_562_PLAIN_DIRECT_CALL_CONTRAST, SHAPE_562_INDIRECT_CALLABLE.
  - string return values (1): every_string_return_shape_still_produces_the_right_value; items SHAPE_B2_BEHAVIOUR.
  - string self append (5): a_reassigned_string_self_append_runs_at_constant_rss, a_returned_string_self_append_runs_at_constant_rss, the_utf32_decoder_runs_at_constant_rss, a_plain_string_self_append_still_runs_at_constant_rss, every_string_self_append_shape_still_produces_the_right_value; items B560_REASSIGNED_SELF_APPEND, B560_RETURNED_SELF_APPEND, B560_DECODER, B560_CONTRAST_PLAIN, B560_BEHAVIOUR.
- `tests/runtime/rt_scope_drop_leaks/trap_binds_hofs_and_for_each.rs` (878): 22 tests from 6 topics [lines 1170-1252, 1283-2309]:
  - trap bound results (6): a_trap_bound_string_result_runs_at_constant_rss, a_trap_bound_collection_result_runs_at_constant_rss, a_trap_bound_inline_builtin_result_runs_at_constant_rss, a_trap_whose_call_always_fails_still_produces_the_right_value, a_trap_bound_scalar_result_still_runs_at_constant_rss, the_same_producers_without_a_trap_still_run_at_constant_rss; items B561_TRAP_STRING, B561_TRAP_LIST, B561_TRAP_INLINE_BUILTIN, B561_CONTRAST_SCALAR, B561_CONTRAST_NO_TRAP.
  - trap bound values (1): every_trap_bound_result_still_produces_the_right_value; items B561_BEHAVIOUR.
  - hof callbacks (6): transform_runs_at_constant_rss_with_a_string_callback, sort_by_runs_at_constant_rss_with_a_string_key, group_by_runs_at_constant_rss_with_a_string_value, map_values_runs_at_constant_rss_with_a_string_callback, a_bare_parameter_callback_runs_at_constant_rss, a_fixed_width_callback_stays_flat; items SHAPE_569_TRANSFORM, SHAPE_569_SORT_BY, SHAPE_569_GROUP_BY, SHAPE_569_MAP_VALUES, SHAPE_569_BARE_PARAM_CALLBACK, SHAPE_569_FIXED_WIDTH_CONTRAST.
  - hof callback frees (1): every_hof_frees_its_callback_result_exactly_once.
  - for each items (7): a_for_each_over_a_list_of_string_does_not_leak_its_element, a_for_each_over_a_map_of_string_does_not_leak_either_side, a_for_each_over_a_set_of_string_does_not_leak_its_element, a_for_each_over_fixed_width_elements_stays_flat, an_early_exit_from_a_for_each_does_not_leak_the_item, returning_the_loop_item_neither_leaks_nor_double_frees, a_for_each_body_that_uses_the_item_does_not_leak_it; items SHAPE_571_LIST_OF_STRING, SHAPE_571_MAP_OF_STRING, SHAPE_571_SET_OF_STRING, SHAPE_571_CONTRAST_MAP_OF_INTEGER, SHAPE_571_CONTRAST_LIST_OF_INTEGER, SHAPE_571_EXIT_FOR, SHAPE_571_CONTINUE_FOR, SHAPE_571_RETURN_ITEM, SHAPE_571_USES_THE_ITEM.
  - for each values (1): every_for_each_body_shape_still_produces_the_right_value.
- `tests/runtime/rt_scope_drop_leaks/closures_errors_and_concats.rs` (867): 24 tests from 5 topics [lines 2310-2702, 2731-3505]:
  - closure arguments (5): a_capturing_lambda_argument_does_not_leak_its_environment, a_captureless_lambda_argument_stays_flat, every_admitted_callback_position_frees_its_closure, a_retained_callback_position_is_freed_by_its_owners, every_closure_argument_shape_still_produces_the_right_value; items SHAPE_572_CAPTURING_FILTER, SHAPE_572_CONTRAST_CAPTURELESS, SHAPE_572_REDUCE, SHAPE_572_FOREACH_BY_REF, SHAPE_572_MAP_VALUES, SHAPE_572_RETAINED_ROUTE.
  - trapped errors (7): the_same_trap_whose_call_never_fails_runs_at_constant_rss, a_trap_whose_call_always_fails_runs_at_constant_rss, a_trapped_error_read_by_its_handler_runs_at_constant_rss, a_failing_call_inside_a_collection_walk_runs_at_constant_rss, an_auto_propagated_error_out_of_a_loop_body_runs_at_constant_rss, an_inline_builtins_own_domain_error_runs_at_constant_rss, every_trapped_error_shape_still_produces_the_right_value; items B565_TRAP_ALWAYS_FAILS, B565_HANDLER_READS_ERROR, B565_FAILING_CALL_IN_A_WALK, B565_AUTO_PROPAGATED, B565_CONTROL_NEVER_FAILS, B565_ERROR_SHAPES.
  - param borrows (5): a_trap_over_a_param_returning_callee_runs_at_constant_rss, a_trap_over_every_scalar_payload_runs_at_constant_rss, a_trap_over_a_param_returning_string_callee_runs_at_constant_rss, a_param_borrow_without_a_trap_still_runs_at_constant_rss, every_param_borrow_shape_still_produces_the_right_value; items B568_TRAP_PARAM_BORROW, B568_TRAP_EVERY_SCALAR, B568_TRAP_STRING_BORROW, B568_CONTRAST_NO_TRAP, B568_BORROW_SHAPES.
  - returned concats (6): returning_a_nested_concat_runs_at_constant_rss, returning_a_concat_of_a_call_result_runs_at_constant_rss, returning_a_two_operand_concat_still_runs_at_constant_rss, returning_a_collection_built_from_interior_concats_runs_at_constant_rss, returning_an_interior_temp_beside_a_live_local_runs_at_constant_rss, a_failing_trap_does_not_leak_its_interior_temp; items SHAPE_567_NESTED_CONCAT, SHAPE_567_CONCAT_OF_A_CALL, SHAPE_567_CONTRAST_TWO_OPERANDS, SHAPE_567_RETURNED_LIST_OF_CONCATS, SHAPE_567_INTERIOR_TEMP_WITH_A_LIVE_LOCAL, SHAPE_567_FAIL_WITH_AN_INTERIOR_TEMP, SHAPE_567_FAIL_CONTRAST_NO_INTERIOR.
  - returned concat values (1): every_returned_concat_shape_still_produces_the_right_value.
- `tests/runtime/rt_scope_drop_leaks/helper_results_and_raised_errors.rs` (768): 19 tests from 5 topics [lines 3506-4525]:
  - runtime helper results (4): a_trapped_runtime_helper_string_result_grows_no_faster_than_the_plain_call, a_trapped_runtime_helper_collection_result_grows_no_faster_than_the_plain_call, a_trapped_runtime_helper_scalar_result_was_never_the_leak, every_trapped_runtime_helper_result_still_produces_the_right_value; items assert_no_extra_growth, SHAPE_566_TRAPPED_READ_TEXT, SHAPE_566_PLAIN_READ_TEXT, SHAPE_566_TRAPPED_READ_BYTES, SHAPE_566_PLAIN_READ_BYTES, SHAPE_566_TRAPPED_SCALAR, SHAPE_566_PLAIN_SCALAR.
  - helper scratch (8): a_short_path_argument_runs_at_constant_rss, a_long_path_argument_runs_at_constant_rss, a_long_path_argument_in_a_local_runs_at_constant_rss, a_long_environment_name_runs_at_constant_rss, a_call_marshalling_two_arguments_runs_at_constant_rss, a_helper_with_no_argument_but_its_own_buffer_runs_at_constant_rss, a_helper_that_allocates_only_its_result_stays_flat, a_tls_connect_host_name_is_not_leaked_in_proportion_to_its_length; items SHAPE_574_SHORT_PATH, SHAPE_574_LONG_PATH, SHAPE_574_LOCAL_PATH, SHAPE_574_ENV_NAME, SHAPE_574_TWO_ARGUMENTS, SHAPE_574_NO_ARGUMENT, SHAPE_574_CONTRAST_ARCH, tls_connect_probe.
  - helper scratch values (1): every_helper_whose_scratch_is_released_still_produces_the_right_value; items SHAPE_574_VALUES.
  - error filename depth (1): a_raised_error_runs_at_constant_rss_however_long_its_filename_is; items peak_rss_at_depth, DEEP_SOURCE_DIRECTORY.
  - raised errors (5): a_user_fail_runs_at_constant_rss, a_propagated_error_runs_at_constant_rss, a_runtime_helper_error_runs_at_constant_rss, a_loop_that_raises_nothing_still_runs_at_constant_rss, every_raised_error_still_reports_its_true_origin; items B573_USER_FAIL, B573_PROPAGATED, B573_HELPER_ERROR, B573_CONTRAST_NO_ERROR, B573_ORIGIN_VALUES.
- `tests/runtime/rt_scope_drop_leaks/unbound_results_and_gets.rs` (835): 25 tests from 6 topics [lines 4526-5660]:
  - unbound helper results (9): an_unbound_helper_hostname_runs_at_constant_rss, an_unbound_helper_result_that_allocates_only_its_result_runs_at_constant_rss, an_unbound_helper_tempdir_runs_at_constant_rss, an_unbound_helper_result_consumed_by_concat_runs_at_constant_rss, an_unbound_helper_result_passed_to_another_call_runs_at_constant_rss, a_bound_helper_result_still_runs_at_constant_rss, a_bound_directory_helper_result_still_runs_at_constant_rss, a_returned_helper_result_still_runs_at_constant_rss, a_helper_result_moved_into_a_collection_still_runs_at_constant_rss; items SHAPE_576_UNBOUND_HOSTNAME, SHAPE_576_UNBOUND_ARCH, SHAPE_576_UNBOUND_TEMPDIR, SHAPE_576_UNBOUND_IN_CONCAT, SHAPE_576_UNBOUND_AS_ARGUMENT, SHAPE_576_CONTRAST_BOUND_ARCH, SHAPE_576_CONTRAST_BOUND_CWD, SHAPE_576_CONTRAST_RETURNED, SHAPE_576_CONTRAST_INTO_COLLECTION.
  - unbound helper values (1): every_unbound_helper_result_position_still_produces_the_right_value; items SHAPE_576_VALUES.
  - getor elements (7): an_unbound_list_getor_hit_runs_at_constant_rss, an_unbound_list_getor_miss_runs_at_constant_rss, an_unbound_hash_map_getor_hit_runs_at_constant_rss, an_unbound_hash_map_getor_miss_runs_at_constant_rss, an_unbound_scan_map_getor_hit_runs_at_constant_rss, an_unbound_scan_map_getor_miss_runs_at_constant_rss, a_returned_getor_element_runs_at_constant_rss; items SHAPE_592_LIST_GETOR_HIT, SHAPE_592_LIST_GETOR_MISS, SHAPE_592_HASH_MAP_GETOR_HIT, SHAPE_592_HASH_MAP_GETOR_MISS, SHAPE_592_SCAN_MAP_GETOR_HIT, SHAPE_592_SCAN_MAP_GETOR_MISS, SHAPE_592_RETURNED.
  - get contrasts (3): a_bound_getor_element_still_runs_at_constant_rss, an_unbound_get_element_still_runs_at_constant_rss, a_borrowed_get_element_still_runs_at_constant_rss; items SHAPE_592_CONTRAST_BOUND_GETOR, SHAPE_592_CONTRAST_UNBOUND_GET, SHAPE_592_CONTRAST_BORROWED_GET.
  - unbound collection values (2): every_unbound_collection_element_position_still_produces_the_right_value, a_borrowed_get_with_a_fresh_key_operand_runs_at_constant_rss; items SHAPE_592_VALUES, SHAPE_592_BORROWED_GET_WITH_A_FRESH_KEY.
  - trapped error retention (3): a_trapped_error_bound_as_a_resource_is_not_retained, a_trapped_error_bound_as_an_address_list_is_not_retained, every_failing_resource_call_still_reports_its_error_and_origin; items b593_trapped_error_probe, b593_assert_error_not_retained, b593_long_message, b593_resource_probe, b593_address_list_probe, B593_VALUES, B593_VALUES_EXPECTED.
- `tests/runtime/rt_scope_drop_leaks/loops_and_condition_temps.rs` (798): 36 tests from 5 topics [lines 5661-6324, 6385-6676]:
  - addresses (5): every_address_reads_back_after_its_builder_scratch_is_freed, a_looped_net_lookup_runs_at_constant_rss, a_looped_local_address_runs_at_constant_rss, a_looped_user_built_address_list_runs_at_constant_rss, a_looped_udp_receive_runs_at_constant_rss; items SHAPE_599_ADDRESS_VALUES, SHAPE_599_LOOKUP_LOOP, SHAPE_599_LOCAL_ADDRESS_LOOP, SHAPE_599_USER_BUILT_LIST_LOOP, SHAPE_599_DATAGRAM_LOOP.
  - recursive loops (7): a_looped_recursive_union_bind_runs_at_constant_rss, a_looped_recursive_record_bind_runs_at_constant_rss, a_looped_recursive_reassignment_runs_at_constant_rss, a_looped_recursive_global_overwrite_runs_at_constant_rss, a_looped_unbound_recursive_temp_runs_at_constant_rss, a_looped_recursive_closure_capture_runs_at_constant_rss, a_looped_recursive_get_result_runs_at_constant_rss; items SHAPE_C_UNION_BIND, SHAPE_C_RECORD_BIND, SHAPE_C_REASSIGN, SHAPE_C_GLOBAL, SHAPE_C_UNBOUND_TEMP, SHAPE_C_CLOSURE, SHAPE_C_GET.
  - json regex trap loops (6): a_looped_inline_trap_union_bind_runs_at_constant_rss, a_looped_failing_call_with_a_temp_argument_runs_at_constant_rss, a_looped_trapped_recursive_json_parse_runs_at_constant_rss, a_repeated_json_parse_of_one_document_runs_at_constant_rss, a_repeated_regex_find_all_runs_at_constant_rss, a_looped_unbound_recursive_json_temp_runs_at_constant_rss; items SHAPE_C_JSON_REPEAT, SHAPE_C_REGEX_REPEAT, SHAPE_C_JSON_UNBOUND_TEMP, SHAPE_C_JSON_TRAP_RECOVER, SHAPE_TRAP_UNION_DEFAULT, SHAPE_FAILING_CALL_TEMP_ARGUMENT.
  - branch exit temps (10): an_if_condition_temp_is_freed_when_the_branch_returns, a_block_if_condition_temp_is_freed_when_the_branch_returns, an_elseif_condition_temp_is_freed_when_the_branch_returns, an_if_condition_temp_is_freed_when_the_branch_returns_a_fresh_string, an_if_condition_record_temp_is_freed_on_exit_do, an_if_condition_temp_is_freed_on_exit_for, an_if_condition_temp_is_freed_on_exit_while, an_if_condition_temp_is_freed_on_continue_while, an_if_condition_temp_is_freed_on_continue_for, a_continue_out_of_a_nested_loop_frees_every_enclosing_condition_temp.
  - loop condition temps (8): a_do_while_condition_temp_is_freed_on_every_pass, a_while_condition_temp_is_freed_on_every_pass, a_loop_until_condition_temp_is_freed_on_every_pass, a_while_condition_record_temp_is_freed_on_every_pass, condition_temps_that_already_had_an_owner_stay_flat, a_return_from_a_for_each_frees_its_fresh_collection, a_return_from_a_while_with_a_condition_temp_frees_every_block, a_resource_temporary_in_a_container_is_closed.

Surface: none (no support file; root keeps every shared helper).
Sweeps:
- `a_resource_temporary_in_a_container_is_closed` (→ `loops_and_condition_temps.rs`) reads
  `concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml")` (line 6632). The path is absolute, so the
  move does not affect it.
- Many tests carry `#[cfg(unix)]` / `#[cfg(any(target_os = …))]`, and those attributes move with
  their tests.

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

Every phase follows the same order:
1. Run K1 on the unsplit tree.
2. Split each file per its §4 entry.
3. Run K2–K5.
4. Commit the phase with its new files, its roots, the ceiling file, and any `.ai` line it fixes.

### Phase 1 — rt_resource_last_holder and rt_self_update_cost (§4.1, §4.2), and the stale `.ai` path

Two root + one part splits with no support module, plus the doc fix.
`T="rt_resource_last_holder rt_self_update_cost"`, `RE="rt_(resource_last_holder|self_update_cost)"`.

- [x] K1 → 27 tests. Result: `rt_resource_last_holder 14`, `rt_self_update_cost 13` (K1 loop,
      `/tmp/176C-<t>.before`).
- [x] `rt_resource_last_holder.rs` → root (11 tests and the helpers) + `trapped_and_maps.rs` (§4.1).
      Moved by a line-range script asserting anchor lines (893 `}`, 906/1046/1301 the test fns);
      every source line assigned once. Non-blank-line multiset of HEAD's file = root + part, minus
      the part header and the one `#[path]`/`mod` pair → `missing: []`, `extra: []`.
- [x] `rt_self_update_cost.rs` → root (linearity and churn tests, `SCALE_PROGRAM`, `counter`) +
      `value_semantics.rs` (`VALUE_PROGRAM` and its two tests) (§4.2). Same script and multiset
      check → `missing: []`, `extra: []`.
- [x] `.ai/resources-packages.md:49` and `:176`: replace `tests/rt_res_state_inplace_mutation.rs`
      with `tests/runtime/rt_res_state_inplace_mutation.rs` (§4.0). Check:
      `grep -n 'tests/rt_res_state' .ai/resources-packages.md` → no output. Result: no output;
      `ls tests/runtime/rt_res_state_inplace_mutation.rs` → exists.
- [x] Size check: `for f in tests/runtime/rt_resource_last_holder.rs tests/runtime/rt_resource_last_holder/trapped_and_maps.rs tests/runtime/rt_self_update_cost.rs tests/runtime/rt_self_update_cost/value_semantics.rs; do python3 -I tools/loc-census/census.py --file $f; done`
      → `0 819`, `0 660`, `0 773`, `0 567`. Result: `0 819`, `0 660`, `0 773`, `0 567`.
- [x] Tighten: K5 and commit the ceiling file and `.ai/resources-packages.md`. Result: first grep →
      `TIGHTEN test 819 (ceiling 1476)` and `TIGHTEN test 773 (ceiling 1337)`, no NEW/GROW/STALE;
      `MFB_LOC_TIGHTEN=1` run → ok; ceiling diff removes exactly those two rows; re-run grep → no output.

Acceptance: 27 tests are listed unchanged and pass, the two paths leave the banner, and the `.ai`
path is real.
  Check: K2 → no output; K3 → 2 × ok; K4 → no output; K5 → 2 TIGHTEN rows, then none; the
  `tests/rt_res_state` grep → no output (est. 12 min: rt_self_update_cost's linearity tests time
  release builds).
  Result (2026-10-18): K2 → no `DIFF` line (14 and 13 names, identical); K3 (landing tier,
  `bash scripts/test-fast.sh --test rt_resource_last_holder --test rt_self_update_cost` →
  `test-fast: ok`; `rt_resource_last_holder` `14 passed; 0 failed`, `rt_self_update_cost`
  `13 passed; 0 failed`); K4 → no output; K5 → 2 TIGHTEN rows, then none; `cargo fmt --all -- --check`
  → exit 0; the `tests/rt_res_state` grep → no output.
Commit: 677a6002d

### Phase 2 — rt_scope_drop_leaks (§4.3)

The largest split in the letter: 154 tests into the root and 6 parts. It goes last because a
missed `#[path]` here drops the most tests.
`T="rt_scope_drop_leaks"`, `RE="rt_(scope_drop_leaks)"`.

- [x] K1 → 154 tests. Result: `rt_scope_drop_leaks 154`.
- [x] `rt_scope_drop_leaks.rs` → root (keeps `peak_rss`, `assert_flat`, `B561_TRAP_ALWAYS_FAILS`,
      `B573_BUILTIN_RAISE`, `condition_temp_report`, `assert_condition_temps_freed`,
      `thread_loop_report` and `thread_loop_growth` with their cfg attributes, and the two thread
      topics) + the 6 parts of §4.3. Every test's and constant's `#[cfg(…)]` moves with it.
      Moved by the same anchor-asserting line-range script (33 anchors at every range boundary);
      non-blank-line multiset check → `missing: []`, `extra: []`. `grep -c '#\[test\]'` per file →
      root 5, parts 23 / 22 / 24 / 19 / 25 / 36 (§4.3 order), as planned.
- [x] Declaration check: `grep -c '^#\[path = "rt_scope_drop_leaks/' tests/runtime/rt_scope_drop_leaks.rs` → `6`,
      and `ls tests/runtime/rt_scope_drop_leaks/*.rs | wc -l` → `6`. Result: `6`, `6`.
- [x] Size check: `for f in tests/runtime/rt_scope_drop_leaks.rs tests/runtime/rt_scope_drop_leaks/*.rs; do python3 -I tools/loc-census/census.py --file $f; done`
      → root `0 517`, and parts `0 831`, `0 878`, `0 867`, `0 768`, `0 835`, `0 798` (in §4.3 order;
      `ls` sorts them alphabetically). Result: root `0 517`; return_and_string_shapes `0 831`,
      trap_binds_hofs_and_for_each `0 878`, closures_errors_and_concats `0 867`,
      helper_results_and_raised_errors `0 768`, unbound_results_and_gets `0 835`,
      loops_and_condition_temps `0 798`.
- [x] Tighten: K5 and commit the ceiling file. Result: first grep → `TIGHTEN test 517 (ceiling 5476)`
      only; `MFB_LOC_TIGHTEN=1` run → ok; ceiling diff → only `-ceiling test 5476 tests/runtime/rt_scope_drop_leaks.rs`;
      re-run grep → no output.

Acceptance: all 154 tests are listed under the same names and pass.
  Check: K2 → no output; K3 → ok; K4 → no output; K5 → 1 TIGHTEN row, then none (est. 25 min: the
  target runs 154 RSS-flatness programs; this is the run of exactly the moved tests).
  Result (2026-10-18): K2 → no `DIFF` line (154 names, identical); K3 (landing tier,
  `bash scripts/test-fast.sh --test rt_scope_drop_leaks --test test_targets_registered --test rt_resource_last_holder --test rt_self_update_cost`
  → `test-fast: ok`; `rt_scope_drop_leaks` `154 passed; 0 failed`, and the Phase 1 targets
  `14 passed` / `13 passed` again); K4 → no output; K5 → 1 TIGHTEN row, then none;
  `cargo fmt --all -- --check` → exit 0.
Commit: 4e345a8b5

## Validation Plan

- Tests: none are added or removed. Every moved test runs in its phase's K3.
- Coverage check: `tests/` is outside the coverage denominator (`scripts/coverage-common.sh`
  IGNORE), and no compiler code changes.
- Runtime proof: each phase's K3.
- Doc sync: `.ai/resources-packages.md` (stale path fixed, Phase 1). No spec or man topic changes,
  and no `mfb spec` contract changes.
- Final gate (once, after Phase 2):
  `cargo test --test file_size_budget 2>&1 | grep -E 'tests/runtime/rt_(resource_last_holder|self_update_cost|scope_drop_leaks)(\.rs|/)'`
  → no output. The same regex over the ceiling file → no output. `cargo test --test test_targets_registered`
  → ok (est. 3 min). The full workspace suite runs once, in plan-176-V.
  Result (2026-10-18, tree of `4e345a8b5`): the budget grep → no output; the ceiling-file grep →
  no output; `test_targets_registered` → `4 passed; 0 failed` (inside the Phase 2 landing-tier
  run, `test-fast: ok`).

## Decisions

- Rule revised — owner, 2026-10-18. It replaces the 2026-10-14 "code ≤ 1000, test ≤ 250" rule.
  Counting is non-comment, non-blank LOC over `.rs`/`.mfb` under `src/`, `packages/`,
  `examples/`, `repository/` and `tests/`:

  | Kind | Applies to | Limit | Over the limit → |
  |---|---|---|---|
  | `code` | code LOC of a file that is not a test-only file | 1000 | split the file into modules |
  | `inline` | inline test LOC (`#[cfg(test)]` items / MFBASIC `TESTING … END TESTING`) inside a file that is not a test-only file | 250 | move the tests out into their own test-only file(s) |
  | `test` | total LOC of a test-only file (path component `tests`, `test_*.mfb`, `#![cfg(test)]`, or a `#[cfg(test)] mod x;` target and its subtree) | 1000 | split the test-only file into modules |

  There are no exemptions. Generated files (`GENERATED` banner) are the only exclusion. Parts
  target ≤ 900. The guard warns and never fails. A new module is always an acceptable split.
  Ceiling rows are `ceiling <code|inline|test> <n> <path>`.
- Parts are regrouped into as few files as keep each ≤ 900, merging neighbouring topics in source
  order, and the root keeps the topics nearest its shared helpers — author, 2026-10-18.
- A program const lives beside its only consumer rather than in a support file
  (`SCALE_PROGRAM`, `VALUE_PROGRAM`) — author, 2026-10-18, because no visibility edit is then
  needed.
- Glob-cycle imports compile without warnings (`/tmp/loc/probe2` printed `6`, no warning) — verified.
- Pre-existing defects found by the map are fixed in the letter that found them: the
  `.ai/resources-packages.md` stale path — AGENTS.md "never leave a bug", 2026-10-14.

## Corrections

- 2026-10-18 — Rule revised by the owner: test-only files limit 1000 (was 250), inline tests 250,
  no exemptions. Dropped 29 files, all now ≤ 1000 (all `tests/runtime/*.rs`):
  rt_package_parameter_defaults, rt_package_private_type_collision,
  rt_package_public_export_name_collision, rt_parameter_default_scope,
  rt_top_level_initializer_globals, rt_place_forward, rt_process_spawn_ambient_fds,
  rt_record_nested_update, rt_record_update_forms, rt_recursive_value_collection_drops,
  rt_recursive_value_construction_copies, rt_recursive_value_copies,
  rt_recursive_value_drop_symmetry, rt_regex_bounds, rt_regex_span, rt_sortby_copying_native,
  rt_thread_block_arena, rt_res_list_from_call_leak, rt_res_state_inplace_mutation,
  rt_resource_aggregate_temporary, rt_resource_drop_reclaims, rt_resource_model,
  rt_resource_review_805, rt_resource_review_806, rt_resource_store_float,
  rt_resource_trap_state_free, rt_resource_union_box_owner, rt_resource_union_member_float,
  rt_resource_union_reclaim.

  Other changes:
  - The three kept files go from 49 new files to 8 (rt_scope_drop_leaks: 35 topics → 6 parts).
  - Both exemptions are gone and their units are not split: `rt_resource_last_holder/map_iteration.rs`
    (284-LOC test) and `rt_self_update_cost/value_program.rs` (444-LOC const).
  - The 23 rt_resource_model `include_str!` repoints went with that file.
  - The `.ai/resources-packages.md` stale-path fix is kept as a standalone doc fix. It is still
    stale at `ce7a995ca`.
  - Five phases became two.
- 2026-10-18 (execution) — the plan's `/tmp/loc/census.py` and `/tmp/loc/span.py` are
  `python3 -I tools/loc-census/census.py [--file PATH [--lines A B]]` after plan-176-A. Every §4
  line range matched the tree at `bdabb4517` (anchor lines asserted by the move script, and
  `git diff --stat 17a9b5799 HEAD -- <the 3 files>` → no output), so no range was re-mapped.
- 2026-10-18 (execution) — owner instruction 2026-10-18: landing tier per phase; the full suite
  runs once before the plan merges. K3 runs as `bash scripts/test-fast.sh --test <t>...` (which
  also runs fmt, `cargo check --workspace --all-targets`, unit tests and the guard binaries), and
  the final gate's `cargo test --test test_targets_registered` runs inside that landing-tier call.
- 2026-10-18 (execution) — part files start with a `//!` header naming what they hold and where
  the shared helpers live, then `use super::*;` (B's convention). The header is comment-only, so
  every measured size equals the plan's.

## Summary

The risk is a part that is never compiled, chiefly among rt_scope_drop_leaks' 6 parts. The
name-list diff per target closes that, and so do the declaration count and one run of exactly the
moved tests. No file read by path moves. Nothing outside `tests/runtime/`, the ceiling file and two
lines of `.ai/resources-packages.md` changes.
