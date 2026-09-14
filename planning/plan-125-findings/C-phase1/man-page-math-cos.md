### 1. Scalar parameter description claims list support
UNIT:      man-page:math/cos
CLAIM:     "The angle in radians, or a list of them."
VERDICT:   misleading
EVIDENCE:  Rendered `mfb man math cos` applies this text to the `Float` and `Fixed` scalar overloads. `src/codegen/builtins/math/func_cos.rs:register` registers only `List OF Float`, `Float`, and `Fixed`; `gen_math.rs:lower_math_trig_array` accepts a list only when its element type is `Float`.
SUGGESTED: Use overload-specific descriptions: “The angle in radians.” for `Float` and `Fixed`; “The angles in radians.” for `List OF Float`.

### 2. Parameter semantics omit empty, zero, negative, and non-finite behavior
UNIT:      man-page:math/cos
CLAIM:     "The angle in radians, or a list of them."
VERDICT:   incomplete
EVIDENCE:  The page does not say that zero and negative angles are valid, that an empty `List OF Float` returns an empty list, or that a non-finite Float result raises `ErrFloatNaN`. Probe `LET empty AS List OF Float = []; io::print(toString(len(math::cos(empty))))` printed `0`. Probe `math::cos((1e200 * 1e200) / (1e200 * 1e200))` raised `Error: 7-705-0013 Floating-point operation produced a NaN result.`
SUGGESTED: “A finite angle in radians; zero and negative angles are valid.” For the list overload: “A list of finite angles in radians. An empty list returns an empty list; each element may be zero or negative.”

### 3. List behavior does not say whether it changes the input
UNIT:      man-page:math/cos
CLAIM:     "`cos` returns the cosine of `value` (an angle in radians), echoing the operand type (`Float` or `Fixed`), plus the `List OF Float` vectorized form."
VERDICT:   incomplete
EVIDENCE:  The description never states whether the list overload changes its input. `src/codegen/builtins/vector/builder_simd_float_math.rs:lower_simd_float_unary` creates `result_base` and writes results there. Probe with input `[0.0, 3.1415926536]` printed the input’s second value as `3.14` and the result’s as `-1.00`.
SUGGESTED: “For a `List OF Float`, it returns a new list containing the cosine of each angle; the input list is unchanged.”

### 4. Error table omits the condition that raises its listed error
UNIT:      man-page:math/cos
CLAIM:     "Floating-point operation produced a NaN result."
VERDICT:   incomplete
EVIDENCE:  The rendered Errors table names `ErrFloatNaN` but gives no `cos`-specific trigger. `src/codegen/builtins/vector/builder_simd_float_math.rs:FloatKernel::errors` assigns cosine the NaN check, and the non-finite-input probe raised code `77050013`.
SUGGESTED: “Raises `ErrFloatNaN` when a Float input produces a NaN result, including a non-finite input.”