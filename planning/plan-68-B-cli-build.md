# plan-68-B: CLI + build modules

Last updated: 2026-07-27
Overall Effort (AI): large (3h–1d)   (whole plan-68 feature)
Effort (Human): large (3h–1d)
Effort (AI): medium (1h–2h)
Depends on: plan-68-A
Produces: nothing downstream; the final artifact is these files at ≥95% line
coverage (or a documented A-owned exception), i.e. this slice of the gate green.

Part **B** of plan-68. Shared context — goal, prerequisites, dependency graph,
measured populations, design, and standing requirements — lives in the overview:
[plan-68-coverage-gate.md](plan-68-coverage-gate.md). The prerequisites stated
there gate this sub-plan too (a red suite or a stale profile poisons every line
number below); re-run them before starting and again before deciding to stop.
This sub-plan also depends on **A**: it consumes A's fresh
`target/coverage/coverage.json` (for the exact uncovered line/region set per
file) and A's worklist (which `src/cli/build/*` files A has already whole-file
excepted vs. left for B). Do **not** re-litigate A's except-vs-backfill calls;
name uncovered regions against A's report, not against a guess.

## Files in scope (measured)

All covered/total figures are from the per-file gate,
`sh scripts/coverage-check.sh` (overview §2 population table); line counts from
`wc -l src/cli/build/*.rs src/cli/mod.rs`.

| File | covered/total | Class | Owner |
|---|---|---|---|
| src/cli/build/signing.rs | 52/144 | integration-only remainder | **A** (dropped here) |
| src/cli/build/test_mode.rs | 17/42 | backfill | B — Phase B1 |
| src/cli/build/packages.rs | 88/143 | backfill | B — Phase B2 |
| src/cli/build/resources.rs | 72/97 | backfill | B — Phase B3 |
| src/cli/build/native_libs.rs | 180/251 | backfill | B — Phase B4 |
| src/cli/mod.rs | 224/269 | backfill | B — Phase B5 |
| src/cli/build/mod.rs | 1024/1233 | backfill | B — Phase B6 |

### Test-module precedent (read before writing)

`src/cli/build/` carries exactly one `#[cfg(test)] mod tests` — in
`src/cli/build/mod.rs` (lines 878–2047; located via
`grep -rl "cfg(test)" src/cli/`). Because every `src/cli/build/*.rs` module is
`use super::*` and its functions are `pub(super)`/`pub(crate)`, that one module
already tests functions **defined in the sibling files** (e.g.
`signing_ident`, `classify_installed_package`, `copy_resources`,
`copy_vendor_libraries`, `emitted_link_targets`) — and llvm-cov attributes those
covered lines to the file the function is *defined* in. **New B1–B4 tests
therefore go into the existing `src/cli/build/mod.rs` `mod tests`**, reusing its
helpers (`tempfile::tempdir`, `resolved(...)`, `write_vendor_source(...)`,
`write_executable_project(...)`, `write_package_project(...)`, `vendor_locator`).
`src/cli/mod.rs` has its own `mod tests` (lines 218–459) with `EnvVarGuard` /
`ENV_LOCK` / `tempfile` helpers — Phase B5 extends that one.

> **Standing rule (overview §4):** run the full `cargo test`, never a single
> module; a coverage test that surfaces a real bug is fixed on its own RED-first
> commit (`write-bug`), not worked around; never edit a golden to pass.

## Phase B0 — signing.rs is A's exception, not B's backfill

No task. Recorded so the executor does not chase it: `src/cli/build/signing.rs`'s
52 covered lines are its pure helpers — `signing_ident` (tested:
`signing_ident_defaults_to_owner_hash_name`, `..._rejects_bad_idents`),
`apply_signing_metadata` (`apply_signing_metadata_copies_fields`), and
`executable_signing_metadata_json` (`executable_signing_metadata_json_is_valid_json`),
all already in `src/cli/build/mod.rs mod tests`. The **92 uncovered lines are
entirely `load_build_signing_info` (signing.rs:47–158)**, which calls
`mfb_repository::client::request_attestation(...)` (signing.rs:70) against a live
HTTP registry AND requires a machine-registered ident key — the exact boundary
the now-stale `src/cli/build.rs` exception named ("load_build_signing_info
requests attestation from a live registry"). Its in-source `// coverage:off`
(signing.rs:44) is **not** honored by cargo-llvm-cov 0.8.7
(`scripts/coverage-exceptions.txt:7`), so the file can never reach 95% by unit
test. → **A repoints the exception onto `src/cli/build/signing.rs`** (plan-68-A
Phase A2). B writes nothing here. If A's worklist instead left signing.rs for B,
STOP and reconcile with A — do not whole-file-except a coverable body, and do not
unit-test a live-registry body.

## Phase B1 — test_mode.rs (17/42)

`src/cli/build/test_mode.rs` has three functions (read in full). None reaches the
network; all three are directly unit-coverable without the build pipeline. The
17 covered lines come from the `mfb test` host path
(`mfb_test_host_run_leaves_project_build_dir_untouched`, which calls
`make_temp_output_dir`→`run_test_binary` success arm). The 25 uncovered lines are
the error/warning arms + `generate_coverage_report`'s body. Add to
`src/cli/build/mod.rs mod tests`:

- [ ] `run_test_binary` (test_mode.rs:56–65) — exercise all three arms directly,
      no build needed: `run_test_binary(Path::new("/usr/bin/true"))` → `Ok(())`;
      `run_test_binary(Path::new("/usr/bin/false"))` → `Err(())` (the
      `Ok(_) => Err(())` arm); `run_test_binary(Path::new("/no/such/binary"))` →
      `Err(())` (the `Err(err)` spawn-failure arm, printing "failed to run"). Gate
      the exact paths on `#[cfg(unix)]` (mirrors the file's other unix tests).
- [ ] `make_temp_output_dir` (test_mode.rs:30–52) — call it, assert the returned
      `PathBuf` `is_dir()` and its file name starts with `mfb-test-`, then
      `remove_dir_all` it. Covers the `Ok(())=>return Ok(dir)` success arm and the
      loop entry. (The `AlreadyExists` retry and the terminal `Err(())` require a
      pre-planted collision on an unpredictable pid+nanos path — leave those two
      arms; they are a minority and non-deterministic to force.)
- [ ] `generate_coverage_report` (test_mode.rs:6–22) — two tests over a
      `tempfile::tempdir`: (a) an empty dir → hits the
      `read_covmap(...) is None` arm, asserting the "coverage map missing" warning
      path returns without writing `coverage.html`; (b) a dir seeded with a real
      `COVMAP_FILE` (write one via `crate::testing::coverage::write_covmap` with a
      small slot set, as the coverage-mode build does) → reaches
      `generate_html` + the `Ok(())` write arm, asserting `COVERAGE_HTML` now
      exists. If seeding a valid covmap is impractical, cover (a) here and note
      the `generate_html`/write tail is exercised by the `--coverage` build tests
      in Phase B6 (`options.mode.coverage()` path at mod.rs:654–655).

Acceptance: `sh scripts/coverage-check.sh src/cli/build/test_mode.rs` shows ≥95%
(requires a fresh `sh scripts/coverage.sh` first).
Commit: —

## Phase B2 — packages.rs (88/143)

`src/cli/build/packages.rs` (read in full). Covered already: `PackageVerification`
+ `::label` (`package_verification_labels`), `verify_and_report_packages`
(four tests, packages.rs:59–131), `source_is_local`
(`source_is_local_classifies_sources`), and the *early* `classify_installed_package`
arms (`..._reads_unsigned_fixture`, `..._treats_missing_file_as_tampered`,
`..._treats_garbage_as_tampered`) + `decode_trust_anchor`
(`decode_trust_anchor_accepts_metadata_key_form`).

**Key property (READ-verified): `classify_installed_package` (packages.rs:149–238)
performs no network I/O** — after `signature_type != 0` it does pure crypto over
the `.mfp` bytes plus local key files (`read_pinned_server_key`,
`verify_attestation`/`verify_proof`/`verify_package_signature`/`verify_payload_hash`).
So the **55 uncovered lines are the signed-package §3.5 chain and its Tampered
arms** — all unit-coverable with a *signed* fixture (the current tests use only an
unsigned one). Signed `.mfp` fixtures exist under
`tests/syntax/security/pkg-0*/packages/*.mfp` (e.g.
`pkg-01-tampered-signature/packages/sec_signed.mfp`; `find tests -name '*.mfp'`
→ 114 files). Add to `src/cli/build/mod.rs mod tests` (confirm each fixture's
`signature_type` and expected refusal against A's fresh region list):

- [ ] Signed pkg + `trust_anchor = None` → `Tampered`,
      `PACKAGE_IDENT_KEY_UNTRUSTED` "pins no identKey" (packages.rs:172–177).
- [ ] Signed pkg + a malformed `trust_anchor` (e.g. `"not-base64!"`) →
      `Tampered`, `PACKAGE_IDENT_KEY_UNTRUSTED` "malformed" (packages.rs:178–186).
- [ ] Signed pkg + a well-formed-but-wrong `trust_anchor` (a valid ed25519 key
      that is not the package's) → `Tampered`, header-≠-pinned arm
      (packages.rs:197–202).
- [ ] Signed pkg + the *correct* pinned `trust_anchor` but **no pinned server
      key** on the (hermetic `MFB_HOME`) machine → `Tampered`,
      `PACKAGE_ATTESTATION_INVALID` "no pinned registry key" (packages.rs:210–218).
      Use the `EnvVarGuard`/`ENV_LOCK` pattern from `src/cli/mod.rs mod tests` to
      point `MFB_HOME` at an empty tempdir so `read_pinned_server_key` errs
      deterministically. This is the reachable frontier without a registry;
      whether the `verify_attestation`/`verify_proof`/`verify_package_signature`/
      `verify_payload_hash` Tampered arms (packages.rs:220–236) and the terminal
      `Verified` arm (237) are reachable depends on whether the security fixtures
      ship a matching pinned `server.pub` + ident — decide from A's report which
      of those lines remain and whether a fixture reaches them; a
      known-tampered fixture (pkg-01) with the correct anchor + pinned key should
      land on exactly one deeper arm (e.g. `PACKAGE_SIGNATURE_INVALID`).

Acceptance: `sh scripts/coverage-check.sh src/cli/build/packages.rs` shows ≥95%
(fresh `sh scripts/coverage.sh` first).
Commit: —

## Phase B3 — resources.rs (72/97)

`src/cli/build/resources.rs` (read in full): `resource_src_fixed_prefix`,
`collect_files_recursive`, `copy_resources`. Covered: the fixed-prefix table
(`resource_src_fixed_prefix_splits_at_first_glob`), the worked examples
(`copy_resources_maps_the_worked_examples`), and the escaping-symlink refusal
(`copy_resources_refuses_a_source_that_resolves_outside_the_project`). The 25
uncovered lines are the remaining `copy_resources` branches. Add to
`src/cli/build/mod.rs mod tests`:

- [ ] Root-level glob (`resource_src_fixed_prefix` empty) → exercises the
      `walk_root = project_root` branch (resources.rs:64–66) and the
      `dest_relative = rel` empty-prefix branch (114–116): a `src: "*.png"` entry
      copying a project-root file to `<dst>/`. The current worked-examples test
      uses only prefixed globs, so this branch is uncovered.
- [ ] Absent fixed-prefix directory → the `!walk_root.is_dir() { continue }`
      no-op (resources.rs:70–72): an entry whose prefix dir does not exist copies
      nothing and is not an error. (The worked-examples "nowhere/*.dat" case
      matches nothing but its parent may still be absent — confirm from the report
      whether line 71 is already hit; if so, drop this task.)
- [ ] A file present under the walk root that does **not** match the `src` glob →
      the `glob_matches ... { continue }` skip (resources.rs:110–112): seed a
      `data/keep.txt` beside a matched `data/x.ogg` with `src: "data/*.ogg"`,
      assert only the `.ogg` is copied.
- [ ] Multi-level recursion in `collect_files_recursive` (resources.rs:35–46):
      the worked-examples test already nests `data/loops/`; if the report shows
      the `is_dir()` recursion arm still uncovered, add a deeper `a/b/c/` tree.

The canonicalize-failure arms (resources.rs:77–88) and the `fs::copy` /
`create_dir_all` error arms (122–132) require an unreadable/unwritable path; if
the report shows them still red after the above, attempt one
`#[cfg(unix)]`-gated `0o000`-permission case, else leave them (a minority
filesystem-error tail) and confirm ≥95% is still met without them.

Acceptance: `sh scripts/coverage-check.sh src/cli/build/resources.rs` shows ≥95%
(fresh `sh scripts/coverage.sh` first).
Commit: —

## Phase B4 — native_libs.rs (180/251)

`src/cli/build/native_libs.rs` (read in full). Covered: `emitted_link_targets`
(`emitted_link_targets_track_what_each_build_mode_actually_emits`),
`vendor_output_dirs` (`vendor_output_dirs_match_the_emitted_rpath_per_shape`),
`resource_output_dirs` (`resource_output_dir_per_build_shape`),
`copy_vendor_libraries` (six tests, native_libs.rs:319–384). The 71 uncovered
lines are the assembly/verify helpers, all pure over a tempdir + crafted
IR/manifest (no network). Add to `src/cli/build/mod.rs mod tests`, reusing
`resolved(...)`, `write_vendor_source(...)`, `vendor_locator(...)`:

- [ ] `verify_vendor_libraries` (native_libs.rs:137–193) — three arms over
      tempdir fixtures: (a) missing vendor file → `false` +
      `NATIVE_LIBRARY_FILE_MISSING` (147–160); (b) a `ResolvedLibrary` whose
      `locator.hash` is `None` → `false` + `NATIVE_LIBRARY_HASH_MISMATCH`
      "records no hash" (164–175); (c) a file whose bytes hash ≠ the recorded
      `hash` → `false` + the "does not match the sha256" arm (176–190); and (d) a
      matching hash → `true`. Use `crate::manifest::libraries::sha256_file` to
      compute the expected hash for the happy case.
- [ ] `vendor_source_path` (native_libs.rs:114–128) — both arms: a library whose
      `declaring_unit == own_unit` → `vendor/<source>`; one whose
      `declaring_unit != own_unit` → `packages/<unit>.vendor/<source>`. (The
      copy_vendor tests exercise this indirectly; add a direct assertion if the
      report shows either arm red.)
- [ ] `assemble_native_library_table` (native_libs.rs:10–28) — its findings loop
      and `rules::is_error` gate: a manifest whose `LINK`/`libraries` produce a
      finding of `Error` severity → `None` (ok=false); one that produces only
      warnings/no findings → `Some(table)`. Drive via
      `crate::manifest::libraries::build_native_library_table` inputs a crafted
      `HashMap`/`IrProject` reaches.
- [ ] `resolved_vendor_libraries` (native_libs.rs:75–106) — the
      `linked.is_empty() → Ok(Vec::new())` early return (a project with no `LINK`
      names) and the dedup-by-`dlopen_name` accumulation across
      `emitted_link_targets`. A no-LINK IR gives the empty case; a two-flavor
      Linux target with one shared `system` locator exercises the dedup.
- [ ] `assemble_native_libraries` / `assemble_native_libraries_for_ir`
      (native_libs.rs:388–418) — the thin `Some(table)=>true` /`None=>false`
      wrappers: one manifest that assembles cleanly (→ true, table stamped into
      `metadata`/`ir`) and one with an `Error` finding (→ false). These are also
      hit by the Phase B6 `build_project` failure branches; if the report shows
      them covered after B6, drop this task.

Acceptance: `sh scripts/coverage-check.sh src/cli/build/native_libs.rs` shows
≥95% (fresh `sh scripts/coverage.sh` first).
Commit: —

## Phase B5 — cli/mod.rs (224/269)

`src/cli/mod.rs` (read in full) has its own `mod tests` (lines 218–459). Covered:
`answer_is_yes`, `confirm` (three tests), `local_paths_for_repo` (four tests),
`stage_bytes`/`stage_package_blob`/`commit_staged_package`/
`install_verified_package` (seven tests). The 45 uncovered lines are:

- [ ] `install_vendor_file` (cli/mod.rs:129–145) — **no test today** (its only
      caller is the excepted `src/cli/pkg.rs:1353`), ~12 lines. Add to
      `src/cli/mod.rs mod tests`, mirroring the `stage_package_blob` symlink test:
      (a) success — `install_vendor_file(dir, "libfoo.so", bytes)` into a fresh
      dir returns the destination and the bytes land there via the stage→rename;
      (b) it never writes through a pre-planted symlink at `dir/filename` (the
      whole reason it stages under a `.part` name), asserting a victim target is
      untouched and the final file is not a symlink; (c) the `create_dir_all`
      error arm (130–135) — pass a `dir` whose path component is an existing
      regular file, asserting the "failed to create" error.
- [ ] `stage_bytes` write/sync-failure arm (cli/mod.rs:56–61) — the
      `write_all/sync_all` error branch that `remove_file`s the partial. Force it
      by staging where the exclusive open succeeds but the write cannot complete
      (e.g. a `staged` path that is itself a directory is rejected earlier by
      `create_new`; instead target a filesystem/`O_EXCL` case the report confirms
      hits 56–61) — if this arm proves impractical to force deterministically,
      leave it and confirm ≥95% is still met.
- [ ] `confirm` non-`assume_yes` I/O arms (cli/mod.rs:203–215) — the flush and
      `read_line` paths run only under a real TTY, which the test harness lacks
      (stdin is non-interactive, already asserted by
      `confirm_refuses_to_prompt_without_a_terminal`). These few lines are a TTY
      boundary; do **not** chase them — confirm from the report they are the only
      residual besides `dispatch_command_error`.

**Note on `dispatch_command_error` (cli/mod.rs:29–40):** it is `-> !` and calls
`std::process::exit(2)`/`exit(1)`; its only callers are the excepted
`src/cli/dispatch.rs` (six sites). Its ~6 lines cannot be reached in-process
without terminating the test runner. That is fine: `269 × 0.95 = 256`, so with
`install_vendor_file` + `stage_bytes` covered, cli/mod.rs clears 95% **with
`dispatch_command_error` (and the TTY arms) left uncovered** — no exception
needed. Verify this arithmetic against A's fresh report; only if the file still
falls short after the coverable helpers are done does the process-exit boundary
become an A-owned exception (flag it back to A, do not except it from B).

Acceptance: `sh scripts/coverage-check.sh src/cli/mod.rs` shows ≥95% (the filter
substring `src/cli/mod.rs` does not match `src/cli/build/mod.rs`; fresh
`sh scripts/coverage.sh` first).
Commit: —

## Phase B6 — build/mod.rs (1024/1233)

`src/cli/build/mod.rs` `build_project` (lines 163–860) is the build orchestrator
and is already 83% covered by the existing host-build integration tests
(`build_project_builds_a_host_executable`, `..._builds_a_package`,
`..._writes_ast_and_ir_dumps`, `mfb_test_host_run_...`, etc., which run real
codegen in tempdirs on the macOS host). The 209 uncovered lines are almost all
**reachable error/validation branches**, coverable on-host by feeding
`build_project` a crafted project (extend `write_executable_project` /
`write_package_project`). Name each target against A's region report; the
READ-verified candidates are:

- [ ] `app` package imported without app mode (mod.rs:265–277) → the
      "requires app mode" error: a console project whose source `IMPORT app`.
- [ ] App-mode icon missing (mod.rs:220–236) → `PROJECT_JSON_ICON_MISSING`: an
      `--app` executable (host target supports MacApp) with `"icon"` pointing at a
      nonexistent file.
- [ ] `EXPORT` in an executable (mod.rs:414–417) →
      `export_in_executable_diagnostics` error, via a top-level `EXPORT` in a
      non-package project.
- [ ] `validate_expect_placement` reject (mod.rs:280–282) → an assertion builtin
      outside a `TCASE` body.
- [ ] `--sign` with output flags (mod.rs:450–455) → the "only supported for
      package and executable builds" reject (`sign_owner = Some` while
      `outputs` non-empty, e.g. `--sign owner --ast`). (The `Some(owner)` +
      empty-outputs happy arm calls `load_build_signing_info` → live registry;
      leave it — that call is the signing.rs boundary A excepts.)
- [ ] Native-library assembly failure on the executable path
      (mod.rs:481–483) and the package path (mod.rs:694–696) → a project with a
      `LINK` naming a library absent from `libraries` (an `Error` finding) returns
      `Err(())`. Also drives Phase B4's `assemble_native_libraries*` wrappers.
- [ ] Vendor hash-verify failure (mod.rs:529–531) → an executable whose resolved
      `vendor` blob mismatches its recorded sha256 (reuse the hash-mismatch
      fixture shape from B4) aborts before codegen.
- [ ] Unknown project `kind` (mod.rs:713–725) → the "Validated MFBASIC project"
      no-op arm: a `"kind": "program"` manifest (warned, not errored) builds
      nothing and returns `Ok(())`. (bug-300 E8 confirmed this arm is live.)
- [ ] Artifact-dump branches: `-br` (`BinaryRepr`, mod.rs:812–830) and the
      package-rejects-native-output arm — the latter is covered
      (`build_project_rejects_native_output_for_a_package`); confirm from the
      report which of `-br` / the five native writers (`--nir/--nplan/--nobj/
      --ncode/--mir`, mod.rs:839–853) and the assemble-for-ir failure in the dump
      path (mod.rs:804–806) are still red, and add a minimal host-target dump test
      per uncovered writer.
- [ ] Reporter verbosity paths (mod.rs:62–77): run one build at `Verbosity::Verbose`
      so `Reporter::phase`/`summary` non-quiet arms execute, if the report shows
      them red.

Genuinely-uncovered-and-integration-only residue to **leave** (a minority; not
B's to chase): the `load_build_signing_info` call (live registry, A's boundary),
the cross-`-target` `LinuxApp`/`WindowsApp` `write_executable`/`vendor_copies`
paths (mod.rs:568–592, 660–664 — the host cannot run a cross linker), the
`remove_dir_all` clear-error arm (mod.rs:511–516), and the `run_test_binary`
`None` arm (mod.rs:649–652). Confirm from the report that covering the reachable
error/validation branches above clears `1233 × 0.95 = 1171` covered; if not, the
specific still-red cross-target codegen lines are named back to A as a
subprocess/cross-linker boundary — B does not except them unilaterally.

Acceptance: `sh scripts/coverage-check.sh src/cli/build/mod.rs` shows ≥95% (fresh
`sh scripts/coverage.sh` first).
Commit: —

## Validation Plan

- **Per-phase:** `sh scripts/coverage-check.sh <path>` (path-substring filter,
  reuses the cached profile) shows the phase's file ≥95%. Requires a fresh
  `sh scripts/coverage.sh` first — the checker reads the profile that run leaves;
  a stale profile reports phantom gaps (overview prereqs).
- **Whole sub-plan:** `sh scripts/coverage-check.sh src/cli/` shows every B file
  ≥95% (or, for signing.rs, moved to "Documented exceptions" by A). Cross-check
  none of the seven still appears under GATE FAILURE.
- **Suite:** `cargo test` → `0 failed`. New tests must not regress the suite; run
  the whole suite, not just `cli::build::tests` (overview §4).
- **No production change:** the diff touches only `#[cfg(test)]` modules under
  `src/cli/`. If a coverage test surfaces a real bug in `build_project` or a
  helper, fix it on its own RED-first commit per `write-bug`, not by weakening the
  test.
- **Interaction with A:** signing.rs must be an A exception (Phase B0); confirm it
  is present in `scripts/coverage-exceptions.txt` at `src/cli/build/signing.rs`
  before claiming B done, else the whole-`src/cli/` check will still be red on it.

## Corrections

<Filled in during execution — especially any file whose reachable branches turn
out NOT to clear 95% (→ named boundary handed back to A), any candidate branch
A's fresh report shows already covered, and any real bug a coverage test exposes.>
