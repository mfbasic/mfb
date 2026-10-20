//! plan-176-P: the terminal backend's grid drawing — clearing, cursor moves, glyph stamping,
//! lines, boxes and filled rectangles. The constants, data objects, `lower_term_helper` and
//! `StampCtx` live in `term/core/term/mod.rs`.

use super::*;

/// `term::clear` (plan-35-B): blank the back buffer (every cell zero-filled — a
/// blank glyph on the default/black background, not the caller's current
/// background) and home the shadow cursor. Emits no ANSI; the cleared state is
/// shown when `term::sync` presents. (bug-175 H: doc corrected to match the
/// zero-fill.)
pub(super) fn emit_clear_grid(
    symbol: &str,
    term_state_offset: usize,
    instructions: &mut Vec<CodeInstruction>,
    vregs: &mut Vregs,
) {
    let gp: Reg = vregs.next().into();
    let words: Reg = vregs.next().into();
    let cols: Reg = vregs.next().into();
    let cursor: Reg = vregs.next().into();
    let zero: Reg = vregs.next().into();
    let inactive = format!("{symbol}_inactive");
    let clr = format!("{symbol}_clr_loop");
    let clr_done = format!("{symbol}_clr_done");
    emit_gate_inactive(term_state_offset, &inactive, gp, instructions);
    instructions.extend([
        abi::load_u64(
            &gp,
            ARENA_STATE_REGISTER,
            term_state_offset + term_grid::TERM_STATE_GRID_OFFSET,
        ),
        abi::compare_immediate(&gp, "0"),
        abi::branch_eq(&inactive),
        // words = rows*cols*CELL_SIZE/8 = rows*cols*2 ; back = gp + HDR_SIZE
        abi::load_u64(&words, &gp, 0),
        abi::load_u64(&cols, &gp, 8),
        abi::multiply_registers(&words, &words, &cols),
        abi::shift_left_immediate(&words, &words, 1),
        abi::add_immediate(&cursor, &gp, 40),
        abi::move_immediate(&zero, "Integer", "0"),
        abi::label(&clr),
        abi::compare_immediate(&words, "0"),
        abi::branch_eq(&clr_done),
        abi::store_u64(&zero, &cursor, 0),
        abi::add_immediate(&cursor, &cursor, 8),
        abi::subtract_immediate(&words, &words, 1),
        abi::branch(&clr),
        abi::label(&clr_done),
        // Home the shadow cursor (cursorRow @ 16, cursorCol @ 24).
        abi::store_u64(&zero, &gp, 16),
        abi::store_u64(&zero, &gp, 24),
    ]);
    instructions.push(abi::label(&inactive));
    instructions.push(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
}

/// `term::moveTo(row, column)` (plan-35-B): set the shadow cursor in the grid
/// header, clamping negatives to 0 and high values to the last valid cell. Emits
/// no ANSI; the cursor is honoured by the next glyph write and by the present.
pub(super) fn emit_move_to(
    symbol: &str,
    term_state_offset: usize,
    instructions: &mut Vec<CodeInstruction>,
    vregs: &mut Vregs,
) {
    let gp: Reg = vregs.next().into();
    let rows: Reg = vregs.next().into();
    let cols: Reg = vregs.next().into();
    let row: Reg = vregs.next().into();
    let col: Reg = vregs.next().into();
    let inactive = format!("{symbol}_inactive");
    let row_lo = format!("{symbol}_row_lo");
    let col_lo = format!("{symbol}_col_lo");
    let row_hi = format!("{symbol}_row_hi");
    let col_hi = format!("{symbol}_col_hi");
    emit_gate_inactive(term_state_offset, &inactive, gp, instructions);
    instructions.extend([
        abi::load_u64(
            &gp,
            ARENA_STATE_REGISTER,
            term_state_offset + term_grid::TERM_STATE_GRID_OFFSET,
        ),
        abi::compare_immediate(&gp, "0"),
        abi::branch_eq(&inactive),
        abi::load_u64(&rows, &gp, 0),
        abi::load_u64(&cols, &gp, 8),
        // row = clamp(ARG[0], 0, rows-1)
        abi::move_register(&row, abi::c_arg(0)),
        abi::compare_immediate(&row, "0"),
        abi::branch_ge(&row_lo),
        abi::move_immediate(&row, "Integer", "0"),
        abi::label(&row_lo),
        abi::compare_registers(&row, &rows),
        abi::branch_lt(&row_hi),
        abi::subtract_immediate(&row, &rows, 1),
        abi::label(&row_hi),
        // col = clamp(ARG[1], 0, cols-1)
        abi::move_register(&col, abi::c_arg(1)),
        abi::compare_immediate(&col, "0"),
        abi::branch_ge(&col_lo),
        abi::move_immediate(&col, "Integer", "0"),
        abi::label(&col_lo),
        abi::compare_registers(&col, &cols),
        abi::branch_lt(&col_hi),
        abi::subtract_immediate(&col, &cols, 1),
        abi::label(&col_hi),
        abi::store_u64(&row, &gp, 16), // cursorRow
        abi::store_u64(&col, &gp, 24), // cursorCol
    ]);
    instructions.push(abi::label(&inactive));
    instructions.push(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
}

/// Pack a code point's UTF-8 bytes little-endian into the u32 grid `glyph`
/// encoding (`byte0 | byte1<<8 | byte2<<16`, the same layout `term_grid` writes
/// and `term::sync` reads back). Every box-drawing glyph here is a 3-byte run.
fn packed_glyph(codepoint: u32) -> u32 {
    let ch = char::from_u32(codepoint).expect("box-drawing code point is valid");
    let mut buf = [0u8; 4];
    let bytes = ch.encode_utf8(&mut buf).as_bytes();
    let mut packed = 0u32;
    for (index, byte) in bytes.iter().enumerate() {
        packed |= (*byte as u32) << (8 * index as u32);
    }
    packed
}

/// Emit `dst = packed_glyph(table[ord])`, a select-by-ordinal chain over a 7-entry
/// `LineStyle` code-point table. Ordinal 0 is the fall-through default, so an
/// out-of-range ordinal can never strand `dst`. `tag` uniquifies the labels.
fn emit_select_glyph(
    ord: Reg,
    dst: Reg,
    table: &[u32],
    tag: &str,
    instructions: &mut Vec<CodeInstruction>,
) {
    let done = format!("{tag}_done");
    instructions.push(abi::move_immediate(
        dst,
        "Integer",
        &packed_glyph(table[0]).to_string(),
    ));
    for (ordinal, codepoint) in table.iter().enumerate().skip(1) {
        let next = format!("{tag}_{ordinal}");
        instructions.extend([
            abi::compare_immediate(ord, &ordinal.to_string()),
            abi::branch_ne(&next),
            abi::move_immediate(dst, "Integer", &packed_glyph(*codepoint).to_string()),
            abi::branch(&done),
            abi::label(&next),
        ]);
    }
    instructions.push(abi::label(&done));
}

/// Stamp `glyph` (with the ctx attributes) across a clamped run. `fixed` is the
/// line coordinate and `ea`/`eb` the two span endpoints (either order). When
/// `is_horizontal`, `fixed` is the row and the run spans columns; otherwise
/// `fixed` is the column and the run spans rows. The span is normalised (lo<=hi)
/// and clamped to the grid; a `fixed` off the grid, or a span with no on-grid
/// cell, branches to `skip` (which the caller places after the call). Does not
/// clobber `fixed`/`ea`/`eb` or the ctx attributes — only ctx scratch — so a
/// caller can reuse the extents for further runs and corners.
#[allow(clippy::too_many_arguments)]
fn emit_stamp_run(
    ctx: &StampCtx,
    is_horizontal: bool,
    fixed: Reg,
    ea: Reg,
    eb: Reg,
    glyph: Reg,
    tag: &str,
    skip: &str,
    instructions: &mut Vec<CodeInstruction>,
) {
    let fixed_limit = if is_horizontal { ctx.rows } else { ctx.cols };
    let span_limit = if is_horizontal { ctx.cols } else { ctx.rows };
    let span_ok = format!("{tag}_span_ok");
    let lo_ok = format!("{tag}_lo_ok");
    let hi_ok = format!("{tag}_hi_ok");
    let loop_top = format!("{tag}_loop");
    let loop_done = format!("{tag}_loop_done");
    instructions.extend([
        // The fixed coordinate must be on the grid: [0, fixed_limit-1].
        abi::compare_immediate(fixed, "0"),
        abi::branch_lt(skip),
        abi::compare_registers(fixed, fixed_limit),
        abi::branch_ge(skip),
        // lo = min(ea, eb), hi = max(ea, eb).
        abi::move_register(ctx.lo, ea),
        abi::move_register(ctx.hi, eb),
        abi::compare_registers(ctx.lo, ctx.hi),
        abi::branch_le(&span_ok),
        abi::move_register(ctx.tmp, ctx.lo),
        abi::move_register(ctx.lo, ctx.hi),
        abi::move_register(ctx.hi, ctx.tmp),
        abi::label(&span_ok),
        // Clamp lo up to 0 and hi down to span_limit-1; empty span → skip.
        abi::compare_immediate(ctx.lo, "0"),
        abi::branch_ge(&lo_ok),
        abi::move_immediate(ctx.lo, "Integer", "0"),
        abi::label(&lo_ok),
        abi::subtract_immediate(ctx.tmp, span_limit, 1),
        abi::compare_registers(ctx.hi, ctx.tmp),
        abi::branch_le(&hi_ok),
        abi::move_register(ctx.hi, ctx.tmp),
        abi::label(&hi_ok),
        abi::compare_registers(ctx.lo, ctx.hi),
        abi::branch_gt(skip),
        abi::move_register(ctx.pos, ctx.lo),
        abi::label(&loop_top),
        abi::compare_registers(ctx.pos, ctx.hi),
        abi::branch_gt(&loop_done),
    ]);
    // plan-70-C: line/box/fill glyphs are all single-width; stamp width 1 into the
    // cell so the presenter advances one column even over a stale wide-cell width
    // byte. `tmp` is reused as the constant-1 source across the run.
    instructions.push(abi::move_immediate(ctx.tmp, "Integer", "1"));
    // Cell index: H → fixed*cols + pos ; V → pos*cols + fixed (`cols` is stride).
    if is_horizontal {
        instructions.extend([
            abi::multiply_registers(ctx.idx, fixed, ctx.cols),
            abi::add_registers(ctx.idx, ctx.idx, ctx.pos),
        ]);
    } else {
        instructions.extend([
            abi::multiply_registers(ctx.idx, ctx.pos, ctx.cols),
            abi::add_registers(ctx.idx, ctx.idx, fixed),
        ]);
    }
    instructions.extend([
        abi::shift_left_immediate(ctx.idx, ctx.idx, 4), // * CELL_SIZE (16)
        abi::add_registers(ctx.cell, ctx.back, ctx.idx),
    ]);
    // plan-70-C Phase 2: clear the paired half of any wide glyph this run cell
    // overwrites. The cell's column is `pos` for a horizontal run, `fixed` for a
    // vertical one.
    let run_col = if is_horizontal { ctx.pos } else { fixed };
    emit_clear_wide_pair(ctx, ctx.cell, run_col, &format!("{tag}_run"), instructions);
    instructions.extend([
        abi::store_u32(glyph, ctx.cell, term_grid::C_GLYPH),
        abi::store_u32(ctx.fg, ctx.cell, term_grid::C_FG),
        abi::store_u32(ctx.bg, ctx.cell, term_grid::C_BG),
        abi::store_u8(ctx.bold, ctx.cell, term_grid::C_BOLD),
        abi::store_u8(ctx.un, ctx.cell, term_grid::C_UN),
        abi::store_u8(ctx.tmp, ctx.cell, term_grid::C_WIDTH),
        abi::add_immediate(ctx.pos, ctx.pos, 1),
        abi::branch(&loop_top),
        abi::label(&loop_done),
    ]);
}

/// plan-70-C: before overwriting the on-grid cell at address `cell` / column
/// `col`, clear the OTHER half of any wide glyph this cell is part of, so no
/// orphaned half-glyph remains. If the cell is a `WIDE_TRAIL`, blank the primary
/// to its left (col-1); if it is a wide primary (stored width 2), blank the trail
/// to its right (col+1). Blanking = glyph 0 (a space) + width 1, attributes kept.
/// Uses `ctx.idx` and `ctx.lo` as scratch — both dead at every call site (the
/// cell address is already in `ctx.cell`; `lo` is a spent span bound).
fn emit_clear_wide_pair(
    ctx: &StampCtx,
    cell: Reg,
    col: Reg,
    tag: &str,
    instructions: &mut Vec<CodeInstruction>,
) {
    let after = format!("{tag}_cwp_after");
    let not_trail = format!("{tag}_cwp_nt");
    instructions.extend([
        abi::load_u32(ctx.lo, cell, term_grid::C_GLYPH),
        abi::move_immediate(ctx.idx, "Integer", term_grid::WIDE_TRAIL),
        abi::compare_registers(ctx.lo, ctx.idx),
        abi::branch_ne(&not_trail),
        // Trailing sentinel: clear the primary to the left (if any).
        abi::compare_immediate(col, "0"),
        abi::branch_le(&after),
        abi::subtract_immediate(ctx.idx, cell, term_grid::CELL_SIZE),
        abi::move_immediate(ctx.lo, "Integer", "0"),
        abi::store_u32(ctx.lo, ctx.idx, term_grid::C_GLYPH),
        abi::move_immediate(ctx.lo, "Integer", "1"),
        abi::store_u8(ctx.lo, ctx.idx, term_grid::C_WIDTH),
        abi::branch(&after),
        abi::label(&not_trail),
        // Wide primary (width 2): clear the trailing sentinel to the right.
        abi::load_u8(ctx.lo, cell, term_grid::C_WIDTH),
        abi::compare_immediate(ctx.lo, "2"),
        abi::branch_ne(&after),
        abi::add_immediate(ctx.idx, col, 1),
        abi::compare_registers(ctx.idx, ctx.cols),
        abi::branch_ge(&after),
        abi::add_immediate(ctx.idx, cell, term_grid::CELL_SIZE),
        abi::move_immediate(ctx.lo, "Integer", "0"),
        abi::store_u32(ctx.lo, ctx.idx, term_grid::C_GLYPH),
        abi::move_immediate(ctx.lo, "Integer", "1"),
        abi::store_u8(ctx.lo, ctx.idx, term_grid::C_WIDTH),
        abi::label(&after),
    ]);
}

/// Stamp a single cell `(row, col)` with `glyph` + the ctx attributes when it is
/// on the grid (`0<=row<rows`, `0<=col<cols`); otherwise branch to `skip` (placed
/// by the caller after the call). Used for `drawBox` corners.
pub(super) fn emit_stamp_cell(
    ctx: &StampCtx,
    row: Reg,
    col: Reg,
    glyph: Reg,
    width: Reg,
    tag: &str,
    skip: &str,
    instructions: &mut Vec<CodeInstruction>,
) {
    instructions.extend([
        abi::compare_immediate(row, "0"),
        abi::branch_lt(skip),
        abi::compare_registers(row, ctx.rows),
        abi::branch_ge(skip),
        abi::compare_immediate(col, "0"),
        abi::branch_lt(skip),
        abi::compare_registers(col, ctx.cols),
        abi::branch_ge(skip),
        abi::multiply_registers(ctx.idx, row, ctx.cols),
        abi::add_registers(ctx.idx, ctx.idx, col),
        abi::shift_left_immediate(ctx.idx, ctx.idx, 4),
        abi::add_registers(ctx.cell, ctx.back, ctx.idx),
    ]);
    // plan-70-C Phase 2: clear the paired half of any wide glyph this cell was
    // part of, before overwriting it, so no orphaned half-glyph remains.
    emit_clear_wide_pair(ctx, ctx.cell, col, tag, instructions);
    instructions.extend([
        abi::store_u32(glyph, ctx.cell, term_grid::C_GLYPH),
        abi::store_u32(ctx.fg, ctx.cell, term_grid::C_FG),
        abi::store_u32(ctx.bg, ctx.cell, term_grid::C_BG),
        abi::store_u8(ctx.bold, ctx.cell, term_grid::C_BOLD),
        abi::store_u8(ctx.un, ctx.cell, term_grid::C_UN),
        // plan-70-C: record the cluster width so the presenter advances correctly
        // (and never inherits a stale width byte from a prior wide occupant).
        abi::store_u8(width, ctx.cell, term_grid::C_WIDTH),
    ]);
}

/// Load the grid pointer, dims, back base, and current attributes into the given
/// registers; branch to `inactive` when the grid is unallocated. Shared prologue
/// for the drawing helpers.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_load_grid(
    term_state_offset: usize,
    gp: Reg,
    rows: Reg,
    cols: Reg,
    back: Reg,
    fg: Reg,
    bg: Reg,
    bold: Reg,
    un: Reg,
    inactive: &str,
    instructions: &mut Vec<CodeInstruction>,
) {
    instructions.extend([
        abi::load_u64(
            gp,
            ARENA_STATE_REGISTER,
            term_state_offset + term_grid::TERM_STATE_GRID_OFFSET,
        ),
        abi::compare_immediate(gp, "0"),
        abi::branch_eq(inactive),
        abi::load_u64(rows, gp, 0),
        abi::load_u64(cols, gp, 8),
        abi::add_immediate(back, gp, term_grid::HDR_SIZE),
        abi::load_u64(
            fg,
            ARENA_STATE_REGISTER,
            term_state_offset + TERM_STATE_FG_OFFSET,
        ),
        abi::load_u64(
            bg,
            ARENA_STATE_REGISTER,
            term_state_offset + TERM_STATE_BG_OFFSET,
        ),
        abi::load_u64(
            bold,
            ARENA_STATE_REGISTER,
            term_state_offset + TERM_STATE_BOLD_OFFSET,
        ),
        abi::load_u64(
            un,
            ARENA_STATE_REGISTER,
            term_state_offset + TERM_STATE_UNDERLINE_OFFSET,
        ),
    ]);
}

/// `term::drawHLine`/`drawVLine` (console): stamp a fixed box-drawing glyph across
/// a run of back-buffer cells with the current colours/attributes; the run is
/// shown on the next `term::sync`. A no-op while TUI mode is off or the grid is
/// unallocated — the same gate every writer honours (§4.2.1).
///
/// Arguments arrive in registers as `ARG[0]` = the `LineStyle` ordinal, then the
/// coordinates in row-before-column order:
///   drawHLine(line, row, columnA, columnB) — fixed `row`, span over columns.
///   drawVLine(line, rowA, column, rowB) — fixed `column`, span over rows.
/// Both name a start point `(row, column)` and then the far end of the run, so the
/// fixed coordinate is `ARG[1]` for the horizontal form and `ARG[2]` for the
/// vertical one.
/// `is_horizontal` selects, at emit time, the glyph table; the span endpoints may
/// be given in either order and are clamped to the grid; a fixed coordinate off
/// the grid, or a span with no on-grid cell, draws nothing.
pub(super) fn emit_draw_line(
    symbol: &str,
    term_state_offset: usize,
    is_horizontal: bool,
    instructions: &mut Vec<CodeInstruction>,
    vregs: &mut Vregs,
) {
    let inactive = format!("{symbol}_inactive");
    let ord: Reg = vregs.next().into();
    let fixed: Reg = vregs.next().into();
    let ea: Reg = vregs.next().into();
    let eb: Reg = vregs.next().into();
    let gp: Reg = vregs.next().into();
    let rows: Reg = vregs.next().into();
    let cols: Reg = vregs.next().into();
    let back: Reg = vregs.next().into();
    let glyph: Reg = vregs.next().into();
    let fg: Reg = vregs.next().into();
    let bg: Reg = vregs.next().into();
    let bold: Reg = vregs.next().into();
    let un: Reg = vregs.next().into();
    let ctx = StampCtx {
        rows,
        cols,
        back,
        fg,
        bg,
        bold,
        un,
        lo: vregs.next().into(),
        hi: vregs.next().into(),
        idx: vregs.next().into(),
        cell: vregs.next().into(),
        pos: vregs.next().into(),
        tmp: vregs.next().into(),
    };

    emit_gate_inactive(term_state_offset, &inactive, ord, instructions);
    // Row-before-column argument order (both members name a start point `(row,
    // column)` and then the far end of the run), so the fixed coordinate is the
    // FIRST argument for `drawHLine(line, row, columnA, columnB)` and the SECOND
    // for `drawVLine(line, rowA, column, rowB)`.
    let (fixed_arg, ea_arg, eb_arg) = if is_horizontal { (1, 2, 3) } else { (2, 1, 3) };
    instructions.extend([
        abi::move_register(ord, abi::c_arg(0)),
        abi::move_register(fixed, abi::c_arg(fixed_arg)),
        abi::move_register(ea, abi::c_arg(ea_arg)),
        abi::move_register(eb, abi::c_arg(eb_arg)),
    ]);
    emit_load_grid(
        term_state_offset,
        gp,
        rows,
        cols,
        back,
        fg,
        bg,
        bold,
        un,
        &inactive,
        instructions,
    );
    let table = if is_horizontal {
        &TERM_HLINE_CODEPOINTS
    } else {
        &TERM_VLINE_CODEPOINTS
    };
    emit_select_glyph(
        ord,
        glyph,
        table,
        &format!("{symbol}_dl_glyph"),
        instructions,
    );
    emit_stamp_run(
        &ctx,
        is_horizontal,
        fixed,
        ea,
        eb,
        glyph,
        &format!("{symbol}_dl"),
        &inactive,
        instructions,
    );
    instructions.push(abi::label(&inactive));
    instructions.push(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
}

/// `term::drawBox(line, rowA, columnA, rowB, columnB)` (console): draw a rectangle
/// in the given `LineStyle`. The two points are opposite corners, each written row
/// before column, and either corner may be given first. Draws the four edges — top/bottom horizontal runs and left/right
/// vertical runs, each using this style's own line glyph (so dashed/dotted styles
/// get dashed/dotted edges) — then overwrites the four corner cells with the
/// matching corner glyph (`*Dash`/`*Dot` reuse the Light or Heavy corners). Each
/// edge and corner is clamped independently, so a box partly off the grid draws
/// the visible part; a fully off-grid box draws nothing. Shown on the next
/// `term::sync`; a no-op while TUI mode is off.
pub(super) fn emit_draw_box(
    symbol: &str,
    term_state_offset: usize,
    instructions: &mut Vec<CodeInstruction>,
    vregs: &mut Vregs,
) {
    let inactive = format!("{symbol}_inactive");
    let ord: Reg = vregs.next().into();
    let ax1: Reg = vregs.next().into();
    let ay1: Reg = vregs.next().into();
    let ax2: Reg = vregs.next().into();
    let ay2: Reg = vregs.next().into();
    let gp: Reg = vregs.next().into();
    let rows: Reg = vregs.next().into();
    let cols: Reg = vregs.next().into();
    let back: Reg = vregs.next().into();
    let fg: Reg = vregs.next().into();
    let bg: Reg = vregs.next().into();
    let bold: Reg = vregs.next().into();
    let un: Reg = vregs.next().into();
    let xlo: Reg = vregs.next().into();
    let xhi: Reg = vregs.next().into();
    let ylo: Reg = vregs.next().into();
    let yhi: Reg = vregs.next().into();
    let hglyph: Reg = vregs.next().into();
    let vglyph: Reg = vregs.next().into();
    let ctl: Reg = vregs.next().into();
    let ctr: Reg = vregs.next().into();
    let cbl: Reg = vregs.next().into();
    let cbr: Reg = vregs.next().into();
    let ctx = StampCtx {
        rows,
        cols,
        back,
        fg,
        bg,
        bold,
        un,
        lo: vregs.next().into(),
        hi: vregs.next().into(),
        idx: vregs.next().into(),
        cell: vregs.next().into(),
        pos: vregs.next().into(),
        tmp: vregs.next().into(),
    };

    emit_gate_inactive(term_state_offset, &inactive, ord, instructions);
    // Corners arrive as `(rowA, columnA, rowB, columnB)` — every `term::` point is
    // written row before column — so the column registers are args 2 and 4 and the
    // row registers args 1 and 3.
    instructions.extend([
        abi::move_register(ord, abi::c_arg(0)),
        abi::move_register(ay1, abi::c_arg(1)),
        abi::move_register(ax1, abi::c_arg(2)),
        abi::move_register(ay2, abi::c_arg(3)),
        abi::move_register(ax2, abi::c_arg(4)),
    ]);
    emit_load_grid(
        term_state_offset,
        gp,
        rows,
        cols,
        back,
        fg,
        bg,
        bold,
        un,
        &inactive,
        instructions,
    );
    // Normalise the two corners: xlo/xhi over columns, ylo/yhi over rows. Edges
    // and corners are placed from these, so left is always the smaller column etc.
    let x_ok = format!("{symbol}_box_x_ok");
    let y_ok = format!("{symbol}_box_y_ok");
    instructions.extend([
        abi::move_register(xlo, ax1),
        abi::move_register(xhi, ax2),
        abi::compare_registers(ax1, ax2),
        abi::branch_le(&x_ok),
        abi::move_register(xlo, ax2),
        abi::move_register(xhi, ax1),
        abi::label(&x_ok),
        abi::move_register(ylo, ay1),
        abi::move_register(yhi, ay2),
        abi::compare_registers(ay1, ay2),
        abi::branch_le(&y_ok),
        abi::move_register(ylo, ay2),
        abi::move_register(yhi, ay1),
        abi::label(&y_ok),
    ]);
    // Resolve the edge glyphs (this style's H/V forms) and the four corner glyphs.
    emit_select_glyph(
        ord,
        hglyph,
        &TERM_HLINE_CODEPOINTS,
        &format!("{symbol}_box_h"),
        instructions,
    );
    emit_select_glyph(
        ord,
        vglyph,
        &TERM_VLINE_CODEPOINTS,
        &format!("{symbol}_box_v"),
        instructions,
    );
    emit_select_glyph(
        ord,
        ctl,
        &TERM_CORNER_TL_CODEPOINTS,
        &format!("{symbol}_box_tl"),
        instructions,
    );
    emit_select_glyph(
        ord,
        ctr,
        &TERM_CORNER_TR_CODEPOINTS,
        &format!("{symbol}_box_tr"),
        instructions,
    );
    emit_select_glyph(
        ord,
        cbl,
        &TERM_CORNER_BL_CODEPOINTS,
        &format!("{symbol}_box_bl"),
        instructions,
    );
    emit_select_glyph(
        ord,
        cbr,
        &TERM_CORNER_BR_CODEPOINTS,
        &format!("{symbol}_box_br"),
        instructions,
    );
    // Four edges (each clamped/skipped independently), then four corners on top.
    // top: row ylo, cols xlo..xhi ; bottom: row yhi.
    let edges: &[(bool, Reg, Reg, Reg, Reg, &str)] = &[
        (true, ylo, xlo, xhi, hglyph, "e0"),
        (true, yhi, xlo, xhi, hglyph, "e1"),
        (false, xlo, ylo, yhi, vglyph, "e2"),
        (false, xhi, ylo, yhi, vglyph, "e3"),
    ];
    for (is_h, fixed, ea, eb, glyph, tag) in edges {
        let skip = format!("{symbol}_box_{tag}_skip");
        emit_stamp_run(
            &ctx,
            *is_h,
            *fixed,
            *ea,
            *eb,
            *glyph,
            &format!("{symbol}_box_{tag}"),
            &skip,
            instructions,
        );
        instructions.push(abi::label(&skip));
    }
    let corners: &[(Reg, Reg, Reg, &str)] = &[
        (ylo, xlo, ctl, "cTL"),
        (ylo, xhi, ctr, "cTR"),
        (yhi, xlo, cbl, "cBL"),
        (yhi, xhi, cbr, "cBR"),
    ];
    // Box corners are single-width box-drawing glyphs (plan-70-C: pass width 1).
    instructions.push(abi::move_immediate(ctx.tmp, "Integer", "1"));
    for (row, col, glyph, tag) in corners {
        let skip = format!("{symbol}_box_{tag}_skip");
        emit_stamp_cell(
            &ctx,
            *row,
            *col,
            *glyph,
            ctx.tmp,
            &skip,
            &skip,
            instructions,
        );
        instructions.push(abi::label(&skip));
    }
    instructions.push(abi::label(&inactive));
    instructions.push(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
}

/// `term::fillRect(fill, rowA, columnA, rowB, columnB)` (console): fill the rectangle
/// between two opposite corners (each written row before column, either order first)
/// with the `FillStyle` block
/// or shade glyph, using the current colours/attributes. Implemented as one clamped
/// horizontal run per row (reusing the line stamper), so the region clamps to the
/// grid the same way; a fully off-grid rectangle fills nothing. Shown on the next
/// `term::sync`; a no-op while TUI mode is off.
pub(super) fn emit_fill_rect(
    symbol: &str,
    term_state_offset: usize,
    instructions: &mut Vec<CodeInstruction>,
    vregs: &mut Vregs,
) {
    let inactive = format!("{symbol}_inactive");
    let ord: Reg = vregs.next().into();
    let ax1: Reg = vregs.next().into();
    let ay1: Reg = vregs.next().into();
    let ax2: Reg = vregs.next().into();
    let ay2: Reg = vregs.next().into();
    let gp: Reg = vregs.next().into();
    let rows: Reg = vregs.next().into();
    let cols: Reg = vregs.next().into();
    let back: Reg = vregs.next().into();
    let fg: Reg = vregs.next().into();
    let bg: Reg = vregs.next().into();
    let bold: Reg = vregs.next().into();
    let un: Reg = vregs.next().into();
    let xlo: Reg = vregs.next().into();
    let xhi: Reg = vregs.next().into();
    let ylo: Reg = vregs.next().into();
    let yhi: Reg = vregs.next().into();
    let glyph: Reg = vregs.next().into();
    let row: Reg = vregs.next().into();
    let ctx = StampCtx {
        rows,
        cols,
        back,
        fg,
        bg,
        bold,
        un,
        lo: vregs.next().into(),
        hi: vregs.next().into(),
        idx: vregs.next().into(),
        cell: vregs.next().into(),
        pos: vregs.next().into(),
        tmp: vregs.next().into(),
    };

    emit_gate_inactive(term_state_offset, &inactive, ord, instructions);
    // Corners arrive as `(rowA, columnA, rowB, columnB)` — every `term::` point is
    // written row before column — so the column registers are args 2 and 4 and the
    // row registers args 1 and 3.
    instructions.extend([
        abi::move_register(ord, abi::c_arg(0)),
        abi::move_register(ay1, abi::c_arg(1)),
        abi::move_register(ax1, abi::c_arg(2)),
        abi::move_register(ay2, abi::c_arg(3)),
        abi::move_register(ax2, abi::c_arg(4)),
    ]);
    emit_load_grid(
        term_state_offset,
        gp,
        rows,
        cols,
        back,
        fg,
        bg,
        bold,
        un,
        &inactive,
        instructions,
    );
    // Normalise the corners (xlo/xhi over columns, ylo/yhi over rows), then clamp
    // the row range to the grid (the per-row run clamps the columns).
    let x_ok = format!("{symbol}_fr_x_ok");
    let y_ok = format!("{symbol}_fr_y_ok");
    let ylo_ok = format!("{symbol}_fr_ylo_ok");
    let yhi_ok = format!("{symbol}_fr_yhi_ok");
    instructions.extend([
        abi::move_register(xlo, ax1),
        abi::move_register(xhi, ax2),
        abi::compare_registers(ax1, ax2),
        abi::branch_le(&x_ok),
        abi::move_register(xlo, ax2),
        abi::move_register(xhi, ax1),
        abi::label(&x_ok),
        abi::move_register(ylo, ay1),
        abi::move_register(yhi, ay2),
        abi::compare_registers(ay1, ay2),
        abi::branch_le(&y_ok),
        abi::move_register(ylo, ay2),
        abi::move_register(yhi, ay1),
        abi::label(&y_ok),
        abi::compare_immediate(ylo, "0"),
        abi::branch_ge(&ylo_ok),
        abi::move_immediate(ylo, "Integer", "0"),
        abi::label(&ylo_ok),
        abi::subtract_immediate(ctx.tmp, rows, 1),
        abi::compare_registers(yhi, ctx.tmp),
        abi::branch_le(&yhi_ok),
        abi::move_register(yhi, ctx.tmp),
        abi::label(&yhi_ok),
        abi::compare_registers(ylo, yhi),
        abi::branch_gt(&inactive),
    ]);
    emit_select_glyph(
        ord,
        glyph,
        &TERM_FILL_CODEPOINTS,
        &format!("{symbol}_fr_glyph"),
        instructions,
    );
    // One horizontal run per row over ylo..=yhi.
    let loop_row = format!("{symbol}_fr_row");
    let row_next = format!("{symbol}_fr_next");
    instructions.extend([
        abi::move_register(row, ylo),
        abi::label(&loop_row),
        abi::compare_registers(row, yhi),
        abi::branch_gt(&inactive),
    ]);
    emit_stamp_run(
        &ctx,
        true,
        row,
        xlo,
        xhi,
        glyph,
        &format!("{symbol}_fr"),
        &row_next,
        instructions,
    );
    instructions.extend([
        abi::label(&row_next),
        abi::add_immediate(row, row, 1),
        abi::branch(&loop_row),
    ]);
    instructions.push(abi::label(&inactive));
    instructions.push(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
}
