### 1. Empty-list behavior omitted
UNIT:      man-page:math/round
CLAIM:     "The number to round to the nearest whole value, or a list of them. Halves round away from zero."
VERDICT:   incomplete
EVIDENCE:  Probe `LET empty AS List OF Float = []; LET emptyResult AS List OF Integer = math::round(empty); io::print("emptyLength=" & toString(len(emptyResult)))` printed `emptyLength=0`.
SUGGESTED: The number to round to the nearest whole value, or a list of them. Halves round away from zero. An empty list returns an empty `List OF Integer`.

### 2. List input/result mutation contract omitted
UNIT:      man-page:math/round
CLAIM:     "`round` returns the nearest integer to `value`, rounding halves away from zero. It accepts `Float`, `Fixed`, and `Money` and returns `Integer` (a deliberate dimension exit — for `Money`, the whole-unit count under a fixed half-away rule, distinct from `money::round`), plus the `List OF Float`/`List OF Fixed` array forms returning `List OF Integer`."
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/math/gen_math.rs:422:lower_math_rounding_array` lowers list overloads to `List OF Integer`. Probe input `[2.5, -2.5]` printed `input=2.50,-2.50` after the call, while its result printed `result=3,-3`.
SUGGESTED: `round` returns the nearest integer to `value`, rounding halves away from zero. For `List OF Float` and `List OF Fixed`, it returns a new `List OF Integer` and leaves the input list unchanged.