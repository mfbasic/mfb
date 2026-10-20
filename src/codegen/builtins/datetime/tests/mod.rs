use crate::codegen::registry::{self, registry};
use crate::types::ParameterType;

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
    assert_eq!(
        registry::default_argument_padding(
            TIME,
            &vec![ParameterType::Unknown; 2],
            &crate::codegen::builtins::NoTypeKinds
        )
        .len(),
        2
    );
    assert_eq!(
        registry::default_argument_padding(
            TIME,
            &vec![ParameterType::Unknown; 3],
            &crate::codegen::builtins::NoTypeKinds
        )
        .len(),
        1
    );
    assert_eq!(
        registry::default_argument_padding(
            TIME,
            &vec![ParameterType::Unknown; 4],
            &crate::codegen::builtins::NoTypeKinds
        )
        .len(),
        0
    );
    assert_eq!(
        registry::default_argument_padding(
            TIME,
            &vec![ParameterType::Unknown; 5],
            &crate::codegen::builtins::NoTypeKinds
        )
        .len(),
        0
    );
    assert_eq!(
        registry::default_argument_padding(
            NOW,
            &vec![ParameterType::Unknown; 0],
            &crate::codegen::builtins::NoTypeKinds
        ),
        Vec::new()
    );
}

#[test]
fn builtin_types_recognized() {
    // The value records/enums are registry-modeled and
    // recognized through the generic registry via `add_record`.
    for t in [
        "Instant", "Duration", "Date", "Time", "Zone", "DateTime", "ZoneKind", "Weekday", "Month",
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
        (
            "datetime.format",
            &[&[INVALID_ARGUMENT, INVALID_FORMAT, "ErrOutOfMemory", OVERFLOW]],
        ),
        ("datetime.formatDuration", &[&["ErrOutOfMemory", OVERFLOW]]),
        (
            "datetime.toIso",
            &[
                &["ErrOutOfMemory", OVERFLOW],
                &[INVALID_ARGUMENT, "ErrOutOfMemory", OVERFLOW],
            ],
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
