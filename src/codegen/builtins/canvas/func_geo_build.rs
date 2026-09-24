//! `canvas::geoBuild` — a common item's whole geometry record, built natively
//! (bug-686), and `canvas::geoSame`, the exact comparison its `--debug` check uses.
//!
//! Internal-only. A geometry-cache miss on the graphics thread used to run the MFBASIC
//! header builders (`__canvas_rectHeader`, `__canvas_circleHeader`,
//! `__canvas_segmentHeader`, `__canvas_polygonHeader` and `__canvas_paintHeader`,
//! `__canvas_boundsHeader`, `__canvas_polygonEdges` beneath them): ~3 µs an item, so a
//! scene of a few thousand moving lines spent most of its frame building geometry.
//! Every one of those costs is per call and per record, not arithmetic.
//!
//! `geoBuild` answers the 47-float header followed by its tail for a `Rectangle`,
//! `RoundedRect`, `Circle`, `Line` or `Polygon` whose paint has the all-zero (identity)
//! transform and fewer than two gradient stops — what an ordinary animated item is. For
//! anything else it answers an EMPTY list and `__canvas_geometryFor` runs the MFBASIC
//! builders, which stay the definition:
//!
//! * a transform — `__canvas_invertTransform` and the transformed-bounds hull;
//! * a gradient — the stop tail and its clamping;
//! * `Ellipse` and `Arc` — the deterministic Taylor trig (`helper_shapes.rs`);
//! * `Picture` — the image reads and the shadow split; `Text` — glyph runs; `Group`.
//!
//! **The record is bit-identical to the MFBASIC one, by construction.** The software
//! rasteriser reads it and its goldens are exact, so every slot is computed with the same
//! IEEE double operations in the same order as the builder it replaces: `x + w / 2.0` is
//! a divide then an add, `__canvas_minF`/`maxF` are a compare and a select (NOT
//! `fminnm`/`fmaxnm`, which differ on a NaN and on a signed zero), a colour channel is
//! `toFloat(toInt(byte))`, and a polygon edge's `dx * dx + dy * dy` is the single
//! `fmadd` the scalar FMA pass (`opt::fma_fusion`) turns the MFBASIC expression into —
//! `dy * dy` rounded, then `dx * dx` added to it with one rounding. A `--debug` build run
//! with `MFB_CANVAS_GEO_VERIFY=1` also builds the MFBASIC record for every item this
//! built and counts the ones that differ in any bit (`geoVerifyMismatches=` on the
//! stats line); `tests/canvas/rt_canvas_geo_native.rs` renders a broad matrix with it on.
//!
//! One difference is deliberate and unobservable in a drawn frame: the MFBASIC builders
//! trap `ErrFloatOverflow`/`ErrFloatNaN` when an intermediate leaves the finite range
//! (a coordinate near 1e308), and this does not. Every finite scene gets the same bits.
//!
//! **Layout comes from the type model, never from reading source.** A data union is
//! `{tag@0, size@8, variant record@16}`; a record field is one 8-byte word; a field the
//! model inlines (`record_field_is_inlined`: a nested record such as `Paint`, `Color`,
//! `Bounds`, or a flat list such as `Polygon.points`) holds an OFFSET from the record's
//! own base to its sub-block. A nested list's data region starts past `capacity` lookup
//! entries (`.ai/collections.md`), never `count`. Variant tags come from
//! `union_variant_tags` and enum ordinals from `enum_members`.

use crate::codegen::collection::layout::{list_element_is_fixed_width, list_entry_stride};
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::{Operand, VirtualRegister};
use crate::codegen::error::constants::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::types::ParameterType;

/// `__CANVAS_GEO_HEADER`: the fixed header's length in floats.
pub(super) const GEO_HEADER: usize = 47;
/// `__CANVAS_KIND_RECT`, `__CANVAS_KIND_CIRCLE`, `__CANVAS_KIND_SEGMENT`,
/// `__CANVAS_GEO_POLYGON`, `__CANVAS_GEO_NONE` — the slot-0 kind tags.
pub(super) const KIND_RECT: f64 = 0.0;
pub(super) const KIND_CIRCLE: f64 = 1.0;
pub(super) const KIND_SEGMENT: f64 = 2.0;
pub(super) const KIND_POLYGON: f64 = 4.0;
pub(super) const KIND_NONE: f64 = 5.0;
/// `__CANVAS_GEO_GRADIENT_KIND` .. `_TOY`, and `__CANVAS_GEO_CAP`.
const SLOT_GRADIENT_KIND: usize = 42;
const SLOT_CAP: usize = 34;
/// The float32 bit pattern of 1.0, which the identity transform stores in slots 27 and
/// 30 (`__canvas_paintHeader`).
const IDENTITY_ONE_BITS: f64 = 1_065_353_216.0;
/// A polygon edge is `x0, y0, dx, dy, invLenSq`.
const EDGE_FLOATS: usize = 5;

// ---- Layout, read from the type model ----------------------------------------------

/// A canvas nominal as the type model keys it. The program's model registers the
/// builtin records package-qualified (`canvas.Paint`, `color.Color`); a helper compiled
/// inside the package may still spell one bare, so both are tried.
pub(super) fn record_type(
    builder: &CodeBuilder,
    ty: &ParameterType,
) -> Result<ParameterType, String> {
    let name = ty.name().into_owned();
    let bare = name.strip_prefix("canvas.").unwrap_or(&name).to_string();
    for spelling in [name.clone(), format!("canvas.{bare}"), bare] {
        let candidate = ParameterType::named(&spelling);
        if builder.type_model.record_fields.contains_key(&candidate) {
            return Ok(candidate);
        }
    }
    Err(format!(
        "canvas: the record type '{name}' has no layout in the type model"
    ))
}

/// The fields of a record type, in slot order.
pub(super) fn record_fields(
    builder: &CodeBuilder,
    record: &ParameterType,
) -> Result<Vec<(String, ParameterType)>, String> {
    builder
        .type_model
        .record_fields
        .get(record)
        .cloned()
        .ok_or_else(|| format!("canvas: the record type '{record}' has no layout"))
}

/// The slot index and declared type of field `name` of `record`.
pub(super) fn field(
    builder: &CodeBuilder,
    record: &ParameterType,
    name: &str,
) -> Result<(usize, ParameterType), String> {
    record_fields(builder, record)?
        .into_iter()
        .enumerate()
        .find(|(_, (field, _))| field == name)
        .map(|(index, (_, ty))| (index, ty))
        .ok_or_else(|| format!("canvas: the record '{record}' has no field '{name}'"))
}

/// The byte offset of scalar field `name` within its record.
pub(super) fn scalar_offset(
    builder: &CodeBuilder,
    record: &ParameterType,
    name: &str,
) -> Result<usize, String> {
    let (index, ty) = field(builder, record, name)?;
    if builder.record_field_is_inlined(&ty) || builder.record_field_is_pointer(&ty) {
        return Err(format!(
            "canvas: '{record}.{name}' is a composite ({ty}), not a scalar slot"
        ));
    }
    Ok(8 * index)
}

/// `dst` = the address of composite field `name` of the record at `base`: `base +
/// [base + 8*i]` when the model inlines it, `[base + 8*i]` when it is a pointer. Returns
/// the field's record/list type as the model keys it. `dst` must not be `base`.
pub(super) fn emit_field_block(
    builder: &mut CodeBuilder,
    dst: &VirtualRegister,
    base: &VirtualRegister,
    record: &ParameterType,
    name: &str,
) -> Result<ParameterType, String> {
    let (index, ty) = field(builder, record, name)?;
    builder.emit(abi::load_u64(dst, base, 8 * index));
    if builder.record_field_is_inlined(&ty) {
        builder.emit(abi::add_registers(dst, base, dst));
    } else if !builder.record_field_is_pointer(&ty) {
        return Err(format!(
            "canvas: '{record}.{name}' is a scalar ({ty}), not a composite"
        ));
    }
    if matches!(ty, ParameterType::ListOf(_)) {
        return Ok(ty);
    }
    record_type(builder, &ty)
}

/// The union tag of `DrawItem` variant `name`, and its record type.
pub(super) fn variant(builder: &CodeBuilder, name: &str) -> Result<(usize, ParameterType), String> {
    for spelling in [name.to_string(), format!("canvas.{name}")] {
        if let Some(tag) = builder
            .type_model
            .union_variant_tags
            .get(&ParameterType::named(&spelling))
        {
            let record = record_type(builder, &ParameterType::named(name))?;
            return Ok((*tag, record));
        }
    }
    Err(format!(
        "canvas: the DrawItem variant '{name}' has no union tag"
    ))
}

/// The ordinal of enum member `member` of the enum type `ty` (a field's declared type).
pub(super) fn enum_ordinal(
    builder: &CodeBuilder,
    ty: &ParameterType,
    member: &str,
) -> Result<usize, String> {
    let name = ty.name().into_owned();
    let bare = name.strip_prefix("canvas.").unwrap_or(&name).to_string();
    for spelling in [name.clone(), format!("canvas.{bare}"), bare] {
        if let Some(ordinal) = builder
            .type_model
            .enum_members
            .get(&(ParameterType::named(&spelling), member.to_string()))
        {
            return Ok(*ordinal);
        }
    }
    Err(format!(
        "canvas: the enum '{name}' has no member '{member}'"
    ))
}

/// A list block's element addressing: its entry array, its data base (capacity-based)
/// and whether an element's payload is a pointer to it rather than the element itself.
pub(super) struct ListView {
    pub(super) entries: VirtualRegister,
    pub(super) data: VirtualRegister,
    /// The element type as the list's own type spells it.
    pub(super) element: ParameterType,
    /// `Some(width)` for an entry-free fixed-width list: element `i` is at
    /// `data + i * width`.
    pub(super) fixed_width: Option<usize>,
    pub(super) pointer_payload: bool,
}

/// Compute the addressing for the list block at `list` whose type is `list_type`.
pub(super) fn emit_list_view(
    builder: &mut CodeBuilder,
    list: &VirtualRegister,
    list_type: &ParameterType,
) -> Result<ListView, String> {
    let ParameterType::ListOf(element) = list_type else {
        return Err(format!("canvas: '{list_type}' is not a list"));
    };
    let element = (**element).clone();
    let stride = list_entry_stride(&element);
    let entries = builder.temporary_vreg();
    let data = builder.temporary_vreg();
    let scratch = builder.temporary_vreg();
    builder.emit(abi::add_immediate(&entries, list, COLLECTION_HEADER_SIZE));
    builder.emit(abi::load_u64(&scratch, list, COLLECTION_OFFSET_CAPACITY));
    let stride_v = builder.temporary_vreg();
    builder.emit(abi::move_immediate(
        &stride_v,
        "Integer",
        &stride.to_string(),
    ));
    builder.emit(abi::multiply_registers(&scratch, &scratch, &stride_v));
    builder.emit(abi::add_registers(&data, &entries, &scratch));
    let fixed_width = list_element_is_fixed_width(&element);
    let pointer_payload = builder.is_pointer_collection_payload_type(&element);
    Ok(ListView {
        entries,
        data,
        element,
        fixed_width,
        pointer_payload,
    })
}

/// `dst` = the address of element `index`'s payload (for a record element, the record's
/// own base). `dst` must not be `index`.
pub(super) fn emit_list_element(
    builder: &mut CodeBuilder,
    dst: &VirtualRegister,
    view: &ListView,
    index: &VirtualRegister,
) {
    if let Some(width) = view.fixed_width {
        let w = builder.temporary_vreg();
        builder.emit(abi::move_immediate(&w, "Integer", &width.to_string()));
        builder.emit(abi::multiply_registers(dst, index, &w));
        builder.emit(abi::add_registers(dst, &view.data, dst));
        return;
    }
    let entry_size = builder.temporary_vreg();
    builder.emit(abi::move_immediate(
        &entry_size,
        "Integer",
        &COLLECTION_ENTRY_SIZE.to_string(),
    ));
    builder.emit(abi::multiply_registers(dst, index, &entry_size));
    builder.emit(abi::add_registers(dst, &view.entries, dst));
    builder.emit(abi::load_u64(
        dst,
        dst,
        COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
    ));
    builder.emit(abi::add_registers(dst, &view.data, dst));
    if view.pointer_payload {
        builder.emit(abi::load_u64(dst, dst, 0));
    }
}

/// Reserve a `List OF Float` of exactly the count in `n_slot` (count = capacity), and
/// return the stack slot holding it. The only call either function makes.
fn reserve_float_list(builder: &mut CodeBuilder, n_slot: usize) -> Result<usize, String> {
    let layout = CollectionTypeLayout::from_type(&ParameterType::list_of(ParameterType::Float))
        .ok_or_else(|| "canvas: no List OF Float layout".to_string())?;
    let n = builder.temporary_vreg();
    let eight = builder.temporary_vreg();
    let bytes = builder.temporary_vreg();
    let result_slot = builder.allocate_stack_object("canvas_geo_list", 8);
    let overflow = builder.label("canvas_geo_list_overflow");
    let alloc_ok = builder.label("canvas_geo_list_ok");
    builder.emit(abi::load_u64(&n, abi::stack_pointer(), n_slot));
    builder.emit(abi::move_immediate(&eight, "Integer", "8"));
    builder.emit_checked_size_multiply(&bytes, &n, &eight, &overflow);
    builder.emit_checked_size_add_immediate(
        abi::return_register(),
        &bytes,
        COLLECTION_HEADER_SIZE,
        &overflow,
    );
    builder.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
    builder.emit_arena_alloc_call();
    builder.emit(abi::branch_eq(&alloc_ok));
    builder.raise_error_bare("ErrOutOfMemory")?;
    builder.emit(abi::label(&overflow));
    let (code, message) =
        crate::codegen::registry::runtime_error("ErrOutOfMemory").expect("errorCode name");
    builder.emit_error_code_return(code, message)?;
    builder.emit(abi::label(&alloc_ok));
    builder.emit(abi::store_u64(
        abi::mfb_return(1),
        abi::stack_pointer(),
        result_slot,
    ));
    let base = builder.temporary_vreg();
    let nn = builder.temporary_vreg();
    let bb = builder.temporary_vreg();
    builder.emit(abi::load_u64(&base, abi::stack_pointer(), result_slot));
    builder.emit(abi::load_u64(&nn, abi::stack_pointer(), n_slot));
    builder.emit(abi::shift_left_immediate(&bb, &nn, 3));
    builder.emit_write_collection_header_full(&layout, &base, &nn, &nn, &bb, &bb);
    Ok(result_slot)
}

// ---- Float arithmetic, in the MFBASIC builders' own operations ----------------------

fn fconst(builder: &mut CodeBuilder, value: f64) -> VirtualRegister {
    let dst = builder.temporary_fp_vreg();
    let scratch = builder.temporary_vreg();
    builder.emit_f64_const(&dst, &scratch, value);
    dst
}

fn fload(builder: &mut CodeBuilder, base: &VirtualRegister, offset: usize) -> VirtualRegister {
    let dst = builder.temporary_fp_vreg();
    builder.emit(abi::load_double(&dst, base, offset));
    dst
}

fn fbin(
    builder: &mut CodeBuilder,
    op: fn(Operand, Operand, Operand) -> crate::codegen::engine::types::CodeInstruction,
    lhs: &VirtualRegister,
    rhs: &VirtualRegister,
) -> VirtualRegister {
    let dst = builder.temporary_fp_vreg();
    builder.emit(op(
        Operand::from(&dst),
        Operand::from(lhs),
        Operand::from(rhs),
    ));
    dst
}

fn fadd(b: &mut CodeBuilder, l: &VirtualRegister, r: &VirtualRegister) -> VirtualRegister {
    fbin(b, |d, l, r| abi::float_add_d(d, l, r), l, r)
}

fn fsub(b: &mut CodeBuilder, l: &VirtualRegister, r: &VirtualRegister) -> VirtualRegister {
    fbin(b, |d, l, r| abi::float_subtract_d(d, l, r), l, r)
}

fn fdiv(b: &mut CodeBuilder, l: &VirtualRegister, r: &VirtualRegister) -> VirtualRegister {
    fbin(b, |d, l, r| abi::float_divide_d(d, l, r), l, r)
}

/// `__canvas_minF(a, b)`: `IF a < b THEN a ELSE b` — a float `<` (`b.mi`), so a NaN
/// operand selects `b` exactly as the MFBASIC does.
fn fmin(builder: &mut CodeBuilder, a: &VirtualRegister, b: &VirtualRegister) -> VirtualRegister {
    fselect(builder, a, b, true)
}

/// `__canvas_maxF(a, b)`: `IF a > b THEN a ELSE b`.
fn fmax(builder: &mut CodeBuilder, a: &VirtualRegister, b: &VirtualRegister) -> VirtualRegister {
    fselect(builder, a, b, false)
}

fn fselect(
    builder: &mut CodeBuilder,
    a: &VirtualRegister,
    b: &VirtualRegister,
    less: bool,
) -> VirtualRegister {
    let dst = builder.temporary_fp_vreg();
    let take_a = builder.label("canvas_geo_sel_a");
    let done = builder.label("canvas_geo_sel_done");
    builder.emit(abi::float_compare_d(a, b));
    builder.emit(if less {
        abi::branch_mi(&take_a)
    } else {
        abi::branch_gt(&take_a)
    });
    builder.emit(abi::float_move_d_from_d(&dst, b));
    builder.emit(abi::branch(&done));
    builder.emit(abi::label(&take_a));
    builder.emit(abi::float_move_d_from_d(&dst, a));
    builder.emit(abi::label(&done));
    dst
}

/// Branch to `target` when `value <= 0.0` (float `<=`, `b.ls`: a NaN falls through).
fn branch_if_not_positive(builder: &mut CodeBuilder, value: &VirtualRegister, target: &str) {
    let zero = fconst(builder, 0.0);
    builder.emit(abi::float_compare_d(value, &zero));
    builder.emit(abi::branch_ls(target));
}

/// Store `value` into header/tail slot `slot` of the record at `out`.
fn put(builder: &mut CodeBuilder, out: &VirtualRegister, slot: usize, value: &VirtualRegister) {
    builder.emit(abi::store_double(value, out, slot * 8));
}

fn put_const(builder: &mut CodeBuilder, out: &VirtualRegister, slot: usize, value: f64) {
    let v = fconst(builder, value);
    put(builder, out, slot, &v);
}

/// Copy a Float field's raw 64 bits into a slot — `collections::set(out, s, field)`.
fn put_field(
    builder: &mut CodeBuilder,
    out: &VirtualRegister,
    slot: usize,
    base: &VirtualRegister,
    offset: usize,
) {
    let v = fload(builder, base, offset);
    put(builder, out, slot, &v);
}

// ---- The paint -----------------------------------------------------------------------

/// The addresses a `Paint` is read through, computed once per item.
struct PaintView {
    paint: VirtualRegister,
    paint_type: ParameterType,
}

/// `__canvas_strokeHalf(paint)`: `-1.0` when the stroke's alpha is 0 or the width is
/// not positive, else `strokeWidth / 2.0`.
fn emit_stroke_half(
    builder: &mut CodeBuilder,
    view: &PaintView,
) -> Result<VirtualRegister, String> {
    let stroke = builder.temporary_vreg();
    let stroke_type = emit_field_block(builder, &stroke, &view.paint, &view.paint_type, "stroke")?;
    let alpha_off = scalar_offset(builder, &stroke_type, "alpha")?;
    let width_off = scalar_offset(builder, &view.paint_type, "strokeWidth")?;
    let half = builder.temporary_fp_vreg();
    let none = builder.label("canvas_geo_half_none");
    let done = builder.label("canvas_geo_half_done");
    let alpha = builder.temporary_vreg();
    builder.emit(abi::load_u8(&alpha, &stroke, alpha_off));
    builder.emit(abi::compare_immediate(&alpha, "0"));
    builder.emit(abi::branch_le(&none));
    let width = fload(builder, &view.paint, width_off);
    branch_if_not_positive(builder, &width, &none);
    let two = fconst(builder, 2.0);
    builder.emit(abi::float_divide_d(&half, &width, &two));
    builder.emit(abi::branch(&done));
    builder.emit(abi::label(&none));
    // `0.0 - 1.0`, which is exactly -1.0.
    let minus_one = fconst(builder, -1.0);
    builder.emit(abi::float_move_d_from_d(&half, &minus_one));
    builder.emit(abi::label(&done));
    Ok(half)
}

/// Write a colour's four channels, `toFloat(toInt(channel))`, into four slots.
fn put_color(
    builder: &mut CodeBuilder,
    out: &VirtualRegister,
    first_slot: usize,
    view: &PaintView,
    which: &str,
) -> Result<(), String> {
    let color = builder.temporary_vreg();
    let color_type = emit_field_block(builder, &color, &view.paint, &view.paint_type, which)?;
    for (k, channel) in ["red", "green", "blue", "alpha"].iter().enumerate() {
        let off = scalar_offset(builder, &color_type, channel)?;
        let byte = builder.temporary_vreg();
        builder.emit(abi::load_u8(&byte, &color, off));
        let f = builder.temporary_fp_vreg();
        builder.emit(abi::signed_convert_to_float_d(&f, &byte));
        put(builder, out, first_slot + k, &f);
    }
    Ok(())
}

/// `__canvas_paintHeader` for a paint with the identity transform and no gradient
/// stops: slots 7-15, 22-26, 41-46 and the identity's 27 and 30. Slot 1 gains
/// `toFloat(0)`, which leaves it unchanged, and slot 41 is the zero it already holds.
fn emit_paint_header(
    builder: &mut CodeBuilder,
    out: &VirtualRegister,
    view: &PaintView,
    stroke_half: &VirtualRegister,
) -> Result<(), String> {
    put(builder, out, 7, stroke_half);
    put_color(builder, out, 8, view, "fill")?;
    put_color(builder, out, 12, view, "stroke")?;

    let clip = builder.temporary_vreg();
    let clip_type = emit_field_block(builder, &clip, &view.paint, &view.paint_type, "clip")?;
    let cx = fload(builder, &clip, scalar_offset(builder, &clip_type, "x")?);
    let cy = fload(builder, &clip, scalar_offset(builder, &clip_type, "y")?);
    let cw = fload(builder, &clip, scalar_offset(builder, &clip_type, "w")?);
    let ch = fload(builder, &clip, scalar_offset(builder, &clip_type, "h")?);
    put(builder, out, 22, &cx);
    put(builder, out, 23, &cy);
    let x1 = fadd(builder, &cx, &cw);
    put(builder, out, 24, &x1);
    let y1 = fadd(builder, &cy, &ch);
    put(builder, out, 25, &y1);

    // The blend mode's tag: Normal 0, Multiply 1, Screen 2, Add 3.
    let (blend_index, blend_type) = field(builder, &view.paint_type, "blend")?;
    let blend = builder.temporary_vreg();
    builder.emit(abi::load_u64(&blend, &view.paint, 8 * blend_index));
    let blend_f = builder.temporary_fp_vreg();
    let blend_done = builder.label("canvas_geo_blend_done");
    let zero = fconst(builder, 0.0);
    builder.emit(abi::float_move_d_from_d(&blend_f, &zero));
    for (member, tag) in [("Multiply", 1.0), ("Screen", 2.0), ("Add", 3.0)] {
        let ordinal = enum_ordinal(builder, &blend_type, member)?;
        let next = builder.label("canvas_geo_blend_next");
        builder.emit(abi::compare_immediate(&blend, &ordinal.to_string()));
        builder.emit(abi::branch_ne(&next));
        let v = fconst(builder, tag);
        builder.emit(abi::float_move_d_from_d(&blend_f, &v));
        builder.emit(abi::branch(&blend_done));
        builder.emit(abi::label(&next));
    }
    builder.emit(abi::label(&blend_done));
    put(builder, out, 26, &blend_f);

    // The gradient's kind and two points are written whatever its stop count.
    let gradient = builder.temporary_vreg();
    let gradient_type = emit_field_block(
        builder,
        &gradient,
        &view.paint,
        &view.paint_type,
        "fillGradient",
    )?;
    let (kind_index, kind_type) = field(builder, &gradient_type, "kind")?;
    let radial = enum_ordinal(builder, &kind_type, "Radial")?;
    let gkind = builder.temporary_vreg();
    builder.emit(abi::load_u64(&gkind, &gradient, 8 * kind_index));
    let gkind_f = builder.temporary_fp_vreg();
    let not_radial = builder.label("canvas_geo_linear");
    let kind_done = builder.label("canvas_geo_kind_done");
    builder.emit(abi::compare_immediate(&gkind, &radial.to_string()));
    builder.emit(abi::branch_ne(&not_radial));
    let one = fconst(builder, 1.0);
    builder.emit(abi::float_move_d_from_d(&gkind_f, &one));
    builder.emit(abi::branch(&kind_done));
    builder.emit(abi::label(&not_radial));
    let zero = fconst(builder, 0.0);
    builder.emit(abi::float_move_d_from_d(&gkind_f, &zero));
    builder.emit(abi::label(&kind_done));
    put(builder, out, SLOT_GRADIENT_KIND, &gkind_f);
    for (slot, point) in [(43usize, "startPoint"), (45, "endPoint")] {
        let p = builder.temporary_vreg();
        let point_type = emit_field_block(builder, &p, &gradient, &gradient_type, point)?;
        let xo = scalar_offset(builder, &point_type, "x")?;
        let yo = scalar_offset(builder, &point_type, "y")?;
        put_field(builder, out, slot, &p, xo);
        put_field(builder, out, slot + 1, &p, yo);
    }

    put_const(builder, out, 27, IDENTITY_ONE_BITS);
    put_const(builder, out, 30, IDENTITY_ONE_BITS);
    Ok(())
}

/// `__canvas_boundsHeader` for an identity transform: the four bounds, unmodified.
fn put_bounds(builder: &mut CodeBuilder, out: &VirtualRegister, bounds: [&VirtualRegister; 4]) {
    for (k, value) in bounds.iter().enumerate() {
        put(builder, out, 16 + k, value);
    }
}

/// `pad = __canvas_maxF(__canvas_strokeHalf(paint), 0.0) + 1.0`.
fn stroke_pad(builder: &mut CodeBuilder, stroke_half: &VirtualRegister) -> VirtualRegister {
    let zero = fconst(builder, 0.0);
    let m = fmax(builder, stroke_half, &zero);
    let one = fconst(builder, 1.0);
    fadd(builder, &m, &one)
}

/// Branch to `unsupported` unless the paint's transform is all zero (float `=`, so a
/// `-0.0` is zero and a NaN is not — the MFBASIC test) and it has fewer than two stops.
fn emit_paint_supported(
    builder: &mut CodeBuilder,
    view: &PaintView,
    unsupported: &str,
) -> Result<(), String> {
    let transform = builder.temporary_vreg();
    let transform_type = emit_field_block(
        builder,
        &transform,
        &view.paint,
        &view.paint_type,
        "transform",
    )?;
    let zero = fconst(builder, 0.0);
    for name in ["a", "b", "c", "d", "tx", "ty"] {
        let v = fload(
            builder,
            &transform,
            scalar_offset(builder, &transform_type, name)?,
        );
        builder.emit(abi::float_compare_d(&v, &zero));
        builder.emit(abi::branch_ne(unsupported));
    }
    let gradient = builder.temporary_vreg();
    let gradient_type = emit_field_block(
        builder,
        &gradient,
        &view.paint,
        &view.paint_type,
        "fillGradient",
    )?;
    let stops = builder.temporary_vreg();
    emit_field_block(builder, &stops, &gradient, &gradient_type, "stops")?;
    let count = builder.temporary_vreg();
    builder.emit(abi::load_u64(&count, &stops, COLLECTION_OFFSET_COUNT));
    builder.emit(abi::compare_immediate(&count, "2"));
    builder.emit(abi::branch_ge(unsupported));
    Ok(())
}

// ---- The kinds -------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Rect,
    Rounded,
    Circle,
    Line,
    Polygon,
}

impl Kind {
    const ALL: [Kind; 5] = [
        Kind::Rect,
        Kind::Rounded,
        Kind::Circle,
        Kind::Line,
        Kind::Polygon,
    ];

    fn variant(self) -> &'static str {
        match self {
            Kind::Rect => "Rectangle",
            Kind::Rounded => "RoundedRect",
            Kind::Circle => "Circle",
            Kind::Line => "Line",
            Kind::Polygon => "Polygon",
        }
    }
}

/// `__canvas_emptyHeader`: kind NONE, length 47, every other slot zero.
fn put_empty(builder: &mut CodeBuilder, out: &VirtualRegister) {
    put_const(builder, out, 0, KIND_NONE);
    put_const(builder, out, 1, GEO_HEADER as f64);
}

/// `__canvas_rectHeader` (a `Rectangle` passes `cornerRadius = 0.0`).
fn fill_rect(
    builder: &mut CodeBuilder,
    out: &VirtualRegister,
    rec: &VirtualRegister,
    record: &ParameterType,
    view: &PaintView,
    rounded: bool,
    done: &str,
) -> Result<(), String> {
    let empty = builder.label("canvas_geo_rect_empty");
    let x = fload(builder, rec, scalar_offset(builder, record, "x")?);
    let y = fload(builder, rec, scalar_offset(builder, record, "y")?);
    let w = fload(builder, rec, scalar_offset(builder, record, "w")?);
    let h = fload(builder, rec, scalar_offset(builder, record, "h")?);
    let corner = if rounded {
        fload(
            builder,
            rec,
            scalar_offset(builder, record, "cornerRadius")?,
        )
    } else {
        fconst(builder, 0.0)
    };
    branch_if_not_positive(builder, &w, &empty);
    branch_if_not_positive(builder, &h, &empty);
    let two = fconst(builder, 2.0);
    let zero = fconst(builder, 0.0);
    let wh = fmin(builder, &w, &h);
    let limit = fdiv(builder, &wh, &two);
    let corner_pos = fmax(builder, &corner, &zero);
    let radius = fmin(builder, &corner_pos, &limit);
    put_const(builder, out, 0, KIND_RECT);
    put_const(builder, out, 1, GEO_HEADER as f64);
    let half_w = fdiv(builder, &w, &two);
    let cx = fadd(builder, &x, &half_w);
    put(builder, out, 2, &cx);
    let half_h = fdiv(builder, &h, &two);
    let cy = fadd(builder, &y, &half_h);
    put(builder, out, 3, &cy);
    // `w / 2.0 - radius`: the same quotient as slot 2's, so it is reused.
    let ex = fsub(builder, &half_w, &radius);
    put(builder, out, 4, &ex);
    let ey = fsub(builder, &half_h, &radius);
    put(builder, out, 5, &ey);
    put(builder, out, 6, &radius);
    let stroke_half = emit_stroke_half(builder, view)?;
    emit_paint_header(builder, out, view, &stroke_half)?;
    let pad = stroke_pad(builder, &stroke_half);
    let x0 = fsub(builder, &x, &pad);
    let y0 = fsub(builder, &y, &pad);
    let xw = fadd(builder, &x, &w);
    let x1 = fadd(builder, &xw, &pad);
    let yh = fadd(builder, &y, &h);
    let y1 = fadd(builder, &yh, &pad);
    put_bounds(builder, out, [&x0, &y0, &x1, &y1]);
    builder.emit(abi::branch(done));
    builder.emit(abi::label(&empty));
    put_empty(builder, out);
    builder.emit(abi::branch(done));
    Ok(())
}

/// `__canvas_circleHeader`.
fn fill_circle(
    builder: &mut CodeBuilder,
    out: &VirtualRegister,
    rec: &VirtualRegister,
    record: &ParameterType,
    view: &PaintView,
    done: &str,
) -> Result<(), String> {
    let empty = builder.label("canvas_geo_circle_empty");
    let x = fload(builder, rec, scalar_offset(builder, record, "x")?);
    let y = fload(builder, rec, scalar_offset(builder, record, "y")?);
    let radius = fload(builder, rec, scalar_offset(builder, record, "radius")?);
    branch_if_not_positive(builder, &radius, &empty);
    put_const(builder, out, 0, KIND_CIRCLE);
    put_const(builder, out, 1, GEO_HEADER as f64);
    put(builder, out, 2, &x);
    put(builder, out, 3, &y);
    put(builder, out, 4, &radius);
    let stroke_half = emit_stroke_half(builder, view)?;
    emit_paint_header(builder, out, view, &stroke_half)?;
    // `radius + __canvas_maxF(half, 0.0) + 1.0`, left to right.
    let zero = fconst(builder, 0.0);
    let m = fmax(builder, &stroke_half, &zero);
    let rm = fadd(builder, &radius, &m);
    let one = fconst(builder, 1.0);
    let reach = fadd(builder, &rm, &one);
    let x0 = fsub(builder, &x, &reach);
    let y0 = fsub(builder, &y, &reach);
    let x1 = fadd(builder, &x, &reach);
    let y1 = fadd(builder, &y, &reach);
    put_bounds(builder, out, [&x0, &y0, &x1, &y1]);
    builder.emit(abi::branch(done));
    builder.emit(abi::label(&empty));
    put_empty(builder, out);
    builder.emit(abi::branch(done));
    Ok(())
}

/// `__canvas_segmentHeader` then `__canvas_strokeAsFill`.
fn fill_line(
    builder: &mut CodeBuilder,
    out: &VirtualRegister,
    rec: &VirtualRegister,
    record: &ParameterType,
    view: &PaintView,
    done: &str,
) -> Result<(), String> {
    let empty = builder.label("canvas_geo_line_empty");
    let half = emit_stroke_half(builder, view)?;
    branch_if_not_positive(builder, &half, &empty);
    let x1 = fload(builder, rec, scalar_offset(builder, record, "x1")?);
    let y1 = fload(builder, rec, scalar_offset(builder, record, "y1")?);
    let x2 = fload(builder, rec, scalar_offset(builder, record, "x2")?);
    let y2 = fload(builder, rec, scalar_offset(builder, record, "y2")?);
    put_const(builder, out, 0, KIND_SEGMENT);
    put_const(builder, out, 1, GEO_HEADER as f64);
    put(builder, out, 2, &x1);
    put(builder, out, 3, &y1);
    put(builder, out, 4, &x2);
    put(builder, out, 5, &y2);
    put(builder, out, 6, &half);
    // `__canvas_capTag`: Round is 1, anything else 0.
    let (cap_index, cap_type) = field(builder, record, "cap")?;
    let round = enum_ordinal(builder, &cap_type, "Round")?;
    let cap = builder.temporary_vreg();
    builder.emit(abi::load_u64(&cap, rec, 8 * cap_index));
    let butt = builder.label("canvas_geo_cap_butt");
    let cap_done = builder.label("canvas_geo_cap_done");
    builder.emit(abi::compare_immediate(&cap, &round.to_string()));
    builder.emit(abi::branch_ne(&butt));
    put_const(builder, out, SLOT_CAP, 1.0);
    builder.emit(abi::branch(&cap_done));
    builder.emit(abi::label(&butt));
    put_const(builder, out, SLOT_CAP, 0.0);
    builder.emit(abi::label(&cap_done));
    emit_paint_header(builder, out, view, &half)?;
    // `__canvas_strokeAsFill`: the stroke colour into the fill slots, half = -1.
    for k in 0..4 {
        let v = fload(builder, out, (12 + k) * 8);
        put(builder, out, 8 + k, &v);
    }
    put_const(builder, out, 7, -1.0);
    let one = fconst(builder, 1.0);
    let pad = fadd(builder, &half, &one);
    let min_x = fmin(builder, &x1, &x2);
    let bx0 = fsub(builder, &min_x, &pad);
    let min_y = fmin(builder, &y1, &y2);
    let by0 = fsub(builder, &min_y, &pad);
    let max_x = fmax(builder, &x1, &x2);
    let bx1 = fadd(builder, &max_x, &pad);
    let max_y = fmax(builder, &y1, &y2);
    let by1 = fadd(builder, &max_y, &pad);
    put_bounds(builder, out, [&bx0, &by0, &bx1, &by1]);
    builder.emit(abi::branch(done));
    builder.emit(abi::label(&empty));
    put_empty(builder, out);
    builder.emit(abi::branch(done));
    Ok(())
}

/// `__canvas_polygonHeader` and `__canvas_polygonEdges`.
fn fill_polygon(
    builder: &mut CodeBuilder,
    out: &VirtualRegister,
    rec: &VirtualRegister,
    record: &ParameterType,
    view: &PaintView,
    done: &str,
) -> Result<(), String> {
    let empty = builder.label("canvas_geo_poly_empty");
    let points = builder.temporary_vreg();
    let points_type = emit_field_block(builder, &points, rec, record, "points")?;
    let count = builder.temporary_vreg();
    builder.emit(abi::load_u64(&count, &points, COLLECTION_OFFSET_COUNT));
    builder.emit(abi::compare_immediate(&count, "2"));
    builder.emit(abi::branch_lt(&empty));
    let list = emit_list_view(builder, &points, &points_type)?;
    let point_type = record_type(builder, &list.element)?;
    let xo = scalar_offset(builder, &point_type, "x")?;
    let yo = scalar_offset(builder, &point_type, "y")?;

    // The bounds: the first point, then minF/maxF over the rest, in order.
    let i = builder.temporary_vreg();
    let q = builder.temporary_vreg();
    builder.emit(abi::move_immediate(&i, "Integer", "0"));
    emit_list_element(builder, &q, &list, &i);
    let min_x = builder.temporary_fp_vreg();
    let max_x = builder.temporary_fp_vreg();
    let min_y = builder.temporary_fp_vreg();
    let max_y = builder.temporary_fp_vreg();
    let fx = fload(builder, &q, xo);
    let fy = fload(builder, &q, yo);
    builder.emit(abi::float_move_d_from_d(&min_x, &fx));
    builder.emit(abi::float_move_d_from_d(&max_x, &fx));
    builder.emit(abi::float_move_d_from_d(&min_y, &fy));
    builder.emit(abi::float_move_d_from_d(&max_y, &fy));
    let head = builder.label("canvas_geo_poly_bounds");
    let bounds_done = builder.label("canvas_geo_poly_bounds_done");
    builder.emit(abi::move_immediate(&i, "Integer", "1"));
    builder.emit(abi::label(&head));
    builder.emit(abi::compare_registers(&i, &count));
    builder.emit(abi::branch_ge(&bounds_done));
    emit_list_element(builder, &q, &list, &i);
    let qx = fload(builder, &q, xo);
    let qy = fload(builder, &q, yo);
    let v = fmin(builder, &min_x, &qx);
    builder.emit(abi::float_move_d_from_d(&min_x, &v));
    let v = fmax(builder, &max_x, &qx);
    builder.emit(abi::float_move_d_from_d(&max_x, &v));
    let v = fmin(builder, &min_y, &qy);
    builder.emit(abi::float_move_d_from_d(&min_y, &v));
    let v = fmax(builder, &max_y, &qy);
    builder.emit(abi::float_move_d_from_d(&max_y, &v));
    builder.emit(abi::add_immediate(&i, &i, 1));
    builder.emit(abi::branch(&head));
    builder.emit(abi::label(&bounds_done));

    // Header: kind, the record length `toFloat(47 + count * 5)`, the paint, the edge
    // count in slot 20, then the padded bounds.
    put_const(builder, out, 0, KIND_POLYGON);
    let length = builder.temporary_vreg();
    let five = builder.temporary_vreg();
    builder.emit(abi::move_immediate(
        &five,
        "Integer",
        &EDGE_FLOATS.to_string(),
    ));
    builder.emit(abi::multiply_registers(&length, &count, &five));
    builder.emit(abi::add_immediate(&length, &length, GEO_HEADER));
    let length_f = builder.temporary_fp_vreg();
    builder.emit(abi::signed_convert_to_float_d(&length_f, &length));
    put(builder, out, 1, &length_f);
    let stroke_half = emit_stroke_half(builder, view)?;
    emit_paint_header(builder, out, view, &stroke_half)?;
    let count_f = builder.temporary_fp_vreg();
    builder.emit(abi::signed_convert_to_float_d(&count_f, &count));
    put(builder, out, 20, &count_f);
    let pad = stroke_pad(builder, &stroke_half);
    let bx0 = fsub(builder, &min_x, &pad);
    let by0 = fsub(builder, &min_y, &pad);
    let bx1 = fadd(builder, &max_x, &pad);
    let by1 = fadd(builder, &max_y, &pad);
    put_bounds(builder, out, [&bx0, &by0, &bx1, &by1]);

    // The edges: `x0, y0, dx, dy, invLenSq` for point i to point (i + 1) MOD count.
    let cursor = builder.temporary_vreg();
    builder.emit(abi::add_immediate(&cursor, out, GEO_HEADER * 8));
    let j = builder.temporary_vreg();
    let a = builder.temporary_vreg();
    let b = builder.temporary_vreg();
    let edge_head = builder.label("canvas_geo_poly_edge");
    let edge_wrap = builder.label("canvas_geo_poly_wrap");
    let edge_done = builder.label("canvas_geo_poly_edges_done");
    builder.emit(abi::move_immediate(&i, "Integer", "0"));
    builder.emit(abi::label(&edge_head));
    builder.emit(abi::compare_registers(&i, &count));
    builder.emit(abi::branch_ge(&edge_done));
    builder.emit(abi::add_immediate(&j, &i, 1));
    builder.emit(abi::compare_registers(&j, &count));
    builder.emit(abi::branch_lt(&edge_wrap));
    builder.emit(abi::move_immediate(&j, "Integer", "0"));
    builder.emit(abi::label(&edge_wrap));
    emit_list_element(builder, &a, &list, &i);
    emit_list_element(builder, &b, &list, &j);
    let ax = fload(builder, &a, xo);
    let ay = fload(builder, &a, yo);
    let bx = fload(builder, &b, xo);
    let by = fload(builder, &b, yo);
    let dx = fsub(builder, &bx, &ax);
    let dy = fsub(builder, &by, &ay);
    // `dx * dx + dy * dy` as `opt::fma_fusion` lowers it: `dy * dy` rounded, then
    // `fmadd(addend = dy*dy, dx, dx)`.
    let dy2 = builder.temporary_fp_vreg();
    builder.emit(abi::float_multiply_d(&dy2, &dy, &dy));
    let len_sq = builder.temporary_fp_vreg();
    builder.emit(abi::float_multiply_add_d(&len_sq, &dy2, &dx, &dx));
    builder.emit(abi::store_double(&ax, &cursor, 0));
    builder.emit(abi::store_double(&ay, &cursor, 8));
    builder.emit(abi::store_double(&dx, &cursor, 16));
    builder.emit(abi::store_double(&dy, &cursor, 24));
    let inv = builder.temporary_fp_vreg();
    let degenerate = builder.label("canvas_geo_poly_degenerate");
    let inv_done = builder.label("canvas_geo_poly_inv_done");
    let zero = fconst(builder, 0.0);
    builder.emit(abi::float_compare_d(&len_sq, &zero));
    builder.emit(abi::branch_gt(&inv_done));
    builder.emit(abi::branch(&degenerate));
    builder.emit(abi::label(&inv_done));
    let one = fconst(builder, 1.0);
    builder.emit(abi::float_divide_d(&inv, &one, &len_sq));
    let stored = builder.label("canvas_geo_poly_inv_store");
    builder.emit(abi::branch(&stored));
    builder.emit(abi::label(&degenerate));
    let zero = fconst(builder, 0.0);
    builder.emit(abi::float_move_d_from_d(&inv, &zero));
    builder.emit(abi::label(&stored));
    builder.emit(abi::store_double(&inv, &cursor, 32));
    builder.emit(abi::add_immediate(&cursor, &cursor, EDGE_FLOATS * 8));
    builder.emit(abi::add_immediate(&i, &i, 1));
    builder.emit(abi::branch(&edge_head));
    builder.emit(abi::label(&edge_done));
    builder.emit(abi::branch(done));

    builder.emit(abi::label(&empty));
    put_empty(builder, out);
    builder.emit(abi::branch(done));
    Ok(())
}

pub(crate) fn lower_geo_build(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let incoming = args
        .first()
        .ok_or_else(|| format!("'{symbol}' expects the DrawItem argument"))?
        .location
        .clone();
    let paint_type = record_type(builder, &ParameterType::named("Paint"))?;
    let kinds = Kind::ALL
        .iter()
        .map(|kind| variant(builder, kind.variant()).map(|(tag, record)| (*kind, tag, record)))
        .collect::<Result<Vec<_>, String>>()?;

    let item_slot = builder.allocate_stack_object("canvas_geo_item", 8);
    builder.emit(abi::store_u64(&incoming, abi::stack_pointer(), item_slot));
    let n_slot = builder.allocate_stack_object("canvas_geo_n", 8);

    // Phase 1: is the item one this builds, and how many floats is its record?
    let item = builder.temporary_vreg();
    let tag = builder.temporary_vreg();
    let rec = builder.temporary_vreg();
    let n = builder.temporary_vreg();
    builder.emit(abi::load_u64(&item, abi::stack_pointer(), item_slot));
    builder.emit(abi::load_u64(&tag, &item, 0));
    builder.emit(abi::add_immediate(&rec, &item, 16));
    let unsupported = builder.label("canvas_geo_unsupported");
    let sized = builder.label("canvas_geo_sized");
    let size_labels: Vec<String> = kinds
        .iter()
        .map(|_| builder.label("canvas_geo_size_kind"))
        .collect();
    for ((_, kind_tag, _), label) in kinds.iter().zip(&size_labels) {
        builder.emit(abi::compare_immediate(&tag, &kind_tag.to_string()));
        builder.emit(abi::branch_eq(label));
    }
    builder.emit(abi::branch(&unsupported));
    for ((kind, _, record), label) in kinds.iter().zip(&size_labels) {
        builder.emit(abi::label(label));
        let paint = builder.temporary_vreg();
        emit_field_block(builder, &paint, &rec, record, "paint")?;
        let view = PaintView {
            paint,
            paint_type: paint_type.clone(),
        };
        emit_paint_supported(builder, &view, &unsupported)?;
        builder.emit(abi::move_immediate(&n, "Integer", &GEO_HEADER.to_string()));
        if *kind == Kind::Polygon {
            let points = builder.temporary_vreg();
            emit_field_block(builder, &points, &rec, record, "points")?;
            let count = builder.temporary_vreg();
            builder.emit(abi::load_u64(&count, &points, COLLECTION_OFFSET_COUNT));
            builder.emit(abi::compare_immediate(&count, "2"));
            builder.emit(abi::branch_lt(&sized));
            let five = builder.temporary_vreg();
            builder.emit(abi::move_immediate(
                &five,
                "Integer",
                &EDGE_FLOATS.to_string(),
            ));
            builder.emit(abi::multiply_registers(&count, &count, &five));
            builder.emit(abi::add_registers(&n, &n, &count));
        }
        builder.emit(abi::branch(&sized));
    }
    builder.emit(abi::label(&unsupported));
    builder.emit(abi::move_immediate(&n, "Integer", "0"));
    builder.emit(abi::label(&sized));
    builder.emit(abi::store_u64(&n, abi::stack_pointer(), n_slot));

    // Phase 2: the one allocation. Everything after it is re-read from the stack.
    let list_slot = reserve_float_list(builder, n_slot)?;

    // Phase 3: fill it.
    let done = builder.label("canvas_geo_done");
    let n = builder.temporary_vreg();
    builder.emit(abi::load_u64(&n, abi::stack_pointer(), n_slot));
    builder.emit(abi::compare_immediate(&n, "0"));
    builder.emit(abi::branch_eq(&done));
    let out = builder.temporary_vreg();
    builder.emit(abi::load_u64(&out, abi::stack_pointer(), list_slot));
    // A fixed-width list is entry-free: its floats start right after the header.
    builder.emit(abi::add_immediate(&out, &out, COLLECTION_HEADER_SIZE));
    let zero = builder.temporary_vreg();
    builder.emit(abi::move_immediate(&zero, "Integer", "0"));
    for slot in 0..GEO_HEADER {
        builder.emit(abi::store_u64(&zero, &out, slot * 8));
    }
    let item = builder.temporary_vreg();
    let tag = builder.temporary_vreg();
    let rec = builder.temporary_vreg();
    builder.emit(abi::load_u64(&item, abi::stack_pointer(), item_slot));
    builder.emit(abi::load_u64(&tag, &item, 0));
    builder.emit(abi::add_immediate(&rec, &item, 16));
    let fill_labels: Vec<String> = kinds
        .iter()
        .map(|_| builder.label("canvas_geo_fill_kind"))
        .collect();
    for ((_, kind_tag, _), label) in kinds.iter().zip(&fill_labels) {
        builder.emit(abi::compare_immediate(&tag, &kind_tag.to_string()));
        builder.emit(abi::branch_eq(label));
    }
    builder.emit(abi::branch(&done));
    for ((kind, _, record), label) in kinds.iter().zip(&fill_labels) {
        builder.emit(abi::label(label));
        let paint = builder.temporary_vreg();
        emit_field_block(builder, &paint, &rec, record, "paint")?;
        let view = PaintView {
            paint,
            paint_type: paint_type.clone(),
        };
        match kind {
            Kind::Rect => fill_rect(builder, &out, &rec, record, &view, false, &done)?,
            Kind::Rounded => fill_rect(builder, &out, &rec, record, &view, true, &done)?,
            Kind::Circle => fill_circle(builder, &out, &rec, record, &view, &done)?,
            Kind::Line => fill_line(builder, &out, &rec, record, &view, &done)?,
            Kind::Polygon => fill_polygon(builder, &out, &rec, record, &view, &done)?,
        }
    }
    builder.emit(abi::label(&done));

    let reg = builder.allocate_register();
    builder.emit(abi::load_u64(&reg, abi::stack_pointer(), list_slot));
    builder.emit(abi::move_register(RESULT_VALUE_REGISTER, &reg));
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
        text: "canvas.geoBuild".to_string(),
    })
}

/// `canvas::geoSame(a, b)`: the two `List OF Float`s hold the same count and the same
/// 64-bit pattern in every slot. A float `=` would call `-0.0` equal to `0.0` and a NaN
/// unequal to itself, and the `--debug` geometry check has to see both.
pub(crate) fn lower_geo_same(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    if args.len() < 2 {
        return Err(format!("'{symbol}' expects two float lists"));
    }
    let a = builder.temporary_vreg();
    let b = builder.temporary_vreg();
    builder.emit(abi::move_register(&a, &args[0].location));
    builder.emit(abi::move_register(&b, &args[1].location));
    let count = builder.temporary_vreg();
    let other = builder.temporary_vreg();
    let differ = builder.label("canvas_geo_same_differ");
    let done = builder.label("canvas_geo_same_done");
    builder.emit(abi::load_u64(&count, &a, COLLECTION_OFFSET_COUNT));
    builder.emit(abi::load_u64(&other, &b, COLLECTION_OFFSET_COUNT));
    builder.emit(abi::compare_registers(&count, &other));
    builder.emit(abi::branch_ne(&differ));
    // Entry-free fixed-width lists: word i at HEADER + 8*i.
    builder.emit(abi::add_immediate(&a, &a, COLLECTION_HEADER_SIZE));
    builder.emit(abi::add_immediate(&b, &b, COLLECTION_HEADER_SIZE));
    let head = builder.label("canvas_geo_same_head");
    let same = builder.label("canvas_geo_same_same");
    let lw = builder.temporary_vreg();
    let rw = builder.temporary_vreg();
    builder.emit(abi::label(&head));
    builder.emit(abi::compare_immediate(&count, "0"));
    builder.emit(abi::branch_eq(&same));
    builder.emit(abi::load_u64(&lw, &a, 0));
    builder.emit(abi::load_u64(&rw, &b, 0));
    builder.emit(abi::compare_registers(&lw, &rw));
    builder.emit(abi::branch_ne(&differ));
    builder.emit(abi::add_immediate(&a, &a, 8));
    builder.emit(abi::add_immediate(&b, &b, 8));
    builder.emit(abi::subtract_immediate(&count, &count, 1));
    builder.emit(abi::branch(&head));
    builder.emit(abi::label(&same));
    builder.emit(abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "1"));
    builder.emit(abi::branch(&done));
    builder.emit(abi::label(&differ));
    builder.emit(abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"));
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
        text: "canvas.geoSame".to_string(),
    })
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "geoBuild",
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
            return_type: ParameterType::list_of(ParameterType::Float),
            errors: vec!["ErrOutOfMemory"],
            body: Body::abi_function(lower_geo_build),
        }],
    });
    pkg.add_function(RegistryFunction {
        name: "geoSame",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: true,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "a",
                    desc: "",
                    aliases: &[],
                    ty: ParameterType::list_of(ParameterType::Float),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "b",
                    desc: "",
                    aliases: &[],
                    ty: ParameterType::list_of(ParameterType::Float),
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Boolean,
            errors: vec![],
            body: Body::abi_function(lower_geo_same),
        }],
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::registry::registry;

    /// The kind tags and the header length are MFBASIC `LET`s the emitter cannot read,
    /// so pin them against the package source (`.ai/canvas-threading.md`, "a layout
    /// constant shared by MFBASIC source and the emitter has no compiler between them").
    #[test]
    fn the_geometry_constants_match_the_mfbasic_source() {
        let source = registry()
            .resolve_package("canvas")
            .expect("canvas")
            .get_mfb_for(false);
        let value = |name: &str| -> f64 {
            let key = format!("LET {name} AS Integer = ");
            let at = source
                .find(&key)
                .unwrap_or_else(|| panic!("{name} not declared"));
            source[at + key.len()..]
                .split_whitespace()
                .next()
                .unwrap()
                .parse()
                .unwrap()
        };
        assert_eq!(value("__CANVAS_GEO_HEADER"), GEO_HEADER as f64);
        assert_eq!(value("__CANVAS_KIND_RECT"), KIND_RECT);
        assert_eq!(value("__CANVAS_KIND_CIRCLE"), KIND_CIRCLE);
        assert_eq!(value("__CANVAS_KIND_SEGMENT"), KIND_SEGMENT);
        assert_eq!(value("__CANVAS_GEO_POLYGON"), KIND_POLYGON);
        assert_eq!(value("__CANVAS_GEO_NONE"), KIND_NONE);
        assert_eq!(
            value("__CANVAS_GEO_GRADIENT_KIND"),
            SLOT_GRADIENT_KIND as f64
        );
        assert_eq!(value("__CANVAS_GEO_CAP"), SLOT_CAP as f64);
    }
}
