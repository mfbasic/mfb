// --- codegen tier imports (migration) ---
use crate::ast::AstProject;
use crate::codegen::registry::{Registry, RegistryPackage};
use std::path::Path;
pub(crate) mod common;
mod func_add;
mod func_all;
mod func_any;
mod func_append;
mod func_chunks;
mod func_contains;
mod func_difference;
mod func_distinct;
mod func_drop;
mod func_filter;
mod func_find;
mod func_find_index;
mod func_find_last_index;
mod func_flatten;
mod func_for_each;
mod func_get;
mod func_get_or;
mod func_group_by;
mod func_has_key;
mod func_insert;
mod func_intersection;
mod func_is_disjoint;
mod func_is_subset;
mod func_is_superset;
mod func_keys;
mod func_map_values;
mod func_merge;
mod func_mid;
mod func_partition;
mod func_prepend;
mod func_reduce;
mod func_reduce_right;
mod func_remove;
mod func_remove_at;
mod func_remove_key;
mod func_replace;
mod func_set;
mod func_sort;
mod func_sort_by;
mod func_sum;
mod func_symmetric_difference;
mod func_take;
mod func_to_list;
mod func_to_set;
mod func_union;
mod func_window;
mod func_zip;
// `pub(crate)`: source-generic fast paths in `src/target` (sortBy, mapValues,
// groupBy) reuse `lower_transform` directly until they too migrate (plan-96).
pub(crate) mod func_transform;
mod func_values;

/// Path of the compiler-owned `collections` package source injected into every
/// project that imports it. This is the `AstFile.path` (see `augmented_project`), so
/// `AstProject::to_json` can filter it out of `-ast` output.
pub(crate) const SOURCE_PATH: &str = "builtins/collections.mfb";

/// The public `collections::` function names (without the `collections.`
/// qualifier). The implementations live in `package.mfb` as generic
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

/// One-line package intro (was `BuiltinModule::doc_intro`).
const INTRO: &str = "Sequence and map helper functions";

/// Package-overview description, from `src/docs/man/builtins/collections/package.md`
/// (its Description section, citation markers stripped).
const COLLECTIONS_DESC: &str = r#"The `collections` package provides package-qualified helpers for `List` and `Map`
values: element access and mutation (`get`, `set`, `append`, `prepend`, `insert`,
`removeAt`, `removeKey`), higher-order transforms (`transform`, `filter`,
`reduce`, `reduceRight`, `forEach`, `mapValues`), queries (`find`, `findIndex`,
`findLastIndex`, `contains`, `any`, `all`, `hasKey`, `keys`, `values`), reshaping
(`sort`, `sortBy`, `distinct`, `flatten`, `zip`, `chunks`, `window`, `partition`,
`groupBy`, `merge`), and numeric folding (`sum`). `collections` is a built-in
package: `IMPORT collections` needs no manifest dependency.

These helpers do not mutate their arguments. A function that changes a collection
returns a new value and leaves the original unchanged. List indexes are
zero-based, and access reads without copying the collection.

Element and key types follow the comparable/orderable rules: `sort` and `sortBy`
require an orderable element or key type, and `distinct` requires a comparable
element type. Map helpers operate on `Map OF K TO V` values, where the key type
`K` is the map's declared key type.

Predicates and other function arguments are passed as function values: a named
`FUNC`, a `LAMBDA`, or a general built-in predicate such as `isEven`,
`isPositive` or `isEmpty`. A built-in predicate resolves against the type
expected at that position — the element type of the list for a higher-order
call, or the declared type of a `FUNC(T) AS Boolean` binding — because a bare
name like `isPositive` is defined over `Integer`, `Float` and `Fixed` and nothing
in the reference alone chooses between them.

Some helpers introduce built-in result types: `zip` produces a `List OF Pair OF
A, B`, and `partition` produces a `Partition OF T` holding the matched and
unmatched elements. See `mfb man types pair` and `mfb man types partition`.

The List-only overloads of `find`, `mid`, and `replace` live here; their String
overloads live in `strings::`."#;

/// Register the `collections` package on the clean-room registry. Only the NATIVE
/// members are registered here (the source-generic members — `sort`, `zip`, … —
/// keep their manifest-injected MFBASIC bodies and are resolved by the
/// monomorphizer, so they are deliberately absent). No records/unions/helpers are
/// added and no `Body::Mfb` member is registered, so `get_mfb()` is empty and
/// `registry().augment_project` does NOT inject `collections` — it stays injected
/// through [`augmented_project`].
pub(crate) fn register(r: &mut Registry) {
    let mut pkg = RegistryPackage::new("collections", INTRO, COLLECTIONS_DESC);

    // The source-generic members (`sort`, `partition`, …) are not registered as
    // `RegistryFunction`s (they are monomorphized from `package.mfb`), but their
    // names are recorded as registry data so the shared pipeline recognizes a call
    // like `collections.sort` as a builtin member without a per-package branch.
    pkg.add_source_generics(FUNCTIONS);

    // The native HOF fast paths for the source-generic members. Recorded as registry
    // data (they cannot ride a `Body::Mfb` — source generics are not registered
    // functions) so the generic `registry::mfb_fast_path` answers a
    // `#collections_<member>$…` monomorph target without a per-package table.
    pkg.add_source_generic_fast_paths(&[
        ("sort", func_sort::sort_fast_path),
        ("sortBy", func_sort_by::sort_by_fast_path),
        ("mapValues", func_map_values::map_values_fast_path),
        ("groupBy", func_group_by::group_by_fast_path),
        ("chunks", func_chunks::chunks_fast_path),
        ("window", func_window::window_fast_path),
        ("merge", func_merge::merge_fast_path),
        ("partition", func_partition::partition_fast_path),
        ("flatten", func_flatten::flatten_fast_path),
        (
            "findLastIndex",
            func_find_last_index::find_last_index_fast_path,
        ),
        ("zip", func_zip::zip_fast_path),
    ]);

    func_get::register(&mut pkg);
    func_get_or::register(&mut pkg);
    func_set::register(&mut pkg);
    func_append::register(&mut pkg);
    func_prepend::register(&mut pkg);
    func_insert::register(&mut pkg);
    func_remove_at::register(&mut pkg);
    func_remove_key::register(&mut pkg);
    func_keys::register(&mut pkg);
    func_values::register(&mut pkg);
    func_has_key::register(&mut pkg);
    func_contains::register(&mut pkg);
    func_for_each::register(&mut pkg);
    func_transform::register(&mut pkg);
    func_filter::register(&mut pkg);
    func_reduce::register(&mut pkg);
    func_reduce_right::register(&mut pkg);
    func_sum::register(&mut pkg);
    func_find::register(&mut pkg);
    func_mid::register(&mut pkg);
    func_replace::register(&mut pkg);
    func_add::register(&mut pkg);
    func_remove::register(&mut pkg);
    func_to_list::register(&mut pkg);

    r.add_package(pkg);
}

/// Injects the `collections` package source into `ast` when the project imports
/// it. The source is appended last (so the monomorphizer's first-file emission
/// target is unchanged) and is filtered out of `-ast` output by its sentinel
/// path. Call rewriting (`collections.sort` -> `__collections_sort`) happens in
/// the monomorphizer. `package.mfb` is self-contained (all source-generic bodies
/// inlined at their original marker positions), so it is parsed directly.
///
/// `collections` is injected by this dedicated late pass (not the generic
/// `registry::augment_project`) because its members are source generics with no
/// modeled registry bodies (`get_mfb` is empty), so the migration keeps this hook.
/// #[deprecated(note = "migrate registry().augment_project once source generics are modeled")]
pub(crate) fn augmented_project(ast: AstProject) -> Result<AstProject, ()> {
    let imported = ast.files.iter().any(|file| {
        file.imports
            .iter()
            .any(|i| i.package_name() == "collections")
    });
    if !imported {
        return Ok(ast);
    }
    let file = crate::ast::parse_source_internal(
        Path::new(SOURCE_PATH),
        SOURCE_PATH,
        include_str!("package.mfb"),
    )?;
    let mut augmented = ast;
    augmented.files.push(file);
    Ok(augmented)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::registry::{self, registry};

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

    // The `is_collections_call` / `is_source_generic_member` shape, exercised through
    // the generic registry accessors the pipeline now routes through.
    fn is_collections_call(name: &str) -> bool {
        registry().owning_package(name) == Some("collections")
            || registry::is_source_generic_member(name)
    }

    #[test]
    fn function_and_native_membership() {
        // Source generics answer through `is_source_generic_member`; native members
        // through `owning_package` (they are the registered functions).
        assert!(registry::is_source_generic_member("collections.sort"));
        assert!(registry::is_source_generic_member("collections.partition"));
        assert!(!registry::is_source_generic_member("collections.get"));
        assert!(!registry::is_source_generic_member("collections.nope"));

        assert_eq!(
            registry().owning_package("collections.get"),
            Some("collections")
        );
        assert_eq!(
            registry().owning_package("collections.replace"),
            Some("collections")
        );
        assert!(registry().owning_package("collections.sort").is_none());
        assert!(registry().owning_package("collections.nope").is_none());
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
        // `is_native_member_call` -> `owning_package == Some("collections")`;
        // the bare-name dequalification -> `crate::builtins::native_builtin_target`.
        assert_eq!(
            registry().owning_package("collections.get"),
            Some("collections")
        );
        assert!(registry().owning_package("collections.sort").is_none());
        assert!(registry().owning_package("get").is_none());
        assert_eq!(
            crate::builtins::native_builtin_target("collections.get"),
            Some("get")
        );
        assert_eq!(
            crate::builtins::native_builtin_target("collections.sort"),
            None
        );
        assert_eq!(crate::builtins::native_builtin_target("get"), None);
    }

    #[test]
    fn call_param_names_all_members() {
        // Every native member's keyword table is served by the generic registry
        // union of its overloads' parameters. The registered functions ARE the native
        // members (source generics are injected source, not registered).
        let pkg = registry().resolve_package("collections").unwrap();
        for function in pkg.functions() {
            let name = format!("collections.{}", function.name);
            assert!(
                registry::call_param_names(&name).is_some(),
                "{}",
                function.name
            );
        }
        assert!(registry::call_param_names("collections.sort").is_none());
        assert!(registry::call_param_names("get").is_none());
        // A concrete union: `get`'s List and Map overloads agree on position 0.
        assert_eq!(
            registry::call_param_names("collections.get"),
            Some(vec![vec!["value", "collection"], vec!["index", "key"]])
        );
    }

    #[test]
    fn expected_arguments_all_members() {
        // Every native member carries a bespoke `"or"`/generic phrasing on its
        // descriptor, served by the generic `registry::expected_arguments`.
        let pkg = registry().resolve_package("collections").unwrap();
        for function in pkg.functions() {
            let name = format!("collections.{}", function.name);
            assert!(
                registry::expected_arguments(&name).is_some(),
                "{}",
                function.name
            );
        }
        assert!(registry::expected_arguments("collections.sort").is_none());
        assert_eq!(
            registry::expected_arguments("collections.get"),
            Some("List OF T, Integer or Map OF K TO V, K")
        );
    }

    #[test]
    fn import_detection_via_registry() {
        let pkg = registry().resolve_package("collections").unwrap();
        let ast = project("IMPORT collections\nSUB main\nEND SUB\n");
        assert!(pkg.is_imported_by(&ast));

        let bare = project("SUB main\nEND SUB\n");
        assert!(!pkg.is_imported_by(&bare));
    }

    #[test]
    fn mfb_fast_path_routes_source_generics() {
        // The generic registry accessor answers a `#collections_<member>$…` target.
        assert!(registry::mfb_fast_path("#collections_sort$Integer").is_some());
        assert!(registry::mfb_fast_path("#collections_zip$Integer$String").is_some());
        assert!(registry::mfb_fast_path("#collections_get$Integer").is_none());
        assert!(registry::mfb_fast_path("#collections_nope$Integer").is_none());
    }

    #[test]
    fn source_file_parses() {
        assert!(crate::ast::parse_source_internal(
            Path::new(SOURCE_PATH),
            SOURCE_PATH,
            include_str!("package.mfb")
        )
        .is_ok());
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
    fn collections_registered_on_the_clean_room_registry() {
        let pkg = registry()
            .resolve_package("collections")
            .expect("collections package");
        // Exactly the 24 native members (source generics are not registered here).
        assert_eq!(pkg.functions().len(), 24);
        assert!(registry().is_member("collections.get"));
        assert!(!registry().is_member("collections.sort")); // source generic
        assert!(!registry().is_member("collections.nope"));
    }

    fn rt(name: &str, args: &[&str]) -> Option<String> {
        registry::resolve_call(name, &strings(args), false)
    }

    #[test]
    fn generic_dispatch_resolves_native_members() {
        assert_eq!(
            rt("collections.get", &["List OF Integer", "Integer"]),
            Some("Integer".to_string())
        );
        assert_eq!(
            rt("collections.get", &["Map OF String TO Integer", "String"]),
            Some("Integer".to_string())
        );
        assert_eq!(
            rt("collections.keys", &["Map OF String TO Integer"]),
            Some("List OF String".to_string())
        );
        assert_eq!(
            rt("collections.append", &["List OF Integer", "Integer"]),
            Some("List OF Integer".to_string())
        );
        // RES marker preserved + STATE-agnostic (bug-427).
        assert_eq!(
            rt(
                "collections.append",
                &["List OF RES fs.File STATE Cursor", "fs.File"]
            ),
            Some("List OF RES fs.File STATE Cursor".to_string())
        );
        assert_eq!(
            rt("collections.toList", &["Set OF Integer"]),
            Some("List OF Integer".to_string())
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
            rt(
                "collections.filter",
                &["List OF Integer", "FUNC(Integer) AS Boolean"]
            ),
            Some("List OF Integer".to_string())
        );
        assert_eq!(
            rt("collections.sum", &["List OF Integer"]),
            Some("Integer".to_string())
        );
        // Wrong-type rejection.
        assert_eq!(rt("collections.get", &["List OF Integer", "String"]), None);
    }
}
