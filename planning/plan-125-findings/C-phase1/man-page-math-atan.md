### 1. Fixed overload incorrectly suggests list input
UNIT:      man-page:math/atan
CLAIM:     “The tangent to invert, or a list of them.”
VERDICT:   misleading
EVIDENCE:  This description is rendered for the `Fixed` overload. Probe `LET xs AS List OF Fixed = [1.0F] : LET ys AS List OF Fixed = math::atan(xs)` fails with `TYPE_CALL_ARGUMENT_MISMATCH`: `List OF Fixed`, expected `Float | Fixed`. The registry registers only `List OF Float` (`src/codegen/builtins/math/func_atan.rs:20-31`).
SUGGESTED: “The tangent to invert.” Keep the `List OF Float` limitation in the description: “A `List OF Float` produces a list of arctangents.”

### 2. Error table falsely assigns ErrFloatNaN to Fixed
UNIT:      man-page:math/atan
CLAIM:     “ErrFloatNaN … Overloads 1, 2, 3”
VERDICT:   wrong
EVIDENCE:  Overload 3 is `Fixed`. Its lowering calls `emit_fixed_atan2(value, 1)` (`src/codegen/builtins/math/gen_math.rs:1160-1167`); `emit_fixed_atan2` contains no `raise_error` or `ErrFloatNaN` path (`src/codegen/builtins/money/gen_fixed_math.rs:345`). `ErrFloatNaN` is instead accumulated only by the Float kernel (`src/codegen/builtins/vector/builder_simd_float_math.rs:579-589`). The descriptor supplies the error to every overload via `preserving_unary` (`src/codegen/builtins/math/mod.rs:214-260`).
SUGGESTED: List `ErrFloatNaN` only for the Float and `List OF Float` overloads, or remove it if non-finite Float inputs are unreachable at the language surface.