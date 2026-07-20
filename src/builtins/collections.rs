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
        "replace" => Some(&[
            &["value", "list"],
            &["old", "needle"],
            &["new", "replacement"],
        ]),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
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
}
