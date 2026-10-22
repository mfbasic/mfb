//! The copy-into-the-current-arena family (`memory/arena/builder_arena_transfer/`) over
//! the shapes a program's corpus does not hand it, driven directly on hand-built type
//! models (`graph_copy::tests::model`).
//!
//! The copy deep-copies a value so the copy aliases nothing of the source: a flat block
//! is one byte copy, a collection is a byte copy plus a per-payload fix of each entry
//! whose payload still points somewhere, a function value is a new closure. These rows
//! pin the edges of that classification — and what it refuses — for the payloads the
//! language never puts in a collection (a key that owns a closure, a mixed union) as
//! well as the ones it does (a list of resource handles, a list of recursive lists in
//! the walker).

use crate::codegen::engine::builder::{CodeBuilder, TypeModel};
use crate::codegen::engine::tests::test_support::TestPlatform;
use crate::codegen::error::constants::{
    ARENA_FREE_SYMBOL, CLOSURE_COPY_SYMBOL, RESOURCE_OFFSET_STATE, RESOURCE_RECORD_SIZE_BYTES,
};
use crate::codegen::memory::arena::graph_copy::tests::{field, harness, model, record, union};
use crate::codegen::memory::arena::graph_copy::GraphCopyWalker;
use crate::types::ParameterType;

fn function() -> ParameterType {
    ParameterType::func(vec![ParameterType::Integer], ParameterType::Integer)
}

/// A declared `RESOURCE Db` (its record is the canonical resource record).
fn db_resource() -> crate::target::shared::nir::NirType {
    let mut db = record("Db", Vec::new());
    db.kind = "resource".to_string();
    db
}

/// The refusal `result` carries, which must be one.
fn refused<T>(result: Result<T, String>, what: &str) -> String {
    match result {
        Err(error) => error,
        Ok(_) => panic!("{what} must be refused"),
    }
}

/// Run `body` on a fresh builder over `model`; the symbols the emitted code calls.
fn calls_of(
    model: &TypeModel,
    body: impl FnOnce(&mut CodeBuilder<'_>) -> Result<(), String>,
) -> Result<Vec<String>, String> {
    let platform = TestPlatform;
    let harness = harness(model);
    let mut builder = harness.builder("copy", &platform);
    body(&mut builder)?;
    Ok(builder
        .relocations
        .iter()
        .map(|relocation| relocation.to.clone())
        .collect())
}

/// Deep-copy a value of `type_` into the current arena; the symbols it calls.
fn copy_calls(model: &TypeModel, type_: &ParameterType) -> Result<Vec<String>, String> {
    calls_of(model, |builder| {
        let source = builder.temporary_vreg();
        builder.emit_thread_copy_real(type_, &source).map(|_| ())
    })
}

/// The payload classifiers answer only for a collection, and refuse anything else by
/// name; a scalar payload has no edge shape.
#[test]
fn the_collection_classifiers_refuse_a_non_collection() {
    let model = model(Vec::new());
    let platform = TestPlatform;
    let harness = harness(&model);
    let builder = harness.builder("copy", &platform);
    let error = refused(
        builder.collection_needs_transfer_fix(&ParameterType::Integer),
        "an Integer as a collection",
    );
    assert!(
        error.contains("collection type 'Integer' does not resolve"),
        "{error}"
    );
    let error = refused(
        builder.collection_payload_edges(&ParameterType::Integer),
        "an Integer's payload edges",
    );
    assert!(
        error.contains("collection type 'Integer' does not resolve"),
        "{error}"
    );
    assert!(builder
        .payload_edge_shape(&ParameterType::Integer)
        .is_none());
}

/// A map whose KEY owns a closure needs the per-payload fix for its keys, and the copy
/// gives each key its own closure. (No comparable key owns anything; this drives the
/// copy directly to pin that the key walk is real.)
#[test]
fn a_map_key_that_owns_a_closure_is_fixed_by_key() {
    let model = model(vec![record(
        "FnKey",
        vec![field("f", "FUNC(Integer) AS Integer")],
    )]);
    let map = ParameterType::map_of(ParameterType::declared("FnKey"), ParameterType::Integer);
    let platform = TestPlatform;
    let harness = harness(&model);
    let builder = harness.builder("copy", &platform);
    assert_eq!(builder.collection_needs_transfer_fix(&map), Ok(true));
    assert_eq!(
        builder.collection_payload_edges(&map),
        Ok(vec![(ParameterType::declared("FnKey"), true)]),
        "the key is the one payload with an edge"
    );
    let calls = copy_calls(&model, &map).expect("the map copies");
    assert!(
        calls.iter().any(|call| call == CLOSURE_COPY_SYMBOL),
        "{calls:?}"
    );
}

/// A list of records holding a resource handle is not a flat block — a handle is not a
/// value two owners share — but no payload needs a fix either: the copy MOVES the
/// handle words. So it is one allocation and one byte copy, with no per-entry walk.
#[test]
fn a_list_of_records_holding_a_handle_moves_its_handle_words() {
    let model = model(vec![
        db_resource(),
        record("Holder", vec![field("db", "Db"), field("n", "Integer")]),
    ]);
    let holders = ParameterType::list_of(ParameterType::declared("Holder"));
    let platform = TestPlatform;
    let harness = harness(&model);
    let mut builder = harness.builder("copy", &platform);
    assert!(
        !builder.type_is_memcpy_copyable(&holders),
        "not a flat block"
    );
    assert_eq!(builder.collection_needs_transfer_fix(&holders), Ok(false));
    let source = builder.temporary_vreg();
    builder
        .emit_thread_copy_real(&holders, &source)
        .expect("the list copies");
    assert!(
        !builder
            .instructions
            .iter()
            .filter_map(|instruction| instruction.get("name"))
            .any(|name| name.starts_with("thread_copy_collection_fix_loop")),
        "no per-entry fix"
    );
    assert!(
        builder
            .relocations
            .iter()
            .all(|relocation| relocation.to != ARENA_FREE_SYMBOL),
        "nothing is freed by a copy"
    );
}

/// A collection payload the copy cannot copy is refused, and the refusal comes back
/// out of the per-entry fix; so is the bare value.
#[test]
fn a_payload_the_copy_cannot_copy_is_refused() {
    let model = model(Vec::new());
    let result = ParameterType::ResultOf(Box::new(function()));
    let error = refused(copy_calls(&model, &result), "a Result OF FUNC");
    assert!(
        error.contains(&format!("cannot copy value of type '{result}'")),
        "{error}"
    );
    let error = refused(
        copy_calls(&model, &ParameterType::list_of(result.clone())),
        "a List OF Result OF FUNC",
    );
    assert!(
        error.contains(&format!("cannot copy value of type '{result}'")),
        "{error}"
    );
}

/// Inside the copy walker a collection payload whose type is a cycle member is pushed
/// onto the work stack with its payload word as the destination — not copied inline,
/// which would recurse per level of the data.
#[test]
fn the_walker_pushes_a_cyclic_collection_payload() {
    let model = model(vec![record(
        "Node",
        vec![field("kids", "List OF Node"), field("tag", "Integer")],
    )]);
    let kinds: Vec<String> = crate::codegen::collection::layout::recursive_transfer_types(&model)
        .into_iter()
        .collect();
    let nested = ParameterType::list_of(ParameterType::list_of(ParameterType::declared("Node")));
    let platform = TestPlatform;
    let harness = harness(&model);
    let mut builder = harness.builder("walker", &platform);
    let stack_slot = builder.allocate_stack_object("stack", 8);
    builder.graph_copy_walker = Some(GraphCopyWalker::new(&kinds, stack_slot));
    let source = builder.temporary_vreg();
    builder
        .emit_thread_copy_real(&nested, &source)
        .expect("the walker body emits");
    let pushed = crate::codegen::memory::arena::graph_copy::tests::pushed_kinds(&builder, &kinds);
    assert_eq!(pushed, vec!["List OF Node".to_string()]);
}

/// A thread's worker view is carried by pointer, never deep-copied (bug-479): nothing is
/// called.
#[test]
fn a_worker_view_is_carried_by_pointer() {
    let model = model(Vec::new());
    let worker = ParameterType::ThreadHandle {
        worker: true,
        msg: Box::new(ParameterType::Integer),
        res: Box::new(ParameterType::Integer),
        out: Box::new(ParameterType::Integer),
    };
    assert_eq!(copy_calls(&model, &worker), Ok(Vec::new()));
}

/// A union is all-data or all-resource (`TYPE_MIXED_RESOURCE_UNION`); a mixed one built
/// past the checker is refused by the union copy rather than copied as either.
#[test]
fn a_mixed_union_is_refused_by_the_union_copy() {
    let note = || vec![field("text", "String"), field("kids", "List OF Note")];
    let model = model(vec![
        record("Note", note()),
        db_resource(),
        union("Mixed", vec![("Db", Vec::new()), ("Note", note())]),
    ]);
    let error = refused(
        copy_calls(&model, &ParameterType::declared("Mixed")),
        "a mixed union value",
    );
    assert!(error.contains("TYPE_MIXED_RESOURCE_UNION"), "{error}");
}

/// The transfer copy carries a resource's declared live slots past its canonical
/// header (bug-464). Every declared slot lies wholly past the header the copy already
/// moved and inside the shared record envelope — a slot inside the header would be
/// copied twice, one past the record would write into its neighbour. The descriptors
/// are static registry data, so this is checked here over every resource and backend,
/// not per emitted copy.
#[test]
fn every_builtin_resource_live_slot_is_past_the_header_and_inside_the_record() {
    let mut seen = 0;
    for package in crate::codegen::registry::registry().packages() {
        for resource in package.resources() {
            for slot in resource.live_slots {
                seen += 1;
                assert!(
                    slot.offset >= RESOURCE_OFFSET_STATE + 8,
                    "{}: a live slot at {} is inside the header: {}",
                    resource.name,
                    slot.offset,
                    slot.what
                );
                assert!(
                    slot.offset + 8 <= RESOURCE_RECORD_SIZE_BYTES,
                    "{}: a live slot at {} runs past the record: {}",
                    resource.name,
                    slot.offset,
                    slot.what
                );
            }
        }
    }
    assert!(seen > 0, "the registry declares live slots (tls, ...)");
}
