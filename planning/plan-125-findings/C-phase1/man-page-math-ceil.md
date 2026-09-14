### 1. Compiler-facing “dimension exit” language
UNIT:      man-page:math/ceil
CLAIM:     "It accepts `Float`, `Fixed`, and `Money` and returns `Integer` (a deliberate dimension exit — for `Money`, the whole-unit count), plus the `List OF Float`/`List OF Fixed` array forms returning `List OF Integer`."
VERDICT:   out-of-scope
EVIDENCE:  `src/codegen/builtins/math/func_ceil.rs:DESC` renders this sentence. “Deliberate dimension exit” describes registry/type-design rationale, not behavior a terminal user needs.
SUGGESTED: "`ceil` accepts `Float`, `Fixed`, and `Money` and returns an `Integer`. For `Money`, it rounds to a whole currency unit. The `List OF Float` and `List OF Fixed` overloads return `List OF Integer`."

### 2. Overflow condition is too vague and overstates affected overloads
UNIT:      man-page:math/ceil
CLAIM:     "A magnitude too large for `Integer` raises `ErrOverflow`."
VERDICT:   misleading
EVIDENCE:  `src/codegen/builtins/math/gen_math.rs:emit_float_rounding_integer_range_check` raises only on Float exponent/range failures; `emit_fixed_rounding_to_integer` states Fixed results always fit, and `emit_money_rounding_to_integer` divides an `i64` Money value by 100000. `/tmp/plan-125-scratch/C-phase1/man-page-math-ceil/overflow-project` printed `Error: 7-705-0010` for `math::ceil(9223372036854775808.0)`, while the probe printed `-9223372036854775808` for `math::ceil(0.0 - 9223372036854775808.0)`.
SUGGESTED: "For `Float` inputs, `ErrOverflow` is raised when the rounded result is outside the `Integer` range."

### 3. List parameter description omits empty-list and non-mutation behavior
UNIT:      man-page:math/ceil
CLAIM:     "The number to round up, or a list of them. Rounds toward positive infinity, so -1.5 becomes -1."
VERDICT:   incomplete
EVIDENCE:  The description appears for both list overload rows. The probe printed `0` for `len(math::ceil([] AS List OF Float))`; it printed rounded results `3,-1,4` while the original inputs remained `2.10,-1.50`. `src/codegen/builtins/math/gen_math.rs:lower_math_rounding_array` constructs a `List OF Integer` result.
SUGGESTED: "The value to round toward positive infinity; `-1.5` becomes `-1`. For a list, each element is rounded into a new `List OF Integer`; an empty list returns an empty list."