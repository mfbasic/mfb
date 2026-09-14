### 1. Errors table assigns ErrInvalidArgument to Float overloads

UNIT:      man-page:math/log  
CLAIM:     `ErrInvalidArgument` — Overloads `1, 2, 3, 4`  
VERDICT:   wrong  
EVIDENCE:  Probe `LET value AS Float = 0.0 : io::print(toString(math::log(value)))` built and ran with the release binary; it printed `Error: 7-705-0012` (`ErrFloatDomain`), not `ErrInvalidArgument`. `src/codegen/builtins/math/func_log.rs:register` supplies both errors to every overload, while `gen_math.rs:lower_math_scalar_transcendental` and `builder_simd_float_math.rs:emit_log_body` raise `ErrFloatDomain` for non-positive Float input.  
SUGGESTED: `ErrInvalidArgument` applies only to the Fixed and List OF Fixed overloads when a value or element is zero or negative.

### 2. Errors table assigns ErrFloatDomain to Fixed overloads

UNIT:      man-page:math/log  
CLAIM:     `ErrFloatDomain` — Overloads `1, 2, 3, 4`  
VERDICT:   wrong  
EVIDENCE:  Probe `LET value AS Fixed = 0.0F : io::print(toString(math::log(value)))` built and ran with the release binary; it printed `Error: 7-705-0002` (`ErrInvalidArgument`), not `ErrFloatDomain`. `src/codegen/builtins/money/gen_fixed_math.rs:emit_fixed_log` checks `value <= 0` and raises `ErrInvalidArgument`.  
SUGGESTED: `ErrFloatDomain` applies only to the Float and List OF Float overloads when a value or element is zero or negative.

### 3. List parameter omits empty-list behavior

UNIT:      man-page:math/log  
CLAIM:     “The number to take the natural logarithm of, or a list of them. Must be greater than zero.”  
VERDICT:   incomplete  
EVIDENCE:  Probe `math::log([] AS List OF Float)` and `math::log([] AS List OF Fixed)` compiled and ran, printing `0` and `0` for the returned lengths. `src/codegen/builtins/math/gen_math.rs:lower_math_log_array` accepts list forms, and the Fixed lowering exits immediately when its count is zero.  
SUGGESTED: “The number to take the natural logarithm of, or a list of numbers. A scalar or each list element must be greater than zero; an empty list returns an empty list.”

### 4. List result and non-mutation are unstated

UNIT:      man-page:math/log  
CLAIM:     “`log` returns the natural logarithm (base `e`) of `value`, echoing the operand type (`Float` or `Fixed`), plus the `List OF Float`/`List OF Fixed` forms.”  
VERDICT:   incomplete  
EVIDENCE:  Probe with `values = [math::e]` and `result = math::log(values)` printed `2.72` for `values[0]` and `1.00` for `result[0]`. `src/codegen/builtins/math/gen_math.rs:lower_math_log_array` dispatches list forms; `builder_simd_fixed_math.rs:lower_simd_log_fixed` creates a result list.  
SUGGESTED: “For a list, `log` returns a new list of the same length and element type; the input list is unchanged.”