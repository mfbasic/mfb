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
        // FIXED_OFFSET's two overloads disagree on position 0 (`offsetSeconds`
        // in the 1-arg form vs `hours` in the 2-arg form), so it cannot use a
        // merged per-position table — a merged alias would bind `hours := N`
        // to the 1-arg `offsetSeconds` slot (bug-94). It uses a per-overload
        // table instead; see `call_param_name_overloads`.
        FIXED_OFFSET => return None,
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

/// Per-overload parameter names for datetime builtins whose overloads have
/// structurally different positional layouts (a named arg binds a different
/// index depending on which overload it selects). Each entry is one overload's
/// parameter names, in order. See `net::call_param_name_overloads` for the
/// pattern and bug-94 for the `fixedOffset` motivation.
pub(crate) fn call_param_name_overloads(name: &str) -> Option<&'static [&'static [&'static str]]> {
    match name {
        FIXED_OFFSET => Some(&[&["offsetSeconds"], &["hours", "mins"]]),
        _ => None,
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn project(src: &str) -> crate::ast::AstProject {
        let file = crate::ast::parse_source(std::path::Path::new("main.mfb"), "main.mfb", src)
            .expect("parse source");
        crate::ast::AstProject {
            name: "test".to_string(),
            files: vec![file],
        }
    }

    fn rt(name: &str, args: &[&str]) -> Option<String> {
        resolve_call(name, &strings(args)).map(|r| r.return_type.into_owned())
    }

    #[test]
    fn builtin_types() {
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
        for n in [
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
        ] {
            assert!(is_datetime_call(n), "{n}");
        }
        assert!(!is_datetime_call("datetime.nope"));
        assert!(!is_datetime_call("other.now"));
    }

    #[test]
    fn param_names_present_and_unknown_none() {
        assert_eq!(call_param_names(NOW), Some(&[][..] as &[&[&str]]));
        assert_eq!(
            call_param_names(INSTANT),
            Some(
                &[
                    &["days"][..],
                    &["hours"],
                    &["mins"],
                    &["seconds"],
                    &["nanos"]
                ][..]
            )
        );
        assert_eq!(call_param_names(DATE).unwrap().len(), 3);
        // FIXED_OFFSET has no merged per-position table (its overloads disagree
        // on position 0); it uses a per-overload table instead (bug-94).
        assert_eq!(call_param_names(FIXED_OFFSET), None);
        assert_eq!(
            call_param_name_overloads(FIXED_OFFSET),
            Some(&[&["offsetSeconds"][..], &["hours", "mins"][..]][..])
        );
        assert_eq!(call_param_names(NOW_NANOS), Some(&[][..] as &[&[&str]]));
        assert!(call_param_names("datetime.nope").is_none());
    }

    #[test]
    fn return_type_name_table() {
        assert_eq!(call_return_type_name(NOW), Some("Instant"));
        assert_eq!(call_return_type_name(MONOTONIC), Some("Duration"));
        assert_eq!(call_return_type_name(DATE), Some("Date"));
        assert_eq!(call_return_type_name(TIME), Some("Time"));
        assert_eq!(call_return_type_name(UTC), Some("Zone"));
        assert_eq!(call_return_type_name(IN_ZONE), Some("DateTime"));
        assert_eq!(call_return_type_name(ADD), Some("Instant"));
        assert_eq!(call_return_type_name(OFFSET_AT), Some("Integer"));
        assert_eq!(call_return_type_name(IS_BEFORE), Some("Boolean"));
        assert_eq!(call_return_type_name(WEEKDAY), Some("Weekday"));
        assert_eq!(call_return_type_name(FORMAT), Some("String"));
        assert_eq!(call_return_type_name(PARSE), Some("DateTime"));
        assert_eq!(call_return_type_name(NOW_NANOS), Some("Integer"));
        assert!(call_return_type_name("datetime.nope").is_none());
    }

    #[test]
    fn resolve_zero_arg_forms() {
        assert_eq!(rt(NOW, &[]), Some("Instant".to_string()));
        assert_eq!(rt(MONOTONIC, &[]), Some("Duration".to_string()));
        assert_eq!(rt(UTC, &[]), Some("Zone".to_string()));
        assert_eq!(rt(LOCAL, &[]), Some("Zone".to_string()));
        assert_eq!(rt(NOW_NANOS, &[]), Some("Integer".to_string()));
        assert_eq!(rt(MONOTONIC_NANOS, &[]), Some("Integer".to_string()));
        // wrong arity
        assert_eq!(rt(NOW, &["Integer"]), None);
    }

    #[test]
    fn resolve_component_builders() {
        assert_eq!(rt(INSTANT, &["Integer"]), Some("Instant".to_string()));
        assert_eq!(
            rt(
                INSTANT,
                &["Integer", "Integer", "Integer", "Integer", "Integer"]
            ),
            Some("Instant".to_string())
        );
        assert_eq!(rt(INSTANT, &[]), None); // below min
        assert_eq!(
            rt(
                INSTANT,
                &["Integer", "Integer", "Integer", "Integer", "Integer", "Integer"]
            ),
            None
        ); // above max
        assert_eq!(rt(INSTANT, &["String"]), None); // wrong type
        assert_eq!(
            rt(DURATION, &["Integer", "Integer"]),
            Some("Duration".to_string())
        );
        assert_eq!(rt(FIXED_OFFSET, &["Integer"]), Some("Zone".to_string()));
        assert_eq!(
            rt(FIXED_OFFSET, &["Integer", "Integer"]),
            Some("Zone".to_string())
        );
        assert_eq!(rt(FIXED_OFFSET, &["Integer", "Integer", "Integer"]), None);
        assert_eq!(
            rt(DATE, &["Integer", "Integer", "Integer"]),
            Some("Date".to_string())
        );
        assert_eq!(rt(DATE, &["Integer", "Integer"]), None);
        assert_eq!(rt(TIME, &["Integer", "Integer"]), Some("Time".to_string()));
        assert_eq!(
            rt(TIME, &["Integer", "Integer", "Integer", "Integer"]),
            Some("Time".to_string())
        );
        assert_eq!(rt(TIME, &["Integer"]), None);
    }

    #[test]
    fn resolve_typed_forms() {
        assert_eq!(
            rt(OFFSET_AT, &["Zone", "Instant"]),
            Some("Integer".to_string())
        );
        assert_eq!(
            rt(IN_ZONE, &["Instant", "Zone"]),
            Some("DateTime".to_string())
        );
        assert_eq!(rt(TO_UTC, &["Instant"]), Some("DateTime".to_string()));
        assert_eq!(rt(TO_LOCAL, &["Instant"]), Some("DateTime".to_string()));
        assert_eq!(rt(RESOLVE, &["DateTime"]), Some("Instant".to_string()));
        assert_eq!(
            rt(CIVIL, &["Date", "Time", "Zone"]),
            Some("DateTime".to_string())
        );
        assert_eq!(
            rt(WITH_ZONE, &["DateTime", "Zone"]),
            Some("DateTime".to_string())
        );
        assert_eq!(
            rt(ADD, &["Instant", "Duration"]),
            Some("Instant".to_string())
        );
        assert_eq!(
            rt(SUBTRACT, &["Instant", "Duration"]),
            Some("Instant".to_string())
        );
        assert_eq!(
            rt(BETWEEN, &["Instant", "Instant"]),
            Some("Duration".to_string())
        );
        assert_eq!(
            rt(ADD_DAYS, &["DateTime", "Integer"]),
            Some("DateTime".to_string())
        );
        assert_eq!(
            rt(ADD_MONTHS, &["DateTime", "Integer"]),
            Some("DateTime".to_string())
        );
        assert_eq!(
            rt(COMPARE, &["Instant", "Instant"]),
            Some("Integer".to_string())
        );
        assert_eq!(
            rt(IS_BEFORE, &["Instant", "Instant"]),
            Some("Boolean".to_string())
        );
        assert_eq!(
            rt(IS_AFTER, &["Instant", "Instant"]),
            Some("Boolean".to_string())
        );
        assert_eq!(
            rt(EQUALS, &["Instant", "Instant"]),
            Some("Boolean".to_string())
        );
        assert_eq!(rt(NEGATE, &["Duration"]), Some("Duration".to_string()));
        assert_eq!(
            rt(PLUS, &["Duration", "Duration"]),
            Some("Duration".to_string())
        );
        assert_eq!(
            rt(MINUS, &["Duration", "Duration"]),
            Some("Duration".to_string())
        );
        assert_eq!(rt(WEEKDAY, &["DateTime"]), Some("Weekday".to_string()));
        assert_eq!(rt(DAY_OF_YEAR, &["DateTime"]), Some("Integer".to_string()));
        assert_eq!(rt(IS_LEAP_YEAR, &["Integer"]), Some("Boolean".to_string()));
        assert_eq!(
            rt(DAYS_IN_MONTH, &["Integer", "Integer"]),
            Some("Integer".to_string())
        );
        assert_eq!(
            rt(START_OF_DAY, &["DateTime"]),
            Some("DateTime".to_string())
        );
        assert_eq!(rt(TO_MILLIS, &["Instant"]), Some("Integer".to_string()));
        assert_eq!(rt(TO_NANOS, &["Instant"]), Some("Integer".to_string()));
        assert_eq!(rt(FROM_MILLIS, &["Integer"]), Some("Instant".to_string()));
        assert_eq!(
            rt(FORMAT, &["DateTime", "String"]),
            Some("String".to_string())
        );
        assert_eq!(
            rt(PARSE, &["String", "String"]),
            Some("DateTime".to_string())
        );
        assert_eq!(
            rt(PARSE, &["String", "String", "Zone"]),
            Some("DateTime".to_string())
        );
        assert_eq!(rt(TO_ISO, &["DateTime"]), Some("String".to_string()));
        assert_eq!(rt(PARSE_ISO, &["String"]), Some("DateTime".to_string()));
        assert_eq!(
            rt(FORMAT_DURATION, &["Duration"]),
            Some("String".to_string())
        );
        assert_eq!(rt(LOCAL_OFFSET, &["Integer"]), Some("Integer".to_string()));
    }

    #[test]
    fn resolve_rejects_wrong_types_and_unknown() {
        assert_eq!(rt(OFFSET_AT, &["Instant", "Zone"]), None);
        assert_eq!(rt(ADD, &["Instant", "Instant"]), None);
        assert_eq!(rt(PARSE, &["String"]), None);
        assert_eq!(rt(FROM_MILLIS, &["String"]), None);
        assert_eq!(rt("datetime.nope", &[]), None);
    }

    #[test]
    fn expected_arguments_table() {
        assert_eq!(expected_arguments(NOW), Some("()"));
        assert_eq!(expected_arguments(INSTANT), Some("1 to 5 Integer"));
        assert_eq!(expected_arguments(DATE), Some("Integer, Integer, Integer"));
        assert_eq!(
            expected_arguments(TIME),
            Some("Integer, Integer[, Integer[, Integer]]")
        );
        assert_eq!(expected_arguments(FIXED_OFFSET), Some("Integer[, Integer]"));
        assert_eq!(expected_arguments(OFFSET_AT), Some("Zone, Instant"));
        assert_eq!(expected_arguments(IN_ZONE), Some("Instant, Zone"));
        assert_eq!(expected_arguments(TO_UTC), Some("Instant"));
        assert_eq!(expected_arguments(RESOLVE), Some("DateTime"));
        assert_eq!(expected_arguments(CIVIL), Some("Date, Time, Zone"));
        assert_eq!(expected_arguments(WITH_ZONE), Some("DateTime, Zone"));
        assert_eq!(expected_arguments(ADD), Some("Instant, Duration"));
        assert_eq!(expected_arguments(BETWEEN), Some("Instant, Instant"));
        assert_eq!(expected_arguments(ADD_DAYS), Some("DateTime, Integer"));
        assert_eq!(expected_arguments(NEGATE), Some("Duration"));
        assert_eq!(expected_arguments(PLUS), Some("Duration, Duration"));
        assert_eq!(expected_arguments(IS_LEAP_YEAR), Some("Integer"));
        assert_eq!(expected_arguments(DAYS_IN_MONTH), Some("Integer, Integer"));
        assert_eq!(expected_arguments(TO_MILLIS), Some("Instant"));
        assert_eq!(expected_arguments(FORMAT), Some("DateTime, String"));
        assert_eq!(expected_arguments(PARSE), Some("String, String[, Zone]"));
        assert_eq!(expected_arguments(PARSE_ISO), Some("String"));
        assert_eq!(expected_arguments(FORMAT_DURATION), Some("Duration"));
        // NOW_NANOS/MONOTONIC_NANOS map to "()"; LOCAL_OFFSET to "Integer".
        assert_eq!(expected_arguments(NOW_NANOS), Some("()"));
        assert_eq!(expected_arguments(LOCAL_OFFSET), Some("Integer"));
        assert!(expected_arguments("datetime.nope").is_none());
    }

    #[test]
    fn arity_table() {
        assert_eq!(arity(NOW), Some((0, 0)));
        assert_eq!(arity(INSTANT), Some((1, 5)));
        assert_eq!(arity(FIXED_OFFSET), Some((1, 2)));
        assert_eq!(arity(TIME), Some((2, 4)));
        assert_eq!(arity(PARSE), Some((2, 3)));
        assert_eq!(arity(DATE), Some((3, 3)));
        assert_eq!(arity(CIVIL), Some((3, 3)));
        assert_eq!(arity(DAYS_IN_MONTH), Some((2, 2)));
        assert_eq!(arity(FORMAT), Some((2, 2)));
        assert_eq!(arity(TO_UTC), Some((1, 1)));
        assert_eq!(arity(NEGATE), Some((1, 1)));
        assert_eq!(arity(NOW_NANOS), Some((0, 0)));
        assert_eq!(arity(LOCAL_OFFSET), Some((1, 1)));
        assert!(arity("datetime.nope").is_none());
    }

    #[test]
    fn implementation_name_mapping() {
        assert_eq!(
            implementation_name(INSTANT, 3),
            Some("__datetime_instant3".to_string())
        );
        assert_eq!(
            implementation_name(DURATION, 2),
            Some("__datetime_duration2".to_string())
        );
        assert_eq!(
            implementation_name(FIXED_OFFSET, 1),
            Some("__datetime_fixedOffset1".to_string())
        );
        assert_eq!(
            implementation_name(PARSE, 3),
            Some("__datetime_parse3".to_string())
        );
        assert_eq!(
            implementation_name(NOW, 0),
            Some("__datetime_now".to_string())
        );
        assert_eq!(
            implementation_name(FORMAT_DURATION, 1),
            Some("__datetime_formatDuration".to_string())
        );
        // OS-seam intrinsics stay as runtime helpers -> None
        assert_eq!(implementation_name(NOW_NANOS, 0), None);
        assert_eq!(implementation_name(MONOTONIC_NANOS, 0), None);
        assert_eq!(implementation_name(LOCAL_OFFSET, 1), None);
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
    fn source_file_parses() {
        assert!(source_file().is_ok());
    }

    #[test]
    fn augmented_project_injects_when_imported() {
        let ast = project("IMPORT datetime\nSUB main\nEND SUB\n");
        assert!(uses_package(&ast));
        let augmented = augmented_project(&ast).expect("augment");
        assert_eq!(augmented.files.len(), ast.files.len() + 1);
    }

    #[test]
    fn augmented_project_noop_without_import() {
        let ast = project("SUB main\nEND SUB\n");
        assert!(!uses_package(&ast));
        assert_eq!(
            augmented_project(&ast).expect("a").files.len(),
            ast.files.len()
        );
    }
}
