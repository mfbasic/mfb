# plan-174-A: `mfb dbg` architecture seam

Last updated: 2026-10-12
Overall Effort: huge (estimate: 4d–6d across A, B, C, D, E)
Effort: large (estimate: 3h–1d)
Depends on: nothing

plan-174 adds a Linux riscv64 backend to `mfb dbg`. Today the debugger runs on
macOS and on Linux on AArch64, and its shared code names AArch64 registers,
4-byte instructions and A64 bit patterns directly. This sub-plan moves every one
of those assumptions behind one description of the architecture, with AArch64 as
its only value. Nothing a user or a test can observe changes: every existing
debugger test passes with its expectations untouched. Sub-plans B to E then add
riscv64 as a second value.

The single outcome of plan-174: on a riscv64 Linux host, `mfb dbg` launches or
attaches to an MFBASIC `--debug` program and every command but `wp` works as it
does on AArch64 Linux, proven by `tests/cli/cli_dbg.rs` passing on box 2229.

The sub-plans, in implementation order:

| Letter | Delivers | Effort |
|---|---|---|
| A | the architecture seam in shared debugger code (this file) | large |
| B | a riscv64 instruction decoder, `src/arch/riscv64/decode` | large |
| C | the riscv64 register file, unwinder, ELF reading and descriptor, tested on any host | large |
| D | the Linux riscv64 backend: registers, breakpoints, software stepping | large |
| E | tests on riscv64, spec, man topic, final gate | large |

References:

- `mfb spec tooling debugger` (`src/docs/spec/tooling/10_debugger.md`): the
  contract of every command.
- `mfb spec memory native-calling-convention`: argument, result and pinned
  registers per architecture; the frame layout.
- `src/debugger/mod.rs:Inferior`, `Backend`, `host_backend`.
- `src/debugger/arm64.rs`: the one architecture module today.
- `.ai/remote_systems.md`: boxes 2226 (linux-aarch64), 2229 and 2232 (riscv64).

## Prerequisites

These hold for all of plan-174. Sub-plans B to E point here.

| Must be true | Command | Status |
|---|---|---|
| The debugger unit tests pass on main | `cargo test --bin mfb debugger::` → `180 passed; 0 failed` | MET (2026-10-20, macOS, at `52e4535b3`: `180 passed; 0 failed`) |
| The end-to-end debugger tests pass on macOS | `cargo test --test cli_dbg` → `21 passed; 0 failed` | MET (2026-10-20, at `52e4535b3` in a detached worktree: `21 passed; 0 failed`, 517 s) |
| The end-to-end debugger tests pass on Linux AArch64 | on box 2226: `. ~/mfbdbg-env.sh; cargo build --release --bin mfb; MFB_TEST_EXE=$PWD/target/release/mfb cargo test --test cli_dbg` → `19 passed; 0 failed` | MET (2026-10-20, at `52e4535b3` in a fresh `~/dbgrv` tree: release build 7 min 30 s, then `19 passed; 0 failed`, 4 s) |
| bug-1004 is fixed: `an_attached_threaded_program_lists_its_threads_and_arenas` no longer flakes | `ls bugs/bug-1004-*` → no match (moved to `bugs/completed/`) | MET (2026-10-20: `ls bugs/bug-1004-*` → no match; `ls bugs/completed/ \| grep 1004` → `bug-1004-cli-dbg-attached-threaded-arena-test-flakes.md`) |
| `mfb` builds and runs on riscv64 | on box 2229: `cargo build` in a `git archive HEAD` tree → `Finished`, and `target/debug/mfb --version` prints a version | MET (2026-10-12: `Finished` in 45 min 36 s; `MFBasic Compiler 0.1.0`) |
| A riscv64 executable the compiler builds runs on both boxes | `mfb build --debug -O0 --target linux-riscv64 examples/debug`; run `debug-musl.out 2 500` on 2229 and `debug-glibc.out 2 500` on 2232 → `total: 68 primes` | MET (2026-10-20, built by the release `mfb` of `52e4535b3`: both print `total: 68 primes`) |

Everything below is written against the world where these hold.

Two facts about box 2226 shape every check that runs there. It has 3.9 GB of
memory, and `rustc` is killed (`signal: 9, SIGKILL`) compiling the `mfb` unit-test
binary, so `cargo test --bin mfb` cannot run on it; the unit tests are
host-independent and run on the Mac. The same kill hits the release build that
`tests/common/mod.rs:mfb_exe` starts from inside a test run, so the release
compiler is built first, by hand, and handed over in `MFB_TEST_EXE`. Both were
observed on 2026-10-12.

bug-1004 is a prerequisite because every gate of plan-174 that runs `cli_dbg`
requires 0 failed, and that test fails some runs for a reason plan-174 does not
touch. If bug-1004 is not fixed, plan-174 cannot start.

> **NOTE: the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again
> before you decide to stop. If you stop, report the status of all of them.

## 1. Goal

- Shared debugger code (`backtrace.rs`, `breakpoints.rs`, `disassemble.rs`,
  `location.rs`, `prompt.rs`, `elf.rs`) names no AArch64 register, instruction
  size, opcode or relocation. It reads them from `arch::Arch`, which the
  `Inferior` answers.
- `arch::AARCH64` is the only descriptor. `arm64.rs` holds everything AArch64.
- The 155 unit tests and the 19 tests of `tests/cli/cli_dbg.rs` pass with no
  expectation edited.

### Non-goals (explicit constraints)

- No output of any `mfb dbg` command changes on macOS or Linux AArch64: prompts, listings,
  error messages and help text are byte-for-byte what they are at `c0a48dd4f`.
- No riscv64 code. No `cfg(target_arch = "riscv64")` anywhere in this sub-plan.
- `macos.rs` stays AArch64-only inside; it only gains `fn arch`.
- `linux.rs` changes only where the seam's signatures force it. Its split into
  shared and per-architecture parts is plan-174-D.
- The `Inferior` trait keeps every existing method and meaning.

## 2. Current State

The debugger has one architecture module, `src/debugger/arm64.rs` (243 lines):
`General` (`[u64; 34]`), `Float`, `registers`, `role`, `slot`, `assign`. The
macOS and Linux backends call it for register reads and writes
(`linux.rs:Tracee::registers`, `set_registers`).

AArch64 assumptions outside that module, by file:

- `location.rs:is_register`: `fp`, `lr`, `sp`, `pc`, `x0`–`x30`.
- `prompt.rs:settable`, `NZCV_FLAGS`, `jump`: which registers `jump` sets, the
  NZCV bit mask, the 32-bit width of `fpsr`/`fpcr`, and "they start every 4
  bytes" for a jump target.
- `prompt.rs:finish_lines`: prints `x0 = …, x1 = …`.
- `prompt.rs:HELP`, `LIST_REG_USAGE`: register names in help text.
  `src/cli/help.rs` embeds `HELP` in `mfb dbg --help`.
- `breakpoints.rs:Breakpoints::watched_raise`: reads the error code from `x1`,
  the origin from `x3`, the return address from `lr`.
- `breakpoints.rs:watchable`, `WATCH_SPAN`: 1 to 8 bytes not crossing a multiple
  of 8, the AArch64 byte-address-select shape.
- `disassemble.rs`: `use crate::arch::aarch64::decode::{decode, Flow}`,
  `INSTRUCTION = 4`, `chunks_exact(4)`, the `adrp`+`add` page tracking in
  `listing`, `.word` for an undecoded word.
- `backtrace.rs`: `Effect::of` decodes A64 bit patterns by hand; `state_at`
  uses register numbers 29, 30, 31; `frames` reads `pc`, `sp`, `fp`, `lr` and
  looks a caller up at its return address less 4; `ADDRESS_MASK` clears bits 63–48
  (63–47 on macOS).
- `elf.rs`: `EM_AARCH64`, `R_AARCH64_GLOB_DAT`, `R_AARCH64_JUMP_SLOT`, and
  `stubs`, which matches the executable's `adrp x16` / `ldr x17` / `br x17`
  import stub.

The decoder's contract with the debugger is small: `decode(word, pc)` answers an
`Instruction` whose `text`, `flow == Flow::Return`, `target`, `value`, `page`
and `page_offset` are the only fields `disassemble.rs` reads.

### Measured populations

| What | Count | Command |
|---|---|---|
| Register-name string literals in shared files | 60 lines (backtrace 4, breakpoints 11, location 12, prompt 33) | `grep -nE '"(x[0-9]+\|w[0-9]+\|fp\|lr\|sp\|pc\|nzcv\|fpsr\|fpcr\|v[0-9]+)"' src/debugger/*.rs \| grep -v arm64.rs \| wc -l` → 60 |
| Fixed 4-byte instruction sites outside the macOS files | 31 lines (disassemble 12, linux 6, elf 5, backtrace 4, breakpoints 2, prompt 2) | `grep -nE 'INSTRUCTION\|% ?4\b\|chunks_exact\(4\)\|\[u8; 4\]\|0xd420\|saturating_sub\((3\|4)\)\|\* 4\b\|/ 4\b' src/debugger/*.rs \| grep -v 'macos.rs\|macho.rs' \| wc -l` → 31 |
| Uses of an `arch::aarch64` module from the debugger | 3 (disassemble 2, arm64 test 1) | `grep -rn 'arch::aarch64' src/debugger \| wc -l` → 3 |
| `impl Inferior for` | 5 (macos `Task`, linux `Tracee`, test fakes `Recorder`, `Program`, `Memory`) | `grep -rn 'impl Inferior for' src/debugger` → 5 |
| Unit tests in `src/debugger` | 155 | `grep -c '#\[test\]' src/debugger/*.rs` summed → 155 |
| Tests in `tests/cli/cli_dbg.rs` | 19 (18 run on macOS, 1 is the not-supported test) | `grep -c '#\[test\]' tests/cli/cli_dbg.rs` → 19 |

### Verified properties

- **The decoder contract is the six fields above.** Read `disassemble.rs`:
  `returns`, `listing`, `comment`, `named_address`. No other field of
  `aarch64::decode::Instruction` is read from `src/debugger`.
- **`backtrace.rs` does not use the decoder.** It has its own A64 patterns in
  `Effect::of` (read the function). It unwinds by replaying a function's code
  from its start, not by a frame-pointer chain.
- **`runtime.rs` and `values.rs` name no register.** `grep -nE 'x19|x20'
  src/debugger/runtime.rs src/debugger/values.rs` → only a test fake. They read
  `ThreadFacts.arena`, which each backend fills.
- **`check_debuggable` is already neutral.** `mod.rs:host_image` takes the
  architecture from `std::env::consts::ARCH`, and `src/os/inspect.rs` maps ELF
  machine 243 to `riscv64`.

## 3. Design Overview

One new file, `src/debugger/arch.rs`, always compiled, holds a plain-data
description of an architecture and the neutral types the shared code needs:

```rust
pub(crate) struct Arch {
    pub(crate) name: &'static str,            // "aarch64"
    pub(crate) pc: &'static str,              // "pc"
    pub(crate) sp: &'static str,              // "sp"
    pub(crate) link: &'static str,            // "lr"
    pub(crate) frame: &'static str,           // "fp"
    pub(crate) results: [&'static str; 2],    // finish prints these: ["x0", "x1"]
    pub(crate) park_code: &'static str,       // "x1": the code at _mfb_rt_park_error
    pub(crate) park_origin: &'static str,     // "x3"
    pub(crate) is_register: fn(&str) -> bool,
    pub(crate) settable: fn(&str, u64) -> Result<(), String>, // jump's rules
    pub(crate) alignment: u64,                // 4: where an instruction may start
    pub(crate) decode: fn(&[u8], u64) -> Option<Decoded>,
    pub(crate) length: fn(&[u8]) -> usize,    // of the instruction at these bytes
    pub(crate) effect: fn(&[u8]) -> Effect,   // the unwinder's reading of one
    pub(crate) address_mask: u64,
    pub(crate) watch_span: Option<u64>,       // Some(8); None = no watchpoints
    pub(crate) help: ArchHelp,                // the register lines of HELP and LIST_REG_USAGE
    pub(crate) elf: ElfArch,                  // machine, relocation types, stub reader
}
```

`Decoded` is the neutral instruction: `length`, `text`, `returns: bool`,
`target`, `value`, `page`, `page_offset`. `Effect` is the unwinder's neutral
vocabulary (move `sp`, save or restore the link or frame register at an offset,
set the frame register from `sp`, set `sp` from the frame register, return, jump
away, hint, other write to `sp`, none). `arm64.rs` gains the AArch64
implementations and the `pub(crate) const AARCH64: Arch`.

`Inferior` gains `fn arch(&self) -> &'static Arch`. All five implementations
answer `&arch::AARCH64`. Shared code asks the inferior; nothing is selected by
`cfg`. That is what lets plan-174-C test the riscv64 descriptor on a Mac by
giving a test fake a different `arch`.

Help text is built, not constant: `prompt::help(arch)` and
`prompt::list_reg_usage(arch)` assemble today's exact text from `arch.help`.
`src/cli/help.rs` calls `prompt::help(arch::host())`, where `arch::host()` is
the build host's descriptor and `AARCH64` on a host with no backend, so
`mfb dbg --help` prints what it prints today everywhere.

**Correctness risk** concentrates in the unwinder move (Phase 4): `Effect::of`
is hand-written bit matching with seven unit tests, and a slip there gives
wrong backtraces without failing to compile. It lands last, and its tests keep
their 16 named A64 words unchanged.

**This sub-plan is provably neutral, and byte-identity of output is its gate.**
The check is that no test expectation is edited: `git diff --stat` for the
sub-plan shows test hunks only where a fake gains `fn arch` or a call gains an
argument, and both suites pass on macOS and on box 2226. A test that fails means
a bug in the move: diagnose that one test and fix the code.

Rejected alternatives:

- **Select the architecture with `cfg(target_arch)` type aliases.** It would
  compile only one architecture per host, so the riscv64 code could not be
  unit-tested on the Mac, where all development happens.
- **A trait object per architecture.** The descriptor has no state; a struct of
  constants and function pointers says the same with less machinery, and can be
  a `const`.
- **Leave `backtrace.rs` AArch64-only and write a second unwinder.** The replay
  loop, the CFA arithmetic and the frame walk are architecture-neutral (read
  `state_at`, `caller`, `frames`); only the instruction reading differs.

## 4. Detailed Design

### 4.1 Registers and text

- `location::is_register(word)` becomes `(arch.is_register)(word)`; its callers
  (`Location::parse`, `location::name_value`, `prompt::settable`) receive `arch`.
  `Location::parse` is called from `prompt::parse`, which has no inferior, so
  `parse` takes `arch: &Arch` and `prompt::run` passes `inferior.arch()`.
- `prompt::settable`, `NZCV_FLAGS` and the width checks in `jump` move to
  `arm64::settable(name, value)`, which returns the same error strings.
- `finish_lines` prints `arch.results[0]` and `arch.results[1]`.
- `watched_raise` reads `arch.park_code`, `arch.park_origin`, `arch.link`.
- `breakpoints::watchable` takes `arch.watch_span`; with `Some(8)` it is today's
  rule and text.

### 4.2 Instructions

- `disassemble.rs` drops `INSTRUCTION` and the `aarch64::decode` import. It
  walks code with `(arch.length)(bytes)` and `(arch.decode)(bytes, pc)`; an
  undecoded instruction prints `.word  0x…` (4 bytes) as today. `arch.alignment`
  replaces `% INSTRUCTION` and the step-back in `counted`.
- `arm64::decode(bytes, pc)` wraps `crate::arch::aarch64::decode::decode` and
  maps `Flow::Return` to `returns`. `arm64::length` answers 4.
- `prompt::jump` uses `arch.alignment` in its check and message.
- `backtrace::frames` looks a caller up at its return address less
  `arch.alignment`. For AArch64 that is 4, as today.

### 4.3 Unwinder

- `backtrace::Effect` moves to `arch.rs` with register roles in place of
  numbers. `state_at` tracks `sp`, the link register and the frame register by
  role. `Effect::of`, `pair_effect`, `sign_extend` and the A64 constants move to
  `arm64.rs` as `arm64::effect`.
- `ADDRESS_MASK` becomes `arch.address_mask`; `AARCH64` computes it with the
  `cfg!(target_os = "linux")` that `backtrace.rs` uses today.

### 4.4 ELF

- `ElfArch { machine: u16, jump_slot: u32, data_slot: u32, stub_size: u64,
  stub_slot: fn(&[u8], u64) -> Option<u64> }`. `elf::read` and `read_dynamic`
  take `&ElfArch`; `stubs` calls `stub_slot` on each 12-byte window.
  `arm64::ELF` carries 183, 1026, 1025, 12 and today's `adrp`/`ldr`/`br` match.
- `PLT_HEADER` (32) and `PLT_ENTRY` (16) stay constants of `elf.rs`: the riscv64
  glibc `.plt` has the same sizes (see plan-174-C).

## Compatibility / Format Impact

None for `mfb dbg`: no command's output, no message and no help text changes.
`mfb spec tooling debugger` gains one sentence about `Inferior::arch`.

## Phases

> **NOTE: keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work. Use `- [~]` for partially done and say what remains. Fill
> the phase's `Commit:` line when it lands. An unticked box means NOT DONE.

### Phase 1 — the descriptor and `Inferior::arch`

Adds the type and its AArch64 value with no reader. Safe alone: nothing calls it.

- [x] Add `src/debugger/arch.rs` with `Arch`, `Decoded`, `Effect`, `ArchHelp`,
      `ElfArch` and `pub(crate) fn host() -> &'static Arch`; declare `mod arch`
      in `mod.rs` with no `cfg`. (Also `Reg`, the unwinder's register roles.
      `mod arm64` lost its `cfg` too, since `host()` names `AARCH64` on every
      host; its register file moved to `arm64/registers.rs` under the old
      `cfg`, see Corrections.)
- [x] In `arm64.rs`, add `pub(crate) const AARCH64: Arch` with the names of
      §3 and placeholder-free functions: `is_register` (moved from
      `location.rs`), `settable`, `decode`, `length`, `effect` (Phase 4 fills
      `effect` by moving code; until then it wraps `backtrace::Effect::of`).
      (In `arm64/mod.rs`. `effect` is the moved code from the start, not a
      wrapper: see Corrections. `holds` carries `jump`'s value checks.)
- [x] Add `fn arch(&self) -> &'static Arch` to `Inferior` in `mod.rs` and to the
      five implementations, each answering `&arm64::AARCH64`. (Seven: see
      Corrections. `src/cli/dbg/tests.rs:Paused`, outside `debugger`, answers
      `arch::host()`.)
- [x] `src/docs/spec/tooling/10_debugger.md`, "Host support": add `arch` to the
      list of `Inferior` methods a platform implements, with one sentence saying
      it names the registers, instruction size, decoder and unwinder rules the
      neutral code uses, cited `[[src/debugger/arch.rs:Arch]]`. (The four
      `[[src/debugger/arm64.rs:…]]` citations now name `arm64/registers.rs`;
      `cargo test --bin mfb spec` → `67 passed; 0 failed`.)
- [x] Tests: `arm64::tests::the_descriptor_names_the_registers_the_backends_read`
      asserts every name in `AARCH64` is a name `arm64::registers` produces.
      `cargo test --bin mfb debugger::arm64` → `4 passed; 0 failed`.

Acceptance: the descriptor exists and agrees with the register file.
  Check: `cargo test --bin mfb debugger::arm64` → 4 passed (3 existing + 1 new)
  (est. 2 min).
Commit: `9e143baae`

### Phase 2 — registers and help text

- [x] `location.rs`: `is_register` delegates to `arch`; `Location::parse` and
      `name_value` take `arch`. (`is_register` is gone from `location.rs`;
      `name_value` asks `inferior.arch()`. `Location::parse` never called it:
      see Corrections.)
- [x] `prompt.rs`: `parse` takes `arch`; `settable`, `NZCV_FLAGS` and `jump`'s
      width checks call `(arch.settable)`; `finish_lines` uses `arch.results`.
      (`parse`, `stop_commands`, `runs_on` and `Disassembly::from` take `arch`;
      the name check is `arch.settable`, the value check `arch.holds`;
      `finish_lines` prints `arch.arguments[0..2]`; the `x0=5` in `jump`'s
      usage error is `arch.arguments[0]`.)
- [x] `prompt.rs`: replace the `HELP` and `LIST_REG_USAGE` constants with
      `help(arch)` and `list_reg_usage(arch)`; `src/cli/help.rs` calls
      `prompt::help(arch::host())`. (The constants stay, private, as templates
      with `{arg0}`, `{jump}`, `{watch}`, `{groups}`, `{arg1}`, `{sp}`.)
- [x] `breakpoints.rs`: `watched_raise` and `watchable` read `arch`. (Also
      `breakpoints/report.rs:open_report` and `take_line`. `watchable` with
      `watch_span: None` refuses with `this system has no watchpoints: the
      <arch> kernel does not provide them`, plan-174-D §1's text.
      `WATCH_SPAN`'s other use, `value_shown`'s "a whole word", is now
      `breakpoints::WORD`.)
- [x] Tests: add `prompt::tests::help_reads_as_before` comparing `help(&AARCH64)`
      and `list_reg_usage(&AARCH64)` with the two literal texts copied from
      `c0a48dd4f`. (Copied from `52e4535b3`, the plan's base.)

Acceptance: no test expectation changed and the help text is identical.
  Check: `cargo test --bin mfb debugger::` → 157 passed; 0 failed, and
  `git diff c0a48dd4f -- src/debugger | grep '^[-+].*assert'` shows no changed
  assertion outside the two new tests (est. 3 min).
  Measured 2026-10-20: `cargo test --bin mfb debugger::` → `182 passed; 0
  failed` (the corrected count); `git diff 52e4535b3 -- src/debugger src/cli |
  grep -E '^[-+].*assert'` → every `-`/`+` pair differs only by the added
  `&AARCH64` argument, plus the two new tests' assertions; `cargo test --bin
  mfb cli::dbg` → `13 passed; 0 failed` (`mfb dbg --help` carries the help).
Commit: —

### Phase 3 — instruction size and the decoder call

- [ ] `disassemble.rs`: remove `INSTRUCTION` and the `aarch64::decode` import;
      `returns`, `listing`, `read_up_to`, `counted` and `disassemble` use
      `arch.length`, `arch.decode`, `arch.alignment`.
- [ ] `prompt.rs:jump`: alignment check and message from `arch.alignment`.
- [ ] `backtrace.rs:frames`: return address less `arch.alignment`.
- [ ] `elf.rs`: `read`, `read_dynamic`, `stubs` take `&ElfArch`; `linux.rs`
      passes `&arm64::ELF` (built on box 2226, since `linux.rs` compiles only
      there).

Acceptance: listings, `finish` and library lookups are unchanged on both hosts.
  Check: `cargo test --bin mfb debugger::` → 157 passed (est. 3 min), and on box
  2226 the Prerequisites command → `16 passed; 0 failed` (est. 10 min; 2226
  because `linux.rs` and `elf.rs`'s live path build nowhere else).
Commit: —

### Phase 4 — the unwinder (largest blast radius last)

- [ ] Move `Effect` to `arch.rs` with register roles; move `Effect::of`,
      `pair_effect`, `sign_extend` and the A64 constants to `arm64.rs` as
      `effect`; `backtrace::state_at`, `saved`, `moved`, `base_offset` track
      roles.
- [ ] `backtrace.rs`: `ADDRESS_MASK` from `arch.address_mask`; `frames` reads
      the registers named `arch.pc`, `arch.sp`, `arch.frame`, `arch.link`.
- [ ] Tests: the seven `backtrace::tests` keep their 16 named A64 words and
      expectations; only the call that turns a word into an `Effect` changes.

Acceptance: every backtrace test passes unedited, and real backtraces are the
same on both hosts.
  Check: `cargo test --bin mfb debugger::backtrace` → 7 passed (est. 2 min);
  `cargo test --test cli_dbg` on macOS → 18 passed (est. 3 min) and the
  Prerequisites command on box 2226 → `16 passed; 0 failed` (est. 10 min).
Commit: —

## Validation Plan

- Tests: the existing 155 unit tests and 19 `cli_dbg` tests, unedited, plus the
  two new ones of Phases 1 and 2.
- Coverage check: `grep -rn 'arch::aarch64\|"x[0-9]\+"\|\bINSTRUCTION\b'
  src/debugger --include='*.rs' | grep -v 'arm64.rs\|macos.rs\|#\[cfg(test)\]'`
  lists only test code, so the shared files no longer name the architecture.
- Runtime proof: Phase 4's `cli_dbg` runs on macOS and box 2226 drive real
  sessions (`bt`, `dis`, `finish`, `jump`, `be`).
- Doc sync: `mfb spec tooling debugger`, "Host support", gains `arch` in its
  list of `Inferior` methods (Phase 1). No command's contract changes. Check:
  `mfb spec tooling debugger | grep -c 'arch'` rises by the new sentence and
  `mfb spec tooling debugger | grep -c '\[\['` stays at 1 (the one literal
  `[[sp+0x10]+8]` the page shows).
- Final gate: plan-174-E, once for all of plan-174.

## Decisions

- `wp` is refused on riscv64 — decided by the owner, 2026-10-12. It sets
  `watch_span: Option<u64>` in the descriptor: `None` means no watchpoints.
  (§3; implemented in plan-174-D)
- Registers are shown and accepted by ABI name only on riscv64 — decided by the
  owner, 2026-10-12. It is why `is_register` is a function of the descriptor
  with no alias table. (§4.1; implemented in plan-174-C)
- riscv64 is documented in one new man topic — decided by the owner,
  2026-10-12. Existing man pages stay AArch64. (plan-174-E)

## Corrections

- **The tree was split after this plan was written (plan-176).** `prompt.rs`
  is `prompt/{mod,command,actions,stops,render}.rs`, `breakpoints.rs` is
  `breakpoints/{mod,errors,report,running}.rs`, `linux.rs` is
  `linux/{mod,inferior,images,process,threads}.rs`, and `macos.rs` is
  `macos/`. Tasks naming the old files apply to their parts. Measured
  2026-10-20 with `find src/debugger -name '*.rs'`.
- **Counts.** Unit tests in `src/debugger`: 180, not 155
  (`grep -rc '#\[test\]' src/debugger | awk -F: '{s+=$2} END {print s}'` → 180,
  and the Prerequisites run → `180 passed`). `cli_dbg` tests: 22 across
  `tests/cli/cli_dbg.rs` and `tests/cli/cli_dbg/` (`grep -rc '#\[test\]'` → 22),
  21 of them run on macOS and 19 on box 2226 (the Prerequisites runs).
  Phase 2's and 3's "157 passed" become 182 (180 + the two new tests).
- **`impl Inferior for`: 7, not 5** (`grep -rn 'impl Inferior for' src` → `Task`,
  `Tracee`, and the test fakes `Recorder`, `Memory`, `Program`, `Exiting`
  (`breakpoints/report/tests`) and `Paused` (`src/cli/dbg/tests.rs`)).
- **More register names than §2 lists.** `breakpoints/report.rs:take_line` and
  `open_report` read `x0`, `x1`, `x2`, `lr` and write `x0`, `pc` (the `write`
  call the debug report is taken from); they read `arch.arguments` and
  `arch.link` like `watched_raise`. So the descriptor carries `arguments: [&str;
  8]` (the registers a call's arguments and results are in) in place of §3's
  `results`, `park_code` and `park_origin`: `finish` prints `arguments[0..2]`,
  and the park's code and origin are its arguments 1 and 3.
- **`Location::parse` does not call `is_register`** (read it); only
  `location::name_value` and `prompt/command.rs:Disassembly::from` do. So
  `Location::parse` keeps its signature and `prompt::parse` (whose
  `Disassembly::from` decides between a location and a function name) takes
  `arch`.
- **`settable` is two checks at two times.** The name is checked when the line
  is parsed (`prompt/command.rs:settable`), the value when `jump` runs
  (`prompt/actions.rs:jump`, against `NZCV_FLAGS` and 32 bits). So the
  descriptor has `settable: fn(&str) -> Result<(), String>` and
  `holds: fn(&str, u64) -> Result<(), String>`, not one `settable(name,
  value)`. `NZCV_FLAGS` (`prompt/command.rs`) and `NZCV_BITS` (`arm64.rs`) were
  the same constant; one, `arm64::NZCV_BITS`, remains.
- **`arm64` must compile on every host.** `arch::host()` answers `AARCH64` on a
  host with no backend (§3), so `mod arm64` loses its `cfg`. Its register file
  (`General`, `Float`, `registers`, `assign`), which only the backends use,
  keeps the old `cfg` in `arm64/registers.rs`, so a host with no backend gets
  no dead code.
- **The `Effect` move happens in Phase 1, not Phase 4.** The descriptor's
  `effect` answers `arch::Effect`; wrapping `backtrace::Effect::of` would have
  meant a conversion between two copies of the same enum, written to be deleted
  three phases later. So Phase 1 adds `arm64::effect` as the moved code and
  Phase 4 switches `backtrace.rs` to it and deletes the old copy. The risk
  stays in Phase 4: `backtrace.rs` reads the old code until then.
- **The descriptor gains `longest`** (the most bytes one instruction takes, 4):
  `disassemble.rs` sizes a counted read as `count * 4` and must keep doing so
  for a 2-or-4-byte processor.
- **Help text is compared with `52e4535b3`'s, not `c0a48dd4f`'s** (Phase 2's
  test): the text at the plan's base commit is the one this plan must keep.

## Summary

The risk is the unwinder move: hand-written bit matching relocated between
files. Everything else is threading one argument. Output, messages, help, the
spec and the man pages are untouched.
