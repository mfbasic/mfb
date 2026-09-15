### 1. Error table assigns `ErrInvalidArgument` to Float overloads
UNIT:      man-page:math/log10
CLAIM:     `ErrInvalidArgument │ 1, 2, 3, 4`
VERDICT:   wrong
EVIDENCE:  Probe `/tmp/plan-125-scratch/C-phase1/man-page-math-log10/float-list-zero/src/main.mfb` calls `math::log10([1.0, 0.0])`; the supplied release binary prints `Error: 7-705-0012` (`ErrFloatDomain`), not `ErrInvalidArgument`. `src/codegen/builtins/math/gen_math.rs:lower_math_log_array` routes `List OF Float` to `FloatKernel::Log10`.
SUGGESTED: `ErrInvalidArgument │ 2, 4` — “Raised when a Fixed value or List OF Fixed contains a non-positive value.”

### 2. Error table assigns `ErrFloatDomain` to Fixed overloads
UNIT:      man-page:math/log10
CLAIM:     `ErrFloatDomain │ 1, 2, 3, 4`
VERDICT:   wrong
EVIDENCE:  Probe `/tmp/plan-125-scratch/C-phase1/man-page-math-log10/fixed-list-zero/src/main.mfb` calls `math::log10([1.0F, 0.0F])`; the supplied release binary prints `Error: 7-705-0002` (`ErrInvalidArgument`), not `ErrFloatDomain`. `src/codegen/builtins/money/gen_fixed_math.rs:emit_fixed_log` raises `ErrInvalidArgument` for `x <= 0`.
SUGGESTED: `ErrFloatDomain │ 1, 3` — “Raised when a Float value or List OF Float contains a non-positive value.”

### 3. List edge behavior is omitted
UNIT:      man-page:math/log10
CLAIM:     “The number to take the base-10 logarithm of, or a list of them. Must be greater than zero.”
VERDICT:   incomplete
EVIDENCE:  Probe `/tmp/plan-125-scratch/C-phase1/man-page-math-log10/list-edges/src/main.mfb` prints `0`, `100.00`, `2.00`, `0.00`, `1.00`: an empty Float list returns an empty list; a non-empty input remains unchanged; results retain element order. The page does not state the empty-list case, that list elements are transformed independently, or that the list overload returns a new ordered list.
SUGGESTED: “For a list, every element must be greater than zero; an empty list returns an empty list. The result is a new list of logarithms in the same order, leaving the input list unchanged.”