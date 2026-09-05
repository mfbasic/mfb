//! `canvas::imageHandle` and `canvas::fontHandle` — the backend id behind a live
//! `Image` or `Font` resource (plan-116-I §4.2).
//!
//! Neither is user-callable. They exist so the renderer can keep reading a plain
//! integer once `Picture.image` and `Text.font` hold the **resource itself** rather
//! than an `ImageRef`/`FontRef` record: `t.font.id` becomes
//! `canvas::fontHandle(t.font)` and nothing downstream of it changes.
//!
//! **A destroyed resource answers `0`; it does not raise.** That is the whole
//! difference from `imageRef`/`fontRef`, which these replace, and it is what the
//! letter's lifetime rule requires: a scene may still hold a `Picture` whose image the
//! program has since destroyed, and that item must render as *nothing* rather than
//! taking the frame down. `0` is already the renderer's "no such backend object"
//! answer — `__canvas_fontBlob(0)` returns an empty list and the glyph run draws
//! nothing — so the bridge reports the absence in the vocabulary the caller already
//! handles, instead of inventing a raise the render loop would have to trap.
//!
//! The read order is the resource system's, not this file's invention: `closed` is
//! checked **before** the handle is loaded. Reading the handle first and testing
//! `closed` afterwards would race a concurrent `destroy` in exactly the window that
//! makes the answer stale.

use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::Operand;
use crate::codegen::error::constants::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::types::ParameterType;

use super::gen_image::emit_closed_guard;

/// The shared body. `what` names the resource for the labels and the error text.
fn lower_handle(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    what: &str,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let record = args
        .first()
        .ok_or_else(|| format!("'{symbol}' expects the {what} argument"))?
        .location
        .clone();

    let closed = builder.label(&format!("canvas_{what}_handle_closed"));
    let done = builder.label(&format!("canvas_{what}_handle_done"));

    // Parked before the guard, because the guard is free to use scratch registers and
    // the record is read again after it.
    let record_slot = builder.allocate_stack_object(&format!("canvas_{what}_handle_rec"), 8);
    builder.emit(abi::store_u64(&record, abi::stack_pointer(), record_slot));
    emit_closed_guard(builder, &record, &closed);

    // Live: the id the backend knows it by. No allocation — `imageRef` boxed this into
    // a one-field record because a scene could not carry a resource; a scene can now,
    // so the bridge hands back the bare `Integer` and the caller keeps its own copy.
    let source = builder.temporary_vreg();
    let handle = builder.temporary_vreg();
    builder.emit(abi::load_u64(&source, abi::stack_pointer(), record_slot));
    builder.emit(abi::load_u64(&handle, &source, RESOURCE_OFFSET_HANDLE));
    builder.emit(abi::move_register(RESULT_VALUE_REGISTER, &handle));
    builder.emit(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    builder.emit(abi::branch(&done));

    // Destroyed: zero, tagged OK. Not a raise — see the module comment.
    builder.emit(abi::label(&closed));
    builder.emit(abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "0"));
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
        text: format!("canvas.{what}Handle"),
    })
}

pub(crate) fn lower_image_handle(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    lower_handle(builder, args, "image")
}

pub(crate) fn lower_font_handle(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    lower_handle(builder, args, "font")
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    for (name, what, resource, lower) in [
        (
            "imageHandle",
            "image",
            "canvas.Image",
            lower_image_handle
                as fn(&mut CodeBuilder, &[ValueResult], &AbiCtx) -> Result<ValueResult, String>,
        ),
        ("fontHandle", "font", "canvas.Font", lower_font_handle),
    ] {
        pkg.add_function(RegistryFunction {
            name,
            intro: "The backend id behind a live resource, or 0 once it is destroyed.",
            desc: "Internal. The renderer reads geometry out of a scene long after the \
                   program may have destroyed what the scene names, so this answers 0 \
                   rather than raising — 0 is already the renderer's 'no such object'.",
            example: "",
            expected_arguments: None,
            internal_only: true,
            implementations: vec![Implementation {
                params: vec![Parameter {
                    name: what,
                    desc: "",
                    aliases: &[],
                    ty: ParameterType::res(ParameterType::named(resource)),
                    default: DefaultValue::None,
                }],
                return_type: ParameterType::Integer,
                errors: vec![],
                body: Body::abi_function(lower),
            }],
        });
    }
}

#[cfg(test)]
mod tests {
    use crate::codegen::registry::registry;
    use crate::types::ParameterType;

    /// Both bridges are internal, take the **resource** and return a bare `Integer` —
    /// and neither declares an error.
    ///
    /// The empty `errors` list is the machine-checkable half of this letter's lifetime
    /// rule. `imageRef`/`fontRef`, which these replace, raise `ErrResourceClosed` for a
    /// destroyed resource; that is exactly what a renderer cannot cope with, because it
    /// reads geometry out of a scene whose contents the program may have destroyed
    /// several frames ago. A member with no declared error cannot have grown a raise
    /// back without this failing.
    ///
    /// The parameter being `RES canvas::Image` rather than `canvas::Image` is the other
    /// half: a value parameter of the resource's name renders identically and would
    /// take a copy of the record instead of aliasing the live one.
    ///
    /// The *runtime* half — a live resource answering its id and a destroyed one
    /// answering 0 — lands with the rt tests in Phase 3, because until Phase 2 wires
    /// these into `helper_geometry` nothing calls them and an internal member has no
    /// caller to observe.
    #[test]
    fn the_handle_bridges_are_internal_take_a_resource_and_cannot_raise() {
        let registry = registry();
        let package = registry
            .packages()
            .iter()
            .find(|p| p.import_name() == "canvas")
            .expect("the canvas package is registered");

        for (member, resource) in [
            ("imageHandle", "canvas.Image"),
            ("fontHandle", "canvas.Font"),
        ] {
            let function = package
                .functions()
                .iter()
                .find(|f| f.name == member)
                .unwrap_or_else(|| panic!("canvas::{member} is not registered"));
            assert!(
                function.internal_only,
                "canvas::{member} must not be user-callable — it is a renderer bridge, \
                 and exporting it would put a raw backend id back in the public surface \
                 this letter exists to remove",
            );
            let implementation = &function.implementations[0];
            assert_eq!(
                implementation.params[0].ty,
                ParameterType::res(ParameterType::named(resource)),
                "canvas::{member} must take the RESOURCE. A value parameter of the same \
                 name renders identically and copies the record instead of aliasing the \
                 live one",
            );
            assert_eq!(implementation.return_type, ParameterType::Integer);
            assert!(
                implementation.errors.is_empty(),
                "canvas::{member} declares {:?}. A destroyed resource must answer 0, not \
                 raise: the renderer reads a scene whose contents the program may have \
                 destroyed frames ago, and a raise there takes the frame down instead of \
                 drawing nothing",
                implementation.errors,
            );
        }
    }
}
