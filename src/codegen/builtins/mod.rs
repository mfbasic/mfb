//! Builtin packages whose lowering has migrated into the codegen layer
//! (plan-95). Each package owns its `BuiltinFunction` descriptors and, per
//! migrated function, the target-generic lowering carried by `Implementation`.

pub(crate) mod app;
pub(crate) mod astrings;
pub(crate) mod audio;
pub(crate) mod big;
pub(crate) mod bits;
pub(crate) mod canvas;
pub(crate) mod collections;
pub(crate) mod color;
pub(crate) mod compress;
pub(crate) mod crypto;
pub(crate) mod csv;
pub(crate) mod datetime;
pub(crate) mod encoding;
pub(crate) mod errorcode;
pub(crate) mod float_result;
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
pub(crate) mod totality_proof;
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
    "big",
    "bits",
    "canvas",
    "collections",
    "color",
    "compress",
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

/// `name` with a BUILT-IN package qualifier stripped (`fs.File` → `File`), and
/// otherwise unchanged.
///
/// The two type-compatibility checks (`ir::verify::compat::compatible`,
/// `ir::shape`'s copy) fall back to bare-name equality because a `.mfp` may
/// record a built-in type by its bare base name even though the built-in's own
/// identity is package-qualified (plan-97/bug-441). bug-632 makes a USER
/// package's type package-qualified end to end, so for those the fallback must
/// not fire: a consumer's own `A`, `pa::A` and `pb::A` are three distinct types,
/// and equating them by the last `.` segment is exactly the confusion this
/// qualification exists to prevent.
pub(crate) fn builtin_qualified_bare_leaf(name: &str) -> &str {
    match name.rsplit_once('.') {
        Some((qualifier, leaf)) if is_builtin_import(qualifier) => leaf,
        _ => name,
    }
}

/// The internal helper a built-in package provides as an **override** of an
/// overridable general built-in (`toString`, `len`, …) over one of its value
/// types (plan-01-overload.md §B.2). A general call `f(x)` whose sole argument
/// has such a type routes to this `__pkg_name` helper instead of the scalar
/// builtin; the name is internalized at lowering so it never collides with the
/// builtin dispatch symbol. Keyed by `(builtin, arg_type)`, where `arg_type` is the
/// type's PACKAGE-QUALIFIED identity (`net.Url`, `vector.Float2`) — a bare leaf is a
/// user type and never selects a package override (bug-668).
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
    // (`net.Url`, `term.TermSize`/`term.LineStyle`, …) — package-scoped
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
/// (uniform surface). The predicate survives only as the **test backstop**
/// (`tests/inline_trap_surface.rs` sweeps the registry), which fails if a *future*
/// inline builtin is added to `native_builtin_target` without also giving it a raw
/// or infallible lowering — catching the mistake instead of miscompiling. The front-end no
/// longer rejects anything here (the old `TYPE_INLINE_TRAP_ON_INLINED_BUILTIN`
/// diagnostic was retired in plan-26-C).
///
/// Excluded (already trappable): the conversion builtins
/// `toInt`/`toFloat`/`toFixed`/`toByte` (`lower_inline_conversion_raw`) and every
/// `runtime::helper_for_call` target (`lower_runtime_helper_call`); user
/// `FUNC` calls carry real symbols and match none of the member forms here.
///
/// `target` is the canonical, dot-qualified callee (`strings.find`,
/// `collections.get`, `bits.sl`) or a bare inline general-builtin name (`len`,
/// `toString`, `typeName`).
// Test-only: the set is empty (plan-26) and `tests/inline_trap_surface.rs` holds it
// empty over the registry; codegen has no path that could consult it.
#[cfg(test)]
pub(crate) fn inline_trap_unsupported(
    target: &str,
    arg_types: &[ParameterType],
    kinds: &dyn TypeKinds,
) -> bool {
    (native_builtin_target(target).is_some() || matches!(target, "len" | "toString" | "typeName"))
        && !inline_builtin_raw_supported(target, arg_types)
        && !inline_builtin_is_infallible(target, arg_types, kinds)
}

/// The inline built-ins whose fallibility depends on their **argument type**
/// rather than their name (bug-533, bug-679).
///
/// Two names are listed: `replace` (bug-533) and `toInt` (bug-679). `replace`'s
/// rule takes away a name-keyed infallibility from every overload that is not
/// provably the `List` one; this function applies that subtraction. `toInt` is
/// the mirror — fallible by name, with one overload that is total — so it
/// appears in the shared list (its verdict does turn on an argument type, which
/// is what [`inline_builtin_fallibility_depends_on_args`] gates on) but this
/// function answers `false` for it and the grant happens in
/// [`inline_builtin_is_infallible`], where the `TypeKinds` oracle is in scope.
///
/// `toString` is **not** listed. bug-486 once put it here because only its
/// `List OF Byte` overload (a UTF-8 decode raising `ErrEncoding`) could fail;
/// since bug-807 every overload builds a new `String`, declares
/// `ErrOutOfMemory`, and is fallible by the owner's rule (2026-10-04: a call
/// that can run out of memory is fallible), so its verdict no longer turns on
/// its argument type and [`inline_builtin_is_infallible`] answers `false` for
/// it by name.
///
/// `len` and `typeName`, the name-keyed infallible general built-ins, were
/// audited for the argument-type hazard and have none: `lower_len`'s two arms
/// (String, collection) emit no error return at all, and `typeName` folds to a
/// string constant at compile time.
///
/// `replace` (bug-533): `strings::replace` and `collections::replace` dequalify
/// to the one bare native target `replace` (`native_builtin_target`), so the
/// registry cannot answer for it by name. The `String` overload refuses an empty
/// `old` with `ErrInvalidArgument`; leaving `replace` on the name-keyed
/// infallible list made the front-end warn `TYPE_INLINE_TRAP_DEAD_HANDLER` on a
/// live handler and drop the error on the floor. Since bug-698 the `List`
/// overload allocates and declares `ErrOutOfMemory` too, so both overloads are
/// fallible either way (`inline_builtin_raw_supported` lists `replace`); this
/// rule is the fail-closed half of that verdict: anything that is not provably
/// the `List` overload counts as fallible.
///
/// A caller that cannot type its arguments passes an empty slice (or `Unknown`);
/// for `replace` that answers "fallible", the safe side, by the fail-closed rule.
/// [`inline_builtin_is_infallible`]'s consumers act on this to decide whether an
/// inline `TRAP`'s handler is live.
pub(crate) fn arg_type_makes_inline_builtin_fallible(
    target: &str,
    arg_types: &[ParameterType],
) -> bool {
    match inline_builtin_arg_fallibility_rule(target) {
        None => false,
        // `replace` over anything but a `List` — i.e. the `String` overload, and
        // any future overload — refuses an empty needle.
        Some(ArgFallibility::ReplaceOverAnythingButAList) => {
            !matches!(arg_types.first(), Some(ParameterType::ListOf(_)))
        }
        // bug-679: `toInt` is already fallible by name, so there is nothing for
        // this function — which only ever subtracts infallibility — to do. The
        // enum overload is granted its verdict in `inline_builtin_is_infallible`,
        // where the `TypeKinds` oracle that can recognise an enum is in scope.
        Some(ArgFallibility::ToIntOverAnEnum) => false,
    }
}

/// Which argument-typed fallibility rule a target uses. One value per entry in
/// the shared list below, so the recogniser and the measurer cannot drift.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ArgFallibility {
    ReplaceOverAnythingButAList,
    /// bug-679, and the one entry that points the other way. The one above
    /// *removes* infallibility from a name that has it; this one *grants* it to a
    /// single overload of a name that does not. It is listed here because the
    /// list's job is "whose verdict turns on an argument type" — which is what
    /// [`inline_builtin_fallibility_depends_on_args`] gates on, and a caller that
    /// skipped typing `toInt`'s arguments could never reach the rule.
    /// [`arg_type_makes_inline_builtin_fallible`] therefore answers `false` for
    /// it: `toInt` has no name-keyed infallibility to take away.
    ToIntOverAnEnum,
}

/// The single list of argument-typed entries, consulted by both
/// [`arg_type_makes_inline_builtin_fallible`] (which applies the rule) and
/// [`inline_builtin_fallibility_depends_on_args`] (which only asks whether one
/// exists). Keeping it as one function is what stops a recogniser and its
/// measurer becoming two lists that lose an entry.
fn inline_builtin_arg_fallibility_rule(target: &str) -> Option<ArgFallibility> {
    if native_builtin_target(target) == Some("replace") {
        return Some(ArgFallibility::ReplaceOverAnythingButAList);
    }
    if target == "toInt" {
        return Some(ArgFallibility::ToIntOverAnEnum);
    }
    None
}

/// Whether `target`'s verdict depends on its argument types at all — the cheap
/// gate a caller uses to skip typing arguments it would not consult.
///
/// Deliberately reads the *same* list [`arg_type_makes_inline_builtin_fallible`]
/// applies, not a second copy of it: a recogniser and its measurer kept as two
/// lists is exactly the shape that loses an entry when one grows. Adding a name
/// there is the only edit needed, and a name cannot appear in one and not the
/// other.
pub(crate) fn inline_builtin_fallibility_depends_on_args(target: &str) -> bool {
    inline_builtin_arg_fallibility_rule(target).is_some()
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
    // Every `toString` overload is fallible (it can raise `ErrOutOfMemory`, and the
    // `List OF Byte` one also `ErrEncoding`), and its raw lowering is
    // `lower_to_string` run under a `raw_result_capture`, like the members below.
    if target == "toString" {
        return true;
    }
    if arg_type_makes_inline_builtin_fallible(target, arg_types) {
        return true;
    }
    // A migrated common-native member that declares an error is fallible: its raw
    // lowering redirects the domain error to the inline-`TRAP` capture point. The
    // `bits` variable shifts (`sl`/`sr`/`sra`) raise `ErrInvalidArgument` on an
    // out-of-range count and so report `Some(true)` here — the census is grounded
    // in registry data, not a `bits.` name predicate.
    // plan-165-B: an inline member that is not proven total (it declares an
    // error, or no raise-checking guard covers it) is captured for real — an
    // inline `TRAP` on it must not be wrapped always-`Ok`. Its raw lowering runs
    // the member's ordinary inline body under the capture, so a member that
    // raises nothing simply always delivers `Ok`.
    totality_proof::inline_member_is_proven_total(target) == Some(false)
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
                // `replace`'s bare target is shared with `strings::replace`, so
                // the registry lookup above cannot answer for it. Both overloads
                // are fallible: the `String` one refuses an empty `old` (bug-533),
                // the `List` one allocates and raises `ErrOutOfMemory` (bug-698).
                | "replace"
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
/// invalid-format) or declares one. Since bug-698 every allocating member declares
/// `ErrOutOfMemory` (owner decision: every function lists every error it can
/// raise), and spec 8 rule 11 makes a declaring member fallible, so the
/// allocating members (`keys`/`values`/`append`/`prepend`/`removeKey`/`replace`,
/// Set `add`/`remove`/`toList`) are fallible and their inline-TRAP handler is live.
///
/// plan-165-B: a grant needs positive proof — every overload declares no error
/// AND a raise-checking guard covers the member (`totality_proof`); an empty
/// `errors` list alone is a claim, and an unproved member stays fallible and is
/// captured for real ([`inline_builtin_raw_supported`]).
///
/// Infallible: `len`, `typeName`, every total `bits::*` op (all but the variable shifts), the non-allocating pure-query /
/// default-returning members `contains`, `hasKey` and `getOr`, and `toInt` **on
/// a single enum argument** (bug-679 — the ordinal the value already holds, a
/// register move that declares no error; plan-140-B).
///
/// `toString` is **fallible on every overload** (bug-807; owner decision
/// 2026-10-04): each one builds a new `String` and can raise `ErrOutOfMemory`,
/// and `toString(<List OF Byte>)` also raises `ErrEncoding` (bug-486). Its
/// handler is live and never flagged `TYPE_INLINE_TRAP_DEAD_HANDLER`.
///
/// That last infallible entry is the only one that runs the other way: the
/// `replace` rule in [`arg_type_makes_inline_builtin_fallible`] takes
/// infallibility AWAY from a name, while `toInt` is fallible by default and one
/// overload is granted an exception. It is therefore
/// the only entry that needs `kinds`, because an enum is not its own
/// `ParameterType` — see [`TypeKinds`]. `NoTypeKinds` answers `false`, which is
/// the over-approximating side, so a caller that cannot supply a real oracle
/// keeps the verdict this had before the exception existed.
///
/// Fallible (NOT infallible — raw-supported, so an inline `TRAP` traps their real
/// error): the `bits::` variable shifts `sl`/`sr`/`sra` (out-of-range count
/// raises `ErrInvalidArgument`), the index members `get`/`set`/`insert`/`removeAt`,
/// `strings::mid`, `find` (negative start raises), and the callback members
/// `forEach`/`transform`/`filter`/`reduce`/`reduceRight` (a failing callback
/// raises a real error). `target` is the canonical callee (`collections.get`, `strings.mid`,
/// `bits.sl`) or a bare general-builtin name.
pub(crate) fn inline_builtin_is_infallible(
    target: &str,
    arg_types: &[ParameterType],
    kinds: &dyn TypeKinds,
) -> bool {
    // The verdict is name-keyed *except* for the overloads
    // `arg_type_makes_inline_builtin_fallible` names — today only a `replace`
    // that is not provably the `List` overload (bug-533).
    if arg_type_makes_inline_builtin_fallible(target, arg_types) {
        return false;
    }
    // bug-679: the one rule in the *granting* direction. `toInt` is fallible by
    // name and stays so; its single-enum overload is the lone exception, because
    // it compiles to a register move of the ordinal the enum value already holds
    // and declares no error (plan-140-B). The verdict comes from the oracle, never
    // from a spelling heuristic: a declared type is not an enum just because it is
    // declared, and a record that answered `true` here would silently delete a
    // live handler.
    if matches!(
        inline_builtin_arg_fallibility_rule(target),
        Some(ArgFallibility::ToIntOverAnEnum)
    ) && arg_types.len() == 1
        && kinds.is_enum(&arg_types[0])
    {
        return true;
    }
    // plan-165-B: an inline registry member is infallible only with positive
    // proof — every implementation declares no error AND a raise-checking guard
    // (a declared-errors sweep) covers it (`totality_proof`) — and when it is not
    // otherwise raw-supported. The collections callback members declare nothing
    // and are swept, but a failing callback raises through them, so they stay
    // raw-supported by name and `!raw_supported` excludes them. Missing or
    // unknown information counts as "can fail" (owner decision 2026-10-04).
    if totality_proof::inline_member_is_proven_total(target) == Some(true)
        && !inline_builtin_raw_supported(target, arg_types)
    {
        return true;
    }
    // `toString` is not here: every overload builds a new `String` and can raise
    // `ErrOutOfMemory` (bug-807; owner decision 2026-10-04: a call that can run out
    // of memory is fallible), so an inline `TRAP` handler on it is live.
    //
    // The string-concatenation operator `&` makes the SAME allocation and can
    // fail the SAME way, but is not a call this function ever sees (it is a
    // `BinaryOp` node, routed through `src/ir/fallible.rs::operator_can_raise`
    // instead) and is deliberately decided the other way from `toString`: see
    // `ARITHMETIC_OPERATORS`'s doc comment there for the owner decision
    // (2026-10-04, bug-814's open question) that keeps `&` out of the
    // TRAP-liveness / declared-errors census.
    // The `general` intrinsics with a raise-checking row per overload
    // (`general_declared_errors`) that raises nothing: `len`, `typeName`. The
    // non-allocating `collections::contains`/`hasKey`/`getOr` are granted by the
    // registry rule above; a bare-name list here would grant any package's member
    // of that name, declared errors or not (plan-165-B removed it).
    !target.contains('.') && totality_proof::builtin_is_raise_checked(target)
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
    resolve_call_return_type_with_kinds(callee, arg_types, strict, &NoTypeKinds)
}

/// plan-140-A: what a resolution site knows about the KIND of a declared type.
///
/// `ParameterType` spells every declared type (record, union, enum) as the same
/// opaque nominal, and deliberately so: it keys the registry and `TypeModel`, and
/// its `Hash`/`Eq` are pinned. So the resolver cannot tell an enum from a record by
/// the type alone; a site that holds the declarations answers for it.
///
/// Only the sites whose answer decides ACCEPTANCE (`ir::shape`, `ir::verify`) or
/// DISPATCH (monomorph's gap-fill override check) pass a real oracle. A typing-only
/// site keeps [`NoTypeKinds`] through [`resolve_call_return_type_typed`].
pub(crate) trait TypeKinds {
    /// Whether `t` names a declared `ENUM`. Must answer from the declaration's
    /// kind, never from "is a declared type": a record that answered `true` would
    /// silently lose a user override at monomorph's gap-fill check.
    fn is_enum(&self, t: &crate::types::ParameterType) -> bool;

    /// Whether the union `union` has `actual` among its variants, so a value of type
    /// `actual` is accepted where `union` is expected (and lowering wraps it). A
    /// generic call's top-level type variable bound to a union asks this (bug-988).
    /// The default knows only the built-in packages' unions; an oracle over declared
    /// types answers for a user union too.
    fn union_accepts(
        &self,
        union: &crate::types::ParameterType,
        actual: &crate::types::ParameterType,
    ) -> bool {
        crate::codegen::registry::registry_union_accepts(union, actual)
    }
}

/// The oracle that knows no declared kinds: every `is_enum` is `false`.
pub(crate) struct NoTypeKinds;

impl TypeKinds for NoTypeKinds {
    fn is_enum(&self, _t: &crate::types::ParameterType) -> bool {
        false
    }
}

/// [`resolve_call_return_type_typed`] with the caller's [`TypeKinds`] oracle, for
/// a site that decides whether a call is accepted or which body it dispatches to.
pub(crate) fn resolve_call_return_type_with_kinds(
    callee: &str,
    arg_types: &[crate::types::ParameterType],
    strict: bool,
    kinds: &dyn TypeKinds,
) -> Option<crate::types::ParameterType> {
    // Migrated (clean-room registry) packages resolve through the generic
    // matcher: `resolve_call_typed` validates arity and argument types (yielding
    // `None` on a mismatch, which the type checker turns into an error), so this
    // cannot blindly hand back the return type. `strict` rejects a
    // scalar-for-nominal argument; the lenient callers (return-type inference
    // feeding IR lowering / codegen) keep the coarse match so a nominally-spelled
    // argument does not perturb type propagation.
    //
    // Four packages carry a computed return the generic matcher cannot express
    // and keep their own co-located resolver:
    //
    // * `general` — bare-named, so disjoint from every qualified member and
    //   order-independent here; its argument-dependent returns come from its own
    //   hand-authored table.
    // * `vector` — dispatches by EXACT record type (`Float2` != `Integer2`) with
    //   a per-type return, which the coarse-nominal matcher cannot select.
    // * `strings` — carries the `AttributedString` Tier-A/Tier-B return typing,
    //   deferring to the generic path for every other call (plan-99 PART B).
    // * `regex` — carries the `AttributedString` Tier-A return typing for its query
    //   members (bug-534). Tier-A only: `regex::replace` has no attribute-preserving
    //   form, so it deliberately keeps the generic path and stays a type error for
    //   an `AttributedString`.
    //
    // plan-111-C: all three take and return `ParameterType` now, so this is the
    // ONE entry — the render-in/parse-out pocket plan-104-C recorded here as a
    // deliberate boundary is gone, and so is the string twin that fed it.
    if general::is_general_call(callee) {
        return general::resolve_return_type(callee, arg_types, kinds);
    }
    if crate::codegen::registry::registry().owning_package(callee) == Some("vector") {
        return crate::codegen::builtins::vector::resolve_return_type(callee, arg_types);
    }
    if crate::codegen::registry::registry().owning_package(callee) == Some("strings") {
        return crate::codegen::builtins::strings::resolve_return_type(callee, arg_types, strict);
    }
    if crate::codegen::registry::registry().owning_package(callee) == Some("regex") {
        return crate::codegen::builtins::regex::resolve_return_type(callee, arg_types, strict);
    }
    if crate::codegen::registry::registry().is_member(callee) {
        return crate::codegen::registry::resolve_call_typed_with_kinds(
            callee, arg_types, strict, kinds,
        );
    }
    None
}

/// The static (argument-independent) nominal return type of a builtin call, as a
/// rendered SPELLING — plan-72-BB: the owning module's static return (a
/// `Custom`-return call has no static nominal and yields `None`; the
/// arg-validated return lives in [`resolve_call_return_type_typed`]).
///
/// plan-111-G: its only production callers are in `binary_repr/writer/`, the
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
    "encoding", "astrings", "crypto", "strings", "math", "bits", "big", "fs", "os", "net", "tcp",
    "tls", "audio", "process", "io", "json", "csv", "regex", "datetime", "money", "app", "http",
    "udp", "vector", "color", "compress", "canvas",
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

/// The expected type of argument `index` at a call to `callee` whose actual argument
/// types are `arg_types`, for a GENERIC parameter the two functions above decline
/// (bug-550, `collections::append([], x)` does not build). See
/// [`crate::codegen::registry::resolved_parameter_type`].
pub(crate) fn resolved_argument_type(
    callee: &str,
    index: usize,
    arg_types: &[crate::types::ParameterType],
    expected_return: Option<&crate::types::ParameterType>,
    kinds: &dyn TypeKinds,
) -> Option<crate::types::ParameterType> {
    crate::codegen::registry::resolved_parameter_type(
        callee,
        index,
        arg_types,
        expected_return,
        kinds,
    )
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

/// The per-overload required-leading-parameter counts matching
/// [`call_param_name_overloads`] (same order); see
/// `registry::call_param_name_overload_required`.
pub(crate) fn call_param_name_overload_required(name: &str) -> Option<Vec<usize>> {
    crate::codegen::registry::call_param_name_overload_required(name)
}

/// Pick the overload a call selects, given how many arguments were passed
/// positionally and the names of the rest.
///
/// The chosen overload names every supplied name, places none of those names in
/// a slot a positional argument already filled, and is filled as a contiguous
/// prefix: the supplied arguments occupy exactly its first
/// `positional_count + names.len()` slots. An overload taking exactly that many
/// arguments wins. Failing one, a longer overload is chosen only when every slot
/// the call leaves out is DEFAULTED — `required[i]` is how many leading
/// parameters overload `i` needs — and those trailing slots then get the same
/// default fill a positional call gets (bug-596: `tls::connect`'s
/// `allowSelfSigned` is a trailing default of each overload, and requiring an
/// exact arity rejected every named call that left it out). This fails CLOSED:
/// an overload with no `required` entry never takes the fallback, so
/// `datetime::instant(days := 5)` — whose 5-arg form defaults nothing — still
/// selects nothing rather than binding `days` onto the 1-arg `seconds` slot
/// (bug-349). A gap before a later supplied name is never selected either; that
/// stays the "omits parameter" diagnostic. Both the type checker and IR lowering
/// resolve named arguments through this, so they cannot disagree about which
/// parameter a name binds to.
pub(crate) fn select_param_name_overload<'a>(
    overloads: &'a [Vec<&'a str>],
    required: &[usize],
    positional_count: usize,
    names: &[&str],
) -> Option<&'a [&'a str]> {
    let supplied = positional_count + names.len();
    let binds_prefix = |params: &Vec<&str>| {
        params.len() >= supplied
            && names.iter().all(|name| {
                params
                    .iter()
                    .position(|param| param == name)
                    .is_some_and(|index| index >= positional_count && index < supplied)
            })
    };
    overloads
        .iter()
        .find(|params| params.len() == supplied && binds_prefix(params))
        .or_else(|| {
            overloads
                .iter()
                .enumerate()
                .filter(|(index, params)| {
                    binds_prefix(params)
                        && required
                            .get(*index)
                            .is_some_and(|needed| supplied >= *needed)
                })
                .map(|(_, params)| params)
                .min_by_key(|params| params.len())
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
#[path = "tests/unit.rs"]
mod tests;

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

/// bug-553: the `tcp`/`tls` mirror pair's declared error lists, pinned against the
/// sets their lowerings can actually raise.
///
/// Twenty-eight of the pair's thirty-four registry implementations declared
/// `errors: vec![]` while raising `ErrConnectionClosed`, `ErrTimeout`,
/// `ErrTlsFailed` and `ErrNetworkFailed` in ordinary operation, so
/// `mfb man tcp read` and `mfb man tls write` shipped no Errors section at all.
///
/// The tables below are derived from each member's LOWERING — the `emit_fail`
/// closure over its selected helper, unioned across the three TLS backends —
/// rather than from the pages' prose, which is what was being corrected. Two
/// properties are pinned, and the second is the one an audit gets wrong:
///
/// - every error a member's lowering can raise is declared, and
/// - **no member declares an error its lowering cannot raise.** A wrong list is
///   worse than an empty one: it understates nothing but it lies, and it would
///   feed bad data to [`inline_builtin_is_infallible`] for any member that moved
///   to an inline-lowerable body.
///
/// Three traps this table is the record of:
///
/// - **Key by (package, member), never by bare member name.** `tcp` and `tls` are
///   mirror packages whose lowerings share every name (`lower_read`,
///   `lower_write`, `lower_close`, …); a bare-name census merges them and reports
///   that `tcp::close` raises `ErrTlsFailed`.
/// - **A raise site inside a Rust `if` on a lowering parameter is not in every
///   caller's set.** `lower_net_read_helper`'s `ErrEncoding` sits inside
///   `if text`, and `tcp::read` passes `text = false` — so `tcp::read` cannot
///   raise it. `lower_fs_close_helper`'s `ErrWriteFailed` sits inside
///   `if flush_on_close`, which `tcp`/`udp` pass `false`; that is why
///   `tcp::close`'s pre-existing three-error list was already right.
///   `lower_net_endpoint_helper` serves both `connect` and `listen`: its
///   `ErrTimeout` label is emitted on the listen path but nothing branches to it,
///   and `ErrAddressNotFound`/`ErrAddressInvalid` are the two arms of one `if
///   listen`.
/// - **A helper that `bl`s another helper by SYMBOL is a call-graph edge no Rust
///   call expression shows.** `lower_tls_poll_list_helper` branch-links
///   `_mfb_rt_tls_tls_poll` and propagates its error, so the list overload's set
///   is the scalar overload's plus `ErrInvalidArgument`/`ErrTimeout`.
#[cfg(test)]
#[path = "tests/tcp_tls_error_declaration.rs"]
mod tcp_tls_error_declaration_tests;
