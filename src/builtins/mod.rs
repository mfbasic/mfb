pub(crate) mod audio;
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
pub(crate) mod money;
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
        "audio"
            | "bits"
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
            | "money"
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
    audio::is_builtin_type(name)
        || crypto::is_builtin_type(name)
        || datetime::is_builtin_type(name)
        || fs::is_builtin_type(name)
        || http::is_builtin_type(name)
        || io::is_builtin_type(name)
        || json::is_builtin_type(name)
        || money::is_builtin_type(name)
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
    // The member type must belong to the *named* package — an independent
    // `is_builtin_type(member)` check would accept any cross pairing (`io.Url`,
    // `csv.Thread`) because that predicate ORs every package together (bug-98).
    let belongs = match package {
        "audio" => audio::is_builtin_type(member),
        "crypto" => crypto::is_builtin_type(member),
        "datetime" => datetime::is_builtin_type(member),
        "fs" => fs::is_builtin_type(member),
        "http" => http::is_builtin_type(member),
        "json" => json::is_builtin_type(member),
        "money" => money::is_builtin_type(member),
        "net" => net::is_builtin_type(member),
        "term" => term::is_builtin_type(member),
        "thread" => thread::is_builtin_type(member),
        "tls" => tls::is_builtin_type(member),
        "vector" => vector::is_builtin_type(member),
        // io + the non-type packages expose no qualified value types.
        _ => false,
    };
    belongs.then(|| member.to_string())
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
                "get"
                    | "set"
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

/// Resolve a built-in call's return type from its package-qualified `callee`
/// name and argument types, dispatching through each package's `resolve_call` in
/// the same order the monomorphizer uses. Returns `None` for a non-built-in, an
/// unknown name, or an argument-type combination that matches no overload.
///
/// This is the single arg-typed return-type oracle shared by monomorph lowering
/// and `ir::verify` (which reconciles a decoded package's attacker-controlled
/// `Call` annotation against it — bug-162).
pub(crate) fn resolve_call_return_type(callee: &str, arg_types: &[String]) -> Option<String> {
    macro_rules! try_pkg {
        ($resolve:expr) => {
            if let Some(resolved) = $resolve {
                return Some(resolved.return_type.into_owned());
            }
        };
    }
    try_pkg!(general::resolve_call(callee, arg_types));
    try_pkg!(collections::resolve_call(callee, arg_types));
    try_pkg!(strings::resolve_call(callee, arg_types));
    try_pkg!(math::resolve_call(callee, arg_types));
    try_pkg!(bits::resolve_call(callee, arg_types));
    try_pkg!(crypto::resolve_call(callee, arg_types));
    try_pkg!(encoding::resolve_call(callee, arg_types));
    try_pkg!(fs::resolve_call(callee, arg_types));
    try_pkg!(io::resolve_call(callee, arg_types));
    try_pkg!(json::resolve_call(callee, arg_types));
    try_pkg!(csv::resolve_call(callee, arg_types));
    try_pkg!(regex::resolve_call(callee, arg_types));
    try_pkg!(datetime::resolve_call(callee, arg_types));
    try_pkg!(money::resolve_call(callee, arg_types));
    try_pkg!(net::resolve_call(callee, arg_types));
    try_pkg!(os::resolve_call(callee, arg_types));
    try_pkg!(http::resolve_call(callee, arg_types));
    try_pkg!(term::resolve_call(callee)); // no arg_types param
    try_pkg!(tls::resolve_call(callee, arg_types));
    try_pkg!(audio::resolve_call(callee, arg_types));
    try_pkg!(vector::resolve_call(callee, arg_types));
    try_pkg!(thread::resolve_call(callee, arg_types));
    None
}

pub(crate) fn call_return_type_name(name: &str) -> Option<&'static str> {
    audio::call_return_type_name(name)
        .or_else(|| general::call_return_type_name(name))
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
        .or_else(|| money::call_return_type_name(name))
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
/// call-bound reference to that binding's slot (§11.2). The callback argument is
/// matched after normalization, so the index is the canonical parameter order.
///
/// `forEach`'s action (index 1) is the only such position today; `transform`,
/// `filter`, and `reduce` deliberately stay out (§9) — broadening is a separate
/// ergonomic decision, not a safety requirement.
pub(crate) fn is_nonescaping_callback_arg(callee: &str, index: usize) -> bool {
    matches!((callee, index), ("forEach", 1) | ("collections.forEach", 1))
}

/// Built-in names that resolve, but only from toolchain-provided source.
///
/// These are the seam between a public built-in written in MFBASIC and the
/// native helper backing it: the injected `*_package.mfb` glue calls them, user
/// source must not. The resolver applies this only when the calling file is not
/// `AstFile::internal`, so the glue still resolves (bug-337-D9).
pub(crate) fn is_internal_only_call(name: &str) -> bool {
    crypto::is_crypto_internal_call(name)
}

pub(crate) fn is_builtin_call(name: &str) -> bool {
    // The `audio::` lowered-only internal names are not user-callable. They must be
    // excluded before the `call_return_type_name` fallback below, which knows their
    // types (IR lowering needs it for the rewritten target) and would otherwise
    // re-admit `audio::readTimeout()` as a builtin and silently miscompile it
    // (bug-213).
    if audio::is_audio_internal_call(name) {
        return false;
    }
    audio::is_audio_call(name)
        || collections::is_collections_call(name)
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
        || money::is_money_call(name)
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

/// Split a comma-separated type list on the commas at paren depth 0.
///
/// A type argument can itself be a comma-bearing type — `FUNC(Integer, String) AS
/// Boolean` is one argument, not two — so a flat `split(", ")` shreds it. Callers
/// parsing a type-argument list or a `FUNC` parameter list must use this.
pub(crate) fn split_top_level_commas(value: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    for (index, ch) in value.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                parts.push(value[start..index].trim());
                start = index + 1;
            }
            _ => {}
        }
    }
    parts.push(value[start..].trim());
    parts
}

/// Split the body of a `FUNC(<params>) AS <return>` type — everything after the
/// `FUNC(` prefix — into its parameter types and its return type.
///
/// The closing paren and the parameter separators are the ones at depth 0, so a
/// parameter that is itself a function type is kept whole. Returns `None` when the
/// parameter list has no top-level close paren or no `) AS ` follows it.
pub(crate) fn split_func_params_and_return(rest: &str) -> Option<(Vec<&str>, &str)> {
    let mut depth = 0usize;
    let mut close = None;
    for (index, ch) in rest.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' if depth == 0 => {
                close = Some(index);
                break;
            }
            ')' => depth -= 1,
            _ => {}
        }
    }
    let close = close?;
    let returns = rest.get(close..)?.strip_prefix(") AS ")?;
    let params_text = &rest[..close];
    let params = if params_text.trim().is_empty() {
        Vec::new()
    } else {
        split_top_level_commas(params_text)
    };
    Some((params, returns))
}

/// Parameter names for a builtin whose overloads disagree on where a given name
/// sits, listed one overload at a time. A builtin with such a table is normalized
/// by selecting the overload first, then binding names within it; every other
/// builtin uses the merged per-position table of [`call_param_names`].
pub(crate) fn call_param_name_overloads(name: &str) -> Option<&'static [&'static [&'static str]]> {
    audio::call_param_name_overloads(name)
        .or_else(|| net::call_param_name_overloads(name))
        .or_else(|| datetime::call_param_name_overloads(name))
}

/// Pick the overload a call selects, given how many arguments were passed
/// positionally and the names of the rest.
///
/// The chosen overload takes exactly this many arguments, names every supplied
/// name, and places none of those names in a slot a positional argument already
/// filled. Both the type checker and IR lowering resolve named arguments through
/// this, so they cannot disagree about which parameter a name binds to.
pub(crate) fn select_param_name_overload<'a>(
    overloads: &'a [&'a [&'a str]],
    positional_count: usize,
    names: &[&str],
) -> Option<&'a [&'a str]> {
    overloads.iter().copied().find(|params| {
        params.len() == positional_count + names.len()
            && names.iter().all(|name| {
                params
                    .iter()
                    .position(|param| param == name)
                    .is_some_and(|index| index >= positional_count)
            })
    })
}

pub(crate) fn call_param_names(name: &str) -> Option<&'static [&'static [&'static str]]> {
    audio::call_param_names(name)
        .or_else(|| general::call_param_names(name))
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
        .or_else(|| money::call_param_names(name))
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

    /// Every documented builtin, as `package.function`, read from the man pages
    /// (`src/docs/man/builtins/<package>/<function>.{md,txt}`).
    ///
    /// Both extensions, deliberately. This used to filter on `txt` alone, so the
    /// migrated Markdown pages — by then most of the corpus — were checked by
    /// nothing, and the `> 100` floor hid it: the metric was **inverted**, since
    /// every page migrated shrank the guarded set while the assertion kept
    /// passing (bug-336-S3). The floor is now an exact-ish lower bound on the
    /// whole corpus rather than a number the legacy half alone could satisfy.
    fn documented_builtins() -> Vec<String> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/docs/man/builtins");
        let mut names = Vec::new();
        for package in std::fs::read_dir(&root).expect("man builtins dir") {
            let package = package.expect("package dir").path();
            let Some(package_name) = package.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if !package.is_dir() {
                continue;
            }
            for page in std::fs::read_dir(&package).expect("package dir") {
                let page = page.expect("man page").path();
                if !matches!(
                    page.extension().and_then(|ext| ext.to_str()),
                    Some("txt") | Some("md")
                ) {
                    continue;
                }
                let Some(function) = page.file_stem().and_then(|name| name.to_str()) else {
                    continue;
                };
                // `package.md` is the package overview, not a function page, and
                // `types.md` is a package's consolidated type page.
                if matches!(function, "package" | "types") {
                    continue;
                }
                names.push(format!("{package_name}.{function}"));
            }
        }
        assert!(
            names.len() > 400,
            "expected the full builtin man corpus, got {} pages",
            names.len()
        );
        names
    }

    #[test]
    fn qualified_builtin_type_requires_matching_package() {
        // bug-98: the member type must belong to the named package. A valid
        // pairing resolves to the bare type; a cross pairing (right type, wrong
        // package) must not.
        assert_eq!(
            qualified_builtin_type("net.Url"),
            Some(net::URL_TYPE.to_string())
        );
        // `Url` is a net type, not an io/csv type — these must be rejected.
        assert_eq!(qualified_builtin_type("io.Url"), None);
        assert_eq!(qualified_builtin_type("crypto.Url"), None);
        // A non-builtin package is rejected outright.
        assert_eq!(qualified_builtin_type("csv.Thread"), None);
        // A bare (unqualified) name is not a qualified type.
        assert_eq!(qualified_builtin_type("Url"), None);
    }

    #[test]
    fn no_named_argument_alias_repeats_across_positions() {
        // `call_param_names` resolves a name to the *first* position group that
        // lists it, with no backtracking. An alias appearing in two groups is
        // therefore unresolvable: it pins to the earlier position and collides
        // with that parameter (bug-28, `net.connectTcp`'s `timeoutMs`). A builtin
        // whose overloads genuinely disagree on a name's position must declare a
        // per-overload table instead.
        for name in documented_builtins() {
            let Some(groups) = call_param_names(&name) else {
                continue;
            };
            for (index, aliases) in groups.iter().enumerate() {
                for alias in *aliases {
                    let earlier = groups[..index].iter().any(|group| group.contains(alias));
                    assert!(
                        !earlier,
                        "`{name}` lists the argument name `{alias}` at two positions; \
                         a named `{alias}` can never bind to position {index}"
                    );
                }
            }
        }
    }

    #[test]
    fn overloaded_param_name_tables_are_well_formed() {
        for name in documented_builtins() {
            let Some(overloads) = call_param_name_overloads(&name) else {
                continue;
            };
            // A per-overload table replaces the merged one; carrying both would
            // leave the merged table silently unused.
            assert!(
                call_param_names(&name).is_none(),
                "`{name}` declares both a merged and a per-overload param table"
            );
            for params in overloads {
                for (index, param) in params.iter().enumerate() {
                    assert!(
                        !params[..index].contains(param),
                        "`{name}` repeats the parameter `{param}` in one overload"
                    );
                }
            }
            // Two overloads of the same arity must differ by name, or selection
            // between them would be arbitrary.
            for (index, params) in overloads.iter().enumerate() {
                for other in &overloads[..index] {
                    assert!(
                        params.len() != other.len() || params != other,
                        "`{name}` declares the same overload twice"
                    );
                }
            }
        }
    }

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
            assert!(
                inline_builtin_raw_supported(c),
                "expected raw-supported: {c}"
            );
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

    /// The full import-gated package set. Kept in one place so the `is_builtin_import`
    /// predicate and the `mfb spec language builtin-functions` §18 list cannot drift
    /// apart (plan-33-D Phase 2 — the earlier `money` omission recurred because no
    /// such test existed).
    const ALL_BUILTIN_PACKAGES: &[&str] = &[
        "audio",
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
        "money",
        "net",
        "os",
        "regex",
        "strings",
        "term",
        "thread",
        "tls",
        "vector",
    ];

    #[test]
    fn every_package_is_a_builtin_import() {
        for pkg in ALL_BUILTIN_PACKAGES {
            assert!(is_builtin_import(pkg), "is_builtin_import missing {pkg}");
        }
        assert!(!is_builtin_import("audioo"));
        assert!(!is_builtin_import("resource"));
    }

    #[test]
    fn spec_section_18_package_list_matches_is_builtin_import() {
        // Extract the backtick-quoted package names from §18's "package set the
        // resolver recognizes is fixed:" sentence and assert it equals the
        // canonical set exactly (no missing, no extra).
        let doc = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("src/docs/spec/language/18_builtin-functions.md"),
        )
        .expect("read §18 spec");
        let anchor = "The package set the resolver recognizes is fixed:";
        let start = doc.find(anchor).expect("§18 package-set sentence");
        // The sentence runs from the anchor to the citation marker that follows it.
        let rest = &doc[start..];
        let end = rest
            .find("[[src/builtins/mod.rs:is_builtin_import]]")
            .expect("§18 citation");
        let sentence = &rest[..end];
        let mut listed: Vec<String> = Vec::new();
        let mut chars = sentence.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '`' {
                let mut name = String::new();
                for c in chars.by_ref() {
                    if c == '`' {
                        break;
                    }
                    name.push(c);
                }
                listed.push(name);
            }
        }
        let mut expected: Vec<String> =
            ALL_BUILTIN_PACKAGES.iter().map(|s| s.to_string()).collect();
        listed.sort();
        expected.sort();
        assert_eq!(
            listed, expected,
            "§18 package list drifted from is_builtin_import; \
             update src/docs/spec/language/18_builtin-functions.md"
        );
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
            "money",
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
            "bits.sl",               // raw-supported fallible bits shift
            "bits.band",             // infallible bits op
            "len",                   // infallible general builtin
            "toString",              // infallible general builtin
            "typeName",              // infallible general builtin
            "collections.contains",  // infallible collection query
            "collections.transform", // raw-supported callback member (plan-26-B)
            "collections.forEach",   // raw-supported callback member (plan-26-B)
            "collections.get",       // raw-supported index member (plan-21-B)
            "toInt",                 // conversion builtin (own raw lowering)
            "nope",                  // not a builtin at all
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
