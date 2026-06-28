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
    name.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}
