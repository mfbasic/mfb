# bug-344: test/tooling infrastructure cleanup — 16 copies of one fixture helper, an agent-facing doc that mandates a fixture layout that no longer exists, and a fast gate that has drifted 11 artifact kinds behind the real harness

Last updated: 2026-07-18
Effort: medium (1h–2h per item; large for the cluster)
Severity: LOW
Class: Other (cleanup)

Status: Open
Regression Test: the harness itself — `scripts/test-accept.sh` and
`scripts/artifact-gate.sh` must be green before and after; new coverage added
only where an item deletes a duplicate (B1, C1, C3).

A cluster of duplication, layout drift, and stale-documentation residue across
`tests/`, `scripts/`, `benchmark/`, `planning/`, and the repo's agent-facing
tooling docs. Every claim below was re-measured against the current worktree;
several leads from the original review were **overstated or wrong** and have
been corrected in place or dropped (see the per-item notes and "Leads that did
not verify").

Nothing in this bug changes shipped compiler output. Two items are nonetheless
worth reading first, because they cause *ongoing* damage rather than sitting
still:

- **`.ai/compiler.md` orders agents to create fixtures in the pre-reorg flat
  layout** — directories that do not exist — and `AGENTS.md` points at that file
  for all compiler work. It actively produces misfiled fixtures (A1).
- **`scripts/artifact-gate.sh` re-implements four pieces of
  `scripts/test-accept.sh` and has already drifted**, checking 8 artifact kinds
  where the real harness compares 19 (C1). A "fast gate green" result is weaker
  than it reads.

The single correct outcome of a fix is that fixture layout is uniform and
documented accurately, each test helper and each script fragment has exactly one
definition, and the harness's own accept/gate runs are unchanged.

References:

- Found during the tree-wide cleanup review (Agent 21 — tests/scripts/tooling,
  plus Agent 22 items 7 and 8), base `25c38ba1`.
- `AGENTS.md:33-39` (MCP mandate), `:59-60` (man-page driver scripts as the
  authoritative rules home).
- `.ai/compiler.md` — the agent-facing compiler workflow doc.
- `scripts/test-accept.sh`, `scripts/artifact-gate.sh`, `scripts/sync-goldens.sh`.
- Memory note: acceptance golden-harness mechanics (`sync-goldens.sh <exe>
  <name-glob>` is filter-aware and ~4s).

### Broken gates — cross-referenced, NOT in scope here

Two findings from the same review are **correctness** defects in the test
infrastructure, not cleanup, and each needs its own bug:

- **`scripts/test-macapp.sh` builds bundles at a path the compiler stopped
  writing.** Nine sites use `$proj/<name>.app`; the compiler writes
  `$proj/build/<name>.app` (`src/os/mod.rs:15`, `BUILD_DIR = "build"`).
  `run_headless` execs a missing path. The sibling `scripts/test-appimage.sh`
  *was* updated. `.ai/compiler.md` designates this the mandatory macOS app-mode
  proof, so that gate has been silently red since plan-46-D.
- **`repository/` is a separate workspace, so its ~13k lines of tests never run
  in CI.** `cargo metadata --no-deps` reports members `['mfb']` only, while
  `.github/workflows/coverage.yml` enforces a global floor those files are
  invisible to; `tests/repo_acceptance.rs` works around it by shelling out to
  `cargo build --manifest-path` from inside a test.

Item C2 below cleans up `test-macapp.sh`'s *structure*; it must land **after**
the path fix, not instead of it.

## Current State

Measured (worktree `cleanup-review`, base `25c38ba1`):

| Measure | Value |
| --- | --- |
| Fixture dirs with `project.json` | 994 |
| …at `<bucket>/<feature>/<name>` | 989 |
| …at `<bucket>/<name>` (misfiled) | 4 |
| …other (`tests/acceptance`, directly under `tests/`) | 1 |
| Fixture dirs with `golden/` | 979 |
| Fixtures with **no** `golden/` (intentional behavioral runs) | 15 |
| Goldens lacking a `project.json` (orphans) | **0** |
| Golden files total | 2,684 |
| `tests/*.rs` files defining `fn temp_project` | 16 |
| Artifact kinds compared by `test-accept.sh` | 19 |
| Artifact kinds compared by `artifact-gate.sh` | 8 |
| `src/` files with an inline `mod tests` | 118 |
| Sibling `src/**/tests.rs` files (total lines) | 12 (26,686) |
| Test subdirectories under `src/` | 0 |

### Verified with NO action required — recorded so it is not re-derived

**The golden/fixture orphan scan is CLEAN.** Re-measured, all six numbers
confirmed: **994** directories contain a `project.json`; **979** contain a
`golden/`; **zero** goldens exist without a corresponding `project.json`; the
**15** fixtures with no `golden/` are all intentional, per
`scripts/test-accept.sh:166-183` ("A test with no golden/ directory is a
behavioral (acceptance) test: run `mfb test` and require exit 0 … Nothing is
compared."); and there are **2,684** golden files in total.

There is no orphaned-golden problem in this repository. Do not re-run this scan.

## Items

### Theme A — fixture layout and harness conventions

#### A1 — `.ai/compiler.md` orders agents to create fixtures in the pre-reorg flat layout, and `AGENTS.md` points every compiler task at it

- `.ai/compiler.md:33` and `:38` mandate fixtures at
  `tests/func_<pkg>_<func>_{valid,invalid}/**`. **No such directories exist.**
- The real layout, enforced by `scripts/test-accept.sh:142-148`, is
  `tests/{syntax,rt-error,rt-behavior}/<feature>/<name>` — 989 of 994 fixtures
  follow it.
- This is the highest-value item in the bug precisely because it is not inert: it
  is the document `AGENTS.md` designates for all compiler work, so every agent
  that follows it creates a misfiled fixture, which then has to be found and
  moved by hand.
- Fix: rewrite `:33` and `:38` to the four-folder layout, with one worked
  example per bucket. Do this **first**.

#### A2 — four `rt-error` fixtures are compile-time diagnostics sitting at the bucket root

- `tests/rt-error/parser_statement_block_depth/`,
  `tests/rt-error/parser_tgroup_depth/`,
  `tests/rt-error/parser_type_name_depth/`,
  `tests/rt-error/monomorph_polymorphic_recursion_depth/`.
- These are exactly the 4 of 994 fixtures at `<bucket>/<name>` rather than
  `<bucket>/<feature>/<name>`.
- Independent corroboration that they are misfiled by *kind*, not just by depth:
  **128** `rt-error` fixtures carry a `.run` golden, and these same 4 are the only
  ones whose `golden/` contains **only** `build.log` — i.e. they never execute.
  They are compile-time diagnostics and belong under `tests/syntax/`.
- Fix: move all four to `tests/syntax/<feature>/<name>`. **Moving a fixture
  changes its harness path**, so this item requires a `scripts/sync-goldens.sh`
  run — see Fix Design.

#### A3 — nine `tests/syntax/` fixtures carry a `.run` golden, and the reason is documented in prose the harness cannot enforce

- 522 syntax fixtures; **9** have a `.run` golden, 513 do not. The nine:
  `native/native-link-const-cint32-overflow-invalid`,
  `native/native-library-vendor-missing-invalid`,
  `native/native-library-missing-invalid`,
  `security/pkg-02-type-confusion`, `security/pkg-02b-computed-confusion`,
  `security/pkg-02c-operator-confusion`, `security/pkg-03-decode-depth`,
  `security/pkg-07-need-overflow`, `json/json_read_invalid`.
- `tests/rt-behavior/security/README.md:23-25` explains the security ones: "PKG-03
  and PKG-07 only trip on the full merge (after `-ast -ir` succeeds via the lossy
  external-type path), so those two carry a `.run` trigger plus `.ast`/`.ir`
  goldens".
- So `.run` is being used as a **merge trigger**, not as an execution proof — a
  load-bearing distinction that exists only in a README. Nothing in the harness
  encodes it, and nothing stops a future reader from concluding these nine
  fixtures execute.
- Fix: encode the distinction where the harness can see it — a marker file, or a
  documented naming convention checked by `test-accept.sh` — and cross-reference
  it from the README rather than the reverse.

### Theme B — duplicated test scaffolding

#### B1 — the same `temp_project` helper and inline manifest JSON, 16 times over, because `src/testutil.rs` is unreachable from integration tests

- **16** `tests/*.rs` files define `fn temp_project`:
  `build_verbosity_output.rs:20`, `fs_create_mode_0600.rs:20`,
  `fs_atomic_int_return.rs:39`, `entry_args.rs:39`, `linux_app_mode.rs:16`,
  `linux_pie_headers.rs:21`, `fs_error_path_hygiene.rs:51`,
  `macos_rodata_readonly.rs:32`, `linux_rodata_readonly.rs:29`,
  `macos_app_io_input_imports.rs:35`, `native_numeric_pow_div_runtime.rs:24`,
  `native_loop_runtime.rs:13`, `native_float_pow_operator_runtime.rs:15`,
  `tls_listen_accept_build.rs:31`, `syscall_return_robustness.rs:112`,
  `native_io_runtime.rs:7`.
- **Correction**: the review said 17 files with 14 byte-identical bodies. The
  measured shape is 16 files whose bodies hash into groups of
  **5 + 4 + 4 + 1 + 1 + 1** — 13 in identical groups, not 14 in one. The two
  largest groups differ *only* by `std::env::temp_dir()` vs `env::temp_dir()`.
- The cause is real and is the thing to fix: `src/testutil.rs` exists but carries
  `#![cfg(test)]` at **`:8`** (line 1 is a doc comment), and `mod testutil` is
  declared only in `src/main.rs:27` — so integration tests genuinely cannot reach
  it.
- The same manifest JSON is duplicated in shell too: `scripts/test-macapp.sh`
  (9 `project.json` heredocs), `scripts/test-appimage.sh`,
  `scripts/check-net-connect-timeout.sh`.
- Fix: `tests/common/mod.rs` with one `temp_project`; delete the 16 copies. Do
  **not** try to make `src/testutil.rs` importable — that changes the crate's
  test configuration for a convenience.

#### B2 — three duplicated integration helpers, two of which have diverged by accident

- `build_ncode` — byte-identical (same md5) in three files:
  `tests/fs_atomic_int_return.rs:57`, `tests/fs_error_path_hygiene.rs:65`,
  `tests/syscall_return_robustness.rs:128`. (Three *other* `build_ncode`s exist
  with different signatures — `tests/entry_args.rs:58`,
  `tests/gtk_term_utf8_grid.rs:33`, `tests/native_size_arith_overflow.rs:45` —
  and are a separate naming problem, not copies.)
- `run_bounded` — `tests/native_numeric_pow_div_runtime.rs:65` vs
  `tests/native_float_pow_operator_runtime.rs:55` diverge in exactly two spots:
  the panic text ("bug-61 hang reintroduced" vs "bug-135 `^` linear loop was
  reintroduced") and the poll interval,
  `sleep(Duration::from_millis(25))` vs `from_millis(20)`. The panic text is a
  deliberate difference; **the 25ms/20ms split is accidental** and is the kind of
  divergence that makes a flaky test look environment-dependent.
- `build_linux_elf` — `tests/linux_pie_headers.rs:39` vs
  `tests/linux_rodata_readonly.rs:47` differ only by a 2-line comment ("Console
  builds emit one flavored executable per libc world…") present in the former.
- Fix: move all three into B1's `tests/common/mod.rs`; parameterize
  `run_bounded`'s panic message; pick one poll interval and say why.

#### B3 — the coverage ignore-regex is copied verbatim in three places that must agree

- `scripts/coverage.sh:19`, `scripts/coverage-check.sh:15`, and
  `.github/workflows/coverage.yml:21` all carry, byte-identically:
  ```
  (^|/)(target|tests)/|repository/target/|_runtime_tables\.rs$|/code/private/unicode\.rs$|/src/testutil\.rs$
  ```
- Only `scripts/coverage.sh:9-14` carries the explanatory comment block. If any
  one copy is edited, the three gates silently measure different denominators.
- Fix: one file (e.g. `scripts/coverage-ignore.txt`) read by all three, or have
  the workflow invoke `coverage.sh` rather than restate its regex.

### Theme C — script duplication and drift

#### C1 — `artifact-gate.sh` re-implements `test-accept.sh` and has drifted 11 artifact kinds behind it

- `scripts/artifact-gate.sh:31` handles **8** kinds: `ast`, `ir`, `hex`, `nir`,
  `nplan`, `nobj`, `ncode`, `mir`.
- `scripts/test-accept.sh:380-434` compares **19**: `build.log`, `testrun`,
  `covmap.json`, `covdata`, `covfail`, `audit`, `ast`, `ir`, `hex`, **`mfp`**
  (`:405`), **`info`** (`:408`), `nir`, `nplan`, `nobj`, `ncode`, `mir`, and
  **`app.nir` / `app.nplan` / `app.ncode`** (`:426-434`).
- **Correction — the drift is worse than the review claimed.** Missing from the
  fast gate: `mfp`, `info`, `app.*` (the three predicted) **plus** `testrun`,
  `covmap.json`, `covdata`, `covfail`, and `audit`.
- The `-q` divergence is confirmed: `scripts/test-accept.sh:239` runs
  `"$MFB_EXE" build -q …`; `scripts/artifact-gate.sh:29` has no `-q`.
- Consequence: "artifact-gate green" is a materially weaker statement than most
  callers assume, and the two scripts must be hand-synced forever.
- Fix: extract the shared discovery + compare loop into a sourced fragment both
  scripts use, with the artifact table defined **once** (see C3). Until then, at
  minimum bring the gate's table to parity and add the `-q`.

#### C2 — `test-macapp.sh` never adopted the helper structure its Linux sibling has

- `scripts/test-macapp.sh` = 385 lines; `scripts/test-appimage.sh` = 388.
- macapp has **9** `project.json` heredocs (`:65, 91, 119, 150, 194, 233, 270,
  308, 348`; 17 heredocs total counting the `MFB` source ones), **16**
  open-coded `failures + 1` increments with **no `fail()` helper** (the review
  said 12), and **5 inlined perl watchdogs whose timeouts have diverged**:
  `alarm 15` at `:45`, `:58`, `:216`, `:253`, but `alarm 10` at `:329`.
- The sibling already solved every one of these: `scripts/test-appimage.sh:79-80`
  (`pass()` / `fail()`), `:86-104` (`box_run` / `timeout_run` with a
  **parameterized** `alarm $limit`), `:107-122` (`build_appimage`), and 20
  `fail "…"` call sites.
- Related: bug-320 proposes copying the watchdog into a **third** script — which
  is the argument for extracting it once instead.
- Fix: adopt the sibling's helpers. **Land only after the broken-path fix** (see
  "Broken gates" above) — restructuring a script that cannot currently pass makes
  the path fix unreviewable.

#### C3 — `test-accept.sh`'s artifact plumbing is hand-written across five regions

- The script is **447** lines. Adding one artifact kind means editing:
  1. `:185-205` — 18 path variables (17 artifact + `log_path`)
  2. `:207` — a single `rm -f` line listing all 17 paths
  3. `:213-232` — 6 golden-probing `if` blocks that build `console_flags`
  4. `:284-323` — **12** `mv` blocks
  5. `:380-434` — **19** compare calls (1 `compare_file` + 18
     `compare_optional_output`)
- **Correction**: the review described 4 regions with 11 `mv` blocks and 14
  compare calls. It is 5 regions, 12, and 19 — and region 3 was missed entirely.
- This is exactly the region `artifact-gate.sh` must mirror (C1), which is why
  the drift happened.
- Fix: one artifact table (name, extension, target-infix flag, optional/required)
  driving all five regions, shared with the fast gate.

#### C4 — `update_man.sh` and `update_man_package.sh` disagree about where a package's source lives, and share ~50 lines of prompt

- `scripts/update_man.sh:92` unconditionally instructs the model to
  "Read src/builtins/${module}.rs".
- `scripts/update_man_package.sh:31-42` instead **probes**
  (`[[ -f "src/builtins/${pkg}.rs" ]]`), adds `_package.mfb`, and special-cases
  `filters` → `general.rs`.
- **Correction — the impact is much smaller than claimed.** The review said the
  unconditional path is wrong for `filters`, `csv`, `encoding`, `http`, `money`,
  and `audio`. Measured: `csv.rs`, `encoding.rs`, `http.rs`, `money.rs`, and
  `audio.rs` **all exist**. Only **`filters`** lacks
  `src/builtins/filters.rs`. Bad-path count is 1, not 6.
- The divergence still matters because `AGENTS.md:59-60` designates these driver
  scripts the authoritative rules home for man-page work — so two authoritative
  documents disagree.
- Fix: `update_man.sh` adopts the probing logic; hoist the shared prompt body
  (`update_man.sh:121-148` vs `update_man_package.sh:87-115`) into one file both
  scripts read. While there: fix the in-prompt typos (F7).

### Theme D — dead and unreferenced tooling

#### D1 — `gen_vector_tests.py` is self-declared legacy, unreferenced, and destructive if re-run

- `scripts/gen_vector_tests.py` = **349** lines (not ~430). Its own docstring,
  lines **1-11**: "do not re-run it without re-bucketing its output, or it will
  recreate stale fixtures at the repo-root-relative `tests/func_vector_*` paths."
- Zero inbound references tree-wide, except `bugs/bug-326-dead-code-sweep.md`
  (`:316, 319, 321, 327`).
- It writes to the same pre-reorg layout A1's doc still mandates — the two
  findings share a root cause.
- Fix: delete it, or move it under `tools/` with the warning promoted to a
  guard. Coordinate with **bug-326**, which already cites it.

#### D2 — three unreferenced scripts, one of which is a real validation that is silently rotting

- `scripts/audit.sh` — **375** lines, superseded by the goal-NN review documents;
  also the only script using `#!/bin/bash` at `:1` where siblings use
  `#!/usr/bin/env bash`.
- `scripts/fix_citations.py` — **172** lines (not ~200).
- `scripts/check-net-connect-timeout.sh` (**102** lines) +
  `scripts/net_blackhole_server.py` (**45**) — this pair is a **genuine runtime
  validation** that nothing invokes, so it silently rots. Deleting it loses real
  coverage.
- All three are referenced only by themselves and by
  `bugs/bug-326-dead-code-sweep.md:316-319`.
- Fix: delete `audit.sh` and `fix_citations.py`; **keep** the net-timeout pair
  and reference it from `.ai/compiler.md` so it is actually run. Coordinate
  ownership with bug-326.

#### D3 — an `#[ignore]`d census test citing plan phases that never existed

- `src/ir/tests.rs:282`:
  `#[ignore = "porting census (plan-20-E..I); run with --ignored --nocapture"]`
  on `fn verify_vs_syntaxcheck_diagnostic_parity()`, with a doc comment at
  `:276-280` ("Porting-progress report (plan-20-E..I)").
- It is the **only** `#[ignore` in all of `src/`.
- `planning/old-plans/` contains only `plan-20-A-ir-spans.md`,
  `plan-20-B-ir-result-types.md`, `plan-20-C-package-checker.md`,
  `plan-20-D-total-elaboration.md`, and `plan-20-typed-ir-single-checker.md` —
  **E through I never existed**.
- Fix: run it. If the parity gap is at zero, promote it to a real (non-ignored)
  test — it is a valuable guard over the plan-20 split. If it is non-zero, that
  is a finding and gets its own bug. Either way, drop the phantom citation.

#### D4 — two genuinely unused imports in `tests/`

- `tests/gtk_term_utf8_grid.rs:20` — `use std::path::PathBuf;`, and `PathBuf`
  appears nowhere else in the file.
- `tests/tls_listen_accept_build.rs:25` — `use std::path::{Path, PathBuf};`;
  `PathBuf` **is** used (`:31` signature), but `Path` appears only on the import
  line. The unused item is `Path` within a partly-used import.
- The review checked all 20 files: six other candidates are trait imports needed
  for method resolution. **There is no systemic problem here** — just these two.
- Fix: delete both.

### Theme E — oversized and mixed test files

#### E1 — `tests/repo_acceptance.rs` is 1,968 lines / 18 tests across five unrelated concerns

- Confirmed: **1,968** lines, **18** `#[test]`.
- As one integration target it also serializes; splitting into four binaries lets
  cargo run them in parallel.
- Fix: split by concern. Note this file is also the workaround for the
  `repository/`-not-in-workspace gate defect (cross-referenced above) — do not
  split it in a way that entrenches the workaround.

#### E2 — `tests/native_io_runtime.rs` mixes `io::` and `term::` in 1,333 lines

- Confirmed: **1,333** lines, with `native_term_*` tests at `:956`, `:1022`,
  `:1074`, `:1137`.
- It also holds 11 local helpers including 2 C-interposer builders and 5 PTY
  drivers that `tests/gtk_term_utf8_grid.rs` would also want.
- Fix: split `native_term_*` into its own target; hoist the PTY drivers and
  interposer builders into B1's `tests/common/`.

#### E3 — test placement has no convention, and the split is not size-driven

- **118** `src/` files with an inline `mod tests {`; **12** sibling `tests.rs`
  files totaling **26,686** lines; **0** test subdirectories under `src/`.
- The split is demonstrably not about size: `src/target/shared/code/tests.rs` was
  extracted at **131** lines, while `src/syntaxcheck/inference.rs:1556` keeps
  **1,086** lines inline.
- Five files carry 880+ line inline test modules:
  `src/syntaxcheck/inference.rs` (1,086), `src/cli/build.rs:1921` (1,026),
  `src/target/shared/code/mir.rs:796` (1,002), `src/monomorph/lower.rs:1884`
  (943), `src/builtins/general.rs:652` (881).
- Fix: adopt a stated rule (e.g. extract at ~300 lines of tests) and record it in
  `CLAUDE.md`. Apply it opportunistically during **bug-327**'s file splits rather
  than as a separate mass migration — this item is the *rule*, not the migration.

#### E4 — four files keep items stranded below their inline `mod tests`

- `src/target/shared/code/link_thunk.rs:1515` — **491** lines below the test
  module in a **2,006**-line file (the review said ~250; it is worse), including
  design documentation.
- `src/doc.rs:636` — 462 lines below, in 1,098.
- `src/cli/resolve.rs:673` — 390 lines below, in 1,063.
- `src/audit/collect/dependencies.rs:77` — 143 lines below, in 220. (Recorded by
  one reviewer as possibly stale; re-measured and **still present**.)
- Fix: move the stranded items above the test module; add
  `clippy::items_after_test_module` to the deny list so it cannot recur.

### Theme F — stale documentation, layout, and citations

#### F1 — `tests/rt-behavior/security/README.md` documents another bucket's fixtures, cites archived paths, and omits the ones that actually live there

- The file is 77 lines. `:3` and `:5` cite `planning/audit-unicode.md` and
  `planning/audit-1-package-decode.md` — **neither resolves**
  (`audit-unicode.md` is at `planning/old-plans/`; `audit-1-package-decode.md`
  exists nowhere).
- `:37` cites `planning/plan-19-ir-semantic-verification.md` (also archived to
  `old-plans/`) and states PKG-02 "has no fixture yet" — **three exist**:
  `tests/syntax/security/pkg-02-type-confusion`, `pkg-02b-computed-confusion`,
  `pkg-02c-operator-confusion`.
- Table rows `:29-34` document 6 `pkg-*` fixtures that live in a **different**
  bucket (`tests/syntax/security/`), which `:48-49` admits.
- The **5** `allocator-0N-*` fixtures that *do* live in this directory get zero
  table rows — one passing mention at `:49`.
- Fix: rewrite. Document what is in this directory; cross-link the syntax-bucket
  fixtures rather than tabulating them; re-point the three archived citations.
  Overlaps A3 (the `.run` merge-trigger explanation lives at `:23-25`).

#### F2 — root `README.md` shows the pre-`build/` output path and omits two shipped packages

- `README.md:18` (`$ ./examples/hello_world/hello_world.out`) and `:65`
  (`$ ./myapp/myapp.out`) predate plan-46-D. The real path is `<proj>/build/`
  per `src/os/mod.rs:15` (`BUILD_DIR = "build"`), and on Linux the name is
  libc-flavored — `{name}-glibc.out` / `{name}-musl.out`, per
  `tests/linux_pie_headers.rs:57-60`.
- The package list at `:113-122` omits **`audio`** and **`money`**, both of which
  ship (`src/builtins/audio.rs`, `src/builtins/money.rs`, plus
  `money_package.mfb`).
- Fix: correct the two paths, add the libc-flavor note, add the two packages.
  This is the first file a new user reads.

#### F3 — the benchmark suite points at a path the compiler stopped writing, runs two timing engines, and contradicts its own README

- Stale output path, in both drivers: `benchmark/run.sh:42`
  (`mfb_out="$here/mfb/benchmark.out"`), `benchmark/runner.sh:54`
  (`MFB_OUT="$dir/$name.out"`), `benchmark/README.md:8` ("`mfb build` →
  `benchmark.out`"). `benchmark/run.sh:78`'s cleanup `rm -f` therefore removes
  nothing.
- Two timing engines: `benchmark/runner.sh:1-16` documents a
  source-and-call-`time_run` contract whose **only** remaining consumer is
  `benchmark/empty/run.sh:5`; the unified `benchmark/run.sh` never sources it and
  re-implements timing in `run_one()` at `:56-66` (dispatch `:67-70`). Both
  engines carry the stale `.out` path independently.
- Naming contradiction: `benchmark/README.md:13-14` claims "one file per package
  surface … the same split in all three". Actual: mfb has `mapchurn.mfb` +
  `listchurn.mfb`; C and Python each fuse them into `churnbench.c` /
  `churnbench.py`; mfb has `iobench.mfb` with no counterpart; C has
  `parsebench.c` with no mfb or Python peer.
- Fix: one timing engine (fold `empty/run.sh` onto `run.sh` and delete
  `runner.sh`), one corrected output path, and either align the file names or
  correct the README to describe what exists.

#### F4 — `planning/` holds three non-plan scratch files, a second archive directory, and a bug doc named as a plan

- `planning/prompts.md` — **152** lines, zero inbound references.
- `planning/mem.md:3` — "**Status:** scratch. This is a design work-pad, *not* a
  plan."
- `planning/res.md:4` — "Status: **THINKING — not a plan.** No design is
  committed here."
- `planning/old-moved-to-src-spec/` (architecture.md, error_codes.md, linker.md)
  sits alongside `planning/old-plans/` as a second archive location.
- `planning/allocator-20-coalesce-size-authority.md:1` reads
  `# allocator-04 — ROBUSTNESS: coalescing trusts the caller's free size
  (compiler-drift canary)` — the title disagrees with the filename, and the
  document is shaped as a bug doc, not a plan.
- Fix: move the scratch files to a `planning/scratch/` (or delete `prompts.md`);
  fold `old-moved-to-src-spec/` under `old-plans/`; reconcile the allocator
  document's name and shape. Per house rule, completed planning docs are **moved**
  to `planning/old-plans/`, never deleted.

#### F5 — `tests/_data/` is a tooling data set inside the four-folder test tree

- `tests/_data/math_kernel_ref/` = **476K**, and is the entirety of
  `tests/_data`.
- Exhaustive consumer list: `tools/math-kernels/capture.sh:20`,
  `tools/math-kernels/README.md:49`, `tools/math-kernels/runtime_ulp.py:311`.
  **Nothing** in `tests/` or `scripts/` reads it.
- It is also the only entry under `tests/` that is not one of the four buckets.
- Fix: move to `tools/math-kernels/ref/`; update the three consumers.

#### F6 — `.mcp.json` hard-codes one developer's absolute home path

- `.mcp.json:5` hard-codes
  `"/Users/justinzaun/Development/mfb/tools/mcp/index.js"`.
- `AGENTS.md:33-39` mandates that server ("The `mfbasic` MCP server (`mfb_man`,
  `mfb_spec`) … prefer `mfb_spec`/`mfb_man` over reading files by hand").
- **Correction — the severity claim did not hold.** The review stated the server
  is "silently unavailable in ANY worktree". It is not: the main checkout lives
  at exactly that absolute path, so the path **does** resolve from this worktree,
  and `tools/mcp/index.js` also exists in-worktree. The real defect is
  **portability** — it breaks for any other developer or a relocated clone — not
  silent unavailability here.
- Fix: make the path repo-relative.

#### F7 — small stale citations and typos, including two inside a model prompt

- `tests/fs_atomic_int_return.rs:16` cites
  `planning/bug-44-c-int-return-width-fsync-close.md`, which does not exist at
  that path. (This is the only broken repo-path reference in all of
  `tests/`, `scripts/`, `README`, `AGENTS.md`, and `.ai/`.)
- `.ai/remote_systems.md` — "Alipine" ×4, at `:5`, `:6`, `:8`, `:10`.
- `scripts/update_man.sh:151` "shuold" and `:153` "informaiton" — both **inside
  the text sent to the model**, so they degrade generated man pages rather than
  just looking untidy. Fix these alongside C4.
- `scripts/test-macapp.sh:342-345` — stale narration ("This case used
  `io::terminalSize` until plan-01-term Phase 3 removed that builtin; because the
  whole case is GUI-gated it kept being skipped…"). (Cited as `:343-345`; the
  block starts at `:342`.)

#### F8 — the `project-entry-*` fixture family

- **Correction**: the review described 18 fixtures as a 3×6 cross-product.
  Measured: **48** `project-entry-*` fixture directories — 17 in
  `tests/syntax/project/`, 16 in `tests/rt-behavior/project/`, 15 in
  `tests/rt-error/project/`. The three-bucket shape is real; the cross-product is
  ~3×16.
- Shared fixture *sources* across the family are largely legitimate: the variable
  under test is `project.json`, not the `.mfb`. (The review's "13 groups / 41
  fixtures share identical sources" figure was **not** independently
  re-measured — treat it as unverified.)
- Fix: **no action beyond documenting the convention.** Recorded here so the
  duplication is not "discovered" and wrongly deduplicated by a future pass.

## Goal

- `.ai/compiler.md` describes the fixture layout that actually exists, so agents
  stop creating misfiled fixtures.
- All 994 fixtures sit at `<bucket>/<feature>/<name>`, and the `.run` merge-trigger
  convention is encoded somewhere the harness can see.
- `temp_project`, `build_ncode`, `run_bounded`, `build_linux_elf`, and the PTY /
  interposer helpers each have exactly one definition, in `tests/common/`.
- `artifact-gate.sh` and `test-accept.sh` share one artifact table; the fast gate
  covers the same kinds as the harness.
- `test-macapp.sh` uses the same helpers as `test-appimage.sh`, with one
  parameterized watchdog.
- The coverage ignore-regex has one definition.
- Every README, script comment, and citation in scope points at something that
  exists.

### Non-goals (must NOT change)

- **Any shipped compiler output.** This bug touches only tests, scripts,
  benchmarks, planning docs, and agent-facing documentation. No `src/` change
  here may alter a generated artifact — the two `src/` items (D3's `#[ignore]`,
  E3/E4's test placement) are test-side only.
- The **set of tests that run** and their pass/fail results. Moving a fixture
  (A2) changes its path, not its assertions.
- Golden **contents**. A2 and F5 change golden *paths*; the bytes inside each
  golden file must be unchanged. Any content delta means something else broke.
- The `test-macapp.sh` output-path defect and the `repository/`-workspace defect —
  both are separate correctness bugs (see "Broken gates"). C2 must not
  incidentally "fix" the path while restructuring, because that would hide the
  real fix in a refactor diff.
- Tempting wrong fix, forbidden: making `src/testutil.rs` reachable from
  integration tests by removing its `#![cfg(test)]` (B1) — that changes the
  crate's build configuration to save a helper file.
- Tempting wrong fix, forbidden: deleting `scripts/check-net-connect-timeout.sh`
  and `net_blackhole_server.py` as "unreferenced" (D2) — they are real coverage
  that needs a caller, not removal.
- Tempting wrong fix, forbidden: deduplicating the `project-entry-*` fixture
  sources (F8) — the duplication is the point.

## Blast Radius

Searched and measured, not recalled.

- `.ai/compiler.md:33`, `:38` (A1) — fixed by this bug; it is also the root cause
  of D1's stale generator target.
- The 4 `tests/rt-error/*` fixture directories (A2) and `tests/_data/` (F5) —
  fixed by this bug; **both change harness paths** and require the golden
  procedure in Fix Design.
- The 9 `tests/syntax/**` fixtures with `.run` goldens and
  `scripts/test-accept.sh` (A3) — fixed by this bug.
- 16 `tests/*.rs` `temp_project` sites (B1, listed in full above) + 3 `build_ncode`
  + 2 `run_bounded` + 2 `build_linux_elf` (B2) + `tests/native_io_runtime.rs`
  helpers (E2) — fixed by this bug; all consolidate into `tests/common/`.
- `src/testutil.rs:8` and `src/main.rs:27` — **read only**; deliberately not
  modified (see Non-goals).
- `scripts/coverage.sh:19`, `scripts/coverage-check.sh:15`,
  `.github/workflows/coverage.yml:21` (B3) — fixed by this bug. Note the CI
  workflow is in scope; a mistake here changes what CI measures.
- `scripts/artifact-gate.sh` and `scripts/test-accept.sh:185-434` (C1, C3) —
  fixed by this bug. **These two scripts are this bug's own validation
  instrument**; changes to them must be validated by a before/after run on an
  unmodified tree, not by inspection.
- `scripts/test-macapp.sh`, `scripts/test-appimage.sh` (C2) — restructured here,
  **after** the separate path fix.
- `scripts/update_man.sh:92`, `:121-148`, `:151`, `:153`;
  `scripts/update_man_package.sh:31-42`, `:87-115` (C4, F7) — fixed by this bug.
- `scripts/gen_vector_tests.py`, `scripts/audit.sh`, `scripts/fix_citations.py`,
  `scripts/check-net-connect-timeout.sh`, `scripts/net_blackhole_server.py`
  (D1, D2) — **overlap `bugs/bug-326-dead-code-sweep.md:316-327`**, which already
  cites all of them. Pick one owner before deleting anything.
- `src/ir/tests.rs:276-282` (D3) — fixed by this bug; if the census is non-zero,
  the finding escalates out of this bug.
- `tests/gtk_term_utf8_grid.rs:20`, `tests/tls_listen_accept_build.rs:25` (D4) —
  fixed by this bug.
- `tests/repo_acceptance.rs` (E1), `tests/native_io_runtime.rs` (E2) — fixed by
  this bug.
- `src/target/shared/code/link_thunk.rs:1515`, `src/doc.rs:636`,
  `src/cli/resolve.rs:673`, `src/audit/collect/dependencies.rs:77` (E4) — fixed
  by this bug; `src/doc.rs` also appears in **bug-343** (D2 citations) and
  **bug-327**, so sequence.
- The 5 files with 880+ line inline test modules (E3) — **latent, same lack of
  convention, out of scope**: this bug states the rule; **bug-327** applies it
  during its splits.
- `tests/rt-behavior/security/README.md`, root `README.md`, `benchmark/**`,
  `planning/**`, `.mcp.json:5`, `.ai/remote_systems.md`,
  `tests/fs_atomic_int_return.rs:16`, `scripts/test-macapp.sh:342-345`
  (F1–F7) — fixed by this bug.
- The `project-entry-*` family (F8) — **unaffected by design**; recorded only.
- Golden/fixture orphans — **none exist**; recorded only.

## Fix Design

This bug changes **no shipped output**, so `scripts/artifact-gate.sh` is not the
instrument here — the harness is. And two items (C1, C3) modify the harness
itself, which is the one real risk in the cluster: a script edit that silently
stops comparing something looks exactly like a passing run.

The discipline that follows from that:

1. **Capture a baseline first.** On an unmodified tree, record the full output
   of `scripts/test-accept.sh <mfb-exe> <actual-dir>` — pass/fail counts, the
   list of compared artifacts, and the `checked`/`ran` counters from
   `scripts/artifact-gate.sh <mfb-exe>`. Every later run is diffed against this.
2. **Harness edits (C1, C3) must be proven by counter parity.** After the
   refactor, `test-accept.sh` must compare the **same 19 artifact kinds** and
   report the same per-test results as the baseline. C1 deliberately *increases*
   `artifact-gate.sh`'s coverage from 8 to 19 kinds — that is the intended change,
   and the new `checked` count must be justified, not merely accepted.
3. **Fixture moves (A2, F5) require a `sync-goldens.sh` run, and the goldens must
   not change content.** Procedure:
   - `git mv` the fixture directory (goldens travel with it), so the file bytes
     are preserved and git records a rename;
   - run `scripts/sync-goldens.sh <mfb-exe> '<name-glob>'` for the moved fixtures
     only — it is filter-aware and takes ~4s per fixture, and it **only refreshes
     existing golden files, never creates new ones**
     (`scripts/sync-goldens.sh:2-6`);
   - `git diff` the result. The expected diff is **renames only, zero content
     changes**. Any byte-level delta means the fixture's harness path feeds into
     its own output and must be investigated before the move lands.
4. **CI edits (B3) get their own commit** and a confirmed workflow run.

Ordering:

- **A1 first**, alone. It is a documentation edit that stops the bleeding, and
  every hour it is not landed is another chance for a misfiled fixture.
- **B1 + B2 next** — mechanical, high line-count, zero risk, and they unblock E2.
- **C3 then C1** — build the shared artifact table in `test-accept.sh` first,
  then have `artifact-gate.sh` consume it. Reversing this order means writing the
  table twice.
- **C2 after the separate `test-macapp.sh` path fix.**
- **A2 and F5** together, as the only fixture-moving commit, with the golden
  procedure above.
- **D1/D2 coordinated with bug-326**; **E3/E4 coordinated with bug-327**.

Rejected alternatives, so they are not re-litigated:

- *Bring `artifact-gate.sh` to parity by hand-adding the 11 missing kinds.*
  Rejected: that is exactly what produced the current drift. The table must be
  shared, not synced.
- *Delete `artifact-gate.sh` and always run the full harness.* Rejected: the fast
  gate's ~5-minute execution-free run is a deliberate, documented workflow for
  codegen changes. Fix it; do not remove it.
- *Leave the 4 misfiled fixtures where they are and document the exception.*
  Rejected: 989 of 994 follow the layout, and the 4 are misfiled by *kind* (they
  are compile-time diagnostics in a runtime-error bucket), not merely by depth.
- *Remove `#![cfg(test)]` from `src/testutil.rs`.* Rejected — see Non-goals.
- *Mass-migrate inline test modules to sibling `tests.rs` files (E3).* Rejected as
  a standalone change: a 26,686-line mechanical churn with no behavioral benefit.
  State the rule; apply it during bug-327's splits.

## Phases

### Phase 1 — baseline + stop the bleeding

- [ ] Record the baseline: full `scripts/test-accept.sh` output and
      `scripts/artifact-gate.sh` `checked`/`ran` counters on an unmodified tree.
      Paste the numbers into this file.
- [x] A1: rewrite the fixture-layout mandate in `.ai/compiler.md` to the
      four-folder layout with one worked example per bucket.
- [x] D4: delete the two unused imports (smoke test that the baseline still
      reproduces).

A1 done 2026-07-22. The mandate lives in `.ai/compiler.md`'s Validation section
(the "For every function created or modified…" bullet, `git`-current — the doc's
`:33/:38` line cites had drifted). It named `tests/func_<package>_<func>_{valid,
invalid}/**`, which do not exist (`ls -d tests/func_*` → no match). Rewritten to
the real four-tree layout — `tests/{syntax,rt-error,rt-behavior}/<feature>/<name>`
plus `tests/acceptance` — with a worked example per bucket and an explicit "never
create the old flat layout" note. Source of truth: `scripts/test-accept.sh:228-234`.

D4 — **already clean; no action.** The doc's two sites are stale:
`tests/gtk_term_utf8_grid.rs` no longer imports `PathBuf` at all (grep: zero
`PathBuf`), and `tests/tls_listen_accept_build.rs:25` is `use std::path::PathBuf;`
— used at `:31` (`fn temp_project(...) -> PathBuf`); the `Path` the doc flagged as
the unused half of `{Path, PathBuf}` is already gone. The repo invariant
(`cargo check --all-targets` clean, no dead-code allows) means any real unused
import would already be a warning; there is none to delete.

Acceptance: `.ai/compiler.md` now describes directories that exist; no code moved.
Commit: —

### Phase 2 — consolidate duplicated scaffolding

- [ ] B1: `tests/common/mod.rs` with one `temp_project`; delete the 16 copies.
- [ ] B2: move `build_ncode`, `run_bounded`, `build_linux_elf` into
      `tests/common/`; parameterize the panic message; settle the 20ms/25ms poll.
- [ ] E2: split `native_term_*` out of `tests/native_io_runtime.rs`; hoist the
      PTY drivers and C-interposer builders into `tests/common/`.
- [ ] E1: split `tests/repo_acceptance.rs` by concern.
- [ ] B3: single-source the coverage ignore-regex (separate commit; confirm a CI
      run).

Acceptance: identical pass/fail set to baseline; `cargo test` green; CI coverage
denominator unchanged.
Commit: —

### Phase 3 — harness, layout, and documentation

- [ ] C3: one artifact table driving all five regions of `test-accept.sh`.
- [ ] C1: `artifact-gate.sh` consumes that table; add the missing `-q`. New
      `checked` count recorded and justified.
- [ ] C2: adopt `test-appimage.sh`'s helpers in `test-macapp.sh` (after the
      separate path fix); one parameterized watchdog.
- [ ] C4 + F7: `update_man.sh` adopts the probing logic; hoist the shared prompt;
      fix the in-prompt typos.
- [ ] A2 + F5: move the 4 misfiled fixtures and `tests/_data/`, per the golden
      procedure in Fix Design.
- [ ] A3: encode the `.run` merge-trigger convention in the harness.
- [ ] D1, D2 (coordinated with bug-326), D3, E4.
- [ ] F1, F2, F3, F4, F6.

Acceptance: full `scripts/test-accept.sh` green with the same pass/fail set as
baseline; `git diff` over `tests/**/golden/` shows **renames only, zero content
changes**; `artifact-gate.sh` covers all 19 artifact kinds.
Commit: —

## Validation Plan

- Regression tests: no new fixture. New coverage only where an item removes a
  duplicate — a single `tests/common/` helper exercised by its consumers, and a
  parity assertion that `artifact-gate.sh` and `test-accept.sh` read the same
  artifact table.
- Runtime proof: the before/after `scripts/test-accept.sh` diff. The pass/fail
  set must be **identical** to the Phase 1 baseline at every commit; for C1, the
  `artifact-gate.sh` `checked` count must **rise** from 8 kinds' worth to 19 and
  the increase must be accounted for.
- Golden guard: after A2 and F5, `git diff --stat` over `tests/**/golden/` must
  show renames only. Any content byte change blocks the commit.
- Doc sync: `.ai/compiler.md` (A1), root `README.md` (F2),
  `tests/rt-behavior/security/README.md` (F1), `benchmark/README.md` (F3),
  `CLAUDE.md` (E3's stated rule).
- Full suite: `cargo test`, `scripts/test-accept.sh`, `scripts/artifact-gate.sh`,
  a CI coverage run (B3 touches the workflow).

## Open Decisions

- **D1/D2 ownership** — recommended: `bugs/bug-326-dead-code-sweep.md` already
  cites all five scripts, so it deletes them and this bug only re-homes the
  net-timeout validation and references it from `.ai/compiler.md`. Alternative:
  this bug owns all of it and bug-326 drops the citations. Settle before either
  lands.
- **A3 mechanism** — recommended: a `merge-trigger` marker file in the fixture
  directory, checked by `test-accept.sh`. Alternative: a naming convention
  (`*-merge` suffix). The marker is explicit; the suffix is cheaper.
- **D3 disposition** — depends on the measurement: if the parity census is zero,
  promote to a real test; if not, file the gap. Run it before deciding.
- **E3 threshold** — recommended: extract at ~300 lines of tests, recorded in
  `CLAUDE.md`; alternative is to leave placement to each split's author. Pick
  one, because "no convention" is the current state.
- **F4 `allocator-20-coalesce-size-authority.md`** — recommended: it is shaped as
  a bug doc, so move it to `bugs/` under a matching number and fix the title;
  alternative is to rename the title to match the filename and leave it in
  `planning/`.

## Summary

Twenty-eight verified items across `tests/`, `scripts/`, `benchmark/`,
`planning/`, and the agent-facing docs. The engineering risk is concentrated in
three places: **A1**, which is the only item causing ongoing damage (agents are
being told to create fixtures in directories that do not exist); **C1/C3**, which
modify the very harness that validates this bug — so they are proven by
before/after counter parity, never by inspection; and **A2/F5**, the only items
that move fixtures, which change harness paths and therefore require a filtered
`scripts/sync-goldens.sh` run whose diff must show **renames only, zero content
changes**. Everything else is deletion, consolidation, and citations pointing at
files that moved.

No shipped compiler output changes. Also recorded, so it is never re-derived:
**the golden/fixture orphan scan is clean** — 994 fixture directories, 979 with
`golden/`, **zero** orphaned goldens, 2,684 golden files, and the 15 fixtures
without a `golden/` are intentional behavioral runs per
`scripts/test-accept.sh:166-183`.

Six leads did **not** survive re-measurement and are corrected in place rather
than carried forward: `.mcp.json`'s path **does** resolve from this worktree (the
defect is portability, not silent unavailability); `update_man.sh`'s bad source
path affects **1** module, not 6; there are **16** `temp_project` copies in
**6** hash groups, not 17 in one; `test-accept.sh`'s artifact plumbing spans
**5** regions, not 4; the `project-entry-*` family is **48** fixtures, not 18;
and `artifact-gate.sh`'s drift is **11** missing artifact kinds, not 3.
