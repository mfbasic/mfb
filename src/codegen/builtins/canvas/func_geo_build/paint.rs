//! plan-176-R: the native geometry builder's the paint header — offsets, stroke, colour,
//! transparency, clip shapes and padding. The slot constants, float helpers and
//! `lower_geo_build` live in `builtins/canvas/func_geo_build/mod.rs`.

use super::*;

// ---- The paint -----------------------------------------------------------------------

/// The addresses a `Paint` is read through, computed once per item.
pub(super) struct PaintView {
    pub(super) paint: VirtualRegister,
    pub(super) paint_type: ParameterType,
    /// plan-172-D: two frame slots holding the item's build offset as doubles —
    /// where its record puts the local origin, which is its `position` (the native
    /// builder only takes an item whose transform is all-zero). `paint.clip` is in
    /// local space, and so are the points of a `Line`, `Polygon` and `PolyLine`
    /// (plan-172-F), so each is read through [`placed_point`] or [`moved_point`],
    /// which add it. The add
    /// happens whatever the offset, as it does in `__canvas_placedClip`, so the two
    /// builders agree on a `-0.0` coordinate.
    pub(super) offset: (usize, usize),
}

/// The view's build offset, loaded into two registers.
///
/// plan-172-F: a point-list walk loads it ONCE, ahead of its loop, and adds it to each
/// point with [`moved_point`]. Reading it back from the frame for every point
/// ([`placed_point`], which the clip walks use) cost the moving-polygon rows of
/// `tools/canvas-bench` 3 to 6% of their build phase; a polygon reads each of its points
/// three times.
pub(super) fn build_offset(
    builder: &mut CodeBuilder,
    view: &PaintView,
) -> (VirtualRegister, VirtualRegister) {
    let ox = builder.temporary_fp_vreg();
    let oy = builder.temporary_fp_vreg();
    builder.emit(abi::load_double(&ox, abi::stack_pointer(), view.offset.0));
    builder.emit(abi::load_double(&oy, abi::stack_pointer(), view.offset.1));
    (ox, oy)
}

/// The local point at `q`, moved by an offset already in registers:
/// `(q.x + ox, q.y + oy)`.
pub(super) fn moved_point(
    builder: &mut CodeBuilder,
    offset: &(VirtualRegister, VirtualRegister),
    q: &VirtualRegister,
    xo: usize,
    yo: usize,
) -> (VirtualRegister, VirtualRegister) {
    let x = fload(builder, q, xo);
    let y = fload(builder, q, yo);
    (fadd(builder, &x, &offset.0), fadd(builder, &y, &offset.1))
}

/// The local point at `q`, moved by the view's build offset: `(q.x + ox, q.y + oy)`.
pub(super) fn placed_point(
    builder: &mut CodeBuilder,
    view: &PaintView,
    q: &VirtualRegister,
    xo: usize,
    yo: usize,
) -> (VirtualRegister, VirtualRegister) {
    let x = fload(builder, q, xo);
    let y = fload(builder, q, yo);
    let ox = builder.temporary_fp_vreg();
    let oy = builder.temporary_fp_vreg();
    builder.emit(abi::load_double(&ox, abi::stack_pointer(), view.offset.0));
    builder.emit(abi::load_double(&oy, abi::stack_pointer(), view.offset.1));
    (fadd(builder, &x, &ox), fadd(builder, &y, &oy))
}

/// Fill the build-offset slots for the item at `rec`: its `position`. The native
/// builder only takes an item whose transform is all-zero, so every kind it builds is
/// built at its position (plan-172-D, E and F between them position all five).
pub(super) fn emit_build_offset(
    builder: &mut CodeBuilder,
    rec: &VirtualRegister,
    record: &ParameterType,
    offset: (usize, usize),
) -> Result<(), String> {
    let position = builder.temporary_vreg();
    let position_type = emit_field_block(builder, &position, rec, record, "position")?;
    let x = fload(
        builder,
        &position,
        scalar_offset(builder, &position_type, "x")?,
    );
    let y = fload(
        builder,
        &position,
        scalar_offset(builder, &position_type, "y")?,
    );
    builder.emit(abi::store_double(&x, abi::stack_pointer(), offset.0));
    builder.emit(abi::store_double(&y, abi::stack_pointer(), offset.1));
    Ok(())
}

/// `__canvas_strokeHalf(paint)`: `-1.0` when the stroke's alpha is 0 or the width is
/// not positive, else `strokeWidth / 2.0`.
pub(super) fn emit_stroke_half(
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
pub(super) fn emit_transparency_step(
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
pub(super) fn emit_fade_alpha(
    builder: &mut CodeBuilder,
    alpha: &VirtualRegister,
    step: &VirtualRegister,
) {
    // plan-175-A: at step 0 -- every paint with no transparency -- the fade is the
    // identity, `(a * 255 + 127) / 255 = a` for every byte `a`, so the divide (the
    // costliest instruction of a refresh) is skipped.
    let same = builder.label("canvas_geo_fade_same");
    builder.emit(abi::compare_immediate(step, "0"));
    builder.emit(abi::branch_eq(&same));
    let keep = builder.temporary_vreg();
    builder.emit(abi::move_immediate(&keep, "Integer", "255"));
    builder.emit(abi::subtract_registers(&keep, &keep, step));
    builder.emit(abi::multiply_registers(alpha, alpha, &keep));
    builder.emit(abi::add_immediate(alpha, alpha, 127));
    let d = builder.temporary_vreg();
    builder.emit(abi::move_immediate(&d, "Integer", "255"));
    builder.emit(abi::signed_divide_registers(alpha, alpha, &d));
    builder.emit(abi::label(&same));
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
        let (x, y) = placed_point(builder, view, &q, xo, yo);
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
pub(super) fn emit_clip_size(
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
pub(super) fn emit_clip_edges(
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
        let (ax, ay) = placed_point(builder, view, &a, xo, yo);
        let (bx, by) = placed_point(builder, view, &b, xo, yo);
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
pub(super) fn emit_paint_header(
    builder: &mut CodeBuilder,
    out: &VirtualRegister,
    view: &PaintView,
    stroke_half: &VirtualRegister,
) -> Result<(), String> {
    put(builder, out, 7, stroke_half);
    emit_paint_appearance(builder, out, view)?;

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
        let (x, y) = placed_point(builder, view, &q, xo, yo);
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

    put_const(builder, out, 27, IDENTITY_ONE_BITS);
    put_const(builder, out, 30, IDENTITY_ONE_BITS);
    Ok(())
}

/// plan-175-A: the APPEARANCE slots `__canvas_paintHeader` writes, for a natively built
/// paint (no transform, fewer than two stops): both colours, the alphas faded by the
/// transparency (8–15), the blend mode's tag (26), and the gradient's kind and two
/// points (42–46, written whatever the stop count). Every slot here comes from a
/// field `func_item_lanes.rs` puts in the appearance lane and nothing else does, which
/// is what lets a record found by its geometry identity be refreshed by this alone
/// (`emit_refresh_record`).
pub(super) fn emit_paint_appearance(
    builder: &mut CodeBuilder,
    out: &VirtualRegister,
    view: &PaintView,
) -> Result<(), String> {
    // plan-163-C: both alphas faded by `transparency` (`__canvas_fadeAlpha`).
    let step = emit_transparency_step(builder, view)?;
    put_color(builder, out, 8, view, "fill", &step)?;
    put_color(builder, out, 12, view, "stroke", &step)?;

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
    Ok(())
}

/// `__canvas_boundsHeader` for an identity transform: the four bounds, unmodified, and
/// the rim they carry. Every kind built here hands that function `__CANVAS_BOUNDS_RIM`
/// (a rectangle, a circle, a line, a polygon, a polyline: none passes a box whole), and
/// untransformed it stores the rim as it was handed, so the two slots are the constant.
pub(super) fn put_bounds(
    builder: &mut CodeBuilder,
    out: &VirtualRegister,
    bounds: [&VirtualRegister; 4],
) {
    for (k, value) in bounds.iter().enumerate() {
        put(builder, out, 16 + k, value);
    }
    put_const(builder, out, SLOT_RIM, BOUNDS_RIM);
    put_const(builder, out, SLOT_RIM + 1, BOUNDS_RIM);
}

/// `__canvas_capTag(paint.cap)` as a whole number: Round 1, Square 2, Butt 0.
pub(super) fn emit_cap_tag(
    builder: &mut CodeBuilder,
    view: &PaintView,
) -> Result<VirtualRegister, String> {
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
pub(super) fn cap_pad(
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
pub(super) fn dash_cap_reach(
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
pub(super) fn stroke_pad(
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
pub(super) fn polyline_pad(
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
pub(super) fn emit_paint_supported(
    builder: &mut CodeBuilder,
    view: &PaintView,
    item: &VirtualRegister,
    item_type: &ParameterType,
    unsupported: &str,
) -> Result<(), String> {
    // plan-172-C: the transform is the item's own field, not the paint's.
    let transform = builder.temporary_vreg();
    let transform_type = emit_field_block(builder, &transform, item, item_type, "transform")?;
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
