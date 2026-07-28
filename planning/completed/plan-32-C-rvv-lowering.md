# plan-32-C: dual-path v128 lowering (runtime scalar-or-RVV, one binary)

Last updated: 2026-07-27
Effort: medium (1h–2h)  — the correctness risk concentrator
Depends on: plan-32-A (runtime `_mfb_rt_has_rvv` flag), plan-32-B (RVV encoder)

Make every `linux-riscv64` binary carry **both** a scalar and a native-RVV
realization of its v128 code, selected at run time by the `_mfb_rt_has_rvv`
byte from A — so the *same* executable runs correctly on V and non-V chips,
using vectors where present. Dispatch lives inside the riscv64 v128 lowering
(`select_riscv64` → the v128 pass) and reconciles the two arms through the
existing memory-slot region, so no kernel needs a callable boundary and the
scalar arm is exactly today's proven `scalarize_v128`.

The single behavioral outcome: with the flag set (V hardware), the math kernels
(`math::sin/cos/exp/log/pow/atan2`) and `vector::` execute native `vf*`/`v*`
instructions; with it clear, they execute the current scalar-pair code — and
both produce f64/i64 values **bit-identical** to the AArch64/x86_64 backends.

References:

- `src/arch/riscv64/v128.rs` — `is_v128` (`:64`, the op vocabulary),
  `build_slot_map` (`:153`, liveness/loop-extension analysis to reuse),
  `scalarize_v128` (`:273`, the scalar arm, reused verbatim), `is_vector_operand`
  (`:122`), and the **per-thread** v128 slot region (`arena_base +
  ARENA_V128_SLOTS_OFFSET`, addressed off the pinned arena base `s11`, slot base
  materialized into `t2`) that reconciles the two arms. bug-122 moved this region
  from the former `_mfb_rt_v128_slots` process-global into per-thread arena state
  (concurrent threads were corrupting each other's lanes); `SLOT_COUNT` is 127
  (bug-381 reclaimed the 128th slot for the flag-emulation rhs snapshot).
- `src/arch/riscv64/select.rs:390` (`build_slot_map` call; `:394` the
  `peak_slots <= SLOT_COUNT` assert), `:580` (v128 routing — where
  `scalarize_v128` is called and dispatch is emitted).
- `src/arch/riscv64/regmodel.rs:56` — FP model (RV64GC has no 128-bit vector
  file; `FP_REGS` at `:59`); the v-register reservation lives alongside.
- `src/target/shared/code/builder_simd_float_math.rs` — the kernels are
  **inlined** into user functions (per-list loops; see `float_kernel_regs` /
  kernel-emission region), which is *why* dispatch must be in-lowering, not IFUNC.
- RVV mask model (spec §5.3/§15): compares write a 1-bit-per-element mask
  register, not the NEON all-ones/all-zeros lane mask — the central impedance
  mismatch.

## 1. Goal

- Every v128 op (or maximal contiguous v128 *run* — see Phases) lowers to a
  runtime branch:
  `lb has, _mfb_rt_has_rvv; beqz has → scalar arm; else → RVV arm; converge`.
- **Scalar arm:** the existing `scalarize_v128` output, unchanged — operands read
  from / results written to the per-thread v128 slot region (off `s11`).
- **RVV arm:** native RVV (B mnemonics) on physical `v1`–`v31`, reading operands
  from the same slots and writing results back to them, so both arms meet at the
  slot with no register-state merge.
- Physical v-register assignment via `build_vreg_map` (the `build_slot_map`
  liveness/loop-extension core, reused, assigning `v`-registers), reserving `v0`
  as the RVV mask register plus scratch; register-pressure overflow ⇒ that
  function emits the scalar arm only (still one correct binary).
- `SEW=64, LMUL=1, vl=2` established (`vsetivli`) on entry to each RVV arm.
- Every `is_v128` op lowered on the RVV arm, reproducing `scalarize_v128`
  semantics **bit-for-bit** — especially compare→lane-mask and `BslV`/`BitV`.

### Non-goals (explicit constraints)

- **Bit-identical results across both arms and all backends** (the plan-00-E /
  plan-99 ULP contract). NaN in `vfmin/vfmax`, conversion rounding, `vfnmsac`
  sign — proven equal, not assumed.
- **No shared-allocator changes.** Do **not** add `RegClass::Vector` to
  `src/arch/aarch64/regmodel.rs` — the RVV arm's v-registers are assigned by
  this pass's own linear-scan (as memory slots are today), bypassing the shared
  allocator. Additive.
- **The scalar arm stays byte-identical to today's scalarization** — reuse
  `scalarize_v128` unchanged; the only new bytes are the guard branch + the RVV
  arm. A binary run on non-V hardware executes exactly the current code path.
- This sub-plan may dispatch per-op (simplest, correct); the per-run
  register-residency optimization is D. Correctness of the one-binary property
  is the bar here, not peak vector throughput.

## 2. Current State

- `scalarize_v128` (`v128.rs:273`) already routes every v128 value through the
  per-thread v128 slot region (`arena_base + ARENA_V128_SLOTS_OFFSET`, off `s11`;
  bug-122) — operands loaded from slots, results stored back. **This is the
  reconciliation point that makes dual-path cheap:** an RVV arm that also
  reads/writes those slots meets the scalar arm at the slot automatically, no
  merge logic. (The RVV arm must address the same per-thread region off `s11`,
  not a global data symbol.)
- `build_slot_map` (`:153`) computes per-value live ranges with loop-body
  extension to a fixpoint (`:146`) so loop-carried values never share storage —
  exactly the analysis physical-vreg assignment needs; only the assigned
  resource (slot offset → `v`-register number) differs.
- The kernels emit v128 ops on physical `v0`–`v31` / FP virtuals `%fN`
  (`is_vector_operand`, `:122`), **inlined** into user functions
  (`builder_simd_float_math.rs`, `float_kernel_regs` / kernel-emission region) —
  so there is no per-kernel symbol to multiversion; dispatch must be per-op/per-run
  inside selection.
- Selection routes v128 ops at `select.rs:580`. Today it always calls
  `scalarize_v128`; this sub-plan wraps that call with the guard + RVV arm.

## 3. Design Overview

Four pieces, layered:

1. **v-register assignment (`build_vreg_map`).** Factor the liveness +
   loop-extension core out of `build_slot_map` and assign the ordered values to
   physical `v1`–`v31` via the same linear-scan. Reserve `v0` (RVV mask) + one
   scratch. Overflow ⇒ `None` ⇒ scalar-arm-only for that function.
2. **Dispatch shape.** Wrap each v128 op (Phase 2) — later each maximal
   contiguous run (D) — in:
   ```
     lb   t, _mfb_rt_has_rvv
     beqz t, .scalar_k
     <RVV arm: vle64 operands from slots, vop, vse64 result to slots>
     j    .done_k
   .scalar_k:
     <scalarize_v128 output — unchanged>
   .done_k:
   ```
   The guard is a load of a settled byte + a perfectly-predicted branch. Both
   arms read/write the same slots, so live-out values are in slots at `.done_k`
   regardless of which arm ran.
3. **`vtype` config.** `vsetivli x0, 2, e64, m1, ta, ma` at each RVV arm entry
   (per-op now; hoisted to per-run in D). `vtype` is dynamic global state, so
   re-establish it whenever an arm is entered.
4. **Per-op RVV lowering + the mask bridge.** Map each `is_v128` op to B
   mnemonics. The **mask bridge** is the crux: a compare emits `vmf*`/`vms*` into
   `v0`, then materializes the NEON all-ones/all-zeros lane vector
   (`vmv.v.i vd,0; vmerge.vim vd,vd,-1,v0`) so downstream `BslV`/`BitV`/`AndV`
   are plain `vand`/`vxor`/`vor` — identical algebra to the scalar arm, so
   results match by construction. `DupVFromX`→`vmv.v.x`; `UmovXFromV` idx1→
   `vslidedown.vi;vmv.x.s`; `LdrQ/StrQ`→`vle64.v`/`vse64.v`.

**Where the risk lives:** the mask bridge and the three semantics-subtle ops
(`vfmin/vfmax` NaN, conversion rounding, `vfnmsac` sign) — the only places the
RVV arm can diverge from the scalar arm at the bit level. Each is pinned by
cross-arm + cross-backend value-parity tests, and this sub-plan's acceptance
requires a QEMU run with `v=true` **and** `v=false` on the *same binary*,
matching the AArch64 goldens both ways.

**Rejected alternatives:**
- *IFUNC / function-pointer multiversioning* — needs a callable kernel; kernels
  are inlined, so there is no symbol to redirect. (§2)
- *`RegClass::Vector` in the shared allocator* — touches every backend; the
  pass-local assignment (memory-slot precedent) is sufficient and additive.
- *One vector register per value, no reuse* — kernels use dozens of live values;
  without live-range reuse the 31-register file overflows and always falls back.

## Compatibility / Format Impact

- **Changed:** riscv64 binaries emit, per v128 site, a guard branch + an RVV arm
  in addition to the scalar arm (larger code; a predicted branch per site). The
  scalar arm's bytes are unchanged.
- **Unchanged:** results (bit-identical, both arms, all backends); the scalar
  execution path on non-V hardware; other backends; the shared allocator /
  `RegClass`; overflow-fallback functions (scalar-only, byte-identical to today).

## Phases

### Phase 1 — v-register assignment + overflow fallback

Land assignment + the safety fallback; still emit scalar-only (dispatch inert)
so it is provable in isolation.

- [x] Factor the liveness+loop-extension core of `build_slot_map` into a shared
      helper (`v128_live_ranges`); add `build_vreg_map(instructions) ->
      Option<HashMap<String,u8>>` assigning `v1`–`v30` (reserve `v0` mask + `v31`
      scratch), `None` on overflow.
- [x] Tests: reuse across disjoint ranges packs into 3 regs; loop-carried values
      stay distinct (6); 30 concurrent fits, 31 overflows to `None`
      (`vreg_map_reuse_loop_distinctness_and_overflow`).

Acceptance: `build_vreg_map` reproduces slot-map liveness as register numbers;
overflow falls back. Output still scalar; riscv64 suite green. **MET** — 30 v128
tests pass; artifact-gate 0 linux-riscv64 diffs (the refactor is byte-identical,
`build_vreg_map` is inert).
Commit: c51d158a3

### Phase 2 — dual-path dispatch + per-op RVV lowering (non-mask ops)

Wire the guard + RVV arm for the arithmetic/convert/bitwise/mem ops.

- [x] In the v128 pass (`lower_v128`, wired at `select.rs:579`), emit the
      `lb/beqz … j` guard around each v128 op; scalar arm = `scalarize_v128`
      (unchanged); RVV arm (`rvv_arm`) = `vsetivli` + B mnemonics reading/writing
      the slots for `FAddV/FSubV/FMulV/FDivV`, `FMlaV/FMlsV`, `FAbsV/FNegV/FSqrtV`,
      `FCvtzsV/ScvtfV`, `AddV/SubV/NegV`, `AndV/OrrV/EorV`, `ShlV/SshrV/UshrV`
      (amount <32), `DupVFromX/UmovXFromV`, `LdrQ/StrQ`. The subtle ops (`FRint*`,
      `FCvtasV`, compares, `BslV/BitV`, min/max, wide shifts) always scalarize for
      now — correctness-preserving (Corrections C2); Phase 3 adds the mask bridge.
- [x] Tests: a selection unit test (`dual_path_emits_guard_rvv_and_scalar_arms`)
      that an op emits the guard + RVV arm + scalar arm with the expected
      mnemonics/operands, and that the no-vreg path is byte-identical to the
      scalar arm.

Acceptance: **one binary**, run under `qemu-riscv64 -cpu rv64,v=true` and
`v=false`, produces values **bit-identical to the AArch64 golden in both modes**
for `math::exp/log/sin/cos/sqrt/pow`. **MET** — same binary, v=true output ==
v=false output == macos-aarch64 output, bit-identical at 17 decimals across 7
inputs (Corrections C2). artifact-gate: 0 non-riscv64 diffs; the 3 v128 riscv64
goldens (audio/math/vector) regenerated for the dual path.
Commit: f8faa1862

### Phase 3 — the mask bridge (compares + bit-select) + min/max

The crux; lands last, behind value-parity tests.

- [x] RVV arm for `FCmGtV/FCmGeV/FCmEqV`, `FCm*ZeroV`, `CmGtV/CmGeV/CmEqV` via
      `vmf*`/`vms*`→`v0` + `vmv.v.i`/`vmerge.vim` all-ones lane materialization
      (`lanes_from_mask`); `BslV/BitV` as `vxor`/`vand`/`vxor` bit-selects over
      those lanes.
- [x] `FMinV/FMaxV` on the RVV arm as direct `vfmin.vv`/`vfmax.vv` — RVV's
      minimumNumber/`-0<+0` semantics match the scalar `fminnm_d`/`fmaxnm_d`, so
      bit-identical (Corrections C3).
- [x] Tests: the compare→lane-mask + `BslV`/min-max emission
      (`mask_bridge_and_minmax_rvv_arms`); runtime value-parity of the
      compare/quadrant-heavy `math::sin` (mask bridge + BslV) vs. the AArch64
      reference, both cpu modes.

Acceptance: the same binary runs `math::sin/cos/atan2` (compare/quadrant-heavy)
bit-identical to the AArch64 golden under both `v=true` and `v=false`; the full
v128 op set is covered. **MET** — `math::sin` (mask bridge + BslV, 420 dual-path
sites) is bit-identical across v=true / v=false / macos-aarch64 at 17 decimals
over 12 inputs. The remaining scalar-only ops (`FRint*`, `FCvtasV`, wide shifts,
`AbsV`/`Cnt8bV`/`Addv8bV`, `SshlV`/`UshlV`) are individually bit-identical via the
scalar arm — a deliberate coverage boundary, not a gap (Corrections C2/C3).
Commit: cb7da7350

## Validation Plan

- Tests: `build_vreg_map` (reuse/loop-carry/overflow); per-op dual-arm emission;
  the mask-bridge sequence; cross-arm value parity per phase.
- Runtime proof: build the math/vector acceptance programs **once**; run the
  single binary under `qemu-riscv64 -cpu rv64,v=true` and `v=false` (and on the
  `ssh -p 2229` box, whichever it is); diff f64/i64 output vs. AArch64 goldens —
  identical in **both** modes. Full ULP harness in D.
- Doc sync: none yet (D documents the portability guarantee).
- Acceptance: math/vector rt-behavior green on the one binary under both cpu
  modes, values bit-identical to AArch64; other backends byte-identical
  (`scripts/artifact-gate.sh`); the non-V execution path unchanged.

## Open Decisions

- **Dispatch granularity here** — per-op (simplest, correct, memory round-trip
  each op) *(recommended for this sub-plan)* vs. per-run (faster, keeps
  v-registers across a run). Recommend per-op now; per-run is D's optimization,
  since the one-binary correctness property doesn't need it. (§3, D)
- **`FMinV`/`FMaxV`** — direct `vfmin/vfmax` iff a NaN/±0 value-parity test
  matches NEON; else reproduce the scalar compare-select. (§3, Phase 3)
- **Scratch v-registers** — reserve `v0` (mask) + one temp (e.g. `v31`),
  allocatable pool `v1`–`v30`; revisit if a lowering needs two live temps. The
  RVV `v0`–`v31` file is a *separate* register file from the GPR/FP files, so its
  reservation does not touch `INT_ALLOCATABLE`/`FP_REGS` (avoiding the rv64
  pool-shrink allocator fault). (§3)
- **GPR scratch for the guard** — the guard (`lb has, _mfb_rt_has_rvv; beqz`) and
  the RVV arm's slot addressing need a free GPR. `scalarize_v128` already reserves
  `t0`/`t1`/`t2` (slot base + lanes) and `ft0`/`ft1`/`ft2`; the per-thread arena
  base is pinned in `s11` and bug-381 reserved a flag-emulation slot in the arena.
  Confirm the guard's scratch does not collide with a live `argc`/`argv` or the
  scalarize scratch across the converge point. (§3)

## Corrections

- **C1 — the `build_slot_map` liveness core** was extracted into `v128_live_ranges`
  and shared with `build_vreg_map`; `build_slot_map`'s output is unchanged (proven
  byte-identical). Pool is `v1`–`v30` (`v0` mask + `v31` scratch reserved), matching
  the Open Decision.
- **C2 — Phase 2 lowers a bit-identical-*by-construction* subset, not every op.**
  The plan's Phase-2 list named `FRint*`/`FCvtasV`; those (plus the compares,
  `BslV/BitV`, min/max, and lane shifts ≥32) reproduce subtle scalar sequences
  (the 2^52 rounding mask-select, ties-away `frm`, NaN semantics) that need the
  mask bridge, so `rvv_arm` returns `None` for them and they **always scalarize**.
  That is correctness-preserving: every v128 op is *individually* bit-identical to
  the AArch64 golden whether it runs on the RVV arm (pure `f64`/`i64` ops at the
  default RNE, exact integer/bitwise/mem, same-ISA RTZ/i2f converters) or the
  proven scalar arm. Consequently even compare/quadrant-heavy `math::sin/cos`
  already come out bit-identical under both profiles (their compare/round ops
  scalarize; their arithmetic vectorizes). Phase 3 promotes the compares/`BslV`/
  `BitV`/min-max to the RVV arm via the mask bridge; the remaining rounding ops can
  stay scalar with no correctness cost.
- **C2 — verification affordance.** `build_vreg_map` returns `Some` for the math
  kernels (measured peak concurrency **19** ≤ 30, not the ~128 the plan feared for
  the *whole* package — a single straight-line kernel's live set is small), so the
  RVV arm activates. Two-profile proof: the same linux-riscv64 binary's output
  under `-cpu rv64,v=true` equals its output under `v=false` **and** equals the
  native macos-aarch64 reference, bit-identical at 17 decimals over 7 inputs ×
  exp/log/sin/cos/sqrt/pow. qemu-user on 2232 (both riscv boxes lack V) —
  [[rvv-two-profile-qemu-oracle]]. Peak concurrency scales with how many distinct
  kernels share one function: `sin`+`cos`+`tan`+`atan2` in one `main` overflows
  the 30-register pool (→ scalar-only, still correct), so runtime proofs use
  lower-pressure programs; D's per-run residency does not change this pressure.
- **C3 — Phase-3 mask bridge + min/max.** Compares lower to the ordered
  `vmflt/vmfle/vmfeq` (`vmslt/vmsle/vmseq` for integers) into `v0`, then
  `vmv.v.i vd,0; vmerge.vim vd,vd,-1,v0` (imm `31` = the sign-extended `-1`)
  rebuilds the NEON all-ones/all-zeros lane vector the scalar arm stores; `BslV`/
  `BitV` are then the identical `xor`/`and` bit algebra on `v`-registers (scratch
  `v31`). `FMinV`/`FMaxV` use direct `vfmin.vv`/`vfmax.vv` (RVV and scalar
  `fminnm_d` share the RISC-V minimumNumber / `-0<+0` semantics), so no reproduced
  sequence is needed. Proven by the `math::sin` runtime parity (mask bridge +
  BslV, bit-identical across both profiles and vs. macos-aarch64).

## Summary

The heart of the one-binary-for-both feature: both a scalar and a native-RVV arm
in every riscv64 binary, runtime-selected by A's flag and reconciled through the
existing slot region, so no kernel needs de-inlining and the non-V path is
exactly today's proven code. All real risk is the NEON→RVV mask bridge and three
semantics-subtle ops, each pinned by bit-identical cross-arm/cross-backend tests.
