# bug-284: backend register/encoding latent-hazard cluster (aarch64 / riscv64 / x86_64)

Last updated: 2026-07-17
Effort: medium (1h–2h across items)
Severity: LOW
Class: Footgun / Correctness (all latent — no current lowering triggers them)

Status: Fixed
Regression Test: per-item (arch encode tests)

A cluster of LOW-severity, latent register-lifetime / encoding-robustness hazards
across all three hand-rolled backends, found during goal-06. None is reachable
from any current MIR producer (verified per item), but each miscompiles silently
or hangs if a future lowering constructs the triggering operand combination — the
class the bug-124/125/126/154/178 fix trains addressed point-wise. The value is
converting each silent-wrong-code corner into a loud failure so a new producer
fails the build instead of shipping wrong bytes. Grouped per the repo's low-cluster
convention; distinct root causes, one document.

References:

- Found during goal-06 review of `src/arch/{aarch64,riscv64,x86_64}/**`.
- Cross-checked against bug-09/124/125/126/154/178/217/267, bug-218 (skipped),
  bug-270; none re-filed.

## Items

### C1 — aarch64: register-form ops conflate SP with XZR for operand 31 (`cmp_imm` magnitude-dependent)
- `src/arch/aarch64/encode/operand.rs:19` (`reg`), `emitter.rs:884-890`
  (`emit_cmp_imm` fallback → `emit_cmp`).
- `reg()` maps `sp`/`raw_sp`/`x31`/`xzr` all to 31 and drops the spelling; in every
  shifted-register-form instruction (cmp/add/sub/and/orr/eor/mul/div…) reg 31 in
  Rn/Rm is XZR, so an `sp` operand silently reads 0. `emit_cmp_imm(lhs="sp",
  rhs≤4095)` is correct (immediate-form Rn=31 = SP) but `rhs>4095` falls back to
  register-form `cmp` where Rn=31 = XZR — the one instruction changes meaning with
  the immediate's magnitude. bug-178 B fixed this class for `mov` only. Latent: all
  current `compare_immediate` sites use plain `xN`; SP arithmetic uses the
  31-correct immediate forms.
- Fix: keep the spelling alongside the number (as `emit_mov` does) and hard-error
  when an `sp`-spelled operand lands in a register-form Rn/Rm slot.

### C2 — aarch64: `emit_add_imm`/`emit_sub_imm`/`sized_add_sub_imm` chunk loop is unbounded
- `src/arch/aarch64/encode/sizing.rs:104-120`, `emitter.rs:802-819`.
- The chunk loop removes at most `4095 << 12` per iteration with no cap. An
  immediate near `u64::MAX` (a wrapped-negative offset from a lowering
  arithmetic-wrap bug) means ~10^12 iterations: `instruction_size` runs first, so
  the compiler spins forever before any error. `mov_imm` handles any u64 in ≤4
  words. Latent: legitimate offsets stay single-digit chunks.
- Fix: cap the decomposition (error or switch to `mov_imm scratch; add reg,reg,
  scratch` beyond 4 chunks); also shrinks worst-case code for large legitimate
  offsets.

### C3 — riscv64: pending-compare invalidation misses `carry_out`/`borrow_out` defs and call clobbers
- `src/arch/riscv64/select.rs:452`.
- A bare `cmp` keeps its rhs by register name in `pending`; the bug-126.2 fix
  invalidates on Label or on an instr whose `"dst"` field equals the rhs. But
  `add_carry` defines `carry_out`, `sub_borrow` defines `borrow_out`, and `bl`/`blr`
  clobber all caller-saved regs with no field — none invalidates `pending`. A
  `cmp x1,x2; add_carry …,carry_out=x2; b.lt` (or `cmp; bl; b.lt` with caller-saved
  rhs) re-reads a clobbered rhs → wrong branch. bug-126.2's record claims "invalidate
  on any def of the saved rhs" but only checks `dst`. Latent: fusion places the
  branch adjacent today.
- Fix: also compare rhs against `carry_out`/`borrow_out`, and clear `pending` (or at
  least `FlagRhs::Reg` for caller-saved rhs) on `bl`/`blr`.

### C4 — riscv64: `UmovXFromV` silently defaults/accepts an out-of-range lane index
- `src/arch/riscv64/v128.rs:717`.
- `idx.parse::<u8>().unwrap_or(0)` maps a missing/malformed index to lane 0, and
  there is no `half <= 1` check: `index=2` computes `off = slot*16 + 16`, reading
  the low lane of the *adjacent slot* (another value's data). AArch64 fails loudly
  ("umov .d lane index out of range"), so a builder bug that errors on aarch64
  miscompiles silently on rv64. Latent: only constructor passes literal 0/1.
- Fix: parse with `expect`/panic and assert `half <= 1`, mirroring aarch64.

### C5 — riscv64: `LdrQ`/`StrQ` high-lane offset `parse::<u64>().unwrap_or(0)` desyncs from low lane
- `src/arch/riscv64/v128.rs:328, 344`.
- `o.parse::<u64>().unwrap_or(0) + 8` makes the high-lane offset `"8"` whenever `o`
  is non-u64 (negative/symbolic/empty) while the low-lane op keeps the raw string →
  the two lanes address inconsistent locations. Fails loud today only because the
  sibling low-lane op forwards the same raw string to `operand::immediate()`, which
  rejects non-u64 and aborts — correctness depends on that coincidence.
- Fix: replace `unwrap_or(0)` with parse-or-panic so the arm fails on its own terms.

### C6 — x86_64: residual implicit-register alias gaps (var_shift dst==rcx; msub dst==rax; div_seq live rax/rdx; rbit dst∈{rax,rdx})
- `src/arch/x86_64/encode/emitter.rs:1879-1961` (var_shift), `:536-552` (msub),
  `:1985-2050` (div_seq), `:392-440` (rbit).
- Corners the bug-125 fix left open: var_shift/var_shift_w with dst==rcx overwrites
  the count with the value (shift uses wrong CL); msub with dst==rax (or dst aliasing
  lhs/rhs) yields 0; div_seq still destroys a live non-dividend rax/rdx
  unconditionally (the exact reasoning bug-125 refuted); rbit corrupts its
  accumulator when dst∈{rax,rdx}. Each produces wrong bytes for the stated combo;
  no current producer colors an operand onto those regs at the expansion. Latent.
- Fix: guard each expansion — `return Err` (like `sub_borrow` dst==rhs) or handle
  (stage value before count; capture minuend via a slot; push/pop rax/rdx in
  div_seq; reject rbit dst∈{rax,rdx}).

### C7 — x86_64: zero-token sentinel (16) silently encodes as r8/rax in unguarded operand positions
- `src/arch/x86_64/encode/emitter.rs` (`enc_mov`/`mov` `:296-300`, `cmp` `:596-601`,
  mul/umulh/div_seq/msub/shift value/amount, `add_carry` lhs `:2107-2118`);
  `operand.rs:45` (`"xzr" => 16`).
- `reg()` returns 16 for the neutral zero token; any arm not checking
  `is_zero_token` feeds it to `modrm`/`rex`, where 16&7=0 with the ≥8 REX bit
  encodes r8 (reg field) or an unintended rm — e.g. `mov dst, xzr` → `mov dst, r8`
  (the bug-154/123 failure mode, in the mirror positions not yet fixed). Current MIR
  producers only place the token in guarded positions. Latent.
- Fix: make the default loud — a `hard_reg()` wrapper that `Err`s on the token
  everywhere except the intentionally token-aware arms, so a new producer fails the
  build instead of miscompiling.

### C8 — peephole store-to-load forwarding models `Mul`/`UMulH`/`SMulH`/`SDiv`/`UDiv`/`MSub` as `DefDst`, ignoring x86 implicit rdx:rax clobbers
- `src/target/shared/code/peephole.rs:82-143` (`classify`), run for all backends via
  `src/target/shared/code/function_lowering.rs:783` (`forward_stores_to_loads`).
- `forward_stores_to_loads` forwards an sp-slot reload to a `mov` from the register
  that last stored the slot, invalidating a slot only when its source register is the
  `dst` of a modelled op. `classify` puts `Mul`/`UMulH`/`SMulH`/`SDiv`/`UDiv`/`MSub`
  in the `DefDst` set (defines exactly `dst`) — correct on aarch64/riscv (no implicit
  clobbers) but on x86 these expand to instructions that clobber rdx:rax
  (see C6/`div_seq`, `umulh`, `msub`). If a value ever lived in rax/rdx (a forwarding
  source) across one of these ops whose `dst` is a different register, the slot would
  not be invalidated and the reload would be forwarded to a clobbered register →
  silent wrong code. Latent for the same reason as C6: the x86 register model
  currently never colors a value onto rax/rdx across these ops (rax/rdx reserved for
  return/div staging). This is a soundness reliance, not a live bug.
- Fix: classify the implicit-clobber ops as `Barrier` (flush) for x86 targets, or
  make the forwarding invalidate the arch's implicit-clobber set for these ops
  (thread the active backend's clobber mask into `classify`, as `remove_fp_shuttles`
  already threads `is_riscv`).
- Prior-work: new (sibling of C6; the peephole's arch-neutral register-effect model
  does not account for x86 implicit clobbers).

## Goal

- Each latent corner is either loudly rejected or correctly handled, so no future
  lowering can silently produce wrong machine code (or hang, C2) through it.

### Non-goals (must NOT change)

- Any currently-emitted instruction bytes for the reachable operand combinations
  (verified correct this pass).

## Blast Radius

Each item is a single encoder/select site (cited). No shared code; land per item.

## Fix Design / Phases

- [ ] Phase 1: add a byte-exact (or hang-avoidance) test per item using the
      triggering operand combo.
- [ ] Phase 2: apply the per-item guard/fix.
- [ ] Phase 3: full arch encode test suite + artifact gate green; no byte drift for
      reachable combos.

## Validation Plan

- Regression: per-item encoder tests.
- Full suite: `scripts/artifact-gate.sh` (execution-free codegen diff) + arch unit
  tests.
- Doc sync: none.

## Summary

Seven latent codegen-robustness corners; each is a localized guard turning silent
wrong-code (or a hang) into a loud failure. No active miscompile today; value is
hardening the backends before new lowerings land.

## Resolution

All eight items landed. Every one is a guard, not a behavior change: the artifact
gate reports **1163 goldens across 985 tests, 0 diffs**, which is the direct
evidence for the stated non-goal that no currently-emitted instruction bytes move.

**C1** (aarch64 sp/xzr conflation) — added `operand::is_stack_pointer` and
`operand::shifted_reg`, and routed the twelve shifted-register-form arms
(add/adds/sub/subs/and/orr/eor/mul/smulh/umulh/sdiv/udiv, plus mvn and cmp)
through the latter, which rejects an `sp`-spelled operand where register 31 means
XZR. `xzr` in those slots stays legal, since reading zero is what it means.
`cmp_imm` needed more than an operand swap: it selects its form from the
immediate's *magnitude*, and only the immediate form reads 31 as SP. The spelling
is now threaded into `emit_cmp_imm`, which accepts `sp` with a 12-bit immediate
and rejects it precisely when the wide fallback would silently compare against
zero. Tests: `sp_is_rejected_in_shifted_register_operand_slots`,
`cmp_imm_against_sp_is_rejected_only_when_the_immediate_forces_register_form`.

**C2** (unbounded chunk loop) — `MAX_ADD_SUB_CHUNKS` (8) and
`add_sub_chunk_count`, which saturates rather than counting out. Both
`sized_add_sub_imm` and the two emitters consult it, so `instruction_size` can no
longer spin for ~10^12 iterations ahead of any error. A real multi-chunk offset
still encodes. Test: `an_absurdly_wide_add_sub_immediate_is_rejected_rather_than_looped_over`.

**C3** (riscv64 pending-compare) — the invalidation now also matches `carry_out`
and `borrow_out` against the saved rhs, and clears `pending` outright on
`Call`/`CallIndirect`, which clobber the caller-saved set while naming no
destination at all. An invalidated pending followed by a branch reaches the
existing "standalone flag branch without a preceding compare" panic — loud,
versus the silently wrong branch it produced before. Four tests, including a
control case proving an undisturbed compare still fuses to `rv.br`.

**C4/C5** (riscv64 v128 silent defaults) — `unwrap_or(0)` replaced by
parse-or-panic in both places: the umov lane index is bounded to `<= 1`, matching
aarch64's loud rejection, and the high-lane offset is derived by a
`high_lane_offset` helper that refuses a non-numeric spelling on its own terms
rather than relying on the sibling op's `operand::immediate()` to catch it. Six
tests including both positive controls.

**C6** (x86_64 fixed-register aliasing) — `var_shift`/`var_shift_w` reject
`dst == rcx` (rcx is the architectural shift count, so it cannot also carry the
result); `msub` rejects `dst == rax` (subtracted from itself, yielding 0) and
`rhs == rax` (destroyed before the multiply, yielding `lhs*lhs`); `rbit` rejects
`dst ∈ {rax, rdx}` (its mask register and accumulator, both restored by the
trailing pops). Test: `fixed_register_aliasing_is_rejected_rather_than_miscompiled`.

`div_seq` was handled differently, deliberately. It already guards the two
reachable cases (an aliasing divisor, and a live rax dividend), and the report's
remaining ask — unconditional push/pop of rax/rdx — would cost every division in
the program to cover a case the register model forbids. What was wrong there was
the *reasoning*: a comment asserting "rax/rdx are non-allocatable, so clobbering
them is safe", which is the exact argument bug-125 refuted. That prose reliance is
now an enforced invariant,
`implicit_clobber_registers_are_never_allocatable`, which fails if rax, rcx or rdx
is ever added to `INT_ALLOCATABLE`. One test covers the reliance shared by
`div_seq`, `var_shift`, `msub` and `rbit`.

**C8** (peephole implicit clobbers) — `classify` takes an `is_x86` flag and gives
`Mul`/`SMulH`/`UMulH`/`SDiv`/`UDiv`/`MSub` their own arm, returning `Barrier` on
x86 and `DefDst` elsewhere. This removes the soundness reliance rather than
documenting it, while leaving aarch64/riscv64 forwarding untouched — the test
asserts both halves. `forward_stores_to_loads` learns the ISA the same way
`remove_fp_shuttles` does, from the active backend's arena base.

That last part is worth recording: the first attempt compared
`arena_base() == "r15"` inline and was caught by
`shared_lowering_names_no_physical_register`, the plan-34-D invariant that shared
code must never name a physical register. The test was right and the change was
wrong; the fix was to add `x86_64::regmodel::ARENA_BASE_REGISTER` and compare
against it, mirroring how the riscv64 side already did it.

Validation: full `cargo test` green, artifact gate 0 diffs, acceptance 1000/1000.
