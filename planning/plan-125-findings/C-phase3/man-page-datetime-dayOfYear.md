### 1. Implementation recipe in developer documentation
UNIT:      man-page:datetime/dayOfYear
CLAIM:     “The day-of-year is computed on the proleptic-Gregorian calendar by taking the days-from-civil count of dt's date, subtracting the days-from-civil count of January 1 of the same year, and adding one (here - start + 1), so leap years correctly extend the count past February.”
VERDICT:   out-of-scope
EVIDENCE:  `src/codegen/builtins/datetime/func_day_of_year.rs:BODY` implements this through private `__datetime_daysFromCivil` calls; the probe printed `common=1,365` and `leap=60,366`.
SUGGESTED: “Leap years include February 29, so December 31 is day 366 instead of day 365.”

### 2. Compiler-detail claim about unperformed work
UNIT:      man-page:datetime/dayOfYear
CLAIM:     “The time-of-day fields, the sub-second nanoseconds, and the zone's UTC offset do not affect the result; no datetime::Instant is resolved and no zone table is consulted.”
VERDICT:   out-of-scope
EVIDENCE:  The first clause is observable and confirmed by `func_day_of_year.rs:BODY`, which reads only `dt.date`; whether an Instant is resolved or a zone table consulted is an implementation detail.
SUGGESTED: “Only dt's calendar date affects the result; its time, nanoseconds, and UTC offset do not.”

### 3. ErrOverflow condition is omitted
UNIT:      man-page:datetime/dayOfYear
CLAIM:     “ErrOverflow — Arithmetic overflow or numeric conversion outside the destination range.”
VERDICT:   incomplete
EVIDENCE:  `func_day_of_year.rs:BODY` computes civil-day counts and subtraction as `Integer`; a scratch probe passed `DateTime[Date[9223372036854775807, 1, 1], ...]` and printed `overflow=77050010` (`ErrOverflow`).
SUGGESTED: “Raises ErrOverflow when computing the day-of-year for dt's calendar date exceeds the Integer range.”