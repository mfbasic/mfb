### 1. Cross-process comparison is incorrectly ruled out
UNIT:      man-page:datetime/monotonic
CLAIM:     "It is unrelated to wall-clock time, carries no calendar meaning, and is not comparable across processes or across reboots, so the absolute value of a single reading is meaningless."
VERDICT:   wrong
EVIDENCE:  `src/codegen/builtins/datetime/func_monotonic_nanos.rs:lower_monotonic_nanos` reads the host `CLOCK_MONOTONIC` clock directly; `func_monotonic.rs:__datetime_monotonic` only splits that one system reading. Two independently launched probe processes printed `reading=9962839.70159000` and `reading=9962839.72191000`, demonstrating the same increasing clock scale rather than a per-process origin.
SUGGESTED: "It is unrelated to wall-clock time, carries no calendar meaning, and resets across reboots, so do not treat a single reading as a calendar timestamp."

### 2. Daylight saving is not a wall-clock adjustment
UNIT:      man-page:datetime/monotonic
CLAIM:     "Because the clock is immune to wall-clock adjustments (NTP steps, manual clock changes, daylight saving), the difference is a reliable interval where `datetime::now` would not be."
VERDICT:   misleading
EVIDENCE:  `src/docs/spec/stdlib/02_datetime.md:Monotonic vs wall clock` states that DST is a zone-offset change, not a wall-clock jump. `datetime::now` is an epoch-relative `Instant`; DST affects local-zone conversion, not the difference between two `now` readings.
SUGGESTED: "Because the clock is immune to wall-clock adjustments such as NTP steps and manual clock changes, its difference is suitable for elapsed-time measurement."

### 3. The nanos upper bound is not explicitly inclusive
UNIT:      man-page:datetime/monotonic
CLAIM:     "The split never fails, and `nanos` always falls in `0 .. 999_999_999`."
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/datetime/helper_norm_duration.rs:BODY` returns a remainder `r` with `0 <= r < 1_000_000_000`; therefore `999_999_999` is included. The probe printed `nanos_in_range=TRUE`, and the implementation establishes the exact inclusive endpoint.
SUGGESTED: "The split never fails, and `nanos` is always an integer from `0` through `999_999_999`, inclusive."