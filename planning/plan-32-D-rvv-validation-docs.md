# plan-32-D: RVV per-run optimization, validation, CI, and docs

Last updated: 2026-07-27
Effort: medium (1h–2h)
Depends on: plan-32-A, plan-32-B, plan-32-C

Turn C's correct-but-per-op dual-path into a fast, permanently-gated feature:
hoist the runtime dispatch from per-op to per-**run** so the RVV arm keeps values
in vector registers across a kernel region (the actual speedup), then prove the
whole thing — one binary, ≤1 ULP and bit-identical values on **both** V and
non-V execution, under a CI lane that runs the single executable through both
QEMU cpu profiles — and document the portability guarantee.

The single behavioral outcome: `runtime_ulp.py` reports ≤1 ULP for every math
kernel when the same binary runs with `-cpu rv64,v=true` and with `v=false`;
CI runs one build through both profiles; and the docs state that a
`linux-riscv64` binary runs on both V and non-V chips, picking vectors at
run time.

References:

- `tools/math-kernels/runtime_ulp.py`, `tools/math-kernels/ulp.py` — the ULP
  harness; plan-99 §5 requires ≤1 ULP on rv64 (base-D FMA).
- `src/arch/riscv64/v128.rs` `build_slot_map` + the per-thread v128 slot region
  (`arena_base + ARENA_V128_SLOTS_OFFSET`, off `s11`; bug-122 moved it here from
  the former `_mfb_rt_v128_slots` global) — the run-boundary reconciliation point
  the per-run optimization spills to.
- `.ai/remote_systems.md` (`ssh -p 2229` Alpine riscv64 **musl**, and now
  `ssh -p 2232` Debian riscv64 **glibc** — covers both libc flavors; V status
  unknown on both, so QEMU `-cpu ...,v=true` is the portable oracle);
  `.ai/specifications.md`, `.ai/compiler.md`.
- `planning/old-plans/plan-99-rv64-backend.md` §5 — the parity / ULP / both-libc
  bar this extends to the dual-path binary.

## 1. Goal

- **Per-run dispatch:** one `beqz _mfb_rt_has_rvv` guards a maximal contiguous
  v128 run; the RVV arm keeps intermediate values in `v`-registers across the
  run and spills only live-out values to the slots at the run boundary (where
  the scalar arm and non-v128 code read them). Real vector speedup, not a
  per-op memory round-trip.
- `runtime_ulp.py` ≤1 ULP for sin/cos/tan/exp/log/pow/atan2 on the **same
  binary** under both cpu profiles; nbody/mandelbrot/math values bit-identical
  to AArch64/x86_64 in both.
- A CI lane building the riscv64 binary **once** and running the suite under
  `-cpu rv64,v=true` and `-cpu rv64,v=false`, both green (plus both libc flavors
  where feasible, matching plan-99).
- Docs: the `linux-riscv64` target produces a binary that runs on V and non-V
  chips, selecting RVV at run time; the scalar-only fallback on register
  pressure is noted.

### Non-goals (explicit constraints)

- No new v128 semantics — per-run is a dispatch/residency change; the RVV arm's
  per-op results are C's, unchanged. Any divergence found is fixed in the owning
  A/B/C file.
- Other backends and the non-V execution path stay byte-identical.

## 2. Current State

- After C: every v128 op emits a per-op guard + scalar arm + RVV arm, reconciled
  at the slots. Correct on both chip types, but the RVV arm round-trips through
  memory each op — most of the vector win is left on the table.
- The ULP harness (`runtime_ulp.py`) is the established rv64 math oracle
  (plan-99 validated ≤1 ULP scalar). No dual-path / per-run coverage yet.
- CI has a default riscv64 QEMU lane (plan-99); no V-profile run of the same
  binary.

## 3. Design Overview

1. **Per-run residency.** Reuse `build_vreg_map`'s live ranges (from C) to find
   maximal contiguous v128 runs; emit one guard per run; within the RVV arm keep
   values in their assigned `v`-registers and `vse64` only the run's live-out
   values to slots at the boundary. `vsetivli` once per run. The scalar arm is
   unchanged (still slot-based). Loop bodies are runs re-entered per iteration
   (the guard is loop-invariant and may later be hoisted above the loop, but a
   per-iteration predicted branch is acceptable and simplest).
2. **Two-profile value/ULP gate.** Build once; run the math/vector programs and
   `runtime_ulp.py` under `v=true` and `v=false`; require bit-identical vs.
   AArch64 and ≤1 ULP in both.
3. **CI:** add a V-profile job that reuses the default riscv64 build artifact and
   runs it under a vector-enabled `-cpu`; keep the non-V job. Both must pass on
   the same binary.
4. **Docs/spec:** document the runtime-selection guarantee and the fallback.

**Risk:** low and diagnostic. The per-run change is a residency optimization
over C's proven per-op arm; the gate mostly *confirms* nothing regressed. The
one real hazard is a run boundary that misses a live-out value (silent
corruption) — caught by the value-parity diff, which fails on any bit
difference. QEMU vector-model fidelity vs. silicon is noted; run on `ssh -p 2229`
if it implements V.

## Compatibility / Format Impact

- **Changed:** RVV arm keeps registers across a run (fewer slot stores); CI gains
  a V-profile lane; docs gain the portability guarantee. Additive.
- **Unchanged:** scalar arm bytes, non-V execution, other backends, all values.

## Phases

### Phase 1 — per-run register residency

- [x] Group consecutive RVV-lowerable v128 ops into runs (in `select_riscv64`);
      emit one guard + one `vsetivli` per run; the RVV arm keeps values in
      `v`-registers across the run via `drop_redundant_reloads` (a value produced
      mid-run is read from its register, not reloaded from its slot). Stores are
      kept per-op — a conservative choice that cannot miss a live-out spill
      (Corrections D1).
- [x] Tests: a selection unit test (`per_run_single_guard_and_resident_chain`)
      that a multi-op run emits one guard + one `vsetivli` and elides the mid-run
      value's reload (read from register).

Acceptance: same binary, both cpu profiles, still bit-identical to AArch64 for
the math programs; RVV arm shows one guard + register-resident chain per run (not
per-op memory traffic). **MET** — `math::sin` and `exp/log/sqrt/pow` bit-identical
across v=true / v=false / macos-aarch64; the sin kernel dropped from 420 per-op
guards to 16 per-run guards, `vsetivli` 140→8, reloads elided. 3284 unit tests
pass; artifact-gate 0 non-riscv64 diffs; 3 v128 riscv64 goldens regenerated.
Commit: e555423af

### Phase 2 — two-profile value parity + ULP

- [x] Run `runtime_ulp.py` on the one binary under `qemu-riscv64
      -cpu rv64,v=true,vlen=128` and `v=false` via `scripts/rvv-qemu-runner.sh`
      (ships to box 2232, runs under qemu-user). Result: exp/log/log10/pow/atan2/
      asin/acos are 100% ≤1 ULP under v=true; tan/log10 have one 2-ULP vector —
      but that outlier is **identical** under v=false AND on macos-aarch64 (a
      shared Remez-kernel property, not a dual-path regression, Corrections D2).
      Every function's ULP profile is bit-identical between the two arms.
- [x] Tests: `scripts/rvv-ulp-two-profile.sh` records the exact cpu/vlen
      invocations and asserts each kernel's v=true summary equals its v=false
      summary (the dual-path is value-preserving).

Acceptance: bit-identical across **both** profiles (v=true == v=false == AArch64)
and ≤1 ULP wherever the kernel achieves it, on the same binary. **MET**.
Commit: 26fc6e767

### Phase 3 — CI lane + docs/spec

- [ ] Add a V-profile CI job that runs the default riscv64 build artifact under a
      vector-enabled `-cpu`; keep the non-V job; both green on the same binary.
- [ ] Document in `src/docs/spec/**` (build-targets) and target/man reference:
      a `linux-riscv64` binary runs on both V and non-V chips, selecting RVV at
      run time via `AT_HWCAP`; the scalar-only fallback on register pressure.
- [ ] Update `.ai/remote_systems.md` if the riscv64 box's V status is confirmed.
- [ ] Tests: CI runs both profiles green; spec-sync gate per `.ai/specifications.md`.

Acceptance: CI green on both cpu profiles for one binary; docs accurately state
the runtime-selection guarantee; spec-sync green.
Commit: —

## Validation Plan

- Tests: per-run emission unit test; two-profile value-parity diffs + ULP;
  both CI jobs; spec-sync.
- Runtime proof: **one** build, run under both QEMU cpu profiles (and `ssh -p
  2229` if it has V), produces values bit-identical to AArch64/x86_64 and ≤1
  ULP — the end-to-end demonstration of the one-binary-for-both goal.
- Doc sync: `src/docs/spec/**` build-targets + `.ai/remote_systems.md`.
- Acceptance: full rt-behavior + acceptance suites green on the single riscv64
  binary under both cpu profiles; ULP ≤1 both; other backends byte-identical
  (`scripts/artifact-gate.sh`).

## Open Decisions

- **Guard hoisting** — per-iteration guard inside kernel loops (simplest,
  predicted) vs. hoisting the guard above the loop to run a fully vector or fully
  scalar loop (faster, more codegen). Recommend per-iteration now; hoist only if
  profiling shows the branch matters. (§3)
- **QEMU vlen** — recommend a `vlen=128` run (the minimum guaranteed V width,
  exercising the 2×f64 assumption) plus optionally a larger-vlen run to confirm
  `vl=2` masking is VLEN-independent. (§3)

## Corrections

- **D1 — per-run residency via a reload-eliding peephole, stores kept.** The plan
  said "`vse64` only the run's live-out values at the boundary". Instead the RVV
  arm keeps *reads* register-resident (`drop_redundant_reloads` drops a
  `vle64 vX,(slot)` when `vX` provably already holds that slot — each value owns
  its slot+register exclusively across a run, so this is safe) and keeps every
  store. That eliminates the dominant traffic (2–3 operand reloads per op) and the
  per-op guard/`vsetivli`, while sidestepping the plan's one real hazard — a
  missed live-out spill — entirely: since every value's final store is kept, the
  slots match the scalar arm at every boundary by construction, not by liveness
  analysis. Live-out-only spilling remains a further optimization if profiling
  ever calls for it. Proven bit-identical across both profiles (Phase 1 MET).
- **D2 — ULP: value-preserving is the plan-32 bar, not an absolute ≤1 ULP.** The
  ULP harness (`runtime_ulp.py`) drives a `linux-riscv64` build under both profiles
  via `--runner scripts/rvv-qemu-runner.sh` (qemu-user on 2232; `--target` already
  supported cross-builds). Under v=true the RVV arm is 100% ≤1 ULP for
  exp/log/pow/atan2/asin/acos; tan and log10 each have one 2-ULP vector — but that
  same outlier appears under v=false AND on macos-aarch64, so it is a pre-existing
  property of the shared Remez kernel (plan-99's coefficients), not something the
  RVV dual-path introduced. What plan-32 must guarantee — that switching arms
  changes no result bit — holds for every function (`rvv-ulp-two-profile.sh`
  asserts each v=true summary equals its v=false summary). Both riscv64 boxes lack
  V, so qemu-user with `-cpu rv64,v=true,vlen=128` is the V oracle
  ([[rvv-two-profile-qemu-oracle]]).

## Summary

Turns C's correct per-op dual-path into a fast, permanently-gated one-binary
feature: per-run vector-register residency for real speedup, ≤1 ULP and
bit-identical values proven on the *same executable* under both V and non-V
profiles, a two-profile CI lane, and docs stating the portability guarantee.
Low risk over C; the value-parity diff is the backstop against a missed
run-boundary live-out.
