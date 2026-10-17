# plan-176-G: IR and monomorph test code

Last updated: 2026-10-17
Effort: large (3h–1d)
Depends on: plan-176-F

G moves the over-budget test code under `src/ir` and `src/monomorph` into `tests/` directory
modules, split by topic so that no new file is over 250 test LOC. Seven files are in scope:

- three whole test files: `src/ir/tests.rs` (6353), `src/ir/variant_corpus_tests.rs` (1748)
  and `src/ir/verify/tests.rs` (8465);
- four code files whose inline `#[cfg(test)] mod tests { … }` is over 250:
  `src/ir/resource_escape.rs`, `src/ir/shape.rs`, `src/monomorph/helpers.rs` and
  `src/monomorph/lower.rs`.

No test is added, removed or renamed. Only the module path of each moved test changes
(`ir::verify::tests::rejects_for_step_zero` → `ir::verify::tests::for_loops::rejects_for_step_zero`).
The host code files keep their code and their paths. Their code-side splits are letters M and N.

The single outcome: after G lands, `cargo test --test file_size_budget` shows no row for any of
G's 7 files, and `tests/guards/file_size_ceilings.txt` has no `ceiling test` row for any of
them. G adds no `exempt` row.

References:

- `planning/plan-176-A-file-size-guard.md`: the rule, §Decisions, and "Splitting an
  over-budget file" in `.ai/build-tooling.md` (src tests: `foo/tests/mod.rs` + topic files;
  the coverage `IGNORE` of `(^|/)tests/`).
- `.ai/testing-gates.md` § "Splits must sweep man AND spec citations", and the note that a
  split can leave the release build green while the TEST build is broken (the gate is an actual
  `test result: ok` line from a `cargo test -p mfb --bin mfb <module>::` run).
- `.ai/codegen-invariants.md`: names `unify_type`, `collect_imported_records` and
  `lower_expression`. All three are code symbols that stay in their host files.
- `.ai/build-tooling.md`: rustfmt/clippy policy.
- Spec topic touched: `src/docs/spec/architecture/23_escape-analysis.md:358`
  (`[[src/ir/resource_escape.rs:tests]]`). Every other spec citation of a G host names code
  that stays (measured per file in §4).
- Guards: `tests/guards/no_type_strings.rs` and `tests/guards/no_operator_strings.rs`
  (`is_test_file` exempts `/tests/` paths); `tests/guards/architecture_guards.rs`
  (`no_hand_picked_vreg`, `emission_lines`, `only_testutil_installs_a_panic_hook` scan all of
  `src/`). After plan-176-E, the panic-hook guard is in
  `architecture_guards/seams_goldens_and_panic_hook.rs`.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-176-F is complete and archived | `ls planning/plan-176-F-* planning/completed/plan-176-F-*` → only `planning/completed/…` | NOT MET (2026-10-14) |
| The file-size guard exists and passes | `cargo test --test file_size_budget` → passes | NOT MET (2026-10-14) |
| G's files have no uncommitted changes | `git status --porcelain -- src/ir/tests.rs src/ir/tests src/ir/variant_corpus_tests.rs src/ir/verify/tests.rs src/ir/resource_escape.rs src/ir/shape.rs src/ir/mod.rs src/monomorph/helpers.rs src/monomorph/lower.rs src/docs/spec/architecture/23_escape-analysis.md` → empty | MET (2026-10-17) |

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

- Every file G creates is ≤ 250 test LOC, and each host keeps ≤ 250 test LOC
  (`python3 -I tools/loc-census/census.py --file <path>` → second field ≤ 250).
- `cargo test --test file_size_budget` prints no row naming a G path, and the ceiling file
  holds no `ceiling test` row for one.
- The `mfb` unit-test binary lists the same number of tests before and after each phase.

### Non-goals (explicit constraints)

- No behavior change. No test body is edited beyond moving it (dedented one level when it
  leaves an inline `mod … { }`). The allowed edits are: adding `use super::*;` at the top of a
  topic file; the `pub(super)` edits (49 items, counted in §4); and the `use`-line edits
  listed in §4 (one more `super::` on two fn-local `use`s; four widened import lists).
- No `#[test]` fn is renamed, added or removed. No `[[test]]` target is touched.
- No host code moves. `resource_escape.rs`, `shape.rs`, `helpers.rs` and `lower.rs` keep
  every non-test line. Letters M and N split the code.
- `crate::ir::tests::helpers::lower_src` (external caller `src/target/shared/config_fold.rs:462`)
  keeps its path, and so does `crate::ir::variant_corpus_tests`.
- No guard weakens. No production file becomes exempt from any guard.

## 2. Current State

The counts come from the planning census (`python3 -I /tmp/loc/census.py "$PWD"`, working tree
of `17a9b5799`) and `python3 -I /tmp/loc/span.py`. `src/ir/shape.rs` changed since
(`267088179`), so it was re-measured on 2026-10-17 at `a3c95fc07`. The test counts come from
`grep -c '#\[test\]' <file>`.

| File | Kind | Test LOC | Tests |
|---|---|---|---|
| `src/ir/tests.rs` | test file (`#[cfg(test)] pub(crate) mod tests;` in `src/ir/mod.rs`) | 6353 | 283 (+5 in the existing `src/ir/tests/inline_trap_short_circuit.rs`) |
| `src/ir/variant_corpus_tests.rs` | test file (`#[cfg(test)] mod variant_corpus_tests;`, `src/ir/mod.rs:56-57`) | 1748 | 34 |
| `src/ir/verify/tests.rs` | test file (`#[cfg(test)] mod tests;`, `src/ir/verify/mod.rs:1838-1839`) | 8465 | 492 |
| `src/ir/resource_escape.rs` | inline `mod tests` (L856-1777) | 774 | 28 |
| `src/ir/shape.rs` | inline `mod tests` (L3767-…; L3762 at `17a9b5799`) | 1228 at `17a9b5799`, +18: `267088179` added `term_measure_text_attributed_requires_astrings_import` (`span.py src/ir/shape.rs 4372 4389` → 18) | 58 (57 at `17a9b5799`) |
| `src/monomorph/helpers.rs` | inline `mod tests` (L781-1538) | 645 | 26 |
| `src/monomorph/lower.rs` | inline `mod tests` (L2975-4526) | 1282 | 45 |

### Verified properties

- **The topic layout keeps module paths and `super` meanings.**
  - `src/ir/tests.rs` → `src/ir/tests/mod.rs`. The directory already exists (it holds
    `inline_trap_short_circuit.rs`), and `mod tests;` in `src/ir/mod.rs` resolves to either
    form.
  - The inline `pub(crate) mod helpers { use super::super::*; … }` becomes
    `src/ir/tests/helpers.rs` at the same path `crate::ir::tests::helpers`, so its
    `use super::super::*;` still means `crate::ir`.
  - For a file module `src/x/foo.rs`, `#[cfg(test)] mod tests;` resolves to
    `src/x/foo/tests/mod.rs`, the plan-176-A convention.
- **A `#[path]`-declared `…/mod.rs` resolves its children relative to its own directory.**
  Probe `/tmp/loc_e_probe`: `src/x/inner/mod.rs` declares
  `#[path = "../../../tests/common/emu/mod.rs"] mod emu;`, and its `mod scan;` resolved to the
  sibling `scan.rs` (`cargo test --lib` → `1 passed`). `variant_corpus_tests` relies on this.
- **The string guards exempt the new paths.** `no_type_strings.rs::is_test_file` and
  `no_operator_strings.rs::is_test_file` return true for `rel.contains("/tests/")`. Every G
  file lands under a `tests/` directory. `variant_corpus_tests.rs` was exempt only through its
  `_tests.rs` suffix, which a directory split loses. Hence its `#[path]` into
  `src/ir/tests/variant_corpus/`.
- **The architecture guards need no change for G.** `no_hand_picked_vreg` and `emission_lines`
  scan all of `src/`. They skip only files named `test_support.rs` or `*tests.rs`, and
  otherwise scan everything above the first `#[cfg(test)]`/`mod tests`. So G's moved files
  would be scanned. They contain none of the needles:
  `grep -cE '"%[vf][0-9]|Reg::physical\(|\.physical_field\(|Operand::phys\(|format!\("%[vf]'`
  over all 7 G files → 0 each.
  `grep -lE 'set_hook|take_hook'` over the same 7 files → none, so the panic-hook guard is
  unaffected. `is_physical_scan_exempt` scans only `src/codegen` and `src/target`, which G
  does not touch.
- **Spec citations.** The only citation into moved code is `[[src/ir/resource_escape.rs:tests]]`
  (`grep -rn 'resource_escape.rs:tests' src/docs/spec` →
  `architecture/23_escape-analysis.md:358`). The other 19 `resource_escape.rs`, 21 `shape.rs`,
  16 `helpers.rs` and 31 `lower.rs` citations name code symbols whose definitions lie outside
  the moved ranges. Each cited symbol's definition line was located against the range.
  `grep -rnoE "\[\[src/ir/(tests|variant_corpus_tests|verify/tests)\.rs[^]]*\]\]" src/docs/spec`
  → none.
- **No path reference elsewhere.**
  `grep -rn "ir/tests.rs\|variant_corpus_tests\|verify/tests.rs\|ir/tests::\|verify::tests\|resource_escape::tests\|shape::tests\|helpers::tests\|lower::tests" .ai AGENTS.md scripts src/docs tests/guards Cargo.toml .github`
  → none. No G file uses `include_str!`, `#[path]`, `module_path!` or `--exact`, and none is a
  coverage exception (`grep -n 'src/ir\|monomorph' scripts/coverage-exceptions.txt` → only a
  comment naming `src/ir/verify/link.rs`). `lowering_totality_tests` reads
  `env!("CARGO_MANIFEST_DIR").join("tests")` (L167 as of 17a9b5799), which is manifest-relative.
- **Coverage denominator.** `scripts/coverage-common.sh` `IGNORE` drops `(^|/)tests/`.
  - `src/ir/variant_corpus_tests.rs` is in the denominator today, because no path component is
    `tests`. After the move it is ignored.
  - The four hosts lose their inline test lines from the denominator. Those lines are executed
    lines: the `--bins` run executes every `#[test]`.
  - The hosts' code lines, and which of them are covered, do not change: no code moves and no
    test is removed. G has no coverage gate (§Validation Plan, §Decisions).

## 3. Design Overview

Conventions, per file:

- A test file `foo.rs` (or an inline `mod tests { }` in `foo.rs`) becomes `foo/tests/mod.rs`
  plus topic files. `mod.rs` keeps the original `use` block and the helpers shared by more than
  one topic. A helper used by exactly one topic moves into that topic.
- When `mod.rs` would exceed 230, the shared helpers go to **support** files instead. Their
  items become `pub(super)`, and `mod.rs` adds `use <support>::*;`. Glob-cycle imports compile
  without warnings (`/tmp/loc/probe2`).
- Each topic file begins `use super::*;` and holds its items verbatim, dedented one level when
  they came out of an inline module. `impl` blocks travel with their type.
- Topic names avoid the host module's own child-module names. In `ir::verify` those are
  `calls`, `compat`, `link`, `matching`, `ops`, `resources`, `trap_init`, `types`, `values`.
- The host keeps `#[cfg(test)] mod tests;` in place of the inline block.

The gate class is test motion: the whole-binary `--list` count is unchanged, the moved modules'
tests pass, there are no new warnings, the guards pass, then tighten. Byte-identity does not
apply, because no compiled compiler code changes (`cargo build --release` never compiles a
`#[cfg(test)]` module).

Risk concentrates in two places. The first is `src/ir/verify/tests.rs`: 492 tests over 52 new
files, 42 support items made `pub(super)`, and one widened import list. A missed `super::`
path there fails only the test build. It lands last. The second is `variant_corpus_tests`'s
`#[path]` into a directory that `ir::tests` does not declare.

Rejected alternatives:

- *`src/ir/variant_corpus_tests/` (no `tests/` component).* The `_tests.rs` suffix that exempts
  the file from both string guards today would be lost, and `/tests/` would not be in the path.
  The moved code would be scanned as production. `src/ir/tests/variant_corpus/` keeps it exempt.
- *Declaring `variant_corpus` as `ir::tests::variant_corpus`.* That changes `super` from
  `crate::ir` to `crate::ir::tests` in every body that spells `super::binary::…` and similar.
  The `#[path]` keeps the module path `crate::ir::variant_corpus_tests` and every `super::`
  unchanged.
- *`foo/tests.rs` instead of `foo/tests/mod.rs`.* Rejected by plan-176-A: coverage `IGNORE`
  and the guards key on a `tests` directory component.

## 4. Detailed Design

The LOC figures are `span.py` counts of the moved items, plus 1 for each new file's
`use super::*;`. Topic layouts were produced by a section-header-aware packer
(`/tmp/loc_gh/render.py`, specs in `/tmp/loc_gh/specG.py`) and checked: every `#[test]` lands
in exactly one file. `[src a-b]` is the original line span as of 17a9b5799. Item and test NAMES
are authoritative if lines drift.

### 4.1 `src/ir/tests.rs` → `src/ir/tests/` (283 tests + 5 unchanged)

`src/ir/tests.rs` is a root of seven inline `mod … { }` blocks plus
`mod inline_trap_short_circuit;` (`src/ir/tests/inline_trap_short_circuit.rs`, 147 LOC,
5 tests, unchanged). The file is deleted, and `src/ir/tests/mod.rs` takes its place.

- `src/ir/tests/mod.rs` (17): `use super::*;`, `use crate::types::ParameterType;`,
  `mod inline_trap_short_circuit;`, and the seven declarations below, each keeping its
  attributes (`#[cfg(test)] pub(crate) mod helpers;`, `#[cfg(test)] mod lowering_totality_tests;`, …).
- `src/ir/tests/helpers.rs` (82, 1 test) [src 11-122]: the body of `pub(crate) mod helpers`, verbatim.
- `src/ir/tests/lowering_totality_tests.rs` (78, 1 test) [src 136-231]: body verbatim.
- `src/ir/tests/lower_construct_tests.rs` (168, 9 tests) [src 4381-4585]: body verbatim.
- `src/ir/tests/lower_coverage_tests.rs` (98, 6 tests) [src 7350-7485]: body verbatim.
- `src/ir/tests/binary_repr_tests/` (inline 541, 20 tests; max 193):
  - `mod.rs` (24): the `use` block, 3 topic `mod` lines; shared helpers `corpus_project`,
    `fn_named`, `project_named`.
  - `round_trip_and_identity.rs` (137, 7) [src 242-435]: `binary_repr_round_trip_is_identity`,
    `capture_index_round_trips_at_the_u32_boundary`, `binary_repr_rejects_bad_magic`,
    `binary_repr_rejects_bad_version`, `package_identity_prefix_is_applied_consistently`,
    `prefix_package_symbols_qualifies_link_tables`, `merge_package_dedups_link_cstructs`.
  - `package_init_order.rs` (193, 4) [src 444-653]: `package_bindings_initialize_dependencies_first`,
    `builtin_bindings_initialize_first`, `a_package_s_builtin_bindings_initialize_first`,
    `link_state_trailer_and_bind_in_round_trip`.
  - `verify_and_json.rs` (190, 9) [src 658-869]: `float_blocked_resource_owner_encodes_as_local`,
    `rejects_an_absurd_cstruct_field_count`, `verify_package_rejects_duplicate_function`,
    `verify_package_rejects_unnamed_function`, `verify_package_rejects_empty_match`,
    `json_covers_loop_state_and_capture_ops`, `json_binding_and_param_null_value_and_default`,
    `json_bind_with_null_value_emits_null`, `visibility_name_maps_all_variants`. Local:
    `json_extra_ops_project`.
- `src/ir/tests/lower_tests/` (inline 2994, 143 tests; max 226):
  - `mod.rs` (60): the `use` block (`use super::super::*; use super::helpers::{function, lower_src, try_lower_src};`),
    19 topic `mod` lines; shared helpers `main_body`, `binding`, `bind_value`, `describe`.
  - `declarations.rs` (144, 10) [src 884-1073]: `top_level_bindings_explicit_and_inferred`,
    `set_type_round_trips_through_front_end`, `inferred_binding_from_call_result`,
    `func_kinds_and_returns`, `func_with_default_param_and_isolated`, `func_returning_nothing`,
    `func_trap_handler_lowers_to_trap_op`, `type_record_with_private_field`,
    `union_with_variants_and_includes`, `enum_members_lowered`. Local: `ir_type`.
  - `statements.rs` (182, 15) [src 1077-1272]: `let_mut_and_assignment_statements`,
    `assignment_to_global_is_assign_global`, `return_without_value_from_sub`,
    `exit_targets_lower_to_expected_ops`, `exit_sub_lowers_to_valueless_return`,
    `exit_func_contributes_no_op`, `fail_statement_lowers_to_fail_op`,
    `if_else_lowers_to_if_op_with_both_branches`, `while_and_do_until_loops`,
    `for_loop_with_step`, `for_loop_default_step`, `for_loop_float_bounds_promote_loop_type`,
    `for_each_over_list`, `for_each_over_map_yields_map_entry`,
    `expression_statement_lowers_to_eval`.
  - `match_literals_and_operators.rs` (183, 7) [src 1276-1484]: `match_with_value_oneof_and_else`,
    `match_with_guard`, `match_on_union_binds_variant`,
    `match_on_fallible_call_uses_ok_error_and_is_ok_flag`, `literal_kinds_lower_to_typed_consts`,
    `most_negative_integer_literal_folds_into_the_const`, `binary_and_unary_operators`.
  - `calls_constructors_literals.rs` (132, 9) [src 1488-1632]: `user_function_call_lowers_to_call`,
    `call_with_named_and_default_arguments`, `error_builtin_lowers_to_error_record`,
    `builtin_call_return_type_resolves`, `constructor_and_member_access`,
    `constructor_named_arguments`, `with_update_expression`, `list_and_map_literals`,
    `empty_list_literal_uses_expected_element_type`.
  - `lambdas.rs` (110, 4) [src 1636-1750]: `lambda_capturing_local_becomes_closure`,
    `lambda_without_capture_is_function_ref`, `named_function_reference_as_argument`,
    `mut_capture_in_foreach_is_by_ref`.
  - `inline_trap.rs` (125, 6) [src 1754-1882]: `inline_trap_bind_with_recover`,
    `inline_trap_assign_target`, `inline_trap_discard_bare_statement`,
    `inline_trap_handler_with_if_then_fail`, `inline_trap_handler_with_match_and_continuation`,
    `inline_trap_handler_diverges_with_return`.
  - `union_wrapping.rs` (137, 4) [src 1886-2057]: `returned_variant_wraps_into_union`,
    `union_typed_binding_wraps_variant`,
    `variant_of_every_expression_kind_wraps_at_a_union_argument`,
    `union_valued_argument_is_not_wrapped_again`. Local: `describe_all`, `show_arguments`.
  - `native_link.rs` (198, 6) [src 2061-2276]: `link_functions_resources_and_aliases`,
    `link_const_pin_and_result_expression`, `link_const_sizeof_folds_to_layout_size`,
    `link_bind_in_field_value_forms`, `link_expr_binary_minus_and_unknown_fallback`,
    `native_resource_close_uses_exported_alias`.
  - `doc_blocks_and_state.rs` (157, 7) [src 2280-2452]: `doc_blocks_collected_for_exports`,
    `doc_block_for_union_and_nothing_func`, `doc_for_nonexported_function_is_dropped`,
    `doc_resource_for_export_is_collected`, `doc_resource_for_nonexport_is_dropped`,
    `qualified_strings_call_lowers_to_internal_target`, `resource_state_param_assign_and_access`.
  - `builtin_calls_and_captures.rs` (226, 8) [src 2456-2714]:
    `many_builtin_package_calls_resolve_return_types`, `strings_and_collections_calls_resolve`,
    `collections_filter_with_named_predicate`, `regex_call_padding_and_internal_name`,
    `nested_call_arguments_capture_and_lower`, `lambda_captures_across_expression_shapes`,
    `link_success_on_with_comparison_and_boolean_ops`,
    `link_const_keeps_the_exact_bits_of_a_64_bit_flag`.
  - `external_and_write_ir.rs` (66, 3) [src 2718-2794]:
    `external_function_return_and_param_types_are_used`, `write_ir_emits_ir_file`,
    `inferred_binding_from_list_literal_types`. Its fn-local
    `use super::super::ExternalFunctionParam;` → `use super::super::super::ExternalFunctionParam;`.
  - `builtin_advisories_and_padding.rs` (180, 7) [src 2798-3023]: `json_and_csv_calls_resolve`,
    `builtin_enum_advisory_warns_once_per_user_occurrence`,
    `the_sha1_advisory_fires_on_hash_and_not_on_hmac_hkdf_or_pbkdf2`,
    `the_sha1_advisory_suppression_is_fail_closed`,
    `builtin_enum_without_advisory_never_warns`, `crypto_call_resolves_and_pads_aad`,
    `net_and_http_calls_resolve_and_pad`.
  - `builtin_padding.rs` (142, 8) [src 3025-3185]: `thread_call_resolves`,
    `datetime_time_pads_optional_arguments`, `regex_find_pads_start_argument`,
    `crypto_aead_pads_aad_as_empty_list`, `tls_connect_pads_optional_arguments`,
    `tls_close_on_listener_routes_to_listener_helper`, `vector_call_and_constant_lower`,
    `datetime_call_pads_default_arguments`.
  - `inline_trap_treeify.rs` (197, 10) [src 3189-3419]:
    `inline_trap_mut_bind_and_global_assign_targets`,
    `inline_trap_handler_with_terminator_then_dead_statement`,
    `inline_trap_handler_is_a_single_if`, `inline_trap_handler_is_a_single_match`,
    `inline_trap_handler_is_a_single_loop`, `inline_trap_handler_with_loop_and_continuation`,
    `inline_trap_handler_with_nested_if_and_continuation`,
    `inline_trap_handler_with_match_else_and_continuation`,
    `builtin_named_arguments_reorder_with_extra`, `local_call_named_arguments_reorder`.
  - `capture_variants.rs` (204, 9) [src 3423-3665]:
    `lambda_captures_through_map_literal_and_with_update`,
    `lambda_captures_called_function_local_and_constructor_arg`,
    `assignment_bodied_lambda_captures_assignment_target`,
    `local_call_named_then_positional_skips_filled_slot`,
    `builtin_call_named_then_positional_skips_filled_slot`,
    `native_resource_visibility_variants`, `lambda_captures_through_unary_and_trapped`,
    `collections_filter_with_lambda_predicate`, `doc_for_unmatched_overload_is_dropped`.
  - `literal_type_fallback.rs` (189, 11) [src 3669-3895]:
    `list_literal_without_expected_type_infers_from_first_literal`,
    `list_literal_without_expected_and_non_literal_first_element`,
    `builtin_type_field_access_and_net_lookup`,
    `collections_filter_with_user_function_predicate`,
    `list_literal_first_element_float_bool_and_nothing`, `nested_lambda_inside_lambda_body`,
    `lambda_captures_through_trapped_expression`,
    `filter_result_typed_binding_resolves_element_type`,
    `call_through_zero_arg_function_typed_local`, `doc_for_nonexported_type_is_dropped`,
    `inline_trap_binding_type_from_trapped_expression`.
  - `named_argument_normalization.rs` (156, 5) [src 3903-4079]:
    `overloaded_builtin_named_arguments_are_reordered_to_param_order`,
    `local_call_named_and_default_arguments_lower_in_param_order`,
    `a_computed_default_lowers_to_a_hidden_function_call`,
    `a_literal_default_lowers_as_a_constant`,
    `default_kind_is_literal_only_for_name_free_constants`. Its fn-local
    `use super::super::lower::{default_kind, DefaultKind};` →
    `use super::super::super::lower::{default_kind, DefaultKind};`.
  - `trap_handler_arms.rs` (129, 6) [src 4088-4232]:
    `trap_handler_global_assign_arm_without_error_read`, `trap_handler_exit_program_arm`,
    `trap_handler_continue_loop_arm`, `trap_handler_if_arm_recurses_both_branches`,
    `trap_handler_match_arm_with_guard_recurses`,
    `trap_handler_empty_loop_body_treeifies_to_nothing`.
  - `literal_type_inference.rs` (112, 8) [src 4236-4374]: `untyped_global_binding_infers_type`,
    `untyped_money_and_scalar_literal_types`, `expected_typed_numeric_literals_coerce`,
    `untyped_list_literal_element_types`, `collections_filter_bare_builtin_predicate_type`,
    `untyped_record_and_variant_constructor_and_field_types`,
    `member_access_builtin_type_field_infers`, `testing_block_item_is_skipped`.
- `src/ir/tests/lower_pipeline_tests/` (inline 2377, 103 tests; max 218):
  - `mod.rs` (30): its four `use` lines (including `use super::super::IrOp;`), 13 topic `mod`
    lines; shared helpers `lower_src`, `try_lower_src`, `function`, `json_of`. `lower_src`
    spells `super::IrProject` and `super::helpers::lower_src_named` and stays in `mod.rs`, so
    its `super::` depth is unchanged.
  - `expressions.rs` (194, 8) [src 4605-4861]: `lowers_every_literal_and_operator_kind`,
    `lowers_nothing_literal`, `lowers_list_and_map_literals`,
    `lowers_constructors_member_access_and_with_update`, `lowers_enum_member_access`,
    `lowers_union_wrap_and_extract`, `lowers_calls_and_function_refs`,
    `lowers_default_argument_padding_for_local_call`.
  - `statements.rs` (159, 7) [src 4865-5032]: `lowers_control_flow_statements`,
    `lowers_float_for_loop_and_exit_do_while`, `lowers_foreach_over_list_and_map`,
    `lowers_exit_program_and_exit_sub`, `lowers_global_assignment_and_state_assign`,
    `lowers_fail_and_function_trap`, `lowers_propagate_in_trap`.
  - `inline_trap_and_match.rs` (215, 6) [src 5036-5266]: `lowers_inline_trap_bind_assign_discard`,
    `lowers_inline_trap_treeify_if_and_match_continuation`,
    `lowers_inline_trap_treeify_match_with_tail_continuation`,
    `lowers_inline_trap_treeify_match_existing_else_with_tail`,
    `lowers_inline_trap_with_loop_and_foreach_in_handler`,
    `lowers_match_value_oneof_else_and_guard`.
  - `lambdas_and_builtins.rs` (188, 11) [src 5276-5492]:
    `lowers_lambda_by_value_capture_and_closure`, `lowers_lambda_mut_byref_capture_in_foreach`,
    `lowers_filter_predicate_function_ref`, `lowers_builtin_package_calls`,
    `lowers_named_argument_reordering_for_builtin`,
    `lowers_builtin_with_overloaded_argument_signature`,
    `lowers_builtin_with_generic_placeholder_argument_signature`,
    `lowers_named_constructor_arg_inside_lambda`, `lowers_builtin_mixed_positional_and_named_args`,
    `lowers_regex_and_datetime_default_padding`, `lowers_tostring_with_base_and_overridable`.
  - `native_link.rs` (138, 2) [src 5496-5640]: `lowers_native_link_functions_resources_and_aliases`,
    `lowers_link_const_and_result_expression_forms`.
  - `docs_and_resources.rs` (204, 8) [src 5642-5863]: `collects_doc_blocks_for_exported_declarations`,
    `skips_docs_for_nonexported_declarations`, `skips_docs_for_nonexported_type`,
    `lowers_package_visibility_native_resource`, `collects_doc_for_union_and_enum`,
    `lowers_resource_binding_with_state`, `lowers_nested_function_call_as_match_scrutinee`,
    `function_helper_finds_main`.
  - `externals_state_and_packages.rs` (218, 8) [src 5867-6083]:
    `lowers_with_external_function_metadata`, `write_ir_serializes_to_disk`,
    `lowers_resource_state_binding_and_assign`, `lowers_error_member_access`,
    `lowers_json_csv_crypto_net_http_calls`, `lowers_io_and_thread_and_bits_and_math_more`,
    `lowers_vector_package_typed_dispatch`, `lowers_vector_record_constant`. Local:
    `PROJECT_JSON`, `temp_dir`.
  - `capture_and_shape_arms.rs` (213, 8) [src 6087-6318]:
    `lowers_lambda_capturing_across_expression_kinds`,
    `lowers_lambda_capturing_list_map_with_and_unary`,
    `lowers_lambda_capturing_map_literal_and_constructor`,
    `lowers_inline_trap_treeify_match_with_else_and_no_tail`,
    `lowers_inline_trap_handler_if_both_branches_terminate`,
    `lowers_native_link_function_types_and_returns`,
    `lowers_constructor_positional_and_named_mixed`, `lowers_match_union_variant_binding`.
  - `result_type_inference.rs` (209, 10) [src 6322-6556]:
    `infers_builtin_result_types_in_inferred_lets`,
    `lowers_call_default_padding_for_regex_datetime_crypto`,
    `lowers_general_override_for_builtin_value_type`, `lowers_lambda_capturing_callable_local`,
    `infers_top_level_binding_types`, `lowers_union_with_includes`,
    `lowers_filter_with_inline_lambda_predicate`,
    `lowers_filter_with_builtin_predicate_reference`,
    `lowers_collections_filter_inferred_result_type`,
    `lowers_bare_function_reference_without_captures`.
  - `inferred_types_and_calls.rs` (177, 9) [src 6560-6757]:
    `lowers_inferred_list_literal_element_type`, `lowers_error_constructor_type_inference`,
    `infers_mapentry_key_and_value_types`, `infers_arithmetic_and_trapped_expression_types`,
    `lowers_assignment_bodied_lambda`, `lowers_assignment_bodied_lambda_target_not_on_rhs`,
    `lowers_nested_lambda_capture_skips_inner_lambda`, `lowers_zero_param_func_typed_local_call`,
    `lowers_isolated_function_reference`.
  - `io_fs_inference_and_trap_fallthrough.rs` (122, 5) [src 6759-6912]:
    `lowers_local_call_with_named_and_positional_args`,
    `lowers_tls_connect_default_argument_padding`,
    `lowers_member_access_on_builtin_record_type`,
    `infers_io_and_fs_native_call_result_types`,
    `inline_trap_fallthrough_handler_lowers_linearly`. Local: `count_ops`.
  - `reachable_arms.rs` (184, 12) [src 6918-7153]: `suffixed_float_literal_types_as_float`,
    `negated_exact_numeric_literals`, `untyped_list_literal_element_type_fixed_and_money`,
    `untyped_global_binding_holding_function_value`, `isolated_func_value_call_typing`,
    `foreach_over_map_entry_value`, `collections_filter_with_bare_general_predicate`,
    `assign_bodied_lambda_captures_write_only_target`, `self_including_union_cycle_terminates`,
    `testing_block_item_is_skipped_by_lowering`, `untyped_lambda_param_defaults_to_unknown`,
    `error_call_with_missing_argument_uses_placeholder`. Local: `binding_of`.
  - `reachable_arms_more.rs` (149, 9) [src 7157-7342]: `lambda_calls_captured_function_local`,
    `untyped_let_inline_trap_infers_success_type`,
    `trap_handler_block_can_terminate_walks_all_nested_arms`,
    `untypeable_match_scrutinee_defaults_to_unknown`,
    `untypeable_loop_bounds_and_iterable_default_to_unknown`,
    `untypeable_negation_operand_defaults_to_unknown`,
    `builtin_named_first_then_positional_reorders`,
    `builtin_overloaded_named_arguments_select_overload`,
    `builtin_named_with_surplus_positionals_overflows_to_extras`.
- Count check: 1 + 1 + 20 + 143 + 9 + 103 + 6 = 283, plus the 5 untouched.

### 4.2 `src/ir/variant_corpus_tests.rs` → `src/ir/tests/variant_corpus/` (34 tests)

- `src/ir/mod.rs:56-57`: `#[cfg(test)] mod variant_corpus_tests;` →
  `#[cfg(test)] #[path = "tests/variant_corpus/mod.rs"] mod variant_corpus_tests;`. The module
  path stays `crate::ir::variant_corpus_tests`, so `super` is still `crate::ir`.
  `src/ir/tests/mod.rs` does NOT declare `variant_corpus`.
- `mod.rs` (24): the `use` block (10 lines), 8 topic `mod` lines, and 3 support
  `mod` + `use <support>::*;` pairs. One import edit: `full_bytes` (→ `binary_malformed_input.rs`)
  spells `super::binary::BINARY_REPR_MAGIC`/`…VERSION`, so
  `use super::binary::{decode_binary_repr, encode_binary_repr, verify_package};` becomes
  `use super::binary::{self, decode_binary_repr, encode_binary_repr, verify_package};`.
- `value_builders.rs` (155), support: `loc`, `c`, `empty_project`, `fn_body`, `every_value`
  (5 made `pub(super)`).
- `op_builders.rs` (225), support: `every_op`, `link_function` (2 made `pub(super)`).
- `corpus.rs` (147), support: `variant_corpus` (already `pub`).
- `binary_round_trip.rs` (185, 8) [src 586-818]: `binary_round_trip_over_full_surface`,
  `binary_round_trip_without_link_tables_is_a_bare_trailer`,
  `rejects_a_previous_format_version`, `rejects_an_absurd_cstruct_count`,
  `rejects_an_invalid_slot_direction`, `binary_round_trip_link_expr_variants`,
  `binary_round_trip_link_expr_arithmetic_is_structural`,
  `binary_round_trip_while_op_with_non_while_loop_kind`.
- `binary_malformed_input.rs` (165, 9) [src 822-1007]: `decode_rejects_bad_magic`,
  `decode_rejects_bad_version`, `decode_rejects_too_short_for_magic`,
  `decode_rejects_truncated_body`, `decode_rejects_invalid_utf8_string`,
  `decode_rejects_unknown_op_tag`, `decode_rejects_unknown_value_tag`,
  `decode_rejects_depth_limit`, `decode_surfaces_every_tag_error_branch`. Local: `decode_err`,
  `full_bytes`.
- `verify_package.rs` (79, 4) [src 1011-1097]: `verify_package_accepts_well_formed`,
  `verify_package_rejects_duplicate_type`, `verify_package_recurses_into_for_and_do_until_bodies`,
  `verify_package_rejects_deeply_nested_ops`.
- `package_identity.rs` (214, 2) [src 1101-1338]: `prefix_and_apply_identity_rewrite_all_reference_shapes`,
  `merge_package_dedups_by_name_and_carries_link_tables`.
- `json_projections.rs` (103, 5) [src 1342-1456]: `json_covers_every_op_and_value`,
  `json_covers_all_type_kinds`, `json_project_with_no_bindings_omits_the_bindings_section`,
  `loop_kind_name_covers_all_kinds`, `visibility_name_covers_all_visibilities`. The fn-local
  `use crate::ast::Visibility;` (L1452) is absolute and needs no edit.
- `annotated_type.rs` (215, 1) [src 1460-1674]: `annotated_type_reports_every_annotated_variant`.
- `op_loc.rs` (149, 2) [src 1678-1836]: `op_loc_returns_the_stored_location_for_every_variant`,
  `binary_round_trip_carries_buffers_and_length`.
- `op_loc_buffers.rs` (112, 3) [src 1840-1988]: `rejects_an_over_limit_buffer_count`,
  `rejects_a_crafted_unknown_slot_ctype`, `rejects_crafted_malformed_buffers_after_decode`.

### 4.3 `src/ir/verify/tests.rs` → `src/ir/verify/tests/` (492 tests)

- `src/ir/verify/mod.rs` keeps `#[cfg(test)] mod tests;` (it now resolves to `tests/mod.rs`).
- `mod.rs` (61): the `use` block (8), 49 topic `mod` lines, and
  `mod builders; mod fixtures; use builders::*; use fixtures::*;`. One import edit. Topic
  bodies spell four `super::` paths, and from a topic `super` is now `verify::tests`:
  `super::TypeEnv` (`union_wrap_and_annotations.rs`, and `type_env` in
  `unsendability_cause_walk.rs`), `super::collect_source_diagnostics`
  (`diagnostics_visibility_and_builtin_args.rs`) and `super::is_thread_type`
  (`cbuffer_positions.rs`). So the first line
  `use super::{check, collect_diagnostics};` becomes
  `use super::{check, collect_diagnostics, collect_source_diagnostics, is_thread_type, TypeEnv};`,
  and the bodies resolve through that import unchanged.
- `builders.rs` (224), support (23 made `pub(super)`): `project`, `rules`, `accept`,
  `expect_rule`, `expect_no_rule`, `ret`, `ret_none`, `const_of`, `binary`, `unary`, `bind`,
  `record_typed`, `enum_type`, `binding`, `func`, `func_returns`, `param`, `record`,
  `int_const`, `union`, `union_variant_case`, `money_const`, `eval`.
- `fixtures.rs` (205), support (19 made `pub(super)`): `res_bind_owned`, `transfer_call`,
  `link_fn`, `cstruct`, `project_with_cstructs`, `abi_slot`, `project_with_link`, `eval_call`,
  `close_eval`, `owner_fn`, `THREAD_VIEW`, `WORKER_VIEW`, `inline_trap_ops`, `recover_zero`,
  `unsendable_record`, `bind_in`, `read_of`, `if_op`, `count_rule`.
- Topic files, each `(LOC, tests) [src range]: tests; local helpers`:
  - `member_and_call_arity.rs` (150, 9) [223-397]: `accepts_member_access_on_known_record_field`, `rejects_member_access_on_integer`, `rejects_member_access_on_money_and_scalar`, `rejects_member_access_missing_field_on_record`, `skips_member_access_on_unknown_type`, `rejects_call_with_too_many_arguments`, `accepts_call_omitting_defaulted_argument`, `skips_arity_for_unknown_call_targets`, `rejects_constructor_with_extra_arguments`.
  - `capture_bounds.rs` (173, 5) [401-592]: `rejects_capture_index_past_slot_count`, `accepts_capture_index_within_slot_count`, `rejects_capture_in_non_closure_body`, `ambiguous_closure_arity_does_not_disarm_the_capture_bounds_check`, `a_body_captured_with_two_arities_is_rejected`.
  - `union_wrap_and_annotations.rs` (204, 10) [596-871]: `resource_union_parameter_widening_is_directional`, `an_isolated_callable_is_assignable_to_the_ordinary_one_only`, `rejects_union_wrap_of_foreign_variant`, `accepts_union_wrap_of_real_variant`, `rejects_result_value_with_fabricated_success_type`, `accepts_result_value_with_matching_success_type`, `rejects_union_wrap_with_mismatched_payload`, `rejects_with_update_with_fabricated_type`, `accepts_with_update_matching_target_type`, `rejects_empty_match`.
  - `function_returns_and_params.rs` (96, 9) [875-983]: `accepts_ordinary_function`, `rejects_func_missing_return_type`, `rejects_func_missing_return_value`, `accepts_func_returning_on_all_paths_via_if`, `nothing_func_may_fall_through`, `rejects_param_missing_type`, `rejects_default_arg_order`, `rejects_default_value_mismatch`, `accepts_matching_default_value`.
  - `operators.rs` (179, 14) [987-1188]: `rejects_arithmetic_on_string`, `rejects_and_on_numeric`, `rejects_concat_on_numeric`, `rejects_relational_on_boolean`, `accepts_string_relational`, `accepts_string_concat`, `accepts_boolean_and`, `rejects_equality_incompatible_types`, `rejects_equality_not_comparable`, `accepts_numeric_equality`, `rejects_not_on_numeric`, `rejects_negate_on_string`, `rejects_unknown_unary_operator`, `accepts_not_on_boolean_and_negate_numeric`.
  - `literal_ranges.rs` (131, 8) [1192-1344]: `rejects_byte_overflow`, `rejects_byte_underflow`, `rejects_integer_overflow`, `rejects_negated_integer_overflow`, `rejects_float_overflow`, `rejects_float_underflow`, `rejects_fixed_overflow`, `rejects_fixed_underflow`.
  - `scalar_literals_and_bindings.rs` (231, 15) [1346-1591]: `accepts_fixed_exact_maximum_and_minimum`, `rejects_fixed_literal_rounding_past_either_end`, `accepts_byte_in_range`, `rejects_scalar_out_of_range`, `rejects_scalar_surrogate`, `rejects_negated_scalar`, `accepts_scalar_in_range`, `rejects_let_requires_value`, `rejects_binding_requires_type_or_value`, `rejects_mut_requires_defaultable`, `accepts_mut_defaultable_without_value`, `rejects_binding_type_mismatch`, `rejects_assign_to_immutable`, `rejects_assignment_type_mismatch`, `accepts_valid_assignment`; local `fixed_bind`, `negated_fixed`.
  - `globals_returns_and_exit.rs` (171, 15) [1595-1787]: `rejects_global_binding_requires_type_or_value`, `rejects_global_let_requires_value`, `rejects_global_mut_requires_defaultable`, `accepts_global_with_value`, `rejects_global_binding_type_mismatch`, `rejects_assign_global_immutable`, `rejects_assign_global_type_mismatch`, `rejects_return_mismatch`, `accepts_a_nothing_call_in_statement_position`, `rejects_exit_program_non_integer`, `rejects_exit_program_out_of_range`, `accepts_exit_program_in_range`, `rejects_exit_program_i128_min_without_panic`, `rejects_propagate_outside_trap`, `rejects_fail_non_error`.
  - `loop_exits_and_conditions.rs` (183, 10) [1791-1990]: `rejects_exit_without_loop`, `rejects_continue_without_loop`, `accepts_exit_inside_matching_loop`, `rejects_unreachable_after_exit`, `rejects_recover_literal_lowered_into_a_byte_slot_out_of_range`, `rejects_attributed_string_constructor`, `rejects_if_condition_non_boolean`, `rejects_while_condition_non_boolean`, `rejects_do_until_condition_non_boolean`, `accepts_do_until_valid`.
  - `for_loops.rs` (117, 6) [1994-2117]: `rejects_for_non_numeric_bound`, `rejects_for_step_zero`, `accepts_valid_for`, `for_step_resolved_through_temp`, `rejects_for_each_non_collection`, `accepts_for_each_list`.
  - `match_exhaustiveness.rs` (120, 6) [2121-2255]: `rejects_non_exhaustive_union_match`, `accepts_exhaustive_union_match`, `accepts_union_match_with_else`, `rejects_enum_match_not_exhaustive`, `rejects_match_open_type_without_else`, `rejects_match_pattern_not_a_variant`.
  - `match_patterns_and_constructors.rs` (166, 10) [2257-2434]: `rejects_result_case_not_matchable`, `rejects_match_pattern_requires_union`, `match_guard_and_oneof`, `rejects_when_guard_non_boolean`, `rejects_constructor_requires_record_for_union`, `rejects_constructor_requires_record_for_enum`, `rejects_constructor_arity`, `rejects_constructor_argument_mismatch`, `accepts_valid_constructor`, `rejects_construct_result_implicit`.
  - `with_update_and_literals.rs` (120, 10) [2438-2568]: `rejects_read_only_record_update_error`, `rejects_duplicate_with_field`, `rejects_with_update_field_mismatch`, `accepts_valid_with_update`, `rejects_list_element_mismatch`, `accepts_valid_list_literal`, `rejects_map_key_mismatch`, `rejects_map_value_mismatch`, `accepts_valid_map_literal`, `rejects_map_key_not_comparable`.
  - `value_literals_and_members.rs` (191, 15) [2572-2806]: `deeply_nested_value_hits_depth_cap`, `with_update_unknown_type_infers_read_only_target`, `rejects_with_update_read_only_mapentry`, `with_update_unknown_field_and_uninferable_value_are_skipped`, `rejects_set_literal_element_mismatch`, `accepts_valid_set_literal`, `rejects_money_literal_precision`, `rejects_money_literal_overflow`, `accepts_money_literal_in_range`, `rejects_negated_money_precision`, `rejects_negated_money_underflow`, `accepts_negated_money_literal_in_range`, `rejects_scalar_literal_overflowing_u64`, `accepts_finite_float_and_fixed_literals`, `accepts_enum_member_access`.
  - `money_and_set_elements.rs` (162, 12) [2810-2994]: `member_access_uninferable_target_is_skipped`, `rejects_read_state_on_stateless_resource`, `rejects_money_compared_with_non_money`, `accepts_money_equality`, `rejects_money_plus_non_money`, `accepts_money_add_and_scale`, `rejects_money_times_money`, `rejects_non_money_divided_by_money`, `rejects_money_exponentiation`, `rejects_set_of_union_element_not_comparable`, `accepts_set_of_enum_element`, `accepts_set_of_unknown_type_element`.
  - `compat_deep_arms.rs` (212, 15) [3000-3248]: `rejects_resource_return_order`, `float_owner_is_non_owning`, `os_call_with_valid_args_is_accepted`, `rejects_collections_find_wrong_arity`, `accepts_collections_contains_comparable`, `rejects_result_of_inner_mismatch`, `rejects_union_extract_foreign_variant`, `equality_with_unknown_operand_is_permissive`, `accepts_set_of_comparable_element`, `rejects_set_of_resource_element`, `rejects_set_of_function_element`, `rejects_set_of_collection_element`, `rejects_unknown_enum_member`, `accepts_enum_member`, `accepts_error_member_access_chain`.
  - `type_declarations_and_call_args.rs` (202, 17) [3259-3482]: `rejects_unmarked_resource_field_in_record`, `rejects_res_marker_on_a_non_resource_field`, `accepts_a_res_marked_resource_field_in_record`, `rejects_recursive_record`, `accepts_recursive_record_through_list`, `rejects_empty_enum`, `rejects_union_include_requires_union`, `rejects_union_member_requires_type`, `rejects_mixed_resource_union`, `rejects_duplicate_variant_via_include`, `rejects_local_variant_conflicts_with_include`, `rejects_call_too_few_args`, `rejects_call_argument_type`, `rejects_package_constant_not_callable`, `rejects_calling_non_function_local`, `rejects_builtin_math_bad_args`, `accepts_builtin_math_good_args`.
  - `resource_axis_and_use_after_move.rs` (172, 12) [3486-3689]: `rejects_resource_without_res`, `rejects_res_on_non_resource`, `rejects_collection_resource_element_without_res`, `rejects_collection_res_on_data`, `accepts_collection_res_file`, `accepts_state_on_union`, `rejects_state_type_not_defaultable`, `rejects_param_state_type_not_defaultable`, `rejects_state_assign_no_state`, `rejects_state_assign_mismatch`, `rejects_use_after_close`, `accepts_close_of_a_res_parameter`.
  - `resource_aliases.rs` (133, 8) [3693-3852]: `rejects_use_after_close_through_alias`, `alias_survives_branch_merge`, `rebind_severs_alias`, `foreach_body_move_leaks_to_outer`, `rejects_state_of_cyclic_record`, `trap_only_body_does_not_always_return`, `match_on_uninferable_scrutinee_not_exhaustive`, `match_on_primitive_scrutinee_not_exhaustive`; local `fs_close`, `grab`.
  - `state_agreement_calls.rs` (150, 7) [3858-4049]: `unary_operand_of_uninferable_value_is_skipped`, `call_argument_of_uninferable_value_is_skipped`, `rejects_argument_state_retype`, `rejects_argument_state_missing`, `accepts_argument_state_agreement`, `rejects_thread_transfer_state_retype`, `rejects_thread_transfer_state_missing`.
  - `state_agreement_bindings.rs` (162, 13) [4051-4277]: `rejects_thread_transfer_state_on_bare_plane`, `thread_transfer_with_one_arg_is_skipped`, `thread_transfer_uninferable_args_are_skipped`, `thread_transfer_non_thread_handle_is_skipped`, `accepts_return_state_union`, `rejects_return_state_not_defaultable`, `rejects_binding_opaque_state_narrowing`, `rejects_binding_state_mismatch`, `rejects_bare_binding_of_stateful_initializer`, `accepts_binding_state_agreement`, `binding_state_of_uninferable_initializer_is_skipped`, `accepts_valid_link_function`, `link_library_names_dedups_in_order`.
  - `cstruct_declarations.rs` (84, 9) [4281-4401]: `accepts_valid_cstruct`, `rejects_cstruct_with_no_fields`, `rejects_cstruct_duplicate_field`, `rejects_cstruct_unknown_field_ctype`, `rejects_cstruct_cvoid_field`, `rejects_nested_cstruct`, `rejects_duplicate_cstruct_name_in_one_alias`, `rejects_oversized_cstruct`, `rejects_cstruct_escape_into_wrapper_signature`.
  - `link_native_arms.rs` (118, 9) [4405-4561]: `rejects_cstruct_maps_to_non_record`, `rejects_cstruct_escape_in_return_type`, `rejects_bind_in_unknown_slot`, `rejects_bind_in_slot_not_cstruct`, `rejects_bind_in_unknown_field`, `rejects_bind_in_field_both_param_and_literal`, `rejects_bind_in_field_unknown_param`, `rejects_bind_state_not_out_cstruct_slot`, `rejects_bind_state_without_stateful_return`.
  - `link_native_bind_state.rs` (102, 7) [4565-4690]: `rejects_bind_state_maps_to_mismatch`, `rejects_bind_state_resource_wrong_slot`, `bind_state_resource_with_computed_result_is_skipped`, `rejects_bind_state_resource_against_abi_return`, `rejects_native_resource_state_disagreement`, `set_of_map_of_resource_is_ownership_violation`, `set_of_cyclic_record_with_resource_is_ownership_violation`.
  - `link_expression_slots.rs` (150, 11) [4697-4870]: `rejects_link_expr_naming_no_slot`, `accepts_link_expr_naming_the_abi_return`, `accepts_link_expr_naming_an_abi_slot`, `rejects_struct_slot_with_uncovered_record_field`, `rejects_struct_slot_with_mistyped_record_field`, `rejects_link_unknown_slot_ctype`, `rejects_link_unknown_return_ctype`, `rejects_link_cvoid_argument_slot`, `rejects_link_cptr_escape_in_param`, `rejects_link_cptr_escape_in_return`, `rejects_link_unbound_input_slot`.
  - `link_expression_pins.rs` (129, 8) [4872-5007]: `rejects_link_unbound_slot`, `rejects_link_out_slot_not_return`, `rejects_link_const_out`, `rejects_link_no_result`, `rejects_link_unbound_param`, `rejects_link_const_unknown_slot`, `rejects_link_invalid_free`, `accepts_link_const_pin`.
  - `trap_and_closures.rs` (211, 13) [5015-5256]: `rejects_a_handler_read_of_a_local_declared_after_a_failure`, `accepts_a_handler_read_of_a_local_declared_before_any_failure`, `a_call_to_a_total_project_function_is_not_a_failure_edge`, `an_unproved_built_in_call_is_a_failure_edge`, `a_proven_total_built_in_call_is_not_a_failure_edge`, `a_failed_reassignment_keeps_the_mutable_local_readable`, `rejects_trap_fallthrough`, `accepts_trap_that_returns`, `recover_type_mismatch`, `accepts_closure_valid_capture_in_bind`, `accepts_deeply_nested_but_bounded`, `accepts_global_list_and_map_values`, `rejects_global_byte_overflow`; local `call_eval`, `trap_returning_local`.
  - `diagnostics_visibility_and_builtin_args.rs` (223, 15) [5260-5509]: `collect_source_diagnostics_maps_rules_to_pending`, `collect_source_diagnostics_generated_path_when_file_empty`, `rejects_construct_private_type_cross_file`, `rejects_construct_hidden_field_cross_file`, `rejects_member_access_hidden_field`, `rejects_read_only_record_constructor`, `rejects_term_call_arity`, `rejects_term_call_argument`, `accepts_term_call_valid`, `rejects_collections_call_arity`, `rejects_collections_contains_not_comparable`, `rejects_general_call_arity`, `rejects_general_call_bad_argument`, `accepts_general_len_string`, `rejects_strings_call_bad_args`; local `private_record`.
  - `exhaustive_returns_and_moves.rs` (209, 9) [5513-5764]: `func_returns_via_exhaustive_enum_match`, `func_missing_return_when_match_not_exhaustive`, `accepts_enum_arm_whose_member_names_a_declared_type`, `rejects_enum_arm_over_union_with_same_named_variant`, `rejects_link_return_on_a_nothing_wrapper`, `return_resource_move_is_not_use_after_move`, `rejects_double_move_close_then_return`, `accepts_temporary_in_resource_list`, `rejects_capture_out_of_range_in_bind_value`; local `enum_member_case`.
  - `capture_value_positions.rs` (189, 4) [5775-5984]: `rejects_capture_out_of_range_in_match_pattern_and_guard`, `rejects_stray_capture_in_parameter_default_and_global_initializer`, `accepts_global_read_in_function`, `captures_walked_through_nested_value_shapes`.
  - `coercion_and_include_cycles.rs` (218, 18) [5988-6257]: `accepts_byte_literal_into_byte_param`, `accepts_integer_literal_into_fixed_param`, `accepts_negated_literal_into_fixed_binding`, `accepts_union_variant_return`, `accepts_list_compatible_recursion`, `accepts_map_compatible_recursion`, `poisoned_initializer_yields_unknown_value`, `poisoned_return_yields_unknown_value`, `rejects_map_key_thread_ownership`, `rejects_map_key_record_with_resource`, `rejects_nested_list_mismatch_return`, `rejects_value_return_from_nothing_function`, `accepts_nothing_value_return_from_nothing_function`, `accepts_nested_map_return`, `accepts_result_of_return`, `union_include_cycle_is_bounded`, `accepts_union_with_included_union`, `record_include_cycle_terminates`.
  - `unknown_bounds_and_package_overloads.rs` (122, 9) [6261-6410]: `for_unknown_bound_is_skipped`, `foreach_unknown_iterable_is_skipped`, `rejects_bits_bad_args`, `rejects_encoding_bad_args`, `rejects_io_bad_args`, `rejects_fs_bad_args`, `rejects_net_bad_args`, `rejects_vector_bad_args`, `unknown_package_call_is_skipped`.
  - `closure_capture_walk.rs` (226, 3) [6414-6659]: `closure_body_captures_walked_over_all_shapes`, `accepts_function_with_union_and_result_value_shapes`, `accepts_localref_and_functionref_values`.
  - `collection_element_borrow.rs` (39, 2) [6663-6711]: `accepts_res_bind_of_a_collection_element`, `accepts_return_of_a_resource_collection_element`; local `get_call`.
  - `defaultability.rs` (219, 16) [6715-6986]: `mut_list_is_defaultable`, `mut_map_is_defaultable`, `mut_set_is_defaultable`, `rejects_mut_set_of_resource_ownership_and_comparable`, `mut_enum_is_defaultable`, `mut_record_with_an_enum_field_is_defaultable`, `rejects_mut_record_with_a_union_field_beside_an_enum_field`, `rejects_mut_func_not_defaultable`, `mut_single_variant_enum_is_defaultable`, `mut_record_of_defaultable_fields_ok`, `rejects_mut_record_with_nondefaultable_field`, `accepts_mut_list_of_union_defaultable_empty`, `accepts_mut_map_value_union_defaultable_empty`, `accepts_mut_record_with_list_of_union_field`, `rejects_state_collection_not_a_record`, `rejects_mut_unknown_record_not_defaultable`.
  - `match_coverage.rs` (194, 9) [6990-7215]: `func_returns_via_exhaustive_union_match`, `func_returns_via_match_else`, `func_returns_via_oneof_exhaustive`, `match_on_record_scrutinee_never_covers`, `union_oneof_partial_not_exhaustive`, `enum_missing_member_wording`, `a_qualified_type_name_and_a_bare_one_are_different_types`, `match_guard_reads_union_extract_bind`, `state_assign_on_data_local_skipped`.
  - `nested_moves_and_thread_views.rs` (197, 13) [7219-7476]: `move_in_if_branch_propagates_past_join`, `move_in_match_case_propagates`, `move_in_while_body_propagates`, `close_in_foreach_body_is_accepted`, `res_transfer_moves_source`, `a_thread_view_transfer_moves_the_source`, `a_let_of_a_worker_view_requires_res`, `a_worker_view_alias_moves_the_source`, `a_worker_view_is_a_resource_with_no_close_op`, `returning_a_thread_view_consumes_it`, `passing_a_thread_view_to_a_function_borrows_it`, `res_on_a_thread_binding_is_accepted`, `a_let_thread_binding_requires_res`; local `thread_op`.
  - `early_exits_and_res_kinds.rs` (222, 12) [7480-7740]: `rejects_thread_result_member`, `rejects_match_literal_pattern_type`, `rejects_collections_get_bad_args`, `builtin_arg_check_skipped_when_arg_type_unknown`, `binding_unknown_expected_skips_mismatch`, `union_wrap_empty_member_skipped`, `rejects_res_on_enum`, `rejects_res_on_record`, `rejects_res_on_data_union`, `capture_out_of_range_inside_union_extract`, `capture_out_of_range_inside_result_value_and_member`, `enum_member_access_returns_after_check`.
  - `computed_node_annotations.rs` (222, 6) [7750-7993]: `call_result_annotated_as_a_foreign_record_is_rejected`, `string_call_annotated_integer_cannot_feed_arithmetic`, `call_result_node_annotation_is_reconciled_too`, `a_truthful_call_annotation_is_accepted`, `member_access_annotated_against_its_field_type_is_rejected`, `operator_result_annotations_are_reconciled_with_their_operands`.
  - `cbuffer_positions.rs` (198, 22) [8019-8306]: `accepts_well_formed_cbuffer_link_function`, `rejects_cbuffer_in_slot`, `rejects_cbuffer_inout_slot`, `rejects_cbuffer_without_buffer_clause`, `rejects_cbuffer_with_two_buffer_clauses`, `rejects_buffer_clause_naming_unknown_slot`, `rejects_buffer_clause_naming_non_cbuffer_slot`, `rejects_cbuffer_const_pin`, `rejects_cbuffer_as_abi_return`, `rejects_cbuffer_not_named_by_return`, `rejects_cbuffer_return_with_wrong_wrapper_type`, `rejects_byte_list_return_without_cbuffer_slot`, `rejects_buffer_size_naming_unknown_slot`, `rejects_buffer_size_reading_the_abi_return`, `rejects_buffer_size_reading_an_out_slot`, `accepts_buffer_size_reading_a_const_pin`, `rejects_returned_cbuffer_without_length`, `rejects_length_without_a_cbuffer_result`, `rejects_length_naming_unknown_slot`, `accepts_length_reading_an_out_slot`, `accepts_length_scaling_the_abi_return`, `truncated_thread_spelling_still_counts_as_a_thread`; local `cbuffer_fn`, `cbuffer_project`.
  - `package_path_pilots.rs` (107, 9) [8321-8497]: `accepts_private_isolated_func`, `rejects_isolated_on_a_non_func_kind`, `accepts_public_isolated_func`, `rejects_inline_trap_on_a_non_call`, `rejects_inline_trap_on_a_package_constant`, `accepts_inline_trap_on_a_call`, `skips_the_testing_desugared_trap_guard`, `rejects_unsendable_thread_message_in_a_parameter`, `rejects_unsendable_thread_message_in_a_record_field`.
  - `package_path_thread_messages.rs` (144, 6) [8499-8663]: `rejects_unsendable_message_sent_across_a_thread`, `rejects_transfer_on_a_thread_without_a_resource_plane`, `rejects_unsendable_resource_plane_state_payload`, `rejects_a_non_resource_on_the_resource_plane`, `rejects_a_resource_in_the_message_plane`, `accepts_a_sendable_thread_message`.
  - `unsendability_cause_walk.rs` (223, 13) [8676-8954]: `cause_walk_reports_a_res_marked_element_as_a_resource`, `cause_walk_reports_a_res_map_value_through_both_positions`, `cause_walk_accepts_a_sendable_bare_resource_but_rejects_an_unsendable_one`, `cause_walk_reports_func_and_thread_handle_as_other_not_resource`, `thread_start_does_not_compare_parent_and_worker_state`, `a_thread_may_carry_parent_threads_on_its_resource_plane`, `a_thread_worker_on_a_resource_plane_is_not_transferable`, `cause_walk_descends_into_a_record_field_and_names_the_leaf`, `cause_walk_reports_the_first_blocking_field_left_to_right`, `cause_walk_accepts_every_plain_sendable_shape`, `the_resource_plane_keeps_the_generic_unsendable_rule`, `func_and_thread_handle_planes_keep_the_generic_unsendable_rule`, `cause_walk_terminates_on_a_self_referential_record`; local `type_env`, `cause_for`.
  - `package_path_general.rs` (157, 9) [8960-9138]: `warns_dead_handler_on_an_infallible_inline_builtin`, `rejects_normal_flow_reaching_the_trap`, `a_stray_recover_counts_as_diverging`, `rejects_a_thread_handle_as_a_list_element`, `rejects_a_thread_handle_as_a_map_value`, `accepts_res_thread_elements_values_and_fields`, `rejects_a_bare_thread_record_field`, `rejects_a_thread_worker_as_a_list_element`, `rejects_a_resource_in_a_set_literal`.
  - `package_path_captures.rs` (147, 8) [9140-9314]: `rejects_a_thread_carrying_union_as_a_map_key`, `rejects_a_by_value_capture_of_a_mut_local`, `accepts_a_by_ref_capture_of_a_mut_local`, `accepts_a_resource_capture_in_either_shape`, `rejects_a_capture_of_a_worker_view`, `rejects_a_capture_of_a_record_reaching_a_worker_view`, `accepts_a_capture_of_a_parent_thread_view`, `accepts_a_copyable_immutable_capture`; local `closure_bind`.
  - `package_path_link_struct_slots.rs` (128, 11) [9324-9528]: `rejects_inout_on_a_non_cstruct_slot`, `rejects_returning_an_in_struct_slot`, `rejects_returning_a_struct_slot_as_another_type`, `rejects_bind_in_on_an_out_slot`, `rejects_bind_in_setting_a_field_twice`, `rejects_bind_in_field_bound_to_nothing`, `rejects_an_unbound_in_struct_slot`, `accepts_a_bound_in_struct_slot`, `accepts_a_body_that_returns_before_its_trap`, `read_only_records_are_refused_under_their_qualified_identity`, `a_close_on_one_arm_does_not_move_the_resource_on_its_sibling`; local `struct_slot_project`.
  - `transfer_success_edge.rs` (187, 9) [9535-9742]: `a_read_after_a_propagating_transfer_is_a_use_after_move`, `a_read_before_a_transfer_is_not_a_use_after_move`, `a_transfer_moves_a_possible_alias`, `an_inline_failure_arm_may_use_the_untransferred_resource`, `the_inline_success_arm_sees_the_resource_moved`, `a_read_after_an_inline_transfer_join_is_a_use_after_move`, `a_diverging_success_arm_does_not_poison_the_failure_continuation`, `a_check_chain_handler_may_use_the_untransferred_resource`, `a_read_after_a_check_chain_join_is_a_use_after_move`; local `RES_PLANE`, `transfer_value`, `result_is_ok`, `transfer_fn`, `inline_transfer`, `chain_transfer`.
  - `state_type_is_record.rs` (132, 3) [9747-9901]: `rejects_state_type_that_is_not_a_record`, `accepts_state_type_that_is_a_record`, `rejects_non_record_state_at_every_position`; local `state_binding_rules`.
  - `state_type_positions.rs` (157, 3) [9907-10080]: `rejects_state_on_a_non_resource_at_every_position`, `res_on_a_non_resource_is_still_the_res_rule`, `imported_types_are_judged_by_their_kind`.

### 4.4 `src/ir/resource_escape.rs` inline tests → `src/ir/resource_escape/tests/` (28 tests)

- `resource_escape.rs`: the inline block (L856-1777) is replaced by `#[cfg(test)] mod tests;`.
  The test LOC left in the host is 17: the `#[cfg(test)]` fns `owner`, `floats`,
  `analyze_function`, plus the 2-line declaration.
- `tests/mod.rs` (143): the `use` block (3, keeping `use super::*;`), 4 topic `mod` lines;
  shared helpers `func`, `res`, `list`, `open`, `append`, `ident`, `holder`, `construct`,
  `with_update`, `call`, `owner_after_inner_assign`, `reads`.
- `record_edges.rs` (153, 7) [945-1176]: `inner_resource_floats_to_an_outer_record_via_with`,
  `same_scope_record_does_not_float`, `a_constructor_nested_in_a_list_routes_to_the_list`,
  `a_record_with_no_resource_argument_routes_nothing`,
  `a_positional_constructor_argument_routes_the_same_as_a_named_one`,
  `with_update_carries_the_targets_existing_contents`,
  `an_inferred_record_binding_is_still_seen_by_the_ordering_gate`.
- `return_order_and_collections.rs` (156, 7) [1187-1393]:
  `a_res_parameter_is_never_blocked_by_return_order`,
  `a_returned_record_declared_after_its_resource_is_blocked`,
  `a_returned_record_without_a_res_field_is_not_blocked`,
  `a_returned_record_declared_before_its_resource_floats`,
  `same_scope_collection_does_not_float`, `inner_resource_floats_to_outer_collection`,
  `a_resource_floats_through_a_repackaging_member`.
- `collection_floats.rs` (140, 6) [1399-1554]: `a_scalar_or_element_result_is_not_a_float_target`,
  `an_element_read_put_into_a_container_floats_its_source`,
  `resource_in_returned_collection_floats_to_it`, `bare_resource_return_does_not_float`,
  `a_borrowed_element_never_floats`, `float_follows_collection_copy_chain`.
- `closures_and_globals.rs` (172, 8) [1561-1776]:
  `a_resource_captured_into_an_outer_closure_floats_to_it`, `a_lambda_parameter_is_not_a_capture`,
  `a_returned_closure_over_a_later_record_is_blocked`,
  `a_returned_closure_binding_declared_after_its_resource_is_blocked`,
  `a_resource_stored_into_a_global_floats_to_the_global`,
  `a_global_outranks_an_outer_local_collection`,
  `a_parameter_stored_into_a_global_does_not_float_statically`,
  `an_assignment_to_an_unknown_name_is_not_a_global`. Local: `lambda`, `closure_binding`,
  `assign`, `globals`.
- Citation: `src/docs/spec/architecture/23_escape-analysis.md:358`
  `[[src/ir/resource_escape.rs:tests]]` → `[[src/ir/resource_escape/tests/]]`. The citation
  closes the table of escape cases, which are these tests. The directory form follows the
  precedent of `[[src/codegen/]]` in the spec. `tests/guards/no_type_strings.rs`'s mention of
  `decl_type` names code (unaffected).

### 4.5 `src/ir/shape.rs` inline tests → `src/ir/shape/tests/` (58 tests)

- `shape.rs`: the inline block (L3767-end of the block) is replaced by
  `#[cfg(test)] mod tests;`. The test LOC left is 23: the `bound_types` probe
  field/initializer/push and the declaration.
- `tests/mod.rs` (41): the `use` block (3), 7 topic `mod` lines; shared `hir_from`,
  `shape_codes`, `rejects_with`, `accepts`.
- `walker_parity.rs` (116, 2) [3771-3946]: `walker_types_bindings_exactly_as_lowering_does`,
  `check_project_accepts_clean_source`. Local: `lowered_binds`.
- `named_arguments.rs` (210, 17) [3950-4192]: `user_named_argument_valid`,
  `user_named_argument_unknown_name`, `user_named_argument_duplicate`,
  `user_named_positional_after_named_walk`,
  `user_private_function_in_another_file_is_not_a_call_target`,
  `imported_package_function_named_arguments`, `builtin_named_argument_valid`,
  `builtin_named_argument_unknown_name`, `builtin_named_argument_matching_parameter_name_accepted`,
  `builtin_named_argument_duplicate`, `builtin_named_then_positional_reorders`,
  `builtin_general_call_named_argument`, `overloaded_named_unknown_argument_rejected`,
  `overloaded_named_duplicate_argument_rejected`,
  `overloaded_duplicate_reported_once_and_before_unknown_names`,
  `overloaded_named_valid_selection_accepted`, `function_value_call_count_rejected`.
  Local: `wrap_import`.
- `argument_types.rs` (209, 14) [4196-4431 as of 17a9b5799, plus the 18-LOC test added by
  `267088179`]: `declared_function_argument_type_mismatch`,
  `declared_function_literal_coercions_accepted`, `builtin_table_argument_type_mismatch`,
  `general_error_call_argument_mismatch`, `thread_start_entry_must_be_an_isolated_func`,
  `thread_start_accepts_a_bare_local_isolated_func`,
  `thread_start_accepts_a_bare_private_isolated_func`,
  `term_draw_text_attributed_requires_astrings_import`,
  `term_measure_text_attributed_requires_astrings_import`, `cascade_spares_typed_seam_cases`,
  `export_in_executable_flags_each_item_kind`,
  `export_in_executable_empty_for_package_project`,
  `export_in_executable_no_export_no_diagnostic`,
  `matched_table_builtin_call_with_an_unknown_argument_is_typed`.
- `package_metadata.rs` (202, 3) [4435-4672]: `corrupt_package_tables_are_package_invalid`,
  `package_type_validation_arms`, `a_dependency_s_builtin_types_resolve_without_the_importer_s_import`.
- `link_constructors_money.rs` (147, 5) [4676-4837]: `const_pin_must_fold_to_an_immediate`,
  `free_deallocator_signature`, `error_records_cannot_be_constructed`,
  `constructor_named_field_set_twice`, `money_scaled_by_a_bare_decimal_literal_warns`.
  Local: `link_project`.
- `testing_and_control_flow.rs` (215, 12) [4841-5083]: `expect_arity_forms`,
  `expect_typed_operands_must_be_the_named_type`,
  `expect_equal_operands_must_be_comparable_and_printable`,
  `expect_trap_code_must_be_integer`, `expect_trap_requires_a_call_to_guard`,
  `exit_func_is_forbidden_and_ends_the_path`,
  `unreachable_after_exit_func_names_each_following_statement`,
  `loop_exit_tail_inside_a_handler_is_shapes`, `recover_outside_a_handler_and_the_count_forms`,
  `inline_trap_handler_must_diverge_on_every_path`,
  `nested_call_rule_follows_enclosing_call_rule`,
  `excess_positional_arguments_are_counted_in_the_arity_detail`. Local: `tcase`.
- `foreign_record_fields.rs` (96, 5) [5100-5216]:
  `foreign_record_field_as_builtin_argument_is_rejected`,
  `foreign_record_field_as_user_func_argument_is_rejected`,
  `unrelated_import_does_not_make_a_foreign_record_field_readable`,
  `foreign_record_field_with_the_owning_import_is_accepted`,
  `foreign_record_field_in_a_binding_still_reports`. Local: `shape_reports`.
- Hazards: 21 citations, all to code symbols outside the moved block (none move).
  - `tests/guards/no_type_strings.rs` allowlists `("src/ir/shape.rs", "resource_types")` and
    `("src/ir/shape.rs", "types")`, and the doc ref `src/ir/shape.rs:158 bound_types` names
    code. All of them are untouched by G.
  - The `.ai` refs (`compiler.md`, `collections.md`, `codegen-invariants.md`,
    `resources-packages.md`) name code.

### 4.6 `src/monomorph/helpers.rs` inline tests → `src/monomorph/helpers/tests/` (26 tests)

- `helpers.rs`: the block (L781-1538) is replaced by `#[cfg(test)] mod tests;`. The file-top
  `#[cfg(test)] use crate::operators::BinaryOp;` stays in `helpers.rs`, and the tests reach it
  through `use super::*`. The test LOC left is 4.
- `tests/mod.rs` (44): the `use` block (6), 3 topic `mod` lines; shared `subs`, `param`, `func`.
- `unify.rs` (182, 7) [790-1050]: `unify_agrees_modulo_the_res_marker`,
  `unify_binds_and_checks_template_params`, `unknown_actual_never_poisons_a_param_binding`,
  `unify_recurses_into_all_container_shapes`, `unify_recurses_into_thread_shapes`,
  `unify_rejects_mismatched_container_shapes`,
  `unify_treats_unknown_actual_as_wildcard_and_matches_concretes`. Local: `unify_str`.
- `type_parts_and_mangling.rs` (228, 10) [1055-1313]:
  `func_type_parts_handles_isolated_and_empty_params`,
  `nested_function_type_arguments_are_not_shredded`,
  `substitution_walks_each_top_level_type_argument`,
  `user_template_parts_excludes_builtin_shapes`, `substitute_type_params_rewrites_every_shape`,
  `callable_matches_compares_the_complete_callable_type`,
  `reference_signature_defaults_a_missing_return_to_nothing`,
  `arg_slot_expected_for_calls_and_bare_names`, `mangle_and_sanitize_encode_types`,
  `overload_concrete_name_encodes_params_and_return`. Local: `substitute_str`, `func_parts`,
  `user_parts`, `PARSE`.
- `overload_keys_and_accessors.rs` (192, 9) [1315-1537]: `overload_key_includes_return_type`,
  `param_types_eq_and_params_match`, `arg_slot_expected_only_for_call_arguments`,
  `constructor_arg_field_type_positional_and_named`, `arg_and_constructor_value_accessors`,
  `numeric_result_and_loop_promotion`, `split_helpers`,
  `collect_imported_overloads_empty_without_imports`,
  `collect_imported_overloads_reads_package_overload_set`.
- Hazards: 16 citations, all to code symbols. `.ai/codegen-invariants.md` and
  `no_type_strings.rs:989` name code (`unify_type`, `collect_imported_records`).

### 4.7 `src/monomorph/lower.rs` inline tests → `src/monomorph/lower/tests/` (45 tests)

- `lower.rs`: the block (L2975-4526) is replaced by `#[cfg(test)] mod tests;`. The test LOC left is 2.
- `tests/mod.rs` (58): the `use` block (3, keeping `use super::super::{ImportedOverload, Monomorphizer};`),
  plus `use super::{mangle_name, strip_qualifier_prefixes};`. Topic bodies spell
  `super::mangle_name` (`generic_instantiation.rs`) and `super::strip_qualifier_prefixes`
  (`lambda_and_type_compat.rs`), and from a topic `super` is `lower::tests`. Then 7 topic
  `mod` lines and the shared `project`, `monomorphize`, `monomorphize_files`, `functions`,
  `types`, `function_names`.
- Bodies spelling `super::super::monomorphize_project` / `super::super::MAX_TOTAL_INSTANTIATIONS`
  (`generic_instantiation`, `imported_overloads`, `instantiation_placement`) now resolve one
  level down, to `monomorph::lower::…`, through `lower.rs`'s `use super::*;` glob. They reach
  the same items with no edit. Letter M's code split of `lower.rs` keeps `use super::*;` in the
  file that declares `mod tests` (G records this as M's input; G itself depends only on today's
  `lower.rs`).
- `generic_instantiation.rs` (173, 6) [2982-3262]: `generic_function_instantiated_per_argument_type`,
  `generic_function_deduplicates_repeated_instantiation`,
  `total_instantiation_budget_halts_wide_fanout`,
  `instantiate_type_disambiguates_mangle_colliding_arguments`,
  `nested_generic_container_argument_unifies`,
  `generic_type_instantiated_from_expected_constructor_type`.
- `overload_and_control_flow.rs` (201, 9) [3264-3488]: `overload_selected_by_parameter_type`,
  `return_type_overload_selected_by_expected_type`, `control_flow_forms_are_lowered`,
  `for_loop_float_bound_promotes_counter_type`, `match_union_variant_binding_is_lowered`,
  `arity_mismatch_reports_error`, `top_level_binding_value_is_lowered`,
  `trap_body_is_lowered`, `plain_program_without_generics_passes_through`.
- `generic_inference.rs` (214, 9) [3490-3735]: `generic_over_map_and_result_container_params`,
  `generic_type_inferred_from_constructor_arguments`,
  `constructor_infers_param_despite_unknown_field_declared_first`,
  `record_member_access_and_with_update_types_are_inferred`,
  `list_literal_and_string_concat_and_unary_types`,
  `general_builtin_override_selected_for_user_type`,
  `overload_no_match_leaves_call_unresolved`,
  `return_type_overload_ambiguous_without_expected_type_errors`,
  `template_argument_unification_failure_errors`.
- `lambda_and_type_compat.rs` (177, 3) [3737-3963]: `lambda_expression_type_is_inferred`,
  `types_compatible_matches_the_token_algorithm`, `normalize_type_matches_the_string_algorithm`.
- `imported_overloads.rs` (216, 9) [3968-4227]:
  `an_untyped_empty_collection_makes_an_imported_overload_ambiguous`,
  `imported_overload_call_is_rewritten_to_package_symbol`,
  `match_literal_oneof_and_else_patterns_lower`, `for_each_over_map_binds_map_entry_type`,
  `named_constructor_arguments_are_lowered`, `encoding_utf8_encode_overload_resolves_to_bytes`,
  `encoding_utf8_encode_overload_ambiguous_without_expected_type`,
  `encoding_utf8_encode_wrong_arg_type_leaves_call`, `bare_list_literal_argument_type_is_inferred`.
- `instantiation_placement.rs` (201, 8) [4229-4468]: `imported_overload_matches_argument_by_type`,
  `ok_and_error_constructor_types_are_inferred`, `two_generic_instantiations_are_emitted_sorted`,
  `return_type_only_template_param_is_a_reported_error`,
  `generic_set_literal_lowers_its_element_type`,
  `a_private_overload_is_one_set_with_its_files_public_sibling`,
  `a_user_instantiation_is_emitted_into_its_templates_file_in_either_order`,
  `a_toolchain_templates_instantiation_stays_in_the_first_file`. Local: `returned_callee`,
  `file_of`, `TEMPLATES`, `CALLER`.
- `import_binding_shadowing.rs` (53, 1) [4470-4525]:
  `a_hosting_files_own_import_binding_wins_and_a_shadowed_package_stays_known`.
- Hazards: 31 citations, all to code symbols. `.ai/codegen-invariants.md` names
  `lower_expression` (code).

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

Every phase uses the same gate, run in this order.

- **G0 (once, before Phase 1's first edit):**
  - `cargo test -p mfb --bin mfb -- --list 2>/dev/null | grep -c ': test$' > /tmp/g_before.txt`
  - `cargo check --all-targets 2>&1 | grep -c '^warning' > /tmp/g_warn_before.txt`
    (est. 5 min cold).
- **G1:** the same two `--list`/warning commands after the phase's edits → numbers equal the G0 files.
- **G2:** run the moved modules' tests (filter per phase) → all pass, 0 failed. This is an
  actual `test result: ok` line, not inferred (`.ai/testing-gates.md`).
- **G3:** `cargo test --test architecture_guards --test no_type_strings --test no_operator_strings`
  → all pass (est. 2 min).
- **G4:** `cargo test --test file_size_budget 2>&1 | grep -E '^  (NEW|GREW)'` → no output.
  Every `TIGHTEN`/`STALE` row names a file of this phase. Then
  `MFB_LOC_TIGHTEN=1 cargo test --test file_size_budget`, rerun →
  `grep -cE '^  (NEW|GREW|STALE|TIGHTEN)'` → `0`. Commit `tests/guards/file_size_ceilings.txt`
  with the phase (est. 2 min).
- **G5:** `cargo fmt --check` → no output (est. 20 s).

### Phase 1 — monomorph inline tests

- [ ] Move `src/monomorph/helpers.rs`'s inline tests to `src/monomorph/helpers/tests/` per §4.6.
- [ ] Move `src/monomorph/lower.rs`'s inline tests to `src/monomorph/lower/tests/` per §4.7
      (`use super::{mangle_name, strip_qualifier_prefixes};` in `tests/mod.rs`).
- [ ] G4: tighten the two `test` rows (the hosts' `code` rows are M's and do not move).

Acceptance: the monomorph tests run from their new modules and the count did not move.
  Check: G1 → equal. G2:
  `cargo test -p mfb --bin mfb monomorph::helpers::tests:: monomorph::lower::tests::` → 0
  failed, and `cargo test -p mfb --bin mfb -- --list 2>/dev/null | grep -cE '^monomorph::(helpers|lower)::tests::'`
  → 26 + 45 = 71. G3; G4; G5 (est. 8 min).
Commit: —

### Phase 2 — resource_escape and shape inline tests

- [ ] Move `src/ir/resource_escape.rs`'s inline tests to `src/ir/resource_escape/tests/` per §4.4.
- [ ] `src/docs/spec/architecture/23_escape-analysis.md:358`: `[[src/ir/resource_escape.rs:tests]]`
      → `[[src/ir/resource_escape/tests/]]`.
- [ ] Move `src/ir/shape.rs`'s inline tests to `src/ir/shape/tests/` per §4.5.
- [ ] G4: tighten the two `test` rows.

Acceptance: as Phase 1, and the spec still resolves.
  Check: G1. G2: `cargo test -p mfb --bin mfb ir::resource_escape::tests:: ir::shape::tests::`
  → 0 failed, and the `--list` count of `^ir::(resource_escape|shape)::tests::` → 28 + 58 = 86.
  `cargo test -p mfb --bin mfb spec_citations_resolve` → passes, and
  `./target/debug/mfb spec architecture escape-analysis 2>/dev/null | grep -c '\[\['` → 0
  (est. 4 min). G3; G4; G5.
Commit: —

### Phase 3 — variant corpus

- [ ] Create `src/ir/tests/variant_corpus/` per §4.2 and delete `src/ir/variant_corpus_tests.rs`.
- [ ] `src/ir/mod.rs`: `#[cfg(test)] #[path = "tests/variant_corpus/mod.rs"] mod variant_corpus_tests;`.
- [ ] G4: tighten (the `src/ir/variant_corpus_tests.rs` row is STALE).

Acceptance: the 34 corpus tests run under the unchanged module path.
  Check: G1. G2: `cargo test -p mfb --bin mfb ir::variant_corpus_tests::` → 0 failed, and the
  `--list` count of `^ir::variant_corpus_tests::` → 34. G3 → passes: the files are under
  `/tests/`, so both string guards still exempt them. G4; G5 (est. 6 min).
Commit: —

### Phase 4 — `src/ir/tests.rs`

- [ ] Replace `src/ir/tests.rs` with `src/ir/tests/mod.rs` and the topic files and directories
      of §4.1. `inline_trap_short_circuit.rs` is untouched.
- [ ] The two fn-local `use` edits (one more `super::`) in `external_and_write_ir.rs` and
      `named_argument_normalization.rs`.
- [ ] G4: tighten (the `src/ir/tests.rs` row is STALE).

Acceptance: the IR lowering tests run from their new modules, and the external helper path
still resolves.
  Check: G1. G2: `cargo test -p mfb --bin mfb ir::tests::` → 0 failed, and the `--list` count
  of `^ir::tests::` → 288 (283 + 5). `cargo test -p mfb --bin mfb target::shared::config_fold`
  → 0 failed (the `crate::ir::tests::helpers::lower_src` caller). G3; G4; G5 (est. 8 min).
Commit: —

### Phase 5 — `src/ir/verify/tests.rs` (largest blast radius)

The 492 verifier tests over 52 files carry most of G's visibility and import edits.

- [ ] Replace `src/ir/verify/tests.rs` with `src/ir/verify/tests/mod.rs`, `builders.rs`,
      `fixtures.rs` and the 49 topic files of §4.3 (42 `pub(super)` edits; the widened
      `use super::{…}` line).
- [ ] G4: tighten (the `src/ir/verify/tests.rs` row is STALE).

Acceptance: every verifier test runs from its topic, and the count did not move.
  Check: G1. G2: `cargo test -p mfb --bin mfb ir::verify::tests::` → 0 failed, and the
  `--list` count of `^ir::verify::tests::` → 492. Then for the topic count,
  `ls src/ir/verify/tests/*.rs | wc -l` → 52. G3; G4; G5 (est. 8 min).
Commit: —

## Validation Plan

- Tests: no test is added or removed. Per phase, the whole-binary `--list` count equals G0's
  (G1), and the moved modules' tests pass (G2) with the per-module counts given in each phase:
  71, 86, 34, 288, 492.
- Coverage check: per-file coverage is not a plan-176 gate — author, 2026-10-17. Evidence: at 7a4b09970,
  `sh scripts/coverage.sh --bins` in a detached worktree (59 min; 4989 unit tests passed), then
  `sh scripts/coverage-check.sh` exited 1. 241 `src/` files are already below the 98% per-file
  floor ("Files below 98% line coverage (GATE FAILURE)"), and overall coverage is 93.67%.
  Examples among plan-176 hosts: `src/ir/shape.rs` 93.21%,
  `src/codegen/engine/control/builder_control.rs` 75.55%, `src/debugger/values.rs` 86.66%. A
  pure move changes no executed line, and moving an inline test block only removes test lines
  from its host's denominator. The pre-existing gate failure is reported to the owner
  separately and is not a plan-176 task.
- Runtime proof: Phase 4's `config_fold` run (the external user of `ir::tests::helpers`) and
  Phase 3's unchanged `ir::variant_corpus_tests::` path show the module paths callers depend
  on are intact.
- Doc sync: `src/docs/spec/architecture/23_escape-analysis.md` (one citation). No `.ai` doc,
  `mfb man` topic or other spec topic names moved code (§2).
- Final gate (once, after Phase 5):
  - `cargo test -p mfb --bin mfb spec_citations_resolve` and
    `cargo test --test architecture_guards --test no_type_strings --test no_operator_strings --test file_size_budget`
    → all pass, and the banner has 0 rows.
  - `grep -cE 'src/ir/(tests|variant_corpus_tests|verify/tests)\.rs|ceiling test [0-9]+ src/(ir/(resource_escape|shape)|monomorph/(helpers|lower))\.rs' tests/guards/file_size_ceilings.txt`
    → 0.
  - `cargo clippy --all-targets 2>&1 | grep -c '^warning'` → equal to its value at G0
    (est. 8 min).
  - The full workspace suite runs once, in plan-176-V.

## Decisions

- Rule: code ≤ 1000, test ≤ 250 non-comment non-blank LOC. Scope: `.rs`/`.mfb` under `src/`
  `packages/` `examples/` `repository/` `tests/` — owner, 2026-10-14.
- Inline test code counts against the 250 test budget; over 250 it moves to its own module,
  split further as needed — owner, 2026-10-14.
- Generated files are exempt. The guard warns with a banner and never fails. A new module is
  always an acceptable split — owner, 2026-10-14.
- A single test-side unit over 250 gets its own file and an `exempt` row — owner, 2026-10-14.
  G has none. Every topic in §4 is ≤ 250; the largest is `scalar_literals_and_bindings.rs` at
  231.
- Extracted src test modules go into `foo/tests/mod.rs` + topic files, never `foo/tests.rs` —
  plan-176-A. Coverage `IGNORE` and the string guards recognise the `tests/` directory.
- `variant_corpus_tests` is loaded by `#[path = "tests/variant_corpus/mod.rs"]` so that its
  module path and `super` stay unchanged while it moves under `tests/` — author.
- Guards that classify test code by file name are updated in the same commit as the move that
  needs them — author, 2026-10-14. G needs none: the string guards already exempt `/tests/`,
  and the architecture guards find no needle in any moved line (measured, §2). Plan-176-H adds
  the `/tests/` rule to `architecture_guards`.
- Glob-cycle imports (`use super::*;` in parts, `use part::*;` in `mod.rs`) compile without
  warnings — verified (`/tmp/loc/probe2`).
- Per-file coverage is not a plan-176 gate; the measured evidence is in §Validation Plan
  "Coverage check" — author, 2026-10-17.

## Corrections

## Summary

G is pure motion of IR and monomorph test code into `tests/` directory modules: 7 hosts, 971
tests (288 + 34 + 492 + 28 + 58 + 26 + 45, counting the 5 untouched), 0 renamed, no exemption.
The risk is the compile-only failure mode of a test split (a missed `super::` level or a helper
left private), concentrated in `ir::verify::tests`. That phase is last, gated by an actual test
run of the module. Host code, host paths and every code citation stay where they are. Letters
M and N split the code next.
