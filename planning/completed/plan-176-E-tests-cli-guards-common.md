# plan-176-E: CLI, codegen, guard and shared-helper test files

Last updated: 2026-10-18
Effort: medium (1h–2h)
Depends on: plan-176-D

E splits the five test-only files under `tests/cli`, `tests/codegen`, `tests/guards` and
`tests/common` that are over the 1000-LOC test-only limit, so that every file is ≤ 900 (the
part target). Each split is one root plus one or two parts, cut on responsibility seams:
`tests/cli/cli_dbg.rs`, `tests/cli/cli_repo_install.rs`, `tests/codegen/app_special_keys.rs`,
`tests/guards/no_type_strings.rs`, and the shared helper module `tests/common/mod.rs`. Each
`tests/<dir>/<name>.rs` stays the `[[test]]` target root and declares its parts as
`#[path = "<name>/<part>.rs"] mod <part>;`. `tests/common/mod.rs` gains one sibling part,
`tests/common/process.rs`. No test is added, removed or renamed. Only the module path of a
moved test changes (`name` → `<part>::name`; in `cli_dbg`, `native::name` → `<part>::name`).

The single outcome: after E lands, `cargo test --test file_size_budget` shows no row for any of
E's files, and `tests/guards/file_size_ceilings.txt` has no row for any of them.

References:

- `planning/plan-176-A-file-size-guard.md`: the rule (code ≤ 1000, inline tests ≤ 250,
  test-only file ≤ 1000), §Decisions, the `#[path]` topic layout (§Verified properties), and
  "Splitting an over-budget file" in `.ai/build-tooling.md`.
- `.ai/testing-gates.md`: § "Splits must sweep man AND spec citations". Line 407 names
  `tests/common/mod.rs`'s pty helpers.
- `.ai/build-tooling.md`: rustfmt/clippy policy. Line 90 names `tests/common/mod.rs::build_project`
  (stays in `mod.rs`).
- `.ai/codegen-invariants.md`: line 94 carries a stale path, `tests/no_operator_strings.rs`.
  Line 628 runs `cargo test --test no_type_strings census_by_file -- --ignored --nocapture`.
- `.ai/remote_systems.md`: box 2223, used for the Linux-gated compile check.
- Spec topics whose citations name E files: `src/docs/spec/architecture/02_frontend.md:228`
  (`[[tests/guards/no_type_strings.rs]]`, file only, which stays valid).
- `scripts/coverage.sh:29` names `tests/common/mod.rs`'s `mfb_exe()` (stays in `mod.rs`).

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-176-D is complete and archived | `ls planning/plan-176-D-* planning/completed/plan-176-D-*` → only `planning/completed/…` | MET (2026-10-19: `planning/completed/plan-176-D-tests-canvas.md` only) |
| The file-size guard exists and passes | `cargo test --test file_size_budget` → passes | MET (2026-10-19, plan-176-A landed; ran in D Phase 4) |
| E's files have no uncommitted changes | `git status --porcelain -- tests/cli/cli_dbg.rs tests/cli/cli_repo_install.rs tests/codegen/app_special_keys.rs tests/guards/no_type_strings.rs tests/common .ai/testing-gates.md .ai/codegen-invariants.md` → empty | MET (2026-10-19) |

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

- Every file in §2's table, and every file E creates, is ≤ 900 test LOC
  (`python3 -I tools/loc-census/census.py --file <path>` → second field ≤ 900).
- `cargo test --test file_size_budget` prints no row naming an E path, and the ceiling file
  holds no row for an E path.
- Every E target lists the same number of tests before and after each phase.

### Non-goals (explicit constraints)

- No behavior change. No test body is edited beyond moving it. The allowed edits are: adding
  `use super::*;` at the top of a part; `pub(super)` on `cli_dbg`'s support items (counts in
  §4.1); the cfg'd declarations that replace `cli_dbg`'s inline `mod native`; the
  `#[allow(unused_imports)]` re-export line in `tests/common/mod.rs`; and the doc-reference
  fixes listed in §4.
- No `[[test]]` target is renamed, added or removed. `Cargo.toml` is not edited. No `#[test]`
  fn is renamed.
- Every `common::<item>` path and every `use common::*;` glob in the 355 includers keeps
  resolving, with no edit to any includer.
- No guard weakens. `no_type_strings.rs` is split with every path list and allowlist moved
  verbatim.

## 2. Current State

Counts: `python3 -I /tmp/loc/census.py "$PWD"` at HEAD `ce7a995ca` (2026-10-18), read from
`/tmp/loc/census.json` (for a test-only file the `test` field is the whole file's LOC). Item
spans: `python3 -I /tmp/loc/span.py <file> <a> <b>` on the same tree. After plan-176-A the same
numbers come from `python3 -I tools/loc-census/census.py --file <path>`. Test counts:
`grep -cE '^\s*#\[test' <file>`.

| File | Kind | Test LOC | Tests |
|---|---|---|---|
| `tests/cli/cli_dbg.rs` | test-only | 1473 | 22 |
| `tests/cli/cli_repo_install.rs` | test-only | 1241 | 10 |
| `tests/codegen/app_special_keys.rs` | test-only | 1148 | 37 |
| `tests/common/mod.rs` | test-only (helpers) | 1228 | 0 |
| `tests/guards/no_type_strings.rs` | test-only | 1179 | 8 |

`cli_dbg.rs` is the only one that changed since the `17a9b5799` planning snapshot
(`git log --oneline 17a9b5799..HEAD -- <the five files>` → `5fbbff48f`, `b1cbedfcb`,
`df8bbd32c`, `3a17824e5`, all on `cli_dbg.rs`): it gained `greeting_program`,
`env_gives_a_restarted_program_its_changes` and
`a_watched_write_under_the_breakpoint_a_thread_is_paused_on_is_seen`. Its §4.1 layout is
re-measured at `ce7a995ca`.

### Verified properties

- **`#[path]` topic children keep test filters working.** See plan-176-A §Verified properties
  (`/tmp/loc/probe`). `cargo test --test rt_demo list_a` ran the topic's test.
- **A root `use <support>::*;` used only by child topics through `use super::*;` produces no
  warning.** Probe `/tmp/loc_e_probe2`: the root declares `model` and `topic`, has
  `use model::*;`, and uses nothing itself. The topic's test calls `mk()`.
  `cargo test --test t2` printed no warning and `1 passed`.
- **Glob-cycle imports compile without warnings.** `use super::*;` in parts and `use part::*;`
  in the root: probe `/tmp/loc/probe2` printed `6` with no warning.
- **A `pub use` of items an includer never touches warns, glob or not.** `/tmp/loc/probe3`
  printed `warning: unused import: pty::*`. With `#[allow(unused_imports)]` on the line, the
  probe printed no warning. So the re-export line in `tests/common/mod.rs` carries that
  attribute.
- **`#![allow(dead_code)]` in `tests/common/mod.rs` covers its parts.** Lint levels inherit
  into child modules. In `/tmp/loc_e_probe`, an unused `pub fn` in a child file of `common`
  produced no `dead_code` warning.
- **A module loaded through `#[path]` owns its directory.** `mod process;` inside the
  `#[path]`-loaded `tests/common/mod.rs` resolves to `tests/common/process.rs` for every
  includer, so no includer is edited.
- **`tests/common` needs no Cargo stanza.**
  `tests/guards/test_targets_registered.rs::NOT_TEST_TARGETS = ["common"]` skips the
  directory. `test_files_in_subdirectories` reads only the direct `.rs` children of
  `tests/<dir>/`, so the part directories `tests/<dir>/<name>/` are never seen.
- **355 includers.** `grep -rl 'path = "../common/mod.rs"' tests | wc -l` → 355 (2026-10-18).
- **No E file respawns itself, reads its own source, or uses `module_path!`/`file!()`.**
  `grep -nE 'current_exe|--exact|module_path!|file!\(\)'` over the E files → none. The one
  `--exact` user in the tree is `tests/net/rt_port_gate_is_cross_process.rs` (not E).
- **No symbol citation names an E file.**
  `grep -rnoE "tests/(cli/cli_dbg|cli/cli_repo_install|codegen/app_special_keys|guards/no_type_strings)\.rs(::|:)\w+" src/docs .ai scripts AGENTS.md .github`
  → none. `grep -n '^tests/' scripts/coverage-exceptions.txt` → none.
- **cfg gates in E files** (`grep -hoE '#!?\[cfg\([^]]*\]'`):
  - `cli_dbg.rs`: one `#[cfg(any(target_os = "macos", all(target_os = "linux", target_arch = "aarch64")))]`
    on `mod native`, its `not(...)` twin on `the_debugger_is_not_supported_off_macos_and_linux_aarch64`,
    `#[cfg(target_os = "linux")]` / `#[cfg(target_os = "macos")]` on the two `signed_mfb`
    variants, and `#[cfg(target_os = "macos")]` on `an_unsigned_mfb_explains_how_to_sign_itself`
    and `an_app_s_threads_are_the_app_and_mfb_main`.
  - `tests/common/mod.rs`: `#[cfg(unix)]` and `#[cfg(not(unix))]` variants, plus
    `#[cfg(target_os = "macos")] build_short_write_interposer`.

  The Windows and Linux variants are therefore compile-checked off-host (§Phase 3, §Phase 2).
  The Windows target is installed (`rustup target list --installed` → `x86_64-pc-windows-msvc`).
- **Pre-existing stale references found while mapping** (fixed in E):
  - `.ai/codegen-invariants.md:94` says `tests/no_operator_strings.rs`, and
    `ls tests/no_operator_strings.rs` → no such file. The file is
    `tests/guards/no_operator_strings.rs`.
  - `tests/guards/no_type_strings.rs` doc comments (lines 13, 53, 97) say
    `tests/architecture_guards.rs` (`grep -n 'tests/architecture_guards' tests/guards/no_type_strings.rs`
    → 3 hits). The file is `tests/guards/architecture_guards.rs`. Line 13 also names
    `builtins_no_hand_picked_vreg`, and `grep -rn 'fn builtins_no_hand_picked_vreg' tests src`
    → none. The guard is `no_hand_picked_vreg`.

## 3. Design Overview

The conventions are plan-176-A's integration-test layout, applied per file:

- The root `tests/<dir>/<name>.rs` keeps `#[path = "../common/mod.rs"] mod common;`, its `use`
  lines, any `#![cfg]`, and the helpers shared by more than one part. Each part is declared
  `#[path = "<name>/<part>.rs"] mod <part>;` (2 LOC, counted in the root's figure; 3 LOC with
  a cfg line).
- Every part starts with `use super::*;` and holds whole `#[test]` fns plus the helpers only
  they use. Those helpers stay private, and no test fn changes visibility (libtest collects
  private `#[test]` fns in child modules).
- A part whose helpers another part also uses gives those items `pub(super)`, and the root
  adds `use <part>::*;` (only `cli_dbg`).
- Each file is split into the fewest parts that keep every file ≤ 900. The cuts follow the
  responsibility seams of the planning map (`/tmp/loc/maps/E.md`): neighbouring topics are
  merged, not re-cut.
- `tests/common/mod.rs` keeps most helpers and gains one plain sibling part
  (`mod process;`), re-exported with `pub use process::*;`.

The gate class is test motion: before == after `--list` counts per target, the moved tests
pass, no new warnings, then tighten. Byte-identity does not apply; no compiler code changes.

Risk concentrates in `tests/common`, which 355 targets compile. A wrong re-export or a missed
cfg variant breaks many of them at once, and the Windows (`#[cfg(not(unix))]`) variants are
invisible on the macOS host. It lands last, alone, and gets the Windows compile check. Second
is `cli_dbg.rs`: dissolving its cfg-gated `mod native` must keep the cfg on every declaration,
or the Linux-x86_64 and Windows builds try to compile macOS debugger code. It gets the box-2223
compile check.

Rejected alternatives:

- *`tests/common/process.rs` behind `#[path]` per includer.* This needs 355 includer edits. The
  plain `mod process;` inside the `#[path]`-loaded `mod.rs` needs none.
- *Keeping `cli_dbg`'s `mod native` inline and moving it whole to one file.* The native module
  is 1409 LOC (`span.py tests/cli/cli_dbg.rs 82 1697` minus its 3 wrapper lines), over 1000 by
  itself; it has to be cut, and cutting it at the root keeps one cfg line per declaration.
- *A separate `cli_dbg/session.rs` support file.* It would be a third part; folding the
  session helpers into the first part keeps both parts ≤ 900 with one fewer file.
- *Leaving `pub use process::*;` unannotated.* It warns in every includer that uses nothing
  from the part (probe3).

## 4. Detailed Design

The LOC figures are the summed `span.py` counts of the moved items plus the added lines
(`use super::*;`, declarations), measured at `ce7a995ca`. For the four files unchanged since
`17a9b5799` (`git log 17a9b5799..HEAD` → no commits) the line ranges are the same at both;
they are labelled "as of ce7a995ca". Item and test NAMES are authoritative if lines drift.
Every part starts with `use super::*;`.

### 4.1 tests/cli

**`tests/cli/cli_dbg.rs`** (1473, 22 tests)
- Today the native tests sit in
  `#[cfg(any(target_os = "macos", all(target_os = "linux", target_arch = "aarch64")))] mod native { use super::*; … }`
  (L81-1697 as of ce7a995ca). The inline module is dissolved. Each part is declared at the root
  with that same `#[cfg(any(...))]` line above its `#[path]` (3 LOC per declaration). The root
  adds `#[cfg(any(...))] use session_launch_and_list::*;` (2 LOC). Test paths change from
  `native::<name>` to `<part>::<name>` (`grep -rn 'native::' scripts .github .ai src/docs` →
  none).
- Root (68): header, `dbg`. Keeps `help_and_usage_errors_on_every_host` and
  `the_debugger_is_not_supported_off_macos_and_linux_aarch64` (with its `#[cfg(not(any(...)))]`).
  68 = 60 (`span.py tests/cli/cli_dbg.rs 1 80`) + 2 × 3 + 2.
- `cli_dbg/session_launch_and_list.rs` (604 = 1 + 194 + 186 + 223), 8 tests:
  - Support (194), `pub(super)` on 17 items, 4 methods and 3 fields: `signed_mfb` (both
    `#[cfg(target_os)]` variants side by side), `Session`, `executable_name`, `image`,
    `attach_allowed`, `NOT_A_DEBUG_BUILD`, `TICKER`, `build_debug`, `is_address`,
    `line_count`, `debug_example`, `EXITED`, `body`, `REGISTER_COLUMNS`, `SEPARATOR`,
    `table_rows`.
  - Launch and attach (186): `a_launched_program_does_not_run_and_is_killed_on_quit`,
    `an_attached_program_pauses_and_runs_on_after_quit`,
    `programs_that_are_not_mfbasic_or_not_native_are_refused`,
    `an_unsigned_mfb_explains_how_to_sign_itself` (its `#[cfg(target_os = "macos")]` moves
    with it). Local: `banner_pid`, `process_exists`, `host_image`.
  - List (223): `list_shows_a_launched_debug_build_s_functions_and_registers`,
    `list_shows_an_attached_program_s_bound_imports`,
    `breakerror_stops_where_a_watched_error_is_raised`,
    `list_func_on_a_normal_build_shows_only_linked_functions`. Local: `function_rows`,
    `RAISES`, `FUNCTION_COLUMNS`.
- `cli_dbg/threads_watch_and_run.rs` (807 = 1 + 192 + 316 + 117 + 181), 12 tests:
  - Threads (192): `a_launched_program_lists_its_one_thread_and_steps`,
    `an_attached_threaded_program_lists_its_threads_and_arenas`,
    `an_app_s_threads_are_the_app_and_mfb_main` (its `#[cfg(target_os = "macos")]` moves with
    it). Local: `THREAD_COLUMNS`, `ARENA_COLUMNS`.
  - Show and watch (316): `show_type_reads_a_returned_string_from_the_arena`,
    `env_gives_a_restarted_program_its_changes`,
    `a_watched_write_under_the_breakpoint_a_thread_is_paused_on_is_seen`,
    `a_watchpoint_stops_when_a_thread_changes_the_watched_bytes`,
    `a_breakpoint_with_commands_runs_them_and_runs_on`,
    `show_error_names_the_fault_a_thread_stopped_on`. Local: `greeting_program` (its one
    caller is `env_gives_a_restarted_program_its_changes`: `grep -nw greeting_program tests/cli/cli_dbg.rs`
    → L760 definition, L1112 use).
  - Breakpoints and report (117): `a_tested_breakpoint_stops_once_and_a_jump_returns_early`,
    `show_report_prints_the_report_the_program_wrote_as_it_exited`.
  - Run to end (181): `run_stops_at_breakpoints_and_at_the_end`. Local: `EXIT_SYMBOL`,
    `exit_stop`, `exit_name`, `c_library`, `BREAKPOINT_COLUMNS`.
- Check: 68 + 604 + 807 = 1479 = 1473 − 4 (`mod native {`, its cfg, `use super::*;`, `}`) + 2
  (`use super::*;` per part) + 6 (two cfg'd declarations) + 2 (cfg'd `use`).
- Every local's users were checked with `grep -nw <name> tests/cli/cli_dbg.rs`: each local is
  used only inside its part; every support item is used from both parts or from the
  support item block itself.
- Hazards: no citation. `scripts/sign-debugger.sh` is reached through
  `env!("CARGO_MANIFEST_DIR")` (absolute), so it is unaffected. `scripts/README.md:260` names
  the target only.

**`tests/cli/cli_repo_install.rs`** (1241, 10 tests)
- Root (520): header; keeps `repo_end_to_end_install_verifies_signed_package` (L7-143 as of
  ce7a995ca), `identical_vendor_blobs_keep_every_source_filename` (L1036-1206) and
  `repo_gc_reclaims_an_orphaned_blob_and_leaves_live_packages_installable` (L1207-1505, one
  255-LOC `#[test]` fn). Declares `remove_update_and_resolve`. Archived bug docs cite
  `cli_repo_install.rs:identical_vendor_blobs_keep_every_source_filename`, which stays valid.
- `cli_repo_install/remove_update_and_resolve.rs` (724 = 1 + 723), L144-1035 verbatim, 7 tests:
  `remove_cascades_to_packages_that_import_the_target`,
  `update_targeted_applies_the_abi_advisory_and_preserves_pin`,
  `update_of_a_pinned_dependency_requires_confirmation`,
  `install_warns_on_floor_drift_and_errors_on_pin_drift`,
  `spike_file_added_package_with_registry_ident_survives_update`,
  `repo_resolver_selects_substitute_and_locks_deterministically`,
  `repo_resolver_reports_diamond_conflict_naming_both_requirers`.
- Check: 520 + 724 = 1244 = 1241 + 2 (declaration) + 1 (`use super::*;`).
- Hazards: no citation. `Cargo.toml` and `tests/guards/test_targets_registered.rs` quote the
  target name only.

### 4.2 tests/codegen

**`tests/codegen/app_special_keys.rs`** (1148, 37 tests)
- The macOS section (L165-639 as of ce7a995ca, from the `// macOS (AppKit)` banner to the end
  of `macos_key_up_reports_nothing_in_line_mode_except_on_the_canvas`) is one contiguous block
  whose items no other section uses (`grep -nw` of `MacModel`, `MacKey`, `MacObject`,
  `MacSurface`, `run_mac`, `mac_press`, `appkit` → every hit in L172-629). It moves verbatim.
- Root (742 = 1148 − 408 + 2): header; helpers `PROGRAM`, `CONSOLE_PROGRAM`, `A_PRESS`,
  `LEFT_PRESS`, `DELETE_PRESS`, `F5_PRESS`, `LEFT_RELEASE`, `SHIFT_LEFT_PRESS`, `PIPE`,
  `ncode_for`, `ncode_of`, `ncode_of_build`, `hex`, `assert_injects`, `key_down`; the GTK
  section (`mod gdk`, `GtkModel` and its `impl Externals`, 9 `gtk_*` tests); the Windows
  section (`mod win32`, `WinModel`, `win_ncode`, `WNDPROC`, `EDITPROC`, `run_win*`,
  `win_each_proc*`, `WinMainModel`, 14 `windows_*` key tests and 3 `windows_main_*` tests).
  Keeps 26 tests. Keeps `use common::ncode_emu::{data_cstr, find_calls, CallSite, Externals, Machine, Sym};`.
  Declares `macos`.
- `app_special_keys/macos.rs` (409 = 1 + 408), 11 tests:
  `macos_every_surface_registers_a_key_down_handler_that_the_model_drives`,
  `macos_a_mode_switching_program_registers_every_key_down_handler`,
  `macos_left_injects_kitty_left`, `macos_forward_delete_injects_kitty_delete`,
  `macos_f5_injects_kitty_f5`, `macos_left_release_injects_kitty_release`,
  `macos_shift_left_injects_kitty_shift_left`, `macos_auto_repeat_injects_a_repeat`,
  `macos_shift_a_injects_its_text`, `macos_ctrl_a_injects_the_key_without_text`,
  `macos_key_up_reports_nothing_in_line_mode_except_on_the_canvas`. Local: `mod appkit`,
  `MacKey`, `MacObject`, `MacModel` (with its `impl Externals`), `MacSurface`,
  `mac_surfaces_in`, `class_name`, `mac_surfaces`, `run_mac`, `run_mac_in_mode`, `mac_press`.
- Surface: none. Every moved item stays private; the root's helpers are visible to the child.
- Hazards: no citation. Trait impls need no visibility change.

### 4.3 tests/guards

**`tests/guards/no_type_strings.rs`** (1179, 8 tests)
- Root (752 = 1179 − 238 − 191 + 2): every helper, scanner and allowlist (`is_excluded_from_scan`,
  `is_test_file`, `test_free_lines`, `rs_files`, `BOUNDARY_FILES`, `is_grammar_file`,
  `is_boundary_file`, `TYPE_PARAM_NAMES`, `SPELLINGS_MATCH`, `SPELLINGS_COMPARE`,
  `GRAMMAR_METHODS`, `GRAMMAR_TOKENS`, `CONSTRUCTED_PREFIXES`, `TYPE_KEYED_TABLES`, the site and
  pattern scanners, `C_CARRIER_NAMES`, `C_SPELLINGS`, `c_type_strings`, `BUDGETS`, `bucket`,
  `CLASSES`, `scan_tree`, `print_budget_table`). Keeps `no_type_strings` and `census_by_file`.
  Declares `self_checks`.
- `no_type_strings/self_checks.rs` (430 = 1 + 238 + 191), 6 tests:
  `scanners_fire_on_their_own_needles` (L1151-1400 as of ce7a995ca, one 238-LOC fn) and the
  closed-list block L1526-1785: `immediate_operand_class_vocabulary_is_closed`,
  `curated_type_keyed_tables_all_exist`, `the_grammar_file_is_exactly_one`,
  `boundary_list_is_closed`, `boundary_files_exist_and_are_justified`. Local:
  `quoted_literals_after`, `immediate_class_argument`, `as_literal`.
- Path lists: `BOUNDARY_FILES`/`is_grammar_file`, `TYPE_KEYED_TABLES`, `is_excluded_from_scan`,
  `BUDGETS`/`bucket` and `c_type_strings` stay in the root. The needle paths inside
  `scanners_fire_on_their_own_needles` and the closed-list assertions move verbatim to
  `self_checks.rs`.
- `census_by_file` stays at its root path, so `.ai/codegen-invariants.md:628`'s command is
  unchanged.
- Surface: none.
- Citations: `src/docs/spec/architecture/02_frontend.md:228` cites the file only (stays valid).
- Stale references fixed here: in the doc comments, `tests/architecture_guards.rs` (lines 13,
  53, 97) → `tests/guards/architecture_guards.rs`, and `builtins_no_hand_picked_vreg` (line 13)
  → `no_hand_picked_vreg`. Separately, `.ai/codegen-invariants.md:94`
  `tests/no_operator_strings.rs` → `tests/guards/no_operator_strings.rs` (§2).

### 4.4 tests/common

**`tests/common/mod.rs`** (1228, 0 tests). This is not a target (`NOT_TEST_TARGETS = ["common"]`).
- `tests/common/mod.rs` (597 = 14 + 3 + 82 + 191 + 144 + 99 + 64): `#![allow(dead_code)]`
  (pre-existing, kept as is; lint levels inherit into the part);
  `pub mod canvas_image; pub mod debug_report; pub mod ncode_emu;`; the std and
  `mfb_repository` `use` lines (including the repo-helper imports at L1214-1220,
  `use mfb_repository::{crypto, store::Store}`, `std::io::{BufRead, BufReader}`, `Child`,
  `Once`); `mod process;` +
  `#[allow(unused_imports)] // each includer uses a subset of these helpers` +
  `pub use process::*;` (3 LOC). It keeps:
  - project helpers (82): `unique_nonce`, `mfb_path_literal`, `temp_project`,
    `exit_description`, `BUILD_RELEASE_MFB`, `mfb_exe`;
  - build helpers (191): `build_project`, `build_project_opt`, `built_executable`,
    `APP_BUILD_FLAG`, `build_app`, `build_app_debug`, `build_app_with`, `app_binary`,
    `host_libc_flavor`, `build_ncode`, `build_ncode_opt`, `build_ncode_app`,
    `build_ncode_app_with`, `build_linux_elf`;
  - ncode checks (144): `assert_no_aligned_bank_result_reads`,
    `assert_no_inplace_sxtw_of_aligned_bank`, `is_stack_base`, `is_stack_register`,
    `outside_inline_error_exits`;
  - repo helpers (99): `BUILD_REPO`, `RepoProcess`, `repo_exe`, `start_repo`, `open_store`,
    `mfb_repo_home`, `run_mfb`, `run_mfb_plain`, `run_mfb_in`;
  - fixtures (64): `fixture_truetype`, `PortGate` (with its impls).
- `tests/common/process.rs` (635 = 1 + 209 + 155 + 85 + 185), everything that runs a child
  process:
  - bounded runs (209): `run_bounded`, `run_bounded_without_inherited_fds`,
    `own_process_group` (both cfg variants), `kill_process_tree`, `run_bounded_command`,
    `run_capture_with_env`, `run_with_stdin`, `run_with_env`, `run_bounded_with_rss`;
  - interposers (155): `build_close_interposer`, `build_short_write_interposer`;
  - stdio (85): `hex`, `python_exe`, `decode_hex`, `run_with_readonly_stdout`;
  - pty (185): `run_under_pty`, `run_under_pty_bytes`, `run_pty_prompt_interaction`,
    `run_pty_prompt_interaction_echo_off`, `run_pty_prompt_interaction_inner`.
  Line ranges as of ce7a995ca: L382-536, L537-711 (interposer and `run_with_*`), L851-1213,
  L1527-1615.
- The per-group figures are the planning map's per-part counts minus each part's
  `use super::*;` line. Check: 597 + 635 = 1232 = 1228 + 3 (declaration and re-export) + 1
  (`use super::*;`).
- Surface: no visibility change. The private items `built_executable`, `build_app_with`,
  `BUILD_REPO`, `BUILD_RELEASE_MFB` stay in `mod.rs` with all their callers;
  `run_bounded_command` moves with its only callers, `run_bounded` and
  `run_bounded_without_inherited_fds` (`grep -n run_bounded_command tests/common/mod.rs` →
  L401, L449, L485). Every `#[cfg(unix)]`, `#[cfg(not(unix))]` and
  `#[cfg(target_os = "macos")]` attribute moves with its item.
- `tests/common/ncode_emu.rs` (612) is not touched; `src/codegen/io/input/mod.rs:32`'s
  `#[path]` to it is unchanged.
- Doc refs: `.ai/testing-gates.md:407` "`tests/common/mod.rs`'s pty helpers" →
  "`tests/common/process.rs`'s pty helpers". `.ai/build-tooling.md:90`
  (`tests/common/mod.rs::build_project`) and `scripts/coverage.sh:29` (`mfb_exe()`) stay
  valid: both items stay in `mod.rs`.

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

Every phase uses the same gate, run in this order. `$T` is the phase's target list.

- **E0 (once, before Phase 1's first edit):**
  `for t in app_special_keys no_type_strings cli_repo_install cli_dbg; do printf '%s ' $t; cargo test --test $t -- --list 2>/dev/null | grep -c ': test$'; done > /tmp/e_before.txt`
  and `cargo check --all-targets 2>&1 | grep -c '^warning' > /tmp/e_warn_before.txt`
  (est. 10 min cold).
- **E1:** the same loop restricted to `$T`, compared against the matching lines of
  `/tmp/e_before.txt` → identical. Then `cargo check --all-targets 2>&1 | grep -c '^warning'`
  → equal to `/tmp/e_warn_before.txt`.
- **E2:** `cargo test` with one `--test <t>` for each target in `$T` → 0 failed.
- **E3:** `cargo test --test no_type_strings --test test_targets_registered` → all pass
  (est. 2 min).
- **E4:** `cargo test --test file_size_budget 2>&1 | grep -E '^  (NEW|GREW)'` → no output.
  Every `TIGHTEN`/`STALE` row names a file of this phase. Then
  `MFB_LOC_TIGHTEN=1 cargo test --test file_size_budget`, rerun →
  `grep -cE '^  (NEW|GREW|STALE|TIGHTEN)'` → `0`. Commit `tests/guards/file_size_ceilings.txt`
  with the phase (est. 2 min).
- **E5:** `cargo fmt --check` → no output (est. 20 s).

### Phase 1 — tests/codegen and tests/guards

- [x] Split `tests/codegen/app_special_keys.rs` per §4.2 (the macOS section → `macos.rs`).
      Done: `python3 -I target/p176/items.py split` (moves named top-level items; lexes strings and comments) →
      root 26 tests, `macos.rs` 25 items / 11 tests; `python3 -I target/p176/motion.py <before> <root> <part>`
      (non-blank line multiset minus the part header, `use super::*;` and the declaration) → 0 only before, 0 only after.
- [x] Split `tests/guards/no_type_strings.rs` per §4.3.
      Done: same tools → root 2 tests, `self_checks.rs` 9 items / 6 tests; motion check → 0 / 0.
- [x] Fix the stale doc comments in `no_type_strings.rs`: `tests/architecture_guards.rs` →
      `tests/guards/architecture_guards.rs` (3 places) and `builtins_no_hand_picked_vreg` →
      `no_hand_picked_vreg`. Check: `grep -rn 'tests/architecture_guards\|builtins_no_hand_picked_vreg' tests/guards`
      → 0 hits.
      Done: → 0 (`grep -rn 'fn no_hand_picked_vreg' tests` → `tests/guards/architecture_guards.rs:176`).
- [x] Fix `.ai/codegen-invariants.md:94` `tests/no_operator_strings.rs` →
      `tests/guards/no_operator_strings.rs` (`grep -n 'tests/no_operator_strings' .ai/*.md` → 0).
      Done: → 0.
- [x] E4: tighten the two rows.
      Done: guard → `TIGHTEN test 742 (ceiling 1148)` app_special_keys, `TIGHTEN test 752 (ceiling 1179)` no_type_strings;
      `MFB_LOC_TIGHTEN=1` → own diff drops exactly those two rows; re-run → `0` rows.

Acceptance: both targets list and pass the same tests, and the guard's self-tests still fire.
  Check: E1 over `app_special_keys no_type_strings` → equal; E2 over both → 0 failed (est.
  4 min); E3; then
  `cargo test --test no_type_strings census_by_file -- --ignored --nocapture 2>&1 | grep -c 'test census_by_file ... ok'`
  → 1 (est. 1 min); `cargo test -p mfb --bin mfb spec_citations_resolve` → passes (the
  `[[tests/guards/no_type_strings.rs]]` citation still resolves; est. 3 min);
  `python3 -I tools/loc-census/census.py --file <f>` over the four files → second fields 742,
  409, 752, 430; E4; E5.
  Verified 2026-10-19: E1 (`bash target/p176/e1.sh`, leaf names vs E0's lists) → app_special_keys SAME 37,
  no_type_strings SAME 8; `cargo check --all-targets` warnings → 0 (E0 0); E2+E3
  `bash scripts/test-fast.sh --test app_special_keys --test no_type_strings --test test_targets_registered` →
  `test-fast: ok` (`37 passed`; `7 passed; 1 ignored`; `4 passed`); census_by_file → 1; spec_citations_resolve →
  `ok. 1 passed`; census → 742, 409, 752, 430; E4 → 0 rows after tightening; E5 `cargo fmt --all --check` → exit 0.
Commit: 7077d7902

### Phase 2 — tests/cli: repository and debugger

- [x] Split `tests/cli/cli_repo_install.rs` per §4.1.
      Done: `items.py split` → root 3 tests, `remove_update_and_resolve.rs` 7 tests; motion check → 0 / 0.
- [x] Split `tests/cli/cli_dbg.rs` per §4.1 (dissolve `mod native`; the cfg line goes on both
      declarations and on `use session_launch_and_list::*;`).
      Done: `items.py split` over the module body (L83-1697), dedenting 4 only where the indent is code (12 lines
      inside string literals kept byte for byte) → part A 32 items / 8 tests, part B 20 items / 12 tests;
      `motion.py --strip` → only the 3 dropped module lines (`mod native {`, `use super::*;`, `}`) and the 3 added cfg'd
      lines differ, besides the `pub(super)` prefixes. `pub(super)`: 13 items (see Corrections), 4 methods, 1 field.
- [x] Linux compile check of `cli_dbg`'s Linux variants on box 2223 (Kali aarch64 glibc, the
      only box where `all(target_os = "linux", target_arch = "aarch64")` selects the native
      debugger parts and the `#[cfg(target_os = "linux")] signed_mfb`; `.ai/remote_systems.md`
      lists `/usr/bin/cargo` there):
      `git archive HEAD | ssh -p 2223 test@127.0.0.1 'rm -rf ~/p176e && mkdir ~/p176e && tar -x -C ~/p176e'`,
      then `ssh -p 2223 test@127.0.0.1 'cd ~/p176e && cargo check --test cli_dbg 2>&1 | tail -3'`
      → `Finished` with no warning (est. 15 min).
      Done on a substitute (Corrections): every box is down (`nc -z 127.0.0.1 2223..2229` → refused) and UTM will not
      start one from this session (`utmctl list` → OSStatus -1743). The same cfg — aarch64 glibc Linux — ran in a
      `podman run docker.io/library/rust:1-bookworm` container on the local Podman machine (`uname -sm` → `Linux aarch64`,
      `ldd --version` → `GLIBC 2.36`) over the worktree mounted read-only: `cargo test --test cli_dbg --no-run` →
      `Finished`, no `warning`/`error` line; `-- --list` → 19 tests: 1 root, 7 `session_launch_and_list::`, 11
      `threads_watch_and_run::` (the macOS 21 minus the two `#[cfg(target_os = "macos")]` tests), so both native
      parts and the Linux `signed_mfb` compile there.
- [x] E4: tighten the two rows.
      Done: guard → `TIGHTEN test 68 (ceiling 1473)` cli_dbg, `TIGHTEN test 520 (ceiling 1241)` cli_repo_install;
      tighten drops exactly those two rows; re-run → 0 rows.

Acceptance: both targets list and pass the same tests.
  Check: E1 over `cli_repo_install cli_dbg`; E2 over both → 0 failed (est. 10 min; each repo
  test runs a live repository server, each dbg test a debugger session).
  `python3 -I tools/loc-census/census.py --file <f>` over the five files → second fields 520,
  724, 68, 604, 807. E3; E4; E5.
  Verified 2026-10-19: E1 → cli_repo_install SAME 10, cli_dbg SAME 21; warnings → 0; E2+E3
  `bash scripts/test-fast.sh --test cli_repo_install --test cli_dbg --test no_type_strings --test test_targets_registered`
  → `test-fast: ok` (`21 passed`, `10 passed`, `7 passed; 1 ignored`, `4 passed`). The first run of that call failed 2
  `cli_dbg` tests (a 30 s prompt timeout; a worker arena not yet listed) while the container build loaded every core;
  `cargo test --test cli_dbg` alone → 21 passed in 11 s, and the rerun above is clean. Census → 520, 724, 68, 605, 806
  (rustfmt reflowed two expressions after the dedent: +1 in part A, −1 in part B). E4 → 0 rows; E5 → exit 0.
Commit: 76fefe06d

### Phase 3 — tests/common (largest blast radius)

Every one of the 355 includers compiles `tests/common`. It lands last and alone.

- [x] Split `tests/common/mod.rs` per §4.4 into `mod.rs` + `process.rs`, with
      `#[allow(unused_imports)]` and its comment on `pub use process::*;`.
      Done: `items.py split` → `process.rs` 21 items (both `own_process_group` cfg variants; the `#[cfg]` attributes
      move with their items); motion check → 0 only before, only after = the 3 declaration/re-export lines.
- [x] Repoint `.ai/testing-gates.md:407` per §4.4
      (`grep -c 'tests/common/mod.rs.s pty helpers' .ai/testing-gates.md` → 0).
      Done: → 0.
- [x] ~~Windows compile check (the `#[cfg(not(unix))]` variants of `own_process_group` and
      `run_bounded`'s timeout path compile only there):
      `cargo check --target x86_64-pc-windows-msvc --test cli_build_config --test app_special_keys --test cli_dbg`
      → `Finished`, no warning (est. 5 min).~~ — moot: owner, 2026-10-19: "no test machines are up... just check
      compile and running on macOS." Evidence of what was tried: box 206 → `ssh … Operation timed out`; a local
      `--target x86_64-pc-windows-msvc` check needs a Windows C toolchain for `ring`/`libsqlite3-sys`. With stand-in
      compilers it reached `Finished`, and its only warnings were 4 unused imports in `cli_dbg.rs`'s root (`Child`,
      `mpsc`, `Duration`, `Read`/`Write`). Those imports fed the `cfg`'d `mod native` before the split too, so the
      warnings are not from E.
- [x] E4: tighten (`tests/common/mod.rs` → TIGHTEN, then removed).
      Done: guard → `TIGHTEN test 597 (ceiling 1228) tests/common/mod.rs` only; tighten removes it; re-run → 0 rows.

Acceptance: all 355 includers compile with no new warning, and both halves of `tests/common`
are exercised.
  Check: E1 over all 4 targets → equal to `/tmp/e_before.txt` line for line, plus the
  `--all-targets` warning count (which compiles all 355 includers); then
  `cargo test --test cli_build_config --test cli_repo_publish --test cli_canvas_package --test rt_tls_connect_allow_self_signed --test rt_native_io_runtime --test rt_json_bounds --test codegen_tls_c_return_x86_64`
  → 0 failed. These cover each helper group (`grep -rlw <helper> tests`): build/project
  (`cli_build_config`), repo (`cli_repo_publish`: `start_repo`), `fixture_truetype`
  (`cli_canvas_package`), `PortGate` (`rt_tls_connect_allow_self_signed`), pty + interposers +
  stdio (`rt_native_io_runtime`: `run_under_pty`, `build_close_interposer`,
  `build_short_write_interposer`, `run_with_readonly_stdout`), `run_bounded_with_rss`
  (`rt_json_bounds`), ncode checks (`codegen_tls_c_return_x86_64`). Est. 12 min.
  `python3 -I tools/loc-census/census.py --file <f>` → `tests/common/mod.rs` 597,
  `tests/common/process.rs` 635. E3; E4; E5.
  Verified 2026-10-19 (macOS): `cargo check --all-targets` (all 355 includers) → 0 warnings, 0 errors; E1 over all 4 →
  SAME 37, 8, 10, 21; `bash scripts/test-fast.sh --test cli_build_config --test cli_repo_publish --test cli_canvas_package
  --test rt_tls_connect_allow_self_signed --test rt_native_io_runtime --test rt_json_bounds --test codegen_tls_c_return_x86_64
  --test no_type_strings --test test_targets_registered` → `test-fast: ok` (9, 6, 7, 4, 15, 4, 1 passed); census → 597,
  635; E4 → 0 rows; E5 → exit 0.
  Final gate: budget rows → 0; `grep -cE 'tests/(cli|codegen|guards|common)/' tests/guards/file_size_ceilings.txt` → 1,
  the file's header comment line, no `ceiling` row; clippy: no pre-Phase-1 count was recorded, so the check is per file —
  `cargo clippy --all-targets 2>&1 | grep -E -- '--> tests/(cli/cli_dbg|cli/cli_repo_install|codegen/app_special_keys|guards/no_type_strings|common/(mod|process))(\.rs|/)'`
  → 0 (the tree-wide total, 37065, is pre-existing and names none of E's files).
Commit: b023b0a4d

## Validation Plan

- Tests: no test is added or removed. Per phase, each moved target's `--list` count equals its
  E0 count (E1) and the target passes (E2). Phase 3 re-lists all 4 targets and compiles every
  includer, because `tests/common` feeds all of them.
- Coverage check: `tests/` is outside the coverage denominator (`scripts/coverage-common.sh`
  `IGNORE` `(^|/)tests/`). Nothing covered changes.
- Runtime proof: Phase 1's `census_by_file` run and `scanners_fire_on_their_own_needles` show
  the split guard still bites. Phase 3's target list exercises each `tests/common` helper group
  from a real includer.
- Doc sync: `.ai/testing-gates.md` (407), `.ai/codegen-invariants.md` (94), and the
  `tests/guards/no_type_strings.rs` doc comments. No `mfb spec` or `mfb man` topic changes;
  the one spec citation into `no_type_strings.rs` names the file, which stays.
- Final gate (once, after Phase 3): `cargo test --test file_size_budget 2>&1 | grep -cE '^  (NEW|GREW|STALE|TIGHTEN)'`
  → 0, and `grep -cE 'tests/(cli|codegen|guards|common)/' tests/guards/file_size_ceilings.txt`
  → 0 (est. 2 min). `cargo clippy --all-targets 2>&1 | grep -c '^warning'` → equal to its value
  before Phase 1 (est. 4 min). The full workspace suite runs once, in plan-176-V.

## Decisions

- Rule revised — owner, 2026-10-18 (replaces the 2026-10-14 rule):

  | Kind | Applies to | Limit | Over the limit → |
  |---|---|---|---|
  | `code` | code LOC of a file that is not a test-only file | 1000 | split the file into modules |
  | `inline` | inline test LOC (`#[cfg(test)]` items / MFBASIC `TESTING … END TESTING`) inside a file that is not a test-only file | 250 | move the tests out into their own test-only file(s) |
  | `test` | total LOC of a test-only file (path component `tests`, `test_*.mfb`, `#![cfg(test)]`, or a `#[cfg(test)] mod x;` target and its subtree) | 1000 | split the test-only file into modules |

  Comments and blank lines do not count. There are no exemptions; generated files
  (`GENERATED` banner) are skipped automatically. Ceiling rows are
  `ceiling <code|inline|test> <n> <path>`. Parts target ≤ 900 LOC of their kind. The guard
  warns with a banner and never fails. A new module is always an acceptable split.
- Scope: `.rs`/`.mfb` under `src/` `packages/` `examples/` `repository/` `tests/` — owner,
  2026-10-14.
- Fewest parts that keep each file ≤ 900, cut on the planning map's responsibility seams —
  author, 2026-10-18.
- `tests/common/mod.rs`: the `pub use process::*;` line carries `#[allow(unused_imports)]` with
  the comment "each includer uses a subset of these helpers". The existing
  `#![allow(dead_code)]` stays as is — author, 2026-10-14. Measured: an unused glob re-export
  warns (`/tmp/loc/probe3`).
- Glob-cycle imports compile without warnings (`/tmp/loc/probe2`, `/tmp/loc_e_probe2`) — verified.
- Tests and helpers cited by archived bug docs stay where they are when a cut allows it
  (`cli_repo_install.rs:identical_vendor_blobs_keep_every_source_filename`,
  `tests/common/mod.rs:fixture_truetype`). Archived bug and plan documents are history and are
  not edited when a cited item moves (`tests/common/mod.rs::run_bounded_command` →
  `process.rs`) — author.
- Pre-existing stale references found while mapping are fixed in E (`.ai/codegen-invariants.md:94`;
  the `no_type_strings.rs` doc comments) — author, 2026-10-14.

## Corrections

- 2026-10-18 — Rule revised by the owner: test-only files limit 1000 (was 250), inline tests
  250, no exemptions. Dropped 31 files (each ≤ 1000 test LOC): `tests/cli/`
  `cli_app_canvas_mode`, `cli_app_term_inactive_gate`, `cli_build_config`, `cli_canvas_package`,
  `cli_entry_args`, `cli_linux_app_mode`, `cli_macos_app_term_draw_text`, `cli_repo_governance`,
  `cli_repo_publish`, `cli_win_app_term_fidelity`; `tests/codegen/`
  `codegen_helper_scratch_release`, `codegen_inplace_record_field`,
  `codegen_inplace_remove_at_insert`, `codegen_return_interior_temp_drop`,
  `codegen_string_return_freshness`; `tests/common/ncode_emu.rs`; `tests/guards/`
  `architecture_guards`, `inplace_self_update_census`, `no_operator_strings`; `tests/interop/`
  `rt_compress_interop`, `rt_crypto_aead_interop`, `rt_crypto_hpke_interop`,
  `rt_crypto_key_interop`; `tests/net/` `rt_http_header_framing_injection`,
  `rt_http_server_dos`, `rt_inline_trap_borrowed_resource`,
  `rt_macos_tls_write_after_clean_close`, `rt_tls_accept_handshake_budget`,
  `rt_tls_connect_allow_self_signed`, `rt_tls_listen_host_validation`,
  `rt_tls_listener_local_address`. Both exemptions are gone with no replacement:
  `repo_gc_reclaims_an_orphaned_blob_and_leaves_live_packages_installable` stays in the
  `cli_repo_install.rs` root and `repo_signs_package_and_embeds_executable_metadata` stays in
  the unsplit `cli_repo_publish.rs`. New files 90 → 7; for the five kept files 34 → 7
  (`cli_dbg` 7 → 2, `cli_repo_install` 6 → 1, `app_special_keys` 6 → 1, `no_type_strings`
  6 → 1, `tests/common/mod.rs` 9 → 1). Six phases → three. `cli_dbg.rs` re-measured at `ce7a995ca` (1473,
  22 tests after 4 commits since `17a9b5799`). The `ncode_emu` directory conversion and the
  `src/codegen/io/input/mod.rs` `#[path]` edit are no longer needed.
- 2026-10-19 (execution) — `/tmp` is wiped by session restarts here, so E0's output lives in
  `target/p176/e0/` (ignored by the repository) instead of `/tmp/e_before.txt`, and E0/E1 record
  the full sorted `--list` names, not counts: E1 compares leaf names (`<part>::name` → `name`) per
  target. `cli_dbg` lists 21 on this macOS host (the 22nd,
  `the_debugger_is_not_supported_off_macos_and_linux_aarch64`, is `cfg`'d out). Per the owner's
  2026-10-18 instruction, E2/E3 run through `bash scripts/test-fast.sh --test <t>...`; the full
  suite runs once before the plan merges.
- 2026-10-19 (execution) — `cli_dbg`'s `pub(super)` set is what the compiler asked for, not §4.1's
  list: 13 items (`signed_mfb` ×2, `Session`, `image`, `attach_allowed`, `TICKER`, `build_debug`,
  `is_address`, `debug_example`, `EXITED`, `body`, `REGISTER_COLUMNS`, `table_rows`), the 4 methods
  `start`/`until_prompt`/`send`/`finish`, and 1 field (`child`). §4.1's other four items
  (`executable_name`, `NOT_A_DEBUG_BUILD`, `line_count`, `SEPARATOR`) and two fields are used only
  inside part A, so they stay private (`cargo check --test cli_dbg` → no error, no warning).
- 2026-10-19 (execution) — Phase 2's box-2223 check ran in an aarch64 glibc Linux container
  instead (every box refused the connection and UTM cannot start one from this session). The
  check's point is the target's `cfg` (`linux` + `aarch64`), which the container has; it also
  links and lists, which `cargo check` on the box would not.
- 2026-10-19 — owner: "no test machines are up... just check compile and running on macOS."
  Phase 3's Windows compile check is moot under that instruction (the task says what was tried).

## Summary

E is pure motion of integration-test code into `#[path]` part files: 5 files, 4 targets,
7 new files, 0 tests renamed, no exemptions. The risk is in `tests/common`. It is compiled into
355 targets and carries `cfg(not(unix))` variants the host never builds, so it lands last with
a Windows compile check. The second risk is the cfg-gated `cli_dbg` native module, which gets
a box-2223 check. `no_type_strings.rs` is split with every path list moved verbatim and its
self-tests run. No compiler code changes, so nothing covered moves.
