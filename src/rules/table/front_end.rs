use super::*;

pub(super) const RULES: &[Rule] = &[
    Rule {
        code: "2-200-0001",
        name: "PROJECT_JSON_MISSING",
        severity: Severity::Error,
        message: "project.json is required",
    },
    Rule {
        code: "2-200-0002",
        name: "PROJECT_JSON_READ_FAILED",
        severity: Severity::Error,
        message: "project.json could not be read",
    },
    Rule {
        code: "2-200-0003",
        name: "PROJECT_JSON_PARSE_FAILED",
        severity: Severity::Error,
        message: "project.json is not valid JSON",
    },
    Rule {
        code: "2-200-0004",
        name: "PROJECT_JSON_ROOT_TYPE",
        severity: Severity::Error,
        message: "project.json must contain a JSON object",
    },
    Rule {
        code: "2-200-0005",
        name: "PROJECT_JSON_REQUIRED_FIELD",
        severity: Severity::Error,
        message: "project.json is missing a required field",
    },
    Rule {
        code: "2-200-0006",
        name: "PROJECT_JSON_FIELD_TYPE",
        severity: Severity::Error,
        message: "project.json field has the wrong type",
    },
    Rule {
        code: "2-200-0007",
        name: "PROJECT_JSON_EMPTY_FIELD",
        severity: Severity::Error,
        message: "project.json field must not be empty",
    },
    Rule {
        code: "2-200-0008",
        name: "PROJECT_JSON_EMPTY_SOURCES",
        severity: Severity::Error,
        message: "project.json must include at least one source entry",
    },
    Rule {
        code: "2-200-0009",
        name: "PROJECT_JSON_UNKNOWN_KIND",
        severity: Severity::Warn,
        message: "project.json kind is not recognized",
    },
    // Reserved, not emitted. `mfb build` reports manifest problems and says
    // nothing on success, so there is no site that would raise this. Kept as a
    // reserved code rather than deleted so `2-200-0010` is never recycled for a
    // different meaning; the spec rows now say "reserved" instead of describing
    // a diagnostic users can never see (bug-326-A15).
    Rule {
        code: "2-200-0010",
        name: "PROJECT_JSON_VALID",
        severity: Severity::Info,
        message: "project.json passed validation",
    },
    Rule {
        code: "1-100-0001",
        name: "MFB_SOURCE_READ_FAILED",
        severity: Severity::Error,
        message: "MFBASIC source could not be read",
    },
    Rule {
        code: "1-100-0002",
        name: "MFB_SOURCE_ROOT_MISSING",
        severity: Severity::Error,
        message: "MFBASIC source root does not exist",
    },
    Rule {
        code: "1-100-0003",
        name: "MFB_SOURCE_EMPTY",
        severity: Severity::Error,
        message: "MFBASIC source root contains no source files",
    },
    Rule {
        code: "1-100-0004",
        name: "MFB_SOURCE_OUTSIDE_PROJECT",
        severity: Severity::Error,
        message: "MFBASIC source path resolves outside the project directory",
    },
    Rule {
        code: "1-100-0005",
        name: "MFB_SOURCE_OVERLAP",
        severity: Severity::Error,
        message: "MFBASIC source file is selected by more than one source entry",
    },
    Rule {
        code: "1-101-0001",
        name: "MFB_LEX_UNEXPECTED_CHARACTER",
        severity: Severity::Error,
        message: "lexer found an unexpected character",
    },
    Rule {
        code: "1-101-0002",
        name: "MFB_LEX_UNTERMINATED_STRING",
        severity: Severity::Error,
        message: "string literal is unterminated",
    },
    Rule {
        code: "1-101-0003",
        name: "MFB_LEX_INVALID_UNICODE_ESCAPE",
        severity: Severity::Error,
        message: "\\u{...} string escape is malformed",
    },
    Rule {
        code: "1-101-0004",
        name: "MFB_LEX_MALFORMED_NUMBER",
        severity: Severity::Error,
        message: "numeric literal is malformed",
    },
    Rule {
        code: "1-101-0005",
        name: "MFB_LEX_NUMBER_OUT_OF_RANGE",
        severity: Severity::Error,
        message: "numeric literal is out of range",
    },
    Rule {
        code: "1-101-0006",
        name: "MFB_LEX_UNCLOSED_BRACKET",
        severity: Severity::Error,
        message: "bracket is never closed",
    },
    Rule {
        code: "1-102-0001",
        name: "MFB_PARSE_EXPECTED_EXPRESSION",
        severity: Severity::Error,
        message: "parser expected an expression",
    },
    // 1-102-0002 (MFB_PARSE_INVALID_FUNCTION_HEADER) dropped (bug-216): it had no
    // emit site — an invalid FUNC header is reported via MFB_PARSE_UNEXPECTED_TOKEN.
    Rule {
        code: "1-102-0003",
        name: "MFB_PARSE_INVALID_IDENTIFIER",
        severity: Severity::Error,
        message: "identifier is invalid",
    },
    Rule {
        code: "1-102-0004",
        name: "MFB_PARSE_UNEXPECTED_STATEMENT",
        severity: Severity::Error,
        message: "parser found an unexpected statement",
    },
    Rule {
        code: "1-102-0005",
        name: "MFB_PARSE_UNEXPECTED_TOKEN",
        severity: Severity::Error,
        message: "parser found an unexpected token",
    },
    Rule {
        code: "1-102-0006",
        name: "MFB_PARSE_UNTERMINATED_BLOCK",
        severity: Severity::Error,
        message: "parser reached end-of-file inside a block",
    },
    Rule {
        code: "1-102-0007",
        name: "MFB_PARSE_PIPELINE_PLACEHOLDER_MISSING",
        severity: Severity::Error,
        message: "pipeline expression is missing a placeholder",
    },
    Rule {
        code: "1-102-0008",
        name: "MFB_PARSE_MISSING_NATIVE_SYMBOL",
        severity: Severity::Error,
        message: "a native LINK function must declare its native SYMBOL",
    },
    Rule {
        code: "1-102-0009",
        name: "MFB_PARSE_MISSING_NATIVE_ABI",
        severity: Severity::Error,
        message: "a native LINK function must declare its ABI signature",
    },
    Rule {
        code: "1-102-0014",
        name: "MFB_PARSE_BLOCK_TOO_DEEP",
        severity: Severity::Error,
        message: "statement block nesting is too deep",
    },
    Rule {
        code: "1-102-0013",
        name: "MFB_PARSE_RECORD_FIELD_ASSIGNMENT",
        severity: Severity::Error,
        message: "record field assignment is not supported",
    },
    // bug-976: a parameter is a resource because its type is one; `RES` is not
    // written on a parameter.
    Rule {
        code: "1-102-0015",
        name: "MFB_PARSE_PARAMETER_RES",
        severity: Severity::Error,
        message: "a parameter takes no RES; its type already makes it a resource",
    },
    Rule {
        code: "2-201-0001",
        name: "IMPORT_PACKAGE_INVALID",
        severity: Severity::Error,
        message: "imported package binary could not be read",
    },
    Rule {
        code: "2-201-0002",
        name: "IMPORT_PACKAGE_NOT_DECLARED",
        severity: Severity::Error,
        message: "imported package is not declared",
    },
    Rule {
        code: "2-201-0003",
        name: "IMPORT_PACKAGE_NOT_INSTALLED",
        severity: Severity::Error,
        message: "declared package is not installed",
    },
    Rule {
        code: "2-201-0004",
        name: "IMPORT_LOCAL_PATH_INVALID",
        severity: Severity::Error,
        message: "local package source must be an absolute local URL",
    },
    Rule {
        code: "2-201-0005",
        name: "IMPORT_PACKAGE_MANIFEST_INVALID",
        severity: Severity::Error,
        message: "imported package manifest is invalid",
    },
    Rule {
        code: "2-201-0006",
        name: "IMPORT_PACKAGE_NAME_MISMATCH",
        severity: Severity::Error,
        message: "imported package manifest name does not match import",
    },
    Rule {
        code: "2-201-0007",
        name: "IMPORT_PACKAGE_KIND_INVALID",
        severity: Severity::Error,
        message: "imported source package must be a package",
    },
    Rule {
        code: "2-201-0008",
        name: "SYMBOL_DUPLICATE_IMPORT",
        severity: Severity::Error,
        message: "import is declared more than once",
    },
    Rule {
        code: "2-201-0009",
        name: "SYMBOL_DUPLICATE_LOCAL",
        severity: Severity::Error,
        message: "local symbol is declared more than once",
    },
    Rule {
        code: "2-201-0010",
        name: "SYMBOL_DUPLICATE_TOP_LEVEL",
        severity: Severity::Error,
        message: "top-level symbol is declared more than once",
    },
    Rule {
        code: "2-201-0011",
        name: "SYMBOL_UNKNOWN_IDENTIFIER",
        severity: Severity::Error,
        message: "identifier could not be resolved",
    },
    Rule {
        code: "2-201-0012",
        name: "SYMBOL_NOT_CALLABLE",
        severity: Severity::Error,
        message: "symbol cannot be called",
    },
    // 2-201-0013 (SYMBOL_NOT_VALUE) dropped (bug-216): it had no emit site — a
    // symbol used in value position is reported via SYMBOL_UNKNOWN_IDENTIFIER.
    Rule {
        code: "2-201-0014",
        name: "SYMBOL_UNKNOWN_IMPORT",
        severity: Severity::Error,
        message: "package-qualified symbol uses an unknown import",
    },
    Rule {
        code: "2-201-0015",
        name: "SYMBOL_UNKNOWN_TYPE",
        severity: Severity::Error,
        message: "type name could not be resolved",
    },
    Rule {
        code: "2-201-0016",
        name: "SYMBOL_RESERVED_BUILTIN_NAME",
        severity: Severity::Error,
        message: "function name is a reserved built-in and may not be redeclared",
    },
    Rule {
        code: "2-201-0017",
        name: "PRIVATE_SHADOWS_PUBLIC",
        severity: Severity::Warn,
        message:
            "PRIVATE declaration shadows a PUBLIC declaration of the same name within its file",
    },
    Rule {
        code: "2-201-0018",
        name: "PRIVATE_PATH_HASH_COLLISION",
        severity: Severity::Error,
        message: "internal: two source file paths produced the same file-scope hash",
    },
    Rule {
        code: "2-201-0020",
        name: "SYMBOL_DEFAULT_NAMES_PARAMETER",
        severity: Severity::Error,
        message: "parameter default value uses a parameter of its own function",
    },
    Rule {
        code: "2-201-0021",
        name: "SYMBOL_LAMBDA_PARAMETER_DEFAULT",
        severity: Severity::Error,
        message: "lambda parameter declares a default value",
    },
    Rule {
        code: "2-201-0022",
        name: "SYMBOL_SHADOWS_TOP_LEVEL_BINDING",
        severity: Severity::Error,
        message: "local binding reuses the name of a visible top-level binding",
    },
];
