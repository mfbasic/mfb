# plan-176-H: Codegen builtins, registry and engine test code

Last updated: 2026-10-14
Effort: large (3h–1d)
Depends on: plan-176-G

H moves the over-budget test code under `src/codegen/builtins`, `src/codegen/registry` and
`src/codegen/engine` into `tests/` directory modules, split by topic so that no new file is
over 250 test LOC. 37 files are in scope: 24 whole test files (most of them suites of the
`builtins_codegen` module) and 13 code files whose inline `#[cfg(test)]` modules are over 250.
No test is added, removed or renamed; only module paths gain a topic segment
(`builtins_codegen::canvas::foo` → `builtins_codegen::canvas::publish_bodies::foo`). Host code
does not move; its splits are letters M, R–U.

H also lands the one guard change the later test letters depend on: the three file-name
exemptions in `tests/guards/architecture_guards.rs` learn that a `tests` directory component
means test code, in the same commit as the first move that needs it (regalloc and MIR tests).

The single outcome: after H lands, `cargo test --test file_size_budget` shows no row for any of
H's 37 files or the files H creates, and `tests/guards/file_size_ceilings.txt` has no `ceiling`
row for them — only the three `exempt` rows H adds (§4.0).

References:

- `planning/plan-176-A-file-size-guard.md` — the rule, §Decisions, the counting rules, and
  "Splitting an over-budget file" in `.ai/build-tooling.md` (src tests: `foo/tests/mod.rs` +
  topic files).
- `.ai/testing-gates.md` § "Splits must sweep man AND spec citations".
- `.ai/codegen-invariants.md` — register and vreg invariants the architecture guards enforce
  (bug-721).
- `.ai/resources-packages.md` — builtin-package authoring seams (names
  `spec_section_18_package_list_matches_is_builtin_import` without a path; unchanged).
- `.ai/man-content.md` — names `range_and_index_parameters_use_the_documented_vocabulary`
  with the path `src/codegen/registry/mod.rs` (repointed in Phase 5).
- `.ai/build-tooling.md` — rustfmt/clippy policy.
- Spec topics whose citations move: `src/docs/spec/app/06_canvas.md` (3),
  `src/docs/spec/language/18_builtin-functions.md` (2), `src/docs/spec/tooling/04_audit-format.md`
  (1), `src/docs/spec/stdlib/02_datetime.md` (1).
- Guards: `tests/guards/architecture_guards.rs` (root file after plan-176-E's split),
  `tests/guards/no_type_strings.rs`, `tests/guards/no_operator_strings.rs`.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-176-G is complete and archived | `ls planning/plan-176-G-* planning/completed/plan-176-G-*` → only `planning/completed/…` | NOT MET (2026-10-14) |
| The file-size guard exists and passes | `cargo test --test file_size_budget` → passes | NOT MET (2026-10-14) |
| H's files and the docs/guards H edits have no uncommitted changes | `git status --porcelain -- src/codegen/builtins src/codegen/registry src/codegen/engine src/docs/spec/app/06_canvas.md src/docs/spec/language/18_builtin-functions.md src/docs/spec/tooling/04_audit-format.md src/docs/spec/stdlib/02_datetime.md .ai/man-content.md tests/guards/architecture_guards.rs tests/guards/architecture_guards` → empty | NOT MET (2026-10-14): ` M src/codegen/registry/mod.rs`, ` M src/docs/spec/app/06_canvas.md` |

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

- Every file in §2's table is ≤ 250 test LOC, and every file H creates is ≤ 250 test LOC
  except the three exempt files of §4.0
  (`python3 -I tools/loc-census/census.py --file <path>` → second field ≤ 250).
- `cargo test --test file_size_budget` prints no row naming an H path; the ceiling file has no
  `ceiling` row for an H path and exactly three new `exempt` rows.
- The unit-test binary lists the same number of tests before and after each phase.
- `tests/guards/architecture_guards.rs` treats a path with a `tests` directory component as
  test code in `is_physical_scan_exempt`, `no_hand_picked_vreg` and `emission_lines`.

### Non-goals (explicit constraints)

- No behavior change. No test body edit beyond moving it (dedented when it leaves an inline
  `mod … { }`), a leading `use super::*;`, and the visibility edits listed in §4.
- No `#[test]` fn renamed, added or removed. No `[[test]]` target renamed (H touches none).
- No host code moves. Every code file keeps its path and its non-test items; code splits are
  letters M (registry), R–T (builtins), U (engine).
- Every existing module name is kept: `schannel_tests`, `error_path_release_tests`,
  `codepage_tables`, `tcp_tls_error_declaration_tests`, `raw_result_block_ownership`,
  `builtins_codegen`, `tests`. Where the name is not `tests`, a `#[path]` puts its file under a
  `tests/` directory.
- No guard weakens: no production file becomes exempt from any guard (§4.1 proof).
- The `builtins_codegen` wiring in `src/codegen/engine/tests/mod.rs` (declared there for the
  coverage reason its comment documents) is untouched.

## 2. Current State

LOC: `/tmp/loc/groups.json` (planning census, `python3 -I /tmp/loc/census.py "$PWD"` on the
working tree of `17a9b5799`); after plan-176-A the same number comes from
`python3 -I tools/loc-census/census.py --file <path>`. Tests:
`grep -cE '#\[(tokio::)?test\]' <file>`. Re-measured now with `python3 -I /tmp/loc/span.py
<file>`: every row agrees except `src/codegen/builtins/encoding/mod.rs` (488 now, 490 in
groups.json).

| File | Kind | Test LOC | Tests |
|---|---|---|---|
| `src/codegen/builtins/canvas/helper_render/tests/mod.rs` | test file | 276 | 11 |
| `src/codegen/builtins/tests/canvas.rs` | test file | 446 | 10 |
| `src/codegen/builtins/tests/collections.rs` | test file | 418 | 11 |
| `src/codegen/builtins/tests/corpus.rs` | test file | 766 | 2 |
| `src/codegen/builtins/tests/declared_errors_all.rs` | test file | 780 | 23 |
| `src/codegen/builtins/tests/declared_errors_sweep.rs` | test file | 493 | 0 |
| `src/codegen/builtins/tests/diagnostics.rs` | test file | 569 | 4 |
| `src/codegen/builtins/tests/general_declared_errors.rs` | test file | 311 | 1 |
| `src/codegen/builtins/tests/inplace.rs` | test file | 306 | 7 |
| `src/codegen/builtins/tests/inplace_compact.rs` | test file | 293 | 11 |
| `src/codegen/builtins/tests/mfb_body_declared_errors.rs` | test file | 1855 | 2 |
| `src/codegen/builtins/tests/nir_fault.rs` | test file | 721 | 1 |
| `src/codegen/builtins/tests/nir_json.rs` | test file | 325 | 6 |
| `src/codegen/builtins/tests/nir_validation.rs` | test file | 813 | 11 |
| `src/codegen/builtins/tests/optimizer_globals.rs` | test file | 325 | 7 |
| `src/codegen/builtins/tests/os.rs` | test file | 372 | 8 |
| `src/codegen/builtins/tests/plan_validation.rs` | test file | 293 | 4 |
| `src/codegen/builtins/tests/platform_failure.rs` | test file | 1088 | 1 |
| `src/codegen/builtins/tests/platform_hooks.rs` | test file | 450 | 7 |
| `src/codegen/builtins/tests/validation.rs` | test file | 337 | 9 |
| `src/codegen/builtins/tls/gen_macos/tests.rs` | test file | 884 | 23 |
| `src/codegen/builtins/tls/gen_schannel_tests.rs` | test file | 392 | 11 |
| `src/codegen/engine/regalloc/tests.rs` | test file | 721 | 16 |
| `src/codegen/engine/tests/tests.rs` | test file | 404 | 16 |
| `src/codegen/builtins/audio/mod.rs` | inline | 296 | 11 |
| `src/codegen/builtins/canvas/mod.rs` | inline | 408 | 14 |
| `src/codegen/builtins/crypto/mod.rs` | inline | 515 | 14 |
| `src/codegen/builtins/datetime/mod.rs` | inline | 366 | 11 |
| `src/codegen/builtins/encoding/mod.rs` | inline | 490 | 16 |
| `src/codegen/builtins/general/mod.rs` | inline | 572 | 29 |
| `src/codegen/builtins/mod.rs` | inline | 1102 | 32 |
| `src/codegen/builtins/tls/gen_openssl.rs` | inline | 301 | 9 |
| `src/codegen/engine/analysis/borrow_get.rs` | inline | 522 | 27 |
| `src/codegen/engine/analysis/handover.rs` | inline | 560 | 10 |
| `src/codegen/engine/analysis/last_use.rs` | inline | 685 | 6 |
| `src/codegen/engine/mir/mir.rs` | inline | 744 | 16 |
| `src/codegen/registry/mod.rs` | inline | 2695 | 62 |

Totals: 37 files, 22,894 test LOC, 459 tests (the loop over the table's paths summing
`grep -cE '#\[(tokio::)?test\]'` → 459).

### Verified properties

- **A `tests/mod.rs` whose `use super::*;` is consumed only by its children's globs does not
  warn, and a private `use` in the host reaches a grandchild topic.** Probe `/tmp/hij_probe`
  (host `src/host.rs` with a private `use std::path::PathBuf;`, `#[cfg(test)] mod tests;`,
  `src/host/tests/mod.rs` = `use super::*; mod topic_a; mod topic_b;`): `cargo test` → no
  `warning`, 2 passed, `--list` shows `host::tests::topic_a::a`. This is the layout of every
  host in §4.
- **A data-only part file must not start with `use super::*;`.** Same probe, `data.rs` holding
  only `pub(super) const TABLE: &[&str]` under `use super::*;` → `warning: unused import:
  super::*`. So `corpus/fixture_list.rs` and `diagnostics/fixture_list.rs` carry no `use` line.
- **`#[path]` on a module declared in a non-`mod.rs` file resolves relative to that file's
  directory, and a `#[path]` target named `mod.rs` resolves its own children next to it.**
  Same probe: `#[path = "host/gen_x/tests/mod.rs"] mod gen_x_tests;` in `src/host.rs` loaded
  `src/host/gen_x/tests/mod.rs`, whose `mod topic_c;` loaded `src/host/gen_x/tests/topic_c.rs`
  (`--list` → `host::gen_x_tests::topic_c::c`). The existing precedent is
  `src/codegen/builtins/tls/gen_schannel.rs:291` `#[path = "gen_schannel_tests.rs"]`, which
  loads `tls/gen_schannel_tests.rs`.
- **Glob-cycle imports (`use part::*;` in `mod.rs`, `use super::*;` in parts) compile without
  warnings** — `/tmp/loc/probe2` printed `6` with no warning.
- **Every `.rs` file under a `src/**/tests/` directory is test-only.** `find src -type d -name
  tests` → 17 directories; for each, the parent declares `mod tests;` directly under
  `#[cfg(test)]`, except `src/codegen/engine/mod.rs:13` `pub(crate) mod tests;`, whose
  `engine/tests/mod.rs` gates every item `#[cfg(test)]` (read: `test_support`, its re-export,
  `tests`, `builtins_codegen`). This is the proof that the §4.1 guard change exempts no
  production file.
- **The architecture guards exempt test code by FILE NAME.** `tests/guards/architecture_guards.rs`
  `is_physical_scan_exempt` matches `"tests.rs" | "test_support.rs" | "abi.rs"` (+ regalloc
  `analysis.rs`); `no_hand_picked_vreg` and `emission_lines` skip `name.ends_with("tests.rs")`;
  everything else is scanned through `code_above_tests` (text before the first `#[cfg(test)]`
  or `mod tests`). A topic file such as `regalloc/tests/linear_scan_basics.rs` matches neither,
  so it would be scanned whole. Replaying the four guards' needles over every moved region
  (H map) finds offenders only in `src/codegen/engine/regalloc/tests.rs` (`format!("d{r}")`
  in `linear_scan_records_callee_saved_reload_scratch_fp`, `format!("xmm{n}")` in
  `x86_fp_values_live_across_a_call_avoid_the_volatile_high_xmm`, two `"%f0 …"` message
  literals in `linear_scan_avoids_live_fp_scratch_token_realization`) and in the inline tests
  of `src/codegen/engine/mir/mir.rs` (102 `"%vN"`/`"%fN"` literals, 138
  `Reg::physical(`/`.physical_field(`/`Operand::phys(` lines). Every other H region: 0.
- **`no_type_strings.rs` / `no_operator_strings.rs` already exempt `/tests/` paths.**
  `is_test_file` in both: `rel.contains("/tests/") || name == "tests.rs" || …`.
- **Nothing in H is cfg-gated to a non-macOS host.** `grep -n "cfg(target_os\|cfg(unix\|cfg(windows"`
  over the 37 files → no hit. The macOS gate compiles every moved line.
- **Moving an inline test module out of a code file removes covered lines from that file's
  coverage denominator** (`.ai/testing-gates.md`: "A test module counts in its own file's
  denominator unless it lives under a `tests/` directory"). The file's percentage after the
  move is its code-only percentage, which can be lower. Per-file coverage is not a plan-176
  gate (§Validation Plan "Coverage check"), so H runs no coverage measurement.

## 3. Design Overview

One convention: a whole test file `x.rs` becomes `x/mod.rs` + topic files; an inline
`#[cfg(test)] mod tests { … }` becomes `#[cfg(test)] mod tests;` → `<host>/tests/mod.rs` +
topic files. `mod.rs` keeps the `use` block, the shared helpers and one `mod <topic>;` line per
topic. Each topic starts with `use super::*;`. A helper used by one topic only moves with it.
`impl` blocks travel with their type. A "part" layout (a helper library, not a topic suite)
adds `mod <part>; use <part>::*;` to `mod.rs` and marks the cross-file items `pub(super)`
(listed per file). Data-only part files carry no `use` line.

**Wiring of `builtins/tests/` is preserved.** `src/codegen/engine/tests/mod.rs` keeps
`#[cfg(test)] #[path = "../../builtins/tests/mod.rs"] mod builtins_codegen;`. Each
`builtins/tests/<x>.rs` becomes `builtins/tests/<x>/mod.rs`; the `mod <x>;` line in
`builtins/tests/mod.rs` does not change, and `<x>/<topic>.rs` resolves with no `#[path]`
(the `#[path]` target is named `mod.rs`, verified above).

Gate class: test letter. Per phase, the whole-binary test count before == after, the moved
modules' tests pass, `cargo check --all-targets` adds no warning, the guards pass, and the
file-size guard shows only TIGHTEN/STALE rows for the phase's files before the tighten.

Risk concentrates in three places:
- **The architecture-guard change** (§4.1). It must exempt exactly the moved test code. It
  lands in Phase 3 with the two splits that need it; the replay and the 17-directory proof
  above are its evidence.
- **`src/codegen/builtins/mod.rs`'s inline tests** — a plain `mod tests;` there would load
  `builtins/tests/mod.rs`, which is the `builtins_codegen` suite root, a second time. The
  `#[path = "tests/unit/mod.rs"]` declaration (§4.32) avoids it.
- **`src/codegen/registry/mod.rs`** — the largest move (2160 LOC into 15 files), in a file that
  is dirty in the working tree today. Last phase.

Rejected alternatives:
- *Plain `mod unit_tests;` → `builtins/unit_tests/`* (and the same for `error_path_release_tests`,
  `schannel_tests`) — puts test files outside a `tests/` directory, so coverage counts them and
  the guards scan them.
- *Splitting `CORPUS` / `DIAGNOSTIC_CORPUS` into several consts concatenated at the call sites,
  or reading a data file* — both are logic edits to the tests; the owner's rule exempts a
  single over-limit test-side unit instead.
- *Splitting `impl CodegenPlatform for FailAt` by moving method bodies into free fns* — a logic
  edit; a trait impl cannot be split across files.
- *Moving `World`-style shared models out of `mod.rs` when `mod.rs` is under 250* — adds
  visibility edits for no budget gain.
- *Widening the architecture guards' file-name list (`*_basics.rs`, …)* — names drift; the
  `/tests/` component is what `no_type_strings.rs` already uses.

## 4. Detailed Design

Line ranges `[src a-b]` are source lines as of `17a9b5799`; item and test NAMES are
authoritative if lines drift. LOC figures are `span.py` counts of the moved items + 1 for the
topic's `use super::*;`. "Shared" items stay in `mod.rs`. Visibility edits: none unless
stated. Hazard commands run per file: `grep -rnoE "\[\[<path>:[^]]+\]\]" src/docs/spec`;
`grep -rn "<path>" tests/guards scripts .ai AGENTS.md src/docs .github Cargo.toml`;
`grep -n "<path>" scripts/coverage-exceptions.txt`; a scan of the moved lines for
`include_str!|include_bytes!|#[path|module_path!|file!()|--exact|current_exe|env!(`. "Hazards:
none" means all of these returned nothing relevant. `spec_citations_resolve` checks only that
a cited FILE exists: a cite into a test file that becomes a directory FAILS it; a cite into a
code file that keeps existing goes silently stale — both kinds are listed and repointed.

### 4.0 Exemptions H adds

Each gets an `exempt test <path> # <reason>` row in `tests/guards/file_size_ceilings.txt` and
the same entry in the `EXEMPT` table of `tools/loc-census/census.py`, in the commit that
creates the file:

| File | Test LOC | Reason (row text) |
|---|---|---|
| `src/codegen/builtins/tests/corpus/fixture_list.rs` | 644 | one `const CORPUS: &[&str]` fixture list (643 LOC, one name per line); a const slice cannot be split without a logic change |
| `src/codegen/builtins/tests/diagnostics/fixture_list.rs` | 419 | one `const DIAGNOSTIC_CORPUS: &[&str]` fixture list (418 LOC); a const slice cannot be split without a logic change |
| `src/codegen/builtins/tests/platform_failure/forwarding.rs` | 867 | one `impl CodegenPlatform for FailAt` (every trait method forwarded, so a new method is a compile error); a trait impl cannot span files |

### 4.1 Guard change: `tests/guards/architecture_guards.rs`

In the root file (plan-176-E keeps these functions there):
- `is_physical_scan_exempt(path)`: also return true when the path relative to
  `CARGO_MANIFEST_DIR` contains `/tests/` (compute `rel` from the manifest, as
  `shared_lowering_names_no_physical_register` already does for its messages, and pass it in).
- `no_hand_picked_vreg`: skip a file when its manifest-relative path contains `/tests/`, beside
  the existing `name.ends_with("tests.rs")` skip.
- `emission_lines`: the same skip (feeds `physical_registers_stay_in_the_backends` and
  `no_constructed_vreg_name`).

Justification (AGENTS.md, guard edit):
1. When/why written: the file-name skips came with the arch lint's extraction
   (`git log -S'"tests.rs" | "test_support.rs"' --format='%h %s'` → `9547ef9cf bug-334 D2:
   extract the plan-34-D arch lint to architecture_guards.rs`) and the register guards
   (`git log -S'name.ends_with("tests.rs")'` → `8818c963e bug-721: root cause, register
   guards, …`).
2. Behavior protected: emission code never names a physical register or hand-numbers a vreg;
   test fixtures that spell registers are exempt.
3. Dependents: the four guard tests above; plan-176-J and K list this change as a prerequisite
   (K's Prerequisites table greps for it).
4. Proof the narrower rule is wrong for the new layout: the guard's own doc says test-fixture
   files are exempt ("the pure test-fixture files"); the moved regions are the same test code
   at a new path; and every `.rs` under a `src/**/tests/` directory is test-only (§2, 17
   directories). No production file becomes exempt.

### 4.2 `src/codegen/builtins/canvas/helper_render/tests/mod.rs` (276, 11 tests)

Existing directory module gains two topics.
- `mod.rs` (23): the `use` block (8), 2 `mod` lines; shared: `source`, `body`.
- `draw_entry_and_budgets.rs` (202, 8 tests) [src 11-363]: `the_draw_entry_width_agrees_with_the_emitter`,
  `block_instances_keeps_the_blend_split_case`, `the_two_gpu_edge_budgets_match_the_emitters`,
  `both_sums_count_each_picture_block_once`, `every_cap_token_in_the_render_source_is_generated`,
  `the_predicates_read_the_edge_count_slot`, `the_text_kind_is_spelled_once`,
  `the_polygon_kind_is_spelled_once`; helper `declared`.
- `renderer_paths.rs` (55, 3 tests) [src 369-431]: `the_render_loop_starts_the_vulkan_build_where_the_handoff_says`,
  `the_vulkan_arm_asks_through_the_wrapper`, `both_renderers_cull_through_the_one_viewport_predicate`.
Hazards: none.

### 4.3 `src/codegen/builtins/tests/canvas.rs` → `canvas/` (446, 10 tests)

- `mod.rs` (99): `use` block (5), 2 `mod` lines; shared: `PRESENT_SRC`, `program`,
  `publish_scene`, `publish_layers`, `both`, `tagged_label`, `scene_slot`, `scene_stores`,
  `scene_store_offsets`.
- `publish_bodies.rs` (168, 6 tests) [src 29-370]: `the_scene_is_addressed_through_the_process_global_symbol`,
  `the_revision_is_the_final_scene_store`, `publishing_one_shape_zeroes_the_other_shapes_pair`,
  `the_two_publish_bodies_do_not_share_label_names`, `the_skip_reports_false_and_the_publish_reports_true`,
  `the_wrong_mode_gate_precedes_the_scene_copys_allocation`; helpers `body_labels`, `exit_result`.
- `frame_retirement_and_backends.rs` (183, 4 tests) [src 396-620]: `retired_scene_blocks_are_freed_only_after_a_frame_completes`,
  `every_publish_retires_the_displaced_blocks_and_stamps_the_frame`,
  `the_canvas_program_lowers_on_every_app_capable_backend`,
  `a_user_type_named_like_a_canvas_variant_does_not_change_the_canvas_code`; helper
  `USER_TYPES_NAMED_LIKE_CANVAS_SRC`.
Hazards: none.

### 4.4 `src/codegen/builtins/tests/collections.rs` → `collections/` (418, 11 tests)

- `mod.rs` (19): `use` block (8), 3 `mod` lines; shared `fast_path_labels`.
- `group_by_and_zip.rs` (124, 4) [src 31-192]: `group_by_takes_the_native_path_for_a_supported_instantiation`,
  `group_by_declines_when_its_source_list_would_be_evaluated_twice`,
  `zip_takes_a_native_path_for_both_element_shapes`, `every_native_zip_selects_a_bound_before_its_copy_loop`;
  helper `interpreted_calls`.
- `find_last_index_and_fast_paths.rs` (155, 5) [src 203-391]: `group_by_takes_the_native_path_for_a_fixed_width_value`,
  `find_last_index_emits_a_reachable_bounds_rejection`, `both_find_last_index_forms_take_the_native_path`,
  `each_fast_path_declines_every_target_but_its_own`, `zip_loads_each_fixed_width_element_at_its_own_size`.
- `declared_errors_and_ownership.rs` (126, 2) [src 408-573]: `collections_get_declares_errors_per_overload_not_merged`,
  `every_collections_member_returning_an_element_has_an_ownership_verdict`.
Hazards: none.

### 4.5 `src/codegen/builtins/tests/corpus.rs` → `corpus/` (766, 2 tests)

- `mod.rs` (126 + `use` block (3) + `mod fixture_list;` + `pub(super) use fixture_list::CORPUS;`)
  [src 1-132, 878-987]: `CROSS_BACKEND`, `program_functions`; tests
  `every_backend_lowers_the_corpus_to_the_same_program`, `no_corpus_function_lowers_to_an_empty_body`.
- `fixture_list.rs` (644, exempt §4.0) [src 133-877]: `CORPUS` only, its visibility changed
  from `pub(super)` to `pub(crate)` (a `pub(super)` item in `fixture_list` could not be
  re-exported past `corpus`). No `use` line.
Surface: `nir_dump.rs` (×2) and `optimizer.rs` read `super::corpus::CORPUS` — unchanged through
the re-export. Hazards: none.

### 4.6 `src/codegen/builtins/tests/declared_errors_all.rs` → `declared_errors_all/` (780, 23 tests)

- `mod.rs` (61): the `use super::declared_errors_sweep::{…}` block (same `super`), 4 `mod`
  lines; shared `raise_check_skipped`, `const_str_eq`, `NONE_P`, `NONE_U`, `ARENA`,
  `ASTRINGS_RAISES_NOTHING`.
- `network_and_media.rs` (141, 7) [src 19-310]: `every_{datetime,net,tls,udp,app,audio,collections}_member_declares_exactly_the_errors_its_lowering_raises`;
  tables `TLS_PROPAGATING`, `APP_RAISES_NOTHING`, `AUDIO_UNREACHABLE_OWN_RAISE`,
  `AUDIO_PROPAGATING`, `COLLECTIONS_RAISES_NOTHING`, `COLLECTIONS_UNREACHABLE_OWN_RAISE`.
- `io_and_term.rs` (211, 2) [src 319-555]: `every_{io,term}_member_declares_exactly_the_errors_its_lowering_raises`;
  tables `IO_RAISES_NOTHING`, `IO_DECLARED_EXTRA`, `TERM_RAISES_NOTHING`, `TERM_PROPAGATING`,
  `TERM_DECLARED_EXTRA`.
- `text_system_and_canvas.rs` (147, 7) [src 557-730]: `every_{astrings,strings,big,os,process,thread,canvas}_member_declares_exactly_the_errors_its_lowering_raises`;
  tables `ARENA_UTF8`, `ASTRINGS_SKIP_MEMBERS`, `THREAD_RAISES_NOTHING`, `THREAD_PROPAGATING`,
  `CANVAS_SKIP_MEMBERS`, `CANVAS_RAISES_NOTHING`.
- `crypto_and_numeric.rs` (228, 7) [src 741-1006]: `every_{crypto,tcp,bits,json,math,money,regex}_member_declares_exactly_the_errors_its_lowering_raises`;
  tables `TCP_UNREACHABLE_OWN_RAISE`, `CRYPTO_SKIP_MEMBERS`, `CRYPTO_IFN_PROPAGATING`,
  `MATH_RAISES_NOTHING`, `JSON_PROPAGATING`, `MATH_UNREACHABLE_OWN_RAISE`.
Hazards: none.

### 4.7 `src/codegen/builtins/tests/declared_errors_sweep.rs` → `declared_errors_sweep/` (493, 0 tests)

Helper library for four sibling suites; part layout.
- `mod.rs` (32 + `use` block (11) + 3 `mod`/`use part::*` pairs) [src 1-42, 78-105]:
  `Propagating`, `codes_by_value`, and
  `pub(super) use fixtures::{lower_body, platforms};`
  `pub(super) use sweep::{assert_package_declares_exactly_the_errors_its_lowering_raises, assert_package_declares_exactly_the_errors_its_lowering_raises_on};`
  so `declared_errors_all`, `fs_declared_errors`, `general_declared_errors` and
  `mfb_body_declared_errors` import exactly as today.
- `fixtures.rs` (177) [src 43-297]: `platforms`, `scan`, `lower_body`, `call_forms`,
  `concretize`, `PROGRAM_FIXTURES`, `package_harness`. `pub(super)`: `concretize`,
  `package_harness` (2 edits). `PROGRAM_FIXTURES` is read only by `package_harness`
  (`grep -n PROGRAM_FIXTURES` → lines 223/247/257, all in this part) — no edit.
- `member_lowering.rs` (100) [src 299-411]: `lower_body_call`, `lower_member` (already `pub(crate)`).
- `sweep.rs` (194) [src 416-639]: `SweepResult` (already `pub(crate)`, no sibling reads it:
  `grep -rn "SweepResult\b" src/codegen/builtins/tests/*.rs` → only this file),
  `sweep_package`, the two `assert_package_…` fns.
Hazards: prose in `declared_errors_all.rs` comments names `declared_errors_sweep::PROGRAM_FIXTURES`
— still true. Otherwise none.

### 4.8 `src/codegen/builtins/tests/diagnostics.rs` → `diagnostics/` (569, 4 tests)

- `mod.rs` (154 + `use` block (2) + `mod fixture_list; use fixture_list::DIAGNOSTIC_CORPUS;`)
  [src 1-58, 477-687]: `golden_rules`, `PACKAGE_DIAGNOSTIC_CORPUS`,
  `TYPE_EXPORTING_PACKAGE_FIXTURES`; tests `the_syntax_corpus_reproduces_its_goldens_in_process`,
  `the_package_bearing_syntax_fixtures_reproduce_their_goldens`,
  `the_syntax_corpus_still_reaches_every_rule_it_used_to`,
  `a_valid_program_importing_a_type_exporting_package_is_silent`.
- `fixture_list.rs` (419, exempt §4.0) [src 59-476]: `DIAGNOSTIC_CORPUS` only, made
  `pub(super)` (1 edit). No `use` line.
Hazards: none.

### 4.9 `src/codegen/builtins/tests/general_declared_errors.rs` → `general_declared_errors/` (311, 1 test)

- `mod.rs` (174 + `use` block (5) + `mod measurement;`) [src 1-243]: `OVERLOADS` (stays
  `pub(super)`; `mfb_body_declared_errors` reads `super::general_declared_errors::OVERLOADS`),
  `UNREACHABLE`, `RAISES_NOTHING`.
- `measurement.rs` (140, 1) [src 244-402]: `probe_source`, `transitive_codes`, `measure`; test
  `every_general_member_declares_exactly_the_errors_its_lowering_raises`.
Hazards (all fixed in the same commit):
- `src/docs/spec/language/18_builtin-functions.md:55`
  `[[src/codegen/builtins/tests/general_declared_errors.rs:OVERLOADS]]` →
  `[[src/codegen/builtins/tests/general_declared_errors/mod.rs:OVERLOADS]]` (required:
  `spec_citations_resolve` fails on the vanished file).
- `src/codegen/builtins/totality_proof.rs:19` `//!` prose `tests/general_declared_errors.rs` →
  `tests/general_declared_errors/`. (`:64` and `builtins/mod.rs:493` name the module, not the
  file — unchanged.)

### 4.10 `src/codegen/builtins/tests/inplace.rs` → `inplace/` (306, 7 tests)

- `mod.rs` (26): `use` block (6), 2 `mod` lines; shared `plan`, `label_stems`, `has_family`.
- `list_appends.rs` (110, 3) [src 51-274]: `only_a_single_element_of_the_element_type_appends_in_place`,
  `a_collection_element_copies_its_payload_into_the_slot`, `every_append_shape_is_emitted_into_its_caller`;
  helpers `SINGLE`, `BULK`, `NESTED`.
- `record_field_growth.rs` (174, 4) [src 277-462]: `a_record_field_grows_in_place_only_when_it_is_the_last_one`,
  `appending_one_field_into_another_rebuilds`,
  `a_variable_width_field_copies_its_entry_array_and_a_fixed_width_one_has_none`,
  `concatenating_into_a_record_field_rebases_the_copied_entries`; helpers `WITH_LAST_FIELD`,
  `WITH_FIRST_FIELD`, `WITH_CROSS_FIELD`, `WITH_STRING_FIELD`, `WITH_BULK_FIELD`.
Hazards: none.

### 4.11 `src/codegen/builtins/tests/inplace_compact.rs` → `inplace_compact/` (293, 11 tests)

- `mod.rs` (26): `use` block (4), 2 `mod` lines; shared `label_stems`, `program`.
- `compact_and_permute.rs` (139, 6) [src 20-200]: `compact_in_place_range_form_on_a_fixed_width_list_is_one_block_move`,
  `compact_in_place_marks_form_on_a_fixed_width_list_walks_the_elements`,
  `compact_in_place_on_an_entry_list_probes_the_payload_order`,
  `compact_in_place_range_form_on_an_entry_list_tries_the_truncation_first`,
  `compact_in_place_range_drop_pass_skips_the_kept_run`,
  `permute_in_place_reorders_lanes_and_entries_without_a_gather`; helpers `IS_POSITIVE`, `LONGISH`.
- `removal_and_growth.rs` (132, 5) [src 208-357]: `map_compact_in_place_marks_then_compacts_and_union_reserves_once`,
  `remove_at_on_an_entry_list_moves_only_the_later_entries`,
  `an_entry_list_grow_reclaims_dead_bytes_and_a_fixed_width_grow_does_not`,
  `a_copying_list_insert_drops_dead_bytes_only_for_an_entry_list`,
  `an_in_place_map_or_set_grow_reclaims_dead_bytes_first`.
Hazards: none (the `//!` header names `tests/runtime/rt_inplace_self_update.rs`, unaffected).

### 4.12 `src/codegen/builtins/tests/mfb_body_declared_errors.rs` → `mfb_body_declared_errors/` (1855, 2 tests)

Scanner/inference library + two sweeps; part layout split by responsibility.
- `mod.rs` (25): the existing `use` block (incl. `use super::declared_errors_sweep::codes_by_value;`
  and `use super::general_declared_errors::OVERLOADS;`) + 10 `mod <part>; use <part>::*;` pairs.
- `call_scan.rs` (187) [src 91-305]: `general_names`, `is_ident_char`, `fail_codes`,
  `preceded_by_case`, `qualified_calls_with_arguments`, `internal_calls`, `general_calls`,
  `call_arguments`. `pub(super)`: all but `preceded_by_case` (7).
- `scope.rs` (213) [src 313-551]: `Scope` + `impl Scope`, `package_globals`, `EACH_PREFIX`,
  `source_spelling`, `package_of_symbol`, `strip_outer_parens`, `has_top_level_operator`,
  `is_arithmetic`. `pub(super)`: `Scope`, its fields `package`, `func`, `names` (read by
  type inference, call codes and trap resolution: `awk 'NR<313||NR>551' … | grep -n
  "scope\.\(package\|func\|names\)"` → 8 hits), `Scope::of` (called at old :1457),
  `package_globals`, `EACH_PREFIX`, `source_spelling`, `strip_outer_parens`,
  `has_top_level_operator`, `is_arithmetic` (11).
- `type_inference.rs` (218) [src 564-803]: `infer_type`, `infer_type_inner`,
  `record_field_type`, `single_return_type`, `overloaded_return_types`,
  `general_override_helper`. `pub(super)`: `infer_type`, `overloaded_return_types`,
  `general_override_helper` (3).
- `call_codes.rs` (230) [src 810-1079]: `is_arithmetic_expression`, `qualified_call_codes`,
  `general_call_codes`. `pub(super)`: the last two (2).
- `func_blocks.rs` (203) [src 1087-1345]: `extract_func_blocks`, `strip_comment_lines`,
  `strip_func_header_and_footer`, `contains_keyword`, `find_function_level_trap`,
  `find_inline_trap_regions`, `scan_calls_and_fails` — all `pub(super)` (7).
- `trap_resolution.rs` (144) [src 1356-1510]: `resolve_inline_traps`, `resolve`,
  `MfbSweepResult` (already `pub(crate)`). `pub(super)`: `resolve` (1).
- `callee_tables.rs` (204) [src 1536-1798]: `UNREACHABLE_VIA_CALLEE` → `pub(super)` (1).
- `call_tables.rs` (219) [src 1806-2279]: `UNREACHABLE_GENERAL_CALL`,
  `UNREACHABLE_QUALIFIED_CALL`, `DECLARED_EXTRA` → `pub(super)` (3).
- `body_sweep.rs` (136, 1) [src 2282-2445]: `SweepTables`, `sweep_tables`, `sweep` (already
  `pub(crate)`); test `every_body_mfb_member_declares_exactly_the_errors_its_source_can_raise`.
  `pub(super)`: `SweepTables`, its fields `member_declared`, `rewrite_to_owner`, `helper_blocks`
  (read by `rewrite_sweep` at old :2493-2515), `sweep_tables` (5).
- `rewrite_sweep.rs` (106, 1) [src 2460-2577]: `RewriteSweepResult`, `REWRITE_UNREACHABLE`,
  `REWRITE_DECLARED_EXTRA`, `rewrite_sweep`; test
  `every_body_rewrite_implementation_declares_exactly_the_errors_its_target_can_raise`.
Visibility edits total: 40. Hazards:
`src/docs/spec/tooling/04_audit-format.md:311`
`[[src/codegen/builtins/tests/mfb_body_declared_errors.rs:rewrite_sweep]]` →
`[[src/codegen/builtins/tests/mfb_body_declared_errors/rewrite_sweep.rs:rewrite_sweep]]`
(required). The untracked `bugs/bug-1092-…md` is not H's file and is not edited.

### 4.13 `src/codegen/builtins/tests/nir_fault.rs` → `nir_fault/` (721, 1 test)

- `mod.rs` (97 + `use` block (5) + 4 part pairs) [src 1-43, 208-303]: `STATEFUL_SRC`; test
  `a_corrupted_value_is_refused_rather_than_lowered`.
- `program_source.rs` (165) [src 44-207]: `SRC` → `pub(super)` (1).
- `corruptors.rs` (136) [src 304-489]: `corrupt`, `corrupt_blank`, `blank_corruptible`,
  `corrupt_op`, `corrupt_arity`, `arity_corruptible`, `op_corruptible`, `corruptible` — all
  `pub(super)` (8).
- `op_walk.rs` (145) [src 498-650]: `walk_ops_mut`, `walk_value_mut`, `snapshot`, `restore`;
  `pub(super)`: `walk_ops_mut`, `snapshot`, `restore` (3).
- `sweep.rs` (190) [src 653-906]: `corrupt_nth`, `corrupt_nth_blank`, `corrupt_nth_arity`,
  `corrupt_nth_op`, `sweep`; `pub(super)`: `sweep` (1).
Hazards: none.

### 4.14 `src/codegen/builtins/tests/nir_json.rs` → `nir_json/` (325, 6 tests)

- `mod.rs` (171 + `use` block (3) + `mod dump_shapes;`) [src 40-243]: `LINKING`, `SHAPES`,
  `MODULE_SHAPES`, `STATE_SHAPES`, `OP_SHAPES`.
- `dump_shapes.rs` (157, 6) [src 250-455]: `the_nir_dump_is_json_on_every_backend`,
  `the_nir_dump_names_every_function_in_the_module`, `the_nir_dump_carries_a_link_block_whole`,
  `the_native_plan_dump_is_json_on_every_backend`, `the_nir_dump_spells_every_shape_the_program_has`,
  `the_nir_dump_spells_the_resource_shapes`.
Hazards: none.

### 4.15 `src/codegen/builtins/tests/nir_validation.rs` → `nir_validation/` (813, 11 tests)

- `mod.rs` (183 + `use` block (4) + 4 `mod` lines) [src 31-140 and the helpers below 756]:
  `SRC`, `module`, `refusal`, `wreck_first_op`, `walk_ops`, `wreck_any_op`.
- `module_header_and_globals.rs` (103, 4) [src 141-278]: `a_module_from_a_real_lowering_validates`,
  `the_module_header_fields_are_required`, `a_malformed_global_is_refused`, `a_malformed_type_is_refused`.
- `functions_and_body_ops.rs` (122, 3) [src 281-432]: helper `first_bind`;
  `a_malformed_function_is_refused`, `an_entry_that_names_no_function_is_refused`,
  `a_malformed_body_op_is_refused`.
- `references_and_params.rs` (201, 3) [src 440-756]: helper `rename_first_value`;
  `a_reference_that_does_not_resolve_is_refused`, `the_entry_point_must_match_the_function_it_names`,
  `a_parameter_needs_a_name_a_type_and_a_unique_one`.
- `op_names.rs` (216, 1) [src 771-991]: `an_op_that_names_something_must_name_something_real`
  (one 215-LOC test fn, under 250).
Hazards: none.

### 4.16 `src/codegen/builtins/tests/optimizer_globals.rs` → `optimizer_globals/` (325, 7 tests)

- `mod.rs` (169 + `use` block (5) + `mod global_simplification;`) [src 57-239]: `SRC`, `module_at`.
- `global_simplification.rs` (159, 7) [src 255-473]: helpers `reads_global`, `global`; tests
  `the_initializer_does_not_store_to_a_removed_global`, `the_global_rows_do_nothing_below_their_level`,
  `a_global_nothing_names_is_removed`, `a_never_written_literal_global_is_folded_away_and_then_collected`,
  `a_never_written_global_the_folder_cannot_touch_is_still_read_only`, `a_written_global_is_left_alone`,
  `an_exported_global_is_never_simplified`.
Hazards: none.

### 4.17 `src/codegen/builtins/tests/os.rs` → `os/` (372, 8 tests)

- `mod.rs` (56): `use` block (9), 2 `mod` lines; shared `SRC`, `program`, `body`, `calls`.
- `members_and_sources.rs` (139, 4) [src 34-259]: `every_backend_implements_every_os_member`,
  `is_admin_tests_the_opposite_sense_on_windows_and_unix`, `version_reads_each_platforms_own_source`,
  `name_and_arch_are_materialized_constants_not_queries`.
- `uptime_and_surface.rs` (181, 4) [src 268-496]: `uptime_is_seconds_on_every_backend`,
  `every_os_body_refuses_an_undeclared_import`, `register_publishes_the_whole_os_surface`,
  `resource_path_lowers_on_every_backend`; helper `OsBody`.
Hazards: none.

### 4.18 `src/codegen/builtins/tests/plan_validation.rs` → `plan_validation/` (293, 4 tests)

- `mod.rs` (46): `use` block (5), 2 `mod` lines; shared `SRC`, `plan`, `refusal`.
- `plan_header.rs` (52, 1) [src 27-152]: `the_plan_header_must_be_complete`.
- `functions_and_storage.rs` (199, 3) [src 162-347]: `a_planned_function_must_name_what_it_uses`,
  `a_storage_type_must_describe_real_storage`, `a_real_plan_validates_on_every_backend`;
  helpers `wreck`, `wreck_main`, `word_storage`.
Hazards: none.

### 4.19 `src/codegen/builtins/tests/platform_failure.rs` → `platform_failure/` (1088, 1 test)

- `mod.rs` (224 + `use` block (12) + `mod forwarding;`) [src 1-110, 977-1224]: `INJECTED`,
  `FailAt` + `impl FailAt`, `os_seam_platforms`, `Needs`, `what_a_clean_lowering_needs`,
  `between`, `lower_body`; test `an_emit_failure_past_the_first_one_still_propagates`.
- `forwarding.rs` (867, exempt §4.0) [src 111-976]: `use super::*;` + `impl CodegenPlatform for
  FailAt<'_>` only. As a child of the defining module it reaches `FailAt`'s private fields and
  private `tick`/`granting` — no visibility edits; `mod forwarding;` needs no glob (an impl has
  no name).
Hazards: none.

### 4.20 `src/codegen/builtins/tests/platform_hooks.rs` → `platform_hooks/` (450, 7 tests)

- `mod.rs` (150): `use` block (9), 2 `mod` lines; shared `platforms`, `Sink` + `impl Sink`,
  `Answer`, `optional_hook_answers`.
- `windows_only_hooks.rs` (201, 4) [src 102-333]: `the_windows_only_primitives_refuse_on_every_posix_backend`,
  `the_openwithin_reparse_checks_are_unreachable_off_windows`,
  `a_backend_that_overrides_nothing_declines_every_optional_hook`,
  `windows_implements_every_primitive_the_others_refuse`.
- `optional_hooks.rs` (103, 3) [src 338-624]: `every_backend_answers_each_optional_hook_exactly_as_recorded`,
  `every_backend_reports_a_family_consistent_with_its_target`,
  `an_app_build_on_a_backend_with_no_app_mode_refuses_the_terminal_predicates`; helper
  `expected_answer`. The fn-local `macro_rules! probe` stays inside its fn.
Hazards: none.

### 4.21 `src/codegen/builtins/tests/validation.rs` → `validation/` (337, 9 tests)

- `mod.rs` (35): `use` block (5), 2 `mod` lines; shared `SRC`, `plan`, `refusal`.
- `plan_structure.rs` (162, 6) [src 31-255]: `a_plan_from_a_real_lowering_validates`,
  `the_plan_header_fields_are_all_required`, `an_entry_symbol_that_resolves_to_no_function_is_refused`,
  `an_incomplete_import_is_refused`, `a_malformed_data_object_is_refused`, `a_malformed_function_is_refused`.
- `branches_and_relocations.rs` (144, 3) [src 263-419]: `a_branch_to_an_undefined_label_is_refused_with_the_function_named`,
  `a_relocation_is_refused_when_it_does_not_match_its_binding`, `a_symbol_defined_twice_is_refused`;
  helper `reloc`.
Hazards: none.

### 4.22 `src/codegen/builtins/tls/gen_macos/tests.rs` → `gen_macos/tests/` (884, 23 tests)

`gen_macos/mod.rs:943` keeps `#[cfg(test)] mod tests;`.
- `mod.rs` (29): `use` block (5), 4 `mod` lines; shared `TlsReadTestPlatform` (unit struct),
  `has_label`, `window`, `resolves_in`.
- `read_platform.rs` (244, 0 tests) [src 15-273]: `impl CodegenPlatform for TlsReadTestPlatform`
  only (under 250).
- `release_paths.rs` (185, 10) [src 277-583]: `read_drain_releases_mapped_and_content_before_publishing`,
  `read_has_no_encoding_error_exit`, `read_timeout_exit_leaves_the_receive_armed`,
  `accept_failure_exits_release_the_connection`, `connect_failure_exits_release_connection_and_queue`,
  `read_does_not_recycle_the_shared_semaphore`, `write_releases_previous_semaphore`,
  `connect_releases_endpoint_and_params`, `close_releases_connection_queue_and_sem`,
  `accept_stores_zero_queue_slot`; helper `blr_between`.
- `cancel_drains.rs` (192, 8) [src 592-908]: `accept_failure_exits_drain_to_cancelled`,
  `accept_handshake_gets_a_fresh_deadline_after_the_pop`, `close_listener_drains_to_cancelled`,
  `close_drains_to_cancelled`, `close_listener_releases_queue_and_sem`,
  `listen_zeroes_its_cleanup_slots_before_any_failure_exit`,
  `write_names_a_posix_send_failure_connection_closed`,
  `the_send_completion_records_the_error_domain_itself`; helper `has_cancel_drain`.
- `gates_and_symbols.rs` (242, 5) [src 910-1164]: `trampolines_publish_the_error_domain_before_the_gate`,
  `write_loads_its_gates_by_load_acquire`, `write_checks_the_terminal_state_before_creating_its_content`,
  `a_read_that_reports_the_close_makes_every_later_write_raise`, `data_objects_define_each_symbol_once`;
  helpers `ctx_stores`, `addressed_by_previous`.
Hazards: none.

### 4.23 `src/codegen/builtins/tls/gen_schannel_tests.rs` → `tls/gen_schannel/tests/` (392, 11 tests)

`gen_schannel.rs:291-292` `#[path = "gen_schannel_tests.rs"] mod schannel_tests;` →
`#[path = "gen_schannel/tests/mod.rs"] mod schannel_tests;` (relative to `tls/`, verified above).
Module name and its `use super::*;` (= `gen_schannel`) unchanged. When plan-176-R/S/T turns
`gen_schannel.rs` into `gen_schannel/mod.rs`, that letter writes `#[path = "tests/mod.rs"]`.
- `mod.rs` (7): `use` block (5), 2 `mod` lines.
- `read_write_and_connect.rs` (218, 6) [src 19-318]: `read_rejects_nonpositive_maxbytes`,
  `listen_accepts_a_pkcs1_key_not_only_pkcs8`, `listen_bind_all_passes_a_non_null_service`,
  `write_classifies_the_winsock_error_behind_a_failed_send`,
  `a_close_notify_ends_the_read_and_stays_ended`,
  `connect_keeps_its_host_scratch_pointer_intact_to_the_release`; helper `reads_reject_invalid_maxbytes`.
- `close_and_accept.rs` (171, 5) [src 325-520]: `a_read_that_reports_the_close_makes_every_later_write_raise`,
  `write_releases_its_send_buffer_on_every_exit_after_allocating_it`,
  `read_names_a_reset_peer_connection_closed`, `accept_retries_an_incomplete_first_call_without_a_context`,
  `accept_rearms_the_handshake_timeouts_to_the_time_left`.
Hazards: `engine/tests/tests.rs::no_overflow_label_returns_through_the_result_tag_register`
scans files named `builder_*`/`func_*`/`gen_*`; `gen_schannel_tests.rs` is in that set today
and its replacements are not. It held 0 overflow labels and the guard's `checked_files > 20`
floor is unaffected. (The same file-name corpus is a hazard for the code letters R–U, which
rename `gen_*` lowering files; recorded here for them.)

### 4.24 `src/codegen/engine/regalloc/tests.rs` → `regalloc/tests/` (721, 16 tests)

`regalloc/mod.rs` keeps `#[cfg(test)] mod tests;`.
- `mod.rs` (6): `use` block (2), 4 `mod` lines.
- `linear_scan_basics.rs` (182, 6) [src 4-245]: `vreg_roundtrips`,
  `linear_scan_keeps_value_across_call_in_callee_saved`, `linear_scan_spills_integer_across_arena_alloc`,
  `linear_scan_records_callee_saved_reload_scratch_int`, `linear_scan_records_callee_saved_reload_scratch_fp`,
  `linear_scan_colors_short_range_in_register`.
- `scratch_and_clobbers.rs` (162, 3) [src 251-448]: `linear_scan_avoids_live_fp_scratch_token_realization`,
  `find_physical_operand_catches_every_class_and_passes_tokens`, `call_clobber_masks_match_each_targets_abi`.
- `abi_clobbers_and_coalescing.rs` (186, 5) [src 455-675]: `audited_runtime_helpers_clobber_only_caller_saved_registers`,
  `x86_fp_values_live_across_a_call_avoid_the_volatile_high_xmm`, `linear_scan_writes_typed_phys_operands`,
  `a_value_dead_at_a_raise_call_keeps_its_register_at_level_one`,
  `coalescing_removes_a_dying_copy_at_level_one`.
- `rv64_eviction_and_splits.rs` (193, 2) [src 681-897]: `eviction_around_rv64_compare_branch_restores_on_both_edges`,
  `live_range_split_never_crosses_control_flow`; helper `run_allocated_rv64`.
Hazards: requires §4.1 in the same commit (the four register spellings listed in §2).

### 4.25 `src/codegen/engine/tests/tests.rs` → `engine/tests/tests/` (404, 16 tests)

`engine/tests/mod.rs` keeps `#[cfg(test)] mod tests;`; `test_support` and `builtins_codegen`
beside it are untouched.
- `mod.rs` (9): `use` block (5), 2 `mod` lines.
- `free_list_and_arena.rs` (191, 9) [src 7-143, 410-511]: `FreeListSim` + `impl FreeListSim`;
  `free_list_bump_then_reuse`, `free_list_coalesces_neighbors`, `free_list_same_shape_churn_stays_short`,
  `free_list_never_merges_across_blocks`, `free_list_over_aligns_to_16_with_front_split`,
  `arena_rejects_invalid_alignment`, `arena_handles_zero_size_allocations`,
  `arena_checks_alignment_rounding_and_capacity`, `arena_checks_arithmetic_overflow`.
- `relocations_and_layout.rs` (210, 7) [src 144-409]: helpers `deferred_reloc_function`,
  `checked_arena_used_after_alloc`; tests `negative_integer_const_materializes_as_its_u64_bit_pattern`,
  `deferred_relocation_binds_from_the_platform_import_map`, `deferred_relocation_rejects_an_undeclared_symbol`,
  `binding_leaves_already_resolved_and_none_relocations_alone`, `fixed_width_agrees_with_payload_alignment`,
  `variable_width_element_types_are_not_fixed_width`, `no_overflow_label_returns_through_the_result_tag_register`.
  `checked_arena_used_after_alloc` sits with its only user (the label-scan test) — no edit.
Hazards: `no_overflow_label_returns_through_the_result_tag_register` reads
`CARGO_MANIFEST_DIR/src/codegen` — manifest-relative, unaffected.

### 4.26 `src/codegen/builtins/audio/mod.rs` inline `mod tests` (296, 11 tests) → `audio/tests/`

`mod tests { … }` (old 526-873) → `#[cfg(test)] mod tests;` (`audio/tests/` does not exist;
`status_tests.rs` is a different module and stays).
- `mod.rs` (9): `use super::*;` + `use super::{gen_alsa_shared, gen_macos_shared, gen_shared, runtime_overload_name, AUDIO_INPUT_TYPE_ID, AUDIO_OUTPUT_TYPE_ID, CLOSE_INPUT, CLOSE_OUTPUT};`
  (topic bodies spell these as `super::…`; from a topic `super` is `audio::tests`, so this one
  line makes them resolve unchanged), 2 `mod` lines; shared `types`.
- `registry_and_overloads.rs` (153, 6) [src 532-712]: `audio_registered_on_the_clean_room_registry`,
  `direction_is_part_of_the_type`, `dual_direction_members_accept_either_handle`,
  `open_and_timed_overloads_resolve`, `source_members_rewrite_to_their_companion_bodies`,
  `named_device_open_rewrites_under_either_device_spelling`; helper `strings`.
- `bodies_and_device_open.rs` (133, 5) [src 714-872]: `close_ops_are_the_per_direction_bodies`,
  `pcm_accumulators_never_cross_a_call_boundary`, `reassembled_source_parses`,
  `core_audio_device_open_reads_the_inlined_id`, `alsa_device_open_reads_the_inlined_id`; helper
  `assert_reads_the_inlined_id`.
Hazards: 1 citation, to code (`AUDIO_INPUT_TYPE`, outside the moved range). `audio/mod.rs`
test LOC after: 4.

### 4.27 `src/codegen/builtins/canvas/mod.rs` inline (408, 14 tests) → `canvas/tests/`

`mod tests` (old 2551-3132) → `#[cfg(test)] mod tests;` (no collision: `helper_render/tests/` is
another module).
- `mod.rs` (4): `use` block (2), 2 `mod` lines.
- `variant_sets.rs` (217, 7) [src 2559-2875]: `draw_item_variant_set_is_frozen`,
  `the_particle_template_is_a_shape`, `no_canvas_type_reaches_itself`, `shape_variant_set_is_frozen`,
  `auto_shape_variant_set_is_frozen`, `every_draw_item_variant_has_a_record`,
  `every_draw_item_variant_carries_a_paint`; helper `members`.
- `resources_and_source.rs` (188, 7) [src 2879-3131]: `no_record_shares_a_name_with_a_resource`,
  `the_resource_naming_variants_hold_the_resource_itself`, `reassembled_source_parses`,
  `the_normal_source_reads_only_the_renderer_headless_and_sync_variables`,
  `the_normal_source_has_no_reporting_hook`, `canvas_text_has_one_glyph_walk`,
  `canvas_types_are_builtin_types`; helpers `DEBUG_ONLY_VARIABLES`, `assert_parses`.
Hazards (silent — `canvas/mod.rs` still exists; repoint by hand):
`src/docs/spec/app/06_canvas.md:96` `…canvas/mod.rs:draw_item_variant_set_is_frozen]]` and
`:1638` `…:no_canvas_type_reaches_itself]]` → `[[src/codegen/builtins/canvas/tests/variant_sets.rs:…]]`;
`:2703` `…:canvas_text_has_one_glyph_walk]]` → `[[src/codegen/builtins/canvas/tests/resources_and_source.rs:canvas_text_has_one_glyph_walk]]`.
`[[…:JoinStyle]]` / `[[…:CANVAS]]` name code — unchanged. Test LOC after: 2.

### 4.28 `src/codegen/builtins/crypto/mod.rs` inline (515, 14 tests) → `crypto/tests/`

- `mod.rs` (4): `use super::*;`, 3 `mod` lines.
- `registry.rs` (160, 5) [src 991-1185]: `crypto_registered_on_the_clean_room_registry`,
  `membership_via_generic_registry`, `native_and_internal_flags`, `builtin_types_recognized`,
  `bytes_text_overloads_select_distinct_rewrite_targets`.
- `resolution_and_padding.rs` (159, 4) [src 1187-1361]: `argument_typed_return_resolution`,
  `random_int_big_overload_registry_facts`, `clock_derived_identifiers_declare_the_clock_overflow`,
  `aead_aad_default_padding`.
- `constant_time.rs` (195, 5) [src 1369-1600]: `keccak_core_is_branch_free`,
  `curve448_secret_paths_are_branch_free`, `curve25519_secret_paths_are_branch_free`,
  `gf448_mul_accumulators_fit_i63`, `reassembled_source_parses`.
Hazards: 1 citation, to code. Test LOC after: 2.

### 4.29 `src/codegen/builtins/datetime/mod.rs` inline (366, 11 tests) → `datetime/tests/`

- `mod.rs` (5): `use super::*;`, 2 `mod` lines; shared `NOW`, `TIME`.
- `registry_and_dispatch.rs` (154, 5) [src 655-873]: `datetime_registered_on_the_clean_room_registry`,
  `generic_dispatch_reaches_datetime`, `argument_typed_return_resolution`,
  `overload_selection_by_arity_yields_the_right_rewrite_target`, `default_padding_time_only`.
- `types_and_errors.rs` (208, 6) [src 875-1051]: `builtin_types_recognized`,
  `membership_via_generic_registry`, `expected_arguments_bespoke_phrasings`, `param_name_tables`,
  `reassembled_source_parses`, `members_declare_the_errors_they_raise`; helpers `INSTANT`,
  `DURATION`, `DATE`, `FIXED_OFFSET`, `PARSE`, `RESOLVE`, `ALL_CALLS`.
Hazards: `src/docs/spec/stdlib/02_datetime.md:197` cites
`[[src/codegen/builtins/datetime/mod.rs:__datetime_fixedOffset1]]`, whose only occurrence in
that file is a test string (old :847). The symbol is defined in
`src/codegen/builtins/datetime/func_fixed_offset.rs:8` (`FUNC __datetime_fixedOffset1`;
`grep -rn __datetime_fixedOffset1 src`). Repoint to
`[[src/codegen/builtins/datetime/func_fixed_offset.rs:__datetime_fixedOffset1]]` — the
definition, which is what the surrounding prose (fixedOffset semantics) describes. Test LOC after: 2.

### 4.30 `src/codegen/builtins/encoding/mod.rs` inline (490, 16 tests) → `encoding/tests/`

Two modules: `mod tests` (old 274-558, 10 tests) and `mod codepage_tables` (old 569-875, 6).
- `mod tests` → `#[cfg(test)] mod tests;`:
  - `tests/mod.rs` (12): `use` block (2), 2 `mod` lines; shared `project`.
  - `tests/registry_and_resolution.rs` (126, 8) [src 279-436]: `encoding_registered_on_the_clean_room_registry`,
    `generic_dispatch_reaches_encoding`, `non_overloaded_members_rewrite_to_their_internal_symbol`,
    `resolve_call_types_the_members`, `arity_is_unary_across_members`, `diagnostic_metadata_via_registry`,
    `reassembled_source_parses`, `augmented_project_injects_when_imported`.
  - `tests/imports_and_errors.rs` (110, 2) [src 438-557]: `augmented_project_noop_without_import`,
    `members_declare_the_errors_they_raise`.
- `mod codepage_tables` → `#[cfg(test)] #[path = "tests/codepage_tables/mod.rs"] mod codepage_tables;`
  (keeps its name and its `use super::helper_codepage_table::{BODY, VARIANTS};`, `super` =
  encoding; not declared by `tests/mod.rs`):
  - `tests/codepage_tables/mod.rs` (46): `use` block (4), 2 `mod` lines; shared `index_dir`,
    `read_index`, `arms`.
  - `tests/codepage_tables/vendored_index.rs` (78, 2) [src 577-726]: `codepage_tables_match_the_vendored_index_files`,
    `every_vendored_index_file_has_a_codepage_variant`; helpers `SENTINEL`, `scalars`.
  - `tests/codepage_tables/registration_and_digests.rs` (114, 4) [src 731-874]:
    `codepage_enum_is_registered_in_generator_order`, `iso_8859_8_i_shares_the_iso_8859_8_table`,
    `codepage_digests_match_the_vendored_index_files`, `codepage_roundtrip_counts_match_the_vendored_index_files`;
    helper `table_of`.
- `#[cfg(test)] pub(crate) fn augmented_hir_project` (old 267-273) stays in `encoding/mod.rs`.
Hazards: no citation into the moved ranges; the three `env!("CARGO_MANIFEST_DIR")` reads of
`tools/codepage-index/` are manifest-relative. Test LOC after: 10.

### 4.31 `src/codegen/builtins/general/mod.rs` inline (572, 29 tests) → `general/tests/`

- `mod.rs` (30): `use super::*;`, 3 `mod` lines; shared `types`, `ft`, `ALL_GENERAL`.
- `call_tables.rs` (177, 8) [src 701-936]: `is_general_call_covers_all`,
  `every_builtin_that_can_be_a_callback_returns_boolean`, `overridable_and_reserved`,
  `override_result_type_all_arms`, `call_param_names_all_arms`, `builtin_function_id_arms`,
  `builtin_function_id_for_type_specialized`, `builtin_function_id_for_type_non_predicate_shape`.
- `resolution.rs` (220, 15) [src 938-1181]: `filter_predicate_type_cases`, `resolve_error`,
  `resolve_len`, `resolve_type_name`, `resolve_to_string`, `resolve_to_int`, `resolve_to_int_enum`,
  `resolve_to_string_enum`, `only_to_int_and_to_string_consult_the_kind_oracle`,
  `resolve_to_float_fixed_byte`, `resolve_to_scalar_and_money`, `resolve_predicates`,
  `resolve_call_unknown`, `resolve_return_type_wrapper_delegates`, `nominal_return_type_matches_fast_oracle`;
  helpers `rt`, `AllEnums` + `impl TypeKinds for AllEnums`, `ColorIsAnEnum` + `impl TypeKinds
  for ColorIsAnEnum`, `rt_kinds`.
- `arguments_and_errors.rs` (148, 6) [src 1183-1357]: `expected_arguments_all_arms`,
  `helpers_exact_and_one_of`, `nested_function_parameters_split_at_the_top_level_arrow`,
  `registers_the_eighteen_globals_with_legacy_arities`, `conversions_declare_their_runtime_errors`,
  `error_arity_is_none_others_delegate`.
Hazards: 6 citations, all to code. Test LOC after: 2.

### 4.32 `src/codegen/builtins/mod.rs` inline (1102, 32 tests) → `builtins/tests/unit/` and `builtins/tests/tcp_tls_error_declaration/`

Three modules: `mod tests` (old 1045-1983, 28 tests), `mod plan111f_probe` (1985-2046, 1),
`mod tcp_tls_error_declaration_tests` (2047-2505, 3).

Name collision: `builtins/tests/` is the `builtins_codegen` suite root, so a plain `mod tests;`
here would load `builtins/tests/mod.rs` a second time. Declarations:
`#[cfg(test)] #[path = "tests/unit/mod.rs"] mod tests;` and
`#[cfg(test)] #[path = "tests/tcp_tls_error_declaration/mod.rs"] mod tcp_tls_error_declaration_tests;`
— module paths `codegen::builtins::tests` / `…::tcp_tls_error_declaration_tests` unchanged;
`builtins/tests/mod.rs` does NOT declare `unit` or `tcp_tls_error_declaration`.
- `tests/unit/mod.rs` (5): `use super::*;`, 4 `mod` lines.
- `tests/unit/named_arguments.rs` (193, 7) [src 1058-1307]: `qualified_builtin_type_requires_matching_package`,
  `no_named_argument_alias_repeats_across_positions`, `overloaded_param_name_tables_are_well_formed`,
  `a_named_call_may_omit_an_overloaded_builtins_trailing_optionals`,
  `the_fallback_never_leaves_out_a_required_parameter`,
  `a_named_call_never_selects_an_overload_with_a_gap_before_a_supplied_name`,
  `no_member_declaring_an_error_is_infallible`; helper `documented_builtins`.
- `tests/unit/fallibility.rs` (189, 4) [src 1309-1598]: `inline_builtin_fallibility_census`,
  `tostring_is_fallible_on_every_overload`, `the_raising_strings_members_are_not_infallible`,
  `replace_is_fallible_only_on_a_string`.
- `tests/unit/package_imports.rs` (198, 7) [src 1607-1851]: `toint_is_infallible_only_on_an_enum`,
  `inline_builtin_raw_supported_set`, `every_package_is_a_builtin_import`,
  `spec_section_18_package_list_matches_is_builtin_import`, `is_builtin_import_cases`,
  `is_builtin_type_aggregates`, `general_override_target_cases`; helpers `ColorIsAnEnum` +
  `impl TypeKinds for ColorIsAnEnum`, `ALL_BUILTIN_PACKAGES` (`= super::BUILTIN_IMPORTS`,
  resolves through `unit/mod.rs`'s glob).
- `tests/unit/aggregates.rs` (107, 10) [src 1853-1982]: `qualified_builtin_type_cases`,
  `resource_helpers`, `native_builtin_target_cases`, `inline_trap_unsupported_cases`,
  `call_return_type_name_aggregates`, `is_nonescaping_callback_arg_cases`,
  `is_builtin_call_aggregates`, `is_builtin_member_and_package_constant`,
  `package_constant_type_and_value`, `call_param_names_aggregates`.
- `tests/tcp_tls_error_declaration/mod.rs` (47): no `use` lines beyond the module's existing
  ones, 2 `mod` lines; shared `declared_rows`, `assert_rows`.
- `tests/tcp_tls_error_declaration/tcp.rs` (162, 1) [src 2053-2282]:
  `every_tcp_member_declares_the_errors_its_lowering_raises`.
- `tests/tcp_tls_error_declaration/tls.rs` (190, 2) [src 2290-2504]:
  `every_tls_member_declares_the_errors_its_lowering_raises`,
  `no_tcp_or_tls_error_list_reaches_the_infallibility_verdict`.
- `mod plan111f_probe` stays inline (14 LOC, 1 test). Test LOC left in `builtins/mod.rs`: 21.
Hazards: 20 citations, all to code symbols outside the moved ranges. The test that reads
`CARGO_MANIFEST_DIR` (old :1737) is manifest-relative. `scripts/coverage-exceptions.txt:36`
mentions `builtins/mod.rs` only in a comment (no entry). `.ai/resources-packages.md:376` names
`spec_section_18_package_list_matches_is_builtin_import` with no file path — unchanged.

### 4.33 `src/codegen/builtins/tls/gen_openssl.rs` inline (301, 9 tests) → `tls/gen_openssl/tests/`

`mod error_path_release_tests { … }` (old 3435-3812) →
`#[cfg(test)] #[path = "gen_openssl/tests/mod.rs"] mod error_path_release_tests;` (relative to
`tls/`; `use super::*;` keeps meaning `gen_openssl`). When a code letter turns `gen_openssl.rs`
into `gen_openssl/mod.rs`, it writes `#[path = "tests/mod.rs"]`.
- `mod.rs` (9): `use` block (4), 2 `mod` lines; shared `reloc_count`.
- `connect_and_accept.rs` (101, 4) [src 3446-3576]: `connect_alloc_fail_frees_ssl_ctx_and_fd`,
  `listen_bind_all_passes_a_non_null_service`, `connect_tls_fail_frees_ssl_and_ctx`,
  `accept_alloc_fail_frees_ssl_and_fd`; helper `resolves_between`.
- `read_and_listen.rs` (192, 5) [src 3592-3811]: `byte_read_fill_loop_writes_no_entry_descriptor`,
  `listen_lowers_with_min_proto_check`, `a_read_that_reports_the_close_makes_every_later_write_raise`,
  `read_names_a_departed_peer_connection_closed`, `accept_bounds_the_whole_handshake_with_one_deadline`.
Hazards: 4 citations, all to code. Test LOC after: 2.

### 4.34 `src/codegen/engine/analysis/borrow_get.rs` inline (522, 27 tests) → `borrow_get/tests/`

- `mod.rs` (48): `use` block (3), 3 `mod` lines; shared `TYPES`, `lower`, `function`, `classify`.
- `pinned_and_windowed.rs` (149, 9) [src 1710-1922]: `a_directly_matched_binding_after_place_forwarding_is_pinned`,
  `a_field_read_binding_over_an_immutable_list_is_pinned`, `a_field_read_binding_over_a_mut_list_is_windowed`,
  `a_write_to_the_container_inside_the_window_refuses_the_borrow`,
  `a_non_lookup_read_of_the_container_inside_the_window_refuses_the_borrow`,
  `a_get_or_with_a_built_default_is_no_borrow`, `get_with_set_on_the_same_slot_is_element_bound`,
  `a_with_over_the_binding_as_the_set_item_is_element_bound`, `a_let_bound_with_before_its_set_is_element_bound`.
- `element_binding.rs` (163, 11) [src 1925-2107]: `write_backs_in_both_if_arms_are_element_bound`,
  `a_write_back_in_one_if_arm_only_is_not_bound`, `a_read_of_the_binding_after_a_with_item_set_is_not_bound`,
  `two_elements_live_across_a_write_are_scratch_copies`, `an_escaping_or_get_or_binding_is_no_scratch_copy`,
  `a_write_to_the_container_between_the_bind_and_the_set_is_not_bound`,
  `an_element_update_that_is_not_adjacent_to_its_write_back_is_not_bound`,
  `an_element_read_after_its_write_back_is_not_bound`, `a_write_back_to_another_slot_is_not_bound`,
  `an_element_passed_whole_before_its_update_is_not_bound`, `a_global_container_needs_no_reachable_store`.
- `rebinding.rs` (165, 7) [src 2109-2297]: `the_single_expression_update_names_its_container_and_index`,
  `a_name_bound_in_two_sequential_loops_borrows_in_each`, `a_name_bound_in_sibling_arms_borrows_in_each`,
  `a_name_rebound_inside_the_same_loop_borrows_in_each`, `an_element_update_survives_a_later_read_only_rebind`,
  `an_unsafe_rebinding_stays_a_copy_beside_a_borrowing_one`, `a_function_without_rebinds_has_no_per_binding_roles`;
  helpers `rebound_roles`, `roles`.
Hazards: 4 citations to code; `.ai/collections.md` names `collect_borrow_gets` (code). Test LOC after: 2.

### 4.35 `src/codegen/engine/analysis/handover.rs` inline (560, 10 tests) → `handover/tests/`

- `mod.rs` (68): `use` block (5), 3 `mod` lines; shared `lower`, `function`, `callees`,
  `HELPERS`, `probe`, `approved`, `approved_owning`.
- `argument_table.rs` (189, 3) [src 901-1096]: `collect_handover_args_follows_the_hand_derived_table`,
  `a_global_argument_is_never_handed_over`, `a_resource_bearing_argument_is_never_handed_over`.
- `parameters_and_records.rs` (212, 6) [src 1100-1335]: `a_fresh_temporary_argument_is_handed_over`,
  `an_owned_parameter_is_handed_on_only_in_a_variant`, `a_bare_string_parameter_is_never_handed_over`,
  `a_record_is_handed_over_only_when_every_field_is`, `consumable_params_follows_the_hand_derived_table`,
  `a_returned_with_over_scalar_fields_consumes_its_owner`; helpers `arg_updates`, `consumable_of`.
- `borrowed_returns.rs` (87, 1) [src 1341-1427]: `a_parameter_returned_as_a_borrow_is_never_handed_over`.
Hazards: 2 citations to code. The `#[cfg(test)] fn len` accessor stays in `handover.rs`
(test LOC after: 9).

### 4.36 `src/codegen/engine/analysis/last_use.rs` inline (685, 6 tests) → `last_use/tests/`

Part layout.
- `mod.rs` (142 + `use` block (3) + 3 part pairs) [src 1070-1223]: `lower`, `lower_unoptimized`,
  `function`, `find_ops`, `ops`, `binds`, `binds_from_local`, `assigns_field`, `is_append_call`,
  `appends_to`, `appends_place_to`, `returns_value`, `local`, `member`.
- `shape_table.rs` (148) [src 1227-1373]: `SHAPES` → `pub(super)` (1).
- `hand_derived_table.rs` (195, 3) [src 1378-1587]: `hand_built`, `zero` → `pub(super)` (2);
  `collect_last_use_moves_follows_the_hand_derived_table`,
  `collect_last_use_moves_views_of_a_match_scrutinee`,
  `collect_last_use_moves_never_moves_a_by_ref_capture`.
- `decoders_and_chains.rs` (206, 3) [src 1591-1827]: `DECODERS`, `CHAIN_BACKTRACKER`;
  `collect_last_use_moves_never_moves_a_resource_bearing_value`,
  `collect_last_use_moves_finds_the_five_decoder_append_sites`,
  `collect_last_use_moves_covers_the_chain_backtracker_views`.
Hazards: no citation; `.ai/codegen-invariants.md` names the analysis (code). Test LOC after: 2.

### 4.37 `src/codegen/engine/mir/mir.rs` inline (744, 16 tests) → `mir/mir/tests/`

The module is `mir::mir` (file `mir/mir.rs`), so its children live in `mir/mir/`.
- `mod.rs` (19): `use` block (2), 5 `mod` lines; shared `assert_round_trips`.
- `op_round_trips.rs` (112, 3) [src 962-1105]: `code_op_round_trips_through_mir`,
  `renamed_ops_are_neutral_but_select_back_identically`, `v128_ops_are_neutral_but_select_back_identically`.
- `lane_and_fusion.rs` (199, 5) [src 1119-1345]: `v128_lane_semantics_contract`,
  `lower_then_select_is_identity`, `flag_pairs_fuse_and_expand_identically`,
  `fused_op_carries_operands_and_condition`, `syscall_carry_check_fuses_and_expands_identically`.
- `carry_and_adrp_fusion.rs` (117, 4) [src 1352-1490]: `explicit_carry_chain_round_trips`,
  `multi_branch_compare_fuses_with_share`, `addr_of_fuses_and_expands_identically`,
  `unpaired_or_mismatched_adrp_is_not_fused`.
- `family_sweep.rs` (219, 1) [src 1502-1742]: `round_trip_sweep_over_every_op_family` (one
  218-LOC test fn).
- `neutral_tokens.rs` (81, 3) [src 1747-1847]: `arena_base_renames_and_realizes_identically`,
  `invariant_registers_are_neutral_tokens`, `reloc_intents_are_neutral_and_realize_to_aarch64_kinds`.
Hazards: 23 citations, all to code. Requires §4.1 in the same commit (the 102 vreg literals and
138 physical-register lines). The `#[cfg(test)] fn is_v128` arm inside `mir_ops!` stays
(test LOC after: 6).

### 4.38 `src/codegen/registry/mod.rs` inline (2695, 62 tests) → `registry/tests/`

Modules: `mod qualification_tests` (old 5208-5320, 8 tests, 79 LOC), `mod tests` (5322-8034,
50), `mod raw_result_block_ownership` (8036-8476, 4), plus `#[cfg(test)]` fns between :3331 and
:3499 (`self_update_shaped`, `is_string_self_update_type`, `strip_res`, `resolve_bound`,
`occurs`, the two-sided unifier at :3407, `resolve_call`).
- `mod tests` → `#[cfg(test)] mod tests;` (`registry/` has no `tests` dir today):
  - `tests/mod.rs` (114): `use super::*;`, 12 `mod` lines; shared `func`, `prop`, `rec`,
    `variant`, `uni`, `res`, `mfb_impl`, `param`, `call`, `list_of`, `map_of`, `generic_impl`, `th`.
  - `source_forms_and_vocabulary.rs` (170, 4) [src 5342-5571]: `no_member_defines_more_source_forms_than_its_descriptor_declares`,
    `range_and_index_parameters_use_the_documented_vocabulary`, `the_consuming_parameters_name_real_members`,
    `the_placing_operands_name_real_members`.
  - `row_shapes_and_strict_matching.rs` (186, 5) [src 5601-5871]: `every_registry_row_has_the_shape_its_builder_requires`,
    `the_consuming_parameter_index_agrees_across_overloads`, `set_group_items_is_the_consuming_parameter`,
    `a_builtin_record_property_may_carry_a_res_type`,
    `strict_matching_separates_resource_params_from_value_union_params`.
  - `builtin_value_identity.rs` (152, 2) [src 5882-6065]: `strict_matching_holds_builtin_value_types_to_their_identity`,
    `descriptor_named_types_are_bare_nominals`.
  - `builders_round_trip.rs` (41, 1) [src 6069-6235]: `package_constants_and_overrides_round_trip_through_the_builders`.
  - `overload_selection.rs` (97, 4) [src 6237-6379]: `constant_and_override_boundary_fns_fall_through_for_unowned_names`,
    `frozen_registry_exposes_the_csv_package`, `an_overload_is_two_implementations_differing_by_parameter_type`,
    `select_binds_type_variables_and_substitutes_the_return`.
  - `thread_probe.rs` (161, 2) [src 6391-6574]: `thread_probe_waitfor_receive_send_poll`,
    `thread_probe_transfer_accept_resource_plane`.
  - `thread_probe_start_and_generics.rs` (207, 4) [src 6576-6845]: `thread_probe_start_extraction_and_resource_echo`,
    `bug_824_plain_callback_param_accepts_isolated_concrete`, `thread_probe_parse_isolated_func_arg`,
    `resolved_parameter_type_infers_a_generic_position_from_the_call`.
  - `union_variables_and_select.rs` (234, 4) [src 6847-7096]: `a_top_level_variable_bound_to_a_union_accepts_its_variant`,
    `reduce_widens_a_variant_initial_value_and_never_narrows_a_union`,
    `select_rejects_inconsistent_variable_binding`, `select_unknown_argument_is_a_wildcard`;
    helper `UnionUOverV` + `impl TypeKinds for UnionUOverV`.
  - `select_and_mfb_rendering.rs` (242, 10) [src 7098-7359]: `select_scalar_mismatch_and_wrong_arity_are_rejected`,
    `public_member_for_rewrite_names_the_member_a_user_calls`, `contains_var_detects_generic_types`,
    `unify_substitute_over_map_entry_and_result_variants`, `add_function_takes_a_function_value`,
    `resolve_func_finds_the_owning_package`, `mfb_body_carries_a_rewrite_target_and_optional_fast_path`,
    `get_mfb_renders_helper_functions_before_member_bodies`,
    `a_debug_split_helper_renders_the_half_matching_the_build`,
    `get_mfb_is_empty_when_the_package_has_no_mfb_member`; helpers `intrinsic`, `rewrite_impl`,
    `sample_fast_path`.
  - `declaration_rendering.rs` (231, 8) [src 7361-7597]: `add_record_renders_the_type_declaration`,
    `add_union_renders_the_union_declaration`, `add_enum_renders_the_enum_declaration`,
    `enum_variant_advisory_is_keyed_by_package_enum_and_member`, `enums_resolve_as_builtin_types`,
    `add_resource_records_name_export_and_close_function`, `get_mfb_places_enums_between_unions_and_helpers`,
    `get_mfb_orders_imports_records_unions_helpers_functions`; helpers `enum_variant`, `enm`.
  - `descriptor_and_gates.rs` (198, 3) [src 7599-7821]: `descriptor_round_trips_the_full_surface`,
    `is_imported_by_checks_the_program_imports`, `a_named_type_opens_its_gate_and_closes_the_exact_complement`.
  - `constructed_records_and_agreement.rs` (148, 3) [src 7828-8033]: `a_constructed_record_opens_its_gate_and_closes_the_complement`,
    `typed_and_string_resolution_agree`, `agreed_argument_type_answers_where_overloads_agree`.
- `mod raw_result_block_ownership` →
  `#[cfg(test)] #[path = "tests/raw_result_block_ownership/mod.rs"] mod raw_result_block_ownership;`
  (name and its `use super::runtime_specs;` unchanged; not declared by `tests/mod.rs`):
  - `mod.rs` (135 + `use` block (3) + 2 `mod` lines) [src 8070-8223]: `FOREIGN_ARENA_RESULTS`,
    `CALLER_ARENA_BLOCK_RESULTS`, `carries_a_block`, `is_generic`.
  - `companion_calls.rs` (49, 1) [src 8236-8283]: `no_companion_calls_a_source_generic_collections_member`.
  - `helper_results.rs` (158, 3) [src 8285-8475]: `STRING_RESULT_HELPERS`;
    `every_block_returning_runtime_helper_is_classified`, `every_string_returning_runtime_helper_is_marked_fresh`,
    `no_thread_family_result_is_ever_freed_by_the_calling_thread`.
- `mod qualification_tests` and the `#[cfg(test)]` helper fns stay in `registry/mod.rs`:
  remaining test LOC 200 (`span.py` total 2695 − moved 2160 − 339 + 4 declaration lines).
Hazards: `src/docs/spec/language/18_builtin-functions.md:245`
`[[src/codegen/registry/mod.rs:range_and_index_parameters_use_the_documented_vocabulary]]` →
`[[src/codegen/registry/tests/source_forms_and_vocabulary.rs:range_and_index_parameters_use_the_documented_vocabulary]]`
(silent: `registry/mod.rs` still exists). `.ai/man-content.md:117-118` names the same test
"(`src/codegen/registry/mod.rs`)" → `src/codegen/registry/tests/source_forms_and_vocabulary.rs`.
The other 32 citations name code. `no_type_strings.rs` names registry code — unchanged. When
plan-176-M splits the registry code, the declarations move with whichever file keeps them; the
paths stay `registry/tests/…`.

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

Every phase uses the same gate, run in this order (time estimates are guesses, not
measurements):

- **G0 (before the phase's first edit):** `cargo test -p mfb --bin mfb -- --list 2>/dev/null | grep -c ': test$' > /tmp/h_before.txt`
  and `cargo check --all-targets 2>&1 | grep -c '^warning' > /tmp/h_warn_before.txt` (est. 5 min
  cold, 1 min warm).
- **G1:** the same two commands after the phase's edits → numbers equal the G0 files.
- **G2:** run the moved modules' tests (filter per phase) → all pass, 0 failed.
- **G3:** `cargo test --test architecture_guards --test no_type_strings --test no_operator_strings`
  and `cargo test -p mfb --bin mfb spec_citations_resolve` → all pass (est. 3 min).
- **G4:** `cargo test --test file_size_budget 2>&1 | grep -E '^  (NEW|GREW)'` → no output, and
  every `TIGHTEN`/`STALE` row names a file of this phase; then
  `MFB_LOC_TIGHTEN=1 cargo test --test file_size_budget` and rerun →
  `grep -cE '^  (NEW|GREW|STALE|TIGHTEN)'` → `0`. Commit `tests/guards/file_size_ceilings.txt`
  (and, when the phase adds exemptions, `tools/loc-census/census.py`) with the phase (est. 2 min).
- **G5:** `cargo fmt --check` → no output (est. 20 s).

### Phase 1 — `builtins_codegen` topic suites

Twelve suites that split into topics with no visibility edit and no hazard.

- [ ] Split `src/codegen/builtins/canvas/helper_render/tests/mod.rs` per §4.2.
- [ ] Split `src/codegen/builtins/tests/canvas.rs` → `canvas/` per §4.3.
- [ ] Split `src/codegen/builtins/tests/collections.rs` → `collections/` per §4.4.
- [ ] Split `src/codegen/builtins/tests/inplace.rs` → `inplace/` per §4.10.
- [ ] Split `src/codegen/builtins/tests/inplace_compact.rs` → `inplace_compact/` per §4.11.
- [ ] Split `src/codegen/builtins/tests/nir_json.rs` → `nir_json/` per §4.14.
- [ ] Split `src/codegen/builtins/tests/nir_validation.rs` → `nir_validation/` per §4.15.
- [ ] Split `src/codegen/builtins/tests/optimizer_globals.rs` → `optimizer_globals/` per §4.16
      (`global` stays private: it sits with its only users).
- [ ] Split `src/codegen/builtins/tests/os.rs` → `os/` per §4.17.
- [ ] Split `src/codegen/builtins/tests/plan_validation.rs` → `plan_validation/` per §4.18.
- [ ] Split `src/codegen/builtins/tests/platform_hooks.rs` → `platform_hooks/` per §4.20.
- [ ] Split `src/codegen/builtins/tests/validation.rs` → `validation/` per §4.21.
- [ ] G4: tighten the ceilings for the twelve files.

Acceptance: the twelve suites run from their topic modules and the counts did not move.
  Check: G1 → equal; G2 `cargo test -p mfb --bin mfb builtins_codegen:: helper_render::` → 0
  failed, and `cargo test -p mfb --bin mfb -- --list 2>/dev/null | grep -cE 'builtins_codegen::(canvas|collections|inplace|inplace_compact|nir_json|nir_validation|optimizer_globals|os|plan_validation|platform_hooks|validation)::|helper_render::tests::'`
  → 10+11+7+11+6+11+7+8+4+7+9+11 = 102; G3; G4; G5 (est. 12 min).
Commit: —

### Phase 2 — `builtins_codegen` helper libraries and the three exemptions

The part layouts with visibility edits, the two citations into vanishing files, and the three
exempt files.

- [ ] Split `src/codegen/builtins/tests/corpus.rs` → `corpus/` per §4.5; add the `exempt` row
      for `corpus/fixture_list.rs` to `tests/guards/file_size_ceilings.txt` and to the
      `EXEMPT` table of `tools/loc-census/census.py` (§4.0 reason text).
- [ ] Split `src/codegen/builtins/tests/diagnostics.rs` → `diagnostics/` per §4.8; add the
      `exempt` row for `diagnostics/fixture_list.rs` in both places.
- [ ] Split `src/codegen/builtins/tests/platform_failure.rs` → `platform_failure/` per §4.19;
      add the `exempt` row for `platform_failure/forwarding.rs` in both places.
- [ ] Split `src/codegen/builtins/tests/declared_errors_sweep.rs` → `declared_errors_sweep/`
      per §4.7 (2 visibility edits + the two `pub(super) use` re-exports).
- [ ] Split `src/codegen/builtins/tests/declared_errors_all.rs` → `declared_errors_all/` per §4.6.
- [ ] Split `src/codegen/builtins/tests/general_declared_errors.rs` → `general_declared_errors/`
      per §4.9; repoint `src/docs/spec/language/18_builtin-functions.md:55` to
      `general_declared_errors/mod.rs:OVERLOADS` and the `//!` prose in
      `src/codegen/builtins/totality_proof.rs` to `tests/general_declared_errors/` (evidence:
      `grep -rn "general_declared_errors.rs" src` → 2 hits before, 0 after).
- [ ] Split `src/codegen/builtins/tests/mfb_body_declared_errors.rs` → `mfb_body_declared_errors/`
      per §4.12 (40 visibility edits); repoint `src/docs/spec/tooling/04_audit-format.md:311`
      to `mfb_body_declared_errors/rewrite_sweep.rs:rewrite_sweep`.
- [ ] Split `src/codegen/builtins/tests/nir_fault.rs` → `nir_fault/` per §4.13 (13 visibility edits).
- [ ] G4: tighten; the three new `exempt` rows stay.

Acceptance: the helper libraries serve their four callers unchanged and the exemptions are
recorded in both tables.
  Check: G1 → equal; G2 `cargo test -p mfb --bin mfb builtins_codegen::` → 0 failed, and
  `cargo test -p mfb --bin mfb -- --list 2>/dev/null | grep -cE 'builtins_codegen::(corpus|diagnostics|platform_failure|declared_errors_sweep|declared_errors_all|general_declared_errors|mfb_body_declared_errors|nir_fault)::'`
  → 2+4+1+0+23+1+2+1 = 34; `grep -c '^exempt test src/codegen/builtins/tests/' tests/guards/file_size_ceilings.txt`
  → 3 and `python3 -I tools/loc-census/census.py --seed | grep -c '^exempt test src/codegen/builtins/tests/'`
  → 3; G3 (the two spec repoints keep `spec_citations_resolve` green); G4; G5 (est. 12 min).
Commit: —

### Phase 3 — Engine tests and the architecture-guard change

The two splits that need the guard change land with it; the other engine moves share the gate.

- [ ] `tests/guards/architecture_guards.rs`: §4.1 — `is_physical_scan_exempt`,
      `no_hand_picked_vreg` and `emission_lines` treat a manifest-relative path containing
      `/tests/` as test code. Commit message carries the 4-point justification of §4.1.
- [ ] Split `src/codegen/engine/regalloc/tests.rs` → `regalloc/tests/` per §4.24.
- [ ] Split `src/codegen/engine/mir/mir.rs`'s inline tests → `mir/mir/tests/` per §4.37.
- [ ] Split `src/codegen/engine/tests/tests.rs` → `engine/tests/tests/` per §4.25.
- [ ] Split `src/codegen/engine/analysis/borrow_get.rs`'s inline tests per §4.34.
- [ ] Split `src/codegen/engine/analysis/handover.rs`'s inline tests per §4.35.
- [ ] Split `src/codegen/engine/analysis/last_use.rs`'s inline tests per §4.36 (3 visibility edits).
- [ ] G4: tighten the ceilings for the six files.

Acceptance: the guards still bind production code and no longer see the moved fixtures.
  Check: G1 → equal; G2 `cargo test -p mfb --bin mfb codegen::engine::` → 0 failed, and
  `cargo test -p mfb --bin mfb -- --list 2>/dev/null | grep -cE 'engine::(regalloc::tests|mir::mir::tests|tests::tests|analysis::(borrow_get|handover|last_use)::tests)::'`
  → 16+16+16+27+10+6 = 91; G3 → pass; negative check: add the line
  `fn zz() -> String { format!("xmm{}", 1) }` to a production file outside every allow-list,
  `src/codegen/engine/analysis/handover.rs`, above its `#[cfg(test)]`, run
  `cargo test --test architecture_guards shared_lowering_names_no_physical_register` → FAILS
  naming that line, then revert the line; G4; G5 (est. 14 min).
Commit: —

### Phase 4 — Builtin package inline tests and the TLS suites

- [ ] Split `src/codegen/builtins/audio/mod.rs`'s inline tests per §4.26.
- [ ] Split `src/codegen/builtins/canvas/mod.rs`'s inline tests per §4.27; repoint
      `src/docs/spec/app/06_canvas.md` cites of `draw_item_variant_set_is_frozen`,
      `no_canvas_type_reaches_itself` (→ `canvas/tests/variant_sets.rs`) and
      `canvas_text_has_one_glyph_walk` (→ `canvas/tests/resources_and_source.rs`).
- [ ] Split `src/codegen/builtins/crypto/mod.rs`'s inline tests per §4.28.
- [ ] Split `src/codegen/builtins/datetime/mod.rs`'s inline tests per §4.29; repoint
      `src/docs/spec/stdlib/02_datetime.md:197` to
      `[[src/codegen/builtins/datetime/func_fixed_offset.rs:__datetime_fixedOffset1]]`.
- [ ] Split `src/codegen/builtins/encoding/mod.rs`'s two inline modules per §4.30.
- [ ] Split `src/codegen/builtins/general/mod.rs`'s inline tests per §4.31.
- [ ] Split `src/codegen/builtins/tls/gen_openssl.rs`'s inline tests per §4.33.
- [ ] Split `src/codegen/builtins/tls/gen_macos/tests.rs` → `gen_macos/tests/` per §4.22.
- [ ] Move `src/codegen/builtins/tls/gen_schannel_tests.rs` → `tls/gen_schannel/tests/` per
      §4.23 (one `#[path]` edit in `gen_schannel.rs`).
- [ ] G4: tighten the ceilings for the nine files.

Acceptance: every package's tests run from `tests/` modules, and the four repointed cites name
the file that holds the symbol.
  Check: G1 → equal; G2 `cargo test -p mfb --bin mfb codegen::builtins::` → 0 failed, and
  `cargo test -p mfb --bin mfb -- --list 2>/dev/null | grep -cE 'builtins::(audio::tests|canvas::tests|crypto::tests|datetime::tests|encoding::(tests|codepage_tables)|general::tests|tls::gen_openssl::error_path_release_tests|tls::gen_macos::tests|tls::gen_schannel::schannel_tests)::'`
  → 11+14+14+11+16+29+9+23+11 = 138; `grep -rn "canvas/mod.rs:\(draw_item_variant_set_is_frozen\|no_canvas_type_reaches_itself\|canvas_text_has_one_glyph_walk\)\|datetime/mod.rs:__datetime_fixedOffset1" src/docs/spec`
  → no output; G3; G4; G5 (est. 12 min).
Commit: —

### Phase 5 — `builtins/mod.rs` and `registry/mod.rs` (largest blast radius)

- [ ] Split `src/codegen/builtins/mod.rs`'s `mod tests` and `mod tcp_tls_error_declaration_tests`
      per §4.32 (two `#[path]` declarations; `plan111f_probe` stays inline).
- [ ] Split `src/codegen/registry/mod.rs`'s `mod tests` and `mod raw_result_block_ownership`
      per §4.38; repoint `src/docs/spec/language/18_builtin-functions.md:245` and
      `.ai/man-content.md:117-118` to `registry/tests/source_forms_and_vocabulary.rs`.
- [ ] G4: tighten the ceilings for the two files.

Acceptance: the two largest hosts keep their module paths.
  Check: G1 → equal; G2 `cargo test -p mfb --bin mfb codegen::builtins::tests:: codegen::builtins::tcp_tls_error_declaration_tests:: codegen::registry::`
  → 0 failed, and `cargo test -p mfb --bin mfb -- --list 2>/dev/null | grep -cE '^codegen::(builtins::(tests|tcp_tls_error_declaration_tests|plan111f_probe)|registry::(tests|raw_result_block_ownership|qualification_tests))::'`
  → 28+3+1+50+4+8 = 94 (`builtins::tests::` here is the unit module, not `builtins_codegen`);
  `python3 -I tools/loc-census/census.py --file src/codegen/registry/mod.rs` → second field
  ≤ 250; G3; G4; G5 (est. 12 min).
Commit: —

## Validation Plan

- Tests: no test is added or removed. Per phase the whole-binary `--list` count is unchanged
  (G1) and the moved modules' tests pass (G2); the per-phase `--list` greps sum to
  102 + 34 + 91 + 138 + 94 = 459, every test in H's 37 files.
- Coverage check: per-file coverage is not a plan-176 gate — author, 2026-10-17. Evidence: at 7a4b09970,
  `sh scripts/coverage.sh --bins` in a detached worktree (59 min; 4989 unit tests passed), then
  `sh scripts/coverage-check.sh` exited 1. 241 `src/` files are already below the 98% per-file
  floor ("Files below 98% line coverage (GATE FAILURE)"), and overall coverage is 93.67%.
  Examples among plan-176 hosts: `src/ir/shape.rs` 93.21%,
  `src/codegen/engine/control/builder_control.rs` 75.55%, `src/debugger/values.rs` 86.66%. A
  pure move changes no executed line, and moving an inline test block only removes test lines
  from its host's denominator. The pre-existing gate failure is reported to the owner
  separately and is not a plan-176 task.
- Runtime proof: the Phase 3 negative check (a constructed register name in production code
  still fails `shared_lowering_names_no_physical_register`) proves the narrowed guard binds.
- Doc sync: `src/docs/spec/app/06_canvas.md`, `src/docs/spec/language/18_builtin-functions.md`,
  `src/docs/spec/tooling/04_audit-format.md`, `src/docs/spec/stdlib/02_datetime.md`,
  `.ai/man-content.md`, `src/codegen/builtins/totality_proof.rs` prose. No compiler contract
  changes, so no other spec or `mfb man` topic changes.
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
- A single test-side unit over 250 (an embedded program, a const table / fixture list, a single
  `#[test]` fn, a single trait impl) gets its own file and an `exempt` row with its reason in
  `tests/guards/file_size_ceilings.txt` and in `tools/loc-census/census.py`'s `EXEMPT` table, in
  the same commit — owner, 2026-10-14. H's three: `src/codegen/builtins/tests/corpus/fixture_list.rs`,
  `src/codegen/builtins/tests/diagnostics/fixture_list.rs`,
  `src/codegen/builtins/tests/platform_failure/forwarding.rs` (§4.0).
- Extracted src test modules go into `foo/tests/mod.rs` + topic files — plan-176-A. Where the
  module name is not `tests`, a `#[path]` puts it under a `tests/` directory and the name is
  kept — author, 2026-10-14.
- Guards that classify test code by file name are updated in the same commit as the move that
  needs them, with the 4-point justification — author, 2026-10-14. H's change is the `/tests/`
  component in `architecture_guards` (§4.1, Phase 3).
- Pre-existing defects found while mapping are fixed in the letter that touches them — author,
  2026-10-14. H fixes the `[[…datetime/mod.rs:__datetime_fixedOffset1]]` cite (it named a test
  string, not the definition) and the stale `.ai/man-content.md` path.
- Glob-cycle imports (`use super::*;` in parts, `use part::*;` in `mod.rs`) compile without
  warnings — verified (`/tmp/loc/probe2`); an intermediate `tests/mod.rs` glob consumed only by
  children does not warn — verified (`/tmp/hij_probe`).
- Per-file coverage is not a plan-176 gate; the measured evidence is in §Validation Plan
  "Coverage check" — author, 2026-10-17.

## Corrections

## Summary

H is pure motion of test code into `tests/` directory modules: 37 hosts, 459 tests, 0 renamed,
3 exempt files. The engineering risk is in the architecture-guard change (narrowly widened to
the `/tests/` component, proven against the 17 test directories and pinned by a negative
check), in `builtins/mod.rs`'s name collision with the `builtins_codegen` root (resolved with
`#[path]`), and in the 2160-LOC registry move, which goes last. Host code, the `builtins_codegen`
wiring and every module name are untouched.
