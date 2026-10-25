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
//! `Circle`, `Line`, `Polygon` or `PolyLine` whose paint has the all-zero
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

mod dashes;
mod fills;
mod joins;
mod paint;
mod polyline;
mod record;
use dashes::*;
use fills::*;
use joins::*;
use paint::*;
use polyline::*;
use record::*;
pub(super) use record::{
    emit_field_block, emit_list_element, emit_list_view, record_fields, record_type, variant,
};

/// `__CANVAS_GEO_HEADER`: the fixed header's length in floats.
pub(super) const GEO_HEADER: usize = 69;
/// `__CANVAS_GEO_RIM` (bug-1083): how much of the box in slots 16-19 is rim, on its x
/// sides and (the slot after) on its y sides.
const SLOT_RIM: usize = 67;
/// `__CANVAS_BOUNDS_RIM`: the rim every natively built kind's pad ends in.
const BOUNDS_RIM: f64 = 1.0;
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

// ---- The kinds -------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Rect,
    Circle,
    Line,
    Polygon,
    PolyLine,
}

impl Kind {
    const ALL: [Kind; 5] = [
        Kind::Rect,
        Kind::Circle,
        Kind::Line,
        Kind::Polygon,
        Kind::PolyLine,
    ];

    fn variant(self) -> &'static str {
        match self {
            Kind::Rect => "Rectangle",
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
    let offset = (
        builder.allocate_stack_object("canvas_geo_size_ox", 8),
        builder.allocate_stack_object("canvas_geo_size_oy", 8),
    );
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
        emit_build_offset(builder, &rec, record, offset)?;
        let view = PaintView {
            paint,
            paint_type: paint_type.clone(),
            offset,
        };
        emit_paint_supported(builder, &view, &rec, record, &unsupported)?;
        if *kind == Kind::Rect {
            emit_rect_inputs(builder, &rec, record, Some(&unsupported))?;
        }
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
        if *kind == Kind::Rect {
            let no_joins = builder.label("canvas_geo_size_rect_no_joins");
            emit_rect_joined(builder, &rec, record, &view, &no_joins)?;
            builder.emit(abi::add_immediate(n, n, 4 * JOIN_FLOATS));
            builder.emit(abi::label(&no_joins));
            let solid = builder.label("canvas_geo_size_rect_solid");
            emit_rect_dash_size(builder, &rec, record, &view, &solid, n)?;
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
            Kind::Rect => {
                let RectInputs { w, h, .. } = emit_rect_inputs(builder, &rec, record, None)?;
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
    // plan-172-D: the item's build offset, filled by its kind's arm and read by every
    // clip point after it, the path-clip edges written past `done` included.
    let offset = (
        builder.allocate_stack_object("canvas_geo_fill_ox", 8),
        builder.allocate_stack_object("canvas_geo_fill_oy", 8),
    );
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
        emit_build_offset(builder, &rec, record, offset)?;
        let view = PaintView {
            paint,
            paint_type: paint_type.clone(),
            offset,
        };
        match kind {
            Kind::Rect => fill_rect(builder, out, &rec, record, &view, &done)?,
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
    // The offset slots still hold what this item's own arm stored.
    let view = PaintView {
        paint,
        paint_type: paint_type.clone(),
        offset,
    };
    let cursor = emit_record_end(builder, out);
    emit_clip_edges(builder, out, &view, &cursor)?;
    builder.emit(abi::label(&clipped));
    Ok(())
}

/// plan-175-A Phase 2: rewrite the APPEARANCE slots of the record at `out` for the
/// `DrawItem` at `item`, whose record it is up to its paint — found by the item's
/// geometry identity, so every slot the geometry decides already holds what a fresh
/// build writes. The slots are what `emit_paint_header` writes from appearance fields
/// (`func_item_lanes.rs`): both colours, faded by the transparency (8–15), the blend mode
/// (26), and the gradient's kind and points (42–46, written whatever the stop count; a
/// natively built item has fewer than two). A `Line` is drawn from its stroke, so its
/// fill slots are the stroke's (`__canvas_strokeAsFill`). A `PolyLine` keeps colours in
/// its tail too, so it is rebuilt whole, in place. Allocates nothing and calls nothing.
///
/// The verifier (`MFB_CANVAS_GEO_VERIFY`) checks every resolved record against a fresh
/// build, refreshed ones included: that is what holds this to "bit-identical".
///
/// It branches to `declined` for an item `canvas::geoBuild` does not build — the
/// MFBASIC walk's to rebuild in place — and, for a `PolyLine`, when the record the
/// item builds is not `count` floats. Whether an item is built here is decided by its
/// kind, its transform, its stop count and a rectangle's corners
/// ([`emit_record_size`]'s checks), all of them geometry, so a record found by the
/// item's geometry identity was built the way the item would be now; only those checks
/// are made, not the sizing.
pub(super) fn emit_refresh_record(
    builder: &mut CodeBuilder,
    item: &VirtualRegister,
    out: &VirtualRegister,
    count: &VirtualRegister,
    declined: &str,
) -> Result<(), String> {
    let paint_type = record_type(builder, &ParameterType::named("Paint"))?;
    let kinds = native_kinds(builder)?;
    let done = builder.label("canvas_geo_refresh_done");
    let offset = (
        builder.allocate_stack_object("canvas_geo_refresh_ox", 8),
        builder.allocate_stack_object("canvas_geo_refresh_oy", 8),
    );
    let tag = builder.temporary_vreg();
    let rec = builder.temporary_vreg();
    builder.emit(abi::load_u64(&tag, item, 0));
    builder.emit(abi::add_immediate(&rec, item, 16));
    let labels: Vec<String> = kinds
        .iter()
        .map(|_| builder.label("canvas_geo_refresh_kind"))
        .collect();
    for ((_, kind_tag, _), label) in kinds.iter().zip(&labels) {
        builder.emit(abi::compare_immediate(&tag, &kind_tag.to_string()));
        builder.emit(abi::branch_eq(label));
    }
    builder.emit(abi::branch(declined));
    for ((kind, _, record), label) in kinds.iter().zip(&labels) {
        builder.emit(abi::label(label));
        if *kind == Kind::PolyLine {
            let n = builder.temporary_vreg();
            emit_record_size(builder, item, &n)?;
            builder.emit(abi::compare_immediate(&n, "0"));
            builder.emit(abi::branch_eq(declined));
            builder.emit(abi::compare_registers(&n, count));
            builder.emit(abi::branch_ne(declined));
            emit_fill_record(builder, item, out)?;
            builder.emit(abi::branch(&done));
            continue;
        }
        let paint = builder.temporary_vreg();
        emit_field_block(builder, &paint, &rec, record, "paint")?;
        let view = PaintView {
            paint,
            paint_type: paint_type.clone(),
            offset,
        };
        emit_paint_supported(builder, &view, &rec, record, declined)?;
        if *kind == Kind::Rect {
            emit_rect_inputs(builder, &rec, record, Some(declined))?;
        }
        // An empty record (`put_empty`: a degenerate size, a stroke-only kind whose
        // stroke is not drawn) writes no paint, so it has nothing to refresh.
        let kind_f = fload(builder, out, 0);
        let none = fconst(builder, KIND_NONE);
        builder.emit(abi::float_compare_d(&kind_f, &none));
        builder.emit(abi::branch_eq(&done));
        emit_paint_appearance(builder, out, &view)?;
        if *kind == Kind::Line {
            for k in 0..4 {
                let v = fload(builder, out, (12 + k) * 8);
                put(builder, out, 8 + k, &v);
            }
        }
        builder.emit(abi::branch(&done));
    }
    builder.emit(abi::label(&done));
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
        assert_eq!(value("__CANVAS_GEO_RIM"), SLOT_RIM as f64);
        assert!(
            source.contains(&format!(
                "LET __CANVAS_BOUNDS_RIM AS Float = {BOUNDS_RIM:?}\n"
            )),
            "__CANVAS_BOUNDS_RIM is not the rim the native builder stores"
        );
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
