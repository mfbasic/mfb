//! Turn the symbol names in an MFBASIC executable back into what the developer
//! wrote, and sort each function into the program's own code, the MFBASIC
//! runtime, or a linked library.
//!
//! A developer's function `f` is emitted as `_mfb_fn_` plus `f`'s IR name with
//! every byte outside `[A-Za-z0-9]` written `_XX` in uppercase hex
//! (`crate::target::shared::nir::symbols::function_symbol`), and
//! a hand-over copy of it appends a literal `$own<hex mask>` after the escaping.
//! An IR name that starts with `#` drops the `#` and uses `_mfb_ifn_` instead:
//! that is a built-in package's internal function, or a file-`PRIVATE` function
//! whose name carries a 16-hex-digit file hash and a `$`.
//!
//! A built-in package member the runtime implements natively is emitted at
//! `_mfb_rt_<family>_<call>` (`crate::target::shared::runtime::symbol_for_call`),
//! so `io::print` is `_mfb_rt_io_io_print`. The compiler's helper catalog maps
//! that symbol back to the member it implements.

/// Where a function in the listing comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Origin {
    /// `MU`: code the developer wrote, in the program or a package it imports.
    User,
    /// `MR`: the MFBASIC runtime and compiler-made support code.
    Runtime,
    /// `LN`: a function the program calls in a linked system library.
    Linked,
}

impl Origin {
    pub(crate) fn code(self) -> &'static str {
        match self {
            Origin::User => "MU",
            Origin::Runtime => "MR",
            Origin::Linked => "LN",
        }
    }
}

use crate::target::shared::runtime::{supported_helper_specs, symbol_for_call};

const USER_PREFIX: &str = "_mfb_fn_";
const INTERNAL_PREFIX: &str = "_mfb_ifn_";
const RUNTIME_PREFIX: &str = "_mfb_rt_";
const OWN_SUFFIX: &str = "$own";

/// Classify a function the program defines, and recover the name its author
/// wrote: for runtime code, the package member it implements, if any.
pub(crate) fn classify_defined(symbol: &str) -> (Origin, Option<String>) {
    if let Some(rest) = symbol.strip_prefix(USER_PREFIX) {
        return (Origin::User, Some(written_name(&decode(strip_own(rest)))));
    }
    if let Some(rest) = symbol.strip_prefix(INTERNAL_PREFIX) {
        if let Some(name) = private_name(&decode(strip_own(rest))) {
            return (Origin::User, Some(written_name(name)));
        }
    }
    if symbol.starts_with(RUNTIME_PREFIX) {
        return (Origin::Runtime, package_member(symbol));
    }
    (Origin::Runtime, None)
}

/// The built-in package member a runtime helper implements, as the developer
/// writes it (`_mfb_rt_io_io_print` is `io::print`), or `None` for the runtime's
/// own support code and for helpers no MFBASIC source can call by name.
fn package_member(symbol: &str) -> Option<String> {
    static MEMBERS: std::sync::OnceLock<std::collections::HashMap<String, String>> =
        std::sync::OnceLock::new();
    MEMBERS
        .get_or_init(|| {
            let registry = crate::codegen::registry::registry();
            supported_helper_specs()
                .iter()
                .filter(|spec| registry.owning_package(spec.call).is_some())
                .map(|spec| {
                    (
                        symbol_for_call(spec.helper, spec.call),
                        spec.call.replacen('.', "::", 1),
                    )
                })
                .collect()
        })
        .get(symbol)
        .cloned()
}

/// The name a linked function has in C: the Mach-O symbol less its leading `_`.
pub(crate) fn linked_name(symbol: &str) -> String {
    symbol.strip_prefix('_').unwrap_or(symbol).to_string()
}

/// Drop a hand-over copy's `$own<hex>` suffix; the copy is the same function.
fn strip_own(fragment: &str) -> &str {
    match fragment.rfind(OWN_SUFFIX) {
        Some(at)
            if fragment.len() > at + OWN_SUFFIX.len()
                && fragment[at + OWN_SUFFIX.len()..]
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit()) =>
        {
            &fragment[..at]
        }
        _ => fragment,
    }
}

/// Undo the `_XX` byte escaping. A `_` not followed by two uppercase hex digits
/// is kept as written.
fn decode(fragment: &str) -> String {
    let bytes = fragment.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'_' {
            if let Some(byte) = bytes
                .get(at + 1..at + 3)
                .and_then(|hex| std::str::from_utf8(hex).ok())
                .filter(|hex| {
                    hex.bytes()
                        .all(|b| b.is_ascii_digit() || b.is_ascii_uppercase())
                })
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
            {
                out.push(byte);
                at += 3;
                continue;
            }
        }
        out.push(bytes[at]);
        at += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The name inside a file-`PRIVATE` function's `<16 hex>$name`, or `None` for a
/// built-in package's internal function.
fn private_name(decoded: &str) -> Option<&str> {
    let (hash, name) = decoded.split_at_checked(16)?;
    let name = name.strip_prefix('$')?;
    (hash.bytes().all(|byte| byte.is_ascii_hexdigit()) && !name.is_empty()).then_some(name)
}

/// The name the developer wrote for a decoded IR function name.
fn written_name(ir: &str) -> String {
    // An imported package's function: `<16 hex identity>.<package>.<name>`.
    if let Some((identity, rest)) = ir.split_once('.') {
        if identity.len() == 16 && identity.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            if let Some((package, name)) = rest.split_once('.') {
                return format!("{package}::{}", written_name(name));
            }
        }
    }
    if let Some(rest) = ir.strip_prefix("$lambda") {
        if rest.bytes().all(|byte| byte.is_ascii_digit()) {
            return "lambda".to_string();
        }
    }
    // A parameter's default value: `$default$<function>$<index>`.
    if let Some(rest) = ir.strip_prefix("$default$") {
        return written_name(rest);
    }
    // An overload or generic instance: `name$Type$…`.
    match ir.split_once('$') {
        Some((name, _)) if !name.is_empty() => name.to_string(),
        _ => ir.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(symbol: &str) -> String {
        match classify_defined(symbol) {
            (Origin::User, Some(name)) => name,
            other => panic!("{symbol} classified as {other:?}"),
        }
    }

    #[test]
    fn a_developer_s_functions_read_as_written() {
        assert_eq!(user("_mfb_fn_main"), "main");
        assert_eq!(user("_mfb_fn_isPrime"), "isPrime");
        assert_eq!(user("_mfb_fn_next_5Fseed"), "next_seed");
        assert_eq!(user("_mfb_fn_a_5F_5Fb"), "a__b");
    }

    #[test]
    fn a_hand_over_copy_reads_as_the_function_it_copies() {
        assert_eq!(user("_mfb_fn_directionFor$own4"), "directionFor");
        assert_eq!(user("_mfb_fn_next_5Fseed$own1f"), "next_seed");
        // Only a hex mask makes the suffix; anything else is part of the name.
        assert_eq!(user("_mfb_fn_town$ownz"), "town");
    }

    #[test]
    fn overloads_generics_lambdas_and_defaults_read_as_their_function() {
        assert_eq!(user("_mfb_fn_f_24List_24OF_24Integer"), "f");
        assert_eq!(user("_mfb_fn_f_24Integer_24AS_24Text"), "f");
        assert_eq!(user("_mfb_fn__24lambda0"), "lambda");
        assert_eq!(user("_mfb_fn__24lambda12"), "lambda");
        assert_eq!(user("_mfb_fn__24default_24greet_241"), "greet");
    }

    #[test]
    fn a_package_function_reads_as_package_and_name() {
        assert_eq!(user("_mfb_fn_04187eb9abe9cc82_2Ewpkg_2Ew"), "wpkg::w");
        assert_eq!(
            user("_mfb_fn_04187eb9abe9cc82_2Ewpkg_2Emake_5Fit_24Integer"),
            "wpkg::make_it"
        );
    }

    #[test]
    fn a_file_private_function_is_the_developer_s() {
        assert_eq!(user("_mfb_ifn_0123456789abcdef_24helper"), "helper");
        assert_eq!(user("_mfb_ifn_0123456789abcdef_24helper$own2"), "helper");
    }

    #[test]
    fn runtime_code_has_no_written_name() {
        for symbol in [
            "_mfb_ifn_json_5Fparse",
            "_mfb_rt_print",
            "_mfb_arena_alloc",
            "_mfb_debug_shutdown",
            "_main",
            "__mfb_init_globals_demo",
            "_mfb_linker_thunk",
        ] {
            assert_eq!(
                classify_defined(symbol),
                (Origin::Runtime, None),
                "{symbol}"
            );
        }
    }

    #[test]
    fn a_runtime_helper_reads_as_the_package_member_it_implements() {
        for (symbol, member) in [
            ("_mfb_rt_io_io_print", "io::print"),
            ("_mfb_rt_fs_fs_open", "fs::open"),
            ("_mfb_rt_net_net_lookup", "net::lookup"),
        ] {
            assert_eq!(
                classify_defined(symbol),
                (Origin::Runtime, Some(member.to_string())),
                "{symbol}"
            );
        }
        // A helper only the compiler calls has no written name.
        assert_eq!(
            classify_defined("_mfb_rt_net_net_connectTcpAddr"),
            (Origin::Runtime, None)
        );
    }

    #[test]
    fn a_broken_escape_is_kept_as_written() {
        assert_eq!(user("_mfb_fn_a_5"), "a_5");
        assert_eq!(user("_mfb_fn_a_zz"), "a_zz");
        assert_eq!(user("_mfb_fn_a_5f"), "a_5f");
    }

    #[test]
    fn linked_functions_drop_the_leading_underscore() {
        assert_eq!(linked_name("_malloc"), "malloc");
        assert_eq!(linked_name("___error"), "__error");
        assert_eq!(linked_name("plain"), "plain");
    }

    #[test]
    fn origins_sort_and_print_in_listing_order() {
        assert!(Origin::User < Origin::Runtime && Origin::Runtime < Origin::Linked);
        assert_eq!(
            [Origin::User, Origin::Runtime, Origin::Linked].map(Origin::code),
            ["MU", "MR", "LN"]
        );
    }
}
