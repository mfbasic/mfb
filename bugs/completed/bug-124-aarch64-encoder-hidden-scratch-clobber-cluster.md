# bug-124 — AArch64 encoder LOW cluster: hidden x15–x17 scratch clobber, d8–d15 v128 high-lane loss, unchecked branch range

**Status:** OPEN. Filed 2026-07-11 (goal-02 review, G6). Three latent
correctness hazards in the AArch64 encoder where the allocator has no model of
an encoder/ABI side effect. All currently unreachable by construction; batched
per goal-02.

## 1. Immediate/offset fallbacks clobber x15–x17, which are allocator-assignable

`src/arch/aarch64/encode/operand.rs:99-104` (`scratch_excluding` →
x17/x16/x15); users: emitter.rs:862-869 (`emit_cmp_imm`, rhs > 4095),
:955-1057 (all `emit_ldr_*/str_*` out-of-range fallbacks), :411-432
(`emit_ldr_q/str_q` > 65520). vs regmodel.rs:43-46 (INT_ALLOCATABLE includes
x15/x16/x17). When a cmp_imm immediate exceeds imm12 or a load/store offset
exceeds scaled-imm12, the encoder silently materializes through x17/x16/x15 —
registers in the linear-scan pool with no allocator model of the hidden
clobber. A vreg colored to x15–x17 whose interval spans such an instruction is
destroyed silently. Unreachable today per bug-09's audit (immediates ≤ 2048,
frames < 32 KB); becomes a miscompile the day any builder compares against a
constant > 4095 or a spill area crosses 32 KB with ≥8 int vregs live. Fix:
reserve the encoder scratch outside INT_ALLOCATABLE, or model the clobber.

## 2. 128-bit vector vreg live across a call may color into d8–d15 (high lane not callee-saved)

`src/arch/aarch64/regmodel.rs:64-75` (FP_ALLOCATABLE ends d8–d15; FP_CALLER_SAVED
excludes them) + regalloc/analysis.rs:69 (`CALLER_SAVED_FP = 0xffff_00ff`) +
codegen_utils.rs:566-580 (callee-saved FP frame save is 64-bit `str_d`). The FP
class carries both scalar f64 and 128-bit v128 in one `%fN` namespace. A value
live across a `bl` deterministically lands in d8–d15 (only bank outside the
call-clobber mask); AAPCS64 preserves only the LOW 64 bits of v8–v15, and this
compiler saves callee-saved FP with 64-bit `str_d`, so a v128 crossing a call
loses its top lane. Sound today only because the vector-carrier builders
materialize vectors to memory at every call boundary (plan-01-vector's choke
point); nothing asserts it. Fix: spill v128 callee-saved as 16-byte, or exclude
d8–d15 from v128 coloring across calls.

## 3. Branch patching has no displacement range check (silent wrap)

`src/arch/aarch64/encode/sizing.rs:138-146` (`branch_imm26`/`branch_imm19` mask
without validation); emitter.rs:1147-1174 (`patch_labels` applies unchecked).
`b.cc` masked to 19 bits (±1 MiB), `b` to 26 bits, no overflow check — a
function whose conditional branch spans > 1 MiB encodes a wrapped wrong target
silently. The riscv encoder checks its `jal` reach (emitter.rs:676-681); this
does not. Latent (functions far below the limit). Fix: range-check and hard-error
on overflow, like riscv.
