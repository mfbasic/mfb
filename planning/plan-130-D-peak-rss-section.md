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
| plan-130-C complete | `ls planning/completed/plan-130-C-*` → one match | NOT MET |

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

- [ ] Prove the four Unix UNVERIFIED rows with a throwaway C probe per box (2223, 2227,
      2228, 2229, and macOS): print `offsetof(struct rusage, ru_maxrss)`, `sizeof`, and a
      64 MiB-touch value. Record results in Corrections.
- [ ] `emit_peak_rss_bytes` for macOS, linux-aarch64, linux-riscv64 (libc `getrusage`),
      linux-x86_64 (raw syscall 98); scale KiB → bytes on Linux.
- [ ] `ProcessFeature` + `_mfb_debug_report_process`; imports only in debug builds.
- [ ] `tests/runtime/rt_debug_report.rs`: host case — a program filling a 64 MiB
      `List OF Byte` reports `process.peak_rss_bytes >= 67108864`; a normal build's
      import table has no `getrusage`.

Acceptance: host test green; the same program's report on 2223, 2227 (musl), 2228
(glibc), 2229 shows `process.peak_rss_bytes >= 67108864` (recorded here); artifact
gate `0 diff(s)`.
Commit: —

### Phase 2 — Windows

- [ ] Prove the Windows UNVERIFIED row on 2230 (a PowerShell `Get-Process` peak
      working set alongside the program's report for the same 64 MiB program).
- [ ] `win_x86_64` `emit_peak_rss_bytes`: `GetCurrentProcess`, `K32GetProcessMemoryInfo`
      with `cb = sizeof(PROCESS_MEMORY_COUNTERS)` (72 on x64 — verify), result read via
      `c_return(0)`; imports in the debug import table only.

Acceptance: 2230 report shows `process.peak_rss_bytes >= 67108864` and within 10% of
PowerShell's `PeakWorkingSet64` for the same run (both recorded); artifact gate
`0 diff(s)`.
Commit: —

### Phase 3 — docs

- [ ] Debug-report spec page (plan-130-A Phase 4): `process` section, units, when read.

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

## Summary

Five small per-backend bodies behind one trait method; the risk is ABI detail on
Windows, and every uncertain offset is proven by running before it is coded.
