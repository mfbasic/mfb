//! Built-in `errorCode` package (plan-06-errorcodes.md): a flat set of `Integer`
//! constants, one per runtime registry row in the embedded spec topic
//! `src/docs/spec/diagnostics/02_error-codes.md` (`mfb spec diagnostics error-codes`).
//!
//! This mirrors the `math` constant mechanism (`math::pi` and friends): a
//! reference such as `errorCode::ErrNotFound` types as `Integer` and folds to an
//! integer literal before lowering, so there is no runtime helper, no codegen,
//! and no binary-representation change. Constants are keyed package-qualified
//! (`"errorCode.<Name>"`) exactly like `math.pi`.
//!
//! The constant table is maintained by hand in this file (`ERRORCODE_CONSTANTS`).
//! The spec topic above documents the same registry; the `table_matches_registry`
//! test keeps the two in agreement.

use super::descriptor::BuiltinModule;

/// `(name, integer-literal)` for every runtime registry row. Hand-maintained
/// here (the source of truth for `errorCode::Err*` values); the spec topic
/// `src/docs/spec/diagnostics/02_error-codes.md` documents the same registry and
/// the `table_matches_registry` test keeps the two in agreement.
pub(crate) const ERRORCODE_CONSTANTS: &[(&str, &str)] = &[
    ("ErrUnknown", "77050000"),
    ("ErrIndexOutOfRange", "77050001"),
    ("ErrInvalidArgument", "77050002"),
    ("ErrInvalidFormat", "77050003"),
    ("ErrNotFound", "77050004"),
    ("ErrAlreadyExists", "77050005"),
    ("ErrPermissionDenied", "77050006"),
    ("ErrUnsupported", "77050007"),
    ("ErrTimeout", "77050008"),
    ("ErrInterrupted", "77050009"),
    ("ErrOutOfMemory", "77010001"),
    ("ErrPathNotFound", "77030001"),
    ("ErrInvalidPath", "77030002"),
    ("ErrAccessDenied", "77030003"),
    ("ErrReadFailed", "77020001"),
    ("ErrWriteFailed", "77020002"),
    ("ErrEndOfFile", "77020003"),
    ("ErrResourceClosed", "77030004"),
    ("ErrResourceBusy", "77030005"),
    ("ErrEncoding", "77020004"),
    ("ErrInputFailed", "77020005"),
    ("ErrAddressInvalid", "77070001"),
    ("ErrAddressNotFound", "77070002"),
    ("ErrNetworkFailed", "77070003"),
    ("ErrConnectionClosed", "77070004"),
    ("ErrMessageTooLarge", "77070007"),
    ("ErrOverflow", "77050010"),
    ("ErrCloseFailed", "77030006"),
    ("ErrNativeBindingUnavailable", "77030007"),
    ("ErrNativeBindingCallFailed", "77030008"),
    ("ErrTlsFailed", "77070008"),
    ("ErrUnderflow", "77050011"),
    ("ErrFloatDomain", "77050012"),
    ("ErrFloatNaN", "77050013"),
    ("ErrFloatInf", "77050014"),
    ("ErrFloatOverflow", "77050015"),
    ("ErrWrapped", "77060001"),
    ("ErrAuthenticationFailed", "77050016"),
    ("ErrAudioUnavailable", "77050017"),
    ("ErrAudioDevice", "77050018"),
    ("ErrInvalidContext", "77050019"),
    ("ErrWrongMode", "77050020"),
    ("ErrResourceMoved", "77030009"),
    ("ErrNativeBufferOverrun", "77030010"),
];

// plan-72-J: `errorCode` exposes only `Integer` constants — no callables, builtin
// types, or source companion — so its descriptor carries an empty function/type
// list and no resolver. The hand-maintained `ERRORCODE_CONSTANTS` table stays the
// metadata authority for `errorCode::Err*` values (the descriptor vocabulary
// models callables, not folded constants); this static exists only to keep the
// `BuiltinRegistry` exhaustive, so plan-72-BB can collapse the aggregate arms for
// every package unconditionally. The census confirms zero descriptor-owned
// helpers (`helpers 0`, `srcglue/btypes/custom 0`).
pub(crate) static ERRORCODE: BuiltinModule = BuiltinModule {
    name: "errorCode",
    functions: &[],
    types: &[],
    source: None,
    resolver: None,
};

/// The package-qualified key (`"errorCode.<Name>"`) of a known constant, or
/// `None` for anything else.
fn member(name: &str) -> Option<&str> {
    name.strip_prefix("errorCode.")
}

pub(crate) fn is_errorcode_constant(name: &str) -> bool {
    constant_value(name).is_some()
}

pub(crate) fn constant_type_name(name: &str) -> Option<&'static str> {
    is_errorcode_constant(name).then_some("Integer")
}

pub(crate) fn constant_value(name: &str) -> Option<&'static str> {
    let member = member(name)?;
    ERRORCODE_CONSTANTS
        .iter()
        .find(|(constant_name, _)| *constant_name == member)
        .map(|(_, value)| *value)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The hand-maintained table must exactly reproduce the "Constant Registry"
    /// rows of the spec topic, with the integer value equal to the hyphen-stripped
    /// `G-SSS-EEEE` code. This is the drift guard from plan-06-errorcodes.md §6.1.
    #[test]
    fn table_matches_registry() {
        let doc = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/docs/spec/diagnostics/02_error-codes.md"
        ));

        let mut in_section = false;
        let mut rows: Vec<(String, String)> = Vec::new();
        for line in doc.lines() {
            if line.starts_with("## ") {
                in_section = line.contains("Constant Registry");
                continue;
            }
            if !in_section || !line.trim_start().starts_with("| `") {
                continue;
            }
            let cells: Vec<&str> = line.split('|').map(str::trim).collect();
            if cells.len() < 4 {
                continue;
            }
            let code = cells[1].trim_matches('`');
            let integer = cells[2].trim_matches('`');
            let name = cells[3].trim_matches('`');
            if code.is_empty() || integer.is_empty() || name.is_empty() {
                continue;
            }
            assert_eq!(
                code.replace('-', ""),
                integer,
                "registry row `{name}`: code `{code}` != integer `{integer}`",
            );
            rows.push((name.to_string(), integer.to_string()));
        }

        assert!(!rows.is_empty(), "no registry rows parsed");

        let table: Vec<(String, String)> = ERRORCODE_CONSTANTS
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect();
        assert_eq!(
            table, rows,
            "errorCode table does not match error_codes.md",
        );

        // Every exported constant resolves through the package-qualified API.
        for (name, value) in ERRORCODE_CONSTANTS {
            let key = format!("errorCode.{name}");
            assert!(is_errorcode_constant(&key), "`{key}` not recognized");
            assert_eq!(constant_type_name(&key), Some("Integer"));
            assert_eq!(constant_value(&key), Some(*value));
        }
    }

    #[test]
    fn spec_example_values() {
        // The concrete examples standard_package.md §13 and plan §6.2 call out.
        assert_eq!(constant_value("errorCode.ErrNotFound"), Some("77050004"));
        assert_eq!(
            constant_value("errorCode.ErrInvalidArgument"),
            Some("77050002")
        );
    }

    #[test]
    fn rejects_unknown_and_unqualified() {
        assert!(!is_errorcode_constant("errorCode.NotARealName"));
        assert!(!is_errorcode_constant("ErrNotFound"));
        assert!(!is_errorcode_constant("math.pi"));
        assert_eq!(constant_value("errorCode.NotARealName"), None);
    }

    // plan-72-J migration gate: `errorCode` owns no callables, so the descriptor
    // is deliberately empty — the point of this letter is only that the registry
    // enumerates `errorCode` alongside every other package. Prove the module is
    // registered, carries the empty shape, and rejects every name lookup cleanly.
    #[test]
    fn descriptor_is_registered_and_empty() {
        use crate::builtins::descriptor::{DefaultResolver, REGISTRY};

        let module = REGISTRY.module("errorCode").expect("errorCode is registered");
        assert_eq!(module.name, "errorCode");
        assert!(module.functions.is_empty());
        assert!(module.types.is_empty());
        assert!(module.source.is_none());
        assert!(module.resolver.is_none());

        // No callable is owned: membership and every derivation is empty/None.
        assert!(!DefaultResolver::contains(&ERRORCODE, "errorCode.ErrNotFound"));
        assert!(REGISTRY.function("errorCode.ErrNotFound").is_none());
        assert!(REGISTRY.function("errorCode.anything").is_none());
        assert_eq!(DefaultResolver::arity(&ERRORCODE, "errorCode.ErrNotFound"), None);

        // The registry stays well-formed with `errorCode` appended.
        assert_eq!(REGISTRY.duplicate_module_name(), None);
        assert_eq!(REGISTRY.duplicate_function_name(), None);
    }
}
