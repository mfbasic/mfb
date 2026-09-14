### 1. Negative seconds do not always make the result negative
UNIT:      man-page:datetime/duration  
CLAIM:     “Whole seconds. Negative values are allowed and make the whole duration negative.”  
VERDICT:   wrong  
EVIDENCE:  `/tmp/plan-125-scratch/C-phase3/man-page-datetime-duration/src/main.mfb`, built and run with the specified release binary, printed `0,500000000` for `datetime::duration(-1, 1_500_000_000)`. `func_duration.rs`’s two-argument body normalizes the combined values.  
SUGGESTED: Whole seconds. Any Integer is allowed; it is combined with the other supplied components, so a negative value does not necessarily make the resulting duration negative.

### 2. A negative component does not necessarily yield a backward span
UNIT:      man-page:datetime/duration  
CLAIM:     “Every numeric argument may be negative, which yields a negative span pointing backward in time.”  
VERDICT:   misleading  
EVIDENCE:  The focused probe printed `0,500000000` for `datetime::duration(-1, 1_500_000_000)`: a negative argument produced a positive half-second span.  
SUGGESTED: Every numeric argument may be negative. The result’s direction is determined by the combined total of all supplied components.

### 3. `nanos` omits its range and normalization behavior
UNIT:      man-page:datetime/duration  
CLAIM:     “Nanoseconds, the finest resolution a datetime::Duration carries.”  
VERDICT:   incomplete  
EVIDENCE:  `src/codegen/builtins/datetime/helper_norm_duration.rs:BODY` accepts any Integer nanos value, carries whole billions into seconds, and converts negative remainders to `0 .. 999_999_999`; the probe printed `11,500000000` for `(10, 1_500_000_000)` and `-1,999999999` for `(0, -1)`.  
SUGGESTED: Nanoseconds. Any Integer is allowed: zero adds no adjustment, whole billions carry into seconds, and negative values borrow a second so stored nanos is `0 .. 999_999_999`.

### 4. `mins` implies a canonical component without documenting accepted values
UNIT:      man-page:datetime/duration  
CLAIM:     “Whole minutes.”  
VERDICT:   incomplete  
EVIDENCE:  `src/codegen/builtins/datetime/func_duration.rs:BODY_3` computes `mins * 60 + seconds` with no range check. The probe printed `5400,0` for `datetime::duration(90, 0, 0)`.  
SUGGESTED: Minutes. Any Integer is allowed; zero contributes nothing, negative values subtract minutes, and values outside `0 .. 59` are folded into the total.

### 5. `hours` omits negative and non-canonical input semantics
UNIT:      man-page:datetime/duration  
CLAIM:     “Whole hours.”  
VERDICT:   incomplete  
EVIDENCE:  `src/codegen/builtins/datetime/func_duration.rs:BODY_4` computes `hours * 3600` with no range check. The probe printed `90000,0` for `datetime::duration(25, 0, 0, 0)`.  
SUGGESTED: Hours. Any Integer is allowed; zero contributes nothing, negative values subtract hours, and values outside `0 .. 23` are folded into the total.

### 6. `days` omits negative-input semantics
UNIT:      man-page:datetime/duration  
CLAIM:     “Whole days. Added to the other parts, so days := 1, hours := 12 is thirty-six hours.”  
VERDICT:   incomplete  
EVIDENCE:  `src/codegen/builtins/datetime/func_duration.rs:BODY_5` computes `days * 86400` without a range check. The probe printed `-86400,0` for `datetime::duration(-1, 0, 0, 0, 0)`.  
SUGGESTED: Days. Any Integer is allowed; zero contributes nothing, negative values subtract days, and each day contributes 86,400 seconds before the other components are added.

### 7. Overflow documentation omits normalization-induced overflow
UNIT:      man-page:datetime/duration  
CLAIM:     “The folding and normalization are ordinary signed Integer arithmetic, so a sufficiently large day, hour, minute, or second magnitude can overflow the Integer range and trap.”  
VERDICT:   incomplete  
EVIDENCE:  `helper_norm_duration.rs:BODY` returns `Duration[seconds + q, r]`. The probe printed `nanos-causes-overflow=TRUE` for `datetime::duration(9223372036854775807, 1_000_000_000)`, which raises `ErrOverflow` solely because nanos normalization carries one second.  
SUGGESTED: Arithmetic raises `ErrOverflow` when component folding or nanos normalization exceeds the Integer range, including when a nanos carry would push the seconds total past that range.