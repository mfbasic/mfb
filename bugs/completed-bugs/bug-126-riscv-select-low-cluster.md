# bug-126 — riscv64 select LOW cluster: v128 rounding ties/round-trip, bare-compare rhs re-read, shared-branch fused re-emit

**Status:** OPEN. Filed 2026-07-11 (goal-02 review, G6). Three
correctness/footgun findings in riscv64 instruction selection & v128, batched
per goal-02.

## 1. v128 rounding: ties-to-even done as ties-away; f64→i64 round-trip breaks for |x| ≥ 2^63

`src/arch/riscv64/v128.rs:339-353` (`FRintmV | FRintpV | FRintzV | FRintaV |
FRintnV` arm). (a) `FRintnV` (`v128.fround_even`, contract "nearest ties to
EVEN" per mir.rs lane-semantics) is lowered with `fcvtas_x_from_d` = RMM
ties-AWAY — wrong at every .5 tie (2.5 → 3 instead of 2). (b) All five `frint*`
round-trip through signed i64 (`fcvt` then `scvtf`), so a lane with |x| ≥ 2^63
(or NaN/Inf) saturates and reconstructs wrong, where AArch64 `frint*` returns
integral/non-finite inputs unchanged. Severity LOW: the round kernels'
Inf/NaN pre-check catches exp==2047 lanes, and transcendental kernels pass only
small reduced values. Trigger: riscv v128 `fround_even` at an exact tie, or
`math::floor/ceil/round(List OF Float)` with values ≥ 9.22e18. Fix: use the
round-to-nearest-even conversion mode and skip the round-trip for
already-integral lanes.

## 2. Bare-compare `gp` mechanism re-reads the rhs register at the branch

`src/arch/riscv64/select.rs:299-392` (`FlagRhs::Reg` + standalone-branch
expansion; `pending` never invalidated by intervening defs or labels). A
non-fused `cmp` saves only its LHS into `gp`; the RHS register is re-read when
the non-adjacent flag branch expands. On AArch64 flags latch at the compare; on
riscv an intervening instruction that redefines the rhs register (or a label
entered from a path that never ran the compare) yields a different branch
decision. The standalone-branch set also omits `b.mi/b.vs/b.vc` (they fall to
the encoder's loud unsupported error). No current hand-written helper has a
`cmp; <redefine rhs>; b.cc` shape; latent. Fix: snapshot rhs too, or invalidate
`pending` on any def of the saved rhs / on label entry.

## 3. Shared-branch re-emission re-executes fused Adds/Subs side effects

`src/arch/riscv64/select.rs:419-424` (`let _ = is_shared(...);
out.extend(expand_fused(...))`). A `share`-marked fused op re-emits the entire
setter expansion. For Cmp/CmpImm/FCmp this is idempotent, but for a shared
branch on an `Adds`/`Subs` fusion the expansion re-executes `dst = lhs + rhs` —
if `dst` aliases `lhs` (the common `adds x,x,y`) the second emission
double-adds; a shared cond other than `b.vs/b.vc` also panics in
`overflow_branch_cond`. No builder emits two flag branches after one `adds`
today; latent. Fix: cache the fused result instead of re-expanding for the
shared consumer.

---
## Resolution (2026-07-11)
- 126.1 (FRintnV ties-to-even + integral/non-finite guard) — FIXED, verified rv64.
- 126.2 (stale bare-compare rhs: pending-clear on Label / rhs redef) — FIXED.
- 126.3 (non-idempotent shared fused arithmetic setter re-expansion) — FIXED with a
  loud guard: a shared `adds`/`subs` (which writes dst) now fails the build instead
  of double-applying its add/sub for the second flag-less-RISC-V branch. Compares
  (idempotent) still re-emit. Byte-identical (0 golden changes) — no builder emits a
  shared arithmetic setter today. Cluster closed.
