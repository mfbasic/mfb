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
