# plan-176-K: Frontend and tooling test code

Last updated: 2026-10-18
Effort: large (3h–1d)
Depends on: plan-176-J

K moves the over-limit test code in the rest of `src/` — the AST, lexer, formatter, numeric
and type helpers, resolver, rules, audit, CLI, debugger, manifest, optimizer and doc renderer —
out of its host files into `tests/` directory modules. 33 files are in scope: 2 test-only files
over 1000 LOC (`src/ast/tests.rs`, `src/cli/info/tests.rs`) and 31 code files whose inline
`#[cfg(test)]` test code is over 250. An inline block of ≤ 1000 LOC moves whole into one
`<host>/tests/mod.rs`; only a block or file over 1000 is split, into parts of ≤ 900. No test is
added, removed or renamed. 28 of the 33 hosts move whole, so their test paths do not change
(`fmt::tests::custom_indent_width` stays `fmt::tests::custom_indent_width`); in the 5 split
hosts, a test placed in a topic file gains one segment
(`ast::tests::parses_map_literal` → `ast::tests::expressions_and_types::parses_map_literal`).
The host code files keep their paths. Their code-side splits are letters L and M.

The single outcome: after K lands, `cargo test --test file_size_budget` shows no `inline` or
`test` row for any of K's 33 files or for any file K creates, and
`tests/guards/file_size_ceilings.txt` has no `ceiling inline` or `ceiling test` row for them.

References:

- `planning/plan-176-A-file-size-guard.md` — the rule (code ≤ 1000, inline tests ≤ 250,
  test-only file ≤ 1000), §Decisions, and "Splitting an over-budget file" in
  `.ai/build-tooling.md` (src tests: `foo/tests/mod.rs`, plus topic files only when over 1000).
- `.ai/testing-gates.md` § "Splits must sweep man AND spec citations".
- `.ai/build-tooling.md` — rustfmt/clippy policy and the split conventions.
- `.ai/codegen-invariants.md` — names `src/ast/scope_privates.rs`, `src/lexer.rs`,
  `src/numeric.rs`, `src/types.rs` (all host paths K keeps).
- `.ai/resources-packages.md` — names `src/resolver/mod.rs` (host path K keeps).
- `.ai/man-content.md` — names `src/cli/man.rs` (host path K keeps).
- Spec topics: no `[[…]]` citation points into a moved range (measured per file in §4). The
  one spec test-path mention, `rules::tests::rule_codes_are_unique`
  (`src/docs/spec/diagnostics/01_rule-codes.md`), stays valid: `src/rules/mod.rs` moves whole.
- `scripts/test-fast-skips.txt` — names `cli::build::tests::builtin_codegen_corpora_lower_in_process`
  as a libtest `--skip` substring (§2).
- Guards K runs but does not edit: `tests/guards/architecture_guards.rs`,
  `tests/guards/no_type_strings.rs`, `tests/guards/no_operator_strings.rs`.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-176-J is complete and archived | `ls planning/plan-176-J-* planning/completed/plan-176-J-*` → only `planning/completed/…` | MET (2026-10-19) |
| The file-size guard exists and passes | `cargo test --test file_size_budget` → passes | MET (2026-10-19) |
| The architecture guards treat a `tests` directory component as test code (landed by plan-176-H) | `grep -n 'contains("/tests/")' tests/guards/architecture_guards.rs` → a hit in each of `is_physical_scan_exempt`, `no_hand_picked_vreg` and `emission_lines` | MET (2026-10-19: 3 hits, `118ec25a8`) |
| K's files have no uncommitted changes | `git status --porcelain -- src/ast src/cli src/optimizer src/audit src/debugger src/doc/html.rs src/fmt.rs src/lexer.rs src/manifest src/numeric.rs src/resolver src/rules src/types.rs scripts/test-accept.sh` → empty | MET (2026-10-19) |

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

- Every host in §2's table is ≤ 250 inline test LOC after the move, and every test-only file K
  creates is ≤ 1000 (`python3 -I tools/loc-census/census.py --file <path>` → second field
  ≤ 250 for a host, ≤ 1000 for a test-only file). Every part of a split host is ≤ 900.
- `cargo test --test file_size_budget` prints no `inline` or `test` row naming a K path, and the
  ceiling file has no `ceiling inline` / `ceiling test` row for any K path. (Hosts over 1000
  code keep their `ceiling code` row; that is L's and M's.)
- The unit-test binary lists the same number of tests before and after each phase.

### Non-goals (explicit constraints)

- No behavior change. No test body edit beyond moving it and adding `use super::*;` at the top
  of a topic file of a split host. No visibility edit (§2 "Child modules see a parent's private
  items").
- No `#[test]` fn renamed, added or removed. No `[[test]]` target renamed (K touches no target).
- No host code moves. `src/lexer.rs`, `src/cli/pkg.rs`, `src/debugger/prompt.rs` etc. keep
  their code and their paths; letters L and M split the code.
- No guard edit and no guard weakens.

## 2. Current State

All counts at HEAD `ce7a995ca` (2026-10-18). Host and file totals:
`python3 -I /tmp/loc/census.py "$PWD"` → `/tmp/loc/census.json` (fields `code`, `test`,
`testfile`; for a test-only file `test` is its total LOC). Moved region:
`python3 -I /tmp/loc/span.py <file> <a> <b>` over the inline module's extent, from its
`#[cfg(test)]` line to its closing `}` (located by `python3 -I /tmp/loc/rw-K/mods.py <files>`).
After plan-176-A the same numbers come from `python3 -I tools/loc-census/census.py --file <path>`.
Test counts: `grep -cE '#\[(tokio::)?test\]' <file>`. "New file" = moved region − the 3 wrapper
lines (`#[cfg(test)]`, `mod tests {`, `}`); "Host inline after" = inline − moved + the 2
declaration lines `#[cfg(test)]` / `mod tests;`.

| File | Kind | Code | Inline / test | Tests | Moved region (HEAD) | New layout (§4) | Host inline after |
|---|---|---|---|---|---|---|---|
| `src/ast/tests.rs` | test-only | — | 2374 | 166 | whole file | split, 4 files | file removed |
| `src/cli/info/tests.rs` | test-only | — | 1059 | 23 | whole file | split, 2 files | file removed |
| `src/ast/scope_privates.rs` | inline | 517 | 390 | 20 | 629-1112 (390) | `tests/mod.rs` 387 | 2 |
| `src/audit/collect/findings.rs` | inline | 253 | 294 | 10 | 281-616 (294) | `tests/mod.rs` 291 | 2 |
| `src/audit/collect/project.rs` | inline | 136 | 342 | 10 | 166-541 (342) | `tests/mod.rs` 339 | 2 |
| `src/audit/collect/source.rs` | inline | 820 | 669 | 26 | 1088-1852 (664) | `tests/mod.rs` 661 | 7 |
| `src/audit/report.rs` | inline | 183 | 301 | 4 | 255-515 (256), 517-565 (45) | `testsupport/mod.rs` 253, `tests/mod.rs` 42 | 4 |
| `src/cli/build/mod.rs` | inline | 847 | 2750 | 142 | 1337-4672 (2742) | split, 4 files | 10 |
| `src/cli/man.rs` | inline | 817 | 971 | 49 | 1102-2365 (971) | `tests/mod.rs` 968 | 2 |
| `src/cli/pkg.rs` | inline | 2019 | 899 | 60 | 2515-3624 (896) | `tests/mod.rs` 893 | 5 |
| `src/cli/resolve.rs` | inline | 1016 | 600 | 27 | 1351-2090 (600) | `tests/mod.rs` 597 | 2 |
| `src/debugger/breakpoints.rs` | inline | 1300 | 999 | 22 | 1591-2686 (999) | `tests/mod.rs` 996 | 2 |
| `src/debugger/breakpoints/report.rs` | inline | 204 | 435 | 12 | 286-764 (435) | `tests/mod.rs` 432 | 2 |
| `src/debugger/prompt.rs` | inline | 2131 | 2047 | 68 | 2438-4588 (2047) | split, 3 files | 2 |
| `src/debugger/runtime.rs` | inline | 448 | 410 | 14 | 569-1023 (410) | `tests/mod.rs` 407 | 2 |
| `src/debugger/values.rs` | inline | 1068 | 501 | 13 | 1208-1735 (501) | `tests/mod.rs` 498 | 2 |
| `src/doc/html.rs` | inline | 269 | 387 | 16 | 295-732 (387) | `tests/mod.rs` 384 | 2 |
| `src/fmt.rs` | inline | 672 | 575 | 53 | 918-1636 (575) | `tests/mod.rs` 572 | 2 |
| `src/lexer.rs` | inline | 1199 | 555 | 35 | 1454-2111 (555) | `tests/mod.rs` 552 | 2 |
| `src/manifest/config.rs` | inline | 554 | 289 | 19 | 673-994 (289) | `tests/mod.rs` 286 | 2 |
| `src/manifest/libraries.rs` | inline | 316 | 380 | 19 | 482-914 (380) | `tests/mod.rs` 377 | 2 |
| `src/manifest/mod.rs` | inline | 1092 | 1002 | 73 | 1461-2578 (945) | `tests/mod.rs` 942 | 59 |
| `src/manifest/package.rs` | inline | 712 | 1177 | 82 | 1013-2494 (1177) | split, 2 files | 2 |
| `src/numeric.rs` | inline | 413 | 835 | 33 | 716-1667 (793) | `tests/mod.rs` 790 | 44 |
| `src/optimizer/opt1/globals.rs` | inline | 275 | 332 | 12 | 371-753 (332) | `tests/mod.rs` 329 | 2 |
| `src/optimizer/opt1/inline.rs` | inline | 254 | 270 | 7 | 351-654 (270) | `tests/mod.rs` 267 | 2 |
| `src/optimizer/opt2/promote.rs` | inline | 330 | 294 | 10 | 495-834 (286) | `tests/mod.rs` 283 | 10 |
| `src/optimizer/opt2/sink.rs` | inline | 372 | 300 | 12 | 518-850 (300) | `tests/mod.rs` 297 | 2 |
| `src/resolver/mod.rs` | inline | 565 | 631 | 28 | 816-1513 (622) | `tests/mod.rs` 619 | 11 |
| `src/resolver/packages.rs` | inline | 221 | 280 | 20 | 290-635 (280) | `tests/mod.rs` 277 | 2 |
| `src/resolver/resolution.rs` | inline | 1496 | 737 | 57 | 1750-2613 (737) | `tests/mod.rs` 734 | 2 |
| `src/rules/mod.rs` | inline | 272 | 283 | 20 | 448-792 (253) | `tests/mod.rs` 250 | 32 |
| `src/types.rs` | inline | 922 | 1011 | 27 | 1682-2928 (992) | `tests/mod.rs` 989 | 21 |

Total tests in the 33 files: 1189 (sum of the per-file `grep -cE` counts above). 1187 move; the
2 tests of `manifest::canvas_auto_paint_tests` stay inline. `src/audit/report.rs` keeps 4
declaration lines (two modules). Files K creates: 44 (27 single `tests/mod.rs`, 2 for
`audit/report`, 15 files of the 5 split hosts).

### Verified properties

- **Every K `#[test]` is listed in exactly one place.** For the 5 split hosts, §4 assigns every
  test by name (the split-map generator checked the topic lists against the `grep -cE` count at
  `17a9b5799`; the 7 tests added since — 2 in `breakpoints.rs`, 5 in `prompt.rs`, found with
  `diff <(git show 17a9b5799:<f> | grep -A2 '#\[test\]' | grep -oE 'fn [a-z0-9_]+') <(grep -A2 '#\[test\]' <f> | grep -oE 'fn [a-z0-9_]+')`
  — are placed in §4.4 and §4.3). The whole-binary `--list` count is the before == after check
  per phase.
- **Glob cycles compile clean.** `use super::*;` in a part plus `mod part;` in `mod.rs`:
  probe `/tmp/loc/probe2` printed `6` with no warning.
- **Child modules see a parent's private items.** Rust privacy is descendant-visible, so a
  helper left in `tests/mod.rs` is callable from every topic with no visibility edit. In the
  split hosts every helper used by more than one part stays in `mod.rs`; every part-only helper
  moves with its part. So K has no visibility edit.
- **`src/cli/build/mod.rs`, `src/manifest/mod.rs`, `src/resolver/mod.rs` and `src/rules/mod.rs`
  are already `mod.rs` files** with no `tests` child today, so their tests go to the sibling
  `tests/` directory. Every other host `foo.rs` gets `foo/tests/` (`mod tests;` in `foo.rs`
  resolves to `foo/tests/mod.rs`). Collisions checked: `ls src/cli/pkg` → `command_tests.rs`
  only; `ls src/debugger/breakpoints` → `report.rs` only; no other host has a same-named
  directory today.
- **The register guards scan moved test code unless the path has a `tests` component.**
  `tests/guards/architecture_guards.rs` `no_hand_picked_vreg` and `emission_lines` skip a file
  only by NAME (`name.ends_with("tests.rs")`) and otherwise scan text above the first
  `#[cfg(test)]`. A moved `tests/mod.rs` has no such marker. K's promote and sink tests carry
  `"%vN"` literals and `.physical_field(` calls
  (`grep -nE '"%[vf][0-9]|\.physical_field\(' src/optimizer/opt2/promote.rs src/optimizer/opt2/sink.rs`
  → 30 lines at `17a9b5799`, all in the moved ranges). plan-176-H adds the `/tests/` component
  rule to all three name checks; K's Prerequisites require it.
- **No fixture path is relative to the moved file.** Every fixture read in a moved range joins
  `env!("CARGO_MANIFEST_DIR")` (absolute; lines as of `17a9b5799`):
  `src/cli/build/mod.rs:2241`, `src/audit/collect/findings.rs:488`,
  `src/resolver/packages.rs:482,520,556`, `src/rules/mod.rs:560`, `src/types.rs:1695`.
- **Stale line reference in a moved range.** `scripts/test-accept.sh:275` cites
  `src/cli/build/mod.rs:1488` for "the flagless build IS `-O1`" (`grep -n 'build/mod.rs'
  scripts/test-accept.sh`). Line 1488 is inside the test module (which starts at 1337), not the
  claim's source. The source is `impl Default for OptLevel` → `OptLevel(1)` at
  `src/optimizer/mod.rs:71` (`grep -n "impl Default for OptLevel" -A3 src/optimizer/mod.rs`).
- **A skip list names a full test path in a split host.** `scripts/test-fast-skips.txt:21` is
  the libtest `--skip` substring `cli::build::tests::builtin_codegen_corpora_lower_in_process`,
  and `scripts/test-fast.sh` fails on an entry that matches no test ("stale entry in
  scripts/test-fast-skips.txt"). §4.3 keeps that test in `cli/build/tests/mod.rs`, so its path
  and the entry are unchanged.
- **Live documents that name a full test path.**
  `grep -rnoE "(ast|cli::build|cli::info|debugger::prompt|manifest::package)::tests::[a-z_0-9:]+" scripts .ai AGENTS.md src/docs .github tests/guards bugs planning/*.md`
  (the 5 split hosts) → live hits: `scripts/test-fast-skips.txt:21` (above) and
  `planning/plan-174-C-dbg-riscv64-descriptor.md:280` (`debugger::prompt::tests::on_riscv64`, a
  test that plan adds; the prefix stays a valid filter wherever it is placed). Every other hit
  is in `bugs/completed/` (history, not edited) or `planning/todo.md` (pasted test-run output,
  history). The 28 whole-file moves change no test path, so
  `rules::tests::rule_codes_are_unique` in `src/docs/spec/diagnostics/01_rule-codes.md:103` and
  `bugs/bug-883-rule-codes-spec-drift-and-unemitted-rules.md:43` stays valid.
- **`src/lexer.rs` line citations are not in K's range.** `[[src/lexer.rs:262]]` and
  `[[src/lexer.rs:752]]` (`src/docs/spec/architecture/19_internal-naming.md:17,50`) point at
  code lines above the test module (1454). K's move does not shift them. They are stale
  (line 262 is `self.advance();`, line 752 is `value.push(self.peek());`); plan-176-M fixes
  them when it moves `src/lexer.rs` to `src/lexer/`.

## 3. Design Overview

One convention, applied 33 times: the inline `#[cfg(test)] mod tests { … }` (or the whole
test-only file) becomes `#[cfg(test)] mod tests;` → `<host dir>/tests/mod.rs`.

- **≤ 1000 LOC (28 hosts):** the module body moves verbatim into `tests/mod.rs`. No topic
  files, no new `use` lines, and every test keeps its path.
- **Over 1000 LOC (5 hosts: `src/ast/tests.rs`, `src/cli/info/tests.rs`,
  `src/cli/build/mod.rs`, `src/debugger/prompt.rs`, `src/manifest/package.rs`):** split into
  parts of ≤ 900. `tests/mod.rs` holds the shared `use` lines, the shared helpers and fixtures,
  one `mod <part>;` per part, and the first group of tests. Each further part starts with
  `use super::*;` and holds a group of neighbouring topics from the split map (the map's old
  topic boundaries are the seams; no new seam is invented). A helper used by one part only
  moves with that part.
- `src/audit/report.rs` has two cfg(test) modules; each moves whole to its own file (§4.6).

Gate class: test letter. Per phase, the whole-binary test count before == after, the moved
modules' tests run, `cargo check --all-targets` adds no warning, the three guards pass, and the
file-size guard shows only TIGHTEN/STALE rows for the phase's files before the tighten.

Risk concentrates in three places:
- **The promote and sink tests depend on plan-176-H's `/tests/` guard rule** — without it the
  moved `"%vN"` literals fail `no_hand_picked_vreg` (§2). Prerequisite row 3.
- **Two whole-file moves sit close to the 1000 limit** — `src/debugger/breakpoints.rs` → 996 and
  `src/types.rs` → 989. Both grew since `17a9b5799` (breakpoints by 2 tests). Each task
  re-measures first; §4.5 gives the split to use if one is over 1000 by then.
- **Test paths change in the 5 split hosts** (345 tests gain a segment). The one live consumer,
  `scripts/test-fast-skips.txt:21`, is kept valid by placement (§4.3).

Rejected alternatives:
- *`#[path = "…"]` attributes to keep `foo.rs` + `foo_tests.rs` side by side* — `tests/`
  directories are what the coverage `IGNORE` (`(^|/)tests/`) and the guards' `is_test_file`
  already recognise; a sibling file would be counted by coverage and scanned by the guards.
- *Moving test code into integration tests under `tests/`* — the tests reach private items
  (`super::*`); an integration crate cannot.
- *Splitting every moved block into topic files anyway* — the rule does not ask for it (a
  test-only file only has to be ≤ 1000); it would change the path of every moved test and add
  a `use super::*;` seam per topic for no limit.
- *Moving only the over-limit part of an inline block* — every host here would still need a
  `tests/` module; moving the whole block keeps one place for a host's tests.

## 4. Detailed Design

Line ranges marked "as of 17a9b5799" are the split map's source lines; item and test NAMES are
authoritative if lines drift. A part's LOC is the sum of the split map's topic LOC
(`/tmp/loc/maps/K.md`, `17a9b5799`) minus the `use super::*;` lines that merging removes and the
`mod` lines it no longer needs — labelled "sum of map parts"; the HEAD total of each split host
is in §2. Section-separator comments between groups move with the group that follows them.
Visibility edits: none.

### 4.1 `src/ast/tests.rs` → `src/ast/tests/` (166 tests; 2374 → 4 files)

`src/ast/mod.rs` keeps `#[cfg(test)] mod tests;`. Spec citations to `src/ast/tests.rs`: 0
(`grep -rnoE "\[\[src/ast/tests.rs:[^]]+\]\]" src/docs/spec`); other refs: none.
- `mod.rs` (707, sum of map parts: shared 106 + 601): the 5 `use` lines (including
  `#[cfg(unix)] use std::os::unix::fs::symlink;`), 3 `mod` lines; shared: `parse_ok`,
  `parse_err`, `ret`, `first_return_expr`, `expr_of`, `type_err`, `manifest_with_sources`,
  `source_entry`, `test_temp_dir`, `parse_file`, `try_parse`, `project_json`, `function`,
  `link_fn`. Tests (37; paths unchanged), the project and source-root group:
  - source roots (map topic `source_roots`, 190; 9-17, 85-209, 1050-1122, 1180-1195 as of
    17a9b5799) — include/exclude, globs, overlap, missing/empty roots, symlink containment.
    Tests (10): glob_patterns_match_nested_and_root_files, file_root_ignores_include_patterns,
    directory_root_applies_include_and_exclude_patterns, overlapping_source_entries_are_rejected,
    symlinked_source_paths_must_stay_inside_project, source_root_missing_is_rejected,
    empty_source_root_is_rejected, source_entry_without_root_is_ignored,
    file_root_ignores_non_mfb_extension, glob_star_and_question_components_match.
  - project parsing (`project_parsing`, 186; 934-1048, 1124-1178, 1197-1243). Tests (12):
    parse_project_reads_files_and_appends_prelude, parse_project_rejects_missing_project_dir,
    selected_source_paths_are_sorted_actual_paths,
    selected_source_paths_rejects_missing_project_dir, write_ast_writes_ast_json_file,
    write_ast_reports_write_failure, parse_source_internal_marks_file_internal,
    directory_cycle_via_symlink_is_tolerated, nested_symlink_escape_is_rejected,
    parse_project_propagates_collection_failure,
    selected_source_paths_propagates_collection_failure,
    broken_symlink_entry_reports_read_failure.
  - source collection (`source_collection`, 228; 1245-1316, 1360-1566). Tests (15):
    parse_source_reports_lex_failure, parse_project_reports_unreadable_source_file,
    unreadable_directory_reports_read_failure, parse_project_propagates_parse_errors,
    parse_project_missing_directory_is_error, collect_reports_missing_and_empty_roots,
    collect_handles_single_file_and_non_mfb_root, source_entries_default_include_when_absent,
    collect_walks_nested_directories, selected_source_paths_missing_dir_is_error,
    nested_symlink_escaping_project_is_rejected, glob_component_edge_cases,
    broken_symlink_root_reports_canonicalize_failure,
    unreadable_source_file_reports_read_failure, symlink_cycle_within_project_is_skipped.
- `expressions_and_types.rs` (610, sum of map parts). Tests (65):
  - literals and operators (`literals_and_operators`, 200; 19-83, 254-409). Tests (15):
    parse_import_aliases, string_concat_has_lower_precedence_than_addition,
    parses_primary_literal_forms, parses_list_literal_including_empty, parses_map_literal,
    parses_empty_map_literal, parses_map_literal_with_res_value, map_literal_requires_to_keyword,
    map_literal_requires_colon_equal_between_key_and_value, parses_operator_precedence_chain,
    parses_every_comparison_operator, parses_arithmetic_and_multiplicative_operators,
    parses_power_right_associative_and_unary_minus, parses_pipeline_with_placeholder,
    pipeline_without_placeholder_is_rejected.
  - calls and lambdas (`calls_and_lambdas`, 203; 411-581, 852-926). Tests (29):
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
  - type syntax (`types`, 209; 585-596, 603-850). Tests (21): parses_simple_and_generic_types,
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
- `serialize_and_signatures.rs` (391, sum of map parts). Tests (12):
  - serializer (`serialize`, 219; 1605-1830). Tests (2): serialize_covers_all_item_kinds,
    serialize_covers_all_statement_and_expression_kinds.
  - signatures and placeholders (`signatures_and_placeholders`, 173; 1832-2039). Tests (10):
    function_signature_line_renders_visibility_isolated_and_params,
    type_decl_signature_line_covers_all_kinds_and_visibility,
    pipeline_placeholder_substitutes_across_all_expression_forms,
    pipeline_placeholder_missing_is_rejected,
    pipeline_placeholder_in_lambda_and_map_and_boolean,
    serialize_covers_none_and_default_branches,
    serialize_exit_targets_func_and_do_and_while, serialize_lambda_without_assign_target,
    pipeline_placeholder_walks_named_args_and_trapped_and_with,
    all_comparison_and_arithmetic_operators_serialize.
- `parse_errors.rs` (672, sum of map parts). Tests (52):
  - declaration errors (`declaration_errors`, 137; 2046-2255). Tests (12):
    top_level_errors_are_reported, function_header_and_body_errors,
    function_declaration_requires_the_func_keyword, trap_header_errors, type_decl_errors,
    statement_errors, expression_errors, doc_header_kind_keywords,
    doc_prose_kind_roundtrips_all_variants, qualified_member_may_be_keyword_or_numeric,
    qualified_member_missing_after_colon_is_error, qualified_numeric_member_edge_cases.
  - DOC and LINK blocks (`doc_and_link_blocks`, 145; 2263-2452). Tests (10):
    doc_block_prose_kinds_and_continuation, doc_block_header_variants_and_errors,
    doc_block_named_line_errors_and_unknown, doc_func_header_signature_forms,
    link_block_errors, native_func_missing_symbol_or_abi,
    native_func_error_on_and_return_and_const, native_func_free_block_and_errors,
    resource_declaration_errors, func_alias_errors.
  - ABI and statement errors (`abi_and_statement_errors`, 192; 2458-2729). Tests (14):
    abi_spec_malformed_paths, free_block_malformed_abi_paths, const_pin_errors,
    doc_block_bare_prose_keyword_flushes, doc_example_dedent_handles_multibyte_whitespace,
    inline_trap_error_paths, deeply_nested_inline_traps_hit_the_statement_depth_cap,
    match_union_pattern_errors, if_elseif_error_paths, loop_close_errors,
    enum_multiple_member_lines, pipeline_placeholder_each_expression_kind_as_rhs,
    lambda_and_function_type_close_errors, native_symbol_must_be_string_and_name_required.
  - TESTING blocks and assignment (`testing_and_assignment`, 201; 2735-2989). Tests (16):
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

Total 2380 (sum of map parts; census 2374).

### 4.2 `src/cli/info/tests.rs` → `src/cli/info/tests/` (23 tests; 1059 → 2 files)

`src/cli/info.rs` keeps `#[cfg(test)] mod tests;`. Citations 0; other refs none.
- `mod.rs` (390, sum of map parts: shared 192 + 198): the 8 `use` lines, `mod signed;`;
  shared: `VERSION`, `DIGEST`, `REGISTRY`, `image`, `import`, `linked_images_with`,
  `linked_images`, `FakeRegistry` + its 3 impls, `report`. Tests (10; paths unchanged):
  - unsigned images (`unsigned_images`, 136; 234-383 as of 17a9b5799). Tests (6):
    an_unsigned_image_reports_format_arch_libraries_compiler_and_unsigned,
    a_file_without_the_note_is_not_a_mfbasic_binary, a_dynamic_elf_reports_its_interpreter,
    a_dynamic_elf_reports_its_libraries_and_search_path,
    a_mach_o_reports_its_dylibs_and_search_path, a_pe_reports_its_import_dlls.
  - metadata (`metadata`, 64; 406, 1061-1103, 1120-1143). Tests (4):
    malformed_signing_metadata_is_reported_as_such,
    pathologically_nested_json_is_refused_without_recursing,
    a_descriptor_of_an_unknown_version_names_no_compiler, format_utc_renders_civil_dates.
    Helper: SIGNED_PLACEHOLDER.
- `signed.rs` (668, sum of map parts): `use super::*;`; the signing-chain fixture `Chain` +
  `impl Chain`, `signed_line`, `verdicts`, `trust_chain`, `report_line` (map `signed/mod.rs`,
  408-588, 626-630). `Chain`'s private fields and methods stay private to this file. Tests (13):
  - contents (`signed/contents`, 223; 385-404, 590-624, 632-742, 1145-1218). Tests (6):
    truncated_and_corrupted_images_never_panic, a_signed_image_reports_its_signer_and_verdicts,
    a_changed_byte_anywhere_outside_the_blob_reports_modified_contents,
    a_blob_copied_into_another_image_reports_modified_contents,
    contents_that_carry_no_usable_signature_say_so,
    a_signed_debug_image_reports_intact_contents.
  - chain checks (`signed/chain_checks`, 184; 744-856, 971-1059, 1105-1118). Tests (5):
    the_chain_is_checked_against_the_registry_the_blob_names,
    the_ident_key_is_checked_against_the_owner_s_current_key,
    a_chain_attested_by_another_registry_key_is_not_verified,
    every_link_of_the_chain_refuses_an_altered_blob, signer_strings_are_sanitized.
  - ownership (`signed/ownership`, 102; 858-969). Tests (2):
    a_transferred_executable_reports_the_new_owner,
    a_foreign_ident_without_a_chain_is_refused. Helper: probe_index.

### 4.3 `src/cli/build/mod.rs` → `src/cli/build/tests/` (142 tests; 2742 moved → 4 files)

The `#[cfg(test)] pub(crate) fn native_libraries_for_test` and `pub(crate) fn build_source_dependency_interfaces`
(1269-1318 as of 17a9b5799; called by `src/testutil.rs`) stay in the host. Citations: 24, all
name host code. `scripts/test-accept.sh:275` cites `src/cli/build/mod.rs:1488` — stale (§2);
repointed in Phase 3. `builtin_codegen_corpora_lower_in_process` stays in `mod.rs` so
`scripts/test-fast-skips.txt:21` keeps matching it (§2).
- `mod.rs` (827, sum of map parts: shared 87 + 740): `use super::*;`, the two mid-module `use`
  lines (old 2502-2503, so every part sees them), 3 `mod` lines; shared: `s`, `build_err`,
  `write_executable_project`, `copy_test_project`, `write_package_project`, `running_as_root`.
  Tests (30; paths unchanged), the `build_project` group:
  - basics (`build_project_basics`, 206; 2233-2422, 2451-2498 as of 17a9b5799). Tests (11):
    builtin_codegen_corpora_lower_in_process, build_project_validates_a_bad_manifest,
    build_project_rejects_app_mode_for_non_app_target, build_project_builds_a_host_executable,
    build_project_rejects_a_traversing_project_name, build_project_clears_stale_build_dir,
    mfb_test_host_run_leaves_project_build_dir_untouched, build_project_writes_ast_and_ir_dumps,
    build_project_builds_a_package, build_project_rejects_native_output_for_a_package,
    build_project_reports_a_source_error.
  - validation (`build_project_validation`, 165; 3754-3939). Tests (8):
    build_project_rejects_expect_outside_a_test_case,
    build_project_coverage_test_writes_a_report,
    build_project_unknown_kind_validates_and_builds_nothing,
    build_project_writes_every_native_dump, build_project_rejects_native_dumps_for_a_package,
    build_project_rejects_sign_with_output_flags,
    build_project_sign_without_a_local_ident_key_fails,
    build_project_rejects_a_link_without_a_libraries_entry.
  - manifests (`build_project_manifests`, 145; 4242-4411). Tests (6):
    build_project_sign_rejects_a_malformed_manifest_ident,
    build_project_export_in_an_executable_is_a_verify_error,
    build_project_errors_when_build_path_is_a_file,
    build_project_executable_missing_dependency_is_a_hard_error,
    build_project_package_missing_dependency_is_a_hard_error,
    build_project_reads_installed_dependency_exports.
  - source dependencies (`source_dependencies`, 135; 4413-4563). Tests (2):
    build_project_compiles_a_source_directory_dependency,
    build_project_rejects_a_source_dependency_cycle. Helper: write_source_package.
  - cross target and read-only (`cross_target_and_readonly`, 94; 4565-4672). Tests (3):
    build_project_cross_target_test_reports_the_artifact,
    build_project_package_write_to_a_readonly_location_fails,
    build_project_dump_writers_report_a_readonly_location.
- `options.rs` (505, sum of map parts). Tests (44):
  - basics (`options_basics`, 203; 1346-1592). Tests (17):
    build_output_from_flag_maps_every_flag,
    build_output_from_flag_single_dash_aliases_double_dash, parse_build_options_defaults,
    parse_build_options_parses_target_both_forms,
    parse_build_options_single_dash_aliases_double_dash,
    parse_build_options_app_debug_implies_app_mode, parse_build_options_debug_sets_flag,
    parse_build_options_rejects_repeated_debug, parse_test_options_debug_sets_flag,
    parse_build_options_parses_config_both_forms, parse_build_options_rejects_repeated_config,
    parse_build_options_rejects_empty_config, parse_test_options_parses_config_both_forms,
    parse_test_options_rejects_repeated_config, parse_test_options_rejects_empty_config,
    parse_test_options_rejects_app_debug, parse_test_options_single_dash_aliases_double_dash.
  - flags (`options_flags`, 193; 1594-1830). Tests (16):
    parse_build_options_target_requires_value, parse_build_options_target_rejects_malformed,
    parse_build_options_sign_both_forms_and_conflicts, parse_build_options_unsigned_flag,
    parse_build_options_verbosity_defaults_to_normal, parse_build_options_quiet_both_spellings,
    parse_build_options_verbose_both_spellings, parse_build_options_quiet_and_verbose_conflict,
    parse_build_options_repeated_verbose_selects_trace,
    parse_build_options_quiet_and_trace_conflict, parse_test_options_is_quiet,
    parse_test_options_verbose_both_spellings, parse_test_options_repeated_verbose_selects_trace,
    parse_test_options_rejects_quiet, parse_build_options_regalloc_both_forms_and_bad_value,
    parse_build_options_opt_level_every_spelling.
  - errors and modes (`options_errors_and_modes`, 111; 1832-1860, 1869-1879, 2865-2955).
    Tests (11): parse_build_options_opt_level_rejects_unlanded_and_malformed,
    parse_build_options_rejects_unknown_option_and_two_locations,
    parse_build_options_takes_a_positional_location,
    parse_build_options_defaults_to_console_mode, parse_build_options_accepts_app_flag,
    parse_build_options_rejects_duplicate_app_flag,
    parse_build_options_app_flag_composes_with_native_output,
    parse_build_options_combines_output_flags_in_order,
    parse_build_options_accepts_single_dash_aliases,
    parse_build_options_rejects_duplicate_output_flag,
    parse_build_options_no_output_flags_means_full_build.
- `signing.rs` (531, sum of map parts). Tests (29):
  - package classification (`package_classification`, 179; 1881-2104). Tests (13):
    package_verification_labels, source_is_local_classifies_sources,
    signing_ident_defaults_to_owner_hash_name, signing_ident_rejects_bad_idents,
    signing_ident_accepts_a_transferred_ident, classify_installed_package_reads_unsigned_fixture,
    classify_installed_package_treats_missing_file_as_tampered,
    classify_installed_package_treats_garbage_as_tampered, verify_and_report_no_packages_is_ok,
    verify_and_report_missing_dependency_file_is_skipped,
    verify_and_report_unsigned_remote_requires_flag,
    verify_and_report_refuses_a_pinned_version_mismatch,
    verify_and_report_unsigned_local_is_allowed.
  - signing metadata (`signing_metadata`, 82; 2106-2190). Tests (3):
    apply_signing_metadata_copies_fields, executable_signing_metadata_json_is_valid_json,
    decode_trust_anchor_accepts_metadata_key_form.
  - signed packages (`signed_packages/mod` 99 + `signed_packages/verification` 175;
    3310-3420, 3506-3519, 3422-3504, 3521-3618, 4126-4148). Fixtures (private to this file):
    `SignedTamper`, `SignedFixture`, `build_signed_fixture`, `write_mfp`, `classify_pinned`.
    Tests (13): classify_signed_package_without_a_trust_anchor_is_untrusted,
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
- `outputs.rs` (878, sum of map parts) — what a build writes besides the executable: vendored
  libraries, resources, app-mode icons, reports. Tests (39):
  - vendoring fixtures (`vendoring/mod`, 29; 2505-2541), private to this file:
    `vendor_locator`, `resolved`, `OWN_UNIT`, `write_vendor_source`.
  - vendor output names (`vendoring/output_names`, 124; 2543-2703). Tests (7):
    the_copied_filename_is_the_emitted_dlopen_name,
    imported_vendor_file_is_read_from_the_per_package_directory,
    two_packages_vendoring_the_same_filename_land_as_two_distinct_files,
    colliding_output_names_with_differing_hashes_are_rejected,
    colliding_output_names_with_identical_hashes_are_allowed,
    no_vendor_locators_writes_nothing, vendor_output_dirs_match_the_emitted_rpath_per_shape.
  - vendor libraries (`vendoring/vendor_libraries`, 108; 3163-3298). Tests (8):
    verify_vendor_libraries_reports_a_missing_file,
    verify_vendor_libraries_rejects_a_locator_without_a_hash,
    verify_vendor_libraries_rejects_a_hash_mismatch,
    verify_vendor_libraries_accepts_a_matching_hash,
    vendor_source_path_distinguishes_own_and_imported_units,
    vendor_source_path_finds_a_source_dependency_s_staged_vendor,
    stage_source_dependency_vendor_copies_the_flat_vendor_files,
    copy_vendor_libraries_errors_when_the_source_is_missing.
  - vendoring builds (`vendoring/build_project`, 160; 3941-4124). Tests (3):
    build_project_builds_an_executable_that_vendors_a_library,
    build_project_reports_a_resource_copy_failure,
    build_project_rejects_a_package_link_without_a_libraries_entry.
  - resources (`resources`, 220; 2705-2819, 3018-3159). Tests (9):
    resource_output_dir_per_build_shape, resource_src_fixed_prefix_splits_at_first_glob,
    copy_resources_refuses_a_source_that_resolves_outside_the_project,
    copy_resources_maps_the_worked_examples, copy_resources_handles_a_root_level_glob,
    copy_resources_reports_an_unreadable_source_directory,
    copy_resources_reports_an_unreadable_subdirectory,
    copy_resources_reports_an_uncreatable_destination,
    copy_resources_reports_an_unreadable_source_file.
  - reporting and modes (`reporting_and_modes`, 99; 2821-2863, 2976-3014, 3622-3672).
    Tests (8): emitted_link_targets_track_what_each_build_mode_actually_emits,
    run_test_binary_maps_every_exit_status,
    generate_coverage_report_warns_when_the_covmap_is_missing,
    generate_coverage_report_writes_html_from_a_seeded_covmap,
    build_output_label_names_every_variant, build_project_verbose_runs_the_phase_reporter,
    build_project_trace_runs_the_compile_profiler,
    build_project_rejects_app_mode_for_a_package.
  - app mode (`app_mode`, 147; 3674-3752, 4150-4240). Tests (4):
    build_project_app_mode_missing_icon_is_rejected_per_target,
    build_project_rejects_importing_app_without_app_mode,
    build_project_app_mode_present_icon_resolves_then_backend_rejects,
    build_project_app_mode_without_an_icon_field_takes_the_none_arm. Helpers:
    write_app_project_missing_icon, write_app_project_present_icon.

Total 2741 (sum of map parts; HEAD moved region 2742 − 3 wrapper lines = 2739).

### 4.4 `src/debugger/prompt.rs` → `src/debugger/prompt/tests/` (68 tests; 2047 moved → 3 files)

Citations: 28, all host code.
- `mod.rs` (629, sum of map parts: shared 74 + 555): the 3 `use` lines, 2 `mod` lines; shared:
  `Recorder` + `impl Recorder`, `c`, `symbol`, `CONSOLE`, `session`, `drive`; the fake
  `impl Inferior for Recorder` (map `fake_recorder`, 173; 2396-2568 as of 17a9b5799).
  Tests (23; paths unchanged):
  - session basics (`session_basics`, 112; 2596-2657, 3267-3323). Tests (10):
    help_lists_every_command_in_both_spellings, quit_ends_the_session_once_in_both_spellings,
    end_of_input_quits, echo_repeats_each_command_after_the_prompt,
    a_bad_command_reports_and_the_prompt_continues, an_externally_ended_process_is_reported,
    a_listing_that_fails_reports_and_the_prompt_continues, a_bad_list_argument_is_reported,
    empty_listings_print_nothing,
    a_table_aligns_every_column_to_its_widest_entry_by_characters.
  - list func and threads (`list_func_and_threads`, 132; 2659-2728, 3530-3594). Tests (6):
    a_bare_list_shows_what_can_be_listed_in_both_spellings, list_function_is_list_func,
    list_func_groups_by_origin_then_address_and_keeps_repeats,
    list_thread_names_each_thread_by_name_frame_or_id,
    list_func_keeps_the_types_and_names_asked_for, a_glob_matches_the_whole_name.
  - list reg (`list_reg`, 142; 2730-2876). Tests (7):
    list_reg_alone_shows_the_registers_with_a_role_then_its_usage,
    list_reg_alone_shows_only_its_usage_after_the_program_ends,
    list_reg_shows_each_group_of_the_main_thread_or_the_one_asked_for,
    list_reg_shows_the_main_thread_or_the_one_asked_for,
    list_reg_refuses_a_thread_or_register_that_does_not_exist,
    list_reg_shows_just_the_registers_named_by_spaces_or_commas,
    a_negative_64_bit_value_shows_as_a_signed_decimal.
- `inspection.rs` (593, sum of map parts). Tests (21):
  - places and memory (`places_and_memory`, 151; 2878-2963, 3099-3105, 3404-3429, 4299-4335).
    Tests (5): show_mem_parses_an_address_or_a_register_and_offset,
    a_function_and_an_offset_name_a_place, memory_lines_lead_with_full_addresses,
    an_address_may_carry_an_offset_either_way, show_mem_reads_through_memory_in_brackets.
  - errors and types (`errors_and_types`, 129; 2965-3097). Tests (5):
    breakerror_takes_codes_names_any_or_clear,
    show_error_takes_a_thread_and_needs_a_debug_build, show_type_needs_a_debug_build_s_arenas,
    show_type_alone_shows_its_usage, show_type_parses_a_type_a_location_and_a_view.
  - arenas (`arenas`, 170; 3107-3148, 3869-3994, 4337-4345). Tests (8):
    restart_needs_a_launched_program, list_arena_without_a_registry_asks_for_a_debug_build,
    arena_lines_round_up_to_kilobytes_and_mark_unowned_arenas,
    the_arena_state_lists_every_field_and_each_non_empty_entry,
    blocks_print_their_headers_numbered_from_the_oldest,
    a_dump_shows_offsets_bytes_and_printable_text, show_arena_parses_its_forms,
    show_arena_without_a_registry_asks_for_a_debug_build.
  - backtrace and dis (`backtrace_and_dis`, 146; 3352-3402, 3431-3528). Tests (3):
    backtrace_parses_its_forms_in_both_spellings,
    dis_takes_a_count_a_place_or_a_function_and_counts_back_when_negative,
    dis_heads_each_function_s_instructions_with_that_function.
- `execution.rs` (823 = 701 sum of map parts + 122 for the env block measured at HEAD).
  Tests (24):
  - stepping (`stepping`, 112; 3150-3265). Tests (6):
    step_runs_the_main_thread_and_says_where_it_stopped,
    step_reports_an_exception_a_wait_or_the_end_of_the_program, step_runs_the_thread_asked_for,
    step_refuses_a_bad_count_or_thread, step_all_runs_every_thread_and_reports_each,
    step_all_reports_an_exception_or_the_end_of_the_program.
  - breakpoint commands (`breakpoint_commands`, 141; 3325-3350, 3596-3613, 3651-3747).
    Tests (3): breakpoints_are_set_listed_and_removed, a_bad_breakpoint_command_is_refused,
    watchpoint_breakpoint_and_breakerror_take_commands_after_a_semicolon.
  - running (`running`, 152; 3615-3649, 3749-3867). Tests (5): run_reports_each_kind_of_stop,
    a_stop_runs_its_commands_and_the_program_runs_on,
    a_stop_whose_commands_do_not_end_in_run_stays_stopped_and_is_counted,
    run_reports_a_changed_watchpoint_and_a_raised_error, run_reports_an_exit_status.
  - conditional breakpoints (`conditional_breakpoints`, 125; 3996-4120). Tests (2):
    breakpointif_takes_a_test_a_place_and_commands,
    a_breakpoint_with_a_test_is_listed_with_it_and_bp_r_takes_every_one_there.
  - jump (`jump`, 175; 4122-4297). Tests (3):
    jump_moves_the_current_thread_and_sets_its_registers,
    jump_refuses_a_bad_place_register_or_value, a_stop_can_jump_the_stopped_thread_and_run_on.
  - restart environment (added since 17a9b5799 by `df8bbd32c`/`3a17824e5`; HEAD lines
    3238-3366, `python3 -I /tmp/loc/span.py src/debugger/prompt.rs 3238 3366` → 122). Helper:
    `variables` (used by these tests only: `grep -n 'variables(' src/debugger/prompt.rs`).
    Tests (5): env_changes_what_the_next_restart_gives_and_list_env_shows_it,
    env_works_once_the_program_has_ended, env_needs_a_launched_program,
    env_refuses_what_it_cannot_do, env_s_takes_the_rest_of_the_line_as_the_value.

Total 2045 (sum of map parts + measured env block; HEAD moved region 2047).

### 4.5 `src/manifest/package.rs` → `src/manifest/package/tests/` (82 tests; 1177 moved → 2 files)

Citations: 24, all host code. `tests/guards/no_type_strings.rs:215,1763` and
`src/docs/spec/package/01_container-format.md:210` name the host path (unchanged; line numbers
as of 17a9b5799 — plan-176-E may move the guard lines into its parts).
- `mod.rs` (593, sum of map parts: shared 66 + 527): `use super::*;`, `mod edits_and_imports;`;
  shared: `push_field`, `build_mfp`, `header_err`, `write_temp`, `json`, `dependency`,
  `fixture_mfp`. Tests (36; paths unchanged), the `.mfp` reading group:
  - metadata readers (`metadata_readers`, 159; 1073-1296 as of 17a9b5799). Tests (11):
    package_metadata_readers_handle_a_malformed_package_by_contract,
    the_two_mfp_decoders_still_disagree_about_an_empty_ident_by_design,
    only_the_manifest_reader_applies_the_name_charset_guard,
    read_mfp_header_parses_valid_unsigned_package,
    read_mfp_header_rejects_a_name_that_is_not_a_path_component,
    package_dependencies_drops_blank_and_traversing_names, read_mfp_header_rejects_missing_file,
    read_mfp_header_rejects_too_small, read_mfp_header_rejects_bad_magic,
    read_mfp_header_rejects_bad_container_version, read_mfp_header_rejects_empty_required_name.
    Helper: build_mfp_for_both_decoders.
  - mfp header (`mfp_header`, 198; 1298-1456, 1464-1541). Tests (17):
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
  - installed packages and exports (`installed_and_exports`, 173; 1543-1746). Tests (8):
    installed_package_files_empty_when_no_packages,
    installed_package_files_reports_missing_and_pin_mismatch,
    installed_package_files_ok_when_present_and_unpinned,
    installed_package_files_resolves_the_source_package_cache,
    resolved_package_file_prefers_the_installed_mfp,
    source_dependency_classifies_every_source_spelling,
    external_package_function_types_lossy_swallows_errors,
    package_export_signature_decodes_and_renders_signature.
- `edits_and_imports.rs` (583, sum of map parts) — `project.json` edits and imported
  types/resources. Tests (46):
  - export signatures and insert (`export_signatures_and_insert`, 182; 1748-1823, 1840-1976).
    Tests (11): package_export_signature_round_trips_every_export_shape,
    project_json_with_package_inserts_new_array,
    project_json_with_package_appends_to_existing_array,
    project_json_with_package_appends_to_empty_array,
    project_json_with_package_rejects_duplicate,
    project_json_without_packages_removes_at_every_position,
    project_json_without_packages_can_empty_the_array,
    project_json_without_packages_removes_the_only_entry,
    project_json_without_packages_removes_several_at_once,
    project_json_without_packages_preserves_survivor_formatting,
    project_json_without_packages_errors_when_none_match. Helper: three_package_manifest.
  - version and pin updates (`version_and_pin_updates`, 196; 1978-2137, 2148-2236). Tests (20):
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
  - ident key and dependencies (`ident_key_and_dependencies`, 207; 2238-2493). Tests (15):
    project_json_with_updated_ident_key_rejects_malformed_entry,
    read_mfp_string_rejects_invalid_utf8, imported_type_def_maps_a_record_export,
    imported_type_def_maps_a_union_export_with_variant_fields,
    imported_type_def_maps_an_enum_export, imported_type_def_drops_function_exports,
    imported_type_defs_from_files_reads_a_record_export_fixture,
    imported_type_defs_from_files_skips_unreadable_packages,
    imported_type_defs_empty_when_no_packages,
    imported_type_defs_reads_from_an_installed_package,
    imported_resource_closers_skips_builtin_backed_resources,
    imported_resource_closers_registers_a_native_resource,
    imported_resource_closers_empty_when_packages_unreadable,
    common_dependency_fields_rejects_a_non_string_name,
    external_package_function_types_lossy_skips_a_corrupt_body. Helpers: type_field,
    type_export, owned.

Total 1176 (sum of map parts; HEAD moved region 1177 − 3 = 1174).

### 4.6 `src/audit/report.rs` → `src/audit/report/` (4 tests)

`report.rs` holds two cfg(test) modules (HEAD lines): `pub(super) mod testsupport` (255-515,
256 LOC with its wrapper: `finding`, `full_report` (one 210-LOC fn), `empty_report`; imported by
`src/audit/json.rs` and `src/audit/text.rs` as `super::super::report::testsupport::*`) and
`mod tests` (517-565, 45).
- Host keeps `#[cfg(test)] pub(super) mod testsupport;` and `#[cfg(test)] mod tests;`.
- `src/audit/report/testsupport/mod.rs` (253): the unchanged body of `mod testsupport`.
- `src/audit/report/tests/mod.rs` (42): the unchanged body of `mod tests`. Tests (4):
  severity_as_str_covers_all_variants, counts_tallies_each_severity,
  counts_empty_report_is_zero, category_rank_orders_known_and_unknown.

The three fns are already `pub(in crate::audit)`, so `report::testsupport::full_report` keeps
resolving for `json.rs`/`text.rs` unchanged. Citations 0 test-side.

### 4.7 Whole-module moves (27 hosts)

Each host's inline `#[cfg(test)] mod tests { … }` (HEAD range in §2) becomes
`#[cfg(test)] mod tests;` and its body moves verbatim to `tests/mod.rs` (LOC in §2: moved − 3).
Every test keeps its path; no `use` line changes. Per host, what stays behind and what names
the path (citation counts: `grep -rnoE "\[\[<host>:[^]]+\]\]" src/docs/spec`; other refs:
`grep -rn "<host>" tests/guards scripts .ai AGENTS.md src/docs .github Cargo.toml`; line numbers
of other refs as of 17a9b5799):

Optimizer and audit:
- `src/optimizer/opt1/globals.rs` → `src/optimizer/opt1/globals/tests/mod.rs` (329; 12 tests).
  Citations: 1, host code.
- `src/optimizer/opt1/inline.rs` → `src/optimizer/opt1/inline/tests/mod.rs` (267; 7 tests).
  Citations 0.
- `src/optimizer/opt2/promote.rs` → `src/optimizer/opt2/promote/tests/mod.rs` (283; 10 tests).
  The cfg(test) `visited` field and its initializer (228-232, 259-269 as of 17a9b5799) stay in
  the host. Citations: 1, host code. The moved code carries `"%vN"` literals and
  `.physical_field(` (§2): it relies on plan-176-H's `/tests/` guard rule (Prerequisites).
- `src/optimizer/opt2/sink.rs` → `src/optimizer/opt2/sink/tests/mod.rs` (297; 12 tests).
  Citations 0. Same guard dependency as promote (`.physical_field(` at old :526).
- `src/audit/collect/findings.rs` → `src/audit/collect/findings/tests/mod.rs` (291; 10 tests).
  Citations: 6, host code. (`src/audit/collect/dependencies/` is an unrelated sibling.)
- `src/audit/collect/project.rs` → `src/audit/collect/project/tests/mod.rs` (339; 10 tests).
  Citations: 6, host code.
- `src/audit/collect/source.rs` → `src/audit/collect/source/tests/mod.rs` (661; 26 tests). The
  `#[cfg(test)] fn in_inline_census` (1021-1026 as of 17a9b5799) stays in the host. Citations:
  15, host code; `src/docs/spec/language/08_error-model.md:157` names the host (unchanged).

Frontend:
- `src/ast/scope_privates.rs` → `src/ast/scope_privates/tests/mod.rs` (387; 20 tests).
  Citations: 2, host code. `.ai/codegen-invariants.md:463` names the host (unchanged).
- `src/lexer.rs` → `src/lexer/tests/mod.rs` (552; 35 tests). No `src/lexer/` directory today;
  K creates it with only `tests/`. Citations: 14, host code (the two stale line cites are M's;
  §2). `tests/guards/no_type_strings.rs:78` (`rel == "src/lexer.rs"`) scopes the lexer's code
  and is untouched by K: the moved tests sit under `src/lexer/tests/`, which `is_test_file`
  exempts (`/tests/`).
- `src/fmt.rs` → `src/fmt/tests/mod.rs` (572; 53 tests). Citations: 34, host code;
  `src/docs/spec/architecture/09_modules.md:19` names the host.
- `src/numeric.rs` → `src/numeric/tests/mod.rs` (790; 33 tests). The 8 other cfg(test) items
  (`TYPE_*` consts, `binary_result_type`, `numeric_variant`, `numeric_variant_name`; 42 LOC)
  stay in the host. Citations: 11, host code. Path refs `tests/guards/no_operator_strings.rs:78`,
  `.ai/codegen-invariants.md:7`, `src/docs/spec/architecture/09_modules.md:69`,
  `src/docs/spec/architecture/22_type-inference.md:327` name the host (unchanged).
- `src/types.rs` → `src/types/tests/mod.rs` (989; 27 tests). The cfg(test)
  `pub(crate) const ALL` (19 LOC) stays in the host. Citations: 22, host code. Path refs in
  `tests/guards/no_type_strings.rs` (lines 3, 43, 180, 218, 230, 572, 808, 812, 832, 935, 1718,
  1721, 1724, 1751 at `17a9b5799`; plan-176-E may move some into its parts),
  `.ai/resources-packages.md:295`, `.ai/codegen-invariants.md:20,58,736,804` name the host or
  its code (unchanged). `no_type_strings` lists `src/types.rs` as a BOUNDARY file
  (`rel == "src/types.rs"`); the host code stays there and `src/types/tests/mod.rs` contains
  `/tests/`, so `is_test_file` exempts it. `split_state_matches_the_name_domain_helpers` is
  named by fn name only (`no_type_strings.rs:927`) — stable. The golden-corpus scan (old :1695)
  joins `env!("CARGO_MANIFEST_DIR")`. **Near the limit:** re-measure first; if the moved body is
  > 1000, split it at the map's seams into `tests/mod.rs` (shared `round_trip` + c_abi_and_names
  170 + variants_and_round_trips 173 + state_split 150) and `tests/structure.rs`
  (hash_and_stateful 186 + structural_state 163 + prefix_and_display 145).
- `src/rules/mod.rs` → `src/rules/tests/mod.rs` (250; 20 tests). The other cfg(test) items
  (`thread_local!`, `collecting_diagnostics`, a block, `code_and_name`; 30 LOC) stay in the host.
  `src/rules/` already holds `table.rs`; no collision. Citations: 27, host code.
  `scripts/diag-set-diff.sh:67` names the host (unchanged).
  `include_str!(concat!(env!("CARGO_MANIFEST_DIR"), …))` (old :560) is absolute.
  `rules::tests::rule_codes_are_unique` (spec `01_rule-codes.md:103`, `bugs/bug-883-…:43`)
  keeps its path.
- `src/resolver/mod.rs` → `src/resolver/tests/mod.rs` (619; 28 tests). The cfg(test)
  `pub fn resolve_hir_project` and `augment_hir_project` stay in the host. Citations: 13, host
  code. Path refs `tests/guards/no_type_strings.rs:102,104,487`,
  `.ai/resources-packages.md:246,247,375` name host code (unchanged).
  `is_c_abi_type_recognizes_and_rejects` is named by fn name only
  (`.ai/resources-packages.md:322`, a comment in `src/resolver/mod.rs`) — stable.
- `src/resolver/packages.rs` → `src/resolver/packages/tests/mod.rs` (277; 20 tests).
  Citations: 3, host code.
- `src/resolver/resolution.rs` → `src/resolver/resolution/tests/mod.rs` (734; 57 tests).
  Citations: 27, host code.
- `src/doc/html.rs` → `src/doc/html/tests/mod.rs` (384; 16 tests). Citations: 13, host code.

Manifest and CLI:
- `src/manifest/config.rs` → `src/manifest/config/tests/mod.rs` (286; 19 tests). Citations: 8,
  host code.
- `src/manifest/libraries.rs` → `src/manifest/libraries/tests/mod.rs` (377; 19 tests).
  Citations: 5, host code.
- `src/manifest/mod.rs` → `src/manifest/tests/mod.rs` (942; 71 of 73 tests). The other
  cfg(test) module, `mod canvas_auto_paint_tests { … }` (171-234, 57 LOC, 2 tests), stays
  inline (host inline after: 59). Citations: 38, host code; `.ai/build-tooling.md:84` names
  host code (plan-176-L repoints it).
- `src/cli/man.rs` → `src/cli/man/tests/mod.rs` (968; 49 tests). Citations: 7, host code.
  Path refs `scripts/man-census.sh:5,517`, `scripts/man-manual.sh:10`,
  `.ai/man-content.md:34,408,443,493` name host code (`render_all_markdown`,
  `render_topic_overview`, lines 1-15 and 473) — unchanged.
- `src/cli/pkg.rs` → `src/cli/pkg/tests/mod.rs` (893; 60 tests). `src/cli/pkg/` exists and holds
  only `command_tests.rs`. The other cfg(test) item, `#[path = "pkg/command_tests.rs"] mod command_tests;`
  (36-38), stays; plan-176-L rewrites it when the host becomes `pkg/mod.rs`. Citations: 38, host
  code. `scripts/coverage-exceptions.txt:17` names the host (unchanged in K).
- `src/cli/resolve.rs` → `src/cli/resolve/tests/mod.rs` (597; 27 tests). Citations: 20, host
  code. `scripts/coverage-exceptions.txt:18` names the host (unchanged).

Debugger:
- `src/debugger/breakpoints.rs` → `src/debugger/breakpoints/tests/mod.rs` (996; 22 tests).
  `src/debugger/breakpoints/` exists with `report.rs` only. Citations: 11, host code;
  `src/docs/spec/tooling/10_debugger.md:69` names the host (plan-176-L changes it). **Near the
  limit** (grew 945 → 999 since 17a9b5799 with 2 watchpoint tests at HEAD lines 2243-2303, 52
  LOC): re-measure first; if the moved body is > 1000, split it at the map's seams into
  `tests/mod.rs` (shared `Program`, `MEMORY`, `PARK`, `RAISER`, `ENTRY`, `STORE`, `IS_PRIME`,
  `EXIT`, `ABORT`, `exit`, `abort`, `RET`, `function`, `impl Program`, `name`, `run`; the fake
  `impl Inferior for Program`; places; breakerror_and_entry — 585, sum of map parts) and
  `tests/watch_and_conditions.rs` (watchpoints_and_commands 156 + the 2 new tests 52 +
  conditions 204 − 1 = 411; helper `test`).
- `src/debugger/breakpoints/report.rs` → `src/debugger/breakpoints/report/tests/mod.rs` (432; 12
  tests). Citations: 5, host code.
- `src/debugger/runtime.rs` → `src/debugger/runtime/tests/mod.rs` (407; 14 tests). Citations: 5,
  host code; `src/docs/spec/tooling/10_debugger.md:67` names the host (unchanged).
- `src/debugger/values.rs` → `src/debugger/values/tests/mod.rs` (498; 13 tests). Citations: 4,
  host code.

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
  `grep -cE '^  (NEW|GREW|STALE|TIGHTEN)'` → `0`. A host that is over 1000 code keeps its
  `ceiling code` row unchanged (K moves no code). Commit `tests/guards/file_size_ceilings.txt`
  with the phase (est. 2 min).
- **G5:** `cargo fmt --check` → no output (est. 20 s).

Every move task first re-measures its moved body against §2 (`python3 -I /tmp/loc/rw-K/mods.py
<host>` or `python3 -I tools/loc-census/census.py --file <host>`); a body that is now > 1000
is split per its §4 entry instead of moved whole.

### Phase 1 — Optimizer and audit tests

Eight hosts with self-contained NIR / audit fixtures; nothing outside them imports their tests
(except `report::testsupport`, whose path is unchanged).

- [x] Move `src/optimizer/opt1/globals.rs` tests per §4.7.
- [x] Move `src/optimizer/opt1/inline.rs` tests per §4.7.
- [x] Move `src/optimizer/opt2/promote.rs` tests per §4.7.
- [x] Move `src/optimizer/opt2/sink.rs` tests per §4.7.
- [x] Move `src/audit/collect/findings.rs` tests per §4.7.
- [x] Move `src/audit/collect/project.rs` tests per §4.7.
- [x] Move `src/audit/collect/source.rs` tests per §4.7.
- [x] Move `src/audit/report.rs` `testsupport` and `tests` per §4.6.
      Done (all eight): `python3 -I target/p176/movemod.py` (finds each module by name, then `items.py extract`) →
      12, 7, 10, 12, 10, 10, 26 tests and `report`'s 4 + `testsupport` (kept `pub(super) mod testsupport;`);
      `bash target/p176/motiondir.sh` → only the module lines differ.
- [x] G4: tighten the ceilings for the eight files.
      Done: → 8 TIGHTEN `inline` rows (2, 2, 7, 4, 2, 2, 10, 2); tighten drops exactly those; re-run → 0.

Acceptance: the eight files' tests run from their new modules and the counts did not move.
  Check: G1 → equal; G2 `cargo test -p mfb --bin mfb optimizer:: audit::` → 0 failed, and
  `cargo test -p mfb --bin mfb -- --list 2>/dev/null | grep -cE '^(optimizer::opt1::(globals|inline)|optimizer::opt2::(promote|sink)|audit::(collect::(findings|project|source)|report))::tests::'`
  → 12+7+10+12+10+10+26+4 = 91, the same before and after (paths are unchanged); G3 pass (the
  promote/sink files are exempt by the H rule); G4; G5 (est. 12 min).
  Verified 2026-10-19: G1 (`bash target/p176/g1.sh`) → 5017 = 5017, leaves SAME, warnings 0; G2 → `ok. 437 passed`;
  path count → 91 before and 91 after; census → hosts inline 2, 2, 10, 2, 2, 2, 7, 4 and new files 329, 267, 283, 297,
  291, 339, 661, 42, 253 (each exactly §2); G3 fast tier → `test-fast: ok` (unit `4998 passed; 2 ignored`; guards 8,
  7 + 1 ignored, 8); G4 → 0; G5 → exit 0.
Commit: c991d80c9

### Phase 2 — Frontend: AST, lexer, fmt, numeric, types, rules, resolver, doc

- [x] Split `src/ast/tests.rs` per §4.1 (delete the file; create `src/ast/tests/`, 4 files).
      Done: `git mv` to `tests/mod.rs`, `planparts.py --plain` (§4.1's own lists) + `items.py extract` → mod.rs keeps
      37 tests, parts 65, 12, 52; motion → only the 3 part `mod` lines differ.
- [x] Move `src/ast/scope_privates.rs` tests per §4.7.
- [x] Move `src/lexer.rs` tests per §4.7.
- [x] Move `src/fmt.rs` tests per §4.7.
- [x] Move `src/numeric.rs` tests per §4.7.
- [x] Move `src/types.rs` tests per §4.7 (re-measure; split per §4.7 if > 1000).
      Done: re-measured → `src/types/tests/mod.rs` 988 ≤ 1000, so one file (no split).
- [x] Move `src/rules/mod.rs` tests per §4.7.
- [x] Move `src/resolver/mod.rs` tests per §4.7.
- [x] Move `src/resolver/packages.rs` tests per §4.7.
- [x] Move `src/resolver/resolution.rs` tests per §4.7.
- [x] Move `src/doc/html.rs` tests per §4.7.
      Done (the ten whole moves): `movemod.py` → 20, 35, 53, 33, 27, 20, 28, 20, 57, 16 tests; `motiondir.sh` → only
      the module lines differ.
- [x] G4: tighten (every row naming one of the phase's eleven files is TIGHTEN or STALE).
      Done: → 1 STALE (`src/ast/tests.rs`) + 10 TIGHTEN `inline` rows; tighten drops exactly those eleven; re-run → 0.

Acceptance: the frontend tests run from their new modules; the spec still resolves.
  Check: G1 → equal; G2 `cargo test -p mfb --bin mfb ast:: lexer:: fmt:: numeric:: types:: rules:: resolver:: doc::html`
  → 0 failed;
  `cargo test -p mfb --bin mfb -- --list 2>/dev/null | grep -cE '^(ast|ast::scope_privates|lexer|fmt|numeric|types|rules|resolver|resolver::packages|resolver::resolution|doc::html)::tests::'`
  → 475, the same before and after; `cargo test -p mfb --bin mfb rules::tests::rule_codes_are_unique`
  → 1 passed (the spec-named path still resolves); `cargo test -p mfb --bin mfb spec_citations_resolve`
  → passes; G3; G4; G5 (est. 12 min).
  Verified 2026-10-19: G1 → 5017 = 5017, leaves SAME, warnings 0; G2 → `ok. 579 passed`; path count → 475 before and
  475 after; `rules::tests::rule_codes_are_unique` → `ok. 1 passed`; census → ast 707/610/391/672, new files 387, 550,
  571, 785, 988, 249, 610, 276, 734, 384, hosts inline 2, 2, 2, 44, 21, 32, 11, 2, 2, 2; G3 fast tier → `test-fast: ok`
  (unit `4998 passed; 2 ignored`, which includes `spec_citations_resolve`; guards 8, 7 + 1 ignored, 8); G4 → 0; G5 → 0.
Commit: 164268d8f

### Phase 3 — Manifest and CLI

- [x] Move `src/manifest/config.rs` tests per §4.7.
- [x] Move `src/manifest/libraries.rs` tests per §4.7.
- [x] Move `src/manifest/mod.rs` `mod tests` per §4.7 (`canvas_auto_paint_tests` stays inline).
- [x] Split `src/manifest/package.rs` tests per §4.5 (2 files).
      Done: `planparts.py --plain` + `items.py extract` → mod.rs 36 tests, `edits_and_imports.rs` 46; `json` (a shared
      helper the `--plain` scan picked up from "`project.json`" in the part's prose) taken back out of the part.
- [x] Split `src/cli/build/mod.rs` tests per §4.3 (4 files; `builtin_codegen_corpora_lower_in_process`
      in `mod.rs`).
      Done: → mod.rs 30 tests (`builtin_codegen_corpora_lower_in_process` among them), options 44, signing 29, outputs 39.
- [x] Move `src/cli/man.rs` tests per §4.7.
- [x] Move `src/cli/pkg.rs` tests per §4.7 (the `#[path = "pkg/command_tests.rs"]` declaration
      stays as is).
- [x] Move `src/cli/resolve.rs` tests per §4.7.
      Done (the six whole moves): `movemod.py` → 19, 19, 71 (`canvas_auto_paint_tests` stays inline), 49, 60 (the
      `#[path = "pkg/command_tests.rs"]` pair stays), 27 tests; motion → only the module lines differ.
- [x] Split `src/cli/info/tests.rs` per §4.2 (delete the file; create `src/cli/info/tests/`,
      2 files).
      Done: `git mv` + `planparts.py --plain` (the `impl` blocks follow their types) → mod.rs 10 tests, `signed.rs` 13;
      `info.rs`'s `#[path = "info/tests.rs"]` line removed, so `mod tests;` resolves to `info/tests/mod.rs`.
- [x] `scripts/test-accept.sh:275`: replace the stale `src/cli/build/mod.rs:1488` with
      `` `impl Default for OptLevel` in `src/optimizer/mod.rs` `` (pre-existing defect, §2;
      `grep -n 'build/mod.rs:1488' scripts/test-accept.sh` → 0 after).
      Done: → 0 hits; `bash -n scripts/test-accept.sh` → exit 0 (Corrections: `sh -n`).
- [x] G4: tighten (every row naming one of the phase's nine files is TIGHTEN or STALE).
      Done: → 1 STALE (`src/cli/info/tests.rs`) + 8 TIGHTEN `inline` rows; tighten drops exactly those nine; re-run → 0.

Acceptance: the manifest and CLI tests run from their new modules.
  Check: G1 → equal; G2 `cargo test -p mfb --bin mfb manifest:: cli::` → 0 failed;
  `cargo test -p mfb --bin mfb -- --list 2>/dev/null | grep -cE '^(manifest|manifest::(config|libraries|package)|cli::(build|man|pkg|resolve|info))::tests::'`
  → 71+19+19+82+142+49+60+27+23 = 492, the same before and after;
  `cargo test -p mfb --bin mfb -- --list 2>/dev/null | grep -c '^cli::build::tests::builtin_codegen_corpora_lower_in_process: test$'`
  → 1 (the `scripts/test-fast-skips.txt:21` entry still matches); `sh -n scripts/test-accept.sh`
  → exit 0; G3; G4; G5 (est. 15 min; `cli::build` tests build real executables).
  Verified 2026-10-19: G1 → 5017 = 5017, leaves SAME, warnings 0; G2 → `ok. 650 passed`; path count → 491 before and
  491 after (Corrections); `builtin_codegen_corpora_lower_in_process` path → 1; census → hosts inline 2, 2, 59, 2, 10,
  2, 5, 2, 2, new files 283, 377, 936, 591/576, 825/503/529/876, 963, 891, 595, 390/671; G3 fast tier → `test-fast: ok`
  (its skip list still matches; unit `4998 passed; 2 ignored`; guards 8, 7 + 1 ignored, 8); G4 → 0; G5 → exit 0.
Commit: —

### Phase 4 — Debugger

- [ ] Move `src/debugger/breakpoints.rs` tests per §4.7 (re-measure; split per §4.7 if > 1000).
- [ ] Move `src/debugger/breakpoints/report.rs` tests per §4.7.
- [ ] Split `src/debugger/prompt.rs` tests per §4.4 (3 files).
- [ ] Move `src/debugger/runtime.rs` tests per §4.7.
- [ ] Move `src/debugger/values.rs` tests per §4.7.
- [ ] G4: tighten.

Acceptance: the debugger tests run from their new modules.
  Check: G1 → equal; G2 `cargo test -p mfb --bin mfb debugger::` → 0 failed (the fakes
  `Program`, `Exiting`, `Recorder`, `Memory`, `Fake` drive every test; no live process);
  `cargo test -p mfb --bin mfb -- --list 2>/dev/null | grep -cE '^debugger::(breakpoints|breakpoints::report|prompt|runtime|values)::tests::'`
  → 22+12+68+14+13 = 129, the same before and after; G3; G4; G5 (est. 8 min).
Commit: —

## Validation Plan

- Tests: no test is added or removed. Per phase, the whole-binary `--list` count is unchanged
  (G1), the per-phase prefix count is unchanged (91 + 475 + 492 + 129 = 1187 moved tests), and
  the moved modules' tests pass (G2).
- Coverage check: per-file coverage is not a plan-176 gate — author, 2026-10-17. Evidence: at 7a4b09970,
  `sh scripts/coverage.sh --bins` in a detached worktree (59 min; 4989 unit tests passed), then
  `sh scripts/coverage-check.sh` exited 1. 241 `src/` files are already below the 98% per-file
  floor ("Files below 98% line coverage (GATE FAILURE)"), and overall coverage is 93.67%.
  Examples among plan-176 hosts: `src/ir/shape.rs` 93.21%,
  `src/codegen/engine/control/builder_control.rs` 75.55%, `src/debugger/values.rs` 86.66%. A
  pure move changes no executed line, and moving an inline test block only removes test lines
  from its host's denominator. The pre-existing gate failure is reported to the owner
  separately and is not a plan-176 task.
- Doc sync: `scripts/test-accept.sh` (stale line ref). No spec test path changes (the rules
  module moves whole). No `mfb man` topic changes. No compiler contract changes, so no spec
  topic changes.
- Final gate (once, after Phase 4): `cargo test -p mfb --bin mfb spec_citations_resolve`,
  `cargo test --test architecture_guards --test no_type_strings --test no_operator_strings --test file_size_budget`
  → all pass and the banner has no `inline`/`test` row for a K path;
  `cargo clippy --all-targets 2>&1 | grep -c '^warning'` → equal to its value at G0 of Phase 1
  (est. 6 min). The full workspace suite runs once, in plan-176-V.

## Decisions

- Rule revised — owner, 2026-10-18. Three kinds, three limits, all non-comment, non-blank LOC
  (counting rules unchanged from plan-176-A §2); scope `.rs`/`.mfb` under `src/` `packages/`
  `examples/` `repository/` `tests/`:

  | Kind | Applies to | Limit | Over the limit → |
  |---|---|---|---|
  | `code` | code LOC of a file that is not a test-only file | 1000 | split the file into modules |
  | `inline` | inline test LOC (`#[cfg(test)]` items / MFBASIC `TESTING … END TESTING`) inside a file that is not a test-only file | 250 | move the tests out into their own test-only file(s) |
  | `test` | total LOC of a test-only file (path component `tests`, `test_*.mfb`, `#![cfg(test)]`, or a `#[cfg(test)] mod x;` target and its subtree) | 1000 | split the test-only file into modules |

  Ceiling rows are `ceiling <code|inline|test> <n> <path>`. There are no exemptions; generated
  files (`GENERATED` banner) are the only automatic exemption. The guard warns with a banner and
  never fails. A new module is always an acceptable split.
- A moved inline block becomes ONE `foo/tests/mod.rs` unless it is over 1000; over 1000 it is
  split into parts of ≤ 900 (headroom) — owner, 2026-10-18. K: 28 single files, 5 split hosts.
- A split host's `tests/mod.rs` holds the shared helpers plus the first group of tests; parts
  merge neighbouring split-map topics, never new seams — author, 2026-10-18. Keeps the most
  test paths unchanged and puts the harness next to the tests that use it most.
- `cli::build`'s `builtin_codegen_corpora_lower_in_process` stays in `cli/build/tests/mod.rs` —
  author, 2026-10-18. `scripts/test-fast-skips.txt:21` names its full path as a `--skip`
  substring and `scripts/test-fast.sh` fails on an entry that matches nothing.
- Extracted src test modules go into `foo/tests/mod.rs` (+ part files), never `foo/tests.rs` —
  plan-176-A (coverage `IGNORE` and the guards recognise the `tests/` directory). For
  `src/audit/report.rs`'s second module this is `report/testsupport/mod.rs`.
- K edits no guard. The `/tests/` rule in `architecture_guards` is plan-176-H's and is a K
  prerequisite.
- Pre-existing defects found by the map are fixed in the letter that touches them — author,
  2026-10-14. K fixes `scripts/test-accept.sh:275`. The stale `[[src/lexer.rs:262]]` /
  `[[src/lexer.rs:752]]` cites are plan-176-M's: they point at lexer code that M moves.
- Glob-cycle imports (`use super::*;` in parts, `mod part;` in `mod.rs`) compile without
  warnings — verified (`/tmp/loc/probe2`).

## Corrections

- 2026-10-18 — Rule revised by the owner: test-only files limit 1000 (was 250), inline tests
  250, no exemptions. Dropped 3 files: `src/cli/dbg/tests.rs` (364), `src/optimizer/opt1/placefwd/tests/mod.rs`
  (916), `src/testutil.rs` (827) — all ≤ 1000 test-only files. With `src/testutil.rs` went the
  old Phase 5, its 3 `pub(super)` edits and all 3 guard edits (panic-hook owner, two
  `is_test_file` clauses); with placefwd went the `render_ops` visibility edit. 28 of the 33
  kept hosts now move whole into one `tests/mod.rs` (test paths unchanged); only
  `src/ast/tests.rs`, `src/cli/info/tests.rs`, `src/cli/build/mod.rs`, `src/debugger/prompt.rs`
  and `src/manifest/package.rs` split, into ≤ 900 parts that merge the old topics. Files
  created: 179 → 44. The `rules::tests::rule_codes_are_unique` spec/bug sweep is no longer
  needed (path unchanged). New since the 17a9b5799 map: 7 tests (2 in `breakpoints.rs`, 5 `env`
  tests in `prompt.rs`, placed in §4.7/§4.4); the stale `test-accept.sh` cite is now line 275;
  found `scripts/test-fast-skips.txt:21`, kept valid by placement (§4.3). K had no exemption
  before or after. Counts re-measured at HEAD `ce7a995ca`: 33 hosts, 1189 tests.
- 2026-10-19 (execution): G0 is the full sorted `--list` in `target/p176/g0/` (`/tmp` is wiped by
  session restarts here); G1 compares count and leaf names. G2/G3 run through
  `bash scripts/test-fast.sh` (owner, 2026-10-18), on macOS only (owner, 2026-10-19).
- 2026-10-19 (execution): Phase 3's count is 491, not 492: `manifest::package` has 82 tests in
  source, but one is `#[cfg(windows)]`, so 81 list on macOS (G0's list → 81). `src/cli/info.rs`
  declared its tests with `#[path = "info/tests.rs"]`; the move removes that line. Phase 3's
  `sh -n scripts/test-accept.sh` check cannot pass: the script is bash (`#!/usr/bin/env bash`,
  process substitution at :786) and `sh -n` fails on HEAD's copy too; `bash -n` → exit 0.

## Summary

K is pure motion of test code into `tests/` directory modules: 33 hosts, 1189 tests (1187
moved), 0 renamed, no visibility or guard edit. 28 hosts move their test module whole, so their
test paths do not change; 5 hosts over 1000 split into 15 files of ≤ 900, and 345 of their
tests gain one path segment. The risk is the promote/sink dependency on plan-176-H's `/tests/`
guard rule and the two whole-file moves sitting just under 1000 (`breakpoints` 996, `types`
989), each with a re-measure step and a pre-planned split. Host code, host paths and every spec
citation stay where they are; letters L and M split the code next.
