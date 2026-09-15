### 1. Unvalidated UTC offset is described as re-resolved
UNIT:      man-page:datetime/addMonths
CLAIM:     “dt's UTC offset is kept whenever it is still valid at the new date and time, and otherwise re-resolved through dt's zone the way datetime::civil resolves a local time.”
VERDICT:   wrong
EVIDENCE:  `src/codegen/builtins/datetime/func_add_months.rs:BODY` returns `DateTime[..., dt.zone, dt.offset]` unchanged for every non-local zone. Probe constructed `DateTime[..., datetime::utc(), 123]` and printed `datetime::toIso(datetime::addMonths(malformed, 1), 0)`; output: `2025-02-28T09:00:00+00:02:03`.
SUGGESTED: For a fixed-offset or UTC zone, the result retains `dt`’s zone and stored offset. For a local zone, it keeps the old offset when that offset applies at the target wall-clock time; otherwise it resolves the target time in the local zone.

### 2. `months` parameter omits its zero behavior and effective range
UNIT:      man-page:datetime/addMonths
CLAIM:     “How many months to add. Negative subtracts.”
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/datetime/func_add_months.rs:BODY` accepts an `Integer`, adds it to the month index, and has no separate range check; zero leaves the computed year/month unchanged. A UTC probe adding `1`, `-1`, and `0` to `2025-01-31T09:00:00.123456789Z` printed respectively `2025-02-28T09:00:00.123456789Z`, `2024-12-31T09:00:00.123456789Z`, and `2025-01-31T09:00:00.123456789Z`.
SUGGESTED: A signed number of calendar months to add. Negative values move earlier; zero returns an equal date-time. Values are `Integer`s, but arithmetic that cannot represent the result raises `ErrOverflow`.

### 3. Errors table gives no conditions for either raisable error
UNIT:      man-page:datetime/addMonths
CLAIM:     “ErrInvalidArgument — Argument value is not valid for the requested operation.” / “ErrOverflow — Arithmetic overflow or numeric conversion outside the destination range.”
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/datetime/func_add_months.rs:register` declares both errors. Its `BODY` can overflow while calculating the target month/year; for local zones it calls `__datetime_civilKeepOffset`, which calls `__datetime_offsetAt`. `src/codegen/builtins/datetime/func_offset_at.rs:__datetime_offsetAt` calls `datetime::localOffset`, whose documented condition is an instant outside the host conversion range (`ErrInvalidArgument`).
SUGGESTED: Add: “`ErrOverflow` is raised when calculating the target calendar date cannot fit an `Integer`. For a local zone, `ErrInvalidArgument` is raised when the host cannot resolve the target date and time.”

### 4. Description exposes the implementation algorithm instead of user behavior
UNIT:      man-page:datetime/addMonths
CLAIM:     “It collapses dt's year and month into a single month index (year * 12 + month - 1), adds months, and splits the sum back into a target year and month with a flooring divide so that crossing year boundaries in either direction is handled correctly.”
VERDICT:   out-of-scope
EVIDENCE:  This is a direct restatement of `src/codegen/builtins/datetime/func_add_months.rs:BODY` (`total`, `__datetime_floorDiv`, and `__datetime_floorMod`). `.ai/man-content.md` §1 and §3 require developer-facing behavior rather than a compiler/source-reading mental model.
SUGGESTED: Adding months crosses year boundaries in either direction: one month after December is January of the next year, and one month before January is December of the previous year.