//! Built-in `datetime::` package — migrated onto the clean-room registry
//! (`crate::codegen::registry`), mirroring `csv`/`json`/`regex`.
//!
//! The portable calendar math, formatting, and parsing live in `package.mfb`
//! (types, enums, private `__datetime_*` helpers, and the arity-dispatched
//! constructor bodies); each callable member's body is spliced in from its
//! `func_*.rs` `BODY` const via [`Body::mfb`]. Registration, the argument-typed
//! return resolution, and the public→internal rewrite mapping all live here.
//! The only platform state is reached through three intrinsics (`nowNanos`,
//! `monotonicNanos`, `localOffset`) that lower to libc/kernel32 runtime helpers
//! (§8.2), kept wired through the shared runtime catalog and
//! [`native::lower_datetime_helper`].
//!
//! Source injection is the registry's ([`crate::codegen::registry::augment_project`]
//! / [`crate::codegen::registry::RegistryPackage::is_imported_by`]) — the
//! per-package `augmented_project`/`uses_package` helpers this module used to own
//! are gone.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Lowering, Parameter, ParameterType, Registry,
    RegistryFunction, RegistryPackage,
};

// Public, documented surface. Each maps to an internal `__datetime_<name>`
// implementation carried by its `Implementation`'s `Body` rewrite target (the
// arity constructors carry one per overload, `__datetime_<name>{N}`), except the
// three OS-seam intrinsics, which stay as runtime-helper calls.
const NOW: &str = "datetime.now";
const MONOTONIC: &str = "datetime.monotonic";
const INSTANT: &str = "datetime.instant";
const DATE: &str = "datetime.date";
const TIME: &str = "datetime.time";
const DURATION: &str = "datetime.duration";
const UTC: &str = "datetime.utc";
const LOCAL: &str = "datetime.local";
const FIXED_OFFSET: &str = "datetime.fixedOffset";
const OFFSET_AT: &str = "datetime.offsetAt";
const IN_ZONE: &str = "datetime.inZone";
const TO_UTC: &str = "datetime.toUtc";
const TO_LOCAL: &str = "datetime.toLocal";
const RESOLVE: &str = "datetime.resolve";
const CIVIL: &str = "datetime.civil";
const WITH_ZONE: &str = "datetime.withZone";
const ADD: &str = "datetime.add";
const SUBTRACT: &str = "datetime.subtract";
const BETWEEN: &str = "datetime.between";
const ADD_DAYS: &str = "datetime.addDays";
const ADD_MONTHS: &str = "datetime.addMonths";
const COMPARE: &str = "datetime.compare";
const IS_BEFORE: &str = "datetime.isBefore";
const IS_AFTER: &str = "datetime.isAfter";
const EQUALS: &str = "datetime.equals";
const NEGATE: &str = "datetime.negate";
const PLUS: &str = "datetime.plus";
const MINUS: &str = "datetime.minus";
const WEEKDAY: &str = "datetime.weekday";
const DAY_OF_YEAR: &str = "datetime.dayOfYear";
const IS_LEAP_YEAR: &str = "datetime.isLeapYear";
const DAYS_IN_MONTH: &str = "datetime.daysInMonth";
const START_OF_DAY: &str = "datetime.startOfDay";
const TO_MILLIS: &str = "datetime.toMillis";
const TO_NANOS: &str = "datetime.toNanos";
const FROM_MILLIS: &str = "datetime.fromMillis";
const FORMAT: &str = "datetime.format";
const PARSE: &str = "datetime.parse";
const TO_ISO: &str = "datetime.toIso";
const PARSE_ISO: &str = "datetime.parseIso";
const FORMAT_DURATION: &str = "datetime.formatDuration";

// OS-seam intrinsics (§8.2). Not documented; callable but only return raw
// integers. They lower to runtime helpers (`_mfb_rt_datetime_*`), so their
// `Implementation` bodies carry no rewrite target (the canonical name reaches the
// native seam instead).
const NOW_NANOS: &str = "datetime.nowNanos";
const MONOTONIC_NANOS: &str = "datetime.monotonicNanos";
const LOCAL_OFFSET: &str = "datetime.localOffset";

/// Every `datetime::` call, in registration order. Used by `is_datetime_call`
/// (keeps the const-name surface live and citable).
const ALL_CALLS: &[&str] = &[
    NOW,
    MONOTONIC,
    INSTANT,
    DATE,
    TIME,
    DURATION,
    UTC,
    LOCAL,
    FIXED_OFFSET,
    OFFSET_AT,
    IN_ZONE,
    TO_UTC,
    TO_LOCAL,
    RESOLVE,
    CIVIL,
    WITH_ZONE,
    ADD,
    SUBTRACT,
    BETWEEN,
    ADD_DAYS,
    ADD_MONTHS,
    COMPARE,
    IS_BEFORE,
    IS_AFTER,
    EQUALS,
    NEGATE,
    PLUS,
    MINUS,
    WEEKDAY,
    DAY_OF_YEAR,
    IS_LEAP_YEAR,
    DAYS_IN_MONTH,
    START_OF_DAY,
    TO_MILLIS,
    TO_NANOS,
    FROM_MILLIS,
    FORMAT,
    PARSE,
    TO_ISO,
    PARSE_ISO,
    FORMAT_DURATION,
    NOW_NANOS,
    MONOTONIC_NANOS,
    LOCAL_OFFSET,
];

// --- descriptor builders shared by the per-member `func_*.rs` via `super::` ---

/// A required parameter of type `ty`.
pub(super) fn req(name: &'static str, ty: ParameterType) -> Parameter {
    Parameter {
        name,
        desc: "",
        aliases: &[],
        ty,
        default: DefaultValue::None,
    }
}

/// An optional trailing parameter that widens arity but is NOT default-padded by
/// the registry (`time`'s trailing `second`/`nanos`, padded instead through the
/// retained `default_argument_padding`).
pub(super) fn optional(name: &'static str, ty: ParameterType) -> Parameter {
    Parameter {
        name,
        desc: "",
        aliases: &[],
        ty,
        default: DefaultValue::Optional,
    }
}

/// The concrete nominal type named by `name` (`Instant`, `Date`, ...).
pub(super) fn named(name: &'static str) -> ParameterType {
    ParameterType::Named(name)
}

/// The `Integer` parameter type.
pub(super) fn int() -> ParameterType {
    ParameterType::Integer
}

/// The `String` parameter type.
pub(super) fn string() -> ParameterType {
    ParameterType::String
}

/// The `Boolean` parameter type.
pub(super) fn boolean() -> ParameterType {
    ParameterType::Boolean
}

/// Register a single-body member: one implementation whose `.mfb` body is
/// spliced from `body` and whose call rewrites to the `FUNC` that body declares.
pub(super) fn single(
    pkg: &mut RegistryPackage,
    name: &'static str,
    intro: &'static str,
    desc: &'static str,
    example: &'static str,
    params: Vec<Parameter>,
    return_type: ParameterType,
    body: &'static str,
    rewrite: &'static str,
) {
    pkg.add_function(RegistryFunction {
        name,
        intro,
        desc,
        example,
        implementations: vec![Implementation {
            params,
            return_type,
            errors: vec![],
            lowering: Lowering::Helper,
            body: Body::mfb(body, rewrite),
        }],
    });
}

/// Register an arity-dispatched constructor family (`instant`/`duration`/
/// `fixedOffset`/`parse`): each `(params, rewrite)` becomes its own
/// implementation, so `select` picks the overload by arity and yields that
/// overload's `__datetime_*N` rewrite target — no resolver needed. The bodies
/// live in `package.mfb`, so each implementation is a plain [`Body::Rewrite`].
pub(super) fn arity_family(
    pkg: &mut RegistryPackage,
    name: &'static str,
    intro: &'static str,
    desc: &'static str,
    example: &'static str,
    return_type: ParameterType,
    overloads: Vec<(Vec<Parameter>, &'static str)>,
) {
    let implementations = overloads
        .into_iter()
        .map(|(params, rewrite)| Implementation {
            params,
            return_type: return_type.clone(),
            errors: vec![],
            lowering: Lowering::Helper,
            body: Body::Rewrite(rewrite),
        })
        .collect();
    pkg.add_function(RegistryFunction {
        name,
        intro,
        desc,
        example,
        implementations,
    });
}

/// Register an OS-seam intrinsic (`nowNanos`/`monotonicNanos`/`localOffset`):
/// lowered natively via [`native::lower_datetime_helper`] (posix + windows share
/// the one all-platform emitter), not through a source companion.
pub(super) fn intrinsic(
    pkg: &mut RegistryPackage,
    name: &'static str,
    intro: &'static str,
    desc: &'static str,
    example: &'static str,
    params: Vec<Parameter>,
) {
    pkg.add_function(RegistryFunction {
        name,
        intro,
        desc,
        example,
        implementations: vec![Implementation {
            params,
            return_type: ParameterType::Integer,
            errors: vec![],
            lowering: Lowering::Helper,
            body: Body::native(
                Some(native::lower_datetime_helper),
                Some(native::lower_datetime_helper),
                None,
            ),
        }],
    });
}

const MODULE_INTRO: &str =
    r#"Instants, civil dates and times, durations, zones, formatting, and parsing"#;
const MODULE_DESC: &str = r#"The `datetime` package models time around a single source of truth: an `Instant`,
an absolute point on the UTC timeline (Unix epoch, leap-second-free) carrying
whole seconds and a nanosecond field in the range `0 .. 999_999_999`. Everything
civil — `Date`, `Time`, and `DateTime` — is a projection of an instant through a
`Zone`, and every projection records the resolved UTC offset, so a `DateTime`
always knows its offset and round-trips back to its `Instant` without
re-consulting the zone. `datetime` is a built-in package: `IMPORT datetime` needs
no manifest dependency.

All public types are flat, copyable value records and enums — `Instant`,
`Duration`, `Date`, `Time`, `Zone`, `DateTime`, and the enums `ZoneKind`,
`Weekday`, and `Month`. There are no resources and no hidden global state, and the
types are referenced bare (`Instant`, `Date`, …), not package-qualified. Calendar
arithmetic is pure integer math (Howard Hinnant's civil ↔ epoch-day conversions)
and produces identical results on every target. Only three operations touch the
host: the wall clock (`now`), a monotonic counter (`monotonic`), and the local
zone's DST-correct offset (`local`).

Zones come in three kinds. `datetime::utc()` is fixed at offset 0;
`datetime::fixedOffset(...)` builds a constant offset rendered as `+HH:MM`; and
`datetime::local()` resolves the host's zone per-instant, so it is DST-correct at
the moment it projects. Named IANA zones are not supported in this version.
`Instant.seconds` spans the full 64-bit `Integer`, so civil dates reach far beyond
any practical need; `datetime::now()` is additionally bounded by its intrinsic
(nanoseconds since the epoch), valid through year 2262. There are no leap seconds:
every day is 86400 seconds, the POSIX convention.

Projection is the primary "to civil" operation: `inZone` maps an instant into a
zone, `toUtc` and `toLocal` are shorthands, and `resolve` maps a civil `DateTime`
back to its `Instant`. Arithmetic operates on instants and durations (`add`,
`subtract`, `between`, `plus`, `minus`, `negate`), on calendar days (`addDays`,
DST-aware) and months (`addMonths`, clamping day-of-month). Formatting and parsing
share a pattern mini-language: a pattern is literal text with token runs, where a
run of the same letter is one token whose length selects width or style, and
literal letters are wrapped in single quotes. `format` renders a `DateTime`,
`parse` reads one back, and `toIso`/`parseIso` handle RFC 3339 / ISO 8601 with a
required offset."#;

/// Register the `datetime` package on the clean-room registry.
///
/// The types (`Instant`, `Date`, …) and enums (`ZoneKind`, `Weekday`, `Month`),
/// the private `__datetime_*` helpers, and the arity constructor bodies all live
/// in `package.mfb` (the registry does not model `ENUM`s or the source `DOC`
/// blocks, so they stay authored there); it is injected via
/// `add_helper_functions`, and each single-body member's `.mfb` body is appended
/// through its [`Body::mfb`].
pub(crate) fn register(r: &mut Registry) {
    let mut pkg = RegistryPackage::new("datetime", MODULE_INTRO, MODULE_DESC);

    // `package.mfb` carries its own `IMPORT datetime/strings/collections`, the
    // `TYPE`/`ENUM` declarations, the private helpers, and the arity bodies.
    pkg.add_helper_functions(vec![include_str!("package.mfb")]);

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

/// The public copyable record/enum types defined in `package.mfb`. Referenced
/// bare (`Instant`, `DateTime`, …) like every other builtin type. The registry
/// models neither `ENUM`s nor the source `DOC` blocks, so these type names stay
/// authored in `package.mfb` and are recognized here rather than via the generic
/// `registry::is_builtin_type`.
pub(crate) fn is_builtin_type(name: &str) -> bool {
    matches!(
        name,
        "Instant"
            | "Duration"
            | "Date"
            | "Time"
            | "Zone"
            | "DateTime"
            | "ZoneKind"
            | "Weekday"
            | "Month"
    )
}

/// Whether `name` is one of the package's public `datetime::` calls.
pub(crate) fn is_datetime_call(name: &str) -> bool {
    ALL_CALLS.contains(&name)
}

/// The expected-argument phrasing for a `datetime::` argument-mismatch diagnostic.
/// Kept hand-authored: the optional-argument `[...]` brackets (`time`'s
/// `"Integer, Integer[, Integer[, Integer]]"`, `parse`'s `"String, String[, Zone]"`)
/// and the range prose (`"1 to 5 Integer"`) are shapes the registry's per-position
/// join cannot reproduce, so `builtins::expected_arguments` reads this before the
/// generic registry rendering.
pub(crate) fn expected_arguments(name: &str) -> Option<&'static str> {
    let text = match name {
        NOW | MONOTONIC | UTC | LOCAL | NOW_NANOS | MONOTONIC_NANOS => "()",
        INSTANT | DURATION => "1 to 5 Integer",
        DATE => "Integer, Integer, Integer",
        TIME => "Integer, Integer[, Integer[, Integer]]",
        FIXED_OFFSET => "Integer[, Integer]",
        OFFSET_AT => "Zone, Instant",
        IN_ZONE => "Instant, Zone",
        TO_UTC | TO_LOCAL => "Instant",
        RESOLVE | WEEKDAY | DAY_OF_YEAR | START_OF_DAY | TO_ISO => "DateTime",
        CIVIL => "Date, Time, Zone",
        WITH_ZONE => "DateTime, Zone",
        ADD | SUBTRACT => "Instant, Duration",
        BETWEEN | COMPARE | IS_BEFORE | IS_AFTER | EQUALS => "Instant, Instant",
        ADD_DAYS | ADD_MONTHS => "DateTime, Integer",
        NEGATE => "Duration",
        PLUS | MINUS => "Duration, Duration",
        IS_LEAP_YEAR | FROM_MILLIS | LOCAL_OFFSET => "Integer",
        DAYS_IN_MONTH => "Integer, Integer",
        TO_MILLIS | TO_NANOS => "Instant",
        FORMAT => "DateTime, String",
        PARSE => "String, String[, Zone]",
        PARSE_ISO => "String",
        FORMAT_DURATION => "Duration",
        _ => return None,
    };
    Some(text)
}

/// Per-overload parameter names for datetime builtins whose overloads have
/// structurally different positional layouts (a named arg binds a different
/// index depending on which overload it selects). Each entry is one overload's
/// parameter names, in order. See bug-94/bug-349 for the motivation.
pub(crate) fn call_param_name_overloads(name: &str) -> Option<&'static [&'static [&'static str]]> {
    match name {
        FIXED_OFFSET => Some(&[&["offsetSeconds"], &["hours", "mins"]]),
        INSTANT | DURATION => Some(&[
            &["seconds"],
            &["seconds", "nanos"],
            &["mins", "seconds", "nanos"],
            &["hours", "mins", "seconds", "nanos"],
            &["days", "hours", "mins", "seconds", "nanos"],
        ]),
        _ => None,
    }
}

/// The per-position `[name]` keyword-matching lists for a `datetime::` call, or
/// `None`. The arity constructors (`instant`/`duration`/`fixedOffset`) whose
/// overloads drop components off the front use `call_param_name_overloads`
/// instead and return `None` here.
pub(crate) fn call_param_names(name: &str) -> Option<&'static [&'static [&'static str]]> {
    let params: &'static [&'static [&'static str]] = match name {
        NOW | MONOTONIC | UTC | LOCAL => &[],
        // INSTANT/DURATION/FIXED_OFFSET drop components off the FRONT / disagree on
        // position 0, so they carry no merged per-position table (bug-349/bug-94).
        INSTANT | DURATION | FIXED_OFFSET => return None,
        DATE => &[&["year"], &["month"], &["day"]],
        TIME => &[&["hour"], &["minute"], &["second"], &["nanos"]],
        OFFSET_AT => &[&["zone"], &["at"]],
        IN_ZONE => &[&["at"], &["zone"]],
        TO_UTC | TO_LOCAL => &[&["at"]],
        RESOLVE => &[&["dt"]],
        CIVIL => &[&["date"], &["time"], &["zone"]],
        WITH_ZONE => &[&["dt"], &["zone"]],
        ADD | SUBTRACT => &[&["at"], &["by"]],
        BETWEEN => &[&["start"], &["finish"]],
        ADD_DAYS => &[&["dt"], &["days"]],
        ADD_MONTHS => &[&["dt"], &["months"]],
        COMPARE | IS_BEFORE | IS_AFTER | EQUALS => &[&["a"], &["b"]],
        NEGATE => &[&["d"]],
        PLUS | MINUS => &[&["a"], &["b"]],
        WEEKDAY | DAY_OF_YEAR | START_OF_DAY => &[&["dt"]],
        IS_LEAP_YEAR => &[&["year"]],
        DAYS_IN_MONTH => &[&["year"], &["month"]],
        TO_MILLIS | TO_NANOS => &[&["at"]],
        FROM_MILLIS => &[&["millis"]],
        FORMAT => &[&["dt"], &["pattern"]],
        PARSE => &[&["value"], &["pattern"], &["zone"]],
        TO_ISO => &[&["dt"]],
        PARSE_ISO => &[&["value"]],
        FORMAT_DURATION => &[&["d"]],
        LOCAL_OFFSET => &[&["epochSeconds"]],
        NOW_NANOS | MONOTONIC_NANOS => &[],
        _ => return None,
    };
    Some(params)
}

/// The machine-readable positional argument-type signature IR lowering hands to
/// `call_argument_expected_type` (bug-340 A1). The variable-arity constructors
/// (`instant`/`duration`), the no-argument clocks, and the optional-tail members
/// (`time`/`fixedOffset`/`parse`) have no single fixed positional signature, so
/// they return `None`.
pub(crate) fn argument_types(name: &str) -> Option<&'static [&'static str]> {
    let types: &'static [&'static str] = match name {
        DATE => &["Integer", "Integer", "Integer"],
        OFFSET_AT => &["Zone", "Instant"],
        IN_ZONE => &["Instant", "Zone"],
        TO_UTC | TO_LOCAL => &["Instant"],
        RESOLVE | WEEKDAY | DAY_OF_YEAR | START_OF_DAY | TO_ISO => &["DateTime"],
        CIVIL => &["Date", "Time", "Zone"],
        WITH_ZONE => &["DateTime", "Zone"],
        ADD | SUBTRACT => &["Instant", "Duration"],
        BETWEEN | COMPARE | IS_BEFORE | IS_AFTER | EQUALS => &["Instant", "Instant"],
        ADD_DAYS | ADD_MONTHS => &["DateTime", "Integer"],
        NEGATE => &["Duration"],
        PLUS | MINUS => &["Duration", "Duration"],
        IS_LEAP_YEAR | FROM_MILLIS | LOCAL_OFFSET => &["Integer"],
        DAYS_IN_MONTH => &["Integer", "Integer"],
        TO_MILLIS | TO_NANOS => &["Instant"],
        FORMAT => &["DateTime", "String"],
        PARSE_ISO => &["String"],
        FORMAT_DURATION => &["Duration"],
        _ => return None,
    };
    Some(types)
}

/// Default trailing arguments injected during IR lowering. Only `time` carries
/// trailing defaults (`second`, `nanos` default to 0); the overloaded
/// constructors return EMPTY so the supplied argument count selects the right
/// `.mfb` overload (§5.1.1).
pub(crate) fn default_argument_padding(
    name: &str,
    provided: usize,
) -> &'static [(&'static str, &'static str)] {
    const TIME_DEFAULTS: &[(&str, &str)] = &[("Integer", "0"), ("Integer", "0")];
    match name {
        TIME => &TIME_DEFAULTS[(provided.saturating_sub(2)).min(TIME_DEFAULTS.len())..],
        _ => &[],
    }
}

use crate::target::shared::runtime::{RuntimeHelper, RuntimeHelperAbi, RuntimeHelperSpec};

mod native;
pub(crate) use native::lower_datetime_helper;

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

// `datetime::` OS-seam intrinsics (plan-01-datetime.md §8.2). `nowNanos` /
// `monotonicNanos` take no arguments; `localOffset` takes the epoch-seconds
// instant in `x0`. All return an `Integer` in the standard result-value register
// with the OK tag set. `nowNanos` / `monotonicNanos` cannot fail; `localOffset`
// raises `ErrInvalidArgument` (ERR tag) for an instant `localtime_r` cannot
// represent (bug-42). These `RuntimeHelperSpec`s register in the shared runtime
// catalog (`target/shared/runtime/catalog.rs`), which imports them from here.
pub(crate) const DATETIME_NOW_NANOS_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Datetime,
    call: "datetime.nowNanos",
    abi: RuntimeHelperAbi { returns: "Integer" },
};

pub(crate) const DATETIME_MONOTONIC_NANOS_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Datetime,
    call: "datetime.monotonicNanos",
    abi: RuntimeHelperAbi { returns: "Integer" },
};

pub(crate) const DATETIME_LOCAL_OFFSET_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Datetime,
    call: "datetime.localOffset",
    abi: RuntimeHelperAbi { returns: "Integer" },
};

/// Whether `name` is one of datetime's three OS-seam runtime intrinsics — lowered
/// natively via [`lower_datetime_helper`], not through the source companion. The
/// shared runtime-call recognizer (`target/shared/runtime/mod.rs`) delegates here.
pub(crate) fn is_datetime_runtime_call(name: &str) -> bool {
    matches!(
        name,
        "datetime.nowNanos" | "datetime.monotonicNanos" | "datetime.localOffset"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::registry::{self, registry};

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
        assert!(registry::is_member("datetime.now"));
        assert!(registry::is_member("datetime.instant"));
        assert!(!registry::is_member("datetime.nope"));
        assert_eq!(registry::owning_package("datetime.now"), Some("datetime"));
        assert_eq!(
            registry::rewrite_target("datetime.now", &[]),
            Some("__datetime_now")
        );
        assert_eq!(
            registry::rewrite_target("datetime.formatDuration", &[]),
            Some("__datetime_formatDuration")
        );
        assert_eq!(registry::arity("datetime.now"), Some((0, 0)));
        assert_eq!(registry::arity("datetime.instant"), Some((1, 5)));
        assert_eq!(registry::arity("datetime.fixedOffset"), Some((1, 2)));
        assert_eq!(registry::arity("datetime.parse"), Some((2, 3)));
        assert_eq!(registry::arity("datetime.time"), Some((2, 4)));
    }

    #[test]
    fn argument_typed_return_resolution() {
        let r = |call: &str, args: &[&str]| {
            let types: Vec<String> = args.iter().map(|s| s.to_string()).collect();
            registry::resolve_call(call, &types)
        };
        assert_eq!(r("datetime.now", &[]), Some("Instant".into()));
        assert_eq!(r("datetime.monotonic", &[]), Some("Duration".into()));
        assert_eq!(r("datetime.utc", &[]), Some("Zone".into()));
        assert_eq!(r("datetime.instant", &["Integer"]), Some("Instant".into()));
        assert_eq!(
            r(
                "datetime.instant",
                &["Integer", "Integer", "Integer", "Integer", "Integer"]
            ),
            Some("Instant".into())
        );
        assert_eq!(
            r("datetime.duration", &["Integer", "Integer"]),
            Some("Duration".into())
        );
        assert_eq!(
            r("datetime.date", &["Integer", "Integer", "Integer"]),
            Some("Date".into())
        );
        assert_eq!(
            r("datetime.time", &["Integer", "Integer"]),
            Some("Time".into())
        );
        assert_eq!(r("datetime.fixedOffset", &["Integer"]), Some("Zone".into()));
        assert_eq!(
            r("datetime.inZone", &["Instant", "Zone"]),
            Some("DateTime".into())
        );
        assert_eq!(
            r("datetime.between", &["Instant", "Instant"]),
            Some("Duration".into())
        );
        assert_eq!(
            r("datetime.isBefore", &["Instant", "Instant"]),
            Some("Boolean".into())
        );
        assert_eq!(
            r("datetime.parse", &["String", "String"]),
            Some("DateTime".into())
        );
        assert_eq!(
            r("datetime.parse", &["String", "String", "Zone"]),
            Some("DateTime".into())
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
                    .map(|a| crate::codegen::registry::ParameterType::parse(a))
                    .collect(),
            };
            function
                .select(&call)
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
        // TIME with 2 provided -> two defaults; 3 -> one; 4 -> none.
        assert_eq!(default_argument_padding(TIME, 2).len(), 2);
        assert_eq!(default_argument_padding(TIME, 3).len(), 1);
        assert_eq!(default_argument_padding(TIME, 4).len(), 0);
        assert_eq!(default_argument_padding(TIME, 5).len(), 0);
        assert_eq!(default_argument_padding(NOW, 0), &[]);
    }

    #[test]
    fn builtin_types_recognized() {
        for t in [
            "Instant", "Duration", "Date", "Time", "Zone", "DateTime", "ZoneKind", "Weekday",
            "Month",
        ] {
            assert!(is_builtin_type(t), "{t}");
        }
        assert!(!is_builtin_type("Nope"));
        assert!(!is_builtin_type("Integer"));
    }

    #[test]
    fn is_call_recognizes_all_and_rejects_unknown() {
        for n in ALL_CALLS {
            assert!(is_datetime_call(n), "{n}");
        }
        assert!(!is_datetime_call("datetime.nope"));
        assert!(!is_datetime_call("other.now"));
    }

    #[test]
    fn expected_arguments_bespoke_phrasings() {
        assert_eq!(expected_arguments(NOW), Some("()"));
        assert_eq!(expected_arguments(INSTANT), Some("1 to 5 Integer"));
        assert_eq!(expected_arguments(DURATION), Some("1 to 5 Integer"));
        assert_eq!(expected_arguments(DATE), Some("Integer, Integer, Integer"));
        assert_eq!(
            expected_arguments(TIME),
            Some("Integer, Integer[, Integer[, Integer]]")
        );
        assert_eq!(expected_arguments(FIXED_OFFSET), Some("Integer[, Integer]"));
        assert_eq!(expected_arguments(PARSE), Some("String, String[, Zone]"));
        assert_eq!(expected_arguments("datetime.nope"), None);
    }

    #[test]
    fn param_name_tables() {
        assert_eq!(call_param_names(NOW), Some(&[][..] as &[&[&str]]));
        assert_eq!(call_param_names(INSTANT), None);
        assert_eq!(call_param_names(DURATION), None);
        assert_eq!(call_param_names(FIXED_OFFSET), None);
        assert_eq!(call_param_names(DATE).unwrap().len(), 3);
        assert_eq!(
            call_param_name_overloads(INSTANT),
            Some(
                &[
                    &["seconds"][..],
                    &["seconds", "nanos"][..],
                    &["mins", "seconds", "nanos"][..],
                    &["hours", "mins", "seconds", "nanos"][..],
                    &["days", "hours", "mins", "seconds", "nanos"][..],
                ][..]
            )
        );
        assert_eq!(
            call_param_name_overloads(FIXED_OFFSET),
            Some(&[&["offsetSeconds"][..], &["hours", "mins"][..]][..])
        );
    }

    #[test]
    fn argument_types_machine_table() {
        assert_eq!(
            argument_types(DATE),
            Some(&["Integer", "Integer", "Integer"][..])
        );
        assert_eq!(argument_types(OFFSET_AT), Some(&["Zone", "Instant"][..]));
        assert_eq!(argument_types(FORMAT), Some(&["DateTime", "String"][..]));
        assert_eq!(argument_types(NOW), None);
        assert_eq!(argument_types(INSTANT), None);
        assert_eq!(argument_types(TIME), None);
        assert_eq!(argument_types(PARSE), None);
        assert_eq!(argument_types("datetime.nope"), None);
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
}
