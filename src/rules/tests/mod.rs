use super::*;

#[test]
fn import_package_invalid_is_a_defined_rule() {
    // bug-40: the corrupt-`.mfp` import path emits `IMPORT_PACKAGE_INVALID`;
    // it must resolve to its defined identity, not the `UNKNOWN_RULE` sentinel.
    assert_eq!(
        code_and_name("IMPORT_PACKAGE_INVALID"),
        ("2-201-0001", "IMPORT_PACKAGE_INVALID")
    );
}

#[test]
fn dead_import_missing_package_name_is_gone() {
    // The old dead rule name was renamed onto slot 2-201-0001; nothing should
    // reference it any longer.
    assert!(
        !RULES
            .iter()
            .any(|rule| rule.name == "IMPORT_MISSING_PACKAGE"),
        "IMPORT_MISSING_PACKAGE was renamed to IMPORT_PACKAGE_INVALID (bug-40)"
    );
}

#[test]
fn severity_displays_all_three_levels() {
    assert_eq!(Severity::Error.to_string(), "error");
    assert_eq!(Severity::Warn.to_string(), "warn");
    // The `Info` arm is otherwise only hit by the (rare) info-severity rules.
    assert_eq!(Severity::Info.to_string(), "info");
}

#[test]
fn is_error_reflects_rule_severity() {
    // An Error-severity rule fails the build; a Warn-severity one does not.
    assert!(is_error("IMPORT_PACKAGE_INVALID"));
    assert!(!is_error("PRIVATE_SHADOWS_PUBLIC"));
    assert!(!is_error("PROJECT_JSON_VALID")); // Info
}

#[test]
fn show_diagnostic_renders_source_context_and_underline() {
    // Drives the on-disk source read, the context-line loop, and the caret
    // underline (start_pos > 0 and display_line == line). Output goes to
    // stderr; we only assert it does not panic on a real, multi-line file.
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("src.mfb");
    std::fs::write(&file, "line one\nline two\nline three\n").expect("write source");
    show_diagnostic("IMPORT_PACKAGE_INVALID", "detail here", &file, 2, 3, 6);
    // A line past the end clamps to the last line, still exercising the reader.
    show_diagnostic("IMPORT_PACKAGE_INVALID", "clamped", &file, 99, 0, 0);
}

// The guard is a `debug_assert!`, so the panic this asserts exists only when
// debug assertions are compiled in — under `cargo test --release` the call
// degrades to the UNKNOWN_RULE sentinel and the test can never pass.
#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "not defined in RULES")]
fn unknown_rule_name_trips_the_debug_assert() {
    // An emit site referencing a rule name absent from `RULES` fails loudly in
    // debug builds (the drift guard, bug-40) rather than silently degrading to
    // the UNKNOWN_RULE sentinel.
    let _ = code_and_name("NO_SUCH_RULE_NAME");
}

#[test]
fn rule_names_are_unique() {
    // `rule_for` resolves by name, so a duplicate name would shadow a rule.
    let mut names: Vec<&str> = RULES.iter().map(|rule| rule.name).collect();
    names.sort_unstable();
    assert!(
        names.windows(2).all(|w| w[0] != w[1]),
        "duplicate rule name in RULES"
    );
}

/// bug-790: a numeric code names exactly one rule. Tooling that keys
/// diagnostics by code cannot tell two rules sharing one apart, and a rule
/// with no live emitter still holds its code. There is no allowlist.
#[test]
fn rule_codes_are_unique() {
    let mut by_code: std::collections::HashMap<&str, Vec<&str>> = std::collections::HashMap::new();
    for rule in RULES {
        by_code.entry(rule.code).or_default().push(rule.name);
    }
    let mut shared: Vec<String> = by_code
        .into_iter()
        .filter(|(_, names)| names.len() > 1)
        .map(|(code, names)| format!("{code}: {}", names.join(", ")))
        .collect();
    shared.sort();
    assert!(
        shared.is_empty(),
        "codes shared by several rules: {shared:?}"
    );
}

/// Every rule in `RULES` must appear in the embedded `mfb spec diagnostics
/// rule-codes` table.
///
/// `.ai/compiler.md` requires the embedded spec to stay current with every
/// diagnostic change, but nothing enforced it: the `errorCode::` registry has
/// a build-time drift guard while the *rule* table had none, so a new rule
/// could ship documented only in the source. Caught exactly that during
/// plan-46 (`NATIVE_LIBRARY_VENDOR_COLLISION` was in `RULES` but not the
/// spec) — by hand, which is the reason this test exists.
#[test]
fn every_rule_is_documented_in_the_spec() {
    let spec = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/docs/spec/diagnostics/01_rule-codes.md"
    ));
    let missing: Vec<&str> = RULES
        .iter()
        .filter(|rule| {
            // The table renders one row per rule as `| `<code>` | `<NAME>` | ...`.
            !spec.contains(&format!("`{}`", rule.code)) || !spec.contains(rule.name)
        })
        .map(|rule| rule.code)
        .collect();
    assert!(
        missing.is_empty(),
        "rules missing from src/docs/spec/diagnostics/01_rule-codes.md: {missing:?}"
    );
}

#[test]
fn show_diagnostic_handles_empty_and_missing_source_files() {
    // An empty source file yields no lines, so the context/underline block is
    // skipped and only the header + detail are rendered (the `!lines.is_empty()`
    // false branch).
    let dir = tempfile::tempdir().expect("temp dir");
    let empty = dir.path().join("empty.mfb");
    std::fs::write(&empty, "").expect("write empty");
    show_diagnostic("IMPORT_PACKAGE_INVALID", "empty file", &empty, 1, 1, 2);

    // A file that does not exist: `fs::read_to_string` fails, so the whole
    // context block is skipped — the diagnostic header still renders.
    let missing = dir.path().join("does-not-exist.mfb");
    show_diagnostic("IMPORT_PACKAGE_INVALID", "missing file", &missing, 3, 1, 4);
}

#[test]
fn show_diagnostic_skips_underline_when_position_precedes_the_reported_line() {
    // start_pos > 0 but the clamped display line differs from the reported
    // line (line past EOF): the caret underline is suppressed even though a
    // start position was given (the `display_line == line` guard is false).
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("src.mfb");
    std::fs::write(&file, "only one line\n").expect("write source");
    show_diagnostic(
        "IMPORT_PACKAGE_INVALID",
        "clamped-with-pos",
        &file,
        42,
        3,
        7,
    );
}

#[test]
fn show_general_diagnostic_renders_header_and_detail() {
    // The context-free renderer (used when there is no source location) emits
    // the rule header and the detail line for each severity.
    show_general_diagnostic("IMPORT_PACKAGE_INVALID", "an error detail");
    show_general_diagnostic("PRIVATE_SHADOWS_PUBLIC", "a warning detail");
    show_general_diagnostic("PROJECT_JSON_VALID", "an info detail");
}

#[test]
fn show_diagnostic_rewrites_mangled_generic_names_to_template_spellings() {
    // bug-697: `show_diagnostic` is the render boundary that turns a
    // post-monomorph mangled nominal (`Box$Integer`) back into what the
    // user wrote (`Box OF Integer`). Drives the table directly (rather
    // than through a full build) so the boundary itself is pinned
    // independent of `monomorph::monomorphize_project`'s table-building.
    let mut table = HashMap::new();
    table.insert("Box$Integer".to_string(), "Box OF Integer".to_string());
    table.insert(
        "Holder$Holder$Integer".to_string(),
        "Holder OF Holder OF Integer".to_string(),
    );
    set_template_spellings(table);

    let (_, collected) = collecting_diagnostics(|| {
        let dir = tempfile::tempdir().expect("temp dir");
        let file = dir.path().join("src.mfb");
        std::fs::write(&file, "line one\n").expect("write source");
        show_diagnostic(
            "IMPORT_PACKAGE_INVALID",
            "Binding `n` has initializer type Box$Integer, expected Integer.",
            &file,
            1,
            1,
            1,
        );
        // A nested instantiation renders fully, never half-mangled —
        // the whole `Holder$Holder$Integer` token is one match, not
        // `Holder$Holder` + a partial leftover.
        show_diagnostic(
            "IMPORT_PACKAGE_INVALID",
            "Binding `h` has initializer type Holder$Holder$Integer, expected Integer.",
            &file,
            1,
            1,
            1,
        );
        // An unknown `$` token (e.g. a mangled name from an imported
        // `.mfp`, which has no entry in this build's table) is left
        // exactly as is, never guessed at.
        show_diagnostic(
            "IMPORT_PACKAGE_INVALID",
            "Binding `u` has initializer type Unknown$Stranger, expected Integer.",
            &file,
            1,
            1,
            1,
        );
        // Text with no `$` at all is untouched (the common case).
        show_diagnostic(
            "IMPORT_PACKAGE_INVALID",
            "Binding `p` has initializer type Integer, expected String.",
            &file,
            1,
            1,
            1,
        );
    });

    // Reset so a later test on this thread starts from an empty table.
    set_template_spellings(HashMap::new());

    assert_eq!(
        collected[0].1,
        "Binding `n` has initializer type Box OF Integer, expected Integer."
    );
    assert_eq!(
        collected[1].1,
        "Binding `h` has initializer type Holder OF Holder OF Integer, expected Integer."
    );
    assert_eq!(
        collected[2].1,
        "Binding `u` has initializer type Unknown$Stranger, expected Integer."
    );
    assert_eq!(
        collected[3].1,
        "Binding `p` has initializer type Integer, expected String."
    );
}

#[test]
fn code_and_name_resolves_representative_rules() {
    // Every entry in the table resolves to its own identity (not the sentinel).
    for rule in RULES {
        let (code, name) = code_and_name(rule.name);
        assert_eq!(name, rule.name, "name round-trip for {}", rule.name);
        assert_eq!(code, rule.code, "code round-trip for {}", rule.name);
    }
}

#[test]
fn is_error_partitions_the_whole_table_by_severity() {
    // Exercise `is_error` across every defined rule so both the Error and the
    // non-Error (Warn/Info) arms are hit for real table entries.
    for rule in RULES {
        let expected = matches!(rule.severity, Severity::Error);
        assert_eq!(is_error(rule.name), expected, "is_error for {}", rule.name);
    }
}

// ---- bug-505: read-once source cache; FE-04: terminal-safe echo ----
// These drive the helpers directly rather than `show_diagnostic`: the
// rendering cap is process-wide, and the thousands of parses this test
// binary runs have long since crossed it.

#[test]
fn cached_source_reads_a_file_once_and_indexes_its_lines() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("src.mfb");
    std::fs::write(&file, "line one\r\nline two\n\nline four").expect("write");
    let first = cached_source(&file).expect("readable");
    let again = cached_source(&file).expect("readable");
    assert!(
        Arc::ptr_eq(&first, &again),
        "a second diagnostic must reuse the indexed file, not re-read it"
    );
    // Exactly `str::lines` semantics: `\r\n` stripped, empty line kept, no
    // phantom line after a missing trailing newline.
    assert_eq!(first.line_count(), 4);
    assert_eq!(first.line(0), Some("line one"));
    assert_eq!(first.line(1), Some("line two"));
    assert_eq!(first.line(2), Some(""));
    assert_eq!(first.line(3), Some("line four"));
    assert_eq!(first.line(4), None);
}

#[test]
fn cached_source_notices_a_rewritten_file() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("src.mfb");
    std::fs::write(&file, "before\n").expect("write");
    let before = cached_source(&file).expect("readable");
    // A different length is a different stamp even within mtime granularity.
    std::fs::write(&file, "after, longer\n").expect("rewrite");
    let after = cached_source(&file).expect("readable");
    assert_eq!(before.line(0), Some("before"));
    assert_eq!(after.line(0), Some("after, longer"));
}

#[test]
fn cached_source_is_none_for_a_missing_file() {
    let dir = tempfile::tempdir().expect("temp dir");
    assert!(cached_source(&dir.path().join("nope.mfb")).is_none());
}

#[test]
fn safe_source_line_escapes_controls_and_bidi_but_keeps_tabs() {
    // Plain text (including a tab) passes through unallocated.
    assert!(matches!(
        safe_source_line("\tLET x = 1"),
        Cow::Borrowed("\tLET x = 1")
    ));
    // ESC/CSI, BEL, CR and RLO are escaped; the tab survives verbatim.
    assert_eq!(
        safe_source_line("\tLET s = \"\u{1b}[31mred\u{7}\r\u{202e}\""),
        "\tLET s = \"\\u{001b}[31mred\\u{0007}\\u{000d}\\u{202e}\""
    );
}

#[test]
fn suppressed_count_is_seen_minus_the_cap() {
    // Only the arithmetic is asserted here (the printer writes to stderr);
    // the end-to-end shape is pinned by tests/cli/cli_diagnostic_stream.rs.
    assert_eq!(0usize.saturating_sub(MAX_RENDERED_DIAGNOSTICS), 0);
    assert_eq!(
        (MAX_RENDERED_DIAGNOSTICS + 51).saturating_sub(MAX_RENDERED_DIAGNOSTICS),
        51
    );
    report_suppressed_diagnostics();
}
