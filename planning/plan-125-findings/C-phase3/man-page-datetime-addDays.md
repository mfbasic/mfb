### 1. Spring-forward gap does not preserve the wall-clock time
UNIT:      man-page:datetime/addDays
CLAIM:     “Shift a civil datetime::DateTime by a whole number of calendar days, preserving its wall-clock time and zone.”
VERDICT:   wrong
EVIDENCE:  Scratch probe `boundaries/src/main.mfb`, built and run with `TZ=America/New_York`, calls `addDays` on `2026-03-07 02:30:00 -05:00`; it prints `2026-03-08 03:30:00 -04:00`. `src/codegen/builtins/datetime/helper_civil_keep_offset.rs:__datetime_civilKeepOffset` falls back to `__datetime_civil`, whose gap policy advances the nonexistent local time.
SUGGESTED: Shift a civil `datetime::DateTime` by whole calendar days, preserving its zone and, when the target wall-clock time exists, its wall-clock time.

### 2. “Never alters” time fields is false at a DST gap
UNIT:      man-page:datetime/addDays
CLAIM:     “The operation works purely in whole days and never alters the hour, minute, second, or nanosecond fields; for month-length-aware shifts use datetime::addMonths, and for uniform physical-time arithmetic on a datetime::Instant use datetime::add.”
VERDICT:   wrong
EVIDENCE:  The same `TZ=America/New_York` probe prints `2026-03-08 03:30:00 -04:00` after adding one day to a `2026-03-07 02:30:00 -05:00` value: the hour changes from 02 to 03. `__datetime_civilKeepOffset` calls `__datetime_civil` when the original offset is invalid at the target local time.
SUGGESTED: The operation uses whole calendar days. It keeps the time fields when that local time exists; a target in a spring-forward gap is shifted forward by the zone’s local-time rule.

### 3. Error rows omit the conditions that raise them
UNIT:      man-page:datetime/addDays
CLAIM:     “ErrInvalidArgument — Argument value is not valid for the requested operation.” / “ErrOverflow — Arithmetic overflow or numeric conversion outside the destination range.”
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/datetime/func_add_days.rs:register` declares both errors, but the authored description gives neither condition. `__datetime_addDays` can overflow while computing the target day; for a local zone it calls `__datetime_civilKeepOffset`, then `__datetime_civil`, then `datetime::localOffset` through `__datetime_offsetAt`, which can raise `ErrInvalidArgument` when the host cannot convert the target instant.
SUGGESTED: For a local zone, a target time outside the host’s convertible range raises `ErrInvalidArgument`. Arithmetic needed to calculate the target date or time that does not fit an `Integer` raises `ErrOverflow`.

### 4. Calendar-conversion implementation detail is out of scope
UNIT:      man-page:datetime/addDays
CLAIM:     “It converts dt's calendar date to a serial day count, adds days, converts that count back to a year-month-day date, and rebuilds the datetime::DateTime from the new date, dt's original wall-clock time, and dt's original zone.”
VERDICT:   out-of-scope
EVIDENCE:  This describes the implementation sequence in `src/codegen/builtins/datetime/func_add_days.rs:BODY` (`__datetime_daysFromCivil` and `__datetime_civilFromDays`), rather than a developer-visible contract. `.ai/man-content.md` §1 requires content useful to an MFBASIC developer rather than someone reading implementation source.
SUGGESTED: The result has the date that many calendar days earlier or later, with the original zone and the applicable local-time rules.