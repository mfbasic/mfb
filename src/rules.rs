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
    Rule {
        name: "MFB_SOURCE_READ_FAILED",
        severity: Severity::Error,
        message: "MFBASIC source could not be read",
    },
    Rule {
        name: "MFB_SOURCE_ROOT_MISSING",
        severity: Severity::Error,
        message: "MFBASIC source root does not exist",
    },
    Rule {
        name: "MFB_SOURCE_EMPTY",
        severity: Severity::Error,
        message: "MFBASIC source root contains no source files",
    },
    Rule {
        name: "MFB_LEX_UNEXPECTED_CHARACTER",
        severity: Severity::Error,
        message: "lexer found an unexpected character",
    },
    Rule {
        name: "MFB_LEX_UNTERMINATED_STRING",
        severity: Severity::Error,
        message: "string literal is unterminated",
    },
    Rule {
        name: "MFB_PARSE_EXPECTED_EXPRESSION",
        severity: Severity::Error,
        message: "parser expected an expression",
    },
    Rule {
        name: "MFB_PARSE_INVALID_FUNCTION_HEADER",
        severity: Severity::Error,
        message: "function header is invalid",
    },
    Rule {
        name: "MFB_PARSE_INVALID_IDENTIFIER",
        severity: Severity::Error,
        message: "identifier is invalid",
    },
    Rule {
        name: "MFB_PARSE_UNEXPECTED_STATEMENT",
        severity: Severity::Error,
        message: "parser found an unexpected statement",
    },
    Rule {
        name: "MFB_PARSE_UNEXPECTED_TOKEN",
        severity: Severity::Error,
        message: "parser found an unexpected token",
    },
    Rule {
        name: "MFB_PARSE_UNTERMINATED_BLOCK",
        severity: Severity::Error,
        message: "parser reached end-of-file inside a block",
    },
    Rule {
        name: "IMPORT_MISSING_PACKAGE",
        severity: Severity::Error,
        message: "imported package could not be resolved",
    },
    Rule {
        name: "IMPORT_PACKAGE_NOT_DECLARED",
        severity: Severity::Error,
        message: "imported package is not declared",
    },
    Rule {
        name: "IMPORT_PACKAGE_NOT_INSTALLED",
        severity: Severity::Error,
        message: "declared package is not installed",
    },
    Rule {
        name: "IMPORT_LOCAL_PATH_INVALID",
        severity: Severity::Error,
        message: "local package source must be an absolute local URL",
    },
    Rule {
        name: "IMPORT_PACKAGE_MANIFEST_INVALID",
        severity: Severity::Error,
        message: "imported package manifest is invalid",
    },
    Rule {
        name: "IMPORT_PACKAGE_NAME_MISMATCH",
        severity: Severity::Error,
        message: "imported package manifest name does not match import",
    },
    Rule {
        name: "IMPORT_PACKAGE_KIND_INVALID",
        severity: Severity::Error,
        message: "imported source package must be a library",
    },
    Rule {
        name: "SYMBOL_DUPLICATE_IMPORT",
        severity: Severity::Error,
        message: "import is declared more than once",
    },
    Rule {
        name: "SYMBOL_DUPLICATE_LOCAL",
        severity: Severity::Error,
        message: "local symbol is declared more than once",
    },
    Rule {
        name: "SYMBOL_DUPLICATE_TOP_LEVEL",
        severity: Severity::Error,
        message: "top-level symbol is declared more than once",
    },
    Rule {
        name: "SYMBOL_UNKNOWN_IDENTIFIER",
        severity: Severity::Error,
        message: "identifier could not be resolved",
    },
    Rule {
        name: "SYMBOL_NOT_CALLABLE",
        severity: Severity::Error,
        message: "symbol cannot be called",
    },
    Rule {
        name: "SYMBOL_NOT_VALUE",
        severity: Severity::Error,
        message: "symbol is not a value",
    },
    Rule {
        name: "SYMBOL_UNKNOWN_IMPORT",
        severity: Severity::Error,
        message: "package-qualified symbol uses an unknown import",
    },
    Rule {
        name: "SYMBOL_UNKNOWN_TYPE",
        severity: Severity::Error,
        message: "type name could not be resolved",
    },
    Rule {
        name: "TYPE_BINARY_OPERATOR_MISMATCH",
        severity: Severity::Error,
        message: "binary operator operands have incompatible types",
    },
    Rule {
        name: "TYPE_BINDING_MISMATCH",
        severity: Severity::Error,
        message: "binding initializer type does not match declared type",
    },
    Rule {
        name: "TYPE_BINDING_REQUIRES_TYPE_OR_VALUE",
        severity: Severity::Error,
        message: "binding requires a type annotation or initializer",
    },
    Rule {
        name: "TYPE_CALL_ARGUMENT_MISMATCH",
        severity: Severity::Error,
        message: "function call argument type does not match parameter type",
    },
    Rule {
        name: "TYPE_CALL_ARITY_MISMATCH",
        severity: Severity::Error,
        message: "function call has the wrong number of arguments",
    },
    Rule {
        name: "TYPE_DEFAULT_ARG_ORDER",
        severity: Severity::Error,
        message: "default parameters must be trailing",
    },
    Rule {
        name: "TYPE_DEFAULT_VALUE_MISMATCH",
        severity: Severity::Error,
        message: "default parameter value has the wrong type",
    },
    Rule {
        name: "TYPE_DUPLICATE_ENUM_MEMBER",
        severity: Severity::Error,
        message: "enum member is declared more than once",
    },
    Rule {
        name: "TYPE_DUPLICATE_FIELD",
        severity: Severity::Error,
        message: "type field is declared more than once",
    },
    Rule {
        name: "TYPE_DUPLICATE_VARIANT",
        severity: Severity::Error,
        message: "union variant is declared more than once",
    },
    Rule {
        name: "TYPE_ENUM_REQUIRES_MEMBER",
        severity: Severity::Error,
        message: "enum must declare at least one member",
    },
    Rule {
        name: "TYPE_FUNC_MISSING_RETURN",
        severity: Severity::Error,
        message: "function is missing a return value",
    },
    Rule {
        name: "TYPE_FUNC_REQUIRES_RETURN_TYPE",
        severity: Severity::Error,
        message: "FUNC must declare a return type",
    },
    Rule {
        name: "TYPE_LET_REQUIRES_VALUE",
        severity: Severity::Error,
        message: "immutable binding must have an initializer",
    },
    Rule {
        name: "TYPE_PARAM_REQUIRES_TYPE",
        severity: Severity::Error,
        message: "parameter must declare a type",
    },
    Rule {
        name: "TYPE_RESULT_IS_IMPLICIT",
        severity: Severity::Error,
        message: "Result return wrapping is implicit",
    },
    Rule {
        name: "TYPE_RETURN_MISMATCH",
        severity: Severity::Error,
        message: "return value type does not match function success type",
    },
    Rule {
        name: "TYPE_SUB_CANNOT_RETURN_VALUE",
        severity: Severity::Error,
        message: "SUB cannot return a value",
    },
    Rule {
        name: "TYPE_UNKNOWN_VALUE",
        severity: Severity::Error,
        message: "value type could not be determined",
    },
    Rule {
        name: "TYPE_UNION_INCLUDE_REQUIRES_UNION",
        severity: Severity::Error,
        message: "union includes must name union types",
    },
    Rule {
        name: "PROJECT_ENTRY_INVALID",
        severity: Severity::Error,
        message: "project entry point is invalid",
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
