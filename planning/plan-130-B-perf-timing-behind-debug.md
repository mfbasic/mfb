# plan-130-B: move the plan-67 perf timings from `--cfg perf` behind `--debug`

Last updated: 2026-09-12
Effort: medium (1h–2h)
Depends on: plan-130-A

The plan-67 perf instrumentation (per-region timing of the program, `_mfb_arena_alloc`
and `_mfb_arena_free`) is switched on today only by building the **compiler** with
`RUSTFLAGS="--cfg perf"`, and it prints its table from the entry exit tail. This
letter makes it a `DebugFeature`: `mfb build --debug` turns it on, its table becomes
the `perf` section of the plan-130-A report, and the `cfg(perf)` build is retired.

Behavioral outcome: `mfb build --debug` on macOS prints `perf.*` lines inside the
report block, after `_mfb_shutdown`'s other work; a normal compiler build with
`--debug` absent emits no `_mfb_rt_perf_*` symbol on any target; there is no
`cfg(perf)` left in the tree.

References: plan-130-A (prerequisites table and §4.3/§4.4);
`src/docs/spec/memory/07_runtime-helper-abi.md` (perf section);
`planning/completed/plan-67-*.md` (design intent).

## Prerequisites

See plan-130-A § Prerequisites. Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-130-A complete | `ls planning/completed/plan-130-A-*` → one match | MET (2026-09-12: plan-130-A archived to `planning/completed/` after Phase 4 `0376a587a`; its full-suite gate moved to plan-130-E Phase 3 per the user) |

## 1. Goal

- `perf_injection_enabled()` and `cfg!(perf)` no longer exist; every former gate asks
  the debug registry.
- The perf feature applies only to `macos-aarch64` entry modules (unchanged scope).
- `perf.done`'s output appears as `perf.<region>.<stat> <value>` lines inside the
  report block, emitted by the perf feature's report section, not by the entry tail.
- Non-`--debug` builds: byte-identical to plan-130-A's end state.

### Non-goals

- No port of the timing helpers to Linux/Windows (the user's decision, 2026-09-12).
- No change to what is timed (`program`, `mfb_alloc`, `mfb_free`) or how.
- No change to the 16 MiB region layout (`PERF_REGION_SIZE`, tables A/B, sample log).

## 2. Current State

- Gate: `src/codegen/engine/builder/mod.rs::perf_injection_enabled()` =
  `cfg!(perf)`; registered in `Cargo.toml` lines 38–41 (`check-cfg = ['cfg(perf)']`).
- Injection sites (each `perf_injection_enabled() && <macOS>`):
  `builder/mod.rs` (data objects, `module.target == "macos-aarch64"`),
  `entry.rs` perf init/start after the main arena global is stored,
  `entry.rs` exit tail perf end/done after `bl _mfb_shutdown`,
  `arena.rs::perf_arena_enabled` (alloc/free regions),
  `target/shared/plan/symbols.rs` (`runtime_symbols` forcing the four helpers;
  `platform_imports` forcing `_clock_gettime`).
- Output: `perf.done` in `src/codegen/builtins/perf/perf.rs::lower_perf_helper`
  writes `name count avg median min max sum` rows to fd 2.
- Tests: `src/codegen/builtins/tests/perf.rs`.

### Measured populations

| What | Count | Command |
|---|---|---|
| Code sites calling `perf_injection_enabled()` | 6 | `git grep -n "perf_injection_enabled()" -- src \| grep -v "fn perf_injection_enabled" \| grep -vE ":\s*//"` → builder 1, entry 2, arena 1, symbols 2 |
| `#[test]`s in `src/codegen/builtins/tests/perf.rs` | 7 | `grep -c "#\[test\]" src/codegen/builtins/tests/perf.rs` |
| Files naming `--cfg perf` / `cfg(perf)` / `cfg!(perf)` | 13 | `git grep -lE "cfg perf\|cfg\(perf\)\|cfg!\(perf\)" -- . ':!planning' \| wc -l` → Cargo.toml, scripts/man-census.sh, perf/perf.rs, tests/perf.rs, builder/mod.rs, entry.rs, error_constants.rs, arena.rs, 07_runtime-helper-abi.md, macos_aarch64/code.rs, symbols.rs, runtime/mod.rs, perf_specs.rs |

### Verified properties

- *Stale docs describe a debug-compiler gate*: `.ai/testing-gates.md` § "Perf goldens
  break execution acceptance" and memory note `mfb-exe-tests-use-release-binary` say
  a **debug** mfb injects perf; the code says `cfg!(perf)` (read 2026-09-12). Both are
  corrected in Phase 3.
- UNVERIFIED: that the perf helpers' `perf.done` can be called from the report helper
  after `_mfb_arena_destroy` — they reference no arena symbol (test
  `no_perf_body_calls_an_arena_helper`) and read only `_mfb_rt_perf_state`, so it
  should hold; Phase 2's runtime test proves it.

## 3. Design Overview

`PerfFeature: DebugFeature` in `src/codegen/debug/perf.rs`: `applies` = entry module on
`macos-aarch64`; `data_objects`/`code_functions`/`runtime_symbols`/`imports` = what the
six gate sites add today; `emit_entry_start` = today's init/start; `report_symbol` =
a new helper `_mfb_debug_report_perf` that calls `perf.end("program")` then
`perf.done`. The entry exit tail loses its perf calls. `perf_arena_enabled(platform)`
becomes `debug_feature_applies::<PerfFeature>(module, platform)`; `lower_arena_alloc`
and `lower_arena_free` take the module's `DebugOptions` (they are emitted from
`lower_module_for_platform`, which has it).

Output rows move from the space-separated table to report lines: `perf.done`'s row
writer becomes `perf.<name>.count/avg/median/min/max/sum <n>` plus
`perf.mismatch`/`perf.overflow` when non-zero, using plan-130-A's `write.rs`.

**Risk:** low — the timed code is unchanged; the gate and the print site move. The one
behavioral shift is that `program` now ends after `_mfb_shutdown`'s other work instead
of before the entry reloads the exit code; it already ended after `bl _mfb_shutdown`,
so the measured span is unchanged.

**Rejected:** keeping `cfg(perf)` as a second switch (two ways to turn one thing on,
and CI never builds it); porting timings to all targets now (user decision).

## Phases

### Phase 1 — perf as a DebugFeature, gated by `--debug`

- [x] `src/codegen/debug/perf.rs`: `PerfFeature`; append to `DEBUG_FEATURES`. (`DEBUG_FEATURES = [&CoreSection, &perf::PerfFeature]`; data objects, runtime
      calls `perf.init/start/end/done`, import call `perf.start`, report `_mfb_debug_report_perf`.)
- [x] (moved here from plan-130-A Phase 2) `DebugEmitCtx` and a
      `DebugFeature::emit_entry_start(&self, ctx: &mut DebugEmitCtx)` hook, called by
      `lower_program_entry` for each active feature after the main arena global is stored
      (the site of today's perf init/start); `PerfFeature` implements it with the
      init/start calls. Landed here because `CoreSection` has no entry work and an
      unread context fails the warning-free tree.
- [x] Replace the 6 `perf_injection_enabled()` sites with registry queries; delete
      `perf_injection_enabled`; delete the `check-cfg` lines in `Cargo.toml`. Builder data objects -> `PerfFeature::data_objects`; entry init/start ->
      `emit_entry_start` via `ProgramEntrySpec::debug_features`; both `symbols.rs` blocks -> the
      registry loops; arena -> `lower_arena_alloc(platform, perf)` / `lower_arena_free(perf)` fed by
      `feature_active(module, PERF_SECTION)`. `git grep -nE "cfg\(perf\)|cfg!\(perf\)|--cfg perf|perf_injection_enabled" -- src Cargo.toml scripts` -> no matches.
- [x] Move perf end/done from the `entry.rs` exit tail into `_mfb_debug_report_perf`.
- [x] ~~the 7 tests build their platform/module with `DebugOptions { enabled: true }`~~ — moot:
      none of them relied on a cfg build; each calls `lower_perf_helper` directly with no gate
      (`grep -n "lower_perf_helper(" src/codegen/builtins/tests/perf.rs`), so they already ran
      in every CI build.
- [x] `src/codegen/builtins/tests/perf.rs`: add `a_normal_build_emits_no_perf_symbol` (macOS code
      plan of a non-debug build has no `_mfb_rt_perf_`; the same program with `--debug` carries
      all four helpers, `_mfb_rt_perf_state` and `_mfb_debug_report_perf`; a linux-aarch64
      `--debug` build carries none). `cargo test --bin mfb -- perf codegen::debug` -> 13 passed.

Acceptance: `cargo test --bin mfb perf` green; `git grep -nE "cfg\(perf\)|cfg!\(perf\)|--cfg perf" -- src Cargo.toml` → no matches; artifact gate `0 diff(s)`.
Commit: fedd21989

### Phase 2 — report format and runtime proof

- [x] `perf.done` rows rewritten as `perf.*` report lines (§3). The header line is gone; each table A entry prints six
      `perf.<name>.<stat> <value>` lines and the header counters print `perf.mismatch <n>` /
      `perf.overflow <n>` when non-zero, each assembled right-to-left by `emit_perf_line` in a
      96-byte stack window (`PERF_LOCALS_SIZE`, was 32) and written with one `write`. The old
      column writers (`emit_write_i64*`, `emit_write_name`) and `PERF_HEADER_SYMBOL` are deleted;
      the key pieces are new `PERF_KEY_*_SYMBOL` objects.
      `cargo test --bin mfb -- perf codegen::debug` -> 13 passed; artifact gate 0 diff(s).
- [x] `tests/runtime/rt_debug_report.rs`: a macOS `--debug` build's report block
      contains `perf.program.count 1`, `perf.mfb_alloc.count <n ≥ 1>`,
      `perf.mfb_free.count <n>`; the same program built for linux-aarch64 with
      `--debug` contains no `perf.` line. `assert_block` requires `perf.program.count 1` plus the other five
      `perf.program.*` statistics on every macOS exit path (and no `perf.` line elsewhere); the
      new `the_perf_section_times_the_arena` requires `perf.mfb_alloc.count >= 1` and all six
      statistics for any arena span it reports; `every_cross_target_calls_the_report_right_after_shutdown_done`
      requires no `_mfb_rt_perf_` in the linux-aarch64/x86_64/riscv64 and windows `--debug` dumps.
      `cargo test --release --no-fail-fast --test rt_debug_report` -> 7 passed.

Acceptance: `cargo test --release --test rt_debug_report` green.
Commit: —

### Phase 3 — docs

- [x] `07_runtime-helper-abi.md` perf section: gate is `--debug`; output is the report. (Landed in Phase 1 `fedd21989`.)
- [x] Doc comments at the former gate sites, `perf.rs` module doc, `runtime/mod.rs`,
      `perf_specs.rs`, `error_constants.rs`, `macos_aarch64/code.rs`,
      `scripts/man-census.sh` comment. (Landed in Phase 1 `fedd21989`.)
- [x] (added) Debug-report spec page (`09_debug-report.md`): the `perf` section's keys, units,
      and macOS-only scope. (`cargo test --bin mfb -- spec citations_resolve` -> 43 passed.)
- [x] `.ai/testing-gates.md` § "Perf goldens break execution acceptance": rewrite — no
      goldens carry perf output now; the cause was a `cfg(perf)`/debug build.
- [x] Memory note `mfb-exe-tests-use-release-binary`: correct the "debug injects perf"
      line (memory edits via a sub-agent, per AGENTS.md).
- [~] Full suite + artifact gate + test-accept as in plan-130-A Phase 4. Scoped per the user (2026-09-12): spec tests 43 passed; artifact gate 0 diff(s) and
      `rt_debug_report` 7 passed on the Phase 2 code (no code change since). Remaining: the full
      `cargo test` and `test-accept.sh` run once at the end of plan-130 (plan-130-E Phase 3).

Acceptance: `git grep -nE "cfg perf|cfg\(perf\)" -- . ':!planning'` → no matches; the
three suites green.
Commit: —

## Validation Plan

- Tests: `src/codegen/builtins/tests/perf.rs` (7 updated + 1 new),
  `tests/runtime/rt_debug_report.rs` perf cases.
- Runtime proof: macOS `--debug` run prints perf lines; linux-aarch64 `--debug` prints none.
- Doc sync: Phase 3 list.

## Open Decisions

- None beyond plan-130-A's.

## Corrections

- **Phase 3 — the testing-gates claim was false, not stale.** Measured: no committed golden names
  `_mfb_rt_perf_` or carries the perf table (the only test-tree hit is `rt_debug_report`'s own
  no-perf assertion), and the artifact gate reads `0 diff(s)` with the RELEASE binary, so its
  advice to run the gate with a debug `mfb` "so perf symbols match" had no basis. The section is
  rewritten under a new heading, "Perf timings never reach a golden (plan-130-B)".
- **Phase 2 — "the same program built for linux-aarch64 contains no `perf.` line" is checked on
  the dump, not a run.** This host cannot run a Linux binary; the no-perf property is what the
  code emits, so `rt_debug_report` asserts no `_mfb_rt_perf_` symbol in each non-macOS `--debug`
  `.ncode`, and a non-macOS `assert_block` requires zero `perf.` lines. `perf.mfb_free.*` is
  asserted only when present (a program need not free).
- **Phase 1 — docs landed early.** Phase 1's acceptance grep covers `src` doc comments, so the
  `--cfg perf` prose in `perf.rs`, `tests/perf.rs`, `error_constants.rs`, `runtime/mod.rs`,
  `perf_specs.rs`, `macos_aarch64/code.rs`, `scripts/man-census.sh` and
  `07_runtime-helper-abi.md` was rewritten in Phase 1, not Phase 3.
- **Phase 1 — `lower_arena_free` lost its `platform` parameter.** Its only use was the perf gate;
  keeping it would leave an unused parameter warning.
- **Phase 1 -> 2 — `rt_debug_report` is red on macOS between the two phases.** Once perf is a
  report section, the macOS `--debug` block contains `perf.done`'s old space-separated table,
  which the test's exact four-line block rejects; Phase 2 rewrites the rows as `perf.*` report
  lines and updates the test.
## Summary

A gate move and a print-site move; the timing code is untouched. The only care needed
is that no former `cfg(perf)` site is left asking a gate that no longer exists.
