### 1. Instant and Duration ranges are not enforced
UNIT:      man-page:datetime/types  
CLAIM:     “Sub-second part in nanoseconds, in the range 0..999_999_999.” / “The sub-second part in nanoseconds, in the range 0..999_999_999.”  
VERDICT:   wrong  
EVIDENCE:  `raw_records.bas`, compiled and run with the specified release binary, constructs `datetime::Instant[0, -1]` and `datetime::Duration[0, 1000000000]`; it printed `-1,1000000000,...`. `src/codegen/builtins/datetime/mod.rs:RegistryRecord Instant/Duration` registers public records without field validation.  
SUGGESTED: `Values returned by datetime constructors and arithmetic use nanos in 0..999_999_999. Direct record construction does not validate or normalize fields.`

### 2. Date field ranges imply validation that direct construction does not perform
UNIT:      man-page:datetime/types  
CLAIM:     “The month of the year, 1..12.” / “The day of the month, 1..31.”  
VERDICT:   wrong  
EVIDENCE:  `raw_records.bas` constructs `datetime::Date[2026, 13, 32]` and printed `...,13,32,...`; `datetime::toIso` then printed `2026-13-32T24:60:60.000+00:02:03`. `src/codegen/builtins/datetime/func_date.rs:__datetime_date` performs validation, while the public `RegistryRecord` in `mod.rs` does not.  
SUGGESTED: `datetime::date validates month and day, including the actual month length. Direct Date record construction accepts its Integer fields unchanged; use datetime::date for a valid calendar date.`

### 3. Time field ranges imply validation that direct construction does not perform
UNIT:      man-page:datetime/types  
CLAIM:     “The hour of the day, 0..23.” / “The minute of the hour, 0..59.” / “The second of the minute, 0..59.” / “The sub-second part in nanoseconds, in the range 0..999_999_999.”  
VERDICT:   wrong  
EVIDENCE:  `raw_records.bas` constructs `datetime::Time[24, 60, 60, -1]` and printed `...,24,-1,...`; formatting its containing DateTime printed `2026-13-32T24:60:60.000+00:02:03`. `src/codegen/builtins/datetime/func_time.rs:__datetime_time` validates these bounds, but direct public record construction does not.  
SUGGESTED: `datetime::time validates hour, minute, second, and nanos. Direct Time record construction accepts its Integer fields unchanged; use datetime::time for a valid time of day.`

### 4. Zone kind is not restricted to the listed enum values
UNIT:      man-page:datetime/types  
CLAIM:     “Which kind of zone this is: datetime::ZoneKind.Utc, datetime::ZoneKind.FixedOffset, or datetime::ZoneKind.Local.”  
VERDICT:   wrong  
EVIDENCE:  `raw_records.bas` constructs `datetime::Zone[999999, 99, "not a zone"]` and printed `99,not a zone`; `src/codegen/builtins/datetime/mod.rs:RegistryRecord Zone` declares `kind` as `Integer`, not `ZoneKind`.  
SUGGESTED: `Zones returned by datetime::utc, datetime::fixedOffset, and datetime::local use ZoneKind.Utc, ZoneKind.FixedOffset, and ZoneKind.Local respectively. Direct Zone record construction does not validate kind, offsetSeconds, or label.`

### 5. DateTime.offset is not necessarily a resolved offset
UNIT:      man-page:datetime/types  
CLAIM:     “A zoned date-and-time: a datetime::Date and datetime::Time interpreted in a datetime::Zone, with the resolved UTC offset cached alongside.”  
VERDICT:   misleading  
EVIDENCE:  `raw_records.bas` constructs `datetime::DateTime[d, t, z, 123]`, where `z` has kind `99`; it printed offset `123`, and `datetime::resolve(dt)` printed `1801529937,-1`, using that supplied offset and nanos unchanged. `src/codegen/builtins/datetime/func_resolve.rs:__datetime_resolve` subtracts `dt.offset` directly.  
SUGGESTED: `DateTime values produced by datetime::civil and datetime::inZone include the resolved UTC offset. Direct DateTime record construction does not validate that its fields or offset agree.`

### 6. The types page has no executable example
UNIT:      man-page:datetime/types  
CLAIM:     `<no example is present on this page>`  
VERDICT:   incomplete  
EVIDENCE:  Rendering `/Users/justinzaun/Development/mfb/.claude/worktrees/P-125/target/release/mfb man datetime types` produced Records and Enums only; there is no Examples section or code block to compile and run.  
SUGGESTED: `Add a compiled example using datetime::date, datetime::time, and datetime::civil, and state that those constructors validate fields whereas direct record construction does not.`