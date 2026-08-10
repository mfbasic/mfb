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

/// `(name, integer-literal, message)` for every runtime registry row. Hand-maintained
/// here (the source of truth for `errorCode::Err*` values); the spec topic
/// `src/docs/spec/diagnostics/02_error-codes.md` documents the same registry and
/// the `table_matches_registry` test keeps the two in agreement.
///
/// The value is a decimal *string*, not an integer, on purpose. `constant_value`
/// feeds `builtins::package_constant_value` — the single constant path shared with
/// `math::`, whose values are non-integer decimals (`math::pi` is
/// `"3.141592653589793"`), so the shared return type must be a string. That path
/// lowers to `IrValue::Const { value: String }` (`src/ir/lower.rs`), an
/// intentionally text-erased literal that carries every scalar kind uniformly and
/// round-trips through the binary/JSON package formats. Storing these codes as
/// strings keeps errorCode on that shared path and matches the IR's own
/// representation, so no parse/format step is needed. A typed `u32` here would buy
/// nothing: it would be re-stringified at the IR boundary anyway, while forking
/// errorCode off the shared constant path.
pub(crate) const ERRORCODE_CONSTANTS: &[(&str, &str, &str)] = &[
    ("ErrUnknown", "77050000", "Unclassified standard-package failure."),
    ("ErrIndexOutOfRange", "77050001", "List or string index/range is outside valid bounds."),
    ("ErrInvalidArgument", "77050002", "Argument value is not valid for the requested operation."),
    ("ErrInvalidFormat", "77050003", "Text parse or non-finite numeric representation conversion failed."),
    ("ErrNotFound", "77050004", "Requested item, key, file, or resource was not found."),
    ("ErrAlreadyExists", "77050005", "Create operation conflicts with an existing item."),
    ("ErrPermissionDenied", "77050006", "Operation is not permitted by the host environment."),
    ("ErrUnsupported", "77050007", "Operation is not supported by the implementation or platform."),
    ("ErrTimeout", "77050008", "Operation did not complete before its deadline."),
    ("ErrInterrupted", "77050009", "Operation was interrupted before completion."),
    ("ErrOutOfMemory", "77010001", "Allocation failed."),
    ("ErrPathNotFound", "77030001", "Filesystem path does not exist."),
    ("ErrInvalidPath", "77030002", "Filesystem path string is invalid for the host platform."),
    ("ErrAccessDenied", "77030003", "Filesystem access was denied."),
    ("ErrReadFailed", "77020001", "Read operation failed."),
    ("ErrWriteFailed", "77020002", "Write or flush operation failed."),
    ("ErrEndOfFile", "77020003", "Read operation reached end of file where a value was required."),
    ("ErrResourceClosed", "77030004", "Resource handle is already closed."),
    ("ErrResourceBusy", "77030005", "Resource is unavailable, locked, busy, or not in the required empty state."),
    ("ErrEncoding", "77020004", "Text encoding or decoding failed."),
    ("ErrInputFailed", "77020005", "Standard input operation failed."),
    ("ErrAddressInvalid", "77070001", "Network host, address, or port is invalid."),
    ("ErrAddressNotFound", "77070002", "Network host name or address could not be resolved."),
    ("ErrNetworkFailed", "77070003", "Network operation failed before a connection was established."),
    ("ErrConnectionClosed", "77070004", "Socket peer closed the connection or the connection is no longer usable."),
    ("ErrMessageTooLarge", "77070007", "Datagram or message exceeds the requested or supported size."),
    ("ErrOverflow", "77050010", "Arithmetic overflow or numeric conversion outside the destination range."),
    ("ErrCloseFailed", "77030006", "Resource close operation failed."),
    ("ErrNativeBindingUnavailable", "77030007", "Native `LINK` binding library or symbol could not be loaded at startup (`dlopen`/`dlsym` failed)."),
    ("ErrNativeBindingCallFailed", "77030008", "Native `LINK` binding call failed its `SUCCESS_ON` gate."),
    ("ErrTlsFailed", "77070008", "TLS handshake, certificate validation, SNI validation, or protocol operation failed."),
    ("ErrUnderflow", "77050011", "Arithmetic underflow below the destination range."),
    ("ErrFloatDomain", "77050012", "Floating-point operation domain is invalid (negative `sqrt`, non-positive `log`/`log10`, out-of-range `asin`/`acos`, a non-whole or negative `^` exponent, or a `Float MOD 0`). Divide-by-zero is not reported here — `x / 0` produces `±Inf`/`NaN` caught at the observation boundary as `ErrFloatOverflow`/`ErrFloatNaN`."),
    ("ErrFloatNaN", "77050013", "Floating-point operation produced a NaN result."),
    ("ErrFloatInf", "77050014", "Floating-point operation produced an infinity result."),
    ("ErrFloatOverflow", "77050015", "Floating-point arithmetic overflowed to infinity."),
    ("ErrWrapped", "77060001", "Generic wrapper code for adding context while preserving the underlying message."),
    ("ErrAuthenticationFailed", "77050016", "Authenticated decryption failed: the message authentication tag did not verify."),
    ("ErrAudioUnavailable", "77050017", "Audio backend library or device is unavailable (no `libasound.so.2`, no audio device, or capture authorization denied)."),
    ("ErrAudioDevice", "77050018", "Audio device open, configuration, or stream operation failed."),
    ("ErrInvalidContext", "77050019", "Operation was invoked from a thread that is not permitted to perform it (e.g. reading stdin from a thread that has not called `thread::openStdIn`)."),
    ("ErrWrongMode", "77050020", "Operation requires a presentation mode the program is not in: in an `--app` build, `term::*` and the console-reading `io::` calls (`io::input`/`io::readLine`/`io::readChar`) require `app::Mode.Console` (plan-62-E)."),
    ("ErrResourceMoved", "77030009", "Resource handle was moved to another thread by `thread::transfer` and is no longer usable by the sender."),
    ("ErrNativeBufferOverrun", "77030010", "Native `LINK` `OUT CBuffer` callee wrote past its declared `SIZE` (buffer overrun detected)."),
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
        .find(|(constant_name, _, _)| *constant_name == member)
        .map(|(_, value, _)| *value)
}

/// The `(code, message)` for a runtime error *name* (e.g. `"ErrIndexOutOfRange"`),
/// as declared in a builtin's [`BuiltinFunction::errors`], or `None` if the name
/// is not a known `errorCode` constant. This is the codegen-facing lookup: the
/// native error-emission path resolves a builtin's declared error to the concrete
/// code and message it passes to `emit_error_code_return`. Distinct from
/// [`constant_value`], which takes the package-qualified `errorCode.<Name>` key and
/// returns only the code for constant folding.
///
/// [`BuiltinFunction::errors`]: super::descriptor::BuiltinFunction::errors
pub(crate) fn runtime_error(name: &str) -> Option<(&'static str, &'static str)> {
    ERRORCODE_CONSTANTS
        .iter()
        .find(|(constant_name, _, _)| *constant_name == name)
        .map(|(_, code, message)| (*code, *message))
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
            .map(|(name, value, _)| (name.to_string(), value.to_string()))
            .collect();
        assert_eq!(
            table, rows,
            "errorCode table does not match error_codes.md",
        );

        // Every exported constant resolves through the package-qualified API.
        for (name, value, _) in ERRORCODE_CONSTANTS {
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
