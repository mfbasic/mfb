//! plan-176-P: the terminal backend's text drawing — UTF-8 encoding, a glyph and a run of text.
//! The constants, data objects, `lower_term_helper` and `StampCtx` live in
//! `term/core/term/mod.rs`.

use super::*;

/// Runtime UTF-8 encode: `glyph = utf8_pack(cp)`, the grid glyph encoding (bytes
/// little-endian in a u32). `cp` is a runtime scalar in a register; `b`/`sh`/`mask`
/// are scratch. Branches on the four length ranges. Assumes `cp` is a valid scalar
/// (the caller guards control code points); an out-of-range value still produces a
/// well-formed 4-byte pack rather than corrupting anything.
pub(crate) fn emit_encode_utf8(
    cp: Reg,
    glyph: Reg,
    b: Reg,
    sh: Reg,
    mask: Reg,
    tag: &str,
    instrs: &mut Vec<CodeInstruction>,
) {
    let e1 = format!("{tag}_1");
    let e2 = format!("{tag}_2");
    let e3 = format!("{tag}_3");
    let done = format!("{tag}_done");
    // Emit `dst = 0x80 | ((cp >> shift) & 0x3F)` shifted into byte position `at`,
    // OR'd into `glyph`. `first` sets `glyph` directly (lead byte).
    instrs.push(abi::move_immediate(mask, "Integer", "63"));
    instrs.extend([
        abi::compare_immediate(cp, "128"),
        abi::branch_lt(&e1),
        abi::compare_immediate(cp, "2048"),
        abi::branch_lt(&e2),
        abi::compare_immediate(cp, "65536"),
        abi::branch_lt(&e3),
        // 4-byte: F0|cp>>18, then continuation bytes at 6/12/18-bit groups.
        abi::shift_right_immediate(sh, cp, 18),
        abi::move_immediate(b, "Integer", "240"),
        abi::or_registers(glyph, b, sh),
        abi::shift_right_immediate(sh, cp, 12),
        abi::and_registers(sh, sh, mask),
        abi::move_immediate(b, "Integer", "128"),
        abi::or_registers(sh, sh, b),
        abi::shift_left_immediate(sh, sh, 8),
        abi::or_registers(glyph, glyph, sh),
        abi::shift_right_immediate(sh, cp, 6),
        abi::and_registers(sh, sh, mask),
        abi::move_immediate(b, "Integer", "128"),
        abi::or_registers(sh, sh, b),
        abi::shift_left_immediate(sh, sh, 16),
        abi::or_registers(glyph, glyph, sh),
        abi::and_registers(sh, cp, mask),
        abi::move_immediate(b, "Integer", "128"),
        abi::or_registers(sh, sh, b),
        abi::shift_left_immediate(sh, sh, 24),
        abi::or_registers(glyph, glyph, sh),
        abi::branch(&done),
        // 3-byte: E0|cp>>12, then 6/12-bit continuation.
        abi::label(&e3),
        abi::shift_right_immediate(sh, cp, 12),
        abi::move_immediate(b, "Integer", "224"),
        abi::or_registers(glyph, b, sh),
        abi::shift_right_immediate(sh, cp, 6),
        abi::and_registers(sh, sh, mask),
        abi::move_immediate(b, "Integer", "128"),
        abi::or_registers(sh, sh, b),
        abi::shift_left_immediate(sh, sh, 8),
        abi::or_registers(glyph, glyph, sh),
        abi::and_registers(sh, cp, mask),
        abi::move_immediate(b, "Integer", "128"),
        abi::or_registers(sh, sh, b),
        abi::shift_left_immediate(sh, sh, 16),
        abi::or_registers(glyph, glyph, sh),
        abi::branch(&done),
        // 2-byte: C0|cp>>6, then 6-bit continuation.
        abi::label(&e2),
        abi::shift_right_immediate(sh, cp, 6),
        abi::move_immediate(b, "Integer", "192"),
        abi::or_registers(glyph, b, sh),
        abi::and_registers(sh, cp, mask),
        abi::move_immediate(b, "Integer", "128"),
        abi::or_registers(sh, sh, b),
        abi::shift_left_immediate(sh, sh, 8),
        abi::or_registers(glyph, glyph, sh),
        abi::branch(&done),
        // 1-byte: ASCII, glyph = cp.
        abi::label(&e1),
        abi::move_register(glyph, cp),
        abi::label(&done),
    ]);
}

/// `term::drawGlyph(row, column, codepoint)` (console): stamp a single Unicode scalar
/// at `row`/`column` with the current attributes; a no-op if the cell is off the
/// grid or `codepoint` is a control character (< 0x20, which would corrupt the
/// presented frame). Shown on the next `term::sync`.
pub(super) fn emit_draw_glyph(
    symbol: &str,
    term_state_offset: usize,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    vregs: &mut Vregs,
) {
    let inactive = format!("{symbol}_inactive");
    let x: Reg = vregs.next().into();
    let y: Reg = vregs.next().into();
    let cp: Reg = vregs.next().into();
    let gp: Reg = vregs.next().into();
    let rows: Reg = vregs.next().into();
    let cols: Reg = vregs.next().into();
    let back: Reg = vregs.next().into();
    let fg: Reg = vregs.next().into();
    let bg: Reg = vregs.next().into();
    let bold: Reg = vregs.next().into();
    let un: Reg = vregs.next().into();
    let glyph: Reg = vregs.next().into();
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
    // `emit_encode_utf8` scratch.
    let enc_b: Reg = vregs.next().into();
    let enc_sh: Reg = vregs.next().into();
    let enc_mask: Reg = vregs.next().into();
    // plan-70-C width path.
    let width: Reg = vregs.next().into();
    let prop: Reg = vregs.next().into();
    let wa: Reg = vregs.next().into();
    let wb: Reg = vregs.next().into();
    let wc: Reg = vregs.next().into();
    let trailc: Reg = vregs.next().into();
    let trailglyph: Reg = vregs.next().into();
    let after_trail = format!("{symbol}_dg_aftertrail");
    let w_have = format!("{symbol}_dg_whave");

    emit_gate_inactive(term_state_offset, &inactive, x, instructions);
    // `drawGlyph(row, column, codepoint)` — the point is row-first.
    instructions.extend([
        abi::move_register(y, abi::c_arg(0)),
        abi::move_register(x, abi::c_arg(1)),
        abi::move_register(cp, abi::c_arg(2)),
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
    // Skip control code points (would corrupt the presented escape stream).
    instructions.push(abi::compare_immediate(cp, "32"));
    instructions.push(abi::branch_lt(&inactive));
    // bug-725: skip anything that is not a Unicode scalar too — above U+10FFFF
    // (1114111) or in the surrogate band U+D800..U+DFFF (55296..57343). The UTF-8
    // encoder below would otherwise write an ill-formed sequence to the terminal.
    let scalar_ok = format!("{symbol}_dg_scalar_ok");
    instructions.extend([
        abi::compare_immediate(cp, "1114111"),
        abi::branch_gt(&inactive),
        abi::compare_immediate(cp, "55296"),
        abi::branch_lt(&scalar_ok),
        abi::compare_immediate(cp, "57343"),
        abi::branch_le(&inactive),
        abi::label(&scalar_ok),
    ]);
    emit_encode_utf8(
        cp,
        glyph,
        enc_b,
        enc_sh,
        enc_mask,
        &format!("{symbol}_dg_enc"),
        instructions,
    );
    // plan-70-C: a single glyph is one scalar (one cluster), so just its width. A
    // width-2 glyph reserves a WIDE_TRAIL neighbor at x+1; a zero-width glyph
    // (a lone combining mark) falls back to width 1.
    crate::codegen::string::unicode_props::emit_unicode_property_ptr_free(
        symbol,
        &format!("{symbol}_dg_pp"),
        cp,
        prop,
        wa,
        instructions,
        relocations,
    );
    crate::codegen::string::unicode_props::emit_read_boundclass_icb_charwidth_free(
        prop,
        wa,
        wb,
        width,
        wc,
        instructions,
    );
    instructions.extend([
        abi::compare_immediate(width, "0"),
        abi::branch_ne(&w_have),
        abi::move_immediate(width, "Integer", "1"),
        abi::label(&w_have),
    ]);
    emit_stamp_cell(
        &ctx,
        y,
        x,
        glyph,
        width,
        &format!("{symbol}_dgp"),
        &inactive,
        instructions,
    );
    instructions.extend([
        // Wide glyph: stamp the wide-trailing sentinel at x+1 (clipped by
        // emit_stamp_cell if off the right edge).
        abi::compare_immediate(width, "2"),
        abi::branch_ne(&after_trail),
        abi::add_immediate(trailc, x, 1),
        abi::move_immediate(trailglyph, "Integer", term_grid::WIDE_TRAIL),
        abi::move_immediate(wa, "Integer", "0"),
    ]);
    emit_stamp_cell(
        &ctx,
        y,
        trailc,
        trailglyph,
        wa,
        &format!("{symbol}_dgt"),
        &after_trail,
        instructions,
    );
    instructions.push(abi::label(&after_trail));
    instructions.push(abi::label(&inactive));
    instructions.push(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
}

/// `term::drawText(row, column, text)` (console): stamp `text` on `row` starting at
/// `column`, one grid cell per Unicode scalar, with the current attributes. It
/// does not move the shadow cursor, does not wrap or scroll, and clips at the right
/// edge (columns before 0 are skipped; the run stops at the last column). Control
/// characters are skipped (not stamped) but still advance a column. A no-op if the
/// row is off the grid or TUI mode is off. Shown on the next `term::sync`.
pub(super) fn emit_draw_text(
    symbol: &str,
    term_state_offset: usize,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    vregs: &mut Vregs,
) {
    let inactive = format!("{symbol}_inactive");
    let x: Reg = vregs.next().into();
    let y: Reg = vregs.next().into();
    let strobj: Reg = vregs.next().into();
    let gp: Reg = vregs.next().into();
    let rows: Reg = vregs.next().into();
    let cols: Reg = vregs.next().into();
    let back: Reg = vregs.next().into();
    let fg: Reg = vregs.next().into();
    let bg: Reg = vregs.next().into();
    let bold: Reg = vregs.next().into();
    let un: Reg = vregs.next().into();
    let ptr: Reg = vregs.next().into();
    let rem: Reg = vregs.next().into();
    let col: Reg = vregs.next().into();
    let b0: Reg = vregs.next().into();
    let len: Reg = vregs.next().into();
    let glyph: Reg = vregs.next().into();
    let t: Reg = vregs.next().into();
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
    // plan-70-C cluster walk + EGC pool + wide handling (mirrors emit_grid_write).
    let cp: Reg = vregs.next().into();
    let width: Reg = vregs.next().into();
    let prop: Reg = vregs.next().into();
    let sbc: Reg = vregs.next().into();
    let sicb: Reg = vregs.next().into();
    let clen: Reg = vregs.next().into();
    let pptr: Reg = vregs.next().into();
    let pb0: Reg = vregs.next().into();
    let plen: Reg = vregs.next().into();
    let pcp: Reg = vregs.next().into();
    let pprop: Reg = vregs.next().into();
    let pbc: Reg = vregs.next().into();
    let picb: Reg = vregs.next().into();
    let pwidth: Reg = vregs.next().into();
    let sa: Reg = vregs.next().into();
    let sb: Reg = vregs.next().into();
    let sc: Reg = vregs.next().into();
    let sd: Reg = vregs.next().into();
    let poolbase: Reg = vregs.next().into();
    let poolslot: Reg = vregs.next().into();
    let ncells: Reg = vregs.next().into();
    let trailc: Reg = vregs.next().into();
    let trailg: Reg = vregs.next().into();
    let loop_top = format!("{symbol}_dt_loop");
    let advance = format!("{symbol}_dt_adv");
    let skip_ctrl = format!("{symbol}_dt_ctrl");
    let l2 = format!("{symbol}_dt_l2");
    let l3 = format!("{symbol}_dt_l3");
    let clamp = format!("{symbol}_dt_clamp");
    let pack = format!("{symbol}_dt_pack");
    let cellw = format!("{symbol}_dt_cellw");
    let peek_top = format!("{symbol}_dt_peek");
    let peek_done = format!("{symbol}_dt_peekdone");
    let peek_nb = format!("{symbol}_dt_peeknb");
    let peek_l2 = format!("{symbol}_dt_pl2");
    let peek_l3 = format!("{symbol}_dt_pl3");
    let peek_clamp = format!("{symbol}_dt_pclamp");
    let peek_hlen = format!("{symbol}_dt_phlen");
    let peek_wok = format!("{symbol}_dt_pwok");
    let w_have = format!("{symbol}_dt_whave");
    let store_inline = format!("{symbol}_dt_inline");
    let pool_copy_loop = format!("{symbol}_dt_pcopy");
    let pool_copy_done = format!("{symbol}_dt_pcopydone");

    emit_gate_inactive(term_state_offset, &inactive, x, instructions);
    // `drawText(row, column, text)` — the point is row-first.
    instructions.extend([
        abi::move_register(y, abi::c_arg(0)),
        abi::move_register(x, abi::c_arg(1)),
        abi::move_register(strobj, abi::c_arg(2)),
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
    instructions.extend([
        // Row must be on the grid.
        abi::compare_immediate(y, "0"),
        abi::branch_lt(&inactive),
        abi::compare_registers(y, rows),
        abi::branch_ge(&inactive),
        // ptr = strobj + 8 (past the length word); rem = length; col = x.
        abi::add_immediate(ptr, strobj, 8),
        abi::load_u64(rem, strobj, 0),
        abi::move_register(col, x),
        abi::label(&loop_top),
        abi::compare_immediate(rem, "0"),
        abi::branch_eq(&inactive),
        // Clip at the right edge: once col reaches cols, nothing more is visible.
        abi::compare_registers(col, cols),
        abi::branch_ge(&inactive),
        abi::load_u8(b0, ptr, 0),
        // Control characters (< 0x20): skip stamping, advance one column + byte.
        abi::compare_immediate(b0, "32"),
        abi::branch_lt(&skip_ctrl),
        // UTF-8 length from the lead byte.
        abi::move_immediate(len, "Integer", "1"),
        abi::compare_immediate(b0, "128"),
        abi::branch_lo(&pack),
        abi::compare_immediate(b0, "224"),
        abi::branch_lo(&l2),
        abi::compare_immediate(b0, "240"),
        abi::branch_lo(&l3),
        abi::move_immediate(len, "Integer", "4"),
        abi::branch(&clamp),
        abi::label(&l2),
        abi::move_immediate(len, "Integer", "2"),
        abi::branch(&clamp),
        abi::label(&l3),
        abi::move_immediate(len, "Integer", "3"),
        abi::label(&clamp),
        // A truncated trailing sequence is treated as one raw byte.
        abi::compare_registers(len, rem),
        abi::branch_ls(&pack),
        abi::move_immediate(len, "Integer", "1"),
        abi::label(&pack),
        abi::move_register(glyph, b0),
        abi::compare_immediate(len, "2"),
        abi::branch_lo(&cellw),
        abi::load_u8(t, ptr, 1),
        abi::shift_left_immediate(t, t, 8),
        abi::or_registers(glyph, glyph, t),
        abi::compare_immediate(len, "3"),
        abi::branch_lo(&cellw),
        abi::load_u8(t, ptr, 2),
        abi::shift_left_immediate(t, t, 16),
        abi::or_registers(glyph, glyph, t),
        abi::compare_immediate(len, "4"),
        abi::branch_lo(&cellw),
        abi::load_u8(t, ptr, 3),
        abi::shift_left_immediate(t, t, 24),
        abi::or_registers(glyph, glyph, t),
        abi::label(&cellw),
    ]);
    // Base scalar → codepoint → boundclass/icb/charwidth; then fold following
    // non-breaking scalars into the cluster (plan-70-C, mirrors emit_grid_write).
    crate::codegen::string::unicode_props::emit_utf8_codepoint_by_len(
        &format!("{symbol}_dtd"),
        ptr,
        len,
        b0,
        cp,
        sa,
        sb,
        instructions,
    );
    crate::codegen::string::unicode_props::emit_unicode_property_ptr_free(
        symbol,
        &format!("{symbol}_dtb"),
        cp,
        prop,
        sa,
        instructions,
        relocations,
    );
    crate::codegen::string::unicode_props::emit_read_boundclass_icb_charwidth_free(
        prop,
        sbc,
        sicb,
        width,
        sa,
        instructions,
    );
    instructions.push(abi::move_register(clen, len));
    instructions.push(abi::label(&peek_top));
    instructions.extend([
        abi::subtract_registers(sc, rem, clen),
        abi::compare_immediate(sc, "0"),
        abi::branch_eq(&peek_done),
        abi::add_registers(pptr, ptr, clen),
        abi::load_u8(pb0, pptr, 0),
        abi::compare_immediate(pb0, "32"),
        abi::branch_lt(&peek_done),
        abi::move_immediate(plen, "Integer", "1"),
        abi::compare_immediate(pb0, "128"),
        abi::branch_lo(&peek_hlen),
        abi::compare_immediate(pb0, "224"),
        abi::branch_lo(&peek_l2),
        abi::compare_immediate(pb0, "240"),
        abi::branch_lo(&peek_l3),
        abi::move_immediate(plen, "Integer", "4"),
        abi::branch(&peek_clamp),
        abi::label(&peek_l2),
        abi::move_immediate(plen, "Integer", "2"),
        abi::branch(&peek_clamp),
        abi::label(&peek_l3),
        abi::move_immediate(plen, "Integer", "3"),
        abi::label(&peek_clamp),
        abi::compare_registers(plen, sc),
        abi::branch_ls(&peek_hlen),
        abi::move_immediate(plen, "Integer", "1"),
        abi::label(&peek_hlen),
        abi::add_registers(sc, clen, plen),
        abi::move_immediate(sd, "Integer", &term_grid::POOL_BYTES_PER_CELL.to_string()),
        abi::compare_registers(sc, sd),
        abi::branch_hi(&peek_done),
    ]);
    crate::codegen::string::unicode_props::emit_utf8_codepoint_by_len(
        &format!("{symbol}_dtpd"),
        pptr,
        plen,
        pb0,
        pcp,
        sa,
        sb,
        instructions,
    );
    crate::codegen::string::unicode_props::emit_unicode_property_ptr_free(
        symbol,
        &format!("{symbol}_dtpl"),
        pcp,
        pprop,
        sa,
        instructions,
        relocations,
    );
    crate::codegen::string::unicode_props::emit_read_boundclass_icb_charwidth_free(
        pprop,
        pbc,
        picb,
        pwidth,
        sa,
        instructions,
    );
    crate::codegen::string::unicode_props::emit_grapheme_break_branch_free(
        &format!("{symbol}_dtpb"),
        sbc,
        sicb,
        pbc,
        picb,
        &peek_done,
        &peek_nb,
        instructions,
    );
    instructions.push(abi::label(&peek_nb));
    instructions.extend([
        abi::add_registers(clen, clen, plen),
        abi::compare_immediate(width, "0"),
        abi::branch_ne(&peek_wok),
        abi::move_register(width, pwidth),
        abi::label(&peek_wok),
    ]);
    crate::codegen::string::unicode_props::emit_grapheme_state_update_free(
        &format!("{symbol}_dtps"),
        sbc,
        sicb,
        pbc,
        picb,
        instructions,
    );
    instructions.push(abi::branch(&peek_top));
    instructions.push(abi::label(&peek_done));
    instructions.extend([
        // A lone zero-width cluster still takes one cell / one column.
        abi::compare_immediate(width, "0"),
        abi::branch_ne(&w_have),
        abi::move_immediate(width, "Integer", "1"),
        abi::label(&w_have),
        // Clip a wide cluster that would exceed the right edge: drop it and stop
        // the run (never split a wide glyph across the edge).
        abi::compare_immediate(width, "2"),
        abi::branch_ne(&format!("{symbol}_dt_afterclip")),
        abi::add_immediate(sa, col, 1),
        abi::compare_registers(sa, cols),
        abi::branch_lo(&format!("{symbol}_dt_afterclip")),
        abi::branch(&inactive),
        abi::label(&format!("{symbol}_dt_afterclip")),
        // Glyph: inline single scalar, else pooled into the on-grid cell's slot.
        abi::compare_registers(clen, len),
        abi::branch_eq(&store_inline),
        abi::move_immediate(glyph, "Integer", &term_grid::GLYPH_POOLED_TAG.to_string()),
        abi::or_registers(glyph, glyph, clen),
        abi::compare_immediate(col, "0"),
        abi::branch_lt(&store_inline),
        // pool_base = gp + HDR + ncells*(2*CELL+OUTBUF); slot = pool_base + (y*cols+col)*POOL
        abi::multiply_registers(ncells, rows, cols),
        abi::move_immediate(
            sa,
            "Integer",
            &(2 * term_grid::CELL_SIZE + term_grid::OUTBUF_PER_CELL).to_string(),
        ),
        abi::multiply_registers(sa, ncells, sa),
        abi::add_immediate(sa, sa, term_grid::HDR_SIZE),
        abi::add_registers(poolbase, gp, sa),
        abi::multiply_registers(sa, y, cols),
        abi::add_registers(sa, sa, col),
        abi::move_immediate(sc, "Integer", &term_grid::POOL_BYTES_PER_CELL.to_string()),
        abi::multiply_registers(sc, sa, sc),
        abi::add_registers(poolslot, poolbase, sc),
        abi::move_immediate(sc, "Integer", "0"),
        abi::label(&pool_copy_loop),
        abi::compare_registers(sc, clen),
        abi::branch_ge(&pool_copy_done),
        abi::add_registers(sd, ptr, sc),
        abi::load_u8(sa, sd, 0),
        abi::add_registers(sd, poolslot, sc),
        abi::store_u8(sa, sd, 0),
        abi::add_immediate(sc, sc, 1),
        abi::branch(&pool_copy_loop),
        abi::label(&pool_copy_done),
        abi::label(&store_inline),
    ]);
    // Stamp the primary at (y, col) — off-grid (col<0 or off-row) skips to advance.
    emit_stamp_cell(
        &ctx,
        y,
        col,
        glyph,
        width,
        &format!("{symbol}_dtp"),
        &advance,
        instructions,
    );
    instructions.extend([
        // Wide glyph: stamp the wide-trailing sentinel at col+1.
        abi::compare_immediate(width, "2"),
        abi::branch_ne(&advance),
        abi::add_immediate(trailc, col, 1),
        abi::move_immediate(trailg, "Integer", term_grid::WIDE_TRAIL),
        abi::move_immediate(sa, "Integer", "0"),
    ]);
    emit_stamp_cell(
        &ctx,
        y,
        trailc,
        trailg,
        sa,
        &format!("{symbol}_dtt"),
        &advance,
        instructions,
    );
    instructions.extend([
        abi::label(&advance),
        abi::add_registers(col, col, width),
        abi::add_registers(ptr, ptr, clen),
        abi::subtract_registers(rem, rem, clen),
        abi::branch(&loop_top),
        // Control character: advance one column and one byte without stamping.
        abi::label(&skip_ctrl),
        abi::add_immediate(col, col, 1),
        abi::add_immediate(ptr, ptr, 1),
        abi::subtract_immediate(rem, rem, 1),
        abi::branch(&loop_top),
    ]);
    instructions.push(abi::label(&inactive));
    instructions.push(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
}
