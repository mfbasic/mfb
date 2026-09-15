### 1. Calendar arithmetic is not target-independent
UNIT:      man-page:datetime/overview
CLAIM:     "Calendar arithmetic produces identical results on every target."
VERDICT:   wrong
EVIDENCE:  `func_add_days.rs:__datetime_addDays` and `func_add_months.rs:__datetime_addMonths` resolve local-zone offsets. The scratch probe run with `TZ=UTC` printed `2026-03-29T09:00:00.000Z`; with `TZ=America/New_York` it printed `2026-03-29T09:00:00.000-04:00` for the same `datetime::civil(..., datetime::local())` input.
SUGGESTED: "Calendar arithmetic is deterministic for UTC and fixed-offset zones. With `datetime::local()`, results use the host's zone rules."

### 2. The host-state list omits local-zone operations
UNIT:      man-page:datetime/overview
CLAIM:     "The operations that read host state are the wall clock (now and nowNanos), the monotonic counter (monotonic and monotonicNanos), and local-zone offset resolution (local, localOffset, and any projection through a local zone); everything else is a pure function of its arguments."
VERDICT:   wrong
EVIDENCE:  `func_local.rs:__datetime_local` returns a constant `Zone[0, 2, "Local"]` and does not read host state. Conversely, `func_civil.rs:__datetime_civil`, `func_add_days.rs:__datetime_addDays`, and `func_add_months.rs:__datetime_addMonths` call local-zone resolution for local zones. The probe produced different civil results under `TZ=UTC`, `TZ=America/New_York`, and `TZ=Pacific/Honolulu`.
SUGGESTED: "The wall-clock and monotonic functions read host clocks. Any operation that resolves a `datetime::local()` zone—including `civil`, local projections, `addDays`, and `addMonths`—uses the host's zone rules."

### 3. Zones are not limited to the three documented kinds
UNIT:      man-page:datetime/overview
CLAIM:     "Zones come in three kinds."
VERDICT:   misleading
EVIDENCE:  `src/codegen/builtins/datetime/mod.rs` registers `Zone` as a public record, and `func_offset_at.rs:__datetime_offsetAt` treats only `kind = 2` as local; every other value uses `offsetSeconds`. The probe constructed `datetime::Zone[0, 99, "custom"]`; `datetime::inZone` accepted it and printed `99:2026-03-29T15:00:00.000Z`.
SUGGESTED: "The zone constructors produce UTC, fixed-offset, and local zones. If you construct a `datetime::Zone` record yourself, use the documented kind and matching fields."

### 4. Invalid-argument row gives no raisable condition
UNIT:      man-page:datetime/overview
CLAIM:     "Argument value is not valid for the requested operation."
VERDICT:   incomplete
EVIDENCE:  The rendered `ErrInvalidArgument` row supplies only that generic message. Registry members raise it for distinct conditions: invalid date fields (`func_date.rs:__datetime_date`), invalid time fields (`func_time.rs:__datetime_time`), invalid fixed offsets (`func_fixed_offset.rs:__datetime_fixedOffset1`), and host-unconvertible local instants (`func_local_offset.rs`).
SUGGESTED: "Invalid arguments raise this error; for the exact accepted ranges and local-zone limits, see the called function's page."

### 5. Invalid-format row gives no raisable condition
UNIT:      man-page:datetime/overview
CLAIM:     "Text parse or non-finite numeric representation conversion failed."
VERDICT:   incomplete
EVIDENCE:  `ErrInvalidFormat` is raised by parsing functions, including `func_parse.rs` and `func_parse_iso.rs:__datetime_parseIso`, for malformed patterns/text, invalid date/time fields, missing required offsets, and trailing text. The overview does not identify any of those conditions.
SUGGESTED: "Malformed datetime text or patterns raise this error; `datetime::parse` and `datetime::parseIso` describe their accepted forms."

### 6. Overflow row gives no raisable condition
UNIT:      man-page:datetime/overview
CLAIM:     "Arithmetic overflow or numeric conversion outside the destination range."
VERDICT:   incomplete
EVIDENCE:  `ErrOverflow` is registered by arithmetic and conversion members, including `func_add.rs`, `func_between.rs`, `func_to_nanos.rs`, `func_resolve.rs`, and constructor overloads in `func_instant.rs`. The rendered overview does not tell a developer which operations can raise it or under what bounds.
SUGGESTED: "Arithmetic or conversion beyond `Integer` range raises this error; each function page states its applicable bound."

### 7. The overview has no executable example
UNIT:      man-page:datetime/overview
CLAIM:     "(No Examples section is rendered.)"
VERDICT:   incomplete
EVIDENCE:  `/Users/justinzaun/Development/mfb/.claude/worktrees/P-125/target/release/mfb man datetime` renders Description, Functions, Errors, and See also, but no Examples section. Therefore there was no page example to copy, compile, or run.
SUGGESTED: "Add a small executable example that constructs an instant, projects it through a fixed-offset zone, and resolves it back."