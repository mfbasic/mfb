//! Built-in `datetime::` package seam (plan-01-datetime.md).
//!
//! Mirrors `json`/`regex`: the portable calendar math, formatting, and parsing
//! live in `datetime_package.mfb` as internal `__datetime_*` functions; this
//! module owns registration, syntaxcheck metadata, and the mapping from a public
//! `datetime::` call onto its internal implementation. The only platform state
//! is reached through three intrinsics (`nowNanos`, `monotonicNanos`,
//! `localOffset`) that lower to libc runtime helpers (§8.2).

use std::borrow::Cow;

use crate::codegen::registry::{
    BuiltinFunction, BuiltinModule, BuiltinOverload, BuiltinResolver, BuiltinSource, BuiltinType,
    DefaultResolver, DefaultValue, InjectionRule, Parameter, ParameterType, ReturnType, TypeKind,
};

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

// Overload/parameter builders, shared by the per-member `func_*.rs` via `super::`.
const fn req(name: &'static str, ty: &'static str) -> Parameter {
    Parameter::required(name, ty)
}
const fn opt(name: &'static str, ty: &'static str, default: &'static str) -> Parameter {
    Parameter {
        name,
        aliases: &[],
        ty: ParameterType::Named(ty),
        default: DefaultValue::Fill {
            type_name: ty,
            expr: default,
        },
    }
}
const fn optn(name: &'static str, ty: &'static str) -> Parameter {
    Parameter {
        name,
        aliases: &[],
        ty: ParameterType::Named(ty),
        default: DefaultValue::Optional,
    }
}
const fn ov(params: &'static [Parameter], ret: &'static str) -> BuiltinOverload {
    BuiltinOverload {
        params,
        return_type: ReturnType::Fixed(ret),
    }
}

const I: &str = "Integer";
// The `instant`/`duration` arity overloads drop components off the front.
const DT_COMPONENTS: &[BuiltinOverload] = &[
    ov(&[req("seconds", I)], "Duration"),
    ov(&[req("seconds", I), req("nanos", I)], "Duration"),
    ov(
        &[req("mins", I), req("seconds", I), req("nanos", I)],
        "Duration",
    ),
    ov(
        &[
            req("hours", I),
            req("mins", I),
            req("seconds", I),
            req("nanos", I),
        ],
        "Duration",
    ),
    ov(
        &[
            req("days", I),
            req("hours", I),
            req("mins", I),
            req("seconds", I),
            req("nanos", I),
        ],
        "Duration",
    ),
];
const INSTANT_OVERLOADS: &[BuiltinOverload] = &[
    ov(&[req("seconds", I)], "Instant"),
    ov(&[req("seconds", I), req("nanos", I)], "Instant"),
    ov(
        &[req("mins", I), req("seconds", I), req("nanos", I)],
        "Instant",
    ),
    ov(
        &[
            req("hours", I),
            req("mins", I),
            req("seconds", I),
            req("nanos", I),
        ],
        "Instant",
    ),
    ov(
        &[
            req("days", I),
            req("hours", I),
            req("mins", I),
            req("seconds", I),
            req("nanos", I),
        ],
        "Instant",
    ),
];

const DATETIME_FUNCTIONS: &[BuiltinFunction] = &[
    func_now::NOW,
    func_monotonic::MONOTONIC,
    func_instant::INSTANT,
    func_date::DATE,
    func_time::TIME,
    func_duration::DURATION,
    func_utc::UTC,
    func_local::LOCAL,
    func_fixed_offset::FIXED_OFFSET,
    func_offset_at::OFFSET_AT,
    func_in_zone::IN_ZONE,
    func_to_utc::TO_UTC,
    func_to_local::TO_LOCAL,
    func_resolve::RESOLVE,
    func_civil::CIVIL,
    func_with_zone::WITH_ZONE,
    func_add::ADD,
    func_subtract::SUBTRACT,
    func_between::BETWEEN,
    func_add_days::ADD_DAYS,
    func_add_months::ADD_MONTHS,
    func_compare::COMPARE,
    func_is_before::IS_BEFORE,
    func_is_after::IS_AFTER,
    func_equals::EQUALS,
    func_negate::NEGATE,
    func_plus::PLUS,
    func_minus::MINUS,
    func_weekday::WEEKDAY,
    func_day_of_year::DAY_OF_YEAR,
    func_is_leap_year::IS_LEAP_YEAR,
    func_days_in_month::DAYS_IN_MONTH,
    func_start_of_day::START_OF_DAY,
    func_to_millis::TO_MILLIS,
    func_to_nanos::TO_NANOS,
    func_from_millis::FROM_MILLIS,
    func_format::FORMAT,
    func_parse::PARSE,
    func_to_iso::TO_ISO,
    func_parse_iso::PARSE_ISO,
    func_format_duration::FORMAT_DURATION,
    func_now_nanos::NOW_NANOS,
    func_monotonic_nanos::MONOTONIC_NANOS,
    func_local_offset::LOCAL_OFFSET,
];

const DATETIME_TYPES: &[BuiltinType] = &[
    BuiltinType {
        name: "Instant",
        kind: TypeKind::Record,
        fields: &[],
    },
    BuiltinType {
        name: "Duration",
        kind: TypeKind::Record,
        fields: &[],
    },
    BuiltinType {
        name: "Date",
        kind: TypeKind::Record,
        fields: &[],
    },
    BuiltinType {
        name: "Time",
        kind: TypeKind::Record,
        fields: &[],
    },
    BuiltinType {
        name: "Zone",
        kind: TypeKind::Record,
        fields: &[],
    },
    BuiltinType {
        name: "DateTime",
        kind: TypeKind::Record,
        fields: &[],
    },
    BuiltinType {
        name: "ZoneKind",
        kind: TypeKind::Enum,
        fields: &[],
    },
    BuiltinType {
        name: "Weekday",
        kind: TypeKind::Enum,
        fields: &[],
    },
    BuiltinType {
        name: "Month",
        kind: TypeKind::Enum,
        fields: &[],
    },
];

/// Argument-dependent resolution for datetime: `resolve_call` argument validation
/// and the arity-keyed `__datetime_*{argc}` implementation selection. Both
/// delegate to the retained `dispatch_*` helpers (`implementation_name` reads only
/// the argument COUNT, so the resolver forwards `arg_types.len()`).
struct DatetimeResolver;
impl BuiltinResolver for DatetimeResolver {
    fn resolve_return_type(
        &self,
        _module: &BuiltinModule,
        name: &str,
        arg_types: &[String],
    ) -> Option<String> {
        dispatch_resolve(name, arg_types).map(|resolved| resolved.return_type.into_owned())
    }

    fn implementation_name(
        &self,
        _module: &BuiltinModule,
        name: &str,
        arg_types: &[String],
    ) -> Option<String> {
        dispatch_implementation_name(name, arg_types.len())
    }
}
static DATETIME_RESOLVER: DatetimeResolver = DatetimeResolver;

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

pub(crate) static DATETIME: BuiltinModule = BuiltinModule {
    name: "datetime",
    doc_intro: MODULE_INTRO,
    doc_desc: MODULE_DESC,
    functions: DATETIME_FUNCTIONS,
    types: DATETIME_TYPES,
    source: Some(BuiltinSource {
        rule: InjectionRule::WhenImported,
        loader: source_file,
    }),
    resolver: Some(&DATETIME_RESOLVER),
};

#[derive(Clone)]
pub(crate) struct ResolvedCall<'a> {
    pub(crate) return_type: Cow<'a, str>,
}

/// The public copyable record/enum types defined in `datetime_package.mfb`.
/// Referenced bare (`Instant`, `DateTime`, …) like every other builtin type.
pub(crate) fn is_builtin_type(name: &str) -> bool {
    DATETIME.types.iter().any(|ty| ty.name == name)
}

pub(crate) fn is_datetime_call(name: &str) -> bool {
    DefaultResolver::contains(&DATETIME, name)
}

/// The expected-argument phrasing for a `datetime::` argument-mismatch diagnostic.
/// Kept hand-authored (plan-72-BB): the optional-argument `[...]` brackets
/// (`time`'s `"Integer, Integer[, Integer[, Integer]]"`, `parse`'s
/// `"String, String[, Zone]"`) and the range prose (`"1 to 5 Integer"`) are shapes
/// the descriptor's per-position type join cannot reproduce, so `builtins::expected_arguments`
/// reads this before falling back to `DefaultResolver`.
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

// `call_param_names`/`call_param_name_overloads` return `&'static` borrowed
// shapes the owned `DefaultResolver` cannot produce; they stay static, PINNED
// equal to `DATETIME` by the parity test (`DefaultResolver::param_names`/
// `param_name_overloads` derive them — None for multi-overload / single-overload
// respectively, exactly the bug-349 split). `expected_arguments`/`argument_types`
// use bespoke phrasing and stay static. BB removes them.
pub(crate) fn call_param_names(name: &str) -> Option<&'static [&'static [&'static str]]> {
    let params: &'static [&'static [&'static str]] = match name {
        NOW | MONOTONIC | UTC | LOCAL => &[],
        // INSTANT/DURATION drop components off the FRONT as arity falls
        // (`__datetime_instant1(seconds)` … `__datetime_instant5(days, hours,
        // mins, seconds, nanos)`, `datetime_package.mfb:113-129, 137-153`), so
        // position 0 is `seconds` at arity 1 and `days` only at arity 5. A
        // merged per-position table is positionally false at arities 1-4 — it
        // bound `instant(days := 5)` to the 1-arg `seconds` slot, i.e. 5
        // seconds rather than 5 days (bug-349, same class as bug-94). They use
        // a per-overload table instead; see `call_param_name_overloads`.
        INSTANT | DURATION => return None,
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

/// The argument-validating return-type resolution, invoked through the descriptor
/// resolver by `resolve_call`. The component builders accept 1..=5 (or 1..=2)
/// `Integer` args; the others require exact typed signatures.
fn dispatch_resolve<'a>(name: &str, arg_types: &'a [String]) -> Option<ResolvedCall<'a>> {
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

/// The machine-readable positional argument-type signature (bug-340 A1): the
/// concrete per-parameter types IR lowering hands to `call_argument_expected_type`,
/// read directly instead of parsing the `expected_arguments` diagnostic string.
/// The variable-arity constructors (`instant`/`duration`), the no-argument clocks,
/// and the optional-tail members (`time`/`fixedOffset`/`parse`) have no single
/// fixed positional signature, so they return `None` (as the string parse's bail
/// conditions did before this package was wired in).
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

/// The internal `__datetime_*` implementation for a public call, given the
/// supplied argument count. Routes through the descriptor resolver (which reads
/// only the count). Returns `None` for the OS-seam intrinsics (runtime helpers).
pub(crate) fn implementation_name(name: &str, argc: usize) -> Option<String> {
    // The resolver hook takes argument TYPES, but datetime's selection depends
    // only on the COUNT, so pass a length-`argc` placeholder.
    let arg_types = vec![String::new(); argc];
    DATETIME
        .resolver?
        .implementation_name(&DATETIME, name, &arg_types)
}

/// The arity-keyed `__datetime_*{argc}` implementation selection, invoked through
/// the descriptor resolver by `implementation_name`.
fn dispatch_implementation_name(name: &str, argc: usize) -> Option<String> {
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
/// `.mfb` overload (§5.1.1). Returns a `&'static` borrowed slice, so it stays
/// static — PINNED equal to `time`'s optional parameters by the parity test
/// (`DefaultResolver::default_padding` derives the same slots).
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

use crate::builtins::exact;
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

/// Parses the built-in `datetime` package source. The 37 single-body members carry
/// their `FUNC __datetime_<slug> ... END FUNC` body in their `func_*.rs`
/// (`Implementation::Mfb`), spliced in by [`assembled_source`]; the 4 arity members
/// (`instant`/`duration`/`fixedOffset`/`parse`, which map one-to-many onto
/// `__datetime_*<n>` bodies that `Mfb` cannot hold) and every private helper stay
/// in `package.mfb`. Synthetic path label preserved byte-for-byte from the
/// pre-migration `package_source_glue!`.
pub(crate) fn source_file() -> Result<crate::ast::AstFile, ()> {
    crate::ast::parse_source_internal(
        std::path::Path::new("<builtin-datetime>"),
        "builtins/datetime.mfb",
        &assembled_source(),
    )
}

/// The `datetime` package source: `package.mfb` (arity-member bodies + private
/// helpers) with each `Implementation::Mfb` member's body spliced in for its
/// `'@@MFB_BODY:<slug>@@` marker at the body's original position, keeping every
/// other line's number unchanged so the injected AST is byte-identical to the
/// pre-migration companion.
fn assembled_source() -> String {
    let mut source = String::from(include_str!("package.mfb"));
    for func in DATETIME_FUNCTIONS {
        if let crate::codegen::registry::Implementation::Mfb { body, .. } = func.implementation {
            let marker = format!("'@@MFB_BODY:{}@@", func.doc_slug);
            debug_assert!(
                source.contains(&marker),
                "datetime package.mfb is missing the '{marker}' body marker",
            );
            source = source.replacen(&marker, body, 1);
        }
    }
    source
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

#[cfg(test)]
mod tests {
    use super::*;

    fn project(src: &str) -> crate::ast::AstProject {
        let file = crate::ast::parse_source(std::path::Path::new("main.mfb"), "main.mfb", src)
            .expect("parse source");
        crate::ast::AstProject {
            name: "test".to_string(),
            files: vec![file],
        }
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
        // INSTANT/DURATION have no merged per-position table either: their
        // overloads drop components off the FRONT, so position 0 is `seconds`
        // at arity 1 and `days` only at arity 5. The merged table this line
        // used to assert bound `instant(days := 5)` to the 1-arg `seconds`
        // slot — 5 seconds, not 5 days (bug-349, the sibling bug-94 missed).
        assert_eq!(call_param_names(INSTANT), None);
        assert_eq!(call_param_names(DURATION), None);
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
    fn argument_types_machine_table() {
        // bug-340 A1: the concrete positional signatures IR lowering reads. The
        // no-argument clocks, the variadic constructors (`instant`/`duration`),
        // and the optional-tail members (`time`/`fixedOffset`/`parse`) have no
        // single fixed signature -> None.
        assert_eq!(
            argument_types(DATE),
            Some(&["Integer", "Integer", "Integer"][..])
        );
        assert_eq!(argument_types(OFFSET_AT), Some(&["Zone", "Instant"][..]));
        assert_eq!(argument_types(CIVIL), Some(&["Date", "Time", "Zone"][..]));
        assert_eq!(argument_types(FORMAT), Some(&["DateTime", "String"][..]));
        assert_eq!(argument_types(PARSE_ISO), Some(&["String"][..]));
        // No fixed positional signature:
        assert_eq!(argument_types(NOW), None); // no arguments
        assert_eq!(argument_types(INSTANT), None); // 1..=5 Integer (variadic)
        assert_eq!(argument_types(TIME), None); // optional tail
        assert_eq!(argument_types(PARSE), None); // optional Zone
        assert_eq!(argument_types("datetime.nope"), None);
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

    /// The parameter names of `FUNC <func>(...)` as written in the assembled
    /// package source — the ground truth a param-name table must match. Reads
    /// `assembled_source()` (not the raw `package.mfb`) because the single-body
    /// members' `FUNC`s are now `Implementation::Mfb` bodies spliced in over their
    /// markers.
    fn mfb_param_names(func: &str) -> Vec<String> {
        let source = assembled_source();
        let prefix = format!("FUNC {func}(");
        let rest = source
            .lines()
            .find_map(|line| line.trim_start().strip_prefix(&prefix).map(str::to_string))
            .unwrap_or_else(|| panic!("`{func}` is not declared in datetime_package.mfb"));
        let close = rest.find(')').expect("parameter list closes");
        if rest[..close].trim().is_empty() {
            return Vec::new();
        }
        crate::builtins::split_top_level_commas(&rest[..close])
            .into_iter()
            .map(|param| {
                param
                    .split_whitespace()
                    .next()
                    .expect("a parameter has a name")
                    .to_string()
            })
            .collect()
    }

    #[test]
    fn arity_dispatched_param_tables_match_the_mfb_overloads() {
        // bug-349: a param-name table must be positionally TRUE against the
        // overload the call actually selects, not merely unambiguous.
        // `no_named_argument_alias_repeats_across_positions` only rules out an
        // alias appearing twice; INSTANT/DURATION passed it while binding
        // `instant(days := 5)` to the 1-arg `seconds` slot — 5 seconds, not 5
        // days — because their overloads drop components off the FRONT.
        //
        // The rule this asserts: for every arity-dispatched family, the names
        // declared for arity N must equal `__datetime_<name>{N}`'s actual
        // parameters. A merged table is only legal when the family is
        // leading-aligned (each shorter overload is a prefix of the longer).
        for (builtin, stem, arities) in [
            (INSTANT, "__datetime_instant", 1..=5),
            (DURATION, "__datetime_duration", 1..=5),
            (FIXED_OFFSET, "__datetime_fixedOffset", 1..=2),
            (PARSE, "__datetime_parse", 2..=3),
        ] {
            for argc in arities {
                let actual = mfb_param_names(&format!("{stem}{argc}"));
                assert_eq!(
                    actual.len(),
                    argc,
                    "`{stem}{argc}` should take {argc} parameters"
                );

                // Whichever table the builtin declares, resolve the names it
                // claims sit at positions 0..argc for this arity.
                let declared: Vec<String> = if let Some(overloads) =
                    call_param_name_overloads(builtin)
                {
                    let params = overloads
                        .iter()
                        .find(|params| params.len() == argc)
                        .unwrap_or_else(|| panic!("`{builtin}` declares no arity-{argc} overload"));
                    params.iter().map(|p| (*p).to_string()).collect()
                } else {
                    let merged = call_param_names(builtin)
                        .unwrap_or_else(|| panic!("`{builtin}` declares no param names at all"));
                    // A merged table names position i once, for every arity.
                    merged
                        .iter()
                        .take(argc)
                        .map(|group| group[0].to_string())
                        .collect()
                };

                assert_eq!(
                    declared, actual,
                    "`{builtin}` at arity {argc} names its parameters {declared:?}, but \
                     `{stem}{argc}` takes {actual:?} — a named argument would bind to the \
                     wrong slot (bug-349)"
                );
            }
        }
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

    /// The `const fn` descriptor constructors (`req`/`opt`/`optn`/`ov`/`df`) are
    /// only invoked in `const` context by `DATETIME_FUNCTIONS`, so they carry no
    /// runtime coverage. Call each at runtime and assert the fields it builds.
    #[test]
    fn const_constructors_build_expected_fields() {
        let r = req("year", "Integer");
        assert_eq!(r.name, "year");
        assert!(r.aliases.is_empty());
        assert_eq!(r.ty, ParameterType::Named("Integer"));
        assert_eq!(r.default, DefaultValue::None);

        // `opt` is default-PADDED (`time`'s trailing `second`/`nanos` -> "0").
        let o = opt("second", "Integer", "0");
        assert_eq!(o.name, "second");
        assert!(o.aliases.is_empty());
        assert_eq!(o.ty, ParameterType::Named("Integer"));
        assert_eq!(
            o.default,
            DefaultValue::Fill {
                type_name: "Integer",
                expr: "0"
            }
        );

        // `optn` widens arity but is NOT padded (`parse`'s trailing `zone`).
        let n = optn("zone", "Zone");
        assert_eq!(n.name, "zone");
        assert_eq!(n.ty, ParameterType::Named("Zone"));
        assert_eq!(n.default, DefaultValue::Optional);

        // `ov` takes a `&'static [Parameter]`; a runtime temporary would be E0716,
        // so the parameter slice is a `const`.
        const PARAMS: &[Parameter] = &[req("year", "Integer")];
        let overload = ov(PARAMS, "Date");
        assert_eq!(overload.return_type, ReturnType::Fixed("Date"));
        assert_eq!(overload.params.len(), 1);
        assert_eq!(overload.params[0].name, "year");

        // Members are declared with the registry-wide `BuiltinFunction::custom`
        // (each in its own `func_*.rs`); exercise it here with a `const` overload
        // slice (a runtime temporary would be E0716).
        const OVS: &[BuiltinOverload] = &[ov(&[req("year", "Integer")], "Date")];
        let f = BuiltinFunction::custom("datetime.date", "date", "", "", &[], OVS);
        assert_eq!(f.name, "datetime.date");
        assert_eq!(f.doc_slug, "date");
        assert_eq!(f.overloads.len(), 1);
        use crate::codegen::registry::{BuiltinFlags, Implementation, Lowering};
        assert_eq!(f.implementation, Implementation::Custom);
        assert_eq!(f.lowering, Lowering::Helper);
        assert_eq!(f.flags, BuiltinFlags::default());
        // A migrated member descriptor lives in its func file.
        assert_eq!(func_date::DATE.name, DATE);
    }

    #[test]
    fn expected_arguments_every_arm() {
        // Zero-argument members render as "()".
        for n in [NOW, MONOTONIC, UTC, LOCAL, NOW_NANOS, MONOTONIC_NANOS] {
            assert_eq!(expected_arguments(n), Some("()"), "{n}");
        }
        assert_eq!(expected_arguments(INSTANT), Some("1 to 5 Integer"));
        assert_eq!(expected_arguments(DURATION), Some("1 to 5 Integer"));
        assert_eq!(expected_arguments(DATE), Some("Integer, Integer, Integer"));
        assert_eq!(
            expected_arguments(TIME),
            Some("Integer, Integer[, Integer[, Integer]]")
        );
        assert_eq!(expected_arguments(FIXED_OFFSET), Some("Integer[, Integer]"));
        assert_eq!(expected_arguments(OFFSET_AT), Some("Zone, Instant"));
        assert_eq!(expected_arguments(IN_ZONE), Some("Instant, Zone"));
        assert_eq!(expected_arguments(TO_UTC), Some("Instant"));
        assert_eq!(expected_arguments(TO_LOCAL), Some("Instant"));
        for n in [RESOLVE, WEEKDAY, DAY_OF_YEAR, START_OF_DAY, TO_ISO] {
            assert_eq!(expected_arguments(n), Some("DateTime"), "{n}");
        }
        assert_eq!(expected_arguments(CIVIL), Some("Date, Time, Zone"));
        assert_eq!(expected_arguments(WITH_ZONE), Some("DateTime, Zone"));
        assert_eq!(expected_arguments(ADD), Some("Instant, Duration"));
        assert_eq!(expected_arguments(SUBTRACT), Some("Instant, Duration"));
        for n in [BETWEEN, COMPARE, IS_BEFORE, IS_AFTER, EQUALS] {
            assert_eq!(expected_arguments(n), Some("Instant, Instant"), "{n}");
        }
        assert_eq!(expected_arguments(ADD_DAYS), Some("DateTime, Integer"));
        assert_eq!(expected_arguments(ADD_MONTHS), Some("DateTime, Integer"));
        assert_eq!(expected_arguments(NEGATE), Some("Duration"));
        assert_eq!(expected_arguments(PLUS), Some("Duration, Duration"));
        assert_eq!(expected_arguments(MINUS), Some("Duration, Duration"));
        for n in [IS_LEAP_YEAR, FROM_MILLIS, LOCAL_OFFSET] {
            assert_eq!(expected_arguments(n), Some("Integer"), "{n}");
        }
        assert_eq!(expected_arguments(DAYS_IN_MONTH), Some("Integer, Integer"));
        assert_eq!(expected_arguments(TO_MILLIS), Some("Instant"));
        assert_eq!(expected_arguments(TO_NANOS), Some("Instant"));
        assert_eq!(expected_arguments(FORMAT), Some("DateTime, String"));
        assert_eq!(expected_arguments(PARSE), Some("String, String[, Zone]"));
        assert_eq!(expected_arguments(PARSE_ISO), Some("String"));
        assert_eq!(expected_arguments(FORMAT_DURATION), Some("Duration"));
        assert_eq!(expected_arguments("datetime.nope"), None);
    }

    fn resolved(name: &str, args: &[&str]) -> Option<String> {
        let types: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        dispatch_resolve(name, &types).map(|r| r.return_type.into_owned())
    }

    #[test]
    fn dispatch_resolve_every_branch() {
        assert_eq!(resolved(NOW, &[]), Some("Instant".into()));
        assert_eq!(resolved(MONOTONIC, &[]), Some("Duration".into()));
        assert_eq!(resolved(UTC, &[]), Some("Zone".into()));
        assert_eq!(resolved(LOCAL, &[]), Some("Zone".into()));
        assert_eq!(resolved(NOW_NANOS, &[]), Some("Integer".into()));
        assert_eq!(resolved(MONOTONIC_NANOS, &[]), Some("Integer".into()));
        // Component builders accept 1..=5 (fixedOffset 1..=2) Integer args.
        assert_eq!(resolved(INSTANT, &["Integer"]), Some("Instant".into()));
        assert_eq!(
            resolved(
                INSTANT,
                &["Integer", "Integer", "Integer", "Integer", "Integer"]
            ),
            Some("Instant".into())
        );
        assert_eq!(
            resolved(DURATION, &["Integer", "Integer"]),
            Some("Duration".into())
        );
        assert_eq!(resolved(FIXED_OFFSET, &["Integer"]), Some("Zone".into()));
        assert_eq!(
            resolved(FIXED_OFFSET, &["Integer", "Integer"]),
            Some("Zone".into())
        );
        assert_eq!(
            resolved(DATE, &["Integer", "Integer", "Integer"]),
            Some("Date".into())
        );
        assert_eq!(resolved(TIME, &["Integer", "Integer"]), Some("Time".into()));
        assert_eq!(
            resolved(TIME, &["Integer", "Integer", "Integer", "Integer"]),
            Some("Time".into())
        );
        assert_eq!(
            resolved(OFFSET_AT, &["Zone", "Instant"]),
            Some("Integer".into())
        );
        assert_eq!(
            resolved(IN_ZONE, &["Instant", "Zone"]),
            Some("DateTime".into())
        );
        assert_eq!(resolved(TO_UTC, &["Instant"]), Some("DateTime".into()));
        assert_eq!(resolved(TO_LOCAL, &["Instant"]), Some("DateTime".into()));
        assert_eq!(resolved(RESOLVE, &["DateTime"]), Some("Instant".into()));
        assert_eq!(
            resolved(CIVIL, &["Date", "Time", "Zone"]),
            Some("DateTime".into())
        );
        assert_eq!(
            resolved(WITH_ZONE, &["DateTime", "Zone"]),
            Some("DateTime".into())
        );
        assert_eq!(
            resolved(ADD, &["Instant", "Duration"]),
            Some("Instant".into())
        );
        assert_eq!(
            resolved(SUBTRACT, &["Instant", "Duration"]),
            Some("Instant".into())
        );
        assert_eq!(
            resolved(BETWEEN, &["Instant", "Instant"]),
            Some("Duration".into())
        );
        assert_eq!(
            resolved(ADD_DAYS, &["DateTime", "Integer"]),
            Some("DateTime".into())
        );
        assert_eq!(
            resolved(ADD_MONTHS, &["DateTime", "Integer"]),
            Some("DateTime".into())
        );
        assert_eq!(
            resolved(COMPARE, &["Instant", "Instant"]),
            Some("Integer".into())
        );
        assert_eq!(
            resolved(IS_BEFORE, &["Instant", "Instant"]),
            Some("Boolean".into())
        );
        assert_eq!(
            resolved(IS_AFTER, &["Instant", "Instant"]),
            Some("Boolean".into())
        );
        assert_eq!(
            resolved(EQUALS, &["Instant", "Instant"]),
            Some("Boolean".into())
        );
        assert_eq!(resolved(NEGATE, &["Duration"]), Some("Duration".into()));
        assert_eq!(
            resolved(PLUS, &["Duration", "Duration"]),
            Some("Duration".into())
        );
        assert_eq!(
            resolved(MINUS, &["Duration", "Duration"]),
            Some("Duration".into())
        );
        assert_eq!(resolved(WEEKDAY, &["DateTime"]), Some("Weekday".into()));
        assert_eq!(resolved(DAY_OF_YEAR, &["DateTime"]), Some("Integer".into()));
        assert_eq!(resolved(IS_LEAP_YEAR, &["Integer"]), Some("Boolean".into()));
        assert_eq!(
            resolved(DAYS_IN_MONTH, &["Integer", "Integer"]),
            Some("Integer".into())
        );
        assert_eq!(
            resolved(START_OF_DAY, &["DateTime"]),
            Some("DateTime".into())
        );
        assert_eq!(resolved(TO_MILLIS, &["Instant"]), Some("Integer".into()));
        assert_eq!(resolved(TO_NANOS, &["Instant"]), Some("Integer".into()));
        assert_eq!(resolved(FROM_MILLIS, &["Integer"]), Some("Instant".into()));
        assert_eq!(
            resolved(FORMAT, &["DateTime", "String"]),
            Some("String".into())
        );
        assert_eq!(
            resolved(PARSE, &["String", "String"]),
            Some("DateTime".into())
        );
        assert_eq!(
            resolved(PARSE, &["String", "String", "Zone"]),
            Some("DateTime".into())
        );
        assert_eq!(resolved(TO_ISO, &["DateTime"]), Some("String".into()));
        assert_eq!(resolved(PARSE_ISO, &["String"]), Some("DateTime".into()));
        assert_eq!(
            resolved(FORMAT_DURATION, &["Duration"]),
            Some("String".into())
        );
        assert_eq!(resolved(LOCAL_OFFSET, &["Integer"]), Some("Integer".into()));
        // Mismatched arity / types / unknown name -> None.
        assert_eq!(resolved(DATE, &["Integer", "Integer"]), None);
        assert_eq!(resolved(INSTANT, &["String"]), None);
        assert_eq!(resolved(OFFSET_AT, &["Instant", "Zone"]), None);
        assert_eq!(resolved("datetime.nope", &[]), None);
    }

    #[test]
    fn argument_types_remaining_arms() {
        assert_eq!(argument_types(IN_ZONE), Some(&["Instant", "Zone"][..]));
        for n in [TO_UTC, TO_LOCAL, TO_MILLIS, TO_NANOS] {
            assert_eq!(argument_types(n), Some(&["Instant"][..]), "{n}");
        }
        for n in [RESOLVE, WEEKDAY, DAY_OF_YEAR, START_OF_DAY, TO_ISO] {
            assert_eq!(argument_types(n), Some(&["DateTime"][..]), "{n}");
        }
        assert_eq!(argument_types(WITH_ZONE), Some(&["DateTime", "Zone"][..]));
        for n in [ADD, SUBTRACT] {
            assert_eq!(argument_types(n), Some(&["Instant", "Duration"][..]), "{n}");
        }
        for n in [BETWEEN, COMPARE, IS_BEFORE, IS_AFTER, EQUALS] {
            assert_eq!(argument_types(n), Some(&["Instant", "Instant"][..]), "{n}");
        }
        for n in [ADD_DAYS, ADD_MONTHS] {
            assert_eq!(argument_types(n), Some(&["DateTime", "Integer"][..]), "{n}");
        }
        assert_eq!(argument_types(NEGATE), Some(&["Duration"][..]));
        assert_eq!(argument_types(FORMAT_DURATION), Some(&["Duration"][..]));
        for n in [PLUS, MINUS] {
            assert_eq!(
                argument_types(n),
                Some(&["Duration", "Duration"][..]),
                "{n}"
            );
        }
        for n in [IS_LEAP_YEAR, FROM_MILLIS, LOCAL_OFFSET] {
            assert_eq!(argument_types(n), Some(&["Integer"][..]), "{n}");
        }
        assert_eq!(
            argument_types(DAYS_IN_MONTH),
            Some(&["Integer", "Integer"][..])
        );
    }

    #[test]
    fn implementation_name_routes_and_no_arg_func_has_no_params() {
        // Non-intrinsic call routes through the descriptor resolver.
        assert_eq!(
            implementation_name(NOW, 0),
            Some("__datetime_now".to_string())
        );
        // OS-seam intrinsic -> runtime helper, so None.
        assert_eq!(implementation_name(LOCAL_OFFSET, 1), None);
    }
}
