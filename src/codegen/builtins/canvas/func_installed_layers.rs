//! `canvas::installedLayers` — read the currently-installed *layered* scene back.
//!
//! The flat twin of this is `canvas::installedItems`. Both exist because a scene is
//! published in exactly one of two shapes and the renderer has to draw whichever one
//! is installed: without this, `canvas::presentLayers` would publish correctly and
//! then render nothing, because the flat reader returns empty for a layered scene.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::Operand;
use crate::codegen::error::constants::*;
use crate::codegen::registry::{AbiCtx, Body, Implementation, RegistryFunction, RegistryPackage};
use crate::target::shared::abi;
use crate::types::ParameterType;

/// `canvas::installedLayers() AS List OF DrawLayer`.
///
/// Returns a copy, for the same reason `canvas::installedItems` does — the arena's
/// block is replaced by the next publish.
pub(crate) fn lower_installed_layers(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let scene_offset = ctx.canvas_scene_offset.ok_or_else(|| {
        format!("native code plan emits '{symbol}' without reserving the canvas scene region")
    })?;

    let empty = builder.label("canvas_layers_empty");
    let done = builder.label("canvas_layers_done");

    let installed = builder.temporary_vreg();
    builder.emit(abi::load_u64(
        &installed,
        ARENA_STATE_REGISTER,
        scene_offset + CANVAS_SCENE_LAYERS_OFFSET,
    ));
    builder.emit(abi::compare_immediate(&installed, "0"));
    builder.emit(abi::branch_eq(&empty));

    let list_type = ParameterType::list_of(ParameterType::named("DrawLayer"));
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
        text: "canvas.installedLayers".to_string(),
    })
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "installedLayers",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: true,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::list_of(ParameterType::named("DrawLayer")),
            errors: vec!["ErrOutOfMemory"],
            body: Body::abi_function(lower_installed_layers),
        }],
    });
}
