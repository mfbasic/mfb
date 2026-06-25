use crate::ast::{AstFile, AstProject};
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
];

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
    name.strip_prefix("collections.")
        .is_some_and(is_native_member)
}

/// The bare native name for a `collections.<member>` native-member call, e.g.
/// `collections.get` -> `get`. Returns `None` for source generic functions and
/// non-`collections` names.
pub(crate) fn native_member_bare(name: &str) -> Option<&str> {
    name.strip_prefix("collections.")
        .filter(|member| is_native_member(member))
}

/// Resolves a `collections.<member>` native-member call by delegating to the
/// granular `general::resolve_*` helpers (which carry the original bare-name
/// semantics). `find`/`mid`/`replace` use the List-only overload here; their
/// String overloads live in `strings::`.
pub(crate) fn resolve_call<'a>(
    name: &str,
    arg_types: &'a [String],
) -> Option<super::general::ResolvedCall<'a>> {
    use super::general;
    match native_member_bare(name)? {
        "get" => general::resolve_get(arg_types),
        "getOr" => general::resolve_get_or(arg_types),
        "set" => general::resolve_set(arg_types),
        "append" => general::resolve_append(arg_types),
        "prepend" => general::resolve_prepend(arg_types),
        "insert" => general::resolve_insert(arg_types),
        "removeAt" => general::resolve_remove_at(arg_types),
        "removeKey" => general::resolve_remove_key(arg_types),
        "keys" => general::resolve_keys(arg_types),
        "values" => general::resolve_values(arg_types),
        "hasKey" => general::resolve_has_key(arg_types),
        "contains" => general::resolve_contains(arg_types),
        "forEach" => general::resolve_for_each(arg_types),
        "transform" => general::resolve_transform(arg_types),
        "filter" => general::resolve_filter(arg_types),
        "reduce" => general::resolve_reduce(arg_types),
        "sum" => general::resolve_sum(arg_types),
        "find" => general::resolve_find_list(arg_types),
        "mid" => general::resolve_mid_list(arg_types),
        "replace" => general::resolve_replace_list(arg_types),
        _ => None,
    }
}

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
        "replace" => Some(&[&["value", "list"], &["old", "needle"], &["new", "replacement"]]),
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
        _ => None,
    }
}

pub(crate) fn arity(name: &str) -> Option<(usize, usize)> {
    match native_member_bare(name)? {
        "removeAt" | "removeKey" | "hasKey" | "contains" | "append" | "prepend" | "get"
        | "forEach" | "transform" | "filter" => Some((2, 2)),
        "getOr" | "set" | "insert" | "reduce" | "mid" | "replace" => Some((3, 3)),
        "keys" | "values" | "sum" => Some((1, 1)),
        "find" => Some((2, 3)),
        _ => None,
    }
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
