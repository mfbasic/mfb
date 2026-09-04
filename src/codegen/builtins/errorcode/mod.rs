//! The built-in `errorCode` package (plan-06-errorcodes.md, clean-room migration).
//!
//! `errorCode` is a flat set of `Integer` constants — one per runtime registry row
//! in the embedded spec topic `src/docs/spec/diagnostics/02_error-codes.md` (`mfb
//! spec diagnostics error-codes`) — and nothing else: no callables, no builtin
//! types, no resource, no source companion, no resolver. A reference such as
//! `errorCode::ErrNotFound` types as `Integer` and folds to an integer literal before
//! lowering (the shared `math::pi` constant mechanism), so there is no runtime helper,
//! no codegen, and no binary-representation change. Constants are keyed
//! package-qualified (`"errorCode.<Name>"`) exactly like `math.pi`, resolved through
//! the registry's [`constant_value`](crate::codegen::registry::constant_value) path.
//!
//! Each row carries FOUR columns: name, integer value, **message**, and
//! **message-symbol** (`_mfb_str_error_*`). The name+value are the folded constant;
//! the message+symbol are the single authority for the whole error-**emission**
//! codegen path (`raise_error_into`, `emit_error_code_return`, the data-object
//! tables), consulted by the bare-name
//! [`runtime_error`](crate::codegen::registry::runtime_error) /
//! [`runtime_error_emission`](crate::codegen::registry::runtime_error_emission) /
//! [`runtime_error_triple`](crate::codegen::registry::runtime_error_triple) free fns.
//!
//! The value is a decimal *string* on purpose: it feeds the shared constant path
//! (`IrValue::Const { value: String }`), which carries every scalar kind uniformly.
//! Most message symbols follow the `_mfb_str_error_<snake(name-without-Err)>`
//! convention, but several are historical and irregular (`ErrOutOfMemory` →
//! `_mfb_str_error_allocation`, `ErrWriteFailed` → `_mfb_str_error_output`,
//! `ErrResourceBusy` → `_mfb_str_error_directory_not_empty`, …); byte-identity
//! requires reproducing them exactly, so the symbol is stored, not derived.

// --- codegen tier imports (migration) ---
use crate::codegen::registry::{Registry, RegistryConstant, RegistryPackage};
const MODULE_INTRO: &str =
    "Named `Integer` constants for the runtime error codes a `TRAP` can match on";

const MODULE_DESC: &str = r#"`errorCode` is a flat set of named `Integer` constants — one per runtime error
code — and nothing else. It exports no functions and declares no types. Its whole
purpose is to let a `TRAP` handler compare `err.code` against a name instead of a
magic number: `errorCode::ErrPathNotFound` rather than `77020001`.

Each name resolves to the same `Integer` the runtime puts in `Error.code`, so a
comparison is an ordinary integer equality, with no conversion at all.
The constants are compile-time values; referencing one costs nothing at run time.
The specification topic `mfb spec diagnostics error-codes` is the single source of
truth for the Name → Integer mapping."#;

/// One `errorCode` scalar constant: an `Integer` folding value plus the two
/// error-emission columns (message + message data-object symbol).
fn constant(
    name: &'static str,
    value: &'static str,
    message: &'static str,
    symbol: &'static str,
) -> RegistryConstant {
    RegistryConstant {
        name,
        type_name: "Integer",
        value: Some(value),
        components: None,
        message: Some(message),
        symbol: Some(symbol),
    }
}

/// Register the `errorCode` package on the clean-room registry.
///
/// A constants-only package: the migration's 45 `Integer` constants plus any added
/// since (`ErrBadPixelCount`, `ErrBadFontFile`), and nothing else. Each legacy row is
/// reproduced verbatim from the legacy `ERRORCODE_CONSTANTS` table — the values are
/// decimal strings equal to the hyphen-stripped `G-SSS-EEEE` code, and several
/// message symbols are historical/irregular, so they are copied exactly (byte-identity
/// of the error-emission path depends on it).
pub(crate) fn register(r: &mut Registry) {
    let mut pkg = RegistryPackage::new("errorCode", MODULE_INTRO, MODULE_DESC);

    pkg.add_constant(constant("ErrUnknown", "77050000", "Unclassified standard-package failure.", "_mfb_str_error_unknown"))
        .add_constant(constant("ErrIndexOutOfRange", "77050001", "List or string index/range is outside valid bounds.", "_mfb_str_error_index_out_of_range"))
        .add_constant(constant("ErrInvalidArgument", "77050002", "Argument value is not valid for the requested operation.", "_mfb_str_error_invalid_argument"))
        .add_constant(constant("ErrInvalidFormat", "77050003", "Text parse or non-finite numeric representation conversion failed.", "_mfb_str_error_invalid_format"))
        .add_constant(constant("ErrNotFound", "77050004", "Requested item, key, file, or resource was not found.", "_mfb_str_error_not_found"))
        .add_constant(constant("ErrAlreadyExists", "77050005", "Create operation conflicts with an existing item.", "_mfb_str_error_already_exists"))
        .add_constant(constant("ErrPermissionDenied", "77050006", "Operation is not permitted by the host environment.", "_mfb_str_error_permission_denied"))
        .add_constant(constant("ErrUnsupported", "77050007", "Operation is not supported by the implementation or platform.", "_mfb_str_error_unsupported"))
        .add_constant(constant("ErrTimeout", "77050008", "Operation did not complete before its deadline.", "_mfb_str_error_timeout"))
        .add_constant(constant("ErrInterrupted", "77050009", "Operation was interrupted before completion.", "_mfb_str_error_interrupted"))
        .add_constant(constant("ErrOutOfMemory", "77010001", "Allocation failed.", "_mfb_str_error_allocation"))
        .add_constant(constant("ErrPathNotFound", "77030001", "Filesystem path does not exist.", "_mfb_str_error_path_not_found"))
        .add_constant(constant("ErrInvalidPath", "77030002", "Filesystem path string is invalid for the host platform.", "_mfb_str_error_invalid_path"))
        .add_constant(constant("ErrAccessDenied", "77030003", "Filesystem access was denied.", "_mfb_str_error_access_denied"))
        .add_constant(constant("ErrReadFailed", "77020001", "Read operation failed.", "_mfb_str_error_read"))
        .add_constant(constant("ErrWriteFailed", "77020002", "Write or flush operation failed.", "_mfb_str_error_output"))
        .add_constant(constant("ErrEndOfFile", "77020003", "Read operation reached end of file where a value was required.", "_mfb_str_error_eof"))
        .add_constant(constant("ErrResourceClosed", "77030004", "Resource handle is already closed.", "_mfb_str_error_resource_closed"))
        // Symbol is historical `_mfb_str_error_directory_not_empty`: code 77030005's
        // only fixed-helper emission is the "directory not empty" case, so byte-identity
        // pins the table symbol to that name (there is no `_mfb_str_error_resource_busy`).
        .add_constant(constant("ErrResourceBusy", "77030005", "Resource is unavailable, locked, busy, or not in the required empty state.", "_mfb_str_error_directory_not_empty"))
        .add_constant(constant("ErrEncoding", "77020004", "Text encoding or decoding failed.", "_mfb_str_error_encoding"))
        .add_constant(constant("ErrInputFailed", "77020005", "Standard input operation failed.", "_mfb_str_error_input"))
        .add_constant(constant("ErrAddressInvalid", "77070001", "Network host, address, or port is invalid.", "_mfb_str_error_address_invalid"))
        .add_constant(constant("ErrAddressNotFound", "77070002", "Network host name or address could not be resolved.", "_mfb_str_error_address_not_found"))
        .add_constant(constant("ErrNetworkFailed", "77070003", "Network operation failed before a connection was established.", "_mfb_str_error_network_failed"))
        .add_constant(constant("ErrConnectionClosed", "77070004", "Socket peer closed the connection or the connection is no longer usable.", "_mfb_str_error_connection_closed"))
        .add_constant(constant("ErrMessageTooLarge", "77070007", "Datagram or message exceeds the requested or supported size.", "_mfb_str_error_message_too_large"))
        .add_constant(constant("ErrOverflow", "77050010", "Arithmetic overflow or numeric conversion outside the destination range.", "_mfb_str_error_overflow"))
        .add_constant(constant("ErrCloseFailed", "77030006", "Resource close operation failed.", "_mfb_str_error_close_failed"))
        .add_constant(constant("ErrNativeBindingUnavailable", "77030007", "Native `LINK` binding library or symbol could not be loaded at startup (`dlopen`/`dlsym` failed).", "_mfb_str_error_native_link_load"))
        .add_constant(constant("ErrNativeBindingCallFailed", "77030008", "Native `LINK` binding call failed its `SUCCESS_ON` gate.", "_mfb_str_error_native_link_call"))
        .add_constant(constant("ErrTlsFailed", "77070008", "TLS handshake, certificate validation, SNI validation, or protocol operation failed.", "_mfb_str_error_tls_failed"))
        .add_constant(constant("ErrUnderflow", "77050011", "Arithmetic underflow below the destination range.", "_mfb_str_error_underflow"))
        .add_constant(constant("ErrFloatDomain", "77050012", "Floating-point operation domain is invalid (negative `sqrt`, non-positive `log`/`log10`, out-of-range `asin`/`acos`, a non-whole or negative `^` exponent, or a `Float MOD 0`). Divide-by-zero is not reported here — `x / 0` produces `±Inf`/`NaN` caught at the observation boundary as `ErrFloatOverflow`/`ErrFloatNaN`.", "_mfb_str_error_float_domain"))
        .add_constant(constant("ErrFloatNaN", "77050013", "Floating-point operation produced a NaN result.", "_mfb_str_error_float_nan"))
        .add_constant(constant("ErrFloatInf", "77050014", "Floating-point operation produced an infinity result.", "_mfb_str_error_float_inf"))
        .add_constant(constant("ErrFloatOverflow", "77050015", "Floating-point arithmetic overflowed to infinity.", "_mfb_str_error_float_overflow"))
        .add_constant(constant("ErrWrapped", "77060001", "Generic wrapper code for adding context while preserving the underlying message.", "_mfb_str_error_wrapped"))
        .add_constant(constant("ErrAuthenticationFailed", "77050016", "Authenticated decryption failed: the message authentication tag did not verify.", "_mfb_str_error_authentication_failed"))
        .add_constant(constant("ErrAudioUnavailable", "77050017", "Audio backend library or device is unavailable (no `libasound.so.2`, no audio device, or capture authorization denied).", "_mfb_str_error_audio_unavailable"))
        .add_constant(constant("ErrAudioDevice", "77050018", "Audio device open, configuration, or stream operation failed.", "_mfb_str_error_audio_device"))
        .add_constant(constant("ErrInvalidContext", "77050019", "Operation was invoked from a thread that is not permitted to perform it (e.g. reading stdin from a thread that has not called `thread::openStdIn`).", "_mfb_str_error_invalid_context"))
        .add_constant(constant("ErrWrongMode", "77050020", "Operation requires a presentation mode the program is not in. In an `--app` build, `term::*` requires `app::Mode.Console` — the character grid exists only there, so `app::Mode.None` and `app::Mode.Canvas` both trap. The console-reading `io::` calls (`io::input`/`io::readLine`/`io::readChar`) need only a window to take key events from, so they trap in `app::Mode.None` alone.", "_mfb_str_error_wrong_mode"))
        .add_constant(constant("ErrResourceMoved", "77030009", "Resource handle was moved to another thread by `thread::transfer` and is no longer usable by the sender.", "_mfb_str_error_resource_moved"))
        .add_constant(constant("ErrNativeBufferOverrun", "77030010", "Native `LINK` `OUT CBuffer` callee wrote past its declared `SIZE` (buffer overrun detected).", "_mfb_str_error_native_buffer_overrun"))
        .add_constant(constant("ErrSpawnFailed", "77080001", "Child process could not be spawned (fork/exec failed, or the program was not found).", "_mfb_str_error_spawn_failed"))
        // plan-98-B: an RGBA8 image is exactly `width * height * 4` bytes, so a
        // wrong-length pixel list is a distinct, actionable mistake rather than a
        // generic bad argument — the message can say what the count should have been.
        .add_constant(constant("ErrBadPixelCount", "77050021", "Pixel list length does not match the image dimensions: an RGBA8 image needs exactly `width * height * 4` bytes.", "_mfb_str_error_bad_pixel_count"))
        .add_constant(constant("ErrBadFontFile", "77050022", "File is not a font this build can read: it must be TrueType outlines (sfnt `0x00010000` or `true`), not CFF/OpenType-PostScript, a collection, or WOFF, and its `head.unitsPerEm` must be within 16..16384.", "_mfb_str_error_bad_font_file"))
        .add_constant(constant("ErrBadImageFile", "77050023", "File is not an image this build can decode: `canvas::loadImage` reads PNG, and refuses anything else — including a PNG whose chunks, filters or compressed data are malformed, that is larger than 16384 pixels a side or 16,777,216 pixels, or whose compressed data does not fit the image it declares.", "_mfb_str_error_bad_image_file"))
        // plan-120-A: two structural-input mistakes that `json::parse` used to
        // report as the same generic `ErrInvalidFormat`. Both are deliberately
        // named for the mistake rather than for `json`, because they belong to
        // any recursive-descent reader of untrusted text — the regex engine's
        // own nesting cap and any future parser report the same two things.
        .add_constant(constant("ErrDepthExceeded", "77050024", "Structural nesting exceeds the implementation depth limit. Distinct from `ErrInvalidFormat`: the text is well-formed, it is just nested deeper than the reader will descend (`json::parse` stops at 256).", "_mfb_str_error_depth_exceeded"))
        .add_constant(constant("ErrInvalidSurrogate", "77050025", "A `\\u` escape encodes an unpaired surrogate. Strings are Unicode text, so a high surrogate must be followed by a `\\u` low surrogate and a lone low surrogate is never valid.", "_mfb_str_error_invalid_surrogate"));

    r.add_package(pkg);
}

// Man/spec citation anchor: the `errorCode/*` man page and the diagnostics §02 spec
// ground their constant-registry facts here (`register` is the table authority;
// `table_matches_registry` is the drift guard).

#[cfg(test)]
mod tests {
    use crate::codegen::registry::{self, registry, RegistryConstant};

    /// The migrated `errorCode` package's constants, in registration order.
    fn errorcode_constants() -> &'static [RegistryConstant] {
        registry()
            .resolve_package("errorCode")
            .expect("errorCode registered")
            .constants()
    }

    /// The hand-maintained table must exactly reproduce the "Constant Registry" rows
    /// of the spec topic, with the integer value equal to the hyphen-stripped
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

        let table: Vec<(String, String)> = errorcode_constants()
            .iter()
            .map(|c| {
                (
                    c.name.to_string(),
                    c.value
                        .expect("errorCode constant folds to a value")
                        .to_string(),
                )
            })
            .collect();
        assert_eq!(table, rows, "errorCode table does not match error_codes.md");

        // Every exported constant resolves through the package-qualified fold API.
        for c in errorcode_constants() {
            let key = format!("errorCode.{}", c.name);
            assert!(
                registry::is_package_constant(&key),
                "`{key}` not recognized"
            );
            assert_eq!(
                registry::constant_type_name(&key),
                Some(crate::types::ParameterType::Integer)
            );
            assert_eq!(registry::constant_value(&key), c.value);
        }
    }

    #[test]
    fn spec_example_values() {
        // The concrete examples standard_package.md §13 and plan §6.2 call out.
        assert_eq!(
            registry::constant_value("errorCode.ErrNotFound"),
            Some("77050004")
        );
        assert_eq!(
            registry::constant_value("errorCode.ErrInvalidArgument"),
            Some("77050002")
        );
    }

    #[test]
    fn rejects_unknown_and_unqualified() {
        assert!(!registry::is_package_constant("errorCode.NotARealName"));
        assert!(!registry::is_package_constant("ErrNotFound"));
        // `math.abs` is a migrated math *function*, not a constant (`math.pi` now IS a
        // registry constant via `add_constant`, so it is no longer a negative case).
        assert!(!registry::is_package_constant("math.abs"));
        assert_eq!(registry::constant_value("errorCode.NotARealName"), None);
        // The bare-name emission lookup rejects an unknown name too.
        assert_eq!(registry::runtime_error("NotARealName"), None);
        assert_eq!(registry::runtime_error_emission("NotARealName"), None);
        assert_eq!(registry::runtime_error_triple("NotARealName"), None);
    }

    /// plan-88-D drift guard: the `errorCode` constants are now the single metadata
    /// authority for every runtime error (code/message/symbol), so the rows must be
    /// unique on name, code, AND message symbol — a duplicate would make
    /// `runtime_error*` / `constant_value` non-deterministic and silently mis-emit.
    #[test]
    fn table_has_no_duplicate_names_or_codes() {
        let mut names = std::collections::HashSet::new();
        let mut codes = std::collections::HashSet::new();
        let mut symbols = std::collections::HashSet::new();
        for c in errorcode_constants() {
            assert!(
                names.insert(c.name),
                "duplicate errorCode name `{}`",
                c.name
            );
            let code = c.value.expect("value");
            assert!(codes.insert(code), "duplicate errorCode code `{code}`");
            let symbol = c.symbol.expect("symbol");
            assert!(
                symbols.insert(symbol),
                "duplicate message symbol `{symbol}`"
            );
        }
        // The migration reproduced every legacy row: 45 constants. Codes added since
        // are counted separately so that claim stays checkable — a bare total would
        // let a *lost* legacy row hide behind a newly added one.
        //
        //   +1  ErrBadPixelCount (plan-98-B): an RGBA8 image is exactly
        //       `width * height * 4` bytes, so a wrong-length pixel list is a
        //       distinct, actionable mistake rather than a generic bad argument.
        //   +1  ErrBadFontFile (plan-98-G): "this is not a font I can read" is a
        //       different mistake from "this file is missing", and the two need
        //       different fixes — one is a path typo, the other is the wrong format.
        //   +1  ErrBadImageFile (plan-98-G): the same distinction for `loadImage`.
        //       Separate from `ErrBadFontFile` because a program can load both, and
        //       "which of the two files was wrong" is the first thing its handler
        //       wants to know.
        //   +1  ErrDepthExceeded (plan-120-A): a document that is well-formed but
        //       nested past the reader's descent limit is not a malformed one, and
        //       the caller's response differs — raise the limit or reject the
        //       source, versus fix the syntax.
        //   +1  ErrInvalidSurrogate (plan-120-A): a `\u` escape naming half a
        //       surrogate pair is well-formed JSON that has no Unicode scalar
        //       behind it. Distinct from a grammar error for the same reason:
        //       the document must be re-encoded, not re-punctuated.
        const LEGACY_ROWS: usize = 45;
        const ADDED_SINCE_MIGRATION: &[&str] = &[
            "ErrBadPixelCount",
            "ErrBadFontFile",
            "ErrBadImageFile",
            "ErrDepthExceeded",
            "ErrInvalidSurrogate",
        ];
        for added in ADDED_SINCE_MIGRATION {
            assert!(names.contains(added), "{added} is not in the table");
        }
        assert_eq!(
            names.len() - ADDED_SINCE_MIGRATION.len(),
            LEGACY_ROWS,
            "the migration's legacy rows must all still be present"
        );
    }

    /// plan-88-D drift guard: every error a builtin declares in its `errors` list must
    /// be a real `errorCode` constant name. Iterate BOTH registries — the clean-room
    /// registry's migrated packages and the legacy `REGISTRY`'s functions — so a bad
    /// declaration in either fails fast in release test runs.
    #[test]
    fn every_builtin_declared_error_is_a_table_name() {
        for package in registry().packages() {
            for function in package.functions() {
                for implementation in function.implementations() {
                    for name in &implementation.errors {
                        assert!(
                            registry::runtime_error(name).is_some(),
                            "{}.{} declares error `{name}`, not an errorCode constant",
                            package.import_name(),
                            function.name,
                        );
                    }
                }
            }
        }
    }

    /// The bare-name error-emission free fns agree with the folded value and each
    /// other for a representative row (message + historical/irregular symbol).
    #[test]
    fn emission_fns_agree_with_the_constant() {
        assert_eq!(
            registry::runtime_error("ErrOutOfMemory"),
            Some(("77010001", "Allocation failed."))
        );
        assert_eq!(
            registry::runtime_error_emission("ErrOutOfMemory"),
            Some(("77010001", "_mfb_str_error_allocation"))
        );
        assert_eq!(
            registry::runtime_error_triple("ErrWriteFailed"),
            Some((
                "77020002",
                "Write or flush operation failed.",
                "_mfb_str_error_output"
            ))
        );
    }
}
