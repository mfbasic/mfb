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

SUB main()
  LET at AS datetime::Instant = datetime::instant(1772884800)
  LET tokyo AS datetime::Zone = datetime::fixedOffset(9, 0)
  LET local AS datetime::DateTime = datetime::inZone(at, tokyo)
  io::print(datetime::format(local, "yyyy-MM-dd HH:mm:ss ZZ"))
  LET later AS datetime::DateTime = datetime::addDays(local, 30)
  io::print(datetime::toIso(later))
  io::print(toString(datetime::resolve(local).seconds = at.seconds))
END SUB
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
        description: "A proleptic-Gregorian calendar date, without any time or zone. `datetime::date` checks the month and the day against that month's length; a `Date` record built directly is not checked.",
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
mod func_monotonic_nanos;
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
mod tests {
    use crate::codegen::registry::{self, registry};

    // Qualified-call names exercised by the tests (the mod-level consts were
    // retired — membership now answers through `registry::owning_package`).
    const NOW: &str = "datetime.now";
    const INSTANT: &str = "datetime.instant";
    const DURATION: &str = "datetime.duration";
    const DATE: &str = "datetime.date";
    const TIME: &str = "datetime.time";
    const FIXED_OFFSET: &str = "datetime.fixedOffset";
    const PARSE: &str = "datetime.parse";
    const RESOLVE: &str = "datetime.resolve";
    const ALL_CALLS: &[&str] = &[
        "datetime.now",
        "datetime.monotonic",
        "datetime.instant",
        "datetime.date",
        "datetime.time",
        "datetime.duration",
        "datetime.utc",
        "datetime.local",
        "datetime.fixedOffset",
        "datetime.offsetAt",
        "datetime.inZone",
        "datetime.toUtc",
        "datetime.toLocal",
        "datetime.resolve",
        "datetime.civil",
        "datetime.withZone",
        "datetime.add",
        "datetime.subtract",
        "datetime.between",
        "datetime.addDays",
        "datetime.addMonths",
        "datetime.compare",
        "datetime.isBefore",
        "datetime.isAfter",
        "datetime.equals",
        "datetime.negate",
        "datetime.plus",
        "datetime.minus",
        "datetime.weekday",
        "datetime.dayOfYear",
        "datetime.isLeapYear",
        "datetime.daysInMonth",
        "datetime.startOfDay",
        "datetime.toMillis",
        "datetime.toNanos",
        "datetime.fromMillis",
        "datetime.format",
        "datetime.parse",
        "datetime.toIso",
        "datetime.parseIso",
        "datetime.formatDuration",
        "datetime.nowNanos",
        "datetime.monotonicNanos",
        "datetime.localOffset",
    ];

    #[test]
    fn datetime_registered_on_the_clean_room_registry() {
        let pkg = registry()
            .resolve_package("datetime")
            .expect("datetime package");
        // 41 documented members + 3 OS-seam intrinsics.
        assert_eq!(pkg.functions().len(), 44);
    }

    #[test]
    fn generic_dispatch_reaches_datetime() {
        assert!(registry().is_member("datetime.now"));
        assert!(registry().is_member("datetime.instant"));
        assert!(!registry().is_member("datetime.nope"));
        assert_eq!(registry().owning_package("datetime.now"), Some("datetime"));
        assert_eq!(
            registry::rewrite_target("datetime.now", &[]),
            Some("__datetime_now")
        );
        assert_eq!(
            registry::rewrite_target("datetime.formatDuration", &[]),
            Some("__datetime_formatDuration")
        );
        assert_eq!(registry().arity("datetime.now"), Some((0, 0)));
        assert_eq!(registry().arity("datetime.instant"), Some((1, 5)));
        assert_eq!(registry().arity("datetime.fixedOffset"), Some((1, 2)));
        assert_eq!(registry().arity("datetime.parse"), Some((2, 3)));
        assert_eq!(registry().arity("datetime.time"), Some((2, 4)));
    }

    #[test]
    fn argument_typed_return_resolution() {
        let r = |call: &str, args: &[&str]| {
            let types: Vec<String> = args.iter().map(|s| s.to_string()).collect();
            registry::resolve_call(call, &types, false)
        };
        assert_eq!(r("datetime.now", &[]), Some("datetime.Instant".into()));
        assert_eq!(
            r("datetime.monotonic", &[]),
            Some("datetime.Duration".into())
        );
        assert_eq!(r("datetime.utc", &[]), Some("datetime.Zone".into()));
        assert_eq!(
            r("datetime.instant", &["Integer"]),
            Some("datetime.Instant".into())
        );
        assert_eq!(
            r(
                "datetime.instant",
                &["Integer", "Integer", "Integer", "Integer", "Integer"]
            ),
            Some("datetime.Instant".into())
        );
        assert_eq!(
            r("datetime.duration", &["Integer", "Integer"]),
            Some("datetime.Duration".into())
        );
        assert_eq!(
            r("datetime.date", &["Integer", "Integer", "Integer"]),
            Some("datetime.Date".into())
        );
        assert_eq!(
            r("datetime.time", &["Integer", "Integer"]),
            Some("datetime.Time".into())
        );
        assert_eq!(
            r("datetime.fixedOffset", &["Integer"]),
            Some("datetime.Zone".into())
        );
        assert_eq!(
            r("datetime.inZone", &["Instant", "Zone"]),
            Some("datetime.DateTime".into())
        );
        assert_eq!(
            r("datetime.between", &["Instant", "Instant"]),
            Some("datetime.Duration".into())
        );
        assert_eq!(
            r("datetime.isBefore", &["Instant", "Instant"]),
            Some("Boolean".into())
        );
        assert_eq!(
            r("datetime.parse", &["String", "String"]),
            Some("datetime.DateTime".into())
        );
        assert_eq!(
            r("datetime.parse", &["String", "String", "Zone"]),
            Some("datetime.DateTime".into())
        );
        assert_eq!(r("datetime.nowNanos", &[]), Some("Integer".into()));
        assert_eq!(
            r("datetime.localOffset", &["Integer"]),
            Some("Integer".into())
        );
        // Wrong arity / scalar type -> None.
        assert_eq!(r("datetime.date", &["Integer", "Integer"]), None);
        assert_eq!(r("datetime.instant", &["String"]), None);
    }

    #[test]
    fn overload_selection_by_arity_yields_the_right_rewrite_target() {
        // Each arity constructor overload carries its OWN `__datetime_*N` rewrite;
        // `select` picks by arity and hands back that overload's body target.
        let pkg = registry().resolve_package("datetime").unwrap();
        let select_rewrite = |name: &str, args: &[&str]| -> Option<&'static str> {
            let function = pkg.function(name).unwrap();
            let call = crate::codegen::registry::CallShape {
                args: args
                    .iter()
                    .map(|a| crate::types::ParameterType::parse(a))
                    .collect(),
            };
            function
                .dispatch(&call)
                .and_then(|s| s.implementation.body.rewrite_target())
        };
        assert_eq!(
            select_rewrite("instant", &["Integer"]),
            Some("__datetime_instant1")
        );
        assert_eq!(
            select_rewrite("instant", &["Integer", "Integer", "Integer"]),
            Some("__datetime_instant3")
        );
        assert_eq!(
            select_rewrite(
                "instant",
                &["Integer", "Integer", "Integer", "Integer", "Integer"]
            ),
            Some("__datetime_instant5")
        );
        assert_eq!(
            select_rewrite("duration", &["Integer", "Integer"]),
            Some("__datetime_duration2")
        );
        assert_eq!(
            select_rewrite("fixedOffset", &["Integer"]),
            Some("__datetime_fixedOffset1")
        );
        assert_eq!(
            select_rewrite("fixedOffset", &["Integer", "Integer"]),
            Some("__datetime_fixedOffset2")
        );
        assert_eq!(
            select_rewrite("parse", &["String", "String"]),
            Some("__datetime_parse2")
        );
        assert_eq!(
            select_rewrite("parse", &["String", "String", "Zone"]),
            Some("__datetime_parse3")
        );
    }

    #[test]
    fn default_padding_time_only() {
        // `time`'s trailing `second`/`nanos` are `Fill { Integer, "0" }`, so the
        // generic registry padder injects the right count; every other member (and
        // the arity constructors) pad nothing.
        assert_eq!(registry::default_argument_padding(TIME, 2, None).len(), 2);
        assert_eq!(registry::default_argument_padding(TIME, 3, None).len(), 1);
        assert_eq!(registry::default_argument_padding(TIME, 4, None).len(), 0);
        assert_eq!(registry::default_argument_padding(TIME, 5, None).len(), 0);
        assert_eq!(registry::default_argument_padding(NOW, 0, None), Vec::new());
    }

    #[test]
    fn builtin_types_recognized() {
        // The value records/enums are registry-modeled and
        // recognized through the generic registry via `add_record`.
        for t in [
            "Instant", "Duration", "Date", "Time", "Zone", "DateTime", "ZoneKind", "Weekday",
            "Month",
        ] {
            assert!(registry().is_builtin_type(t), "{t}");
        }
        assert!(!registry().is_builtin_type("Nope"));
        assert!(!registry().is_builtin_type("Integer"));
        // The qualified form resolves the same source-declared names.
        assert_eq!(
            registry().qualified_builtin_type("datetime.Instant"),
            Some("datetime.Instant".to_string())
        );
    }

    #[test]
    fn membership_via_generic_registry() {
        for n in ALL_CALLS {
            assert_eq!(registry().owning_package(n), Some("datetime"), "{n}");
        }
        assert!(registry().owning_package("datetime.nope").is_none());
        assert!(registry().owning_package("other.now").is_none());
    }

    #[test]
    fn expected_arguments_bespoke_phrasings() {
        // The bespoke phrasings now live on each member's descriptor field and are
        // served by the generic `registry::expected_arguments`.
        assert_eq!(registry::expected_arguments(NOW), Some("()"));
        assert_eq!(
            registry::expected_arguments(INSTANT),
            Some("1 to 5 Integer")
        );
        assert_eq!(
            registry::expected_arguments(DURATION),
            Some("1 to 5 Integer")
        );
        assert_eq!(
            registry::expected_arguments(DATE),
            Some("Integer, Integer, Integer")
        );
        assert_eq!(
            registry::expected_arguments(TIME),
            Some("Integer, Integer[, Integer[, Integer]]")
        );
        assert_eq!(
            registry::expected_arguments(FIXED_OFFSET),
            Some("Integer[, Integer]")
        );
        assert_eq!(
            registry::expected_arguments(PARSE),
            Some("String, String[, Zone]")
        );
        // A single-parameter member's phrasing equals the per-position render.
        assert_eq!(registry::expected_arguments(RESOLVE), Some("DateTime"));
        assert_eq!(registry::expected_arguments("datetime.nope"), None);
    }

    #[test]
    fn param_name_tables() {
        // Single-overload and layout-agreeing members merge into one per-position
        // table; the front-dropping constructors carry a per-overload table instead.
        assert_eq!(registry::call_param_names(NOW), Some(vec![]));
        assert_eq!(registry::call_param_names(INSTANT), None);
        assert_eq!(registry::call_param_names(DURATION), None);
        assert_eq!(registry::call_param_names(FIXED_OFFSET), None);
        assert_eq!(registry::call_param_names(DATE).unwrap().len(), 3);
        assert_eq!(
            registry::call_param_name_overloads(INSTANT),
            Some(vec![
                vec!["seconds"],
                vec!["seconds", "nanos"],
                vec!["mins", "seconds", "nanos"],
                vec!["hours", "mins", "seconds", "nanos"],
                vec!["days", "hours", "mins", "seconds", "nanos"],
            ])
        );
        assert_eq!(
            registry::call_param_name_overloads(FIXED_OFFSET),
            Some(vec![vec!["offsetSeconds"], vec!["hours", "mins"]])
        );
    }

    #[test]
    fn reassembled_source_parses() {
        let source = registry()
            .resolve_package("datetime")
            .expect("datetime")
            .get_mfb();
        crate::ast::parse_source_internal(
            std::path::Path::new("<builtin-datetime>"),
            "builtins/datetime.mfb",
            &source,
        )
        .expect("reassembled datetime source parses");
    }

    /// bug-611: every member's declared errors, per overload, are exactly the codes a
    /// boundary probe saw it raise (Integer max/min, huge day/month counts, a Local zone
    /// far from the epoch, a directly built `DateTime` record). The Errors table on
    /// `mfb man datetime <member>` is rendered from these lists, so an empty list here
    /// is a page claiming the call never fails. The runtime half is the
    /// `datetime-arith-errors-rt` fixture. Members bug-520 declared are not repeated.
    #[test]
    fn members_declare_the_errors_they_raise() {
        const OVERFLOW: &str = "ErrOverflow";
        const INVALID_ARGUMENT: &str = "ErrInvalidArgument";
        const INVALID_FORMAT: &str = "ErrInvalidFormat";
        let cases: &[(&str, &[&[&str]])] = &[
            ("datetime.add", &[&[OVERFLOW]]),
            ("datetime.subtract", &[&[OVERFLOW]]),
            ("datetime.plus", &[&[OVERFLOW]]),
            ("datetime.minus", &[&[OVERFLOW]]),
            ("datetime.negate", &[&[OVERFLOW]]),
            ("datetime.between", &[&[OVERFLOW]]),
            (
                "datetime.instant",
                &[&[], &[OVERFLOW], &[OVERFLOW], &[OVERFLOW], &[OVERFLOW]],
            ),
            (
                "datetime.duration",
                &[&[], &[OVERFLOW], &[OVERFLOW], &[OVERFLOW], &[OVERFLOW]],
            ),
            ("datetime.toMillis", &[&[OVERFLOW]]),
            ("datetime.toNanos", &[&[OVERFLOW]]),
            ("datetime.addDays", &[&[INVALID_ARGUMENT, OVERFLOW]]),
            ("datetime.addMonths", &[&[INVALID_ARGUMENT, OVERFLOW]]),
            ("datetime.startOfDay", &[&[INVALID_ARGUMENT, OVERFLOW]]),
            ("datetime.withZone", &[&[INVALID_ARGUMENT, OVERFLOW]]),
            ("datetime.dayOfYear", &[&[OVERFLOW]]),
            ("datetime.weekday", &[&[OVERFLOW]]),
            ("datetime.resolve", &[&[OVERFLOW]]),
            ("datetime.format", &[&[INVALID_FORMAT, OVERFLOW]]),
            ("datetime.formatDuration", &[&[OVERFLOW]]),
            (
                "datetime.toIso",
                &[&[OVERFLOW], &[INVALID_ARGUMENT, OVERFLOW]],
            ),
            // bug-640: a clock reading whose nanosecond count leaves the `Integer`
            // range raises (`tests/runtime/rt_datetime_clock_overflow.rs`).
            ("datetime.now", &[&[OVERFLOW]]),
            ("datetime.nowNanos", &[&[OVERFLOW]]),
            ("datetime.monotonic", &[&[OVERFLOW]]),
            ("datetime.monotonicNanos", &[&[OVERFLOW]]),
            // Probed at their extremes and never raised.
            ("datetime.fromMillis", &[&[]]),
            ("datetime.toUtc", &[&[]]),
            ("datetime.compare", &[&[]]),
            ("datetime.equals", &[&[]]),
            ("datetime.isBefore", &[&[]]),
            ("datetime.isAfter", &[&[]]),
            ("datetime.isLeapYear", &[&[]]),
            ("datetime.daysInMonth", &[&[]]),
            ("datetime.local", &[&[]]),
            ("datetime.utc", &[&[]]),
        ];
        for (member, expected) in cases {
            let function = registry().resolve_func(member).expect(member).function;
            let declared: Vec<Vec<&str>> = function
                .implementations
                .iter()
                .map(|implementation| implementation.errors.clone())
                .collect();
            let expected: Vec<Vec<&str>> = expected.iter().map(|errors| errors.to_vec()).collect();
            assert_eq!(declared, expected, "{member} declared errors per overload");
        }
    }
}
