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
    Body, DefaultValue, EnumVariant, Implementation, Parameter, Registry, RegistryEnum,
    RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

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

    // The public value RECORDS are authored (with their `DOC` blocks and byte-exact
    // field formatting) in `package.mfb`; recording their names as source-declared
    // types lets the generic `registry::is_builtin_type` / `qualified_builtin_type`
    // recognize them without double-declaring.
    pkg.add_source_types(&["Instant", "Duration", "Date", "Time", "Zone", "DateTime"]);

    // The public value ENUMS are modeled on the registry — `get_mfb` renders them into
    // the injected source in place of a hand-written `EXPORT ENUM` in `package.mfb`.
    pkg.add_enum(RegistryEnum {
        name: "ZoneKind",
        export: true,
        variants: vec![
            EnumVariant {
                name: "Utc",
                description: "Coordinated Universal Time (offset 0).",
            },
            EnumVariant {
                name: "FixedOffset",
                description: "A constant offset from UTC.",
            },
            EnumVariant {
                name: "Local",
                description: "The host system's local time zone.",
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
            },
            EnumVariant {
                name: "Tuesday",
                description: "Tuesday.",
            },
            EnumVariant {
                name: "Wednesday",
                description: "Wednesday.",
            },
            EnumVariant {
                name: "Thursday",
                description: "Thursday.",
            },
            EnumVariant {
                name: "Friday",
                description: "Friday.",
            },
            EnumVariant {
                name: "Saturday",
                description: "Saturday.",
            },
            EnumVariant {
                name: "Sunday",
                description: "Sunday.",
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
            },
            EnumVariant {
                name: "February",
                description: "February.",
            },
            EnumVariant {
                name: "March",
                description: "March.",
            },
            EnumVariant {
                name: "April",
                description: "April.",
            },
            EnumVariant {
                name: "May",
                description: "May.",
            },
            EnumVariant {
                name: "June",
                description: "June.",
            },
            EnumVariant {
                name: "July",
                description: "July.",
            },
            EnumVariant {
                name: "August",
                description: "August.",
            },
            EnumVariant {
                name: "September",
                description: "September.",
            },
            EnumVariant {
                name: "October",
                description: "October.",
            },
            EnumVariant {
                name: "November",
                description: "November.",
            },
            EnumVariant {
                name: "December",
                description: "December.",
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

// Man-page citation anchor: `DATETIME`. The ~50 `datetime/*` man pages ground their
// value-type and OS-seam facts in this package with `[[…/datetime/mod.rs:DATETIME]]`.
//
// The `datetime::` OS-seam intrinsics (`nowNanos` / `monotonicNanos` / `localOffset`,
// plan-01-datetime.md §8.2) are ordinary `Body::native` members (`func_now_nanos` et
// al.), so the shared runtime catalog DERIVES their specs from the registry
// (`registry::runtime_specs`) — no hand-written `RuntimeHelperSpec` consts here.

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
        assert_eq!(registry::default_argument_padding(TIME, 2).len(), 2);
        assert_eq!(registry::default_argument_padding(TIME, 3).len(), 1);
        assert_eq!(registry::default_argument_padding(TIME, 4).len(), 0);
        assert_eq!(registry::default_argument_padding(TIME, 5).len(), 0);
        assert_eq!(registry::default_argument_padding(NOW, 0), Vec::new());
    }

    #[test]
    fn builtin_types_recognized() {
        // The value records/enums are source-declared (in `package.mfb`) and
        // recognized through the generic registry via `add_source_types`.
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
            Some("Instant".to_string())
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
            registry::call_param_name_overloads(FIXED_OFFSET),
            Some(&[&["offsetSeconds"][..], &["hours", "mins"][..]][..])
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
}
