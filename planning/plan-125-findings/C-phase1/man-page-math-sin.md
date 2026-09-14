### 1. Large finite angles are not handled as sine values
UNIT:      man-page:math/sin
CLAIM:     “Sine of an angle in radians.”
VERDICT:   wrong
EVIDENCE:  Probe `math::sin(100000000000000000000.0)` printed `196250469054002088973237747712.00`; the mathematical sine is bounded to `[-1, 1]`. `src/codegen/builtins/vector/builder_simd_float_math.rs` documents its Cody-Waite reduction as accurate only below `2^20 * pi/2`.
SUGGESTED: Fix the large-angle reduction so every finite `Float` angle returns a sine value; then retain this wording.

### 2. Parameter text promises unsupported list forms and omits list behavior
UNIT:      man-page:math/sin
CLAIM:     “The angle in radians, or a list of them.”
VERDICT:   misleading
EVIDENCE:  Rendered overloads permit `List OF Float`, `Float`, and `Fixed`, but not `List OF Fixed`; nevertheless the same sentence appears on the `Float` and `Fixed` rows. Probe with zero, `-2.0`, and `[]` printed `0.00`, `-0.91`, and `0`; `lower_simd_float_unary` in `src/codegen/builtins/vector/builder_simd_float_math.rs` produces a separate result list.
SUGGESTED: For scalar rows: “An angle in radians; zero returns zero and negative angles are accepted.” For `List OF Float`: “Angles in radians. Returns a new list of their sines; an empty list returns an empty list.”

### 3. The NaN error lacks its actual trigger condition
UNIT:      man-page:math/sin
CLAIM:     “Floating-point operation produced a NaN result.”
VERDICT:   incomplete
EVIDENCE:  Probe `math::sin(0.0 / 0.0)` exited `255` with `Error: 7-705-0013` and that message. `FloatKernel::errors` in `src/codegen/builtins/vector/builder_simd_float_math.rs` assigns `FloatError::Nan` to `Sin`.
SUGGESTED: “Raises `ErrFloatNaN` when `value` is NaN or the calculation produces NaN.”