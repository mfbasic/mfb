# plan-176-G: IR and monomorph test code

Last updated: 2026-10-18
Effort: medium (1h–2h)
Depends on: plan-176-F

G moves the over-limit test code under `src/ir` and `src/monomorph` into `tests/` directory
modules. Under the plan-176 rule a test-only file may hold at most 1000 LOC, and a code file
at most 250 inline test LOC. Every file G creates targets ≤ 900. Seven files are in scope:

- three test-only files over 1000: `src/ir/tests.rs` (6386), `src/ir/variant_corpus_tests.rs`
  (1748) and `src/ir/verify/tests.rs` (8495);
- four code files whose inline `#[cfg(test)] mod tests { … }` is over 250:
  `src/ir/resource_escape.rs` (774), `src/ir/shape.rs` (1231), `src/monomorph/helpers.rs`
  (643) and `src/monomorph/lower.rs` (1292).

No test is added, removed or renamed. Some tests keep their module path exactly. These are the
inline modules that stay in `src/ir/tests/mod.rs`, the single-file `binary_repr_tests`, and the
two inline hosts that become one `tests/mod.rs` each. The rest gain one path segment, the part
file (`ir::verify::tests::rejects_for_step_zero` →
`ir::verify::tests::literals_bindings_and_loops::rejects_for_step_zero`). The host code files
keep their code and their paths. Their code-side splits are letters M and N.

The single outcome: after G lands, `cargo test --test file_size_budget` shows no row for any of
G's 7 files. `tests/guards/file_size_ceilings.txt` has no `ceiling test` row for the three test
files and no `ceiling inline` row for the four hosts. No file G creates gets a row.

References:

- `planning/plan-176-A-file-size-guard.md`: the rule (`code` ≤ 1000, `inline` ≤ 250,
  test-only file `test` ≤ 1000), §Decisions, and "Splitting an over-budget file" in
  `.ai/build-tooling.md` (src tests: `foo/tests/mod.rs`, plus part files only when it is over
  1000; the coverage `IGNORE` of `(^|/)tests/`).
- `.ai/testing-gates.md` § "Splits must sweep man AND spec citations". It also notes that a
  split can leave the release build green while the TEST build is broken, so the gate is an
  actual `test result: ok` line from a `cargo test -p mfb --bin mfb <module>::` run.
- `.ai/codegen-invariants.md`: names `unify_type`, `collect_imported_records` and
  `lower_expression`. All three are code symbols that stay in their host files.
- `.ai/build-tooling.md`: rustfmt/clippy policy.
- Spec topic touched: `src/docs/spec/architecture/23_escape-analysis.md:358`
  (`[[src/ir/resource_escape.rs:tests]]`). Every other spec citation of a G host names code
  that stays (measured per file in §4).
- Guards:
  - `tests/guards/no_type_strings.rs` and `tests/guards/no_operator_strings.rs`:
    `is_test_file` exempts `/tests/` paths.
  - `tests/guards/architecture_guards.rs`: `no_hand_picked_vreg`, `emission_lines` and
    `only_testutil_installs_a_panic_hook` scan all of `src/`. The file is 370 test LOC, under the
    1000 limit, so no letter splits it.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-176-F is complete and archived | `ls planning/plan-176-F-* planning/completed/plan-176-F-*` → only `planning/completed/…` | MET (2026-10-19) |
| The file-size guard exists and passes | `cargo test --test file_size_budget` → passes | MET (2026-10-19) |
| G's files have no uncommitted changes | `git status --porcelain -- src/ir/tests.rs src/ir/tests src/ir/variant_corpus_tests.rs src/ir/verify/tests.rs src/ir/resource_escape.rs src/ir/shape.rs src/ir/mod.rs src/monomorph/helpers.rs src/monomorph/lower.rs src/docs/spec/architecture/23_escape-analysis.md` → empty | MET (2026-10-19) |

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

- Every file G creates is ≤ 900 LOC: `python3 -I tools/loc-census/census.py --file <path>`
  → second field ≤ 900. For a test-only file that field is the file's total LOC.
- Each host keeps ≤ 250 inline test LOC: the same command on the host → second field ≤ 250.
- `cargo test --test file_size_budget` prints no row naming a G path. The ceiling file holds no
  `ceiling test` row for a G test file and no `ceiling inline` row for a G host.
- The `mfb` unit-test binary lists the same number of tests before and after each phase.

### Non-goals (explicit constraints)

- No behavior change. No test body is edited beyond moving it. A body is dedented one level
  when it leaves an inline `mod … { }`. The other allowed edits are listed in §4:
  - adding `use super::*;` at the top of a part file;
  - one more `super::` on two fn-local `use`s;
  - three import-line edits (one widened list in `variant_corpus_tests`, one in
    `verify::tests`, one added line in `monomorph::lower::tests`).
  No item changes visibility.
- No `#[test]` fn is renamed, added or removed. No `[[test]]` target is touched.
- No host code moves. `resource_escape.rs`, `shape.rs`, `helpers.rs` and `lower.rs` keep
  every non-test line. Letters M and N split the code.
- `crate::ir::tests::helpers::lower_src` (external caller `src/target/shared/config_fold.rs:462`)
  keeps its path, and so does `crate::ir::variant_corpus_tests`.
- No guard weakens. No production file becomes exempt from any guard.

## 2. Current State

The LOC counts come from the planning census `python3 -I /tmp/loc/census.py "$PWD"` at HEAD
`ce7a995ca` (2026-10-18; `/tmp/loc/census.json`). Block extents and LOC come from
`python3 -I /tmp/loc/span.py <file> <a> <b>` at the same HEAD. Test counts come from
`grep -c '#\[test\]' <file>`.

| File | Kind | LOC | Tests |
|---|---|---|---|
| `src/ir/tests.rs` | test-only (`#[cfg(test)] pub(crate) mod tests;`, `src/ir/mod.rs:65-66`) | `test` 6386 | 285 (+5 in the existing `src/ir/tests/inline_trap_short_circuit.rs`, 147 LOC) |
| `src/ir/variant_corpus_tests.rs` | test-only (`#[cfg(test)] mod variant_corpus_tests;`, `src/ir/mod.rs:56-57`) | `test` 1748 | 34 |
| `src/ir/verify/tests.rs` | test-only (`#[cfg(test)] mod tests;`, `src/ir/verify/mod.rs:1838-1839`) | `test` 8495 | 493 |
| `src/ir/resource_escape.rs` | `inline` (`mod tests` block L856-1777, 759 LOC) | `inline` 774 (code 495) | 28 |
| `src/ir/shape.rs` | `inline` (`mod tests` block L3764-5238, 1225 LOC) | `inline` 1231 (code 2950) | 58 |
| `src/monomorph/helpers.rs` | `inline` (`mod tests` block L778-1533, 641 LOC) | `inline` 643 (code 530) | 26 |
| `src/monomorph/lower.rs` | `inline` (`mod tests` block L3166-4729, 1292 LOC) | `inline` 1292 (code 2521) | 45 |

The §4 part layouts were planned against the `17a9b5799` snapshot (`/tmp/loc/maps/G.md`).
Five commits have touched G files since:

- `267088179` and `cdc87b0a0` added `term_measure_text_with_attributed_string_routes_to_the_attr_companion`
  and `term_measure_text_internal_helpers_are_not_user_callable` to `lower_coverage_tests` in
  `src/ir/tests.rs` (283 → 285). `267088179` also added
  `term_measure_text_attributed_requires_astrings_import` to `shape.rs`.
- `44288aae1` (bug-945) touched `verify/tests.rs`: 492 → 493 tests, +30 LOC, all inside
  `match_patterns_and_constructors`. It removed `rejects_result_case_not_matchable` and
  `rejects_construct_result_implicit`, and added `accepts_user_union_member_named_ok`,
  `rejects_case_error_not_a_union_member` and `accepts_construct_user_record_named_ok`.
  Lines after old L2434 shift by +36.
- `7589ec9d5`, `7290a1fae` and `f6ff6e1fe` touched `lower.rs`. In its tests,
  `ok_and_error_constructor_types_are_inferred` became
  `user_record_named_ok_constructor_types_as_the_record` (same position, 45 tests).
- The block itself moved 2975 → 3166.

The diff was checked with
`diff <(git show 17a9b5799:<f> | grep -A3 '#\[test\]' | grep -oE 'fn [a-z0-9_]+' | sort) <(grep -A3 '#\[test\]' <f> | grep -oE 'fn [a-z0-9_]+' | sort)`.
The `[src a-b]` ranges in §4 are as of `17a9b5799`. Item and test NAMES are authoritative if
lines drift.

### Verified properties

- **The layout keeps module paths and `super` meanings.**
  - `src/ir/tests.rs` → `src/ir/tests/mod.rs`. The directory already exists (it holds
    `inline_trap_short_circuit.rs`), and `mod tests;` in `src/ir/mod.rs` resolves to either
    form.
  - The inline `pub(crate) mod helpers { use super::super::*; … }` stays inline in
    `src/ir/tests/mod.rs`, at the same path `crate::ir::tests::helpers`.
  - For a file module `src/x/foo.rs`, `#[cfg(test)] mod tests;` resolves to
    `src/x/foo/tests/mod.rs`, the plan-176-A convention.
- **A child sees its parent's private items.** Shared helpers stay in each `mod.rs`, so the
  part files reach them through `use super::*;` and no item changes visibility.
- **A `#[path]`-declared `…/mod.rs` resolves its children relative to its own directory.**
  Probe `/tmp/loc_e_probe`: `src/x/inner/mod.rs` declares
  `#[path = "../../../tests/common/emu/mod.rs"] mod emu;`, and its `mod scan;` resolved to the
  sibling `scan.rs` (`cargo test --lib` → `1 passed`). `variant_corpus_tests` relies on this.
- **The string guards exempt the new paths.** `no_type_strings.rs::is_test_file` and
  `no_operator_strings.rs::is_test_file` return true for `rel.contains("/tests/")`, and every G
  file lands under a `tests/` directory. `variant_corpus_tests.rs` is exempt today only
  through its `_tests.rs` suffix, which a directory split loses. Hence its `#[path]` into
  `src/ir/tests/variant_corpus/`.
- **The architecture guards need no change for G.**
  - `no_hand_picked_vreg` and `emission_lines` scan all of `src/`. They skip only files named
    `test_support.rs` or `*tests.rs`, and otherwise scan everything above the first
    `#[cfg(test)]`/`mod tests`. So G's moved files would be scanned. They contain none of the
    needles:
    `grep -cE '"%[vf][0-9]|Reg::physical\(|\.physical_field\(|Operand::phys\(|format!\("%[vf]'`
    over all 7 G files → 0 each.
  - `grep -lE 'set_hook|take_hook'` over the same 7 files → none, so the panic-hook guard is
    unaffected.
  - `is_physical_scan_exempt` scans only `src/codegen` and `src/target`, which G does not
    touch.
- **Spec citations.** The only citation into moved code is `[[src/ir/resource_escape.rs:tests]]`
  (`grep -rn 'resource_escape.rs:tests' src/docs/spec` →
  `architecture/23_escape-analysis.md:358`).
  - The other 19 `resource_escape.rs`, 21 `shape.rs`, 16 `helpers.rs` and 31 `lower.rs`
    citations name code symbols whose definitions lie outside the moved blocks. Each cited
    symbol's definition line was located against the block (as of `17a9b5799`).
  - `grep -rnoE "\[\[src/ir/(tests|variant_corpus_tests|verify/tests)\.rs[^]]*\]\]" src/docs/spec`
    → none.
  - A bare file citation is an accepted form (`[[src/trace.rs]]`,
    `[[src/codegen/engine/builder/mod.rs]]` in the spec today).
- **No path reference elsewhere.**
  - `grep -rn "ir/tests.rs\|variant_corpus_tests\|verify/tests.rs\|ir/tests::\|verify::tests\|resource_escape::tests\|shape::tests\|helpers::tests\|lower::tests" .ai AGENTS.md scripts src/docs tests/guards Cargo.toml .github`
    → none.
  - No G file uses `include_str!`, `#[path]`, `module_path!` or `--exact`.
  - None is a coverage exception: `grep -n 'src/ir\|monomorph' scripts/coverage-exceptions.txt`
    → only a comment naming `src/ir/verify/link.rs`.
  - `lowering_totality_tests` reads `env!("CARGO_MANIFEST_DIR").join("tests")` (L167), which
    is manifest-relative. That module stays inline in `src/ir/tests/mod.rs`.
- **Coverage denominator.** `scripts/coverage-common.sh` `IGNORE` drops `(^|/)tests/`.
  - `src/ir/variant_corpus_tests.rs` is in the denominator today, because no path component is
    `tests`. After the move it is ignored.
  - The four hosts lose their inline test lines from the denominator. Those lines are executed
    lines: the `--bins` run executes every `#[test]`.
  - The hosts' code lines, and which of them are covered, do not change: no code moves and no
    test is removed. G has no coverage gate (§Validation Plan, §Decisions).

## 3. Design Overview

Conventions, per file:

- An inline `mod tests { }` of ≤ 1000 LOC becomes ONE file, `foo/tests/mod.rs`. Its tests keep
  their module path.
- A test-only file or block over 1000 becomes `foo/tests/mod.rs` plus part files of ≤ 900
  each.
  - `mod.rs` keeps the original `use` block, the `mod` lines, and every helper shared by more
    than one part. No item changes visibility.
  - A helper used by exactly one part moves into that part.
- Parts are contiguous runs of the topic sections the planning packer found
  (`/tmp/loc_gh/render.py`, specs in `/tmp/loc_gh/specG.py`), merged by responsibility.
  - The source order is kept, so each part is one or more consecutive source ranges.
  - Merging sections that came out of one module cannot collide on a name. They shared one
    namespace before the move.
- Each part file begins `use super::*;` and holds its items verbatim, dedented one level when
  they came out of an inline module. `impl` blocks travel with their type.
- Part names avoid the host module's own child-module names. In `ir::verify` those are
  `calls`, `compat`, `link`, `matching`, `ops`, `resources`, `trap_init`, `types`, `values`.
- An inline host keeps `#[cfg(test)] mod tests;` in place of the inline block.

The gate class is test motion. The whole-binary `--list` count is unchanged, the moved modules'
tests pass, there are no new warnings, and the guards pass. Then the ceilings are tightened.
Byte-identity does not apply, because no compiled compiler code changes
(`cargo build --release` never compiles a `#[cfg(test)]` module).

Risk concentrates in two places. The first is `src/ir/verify/tests.rs`: 493 tests over 10
parts, and one widened import list. A missed `super::` path there fails only the test build,
so it lands last. The second is `variant_corpus_tests`'s `#[path]` into a directory that
`ir::tests` does not declare.

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
- *Shared helpers in `pub(super)` support files (`builders.rs`, `fixtures.rs`, …).* With a
  900-LOC part target, every `mod.rs` holds its shared helpers with room to spare (largest:
  `ir/tests/mod.rs` 480). A support file would add visibility edits and glob imports and buy
  nothing.
- *Breaking the five small inline modules of `src/ir/tests.rs` out into their own files.*
  Together with the root they are 480 LOC, and left inline their 39 tests keep their paths.

## 4. Detailed Design

LOC figures come from one of two sources. "measured" is a `span.py` count at HEAD
`ce7a995ca`. "sum of map parts" is the sum of the map's per-topic `span.py` counts at
`17a9b5799` (each topic counted with its own `use super::*;`), minus one `use super::*;` per
extra topic merged in, plus the drift measured since. The sums slightly over-estimate the
measured totals: `src/ir/tests.rs`'s `lower_tests` sums to 2999 against 2991 measured. `[src a-b]`
is the original line span as of `17a9b5799`. Every `#[test]` lands in exactly one file.

### 4.1 `src/ir/tests.rs` → `src/ir/tests/` (285 tests + 5 unchanged)

`src/ir/tests.rs` is a root of seven inline `mod … { }` blocks plus
`mod inline_trap_short_circuit;` (`src/ir/tests/inline_trap_short_circuit.rs`, 147 LOC,
5 tests, unchanged). The blocks were measured at HEAD with
`span.py src/ir/tests.rs <a> <b>`:

| Block | Lines | LOC | Tests |
|---|---|---|---|
| root (`use`s, `mod inline_trap_short_circuit;`) | 1-9 | 3 | – |
| `helpers` | 10-134 | 85 | 1 |
| `lowering_totality_tests` | 135-233 | 81 | 1 |
| `binary_repr_tests` | 234-876 | 541 | 20 |
| `lower_tests` | 877-4379 | 2994 | 143 |
| `lower_construct_tests` | 4380-4591 | 171 | 9 |
| `lower_pipeline_tests` | 4592-7348 | 2377 | 103 |
| `lower_coverage_tests` | 7349-7521 | 134 | 8 |

The file is deleted, and the following take its place:

- `src/ir/tests/mod.rs` (480, measured: 3 + 85 + 81 + 171 + 134 + 3 × 2; 19 tests in place).
  It is the original file with the three large inline blocks replaced by declarations
  (`#[cfg(test)] mod binary_repr_tests;`, `#[cfg(test)] mod lower_tests;`,
  `#[cfg(test)] mod lower_pipeline_tests;`). `helpers`, `lowering_totality_tests`,
  `lower_construct_tests` and `lower_coverage_tests` stay inline and verbatim, so their 19
  tests and `crate::ir::tests::helpers::lower_src` keep their paths.
- `src/ir/tests/binary_repr_tests.rs` (538, measured: 541 − the attribute, `mod … {` and `}`;
  20 tests) [src 242-869]. The block body, dedented, kept in one file, so the 20 tests keep
  their paths. In order:
  - its `use` block and shared helpers `corpus_project`, `fn_named`, `project_named`;
  - round trip and identity: `binary_repr_round_trip_is_identity`,
    `capture_index_round_trips_at_the_u32_boundary`, `binary_repr_rejects_bad_magic`,
    `binary_repr_rejects_bad_version`, `package_identity_prefix_is_applied_consistently`,
    `prefix_package_symbols_qualifies_link_tables`, `merge_package_dedups_link_cstructs`;
  - package init order: `package_bindings_initialize_dependencies_first`,
    `builtin_bindings_initialize_first`, `a_package_s_builtin_bindings_initialize_first`,
    `link_state_trailer_and_bind_in_round_trip`;
  - verify and JSON: `float_blocked_resource_owner_encodes_as_local`,
    `rejects_an_absurd_cstruct_field_count`, `verify_package_rejects_duplicate_function`,
    `verify_package_rejects_unnamed_function`, `verify_package_rejects_empty_match`,
    `json_covers_loop_state_and_capture_ops`, `json_binding_and_param_null_value_and_default`,
    `json_bind_with_null_value_emits_null`, `visibility_name_maps_all_variants`; helper
    `json_extra_ops_project`.
- `src/ir/tests/lower_tests/` (2991 measured, 143 tests; largest part 839):
  - `mod.rs` (45, sum of map parts: the map's 60 less 15 of its 19 topic `mod` lines). It
    holds the `use` block (`use super::super::*; use super::helpers::{function, lower_src, try_lower_src};`),
    4 part `mod` lines, and the shared helpers `main_body`, `binding`, `bind_value`, `describe`.
  - `declarations_statements_and_expressions.rs` (747 = 144 + 182 + 183 + 132 + 110 − 4,
    45 tests) [src 884-1750]:
    - declarations: `top_level_bindings_explicit_and_inferred`,
      `set_type_round_trips_through_front_end`, `inferred_binding_from_call_result`,
      `func_kinds_and_returns`, `func_with_default_param_and_isolated`,
      `func_returning_nothing`, `func_trap_handler_lowers_to_trap_op`,
      `type_record_with_private_field`, `union_with_variants_and_includes`,
      `enum_members_lowered`; local `ir_type`.
    - statements: `let_mut_and_assignment_statements`,
      `assignment_to_global_is_assign_global`, `return_without_value_from_sub`,
      `exit_targets_lower_to_expected_ops`, `exit_sub_lowers_to_valueless_return`,
      `exit_func_contributes_no_op`, `fail_statement_lowers_to_fail_op`,
      `if_else_lowers_to_if_op_with_both_branches`, `while_and_do_until_loops`,
      `for_loop_with_step`, `for_loop_default_step`, `for_loop_float_bounds_promote_loop_type`,
      `for_each_over_list`, `for_each_over_map_yields_map_entry`,
      `expression_statement_lowers_to_eval`.
    - match, literals and operators: `match_with_value_oneof_and_else`, `match_with_guard`,
      `match_on_union_binds_variant`, `match_on_fallible_call_uses_ok_error_and_is_ok_flag`,
      `literal_kinds_lower_to_typed_consts`,
      `most_negative_integer_literal_folds_into_the_const`, `binary_and_unary_operators`.
    - calls, constructors and literals: `user_function_call_lowers_to_call`,
      `call_with_named_and_default_arguments`, `error_builtin_lowers_to_error_record`,
      `builtin_call_return_type_resolves`, `constructor_and_member_access`,
      `constructor_named_arguments`, `with_update_expression`, `list_and_map_literals`,
      `empty_list_literal_uses_expected_element_type`.
    - lambdas: `lambda_capturing_local_becomes_closure`,
      `lambda_without_capture_is_function_ref`, `named_function_reference_as_argument`,
      `mut_capture_in_foreach_is_by_ref`.
  - `traps_unions_links_and_docs.rs` (839 = 125 + 137 + 198 + 157 + 226 − 4, 31 tests)
    [src 1754-2714]:
    - inline trap: `inline_trap_bind_with_recover`, `inline_trap_assign_target`,
      `inline_trap_discard_bare_statement`, `inline_trap_handler_with_if_then_fail`,
      `inline_trap_handler_with_match_and_continuation`,
      `inline_trap_handler_diverges_with_return`.
    - union wrapping: `returned_variant_wraps_into_union`, `union_typed_binding_wraps_variant`,
      `variant_of_every_expression_kind_wraps_at_a_union_argument`,
      `union_valued_argument_is_not_wrapped_again`; local `describe_all`, `show_arguments`.
    - native link: `link_functions_resources_and_aliases`,
      `link_const_pin_and_result_expression`, `link_const_sizeof_folds_to_layout_size`,
      `link_bind_in_field_value_forms`, `link_expr_binary_minus_and_unknown_fallback`,
      `native_resource_close_uses_exported_alias`.
    - doc blocks and state: `doc_blocks_collected_for_exports`,
      `doc_block_for_union_and_nothing_func`, `doc_for_nonexported_function_is_dropped`,
      `doc_resource_for_export_is_collected`, `doc_resource_for_nonexport_is_dropped`,
      `qualified_strings_call_lowers_to_internal_target`,
      `resource_state_param_assign_and_access`.
    - builtin calls and captures: `many_builtin_package_calls_resolve_return_types`,
      `strings_and_collections_calls_resolve`, `collections_filter_with_named_predicate`,
      `regex_call_padding_and_internal_name`, `nested_call_arguments_capture_and_lower`,
      `lambda_captures_across_expression_shapes`,
      `link_success_on_with_comparison_and_boolean_ops`,
      `link_const_keeps_the_exact_bits_of_a_64_bit_flag`.
  - `builtin_resolution_and_captures.rs` (785 = 66 + 180 + 142 + 197 + 204 − 4, 37 tests)
    [src 2718-3665]:
    - external and write-IR: `external_function_return_and_param_types_are_used`,
      `write_ir_emits_ir_file`, `inferred_binding_from_list_literal_types`. Its fn-local
      `use super::super::ExternalFunctionParam;` → `use super::super::super::ExternalFunctionParam;`.
    - builtin advisories and padding: `json_and_csv_calls_resolve`,
      `builtin_enum_advisory_warns_once_per_user_occurrence`,
      `the_sha1_advisory_fires_on_hash_and_not_on_hmac_hkdf_or_pbkdf2`,
      `the_sha1_advisory_suppression_is_fail_closed`,
      `builtin_enum_without_advisory_never_warns`, `crypto_call_resolves_and_pads_aad`,
      `net_and_http_calls_resolve_and_pad`.
    - builtin padding: `thread_call_resolves`, `datetime_time_pads_optional_arguments`,
      `regex_find_pads_start_argument`, `crypto_aead_pads_aad_as_empty_list`,
      `tls_connect_pads_optional_arguments`, `tls_close_on_listener_routes_to_listener_helper`,
      `vector_call_and_constant_lower`, `datetime_call_pads_default_arguments`.
    - inline trap treeify: `inline_trap_mut_bind_and_global_assign_targets`,
      `inline_trap_handler_with_terminator_then_dead_statement`,
      `inline_trap_handler_is_a_single_if`, `inline_trap_handler_is_a_single_match`,
      `inline_trap_handler_is_a_single_loop`, `inline_trap_handler_with_loop_and_continuation`,
      `inline_trap_handler_with_nested_if_and_continuation`,
      `inline_trap_handler_with_match_else_and_continuation`,
      `builtin_named_arguments_reorder_with_extra`, `local_call_named_arguments_reorder`.
    - capture variants: `lambda_captures_through_map_literal_and_with_update`,
      `lambda_captures_called_function_local_and_constructor_arg`,
      `assignment_bodied_lambda_captures_assignment_target`,
      `local_call_named_then_positional_skips_filled_slot`,
      `builtin_call_named_then_positional_skips_filled_slot`,
      `native_resource_visibility_variants`, `lambda_captures_through_unary_and_trapped`,
      `collections_filter_with_lambda_predicate`, `doc_for_unmatched_overload_is_dropped`.
  - `type_inference_and_named_arguments.rs` (583 = 189 + 156 + 129 + 112 − 3, 30 tests)
    [src 3669-4374]:
    - literal type fallback: `list_literal_without_expected_type_infers_from_first_literal`,
      `list_literal_without_expected_and_non_literal_first_element`,
      `builtin_type_field_access_and_net_lookup`,
      `collections_filter_with_user_function_predicate`,
      `list_literal_first_element_float_bool_and_nothing`, `nested_lambda_inside_lambda_body`,
      `lambda_captures_through_trapped_expression`,
      `filter_result_typed_binding_resolves_element_type`,
      `call_through_zero_arg_function_typed_local`, `doc_for_nonexported_type_is_dropped`,
      `inline_trap_binding_type_from_trapped_expression`.
    - named argument normalization: `overloaded_builtin_named_arguments_are_reordered_to_param_order`,
      `local_call_named_and_default_arguments_lower_in_param_order`,
      `a_computed_default_lowers_to_a_hidden_function_call`,
      `a_literal_default_lowers_as_a_constant`,
      `default_kind_is_literal_only_for_name_free_constants`. Its fn-local
      `use super::super::lower::{default_kind, DefaultKind};` →
      `use super::super::super::lower::{default_kind, DefaultKind};`.
    - trap handler arms: `trap_handler_global_assign_arm_without_error_read`,
      `trap_handler_exit_program_arm`, `trap_handler_continue_loop_arm`,
      `trap_handler_if_arm_recurses_both_branches`,
      `trap_handler_match_arm_with_guard_recurses`,
      `trap_handler_empty_loop_body_treeifies_to_nothing`.
    - literal type inference: `untyped_global_binding_infers_type`,
      `untyped_money_and_scalar_literal_types`, `expected_typed_numeric_literals_coerce`,
      `untyped_list_literal_element_types`, `collections_filter_bare_builtin_predicate_type`,
      `untyped_record_and_variant_constructor_and_field_types`,
      `member_access_builtin_type_field_infers`, `testing_block_item_is_skipped`.
- `src/ir/tests/lower_pipeline_tests/` (2374 measured, 103 tests; largest part 837):
  - `mod.rs` (20, sum of map parts: the map's 30 less 10 of its 13 topic `mod` lines). It holds
    its four `use` lines (including `use super::super::IrOp;`), 3 part `mod` lines, and the
    shared helpers `lower_src`, `try_lower_src`, `function`, `json_of`. `lower_src` spells
    `super::IrProject` and `super::helpers::lower_src_named` and stays in `mod.rs`, so its
    `super::` depth is unchanged.
  - `expressions_statements_and_lambdas.rs` (753 = 194 + 159 + 215 + 188 − 3, 32 tests)
    [src 4605-5492]:
    - expressions: `lowers_every_literal_and_operator_kind`, `lowers_nothing_literal`,
      `lowers_list_and_map_literals`, `lowers_constructors_member_access_and_with_update`,
      `lowers_enum_member_access`, `lowers_union_wrap_and_extract`,
      `lowers_calls_and_function_refs`, `lowers_default_argument_padding_for_local_call`.
    - statements: `lowers_control_flow_statements`, `lowers_float_for_loop_and_exit_do_while`,
      `lowers_foreach_over_list_and_map`, `lowers_exit_program_and_exit_sub`,
      `lowers_global_assignment_and_state_assign`, `lowers_fail_and_function_trap`,
      `lowers_propagate_in_trap`.
    - inline trap and match: `lowers_inline_trap_bind_assign_discard`,
      `lowers_inline_trap_treeify_if_and_match_continuation`,
      `lowers_inline_trap_treeify_match_with_tail_continuation`,
      `lowers_inline_trap_treeify_match_existing_else_with_tail`,
      `lowers_inline_trap_with_loop_and_foreach_in_handler`,
      `lowers_match_value_oneof_else_and_guard`.
    - lambdas and builtins: `lowers_lambda_by_value_capture_and_closure`,
      `lowers_lambda_mut_byref_capture_in_foreach`, `lowers_filter_predicate_function_ref`,
      `lowers_builtin_package_calls`, `lowers_named_argument_reordering_for_builtin`,
      `lowers_builtin_with_overloaded_argument_signature`,
      `lowers_builtin_with_generic_placeholder_argument_signature`,
      `lowers_named_constructor_arg_inside_lambda`,
      `lowers_builtin_mixed_positional_and_named_args`,
      `lowers_regex_and_datetime_default_padding`, `lowers_tostring_with_base_and_overridable`.
  - `links_docs_state_and_captures.rs` (770 = 138 + 204 + 218 + 213 − 3, 26 tests)
    [src 5496-6318]:
    - native link: `lowers_native_link_functions_resources_and_aliases`,
      `lowers_link_const_and_result_expression_forms`.
    - docs and resources: `collects_doc_blocks_for_exported_declarations`,
      `skips_docs_for_nonexported_declarations`, `skips_docs_for_nonexported_type`,
      `lowers_package_visibility_native_resource`, `collects_doc_for_union_and_enum`,
      `lowers_resource_binding_with_state`, `lowers_nested_function_call_as_match_scrutinee`,
      `function_helper_finds_main`.
    - externals, state and packages: `lowers_with_external_function_metadata`,
      `write_ir_serializes_to_disk`, `lowers_resource_state_binding_and_assign`,
      `lowers_error_member_access`, `lowers_json_csv_crypto_net_http_calls`,
      `lowers_io_and_thread_and_bits_and_math_more`, `lowers_vector_package_typed_dispatch`,
      `lowers_vector_record_constant`; local `PROJECT_JSON`, `temp_dir`.
    - capture and shape arms: `lowers_lambda_capturing_across_expression_kinds`,
      `lowers_lambda_capturing_list_map_with_and_unary`,
      `lowers_lambda_capturing_map_literal_and_constructor`,
      `lowers_inline_trap_treeify_match_with_else_and_no_tail`,
      `lowers_inline_trap_handler_if_both_branches_terminate`,
      `lowers_native_link_function_types_and_returns`,
      `lowers_constructor_positional_and_named_mixed`, `lowers_match_union_variant_binding`.
  - `type_inference_and_reachable_arms.rs` (837 = 209 + 177 + 122 + 184 + 149 − 4, 45 tests)
    [src 6322-7342]:
    - result type inference: `infers_builtin_result_types_in_inferred_lets`,
      `lowers_call_default_padding_for_regex_datetime_crypto`,
      `lowers_general_override_for_builtin_value_type`, `lowers_lambda_capturing_callable_local`,
      `infers_top_level_binding_types`, `lowers_union_with_includes`,
      `lowers_filter_with_inline_lambda_predicate`,
      `lowers_filter_with_builtin_predicate_reference`,
      `lowers_collections_filter_inferred_result_type`,
      `lowers_bare_function_reference_without_captures`.
    - inferred types and calls: `lowers_inferred_list_literal_element_type`,
      `lowers_error_constructor_type_inference`, `infers_mapentry_key_and_value_types`,
      `infers_arithmetic_and_trapped_expression_types`, `lowers_assignment_bodied_lambda`,
      `lowers_assignment_bodied_lambda_target_not_on_rhs`,
      `lowers_nested_lambda_capture_skips_inner_lambda`, `lowers_zero_param_func_typed_local_call`,
      `lowers_isolated_function_reference`.
    - IO/FS inference and trap fall-through: `lowers_local_call_with_named_and_positional_args`,
      `lowers_tls_connect_default_argument_padding`,
      `lowers_member_access_on_builtin_record_type`,
      `infers_io_and_fs_native_call_result_types`,
      `inline_trap_fallthrough_handler_lowers_linearly`; local `count_ops`.
    - reachable arms: `suffixed_float_literal_types_as_float`,
      `negated_exact_numeric_literals`, `untyped_list_literal_element_type_fixed_and_money`,
      `untyped_global_binding_holding_function_value`, `isolated_func_value_call_typing`,
      `foreach_over_map_entry_value`, `collections_filter_with_bare_general_predicate`,
      `assign_bodied_lambda_captures_write_only_target`, `self_including_union_cycle_terminates`,
      `testing_block_item_is_skipped_by_lowering`, `untyped_lambda_param_defaults_to_unknown`,
      `error_call_with_missing_argument_uses_placeholder`; local `binding_of`.
    - reachable arms, more: `lambda_calls_captured_function_local`,
      `untyped_let_inline_trap_infers_success_type`,
      `trap_handler_block_can_terminate_walks_all_nested_arms`,
      `untypeable_match_scrutinee_defaults_to_unknown`,
      `untypeable_loop_bounds_and_iterable_default_to_unknown`,
      `untypeable_negation_operand_defaults_to_unknown`,
      `builtin_named_first_then_positional_reorders`,
      `builtin_overloaded_named_arguments_select_overload`,
      `builtin_named_with_surplus_positionals_overflows_to_extras`.
- Count check: 1 + 1 + 9 + 8 (inline in `mod.rs`) + 20 + 143 + 103 = 285, plus the 5
  untouched. New files: 11 (`mod.rs`, `binary_repr_tests.rs`, 5 under `lower_tests/`,
  4 under `lower_pipeline_tests/`).

### 4.2 `src/ir/variant_corpus_tests.rs` → `src/ir/tests/variant_corpus/` (34 tests)

- `src/ir/mod.rs:56-57`: `#[cfg(test)] mod variant_corpus_tests;` →
  `#[cfg(test)] #[path = "tests/variant_corpus/mod.rs"] mod variant_corpus_tests;`. The module
  path stays `crate::ir::variant_corpus_tests`, so `super` is still `crate::ir`.
  `src/ir/tests/mod.rs` does NOT declare `variant_corpus`.
- `mod.rs` (536, sum of map parts: `use` block 10 + 2 part `mod` lines + the three builder
  groups 155 + 225 + 147 less their `use super::*;`). Holds the shared builders verbatim and in
  place, with no visibility change: `loc`, `c`, `empty_project`, `fn_body`, `every_value`,
  `every_op`, `link_function`, `variant_corpus`. Their bodies stay in the module whose `super`
  is `crate::ir`, so they need no edit. One import edit: `full_bytes` (→ the first part)
  spells `super::binary::BINARY_REPR_MAGIC`/`…VERSION`, so
  `use super::binary::{decode_binary_repr, encode_binary_repr, verify_package};` becomes
  `use super::binary::{self, decode_binary_repr, encode_binary_repr, verify_package};`.
- `binary_and_package.rs` (640 = 185 + 165 + 79 + 214 − 3, 23 tests) [src 586-1338]:
  - binary round trip: `binary_round_trip_over_full_surface`,
    `binary_round_trip_without_link_tables_is_a_bare_trailer`,
    `rejects_a_previous_format_version`, `rejects_an_absurd_cstruct_count`,
    `rejects_an_invalid_slot_direction`, `binary_round_trip_link_expr_variants`,
    `binary_round_trip_link_expr_arithmetic_is_structural`,
    `binary_round_trip_while_op_with_non_while_loop_kind`.
  - binary malformed input: `decode_rejects_bad_magic`, `decode_rejects_bad_version`,
    `decode_rejects_too_short_for_magic`, `decode_rejects_truncated_body`,
    `decode_rejects_invalid_utf8_string`, `decode_rejects_unknown_op_tag`,
    `decode_rejects_unknown_value_tag`, `decode_rejects_depth_limit`,
    `decode_surfaces_every_tag_error_branch`; local `decode_err`, `full_bytes`.
  - verify package: `verify_package_accepts_well_formed`,
    `verify_package_rejects_duplicate_type`,
    `verify_package_recurses_into_for_and_do_until_bodies`,
    `verify_package_rejects_deeply_nested_ops`.
  - package identity: `prefix_and_apply_identity_rewrite_all_reference_shapes`,
    `merge_package_dedups_by_name_and_carries_link_tables`.
- `projections_and_op_loc.rs` (576 = 103 + 215 + 149 + 112 − 3, 11 tests) [src 1342-1988]:
  - JSON projections: `json_covers_every_op_and_value`, `json_covers_all_type_kinds`,
    `json_project_with_no_bindings_omits_the_bindings_section`,
    `loop_kind_name_covers_all_kinds`, `visibility_name_covers_all_visibilities`. The fn-local
    `use crate::ast::Visibility;` (L1452) is absolute and needs no edit.
  - annotated type: `annotated_type_reports_every_annotated_variant`.
  - op loc: `op_loc_returns_the_stored_location_for_every_variant`,
    `binary_round_trip_carries_buffers_and_length`.
  - op loc buffers: `rejects_an_over_limit_buffer_count`,
    `rejects_a_crafted_unknown_slot_ctype`, `rejects_crafted_malformed_buffers_after_decode`.
- Count check: 23 + 11 = 34. New files: 3.

### 4.3 `src/ir/verify/tests.rs` → `src/ir/verify/tests/` (493 tests)

- `src/ir/verify/mod.rs` keeps `#[cfg(test)] mod tests;` (it now resolves to `tests/mod.rs`).
- `mod.rs` (445, sum of map parts: `use` block 8 + 10 part `mod` lines + the builders 223 +
  the fixtures 204). The shared helpers stay here verbatim, with no visibility change:
  - builders: `project`, `rules`, `accept`, `expect_rule`, `expect_no_rule`, `ret`,
    `ret_none`, `const_of`, `binary`, `unary`, `bind`, `record_typed`, `enum_type`, `binding`,
    `func`, `func_returns`, `param`, `record`, `int_const`, `union`, `union_variant_case`,
    `money_const`, `eval`;
  - fixtures: `res_bind_owned`, `transfer_call`, `link_fn`, `cstruct`,
    `project_with_cstructs`, `abi_slot`, `project_with_link`, `eval_call`, `close_eval`,
    `owner_fn`, `THREAD_VIEW`, `WORKER_VIEW`, `inline_trap_ops`, `recover_zero`,
    `unsendable_record`, `bind_in`, `read_of`, `if_op`, `count_rule`.
- One import edit. Part bodies spell four `super::` paths, and from a part `super` is now
  `verify::tests`:
  - `super::TypeEnv` (union wrap and annotations in part 1, and `type_env` in the
    unsendability cause walk in part 9);
  - `super::collect_source_diagnostics` (diagnostics, visibility and builtin args in part 6);
  - `super::is_thread_type` (cbuffer positions in part 9).

  So the first line `use super::{check, collect_diagnostics};` becomes
  `use super::{check, collect_diagnostics, collect_source_diagnostics, is_thread_type, TypeEnv};`,
  and the bodies resolve through that import unchanged.
- Parts, each `(LOC = sum of map parts, tests) [src range]`, then its sections with their tests and
  local helpers:
  1. `members_captures_unions_and_operators.rs` (798, 47) [223-1188]:
     - member and call arity: `accepts_member_access_on_known_record_field`, `rejects_member_access_on_integer`, `rejects_member_access_on_money_and_scalar`, `rejects_member_access_missing_field_on_record`, `skips_member_access_on_unknown_type`, `rejects_call_with_too_many_arguments`, `accepts_call_omitting_defaulted_argument`, `skips_arity_for_unknown_call_targets`, `rejects_constructor_with_extra_arguments`.
     - capture bounds: `rejects_capture_index_past_slot_count`, `accepts_capture_index_within_slot_count`, `rejects_capture_in_non_closure_body`, `ambiguous_closure_arity_does_not_disarm_the_capture_bounds_check`, `a_body_captured_with_two_arities_is_rejected`.
     - union wrap and annotations: `resource_union_parameter_widening_is_directional`, `an_isolated_callable_is_assignable_to_the_ordinary_one_only`, `rejects_union_wrap_of_foreign_variant`, `accepts_union_wrap_of_real_variant`, `rejects_result_value_with_fabricated_success_type`, `accepts_result_value_with_matching_success_type`, `rejects_union_wrap_with_mismatched_payload`, `rejects_with_update_with_fabricated_type`, `accepts_with_update_matching_target_type`, `rejects_empty_match`.
     - function returns and params: `accepts_ordinary_function`, `rejects_func_missing_return_type`, `rejects_func_missing_return_value`, `accepts_func_returning_on_all_paths_via_if`, `nothing_func_may_fall_through`, `rejects_param_missing_type`, `rejects_default_arg_order`, `rejects_default_value_mismatch`, `accepts_matching_default_value`.
     - operators: `rejects_arithmetic_on_string`, `rejects_and_on_numeric`, `rejects_concat_on_numeric`, `rejects_relational_on_boolean`, `accepts_string_relational`, `accepts_string_concat`, `accepts_boolean_and`, `rejects_equality_incompatible_types`, `rejects_equality_not_comparable`, `accepts_numeric_equality`, `rejects_not_on_numeric`, `rejects_negate_on_string`, `rejects_unknown_unary_operator`, `accepts_not_on_boolean_and_negate_numeric`.
  2. `literals_bindings_and_loops.rs` (829, 54) [1192-2117]:
     - literal ranges: `rejects_byte_overflow`, `rejects_byte_underflow`, `rejects_integer_overflow`, `rejects_negated_integer_overflow`, `rejects_float_overflow`, `rejects_float_underflow`, `rejects_fixed_overflow`, `rejects_fixed_underflow`.
     - scalar literals and bindings: `accepts_fixed_exact_maximum_and_minimum`, `rejects_fixed_literal_rounding_past_either_end`, `accepts_byte_in_range`, `rejects_scalar_out_of_range`, `rejects_scalar_surrogate`, `rejects_negated_scalar`, `accepts_scalar_in_range`, `rejects_let_requires_value`, `rejects_binding_requires_type_or_value`, `rejects_mut_requires_defaultable`, `accepts_mut_defaultable_without_value`, `rejects_binding_type_mismatch`, `rejects_assign_to_immutable`, `rejects_assignment_type_mismatch`, `accepts_valid_assignment`; local `fixed_bind`, `negated_fixed`.
     - globals, returns and exit: `rejects_global_binding_requires_type_or_value`, `rejects_global_let_requires_value`, `rejects_global_mut_requires_defaultable`, `accepts_global_with_value`, `rejects_global_binding_type_mismatch`, `rejects_assign_global_immutable`, `rejects_assign_global_type_mismatch`, `rejects_return_mismatch`, `accepts_a_nothing_call_in_statement_position`, `rejects_exit_program_non_integer`, `rejects_exit_program_out_of_range`, `accepts_exit_program_in_range`, `rejects_exit_program_i128_min_without_panic`, `rejects_propagate_outside_trap`, `rejects_fail_non_error`.
     - loop exits and conditions: `rejects_exit_without_loop`, `rejects_continue_without_loop`, `accepts_exit_inside_matching_loop`, `rejects_unreachable_after_exit`, `rejects_recover_literal_lowered_into_a_byte_slot_out_of_range`, `rejects_attributed_string_constructor`, `rejects_if_condition_non_boolean`, `rejects_while_condition_non_boolean`, `rejects_do_until_condition_non_boolean`, `accepts_do_until_valid`.
     - for loops: `rejects_for_non_numeric_bound`, `rejects_for_step_zero`, `accepts_valid_for`, `for_step_resolved_through_temp`, `rejects_for_each_non_collection`, `accepts_for_each_list`.
  3. `matches_with_updates_and_money.rs` (785 = 755 at `17a9b5799` + 30 from `44288aae1`, 54) [2121-2994]:
     - match exhaustiveness: `rejects_non_exhaustive_union_match`, `accepts_exhaustive_union_match`, `accepts_union_match_with_else`, `rejects_enum_match_not_exhaustive`, `rejects_match_open_type_without_else`, `rejects_match_pattern_not_a_variant`.
     - match patterns and constructors (196, 11 at HEAD): `accepts_user_union_member_named_ok`, `rejects_case_error_not_a_union_member`, `rejects_match_pattern_requires_union`, `match_guard_and_oneof`, `rejects_when_guard_non_boolean`, `rejects_constructor_requires_record_for_union`, `rejects_constructor_requires_record_for_enum`, `rejects_constructor_arity`, `rejects_constructor_argument_mismatch`, `accepts_valid_constructor`, `accepts_construct_user_record_named_ok`.
     - with update and literals: `rejects_read_only_record_update_error`, `rejects_duplicate_with_field`, `rejects_with_update_field_mismatch`, `accepts_valid_with_update`, `rejects_list_element_mismatch`, `accepts_valid_list_literal`, `rejects_map_key_mismatch`, `rejects_map_value_mismatch`, `accepts_valid_map_literal`, `rejects_map_key_not_comparable`.
     - value literals and members: `deeply_nested_value_hits_depth_cap`, `with_update_unknown_type_infers_read_only_target`, `rejects_with_update_read_only_mapentry`, `with_update_unknown_field_and_uninferable_value_are_skipped`, `rejects_set_literal_element_mismatch`, `accepts_valid_set_literal`, `rejects_money_literal_precision`, `rejects_money_literal_overflow`, `accepts_money_literal_in_range`, `rejects_negated_money_precision`, `rejects_negated_money_underflow`, `accepts_negated_money_literal_in_range`, `rejects_scalar_literal_overflowing_u64`, `accepts_finite_float_and_fixed_literals`, `accepts_enum_member_access`.
     - money and set elements: `member_access_uninferable_target_is_skipped`, `rejects_read_state_on_stateless_resource`, `rejects_money_compared_with_non_money`, `accepts_money_equality`, `rejects_money_plus_non_money`, `accepts_money_add_and_scale`, `rejects_money_times_money`, `rejects_non_money_divided_by_money`, `rejects_money_exponentiation`, `rejects_set_of_union_element_not_comparable`, `accepts_set_of_enum_element`, `accepts_set_of_unknown_type_element`.
  4. `declarations_and_resource_states.rs` (865, 59) [3000-4049]:
     - compat deep arms: `rejects_resource_return_order`, `float_owner_is_non_owning`, `os_call_with_valid_args_is_accepted`, `rejects_collections_find_wrong_arity`, `accepts_collections_contains_comparable`, `rejects_result_of_inner_mismatch`, `rejects_union_extract_foreign_variant`, `equality_with_unknown_operand_is_permissive`, `accepts_set_of_comparable_element`, `rejects_set_of_resource_element`, `rejects_set_of_function_element`, `rejects_set_of_collection_element`, `rejects_unknown_enum_member`, `accepts_enum_member`, `accepts_error_member_access_chain`.
     - type declarations and call args: `rejects_unmarked_resource_field_in_record`, `rejects_res_marker_on_a_non_resource_field`, `accepts_a_res_marked_resource_field_in_record`, `rejects_recursive_record`, `accepts_recursive_record_through_list`, `rejects_empty_enum`, `rejects_union_include_requires_union`, `rejects_union_member_requires_type`, `rejects_mixed_resource_union`, `rejects_duplicate_variant_via_include`, `rejects_local_variant_conflicts_with_include`, `rejects_call_too_few_args`, `rejects_call_argument_type`, `rejects_package_constant_not_callable`, `rejects_calling_non_function_local`, `rejects_builtin_math_bad_args`, `accepts_builtin_math_good_args`.
     - resource axis and use after move: `rejects_resource_without_res`, `rejects_res_on_non_resource`, `rejects_collection_resource_element_without_res`, `rejects_collection_res_on_data`, `accepts_collection_res_file`, `accepts_state_on_union`, `rejects_state_type_not_defaultable`, `rejects_param_state_type_not_defaultable`, `rejects_state_assign_no_state`, `rejects_state_assign_mismatch`, `rejects_use_after_close`, `accepts_close_of_a_res_parameter`.
     - resource aliases: `rejects_use_after_close_through_alias`, `alias_survives_branch_merge`, `rebind_severs_alias`, `foreach_body_move_leaks_to_outer`, `rejects_state_of_cyclic_record`, `trap_only_body_does_not_always_return`, `match_on_uninferable_scrutinee_not_exhaustive`, `match_on_primitive_scrutinee_not_exhaustive`; local `fs_close`, `grab`.
     - state agreement, calls: `unary_operand_of_uninferable_value_is_skipped`, `call_argument_of_uninferable_value_is_skipped`, `rejects_argument_state_retype`, `rejects_argument_state_missing`, `accepts_argument_state_agreement`, `rejects_thread_transfer_state_retype`, `rejects_thread_transfer_state_missing`.
  5. `state_bindings_and_native_links.rs` (740, 57) [4051-5007]:
     - state agreement, bindings: `rejects_thread_transfer_state_on_bare_plane`, `thread_transfer_with_one_arg_is_skipped`, `thread_transfer_uninferable_args_are_skipped`, `thread_transfer_non_thread_handle_is_skipped`, `accepts_return_state_union`, `rejects_return_state_not_defaultable`, `rejects_binding_opaque_state_narrowing`, `rejects_binding_state_mismatch`, `rejects_bare_binding_of_stateful_initializer`, `accepts_binding_state_agreement`, `binding_state_of_uninferable_initializer_is_skipped`, `accepts_valid_link_function`, `link_library_names_dedups_in_order`.
     - cstruct declarations: `accepts_valid_cstruct`, `rejects_cstruct_with_no_fields`, `rejects_cstruct_duplicate_field`, `rejects_cstruct_unknown_field_ctype`, `rejects_cstruct_cvoid_field`, `rejects_nested_cstruct`, `rejects_duplicate_cstruct_name_in_one_alias`, `rejects_oversized_cstruct`, `rejects_cstruct_escape_into_wrapper_signature`.
     - link native arms: `rejects_cstruct_maps_to_non_record`, `rejects_cstruct_escape_in_return_type`, `rejects_bind_in_unknown_slot`, `rejects_bind_in_slot_not_cstruct`, `rejects_bind_in_unknown_field`, `rejects_bind_in_field_both_param_and_literal`, `rejects_bind_in_field_unknown_param`, `rejects_bind_state_not_out_cstruct_slot`, `rejects_bind_state_without_stateful_return`.
     - link native bind state: `rejects_bind_state_maps_to_mismatch`, `rejects_bind_state_resource_wrong_slot`, `bind_state_resource_with_computed_result_is_skipped`, `rejects_bind_state_resource_against_abi_return`, `rejects_native_resource_state_disagreement`, `set_of_map_of_resource_is_ownership_violation`, `set_of_cyclic_record_with_resource_is_ownership_violation`.
     - link expression slots: `rejects_link_expr_naming_no_slot`, `accepts_link_expr_naming_the_abi_return`, `accepts_link_expr_naming_an_abi_slot`, `rejects_struct_slot_with_uncovered_record_field`, `rejects_struct_slot_with_mistyped_record_field`, `rejects_link_unknown_slot_ctype`, `rejects_link_unknown_return_ctype`, `rejects_link_cvoid_argument_slot`, `rejects_link_cptr_escape_in_param`, `rejects_link_cptr_escape_in_return`, `rejects_link_unbound_input_slot`.
     - link expression pins: `rejects_link_unbound_slot`, `rejects_link_out_slot_not_return`, `rejects_link_const_out`, `rejects_link_no_result`, `rejects_link_unbound_param`, `rejects_link_const_unknown_slot`, `rejects_link_invalid_free`, `accepts_link_const_pin`.
  6. `traps_diagnostics_and_capture_positions.rs` (829, 41) [5015-5984]:
     - trap and closures: `rejects_a_handler_read_of_a_local_declared_after_a_failure`, `accepts_a_handler_read_of_a_local_declared_before_any_failure`, `a_call_to_a_total_project_function_is_not_a_failure_edge`, `an_unproved_built_in_call_is_a_failure_edge`, `a_proven_total_built_in_call_is_not_a_failure_edge`, `a_failed_reassignment_keeps_the_mutable_local_readable`, `rejects_trap_fallthrough`, `accepts_trap_that_returns`, `recover_type_mismatch`, `accepts_closure_valid_capture_in_bind`, `accepts_deeply_nested_but_bounded`, `accepts_global_list_and_map_values`, `rejects_global_byte_overflow`; local `call_eval`, `trap_returning_local`.
     - diagnostics, visibility and builtin args: `collect_source_diagnostics_maps_rules_to_pending`, `collect_source_diagnostics_generated_path_when_file_empty`, `rejects_construct_private_type_cross_file`, `rejects_construct_hidden_field_cross_file`, `rejects_member_access_hidden_field`, `rejects_read_only_record_constructor`, `rejects_term_call_arity`, `rejects_term_call_argument`, `accepts_term_call_valid`, `rejects_collections_call_arity`, `rejects_collections_contains_not_comparable`, `rejects_general_call_arity`, `rejects_general_call_bad_argument`, `accepts_general_len_string`, `rejects_strings_call_bad_args`; local `private_record`.
     - exhaustive returns and moves: `func_returns_via_exhaustive_enum_match`, `func_missing_return_when_match_not_exhaustive`, `accepts_enum_arm_whose_member_names_a_declared_type`, `rejects_enum_arm_over_union_with_same_named_variant`, `rejects_link_return_on_a_nothing_wrapper`, `return_resource_move_is_not_use_after_move`, `rejects_double_move_close_then_return`, `accepts_temporary_in_resource_list`, `rejects_capture_out_of_range_in_bind_value`; local `enum_member_case`.
     - capture value positions: `rejects_capture_out_of_range_in_match_pattern_and_guard`, `rejects_stray_capture_in_parameter_default_and_global_initializer`, `accepts_global_read_in_function`, `captures_walked_through_nested_value_shapes`.
  7. `coercion_closure_walk_and_defaultability.rs` (820, 48) [5988-6986]:
     - coercion and include cycles: `accepts_byte_literal_into_byte_param`, `accepts_integer_literal_into_fixed_param`, `accepts_negated_literal_into_fixed_binding`, `accepts_union_variant_return`, `accepts_list_compatible_recursion`, `accepts_map_compatible_recursion`, `poisoned_initializer_yields_unknown_value`, `poisoned_return_yields_unknown_value`, `rejects_map_key_thread_ownership`, `rejects_map_key_record_with_resource`, `rejects_nested_list_mismatch_return`, `rejects_value_return_from_nothing_function`, `accepts_nothing_value_return_from_nothing_function`, `accepts_nested_map_return`, `accepts_result_of_return`, `union_include_cycle_is_bounded`, `accepts_union_with_included_union`, `record_include_cycle_terminates`.
     - unknown bounds and package overloads: `for_unknown_bound_is_skipped`, `foreach_unknown_iterable_is_skipped`, `rejects_bits_bad_args`, `rejects_encoding_bad_args`, `rejects_io_bad_args`, `rejects_fs_bad_args`, `rejects_net_bad_args`, `rejects_vector_bad_args`, `unknown_package_call_is_skipped`.
     - closure capture walk: `closure_body_captures_walked_over_all_shapes`, `accepts_function_with_union_and_result_value_shapes`, `accepts_localref_and_functionref_values`.
     - collection element borrow: `accepts_res_bind_of_a_collection_element`, `accepts_return_of_a_resource_collection_element`; local `get_call`.
     - defaultability: `mut_list_is_defaultable`, `mut_map_is_defaultable`, `mut_set_is_defaultable`, `rejects_mut_set_of_resource_ownership_and_comparable`, `mut_enum_is_defaultable`, `mut_record_with_an_enum_field_is_defaultable`, `rejects_mut_record_with_a_union_field_beside_an_enum_field`, `rejects_mut_func_not_defaultable`, `mut_single_variant_enum_is_defaultable`, `mut_record_of_defaultable_fields_ok`, `rejects_mut_record_with_nondefaultable_field`, `accepts_mut_list_of_union_defaultable_empty`, `accepts_mut_map_value_union_defaultable_empty`, `accepts_mut_record_with_list_of_union_field`, `rejects_state_collection_not_a_record`, `rejects_mut_unknown_record_not_defaultable`.
  8. `match_coverage_moves_and_annotations.rs` (832, 40) [6990-7993]:
     - match coverage: `func_returns_via_exhaustive_union_match`, `func_returns_via_match_else`, `func_returns_via_oneof_exhaustive`, `match_on_record_scrutinee_never_covers`, `union_oneof_partial_not_exhaustive`, `enum_missing_member_wording`, `a_qualified_type_name_and_a_bare_one_are_different_types`, `match_guard_reads_union_extract_bind`, `state_assign_on_data_local_skipped`.
     - nested moves and thread views: `move_in_if_branch_propagates_past_join`, `move_in_match_case_propagates`, `move_in_while_body_propagates`, `close_in_foreach_body_is_accepted`, `res_transfer_moves_source`, `a_thread_view_transfer_moves_the_source`, `a_let_of_a_worker_view_requires_res`, `a_worker_view_alias_moves_the_source`, `a_worker_view_is_a_resource_with_no_close_op`, `returning_a_thread_view_consumes_it`, `passing_a_thread_view_to_a_function_borrows_it`, `res_on_a_thread_binding_is_accepted`, `a_let_thread_binding_requires_res`; local `thread_op`.
     - early exits and res kinds: `rejects_thread_result_member`, `rejects_match_literal_pattern_type`, `rejects_collections_get_bad_args`, `builtin_arg_check_skipped_when_arg_type_unknown`, `binding_unknown_expected_skips_mismatch`, `union_wrap_empty_member_skipped`, `rejects_res_on_enum`, `rejects_res_on_record`, `rejects_res_on_data_union`, `capture_out_of_range_inside_union_extract`, `capture_out_of_range_inside_result_value_and_member`, `enum_member_access_returns_after_check`.
     - computed node annotations: `call_result_annotated_as_a_foreign_record_is_rejected`, `string_call_annotated_integer_cannot_feed_arithmetic`, `call_result_node_annotation_is_reconciled_too`, `a_truthful_call_annotation_is_accepted`, `member_access_annotated_against_its_field_type_is_rejected`, `operator_result_annotations_are_reconciled_with_their_operands`.
  9. `cbuffers_and_thread_messages.rs` (825, 59) [8019-9138]:
     - cbuffer positions: `accepts_well_formed_cbuffer_link_function`, `rejects_cbuffer_in_slot`, `rejects_cbuffer_inout_slot`, `rejects_cbuffer_without_buffer_clause`, `rejects_cbuffer_with_two_buffer_clauses`, `rejects_buffer_clause_naming_unknown_slot`, `rejects_buffer_clause_naming_non_cbuffer_slot`, `rejects_cbuffer_const_pin`, `rejects_cbuffer_as_abi_return`, `rejects_cbuffer_not_named_by_return`, `rejects_cbuffer_return_with_wrong_wrapper_type`, `rejects_byte_list_return_without_cbuffer_slot`, `rejects_buffer_size_naming_unknown_slot`, `rejects_buffer_size_reading_the_abi_return`, `rejects_buffer_size_reading_an_out_slot`, `accepts_buffer_size_reading_a_const_pin`, `rejects_returned_cbuffer_without_length`, `rejects_length_without_a_cbuffer_result`, `rejects_length_naming_unknown_slot`, `accepts_length_reading_an_out_slot`, `accepts_length_scaling_the_abi_return`, `truncated_thread_spelling_still_counts_as_a_thread`; local `cbuffer_fn`, `cbuffer_project`.
     - package path pilots: `accepts_private_isolated_func`, `rejects_isolated_on_a_non_func_kind`, `accepts_public_isolated_func`, `rejects_inline_trap_on_a_non_call`, `rejects_inline_trap_on_a_package_constant`, `accepts_inline_trap_on_a_call`, `skips_the_testing_desugared_trap_guard`, `rejects_unsendable_thread_message_in_a_parameter`, `rejects_unsendable_thread_message_in_a_record_field`.
     - package path thread messages: `rejects_unsendable_message_sent_across_a_thread`, `rejects_transfer_on_a_thread_without_a_resource_plane`, `rejects_unsendable_resource_plane_state_payload`, `rejects_a_non_resource_on_the_resource_plane`, `rejects_a_resource_in_the_message_plane`, `accepts_a_sendable_thread_message`.
     - unsendability cause walk: `cause_walk_reports_a_res_marked_element_as_a_resource`, `cause_walk_reports_a_res_map_value_through_both_positions`, `cause_walk_accepts_a_sendable_bare_resource_but_rejects_an_unsendable_one`, `cause_walk_reports_func_and_thread_handle_as_other_not_resource`, `thread_start_does_not_compare_parent_and_worker_state`, `a_thread_may_carry_parent_threads_on_its_resource_plane`, `a_thread_worker_on_a_resource_plane_is_not_transferable`, `cause_walk_descends_into_a_record_field_and_names_the_leaf`, `cause_walk_reports_the_first_blocking_field_left_to_right`, `cause_walk_accepts_every_plain_sendable_shape`, `the_resource_plane_keeps_the_generic_unsendable_rule`, `func_and_thread_handle_planes_keep_the_generic_unsendable_rule`, `cause_walk_terminates_on_a_self_referential_record`; local `type_env`, `cause_for`.
     - package path general: `warns_dead_handler_on_an_infallible_inline_builtin`, `rejects_normal_flow_reaching_the_trap`, `a_stray_recover_counts_as_diverging`, `rejects_a_thread_handle_as_a_list_element`, `rejects_a_thread_handle_as_a_map_value`, `accepts_res_thread_elements_values_and_fields`, `rejects_a_bare_thread_record_field`, `rejects_a_thread_worker_as_a_list_element`, `rejects_a_resource_in_a_set_literal`.
  10. `captures_struct_slots_and_state_types.rs` (747, 34) [9140-10080]:
     - package path captures: `rejects_a_thread_carrying_union_as_a_map_key`, `rejects_a_by_value_capture_of_a_mut_local`, `accepts_a_by_ref_capture_of_a_mut_local`, `accepts_a_resource_capture_in_either_shape`, `rejects_a_capture_of_a_worker_view`, `rejects_a_capture_of_a_record_reaching_a_worker_view`, `accepts_a_capture_of_a_parent_thread_view`, `accepts_a_copyable_immutable_capture`; local `closure_bind`.
     - package path link struct slots: `rejects_inout_on_a_non_cstruct_slot`, `rejects_returning_an_in_struct_slot`, `rejects_returning_a_struct_slot_as_another_type`, `rejects_bind_in_on_an_out_slot`, `rejects_bind_in_setting_a_field_twice`, `rejects_bind_in_field_bound_to_nothing`, `rejects_an_unbound_in_struct_slot`, `accepts_a_bound_in_struct_slot`, `accepts_a_body_that_returns_before_its_trap`, `read_only_records_are_refused_under_their_qualified_identity`, `a_close_on_one_arm_does_not_move_the_resource_on_its_sibling`; local `struct_slot_project`.
     - transfer success edge: `a_read_after_a_propagating_transfer_is_a_use_after_move`, `a_read_before_a_transfer_is_not_a_use_after_move`, `a_transfer_moves_a_possible_alias`, `an_inline_failure_arm_may_use_the_untransferred_resource`, `the_inline_success_arm_sees_the_resource_moved`, `a_read_after_an_inline_transfer_join_is_a_use_after_move`, `a_diverging_success_arm_does_not_poison_the_failure_continuation`, `a_check_chain_handler_may_use_the_untransferred_resource`, `a_read_after_a_check_chain_join_is_a_use_after_move`; local `RES_PLANE`, `transfer_value`, `result_is_ok`, `transfer_fn`, `inline_transfer`, `chain_transfer`.
     - state type is record: `rejects_state_type_that_is_not_a_record`, `accepts_state_type_that_is_a_record`, `rejects_non_record_state_at_every_position`; local `state_binding_rules`.
     - state type positions: `rejects_state_on_a_non_resource_at_every_position`, `res_on_a_non_resource_is_still_the_res_rule`, `imported_types_are_judged_by_their_kind`.
- Count check: 47 + 54 + 54 + 59 + 57 + 41 + 48 + 40 + 59 + 34 = 493. Largest part: 865
  (part 4). New files: 11.

### 4.4 `src/ir/resource_escape.rs` inline tests → `src/ir/resource_escape/tests/mod.rs` (28 tests)

- `resource_escape.rs`: the inline block (L856-1777, 759 LOC measured) is replaced by
  `#[cfg(test)] mod tests;`. The host keeps 17 inline test LOC (774 − 759 + 2): the
  `#[cfg(test)]` fns `owner`, `floats` and `analyze_function`, plus the 2-line declaration.
- `tests/mod.rs` (756 measured: 759 less the attribute, `mod tests {` and `}`). The block body
  is moved verbatim and dedented into one file. The 28 tests keep their paths
  (`ir::resource_escape::tests::…`), and no `use` changes: `use super::*;` still means the
  host. In order:
  - the `use` block (3) and shared helpers `func`, `res`, `list`, `open`, `append`, `ident`,
    `holder`, `construct`, `with_update`, `call`, `owner_after_inner_assign`, `reads`;
  - record edges [945-1176]: `inner_resource_floats_to_an_outer_record_via_with`,
    `same_scope_record_does_not_float`, `a_constructor_nested_in_a_list_routes_to_the_list`,
    `a_record_with_no_resource_argument_routes_nothing`,
    `a_positional_constructor_argument_routes_the_same_as_a_named_one`,
    `with_update_carries_the_targets_existing_contents`,
    `an_inferred_record_binding_is_still_seen_by_the_ordering_gate`;
  - return order and collections [1187-1393]: `a_res_parameter_is_never_blocked_by_return_order`,
    `a_returned_record_declared_after_its_resource_is_blocked`,
    `a_returned_record_without_a_res_field_is_not_blocked`,
    `a_returned_record_declared_before_its_resource_floats`,
    `same_scope_collection_does_not_float`, `inner_resource_floats_to_outer_collection`,
    `a_resource_floats_through_a_repackaging_member`;
  - collection floats [1399-1554]: `a_scalar_or_element_result_is_not_a_float_target`,
    `an_element_read_put_into_a_container_floats_its_source`,
    `resource_in_returned_collection_floats_to_it`, `bare_resource_return_does_not_float`,
    `a_borrowed_element_never_floats`, `float_follows_collection_copy_chain`;
  - closures and globals [1561-1776]: `a_resource_captured_into_an_outer_closure_floats_to_it`,
    `a_lambda_parameter_is_not_a_capture`, `a_returned_closure_over_a_later_record_is_blocked`,
    `a_returned_closure_binding_declared_after_its_resource_is_blocked`,
    `a_resource_stored_into_a_global_floats_to_the_global`,
    `a_global_outranks_an_outer_local_collection`,
    `a_parameter_stored_into_a_global_does_not_float_statically`,
    `an_assignment_to_an_unknown_name_is_not_a_global`; helpers `lambda`, `closure_binding`,
    `assign`, `globals`.
- Citation: `src/docs/spec/architecture/23_escape-analysis.md:358`
  `[[src/ir/resource_escape.rs:tests]]` → `[[src/ir/resource_escape/tests/mod.rs]]`. The
  citation closes the table of escape cases, and those cases are these tests, now one file. A
  bare file citation is an accepted form (§2). `tests/guards/no_type_strings.rs` mentions
  `decl_type`, which names code and is unaffected.

### 4.5 `src/ir/shape.rs` inline tests → `src/ir/shape/tests/` (58 tests)

The block is 1222 LOC once moved, which is over 1000, so it gets `mod.rs` plus two parts.

- `shape.rs`: the inline block (L3764-5238, 1225 LOC measured) is replaced by
  `#[cfg(test)] mod tests;`. The host keeps 8 inline test LOC (1231 − 1225 + 2): the
  `bound_types` probe's `#[cfg(test)]` lines (L187, L982, L3737) and the declaration.
- `tests/mod.rs` (36, sum of map parts: the map's 41 less 5 of its 7 topic `mod` lines): the
  `use` block (3), 2 part `mod` lines, and the shared `hir_from`, `shape_codes`, `rejects_with`,
  `accepts`.
- `walker_and_call_arguments.rs` (533 = 116 + 210 + 209 − 2, 33 tests) [src 3771-4431 as of
  `17a9b5799`, plus the 18-LOC test added by `267088179`]:
  - walker parity: `walker_types_bindings_exactly_as_lowering_does`,
    `check_project_accepts_clean_source`; local `lowered_binds`.
  - named arguments: `user_named_argument_valid`, `user_named_argument_unknown_name`,
    `user_named_argument_duplicate`, `user_named_positional_after_named_walk`,
    `user_private_function_in_another_file_is_not_a_call_target`,
    `imported_package_function_named_arguments`, `builtin_named_argument_valid`,
    `builtin_named_argument_unknown_name`,
    `builtin_named_argument_matching_parameter_name_accepted`,
    `builtin_named_argument_duplicate`, `builtin_named_then_positional_reorders`,
    `builtin_general_call_named_argument`, `overloaded_named_unknown_argument_rejected`,
    `overloaded_named_duplicate_argument_rejected`,
    `overloaded_duplicate_reported_once_and_before_unknown_names`,
    `overloaded_named_valid_selection_accepted`, `function_value_call_count_rejected`;
    local `wrap_import`.
  - argument types: `declared_function_argument_type_mismatch`,
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
- `packages_links_testing_and_fields.rs` (657 = 202 + 147 + 215 + 96 − 3, 25 tests)
  [src 4435-5216]:
  - package metadata: `corrupt_package_tables_are_package_invalid`,
    `package_type_validation_arms`,
    `a_dependency_s_builtin_types_resolve_without_the_importer_s_import`.
  - link, constructors and money: `const_pin_must_fold_to_an_immediate`,
    `free_deallocator_signature`, `error_records_cannot_be_constructed`,
    `constructor_named_field_set_twice`, `money_scaled_by_a_bare_decimal_literal_warns`;
    local `link_project`.
  - testing and control flow: `expect_arity_forms`,
    `expect_typed_operands_must_be_the_named_type`,
    `expect_equal_operands_must_be_comparable_and_printable`,
    `expect_trap_code_must_be_integer`, `expect_trap_requires_a_call_to_guard`,
    `exit_func_is_forbidden_and_ends_the_path`,
    `unreachable_after_exit_func_names_each_following_statement`,
    `loop_exit_tail_inside_a_handler_is_shapes`, `recover_outside_a_handler_and_the_count_forms`,
    `inline_trap_handler_must_diverge_on_every_path`,
    `nested_call_rule_follows_enclosing_call_rule`,
    `excess_positional_arguments_are_counted_in_the_arity_detail`; local `tcase`.
  - foreign record fields: `foreign_record_field_as_builtin_argument_is_rejected`,
    `foreign_record_field_as_user_func_argument_is_rejected`,
    `unrelated_import_does_not_make_a_foreign_record_field_readable`,
    `foreign_record_field_with_the_owning_import_is_accepted`,
    `foreign_record_field_in_a_binding_still_reports`; local `shape_reports`.
- Count check: 33 + 25 = 58. New files: 3.
- Hazards: 21 citations, all to code symbols outside the moved block (none move).
  - `tests/guards/no_type_strings.rs` allowlists `("src/ir/shape.rs", "resource_types")` and
    `("src/ir/shape.rs", "types")`, and the doc ref `src/ir/shape.rs:158 bound_types` names
    code. G touches none of them.
  - The `.ai` refs (`compiler.md`, `collections.md`, `codegen-invariants.md`,
    `resources-packages.md`) name code.

### 4.6 `src/monomorph/helpers.rs` inline tests → `src/monomorph/helpers/tests/mod.rs` (26 tests)

- `helpers.rs`: the block (L778-1533, 641 LOC measured) is replaced by `#[cfg(test)] mod tests;`.
  The file-top `#[cfg(test)] use crate::operators::BinaryOp;` stays in `helpers.rs`, and the
  tests reach it through `use super::*`. The host keeps 4 inline test LOC (643 − 641 + 2).
- `tests/mod.rs` (638 measured: 641 less the attribute, `mod tests {` and `}`). The block body
  is moved verbatim and dedented into one file. The 26 tests keep their paths
  (`monomorph::helpers::tests::…`). In order:
  - the `use` block (6) and shared `subs`, `param`, `func`;
  - unify [790-1050]: `unify_agrees_modulo_the_res_marker`,
    `unify_binds_and_checks_template_params`, `unknown_actual_never_poisons_a_param_binding`,
    `unify_recurses_into_all_container_shapes`, `unify_recurses_into_thread_shapes`,
    `unify_rejects_mismatched_container_shapes`,
    `unify_treats_unknown_actual_as_wildcard_and_matches_concretes`; helper `unify_str`;
  - type parts and mangling [1055-1313]: `func_type_parts_handles_isolated_and_empty_params`,
    `nested_function_type_arguments_are_not_shredded`,
    `substitution_walks_each_top_level_type_argument`,
    `user_template_parts_excludes_builtin_shapes`, `substitute_type_params_rewrites_every_shape`,
    `callable_matches_compares_the_complete_callable_type`,
    `reference_signature_defaults_a_missing_return_to_nothing`,
    `arg_slot_expected_for_calls_and_bare_names`, `mangle_and_sanitize_encode_types`,
    `overload_concrete_name_encodes_params_and_return`; helpers `substitute_str`,
    `func_parts`, `user_parts`, `PARSE`;
  - overload keys and accessors [1315-1537]: `overload_key_includes_return_type`,
    `param_types_eq_and_params_match`, `arg_slot_expected_only_for_call_arguments`,
    `constructor_arg_field_type_positional_and_named`, `arg_and_constructor_value_accessors`,
    `numeric_result_and_loop_promotion`, `split_helpers`,
    `collect_imported_overloads_empty_without_imports`,
    `collect_imported_overloads_reads_package_overload_set`.
- Hazards: 16 citations, all to code symbols. `.ai/codegen-invariants.md` and
  `no_type_strings.rs:989` name code (`unify_type`, `collect_imported_records`).

### 4.7 `src/monomorph/lower.rs` inline tests → `src/monomorph/lower/tests/` (45 tests)

The block is 1289 LOC once moved, which is over 1000, so it gets `mod.rs` plus two parts.

- `lower.rs`: the block (L3166-4729, 1292 LOC measured) is replaced by `#[cfg(test)] mod tests;`.
  The host keeps 2 inline test LOC.
- `tests/mod.rs` (53, sum of map parts: the map's 58 less 5 of its 7 topic `mod` lines). It holds:
  - the `use` block (3, keeping `use super::super::{ImportedOverload, Monomorphizer};`);
  - the added line `use super::{mangle_name, strip_qualifier_prefixes};`, because part bodies
    spell `super::mangle_name` (generic instantiation, part A) and
    `super::strip_qualifier_prefixes` (lambda and type compat, part B), and from a part
    `super` is `lower::tests`;
  - 2 part `mod` lines;
  - the shared `project`, `monomorphize`, `monomorphize_files`, `functions`, `types`,
    `function_names`.
- Bodies that spell `super::super::monomorphize_project` or
  `super::super::MAX_TOTAL_INSTANTIATIONS` (generic instantiation, imported overloads,
  instantiation placement) now resolve one level down, to `monomorph::lower::…`. They go
  through `lower.rs`'s `use super::*;` glob and reach the same items with no edit. Letter M's
  code split of `lower.rs` keeps `use super::*;` in the file that declares `mod tests`. G
  records this as M's input, but G itself depends only on today's `lower.rs`.
- `instantiation_overloads_and_inference.rs` (583 = 173 + 201 + 214 − 2, then −3 drift: the
  span of the range at HEAD is 633 against 636 at `17a9b5799`; 24 tests) [src 2982-3735]:
  - generic instantiation: `generic_function_instantiated_per_argument_type`,
    `generic_function_deduplicates_repeated_instantiation`,
    `total_instantiation_budget_halts_wide_fanout`,
    `instantiate_type_disambiguates_mangle_colliding_arguments`,
    `nested_generic_container_argument_unifies`,
    `generic_type_instantiated_from_expected_constructor_type`.
  - overload and control flow: `overload_selected_by_parameter_type`,
    `return_type_overload_selected_by_expected_type`, `control_flow_forms_are_lowered`,
    `for_loop_float_bound_promotes_counter_type`, `match_union_variant_binding_is_lowered`,
    `arity_mismatch_reports_error`, `top_level_binding_value_is_lowered`,
    `trap_body_is_lowered`, `plain_program_without_generics_passes_through`.
  - generic inference: `generic_over_map_and_result_container_params`,
    `generic_type_inferred_from_constructor_arguments`,
    `constructor_infers_param_despite_unknown_field_declared_first`,
    `record_member_access_and_with_update_types_are_inferred`,
    `list_literal_and_string_concat_and_unary_types`,
    `general_builtin_override_selected_for_user_type`,
    `overload_no_match_leaves_call_unresolved`,
    `return_type_overload_ambiguous_without_expected_type_errors`,
    `template_argument_unification_failure_errors`.
- `type_compat_imports_and_placement.rs` (657 = 177 + 216 + 201 + 53 − 3, then +13 drift: the
  span of the range at HEAD is 656 against 643; 21 tests) [src 3737-4525]:
  - lambda and type compat: `lambda_expression_type_is_inferred`,
    `types_compatible_matches_the_token_algorithm`,
    `normalize_type_matches_the_string_algorithm`.
  - imported overloads: `an_untyped_empty_collection_makes_an_imported_overload_ambiguous`,
    `imported_overload_call_is_rewritten_to_package_symbol`,
    `match_literal_oneof_and_else_patterns_lower`, `for_each_over_map_binds_map_entry_type`,
    `named_constructor_arguments_are_lowered`, `encoding_utf8_encode_overload_resolves_to_bytes`,
    `encoding_utf8_encode_overload_ambiguous_without_expected_type`,
    `encoding_utf8_encode_wrong_arg_type_leaves_call`,
    `bare_list_literal_argument_type_is_inferred`.
  - instantiation placement: `imported_overload_matches_argument_by_type`,
    `user_record_named_ok_constructor_types_as_the_record`,
    `two_generic_instantiations_are_emitted_sorted`,
    `return_type_only_template_param_is_a_reported_error`,
    `generic_set_literal_lowers_its_element_type`,
    `a_private_overload_is_one_set_with_its_files_public_sibling`,
    `a_user_instantiation_is_emitted_into_its_templates_file_in_either_order`,
    `a_toolchain_templates_instantiation_stays_in_the_first_file`; local `returned_callee`,
    `file_of`, `TEMPLATES`, `CALLER`.
  - import binding shadowing:
    `a_hosting_files_own_import_binding_wins_and_a_shadowed_package_stays_known`.
- Count check: 24 + 21 = 45. New files: 3.
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

Every phase runs the same gate, in this order.

- **G0 (once, before Phase 1's first edit):**
  - `cargo test -p mfb --bin mfb -- --list 2>/dev/null | grep -c ': test$' > /tmp/g_before.txt`
  - `cargo check --all-targets 2>&1 | grep -c '^warning' > /tmp/g_warn_before.txt`
    (est. 5 min cold).
- **G1:** the same two `--list`/warning commands after the phase's edits → numbers equal the G0 files.
- **G2:** run the moved modules' tests (filter per phase) → all pass, 0 failed. This must be an
  actual `test result: ok` line, not an inference (`.ai/testing-gates.md`).
- **G3:** `cargo test --test architecture_guards --test no_type_strings --test no_operator_strings`
  → all pass (est. 2 min).
- **G4:** the size check (est. 2 min).
  - `cargo test --test file_size_budget 2>&1 | grep -E '^  (NEW|GREW)'` → no output. Every
    `TIGHTEN`/`STALE` row names a file of this phase: a `ceiling test` row for a test file, or
    a `ceiling inline` row for a host.
  - Run `MFB_LOC_TIGHTEN=1 cargo test --test file_size_budget`, then rerun with
    `grep -cE '^  (NEW|GREW|STALE|TIGHTEN)'` → `0`.
  - `for f in <the phase's new files>; do python3 -I tools/loc-census/census.py --file $f; done`
    → every second field ≤ 900.
  - Commit `tests/guards/file_size_ceilings.txt` with the phase.
- **G5:** `cargo fmt --check` → no output (est. 20 s).

### Phase 1 — monomorph inline tests

- [x] Move `src/monomorph/helpers.rs`'s inline tests to `src/monomorph/helpers/tests/mod.rs`
      per §4.6 (one file).
      Done: `python3 -I target/p176/items.py extract` (body moved, dedented 4 only where the indent is code) → 26 tests;
      `motion.py --strip` → only `mod tests {`/`}` → `mod tests;` differ.
- [x] Move `src/monomorph/lower.rs`'s inline tests to `src/monomorph/lower/tests/` per §4.7
      (`mod.rs` + 2 parts; `use super::{mangle_name, strip_qualifier_prefixes};` in `tests/mod.rs`).
      Done: part A 24 tests, part B 21 (22 lines inside string literals kept byte for byte); motion → only the module
      lines, the added `use` (with a comment) and the 2 part `mod` lines differ.
- [x] G4: drop the two `ceiling inline` rows. The hosts' `code` row for `lower.rs` belongs to
      M and does not move.
      Done: → `TIGHTEN inline 4 (ceiling 643)` helpers, `TIGHTEN inline 2 (ceiling 1292)` lower; tighten drops
      exactly those two rows; re-run → 0.

Acceptance: the monomorph tests run from their new modules and the count did not move.
  Check: G1 → equal. G2:
  `cargo test -p mfb --bin mfb monomorph::helpers::tests:: monomorph::lower::tests::` → 0
  failed, and `cargo test -p mfb --bin mfb -- --list 2>/dev/null | grep -cE '^monomorph::(helpers|lower)::tests::'`
  → 26 + 45 = 71. `python3 -I tools/loc-census/census.py --file src/monomorph/lower.rs` →
  second field 2, and for `src/monomorph/helpers.rs` → 4. G3; G4; G5 (est. 8 min).
  Verified 2026-10-19: G1 (`bash target/p176/g1.sh`) → count 5017 = 5017, leaves SAME, warnings 0 (G0 0); G2 →
  `ok. 26 passed` and `ok. 45 passed`; list count → 71; census → lower.rs `2521 2`, helpers.rs `530 4`, new files
  639, 53, 583, 653; G3 via `bash scripts/test-fast.sh --test architecture_guards --test no_type_strings --test no_operator_strings`
  → `test-fast: ok` (unit tests `4998 passed; 2 ignored`; guards 8, 7+1 ignored, 8 passed); G4 → 0; G5 → exit 0.
Commit: 587df80c7

### Phase 2 — resource_escape and shape inline tests

- [x] Move `src/ir/resource_escape.rs`'s inline tests to `src/ir/resource_escape/tests/mod.rs`
      per §4.4 (one file).
      Done: `items.py extract` → 28 tests; motion → only the module lines differ.
- [x] `src/docs/spec/architecture/23_escape-analysis.md:358`: `[[src/ir/resource_escape.rs:tests]]`
      → `[[src/ir/resource_escape/tests/mod.rs]]`.
- [x] Move `src/ir/shape.rs`'s inline tests to `src/ir/shape/tests/` per §4.5 (`mod.rs` + 2 parts).
      Done: parts 33 + 25 tests (82 string-literal lines kept byte for byte); motion → only the module lines and the
      2 part `mod` lines differ; no `use` edit was needed.
- [x] G4: drop the two `ceiling inline` rows. `shape.rs`'s `code` row belongs to M.
      Done: → `TIGHTEN inline 17 (ceiling 774)` resource_escape, `TIGHTEN inline 8 (ceiling 1231)` shape; tighten drops
      exactly those; re-run → 0.

Acceptance: as Phase 1, and the spec still resolves.
  Check:
  - G1.
  - G2: `cargo test -p mfb --bin mfb ir::resource_escape::tests:: ir::shape::tests::` → 0
    failed. The `--list` count of `^ir::(resource_escape|shape)::tests::` → 28 + 58 = 86.
  - `census.py --file` on `src/ir/resource_escape.rs` → second field 17, and on
    `src/ir/shape.rs` → 8.
  - `cargo test -p mfb --bin mfb spec_citations_resolve` → passes.
  - `./target/debug/mfb spec architecture escape-analysis 2>/dev/null | grep -c '\[\['` → 0
    (est. 4 min).
  - G3; G4; G5.
  Verified 2026-10-19: G1 → 5017 = 5017, leaves SAME, warnings 0; G2 → `ok. 28 passed`, `ok. 58 passed`;
  census → resource_escape.rs `495 17`, shape.rs `2950 8`, new files 756, 36, 532, 655; spec_citations_resolve →
  `ok. 1 passed`; the rendered topic (debug `mfb` rebuilt by the fast tier) → `[[` count 0; G3 fast tier →
  `test-fast: ok` (unit `4998 passed; 2 ignored`; guards 8, 8, 7+1 ignored); G4 → 0; G5 → exit 0.
Commit: 2733eae7d

### Phase 3 — variant corpus

- [x] Create `src/ir/tests/variant_corpus/` per §4.2 (`mod.rs` + 2 parts) and delete
      `src/ir/variant_corpus_tests.rs`.
      Done: `items.py extract` (whole file) → parts 23 + 11 tests; the file is deleted; motion → only the widened
      `use super::binary::{self, …}` line and the 2 part `mod` lines differ.
- [x] `src/ir/mod.rs`: `#[cfg(test)] #[path = "tests/variant_corpus/mod.rs"] mod variant_corpus_tests;`.
- [x] G4: tighten (the `ceiling test … src/ir/variant_corpus_tests.rs` row is STALE).
      Done: → `STALE test (ceiling 1748) src/ir/variant_corpus_tests.rs` only; tighten drops it; re-run → 0.

Acceptance: the 34 corpus tests run under the unchanged module path.
  Check: G1. G2: `cargo test -p mfb --bin mfb ir::variant_corpus_tests::` → 0 failed, and the
  `--list` count of `^ir::variant_corpus_tests::` → 34. G3 → passes: the files are under
  `/tests/`, so both string guards still exempt them. G4; G5 (est. 6 min).
  Verified 2026-10-19: G1 → 5017 = 5017, leaves SAME, warnings 0; G2 → `ok. 34 passed`; list count → 34; census →
  536, 640, 576; G3 fast tier → `test-fast: ok` (unit `4998 passed; 2 ignored`; architecture_guards 8,
  no_operator_strings 8, no_type_strings 7 + 1 ignored); G4 → 0; G5 → exit 0.
Commit: 4c1601ae3

### Phase 4 — `src/ir/tests.rs`

- [x] Replace `src/ir/tests.rs` with `src/ir/tests/mod.rs` (four inline modules kept),
      `binary_repr_tests.rs`, `lower_tests/` (`mod.rs` + 4 parts) and `lower_pipeline_tests/`
      (`mod.rs` + 3 parts) per §4.1. `inline_trap_short_circuit.rs` is untouched.
      Done: `git mv src/ir/tests.rs src/ir/tests/mod.rs`, then three `items.py extract` runs bottom-up
      (`lower_pipeline_tests` 32 + 26 + 45, `lower_tests` 45 + 31 + 37 + 30, `binary_repr_tests` 20; 1052 lines
      inside string literals kept byte for byte); `motion.py --strip` over the 11 files → only the 3 `mod … {`/`}` →
      `mod …;` swaps and the 7 part `mod` lines differ.
      Added (see Corrections): `use super::{write_ir, EntryPoint, ExternalFunctionParam, ExternalSignature, IrBinding,
      IrProject};` in `lower_pipeline_tests/mod.rs`, with a comment.
- [x] ~~The two fn-local `use` edits (one more `super::`) for external and write-IR
      (`builtin_resolution_and_captures.rs`) and named argument normalization
      (`type_inference_and_named_arguments.rs`).~~ — moot: both `use super::super::…` paths still resolve from the
      part through `ir::tests`'s glob import (`cargo check -p mfb --bin mfb --tests` → no error at either), so the
      bodies are not edited.
- [x] G4: tighten (the `ceiling test … src/ir/tests.rs` row is STALE).
      Done: → `STALE test (ceiling 6386) src/ir/tests.rs` only; tighten drops it; re-run → 0.

Acceptance: the IR lowering tests run from their new modules, and the external helper path
still resolves.
  Check:
  - G1.
  - G2: `cargo test -p mfb --bin mfb ir::tests::` → 0 failed. The `--list` count of
    `^ir::tests::` → 290 (285 + 5).
  - The `--list` count of
    `^ir::tests::(helpers|lowering_totality_tests|lower_construct_tests|lower_coverage_tests|binary_repr_tests)::[a-z0-9_]+: test$`
    → 39. Those tests keep their paths.
  - `cargo test -p mfb --bin mfb target::shared::config_fold` → 0 failed. It is the caller of
    `crate::ir::tests::helpers::lower_src`.
  - G3; G4; G5 (est. 8 min).
  Verified 2026-10-19: G1 → 5017 = 5017, leaves SAME, warnings 0; G2 `cargo test -p mfb --bin mfb ir::tests::` →
  `ok. 320 passed` (the filter is a substring, so it also runs other modules' `ir::tests::` paths); list count of
  `^ir::tests::` → 290; kept-path count → 39 (G0 list → 39); `target::shared::config_fold` → `ok. 6 passed`;
  census → mod.rs 480, binary_repr_tests 538, lower_tests 45/744/839/784/583, lower_pipeline_tests 21/753/763/837;
  G3 fast tier → `test-fast: ok` (unit `4998 passed; 2 ignored`; guards 8, 8, 7 + 1 ignored); G4 → 0; G5 → exit 0.
Commit: 7ea6d182b

### Phase 5 — `src/ir/verify/tests.rs` (largest blast radius)

The 493 verifier tests across 10 parts carry the one widened import list.

- [x] Replace `src/ir/verify/tests.rs` with `src/ir/verify/tests/mod.rs` (the builders and
      fixtures in place) and the 10 parts of §4.3. Widen the `use super::{…}` line.
      Done: `git mv` to `tests/mod.rs`, then `items.py extract` in place with the parts chosen by name
      (`target/p176/g5-verify.py`: each part runs from its first test to the next part's, the 42 shared builders and
      fixtures stay in `mod.rs`) → 47, 54, 54, 59, 57, 41, 48, 40, 59, 34 tests and exactly §4.3's local helpers;
      motion → only the import line and the 10 part `mod` lines differ. The import is widened with two more names
      than §4.3 lists (Corrections).
- [x] G4: tighten (the `ceiling test … src/ir/verify/tests.rs` row is STALE).
      Done: → `STALE test (ceiling 8495) src/ir/verify/tests.rs` only; tighten drops it; re-run → 0.

Acceptance: every verifier test runs from its part, and the count did not move.
  Check: G1. G2: `cargo test -p mfb --bin mfb ir::verify::tests::` → 0 failed, and the
  `--list` count of `^ir::verify::tests::` → 493. `ls src/ir/verify/tests/*.rs | wc -l` → 11.
  G3; G4; G5 (est. 8 min).
  Verified 2026-10-19: G1 → 5017 = 5017, leaves SAME, warnings 0; G2 → `ok. 493 passed`; list count → 493;
  `ls src/ir/verify/tests/*.rs | wc -l` → 11; census → mod.rs 448 (445 + the 3-line import), parts 798, 829, 785,
  865, 740, 829, 820, 832, 825, 747; G3 fast tier → `test-fast: ok` (unit `4998 passed; 2 ignored`; guards 8, 8,
  7 + 1 ignored); G4 → 0; G5 → exit 0.
  Final gate: `spec_citations_resolve` runs inside the fast tier's unit tests (passed); the guards and the budget
  banner → 0 rows; the ceiling-file grep → 0; clippy — no G0 count was recorded, so the check is per path:
  `cargo clippy --all-targets` warnings under G's new directories → 3, all pre-existing in the moved text (two
  `empty line after doc comment` at blank lines that sat between doc blocks in the originals, `resource_escape.rs`
  L1131 and `verify/tests.rs` L8042; one auto-deref in an unchanged builder).
Commit: —

## Validation Plan

- Tests: no test is added or removed. Per phase, the whole-binary `--list` count equals G0's
  (G1). The moved modules' tests pass (G2), with the per-module counts given in each phase:
  71, 86, 34, 290, 493.
- Coverage check: per-file coverage is not a plan-176 gate (author, 2026-10-17). The evidence:
  - At 7a4b09970, `sh scripts/coverage.sh --bins` ran in a detached worktree (59 min; 4989
    unit tests passed). Then `sh scripts/coverage-check.sh` exited 1.
  - 241 `src/` files are already below the 98% per-file floor ("Files below 98% line coverage
    (GATE FAILURE)"), and overall coverage is 93.67%. Examples among plan-176 hosts:
    `src/ir/shape.rs` 93.21%, `src/codegen/engine/control/builder_control.rs` 75.55%,
    `src/debugger/values.rs` 86.66%.
  - A pure move changes no executed line. Moving an inline test block only removes test
    lines from its host's denominator.
  - The pre-existing gate failure is reported to the owner separately and is not a plan-176
    task.
- Runtime proof: two checks show that the module paths callers depend on are intact.
  - Phase 4's `config_fold` run exercises the external user of `ir::tests::helpers`.
  - Phase 3's `ir::variant_corpus_tests::` path is unchanged.
- Doc sync: `src/docs/spec/architecture/23_escape-analysis.md` (one citation). No `.ai` doc,
  `mfb man` topic or other spec topic names moved code (§2).
- Final gate (once, after Phase 5):
  - `cargo test -p mfb --bin mfb spec_citations_resolve` and
    `cargo test --test architecture_guards --test no_type_strings --test no_operator_strings --test file_size_budget`
    → all pass, and the banner has 0 rows.
  - `grep -cE 'ceiling (test [0-9]+ src/ir/(tests|variant_corpus_tests|verify/tests)\.rs|inline [0-9]+ src/(ir/(resource_escape|shape)|monomorph/(helpers|lower))\.rs)|src/(ir/(tests|verify/tests|shape/tests|resource_escape/tests)|monomorph/(helpers|lower)/tests)/' tests/guards/file_size_ceilings.txt`
    → 0.
  - `cargo clippy --all-targets 2>&1 | grep -c '^warning'` → equal to its value at G0
    (est. 8 min).
  - The full workspace suite runs once, in plan-176-V.

## Decisions

- Rule revised — owner, 2026-10-18. It replaces the 2026-10-14 "code ≤ 1000, test ≤ 250" rule.
  All three kinds are counted as non-comment, non-blank LOC:

  | Kind | Applies to | Limit | Over the limit → |
  |---|---|---|---|
  | `code` | code LOC of a file that is not a test-only file | 1000 | split the file into modules |
  | `inline` | inline test LOC (`#[cfg(test)]` items / MFBASIC `TESTING … END TESTING`) inside a file that is not a test-only file | 250 | move the tests out into their own test-only file(s) |
  | `test` | total LOC of a test-only file (path component `tests`, `test_*.mfb`, `#![cfg(test)]`, or a `#[cfg(test)] mod x;` target and its subtree) | 1000 | split the test-only file into modules |

  Scope: `.rs`/`.mfb` under `src/`, `packages/`, `examples/`, `repository/` and `tests/`.
  Parts target ≤ 900. Ceiling rows are `ceiling <code|inline|test> <n> <path>`.
- Generated files (`GENERATED` banner) are the only exemption. The guard warns with a banner
  and never fails. A new module is always an acceptable split — owner, 2026-10-14/2026-10-18.
- A moved inline block becomes ONE test-only file `foo/tests/mod.rs` unless it is over 1000.
  Then it is split into parts of ≤ 900 each — owner, 2026-10-18. Here `resource_escape` and
  `helpers` take one file each; `shape` and `lower` take `mod.rs` + 2 parts.
- G has no unit over its limit: the largest new file is 865 (`ir::verify::tests` part 4) —
  measured, §4.
- Parts merge the planning packer's topic sections, contiguous and in source order. Shared
  helpers stay in `mod.rs` with no visibility change — author, 2026-10-18. `mod.rs` is far
  under 900 in every G module (largest: 536, `variant_corpus`).
- The five small inline modules of `src/ir/tests.rs` stay inline in `src/ir/tests/mod.rs`, and
  `binary_repr_tests` becomes one file, so 39 tests keep their paths — author, 2026-10-18.
- Extracted src test modules go into `foo/tests/mod.rs` (+ parts), never `foo/tests.rs` —
  plan-176-A. Coverage `IGNORE` and the string guards recognise the `tests/` directory.
- `variant_corpus_tests` is loaded by `#[path = "tests/variant_corpus/mod.rs"]` so that its
  module path and `super` stay unchanged while it moves under `tests/` — author.
- Guards that classify test code by file name are updated in the same commit as the move that
  needs them — author, 2026-10-14. G needs none: the string guards already exempt `/tests/`,
  and the architecture guards find no needle in any moved line (measured, §2). Plan-176-H adds
  the `/tests/` rule to `architecture_guards`.
- Per-file coverage is not a plan-176 gate. The measured evidence is under "Coverage check" in
  §Validation Plan — author, 2026-10-17.

## Corrections

- 2026-10-18 — Rule revised by the owner: test-only files limit 1000 (was 250), inline tests
  250, no exemptions. Dropped 0 files: all 7 are still over a limit.
  - Structural: new files 132 → 33 (43 → 11 under `src/ir/tests/`, 12 → 3 `variant_corpus`,
    52 → 11 `verify/tests`, 5 → 1 `resource_escape`, 8 → 3 `shape`, 4 → 1 `helpers`,
    8 → 3 `lower`).
  - The support files (`builders.rs`, `fixtures.rs`, `value_builders.rs`, `op_builders.rs`,
    `corpus.rs`) are gone, along with their 49 `pub(super)` edits and glob imports. Their
    helpers stay in `mod.rs`.
  - `src/ir/tests.rs`'s four small inline modules stay inline, and `binary_repr_tests` is
    one file.
  - Counts were re-taken at `ce7a995ca`: `ir::tests` 283 → 285 and `verify::tests`
    492 → 493 (commits in §2). The phase acceptance counts are 290 and 493, and the total
    is 974.
- 2026-10-19 (execution) — `/tmp` is wiped by session restarts here, so G0's output lives in
  `target/p176/g0/` (ignored by the repository) and records the full sorted `--list` names, not
  only the count; G1 compares the count and the leaf names (a moved test gains a part segment).
  Per the owner (2026-10-18) G2/G3 run through `bash scripts/test-fast.sh`, and (2026-10-19) every
  check runs on macOS only.
- 2026-10-19 (execution) — Phase 4: `lower_pipeline_tests`' part bodies spell `super::IrProject`,
  `super::IrBinding`, `super::write_ir`, `super::EntryPoint`, `super::ExternalSignature` and
  `super::ExternalFunctionParam` (6 compile errors after the move; §4.1 only foresaw `lower_src`).
  As in §4.7's `lower.rs` line, `lower_pipeline_tests/mod.rs` imports those six names from
  `ir::tests`, so the bodies stay byte for byte. The two fn-local `use` edits §4.1 planned in
  `lower_tests` were not needed (moot task).
- 2026-10-19 (execution) — Phase 5's widened import also needs `read_only_record_type` (4 uses in
  part 10) and the `resources` module (8 uses in part 9's cause walk), which §4.3 missed:
  `use super::{check, collect_diagnostics, collect_source_diagnostics, is_thread_type,
  read_only_record_type, resources, TypeEnv};`. The part headers are comment-only (no LOC).

## Summary

G is pure motion of IR and monomorph test code into `tests/` directory modules. It covers 7
hosts and 974 tests (290 + 34 + 493 + 28 + 58 + 26 + 45, counting the 5 untouched), with
0 renamed, 33 new files, and no file over 900. The rule is code ≤ 1000, inline ≤ 250 and
test-only file ≤ 1000. 98 tests keep their exact module path: 39 in `ir::tests`, the 5
untouched, 28 in `resource_escape` and 26 in `helpers`. The rest gain one part segment.

The risk is the compile-only failure mode of a test split: a missed `super::` level. It is
concentrated in `ir::verify::tests`'s widened import. That phase is last, and an actual test
run of the module gates it. Host code, host paths and every code citation stay where they
are. Letters M and N split the code next.
