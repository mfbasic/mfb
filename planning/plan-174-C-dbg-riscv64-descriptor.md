# plan-174-C: the riscv64 descriptor for `mfb dbg`

Last updated: 2026-10-12
Effort: large (estimate: 3h–1d)
Depends on: plan-174-A, plan-174-B

Adds `src/debugger/riscv64.rs`: the riscv64 register file, the unwinder's
reading of riscv64 instructions, the ELF constants and import-stub reader, and
`pub(crate) const RISCV64: Arch`. All of it is plain computation over bytes, so
it compiles and is unit-tested on every host, the Mac included. The outcome:
given the bytes of a riscv64 thread state and of riscv64 code, the shared
debugger code lists registers, disassembles, finds returns and unwinds
correctly, proven by unit tests fed with real bytes from the boxes.

No process is debugged in this sub-plan. The Linux backend that produces those
bytes is plan-174-D.

References:

- plan-174-A §3: `arch::Arch`, `Decoded`, `Effect`, `ElfArch`.
- plan-174-B §3: `riscv64::decode::Instruction`, `Flow`.
- `src/arch/riscv64/regmodel.rs`: `GPRS`, `FP_REGS`, `arena_base`,
  `closure_env`, `current_thread`.
- `mfb spec memory native-calling-convention`: the RISC-V 64 row of the
  argument and result table; "Stack Frame, Prologue, and Epilogue".
- `src/os/linux/link/mod.rs:emit_import_stub`: the riscv64 import stub.
- `src/debugger/tests.rs:linked_probes`: already links a riscv64 ELF probe.

## Prerequisites

The prerequisites are stated once in plan-174-A. In addition:

| Must be true | Command | Status |
|---|---|---|
| plan-174-A is complete | `ls planning/plan-174-A-*` → no match | MET (2026-10-20: `ls planning/plan-174-A-*` → no match; archived to `planning/completed/`) |
| plan-174-B is complete | `ls planning/plan-174-B-*` → no match | MET (2026-10-20: `ls planning/plan-174-B-*` → no match; archived to `planning/completed/`) |

If either is not complete, this plan cannot start.

> **NOTE: the Status column is a snapshot; the Command column is the truth.**
> Re-run every command before you continue. If you stop, report all of them.

## 1. Goal

- `riscv64::RISCV64` is a complete `Arch`. A test fake whose `arch()` answers it
  drives `list reg`, `dis`, `finish`'s return search, `bt`, `jump`'s checks and
  location parsing through the shared code with riscv64 names and bytes.
- `elf::read` with `riscv64::ELF` reads a riscv64 executable and library:
  symbols, `R_RISCV_JUMP_SLOT` imports and the 12-byte import stubs.

### Non-goals (explicit constraints)

- No `cfg(target_arch)` and no `ptrace`. Nothing here runs a process.
- No change to AArch64 behaviour or to any existing test's expectation.
- Vector registers are not listed: neither box has `V`
  (`.ai/remote_systems.md`, "RISC-V Vector (`V`) status"), so the kernel has no
  vector state to read.
- `x`-number register names (`x10`) are not accepted anywhere.

## 2. Current State

After plan-174-A the shared code reads everything architecture-specific from
`Arch`, and `arm64::AARCH64` is the only value. After plan-174-B
`riscv64::decode` exists with no caller.

The riscv64 facts this sub-plan encodes, each measured on the boxes on
2026-10-12:

- **Thread state.** `struct user_regs_struct` is 32 `unsigned long` in the
  order `pc, ra, sp, gp, tp, t0, t1, t2, s0, s1, a0`–`a7`, `s2`–`s11`,
  `t3`–`t6` (`/usr/include/riscv64-linux-gnu/asm/ptrace.h` on box 2232).
  `PTRACE_GETREGSET` with `NT_PRSTATUS` (1) returns 256 bytes; with `NT_PRFPREG`
  (2) it returns 264 bytes, `struct __riscv_d_ext_state { __u64 f[32]; __u32
  fcsr; }`. Probe `/tmp/rvprobe/probe.c` on both boxes: `GETREGSET NT_PRSTATUS
  -> 0 len 256`, `GETREGSET NT_PRFPREG -> 0 len 264 (struct 264)`.
- **Roles.** `mfb spec memory native-calling-convention`: arguments
  `a0`–`a7`, results `a0`–`a3`. `regmodel.rs`: arena base `s11`, closure
  environment `s10`, current thread `s2`. The C convention returns in `a0`,
  `a1`.
- **Function shape.** `objdump` of `_mfb_fn_isPrime` in `examples/debug` built
  for `linux-riscv64`:

  ```text
  17e8:  addi  sp,sp,-208
  17ec:  sd    ra,0(sp)
  ...
  1a90:  auipc ra,0xf
  1a94:  jalr  -260(ra) # 1098c <_mfb_make_error_result>
  1aa0:  ld    ra,0(sp)
  1aa4:  addi  sp,sp,208
  1aa8:  ret
  ```

- **Executable imports.** The `--debug` executable has no `.plt` section;
  `readelf --use-dynamic -r` shows 72 `R_RISCV_JUMP_SLOT` and no other type.
  `emit_import_stub` writes `auipc t3, hi; ld t3, lo(t3); jalr x0, 0(t3)`, 12
  bytes, the size AArch64's stub has.
- **Library `.plt`.** glibc 2.41's `libc.so.6` on box 2232: a 32-byte header,
  then 16-byte entries `auipc t3; ld t3,…(t3); jalr t1,t3; nop`
  (`objdump -d -j .plt`). `elf.rs`'s `PLT_HEADER` (32) and `PLT_ENTRY` (16) fit
  unchanged.
- **ELF identity.** `EM_RISCV` is 243 (`/usr/include/elf.h`);
  `src/os/linux/link/mod.rs` has `R_RISCV_64 = 2`, `R_RISCV_JUMP_SLOT = 5`.

### Measured populations

| What | Count | Command |
|---|---|---|
| General registers in the thread state | 32 (`pc` + `x1`–`x31`) | `struct user_regs_struct` on box 2232 → 32 fields |
| FP registers | 32 + `fcsr` | `struct __riscv_d_ext_state` → `f[32]`, `fcsr` |
| Dynamic relocation types in a riscv64 `--debug` executable | 1 (`R_RISCV_JUMP_SLOT`, 72 entries) | on 2232: `readelf --use-dynamic -r -W /tmp/debug-glibc.out \| awk 'NR>2{print $3}' \| sort \| uniq -c` |
| `elf.rs` unit tests that name AArch64 | 5 | `grep -c '#\[test\]' src/debugger/elf.rs` → 5 |
| `backtrace.rs` unit tests | 7 | `grep -c '#\[test\]' src/debugger/backtrace.rs` → 7 |

### Verified properties

- **`pc` stays on a trapping `ebreak`.** Probe on both boxes: `stop 0: sig 5
  si_code 1 … pc 0x…c966 insn-at-pc 0x100073`, and for `c.ebreak` `insn-at-pc
  0x…9002`. The shared code's "a breakpoint stop leaves `pc` on the breakpoint"
  holds, so `breakpoints.rs` needs nothing riscv64-specific.
- **The return address is in a register on entry** (`ra`), as `lr` is on
  AArch64, so `backtrace::FrameState::ENTRY` carries over.
- **The compiler keeps no frame-pointer chain on riscv64** either: the spec says
  "There is **no `x29` frame-pointer chain**" for every target, and the function
  above saves only `ra`. C library code does use `s0` as a frame pointer, which
  the unwinder's frame-register effects cover.
- **User addresses need no masking.** riscv64 has no pointer authentication, and
  the saved `pc` values the probes printed (`0x7fffb44f7e8e`, `0x1527a`) are
  plain addresses, so `address_mask` is `u64::MAX`.
- **`tests.rs:linked_probes` already builds a riscv64 ELF**
  (`riscv_ret = [0x67, 0x80, 0x00, 0x00]`), so the ELF tests have a fixture.

## 3. Design Overview

`riscv64.rs` mirrors `arm64.rs` piece for piece:

| Piece | AArch64 (`arm64.rs`) | riscv64 (`riscv64.rs`) |
|---|---|---|
| general state | `General = [u64; 34]` | `General = [u64; 32]`, `user_regs_struct` order |
| FP state | `Float { v: [u128; 32], fpsr, fpcr }` | `Float { f: [u64; 32], fcsr: u32 }`, 264 bytes |
| `gpr` group | `x0`–`x28`, `fp`, `lr` | `ra`, `gp`, `tp`, `t0`–`t2`, `s0`, `s1`, `a0`–`a7`, `s2`–`s11`, `t3`–`t6` |
| `sys` group | `sp`, `pc`, `nzcv`, `fpsr`, `fpcr` | `sp`, `pc`, `fcsr` |
| `simd` group | `v0`–`v31`, 128-bit | `ft0`–`ft7`, `fs0`, `fs1`, `fa0`–`fa7`, `fs2`–`fs11`, `ft8`–`ft11`, 64-bit |
| roles | `x0`–`x7`, `x19`, `x20`, `x28` | `a0`–`a7`, `s11`, `s2`, `s10` |
| `pc` / `sp` / link / frame | `pc`, `sp`, `lr`, `fp` | `pc`, `sp`, `ra`, `s0` |
| `finish` results | `x0`, `x1` | `a0`, `a1` |
| park registers | code `x1`, origin `x3` | code `a1`, origin `a3` |
| settable by `jump` | `x0`–`x28`, `fp`, `lr`, `sp`, `nzcv`, `fpsr`, `fpcr` | every `gpr` register, `sp`, `fcsr` (32 bits) |
| instruction alignment | 4 | 2 |
| address mask | 48 bits (47 on macOS) | none |
| watch span | `Some(8)` | `None` |

The `gpr` order is `x1` to `x31` with `sp` moved to `sys`, where AArch64 has it.
The zero register is not listed. The role strings are the ones AArch64 uses, so
`list reg` reads the same on both: `a0`, `a1` are `MFB arg, MFB result, C arg,
C result`; `a2`, `a3` are `MFB arg, MFB result, C arg`; `a4`–`a7` are `MFB arg,
C arg`; `s11` `Arena`; `s2` `Current Thread`; `s10` `Closure`.

**Correctness risk** concentrates in the unwinder's effects: a wrong reading of
a stack adjustment walks the stack into garbage and prints a plausible wrong
backtrace. The effects are tested with the real words of a compiled function
and of C library prologues taken from plan-174-B's corpora.

**Byte-identity is not this sub-plan's gate.** New code is added behind a
descriptor nothing selects in production yet; AArch64 tests must stay green as
the neutrality check, and the new behaviour is gated by its own unit tests.

Rejected alternatives:

- **Order `gpr` as the kernel does, `pc` first.** The listing is for a person;
  `x`-number order with `sp` and `pc` under `sys` matches what `list reg gpr`
  shows on AArch64.
- **Show FP registers as `f0`–`f31`.** The compiler's `FP_REGS` and `objdump`
  both use the ABI names, and `dis` output would then name registers `list reg`
  does not.

## 4. Detailed Design

### 4.1 Registers

`registers(general, float) -> Vec<Register>` in the table's order, each
`RegisterValue::Bits64`, `role` set from `role(name)`. `slot(name)` maps a name
to its `General` index (`pc` 0, `ra` 1, `sp` 2, `gp` 3, `tp` 4, `t0`–`t2` 5–7,
`s0` 8, `s1` 9, `a0`–`a7` 10–17, `s2`–`s11` 18–27, `t3`–`t6` 28–31) or to
`fcsr`. `assign` writes one; `fcsr` takes 32 bits and reads the FP state on
first use, as `arm64::assign` does for `fpsr`. `settable(name, value)` refuses
`pc` ("jump moves the thread; give the place as its target"), an FP register,
and an `fcsr` value over 32 bits, with messages in the form `arm64::settable`
uses.

### 4.2 Decoder adapter

`decode(bytes, pc)` wraps `riscv64::decode::decode` into `arch::Decoded`:
`returns` is `Flow::Return`; `page`/`page_offset` carry `auipc` and the
`addi`/`ld`/`jalr` that completes it, so `disassemble::listing` names the target
of `auipc ra,0xf` + `jalr -260(ra)` as it names `adrp` + `add` today. `length`
is `riscv64::decode::length`.

`disassemble::listing` prints the decoder's text with the mnemonic padded to
the column AArch64 listings use and the tab replaced by that padding, and an
undecoded instruction as `.word  0x%08x` (4 bytes) or `.short 0x%04x` (2).

### 4.3 Unwinder effects

`effect(bytes) -> Effect` reads the instruction and answers, in plan-174-A's
vocabulary:

| Instruction | Effect |
|---|---|
| `addi sp,sp,imm`, `c.addi16sp`, `c.addi sp,imm` | move `sp` by `imm` |
| `sd ra,off(sp)`, `c.sdsp ra,off` | link saved at `sp + off` |
| `sd s0,off(sp)`, `c.sdsp s0,off` | frame saved at `sp + off` |
| `sd ra,off(s0)`, `sd s0,off(s0)` | saved at `frame + off` |
| `ld ra,off(sp)`, `c.ldsp ra,off`, and the `s0` forms | link or frame restored |
| `addi s0,sp,imm`, `c.addi4spn s0,imm` | frame = `sp + imm` |
| `addi sp,s0,imm` | `sp` = `frame + imm` |
| `ret`, `c.jr ra` | return |
| `j`, `c.j`, `jr`, `c.jr` to outside the function | jump away |
| `nop`, `c.nop` | hint |
| any other write to `sp` | `sp` unknown |
| anything else | none |

A prologue instruction, for `Effect::in_prologue`, is an `sp` decrement, a store
through `sp` or `s0`, `addi s0,sp,imm`, or a hint.

### 4.4 ELF

`riscv64::ELF = ElfArch { machine: 243, jump_slot: 5, data_slot: 2, stub_size:
12, stub_slot }`. `stub_slot(window, address)` matches `auipc t3, hi` (low 12
bits `0xe17`), `ld t3, lo(t3)` (`funct3` 3, `rs1` = `rd` = 28, opcode `0x03`)
and `jalr x0, 0(t3)` (`0x000e0067`), and answers the GOT slot
`address + sign_extend(hi << 12) + sign_extend(lo)`.

## Compatibility / Format Impact

None on any supported host. `RISCV64` is reachable only from tests until
plan-174-D.

## Phases

> **NOTE: keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work. Fill the phase's `Commit:` line when it lands. An unticked
> box means NOT DONE.

### Phase 1 — the register file

- [x] Add `src/debugger/riscv64.rs` with `General`, `Float`, `registers`,
      `role`, `slot`, `assign`, `is_register`, `settable`; declare `mod riscv64`
      in `mod.rs` with no `cfg`. (As `riscv64/mod.rs` and
      `riscv64/registers.rs`, the shape `arm64/` took in plan-174-A; with
      `holds`, `jump`'s value check, and `pc`/`sp` readers for the backend; and
      under `cfg(any(all(linux, riscv64), test))`: see Corrections.)
- [x] Tests in `riscv64.rs`: `the_layouts_are_the_kernel_s` (`size_of::<General>()
      == 256`, `size_of::<Float>()` rounded as the regset is, 264 bytes read);
      `roles_name_the_registers_the_compiler_uses`, checking `role` against
      `Riscv64RegisterModel::arena_base`, `closure_env`, `current_thread` and the
      argument and result counts of `abi_token`, as
      `arm64::tests::roles_name_the_registers_the_compiler_uses` does;
      `registers_list_in_group_order`; `assign_writes_one_register`. (In
      `riscv64/registers/tests.rs`. The role test maps each AArch64 position
      to its riscv64 register with the selection's own
      `select::remap_register`, made `pub(crate)` for it.)

Acceptance: the register file matches the kernel layout and the compiler's
register model.
  Check: `cargo test --bin mfb debugger::riscv64` → 4 passed (est. 2 min).
  Measured 2026-10-20: `4 passed; 0 failed`.
Commit: —

### Phase 2 — the descriptor, the decoder adapter and listings

- [ ] `riscv64.rs`: `decode`, `length`, `help` (the register lines of `help` and
      `list_reg_usage` for riscv64), and `pub(crate) const RISCV64: Arch` with
      `effect` wired to Phase 3's function (added here answering only `ret` and
      `none`, completed in Phase 3).
- [ ] `disassemble.rs`: print an undecoded 2-byte instruction as `.short`.
- [ ] `disassemble.rs:counted` (added from plan-174-A Phase 3): `dis -<n>`
      steps back `n * alignment` bytes, which is `n` instructions only when
      every instruction is `alignment` bytes. With 2-and-4-byte code, find the
      `n`th instruction start before the anchor by decoding forward from the
      start of the function holding it, and refuse (as today's "there are not
      `n` instructions before") when the walk cannot reach back that far.
      AArch64's result is unchanged: its starts are every 4 bytes from any
      function's start. Test: `on_riscv64_dis_counts_back_by_instructions`.
- [ ] `prompt.rs` test fake `Recorder`: add an `arch: &'static Arch` field,
      default `&AARCH64`, with riscv64 code and registers when it is `&RISCV64`.
- [ ] Tests in `prompt.rs`: `on_riscv64_list_reg_shows_the_role_registers`
      (`a0`–`a7`, `s11`, `s2`, `s10`, then the riscv64 usage);
      `on_riscv64_dis_lists_two_and_four_byte_instructions` over the bytes of
      `addi sp,sp,-208`, `sd ra,0(sp)`, `c.mv`, `ret`;
      `on_riscv64_locations_take_abi_names` (`show mem sp+16`, `a0`, and `x10`
      refused as no register or function).

Acceptance: the shared code lists riscv64 registers and code through a fake.
  Check: `cargo test --bin mfb debugger::prompt::tests::on_riscv64` → 3 passed
  (est. 2 min).
Commit: —

### Phase 3 — the unwinder effects

- [ ] `riscv64.rs`: complete `effect` per §4.3.
- [ ] Tests in `backtrace.rs`: `a_riscv64_compiled_frame_unwinds` with the words
      of `_mfb_fn_isPrime`'s prologue and epilogue (from plan-174-B's corpus of
      `debug-glibc.out`); `a_riscv64_library_frame_with_a_frame_pointer_unwinds`
      with a `libc.so.6` function that uses `c.addi16sp`, `c.sdsp ra`, `c.sdsp
      s0`, `c.addi4spn s0`; `a_riscv64_frame_before_its_prologue_ran` (the
      return address still in `ra`).

Acceptance: frames of compiled and of C library functions unwind to the right
caller and stack pointer.
  Check: `cargo test --bin mfb debugger::backtrace` → 10 passed (7 existing + 3)
  (est. 2 min).
Commit: —

### Phase 4 — ELF

- [ ] `riscv64.rs`: `ELF` and `stub_slot`.
- [ ] Tests in `elf.rs`: `a_riscv64_executable_reads_its_symbols_and_stubs`,
      linking the riscv64 probe as `tests.rs:linked_probes` does, expecting
      `/lib/ld-linux-riscv64-lp64d.so.1` and one stub per import;
      `a_riscv64_elf_is_refused_as_aarch64_and_the_reverse`.

Acceptance: `elf::read` reads a riscv64 image with `riscv64::ELF` and refuses it
with `arm64::ELF`.
  Check: `cargo test --bin mfb debugger::elf` → 7 passed (5 existing + 2)
  (est. 2 min).
Commit: —

## Validation Plan

- Tests: the twelve new unit tests above, all runnable on the Mac.
- Coverage check: `cargo test --bin mfb debugger::` runs them in the same
  target as the existing 157, so a failure blocks the same gate.
- Runtime proof: none here; plan-174-D runs real sessions. The unit tests use
  bytes captured from real riscv64 code, not hand-assembled ones.
- Doc sync: none. The observable contract changes when a riscv64 host gets a
  backend, and `mfb spec tooling debugger` is updated in that same change
  (plan-174-D Phase 3).
- Final gate: plan-174-E, once for all of plan-174.

## Decisions

- Registers are shown and accepted by ABI name only — decided by the owner,
  2026-10-12. FP registers follow the same rule and use the ABI names the
  compiler's `FP_REGS` and `objdump` use (`ft0`, `fa0`, `fs0`); the option the
  owner picked listed them as `f0`–`f31` in shorthand. (§3)
- `wp` is refused on riscv64 — decided by the owner, 2026-10-12. The
  descriptor's `watch_span` is `None`. (§3; the refusal is plan-174-D)

## Corrections

- **`mod riscv64` is compiled where it is read.** With no `cfg`, every item is
  dead in a macOS build and stays so: only a riscv64 Linux backend uses it
  (plan-174-D), and the macOS backend never will. Like the decoder it reads
  (plan-174-B's Corrections), it is under `cfg(any(all(target_os = "linux",
  target_arch = "riscv64"), test))`, so the Mac compiles it for its tests.
  Phase 2's descriptor is reached on a riscv64 Linux host through the backend,
  and in tests through the `Recorder` fake.
- **The file layout follows plan-174-A's `arm64/`:** `riscv64/mod.rs` holds the
  descriptor and its rules, `riscv64/registers.rs` the register file. A role
  test's "register after" is by riscv64 number (`a7` is followed by `s2`),
  since the compiler maps AArch64's `x8` to `a7`, the system call register.

## Summary

The risk is the unwinder's effects on compressed C library prologues. The
register file, ELF constants and listing changes are mechanical and pinned by
kernel layouts and the compiler's own tables. No process is touched, and
AArch64 is unchanged.
