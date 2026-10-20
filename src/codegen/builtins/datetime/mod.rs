//! Built-in `datetime::` package — migrated onto the clean-room registry
//! (`crate::codegen::registry`), mirroring `csv`/`json`/`regex`.
//!
//! The portable calendar math, formatting, and parsing live in the injected
//! source (registry-modeled types and enums, private `__datetime_*` helpers,
//! and the arity-dispatched constructor bodies); each member's body rides its
//! `func_*.rs` `BODY` const via [`Body::mfb`]. Registration, the argument-typed
//! return resolution, and the public→internal rewrite mapping all live here.
//! The only platform state is reached through three intrinsics (`nowNanos`,
//! `monotonicNanos`, `localOffset`) that lower to libc/kernel32 runtime helpers
//! (§8.2) via `Body::abi_function`; each owns its lowering in its own `func_*.rs`
//! (crypto/io's shape), with the shared libc clock reading kept in
//! [`gen_shared::emit_libc_clock_nanos`], and is wired through the shared runtime
//! catalog.
//!
//! Source injection is the registry's ([`crate::codegen::registry::augment_project`]
//! / [`crate::codegen::registry::RegistryPackage::is_imported_by`]) — the
//! per-package `augmented_project`/`uses_package` helpers this module used to own
//! are gone.

use crate::codegen::registry::{
    Body, DefaultValue, EnumVariant, Implementation, Parameter, RecordProp, Registry, RegistryEnum,
    RegistryFunction, RegistryPackage, RegistryRecord,
};
use crate::types::ParameterType;

const MODULE_INTRO: &str =
    r#"Instants, civil dates and times, durations, zones, formatting, and parsing"#;
const MODULE_DESC: &str = r#"The `datetime` package models time around a single source of truth: a `datetime::Instant`,
an absolute point on the UTC timeline (Unix epoch, leap-second-free) carrying
whole seconds and a nanosecond field in the range `0 .. 999_999_999`. A
`datetime::DateTime` that this package *produces* is a projection of an instant through a
`datetime::Zone`, and every projection records the resolved UTC offset, so a
`datetime::DateTime` always knows its offset and round-trips back to its
`datetime::Instant` without re-consulting the zone. `datetime::Date` and `datetime::Time`
are standalone civil components: `datetime::date` and `datetime::time` build them from
their fields alone, with no instant or zone. (A `datetime::DateTime` you build
yourself with the record constructor is not checked: if you supply an `offset`
that does not match the civil fields, `datetime::resolve` believes the offset.) `datetime` is a built-in package: `IMPORT datetime` needs
no manifest dependency.

All public types are flat, copyable value records and enums — `datetime::Instant`,
`datetime::Duration`, `datetime::Date`, `datetime::Time`, `datetime::Zone`, `datetime::DateTime`, and the enums `datetime::ZoneKind`,
`datetime::Weekday`, and `datetime::Month`. There are no resources and no hidden global state, and the
types are always written package-qualified (`datetime::Instant`, `datetime::Date`, …); a bare
`Instant` is `SYMBOL_UNKNOWN_TYPE`. The constructors and arithmetic
return normalized values, but a record you build directly is not checked:
`datetime::Date[2026, 13, 32]` keeps month 13 and day 32, and an `Instant` or
`Duration` keeps a `nanos` outside `0 .. 999_999_999`.

The wall clock (`now` and `nowNanos`) and the monotonic counter (`monotonic` and
`monotonicNanos`) read host clocks. Every operation that resolves a
`datetime::local()` zone reads the host's zone rules: `localOffset`, `civil`,
`inZone`/`toLocal`, `addDays`, and `addMonths`. The same call can therefore
give a different result on a host set to a different zone, while UTC and
fixed-offset arguments give the same result everywhere. Everything else is a
pure function of its arguments.

The zone constructors produce three kinds. `datetime::utc()` is fixed at offset 0;
`datetime::fixedOffset(...)` builds a constant offset rendered as `+HH:MM`; and
`datetime::local()` resolves the host's zone per-instant, so it is DST-correct at
the moment it projects. Named IANA zones are not supported in this version.
`datetime::Instant.seconds` spans the full 64-bit `Integer`, so civil dates reach far beyond
any practical need; `datetime::now()` is additionally bounded by the nanosecond
count it reads, valid from 1677 through 2262; a clock reading outside that range
raises `ErrOverflow`. There are no leap seconds:
every day is 86400 seconds, the POSIX convention.

Projection is the primary "to civil" operation: `inZone` maps an instant into a
zone, `toUtc` and `toLocal` are shorthands, and `resolve` maps a civil `datetime::DateTime`
back to its `datetime::Instant`. Arithmetic operates on instants and durations (`add`,
`subtract`, `between`, `plus`, `minus`, `negate`), on calendar days (`addDays`,
DST-aware) and months (`addMonths`, clamping day-of-month). Formatting and parsing
share a pattern mini-language: a pattern is literal text with token runs, where a
run of the same letter is one token whose length selects width or style, and
literal letters are wrapped in single quotes. `format` renders a `datetime::DateTime`,
`parse` reads one back, and `toIso`/`parseIso` handle RFC 3339 / ISO 8601 with a
required offset.

Project an instant into a fixed-offset zone, move it by calendar days, and
resolve it back:

```
IMPORT io
IMPORT datetime

FUNC main() AS Nothing
  LET at AS datetime::Instant = datetime::instant(1772884800)
  LET tokyo AS datetime::Zone = datetime::fixedOffset(9, 0)
  LET local AS datetime::DateTime = datetime::inZone(at, tokyo)
  io::print(datetime::format(local, "yyyy-MM-dd HH:mm:ss ZZ"))
  LET later AS datetime::DateTime = datetime::addDays(local, 30)
  io::print(datetime::toIso(later))
  io::print(toString(datetime::resolve(local).seconds = at.seconds))
END FUNC
```

prints:

```
2026-03-07 21:00:00 +09:00
2026-04-06T21:00:00.000+09:00
TRUE
```"#;

/// Register the `datetime` package on the clean-room registry.
///
/// Everything renders from registry models: the value records (`Instant`, …,
/// with their `DOC` blocks round-tripped through `RegistryRecord::description`),
/// the enums (`ZoneKind`, `Weekday`, `Month`), the private `__datetime_*`
/// helpers (one `helper_*.rs` per FUNC — private-only), and every member body
/// through its [`Body::mfb`] (the multi-overload constructors carry one body per
/// `Implementation` in their `func_*.rs`).
pub(crate) fn register(r: &mut Registry) {
    let mut pkg = RegistryPackage::new("datetime", MODULE_INTRO, MODULE_DESC);

    // The injected source's IMPORT lines (verbatim order from the old companion;
    // the self-import lets the bodies call the native clock intrinsics).
    pkg.add_imports(vec!["datetime", "strings", "collections"]);

    // The public value records (§3): every one a flat copyable record; the DOC
    // blocks render into the injected source via `description`.
    pkg.add_record(RegistryRecord {
        name: "Instant",
        export: true,
        description: "An absolute point on the UTC timeline, stored as a count of seconds since the Unix epoch plus a sub-second nanosecond part.",
        props: vec![
            RecordProp {
                name: "seconds",
                ty: ParameterType::Integer,
                description: "Whole seconds since the Unix epoch (1970-01-01T00:00:00Z).",
            },
            RecordProp {
                name: "nanos",
                ty: ParameterType::Integer,
                description: "Sub-second part in nanoseconds, 0..999_999_999 in every value the package returns (not checked in a record built directly).",
            },
        ],
    });
    pkg.add_record(RegistryRecord {
        name: "Duration",
        export: true,
        description:
            "A signed span of time between two instants, stored as seconds plus a nanosecond part.",
        props: vec![
            RecordProp {
                name: "seconds",
                ty: ParameterType::Integer,
                description: "The whole-seconds component of the span.",
            },
            RecordProp {
                name: "nanos",
                ty: ParameterType::Integer,
                description: "The sub-second part in nanoseconds, 0..999_999_999 in every value the package returns (not checked in a record built directly).",
            },
        ],
    });
    pkg.add_record(RegistryRecord {
        name: "Date",
        export: true,
        description: "A proleptic-Gregorian calendar date, without any time or zone. `datetime::date` checks the month and the day against that month's length; a `Date` record built directly is not checked, except that a month-name token of `datetime::format` raises `ErrInvalidArgument` for a `month` outside 1..12.",
        props: vec![
            RecordProp {
                name: "year",
                ty: ParameterType::Integer,
                description: "The year (e.g. 2026).",
            },
            RecordProp {
                name: "month",
                ty: ParameterType::Integer,
                description: "The month of the year, 1..12.",
            },
            RecordProp {
                name: "day",
                ty: ParameterType::Integer,
                description: "The day of the month, 1..31.",
            },
        ],
    });
    pkg.add_record(RegistryRecord {
        name: "Time",
        export: true,
        description: "A wall-clock time of day, without any date or zone. `datetime::time` checks the field ranges below; a `Time` record built directly is not checked.",
        props: vec![
            RecordProp {
                name: "hour",
                ty: ParameterType::Integer,
                description: "The hour of the day, 0..23.",
            },
            RecordProp {
                name: "minute",
                ty: ParameterType::Integer,
                description: "The minute of the hour, 0..59.",
            },
            RecordProp {
                name: "second",
                ty: ParameterType::Integer,
                description: "The second of the minute, 0..59.",
            },
            RecordProp {
                name: "nanos",
                ty: ParameterType::Integer,
                description: "The sub-second part in nanoseconds, 0..999_999_999.",
            },
        ],
    });
    pkg.add_record(RegistryRecord {
        name: "Zone",
        export: true,
        description: "A time zone, described by its offset from UTC together with the kind of zone and a display label. Build one with `datetime::utc`, `datetime::fixedOffset`, or `datetime::local`; a `Zone` record built directly is not checked, and any `kind` other than `ZoneKind.Local` is treated as a constant offset.",
        props: vec![
            RecordProp {
                name: "offsetSeconds",
                ty: ParameterType::Integer,
                description: "Offset from UTC in seconds (east positive).",
            },
            RecordProp {
                name: "kind",
                ty: ParameterType::Integer,
                description: "Which kind of zone this is: `datetime::ZoneKind.Utc`, `datetime::ZoneKind.FixedOffset`, or `datetime::ZoneKind.Local` for a zone from the constructors. The field is a plain `Integer`.",
            },
            RecordProp {
                name: "label",
                ty: ParameterType::String,
                description: "A human-readable label for the zone (e.g. `\"UTC\"` or `\"+05:30\"`).",
            },
        ],
    });
    pkg.add_record(RegistryRecord {
        name: "DateTime",
        export: true,
        description: "A zoned date-and-time: a `datetime::Date` and `datetime::Time` interpreted in a `datetime::Zone`, with a UTC offset stored alongside. `datetime::civil` and `datetime::inZone` store the resolved offset; a `DateTime` record built directly is not checked, and `datetime::resolve` uses its stored offset as given.",
        props: vec![
            RecordProp {
                name: "date",
                ty: ParameterType::named("Date"),
                description: "The calendar date component.",
            },
            RecordProp {
                name: "time",
                ty: ParameterType::named("Time"),
                description: "The wall-clock time component.",
            },
            RecordProp {
                name: "zone",
                ty: ParameterType::named("Zone"),
                description: "The zone the date and time are expressed in.",
            },
            RecordProp {
                name: "offset",
                ty: ParameterType::Integer,
                description: "The resolved offset from UTC in seconds at this moment.",
            },
        ],
    });
    // The parser's internal accumulators (not exported: only the `__datetime_*`
    // helper bodies touch them).
    pkg.add_record(RegistryRecord {
        name: "__datetime_NumRead",
        export: false,
        description: "",
        props: vec![
            RecordProp {
                name: "value",
                ty: ParameterType::Integer,
                description: "",
            },
            RecordProp {
                name: "nextPos",
                ty: ParameterType::Integer,
                description: "",
            },
        ],
    });
    pkg.add_record(RegistryRecord {
        name: "__datetime_Fields",
        export: false,
        description: "",
        props: vec![
            RecordProp {
                name: "year",
                ty: ParameterType::Integer,
                description: "",
            },
            RecordProp {
                name: "month",
                ty: ParameterType::Integer,
                description: "",
            },
            RecordProp {
                name: "day",
                ty: ParameterType::Integer,
                description: "",
            },
            RecordProp {
                name: "hour",
                ty: ParameterType::Integer,
                description: "",
            },
            RecordProp {
                name: "minute",
                ty: ParameterType::Integer,
                description: "",
            },
            RecordProp {
                name: "second",
                ty: ParameterType::Integer,
                description: "",
            },
            RecordProp {
                name: "nanos",
                ty: ParameterType::Integer,
                description: "",
            },
            RecordProp {
                name: "offset",
                ty: ParameterType::Integer,
                description: "",
            },
            RecordProp {
                name: "hasOff",
                ty: ParameterType::Boolean,
                description: "",
            },
            RecordProp {
                name: "isPM",
                ty: ParameterType::Boolean,
                description: "",
            },
            RecordProp {
                name: "is12",
                ty: ParameterType::Boolean,
                description: "",
            },
            RecordProp {
                name: "hadPM",
                ty: ParameterType::Boolean,
                description: "",
            },
            RecordProp {
                name: "nextPos",
                ty: ParameterType::Integer,
                description: "",
            },
        ],
    });

    // The private `__datetime_*` helper bodies (normalization, civil math,
    // formatting, and the pattern parser), one `helper_*.rs` per FUNC
    // (`add_helper` — private-only), registered in the old companion order.
    helper_norm_instant::register(&mut pkg);
    helper_norm_duration::register(&mut pkg);
    helper_floor_div::register(&mut pkg);
    helper_floor_mod::register(&mut pkg);
    helper_days_from_civil::register(&mut pkg);
    helper_civil_from_days::register(&mut pkg);
    helper_pad2::register(&mut pkg);
    helper_offset_label_sep::register(&mut pkg);
    helper_offset_label::register(&mut pkg);
    helper_resolve_local::register(&mut pkg);
    helper_civil_keep_offset::register(&mut pkg);
    helper_is_letter::register(&mut pkg);
    helper_pad_n::register(&mut pkg);
    helper_iso_weekday::register(&mut pkg);
    helper_month_name::register(&mut pkg);
    helper_weekday_name::register(&mut pkg);
    helper_offset_label_compact::register(&mut pkg);
    helper_hour12::register(&mut pkg);
    helper_format_token::register(&mut pkg);
    helper_is_digit::register(&mut pkg);
    helper_peek::register(&mut pkg);
    helper_read_num::register(&mut pkg);
    helper_month_from_name::register(&mut pkg);
    helper_skip_weekday_name::register(&mut pkg);
    helper_read_offset::register(&mut pkg);
    helper_parse_fields::register(&mut pkg);
    helper_check_fields::register(&mut pkg);
    helper_build_from_fields::register(&mut pkg);
    helper_expect::register(&mut pkg);
    helper_iso_zone::register(&mut pkg);

    // The public value ENUMS are modeled on the registry — `get_mfb` renders them into
    // the injected source alongside the modeled records.
    pkg.add_enum(RegistryEnum {
        name: "ZoneKind",
        export: true,
        variants: vec![
            EnumVariant {
                name: "Utc",
                description: "Coordinated Universal Time (offset 0).",
                advisory: None,
            },
            EnumVariant {
                name: "FixedOffset",
                description: "A constant offset from UTC.",
                advisory: None,
            },
            EnumVariant {
                name: "Local",
                description: "The host system's local time zone.",
                advisory: None,
            },
        ],
    });
    pkg.add_enum(RegistryEnum {
        name: "Weekday",
        export: true,
        variants: vec![
            EnumVariant {
                name: "Monday",
                description: "Monday.",
                advisory: None,
            },
            EnumVariant {
                name: "Tuesday",
                description: "Tuesday.",
                advisory: None,
            },
            EnumVariant {
                name: "Wednesday",
                description: "Wednesday.",
                advisory: None,
            },
            EnumVariant {
                name: "Thursday",
                description: "Thursday.",
                advisory: None,
            },
            EnumVariant {
                name: "Friday",
                description: "Friday.",
                advisory: None,
            },
            EnumVariant {
                name: "Saturday",
                description: "Saturday.",
                advisory: None,
            },
            EnumVariant {
                name: "Sunday",
                description: "Sunday.",
                advisory: None,
            },
        ],
    });
    pkg.add_enum(RegistryEnum {
        name: "Month",
        export: true,
        variants: vec![
            EnumVariant {
                name: "January",
                description: "January.",
                advisory: None,
            },
            EnumVariant {
                name: "February",
                description: "February.",
                advisory: None,
            },
            EnumVariant {
                name: "March",
                description: "March.",
                advisory: None,
            },
            EnumVariant {
                name: "April",
                description: "April.",
                advisory: None,
            },
            EnumVariant {
                name: "May",
                description: "May.",
                advisory: None,
            },
            EnumVariant {
                name: "June",
                description: "June.",
                advisory: None,
            },
            EnumVariant {
                name: "July",
                description: "July.",
                advisory: None,
            },
            EnumVariant {
                name: "August",
                description: "August.",
                advisory: None,
            },
            EnumVariant {
                name: "September",
                description: "September.",
                advisory: None,
            },
            EnumVariant {
                name: "October",
                description: "October.",
                advisory: None,
            },
            EnumVariant {
                name: "November",
                description: "November.",
                advisory: None,
            },
            EnumVariant {
                name: "December",
                description: "December.",
                advisory: None,
            },
        ],
    });

    func_now::register(&mut pkg);
    func_monotonic::register(&mut pkg);
    func_instant::register(&mut pkg);
    func_date::register(&mut pkg);
    func_time::register(&mut pkg);
    func_duration::register(&mut pkg);
    func_utc::register(&mut pkg);
    func_local::register(&mut pkg);
    func_fixed_offset::register(&mut pkg);
    func_offset_at::register(&mut pkg);
    func_in_zone::register(&mut pkg);
    func_to_utc::register(&mut pkg);
    func_to_local::register(&mut pkg);
    func_resolve::register(&mut pkg);
    func_civil::register(&mut pkg);
    func_with_zone::register(&mut pkg);
    func_add::register(&mut pkg);
    func_subtract::register(&mut pkg);
    func_between::register(&mut pkg);
    func_add_days::register(&mut pkg);
    func_add_months::register(&mut pkg);
    func_compare::register(&mut pkg);
    func_is_before::register(&mut pkg);
    func_is_after::register(&mut pkg);
    func_equals::register(&mut pkg);
    func_negate::register(&mut pkg);
    func_plus::register(&mut pkg);
    func_minus::register(&mut pkg);
    func_weekday::register(&mut pkg);
    func_day_of_year::register(&mut pkg);
    func_is_leap_year::register(&mut pkg);
    func_days_in_month::register(&mut pkg);
    func_start_of_day::register(&mut pkg);
    func_to_millis::register(&mut pkg);
    func_to_nanos::register(&mut pkg);
    func_from_millis::register(&mut pkg);
    func_format::register(&mut pkg);
    func_parse::register(&mut pkg);
    func_to_iso::register(&mut pkg);
    func_parse_iso::register(&mut pkg);
    func_format_duration::register(&mut pkg);
    func_now_nanos::register(&mut pkg);
    func_monotonic_nanos::register(&mut pkg);
    func_local_offset::register(&mut pkg);

    r.add_package(pkg);
}

mod gen_shared;

mod func_add;
mod func_add_days;
mod func_add_months;
mod func_between;
mod func_civil;
mod func_compare;
mod func_date;
mod func_day_of_year;
mod func_days_in_month;
mod func_duration;
mod func_equals;
mod func_fixed_offset;
mod func_format;
mod func_format_duration;
mod func_from_millis;
mod func_in_zone;
mod func_instant;
mod func_is_after;
mod func_is_before;
mod func_is_leap_year;
mod func_local;
mod func_local_offset;
mod func_minus;
mod func_monotonic;
pub(crate) mod func_monotonic_nanos;
mod func_negate;
mod func_now;
mod func_now_nanos;
mod func_offset_at;
mod func_parse;
mod func_parse_iso;
mod func_plus;
mod func_resolve;
mod func_start_of_day;
mod func_subtract;
mod func_time;
mod func_to_iso;
mod func_to_local;
mod func_to_millis;
mod func_to_nanos;
mod func_to_utc;
mod func_utc;
mod func_weekday;
mod func_with_zone;

mod helper_build_from_fields;
mod helper_check_fields;
mod helper_civil_from_days;
mod helper_civil_keep_offset;
mod helper_days_from_civil;
mod helper_expect;
mod helper_floor_div;
mod helper_floor_mod;
mod helper_format_token;
mod helper_hour12;
mod helper_is_digit;
mod helper_is_letter;
mod helper_iso_weekday;
mod helper_iso_zone;
mod helper_month_from_name;
mod helper_month_name;
mod helper_norm_duration;
mod helper_norm_instant;
mod helper_offset_label;
mod helper_offset_label_compact;
mod helper_offset_label_sep;
mod helper_pad2;
mod helper_pad_n;
mod helper_parse_fields;
mod helper_peek;
mod helper_read_num;
mod helper_read_offset;
mod helper_resolve_local;
mod helper_skip_weekday_name;
mod helper_weekday_name;

// Man-page citation anchor: `DATETIME`. The ~50 `datetime/*` man pages ground their
// value-type and OS-seam facts in this package with `[[…/datetime/mod.rs:DATETIME]]`.
//
// The `datetime::` OS-seam intrinsics (`nowNanos` / `monotonicNanos` / `localOffset`,
// plan-01-datetime.md §8.2) are `Body::abi_function` members (`func_now_nanos` et
// al.) each owning its lowering in its own `func_*.rs`, so the shared runtime catalog
// DERIVES their specs from the registry (`registry::runtime_specs`) and routes them
// through the `Datetime` family via `abi_function_family` — no hand-written
// `RuntimeHelperSpec` consts here.

#[cfg(test)]
mod tests;
