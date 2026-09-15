### 1. Negative-duration wording reverses “length”
UNIT:      man-page:datetime/minus
CLAIM:     "Because both operands are signed datetime::Durations, minus handles spans of either direction: subtracting a negative datetime::Duration lengthens the total, and subtracting a larger span from a smaller one yields a negative datetime::Duration."
VERDICT:   misleading
EVIDENCE:  Probe `/tmp/plan-125-scratch/C-phase3/man-page-datetime-minus/src/main.mfb`, compiled and run with the specified release binary, printed `-10:-20:10` for `minus(duration(-10), duration(-20))`: subtracting the longer-magnitude negative span produces positive 10 seconds, rather than a negative result. `src/codegen/builtins/datetime/func_minus.rs:BODY` implements signed `a.seconds - b.seconds` and `a.nanos - b.nanos`.
SUGGESTED: `minus` computes the signed difference `a - b`. Subtracting a negative duration adds its signed opposite; the result is negative exactly when the signed value of `a` is less than that of `b`.

### 2. Parameter description repeats the ambiguous length rule
UNIT:      man-page:datetime/minus
CLAIM:     "The duration to take away. The result is negative when b is the longer one."
VERDICT:   misleading
EVIDENCE:  The same executable probe printed `-10:-20:10`: `b` is the longer span by magnitude, yet the result is positive. The descriptor at `src/codegen/builtins/datetime/func_minus.rs:register` accepts signed `Duration` values without a sign restriction.
SUGGESTED: `The duration to subtract. Negative values add their signed opposite; the result is negative when the signed value of a is less than that of b.`