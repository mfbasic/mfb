//! plan-179-A: the `options.maxArenaMemory*` reader and validator.

use super::*;

fn manifest(options: &str) -> HashMap<String, JsonValue> {
    let parsed: JsonValue = format!("{{\"options\": {options}}}")
        .parse()
        .expect("valid JSON");
    parsed
        .get::<HashMap<String, JsonValue>>()
        .expect("object")
        .clone()
}

/// Run the validator on `options`, returning its verdict and the rules it raised.
fn validate(options: &str) -> (bool, Vec<(String, String)>) {
    let contents = format!("{{\"options\": {options}}}");
    let parsed = manifest(options);
    rules::collecting_diagnostics(|| {
        validate_arena_memory(&parsed, Path::new("project.json"), &contents)
    })
}

#[test]
fn absent_keys_read_as_the_defaults() {
    let expected = ArenaMemoryLimits {
        disable: false,
        percent: 50,
        mib: -1,
        check: ArenaMemoryCheck::Halt,
    };
    assert_eq!(ArenaMemoryLimits::default(), expected);
    assert_eq!(arena_memory_limits(&HashMap::new()), expected);
    assert_eq!(arena_memory_limits(&manifest("{}")), expected);
    assert_eq!(validate("{}"), (true, Vec::new()));
}

#[test]
fn valid_values_are_read_as_written_with_no_diagnostic() {
    let max = MAX_ARENA_MEMORY_MIB;
    assert_eq!(max, (1u64 << 44) - 1);
    for (options, limits) in [
        (
            "{\"maxArenaMemoryPercent\": 1, \"maxArenaMemoryCheck\": \"warn\"}".to_string(),
            (false, 1, -1, ArenaMemoryCheck::Warn),
        ),
        (
            "{\"maxArenaMemoryPercent\": 100, \"maxArenaMemoryMiB\": 1}".to_string(),
            (false, 100, 1, ArenaMemoryCheck::Halt),
        ),
        (
            "{\"maxArenaMemoryMiB\": -1, \"maxArenaMemoryCheck\": \"halt\"}".to_string(),
            (false, 50, -1, ArenaMemoryCheck::Halt),
        ),
        (
            format!("{{\"maxArenaMemoryMiB\": {max}}}"),
            (false, 50, max as i64, ArenaMemoryCheck::Halt),
        ),
        (
            "{\"maxArenaMemoryDisable\": true}".to_string(),
            (true, 50, -1, ArenaMemoryCheck::Halt),
        ),
        (
            "{\"maxArenaMemoryDisable\": false, \"maxArenaMemoryMiB\": 4096}".to_string(),
            (false, 50, 4096, ArenaMemoryCheck::Halt),
        ),
    ] {
        let (disable, percent, mib, check) = limits;
        assert_eq!(
            arena_memory_limits(&manifest(&options)),
            ArenaMemoryLimits {
                disable,
                percent,
                mib,
                check
            },
            "{options}"
        );
        assert_eq!(validate(&options), (true, Vec::new()), "{options}");
    }
}

/// Every invalid value fails validation with one `PROJECT_JSON_FIELD_TYPE` that
/// names its key, and the reader falls back to that key's default rather than
/// returning a value the validator rejects (a fraction is not truncated).
#[test]
fn invalid_values_are_rejected_naming_the_key_and_read_as_the_default() {
    let max_plus_one = MAX_ARENA_MEMORY_MIB + 1;
    let defaults = ArenaMemoryLimits::default();
    for (key, value) in [
        ("maxArenaMemoryDisable", "\"yes\"".to_string()),
        ("maxArenaMemoryDisable", "1".to_string()),
        ("maxArenaMemoryPercent", "0".to_string()),
        ("maxArenaMemoryPercent", "101".to_string()),
        ("maxArenaMemoryPercent", "12.5".to_string()),
        ("maxArenaMemoryPercent", "-1".to_string()),
        ("maxArenaMemoryPercent", "\"50\"".to_string()),
        ("maxArenaMemoryMiB", "0".to_string()),
        ("maxArenaMemoryMiB", "-2".to_string()),
        ("maxArenaMemoryMiB", "2.5".to_string()),
        ("maxArenaMemoryMiB", "-1.5".to_string()),
        ("maxArenaMemoryMiB", max_plus_one.to_string()),
        ("maxArenaMemoryMiB", "1e30".to_string()),
        ("maxArenaMemoryMiB", "\"4096\"".to_string()),
        ("maxArenaMemoryCheck", "\"stop\"".to_string()),
        ("maxArenaMemoryCheck", "\"Halt\"".to_string()),
        ("maxArenaMemoryCheck", "true".to_string()),
    ] {
        let options = format!("{{\"{key}\": {value}}}");
        let (valid, raised) = validate(&options);
        assert!(!valid, "{options}");
        assert_eq!(raised.len(), 1, "{options}: {raised:?}");
        assert_eq!(raised[0].0, "PROJECT_JSON_FIELD_TYPE", "{options}");
        assert!(
            raised[0].1.contains(&format!("`{key}`")),
            "{options}: {raised:?}"
        );
        assert_eq!(
            arena_memory_limits(&manifest(&options)),
            defaults,
            "{options}"
        );
    }
}

/// With the cap disabled, each other key that is present is reported as ignored —
/// a warning, so validation still succeeds — and nothing is said about absent keys.
#[test]
fn disable_with_other_keys_warns_for_each_ignored_key() {
    let (valid, raised) = validate(
        "{\"maxArenaMemoryDisable\": true, \"maxArenaMemoryPercent\": 20, \
         \"maxArenaMemoryCheck\": \"warn\"}",
    );
    assert!(valid);
    let names: Vec<&str> = raised.iter().map(|(rule, _)| rule.as_str()).collect();
    assert_eq!(
        names,
        [
            "PROJECT_JSON_ARENA_MEMORY_IGNORED",
            "PROJECT_JSON_ARENA_MEMORY_IGNORED"
        ]
    );
    assert!(
        raised[0].1.contains("`maxArenaMemoryPercent`"),
        "{raised:?}"
    );
    assert!(raised[1].1.contains("`maxArenaMemoryCheck`"), "{raised:?}");
    assert!(!rules::is_error("PROJECT_JSON_ARENA_MEMORY_IGNORED"));

    // `false` ignores nothing.
    assert_eq!(
        validate("{\"maxArenaMemoryDisable\": false, \"maxArenaMemoryPercent\": 20}"),
        (true, Vec::new())
    );
}
