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
- [ ] **H1 (biggest, vector math)** — inline `normalize` as `scale(v, 1/length(v))` with the zero-length
  `FAIL error(77050002)` guard — needs a **guard-capable inline path** (the guard is control flow the
  pure-expression inliner can't emit today).
- [ ] **H2 (vector int/fixed)** — relax the `element=="Float"` clause at `:108` for length/distance/lerp,
  feeding register-native lanes to skip the block materialize, and inline the Fixed/Integer isqrt
  (`emit_fixed_sqrt` already deterministic).

## Acceptance
`tools/math-kernels/runtime_ulp.py` (normalize reuses gated `math::sqrt`) + scalar-vs-array bit identity +
vector checksums + `scripts/artifact-gate.sh`.
