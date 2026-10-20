//! plan-176-R: the native geometry cache's scene snapshots (`lower_scene_snapshot` and the
//! snapshot readers). The globals, word helpers and registration live in
//! `builtins/canvas/func_geo_cache/mod.rs`.

use super::*;

/// `canvas::sceneSnapshot() AS Boolean`: take the frame's ONE view of the installed scene
/// (bug-686). Every reader in the frame — `canvas::sceneResolve`, `canvas::sceneLayout`,
/// and the MFBASIC paths through `canvas::snapshotItems`/`snapshotLayers`/
/// `snapshotHashes` — uses what this stores, so the items, the layers and the hashes a
/// frame draws are one publish's. They used to be re-read from the scene region at
/// several points in a frame, and a publish landing in between paired one scene's items
/// with another's (and a frame between `publishScene` and `publishHashes` paired the new
/// items with the old hashes).
///
/// A sequence-lock read of the scene region (`CANVAS_SCENE_PENDING_OFFSET`): the
/// revision, `pending`, the pointers, `pending` again; retried until both reads of
/// `pending` equal the revision, i.e. no publish was in progress across the read. Every
/// load is an acquire on AArch64, so they happen in program order. The hashes are
/// accepted only when the revision they were published for (read before and after the
/// pointer) is this snapshot's; the answer says whether they were — FALSE means the
/// caller hashes the snapshot's items itself.
///
/// Pointers, not copies, into the worker's arena, held in `Integer` globals so nothing
/// MFBASIC owns or frees aliases them: `__CANVAS_SNAP_ITEMS`, `__CANVAS_SNAP_LAYERS`,
/// `__CANVAS_SNAP_HASHES` (0 when not accepted). They stay valid for the whole frame: a
/// publish retires the blocks it displaces and they are freed only after the frame
/// counter passes the stamp taken AFTER that publish marked itself pending — so after
/// every frame that could have snapshotted them has completed.
pub(super) fn lower_scene_snapshot(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let items_off = global(builder, "CANVAS_SNAP_ITEMS")?;
    let layers_off = global(builder, "CANVAS_SNAP_LAYERS")?;
    let hashes_off = global(builder, "CANVAS_SNAP_HASHES")?;
    let scene = scene_base(builder);
    let retry = builder.label("canvas_snapshot_retry");
    let reject = builder.label("canvas_snapshot_no_hashes");
    let done = builder.label("canvas_snapshot_done");
    let rev = builder.temporary_vreg();
    let pending = builder.temporary_vreg();
    let items = builder.temporary_vreg();
    let layers = builder.temporary_vreg();
    let hrev1 = builder.temporary_vreg();
    let hashes = builder.temporary_vreg();
    let hrev2 = builder.temporary_vreg();
    builder.emit(abi::label(&retry));
    emit_scene_load(builder, &scene, CANVAS_SCENE_REVISION_OFFSET, &rev);
    emit_scene_load(builder, &scene, CANVAS_SCENE_PENDING_OFFSET, &pending);
    builder.emit(abi::compare_registers(&pending, &rev));
    builder.emit(abi::branch_ne(&retry));
    emit_scene_load(builder, &scene, CANVAS_SCENE_ITEMS_OFFSET, &items);
    emit_scene_load(builder, &scene, CANVAS_SCENE_LAYERS_OFFSET, &layers);
    emit_scene_load(builder, &scene, CANVAS_SCENE_HASHES_REVISION_OFFSET, &hrev1);
    emit_scene_load(builder, &scene, CANVAS_SCENE_HASHES_OFFSET, &hashes);
    emit_scene_load(builder, &scene, CANVAS_SCENE_HASHES_REVISION_OFFSET, &hrev2);
    emit_scene_load(builder, &scene, CANVAS_SCENE_PENDING_OFFSET, &pending);
    builder.emit(abi::compare_registers(&pending, &rev));
    builder.emit(abi::branch_ne(&retry));
    gstore(builder, &items, items_off);
    gstore(builder, &layers, layers_off);
    let valid = builder.allocate_register();
    builder.emit(abi::move_immediate(&valid, "Boolean", "0"));
    builder.emit(abi::compare_registers(&hrev1, &rev));
    builder.emit(abi::branch_ne(&reject));
    builder.emit(abi::compare_registers(&hrev2, &rev));
    builder.emit(abi::branch_ne(&reject));
    builder.emit(abi::compare_immediate(&hashes, "0"));
    builder.emit(abi::branch_eq(&reject));
    gstore(builder, &hashes, hashes_off);
    builder.emit(abi::move_immediate(&valid, "Boolean", "1"));
    builder.emit(abi::branch(&done));
    builder.emit(abi::label(&reject));
    let zero = imm(builder, 0);
    gstore(builder, &zero, hashes_off);
    builder.emit(abi::label(&done));
    Ok(ValueResult {
        origin: None,
        type_: ParameterType::Boolean,
        location: Operand::from(valid),
        text: "canvas.sceneSnapshot".to_string(),
    })
}

/// A copy of the block the snapshot global at `name` points at, as a `list_type` value
/// MFBASIC owns — or an empty one when the snapshot holds none.
fn snapshot_copy(
    builder: &mut CodeBuilder,
    name: &str,
    list_type: ParameterType,
) -> Result<ValueResult, String> {
    let source = gload(builder, global(builder, name)?);
    let result_slot = builder.allocate_stack_object("canvas_snapshot_copy", 8);
    let empty = builder.label("canvas_snapshot_copy_empty");
    let done = builder.label("canvas_snapshot_copy_done");
    builder.emit(abi::compare_immediate(&source, "0"));
    builder.emit(abi::branch_eq(&empty));
    let copy = builder.copy_flat_block(&list_type, &source)?;
    builder.emit(abi::store_u64(&copy, abi::stack_pointer(), result_slot));
    builder.emit(abi::branch(&done));
    builder.emit(abi::label(&empty));
    let fresh = builder.lower_empty_collection(&list_type)?;
    builder.emit(abi::store_u64(
        &fresh.location,
        abi::stack_pointer(),
        result_slot,
    ));
    builder.emit(abi::label(&done));
    let result = builder.allocate_register();
    builder.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
    Ok(ValueResult {
        origin: None,
        type_: list_type,
        location: Operand::from(result),
        text: format!("canvas.{name}"),
    })
}

/// `canvas::snapshotItems() AS List OF DrawItem`: a copy of the frame snapshot's flat items.
pub(super) fn lower_snapshot_items(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    snapshot_copy(
        builder,
        "CANVAS_SNAP_ITEMS",
        ParameterType::list_of(ParameterType::named("DrawItem")),
    )
}

/// `canvas::snapshotLayers() AS List OF DrawLayer`: a copy of the frame snapshot's layers.
pub(super) fn lower_snapshot_layers(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    snapshot_copy(
        builder,
        "CANVAS_SNAP_LAYERS",
        ParameterType::list_of(ParameterType::named("DrawLayer")),
    )
}

/// `canvas::snapshotHashes() AS List OF Integer`: a copy of the hashes the frame snapshot
/// accepted (empty when it accepted none).
pub(super) fn lower_snapshot_hashes(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    snapshot_copy(builder, "CANVAS_SNAP_HASHES", int_list())
}
