# plan-174-D: the Linux riscv64 backend for `mfb dbg`

Last updated: 2026-10-12
Effort: large (estimate: 3h–1d)
Depends on: plan-174-C

Makes `mfb dbg` run on riscv64 Linux. `src/debugger/linux.rs` is the `ptrace`
backend for AArch64 today; this sub-plan separates its processor-specific parts
from the rest and adds the riscv64 ones: reading and writing thread state,
2- and 4-byte breakpoints, stepping without kernel support, and the refusal of
watchpoints. The outcome: on box 2229 (musl) and box 2232 (glibc), a session on
`examples/debug` sets breakpoints, runs, steps, finishes, backtraces, lists
registers and threads, and reads values, each printing what the same session
prints on Linux AArch64 with riscv64 names.

References:

- `src/debugger/linux.rs`: `Tracee`, `step_rounds`, `resume`, `signal_stop`,
  `insert_breakpoint`, `read_code`, `facts`, `pthread_gap`,
  `set_thread_watchpoints`.
- `mfb spec tooling debugger`, "Linux backend".
- plan-174-B §3: `riscv64::decode::Flow`. plan-174-C: `riscv64::RISCV64`.
- `.ai/remote_systems.md`: boxes 2226, 2229, 2232.

## Prerequisites

The prerequisites are stated once in plan-174-A. In addition:

| Must be true | Command | Status |
|---|---|---|
| plan-174-C is complete | `ls planning/plan-174-C-*` → no match | MET (2026-10-21: archived to `planning/completed/` with every box ticked) |
| A statically linked riscv64 `mfb` built on box 2229 runs on box 2232 | on 2229: `RUSTFLAGS='-C target-feature=+crt-static' CARGO_TARGET_DIR=$HOME/p174/static cargo build`; copy `static/debug/mfb` to 2232; `./mfb --version` there → a version line | NOT MET (2026-10-12: not run. A small static Rust program built this way on 2229 does run on 2232; the full `mfb` has not been built static) |

If plan-174-C is not complete, this plan cannot start. The static build is how
the glibc box gets an `mfb`: box 2232 has no `cargo`
(`ssh -p 2232 … 'command -v cargo'` → nothing) and box 2229, which has, is musl.

> **NOTE: the Status column is a snapshot; the Command column is the truth.**
> Re-run every command before you continue. If you stop, report all of them.

## 1. Goal

- `host_backend()` answers a backend on `linux` + `riscv64`.
- On boxes 2229 and 2232, `mfb dbg` on an `--debug -O0` build of
  `examples/debug`: `bp`, `bpi`, `run`, `step`, `step-all`, `finish`, `bt`,
  `dis`, `list func`, `list reg`, `list thread`, `list arena`, `show mem`,
  `show type`, `show arena`, `show error`, `be`, `jump`, `restart`, attach by
  pid, and Ctrl-C all work.
- `wp <where>` answers `this system has no watchpoints: the riscv64 kernel does
  not provide them`.
- Linux AArch64 behaves exactly as before.

### Non-goals (explicit constraints)

- No watchpoint emulation.
- No vector registers.
- No `--app` (GTK) programs: riscv64 app mode does not exist
  (`.ai/remote_systems.md`: "riscv64 app mode is still impossible", bug-117.1).
- No cross-debugging: the debugger debugs programs of its own host, as today
  (`mod.rs:check_debuggable`).
- No riscv64 on macOS or any other OS.
- `tests/cli/cli_dbg.rs` and the riscv64 man topic are plan-174-E. The spec,
  and the man pages' sentence saying where `mfb dbg` runs, are updated here
  with the code.

## 2. Current State

`linux.rs` (1,987 lines) is compiled only under
`cfg(all(target_os = "linux", target_arch = "aarch64"))` (`mod.rs`). Its
processor-specific parts:

- `General`/`Float` from `arm64.rs`; `Tracee::general` reads `NT_PRSTATUS` into
  `[0u64; 34]`, `Tracee::float` reads `NT_FPREGSET` into 528 bytes.
- `general[32]` as `pc` in `signal_stop`, `facts` and `run`; `general[31]` as
  `sp` and `general[19]` as the arena state in `facts`.
- `NT_ARM_TLS` for the thread pointer in `facts`, feeding `pthread_gap`.
- `BREAKPOINT_INSTRUCTION` (`brk #0`, 4 bytes); `breakpoints: BTreeMap<u64,
  [u8; 4]>`; `read_code` overlays originals with a 3-byte look-back.
- `resume(index, step)` uses `PTRACE_SINGLESTEP`; `step_rounds` counts a
  `SIGTRAP` that is not `TRAP_BRKPT` as one step.
- Watchpoints through the `NT_ARM_HW_WATCH` regset: `watch_slots`,
  `set_thread_watchpoints`, `arm_watchpoints`, and `TRAP_HWBKPT` in
  `signal_stop`.
- `library_function_at` gives a library stub `size: Some(16)`.
- `struct LinuxAarch64`, and the comments on `SigAction`, `SYS_GETTID`,
  `SYS_TGKILL`.

Everything else in the file is `ptrace`, `/proc` and signal handling common to
every Linux architecture.

### Measured populations

| What | Count | Command |
|---|---|---|
| Uses of a `General` index in `linux.rs` | 5 | `grep -c 'general\[\|general(tid)?\[' src/debugger/linux.rs` → 5 |
| `NT_ARM_*` mentions in `linux.rs` | 6 | `grep -c 'NT_ARM' src/debugger/linux.rs` → 6 |
| `cfg` gates naming an OS or architecture in the debugger and its CLI | 12 | `grep -rn 'target_arch\|target_os' src/debugger src/cli/dbg.rs src/cli/help.rs \| grep -v 'cfg(test)' \| wc -l` → 12 |
| Instructions stepped per `lr`/`sc` sequence in glibc | `sc.w` 400, `sc.d` 19, `lr.w.aq` 382, `lr.w` 30, `lr.d.aq` 19 | plan-174-B's mnemonic census of `libc.so.6` |

### Verified properties

Each was measured on boxes 2229 (kernel 6.18.38, musl) and 2232 (kernel
6.12.95, glibc 2.41) on 2026-10-12 with `/tmp/rvprobe/probe.c` (a static build
of it ran on 2229) unless another probe is named.

- **The kernel cannot single-step.** `PTRACE_SINGLESTEP -> -1 errno 5
  (Input/output error)` on both.
- **The kernel has no hardware watchpoints.** A breakpoint perf event answers
  `errno 2 (No such file or directory)` on 2229; on 2232
  `grep HW_BREAKPOINT /boot/config-6.12.95+deb13-riscv64` finds no line, and
  neither box has `/sys/bus/event_source/devices/breakpoint`.
- **`ebreak` and `c.ebreak` trap as a breakpoint with `pc` on them.** `stop 0:
  sig 5 si_code 1 … insn-at-pc 0x100073` and `stop 1: sig 5 si_code 1 …
  insn-at-pc 0x…9002`; `si_code` 1 is `TRAP_BRKPT`. Setting `pc` with
  `PTRACE_SETREGSET` works (`SETREGSET pc -> 0`, and the child then exits with
  its own status 7).
- **Both boxes run compressed code** (`zca`, `zcd` in `/proc/cpuinfo`), so a
  2-byte `c.ebreak` is valid wherever a 2-byte instruction is.
- **Regsets.** `NT_PRSTATUS` is 256 bytes and `NT_PRFPREG` 264. The thread
  pointer is the `tp` field of the general state; there is no `NT_ARM_TLS`.
- **System call numbers** are the generic ones `linux.rs` already uses:
  `SYS_gettid=178 SYS_tgkill=131` (`/tmp/rvprobe/p2.c` on 2232).
- **`struct sigaction`** in glibc on riscv64 is handler at 0, a 128-byte mask at
  8, flags at 136 (`p2.c`): the layout of `linux.rs:SigAction`. musl declares the
  same four fields on every architecture.
- **musl's thread handle** is found as on AArch64: `p4.rs` on 2229 prints
  `tp-self=200 first_word_is_self=true` for the main thread and a worker.
- **glibc's thread handle is 32 bytes further from `tp` than on AArch64.**
  `p2.c` on 2232: `tp-self=1856 … _thread_db_sizeof_pthread … value=1824`.
  glibc's riscv64 `TLS_PRE_TCB_SIZE` is `sizeof (struct pthread)` plus the
  16-byte `tcbhead_t` rounded up to the alignment of `struct pthread`, which is
  32. `readelf --dyn-syms libc.so.6 | grep -c _thread_db_sizeof_pthread` → 4,
  so the size symbol is there to read.
- **The saved `pc` of a signal frame is at `sp + 304`** when the handler's
  return reaches `rt_sigreturn` (`/tmp/rvprobe/p3.c`: `uc-info=128
  pcslot-info=304` on both kernels; `__NR_rt_sigreturn` is 139).
- **A riscv64 `mfb` exists.** `cargo build` on 2229 finished in 45 min 36 s and
  `mfb dbg` on it answers `mfb dbg is not supported on linux riscv64`.
- **A static Rust binary built on 2229 runs on 2232** (`rustc -C
  target-feature=+crt-static p4.rs`, copied over, printed its two lines).

## 3. Design Overview

Three files replace one:

- `linux.rs`: everything common. Compiled under `all(linux, any(aarch64,
  riscv64))`. It names the processor only through `cpu::`.
- `linux_arm64.rs`: the AArch64 parts, moved, compiled under linux + aarch64.
- `linux_riscv64.rs`: the riscv64 parts, new, compiled under linux + riscv64.

`linux.rs` selects one with `#[cfg(target_arch = …)] use super::linux_… as
cpu;`. Both modules export the same items:

| Item | AArch64 | riscv64 |
|---|---|---|
| `ARCH: &Arch` | `&arm64::AARCH64` | `&riscv64::RISCV64` |
| `General`, `Float`, `registers`, `assign` | from `arm64.rs` | from `riscv64.rs` |
| `pc`, `set_pc`, `sp`, `arena` (of a `General`) | words 32, 31, 19 | words 0, 2, 27 |
| `thread_pointer(&General, tls)`, `tls` a reader of the TLS regset | word 0 of the `NT_ARM_TLS` regset | word 4 (`tp`); the reader is not called |
| `glibc_gap(size)` | `size` | `size + 32` |
| `breakpoint(original: &[u8]) -> &'static [u8]` | `brk #0`, 4 bytes | `c.ebreak` when the instruction there is 2 bytes, else `ebreak` |
| `HARDWARE_STEP` | `true` | `false` |
| `watch_slots`, `set_watchpoints` | the `NT_ARM_HW_WATCH` regset | none; `insert_watchpoint` is refused |

Breakpoints become variable length: `breakpoints: BTreeMap<u64, Vec<u8>>` holds
the original bytes, `insert_breakpoint` reads `length(bytes)` at the address
first, and `read_code` overlays each original over its own length.

**Stepping without the kernel.** `step_rounds` keeps its structure. Where it
calls `resume(index, true)`, it calls `self.start_step(index)`. On AArch64 that
is `resume(index, true)`. On riscv64 it:

1. reads `pc` and decodes the instruction there (breakpoints are already lifted
   for a step);
2. works out every address the thread can be at after it:

   | `Flow` | Next addresses |
   |---|---|
   | `Next`, or undecoded | `pc + length` |
   | `Branch` | `pc + length` and `target` |
   | `Jump`, direct `Call` | `target` |
   | `Indirect`, indirect `Call`, `Return` | `(register + offset) & !1`, from the thread's state |
   | `Trap` `ecall` with `a7` = 139 | the word at `sp + 304` |
   | `Trap` `ecall` otherwise | `pc + 4` |
   | `Trap` `ebreak`/`c.ebreak` | none: the step reports `a breakpoint instruction` without running, as the AArch64 path does |
   | `AtomicStart` | the address after the first `sc.w`/`sc.d` within 16 instructions, and the target of any branch in between that leaves that range |

3. writes a temporary breakpoint at each (`step_traps`), and continues the
   thread with `PTRACE_CONT`.

A `SIGTRAP` with `TRAP_BRKPT` whose `pc` is one of the thread's temporary
breakpoints is the step completing: `taken += 1`. `step_rounds` removes a
thread's temporary breakpoints as soon as that thread stops for any reason, and
all of them before it returns, on every path.

On riscv64 `step_rounds` steps the threads of a round one after another, not
together. A temporary breakpoint is in memory every thread shares, so two
threads stepping at once could stop on each other's; stepping in turn means the
only running thread is the one whose breakpoints are planted. A thread blocked
in a system call still ends its step as `Waiting` after `STEP_SLICE`, once per
thread.

**Correctness risk** concentrates in the next-address table. A missed
destination lets the thread run away from a `step`, and a breakpoint planted
inside an `lr`/`sc` loop makes that loop spin forever. The table is tested as a
pure function over bytes and register values before any process is involved,
and then by stepping through real C library code on both boxes.

**Byte-identity is this sub-plan's gate only for Phase 1**, the move of AArch64
code between files: `cli_dbg` on box 2226 must pass with no expectation edited.
Phase 2 is gated by unit tests, and Phases 3 and 4 add behaviour on a host that
had none, gated by real sessions.

Two behaviours differ from AArch64 by construction, and Phase 3 records them in
the spec:

- A signal delivered during a `step` runs its handler to completion before the
  step ends, since the thread runs freely to its next address.
- `step-all` advances its threads in turn.

Rejected alternatives:

- **Emulate the instruction in the debugger.** It needs full execution
  semantics for every instruction and has no answer for a system call.
- **Plant breakpoints for all stepping threads at once.** A thread running a
  signal handler or an `lr`/`sc` sequence can reach another thread's temporary
  breakpoint, and stepping it off that needs a second, nested step.
- **Keep one `linux.rs` with `cfg` blocks inline.** The two processors differ in
  about ten places spread through 2,000 lines; one `cpu` module per processor
  keeps each compilable and readable on its own.

## 4. Detailed Design

### 4.1 `linux_riscv64.rs`

- The file names no `Tracee` and makes no system call, so it also compiles
  under `cfg(test)` on the Mac. `thread_pointer` returns `general[4]`. `glibc_gap(size)` returns `size + 32`
  with the measurement of §2 in its doc comment.
- `breakpoint(original)` returns `&[0x02, 0x90]` (`c.ebreak`) when
  `riscv64::decode::length(original) == 2`, else `&[0x73, 0x00, 0x10, 0x00]`
  (`ebreak`).
- `next_addresses(instruction, pc, general, read_u64, read_code) ->
  Result<Vec<u64>, String>` is the table of §3, a pure function apart from its
  two readers.
- `insert_watchpoint` on riscv64 returns `Err("this system has no watchpoints:
  the riscv64 kernel does not provide them")`; `remove_watchpoint` returns the
  shared "there is no watchpoint at 0x…" refusal, and the watch list stays
  empty so `signal_stop`'s `TRAP_HWBKPT` arm is never taken.

### 4.2 `linux.rs`

- `Tracee::general`/`float` read into `cpu::General`/`cpu::Float`.
- `facts` uses `cpu::pc`, `cpu::sp`, `cpu::arena`, `cpu::thread_pointer`;
  `pthread_gap`'s glibc branch returns `cpu::glibc_gap(size)`.
- `signal_stop` and `run` use `cpu::pc`.
- `insert_breakpoint`, `remove_breakpoint`, `lift`, `reinsert`, `read_code` use
  variable-length originals and `cpu::breakpoint`.
- `step_rounds` calls `start_step`, recognises a temporary-breakpoint stop when
  `!cpu::HARDWARE_STEP`, steps threads in turn when `!cpu::HARDWARE_STEP`, and
  clears `step_traps` on every exit.
- `struct LinuxAarch64` becomes `struct Linux`; `Inferior::arch` answers
  `cpu::ARCH`.

### 4.3 Host support

- `mod.rs`: `mod linux` and `mod elf` under `all(linux, any(aarch64,
  riscv64))`; `host_backend` answers `linux::Linux` there; its doc and
  `tests.rs:the_backend_exists_exactly_where_mfb_dbg_is_supported` name the
  three hosts.
- `src/cli/dbg.rs`: the refusal reads `mfb dbg is not supported on {} {}; it
  runs on macOS, and on Linux on AArch64 and riscv64`.
- `src/cli/help.rs`: the Linux sentence names both architectures.
- `arch::host()` answers `&riscv64::RISCV64` on riscv64, so `mfb dbg --help`
  there shows riscv64 register names.

## Compatibility / Format Impact

- New: `mfb dbg` works on riscv64 Linux.
- Changed text: the not-supported message and one sentence of `mfb dbg --help`
  name riscv64.
- Unchanged: every command's behaviour and output on macOS and Linux AArch64.

## Phases

> **NOTE: keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work. Fill the phase's `Commit:` line when it lands. An unticked
> box means NOT DONE.

### Phase 1 — split `linux.rs` (AArch64 only, neutral)

Moves code without changing it. Safe alone: riscv64 is not mentioned yet.

- [ ] Add `src/debugger/linux_arm64.rs` with the items of §3's table, moved from
      `linux.rs`; `linux.rs` reads them through `cpu::`.
- [ ] `linux.rs`: `breakpoints` to `BTreeMap<u64, Vec<u8>>`; `read_code`
      overlays by each original's length.
- [ ] `linux.rs`: `start_step` wrapping `resume(index, true)`; `struct
      LinuxAarch64` renamed `Linux`.

Acceptance: Linux AArch64 is unchanged.
  Check: on box 2226, plan-174-A's Prerequisites command (the release build,
  then `cli_dbg` with `MFB_TEST_EXE`) → `16 passed; 0 failed`, with `git diff --
  tests/` empty (est. 10 min; box 2226 is the only host where `linux.rs`
  compiles for AArch64).
Commit: —

### Phase 2 — the next-address function

A pure function with no caller. Safe alone, and tested on the Mac.

- [ ] Add `src/debugger/linux_riscv64.rs`, declared in `mod.rs` under
      `any(all(target_os = "linux", target_arch = "riscv64"), test)`, with
      `next_addresses` per §3.
- [ ] Tests in `linux_riscv64.rs`: `next_addresses` for a straight instruction
      of each length, `beq` taken and not, `jal`, `jalr` through a register,
      `ret`, `c.jr`, `c.beqz`, `ecall`, `ecall` with `a7` = 139 reading
      `sp + 304`, an `lr.w` … `sc.w` loop with a branch out, an `lr.w` with no
      `sc` in 16 instructions, and an undecoded word.

Acceptance: every row of §3's table answers the addresses the table gives.
  Check: `cargo test --bin mfb debugger::linux_riscv64` → 0 failed (est. 2 min).
Commit: —

### Phase 3 — the riscv64 backend (largest blast radius)

State, breakpoints, stepping and host support land together: a backend that
stops at a breakpoint cannot run on from it without stepping, so no smaller
piece is usable.

- [ ] `linux_riscv64.rs`: `ARCH`, the state accessors, `thread_pointer`,
      `glibc_gap`, `breakpoint`, the watchpoint refusal, `HARDWARE_STEP = false`.
- [ ] `linux.rs`: `start_step` on riscv64 plants `step_traps` from
      `next_addresses` and continues; `step_rounds` recognises their stops,
      steps in turn, and clears them on every exit.
- [ ] `mod.rs`, `src/cli/dbg.rs`, `src/cli/help.rs`, `tests.rs` per §4.3.
- [ ] `src/docs/spec/tooling/10_debugger.md`, in this same change: "Host
      support" (the three hosts and the new message); a "Linux backend"
      subsection for riscv64 (regsets and their sizes, `tp`, `ebreak` and
      `c.ebreak`, the next-address table, stepping in turn, no watchpoints, the
      glibc thread handle); `list reg` (the riscv64 groups and role table);
      `step` (a signal's handler runs to completion within a step); `jump`
      (settable registers); `finish` (`a0`, `a1`); `disassemble` (text as GNU
      `objdump` spells it, `.short`); `backtrace` (the riscv64 effects, no
      address mask); Watchpoints (the refusal). Each claim cited to its symbol
      in `linux_riscv64.rs`, `riscv64.rs` or `linux.rs`.
- [ ] `src/docs/man/debug/01_start.md` and `package.md`: the sentence saying
      where `mfb dbg` runs becomes "macOS, and Linux on Arm (AArch64) or RISC-V
      (riscv64) computers", with x86-64 Linux and Windows still named as not
      supported. Check: `mfb man debug start | grep -c riscv64` → 1.
- [ ] Build on box 2229 (`cargo build`: 46 min from clean, minutes after).

Acceptance: breakpoints, registers, backtraces, `step`, `step-all`, `finish`
and running on from a breakpoint work through compiled and C library code on
box 2229, and `wp` is refused.
  Check: on 2229, `printf 'bp isPrime\nrun\nlist reg\nbt\nlist thread\nwp sp
  8\nstep 40\nfinish\nrun\nbp r isPrime\nbp _exit\nrun\nbt\nstep 200\nq\n' |
  mfb dbg debug-musl.out -- 2 500` → the stop line `--[BP] T: 0 P: 0x… F:
  (isPrime) _mfb_fn_isPrime`; the role table with `a0` = 401575 and `a1` = 500;
  a backtrace ending at `_main`; `MFB Main`; the watchpoint refusal; `Stepped 40
  instructions`; `Thread 0 returned from isPrime … a0 = … (0), a1 = …`; a second
  breakpoint stop (which steps the thread off the first); a stop at `_exit` in
  the C library; and 200 steps through C library code ending either `Stepped 200
  instructions` or the program's exit (est. 5 min after the build).
Commit: —

### Phase 4 — threads, glibc, attach

- [ ] Build the static `mfb` on 2229 per the Prerequisites command, copy it to
      2232, and run Phase 3's session there on `debug-glibc.out`.
- [ ] On both boxes run the threaded example (`-- 3 2000 1`): `bp roundWorker`,
      `run`, `list thread`, `list arena`, `bt all`, `step-all 5`.
- [ ] On both boxes attach: start `debug-*.out 400 20000` in the background,
      `mfb dbg <pid>`, `list thread`, `bt`, `q`, and see the program run on.

Acceptance: a worker thread is recognised on both C libraries, which proves the
thread handle (`tp - 200` found by search on musl, `tp - (1824 + 32)` on
glibc), and attach works.
  Check: `list thread` shows the worker's task as `User Thread` on 2229 and on
  2232; `list arena` shows one arena per thread; after `q` from the attached
  session `kill -0 <pid>` succeeds (est. 10 min for both boxes).
Commit: —

## Validation Plan

- Tests: `linux_riscv64::tests` for `next_addresses`; the AArch64 `cli_dbg` run
  on 2226 for the neutral split. The riscv64 `cli_dbg` run is plan-174-E.
- Coverage check: `linux.rs` compiles only on a Linux host, so the Mac's
  `cargo test` does not build it; the builds on 2226 and 2229 are what compile
  it. `next_addresses` is deliberately a pure function in a file the Mac's test
  build includes, so its tests run in the Mac's denominator.
- Runtime proof: the sessions of Phases 3 and 4 on boxes 2229 and 2232.
- Doc sync: `mfb spec tooling debugger` is updated in Phase 3, in the same
  change as the code that alters the contract. Check: `cargo test --bin mfb
  spec` → 0 failed, and `mfb spec tooling debugger | grep -c riscv64` is above
  0. The man topic is plan-174-E.
- Final gate: plan-174-E, once for all of plan-174.

## Decisions

- `wp` is refused on riscv64 with one clear error — decided by the owner,
  2026-10-12. Neither kernel has hardware watchpoints, and emulating them by
  stepping was declined. (§4.1)
- Registers are shown and accepted by ABI name only — decided by the owner,
  2026-10-12. The sessions above use `a0`, `ra`, `sp`. (plan-174-C)

## Corrections

## Summary

The risk is the next-address computation: indirect jumps, atomic sequences and
the signal return. It is isolated in one pure function with unit tests, then
exercised through C library code on two kernels and two C libraries. The
AArch64 Linux backend is moved, not changed, and proven so on box 2226.
