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

use super::gen_image::{emit_closed_guard, IMAGE_HEIGHT, IMAGE_PIXELS, IMAGE_WIDTH};

/// The shared body. `what` names the member for the labels and the error text, and
/// `field` is the record word answered while the resource is live.
fn lower_field(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    what: &str,
    field: usize,
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
    builder.emit(abi::load_u64(&handle, &source, field));
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
        text: format!("canvas.{what}"),
    })
}

fn lower_handle(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    what: &str,
) -> Result<ValueResult, String> {
    lower_field(
        builder,
        args,
        &format!("{what}Handle"),
        RESOURCE_OFFSET_HANDLE,
    )
}

/// bug-484: `canvas::imageShadow`, `canvas::imageWidthOf`, `canvas::imageHeightOf` —
/// the three facts a `Picture`'s geometry needs, read on the graphics thread in the same
/// closed-before-field order as the handle, and answering `0` for a destroyed image for
/// the same reason.
///
/// The shadow is the address of the image's pixel block, and it serves twice. It is what
/// the draw samples, through `canvas::shadowTexel`. And it is the image's **content
/// generation**: `canvas::setBytes` swaps in a fresh block rather than writing into the
/// old one, and no shadow is ever freed — neither a close nor the scope-drop frees an
/// `Image` — so a new address means new pixels, and an old address stays readable for a
/// frame still sampling it. That is what lets a picture's geometry header stand for its
/// content without copying it.
fn lower_image_shadow(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    lower_field(builder, args, "imageShadow", IMAGE_PIXELS)
}

fn lower_image_width(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    lower_field(builder, args, "imageWidthOf", IMAGE_WIDTH)
}

fn lower_image_height(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    lower_field(builder, args, "imageHeightOf", IMAGE_HEIGHT)
}

/// `canvas::shadowTexel(shadow, index) AS Integer` — texel `index` of a pixel block, as
/// `r | g << 8 | b << 16 | a << 24`, or `0` (transparent black) when `shadow` is `0` or
/// `index` is outside the block.
///
/// One call per sampled pixel, and it allocates nothing: `__canvas_drawGeometry` owns
/// the 2.3 MB surface local, and `collections::set` stays in place only while nothing
/// allocates beneath it (`helper_items.rs`). Packing the four channels into one word is
/// what keeps it one call rather than four.
///
/// The bound is the block's own count, not a width and height the caller also holds, so
/// a caller whose arithmetic is wrong reads transparent rather than past the block.
fn lower_shadow_texel(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    if args.len() < 2 {
        return Err(format!("'{symbol}' expects the shadow and the texel index"));
    }
    let shadow_in = args[0].location.clone();
    let index_in = args[1].location.clone();
    let outside = builder.label("canvas_shadow_texel_outside");
    let done = builder.label("canvas_shadow_texel_done");

    let shadow = builder.temporary_vreg();
    let byte = builder.temporary_vreg();
    let count = builder.temporary_vreg();
    let end = builder.temporary_vreg();
    let texel = builder.temporary_vreg();
    builder.emit(abi::move_register(&shadow, &shadow_in));
    builder.emit(abi::move_register(&byte, &index_in));
    builder.emit(abi::compare_immediate(&shadow, "0"));
    builder.emit(abi::branch_eq(&outside));
    builder.emit(abi::compare_immediate(&byte, "0"));
    builder.emit(abi::branch_lt(&outside));
    // byte = index * 4, in range when byte + 4 <= count.
    builder.emit(abi::shift_left_immediate(&byte, &byte, 2));
    builder.emit(abi::load_u64(&count, &shadow, COLLECTION_OFFSET_COUNT));
    builder.emit(abi::add_immediate(&end, &byte, 4));
    builder.emit(abi::compare_registers(&end, &count));
    builder.emit(abi::branch_gt(&outside));
    // A `List OF Byte` has the fixed-width layout: payload `i` at `HEADER + i`.
    builder.emit(abi::add_registers(&byte, &shadow, &byte));
    builder.emit(abi::load_u32(&texel, &byte, COLLECTION_HEADER_SIZE));
    builder.emit(abi::move_register(RESULT_VALUE_REGISTER, &texel));
    builder.emit(abi::branch(&done));
    builder.emit(abi::label(&outside));
    builder.emit(abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "0"));
    builder.emit(abi::label(&done));
    builder.emit(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    builder.emit(abi::return_());

    Ok(ValueResult {
        origin: None,
        type_: ParameterType::Nothing,
        location: Operand::from("void"),
        text: "canvas.shadowTexel".to_string(),
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

type Lower = fn(&mut CodeBuilder, &[ValueResult], &AbiCtx) -> Result<ValueResult, String>;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    for (name, intro, lower) in [
        (
            "imageShadow",
            "The address of a live image's pixel block, or 0 once it is destroyed.",
            lower_image_shadow as Lower,
        ),
        (
            "imageWidthOf",
            "A live image's width, or 0 once it is destroyed.",
            lower_image_width as Lower,
        ),
        (
            "imageHeightOf",
            "A live image's height, or 0 once it is destroyed.",
            lower_image_height as Lower,
        ),
    ] {
        pkg.add_function(RegistryFunction {
            name,
            intro,
            desc: "Internal. Read by the Picture geometry on the graphics thread, which \
                   may see a scene naming an image the program has since destroyed — so \
                   it answers 0 rather than raising.",
            example: "",
            expected_arguments: None,
            internal_only: true,
            implementations: vec![Implementation {
                params: vec![Parameter {
                    name: "image",
                    desc: "",
                    aliases: &[],
                    ty: ParameterType::res(ParameterType::named("canvas.Image")),
                    default: DefaultValue::None,
                }],
                return_type: ParameterType::Integer,
                errors: vec![],
                body: Body::abi_function(lower),
            }],
        });
    }
    pkg.add_function(RegistryFunction {
        name: "shadowTexel",
        intro: "One RGBA texel of an image's pixel block, packed into an Integer.",
        desc: "Internal. The Picture draw's sampler: it allocates nothing, and answers 0 \
               for a null block or an index outside it.",
        example: "",
        expected_arguments: None,
        internal_only: true,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "shadow",
                    desc: "",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "index",
                    desc: "",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Integer,
            errors: vec![],
            body: Body::abi_function(lower_shadow_texel),
        }],
    });
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
