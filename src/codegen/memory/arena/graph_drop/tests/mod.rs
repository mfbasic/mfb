//! The drop walker's tests (moved out of `graph_drop.rs`, Corrections C15 of
//! `planning/tests.md`: a suite inside the file is measured as part of it).

use super::*;
use crate::codegen::engine::tests::test_support::TestPlatform;
use crate::codegen::memory::arena::graph_copy::tests::{
    calls_and_pushes, field, harness, json_like_types, model, pushed_kinds, record, union,
};

mod hook;
mod shapes;

/// For kind `name`: the kinds the drop walker's body pushes, sorted. Also asserts the
/// body calls neither a per-type copy nor either walker — no native recursion.
fn drop_pushes(model: &TypeModel, name: &str) -> Vec<String> {
    let kinds: Vec<String> = recursive_transfer_types(model).into_iter().collect();
    let platform = TestPlatform;
    let harness = harness(model);
    let mut walker = harness.builder("drop", &platform);
    let stack_slot = walker.allocate_stack_object("stack", 8);
    walker.graph_copy_walker = Some(GraphCopyWalker::new(&kinds, stack_slot));
    let pointer_slot = walker.allocate_stack_object("pointer", 8);
    walker
        .emit_graph_drop_block(&ParameterType::declared(name), pointer_slot)
        .unwrap_or_else(|error| panic!("{name}: the drop body emits: {error}"));
    let per_type: Vec<String> = kinds
        .iter()
        .map(|kind| thread_copy_symbol(&ParameterType::declared(kind)))
        .collect();
    let calls: Vec<&String> = walker
        .relocations
        .iter()
        .map(|relocation| &relocation.to)
        .filter(|to| {
            per_type.contains(to)
                || to.as_str() == GRAPH_DROP_SYMBOL
                || to.as_str() == crate::codegen::memory::arena::graph_copy::GRAPH_COPY_SYMBOL
        })
        .collect();
    assert!(
        calls.is_empty(),
        "{name}: the drop body calls back into a copy or a walker: {calls:?}"
    );
    pushed_kinds(&walker, &kinds)
}

/// Every resource-free kind of `model`: its drop pushes exactly the edges its copy
/// pushes, and only resource-free kinds. Returns the number of edges seen.
fn assert_drop_edges_match(label: &str, model: &TypeModel) -> usize {
    let kinds = recursive_transfer_types(model);
    assert!(
        !kinds.is_empty(),
        "{label}: the model has no recursive type"
    );
    let mut edges = 0;
    for kind in &kinds {
        if type_contains_resource(model, &ParameterType::declared(kind)) {
            continue;
        }
        let (_, copy_pushes) = calls_and_pushes(model, kind);
        let drop_pushes = drop_pushes(model, kind);
        assert_eq!(
            drop_pushes, copy_pushes,
            "{label}: kind {kind}: the drop must push exactly the edges the copy pushes"
        );
        for pushed in &drop_pushes {
            assert!(
                !type_contains_resource(model, &ParameterType::declared(pushed)),
                "{label}: kind {kind} pushes resource-bearing kind {pushed}, which has no arm"
            );
        }
        edges += drop_pushes.len();
    }
    edges
}

/// The drop walker visits exactly the edges the copy walker visits, per kind, over the
/// hand-built models of `graph_copy_edges_match_the_copy_calls` and a recursive user
/// union.
#[test]
fn graph_drop_edges_match_the_copy_edges() {
    let node = model(vec![record(
        "Node",
        vec![field("kids", "List OF Node"), field("tag", "Integer")],
    )]);
    let json = model(json_like_types());
    let mut bridged_types = json_like_types();
    bridged_types.push(record(
        "Node2",
        vec![
            field("kids", "List OF Node2"),
            field("extra", "Map OF Integer TO J"),
        ],
    ));
    let bridged = model(bridged_types);
    let leaf = || vec![field("v", "Integer")];
    let pair = || vec![field("left", "Tree"), field("right", "Tree")];
    let tree = model(vec![
        record("Leaf", leaf()),
        record("Pair", pair()),
        union("Tree", vec![("Leaf", leaf()), ("Pair", pair())]),
    ]);
    for (label, model) in [
        ("TYPE Node (kids AS List OF Node)", &node),
        ("a Json-shaped recursive union", &json),
        ("a cycle member reaching a second cycle", &bridged),
        ("UNION Tree (Pair holds two Tree fields)", &tree),
    ] {
        let edges = assert_drop_edges_match(label, model);
        assert!(
            edges > 0,
            "{label}: no edges at all — not looking at a cycle"
        );
    }
}

/// The same table over the builtin recursive types, from the models of the
/// recursive-value bench programs that use them: `json::Json`, and the regex parser's
/// `__regex_Node` with the `Concat` and `Repeat` records that hold it. (The matcher's
/// `__regex_Cont` and `__regex_Choices` were here until plan-134-I deleted them.)
#[test]
fn graph_drop_edges_match_the_copy_edges_for_the_builtin_types() {
    use crate::testutil::{nir_for_src, CodeTarget};
    for (label, source, expected) in [
        (
            "json_repeat",
            include_str!(
                "../../../../../../tools/recursive-value-bench/programs/json_repeat/src/main.mfb"
            ),
            &["Json"][..],
        ),
        (
            "regex_repeat",
            include_str!(
                "../../../../../../tools/recursive-value-bench/programs/regex_repeat/src/main.mfb"
            ),
            // The package's private types render with its internal `#` prefix.
            &["#regex_Node", "#regex_Concat", "#regex_Repeat"][..],
        ),
    ] {
        let module = nir_for_src(
            source,
            CodeTarget::MacosAarch64,
            crate::target::NativeBuildMode::Console,
        )
        .unwrap_or_else(|error| panic!("{label}: the program lowers to NIR: {error}"));
        let model = TypeModel::from_module(&module).expect("the program's type model builds");
        let kinds = recursive_transfer_types(&model);
        for name in expected {
            assert!(
                kinds.iter().any(|kind| kind.ends_with(name)),
                "{label}: `{name}` is not a recursive kind: {kinds:?}"
            );
        }
        let edges = assert_drop_edges_match(label, &model);
        assert!(
            edges > 0,
            "{label}: no edges at all — not looking at a cycle"
        );
    }
}
