//! plan-176-P: the terminal grid's cell writer (`emit_grid_write`). The grid layout constants,
//! allocation, scrollback and freeing live in `term/grid/term_grid/mod.rs`.

use super::*;

/// Grid writer, inlined into the console `io::write` helper's TUI-active branch.
/// Reads the String object at `strobj` (`[len:u64 | bytes…]`), the current
/// attributes from the term-state global, and the header cursor, then stamps each
/// glyph into the back buffer honouring `\n`/`\r`, wrap, and scroll — never
/// emitting to the terminal. When `append_newline` is set (io::print) a trailing
/// newline advances the cursor too.
pub(crate) fn emit_grid_write(
    symbol: &str,
    term_state_offset: usize,
    strobj: impl Into<Reg>,
    append_newline: bool,
    instrs: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    vregs: &mut Vregs,
) {
    let strobj: Reg = strobj.into();
    let gp: Reg = vregs.next().into();
    let back: Reg = vregs.next().into();
    let rows: Reg = vregs.next().into();
    let cols: Reg = vregs.next().into();
    let row: Reg = vregs.next().into();
    let col: Reg = vregs.next().into();
    let fg: Reg = vregs.next().into();
    let bg: Reg = vregs.next().into();
    let bold: Reg = vregs.next().into();
    let un: Reg = vregs.next().into();
    let ptr: Reg = vregs.next().into();
    let rem: Reg = vregs.next().into();
    let b0: Reg = vregs.next().into();
    let len: Reg = vregs.next().into();
    let glyph: Reg = vregs.next().into();
    let t: Reg = vregs.next().into();
    let idx: Reg = vregs.next().into();
    let cell: Reg = vregs.next().into();
    // plan-70-B width path: codepoint decode + charwidth lookup + wide handling.
    let cp: Reg = vregs.next().into();
    let width: Reg = vregs.next().into();
    let sa: Reg = vregs.next().into();
    let sb: Reg = vregs.next().into();
    let trail: Reg = vregs.next().into();
    // plan-70-B Phase 2 cluster walk + EGC pool.
    let prop: Reg = vregs.next().into();
    let state_bc: Reg = vregs.next().into();
    let state_icb: Reg = vregs.next().into();
    let clen: Reg = vregs.next().into(); // total cluster byte length
    let pptr: Reg = vregs.next().into(); // peek: pointer at ptr + clen
    let pb0: Reg = vregs.next().into(); // peek: lead byte
    let plen: Reg = vregs.next().into(); // peek: scalar byte length
    let pcp: Reg = vregs.next().into(); // peek: codepoint
    let pprop: Reg = vregs.next().into(); // peek: property ptr
    let pbc: Reg = vregs.next().into(); // peek: boundclass
    let picb: Reg = vregs.next().into(); // peek: indic-conjunct-break
    let pwidth: Reg = vregs.next().into(); // peek: charwidth
    let sc: Reg = vregs.next().into(); // scratch
    let sd: Reg = vregs.next().into(); // scratch
    let sd2: Reg = vregs.next().into(); // scratch byte
    let cellidx: Reg = vregs.next().into(); // linear cell index (row*cols+col) for the pool slot
    let ncells: Reg = vregs.next().into();
    let poolbase: Reg = vregs.next().into();
    let poolslot: Reg = vregs.next().into();
    // The scroll's scratch is this writer's per-glyph scratch: at every scroll
    // point (after the cluster is measured, at `\n`, and at the newline tail) each
    // of these is rewritten before it is next read.
    let scroll = ScrollScratch {
        ncells: sa,
        moved: sb,
        src: trail,
        dst: prop,
        word: state_bc,
        cnt: state_icb,
    };
    let done = format!("{symbol}_gw_done");
    let loop_top = format!("{symbol}_gw_loop");
    let handle_nl = format!("{symbol}_gw_nl");
    let handle_cr = format!("{symbol}_gw_cr");
    let l2 = format!("{symbol}_gw_l2");
    let l3 = format!("{symbol}_gw_l3");
    let clamp = format!("{symbol}_gw_clamp");
    let pack = format!("{symbol}_gw_pack");
    let cellw = format!("{symbol}_gw_cell");
    let col_ok = format!("{symbol}_gw_colok");
    let row_ok = format!("{symbol}_gw_rowok");
    let nl_ok = format!("{symbol}_gw_nlok");
    let w_have = format!("{symbol}_gw_whave");
    let wide_edge = format!("{symbol}_gw_wideedge");
    let after_trail = format!("{symbol}_gw_aftertrail");
    let peek_top = format!("{symbol}_gw_peek");
    let peek_done = format!("{symbol}_gw_peekdone");
    let peek_nobreak = format!("{symbol}_gw_peeknb");
    let peek_l2 = format!("{symbol}_gw_pl2");
    let peek_l3 = format!("{symbol}_gw_pl3");
    let peek_clamp = format!("{symbol}_gw_pclamp");
    let peek_have_len = format!("{symbol}_gw_phlen");
    let peek_wok = format!("{symbol}_gw_pwok");
    let pool_copy_loop = format!("{symbol}_gw_pcopy");
    let pool_copy_done = format!("{symbol}_gw_pcopydone");
    let store_inline = format!("{symbol}_gw_inline");

    instrs.extend([
        abi::load_u64(
            gp,
            ARENA_STATE_REGISTER,
            term_state_offset + TERM_STATE_GRID_OFFSET,
        ),
        abi::compare_immediate(gp, "0"),
        abi::branch_eq(&done),
        abi::add_immediate(back, gp, HDR_SIZE),
        abi::load_u64(rows, gp, H_ROWS),
        abi::load_u64(cols, gp, H_COLS),
        abi::load_u64(row, gp, H_CUR_ROW),
        abi::load_u64(col, gp, H_CUR_COL),
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
        abi::add_immediate(ptr, strobj, 8),
        abi::load_u64(rem, strobj, 0),
        abi::label(&loop_top),
        abi::compare_immediate(rem, "0"),
        abi::branch_eq(&done),
        abi::load_u8(b0, ptr, 0),
        abi::compare_immediate(b0, "10"),
        abi::branch_eq(&handle_nl),
        abi::compare_immediate(b0, "13"),
        abi::branch_eq(&handle_cr),
        // Determine UTF-8 sequence length from the lead byte.
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
        // Clamp len to remaining bytes; a truncated tail is treated as 1 raw byte.
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
    // plan-70-B Phase 2: build the whole extended grapheme cluster starting at the
    // base scalar, then place it width-aware. The base scalar's bytes are already
    // packed into `glyph` (inline path); `clen` grows as following non-breaking
    // scalars are folded in, and a multi-scalar cluster is stored in the cell's
    // pool slot instead. Base scalar: codepoint, then boundclass/icb/charwidth.
    crate::codegen::string::unicode_props::emit_utf8_codepoint_by_len(
        &format!("{symbol}_gwd"),
        ptr,
        len,
        b0,
        cp,
        sa,
        sb,
        instrs,
    );
    crate::codegen::string::unicode_props::emit_unicode_property_ptr_free(
        symbol,
        &format!("{symbol}_gwb"),
        cp,
        prop,
        sa,
        instrs,
        relocations,
    );
    crate::codegen::string::unicode_props::emit_read_boundclass_icb_charwidth_free(
        prop, state_bc, state_icb, width, sa, instrs,
    );
    crate::codegen::string::unicode_props::emit_seed_indic_conjunct_state_free(
        &format!("{symbol}_gws"),
        state_icb,
        instrs,
    );
    instrs.push(abi::move_register(clen, len));
    // Peek: fold following scalars into the cluster while there is no UAX #29
    // grapheme boundary, capping the cluster at POOL_BYTES_PER_CELL bytes.
    instrs.push(abi::label(&peek_top));
    instrs.extend([
        abi::subtract_registers(sc, rem, clen),
        abi::compare_immediate(sc, "0"),
        abi::branch_eq(&peek_done),
        abi::add_registers(pptr, ptr, clen),
        abi::load_u8(pb0, pptr, 0),
        // A newline/CR always ends the cluster (handled by the outer loop next).
        abi::compare_immediate(pb0, "10"),
        abi::branch_eq(&peek_done),
        abi::compare_immediate(pb0, "13"),
        abi::branch_eq(&peek_done),
        // UTF-8 length of the peeked scalar, clamped to what remains.
        abi::move_immediate(plen, "Integer", "1"),
        abi::compare_immediate(pb0, "128"),
        abi::branch_lo(&peek_have_len),
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
        abi::branch_ls(&peek_have_len),
        abi::move_immediate(plen, "Integer", "1"),
        abi::label(&peek_have_len),
        // Graceful cap: never let a cluster exceed the pool slot.
        abi::add_registers(sc, clen, plen),
        abi::move_immediate(sd, "Integer", &POOL_BYTES_PER_CELL.to_string()),
        abi::compare_registers(sc, sd),
        abi::branch_hi(&peek_done),
    ]);
    crate::codegen::string::unicode_props::emit_utf8_codepoint_by_len(
        &format!("{symbol}_gwpd"),
        pptr,
        plen,
        pb0,
        pcp,
        sa,
        sb,
        instrs,
    );
    crate::codegen::string::unicode_props::emit_unicode_property_ptr_free(
        symbol,
        &format!("{symbol}_gwpl"),
        pcp,
        pprop,
        sa,
        instrs,
        relocations,
    );
    crate::codegen::string::unicode_props::emit_read_boundclass_icb_charwidth_free(
        pprop, pbc, picb, pwidth, sa, instrs,
    );
    crate::codegen::string::unicode_props::emit_grapheme_break_branch_free(
        &format!("{symbol}_gwpb"),
        state_bc,
        state_icb,
        pbc,
        picb,
        &peek_done,
        &peek_nobreak,
        instrs,
    );
    instrs.push(abi::label(&peek_nobreak));
    instrs.extend([
        abi::add_registers(clen, clen, plen),
        // Cluster width = its first non-zero-width scalar's width.
        abi::compare_immediate(width, "0"),
        abi::branch_ne(&peek_wok),
        abi::move_register(width, pwidth),
        abi::label(&peek_wok),
    ]);
    crate::codegen::string::unicode_props::emit_grapheme_state_update_free(
        &format!("{symbol}_gwps"),
        state_bc,
        state_icb,
        pbc,
        picb,
        instrs,
    );
    instrs.push(abi::branch(&peek_top));
    instrs.push(abi::label(&peek_done));
    instrs.extend([
        // An all-zero-width cluster (a lone combining mark) still takes one cell
        // and advances one column.
        abi::compare_immediate(width, "0"),
        abi::branch_ne(&w_have),
        abi::move_immediate(width, "Integer", "1"),
        abi::label(&w_have),
        // Wrap at the right edge.
        abi::compare_registers(col, cols),
        abi::branch_lo(&wide_edge),
        abi::move_immediate(col, "Integer", "0"),
        abi::add_immediate(row, row, 1),
        abi::label(&wide_edge),
        // A width-2 glyph at the last column wraps rather than straddling the edge.
        abi::compare_immediate(width, "2"),
        abi::branch_ne(&col_ok),
        abi::add_immediate(sa, col, 1),
        abi::compare_registers(sa, cols),
        abi::branch_lo(&col_ok),
        abi::move_immediate(col, "Integer", "0"),
        abi::add_immediate(row, row, 1),
        abi::label(&col_ok),
        // Scroll at the bottom.
        abi::compare_registers(row, rows),
        abi::branch_lo(&row_ok),
    ]);
    emit_scroll_back(back, rows, cols, &scroll, &format!("{symbol}_gwp"), instrs);
    instrs.extend([
        abi::subtract_immediate(row, rows, 1),
        abi::label(&row_ok),
        // cell = back + (row*cols + col) * CELL_SIZE; keep the linear index for the
        // per-cell pool slot.
        abi::multiply_registers(idx, row, cols),
        abi::add_registers(idx, idx, col),
        abi::move_register(cellidx, idx),
        abi::shift_left_immediate(idx, idx, 4),
        abi::add_registers(cell, back, idx),
        // A single-scalar cluster (clen == the base scalar length) stays inline in
        // the packed glyph word; a multi-scalar cluster goes to the pool slot.
        abi::compare_registers(clen, len),
        abi::branch_eq(&store_inline),
        // pool_base = gp + HDR_SIZE + ncells*(2*CELL + OUTBUF) (the pool region
        // follows back+front+outbuf); slot = pool_base + cellidx*POOL_BYTES_PER_CELL.
        abi::multiply_registers(ncells, rows, cols),
        abi::move_immediate(
            sa,
            "Integer",
            &(2 * CELL_SIZE + OUTBUF_PER_CELL).to_string(),
        ),
        abi::multiply_registers(sa, ncells, sa),
        abi::add_immediate(sa, sa, HDR_SIZE),
        abi::add_registers(poolbase, gp, sa),
        abi::move_immediate(sa, "Integer", &POOL_BYTES_PER_CELL.to_string()),
        abi::multiply_registers(sa, cellidx, sa),
        abi::add_registers(poolslot, poolbase, sa),
        // Copy the cluster's bytes [ptr, ptr+clen) into the slot.
        abi::move_immediate(sc, "Integer", "0"),
        abi::label(&pool_copy_loop),
        abi::compare_registers(sc, clen),
        abi::branch_ge(&pool_copy_done),
        abi::add_registers(sd, ptr, sc),
        abi::load_u8(sd2, sd, 0),
        abi::add_registers(sd, poolslot, sc),
        abi::store_u8(sd2, sd, 0),
        abi::add_immediate(sc, sc, 1),
        abi::branch(&pool_copy_loop),
        abi::label(&pool_copy_done),
        // glyph = pooled tag | byte length.
        abi::move_immediate(glyph, "Integer", &GLYPH_POOLED_TAG.to_string()),
        abi::or_registers(glyph, glyph, clen),
        abi::label(&store_inline),
        abi::store_u32(glyph, cell, C_GLYPH),
        abi::store_u32(fg, cell, C_FG),
        abi::store_u32(bg, cell, C_BG),
        abi::store_u8(bold, cell, C_BOLD),
        abi::store_u8(un, cell, C_UN),
        abi::store_u8(width, cell, C_WIDTH),
        // A wide glyph reserves the next cell as a wide-trailing sentinel (same
        // row: the wide-at-edge wrap above guarantees col <= cols-2 here). The
        // presenter emits nothing for it and does not advance the cursor past it.
        abi::compare_immediate(width, "2"),
        abi::branch_ne(&after_trail),
        abi::add_immediate(trail, cell, CELL_SIZE),
        abi::move_immediate(sa, "Integer", WIDE_TRAIL),
        abi::store_u32(sa, trail, C_GLYPH),
        abi::store_u32(fg, trail, C_FG),
        abi::store_u32(bg, trail, C_BG),
        abi::store_u8(bold, trail, C_BOLD),
        abi::store_u8(un, trail, C_UN),
        abi::move_immediate(sb, "Integer", "0"),
        abi::store_u8(sb, trail, C_WIDTH),
        abi::label(&after_trail),
        abi::add_registers(col, col, width),
        abi::add_registers(ptr, ptr, clen),
        abi::subtract_registers(rem, rem, clen),
        abi::branch(&loop_top),
        // \n : col = 0, row++, scroll if needed.
        abi::label(&handle_nl),
        abi::move_immediate(col, "Integer", "0"),
        abi::add_immediate(row, row, 1),
        abi::compare_registers(row, rows),
        abi::branch_lo(&nl_ok),
    ]);
    emit_scroll_back(back, rows, cols, &scroll, &format!("{symbol}_gwn"), instrs);
    instrs.extend([
        abi::subtract_immediate(row, rows, 1),
        abi::label(&nl_ok),
        abi::add_immediate(ptr, ptr, 1),
        abi::subtract_immediate(rem, rem, 1),
        abi::branch(&loop_top),
        // \r : col = 0.
        abi::label(&handle_cr),
        abi::move_immediate(col, "Integer", "0"),
        abi::add_immediate(ptr, ptr, 1),
        abi::subtract_immediate(rem, rem, 1),
        abi::branch(&loop_top),
        abi::label(&done),
    ]);
    if append_newline {
        let tail_ok = format!("{symbol}_gw_tail_ok");
        let tail_skip = format!("{symbol}_gw_tail_skip");
        // Only advance if the grid exists (gp still valid; recompute defensively).
        instrs.extend([
            abi::compare_immediate(gp, "0"),
            abi::branch_eq(&tail_skip),
            abi::move_immediate(col, "Integer", "0"),
            abi::add_immediate(row, row, 1),
            abi::compare_registers(row, rows),
            abi::branch_lo(&tail_ok),
        ]);
        emit_scroll_back(back, rows, cols, &scroll, &format!("{symbol}_gwt"), instrs);
        instrs.extend([
            abi::subtract_immediate(row, rows, 1),
            abi::label(&tail_ok),
            abi::label(&tail_skip),
        ]);
    }
    // Persist the cursor back into the header (skipped harmlessly if gp was null:
    // the store target would be gp+offset, but we only reach here after the null
    // check branched to `done`; guard once more for the newline-tail path).
    let persist_skip = format!("{symbol}_gw_persist_skip");
    instrs.extend([
        abi::compare_immediate(gp, "0"),
        abi::branch_eq(&persist_skip),
        abi::store_u64(row, gp, H_CUR_ROW),
        abi::store_u64(col, gp, H_CUR_COL),
        abi::label(&persist_skip),
    ]);
}
