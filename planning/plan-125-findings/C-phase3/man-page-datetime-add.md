### 1. Intro promises the wrong direction for negative durations
UNIT:      man-page:datetime/add
CLAIM:     "Shift a datetime::Instant forward along the UTC timeline by a datetime::Duration."
VERDICT:   misleading
EVIDENCE:  `src/codegen/builtins/datetime/func_add.rs:__datetime_add` adds signed `by.seconds` and `by.nanos`; a probe using `datetime::add(datetime::instant(1, 0), datetime::duration(0, -500_000_000))` printed `0:500000000`.
SUGGESTED: Shift a `datetime::Instant` along the UTC timeline by a `datetime::Duration`.

### 2. `by` omits the zero-duration behavior
UNIT:      man-page:datetime/add
CLAIM:     "How far to shift it. A negative datetime::Duration moves backwards, so add covers both directions."
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/datetime/func_add.rs:__datetime_add` performs signed addition and `__datetime_normInstant`; the probe `add(instant(1, 0), duration(0))` printed `1:0`, identical to its input. The same probe printed `0:500000000` for a negative half-second duration and `2:100000000` for a carry across a second.
SUGGESTED: The duration to add. It may be positive, zero, or negative: zero returns an instant equal to `at`, and a negative duration moves backwards.