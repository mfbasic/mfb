//! plan-176-R: the native geometry builder's stroke joins — join styles, join records and
//! rectangle sides. The slot constants, float helpers and `lower_geo_build` live in
//! `builtins/canvas/func_geo_build/mod.rs`.

use super::*;

// ---- Joins (plan-163-B) ---------------------------------------------------------------
//
// `helper_joins.rs`'s `__canvas_joinTail` and `__canvas_joinRecord`, instruction for
// instruction: one 8-float record per edge, the corner at its start. The MFBASIC
// arithmetic passes through `opt::fma_fusion`, which fuses the FIRST single-use product
// of an add or subtract (`a*b + c*d` becomes `fmadd(round(c*d), a, b)`, `a*b - c*d`
// becomes `fmsub(round(c*d), a, b)`, and `v + n*h` becomes `fmadd(v, n, h)`), so the
// fused forms are written out here.

/// A join record's length in floats: the four points `V, A, M, B`.
pub(super) const JOIN_FLOATS: usize = 8;

/// A paint's join: `__canvas_joinTag` (Round 0, Miter 1, Bevel 2) and the miter limit
/// applied, `__canvas_maxF(paint.miterLimit, 1.0)`; and what decides its dash tail —
/// whether the stroke of half-width `half` is dashed (1) or solid (0), and its cap tag.
pub(super) struct JoinStyle {
    pub(super) tag: VirtualRegister,
    pub(super) limit: VirtualRegister,
    pub(super) dashed: VirtualRegister,
    pub(super) cap: VirtualRegister,
}

pub(super) fn emit_join_style(
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

pub(super) fn fmul(
    b: &mut CodeBuilder,
    l: &VirtualRegister,
    r: &VirtualRegister,
) -> VirtualRegister {
    fbin(b, |d, l, r| abi::float_multiply_d(d, l, r), l, r)
}

/// `v + n * h`, as the fusion pass lowers it: `fmadd(v, n, h)`.
pub(super) fn fmadd(
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
pub(super) fn emit_ring_joins(
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
pub(super) fn branch_unless_joined(
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
pub(super) fn emit_join_reach(
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
pub(super) fn emit_put_joins(
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
pub(super) fn emit_add_join_floats(
    builder: &mut CodeBuilder,
    records: &VirtualRegister,
    n: &VirtualRegister,
) {
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
pub(super) fn emit_rect_joined(
    builder: &mut CodeBuilder,
    rec: &VirtualRegister,
    record: &ParameterType,
    view: &PaintView,
    none: &str,
) -> Result<(), String> {
    let RectInputs { w, h, corner, .. } = emit_rect_inputs(builder, rec, record, None)?;
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
pub(super) fn emit_rect_dash_size(
    builder: &mut CodeBuilder,
    rec: &VirtualRegister,
    record: &ParameterType,
    view: &PaintView,
    solid: &str,
    n: &VirtualRegister,
) -> Result<(), String> {
    let RectInputs { w, h, corner, .. } = emit_rect_inputs(builder, rec, record, None)?;
    branch_if_not_positive(builder, &w, solid);
    branch_if_not_positive(builder, &h, solid);
    let half = emit_stroke_half(builder, view)?;
    let pattern = emit_dash_pattern(builder, view, &half, solid)?;
    let data = builder.temporary_vreg();
    builder.emit(abi::move_immediate(&data, "Integer", "0"));
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
pub(super) fn emit_rect_sides(
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
