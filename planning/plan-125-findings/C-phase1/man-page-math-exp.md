### 1. Negative inputs’ underflow is omitted and “large” is misleading
UNIT:      man-page:math/exp
CLAIM:     “The exponent to raise e to, or a list of them. A large value overflows the result range.”
VERDICT:   incomplete
EVIDENCE:  Probe `classifyFloat(-800.0)` printed `0.00`, while `classifyFloat(710.0)` printed `trap:77050014`; `FloatKernel::Exp` explicitly saturates below `EXP_UNDERFLOW_THRESHOLD` and reports `ErrFloatInf` above the overflow threshold (`src/codegen/builtins/vector/builder_simd_float_math.rs:518`, `FloatKernel::errors`). The page does not state the zero or negative-input behavior.
SUGGESTED: “The exponent to raise e to. `0` returns `1`; negative values return a positive fraction, and sufficiently negative `Float` values underflow to `0`. A `List OF Float` is processed element by element. Too-large positive `Float` values raise `ErrFloatInf`; too-large `Fixed` values raise `ErrOverflow`.”

### 2. The Errors table falsely attributes `ErrOverflow` to Float overloads
UNIT:      man-page:math/exp
CLAIM:     “ErrOverflow … Overloads 1, 2, 3”
VERDICT:   wrong
EVIDENCE:  The rendered table lists `ErrOverflow` for the `List OF Float`, `Float`, and `Fixed` overloads. Probe output: `math::exp(710.0)` → `trap:77050014`; `math::exp(30.0F)` → `trap:77050010`. The Float and list paths use `FloatKernel::Exp` (`src/codegen/builtins/math/gen_math.rs:197,362`), whose errors are only `ErrFloatNaN` and `ErrFloatInf` (`src/codegen/builtins/vector/builder_simd_float_math.rs:258`); only Fixed reaches `emit_fixed_exp`, which raises `ErrOverflow` (`src/codegen/builtins/math/gen_math.rs:1174`, `src/codegen/builtins/money/gen_fixed_math.rs:621`).
SUGGESTED: “List `ErrFloatInf` for the `Float` and `List OF Float` overloads, and list `ErrOverflow` only for the `Fixed` overload.”