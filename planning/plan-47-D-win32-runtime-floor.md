# plan-47-D: the Win32 console runtime floor

Last updated: 2026-07-20
Effort: medium (1h–2h)
Depends on: plan-47-B (a Win64 backend and the `CodegenPlatform` stub wall to fill in),
plan-47-C2 (a PE writer wired to that backend — needed for the *proof*, not the code).
Feature-wide precondition: master §Prerequisites.
Produces: `src/target/win_x86_64/{code,plan}.rs` — a working `emit_libc_call` IAT path
(**every later surface's only mechanism**), the Win32 entry/arena/write/exit floor, the
kernel32 import tables, and `executable: true`. Consumed by D, S, F4.

Makes `hello.exe` real. This is the sub-plan where `mfb build -target windows-x86_64`
stops producing a rejected target and starts producing a program that runs.

The single behavioral outcome: `mfb build -target windows-x86_64 hello` produces
`hello.exe` which, run on the Win11 box, prints byte-identical stdout to the Linux
x86-64 build of the same program and exits `0` — for any program using only integers,
strings, collections and `io::print`.

References (read first):

- `src/target/macos_aarch64/code.rs:38` — **the shape to mirror.** A standalone
  `CodegenPlatform` impl for a whole OS. *Not* `linux_common/code.rs:302`, which is
  parameterized over three arches by a `LinuxArch` delta and is the wrong model for a
  new OS (master §2.2).
- `src/target/linux_x86_64/plan.rs:53` — the `NativePlanPlatform` impl to mirror
  (7 required methods + 1 defaulted).
- `src/target/shared/code/entry_and_arena.rs:4` (`lower_program_entry`), especially
  **`:40`** — `let args_in_registers = platform.entry_args_in_registers() || …`. §3.1.
- `src/target/shared/code/mod.rs:712` — `skip_entry_arena_destroy`, a
  `starts_with("linux")` test. §3.2.
- `src/target/linux_common/code.rs:679` — `emit_libc_call`, the mechanism every OS call
  rides. Windows reuses it verbatim; only the import *library* differs.
- `src/target.rs:94` — `BackendCapabilities.executable`, and `:280` — the gate whose
  message is "native executable output does not support windows-x86_64 yet".

## Prerequisites

See the master §Prerequisites for the feature-wide gate; and:

| Must be true | Command | Status 2026-07-20 |
|---|---|---|
| plan-47-A has landed (branches are exhaustive) | `rg -n 'enum PlatformFamily' src/` | **NOT MET** |
| plan-47-B has landed (Win64 backend + stub wall) | `ls src/target/win_x86_64/` | **NOT MET** |
| plan-47-C2 has landed (PE writer reachable from the backend) | `ls src/os/windows/` | **NOT MET** |
| The Win11 box answers | `ssh -p 2230 test@127.0.0.1 true` | **UNVERIFIED — run it** |

> **NOTE — the Status column is a snapshot; the Command column is the truth.** Re-run
> every row before you continue and again before you decide to stop. If you stop, report
> all four statuses.

## 1. Goal

- `src/target/win_x86_64/code.rs` implements, for real, the machine-floor subset of the
  54 required `CodegenPlatform` methods (§2.1): `emit_program_entry`,
  `emit_program_exit`, `emit_arena_map`, `emit_arena_unmap`, `emit_write`,
  `emit_random_bytes`, `emit_errno`, `emit_libc_call`, `emit_variadic_call`.
- `src/target/win_x86_64/plan.rs` implements `NativePlanPlatform`'s 7 required methods,
  producing kernel32/shell32/bcrypt `PlatformImport`s.
- The backend advertises `executable: true` and a `runtime_calls` set covering exactly
  the floor — **nothing more**. An unimplemented surface is a clean compile-time
  rejection, never a broken `.exe`.
- Program arguments work: `GetCommandLineW` → `CommandLineToArgvW` → UTF-16→UTF-8 into
  the `os::args` globals.
- `hello.exe`, an integer-arithmetic program, and a string/collection program all run
  correctly on Windows.

### Non-goals (explicit constraints)

- **No file, terminal, thread, socket or crypto surface.** Those are D/E/F/G/H. If a
  program using them compiles for Windows after this sub-plan, `runtime_calls` is wrong.
- **No app mode.** `supports_app_mode()` returns `false`; the 8 app-mode trait methods
  are defaulted (master §2.1) and stay that way.
- **Do not touch `emit_libc_call`'s shared contract.** Windows reuses it verbatim.
- **Do not fill in `unreachable!` arms plan-47-A left for other sub-plans.** If this
  sub-plan needs one, that is a signal the floor is bigger than scoped — record it in
  §Corrections rather than reaching into E/G's territory.

## 2. Current State

### 2.1 Measured populations

| What | Count | Command |
|---|---|---|
| `CodegenPlatform` methods a new OS must author | **54** required of 65 (11 defaulted) | master §2.1 |
| — of those, the machine floor this sub-plan implements for real | **9** | the list in §1 |
| — POSIX ABI constants (fabricated here, corrected by 47-E) | **21** | master §2.1 |
| — app-mode (defaulted; Windows inherits free) | 8 | `… \| grep -c '^app_\|^emit_app_'` |
| `NativePlanPlatform` methods | 8 (7 required + `app_mode_imports` defaulted) | `awk '/trait NativePlanPlatform/,/^}/' src/target/shared/plan/mod.rs \| grep -cE '^\s+fn '` |
| kernel32/shell32/bcrypt imports the floor needs | **9** | §4.4 |

### 2.2 The two shared edits this sub-plan cannot avoid

Both are in code every backend compiles, so both are byte-identity-gated.

**§3.1 — the entry seam is a `bool`.** `entry_and_arena.rs:40` reads:

```rust
let args_in_registers = platform.entry_args_in_registers() || entry_called_as_function;
```

macOS delivers argc/argv in `ARG[0]`/`ARG[1]`; a raw Linux ELF entry has them at
`[sp,0]` and `sp+8`. **Windows is a third case** — neither: the entry takes no arguments
at all, and the command line is *fetched* via `GetCommandLineW`. A `bool` has no third
value, so this method's shape must change. The 2026-07-14 master said C would "extend
`entry_and_arena.rs` so `entry_args_in_registers`'s Windows case…" — there is no Windows
case to extend a boolean into.

**§3.2 — `skip_entry_arena_destroy` is a `starts_with("linux")` test**
(`mod.rs:712`). It governs whether the entry destroys the arena while worker threads may
still be live. Windows currently gets `false` by fallthrough. That is an *undeliberated*
answer, not a decided one, and it is a use-after-free question. Decide it explicitly.

### 2.3 Verified properties

| Claim | Verdict | How checked |
|---|---|---|
| `emit_libc_call` is OS-neutral and reusable for IAT calls | **CONFIRMED** | `linux_common/code.rs:679` emits `bl symbol` + an external `RelocIntent::Call`; only the import library differs |
| `entry_args_in_registers()` returns `bool` and has no third state | **CONFIRMED** | `entry_and_arena.rs:40`; the method is one of the 11 defaulted |
| macOS is the right impl shape to mirror, not Linux | **CONFIRMED** | `macos_aarch64/code.rs:38` is standalone; `linux_common/code.rs:302` is `impl<A: LinuxArch>` over three arches |
| Windows needs no `environ` pointer | **CONFIRMED** | no Windows analog; `GetEnvironmentStringsW` is the replacement, and `emit_environ_pointer` is only reached by `os::` calls this sub-plan does not advertise |
| The floor needs 9 imports | **CONFIRMED** | §4.4; note the master's overview said "six" while its own Phase C required the time import |
| `hello.exe` runs correctly on Windows | **UNVERIFIED — this is the acceptance criterion** | proven on the Win11 box, not by reasoning |

## 3. Design Overview

Four pieces:

1. **The entry seam (shared edit).** Replace the `bool` with a three-valued
   `EntryArgsSource { Registers, Stack, Fetched }` returned by a defaulted trait method.
   `Registers` and `Stack` reproduce today's two arms byte-identically; `Fetched` is the
   Windows path and is unreachable until this backend registers.
2. **The Win32 platform impl** — the 9 floor methods, each an IAT call through the
   existing `emit_libc_call`.
3. **The import tables** — `NativePlanPlatform`'s 7 methods returning kernel32/shell32/
   bcrypt `PlatformImport`s.
4. **Capability advertisement** — `executable: true` plus a floor-only `runtime_calls`.

**Where design uncertainty concentrates: the entry path, and only there.** Every other
method is a mechanical translation of a syscall into a documented Win32 call with a
known signature. The entry is different: it changes a shared seam's *type*, it runs
before the arena exists, and its UTF-16→UTF-8 conversion has nowhere to allocate yet.
**Phase 1 is a spike against exactly that** — a `.exe` that fetches and prints its own
argv and nothing else — because if the pre-arena conversion doesn't work, the shape of
this sub-plan changes.

**Where correctness risk concentrates:**

- **The 21 fabricated POSIX constants.** This sub-plan must return *something* from
  `termios_size()`, `stat_mode_offset()`, `sol_socket()` and the rest, because they are
  required and return plain `usize`. Every value is a lie until 47-E removes them. Make
  them loudly wrong (a poison value like `usize::MAX`) rather than plausibly wrong (`0`),
  so a path that reaches one crashes instead of silently mis-addressing.
- **`skip_entry_arena_destroy`** (§3.2) — a use-after-free question currently answered by
  fallthrough.

**Rejected alternative:** *link the UCRT and use `main`/`argv` directly.* Rejected — a
hard non-goal (no external toolchain or CRT dependency), and it would make the entry path
diverge from every other target's raw entry.

**Rejected alternative:** *return `true` from `entry_args_in_registers()` and stage
argc/argv into `ARG[0]`/`ARG[1]` before calling the shared path.* Tempting — it avoids
the shared edit entirely. Rejected: it means fetching and converting the command line
*before* the frame is carved and before the arena exists, in the one place where there is
no scratch space, purely to satisfy a boolean. The three-valued enum is honest and the
shared edit is small.

## 4. Detailed Design

### 4.1 The entry

```
_start:                       ; raw entry, no arguments
  GetCommandLineW()           -> LPWSTR  (static, no free)
  CommandLineToArgvW(cmd, &argc) -> LPWSTR*  (shell32; LocalFree'd after conversion)
  <carve frame, VirtualAlloc the arena>
  <UTF-16 -> UTF-8 each argv[i] into arena, populate os::args globals>
  <call main>
  ExitProcess(status)
```

Ordering constraint: `CommandLineToArgvW` allocates with `LocalAlloc`, so the UTF-8
conversion must land in the arena and the LPWSTR* must be `LocalFree`d after. That means
the arena is created **before** args are materialized — the reverse of the raw-ELF path,
which reads argv off the stack before carving. This is why the entry is a third case and
not a variant of either existing one.

### 4.2 Arena

`VirtualAlloc(NULL, size, MEM_COMMIT|MEM_RESERVE, PAGE_READWRITE)` for `emit_arena_map`;
`VirtualFree(ptr, 0, MEM_RELEASE)` for `emit_arena_unmap`. Both return/take a pointer
directly — simpler than `mmap`'s six arguments, and well inside the 4-register Win64
cap so no stack tail is needed here.

### 4.3 Write, exit, RNG, errno

- `emit_write`: `GetStdHandle(STD_OUTPUT_HANDLE / STD_ERROR_HANDLE)` then
  `WriteFile(h, buf, len, &written, NULL)` — **5 arguments, so the 5th goes on the stack
  above the shadow space.** This is the floor's dependency on 47-B's outgoing tail.
- `emit_program_exit`: `ExitProcess(code)`.
- `emit_random_bytes`: `BCryptGenRandom(NULL, buf, len, BCRYPT_USE_SYSTEM_PREFERRED_RNG)`.
- `emit_errno`: **there is no `errno`.** Windows reports failure via `GetLastError()`.
  The shared EINTR-retry constructs that read `errno` are POSIX-shaped; the floor's
  calls do not need retry, so `emit_errno` returns a `GetLastError()` call here and the
  divergence is documented for 47-F/G to confront properly.

### 4.4 The import set (9)

| DLL | Symbols |
|---|---|
| kernel32 | `GetStdHandle`, `WriteFile`, `VirtualAlloc`, `VirtualFree`, `ExitProcess`, `GetCommandLineW`, `GetSystemTimePreciseAsFileTime` |
| shell32 | `CommandLineToArgvW` |
| bcrypt | `BCryptGenRandom` |

(`LocalFree` is kernel32 too if §4.1's free is emitted — verify during Phase 1 and
correct this table rather than leaving it at 9 by assumption.)

## Compatibility / Format Impact

- **New:** `src/target/win_x86_64/{code,plan}.rs`; `windows-x86_64` becomes an
  executable target.
- **Changed (shared):** `entry_args_in_registers() -> bool` becomes
  `entry_args_source() -> EntryArgsSource`; `skip_entry_arena_destroy` gains an explicit
  Windows answer. Both must leave all four existing targets byte-identical.
- **Unchanged:** `emit_libc_call`'s contract, the PE writer, the language/IR, and every
  other backend's behavior.

## Phases

> **Keep the checkboxes current as you go — tick `- [x]` in the same commit as the work.**
> An unticked box means NOT DONE.

### Phase 1 — spike: the entry path (falsifies the one unproven premise)

The smallest `.exe` that fetches its own command line and prints it. **Do not build the
rest until this runs on Windows.**

- [ ] Change `entry_args_in_registers() -> bool` to `entry_args_source() -> EntryArgsSource`
      (defaulted), with `Registers`/`Stack` reproducing today's arms; `entry_and_arena.rs:40`
      matches on it.
- [ ] Prove byte-identity for all four existing targets **before** adding the Windows arm.
- [ ] Implement the `Fetched` arm: `GetCommandLineW` → `CommandLineToArgvW` → arena →
      UTF-8 → `os::args`, with `LocalFree`.
- [ ] Runtime: a program that prints its argv, run on the Win11 box with several
      arguments including a non-ASCII one.

Acceptance: `scripts/artifact-gate.sh` 0 diffs on all four existing targets after the
seam change alone; then the argv program prints the right arguments on Windows,
including the non-ASCII one. If the pre-arena ordering in §4.1 does not work, stop and
redesign §3 — that is what this phase exists to find out.
Commit: —

### Phase 2 — arena, write, exit

- [ ] `emit_arena_map`/`emit_arena_unmap` over `VirtualAlloc`/`VirtualFree`.
- [ ] `emit_write` over `GetStdHandle` + `WriteFile` — **exercises 47-B's stack-arg tail
      (5 arguments)**.
- [ ] `emit_program_exit` over `ExitProcess`.
- [ ] `plan.rs`: `NativePlanPlatform`'s 7 required methods, kernel32 imports.
- [ ] Runtime: `hello.exe` prints and exits 0 on Windows.

Acceptance: `hello.exe` stdout is **byte-identical** to the linux-x86_64 build's, exit
code 0, on the Win11 box.
Commit: —

### Phase 3 — the fabricated-constant wall and capability advertisement

- [ ] Fill the 21 POSIX constant accessors with **poison values** (`usize::MAX`-style),
      never plausible ones, each with a comment naming the sub-plan that will remove it
      (47-E). A path that reaches one must crash, not mis-address.
- [ ] `emit_random_bytes` over `BCryptGenRandom`; `emit_errno` over `GetLastError`.
- [ ] Decide `skip_entry_arena_destroy` for Windows explicitly (§3.2) and comment the
      reasoning — it is a use-after-free question.
- [ ] `BackendCapabilities.executable = true`; `runtime_calls` = the floor set only.
- [ ] Tests: a program using `fs::`/`term::`/`thread::`/`net::` is **rejected at compile
      time** for `-target windows-x86_64`, with the unsupported-runtime-call diagnostic.

Acceptance: floor programs build and run; every non-floor surface is rejected at compile
time rather than producing a broken `.exe`; no poison constant is reachable from any
advertised call.
Commit: —

### Phase 4 — the real proof (largest blast radius last)

- [ ] Integer arithmetic, string building, and collection programs run on Windows with
      byte-identical stdout to their linux-x86_64 builds.
- [ ] Add the Windows fixtures to the acceptance suite and seed their goldens.

Acceptance: three non-trivial programs produce byte-identical stdout on Windows and
Linux, and exit with the same codes.
Commit: —

## Validation Plan

- Tests: per phase. The compile-time-rejection test in Phase 3 is the one that keeps
  partial progress shippable — without it, an unimplemented surface produces a broken
  `.exe` instead of a diagnostic.
- Coverage check: the shared seam change (Phase 1) is guarded by
  `scripts/artifact-gate.sh` — but `linux-riscv64` has **zero** native goldens (master
  §Prerequisites row 3), so that guard is vacuous there. Seed them, or the entry-seam
  change is unguarded on riscv64.
- Runtime proof: the Win11 box (ssh port 2230). **Byte-identical stdout against the
  linux-x86_64 build**, not "looks right" — a plausible-looking string is exactly what a
  wrong UTF-16 conversion produces.
- Doc sync: `mfb spec memory 08_program-startup` gains the Windows entry sequence.
- Acceptance: the full suite, plus `scripts/artifact-gate.sh` 0 diffs on the four
  existing targets.

## Open Decisions

1. **`entry_args_source()` shape** — a three-valued enum (recommended) vs. keeping the
   bool and staging argv before the shared path. Recommended the enum: the alternative
   does pre-arena work in the one place with no scratch space, purely to preserve a
   boolean. (§3, Rejected alternatives)
2. **What the 21 fabricated constants return.** Recommended poison values that crash on
   use, not `0`. A plausible zero is how a wrong offset silently mis-addresses; a poison
   value turns the same bug into an immediate, attributable crash. (§Phase 3)
3. **`skip_entry_arena_destroy` for Windows.** Recommended `false` (matching the
   fallthrough) **only after** confirming no Windows thread path exists yet — which is
   true until 47-H4. Re-decide when F4 lands; record the dependency here. (§3.2)

## Corrections

<!-- Filled in during execution. -->

- 2026-07-20 — **The floor needs 9 imports, not "six kernel32".** The 2026-07-14 master
  said six in its overview while its own Phase C required
  `GetSystemTimePreciseAsFileTime` — an internal contradiction. Seven kernel32 + shell32
  + bcrypt = 9, and `LocalFree` may make it 10 (§4.4).
- 2026-07-20 — **`entry_args_in_registers()` is a `bool` with no third state.** The
  master said C would "extend `entry_args_in_registers`'s Windows case"; there is no
  case to extend a boolean into. The seam's *type* changes (§3.1).
- 2026-07-20 — **The impl shape to mirror is macOS, not `linux_x86_64`.** The master
  cited `linux_x86_64/code.rs:53` and `:509`; that file is 235 lines and contains
  neither — it is a `LinuxArch` delta, and `emit_libc_call` lives at
  `linux_common/code.rs:679`.

## Summary

The engineering risk is concentrated in one place — the entry path, which changes a
shared seam's type and does its UTF-16 conversion before the arena exists — and Phase 1
is a spike against exactly that.

The second risk is quieter: this sub-plan must return fabricated values from 21 required
POSIX constant accessors, because they cannot be left unimplemented and cannot carry an
error. Making them poison rather than plausible is what keeps 47-F/E/G honest until 47-E
removes them.

What is left untouched: `emit_libc_call`'s contract, the PE writer, every other
backend's emitted bytes, and every OS surface beyond the floor — which is rejected at
compile time, not half-implemented.
