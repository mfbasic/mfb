# plan-32-C: dual-path v128 lowering (runtime scalar-or-RVV, one binary)

Last updated: 2026-07-08
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

- `src/arch/riscv64/v128.rs` — `is_v128` (`:58`, the op vocabulary),
  `build_slot_map` (`:99`, liveness/loop-extension analysis to reuse),
  `scalarize_v128` (`:219`, the scalar arm, reused verbatim), `is_vector_operand`
  (`:78`), the global slot region `_mfb_rt_v128_slots` (`:33`) that reconciles
  the two arms.
- `src/arch/riscv64/select.rs:314` (`build_slot_map` call), `:426` (v128 routing
  — where dispatch is emitted).
- `src/arch/riscv64/regmodel.rs:55` — FP model; the v-register reservation lives
  alongside.
- `src/target/shared/code/builder_simd_float_math.rs:312` — the kernels are
  **inlined** into user functions (per-list loops), which is *why* dispatch must
  be in-lowering, not IFUNC.
- RVV mask model (spec §5.3/§15): compares write a 1-bit-per-element mask
  register, not the NEON all-ones/all-zeros lane mask — the central impedance
  mismatch.

## 1. Goal

- Every v128 op (or maximal contiguous v128 *run* — see Phases) lowers to a
  runtime branch:
  `lb has, _mfb_rt_has_rvv; beqz has → scalar arm; else → RVV arm; converge`.
- **Scalar arm:** the existing `scalarize_v128` output, unchanged — operands read
  from / results written to the `_mfb_rt_v128_slots` region.
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

- `scalarize_v128` (`v128.rs:219`) already routes every v128 value through the
  global `_mfb_rt_v128_slots` region — operands loaded from slots, results
  stored back. **This is the reconciliation point that makes dual-path cheap:**
  an RVV arm that also reads/writes those slots meets the scalar arm at the slot
  automatically, no merge logic.
- `build_slot_map` (`:99`) computes per-value live ranges with loop-body
  extension to a fixpoint (`:146`) so loop-carried values never share storage —
  exactly the analysis physical-vreg assignment needs; only the assigned
  resource (slot offset → `v`-register number) differs.
- The kernels emit v128 ops on physical `v0`–`v31` / FP virtuals `%fN`
  (`is_vector_operand`, `:78`), **inlined** into user functions
  (`builder_simd_float_math.rs:312`) — so there is no per-kernel symbol to
  multiversion; dispatch must be per-op/per-run inside selection.
- Selection routes v128 ops at `select.rs:426`. Today it always calls
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

- [ ] Factor the liveness+loop-extension core of `build_slot_map` into a shared
      helper; add `build_vreg_map(instructions) -> Option<HashMap<String,u8>>`
      assigning `v1`–`v31` (reserve `v0`+scratch), `None` on overflow.
- [ ] Tests: reuse across disjoint ranges packs into few v-regs; loop-carried
      values stay distinct; overflow returns `None` (mirror the slot-map tests).

Acceptance: `build_vreg_map` reproduces slot-map liveness as register numbers;
overflow falls back. Output still scalar; riscv64 suite green.
Commit: —

### Phase 2 — dual-path dispatch + per-op RVV lowering (non-mask ops)

Wire the guard + RVV arm for the arithmetic/convert/bitwise/mem ops.

- [ ] In the v128 pass, emit the `lb/beqz … j` guard around each v128 op; scalar
      arm = `scalarize_v128` (unchanged); RVV arm = `vsetivli` + B mnemonics for
      `FAddV/FSubV/FMulV/FDivV`, `FMlaV/FMlsV`, `FAbsV/FNegV/FSqrtV`, `FRint*`,
      `FCvtzsV/FCvtasV/ScvtfV`, `AddV/SubV/NegV`, `AndV/OrrV/EorV`,
      `ShlV/SshrV/UshrV`, `DupVFromX/UmovXFromV`, `LdrQ/StrQ`, reading/writing the
      slots.
- [ ] Tests: selection unit tests that each op emits guard + both arms; the RVV
      arm's mnemonic/operand sequence is as expected.

Acceptance: **one binary**, run under `qemu-riscv64 -cpu rv64,v=true` and
`v=false`, produces values **bit-identical to the AArch64 golden in both modes**
for nbody/mandelbrot and `math::exp/log/pow` (non-compare-heavy).
Commit: —

### Phase 3 — the mask bridge (compares + bit-select) + min/max

The crux; lands last, behind value-parity tests.

- [ ] RVV arm for `FCmGtV/FCmGeV/FCmEqV`, `FCm*ZeroV`, `CmGtV/CmGeV/CmEqV` via
      `vmf*`/`vms*`→`v0` + `vmv.v.i`/`vmerge.vim` all-ones lane materialization;
      `BslV/BitV` as bitwise selects over those lanes.
- [ ] `FMinV/FMaxV` on the RVV arm in whichever form matches NEON NaN/±0 (direct
      `vfmin/vfmax` or a reproduced sequence — decided by test).
- [ ] Tests: the compare→lane-mask sequence and `BslV`; value-parity of
      compare-heavy kernel output vs. the AArch64 golden, both cpu modes.

Acceptance: the same binary runs `math::sin/cos/atan2` (compare/quadrant-heavy)
bit-identical to the AArch64 golden under both `v=true` and `v=false`; the full
v128 op set is covered.
Commit: —

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
  allocatable pool `v1`–`v30`; revisit if a lowering needs two live temps. (§3)

## Summary

The heart of the one-binary-for-both feature: both a scalar and a native-RVV arm
in every riscv64 binary, runtime-selected by A's flag and reconciled through the
existing slot region, so no kernel needs de-inlining and the non-V path is
exactly today's proven code. All real risk is the NEON→RVV mask bridge and three
semantics-subtle ops, each pinned by bit-identical cross-arm/cross-backend tests.
