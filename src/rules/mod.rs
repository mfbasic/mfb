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
            !RULES.iter().any(|rule| rule.name == "IMPORT_MISSING_PACKAGE"),
            "IMPORT_MISSING_PACKAGE was renamed to IMPORT_PACKAGE_INVALID (bug-40)"
        );
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
}
