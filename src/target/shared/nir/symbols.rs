pub(crate) fn function_symbol(name: &str) -> String {
    // Compiler-internal functions (injected built-ins, sigil-prefixed) get a
    // reserved symbol namespace that user functions — always routed through
    // `_mfb_fn_` — can never reach, so a sigil name cannot collide at link time.
    match crate::internal_name::strip_sigil(name) {
        Some(rest) => format!("_mfb_ifn_{}", symbol_fragment(rest)),
        None => format!("_mfb_fn_{}", symbol_fragment(name)),
    }
}

pub(crate) fn global_symbol(project: &str, name: &str) -> String {
    format!(
        "_mfb_global_{}_{}",
        symbol_fragment(project),
        symbol_fragment(name)
    )
}

pub(crate) fn global_initializer_name(project: &str) -> String {
    format!("__mfb_init_globals_{}", symbol_fragment(project))
}

pub(crate) fn symbol_fragment(name: &str) -> String {
    // Escape every byte that is not `[A-Za-z0-9]` — including `_` itself — to an
    // unambiguous `_XX` two-hex-digit form. The previous mapping folded `$`,
    // space, and a literal `_` all onto `_`, so the distinct monomorphized name
    // `f$List$OF$Integer` (from an overloaded `f(List OF Integer)`) and a plain
    // user function literally named `f_List_OF_Integer` both mangled to
    // `_mfb_fn_f_List_OF_Integer` and silently shadowed each other at link time
    // (bug-161). Escaping the interior `_` bytes keeps them apart, matching the
    // same fix already applied to `link_thunk_symbol` (bug-139.6).
    let mut out = String::new();
    for byte in name.bytes() {
        if byte.is_ascii_alphanumeric() {
            out.push(byte as char);
        } else {
            out.push_str(&format!("_{byte:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod symbol_fragment_tests {
    use super::{function_symbol, global_symbol, symbol_fragment};

    #[test]
    fn plain_alphanumerics_pass_through() {
        assert_eq!(symbol_fragment("main"), "main");
        assert_eq!(symbol_fragment("foo123"), "foo123");
    }

    #[test]
    fn underscore_is_escaped_not_passed_through() {
        // A literal `_` must become `_5F`, not stay `_`, so it cannot be confused
        // with an escaped mangling separator.
        assert_eq!(symbol_fragment("a_b"), "a_5Fb");
    }

    #[test]
    fn mangled_overload_and_underscore_name_no_longer_collide() {
        // bug-161: `f(List OF Integer)` mangles to `f$List$OF$Integer`; a plain
        // user function can be literally named `f_List_OF_Integer`. Both used to
        // fold to `_mfb_fn_f_List_OF_Integer`; they must now be distinct symbols.
        let overload = function_symbol("f$List$OF$Integer");
        let underscored = function_symbol("f_List_OF_Integer");
        assert_ne!(overload, underscored);
    }

    #[test]
    fn global_symbol_escapes_both_parts() {
        // The `$`-mangled and `_`-literal forms must also stay apart for globals.
        assert_ne!(
            global_symbol("p", "g$List$OF$Integer"),
            global_symbol("p", "g_List_OF_Integer"),
        );
    }
}
