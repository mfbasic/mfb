pub(crate) mod bits;
pub(crate) mod collections;
pub(crate) mod crypto;
pub(crate) mod csv;
pub(crate) mod datetime;
pub(crate) mod encoding;
pub(crate) mod errorcode;
pub(crate) mod fs;
pub(crate) mod general;
pub(crate) mod http;
pub(crate) mod io;
pub(crate) mod json;
pub(crate) mod math;
pub(crate) mod net;
pub(crate) mod os;
pub(crate) mod regex;
pub(crate) mod resource;
pub(crate) mod strings;
pub(crate) mod term;
pub(crate) mod testing;
pub(crate) mod thread;
pub(crate) mod tls;
pub(crate) mod vector;

pub(crate) use resource::{ResourceInfo, ResourceKind, ResourceRegistry};

pub(crate) fn is_builtin_import(name: &str) -> bool {
    matches!(
        name,
        "bits"
            | "collections"
            | "crypto"
            | "csv"
            | "datetime"
            | "encoding"
            | "errorCode"
            | "fs"
            | "http"
            | "io"
            | "json"
            | "math"
            | "net"
            | "os"
            | "regex"
            | "strings"
            | "term"
            | "thread"
            | "tls"
            | "vector"
    )
}

pub(crate) fn is_builtin_type(name: &str) -> bool {
    crypto::is_builtin_type(name)
        || datetime::is_builtin_type(name)
        || fs::is_builtin_type(name)
        || http::is_builtin_type(name)
        || io::is_builtin_type(name)
        || json::is_builtin_type(name)
        || net::is_builtin_type(name)
        || term::is_builtin_type(name)
        || thread::is_builtin_type(name)
        || tls::is_builtin_type(name)
        || vector::is_builtin_type(name)
}

/// The internal helper a built-in package provides as an **override** of an
/// overridable general built-in (`toString`, `len`, …) over one of its value
/// types (plan-01-overload.md §B.2). A general call `f(x)` whose sole argument
/// has such a type routes to this `__pkg_name` helper instead of the scalar
/// builtin; the name is internalized at lowering so it never collides with the
/// builtin dispatch symbol. Keyed by `(builtin, arg_type)`; the only row today is
/// the `toString(net::Url)` renderer (plan-03-http.md §A.3).
pub(crate) fn general_override_target(builtin: &str, arg_type: &str) -> Option<&'static str> {
    match (builtin, arg_type) {
        ("toString", t) if t == net::URL_TYPE => Some("__net_urlToString"),
        // The nine `vector::` value records render `"(x, y, z)"` via a companion
        // renderer (plan-06-vector.md §4.18).
        ("toString", t) if vector::is_builtin_type(t) => vector::tostring_override_target(t),
        _ => None,
    }
}

/// Resolve a package-qualified built-in type reference (`net.Url`,
/// `http.Response`) to its bare internal type id, or `None` when it is not a
/// qualified built-in type (plan-03-http.md §A.1).
pub(crate) fn qualified_builtin_type(qualified: &str) -> Option<String> {
    let (package, member) = qualified.split_once('.')?;
    if !is_builtin_import(package) {
        return None;
    }
    if is_builtin_type(member) {
        Some(member.to_string())
    } else {
        None
    }
}

pub(crate) fn resource_close_function(type_name: &str) -> Option<&'static str> {
    resource::builtin_resource_close_function(type_name)
}

pub(crate) fn is_resource_type(type_name: &str) -> bool {
    resource::is_builtin_resource_type(type_name)
}

pub(crate) fn is_thread_sendable_resource_type(type_name: &str) -> bool {
    resource::is_builtin_sendable_resource_type(type_name)
}

/// The bare native lowering name for a migrated `collections::`/`strings::`
/// member (plan-01-functions.md §5). The native code generator stays keyed on the
/// original bare names (`get`, `transform`, `find`, `mid`, `replace`, ...), so the
/// IR call target for these members is dequalified back to the bare name. Returns
/// `None` for every other call (including the `collections::` source generics,
/// which the monomorphizer rewrites to `__collections_X` instead).
pub(crate) fn native_builtin_target(name: &str) -> Option<&'static str> {
    if let Some(member) = name.strip_prefix("strings.") {
        return match member {
            "find" => Some("find"),
            "mid" => Some("mid"),
            "replace" => Some("replace"),
            _ => None,
        };
    }
    match collections::native_member_bare(name)? {
        "get" => Some("get"),
        "getOr" => Some("getOr"),
        "set" => Some("set"),
        "append" => Some("append"),
        "prepend" => Some("prepend"),
        "insert" => Some("insert"),
        "removeAt" => Some("removeAt"),
        "removeKey" => Some("removeKey"),
        "keys" => Some("keys"),
        "values" => Some("values"),
        "hasKey" => Some("hasKey"),
        "contains" => Some("contains"),
        "forEach" => Some("forEach"),
        "transform" => Some("transform"),
        "filter" => Some("filter"),
        "reduce" => Some("reduce"),
        "sum" => Some("sum"),
        "find" => Some("find"),
        "mid" => Some("mid"),
        "replace" => Some("replace"),
        _ => None,
    }
}

/// Whether an inline `TRAP` on `target` would reach codegen's raw-`TRAP` path
/// with **no** lowering to emit — an inline-lowered builtin (string/collection
/// member, `bits::*` op, or `len`/`toString`/`typeName`) that is neither
/// raw-supported (`lower_inline_builtin_raw`) nor infallible
/// (`lower_inline_infallible_raw`). Such a target has its machine code spliced in
/// at the call site and owns no standalone symbol, so the generic raw path would
/// emit `bl <target>` to a symbol that does not exist.
///
/// After plan-26 this set is **empty**: every inline builtin is either
/// raw-supported or infallible, so an inline `TRAP` is legal on all of them
/// (uniform surface). The predicate survives only as the **codegen backstop**
/// (`lower_ops` `CallResult`), which fails loudly if a *future* inline builtin is
/// added to `native_builtin_target` without also giving it a raw or infallible
/// lowering — catching the mistake instead of miscompiling. The front-end no
/// longer rejects anything here (the old `TYPE_INLINE_TRAP_ON_INLINED_BUILTIN`
/// diagnostic was retired in plan-26-C).
///
/// Excluded (already trappable): the conversion builtins
/// `toInt`/`toFloat`/`toFixed`/`toByte` (`lower_inline_conversion_raw`) and every
/// `runtime::helper_for_call` target (`lower_runtime_helper_call`); user
/// `FUNC`/`SUB` calls carry real symbols and match none of the member forms here.
///
/// `target` is the canonical, dot-qualified callee (`strings.find`,
/// `collections.get`, `bits.sl`) or a bare inline general-builtin name (`len`,
/// `toString`, `typeName`).
pub(crate) fn inline_trap_unsupported(target: &str) -> bool {
    (bits::is_bits_call(target)
        || native_builtin_target(target).is_some()
        || matches!(target, "len" | "toString" | "typeName"))
        && !inline_builtin_raw_supported(target)
        && !inline_builtin_is_infallible(target)
}

/// Whether a fallible inline member has a raw-`Result` inline lowering
/// (`lower_inline_builtin_raw`) so an inline `TRAP` on it compiles and traps the
/// real runtime error. Two failure seams reach the capture point:
///
/// - the index/range members `collections::get`/`set`/`insert`/`removeAt`,
///   `strings::mid`, and `find` (`collections::find`/`strings::find`) raise
///   through the shared `emit_error_register_return` tail, whose
///   `raw_result_capture` branch redirects the domain error (plan-21-B);
/// - the callback loop members `forEach`/`transform`/`filter`/`reduce` route a
///   failing user callback through `emit_callback_failure_exit`, which frees each
///   member's loop-scoped intermediate before joining the capture (plan-26-B).
///
/// The infallible members are excluded here (they cannot fail, so there is
/// nothing to capture; `lower_inline_infallible_raw` wraps them always-`Ok`
/// instead). `target` is the canonical callee (`collections.get`,
/// `strings.mid`, ...).
pub(crate) fn inline_builtin_raw_supported(target: &str) -> bool {
    // The variable-shift `bits::` ops raise `ErrInvalidArgument` on an out-of-range
    // count through the shared `emit_error_register_return` tail, so their raw
    // lowering redirects that domain error to the inline-`TRAP` capture point.
    bits::is_bits_shift(target)
        || matches!(
            native_builtin_target(target),
            Some(
                "get" | "set"
                    | "insert"
                    | "removeAt"
                    | "find"
                    | "mid"
                    | "forEach"
                    | "transform"
                    | "filter"
                    | "reduce"
            )
        )
}

/// Whether an inline-lowered built-in callee can raise **no** user-trappable
/// domain error. Under an inline `TRAP` such a call is *allowed* but its handler
/// is dead code — the front-end warns `TYPE_INLINE_TRAP_DEAD_HANDLER` and codegen
/// wraps it always-`Ok` (`lower_inline_infallible_raw`, plan-26-A). The
/// fallibility census is grounded in each member's `lower_*` method: a member is
/// infallible here iff no success-relevant path emits a domain error
/// (`emit_index_out_of_range_return` / `emit_not_found_return` / range /
/// invalid-format). Allocation OOM does **not** count as trappable (umbrella Open
/// Decision), so growth-only mutators (`append`/`prepend`) are infallible.
///
/// Infallible: `len`, `toString`, `typeName`, every total `bits::*` op (all but
/// the variable shifts), and the pure-query / default-returning / OOM-only members
/// `contains`, `hasKey`, `keys`, `values`, `sum`, `getOr`, `append`, `prepend`,
/// `removeKey`, `replace`.
///
/// Fallible (NOT infallible — raw-supported, so an inline `TRAP` traps their real
/// error): the `bits::` variable shifts `sl`/`sr`/`sra` (out-of-range count
/// raises `ErrInvalidArgument`), the index members `get`/`set`/`insert`/`removeAt`,
/// `strings::mid`, `find` (negative start raises), and the callback members
/// `forEach`/`transform`/`filter`/`reduce` (a failing callback raises a real
/// error). `target` is the canonical callee (`collections.get`, `strings.mid`,
/// `bits.sl`) or a bare general-builtin name.
pub(crate) fn inline_builtin_is_infallible(target: &str) -> bool {
    // Every `bits::` op is total EXCEPT the variable shifts (`sl`/`sr`/`sra`),
    // which trap `ErrInvalidArgument` on an out-of-range count — those are
    // raw-supported (fallible) instead.
    if bits::is_bits_call(target) && !bits::is_bits_shift(target) {
        return true;
    }
    if matches!(target, "len" | "toString" | "typeName") {
        return true;
    }
    matches!(
        native_builtin_target(target),
        Some(
            "contains"
                | "hasKey"
                | "keys"
                | "values"
                | "sum"
                | "getOr"
                | "append"
                | "prepend"
                | "removeKey"
                | "replace"
        )
    )
}

pub(crate) fn call_return_type_name(name: &str) -> Option<&'static str> {
    general::call_return_type_name(name)
        .or_else(|| collections::call_return_type_name(name))
        .or_else(|| strings::call_return_type_name(name))
        .or_else(|| math::call_return_type_name(name))
        .or_else(|| bits::call_return_type_name(name))
        .or_else(|| crypto::call_return_type_name(name))
        .or_else(|| encoding::call_return_type_name(name))
        .or_else(|| fs::call_return_type_name(name))
        .or_else(|| io::call_return_type_name(name))
        .or_else(|| json::call_return_type_name(name))
        .or_else(|| csv::call_return_type_name(name))
        .or_else(|| regex::call_return_type_name(name))
        .or_else(|| datetime::call_return_type_name(name))
        .or_else(|| net::call_return_type_name(name))
        .or_else(|| os::call_return_type_name(name))
        .or_else(|| http::call_return_type_name(name))
        .or_else(|| term::call_return_type_name(name))
        .or_else(|| tls::call_return_type_name(name))
}

/// Whether parameter `index` of the built-in `callee` is a compiler-known
/// *non-escaping* callback position: the callee is
/// guaranteed to invoke the callback only synchronously during the call, never
/// to store, forward, return, or concurrently/cross-thread invoke it. A lambda
/// passed in such a position may capture an outer `MUT` binding as a temporary
/// call-bound borrow of that binding's slot (§11.2). The callback argument is
/// matched after normalization, so the index is the canonical parameter order.
///
/// `forEach`'s action (index 1) is the only such position today; `transform`,
/// `filter`, and `reduce` deliberately stay out (§9) — broadening is a separate
/// ergonomic decision, not a safety requirement.
pub(crate) fn is_nonescaping_callback_arg(callee: &str, index: usize) -> bool {
    matches!((callee, index), ("forEach", 1) | ("collections.forEach", 1))
}

pub(crate) fn is_builtin_call(name: &str) -> bool {
    collections::is_collections_call(name)
        || general::is_general_call(name)
        || strings::is_strings_call(name)
        || math::is_math_call(name)
        || bits::is_bits_call(name)
        || crypto::is_crypto_call(name)
        || encoding::is_encoding_call(name)
        || fs::is_fs_call(name)
        || io::is_io_call(name)
        || json::is_json_call(name)
        || csv::is_csv_call(name)
        || regex::is_regex_call(name)
        || datetime::is_datetime_call(name)
        || net::is_net_call(name)
        || os::is_os_call(name)
        || http::is_http_call(name)
        || term::is_term_call(name)
        || thread::is_thread_call(name)
        || tls::is_tls_call(name)
        || vector::is_vector_call(name)
        || call_return_type_name(name).is_some()
}

pub(crate) fn is_builtin_member(name: &str) -> bool {
    is_builtin_call(name) || is_package_constant(name)
}

/// A compile-time package constant that folds to a literal: `math::pi` and
/// friends (`Float`/`Fixed`) or an `errorCode::Err*` registry value (`Integer`).
/// These are keyed package-qualified (`"math.pi"`, `"errorCode.ErrNotFound"`).
pub(crate) fn is_package_constant(name: &str) -> bool {
    math::is_math_constant(name)
        || errorcode::is_errorcode_constant(name)
        || vector::is_vector_constant(name)
}

pub(crate) fn package_constant_type_name(name: &str) -> Option<&'static str> {
    math::constant_type_name(name)
        .or_else(|| errorcode::constant_type_name(name))
        .or_else(|| vector::constant_type_name(name))
}

pub(crate) fn package_constant_value(name: &str) -> Option<&'static str> {
    math::constant_value(name).or_else(|| errorcode::constant_value(name))
}

pub(crate) fn call_param_names(name: &str) -> Option<&'static [&'static [&'static str]]> {
    general::call_param_names(name)
        .or_else(|| collections::call_param_names(name))
        .or_else(|| strings::call_param_names(name))
        .or_else(|| math::call_param_names(name))
        .or_else(|| bits::call_param_names(name))
        .or_else(|| crypto::call_param_names(name))
        .or_else(|| encoding::call_param_names(name))
        .or_else(|| fs::call_param_names(name))
        .or_else(|| io::call_param_names(name))
        .or_else(|| json::call_param_names(name))
        .or_else(|| csv::call_param_names(name))
        .or_else(|| regex::call_param_names(name))
        .or_else(|| datetime::call_param_names(name))
        .or_else(|| net::call_param_names(name))
        .or_else(|| os::call_param_names(name))
        .or_else(|| http::call_param_names(name))
        .or_else(|| term::call_param_names(name))
        .or_else(|| tls::call_param_names(name))
        .or_else(|| thread::call_param_names(name))
        .or_else(|| vector::call_param_names(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_builtin_fallibility_census() {
        // Infallible-for-TRAP: raise no user-trappable domain error (plan-21-A).
        for c in [
            "len",
            "toString",
            "typeName",
            "bits.band",
            "bits.bor",
            "bits.rl64",
            "bits.clz",
            "bits.popCount",
            "collections.contains",
            "collections.hasKey",
            "collections.keys",
            "collections.values",
            "collections.sum",
            "collections.getOr",
            "collections.append",
            "collections.prepend",
            "collections.removeKey",
            "strings.replace",
        ] {
            assert!(inline_builtin_is_infallible(c), "expected infallible: {c}");
        }
        // Fallible inline members: a real domain error (index/range/not-found), an
        // out-of-range shift count, or a failing callback.
        for c in [
            "bits.sl",
            "bits.sr",
            "bits.sra",
            "collections.get",
            "collections.set",
            "collections.insert",
            "collections.removeAt",
            "collections.find",
            "strings.mid",
            "strings.find",
            "collections.forEach",
            "collections.transform",
            "collections.filter",
            "collections.reduce",
        ] {
            assert!(!inline_builtin_is_infallible(c), "expected fallible: {c}");
        }
        // Every inline member is classified one way or the other, and non-inline
        // callees (user functions) are not infallible built-ins.
        assert!(!inline_builtin_is_infallible("myFunc"));
        assert!(!inline_builtin_is_infallible("math.sqrt"));
    }

    #[test]
    fn inline_builtin_raw_supported_set() {
        // The fallible inline members with a raw-`Result` inline lowering
        // (plan-21-B): an inline TRAP on them compiles instead of being rejected.
        for c in [
            "collections.get",
            "collections.set",
            "collections.insert",
            "collections.removeAt",
            "collections.find",
            "strings.find",
            "strings.mid",
            "bits.sl",
            "bits.sr",
            "bits.sra",
        ] {
            assert!(
                inline_builtin_raw_supported(c),
                "expected raw-supported: {c}"
            );
            assert!(
                !inline_trap_unsupported(c),
                "raw-supported must not be unsupported: {c}"
            );
        }
        // The callback members are now raw-supported too (plan-26-B).
        for c in [
            "collections.forEach",
            "collections.transform",
            "collections.filter",
            "collections.reduce",
        ] {
            assert!(inline_builtin_raw_supported(c), "expected raw-supported: {c}");
            assert!(
                !inline_trap_unsupported(c),
                "raw-supported must not be unsupported: {c}"
            );
        }
        // The infallible members are NOT raw-supported (nothing to capture) but are
        // still trappable via the always-`Ok` path — so also not unsupported.
        for c in ["collections.contains", "len", "bits.band"] {
            assert!(
                !inline_builtin_raw_supported(c),
                "expected NOT raw-supported: {c}"
            );
            assert!(
                !inline_trap_unsupported(c),
                "infallible must not be unsupported: {c}"
            );
        }
    }

    #[test]
    fn is_builtin_import_cases() {
        for pkg in [
            "bits",
            "collections",
            "crypto",
            "csv",
            "datetime",
            "encoding",
            "errorCode",
            "fs",
            "http",
            "io",
            "json",
            "math",
            "net",
            "regex",
            "strings",
            "term",
            "thread",
            "tls",
            "vector",
        ] {
            assert!(is_builtin_import(pkg), "{pkg}");
        }
        assert!(!is_builtin_import("nope"));
        assert!(!is_builtin_import("resource"));
    }

    #[test]
    fn is_builtin_type_aggregates() {
        // A thread type routes through thread::is_builtin_type.
        assert!(is_builtin_type("Thread"));
        assert!(!is_builtin_type("Integer"));
        assert!(!is_builtin_type("List OF Integer"));
    }

    #[test]
    fn general_override_target_cases() {
        assert_eq!(
            general_override_target("toString", net::URL_TYPE),
            Some("__net_urlToString")
        );
        assert_eq!(general_override_target("toString", "Integer"), None);
        assert_eq!(general_override_target("len", net::URL_TYPE), None);
    }

    #[test]
    fn qualified_builtin_type_cases() {
        // net.Url -> bare Url type id.
        let url = qualified_builtin_type("net.Url");
        assert_eq!(url.as_deref(), Some(net::URL_TYPE));
        // Not a builtin package.
        assert_eq!(qualified_builtin_type("mymod.Thing"), None);
        // Builtin package, non-type member.
        assert_eq!(qualified_builtin_type("net.notAType"), None);
        // No dot at all.
        assert_eq!(qualified_builtin_type("Url"), None);
    }

    #[test]
    fn resource_helpers() {
        // File is a builtin resource type.
        assert!(is_resource_type("File"));
        assert!(!is_resource_type("Integer"));
        assert!(resource_close_function("File").is_some());
        assert!(resource_close_function("Integer").is_none());
        // is_thread_sendable_resource_type routes to resource module.
        let _ = is_thread_sendable_resource_type("File");
    }

    #[test]
    fn native_builtin_target_cases() {
        assert_eq!(native_builtin_target("strings.find"), Some("find"));
        assert_eq!(native_builtin_target("strings.mid"), Some("mid"));
        assert_eq!(native_builtin_target("strings.replace"), Some("replace"));
        assert_eq!(native_builtin_target("strings.other"), None);
        assert_eq!(native_builtin_target("collections.get"), Some("get"));
        assert_eq!(
            native_builtin_target("collections.transform"),
            Some("transform")
        );
        assert_eq!(native_builtin_target("collections.sum"), Some("sum"));
        assert_eq!(native_builtin_target("collections.sort"), None);
        assert_eq!(native_builtin_target("nope"), None);
    }

    #[test]
    fn inline_trap_unsupported_cases() {
        // Post plan-26 every inline builtin is trappable — infallible ones via the
        // always-`Ok` path, fallible ones via a raw capture — so `inline_trap_
        // unsupported` (the codegen backstop for a future un-lowered builtin) is
        // false for all of them.
        for target in [
            "bits.sl",            // raw-supported fallible bits shift
            "bits.band",          // infallible bits op
            "len",                // infallible general builtin
            "toString",           // infallible general builtin
            "typeName",           // infallible general builtin
            "collections.contains", // infallible collection query
            "collections.transform", // raw-supported callback member (plan-26-B)
            "collections.forEach",   // raw-supported callback member (plan-26-B)
            "collections.get",       // raw-supported index member (plan-21-B)
            "toInt",              // conversion builtin (own raw lowering)
            "nope",               // not a builtin at all
        ] {
            assert!(
                !inline_trap_unsupported(target),
                "expected trappable (not unsupported): {target}"
            );
        }
    }

    #[test]
    fn call_return_type_name_aggregates() {
        // general
        assert_eq!(call_return_type_name("toInt"), Some("Integer"));
        // strings::find contributes a return type through the aggregate.
        assert_eq!(call_return_type_name("strings.find"), Some("Integer"));
        assert_eq!(call_return_type_name("nope"), None);
    }

    #[test]
    fn is_nonescaping_callback_arg_cases() {
        assert!(is_nonescaping_callback_arg("forEach", 1));
        assert!(is_nonescaping_callback_arg("collections.forEach", 1));
        assert!(!is_nonescaping_callback_arg("forEach", 0));
        assert!(!is_nonescaping_callback_arg("transform", 1));
    }

    #[test]
    fn is_builtin_call_aggregates() {
        assert!(is_builtin_call("collections.get")); // collections
        assert!(is_builtin_call("len")); // general
        assert!(is_builtin_call("thread.start")); // thread
        assert!(is_builtin_call("toInt")); // via call_return_type_name
        assert!(!is_builtin_call("nope"));
    }

    #[test]
    fn is_builtin_member_and_package_constant() {
        assert!(is_package_constant("math.pi"));
        assert!(is_builtin_member("math.pi"));
        assert!(is_builtin_member("len"));
        assert!(!is_builtin_member("nope"));
        assert!(!is_package_constant("nope"));
    }

    #[test]
    fn package_constant_type_and_value() {
        assert!(package_constant_type_name("math.pi").is_some());
        assert!(package_constant_type_name("nope").is_none());
        assert!(package_constant_value("math.pi").is_some());
        assert!(package_constant_value("nope").is_none());
    }

    #[test]
    fn call_param_names_aggregates() {
        // general
        assert!(call_param_names("len").is_some());
        // collections
        assert!(call_param_names("collections.get").is_some());
        // thread
        assert!(call_param_names("thread.start").is_some());
        assert!(call_param_names("nope").is_none());
    }
}
