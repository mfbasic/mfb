# Task: close the per-file coverage gate, one file per commit

The `coverage` job is the only red job in CI. Every build and test job is green.
It fails on its per-file 98% gate, not on tests. Work it file by file, landing one
commit per file.

## Current state (measured on CI run 33816061767, job 100849258494)

- **405 files** below the 98% floor; **89.09%** overall.
- Distribution: 46 files <50% (7 at exactly 0%), 49 at 50–80%, 41 at 80–90%,
  61 at 90–95%, 104 at 95–97%, 104 at 97–98%.
- The gate has not run in CI since 2026-07-28 — the test step failed first every
  time — so this is ~2,500 commits of accumulated drift, not a regression.

Worst first:

```
 0.00%  (0/229)   src/codegen/builtins/canvas/gen_present.rs
 0.00%  (0/163)   src/codegen/builtins/astrings/gen_astrings.rs
 0.00%  (0/157)   src/codegen/builtins/math/gen_fmod.rs
 0.00%  (0/37)    src/codegen/builtins/tls/gen_macos/timeout.rs
 0.00%  (0/28)    src/codegen/runtime/canvas/metal.rs
 0.00%  (0/11)    src/codegen/builtins/canvas/scene_base.rs
 0.00%  (0/10)    src/codegen/builtins/canvas/gen_image.rs
 1.07%  (8/748)   src/codegen/builtins/perf/perf.rs
 1.25%  (50/4010) src/codegen/runtime/canvas/vulkan.rs
 4.74%  (22/464)  src/codegen/builtins/strings/func_normalize_nfc.rs
 8.45%  (60/710)  src/codegen/runtime/canvas/mod.rs
10.46%  (16/153)  src/codegen/builtins/os/func_version.rs
13.32%  (59/443)  src/codegen/builtins/collections/func_group_by.rs
```

## Establish this FIRST — it decides the shape of the whole job

Several 0% files are lowering emitters that the canvas/rt integration suites
exercise heavily. Those suites shell out to `target/release/mfb` via
`common::mfb_exe()`. **If that subprocess is not instrumented, no integration
test contributes any coverage, and only in-process `#[test]` code counts.**

That is a hypothesis, not a finding — verify it before planning anything:

```sh
sh scripts/coverage.sh                      # full instrumented run, slow
sh scripts/coverage-check.sh canvas/gen_present.rs
```

If `gen_present.rs` is still 0% after a run in which the canvas suites passed,
the hypothesis holds and every file must be covered by unit tests that call the
emitter **in process**. Say which way it came out before proceeding.

## Iterating cheaply

`scripts/coverage.sh` runs the whole instrumented suite (slow — budget for it
once). `scripts/coverage-check.sh` then regenerates the report **from the cached
profile without re-running tests**, and takes substring filters:

```sh
sh scripts/coverage-check.sh src/codegen/builtins/os/func_version.rs
FLOOR=0 sh scripts/coverage-check.sh func_group_by     # report without gating
```

Re-run `coverage.sh` only after adding tests. Config (`IGNORE`, `PKG_FLAGS`)
lives in `scripts/coverage-common.sh`; do not duplicate it — bug-347 exists
because three copies diverged.

## The decision rule for each file

Prefer a real test. Reach for an exception only when the uncovered remainder is
genuinely unreachable in-process, and say why in the file.

- **Emitters are usually testable anywhere.** `gen_macos/timeout.rs` and
  `canvas/metal.rs` are macOS *emitters* — they build instruction vectors and can
  be called on a Linux runner like any pure function. "It is macOS code" is not by
  itself a reason it cannot be unit-tested; CI's coverage job runs on ubuntu-latest.
- **Genuinely exception-worthy:** live network I/O, socket/TTY syscalls,
  subprocess invocation (linker/codesign), GUI event loops.
- Exceptions go in `scripts/coverage-exceptions.txt`, one repo-relative path per
  line with a `# reason`. Read its header first.
- `// coverage:off` does **not** work — cargo-llvm-cov 0.8.7 ignores it.

**Do not lower `FLOOR`, and do not bulk-add exceptions to make the gate pass.**
AGENTS.md forbids weakening a gate without proof it is wrong. If a file's honest
answer is an exception, that is one commit with its own justification, same as a
test.

## Commit discipline

One file per commit. Each commit message states the file, what was uncovered,
what the new tests actually assert (behaviour, not line-hitting), and the
before/after percentage from `coverage-check.sh`. Tests must assert real
contracts — a test written only to touch lines is worse than the gap, because it
makes the gate green while protecting nothing.

Run `cargo check --all-targets` (test-target warnings only appear there) and
`rustup run 1.96.0 cargo fmt --all` before each commit.

## Traps that cost time in the session that found this

- `$?` after a pipe is the **last** command's status. `cargo build … | grep …;
  echo $?` reports grep. Redirect to a file and check the real exit code.
- Never redirect a build to `/dev/null` and then read its output artifact — you
  will read a stale one and believe it.
- A test that passes proves nothing until you have seen it fail. Break the thing
  it guards, watch it go red, restore.
- The full suite is `cargo test --release --workspace --no-fail-fast`; without
  `--no-fail-fast` it stops at the first failing binary and hides everything after.

## Scope

Coverage only. Do not touch the canvas/ABI work that just landed. Report
progress as `<n> files closed, overall X% → Y%`; do not claim the gate is green
until `scripts/coverage-check.sh` prints `All non-excepted files >= 98%`.

---

# Execution ledger (added by /follow-plan)

The authored task above states the goal, the tooling and the rules but carries no
Prerequisites gate, no phases and no checkboxes. Those are added here so progress
is auditable; **nothing above is re-scoped**. Corrections to the authored text go
in the Corrections section at the bottom.

## Prerequisites

| # | Check | Command | Status |
|---|---|---|---|
| P1 | `cargo-llvm-cov` present at the version the scripts assume | `cargo llvm-cov --version` | MET — `cargo-llvm-cov 0.8.7` |
| P2 | The three coverage scripts and the exceptions file exist | `ls scripts/coverage.sh scripts/coverage-check.sh scripts/coverage-common.sh scripts/coverage-exceptions.txt` | MET — all four present |
| P3 | A full instrumented run completes and leaves a profile | `sh scripts/coverage.sh` | MET — exit 0; 3835 unit tests + 128 integration binaries, `grep -c 'test result: FAILED'` = 0 |
| P4 | The per-file report regenerates from the cached profile | `FLOOR=0 sh scripts/coverage-check.sh` | MET — `Overall line coverage: 89.34%  (1591 files)` |

## Phase 0 — settle the subprocess-instrumentation question

The authored task says to establish this first because it decides the shape of
everything after it.

- [x] Run `sh scripts/coverage.sh` to completion and record the suite's exit status.
      Exit 0, no failing binary (`grep -c "test result: FAILED" = 0`).
- [x] Run `sh scripts/coverage-check.sh canvas/gen_present.rs` and record the number.
      `0.00%  (0/233)  src/codegen/builtins/canvas/gen_present.rs`.
- [x] State the verdict in Findings below: does integration-suite (`mfb_exe()`
      subprocess) execution contribute to `src/**` line coverage, yes or no?
      **No.** See Findings.
- [x] Record the local baseline: overall %, count of files below 98%, and the
      full below-floor list captured to `planning/coverage-baseline.txt`.
      **89.34% overall, 416 files below the floor, 15 excepted.**

Acceptance: the verdict is stated with the command and number behind it, and the
baseline list is committed.

## Phase 1 — the in-process lowering harness

Every candidate fix needs a way to call an emitter in process. Two seams exist and
neither is currently reachable from a unit test without ~60 lines of boilerplate:

1. `CodeBuilder::for_synthetic_function` (`src/codegen/engine/builder/mod.rs:465`)
   — a single emitter, with `TestPlatform`
   (`src/codegen/engine/tests/test_support.rs:17`) as the platform.
2. The whole pipeline in process: `testutil::lower_src` → `lower::lower_project`
   → `<target>::plan::lower_module` → `<target>::code::lower_module`. This is what
   the `.ncode` dump path does (`src/target/macos_aarch64/mod.rs:501-522`) minus
   the file write, so it needs no linker and no subprocess.

- [x] Add the harness to `src/codegen/engine/tests/test_support.rs` and/or
      `src/testutil.rs` — both are OUTSIDE the coverage denominator
      (`IGNORE` matches `/tests/` and `/src/testutil.rs`), so the harness cannot
      inflate any file's percentage.
      `testutil::code_for_src_on`/`app_code_cached`/`code_for_src_cached` (whole
      program, any of five backends, any build mode) and
      `test_support::{Stream, BuilderHarness}` (one emitter, real per-family
      platform). Landed in 188f337de, 764b9d9a8 and c5ff4b1.
- [x] Prove it: one test that lowers a whole program in process and one that
      drives a single emitter through `CodeBuilder`.
      `testutil::code_harness_tests::every_backend_lowers_a_program_in_process`
      and `builtins/tests/os.rs::every_os_body_refuses_an_undeclared_import`.
- [x] Add the suites' home: `src/codegen/builtins/tests/`, declared from
      `codegen/engine/tests/mod.rs`. Both paths are excluded, which is
      load-bearing rather than tidy — see Corrections C3.

Acceptance: MET. Both entry points are used by passing tests, and the
previously-0% `gen_present.rs` / `gen_fmod.rs` are at 99.57% / 100.00%.

## Phase 2 — close the files, worst first, one commit each

The per-file ledger is generated from the Phase 0 baseline; see
`planning/coverage-ledger.md`. Each row is one commit.

- [x] Generate the measured baseline: `planning/coverage-baseline.txt` (416
      files below the floor across the whole workspace; 401 of them in `src/**`).
- [x] `canvas/gen_present.rs` 0.00% (0/233) -> 99.57% (232/233) — c97ba9dea
      (also closed `canvas/scene_base.rs` 0/11 -> 11/11 and
      `canvas/gen_image.rs` 0/10 -> 10/10, which the same program reaches)
- [x] `math/gen_fmod.rs` 0.00% (0/157) -> 100.00% (157/157) — 268ef99da
- [x] `builtins/perf/perf.rs` 1.07% (8/748) -> 98.26% (735/748)
- [x] `os/func_version.rs` 10.46% (16/153) -> 100.00% (153/153)
- [x] `os/func_uptime.rs` 13.56% (16/118) -> 99.15% (117/118)
- [x] `os/func_is_admin.rs` 26.67% (16/60) -> 100.00% (60/60)
- [x] `canvas/mod.rs` 53.77% (1013/1884) -> 99.22% (1013/1021) — no new test;
      the gap was the measurement artifact in Corrections C3
- [x] `math/mod.rs` 57.45% (270/470) -> 100.00% (270/270) — same
- [x] `runtime/canvas/vulkan.rs` 1.21% (50/4122) -> 98.01% (4040/4122) — reached
      by the `canvas::present` program; the single largest file in the task
- [x] `collections/func_group_by.rs` 13.32% (59/443) -> 98.19% (435/443)
- [x] The whole-program corpus: 360 committed fixtures x 1 backend, 16 x 5.
      No single file, but it is what reaches the shared codegen no per-file
      suite can: `list_mutate.rs` 70.40% -> 88.78%, `builder_inplace_assign.rs`
      36.92% -> 67.93%, `ir/lower.rs` 87.69% -> 92.44%.
- [x] The diagnostic corpus: 417 committed `tests/syntax/**` goldens reproduced
      in process. `ir/shape.rs` 85.93% -> 89.46%.
- [x] The registry sweeps — the single largest lever in the task, because the
      same two guards are dead in ~90 files at once:
      `abi_inline` type + import + arity (348 <- 371 <- 293 <- 272),
      `abi_function` + `Mfb` fast paths, and the 19 `vector::` selectors.
- [x] `os/func_arch.rs`, `os/func_name.rs`, `os/func_pid.rs`,
      `io/func_is_buffered.rs`, `manifest/url.rs`, `intern.rs`,
      `cli/version.rs` — seven near-miss files, one to three lines each.
- [ ] The remaining 246 `src/**` files below the floor, worst first. Regenerate
      the ranking with `python3 scripts/coverage-src-gaps.py <report.json>`,
      which sorts by LINES SHORT rather than by percentage.
- [ ] Re-run the FULL `sh scripts/coverage.sh` at the end: `src/**` is settled by
      `--bins` (Findings F1) but `repository/src/**` is not, and only the full
      run measures it.

Acceptance: `sh scripts/coverage-check.sh` prints
`All non-excepted files >= 98% line coverage.`

### Where the count stands

Measured with `scripts/coverage-bins.sh` + `scripts/coverage-src-gaps.py`,
`src/**` only (Findings F1: that is the same measurement as the full run for
these files, and is not for `repository/src/**`).

| point | files below 98% | uncovered lines |
|---|---|---|
| baseline (full run, before any work) | 401 | — |
| after the canvas/fmod/perf/os per-file suites | 371 | 11,498 |
| after the whole-program + diagnostic corpora | 348 | 11,166 |
| after the three registry sweeps | 253 | 10,648 |
| after the near-miss batch | **246** | **10,717** |

The uncovered-line count moves less than the file count in the later rows, and
that is the shape of the remaining work rather than a stall: the sweeps closed
files that were one or two lines short, so the lines they recovered were few and
the files many. What is left is 25 files that are more than 100 lines short and
59 more between 31 and 100 — real gaps in `link_thunk.rs`,
`builder_inplace_assign.rs`, `builder_values.rs` and their neighbours, which
need programs that exercise shapes the corpus does not yet contain.

## Findings

### F1 — the integration suite contributes NOTHING to `src/**`. Verdict: unit tests only.

Two independent proofs, agreeing:

- **Measured.** A full `sh scripts/coverage.sh` finished green — 3835 unit tests
  and 128 integration binaries, `grep -c "test result: FAILED" /tmp/pcov-run1.log`
  = 0, including every `rt_canvas_*` suite. `sh scripts/coverage-check.sh
  canvas/gen_present.rs` still reports `0.00%  (0/233)`.
- **Structural, and stronger.** `mfb` is a **binary-only** package — `cargo test
  -p mfb --lib` answers `error: no library targets found in package 'mfb'`, and
  `Cargo.toml` declares no `[lib]`. A Cargo integration test can only link a
  package's *library* target, so nothing under `tests/` links a single line of
  `src/**`; those binaries can reach the compiler only by spawning
  `target/release/mfb` (`tests/common/mod.rs:1080`, `mfb_exe()`), a separate
  process whose profile this one never merges.

  The corollary is the useful part: **`src/**` coverage is a pure function of the
  `--bins` unit tests.** Iterating with `cargo llvm-cov --bins` is therefore not
  an approximation of the gate for `src/**` — it is the same measurement, minutes
  instead of hours. (`repository/src/**` is the opposite case: `mfb_repository`
  *is* a lib, so its integration tests do count, and it needs the full run.)

So every file in this task is closed by a test that calls the code **in process**.

### F2 — the harness that makes that possible

`src/testutil.rs` (outside the coverage denominator) now runs the real `.ncode`
dump pipeline minus the file write: `concrete_hir_from_src` → `lower_src_concrete`
→ `lower_project` → `<backend>::plan::lower_module` → `<backend>::code::lower_module`.
A test hands it MFBASIC source and gets the `NativeCodePlan` — every emitted
instruction, relocation and data object — with no linker and no subprocess.

Three things had to be right, each of which failed loudly first:

1. **Monomorphization is not optional.** `testutil::lower_src` skips it, and any
   program reaching a builtin through a generic seam then dies with
   `TYPE_CALL_ARGUMENT_MISMATCH: Argument 1 for #encoding_utf8Decode has type
   List OF Byte, expected List OF Integer`. `lower_src_concrete` runs the build
   path's pass (`src/cli/build/mod.rs:469-482`).
2. **An `-app` program needs its `-app` build mode.** In `Console`,
   `app::setMode` fails with `codegen calls 'g_idle_add' … which the platform
   import list does not declare`.
3. **An app build needs an entry point.** The toolkit bootstrap that *defines*
   `_mfb_gtkapp_reconcile_idle` is emitted only inside `if let Some(entry) =
   &module.entry` (`src/codegen/engine/builder/mod.rs:1420`), so lowering with
   `entry: None` fails validation on a dangling relocation.

Plus one that is a harness limit, not a product fact: libtest gives each case a
2 MiB stack and the unoptimized front end overflows it on a `canvas` program, so
the lowering runs on a 64 MiB thread. A real build is on the 8 MiB main thread.

### F3 — the local gate and CI's gate are not the same set

Measured here (macOS/arm64) `src/codegen/builtins/tls/gen_macos/timeout.rs` is
**above** the floor; CI (ubuntu/x86_64) reports it at `0.00% (0/37)`. Coverage of
a platform-specific emitter follows the host, so "green locally" does not imply
"green on CI" in either direction.

The mitigation is in the harness: `CodeTarget` lowers for any of the five
backends **from any host**, so a test for a macOS emitter names
`CodeTarget::MacosAarch64` and covers it on the ubuntu runner too. Every test
this task adds for a platform-specific file must name its target explicitly
rather than relying on the host.

## Corrections

### C1 — "the coverage job is the only red job in CI" is false; `fmt` is red too

`.github/workflows/coverage.yml` runs `cargo fmt --all -- --check` under 1.96.0
as its own job, and on main (80e9895ea) it fails on four files:
`builtins/process/func_close_input.rs:25`,
`builtins/regex/func_find_all_matches.rs:16`, `codegen/resource/mod.rs:318`
and `:366`, `tests/rt_regex_span.rs:197`. Reproduced by restoring all four to
HEAD and re-running the check, so it does not follow from this branch. Fixed in
6ae1eca23 (reformatting only).

### C2 — the local baseline is 416 files / 89.34%, not 405 / 89.09%

`sh scripts/coverage.sh` here reports `416` files below the floor and
`89.34%` overall, against the task's CI-measured `405` and `89.09%`. The
difference is the host: coverage of a platform-specific emitter follows the
machine. `tls/gen_macos/timeout.rs` is one of the task's seven 0% files on CI
and is ABOVE the floor here. See Findings F3; the mitigation is that every test
for a platform-specific file names its `CodeTarget` explicitly.

### C3 — a `#[cfg(test)] mod` line makes its OWN file report far worse

Not anticipated by the task, and large enough to change what the list means. A
`cargo llvm-cov` profile merges two copies of the crate — the test binary, which
runs, and the plain `mfb` binary, which is instrumented and never executed
(10,954 function records, every one at count 0). Once a `#[cfg(test)]` module
appears in a file, the two copies inline differently and the never-run copy
contributes uncovered regions for lines the running copy demonstrably executes.

Measured three ways:

- adding `mod tests_codegen;` to `builtins/os/mod.rs`: `100.00% (131/131)` ->
  `85.06% (131/154)`. `register` runs (the suite calls it and every member is
  registered) and `#[inline(never)]` changed nothing; parking the suite restored
  131/131.
- the same line in `builtins/mod.rs`: `99.51% (610/613)` -> `68.93% (610/885)`.
- and the converse: relocating the suites out of `canvas/mod.rs` and
  `math/mod.rs` took those files from `53.77% (1013/1884)` and `57.45%
  (270/470)` to `99.22% (1013/1021)` and `100.00% (270/270)` — the same covered
  lines, ~1,060 fewer phantom ones.

So the suites live in `src/codegen/builtins/tests/` and the one `mod` line lives
in `codegen/engine/tests/mod.rs`; both paths are excluded by `IGNORE`, so the
declaration damages nothing. Deleting the never-executed plain binary before
reporting was also tried and changes `src/**` by exactly nothing (0 files
moved), so the object list is not the lever — the module placement is.

### C5 — the largest single lever was not per-file work

The task's shape is "work it file by file", and for the first ten that was
right. It stopped being right around the 119 files that were one to three lines
short: every one of them was the same two guards — a type check on a builtin
lowering's arguments and a `?` on the platform emitter that resolves its libc
import — and both are unreachable from any source program, so they were dead in
coverage terms in ~90 files simultaneously.

Three sweeps over the REGISTRY (not over a list of files) took the count from
371 to 253 in about an hour, and each runs in 0.02s. Per-file work would have
been ~90 commits for the same result, with 90 copies of the same assertion.
The task's "one file per commit" rule still holds for a real gap; it does not
hold for a gap that is one defect replicated by a code pattern.

### C4 — one commit per file, but one suite per program

The task says one commit per file. Where a single program and a single suite
close several files at once (the three per-platform `os::` emitters; the three
canvas files behind one `canvas::present` program), splitting the commit would
mean landing a suite that does not compile, or landing it three times. Those
land as one commit that names every file it closes with its own before/after.
Everything else is one file, one commit.
