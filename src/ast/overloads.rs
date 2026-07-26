//! Parameter-type extraction and whitespace normalization for DOC-overload
//! resolution.
//!
//! A `DOC` header may name a specific overload by its parameter types; matching
//! that against a `FUNC`'s declared parameters requires both sides to be
//! whitespace-normalized identically. [`param_types`] reads a function's
//! parameter types (normalized), [`normalize_types`] normalizes an explicit
//! list, and [`normalize_ws`] is the shared primitive they and the DOC-header
//! parser build on. Before bug-343 A2/B1 these were four copies under five names
//! across `doc`, `ir`, and `resolver`, with `normalize_ws` itself exported from
//! the unrelated DOC-header parser.

use super::Function;

/// Collapse internal whitespace runs to single spaces and trim.
pub fn normalize_ws(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A function's declared parameter type names, each whitespace-normalized.
pub fn param_types(function: &Function) -> Vec<String> {
    function
        .params
        .iter()
        .map(|param| normalize_ws(param.type_name.as_deref().unwrap_or("")))
        .collect()
}

/// Whitespace-normalize each type name in `types`.
pub fn normalize_types(types: &[String]) -> Vec<String> {
    types.iter().map(|t| normalize_ws(t)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_ws_collapses_and_trims() {
        assert_eq!(normalize_ws("  List   of  \tString "), "List of String");
        assert_eq!(normalize_ws(""), "");
        assert_eq!(normalize_ws("Int"), "Int");
    }

    #[test]
    fn normalize_types_normalizes_each_entry() {
        let got = normalize_types(&["  Int ".to_string(), "Map  of\tString".to_string()]);
        assert_eq!(got, vec!["Int".to_string(), "Map of String".to_string()]);
    }
}
