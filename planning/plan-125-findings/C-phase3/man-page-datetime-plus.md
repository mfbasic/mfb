### 1. Algebra identity fails at the Integer boundary
UNIT:      man-page:datetime/plus
CLAIM:     "`datetime::plus(a, datetime::negate(b))` equals `datetime::minus(a, b)`."
VERDICT:   wrong
EVIDENCE:  `/tmp/plan-125-scratch/C-phase3/man-page-datetime-plus/edge-probe.mfb` printed `minus-min-min=0,0` and `plus-negate-min-overflow=1` for `a = b = datetime::duration(-9223372036854775808)`. `datetime::negate(b)` raises `ErrOverflow`, while `datetime::minus(a, b)` returns zero; see `src/codegen/builtins/datetime/func_plus.rs:58` and `func_negate.rs:51`.
SUGGESTED: For inputs whose intermediate arithmetic does not overflow, `datetime::plus(a, datetime::negate(b))` equals `datetime::minus(a, b)`.

### 2. Overflow condition omits public raw-Duration inputs
UNIT:      man-page:datetime/plus
CLAIM:     "The addition is ordinary signed `Integer` arithmetic, so a combined second count that exceeds the `Integer` range overflows and traps."
VERDICT:   incomplete
EVIDENCE:  `/tmp/plan-125-scratch/C-phase3/man-page-datetime-plus/edge-probe.mfb` printed `raw-nanos-overflow=1` for `datetime::plus(datetime::Duration[0, 9223372036854775807], datetime::Duration[0, 1])`: the nanosecond-field addition raises `ErrOverflow` even though the seconds sum is zero. `src/codegen/builtins/datetime/func_plus.rs:59` adds both fields before `helper_norm_duration.rs:11`.
SUGGESTED: `plus` raises `ErrOverflow` if either field sum, or the normalization carry added to seconds, exceeds the `Integer` range.