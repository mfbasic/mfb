//! Built-in `datetime::` package seam (plan-01-datetime.md).
//!
//! Mirrors `json`/`regex`: the portable calendar math, formatting, and parsing
//! live in `datetime_package.mfb` as internal `__datetime_*` functions; this
//! module owns registration, syntaxcheck metadata, and the mapping from a public
//! `datetime::` call onto its internal implementation. The only platform state
//! is reached through three intrinsics (`nowNanos`, `monotonicNanos`,
//! `localOffset`) that lower to libc runtime helpers (§8.2).

use std::borrow::Cow;
use std::path::Path;

// Public, documented surface. Each maps to an internal `__datetime_<name>`
// implementation in the `.mfb` (see `implementation_name`), except the three
// OS-seam intrinsics, which stay as runtime-helper calls.
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
// integers. They lower to runtime helpers (`_mfb_rt_datetime_*`), so they are
// deliberately excluded from `implementation_name`.
const NOW_NANOS: &str = "datetime.nowNanos";
const MONOTONIC_NANOS: &str = "datetime.monotonicNanos";
const LOCAL_OFFSET: &str = "datetime.localOffset";

#[derive(Clone)]
pub(crate) struct ResolvedCall<'a> {
    pub(crate) return_type: Cow<'a, str>,
}

/// The public copyable record/enum types defined in `datetime_package.mfb`.
/// Referenced bare (`Instant`, `DateTime`, …) like every other builtin type.
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

pub(crate) fn is_datetime_call(name: &str) -> bool {
    matches!(
        name,
        NOW | MONOTONIC
            | INSTANT
            | DATE
            | TIME
            | DURATION
            | UTC
            | LOCAL
            | FIXED_OFFSET
            | OFFSET_AT
            | IN_ZONE
            | TO_UTC
            | TO_LOCAL
            | RESOLVE
            | CIVIL
            | WITH_ZONE
            | ADD
            | SUBTRACT
            | BETWEEN
            | ADD_DAYS
            | ADD_MONTHS
            | COMPARE
            | IS_BEFORE
            | IS_AFTER
            | EQUALS
            | NEGATE
            | PLUS
            | MINUS
            | WEEKDAY
            | DAY_OF_YEAR
            | IS_LEAP_YEAR
            | DAYS_IN_MONTH
            | START_OF_DAY
            | TO_MILLIS
            | TO_NANOS
            | FROM_MILLIS
            | FORMAT
            | PARSE
            | TO_ISO
            | PARSE_ISO
            | FORMAT_DURATION
            | NOW_NANOS
            | MONOTONIC_NANOS
            | LOCAL_OFFSET
    )
}

pub(crate) fn call_param_names(name: &str) -> Option<&'static [&'static [&'static str]]> {
    let params: &'static [&'static [&'static str]] = match name {
        NOW | MONOTONIC | UTC | LOCAL => &[],
        // Overloaded/component constructors: name parameters by their maximal
        // arity. Overload selection is by count, so the leading names line up.
        INSTANT | DURATION => &[&["days"], &["hours"], &["mins"], &["seconds"], &["nanos"]],
        DATE => &[&["year"], &["month"], &["day"]],
        TIME => &[&["hour"], &["minute"], &["second"], &["nanos"]],
        FIXED_OFFSET => &[&["hours", "offsetSeconds"], &["mins"]],
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

pub(crate) fn call_return_type_name(name: &str) -> Option<&'static str> {
    let type_ = match name {
        NOW | INSTANT | RESOLVE | FROM_MILLIS => "Instant",
        MONOTONIC | DURATION | BETWEEN | NEGATE | PLUS | MINUS => "Duration",
        DATE => "Date",
        TIME => "Time",
        UTC | LOCAL | FIXED_OFFSET => "Zone",
        IN_ZONE | TO_UTC | TO_LOCAL | CIVIL | WITH_ZONE | ADD_DAYS | ADD_MONTHS | START_OF_DAY => {
            "DateTime"
        }
        ADD | SUBTRACT => "Instant",
        OFFSET_AT | COMPARE | DAY_OF_YEAR | DAYS_IN_MONTH | TO_MILLIS | TO_NANOS | NOW_NANOS
        | MONOTONIC_NANOS | LOCAL_OFFSET => "Integer",
        IS_BEFORE | IS_AFTER | EQUALS | IS_LEAP_YEAR => "Boolean",
        WEEKDAY => "Weekday",
        FORMAT | TO_ISO | FORMAT_DURATION => "String",
        PARSE | PARSE_ISO => "DateTime",
        _ => return None,
    };
    Some(type_)
}

pub(crate) fn resolve_call<'a>(name: &str, arg_types: &'a [String]) -> Option<ResolvedCall<'a>> {
    let all_integer = |types: &[String]| types.iter().all(|t| t == "Integer");
    let return_type: &str = match name {
        NOW if arg_types.is_empty() => "Instant",
        MONOTONIC if arg_types.is_empty() => "Duration",
        UTC | LOCAL if arg_types.is_empty() => "Zone",
        NOW_NANOS | MONOTONIC_NANOS if arg_types.is_empty() => "Integer",
        // Component builders: 1..=5 / 1..=2 Integer args (§5.1.1).
        INSTANT if (1..=5).contains(&arg_types.len()) && all_integer(arg_types) => "Instant",
        DURATION if (1..=5).contains(&arg_types.len()) && all_integer(arg_types) => "Duration",
        FIXED_OFFSET if (1..=2).contains(&arg_types.len()) && all_integer(arg_types) => "Zone",
        DATE if exact(arg_types, &["Integer", "Integer", "Integer"]) => "Date",
        TIME if (2..=4).contains(&arg_types.len()) && all_integer(arg_types) => "Time",
        OFFSET_AT if exact(arg_types, &["Zone", "Instant"]) => "Integer",
        IN_ZONE if exact(arg_types, &["Instant", "Zone"]) => "DateTime",
        TO_UTC | TO_LOCAL if exact(arg_types, &["Instant"]) => "DateTime",
        RESOLVE if exact(arg_types, &["DateTime"]) => "Instant",
        CIVIL if exact(arg_types, &["Date", "Time", "Zone"]) => "DateTime",
        WITH_ZONE if exact(arg_types, &["DateTime", "Zone"]) => "DateTime",
        ADD | SUBTRACT if exact(arg_types, &["Instant", "Duration"]) => "Instant",
        BETWEEN if exact(arg_types, &["Instant", "Instant"]) => "Duration",
        ADD_DAYS if exact(arg_types, &["DateTime", "Integer"]) => "DateTime",
        ADD_MONTHS if exact(arg_types, &["DateTime", "Integer"]) => "DateTime",
        COMPARE if exact(arg_types, &["Instant", "Instant"]) => "Integer",
        IS_BEFORE | IS_AFTER | EQUALS if exact(arg_types, &["Instant", "Instant"]) => "Boolean",
        NEGATE if exact(arg_types, &["Duration"]) => "Duration",
        PLUS | MINUS if exact(arg_types, &["Duration", "Duration"]) => "Duration",
        WEEKDAY if exact(arg_types, &["DateTime"]) => "Weekday",
        DAY_OF_YEAR if exact(arg_types, &["DateTime"]) => "Integer",
        IS_LEAP_YEAR if exact(arg_types, &["Integer"]) => "Boolean",
        DAYS_IN_MONTH if exact(arg_types, &["Integer", "Integer"]) => "Integer",
        START_OF_DAY if exact(arg_types, &["DateTime"]) => "DateTime",
        TO_MILLIS | TO_NANOS if exact(arg_types, &["Instant"]) => "Integer",
        FROM_MILLIS if exact(arg_types, &["Integer"]) => "Instant",
        FORMAT if exact(arg_types, &["DateTime", "String"]) => "String",
        PARSE
            if exact(arg_types, &["String", "String"])
                || exact(arg_types, &["String", "String", "Zone"]) =>
        {
            "DateTime"
        }
        TO_ISO if exact(arg_types, &["DateTime"]) => "String",
        PARSE_ISO if exact(arg_types, &["String"]) => "DateTime",
        FORMAT_DURATION if exact(arg_types, &["Duration"]) => "String",
        LOCAL_OFFSET if exact(arg_types, &["Integer"]) => "Integer",
        _ => return None,
    };
    Some(ResolvedCall {
        return_type: Cow::Borrowed(return_type),
    })
}

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

pub(crate) fn arity(name: &str) -> Option<(usize, usize)> {
    let span = match name {
        NOW | MONOTONIC | UTC | LOCAL | NOW_NANOS | MONOTONIC_NANOS => (0, 0),
        INSTANT | DURATION => (1, 5),
        FIXED_OFFSET => (1, 2),
        TIME => (2, 4),
        PARSE => (2, 3),
        DATE | CIVIL => (3, 3),
        DAYS_IN_MONTH | OFFSET_AT | IN_ZONE | WITH_ZONE | ADD | SUBTRACT | BETWEEN | ADD_DAYS
        | ADD_MONTHS | COMPARE | IS_BEFORE | IS_AFTER | EQUALS | PLUS | MINUS | FORMAT => (2, 2),
        TO_UTC | TO_LOCAL | RESOLVE | WEEKDAY | DAY_OF_YEAR | IS_LEAP_YEAR | START_OF_DAY
        | TO_MILLIS | TO_NANOS | FROM_MILLIS | TO_ISO | PARSE_ISO | FORMAT_DURATION | NEGATE
        | LOCAL_OFFSET => (1, 1),
        _ => return None,
    };
    Some(span)
}

/// The internal `__datetime_*` implementation for a public call, given the
/// supplied argument count. Returns `None` for the OS-seam intrinsics (which
/// stay as `datetime.*` runtime-helper calls). For the arity-overloaded
/// constructors and `parse`, the count selects a distinct internal name so the
/// `.mfb` need not rely on overload resolution through the implementation seam.
pub(crate) fn implementation_name(name: &str, argc: usize) -> Option<String> {
    let internal = match name {
        NOW_NANOS | MONOTONIC_NANOS | LOCAL_OFFSET => return None,
        INSTANT => format!("__datetime_instant{argc}"),
        DURATION => format!("__datetime_duration{argc}"),
        FIXED_OFFSET => format!("__datetime_fixedOffset{argc}"),
        PARSE => format!("__datetime_parse{argc}"),
        _ => format!("__datetime_{}", name.strip_prefix("datetime.")?),
    };
    Some(internal)
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

pub(crate) fn source_file() -> Result<crate::ast::AstFile, ()> {
    crate::ast::parse_source_internal(
        Path::new("<builtin-datetime>"),
        "builtins/datetime.mfb",
        include_str!("datetime_package.mfb"),
    )
}

pub(crate) fn uses_package(ast: &crate::ast::AstProject) -> bool {
    ast.files.iter().any(|file| {
        file.imports
            .iter()
            .any(|import| import.package_name() == "datetime")
    })
}

pub(crate) fn augmented_project(
    ast: &crate::ast::AstProject,
) -> Result<crate::ast::AstProject, ()> {
    if !uses_package(ast) {
        return Ok(ast.clone());
    }
    let mut augmented = ast.clone();
    augmented.files.push(source_file()?);
    Ok(augmented)
}

fn exact(arg_types: &[String], expected: &[&str]) -> bool {
    arg_types.len() == expected.len()
        && arg_types
            .iter()
            .zip(expected.iter())
            .all(|(actual, expected)| actual == expected)
}
