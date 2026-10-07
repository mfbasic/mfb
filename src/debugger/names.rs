//! Turn the symbol names in an MFBASIC executable back into what the developer
//! wrote, and sort each function into the program's own code, the MFBASIC
//! runtime, or a linked library.
//!
//! A developer's function `f` is emitted as `_mfb_fn_` plus `f`'s IR name with
//! every byte outside `[A-Za-z0-9]` written `_XX` in uppercase hex
//! (`crate::target::shared::nir::symbols::function_symbol`), and
//! a hand-over copy of it appends a literal `$own<hex mask>` after the escaping.
//! An IR name that starts with `#` drops the `#` and uses `_mfb_ifn_` instead:
//! that is a built-in package's function, or a file-`PRIVATE` function whose
//! name carries a 16-hex-digit file hash and a `$`. A built-in package written
//! in MFBASIC names every function in its source `__<package>_<name>`
//! (`crate::internal_name`), so `_mfb_ifn_datetime_5FnormInstant` is
//! `datetime`'s `normInstant`: the public `datetime::<name>` when the package
//! registers a member of that name, else its internal `datetime::#normInstant`
//! (`#` is the compiler's internal-name sigil, which MFBASIC source never
//! contains, so the name cannot be mistaken for anything a developer wrote).
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
    /// `MB`: a built-in package's MFBASIC code.
    Builtin,
    /// `MR`: the MFBASIC runtime and compiler-made support code.
    Runtime,
    /// `SL`: a system library function the program imports, bound by the
    /// loader as the program starts.
    Linked,
    /// `SD`: a function in a library the program loaded while running.
    Dynamic,
}

impl Origin {
    pub(crate) fn code(self) -> &'static str {
        match self {
            Origin::User => "MU",
            Origin::Builtin => "MB",
            Origin::Runtime => "MR",
            Origin::Linked => "SL",
            Origin::Dynamic => "SD",
        }
    }

    /// The type a `list func` filter word names: its code, as the listing
    /// prints it.
    pub(crate) fn parse(code: &str) -> Option<Origin> {
        [
            Origin::User,
            Origin::Builtin,
            Origin::Runtime,
            Origin::Linked,
            Origin::Dynamic,
        ]
        .into_iter()
        .find(|origin| origin.code() == code)
    }

    /// Whether the code is native — the runtime's or a system library's —
    /// rather than written in MFBASIC.
    fn native(self) -> bool {
        !matches!(self, Origin::User | Origin::Builtin)
    }

    /// The legend line `list func` prints for this type.
    pub(crate) fn legend(self) -> &'static str {
        match self {
            Origin::User => "MFB User: code you wrote, in your program or a package it imports",
            Origin::Builtin => {
                "MFB Builtin: a built-in package's code; a #name is internal to the package \
                 (you can break on it, not call it)"
            }
            Origin::Runtime => "MFB Runtime: the MFBASIC runtime every program carries",
            Origin::Linked => {
                "System Linked: a system library function the program imports (via the linker)"
            }
            Origin::Dynamic => {
                "System Dynamic: a function in a library the program loaded while running \
                 (via the dynamic loader)"
            }
        }
    }
}

use super::FunctionSymbol;
use crate::target::shared::runtime::{supported_helper_specs, symbol_for_call};

const USER_PREFIX: &str = "_mfb_fn_";
const INTERNAL_PREFIX: &str = "_mfb_ifn_";
const RUNTIME_PREFIX: &str = "_mfb_rt_";
const OWN_SUFFIX: &str = "$own";

/// Classify any function symbol: one in a library the program loaded while
/// running or imports by its C name, else as [`classify_defined`].
pub(crate) fn classify(function: &FunctionSymbol) -> (Origin, Option<String>) {
    if function.loaded {
        (Origin::Dynamic, Some(linked_name(&function.name)))
    } else if function.linked {
        (Origin::Linked, Some(linked_name(&function.name)))
    } else {
        classify_defined(&function.name)
    }
}

/// The names an MFBASIC function and a native one share — the program's `main`
/// and the runtime's `_main`, a developer's `exit` and the C library's — which
/// are shown with a tag saying which is meant: `main <mfb>`, `main <native>`.
/// A function's name here is its written name, else its [`plain_name`], as
/// listings show it.
#[derive(Debug, Default)]
pub(crate) struct Collisions(std::collections::HashSet<String>);

impl Collisions {
    pub(crate) fn of(functions: &[FunctionSymbol]) -> Self {
        let mut sides = std::collections::HashMap::<String, [bool; 2]>::new();
        for function in functions {
            let (origin, written) = classify(function);
            let name = written.unwrap_or_else(|| plain_name(&function.name));
            sides.entry(name).or_default()[usize::from(origin.native())] = true;
        }
        Self(
            sides
                .into_iter()
                .filter(|(_, [mfb, native])| *mfb && *native)
                .map(|(name, _)| name)
                .collect(),
        )
    }

    /// The tag for `name`, a function of `origin`'s: `<mfb>` or `<native>`
    /// when the other side has a function of that name too.
    pub(crate) fn tag(&self, name: &str, origin: Origin) -> Option<&'static str> {
        self.0
            .contains(name)
            .then_some(if origin.native() { "<native>" } else { "<mfb>" })
    }

    /// `name` with its tag, if it needs one: `main <mfb>`.
    pub(crate) fn tagged(&self, name: String, origin: Origin) -> String {
        match self.tag(&name, origin) {
            Some(tag) => format!("{name} {tag}"),
            None => name,
        }
    }
}

/// Classify a function the program defines, and recover the name its author
/// wrote: for runtime code, the package member it implements, if any.
pub(crate) fn classify_defined(symbol: &str) -> (Origin, Option<String>) {
    if let Some(rest) = symbol.strip_prefix(USER_PREFIX) {
        return (Origin::User, Some(written_name(&decode(strip_own(rest)))));
    }
    if let Some(rest) = symbol.strip_prefix(INTERNAL_PREFIX) {
        let decoded = decode(strip_own(rest));
        if let Some(name) = private_name(&decoded) {
            return (Origin::User, Some(written_name(name)));
        }
        if let Some(name) = builtin_name(&decoded) {
            return (Origin::Builtin, Some(name));
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

/// The name of a built-in package's function from its decoded IR name less the
/// `#` (`datetime_normInstant`, `collections_drop$LogLine`): `datetime::add`
/// for a member the package registers, `datetime::#normInstant` for one of its
/// internal functions; `None` when no package owns the name.
fn builtin_name(decoded: &str) -> Option<String> {
    let base = decoded.split_once('$').map_or(decoded, |(base, _)| base);
    let (package, name) = base.split_once('_')?;
    let package = crate::codegen::registry::registry()
        .packages()
        .iter()
        .find(|candidate| candidate.import_name() == package)?;
    if name.is_empty() {
        return None;
    }
    let public = package
        .function(name)
        .is_some_and(|function| !function.internal_only);
    Some(if public {
        format!("{package}::{name}", package = package.import_name())
    } else {
        format!("{package}::#{name}", package = package.import_name())
    })
}

/// How the host's executable format spells a C function's symbol: Mach-O
/// with a leading `_` (`_malloc` is `malloc`), ELF bare (`_exit` is `_exit`).
/// The debugger debugs only programs in its own host's format.
pub(crate) const C_PREFIX: &str = if cfg!(target_os = "linux") { "" } else { "_" };

/// The name a linked function has in C: its symbol less [`C_PREFIX`].
pub(crate) fn linked_name(symbol: &str) -> String {
    symbol.strip_prefix(C_PREFIX).unwrap_or(symbol).to_string()
}

/// The name a symbol with no written name is shown by: an MFBASIC
/// executable's own symbols (`_main`, `_mfb_…`), which every format spells
/// with a leading `_`, less it; any other, a system library's, as
/// [`linked_name`] reads it.
pub(crate) fn plain_name(symbol: &str) -> String {
    match symbol.strip_prefix('_') {
        Some(rest) if symbol == "_main" || symbol.starts_with("_mfb") => rest.to_string(),
        _ => linked_name(symbol),
    }
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
    fn a_built_in_package_s_code_reads_as_package_and_name() {
        for (symbol, name) in [
            // Members the package registers.
            ("_mfb_ifn_color_5Fbrighten", "color::brighten"),
            ("_mfb_ifn_collections_5Fdrop_24LogLine", "collections::drop"),
            ("_mfb_ifn_color_5Fbrighten$own1", "color::brighten"),
            // The package's internal functions.
            ("_mfb_ifn_datetime_5FnormInstant", "datetime::#normInstant"),
            (
                "_mfb_ifn_color_5FbrightenChannel",
                "color::#brightenChannel",
            ),
        ] {
            assert_eq!(
                classify_defined(symbol),
                (Origin::Builtin, Some(name.to_string())),
                "{symbol}"
            );
        }
    }

    #[test]
    fn runtime_code_has_no_written_name() {
        for symbol in [
            // `nosuchpkg` is no built-in package.
            "_mfb_ifn_nosuchpkg_5Fparse",
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
        if cfg!(target_os = "linux") {
            // ELF spells a C function bare: `_exit` is not `exit`.
            assert_eq!(linked_name("_exit"), "_exit");
            assert_eq!(linked_name("__libc_start_main"), "__libc_start_main");
        } else {
            assert_eq!(linked_name("_malloc"), "malloc");
            assert_eq!(linked_name("___error"), "__error");
        }
        assert_eq!(linked_name("plain"), "plain");
        // The executable's own symbols read the same on every format.
        assert_eq!(plain_name("_main"), "main");
        assert_eq!(plain_name("_mfb_rt_print"), "mfb_rt_print");
        assert_eq!(plain_name(&format!("{C_PREFIX}write")), "write");
    }

    /// A C function's symbol as the host's format spells it.
    fn c(name: &str) -> String {
        format!("{C_PREFIX}{name}")
    }

    fn symbol(name: &str, linked: bool) -> FunctionSymbol {
        FunctionSymbol {
            name: name.to_string(),
            linked,
            loaded: false,
            address: None,
            size: None,
        }
    }

    #[test]
    fn a_name_both_sides_use_is_tagged_with_the_side() {
        let collisions = Collisions::of(&[
            symbol("_mfb_fn_main", false),
            symbol("_main", false),
            symbol("_mfb_fn_exit", false),
            symbol(&c("exit"), true),
            symbol("_mfb_fn_isPrime", false),
            symbol(&c("malloc"), true),
        ]);
        let tagged = |name: &str, origin| collisions.tagged(name.to_string(), origin);
        assert_eq!(tagged("main", Origin::User), "main <mfb>");
        assert_eq!(tagged("main", Origin::Runtime), "main <native>");
        assert_eq!(tagged("exit", Origin::User), "exit <mfb>");
        assert_eq!(tagged("exit", Origin::Linked), "exit <native>");
        assert_eq!(tagged("isPrime", Origin::User), "isPrime");
        assert_eq!(tagged("malloc", Origin::Linked), "malloc");
        // Two native functions of one name are no MFBASIC/native collision.
        let native = Collisions::of(&[symbol(&c("write"), true), symbol(&c("write"), false)]);
        assert_eq!(native.tagged("write".to_string(), Origin::Linked), "write");
    }

    #[test]
    fn origins_sort_and_print_in_listing_order() {
        let order = [
            Origin::User,
            Origin::Builtin,
            Origin::Runtime,
            Origin::Linked,
            Origin::Dynamic,
        ];
        assert!(order.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(order.map(Origin::code), ["MU", "MB", "MR", "SL", "SD"]);
    }
}
