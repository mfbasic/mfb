use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

/// A rejection collected but not yet rendered. The source-path checkers
/// (`syntaxcheck` for not-yet-relocated rules, `ir::verify` for relocated ones)
/// each return these so the caller can merge both streams and render them in a
/// single source-order pass — otherwise a relocated rule would print after all
/// of syntaxcheck's, breaking the line-ordered diagnostic sequence (plan-20-Z).
pub struct PendingDiagnostic {
    pub rule: String,
    pub detail: String,
    pub path: PathBuf,
    pub line: usize,
}

/// Render `diagnostics` in the order given. The caller concatenates
/// `syntaxcheck`'s stream (not-yet-relocated rules, in its traversal order) with
/// `ir::verify`'s relocated stream, matching the sequence the goldens record
/// and the eventual single-checker (ir::verify traversal) end state — not a
/// line sort, which neither checker produces (plan-20-Z).
pub fn render_pending(diagnostics: Vec<PendingDiagnostic>) {
    for d in &diagnostics {
        show_diagnostic(&d.rule, &d.detail, &d.path, d.line, 1, 1);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Severity {
    Error,
    Warn,
    Info,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Severity::Error => write!(f, "error"),
            Severity::Warn => write!(f, "warn"),
            Severity::Info => write!(f, "info"),
        }
    }
}

pub struct Rule {
    pub code: &'static str,
    pub name: &'static str,
    pub severity: Severity,
    pub message: &'static str,
}

mod table;
use table::RULES;

pub fn show_diagnostic(
    rule_name: &str,
    detailed_message: &str,
    filename: &Path,
    line: usize,
    start_pos: usize,
    end_pos: usize,
) {
    let rule = rule_for(rule_name);

    if let Ok(contents) = fs::read_to_string(filename) {
        let lines: Vec<&str> = contents.lines().collect();
        let display_line = line.min(lines.len()).max(1);
        if !lines.is_empty() {
            let first_context_line = display_line.saturating_sub(2).max(1);
            for context_line in first_context_line..=display_line {
                if let Some(source_line) = lines.get(context_line - 1) {
                    eprintln!("{:>4} | {}", context_line, source_line);
                }
            }

            if start_pos > 0 && display_line == line {
                let underline_width = end_pos.saturating_sub(start_pos).max(1);
                eprintln!(
                    "     | {}{}",
                    " ".repeat(start_pos.saturating_sub(1)),
                    "^".repeat(underline_width)
                );
            }
        }
    }

    eprintln!(
        "{}:{} {}[{} {}]: {}",
        filename.display(),
        line.max(1),
        rule.severity,
        rule.code,
        rule.name,
        rule.message
    );
    eprintln!("               {}", detailed_message);
}

pub fn show_general_diagnostic(rule_name: &str, detailed_message: &str) {
    let rule = rule_for(rule_name);
    eprintln!(
        "{}[{} {}]: {}",
        rule.severity, rule.code, rule.name, rule.message
    );
    eprintln!("               {}", detailed_message);
}

/// Whether a diagnostic rule is `Error` severity (as opposed to `Warn`/`Info`).
/// Lets a collected diagnostic stream fail the build only on real errors while
/// still rendering warnings.
pub fn is_error(rule_name: &str) -> bool {
    matches!(rule_for(rule_name).severity, Severity::Error)
}

/// Resolve a rule name to its `(code, name)` identity as rendered in a
/// diagnostic header. Returns the `0-000-0000 UNKNOWN_RULE` sentinel when the
/// name is not defined in `RULES` (and, in debug builds, asserts). Used by tests
/// to prove an emit site references a defined rule.
#[cfg(test)]
pub(crate) fn code_and_name(rule_name: &str) -> (&'static str, &'static str) {
    let rule = rule_for(rule_name);
    (rule.code, rule.name)
}

fn rule_for(rule_name: &str) -> &'static Rule {
    match RULES.iter().find(|rule| rule.name == rule_name) {
        Some(rule) => rule,
        None => {
            // An emit site referenced a rule name absent from `RULES`: the emit
            // site and the table have drifted (see bug-40). Fail loudly in debug
            // builds so the mismatch is caught by tests rather than silently
            // degraded to the `0-000-0000 UNKNOWN_RULE` sentinel at runtime.
            debug_assert!(
                false,
                "diagnostic rule `{rule_name}` is not defined in RULES (src/rules/table.rs)"
            );
            &Rule {
                code: "0-000-0000",
                name: "UNKNOWN_RULE",
                severity: Severity::Error,
                message: "unknown diagnostic rule",
            }
        }
    }
}

#[cfg(test)]
mod tests {
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
}
