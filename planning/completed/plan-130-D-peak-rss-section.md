# plan-130-D: the `process` report section — peak resident set size on all five targets

Last updated: 2026-09-12
Effort: medium (1h–2h)
Depends on: plan-130-C

A `--debug` build's report gains a `process` section with the process's peak
resident set size, read at report time by the OS call each platform provides. Arena
counters (plan-130-C) say how much the allocator *mapped*; this says how much memory
the process *actually touched*, which is the number the A/B tests in
`planning/todo.md` § Memory section 3 compare.

Behavioral outcome: every `--debug` report contains `process.peak_rss_bytes <n>`, in
bytes on every target, and for a program that allocates and fills 64 MiB the value is
at least 64 MiB on every target.

References: plan-130-A (§4.3 registry, §4.4 format); `.ai/arch-abi.md` (per-arch call
traps: Win64 C results in `rax` via `c_return(0)`, x86-64 foreign-call alignment);
`src/target/linux_common/plan.rs` (import tables, `raw_write`).

## Prerequisites

See plan-130-A § Prerequisites. Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-130-C complete | `ls planning/completed/plan-130-C-*` → one match | MET (2026-09-12) |

## 1. Goal

- `process.peak_rss_bytes` in every `--debug` report on macOS aarch64, Linux aarch64
  (glibc, musl), Linux x86_64 (glibc, musl), Linux riscv64, Windows x86_64.
- The value is bytes on every target (Linux `ru_maxrss` is KiB and is scaled).
- Non-`--debug` builds unchanged (no new import in any non-debug program).

### Non-goals

- No RSS time series and no per-thread RSS (the OS has no per-thread RSS); see Open
  Decisions.
- A debug build of a program that imports nothing on linux-x86_64 stays a static ELF.

## 2. Current State

- No backend imports a resource-usage call today:
  `git grep -nE "getrusage|GetProcessMemoryInfo|K32GetProcessMemoryInfo|task_info" -- src ':!src/docs'` → no matches.
- linux-x86_64 raw-syscalls `write`/`exit`/`mmap`/`getrandom` and links libc
  dynamically only when something imports (`src/target/linux_x86_64/plan.rs` module
  doc; `LinuxAbi.raw_write: true`).
- macOS imports libSystem symbols through its plan (`src/target/macos_aarch64/plan.rs`
  `runtime_imports`); Windows imports kernel32 through its app/console import tables.

### Verified properties

- UNVERIFIED — macOS `getrusage(RUSAGE_SELF, &ru)`: `ru_maxrss` is **bytes** on Darwin,
  at offset 32 of `struct rusage` (two 16-byte `timeval`s precede it). Phase 1 proves
  it by running.
- UNVERIFIED — Linux `ru_maxrss` is **KiB**, same offset 32 on LP64 glibc and musl.
- UNVERIFIED — x86_64 Linux `getrusage` is syscall 98; aarch64 and riscv64 use the libc
  import (their `write` is already a libc call, so no static-ELF property is lost).
- UNVERIFIED — Windows `K32GetProcessMemoryInfo` (kernel32, Windows 7+) fills
  `PROCESS_MEMORY_COUNTERS` whose `PeakWorkingSetSize` (SIZE_T) is at offset 8 after
  `cb`/`PageFaultCount`; needs `GetCurrentProcess()` as the handle.

Each UNVERIFIED row becomes the first task of its phase, proven by a run, not a read.

## 3. Design Overview

- New `CodegenPlatform` method `emit_peak_rss_bytes(dst: &str, instructions) -> Result<(), String>`
  with per-backend bodies (`macos_aarch64/code.rs`, `linux_common/code.rs` delegating to
  the arch, `linux_x86_64/code.rs` raw syscall, `win_x86_64/code.rs`). Each reserves its
  output struct on the stack, never the arena (the report runs after
  `_mfb_arena_destroy`).
- `ProcessFeature: DebugFeature` (`src/codegen/debug/process.rs`): `applies` = every
  entry module; `imports` = the backend's call when not a raw syscall; report helper
  `_mfb_debug_report_process` writes `process.peak_rss_bytes`.
- Placed after `arena` in `DEBUG_FEATURES`.

**Risk:** per-backend foreign-call ABI (Win64 shadow space, result register) —
concentrated in the Windows body, proven on 2230.

**Rejected:** reading `/proc/self/status` (Linux-only, needs file I/O in a helper that
must not allocate); `mach_task_basic_info` on macOS (more imports, and `getrusage`
already gives the peak).

## Phases

### Phase 1 — Unix targets

- [x] Prove the four Unix UNVERIFIED rows with a throwaway C probe per box (2223, 2227,
      2228, 2229, and macOS): print `offsetof(struct rusage, ru_maxrss)`, `sizeof`, and a
      64 MiB-touch value. Record results in Corrections.
      `/tmp/p130-d-rusage.c` (malloc + memset 64 MiB between two `getrusage` calls):
      | target | offsetof ru_maxrss | sizeof rusage | maxrss before → after | SYS_getrusage |
      | macOS aarch64 | 32 | 144 | 1,032,192 → 68,141,056 (bytes) | — |
      | 2223 aarch64 glibc | 32 | 144 | 4,404 → 66,596 (KiB) | 165 |
      | 2227 x86_64 musl | 32 | 272 | 2,476 → 65,972 (KiB) | 98 |
      | 2228 x86_64 glibc | 32 | 144 | 4,392 → 66,732 (KiB) | 98 |
      | 2229 riscv64 musl | 32 | 272 | 2,400 → 65,928 (KiB) | 165 |
- [x] `emit_peak_rss_bytes` for macOS, linux-aarch64, linux-riscv64 (libc `getrusage`),
      linux-x86_64 (raw syscall 98); scale KiB → bytes on Linux.
      Landed as one emitter in `src/codegen/debug/process.rs` (`lower_report`) branching on
      `platform.family()`, with libc `getrusage` on every Linux target (see Corrections).
- [x] `ProcessFeature` + `_mfb_debug_report_process`; imports only in debug builds.
      `DebugFeature::os_imports` → `NativePlanPlatform::peak_rss_imports` (a required method on
      all five backends), pushed by `plan::symbols::platform_imports` with the entry's
      attribution. `cargo test --bin mfb -- codegen::debug target::shared::plan` → 5 passed.
- [x] `tests/runtime/rt_debug_report.rs`: host case — a program filling a 64 MiB
      `List OF Byte` reports `process.peak_rss_bytes >= 67108864`; a normal build's
      import table has no `getrusage`.
      `a_64_mib_string_reports_at_least_64_mib_peak_rss` (a string doubled 26 times, see
      Corrections) and `only_a_debug_build_imports_the_peak_rss_call` (all five targets' ncode
      dumps); `assert_block` now requires one positive `process.peak_rss_bytes` line in every
      report. `cargo test --release --no-fail-fast --test rt_debug_report --test
      rt_debug_arena` → 9 passed, 5 passed.

Acceptance: host test green; the same program's report on 2223, 2227 (musl), 2228
(glibc), 2229 shows `process.peak_rss_bytes >= 67108864` (recorded here); artifact
gate `0 diff(s)`.
Measured: every box prints `67108864` and reports 2223 aarch64 glibc `135360512`, 2227 x86_64
musl `134791168`, 2228 x86_64 glibc `135622656`, 2229 riscv64 musl `134569984` (the final
`s & s` holds the old and new string, ~128 MiB). Artifact gate: `1436 tests, 1602 build(s),
2011 golden(s) checked, 0 diff(s)`.
Commit: —

### Phase 2 — Windows

- [x] Prove the Windows UNVERIFIED row on 2230 (a PowerShell `Get-Process` peak
      working set alongside the program's report for the same 64 MiB program).
      Layout proven before coding (Corrections). Same-run comparison: the program holds its
      64 MiB string for 2 s (`os::sleep(2000)`) while `/tmp/p130-d-win.ps1` samples
      `Process.PeakWorkingSet64` every 50 ms: PowerShell `138399744`, report
      `process.peak_rss_bytes 138399744` (identical); exit 0, stdout `67108864`.
- [x] `win_x86_64` `emit_peak_rss_bytes`: `GetCurrentProcess`, `K32GetProcessMemoryInfo`
      with `cb = sizeof(PROCESS_MEMORY_COUNTERS)` (72 on x64 — verify), result read via
      `c_return(0)`; imports in the debug import table only.
      The Windows branch of `process.rs` `lower_report` (landed with Phase 1's emitter);
      `only_a_debug_build_imports_the_peak_rss_call` covers `windows-x86_64`.

Acceptance: 2230 report shows `process.peak_rss_bytes >= 67108864` and within 10% of
PowerShell's `PeakWorkingSet64` for the same run (both recorded); artifact gate
`0 diff(s)`.
Commit: —

### Phase 3 — docs

- [x] Debug-report spec page (plan-130-A Phase 4): `process` section, units, when read.
      `src/docs/spec/tooling/09_debug-report.md` § Sections: the `process` row.

Acceptance: `cargo test -p mfb --bins citations_resolve` green; full suite green.
Commit: —

## Validation Plan

- Tests: `tests/runtime/rt_debug_report.rs` process case (host); box runs recorded here.
- Runtime proof: the 64 MiB program on all five targets.
- Doc sync: debug-report spec page.

## Open Decisions

- **RSS over time** (the todo's "RSS over time per thread") — recommended: not a
  sampling thread; plan-130-C's per-arena `peak_live_bytes` high-water mark already
  gives a per-thread over-time signal, and process peak here gives the OS's view.
  Alternative: a debug-only sampler thread writing `process.rss_sample.<ms> <bytes>`
  lines (costs a thread and a timer per target).

## Corrections

- **Phase 1 — no raw syscall on linux-x86_64.** A `--debug` entry already links libc there
  (`clock_gettime` for the arena seed, the arena registry's `pthread_mutex_*`), so a raw
  `getrusage` syscall would protect no static-ELF property; every Linux target calls libc
  `getrusage`, one emitter for all Unix targets.
- **Phase 1 — imports are per backend, not a runtime-call spec.** `DebugFeature::import_calls`
  only resolves catalogued runtime calls; a peak-RSS spec would have added a catalog family
  row for a call no program makes. Instead each backend names its symbols in the required
  `NativePlanPlatform::peak_rss_imports`, and the plan attributes them to the entry (a
  code-layer helper is not a valid relocation source, as for the arena lock imports).
- **Phase 1 — the buffer is sized for musl (272 bytes) and the peak word is zeroed first,** so a
  failed call prints 0 rather than stack contents.
- **Phase 1 — the test program doubles a string instead of filling a `List OF Byte`:** 26
  doublings reach exactly 67,108,864 bytes (printed and asserted), with no collection API.
- **Phase 2 layout, proven before coding (2230, no C compiler: a PowerShell `Add-Type` P/Invoke
  probe, `/tmp/p130-d-pmc.ps1`).** `K32GetProcessMemoryInfo(GetCurrentProcess(), buf, cb)`
  succeeds with `cb = 72` and fails with `cb = 64`, so `sizeof(PROCESS_MEMORY_COUNTERS)` is 72.
  The SIZE_T at offset 8 is the peak working set: 80,699,392 before touching 64 MiB and
  143,589,376 after (+62.9 MB); `Get-Process` `PeakWorkingSet64` read just after was
  148,934,656. Offset 16 is the current working set.
- **Phase 1 — musl's `struct rusage` is 272 bytes, not 144.** The offset of `ru_maxrss` is 32
  on every Unix target (verified rows 1–3 hold), but musl reserves 16 `long`s where glibc and
  Darwin reserve fewer, so the stack buffer for the call must be at least 272 bytes; the
  kernel's own struct is 144. Linux units are KiB (66,596 after touching 64 MiB = 65 MiB) and
  Darwin's are bytes. `getrusage` is syscall 98 on x86_64 and 165 on aarch64/riscv64.

## Summary

Five small per-backend bodies behind one trait method; the risk is ABI detail on
Windows, and every uncertain offset is proven by running before it is coded.
