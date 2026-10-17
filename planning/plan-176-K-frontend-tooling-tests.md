# plan-176-K: Frontend and tooling test code

Last updated: 2026-10-14
Effort: large (3h–1d)
Depends on: plan-176-J

K moves the over-budget test code in the rest of `src/` — the AST, lexer, formatter, numeric
and type helpers, resolver, rules, audit, CLI, debugger, manifest, optimizer, doc renderer and
the shared `src/testutil.rs` harness — out of its host files into `tests/` directory modules,
split by topic so that no new file is over 250 test LOC. 36 files are in scope: 5 whole test
files and 31 code files whose inline `#[cfg(test)] mod tests` is over 250. No test is added,
removed or renamed; only the module path of each moved test changes (`ast::tests::foo` →
`ast::tests::source_roots::foo`). The host code files keep their paths. Their code-side
splits are letters L and M.

The single outcome: after K lands, `cargo test --test file_size_budget` shows no row for any
of K's 36 files and `tests/guards/file_size_ceilings.txt` has no `ceiling test` row for any of
them. K adds no `exempt` row.

References:

- `planning/plan-176-A-file-size-guard.md` — the rule, §Decisions, and "Splitting an
  over-budget file" in `.ai/build-tooling.md` (src tests: `foo/tests/mod.rs` + topic files).
- `.ai/testing-gates.md` § "Splits must sweep man AND spec citations".
- `.ai/build-tooling.md` — rustfmt/clippy policy and the split conventions.
- `.ai/codegen-invariants.md` — names `src/ast/scope_privates.rs`, `src/lexer.rs`,
  `src/numeric.rs`, `src/types.rs` (all host paths K keeps).
- `.ai/resources-packages.md` — names `src/resolver/mod.rs` (host path K keeps).
- `.ai/man-content.md` — names `src/cli/man.rs` (host path K keeps).
- Spec topics touched: `src/docs/spec/diagnostics/01_rule-codes.md` (prose names the test
  path `rules::tests::rule_codes_are_unique`). No `[[…]]` citation points into a moved range
  (measured per file in §4).
- Guards: `tests/guards/architecture_guards.rs` (+ `architecture_guards/seams_goldens_and_panic_hook.rs`
  after plan-176-E), `tests/guards/no_type_strings.rs`, `tests/guards/no_operator_strings.rs`.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-176-J is complete and archived | `ls planning/plan-176-J-* planning/completed/plan-176-J-*` → only `planning/completed/…` | NOT MET (2026-10-14) |
| The file-size guard exists and passes | `cargo test --test file_size_budget` → passes | NOT MET (2026-10-14) |
| The architecture guards treat a `tests` directory component as test code (landed by plan-176-H) | `grep -rn 'contains("/tests/")' tests/guards/architecture_guards.rs tests/guards/architecture_guards/` → a hit in each of `is_physical_scan_exempt`, `no_hand_picked_vreg` and `emission_lines` | NOT MET (2026-10-14) |
| K's files have no uncommitted changes | `git status --porcelain -- src/ast src/cli src/optimizer src/testutil.rs src/audit src/debugger src/doc/html.rs src/fmt.rs src/lexer.rs src/manifest src/numeric.rs src/resolver src/rules src/types.rs tests/guards` → empty | MET (2026-10-14) |

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

- Every file in §2's table is ≤ 250 test LOC, and every new file K creates is ≤ 250 test LOC
  (`python3 -I tools/loc-census/census.py --file <path>` → second field ≤ 250).
- `cargo test --test file_size_budget` prints no row naming a K path, and the ceiling file has
  no `ceiling test` row for any K path.
- The unit-test binary lists the same number of tests before and after each phase.

### Non-goals (explicit constraints)

- No behavior change. No test body edit beyond moving it, adding `use super::*;` at the top of
  a topic file, and the visibility edits listed in §4 (`pub(super)` on 4 items).
- No `#[test]` fn renamed, added or removed. No `[[test]]` target renamed (K touches no target).
- No host code moves. `src/lexer.rs`, `src/cli/pkg.rs`, `src/debugger/prompt.rs` etc. keep
  their code and their paths; letters L and M split the code.
- `crate::testutil::X` keeps resolving for every caller unchanged.
- No guard weakens: no production file becomes exempt from any guard.

## 2. Current State

All counts: `python3 -I /tmp/loc/span.py <file>` on the working tree of `17a9b5799` (prints
`(code, test)`; for whole test files the census `python3 -I /tmp/loc/census.py "$PWD"` row,
because `span.py` cannot see the `mod` declaration that makes the file test code). After
plan-176-A the same numbers come from `python3 -I tools/loc-census/census.py --file <path>`.
Test counts: `grep -cE '#\[(tokio::)?test\]' <file>`. The moved region is measured with
`python3 -I /tmp/loc/span.py <file> <a> <b>`; "host test after" = test − moved + the 2
declaration lines `#[cfg(test)]` / `mod tests;`.

| File | Kind | Code | Test | Tests | Moved region (as of 17a9b5799) | Host test after |
|---|---|---|---|---|---|---|
| `src/ast/tests.rs` | test file | — | 2374 | 166 | whole file | file removed |
| `src/cli/dbg/tests.rs` | test file | — | 359 | 13 | whole file | file removed |
| `src/cli/info/tests.rs` | test file | — | 1059 | 23 | whole file | file removed |
| `src/optimizer/opt1/placefwd/tests/mod.rs` | test file | — | 916 | 41 | whole file | 184 |
| `src/testutil.rs` | test file | — | 827 | 3 | whole file | file removed |
| `src/ast/scope_privates.rs` | inline | 517 | 390 | 20 | 629-1112 (390) | 2 |
| `src/audit/collect/findings.rs` | inline | 253 | 294 | 10 | 281-616 (294) | 2 |
| `src/audit/collect/project.rs` | inline | 136 | 342 | 10 | 166-541 (342) | 2 |
| `src/audit/collect/source.rs` | inline | 820 | 669 | 26 | 1088-1852 (664) | 7 |
| `src/audit/report.rs` | inline | 183 | 301 | 4 | 255-565 (301) | 4 |
| `src/cli/build/mod.rs` | inline | 843 | 2755 | 142 | 1338-4673 (2742) | 15 |
| `src/cli/man.rs` | inline | 817 | 971 | 49 | 1102-2363 (971) | 2 |
| `src/cli/pkg.rs` | inline | 2019 | 899 | 60 | 2515-3624 (896) | 5 |
| `src/cli/resolve.rs` | inline | 1016 | 600 | 27 | 1351-2090 (600) | 2 |
| `src/debugger/breakpoints.rs` | inline | 1271 | 945 | 20 | 1552-2584 (945) | 2 |
| `src/debugger/breakpoints/report.rs` | inline | 204 | 431 | 12 | 286-760 (431) | 2 |
| `src/debugger/prompt.rs` | inline | 2041 | 1922 | 63 | 2332-4346 (1922) | 2 |
| `src/debugger/runtime.rs` | inline | 448 | 407 | 14 | 569-1020 (407) | 2 |
| `src/debugger/values.rs` | inline | 1068 | 501 | 13 | 1208-1735 (501) | 2 |
| `src/doc/html.rs` | inline | 269 | 387 | 16 | 295-732 (387) | 2 |
| `src/fmt.rs` | inline | 672 | 575 | 53 | 918-1636 (575) | 2 |
| `src/lexer.rs` | inline | 1199 | 555 | 35 | 1454-2111 (555) | 2 |
| `src/manifest/config.rs` | inline | 554 | 289 | 19 | 673-994 (289) | 2 |
| `src/manifest/libraries.rs` | inline | 316 | 380 | 19 | 482-914 (380) | 2 |
| `src/manifest/mod.rs` | inline | 1092 | 1002 | 73 | 1461-2578 (945) | 59 |
| `src/manifest/package.rs` | inline | 712 | 1177 | 82 | 1013-2494 (1177) | 2 |
| `src/numeric.rs` | inline | 413 | 835 | 33 | 716-1667 (793) | 44 |
| `src/optimizer/opt1/globals.rs` | inline | 275 | 332 | 12 | 371-753 (332) | 2 |
| `src/optimizer/opt1/inline.rs` | inline | 254 | 270 | 7 | 351-654 (270) | 2 |
| `src/optimizer/opt2/promote.rs` | inline | 330 | 294 | 10 | 495-834 (286) | 10 |
| `src/optimizer/opt2/sink.rs` | inline | 372 | 300 | 12 | 518-850 (300) | 2 |
| `src/resolver/mod.rs` | inline | 557 | 631 | 28 | 806-1503 (622) | 11 |
| `src/resolver/packages.rs` | inline | 221 | 280 | 20 | 290-635 (280) | 2 |
| `src/resolver/resolution.rs` | inline | 1514 | 737 | 57 | 1776-2639 (737) | 2 |
| `src/rules/mod.rs` | inline | 272 | 283 | 20 | 448-792 (253) | 32 |
| `src/types.rs` | inline | 830 | 1011 | 27 | 1558-2804 (992) | 21 |

Total tests in the 36 files: 1239 (sum of the per-file `grep -cE` counts above).

`/tmp/loc/groups.json` lists three of these at an older census (`src/cli/build/mod.rs` 2774,
`src/optimizer/opt2/promote.rs` 362, `src/resolver/mod.rs` 647). The table uses the current
`/tmp/loc/loclib.py` extent rule, which is the rule plan-176-A ships.

### Verified properties

- **Every K `#[test]` is listed in exactly one topic** (the split-map generator checked each
  file's topic lists against its `grep -cE` count). The whole-binary `--list` count is the
  before == after check per phase.
- **Glob cycles compile clean.** `use super::*;` in a part plus `use part::*;` in `mod.rs`:
  probe `/tmp/loc/probe2` printed `6` with no warning.
- **Child modules see a parent's private items.** Rust privacy is descendant-visible, so a
  helper left in `tests/mod.rs` is callable from every topic with no visibility edit. This is
  why only 4 items in K change visibility.
- **`src/cli/build/mod.rs`, `src/manifest/mod.rs`, `src/resolver/mod.rs` and `src/rules/mod.rs`
  are already `mod.rs` files** with no `tests` child today, so their tests go to the sibling
  `tests/` directory. Every other host `foo.rs` gets `foo/tests/` (`mod tests;` in `foo.rs`
  resolves to `foo/tests/mod.rs`). Collisions checked: `ls src/cli/pkg` → `command_tests.rs`
  only; `ls src/debugger/breakpoints` → `report.rs` only; no other host has a same-named
  directory today.
- **The register guards scan moved test code unless the path has a `tests` component.**
  `tests/guards/architecture_guards.rs` `no_hand_picked_vreg` and `emission_lines` skip a file
  only by NAME (`name.ends_with("tests.rs")`) and otherwise scan text above the first
  `#[cfg(test)]`. A topic file has no such marker. K's promote and sink tests carry `"%vN"`
  literals and `.physical_field(` calls
  (`grep -nE '"%[vf][0-9]|\.physical_field\(' src/optimizer/opt2/promote.rs src/optimizer/opt2/sink.rs`
  → 30 lines, all in the moved ranges). plan-176-H adds the `/tests/` component rule to all
  three name checks (H map, "architecture_guards exempts by FILE NAME"); K's Prerequisites
  require it.
- **The panic-hook guard keys on a file name.** `only_testutil_installs_a_panic_hook` skips
  the file named `PANIC_HOOK_OWNER = "testutil.rs"`. The only `set_hook`/`take_hook` calls in
  `src/` are `src/testutil.rs:50-51` (`grep -rn "set_hook\|take_hook" src | grep -v '//'`).
  After the split they sit in `src/testutil/panics.rs`, which the name check does not match.
- **`no_type_strings` / `no_operator_strings` exempt `testutil.rs` by name.** `is_test_file`
  has `name == "testutil.rs"`; `rel` there is repo-relative (`rel == "src/lexer.rs"` in
  `is_excluded_from_scan`). The parts `src/testutil/*.rs` match no clause of `is_test_file`
  (no `/tests/` component, no `tests` prefix).
- **No fixture path is relative to the moved file.** Every fixture read in a moved range joins
  `env!("CARGO_MANIFEST_DIR")` (absolute): `src/cli/build/mod.rs:2241`,
  `src/audit/collect/findings.rs:488`, `src/resolver/packages.rs:482,520,556`,
  `src/rules/mod.rs:560`, `src/types.rs:1695`, `src/testutil.rs` `fixture_dir`.
  `src/cli/dbg/tests.rs` `existing()` uses `std::env::current_exe()` only as a path that
  exists, not to respawn.
- **Stale line reference in a moved range.** `scripts/test-accept.sh:260` cites
  `src/cli/build/mod.rs:1488` for "the flagless build IS `-O1`". Line 1488 is inside
  `parse_test_options_debug_sets_flag` (`sed -n 1480,1490p src/cli/build/mod.rs`). The claim's
  source is `impl Default for OptLevel` → `OptLevel(1)` at `src/optimizer/mod.rs:71`
  (`grep -n "impl Default for OptLevel" -A3 src/optimizer/mod.rs`).
- **Live documents that name a full test path.**
  `grep -rnoE "(ast|cli|…|types)::tests::[a-z_:]+" scripts .ai AGENTS.md src/docs .github tests/guards bugs planning/*.md`
  → live hits: `src/docs/spec/diagnostics/01_rule-codes.md:103` and
  `bugs/bug-883-rule-codes-spec-drift-and-unemitted-rules.md:43`
  (`rules::tests::rule_codes_are_unique`). Every other hit is in `bugs/completed/` (history,
  not edited), in `planning/todo.md` (pasted test-run output, history), or
  `planning/plan-174-C-dbg-riscv64-descriptor.md:280` (`debugger::prompt::tests::on_riscv64`,
  a test that plan adds; the prefix stays a valid filter for a test placed in
  `prompt/tests/mod.rs`).
- **`src/lexer.rs` line citations are not in K's range.** `[[src/lexer.rs:262]]` and
  `[[src/lexer.rs:752]]` (`src/docs/spec/architecture/19_internal-naming.md:17,50`) point at
  code lines above the test module (1454). K's move does not shift them. They are stale
  (line 262 is `self.advance();`, line 752 is `value.push(self.peek());`); plan-176-M fixes
  them when it moves `src/lexer.rs` to `src/lexer/`.

## 3. Design Overview

One convention, applied 36 times: the inline `#[cfg(test)] mod tests { … }` (or the whole
test file) becomes `#[cfg(test)] mod tests;` → `<host dir>/tests/mod.rs`, which holds the
shared `use` lines, the shared helpers and fixtures, and one `mod <topic>;` per topic file.
Each topic file starts with `use super::*;`. A helper used by one topic only moves with that
topic. A group of topics that share private fixtures becomes a nested directory module
(`tests/<group>/mod.rs` + topic files), one extra level, no `#[path]`.

Gate class: test letter. Per phase, the whole-binary test count before == after, the moved
modules' tests run, `cargo check --all-targets` adds no warning, the three guards pass, and the
file-size guard shows only TIGHTEN/STALE rows for the phase's files before the tighten.

Risk concentrates in two places:
- **`src/testutil.rs`** — every unit test in the binary calls into it (`grep -rhoE
  "testutil::(\{[^}]*\}|\w+)" src` → 29 distinct names, `CodeTarget` alone 50 uses). It is the
  last phase. Its split needs 3 visibility edits and 3 guard edits.
- **Guards keyed on file names** — the panic-hook owner and two `is_test_file` checks. Each
  edit is narrower than the code it replaces (§4.36).

Rejected alternatives:
- *`#[path = "…"]` attributes to keep `foo.rs` + `foo_tests.rs` side by side* — `tests/`
  directories are what the coverage `IGNORE` (`(^|/)tests/`) and the guards' `is_test_file`
  already recognise; a sibling file would be counted by coverage and scanned by the guards.
- *Moving test code into integration tests under `tests/`* — the tests reach private items
  (`super::*`); an integration crate cannot.
- *One `tests.rs` per host* — over 250 for 33 of the 36 hosts; the rule needs topic files.
- *Re-exporting testutil parts by explicit name lists* — 29 names, and a new helper would need
  two edits; `pub use part::*;` keeps one.

## 4. Detailed Design

Line ranges are source lines as of `17a9b5799`; item and test NAMES are authoritative if lines
drift. A topic's LOC includes its `use super::*;` line. "Shared" helpers stay in `mod.rs`.
Every topic starts with `use super::*;`. Section-separator comments between groups move with
the group that follows them. Visibility edits: none unless stated.

### 4.1 `src/ast/tests.rs` → `src/ast/tests/` (166 tests)

`src/ast/mod.rs` keeps `#[cfg(test)] mod tests;`. Spec citations to `src/ast/tests.rs`: 0
(`grep -rnoE "\[\[src/ast/tests.rs:[^]]+\]\]" src/docs/spec`); other refs: none.
- `mod.rs` (115): the 5 `use` lines (including `#[cfg(unix)] use std::os::unix::fs::symlink;`),
  12 `mod` lines; shared: `parse_ok`, `parse_err`, `ret`, `first_return_expr`, `expr_of`,
  `type_err`, `manifest_with_sources`, `source_entry`, `test_temp_dir`, `parse_file`,
  `try_parse`, `project_json`, `function`, `link_fn`.
- `source_roots.rs` (190; 9-17, 85-209, 1050-1122, 1180-1195) — include/exclude, globs,
  overlap, missing/empty roots, symlink containment. Tests (10):
  glob_patterns_match_nested_and_root_files, file_root_ignores_include_patterns,
  directory_root_applies_include_and_exclude_patterns, overlapping_source_entries_are_rejected,
  symlinked_source_paths_must_stay_inside_project, source_root_missing_is_rejected,
  empty_source_root_is_rejected, source_entry_without_root_is_ignored,
  file_root_ignores_non_mfb_extension, glob_star_and_question_components_match.
- `literals_and_operators.rs` (200; 19-83, 254-409). Tests (15): parse_import_aliases,
  string_concat_has_lower_precedence_than_addition, parses_primary_literal_forms,
  parses_list_literal_including_empty, parses_map_literal, parses_empty_map_literal,
  parses_map_literal_with_res_value, map_literal_requires_to_keyword,
  map_literal_requires_colon_equal_between_key_and_value, parses_operator_precedence_chain,
  parses_every_comparison_operator, parses_arithmetic_and_multiplicative_operators,
  parses_power_right_associative_and_unary_minus, parses_pipeline_with_placeholder,
  pipeline_without_placeholder_is_rejected.
- `calls_and_lambdas.rs` (203; 411-581, 852-926). Tests (29):
  parses_call_with_positional_and_named_args, parses_empty_call,
  call_on_non_identifier_is_rejected, call_missing_closing_paren_is_rejected,
  parses_constructor_positional_and_named, parses_empty_constructor,
  constructor_on_non_identifier_is_rejected, constructor_missing_closing_bracket_is_rejected,
  parses_member_access_chain, member_access_requires_identifier, parses_with_update,
  parses_with_update_empty, with_update_requires_open_brace,
  with_update_field_must_be_identifier, with_update_requires_colon_equal,
  with_update_requires_close_brace, parses_lambda_expression,
  parses_lambda_with_assignment_body, parses_lambda_with_no_params, lambda_requires_open_paren,
  lambda_requires_close_paren, lambda_requires_arrow, bare_expression_error_is_rejected,
  map_literal_with_res_value_type, map_literal_missing_to_is_error,
  div_operator_and_lambda_bare_body, with_update_error_paths,
  call_and_constructor_argument_errors, list_and_map_literal_forms.
- `types.rs` (209; 585-596, 603-850). Tests (21): parses_simple_and_generic_types,
  parses_map_type_variants, parses_res_collection_element_state_clause,
  state_clause_requires_res_collection_element, map_type_requires_to_keyword,
  parses_set_type_variants, set_type_rejects_res_element,
  parses_template_type_with_multiple_args, parses_thread_type_variants,
  thread_type_requires_to_keyword, parses_function_type_names,
  function_type_requires_open_paren, function_type_requires_close_paren,
  function_type_requires_as, isolated_type_requires_func, grouped_type_requires_close_paren,
  type_base_name_rejects_non_identifier, qualified_names_must_have_two_parts,
  parses_qualified_identifier, type_names_cover_thread_map_list_and_function_forms,
  type_name_errors. Helpers: type_of, param_type.
- `project_parsing.rs` (186; 934-1048, 1124-1178, 1197-1243). Tests (12):
  parse_project_reads_files_and_appends_prelude, parse_project_rejects_missing_project_dir,
  selected_source_paths_are_sorted_actual_paths,
  selected_source_paths_rejects_missing_project_dir, write_ast_writes_ast_json_file,
  write_ast_reports_write_failure, parse_source_internal_marks_file_internal,
  directory_cycle_via_symlink_is_tolerated, nested_symlink_escape_is_rejected,
  parse_project_propagates_collection_failure,
  selected_source_paths_propagates_collection_failure,
  broken_symlink_entry_reports_read_failure.
- `source_collection.rs` (228; 1245-1316, 1360-1566). Tests (15):
  parse_source_reports_lex_failure, parse_project_reports_unreadable_source_file,
  unreadable_directory_reports_read_failure, parse_project_propagates_parse_errors,
  parse_project_missing_directory_is_error, collect_reports_missing_and_empty_roots,
  collect_handles_single_file_and_non_mfb_root, source_entries_default_include_when_absent,
  collect_walks_nested_directories, selected_source_paths_missing_dir_is_error,
  nested_symlink_escaping_project_is_rejected, glob_component_edge_cases,
  broken_symlink_root_reports_canonicalize_failure,
  unreadable_source_file_reports_read_failure, symlink_cycle_within_project_is_skipped.
- `serialize.rs` (219; 1605-1830). Tests (2): serialize_covers_all_item_kinds,
  serialize_covers_all_statement_and_expression_kinds.
- `signatures_and_placeholders.rs` (173; 1832-2039). Tests (10):
  function_signature_line_renders_visibility_isolated_and_params,
  type_decl_signature_line_covers_all_kinds_and_visibility,
  pipeline_placeholder_substitutes_across_all_expression_forms,
  pipeline_placeholder_missing_is_rejected,
  pipeline_placeholder_in_lambda_and_map_and_boolean,
  serialize_covers_none_and_default_branches,
  serialize_exit_targets_func_and_do_and_while, serialize_lambda_without_assign_target,
  pipeline_placeholder_walks_named_args_and_trapped_and_with,
  all_comparison_and_arithmetic_operators_serialize.
- `declaration_errors.rs` (137; 2046-2255). Tests (12): top_level_errors_are_reported,
  function_header_and_body_errors, function_declaration_requires_the_func_keyword,
  trap_header_errors, type_decl_errors, statement_errors, expression_errors,
  doc_header_kind_keywords, doc_prose_kind_roundtrips_all_variants,
  qualified_member_may_be_keyword_or_numeric, qualified_member_missing_after_colon_is_error,
  qualified_numeric_member_edge_cases.
- `doc_and_link_blocks.rs` (145; 2263-2452). Tests (10):
  doc_block_prose_kinds_and_continuation, doc_block_header_variants_and_errors,
  doc_block_named_line_errors_and_unknown, doc_func_header_signature_forms,
  link_block_errors, native_func_missing_symbol_or_abi,
  native_func_error_on_and_return_and_const, native_func_free_block_and_errors,
  resource_declaration_errors, func_alias_errors.
- `abi_and_statement_errors.rs` (192; 2458-2729). Tests (14): abi_spec_malformed_paths,
  free_block_malformed_abi_paths, const_pin_errors, doc_block_bare_prose_keyword_flushes,
  doc_example_dedent_handles_multibyte_whitespace, inline_trap_error_paths,
  deeply_nested_inline_traps_hit_the_statement_depth_cap, match_union_pattern_errors,
  if_elseif_error_paths, loop_close_errors, enum_multiple_member_lines,
  pipeline_placeholder_each_expression_kind_as_rhs, lambda_and_function_type_close_errors,
  native_symbol_must_be_string_and_name_required.
- `testing_and_assignment.rs` (201; 2735-2989). Tests (16):
  parse_testing_block_with_nested_groups_and_cases, parse_testing_rejects_a_non_group_member,
  parse_tgroup_rejects_a_non_case_member, parse_tgroup_requires_a_string_description,
  parse_tcase_requires_a_string_description, parse_tcase_without_end_tcase_is_rejected,
  parse_rejects_a_record_field_assignment_statement,
  parse_rejects_a_nested_record_field_assignment_statement,
  parse_rejects_a_record_field_assignment_in_an_inline_if_branch,
  parse_keeps_the_state_member_assignment_forms,
  parse_keeps_a_member_comparison_in_expression_position,
  parse_keeps_a_plain_identifier_assignment,
  parse_leaves_a_malformed_member_path_to_its_own_diagnostic,
  parse_gives_a_next_line_else_to_the_enclosing_block_if,
  parse_still_binds_a_same_line_else_to_the_inline_if,
  a_res_parameter_keyword_is_a_parse_error_and_state_needs_no_keyword.

### 4.2 `src/cli/dbg/tests.rs` → `src/cli/dbg/tests/` (13 tests)

`src/cli/dbg.rs` keeps `#[cfg(test)] mod tests;`. Citations 0; other refs none.
- `mod.rs` (168): the 8 `use` lines (including `crate::debugger::tests::{…}` — that is
  `src/debugger/tests.rs`, which K does not touch), 2 `mod` lines; shared: `args`, `refusal`,
  `existing`, the fake backend `Paused` + `impl Inferior for Paused`, `symbol`, `Recorded` +
  its 2 impls, `run_session` (used by `sessions.rs` only; kept with the backend so `mod.rs` is
  the harness).
- `operands.rs` (63; 36-103). Tests (7):
  an_existing_file_is_launched_with_the_arguments_after_the_separator,
  digits_that_name_no_file_are_a_pid, a_pid_takes_no_program_arguments,
  a_pid_out_of_range_names_no_process, the_operand_count_is_exactly_one,
  an_unknown_option_is_a_usage_error, a_missing_file_that_is_not_a_pid_fails.
- `sessions.rs` (132; 257-398). Tests (6):
  a_launched_program_is_paused_at_the_prompt_and_killed_on_quit,
  an_attached_program_is_paused_at_the_prompt_and_detached_on_quit,
  a_program_that_is_not_debuggable_is_refused_before_any_process_is_touched,
  a_backend_refusal_is_the_command_s_failure,
  a_program_without_function_names_is_announced_as_not_a_debug_build,
  the_dbg_help_lists_every_prompt_command.

### 4.3 `src/cli/info/tests.rs` → `src/cli/info/tests/` (23 tests)

`src/cli/info.rs` keeps `#[cfg(test)] mod tests;`. Citations 0; other refs none.
- `mod.rs` (197): the 8 `use` lines, 6 `mod` lines; shared: `VERSION`, `DIGEST`, `REGISTRY`,
  `image`, `import`, `linked_images_with`, `linked_images`, `FakeRegistry` + its 3 impls,
  `report`.
- `unsigned_images.rs` (136; 234-383). Tests (6):
  an_unsigned_image_reports_format_arch_libraries_compiler_and_unsigned,
  a_file_without_the_note_is_not_a_mfbasic_binary, a_dynamic_elf_reports_its_interpreter,
  a_dynamic_elf_reports_its_libraries_and_search_path,
  a_mach_o_reports_its_dylibs_and_search_path, a_pe_reports_its_import_dlls.
- `metadata.rs` (64; 406, 1061-1103, 1120-1143). Tests (4):
  malformed_signing_metadata_is_reported_as_such,
  pathologically_nested_json_is_refused_without_recursing,
  a_descriptor_of_an_unknown_version_names_no_compiler, format_utc_renders_civil_dates.
  Helper: SIGNED_PLACEHOLDER.
- `signed/mod.rs` (165; 408-588, 626-630) — nested module root: `use super::*;` + 3 `mod`
  lines; the signing-chain fixture `Chain` + `impl Chain`, `signed_line`, `verdicts`,
  `trust_chain`, `report_line`. `Chain`'s private fields and methods stay private to
  `signed/` and its children.
- `signed/contents.rs` (223; 385-404, 590-624, 632-742, 1145-1218). Tests (6):
  truncated_and_corrupted_images_never_panic, a_signed_image_reports_its_signer_and_verdicts,
  a_changed_byte_anywhere_outside_the_blob_reports_modified_contents,
  a_blob_copied_into_another_image_reports_modified_contents,
  contents_that_carry_no_usable_signature_say_so,
  a_signed_debug_image_reports_intact_contents.
- `signed/chain_checks.rs` (184; 744-856, 971-1059, 1105-1118). Tests (5):
  the_chain_is_checked_against_the_registry_the_blob_names,
  the_ident_key_is_checked_against_the_owner_s_current_key,
  a_chain_attested_by_another_registry_key_is_not_verified,
  every_link_of_the_chain_refuses_an_altered_blob, signer_strings_are_sanitized.
- `signed/ownership.rs` (102; 858-969). Tests (2):
  a_transferred_executable_reports_the_new_owner,
  a_foreign_ident_without_a_chain_is_refused. Helper: probe_index.

### 4.4 `src/optimizer/opt1/placefwd/tests/mod.rs` (41 tests; directory already exists)

- `mod.rs` (184 = the map's 172 + `for_loop`'s 12): the 4 `use` lines, 6 `mod` lines + `mod render; use render::render_ops;`;
  shared NIR builders `list_int`, `rec_type`, `record`, `module_of`, `function`, `param`,
  `bind`, `bind_mut`, `member`, `call`, `with_field`, `assign`, `eval`, `rec_local`, `run`,
  `body_after`, `set_shape`, `for_loop`, `assert_kept`, `global`.
- `render.rs` (56; 151-207) — NIR body rendering: `render`, `render_ops`.
  **Visibility edit:** `render_ops` → `pub(super)` (the topics call it through `mod.rs`'s
  `use render::render_ops;`); `render` stays private.
- `forwarded.rs` (125 = the map's 137 − `for_loop`'s 12; 243-386 without `for_loop` at 333-343). Tests (7): the_benchmark_set_shape_is_forwarded,
  the_remove_at_shape_is_forwarded, a_nested_path_is_forwarded, a_bare_local_is_forwarded,
  a_read_only_window_reaching_the_block_end_is_forwarded,
  a_read_inside_a_nested_for_in_the_window_is_forwarded, a_forward_is_counted.
  `for_loop` (`python3 -I /tmp/loc/span.py src/optimizer/opt1/placefwd/tests/mod.rs 333 345` → 12) is called
  by `forwarded.rs`, `kept.rs` and `scoping.rs` (`grep -n for_loop` → lines 350, 436, 571, 968, 1014, 1045),
  so it stays in `mod.rs`.
- `kept.rs` (166; 395-583). Tests (12):
  an_owner_write_between_the_bind_and_a_read_keeps_the_copy,
  a_read_after_the_owner_write_keeps_the_copy,
  an_owner_write_nested_in_the_last_op_keeps_the_copy, a_read_in_a_trap_handler_keeps_the_copy,
  a_captured_binding_keeps_the_copy, an_address_taken_owner_keeps_the_copy,
  a_captured_owner_keeps_the_copy, a_bare_parameter_source_keeps_the_copy,
  a_member_path_of_a_parameter_is_forwarded, a_scalar_binding_keeps_the_copy,
  a_rebound_owner_keeps_the_copy, level_zero_keeps_the_copy. Helper: closure_capturing.
- `user_calls_and_globals.rs` (194; 597-815). Tests (10): the_global_set_shape_is_forwarded,
  the_global_remove_at_shape_is_forwarded, a_by_ref_capture_field_set_is_forwarded,
  a_by_ref_capture_with_a_user_call_keeps_the_copy, a_user_call_in_the_window_keeps_the_copy,
  a_user_call_inside_the_owner_write_keeps_the_copy,
  a_collections_call_with_a_lambda_in_the_window_keeps_the_copy,
  an_indirect_call_in_the_window_keeps_the_copy,
  a_store_global_between_the_bind_and_a_read_keeps_the_copy,
  an_owned_local_forwards_across_a_user_call. Helpers: store_global, helper,
  body_after_with_helper, assert_kept_with_helper, global_set_shape, by_ref_rec.
- `state_resources.rs` (116; 819-957). Tests (8): the_state_set_shape_is_forwarded,
  the_state_remove_at_shape_is_forwarded, the_state_remove_key_shape_is_forwarded,
  the_state_remove_shape_is_forwarded, closing_the_resource_in_the_window_keeps_the_copy,
  passing_the_resource_to_a_user_sub_keeps_the_copy,
  a_state_assign_between_the_bind_and_a_read_keeps_the_copy,
  a_non_state_member_of_a_resource_keeps_the_copy. Helpers: state_type, state_of,
  state_assign, state_field_update, state_body_after, assert_state_kept, state_cur,
  state_set_shape.
- `scoping.rs` (87; 961-1060). Tests (4):
  a_name_reused_by_a_later_scope_forwards_both_bindings,
  a_binding_bound_again_in_its_own_region_keeps_the_copy, an_owner_not_in_scope_keeps_the_copy,
  a_name_that_is_a_function_anywhere_trips_the_call_gate.

Citations 0; other refs none.

### 4.5 `src/ast/scope_privates.rs` → `src/ast/scope_privates/tests/` (20 tests)

Citations: 2, both name host code that stays. `.ai/codegen-invariants.md:463` names the host
path (unchanged).
- `mod.rs` (10): the 3 `use` lines, 3 `mod` lines; shared: `MAIN`, `mangled`.
- `renaming.rs` (187; 642-854). Tests (6): renames_private_decls_and_rewrites_references,
  locals_shadow_private_names_and_are_left_alone, a_file_with_no_private_items_is_untouched,
  private_shadowing_a_public_of_the_same_name_warns,
  item_name_vis_and_line_cover_non_visibility_items,
  private_type_used_in_nested_type_strings_is_rewritten. Helper: BROAD.
- `state_and_types.rs` (147; 856-1050). Tests (12):
  private_binding_used_as_state_assign_target_is_rewritten,
  a_local_shadowing_a_private_keeps_its_state_assign_target_bare,
  a_local_named_like_a_private_func_is_left_alone,
  rewrite_type_str_is_identity_when_no_private_types,
  two_files_sharing_a_path_hit_the_duplicate_hash_arm,
  shadow_warning_reports_the_line_of_each_decl_kind,
  item_line_of_a_non_visibility_item_is_zero, function_param_state_type_is_rewritten,
  resource_close_fn_naming_a_private_func_is_rewritten,
  link_block_signatures_referencing_a_private_type_are_rewritten,
  let_binding_state_type_is_rewritten, set_literal_element_type_and_elements_are_rewritten.
- `overloads.rs` (49; 1052-1111). Tests (2):
  a_public_overload_beside_a_private_overload_keeps_its_name,
  a_public_func_beside_a_private_non_function_is_still_renamed. Helper: declared_functions.

### 4.6 `src/cli/build/mod.rs` → `src/cli/build/tests/` (142 tests)

The two `#[cfg(test)] pub(crate) fn native_libraries_for_test` / `build_source_dependencies_for_test`
(1269-1318; called by `src/testutil.rs`) stay in the host. Citations: 24, all name host code.
`scripts/test-accept.sh:260` cites `src/cli/build/mod.rs:1488` — stale (§2); repointed in
Phase 3.
- `mod.rs` (103): `use super::*;`, the two mid-module `use` lines (old 2502-2503, so every
  topic sees them), 19 `mod` lines; shared: `s`, `build_err`, `write_executable_project`,
  `copy_test_project`, `write_package_project`, `running_as_root`.
- `options_basics.rs` (203; 1346-1592). Tests (17): build_output_from_flag_maps_every_flag,
  build_output_from_flag_single_dash_aliases_double_dash, parse_build_options_defaults,
  parse_build_options_parses_target_both_forms,
  parse_build_options_single_dash_aliases_double_dash,
  parse_build_options_app_debug_implies_app_mode, parse_build_options_debug_sets_flag,
  parse_build_options_rejects_repeated_debug, parse_test_options_debug_sets_flag,
  parse_build_options_parses_config_both_forms, parse_build_options_rejects_repeated_config,
  parse_build_options_rejects_empty_config, parse_test_options_parses_config_both_forms,
  parse_test_options_rejects_repeated_config, parse_test_options_rejects_empty_config,
  parse_test_options_rejects_app_debug, parse_test_options_single_dash_aliases_double_dash.
- `options_flags.rs` (193; 1594-1830). Tests (16):
  parse_build_options_target_requires_value, parse_build_options_target_rejects_malformed,
  parse_build_options_sign_both_forms_and_conflicts, parse_build_options_unsigned_flag,
  parse_build_options_verbosity_defaults_to_normal, parse_build_options_quiet_both_spellings,
  parse_build_options_verbose_both_spellings, parse_build_options_quiet_and_verbose_conflict,
  parse_build_options_repeated_verbose_selects_trace,
  parse_build_options_quiet_and_trace_conflict, parse_test_options_is_quiet,
  parse_test_options_verbose_both_spellings, parse_test_options_repeated_verbose_selects_trace,
  parse_test_options_rejects_quiet, parse_build_options_regalloc_both_forms_and_bad_value,
  parse_build_options_opt_level_every_spelling.
- `options_errors_and_modes.rs` (111; 1832-1860, 1869-1879, 2865-2955). Tests (11):
  parse_build_options_opt_level_rejects_unlanded_and_malformed,
  parse_build_options_rejects_unknown_option_and_two_locations,
  parse_build_options_takes_a_positional_location,
  parse_build_options_defaults_to_console_mode, parse_build_options_accepts_app_flag,
  parse_build_options_rejects_duplicate_app_flag,
  parse_build_options_app_flag_composes_with_native_output,
  parse_build_options_combines_output_flags_in_order,
  parse_build_options_accepts_single_dash_aliases,
  parse_build_options_rejects_duplicate_output_flag,
  parse_build_options_no_output_flags_means_full_build.
- `package_classification.rs` (179; 1881-2104). Tests (13): package_verification_labels,
  source_is_local_classifies_sources, signing_ident_defaults_to_owner_hash_name,
  signing_ident_rejects_bad_idents, signing_ident_accepts_a_transferred_ident,
  classify_installed_package_reads_unsigned_fixture,
  classify_installed_package_treats_missing_file_as_tampered,
  classify_installed_package_treats_garbage_as_tampered, verify_and_report_no_packages_is_ok,
  verify_and_report_missing_dependency_file_is_skipped,
  verify_and_report_unsigned_remote_requires_flag,
  verify_and_report_refuses_a_pinned_version_mismatch,
  verify_and_report_unsigned_local_is_allowed.
- `signing_metadata.rs` (82; 2106-2190). Tests (3): apply_signing_metadata_copies_fields,
  executable_signing_metadata_json_is_valid_json,
  decode_trust_anchor_accepts_metadata_key_form.
- `build_project_basics.rs` (206; 2233-2422, 2451-2498). Tests (11):
  builtin_codegen_corpora_lower_in_process, build_project_validates_a_bad_manifest,
  build_project_rejects_app_mode_for_non_app_target, build_project_builds_a_host_executable,
  build_project_rejects_a_traversing_project_name, build_project_clears_stale_build_dir,
  mfb_test_host_run_leaves_project_build_dir_untouched, build_project_writes_ast_and_ir_dumps,
  build_project_builds_a_package, build_project_rejects_native_output_for_a_package,
  build_project_reports_a_source_error.
- `vendoring/mod.rs` (29; 2505-2541) — nested module root: `use super::*;` + 3 `mod` lines;
  `vendor_locator`, `resolved`, `OWN_UNIT`, `write_vendor_source`.
- `vendoring/output_names.rs` (124; 2543-2703). Tests (7):
  the_copied_filename_is_the_emitted_dlopen_name,
  imported_vendor_file_is_read_from_the_per_package_directory,
  two_packages_vendoring_the_same_filename_land_as_two_distinct_files,
  colliding_output_names_with_differing_hashes_are_rejected,
  colliding_output_names_with_identical_hashes_are_allowed, no_vendor_locators_writes_nothing,
  vendor_output_dirs_match_the_emitted_rpath_per_shape.
- `vendoring/vendor_libraries.rs` (108; 3163-3298). Tests (8):
  verify_vendor_libraries_reports_a_missing_file,
  verify_vendor_libraries_rejects_a_locator_without_a_hash,
  verify_vendor_libraries_rejects_a_hash_mismatch,
  verify_vendor_libraries_accepts_a_matching_hash,
  vendor_source_path_distinguishes_own_and_imported_units,
  vendor_source_path_finds_a_source_dependency_s_staged_vendor,
  stage_source_dependency_vendor_copies_the_flat_vendor_files,
  copy_vendor_libraries_errors_when_the_source_is_missing.
- `vendoring/build_project.rs` (160; 3941-4124). Tests (3):
  build_project_builds_an_executable_that_vendors_a_library,
  build_project_reports_a_resource_copy_failure,
  build_project_rejects_a_package_link_without_a_libraries_entry.
- `resources.rs` (220; 2705-2819, 3018-3159). Tests (9): resource_output_dir_per_build_shape,
  resource_src_fixed_prefix_splits_at_first_glob,
  copy_resources_refuses_a_source_that_resolves_outside_the_project,
  copy_resources_maps_the_worked_examples, copy_resources_handles_a_root_level_glob,
  copy_resources_reports_an_unreadable_source_directory,
  copy_resources_reports_an_unreadable_subdirectory,
  copy_resources_reports_an_uncreatable_destination,
  copy_resources_reports_an_unreadable_source_file.
- `reporting_and_modes.rs` (99; 2821-2863, 2976-3014, 3622-3672). Tests (8):
  emitted_link_targets_track_what_each_build_mode_actually_emits,
  run_test_binary_maps_every_exit_status,
  generate_coverage_report_warns_when_the_covmap_is_missing,
  generate_coverage_report_writes_html_from_a_seeded_covmap,
  build_output_label_names_every_variant, build_project_verbose_runs_the_phase_reporter,
  build_project_trace_runs_the_compile_profiler,
  build_project_rejects_app_mode_for_a_package.
- `signed_packages/mod.rs` (99; 3310-3420, 3506-3519) — nested module root: `use super::*;`
  + 1 `mod` line; `SignedTamper`, `SignedFixture`, `build_signed_fixture`, `write_mfp`,
  `classify_pinned`.
- `signed_packages/verification.rs` (175; 3422-3504, 3521-3618, 4126-4148). Tests (13):
  classify_signed_package_without_a_trust_anchor_is_untrusted,
  classify_signed_package_with_a_malformed_anchor_is_untrusted,
  classify_signed_package_with_a_malformed_header_ident_is_untrusted,
  classify_signed_package_with_a_mismatched_anchor_is_untrusted,
  classify_signed_package_without_a_pinned_server_key_is_untrusted,
  classify_signed_package_with_a_bad_attestation_signature,
  classify_signed_package_with_a_bad_proof_signature,
  classify_signed_package_with_a_bad_container_signature,
  classify_signed_package_with_a_broken_payload_hash, classify_fully_signed_package_verifies,
  verify_and_report_skips_malformed_package_entries,
  verify_and_report_accepts_a_verified_dependency,
  verify_and_report_refuses_a_tampered_dependency.
- `app_mode.rs` (147; 3674-3752, 4150-4240). Tests (4):
  build_project_app_mode_missing_icon_is_rejected_per_target,
  build_project_rejects_importing_app_without_app_mode,
  build_project_app_mode_present_icon_resolves_then_backend_rejects,
  build_project_app_mode_without_an_icon_field_takes_the_none_arm. Helpers:
  write_app_project_missing_icon, write_app_project_present_icon.
- `build_project_validation.rs` (165; 3754-3939). Tests (8):
  build_project_rejects_expect_outside_a_test_case,
  build_project_coverage_test_writes_a_report,
  build_project_unknown_kind_validates_and_builds_nothing,
  build_project_writes_every_native_dump, build_project_rejects_native_dumps_for_a_package,
  build_project_rejects_sign_with_output_flags,
  build_project_sign_without_a_local_ident_key_fails,
  build_project_rejects_a_link_without_a_libraries_entry.
- `build_project_manifests.rs` (145; 4242-4411). Tests (6):
  build_project_sign_rejects_a_malformed_manifest_ident,
  build_project_export_in_an_executable_is_a_verify_error,
  build_project_errors_when_build_path_is_a_file,
  build_project_executable_missing_dependency_is_a_hard_error,
  build_project_package_missing_dependency_is_a_hard_error,
  build_project_reads_installed_dependency_exports.
- `source_dependencies.rs` (135; 4413-4563). Tests (2):
  build_project_compiles_a_source_directory_dependency,
  build_project_rejects_a_source_dependency_cycle. Helper: write_source_package.
- `cross_target_and_readonly.rs` (94; 4565-4672). Tests (3):
  build_project_cross_target_test_reports_the_artifact,
  build_project_package_write_to_a_readonly_location_fails,
  build_project_dump_writers_report_a_readonly_location.

### 4.7 `src/audit/collect/findings.rs` → `src/audit/collect/findings/tests/` (10 tests)

Citations: 6, all host code. (`src/audit/collect/dependencies/` is an unrelated sibling.)
- `mod.rs` (13): `use super::*;`, 2 `mod` lines; shared: `lockfile`.
- `lockfile_and_packages.rs` (182; 296-510). Tests (7): lock_missing_when_required,
  lock_stale_is_error_when_locked_warning_otherwise, lock_matching_present_yields_no_finding,
  malformed_lockfile_is_a_finding_and_errors_under_locked,
  dependency_findings_cover_each_status, package_findings_flag_failed_and_unsigned,
  package_findings_flags_exported_mutable_state_from_mfp. Helpers: malformed_lockfile,
  inputs, dependency, package_entry.
- `resources_and_permissions.rs` (100; 512-615). Tests (3):
  resource_findings_flag_lexical_close, permission_findings_map_each_capability_and_dedup,
  sort_findings_orders_by_category_then_code. Helper: permission.

### 4.8 `src/audit/collect/project.rs` → `src/audit/collect/project/tests/` (10 tests)

Citations: 6, all host code.
- `mod.rs` (73): the 4 `use` lines, 2 `mod` lines; shared: `internal_file`, `resource_decl`,
  `link_block`, `file`.
- `native_resources.rs` (116; 244-368). Tests (5):
  native_resources_derive_close_may_fail_from_link_success_on,
  native_resource_unknown_close_fn_defaults_may_fail_false,
  native_links_report_every_linked_symbol_sorted, no_resources_yields_empty,
  native_resources_sorted_across_files.
- `links_and_summary.rs` (154; 370-540). Tests (5):
  internal_files_are_excluded_from_resources_and_links,
  native_link_free_symbol_populates_close_function,
  collect_libraries_reads_locators_and_resource_files, project_summary_reads_manifest_fields,
  project_summary_defaults_ident_to_name_and_empties.

### 4.9 `src/audit/collect/source.rs` → `src/audit/collect/source/tests/` (26 tests)

The `#[cfg(test)] fn in_inline_census` (1021-1026) stays in the host. Citations: 15, all host
code; `src/docs/spec/language/08_error-model.md:157` names the host path (unchanged).
- `mod.rs` (19): `use super::*;`, 4 `mod` lines; shared: `project`, `trap_classification`.
- `builtin_fallibility.rs` (192; 1102-1330). Tests (6):
  callee_qualified_name_uses_dot_separator, overloads_get_their_own_fallibility_verdict,
  toint_over_an_enum_is_not_fallible, tostring_is_fallible_on_every_overload,
  census_builtins_are_fallible_exactly_when_they_declare_an_error,
  every_declaring_builtin_is_fallible.
- `capabilities_and_resources.rs` (155; 1332-1507). Tests (6):
  every_builtin_is_fallible_exactly_when_it_declares_an_error,
  every_host_capability_is_disclosed,
  relaxed_trust_is_reported_only_when_the_flag_is_literally_true,
  permissions_are_collected_and_deduplicated, resources_found_across_nested_control_flow,
  resources_found_inside_match_case.
- `trap_classification.rs` (144; 1509-1567, 1731-1825). Tests (7):
  trap_classification_propagates, trap_classification_fails,
  trap_classification_returns_value, trap_classification_recovers,
  classify_recurses_through_if_match_and_loop_for_propagate,
  classify_recurses_through_if_match_and_loop_for_fail,
  classify_recurses_through_if_match_and_loop_for_return_value.
- `propagation_and_walks.rs` (159; 1569-1721, 1827-1851). Tests (7):
  user_function_becomes_fallible_by_calling_fallible_builtin,
  tcp_and_udp_calls_are_fallible_and_propagate, fail_statement_makes_function_fallible,
  walk_visits_loop_control_and_named_and_lambda_and_trapped,
  match_guard_and_case_bodies_are_walked,
  nested_fail_and_propagate_inside_control_flow_is_detected,
  walk_visits_diverse_expression_and_statement_forms.

### 4.10 `src/audit/report.rs` → `src/audit/report/` (4 tests)

`report.rs` holds two cfg(test) modules: `pub(super) mod testsupport` (255-515: `finding` 11
LOC, `full_report` 210, `empty_report` 31; imported by `src/audit/json.rs:421`,
`src/audit/text.rs:301` as `super::super::report::testsupport::*`) and `mod tests` (517-565).
- Host keeps `#[cfg(test)] pub(super) mod testsupport;` and `#[cfg(test)] mod tests;`.
- `src/audit/report/testsupport/mod.rs` (45): `use super::*;`, `finding`, `empty_report`,
  `mod full_report; pub(in crate::audit) use full_report::full_report;`.
- `src/audit/report/testsupport/full_report.rs` (211): `use super::*;` + `full_report` (one
  210-LOC fn).
- `src/audit/report/tests/mod.rs` (42): the unchanged body of `mod tests`. Tests (4):
  severity_as_str_covers_all_variants, counts_tallies_each_severity,
  counts_empty_report_is_zero, category_rank_orders_known_and_unknown.

The three fns are already `pub(in crate::audit)`, so `report::testsupport::full_report` keeps
resolving for `json.rs`/`text.rs` through the re-export. Citations 0 test-side.

### 4.11 `src/cli/man.rs` → `src/cli/man/tests/` (49 tests)

Citations: 7, all host code. Path refs `scripts/man-census.sh:5,517`, `scripts/man-manual.sh:10`,
`.ai/man-content.md:34,408,443,493` name host code (`render_all_markdown`,
`render_topic_overview`, lines 1-15 and 473) — unchanged.
- `mod.rs` (39): `use super::*;`, 7 `mod` lines; shared: `s`, `errors_row`,
  `parameters_table`, `signature_section`.
- `resource_and_errors_sections.rs` (167; 1110-1340). Tests (6):
  every_rendered_signature_marks_a_resource_collection_element_res,
  every_resource_page_states_transferability_from_the_sendable_bit,
  renders_a_csv_function_from_the_clean_room_registry,
  markdown_includes_summary_description_and_parameters,
  a_tcp_member_that_gained_errors_still_renders_its_other_sections,
  a_member_with_no_declared_errors_omits_the_errors_section.
- `errors_tables.rs` (127; 1349-1515). Tests (6):
  a_multi_overload_errors_table_names_the_overloads_that_raise_each_error,
  every_errors_overload_number_names_a_rendered_signature,
  an_overload_that_declares_no_errors_is_named_as_infallible,
  a_single_overload_member_errors_table_is_unchanged,
  a_package_overview_errors_table_has_no_overload_column,
  multi_overload_function_renders_overloads_and_union_parameters.
- `parameters_tables.rs` (139; 1528-1692). Tests (6):
  every_overloads_parameters_appear_in_its_parameters_table,
  collections_get_shows_the_map_key_as_its_own_parameter_row,
  a_parameters_table_with_no_disagreement_has_no_overloads_column,
  every_parameters_overload_number_names_a_rendered_signature,
  function_types_use_public_package_qualification, no_function_renders_the_package_overview.
- `types_pages.rs` (149; 1694-1887). Tests (6):
  renders_a_types_page_with_records_and_unions, the_stroke_style_enums_and_paint_fields_render,
  a_records_own_description_renders_above_its_fields,
  types_page_renders_a_record_only_package, types_page_lists_enums_and_resources,
  types_shows_a_friendly_message_when_the_package_has_no_public_types.
- `index_topics_and_options.rs` (90; 1889-2003). Tests (12):
  bare_man_renders_the_index_of_packages_and_topics,
  package_overview_sorts_functions_and_links_its_types_page, rejects_unknown_package,
  renders_a_guide_topic_overview, renders_a_guide_topic_sub_page,
  rejects_unknown_topic_sub_page, topic_all_renders_the_overview_and_every_sub_page,
  all_renders_the_whole_registry_manual, all_renders_one_package_in_full,
  all_rejects_a_function_argument, rejects_unknown_option, rejects_missing_function.
- `declarations_and_constants.rs` (130; 2022-2190). Tests (8):
  a_return_type_overload_renders_every_form,
  only_a_return_type_overload_gets_the_expected_type_note,
  every_member_renders_one_declaration_per_implementation,
  a_constants_only_package_lists_every_constant_with_value_and_message,
  every_registered_constant_is_listed_on_its_package_page,
  a_record_constant_renders_as_a_qualified_record_construction,
  the_no_types_page_names_what_the_package_has,
  naming_a_constant_as_a_page_points_at_the_package_page.
- `placeholders_and_qualification.rs` (141; 2192-2362). Tests (5):
  referenced_functions_dedupes_sorts_and_drops_self,
  types_page_field_tables_use_public_package_qualification,
  a_declaration_resolves_an_arg_placeholder_to_the_parameters_type,
  no_rendered_page_spells_an_arg_placeholder, no_rendered_page_spells_a_dotted_package_type.

### 4.12 `src/cli/pkg.rs` → `src/cli/pkg/tests/` (60 tests)

`src/cli/pkg/` exists and holds only `command_tests.rs`. The other cfg(test) item,
`#[path = "pkg/command_tests.rs"] mod command_tests;` (36-38), stays; plan-176-L rewrites it
when the host becomes `pkg/mod.rs`. Citations: 38, all host code.
`scripts/coverage-exceptions.txt:17` names the host path (unchanged in K).
- `mod.rs` (62): `use super::*;`, 6 `mod` lines; shared: `UNSIGNED_FIXTURE`, `s`, `usage`,
  `parsed`, `add_opts`, `index_version_with_abi`, `imports_map`.
- `formatting_and_selection.rs` (180; 2536-2735). Tests (13):
  signature_type_name_maps_known_and_unknown, hex_bytes_formats_lowercase_two_digit,
  empty_marker_marks_empty_strings, package_export_kind_names, package_verify_status_messages,
  state_is_floating_eligible_classifies_states, select_index_version_empty_index_errors,
  select_index_version_exact_request, select_index_version_exact_request_rejects_blocked,
  select_index_version_floating_picks_newest_eligible,
  select_index_version_floating_none_eligible_errors,
  a_pinned_dependency_demands_an_exact_version,
  package_verify_line_formats_with_and_without_version. Helpers: index_version, index.
- `command_dispatch.rs` (82; 2737-2854). Tests (6): run_pkg_requires_a_subcommand,
  run_pkg_rejects_unknown_command, run_pkg_usage_errors_show_the_full_pkg_command_set,
  help_lists_each_moved_command_under_repo_only, run_pkg_usage_errors_for_wrong_arity,
  run_pkg_rejects_the_moved_publisher_commands.
- `remove.rs` (103; 2905-3038). Tests (6): parse_remove_options_reads_the_target_and_yes,
  remove_rejects_a_target_that_is_not_declared,
  remove_refuses_when_a_declared_package_is_not_installed,
  removal_closure_walks_reverse_dependencies, removal_closure_terminates_on_a_cycle,
  removal_closure_yields_each_ident_once_in_a_diamond.
- `update_and_add.rs` (174; 3040-3268). Tests (9):
  select_update_version_covers_the_selection_matrix,
  parse_update_options_reads_the_target_and_flags,
  parse_update_options_rejects_bad_argument_shapes,
  update_rejects_a_target_that_is_not_declared,
  update_treats_a_path_positional_as_an_undeclared_ident, infer_pin_follows_the_matrix,
  parse_add_options_reads_the_target_and_flags, parse_add_options_rejects_bad_argument_shapes,
  add_rejects_no_pin_on_a_file_url.
- `command_errors.rs` (184; 3270-3495). Tests (22): add_package_rejects_bad_target,
  add_package_from_registry_rejects_malformed_ident, transfer_offer_rejects_bad_ident,
  transfer_accept_requires_at_sign, set_release_state_rejects_bad_state,
  set_release_state_requires_package_project, run_pkg_doc_rejects_bad_arguments,
  write_package_doc_reports_missing_package, write_package_doc_renders_from_a_real_package,
  print_package_info_reads_a_real_package, print_package_info_reports_missing_file,
  validate_package_file_reports_missing_package,
  validate_package_file_validates_an_unsigned_fixture,
  validate_package_file_rejects_garbage_container, project_pinned_ident_key_finds_declared_pin,
  verify_packages_reports_no_manifest, verify_packages_ok_with_no_dependencies,
  check_abi_requires_package_project, publish_requires_package_project,
  print_publish_verify_report_handles_both_diagnostic_states,
  verify_package_dependency_reports_missing_package_as_invalid,
  add_package_from_file_reports_missing_source.
- `package_verify.rs` (120; 3497-3623). Tests (4):
  package_verify_status_checks_name_and_version,
  package_verify_rejects_range_syntax_as_literal_version,
  package_verify_line_shows_project_and_package_versions,
  package_verify_reads_source_package_manifest. Helper: test_temp_dir.

### 4.13 `src/cli/resolve.rs` → `src/cli/resolve/tests/` (27 tests)

Citations: 20, all host code. `scripts/coverage-exceptions.txt:18` names the host (unchanged).
- `mod.rs` (67): `use super::*; use std::cmp::Ordering;`, 4 `mod` lines; shared:
  `index_version`, `node_with`, `select_node_err`, `requirer`, `manifest_with_packages`.
- `versions_and_install.rs` (162; 1356-1404, 1465-1628). Tests (10):
  version_comparison_orders_releases_and_prereleases,
  version_comparison_does_not_coerce_malformed_components_to_zero,
  superset_requires_every_symbol_hash, read_manifest_reads_and_validates,
  install_without_a_lock_errors_before_network,
  install_without_declared_dependencies_is_a_no_op,
  install_with_stale_lock_errors_before_network,
  install_with_a_moved_pin_errors_before_network,
  install_with_a_moved_floor_passes_the_drift_gate, read_lock_absent_returns_none.
- `manifest_and_lock.rs` (141; 1638-1776, 2049-2089). Tests (7):
  apply_manifest_change_zero_dependencies_writes_manifest_and_drops_the_lock,
  apply_manifest_change_zero_dependencies_tolerates_a_missing_lock,
  apply_manifest_change_rejects_bad_text_without_writing,
  registry_dependency_count_matches_the_resolver_seeding_filter, read_lock_rejects_non_json,
  write_lock_writes_a_readable_lockfile, lockfile_round_trips_byte_identically.
- `select_node.rs` (75; 1778-1859). Tests (7): select_node_picks_highest_superset_version,
  select_node_honors_a_pin, select_node_rejects_conflicting_pins,
  select_node_rejects_unpublished_pin, select_node_rejects_blocked_pin,
  select_node_reports_diamond_conflict, select_node_reports_no_satisfying_version.
- `drift.rs` (160; 1861-2047). Tests (3): classify_drift_covers_every_class,
  classify_drift_reports_every_drifted_dependency,
  print_lock_diff_covers_add_change_keep_and_remove.

### 4.14 `src/debugger/breakpoints.rs` → `src/debugger/breakpoints/tests/` (20 tests)

`src/debugger/breakpoints/` exists with `report.rs` only. Citations: 11, all host code;
`src/docs/spec/tooling/10_debugger.md:69` names the host path (plan-176-L changes it).
- `mod.rs` (88): the 2 `use` lines, 5 `mod` lines; shared: `Program`, `MEMORY`, `PARK`,
  `RAISER`, `ENTRY`, `STORE`, `IS_PRIME`, `EXIT`, `ABORT`, `exit`, `abort`, `RET`, `function`,
  `impl Program` (inherent methods stay here so every topic calls them), `name`, `run`.
- `fake_program.rs` (160; 1641-1801): `impl Inferior for Program` (trait impl; its methods
  need no visibility change; `Program`'s private fields are visible to the child).
- `places.rs` (199; 1828-1918, 2000-2120). Tests (6): an_address_or_a_name_is_parsed,
  a_symbol_wins_over_a_written_name_and_every_copy_is_covered,
  a_function_and_an_offset_is_a_place_in_each_function_so_named,
  an_address_is_described_by_the_function_that_holds_it,
  an_unbound_linked_function_is_pending_until_the_entry_point,
  removing_by_name_takes_symbols_first_then_written_names.
- `breakerror_and_entry.rs` (145; 1920-1998, 2122-2152, 2537-2583). Tests (6):
  breakerror_stops_in_the_raiser_only_for_a_watched_code,
  a_breakpoint_of_the_user_s_on_the_park_shares_it_with_breakerror,
  a_user_breakpoint_at_the_entry_point_is_reported, an_exit_leaves_no_breakpoint_to_remove,
  a_restart_places_a_shared_park_breakpoint_once,
  a_watched_error_with_commands_runs_them_and_keeps_them.
- `watchpoints_and_commands.rs` (156; 2154-2315). Tests (4):
  a_watchpoint_stops_only_when_a_write_changes_its_bytes,
  a_watchpoint_covers_1_to_8_bytes_within_a_multiple_of_8, a_restart_drops_every_watchpoint,
  a_stop_with_commands_runs_on_and_one_without_ends_the_run.
- `conditions.rs` (204; 2317-2535). Tests (4): a_breakpoint_with_a_test_stops_only_when_it_holds,
  a_test_that_cannot_be_read_stops_only_in,
  pending_breakpoints_with_different_tests_share_their_place,
  a_finish_returns_at_a_ret_holding_a_breakpoint_with_commands. Helper: test.

### 4.15 `src/debugger/breakpoints/report.rs` → `src/debugger/breakpoints/report/tests/` (12 tests)

Citations: 5, all host code.
- `mod.rs` (112): the 3 `use` lines, 3 `mod` lines; shared: `SHUTDOWN`, `BACK`, `MIDWAY`,
  `AFTER`, `WRITE`, `LINE`, `WRITTEN`, `Event`, `Exiting` + `impl Exiting`, `function`,
  `session`, `report`, `EXIT_WITH_REPORT`.
- `fake_exiting.rs` (150; 389-537): `impl Inferior for Exiting`.
- `report_capture.rs` (87; 577-669). Tests (6):
  the_report_is_kept_from_the_terminal_and_shown_after_the_exit,
  only_the_reporting_threads_writes_to_stderr_are_taken,
  a_write_before_the_report_is_the_programs,
  a_second_pass_through_the_shutdown_does_not_replace_the_report,
  a_stop_inside_the_report_leaves_the_rest_to_the_program,
  a_breakpoint_of_the_users_on_the_shutdown_leaves_the_report_to_the_program.
- `endings.rs` (85; 671-759). Tests (6): a_program_that_ends_inside_the_report_keeps_what_it_wrote,
  a_program_that_ends_without_a_report_says_so, a_build_without_debug_has_no_report,
  a_restart_forgets_the_report_and_keeps_the_next,
  a_step_that_ends_the_program_says_where_the_report_is,
  a_report_line_is_shown_safe_for_a_terminal.

### 4.16 `src/debugger/prompt.rs` → `src/debugger/prompt/tests/` (63 tests)

Citations: 28, all host code.
- `mod.rs` (85): the 3 `use` lines, 13 `mod` lines; shared: `Recorder` + `impl Recorder`, `c`,
  `symbol`, `CONSOLE`, `session`, `drive`.
- `fake_recorder.rs` (173; 2396-2568): `impl Inferior for Recorder`.
- `session_basics.rs` (112; 2596-2657, 3267-3323). Tests (10):
  help_lists_every_command_in_both_spellings, quit_ends_the_session_once_in_both_spellings,
  end_of_input_quits, echo_repeats_each_command_after_the_prompt,
  a_bad_command_reports_and_the_prompt_continues, an_externally_ended_process_is_reported,
  a_listing_that_fails_reports_and_the_prompt_continues, a_bad_list_argument_is_reported,
  empty_listings_print_nothing,
  a_table_aligns_every_column_to_its_widest_entry_by_characters.
- `list_func_and_threads.rs` (132; 2659-2728, 3530-3594). Tests (6):
  a_bare_list_shows_what_can_be_listed_in_both_spellings, list_function_is_list_func,
  list_func_groups_by_origin_then_address_and_keeps_repeats,
  list_thread_names_each_thread_by_name_frame_or_id,
  list_func_keeps_the_types_and_names_asked_for, a_glob_matches_the_whole_name.
- `list_reg.rs` (142; 2730-2876). Tests (7):
  list_reg_alone_shows_the_registers_with_a_role_then_its_usage,
  list_reg_alone_shows_only_its_usage_after_the_program_ends,
  list_reg_shows_each_group_of_the_main_thread_or_the_one_asked_for,
  list_reg_shows_the_main_thread_or_the_one_asked_for,
  list_reg_refuses_a_thread_or_register_that_does_not_exist,
  list_reg_shows_just_the_registers_named_by_spaces_or_commas,
  a_negative_64_bit_value_shows_as_a_signed_decimal.
- `places_and_memory.rs` (151; 2878-2963, 3099-3105, 3404-3429, 4299-4335). Tests (5):
  show_mem_parses_an_address_or_a_register_and_offset, a_function_and_an_offset_name_a_place,
  memory_lines_lead_with_full_addresses, an_address_may_carry_an_offset_either_way,
  show_mem_reads_through_memory_in_brackets.
- `errors_and_types.rs` (129; 2965-3097). Tests (5): breakerror_takes_codes_names_any_or_clear,
  show_error_takes_a_thread_and_needs_a_debug_build, show_type_needs_a_debug_build_s_arenas,
  show_type_alone_shows_its_usage, show_type_parses_a_type_a_location_and_a_view.
- `arenas.rs` (170; 3107-3148, 3869-3994, 4337-4345). Tests (8):
  restart_needs_a_launched_program, list_arena_without_a_registry_asks_for_a_debug_build,
  arena_lines_round_up_to_kilobytes_and_mark_unowned_arenas,
  the_arena_state_lists_every_field_and_each_non_empty_entry,
  blocks_print_their_headers_numbered_from_the_oldest,
  a_dump_shows_offsets_bytes_and_printable_text, show_arena_parses_its_forms,
  show_arena_without_a_registry_asks_for_a_debug_build.
- `stepping.rs` (112; 3150-3265). Tests (6): step_runs_the_main_thread_and_says_where_it_stopped,
  step_reports_an_exception_a_wait_or_the_end_of_the_program, step_runs_the_thread_asked_for,
  step_refuses_a_bad_count_or_thread, step_all_runs_every_thread_and_reports_each,
  step_all_reports_an_exception_or_the_end_of_the_program.
- `backtrace_and_dis.rs` (146; 3352-3402, 3431-3528). Tests (3):
  backtrace_parses_its_forms_in_both_spellings,
  dis_takes_a_count_a_place_or_a_function_and_counts_back_when_negative,
  dis_heads_each_function_s_instructions_with_that_function.
- `breakpoint_commands.rs` (141; 3325-3350, 3596-3613, 3651-3747). Tests (3):
  breakpoints_are_set_listed_and_removed, a_bad_breakpoint_command_is_refused,
  watchpoint_breakpoint_and_breakerror_take_commands_after_a_semicolon.
- `running.rs` (152; 3615-3649, 3749-3867). Tests (5): run_reports_each_kind_of_stop,
  a_stop_runs_its_commands_and_the_program_runs_on,
  a_stop_whose_commands_do_not_end_in_run_stays_stopped_and_is_counted,
  run_reports_a_changed_watchpoint_and_a_raised_error, run_reports_an_exit_status.
- `conditional_breakpoints.rs` (125; 3996-4120). Tests (2):
  breakpointif_takes_a_test_a_place_and_commands,
  a_breakpoint_with_a_test_is_listed_with_it_and_bp_r_takes_every_one_there.
- `jump.rs` (175; 4122-4297). Tests (3): jump_moves_the_current_thread_and_sets_its_registers,
  jump_refuses_a_bad_place_register_or_value, a_stop_can_jump_the_stopped_thread_and_run_on.

### 4.17 `src/debugger/runtime.rs` → `src/debugger/runtime/tests/` (14 tests)

Citations: 5, all host code; `src/docs/spec/tooling/10_debugger.md:67` names the host
(unchanged).
- `mod.rs` (122): the 3 `use` lines, 3 `mod` lines; shared: `Memory` + `impl Memory` +
  `impl Inferior for Memory`, `thread`, `TABLE`, `SLOTS`, `BLOCK`, `WORKER_ARENA`,
  `with_worker`.
- `threads.rs` (77; 703-789). Tests (4):
  a_console_program_s_initial_thread_runs_main_and_workers_are_user_threads,
  an_app_s_initial_thread_is_the_app_and_main_is_where_the_main_arena_is,
  without_symbols_the_registers_decide, without_symbols_two_threads_that_fit_name_neither.
  Helper: main_arena_at.
- `arenas_and_registry.rs` (122; 791-880, 922-963). Tests (6):
  the_registry_lists_live_arenas_with_their_owner_and_mapped_bytes,
  a_registry_not_yet_made_lists_nothing, a_block_chain_that_loops_is_refused,
  an_unreadable_thread_table_is_reported, blocks_list_oldest_first,
  the_state_fields_tile_the_arena_state. Helper: blocks_err.
- `names_and_errors.rs` (89; 882-920, 965-1019). Tests (4):
  a_pc_is_named_by_the_function_that_holds_it, every_role_has_its_label,
  the_error_slot_names_the_parked_error_s_code_message_and_origin,
  an_error_code_is_named_by_its_errorcode_constant.

### 4.18 `src/debugger/values.rs` → `src/debugger/values/tests/` (13 tests)

Citations: 4, all host code.
- `mod.rs` (75): `use super::*;`, 3 `mod` lines; shared: `BASE`, `Fake` + impl, `words`,
  `string_record`, `collection`, `all`.
- `scalars_and_strings.rs` (115; 1297-1414). Tests (5):
  scalars_show_their_bits_and_their_value, money_shows_all_five_places_and_its_sign,
  a_string_literal_reads_as_a_string_outside_the_arena,
  memory_outside_the_arena_or_past_a_block_is_refused,
  a_string_is_its_length_and_its_text_escaped.
- `lists_and_sets.rs` (172; 1416-1589). Tests (4):
  a_fixed_width_list_shows_its_header_and_its_elements,
  a_list_of_strings_reads_each_from_its_entry,
  a_header_that_cannot_be_the_collection_asked_for_is_refused,
  a_set_skips_removed_elements_and_counts_them.
- `maps_resources_views.rs` (142; 1591-1734). Tests (4):
  a_map_shows_its_keys_its_values_an_entry_and_a_lookup,
  keys_are_encoded_as_their_type_stores_them,
  a_resource_shows_its_kind_flags_owner_and_fields,
  views_parse_their_types_indexes_and_ranges.

### 4.19 `src/doc/html.rs` → `src/doc/html/tests/` (16 tests)

Citations: 13, all host code.
- `mod.rs` (28): `use super::*;`, the `crate::binary_repr` import, 2 `mod` lines; shared:
  `decl`, `parse`.
- `rendering.rs` (146; 326-498). Tests (9): kind_helpers_cover_every_kind,
  anchor_deduplicates_and_slugifies,
  a_declaration_named_intro_does_not_collide_with_the_page_intro,
  escape_and_inline_handle_all_specials, split_subtitle_without_leading_desc_keeps_prose,
  from_package_no_package_uses_fallback_and_empty_render,
  from_package_full_page_renders_every_element, render_empty_html_is_valid,
  subtitle_without_intro_still_emits_intro_anchor.
- `from_source.rs` (214; 500-731). Tests (7): from_source_func_type_union_enum_and_package,
  from_source_overload_matched_by_params, from_source_internal_attribute_and_missing_target,
  from_source_package_deprecated_no_prose,
  from_source_skips_internal_files_and_second_package_block,
  source_decl_meta_returns_none_for_package_header,
  package_deprecated_empty_message_uses_default_text.

### 4.20 `src/fmt.rs` → `src/fmt/tests/` (53 tests)

Citations: 34, all host code; `src/docs/spec/architecture/09_modules.md:19` names the host.
- `mod.rs` (7): `use super::*;`, 3 `mod` lines; shared: `fmt`.
- `nesting_and_blocks.rs` (193; 928-1168). Tests (26): nesting_at_the_cap_still_formats,
  nesting_past_the_cap_is_refused_at_the_crossing_line,
  link_block_nesting_past_the_cap_is_refused, format_error_display_names_the_line_and_cap,
  reindents_nested_blocks_and_uppercases_keywords, match_case_indentation_matches_spec,
  elseif_chain_dedents, field_and_package_names_keep_case,
  single_line_if_does_not_indent_following_line, one_line_block_with_separators_is_net_zero,
  exit_and_continue_targets_do_not_open_blocks, doc_block_reindents_body_preserving_text,
  doc_example_source_is_anchored_keeping_relative_indent,
  comments_blank_lines_and_strings_are_preserved, line_continuation_keeps_following_lines,
  inline_trap_block_indents_handler, bare_trap_block_indents_handler_both_positions,
  do_while_opens_one_block_not_two, exported_declarations_open_a_block, custom_indent_width,
  nothing_is_a_type_after_as_but_a_value_elsewhere,
  function_type_annotation_does_not_open_a_block, nothing_is_a_type_after_of_and_to,
  func_alias_does_not_open_a_block,
  parameterless_func_returning_a_qualified_type_opens_a_block,
  parameterless_func_returning_a_collection_of_a_qualified_type_opens_a_block.
  Helper: nested_ifs.
- `testing_link_and_literals.rs` (196; 1170-1425). Tests (17):
  single_line_while_inside_if_is_balanced, end_while_closes_the_block_without_reopening,
  testing_blocks_indent_and_are_a_fixed_point,
  a_testing_block_does_not_disturb_the_code_after_it,
  link_cstruct_and_bind_in_blocks_indent_their_bodies,
  link_block_reindents_from_func_free_nesting_preserving_text, empty_source_stays_empty,
  blank_line_inside_block_is_emitted_empty, missing_trailing_newline_is_added,
  escaped_characters_in_strings_are_preserved,
  scalar_literals_containing_quotes_do_not_desynchronize_the_scanner,
  float_literals_are_preserved, rem_comment_at_statement_start_is_verbatim,
  walrus_assignment_operator_is_preserved, union_and_enum_open_blocks,
  sub_declaration_opens_a_block, doc_header_keeps_attribute_words.
- `doc_and_bracket_layouts.rs` (182; 1427-1635). Tests (10): doc_block_preserves_blank_lines,
  unterminated_doc_example_is_flushed, link_block_preserves_blank_lines,
  multi_line_bracket_layouts_are_preserved, multi_line_parameter_list_opens_one_block,
  leading_commas_become_trailing_commas, malformed_leading_commas_are_left_as_written,
  brackets_in_strings_scalars_and_comments_do_not_continue_a_line,
  rem_inside_a_continuation_is_an_identifier_like_the_lexer_reads_it,
  unclosed_bracket_stops_at_a_statement_keyword. Helper: fmt_fixed_point.

### 4.21 `src/lexer.rs` → `src/lexer/tests/` (35 tests)

No `src/lexer/` directory today; K creates it with only `tests/`. Citations: 14, all host
code (the two stale line cites are M's; §2). `tests/guards/no_type_strings.rs:78`
(`rel == "src/lexer.rs"`) scopes the lexer's code and is untouched by K: the moved tests sit
under `src/lexer/tests/`, which `is_test_file` exempts (`/tests/`).
- `mod.rs` (4): `use super::*;`, 3 `mod` lines.
- `docs_and_keywords.rs` (194; 1458-1677). Tests (7): doc_captures_only_at_the_start_of_a_line,
  trailing_underscore_continues_line_without_newline_token,
  underscore_identifier_remains_available_when_not_trailing,
  apostrophe_comments_skip_to_newline, rem_comments_skip_to_newline_at_statement_start,
  identifiers_containing_rem_remain_identifiers,
  keyword_lookup_and_lexeme_round_trip_for_all_keywords.
- `strings_and_numbers.rs` (181; 1679-1883). Tests (14):
  is_end_line_matches_only_two_word_end_keyword,
  is_doc_attr_rest_accepts_empty_and_attribute_words_only,
  string_escapes_are_decoded_including_unknown_escapes, carriage_return_and_nul_escapes_decode,
  unicode_scalar_escapes_decode, malformed_unicode_escapes_are_errors,
  scalar_literals_decode_to_codepoints, apostrophe_comment_swallows_backticks_inertly,
  malformed_scalar_literals_are_errors, radix_literals_decode_to_canonical_decimal,
  digit_separators_are_stripped, malformed_numbers_are_errors,
  exponent_literals_keep_their_text, malformed_exponent_lexes_as_number_then_identifier.
- `suffixes_and_brackets.rs` (179; 1885-2110). Tests (14): type_suffixes_are_kept_in_the_token,
  leading_underscore_is_an_identifier_not_a_number,
  trailing_underscore_after_digit_still_continues_line,
  unterminated_string_on_line_is_an_error, backslash_before_newline_is_an_unterminated_string,
  unterminated_string_at_eof_is_an_error, unexpected_character_is_reported_as_an_error,
  pipe_greater_is_a_single_operator_token, decimal_numbers_lex_as_a_single_number_token,
  newline_inside_open_brackets_is_not_a_separator,
  brackets_in_strings_scalars_and_comments_do_not_change_depth,
  mismatched_and_stray_closers_are_left_to_the_parser, unclosed_bracket_at_eof_is_reported,
  statement_keyword_after_unclosed_bracket_is_reported. Helpers: kinds, newline_count,
  lex_diagnostics.

### 4.22 `src/manifest/config.rs` → `src/manifest/config/tests/` (19 tests)

Citations: 8, all host code.
- `mod.rs` (26): `use super::*;`, 2 `mod` lines; shared: `value`, `BASE`, `merged`,
  `local_errors`.
- `configs.rs` (164; 677-687, 699-852, 872-883). Tests (12):
  absent_config_is_an_implicit_empty_release, valid_multi_config_builds_the_model,
  integer_and_float_edges, moved_option_is_the_only_diagnostic_for_the_old_shape,
  shape_errors, release_is_required, name_pattern_applies_to_configs_and_keys,
  duplicates_are_case_insensitive, values_must_be_string_integer_or_float,
  a_key_has_one_type_across_configs, config_keys_named_like_top_level_fields_get_their_own_line,
  a_local_override_that_keeps_the_type_replaces_the_value. Helpers: manifest, errors.
- `local_overrides.rs` (100; 885-993). Tests (7):
  a_local_override_that_changes_the_type_is_rejected_at_the_local_line,
  a_local_file_adds_keys_and_configs, the_local_root_may_only_hold_config,
  a_local_file_needs_no_release,
  local_duplicates_and_type_conflicts_are_reported_in_the_local_file,
  malformed_local_json_is_a_parse_error_in_the_local_file,
  load_project_config_reads_the_local_file_only_when_present.

### 4.23 `src/manifest/libraries.rs` → `src/manifest/libraries/tests/` (19 tests)

Citations: 5, all host code.
- `mod.rs` (10): `use super::*;`, 2 `mod` lines; shared: `manifest`.
- `parsing.rs` (185; 494-700). Tests (9): absent_section_yields_empty_map,
  parses_multi_entry_section_in_key_order, absent_type_defaults_to_vendor,
  omitted_arch_and_libc_are_wildcards, both_libc_tokens_parse,
  the_supported_matrix_is_the_registry_crossed_with_libc,
  one_wildcard_system_entry_covers_every_linux_slot,
  a_concrete_locator_covers_exactly_its_slot,
  a_wildcard_arch_locator_pinned_to_one_libc_covers_three_slots.
- `table.rs` (186; 702-913). Tests (10): explicit_system_type_parses,
  a_fully_covering_manifest_builds_a_table_with_no_findings,
  a_link_with_no_libraries_entry_is_a_missing_error,
  an_unused_libraries_entry_warns_and_is_not_encoded,
  a_macos_only_manifest_warns_once_per_uncovered_linux_slot,
  a_wildcard_linux_entry_emits_zero_uncovered_warnings,
  a_vendor_locator_hashes_its_file_from_the_vendor_directory,
  a_vendor_locator_with_no_file_is_an_unreadable_error_naming_the_path,
  vendor_path_is_flat_under_the_project_root,
  the_table_is_sorted_by_logical_name_for_deterministic_encoding. Helpers: FULL_COVERAGE,
  build.

### 4.24 `src/manifest/mod.rs` → `src/manifest/tests/` (71 of 73 tests)

The other cfg(test) module, `mod canvas_auto_paint_tests { … }` (170-234, 57 LOC, 2 tests),
stays inline (host test LOC after: 59). Citations: 38, all host code;
`.ai/build-tooling.md:84` names host code (plan-176-L repoints it).
- `mod.rs` (37): `use super::*;`, 5 `mod` lines; shared: `write_manifest`, `VALID`,
  `libraries_manifest`, `only_finding`, `assert_valid`.
- `paths_and_sources.rs` (196; 1463-1513, 1525-1723). Tests (17):
  resource_paths_may_not_escape_the_project,
  description_is_capped_and_required_only_for_packages, valid_manifest_parses,
  non_array_packages_is_rejected_by_the_build_gate, missing_file_is_error,
  unparseable_json_is_error, non_object_root_is_error, missing_required_field_is_error,
  wrong_type_field_is_error, empty_string_field_is_error, optional_field_wrong_type_is_error,
  sources_missing_is_error, sources_wrong_type_is_error, sources_empty_is_error,
  source_entry_not_object_is_error, source_pattern_wrong_type_is_error,
  source_pattern_non_string_element_is_error.
- `kind_and_mode.rs` (172; 1725-1922). Tests (21): kind_missing_is_error,
  kind_wrong_type_is_error, unknown_kind_still_validates_ok, mode_app_validates_and_is_detected,
  mode_console_is_not_app, absent_mode_is_not_app, unknown_mode_still_validates_ok,
  non_string_mode_is_error, non_string_icon_is_error, icon_path_accessor_reads_field,
  resources_valid_entry_is_accepted, resources_absent_is_valid_and_empty,
  resources_non_array_is_rejected, resources_non_object_entry_is_rejected,
  resources_missing_or_empty_src_is_rejected, resources_missing_or_nonstring_dst_is_rejected,
  resources_escaping_dst_is_rejected, entry_point_defaults_to_main,
  packages_array_wrong_type_is_error, parse_project_json_rejects_non_object,
  field_position_finds_and_falls_back. Helper: manifest_with_resources.
- `libraries.rs` (194; 1959-2172). Tests (16): absent_libraries_section_is_valid,
  non_object_libraries_is_field_type_error, non_array_library_value_is_field_type_error,
  empty_locator_array_is_empty_field_error, non_object_locator_is_field_type_error,
  missing_os_names_the_field_and_accepted_set, unknown_os_token_names_the_token_and_accepted_set,
  unknown_arch_token_names_the_token_and_the_wildcard_option,
  unknown_libc_token_names_the_token_and_the_wildcard_option,
  unknown_type_token_explains_the_vendor_default,
  macos_locator_carrying_libc_is_rejected_with_the_reason, blank_source_is_rejected,
  missing_source_is_rejected, source_with_a_path_separator_names_the_offending_character,
  source_of_dot_dot_is_rejected, source_with_an_interior_nul_is_rejected.
- `locators_and_appends.rs` (193; 2174-2395). Tests (13):
  source_with_a_windows_drive_prefix_is_rejected,
  linux_vendor_locator_missing_arch_is_rejected_with_the_reason,
  linux_vendor_locator_missing_libc_is_rejected_with_the_reason,
  linux_vendor_locator_missing_both_axes_reports_both,
  two_vendor_locators_sharing_a_source_conflict, two_system_locators_may_share_a_soname,
  linux_system_locator_may_omit_both_axes, macos_vendor_locator_may_omit_arch,
  the_representative_manifest_validates, a_well_formed_section_passes_full_manifest_validation,
  a_malformed_section_fails_full_manifest_validation,
  package_add_manifest_insert_creates_packages_array,
  package_add_manifest_append_preserves_json_array_format.
- `entry_and_project.rs` (160; 2397-2577). Tests (4):
  validate_entry_point_rejects_multiple_matching_declarations,
  validate_project_manifest_rejects_missing_kind,
  validate_project_manifest_rejects_a_name_that_is_not_a_path_component,
  validate_project_manifest_checks_max_buffer. Helper: test_temp_dir.

`src/manifest/` already holds `libraries.rs` (code); the test topic `tests/libraries.rs` is a
different path (`manifest::tests::libraries`), no collision.

### 4.25 `src/manifest/package.rs` → `src/manifest/package/tests/` (82 tests)

Citations: 24, all host code. `tests/guards/no_type_strings.rs:215,1763` and
`src/docs/spec/package/01_container-format.md:210` name the host path (unchanged).
- `mod.rs` (71): `use super::*;`, 6 `mod` lines; shared: `push_field`, `build_mfp`,
  `header_err`, `write_temp`, `json`, `dependency`, `fixture_mfp`.
- `metadata_readers.rs` (159; 1073-1296). Tests (11):
  package_metadata_readers_handle_a_malformed_package_by_contract,
  the_two_mfp_decoders_still_disagree_about_an_empty_ident_by_design,
  only_the_manifest_reader_applies_the_name_charset_guard,
  read_mfp_header_parses_valid_unsigned_package,
  read_mfp_header_rejects_a_name_that_is_not_a_path_component,
  package_dependencies_drops_blank_and_traversing_names, read_mfp_header_rejects_missing_file,
  read_mfp_header_rejects_too_small, read_mfp_header_rejects_bad_magic,
  read_mfp_header_rejects_bad_container_version, read_mfp_header_rejects_empty_required_name.
  Helper: build_mfp_for_both_decoders.
- `mfp_header.rs` (198; 1298-1456, 1464-1541). Tests (17):
  read_mfp_header_rejects_signature_length_mismatch,
  read_mfp_header_rejects_unknown_signature_type,
  read_mfp_header_rejects_trailing_length_mismatch,
  read_mfp_header_rejects_a_length_that_cannot_address_the_body,
  read_mfp_header_rejects_truncated_field, read_mfp_header_rejects_oversized_field,
  package_file_url_path_validates_scheme_and_extension,
  package_file_url_path_validates_windows_paths,
  package_file_url_path_accepts_existing_file_and_decodes_percent,
  percent_decode_rejects_malformed_escapes, hex_value_covers_all_cases,
  project_package_dependency_reads_all_fields,
  project_package_dependency_defaults_and_ident_key_alias,
  project_package_dependency_rejects_non_object_and_blank_name,
  project_package_dependency_rejects_path_traversal_name,
  package_metadata_and_dependencies_from_manifest,
  package_metadata_defaults_when_optional_fields_absent.
- `installed_and_exports.rs` (173; 1543-1746). Tests (8):
  installed_package_files_empty_when_no_packages,
  installed_package_files_reports_missing_and_pin_mismatch,
  installed_package_files_ok_when_present_and_unpinned,
  installed_package_files_resolves_the_source_package_cache,
  resolved_package_file_prefers_the_installed_mfp,
  source_dependency_classifies_every_source_spelling,
  external_package_function_types_lossy_swallows_errors,
  package_export_signature_decodes_and_renders_signature.
- `export_signatures_and_insert.rs` (182; 1748-1823, 1840-1976). Tests (11):
  package_export_signature_round_trips_every_export_shape,
  project_json_with_package_inserts_new_array,
  project_json_with_package_appends_to_existing_array,
  project_json_with_package_appends_to_empty_array, project_json_with_package_rejects_duplicate,
  project_json_without_packages_removes_at_every_position,
  project_json_without_packages_can_empty_the_array,
  project_json_without_packages_removes_the_only_entry,
  project_json_without_packages_removes_several_at_once,
  project_json_without_packages_preserves_survivor_formatting,
  project_json_without_packages_errors_when_none_match. Helper: three_package_manifest.
- `version_and_pin_updates.rs` (196; 1978-2137, 2148-2236). Tests (20):
  project_json_with_updated_version_preserves_pin_by_default,
  project_json_with_updated_version_rewrites_pin_when_asked,
  project_json_with_updated_version_appends_a_missing_pin,
  project_json_with_updated_version_leaves_siblings_and_formatting_alone,
  project_json_with_updated_version_matches_a_bare_name_entry,
  project_json_with_updated_version_errors_when_absent,
  project_json_with_updated_ident_key_rewrites_existing,
  project_json_with_updated_ident_key_inserts_when_absent,
  project_json_with_updated_ident_key_errors, insert_helpers_report_missing_structures,
  json_scanning_primitives, read_int_helpers_reject_truncation,
  read_mfp_header_reads_real_package_fixture,
  external_package_function_types_from_files_reads_exports,
  external_package_function_types_lossy_matches_strict_for_valid_files,
  external_package_function_types_lossy_skips_bad_files,
  external_package_function_types_from_files_propagates_error,
  read_mfp_header_rejects_ed25519_wrong_signature_length,
  read_mfp_header_rejects_truncated_binary_hash,
  project_json_with_updated_ident_key_rewrites_snake_case_field.
- `ident_key_and_dependencies.rs` (207; 2238-2493). Tests (15):
  project_json_with_updated_ident_key_rejects_malformed_entry,
  read_mfp_string_rejects_invalid_utf8, imported_type_def_maps_a_record_export,
  imported_type_def_maps_a_union_export_with_variant_fields,
  imported_type_def_maps_an_enum_export, imported_type_def_drops_function_exports,
  imported_type_defs_from_files_reads_a_record_export_fixture,
  imported_type_defs_from_files_skips_unreadable_packages,
  imported_type_defs_empty_when_no_packages, imported_type_defs_reads_from_an_installed_package,
  imported_resource_closers_skips_builtin_backed_resources,
  imported_resource_closers_registers_a_native_resource,
  imported_resource_closers_empty_when_packages_unreadable,
  common_dependency_fields_rejects_a_non_string_name,
  external_package_function_types_lossy_skips_a_corrupt_body. Helpers: type_field,
  type_export, owned.

### 4.26 `src/numeric.rs` → `src/numeric/tests/` (33 tests)

The 8 other cfg(test) items (`TYPE_*` consts 12-26, `binary_result_type` 542-551,
`numeric_variant` 625-635, `numeric_variant_name` 639-649; 42 LOC) stay in the host.
Citations: 11, all host code. Path refs `tests/guards/no_operator_strings.rs:78`,
`.ai/codegen-invariants.md:7`, `src/docs/spec/architecture/09_modules.md:69`,
`src/docs/spec/architecture/22_type-inference.md:327` name the host (unchanged).
- `mod.rs` (11): `use super::*;`, 4 `mod` lines; shared: `legacy_is_numeric_type`.
- `scientific_and_constants.rs` (188; 720-949). Tests (8):
  expand_scientific_notation_bounds_extreme_exponents,
  default_to_string_text_matches_the_runtime_default,
  fixed_constant_from_float_literal_converts_the_binary64_value,
  fixed_decimal_from_raw_round_trips_through_the_parser,
  money_decimal_from_raw_round_trips_through_the_parser,
  fixed_raw_from_decimal_rejects_extreme_exponents_in_o1,
  fixed_raw_from_decimal_rounds_sub_ulp_literals_to_zero,
  classify_literal_types_integers_and_floats.
- `literal_classification.rs` (195; 951-1191). Tests (13):
  classify_literal_handles_exponent_and_suffixes,
  expand_scientific_notation_returns_input_on_unparseable_exponent,
  fixed_raw_from_decimal_rejects_malformed_values,
  fixed_raw_from_decimal_carries_rounding_into_the_whole_part,
  fixed_raw_from_decimal_rounds_exactly_at_boundaries,
  money_conversion_rounds_extreme_negative_exponents_to_zero,
  fixed_raw_from_decimal_rejects_out_of_range_magnitudes,
  expand_scientific_notation_shifts_the_point, non_numeric_operand_has_no_result_type,
  div_always_yields_float, fixed_dominates_all_other_numerics,
  float_dominates_integer_and_byte, byte_pair_stays_byte_but_mixed_widens_to_integer.
- `money.rs` (193; 1193-1410). Tests (7): is_numeric_type_accepts_only_the_five_numerics,
  classify_literal_types_money_suffix, money_raw_from_decimal_scales_and_rounds,
  money_raw_from_decimal_covers_the_full_i64_range,
  money_raw_from_decimal_rejects_malformed_values, money_conversion_reports_lost_precision,
  money_dimensional_lattice_table.
- `typed_lattice.rs` (211; 1430-1666). Tests (5):
  typed_binary_result_type_matches_the_legacy_table_exhaustively,
  typed_promote_loop_matches_the_legacy_fold_exhaustively,
  typed_money_result_type_matches_the_legacy_table,
  is_numeric_agrees_with_the_name_keyed_predicate,
  non_money_lattice_is_unchanged_by_money_rules. Helpers: legacy_binary_result_type,
  legacy_money_result_type, legacy_promote_loop_numeric_type_name, LATTICE_NAMES,
  lattice_types, OPERATORS.

### 4.27 `src/optimizer/opt1/globals.rs` → `src/optimizer/opt1/globals/tests/` (12 tests)

Citations: 1, host code.
- `mod.rs` (55): `use super::*;`, 2 `mod` lines; shared: `global`, `function`, `read`, `run`,
  `fold`, `returned`, `typed_const`.
- `constant_globals.rs` (145; 416-584). Tests (6): constant_globals_inline_and_then_die,
  written_globals_are_untouched, exported_globals_are_never_touched,
  non_literal_initializers_are_read_only_but_not_inlined,
  initializer_reads_keep_a_global_alive, reads_that_can_run_during_initialization_are_kept.
- `literal_lets.rs` (133; 604-752). Tests (6):
  literal_lets_fold_at_level_one_regardless_of_visibility, only_literal_word_scalar_lets_fold,
  a_stored_let_is_not_folded,
  literal_let_folding_keeps_reads_that_can_run_during_initialization,
  literal_let_folding_is_off_at_level_zero, level_one_disables_the_rows.

### 4.28 `src/optimizer/opt1/inline.rs` → `src/optimizer/opt1/inline/tests/` (7 tests)

Citations 0.
- `mod.rs` (100): `use super::*;`, 2 `mod` lines; shared NIR builders `loc`, `local`,
  `binary`, `call`, `function`, `gi`, `caller`, `run`, `returned`.
- `leaf_calls.rs` (87; 464-561). Tests (4): a_leaf_call_becomes_its_expression,
  a_cross_file_inline_tags_the_helpers_file, ineligible_calls_are_left_alone,
  a_call_through_a_same_named_local_is_left_alone.
- `declined_calls.rs` (84; 563-653). Tests (3): non_leaf_callees_are_left_alone,
  calls_under_an_error_capture_are_left_alone, the_row_is_off_at_level_zero.

### 4.29 `src/optimizer/opt2/promote.rs` → `src/optimizer/opt2/promote/tests/` (10 tests)

The cfg(test) `visited` field (228-232) and its initializer (259-269) stay in the host.
Citations: 1, host code. The moved code carries `"%vN"` literals and `.physical_field(`
(§2): it relies on plan-176-H's `/tests/` guard rule (Prerequisites).
- `mod.rs` (95): `use super::*;`, 2 `mod` lines; shared: `ci`, `store`, `load`, `run`,
  `read_before_written_dense`.
- `promotion.rs` (108; 543-674). Tests (7): a_private_slot_becomes_a_register,
  a_zero_store_becomes_an_immediate, a_slot_read_before_written_stays_in_memory,
  a_slot_handed_to_a_drop_helper_is_pinned, any_other_frame_address_disables_the_function,
  an_access_outside_every_object_disables_the_function, pinned_and_wide_objects_stay_in_memory.
  Helper: ops.
- `search.rs` (84; 733-833). Tests (3): the_per_slot_search_agrees_with_the_dense_dataflow,
  the_search_visits_only_the_blocks_a_slot_is_live_into, the_row_is_off_at_level_zero.
  Helper: word_accesses.

### 4.30 `src/optimizer/opt2/sink.rs` → `src/optimizer/opt2/sink/tests/` (12 tests)

Citations 0. Same guard dependency as 4.29 (`.physical_field(` at old :526).
- `mod.rs` (18): `use super::*;`, 2 `mod` lines; shared: `ci`, `ops`, `run`.
- `sinking.rs` (183; 540-742). Tests (8): a_one_sided_computation_sinks_into_its_branch,
  a_two_sided_computation_stays, a_shared_successor_is_not_a_sink_target,
  a_load_does_not_sink_past_a_call, a_load_sinks_when_the_path_is_memory_quiet,
  a_multiply_defined_destination_stays, an_identical_leading_load_is_hoisted,
  the_hoist_lands_before_the_compare.
- `hoisting.rs` (100; 744-849). Tests (4): a_different_slot_is_not_hoisted,
  a_shared_arm_is_not_hoisted_from, an_identical_leading_store_is_hoisted,
  level_two_disables_the_rows.

### 4.31 `src/resolver/mod.rs` → `src/resolver/tests/` (28 tests)

The cfg(test) `pub fn resolve_hir_project` (114-127) and `augment_hir_project` (222-232) stay in
the host. Citations: 13, all host code. Path refs `tests/guards/no_type_strings.rs:102,104,487`,
`.ai/resources-packages.md:246,247,375` name host code (unchanged).
`is_c_abi_type_recognizes_and_rejects` is named by fn name only (`.ai/resources-packages.md:322`,
a comment at `src/resolver/mod.rs:282`) — stable.
- `mod.rs` (103): `use super::*;`, 3 `mod` lines; shared: `quiet`, `resolve_fixture`, `param`,
  `hir_param`, `func`, `hir_file`, `project_of`, `binding`, `binding_at`, `func_at`.
- `arguments_and_overloads.rs` (110; 830-832, 901-1023). Tests (7):
  constructor_arg_value_positional_and_named, call_arg_value_positional_and_named,
  overload_types_match_variants, overload_types_match_none_type_name,
  is_c_abi_type_recognizes_and_rejects, resource_base_type_strips_state_suffix,
  visible_from_rules. Helper: empty_expr.
- `lookups_and_fixtures.rs` (189; 1025-1221). Tests (10):
  top_level_and_function_visibility_lookups, link_target_signature_lookup,
  resolve_valid_fixtures_succeed, validate_project_docs_true_for_valid_docs,
  validate_project_docs_false_for_invalid_docs, resolve_invalid_fixtures_fail,
  resolve_project_with_no_doc_validation, duplicate_top_level_function_reports,
  reserved_builtin_name_rejected, type_and_resource_names_registered.
- `name_collisions.rs` (223; 1241-1437, 1447-1502). Tests (11):
  function_name_collides_with_prior_binding_reports,
  binding_name_collides_with_prior_function_reports,
  alias_function_name_collides_with_prior_binding_reports,
  a_local_named_like_an_own_file_private_binding_is_refused,
  a_local_named_like_another_files_private_binding_is_allowed,
  a_local_named_like_a_public_binding_of_another_file_is_refused,
  a_local_named_like_a_function_is_allowed,
  an_internal_name_or_a_built_in_source_file_is_exempt,
  alias_function_registers_when_unique,
  a_private_and_a_public_function_of_one_signature_in_one_file_are_duplicates,
  a_private_overload_or_another_files_private_is_not_a_duplicate. Helpers: binding_with,
  new_local_is_refused, private_secret.

### 4.32 `src/resolver/packages.rs` → `src/resolver/packages/tests/` (20 tests)

Citations: 3, all host code.
- `mod.rs` (40): `use super::*;`, 2 `mod` lines; shared: `empty_file`,
  `manifest_with_package`, `resolve_import`.
- `dependencies_and_build_cache.rs` (155; 339-540). Tests (13):
  dependency_packages_parses_name_and_optional_source,
  dependency_packages_empty_when_no_packages_key,
  dependency_packages_skips_entries_without_name, read_manifest_ok_and_error_paths,
  qualify_package_name_both_branches, builtin_import_short_circuits,
  self_is_an_ordinary_package_name, undeclared_package_is_reported,
  declared_but_not_installed_is_reported, present_mfp_that_is_garbage_is_reported,
  present_valid_mfp_installs_type_names, source_package_dir_valid_manifest_passes,
  source_package_dir_installs_type_names_from_the_build_cache. Helper: manifest_with_kind.
- `source_manifests.rs` (86; 542-634). Tests (7):
  source_package_dir_bad_manifest_reports_before_loading_exports,
  source_package_dir_unreadable_manifest_is_reported, source_package_dir_name_mismatch_is_reported,
  source_package_dir_wrong_kind_is_reported, local_source_relative_path_is_reported,
  local_source_missing_manifest_is_reported, local_source_valid_manifest_passes.

### 4.33 `src/resolver/resolution.rs` → `src/resolver/resolution/tests/` (57 tests)

Citations: 27, all host code.
- `mod.rs` (34): `use super::*;`, 4 `mod` lines; shared: `quiet`, `resolve_source_fails`,
  `assert_source_ok`.
- `fixtures.rs` (178; 1808-2008). Tests (14):
  crypto_removed_key_generators_are_not_user_callable,
  crypto_removed_hash_and_signature_members_are_not_user_callable,
  broad_valid_fixtures_resolve, unknown_identifier_reports, unknown_type_reports,
  result_type_not_user_visible_reports, duplicate_local_reports, duplicate_parameter_reports,
  unknown_import_qualified_use_reports, duplicate_import_binding_reports,
  unknown_type_field_reports, builtin_member_unknown_reports, callable_not_top_level_reports,
  resolution_error_fixtures_fail. Helper: resolve_fixture_fails.
- `statements_and_declarations.rs` (188; 2021-2225). Tests (12):
  statements_return_exit_fail_recover_continue, statements_assign_and_propagate,
  nested_blocks_if_while_dountil, foreach_with_list_and_map_types,
  duplicate_local_in_for_loop_reports, duplicate_local_in_foreach_reports,
  match_with_union_guard_and_oneof, expressions_binary_unary_lambda_constructor,
  trapped_expression_binding_in_scope, type_name_function_type_and_nested_generics,
  malformed_function_type_reports, union_and_enum_declarations_resolve.
- `link_blocks_and_resources.rs` (200; 2229-2460). Tests (18):
  link_block_and_resource_and_alias_resolve, resource_close_unknown_alias_reports,
  resource_close_missing_func_in_link_reports, func_alias_unknown_target_reports,
  link_member_unknown_reports, doc_block_valid_variants_resolve,
  doc_overload_disambiguated_by_types, doc_ret_on_non_callable_reports,
  doc_type_name_mismatch_reports, doc_duplicate_internal_attr_reports,
  doc_arg_and_error_invalid_context_reports, doc_duplicate_ret_example_deprecated_group_reports,
  doc_prop_invalid_context_and_internal_on_package_reports, doc_type_kind_mismatch_reports,
  doc_type_unresolved_reports, doc_duplicate_block_for_same_target_reports,
  resource_close_not_dotted_reports, link_block_unknown_param_type_reports.
- `bindings_unions_docs.rs` (142; 2464-2638). Tests (13):
  top_level_binding_with_type_and_value_resolves, union_with_includes_resolves,
  union_duplicate_variant_reports, enum_duplicate_member_reports,
  state_assign_statement_resolves, thread_and_mapentry_type_names_resolve,
  import_alias_conflicts_with_builtin_reports, import_alias_conflicts_with_top_level_reports,
  duplicate_type_field_reports, propagate_statement_resolves, isolated_func_type_name_resolves,
  doc_func_names_a_type_reports, doc_duplicate_overload_block_reports.

### 4.34 `src/rules/mod.rs` → `src/rules/tests/` (20 tests)

The other cfg(test) items (`thread_local!` 22-27, `collecting_diagnostics` 152-160, a block at
347-357, `code_and_name` 420-424; 30 LOC) stay in the host. `src/rules/` already holds
`table.rs`; no collision. Citations: 27, all host code. `scripts/diag-set-diff.sh:67` names the
host (unchanged). `include_str!(concat!(env!("CARGO_MANIFEST_DIR"), …))` (old :560) is absolute.
- `mod.rs` (3): `use super::*;`, 2 `mod` lines.
- `rule_table.rs` (196; 452-720). Tests (15): import_package_invalid_is_a_defined_rule,
  dead_import_missing_package_name_is_gone, severity_displays_all_three_levels,
  is_error_reflects_rule_severity, show_diagnostic_renders_source_context_and_underline,
  unknown_rule_name_trips_the_debug_assert, rule_names_are_unique, rule_codes_are_unique,
  every_rule_is_documented_in_the_spec, show_diagnostic_handles_empty_and_missing_source_files,
  show_diagnostic_skips_underline_when_position_precedes_the_reported_line,
  show_general_diagnostic_renders_header_and_detail,
  show_diagnostic_rewrites_mangled_generic_names_to_template_spellings,
  code_and_name_resolves_representative_rules, is_error_partitions_the_whole_table_by_severity.
- `cached_source.rs` (55; 727-791). Tests (5):
  cached_source_reads_a_file_once_and_indexes_its_lines, cached_source_notices_a_rewritten_file,
  cached_source_is_none_for_a_missing_file,
  safe_source_line_escapes_controls_and_bidi_but_keeps_tabs,
  suppressed_count_is_seen_minus_the_cap.

Sweep: `src/docs/spec/diagnostics/01_rule-codes.md:103` and
`bugs/bug-883-rule-codes-spec-drift-and-unemitted-rules.md:43` name
`rules::tests::rule_codes_are_unique` → `rules::tests::rule_table::rule_codes_are_unique`.

### 4.35 `src/types.rs` → `src/types/tests/` (27 tests)

The cfg(test) `pub(crate) const ALL` (115-133, 19 LOC) stays in the host. Citations: 22, all
host code. Path refs in `tests/guards/no_type_strings.rs` (lines 3, 43, 180, 218, 230, 572, 808,
812, 832, 935, 1718, 1721, 1724, 1751 at `17a9b5799`; plan-176-E moved some into
`tests/guards/no_type_strings/*.rs`), `.ai/resources-packages.md:295`,
`.ai/codegen-invariants.md:20,58,736,804` name the host or its code (unchanged).
`no_type_strings` lists `src/types.rs` as a BOUNDARY file (`rel == "src/types.rs"`); the host
code stays there and the new `src/types/tests/*` paths contain `/tests/`, so `is_test_file`
exempts them. `split_state_matches_the_name_domain_helpers` is named by fn name only
(`no_type_strings.rs:927`) — stable. The golden-corpus scan (old :1695) joins
`env!("CARGO_MANIFEST_DIR")`.
- `mod.rs` (14): `use super::*;`, 6 `mod` lines; shared: `round_trip`.
- `c_abi_and_names.rs` (170; 1562-1771). Tests (7): a_stateful_thread_view_keeps_its_side,
  c_abi_types_round_trip_through_parameter_type, a_cstruct_name_stays_a_nominal,
  near_miss_spellings_are_not_c_abi_types,
  with_vars_reclassifies_a_c_abi_spelling_used_as_a_template_param,
  every_golden_corpus_ctype_round_trips, is_builtin_named_accepts_both_spellings.
  Helper: collect_ctypes.
- `variants_and_round_trips.rs` (173; 1784-1996). Tests (6):
  a_grouped_type_name_decomposes_at_every_level, thread_handle_parses_into_variant,
  thread_handle_round_trips_byte_exact, isolated_func_round_trips_and_decomposes,
  map_entry_and_result_parse_into_variants_and_round_trip,
  res_collection_element_parses_into_variant.
- `state_split.rs` (150; 1998-2187). Tests (4):
  with_state_matches_parse_of_the_concatenated_spelling,
  split_state_is_the_inverse_of_with_state_on_leaves, split_state_is_top_level_only,
  split_state_matches_the_name_domain_helpers.
- `hash_and_stateful.rs` (186; 2189-2419). Tests (2): parameter_type_hash_agrees_with_eq,
  stateful_parses_top_level_only.
- `structural_state.rs` (163; 2421-2629). Tests (5):
  structural_split_state_agrees_with_the_string_grammar, stateful_round_trips_byte_exact,
  contains_state_agrees_with_the_rendered_name, res_collection_round_trips_byte_exact,
  res_collection_on_thread_plane_parses_structurally.
- `prefix_and_display.rs` (145; 2631-2803). Tests (3):
  type_prefix_len_measures_every_parse_constructor,
  thread_message_plane_measures_every_parse_constructor,
  display_spells_package_qualifiers_as_source_does.

### 4.36 `src/testutil.rs` → `src/testutil/` (3 tests; whole file is test code)

`src/main.rs` keeps `#[cfg(test)] mod testutil;` (it resolves to `src/testutil/mod.rs`).
Measured users of the surface (`grep -rhoE "testutil::(\{[^}]*\}|\w+)" src`): `CodeTarget` (50),
`fixture_dir` (17), `silence_panics` (16), `project_from_src` (16), `nir_for_src` (15),
`code_for_src_cached` (13), `check_src` (13), `code_function` (10), `try_code_for_src` (8),
`named_ir_for_src` (6), `parse_file` (5), `fixture_src` (5), `code_for_src_on` (5),
`try_code_for_fixture_project` (4), `app_code_cached` (4), `try_code_for_linking_src` (3),
`native_plan_for_src` (3), `lower_src`, `code_for_src_with_input_buffer`, `code_for_src_at`,
`code_for_nir`, `code_for_linking_src`, `check_src_details`, `accepts` (2 each),
`fixture_project`, `code_for_src_mode`, `code_for_fixture`, `check_src_with_imports`,
`check_fixture_project` (1 each). (`…::local_rewrites::testutil::…` is a different module.)

Layout (`python3 -I /tmp/loc/span.py src/testutil.rs <a> <b>`):
- `mod.rs` (74 = 5 + 55 + 14 declaration lines; source 1-15, 1210-1283): the `//!` header,
  `#![cfg(test)]`, the 4 `use` lines, `mod panics; mod fixtures; mod frontend; mod codegen;
  mod native; mod cache; mod fixture_projects;`, `pub use panics::*; pub use fixtures::*;
  pub use frontend::*; pub use codegen::*; pub use native::*; pub use cache::*;
  pub use fixture_projects::*;`, and `#[cfg(test)] mod code_harness_tests` (3 tests,
  unchanged).
- `panics.rs` (26; 16-71): `silence_panics` + its hook statics and thread-local.
- `fixtures.rs` (73; 72-160, 900-928): `fixture_dir`, `fixture_src`, `code_for_fixture`.
- `frontend.rs` (107; 161-368): `parse_file`, `project_from_src`, `lower_src`,
  `concrete_hir_from_src`, `lower_src_concrete`, `main_entry`, `named_ir_for_src`,
  `check_src_with_imports`, `check_src_details`, `check_src`, `accepts`.
- `codegen.rs` (192; 369-650): `CodeTarget` + impl, `code_for_src_on`, `code_for_src_mode`,
  `try_code_for_src`, `try_code_for_linking_src`, `try_code_for_src_with`, `code_for_src_at`,
  `code_for_src_with`, `code_for_src_with_input_buffer`, `explain_lowering_failure`,
  `code_for_src_inner`.
- `native.rs` (134; 651-840): `nir_for_src`, `code_for_nir`, `native_plan_for_src`,
  `lower_ir_to_code`.
- `cache.rs` (90; 841-899, 929-990): `code_for_src_cached`, `app_code_cached`,
  `code_for_linking_src`, `link_library_table`, `code_function`.
- `fixture_projects.rs` (152; 991-1209): `FixtureProject`, `build_fixture_source_packages`,
  `fixture_project`, `check_fixture_project`, `try_code_for_fixture_project`.

Each part starts with `use super::*;` (the shared `use` lines and every sibling's items through
the globs). **Visibility edits (3):** `main_entry` (frontend → used by codegen, native),
`lower_ir_to_code` (native → used by codegen, fixture_projects), `link_library_table`
(cache → used by codegen) become `pub(super)`; siblings reach them through the `pub use part::*`
globs (a glob re-exports each item at its own visibility). Every `pub` item keeps
`crate::testutil::X` through the globs; no caller changes.

**Guard edits (same commit).** Each keeps the guard's protected behavior and exempts no
production file; every file under `src/testutil/` is compiled only under `#[cfg(test)]`
(`#![cfg(test)]` in `mod.rs`, `#[cfg(test)] mod testutil;` in `src/main.rs`).
1. `tests/guards/architecture_guards/seams_goldens_and_panic_hook.rs` (where plan-176-E put
   `PANIC_HOOK_OWNER`): `const PANIC_HOOK_OWNER: &str = "testutil/panics.rs";` and the skip
   compares the path relative to `src/` (`path.strip_prefix(&src_root)`, rendered with `/`)
   against it instead of the file name.
   (1) `git log -S PANIC_HOOK_OWNER` → `d763efc77` (2026-09-08, "testutil: one panic hook for
   the process, silenced per THREAD"). (2) Protects: only the testutil hook installer may call
   `panic::set_hook`/`take_hook`. (3) Dependents: none
   (`grep -rn PANIC_HOOK_OWNER tests src` → the guard only). (4) Proof: the guard's own intent
   is "the testutil hook installer is the one owner"; the moved code is the same code at
   `src/testutil/panics.rs`. The new check is narrower than the old (one path instead of any
   file named `testutil.rs`).
2. `tests/guards/no_type_strings.rs` `is_test_file`: replace `name == "testutil.rs"` with
   `rel.starts_with("src/testutil/")`; the doc comment's `!src/testutil.rs` becomes
   `!src/testutil/`.
3. `tests/guards/no_operator_strings.rs` `is_test_file`: the same replacement.
   For 2 and 3: (1) `git log -S 'name == "testutil.rs"'` → `9174857b8` (2026-08-29, plan-111-A
   Phase 1, no_type_strings) and `875189218` (2026-09-01, plan-112 Phase 5,
   no_operator_strings). (2) Protects: no type/operator spelling decides anything outside the
   exempt string domain and test code. (3) Dependents: the two guards only. (4) Proof: the
   clause's intent is "the shared test harness is test code"; the harness is the same code at
   `src/testutil/`. After K no file named `testutil.rs` exists (`find src -name testutil.rs`
   → none), so the old clause would exempt nothing and the parts would be scanned as
   production code.

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

Every phase uses the same gate, run in this order:

- **G0 (before the first edit):** `cargo test -p mfb --bin mfb -- --list 2>/dev/null | grep -c ': test$' > /tmp/k_before.txt`
  and `cargo check --all-targets 2>&1 | grep -c '^warning' > /tmp/k_warn_before.txt` (est. 5 min
  cold, 1 min warm).
- **G1:** the same two commands after the phase's edits → numbers equal the G0 files.
- **G2:** run the moved modules' tests (filter per phase) → all pass, 0 failed.
- **G3:** `cargo test --test architecture_guards --test no_type_strings --test no_operator_strings`
  → all pass (est. 2 min).
- **G4:** `cargo test --test file_size_budget 2>&1 | grep -E '^  (NEW|GREW)'` → no output, and
  every `TIGHTEN`/`STALE` row names a file of this phase; then
  `MFB_LOC_TIGHTEN=1 cargo test --test file_size_budget` and rerun →
  `grep -cE '^  (NEW|GREW|STALE|TIGHTEN)'` → `0`. Commit `tests/guards/file_size_ceilings.txt`
  with the phase (est. 2 min).
- **G5:** `cargo fmt --check` → no output (est. 20 s).

### Phase 1 — Optimizer and audit tests

Nine hosts with self-contained NIR / audit fixtures; nothing outside them imports their tests.

- [ ] Split `src/optimizer/opt1/placefwd/tests/mod.rs` per §4.4 (`render_ops` → `pub(super)`).
- [ ] Split `src/optimizer/opt1/globals.rs` per §4.27.
- [ ] Split `src/optimizer/opt1/inline.rs` per §4.28.
- [ ] Split `src/optimizer/opt2/promote.rs` per §4.29.
- [ ] Split `src/optimizer/opt2/sink.rs` per §4.30.
- [ ] Split `src/audit/collect/findings.rs` per §4.7.
- [ ] Split `src/audit/collect/project.rs` per §4.8.
- [ ] Split `src/audit/collect/source.rs` per §4.9.
- [ ] Split `src/audit/report.rs` per §4.10 (`testsupport/` + `tests/mod.rs`).
- [ ] G4: tighten the ceilings for the nine files.

Acceptance: the nine files' tests run from their new modules and the counts did not move.
  Check: G1 → equal; G2 `cargo test -p mfb --bin mfb optimizer:: audit::` → 0 failed, and
  `cargo test -p mfb --bin mfb -- --list 2>/dev/null | grep -cE '^(optimizer::opt1::(placefwd|globals|inline)|optimizer::opt2::(promote|sink)|audit::(collect::(findings|project|source)|report))::tests::'`
  → 41+12+7+10+12+10+10+26+4 = 132; G3 pass (the promote/sink topics are exempt by the H
  rule); G4; G5 (est. 12 min).
Commit: —

### Phase 2 — Frontend: AST, lexer, fmt, numeric, types, rules, resolver, doc

- [ ] Split `src/ast/tests.rs` per §4.1 (delete the file; create `src/ast/tests/`).
- [ ] Split `src/ast/scope_privates.rs` per §4.5.
- [ ] Split `src/lexer.rs` per §4.21.
- [ ] Split `src/fmt.rs` per §4.20.
- [ ] Split `src/numeric.rs` per §4.26.
- [ ] Split `src/types.rs` per §4.35.
- [ ] Split `src/rules/mod.rs` per §4.34.
- [ ] Split `src/resolver/mod.rs` per §4.31.
- [ ] Split `src/resolver/packages.rs` per §4.32.
- [ ] Split `src/resolver/resolution.rs` per §4.33.
- [ ] Split `src/doc/html.rs` per §4.19.
- [ ] `src/docs/spec/diagnostics/01_rule-codes.md:103` and
      `bugs/bug-883-rule-codes-spec-drift-and-unemitted-rules.md:43`:
      `rules::tests::rule_codes_are_unique` → `rules::tests::rule_table::rule_codes_are_unique`
      (`grep -rn 'rules::tests::rule_codes_are_unique' src/docs bugs/*.md` → 0 after).
- [ ] G4: tighten (the `src/ast/tests.rs` row is STALE, the other ten TIGHTEN).

Acceptance: the frontend tests run from their new modules; the spec still resolves.
  Check: G1 → equal; G2 `cargo test -p mfb --bin mfb ast:: lexer:: fmt:: numeric:: types:: rules:: resolver:: doc::html`
  → 0 failed; `cargo test -p mfb --bin mfb spec_citations_resolve` → passes; G3; G4; G5
  (est. 12 min).
Commit: —

### Phase 3 — Manifest and CLI

- [ ] Split `src/manifest/config.rs` per §4.22.
- [ ] Split `src/manifest/libraries.rs` per §4.23.
- [ ] Split `src/manifest/mod.rs` per §4.24 (`canvas_auto_paint_tests` stays inline).
- [ ] Split `src/manifest/package.rs` per §4.25.
- [ ] Split `src/cli/build/mod.rs` per §4.6.
- [ ] Split `src/cli/man.rs` per §4.11.
- [ ] Split `src/cli/pkg.rs` per §4.12 (the `#[path = "pkg/command_tests.rs"]` declaration
      stays as is).
- [ ] Split `src/cli/resolve.rs` per §4.13.
- [ ] Split `src/cli/dbg/tests.rs` per §4.2 (delete the file; create `src/cli/dbg/tests/`).
- [ ] Split `src/cli/info/tests.rs` per §4.3 (delete the file; create `src/cli/info/tests/`).
- [ ] `scripts/test-accept.sh:260`: replace the stale `src/cli/build/mod.rs:1488` with
      `` `impl Default for OptLevel` in `src/optimizer/mod.rs` `` (pre-existing defect, §2;
      `grep -n 'build/mod.rs:1488' scripts/test-accept.sh` → 0 after).
- [ ] G4: tighten (the two deleted test files are STALE, the other eight TIGHTEN).

Acceptance: the manifest and CLI tests run from their new modules.
  Check: G1 → equal; G2 `cargo test -p mfb --bin mfb manifest:: cli::` → 0 failed;
  `sh -n scripts/test-accept.sh` → exit 0; G3; G4; G5 (est. 15 min; `cli::build` tests build
  real executables).
Commit: —

### Phase 4 — Debugger

- [ ] Split `src/debugger/breakpoints.rs` per §4.14.
- [ ] Split `src/debugger/breakpoints/report.rs` per §4.15.
- [ ] Split `src/debugger/prompt.rs` per §4.16.
- [ ] Split `src/debugger/runtime.rs` per §4.17.
- [ ] Split `src/debugger/values.rs` per §4.18.
- [ ] G4: tighten.

Acceptance: the debugger tests run from their new modules.
  Check: G1 → equal; G2 `cargo test -p mfb --bin mfb debugger::` → 0 failed (the fakes
  `Program`, `Exiting`, `Recorder`, `Memory`, `Fake` drive every test; no live process); G3; G4;
  G5 (est. 8 min).
Commit: —

### Phase 5 — The shared test harness (largest blast radius)

Every unit test in the binary calls `crate::testutil`; a wrong re-export breaks hundreds of
them, so it lands last and alone.

- [ ] Split `src/testutil.rs` per §4.36 (delete the file; create `src/testutil/`; the 3
      `pub(super)` edits).
- [ ] `tests/guards/architecture_guards/seams_goldens_and_panic_hook.rs`: `PANIC_HOOK_OWNER`
      = `"testutil/panics.rs"`, compared against the `src/`-relative path (§4.36 edit 1).
- [ ] `tests/guards/no_type_strings.rs` `is_test_file` + doc comment (§4.36 edit 2).
- [ ] `tests/guards/no_operator_strings.rs` `is_test_file` (§4.36 edit 3).
- [ ] G4: tighten (the `src/testutil.rs` row is STALE).

Acceptance: the whole unit-test binary still runs every test and the guards still bind.
  Check: G1 → equal; G2 `cargo test -p mfb --bin mfb` → 0 failed (est. 10 min; the harness
  is used by every module, so only the whole binary proves the re-exports); G3 → pass; and the
  panic-hook guard still bites: add `std::panic::take_hook();` to `src/testutil/fixtures.rs`,
  run `cargo test --test architecture_guards only_testutil_installs_a_panic_hook` → FAILED
  naming `src/testutil/fixtures.rs`, then remove the line and rerun → passes (est. 2 min);
  G4; G5.
Commit: —

## Validation Plan

- Tests: no test is added or removed. Per phase, the whole-binary `--list` count is unchanged
  (G1) and the moved modules' tests pass (G2). Phase 5 runs the whole unit-test binary because
  the harness feeds all of it.
- Coverage check: per-file coverage is not a plan-176 gate — author, 2026-10-17. Evidence: at 7a4b09970,
  `sh scripts/coverage.sh --bins` in a detached worktree (59 min; 4989 unit tests passed), then
  `sh scripts/coverage-check.sh` exited 1. 241 `src/` files are already below the 98% per-file
  floor ("Files below 98% line coverage (GATE FAILURE)"), and overall coverage is 93.67%.
  Examples among plan-176 hosts: `src/ir/shape.rs` 93.21%,
  `src/codegen/engine/control/builder_control.rs` 75.55%, `src/debugger/values.rs` 86.66%. A
  pure move changes no executed line, and moving an inline test block only removes test lines
  from its host's denominator. The pre-existing gate failure is reported to the owner
  separately and is not a plan-176 task.
- Runtime proof: the Phase 5 negative check (a hook call in a non-owner testutil part fails the
  guard) proves the narrowed guard still binds.
- Doc sync: `src/docs/spec/diagnostics/01_rule-codes.md` (test path), `scripts/test-accept.sh`
  (stale line ref). No `mfb man` topic changes. No compiler contract changes, so no other spec
  topic changes.
- Final gate (once, after Phase 5): `cargo test -p mfb --bin mfb spec_citations_resolve`,
  `cargo test --test architecture_guards --test no_type_strings --test no_operator_strings --test file_size_budget`
  → all pass and the banner has 0 rows; `cargo clippy --all-targets 2>&1 | grep -c '^warning'`
  → equal to its value at G0 of Phase 1 (est. 6 min). The full workspace suite runs once, in
  plan-176-V.

## Decisions

- Rule: code ≤ 1000, test ≤ 250 non-comment non-blank LOC; scope `.rs`/`.mfb` under `src/`
  `packages/` `examples/` `repository/` `tests/` — owner, 2026-10-14.
- Inline test code counts against the 250 test budget; over 250 it moves to its own module,
  split further as needed — owner, 2026-10-14.
- Generated files are exempt. The guard warns with a banner and never fails. A new module is
  always an acceptable split — owner, 2026-10-14.
- A single test-side unit over 250 gets its own file and an `exempt` row — owner, 2026-10-14.
  K has none: every topic in §4 is ≤ 250 (largest: `name_collisions.rs` 223, `src/ast/tests/source_collection.rs` 228).
- Extracted src test modules go into `foo/tests/mod.rs` + topic files, never `foo/tests.rs` —
  plan-176-A (coverage `IGNORE` and the guards recognise the `tests/` directory). This applies
  to `src/audit/report/tests/mod.rs` too.
- Guards that classify test code by file name are updated in the same commit as the move that
  needs them, recognising the new path, with the 4-point justification in the task — author,
  2026-10-14. K edits the panic-hook owner and the two `is_test_file` checks (§4.36); the
  `/tests/` rule in `architecture_guards` is plan-176-H's and is a K prerequisite.
- Pre-existing defects found by the map are fixed in the letter that touches them — author,
  2026-10-14. K fixes `scripts/test-accept.sh:260`. The stale `[[src/lexer.rs:262]]` /
  `[[src/lexer.rs:752]]` cites are plan-176-M's: they point at lexer code that M moves.
- Glob-cycle imports (`use super::*;` in parts, `pub use part::*;` in `mod.rs`) compile without
  warnings — verified (`/tmp/loc/probe2`).
- testutil parts reach each other's `pub(super)` items through the `pub use part::*` globs, not
  through per-name `use` lines — author (one mechanism, no per-name lists).

## Corrections

## Summary

K is pure motion of test code into `tests/` directory modules: 36 hosts, 1239 tests, 0 renamed.
The risk is in two places: `src/testutil.rs`, which every unit test imports (its own phase,
gated by the whole binary), and the three guard checks keyed on the name `testutil.rs`, each
replaced by a narrower path check with a negative test. Host code, host paths and every spec
citation stay where they are; letters L and M split the code next.
