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
/// mapped back to the readable `__` prefix so error messages never expose it.
/// Non-internal names are returned unchanged.
pub fn display_name(name: &str) -> std::borrow::Cow<'_, str> {
    match strip_sigil(name) {
        Some(rest) => std::borrow::Cow::Owned(format!("__{rest}")),
        None => std::borrow::Cow::Borrowed(name),
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
}
