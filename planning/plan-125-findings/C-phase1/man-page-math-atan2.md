### 1. Signed zero violates the stated interval
UNIT:      man-page:math/atan2
CLAIM:     "`atan2` returns the angle in radians (in `(-pi, pi]`) between the positive x-axis and the point `(x, y)`, using the signs of both arguments to select the correct quadrant."
VERDICT:   wrong
EVIDENCE:  Probe `/tmp/plan-125-scratch/C-phase1/man-page-math-atan2/src/main.mfb`, built and run with the specified release binary, printed `-3.14` for `math::atan2(-0.0, -1.0)`. `src/codegen/builtins/vector/builder_simd_float_math.rs:emit_float_binary_body` preserves the sign of `y` when applying π.
SUGGESTED: "`atan2` returns an angle in radians in `[-pi, pi]` between the positive x-axis and `(x, y)`, using both signs to select the direction."

### 2. Zero-coordinate behavior is omitted
UNIT:      man-page:math/atan2
CLAIM:     "`atan2` returns the angle in radians (in `(-pi, pi]`) between the positive x-axis and the point `(x, y)`, using the signs of both arguments to select the correct quadrant."
VERDICT:   incomplete
EVIDENCE:  The same probe printed `0.00` for `atan2(0.0, 0.0)`, `1.57` for `atan2(1.0, 0.0)`, `-1.57` for `atan2(-1.0, 0.0)`, and `3.14` for `atan2(0.0, -1.0)`. `src/codegen/builtins/money/gen_fixed_math.rs:emit_fixed_atan2` explicitly handles these axis and origin cases.
SUGGESTED: "It returns `0` for `(0, 0)`; when `x` is zero it returns `pi / 2` or `-pi / 2` according to `y`, and when `y` is zero with negative `x` it returns `pi` or `-pi` according to the sign of zero."

### 3. ErrFloatNaN has no documented condition
UNIT:      man-page:math/atan2
CLAIM:     "Floating-point operation produced a NaN result."
VERDICT:   incomplete
EVIDENCE:  `/tmp/plan-125-scratch/C-phase1/man-page-math-atan2/nan/src/main.mfb` calls `atan2((big * big) / (big * big), 1.0)` and exits 255 with `Error: 7-705-0013 Floating-point operation produced a NaN result.` `src/codegen/builtins/vector/builder_simd_float_math.rs:FloatBinaryKernel::errors` declares `Nan` for `atan2`; the origin is specially converted to zero rather than raising.
SUGGESTED: "ErrFloatNaN is raised when a Float argument expression produces NaN; `atan2(0, 0)` instead returns `0`."

### 4. Array overload does not state that it returns a new list
UNIT:      man-page:math/atan2
CLAIM:     "Both arguments must be the same type (`Float` or `Fixed`), echoing that type; the `List OF Float` array form takes two equal-length lists (mismatched lengths raise `ErrInvalidArgument`)."
VERDICT:   incomplete
EVIDENCE:  The probe’s equal-length list call returned a list of length `2`. `src/codegen/builtins/vector/builder_simd_float_math.rs:lower_simd_float_binary` creates `result_base` with `emit_alloc_result_list` and stores results only through `out_data`, distinct from both input data pointers.
SUGGESTED: "The `List OF Float` form returns a new list of angles, leaving both input lists unchanged; its two input lists must have equal length or it raises `ErrInvalidArgument`."