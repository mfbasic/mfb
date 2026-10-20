//! plan-176-R: the native geometry builder's fills — rectangles, circles, lines and polygons.
//! The slot constants, float helpers and `lower_geo_build` live in
//! `builtins/canvas/func_geo_build/mod.rs`.

use super::*;

/// `__canvas_rectHeader` (four square corners pass `corner = 0.0`).
pub(super) fn fill_rect(
    builder: &mut CodeBuilder,
    out: &VirtualRegister,
    rec: &VirtualRegister,
    record: &ParameterType,
    view: &PaintView,
    done: &str,
) -> Result<(), String> {
    let empty = builder.label("canvas_geo_rect_empty");
    let RectInputs { x, y, w, h, corner } = emit_rect_inputs(builder, rec, record, None)?;
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

/// A rectangle's geometry inputs: its top-left corner, its extent and its corner
/// radius, as `__canvas_rectItem` hands them to `__canvas_rectHeader`.
pub(super) struct RectInputs {
    pub(super) x: VirtualRegister,
    pub(super) y: VirtualRegister,
    pub(super) w: VirtualRegister,
    pub(super) h: VirtualRegister,
    pub(super) corner: VirtualRegister,
}

/// plan-172-E: read a `Rectangle`'s inputs.
///
/// The top-left corner is `position` plus where `bind` puts that corner
/// (`__canvas_bindOffset`): `0.0` for `TopLeft`, `0.0 - extent / 2.0` for `Center`. The
/// add happens for both, as it does in MFBASIC, so a `-0.0` coordinate comes out the
/// same.
///
/// This builder draws only the exact record, and only the cases whose radius needs
/// no resolving: **four equal radii that are either all `<= 0.0` or all `> 1.0`**. The
/// value is passed on as it is and the caller clamps it to half the shorter side —
/// the arithmetic a rounded rectangle's one radius always had. Anything else (unequal
/// radii, or a fraction of the half-size) branches to `decline`, and the MFBASIC
/// builder resolves it. The size pass passes `decline`; the fill pass, which runs only
/// for an item the size pass took, passes `None`.
pub(super) fn emit_rect_inputs(
    builder: &mut CodeBuilder,
    rec: &VirtualRegister,
    record: &ParameterType,
    decline: Option<&str>,
) -> Result<RectInputs, String> {
    let position = builder.temporary_vreg();
    let position_type = emit_field_block(builder, &position, rec, record, "position")?;
    let px = fload(
        builder,
        &position,
        scalar_offset(builder, &position_type, "x")?,
    );
    let py = fload(
        builder,
        &position,
        scalar_offset(builder, &position_type, "y")?,
    );
    let size = builder.temporary_vreg();
    let size_type = emit_field_block(builder, &size, rec, record, "size")?;
    let w = fload(builder, &size, scalar_offset(builder, &size_type, "width")?);
    let h = fload(
        builder,
        &size,
        scalar_offset(builder, &size_type, "height")?,
    );

    let (bind_index, bind_type) = field(builder, record, "bind")?;
    let center = enum_ordinal(builder, &bind_type, "Center")?;
    let bind = builder.temporary_vreg();
    builder.emit(abi::load_u64(&bind, rec, 8 * bind_index));
    let lx = builder.temporary_fp_vreg();
    let ly = builder.temporary_fp_vreg();
    let zero = fconst(builder, 0.0);
    builder.emit(abi::float_move_d_from_d(&lx, &zero));
    builder.emit(abi::float_move_d_from_d(&ly, &zero));
    let top_left = builder.label("canvas_geo_rect_top_left");
    builder.emit(abi::compare_immediate(&bind, &center.to_string()));
    builder.emit(abi::branch_ne(&top_left));
    let two = fconst(builder, 2.0);
    let half_w = fdiv(builder, &w, &two);
    let back_x = fsub(builder, &zero, &half_w);
    builder.emit(abi::float_move_d_from_d(&lx, &back_x));
    let half_h = fdiv(builder, &h, &two);
    let back_y = fsub(builder, &zero, &half_h);
    builder.emit(abi::float_move_d_from_d(&ly, &back_y));
    builder.emit(abi::label(&top_left));
    let x = fadd(builder, &px, &lx);
    let y = fadd(builder, &py, &ly);

    let corner = fload(builder, rec, scalar_offset(builder, record, "radiusTL")?);
    if let Some(decline) = decline {
        for other in ["radiusTR", "radiusBL", "radiusBR"] {
            let v = fload(builder, rec, scalar_offset(builder, record, other)?);
            builder.emit(abi::float_compare_d(&corner, &v));
            builder.emit(abi::branch_ne(decline));
        }
        // `0 < r <= 1` is a fraction of the half-size: an elliptical corner unless the
        // rectangle is square. Not this builder's.
        let exact = builder.label("canvas_geo_rect_radius_exact");
        branch_if_not_positive(builder, &corner, &exact);
        let one = fconst(builder, 1.0);
        builder.emit(abi::float_compare_d(&corner, &one));
        builder.emit(abi::branch_gt(&exact));
        builder.emit(abi::branch(decline));
        builder.emit(abi::label(&exact));
    }
    Ok(RectInputs { x, y, w, h, corner })
}

/// `__canvas_circleHeader`.
pub(super) fn fill_circle(
    builder: &mut CodeBuilder,
    out: &VirtualRegister,
    rec: &VirtualRegister,
    record: &ParameterType,
    view: &PaintView,
    done: &str,
) -> Result<(), String> {
    let empty = builder.label("canvas_geo_circle_empty");
    // plan-172-D: the centre is `position`; the transform is all-zero here, so the
    // circle is built at it.
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
pub(super) fn fill_line(
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
    // plan-172-F: the two ends are offsets from `position`.
    let start = builder.temporary_vreg();
    let start_type = emit_field_block(builder, &start, rec, record, "startAt")?;
    let (x1, y1) = placed_point(
        builder,
        view,
        &start,
        scalar_offset(builder, &start_type, "x")?,
        scalar_offset(builder, &start_type, "y")?,
    );
    let end = builder.temporary_vreg();
    let end_type = emit_field_block(builder, &end, rec, record, "endAt")?;
    let (x2, y2) = placed_point(
        builder,
        view,
        &end,
        scalar_offset(builder, &end_type, "x")?,
        scalar_offset(builder, &end_type, "y")?,
    );
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
/// (`List OF vector::Float2`).
pub(super) fn ring_point_offsets(
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
///
/// plan-172-F: a point is an offset from `position`, so each is read through
/// [`moved_point`] — `position + point`, then everything as before. The joins and
/// the dashes are computed from the edges written here, never from the item's points,
/// so they follow without an edit.
pub(super) fn fill_polygon(
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

    let offset = build_offset(builder, view);
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
    let (fx, fy) = moved_point(builder, &offset, &q, xo, yo);
    builder.emit(abi::float_move_d_from_d(&min_x, &fx));
    builder.emit(abi::float_move_d_from_d(&max_x, &fx));
    builder.emit(abi::float_move_d_from_d(&min_y, &fy));
    builder.emit(abi::float_move_d_from_d(&max_y, &fy));
    builder.emit(abi::move_immediate(&i, "Integer", "1"));
    builder.emit(abi::label(&head));
    builder.emit(abi::compare_registers(&i, &n));
    builder.emit(abi::branch_ge(&bounds_done));
    emit_list_element(builder, &q, &list, &i);
    let (qx, qy) = moved_point(builder, &offset, &q, xo, yo);
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
    let ax = builder.temporary_fp_vreg();
    let ay = builder.temporary_fp_vreg();
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
    // plan-172-F: an edge's first point is the edge before's second, so it is carried
    // in two registers instead of being read and moved again. Per edge that is one
    // point read where there were two, which pays for adding `position` to it.
    builder.emit(abi::move_immediate(&i, "Integer", "0"));
    emit_list_element(builder, &a, &list, &i);
    let (first_x, first_y) = moved_point(builder, &offset, &a, xo, yo);
    builder.emit(abi::float_move_d_from_d(&ax, &first_x));
    builder.emit(abi::float_move_d_from_d(&ay, &first_y));
    builder.emit(abi::label(&edge_head));
    builder.emit(abi::compare_registers(&i, &n));
    builder.emit(abi::branch_ge(&edge_ring_next));
    builder.emit(abi::add_immediate(&j, &i, 1));
    builder.emit(abi::compare_registers(&j, &n));
    builder.emit(abi::branch_lt(&edge_wrap));
    builder.emit(abi::move_immediate(&j, "Integer", "0"));
    builder.emit(abi::label(&edge_wrap));
    emit_list_element(builder, &b, &list, &j);
    let (bx, by) = moved_point(builder, &offset, &b, xo, yo);
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
    builder.emit(abi::float_move_d_from_d(&ax, &bx));
    builder.emit(abi::float_move_d_from_d(&ay, &by));
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
