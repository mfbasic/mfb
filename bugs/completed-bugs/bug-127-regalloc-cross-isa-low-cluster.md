# bug-127 — regalloc & cross-ISA LOW cluster: dup-label guard gap, eviction panic invariant, %scratch token cross-ISA index

**Status:** OPEN. Filed 2026-07-11 (goal-02 review, G6). Three
footgun/latent-correctness findings in the register allocator and cross-ISA
scratch model, batched per goal-02.

## 1. Duplicate-label guard exists only in the x86 encoder

`src/arch/aarch64/encode/mod.rs:117-127` and `src/arch/riscv64/encode/mod.rs:90-100`
(plain `labels.insert(...)`) vs `src/arch/x86_64/encode/mod.rs:99-108` (bug-15
duplicate check with hard error). bug-15's hardening (reject duplicate label
names per function) was added only to x86. On aarch64/riscv a duplicate label
silently rebinds every branch to the last definition — the identical
silent-misbranch failure mode if any future lowering synthesizes colliding
names. No aarch64/riscv path synthesizes per-site labels today; regression-guard
gap only. Fix: port the duplicate-label check to the other two encoders.

## 2. Linear-scan eviction panics when an instruction has more spilled operands than registers

`src/target/shared/code/regalloc/linear_scan.rs:249-254` (`.expect("register
allocator: instruction has more operands than registers")`). The eviction
fallback asserts "there are always more registers than operands". On x86 the
integer pool is 4 (see bug-125 item 2), and `add_carry`/`sub_borrow` carry 5
register operands (dst, carry_out, lhs, rhs, carry_in). If all five are spilled
vregs at one instruction, the 5th scratch lookup finds every allocatable
register reserved and the compiler panics. Not constructible from today's
emission sites (RNG helpers have ~4 concurrent vregs); latent, but the invariant
is false as stated. Fix: reserve a guaranteed scratch or handle the
more-operands-than-registers case without panicking.

## 3. `%scratch` token occupancy indices are AArch64-specific while realizations are per-ISA

`src/target/shared/code/regalloc/analysis.rs:156-198` (`int_physical_index`:
`%scratchN` → AArch64 index 9+n/10+n) vs `src/arch/riscv64/select.rs:474-489`
(`map_scratch_register`: x9→t3, x14→s3, …) and `src/arch/x86_64/select.rs:36-55`
(x9→rbx, …). If an allocator-managed stream ever carries an int `%scratchN`
token, the occupancy model marks the AArch64-index register busy (e.g.
`%scratch5` → index 14 = riscv `a4`) while the token realizes to a different
register (riscv `s3`, which IS allocatable) → post-selection double occupancy.
Safe today by construction: int SCRATCH tokens appear only in machine-floor code
that never passes through `regalloc::allocate` (entry stub, thread trampoline
are hand-framed). First helper that mixes `abi::SCRATCH[i]` with vregs on
riscv/x86 miscompiles silently. Fix: make the occupancy index per-ISA, matching
the realization map.

## Also noted (dup, not re-filed)

`src/arch/aarch64/ops.rs` carries x86/rv-only CodeOp variants misfiled under
aarch64 — **dup of bug-82** (open).

---
## Resolution (2026-07-11)
- 127.1 (duplicate-label guard on aarch64 + riscv) — FIXED.
- 127.2 (eviction "more operands than registers" ICE) — FIXED (surfaced as a
  RunResult error / centralized panic with an actionable message).
- 127.3 (per-ISA %scratch/%sysnr occupancy) — FIXED: split the AArch64-indexed
  scratch arms out of int_physical_index; x86/riscv use int_physical_index_non_aarch64
  (concrete-register lookup only), since those tokens realize to different per-ISA
  registers and are lowered to concrete names before regalloc. Byte-identical (0
  golden changes); guaranteed-unreachable per plan-34-D, so purely defensive
  correctness. Cluster closed.
