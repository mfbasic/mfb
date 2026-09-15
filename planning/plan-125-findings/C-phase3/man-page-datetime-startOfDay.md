### 1. Intro promises midnight when midnight can be skipped
UNIT:      man-page:datetime/startOfDay
CLAIM:     “Return the civil datetime::DateTime naming midnight at the start of a datetime::DateTime's day, in its own zone.”
VERDICT:   wrong
EVIDENCE:  Probe built with the specified release binary, with `TZ=America/Havana`, made a local `2026-03-08 12:30` and printed `datetime::startOfDay(dt)` as `2026-03-08 01:00:00.000000000 -04:00`. `src/codegen/builtins/datetime/func_start_of_day.rs:BODY` calls `__datetime_civil(... Time[0,0,0,0] ...)`, whose gap resolution advances to the post-transition time.
SUGGESTED: Return the civil `datetime::DateTime` at the start of its day in its own zone—normally midnight, or the first existing time when midnight is skipped.

### 2. Description repeats the false midnight guarantee
UNIT:      man-page:datetime/startOfDay
CLAIM:     “datetime::startOfDay returns the datetime::DateTime naming 00:00:00 (midnight) at the beginning of dt's civil day, in dt's own zone.”
VERDICT:   wrong
EVIDENCE:  The same `TZ=America/Havana` probe printed `2026-03-08 01:00:00.000000000 -04:00`, not midnight. The rendered page itself later describes this exception.
SUGGESTED: `datetime::startOfDay` returns the `datetime::DateTime` at the beginning of `dt`'s civil day in `dt`'s own zone: normally `00:00:00`, or the first existing time when midnight is skipped.

### 3. Repeated-midnight behavior is omitted
UNIT:      man-page:datetime/startOfDay
CLAIM:     “Not always midnight: in a zone whose clocks jump forward at midnight, the day starts at the first time that exists.”
VERDICT:   incomplete
EVIDENCE:  A release-binary probe with `TZ=America/Havana` and local `2026-11-01 12:00` printed `2026-11-01 00:00:00 -04:00`. That midnight occurs twice; `src/codegen/builtins/datetime/helper_resolve_local.rs:__datetime_resolveLocal` selects the earlier offset when both candidates are valid. The page documents only the spring-forward edge.
SUGGESTED: Not always uniquely midnight: if midnight is skipped, the day starts at the first existing time; if midnight occurs twice, it uses the first occurrence.

### 4. Errors table gives no raising conditions
UNIT:      man-page:datetime/startOfDay
CLAIM:     “ErrInvalidArgument — Argument value is not valid for the requested operation.”
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/datetime/func_start_of_day.rs:BODY` delegates to `__datetime_civil`; `src/codegen/builtins/datetime/func_civil.rs:DESC` identifies the actual condition: with `datetime::local`, a wall-clock time outside the host conversion range raises `ErrInvalidArgument`. The rendered `startOfDay` page supplies only the generic derived message.
SUGGESTED: With `datetime::local`, the result raises `ErrInvalidArgument` when the host cannot convert that day’s local start time.

### 5. Overflow condition is also omitted
UNIT:      man-page:datetime/startOfDay
CLAIM:     “ErrOverflow — Arithmetic overflow or numeric conversion outside the destination range.”
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/datetime/func_civil.rs:__datetime_civil` calculates a civil second count, and `src/codegen/builtins/datetime/helper_resolve_local.rs:__datetime_resolveLocal` adjusts it while resolving the zone. The `civil` documentation identifies an out-of-range date’s second count as the `ErrOverflow` condition; the rendered `startOfDay` page does not.
SUGGESTED: It raises `ErrOverflow` when the day’s civil second count or its zone-resolution arithmetic does not fit an `Integer`.