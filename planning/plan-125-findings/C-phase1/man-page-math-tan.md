### 1. Parameter descriptions promise unsupported list forms
UNIT:      man-page:math/tan
CLAIM:     “The angle in radians, or a list of them.”
VERDICT:   wrong
EVIDENCE:  The rendered table applies this text to all three overloads, but `func_tan::register` accepts only `List OF Float`; the `Float` and `Fixed` overloads accept scalars only. The probe compiled `math::tan([0.0, math::pi2])` as `List OF Float` and printed `float-list-built=2`.
SUGGESTED: For `List OF Float`: “Angles in radians.” For `Float` and `Fixed`: “An angle in radians.”

### 2. Errors table declares errors the Float overloads do not raise
UNIT:      man-page:math/tan
CLAIM:     “77050002 ErrInvalidArgument — Overloads 1, 2, 3” and “77050014 ErrFloatInf — Overloads 1, 2, 3”
VERDICT:   wrong
EVIDENCE:  `FloatKernel::errors` in `src/codegen/builtins/vector/builder_simd_float_math.rs:254` gives `FloatKernel::Tan` only `FloatError::Nan`; `lower_math_scalar_transcendental` in `src/codegen/builtins/math/gen_math.rs` routes scalar Float `tan` through that same kernel. The probe ran Float `math::tan(math::pi2)` and printed the finite `16331239353195370.00`.
SUGGESTED: Declare `ErrFloatNaN` for the Float scalar and list overloads only when its result is NaN; do not list `ErrInvalidArgument` or `ErrFloatInf` for those overloads.