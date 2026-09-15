### 1. Input mutation is not stated
UNIT:      man-page:datetime/withZone
CLAIM:     “The date-time whose zone to change.”
VERDICT:   misleading
EVIDENCE:  `src/codegen/builtins/datetime/func_with_zone.rs:BODY` returns `inZone(resolve(dt), z)` and does not alter `dt`. The behavior probe printed `before=2026-06-26T12:00:00.123456789Z` and `withZone=2026-06-26T17:30:00.123456789+05:30`, confirming a distinct returned presentation; command: `.../mfb build -q /tmp/plan-125-scratch/C-phase3/man-page-datetime-withZone/behavior && .../withzone-behavior-probe.out`.
SUGGESTED: The date-time to re-project. It is unchanged; the function returns a new date-time in `zone`.

### 2. ErrInvalidArgument has no developer-visible condition
UNIT:      man-page:datetime/withZone
CLAIM:     “Argument value is not valid for the requested operation.”
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/datetime/func_with_zone.rs:BODY` calls `__datetime_inZone(__datetime_resolve(dt), z)`; `src/codegen/builtins/datetime/func_in_zone.rs:BODY` resolves the zone offset. The probe’s local-zone instant outside host conversion range printed `localRange=TRUE`, meaning it raised `77050002` (`ErrInvalidArgument`); command: `.../withzone-behavior-probe.out`.
SUGGESTED: With a local zone, raises `ErrInvalidArgument` when the preserved instant is outside the range the host can convert to local time.

### 3. ErrOverflow has no triggering condition
UNIT:      man-page:datetime/withZone
CLAIM:     “Arithmetic overflow or numeric conversion outside the destination range.”
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/datetime/func_with_zone.rs:BODY` composes `resolve` and `inZone`; `src/codegen/builtins/datetime/func_in_zone.rs:BODY` adds the target-zone offset to instant seconds. The probe printed `overflow=TRUE` for an instant at the integer limit projected into `+01:00`, raising `77050010` (`ErrOverflow`); command: `.../withzone-behavior-probe.out`.
SUGGESTED: Raises `ErrOverflow` if resolving `dt`, or applying the target zone’s offset to the preserved instant, exceeds the `Integer` range.