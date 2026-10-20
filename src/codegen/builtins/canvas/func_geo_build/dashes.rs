//! plan-176-R: the native geometry builder's dashes — the dash pattern, head, phase, ring data
//! and tail. The slot constants, float helpers and `lower_geo_build` live in
//! `builtins/canvas/func_geo_build/mod.rs`.

use super::*;

// ---- Dashes (plan-163-B) --------------------------------------------------------------
//
// `helper_dashes.rs`'s build side, operation for operation: `__canvas_dashPattern`,
// `__canvas_dashTail`, `__canvas_dashPhase`, `__canvas_dashEdgeData` and
// `__canvas_withDashes`.

/// An edge's perimeter data: where its ring's length before it ends, where it ends, and
/// its ring's length (negative for an open path).
pub(super) const DASH_EDGE_FLOATS: usize = 3;

/// A dashed paint's pattern: `paint.dashes` read through `list`, its length `n`, and the
/// drawn entry count `entries` — `n`, or `2 * n` for an odd `n`, which repeats once.
pub(super) struct DashPattern {
    pub(super) list: ListView,
    pub(super) n: VirtualRegister,
    pub(super) entries: VirtualRegister,
}

/// `__canvas_dashPattern(paint, half)`: falls through for a dashed stroke, branches to
/// `none` for a solid one — no lengths, `NOT (half > 0.0)`, an entry `NOT (d >= 0.0)`, or
/// a sum (added left to right from 0.0) `NOT (> 0.0)`.
pub(super) fn emit_dash_pattern(
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
pub(super) fn emit_add_dash_floats(
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
pub(super) fn emit_ring_dash_data(
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
pub(super) fn emit_dash_tail(
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
pub(super) fn emit_record_end(builder: &mut CodeBuilder, out: &VirtualRegister) -> VirtualRegister {
    let length = fload(builder, out, 8);
    let end = builder.temporary_vreg();
    builder.emit(abi::float_convert_to_signed_x(&end, &length));
    builder.emit(abi::shift_left_immediate(&end, &end, 3));
    builder.emit(abi::add_registers(&end, out, &end));
    end
}
