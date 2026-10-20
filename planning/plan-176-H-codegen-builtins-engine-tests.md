# plan-176-H: Codegen builtins, registry and engine test code

Last updated: 2026-10-18
Effort: medium (1h–2h)
Depends on: plan-176-G

H moves the over-limit test code under `src/codegen/builtins`, `src/codegen/registry` and
`src/codegen/engine` into test-only files under `tests/` directories. The rule (owner,
2026-10-18): a code file holds at most 1000 code LOC and at most 250 inline test LOC; a
test-only file holds at most 1000 LOC; split parts aim for ≤ 900. 15 files are in scope: 2
test-only files over 1000 (both suites of the `builtins_codegen` module) and 13 code files whose
inline `#[cfg(test)]` code is over 250. Each inline block becomes ONE test-only file, except
`registry/mod.rs`'s `mod tests` (2160 LOC), which is split into three. No test is added,
removed or renamed. Only the 38 registry tests that land in a topic file gain a path segment
(`registry::tests::thread_probe_waitfor_receive_send_poll` →
`registry::tests::selection_and_thread_probes::thread_probe_waitfor_receive_send_poll`). Every
other module path is unchanged. Host code does not move. The four hosts whose code is over 1000
are split by letters M (registry), S (canvas), T (gen_openssl) and U (borrow_get).

H also lands the one guard change the later test letters depend on. The three file-name
exemptions in `tests/guards/architecture_guards.rs` learn that a `tests` directory component
means test code. The change lands in the same commit as the first move that needs it (the MIR
tests).

The single outcome: after H lands, `cargo test --test file_size_budget` prints no `inline` or
`test` row for any of H's 15 files or the files H creates, and
`tests/guards/file_size_ceilings.txt` has no `ceiling inline` or `ceiling test` row for them.
The `ceiling code` rows of `canvas/mod.rs`, `tls/gen_openssl.rs`, `analysis/borrow_get.rs` and
`registry/mod.rs` stay; letters S, T, U and M own them.

References:

- `planning/plan-176-A-file-size-guard.md`: the rule, §Decisions, the counting rules, and
  "Splitting an over-budget file" in `.ai/build-tooling.md` (src tests: `foo/tests/mod.rs`,
  plus topic files when over 1000).
- `.ai/testing-gates.md` § "Splits must sweep man AND spec citations".
- `.ai/codegen-invariants.md`: register and vreg invariants the architecture guards enforce
  (bug-721).
- `.ai/resources-packages.md`: builtin-package authoring seams. It names
  `spec_section_18_package_list_matches_is_builtin_import` without a path, so it is unchanged.
- `.ai/man-content.md` names `range_and_index_parameters_use_the_documented_vocabulary` with
  the path `src/codegen/registry/mod.rs`. Phase 4 repoints it.
- Spec topics whose citations move: `src/docs/spec/app/06_canvas.md` (3),
  `src/docs/spec/language/18_builtin-functions.md` (1), `src/docs/spec/tooling/04_audit-format.md`
  (1), `src/docs/spec/stdlib/02_datetime.md` (1).
- Guards: `tests/guards/architecture_guards.rs` (a 370-LOC test-only file, under the limit, so
  no letter splits it), `tests/guards/no_type_strings.rs`, `tests/guards/no_operator_strings.rs`.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-176-G is complete and archived | `ls planning/plan-176-G-* planning/completed/plan-176-G-*` → only `planning/completed/…` | MET (2026-10-19) |
| The file-size guard exists and passes | `cargo test --test file_size_budget` → passes | MET (2026-10-19) |
| H's files and the docs/guards H edits have no uncommitted changes | `git status --porcelain -- src/codegen/builtins src/codegen/registry src/codegen/engine src/audit/collect/source.rs src/docs/spec/app/06_canvas.md src/docs/spec/language/18_builtin-functions.md src/docs/spec/tooling/04_audit-format.md src/docs/spec/stdlib/02_datetime.md .ai/man-content.md tests/guards/architecture_guards.rs` → empty | MET (2026-10-19) |

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

- After the move, every code file in §2's table holds ≤ 250 inline test LOC
  (`python3 -I tools/loc-census/census.py --file <path>` → second field ≤ 250).
- Every test-only file H creates holds ≤ 1000 LOC, and in practice ≤ 900 (same command,
  second field).
- `cargo test --test file_size_budget` prints no `inline` or `test` row naming an H path. The
  ceiling file has no `ceiling inline` / `ceiling test` row for an H path and no `exempt` row.
- The unit-test binary lists the same number of tests before and after each phase.
- `tests/guards/architecture_guards.rs` treats a path with a `tests` directory component as
  test code in `is_physical_scan_exempt`, `no_hand_picked_vreg` and `emission_lines`.

### Non-goals (explicit constraints)

- No behavior change. A test body may change only in these ways: it moves, it is dedented when
  it leaves an inline `mod … { }`, it gains a leading `use super::*;` in a topic or part file,
  or it takes one of the visibility edits listed in §4.
- No `#[test]` fn renamed, added or removed. No `[[test]]` target renamed (H touches none).
- No host code moves. Every code file keeps its path and its non-test items. Code splits are
  letters M (registry), S (canvas), T (gen_openssl) and U (borrow_get).
- Every existing module name is kept: `tests`, `codepage_tables`, `error_path_release_tests`,
  `tcp_tls_error_declaration_tests`, `raw_result_block_ownership`, and the `builtins_codegen`
  suites `mfb_body_declared_errors` and `platform_failure`. Where the name is not `tests`, a
  `#[path]` puts its file under a `tests/` directory.
- No guard weakens: no production file becomes exempt from any guard (§4.1 proof).
- The `builtins_codegen` wiring in `src/codegen/engine/tests/mod.rs` is untouched. It is
  declared there for the coverage reason its comment documents.
- The file-size guard gets no exemption row. The only exemption in plan-176 is a file's own
  `GENERATED` banner.

## 2. Current State

LOC comes from `/tmp/loc/census.json`, written at HEAD `ce7a995ca` on 2026-10-18 by the
planning census `python3 -I /tmp/loc/census.py "$PWD"`. Fields: `test` is the inline test LOC
of a code file and the total LOC of a test-only file. Once plan-176-A lands, the same number
comes from `python3 -I tools/loc-census/census.py --file <path>`. Tests are counted with
`grep -cE '#\[(tokio::)?test\]' <file>` at `ce7a995ca`.

| File | Kind | LOC of that kind | Tests |
|---|---|---|---|
| `src/codegen/builtins/tests/mfb_body_declared_errors.rs` | test | 1856 | 2 |
| `src/codegen/builtins/tests/platform_failure.rs` | test | 1088 | 1 |
| `src/codegen/builtins/audio/mod.rs` | inline | 296 | 11 |
| `src/codegen/builtins/canvas/mod.rs` | inline | 421 | 14 |
| `src/codegen/builtins/crypto/mod.rs` | inline | 574 | 15 |
| `src/codegen/builtins/datetime/mod.rs` | inline | 406 | 11 |
| `src/codegen/builtins/encoding/mod.rs` | inline | 488 | 16 |
| `src/codegen/builtins/general/mod.rs` | inline | 572 | 29 |
| `src/codegen/builtins/mod.rs` | inline | 1104 | 32 |
| `src/codegen/builtins/tls/gen_openssl.rs` | inline | 301 | 9 |
| `src/codegen/engine/analysis/borrow_get.rs` | inline | 522 | 27 |
| `src/codegen/engine/analysis/handover.rs` | inline | 560 | 10 |
| `src/codegen/engine/analysis/last_use.rs` | inline | 685 | 6 |
| `src/codegen/engine/mir/mir.rs` | inline | 744 | 16 |
| `src/codegen/registry/mod.rs` | inline | 2695 | 62 |

Totals: 15 files, 12,312 LOC of the limited kind, 261 tests. The test count is the loop over
the table's paths summing `grep -cE '#\[(tokio::)?test\]'` → 261.

The module extents below (`[lines a-b]`) are as of `ce7a995ca`. Item and test NAMES are
authoritative if lines drift. Each extent's LOC uses the census counting rules on that line
range: `python3 -I tools/loc-census/census.py --file <path> --lines A B` after plan-176-A; at
planning, the same rules in `/tmp/loc/rw-H/rng.py` and `/tmp/loc/rw-H/mods.py`.

### Verified properties

- **A `tests/mod.rs` whose `use super::*;` is consumed only by its children's globs does not
  warn, and a private `use` in the host reaches a grandchild topic.** Probe `/tmp/hij_probe`
  used this layout: a host `src/host.rs` with a private `use std::path::PathBuf;` and
  `#[cfg(test)] mod tests;`, plus `src/host/tests/mod.rs` = `use super::*; mod topic_a; mod
  topic_b;`. `cargo test` → no `warning`, 2 passed, and `--list` shows
  `host::tests::topic_a::a`. This is the layout of `registry/tests/` (§4.16).
- **A data-only part file must not start with `use super::*;`.** Same probe:
  `data.rs` holding only `pub(super) const TABLE: &[&str]` under `use super::*;` → `warning:
  unused import: super::*`. So `mfb_body_declared_errors/exception_tables.rs` carries no `use`
  line.
- **`#[path]` on a module declared in a non-`mod.rs` file resolves relative to that file's
  directory.** Same probe: `#[path = "host/gen_x/tests/mod.rs"] mod gen_x_tests;` in
  `src/host.rs` loaded `src/host/gen_x/tests/mod.rs`. The existing precedent is
  `src/codegen/builtins/tls/gen_schannel.rs` `#[path = "gen_schannel_tests.rs"] mod
  schannel_tests;`, which loads `tls/gen_schannel_tests.rs`.
- **Glob-cycle imports (`use part::*;` in `mod.rs`, `use super::*;` in parts) compile without
  warnings.** `/tmp/loc/probe2` printed `6` with no warning.
- **Every `.rs` file under a `src/**/tests/` directory is test-only.** `find src -type d -name
  tests | wc -l` → 17 at `ce7a995ca`. For each, the parent declares `mod tests;` directly
  under `#[cfg(test)]`. The one exception is `src/codegen/engine/mod.rs:13` `pub(crate) mod
  tests;`, whose `engine/tests/mod.rs` gates every item `#[cfg(test)]` (read: `test_support`,
  its re-export, `tests`, `builtins_codegen`). Every directory H creates is declared the same
  way. This is the proof that the §4.1 guard change exempts no production file.
- **The architecture guards exempt test code by FILE NAME.** In `tests/guards/architecture_guards.rs`,
  `is_physical_scan_exempt` matches `"tests.rs" | "test_support.rs" | "abi.rs"` (plus regalloc
  `analysis.rs`). `no_hand_picked_vreg` and `emission_lines` skip `name.ends_with("tests.rs")`.
  Everything else is scanned through `code_above_tests`, the text before the first
  `#[cfg(test)]` or `mod tests`. A file such as `mir/mir/tests/mod.rs` matches neither rule,
  so it would be scanned whole. Replaying the four guards' needles over every region H moves
  (H map) finds offenders only in the inline tests of `src/codegen/engine/mir/mir.rs`: 102
  `"%vN"`/`"%fN"` literals and 138 `Reg::physical(`/`.physical_field(`/`Operand::phys(`
  lines. Every other H region has 0.
- **`no_type_strings.rs` / `no_operator_strings.rs` already exempt `/tests/` paths.**
  `is_test_file` in both: `rel.contains("/tests/") || name == "tests.rs" || …`.
- **Nothing in H is cfg-gated to a non-macOS host.** `grep -n "cfg(target_os\|cfg(unix\|cfg(windows"`
  over the 15 files → no hit. The macOS gate compiles every moved line.
- **Moving an inline test module out of a code file removes covered lines from that file's
  coverage denominator.** `.ai/testing-gates.md` says: "A test module counts in its own file's
  denominator unless it lives under a `tests/` directory". The file's percentage after the
  move is its code-only percentage, which can be lower. Per-file coverage is not a plan-176
  gate (§Validation Plan "Coverage check"), so H runs no coverage measurement.

## 3. Design Overview

The convention depends on whether the file is a whole test-only file or an inline block.

- **Inline block.** An inline `#[cfg(test)] mod tests { … }` becomes `#[cfg(test)] mod tests;`
  plus ONE test-only file `<host>/tests/mod.rs`, which holds the block's body verbatim and
  dedented. The body already starts with `use super::*;`, so `super` still means the host and
  every test path is unchanged. An inline block over 1000 is split into `tests/mod.rs` plus
  topic files of ≤ 900. Only `registry/mod.rs`'s `mod tests` (2160) is that large.
- **Whole test-only file over 1000.** `x.rs` becomes `x/mod.rs` plus parts of ≤ 900, grouped
  by responsibility.
- **Parts.** `mod.rs` keeps the `use` block, the shared helpers and one `mod <part>;
  use <part>::*;` pair per part. Each part starts with `use super::*;`, except a data-only
  part, which has no `use` line. Items used across files become `pub(super)`; each file lists
  them. `impl` blocks travel with their type.
- **Inline module not named `tests`.** It keeps its name and gets
  `#[cfg(test)] #[path = "tests/<name>.rs"] mod <name>;`, so its file lands in the host's
  `tests/` directory and its `super` is unchanged. `tests/mod.rs` does not declare it.

**Wiring of `builtins/tests/` is preserved.** `src/codegen/engine/tests/mod.rs` keeps
`#[cfg(test)] #[path = "../../builtins/tests/mod.rs"] mod builtins_codegen;`. Each
`builtins/tests/<x>.rs` H splits becomes `builtins/tests/<x>/mod.rs`. The `mod <x>;` line in
`builtins/tests/mod.rs` does not change, and `<x>/<part>.rs` resolves with no `#[path]`,
because the `#[path]` target is named `mod.rs` (verified above).

Gate class: test letter. Each phase must meet five checks:
- the whole-binary test count is the same before and after;
- the moved modules' tests pass;
- `cargo check --all-targets` adds no warning;
- the guards pass;
- the file-size guard shows only TIGHTEN/STALE rows for the phase's files, before the tighten.

Risk concentrates in three places:
- **The architecture-guard change** (§4.1) must exempt exactly the moved test code. It lands
  in Phase 2 with the MIR move that needs it. Its evidence is the replay and the 17-directory
  proof above.
- **`src/codegen/builtins/mod.rs`'s inline tests.** A plain `mod tests;` there would load
  `builtins/tests/mod.rs`, which is the `builtins_codegen` suite root, a second time. The
  `#[path = "tests/unit.rs"]` declaration (§4.10) avoids that.
- **`src/codegen/registry/mod.rs`** is the largest move (2160 + 339 LOC into 4 files) and the
  only inline block split into topics. It goes last.

Rejected alternatives:
- *Plain `mod unit_tests;` → `builtins/unit_tests.rs`* (and the same for
  `error_path_release_tests`, `codepage_tables`, `raw_result_block_ownership`). This puts test
  files outside a `tests/` directory, so coverage counts them and the guards scan them.
- *Splitting each moved inline block into topic files.* Every block except the registry's is
  ≤ 1000 (largest: `mir.rs`, 737 after the move). One file keeps every test path unchanged and
  needs no visibility edit.
- *Naming the MIR file `mir/mir/tests.rs` so the existing file-name skip covers it with no
  guard change.* This breaks the `foo/tests/mod.rs` convention, and plan-176-J and K need the
  `/tests/` rule for their own moves anyway.
- *Splitting `impl CodegenPlatform for FailAt` across files* is impossible: a trait impl cannot
  span files. It is also unnecessary, because the impl alone is 867 LOC, under the 900 part
  target.
- *Widening the architecture guards' file-name list (`*_basics.rs`, …).* Names drift. The
  `/tests/` component is what `no_type_strings.rs` already uses.

## 4. Detailed Design

The hazard commands below run once per file:
- `grep -rnoE "\[\[<path>:[^]]+\]\]" src/docs/spec`
- `grep -rn "<path>" tests/guards scripts .ai AGENTS.md src/docs .github Cargo.toml src`
- `grep -n "<path>" scripts/coverage-exceptions.txt`
- a scan of the moved lines for `include_str!|include_bytes!|#[path|module_path!|file!()|--exact|current_exe|env!(`

"Hazards: none" means none of these returned anything relevant. `spec_citations_resolve`
checks only that a cited FILE exists. So a cite into a test file that becomes a directory FAILS
it, while a cite into a code file that keeps existing goes silently stale. Both kinds are listed
and repointed. "Moved file LOC" = the block's LOC − 3 (the `#[cfg(test)]`, `mod x {` and `}`
lines). "Left inline" = the host's inline LOC − the block + 2 (the new `#[cfg(test)]` and
`mod x;` lines).

### 4.0 Exemptions

H adds no `exempt` row and no census `EXEMPT` entry. The three single-unit files of the
earlier design have these dispositions:

| Earlier exemption | Disposition |
|---|---|
| `src/codegen/builtins/tests/corpus/fixture_list.rs` (`CORPUS`, 644) | Gone. `src/codegen/builtins/tests/corpus.rs` is 766 ≤ 1000, so it is not split. |
| `src/codegen/builtins/tests/diagnostics/fixture_list.rs` (`DIAGNOSTIC_CORPUS`, 419) | Gone. `src/codegen/builtins/tests/diagnostics.rs` is 569 ≤ 1000, so it is not split. |
| `src/codegen/builtins/tests/platform_failure/forwarding.rs` (`impl CodegenPlatform for FailAt`, 867) | Kept as an ordinary part. `platform_failure.rs` (1088) still has to split, and the impl alone is 867 ≤ 900 (§4.3). |

### 4.1 Guard change: `tests/guards/architecture_guards.rs`

- `is_physical_scan_exempt(path)` also returns true when the path relative to
  `CARGO_MANIFEST_DIR` contains `/tests/`. Compute `rel` from the manifest, as
  `shared_lowering_names_no_physical_register` already does for its messages, and pass it in.
- `no_hand_picked_vreg` skips a file when its manifest-relative path contains `/tests/`, beside
  the existing `name.ends_with("tests.rs")` skip.
- `emission_lines` gets the same skip. It feeds `physical_registers_stay_in_the_backends` and
  `no_constructed_vreg_name`.

Justification (AGENTS.md, guard edit):
1. When/why written: the file-name skips came with the arch lint's extraction
   (`git log -S'"tests.rs" | "test_support.rs"' --format='%h %s'` → `9547ef9cf bug-334 D2:
   extract the plan-34-D arch lint to architecture_guards.rs`) and the register guards
   (`git log -S'name.ends_with("tests.rs")'` → `8818c963e bug-721: root cause, register
   guards, …`).
2. Behavior protected: emission code never names a physical register or hand-numbers a vreg;
   test fixtures that spell registers are exempt.
3. Dependents: the four guard tests above. plan-176-J and K list this change as a
   prerequisite (both grep for `contains("/tests/")`).
4. Proof that the narrower rule is wrong for the new layout:
   - the guard's own doc says test-fixture files are exempt ("the pure test-fixture files");
   - the moved MIR region is the same test code at a new path;
   - every `.rs` under a `src/**/tests/` directory is test-only (§2, 17 directories).

   No production file becomes exempt.

### 4.2 `src/codegen/builtins/tests/mfb_body_declared_errors.rs` → `mfb_body_declared_errors/` (test 1856, 2 tests)

The file is a scanner/inference library plus two sweeps. It splits into three files by
responsibility. Both tests stay in `mod.rs`, so their paths are unchanged. Line ranges are at
`ce7a995ca`.
- `mod.rs` (597): the `//!` header; the existing `use` block [lines 83-88], including
  `use super::declared_errors_sweep::codes_by_value;` and
  `use super::general_declared_errors::OVERLOADS;`; and `mod source_scan; use source_scan::*;`
  plus `mod exception_tables; use exception_tables::*;`. It also holds the sweeps:
  - FUNC-block extraction [1080-1345]: `extract_func_blocks`, `strip_comment_lines`,
    `strip_func_header_and_footer`, `contains_keyword`, `find_function_level_trap`,
    `find_inline_trap_regions`, `scan_calls_and_fails`;
  - trap resolution [1346-1530]: `resolve_inline_traps`, `resolve`, `MfbSweepResult`;
  - the body sweep [2285-2465]: `SweepTables`, `sweep_tables`, `sweep`,
    `every_body_mfb_member_declares_exactly_the_errors_its_source_can_raise`;
  - the rewrite sweep [2466-2585]: `RewriteSweepResult`, `REWRITE_UNREACHABLE`,
    `REWRITE_DECLARED_EXTRA`, `rewrite_sweep`,
    `every_body_rewrite_implementation_declares_exactly_the_errors_its_target_can_raise`.
- `source_scan.rs` (845) [89-1079]: what a call in MFBASIC source can raise.
  - call scanning: `general_names`, `is_ident_char`, `fail_codes`, `preceded_by_case`,
    `qualified_calls_with_arguments`, `internal_calls`, `general_calls`, `call_arguments`;
  - scope and source spelling: `Scope` + `impl Scope`, `package_globals`, `EACH_PREFIX`,
    `source_spelling`, `package_of_symbol`, `strip_outer_parens`, `has_top_level_operator`,
    `is_arithmetic`;
  - type inference: `infer_type`, `infer_type_inner`, `record_field_type`,
    `single_return_type`, `overloaded_return_types`, `general_override_helper`;
  - error-code derivation: `is_arithmetic_expression`, `qualified_call_codes`,
    `general_call_codes`.
- `exception_tables.rs` (419, data-only, no `use` line) [1531-2284]: the measured tables
  `UNREACHABLE_VIA_CALLEE`, `UNREACHABLE_GENERAL_CALL`, `UNREACHABLE_QUALIFIED_CALL`,
  `DECLARED_EXTRA`.

Visibility edits (16): each item below is used from `mod.rs`, a file other than its own, and
becomes `pub(super)`.
- in `source_scan.rs` (12): `general_names`, `is_ident_char`, `fail_codes`,
  `qualified_calls_with_arguments`, `internal_calls`, `general_calls`, `general_override_helper`,
  `qualified_call_codes`, `general_call_codes`, `Scope`, `Scope::of`, and the field
  `Scope::func`;
- in `exception_tables.rs` (4): all four tables.

Evidence: `/tmp/loc/rw-H/xref.py` (word references of each top-level item, comments stripped,
by file group) and `awk 'NR>=1080&&NR<1531||NR>=2285' … | grep -nE 'scope\.(package|func|names)|Scope::'`
→ only `scope.func` (2) and `Scope::of` (1). `MfbSweepResult`, `sweep`, `RewriteSweepResult` and
`rewrite_sweep` are already `pub(crate)` and stay in `mod.rs`. The compiler names any field
missed.

Hazards (all fixed in the same commit):
- `src/docs/spec/tooling/04_audit-format.md:311`
  `[[src/codegen/builtins/tests/mfb_body_declared_errors.rs:rewrite_sweep]]` →
  `[[src/codegen/builtins/tests/mfb_body_declared_errors/mod.rs:rewrite_sweep]]`. This is
  required: `spec_citations_resolve` fails on the vanished file.
- Prose naming the vanished file → `mfb_body_declared_errors/`:
  `src/codegen/builtins/tests/general_declared_errors.rs:8` and `:39` (`//!`/`///`),
  `src/codegen/builtins/canvas/func_poll_mouse.rs:413` (comment
  `tests/mfb_body_declared_errors.rs`), and `src/audit/collect/source.rs:1006` (comment in the
  code region, above its first `#[cfg(test)]` at :1022). Found by
  `grep -rn "mfb_body_declared_errors\.rs" src .ai tests scripts` → 5 hits before, 0 after.
- The untracked `bugs/bug-1092-…md` is not H's file and is not edited.

### 4.3 `src/codegen/builtins/tests/platform_failure.rs` → `platform_failure/` (test 1088, 1 test)

- `mod.rs` (223) [lines 1-110, 977-1224 at `ce7a995ca`] holds:
  - the `use` block;
  - `INJECTED`, `FailAt` + `impl FailAt`, `os_seam_platforms`, `Needs`,
    `what_a_clean_lowering_needs`, `between`, `lower_body`;
  - the test `an_emit_failure_past_the_first_one_still_propagates`;
  - `mod forwarding;`.
- `forwarding.rs` (867) [111-976] holds `use super::*;` and `impl CodegenPlatform for
  FailAt<'_>` only. This impl forwards every trait method, so a new trait method is a compile
  error here. As a child of the defining module, it reaches `FailAt`'s private fields and the
  private `tick`/`granting`, so no visibility edits are needed. `mod forwarding;` needs no glob,
  because an impl has no name.

The test path is unchanged. Hazards: none. `grep -rn "tests/platform_failure\.rs" src .ai
tests scripts` → only the file itself.

### 4.4 `src/codegen/builtins/audio/mod.rs` inline `mod tests` (296, 11 tests) → `audio/tests/mod.rs`

`mod tests { … }` [526-873] → `#[cfg(test)] mod tests;`. `audio/tests/` does not exist
today; `status_tests.rs` is a different module and stays. Moved file LOC: 291. Bodies that spell
`super::runtime_overload_name`, `super::AUDIO_INPUT_TYPE_ID`, `super::gen_shared::…` and similar
still resolve, because `super` is still `audio`. Tests: `audio_registered_on_the_clean_room_registry`,
`direction_is_part_of_the_type`, `dual_direction_members_accept_either_handle`,
`open_and_timed_overloads_resolve`, `source_members_rewrite_to_their_companion_bodies`,
`named_device_open_rewrites_under_either_device_spelling`, `close_ops_are_the_per_direction_bodies`,
`pcm_accumulators_never_cross_a_call_boundary`, `reassembled_source_parses`,
`core_audio_device_open_reads_the_inlined_id`, `alsa_device_open_reads_the_inlined_id`.
Hazards: 1 citation, to code (`AUDIO_INPUT_TYPE`, outside the moved range). Left inline: 4
(the existing `#[cfg(test)] mod status_tests;` plus the new declaration).

### 4.5 `src/codegen/builtins/canvas/mod.rs` inline (421, 14 tests) → `canvas/tests/mod.rs`

`mod tests` [2618-3222] → `#[cfg(test)] mod tests;`. There is no collision:
`helper_render/tests/` is another module. Moved file LOC: 418. Tests:
- `draw_item_variant_set_is_frozen`, `the_particle_template_is_a_shape`,
  `no_canvas_type_reaches_itself`, `shape_variant_set_is_frozen`,
  `auto_shape_variant_set_is_frozen`, `every_draw_item_variant_has_a_record`,
  `every_draw_item_variant_carries_a_paint`;
- `no_record_shares_a_name_with_a_resource`,
  `the_resource_naming_variants_hold_the_resource_itself`, `reassembled_source_parses`,
  `the_normal_source_reads_only_the_renderer_headless_and_sync_variables`,
  `the_normal_source_has_no_reporting_hook`, `canvas_text_has_one_glyph_walk`,
  `canvas_types_are_builtin_types`.

Hazards: these are silent, because `canvas/mod.rs` still exists, so repoint them by hand. In
`src/docs/spec/app/06_canvas.md`, `:96` `…canvas/mod.rs:draw_item_variant_set_is_frozen]]`,
`:1674` `…:no_canvas_type_reaches_itself]]` and `:2769` `…:canvas_text_has_one_glyph_walk]]`
→ `[[src/codegen/builtins/canvas/tests/mod.rs:…]]`. `[[…:JoinStyle]]` / `[[…:CANVAS]]` name
code and are unchanged. Left inline: 2.

### 4.6 `src/codegen/builtins/crypto/mod.rs` inline (574, 15 tests) → `crypto/tests/mod.rs`

`mod tests` [987-1664] → `#[cfg(test)] mod tests;`. Moved file LOC: 571. Tests:
`crypto_registered_on_the_clean_room_registry`, `membership_via_generic_registry`,
`native_and_internal_flags`, `builtin_types_recognized`,
`bytes_text_overloads_select_distinct_rewrite_targets`, `argument_typed_return_resolution`,
`random_int_big_overload_registry_facts`, `clock_derived_identifiers_declare_the_clock_overflow`,
`aead_aad_default_padding`, `open_pads_the_overload_its_arguments_select`,
`keccak_core_is_branch_free`, `curve448_secret_paths_are_branch_free`,
`curve25519_secret_paths_are_branch_free`, `gf448_mul_accumulators_fit_i63`,
`reassembled_source_parses`. Hazards: 1 citation, to code. Left inline: 2.

### 4.7 `src/codegen/builtins/datetime/mod.rs` inline (406, 11 tests) → `datetime/tests/mod.rs`

`mod tests` [649-1092] → `#[cfg(test)] mod tests;`. Moved file LOC: 403. Tests:
`datetime_registered_on_the_clean_room_registry`, `generic_dispatch_reaches_datetime`,
`argument_typed_return_resolution`, `overload_selection_by_arity_yields_the_right_rewrite_target`,
`default_padding_time_only`, `builtin_types_recognized`, `membership_via_generic_registry`,
`expected_arguments_bespoke_phrasings`, `param_name_tables`, `reassembled_source_parses`,
`members_declare_the_errors_they_raise`.

Hazards: `src/docs/spec/stdlib/02_datetime.md:197` cites
`[[src/codegen/builtins/datetime/mod.rs:__datetime_fixedOffset1]]`. That symbol's only
occurrence in the file is a test string inside the moved block. It is defined in
`src/codegen/builtins/datetime/func_fixed_offset.rs:8` (`FUNC __datetime_fixedOffset1`;
`grep -n __datetime_fixedOffset1 src/codegen/builtins/datetime/func_fixed_offset.rs` → :8,
:31). Repoint the cite to
`[[src/codegen/builtins/datetime/func_fixed_offset.rs:__datetime_fixedOffset1]]`, the
definition, which is what the surrounding prose (fixedOffset semantics) describes. Left
inline: 2.

### 4.8 `src/codegen/builtins/encoding/mod.rs` inline (488, 16 tests) → `encoding/tests/`

There are two modules (484 LOC together). Moving only `mod tests` would leave 243 inline,
seven under the limit, and moving only `codepage_tables` would leave 253, which is over it.
Both move, leaving 8:
- `mod tests` [274-558, 10 tests] → `#[cfg(test)] mod tests;` → `encoding/tests/mod.rs`
  (244). Tests: `encoding_registered_on_the_clean_room_registry`,
  `generic_dispatch_reaches_encoding`, `non_overloaded_members_rewrite_to_their_internal_symbol`,
  `resolve_call_types_the_members`, `arity_is_unary_across_members`,
  `diagnostic_metadata_via_registry`, `reassembled_source_parses`,
  `augmented_project_injects_when_imported`, `augmented_project_noop_without_import`,
  `members_declare_the_errors_they_raise`.
- `mod codepage_tables` [569-875, 6 tests] →
  `#[cfg(test)] #[path = "tests/codepage_tables.rs"] mod codepage_tables;` →
  `encoding/tests/codepage_tables.rs` (234). The module keeps its name and its
  `use super::helper_codepage_table::{BODY, VARIANTS};` (`super` = encoding).
  `tests/mod.rs` does NOT declare it. Tests: `codepage_tables_match_the_vendored_index_files`,
  `every_vendored_index_file_has_a_codepage_variant`,
  `codepage_enum_is_registered_in_generator_order`, `iso_8859_8_i_shares_the_iso_8859_8_table`,
  `codepage_digests_match_the_vendored_index_files`,
  `codepage_roundtrip_counts_match_the_vendored_index_files`.
- `#[cfg(test)] pub(crate) fn augmented_hir_project` [267-273] stays in `encoding/mod.rs`.

Hazards: no citation names the moved ranges. The three `env!("CARGO_MANIFEST_DIR")` reads of
`tools/codepage-index/` are manifest-relative. Left inline: 8.

### 4.9 `src/codegen/builtins/general/mod.rs` inline (572, 29 tests) → `general/tests/mod.rs`

`mod tests` [696-1358] → `#[cfg(test)] mod tests;`. Moved file LOC: 569. The helpers `types`,
`ft`, `ALL_GENERAL`, `rt`, `AllEnums`, `ColorIsAnEnum` and `rt_kinds` move with it, along with
the `impl TypeKinds` blocks. Tests:
- `is_general_call_covers_all`, `every_builtin_that_can_be_a_callback_returns_boolean`,
  `overridable_and_reserved`, `override_result_type_all_arms`, `call_param_names_all_arms`,
  `builtin_function_id_arms`, `builtin_function_id_for_type_specialized`,
  `builtin_function_id_for_type_non_predicate_shape`;
- `filter_predicate_type_cases`, `resolve_error`, `resolve_len`, `resolve_type_name`,
  `resolve_to_string`, `resolve_to_int`, `resolve_to_int_enum`, `resolve_to_string_enum`,
  `only_to_int_and_to_string_consult_the_kind_oracle`, `resolve_to_float_fixed_byte`,
  `resolve_to_scalar_and_money`, `resolve_predicates`, `resolve_call_unknown`,
  `resolve_return_type_wrapper_delegates`, `nominal_return_type_matches_fast_oracle`;
- `expected_arguments_all_arms`, `helpers_exact_and_one_of`,
  `nested_function_parameters_split_at_the_top_level_arrow`,
  `registers_the_eighteen_globals_with_legacy_arities`, `conversions_declare_their_runtime_errors`,
  `error_arity_is_none_others_delegate`.

Hazards: 6 citations, all to code. Left inline: 2.

### 4.10 `src/codegen/builtins/mod.rs` inline (1104, 32 tests) → `builtins/tests/unit.rs` and `builtins/tests/tcp_tls_error_declaration.rs`

The host has three modules:
- `mod tests` [1045-1979, 28 tests, 683];
- `mod plan111f_probe` [1981-2002, 1 test, 17];
- `mod tcp_tls_error_declaration_tests` [2043-2507, 3 tests, 404].

Name collision: `builtins/tests/` is the `builtins_codegen` suite root, so a plain
`mod tests;` here would load `builtins/tests/mod.rs` a second time. The declarations are:
- `#[cfg(test)] #[path = "tests/unit.rs"] mod tests;` → `builtins/tests/unit.rs` (680).
- `#[cfg(test)] #[path = "tests/tcp_tls_error_declaration.rs"] mod tcp_tls_error_declaration_tests;`
  → `builtins/tests/tcp_tls_error_declaration.rs` (401).

The module paths `codegen::builtins::tests` and `…::tcp_tls_error_declaration_tests` are
unchanged. `builtins/tests/mod.rs` does NOT declare `unit` or `tcp_tls_error_declaration`.
`ls src/codegen/builtins/tests/` at `ce7a995ca` has neither name, so nothing collides.

`unit.rs` holds 28 tests:
- `qualified_builtin_type_requires_matching_package`,
  `no_named_argument_alias_repeats_across_positions`,
  `overloaded_param_name_tables_are_well_formed`,
  `a_named_call_may_omit_an_overloaded_builtins_trailing_optionals`,
  `the_fallback_never_leaves_out_a_required_parameter`,
  `a_named_call_never_selects_an_overload_with_a_gap_before_a_supplied_name`,
  `no_member_declaring_an_error_is_infallible`;
- `inline_builtin_fallibility_census`, `tostring_is_fallible_on_every_overload`,
  `the_raising_strings_members_are_not_infallible`, `replace_is_fallible_only_on_a_string`;
- `toint_is_infallible_only_on_an_enum`, `inline_builtin_raw_supported_set`,
  `every_package_is_a_builtin_import`, `spec_section_18_package_list_matches_is_builtin_import`,
  `is_builtin_import_cases`, `is_builtin_type_aggregates`, `general_override_target_cases`;
- `qualified_builtin_type_cases`, `resource_helpers`, `native_builtin_target_cases`,
  `inline_trap_unsupported_cases`, `call_return_type_name_aggregates`,
  `is_nonescaping_callback_arg_cases`, `is_builtin_call_aggregates`,
  `is_builtin_member_and_package_constant`, `package_constant_type_and_value`,
  `call_param_names_aggregates`.

`ALL_BUILTIN_PACKAGES = super::BUILTIN_IMPORTS` keeps resolving, because `super` is still
`builtins`.

`tcp_tls_error_declaration.rs` holds 3 tests: `every_tcp_member_declares_the_errors_its_lowering_raises`,
`every_tls_member_declares_the_errors_its_lowering_raises`,
`no_tcp_or_tls_error_list_reaches_the_infallibility_verdict`.

`mod plan111f_probe` stays inline. Left inline: 21.

Hazards:
- 20 citations, all to code symbols outside the moved ranges.
- The test that reads `CARGO_MANIFEST_DIR` is manifest-relative.
- `scripts/coverage-exceptions.txt:36` mentions `builtins/mod.rs` only in a comment (no entry).
- `.ai/resources-packages.md:376` names `spec_section_18_package_list_matches_is_builtin_import`
  with no file path, so it is unchanged.

### 4.11 `src/codegen/builtins/tls/gen_openssl.rs` inline (301, 9 tests) → `tls/gen_openssl/tests/mod.rs`

`mod error_path_release_tests { … }` [3460-3837] →
`#[cfg(test)] #[path = "gen_openssl/tests/mod.rs"] mod error_path_release_tests;`. The path is
relative to `tls/`, and `use super::*;` keeps meaning `gen_openssl`. Moved file LOC: 298. When
plan-176-T turns `gen_openssl.rs` into `gen_openssl/mod.rs`, it writes
`#[path = "tests/mod.rs"]`; the file is the same either way. Tests:
`connect_alloc_fail_frees_ssl_ctx_and_fd`, `listen_bind_all_passes_a_non_null_service`,
`connect_tls_fail_frees_ssl_and_ctx`, `accept_alloc_fail_frees_ssl_and_fd`,
`byte_read_fill_loop_writes_no_entry_descriptor`, `listen_lowers_with_min_proto_check`,
`a_read_that_reports_the_close_makes_every_later_write_raise`,
`read_names_a_departed_peer_connection_closed`,
`accept_bounds_the_whole_handshake_with_one_deadline`. Hazards: 4 citations, all to code.
Left inline: 2.

### 4.12 `src/codegen/engine/analysis/borrow_get.rs` inline (522, 27 tests) → `analysis/borrow_get/tests/mod.rs`

`mod tests` [1704-2298] → `#[cfg(test)] mod tests;`. Moved file LOC: 519. Tests:
- `a_directly_matched_binding_after_place_forwarding_is_pinned`,
  `a_field_read_binding_over_an_immutable_list_is_pinned`,
  `a_field_read_binding_over_a_mut_list_is_windowed`,
  `a_write_to_the_container_inside_the_window_refuses_the_borrow`,
  `a_non_lookup_read_of_the_container_inside_the_window_refuses_the_borrow`,
  `a_get_or_with_a_built_default_is_no_borrow`, `get_with_set_on_the_same_slot_is_element_bound`,
  `a_with_over_the_binding_as_the_set_item_is_element_bound`,
  `a_let_bound_with_before_its_set_is_element_bound`;
- `write_backs_in_both_if_arms_are_element_bound`, `a_write_back_in_one_if_arm_only_is_not_bound`,
  `a_read_of_the_binding_after_a_with_item_set_is_not_bound`,
  `two_elements_live_across_a_write_are_scratch_copies`,
  `an_escaping_or_get_or_binding_is_no_scratch_copy`,
  `a_write_to_the_container_between_the_bind_and_the_set_is_not_bound`,
  `an_element_update_that_is_not_adjacent_to_its_write_back_is_not_bound`,
  `an_element_read_after_its_write_back_is_not_bound`, `a_write_back_to_another_slot_is_not_bound`,
  `an_element_passed_whole_before_its_update_is_not_bound`,
  `a_global_container_needs_no_reachable_store`;
- `the_single_expression_update_names_its_container_and_index`,
  `a_name_bound_in_two_sequential_loops_borrows_in_each`,
  `a_name_bound_in_sibling_arms_borrows_in_each`,
  `a_name_rebound_inside_the_same_loop_borrows_in_each`,
  `an_element_update_survives_a_later_read_only_rebind`,
  `an_unsafe_rebinding_stays_a_copy_beside_a_borrowing_one`,
  `a_function_without_rebinds_has_no_per_binding_roles`.

Hazards: 4 citations, to code. `.ai/collections.md` names `collect_borrow_gets` (code). Left
inline: 2. When plan-176-U turns `borrow_get.rs` into `borrow_get/mod.rs`, `mod tests;`
resolves to the same file.

### 4.13 `src/codegen/engine/analysis/handover.rs` inline (560, 10 tests) → `analysis/handover/tests/mod.rs`

`mod tests` [791-1428] → `#[cfg(test)] mod tests;`. Moved file LOC: 550. Tests:
`collect_handover_args_follows_the_hand_derived_table`, `a_global_argument_is_never_handed_over`,
`a_resource_bearing_argument_is_never_handed_over`, `a_fresh_temporary_argument_is_handed_over`,
`an_owned_parameter_is_handed_on_only_in_a_variant`, `a_bare_string_parameter_is_never_handed_over`,
`a_record_is_handed_over_only_when_every_field_is`, `consumable_params_follows_the_hand_derived_table`,
`a_returned_with_over_scalar_fields_consumes_its_owner`,
`a_parameter_returned_as_a_borrow_is_never_handed_over`. Hazards: 2 citations, to code. The
`#[cfg(test)] fn len` accessor stays in `handover.rs`. Left inline: 9.

### 4.14 `src/codegen/engine/analysis/last_use.rs` inline (685, 6 tests) → `analysis/last_use/tests/mod.rs`

`mod tests` [1064-1828] → `#[cfg(test)] mod tests;`. Moved file LOC: 682. The `SHAPES`,
`DECODERS` and `CHAIN_BACKTRACKER` tables move with the tests in one file, so no visibility
edit is needed. Tests: `collect_last_use_moves_follows_the_hand_derived_table`,
`collect_last_use_moves_views_of_a_match_scrutinee`,
`collect_last_use_moves_never_moves_a_by_ref_capture`,
`collect_last_use_moves_never_moves_a_resource_bearing_value`,
`collect_last_use_moves_finds_the_five_decoder_append_sites`,
`collect_last_use_moves_covers_the_chain_backtracker_views`. Hazards: no citation;
`.ai/codegen-invariants.md` names the analysis (code). Left inline: 2.

### 4.15 `src/codegen/engine/mir/mir.rs` inline (744, 16 tests) → `mir/mir/tests/mod.rs`

The module is `mir::mir` (file `mir/mir.rs`), so its child lives in `mir/mir/`. `mod tests`
[957-1848] → `#[cfg(test)] mod tests;`. Moved file LOC: 737. Tests:
`code_op_round_trips_through_mir`, `renamed_ops_are_neutral_but_select_back_identically`,
`v128_ops_are_neutral_but_select_back_identically`, `v128_lane_semantics_contract`,
`lower_then_select_is_identity`, `flag_pairs_fuse_and_expand_identically`,
`fused_op_carries_operands_and_condition`, `syscall_carry_check_fuses_and_expands_identically`,
`explicit_carry_chain_round_trips`, `multi_branch_compare_fuses_with_share`,
`addr_of_fuses_and_expands_identically`, `unpaired_or_mismatched_adrp_is_not_fused`,
`round_trip_sweep_over_every_op_family`, `arena_base_renames_and_realizes_identically`,
`invariant_registers_are_neutral_tokens`, `reloc_intents_are_neutral_and_realize_to_aarch64_kinds`.

Hazards: 23 citations, all to code. The move requires §4.1 in the same commit, because of the
102 vreg literals and 138 physical-register lines. Today they are skipped only because they
sit below `#[cfg(test)]` in `mir.rs` (`code_above_tests`), and `src/codegen/engine/mir/` is
not in `physical_registers_stay_in_the_backends`' allow-list. The `#[cfg(test)] fn is_v128`
arm inside `mir_ops!` stays. Left inline: 6.

### 4.16 `src/codegen/registry/mod.rs` inline (2695, 62 tests) → `registry/tests/`

The host has these test modules:
- `mod qualification_tests` [5226-5338, 8 tests, 79 LOC];
- `mod tests` [5340-8052, 50 tests, 2160];
- `mod raw_result_block_ownership` [8054-8494, 4 tests, 339];
- `#[cfg(test)]` fns between the registry code's `self_update_shaped` and `resolve_call`:
  `self_update_shaped`, `is_string_self_update_type`, `strip_res`, `resolve_bound`, `occurs`,
  the two-sided unifier, `resolve_call`.

`mod tests` → `#[cfg(test)] mod tests;`. `registry/` has no `tests` dir at `ce7a995ca`.
`mod tests` is 2160, over 1000, so it splits into three files by responsibility. LOC are
measured on the ranges named, at `ce7a995ca`.
- `tests/mod.rs` (650) holds:
  - the module's `use super::*;` and 2 `mod` lines;
  - the shared builders `func`, `prop`, `rec`, `variant`, `uni`, `res`, `mfb_impl`, `param`
    [6087-6205], `call`, `list_of`, `map_of`, `generic_impl` [6326-6351] and `th` [6409-6418];
  - the registry-row and descriptor-shape tests [5361-6253], 12 tests, whose paths are
    unchanged:
    - `no_member_defines_more_source_forms_than_its_descriptor_declares`,
      `range_and_index_parameters_use_the_documented_vocabulary`,
      `the_consuming_parameters_name_real_members`, `the_placing_operands_name_real_members`;
    - `every_registry_row_has_the_shape_its_builder_requires`,
      `the_consuming_parameter_index_agrees_across_overloads`,
      `set_group_items_is_the_consuming_parameter`,
      `a_builtin_record_property_may_carry_a_res_type`,
      `strict_matching_separates_resource_params_from_value_union_params`;
    - `strict_matching_holds_builtin_value_types_to_their_identity`,
      `descriptor_named_types_are_bare_nominals`,
      `package_constants_and_overrides_round_trip_through_the_builders`.
- `tests/selection_and_thread_probes.rs` (694) [6254-6325, 6352-6408, 6419-7114], 14 tests.
  Helpers: `UnionUOverV` + `impl TypeKinds for UnionUOverV`. Tests:
  - `constant_and_override_boundary_fns_fall_through_for_unowned_names`,
    `frozen_registry_exposes_the_csv_package`,
    `an_overload_is_two_implementations_differing_by_parameter_type`,
    `select_binds_type_variables_and_substitutes_the_return`;
  - `thread_probe_waitfor_receive_send_poll`, `thread_probe_transfer_accept_resource_plane`,
    `thread_probe_start_extraction_and_resource_echo`,
    `bug_824_plain_callback_param_accepts_isolated_concrete`,
    `thread_probe_parse_isolated_func_arg`,
    `resolved_parameter_type_infers_a_generic_position_from_the_call`;
  - `a_top_level_variable_bound_to_a_union_accepts_its_variant`,
    `reduce_widens_a_variant_initial_value_and_never_narrows_a_union`,
    `select_rejects_inconsistent_variable_binding`, `select_unknown_argument_is_a_wildcard`.
- `tests/rendering_and_gates.rs` (817) [7115-8051, plus the helpers `enum_variant`, `enm`
  (6131-6147) and `intrinsic`, `rewrite_impl` (6175-6194), whose only users are here], 24
  tests. Helpers also include `sample_fast_path`. Tests:
  - `select_scalar_mismatch_and_wrong_arity_are_rejected`,
    `public_member_for_rewrite_names_the_member_a_user_calls`,
    `contains_var_detects_generic_types`,
    `unify_substitute_over_map_entry_and_result_variants`,
    `add_function_takes_a_function_value`, `resolve_func_finds_the_owning_package`;
  - `mfb_body_carries_a_rewrite_target_and_optional_fast_path`,
    `get_mfb_renders_helper_functions_before_member_bodies`,
    `a_debug_split_helper_renders_the_half_matching_the_build`,
    `get_mfb_is_empty_when_the_package_has_no_mfb_member`;
  - `add_record_renders_the_type_declaration`, `add_union_renders_the_union_declaration`,
    `add_enum_renders_the_enum_declaration`,
    `enum_variant_advisory_is_keyed_by_package_enum_and_member`,
    `enums_resolve_as_builtin_types`, `add_resource_records_name_export_and_close_function`,
    `get_mfb_places_enums_between_unions_and_helpers`,
    `get_mfb_orders_imports_records_unions_helpers_functions`;
  - `descriptor_round_trips_the_full_surface`, `is_imported_by_checks_the_program_imports`,
    `a_named_type_opens_its_gate_and_closes_the_exact_complement`,
    `a_constructed_record_opens_its_gate_and_closes_the_complement`,
    `typed_and_string_resolution_agree`, `agreed_argument_type_answers_where_overloads_agree`.

No visibility edits: the topics are children of `tests` and see its private builders.

`mod raw_result_block_ownership` →
`#[cfg(test)] #[path = "tests/raw_result_block_ownership.rs"] mod raw_result_block_ownership;`
→ `registry/tests/raw_result_block_ownership.rs` (336). The module keeps its name and its
`use super::runtime_specs;`, and `tests/mod.rs` does not declare it. Tests:
`no_companion_calls_a_source_generic_collections_member`,
`every_block_returning_runtime_helper_is_classified`,
`every_string_returning_runtime_helper_is_marked_fresh`,
`no_thread_family_result_is_ever_freed_by_the_calling_thread`.

`mod qualification_tests` and the `#[cfg(test)]` helper fns stay in `registry/mod.rs`. Left
inline: 200 (2695 − 2160 − 339 + 4 declaration lines).

Hazards:
- `src/docs/spec/language/18_builtin-functions.md:245`
  `[[src/codegen/registry/mod.rs:range_and_index_parameters_use_the_documented_vocabulary]]`
  → `[[src/codegen/registry/tests/mod.rs:range_and_index_parameters_use_the_documented_vocabulary]]`.
  This one is silent, because `registry/mod.rs` still exists.
- `.ai/man-content.md:117-118` names the same test "(`src/codegen/registry/mod.rs`)" →
  `src/codegen/registry/tests/mod.rs`.
- The other 32 citations name code. `no_type_strings.rs` names registry code, which is
  unchanged.
- The untracked `bugs/bug-1092-…md` cites `registry::tests::the_placing_operands_name_real_members`.
  That test stays in `tests/mod.rs`, so its path is unchanged.
- When plan-176-M splits the registry code, the three declarations move with whichever file
  keeps them; the paths stay `registry/tests/…`.

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

Every phase uses the same gate, run in this order. Time estimates are guesses, not
measurements.

- **G0 (before the phase's first edit):** run
  `cargo test -p mfb --bin mfb -- --list 2>/dev/null | grep -c ': test$' > /tmp/h_before.txt`
  and `cargo check --all-targets 2>&1 | grep -c '^warning' > /tmp/h_warn_before.txt` (est.
  5 min cold, 1 min warm).
- **G1:** run the same two commands after the phase's edits → the numbers equal the G0 files.
- **G2:** run the moved modules' tests (libtest filters after `--`, one per module prefix) →
  all pass, 0 failed.
- **G3:** `cargo test --test architecture_guards --test no_type_strings --test no_operator_strings`
  and `cargo test -p mfb --bin mfb spec_citations_resolve` → all pass (est. 3 min).
- **G4:** `cargo test --test file_size_budget 2>&1 | grep -E '^  (NEW|GREW)'` → no output, and
  every `TIGHTEN`/`STALE` row names a file of this phase. Then run
  `MFB_LOC_TIGHTEN=1 cargo test --test file_size_budget` and rerun →
  `grep -cE '^  (NEW|GREW|STALE|TIGHTEN)'` → `0`. Commit `tests/guards/file_size_ceilings.txt`
  with the phase. Its `ceiling inline`/`ceiling test` rows for the phase's files are gone;
  `ceiling code` rows are untouched (est. 2 min).
- **G5:** `cargo fmt --check` → no output (est. 20 s).

### Phase 1 — The two over-limit `builtins_codegen` suites

- [x] Split `src/codegen/builtins/tests/mfb_body_declared_errors.rs` →
      `mfb_body_declared_errors/{mod,source_scan,exception_tables}.rs` per §4.2 (16 visibility
      edits). Repoint `src/docs/spec/tooling/04_audit-format.md:311` to
      `mfb_body_declared_errors/mod.rs:rewrite_sweep`, and the four prose mentions to
      `mfb_body_declared_errors/`.
      Done: `git mv` to `…/mod.rs`, then `items.py extract` → `source_scan.rs` 26 items, `exception_tables.rs` 4 (no
      `use` line); the 16 `pub(super)` edits are exactly §4.2's (the compiler asked for each); motion → only those
      prefixes and the 4 `mod`/`use` lines differ. The spec cite and the 4 prose mentions are repointed
      (`general_declared_errors.rs` :8 :39, `canvas/func_poll_mouse.rs` :413, `audit/collect/source.rs` :1006).
- [x] Split `src/codegen/builtins/tests/platform_failure.rs` →
      `platform_failure/{mod,forwarding}.rs` per §4.3.
      Done: `forwarding.rs` holds the one impl; no visibility edit; motion → only `mod forwarding;` added.
- [x] G4: tighten the ceilings for the two files.
      Done: → two STALE rows (`mfb_body_declared_errors.rs` 1856, `platform_failure.rs` 1088); tighten drops exactly
      those; re-run → 0.

Acceptance: both suites run from their directory modules, their test paths and counts did not
move, and no part is over 900. Check:
- G1 → equal.
- G2 `cargo test -p mfb --bin mfb -- builtins_codegen::mfb_body_declared_errors:: builtins_codegen::platform_failure::`
  → 3 passed, 0 failed.
- `cargo test -p mfb --bin mfb -- --list 2>/dev/null | grep -cE 'builtins_codegen::(mfb_body_declared_errors|platform_failure)::'`
  → 2+1 = 3.
- `for f in src/codegen/builtins/tests/mfb_body_declared_errors/*.rs src/codegen/builtins/tests/platform_failure/*.rs; do python3 -I tools/loc-census/census.py --file $f; done`
  → second fields 597, 845, 419, 223, 867 (each ≤ 900, ±5 for formatting).
- `grep -rn "mfb_body_declared_errors\.rs" src .ai tests scripts` → no output.
- G3, G4, G5 (est. 10 min).

Verified 2026-10-19: G1 (`bash target/p176/g1.sh`, full name list vs `target/p176/g0/`) → 5017 = 5017, leaves SAME,
warnings 0; G2 → `ok. 3 passed`; list count → 3; census → 594, 845, 422, 223, 867; the `mfb_body_declared_errors\.rs`
grep → only the ceiling row (now removed); G3 `bash scripts/test-fast.sh --test architecture_guards --test no_type_strings
--test no_operator_strings` → `test-fast: ok` (unit `4998 passed; 2 ignored`, which includes `spec_citations_resolve`;
guards 8, 7 + 1 ignored, 8); G4 → 0; G5 → exit 0.

Commit: eb94a53fc

### Phase 2 — Engine inline tests and the architecture-guard change

The MIR move needs the guard change and lands with it. The other three engine moves share the
gate.

- [x] `tests/guards/architecture_guards.rs`: apply §4.1, so that `is_physical_scan_exempt`,
      `no_hand_picked_vreg` and `emission_lines` treat a manifest-relative path containing
      `/tests/` as test code. The commit message carries the 4-point justification of §4.1.
      Done: `manifest_relative()` (forward slashes on every host) and an inline `rel.contains("/tests/")` in each of
      the three (`grep -n 'contains("/tests/")'` → 3 hits, one per function). RED: the old guard over the moved MIR
      tests → `no_hand_picked_vreg` and `physical_registers_stay_in_the_backends` FAIL (`102` vreg literals).
- [x] (discovered) `code_above_tests` cut every file at its FIRST `#[cfg(test)]` of any kind, so an
      item-level helper (e.g. `handover.rs:116`'s indented `#[cfg(test)] fn len`) hid the rest of the file:
      52,502 production lines in 66 files (`python3 -I target/p176/archprobe.py`). It now cuts only at the test
      module (a column-0 `#[cfg(test)]` gating a `mod`, or a column-0 `mod tests`). The one violation that was
      hiding, `src/codegen/runtime/canvas/vulkan_handoff.rs:204` building `x19`–`x28` with `format!("x{n}")`,
      now takes `aapcs64_callee_saved_int_registers()` without its last entry (`x29`): the same `Reg`s in the
      same order. RED: the old `vulkan_handoff.rs` under the new guard → FAILS at :204. Byte identity:
      `bash target/p176/bytecmp.sh <release mfb before> <after> examples/canvas-demo linux-aarch64` → both
      AppImages SAME.
- [x] Move `src/codegen/engine/mir/mir.rs`'s inline tests → `mir/mir/tests/mod.rs` per §4.15.
- [x] Move `src/codegen/engine/analysis/borrow_get.rs`'s inline tests per §4.12.
- [x] Move `src/codegen/engine/analysis/handover.rs`'s inline tests per §4.13.
- [x] Move `src/codegen/engine/analysis/last_use.rs`'s inline tests per §4.14.
      Done (all four): `python3 -I target/p176/inline1.py` (one `items.py extract` per host) → 16, 27, 10, 6 tests;
      `bash target/p176/motion1.sh` → each host differs only in `mod tests {`/`}` → `mod tests;`.
- [x] G4: tighten the ceilings for the four files.
      Done: → TIGHTEN inline 6/2/9/2 for mir/borrow_get/handover/last_use; tighten drops exactly those four; re-run → 0.

Acceptance: the guards still bind production code and no longer see the moved fixtures. Check:
- G1 → equal.
- G2 `cargo test -p mfb --bin mfb -- codegen::engine::mir::mir::tests:: codegen::engine::analysis::`
  → 0 failed.
- `cargo test -p mfb --bin mfb -- --list 2>/dev/null | grep -cE 'engine::(mir::mir::tests|analysis::(borrow_get|handover|last_use)::tests)::'`
  → 16+27+10+6 = 59.
- G3 → pass.
- Negative check:
  1. Add the line `fn zz() -> String { format!("xmm{}", 1) }` to a production file outside
     every allow-list: `src/codegen/engine/analysis/handover.rs`, above its `#[cfg(test)]`.
  2. Run `cargo test --test architecture_guards shared_lowering_names_no_physical_register` →
     it FAILS, naming that line.
  3. Revert the line.
- G4, G5 (est. 14 min).

Verified 2026-10-19: G1 → 5017 = 5017, leaves SAME, warnings 0; G2 → `ok. 61 passed` (the 59 plus 2 other
`analysis::` tests the filter matches); list count → 59; census → hosts `548 6`, `1373 2`, `503 9`, `874 2`; new
files 737, 519, 548, 680; G3 fast tier → `test-fast: ok` (unit `4998 passed; 2 ignored`; guards 8, 7 + 1 ignored,
8); negative check → FAILS naming the planted line both above `handover.rs`'s first `#[cfg(test)]` (:37) and,
after the `code_above_tests` fix, below its item-level one (:791); reverted; G4 → 0; G5 → exit 0.

Commit: 118ec25a8 (guards and the vulkan fix), 418208f0d

### Phase 3 — Builtin package inline tests

- [x] Move `src/codegen/builtins/audio/mod.rs`'s inline tests per §4.4.
- [x] Move `src/codegen/builtins/canvas/mod.rs`'s inline tests per §4.5. Repoint the three
      `src/docs/spec/app/06_canvas.md` cites (`draw_item_variant_set_is_frozen`,
      `no_canvas_type_reaches_itself`, `canvas_text_has_one_glyph_walk`) to
      `canvas/tests/mod.rs`.
- [x] Move `src/codegen/builtins/crypto/mod.rs`'s inline tests per §4.6.
- [x] Move `src/codegen/builtins/datetime/mod.rs`'s inline tests per §4.7. Repoint
      `src/docs/spec/stdlib/02_datetime.md:197` to
      `[[src/codegen/builtins/datetime/func_fixed_offset.rs:__datetime_fixedOffset1]]`.
- [x] Move `src/codegen/builtins/encoding/mod.rs`'s two inline modules per §4.8.
- [x] Move `src/codegen/builtins/general/mod.rs`'s inline tests per §4.9.
- [x] Move `src/codegen/builtins/tls/gen_openssl.rs`'s inline tests per §4.11.
      Done (all seven): `python3 -I target/p176/h3.py` (eight `items.py extract` runs; encoding bottom-up, the two
      `#[path]` declarations for `codepage_tables` and `error_path_release_tests`) → 11, 14, 15, 11, 6 + 10, 29, 9
      tests; `bash target/p176/h3motion.sh` → only the module lines differ. The 3 canvas cites and the datetime cite
      are repointed.
- [x] G4: tighten the ceilings for the seven files.
      Done: → 7 TIGHTEN `inline` rows (4, 2, 2, 2, 9, 2, 3); tighten drops exactly those; re-run → 0.

Acceptance: every package's tests run from `tests/` files under unchanged paths, and the four
repointed cites name the file that holds the symbol. Check:
- G1 → equal.
- G2 `cargo test -p mfb --bin mfb -- codegen::builtins::audio::tests:: codegen::builtins::canvas::tests:: codegen::builtins::crypto::tests:: codegen::builtins::datetime::tests:: codegen::builtins::encoding:: codegen::builtins::general::tests:: codegen::builtins::tls::gen_openssl::error_path_release_tests::`
  → 0 failed.
- `cargo test -p mfb --bin mfb -- --list 2>/dev/null | grep -cE 'builtins::(audio::tests|canvas::tests|crypto::tests|datetime::tests|encoding::(tests|codepage_tables)|general::tests|tls::gen_openssl::error_path_release_tests)::'`
  → 11+14+15+11+16+29+9 = 105.
- `grep -rn "canvas/mod.rs:\(draw_item_variant_set_is_frozen\|no_canvas_type_reaches_itself\|canvas_text_has_one_glyph_walk\)\|datetime/mod.rs:__datetime_fixedOffset1" src/docs/spec`
  → no output.
- G3, G4, G5 (est. 12 min).

Verified 2026-10-19: G1 → 5017 = 5017, leaves SAME, warnings 0; G2 → `ok. 105 passed`; list count → 105; the stale
cite grep → 0; census → hosts inline 4, 2, 2, 2, 9, 2, 3 (encoding and gen_openssl are 1 over §4's "left inline"
because the `#[path]` line counts), new files 291, 418, 571, 402, 244, 234, 569, 291; G3 fast tier → `test-fast: ok`
(unit `4998 passed; 2 ignored`; guards 8, 7 + 1 ignored, 8); G4 → 0; G5 → exit 0.

Commit: b352f44fc

### Phase 4 — `builtins/mod.rs` and `registry/mod.rs` (largest blast radius)

- [x] Move `src/codegen/builtins/mod.rs`'s `mod tests` and `mod tcp_tls_error_declaration_tests`
      per §4.10. This uses two `#[path]` declarations; `plan111f_probe` stays inline.
      Done (`python3 -I target/p176/h4.py`, bottom-up): `tests/tcp_tls_error_declaration.rs` 3 tests, `tests/unit.rs`
      28; motion → only the module lines differ.
- [x] Move `src/codegen/registry/mod.rs`'s `mod tests` (split into three files) and
      `mod raw_result_block_ownership` per §4.16. Repoint
      `src/docs/spec/language/18_builtin-functions.md:245` and `.ai/man-content.md:117-118` to
      `registry/tests/mod.rs`.
      Done: `tests/raw_result_block_ownership.rs` 4 tests (its 15 `//!` lines move with it), then `tests/mod.rs` 12 +
      `selection_and_thread_probes.rs` 14 (+ `UnionUOverV`) + `rendering_and_gates.rs` 24 (+ `enum_variant`, `enm`,
      `intrinsic`, `rewrite_impl`, `sample_fast_path`); no visibility edit; motion → only the module lines and the 2 part
      `mod` lines differ. Both references repointed.
- [x] G4: tighten the inline ceilings for the two files.
      Done: → `TIGHTEN inline 23 (ceiling 1104)` builtins, `TIGHTEN inline 201 (ceiling 2695)` registry; tighten drops
      exactly those; re-run → 0.

Acceptance: the two largest hosts keep their module paths, apart from the two registry topic
segments. Check:
- G1 → equal.
- G2 `cargo test -p mfb --bin mfb -- codegen::builtins::tests:: codegen::builtins::tcp_tls_error_declaration_tests:: codegen::builtins::plan111f_probe:: codegen::registry::`
  → 0 failed.
- `cargo test -p mfb --bin mfb -- --list 2>/dev/null | grep -cE '^codegen::(builtins::(tests|tcp_tls_error_declaration_tests|plan111f_probe)|registry::(tests|raw_result_block_ownership|qualification_tests))::'`
  → 28+3+1+50+4+8 = 94. Here `builtins::tests::` is the unit module, not `builtins_codegen`.
- `cargo test -p mfb --bin mfb -- --list 2>/dev/null | grep -cE '^codegen::registry::tests::(selection_and_thread_probes|rendering_and_gates)::'`
  → 14+24 = 38.
- `python3 -I tools/loc-census/census.py --file src/codegen/registry/mod.rs` → second field
  200, and `… --file src/codegen/builtins/mod.rs` → 21.
- `for f in src/codegen/registry/tests/*.rs src/codegen/builtins/tests/unit.rs src/codegen/builtins/tests/tcp_tls_error_declaration.rs; do python3 -I tools/loc-census/census.py --file $f; done`
  → every second field ≤ 900.
- G3, G4, G5 (est. 12 min).

Verified 2026-10-19: G1 → 5017 = 5017, leaves SAME, warnings 0; G2 → `ok. 94 passed`; list counts → 94 and 38;
census → registry/mod.rs inline 201, builtins/mod.rs 23 (each over §4's figure by its `#[path]` lines), new files 643,
694, 812, 336, 675, 401; G3 fast tier → `test-fast: ok` (unit `4998 passed; 2 ignored`, which includes
`spec_citations_resolve`; guards 8, 7 + 1 ignored, 8); G4 → 0; G5 → exit 0.
Final gate: the ceiling file holds no `inline`/`test` row for an H path, and exactly the four `code` rows S, T, U and M
own (canvas 2371, gen_openssl 2997, borrow_get 1373, registry 3108); clippy per path over H's new files → 4 warnings,
all content lints of text moved verbatim (`while let`, two manual char comparisons, one complex type), so pre-existing.

Commit: —

## Validation Plan

- Tests: no test is added or removed. Per phase, the whole-binary `--list` count is unchanged
  (G1) and the moved modules' tests pass (G2). The per-phase `--list` greps sum to
  3 + 59 + 105 + 94 = 261, every test in H's 15 files.
- Coverage check: per-file coverage is not a plan-176 gate (author, 2026-10-17). Evidence, at
  7a4b09970:
  - `sh scripts/coverage.sh --bins` ran in a detached worktree (59 min; 4989 unit tests
    passed); then `sh scripts/coverage-check.sh` exited 1.
  - 241 `src/` files were already below the 98% per-file floor ("Files below 98% line coverage
    (GATE FAILURE)"), and overall coverage was 93.67%.
  - Examples among plan-176 hosts: `src/ir/shape.rs` 93.21%,
    `src/codegen/engine/control/builder_control.rs` 75.55%, `src/debugger/values.rs` 86.66%.

  A pure move changes no executed line, and moving an inline test block only removes test lines
  from its host's denominator. The pre-existing gate failure is reported to the owner
  separately and is not a plan-176 task.
- Runtime proof: the Phase 2 negative check proves the narrowed guard still binds. A
  constructed register name in production code still fails
  `shared_lowering_names_no_physical_register`.
- Doc sync:
  - spec: `src/docs/spec/app/06_canvas.md`, `src/docs/spec/language/18_builtin-functions.md`,
    `src/docs/spec/tooling/04_audit-format.md`, `src/docs/spec/stdlib/02_datetime.md`;
  - `.ai/man-content.md`;
  - prose comments in `src/codegen/builtins/tests/general_declared_errors.rs`,
    `src/codegen/builtins/canvas/func_poll_mouse.rs` and `src/audit/collect/source.rs`.

  No compiler contract changes, so no other spec or `mfb man` topic changes.
- Final gate (once, after Phase 4), est. 6 min:
  - `cargo test -p mfb --bin mfb spec_citations_resolve` and
    `cargo test --test architecture_guards --test no_type_strings --test no_operator_strings --test file_size_budget`
    → all pass.
  - The banner names no H path except the `code` rows of `src/codegen/builtins/canvas/mod.rs`,
    `src/codegen/builtins/tls/gen_openssl.rs`, `src/codegen/engine/analysis/borrow_get.rs` and
    `src/codegen/registry/mod.rs` (owned by S, T, U, M).
  - `cargo clippy --all-targets 2>&1 | grep -c '^warning'` → equal to its value at G0 of
    Phase 1.

  The full workspace suite runs once, in plan-176-V.

## Decisions

- Rule revised (owner, 2026-10-18). All limits count non-comment, non-blank LOC. The scope is
  `.rs`/`.mfb` under `src/` `packages/` `examples/` `repository/` `tests/`.

  | Kind | Applies to | Limit | Over the limit → |
  |---|---|---|---|
  | `code` | code LOC of a file that is not a test-only file | 1000 | split the file into modules |
  | `inline` | inline test LOC (`#[cfg(test)]` items / MFBASIC `TESTING … END TESTING`) inside a file that is not a test-only file | 250 | move the tests out into their own test-only file(s) |
  | `test` | total LOC of a test-only file (path component `tests`, `test_*.mfb`, `#![cfg(test)]`, or a `#[cfg(test)] mod x;` target and its subtree) | 1000 | split the test-only file into modules |

  Parts aim for ≤ 900 LOC of their kind. A moved inline block becomes one test-only file unless
  it is over 1000. Ceiling rows are `ceiling <code|inline|test> <n> <path>`.
- No exemptions. The guard warns with a banner and never fails. Generated files (`GENERATED`
  banner) are the only exemption. A new module is always an acceptable split (owner,
  2026-10-14 and 2026-10-18).
- Extracted src test modules go into `foo/tests/mod.rs` (plus topic files only when over 1000),
  per plan-176-A. Where the module name is not `tests`, a `#[path]` puts it under a `tests/`
  directory and the name is kept (author, 2026-10-14).
- Guards that classify test code by file name are updated in the same commit as the move that
  needs them, with the 4-point justification (author, 2026-10-14). H's change is the `/tests/`
  component in `architecture_guards` (§4.1, Phase 2).
- Pre-existing defects found while mapping are fixed in the letter that touches them (author,
  2026-10-14). H fixes two:
  - the `[[…datetime/mod.rs:__datetime_fixedOffset1]]` cite, which named a test string, not the
    definition;
  - the stale `.ai/man-content.md` path.
- Glob-cycle imports (`use super::*;` in parts, `use part::*;` in `mod.rs`) compile without
  warnings (verified, `/tmp/loc/probe2`). An intermediate `tests/mod.rs` glob consumed only by
  children does not warn (verified, `/tmp/hij_probe`).
- Per-file coverage is not a plan-176 gate (author, 2026-10-17). The measured evidence is in
  §Validation Plan "Coverage check".

## Corrections

- 2026-10-18: Rule revised by the owner: test-only files limit 1000 (was 250), inline tests
  250, no exemptions. Dropped 22 files:
  - `src/codegen/builtins/canvas/helper_render/tests/mod.rs`;
  - under `src/codegen/builtins/tests/`: `canvas.rs`, `collections.rs`, `corpus.rs`,
    `declared_errors_all.rs`, `declared_errors_sweep.rs`, `diagnostics.rs`,
    `general_declared_errors.rs`, `inplace.rs`, `inplace_compact.rs`, `nir_fault.rs`,
    `nir_json.rs`, `nir_validation.rs`, `optimizer_globals.rs`, `os.rs`, `plan_validation.rs`,
    `platform_hooks.rs`, `validation.rs`;
  - `src/codegen/builtins/tls/gen_macos/tests.rs`, `src/codegen/builtins/tls/gen_schannel_tests.rs`;
  - `src/codegen/engine/regalloc/tests.rs`, `src/codegen/engine/tests/tests.rs`.

  Other structural changes:
  - Each of the 13 inline blocks now moves to ONE test-only file. Only `registry/mod.rs`'s
    2160-LOC `mod tests` splits, into 3 files. Created files: 154 → 23.
  - `mfb_body_declared_errors.rs` regroups from 10 parts to 3 files.
  - The three exemptions are gone: two because their hosts (`corpus.rs` 766, `diagnostics.rs`
    569) are no longer split, and `platform_failure/forwarding.rs` (867) because it is an
    ordinary part ≤ 900.
  - Phases go from 5 to 4. Tests in scope go from 459 to 261. Counts were re-taken at
    `ce7a995ca`: crypto has 15 tests, not 14 (`open_pads_the_overload_its_arguments_select`,
    bug-863).
  - Added a prose sweep the earlier design missed: 4 comments name
    `mfb_body_declared_errors.rs`.
- 2026-10-19 (execution): `/tmp` is wiped by session restarts here, so G0 is the full sorted
  `--list` kept in `target/p176/g0/` (ignored by the repository), and G1 compares the count and
  the leaf names. Per the owner, G2/G3 run through `bash scripts/test-fast.sh` (2026-10-18) and
  every check runs on macOS only (2026-10-19). No clippy count was taken at Phase 1's G0, so the
  final gate checks clippy per path instead.
- 2026-10-19 (execution): Phase 2's negative check as written (the planted line above
  `handover.rs`'s `#[cfg(test)]` at :791) PASSED the guard, because `code_above_tests` stopped at
  the earlier item-level `#[cfg(test)]` at :116. That exposed a guard gap (52,502 unscanned
  production lines) and one hidden violation; both fixed in `118ec25a8` (a discovered task).

## Summary

H is pure motion of test code into `tests/` directories: 15 hosts, 261 tests, 0 renamed, 23
new files, and no exemption. Thirteen inline blocks each become one test-only file with
unchanged test paths. The two over-1000 test files split by responsibility
(`mfb_body_declared_errors` into 3 files, `platform_failure` into 2). The registry's 2160-LOC
`mod tests` splits into 3, and its two topics add a path segment to 38 tests.

The engineering risk is in three places:
- the architecture-guard change, narrowly widened to the `/tests/` component, proven against
  the 17 test directories and pinned by a negative check;
- `builtins/mod.rs`'s name collision with the `builtins_codegen` root, resolved with `#[path]`;
- the registry move, which goes last.

Host code, the `builtins_codegen` wiring and every module name are untouched.
