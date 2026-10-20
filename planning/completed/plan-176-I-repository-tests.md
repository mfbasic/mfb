# plan-176-I: Repository crate inline tests

Last updated: 2026-10-18
Effort: medium (1h–2h)
Depends on: plan-176-H

I moves the inline `#[cfg(test)] mod tests` of the 12 files of the `mfb_repository` workspace
member (`repository/`) whose inline test LOC is over the 250 `inline` limit into test-only
files under `tests/` directory modules. Nine blocks are ≤ 1000 and each becomes ONE
`<host>/tests/mod.rs`; the three over 1000 (`client.rs`, `store.rs`, `server.rs`) become a
`tests/mod.rs` holding the shared harness plus parts grouped by responsibility, each ≤ 900. No
test is added, removed or renamed; in the three split hosts the module path of each moved test
gains a part segment (`server::tests::foo` → `server::tests::accounts::foo`), in the other nine
it is unchanged. Production code does not move; the code-side splits of `client.rs`,
`server.rs` and `store.rs` are letter L.

The single outcome: after I lands, `cargo test --test file_size_budget` shows no row for any of
I's 12 files or the files I creates, and `tests/guards/file_size_ceilings.txt` has no `ceiling
inline` row for the 12 hosts and no `ceiling test` row for any file I creates. (The `ceiling
code` rows of `client.rs`, `server.rs` and `store.rs` are untouched by I and stay at `n == c`, so
they print no row; L removes them.)

References:

- `planning/plan-176-A-file-size-guard.md` — the rule, §Decisions, and "Splitting an
  over-budget file" in `.ai/build-tooling.md` (inline tests: `foo/tests/mod.rs`, split into
  parts ≤ 900 only when the moved block is over 1000).
- `.ai/testing-gates.md` — names `local::tests`' `ENV_LOCK` by module (still true after I) and
  § "`src/**` coverage comes only from the `--bins` unit tests" (`repository/` is the opposite:
  a library whose integration tests count).
- `.ai/net-tls.md` — repository-client transport security; cites production fns and test names
  only (unchanged by I).
- `.ai/build-tooling.md` — rustfmt/clippy policy; `repository/Cargo.toml` denies
  `clippy::items_after_test_module`.
- Spec topics: none move. All 175 distinct `[[repository/src/…]]` citations resolve to symbols
  outside the moved ranges (I map, measured per file).
- `planning/plan-176-L-…` — owns the code split of `store.rs` and replaces the
  `include_str!` that I repoints (§4.11).

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-176-H is complete and archived | `ls planning/plan-176-H-* planning/completed/plan-176-H-*` → only `planning/completed/…` | MET (2026-10-19) |
| The file-size guard exists and passes | `cargo test --test file_size_budget` → passes | MET (2026-10-19) |
| I's files have no uncommitted changes | `git status --porcelain -- repository/src` → empty | MET (2026-10-19) |

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

- Every file in §2's table is ≤ 250 inline test LOC, and every test-only file I creates is
  ≤ 1000 LOC (≤ 900 for the parts of the three split hosts)
  (`python3 -I tools/loc-census/census.py --file <path>` → second field within those limits).
- `cargo test --test file_size_budget` prints no row naming an I path, and the ceiling file has
  no `ceiling inline` row for an I host and no `ceiling test` row for a file I creates.
- The repository library and binary list the same number of tests before and after each phase.

### Non-goals (explicit constraints)

- No behavior change. No test body edit beyond moving it (dedented), a leading `use super::*;`
  in each part file of the three split hosts, hoisting `client.rs`'s mid-module `use` lines to
  the top of its `tests/mod.rs`, and repointing one `include_str!` path (§4.11). No visibility
  edit anywhere: every shared helper stays in its host's `tests/mod.rs`, whose private items
  its child parts see.
- No `#[test]` / `#[tokio::test]` fn renamed, added or removed.
- No production code moves. Every host keeps its non-test items, and its cfg(test) accessors in
  production position (`abi.rs::vendor_ref_for_hash`, `package.rs::test_support`, `store.rs`'s
  five `*_for_test` items).
- Cross-module test helpers keep their paths: `crate::store::tests::{register_keys, legacy_transfer, drop_tables}`,
  `crate::local::tests::env_guard`, `crate::backfill::tests::package_bytes`,
  `crate::package::test_support::*`, `crate::abi::vendor_ref_for_hash`.
- `#[cfg(test)] mod tests;` stays at each host's old position (end of file):
  `repository/Cargo.toml` denies `clippy::items_after_test_module`.

## 2. Current State

LOC: `python3 -I /tmp/loc/census.py "$PWD"` at HEAD `ce7a995ca` (2026-10-18; output
`/tmp/loc/census.json`, field `test` = the file's inline test LOC); after plan-176-A,
`python3 -I tools/loc-census/census.py --file <path>`. The `mod tests` block LOC and its range:
`python3 -I /tmp/loc/rw-I/m.py <files>` (census line classifier, brace-matched from the
`#[cfg(test)]` line; the moved file is the block minus its `#[cfg(test)]`, `mod tests {` and `}`
lines). `git log --oneline 17a9b5799..ce7a995ca -- repository/src` → empty, so the I map's
`17a9b5799` line numbers and names still hold. Tests: `grep -cE '#\[(tokio::)?test\]' <file>`.

| File | Inline test LOC | of which in `mod tests` | Tests | `mod tests` at | Moved file(s) |
|---|---|---|---|---|---|
| `repository/src/abi.rs` | 776 | 764 | 31 | 498-1533 | 1 (761) |
| `repository/src/backfill.rs` | 546 | 546 | 9 | 211-869 | 1 (543) |
| `repository/src/blobstore.rs` | 319 | 319 | 21 | 551-953 | 1 (316) |
| `repository/src/client.rs` | 2838 | 2838 | 75 | 1980-5602 | 4 (mod + 3 parts) |
| `repository/src/gc.rs` | 610 | 610 | 17 | 397-1183 | 1 (607) |
| `repository/src/local.rs` | 457 | 457 | 21 | 456-1020 | 1 (454) |
| `repository/src/main.rs` | 637 | 637 | 24 | 737-1476 | 1 (634) |
| `repository/src/package.rs` | 434 | 383 | 17 | 402-855 | 1 (380) |
| `repository/src/server.rs` | 7487 | 7487 | 113 | 3911-12653 | 10 (mod + 9 parts) |
| `repository/src/store.rs` | 3468 | 3408 | 88 | 4399-8560 | 6 (mod + 5 parts) |
| `repository/src/trust.rs` | 523 | 523 | 16 | 577-1166 | 1 (520) |
| `repository/src/web/mod.rs` | 584 | 584 | 23 | 1228-1933 | 1 (581) |

Totals: 12 files, 18,679 inline test LOC, 455 tests (431 in the `mfb_repository` library, 24 in
the `mfb-repo` binary, `src/main.rs`; `repository/Cargo.toml` `[[bin]] name = "mfb-repo"`); I
creates 29 test-only files.

### Verified properties

- **Every test is assigned exactly once.** Per-file totals before == after in the I map's
  generator for all 12 files (`grep -c '#\[test\]'` + `grep -c 'tokio::test'`), and the part
  grouping of §4.10-§4.12 assigns each map topic to exactly one part
  (`python3 -I /tmp/loc/rw-I/parts.py` asserts the topic sets match and prints 75 / 88 / 113
  tests per host). No `macro_rules!` in any test module
  (`grep -n macro_rules repository/src/*.rs repository/src/web/mod.rs` → none), so no
  textual-scope ordering hazard.
- **Parts see `tests/mod.rs`'s private items and private imports with no visibility edit.**
  Probe `/tmp/hij_probe` (host with a private `use std::path::PathBuf;`, `tests/mod.rs` =
  `use super::*;` + `mod` lines, children `use super::*;`) → no warning, 2 passed. Because
  every shared helper (including `client.rs`'s HTTP stub and `server.rs`'s owner, transfer,
  MFPC and signed-request harnesses) stays in `tests/mod.rs`, I makes no `pub(super)` edit.
- **Glob-cycle imports compile without warnings** — `/tmp/loc/probe2` printed `6`.
- **Part names shadow nothing the tests use.** A `mod <part>;` in `tests/mod.rs` shadows any
  glob-imported type-namespace item of the same name. No moved test names a path through a
  part name: `sed -n <range>p <host> | grep -cE '(^|[^a-z_:])<part>::'` → 0 for every part name
  of §4.10-§4.12 (`transport verification accounts` in client.rs 1980-5602;
  `accounts packages blobs transfers registry_tables` in store.rs 4399-8560;
  `accounts identity tokens publish validate log_routes pages search_limits transfers` in
  server.rs 3911-12653).
- **`#[path]` from a crate root resolves relative to `src/`.** Probe `/tmp/hij_probe`:
  `#[path = "host/gen_x/tests/mod.rs"]` in `src/host.rs` loaded `src/host/gen_x/tests/mod.rs`.
  `main.rs` uses the same rule (§4.6).
- **No script, doc or guard names a repository `::tests::` path.**
  `grep -rn "\(server\|store\|client\|local\|gc\|abi\|backfill\|blobstore\|trust\|package\|web\)::tests::" .ai scripts tests/guards AGENTS.md src/docs .github repository/*.md`
  → none. `grep -rn "repository/" tests/guards scripts/coverage-exceptions.txt` → only the
  `repository/src/main.rs` production-entrypoint exception (`main()`), unaffected.
- **No respawn or source-location dependency.** `grep -n "current_exe\|--exact\|module_path!" repository/src`
  → none. The only include path in a moved range is `store.rs`'s `include_str!("store.rs")`
  (`grep -n 'include_str!("store.rs")' repository/src/store.rs` → line 5730).
- **The only Windows-gated moved line compiles under the same names as its ungated siblings.**
  `local.rs`'s test `from_env_prefers_mfb_home_then_falls_back_to_platform_home` has a
  `#[cfg(windows)]` block naming `LocalPaths::from_env`, `PathBuf::from` and `std::env::var`;
  the same fn calls `LocalPaths::from_env()` and `PathBuf::from(…)` ungated (local.rs test
  module lines 103-111, `awk 'NR>=456 && NR<=1020' repository/src/local.rs | grep -n "PathBuf\|LocalPaths::from_env"`).
  The macOS compile of the moved fn therefore proves the Windows block's names resolve too; no
  Windows host build is needed (the Windows box `192.168.1.206` has no documented Rust
  toolchain). Every other cfg gate in the moved code is `#[cfg(unix)]`, compiled on macOS.
- **Moving an inline test module out of a code file removes covered lines from that file's
  coverage denominator** (`.ai/testing-gates.md`). Per-file coverage is not a plan-176 gate
  (§Validation Plan "Coverage check"), so I runs no coverage measurement.

## 3. Design Overview

One convention: the inline module becomes `#[cfg(test)] mod tests;` (or
`#[cfg(test)] pub(crate) mod tests;` where it is `pub(crate)` today) →
`repository/src/<host>/tests/mod.rs`, a test-only file (limit 1000).

- **Block ≤ 1000 (nine hosts):** the whole block moves verbatim (dedented) into
  `tests/mod.rs`. Its first line is already `use super::*;`. No part files, no path change,
  no visibility edit.
- **Block > 1000 (`client.rs` 2838, `store.rs` 3408, `server.rs` 7487):** `tests/mod.rs` holds
  the shared `use` lines and every helper used by 2+ parts, from a `mod.rs` helper or from
  outside the module (the I map's transitive placement pass), plus one `mod <part>;` per part.
  Each part starts with `use super::*;` and holds its tests plus the helpers only it uses.
  Parts are the I map's topics merged by responsibility into the fewest files that keep each
  ≤ 900 (part LOC = sum of map topics − one `use super::*;` per merged neighbour; "sum of map
  parts", `python3 -I /tmp/loc/rw-I/parts.py`). The map's support files fold back into
  `tests/mod.rs`, which stays ≤ 900 with them (client 369, server 649), so no item needs
  `pub(super)`.

Gate class: test letter. Per phase: library and binary `--list` counts before == after, the
moved modules' tests pass, `cargo check -p mfb_repository --all-targets` and `cargo clippy -p
mfb_repository --all-targets` add no warning, and the file-size guard shows only TIGHTEN
rows for the phase's files before the tighten.

Risk concentrates in:
- **`server.rs`** (7487 LOC, 113 tests, 10 new files) — last phase.
- **Cross-module test helpers** — `store::tests`, `local::tests` and `backfill::tests` are
  imported by other hosts' tests; each such item stays in its `tests/mod.rs` so the importers'
  paths do not change.
- **`store.rs`'s self-reading test** — its `include_str!` path changes with the move (§4.11).

Rejected alternatives:
- *Moving the test modules into `repository/tests/` integration tests* — they reach private
  items through `super::*`; an integration crate cannot.
- *`main.rs` tests at `repository/src/tests/`* (plain `mod tests;` from the crate root) — the
  directory would also be where `lib.rs`'s `mod tests;` resolves, a latent collision; `main/tests/`
  via `#[path]` follows the `<host>/tests/` convention every other host uses.
- *Splitting a ≤ 1000 block by topic* — the rule does not require it, and each split costs
  files and (for support files) `pub(super)` edits for no budget gain.
- *Harness support files beside `mod.rs` (`client/tests/stub.rs`; `owners.rs`,
  `transfer_support.rs`, `mfpc.rs`, `signed_requests.rs` in `server/tests/`)* — they would cost
  53 `pub(super)` edits (27 + 26, the I map's name search) for no budget gain: with the harness
  in `mod.rs` it is 369 / 649 LOC, under 900.

## 4. Detailed Design

Line numbers are as of `17a9b5799` (unchanged at `ce7a995ca`); item and test NAMES are
authoritative if lines drift. Every host keeps `#[cfg(test)] mod tests;` at the old position.
Visibility edits: none.

### 4.1 `repository/src/abi.rs` (31 tests) → `repository/src/abi/tests/mod.rs` (761)

The whole block: `use super::*;`, `use mfb_wire::mfpc::{MAX_MFPC_SECTIONS, MFPC_MAGIC};`; helpers
`put_u16`, `put_u32`, `put_u64`, `string_pool`, `abi_section`, `container`,
`native_library_table`, `container_with_major`, `empty_entry_pool`; tests (container)
`a_wrong_mfpc_major_version_is_now_rejected_by_this_crate_too`,
`a_non_container_payload_still_means_no_such_section_not_an_error`,
`parses_symbol_hash_map_from_container`, `non_container_payloads_are_an_empty_index`,
`missing_string_pool_or_abi_section_is_an_error`, `duplicate_section_id_is_rejected`,
`truncated_section_table_and_section_are_rejected`,
`unsupported_abi_version_and_out_of_range_name_are_rejected`,
`truncated_string_pool_entry_is_rejected`, `read_integer_helpers_report_truncation`,
`a_real_compiler_produced_package_still_parses`; (vendor) `parses_vendor_hashes_from_section_ten`,
`resolves_the_platform_triple_for_each_locator`,
`an_empty_arch_is_the_any_arch_wildcard_not_a_concrete_arch`,
`an_unknown_libc_discriminant_is_rejected`, `two_locators_sharing_one_hash_stay_two_locators`,
`no_section_ten_means_no_vendor_blobs`, `truncated_vendor_hash_is_rejected`,
`oversized_vendor_table_is_rejected_before_probing`, `normal_sized_vendor_table_still_parses`;
(manifest) `reads_author_and_url_from_the_manifest_section`,
`manifest_metadata_is_absent_or_malformed_but_never_silently_wrong`,
`reads_the_description_from_section_eighteen`; (limits)
`string_pool_does_not_preallocate_beyond_what_the_section_can_hold`,
`string_pool_rejects_excessive_entry_count`, `every_consumer_rejects_an_oversized_string_pool`,
`string_pool_rejects_excessive_aggregate_bytes`, `section_table_rejects_excessive_section_count`,
`abi_index_rejects_excessive_export_count`, `package_meta_rejects_excessive_field_count`,
`payloads_exactly_at_every_ceiling_are_still_accepted`.
Host keeps `#[cfg(test)] pub(crate) fn vendor_ref_for_hash` (485-496; used by `gc.rs` and
`store.rs` tests). abi.rs inline test LOC after: 14. Hazards: `a_real_compiler_produced_package_still_parses`
reads `env!("CARGO_MANIFEST_DIR")/tests/fixtures/libsnd.mfp` — manifest-relative.

### 4.2 `repository/src/backfill.rs` (9 tests) → `backfill/tests/mod.rs` (543)

The whole block: `use super::*;`, `use crate::store::tests::register_keys;`; helpers `harness`,
`put_u16`, `put_u32`, `string_pool`, `manifest_section`, `vendor_table`, `container`,
`payload_for`, `package_bytes` (stays `pub(super)`; called as
`crate::backfill::tests::package_bytes(...)`, old :386, which still resolves), `serialize`,
`payload_with_docs`, `sample_doc_section`, `publish_stored`, `mismatched_package_bytes`,
`payload_with_description`; tests `backfill_fills_doc_sections_and_is_idempotent`,
`a_malformed_doc_section_is_counted_skipped_and_not_recorded`,
`backfill_populates_metadata_and_is_idempotent`,
`a_metadata_mismatch_is_skipped_counted_and_left_untouched`,
`no_manifest_section_still_updates_description_with_null_author`,
`backfill_fills_descriptions_and_stays_quiet_about_packages_that_have_none`,
`a_missing_blob_is_counted_and_the_sweep_continues`, `a_malformed_manifest_section_is_unparseable`,
`a_malformed_vendor_table_is_unparseable`. All 9 are `#[tokio::test]`; attributes move verbatim.

### 4.3 `repository/src/blobstore.rs` (21 tests) → `blobstore/tests/mod.rs` (316)

The whole block: `use super::*;`; helpers `expect_err`, `unusable_root`; tests
`parse_local_datapath`, `parse_s3_datapath_variants_normalize_prefix`,
`parse_s3_carries_endpoint`, `parse_rejects_endpoint_without_s3`, `parse_rejects_bucketless_s3`,
`into_store_rejects_datapath_that_is_a_file`, `into_store_reports_undirectory_creation_failure`,
`into_store_creates_missing_local_directory`, `into_store_rejects_s3_backend_when_feature_is_off`,
`blob_ref_and_name_are_content_addressed`, `blob_kind_db_roundtrip`,
`local_stage_promote_get_roundtrip`, `local_native_blob_roundtrip_uses_bin_suffix`,
`local_abort_removes_staged_blob`, `stage_reports_write_failure`,
`promote_reports_rename_failure_and_leaves_nothing_servable`,
`size_reports_missing_absent_and_present_blobs`, `size_surfaces_non_not_found_stat_errors`,
`delete_removes_blob_and_is_idempotent`, `delete_surfaces_non_not_found_errors`,
`get_surfaces_non_not_found_read_errors` (7 `#[test]` + 14 `#[tokio::test]`).

### 4.4 `repository/src/gc.rs` (17 tests) → `gc/tests/mod.rs` (607)

The whole block: `use super::*;`, `use crate::store::now_unix;`; helpers `DAY`,
`UNSTATTABLE_HASH`, `fixture`, `put_object`, `owner_id`, `raw_conn`, `raw_blob_row`, `publish`;
tests `dry_run_reports_only_the_orphan_and_deletes_nothing`,
`delete_reclaims_the_orphan_and_spares_live_blobs`, `interrupted_delete_is_recollected`,
`grace_window_shields_a_fresh_orphan`, `a_publish_landing_mid_sweep_saves_a_listed_candidate`,
`zero_and_negative_grace_are_refused`, `run_refuses_a_zero_grace_period_before_touching_anything`,
`run_refuses_a_grace_period_that_underflows_the_clock`,
`unparseable_candidate_kind_is_reported_and_never_collected`,
`a_stat_failure_is_reported_and_the_candidate_is_still_listed`,
`a_failed_object_delete_leaves_the_row_in_place`,
`a_failed_row_delete_is_reported_and_the_next_run_self_heals`, `json_report_totals_reachable_bytes`,
`json_mode_surfaces_every_kind_of_broken_reachable_blob`,
`text_report_renders_a_plural_delete_run_with_failures`,
`text_report_renders_an_empty_sweep_and_a_plural_delete_count`, `byte_and_age_formatting`.
`crate::store::tests::register_keys` (old gc.rs:434) keeps resolving (§4.11 keeps it in
`store/tests/mod.rs`).

### 4.5 `repository/src/local.rs` (21 tests) → `local/tests/mod.rs` (454)

Declaration stays `#[cfg(test)] pub(crate) mod tests;`. The whole block: `use super::*;`; helpers
`mode`, `ENV_LOCK`, `env_guard` (stays `pub(crate)`; called as `crate::local::tests::env_guard()`
from `client.rs` and `server.rs` tests), `home_is_a_file`, `fixture`,
`assert_keypair_writer_surfaces_write_failures`; tests `writes_and_reads_both_keypairs`,
`from_env_prefers_mfb_home_then_falls_back_to_platform_home`,
`read_key_errors_when_missing_or_corrupt`, `writes_owner_scoped_sessions`,
`every_writer_reports_the_directory_it_could_not_create`,
`auth_keypair_writer_surfaces_write_failures`, `ident_keypair_writer_surfaces_write_failures`,
`pending_ident_keypair_writer_surfaces_write_failures`, `pins_server_key_once_and_rejects_changes`,
`root_pin_round_trips_and_reads_none_when_absent`,
`snapshot_version_round_trips_and_reads_none_when_absent`,
`checkpoint_round_trips_and_rejects_malformed`,
`pinned_state_readers_fail_closed_on_unreadable_files`,
`root_version_round_trips_and_fails_closed_when_malformed`,
`root_pin_without_a_separator_is_malformed`, `pin_server_key_refuses_a_corrupt_pin_file`,
`pin_server_key_reports_a_home_that_is_not_a_directory`,
`pin_server_key_honours_an_out_of_band_fingerprint`, `pending_ident_rotation_stages_then_promotes`,
`discarding_a_staged_rotation_keeps_the_live_keypair`,
`promoting_without_a_staged_rotation_reports_the_missing_file`.
Hazards: `.ai/testing-gates.md` describes `local::tests`' `ENV_LOCK` by module name — still true
(the module path is unchanged).

### 4.6 `repository/src/main.rs` (24 tests, `mfb-repo` binary) → `repository/src/main/tests/mod.rs` (634)

Declaration: `#[cfg(test)] #[path = "main/tests/mod.rs"] mod tests;` (relative to `src/`; the
module path stays `tests`). The whole block: `use super::*;`; helper `args`; tests
`init_root_expires_at_guards_range_and_overflow`,
`parse_args_reads_space_and_equals_forms_and_defaults_listen`,
`parse_args_rejects_missing_values_bad_listen_and_unknown_options`,
`parse_args_reads_s3_datapath_and_endpoint`, `parse_args_rejects_s3_endpoint_without_s3_datapath`,
`parse_args_reads_the_equals_form_of_s3_endpoint`, `parse_gc_args_defaults_to_a_dry_run_with_a_24h_grace`,
`parse_gc_args_accepts_an_s3_datapath`, `parse_gc_args_refuses_a_zero_or_bad_grace_period`,
`parse_gc_args_rejects_every_flag_missing_its_value`, `parse_gc_args_reads_the_equals_form_of_every_flag`,
`parse_gc_args_rejects_an_s3_endpoint_without_an_s3_datapath`,
`parse_init_root_args_reads_fields_defaults_and_validates_expiry`,
`parse_init_root_args_rejects_every_flag_missing_its_value`,
`parse_init_root_args_names_each_missing_required_flag`,
`parse_renew_root_args_requires_the_root_key_file`,
`read_root_key_file_round_trips_what_init_root_printed`,
`renew_root_operation_keeps_the_anchor_and_a_second_init_is_refused`,
`init_root_operation_creates_config_and_returns_root_private_key`,
`parse_reanchor_args_reads_all_fields_and_reports_missing`,
`parse_backfill_args_reads_both_forms_and_requires_both_paths`,
`parse_reanchor_args_rejects_every_flag_missing_its_value`,
`parse_reanchor_args_names_each_missing_required_flag`, `reanchor_operation_binds_a_fresh_ident`.
Hazards: `scripts/coverage-exceptions.txt:41` excepts `repository/src/main.rs` for the production
`main()` only — unaffected.

### 4.7 `repository/src/package.rs` (17 tests) → `package/tests/mod.rs` (380)

Only `mod tests` (402-855) moves; `#[cfg(test)] pub(crate) mod test_support` (347-400, 51 LOC,
used as `crate::package::test_support::*` by `backfill.rs` and `server.rs` tests) stays inline.
The whole block: `use super::test_support::{serialize, TestPackage};` (`super` is still
`package`), `use super::*;`; helpers `Fixture`, `signed_fixture`, `signed_fixture_for`,
`RawPackage` + `impl Default for RawPackage`, `put`, `build_raw`; tests
`parses_and_verifies_a_fully_signed_package`, `parse_rejects_malformed_prefixes_and_versions`,
`parse_rejects_truncation_and_bad_signature_length`,
`decode_metadata_key_handles_prefix_bare_and_malformed`, `metadata_key_fingerprint_is_empty_for_empty_key`,
`package_content_hash_rejects_non_packages`, `read_integer_helpers_report_truncation`,
`verify_functions_reject_tampering`, `proof_owner_binds_to_the_author_not_the_prefix`,
`a_case_differing_legacy_prefix_still_verifies`, `verify_proof_rejects_field_mismatch_and_bad_json`,
`verify_package_signature_rejects_unsigned_type`, `validate_signature_header_accepts_valid_and_rejects_invalid`,
`verify_attestation_rejects_ident_without_hash`, `unsigned_package_parses_and_must_carry_no_chain`,
`signed_package_missing_a_chain_field_is_rejected`, `parse_rejects_field_limits_and_empty_required_fields`.
package.rs inline test LOC after: 53 (`test_support` 51 + the 2-line declaration).

### 4.8 `repository/src/trust.rs` (16 tests) → `trust/tests/mod.rs` (520)

The whole block: `use super::*;`, `use crate::server::{OwnerBinding, Publisher};`,
`use std::cell::RefCell;`; helpers `IDENT`, `World`, `PendingLink`, `Owner`, `key`,
`impl World`, `placeholder_binding`, `pinned`, `untrusted_detail`, `alice_to_bob`; tests
`pre_transfer_release_still_verifies`, `recipient_release_verifies_through_the_chain`,
`chain_of_two_transfers_verifies`, `withheld_release_breaks_the_digest`,
`rotation_before_and_after_transfer_verifies`, `forged_offer_is_rejected`,
`forged_accept_is_rejected`, `replayed_link_at_wrong_sequence_is_rejected`,
`legacy_link_terminates_the_chain`,
`a_legacy_link_with_a_foreign_from_owner_does_not_fail_pre_transfer_releases`,
`legacy_round_trip_origin_owner_release_verifies`, `index_from_an_older_server_is_a_single_owner_chain`,
`round_trip_owner_pin_selects_its_first_tenure`, `a_pin_on_bob_verifies_alice_release_by_digest`,
`first_use_trusts_bound_key_ancestors_only_for_the_origin`, `a_pin_off_every_chain_is_a_reanchor`.

### 4.9 `repository/src/web/mod.rs` (23 tests) → `web/tests/mod.rs` (581)

`web/mod.rs` is a `mod.rs`, so `mod tests;` resolves to `web/tests/mod.rs` (`ls repository/src/web`
→ `mod.rs style.css`; no collision). The production `include_str!("style.css")` is not in the
test module. The whole block: `use super::*;`; helpers `docs_view`, `doc_decl_entry`,
`doc_page_with_decls`, `header_test_view`, `header_test_version`, `doc_page_with_intro`; tests
`safe_href_rejects_every_non_http_scheme`, `safe_href_accepts_ordinary_http_and_https_urls`,
`a_rejected_url_renders_as_inert_escaped_text_not_a_link`,
`only_the_latest_release_shows_its_native_targets_expanded`,
`the_header_names_the_newest_active_release_while_the_table_keeps_the_yanked_one`,
`a_deprecated_headline_release_carries_its_state_badge`, `no_active_release_is_stated_not_omitted`,
`a_package_with_no_versions_renders_no_release_copy_at_all`, `package_tabs_mark_exactly_the_current_tab`,
`the_empty_docs_page_states_the_author_included_no_documentation`,
`the_docs_page_renders_package_prose_and_every_callout_kind`, `docs_page_escapes_publisher_prose`,
`the_docs_page_renders_every_part_of_a_declaration`, `the_sidebar_index_links_every_group_heading`,
`internal_declarations_are_not_rendered`, `a_page_with_no_public_declarations_says_so`,
`doc_inline_renders_backtick_spans_as_escaped_code`,
`a_declaration_named_like_a_shell_id_does_not_duplicate_it`,
`docs_page_escapes_every_publisher_controlled_field`,
`interpolated_values_are_escaped_by_the_template_engine`,
`every_html_response_carries_the_csp_from_the_shared_builder`, `the_policy_permits_no_script_at_all`,
`the_page_shell_is_script_free_and_links_a_real_stylesheet`.

### 4.10 `repository/src/client.rs` (75 tests) → `client/tests/` (mod + 3 parts)

- `mod.rs` (369 = map mod.rs 184 + map `stub.rs` 199 − its `use super::*;` − `mod stub; use
  stub::*;` − 11 topic `mod` lines): `use super::*;`, `use std::io::{Read, Write};`,
  `use std::net::TcpListener;`, `use std::thread;`, `use crate::server::IdentChainLink;`,
  `use std::net::TcpStream;`, `use std::sync::{Arc, Mutex};` (the mid-module `use` lines at
  2609 and 3030-3031, `grep -nE '^\s*use ' repository/src/client.rs`, hoisted here; the
  fn-local `use crate::server::IndexVersion;` at 2730 moves with its fn), `mod transport;`,
  `mod verification;`, `mod accounts;`; shared `MetadataFixture`, `temp_paths`,
  `MetadataAuthority` + impl, `build_metadata_delegating`, `build_metadata_delegating_at_root_version`,
  `with_metadata_routes`, `index_body`, `index_body_for`, and the in-process HTTP stub
  `method_of`, `target_of`, `path_of`, `body_of`, `json_body`, `field`, `read_request`,
  `reason`, `http_reply`, `Stub` + impl, `spawn_raw`, `Routes` + impl, `Registry` + impl,
  `seed_ident`, `assert_signed` — all private, as today.
- `transport.rs` (798, 38 tests; map topics redirects 222 + transport 123 + parsing 150 +
  offline_failures 179 + blobs_publish 128):
  `a_credentialed_post_does_not_follow_a_cross_origin_redirect`,
  `a_blob_get_still_consults_the_redirect_policy`, `a_registry_redirect_to_an_internal_plaintext_host_is_refused`,
  `a_hostname_redirect_resolving_to_loopback_is_refused_before_connecting`,
  `a_presigned_blob_redirect_to_a_public_host_is_still_allowed`,
  `a_redirect_host_resolving_into_a_blocked_range_is_refused`, `an_unresolvable_redirect_host_fails_closed`,
  `the_redirect_guard_uses_the_platform_resolver`,
  `redirect_targets_block_internal_and_downgrade_but_allow_public_https`,
  `a_server_authored_error_is_escaped_at_the_boundary`,
  `a_server_authored_newline_is_escaped`, `an_ordinary_server_error_is_unchanged`,
  `http_client_is_built_once_and_shared`, `fetch_blob_survives_a_body_slower_than_the_control_timeout`,
  `response_bodies_are_capped_and_errors_keep_their_status`,
  `transport_security_rejects_an_unparseable_url`, `transport_security_requires_https_for_non_loopback`,
  `repo_url_env_defaults_and_overrides`, `decode_hex32_accepts_only_32_byte_hex`,
  `percent_encode_preserves_unreserved_and_escapes_the_rest`, `metadata_helpers_read_fields_and_report_missing`,
  `decode_delegated_key_reads_or_errors`, `follow_ident_chain_walks_signed_links_and_detects_reanchor`,
  `abi_map_reads_object_and_defaults_empty`, `package_request_carries_all_fields`,
  `an_older_index_without_transfer_fields_deserializes`,
  `register_refuses_and_preserves_keys_when_owner_keys_already_exist`,
  `register_duplicate_failure_leaves_no_local_keys`,
  `network_flows_reject_invalid_owner_names_before_any_request`,
  `network_flows_report_connection_failure_against_a_dead_endpoint`,
  `auth_without_local_key_probes_owner_and_surfaces_the_local_error`,
  `flows_needing_local_ident_key_fail_when_it_is_absent`, `verify_pinned_metadata_is_a_noop_without_a_pin`,
  `trust_registry_fails_to_connect_against_a_dead_endpoint`,
  `fetch_blob_checks_the_content_hash_and_surfaces_error_bodies`,
  `blob_exists_maps_status_to_presence`, `put_blob_authenticates_with_a_bearer_header_and_reports_failures`,
  `validate_and_publish_post_the_artifact_with_the_local_session`; helpers
  `spawn_connection_probe`, `DEAD_URL`.
- `verification.rs` (844, 15 tests; metadata_chain 194 + metadata_versions 212 + checkpoint 219
  + inclusion 222): `metadata_chain_verifies_and_returns_the_delegated_server_key`,
  `metadata_chain_rejects_tampering_and_rollback`,
  `metadata_chain_rejects_undelegated_server_key_and_bad_root_signature`,
  `metadata_chain_rejects_snapshot_timestamp_version_disagreement`,
  `trust_registry_pins_only_a_root_that_delegates_the_pinned_server_key`,
  `verify_pinned_metadata_advances_the_version_and_refuses_a_rollback`,
  `verify_pinned_metadata_refuses_a_re_anchored_root`,
  `verify_pinned_metadata_follows_a_renewal_and_refuses_the_retired_root`,
  `fetch_checkpoint_pins_then_refuses_rollback_fork_and_bad_signatures`,
  `verify_log_consistency_advances_the_pin_only_on_a_proven_extension`,
  `fetch_checkpoint_rejects_an_unproven_larger_fork`,
  `publish_inclusion_rejects_a_leaf_for_a_different_package`,
  `publish_leaf_payload_matches_the_store_encoding`, `verify_publish_inclusion_rejects_an_unproven_larger_fork`,
  `verify_publish_inclusion_rejects_a_mismatched_proof_envelope`; helpers `build_metadata`,
  `build_metadata_versions`.
- `accounts.rs` (830, 22 tests; registration_auth 166 + linking 167 + rotation 129 +
  signed_actions 202 + attestation_index 170):
  `ensure_server_key_pins_the_key_and_rejects_a_self_inconsistent_ident`,
  `register_posts_role_separated_proofs_for_the_keys_it_stores`,
  `auth_signs_the_challenge_and_stores_the_session`,
  `auth_refuses_a_local_keypair_whose_halves_disagree`, `auth_surfaces_unknown_owner_from_the_probe`,
  `link_start_relays_only_a_blob_sealed_under_the_pairing_code`,
  `link_fetch_installs_the_relayed_ident_keypair`, `link_fetch_rejects_a_blob_that_is_not_an_ident_keypair`,
  `revoke_machine_signs_the_challenge_with_the_ident_key` (its `crate::local::tests::env_guard()`
  calls keep resolving), `rotate_ident_promotes_the_staged_keypair_on_success`,
  `rotate_ident_promotes_when_the_registry_already_holds_the_staged_key`,
  `rotate_ident_discards_the_staging_when_the_registry_kept_the_old_ident`,
  `rotate_ident_keeps_the_staging_when_the_registry_cannot_be_consulted`,
  `set_org_member_signs_grant_and_removal_distinctly`,
  `issue_publish_token_keeps_the_private_half_local`,
  `revoke_publish_token_signs_the_revocation_with_the_ident_key`,
  `transfer_offer_and_accept_are_each_ident_signed`,
  `set_release_state_carries_both_session_and_ident_signature`,
  `request_attestation_verifies_the_attestation_under_the_pinned_key`,
  `fetch_index_verifies_the_name_binding_before_returning_the_ident_key`,
  `fetch_index_rejects_a_validly_signed_response_for_another_ident`,
  `fetch_index_accepts_an_honest_response_whose_owner_case_differs`; helpers
  `rotate_reconcile_stub`, `ident_chain_body`.
Hazards: `.ai/net-tls.md` and `.ai/testing-gates.md` cite production fns / test names, not test
module paths — unaffected.

### 4.11 `repository/src/store.rs` (88 tests) → `store/tests/` (mod + 5 parts)

Declaration stays `#[cfg(test)] pub(crate) mod tests;`.
- `mod.rs` (195 = map mod.rs 209 − 19 topic `mod` lines + 5 part `mod` lines): `use super::*;`;
  `mod accounts;`, `mod packages;`, `mod blobs;`, `mod transfers;`, `mod registry_tables;`;
  shared `test_store`, `RegisteredKeys`, `sign_offer_for_test`, `sign_accept_for_test`,
  `offer_and_accept_for_test`, `register_keys`, `register`, `approved`, `publish_as`,
  `legacy_log`, `legacy_transfer`, `registration_payload`, `drop_tables`. Forced here because
  other hosts reach them as `crate::store::tests::X`: `register_keys` (backfill, gc),
  `legacy_transfer` (server ×5), `drop_tables` (server), with `RegisteredKeys`, `legacy_log`
  and `test_store` on their paths.
- `accounts.rs` (864, 27 tests; open 137 + registration 185 + challenges 77 + linking 210 +
  key_roles 173 + tokens_orgs 87): `startup_creates_database_and_packages_dir`,
  `poisoned_connection_lock_recovers_and_keeps_serving`,
  `open_repository_makes_the_key_bearing_database_private`,
  `open_repository_leaves_an_already_private_database_alone_and_still_works`,
  `open_repository_rejects_non_file_db_and_non_dir_data`,
  `open_repository_reports_directories_it_cannot_create`,
  `registration_persists_owner_and_both_keys`, `duplicate_registration_is_case_folded`,
  `registration_rejects_bad_proof`, `registration_rejects_role_replayed_proofs`,
  `server_keypair_is_created_once_and_stable`,
  `owner_name_validation_guards_every_key_and_challenge_entry_point`,
  `ident_challenge_rejects_an_unknown_owner`, `challenge_lifecycle_accepts_signature_once`,
  `challenge_rejects_bad_signature_and_unknown_owner`, `challenge_rejects_expired_challenge`,
  `complete_challenge_rejects_unknown_id`, `add_auth_key_and_rotate_reject_unknown_owner_and_bad_signatures`,
  `ident_chain_is_empty_before_rotation`, `pairing_blob_is_single_use_and_expires`,
  `linked_machine_key_works_and_revocation_kills_sessions`,
  `revocation_challenge_requires_the_ident_key`,
  `key_role_separation_is_enforced_at_every_creation_path`,
  `distinct_keys_still_register_link_tokenize_and_rotate`, `key_insertion_has_exactly_one_writer`,
  `org_member_removal_and_listing`, `publish_token_issue_validates_scope_and_ttl`; helper
  `is_missing`.
- `packages.rs` (761, 26 tests; publish 206 + versions 187 + search 158 + log 213):
  `publish_rejects_unsafe_package_and_version`, `publish_records_publisher_and_epoch`,
  `package_version_exists_reflects_publishes`,
  `publishing_into_another_accounts_package_ident_is_refused`,
  `latest_active_version_skips_a_yanked_newest_release`,
  `latest_active_version_skips_a_blocked_newest_release`,
  `latest_active_version_skips_a_legal_tombstoned_newest_release`,
  `latest_active_version_returns_a_deprecated_release_with_its_state`,
  `latest_active_version_is_none_when_every_version_is_inactive`,
  `latest_active_version_is_none_for_no_versions_and_unknown_idents`,
  `version_docs_round_trip_byte_for_byte`, `a_version_with_no_docs_row_yields_none`,
  `a_yanked_newest_release_falls_back_to_the_older_active_releases_docs`,
  `an_undocumented_latest_release_never_borrows_an_older_releases_docs`,
  `putting_version_docs_twice_keeps_the_first_row`,
  `set_release_state_rejects_unpublished_and_updates_published`,
  `fuzzy_search_only_scans_length_compatible_candidates`, `typosquat_and_reaping_hardening`,
  `edit_distance_one_covers_insert_delete_substitute`,
  `owner_version_count_totals_all_versions_across_packages`,
  `publish_log_lookup_does_not_cross_match_like_wildcards`,
  `every_state_change_appends_exactly_one_log_entry`, `signed_checkpoint_is_memoised_by_log_size`,
  `a_negative_log_size_is_rejected`, `index_canonical_hash_changes_with_index_state`,
  `a_malformed_log_leaf_hash_is_rejected_rather_than_truncated`; helpers `store_with_versions`,
  `version_id`.
- `blobs.rs` (394, 10 tests; blob_reachability 224 + blob_targets 171):
  `a_published_mfp_blob_is_never_a_candidate`, `candidates_are_unreferenced_blobs_past_the_grace_period`,
  `a_yanked_versions_blobs_stay_reachable`, `a_shared_blob_survives_removing_one_of_its_versions`,
  `forget_blob_is_idempotent_and_fk_guarded`, `blob_is_reachable_tracks_both_halves_of_the_union`,
  `unreachable_blobs_refuses_a_bad_grace_period`,
  `recording_a_native_blob_over_a_package_blob_skips_the_bin_promote`,
  `two_locators_sharing_one_blob_hash_write_two_target_rows`,
  `a_wildcard_arch_is_stored_as_null_and_stays_distinct`.
- `transfers.rs` (657, 13 tests; transfers 160 + transfer_history 144 + legacy_migration 217 +
  schema_migration 139): `a_stale_offer_cannot_rebind_a_package_after_ownership_moved`,
  `transfer_offer_and_accept_error_branches`, `legacy_pending_offer_cannot_be_accepted`,
  `transfer_history_is_append_only_and_sequenced`, `stale_terms_refuse_offer_and_accept`,
  `migrating_a_legacy_database_records_legacy_transfers_and_publishers`,
  `migrating_recovers_an_accepted_transfer_hidden_by_a_later_offer`,
  `migrating_replays_a_multi_hop_legacy_chain`, `migrating_recovers_epochs_after_a_legacy_round_trip`,
  `migrating_refuses_a_log_that_disagrees_with_the_owner`,
  `migrating_a_legacy_database_adds_the_abi_index_and_kind_columns`,
  `migrating_a_legacy_database_adds_the_metadata_columns_and_target_table`,
  `add_column_if_missing_distinguishes_a_duplicate_column_from_a_real_failure`; helpers
  `history_summary`, `epochs`, `store_owner_id_unlocked`.
- `registry_tables.rs` (544, 12 tests; registry_root 142 + lost_account_tables 188 +
  lost_package_tables 216): `registry_config_absent_then_present_after_init`,
  `renew_registry_root_keeps_the_anchor_and_rotates_the_delegated_keys`,
  `renew_registry_root_refuses_anything_but_the_configured_root_key`,
  `reanchor_registry_root_replaces_the_anchor_only_when_explicitly_selected`,
  `init_registry_root_refuses_to_replace_a_configured_root`,
  `losing_the_owners_table_errors_every_account_lookup_instead_of_panicking`,
  `losing_the_key_tables_errors_revocation_and_chain_reads`,
  `losing_the_session_and_challenge_tables_errors_the_auth_path`,
  `losing_the_package_tables_errors_publish_and_index_reads`,
  `losing_the_log_table_errors_reads_and_aborts_state_changes`,
  `losing_the_server_key_and_config_tables_errors_signing_and_metadata`,
  `losing_the_org_and_transfer_tables_errors_membership_and_handover`; helper `log_kinds`.
The other inline cfg(test) items stay in production position: `type TargetRowForTest` (76-77),
`Store::version_metadata_for_test` (393-408), `version_description_for_test` (411-426),
`target_rows_for_test` (435-458), `force_expire_challenge` (3641-3650) — store.rs inline test
LOC after: 62 (census 3468 − block 3408 + the 2-line declaration).

Include path: `key_insertion_has_exactly_one_writer` (→ `accounts.rs`) does
`include_str!("store.rs")` (old :5730) and counts `INSERT INTO keys (owner_id, role, public_key`
sites. `include_str!` resolves relative to the file that contains it, so in
`repository/src/store/tests/accounts.rs` it becomes `include_str!("../../store.rs")` — same
file, same count. plan-176-L, which turns `store.rs` into `store/mod.rs` + parts, replaces this
include with a runtime read of every top-level `*.rs` under `src/store` (its §4.3); L locates
the test by name (`grep -rln key_insertion_has_exactly_one_writer repository/src/store/tests`),
not by I's part name.

### 4.12 `repository/src/server.rs` (113 tests) → `server/tests/` (mod + 9 parts)

- `mod.rs` (649 = 4 `use` lines + 160 shared-helper LOC + 476 harness LOC from the map's four
  support files less their `use super::*;` lines + 9 part `mod` lines): `use super::*;`,
  `use crate::store::Store;`, `use base64::engine::general_purpose::URL_SAFE_NO_PAD;`,
  `use base64::Engine;`; `mod accounts;`, `mod identity;`, `mod tokens;`, `mod publish;`,
  `mod validate;`, `mod log_routes;`, `mod pages;`, `mod search_limits;`, `mod transfers;`;
  shared `publish_valid_package`, `Harness`, `harness`, `seed_packages`, `search_for`,
  `get_page`, `seed_toolbox`, `peer`, `err_of`, `bearer_headers`, `NOT_BASE64`; owner harness
  `TestOwnerKeys`, `register_owner_with_all_keys`, `register_owner_with_keys`, `open_session`,
  `real_attestation`, `open_session_for_key`, `session_with_claims`; transfer harness
  `index_transfer_terms`, `signed_offer`, `signed_accept`, `transfer_through_handlers`; MFPC
  builders `put_u16`, `put_u32`, `put_u64`, `mfpc_string_pool`, `mfpc_vendor_table`,
  `mfpc_manifest_section`, `mfpc_abi_section`, `mfpc_container`; signed-request builders
  `clone_request`, `signed_request`, `signed_request_with_header_metadata`,
  `owner_signed_request`, `owner_signed_request_attested_for` — all private, as today.
- `accounts.rs` (751, 8 tests; accounts 223 + sessions 201 + pairing 112 + machine_link 218):
  `register_handler_creates_the_account_and_rejects_bad_keys`,
  `challenge_handler_issues_a_nonce_and_refuses_unknown_owners`,
  `login_handler_mints_a_verifiable_session_and_refuses_replay`,
  `jwt_creation_sets_expected_claims_and_verifies`,
  `jwt_verification_rejects_expired_wrong_signature_and_unknown_session`,
  `session_preamble_cross_checks_jwt_claims_against_the_database`,
  `pairing_lookup_without_code_cannot_enrol_an_auth_key`,
  `machine_link_relays_an_opaque_blob_exactly_once`; helper `registration_payload`.
- `identity.rs` (831, 10 tests; signing 204 + rotation 170 + rotation_retry 223 +
  machine_revocation 237): `signing_requires_session_and_matching_owner_and_issues_attestation`,
  `signing_validates_ident_version_and_fingerprint_shapes`, `signing_is_rate_limited_per_client_ip`,
  `rotation_refuses_stale_attestations_and_serves_the_chain`,
  `rotate_ident_rejects_bad_sessions_and_malformed_material`,
  `rotation_retry_preserves_committed_staged_key`, `rotation_retry_finishes_an_interrupted_promotion`,
  `rotation_retry_replaces_an_uncommitted_staging`,
  `rotation_retry_keeps_the_staging_when_the_registry_holds_another_key`,
  `machine_revocation_is_ident_authorized_and_kills_the_session`; helpers `LostRotation`,
  `stage_rotation`, `retry_rotation`.
- `tokens.rs` (705, 8 tests; orgs 212 + publish_tokens 117 + token_issue 166 + token_sessions
  213): `org_roles_are_ident_authorized_and_role_gated`, `admin_cannot_grant_or_remove_the_owner_role`,
  `publish_tokens_are_scoped_and_revocable`, `token_scope_matching_is_owner_folded_and_separator_strict`,
  `issue_token_enforces_scope_ownership_and_signature`, `an_expired_publish_token_cannot_attest`,
  `a_publish_token_session_cannot_enrol_a_machine`,
  `a_publish_token_is_bounded_on_every_authenticated_route`; helper `issue_scoped_token`.
- `publish.rs` (839, 12 tests; publish_chain 222 + publish_records 128 + publish_failures 192 +
  native_blobs 185 + oversized 116): `publish_chain_enforces_two_credentials_and_pinning`,
  `validate_writes_no_target_rows_but_publish_does`,
  `publishing_a_documented_package_stores_its_doc_section_byte_for_byte`,
  `publishing_an_undocumented_package_succeeds_and_stores_no_doc_row`,
  `a_truncated_doc_section_still_publishes_and_records_nothing`,
  `a_header_manifest_metadata_mismatch_is_refused`,
  `publish_refuses_invalid_packages_and_tolerates_a_pre_staged_blob`,
  `publish_aborts_the_staged_blob_when_the_ident_moved_owner`,
  `publish_aborts_the_staged_blob_when_the_store_refuses_the_write`,
  `native_blob_put_head_get_roundtrip_and_auth`, `install_path_serves_blob_and_index`,
  `oversized_string_pool_is_rejected_at_the_request_boundary`; helpers `CraftArgs`,
  `craft_package`, `payload_with_doc_section`, `sample_doc_section`, `publish_payload`.
- `validate.rs` (789, 10 tests; validate 203 + validate_owner 207 + malformed_requests 201 +
  release_state 181): `validate_handler_reports_every_request_field_mismatch`,
  `validate_refuses_an_unsigned_container`, `validate_requires_every_vendor_blob_to_be_uploaded_first`,
  `validate_reports_owner_author_and_duplicate_problems`,
  `author_and_url_round_trip_from_the_signed_manifest`,
  `transfer_and_org_endpoints_reject_malformed_and_wrong_signatures`,
  `put_blob_requires_a_well_formed_bearer_header`, `put_blob_is_throttled_per_owner`,
  `release_state_requires_ident_signature_and_is_served_and_logged`,
  `release_state_rejects_every_malformed_request_shape`; helper `put_field`.
- `log_routes.rs` (781, 13 tests; log 173 + log_bounds 111 + metadata_root 163 + routes 205 +
  serve 133): `log_endpoints_serve_verifiable_checkpoints_and_proofs`,
  `log_checkpoint_follows_an_append_and_still_verifies`,
  `log_publish_entry_and_proof_bounds_are_enforced`,
  `package_index_and_ident_chain_reject_malformed_lookups`,
  `a_pinned_client_verifies_the_chain_across_an_authenticated_root_renewal`,
  `signed_metadata_is_absent_until_the_root_ceremony_then_verifies`,
  `router_has_no_route_conflicts_and_static_beats_param`,
  `the_read_routes_404_without_leaking_whether_an_owner_exists`,
  `the_read_routes_ignore_credentials_entirely`, `health_and_ident_endpoints_report_the_registry_key`,
  `error_helpers_map_store_messages_to_status_codes`, `blob_hash_paths_must_be_lowercase_hex`,
  `serve_binds_the_router_and_answers_over_http`; helper `registration_payload_with_auth_private`.
- `pages.rs` (825, 21 tests; html_pages 177 + html_escaping 123 + package_page 126 + docs_tab
  201 + package_detail 202): `the_search_page_never_shows_a_yanked_version_in_the_result_chip`,
  `the_search_page_badges_a_deprecated_headline_release`, `the_html_routes_do_not_shadow_any_json_route`,
  `the_landing_page_presents_the_root_fingerprint_as_a_thing_to_compare`,
  `the_search_page_renders_all_three_states`,
  `search_results_escape_html_metacharacters_in_publisher_values`,
  `a_hostile_author_and_url_render_inert`, `an_unknown_package_renders_a_404_page_with_the_csp`,
  `the_package_page_shows_yanked_versions_and_the_target_matrix`,
  `the_audit_tab_renders_proofs_and_links_the_raw_json`,
  `every_package_tab_page_marks_exactly_one_current_tab`,
  `the_docs_tab_states_when_a_release_has_no_documentation`,
  `the_docs_tab_renders_stored_documentation_under_the_site_csp`,
  `an_unknown_packages_docs_tab_404s_like_the_other_tabs`, `the_docs_json_route_mirrors_the_docs_tab`,
  `the_docs_json_route_reports_null_for_an_undocumented_release`,
  `the_docs_json_route_404s_like_the_package_route`,
  `the_docs_tab_documents_the_older_active_release_when_the_newest_is_yanked`,
  `package_detail_lists_every_version_including_yanked_ones`,
  `package_detail_names_the_newest_active_release_not_a_yanked_newest`,
  `package_audit_returns_a_checkpoint_with_verifiable_inclusion_proofs`; helpers
  `toolbox_doc_section`, `release_doc_section`, `publish_toolbox_release`, `docs_json`.
- `search_limits.rs` (466, 17 tests; search 152 + descriptions 106 + rate_limits 210):
  `search_ranks_exact_then_prefix_then_substring`,
  `search_names_the_newest_active_release_and_carries_its_state`,
  `search_with_an_empty_query_returns_nothing`, `search_clamps_an_over_cap_limit_and_pages_by_offset`,
  `search_treats_like_and_sql_metacharacters_as_literal_text`,
  `search_falls_back_to_the_fuzzy_tail_only_when_nothing_matched`,
  `a_description_reaches_the_json_and_the_page`,
  `search_matches_descriptions_but_ranks_them_below_idents`,
  `search_results_clamp_long_descriptions_on_a_character_boundary`,
  `challenge_rate_limit_trips_after_the_window_cap`,
  `challenge_rate_limit_cannot_lock_out_a_victim_by_owner_name`, `register_rate_limit_is_per_client_ip`,
  `search_is_rate_limited_per_peer_ip`, `anonymous_log_routes_are_rate_limited_per_ip`,
  `the_log_budget_clears_a_full_install_burst_from_one_ip`, `login_is_throttled_per_client_ip`,
  `rate_limiter_prune_drops_only_fully_elapsed_windows`; helpers `seed_with_description`,
  `register_dummy`.
- `transfers.rs` (866, 14 tests; transfers 179 + transfer_index 173 + transfer_publish 219 +
  transfer_tokens 171 + transfer_governance 128):
  `ownership_transfer_is_two_sided_and_rebinds_the_package`,
  `a_second_transfer_is_offered_by_the_current_owner`, `a_replayed_link_is_refused`,
  `the_index_serves_a_verifiable_transfer_chain`, `a_legacy_transfer_is_served_unsigned`,
  `the_package_page_pairs_the_current_owner_with_their_key`,
  `spec_probe_transfer_recipient_can_publish`, `the_former_owner_cannot_publish_after_transfer`,
  `first_publish_is_still_prefix_bound`, `a_legacy_recipient_cannot_publish_an_unverifiable_release`,
  `the_origin_owner_publishes_after_a_legacy_round_trip`,
  `a_recipient_token_scoped_to_the_transferred_package_can_attest`,
  `the_publish_records_bob_as_publisher_at_epoch_1`,
  `a_transferred_package_is_governed_by_its_current_owner`; helpers `validation_diagnostics`,
  `token_session_for`.
Hazards: calls to `crate::store::tests::{drop_tables, legacy_transfer}`,
`crate::local::tests::env_guard()` and `crate::package::test_support::*` keep resolving.
6 `#[test]` + 107 `#[tokio::test]` = 113. The two longest single test fns
(`machine_revocation_is_ident_authorized_and_kills_the_session`, 236 LOC;
`machine_link_relays_an_opaque_blob_exactly_once`, 217 LOC) move unchanged into parts far under
900.

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

- **G0 (before the phase's first edit):**
  `cargo test -p mfb_repository --lib -- --list 2>/dev/null | grep -c ': test$' > /tmp/i_lib_before.txt`,
  `cargo test -p mfb_repository --bin mfb-repo -- --list 2>/dev/null | grep -c ': test$' > /tmp/i_bin_before.txt`,
  `cargo clippy -p mfb_repository --all-targets 2>&1 | grep -c '^warning' > /tmp/i_warn_before.txt`
  (est. 4 min cold, 1 min warm).
- **G1:** the same three commands after the phase's edits → numbers equal the G0 files
  (clippy also proves no `items_after_test_module` and no unused import).
- **G2:** run the moved modules' tests (filter per phase) → all pass, 0 failed.
- **G3:** `cargo test -p mfb --bin mfb spec_citations_resolve` → pass (no I file is cited, so
  this only proves nothing regressed; est. 2 min).
- **G4:** `cargo test --test file_size_budget 2>&1 | grep -E '^  (NEW|GREW|STALE)'` → no output,
  and every `TIGHTEN` row is `inline` for a host of this phase; then
  `MFB_LOC_TIGHTEN=1 cargo test --test file_size_budget` and rerun →
  `grep -cE '^  (NEW|GREW|STALE|TIGHTEN)'` → `0`. Commit `tests/guards/file_size_ceilings.txt`
  with the phase (est. 2 min).
- **G5:** `cargo fmt --check` → no output (est. 20 s).
- **G6 (size):** `for f in <the phase's new files>; do python3 -I tools/loc-census/census.py --file $f; done`
  → every second field ≤ 1000 (≤ 900 for `client/`, `store/`, `server/` files), and
  `python3 -I tools/loc-census/census.py --file <host>` → second field ≤ 250 for each host
  (est. 10 s).

### Phase 1 — Six self-contained hosts

- [x] Move `repository/src/abi.rs`'s tests to `abi/tests/mod.rs` per §4.1 (`vendor_ref_for_hash`
      stays in the host).
- [x] Move `repository/src/backfill.rs`'s tests to `backfill/tests/mod.rs` per §4.2.
- [x] Move `repository/src/blobstore.rs`'s tests to `blobstore/tests/mod.rs` per §4.3.
- [x] Move `repository/src/gc.rs`'s tests to `gc/tests/mod.rs` per §4.4.
- [x] Move `repository/src/package.rs`'s `mod tests` to `package/tests/mod.rs` per §4.7
      (`test_support` stays inline).
- [x] Move `repository/src/web/mod.rs`'s tests to `web/tests/mod.rs` per §4.9.
      Done (all six): `python3 -I target/p176/inline1.py` → 31, 9, 21, 17, 17, 23 tests; motion → only the module
      lines differ.
- [x] G4: tighten the ceilings for the six files.
      Done: → 6 TIGHTEN `inline` rows (14, 2, 2, 2, 53, 2); tighten drops exactly those; re-run → 0.

Acceptance: the six hosts' tests run from `tests/mod.rs` files and the counts did not move.
  Check: G1 → equal; G2 `cargo test -p mfb_repository --lib -- abi:: backfill:: blobstore:: gc:: package:: web::`
  → 0 failed, and `cargo test -p mfb_repository --lib -- --list 2>/dev/null | grep -cE '^(abi|backfill|blobstore|gc|package|web)::tests::'`
  → 31+9+21+17+17+23 = 118; G3; G4; G5; G6 over the 6 new files (largest `abi/tests/mod.rs`
  761) and the 6 hosts (est. 10 min).
  Verified 2026-10-19: G1 (`bash target/p176/ig.sh check`, full name lists in `target/p176/i0/`) → lib 450 = 450,
  bin 24 = 24, leaves SAME, clippy 21 = 21; G2 → `ok. 118 passed`; list count → 118; G3 + landing tier
  `bash scripts/test-fast.sh` → `test-fast: ok` (mfb unit `4998 passed`, which includes `spec_citations_resolve`;
  `mfb_repository` lib `450 passed`; `mfb-repo` `24 passed`); G4 → 0; G5 `cargo fmt --all --check` and
  `cargo fmt --all --check --manifest-path repository/Cargo.toml` → clean; G6 → new files 760, 543, 316, 606, 380,
  581; hosts inline 14, 2, 2, 2, 53, 2.
Commit: 76d23b173

### Phase 2 — `local`, `trust` and the binary's tests

- [x] Move `repository/src/local.rs`'s tests to `local/tests/mod.rs` per §4.5 (declaration
      stays `pub(crate)`).
- [x] Move `repository/src/trust.rs`'s tests to `trust/tests/mod.rs` per §4.8.
- [x] Move `repository/src/main.rs`'s tests to `main/tests/mod.rs` per §4.6
      (`#[cfg(test)] #[path = "main/tests/mod.rs"] mod tests;`).
      Done (all three, `python3 -I target/p176/i2.py`): 21, 16, 24 tests; `local` keeps `pub(crate) mod tests;`,
      `main.rs` declares `#[path = "main/tests/mod.rs"] mod tests;`; motion → only the module lines differ.
- [x] G4: tighten the ceilings for the three files.
      Done: → TIGHTEN `inline` 2 (local), 3 (main, its `#[path]` line counts), 2 (trust); tighten drops exactly those;
      re-run → 0.

Acceptance: `local::tests::env_guard` still serves its importers and the binary's tests run
from `main/tests/mod.rs`.
  Check: G1 → equal (library and binary); G2 `cargo test -p mfb_repository --lib -- local:: trust:: client:: server::`
  → 0 failed (client/server exercise `env_guard`), `cargo test -p mfb_repository --bin mfb-repo`
  → 24 passed; `cargo test -p mfb_repository --lib -- --list 2>/dev/null | grep -cE '^(local|trust)::tests::'`
  → 21+16 = 37, and `cargo test -p mfb_repository --bin mfb-repo -- --list 2>/dev/null | grep -cE '^tests::'`
  → 24; G3; G4; G5; G6 over the 3 new files and 3 hosts (est. 12 min).
  Verified 2026-10-19: G1 → lib 450 = 450, bin 24 = 24, leaves SAME, clippy 21 = 21; G2 → lib `ok. 225 passed`
  (local, trust, client, server), bin `ok. 24 passed`; list counts → 37 and 24; G3 + landing tier → `test-fast: ok`
  (mfb unit `4998 passed`; `mfb_repository` `450 passed`; `mfb-repo` `24 passed`); G4 → 0; G5 → both workspaces
  clean; G6 → new files 454, 517, 630; hosts inline 2, 2, 3.
Commit: 24be46317

### Phase 3 — `store`

- [x] Split `repository/src/store.rs`'s tests per §4.11 into `store/tests/mod.rs` + `accounts.rs`,
      `packages.rs`, `blobs.rs`, `transfers.rs`, `registry_tables.rs` (declaration stays
      `pub(crate)`; the 13 cross-host helpers stay in `mod.rs`; the five production-position
      cfg(test) items stay).
      Done: `python3 -I target/p176/planparts.py` reads §4.11's own per-part lists (every named item assigned once)
      → 27, 26, 10, 13, 12 tests; the 13 shared helpers stay in `mod.rs`; `items.py extract` (112 string-literal lines
      kept byte for byte); motion → only the module lines and the 5 part `mod` lines differ.
- [x] `repository/src/store/tests/accounts.rs`: `include_str!("store.rs")` →
      `include_str!("../../store.rs")`.
      Done: → 1 hit; `key_insertion_has_exactly_one_writer` → `ok. 1 passed`.
- [x] G4: tighten the ceiling for `store.rs`.
      Done: → `TIGHTEN inline 62 (ceiling 3468)` only; tighten drops it; re-run → 0.

Acceptance: store's tests and the three hosts that import its helpers pass, and the
single-writer scan still reads `store.rs`.
  Check: G1 → equal; G2 `cargo test -p mfb_repository --lib -- store:: backfill:: gc:: server::`
  → 0 failed; `cargo test -p mfb_repository --lib -- --list 2>/dev/null | grep -cE '^store::tests::'`
  → 88; `grep -c 'include_str!("../../store.rs")' repository/src/store/tests/accounts.rs` → 1
  (a wrong path fails to compile, so G1 already proves it names an existing file, and
  `key_insertion_has_exactly_one_writer` passing proves it found the one writer in `store.rs`);
  G3; G4; G5; G6 over `repository/src/store/tests/*.rs` (largest `accounts.rs` 864) (est. 10 min).
  Verified 2026-10-19: G1 → lib 450 = 450, bin 24 = 24, leaves SAME, clippy 21 = 21; G2 → `ok. 248 passed`; list
  count → 88; G3 + landing tier → `test-fast: ok` (mfb unit `4998 passed`; `mfb_repository` `450 passed`; `mfb-repo`
  `24 passed`); G4 → 0; G5 → both workspaces clean; G6 → 195, 864, 760, 394, 656, 544; store.rs inline 62.
Commit: b53744616

### Phase 4 — `client`

- [x] Split `repository/src/client.rs`'s tests per §4.10 into `client/tests/mod.rs` (shared
      helpers + the HTTP stub, mid-module `use` lines hoisted) + `transport.rs`,
      `verification.rs`, `accounts.rs`.
      Done: `planparts.py` from §4.10's lists → 38, 15, 22 tests; the stub and fixtures stay in `mod.rs`; the three
      mid-module `use` groups hoisted into one block at its top; motion → only the module lines and the 3 part `mod`
      lines differ.
- [x] G4: tighten the ceiling for `client.rs`.
      Done: → `TIGHTEN inline 2 (ceiling 2838)` only; tighten drops it; re-run → 0.

Acceptance: every client test runs against the in-process stub from its part.
  Check: G1 → equal; G2 `cargo test -p mfb_repository --lib -- client::` → 0 failed;
  `cargo test -p mfb_repository --lib -- --list 2>/dev/null | grep -cE '^client::tests::'` → 75
  (`^client::tests::(transport|verification|accounts)::` → 38, 15, 22);
  G3; G4; G5; G6 over `repository/src/client/tests/*.rs` (largest `verification.rs` 844)
  (est. 8 min).
  Verified 2026-10-19: G1 → lib 450 = 450, bin 24 = 24, leaves SAME, clippy 21 = 21; G2 → `ok. 75 passed`; list
  counts → 75 (38, 15, 22); G3 + landing tier → `test-fast: ok` (mfb unit `4998 passed`; `mfb_repository` `450
  passed`; `mfb-repo` `24 passed`); G4 → 0; G5 → both workspaces clean; G6 → 369, 797, 840, 824; client.rs inline 2.
Commit: 6485408f2

### Phase 5 — `server` (largest blast radius)

- [x] Split `repository/src/server.rs`'s tests per §4.12 into `server/tests/mod.rs` (shared
      helpers + the four harnesses) + 9 parts.
      Done: `planparts.py` from §4.12's lists → 8, 10, 8, 12, 10, 13, 21, 17, 14 tests; the 35 shared harness items stay
      in `mod.rs`; the block has no `impl` to separate; motion → only the module lines and the 9 part `mod` lines
      differ.
- [x] G4: tighten the ceiling for `server.rs`.
      Done: → `TIGHTEN inline 2 (ceiling 7487)` only; tighten drops it; re-run → 0.

Acceptance: the server suite runs from 9 parts with every count unchanged.
  Check: G1 → equal; G2 `cargo test -p mfb_repository --lib -- server::` → 0 failed;
  `cargo test -p mfb_repository --lib -- --list 2>/dev/null | grep -cE '^server::tests::'` → 113
  (per part `accounts` 8, `identity` 10, `tokens` 8, `publish` 12, `validate` 10,
  `log_routes` 13, `pages` 21, `search_limits` 17, `transfers` 14);
  G3; G4; G5; G6 over `repository/src/server/tests/*.rs` (largest `transfers.rs` 866)
  (est. 8 min).
  Verified 2026-10-19: G1 → lib 450 = 450, bin 24 = 24, leaves SAME, clippy 21 = 21; G2 → `ok. 113 passed`; list
  count → 113; G3 + landing tier → `test-fast: ok` (mfb unit `4998 passed`; `mfb_repository` `450 passed`; `mfb-repo`
  `24 passed`); G4 → 0; G5 → both workspaces clean; G6 → mod.rs 643, parts 748, 820, 698, 832, 784, 778, 823, 462,
  860; server.rs inline 2.
  Final gate: `cargo test -p mfb_repository` → exit 0 (lib `450 passed`, bin `24 passed`, `tests/s3_backend.rs` and
  the doc tests `0 passed`); the banner grep over the 12 hosts → no output; the ceiling file keeps only the three
  `ceiling code` rows of client.rs (1445), server.rs (3056) and store.rs (3431), which L owns.
Commit: 4e26a2510

## Validation Plan

- Tests: no test is added or removed. Per phase the library and binary `--list` counts are
  unchanged (G1) and the moved modules' tests pass (G2); the per-phase greps sum to
  118 + 37 + 24 + 88 + 75 + 113 = 455, every test in I's 12 files.
- Coverage check: per-file coverage is not a plan-176 gate — author, 2026-10-17. Evidence: at 7a4b09970,
  `sh scripts/coverage.sh --bins` in a detached worktree (59 min; 4989 unit tests passed), then
  `sh scripts/coverage-check.sh` exited 1. 241 `src/` files are already below the 98% per-file
  floor ("Files below 98% line coverage (GATE FAILURE)"), and overall coverage is 93.67%.
  Examples among plan-176 hosts: `src/ir/shape.rs` 93.21%,
  `src/codegen/engine/control/builder_control.rs` 75.55%, `src/debugger/values.rs` 86.66%. A
  pure move changes no executed line, and moving an inline test block only removes test lines
  from its host's denominator. The pre-existing gate failure is reported to the owner
  separately and is not a plan-176 task.
- Runtime proof: `key_insertion_has_exactly_one_writer` passing after Phase 3 proves the
  repointed `include_str!` still reads the production file.
- Doc sync: none. No spec citation, `.ai` doc or script names a moved path or a `::tests::`
  module path (§2).
- Final gate (once, after Phase 5): `cargo test -p mfb_repository` (lib, bin and
  `repository/tests/`) → 0 failed (est. 6 min);
  `cargo test --test file_size_budget 2>&1 | grep -E 'repository/src/(abi|backfill|blobstore|gc|local|main|package|trust|web|client|store|server)'`
  → no output (the `ceiling code` rows of `client.rs`, `server.rs`, `store.rs` are at
  `n == c` and print nothing until L). The full workspace suite runs once, in plan-176-V.

## Decisions

- Rule revised — owner, 2026-10-18 (replaces the 2026-10-14 "code ≤ 1000, test ≤ 250" rule):
  three kinds, non-comment non-blank LOC, scope `.rs`/`.mfb` under `src/` `packages/`
  `examples/` `repository/` `tests/`:

  | Kind | Applies to | Limit | Over the limit → |
  |---|---|---|---|
  | `code` | code LOC of a file that is not a test-only file | 1000 | split the file into modules |
  | `inline` | inline test LOC (`#[cfg(test)]` items / MFBASIC `TESTING … END TESTING`) inside a file that is not a test-only file | 250 | move the tests out into their own test-only file(s) |
  | `test` | total LOC of a test-only file (path component `tests`, `test_*.mfb`, `#![cfg(test)]`, or a `#[cfg(test)] mod x;` target and its subtree) | 1000 | split the test-only file into modules |

  A moved inline block becomes one `foo/tests/mod.rs` unless it is over 1000; then parts
  ≤ 900. No exemptions except `GENERATED` files; ceiling rows are
  `ceiling <code|inline|test> <n> <path>`; the guard warns with a banner, never fails; a new
  module is always an acceptable split.
- I repoints `include_str!("store.rs")` to the moved file's relative path
  (`include_str!("../../store.rs")` in `store/tests/accounts.rs`); plan-176-L replaces it with
  the directory read its §4.3 gives — author, 2026-10-14.
- `main.rs`'s tests go to `repository/src/main/tests/mod.rs` through `#[path]` — author,
  2026-10-14 (§3 rejected alternatives).
- Split hosts keep every shared helper, including the map's support-file harnesses, in
  `tests/mod.rs`, so I makes no visibility edit — author, 2026-10-18 (`mod.rs` is 195 / 369 /
  649 for store / client / server, under 900).
- Glob-cycle imports compile without warnings — verified (`/tmp/loc/probe2`, `/tmp/hij_probe`).
- Per-file coverage is not a plan-176 gate; the measured evidence is in §Validation Plan
  "Coverage check" — author, 2026-10-17.

## Corrections

- 2026-10-18: Rule revised by the owner: test-only files limit 1000 (was 250), inline tests 250,
  no exemptions. Dropped 0 files (all 12 hosts are still over 250 inline). Nine blocks ≤ 1000
  now move as one `tests/mod.rs` each instead of 4-5 topic files; `client`, `store` and
  `server` regroup their map topics into 3 / 5 / 9 parts ≤ 900; the map's five support files
  fold into `tests/mod.rs`, removing all 53 `pub(super)` edits; new files 121 → 29; test
  count unchanged (455); the writer test's part is now `store/tests/accounts.rs`. Store inline
  LOC re-taken with the census at `ce7a995ca` (3468, was 3476 in `groups.json`).
- 2026-10-19 (execution): `/tmp` is wiped by session restarts here, so G0 keeps the full sorted
  `--list` names and the clippy count in `target/p176/i0/` (ignored by the repository); G1
  compares counts and leaf names. Per the owner, G2/G3 also run the landing tier
  (`bash scripts/test-fast.sh`, 2026-10-18), on macOS only (2026-10-19).

## Summary

I is pure motion of the repository crate's inline tests into test-only `tests/` modules: 12
hosts, 455 tests, 0 renamed, 29 new files, 0 visibility edits. Nine hosts move their block
whole into `tests/mod.rs`; `client.rs`, `store.rs` and `server.rs` (each over the 1000
test-only limit) split into parts ≤ 900 under a `tests/mod.rs` that keeps the shared harness.
The risk is in `server.rs` (last phase), the cross-host test helpers that must stay in their
`tests/mod.rs`, and the one self-reading `include_str!` (pinned by its own test passing).
Production code, the binary's module path and every cross-module test helper path are
untouched.
