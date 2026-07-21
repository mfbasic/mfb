//! Compiler-internal symbol naming.
//!
//! The built-in packages `json`, `regex`, and `collections` are injected as
//! MFBASIC source (see `src/builtins/*.mfb`). Their private helpers follow a
//! `__pkg_name` convention to avoid clashing with user code. That convention is
//! only *probabilistic* — nothing stops a user who imports the package from
//! declaring a colliding `__pkg_name` of their own.
//!
//! To make those names *unforgeable*, the lexer rewrites a leading `__` to
//! [`INTERNAL_SIGIL`] when it lexes an internal file (`AstFile::internal`). The
//! sigil is a character the lexer never accepts inside a user identifier, so a
//! user can never author the rewritten name. It survives through the AST and IR
//! — where it guarantees no collision with any user symbol — and is mapped to a
//! reserved native-symbol namespace (`_mfb_ifn_…`) at code generation.

/// Untypeable marker for compiler-internal symbols.
///
/// `#` is rejected by the lexer in normal mode (`MFB_LEX_UNEXPECTED_CHARACTER`),
/// so a user identifier can never contain it. `$` is deliberately avoided: it is
/// already used for synthesized lambda names (`$lambda0`) and generic-type
/// sanitization in the monomorphizer.
pub const INTERNAL_SIGIL: char = '#';

/// Rewrite a `__`-prefixed internal name to its sigil form
/// (`__json_parse` -> `#json_parse`). Names without the `__` prefix — including
/// the public package types like `Json` — are returned unchanged.
pub fn internalize(name: &str) -> String {
    match name.strip_prefix("__") {
        Some(rest) => format!("{INTERNAL_SIGIL}{rest}"),
        None => name.to_string(),
    }
}

/// The non-sigil remainder of an internal name (`#json_parse` -> `json_parse`),
/// or `None` when `name` is not a compiler-internal symbol.
pub fn strip_sigil(name: &str) -> Option<&str> {
    name.strip_prefix(INTERNAL_SIGIL)
}

/// Render an internal name for a user-facing diagnostic. The untypeable sigil is
/// mapped back to the readable `__` prefix so error messages never expose it. A
/// file-scoped PRIVATE name (`#<hash>$name`, see [`mangle_private`]) demangles to
/// its plain source name. Non-internal names are returned unchanged.
pub fn display_name(name: &str) -> std::borrow::Cow<'_, str> {
    match strip_sigil(name) {
        Some(rest) => match strip_private_hash(rest) {
            Some(plain) => std::borrow::Cow::Owned(plain.to_string()),
            None => std::borrow::Cow::Owned(format!("__{rest}")),
        },
        None => std::borrow::Cow::Borrowed(name),
    }
}

/// Width of the hex file-scope hash embedded in a mangled PRIVATE name.
const FILE_HASH_HEX_LEN: usize = 16;

/// 64-bit FNV-1a of a project-relative source path, rendered as 16 lowercase hex
/// digits. Deterministic and machine-independent (the path is project-relative
/// and `/`-normalized), so native goldens stay reproducible.
pub fn file_scope_hash(project_relative_path: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in project_relative_path.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// Mangle a file-local PRIVATE top-level name to an untypeable, file-unique
/// internal name `#<hash>$<name>`. The leading `#` sigil makes it unforgeable by
/// user code; the `<hash>` (see [`file_scope_hash`]) ties it to its declaring
/// file so same-named privates in different files never collide downstream. `$`
/// is the monomorphizer's existing mangle separator, so the name survives to
/// native codegen unchanged.
pub fn mangle_private(file_hash: &str, name: &str) -> String {
    format!("{INTERNAL_SIGIL}{file_hash}${name}")
}

/// If `rest` (a sigil-stripped name) is a mangled PRIVATE name `<hash>$<plain>`,
/// return `<plain>`; otherwise `None`.
fn strip_private_hash(rest: &str) -> Option<&str> {
    let (hash, plain) = rest.split_once('$')?;
    if hash.len() == FILE_HASH_HEX_LEN && hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        Some(plain)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internalize_swaps_double_underscore_prefix() {
        assert_eq!(internalize("__json_parse"), "#json_parse");
        assert_eq!(internalize("__collections_sort"), "#collections_sort");
    }

    #[test]
    fn internalize_leaves_public_names_untouched() {
        assert_eq!(internalize("Json"), "Json");
        assert_eq!(internalize("parse"), "parse");
        // A single leading underscore is not the internal prefix.
        assert_eq!(internalize("_json"), "_json");
    }

    #[test]
    fn strip_sigil_round_trips_internalize() {
        let internal = internalize("__regex_match");
        assert_eq!(internal, "#regex_match");
        assert_eq!(strip_sigil(&internal), Some("regex_match"));
        // A plain `__`-prefixed name (as a user would type) is not internal.
        assert_eq!(strip_sigil("__regex_match"), None);
    }

    #[test]
    fn file_scope_hash_is_deterministic_and_16_hex() {
        let a = file_scope_hash("src/a.mfb");
        let b = file_scope_hash("src/b.mfb");
        assert_eq!(a.len(), 16);
        assert!(a.bytes().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
        assert_eq!(a, file_scope_hash("src/a.mfb"));
    }

    #[test]
    fn mangle_private_round_trips_through_display_name() {
        let hash = file_scope_hash("src/helpers.mfb");
        let mangled = mangle_private(&hash, "helper");
        assert_eq!(mangled, format!("#{hash}$helper"));
        // A mangled PRIVATE name demangles to its plain source name for diagnostics.
        assert_eq!(display_name(&mangled), "helper");
        // A plain sigil name (builtin internal) still maps to the `__` form.
        assert_eq!(display_name("#json_parse"), "__json_parse");
    }

    #[test]
    fn display_name_restores_double_underscore_only_for_internal_names() {
        // Sigil form maps back to the readable `__` prefix (owned).
        let shown = display_name("#json_parse");
        assert_eq!(shown, "__json_parse");
        assert!(matches!(shown, std::borrow::Cow::Owned(_)));
        // Non-internal names pass through unchanged (`Cow::Borrowed`, no allocation).
        let plain = display_name("parse");
        assert_eq!(plain, "parse");
        assert!(matches!(plain, std::borrow::Cow::Borrowed(_)));
    }
}
