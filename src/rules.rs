use std::fmt;
use std::fs;
use std::path::Path;

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
    pub name: &'static str,
    pub severity: Severity,
    pub message: &'static str,
}

pub const RULES: &[Rule] = &[
    Rule {
        name: "PROJECT_JSON_MISSING",
        severity: Severity::Error,
        message: "project.json is required",
    },
    Rule {
        name: "PROJECT_JSON_READ_FAILED",
        severity: Severity::Error,
        message: "project.json could not be read",
    },
    Rule {
        name: "PROJECT_JSON_PARSE_FAILED",
        severity: Severity::Error,
        message: "project.json is not valid JSON",
    },
    Rule {
        name: "PROJECT_JSON_ROOT_TYPE",
        severity: Severity::Error,
        message: "project.json must contain a JSON object",
    },
    Rule {
        name: "PROJECT_JSON_REQUIRED_FIELD",
        severity: Severity::Error,
        message: "project.json is missing a required field",
    },
    Rule {
        name: "PROJECT_JSON_FIELD_TYPE",
        severity: Severity::Error,
        message: "project.json field has the wrong type",
    },
    Rule {
        name: "PROJECT_JSON_EMPTY_FIELD",
        severity: Severity::Error,
        message: "project.json field must not be empty",
    },
    Rule {
        name: "PROJECT_JSON_EMPTY_SOURCES",
        severity: Severity::Error,
        message: "project.json must include at least one source entry",
    },
    Rule {
        name: "PROJECT_JSON_UNKNOWN_KIND",
        severity: Severity::Warn,
        message: "project.json kind is not recognized",
    },
    Rule {
        name: "PROJECT_JSON_VALID",
        severity: Severity::Info,
        message: "project.json passed validation",
    },
];

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
        "{}:{} {}[{}]: {}",
        filename.display(),
        line.max(1),
        rule.severity,
        rule.name,
        rule.message
    );
    eprintln!("               {}", detailed_message);
}

fn rule_for(rule_name: &str) -> &'static Rule {
    RULES
        .iter()
        .find(|rule| rule.name == rule_name)
        .unwrap_or(&Rule {
            name: "UNKNOWN_RULE",
            severity: Severity::Error,
            message: "unknown diagnostic rule",
        })
}
