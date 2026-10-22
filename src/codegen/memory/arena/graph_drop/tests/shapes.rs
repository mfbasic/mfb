//! The shapes the edge tables in `mod.rs` never hand the drop: a resource-bearing kind,
//! a scalar or flat value, a pointer-free block, a map key that owns a closure, a function
//! kind, and the refusals.

use super::*;
use crate::arch::ops::CodeOp;
use crate::codegen::error::constants::ARENA_FREE_SYMBOL;

/// The symbols a builder's code calls.
fn called(builder: &CodeBuilder<'_>) -> Vec<String> {
    builder
        .relocations
        .iter()
        .map(|relocation| relocation.to.clone())
        .collect()
}

/// The refusal `result` carries, which must be one.
fn refused<T>(result: Result<T, String>, what: &str) -> String {
    match result {
        Err(error) => error,
        Ok(_) => panic!("{what} must be refused"),
    }
}

/// A recursive record that holds a resource is a copy-walker kind — the copy indexes
/// it — but never a drop kind: a resource is closed by its own op, so the drop has no
/// arm for it, and asking it to drop one is refused rather than freeing a handle.
#[test]
fn a_resource_bearing_recursive_type_is_a_copy_kind_and_never_a_drop_kind() {
    let model = model(vec![record(
        "Holder",
        vec![
            field("kids", "List OF Holder"),
            field("file", "RES fs.File"),
        ],
    )]);
    assert!(
        recursive_transfer_types(&model).contains("Holder"),
        "the copy walker indexes the type"
    );
    let platform = TestPlatform;
    let harness = harness(&model);
    let mut builder = harness.builder("drop", &platform);
    let holder = ParameterType::declared("Holder");
    assert_eq!(builder.graph_drop_kind(&holder), None);
    let slot = builder.allocate_stack_object("pointer", 8);
    let error = refused(
        builder.emit_graph_drop_block(&holder, slot),
        "a resource-bearing block",
    );
    assert!(error.contains("holds a resource"), "{error}");
}

/// A scalar edge is a word, not a block: dropping one emits nothing at all. A flat list
/// element likewise owns no graph, so discarding one frees nothing.
#[test]
fn a_scalar_or_flat_value_owns_nothing_to_drop() {
    let model = model(Vec::new());
    let platform = TestPlatform;
    let harness = harness(&model);
    let mut builder = harness.builder("drop", &platform);
    let slot = builder.allocate_stack_object("pointer", 8);
    let before = builder.instructions.len();
    let freed = builder
        .emit_graph_drop_edges(&ParameterType::Integer, slot)
        .expect("a scalar edge is answered");
    assert!(!freed, "an Integer is not a block its caller frees");
    builder
        .emit_drop_list_element(slot, slot, &ParameterType::Integer)
        .expect("a flat element is answered");
    assert_eq!(
        builder.instructions.len(),
        before,
        "nothing is emitted for a value that owns no graph"
    );
}

/// What `emit_drop_entry_value` calls to free the payload of one collection entry of
/// `value_type`, over `model`.
fn entry_value_drop_calls(model: &TypeModel, value_type: &ParameterType) -> Vec<String> {
    let platform = TestPlatform;
    let harness = harness(model);
    let mut builder = harness.builder("drop", &platform);
    let collection = builder.allocate_stack_object("collection", 8);
    let entry = builder.allocate_stack_object("entry", 8);
    builder
        .emit_drop_entry_value(collection, entry, value_type)
        .unwrap_or_else(|error| panic!("{value_type}: the entry drop emits: {error}"));
    called(&builder)
}

/// A collection entry whose value is a POINTER payload — a function value or a nested
/// list — is freed through the pointer word it holds: a function value's closure by
/// `_mfb_rt_closure_drop` (bug-781), a `List OF Node`'s graph by the walker. An in-place
/// `set` over a `List OF FUNC(..)` or a `List OF List OF Node` frees the element it
/// replaces this way (`mfb build -ncode`: `drop_element_payload` in `main`).
#[test]
fn an_entry_holding_a_pointer_payload_is_freed_through_its_pointer() {
    let model = model(vec![record(
        "Node",
        vec![field("kids", "List OF Node"), field("tag", "Integer")],
    )]);
    let function = ParameterType::func(vec![ParameterType::Integer], ParameterType::Integer);
    assert_eq!(
        entry_value_drop_calls(&model, &function),
        vec![CLOSURE_DROP_SYMBOL.to_string()]
    );
    let nodes = ParameterType::list_of(ParameterType::declared("Node"));
    assert_eq!(
        entry_value_drop_calls(&model, &nodes),
        vec![GRAPH_DROP_SYMBOL.to_string()]
    );
    let record = ParameterType::declared("Node");
    assert_eq!(
        entry_value_drop_calls(&model, &record),
        vec![GRAPH_DROP_EDGES_SYMBOL.to_string()],
        "a record inlined in the data region keeps its bytes: only its edges are freed"
    );
}

/// A pointer-free block (here a `List OF Integer`, copied by one block copy) is freed
/// whole, with no walk over its entries and no push.
#[test]
fn a_pointer_free_block_is_freed_whole() {
    let model = model(Vec::new());
    let platform = TestPlatform;
    let harness = harness(&model);
    let mut builder = harness.builder("drop", &platform);
    let slot = builder.allocate_stack_object("pointer", 8);
    builder
        .emit_graph_drop_block(&ParameterType::list_of(ParameterType::Integer), slot)
        .expect("a flat list is dropped");
    assert_eq!(
        called(&builder),
        vec![ARENA_FREE_SYMBOL.to_string()],
        "one free, of the block itself"
    );
    assert!(
        !builder
            .instructions
            .iter()
            .filter(|instruction| instruction.op == CodeOp::Label)
            .filter_map(|instruction| instruction.get("name"))
            .any(|name| name.starts_with("graph_drop_collection_loop")),
        "a flat block's entries are not walked"
    );
}

/// The refusals: an edge outside the walker (whose stack it would push to) — reached
/// from a `List OF FUNC(..)` block, whose function payloads are such edges, the refusal
/// coming back out of the entry walk — a type with no drop arm, and an inline-`TRAP`
/// wrapper drop handed something that is not a `Result`.
#[test]
fn the_drop_refuses_what_it_cannot_free() {
    let model = model(Vec::new());
    let platform = TestPlatform;
    let harness = harness(&model);
    let mut builder = harness.builder("drop", &platform);
    let function = ParameterType::func(vec![ParameterType::Integer], ParameterType::Integer);
    let slot = builder.allocate_stack_object("pointer", 8);
    let error = refused(
        builder.emit_graph_drop_block(&ParameterType::list_of(function.clone()), slot),
        "an edge outside the walker",
    );
    assert!(error.contains("outside the walker"), "{error}");
    // A `Result` whose payload is not flat is a block the drop has no arm for, so it is
    // refused rather than freed as if it were flat.
    let no_arm = ParameterType::ResultOf(Box::new(function));
    let error = refused(
        builder.emit_graph_drop_block(&no_arm, slot),
        "a Result of a function value",
    );
    assert!(
        error.contains(&format!("cannot drop a value of type '{no_arm}'")),
        "{error}"
    );
    let error = refused(
        builder.emit_ok_result_payload_edges_drop(&ParameterType::Integer, slot),
        "an Integer as a Result wrapper",
    );
    assert!(error.contains("is not a Result wrapper"), "{error}");
}

/// A map whose KEY payload owns something is walked by key as well as by value: each
/// key's closure is freed. (No comparable key owns anything — a comparable record is
/// flat — so this drives the walk directly with a key the checker would refuse, to pin
/// that the key walk is the copy's, not a no-op.)
#[test]
fn a_map_key_payload_with_an_edge_is_walked_and_freed() {
    let keyed = model(vec![record(
        "FnKey",
        vec![field("f", "FUNC(Integer) AS Integer")],
    )]);
    let platform = TestPlatform;
    let harness = harness(&keyed);
    let mut builder = harness.builder("drop", &platform);
    let stack_slot = builder.allocate_stack_object("stack", 8);
    builder.graph_copy_walker = Some(GraphCopyWalker::new(&[], stack_slot));
    let slot = builder.allocate_stack_object("pointer", 8);
    builder
        .emit_graph_drop_block(
            &ParameterType::map_of(ParameterType::declared("FnKey"), ParameterType::Integer),
            slot,
        )
        .expect("the map is dropped");
    assert_eq!(
        called(&builder),
        vec![
            CLOSURE_DROP_SYMBOL.to_string(),
            ARENA_FREE_SYMBOL.to_string()
        ],
        "each key's closure, then the map's own block"
    );
}

/// The walker's arm for a function kind frees the closure through `_mfb_rt_closure_drop`
/// and frees no block of its own: the closure drop already freed it.
#[test]
fn the_walker_frees_a_function_kind_through_the_closure_drop() {
    let model = model(Vec::new());
    let function = ParameterType::func(vec![ParameterType::Integer], ParameterType::Integer);
    let harness = harness(&model);
    let platform = TestPlatform;
    crate::codegen::engine::mir::set_backend(platform.backend());
    let walker = lower_graph_drop_walker(
        GRAPH_DROP_SYMBOL,
        true,
        &[(function.name().to_string(), function)],
        &harness.function_symbols,
        &harness.functions,
        &harness.package_return_types,
        &harness.platform_imports,
        &platform,
        harness.build_mode,
        &harness.globals,
        &harness.string_symbols,
        model.clone(),
    )
    .expect("the walker lowers");
    let calls: Vec<&str> = walker
        .relocations
        .iter()
        .map(|relocation| relocation.to.as_str())
        .collect();
    assert!(calls.contains(&CLOSURE_DROP_SYMBOL), "{calls:?}");
    assert_eq!(
        calls.iter().filter(|to| **to == ARENA_FREE_SYMBOL).count(),
        1,
        "the only free is the work stack's, at the finish: {calls:?}"
    );
}
