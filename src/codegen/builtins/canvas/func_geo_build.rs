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
//! `geoBuild` answers the 51-float header followed by its tail for a `Rectangle`,
//! `RoundedRect`, `Circle`, `Line`, `Polygon` or `PolyLine` whose paint has the all-zero
//! (identity) transform and fewer than two gradient stops — what an ordinary animated
//! item is. For
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
//! `Transform`, or a flat list such as `Polygon.contours`) holds an OFFSET from the record's
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
pub(super) const GEO_HEADER: usize = 67;
/// `__CANVAS_KIND_RECT`, `__CANVAS_KIND_CIRCLE`, `__CANVAS_KIND_SEGMENT`,
/// `__CANVAS_GEO_POLYGON`, `__CANVAS_GEO_NONE` — the slot-0 kind tags.
pub(super) const KIND_RECT: f64 = 0.0;
pub(super) const KIND_CIRCLE: f64 = 1.0;
pub(super) const KIND_SEGMENT: f64 = 2.0;
pub(super) const KIND_POLYGON: f64 = 4.0;
pub(super) const KIND_NONE: f64 = 5.0;
/// `__CANVAS_GEO_POLYLINE` (plan-154-A).
pub(super) const KIND_POLYLINE: f64 = 10.0;
/// `__CANVAS_GEO_GRADIENT_COUNT`, `__CANVAS_GEO_GRADIENT_KIND` .. `_TOY`, and
/// `__CANVAS_GEO_CAP`.
const SLOT_GRADIENT_COUNT: usize = 41;
const SLOT_GRADIENT_KIND: usize = 42;
const SLOT_CAP: usize = 34;
/// `__CANVAS_GEO_POLYGON_RULE` (plan-163-A): a polygon's fill rule, 1.0 for NonZero.
const SLOT_POLYGON_RULE: usize = 21;
/// `__CANVAS_GEO_JOIN_COUNT` and `__CANVAS_GEO_JOIN_REACH` (plan-163-B): the join
/// records in the tail, and how far one reaches from its vertex.
const SLOT_JOIN_COUNT: usize = 47;
const SLOT_JOIN_REACH: usize = 48;
/// `__CANVAS_GEO_DASH_COUNT` and `__CANVAS_GEO_DASH_PHASE` (plan-163-B): the dash tail's
/// length in floats, and the dash offset wrapped into one period.
const SLOT_DASH_COUNT: usize = 49;
const SLOT_DASH_PHASE: usize = 50;
/// `__CANVAS_GEO_CLIP_MODE` and `__CANVAS_GEO_CLIP_RULE` (plan-163-C): the clip's mode (0
/// none, 1 an axis-aligned rectangle in slots 22-25) and its FillRule ordinal.
const SLOT_CLIP_MODE: usize = 51;
const SLOT_CLIP_EDGE_COUNT: usize = 52;
const SLOT_CLIP_RULE: usize = 53;
/// `__CANVAS_GEO_POLYLINE_FIRST` / `_LAST`: a polyline's first and last segments with
/// length, in the gradient axis slots it does not otherwise use.
const SLOT_POLYLINE_FIRST: usize = 43;
const SLOT_POLYLINE_LAST: usize = 44;
/// The float32 bit pattern of 1.0, which the identity transform stores in slots 27 and
/// 30 (`__canvas_paintHeader`).
const IDENTITY_ONE_BITS: f64 = 1_065_353_216.0;
/// A polygon edge is `x0, y0, dx, dy, invLenSq`.
const EDGE_FLOATS: usize = 5;

// ---- Layout, read from the type model ----------------------------------------------

/// A canvas nominal as the type model keys it. The program's model registers the
/// builtin records package-qualified (`canvas.Paint`, `color.Color`); a helper compiled
/// inside the package may still spell one bare, so both are tried — the
/// package-qualified spelling FIRST. A bare spelling in the program's model is a USER
/// type (`.ai/codegen-invariants.md`, "Inside the compiler a BARE type spelling is
/// always a USER type"): a program's own `TYPE Circle` must not stand in for
/// `canvas.Circle`, whose layout the helper reads.
pub(super) fn record_type(
    builder: &CodeBuilder,
    ty: &ParameterType,
) -> Result<ParameterType, String> {
    let name = ty.name().into_owned();
    let bare = name.strip_prefix("canvas.").unwrap_or(&name).to_string();
    for spelling in [format!("canvas.{bare}"), name.clone(), bare] {
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

/// The union tag of `DrawItem` variant `name`, and its record type. Qualified first, as
/// in [`record_type`]: a bare `Circle` in the model is a user union's variant, with that
/// union's tag.
pub(super) fn variant(builder: &CodeBuilder, name: &str) -> Result<(usize, ParameterType), String> {
    for spelling in [format!("canvas.{name}"), name.to_string()] {
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
    // Qualified first, as in [`record_type`].
    for spelling in [format!("canvas.{bare}"), name.clone(), bare] {
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

/// Write a colour's four channels, `toFloat(toInt(channel))`, into four slots, the alpha
/// faded by the transparency step `step` (`__canvas_fadeAlpha`).
fn put_color(
    builder: &mut CodeBuilder,
    out: &VirtualRegister,
    first_slot: usize,
    view: &PaintView,
    which: &str,
    step: &VirtualRegister,
) -> Result<(), String> {
    let color = builder.temporary_vreg();
    let color_type = emit_field_block(builder, &color, &view.paint, &view.paint_type, which)?;
    for (k, channel) in ["red", "green", "blue", "alpha"].iter().enumerate() {
        let off = scalar_offset(builder, &color_type, channel)?;
        let byte = builder.temporary_vreg();
        builder.emit(abi::load_u8(&byte, &color, off));
        if k == 3 {
            emit_fade_alpha(builder, &byte, step);
        }
        let f = builder.temporary_fp_vreg();
        builder.emit(abi::signed_convert_to_float_d(&f, &byte));
        put(builder, out, first_slot + k, &f);
    }
    Ok(())
}

/// `__canvas_transparencyStep(paint)`: `toInt(maxF(minF(t, 1.0), 0.0) * 255.0 + 0.5)`,
/// the product and the add one `fmadd`.
fn emit_transparency_step(
    builder: &mut CodeBuilder,
    view: &PaintView,
) -> Result<VirtualRegister, String> {
    let t = fload(
        builder,
        &view.paint,
        scalar_offset(builder, &view.paint_type, "transparency")?,
    );
    let one = fconst(builder, 1.0);
    let zero = fconst(builder, 0.0);
    let m = fmin(builder, &t, &one);
    let c = fmax(builder, &m, &zero);
    let half = fconst(builder, 0.5);
    let scale = fconst(builder, 255.0);
    let x = fmadd(builder, &half, &c, &scale);
    let step = builder.temporary_vreg();
    builder.emit(abi::float_convert_to_signed_x(&step, &x));
    Ok(step)
}

/// `alpha = __canvas_fadeAlpha(alpha, step)`: `(alpha * (255 - step) + 127) / 255`.
fn emit_fade_alpha(builder: &mut CodeBuilder, alpha: &VirtualRegister, step: &VirtualRegister) {
    let keep = builder.temporary_vreg();
    builder.emit(abi::move_immediate(&keep, "Integer", "255"));
    builder.emit(abi::subtract_registers(&keep, &keep, step));
    builder.emit(abi::multiply_registers(alpha, alpha, &keep));
    builder.emit(abi::add_immediate(alpha, alpha, 127));
    let d = builder.temporary_vreg();
    builder.emit(abi::move_immediate(&d, "Integer", "255"));
    builder.emit(abi::signed_divide_registers(alpha, alpha, &d));
}

/// The paint's clip as `__canvas_clipHeader` classifies it on an untransformed item:
/// `mode` 0 for the empty ring list, 1 for one ring of four points that is an
/// axis-aligned rectangle (`__canvas_clipRect`: its sides alternate horizontal and
/// vertical, from either; its points in `xs`/`ys`), 2 for every other clip — a path.
struct ClipShape {
    mode: VirtualRegister,
    xs: Vec<VirtualRegister>,
    ys: Vec<VirtualRegister>,
}

fn emit_clip_shape(builder: &mut CodeBuilder, view: &PaintView) -> Result<ClipShape, String> {
    let rings = builder.temporary_vreg();
    let rings_type = emit_field_block(builder, &rings, &view.paint, &view.paint_type, "clip")?;
    let count = builder.temporary_vreg();
    builder.emit(abi::load_u64(&count, &rings, COLLECTION_OFFSET_COUNT));
    let mode = builder.temporary_vreg();
    builder.emit(abi::move_immediate(&mode, "Integer", "0"));
    let xs: Vec<VirtualRegister> = (0..4).map(|_| builder.temporary_fp_vreg()).collect();
    let ys: Vec<VirtualRegister> = (0..4).map(|_| builder.temporary_fp_vreg()).collect();
    let zero = fconst(builder, 0.0);
    for v in xs.iter().chain(ys.iter()) {
        builder.emit(abi::float_move_d_from_d(v, &zero));
    }
    let done = builder.label("canvas_geo_clip_shape_done");
    builder.emit(abi::compare_immediate(&count, "0"));
    builder.emit(abi::branch_eq(&done));
    builder.emit(abi::move_immediate(&mode, "Integer", "2"));
    builder.emit(abi::compare_immediate(&count, "1"));
    builder.emit(abi::branch_ne(&done));
    let outer = emit_list_view(builder, &rings, &rings_type)?;
    let ring_type = outer.element.clone();
    let (xo, yo) = ring_point_offsets(builder, &ring_type)?;
    let ring = builder.temporary_vreg();
    let first = builder.temporary_vreg();
    builder.emit(abi::move_immediate(&first, "Integer", "0"));
    emit_list_element(builder, &ring, &outer, &first);
    let n = builder.temporary_vreg();
    builder.emit(abi::load_u64(&n, &ring, COLLECTION_OFFSET_COUNT));
    builder.emit(abi::compare_immediate(&n, "4"));
    builder.emit(abi::branch_ne(&done));
    let list = emit_list_view(builder, &ring, &ring_type)?;
    for k in 0..4usize {
        let idx = builder.temporary_vreg();
        builder.emit(abi::move_immediate(&idx, "Integer", &k.to_string()));
        let q = builder.temporary_vreg();
        emit_list_element(builder, &q, &list, &idx);
        let x = fload(builder, &q, xo);
        let y = fload(builder, &q, yo);
        builder.emit(abi::float_move_d_from_d(&xs[k], &x));
        builder.emit(abi::float_move_d_from_d(&ys[k], &y));
    }
    // across: a.y = b.y, b.x = c.x, c.y = d.y, d.x = a.x; down: the other axis first.
    let down = builder.label("canvas_geo_clip_down");
    let rect = builder.label("canvas_geo_clip_rect");
    for (l, r) in [
        (&ys[0], &ys[1]),
        (&xs[1], &xs[2]),
        (&ys[2], &ys[3]),
        (&xs[3], &xs[0]),
    ] {
        builder.emit(abi::float_compare_d(l, r));
        builder.emit(abi::branch_ne(&down));
    }
    builder.emit(abi::branch(&rect));
    builder.emit(abi::label(&down));
    for (l, r) in [
        (&xs[0], &xs[1]),
        (&ys[1], &ys[2]),
        (&xs[2], &xs[3]),
        (&ys[3], &ys[0]),
    ] {
        builder.emit(abi::float_compare_d(l, r));
        builder.emit(abi::branch_ne(&done));
    }
    builder.emit(abi::label(&rect));
    builder.emit(abi::move_immediate(&mode, "Integer", "1"));
    builder.emit(abi::label(&done));
    Ok(ClipShape { mode, xs, ys })
}

/// Walk a path clip's contributing rings (of at least 2 points), in order, calling
/// `point` with each point's address — `__canvas_clipHeader`'s and
/// `__canvas_polygonEdges`'s walk.
fn emit_clip_rings(
    builder: &mut CodeBuilder,
    view: &PaintView,
    mut ring_body: impl FnMut(&mut CodeBuilder, &ListView, &VirtualRegister, usize, usize),
) -> Result<(), String> {
    let rings = builder.temporary_vreg();
    let rings_type = emit_field_block(builder, &rings, &view.paint, &view.paint_type, "clip")?;
    let count = builder.temporary_vreg();
    builder.emit(abi::load_u64(&count, &rings, COLLECTION_OFFSET_COUNT));
    let outer = emit_list_view(builder, &rings, &rings_type)?;
    let ring_type = outer.element.clone();
    let (xo, yo) = ring_point_offsets(builder, &ring_type)?;
    let r = builder.temporary_vreg();
    let ring = builder.temporary_vreg();
    let n = builder.temporary_vreg();
    let head = builder.label("canvas_geo_clip_ring");
    let next = builder.label("canvas_geo_clip_ring_next");
    let walked = builder.label("canvas_geo_clip_rings_walked");
    builder.emit(abi::move_immediate(&r, "Integer", "0"));
    builder.emit(abi::label(&head));
    builder.emit(abi::compare_registers(&r, &count));
    builder.emit(abi::branch_ge(&walked));
    emit_list_element(builder, &ring, &outer, &r);
    builder.emit(abi::load_u64(&n, &ring, COLLECTION_OFFSET_COUNT));
    builder.emit(abi::compare_immediate(&n, "2"));
    builder.emit(abi::branch_lt(&next));
    let list = emit_list_view(builder, &ring, &ring_type)?;
    ring_body(builder, &list, &n, xo, yo);
    builder.emit(abi::label(&next));
    builder.emit(abi::add_immediate(&r, &r, 1));
    builder.emit(abi::branch(&head));
    builder.emit(abi::label(&walked));
    Ok(())
}

/// The points of a path clip's contributing rings: `__canvas_clipHeader`'s count.
fn emit_clip_point_count(
    builder: &mut CodeBuilder,
    view: &PaintView,
) -> Result<VirtualRegister, String> {
    let total = builder.temporary_vreg();
    builder.emit(abi::move_immediate(&total, "Integer", "0"));
    emit_clip_rings(builder, view, |builder, _, n, _, _| {
        builder.emit(abi::add_registers(&total, &total, n));
    })?;
    Ok(total)
}

/// `n += 5 * points` when the paint's clip is a path (mode 2).
fn emit_clip_size(
    builder: &mut CodeBuilder,
    view: &PaintView,
    n: &VirtualRegister,
) -> Result<(), String> {
    let shape = emit_clip_shape(builder, view)?;
    let skip = builder.label("canvas_geo_size_clip_skip");
    builder.emit(abi::compare_immediate(&shape.mode, "2"));
    builder.emit(abi::branch_ne(&skip));
    let points = emit_clip_point_count(builder, view)?;
    let five = builder.temporary_vreg();
    builder.emit(abi::move_immediate(
        &five,
        "Integer",
        &EDGE_FLOATS.to_string(),
    ));
    builder.emit(abi::multiply_registers(&points, &points, &five));
    builder.emit(abi::add_registers(n, n, &points));
    builder.emit(abi::label(&skip));
    Ok(())
}

/// `__canvas_withClipEdges` for a native record: when slot 51 says a path clip with
/// edges, its rings' edges (`__canvas_polygonEdges`) at `cursor`, which moves past them,
/// and slot 1 grown by their floats.
fn emit_clip_edges(
    builder: &mut CodeBuilder,
    out: &VirtualRegister,
    view: &PaintView,
    cursor: &VirtualRegister,
) -> Result<(), String> {
    let skip = builder.label("canvas_geo_clip_edges_skip");
    let mode = fload(builder, out, SLOT_CLIP_MODE * 8);
    let two = fconst(builder, 2.0);
    builder.emit(abi::float_compare_d(&mode, &two));
    builder.emit(abi::branch_ne(&skip));
    let start = builder.temporary_vreg();
    builder.emit(abi::move_register(&start, cursor));
    emit_clip_rings(builder, view, |builder, list, n, xo, yo| {
        let i = builder.temporary_vreg();
        let j = builder.temporary_vreg();
        let a = builder.temporary_vreg();
        let b = builder.temporary_vreg();
        let head = builder.label("canvas_geo_clip_edge");
        let wrap = builder.label("canvas_geo_clip_edge_wrap");
        let done = builder.label("canvas_geo_clip_edges_done");
        builder.emit(abi::move_immediate(&i, "Integer", "0"));
        builder.emit(abi::label(&head));
        builder.emit(abi::compare_registers(&i, n));
        builder.emit(abi::branch_ge(&done));
        builder.emit(abi::add_immediate(&j, &i, 1));
        builder.emit(abi::compare_registers(&j, n));
        builder.emit(abi::branch_lt(&wrap));
        builder.emit(abi::move_immediate(&j, "Integer", "0"));
        builder.emit(abi::label(&wrap));
        emit_list_element(builder, &a, list, &i);
        emit_list_element(builder, &b, list, &j);
        let ax = fload(builder, &a, xo);
        let ay = fload(builder, &a, yo);
        let bx = fload(builder, &b, xo);
        let by = fload(builder, &b, yo);
        let dx = fsub(builder, &bx, &ax);
        let dy = fsub(builder, &by, &ay);
        let len_sq = fused_len_sq(builder, &dx, &dy);
        builder.emit(abi::store_double(&ax, cursor, 0));
        builder.emit(abi::store_double(&ay, cursor, 8));
        builder.emit(abi::store_double(&dx, cursor, 16));
        builder.emit(abi::store_double(&dy, cursor, 24));
        let inv = builder.temporary_fp_vreg();
        let zero = fconst(builder, 0.0);
        builder.emit(abi::float_move_d_from_d(&inv, &zero));
        let stored = builder.label("canvas_geo_clip_inv_store");
        let positive = builder.label("canvas_geo_clip_positive");
        builder.emit(abi::float_compare_d(&len_sq, &zero));
        builder.emit(abi::branch_gt(&positive));
        builder.emit(abi::branch(&stored));
        builder.emit(abi::label(&positive));
        let one = fconst(builder, 1.0);
        builder.emit(abi::float_divide_d(&inv, &one, &len_sq));
        builder.emit(abi::label(&stored));
        builder.emit(abi::store_double(&inv, cursor, 32));
        builder.emit(abi::add_immediate(cursor, cursor, EDGE_FLOATS * 8));
        builder.emit(abi::add_immediate(&i, &i, 1));
        builder.emit(abi::branch(&head));
        builder.emit(abi::label(&done));
    })?;
    let floats = builder.temporary_vreg();
    builder.emit(abi::subtract_registers(&floats, cursor, &start));
    builder.emit(abi::shift_right_immediate(&floats, &floats, 3));
    let floats_f = builder.temporary_fp_vreg();
    builder.emit(abi::signed_convert_to_float_d(&floats_f, &floats));
    let length = fload(builder, out, 8);
    let grown = fadd(builder, &length, &floats_f);
    put(builder, out, 1, &grown);
    builder.emit(abi::label(&skip));
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
    // plan-163-C: both alphas faded by `transparency` (`__canvas_fadeAlpha`).
    let step = emit_transparency_step(builder, view)?;
    put_color(builder, out, 8, view, "fill", &step)?;
    put_color(builder, out, 12, view, "stroke", &step)?;

    // plan-163-C: `__canvas_clipHeader` on an untransformed item -- nothing for the empty
    // ring list; else the rule, then the rectangle in slots 22-25 (mode 1) or a path's
    // point count and its bounds a pixel wider each way (mode 2; zero bounds and no
    // count when no ring contributes).
    let unclipped = builder.label("canvas_geo_unclipped");
    let shape = emit_clip_shape(builder, view)?;
    builder.emit(abi::compare_immediate(&shape.mode, "0"));
    builder.emit(abi::branch_eq(&unclipped));
    let (rule_index, rule_type) = field(builder, &view.paint_type, "clipRule")?;
    let non_zero = enum_ordinal(builder, &rule_type, "NonZero")?;
    let rule = builder.temporary_vreg();
    builder.emit(abi::load_u64(&rule, &view.paint, 8 * rule_index));
    let ruled = builder.label("canvas_geo_clip_ruled");
    builder.emit(abi::compare_immediate(&rule, &non_zero.to_string()));
    builder.emit(abi::branch_ne(&ruled));
    put_const(builder, out, SLOT_CLIP_RULE, 1.0);
    builder.emit(abi::label(&ruled));
    let path = builder.label("canvas_geo_clip_path");
    builder.emit(abi::compare_immediate(&shape.mode, "1"));
    builder.emit(abi::branch_ne(&path));
    let x0 = fmin(builder, &shape.xs[0], &shape.xs[2]);
    let y0 = fmin(builder, &shape.ys[0], &shape.ys[2]);
    let x1 = fmax(builder, &shape.xs[0], &shape.xs[2]);
    let y1 = fmax(builder, &shape.ys[0], &shape.ys[2]);
    put(builder, out, 22, &x0);
    put(builder, out, 23, &y0);
    put(builder, out, 24, &x1);
    put(builder, out, 25, &y1);
    put_const(builder, out, SLOT_CLIP_MODE, 1.0);
    builder.emit(abi::branch(&unclipped));
    builder.emit(abi::label(&path));
    put_const(builder, out, SLOT_CLIP_MODE, 2.0);
    let seen = builder.temporary_vreg();
    builder.emit(abi::move_immediate(&seen, "Integer", "0"));
    let bounds: Vec<VirtualRegister> = (0..4).map(|_| builder.temporary_fp_vreg()).collect();
    let zero = fconst(builder, 0.0);
    for v in &bounds {
        builder.emit(abi::float_move_d_from_d(v, &zero));
    }
    emit_clip_rings(builder, view, |builder, list, n, xo, yo| {
        let i = builder.temporary_vreg();
        let q = builder.temporary_vreg();
        let head = builder.label("canvas_geo_clip_bound");
        let seeded = builder.label("canvas_geo_clip_bound_seeded");
        let next = builder.label("canvas_geo_clip_bound_next");
        let done = builder.label("canvas_geo_clip_bounds_done");
        builder.emit(abi::move_immediate(&i, "Integer", "0"));
        builder.emit(abi::label(&head));
        builder.emit(abi::compare_registers(&i, n));
        builder.emit(abi::branch_ge(&done));
        emit_list_element(builder, &q, list, &i);
        let x = fload(builder, &q, xo);
        let y = fload(builder, &q, yo);
        builder.emit(abi::compare_immediate(&seen, "0"));
        builder.emit(abi::branch_ne(&seeded));
        builder.emit(abi::float_move_d_from_d(&bounds[0], &x));
        builder.emit(abi::float_move_d_from_d(&bounds[2], &x));
        builder.emit(abi::float_move_d_from_d(&bounds[1], &y));
        builder.emit(abi::float_move_d_from_d(&bounds[3], &y));
        builder.emit(abi::branch(&next));
        builder.emit(abi::label(&seeded));
        let v = fmin(builder, &bounds[0], &x);
        builder.emit(abi::float_move_d_from_d(&bounds[0], &v));
        let v = fmax(builder, &bounds[2], &x);
        builder.emit(abi::float_move_d_from_d(&bounds[2], &v));
        let v = fmin(builder, &bounds[1], &y);
        builder.emit(abi::float_move_d_from_d(&bounds[1], &v));
        let v = fmax(builder, &bounds[3], &y);
        builder.emit(abi::float_move_d_from_d(&bounds[3], &v));
        builder.emit(abi::label(&next));
        builder.emit(abi::add_immediate(&seen, &seen, 1));
        builder.emit(abi::add_immediate(&i, &i, 1));
        builder.emit(abi::branch(&head));
        builder.emit(abi::label(&done));
    })?;
    builder.emit(abi::compare_immediate(&seen, "0"));
    builder.emit(abi::branch_eq(&unclipped));
    let seen_f = builder.temporary_fp_vreg();
    builder.emit(abi::signed_convert_to_float_d(&seen_f, &seen));
    put(builder, out, SLOT_CLIP_EDGE_COUNT, &seen_f);
    let one = fconst(builder, 1.0);
    let bx0 = fsub(builder, &bounds[0], &one);
    let by0 = fsub(builder, &bounds[1], &one);
    let bx1 = fadd(builder, &bounds[2], &one);
    let by1 = fadd(builder, &bounds[3], &one);
    put(builder, out, 22, &bx0);
    put(builder, out, 23, &by0);
    put(builder, out, 24, &bx1);
    put(builder, out, 25, &by1);
    builder.emit(abi::label(&unclipped));

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
        "gradient",
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

/// `__canvas_capTag(paint.cap)` as a whole number: Round 1, Square 2, Butt 0.
fn emit_cap_tag(builder: &mut CodeBuilder, view: &PaintView) -> Result<VirtualRegister, String> {
    let (cap_index, cap_type) = field(builder, &view.paint_type, "cap")?;
    let round = enum_ordinal(builder, &cap_type, "Round")?;
    let square = enum_ordinal(builder, &cap_type, "Square")?;
    let raw = builder.temporary_vreg();
    builder.emit(abi::load_u64(&raw, &view.paint, 8 * cap_index));
    let cap = builder.temporary_vreg();
    let not_round = builder.label("canvas_geo_cap_not_round");
    let not_square = builder.label("canvas_geo_cap_not_square");
    let done = builder.label("canvas_geo_cap_tag_done");
    builder.emit(abi::compare_immediate(&raw, &round.to_string()));
    builder.emit(abi::branch_ne(&not_round));
    builder.emit(abi::move_immediate(&cap, "Integer", "1"));
    builder.emit(abi::branch(&done));
    builder.emit(abi::label(&not_round));
    builder.emit(abi::compare_immediate(&raw, &square.to_string()));
    builder.emit(abi::branch_ne(&not_square));
    builder.emit(abi::move_immediate(&cap, "Integer", "2"));
    builder.emit(abi::branch(&done));
    builder.emit(abi::label(&not_square));
    builder.emit(abi::move_immediate(&cap, "Integer", "0"));
    builder.emit(abi::label(&done));
    Ok(cap)
}

/// `__canvas_capReach(half, cap) + 1.0`: the bounds pad of a stroke-as-fill kind, with a
/// Square cap's corner a diagonal half-width out (`half * 1.4142135623730951`).
fn cap_pad(
    builder: &mut CodeBuilder,
    half: &VirtualRegister,
    cap: &VirtualRegister,
) -> VirtualRegister {
    let reach = builder.temporary_fp_vreg();
    builder.emit(abi::float_move_d_from_d(&reach, half));
    let plain = builder.label("canvas_geo_cap_reach_plain");
    builder.emit(abi::compare_immediate(cap, "2"));
    builder.emit(abi::branch_ne(&plain));
    let root2 = fconst(builder, std::f64::consts::SQRT_2);
    let wide = builder.temporary_fp_vreg();
    builder.emit(abi::float_multiply_d(&wide, half, &root2));
    builder.emit(abi::float_move_d_from_d(&reach, &wide));
    builder.emit(abi::label(&plain));
    let one = fconst(builder, 1.0);
    fadd(builder, &reach, &one)
}

/// `__canvas_dashCapReach(half, paint)` for `half = maxF(strokeHalf, 0.0)`: `half *
/// 1.4142135623730951` for a dashed Square paint, else `half`.
fn dash_cap_reach(
    builder: &mut CodeBuilder,
    m: &VirtualRegister,
    style: &JoinStyle,
) -> VirtualRegister {
    let reach = builder.temporary_fp_vreg();
    builder.emit(abi::float_move_d_from_d(&reach, m));
    let plain = builder.label("canvas_geo_dash_reach_plain");
    builder.emit(abi::compare_immediate(&style.dashed, "0"));
    builder.emit(abi::branch_eq(&plain));
    builder.emit(abi::compare_immediate(&style.cap, "2"));
    builder.emit(abi::branch_ne(&plain));
    let root2 = fconst(builder, std::f64::consts::SQRT_2);
    let wide = fmul(builder, m, &root2);
    builder.emit(abi::float_move_d_from_d(&reach, &wide));
    builder.emit(abi::label(&plain));
    reach
}

/// `__canvas_strokePad(paint)`: the dash-cap reach of `maxF(strokeHalf, 0.0)` plus 1.0,
/// or for a Miter join, unless that reach is the longer, `half * limit + 1.0` (one
/// `fmadd`).
fn stroke_pad(
    builder: &mut CodeBuilder,
    stroke_half: &VirtualRegister,
    style: &JoinStyle,
) -> VirtualRegister {
    let zero = fconst(builder, 0.0);
    let m = fmax(builder, stroke_half, &zero);
    let reach = dash_cap_reach(builder, &m, style);
    let one = fconst(builder, 1.0);
    let pad = builder.temporary_fp_vreg();
    let plain = builder.label("canvas_geo_pad_plain");
    let done = builder.label("canvas_geo_pad_done");
    builder.emit(abi::compare_immediate(&style.tag, "1"));
    builder.emit(abi::branch_ne(&plain));
    let tip = fmul(builder, &m, &style.limit);
    builder.emit(abi::float_compare_d(&reach, &tip));
    builder.emit(abi::branch_gt(&plain));
    builder.emit(abi::float_multiply_add_d(&pad, &one, &m, &style.limit));
    builder.emit(abi::branch(&done));
    builder.emit(abi::label(&plain));
    let v = fadd(builder, &reach, &one);
    builder.emit(abi::float_move_d_from_d(&pad, &v));
    builder.emit(abi::label(&done));
    pad
}

/// A polyline's pad, `__canvas_maxF(__canvas_capReach(half, cap),
/// __canvas_joinReach(half, paint)) + 1.0`.
fn polyline_pad(
    builder: &mut CodeBuilder,
    half: &VirtualRegister,
    cap: &VirtualRegister,
    style: &JoinStyle,
) -> VirtualRegister {
    let cap_reach = builder.temporary_fp_vreg();
    builder.emit(abi::float_move_d_from_d(&cap_reach, half));
    let plain = builder.label("canvas_geo_pl_reach_plain");
    builder.emit(abi::compare_immediate(cap, "2"));
    builder.emit(abi::branch_ne(&plain));
    let root2 = fconst(builder, std::f64::consts::SQRT_2);
    let wide = fmul(builder, half, &root2);
    builder.emit(abi::float_move_d_from_d(&cap_reach, &wide));
    builder.emit(abi::label(&plain));
    let join_reach = builder.temporary_fp_vreg();
    builder.emit(abi::float_move_d_from_d(&join_reach, half));
    let round = builder.label("canvas_geo_pl_reach_round");
    builder.emit(abi::compare_immediate(&style.tag, "1"));
    builder.emit(abi::branch_ne(&round));
    let tip = fmul(builder, half, &style.limit);
    builder.emit(abi::float_move_d_from_d(&join_reach, &tip));
    builder.emit(abi::label(&round));
    let m = fmax(builder, &cap_reach, &join_reach);
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
        "gradient",
    )?;
    let stops = builder.temporary_vreg();
    emit_field_block(builder, &stops, &gradient, &gradient_type, "stops")?;
    let count = builder.temporary_vreg();
    builder.emit(abi::load_u64(&count, &stops, COLLECTION_OFFSET_COUNT));
    builder.emit(abi::compare_immediate(&count, "2"));
    builder.emit(abi::branch_ge(unsupported));
    Ok(())
}

// ---- Joins (plan-163-B) ---------------------------------------------------------------
//
// `helper_joins.rs`'s `__canvas_joinTail` and `__canvas_joinRecord`, instruction for
// instruction: one 8-float record per edge, the corner at its start. The MFBASIC
// arithmetic passes through `opt::fma_fusion`, which fuses the FIRST single-use product
// of an add or subtract (`a*b + c*d` becomes `fmadd(round(c*d), a, b)`, `a*b - c*d`
// becomes `fmsub(round(c*d), a, b)`, and `v + n*h` becomes `fmadd(v, n, h)`), so the
// fused forms are written out here.

/// A join record's length in floats: the four points `V, A, M, B`.
const JOIN_FLOATS: usize = 8;

/// A paint's join: `__canvas_joinTag` (Round 0, Miter 1, Bevel 2) and the miter limit
/// applied, `__canvas_maxF(paint.miterLimit, 1.0)`; and what decides its dash tail —
/// whether the stroke of half-width `half` is dashed (1) or solid (0), and its cap tag.
struct JoinStyle {
    tag: VirtualRegister,
    limit: VirtualRegister,
    dashed: VirtualRegister,
    cap: VirtualRegister,
}

fn emit_join_style(
    builder: &mut CodeBuilder,
    view: &PaintView,
    half: &VirtualRegister,
) -> Result<JoinStyle, String> {
    let (join_index, join_type) = field(builder, &view.paint_type, "join")?;
    let miter = enum_ordinal(builder, &join_type, "Miter")?;
    let bevel = enum_ordinal(builder, &join_type, "Bevel")?;
    let raw = builder.temporary_vreg();
    builder.emit(abi::load_u64(&raw, &view.paint, 8 * join_index));
    let tag = builder.temporary_vreg();
    let not_miter = builder.label("canvas_geo_join_not_miter");
    let not_bevel = builder.label("canvas_geo_join_not_bevel");
    let done = builder.label("canvas_geo_join_tag_done");
    builder.emit(abi::compare_immediate(&raw, &miter.to_string()));
    builder.emit(abi::branch_ne(&not_miter));
    builder.emit(abi::move_immediate(&tag, "Integer", "1"));
    builder.emit(abi::branch(&done));
    builder.emit(abi::label(&not_miter));
    builder.emit(abi::compare_immediate(&raw, &bevel.to_string()));
    builder.emit(abi::branch_ne(&not_bevel));
    builder.emit(abi::move_immediate(&tag, "Integer", "2"));
    builder.emit(abi::branch(&done));
    builder.emit(abi::label(&not_bevel));
    builder.emit(abi::move_immediate(&tag, "Integer", "0"));
    builder.emit(abi::label(&done));
    let limit_raw = fload(
        builder,
        &view.paint,
        scalar_offset(builder, &view.paint_type, "miterLimit")?,
    );
    let one = fconst(builder, 1.0);
    let limit = fmax(builder, &limit_raw, &one);
    let dashed = builder.temporary_vreg();
    builder.emit(abi::move_immediate(&dashed, "Integer", "0"));
    let solid = builder.label("canvas_geo_style_solid");
    emit_dash_pattern(builder, view, half, &solid)?;
    builder.emit(abi::move_immediate(&dashed, "Integer", "1"));
    builder.emit(abi::label(&solid));
    let cap = emit_cap_tag(builder, view)?;
    Ok(JoinStyle {
        tag,
        limit,
        dashed,
        cap,
    })
}

fn fmul(b: &mut CodeBuilder, l: &VirtualRegister, r: &VirtualRegister) -> VirtualRegister {
    fbin(b, |d, l, r| abi::float_multiply_d(d, l, r), l, r)
}

/// `v + n * h`, as the fusion pass lowers it: `fmadd(v, n, h)`.
fn fmadd(
    builder: &mut CodeBuilder,
    v: &VirtualRegister,
    n: &VirtualRegister,
    h: &VirtualRegister,
) -> VirtualRegister {
    let dst = builder.temporary_fp_vreg();
    builder.emit(abi::float_multiply_add_d(&dst, v, n, h));
    dst
}

/// Branch to `dead` unless `dx * dx + dy * dy > 0.0` (the fused length test every edge
/// walk uses).
fn branch_unless_live(
    builder: &mut CodeBuilder,
    dx: &VirtualRegister,
    dy: &VirtualRegister,
    dead: &str,
) {
    let len_sq = fused_len_sq(builder, dx, dy);
    let zero = fconst(builder, 0.0);
    let live = builder.label("canvas_geo_join_live");
    builder.emit(abi::float_compare_d(&len_sq, &zero));
    builder.emit(abi::branch_gt(&live));
    builder.emit(abi::branch(dead));
    builder.emit(abi::label(&live));
}

/// Store the four points `p` at `cursor` and move `cursor` past them.
fn emit_store_join(builder: &mut CodeBuilder, cursor: &VirtualRegister, p: [&VirtualRegister; 8]) {
    for (k, value) in p.iter().enumerate() {
        builder.emit(abi::store_double(*value, cursor, k * 8));
    }
    builder.emit(abi::add_immediate(cursor, cursor, JOIN_FLOATS * 8));
}

/// `__canvas_joinRecord`: the corner at `v` between the incoming edge `a` and the
/// outgoing `b`, written at `cursor` (which then moves past it) as `V, A, P, B`: the disc
/// alone (`A = B`) straight on, no piece (`V` four times) straight back, a miter `V, A,
/// M, B`, or a bevel `V, A, V, B`.
#[allow(clippy::too_many_arguments)]
fn emit_join_record(
    builder: &mut CodeBuilder,
    cursor: &VirtualRegister,
    vx: &VirtualRegister,
    vy: &VirtualRegister,
    ax: &VirtualRegister,
    ay: &VirtualRegister,
    bx: &VirtualRegister,
    by: &VirtualRegister,
    half: &VirtualRegister,
    style: &JoinStyle,
) {
    let la = fused_len_sq(builder, ax, ay);
    let l1 = builder.temporary_fp_vreg();
    builder.emit(abi::float_sqrt_d(&l1, &la));
    let lb = fused_len_sq(builder, bx, by);
    let l2 = builder.temporary_fp_vreg();
    builder.emit(abi::float_sqrt_d(&l2, &lb));
    let u1x = fdiv(builder, ax, &l1);
    let u1y = fdiv(builder, ay, &l1);
    let u2x = fdiv(builder, bx, &l2);
    let u2y = fdiv(builder, by, &l2);
    // `u1x * u2y - u1y * u2x`.
    let p = fmul(builder, &u1y, &u2x);
    let cross = builder.temporary_fp_vreg();
    builder.emit(abi::float_multiply_sub_d(&cross, &p, &u1x, &u2y));
    let zero = fconst(builder, 0.0);
    // The outer side: right of the edges, or left when `cross < 0.0`.
    let n1x = builder.temporary_fp_vreg();
    let n1y = builder.temporary_fp_vreg();
    let n2x = builder.temporary_fp_vreg();
    let n2y = builder.temporary_fp_vreg();
    builder.emit(abi::float_move_d_from_d(&n1x, &u1y));
    let v = fsub(builder, &zero, &u1x);
    builder.emit(abi::float_move_d_from_d(&n1y, &v));
    builder.emit(abi::float_move_d_from_d(&n2x, &u2y));
    let v = fsub(builder, &zero, &u2x);
    builder.emit(abi::float_move_d_from_d(&n2y, &v));
    let sided = builder.label("canvas_geo_join_sided");
    let left = builder.label("canvas_geo_join_left");
    builder.emit(abi::float_compare_d(&cross, &zero));
    builder.emit(abi::branch_mi(&left));
    builder.emit(abi::branch(&sided));
    builder.emit(abi::label(&left));
    let v = fsub(builder, &zero, &u1y);
    builder.emit(abi::float_move_d_from_d(&n1x, &v));
    builder.emit(abi::float_move_d_from_d(&n1y, &u1x));
    let v = fsub(builder, &zero, &u2y);
    builder.emit(abi::float_move_d_from_d(&n2x, &v));
    builder.emit(abi::float_move_d_from_d(&n2y, &u2x));
    builder.emit(abi::label(&sided));
    // `u1x * u2x + u1y * u2y`.
    let q = fmul(builder, &u1y, &u2y);
    let dot = fmadd(builder, &q, &u1x, &u2x);
    // The incoming band's outer corner A.
    let ex = fmadd(builder, vx, &n1x, half);
    let ey = fmadd(builder, vy, &n1y, half);
    let written = builder.label("canvas_geo_join_written");
    let turned = builder.label("canvas_geo_join_turned");
    let back = builder.label("canvas_geo_join_back");
    // A dashed Round paint's corner: the disc alone, whatever the turn.
    let shaped = builder.label("canvas_geo_join_shaped");
    builder.emit(abi::compare_immediate(&style.tag, "0"));
    builder.emit(abi::branch_ne(&shaped));
    emit_store_join(builder, cursor, [vx, vy, &ex, &ey, &ex, &ey, &ex, &ey]);
    builder.emit(abi::branch(&written));
    builder.emit(abi::label(&shaped));
    builder.emit(abi::float_compare_d(&cross, &zero));
    builder.emit(abi::branch_ne(&turned));
    // Parallel: the disc alone straight on, no piece straight back.
    builder.emit(abi::float_compare_d(&dot, &zero));
    let ahead = builder.label("canvas_geo_join_ahead");
    builder.emit(abi::branch_gt(&ahead));
    builder.emit(abi::branch(&back));
    builder.emit(abi::label(&ahead));
    emit_store_join(builder, cursor, [vx, vy, &ex, &ey, &ex, &ey, &ex, &ey]);
    builder.emit(abi::branch(&written));
    builder.emit(abi::label(&back));
    emit_store_join(builder, cursor, [vx, vy, vx, vy, vx, vy, vx, vy]);
    builder.emit(abi::branch(&written));
    builder.emit(abi::label(&turned));
    // The outgoing band's outer corner B.
    let fx = fmadd(builder, vx, &n2x, half);
    let fy = fmadd(builder, vy, &n2y, half);
    let one = fconst(builder, 1.0);
    let bevel = builder.label("canvas_geo_join_bevel");
    builder.emit(abi::compare_immediate(&style.tag, "1"));
    builder.emit(abi::branch_ne(&bevel));
    // `(1.0 + dot) * limit * limit >= 2.0`.
    let e = fadd(builder, &one, &dot);
    let p1 = fmul(builder, &e, &style.limit);
    let p2 = fmul(builder, &p1, &style.limit);
    let two = fconst(builder, 2.0);
    let mitered = builder.label("canvas_geo_join_miter");
    builder.emit(abi::float_compare_d(&p2, &two));
    builder.emit(abi::branch_ge(&mitered));
    builder.emit(abi::branch(&bevel));
    builder.emit(abi::label(&mitered));
    // `k = half / (1.0 + dot)`, the point `V + (n1 + n2) * k`.
    let e2 = fadd(builder, &one, &dot);
    let k = fdiv(builder, half, &e2);
    let sx = fadd(builder, &n1x, &n2x);
    let mx = fmadd(builder, vx, &sx, &k);
    let sy = fadd(builder, &n1y, &n2y);
    let my = fmadd(builder, vy, &sy, &k);
    emit_store_join(builder, cursor, [vx, vy, &ex, &ey, &mx, &my, &fx, &fy]);
    builder.emit(abi::branch(&written));
    builder.emit(abi::label(&bevel));
    emit_store_join(builder, cursor, [vx, vy, &ex, &ey, vx, vy, &fx, &fy]);
    builder.emit(abi::label(&written));
}

/// `__canvas_joinTail` over one ring: the `n` edges at `edges` (the tail layout, 40
/// bytes each), one record per edge at `cursor`. A live edge joins the live edge before
/// it; a `closed` ring with at least two live edges joins its first live edge to its
/// last; every other edge takes the no-join record.
#[allow(clippy::too_many_arguments)]
fn emit_ring_joins(
    builder: &mut CodeBuilder,
    edges: &VirtualRegister,
    n: &VirtualRegister,
    closed: bool,
    cursor: &VirtualRegister,
    half: &VirtualRegister,
    style: &JoinStyle,
) {
    let i = builder.temporary_vreg();
    let prev = builder.temporary_vreg();
    let live = builder.temporary_vreg();
    let at = builder.temporary_vreg();
    let forty = builder.temporary_vreg();
    builder.emit(abi::move_immediate(&forty, "Integer", "40"));
    // -1 as `0 - 1`: the immediate encoder takes no negative literal.
    builder.emit(abi::move_immediate(&prev, "Integer", "0"));
    builder.emit(abi::subtract_immediate(&prev, &prev, 1));
    builder.emit(abi::move_immediate(&live, "Integer", "0"));
    // The ring's last live edge and how many there are.
    let scan = builder.label("canvas_geo_ring_join_scan");
    let scan_next = builder.label("canvas_geo_ring_join_scan_next");
    let scanned = builder.label("canvas_geo_ring_join_scanned");
    builder.emit(abi::move_immediate(&i, "Integer", "0"));
    builder.emit(abi::label(&scan));
    builder.emit(abi::compare_registers(&i, n));
    builder.emit(abi::branch_ge(&scanned));
    builder.emit(abi::multiply_registers(&at, &i, &forty));
    builder.emit(abi::add_registers(&at, edges, &at));
    let dx = fload(builder, &at, 16);
    let dy = fload(builder, &at, 24);
    branch_unless_live(builder, &dx, &dy, &scan_next);
    builder.emit(abi::move_register(&prev, &i));
    builder.emit(abi::add_immediate(&live, &live, 1));
    builder.emit(abi::label(&scan_next));
    builder.emit(abi::add_immediate(&i, &i, 1));
    builder.emit(abi::branch(&scan));
    builder.emit(abi::label(&scanned));
    let keep = builder.label("canvas_geo_ring_join_keep");
    if closed {
        builder.emit(abi::compare_immediate(&live, "2"));
        builder.emit(abi::branch_ge(&keep));
    }
    builder.emit(abi::move_immediate(&prev, "Integer", "0"));
    builder.emit(abi::subtract_immediate(&prev, &prev, 1));
    builder.emit(abi::label(&keep));
    // One record per edge.
    let head = builder.label("canvas_geo_ring_join");
    let next = builder.label("canvas_geo_ring_join_next");
    let none = builder.label("canvas_geo_ring_join_none");
    let walked = builder.label("canvas_geo_ring_join_walked");
    builder.emit(abi::move_immediate(&i, "Integer", "0"));
    builder.emit(abi::label(&head));
    builder.emit(abi::compare_registers(&i, n));
    builder.emit(abi::branch_ge(&walked));
    builder.emit(abi::multiply_registers(&at, &i, &forty));
    builder.emit(abi::add_registers(&at, edges, &at));
    let vx = fload(builder, &at, 0);
    let vy = fload(builder, &at, 8);
    let dx = fload(builder, &at, 16);
    let dy = fload(builder, &at, 24);
    branch_unless_live(builder, &dx, &dy, &none);
    let joined = builder.label("canvas_geo_ring_join_has_prev");
    let remember = builder.label("canvas_geo_ring_join_remember");
    builder.emit(abi::compare_immediate(&prev, "0"));
    builder.emit(abi::branch_ge(&joined));
    emit_store_join(builder, cursor, [&vx, &vy, &vx, &vy, &vx, &vy, &vx, &vy]);
    builder.emit(abi::branch(&remember));
    builder.emit(abi::label(&joined));
    let pat = builder.temporary_vreg();
    builder.emit(abi::multiply_registers(&pat, &prev, &forty));
    builder.emit(abi::add_registers(&pat, edges, &pat));
    let pdx = fload(builder, &pat, 16);
    let pdy = fload(builder, &pat, 24);
    emit_join_record(builder, cursor, &vx, &vy, &pdx, &pdy, &dx, &dy, half, style);
    builder.emit(abi::label(&remember));
    builder.emit(abi::move_register(&prev, &i));
    builder.emit(abi::branch(&next));
    builder.emit(abi::label(&none));
    emit_store_join(builder, cursor, [&vx, &vy, &vx, &vy, &vx, &vy, &vx, &vy]);
    builder.emit(abi::label(&next));
    builder.emit(abi::add_immediate(&i, &i, 1));
    builder.emit(abi::branch(&head));
    builder.emit(abi::label(&walked));
}

/// Branch to `none` unless the paint's join draws a tail: a positive half-width and a
/// tag other than Round, or a dashed stroke (`__canvas_joinTail`'s early returns).
fn branch_unless_joined(
    builder: &mut CodeBuilder,
    style: &JoinStyle,
    half: &VirtualRegister,
    none: &str,
) {
    branch_if_not_positive(builder, half, none);
    let joined = builder.label("canvas_geo_joined");
    builder.emit(abi::compare_immediate(&style.tag, "0"));
    builder.emit(abi::branch_ne(&joined));
    builder.emit(abi::compare_immediate(&style.dashed, "0"));
    builder.emit(abi::branch_eq(none));
    builder.emit(abi::label(&joined));
}

/// `__canvas_joinReach(half, paint)`: `half * limit` for a Miter, else `half`.
fn emit_join_reach(
    builder: &mut CodeBuilder,
    half: &VirtualRegister,
    style: &JoinStyle,
) -> VirtualRegister {
    let reach = builder.temporary_fp_vreg();
    builder.emit(abi::float_move_d_from_d(&reach, half));
    let plain = builder.label("canvas_geo_join_reach_plain");
    builder.emit(abi::compare_immediate(&style.tag, "1"));
    builder.emit(abi::branch_ne(&plain));
    let tip = fmul(builder, half, &style.limit);
    builder.emit(abi::float_move_d_from_d(&reach, &tip));
    builder.emit(abi::label(&plain));
    reach
}

/// `__canvas_withJoins` for `records` records after floats totalling `length`: slot
/// `__CANVAS_GEO_JOIN_COUNT`, the reach in `__CANVAS_GEO_JOIN_REACH`, and slot 1 =
/// `length + records * 8`.
fn emit_put_joins(
    builder: &mut CodeBuilder,
    out: &VirtualRegister,
    records: &VirtualRegister,
    length: &VirtualRegister,
    reach: &VirtualRegister,
) {
    let records_f = builder.temporary_fp_vreg();
    builder.emit(abi::signed_convert_to_float_d(&records_f, records));
    put(builder, out, SLOT_JOIN_COUNT, &records_f);
    put(builder, out, SLOT_JOIN_REACH, reach);
    let eight = builder.temporary_vreg();
    builder.emit(abi::move_immediate(
        &eight,
        "Integer",
        &JOIN_FLOATS.to_string(),
    ));
    let total = builder.temporary_vreg();
    builder.emit(abi::multiply_registers(&total, records, &eight));
    builder.emit(abi::add_registers(&total, &total, length));
    let total_f = builder.temporary_fp_vreg();
    builder.emit(abi::signed_convert_to_float_d(&total_f, &total));
    put(builder, out, 1, &total_f);
}

/// `n += records * 8`.
fn emit_add_join_floats(builder: &mut CodeBuilder, records: &VirtualRegister, n: &VirtualRegister) {
    let eight = builder.temporary_vreg();
    builder.emit(abi::move_immediate(
        &eight,
        "Integer",
        &JOIN_FLOATS.to_string(),
    ));
    let floats = builder.temporary_vreg();
    builder.emit(abi::multiply_registers(&floats, records, &eight));
    builder.emit(abi::add_registers(n, n, &floats));
}

/// Whether a rectangle has join records (`__canvas_rectJoins`): a positive size, no
/// corner radius, and a joined live stroke -- computed as `fill_rect` computes them.
/// Falls through when it has, branches to `none` when not.
fn emit_rect_joined(
    builder: &mut CodeBuilder,
    rec: &VirtualRegister,
    record: &ParameterType,
    view: &PaintView,
    rounded: bool,
    none: &str,
) -> Result<(), String> {
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
    branch_if_not_positive(builder, &w, none);
    branch_if_not_positive(builder, &h, none);
    let two = fconst(builder, 2.0);
    let zero = fconst(builder, 0.0);
    let wh = fmin(builder, &w, &h);
    let limit = fdiv(builder, &wh, &two);
    let corner_pos = fmax(builder, &corner, &zero);
    let radius = fmin(builder, &corner_pos, &limit);
    let half = emit_stroke_half(builder, view)?;
    let style = emit_join_style(builder, view, &half)?;
    branch_unless_joined(builder, &style, &half, none);
    builder.emit(abi::float_compare_d(&radius, &zero));
    builder.emit(abi::branch_ne(none));
    Ok(())
}

/// `n +=` a rectangle's dash tail (`__canvas_rectDashes`): a positive size and a dashed
/// stroke, the pattern's head and, with no corner radius, four sides' perimeter data.
/// Branches to `solid` when it has none.
fn emit_rect_dash_size(
    builder: &mut CodeBuilder,
    rec: &VirtualRegister,
    record: &ParameterType,
    view: &PaintView,
    rounded: bool,
    solid: &str,
    n: &VirtualRegister,
) -> Result<(), String> {
    let w = fload(builder, rec, scalar_offset(builder, record, "w")?);
    let h = fload(builder, rec, scalar_offset(builder, record, "h")?);
    branch_if_not_positive(builder, &w, solid);
    branch_if_not_positive(builder, &h, solid);
    let half = emit_stroke_half(builder, view)?;
    let pattern = emit_dash_pattern(builder, view, &half, solid)?;
    let data = builder.temporary_vreg();
    builder.emit(abi::move_immediate(&data, "Integer", "0"));
    let corner = if rounded {
        fload(
            builder,
            rec,
            scalar_offset(builder, record, "cornerRadius")?,
        )
    } else {
        fconst(builder, 0.0)
    };
    let two = fconst(builder, 2.0);
    let zero = fconst(builder, 0.0);
    let wh = fmin(builder, &w, &h);
    let limit = fdiv(builder, &wh, &two);
    let corner_pos = fmax(builder, &corner, &zero);
    let radius = fmin(builder, &corner_pos, &limit);
    let curved = builder.label("canvas_geo_size_rect_curved");
    builder.emit(abi::float_compare_d(&radius, &zero));
    builder.emit(abi::branch_ne(&curved));
    builder.emit(abi::move_immediate(
        &data,
        "Integer",
        &(4 * DASH_EDGE_FLOATS).to_string(),
    ));
    builder.emit(abi::label(&curved));
    emit_add_dash_floats(builder, &pattern, &data, n);
    Ok(())
}

/// `__canvas_rectSides`: a rectangle's four sides from the centre and half-extents its
/// header holds, into the 160-byte block at `sides` (the tail layout; the unused
/// `invLenSq` words are left as they are, nothing reads them).
#[allow(clippy::too_many_arguments)]
fn emit_rect_sides(
    builder: &mut CodeBuilder,
    sides: &VirtualRegister,
    cx: &VirtualRegister,
    cy: &VirtualRegister,
    hw: &VirtualRegister,
    hh: &VirtualRegister,
) {
    let x0 = fsub(builder, cx, hw);
    let x1 = fadd(builder, cx, hw);
    let y0 = fsub(builder, cy, hh);
    let y1 = fadd(builder, cy, hh);
    let zero = fconst(builder, 0.0);
    let w = fsub(builder, &x1, &x0);
    let h = fsub(builder, &y1, &y0);
    let nw = fsub(builder, &x0, &x1);
    let nh = fsub(builder, &y0, &y1);
    let rows: [[&VirtualRegister; 4]; 4] = [
        [&x0, &y0, &w, &zero],
        [&x1, &y0, &zero, &h],
        [&x1, &y1, &nw, &zero],
        [&x0, &y1, &zero, &nh],
    ];
    for (r, row) in rows.iter().enumerate() {
        for (c, value) in row.iter().enumerate() {
            builder.emit(abi::store_double(*value, sides, r * 40 + c * 8));
        }
    }
}

// ---- Dashes (plan-163-B) --------------------------------------------------------------
//
// `helper_dashes.rs`'s build side, operation for operation: `__canvas_dashPattern`,
// `__canvas_dashTail`, `__canvas_dashPhase`, `__canvas_dashEdgeData` and
// `__canvas_withDashes`.

/// An edge's perimeter data: where its ring's length before it ends, where it ends, and
/// its ring's length (negative for an open path).
const DASH_EDGE_FLOATS: usize = 3;

/// A dashed paint's pattern: `paint.dashes` read through `list`, its length `n`, and the
/// drawn entry count `entries` — `n`, or `2 * n` for an odd `n`, which repeats once.
struct DashPattern {
    list: ListView,
    n: VirtualRegister,
    entries: VirtualRegister,
}

/// `__canvas_dashPattern(paint, half)`: falls through for a dashed stroke, branches to
/// `none` for a solid one — no lengths, `NOT (half > 0.0)`, an entry `NOT (d >= 0.0)`, or
/// a sum (added left to right from 0.0) `NOT (> 0.0)`.
fn emit_dash_pattern(
    builder: &mut CodeBuilder,
    view: &PaintView,
    half: &VirtualRegister,
    none: &str,
) -> Result<DashPattern, String> {
    let dashes = builder.temporary_vreg();
    let dashes_type = emit_field_block(builder, &dashes, &view.paint, &view.paint_type, "dashes")?;
    let n = builder.temporary_vreg();
    builder.emit(abi::load_u64(&n, &dashes, COLLECTION_OFFSET_COUNT));
    builder.emit(abi::compare_immediate(&n, "0"));
    builder.emit(abi::branch_eq(none));
    let zero = fconst(builder, 0.0);
    let live = builder.label("canvas_geo_dash_live");
    builder.emit(abi::float_compare_d(half, &zero));
    builder.emit(abi::branch_gt(&live));
    builder.emit(abi::branch(none));
    builder.emit(abi::label(&live));
    let list = emit_list_view(builder, &dashes, &dashes_type)?;
    let total = builder.temporary_fp_vreg();
    builder.emit(abi::float_move_d_from_d(&total, &zero));
    let i = builder.temporary_vreg();
    let at = builder.temporary_vreg();
    let head = builder.label("canvas_geo_dash_sum");
    let summed = builder.label("canvas_geo_dash_summed");
    builder.emit(abi::move_immediate(&i, "Integer", "0"));
    builder.emit(abi::label(&head));
    builder.emit(abi::compare_registers(&i, &n));
    builder.emit(abi::branch_ge(&summed));
    emit_list_element(builder, &at, &list, &i);
    let d = fload(builder, &at, 0);
    let ok = builder.label("canvas_geo_dash_entry_ok");
    builder.emit(abi::float_compare_d(&d, &zero));
    builder.emit(abi::branch_ge(&ok));
    builder.emit(abi::branch(none));
    builder.emit(abi::label(&ok));
    let t = fadd(builder, &total, &d);
    builder.emit(abi::float_move_d_from_d(&total, &t));
    builder.emit(abi::add_immediate(&i, &i, 1));
    builder.emit(abi::branch(&head));
    builder.emit(abi::label(&summed));
    let positive = builder.label("canvas_geo_dash_positive");
    builder.emit(abi::float_compare_d(&total, &zero));
    builder.emit(abi::branch_gt(&positive));
    builder.emit(abi::branch(none));
    builder.emit(abi::label(&positive));
    let entries = builder.temporary_vreg();
    builder.emit(abi::move_register(&entries, &n));
    let halved = builder.temporary_vreg();
    builder.emit(abi::shift_right_immediate(&halved, &n, 1));
    builder.emit(abi::shift_left_immediate(&halved, &halved, 1));
    let even = builder.label("canvas_geo_dash_even");
    builder.emit(abi::compare_registers(&halved, &n));
    builder.emit(abi::branch_eq(&even));
    builder.emit(abi::add_registers(&entries, &n, &n));
    builder.emit(abi::label(&even));
    Ok(DashPattern { list, n, entries })
}

/// `n += 2 + entries + data`: a dash tail's floats, the pattern's head and `data` more.
fn emit_add_dash_floats(
    builder: &mut CodeBuilder,
    pattern: &DashPattern,
    data: &VirtualRegister,
    n: &VirtualRegister,
) {
    builder.emit(abi::add_registers(n, n, &pattern.entries));
    builder.emit(abi::add_registers(n, n, data));
    builder.emit(abi::add_immediate(n, n, 2));
}

/// `__canvas_dashTail`'s head at `cursor`, which then moves past it: the entry count,
/// the period, and each entry's end along the period, the ends added left to right from
/// 0.0. Answers the period.
fn emit_dash_head(
    builder: &mut CodeBuilder,
    cursor: &VirtualRegister,
    pattern: &DashPattern,
) -> VirtualRegister {
    let zero = fconst(builder, 0.0);
    let at = builder.temporary_fp_vreg();
    builder.emit(abi::float_move_d_from_d(&at, &zero));
    let k = builder.temporary_vreg();
    let index = builder.temporary_vreg();
    let element = builder.temporary_vreg();
    let slot = builder.temporary_vreg();
    let head = builder.label("canvas_geo_dash_head");
    let wrapped = builder.label("canvas_geo_dash_index");
    let done = builder.label("canvas_geo_dash_head_done");
    builder.emit(abi::move_immediate(&k, "Integer", "0"));
    builder.emit(abi::label(&head));
    builder.emit(abi::compare_registers(&k, &pattern.entries));
    builder.emit(abi::branch_ge(&done));
    builder.emit(abi::move_register(&index, &k));
    builder.emit(abi::compare_registers(&index, &pattern.n));
    builder.emit(abi::branch_lt(&wrapped));
    builder.emit(abi::subtract_registers(&index, &index, &pattern.n));
    builder.emit(abi::label(&wrapped));
    emit_list_element(builder, &element, &pattern.list, &index);
    let d = fload(builder, &element, 0);
    let t = fadd(builder, &at, &d);
    builder.emit(abi::float_move_d_from_d(&at, &t));
    builder.emit(abi::shift_left_immediate(&slot, &k, 3));
    builder.emit(abi::add_registers(&slot, cursor, &slot));
    builder.emit(abi::store_double(&at, &slot, 16));
    builder.emit(abi::add_immediate(&k, &k, 1));
    builder.emit(abi::branch(&head));
    builder.emit(abi::label(&done));
    let entries_f = builder.temporary_fp_vreg();
    builder.emit(abi::signed_convert_to_float_d(&entries_f, &pattern.entries));
    builder.emit(abi::store_double(&entries_f, cursor, 0));
    builder.emit(abi::store_double(&at, cursor, 8));
    builder.emit(abi::shift_left_immediate(&slot, &pattern.entries, 3));
    builder.emit(abi::add_registers(cursor, cursor, &slot));
    builder.emit(abi::add_immediate(cursor, cursor, 16));
    at
}

/// `__canvas_dashPhase(paint.dashOffset, period)`: the offset less `floor(offset /
/// period)` periods (one `fnmsub`), or 0.0 when that is not in [0, period) or the offset
/// is 10^15 periods or more away.
fn emit_dash_phase(
    builder: &mut CodeBuilder,
    view: &PaintView,
    period: &VirtualRegister,
) -> Result<VirtualRegister, String> {
    let offset = fload(
        builder,
        &view.paint,
        scalar_offset(builder, &view.paint_type, "dashOffset")?,
    );
    let zero = fconst(builder, 0.0);
    let phase = builder.temporary_fp_vreg();
    builder.emit(abi::float_move_d_from_d(&phase, &zero));
    let done = builder.label("canvas_geo_dash_phase_done");
    let near = builder.label("canvas_geo_dash_phase_near");
    let turns = fdiv(builder, &offset, period);
    let size = builder.temporary_fp_vreg();
    builder.emit(abi::float_abs_d(&size, &turns));
    let far = fconst(builder, 1_000_000_000_000_000.0);
    builder.emit(abi::float_compare_d(&size, &far));
    builder.emit(abi::branch_mi(&near));
    builder.emit(abi::branch(&done));
    builder.emit(abi::label(&near));
    let whole = builder.temporary_vreg();
    builder.emit(abi::float_floor_to_signed_x(&whole, &turns));
    let whole_f = builder.temporary_fp_vreg();
    builder.emit(abi::signed_convert_to_float_d(&whole_f, &whole));
    let p = builder.temporary_fp_vreg();
    builder.emit(abi::float_negate_multiply_sub_d(
        &p, &offset, &whole_f, period,
    ));
    builder.emit(abi::float_compare_d(&p, &zero));
    builder.emit(abi::branch_mi(&done));
    builder.emit(abi::float_compare_d(&p, period));
    builder.emit(abi::branch_ge(&done));
    builder.emit(abi::float_move_d_from_d(&phase, &p));
    builder.emit(abi::label(&done));
    Ok(phase)
}

/// `__canvas_dashEdgeData` over one ring: the `n` edges at `edges` (the tail layout, 40
/// bytes each), three floats each at `cursor`, which then moves past them. The running
/// length adds `sqrt(dx * dx + dy * dy)` (the fused square) left to right from 0.0; the
/// ring's length, `at` or `0.0 - at`, is written once the ring is walked.
fn emit_ring_dash_data(
    builder: &mut CodeBuilder,
    edges: &VirtualRegister,
    n: &VirtualRegister,
    closed: bool,
    cursor: &VirtualRegister,
) {
    let zero = fconst(builder, 0.0);
    let at = builder.temporary_fp_vreg();
    let before = builder.temporary_fp_vreg();
    builder.emit(abi::float_move_d_from_d(&at, &zero));
    builder.emit(abi::float_move_d_from_d(&before, &zero));
    let start = builder.temporary_vreg();
    builder.emit(abi::move_register(&start, cursor));
    let i = builder.temporary_vreg();
    let edge = builder.temporary_vreg();
    let forty = builder.temporary_vreg();
    builder.emit(abi::move_immediate(&forty, "Integer", "40"));
    let head = builder.label("canvas_geo_dash_edge");
    let walked = builder.label("canvas_geo_dash_edges_walked");
    builder.emit(abi::move_immediate(&i, "Integer", "0"));
    builder.emit(abi::label(&head));
    builder.emit(abi::compare_registers(&i, n));
    builder.emit(abi::branch_ge(&walked));
    builder.emit(abi::multiply_registers(&edge, &i, &forty));
    builder.emit(abi::add_registers(&edge, edges, &edge));
    let dx = fload(builder, &edge, 16);
    let dy = fload(builder, &edge, 24);
    let len_sq = fused_len_sq(builder, &dx, &dy);
    let length = builder.temporary_fp_vreg();
    builder.emit(abi::float_sqrt_d(&length, &len_sq));
    let t = fadd(builder, &at, &length);
    builder.emit(abi::float_move_d_from_d(&at, &t));
    builder.emit(abi::store_double(&before, cursor, 0));
    builder.emit(abi::store_double(&at, cursor, 8));
    builder.emit(abi::float_move_d_from_d(&before, &at));
    builder.emit(abi::add_immediate(cursor, cursor, DASH_EDGE_FLOATS * 8));
    builder.emit(abi::add_immediate(&i, &i, 1));
    builder.emit(abi::branch(&head));
    builder.emit(abi::label(&walked));
    let ring = if closed {
        at
    } else {
        fsub(builder, &zero, &at)
    };
    let fill = builder.label("canvas_geo_dash_ring_length");
    let filled = builder.label("canvas_geo_dash_ring_filled");
    builder.emit(abi::label(&fill));
    builder.emit(abi::compare_registers(&start, cursor));
    builder.emit(abi::branch_ge(&filled));
    builder.emit(abi::store_double(&ring, &start, 16));
    builder.emit(abi::add_immediate(&start, &start, DASH_EDGE_FLOATS * 8));
    builder.emit(abi::branch(&fill));
    builder.emit(abi::label(&filled));
}

/// `__canvas_withDashes`: the tail's `floats` into `__CANVAS_GEO_DASH_COUNT`, the phase,
/// the cap tag into `__CANVAS_GEO_CAP`, and slot 1 grown by `floats` (a float add).
fn emit_put_dashes(
    builder: &mut CodeBuilder,
    out: &VirtualRegister,
    floats: &VirtualRegister,
    phase: &VirtualRegister,
    cap: &VirtualRegister,
) {
    let floats_f = builder.temporary_fp_vreg();
    builder.emit(abi::signed_convert_to_float_d(&floats_f, floats));
    put(builder, out, SLOT_DASH_COUNT, &floats_f);
    put(builder, out, SLOT_DASH_PHASE, phase);
    let cap_f = builder.temporary_fp_vreg();
    builder.emit(abi::signed_convert_to_float_d(&cap_f, cap));
    put(builder, out, SLOT_CAP, &cap_f);
    let length = fload(builder, out, 8);
    let grown = fadd(builder, &length, &floats_f);
    put(builder, out, 1, &grown);
}

/// A dash tail at `cursor` (moved past it) for `pattern`, its perimeter data written by
/// `data`, then the header's dash slots. `cursor` ends where the tail ends.
fn emit_dash_tail(
    builder: &mut CodeBuilder,
    out: &VirtualRegister,
    cursor: &VirtualRegister,
    view: &PaintView,
    pattern: &DashPattern,
    cap: &VirtualRegister,
    data: impl FnOnce(&mut CodeBuilder, &VirtualRegister),
) -> Result<(), String> {
    let start = builder.temporary_vreg();
    builder.emit(abi::move_register(&start, cursor));
    let period = emit_dash_head(builder, cursor, pattern);
    data(builder, cursor);
    let floats = builder.temporary_vreg();
    builder.emit(abi::subtract_registers(&floats, cursor, &start));
    builder.emit(abi::shift_right_immediate(&floats, &floats, 3));
    let phase = emit_dash_phase(builder, view, &period)?;
    emit_put_dashes(builder, out, &floats, &phase, cap);
    Ok(())
}

/// The record's end so far, `out + slot 1 * 8`: where a tail written last goes.
fn emit_record_end(builder: &mut CodeBuilder, out: &VirtualRegister) -> VirtualRegister {
    let length = fload(builder, out, 8);
    let end = builder.temporary_vreg();
    builder.emit(abi::float_convert_to_signed_x(&end, &length));
    builder.emit(abi::shift_left_immediate(&end, &end, 3));
    builder.emit(abi::add_registers(&end, out, &end));
    end
}

// ---- The kinds -------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Rect,
    Rounded,
    Circle,
    Line,
    Polygon,
    PolyLine,
}

impl Kind {
    const ALL: [Kind; 6] = [
        Kind::Rect,
        Kind::Rounded,
        Kind::Circle,
        Kind::Line,
        Kind::Polygon,
        Kind::PolyLine,
    ];

    fn variant(self) -> &'static str {
        match self {
            Kind::Rect => "Rectangle",
            Kind::Rounded => "RoundedRect",
            Kind::Circle => "Circle",
            Kind::Line => "Line",
            Kind::Polygon => "Polygon",
            Kind::PolyLine => "PolyLine",
        }
    }
}

/// `__canvas_emptyHeader`: kind NONE, length 51, every other slot zero.
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
    let style = emit_join_style(builder, view, &stroke_half)?;
    let pad = stroke_pad(builder, &stroke_half, &style);
    let x0 = fsub(builder, &x, &pad);
    let y0 = fsub(builder, &y, &pad);
    let xw = fadd(builder, &x, &w);
    let x1 = fadd(builder, &xw, &pad);
    let yh = fadd(builder, &y, &h);
    let y1 = fadd(builder, &yh, &pad);
    put_bounds(builder, out, [&x0, &y0, &x1, &y1]);
    // plan-163-B: `__canvas_rectJoins` -- a sharp-cornered rectangle's four corners.
    let no_joins = builder.label("canvas_geo_rect_no_joins");
    branch_unless_joined(builder, &style, &stroke_half, &no_joins);
    let zero = fconst(builder, 0.0);
    builder.emit(abi::float_compare_d(&radius, &zero));
    builder.emit(abi::branch_ne(&no_joins));
    let sides_slot = builder.allocate_stack_object("canvas_geo_rect_sides", 160);
    let sides = builder.temporary_vreg();
    builder.emit(abi::add_immediate(&sides, abi::stack_pointer(), sides_slot));
    emit_rect_sides(builder, &sides, &cx, &cy, &ex, &ey);
    let cursor = builder.temporary_vreg();
    builder.emit(abi::add_immediate(&cursor, out, GEO_HEADER * 8));
    let four = builder.temporary_vreg();
    builder.emit(abi::move_immediate(&four, "Integer", "4"));
    emit_ring_joins(builder, &sides, &four, true, &cursor, &stroke_half, &style);
    let length = builder.temporary_vreg();
    builder.emit(abi::move_immediate(
        &length,
        "Integer",
        &GEO_HEADER.to_string(),
    ));
    let reach = emit_join_reach(builder, &stroke_half, &style);
    emit_put_joins(builder, out, &four, &length, &reach);
    builder.emit(abi::label(&no_joins));
    // plan-163-B: `__canvas_rectDashes` -- after the join records, a sharp-cornered
    // rectangle's four sides' perimeter data; a rounded one's perimeter needs none.
    let solid = builder.label("canvas_geo_rect_solid");
    let pattern = emit_dash_pattern(builder, view, &stroke_half, &solid)?;
    let cursor = emit_record_end(builder, out);
    let dash_sides_slot = builder.allocate_stack_object("canvas_geo_rect_dash_sides", 160);
    emit_dash_tail(
        builder,
        out,
        &cursor,
        view,
        &pattern,
        &style.cap,
        |builder, cursor| {
            let curved = builder.label("canvas_geo_rect_dash_curved");
            let zero = fconst(builder, 0.0);
            builder.emit(abi::float_compare_d(&radius, &zero));
            builder.emit(abi::branch_ne(&curved));
            let sides = builder.temporary_vreg();
            builder.emit(abi::add_immediate(
                &sides,
                abi::stack_pointer(),
                dash_sides_slot,
            ));
            emit_rect_sides(builder, &sides, &cx, &cy, &ex, &ey);
            let four = builder.temporary_vreg();
            builder.emit(abi::move_immediate(&four, "Integer", "4"));
            emit_ring_dash_data(builder, &sides, &four, true, cursor);
            builder.emit(abi::label(&curved));
        },
    )?;
    builder.emit(abi::label(&solid));
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
    // `radius + __canvas_dashCapReach(__canvas_maxF(half, 0.0), paint) + 1.0`, left to
    // right.
    let zero = fconst(builder, 0.0);
    let m = fmax(builder, &stroke_half, &zero);
    let style = emit_join_style(builder, view, &stroke_half)?;
    let cap_reach = dash_cap_reach(builder, &m, &style);
    let rm = fadd(builder, &radius, &cap_reach);
    let one = fconst(builder, 1.0);
    let reach = fadd(builder, &rm, &one);
    let x0 = fsub(builder, &x, &reach);
    let y0 = fsub(builder, &y, &reach);
    let x1 = fadd(builder, &x, &reach);
    let y1 = fadd(builder, &y, &reach);
    put_bounds(builder, out, [&x0, &y0, &x1, &y1]);
    // plan-163-B: `__canvas_roundDashes` -- the pattern alone.
    let pattern = emit_dash_pattern(builder, view, &stroke_half, done)?;
    let cursor = emit_record_end(builder, out);
    emit_dash_tail(builder, out, &cursor, view, &pattern, &style.cap, |_, _| {})?;
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
    // `__canvas_capTag(paint.cap)`: Round 1, Square 2, Butt 0.
    let cap = emit_cap_tag(builder, view)?;
    let cap_f = builder.temporary_fp_vreg();
    builder.emit(abi::signed_convert_to_float_d(&cap_f, &cap));
    put(builder, out, SLOT_CAP, &cap_f);
    emit_paint_header(builder, out, view, &half)?;
    // `__canvas_strokeAsFill`: the stroke colour into the fill slots, half = -1.
    for k in 0..4 {
        let v = fload(builder, out, (12 + k) * 8);
        put(builder, out, 8 + k, &v);
    }
    put_const(builder, out, 7, -1.0);
    let pad = cap_pad(builder, &half, &cap);
    let min_x = fmin(builder, &x1, &x2);
    let bx0 = fsub(builder, &min_x, &pad);
    let min_y = fmin(builder, &y1, &y2);
    let by0 = fsub(builder, &min_y, &pad);
    let max_x = fmax(builder, &x1, &x2);
    let bx1 = fadd(builder, &max_x, &pad);
    let max_y = fmax(builder, &y1, &y2);
    let by1 = fadd(builder, &max_y, &pad);
    put_bounds(builder, out, [&bx0, &by0, &bx1, &by1]);
    // plan-163-B: `__canvas_lineDashes` -- its one edge, `x2 - x1, y2 - y1`.
    let pattern = emit_dash_pattern(builder, view, &half, done)?;
    let edge_slot = builder.allocate_stack_object("canvas_geo_line_dash_edge", 40);
    let edge = builder.temporary_vreg();
    builder.emit(abi::add_immediate(&edge, abi::stack_pointer(), edge_slot));
    let dx = fsub(builder, &x2, &x1);
    let dy = fsub(builder, &y2, &y1);
    builder.emit(abi::store_double(&dx, &edge, 16));
    builder.emit(abi::store_double(&dy, &edge, 24));
    let cursor = emit_record_end(builder, out);
    emit_dash_tail(
        builder,
        out,
        &cursor,
        view,
        &pattern,
        &cap,
        |builder, cursor| {
            let one = builder.temporary_vreg();
            builder.emit(abi::move_immediate(&one, "Integer", "1"));
            emit_ring_dash_data(builder, &edge, &one, false, cursor);
        },
    )?;
    builder.emit(abi::branch(done));
    builder.emit(abi::label(&empty));
    put_empty(builder, out);
    builder.emit(abi::branch(done));
    Ok(())
}

/// The `x`/`y` offsets of a point in a polygon ring of type `ring_type`
/// (`List OF Point`).
fn ring_point_offsets(
    builder: &CodeBuilder,
    ring_type: &ParameterType,
) -> Result<(usize, usize), String> {
    let ParameterType::ListOf(point) = ring_type else {
        return Err(format!(
            "canvas: a polygon ring '{ring_type}' is not a list"
        ));
    };
    let point_type = record_type(builder, point)?;
    Ok((
        scalar_offset(builder, &point_type, "x")?,
        scalar_offset(builder, &point_type, "y")?,
    ))
}

/// `__canvas_polygonHeader` and `__canvas_polygonEdges` (plan-163-A: rings and a rule).
///
/// Two walks over `contours`, each skipping a ring of fewer than 2 points: the bounds
/// and the edge total, then every contributing ring's edges with the wrap inside the
/// ring. Each walk is the MFBASIC builder's loop in its order, so the record is
/// bit-identical to it.
fn fill_polygon(
    builder: &mut CodeBuilder,
    out: &VirtualRegister,
    rec: &VirtualRegister,
    record: &ParameterType,
    view: &PaintView,
    done: &str,
) -> Result<(), String> {
    let empty = builder.label("canvas_geo_poly_empty");
    let rings = builder.temporary_vreg();
    let rings_type = emit_field_block(builder, &rings, rec, record, "contours")?;
    let ring_count = builder.temporary_vreg();
    builder.emit(abi::load_u64(&ring_count, &rings, COLLECTION_OFFSET_COUNT));
    let outer = emit_list_view(builder, &rings, &rings_type)?;
    let ring_type = outer.element.clone();
    let (xo, yo) = ring_point_offsets(builder, &ring_type)?;

    // The bounds: `0.0` until the first contributing ring's first point, then
    // minF/maxF over every later point in order. `total` is the edge count so far.
    let total = builder.temporary_vreg();
    let r = builder.temporary_vreg();
    let ring = builder.temporary_vreg();
    let n = builder.temporary_vreg();
    let i = builder.temporary_vreg();
    let q = builder.temporary_vreg();
    let min_x = builder.temporary_fp_vreg();
    let max_x = builder.temporary_fp_vreg();
    let min_y = builder.temporary_fp_vreg();
    let max_y = builder.temporary_fp_vreg();
    let zero = fconst(builder, 0.0);
    for v in [&min_x, &max_x, &min_y, &max_y] {
        builder.emit(abi::float_move_d_from_d(v, &zero));
    }
    builder.emit(abi::move_immediate(&total, "Integer", "0"));
    builder.emit(abi::move_immediate(&r, "Integer", "0"));
    let ring_head = builder.label("canvas_geo_poly_ring");
    let ring_next = builder.label("canvas_geo_poly_ring_next");
    let rings_done = builder.label("canvas_geo_poly_rings_done");
    let head = builder.label("canvas_geo_poly_bounds");
    let bounds_done = builder.label("canvas_geo_poly_bounds_done");
    builder.emit(abi::label(&ring_head));
    builder.emit(abi::compare_registers(&r, &ring_count));
    builder.emit(abi::branch_ge(&rings_done));
    emit_list_element(builder, &ring, &outer, &r);
    builder.emit(abi::load_u64(&n, &ring, COLLECTION_OFFSET_COUNT));
    builder.emit(abi::compare_immediate(&n, "2"));
    builder.emit(abi::branch_lt(&ring_next));
    let list = emit_list_view(builder, &ring, &ring_type)?;
    builder.emit(abi::move_immediate(&i, "Integer", "0"));
    builder.emit(abi::compare_immediate(&total, "0"));
    builder.emit(abi::branch_ne(&head));
    emit_list_element(builder, &q, &list, &i);
    let fx = fload(builder, &q, xo);
    let fy = fload(builder, &q, yo);
    builder.emit(abi::float_move_d_from_d(&min_x, &fx));
    builder.emit(abi::float_move_d_from_d(&max_x, &fx));
    builder.emit(abi::float_move_d_from_d(&min_y, &fy));
    builder.emit(abi::float_move_d_from_d(&max_y, &fy));
    builder.emit(abi::move_immediate(&i, "Integer", "1"));
    builder.emit(abi::label(&head));
    builder.emit(abi::compare_registers(&i, &n));
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
    builder.emit(abi::add_registers(&total, &total, &n));
    builder.emit(abi::label(&ring_next));
    builder.emit(abi::add_immediate(&r, &r, 1));
    builder.emit(abi::branch(&ring_head));
    builder.emit(abi::label(&rings_done));
    builder.emit(abi::compare_immediate(&total, "0"));
    builder.emit(abi::branch_eq(&empty));

    // Header: kind, the record length `toFloat(51 + count * 5)`, the paint, the edge
    // count in slot 20, the rule in slot 21 (1.0 for NonZero; the slot was zeroed),
    // then the padded bounds.
    put_const(builder, out, 0, KIND_POLYGON);
    let length = builder.temporary_vreg();
    let five = builder.temporary_vreg();
    builder.emit(abi::move_immediate(
        &five,
        "Integer",
        &EDGE_FLOATS.to_string(),
    ));
    builder.emit(abi::multiply_registers(&length, &total, &five));
    builder.emit(abi::add_immediate(&length, &length, GEO_HEADER));
    let length_f = builder.temporary_fp_vreg();
    builder.emit(abi::signed_convert_to_float_d(&length_f, &length));
    put(builder, out, 1, &length_f);
    let stroke_half = emit_stroke_half(builder, view)?;
    emit_paint_header(builder, out, view, &stroke_half)?;
    let count_f = builder.temporary_fp_vreg();
    builder.emit(abi::signed_convert_to_float_d(&count_f, &total));
    put(builder, out, 20, &count_f);
    let (rule_index, rule_type) = field(builder, record, "rule")?;
    let non_zero = enum_ordinal(builder, &rule_type, "NonZero")?;
    let rule = builder.temporary_vreg();
    builder.emit(abi::load_u64(&rule, rec, 8 * rule_index));
    let rule_done = builder.label("canvas_geo_poly_rule_done");
    builder.emit(abi::compare_immediate(&rule, &non_zero.to_string()));
    builder.emit(abi::branch_ne(&rule_done));
    put_const(builder, out, SLOT_POLYGON_RULE, 1.0);
    builder.emit(abi::label(&rule_done));
    let style = emit_join_style(builder, view, &stroke_half)?;
    let pad = stroke_pad(builder, &stroke_half, &style);
    let bx0 = fsub(builder, &min_x, &pad);
    let by0 = fsub(builder, &min_y, &pad);
    let bx1 = fadd(builder, &max_x, &pad);
    let by1 = fadd(builder, &max_y, &pad);
    put_bounds(builder, out, [&bx0, &by0, &bx1, &by1]);

    // The edges: `x0, y0, dx, dy, invLenSq` for point i to point (i + 1) MOD count of
    // each contributing ring, the rings one after another.
    let cursor = builder.temporary_vreg();
    builder.emit(abi::add_immediate(&cursor, out, GEO_HEADER * 8));
    let j = builder.temporary_vreg();
    let a = builder.temporary_vreg();
    let b = builder.temporary_vreg();
    let edge_ring = builder.label("canvas_geo_poly_edge_ring");
    let edge_ring_next = builder.label("canvas_geo_poly_edge_ring_next");
    let edge_head = builder.label("canvas_geo_poly_edge");
    let edge_wrap = builder.label("canvas_geo_poly_wrap");
    let edge_done = builder.label("canvas_geo_poly_edges_done");
    builder.emit(abi::move_immediate(&r, "Integer", "0"));
    builder.emit(abi::label(&edge_ring));
    builder.emit(abi::compare_registers(&r, &ring_count));
    builder.emit(abi::branch_ge(&edge_done));
    emit_list_element(builder, &ring, &outer, &r);
    builder.emit(abi::load_u64(&n, &ring, COLLECTION_OFFSET_COUNT));
    builder.emit(abi::compare_immediate(&n, "2"));
    builder.emit(abi::branch_lt(&edge_ring_next));
    let list = emit_list_view(builder, &ring, &ring_type)?;
    builder.emit(abi::move_immediate(&i, "Integer", "0"));
    builder.emit(abi::label(&edge_head));
    builder.emit(abi::compare_registers(&i, &n));
    builder.emit(abi::branch_ge(&edge_ring_next));
    builder.emit(abi::add_immediate(&j, &i, 1));
    builder.emit(abi::compare_registers(&j, &n));
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
    builder.emit(abi::label(&edge_ring_next));
    builder.emit(abi::add_immediate(&r, &r, 1));
    builder.emit(abi::branch(&edge_ring));
    builder.emit(abi::label(&edge_done));

    // plan-163-B: the join records, ring by ring over the edges just written
    // (`__canvas_joinTail` with `__canvas_ringStarts`), then their count.
    let joins_skipped = builder.label("canvas_geo_poly_joins_skipped");
    branch_unless_joined(builder, &style, &stroke_half, &joins_skipped);
    let ring_edges = builder.temporary_vreg();
    builder.emit(abi::add_immediate(&ring_edges, out, GEO_HEADER * 8));
    let join_ring = builder.label("canvas_geo_poly_join_ring");
    let join_ring_next = builder.label("canvas_geo_poly_join_ring_next");
    let joins_done = builder.label("canvas_geo_poly_joins_done");
    builder.emit(abi::move_immediate(&r, "Integer", "0"));
    builder.emit(abi::label(&join_ring));
    builder.emit(abi::compare_registers(&r, &ring_count));
    builder.emit(abi::branch_ge(&joins_done));
    emit_list_element(builder, &ring, &outer, &r);
    builder.emit(abi::load_u64(&n, &ring, COLLECTION_OFFSET_COUNT));
    builder.emit(abi::compare_immediate(&n, "2"));
    builder.emit(abi::branch_lt(&join_ring_next));
    emit_ring_joins(
        builder,
        &ring_edges,
        &n,
        true,
        &cursor,
        &stroke_half,
        &style,
    );
    let bytes = builder.temporary_vreg();
    let forty = builder.temporary_vreg();
    builder.emit(abi::move_immediate(&forty, "Integer", "40"));
    builder.emit(abi::multiply_registers(&bytes, &n, &forty));
    builder.emit(abi::add_registers(&ring_edges, &ring_edges, &bytes));
    builder.emit(abi::label(&join_ring_next));
    builder.emit(abi::add_immediate(&r, &r, 1));
    builder.emit(abi::branch(&join_ring));
    builder.emit(abi::label(&joins_done));
    builder.emit(abi::multiply_registers(&length, &total, &five));
    builder.emit(abi::add_immediate(&length, &length, GEO_HEADER));
    let reach = emit_join_reach(builder, &stroke_half, &style);
    emit_put_joins(builder, out, &total, &length, &reach);
    builder.emit(abi::label(&joins_skipped));
    // plan-163-B: `__canvas_edgeDashes` -- each contributing ring's perimeter data, after
    // the join records.
    let pattern = emit_dash_pattern(builder, view, &stroke_half, done)?;
    emit_dash_tail(
        builder,
        out,
        &cursor,
        view,
        &pattern,
        &style.cap,
        |builder, cursor| {
            let ring_edges = builder.temporary_vreg();
            builder.emit(abi::add_immediate(&ring_edges, out, GEO_HEADER * 8));
            let ring_head = builder.label("canvas_geo_poly_dash_ring");
            let ring_next = builder.label("canvas_geo_poly_dash_ring_next");
            let rings_walked = builder.label("canvas_geo_poly_dash_rings_done");
            builder.emit(abi::move_immediate(&r, "Integer", "0"));
            builder.emit(abi::label(&ring_head));
            builder.emit(abi::compare_registers(&r, &ring_count));
            builder.emit(abi::branch_ge(&rings_walked));
            emit_list_element(builder, &ring, &outer, &r);
            builder.emit(abi::load_u64(&n, &ring, COLLECTION_OFFSET_COUNT));
            builder.emit(abi::compare_immediate(&n, "2"));
            builder.emit(abi::branch_lt(&ring_next));
            emit_ring_dash_data(builder, &ring_edges, &n, true, cursor);
            let bytes = builder.temporary_vreg();
            let forty = builder.temporary_vreg();
            builder.emit(abi::move_immediate(&forty, "Integer", "40"));
            builder.emit(abi::multiply_registers(&bytes, &n, &forty));
            builder.emit(abi::add_registers(&ring_edges, &ring_edges, &bytes));
            builder.emit(abi::label(&ring_next));
            builder.emit(abi::add_immediate(&r, &r, 1));
            builder.emit(abi::branch(&ring_head));
            builder.emit(abi::label(&rings_walked));
        },
    )?;
    builder.emit(abi::branch(done));

    builder.emit(abi::label(&empty));
    put_empty(builder, out);
    builder.emit(abi::branch(done));
    Ok(())
}

/// What `__canvas_polylineHeader` decides before it writes a slot (plan-154-A).
struct PolylineScan {
    /// `len(points)` and `len(colors)`.
    count: VirtualRegister,
    given: VirtualRegister,
    /// `__canvas_polylineHalf`: positive, or the scan branched to `empty`.
    half: VirtualRegister,
    /// The first and last segments with length, and the cap tag (Round 1, Square 2,
    /// Butt 0).
    first: VirtualRegister,
    last: VirtualRegister,
    cap: VirtualRegister,
    points: ListView,
    colors: ListView,
    xo: usize,
    yo: usize,
    /// `minX, minY, maxX, maxY` over the points, when asked for.
    bounds: Option<[VirtualRegister; 4]>,
}

/// `dx * dx + dy * dy` exactly as `opt::fma_fusion` lowers the MFBASIC expression:
/// `dy * dy` rounded, then `fmadd(addend = dy*dy, dx, dx)`.
fn fused_len_sq(
    builder: &mut CodeBuilder,
    dx: &VirtualRegister,
    dy: &VirtualRegister,
) -> VirtualRegister {
    let dy2 = builder.temporary_fp_vreg();
    builder.emit(abi::float_multiply_d(&dy2, dy, dy));
    let len_sq = builder.temporary_fp_vreg();
    builder.emit(abi::float_multiply_add_d(&len_sq, &dy2, dx, dx));
    len_sq
}

/// The part of `__canvas_polylineHeader` before its first slot write: the point and
/// colour counts, `__canvas_polylineHalf`, the cap, and the walk that finds the first and
/// last segments with length (folding the bounds on the way when `with_bounds`). Branches
/// to `empty` exactly where that header returns `__canvas_emptyHeader()`: fewer than two
/// points, no positive half-width, or a butt path with no segment that has length.
///
/// `first` starts at `count`, an index no segment has, as the "not found yet" mark —
/// the MFBASIC's `-1`, without a negative immediate.
fn emit_polyline_scan(
    builder: &mut CodeBuilder,
    rec: &VirtualRegister,
    record: &ParameterType,
    view: &PaintView,
    empty: &str,
    with_bounds: bool,
) -> Result<PolylineScan, String> {
    let points = builder.temporary_vreg();
    let points_type = emit_field_block(builder, &points, rec, record, "points")?;
    let count = builder.temporary_vreg();
    builder.emit(abi::load_u64(&count, &points, COLLECTION_OFFSET_COUNT));
    builder.emit(abi::compare_immediate(&count, "2"));
    builder.emit(abi::branch_lt(empty));
    let colors = builder.temporary_vreg();
    let colors_type = emit_field_block(builder, &colors, rec, record, "colors")?;
    let given = builder.temporary_vreg();
    builder.emit(abi::load_u64(&given, &colors, COLLECTION_OFFSET_COUNT));

    // `__canvas_polylineHalf`: with no colours the paint's stroke half; with colours,
    // `strokeWidth / 2.0` when the width is positive.
    let half = builder.temporary_fp_vreg();
    let no_colors = builder.label("canvas_geo_pl_plain");
    let half_done = builder.label("canvas_geo_pl_half");
    builder.emit(abi::compare_immediate(&given, "0"));
    builder.emit(abi::branch_eq(&no_colors));
    let width_off = scalar_offset(builder, &view.paint_type, "strokeWidth")?;
    let width = fload(builder, &view.paint, width_off);
    branch_if_not_positive(builder, &width, empty);
    let two = fconst(builder, 2.0);
    builder.emit(abi::float_divide_d(&half, &width, &two));
    builder.emit(abi::branch(&half_done));
    builder.emit(abi::label(&no_colors));
    let stroke_half = emit_stroke_half(builder, view)?;
    builder.emit(abi::float_move_d_from_d(&half, &stroke_half));
    builder.emit(abi::label(&half_done));
    branch_if_not_positive(builder, &half, empty);

    let list = emit_list_view(builder, &points, &points_type)?;
    let point_type = record_type(builder, &list.element)?;
    let xo = scalar_offset(builder, &point_type, "x")?;
    let yo = scalar_offset(builder, &point_type, "y")?;
    let color_list = emit_list_view(builder, &colors, &colors_type)?;

    let i = builder.temporary_vreg();
    let j = builder.temporary_vreg();
    let a = builder.temporary_vreg();
    let b = builder.temporary_vreg();
    let bounds = if with_bounds {
        builder.emit(abi::move_immediate(&i, "Integer", "0"));
        emit_list_element(builder, &a, &list, &i);
        let fx = fload(builder, &a, xo);
        let fy = fload(builder, &a, yo);
        let slots = [
            builder.temporary_fp_vreg(),
            builder.temporary_fp_vreg(),
            builder.temporary_fp_vreg(),
            builder.temporary_fp_vreg(),
        ];
        builder.emit(abi::float_move_d_from_d(&slots[0], &fx));
        builder.emit(abi::float_move_d_from_d(&slots[1], &fy));
        builder.emit(abi::float_move_d_from_d(&slots[2], &fx));
        builder.emit(abi::float_move_d_from_d(&slots[3], &fy));
        Some(slots)
    } else {
        None
    };
    let first = builder.temporary_vreg();
    let last = builder.temporary_vreg();
    builder.emit(abi::move_register(&first, &count));
    builder.emit(abi::move_immediate(&last, "Integer", "0"));
    let head = builder.label("canvas_geo_pl_live");
    let dead = builder.label("canvas_geo_pl_dead");
    let seen = builder.label("canvas_geo_pl_seen");
    let walked = builder.label("canvas_geo_pl_walked");
    builder.emit(abi::move_immediate(&i, "Integer", "0"));
    builder.emit(abi::label(&head));
    builder.emit(abi::add_immediate(&j, &i, 1));
    builder.emit(abi::compare_registers(&j, &count));
    builder.emit(abi::branch_ge(&walked));
    emit_list_element(builder, &a, &list, &i);
    emit_list_element(builder, &b, &list, &j);
    let ax = fload(builder, &a, xo);
    let ay = fload(builder, &a, yo);
    let bx = fload(builder, &b, xo);
    let by = fload(builder, &b, yo);
    let dx = fsub(builder, &bx, &ax);
    let dy = fsub(builder, &by, &ay);
    let len_sq = fused_len_sq(builder, &dx, &dy);
    // `IF dx * dx + dy * dy > 0.0` -- a float `>`, so a NaN is not live.
    let zero = fconst(builder, 0.0);
    let live = builder.label("canvas_geo_pl_is_live");
    builder.emit(abi::float_compare_d(&len_sq, &zero));
    builder.emit(abi::branch_gt(&live));
    builder.emit(abi::branch(&dead));
    builder.emit(abi::label(&live));
    builder.emit(abi::compare_registers(&first, &count));
    builder.emit(abi::branch_ne(&seen));
    builder.emit(abi::move_register(&first, &i));
    builder.emit(abi::label(&seen));
    builder.emit(abi::move_register(&last, &i));
    builder.emit(abi::label(&dead));
    if let Some(slots) = &bounds {
        let v = fmin(builder, &slots[0], &bx);
        builder.emit(abi::float_move_d_from_d(&slots[0], &v));
        let v = fmax(builder, &slots[2], &bx);
        builder.emit(abi::float_move_d_from_d(&slots[2], &v));
        let v = fmin(builder, &slots[1], &by);
        builder.emit(abi::float_move_d_from_d(&slots[1], &v));
        let v = fmax(builder, &slots[3], &by);
        builder.emit(abi::float_move_d_from_d(&slots[3], &v));
    }
    builder.emit(abi::move_register(&i, &j));
    builder.emit(abi::branch(&head));
    builder.emit(abi::label(&walked));

    // `__canvas_capTag(paint.cap)`, then the no-live-segment rule: nothing for Butt,
    // segment 0 as both ends for Round and Square.
    let cap = emit_cap_tag(builder, view)?;
    let has_live = builder.label("canvas_geo_pl_has_live");
    builder.emit(abi::compare_registers(&first, &count));
    builder.emit(abi::branch_ne(&has_live));
    builder.emit(abi::compare_immediate(&cap, "0"));
    builder.emit(abi::branch_eq(empty));
    builder.emit(abi::move_immediate(&first, "Integer", "0"));
    builder.emit(abi::move_immediate(&last, "Integer", "0"));
    builder.emit(abi::label(&has_live));
    Ok(PolylineScan {
        count,
        given,
        half,
        first,
        last,
        cap,
        points: list,
        colors: color_list,
        xo,
        yo,
        bounds,
    })
}

/// `n += (count - 1) * 5 + stops * 5` for a polyline the scan did not reject, where
/// `stops` is `count` when it has colours and 0 otherwise.
fn emit_polyline_tail_size(builder: &mut CodeBuilder, scan: &PolylineScan, n: &VirtualRegister) {
    let segs = builder.temporary_vreg();
    builder.emit(abi::subtract_immediate(&segs, &scan.count, 1));
    let five = builder.temporary_vreg();
    builder.emit(abi::move_immediate(
        &five,
        "Integer",
        &EDGE_FLOATS.to_string(),
    ));
    builder.emit(abi::multiply_registers(&segs, &segs, &five));
    builder.emit(abi::add_registers(n, n, &segs));
    let plain = builder.label("canvas_geo_pl_nostops");
    builder.emit(abi::compare_immediate(&scan.given, "0"));
    builder.emit(abi::branch_eq(&plain));
    let stops = builder.temporary_vreg();
    builder.emit(abi::multiply_registers(&stops, &scan.count, &five));
    builder.emit(abi::add_registers(n, n, &stops));
    builder.emit(abi::label(&plain));
}

/// `__canvas_polylineHeader` and `__canvas_polylineTail` (plan-154-A).
fn fill_polyline(
    builder: &mut CodeBuilder,
    out: &VirtualRegister,
    rec: &VirtualRegister,
    record: &ParameterType,
    view: &PaintView,
    done: &str,
) -> Result<(), String> {
    let empty = builder.label("canvas_geo_pl_empty");
    let scan = emit_polyline_scan(builder, rec, record, view, &empty, true)?;
    let [min_x, min_y, max_x, max_y] = scan.bounds.clone().expect("asked for bounds");

    put_const(builder, out, 0, KIND_POLYLINE);
    put(builder, out, 6, &scan.half);
    let cap_f = builder.temporary_fp_vreg();
    builder.emit(abi::signed_convert_to_float_d(&cap_f, &scan.cap));
    put(builder, out, SLOT_CAP, &cap_f);
    emit_paint_header(builder, out, view, &scan.half)?;
    // `__canvas_strokeAsFill`: the stroke colour into the fill slots, half = -1.
    for k in 0..4 {
        let v = fload(builder, out, (12 + k) * 8);
        put(builder, out, 8 + k, &v);
    }
    put_const(builder, out, 7, -1.0);
    // `toFloat(__CANVAS_GEO_HEADER + (count - 1) * 5 + stops * 5)`.
    let length = builder.temporary_vreg();
    builder.emit(abi::move_immediate(
        &length,
        "Integer",
        &GEO_HEADER.to_string(),
    ));
    emit_polyline_tail_size(builder, &scan, &length);
    let length_f = builder.temporary_fp_vreg();
    builder.emit(abi::signed_convert_to_float_d(&length_f, &length));
    put(builder, out, 1, &length_f);
    let stops = builder.temporary_vreg();
    let stops_done = builder.label("canvas_geo_pl_stopcount");
    builder.emit(abi::move_immediate(&stops, "Integer", "0"));
    builder.emit(abi::compare_immediate(&scan.given, "0"));
    builder.emit(abi::branch_eq(&stops_done));
    builder.emit(abi::move_register(&stops, &scan.count));
    builder.emit(abi::label(&stops_done));
    let stops_f = builder.temporary_fp_vreg();
    builder.emit(abi::signed_convert_to_float_d(&stops_f, &stops));
    put(builder, out, SLOT_GRADIENT_COUNT, &stops_f);
    put_const(builder, out, SLOT_GRADIENT_KIND, 0.0);
    let segs = builder.temporary_vreg();
    builder.emit(abi::subtract_immediate(&segs, &scan.count, 1));
    let segs_f = builder.temporary_fp_vreg();
    builder.emit(abi::signed_convert_to_float_d(&segs_f, &segs));
    put(builder, out, 20, &segs_f);
    let first_f = builder.temporary_fp_vreg();
    builder.emit(abi::signed_convert_to_float_d(&first_f, &scan.first));
    put(builder, out, SLOT_POLYLINE_FIRST, &first_f);
    let last_f = builder.temporary_fp_vreg();
    builder.emit(abi::signed_convert_to_float_d(&last_f, &scan.last));
    put(builder, out, SLOT_POLYLINE_LAST, &last_f);
    put_const(builder, out, 45, 0.0);
    put_const(builder, out, 46, 0.0);
    let style = emit_join_style(builder, view, &scan.half)?;
    let pad = polyline_pad(builder, &scan.half, &scan.cap, &style);
    let bx0 = fsub(builder, &min_x, &pad);
    let by0 = fsub(builder, &min_y, &pad);
    let bx1 = fadd(builder, &max_x, &pad);
    let by1 = fadd(builder, &max_y, &pad);
    put_bounds(builder, out, [&bx0, &by0, &bx1, &by1]);

    // The segments: `x0, y0, dx, dy, invLenSq` for point i to point i + 1 -- the polygon
    // layout without its wrap edge.
    let cursor = builder.temporary_vreg();
    builder.emit(abi::add_immediate(&cursor, out, GEO_HEADER * 8));
    let i = builder.temporary_vreg();
    let j = builder.temporary_vreg();
    let a = builder.temporary_vreg();
    let b = builder.temporary_vreg();
    let edge_head = builder.label("canvas_geo_pl_edge");
    let edge_done = builder.label("canvas_geo_pl_edges_done");
    builder.emit(abi::move_immediate(&i, "Integer", "0"));
    builder.emit(abi::label(&edge_head));
    builder.emit(abi::add_immediate(&j, &i, 1));
    builder.emit(abi::compare_registers(&j, &scan.count));
    builder.emit(abi::branch_ge(&edge_done));
    emit_list_element(builder, &a, &scan.points, &i);
    emit_list_element(builder, &b, &scan.points, &j);
    let ax = fload(builder, &a, scan.xo);
    let ay = fload(builder, &a, scan.yo);
    let bx = fload(builder, &b, scan.xo);
    let by = fload(builder, &b, scan.yo);
    let dx = fsub(builder, &bx, &ax);
    let dy = fsub(builder, &by, &ay);
    let len_sq = fused_len_sq(builder, &dx, &dy);
    builder.emit(abi::store_double(&ax, &cursor, 0));
    builder.emit(abi::store_double(&ay, &cursor, 8));
    builder.emit(abi::store_double(&dx, &cursor, 16));
    builder.emit(abi::store_double(&dy, &cursor, 24));
    let inv = builder.temporary_fp_vreg();
    let degenerate = builder.label("canvas_geo_pl_degenerate");
    let stored = builder.label("canvas_geo_pl_inv_store");
    let zero = fconst(builder, 0.0);
    let positive = builder.label("canvas_geo_pl_positive");
    builder.emit(abi::float_compare_d(&len_sq, &zero));
    builder.emit(abi::branch_gt(&positive));
    builder.emit(abi::branch(&degenerate));
    builder.emit(abi::label(&positive));
    let one = fconst(builder, 1.0);
    builder.emit(abi::float_divide_d(&inv, &one, &len_sq));
    builder.emit(abi::branch(&stored));
    builder.emit(abi::label(&degenerate));
    let zero = fconst(builder, 0.0);
    builder.emit(abi::float_move_d_from_d(&inv, &zero));
    builder.emit(abi::label(&stored));
    builder.emit(abi::store_double(&inv, &cursor, 32));
    builder.emit(abi::add_immediate(&cursor, &cursor, EDGE_FLOATS * 8));
    builder.emit(abi::move_register(&i, &j));
    builder.emit(abi::branch(&edge_head));
    builder.emit(abi::label(&edge_done));

    // plan-163-B: the join records of the open path, between the segments and the
    // stops (`__canvas_polylineTail`), then their count.
    let no_joins = builder.label("canvas_geo_pl_no_joins");
    branch_unless_joined(builder, &style, &scan.half, &no_joins);
    let segments = builder.temporary_vreg();
    builder.emit(abi::add_immediate(&segments, out, GEO_HEADER * 8));
    emit_ring_joins(
        builder, &segments, &segs, false, &cursor, &scan.half, &style,
    );
    let reach = emit_join_reach(builder, &scan.half, &style);
    emit_put_joins(builder, out, &segs, &length, &reach);
    builder.emit(abi::label(&no_joins));

    // plan-163-B: the dash tail (`__canvas_polylineDashes`), between the join records and
    // the stops.
    let solid = builder.label("canvas_geo_pl_solid");
    let pattern = emit_dash_pattern(builder, view, &scan.half, &solid)?;
    emit_dash_tail(
        builder,
        out,
        &cursor,
        view,
        &pattern,
        &scan.cap,
        |builder, cursor| {
            let segments = builder.temporary_vreg();
            builder.emit(abi::add_immediate(&segments, out, GEO_HEADER * 8));
            emit_ring_dash_data(builder, &segments, &segs, false, cursor);
        },
    )?;
    builder.emit(abi::label(&solid));
    // plan-163-C: a path clip's edges, between the dash tail and the stops.
    emit_clip_edges(builder, out, view, &cursor)?;

    // The colours, one stop per point: `toFloat(k) / toFloat(count - 1)`, then colour k
    // -- `colors[k]`, or `paint.stroke` past the end of the list.
    builder.emit(abi::compare_immediate(&scan.given, "0"));
    builder.emit(abi::branch_eq(done));
    let color_type = record_type(builder, &scan.colors.element)?;
    let stroke = builder.temporary_vreg();
    let stroke_type = emit_field_block(builder, &stroke, &view.paint, &view.paint_type, "stroke")?;
    let mut channels = Vec::new();
    for channel in ["red", "green", "blue", "alpha"] {
        let off = scalar_offset(builder, &color_type, channel)?;
        if off != scalar_offset(builder, &stroke_type, channel)? {
            return Err(format!(
                "canvas: PolyLine.colors and Paint.stroke disagree on where '{channel}' is"
            ));
        }
        channels.push(off);
    }
    let step = emit_transparency_step(builder, view)?;
    let k = builder.temporary_vreg();
    let c = builder.temporary_vreg();
    let stop_head = builder.label("canvas_geo_pl_stop");
    let from_stroke = builder.label("canvas_geo_pl_stop_stroke");
    let picked = builder.label("canvas_geo_pl_stop_picked");
    builder.emit(abi::move_immediate(&k, "Integer", "0"));
    builder.emit(abi::label(&stop_head));
    builder.emit(abi::compare_registers(&k, &scan.count));
    builder.emit(abi::branch_ge(done));
    let k_f = builder.temporary_fp_vreg();
    builder.emit(abi::signed_convert_to_float_d(&k_f, &k));
    let offset = fdiv(builder, &k_f, &segs_f);
    builder.emit(abi::store_double(&offset, &cursor, 0));
    builder.emit(abi::compare_registers(&k, &scan.given));
    builder.emit(abi::branch_ge(&from_stroke));
    emit_list_element(builder, &c, &scan.colors, &k);
    builder.emit(abi::branch(&picked));
    builder.emit(abi::label(&from_stroke));
    builder.emit(abi::move_register(&c, &stroke));
    builder.emit(abi::label(&picked));
    for (n, off) in channels.iter().enumerate() {
        let byte = builder.temporary_vreg();
        builder.emit(abi::load_u8(&byte, &c, *off));
        // plan-163-C: the alpha faded as the stroke's is.
        if n == 3 {
            emit_fade_alpha(builder, &byte, &step);
        }
        let f = builder.temporary_fp_vreg();
        builder.emit(abi::signed_convert_to_float_d(&f, &byte));
        builder.emit(abi::store_double(&f, &cursor, 8 * (n + 1)));
    }
    builder.emit(abi::add_immediate(&cursor, &cursor, EDGE_FLOATS * 8));
    builder.emit(abi::add_immediate(&k, &k, 1));
    builder.emit(abi::branch(&stop_head));

    builder.emit(abi::label(&empty));
    put_empty(builder, out);
    builder.emit(abi::branch(done));
    Ok(())
}

/// The six kinds, as `(kind, union tag, record type)`.
fn native_kinds(builder: &CodeBuilder) -> Result<Vec<(Kind, usize, ParameterType)>, String> {
    Kind::ALL
        .iter()
        .map(|kind| variant(builder, kind.variant()).map(|(tag, record)| (*kind, tag, record)))
        .collect()
}

/// `n` = the length in floats of the record `canvas::geoBuild` would build for the
/// `DrawItem` data union at `item`, or 0 when it is not one this builds. Allocates
/// nothing and calls nothing.
pub(super) fn emit_record_size(
    builder: &mut CodeBuilder,
    item: &VirtualRegister,
    n: &VirtualRegister,
) -> Result<(), String> {
    let paint_type = record_type(builder, &ParameterType::named("Paint"))?;
    let kinds = native_kinds(builder)?;
    let tag = builder.temporary_vreg();
    let rec = builder.temporary_vreg();
    builder.emit(abi::load_u64(&tag, item, 0));
    builder.emit(abi::add_immediate(&rec, item, 16));
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
        builder.emit(abi::move_immediate(n, "Integer", &GEO_HEADER.to_string()));
        // plan-163-A: five floats per point of every ring of at least 2 points.
        if *kind == Kind::Polygon {
            let rings = builder.temporary_vreg();
            let rings_type = emit_field_block(builder, &rings, &rec, record, "contours")?;
            let ring_count = builder.temporary_vreg();
            builder.emit(abi::load_u64(&ring_count, &rings, COLLECTION_OFFSET_COUNT));
            let outer = emit_list_view(builder, &rings, &rings_type)?;
            let r = builder.temporary_vreg();
            let ring = builder.temporary_vreg();
            let count = builder.temporary_vreg();
            let five = builder.temporary_vreg();
            builder.emit(abi::move_immediate(
                &five,
                "Integer",
                &EDGE_FLOATS.to_string(),
            ));
            let head = builder.label("canvas_geo_size_ring");
            let next = builder.label("canvas_geo_size_ring_next");
            let edges_sized = builder.label("canvas_geo_size_edges_done");
            builder.emit(abi::move_immediate(&r, "Integer", "0"));
            builder.emit(abi::label(&head));
            builder.emit(abi::compare_registers(&r, &ring_count));
            builder.emit(abi::branch_ge(&edges_sized));
            emit_list_element(builder, &ring, &outer, &r);
            builder.emit(abi::load_u64(&count, &ring, COLLECTION_OFFSET_COUNT));
            builder.emit(abi::compare_immediate(&count, "2"));
            builder.emit(abi::branch_lt(&next));
            builder.emit(abi::multiply_registers(&count, &count, &five));
            builder.emit(abi::add_registers(n, n, &count));
            builder.emit(abi::label(&next));
            builder.emit(abi::add_immediate(&r, &r, 1));
            builder.emit(abi::branch(&head));
            builder.emit(abi::label(&edges_sized));
            // plan-163-B: one join record per edge -- eight floats per point of every
            // contributing ring -- when the stroke joins.
            let half = emit_stroke_half(builder, &view)?;
            let style = emit_join_style(builder, &view, &half)?;
            let no_joins = builder.label("canvas_geo_size_poly_no_joins");
            branch_unless_joined(builder, &style, &half, &no_joins);
            let edges = builder.temporary_vreg();
            builder.emit(abi::move_immediate(&edges, "Integer", "0"));
            let ring_head = builder.label("canvas_geo_size_join_ring");
            let ring_next = builder.label("canvas_geo_size_join_ring_next");
            let summed = builder.label("canvas_geo_size_join_summed");
            builder.emit(abi::move_immediate(&r, "Integer", "0"));
            builder.emit(abi::label(&ring_head));
            builder.emit(abi::compare_registers(&r, &ring_count));
            builder.emit(abi::branch_ge(&summed));
            emit_list_element(builder, &ring, &outer, &r);
            builder.emit(abi::load_u64(&count, &ring, COLLECTION_OFFSET_COUNT));
            builder.emit(abi::compare_immediate(&count, "2"));
            builder.emit(abi::branch_lt(&ring_next));
            builder.emit(abi::add_registers(&edges, &edges, &count));
            builder.emit(abi::label(&ring_next));
            builder.emit(abi::add_immediate(&r, &r, 1));
            builder.emit(abi::branch(&ring_head));
            builder.emit(abi::label(&summed));
            emit_add_join_floats(builder, &edges, n);
            builder.emit(abi::label(&no_joins));
            // plan-163-B: three dash floats per edge, when the header is a polygon (it has
            // an edge) and the stroke is dashed.
            let solid = builder.label("canvas_geo_size_poly_solid");
            let pattern = emit_dash_pattern(builder, &view, &half, &solid)?;
            builder.emit(abi::move_immediate(&edges, "Integer", "0"));
            let dash_head = builder.label("canvas_geo_size_dash_ring");
            let dash_next = builder.label("canvas_geo_size_dash_ring_next");
            let dash_summed = builder.label("canvas_geo_size_dash_summed");
            builder.emit(abi::move_immediate(&r, "Integer", "0"));
            builder.emit(abi::label(&dash_head));
            builder.emit(abi::compare_registers(&r, &ring_count));
            builder.emit(abi::branch_ge(&dash_summed));
            emit_list_element(builder, &ring, &outer, &r);
            builder.emit(abi::load_u64(&count, &ring, COLLECTION_OFFSET_COUNT));
            builder.emit(abi::compare_immediate(&count, "2"));
            builder.emit(abi::branch_lt(&dash_next));
            builder.emit(abi::add_registers(&edges, &edges, &count));
            builder.emit(abi::label(&dash_next));
            builder.emit(abi::add_immediate(&r, &r, 1));
            builder.emit(abi::branch(&dash_head));
            builder.emit(abi::label(&dash_summed));
            builder.emit(abi::compare_immediate(&edges, "0"));
            builder.emit(abi::branch_eq(&solid));
            let data = builder.temporary_vreg();
            let three = builder.temporary_vreg();
            builder.emit(abi::move_immediate(
                &three,
                "Integer",
                &DASH_EDGE_FLOATS.to_string(),
            ));
            builder.emit(abi::multiply_registers(&data, &edges, &three));
            emit_add_dash_floats(builder, &pattern, &data, n);
            builder.emit(abi::label(&solid));
        }
        // plan-163-B: a sharp-cornered rectangle's four join records, and its dash tail.
        if *kind == Kind::Rect || *kind == Kind::Rounded {
            let no_joins = builder.label("canvas_geo_size_rect_no_joins");
            emit_rect_joined(
                builder,
                &rec,
                record,
                &view,
                *kind == Kind::Rounded,
                &no_joins,
            )?;
            builder.emit(abi::add_immediate(n, n, 4 * JOIN_FLOATS));
            builder.emit(abi::label(&no_joins));
            let solid = builder.label("canvas_geo_size_rect_solid");
            emit_rect_dash_size(
                builder,
                &rec,
                record,
                &view,
                *kind == Kind::Rounded,
                &solid,
                n,
            )?;
            builder.emit(abi::label(&solid));
        }
        // plan-163-B: a circle's and a line's dash tails.
        if *kind == Kind::Circle {
            let solid = builder.label("canvas_geo_size_circle_solid");
            let radius = fload(builder, &rec, scalar_offset(builder, record, "radius")?);
            branch_if_not_positive(builder, &radius, &solid);
            let half = emit_stroke_half(builder, &view)?;
            let pattern = emit_dash_pattern(builder, &view, &half, &solid)?;
            let data = builder.temporary_vreg();
            builder.emit(abi::move_immediate(&data, "Integer", "0"));
            emit_add_dash_floats(builder, &pattern, &data, n);
            builder.emit(abi::label(&solid));
        }
        if *kind == Kind::Line {
            let solid = builder.label("canvas_geo_size_line_solid");
            let half = emit_stroke_half(builder, &view)?;
            let pattern = emit_dash_pattern(builder, &view, &half, &solid)?;
            let data = builder.temporary_vreg();
            builder.emit(abi::move_immediate(
                &data,
                "Integer",
                &DASH_EDGE_FLOATS.to_string(),
            ));
            emit_add_dash_floats(builder, &pattern, &data, n);
            builder.emit(abi::label(&solid));
        }
        // plan-154-A: a polyline the header would answer empty for is 51 floats, and
        // the scan branches straight to `sized` with `n` already 51.
        if *kind == Kind::PolyLine {
            let scan = emit_polyline_scan(builder, &rec, record, &view, &sized, false)?;
            emit_polyline_tail_size(builder, &scan, n);
            // plan-163-B: one join record per segment.
            let style = emit_join_style(builder, &view, &scan.half)?;
            let no_joins = builder.label("canvas_geo_size_pl_no_joins");
            branch_unless_joined(builder, &style, &scan.half, &no_joins);
            let segs = builder.temporary_vreg();
            builder.emit(abi::subtract_immediate(&segs, &scan.count, 1));
            emit_add_join_floats(builder, &segs, n);
            builder.emit(abi::label(&no_joins));
            // plan-163-B: three dash floats per segment.
            let solid = builder.label("canvas_geo_size_pl_solid");
            let pattern = emit_dash_pattern(builder, &view, &scan.half, &solid)?;
            let data = builder.temporary_vreg();
            let three = builder.temporary_vreg();
            builder.emit(abi::move_immediate(
                &three,
                "Integer",
                &DASH_EDGE_FLOATS.to_string(),
            ));
            builder.emit(abi::subtract_immediate(&data, &scan.count, 1));
            builder.emit(abi::multiply_registers(&data, &data, &three));
            emit_add_dash_floats(builder, &pattern, &data, n);
            builder.emit(abi::label(&solid));
        }
        // plan-163-C: a path clip's edges, five floats a point, on a record whose header
        // is not the empty one (only `__canvas_paintHeader` writes the clip): a positive
        // size, radius or half-width, a polygon with an edge; a polyline the scan above
        // already left for `sized` when its header is empty.
        let unclipped = builder.label("canvas_geo_size_unclipped");
        match kind {
            Kind::Rect | Kind::Rounded => {
                let w = fload(builder, &rec, scalar_offset(builder, record, "w")?);
                let h = fload(builder, &rec, scalar_offset(builder, record, "h")?);
                branch_if_not_positive(builder, &w, &unclipped);
                branch_if_not_positive(builder, &h, &unclipped);
            }
            Kind::Circle => {
                let radius = fload(builder, &rec, scalar_offset(builder, record, "radius")?);
                branch_if_not_positive(builder, &radius, &unclipped);
            }
            Kind::Line => {
                let half = emit_stroke_half(builder, &view)?;
                branch_if_not_positive(builder, &half, &unclipped);
            }
            Kind::Polygon => {
                builder.emit(abi::compare_immediate(n, &GEO_HEADER.to_string()));
                builder.emit(abi::branch_le(&unclipped));
            }
            Kind::PolyLine => {}
        }
        emit_clip_size(builder, &view, n)?;
        builder.emit(abi::label(&unclipped));
        builder.emit(abi::branch(&sized));
    }
    builder.emit(abi::label(&unsupported));
    builder.emit(abi::move_immediate(n, "Integer", "0"));
    builder.emit(abi::label(&sized));
    Ok(())
}

/// Write the record for the `DrawItem` at `item` at `out` (the first float), which
/// must have room for the length [`emit_record_size`] answered — call it only when
/// that was non-zero. Allocates nothing and calls nothing.
pub(super) fn emit_fill_record(
    builder: &mut CodeBuilder,
    item: &VirtualRegister,
    out: &VirtualRegister,
) -> Result<(), String> {
    let paint_type = record_type(builder, &ParameterType::named("Paint"))?;
    let kinds = native_kinds(builder)?;
    let done = builder.label("canvas_geo_done");
    let zero = builder.temporary_vreg();
    builder.emit(abi::move_immediate(&zero, "Integer", "0"));
    for slot in 0..GEO_HEADER {
        builder.emit(abi::store_u64(&zero, out, slot * 8));
    }
    // plan-163-C: the item's paint, for its path-clip edges once the kind is written.
    let paint_slot = builder.allocate_stack_object("canvas_geo_fill_paint", 8);
    let tag = builder.temporary_vreg();
    let rec = builder.temporary_vreg();
    builder.emit(abi::load_u64(&tag, item, 0));
    builder.emit(abi::add_immediate(&rec, item, 16));
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
        builder.emit(abi::store_u64(&paint, abi::stack_pointer(), paint_slot));
        let view = PaintView {
            paint,
            paint_type: paint_type.clone(),
        };
        match kind {
            Kind::Rect => fill_rect(builder, out, &rec, record, &view, false, &done)?,
            Kind::Rounded => fill_rect(builder, out, &rec, record, &view, true, &done)?,
            Kind::Circle => fill_circle(builder, out, &rec, record, &view, &done)?,
            Kind::Line => fill_line(builder, out, &rec, record, &view, &done)?,
            Kind::Polygon => fill_polygon(builder, out, &rec, record, &view, &done)?,
            Kind::PolyLine => fill_polyline(builder, out, &rec, record, &view, &done)?,
        }
    }
    builder.emit(abi::label(&done));
    // plan-163-C: a path clip's edges at the record's end -- after every other tail; the
    // polyline, whose stops come last, wrote them before its stops.
    let clipped = builder.label("canvas_geo_clip_written");
    let kind_f = fload(builder, out, 0);
    let polyline = fconst(builder, KIND_POLYLINE);
    builder.emit(abi::float_compare_d(&kind_f, &polyline));
    builder.emit(abi::branch_eq(&clipped));
    let mode = fload(builder, out, SLOT_CLIP_MODE * 8);
    let two = fconst(builder, 2.0);
    builder.emit(abi::float_compare_d(&mode, &two));
    builder.emit(abi::branch_ne(&clipped));
    let paint = builder.temporary_vreg();
    builder.emit(abi::load_u64(&paint, abi::stack_pointer(), paint_slot));
    let view = PaintView {
        paint,
        paint_type: paint_type.clone(),
    };
    let cursor = emit_record_end(builder, out);
    emit_clip_edges(builder, out, &view, &cursor)?;
    builder.emit(abi::label(&clipped));
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
    let item_slot = builder.allocate_stack_object("canvas_geo_item", 8);
    builder.emit(abi::store_u64(&incoming, abi::stack_pointer(), item_slot));
    let n_slot = builder.allocate_stack_object("canvas_geo_n", 8);

    // Phase 1: is the item one this builds, and how many floats is its record?
    let item = builder.temporary_vreg();
    let n = builder.temporary_vreg();
    builder.emit(abi::load_u64(&item, abi::stack_pointer(), item_slot));
    emit_record_size(builder, &item, &n)?;
    builder.emit(abi::store_u64(&n, abi::stack_pointer(), n_slot));

    // Phase 2: the one allocation. Everything after it is re-read from the stack.
    let list_slot = reserve_float_list(builder, n_slot)?;

    // Phase 3: fill it.
    let done = builder.label("canvas_geo_built");
    let n = builder.temporary_vreg();
    builder.emit(abi::load_u64(&n, abi::stack_pointer(), n_slot));
    builder.emit(abi::compare_immediate(&n, "0"));
    builder.emit(abi::branch_eq(&done));
    let out = builder.temporary_vreg();
    builder.emit(abi::load_u64(&out, abi::stack_pointer(), list_slot));
    // A fixed-width list is entry-free: its floats start right after the header.
    builder.emit(abi::add_immediate(&out, &out, COLLECTION_HEADER_SIZE));
    let item = builder.temporary_vreg();
    builder.emit(abi::load_u64(&item, abi::stack_pointer(), item_slot));
    emit_fill_record(builder, &item, &out)?;
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
        assert_eq!(value("__CANVAS_GEO_POLYLINE"), KIND_POLYLINE);
        assert_eq!(
            value("__CANVAS_GEO_POLYLINE_FIRST"),
            SLOT_POLYLINE_FIRST as f64
        );
        assert_eq!(
            value("__CANVAS_GEO_POLYLINE_LAST"),
            SLOT_POLYLINE_LAST as f64
        );
        assert_eq!(
            value("__CANVAS_GEO_GRADIENT_COUNT"),
            SLOT_GRADIENT_COUNT as f64
        );
        assert_eq!(
            value("__CANVAS_GEO_GRADIENT_KIND"),
            SLOT_GRADIENT_KIND as f64
        );
        assert_eq!(value("__CANVAS_GEO_CAP"), SLOT_CAP as f64);
        assert_eq!(value("__CANVAS_GEO_POLYGON_RULE"), SLOT_POLYGON_RULE as f64);
        assert_eq!(value("__CANVAS_GEO_JOIN_COUNT"), SLOT_JOIN_COUNT as f64);
        assert_eq!(value("__CANVAS_GEO_JOIN_REACH"), SLOT_JOIN_REACH as f64);
        assert_eq!(value("__CANVAS_GEO_DASH_COUNT"), SLOT_DASH_COUNT as f64);
        assert_eq!(value("__CANVAS_GEO_DASH_PHASE"), SLOT_DASH_PHASE as f64);
        assert_eq!(value("__CANVAS_GEO_CLIP_MODE"), SLOT_CLIP_MODE as f64);
        assert_eq!(value("__CANVAS_GEO_CLIP_RULE"), SLOT_CLIP_RULE as f64);
        assert_eq!(
            value("__CANVAS_GEO_CLIP_EDGE_COUNT"),
            SLOT_CLIP_EDGE_COUNT as f64
        );
    }
}
