//! plan-176-P: the terminal grid's presenter — building the escape run for changed cells
//! (`emit_grid_present`). The grid layout constants, allocation, scrollback and freeing live in
//! `term/grid/term_grid/mod.rs`.

use super::*;

/// Emit an append of `bytes` (a fixed ASCII/escape run) into the buffer at
/// cursor `buf`, advancing `buf` by `bytes.len()`. `scratch` is a throwaway vreg.
fn append_const(buf: Reg, bytes: &[u8], scratch: Reg, instrs: &mut Vec<CodeInstruction>) {
    for byte in bytes {
        instrs.push(abi::move_immediate(scratch, "Integer", &byte.to_string()));
        instrs.push(abi::store_u8(scratch, buf, 0));
        instrs.push(abi::add_immediate(buf, buf, 1));
    }
}

/// Throwaway vregs for the present's append helpers, minted once by
/// `emit_grid_present` and shared by every append it emits: each helper clobbers
/// them and none holds a value across a call to another. Minting them once (rather
/// than per call) keeps the present's register count flat however many appends it
/// makes.
struct AppendScratch {
    /// `append_decimal`: quotient, constant 10, digit cursor, divide result,
    /// remainder digit, digit-region end, copied byte.
    q: Reg,
    ten: Reg,
    tp: Reg,
    d: Reg,
    r: Reg,
    e: Reg,
    c: Reg,
    /// `append_rgb`: the channel being appended and the 0xFF mask.
    ch: Reg,
    ch_mask: Reg,
    /// `append_glyph`: the byte being appended, the blank's space, the 0xFF mask.
    glyph_byte: Reg,
    glyph_space: Reg,
    glyph_mask: Reg,
}

/// Append the unsigned decimal text of `val` into `buf`, advancing `buf`. Digits
/// are generated least-significant-first into the sp-relative scratch region
/// `[tmp_start, tmp_end)` (no call happens, so the region is stable), then copied
/// forward into the buffer. `tag` makes the loop labels unique.
fn append_decimal(
    buf: Reg,
    val: Reg,
    tmp_end: usize,
    tag: &str,
    scratch: &AppendScratch,
    instrs: &mut Vec<CodeInstruction>,
) {
    let AppendScratch {
        q,
        ten,
        tp,
        d,
        r,
        e,
        c,
        ..
    } = *scratch;
    let gen = format!("{tag}_dgen");
    let copy = format!("{tag}_dcopy");
    let cdone = format!("{tag}_ddone");
    instrs.extend([
        abi::move_register(q, val),
        abi::move_immediate(ten, "Integer", "10"),
        abi::add_immediate(tp, abi::stack_pointer(), tmp_end),
        abi::label(&gen),
        abi::unsigned_divide_registers(d, q, ten),
        abi::multiply_subtract_registers(r, d, ten, q),
        abi::add_immediate(r, r, 48),
        abi::subtract_immediate(tp, tp, 1),
        abi::store_u8(r, tp, 0),
        abi::move_register(q, d),
        abi::compare_immediate(q, "0"),
        abi::branch_ne(&gen),
        // Copy digits forward from tp..(sp+tmp_end) into buf. The loop-exit test
        // is `(tp ^ e) == 0` so the rv64 selector's pending rhs stays an immediate
        // (register-reuse safe under spilling). [[bug-126.2]]
        abi::add_immediate(e, abi::stack_pointer(), tmp_end),
        abi::label(&copy),
        abi::exclusive_or_registers(d, tp, e),
        abi::compare_immediate(d, "0"),
        abi::branch_eq(&cdone),
        abi::load_u8(c, tp, 0),
        abi::store_u8(c, buf, 0),
        abi::add_immediate(buf, buf, 1),
        abi::add_immediate(tp, tp, 1),
        abi::branch(&copy),
        abi::label(&cdone),
    ]);
}

/// Append the three decimal channels of a packed `r|g<<8|b<<16` colour separated
/// by `;` (the tail of an SGR truecolor sequence, without the leading prefix).
fn append_rgb(
    buf: Reg,
    packed: Reg,
    tmp_end: usize,
    tag: &str,
    scratch: &AppendScratch,
    instrs: &mut Vec<CodeInstruction>,
) {
    let ch = scratch.ch;
    let m = scratch.ch_mask;
    instrs.push(abi::move_immediate(m, "Integer", "255"));
    instrs.push(abi::and_registers(ch, packed, m));
    append_decimal(buf, ch, tmp_end, &format!("{tag}_r"), scratch, instrs);
    append_const(buf, b";", ch, instrs);
    instrs.push(abi::shift_right_immediate(ch, packed, 8));
    instrs.push(abi::move_immediate(m, "Integer", "255"));
    instrs.push(abi::and_registers(ch, ch, m));
    append_decimal(buf, ch, tmp_end, &format!("{tag}_g"), scratch, instrs);
    append_const(buf, b";", ch, instrs);
    instrs.push(abi::shift_right_immediate(ch, packed, 16));
    instrs.push(abi::move_immediate(m, "Integer", "255"));
    instrs.push(abi::and_registers(ch, ch, m));
    append_decimal(buf, ch, tmp_end, &format!("{tag}_b"), scratch, instrs);
}

/// Append the glyph's raw UTF-8 bytes (packed little-endian in `glyph`), or a
/// single space when the cell is blank (glyph 0). Stops at the first zero byte.
fn append_glyph(
    buf: Reg,
    glyph: Reg,
    tag: &str,
    scratch: &AppendScratch,
    instrs: &mut Vec<CodeInstruction>,
) {
    let b = scratch.glyph_byte;
    let sp = scratch.glyph_space;
    let blank = format!("{tag}_gblank");
    let done = format!("{tag}_gdone");
    let m = scratch.glyph_mask;
    instrs.extend([
        abi::compare_immediate(glyph, "0"),
        abi::branch_eq(&blank),
        abi::move_immediate(m, "Integer", "255"),
        // byte 0 (always present for a non-blank glyph)
        abi::and_registers(b, glyph, m),
        abi::store_u8(b, buf, 0),
        abi::add_immediate(buf, buf, 1),
        // byte 1
        abi::shift_right_immediate(b, glyph, 8),
        abi::and_registers(b, b, m),
        abi::compare_immediate(b, "0"),
        abi::branch_eq(&done),
        abi::store_u8(b, buf, 0),
        abi::add_immediate(buf, buf, 1),
        // byte 2
        abi::shift_right_immediate(b, glyph, 16),
        abi::and_registers(b, b, m),
        abi::compare_immediate(b, "0"),
        abi::branch_eq(&done),
        abi::store_u8(b, buf, 0),
        abi::add_immediate(buf, buf, 1),
        // byte 3
        abi::shift_right_immediate(b, glyph, 24),
        abi::and_registers(b, b, m),
        abi::compare_immediate(b, "0"),
        abi::branch_eq(&done),
        abi::store_u8(b, buf, 0),
        abi::add_immediate(buf, buf, 1),
        abi::branch(&done),
        abi::label(&blank),
        abi::move_immediate(sp, "Integer", "32"),
        abi::store_u8(sp, buf, 0),
        abi::add_immediate(buf, buf, 1),
        abi::label(&done),
    ]);
}

#[allow(clippy::too_many_arguments)]
/// Console present (plan-35-C): diff the back buffer against the front buffer and
/// emit only the changed cells (minimal CUP + coalesced SGR + glyphs) as one
/// batched `write(2)` into fd 1, then copy back→front and restore the cursor.
/// A set `dirty` flag forces a full repaint (first present after on/resize).
/// Inlined into the `term::sync` helper; `term::off` reaches it via `bl`.
pub(crate) fn emit_grid_present(
    symbol: &str,
    term_state_offset: usize,
    tmp_end: usize,
    request: &str,
    platform: &dyn CodegenPlatform,
    platform_imports: &HashMap<String, String>,
    instrs: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    vregs: &mut Vregs,
) -> Result<(), String> {
    let gp: Reg = vregs.next().into();
    let rows: Reg = vregs.next().into();
    let cols: Reg = vregs.next().into();
    let dirty: Reg = vregs.next().into();
    let ncells: Reg = vregs.next().into();
    let back: Reg = vregs.next().into();
    let front: Reg = vregs.next().into();
    let outbuf: Reg = vregs.next().into();
    let buf: Reg = vregs.next().into();
    let idx: Reg = vregs.next().into();
    let row: Reg = vregs.next().into();
    let col: Reg = vregs.next().into();
    let off: Reg = vregs.next().into();
    let bc: Reg = vregs.next().into();
    let fc: Reg = vregs.next().into();
    let last_valid: Reg = vregs.next().into();
    let last_row: Reg = vregs.next().into();
    let last_col: Reg = vregs.next().into();
    let last_fg: Reg = vregs.next().into();
    let last_bg: Reg = vregs.next().into();
    let last_bold: Reg = vregs.next().into();
    let last_un: Reg = vregs.next().into();
    let cfg: Reg = vregs.next().into();
    let cbg: Reg = vregs.next().into();
    let cbold: Reg = vregs.next().into();
    let cun: Reg = vregs.next().into();
    let glyph: Reg = vregs.next().into();
    let a0: Reg = vregs.next().into();
    let b0: Reg = vregs.next().into();
    let tmp: Reg = vregs.next().into();
    let rowp1: Reg = vregs.next().into();
    let colp1: Reg = vregs.next().into();
    // bug-314 H3: the present-write loop's cursor / remaining / result.
    let wcur: Reg = vregs.next().into();
    let wrem: Reg = vregs.next().into();
    let wres: Reg = vregs.next().into();
    // plan-70-B: this cell's stored display width, for the cursor-advance model.
    let cwidth: Reg = vregs.next().into();
    // plan-70-B Phase 2: pooled-cluster emission.
    let pe_len: Reg = vregs.next().into();
    let pe_base: Reg = vregs.next().into();
    let pe_slot: Reg = vregs.next().into();
    let pe_i: Reg = vregs.next().into();
    let pe_byte: Reg = vregs.next().into();
    let pe_sc: Reg = vregs.next().into();
    let pe_dst: Reg = vregs.next().into();
    // The append helpers' scratch. The decimal formatter's first three share the
    // pooled-cluster copy's byte/scratch/destination vregs: the copy and each
    // decimal append run at different points of a cell's emission, and neither
    // holds a value across the other.
    let append = AppendScratch {
        q: pe_byte,
        ten: pe_sc,
        tp: pe_dst,
        d: vregs.next().into(),
        r: vregs.next().into(),
        e: vregs.next().into(),
        c: vregs.next().into(),
        ch: vregs.next().into(),
        ch_mask: vregs.next().into(),
        glyph_byte: vregs.next().into(),
        glyph_space: vregs.next().into(),
        glyph_mask: vregs.next().into(),
    };

    let done_ok = format!("{symbol}_pr_ok");
    let loop_top = format!("{symbol}_pr_loop");
    let after = format!("{symbol}_pr_after");
    let emit_cell = format!("{symbol}_pr_emit");
    let advance = format!("{symbol}_pr_adv");
    let trail_present = format!("{symbol}_pr_trail");
    let adv1 = format!("{symbol}_pr_adv1");
    let after_setcol = format!("{symbol}_pr_aftercol");
    let inline_glyph = format!("{symbol}_pr_inlineg");
    let pool_emit_loop = format!("{symbol}_pr_pemit");
    let pool_emit_done = format!("{symbol}_pr_pemitdone");
    let after_glyph = format!("{symbol}_pr_afterg");
    let cup_done = format!("{symbol}_pr_cupdone");
    let do_cup = format!("{symbol}_pr_docup");
    let sgr_done = format!("{symbol}_pr_sgrdone");
    let do_sgr = format!("{symbol}_pr_dosgr");
    let no_bold = format!("{symbol}_pr_nobold");
    let no_un = format!("{symbol}_pr_noun");
    let col_wrap = format!("{symbol}_pr_colwrap");
    let cur_hide = format!("{symbol}_pr_curhide");
    let cur_after = format!("{symbol}_pr_curafter");

    instrs.extend([
        abi::load_u64(
            gp,
            ARENA_STATE_REGISTER,
            term_state_offset + TERM_STATE_GRID_OFFSET,
        ),
        abi::compare_immediate(gp, "0"),
        abi::branch_eq(&done_ok),
    ]);
    // Reflow the grid first if the terminal was resized (may replace the block
    // and set `dirty`), then re-read the base pointer + dims.
    emit_grid_resize(
        symbol,
        term_state_offset,
        request,
        platform,
        platform_imports,
        instrs,
        relocations,
        vregs,
    )?;
    instrs.extend([
        abi::load_u64(
            gp,
            ARENA_STATE_REGISTER,
            term_state_offset + TERM_STATE_GRID_OFFSET,
        ),
        abi::load_u64(rows, gp, H_ROWS),
        abi::load_u64(cols, gp, H_COLS),
        abi::load_u64(dirty, gp, H_DIRTY),
        abi::multiply_registers(ncells, rows, cols),
        abi::add_immediate(back, gp, HDR_SIZE),
        abi::shift_left_immediate(off, ncells, 4),
        abi::add_registers(front, back, off),
        abi::add_registers(outbuf, front, off),
        abi::move_register(buf, outbuf),
        abi::move_immediate(last_valid, "Integer", "0"),
        abi::move_immediate(last_row, "Integer", "0"),
        abi::move_immediate(last_col, "Integer", "0"),
        abi::move_immediate(last_fg, "Integer", "0"),
        abi::move_immediate(last_bg, "Integer", "0"),
        abi::move_immediate(last_bold, "Integer", "0"),
        abi::move_immediate(last_un, "Integer", "0"),
        abi::move_immediate(idx, "Integer", "0"),
        abi::move_immediate(row, "Integer", "0"),
        abi::move_immediate(col, "Integer", "0"),
        abi::label(&loop_top),
        abi::compare_registers(idx, ncells),
        abi::branch_ge(&after),
        abi::shift_left_immediate(off, idx, 4),
        abi::add_registers(bc, back, off),
        abi::add_registers(fc, front, off),
        // Skip unchanged cells unless a full repaint is forced. Equality is tested
        // as `(a ^ b) == 0` — the compare's rhs is the immediate 0, which the rv64
        // flagless selector keeps register-independent, so a spill reusing a
        // compare operand under pressure cannot strand the branch (select.rs
        // pending is by-register). [[bug-126.2]]
        abi::compare_immediate(dirty, "0"),
        abi::branch_ne(&emit_cell),
        // plan-70-B Phase 2: a pooled cell's cluster bytes live in the pool slot,
        // outside the 16-byte cell the XOR diff compares, so a same-tag same-attr
        // overwrite would be missed — always re-emit a pooled cell (glyph top byte
        // == 0xC0). Inline 4-byte scalars can set bit 31 but never the 0xC0 top
        // byte, so this does not force-emit ordinary astral glyphs.
        abi::load_u32(a0, bc, C_GLYPH),
        abi::shift_right_immediate(a0, a0, 24),
        abi::compare_immediate(a0, "192"),
        abi::branch_eq(&emit_cell),
        abi::load_u64(a0, bc, 0),
        abi::load_u64(b0, fc, 0),
        abi::exclusive_or_registers(a0, a0, b0),
        abi::compare_immediate(a0, "0"),
        abi::branch_ne(&emit_cell),
        abi::load_u64(a0, bc, 8),
        abi::load_u64(b0, fc, 8),
        abi::exclusive_or_registers(a0, a0, b0),
        abi::compare_immediate(a0, "0"),
        abi::branch_ne(&emit_cell),
        abi::branch(&advance),
        abi::label(&emit_cell),
        // plan-70-B: a wide-trailing sentinel draws nothing — the wide glyph in its
        // primary cell already covers this column, and the primary advanced the
        // cursor model by 2. Mark it presented and move on without a CUP/SGR/glyph.
        abi::load_u32(glyph, bc, C_GLYPH),
        abi::move_immediate(a0, "Integer", WIDE_TRAIL),
        abi::compare_registers(glyph, a0),
        abi::branch_eq(&trail_present),
        // Cursor: emit a CUP unless the terminal cursor is already here. Equality
        // via xor + compare-to-zero keeps the rv64 selector's pending rhs an
        // immediate (register-reuse safe under spilling). [[bug-126.2]]
        abi::compare_immediate(last_valid, "0"),
        abi::branch_eq(&do_cup),
        abi::exclusive_or_registers(a0, last_row, row),
        abi::compare_immediate(a0, "0"),
        abi::branch_ne(&do_cup),
        abi::exclusive_or_registers(a0, last_col, col),
        abi::compare_immediate(a0, "0"),
        abi::branch_ne(&do_cup),
        abi::branch(&cup_done),
        abi::label(&do_cup),
    ]);
    append_const(buf, b"\x1b[", tmp, instrs);
    instrs.push(abi::add_immediate(rowp1, row, 1));
    append_decimal(
        buf,
        rowp1,
        tmp_end,
        &format!("{symbol}_prcur"),
        &append,
        instrs,
    );
    append_const(buf, b";", tmp, instrs);
    instrs.push(abi::add_immediate(colp1, col, 1));
    append_decimal(
        buf,
        colp1,
        tmp_end,
        &format!("{symbol}_prcuc"),
        &append,
        instrs,
    );
    append_const(buf, b"H", tmp, instrs);
    instrs.push(abi::label(&cup_done));
    // Load this cell's attributes.
    instrs.extend([
        abi::load_u32(cfg, bc, C_FG),
        abi::load_u32(cbg, bc, C_BG),
        abi::load_u8(cbold, bc, C_BOLD),
        abi::load_u8(cun, bc, C_UN),
        // SGR only when the attributes differ from the last emitted set (xor +
        // compare-to-zero, register-reuse safe on rv64). [[bug-126.2]]
        abi::compare_immediate(last_valid, "0"),
        abi::branch_eq(&do_sgr),
        abi::exclusive_or_registers(a0, cfg, last_fg),
        abi::compare_immediate(a0, "0"),
        abi::branch_ne(&do_sgr),
        abi::exclusive_or_registers(a0, cbg, last_bg),
        abi::compare_immediate(a0, "0"),
        abi::branch_ne(&do_sgr),
        abi::exclusive_or_registers(a0, cbold, last_bold),
        abi::compare_immediate(a0, "0"),
        abi::branch_ne(&do_sgr),
        abi::exclusive_or_registers(a0, cun, last_un),
        abi::compare_immediate(a0, "0"),
        abi::branch_ne(&do_sgr),
        abi::branch(&sgr_done),
        abi::label(&do_sgr),
    ]);
    // Reset, then apply bold/underline/fg/bg for an exact attribute set.
    append_const(buf, b"\x1b[0m", tmp, instrs);
    instrs.push(abi::compare_immediate(cbold, "0"));
    instrs.push(abi::branch_eq(&no_bold));
    append_const(buf, b"\x1b[1m", tmp, instrs);
    instrs.push(abi::label(&no_bold));
    instrs.push(abi::compare_immediate(cun, "0"));
    instrs.push(abi::branch_eq(&no_un));
    append_const(buf, b"\x1b[4m", tmp, instrs);
    instrs.push(abi::label(&no_un));
    append_const(buf, b"\x1b[38;2;", tmp, instrs);
    append_rgb(
        buf,
        cfg,
        tmp_end,
        &format!("{symbol}_prfg"),
        &append,
        instrs,
    );
    append_const(buf, b"m", tmp, instrs);
    append_const(buf, b"\x1b[48;2;", tmp, instrs);
    append_rgb(
        buf,
        cbg,
        tmp_end,
        &format!("{symbol}_prbg"),
        &append,
        instrs,
    );
    append_const(buf, b"m", tmp, instrs);
    instrs.extend([
        abi::move_register(last_fg, cfg),
        abi::move_register(last_bg, cbg),
        abi::move_register(last_bold, cbold),
        abi::move_register(last_un, cun),
        abi::label(&sgr_done),
    ]);
    // plan-70-B Phase 2: emit the cluster. A pooled cell (glyph high bit set)
    // copies its cluster bytes from this cell's pool slot; otherwise the inline
    // packed scalar bytes via `append_glyph`. `glyph` was loaded at `emit_cell`.
    instrs.extend([
        abi::move_immediate(pe_sc, "Integer", &GLYPH_POOLED_TAG.to_string()),
        abi::compare_registers(glyph, pe_sc),
        abi::branch_lo(&inline_glyph),
        // len = glyph & 0x00FF_FFFF (strip the 0xC0 pooled tag byte).
        abi::move_immediate(pe_sc, "Integer", &GLYPH_POOLED_LEN_MASK.to_string()),
        abi::and_registers(pe_len, glyph, pe_sc),
        // pool_base = gp + HDR_SIZE + ncells*(2*CELL + OUTBUF).
        abi::move_immediate(
            pe_sc,
            "Integer",
            &(2 * CELL_SIZE + OUTBUF_PER_CELL).to_string(),
        ),
        abi::multiply_registers(pe_base, ncells, pe_sc),
        abi::add_immediate(pe_base, pe_base, HDR_SIZE),
        abi::add_registers(pe_base, gp, pe_base),
        // slot = pool_base + idx*POOL_BYTES_PER_CELL.
        abi::move_immediate(pe_sc, "Integer", &POOL_BYTES_PER_CELL.to_string()),
        abi::multiply_registers(pe_slot, idx, pe_sc),
        abi::add_registers(pe_slot, pe_base, pe_slot),
        // Copy the cluster's bytes into the output buffer and advance it.
        abi::move_immediate(pe_i, "Integer", "0"),
        abi::label(&pool_emit_loop),
        abi::compare_registers(pe_i, pe_len),
        abi::branch_ge(&pool_emit_done),
        abi::add_registers(pe_dst, pe_slot, pe_i),
        abi::load_u8(pe_byte, pe_dst, 0),
        abi::add_registers(pe_dst, buf, pe_i),
        abi::store_u8(pe_byte, pe_dst, 0),
        abi::add_immediate(pe_i, pe_i, 1),
        abi::branch(&pool_emit_loop),
        abi::label(&pool_emit_done),
        abi::add_registers(buf, buf, pe_len),
        abi::branch(&after_glyph),
        abi::label(&inline_glyph),
    ]);
    append_glyph(buf, glyph, &format!("{symbol}_prg"), &append, instrs);
    instrs.push(abi::label(&after_glyph));
    // The terminal cursor is now `width` columns past this cell's start: a wide
    // glyph (stored width 2) leaves the cursor 2 columns on, so a following run of
    // changed cells stays column-aligned without a redundant CUP (the bug-392
    // auto-advance fix). Any stored width other than 2 (including 0 on cells the
    // width-aware writer never touched) advances a single column.
    instrs.extend([
        abi::move_immediate(last_valid, "Integer", "1"),
        abi::move_register(last_row, row),
        abi::load_u8(cwidth, bc, C_WIDTH),
        abi::compare_immediate(cwidth, "2"),
        abi::branch_ne(&adv1),
        abi::add_immediate(last_col, col, 2),
        abi::branch(&after_setcol),
        abi::label(&adv1),
        abi::add_immediate(last_col, col, 1),
        abi::label(&after_setcol),
        // Mark the cell presented: copy back→front (16 bytes = 2 words). A
        // wide-trailing sentinel joins here too (nothing emitted, still synced).
        abi::label(&trail_present),
        abi::load_u64(a0, bc, 0),
        abi::store_u64(a0, fc, 0),
        abi::load_u64(a0, bc, 8),
        abi::store_u64(a0, fc, 8),
        abi::label(&advance),
        abi::add_immediate(idx, idx, 1),
        abi::add_immediate(col, col, 1),
        abi::compare_registers(col, cols),
        abi::branch_lo(&col_wrap),
        abi::move_immediate(col, "Integer", "0"),
        abi::add_immediate(row, row, 1),
        abi::label(&col_wrap),
        abi::branch(&loop_top),
        abi::label(&after),
    ]);
    // Reset SGR, restore cursor position + visibility.
    append_const(buf, b"\x1b[0m", tmp, instrs);
    append_const(buf, b"\x1b[", tmp, instrs);
    instrs.push(abi::load_u64(tmp, gp, H_CUR_ROW));
    instrs.push(abi::add_immediate(rowp1, tmp, 1));
    append_decimal(
        buf,
        rowp1,
        tmp_end,
        &format!("{symbol}_prrr"),
        &append,
        instrs,
    );
    append_const(buf, b";", tmp, instrs);
    instrs.push(abi::load_u64(tmp, gp, H_CUR_COL));
    instrs.push(abi::add_immediate(colp1, tmp, 1));
    append_decimal(
        buf,
        colp1,
        tmp_end,
        &format!("{symbol}_prrc"),
        &append,
        instrs,
    );
    append_const(buf, b"H", tmp, instrs);
    instrs.push(abi::load_u64(
        tmp,
        ARENA_STATE_REGISTER,
        term_state_offset + TERM_STATE_CURSOR_VISIBLE_OFFSET,
    ));
    instrs.push(abi::compare_immediate(tmp, "0"));
    instrs.push(abi::branch_eq(&cur_hide));
    append_const(buf, b"\x1b[?25h", tmp, instrs);
    instrs.push(abi::branch(&cur_after));
    instrs.push(abi::label(&cur_hide));
    append_const(buf, b"\x1b[?25l", tmp, instrs);
    instrs.push(abi::label(&cur_after));
    // Clear the dirty flag; the next present diffs from the now-synced front.
    instrs.push(abi::move_immediate(tmp, "Integer", "0"));
    instrs.push(abi::store_u64(tmp, gp, H_DIRTY));
    // Batched write(fd=1, outbuf, buf - outbuf), looped until every byte lands.
    //
    // bug-314 H3: the result used to be discarded. That is uniquely damaging here
    // because the diff loop copies each emitted cell back->front BEFORE this write:
    // if the write went short or was interrupted after a partial transfer, the
    // terminal showed only part of the frame while `front == back` claimed it was
    // fully painted, so the next `term::sync` diffed to nothing and never repaired
    // the missing cells. Not a dropped frame -- permanent corruption until
    // something else happened to mark those cells dirty again.
    //
    // Looping to completion keeps the front buffer's claim true. The alternative
    // (copy back->front only after a successful write) would need the emitted-cell
    // list kept alive across the write, which the streaming diff does not retain.
    let write_loop = format!("{symbol}_present_write_loop");
    let write_done = format!("{symbol}_present_write_done");
    let cursor = wcur;
    let remaining = wrem;
    instrs.extend([
        abi::move_register(cursor, outbuf),
        abi::subtract_registers(remaining, buf, outbuf),
        abi::label(&write_loop),
        // Nothing left: done.
        abi::compare_immediate(remaining, "0"),
        abi::branch_eq(&write_done),
        abi::move_register(abi::string_length_register(), remaining),
        abi::move_register(abi::string_data_register(), cursor),
        abi::move_immediate(abi::return_register(), "Integer", "1"),
    ]);
    platform.emit_write(symbol, platform_imports, instrs, relocations)?;
    instrs.push(abi::move_register(wres, abi::return_register()));
    // bug-410: route the write result through the shared transfer tail every
    // sibling write loop uses. A positive count advances the cursor and re-loops; a
    // negative return is EINTR (a signal — SIGWINCH resize / SIGCHLD / the console
    // SIGINT — interrupted the present mid-`write`; MFBASIC handlers are not
    // SA_RESTART) and is re-issued at `write_loop` from the unchanged
    // cursor/remaining; a 0 return for a nonzero-length write moved nothing and is a
    // hard give-up (bug-62). The give-up target is `write_done`: this present has no
    // error channel and `front == back` already claims the frame painted, but a real
    // EINTR is now repaired instead of leaving the frame permanently corrupt. The
    // dead `write_retry` label (bug-314 H3's vestige, never branched to) is gone —
    // the retry edge is the loop top, matching `emit_transfer_loop_tail`.
    emit_transfer_loop_tail(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: instrs,
            relocations,
        },
        wres,
        write_uses_raw_syscall(platform),
        cursor,
        remaining,
        &write_loop,
        &write_done,
    )?;
    instrs.push(abi::label(&write_done));
    instrs.push(abi::label(&done_ok));
    Ok(())
}
