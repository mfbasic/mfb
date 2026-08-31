//! `canvas::installedItems` — read the currently-installed scene back.
//!
//! Internal-only: a program already has whatever it presented, so this exists for
//! the renderer, which must draw *what was installed* rather than whatever the
//! caller happened to pass. Re-reading the published copy is what keeps "what was
//! installed" and "what is drawn" the same object when a resize or damage event
//! re-renders with no `present` in sight.
//!
//! It is also the reader plan-98-B's Phase 2 acceptance wanted and could not have:
//! with this, "a scene whose sources go out of scope is fully readable from runtime
//! storage after `present`" is directly testable rather than inferred from the
//! emitted code.

// --- codegen tier imports (migration) ---
use super::scene_base::scene_base;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::Operand;
use crate::codegen::error::constants::*;
use crate::codegen::registry::{AbiCtx, Body, Implementation, RegistryFunction, RegistryPackage};
use crate::target::shared::abi;
use crate::types::ParameterType;

/// `canvas::installedItems() AS List OF DrawItem`.
///
/// Returns a **copy**, for the same reason `canvas::getBytes` does: an MFBASIC
/// collection is a value, so handing back the runtime's own block would alias
/// storage the next `present` replaces.
///
/// An empty result when nothing is installed is the right answer rather than an
/// error: "no scene" and "an empty scene" render identically, so making the caller
/// distinguish them would buy nothing.
pub(crate) fn lower_installed_items(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let scene = scene_base(builder);

    let empty = builder.label("canvas_installed_empty");
    let done = builder.label("canvas_installed_done");

    let installed = builder.temporary_vreg();
    builder.emit(abi::load_u64(&installed, &scene, CANVAS_SCENE_ITEMS_OFFSET));
    builder.emit(abi::compare_immediate(&installed, "0"));
    builder.emit(abi::branch_eq(&empty));

    let list_type = ParameterType::list_of(ParameterType::named("DrawItem"));
    let copy = builder.copy_flat_block(&list_type, &installed)?;
    builder.emit(abi::move_register(RESULT_VALUE_REGISTER, &copy));
    builder.emit(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    builder.emit(abi::branch(&done));

    // Nothing installed: hand back an empty list, built the same way any empty
    // collection literal is, so the caller cannot tell this case apart by shape.
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
        text: "canvas.installedItems".to_string(),
    })
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "installedItems",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: true,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::list_of(ParameterType::named("DrawItem")),
            errors: vec!["ErrOutOfMemory"],
            body: Body::abi_function(lower_installed_items),
        }],
    });
}
