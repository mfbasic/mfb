//! Builtin packages whose lowering has migrated into the codegen layer
//! (plan-95). Each package owns its `BuiltinFunction` descriptors and, per
//! migrated function, the target-generic lowering carried by `Implementation`.

pub(crate) mod app;
pub(crate) mod astrings;
pub(crate) mod audio;
pub(crate) mod bits;
pub(crate) mod canvas;
pub(crate) mod collections;
pub(crate) mod color;
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
pub(crate) mod perf;
pub(crate) mod process;
pub(crate) mod regex;
pub(crate) mod strings;
pub(crate) mod tcp;
pub(crate) mod term;
pub(crate) mod testing;
pub(crate) mod thread;
pub(crate) mod tls;
pub(crate) mod udp;
pub(crate) mod vector;

// ---------------------------------------------------------------------------
// Builtin dispatch facade (relocated from `src/builtins/mod.rs`, plan-103).
//
// The aggregate builtin-dispatch surface fronting `codegen::registry` and the
// per-package lowering modules above. Most functions delegate; a few carry
// genuine dispatch logic (`resolve_call_return_type`'s general/vector/strings
// special-cases, the inline-TRAP census trio) and the pure type-list utilities
// have no other home. `general` is the child module declared above; `resource`
// is the relocated resource registry (`codegen::resource`).
// ---------------------------------------------------------------------------
use crate::codegen::resource;
use crate::types::ParameterType;

/// Every package name `IMPORT` accepts, sorted.
///
/// This is the **single** source of truth for the import-gated package set. It was
/// previously a `matches!` arm with a hand-maintained mirror list in this module's
/// test section, and the two drifted: the mirror (and therefore §18 of the spec,
/// which is pinned against it) omitted `tcp` and `udp` for the whole of plan-110's
/// lifetime, while §18's own transport paragraph documented `IMPORT tcp` and
/// `IMPORT udp` two dozen lines further down. A `matches!` cannot be enumerated, so
/// no test could see the omission from this side. As a slice it can, and
/// `spec_section_18_package_list_matches_is_builtin_import` now compares §18
/// against this list itself rather than against a copy of it.
pub(crate) const BUILTIN_IMPORTS: &[&str] = &[
    "app",
    "astrings",
    "audio",
    "bits",
    "canvas",
    "collections",
    "color",
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
    "tcp",
    "term",
    "thread",
    "tls",
    "udp",
    "vector",
];

pub(crate) fn is_builtin_import(name: &str) -> bool {
    BUILTIN_IMPORTS.contains(&name)
}

/// The internal helper a built-in package provides as an **override** of an
/// overridable general built-in (`toString`, `len`, …) over one of its value
/// types (plan-01-overload.md §B.2). A general call `f(x)` whose sole argument
/// has such a type routes to this `__pkg_name` helper instead of the scalar
/// builtin; the name is internalized at lowering so it never collides with the
/// builtin dispatch symbol. Keyed by `(builtin, arg_type)`. The `toString(net::Url)`
/// renderer now rides on the migrated `net` package's `add_override`
/// (`registry::general_override_target`); the remaining hand row is `vector`'s.
pub(crate) fn general_override_target(
    builtin: &str,
    arg_type: &crate::types::ParameterType,
) -> Option<&'static str> {
    // Every override — `toString(net::Url)` and the nine `toString(VecN)` renderers — is
    // now registered on the clean-room registry via `add_override`; no hand rows remain.
    crate::codegen::registry::general_override_target(builtin, arg_type)
}

/// Whether `qualified` (dot form, `process.Process`, `fs.File`) names a built-in
/// **resource** type. Resources keep their package-qualified identity through the
/// type system (plan-97) instead of collapsing to a bare id like value types, so the
/// parse-time type-normalization seams consult this to decide. Backed by the resource
/// table (keyed by the qualified identity), so it covers every resource uniformly —
/// clean-room registry (`process`) and old-branch (`fs`/`net`/`tls`/`audio`) alike.
pub(crate) fn is_qualified_builtin_resource(qualified: &str) -> bool {
    resource::is_builtin_resource_type(&ParameterType::declared(qualified))
}

/// Resolve a package-qualified built-in type reference (`net.Url`,
/// `http.Response`) to its bare internal type id, or `None` when it is not a
/// qualified built-in type (plan-03-http.md §A.1).
pub(crate) fn qualified_builtin_type(qualified: &str) -> Option<String> {
    // Every builtin value type now resolves through the clean-room registry
    // (`csv.CsvReader`, `net.Url`, `term.TermSize`/`term.LineStyle`, …) — package-scoped
    // there, so a cross pairing (`io.Url`, `csv.Thread`) is correctly rejected (bug-98).
    // `term` was the last package to retain a hand-written fallback arm; with it
    // migrated, no per-package fallback remains.
    crate::codegen::registry::registry().qualified_builtin_type(qualified)
}

pub(crate) fn resource_close_function(type_: &ParameterType) -> Option<&'static str> {
    resource::builtin_resource_close_function(&type_)
}

pub(crate) fn is_resource_type(type_: &ParameterType) -> bool {
    resource::is_builtin_resource_type(&type_)
}

pub(crate) fn is_thread_sendable_resource_type(type_: &ParameterType) -> bool {
    resource::is_builtin_sendable_resource_type(&type_)
}

/// The bare native lowering name for a migrated `collections::`/`strings::`
/// member (plan-01-functions.md §5). The native code generator stays keyed on the
/// original bare names (`get`, `transform`, `find`, `mid`, `replace`, ...), so the
/// IR call target for these members is dequalified back to the bare name. Returns
/// `None` for every other call (including the `collections::` source generics,
/// which the monomorphizer rewrites to `__collections_X` instead).
pub(crate) fn native_builtin_target(name: &str) -> Option<&'static str> {
    // `find`/`mid`/`replace` dequalify to the same bare native name for both their
    // `strings::` (String) and `collections::` (List) overloads — the pair of
    // `Body::Intrinsic` members the registry does not distinguish from any other
    // intrinsic, so they are handled here by name rather than through the registry.
    if let Some(member) = name
        .strip_prefix("strings.")
        .or_else(|| name.strip_prefix("collections."))
    {
        match member {
            "find" => return Some("find"),
            "mid" => return Some("mid"),
            "replace" => return Some("replace"),
            _ => {}
        }
    }
    // Every other migrated collections native member (`get`, `set`, `transform`, …)
    // owns a `Body::abi_inline` call-site lowering; the registry hands back its bare name.
    crate::codegen::registry::native_bare_target(name)
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
pub(crate) fn inline_trap_unsupported(target: &str, arg_types: &[ParameterType]) -> bool {
    (native_builtin_target(target).is_some() || matches!(target, "len" | "toString" | "typeName"))
        && !inline_builtin_raw_supported(target, arg_types)
        && !inline_builtin_is_infallible(target, arg_types)
}

/// The one inline built-in whose fallibility depends on its **argument type**
/// rather than its name (bug-486).
///
/// `toString` is overloaded across every type, and exactly one of those overloads
/// can fail: `List OF Byte → String` decodes UTF-8 and raises `ErrEncoding`
/// (`77020004`) on an ill-formed sequence
/// (`builder_strings.rs:emit_byte_list_to_string_value`, the `invalid` label). Every
/// other overload — `Integer`, `Float`, `Boolean`, `Scalar`, `AttributedString`,
/// a record — is total, which is why the name-keyed verdict looked safe and why
/// deleting `"toString"` from the census outright is the wrong fix: it would make
/// `toString(42)` fallible and force `Result` plumbing through every interpolation.
///
/// `len` and `typeName`, the other two name-keyed general built-ins, were audited
/// for the same hazard and have none: `lower_len`'s two arms (String, collection)
/// emit no error return at all, and `typeName` folds to a string constant at
/// compile time.
///
/// A caller that cannot type its arguments passes an empty slice (or `Unknown`),
/// which answers the name-keyed verdict — the same answer as before this existed.
/// That is the *under*-approximating side, so every site that can type its
/// arguments must: [`inline_builtin_is_infallible`]'s consumers act on this to
/// decide whether an inline `TRAP`'s handler is live.
pub(crate) fn arg_type_makes_inline_builtin_fallible(
    target: &str,
    arg_types: &[ParameterType],
) -> bool {
    inline_builtin_fallibility_depends_on_args(target)
        && matches!(
            arg_types.first(),
            Some(ParameterType::ListOf(element)) if **element == ParameterType::Byte
        )
}

/// Whether `target`'s verdict depends on its argument types at all — the cheap
/// gate a caller uses to skip typing arguments it would not consult.
///
/// Deliberately the *same* name list [`arg_type_makes_inline_builtin_fallible`]
/// tests, not a second copy of it: a recogniser and its measurer kept as two
/// lists is exactly the shape that loses an entry when one grows. Adding a name
/// here without a rule there only costs wasted work; the reverse would be a
/// miscompile, and the shared `matches!` makes it impossible.
pub(crate) fn inline_builtin_fallibility_depends_on_args(target: &str) -> bool {
    matches!(target, "toString")
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
/// `strings.mid`, ...); `arg_types` discriminates the one overload-dependent
/// entry (see [`arg_type_makes_inline_builtin_fallible`]).
pub(crate) fn inline_builtin_raw_supported(target: &str, arg_types: &[ParameterType]) -> bool {
    // bug-486: `toString(<List OF Byte>)` is the one overload-dependent entry. Its
    // raw lowering is `lower_to_string` run under a `raw_result_capture`, exactly
    // like the members below.
    if arg_type_makes_inline_builtin_fallible(target, arg_types) {
        return true;
    }
    // A migrated common-native member that declares an error is fallible: its raw
    // lowering redirects the domain error to the inline-`TRAP` capture point. The
    // `bits` variable shifts (`sl`/`sr`/`sra`) raise `ErrInvalidArgument` on an
    // out-of-range count and so report `Some(true)` here — the census is grounded
    // in registry data, not a `bits.` name predicate.
    matches!(
        crate::codegen::registry::native_member_declares_error(target),
        Some(true)
    ) || matches!(
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
/// Infallible: `len`, `typeName`, `toString` on every argument type **except**
/// `List OF Byte` (bug-486 — that overload decodes UTF-8 and raises
/// `ErrEncoding`; see [`arg_type_makes_inline_builtin_fallible`]), every total
/// `bits::*` op (all but the variable shifts), and the pure-query /
/// default-returning / OOM-only members `contains`, `hasKey`, `keys`, `values`,
/// `sum`, `getOr`, `append`, `prepend`, `removeKey`, `replace`.
///
/// Fallible (NOT infallible — raw-supported, so an inline `TRAP` traps their real
/// error): the `bits::` variable shifts `sl`/`sr`/`sra` (out-of-range count
/// raises `ErrInvalidArgument`), the index members `get`/`set`/`insert`/`removeAt`,
/// `strings::mid`, `find` (negative start raises), and the callback members
/// `forEach`/`transform`/`filter`/`reduce`/`reduceRight` (a failing callback
/// raises a real error). `target` is the canonical callee (`collections.get`, `strings.mid`,
/// `bits.sl`) or a bare general-builtin name.
pub(crate) fn inline_builtin_is_infallible(target: &str, arg_types: &[ParameterType]) -> bool {
    // bug-486: the verdict is name-keyed *except* for the overloads
    // `arg_type_makes_inline_builtin_fallible` names — today only
    // `toString(<List OF Byte>)`, whose UTF-8 decode raises `ErrEncoding`.
    if arg_type_makes_inline_builtin_fallible(target, arg_types) {
        return false;
    }
    // A migrated common-native member is infallible when it declares no error and
    // is not otherwise raw-supported. Every `bits` op qualifies (empty `errors`)
    // except the three variable shifts, which declare `ErrInvalidArgument`
    // (`Some(true)`) and are raw-supported instead; the collections callback
    // members are raw-supported despite an empty `errors` list, so `!raw_supported`
    // excludes them. Keyed on registry data, not a `bits.` name predicate.
    if matches!(
        crate::codegen::registry::native_member_declares_error(target),
        Some(false)
    ) && !inline_builtin_raw_supported(target, arg_types)
    {
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
///
/// The module owning `callee` resolves it via its own co-located resolver, or,
/// for a fully data-only package, via the registry's exact per-position match.
/// The registry guarantees each qualified name is owned by exactly one module
/// (`duplicate_function_name` is `None`), so this is order-independent —
/// replacing the hand-ordered per-package `resolve_call` chain it grew from.
///
/// plan-111-G: this doc used to open "Typed twin of `resolve_call_return_type`
/// (plan-104-C)" and describe a render-in/parse-out pocket at the three bespoke
/// resolvers. Both are gone — letter C retyped `general`/`vector`/`strings` and
/// deleted the string twin, so this is no longer a twin of anything and there is
/// no pocket. The old text survived as an ORPHANED doc block after its item was
/// deleted, silently concatenated onto its neighbour's docs.
pub(crate) fn resolve_call_return_type_typed(
    callee: &str,
    arg_types: &[crate::types::ParameterType],
    strict: bool,
) -> Option<crate::types::ParameterType> {
    // Migrated (clean-room registry) packages resolve through the generic
    // matcher: `resolve_call_typed` validates arity and argument types (yielding
    // `None` on a mismatch, which the type checker turns into an error), so this
    // cannot blindly hand back the return type. `strict` rejects a
    // scalar-for-nominal argument; the lenient callers (return-type inference
    // feeding IR lowering / codegen) keep the coarse match so a nominally-spelled
    // argument does not perturb type propagation.
    //
    // Three packages carry a computed return the generic matcher cannot express
    // and keep their own co-located resolver:
    //
    // * `general` — bare-named, so disjoint from every qualified member and
    //   order-independent here; its argument-dependent returns come from its own
    //   hand-authored table.
    // * `vector` — dispatches by EXACT record type (`Float2` != `Integer2`) with
    //   a per-type return, which the coarse-nominal matcher cannot select.
    // * `strings` — carries the `AttributedString` Tier-A/Tier-B return typing,
    //   deferring to the generic path for every other call (plan-99 PART B).
    //
    // plan-111-C: all three take and return `ParameterType` now, so this is the
    // ONE entry — the render-in/parse-out pocket plan-104-C recorded here as a
    // deliberate boundary is gone, and so is the string twin that fed it.
    if general::is_general_call(callee) {
        return general::resolve_return_type(callee, arg_types);
    }
    if crate::codegen::registry::registry().owning_package(callee) == Some("vector") {
        return crate::codegen::builtins::vector::resolve_return_type(callee, arg_types);
    }
    if crate::codegen::registry::registry().owning_package(callee) == Some("strings") {
        return crate::codegen::builtins::strings::resolve_return_type(callee, arg_types, strict);
    }
    if crate::codegen::registry::registry().is_member(callee) {
        return crate::codegen::registry::resolve_call_typed(callee, arg_types, strict);
    }
    None
}

/// The static (argument-independent) nominal return type of a builtin call, as a
/// rendered SPELLING — plan-72-BB: the owning module's static return (a
/// `Custom`-return call has no static nominal and yields `None`; the
/// arg-validated return lives in [`resolve_call_return_type_typed`]).
///
/// plan-111-G: its only production callers are in `binary_repr/writer.rs`, the
/// `.mfp` ENCODER, which needs the spelling because that is what the wire stores.
/// The render is therefore the point here, not a leftover — everything on the
/// compiler side asks [`call_return_type`], the typed twin below. The
/// lowered-only internal names
/// (`audio` device opens / timed I/O, `tls.closeListener`) are not descriptor
/// functions, so IR lowering's queries for their rewritten targets fall back to
/// those two packages' explicit internal-name maps.
pub(crate) fn call_return_type_name(name: &str) -> Option<std::borrow::Cow<'static, str>> {
    // `general` (bare-named) is disjoint from every qualified member. Only the six
    // numeric narrowing conversions carry a static nominal return; every other general
    // call is `Custom` and yields `None`, reproducing the legacy fast-oracle exactly.
    if general::is_general_call(name) {
        return general::nominal_return_type(name)
            .map(|type_| std::borrow::Cow::Owned(type_.name().into_owned()));
    }
    // `vector` members have an ARGUMENT-dependent return type (`length(Float3) AS
    // Float`, `length(Integer3) AS Integer`) with no static nominal — the pre-migration
    // `ReturnType::Custom` yielded `None` here. The generic `call_return_type` below
    // would coarsely report the first overload's return (the matcher cannot pick by
    // record type), so `vector` is excluded to preserve the `None`; its arg-validated
    // return lives in `resolve_call_return_type`.
    if crate::codegen::registry::registry().owning_package(name) == Some("vector") {
        return None;
    }
    // plan-111-C: the registry has ONE query, typed. This oracle still hands its
    // seven codegen callers a NAME, so it renders here — those signatures are
    // letters D-F's.
    if let Some(return_type) = crate::codegen::registry::call_return_type_typed(name) {
        return Some(std::borrow::Cow::Owned(return_type.name().into_owned()));
    }
    None
}

/// The typed twin of [`call_return_type_name`] (plan-106-A): the static
/// (argument-independent) nominal return of a builtin call as a
/// [`ParameterType`]. Same dispatch, same `None`s — `general`'s six numeric
/// narrowing conversions map to their scalar variants, `vector` is excluded to
/// preserve its `None`, and the registry path clones the descriptor's already-
/// typed return instead of rendering it.
pub(crate) fn call_return_type(name: &str) -> Option<crate::types::ParameterType> {
    if general::is_general_call(name) {
        return general::nominal_return_type(name);
    }
    if crate::codegen::registry::registry().owning_package(name) == Some("vector") {
        return None;
    }
    crate::codegen::registry::call_return_type_typed(name)
}

/// The name of the builtin package that owns a fully qualified call, or `None`
/// (plan-72-BB: the registry's single owner). Used by the former source checker's dispatcher
/// to select a table package's argument-inference mode without a per-package
/// `is_<pkg>_call` chain.
pub(crate) fn builtin_package_name(callee: &str) -> Option<&'static str> {
    crate::codegen::registry::registry().owning_package(callee)
}

/// The arity range `(min, max)` of a builtin call — plan-72-BB: the owning
/// module's `DefaultResolver::arity`. `None` for a call no package owns.
pub(crate) fn arity(name: &str) -> Option<(usize, usize)> {
    // `general` (bare-named) never carries a `.`, so `registry().arity` (which requires a
    // qualified name) would miss it. The boundary helper resolves the `general.<name>`
    // key, reproducing the legacy arity — including `error`'s `None` (validated by
    // `resolve_call`, not the arity gate) — so an over/under-argument general call still
    // reports `TYPE_CALL_ARITY_MISMATCH` rather than an argument mismatch.
    if general::is_general_call(name) {
        return general::arity(name);
    }
    crate::codegen::registry::registry().arity(name)
}

/// The packages whose calls the shared table checker validates (arity and
/// arg-typed overload resolution). `general`, `collections`, `term` and
/// `thread` have bespoke checkers and are deliberately absent; a package in
/// neither set has its calls' arguments merely inferred, never bound or
/// validated. Shared by `ir::shape` (the named-argument rules) and `ir::verify`
/// (the arity/argument rules) so both draw the same boundary (plan-107-E).
/// A package missing from this list is NOT a compile error and produces no
/// warning — its calls are merely inferred, so an arity or argument-type mistake
/// degrades into a bare `TYPE_UNKNOWN_VALUE` on the binding instead of naming the
/// problem. plan-110-B added `tcp` here after observing exactly that: with `tcp`
/// absent, `tcp::connect(1, 80)` reported nothing at all while the identical
/// `net::connectTcp(1, 80)` reported `TYPE_CALL_ARGUMENT_MISMATCH`. Any new
/// package needs a row here.
const ARGUMENT_CHECKED_PACKAGES: &[&str] = &[
    "encoding", "astrings", "crypto", "strings", "math", "bits", "fs", "os", "net", "tcp", "tls",
    "audio", "process", "io", "json", "csv", "regex", "datetime", "money", "app", "http", "udp",
    "vector", "color",
];

/// Whether a builtin call (canonical `package.member` name) is checked by the
/// package TABLE arm (`expected_arguments` overloads) — as opposed to the four
/// bespoke arms. The table arm types a matched call by the overload's declared
/// return type even when an argument's own type is unknown.
pub(crate) fn table_checked_call(callee: &str) -> bool {
    crate::codegen::registry::registry()
        .owning_package(callee)
        .is_some_and(|package| ARGUMENT_CHECKED_PACKAGES.contains(&package))
}

/// Whether a builtin call (canonical `package.member` name) reaches one of the
/// argument checkers — the four bespoke arms or the package table — and so has
/// its argument list normalized and validated.
pub(crate) fn checks_call_arguments(callee: &str) -> bool {
    general::is_general_call(callee)
        || matches!(
            crate::codegen::registry::registry().owning_package(callee),
            Some("collections") | Some("term")
        )
        || thread::is_thread_call(callee)
        || builtin_package_name(callee)
            .is_some_and(|package| ARGUMENT_CHECKED_PACKAGES.contains(&package))
}

/// The human-readable expected-argument rendering for a builtin call's
/// argument-mismatch diagnostic — plan-72-BB. Most packages render per-position
/// from the descriptor (`DefaultResolver::expected_arguments`); the packages whose
/// phrasing is an argument *union* (`"Socket or Listener or UdpSocket"`) or prose keep
/// their hand-authored string, which the descriptor's per-position join cannot
/// reproduce (a genuine non-descriptor behavior, per BB's non-goals). The migrated
/// `vector` package's prose (`"two vectors of the same type"`) rides on the
/// `RegistryFunction::expected_arguments` field, served by `registry::expected_arguments`.
pub(crate) fn expected_arguments(name: &str) -> Option<String> {
    // Every package that still owns an `expected_arguments` free function keeps its
    // hand-authored phrasing — the `[optional]` bracket (`strings.find`'s
    // `"String, String[, Integer]"`), the `"or"`-union, or prose — that the
    // descriptor's per-position join cannot reproduce. Each returns `Some` only for
    // its own calls, so the chain yields the owner's string. The migrated
    // (clean-room registry) packages — csv/json/regex/collections/datetime/encoding/
    // process — no longer own an `expected_arguments` seam: their bespoke phrasing
    // rides on the `RegistryFunction::expected_arguments` descriptor field and is
    // served by the generic `registry::expected_arguments` below.
    if let Some(text) = general::expected_arguments(name)
        .or_else(|| crate::codegen::registry::expected_arguments(name))
    {
        return Some(text.to_string());
    }
    None
}

/// The concrete per-position argument-type signature IR lowering uses for literal
/// coercion (bug-340 A1), or `None` when the call has no single positional
/// signature (generic/overloaded members, or a bracketed/`"or"`-phrased
/// description). plan-72-BB: this is the exact heuristic ir/lower previously
/// inlined, relocated here so the per-package reads live behind one aggregate.
/// Packages carrying a machine-readable positional table are read directly;
/// `collections`/`vector` are absent on purpose (every member is generic or
/// overloaded, so the monomorphizer types them). The migrated
/// (clean-room registry) packages — datetime/encoding, formerly read from a
/// machine table here — now derive through the generic `registry::expected_arguments`
/// path below: a member with a concrete positional signature renders one, and the
/// non-signature shapes (variadic `"1 to 5 Integer"`, zero-arg `"()"`, the
/// optional-tail brackets, `utf8Decode`'s `"or"`-union) decline via the guard.
/// The typed form (plan-106-A). Same dispatch and the same
/// `None`s; the registry half clones already-typed descriptor params, and the
/// `general` half classifies its *descriptor text* through the canonical grammar
/// — `general::expected_arguments` is a hand-authored signature string, so this
/// is that table's one text→type boundary rather than a re-parse of a render.
pub(crate) fn argument_types_typed(callee: &str) -> Option<Vec<crate::types::ParameterType>> {
    // Migrated packages: the registry's MACHINE coercion table (positional parameter
    // types), decoupled from the human `expected_arguments` diagnostic string so
    // widening the diagnostic never changes per-argument coercion (bug-443). A generic
    // or overloaded member yields `None` here and needs no coercion table.
    // plan-111-F: this used to fall back to splitting `general`'s hand-authored
    // `expected_arguments` diagnostic string and parsing each piece. That tail is
    // DEAD and is deleted: every arm of that table is an argument union
    // (`" or "`), a bracketed optional, a variadic range, or a bare placeholder —
    // except `isNumeric`, `isEven` and `isOdd`, and the registry branch above
    // answers for all three. Pinned by
    // `plan111f_probe::general_scalar_predicates_resolve_through_the_registry`,
    // which fails if any of them ever stops resolving here, rather than silently
    // falling through to a text split that no longer exists.
    crate::codegen::registry::argument_types_typed(callee)
}

/// The expected type of argument `index` when every overload of `callee` agrees on
/// it (plan-120-D). Answers the position `argument_types_typed` declines for an
/// overload set, so union wrapping survives a member gaining a second overload —
/// see [`crate::codegen::registry::agreed_argument_type`] for why that matters.
pub(crate) fn agreed_argument_type(
    callee: &str,
    index: usize,
) -> Option<crate::types::ParameterType> {
    crate::codegen::registry::agreed_argument_type(callee, index)
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
    // Honored generically from the registry's `internal_only: true` members —
    // e.g. `astrings`' overlay-bridge natives (`readSpans`/`writeSpans`/`scalarLen`,
    // plan-99 PART C). The crypto NIST-EC raw generators that once needed a
    // per-package predicate here were collapsed into the public `generateP*`
    // members, so no bespoke crypto case remains.
    crate::codegen::registry::registry().is_internal_only_member(name)
}

pub(crate) fn is_builtin_call(name: &str) -> bool {
    // The `audio::` lowered-only internal names (device opens, timed I/O, the
    // per-direction closes) are `os_aliases`, not registry members, so `is_member`
    // and the `call_return_type_name` tail both decline them — `audio::readTimeout()`
    // in user source draws an unknown-function diagnostic, never a silent miscompile
    // (bug-213), with no explicit guard needed.
    // Migrated (clean-room registry) packages first; else the old REGISTRY.
    // plan-72-BB: descriptor membership is every package's `is_<pkg>_call`
    // (`DefaultResolver::contains`). (`vector`'s constants are registry constants,
    // admitted through `is_package_constant`, not as calls.) The
    // `call_return_type_name` tail preserves the pre-existing admission of lowered-only
    // names whose return type is known (e.g. `tls.closeListener`).
    // `general` (the unqualified global builtins) is bare-named, so `is_member` (which
    // requires a `.`) misses it and `call_return_type_name` reports `None` for the
    // `Custom`-return members (`len`/`typeName`/`isEmpty`/…). Recognize it explicitly —
    // this is the membership the legacy `REGISTRY.function(<bare>)` used to carry.
    general::is_general_call(name)
        || crate::codegen::registry::registry().is_member(name)
        || call_return_type_name(name).is_some()
}

pub(crate) fn is_builtin_member(name: &str) -> bool {
    is_builtin_call(name) || is_package_constant(name)
}

/// A compile-time package constant that folds to a literal: `math::pi` and
/// friends (`Float`/`Fixed`) or an `errorCode::Err*` registry value (`Integer`).
/// These are keyed package-qualified (`"math.pi"`, `"errorCode.ErrNotFound"`).
pub(crate) fn is_package_constant(name: &str) -> bool {
    crate::codegen::registry::is_package_constant(name)
}

/// A package constant's type (plan-106-A).
pub(crate) fn package_constant_type(name: &str) -> Option<crate::types::ParameterType> {
    crate::codegen::registry::constant_type_name(name)
}

pub(crate) fn package_constant_value(name: &str) -> Option<&'static str> {
    crate::codegen::registry::constant_value(name)
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
pub(crate) fn call_param_name_overloads(name: &str) -> Option<Vec<Vec<&'static str>>> {
    // datetime's front-dropping constructors (instant/duration/fixedOffset) are served by
    // the generic registry, which derives the per-overload table from each
    // implementation's parameters. Every package that carried an overload-disagreeing
    // param table has migrated, so the registry is the sole provider.
    crate::codegen::registry::call_param_name_overloads(name)
}

/// Pick the overload a call selects, given how many arguments were passed
/// positionally and the names of the rest.
///
/// The chosen overload takes exactly this many arguments, names every supplied
/// name, and places none of those names in a slot a positional argument already
/// filled. Both the type checker and IR lowering resolve named arguments through
/// this, so they cannot disagree about which parameter a name binds to.
pub(crate) fn select_param_name_overload<'a>(
    overloads: &'a [Vec<&'a str>],
    positional_count: usize,
    names: &[&str],
) -> Option<&'a [&'a str]> {
    overloads
        .iter()
        .find(|params| {
            params.len() == positional_count + names.len()
                && names.iter().all(|name| {
                    params
                        .iter()
                        .position(|param| param == name)
                        .is_some_and(|index| index >= positional_count)
                })
        })
        .map(|params| params.as_slice())
}

pub(crate) fn call_param_names(name: &str) -> Option<Vec<Vec<&'static str>>> {
    // Migrated (clean-room registry) packages first, then the legacy per-package tables.
    if let Some(names) = crate::codegen::registry::call_param_names(name) {
        return Some(names);
    }
    // `astrings` migrated to the clean-room registry (plan-99 PART C); its per-position
    // parameter names are served by the generic `registry::call_param_names` above.
    let borrowed: &'static [&'static [&'static str]] = general::call_param_names(name)?;
    Some(borrowed.iter().map(|aliases| aliases.to_vec()).collect())
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
        // Man rendering became registry-driven (the Markdown `src/docs/man`
        // corpus was retired), so enumerate the builtin members from the clean-
        // room registry — the source of truth the man pages are now generated
        // from — rather than a directory of pages.
        let mut names = Vec::new();
        for package in crate::codegen::registry::registry().packages() {
            for function in package.functions() {
                names.push(format!("{}.{}", package.import_name(), function.name));
            }
        }
        assert!(
            names.len() > 100,
            "expected the registry builtin corpus, got {} members",
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
            Some("net.Url".to_string())
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
                for alias in aliases {
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
            for params in &overloads {
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
            assert!(
                inline_builtin_is_infallible(c, &[]),
                "expected infallible: {c}"
            );
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
            assert!(
                !inline_builtin_is_infallible(c, &[]),
                "expected fallible: {c}"
            );
        }
        // Every inline member is classified one way or the other, and non-inline
        // callees (user functions) are not infallible built-ins.
        assert!(!inline_builtin_is_infallible("myFunc", &[]));
        assert!(!inline_builtin_is_infallible("math.sqrt", &[]));
    }

    /// bug-486: the census answers per OVERLOAD for the names whose fallibility
    /// depends on the argument type. `toString(<List OF Byte>)` decodes UTF-8 and
    /// raises `ErrEncoding`; every other `toString` is total, and the name-keyed
    /// answer above must be untouched for them.
    #[test]
    fn tostring_is_fallible_only_on_a_byte_list() {
        let bytes = [ParameterType::list_of(ParameterType::Byte)];
        assert!(!inline_builtin_is_infallible("toString", &bytes));
        assert!(inline_builtin_raw_supported("toString", &bytes));
        assert!(!inline_trap_unsupported("toString", &bytes));

        // Every other overload — including the two-argument precision form, a
        // list of something else, and the no-types fallback — stays infallible.
        for args in [
            vec![ParameterType::Integer],
            vec![ParameterType::Float, ParameterType::Byte],
            vec![ParameterType::String],
            vec![ParameterType::list_of(ParameterType::Integer)],
            vec![ParameterType::Unknown],
            vec![],
        ] {
            assert!(
                inline_builtin_is_infallible("toString", &args),
                "expected infallible: toString{args:?}"
            );
            assert!(!inline_builtin_raw_supported("toString", &args));
        }

        // The cheap gate every consumer uses to decide whether to type its
        // arguments must agree with the rule it gates: a name the gate skips can
        // never be flipped by an argument type, or the consumer would skip the
        // typing and get the wrong verdict.
        for name in [
            "toString",
            "len",
            "typeName",
            "collections.get",
            "strings.mid",
            "bits.sl",
            "myFunc",
        ] {
            assert!(
                inline_builtin_fallibility_depends_on_args(name)
                    || !arg_type_makes_inline_builtin_fallible(name, &bytes),
                "{name} is flipped by an argument type but the gate skips typing it"
            );
        }

        // The overload rule is `toString`'s alone: `len` and `typeName` were
        // audited and have no fallible overload, so a byte-list argument must not
        // flip them (`lower_len`'s two arms emit no error return; `typeName` folds
        // to a string constant at compile time).
        for name in ["len", "typeName"] {
            assert!(
                inline_builtin_is_infallible(name, &bytes),
                "expected infallible: {name}(List OF Byte)"
            );
        }
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
                inline_builtin_raw_supported(c, &[]),
                "expected raw-supported: {c}"
            );
            assert!(
                !inline_trap_unsupported(c, &[]),
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
                inline_builtin_raw_supported(c, &[]),
                "expected raw-supported: {c}"
            );
            assert!(
                !inline_trap_unsupported(c, &[]),
                "raw-supported must not be unsupported: {c}"
            );
        }
        // The infallible members are NOT raw-supported (nothing to capture) but are
        // still trappable via the always-`Ok` path — so also not unsupported.
        for c in ["collections.contains", "len", "bits.band"] {
            assert!(
                !inline_builtin_raw_supported(c, &[]),
                "expected NOT raw-supported: {c}"
            );
            assert!(
                !inline_trap_unsupported(c, &[]),
                "infallible must not be unsupported: {c}"
            );
        }
    }

    /// The full import-gated package set. Kept in one place so the `is_builtin_import`
    /// predicate and the `mfb spec language builtin-functions` §18 list cannot drift
    /// apart (plan-33-D Phase 2 — the earlier `money` omission recurred because no
    /// such test existed).
    ///
    /// plan-122-A: this was a hand-written *copy* of the predicate's arm, and a copy
    /// is exactly what the test was supposed to make impossible — it silently omitted
    /// `tcp` and `udp`, so §18's package sentence omitted them too while §18's own
    /// transport paragraph documented `IMPORT tcp`/`IMPORT udp`. It is now an alias
    /// for the production list, so the §18 comparison below is against the predicate
    /// itself and no third spelling exists to drift.
    const ALL_BUILTIN_PACKAGES: &[&str] = super::BUILTIN_IMPORTS;

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
            .find("[[src/codegen/builtins/mod.rs:is_builtin_import]]")
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
            "color",
        ] {
            assert!(is_builtin_import(pkg), "{pkg}");
        }
        assert!(!is_builtin_import("nope"));
        assert!(!is_builtin_import("resource"));
    }

    #[test]
    fn is_builtin_type_aggregates() {
        // A thread type routes through the registry's type table. plan-106-C
        // deleted the `builtins::is_builtin_type` wrapper — the former source checker's parser
        // was its last caller, and that call discarded the answer — so these
        // assertions follow it to the registry.
        assert!(crate::codegen::registry::registry().is_builtin_type("Thread"));
        assert!(!crate::codegen::registry::registry().is_builtin_type("Integer"));
        assert!(!crate::codegen::registry::registry().is_builtin_type("List OF Integer"));
    }

    #[test]
    fn general_override_target_cases() {
        assert_eq!(
            general_override_target(
                "toString",
                &crate::types::ParameterType::parse(crate::codegen::builtins::net::URL_TYPE),
            ),
            Some("__net_urlToString")
        );
        assert_eq!(
            general_override_target("toString", &crate::types::ParameterType::parse("Integer")),
            None
        );
        assert_eq!(
            general_override_target(
                "len",
                &crate::types::ParameterType::parse(crate::codegen::builtins::net::URL_TYPE),
            ),
            None
        );
    }

    #[test]
    fn qualified_builtin_type_cases() {
        // net.Url -> bare Url type id.
        let url = qualified_builtin_type("net.Url");
        assert_eq!(url.as_deref(), Some("net.Url"));
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
        assert!(is_resource_type(&crate::types::ParameterType::declared(
            "fs.File"
        )));
        assert!(!is_resource_type(&crate::types::ParameterType::declared(
            "Integer"
        )));
        assert!(
            resource_close_function(&crate::types::ParameterType::declared("fs.File")).is_some()
        );
        assert!(
            resource_close_function(&crate::types::ParameterType::declared("Integer")).is_none()
        );
        // is_thread_sendable_resource_type routes to resource module.
        let _ = is_thread_sendable_resource_type(&crate::types::ParameterType::declared("fs.File"));
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
                !inline_trap_unsupported(target, &[]),
                "expected trappable (not unsupported): {target}"
            );
        }
    }

    #[test]
    fn call_return_type_name_aggregates() {
        // general
        assert_eq!(call_return_type_name("toInt").as_deref(), Some("Integer"));
        // strings::find contributes a return type through the aggregate.
        assert_eq!(
            call_return_type_name("strings.find").as_deref(),
            Some("Integer")
        );
        assert_eq!(call_return_type_name("nope").as_deref(), None);
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
        assert!(package_constant_type("math.pi").is_some());
        assert!(package_constant_type("nope").is_none());
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

#[cfg(test)]
mod plan111f_probe {
    /// plan-111-F: is `argument_types_typed`'s `general::expected_arguments`
    /// fallback reachable? Every arm of that table is a union (`" or "`), a
    /// bracketed optional, or a bare placeholder — EXCEPT `isNumeric`, `isEven`
    /// and `isOdd`. If the registry branch above answers for those three, the
    /// string-splitting tail is dead and its `ParameterType::parse` with it.
    #[test]
    fn general_scalar_predicates_resolve_through_the_registry() {
        for (call, expected) in [
            ("general.isNumeric", crate::types::ParameterType::String),
            ("general.isEven", crate::types::ParameterType::Integer),
            ("general.isOdd", crate::types::ParameterType::Integer),
        ] {
            assert_eq!(
                crate::codegen::registry::argument_types_typed(call),
                Some(vec![expected]),
                "{call} must resolve through the registry, not the string tail"
            );
        }
    }
}
