# plan-86-H — vector op-inlining

Sub-plan **H** of [plan-86](plan-86-benchmark-perf.md). Open.

**Covers (3 P2):** vector int (55.7), math (30.9), float (20.9). Also lifts `vector fixed` (mfb-only).

## Root cause
`vector_op_inlinable` (`builder_vector_inline.rs:104-111`): `scale`/`dot` inline all types, `cross` for 3D,
but `length`/`distance`/`lerp` inline **Float-only** and `normalize` is **absent from the match** → no type
inlines it. Non-inlined ops make a `#vector_<op>` FUNC call + materialize the register-native operand to a
fresh N×8 arena block; Integer/Fixed `length` also runs software isqrt. `vector math` is dominated by
`normalize` ×2/iter.

## Fixes
- [ ] **H1 (vector math — the biggest single lever, but HARDER)** — inline `normalize`. **Scout
  (plan-86-A session): needs NEW statement-emitting inline machinery with no precedent in the module.** The
  `.mfb` body (`vector_package.mfb:367-376`) is `len=sqrt(Σx²); IF len=0.0 THEN FAIL error(77050002); RETURN
  Float_N[x/len,…]` (divide each lane by len, NOT `scale(v,1/len)`). `try_inline_vector_op`
  (`builder_vector_inline.rs:242-416`) is a PURE-EXPRESSION rewriter — it only assembles
  `NirValue::{Binary,Constructor,Call,MemberAccess}` trees and hands them to `lower_value`; it has NO
  vocabulary for a compare/branch/FAIL. H1 needs a statement-emitting variant: lower Σx²+`math.sqrt` to a
  register, emit `compare_immediate`/`branch_eq` to a fail label emitting the same `FAIL error(77050002)` NIR,
  then lower the per-lane divides into a constructor. **Only lever for vector math (30.9ms, normalize ×2/iter)
  AND vector float (20.9, already fully inlined except normalize).** Higher-risk; do 2nd.
- [x] **H2 (vector int/fixed — TRACTABLE, do FIRST)** — Commit: `89e5a8b7c`. relax the `element=="Float"` clause
  (`builder_vector_inline.rs:108`) for **length/distance ONLY** (KEEP `lerp`/`lerp_unclamped` Float-gated —
  their Fixed/Integer `.mfb` bodies genuinely differ via `toFixed`/`toFloat`/`round`, `vector_package.mfb:981`/
  `:1122`, so they'd need type-specific rewrite branches, not just the gate drop). `vector_call_is_inlined`
  (`:117-128`) already funnels through `vector_op_inlinable`, so the escape analysis + register-lane feeding
  follow AUTOMATICALLY — no second edit for lanes. Then extend the `length` (`:371-384`)/`distance`
  (`:390-406`) rewrite branches: Float keeps `math.sqrt` Call; **Fixed** emits the same `math.sqrt` Call
  (dispatches to `emit_fixed_sqrt`, deterministic `builder_fixed_math.rs:135-201`); **Integer** wraps Σx² in a
  Call to the mangled `__vector_isqrtRound` target (do NOT open-code — bit-identity requires calling the same
  deterministic helper, `vector_package.mfb:137-165`) instead of `math.sqrt`. This removes the operand N×8
  arena block-materialize (`vector_value_as_block`, `builder_vector_inline.rs:184-219`, fired at
  `builder_emit_helpers.rs:66`) for every Integer/Fixed length/distance — helps **vector int (55.7ms, ~18
  length+distance/iter): solid double-digit-% cut** (the isqrt compute stays, so not to zero) + vector fixed.
  Does NOT help vector float/math (those need H1). **Bit-identity mandatory** (module invariant
  `builder_vector_inline.rs:8-17`); gate on the 4 vector checksums + `tools/math-kernels/runtime_ulp.py`
  (sqrt-adjacent) + scalar-vs-array bit identity.

## Acceptance
`tools/math-kernels/runtime_ulp.py` (normalize reuses gated `math::sqrt`) + scalar-vs-array bit identity +
vector checksums + `scripts/artifact-gate.sh`.

## Corrections
- **H2 landed, correct & non-regressing, but the predicted "solid double-digit-% cut" was WRONG — the win
  is MARGINAL.** Measured (`--run 10`, release, box-local): `vector int` 55.71 → 55.34ms (~0.7%, WITHIN
  noise: min 54.885); `vector fixed` 13.58 → 13.16ms (~3%, just past noise: min 12.961). Two reasons the
  root-cause estimate overshot: (1) the **software isqrt/`emit_fixed_sqrt` compute dominates** the residual,
  so removing the N×8 operand block-materialize (the only thing H2 removes) barely moves the total; (2) the
  benchmark's `vector int` body calls `length(normalize(a))` in the hot loop — the arg is a `normalize(...)`
  CALL, NOT re-eval-safe, so `vector_call_is_inlined` keeps those FUNC (H2 only reaches the `length(local)` /
  `distance(l1,l2)` subset). Kept anyway: it is bit-identical to the FUNC oracle (fixture
  `vector-length-distance-inline-rt`: iLen2=5, iAcc=28568, fLen2=5.00 — IDENTICAL to a no-H2 build), a real
  (if small) block-materialize removal, and it makes `length`/`distance` inline for ALL numeric element types
  — consistent with `scale`/`dot`/`cross`. H1 (`normalize`) remains the only real lever for vector
  int/math/float and is still open (needs the statement-emitting inline machinery described in H1).
