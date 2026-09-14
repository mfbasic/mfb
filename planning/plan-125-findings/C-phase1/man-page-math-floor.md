### 1. Error table assigns ErrOverflow to overloads that cannot raise it
UNIT:      man-page:math/floor
CLAIM:     “ErrOverflow … Overloads 1, 2, 3, 4, 5”
VERDICT:   wrong
EVIDENCE:  `src/codegen/builtins/math/func_floor.rs:register` attaches `ErrOverflow` to every overload, but `src/codegen/builtins/money/gen_fixed_math.rs:CodeBuilder::emit_fixed_rounding_to_integer` and `src/codegen/builtins/money/gen_money_math.rs:CodeBuilder::emit_money_rounding_to_integer` contain no `ErrOverflow` path. The Float-only range check is `src/codegen/builtins/math/gen_math.rs:CodeBuilder::emit_float_rounding_integer_range_check`. Probe `/tmp/plan-125-scratch/C-phase1/man-page-math-floor/overflow` ran `math::floor(9223372036854775808.0)` and printed `Error: 7-705-0010`.
SUGGESTED: `ErrOverflow is raised when a Float value, or an element of a List OF Float, cannot be represented as an Integer.`

### 2. Money parameter description incorrectly says it accepts a list
UNIT:      man-page:math/floor
CLAIM:     “The number to round down, or a list of them. Rounds toward negative infinity, so -1.5 becomes -2.”
VERDICT:   misleading
EVIDENCE:  The rendered Parameters table repeats this text for overload 5, `value AS Money`, but the rendered signature is only `math::floor(value AS Money) AS Integer`; `src/codegen/builtins/math/func_floor.rs:register` supplies list overloads only for `Float` and `Fixed`.
SUGGESTED: `The Money amount to round down. Rounds toward negative infinity, so -1.50m becomes -2.`

### 3. List behavior omits both empty input and non-mutation
UNIT:      man-page:math/floor
CLAIM:     “It accepts Float, Fixed, and Money and returns Integer (a deliberate dimension exit — for Money, the whole-unit count), plus the List OF Float/List OF Fixed array forms returning List OF Integer.”
VERDICT:   incomplete
EVIDENCE:  Probe `/tmp/plan-125-scratch/C-phase1/man-page-math-floor/src/main.mfb` printed `2,-2,0` for each result list, then `2.70,2.70` for the original Float and Fixed inputs, and `0` for `len(math::floor(emptyFloats))`. `src/codegen/builtins/math/gen_math.rs:CodeBuilder::lower_math_rounding_array` returns a new `List OF Integer`.
SUGGESTED: `For List OF Float and List OF Fixed, floor returns a new List OF Integer, leaves the input list unchanged, and returns an empty list for an empty input.`

### 4. “Deliberate dimension exit” is compiler-facing terminology
UNIT:      man-page:math/floor
CLAIM:     “It accepts Float, Fixed, and Money and returns Integer (a deliberate dimension exit — for Money, the whole-unit count)”
VERDICT:   out-of-scope
EVIDENCE:  “Dimension exit” is compiler-registry terminology: `src/codegen/builtins/math/mod.rs` documents the rounding shape as “a deliberate dimension exit,” while the observable contract is simply the Integer result and Money whole-unit count. The probe printed `-2` for `math::floor(0.00m - 1.50m)`.
SUGGESTED: `It accepts Float, Fixed, and Money and returns an Integer. For Money, the result is the whole-unit count.`