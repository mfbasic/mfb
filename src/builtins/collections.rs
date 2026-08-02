use super::descriptor::{
    BuiltinFlags, BuiltinFunction, BuiltinModule, BuiltinOverload, BuiltinResolver, BuiltinSource,
    DefaultResolver, DefaultValue, Implementation, InjectionRule, Lowering, Parameter,
    ParameterType, ReturnType,
};
use crate::ast::{AstFile, AstProject};
use std::borrow::Cow;
use std::collections::HashMap;
use std::path::Path;

/// Path of the compiler-owned `collections` package source injected into every
/// project that imports it. This is the `AstFile.path` (see `source_file`), so
/// `AstProject::to_json` can filter it out of `-ast` output.
pub(crate) const SOURCE_PATH: &str = "builtins/collections.mfb";

/// The public `collections::` function names (without the `collections.`
/// qualifier). The implementations live in `collections_package.mfb` as generic
/// `__collections_<name>` functions; a user call `collections::sort(...)` is
/// rewritten to `__collections_sort(...)` during monomorphization so the generic
/// machinery instantiates it like any other generic function.
// `toMap`, `zipWith`, and `filterEntries` from §6.4 are not yet exported: they
// depend on runtime capabilities MFBASIC does not have today — storing the
// compiler-owned `MapEntry` record inside a `List` (toMap/filterEntries) and
// applying a two-argument function value element-wise (zipWith). They are
// deferred until that infrastructure lands; see plan-01-functions.md §6.4.
const FUNCTIONS: &[&str] = &[
    "sort",
    "sortBy",
    "take",
    "drop",
    "reduceRight",
    "any",
    "all",
    "findIndex",
    "findLastIndex",
    "groupBy",
    "mapValues",
    "flatten",
    "zip",
    "chunks",
    "window",
    "distinct",
    "merge",
    "partition",
    // Set algebra (plan-63-C): source generics over B's native Set members.
    "toSet",
    "union",
    "intersection",
    "difference",
    "symmetricDifference",
    "isSubset",
    "isSuperset",
    "isDisjoint",
];

/// The native `collections::` members migrated out of the bare global namespace
/// (plan-01-functions.md §5). These keep the native code generator's bare-name
/// lowering: the resolve logic is reused verbatim from `general`, and the IR
/// call target is dequalified back to the bare native name (see
/// `super::native_builtin_target`). `find`/`mid`/`replace` accept ONLY the List
/// overload here; their String overloads live in `strings::`.
const NATIVE_MEMBERS: &[&str] = &[
    "get",
    "getOr",
    "set",
    "append",
    "prepend",
    "insert",
    "removeAt",
    "removeKey",
    "keys",
    "values",
    "hasKey",
    "contains",
    "forEach",
    "transform",
    "filter",
    "reduce",
    "sum",
    "find",
    "mid",
    "replace",
    // Set members (plan-63-B): `add`/`remove`/`toList` are Set-only; `contains`
    // gains a Set overload alongside its List overload.
    "add",
    "remove",
    "toList",
];

// plan-72-E: `COLLECTIONS` is the descriptor authority for this package's NATIVE
// members (the source-generic `FUNCTIONS` above are resolved by the monomorphizer,
// not here, so they are not descriptor functions). The descriptor owns
// membership, per-position parameter names/aliases, and arity. Return-type
// resolution is genuinely generic (`get(List OF T, Integer) → T`, map/set/
// function-typed overloads), so it lives on a `BuiltinResolver` that delegates to
// the existing `dispatch_resolve` (below). Parameter *types* are documentation
// only — a member like `get` has List/Map overloads a single `ParameterType`
// cannot express, and no delegating wrapper reads them (resolution is
// resolver-owned; `expected_arguments` keeps its hand-authored "or"-phrased
// strings). The `.mfb` source companion is modelled as `WhenImported`.
const fn req(
    name: &'static str,
    aliases: &'static [&'static str],
    ty: &'static str,
) -> Parameter {
    Parameter {
        name,
        aliases,
        ty: ParameterType::Named(ty),
        default: DefaultValue::None,
    }
}

/// An optional trailing parameter (only `find`'s `start`). The `Fill` is inert:
/// collections has no default-argument padding, so nothing reads it — it exists
/// solely so `DefaultResolver::arity` derives `find`'s `(2, 3)` range.
const fn opt(
    name: &'static str,
    aliases: &'static [&'static str],
    ty: &'static str,
) -> Parameter {
    Parameter {
        name,
        aliases,
        ty: ParameterType::Named(ty),
        default: DefaultValue::Fill {
            type_name: ty,
            expr: "",
        },
    }
}

const fn native(
    name: &'static str,
    slug: &'static str,
    overloads: &'static [BuiltinOverload],
) -> BuiltinFunction {
    BuiltinFunction {
        name,
        doc_slug: slug,
        overloads,
        implementation: Implementation::Same,
        lowering: Lowering::Helper,
        flags: BuiltinFlags {
            internal_only: false,
            return_type_overloaded: false,
        },
    }
}

// One overload per member, carrying the merged parameter-name table; the resolver
// owns the actual per-overload (List/Map/Set) type resolution, so `return_type` is
// `Custom` throughout and the parameter *types* are documentation only.
const fn custom(params: &'static [Parameter]) -> BuiltinOverload {
    BuiltinOverload {
        params,
        return_type: ReturnType::Custom,
    }
}

const COLLECTIONS_FUNCTIONS: &[BuiltinFunction] = &[
    native("collections.get", "get", &[custom(&[
        req("value", &["collection"], "List OF T"),
        req("index", &["key"], "Integer"),
    ])]),
    native("collections.getOr", "getOr", &[custom(&[
        req("value", &["collection"], "List OF T"),
        req("index", &["key"], "Integer"),
        req("default", &["fallback"], "T"),
    ])]),
    native("collections.set", "set", &[custom(&[
        req("value", &["collection"], "List OF T"),
        req("index", &["key"], "Integer"),
        req("item", &[], "T"),
    ])]),
    native("collections.append", "append", &[custom(&[
        req("value", &["list"], "List OF T"),
        req("item", &["items"], "T"),
    ])]),
    native("collections.prepend", "prepend", &[custom(&[
        req("value", &["list"], "List OF T"),
        req("item", &[], "T"),
    ])]),
    native("collections.insert", "insert", &[custom(&[
        req("value", &["list"], "List OF T"),
        req("index", &[], "Integer"),
        req("item", &[], "T"),
    ])]),
    native("collections.removeAt", "removeAt", &[custom(&[
        req("value", &["list"], "List OF T"),
        req("index", &[], "Integer"),
    ])]),
    native("collections.removeKey", "removeKey", &[custom(&[
        req("value", &["map"], "Map OF K TO V"),
        req("key", &[], "K"),
    ])]),
    native("collections.keys", "keys", &[custom(&[req("value", &["map"], "Map OF K TO V")])]),
    native("collections.values", "values", &[custom(&[req("value", &["map"], "Map OF K TO V")])]),
    native("collections.hasKey", "hasKey", &[custom(&[
        req("value", &["map"], "Map OF K TO V"),
        req("key", &[], "K"),
    ])]),
    native("collections.contains", "contains", &[custom(&[
        req("value", &["collection"], "List OF T"),
        req("item", &[], "T"),
    ])]),
    native("collections.forEach", "forEach", &[custom(&[
        req("value", &["collection"], "List OF T"),
        req("action", &[], "FUNC(T) AS Nothing"),
    ])]),
    native("collections.transform", "transform", &[custom(&[
        req("value", &["collection"], "List OF T"),
        req("f", &["transform"], "FUNC(T) AS U"),
    ])]),
    native("collections.filter", "filter", &[custom(&[
        req("value", &["collection"], "List OF T"),
        req("predicate", &[], "FUNC(T) AS Boolean"),
    ])]),
    native("collections.reduce", "reduce", &[custom(&[
        req("value", &["collection"], "List OF T"),
        req("initial", &["seed"], "U"),
        req("f", &["combine"], "FUNC(U, T) AS U"),
    ])]),
    native("collections.sum", "sum", &[custom(&[req("value", &["collection"], "List OF Number")])]),
    native("collections.find", "find", &[custom(&[
        req("value", &["list"], "List OF T"),
        req("item", &["needle"], "T"),
        opt("start", &[], "Integer"),
    ])]),
    native("collections.mid", "mid", &[custom(&[
        req("value", &["list"], "List OF T"),
        req("start", &[], "Integer"),
        req("count", &[], "Integer"),
    ])]),
    native("collections.replace", "replace", &[custom(&[
        req("value", &["list"], "List OF T"),
        req("old", &["needle"], "T"),
        req("new", &["replacement"], "T"),
    ])]),
    native("collections.add", "add", &[custom(&[
        req("value", &["set"], "Set OF T"),
        req("item", &["element"], "T"),
    ])]),
    native("collections.remove", "remove", &[custom(&[
        req("value", &["set"], "Set OF T"),
        req("item", &["element"], "T"),
    ])]),
    native("collections.toList", "toList", &[custom(&[req("value", &["set"], "Set OF T")])]),
];

/// Generic return-type resolution for collections native members. Delegates to
/// the same `dispatch_resolve` logic the public `resolve_call` used pre-migration,
/// so every List/Map/Set/function-typed overload resolves byte-identically.
struct CollectionsResolver;
impl BuiltinResolver for CollectionsResolver {
    fn resolve_return_type(
        &self,
        _module: &BuiltinModule,
        name: &str,
        arg_types: &[String],
    ) -> Option<String> {
        dispatch_resolve(name, arg_types).map(|resolved| resolved.return_type.into_owned())
    }
}
static COLLECTIONS_RESOLVER: CollectionsResolver = CollectionsResolver;

pub(crate) static COLLECTIONS: BuiltinModule = BuiltinModule {
    name: "collections",
    functions: COLLECTIONS_FUNCTIONS,
    types: &[],
    source: Some(BuiltinSource {
        rule: InjectionRule::WhenImported,
        loader: source_file,
    }),
    resolver: Some(&COLLECTIONS_RESOLVER),
};

/// The internal generic-function name implementing a public `collections::`
/// member, e.g. `sort` -> `#collections_sort`. The injected package is lexed in
/// internal mode, so its `__collections_*` definitions carry the internal sigil;
/// the monomorphizer's rewrite target must match.
pub(crate) fn internal_name(member: &str) -> String {
    crate::internal_name::internalize(&format!("__collections_{member}"))
}

/// Whether `member` is a public `collections::` function name.
pub(crate) fn is_collections_function(member: &str) -> bool {
    FUNCTIONS.contains(&member)
}

/// Whether `member` is a migrated native `collections::` member (`get`,
/// `transform`, the List overloads of `find`/`mid`/`replace`, ...).
pub(crate) fn is_native_member(member: &str) -> bool {
    NATIVE_MEMBERS.contains(&member)
}

/// Whether `name` (a canonical `collections.<fn>` call) names a `collections::`
/// builtin — either a source generic function (`sort`, ...) or a migrated native
/// member (`get`, ...). Used by the resolver's builtin-member check.
pub(crate) fn is_collections_call(name: &str) -> bool {
    name.strip_prefix("collections.")
        .is_some_and(|member| is_collections_function(member) || is_native_member(member))
}

/// Whether `name` is a migrated native `collections::` member call
/// (`collections.get`, ...). Used to route the call into `general`'s resolve
/// logic and to dequalify the IR target back to the bare native name.
pub(crate) fn is_native_member_call(name: &str) -> bool {
    DefaultResolver::contains(&COLLECTIONS, name)
}

/// Whether a native `collections.<member>` call takes a **unary callback over
/// the list's element type** as its second argument.
///
/// These are the positions where a bare general built-in predicate (`isEven`,
/// `isPositive`, …) must resolve: the callback's parameter type is not written
/// at the call site, it is the element type of the first argument, so the
/// checker has to bind it before the predicate reference can be typed
/// (bug-368).
///
/// `reduce` is deliberately absent — its callback is binary, so no unary
/// predicate fits it.
pub(crate) fn unary_callback_member(name: &str) -> bool {
    unary_callback_member_bare(name.strip_prefix("collections.").unwrap_or(name))
}

/// The bare-member form of [`unary_callback_member`], for the unqualified call
/// spelling that reaches `ir::lower` before canonicalization.
pub(crate) fn unary_callback_member_bare(name: &str) -> bool {
    matches!(name, "filter" | "transform" | "forEach")
}

/// `collections.get` -> `get`. Returns `None` for source generic functions and
/// non-`collections` names.
pub(crate) fn native_member_bare(name: &str) -> Option<&str> {
    name.strip_prefix("collections.")
        .filter(|member| is_native_member(member))
}

/// Resolves a `collections.<member>` native-member call by routing through the
/// descriptor's `BuiltinResolver` (plan-72-E), which delegates to
/// `dispatch_resolve`. The returned type string is identical to the pre-migration
/// path; only the `Cow` variant changes (`Owned` vs `Borrowed`), which no
/// consumer observes.
pub(crate) fn resolve_call<'a>(
    name: &str,
    arg_types: &'a [String],
) -> Option<super::general::ResolvedCall<'a>> {
    let return_type = COLLECTIONS
        .resolver?
        .resolve_return_type(&COLLECTIONS, name, arg_types)?;
    Some(super::general::ResolvedCall {
        return_type: Cow::Owned(return_type),
    })
}

/// The generic per-member resolution, delegating to the granular
/// `general::resolve_*`/local `resolve_*` helpers (which carry the original
/// bare-name semantics). `find`/`mid`/`replace` use the List-only overload here;
/// their String overloads live in `strings::`. Invoked through the descriptor
/// resolver by `resolve_call`.
fn dispatch_resolve<'a>(
    name: &str,
    arg_types: &'a [String],
) -> Option<super::general::ResolvedCall<'a>> {
    match native_member_bare(name)? {
        "get" => resolve_get(arg_types),
        "getOr" => resolve_get_or(arg_types),
        "set" => resolve_set(arg_types),
        "append" => resolve_append(arg_types),
        "prepend" => resolve_prepend(arg_types),
        "insert" => resolve_insert(arg_types),
        "removeAt" => resolve_remove_at(arg_types),
        "removeKey" => resolve_remove_key(arg_types),
        "keys" => resolve_keys(arg_types),
        "values" => resolve_values(arg_types),
        "hasKey" => resolve_has_key(arg_types),
        "contains" => resolve_contains(arg_types),
        "forEach" => resolve_for_each(arg_types),
        "transform" => resolve_transform(arg_types),
        "filter" => resolve_filter(arg_types),
        "reduce" => resolve_reduce(arg_types),
        "sum" => resolve_sum(arg_types),
        "find" => resolve_find_list(arg_types),
        "mid" => resolve_mid_list(arg_types),
        "replace" => resolve_replace_list(arg_types),
        "add" => resolve_set_add(arg_types),
        "remove" => resolve_set_remove(arg_types),
        "toList" => resolve_set_to_list(arg_types),
        _ => None,
    }
}

/// `collections::add(Set OF T, T) AS Set OF T` (plan-63-B): insert an element,
/// idempotent (a duplicate is dropped). Set-only — a List uses `append`.
fn resolve_set_add<'a>(arg_types: &'a [String]) -> Option<super::general::ResolvedCall<'a>> {
    if arg_types.len() != 2 {
        return None;
    }
    let element = super::general::set_element(&arg_types[0])?;
    (arg_types[1] == element).then_some(super::general::ResolvedCall {
        return_type: Cow::Borrowed(&arg_types[0]),
    })
}

/// `collections::remove(Set OF T, T) AS Set OF T` (plan-63-B): remove an element;
/// removing an absent element is a no-op. Set-only.
fn resolve_set_remove<'a>(arg_types: &'a [String]) -> Option<super::general::ResolvedCall<'a>> {
    if arg_types.len() != 2 {
        return None;
    }
    let element = super::general::set_element(&arg_types[0])?;
    (arg_types[1] == element).then_some(super::general::ResolvedCall {
        return_type: Cow::Borrowed(&arg_types[0]),
    })
}

/// `collections::toList(Set OF T) AS List OF T` (plan-63-B): the elements in
/// stable insertion order. Set-only.
fn resolve_set_to_list<'a>(arg_types: &'a [String]) -> Option<super::general::ResolvedCall<'a>> {
    if arg_types.len() != 1 {
        return None;
    }
    let element = super::general::set_element(&arg_types[0])?;
    Some(super::general::ResolvedCall {
        return_type: Cow::Owned(format!("List OF {element}")),
    })
}

/// List-overload resolvers for `find`/`mid`/`replace`, migrated to `collections::`
/// (plan-01-functions.md §5). These keep the original bare-name overload logic so
/// `collections::` can reuse it; the String overloads live in `strings::`.
fn resolve_find_list<'a>(arg_types: &'a [String]) -> Option<super::general::ResolvedCall<'a>> {
    if !(2..=3).contains(&arg_types.len()) {
        return None;
    }
    let element = super::general::list_element(&arg_types[0])?;
    (arg_types.get(2).is_none_or(|type_| type_ == "Integer")
        && (arg_types[1] == element || arg_types[1] == arg_types[0]))
        .then_some(super::general::ResolvedCall {
            return_type: Cow::Borrowed("Integer"),
        })
}

fn resolve_mid_list<'a>(arg_types: &'a [String]) -> Option<super::general::ResolvedCall<'a>> {
    (arg_types.len() == 3
        && super::general::list_element(&arg_types[0]).is_some()
        && arg_types[1] == "Integer"
        && arg_types[2] == "Integer")
        .then_some(super::general::ResolvedCall {
            return_type: Cow::Borrowed(&arg_types[0]),
        })
}

fn resolve_replace_list<'a>(arg_types: &'a [String]) -> Option<super::general::ResolvedCall<'a>> {
    // Arity first: `arg_types[0]`/`list_element` must not be indexed before the
    // length is known, or an empty/short slice panics (bug-98).
    if arg_types.len() != 3 {
        return None;
    }
    let element = super::general::list_element(&arg_types[0])?;
    (arg_types[1] == element && arg_types[2] == element).then_some(super::general::ResolvedCall {
        return_type: Cow::Borrowed(&arg_types[0]),
    })
}

fn resolve_get<'a>(arg_types: &'a [String]) -> Option<super::general::ResolvedCall<'a>> {
    if arg_types.len() != 2 {
        return None;
    }
    if let Some(element) = super::general::list_element(&arg_types[0]) {
        return (arg_types[1] == "Integer").then_some(super::general::ResolvedCall {
            return_type: Cow::Borrowed(element),
        });
    }
    let (key, value) = super::general::map_parts(&arg_types[0])?;
    (arg_types[1] == key).then_some(super::general::ResolvedCall {
        return_type: Cow::Borrowed(value),
    })
}

fn resolve_get_or<'a>(arg_types: &'a [String]) -> Option<super::general::ResolvedCall<'a>> {
    if arg_types.len() != 3 {
        return None;
    }
    if let Some(element) = super::general::list_element(&arg_types[0]) {
        return (arg_types[1] == "Integer" && arg_types[2] == element).then_some(
            super::general::ResolvedCall {
                return_type: Cow::Borrowed(element),
            },
        );
    }
    let (key, value) = super::general::map_parts(&arg_types[0])?;
    (arg_types[1] == key && arg_types[2] == value).then_some(super::general::ResolvedCall {
        return_type: Cow::Borrowed(value),
    })
}

fn resolve_set<'a>(arg_types: &'a [String]) -> Option<super::general::ResolvedCall<'a>> {
    if arg_types.len() != 3 {
        return None;
    }
    if let Some(element) = super::general::list_element(&arg_types[0]) {
        return (arg_types[1] == "Integer" && arg_types[2] == element).then_some(
            super::general::ResolvedCall {
                return_type: Cow::Borrowed(&arg_types[0]),
            },
        );
    }
    let (key, value) = super::general::map_parts(&arg_types[0])?;
    (arg_types[1] == key && arg_types[2] == value).then_some(super::general::ResolvedCall {
        return_type: Cow::Borrowed(&arg_types[0]),
    })
}

fn resolve_append<'a>(arg_types: &'a [String]) -> Option<super::general::ResolvedCall<'a>> {
    if arg_types.len() != 2 {
        return None;
    }
    let element = super::general::list_element(&arg_types[0])?;
    (arg_types[1] == element || arg_types[1] == arg_types[0]).then_some(
        super::general::ResolvedCall {
            return_type: Cow::Borrowed(&arg_types[0]),
        },
    )
}

fn resolve_prepend<'a>(arg_types: &'a [String]) -> Option<super::general::ResolvedCall<'a>> {
    if arg_types.len() != 2 {
        return None;
    }
    let element = super::general::list_element(&arg_types[0])?;
    (arg_types[1] == element).then_some(super::general::ResolvedCall {
        return_type: Cow::Borrowed(&arg_types[0]),
    })
}

fn resolve_insert<'a>(arg_types: &'a [String]) -> Option<super::general::ResolvedCall<'a>> {
    if arg_types.len() != 3 {
        return None;
    }
    let element = super::general::list_element(&arg_types[0])?;
    (arg_types[1] == "Integer" && arg_types[2] == element).then_some(super::general::ResolvedCall {
        return_type: Cow::Borrowed(&arg_types[0]),
    })
}

fn resolve_remove_at<'a>(arg_types: &'a [String]) -> Option<super::general::ResolvedCall<'a>> {
    (arg_types.len() == 2
        && super::general::list_element(&arg_types[0]).is_some()
        && arg_types[1] == "Integer")
        .then_some(super::general::ResolvedCall {
            return_type: Cow::Borrowed(&arg_types[0]),
        })
}

fn resolve_remove_key<'a>(arg_types: &'a [String]) -> Option<super::general::ResolvedCall<'a>> {
    if arg_types.len() != 2 {
        return None;
    }
    let (key, _) = super::general::map_parts(&arg_types[0])?;
    (arg_types[1] == key).then_some(super::general::ResolvedCall {
        return_type: Cow::Borrowed(&arg_types[0]),
    })
}

fn resolve_keys<'a>(arg_types: &'a [String]) -> Option<super::general::ResolvedCall<'a>> {
    if arg_types.len() != 1 {
        return None;
    }
    let (key, _) = super::general::map_parts(&arg_types[0])?;
    Some(super::general::ResolvedCall {
        return_type: Cow::Owned(format!("List OF {key}")),
    })
}

fn resolve_values<'a>(arg_types: &'a [String]) -> Option<super::general::ResolvedCall<'a>> {
    if arg_types.len() != 1 {
        return None;
    }
    let (_, value) = super::general::map_parts(&arg_types[0])?;
    Some(super::general::ResolvedCall {
        return_type: Cow::Owned(format!("List OF {value}")),
    })
}

fn resolve_has_key<'a>(arg_types: &'a [String]) -> Option<super::general::ResolvedCall<'a>> {
    if arg_types.len() != 2 {
        return None;
    }
    let (key, _) = super::general::map_parts(&arg_types[0])?;
    (arg_types[1] == key).then_some(super::general::ResolvedCall {
        return_type: Cow::Borrowed("Boolean"),
    })
}

fn resolve_contains<'a>(arg_types: &'a [String]) -> Option<super::general::ResolvedCall<'a>> {
    if arg_types.len() != 2 {
        return None;
    }
    // `contains` has a List overload (linear scan) and a Set overload (hash
    // probe, plan-63-B); both take `(collection, element) AS Boolean`.
    let element = super::general::list_element(&arg_types[0])
        .or_else(|| super::general::set_element(&arg_types[0]))?;
    (arg_types[1] == element).then_some(super::general::ResolvedCall {
        return_type: Cow::Borrowed("Boolean"),
    })
}

fn resolve_sum<'a>(arg_types: &'a [String]) -> Option<super::general::ResolvedCall<'a>> {
    if arg_types.len() != 1 {
        return None;
    }
    match arg_types[0].as_str() {
        "List OF Integer" => Some(super::general::ResolvedCall {
            return_type: Cow::Borrowed("Integer"),
        }),
        "List OF Float" => Some(super::general::ResolvedCall {
            return_type: Cow::Borrowed("Float"),
        }),
        "List OF Fixed" => Some(super::general::ResolvedCall {
            return_type: Cow::Borrowed("Fixed"),
        }),
        _ => None,
    }
}

fn resolve_for_each<'a>(arg_types: &'a [String]) -> Option<super::general::ResolvedCall<'a>> {
    if arg_types.len() != 2 {
        return None;
    }
    let element = super::general::list_element(&arg_types[0])?;
    let (params, returns) = super::general::function_parts(&arg_types[1])?;
    (params.len() == 1 && params[0] == element && returns == "Nothing").then_some(
        super::general::ResolvedCall {
            return_type: Cow::Borrowed("Nothing"),
        },
    )
}

fn resolve_transform<'a>(arg_types: &'a [String]) -> Option<super::general::ResolvedCall<'a>> {
    if arg_types.len() != 2 {
        return None;
    }
    let element = super::general::list_element(&arg_types[0])?;
    let (params, returns) = super::general::function_parts(&arg_types[1])?;
    (params.len() == 1 && params[0] == element && returns != "Nothing").then_some(
        super::general::ResolvedCall {
            return_type: Cow::Owned(format!("List OF {returns}")),
        },
    )
}

fn resolve_filter<'a>(arg_types: &'a [String]) -> Option<super::general::ResolvedCall<'a>> {
    if arg_types.len() != 2 {
        return None;
    }
    let element = super::general::list_element(&arg_types[0])?;
    let (params, returns) = super::general::function_parts(&arg_types[1])?;
    (params.len() == 1 && params[0] == element && returns == "Boolean").then_some(
        super::general::ResolvedCall {
            return_type: Cow::Borrowed(&arg_types[0]),
        },
    )
}

fn resolve_reduce<'a>(arg_types: &'a [String]) -> Option<super::general::ResolvedCall<'a>> {
    if arg_types.len() != 3 {
        return None;
    }
    let element = super::general::list_element(&arg_types[0])?;
    let (params, returns) = super::general::function_parts(&arg_types[2])?;
    (params.len() == 2
        && params[0] == arg_types[1]
        && params[1] == element
        && returns == arg_types[1])
        .then_some(super::general::ResolvedCall {
            return_type: Cow::Borrowed(&arg_types[1]),
        })
}

// `call_param_names` returns a `&'static` borrowed nested slice the owned
// `DefaultResolver::param_names` cannot produce, so it stays a static literal
// PINNED equal to `COLLECTIONS` by `parity_matches_descriptor` until plan-72-BB.
// `expected_arguments` keeps its hand-authored "or"-phrased strings (the
// descriptor's per-position types cannot express `List OF T, Integer or Map OF K
// TO V, K`), and `call_return_type_name` delegates to `general` — neither is
// descriptor-derivable, so both stay as-is (documented in the plan Corrections).
pub(crate) fn call_param_names(name: &str) -> Option<&'static [&'static [&'static str]]> {
    match native_member_bare(name)? {
        "get" => Some(&[&["value", "collection"], &["index", "key"]]),
        "getOr" => Some(&[
            &["value", "collection"],
            &["index", "key"],
            &["default", "fallback"],
        ]),
        "set" => Some(&[&["value", "collection"], &["index", "key"], &["item"]]),
        "append" => Some(&[&["value", "list"], &["item", "items"]]),
        "prepend" => Some(&[&["value", "list"], &["item"]]),
        "insert" => Some(&[&["value", "list"], &["index"], &["item"]]),
        "removeAt" => Some(&[&["value", "list"], &["index"]]),
        "removeKey" => Some(&[&["value", "map"], &["key"]]),
        "keys" => Some(&[&["value", "map"]]),
        "values" => Some(&[&["value", "map"]]),
        "hasKey" => Some(&[&["value", "map"], &["key"]]),
        "contains" => Some(&[&["value", "collection"], &["item"]]),
        "forEach" => Some(&[&["value", "collection"], &["action"]]),
        "transform" => Some(&[&["value", "collection"], &["f", "transform"]]),
        "filter" => Some(&[&["value", "collection"], &["predicate"]]),
        "reduce" => Some(&[
            &["value", "collection"],
            &["initial", "seed"],
            &["f", "combine"],
        ]),
        "sum" => Some(&[&["value", "collection"]]),
        "find" => Some(&[&["value", "list"], &["item", "needle"], &["start"]]),
        "mid" => Some(&[&["value", "list"], &["start"], &["count"]]),
        "replace" => Some(&[
            &["value", "list"],
            &["old", "needle"],
            &["new", "replacement"],
        ]),
        // Set members (plan-63-B).
        "add" => Some(&[&["value", "set"], &["item", "element"]]),
        "remove" => Some(&[&["value", "set"], &["item", "element"]]),
        "toList" => Some(&[&["value", "set"]]),
        _ => None,
    }
}

pub(crate) fn call_return_type_name(name: &str) -> Option<&'static str> {
    super::general::call_return_type_name(native_member_bare(name)?)
}

pub(crate) fn expected_arguments(name: &str) -> Option<&'static str> {
    match native_member_bare(name)? {
        "get" => Some("List OF T, Integer or Map OF K TO V, K"),
        "getOr" => Some("List OF T, Integer, T or Map OF K TO V, K, V"),
        "set" => Some("List OF T, Integer, T or Map OF K TO V, K, V"),
        "append" => Some("List OF T, T or List OF T, List OF T"),
        "prepend" => Some("List OF T, T"),
        "insert" => Some("List OF T, Integer, T"),
        "removeAt" => Some("List OF T, Integer"),
        "removeKey" => Some("Map OF K TO V, K"),
        "keys" => Some("Map OF K TO V"),
        "values" => Some("Map OF K TO V"),
        "hasKey" => Some("Map OF K TO V, K"),
        "contains" => Some("List OF T, T"),
        "forEach" => Some("List OF T, FUNC(T) AS Nothing"),
        "transform" => Some("List OF T, FUNC(T) AS U"),
        "filter" => Some("List OF T, FUNC(T) AS Boolean"),
        "reduce" => Some("List OF T, U, FUNC(U, T) AS U"),
        "sum" => Some("List OF Integer, List OF Float, or List OF Fixed"),
        "find" => Some("List OF T, T[, Integer]"),
        "mid" => Some("List OF T, Integer, Integer"),
        "replace" => Some("List OF T, T, T"),
        // Set members (plan-63-B). `contains` also accepts `Set OF T, T` (its
        // overload); the message above stays List-first to match its historical
        // shape.
        "add" => Some("Set OF T, T"),
        "remove" => Some("Set OF T, T"),
        "toList" => Some("Set OF T"),
        _ => None,
    }
}

pub(crate) fn arity(name: &str) -> Option<(usize, usize)> {
    DefaultResolver::arity(&COLLECTIONS, name)
}

/// Whether any file in `ast` imports the `collections` package.
pub(crate) fn uses_package(ast: &AstProject) -> bool {
    ast.files.iter().any(|file| {
        file.imports
            .iter()
            .any(|import| import.package_name() == "collections")
    })
}

/// Parses the built-in `collections` package source.
pub(crate) fn source_file() -> Result<AstFile, ()> {
    crate::ast::parse_source_internal(
        Path::new(SOURCE_PATH),
        SOURCE_PATH,
        include_str!("collections_package.mfb"),
    )
}

/// Injects the `collections` package source into `ast` when the project imports
/// it. The source is appended last (so the monomorphizer's first-file emission
/// target is unchanged) and is filtered out of `-ast` output by its sentinel
/// path. Call rewriting (`collections.sort` -> `__collections_sort`) happens in
/// the monomorphizer.
pub(crate) fn augmented_project(ast: AstProject) -> Result<AstProject, ()> {
    if !uses_package(&ast) {
        return Ok(ast);
    }
    let mut augmented = ast;
    augmented.files.push(source_file()?);
    Ok(augmented)
}

/// Builds a binding-name -> package-name map covering every `collections` import
/// (including aliases) across the project. The monomorphizer uses it to map a
/// call's `binding.member` callee onto the internal generic implementation.
pub(crate) fn collections_bindings(ast: &AstProject) -> HashMap<String, ()> {
    let mut bindings = HashMap::new();
    for file in &ast.files {
        for import in &file.imports {
            if import.package_name() == "collections" {
                bindings.insert(import.binding_name().to_string(), ());
            }
        }
    }
    bindings
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn rc(r: Option<crate::builtins::general::ResolvedCall>) -> Option<String> {
        r.map(|r| r.return_type.into_owned())
    }

    fn project(src: &str) -> AstProject {
        let file =
            crate::ast::parse_source(Path::new("main.mfb"), "main.mfb", src).expect("parse source");
        AstProject {
            name: "test".to_string(),
            files: vec![file],
        }
    }

    #[test]
    fn function_and_native_membership() {
        assert!(is_collections_function("sort"));
        assert!(is_collections_function("partition"));
        assert!(!is_collections_function("get"));
        assert!(!is_collections_function("nope"));

        assert!(is_native_member("get"));
        assert!(is_native_member("replace"));
        assert!(!is_native_member("sort"));
        assert!(!is_native_member("nope"));
    }

    #[test]
    fn is_collections_call_cases() {
        assert!(is_collections_call("collections.sort")); // source generic
        assert!(is_collections_call("collections.get")); // native member
        assert!(!is_collections_call("collections.nope"));
        assert!(!is_collections_call("strings.find"));
        assert!(!is_collections_call("sort"));
    }

    #[test]
    fn native_member_call_and_bare() {
        assert!(is_native_member_call("collections.get"));
        assert!(!is_native_member_call("collections.sort"));
        assert!(!is_native_member_call("get"));
        assert_eq!(native_member_bare("collections.get"), Some("get"));
        assert_eq!(native_member_bare("collections.sort"), None);
        assert_eq!(native_member_bare("get"), None);
    }

    #[test]
    fn internal_name_shape() {
        let name = internal_name("sort");
        assert!(name.contains("collections_sort"), "{name}");
    }

    fn rt(name: &str, args: &[&str]) -> Option<String> {
        resolve_call(name, &strings(args)).map(|r| r.return_type.into_owned())
    }

    #[test]
    fn resolve_call_delegates_every_member() {
        assert_eq!(
            rt("collections.get", &["List OF Integer", "Integer"]),
            Some("Integer".to_string())
        );
        assert_eq!(
            rt(
                "collections.getOr",
                &["List OF Integer", "Integer", "Integer"]
            ),
            Some("Integer".to_string())
        );
        assert_eq!(
            rt(
                "collections.set",
                &["List OF Integer", "Integer", "Integer"]
            ),
            Some("List OF Integer".to_string())
        );
        assert_eq!(
            rt("collections.append", &["List OF Integer", "Integer"]),
            Some("List OF Integer".to_string())
        );
        assert_eq!(
            rt("collections.prepend", &["List OF Integer", "Integer"]),
            Some("List OF Integer".to_string())
        );
        assert_eq!(
            rt(
                "collections.insert",
                &["List OF Integer", "Integer", "Integer"]
            ),
            Some("List OF Integer".to_string())
        );
        assert_eq!(
            rt("collections.removeAt", &["List OF Integer", "Integer"]),
            Some("List OF Integer".to_string())
        );
        assert_eq!(
            rt(
                "collections.removeKey",
                &["Map OF String TO Integer", "String"]
            ),
            Some("Map OF String TO Integer".to_string())
        );
        assert_eq!(
            rt("collections.keys", &["Map OF String TO Integer"]),
            Some("List OF String".to_string())
        );
        assert_eq!(
            rt("collections.values", &["Map OF String TO Integer"]),
            Some("List OF Integer".to_string())
        );
        assert_eq!(
            rt(
                "collections.hasKey",
                &["Map OF String TO Integer", "String"]
            ),
            Some("Boolean".to_string())
        );
        assert_eq!(
            rt("collections.contains", &["List OF Integer", "Integer"]),
            Some("Boolean".to_string())
        );
        assert_eq!(
            rt(
                "collections.forEach",
                &["List OF Integer", "FUNC(Integer) AS Nothing"]
            ),
            Some("Nothing".to_string())
        );
        assert_eq!(
            rt(
                "collections.transform",
                &["List OF Integer", "FUNC(Integer) AS String"]
            ),
            Some("List OF String".to_string())
        );
        assert_eq!(
            rt(
                "collections.filter",
                &["List OF Integer", "FUNC(Integer) AS Boolean"]
            ),
            Some("List OF Integer".to_string())
        );
        assert_eq!(
            rt(
                "collections.reduce",
                &[
                    "List OF Integer",
                    "String",
                    "FUNC(String, Integer) AS String"
                ]
            ),
            Some("String".to_string())
        );
        assert_eq!(
            rt("collections.sum", &["List OF Integer"]),
            Some("Integer".to_string())
        );
        assert_eq!(
            rt("collections.find", &["List OF Integer", "Integer"]),
            Some("Integer".to_string())
        );
        assert_eq!(
            rt(
                "collections.mid",
                &["List OF Integer", "Integer", "Integer"]
            ),
            Some("List OF Integer".to_string())
        );
        assert_eq!(
            rt(
                "collections.replace",
                &["List OF Integer", "Integer", "Integer"]
            ),
            Some("List OF Integer".to_string())
        );
        // Non-native member and unknown name.
        assert!(resolve_call("collections.sort", &strings(&["List OF Integer"])).is_none());
        assert!(resolve_call("get", &strings(&["List OF Integer", "Integer"])).is_none());
        // Wrong types -> None.
        assert_eq!(rt("collections.get", &["List OF Integer", "String"]), None);
    }

    #[test]
    fn call_param_names_all_members() {
        for member in NATIVE_MEMBERS {
            let name = format!("collections.{member}");
            assert!(call_param_names(&name).is_some(), "{member}");
        }
        assert!(call_param_names("collections.sort").is_none());
        assert!(call_param_names("get").is_none());
    }

    #[test]
    fn call_return_type_name_delegates() {
        // Delegates to general::call_return_type_name(bare), which returns Some only
        // for the conversion builtins (toInt/...) — none of which are native members,
        // so every collections member resolves to None here.
        assert_eq!(call_return_type_name("collections.find"), None);
        assert_eq!(call_return_type_name("collections.get"), None);
        assert_eq!(call_return_type_name("collections.sort"), None);
        assert_eq!(call_return_type_name("nope"), None);
    }

    #[test]
    fn expected_arguments_all_members() {
        for member in NATIVE_MEMBERS {
            let name = format!("collections.{member}");
            assert!(expected_arguments(&name).is_some(), "{member}");
        }
        assert!(expected_arguments("collections.sort").is_none());
    }

    #[test]
    fn arity_all_members() {
        assert_eq!(arity("collections.get"), Some((2, 2)));
        assert_eq!(arity("collections.getOr"), Some((3, 3)));
        assert_eq!(arity("collections.keys"), Some((1, 1)));
        assert_eq!(arity("collections.find"), Some((2, 3)));
        assert_eq!(arity("collections.set"), Some((3, 3)));
        assert_eq!(arity("collections.forEach"), Some((2, 2)));
        for member in NATIVE_MEMBERS {
            let name = format!("collections.{member}");
            assert!(arity(&name).is_some(), "{member}");
        }
        assert!(arity("collections.sort").is_none());
    }

    #[test]
    fn uses_package_and_bindings() {
        let ast = project("IMPORT collections\nSUB main\nEND SUB\n");
        assert!(uses_package(&ast));
        assert!(collections_bindings(&ast).contains_key("collections"));

        let bare = project("SUB main\nEND SUB\n");
        assert!(!uses_package(&bare));
        assert!(collections_bindings(&bare).is_empty());
    }

    #[test]
    fn source_file_parses() {
        assert!(source_file().is_ok());
    }

    #[test]
    fn augmented_project_injects_when_imported() {
        let ast = project("IMPORT collections\nSUB main\nEND SUB\n");
        let before = ast.files.len();
        let augmented = augmented_project(ast).expect("augment");
        assert_eq!(augmented.files.len(), before + 1);
    }

    #[test]
    fn augmented_project_noop_without_import() {
        let ast = project("SUB main\nEND SUB\n");
        let before = ast.files.len();
        assert_eq!(augmented_project(ast).expect("a").files.len(), before);
    }

    #[test]
    fn resolve_replace_list_arity_checks_before_indexing() {
        // bug-98: an empty or short arg slice must not panic (index OOB) before
        // the arity is verified.
        let empty: Vec<String> = Vec::new();
        assert!(resolve_replace_list(&empty).is_none());
        let one = strings(&["List OF Integer"]);
        assert!(resolve_replace_list(&one).is_none());
        let two = strings(&["List OF Integer", "Integer"]);
        assert!(resolve_replace_list(&two).is_none());
        // The valid 3-arg form still resolves.
        let three = strings(&["List OF Integer", "Integer", "Integer"]);
        let ok = resolve_replace_list(&three).map(|r| r.return_type.into_owned());
        assert_eq!(ok, Some("List OF Integer".to_string()));
    }

    #[test]
    fn resolve_find_list_cases() {
        assert_eq!(
            rc(resolve_find_list(&strings(&["List OF Integer", "Integer"]))),
            Some("Integer".to_string())
        );
        assert_eq!(
            rc(resolve_find_list(&strings(&[
                "List OF Integer",
                "Integer",
                "Integer"
            ]))),
            Some("Integer".to_string())
        );
        // sublist search (arg1 == whole list type)
        assert_eq!(
            rc(resolve_find_list(&strings(&[
                "List OF Integer",
                "List OF Integer"
            ]))),
            Some("Integer".to_string())
        );
        assert_eq!(
            rc(resolve_find_list(&strings(&["List OF Integer", "String"]))),
            None
        );
        assert_eq!(
            rc(resolve_find_list(&strings(&["Integer", "Integer"]))),
            None
        );
        assert_eq!(rc(resolve_find_list(&strings(&["List OF Integer"]))), None);
        assert_eq!(
            rc(resolve_find_list(&strings(&[
                "List OF Integer",
                "Integer",
                "String"
            ]))),
            None
        );
    }

    #[test]
    fn resolve_mid_list_cases() {
        assert_eq!(
            rc(resolve_mid_list(&strings(&[
                "List OF Integer",
                "Integer",
                "Integer"
            ]))),
            Some("List OF Integer".to_string())
        );
        assert_eq!(
            rc(resolve_mid_list(&strings(&[
                "List OF Integer",
                "Integer",
                "String"
            ]))),
            None
        );
        assert_eq!(
            rc(resolve_mid_list(&strings(&[
                "Integer", "Integer", "Integer"
            ]))),
            None
        );
    }

    #[test]
    fn resolve_replace_list_cases() {
        assert_eq!(
            rc(resolve_replace_list(&strings(&[
                "List OF Integer",
                "Integer",
                "Integer"
            ]))),
            Some("List OF Integer".to_string())
        );
        assert_eq!(
            rc(resolve_replace_list(&strings(&[
                "List OF Integer",
                "Integer",
                "String"
            ]))),
            None
        );
        assert_eq!(rc(resolve_replace_list(&strings(&["Integer"]))), None);
    }

    #[test]
    fn resolve_get_and_getor() {
        assert_eq!(
            rc(resolve_get(&strings(&["List OF Integer", "Integer"]))),
            Some("Integer".to_string())
        );
        assert_eq!(
            rc(resolve_get(&strings(&["List OF Integer", "String"]))),
            None
        );
        assert_eq!(
            rc(resolve_get(&strings(&[
                "Map OF String TO Integer",
                "String"
            ]))),
            Some("Integer".to_string())
        );
        assert_eq!(
            rc(resolve_get(&strings(&[
                "Map OF String TO Integer",
                "Integer"
            ]))),
            None
        );
        assert_eq!(rc(resolve_get(&strings(&["Integer", "Integer"]))), None);
        assert_eq!(rc(resolve_get(&strings(&["List OF Integer"]))), None);

        assert_eq!(
            rc(resolve_get_or(&strings(&[
                "List OF Integer",
                "Integer",
                "Integer"
            ]))),
            Some("Integer".to_string())
        );
        assert_eq!(
            rc(resolve_get_or(&strings(&[
                "List OF Integer",
                "Integer",
                "String"
            ]))),
            None
        );
        assert_eq!(
            rc(resolve_get_or(&strings(&[
                "Map OF String TO Integer",
                "String",
                "Integer"
            ]))),
            Some("Integer".to_string())
        );
        assert_eq!(
            rc(resolve_get_or(&strings(&[
                "Map OF String TO Integer",
                "String",
                "String"
            ]))),
            None
        );
        assert_eq!(rc(resolve_get_or(&strings(&["List OF Integer"]))), None);
    }

    #[test]
    fn resolve_set_cases() {
        assert_eq!(
            rc(resolve_set(&strings(&[
                "List OF Integer",
                "Integer",
                "Integer"
            ]))),
            Some("List OF Integer".to_string())
        );
        assert_eq!(
            rc(resolve_set(&strings(&[
                "List OF Integer",
                "String",
                "Integer"
            ]))),
            None
        );
        assert_eq!(
            rc(resolve_set(&strings(&[
                "Map OF String TO Integer",
                "String",
                "Integer"
            ]))),
            Some("Map OF String TO Integer".to_string())
        );
        assert_eq!(
            rc(resolve_set(&strings(&[
                "Map OF String TO Integer",
                "Integer",
                "Integer"
            ]))),
            None
        );
        assert_eq!(rc(resolve_set(&strings(&["Integer", "a", "b"]))), None);
        assert_eq!(rc(resolve_set(&strings(&["List OF Integer"]))), None);
    }

    #[test]
    fn resolve_append_prepend_insert() {
        assert_eq!(
            rc(resolve_append(&strings(&["List OF Integer", "Integer"]))),
            Some("List OF Integer".to_string())
        );
        assert_eq!(
            rc(resolve_append(&strings(&[
                "List OF Integer",
                "List OF Integer"
            ]))),
            Some("List OF Integer".to_string())
        );
        assert_eq!(
            rc(resolve_append(&strings(&["List OF Integer", "String"]))),
            None
        );
        assert_eq!(rc(resolve_append(&strings(&["Integer", "Integer"]))), None);
        assert_eq!(rc(resolve_append(&strings(&["List OF Integer"]))), None);

        assert_eq!(
            rc(resolve_prepend(&strings(&["List OF Integer", "Integer"]))),
            Some("List OF Integer".to_string())
        );
        assert_eq!(
            rc(resolve_prepend(&strings(&["List OF Integer", "String"]))),
            None
        );
        assert_eq!(rc(resolve_prepend(&strings(&["Integer", "Integer"]))), None);
        assert_eq!(rc(resolve_prepend(&strings(&["List OF Integer"]))), None);

        assert_eq!(
            rc(resolve_insert(&strings(&[
                "List OF Integer",
                "Integer",
                "Integer"
            ]))),
            Some("List OF Integer".to_string())
        );
        assert_eq!(
            rc(resolve_insert(&strings(&[
                "List OF Integer",
                "String",
                "Integer"
            ]))),
            None
        );
        assert_eq!(rc(resolve_insert(&strings(&["Integer", "a", "b"]))), None);
        assert_eq!(rc(resolve_insert(&strings(&["List OF Integer"]))), None);
    }

    #[test]
    fn resolve_remove_at_and_key() {
        assert_eq!(
            rc(resolve_remove_at(&strings(&["List OF Integer", "Integer"]))),
            Some("List OF Integer".to_string())
        );
        assert_eq!(
            rc(resolve_remove_at(&strings(&["List OF Integer", "String"]))),
            None
        );
        assert_eq!(
            rc(resolve_remove_at(&strings(&["Integer", "Integer"]))),
            None
        );

        assert_eq!(
            rc(resolve_remove_key(&strings(&[
                "Map OF String TO Integer",
                "String"
            ]))),
            Some("Map OF String TO Integer".to_string())
        );
        assert_eq!(
            rc(resolve_remove_key(&strings(&[
                "Map OF String TO Integer",
                "Integer"
            ]))),
            None
        );
        assert_eq!(rc(resolve_remove_key(&strings(&["Integer", "a"]))), None);
        assert_eq!(
            rc(resolve_remove_key(&strings(&["Map OF String TO Integer"]))),
            None
        );
    }

    #[test]
    fn resolve_keys_values() {
        assert_eq!(
            rc(resolve_keys(&strings(&["Map OF String TO Integer"]))),
            Some("List OF String".to_string())
        );
        assert_eq!(rc(resolve_keys(&strings(&["Integer"]))), None);
        assert_eq!(
            rc(resolve_keys(&strings(&["Map OF String TO Integer", "x"]))),
            None
        );
        assert_eq!(
            rc(resolve_values(&strings(&["Map OF String TO Integer"]))),
            Some("List OF Integer".to_string())
        );
        assert_eq!(rc(resolve_values(&strings(&["Integer"]))), None);
        assert_eq!(
            rc(resolve_values(&strings(&["Map OF String TO Integer", "x"]))),
            None
        );
    }

    #[test]
    fn resolve_has_key_contains() {
        assert_eq!(
            rc(resolve_has_key(&strings(&[
                "Map OF String TO Integer",
                "String"
            ]))),
            Some("Boolean".to_string())
        );
        assert_eq!(
            rc(resolve_has_key(&strings(&[
                "Map OF String TO Integer",
                "Integer"
            ]))),
            None
        );
        assert_eq!(rc(resolve_has_key(&strings(&["Integer", "a"]))), None);
        assert_eq!(
            rc(resolve_has_key(&strings(&["Map OF String TO Integer"]))),
            None
        );

        assert_eq!(
            rc(resolve_contains(&strings(&["List OF Integer", "Integer"]))),
            Some("Boolean".to_string())
        );
        assert_eq!(
            rc(resolve_contains(&strings(&["List OF Integer", "String"]))),
            None
        );
        assert_eq!(
            rc(resolve_contains(&strings(&["Integer", "Integer"]))),
            None
        );
        assert_eq!(rc(resolve_contains(&strings(&["List OF Integer"]))), None);
    }

    #[test]
    fn resolve_sum_cases() {
        assert_eq!(
            rc(resolve_sum(&strings(&["List OF Integer"]))),
            Some("Integer".to_string())
        );
        assert_eq!(
            rc(resolve_sum(&strings(&["List OF Float"]))),
            Some("Float".to_string())
        );
        assert_eq!(
            rc(resolve_sum(&strings(&["List OF Fixed"]))),
            Some("Fixed".to_string())
        );
        assert_eq!(rc(resolve_sum(&strings(&["List OF String"]))), None);
        assert_eq!(rc(resolve_sum(&strings(&["List OF Integer", "x"]))), None);
    }

    #[test]
    fn resolve_for_each_transform_filter_reduce() {
        assert_eq!(
            rc(resolve_for_each(&strings(&[
                "List OF Integer",
                "FUNC(Integer) AS Nothing"
            ]))),
            Some("Nothing".to_string())
        );
        // wrong return
        assert_eq!(
            rc(resolve_for_each(&strings(&[
                "List OF Integer",
                "FUNC(Integer) AS Boolean"
            ]))),
            None
        );
        // wrong element
        assert_eq!(
            rc(resolve_for_each(&strings(&[
                "List OF Integer",
                "FUNC(String) AS Nothing"
            ]))),
            None
        );
        assert_eq!(rc(resolve_for_each(&strings(&["Integer", "x"]))), None);
        assert_eq!(rc(resolve_for_each(&strings(&["List OF Integer"]))), None);

        assert_eq!(
            rc(resolve_transform(&strings(&[
                "List OF Integer",
                "FUNC(Integer) AS String"
            ]))),
            Some("List OF String".to_string())
        );
        assert_eq!(
            rc(resolve_transform(&strings(&[
                "List OF Integer",
                "FUNC(Integer) AS Nothing"
            ]))),
            None
        );
        assert_eq!(rc(resolve_transform(&strings(&["Integer", "x"]))), None);
        assert_eq!(rc(resolve_transform(&strings(&["List OF Integer"]))), None);

        assert_eq!(
            rc(resolve_filter(&strings(&[
                "List OF Integer",
                "FUNC(Integer) AS Boolean"
            ]))),
            Some("List OF Integer".to_string())
        );
        assert_eq!(
            rc(resolve_filter(&strings(&[
                "List OF Integer",
                "FUNC(Integer) AS Integer"
            ]))),
            None
        );
        assert_eq!(rc(resolve_filter(&strings(&["Integer", "x"]))), None);
        assert_eq!(rc(resolve_filter(&strings(&["List OF Integer"]))), None);

        assert_eq!(
            rc(resolve_reduce(&strings(&[
                "List OF Integer",
                "String",
                "FUNC(String, Integer) AS String"
            ]))),
            Some("String".to_string())
        );
        assert_eq!(
            rc(resolve_reduce(&strings(&[
                "List OF Integer",
                "String",
                "FUNC(String, Integer) AS Integer"
            ]))),
            None
        );
        assert_eq!(rc(resolve_reduce(&strings(&["Integer", "a", "b"]))), None);
        assert_eq!(
            rc(resolve_reduce(&strings(&["List OF Integer", "String"]))),
            None
        );
    }

    #[test]
    fn higher_order_resolvers_accept_function_valued_elements() {
        // `transform` over a list of two-argument function values: the mapper's
        // sole parameter *is* the element type, so the call must resolve.
        let element = "FUNC(Integer, Integer) AS Integer";
        let mapper = strings(&[
            &format!("List OF {element}"),
            &format!("FUNC({element}) AS String"),
        ]);
        let resolved =
            resolve_transform(&mapper).expect("transform over function-valued elements resolves");
        assert_eq!(resolved.return_type, "List OF String");

        let predicate = strings(&[
            &format!("List OF {element}"),
            &format!("FUNC({element}) AS Boolean"),
        ]);
        let resolved =
            resolve_filter(&predicate).expect("filter over function-valued elements resolves");
        assert_eq!(resolved.return_type, format!("List OF {element}"));

        // A mapper whose parameter is a *different* function type still fails.
        let mismatched = strings(&[
            &format!("List OF {element}"),
            "FUNC(FUNC(String) AS Integer) AS String",
        ]);
        assert!(resolve_transform(&mismatched).is_none());
    }

    // plan-72-E migration gate: prove `COLLECTIONS` reproduces the legacy answers
    // for every native member — membership, arity, and parameter names/aliases —
    // and that its `BuiltinResolver` resolves List/Map/Set/generic return types
    // identically to the legacy dispatch. `expected_arguments` and
    // `call_return_type_name` are not descriptor-derivable (custom phrasing /
    // general delegation) and are excluded. Keep until plan-72-BB.
    #[test]
    fn parity_matches_descriptor() {
        use crate::builtins::descriptor::parity;

        let calls: Vec<&str> = COLLECTIONS_FUNCTIONS.iter().map(|f| f.name).collect();
        let legacy = parity::LegacySet {
            is_call: &is_native_member_call,
            arity: &arity,
            param_names: &|name| {
                call_param_names(name).map(|rows| rows.iter().map(|row| row.to_vec()).collect())
            },
            // Resolver-backed: the harness skips return-type parity when the
            // module has a resolver, so this value is unused.
            return_type_name: &|_| None,
            // Not descriptor-derivable (custom "or"-phrased strings).
            expected_arguments: None,
            param_name_overloads: None,
            argument_types: None,
            implementation_name: None,
            default_padding: None,
            builtin_type_fields: None,
        };
        // `collections.sort` is a source generic (not a native member) and
        // `collections.nope` is unknown; both must be non-members.
        let mut probe = calls.clone();
        probe.push("collections.sort");
        probe.push("collections.nope");

        // Resolver samples covering List/Map/Set and generic resolution.
        let samples = [
            parity::ResolverSample {
                call: "collections.get",
                arg_types: &["List OF Integer", "Integer"],
                expected_return: Some("Integer"),
                expected_impl: None,
                expected_padding: None,
                expected_type: None,
                expected_overload_target: None,
            },
            parity::ResolverSample {
                call: "collections.get",
                arg_types: &["Map OF String TO Integer", "String"],
                expected_return: Some("Integer"),
                expected_impl: None,
                expected_padding: None,
                expected_type: None,
                expected_overload_target: None,
            },
            parity::ResolverSample {
                call: "collections.keys",
                arg_types: &["Map OF String TO Integer"],
                expected_return: Some("List OF String"),
                expected_impl: None,
                expected_padding: None,
                expected_type: None,
                expected_overload_target: None,
            },
            parity::ResolverSample {
                call: "collections.values",
                arg_types: &["Map OF String TO Integer"],
                expected_return: Some("List OF Integer"),
                expected_impl: None,
                expected_padding: None,
                expected_type: None,
                expected_overload_target: None,
            },
            parity::ResolverSample {
                call: "collections.append",
                arg_types: &["List OF Integer", "Integer"],
                expected_return: Some("List OF Integer"),
                expected_impl: None,
                expected_padding: None,
                expected_type: None,
                expected_overload_target: None,
            },
            parity::ResolverSample {
                call: "collections.contains",
                arg_types: &["List OF Integer", "Integer"],
                expected_return: Some("Boolean"),
                expected_impl: None,
                expected_padding: None,
                expected_type: None,
                expected_overload_target: None,
            },
            parity::ResolverSample {
                call: "collections.contains",
                arg_types: &["Set OF Integer", "Integer"],
                expected_return: Some("Boolean"),
                expected_impl: None,
                expected_padding: None,
                expected_type: None,
                expected_overload_target: None,
            },
            parity::ResolverSample {
                call: "collections.add",
                arg_types: &["Set OF Integer", "Integer"],
                expected_return: Some("Set OF Integer"),
                expected_impl: None,
                expected_padding: None,
                expected_type: None,
                expected_overload_target: None,
            },
            parity::ResolverSample {
                call: "collections.toList",
                arg_types: &["Set OF Integer"],
                expected_return: Some("List OF Integer"),
                expected_impl: None,
                expected_padding: None,
                expected_type: None,
                expected_overload_target: None,
            },
            parity::ResolverSample {
                call: "collections.sum",
                arg_types: &["List OF Integer"],
                expected_return: Some("Integer"),
                expected_impl: None,
                expected_padding: None,
                expected_type: None,
                expected_overload_target: None,
            },
            parity::ResolverSample {
                call: "collections.find",
                arg_types: &["List OF Integer", "Integer"],
                expected_return: Some("Integer"),
                expected_impl: None,
                expected_padding: None,
                expected_type: None,
                expected_overload_target: None,
            },
            parity::ResolverSample {
                call: "collections.mid",
                arg_types: &["List OF Integer", "Integer", "Integer"],
                expected_return: Some("List OF Integer"),
                expected_impl: None,
                expected_padding: None,
                expected_type: None,
                expected_overload_target: None,
            },
        ];
        parity::assert_parity(&COLLECTIONS, &probe, &legacy, &samples);

        // The source companion injects on import (WhenImported).
        assert_eq!(
            COLLECTIONS.source.expect("collections has a source").rule,
            crate::builtins::descriptor::InjectionRule::WhenImported
        );
    }
}
