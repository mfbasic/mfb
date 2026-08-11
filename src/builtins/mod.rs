pub(crate) mod app;
pub(crate) mod astrings;
pub(crate) mod audio;
pub(crate) mod bits;
pub(crate) mod crypto;
pub(crate) mod errorcode;
pub(crate) mod fs;
pub(crate) mod general;
pub(crate) mod http;
pub(crate) mod io;
pub(crate) mod math;
pub(crate) mod money;
pub(crate) mod net;
pub(crate) mod os;
pub(crate) mod process;
pub(crate) mod resource;
pub(crate) mod strings;
pub(crate) mod term;
pub(crate) mod testing;
pub(crate) mod thread;
pub(crate) mod tls;
pub(crate) mod vector;

pub(crate) use resource::{ResourceInfo, ResourceKind, ResourceRegistry};

/// bug-340 A3: exact argument-type match, `arg_types == expected` element-wise.
/// The single home for what were fifteen byte-identical `fn exact` copies, one
/// per builtin-package module.
pub(super) fn exact(arg_types: &[String], expected: &[&str]) -> bool {
    arg_types.len() == expected.len()
        && arg_types
            .iter()
            .zip(expected.iter())
            .all(|(actual, expected)| actual == expected)
}

/// bug-340 A2: generate the `source_file` / `uses_package` / `augmented_project`
/// trio that was copied byte-for-byte into every uniform builtin-package module.
/// The four per-module literals are the package name, the synthetic path label,
/// the doc path, and the package source text. `$src` is an *expression* (not a
/// path), so the `include_str!` is written at the invocation site and resolves
/// relative to the invoking module's file; a module may also pass a `concat!` of
/// several `include_str!`s (crypto's five-file companion).
///
/// `regex`, `strings`, and `collections` opt out: `regex::source_file` joins two
/// sources via `format!`; `strings::uses_package` gates on scalar-seam member
/// usage; `collections::augmented_project` takes `AstProject` by value.
macro_rules! package_source_glue {
    ($pkg:literal, $label:literal, $doc:literal, $src:expr $(,)?) => {
        pub(crate) fn source_file() -> Result<crate::ast::AstFile, ()> {
            crate::ast::parse_source_internal(std::path::Path::new($label), $doc, $src)
        }

        pub(crate) fn uses_package(ast: &crate::ast::AstProject) -> bool {
            ast.files.iter().any(|file| {
                file.imports
                    .iter()
                    .any(|import| import.package_name() == $pkg)
            })
        }

        pub(crate) fn augmented_project(
            ast: &crate::ast::AstProject,
        ) -> Result<crate::ast::AstProject, ()> {
            if !uses_package(ast) {
                return Ok(ast.clone());
            }
            let mut augmented = ast.clone();
            augmented.files.push(source_file()?);
            Ok(augmented)
        }
    };
}
pub(crate) use package_source_glue;

pub(crate) fn is_builtin_import(name: &str) -> bool {
    matches!(
        name,
        "app"
            | "astrings"
            | "audio"
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
            | "process"
            | "regex"
            | "strings"
            | "term"
            | "thread"
            | "tls"
            | "vector"
    )
}

/// Whether `name` is a builtin value/opaque type contributed by any package
/// (plan-72-BB: iterated over the descriptor registry's `types`). Every package's
/// base type names live in its descriptor; `thread`'s parametric
/// `Thread OF ...` / `ThreadWorker OF ...` forms are the one shape a static type
/// list cannot enumerate, so they stay a structural prefix check.
pub(crate) fn is_builtin_type(name: &str) -> bool {
    crate::codegen::registry::REGISTRY
        .modules()
        .iter()
        .any(|module| module.types.iter().any(|ty| ty.name == name))
        || name.starts_with("Thread OF ")
        || name.starts_with("ThreadWorker OF ")
}

/// The record `(field, type)` list of a builtin type, or `None` when the type is
/// opaque/enum or unknown (plan-72-BB: the owning module's descriptor `types`
/// entry — only `io`, `net`, `term`, and `audio` contribute record types today).
/// A type with no fields (opaque handle / source-companion record) reports `None`,
/// matching the legacy per-package `builtin_type_fields`.
pub(crate) fn builtin_type_fields(name: &str) -> Option<&'static [(&'static str, &'static str)]> {
    crate::codegen::registry::REGISTRY
        .modules()
        .iter()
        .find_map(|module| {
            module
                .types
                .iter()
                .find(|ty| ty.name == name)
                .and_then(|ty| (!ty.fields.is_empty()).then_some(ty.fields))
        })
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
        "app" => app::is_builtin_type(member),
        "audio" => audio::is_builtin_type(member),
        "crypto" => crypto::is_builtin_type(member),
        "csv" => crate::codegen::builtins::csv::is_builtin_type(member),
        "datetime" => crate::codegen::builtins::datetime::is_builtin_type(member),
        "fs" => fs::is_builtin_type(member),
        "http" => http::is_builtin_type(member),
        "json" => crate::codegen::builtins::json::is_builtin_type(member),
        "money" => money::is_builtin_type(member),
        "net" => net::is_builtin_type(member),
        "process" => process::is_builtin_type(member),
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
    match crate::codegen::builtins::collections::native_member_bare(name)? {
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
        "reduceRight" => Some("reduceRight"),
        "sum" => Some("sum"),
        "find" => Some("find"),
        "mid" => Some("mid"),
        "replace" => Some("replace"),
        // Set members (plan-63-B).
        "add" => Some("add"),
        "remove" => Some("remove"),
        "toList" => Some("toList"),
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
/// - the callback loop members `forEach`/`transform`/`filter`/`reduce`/
///   `reduceRight` route a
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
                    | "reduceRight"
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
/// `forEach`/`transform`/`filter`/`reduce`/`reduceRight` (a failing callback
/// raises a real error). `target` is the canonical callee (`collections.get`, `strings.mid`,
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
                // Set members (plan-63-B): pure, total — `add`/`remove` return a
                // new set, `toList` a list; none can fail.
                | "add"
                | "remove"
                | "toList"
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
/// The argument-validated return type of a builtin call, resolved through the
/// descriptor registry (plan-72-BB). The module owning `callee` resolves it via
/// its `BuiltinResolver` (every computed-return / argument-union package overrides
/// `resolve_return_type`) or, for a fully data-only package, via
/// `DefaultResolver::resolve_call`'s exact per-position match. The registry
/// guarantees each qualified name is owned by exactly one module
/// (`duplicate_function_name` is `None`), so this is order-independent — replacing
/// the hand-ordered per-package `resolve_call` chain it grew from.
pub(crate) fn resolve_call_return_type(callee: &str, arg_types: &[String]) -> Option<String> {
    let (module, function) = crate::codegen::registry::REGISTRY.function(callee)?;
    match module.resolver {
        Some(resolver) => resolver.resolve_return_type(module, function.name, arg_types),
        None => crate::codegen::registry::DefaultResolver::resolve_call(
            module,
            function.name,
            arg_types,
        )
        .map(str::to_string),
    }
}

/// The static (argument-independent) nominal return type of a builtin call —
/// plan-72-BB: the owning module's `DefaultResolver::return_type_name` (a
/// `Custom`-return call has no static nominal and yields `None`; the arg-validated
/// return lives in [`resolve_call_return_type`]). The lowered-only internal names
/// (`audio` device opens / timed I/O, `tls.closeListener`) are not descriptor
/// functions, so IR lowering's queries for their rewritten targets fall back to
/// those two packages' explicit internal-name maps.
pub(crate) fn call_return_type_name(name: &str) -> Option<&'static str> {
    if let Some((module, function)) = crate::codegen::registry::REGISTRY.function(name) {
        return crate::codegen::registry::DefaultResolver::return_type_name(module, function.name);
    }
    audio::call_return_type_name(name).or_else(|| tls::call_return_type_name(name))
}

/// The name of the builtin package that owns a fully qualified call, or `None`
/// (plan-72-BB: the registry's single owner). Used by the syntaxcheck dispatcher
/// to select a table package's argument-inference mode without a per-package
/// `is_<pkg>_call` chain.
pub(crate) fn builtin_package_name(callee: &str) -> Option<&'static str> {
    crate::codegen::registry::REGISTRY
        .function(callee)
        .map(|(module, _)| module.name)
}

/// The arity range `(min, max)` of a builtin call — plan-72-BB: the owning
/// module's `DefaultResolver::arity`. `None` for a call no package owns.
pub(crate) fn arity(name: &str) -> Option<(usize, usize)> {
    let (module, function) = crate::codegen::registry::REGISTRY.function(name)?;
    crate::codegen::registry::DefaultResolver::arity(module, function.name)
}

/// The human-readable expected-argument rendering for a builtin call's
/// argument-mismatch diagnostic — plan-72-BB. Most packages render per-position
/// from the descriptor (`DefaultResolver::expected_arguments`); the packages whose
/// phrasing is an argument *union* (`"Socket or Listener or UdpSocket"`) or prose
/// (`vector`'s `"two vectors of the same type"`) keep their hand-authored string,
/// which the descriptor's per-position join cannot reproduce (a genuine
/// non-descriptor behavior, per BB's non-goals).
pub(crate) fn expected_arguments(name: &str) -> Option<String> {
    // `term` alone returns an owned `String` (its `"no arguments"` zero-arg form).
    if let Some(text) = term::expected_arguments(name) {
        return Some(text);
    }
    // Every package that still owns an `expected_arguments` free function keeps its
    // hand-authored phrasing — the `[optional]` bracket (`strings.find`'s
    // `"String, String[, Integer]"`), the `"or"`-union, or prose — that the
    // descriptor's per-position join cannot reproduce. Each returns `Some` only for
    // its own calls, so the chain yields the owner's string; a package whose
    // `expected_arguments` was deletable (renderable == `DefaultResolver`, i.e.
    // `app`/`datetime`/`money`) falls through to the descriptor rendering below.
    if let Some(text) = crate::codegen::builtins::encoding::expected_arguments(name)
        .or_else(|| crypto::expected_arguments(name))
        .or_else(|| math::expected_arguments(name))
        .or_else(|| net::expected_arguments(name))
        .or_else(|| tls::expected_arguments(name))
        .or_else(|| audio::expected_arguments(name))
        .or_else(|| process::expected_arguments(name))
        .or_else(|| http::expected_arguments(name))
        .or_else(|| vector::expected_arguments(name))
        .or_else(|| crate::codegen::builtins::collections::expected_arguments(name))
        .or_else(|| general::expected_arguments(name))
        .or_else(|| thread::expected_arguments(name))
        .or_else(|| strings::expected_arguments(name))
        .or_else(|| crate::codegen::builtins::regex::expected_arguments(name))
        .or_else(|| fs::expected_arguments(name))
        .or_else(|| os::expected_arguments(name))
        .or_else(|| io::expected_arguments(name))
        .or_else(|| crate::codegen::builtins::json::expected_arguments(name))
        .or_else(|| crate::codegen::builtins::csv::expected_arguments(name))
        .or_else(|| bits::expected_arguments(name))
        .or_else(|| crate::codegen::builtins::datetime::expected_arguments(name))
    {
        return Some(text.to_string());
    }
    let (module, function) = crate::codegen::registry::REGISTRY.function(name)?;
    crate::codegen::registry::DefaultResolver::expected_arguments(module, function.name)
}

/// The concrete per-position argument-type signature IR lowering uses for literal
/// coercion (bug-340 A1), or `None` when the call has no single positional
/// signature (generic/overloaded members, or a bracketed/`"or"`-phrased
/// description). plan-72-BB: this is the exact heuristic ir/lower previously
/// inlined, relocated here so the per-package reads live behind one aggregate.
/// Packages carrying a machine-readable positional table are read directly;
/// `collections`/`vector` are absent on purpose (every member is generic or
/// overloaded, so the monomorphizer types them).
pub(crate) fn argument_types(callee: &str) -> Option<Vec<String>> {
    let machine_table = term::param_types(callee)
        .or_else(|| crate::codegen::builtins::datetime::argument_types(callee))
        .or_else(|| crate::codegen::builtins::encoding::argument_types(callee))
        .or_else(|| money::argument_types(callee))
        .or_else(|| app::argument_types(callee));
    if let Some(types) = machine_table {
        return Some(types.iter().map(|type_| (*type_).to_string()).collect());
    }

    let expected = general::expected_arguments(callee)
        .or_else(|| strings::expected_arguments(callee))
        .or_else(|| math::expected_arguments(callee))
        .or_else(|| bits::expected_arguments(callee))
        .or_else(|| fs::expected_arguments(callee))
        .or_else(|| os::expected_arguments(callee))
        .or_else(|| io::expected_arguments(callee))
        .or_else(|| crate::codegen::builtins::json::expected_arguments(callee))
        .or_else(|| crate::codegen::builtins::csv::expected_arguments(callee))
        .or_else(|| crate::codegen::builtins::regex::expected_arguments(callee))
        .or_else(|| net::argument_types(callee))
        .or_else(|| tls::argument_types(callee))
        .or_else(|| audio::argument_types(callee))
        .or_else(|| crypto::argument_types(callee))
        .or_else(|| http::expected_arguments(callee))
        .or_else(|| thread::expected_arguments(callee))?;
    // Overloaded/optional-argument descriptions (e.g. `strings.find`'s
    // `"String, String[, Integer]"`) are not a concrete positional signature; skip
    // them so we don't hand the lowerer a bracket-mangled expected type.
    if expected.contains('[') || expected.contains(" or ") {
        return None;
    }
    let params = expected.split(", ").map(str::to_string).collect::<Vec<_>>();
    if params.iter().any(|param| uses_generic_placeholder(param)) {
        return None;
    }
    Some(params)
}

/// The `(type, value)` constants to append after the `provided` real arguments so
/// a fixed-ABI runtime helper always receives every parameter — plan-72-BB: the
/// owning package's `default_argument_padding` (only `tls`/`regex`/`datetime`/
/// `crypto`/`http` default-pad; each owns its callee uniquely, so the first
/// non-empty result is the owner's).
pub(crate) fn default_argument_padding(
    callee: &str,
    provided: usize,
) -> &'static [(&'static str, &'static str)] {
    for pad in [
        tls::default_argument_padding(callee, provided),
        crate::codegen::builtins::regex::default_argument_padding(callee, provided),
        crate::codegen::builtins::datetime::default_argument_padding(callee, provided),
        crypto::default_argument_padding(callee, provided),
        http::default_argument_padding(callee, provided),
        crate::codegen::builtins::csv::default_argument_padding(callee, provided),
    ] {
        if !pad.is_empty() {
            return pad;
        }
    }
    &[]
}

/// Whether a type name is a generic placeholder (`T`/`K`/`V` bare or inside a
/// container), used by [`argument_types`] to skip generic member signatures.
fn uses_generic_placeholder(type_: &str) -> bool {
    matches!(type_, "T" | "K" | "V")
        || type_.contains(" OF T")
        || type_.contains(" OF K")
        || type_.contains(" OF V")
        || type_.contains(" TO T")
        || type_.contains(" TO K")
        || type_.contains(" TO V")
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
    crypto::is_crypto_internal_call(name) || astrings::is_astrings_internal_call(name)
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
    // plan-72-BB: descriptor membership is every package's `is_<pkg>_call`
    // (`DefaultResolver::contains`). The two non-descriptor member surfaces stay
    // explicit: `collections`' source-generic functions and `vector`'s
    // dynamically-parsed constants. The `call_return_type_name` tail preserves the
    // pre-existing admission of lowered-only names whose return type is known
    // (e.g. `tls.closeListener`).
    crate::codegen::registry::REGISTRY.function(name).is_some()
        || crate::codegen::builtins::collections::is_collections_call(name)
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

/// Owned, empty-aware variant of [`split_top_level_commas`]: an empty (or
/// all-whitespace) list is zero types rather than one empty string, and each part
/// is returned owned. The single home for what were three byte-identical
/// depth-tracked splitters — in `thread`, `binary_repr::writer`, and the native
/// value-semantics builder (bug-340 A5).
pub(crate) fn split_top_level_types(params: &str) -> Vec<String> {
    if params.trim().is_empty() {
        return Vec::new();
    }
    split_top_level_commas(params)
        .into_iter()
        .map(str::to_string)
        .collect()
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
        .or_else(|| crate::codegen::builtins::datetime::call_param_name_overloads(name))
        .or_else(|| tls::call_param_name_overloads(name))
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
    app::call_param_names(name)
        .or_else(|| astrings::call_param_names(name))
        .or_else(|| audio::call_param_names(name))
        .or_else(|| general::call_param_names(name))
        .or_else(|| crate::codegen::builtins::collections::call_param_names(name))
        .or_else(|| strings::call_param_names(name))
        .or_else(|| math::call_param_names(name))
        .or_else(|| bits::call_param_names(name))
        .or_else(|| crypto::call_param_names(name))
        .or_else(|| crate::codegen::builtins::encoding::call_param_names(name))
        .or_else(|| fs::call_param_names(name))
        .or_else(|| io::call_param_names(name))
        .or_else(|| crate::codegen::builtins::json::call_param_names(name))
        .or_else(|| crate::codegen::builtins::csv::call_param_names(name))
        .or_else(|| crate::codegen::builtins::regex::call_param_names(name))
        .or_else(|| crate::codegen::builtins::datetime::call_param_names(name))
        .or_else(|| money::call_param_names(name))
        .or_else(|| net::call_param_names(name))
        .or_else(|| os::call_param_names(name))
        .or_else(|| http::call_param_names(name))
        .or_else(|| term::call_param_names(name))
        .or_else(|| tls::call_param_names(name))
        .or_else(|| thread::call_param_names(name))
        .or_else(|| vector::call_param_names(name))
}

// plan-72 registry adapters. Each queries a descriptor `registry` for a call's
// metadata and falls back to the legacy per-package helper when the call's
// package has not migrated yet. In letter A the production
// `crate::codegen::registry::REGISTRY` is empty, so these always take the fallback path and
// production behavior is byte-identical to calling the legacy helper directly;
// letters B..AA populate the registry (moving each package onto the descriptor
// branch) and BB removes the `legacy` fallbacks once every package has migrated.
// They are unused in production until the first package migrates, hence the
// `not(test)` dead-code allow; the tests below exercise both branches.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn registry_is_call(
    registry: &crate::codegen::registry::BuiltinRegistry,
    callee: &str,
    legacy: impl Fn(&str) -> bool,
) -> bool {
    registry.function(callee).is_some() || legacy(callee)
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn registry_arity(
    registry: &crate::codegen::registry::BuiltinRegistry,
    callee: &str,
    legacy: impl Fn(&str) -> Option<(usize, usize)>,
) -> Option<(usize, usize)> {
    if let Some((module, function)) = registry.function(callee) {
        return crate::codegen::registry::DefaultResolver::arity(module, function.name);
    }
    legacy(callee)
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn registry_return_type_name(
    registry: &crate::codegen::registry::BuiltinRegistry,
    callee: &str,
    legacy: impl Fn(&str) -> Option<&'static str>,
) -> Option<&'static str> {
    if let Some((module, function)) = registry.function(callee) {
        return crate::codegen::registry::DefaultResolver::return_type_name(module, function.name);
    }
    legacy(callee)
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn registry_expected_arguments(
    registry: &crate::codegen::registry::BuiltinRegistry,
    callee: &str,
    legacy: impl Fn(&str) -> Option<String>,
) -> Option<String> {
    if let Some((module, function)) = registry.function(callee) {
        return crate::codegen::registry::DefaultResolver::expected_arguments(
            module,
            function.name,
        );
    }
    legacy(callee)
}

/// plan-72-I: resolve an overloaded builtin call to its concrete monomorph target
/// via the descriptor registry, delegating to the owning package's resolver. This
/// is the descriptor-API entry point the monomorphizer uses in place of the
/// `encoding`-specific free function. `Ok(None)` when the callee is not a
/// registered overloaded builtin; `Err(())` when a return-type overload needs an
/// expected type that is absent (`utf8Encode` with no `List OF Byte`/`List OF
/// Integer` context).
pub(crate) fn resolve_overload_target(
    callee: &str,
    arg_types: &[String],
    expected_type: Option<&str>,
) -> Result<Option<String>, ()> {
    let Some((module, _function)) = crate::codegen::registry::REGISTRY.function(callee) else {
        return Ok(None);
    };
    match module.resolver {
        Some(resolver) => {
            resolver.resolve_overload_target(module, callee, arg_types, expected_type)
        }
        None => Ok(None),
    }
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
        "app",
        "astrings",
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
        "process",
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
            "app",
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

    // ---- plan-72-A2: registry adapters -------------------------------------

    #[test]
    fn adapters_fall_back_on_registry_miss() {
        // Every real builtin package is now migrated (plan-72-A..AA), so the
        // production registry owns every real call name and no real package
        // exercises the legacy fallback anymore. The mod.rs adapters still fall
        // back to their legacy closure on a registry MISS — the mechanism BB will
        // delete once aggregate dispatch is registry-only. Prove that mechanism
        // with a synthetic name no module owns: the registry misses and the
        // adapter returns exactly the closure's answer. (This test tracked a
        // still-unmigrated real example — `math` until plan-72-P, `regex` until -T,
        // `tls` until -Z — but none remains.)
        assert!(registry_is_call(
            &crate::codegen::registry::REGISTRY,
            "nonesuch.thing",
            |name| { name == "nonesuch.thing" }
        ));
        assert!(!registry_is_call(
            &crate::codegen::registry::REGISTRY,
            "nonesuch.other",
            |_| false
        ));
        assert_eq!(
            registry_arity(
                &crate::codegen::registry::REGISTRY,
                "nonesuch.thing",
                |_| Some((1, 2))
            ),
            Some((1, 2))
        );
        assert_eq!(
            registry_return_type_name(
                &crate::codegen::registry::REGISTRY,
                "nonesuch.thing",
                |_| Some("Nothing")
            ),
            Some("Nothing")
        );
        assert_eq!(
            registry_expected_arguments(
                &crate::codegen::registry::REGISTRY,
                "nonesuch.thing",
                |_| Some("X".to_string())
            ),
            Some("X".to_string())
        );
    }
}
