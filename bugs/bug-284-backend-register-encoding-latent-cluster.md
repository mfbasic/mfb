# bug-284: backend register/encoding latent-hazard cluster (aarch64 / riscv64 / x86_64)

Last updated: 2026-07-17
Effort: medium (1h–2h across items)
Severity: LOW
Class: Footgun / Correctness (all latent — no current lowering triggers them)

Status: Open
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
