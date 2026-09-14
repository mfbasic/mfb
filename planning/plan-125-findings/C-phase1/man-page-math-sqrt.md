### 1. Float-list domain error is misclassified
UNIT:      man-page:math/sqrt
CLAIM:     “A negative argument is outside the domain and raises `ErrFloatDomain` (scalar `Float`) or `ErrInvalidArgument` (`Fixed` and the array forms).”
VERDICT:   wrong
EVIDENCE:  Probe `LET xs AS List OF Float = [4.0, -1.0, 9.0] : LET ys AS List OF Float = math::sqrt(xs)` built successfully and printed `Error: 7-705-0012` (`ErrFloatDomain`). `src/codegen/builtins/math/gen_math.rs:lower_math_sqrt_array` selects `SimdUnaryKernel::SqrtFloat`; `src/codegen/builtins/vector/builder_simd_math.rs:SimdUnaryKernel::error` maps it to `FloatDomain`.
SUGGESTED: “A negative `Float` value, whether scalar or in a list, raises `ErrFloatDomain`; a negative `Fixed` value raises `ErrInvalidArgument`.”

### 2. `ErrInvalidArgument` row lists wrong overloads
UNIT:      man-page:math/sqrt
CLAIM:     “`ErrInvalidArgument` — Overloads 1, 2, 3, 4”
VERDICT:   wrong
EVIDENCE:  The rendered table assigns the error to every overload, but the negative-`List OF Float` probe printed `Error: 7-705-0012`, not `7-705-0002`. `src/codegen/builtins/math/mod.rs:preserving_unary` attaches every declared error to every overload, while `lower_math_sqrt_array` only raises `ErrInvalidArgument` for the `Fixed` list path.
SUGGESTED: “List `ErrInvalidArgument` only for `Fixed` and `List OF Fixed` overloads, with the condition ‘a value is negative.’”

### 3. `ErrFloatDomain` row lists wrong overloads
UNIT:      man-page:math/sqrt
CLAIM:     “`ErrFloatDomain` — Overloads 1, 2, 3, 4”
VERDICT:   wrong
EVIDENCE:  Probe `LET fixed AS Fixed = -1.0F : LET root AS Fixed = math::sqrt(fixed)` printed `Error: 7-705-0002` (`ErrInvalidArgument`). `src/codegen/builtins/math/gen_math.rs:lower_math_sqrt` raises `ErrInvalidArgument` on negative `Fixed`; the list-fixed probe `[4.0F, -1.0F, 9.0F]` printed the same error.
SUGGESTED: “List `ErrFloatDomain` only for `Float` and `List OF Float` overloads, with the condition ‘a value is negative.’”

### 4. Parameter omits required zero, empty-list, and result-list behavior
UNIT:      man-page:math/sqrt
CLAIM:     “The number to take the square root of, or a list of them. Must not be negative.”
VERDICT:   incomplete
EVIDENCE:  Probe compiled and ran with an empty `List OF Float`, printing `empty=0`; it then printed `input=4.00`, `input=9.00`, `root=2.00`, `root=3.00`, showing zero-length input produces zero-length output and the input list remains unchanged. A prior zero probe printed `0.00` for both `math::sqrt(0.0)` and `math::sqrt(0.0F)`.
SUGGESTED: “A non-negative `Float` or `Fixed`, or a list of non-negative values. Zero returns zero. For a list, `sqrt` returns a same-length list in the same order; an empty list returns an empty list, and the input list is unchanged.”