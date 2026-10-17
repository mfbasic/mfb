# plan-176-E: CLI, codegen, net, interop, guard and shared-helper test files

Last updated: 2026-10-17
Effort: large (3h–1d)
Depends on: plan-176-D

E splits the 36 over-budget integration-test files under `tests/cli`, `tests/codegen`,
`tests/net`, `tests/interop`, `tests/guards` and `tests/common` into topic files, so that no
file is over 250 test LOC. Each `tests/<dir>/<name>.rs` stays the `[[test]]` target root and
declares its topics as `#[path = "<name>/<topic>.rs"] mod <topic>;`. The shared helper module
`tests/common/mod.rs` becomes a root with nine sibling parts. `tests/common/ncode_emu.rs`
becomes the directory module `tests/common/ncode_emu/mod.rs`. No test is added, removed or
renamed. Only the module path of a moved test changes (`name` → `<topic>::name`).

The single outcome: after E lands, `cargo test --test file_size_budget` shows no row for any of
E's files, and `tests/guards/file_size_ceilings.txt` has no `ceiling` row for any of them. Its
only E entries are the two `exempt` rows this letter adds:
`tests/cli/cli_repo_install/garbage_collection.rs` and
`tests/cli/cli_repo_publish/signing_and_metadata.rs`.

References:

- `planning/plan-176-A-file-size-guard.md`: the rule, §Decisions, the `#[path]` topic layout
  (§Verified properties), and "Splitting an over-budget file" in `.ai/build-tooling.md`.
- `.ai/testing-gates.md`: § "Splits must sweep man AND spec citations". Line 117 names
  `tests/cli/cli_linux_app_mode.rs:linux_app_mode_gtk_term_helpers_are_structurally_sound_on_aarch64`.
  Line 389 names `tests/common/mod.rs`'s pty helpers.
- `.ai/build-tooling.md`: rustfmt/clippy policy. Line 90 names `tests/common/mod.rs::build_project`.
- `.ai/codegen-invariants.md`: line 64 names `architecture_guards.rs`'s `code_above_tests`.
  Line 94 carries a stale path, `tests/no_operator_strings.rs`. Line 304 names
  `codegen_string_return_freshness.rs::being_used_as_a_callback_never_removes_the_return_copy`.
- `.ai/remote_systems.md`: box 2223, used for the Linux-gated compile check.
- Spec topics whose citations name E files: `src/docs/spec/memory/08_program-startup.md:315-316`
  (`architecture_guards.rs:shared_lowering_names_no_physical_register` and `:no_hand_picked_vreg`,
  both of which stay in the root), and `src/docs/spec/architecture/02_frontend.md:228`
  (`[[tests/guards/no_type_strings.rs]]`, file only, which stays valid).
- `scripts/coverage.sh:29` names `tests/common/mod.rs`'s `mfb_exe()`.
- `src/codegen/io/input/mod.rs:32` is the second includer of `tests/common/ncode_emu.rs` (`#[path]`).

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-176-D is complete and archived | `ls planning/plan-176-D-* planning/completed/plan-176-D-*` → only `planning/completed/…` | NOT MET (2026-10-14) |
| The file-size guard exists and passes | `cargo test --test file_size_budget` → passes | NOT MET (2026-10-14) |
| E's files have no uncommitted changes | `git status --porcelain -- tests/cli tests/codegen tests/net tests/interop tests/guards tests/common src/codegen/io/input/mod.rs .ai/testing-gates.md .ai/build-tooling.md .ai/codegen-invariants.md scripts/coverage.sh` → empty | MET (2026-10-17) |

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

- Every file in §2's table, and every file E creates, is ≤ 250 test LOC
  (`python3 -I tools/loc-census/census.py --file <path>` → second field ≤ 250). The exceptions
  are the two `exempt` files, which are 256 and 265.
- `cargo test --test file_size_budget` prints no row naming an E path, and the ceiling file
  holds no `ceiling` row for an E path.
- Every E target lists the same number of tests before and after each phase.

### Non-goals (explicit constraints)

- No behavior change. No test body is edited beyond moving it. The allowed edits are: adding
  `use super::*;` at the top of a part; `pub(super)` on support items (counts in §4);
  `pub(super) fn enter` in `ncode_emu`; the `#[allow(unused_imports)]` re-export lines; and the
  one `#[path]` repoint in `src/codegen/io/input/mod.rs`.
- No `[[test]]` target is renamed, added or removed. `Cargo.toml` is not edited. No `#[test]`
  fn is renamed.
- Every `common::<item>` path and every `use common::*;` glob in the 338 includers keeps
  resolving, with no edit to any includer. The same holds for every
  `common::ncode_emu::{Abi, Externals, Machine, data_cstr, find_calls, CallSite, Sym}` path.
- No guard weakens. The guard files are split, and every path list and allowlist they hold
  moves verbatim.

## 2. Current State

The counts come from the planning census (`python3 -I /tmp/loc/census.py "$PWD"`, working tree
of `17a9b5799`) and from `python3 -I /tmp/loc/span.py <file>`, re-run on 2026-10-17 at
`a3c95fc07` for the five files that changed since. After plan-176-A, the same numbers come from
`python3 -I tools/loc-census/census.py --file <path>`. The test counts come from
`grep -cE '^\s*#\[test' <file>`.

| File | Kind | Test LOC | Tests |
|---|---|---|---|
| `tests/cli/cli_app_canvas_mode.rs` | test file | 403 (394 at `17a9b5799`; `5b1998896` added 9 to `TERM_TRAPS_IN_CANVAS_SOURCE`) | 11 |
| `tests/cli/cli_app_term_inactive_gate.rs` | test file | 258 | 4 |
| `tests/cli/cli_build_config.rs` | test file | 254 | 9 |
| `tests/cli/cli_canvas_package.rs` | test file | 305 | 7 |
| `tests/cli/cli_dbg.rs` | test file | 1352 (1339 at `17a9b5799`; `5fbbff48f` added 13 to `an_attached_threaded_program_lists_its_threads_and_arenas`) | 20 |
| `tests/cli/cli_entry_args.rs` | test file | 279 | 8 |
| `tests/cli/cli_linux_app_mode.rs` | test file | 517 | 11 |
| `tests/cli/cli_macos_app_term_draw_text.rs` | test file | 358 | 3 |
| `tests/cli/cli_repo_governance.rs` | test file | 992 | 9 |
| `tests/cli/cli_repo_install.rs` | test file | 1241 | 10 |
| `tests/cli/cli_repo_publish.rs` | test file | 889 | 6 |
| `tests/cli/cli_win_app_term_fidelity.rs` | test file | 667 | 11 |
| `tests/codegen/app_special_keys.rs` | test file | 1148 | 37 |
| `tests/codegen/codegen_helper_scratch_release.rs` | test file | 427 | 12 |
| `tests/codegen/codegen_inplace_record_field.rs` | test file | 369 | 10 |
| `tests/codegen/codegen_inplace_remove_at_insert.rs` | test file | 385 | 12 |
| `tests/codegen/codegen_return_interior_temp_drop.rs` | test file | 298 | 5 |
| `tests/codegen/codegen_string_return_freshness.rs` | test file | 435 | 11 |
| `tests/common/mod.rs` | test file (helpers) | 1228 | 0 |
| `tests/common/ncode_emu.rs` | test file (helpers) | 612 | 0 |
| `tests/guards/architecture_guards.rs` | test file | 370 | 8 |
| `tests/guards/inplace_self_update_census.rs` | test file | 392 | 2 |
| `tests/guards/no_operator_strings.rs` | test file | 496 | 8 |
| `tests/guards/no_type_strings.rs` | test file | 1179 | 8 |
| `tests/interop/rt_compress_interop.rs` | test file | 705 | 6 |
| `tests/interop/rt_crypto_aead_interop.rs` | test file | 346 | 1 |
| `tests/interop/rt_crypto_hpke_interop.rs` | test file | 700 | 5 |
| `tests/interop/rt_crypto_key_interop.rs` | test file | 459 | 2 |
| `tests/net/rt_http_header_framing_injection.rs` | test file | 352 | 6 |
| `tests/net/rt_http_server_dos.rs` | test file | 266 | 7 |
| `tests/net/rt_inline_trap_borrowed_resource.rs` | test file | 308 | 5 |
| `tests/net/rt_macos_tls_write_after_clean_close.rs` | test file | 257 (251 at `17a9b5799`; `ff96e44be` added 6 to the test fn) | 1 |
| `tests/net/rt_tls_accept_handshake_budget.rs` | test file | 495 | 2 |
| `tests/net/rt_tls_connect_allow_self_signed.rs` | test file | 460 | 4 |
| `tests/net/rt_tls_listen_host_validation.rs` | test file | 370 | 3 |
| `tests/net/rt_tls_listener_local_address.rs` | test file | 275 | 2 |

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
  printed `warning: unused import: pty::*`. `/tmp/loc_e_probe` printed
  `warning: unused imports: Sym and data_cstr` for an explicit `pub use scan::{data_cstr, Sym};`.
  With `#[allow(unused_imports)]` on the line, the probe printed no warning. So every
  re-export line in `tests/common/mod.rs` and in `tests/common/ncode_emu/mod.rs` carries that
  attribute.
- **`#![allow(dead_code)]` in `tests/common/mod.rs` covers its parts.** Lint levels inherit
  into child modules. In `/tmp/loc_e_probe`, the unused `pub fn data_cstr`/`enum Sym` in a
  child file of `common` produced no `dead_code` warning.
- **A `#[path]`-loaded `…/ncode_emu/mod.rs` resolves its own children from a `src/` includer.**
  `/tmp/loc_e_probe`: `src/x/inner/mod.rs` declares
  `#[cfg(test)] #[allow(dead_code)] #[path = "../../../tests/common/emu/mod.rs"] mod emu;`.
  `emu/mod.rs` declares `mod scan;`, which resolved to `tests/common/emu/scan.rs`, and
  `cargo test --lib` printed `1 passed`.
- **`tests/common` needs no Cargo stanza.**
  `tests/guards/test_targets_registered.rs::NOT_TEST_TARGETS = ["common"]` skips the
  directory. `test_files_in_subdirectories` reads only the direct `.rs` children of
  `tests/<dir>/`, so the topic directories `tests/<dir>/<name>/` are never seen.
- **338 includers.** `grep -rl 'path = "../common/mod.rs"' tests | wc -l` → 338. The only other
  includer of a `tests/common` file is `src/codegen/io/input/mod.rs:32`
  (`grep -rn 'tests/common' src | grep path` → 1 hit).
- **No E file respawns itself, reads its own source, or uses `module_path!`/`file!()`.**
  `grep -nE 'current_exe|--exact|module_path!|file!\(\)'` over the E files → none. The one
  `--exact` user in the tree is `tests/net/rt_port_gate_is_cross_process.rs` (not E).
- **No coverage-exception and no guard allowlist names an E path.**
  `grep -n '^tests/' scripts/coverage-exceptions.txt` → none. The guards scan `src/` only.
- **cfg gates in E files** (`grep -hoE '#!?\[cfg\([^]]*\]'`):
  - `#[cfg(target_os = "macos")]`: 20 in `cli_app_canvas_mode.rs`, 8 in `cli_canvas_package.rs`.
  - `cli_dbg.rs`: one `#[cfg(any(target_os = "macos", all(target_os = "linux", target_arch = "aarch64")))]`
    on `mod native`, its `not(...)` twin, and `#[cfg(target_os = "linux")]` /
    `#[cfg(target_os = "macos")]` on the two `signed_mfb` variants.
  - `#[cfg(unix)]`: 3 in `cli_entry_args.rs`.
  - `tests/common/mod.rs`: `#[cfg(unix)]` and `#[cfg(not(unix))]` variants, plus
    `#[cfg(target_os = "macos")] build_short_write_interposer`.
  - A file-level `#![cfg(...)]` in seven `tests/net` roots. These stay in the roots, so the
    topics inherit them.

  Windows and Linux variants are therefore compile-checked off-host (§Phase 6, §Phase 4).
- **Pre-existing stale references found while mapping** (fixed in E):
  - `.ai/codegen-invariants.md:94` says `tests/no_operator_strings.rs`, and
    `ls tests/no_operator_strings.rs` → no such file. The file is
    `tests/guards/no_operator_strings.rs`.
  - `tests/guards/no_type_strings.rs` doc comments (lines 13, 53, 97) say
    `tests/architecture_guards.rs` (`grep -n 'tests/architecture_guards' tests/guards/*.rs` → 3
    hits). The file is `tests/guards/architecture_guards.rs`. Line 13 also names
    `builtins_no_hand_picked_vreg`, and `grep -rn 'fn builtins_no_hand_picked_vreg' tests src`
    → none. The guard is `no_hand_picked_vreg`.

## 3. Design Overview

The conventions are plan-176-A's integration-test layout, applied per file:

- The root `tests/<dir>/<name>.rs` keeps `#[path = "../common/mod.rs"] mod common;`, its `use`
  lines, any `#![cfg]`, and the helpers shared by more than one topic. Each child is declared
  `#[path = "<name>/<child>.rs"] mod <child>;` (2 LOC each, counted in the root's figure).
- Every **topic** file starts with `use super::*;` and holds whole `#[test]` fns plus the
  helpers only they use. Those helpers stay private, and no test fn changes visibility (libtest
  collects private `#[test]` fns in child modules).
- Every **support** file (shared helpers, no tests) starts with `use super::*;`. Its items
  become `pub(super)` (the per-file counts in §4 include struct fields and inherent methods
  used outside the file), and the root adds `use <support>::*;` (1 LOC).
- Where a root keeps tests, only the listed topics move: the smallest split that gets every
  file under 250.
- `tests/common/mod.rs` becomes a root with plain sibling parts (`mod <part>;`). A module
  loaded through `#[path]` owns its directory, so `mod build;` resolves to
  `tests/common/build.rs` for every includer. `tests/common/ncode_emu.rs` becomes
  `ncode_emu/mod.rs` for the same reason, so that both includers resolve its children
  identically.

The gate class is test motion: before == after `--list` counts per target, the moved tests
pass, no new warnings, then tighten. Byte-identity does not apply; no compiler code changes.

Risk concentrates in `tests/common`, which 338 targets compile. A wrong re-export or a missed
cfg variant breaks many of them at once, and the Windows (`#[cfg(not(unix))]`) variants are
invisible on the macOS host. It lands last, alone, and gets the Windows compile check. Second
is `cli_dbg.rs`: dissolving its cfg-gated `mod native` must keep the cfg on every declaration,
or the Linux-x86_64 and Windows builds try to compile macOS debugger code. It gets the box-2223
compile check.

Rejected alternatives:

- *`tests/common/<part>.rs` behind `#[path]` per includer.* This needs 338 includer edits. The
  plain `mod <part>;` inside the `#[path]`-loaded `mod.rs` needs none.
- *`tests/common/ncode_emu.rs` + `ncode_emu/` children.* Children of a `#[path]`-loaded
  `ncode_emu.rs` resolve differently for the `src/` includer (`tests/common/machine.rs`) and
  for `tests/common/mod.rs`'s `pub mod ncode_emu;` (`tests/common/ncode_emu/machine.rs`).
  The `mod.rs` form resolves identically for both.
- *Splitting `repo_gc_reclaims_an_orphaned_blob_and_leaves_live_packages_installable` and
  `repo_signs_package_and_embeds_executable_metadata` into helper fns.* That is logic
  restructuring, not motion: each is one `#[test]` fn with no nested items. They take their own
  files and `exempt` rows (owner decision).
- *Leaving `pub use <part>::*;` unannotated.* It warns in every includer that uses nothing
  from a part (probe3).

## 4. Detailed Design

The LOC figures are the summed `span.py` counts of the moved items plus the added lines
(`use super::*;`, declarations), as of `17a9b5799`, adjusted for the drift in §2 where noted.
Line ranges are labelled "as of 17a9b5799". Item and test NAMES are authoritative if lines
drift. Every topic and support file starts with `use super::*;`. "Surface" lists only what
differs from §3's defaults.

### 4.1 tests/cli

**`tests/cli/cli_app_canvas_mode.rs`** (403, 11 tests)
- Root (142): header; helpers `CANVAS_ROUNDTRIP_SOURCE`, `CANVAS_DISTINCT_SOURCE`, `build_app`,
  `run_headless`, `run_headless_with_stdin`. Keeps 4 tests:
  `canvas_mode_compiles_for_the_host_app_target`,
  `macos_canvas_mode_round_trips_through_the_presentation_slot`,
  `macos_canvas_is_distinct_from_console_and_none`, `linux_app_target_accepts_canvas_mode`.
  Declares `mode_gates`, `canvas_runtime`.
- `cli_app_canvas_mode/mode_gates.rs` (127, after `5b1998896`'s +9): 4 tests
  `macos_term_traps_wrong_mode_in_canvas`,
  `macos_io_reads_are_permitted_in_canvas_and_still_trap_in_none`,
  `macos_console_reads_are_unchanged_by_the_relaxation`,
  `macos_io_writes_degrade_to_stdout_in_canvas`. Local: `TERM_TRAPS_IN_CANVAS_SOURCE`,
  `IO_READ_GATE_SOURCE`, `IO_READ_CONSOLE_SOURCE`, `IO_WRITE_IN_CANVAS_SOURCE`.
- `cli_app_canvas_mode/canvas_runtime.rs` (140): 3 tests
  `macos_canvas_readraw_returns_bytes_in_order_then_eof`,
  `macos_group_members_trap_wrong_mode_outside_canvas`,
  `macos_set_group_past_the_table_limit_raises`. Local: `CANVAS_READ_ORDER_SOURCE`,
  `GROUPS_TRAP_OUTSIDE_CANVAS_SOURCE`, `GROUP_LIMIT_SOURCE`.
- Each test's `#[cfg(target_os = "macos")]` moves with it.
- Hazards: no citation
  (`grep -rnoE "tests/cli/cli_app_canvas_mode.rs(::|:)\w+" src/docs .ai scripts AGENTS.md .github` → none).
  `.github/workflows/coverage.yml:116` names the target only.

**`tests/cli/cli_app_term_inactive_gate.rs`** (258, 4 tests)
- Root (190): helpers `TERM_SOURCE`, `app_ncode_functions`, `body`, `active_slot`,
  `gate_position`, `WINDOWS_GATED`, `WINDOWS_GATED_LEAVES`, `mfb_error_unsupported`. Keeps
  `windows_app_mode_term_bodies_gate_on_the_shared_active_slot` and
  `windows_app_mode_terminal_size_raises_while_tui_mode_is_off`. Declares `linux_and_ungated`.
- `cli_app_term_inactive_gate/linux_and_ungated.rs` (71): tests
  `linux_app_mode_term_off_is_a_no_op_while_tui_mode_is_off` and
  `the_three_ungated_term_members_stay_ungated_on_every_app_backend`.
- Hazards: no citation.

**`tests/cli/cli_build_config.rs`** (254, 9 tests)
- Root (141): helpers `SOURCE`, `config_project`, `build`, `build_and_run`. Keeps
  `no_flag_selects_release`, `config_flag_selects_dev_in_both_forms`,
  `an_undeclared_config_fails_the_build`, `mfb_test_honors_config`. Declares `local_files`.
- `cli_build_config/local_files.rs` (116): tests
  `a_local_file_overrides_and_extends_a_config`, `a_local_only_config_is_selectable`,
  `a_bad_local_file_fails_the_build_with_its_path`, `a_local_type_change_fails_the_build`,
  `a_dependency_local_file_is_never_read`.
- Hazards: no citation.

**`tests/cli/cli_canvas_package.rs`** (305, 7 tests)
- Root (130): helpers `build`, `SURFACE_SOURCE`, `CONSOLE_IMPORT_SOURCE`. Keeps
  `canvas_surface_compiles_in_an_app_build`, `canvas_import_is_rejected_in_a_console_build`,
  `canvas_surface_compiles_for_the_linux_app_target`. Declares `macos_runtime`.
- `cli_canvas_package/macos_runtime.rs` (178): tests `macos_canvas_surface_runs`,
  `macos_repeated_and_changed_presents_are_sound`,
  `macos_present_layers_and_shape_switching_are_sound`, `macos_paint_zero_values_are_no_ops`.
  Local: `PAINT_DEFAULTS_SOURCE`, `run_headless`, `PRESENT_SKIP_SOURCE`, `PRESENT_LAYERS_SOURCE`.
- Hazards: no citation.

**`tests/cli/cli_entry_args.rs`** (279, 8 tests)
- Root (198): helpers `ARGS_SOURCE`, `build_ncode`, `function`, `field`. Keeps
  `linux_app_entry_reads_args_from_registers_not_the_stack`,
  `host_arg_accepting_program_receives_its_arguments`, `os_args_rejects_malformed_host_utf8`,
  `app_build_os_args_without_a_main_parameter`. Declares `argv_parking`.
- `cli_entry_args/argv_parking.rs` (84): tests
  `linux_x86_64_console_entry_parks_argv_before_clobbering_it`,
  `linux_x86_64_app_entry_parks_argv_before_clobbering_it`,
  `linux_aarch64_console_entry_parks_argv_before_clobbering_it`,
  `windows_console_entry_builds_and_parks_argv_for_main`. Local: `destination`,
  `assert_argv_survives_to_park`.
- Hazards: no citation. The `#[cfg(unix)]` attributes move with their items.

**`tests/cli/cli_linux_app_mode.rs`** (517, 11 tests)
- Root (169): helpers `TARGET`, `run_mfb`, `APP_SOURCE`, `TERM_DRAW_SOURCE`,
  `app_ncode_functions`. Keeps `linux_app_mode_nir_records_build_mode`,
  `linux_app_mode_plan_declares_gtk_libraries`,
  `linux_app_mode_carries_the_per_project_gtk_identity`,
  `linux_console_mode_still_emits_both_flavors`,
  `linux_app_mode_no_symbol_address_is_overwritten_by_the_offset_it_needs`. Declares
  `appimage`, `term_helpers`.
- `cli_linux_app_mode/appimage.rs` (160): tests `linux_app_mode_emits_a_single_sealed_appimage`,
  `linux_app_debug_keeps_the_appdir_beside_the_appimage`. Local: `squashfs_offset`.
- `cli_linux_app_mode/term_helpers.rs` (194): tests
  `linux_app_mode_positioned_term_members_reach_the_gtk_backend`,
  `linux_app_mode_draw_text_specializes_the_write_path_without_taking_its_cursor`,
  `linux_app_mode_without_term_emits_no_positioned_helpers`,
  `linux_app_mode_gtk_term_helpers_are_structurally_sound_on_aarch64`.
- Hazards: `.ai/testing-gates.md:117` cites
  `tests/cli/cli_linux_app_mode.rs:linux_app_mode_gtk_term_helpers_are_structurally_sound_on_aarch64`.
  It is repointed to `tests/cli/cli_linux_app_mode/term_helpers.rs:linux_app_mode_gtk_term_helpers_are_structurally_sound_on_aarch64`.

**`tests/cli/cli_macos_app_term_draw_text.rs`** (358, 3 tests)
- Root (193): helpers `DRAW_TEXT_SYMBOL`, `WRITE_STRING_SYMBOL`, `TERM_SOURCE`, `PLAIN_SOURCE`,
  `app_ncode_functions`. Keeps all 3 tests:
  `macos_app_draw_text_control_character_advances_one_column_without_stamping`,
  `macos_app_draw_text_printable_path_still_clips_and_advances_by_display_width`,
  `macos_app_write_string_keeps_its_own_walk`. Declares support `walk` and adds `use walk::*;`.
- `cli_macos_app_term_draw_text/walk.rs` (169), support: `op`, `field`,
  `is_conditional_branch`, `Walk`, `walk_of`, `is_store_off_stack`. `pub(super)` goes on 6
  items, 3 methods and 10 fields.
- Hazards: no citation.

**`tests/cli/cli_repo_governance.rs`** (992, 9 tests)
- Root (176): keeps `repo_transfer_recipient_publishes_and_consumers_verify_both_owners`. That
  test stays in the root because archived bug docs cite it as
  `cli_repo_governance.rs:repo_transfer_recipient_publishes_and_consumers_verify_both_owners`.
  Declares `machine_link_and_abi`, `ident_rotation_and_metadata_root`,
  `root_renewal_and_pre_rotation`, `ownership_and_release_state`.
- `cli_repo_governance/machine_link_and_abi.rs` (209): tests
  `repo_machine_link_makes_an_equal_and_revoke_cuts_it_off`,
  `repo_check_abi_reports_superset_and_breaking_changes`.
- `cli_repo_governance/ident_rotation_and_metadata_root.rs` (224): tests
  `repo_ident_rotation_follows_pins_and_reanchor_hard_errors`,
  `repo_signed_metadata_root_verifies_chain_and_gates_add`.
- `cli_repo_governance/root_renewal_and_pre_rotation.rs` (222): tests
  `repo_root_renewal_keeps_an_already_pinned_client_verifying`,
  `repo_registry_add_of_a_pre_rotation_release_installs`.
- `cli_repo_governance/ownership_and_release_state.rs` (173): tests
  `repo_ownership_transfer_is_two_sided_and_rebinds_the_package`,
  `repo_release_state_yank_excludes_floating_but_allows_pin`.
- Hazards: no citation. `Cargo.toml:97` and `tests/guards/test_targets_registered.rs:19` quote
  target names only (unchanged).

**`tests/cli/cli_repo_install.rs`** (1241, 10 tests)
- Root (139): keeps `repo_end_to_end_install_verifies_signed_package`. Declares
  `remove_cascade`, `update`, `drift_and_registry_ident`, `resolver`, `vendor_blobs`,
  `garbage_collection`.
- `cli_repo_install/remove_cascade.rs` (116): `remove_cascades_to_packages_that_import_the_target`.
- `cli_repo_install/update.rs` (186): `update_targeted_applies_the_abi_advisory_and_preserves_pin`,
  `update_of_a_pinned_dependency_requires_confirmation`.
- `cli_repo_install/drift_and_registry_ident.rs` (194):
  `install_warns_on_floor_drift_and_errors_on_pin_drift`,
  `spike_file_added_package_with_registry_ident_survives_update`.
- `cli_repo_install/resolver.rs` (231): `repo_resolver_selects_substitute_and_locks_deterministically`,
  `repo_resolver_reports_diamond_conflict_naming_both_requirers`.
- `cli_repo_install/vendor_blobs.rs` (137): `identical_vendor_blobs_keep_every_source_filename`.
- `cli_repo_install/garbage_collection.rs` (256, **exempt**):
  `repo_gc_reclaims_an_orphaned_blob_and_leaves_live_packages_installable`. This is one 255-LOC
  `#[test]` fn (L1207-1505 as of 17a9b5799) with no nested items: its body holds only
  statements and closures. Reason row: `exempt test tests/cli/cli_repo_install/garbage_collection.rs # one 255-LOC #[test] fn with no nested items; shrinking it means inventing helper fns (logic restructuring, not motion)`.
- Hazards: no citation.

**`tests/cli/cli_repo_publish.rs`** (889, 6 tests)
- Root (229): keeps `repo_publishes_signed_package_and_rejects_duplicate_version`,
  `repo_publish_without_a_path_publishes_the_current_directory`,
  `repo_publish_rejects_non_package_and_missing_session`. Declares `signing_and_metadata`,
  `registry_index`, `vendored_native_libraries`.
- `cli_repo_publish/signing_and_metadata.rs` (265, **exempt**):
  `repo_signs_package_and_embeds_executable_metadata`. This is one 264-LOC `#[test]` fn
  (L8-309 as of 17a9b5799) with no nested fn items. The `enum`-looking span at body L194-205
  is a statement, not an item. Reason row:
  `exempt test tests/cli/cli_repo_publish/signing_and_metadata.rs # one 264-LOC #[test] fn with no nested items; shrinking it means inventing helper fns (logic restructuring, not motion)`.
- `cli_repo_publish/registry_index.rs` (174): `repo_registry_add_installs_and_verifies_from_index`.
- `cli_repo_publish/vendored_native_libraries.rs` (230):
  `repo_vendored_native_libraries_publish_and_install_with_the_package` (one 229-LOC fn, under 250).
- Hazards: no citation. `Cargo.toml:97` and `test_targets_registered.rs:19` quote the target
  name only.

**`tests/cli/cli_win_app_term_fidelity.rs`** (667, 11 tests)
- Root (135): helpers `TERM_SOURCE`, `app_ncode_functions`, `functions_of`, `app_ncode_plan`,
  `app_ncode_plan_with`, `body`, `slot_of`, `fields_base`, `io_write_grid_body`,
  `RESIZE_SOURCE`, `resize_ncode_functions`, `addressed`, `size_symbols`. Keeps no test.
  Declares `glyphs_and_term_state`, `draw_text_clusters`, `live_size`, `resize_handler_and_abi`.
- `…/glyphs_and_term_state.rs` (112): `windows_app_mode_selects_line_and_fill_glyphs_from_the_shared_tables`,
  `windows_app_mode_still_stamps_the_selected_glyph_through_textoutw`,
  `windows_app_mode_term_on_resets_every_term_state_field`. Local: `glyph_immediates`.
- `…/draw_text_clusters.rs` (143): `windows_app_mode_draw_text_shares_the_io_write_cluster_walk`,
  `windows_app_mode_draw_text_drops_a_wide_cluster_at_the_edge_and_skips_controls`. Local:
  `label_index`, `untag`, `cluster_window`, `first_frame_offset`, `rebase_frame`.
- `…/live_size.rs` (118): `windows_app_mode_terminal_size_reads_the_live_surface_size`,
  `windows_app_mode_every_grid_bound_follows_the_live_size`,
  `windows_app_mode_did_resize_reads_the_windows_surface_record`.
- `…/resize_handler_and_abi.rs` (171): `windows_app_mode_window_and_scripted_resize_share_one_term_handler`,
  `a_normal_windows_app_build_carries_no_test_hook`,
  `windows_app_mode_reads_win32_results_from_the_c_return_register`. Local: `utf16_value`,
  `WIN64_VOLATILE_ARGS`, `reads_of_clobbered_args`.
- Hazards: no citation. `scripts/test-winapp.sh:492,577` name the root path only (it still exists).

**`tests/cli/cli_dbg.rs`** (1352, 20 tests)
- Today the native tests sit in
  `#[cfg(any(target_os = "macos", all(target_os = "linux", target_arch = "aarch64")))] mod native { use super::*; … }`
  (L81-1550 as of 17a9b5799). The inline module is dissolved. Each native topic and
  `session.rs` is declared at the root with that same `#[cfg(any(...))]` line above its
  `#[path]` (3 LOC per declaration). The root adds
  `#[cfg(any(...))] use session::*;`. Test paths change from `native::<name>` to
  `<topic>::<name>` (`grep -rn 'native::' scripts .github .ai src/docs` → none).
- Root (83): header, `dbg`. Keeps `help_and_usage_errors_on_every_host` and
  `the_debugger_is_not_supported_off_macos_and_linux_aarch64` (with its `#[cfg(not(any(...)))]`).
  Declares the 7 modules below. 83 = 60 (`span.py tests/cli/cli_dbg.rs 1 80`) + 7 × 3 + 2.
- `cli_dbg/session.rs` (195), support: `signed_mfb` (both `#[cfg(target_os)]` variants side by
  side), `Session`, `executable_name`, `image`, `attach_allowed`, `NOT_A_DEBUG_BUILD`,
  `TICKER`, `build_debug`, `is_address`, `line_count`, `debug_example`, `EXITED`, `body`,
  `REGISTER_COLUMNS`, `SEPARATOR`, `table_rows`. `pub(super)` goes on 17 items, 4 methods and
  3 fields.
- `cli_dbg/launch_and_attach.rs` (187): `a_launched_program_does_not_run_and_is_killed_on_quit`,
  `an_attached_program_pauses_and_runs_on_after_quit`,
  `programs_that_are_not_mfbasic_or_not_native_are_refused`,
  `an_unsigned_mfb_explains_how_to_sign_itself`. Local: `banner_pid`, `process_exists`, `host_image`.
- `cli_dbg/list.rs` (224): `list_shows_a_launched_debug_build_s_functions_and_registers`,
  `list_shows_an_attached_program_s_bound_imports`,
  `breakerror_stops_where_a_watched_error_is_raised`,
  `list_func_on_a_normal_build_shows_only_linked_functions`. Local: `function_rows`, `RAISES`,
  `FUNCTION_COLUMNS`.
- `cli_dbg/threads.rs` (193, after `5fbbff48f`'s +13): `a_launched_program_lists_its_one_thread_and_steps`,
  `an_attached_threaded_program_lists_its_threads_and_arenas`,
  `an_app_s_threads_are_the_app_and_mfb_main`. Local: `THREAD_COLUMNS`, `ARENA_COLUMNS`.
- `cli_dbg/show_and_watch.rs` (196): `show_type_reads_a_returned_string_from_the_arena`,
  `a_watchpoint_stops_when_a_thread_changes_the_watched_bytes`,
  `a_breakpoint_with_commands_runs_them_and_runs_on`,
  `show_error_names_the_fault_a_thread_stopped_on`.
- `cli_dbg/breakpoints_and_report.rs` (118): `a_tested_breakpoint_stops_once_and_a_jump_returns_early`,
  `show_report_prints_the_report_the_program_wrote_as_it_exited`.
- `cli_dbg/run_to_end.rs` (182): `run_stops_at_breakpoints_and_at_the_end`. Local:
  `EXIT_SYMBOL`, `exit_stop`, `exit_name`, `c_library`, `BREAKPOINT_COLUMNS`.
- Hazards: no citation. `scripts/sign-debugger.sh` is reached through
  `env!("CARGO_MANIFEST_DIR")` (absolute), so it is unaffected. `scripts/README.md:232` names
  the target only.

### 4.2 tests/codegen

**`tests/codegen/app_special_keys.rs`** (1148, 37 tests)
- Root (149): helpers `PROGRAM`, `CONSOLE_PROGRAM`, `A_PRESS`, `LEFT_PRESS`, `DELETE_PRESS`,
  `F5_PRESS`, `LEFT_RELEASE`, `SHIFT_LEFT_PRESS`, `PIPE`, `ncode_for`, `ncode_of`,
  `ncode_of_build`, `hex`, `assert_injects`, `key_down`. The three inline constant modules stay
  in the root: `mod appkit {…}` (L170-186), `mod gdk {…}` (L645-657) and `mod win32 {…}`
  (L935-960). Topics reach them as `appkit::X` through `use super::*`. The root keeps
  `use common::ncode_emu::{data_cstr, find_calls, CallSite, Externals, Machine, Sym};`, which
  `ncode_emu/mod.rs` re-exports (§4.6). Declares topics `macos`, `gtk`, `windows_main_input`,
  `windows_keys` and supports `mac_model`, `win_model`, with `use mac_model::*; use win_model::*;`.
- `app_special_keys/mac_model.rs` (196), support: `MacKey`, `MacObject`, `MacModel` with its
  `impl Externals`. `pub(super)` goes on 3 items, 10 methods and 10 fields.
- `app_special_keys/win_model.rs` (119), support: `WinModel`, `win_ncode`, `WNDPROC`,
  `EDITPROC`, `run_win`, `run_win_in_mode`, `win_each_proc_in_mode`, `win_each_proc`.
  `pub(super)` goes on 8 items, 1 method and 2 fields.
- `app_special_keys/macos.rs` (199), 11 tests:
  `macos_every_surface_registers_a_key_down_handler_that_the_model_drives`,
  `macos_a_mode_switching_program_registers_every_key_down_handler`,
  `macos_left_injects_kitty_left`, `macos_forward_delete_injects_kitty_delete`,
  `macos_f5_injects_kitty_f5`, `macos_left_release_injects_kitty_release`,
  `macos_shift_left_injects_kitty_shift_left`, `macos_auto_repeat_injects_a_repeat`,
  `macos_shift_a_injects_its_text`, `macos_ctrl_a_injects_the_key_without_text`,
  `macos_key_up_reports_nothing_in_line_mode_except_on_the_canvas`. Local: `MacSurface`,
  `mac_surfaces_in`, `class_name`, `mac_surfaces`, `run_mac`, `run_mac_in_mode`, `mac_press`.
- `app_special_keys/gtk.rs` (229), 9 tests: `gtk_key_pressed_handler_is_driven_by_the_model`,
  `gtk_left_injects_kitty_left`, `gtk_delete_injects_kitty_delete`, `gtk_f5_injects_kitty_f5`,
  `gtk_left_release_injects_kitty_release`, `gtk_shift_left_injects_kitty_shift_left`,
  `gtk_a_second_press_without_a_release_is_a_repeat`,
  `gtk_a_release_in_line_mode_is_not_reported_but_ends_the_hold`, `gtk_shift_a_injects_its_text`.
  Local: `gdk_keyval_to_unicode`, `gdk_keyval_to_lower`, `GTK_ST_PIPE_WRITE_FD`,
  `GTK_ST_INPUT_MODE`, `GTK_MODE_RAW`, `GtkModel` (with its `impl Externals`), `gtk_ncode`,
  `gtk_handlers`, `run_gtk`, `GtkEvent`, `run_gtk_events`, `gtk_press`.
- `app_special_keys/windows_main_input.rs` (104), 3 tests:
  `windows_main_posts_no_keystrokes_when_mfb_winapp_input_is_unset`,
  `windows_main_ignores_mfb_winapp_input_in_a_normal_build`,
  `windows_main_posts_mfb_winapp_input_then_enter`. Local: `WinMainModel` (with its
  `impl Externals`), `read_utf16z`, `win_main_posted_chars`.
- `app_special_keys/windows_keys.rs` (172), 14 tests: `windows_procs_are_driven_by_the_model`,
  `windows_left_injects_kitty_left`, `windows_delete_injects_kitty_delete`,
  `windows_f5_injects_kitty_f5`, `windows_left_release_injects_kitty_release`,
  `windows_shift_left_injects_kitty_shift_left`, `windows_auto_repeat_injects_a_repeat`,
  `windows_enter_is_injected_once`, `windows_raw_mode_text_key_is_a_key_event`,
  `windows_default_mode_text_key_is_its_text`,
  `windows_raw_mode_ctrl_chord_is_injected_once_at_key_down`,
  `windows_raw_mode_altgr_types_plain_text`, `windows_non_ascii_wm_char_is_written_as_utf8`,
  `windows_ascii_wm_char_is_written_verbatim`. Local: `VK_CONTROL`, `VK_MENU`.
- Hazards: no citation. Trait impls need no visibility change.

**`tests/codegen/codegen_helper_scratch_release.rs`** (427, 12 tests)
- Root (141): helpers `TARGET`, `Counts`, `cover_plan`, `copy_tree`, `helper_counts`,
  `guarded_scratch_releases`, `assert_package_for`. Keeps no test. Declares `audio_and_tls`,
  `fs_os_and_network`.
- `…/audio_and_tls.rs` (173): `every_core_audio_helper_frees_its_device_scratch`,
  `every_alsa_audio_helper_frees_its_device_scratch`,
  `every_wasapi_audio_helper_frees_its_device_scratch`,
  `every_openssl_tls_helper_releases_what_it_marshalled`,
  `every_network_framework_tls_helper_releases_what_it_marshalled`,
  `every_schannel_tls_helper_releases_what_it_marshalled`,
  `every_tls_marshalling_site_is_handed_a_scratch`. Local: `TARGET_MACOS`, `TARGET_WINDOWS`,
  `TLS_HELPERS_OPENSSL`, `TLS_HELPERS_NETWORK_FRAMEWORK`, `TLS_HELPERS_SCHANNEL`,
  `AUDIO_HELPERS_MACOS`, `AUDIO_HELPERS_ALSA`, `AUDIO_HELPERS_WASAPI`, `walk`.
- `…/fs_os_and_network.rs` (119): `every_fs_helper_releases_the_path_it_marshalled`,
  `every_os_helper_releases_the_name_it_marshalled`,
  `every_net_helper_releases_the_host_it_marshalled`,
  `every_tcp_helper_releases_the_host_it_marshalled`,
  `every_udp_helper_releases_the_host_it_marshalled`. Local: `assert_package`, `FS_HELPERS`,
  `OS_HELPERS`, `NET_HELPERS`, `TCP_HELPERS`, `UDP_HELPERS`.
- Hazards: no citation. Paths are read through `env!("CARGO_MANIFEST_DIR")`. The helper lists
  name runtime symbols, not file paths, so no other letter's split repoints them.

**`tests/codegen/codegen_inplace_record_field.rs`** (369, 10 tests)
- Root (216): helpers `TARGET`, `label_count`, `stack_slot_count`, `function`, `ncode`,
  `BOX_TYPE`. Keeps `remove_key_on_a_record_field_mutates_in_place`,
  `a_second_updated_field_is_stored_beside_the_arm`,
  `a_field_followed_by_an_inlined_sibling_shrinks_in_place_and_declines_a_grow`,
  `add_on_a_record_field_grows_the_record_in_place`,
  `add_on_a_plain_local_does_not_use_the_record_grow`. Declares `collection_records`.
- `…/collection_records.rs` (156): `set_on_a_fixed_width_record_list_writes_in_place`,
  `set_on_a_variable_width_record_list_grows_the_record`, `set_on_a_record_map_grows_the_record`,
  `insert_and_prepend_on_a_record_list_grow_the_record`,
  `prepend_on_a_plain_local_does_not_use_the_record_grow`.

**`tests/codegen/codegen_inplace_remove_at_insert.rs`** (385, 12 tests)
- Root (205): helpers `TARGET`, `label_count`, `has_slot`, `function`, `ncode`. Keeps
  `remove_at_on_a_plain_local_mutates_in_place`,
  `remove_at_under_a_live_for_each_runs_in_place_on_a_loop_copy`,
  `insert_on_a_plain_local_mutates_in_place`,
  `insert_under_a_live_for_each_runs_in_place_on_a_loop_copy`,
  `set_remove_on_a_plain_local_mutates_in_place`,
  `set_remove_under_a_live_for_each_runs_in_place_on_a_loop_copy`. Declares `bounds_and_splices`.
- `…/bounds_and_splices.rs` (183): `insert_in_place_still_bounds_checks`,
  `remove_at_in_place_still_bounds_checks`,
  `remove_at_takes_the_in_place_path_for_a_recursive_element_type`,
  `remove_at_still_fires_for_a_non_recursive_union_element`,
  `a_fixed_width_splice_emits_no_identity_entry_loop`,
  `a_variable_width_splice_still_shifts_its_entry_table`.

**`tests/codegen/codegen_return_interior_temp_drop.rs`** (298, 5 tests)
- Root (177): helpers `TARGET`, `ncode`, `function`, `slots`, `pending_temps`, `escaping_parks`,
  `frees`, `string_frees`, `concats`, `escape_guards`, `callee`. Keeps
  `a_nested_concat_return_frees_its_interior_block_and_a_flat_one_frees_nothing`,
  `the_return_shapes_with_nothing_interior_gain_no_free`. Declares `collections_cleanup_and_fail`.
- `…/collections_cleanup_and_fail.rs` (124): `a_returned_collection_frees_the_strings_that_built_it`,
  `the_cleanup_bearing_return_path_frees_its_interior_block_too`,
  `a_fail_is_never_given_an_interior_free`.

**`tests/codegen/codegen_string_return_freshness.rs`** (435, 11 tests)
- Root (217): helpers `TARGET`, `ncode`, `function`, `pending_temps`, `string_drops`,
  `main_calling`, `BORROW_CALLEE`, `FRESH_CALLEE`, `flat_copies`, `callee_reached_as_callback`,
  `callee_reached_directly`, `IDENTITY_CALLEE`, `CONCAT_CALLEE`. Keeps
  `a_fresh_string_callee_result_gets_exactly_one_owner_at_the_call_site`,
  `a_param_borrow_string_callee_result_is_never_given_an_owner`,
  `the_tostring_identity_adds_no_owner`,
  `being_used_as_a_callback_never_removes_the_return_copy`,
  `a_callback_whose_result_is_already_fresh_is_not_copied_twice`. Declares `callback_ownership`.
- `…/callback_ownership.rs` (221): `transform_owns_the_string_block_its_callback_returns`,
  `sort_by_owns_the_string_key_its_callback_returns`,
  `group_by_owns_the_string_value_its_callback_returns`,
  `map_values_owns_the_string_value_its_callback_returns`,
  `an_indirect_callback_result_is_owned_regardless_of_a_shadowing_name`,
  `a_callback_that_returns_its_own_parameter_is_freed_exactly_once`. Local: `loop_item_frees`,
  `mfb_hof`, `MAP_VALUES_STRING`, `MAP_VALUES_STRING_SHADOWING`.
- Hazards: `.ai/codegen-invariants.md:304` cites
  `being_used_as_a_callback_never_removes_the_return_copy`, which stays in the root (no edit).

### 4.3 tests/guards

These four guards scan `src/`. Their `"src/…"` path lists and allowlists move verbatim, and
§4.3 records where each one lands so later letters repoint the right file.

**`tests/guards/architecture_guards.rs`** (370, 8 tests)
- Root (215): helpers `is_physical_scan_exempt`, `code_above_tests`, `rs_files`,
  `is_emission_context`, `count_hand_picked_vregs`, `emission_lines`. Keeps
  `shared_lowering_names_no_physical_register`, `no_hand_picked_vreg`,
  `physical_registers_stay_in_the_backends`, `no_constructed_vreg_name`. Declares
  `seams_goldens_and_panic_hook`.
- `architecture_guards/seams_goldens_and_panic_hook.rs` (158): `no_cross_member_os_seam_dispatcher`,
  `only_syntax_goldens_may_pin_a_compiler_diagnostic`, `no_golden_pins_a_fatal_signal`,
  `only_testutil_installs_a_panic_hook`. Local: `golden_build_logs`, `is_compiler_diagnostic`,
  `CRASH_EXITS`, `PANIC_HOOK_OWNER` (`= "testutil.rs"`, unchanged).
- Path lists: L84 (`shared_lowering_names_no_physical_register`), L252-258
  (`physical_registers_stay_in_the_backends`) and L291 (`no_constructed_vreg_name`) stay in
  the root. L324 (`no_cross_member_os_seam_dispatcher`) moves to `seams_goldens_and_panic_hook.rs`.
- Citations: `src/docs/spec/memory/08_program-startup.md:315-316` cite the two root tests (no
  edit). `.ai/codegen-invariants.md:64` and `.ai/testing-gates.md:103` cite `code_above_tests`
  (root, no edit).

**`tests/guards/inplace_self_update_census.rs`** (392, 2 tests)
- Root (121): helpers `man`, `packages`, `record_types`, `field_kinds_tsv`. Keeps
  `every_package_record_type_has_a_field_kind_line`. Declares topic `self_update_overloads` and
  support `signatures` (`use signatures::*;`).
- `…/signatures.rs` (173), support: `functions`, `signatures`, `Ty`, `parse_ty`, `resolve`,
  `occurs`, `coincide`, `first_param_and_return`, `self_update_shaped`. `pub(super)` goes on 9 items.
- `…/self_update_overloads.rs` (105): `every_self_update_shaped_overload_has_a_runtime_case`.
  Local: `TIER_B_SENTENCE`, `tier_b_signatures`, `cases_tsv_signatures`, `LITERAL_FIRST_ARGUMENT`.
- Path list: `tier_b_signatures` reads `src/codegen/builtins/strings/mod.rs` (`TIER_B_TRANSFORMS`)
  at run time. The path string now lives in `self_update_overloads.rs`. The comment at L346
  citing the same path moves with it.

**`tests/guards/no_operator_strings.rs`** (496, 8 tests)
- Root (138): helpers `SPELLINGS`, `is_excluded_from_scan`, `is_test_file`,
  `is_vocabulary_file`, `OPERATOR_SHAPED_NON_OPERATORS`, `is_operator_shaped_non_operator`,
  `test_free_lines`, `rs_files`. Keeps `no_operator_strings`. Declares topics
  `scanner_self_tests`, `vocabulary_and_exemptions` and support `scanners` (`use scanners::*;`).
- `…/scanners.rs` (172), support: `literal_at`, `is_spelling`, `is_comment`,
  `spelling_match_arms`, `spelling_compares`, `node_string_operator`, `str_operator_param`,
  `scan_tree`. `pub(super)` goes on 8 items.
- `…/scanner_self_tests.rs` (49): `scanners_fire_on_their_own_needles`,
  `the_line_filter_strips_cfg_test_items_not_the_file_tail`.
- `…/vocabulary_and_exemptions.rs` (147): `the_vocabulary_file_is_exactly_one`,
  `vocabulary_matches_the_enum`, `operator_shaped_exemptions_are_closed`,
  `every_exemption_is_still_load_bearing`, `exempt_files_are_still_scanned_for_decisions`.
- Path lists: `is_excluded_from_scan`, `is_vocabulary_file` and `OPERATOR_SHAPED_NON_OPERATORS`
  (L74-142) stay in the root. The per-file exemption assertions (L543-629) move to
  `vocabulary_and_exemptions.rs`.
- Stale reference fixed here: `.ai/codegen-invariants.md:94` `tests/no_operator_strings.rs` →
  `tests/guards/no_operator_strings.rs`.

**`tests/guards/no_type_strings.rs`** (1179, 8 tests)
- Root (205): helpers `is_excluded_from_scan`, `is_test_file`, `test_free_lines`, `rs_files`,
  `BUDGETS`, `bucket`, `CLASSES`, `scan_tree`, `print_budget_table`. Keeps `no_type_strings`.
  Declares topics `scanner_self_tests`, `census`, `closed_lists` and supports `allowlists`,
  `site_scanners`, `pattern_scanners` (three `use <support>::*;` lines).
- `…/allowlists.rs` (172), support: `BOUNDARY_FILES`, `is_grammar_file`, `is_boundary_file`,
  `TYPE_PARAM_NAMES`, `SPELLINGS_MATCH`, `SPELLINGS_COMPARE`, `GRAMMAR_METHODS`,
  `GRAMMAR_TOKENS`, `CONSTRUCTED_PREFIXES`, `TYPE_KEYED_TABLES`. `pub(super)` goes on 10 items.
- `…/site_scanners.rs` (117), support: `is_word_byte`, `skip_spaces`, `word_start_matches`,
  `quoted_name_end`, `Hit`, `hit`, `code_lines`, `parse_sites`, `declared_sites`,
  `str_type_params`, `mask_is_named`. `pub(super)` goes on 11 items and 2 fields.
- `…/pattern_scanners.rs` (173), support: `spelling_match_arms`, `spelling_compares`,
  `hand_rolled_grammar`, `format_type_construction`, `string_keyed_type_maps`,
  `C_CARRIER_NAMES`, `C_SPELLINGS`, `c_type_strings`. `pub(super)` goes on 8 items.
- `…/scanner_self_tests.rs` (239): `scanners_fire_on_their_own_needles`. This is one 238-LOC fn,
  under 250.
- `…/census.rs` (102): `census_by_file`. Still reached by
  `cargo test --test no_type_strings census_by_file -- --ignored --nocapture`
  (`.ai/codegen-invariants.md:628`); the substring filter matches `census::census_by_file`.
- `…/closed_lists.rs` (192): `immediate_operand_class_vocabulary_is_closed`,
  `curated_type_keyed_tables_all_exist`, `the_grammar_file_is_exactly_one`,
  `boundary_list_is_closed`, `boundary_files_exist_and_are_justified`. Local:
  `quoted_literals_after`, `immediate_class_argument`, `as_literal`.
- Path lists: `BOUNDARY_FILES`/`is_grammar_file` (L171-231) and `TYPE_KEYED_TABLES` (L362-488)
  move to `allowlists.rs`. `is_excluded_from_scan` (L78) and `BUDGETS`/`bucket` (L888-1013)
  stay in the root. `c_type_strings` (L837) moves to `pattern_scanners.rs`. The needle paths
  (L1191-1395) move to `scanner_self_tests.rs`, and L1718-1763 to `closed_lists.rs`.
- Citations: `src/docs/spec/architecture/02_frontend.md:228` cites the file only (stays valid).
- Stale references fixed here: in the doc comments, `tests/architecture_guards.rs` (lines 13,
  53, 97) → `tests/guards/architecture_guards.rs`, and `builtins_no_hand_picked_vreg` (line 13)
  → `no_hand_picked_vreg`.

### 4.4 tests/interop

**`tests/interop/rt_compress_interop.rs`** (705, 6 tests)
- Root (169): helpers `Lcg`, `reference_crc32`, `ERR_INVALID_FORMAT`, `RAW`, `ZLIB`, `GZIP`,
  `DECODE_SOURCE`, `decode_with_mfb`, `ok`, `refused`, `encode`, `decode_corpus`. Keeps no
  test. Declares `crc32`, `decode_verdicts`, `decided_behaviours`, `encoders`.
- `…/crc32.rs` (123): `crc32_matches_crc32fast_over_a_generated_corpus`. Local:
  `EXPECTED_CRC32_CASES`, `ERR_INVALID_ARGUMENT`, `SOURCE`.
- `…/decode_verdicts.rs` (79): `flate2_streams_decode_at_every_level_and_format`,
  `tampered_checksummed_streams_get_flate2s_verdict`, `dec57_dec58_regressions_are_refused`.
  Local: `EXPECTED_ENCODE_CASES`, `EXPECTED_TAMPER_CASES`, `EXPECTED_REGRESSION_CASES`.
- `…/decided_behaviours.rs` (112): `decided_behaviours_hold`. Local: `EXPECTED_DECIDED_CASES`,
  `LENIENT`, `gzip_with_fhcrc`.
- `…/encoders.rs` (234): `encoders_output_decodes_with_flate2_and_is_deterministic`. Local:
  `EXPECTED_ENCODED_CASES`, `ENCODE_SOURCE`, `de_bruijn`, `encode_payloads`, `read_job`.
  None of these has another user.

**`tests/interop/rt_crypto_aead_interop.rs`** (346, 1 test)
- Root (189): helpers `EXPECTED_SEAL_CASES`, `EXPECTED_OPEN_OK`, `EXPECTED_OPEN_FAIL`,
  `ERR_AUTHENTICATION_FAILED`. Keeps `aead_boxes_interoperate_with_ring_both_ways`. Declares
  support `harness` (`use harness::*;`).
- `…/harness.rs` (161), support: `SOURCE`, `unhex`, `hex`, `field`, `ring_key`, `ring_seal`,
  `ring_open`, `random`. `pub(super)` goes on 8 items.

**`tests/interop/rt_crypto_hpke_interop.rs`** (700, 5 tests)
- Root (177): helpers `Profile`, `PROFILES`, `ED25519_SEED`, `ED25519_PUB`, `ED448_SEED`,
  `ED448_PUB`, `recipient_keys`. Keeps
  `hpke_boxes_interoperate_with_independent_implementation_both_ways`. Declares topic
  `reference_vectors`, supports `keccak`, `hpke_reference` (two `use` lines), and
  `#[path = "rt_crypto_hpke_interop/x448.rs"] mod x448;`.
- `…/keccak.rs` (70), support: `keccak_f1600`, `shake256`. `pub(super)` goes on 2 items.
- `…/hpke_reference.rs` (164), support: `hex_decode`, `hex_encode`, `Len`, `hmac_alg`,
  `hkdf_alg`, `labeled_extract`, `labeled_expand`, `kem_suite_id`, `hpke_suite_id`, `x25519`,
  `dh`, `base_point`, `extract_and_expand`, `key_schedule`, `aead_key`, `hpke_seal`,
  `hpke_open`. `pub(super)` goes on 17 items.
- `…/reference_vectors.rs` (134): `shake256_rust_side_reproduces_known_answers`,
  `x448_rust_side_reproduces_rfc7748_vectors`, `hpke_rust_side_reproduces_rfc9180_a1`,
  `hpke_rust_side_reproduces_rfc9180_a6`.
- `…/x448.rs` (166): the body of the former inline `mod x448 { … }` (L88-273, 168 LOC including
  its `mod`/`}` lines), moved verbatim. Its items keep their visibility, because callers
  already reach them as `x448::…`.

**`tests/interop/rt_crypto_key_interop.rs`** (459, 2 tests)
- Root (208): helpers `SOURCE`, `unhex`, `hex`, `Mfb`, `parse_gen`, `parse_one`,
  `parse_verify`, `x25519`, `MESSAGE`. Keeps no test. Declares `ed25519_x25519`, `nist_ecdsa`.
- `…/ed25519_x25519.rs` (140): `ed25519_x25519_and_convert_interoperate_both_ways`. Local:
  `ed25519_seed_to_x25519_secret`, `x25519_base`.
- `…/nist_ecdsa.rs` (117): `nist_ecdsa_interoperates_both_ways`.

None of the interop files has a citation or an `include_str!`.

### 4.5 tests/net

Each root keeps its file-level `#![cfg(...)]`. None of these files has a citation
(`grep -rnoE "tests/net/<file>(::|:)\w+" src/docs .ai scripts AGENTS.md .github` → none) or an
`include_str!`.

**`tests/net/rt_http_header_framing_injection.rs`** (352, 6 tests)
- Root (225): helpers `SERVER`, `Server`, `nonce`, `spawn_server`, `send_raw`, `text`,
  `status_line`, `assert_well_framed`. Keeps `well_formed_request_reply_is_byte_identical`,
  `crlf_in_a_reflected_header_value_does_not_split_the_response`,
  `control_byte_in_a_header_name_or_reason_does_not_split_the_response`. Declares
  `smuggling_and_client`.
- `…/smuggling_and_client.rs` (130): `server_refuses_the_request_smuggling_primitives`,
  `server_truncates_the_body_to_content_length`,
  `client_method_with_crlf_is_rejected_before_connecting`.

**`tests/net/rt_http_server_dos.rs`** (266, 7 tests)
- Root (177): helpers `SERVER`, `Server`, `nonce`, `spawn_server`, `read_reply`, `send_raw`,
  `status_line`, `assert_still_serving`. Keeps
  `malformed_chunk_size_is_answered_400_and_the_server_survives` and
  `idle_connection_is_dropped_on_the_read_deadline_and_the_next_client_is_served`. Declares
  `header_caps`.
- `…/header_caps.rs` (92): `oversized_head_is_rejected_with_431_before_the_terminator_arrives`,
  `too_many_header_fields_is_rejected_with_431`, `overlong_header_line_is_rejected_with_431`,
  `request_under_the_caps_is_still_served`, `tiny_chunk_body_is_framed_in_linear_time`.

**`tests/net/rt_inline_trap_borrowed_resource.rs`** (308, 5 tests)
- Root (162): helpers `FD_LIMIT`, `run`, `assert_clean_run`. Keeps
  `a_trapped_tcp_and_udp_poll_leaves_the_list_element_open`,
  `a_trapped_collections_get_neither_tombstones_nor_closes_the_element`,
  `mixed_an_owned_success_with_an_aliasing_recover_closes_only_what_it_opened`. Declares
  `borrowed_success_and_tls`.
- `…/borrowed_success_and_tls.rs` (149):
  `mixed_a_borrowed_success_with_an_owned_recover_closes_only_what_it_opened`,
  `a_trapped_tls_poll_leaves_the_list_element_open`. Local: `have_openssl`, `write_cert`.

**`tests/net/rt_macos_tls_write_after_clean_close.rs`** (257, 1 test)
- Root (161, after `ff96e44be`'s +6): helpers `PAYLOAD`, `DEADLINE`. Keeps
  `macos_tls_write_succeeds_live_and_raises_connection_closed_after_close_notify`. Declares
  support `server` (`use server::*;`).
- `…/server.rs` (100), support: `have_openssl`, `write_cert`, `build_server`, `fail`.
  `pub(super)` goes on 4 items.

**`tests/net/rt_tls_accept_handshake_budget.rs`** (495, 2 tests)
- Root (79): helpers `PAYLOAD`, `ERR_TIMEOUT`, `DEADLINE`, `have_tools`, `write_cert`, `kill`,
  `fail`, `outcome`. Keeps no test. Declares `late_connection`, `whole_handshake`.
- `…/late_connection.rs` (206): `tls_accept_gives_a_late_connection_a_full_handshake_budget`.
  Local: `ACCEPT_MS`, `CONNECT_AT_MS`, `HELLO_AT_MS`, `ZERO_BOUND_MS`, `CLIENT`, `build_server`.
- `…/whole_handshake.rs` (216): `tls_accept_bounds_the_whole_handshake`. Local:
  `TRICKLE_CLIENT`, `CONTROL_ACCEPT_MS`, `TRICKLE_ACCEPT_MS`, `TRICKLE_STEP_MS`,
  `TRICKLE_BOUND_MS`, `build_trickle_server`, `run_trickle_client`.

**`tests/net/rt_tls_connect_allow_self_signed.rs`** (460, 4 tests)
- Root (80): helpers `DEADLINE`, `EXPECT_NAME`. Keeps `accepts_a_self_signed_peer`,
  `still_rejects_a_name_mismatch`, `still_rejects_an_expired_certificate`,
  `defaults_to_rejecting_a_self_signed_peer`. Declares supports `certs`, `endpoints` (two `use` lines).
- `…/certs.rs` (173), support: `have_openssl`, `run_openssl`, `write_cert`,
  `write_expired_cert`, `assert_expired_cert_shape`. `pub(super)` goes on 5 items.
- `…/endpoints.rs` (215), support: `nonce`, `Peer`, `free_port`, `start_peer`,
  `served_common_name`, `build_client`, `run_client`, `outcome`. `pub(super)` goes on 8 items
  and 3 methods.

**`tests/net/rt_tls_listen_host_validation.rs`** (370, 3 tests)
- Root (178): helpers `write_cert`, `build_server`, `TLS_PEEK_CLIENT`, `tls_peek`,
  `requested_port`, `have_openssl`. Keeps `localhost_binds_loopback_on_the_exact_requested_port`.
  Declares `invalid_hosts_and_leaks`.
- `…/invalid_hosts_and_leaks.rs` (195): `malformed_or_ipv6_hosts_raise_address_invalid_promptly`,
  `resolve_path_leaks_nothing_beyond_the_numeric_path`. Local: `host_listen_loop_arena_totals`.

**`tests/net/rt_tls_listener_local_address.rs`** (275, 2 tests)
- Root (205): helpers `PAYLOAD`, `DEADLINE`, `ACCEPT_MS`, `nonce`, `have_openssl`, `write_cert`,
  `build_project`, `read_bound_line`. Keeps
  `tls_listen_binds_every_interface_when_the_host_is_empty`. Declares `local_address`.
- `…/local_address.rs` (73): `tls_local_address_reports_the_port_a_listener_bound_to`.

### 4.6 tests/common

**`tests/common/mod.rs`** (1228, 0 tests). This is not a target (`NOT_TEST_TARGETS = ["common"]`).
- `tests/common/mod.rs` (41): `#![allow(dead_code)]` (pre-existing, kept as is; lint levels
  inherit into the parts); `pub mod canvas_image; pub mod debug_report; pub mod ncode_emu;`;
  the std and `mfb_repository` `use` lines, all of which stay here, including the
  repo_acceptance imports at L1214-1219 (`use mfb_repository::{crypto, store::Store}`,
  `std::io::{BufRead, BufReader}`, `Child`, `Once`); and, for each part, `mod <part>;` +
  `#[allow(unused_imports)] // each of the 338 includers uses a subset of these helpers` +
  `pub use <part>::*;`. The parts are `project`, `build`, `process`, `interposers`, `stdio`,
  `pty`, `ncode_checks`, `repo`, `fixtures`. LOC: 32 for the root body and nine `mod` +
  `pub use` pairs, plus 9 attribute lines = 41. Each part starts with `use super::*;`. The
  std imports come from `mod.rs`.
- `tests/common/project.rs` (83): `unique_nonce`, `mfb_path_literal`, `temp_project`,
  `exit_description`, `BUILD_RELEASE_MFB`, `mfb_exe`.
- `tests/common/build.rs` (192): `build_project`, `build_project_opt`, `built_executable`,
  `APP_BUILD_FLAG`, `build_app`, `build_app_debug`, `build_app_with`, `app_binary`,
  `host_libc_flavor`, `build_ncode`, `build_ncode_opt`, `build_ncode_app`,
  `build_ncode_app_with`, `build_linux_elf`.
- `tests/common/process.rs` (210): `run_bounded`, `run_bounded_without_inherited_fds`,
  `own_process_group` (both cfg variants), `kill_process_tree`, `run_bounded_command`,
  `run_capture_with_env`, `run_with_stdin`, `run_with_env`, `run_bounded_with_rss`.
- `tests/common/interposers.rs` (156): `build_close_interposer`, `build_short_write_interposer`.
- `tests/common/stdio.rs` (86): `hex`, `python_exe`, `decode_hex`, `run_with_readonly_stdout`.
- `tests/common/pty.rs` (186): `run_under_pty`, `run_under_pty_bytes`,
  `run_pty_prompt_interaction`, `run_pty_prompt_interaction_echo_off`,
  `run_pty_prompt_interaction_inner`.
- `tests/common/ncode_checks.rs` (145): `assert_no_aligned_bank_result_reads`,
  `assert_no_inplace_sxtw_of_aligned_bank`, `is_stack_base`, `is_stack_register`,
  `outside_inline_error_exits`.
- `tests/common/repo.rs` (100): `BUILD_REPO`, `RepoProcess`, `repo_exe`, `start_repo`,
  `open_store`, `mfb_repo_home`, `run_mfb`, `run_mfb_plain`, `run_mfb_in`.
- `tests/common/fixtures.rs` (65): `fixture_truetype`, `PortGate` (with its impls).
- Surface: no visibility change. The private items (`built_executable`, `build_app_with`,
  `run_bounded_command`, `BUILD_REPO`, `BUILD_RELEASE_MFB`) land in the same part as all their
  callers. Every `#[cfg(unix)]`, `#[cfg(not(unix))]` and `#[cfg(target_os = "macos")]`
  attribute moves with its item.
- Doc refs repointed: `.ai/build-tooling.md:90` `tests/common/mod.rs::build_project` →
  `tests/common/build.rs::build_project`. `.ai/testing-gates.md:389` "`tests/common/mod.rs`'s
  pty helpers" → `tests/common/pty.rs`'s. `scripts/coverage.sh:29` "`tests/common/mod.rs`'s
  `mfb_exe()`" → `tests/common/project.rs`'s.

**`tests/common/ncode_emu.rs`** (612, 0 tests) → directory module. Spans were measured with
`span.py tests/common/ncode_emu.rs <a> <b>`.
- `tests/common/ncode_emu/mod.rs` (137): L1-198 verbatim (module docs, `use`s, `Abi` +
  `impl Abi`, `Externals`, `Function`, the 7 address/limit consts, `struct Machine`, `field`,
  `parse_int`, `decode_hex`, `data_object_bytes`: 131 LOC), plus `mod machine; mod step;
  mod scan;` and `#[allow(unused_imports)] // re-exported for the includers; most use a subset`
  + `pub use scan::{data_cstr, find_calls, CallSite, Sym};`.
- `tests/common/ncode_emu/machine.rs` (171): `use super::*;` + `impl<'a> Machine<'a> { … }`
  holding L200-396 verbatim (`new`, `has_function`, `address_of`, `symbol_at`, `reg`,
  `set_reg`, `arg`, `read_u8`, `write_u8`, `read_uint`, `read_u64`, `write_uint`,
  `write_bytes`, `read_bytes`, `read_cstr`, `halt`, `halted`, `alloc`, `call`).
- `tests/common/ncode_emu/step.rs` (235): `use super::*;` + `impl<'a> Machine<'a> { … }`
  holding `fn enter` (L398-649) verbatim. It is the largest single fn, under 250.
- `tests/common/ncode_emu/scan.rs` (80): `use super::*;` + L651-750 verbatim (`enum Sym`,
  `struct CallSite`, `find_calls`, `data_cstr`).
- Surface: `fn enter` → `pub(super) fn enter` (it is called by `call` in `machine.rs`). That is
  the only change. Private items of `mod.rs` are visible to its children. The callers'
  surface `ncode_emu::{Abi, Externals, Machine, data_cstr, find_calls, CallSite, Sym}` is
  unchanged (`grep -rn ncode_emu tests src`: `tests/codegen/app_special_keys.rs`,
  `tests/codegen/codegen_canvas_vulkan_off_frame_path.rs`,
  `src/codegen/io/input/{tests,encode_tests,reader_tests}.rs`).
- Second includer: `src/codegen/io/input/mod.rs:32`
  `#[path = "../../../../tests/common/ncode_emu.rs"]` → `#[path = "../../../../tests/common/ncode_emu/mod.rs"]`.
  This adds no LOC to that file. `tests/common/mod.rs`'s `pub mod ncode_emu;` needs no edit.

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
  `for t in <the 34 targets of §2>; do printf '%s ' $t; cargo test --test $t -- --list 2>/dev/null | grep -c ': test$'; done > /tmp/e_before.txt`
  and `cargo check --all-targets 2>&1 | grep -c '^warning' > /tmp/e_warn_before.txt`
  (est. 10 min cold).
- **E1:** the same loop restricted to `$T`, compared against the matching lines of
  `/tmp/e_before.txt` → identical. Then `cargo check --all-targets 2>&1 | grep -c '^warning'`
  → equal to `/tmp/e_warn_before.txt`.
- **E2:** `cargo test` with one `--test <t>` for each target in `$T` → 0 failed.
- **E3:** `cargo test --test architecture_guards --test no_type_strings --test no_operator_strings --test test_targets_registered`
  → all pass (est. 2 min).
- **E4:** `cargo test --test file_size_budget 2>&1 | grep -E '^  (NEW|GREW)'` → no output.
  Every `TIGHTEN`/`STALE` row names a file of this phase. Then
  `MFB_LOC_TIGHTEN=1 cargo test --test file_size_budget`, rerun →
  `grep -cE '^  (NEW|GREW|STALE|TIGHTEN)'` → `0`. Commit `tests/guards/file_size_ceilings.txt`
  with the phase (est. 2 min).
- **E5:** `cargo fmt --check` → no output (est. 20 s).

### Phase 1 — tests/net and tests/interop

Twelve self-contained runtime-interop targets. Nothing imports them.

- [ ] Split `tests/net/rt_http_header_framing_injection.rs` per §4.5.
- [ ] Split `tests/net/rt_http_server_dos.rs` per §4.5.
- [ ] Split `tests/net/rt_inline_trap_borrowed_resource.rs` per §4.5.
- [ ] Split `tests/net/rt_macos_tls_write_after_clean_close.rs` per §4.5.
- [ ] Split `tests/net/rt_tls_accept_handshake_budget.rs` per §4.5.
- [ ] Split `tests/net/rt_tls_connect_allow_self_signed.rs` per §4.5.
- [ ] Split `tests/net/rt_tls_listen_host_validation.rs` per §4.5.
- [ ] Split `tests/net/rt_tls_listener_local_address.rs` per §4.5.
- [ ] Split `tests/interop/rt_compress_interop.rs` per §4.4.
- [ ] Split `tests/interop/rt_crypto_aead_interop.rs` per §4.4.
- [ ] Split `tests/interop/rt_crypto_hpke_interop.rs` per §4.4 (inline `mod x448` → `x448.rs`).
- [ ] Split `tests/interop/rt_crypto_key_interop.rs` per §4.4.
- [ ] E4: tighten the twelve rows.

Acceptance: the twelve targets list and pass the same tests.
  Check: E1 → equal; E2 over the 12 targets → 0 failed (est. 8 min; the TLS and HTTP tests
  spawn real peers); E3; E4; E5.
Commit: —

### Phase 2 — tests/codegen

- [ ] Split `tests/codegen/codegen_helper_scratch_release.rs` per §4.2.
- [ ] Split `tests/codegen/codegen_inplace_record_field.rs` per §4.2.
- [ ] Split `tests/codegen/codegen_inplace_remove_at_insert.rs` per §4.2.
- [ ] Split `tests/codegen/codegen_return_interior_temp_drop.rs` per §4.2.
- [ ] Split `tests/codegen/codegen_string_return_freshness.rs` per §4.2.
- [ ] Split `tests/codegen/app_special_keys.rs` per §4.2 (the `appkit`/`gdk`/`win32` modules stay in the root).
- [ ] E4: tighten the six rows.

Acceptance: the six targets list and pass the same tests.
  Check: E1; E2 over the 6 targets → 0 failed (est. 6 min); E3; E4; E5.
Commit: —

### Phase 3 — tests/cli (single-host topics)

- [ ] Split `tests/cli/cli_app_canvas_mode.rs` per §4.1.
- [ ] Split `tests/cli/cli_app_term_inactive_gate.rs` per §4.1.
- [ ] Split `tests/cli/cli_build_config.rs` per §4.1.
- [ ] Split `tests/cli/cli_canvas_package.rs` per §4.1.
- [ ] Split `tests/cli/cli_entry_args.rs` per §4.1.
- [ ] Split `tests/cli/cli_linux_app_mode.rs` per §4.1.
- [ ] Split `tests/cli/cli_macos_app_term_draw_text.rs` per §4.1.
- [ ] Split `tests/cli/cli_win_app_term_fidelity.rs` per §4.1.
- [ ] `.ai/testing-gates.md:117`: repoint the citation to
      `tests/cli/cli_linux_app_mode/term_helpers.rs:linux_app_mode_gtk_term_helpers_are_structurally_sound_on_aarch64`
      (`grep -c 'cli_linux_app_mode.rs:linux_app_mode_gtk' .ai/testing-gates.md` → 0 after).
- [ ] E4: tighten the eight rows.

Acceptance: the eight targets list and pass the same tests.
  Check: E1; E2 over the 8 targets → 0 failed (est. 10 min; they build real app executables);
  E3; E4; E5.
Commit: —

### Phase 4 — tests/cli: repository and debugger

- [ ] Split `tests/cli/cli_repo_governance.rs` per §4.1.
- [ ] Split `tests/cli/cli_repo_install.rs` per §4.1, and in the same commit add
      `exempt test tests/cli/cli_repo_install/garbage_collection.rs # one 255-LOC #[test] fn with no nested items; shrinking it means inventing helper fns (logic restructuring, not motion)`
      to `tests/guards/file_size_ceilings.txt` and the same path and reason to the `EXEMPT` table in
      `tools/loc-census/census.py`.
- [ ] Split `tests/cli/cli_repo_publish.rs` per §4.1, and in the same commit add the
      `signing_and_metadata.rs` exempt row (§4.1 reason) to both the ceiling file and the
      census `EXEMPT` table.
- [ ] Split `tests/cli/cli_dbg.rs` per §4.1 (dissolve `mod native`; the cfg line goes on every
      declaration and on `use session::*;`).
- [ ] Linux compile check of `cli_dbg`'s Linux variants on box 2223 (Kali aarch64 glibc, the
      only box where `all(target_os = "linux", target_arch = "aarch64")` selects the native
      debugger modules and the `#[cfg(target_os = "linux")] signed_mfb`; `.ai/remote_systems.md`
      prefers it for Linux cargo rows):
      `git archive HEAD | ssh -p 2223 test@127.0.0.1 'rm -rf ~/p176e && mkdir ~/p176e && tar -x -C ~/p176e'`,
      then `ssh -p 2223 test@127.0.0.1 'cd ~/p176e && cargo check --test cli_dbg 2>&1 | tail -3'`
      → `Finished` with no warning (est. 15 min).
- [ ] E4: tighten the four rows. The two exempt rows stay and print nothing.

Acceptance: the four targets list and pass the same tests, and the exempt files are the only E
files over 250.
  Check: E1; E2 over `cli_repo_governance cli_repo_install cli_repo_publish cli_dbg` → 0 failed
  (est. 15 min; each repo test runs a live repository server). `python3 -I tools/loc-census/census.py --file tests/cli/cli_repo_install/garbage_collection.rs`
  → `0 256`, and `… --file tests/cli/cli_repo_publish/signing_and_metadata.rs` → `0 265`.
  E3; E4; E5.
Commit: —

### Phase 5 — tests/guards

The guards police `src/`. A wrong move would silently narrow a scan, so the tests that prove
each scanner still fires (`scanners_fire_on_their_own_needles` in both string guards) run here.

- [ ] Split `tests/guards/architecture_guards.rs` per §4.3.
- [ ] Split `tests/guards/inplace_self_update_census.rs` per §4.3.
- [ ] Split `tests/guards/no_operator_strings.rs` per §4.3.
- [ ] Split `tests/guards/no_type_strings.rs` per §4.3.
- [ ] Fix the stale doc comments in `no_type_strings.rs`: `tests/architecture_guards.rs` →
      `tests/guards/architecture_guards.rs` (3 places) and `builtins_no_hand_picked_vreg` →
      `no_hand_picked_vreg`. Check: `grep -rn 'tests/architecture_guards\|builtins_no_hand_picked_vreg' tests/guards`
      → 0 hits.
- [ ] Fix `.ai/codegen-invariants.md:94` `tests/no_operator_strings.rs` →
      `tests/guards/no_operator_strings.rs` (`grep -n 'tests/no_operator_strings' .ai/*.md` → 0).
- [ ] E4: tighten the four rows.

Acceptance: the four guards run the same tests, still pass, and their self-tests still fire.
  Check: E1; E2 = E3 plus `--test inplace_self_update_census` → 0 failed; then
  `cargo test --test no_type_strings census_by_file -- --ignored --nocapture 2>&1 | grep -c 'test census::census_by_file ... ok'`
  → 1 (est. 3 min); `cargo test -p mfb --bin mfb spec_citations_resolve` → passes (the two
  `architecture_guards.rs` citations still resolve; est. 3 min); E4; E5.
Commit: —

### Phase 6 — tests/common (largest blast radius)

Every one of the 338 targets compiles `tests/common`. It lands last and alone.

- [ ] Split `tests/common/mod.rs` per §4.6 into the nine parts, with
      `#[allow(unused_imports)]` and its comment on each `pub use <part>::*;`.
- [ ] Convert `tests/common/ncode_emu.rs` into `tests/common/ncode_emu/{mod,machine,step,scan}.rs`
      per §4.6 (`pub(super) fn enter`; the re-export line carries the allow).
- [ ] `src/codegen/io/input/mod.rs:32`: `#[path]` → `"../../../../tests/common/ncode_emu/mod.rs"`.
- [ ] Repoint `.ai/build-tooling.md:90`, `.ai/testing-gates.md:389` and `scripts/coverage.sh:29`
      per §4.6 (`grep -rn 'tests/common/mod.rs' .ai scripts` → 0 hits).
- [ ] Windows compile check (the `#[cfg(not(unix))]` variants of `own_process_group` and
      `run_bounded`'s timeout path compile only there):
      `cargo check --target x86_64-pc-windows-msvc --test cli_build_config --test app_special_keys --test cli_dbg`
      → `Finished`, no warning (est. 5 min).
- [ ] E4: tighten (`tests/common/mod.rs` → TIGHTEN, `tests/common/ncode_emu.rs` → STALE).

Acceptance: all 338 includers and the `src/` includer compile with no new warning, and every
part is exercised.
  Check: E1 over all 34 targets → equal to `/tmp/e_before.txt` line for line, plus the
  `--all-targets` warning count; then
  `cargo test --test cli_build_config --test cli_repo_publish --test cli_canvas_package --test rt_tls_connect_allow_self_signed --test rt_native_io_runtime --test rt_json_bounds --test codegen_tls_c_return_x86_64 --test app_special_keys --test codegen_canvas_vulkan_off_frame_path`
  → 0 failed. These cover each part: build/project, repo, fixtures, `PortGate`, pty +
  interposers + stdio, `run_bounded_with_rss`, ncode_checks, and ncode_emu
  (`grep -rlw <helper> tests`). Est. 12 min. Then
  `cargo test -p mfb --bin mfb codegen::io::input` → 0 failed (the `src/` includer; est. 4 min).
  E3; E4; E5.
Commit: —

## Validation Plan

- Tests: no test is added or removed. Per phase, each moved target's `--list` count equals its
  E0 count (E1) and the target passes (E2). Phase 6 re-lists all 34 targets, because
  `tests/common` feeds all of them.
- Coverage check: `tests/` is outside the coverage denominator (`scripts/coverage-common.sh`
  `IGNORE` `(^|/)tests/`), and `src/codegen/io/input/mod.rs` changes only a `#[path]` string
  under `#[cfg(test)]`. Nothing covered changes.
- Runtime proof: Phase 5's `census_by_file` run and the guards' own
  `scanners_fire_on_their_own_needles` tests show the split guards still bite. Phase 6's
  per-part target list exercises each `tests/common` part from a real includer.
- Doc sync: `.ai/testing-gates.md` (117, 389), `.ai/build-tooling.md` (90),
  `.ai/codegen-invariants.md` (94), `scripts/coverage.sh` (29), and the
  `tests/guards/no_type_strings.rs` doc comments. No `mfb spec` or `mfb man` topic changes;
  the two spec citations into `architecture_guards.rs` keep their file and symbol.
- Final gate (once, after Phase 6): `cargo test --test file_size_budget 2>&1 | grep -cE '^  (NEW|GREW|STALE|TIGHTEN)'`
  → 0, and `grep -E 'tests/(cli|codegen|net|interop|guards|common)/' tests/guards/file_size_ceilings.txt`
  → exactly the two `exempt` rows (est. 2 min). `cargo clippy --all-targets 2>&1 | grep -c '^warning'`
  → equal to its value before Phase 1 (est. 4 min). The full workspace suite runs once, in
  plan-176-V.

## Decisions

- Rule: code ≤ 1000, test ≤ 250 non-comment non-blank LOC. Scope: `.rs`/`.mfb` under `src/`
  `packages/` `examples/` `repository/` `tests/` — owner, 2026-10-14.
- Inline test code counts against the 250 test budget; over 250 it moves to its own module,
  split further as needed — owner, 2026-10-14.
- Generated files are exempt. The guard warns with a banner and never fails. A new module is
  always an acceptable split — owner, 2026-10-14.
- A single test-side unit over 250 gets its own file and an `exempt` row (with reason) in both
  `tests/guards/file_size_ceilings.txt` and `tools/loc-census/census.py`'s `EXEMPT` table, in
  the same commit — owner, 2026-10-14. In E those units are
  `tests/cli/cli_repo_install/garbage_collection.rs` (256) and
  `tests/cli/cli_repo_publish/signing_and_metadata.rs` (265), each one `#[test]` fn with no
  nested items.
- `tests/common/mod.rs`: each `pub use <part>::*;` line carries `#[allow(unused_imports)]` with
  the comment "each of the 338 includers uses a subset of these helpers". The existing
  `#![allow(dead_code)]` stays as is — author, 2026-10-14. Measured: an unused glob re-export
  warns (`/tmp/loc/probe3`). The same applies to `ncode_emu/mod.rs`'s explicit re-export
  (`/tmp/loc_e_probe`).
- Glob-cycle imports compile without warnings (`/tmp/loc/probe2`, `/tmp/loc_e_probe2`) — verified.
- `ncode_emu` becomes `ncode_emu/mod.rs`, not `ncode_emu.rs` + `ncode_emu/`, so both includers
  resolve its children the same way — author.
- Tests cited by archived bug docs stay in their root where the map kept them
  (`cli_repo_governance`, `rt_tls_listener_local_address`). Archived bug and plan documents are
  history and are not edited when a cited test moves (`vendor_blobs`, `callback_ownership`,
  `whole_handshake`, `fixtures`, `process`) — author.
- Pre-existing stale references found while mapping are fixed in E (`.ai/codegen-invariants.md:94`;
  the `no_type_strings.rs` doc comments) — author, 2026-10-14.

## Corrections

## Summary

E is pure motion of integration-test code into `#[path]` topic files: 36 files, 34 targets, 0
tests renamed, two exemptions. The risk is in `tests/common`. It is compiled into 338 targets
and carries `cfg(not(unix))` variants the host never builds, so it lands last with a Windows
compile check. The second risk is the cfg-gated `cli_dbg` native module, which gets a box-2223
check. The guards are split with every path list moved verbatim and their self-tests run. No
compiler code changes, so nothing covered moves.
