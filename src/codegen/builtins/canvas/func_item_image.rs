//! plan-175-A Phase 6: `canvas::itemImageShadow(item) AS Integer` — the pixel block of
//! the image a `Tile` or a `Picture` draws from, read in place from the item.
//!
//! A cached tile or picture record is confirmed against its image's CURRENT pixel block
//! every frame (`__canvas_pictureCachedOf`), because `setBytes` swaps the block without
//! changing the item. The walk used to reach that block through the variant —
//! `CASE Tile(tile)`, `CASE Picture(pic)` — and extracting a variant copies it: one
//! allocation per tile and per picture per frame, only to read one field. This reads
//! the field where it is, so a tile or picture whose records are all cached is laid out
//! from its identities and this one word (`__canvas_appendDraw`).
//!
//! Answers:
//! * `-1` — the item is neither a `Tile` nor a `Picture`, or it is a `Picture` whose
//!   shape is an `AutoPaint` (its records are keyed by the state it shows, which only
//!   the MFBASIC walk knows);
//! * `0` — it names no image, or a closed one (the closed flag read FIRST, as every
//!   image read does: `emit_closed_guard`);
//! * else the block's address, as `canvas::imageShadow` answers it.

use super::func_geo_build::{record_fields, variant};
use super::gen_image::{emit_closed_guard, IMAGE_PIXELS};
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::{Operand, VirtualRegister};
use crate::codegen::error::constants::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::types::ParameterType;

/// The slot index of field `name` of `record`, and its type.
fn slot_of(
    builder: &CodeBuilder,
    record: &ParameterType,
    name: &str,
) -> Result<(usize, ParameterType), String> {
    record_fields(builder, record)?
        .into_iter()
        .enumerate()
        .find(|(_, (field, _))| field == name)
        .map(|(index, (_, ty))| (index, ty))
        .ok_or_else(|| format!("canvas::itemImageShadow: '{record}' has no field '{name}'"))
}

/// `out` = the pixel block behind the image resource in the slot at `rec + offset`, or
/// `0` for no resource or a closed one.
fn emit_image_block(
    builder: &mut CodeBuilder,
    out: &VirtualRegister,
    rec: &VirtualRegister,
    offset: usize,
    done: &str,
) {
    let image = builder.temporary_vreg();
    let none = builder.label("canvas_item_image_none");
    builder.emit(abi::load_u64(&image, rec, offset));
    builder.emit(abi::compare_immediate(&image, "0"));
    builder.emit(abi::branch_eq(&none));
    emit_closed_guard(builder, &image, &none);
    builder.emit(abi::load_u64(out, &image, IMAGE_PIXELS));
    builder.emit(abi::branch(done));
    builder.emit(abi::label(&none));
    builder.emit(abi::move_immediate(out, "Integer", "0"));
    builder.emit(abi::branch(done));
}

pub(crate) fn lower_item_image_shadow(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let item_in = args
        .first()
        .ok_or_else(|| format!("'{symbol}' expects the item argument"))?
        .location
        .clone();
    let (tile_tag, tile) = variant(builder, "Tile")?;
    let (picture_tag, picture) = variant(builder, "Picture")?;
    let (auto_tag, _) = variant(builder, "AutoPaint")?;
    let (tile_image, _) = slot_of(builder, &tile, "image")?;
    let (picture_image, _) = slot_of(builder, &picture, "image")?;
    let (shape_index, shape_ty) = slot_of(builder, &picture, "shape")?;
    let shape_inlined = builder.record_field_is_inlined(&shape_ty);
    if !shape_inlined && !builder.record_field_is_pointer(&shape_ty) {
        return Err(format!(
            "canvas::itemImageShadow: the union '{shape_ty}' in '{picture}' is neither \
             inlined nor a pointer"
        ));
    }

    let out = builder.temporary_vreg();
    let done = builder.label("canvas_item_image_done");
    let not_tile = builder.label("canvas_item_image_not_tile");
    let declined = builder.label("canvas_item_image_declined");
    let item = builder.temporary_vreg();
    builder.emit(abi::move_register(&item, &item_in));
    // A data union is its tag at +0 and the variant's record at +16.
    let tag = builder.temporary_vreg();
    let rec = builder.temporary_vreg();
    builder.emit(abi::load_u64(&tag, &item, 0));
    builder.emit(abi::add_immediate(&rec, &item, 16));

    builder.emit(abi::compare_immediate(&tag, &tile_tag.to_string()));
    builder.emit(abi::branch_ne(&not_tile));
    emit_image_block(builder, &out, &rec, 8 * tile_image, &done);

    builder.emit(abi::label(&not_tile));
    builder.emit(abi::compare_immediate(&tag, &picture_tag.to_string()));
    builder.emit(abi::branch_ne(&declined));
    let shape = builder.temporary_vreg();
    builder.emit(abi::load_u64(&shape, &rec, 8 * shape_index));
    if shape_inlined {
        builder.emit(abi::add_registers(&shape, &rec, &shape));
    }
    let shape_tag = builder.temporary_vreg();
    builder.emit(abi::load_u64(&shape_tag, &shape, 0));
    builder.emit(abi::compare_immediate(&shape_tag, &auto_tag.to_string()));
    builder.emit(abi::branch_eq(&declined));
    emit_image_block(builder, &out, &rec, 8 * picture_image, &done);

    // -1, as `0 - 1`: the immediate encoder takes no negative literal.
    builder.emit(abi::label(&declined));
    builder.emit(abi::move_immediate(&out, "Integer", "0"));
    builder.emit(abi::subtract_immediate(&out, &out, 1));
    builder.emit(abi::label(&done));
    builder.emit(abi::move_register(RESULT_VALUE_REGISTER, &out));
    builder.emit(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    builder.emit(abi::return_());
    Ok(ValueResult {
        origin: None,
        type_: ParameterType::Integer,
        location: Operand::from(RESULT_VALUE_REGISTER),
        text: symbol,
    })
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "itemImageShadow",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: true,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "item",
                desc: "",
                aliases: &[],
                ty: ParameterType::named("DrawItem"),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Integer,
            errors: vec![],
            body: Body::abi_function(lower_item_image_shadow),
        }],
    });
}
