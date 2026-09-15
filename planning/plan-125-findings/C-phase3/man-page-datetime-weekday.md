### 1. Implementation formula exposed
UNIT:      man-page:datetime/weekday
CLAIM:     “The day count for that civil date is computed on the proleptic-Gregorian calendar and reduced modulo seven against a fixed reference (floorMod(days + 3, 7)), so the answer is the wall-clock weekday a person reading dt's date in its zone would name.”
VERDICT:   out-of-scope
EVIDENCE:  `src/codegen/builtins/datetime/func_weekday.rs:BODY` contains the `__datetime_daysFromCivil` / `__datetime_floorMod(days + 3, 7)` implementation. The fixed-offset probe printed `2026-06-26 Friday +00:00` and `2026-06-25 Thursday -01:00`, confirming the user-visible civil-date behavior without requiring the formula.
SUGGESTED: `Weekday is calculated from dt's civil date using the proleptic-Gregorian calendar.`

### 2. Internal resolution details
UNIT:      man-page:datetime/weekday
CLAIM:     “The time-of-day fields, the sub-second nanoseconds, and the zone's UTC offset do not affect the result; no datetime::Instant is resolved and no zone table is consulted.”
VERDICT:   out-of-scope
EVIDENCE:  `src/codegen/builtins/datetime/func_weekday.rs:BODY` reads only `dt.date.year`, `.month`, and `.day`; whether it resolves an `Instant` or consults a zone table is implementation detail.
SUGGESTED: `The time of day, nanoseconds, and UTC offset do not affect the result.`

### 3. Overflow condition omitted
UNIT:      man-page:datetime/weekday
CLAIM:     “Arithmetic overflow or numeric conversion outside the destination range.”
VERDICT:   incomplete
EVIDENCE:  The rendered Errors row supplies only the generic message. `overflow-project/src/main.mfb` constructed date year `999999999999999999` and called `datetime::weekday`; the specified release binary printed `77050010` and `Arithmetic overflow or numeric conversion outside the destination range.` `src/codegen/builtins/datetime/helper_days_from_civil.rs:BODY` performs the overflowing calendar-day arithmetic.
SUGGESTED: `Raises ErrOverflow when the calendar-day calculation for dt's year exceeds the Integer range.`