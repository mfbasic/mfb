//! `canvas::publishHashes` / `canvas::installedHashes` — the per-item content hashes
//! that key the renderer's geometry cache.
//!
//! Both are internal-only. A program has no use for them: they exist so the renderer
//! can ask "is this item the same one I generated geometry for last frame?" without
//! re-examining the item itself.
//!
//! **Why the hashes live in the arena beside the scene** rather than in the
//! renderer's own state: a resize or a damage repaint re-renders the *installed*
//! scene with no `present` call in sight (that is the whole point of installing it),
//! and it has to probe the same cache keys the presenting frame did. Keys held only
//! by the presenting path would be unavailable exactly when the repaint needs them.
//!
//! **Why the hash is computed over an item's fields rather than its bytes.** The
//! plan said bytes, and bytes would be cheaper. But a record's padding is not
//! specified to be initialized, so two items with identical content can differ in
//! their padding, and a byte hash would then miss a cache hit that is really there —
//! silently turning invariant 2 ("re-presenting unchanged content is free") back
//! off. Hashing the fields is padding-independent and therefore exactly as stable as
//! the content it stands for. See this letter's Corrections.

// --- codegen tier imports (migration) ---
use crate::codegen::app::hook::app::{prepend_wrong_mode_gate, ModeRequirement};
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::Operand;
use crate::codegen::error::constants::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::types::ParameterType;

/// The type of the published hash list.
fn hash_list_type() -> ParameterType {
    ParameterType::list_of(ParameterType::Integer)
}

/// `canvas::publishHashes(hashes AS List OF Integer) AS Nothing`.
///
/// Deep-copies the hash list into the arena and stores it at the scene region's
/// hashes slot. It does **not** bump the revision: the hashes describe a scene that
/// `publishScene` already published, and bumping again would make one logical frame
/// look like two to any reader gating on the revision.
pub(crate) fn lower_publish_hashes(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let scene_offset = ctx.canvas_scene_offset.ok_or_else(|| {
        format!("native code plan emits '{symbol}' without reserving the canvas scene region")
    })?;
    let incoming = args
        .first()
        .ok_or_else(|| format!("'{symbol}' expects the hash list argument"))?
        .location
        .clone();

    let copy = builder.copy_flat_block(&hash_list_type(), &incoming)?;
    builder.emit(abi::store_u64(
        &copy,
        ARENA_STATE_REGISTER,
        scene_offset + CANVAS_SCENE_HASHES_OFFSET,
    ));

    builder.emit(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    builder.emit(abi::return_());

    // Same gate as `publishScene`: outside `Mode.Canvas` there is no scene region to
    // write into, so the call must trap before it allocates rather than after.
    prepend_wrong_mode_gate(
        &mut builder.instructions,
        &mut builder.relocations,
        &symbol,
        ctx.presentation_mode_offset,
        ModeRequirement::Canvas,
    );

    Ok(ValueResult {
        origin: None,
        type_: ParameterType::Nothing,
        location: Operand::from("void"),
        text: symbol,
    })
}

/// `canvas::installedHashes() AS List OF Integer`.
///
/// Returns a copy, for the same reason `canvas::installedItems` does — the arena's
/// block is replaced by the next publish, and a caller holding it would be aliasing
/// storage that moves.
pub(crate) fn lower_installed_hashes(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let scene_offset = ctx.canvas_scene_offset.ok_or_else(|| {
        format!("native code plan emits '{symbol}' without reserving the canvas scene region")
    })?;

    let empty = builder.label("canvas_hashes_empty");
    let done = builder.label("canvas_hashes_done");

    let installed = builder.temporary_vreg();
    builder.emit(abi::load_u64(
        &installed,
        ARENA_STATE_REGISTER,
        scene_offset + CANVAS_SCENE_HASHES_OFFSET,
    ));
    builder.emit(abi::compare_immediate(&installed, "0"));
    builder.emit(abi::branch_eq(&empty));

    let list_type = hash_list_type();
    let copy = builder.copy_flat_block(&list_type, &installed)?;
    builder.emit(abi::move_register(RESULT_VALUE_REGISTER, &copy));
    builder.emit(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    builder.emit(abi::branch(&done));

    builder.emit(abi::label(&empty));
    let fresh = builder.lower_empty_collection(&list_type)?;
    builder.emit(abi::move_register(RESULT_VALUE_REGISTER, &fresh.location));
    builder.emit(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));

    builder.emit(abi::label(&done));
    builder.emit(abi::return_());

    Ok(ValueResult {
        origin: None,
        type_: ParameterType::Nothing,
        location: Operand::from("void"),
        text: "canvas.installedHashes".to_string(),
    })
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "publishHashes",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: true,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "hashes",
                desc: "",
                aliases: &[],
                ty: hash_list_type(),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Nothing,
            errors: vec!["ErrOutOfMemory", "ErrWrongMode"],
            body: Body::abi_function(lower_publish_hashes),
        }],
    });
    pkg.add_function(RegistryFunction {
        name: "installedHashes",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: true,
        implementations: vec![Implementation {
            params: vec![],
            return_type: hash_list_type(),
            errors: vec!["ErrOutOfMemory"],
            body: Body::abi_function(lower_installed_hashes),
        }],
    });
}
