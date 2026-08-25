// --- codegen tier imports (migration) ---
use crate::ast::AstProject;
use crate::codegen::registry::{Registry, RegistryPackage};
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
// Shared, collections-package-only codegen seams (the plan-96 "A1" tier): target-
// generic `impl CodeBuilder` primitives whose only callers are collection-domain
// lowerings (the `func_*.rs` entries and their sibling collection code in
// `src/target`). They stay `impl CodeBuilder` methods — only the defining module
// moved — so call sites (`builder.lower_list_get(..)`) are unchanged. `gen_memory`
// and `gen_mutate` also use the shared memory/error layer and carry a wider
// `codegen -> target` import surface until `src/codegen/memory` exists.
mod gen_flow;
mod gen_list;
mod gen_map;
mod gen_memory;
mod gen_mutate;
mod gen_set;
mod gen_slice;

mod helper_slice;

/// Path of the compiler-owned `collections` package source injected into every
/// project that imports it. This is the `AstFile.path` (see `augmented_project`), so
/// `AstProject::to_json` can filter it out of `-ast` output.
pub(crate) const SOURCE_PATH: &str = "builtins/collections.mfb";

// The source-generic public members (`sort`, `sortBy`, …, the set algebra) are
// registered `RegistryFunction`s like every other member — see the `func_*.rs`
// registrations below; each carries its generic `__collections_<name>` body as
// `Body::Mfb`, rewritten and instantiated during monomorphization.
//
// `toMap`, `zipWith`, and `filterEntries` from §6.4 are not yet exported: they
// depend on runtime capabilities MFBASIC does not have today — storing the
// compiler-owned `MapEntry` record inside a `List` (toMap/filterEntries) and
// applying a two-argument function value element-wise (zipWith). They are
// deferred until that infrastructure lands; see plan-01-functions.md §6.4.

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

/// Register the `collections` package on the clean-room registry. Every public
/// member is a registered function: the NATIVE members lower at the call site
/// (`Body::abi_inline`), and the source-generic members (`sort`, `zip`, …) carry
/// their generic MFBASIC bodies as `Body::Mfb` — rendered into the assembled
/// source by `get_mfb` and instantiated by the monomorphizer. The generic
/// `registry().augment_project` deliberately SKIPS `collections` (see
/// `synthetic_files`): the source must be present at parse time — before
/// monomorphization instantiates the generics — so it is injected by the
/// [`augmented_project`] pass `parse_project` runs.
pub(crate) fn register(r: &mut Registry) {
    let mut pkg = RegistryPackage::new("collections", INTRO, COLLECTIONS_DESC);

    // The injected source's single IMPORT line: the bodies qualify their native
    // member calls (`collections::get`/`set`/`append`/…), so the file imports the
    // package itself. `len(...)` stays a global builtin; the internal
    // `__collections_slice` helper is a plain top-level function called
    // unqualified (plan-01-functions.md §5).
    pkg.add_imports(vec!["collections"]);

    // The private `__collections_slice` helper the take/drop/chunks/window bodies
    // call (`add_helper` — helper_* files are for PRIVATE helpers only; it renders
    // in the helper section of the assembled source, before the member bodies).
    helper_slice::register(&mut pkg);

    // The source-generic PUBLIC members (plan-01-functions.md §6.4): each is a
    // registered `RegistryFunction` in its `func_*.rs` (the csv/json shape) whose
    // `Body::Mfb` carries the generic `__collections_*` MFBASIC body — rendered
    // into the assembled source in this order and instantiated by the
    // monomorphizer per call site (rewritten in `monomorph::lower`, not IR lower).
    func_sort::register(&mut pkg);
    func_sort_by::register(&mut pkg);
    func_take::register(&mut pkg);
    func_drop::register(&mut pkg);
    func_any::register(&mut pkg);
    func_all::register(&mut pkg);
    func_find_index::register(&mut pkg);
    func_find_last_index::register(&mut pkg);
    func_group_by::register(&mut pkg);
    func_map_values::register(&mut pkg);
    func_flatten::register(&mut pkg);
    func_zip::register(&mut pkg);
    func_chunks::register(&mut pkg);
    func_window::register(&mut pkg);
    func_distinct::register(&mut pkg);
    func_merge::register(&mut pkg);
    func_partition::register(&mut pkg);
    // Set algebra (plan-63-C): eight source generics over B's native Set
    // primitives (`add`/`contains`/`toList`/`FOR EACH`). Pure: each returns a new
    // value and mutates no argument. Instantiated only for comparable `T`, since
    // `Set OF T` already requires it.
    func_to_set::register(&mut pkg);
    func_union::register(&mut pkg);
    func_intersection::register(&mut pkg);
    func_difference::register(&mut pkg);
    func_symmetric_difference::register(&mut pkg);
    func_is_subset::register(&mut pkg);
    func_is_superset::register(&mut pkg);
    func_is_disjoint::register(&mut pkg);

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
/// the monomorphizer.
///
/// The injected source is the generic [`RegistryPackage::get_mfb`] assembly
/// (imports → the `helper_*.rs` generic bodies), identical to what the generic
/// `registry::augment_project` would produce; only the injection *position* is
/// bespoke — `parse_project` runs this pass at parse time because the
/// monomorphizer must see the generic bodies to instantiate them, long before the
/// ir-lower augmentation chain. The generic pass therefore skips `collections`
/// (see `Registry::augment_project`), which also prevents a double injection.
pub(crate) fn augmented_project(ast: AstProject) -> Result<AstProject, ()> {
    crate::codegen::registry::inject_late_pass(&ast, "collections", SOURCE_PATH, SOURCE_PATH)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::registry::{self, registry};
    use std::path::Path;

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

    // The old `is_collections_call` shape: every member — native and source-generic
    // — is a registered function now, so `owning_package` alone answers it.
    fn is_collections_call(name: &str) -> bool {
        registry().owning_package(name) == Some("collections")
    }

    #[test]
    fn function_and_native_membership() {
        // Native members AND source-generic members are registered functions.
        assert_eq!(
            registry().owning_package("collections.get"),
            Some("collections")
        );
        assert_eq!(
            registry().owning_package("collections.replace"),
            Some("collections")
        );
        assert_eq!(
            registry().owning_package("collections.sort"),
            Some("collections")
        );
        assert_eq!(
            registry().owning_package("collections.partition"),
            Some("collections")
        );
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
        // The bare-name dequalification (`native_builtin_target`) covers only the
        // `Body::AbiInline` native members: a `Body::Mfb` source generic (`sort`)
        // is registered but does NOT dequalify to a bare native name.
        assert_eq!(
            registry().owning_package("collections.get"),
            Some("collections")
        );
        assert!(registry().owning_package("get").is_none());
        assert_eq!(
            crate::codegen::builtins::native_builtin_target("collections.get"),
            Some("get")
        );
        assert_eq!(
            crate::codegen::builtins::native_builtin_target("collections.sort"),
            None
        );
        assert_eq!(crate::codegen::builtins::native_builtin_target("get"), None);
    }

    #[test]
    fn call_param_names_all_members() {
        // Every member's keyword table — native and source-generic alike — is
        // served by the generic registry union of its overloads' parameters.
        let pkg = registry().resolve_package("collections").unwrap();
        for function in pkg.functions() {
            let name = format!("collections.{}", function.name);
            assert!(
                registry::call_param_names(&name).is_some(),
                "{}",
                function.name
            );
        }
        assert_eq!(
            registry::call_param_names("collections.sort"),
            Some(vec![vec!["value"]])
        );
        assert!(registry::call_param_names("get").is_none());
        // A concrete union: `get`'s List and Map overloads agree on position 0.
        assert_eq!(
            registry::call_param_names("collections.get"),
            Some(vec![vec!["value", "collection"], vec!["index", "key"]])
        );
    }

    #[test]
    fn expected_arguments_all_members() {
        // Every member carries a bespoke `"or"`/generic phrasing on its
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
        assert_eq!(
            registry::expected_arguments("collections.sort"),
            Some("List OF T")
        );
        assert_eq!(
            registry::expected_arguments("collections.get"),
            Some("List OF T, Integer or Map OF K TO V, K")
        );
    }

    #[test]
    fn import_detection_via_registry() {
        let pkg = registry().resolve_package("collections").unwrap();
        let ast = project("IMPORT collections\nSUB main\nEND SUB\n");
        assert!(pkg.is_imported_by(&crate::codegen::registry::ProjectView::of_ast(&ast)));

        let bare = project("SUB main\nEND SUB\n");
        assert!(!pkg.is_imported_by(&crate::codegen::registry::ProjectView::of_ast(&bare)));
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
    fn reassembled_source_parses() {
        let source = registry()
            .resolve_package("collections")
            .expect("collections")
            .get_mfb();
        assert!(source.contains("FUNC __collections_sort OF T"));
        assert!(source.contains("FUNC __collections_isDisjoint OF T"));
        assert!(
            crate::ast::parse_source_internal(Path::new(SOURCE_PATH), SOURCE_PATH, &source).is_ok()
        );
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
        // The 24 native members + the 25 source-generic `Body::Mfb` members.
        assert_eq!(pkg.functions().len(), 49);
        assert!(registry().is_member("collections.get"));
        assert!(registry().is_member("collections.sort")); // source generic, registered
        assert!(!registry().is_member("collections.nope"));
        // A source-generic member rewrites to its generic body's internal name
        // (the monomorph rewrite source of truth), resolves a generic call, and
        // carries its arity.
        assert_eq!(
            registry::rewrite_target("collections.sort", &strings(&["List OF Integer"])),
            Some("__collections_sort")
        );
        assert_eq!(registry().arity("collections.sort"), Some((1, 1)));
        assert_eq!(registry().arity("collections.findIndex"), Some((2, 3)));
        assert_eq!(
            registry::resolve_call(
                "collections.union",
                &strings(&["Set OF Integer", "Set OF Integer"]),
                true
            ),
            Some("Set OF Integer".to_string())
        );
        assert_eq!(
            registry::resolve_call(
                "collections.zip",
                &strings(&["List OF Integer", "List OF String"]),
                true
            ),
            Some("List OF Pair OF Integer, String".to_string())
        );
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
