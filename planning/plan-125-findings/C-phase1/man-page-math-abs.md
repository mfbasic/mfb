### 1. Money overflow omitted from its parameter semantics
UNIT:      man-page:math/abs
CLAIM:     "The number to take the magnitude of, or a list of them. The most negative `Integer`/`Fixed` has no positive counterpart and raises `ErrOverflow`."
VERDICT:   incomplete
EVIDENCE:  Rendered as the parameter description for the `Money` overload. `src/codegen/builtins/math/gen_math.rs:lower_math_abs` raises `ErrOverflow` for `Money` at its signed minimum. Probe `LET minMoney AS Money = toMoney("-92233720368547.75808"); io::print(toString(math::abs(minMoney)))` printed `Error: 7-705-0010`.
SUGGESTED: The number or list whose magnitude to return. For `Integer`, `Fixed`, or `Money`, the most negative value has no positive counterpart and raises `ErrOverflow`.

### 2. Errors table assigns overflow to Float overloads that cannot raise it
UNIT:      man-page:math/abs
CLAIM:     "`ErrOverflow` — Overloads 1, 2, 3, 4, 5, 6, 7."
VERDICT:   wrong
EVIDENCE:  The rendered Errors table lists every overload. `src/codegen/builtins/math/gen_math.rs:lower_math_abs` raises only in the `Integer | Fixed | Money` branch; its `Float` branch clears the sign and has no error path. `lower_math_abs_array` likewise uses `AbsFloat` for `List OF Float`. Thus overloads 2 and 5 cannot raise `ErrOverflow`; applicable overloads are 1, 3, 4, 6, and 7.
SUGGESTED: `ErrOverflow` applies to `List OF Integer`, `List OF Fixed`, `Integer`, `Fixed`, and `Money` only, when an input is that type’s most negative value.

### 3. List-order guarantee is omitted
UNIT:      man-page:math/abs
CLAIM:     "`List OF Integer`/`Float`/`Fixed` array forms (element-wise, returning a new list of the same type)."
VERDICT:   incomplete
EVIDENCE:  The claim does not state whether output order is preserved. Probe input `[0 - 7, 3, 0]` printed `7,3,0`; `src/codegen/builtins/math/gen_math.rs:lower_math_abs_array` applies the unary kernel per list element. The source input subsequently printed `-7`, confirming the call does not alter it.
SUGGESTED: "`List OF Integer`, `Float`, and `Fixed` inputs return a new list of the same type, with each element’s magnitude in the original order."