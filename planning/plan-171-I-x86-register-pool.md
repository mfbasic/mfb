# plan-171-I: x86-64 register pool (`rbp`, `r13`, `rcx`, `rsi`, `rdi`, `r8`, `r9` become allocatable)

Last updated: 2026-10-21
Effort: large (3h–1d)
Depends on: plan-171-H (letter order; H also brings plan-169 in as a
prerequisite, which this letter needs for Win64 unwind of the new saves)

The x86-64 allocator colors values onto four registers: `r10 r11 r12 r14`
(`src/arch/x86_64/regmodel.rs:INT_ALLOCATABLE`, and `WIN64_INT_ALLOCATABLE`
is the same list). AArch64 gets 14 and riscv64 12. Everything else spills. In
`sorts` (`linux-x86_64`, `-O1`), 306,886 of 1,169,823 instructions (26.2%) are
`rsp`-based loads and stores; on `macos-aarch64` the same program has 14.4%.

The main obstacle to more registers is that the allocator does not know a call
reads its argument registers. A `bl`/`blr`/`svc` names no argument operand
(`src/codegen/engine/regalloc/analysis.rs:effect`), so between `mov rdi, x` and
the call, `rdi` looks free. Every register that ever carries an argument
(`rdi rsi rdx rcx r8 r9`, the internal 7th/8th `rax`/`rbp`, and the closure
environment `r13`) is kept out of the pool for that reason.

This sub-plan teaches liveness that a call reads exactly the argument registers
whose definitions reach it. It then adds `rbp`, `r13`, `rcx`, `rsi`, `rdi`,
`r8`, `r9` to both x86 pools.

Outcome: on `linux-x86_64` and `windows-x86_64` the integer pool is 11
registers. `rbp` stays the internal 8th argument register and `r13` stays the
closure environment register; both are allocatable wherever they carry
nothing. Every program's output is unchanged, and AArch64/riscv64 output is
byte-identical.

References: plan-171-A's list, plus
- `src/arch/x86_64/regmodel.rs`: `INT_ALLOCATABLE`, `WIN64_INT_ALLOCATABLE`,
  `INT_CALLER_SAVED`, `INT_CALLEE_SAVED`, `WIN64_INT_CALLEE_SAVED`,
  `closure_env`, `current_thread`, `ARENA_BASE_REGISTER`.
- `src/arch/x86_64/select.rs`: `CALL_ARGS`, `CALL_ARGS_WIN64`, `SYS_ARGS`,
  `C_RETS`, `map_scratch_register`'s `POOL`.
- `src/codegen/engine/regalloc/analysis.rs`: `effect`, `call_clobber_mask`,
  the physical-liveness dataflow; `linear_scan.rs` (the "never colored to a
  busy physical" rule, `phys_busy_in`).
- `src/codegen/engine/operand/operand.rs`: `Operand::Abi`
  (`%argMFB*`/`%argC*`/`%argSys*`), `%closure_env`.
- `src/arch/x86_64/encode/emitter/integer.rs`: `var_shift`, `var_shift_w`,
  `div_seq`; `encode/emitter/instruction.rs`: `rbit`, `umulh`/`smulh`,
  `fcvtas_x_from_d`.
- plan-169-A/B/C (Win64 unwind codes for saved registers; encoder scratch in
  frame slots).

## Prerequisites

See plan-171-A §Prerequisites. In addition:

| Must be true | Command | Status |
|---|---|---|
| plan-171-H complete | `ls planning/plan-171-H-* 2>/dev/null` → no match | NOT MET |
| plan-169 complete | `ls planning/plan-169-* 2>/dev/null` → no match | NOT MET |

## 1. Goal

- Physical liveness (`analysis.rs`) gains two facts, both from the register
  model:
  - **A call reads its argument registers.** At every `bl`/`blr`/`svc`, the
    model's argument registers that have a reaching definition count as used.
    The registers are `RegisterModel::call_argument_registers()`; on x86 that
    is `CALL_ARGS ∪ SYS_ARGS ∪ {closure_env}` (SysV) or `CALL_ARGS_WIN64 ∪
    {closure_env}` (Win64).
  - **A call defines what it clobbers.** It writes `call_clobber_mask` in the
    allocator's liveness, as `integer_live_out` already does.
  Gated by `RegisterModel::models_call_argument_reads()` (x86 `true`; AArch64
  and riscv64 `false`, so their liveness is unchanged).
- `INT_ALLOCATABLE` and `WIN64_INT_ALLOCATABLE` become
  `r10 r11 r12 r14 rbp r13 rcx rsi rdi r8 r9`. Preference order: the
  caller-saved ones first for values that do not cross a call, as today's
  coloring order does.
- `rax` (encoder scratch, 7th internal argument, C return), `rdx` (`div_seq`
  overwrites it unsaved; `rbit` rejects it as `dst`), `rbx` (`%thread`) and
  `r15` (arena base) stay reserved.

### Non-goals

- No calling-convention change: `CALL_ARGS`, `CALL_ARGS_WIN64`, `C_RETS` and
  the closure-environment register are unchanged.
- No change to AArch64 or riscv64 allocation (the gate is off there).
- No thread-local storage for `rbx`/`r15` (owner decision 2026-10-21).
- No change to which values cross a call in a register. A value live across a
  call still needs a register its callee preserves (`call_clobber_mask`).

## 2. Current State

- **Allocator soundness rule.** A vreg is never colored to a physical that is
  live, used or defined anywhere in its interval (`linear_scan.rs` module doc;
  `phys_busy_in`). Selection runs before allocation (`builder_registers.rs`:
  "NIR → MIR → select → alloc"), so the allocator sees x86 register names,
  including every hard-coded register in hand-written helpers
  (`map_scratch_register`'s `POOL` maps `x9`… onto `rbx rsi rdi r8 r9 r10 r11
  r12 r13 rcx rbp`).
- **The gap.** `effect` gives a call no uses. In the allocator's physical
  liveness, a call adds no definitions either (`call_clobber` is collected
  separately). `integer_live_out`, used by the post-allocation peepholes, does
  add the clobber as a definition. So an argument register is "busy" only at
  its own `mov`, and coloring a vreg onto it between setup and call would
  corrupt the argument. That is why the regmodel comments exclude `rdi`/`rsi`
  ("so the allocator never colors them while they carry an argument") and
  `r13` ("so no body vreg collides with the closure call's hardcoded env
  write").
- **Where argument definitions sit** (MIR of `sorts`, `mfb build --mir --target
  linux-x86_64`, `/tmp/p171x/argcheck.py`): 47,898 calls. 112,143 definitions
  of `%arg*` tokens; 110,146 reach a call in the same block. 1,995 cross a
  conditional branch first; all inspected ones are the
  `ldr %argC0; br_cc_imm %argC0, 0 → owned_value_free_skip_N; …; call` shape.
  2 are redefined before any call. A same-block rule would therefore be
  unsound, and a reaching-definitions dataflow is required (§4.1).
- **Closure calls** stage the environment exactly like an argument:
  `str %closure_env → slot; mov %closure_env, env; call_indirect; ldr
  %closure_env ← slot` (`sorts.mir`, the `transform_call_ok_24` site).
- **Encoder expansions that use fixed registers** (audited by reading every
  push/pop and fixed-register write in `encode/emitter/`):
  - `var_shift`/`var_shift_w` take the count in `rcx`. They push/pop `rcx` and
    handle `dst`/`value` = `rcx`.
  - `umulh`/`smulh` and `rbit` push/pop `rax`/`rdx`. `rbit` errors when `dst` is
    `rax`/`rdx`.
  - `fcvtas_x_from_d` pushes/pops `rax` or `rcx`, whichever is not `dst`.
  - `div_seq` overwrites `rax` and `rdx` **without saving them** ("rax/rdx are
    non-allocatable, so clobbering them is safe").
  - `svc` (`syscall`) overwrites `rcx` and `r11`; both are in
    `INT_CALLER_SAVED`, so `call_clobber_mask` already covers them.
  Hence `rcx` can join the pool and `rdx` cannot. plan-169-C moves the Win64
  push/pop scratch into frame slots; the preservation stays.
- **Reserved-register use** in `sorts` (`linux-x86_64`, operand mentions /
  functions): `r15` 6,811 / 811; `rbx` 2,009 / 48; `rax` 658 / 120; `rbp` 216 /
  67; `r13` 162 / 41. 66 calls pass an 8th argument in `rbp`.

### Measured populations

`sorts`, `linux-x86_64`, `-O1`. Pool probes are HEAD (`0c6456873`) with only
`INT_ALLOCATABLE` edited, in a detached worktree (`/tmp/p171x/probe.sh`). These
probe builds lack §4.1, so they measure size only, not correctness.

| pool | instructions | `rsp` loads/stores | `mov` |
|---|---|---|---|
| today (4) | 1,169,823 | 306,886 | 169,080 |
| + `rbp` (5) | 1,105,112 (−5.5%) | 246,707 | 164,550 |
| + `rbp r13` (6) | 1,062,970 (−9.1%) | 206,052 | 163,063 |
| + `rbp r13 rsi rdi r8 r9` (10) | 1,009,291 (−13.7%) | 154,451 | 160,988 |
| *(not chosen)* + `rbp r13 rbx r15` (8) | 1,034,386 (−11.6%) | 179,503 | 161,029 |

With §4.1 a call keeps its argument registers busy from their setup to the
call. The probe allocator did not, so the real result is smaller than the
10-register row. Phase 3 measures it. `rcx` was not in any probe; Phase 3
measures it too.

## 3. Design Overview

Two independent pieces, layered:

1. **Liveness** (shared code, gated by the model; inert until a pool grows):
   calls read their reaching argument registers and define their clobbers.
2. **Pools**: the seven registers join both x86 pools. `finalize_frame` already
   saves any callee-saved register the coloring uses (`used_callee_saved`),
   which covers `rbp`/`r13`, and `rsi`/`rdi` on Win64. plan-169's unwind codes
   describe those saves.

**Correctness risk** is entirely in piece 1. A missed implicit read is a
silently corrupted argument. Mitigations: the reaching-definitions analysis
over-approximates in every unknown case (§4.1), unit tests cover every call
shape, and the x86 runtime proofs on 2227/2228 and box 206 run the whole
acceptance corpus.

**Gate class:** behavior-preserving, golden-changing. Expected diffs: every
`linux-x86_64` and `windows-x86_64` golden. An aarch64 or riscv64 golden diff
is a bug to localize (the gate keeps their liveness identical).

**Rejected alternatives**
- *Same-block rule (a call reads the argument registers written since the
  previous call in its block):* unsound for the 1,995 cross-branch setups
  measured in §2.
- *Every call reads every argument register:* sound, but the reaching
  definition of an untouched argument register reaches back to the previous
  call or to entry. That keeps the register busy across whole stretches, which
  removes most of the gain.
- *An `args` count field on every call:* 255 `branch_link(` constructions in 98
  files (`rg -c 'branch_link\(' src` summed; `rg -l … | wc -l`). The dataflow
  needs none of them changed.
- *`rbp` as an `rbp`-based frame base instead of a register:* about 1 byte per
  `rsp` access (306,886) vs. 64,711 fewer instructions. The owner chose the
  register.
- *Thread-local `rbx`/`r15`:* owner decision; it adds a load at each of 8,820 uses.
- *`rdx`:* `div_seq` and `rbit` rely on it being unallocatable (§2). Freeing it
  means rewriting both expansions for about one register's worth of gain.

## 4. Detailed Design

### 4.1 Call argument reads (reaching definitions)

In `analysis.rs`, when `model.models_call_argument_reads()`:

- Let `A` be the mask of `call_argument_registers()`.
- Forward dataflow over `build_cfg`'s blocks: `reach[i]` ⊆ `A` is the set of
  argument registers with a definition reaching instruction `i`.
  - A definition is any explicit def of a register in `A`.
  - At entry, the reaching set is the function's incoming argument registers:
    the first `params` of the convention's argument list (`params` is the
    function's declared parameter count). Synthetic helpers whose parameter
    count is unknown start with all of `A`, and so does `%closure_env` for a
    function that is a closure body.
  - A call kills `call_clobber_mask(call) ∩ A`. Callee-saved argument
    registers (`rbp`, `r13`, and `rsi`/`rdi` on Win64) survive a call, so their
    definitions keep reaching later calls. This over-approximates, which is
    sound.
  - Join is union (any path).
- At each call `c`, `phys_use[c] |= reach[c]`, and `phys_def[c] |=
  call_clobber_mask(c)`.

Effect: an argument register is busy from its setup to the call that reads it.
A result register is busy from the call to its read. Outside those windows the
register is free to color.

### 4.2 Pools

`INT_ALLOCATABLE` and `WIN64_INT_ALLOCATABLE`:
`["r10","r11","rcx","rsi","rdi","r8","r9","r12","r14","rbp","r13"]` (the
caller-saved registers first). Regmodel tests that pin the old lists change
with their reason (AGENTS.md four questions answered in the commit):
`regmodel.rs` tests at the `INT_ALLOCATABLE` asserts, and the
`!["rax","rcx","rdx","rsi","rdi","r8","r9","rsp","rbp"].contains(reg)` assert.
That assert pins "no ABI register is allocatable". This sub-plan overturns
it deliberately, and the proof is §4.1 plus the Phase 2 tests.

The regmodel comments that justify the old exclusions (`rdi`/`rsi` "while they
carry an argument", `r13` "hardcoded env write", "Tight (4)") are rewritten to
point at §4.1.

## Phases

> **NOTE — keep the checkboxes current** (same rules as plan-171-A).

### Phase 1 — liveness: calls read reaching argument registers (byte-identical)

- [ ] `RegisterModel::call_argument_registers()` and
      `models_call_argument_reads()` (x86 SysV/Win64 return the §1 sets and
      `true`; AArch64 and riscv64 return `false`).
- [ ] `analysis.rs`: the reaching-definitions pass and the `phys_use`/
      `phys_def` additions of §4.1. The function's parameter count is threaded
      from the builder (`run_register_allocation`) into the analysis.
- [ ] Tests in `src/codegen/engine/regalloc/tests/call_argument_reads.rs`
      (test-only file):
      - setup and call in one block;
      - setup before a conditional branch (the `owned_value_free_skip` shape);
      - a redefinition before the call;
      - an untouched argument register (not busy);
      - `rbp` set for one call and reaching a second (busy across, by design);
      - `%closure_env` before `call_indirect`;
      - `svc` with `%argSys*`;
      - a result register read after a call (busy only from the call);
      - entry with 2 params vs. an unknown-param helper.

Acceptance: the analysis is correct, and no target's code changes (every pool
is still the old one).
  Check: `cargo test --bin mfb regalloc::` → pass (est. 3 min);
  `bash scripts/artifact-gate.sh target/release/mfb all` → `0 diff(s)` (est. 15 min — the only check that every target's allocation is untouched by the new liveness facts).
Commit: —

### Phase 2 — encoder expansions on the new registers

- [ ] `src/arch/x86_64/encode/tests/pool_registers.rs` (test-only): for every
      expansion in §2's audit (`var_shift`, `var_shift_w`, `umulh`, `smulh`,
      `rbit`, `fcvtas_x_from_d`, `div_seq`, `alu3`, `enc_add_carry`,
      `enc_sub_borrow`, `msub`), each operand position on each of `rbp r13 rcx
      rsi rdi r8 r9`. The host cannot execute x86
      (`encode/tests/lanes_plans_and_aliasing.rs`: "The x86 backend cannot be
      executed from this host"), so each case asserts the exact expected
      bytes. It also asserts structurally that every fixed register the
      sequence writes, other than `dst`, is pushed before and popped after.
      Execution is proven by the Phase 3 fixture on 2227/2228.
- [ ] Memory operands with base `rbp`/`r13` (ModRM `mod=00` means
      RIP/disp32 there, so they need an explicit displacement) and index/base
      `r13`: encoder tests for `ldr`/`str` at offset 0 and at large offsets.
- [ ] Fix any expansion that fails, in the same commit as its test.

Acceptance: every expansion is correct on every new pool register.
  Check: `cargo test --bin mfb x86_64::encode` → pass (est. 3 min).
Commit: —

### Phase 3 — the pools

- [ ] `regmodel.rs`: the §4.2 lists, rewritten comments, and the regmodel tests
      updated as §4.2 states.
- [ ] Census: re-run the §2 table's command on `sorts` for `linux-x86_64` and
      `windows-x86_64`; record instructions, `rsp` loads/stores and `mov`s in
      Corrections.
- [ ] rt fixture `tests/rt-behavior/functions/x86-pool-pressure` (valid): it
      has an 8-parameter function (passes `rax`/`rbp`), closures called in
      loops (`r13`), calls between which many locals stay live, a variable
      shift, a divide and a high multiply in the same expressions, and a
      syscall-backed builtin (`os::` file write) amid live locals. It prints a
      checksum.
- [ ] `rt_regalloc_levels_agree`: add the fixture.
- [ ] Golden re-baseline (`linux-x86_64`, `windows-x86_64` only), following
      plan-171-A Phase 3's procedure.
- [ ] Runtime: `bash scripts/linux-runtime-proof.sh target/release/mfb 2227 linux-x86_64`
      (musl) and `… 2228 linux-x86_64` (glibc); Windows box 206: the
      acceptance runnable fixtures and plan-169-C's attach-and-backtrace
      check on an `-O1` build.
- [ ] Size: `ls -l` of the four corpus programs' `linux-x86_64` builds,
      before and after, in Corrections. `benchmark/language/run.sh` runs on
      the local host only (its README), so there is no x86 timing run here.

Acceptance: 11-register pools on both x86 targets; outputs unchanged on every
x86 runtime proof.
  Check: `linux-runtime-proof.sh … 2227` and `… 2228` → 0 mismatches (est. 60–120 min each, emulated — the only Linux x86 execution);
  box 206 fixtures and backtrace check → pass (est. 30 min — the only Win64 execution).
Commit: —

### Phase 4 — spec and docs

- [ ] `mfb spec architecture x86_64-instruction-set` (or the register-model
      topic): the pool, the reserved registers and why. `mir-instruction-set`
      / the allocator topic: calls read reaching argument registers.
- [ ] `.ai/arch-abi.md` x86-64 section: "argument and closure-env registers are
      allocatable; the allocator keeps them busy from setup to call by
      reaching definitions. `rax`/`rdx`/`rbx`/`r15` stay reserved, each with its
      reason".

Acceptance: specs render.
  Check: `cargo test --bin mfb spec` → pass (est. 2 min).
Commit: —

## Validation Plan

- Tests: `regalloc::` units (call argument reads), x86 encoder pool-register
  tests, the fixture, levels-agree.
- Coverage: the fixture reaches every implicit-read shape of Phase 1. Check
  with `-ncode` that its `linux-x86_64` dump colors at least one value onto
  each of the seven new registers.
- Runtime proof: 2227, 2228, box 206. AArch64/riscv64 are byte-identical
  (Phase 1 gate; `artifact-gate.sh` again after Phase 3 shows diffs only in
  x86 goldens).
- Doc sync: Phase 4.
- Final gate (once, after Phase 4): `cargo test --no-fail-fast > /tmp/171i.log 2>&1; echo EXIT=$?` → `EXIT=0`, `grep -c '^failures:' /tmp/171i.log` → 0 (est. 60–90 min).

## Decisions (owner, 2026-10-21)

1. Free `rbp`, `r13` and the argument registers; keep `rbx`/`r15` pinned.
2. `rbp` is a register, not a frame base.
3. Both x86 targets, with plan-169 complete first.
4. This is plan-171 letter I, ahead of J (frame and memory operands) and K
   (short encodings, closing review).

## Corrections

<Filled in during execution.>

## Summary

The x86 backend spills because it has four registers. This sub-plan frees
seven more by giving liveness the one fact it lacked: a call reads its
arguments. Probe builds put the size effect at up to −13.7% of instructions in
`sorts`. Correctness rests on one dataflow, tested shape by shape and run on
both x86 operating systems.
