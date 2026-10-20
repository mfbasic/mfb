//! plan-176-R: the native geometry builder's polylines — the scan, tail size and fill. The slot
//! constants, float helpers and `lower_geo_build` live in
//! `builtins/canvas/func_geo_build/mod.rs`.

use super::*;

/// What `__canvas_polylineHeader` decides before it writes a slot (plan-154-A).
pub(super) struct PolylineScan {
    /// `len(points)` and `len(colors)`.
    pub(super) count: VirtualRegister,
    pub(super) given: VirtualRegister,
    /// `__canvas_polylineHalf`: positive, or the scan branched to `empty`.
    pub(super) half: VirtualRegister,
    /// The first and last segments with length, and the cap tag (Round 1, Square 2,
    /// Butt 0).
    pub(super) first: VirtualRegister,
    pub(super) last: VirtualRegister,
    pub(super) cap: VirtualRegister,
    pub(super) points: ListView,
    pub(super) colors: ListView,
    pub(super) xo: usize,
    pub(super) yo: usize,
    /// The build offset (`position`), in registers for the segment walk.
    pub(super) offset: (VirtualRegister, VirtualRegister),
    /// `minX, minY, maxX, maxY` over the points, when asked for.
    pub(super) bounds: Option<[VirtualRegister; 4]>,
}

/// `dx * dx + dy * dy` exactly as `opt::fma_fusion` lowers the MFBASIC expression:
/// `dy * dy` rounded, then `fmadd(addend = dy*dy, dx, dx)`.
pub(super) fn fused_len_sq(
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
pub(super) fn emit_polyline_scan(
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

    let offset = build_offset(builder, view);
    let i = builder.temporary_vreg();
    let j = builder.temporary_vreg();
    let a = builder.temporary_vreg();
    let b = builder.temporary_vreg();
    let bounds = if with_bounds {
        builder.emit(abi::move_immediate(&i, "Integer", "0"));
        emit_list_element(builder, &a, &list, &i);
        let (fx, fy) = moved_point(builder, &offset, &a, xo, yo);
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
    // plan-172-F: an edge's first point is the edge before's second, so it is carried
    // in two registers instead of being read and moved again. Per edge that is one
    // point read where there were two, which pays for adding `position` to it.
    let ax = builder.temporary_fp_vreg();
    let ay = builder.temporary_fp_vreg();
    builder.emit(abi::move_immediate(&i, "Integer", "0"));
    emit_list_element(builder, &a, &list, &i);
    let (first_x, first_y) = moved_point(builder, &offset, &a, xo, yo);
    builder.emit(abi::float_move_d_from_d(&ax, &first_x));
    builder.emit(abi::float_move_d_from_d(&ay, &first_y));
    builder.emit(abi::label(&head));
    builder.emit(abi::add_immediate(&j, &i, 1));
    builder.emit(abi::compare_registers(&j, &count));
    builder.emit(abi::branch_ge(&walked));
    emit_list_element(builder, &b, &list, &j);
    let (bx, by) = moved_point(builder, &offset, &b, xo, yo);
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
    builder.emit(abi::float_move_d_from_d(&ax, &bx));
    builder.emit(abi::float_move_d_from_d(&ay, &by));
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
        offset,
        bounds,
    })
}

/// `n += (count - 1) * 5 + stops * 5` for a polyline the scan did not reject, where
/// `stops` is `count` when it has colours and 0 otherwise.
pub(super) fn emit_polyline_tail_size(
    builder: &mut CodeBuilder,
    scan: &PolylineScan,
    n: &VirtualRegister,
) {
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
pub(super) fn fill_polyline(
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
    // The carried first point, as in the scan.
    let ax = builder.temporary_fp_vreg();
    let ay = builder.temporary_fp_vreg();
    builder.emit(abi::move_immediate(&i, "Integer", "0"));
    emit_list_element(builder, &a, &scan.points, &i);
    let (first_x, first_y) = moved_point(builder, &scan.offset, &a, scan.xo, scan.yo);
    builder.emit(abi::float_move_d_from_d(&ax, &first_x));
    builder.emit(abi::float_move_d_from_d(&ay, &first_y));
    builder.emit(abi::label(&edge_head));
    builder.emit(abi::add_immediate(&j, &i, 1));
    builder.emit(abi::compare_registers(&j, &scan.count));
    builder.emit(abi::branch_ge(&edge_done));
    emit_list_element(builder, &b, &scan.points, &j);
    let (bx, by) = moved_point(builder, &scan.offset, &b, scan.xo, scan.yo);
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
    builder.emit(abi::float_move_d_from_d(&ax, &bx));
    builder.emit(abi::float_move_d_from_d(&ay, &by));
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
