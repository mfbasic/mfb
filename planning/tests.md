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

- [ ] Add the harness to `src/codegen/engine/tests/test_support.rs` and/or
      `src/testutil.rs` — both are OUTSIDE the coverage denominator
      (`IGNORE` matches `/tests/` and `/src/testutil.rs`), so the harness cannot
      inflate any file's percentage.
- [ ] Prove it: one test that lowers a whole program in process and one that
      drives a single emitter through `CodeBuilder`.

Acceptance: both harness entry points are used by at least one passing test, and
`coverage-check.sh` shows movement on at least one previously-0% file.

## Phase 2 — close the files, worst first, one commit each

The per-file ledger is generated from the Phase 0 baseline; see
`planning/coverage-ledger.md`. Each row is one commit.

- [ ] Generate `planning/coverage-ledger.md` from the measured baseline.

Acceptance: `sh scripts/coverage-check.sh` prints
`All non-excepted files >= 98% line coverage.`

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

_(Empty until something diverges.)_
