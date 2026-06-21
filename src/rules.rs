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
    pub code: &'static str,
    pub name: &'static str,
    pub severity: Severity,
    pub message: &'static str,
}

pub const RULES: &[Rule] = &[
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
        code: "1-102-0001",
        name: "MFB_PARSE_EXPECTED_EXPRESSION",
        severity: Severity::Error,
        message: "parser expected an expression",
    },
    Rule {
        code: "1-102-0002",
        name: "MFB_PARSE_INVALID_FUNCTION_HEADER",
        severity: Severity::Error,
        message: "function header is invalid",
    },
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
        code: "2-201-0001",
        name: "IMPORT_MISSING_PACKAGE",
        severity: Severity::Error,
        message: "imported package could not be resolved",
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
    Rule {
        code: "2-201-0013",
        name: "SYMBOL_NOT_VALUE",
        severity: Severity::Error,
        message: "symbol is not a value",
    },
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
        code: "2-203-0001",
        name: "TYPE_BINARY_OPERATOR_MISMATCH",
        severity: Severity::Error,
        message: "binary operator operands have incompatible types",
    },
    Rule {
        code: "2-203-0002",
        name: "TYPE_UNARY_OPERATOR_MISMATCH",
        severity: Severity::Error,
        message: "unary operator operand has an incompatible type",
    },
    Rule {
        code: "2-203-0003",
        name: "TYPE_UNARY_OPERATOR_UNKNOWN",
        severity: Severity::Error,
        message: "unary operator is not recognized",
    },
    Rule {
        code: "2-203-0004",
        name: "TYPE_FOR_REQUIRES_NUMERIC",
        severity: Severity::Error,
        message: "FOR loop operands must be numeric",
    },
    Rule {
        code: "2-203-0005",
        name: "TYPE_FOR_STEP_ZERO",
        severity: Severity::Error,
        message: "FOR loop step must not be zero",
    },
    Rule {
        code: "2-203-0006",
        name: "TYPE_CONDITION_REQUIRES_BOOLEAN",
        severity: Severity::Error,
        message: "control-flow condition must be Boolean",
    },
    Rule {
        code: "2-203-0007",
        name: "TYPE_BINDING_MISMATCH",
        severity: Severity::Error,
        message: "binding initializer type does not match declared type",
    },
    Rule {
        code: "2-203-0008",
        name: "TYPE_ASSIGNMENT_MISMATCH",
        severity: Severity::Error,
        message: "assignment value type does not match binding type",
    },
    Rule {
        code: "2-203-0009",
        name: "TYPE_INTEGER_LITERAL_OVERFLOW",
        severity: Severity::Error,
        message: "integer literal is outside the Integer range",
    },
    Rule {
        code: "2-203-0010",
        name: "TYPE_FAIL_REQUIRES_ERROR",
        severity: Severity::Error,
        message: "FAIL requires an Error value",
    },
    Rule {
        code: "2-203-0011",
        name: "TYPE_PROPAGATE_REQUIRES_TRAP",
        severity: Severity::Error,
        message: "PROPAGATE requires a TRAP context",
    },
    Rule {
        code: "2-203-0012",
        name: "TYPE_TRAP_FALLTHROUGH",
        severity: Severity::Error,
        message: "TRAP path can fall through",
    },
    Rule {
        code: "2-203-0013",
        name: "TYPE_BYTE_LITERAL_OVERFLOW",
        severity: Severity::Error,
        message: "integer literal is outside the Byte range",
    },
    Rule {
        code: "2-203-0014",
        name: "TYPE_BYTE_LITERAL_UNDERFLOW",
        severity: Severity::Error,
        message: "integer literal is outside the Byte range",
    },
    Rule {
        code: "2-203-0015",
        name: "TYPE_FLOAT_LITERAL_OVERFLOW",
        severity: Severity::Error,
        message: "numeric literal is outside the Float range",
    },
    Rule {
        code: "2-203-0016",
        name: "TYPE_FLOAT_LITERAL_UNDERFLOW",
        severity: Severity::Error,
        message: "numeric literal is outside the Float range",
    },
    Rule {
        code: "2-203-0017",
        name: "TYPE_FIXED_LITERAL_OVERFLOW",
        severity: Severity::Error,
        message: "numeric literal is outside the Fixed range",
    },
    Rule {
        code: "2-203-0018",
        name: "TYPE_FIXED_LITERAL_UNDERFLOW",
        severity: Severity::Error,
        message: "numeric literal is outside the Fixed range",
    },
    Rule {
        code: "2-203-0019",
        name: "TYPE_LAMBDA_CAPTURE_UNSUPPORTED",
        severity: Severity::Error,
        message: "lambda capture is invalid",
    },
    Rule {
        code: "2-203-0020",
        name: "TYPE_BINDING_REQUIRES_TYPE_OR_VALUE",
        severity: Severity::Error,
        message: "binding requires a type annotation or initializer",
    },
    Rule {
        code: "2-203-0021",
        name: "TYPE_CALL_ARGUMENT_MISMATCH",
        severity: Severity::Error,
        message: "function call argument type does not match parameter type",
    },
    Rule {
        code: "2-203-0022",
        name: "TYPE_CALL_ARITY_MISMATCH",
        severity: Severity::Error,
        message: "function call has the wrong number of arguments",
    },
    Rule {
        code: "2-203-0023",
        name: "TYPE_CONSTRUCTOR_ARGUMENT_MISMATCH",
        severity: Severity::Error,
        message: "constructor argument type does not match field type",
    },
    Rule {
        code: "2-203-0024",
        name: "TYPE_CONSTRUCTOR_ARITY_MISMATCH",
        severity: Severity::Error,
        message: "constructor has the wrong number of arguments",
    },
    Rule {
        code: "2-203-0025",
        name: "TYPE_CONSTRUCTOR_REQUIRES_RECORD",
        severity: Severity::Error,
        message: "record constructor syntax requires a TYPE",
    },
    Rule {
        code: "2-203-0026",
        name: "TYPE_DEFAULT_ARG_ORDER",
        severity: Severity::Error,
        message: "default parameters must be trailing",
    },
    Rule {
        code: "2-203-0027",
        name: "TYPE_DEFAULT_VALUE_MISMATCH",
        severity: Severity::Error,
        message: "default parameter value has the wrong type",
    },
    Rule {
        code: "2-203-0028",
        name: "TYPE_DUPLICATE_ENUM_MEMBER",
        severity: Severity::Error,
        message: "enum member is declared more than once",
    },
    Rule {
        code: "2-203-0029",
        name: "TYPE_DUPLICATE_FIELD",
        severity: Severity::Error,
        message: "type field is declared more than once",
    },
    Rule {
        code: "2-203-0030",
        name: "TYPE_DUPLICATE_VARIANT",
        severity: Severity::Error,
        message: "union variant is declared more than once",
    },
    Rule {
        code: "2-203-0031",
        name: "TYPE_ENUM_REQUIRES_MEMBER",
        severity: Severity::Error,
        message: "enum must declare at least one member",
    },
    Rule {
        code: "2-203-0032",
        name: "TYPE_FUNC_MISSING_RETURN",
        severity: Severity::Error,
        message: "function is missing a return value",
    },
    Rule {
        code: "2-203-0033",
        name: "TYPE_FUNC_REQUIRES_RETURN_TYPE",
        severity: Severity::Error,
        message: "FUNC must declare a return type",
    },
    Rule {
        code: "2-203-0034",
        name: "TYPE_FIELD_ACCESS_REQUIRES_RECORD",
        severity: Severity::Error,
        message: "field access requires a record value",
    },
    Rule {
        code: "2-203-0035",
        name: "TYPE_LET_REQUIRES_VALUE",
        severity: Severity::Error,
        message: "immutable binding must have an initializer",
    },
    Rule {
        code: "2-203-0036",
        name: "TYPE_MEMBER_NOT_VISIBLE",
        severity: Severity::Error,
        message: "type member is not visible from this scope",
    },
    Rule {
        code: "2-203-0037",
        name: "TYPE_PARAM_REQUIRES_TYPE",
        severity: Severity::Error,
        message: "parameter must declare a type",
    },
    Rule {
        code: "2-203-0038",
        name: "TYPE_READ_ONLY_RECORD_UPDATE",
        severity: Severity::Error,
        message: "read-only record cannot be updated",
    },
    Rule {
        code: "2-203-0039",
        name: "TYPE_READ_ONLY_RECORD_CONSTRUCTOR",
        severity: Severity::Error,
        message: "read-only record cannot be constructed",
    },
    Rule {
        code: "2-203-0040",
        name: "TYPE_RESULT_IS_IMPLICIT",
        severity: Severity::Error,
        message: "Result return wrapping is implicit",
    },
    Rule {
        code: "2-203-0041",
        name: "TYPE_RETURN_MISMATCH",
        severity: Severity::Error,
        message: "return value type does not match function success type",
    },
    Rule {
        code: "2-203-0042",
        name: "TYPE_SUB_CANNOT_RETURN_VALUE",
        severity: Severity::Error,
        message: "SUB cannot return a value",
    },
    Rule {
        code: "2-203-0043",
        name: "TYPE_UNKNOWN_VALUE",
        severity: Severity::Error,
        message: "value type could not be determined",
    },
    Rule {
        code: "2-203-0044",
        name: "TYPE_UNKNOWN_ENUM_MEMBER",
        severity: Severity::Error,
        message: "enum member does not exist",
    },
    Rule {
        code: "2-203-0045",
        name: "TYPE_UNKNOWN_FIELD",
        severity: Severity::Error,
        message: "record field does not exist",
    },
    Rule {
        code: "2-203-0046",
        name: "TYPE_UNION_INCLUDE_REQUIRES_UNION",
        severity: Severity::Error,
        message: "union includes must name union types",
    },
    Rule {
        code: "2-203-0047",
        name: "TYPE_VARIANT_CONSTRUCTOR_AMBIGUOUS",
        severity: Severity::Error,
        message: "variant constructor name is ambiguous",
    },
    Rule {
        code: "2-203-0048",
        name: "TYPE_ASSIGN_REQUIRES_MUT",
        severity: Severity::Error,
        message: "assignment target must be mutable",
    },
    Rule {
        code: "2-203-0049",
        name: "TYPE_MATCH_PATTERN_MISMATCH",
        severity: Severity::Error,
        message: "match pattern type does not match the scrutinee type",
    },
    Rule {
        code: "2-203-0050",
        name: "TYPE_FOR_EACH_REQUIRES_COLLECTION",
        severity: Severity::Error,
        message: "FOR EACH source must be a List or Map",
    },
    Rule {
        code: "2-203-0051",
        name: "TYPE_LIST_ELEMENT_MISMATCH",
        severity: Severity::Error,
        message: "list element type does not match the expected element type",
    },
    Rule {
        code: "2-203-0052",
        name: "TYPE_MAP_KEY_MISMATCH",
        severity: Severity::Error,
        message: "map key type does not match the declared key type",
    },
    Rule {
        code: "2-203-0053",
        name: "TYPE_MAP_VALUE_MISMATCH",
        severity: Severity::Error,
        message: "map value type does not match the declared value type",
    },
    Rule {
        code: "2-203-0055",
        name: "TYPE_USE_AFTER_MOVE",
        severity: Severity::Error,
        message: "binding is used after move",
    },
    Rule {
        code: "2-203-0056",
        name: "TYPE_COLLECTION_OWNERSHIP_VIOLATION",
        severity: Severity::Error,
        message: "ordinary collections cannot store resource or thread ownership",
    },
    Rule {
        code: "2-203-0058",
        name: "TYPE_DUPLICATE_ARGUMENT_NAME",
        severity: Severity::Error,
        message: "call argument is supplied more than once",
    },
    Rule {
        code: "2-203-0059",
        name: "TYPE_UNKNOWN_ARGUMENT_NAME",
        severity: Severity::Error,
        message: "call argument name does not match any parameter",
    },
    Rule {
        code: "2-203-0060",
        name: "TYPE_MUT_REQUIRES_DEFAULTABLE_TYPE",
        severity: Severity::Error,
        message: "uninitialized mutable binding requires a defaultable type",
    },
    Rule {
        code: "2-203-0061",
        name: "TYPE_REQUIRES_COMPARABLE",
        severity: Severity::Error,
        message: "operation requires a comparable type",
    },
    Rule {
        code: "2-203-0062",
        name: "TYPE_MATCH_NOT_EXHAUSTIVE",
        severity: Severity::Error,
        message: "match cases do not cover every possible value",
    },
    Rule {
        code: "2-203-0063",
        name: "TYPE_THREAD_NOT_SENDABLE",
        severity: Severity::Error,
        message: "thread boundary type is not sendable",
    },
    Rule {
        code: "2-203-0064",
        name: "TYPE_UNION_MEMBER_REQUIRES_TYPE",
        severity: Severity::Error,
        message: "union members must name concrete TYPE declarations",
    },
    Rule {
        code: "2-203-0065",
        name: "TYPE_RECURSIVE_RECORD_REQUIRES_INDIRECTION",
        severity: Severity::Error,
        message: "recursive record cycle must pass through a List, Map, or UNION",
    },
    Rule {
        code: "2-203-0066",
        name: "TYPE_INLINE_TRAP_FALLS_THROUGH",
        severity: Severity::Error,
        message: "inline TRAP handler path neither recovers nor diverges",
    },
    Rule {
        code: "2-203-0067",
        name: "TYPE_RECOVER_TYPE_MISMATCH",
        severity: Severity::Error,
        message: "RECOVER value does not match the trapped expression's success type",
    },
    Rule {
        code: "2-203-0068",
        name: "TYPE_RECOVER_OUTSIDE_INLINE_TRAP",
        severity: Severity::Error,
        message: "RECOVER is valid only inside an inline TRAP handler",
    },
    Rule {
        code: "2-203-0069",
        name: "TYPE_INLINE_TRAP_REQUIRES_FALLIBLE",
        severity: Severity::Error,
        message: "inline TRAP requires a fallible call",
    },
    Rule {
        code: "2-203-0070",
        name: "TYPE_RESULT_NOT_USER_VISIBLE",
        severity: Severity::Error,
        message: "Result is an internal type and cannot be named in user code",
    },
    Rule {
        code: "2-203-0071",
        name: "TYPE_RESULT_NOT_MATCHABLE",
        severity: Severity::Error,
        message: "Ok and Error are not matchable as Result members in user code",
    },
    Rule {
        code: "2-203-0072",
        name: "TYPE_THREAD_RESULT_REMOVED",
        severity: Severity::Error,
        message: "the thread result field is removed; use thread::waitFor",
    },
    Rule {
        code: "2-203-0073",
        name: "SUB_RETURN_FORBIDDEN",
        severity: Severity::Error,
        message: "RETURN is forbidden in a SUB; use EXIT SUB",
    },
    Rule {
        code: "2-203-0074",
        name: "TYPE_SUB_HAS_NO_VALUE",
        severity: Severity::Error,
        message: "a SUB call produces no value and cannot be used in value position",
    },
    Rule {
        code: "2-203-0075",
        name: "EXIT_NO_MATCHING_LOOP",
        severity: Severity::Error,
        message: "EXIT has no matching enclosing loop",
    },
    Rule {
        code: "2-203-0076",
        name: "CONTINUE_NO_MATCHING_LOOP",
        severity: Severity::Error,
        message: "CONTINUE has no matching enclosing loop",
    },
    Rule {
        code: "2-203-0077",
        name: "EXIT_SUB_IN_FUNC",
        severity: Severity::Error,
        message: "EXIT SUB is valid only inside a SUB",
    },
    Rule {
        code: "2-203-0078",
        name: "EXIT_FUNC_FORBIDDEN",
        severity: Severity::Error,
        message: "EXIT FUNC is forbidden; functions must RETURN a value",
    },
    Rule {
        code: "2-203-0079",
        name: "TYPE_EXIT_PROGRAM_REQUIRES_INTEGER",
        severity: Severity::Error,
        message: "EXIT PROGRAM requires an Integer exit code",
    },
    Rule {
        code: "2-203-0080",
        name: "EXIT_PROGRAM_CODE_OUT_OF_RANGE",
        severity: Severity::Error,
        message: "EXIT PROGRAM constant exit code is outside the host range",
    },
    Rule {
        code: "2-203-0081",
        name: "UNREACHABLE_AFTER_EXIT",
        severity: Severity::Error,
        message: "statement is unreachable after EXIT or CONTINUE",
    },
    Rule {
        code: "2-203-0082",
        name: "TYPE_RESOURCE_REQUIRES_RES",
        severity: Severity::Error,
        message: "resource must be bound with RES",
    },
    Rule {
        code: "2-203-0083",
        name: "TYPE_RES_REQUIRES_RESOURCE",
        severity: Severity::Error,
        message: "RES binds only resource types",
    },
    Rule {
        code: "2-203-0084",
        name: "TYPE_RESOURCE_FIELD_FORBIDDEN",
        severity: Severity::Error,
        message: "a record field cannot be a resource",
    },
    Rule {
        code: "2-203-0085",
        name: "TYPE_STATE_INVALID",
        severity: Severity::Error,
        message: "STATE must be a copyable, defaultable data type",
    },
    Rule {
        code: "2-203-0086",
        name: "TYPE_RESOURCE_BORROW_INVALIDATE",
        severity: Severity::Error,
        message: "a borrowed resource cannot be closed, returned, or transferred",
    },
    Rule {
        code: "2-203-0087",
        name: "TYPE_MIXED_RESOURCE_UNION",
        severity: Severity::Error,
        message: "a union must be all-data or all-resource, never mixed",
    },
    Rule {
        code: "2-203-0088",
        name: "TYPE_UNION_STATE_FORBIDDEN",
        severity: Severity::Error,
        message: "a resource union carries no STATE",
    },
    Rule {
        code: "2-203-0089",
        name: "RESOURCE_CLOSE_NOT_NATIVE",
        severity: Severity::Error,
        message: "a resource's CLOSE BY op must be a native LINK function",
    },
    Rule {
        code: "2-203-0090",
        name: "RESOURCE_CLOSE_MISSING",
        severity: Severity::Error,
        message: "a resource's CLOSE BY op names no function in its LINK block",
    },
    Rule {
        code: "2-203-0091",
        name: "RESOURCE_CLOSE_SIGNATURE",
        severity: Severity::Error,
        message: "a close op must consume exactly one RES parameter of its resource",
    },
    Rule {
        code: "2-203-0092",
        name: "NATIVE_CPTR_ESCAPE",
        severity: Severity::Error,
        message: "a raw C ABI type may appear only inside an ABI slot",
    },
    Rule {
        code: "2-203-0093",
        name: "NATIVE_ABI_RESULT_MARKER",
        severity: Severity::Error,
        message: "a native function's ABI result marker is malformed",
    },
    Rule {
        code: "2-203-0094",
        name: "NATIVE_ABI_UNBOUND_SLOT",
        severity: Severity::Error,
        message: "an ABI slot binds to no parameter, CONST pin, or result marker",
    },
    Rule {
        code: "2-203-0095",
        name: "NATIVE_ABI_UNBOUND_PARAM",
        severity: Severity::Error,
        message: "a native function parameter has no matching ABI slot",
    },
    Rule {
        code: "2-203-0096",
        name: "NATIVE_ABI_NO_RESULT",
        severity: Severity::Error,
        message: "a value-returning native function marks no ABI result",
    },
    Rule {
        code: "2-203-0097",
        name: "NATIVE_CONST_OUT",
        severity: Severity::Error,
        message: "a CONST-pinned ABI slot cannot also be OUT",
    },
    Rule {
        code: "2-203-0098",
        name: "NATIVE_CONST_UNKNOWN_SLOT",
        severity: Severity::Error,
        message: "a CONST pin names an unknown ABI slot",
    },
    Rule {
        code: "2-200-0011",
        name: "PROJECT_ENTRY_INVALID",
        severity: Severity::Error,
        message: "project entry point is invalid",
    },
    Rule {
        code: "2-200-0100",
        name: "BUILD_FAILED",
        severity: Severity::Error,
        message: "build failed for an unclassified orchestration reason",
    },
    Rule {
        code: "2-205-0001",
        name: "PACKAGE_VERSION_UNSUPPORTED",
        severity: Severity::Error,
        message: "package binary representation or metadata version is unsupported",
    },
    Rule {
        code: "2-205-0002",
        name: "NATIVE_MANIFEST_INVALID",
        severity: Severity::Error,
        message: "native-link metadata in a package is malformed or inconsistent",
    },
    Rule {
        code: "3-302-0001",
        name: "VERIFICATION_FAILED",
        severity: Severity::Error,
        message: "binary representation or native validation failed",
    },
    Rule {
        code: "3-304-0001",
        name: "TARGET_UNSUPPORTED",
        severity: Severity::Error,
        message: "requested target OS, CPU, or ABI is unsupported",
    },
    Rule {
        code: "5-500-0001",
        name: "LINK_FAILED",
        severity: Severity::Error,
        message: "linking packages, native libraries, symbols, objects, or executables failed",
    },
    Rule {
        code: "6-603-0001",
        name: "LOCKFILE_MISMATCH",
        severity: Severity::Error,
        message: "resolved package state does not match mfb.lock",
    },
    Rule {
        code: "6-605-0001",
        name: "PACKAGE_INVALID",
        severity: Severity::Error,
        message: "package container is malformed or incompatible",
    },
    Rule {
        code: "6-605-0002",
        name: "PACKAGE_SIGNATURE_INVALID",
        severity: Severity::Error,
        message: "package signature, hash, or trust record is missing or invalid",
    },
    Rule {
        code: "3-304-0002",
        name: "PACKAGE_NATIVE_OUTPUT_UNSUPPORTED",
        severity: Severity::Error,
        message: "package projects do not support the requested native output mode",
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

fn rule_for(rule_name: &str) -> &'static Rule {
    RULES
        .iter()
        .find(|rule| rule.name == rule_name)
        .unwrap_or(&Rule {
            code: "0-000-0000",
            name: "UNKNOWN_RULE",
            severity: Severity::Error,
            message: "unknown diagnostic rule",
        })
}
