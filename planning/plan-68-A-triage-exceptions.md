# plan-68-A: Triage, exception-list repair, fresh line-level report

Last updated: 2026-07-27
Overall Effort (AI): large (3h–1d)   (whole plan-68 feature)
Effort (Human): medium (1h–2h)
Effort (AI): small (<1h)
Depends on: nothing
Produces:
- A regenerated `target/coverage/coverage.json` (current-HEAD line-level
  coverage) that B–H read to name their uncovered target regions.
- A repaired `scripts/coverage-exceptions.txt`: stale paths repointed, genuinely
  integration-only moved files re-excepted with fresh boundary justifications.
- The authoritative **worklist**: for each of the 55 failing files, the decision
  `except` | `backfill (letter X)` | `denominator-fix`, recorded in this doc's
  triage table. B–H trust these decisions and do not re-litigate them.

Part **A** of plan-68. Shared goal, prerequisites, dependency graph, measured
populations, design, and standing requirements live in the overview:
[plan-68-coverage-gate.md](plan-68-coverage-gate.md). Prerequisites are stated
there and gate this sub-plan too — re-run them before starting.

## Why this lands first

The overview's design section: the premise "all 55 files need unit tests" is
false, and the B–H split assumes A already removed the exceptions. A is the
cheapest experiment that falsifies the naive premise: it produces both the fresh
line data every backfill task needs and the definitive except-vs-backfill call
for every file. Nothing in B–H can be written followably until A's report and
worklist exist.

## Phases

### Phase A1 — Regenerate the coverage profile + report

The on-disk `target/coverage/coverage.json` is stale (overview §2 Verified
properties). A full instrumented run is the only source of current line-level
data.

- [ ] Confirm prerequisites: `cargo llvm-cov --version` succeeds; `cargo test`
      → `0 failed`.
- [ ] Run `sh scripts/coverage.sh`. It must exit 0 and (re)write
      `target/coverage/coverage.json`, `.../html/`, `lcov.info`, `cobertura.xml`.
- [ ] Run `sh scripts/coverage-check.sh` and capture the current failing set.
      Cross-check the count against the overview's 55-row table; record any
      delta (files that landed/drifted since the user's snapshot) in this doc's
      Corrections and update the overview table + letter assignments to match.

Acceptance: `target/coverage/coverage.json` mtime is current;
`sh scripts/coverage-check.sh` reproduces a failing list; any difference from the
overview's 55-file table is written into Corrections with the measuring command.
Commit: — (no source change; this phase only produces artifacts + notes)

### Phase A2 — Repoint the stale exceptions

`scripts/coverage-exceptions.txt` names two paths whose code moved (overview §2).
Repair them so the exception follows the code, without excepting anything newly.

- [ ] Replace the `src/cli/build.rs` entry: the linker-spawn (`build_project`)
      and registry-attestation (`load_build_signing_info`) orchestration now lives
      under `src/cli/build/`. Determine — from A1's report + reading the files —
      which `src/cli/build/*.rs` modules are integration-only (subprocess/network,
      → except here, at their new path, with the specific boundary named) and
      which carry unit-coverable parsing/validation (→ leave for sub-plan B).
      Do NOT blanket-except all of `src/cli/build/`; the overview's population
      table shows several are majority-coverable (e.g. `test_mode.rs` 40%,
      `resources.rs` 74%).
- [ ] Replace/adjust the `src/main.rs` entry: its excepted dispatch body moved to
      `src/cli/dispatch.rs` (verified integration-only, overview §2). Add
      `src/cli/dispatch.rs` to the list with a boundary naming the
      subprocess/registry command dispatch — mirroring the old `src/main.rs`
      wording — while noting its pure `is_help_flag`/USAGE/`--version` arms are
      left to sub-plan B if they alone can reach ≥95% without the command bodies.
      Keep or drop the `src/main.rs` line per whether `src/main.rs` still exists
      and still contains excepted orchestration (`ls src/main.rs`; read it).
- [ ] For every entry added or changed, the comment must name the concrete
      syscall/network/subprocess/GUI boundary (the list's standing rule). No bare
      path.

Acceptance: after this phase, `sh scripts/coverage-check.sh` no longer reports
`src/cli/dispatch.rs` (and any newly-excepted `src/cli/build/*` orchestration
module) as a GATE FAILURE — they appear under "Documented exceptions" instead;
and no file with a coverable body was whole-file excepted (spot-check each new
entry against its source). `cargo test` still `0 failed` (exception list is not
compiled, so this is a no-op check confirming nothing else was touched).
Commit: — (touches only `scripts/coverage-exceptions.txt`)

### Phase A3 — Triage `src/os/windows/mod.rs` and finalize the worklist

`src/os/windows/mod.rs` is 0/38 and small. Decide its class, then freeze the
per-letter worklist B–H will consume.

- [ ] Read `src/os/windows/mod.rs`. Classify: if it is pure declarations /
      cfg-gated re-exports / a subprocess-driven Windows link driver with no
      unit-reachable logic on the macOS/Linux build, except it with the boundary
      named. If it holds coverable helpers, assign it to sub-plan C and leave it
      on the worklist. Record the decision + evidence in the triage table below.
- [ ] Resolve the two overview Open Decisions with A1's data:
      `src/arch/aarch64/backend.rs` (3/9) and `src/json.rs` (13/14) —
      coverable-vs-except, with the uncovered lines read from the fresh report.
- [ ] Fill the **Triage table** below for all 55 files: `except` (record the
      boundary) | `backfill:<letter>` | `denominator-fix`. This table is the
      authoritative worklist; the overview's letter column is provisional until
      this table confirms it.

Acceptance: the Triage table is complete for all 55 files; every `except` row has
a boundary reason present in `coverage-exceptions.txt`; `sh scripts/coverage-check.sh`
now lists only files marked `backfill:<letter>` as GATE FAILUREs (the excepted
ones moved to the "Documented exceptions" section). The residual failing count
equals the number of `backfill` rows.
Commit: — (any exception additions from A3 go in `scripts/coverage-exceptions.txt`)

## Triage table (the worklist — filled during A3)

| File | uncov | Decision | Evidence / boundary |
|---|---|---|---|
| src/cli/dispatch.rs | 195 | except (provisional) | ex-main.rs router; spawns linker via build_project + live registry via run_pkg/repo (verified, overview §2) |
| src/os/windows/mod.rs | 38 | TBD (A3) | read the file |
| src/arch/aarch64/backend.rs | 6 | TBD (A3) | read fresh report |
| src/json.rs | 1 | TBD (A3) | read fresh report |
| src/cli/build/signing.rs | 92 | except (repoint build.rs) | B verified: 92 uncov lines are all `load_build_signing_info` → `request_attestation` live registry; pure helpers already covered. This IS the boundary the stale `src/cli/build.rs` entry named — A2 repoints here |
| … (remaining files default to their overview letter unless A moves them) | | backfill:<letter> | |

## Exception / unreachable candidates surfaced by B–H (confirm against A1's fresh report)

The backfill sub-plans read their files and flagged these specific lines/arms as
NOT unit-coverable. A must confirm each against the fresh `coverage.json` and, if
truly unreachable-by-construction, add it as a **line-level** exception (or delete
it if it is dead code — `AGENTS.md`: no dead-code filler). Do NOT let a letter
chase these; they are A's to resolve so the file can still reach ≥95% on its
reachable lines.

| From | Site | Nature | A's action |
|---|---|---|---|
| B | `src/cli/build/signing.rs` (whole) | `load_build_signing_info` live-registry attestation | except at new path (repoints `build.rs`) |
| C | `src/os/linux/appimage/mod.rs` `seal` blob-length guard; `src/os/macos/icon.rs` icns `.map_err` arms | defensive, unreachable with valid input | line-level exception only if the file can't otherwise hit 95% (icon needs just 1 more line) |
| E | `src/ir/lower.rs:3108` `unreachable!("inline TRAP…")`; `src/ir/link.rs:585` `AbiDirection::Out => unreachable!`; `write_ir` `Err` arm | `unreachable!` guards + `std::fs` error arm | line-level exception; happy paths are E's backfill |
| F | `src/syntaxcheck/link.rs:844` `native_function_sig` `Type::Unknown` fallback; `Type::Result`/internal-only match arms in inference.rs/mod.rs | param type not source-spellable / exhaustiveness arms | confirm unreachable, then line-level exception (mirrors the existing `syntaxcheck/resources.rs` exception rationale) |
| G | `src/ast/scope_privates.rs:41-49` `PRIVATE_PATH_HASH_COLLISION` (`Some(prev) if prev != &file.path`) | fires only on a real `file_scope_hash` collision, not constructible | line-level exception. NOTE: use the exception FILE, not an inline `coverage:off` fence — cargo-llvm-cov 0.8.7 ignores inline markers (`coverage-exceptions.txt:7`) |
| H | `repository/src/main.rs::main()` socket-bind/live-HTTP tail (already fenced); `backfill.rs` `BlobFetch::Redirect` (S3-only); `src/docs/spec/mod.rs` `sort_by_key` fallback (package always in `PACKAGE_ORDER`); `src/manifest/json_edit.rs` 3 malformed-value arms | live I/O + unreachable defensive arms | except the `main.rs` tail with the socket/HTTP boundary named; line-level for the rest per the fresh report |
| H / Open Decision | `src/json.rs` (13/14, one line) | H judged the lone uncovered line an **infallible/dead** `stringify` fallback — contradicts the overview's "cover it" recommendation | READ it against the fresh report: if genuinely dead, **delete it** (→ 13/13 = 100%, no exception); if reachable, H covers it. Do NOT except a 14-line file for one line without proving unreachability |

## Validation Plan

- `sh scripts/coverage.sh` exits 0 and produces a current `coverage.json`.
- After A2+A3, `sh scripts/coverage-check.sh` shows the previously-excepted-then-
  moved files under "Documented exceptions", not GATE FAILURE.
- `cargo test` → `0 failed` (unchanged; confirms only the exception list moved).
- Each new exception line maps to a concrete boundary a reviewer can find in the
  named source file.

## Corrections

<Filled during execution — especially any delta between A1's fresh failing set
and the overview's 55-file table, with the measuring command.>
