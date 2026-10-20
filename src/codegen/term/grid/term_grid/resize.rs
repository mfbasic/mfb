//! plan-176-P: the terminal grid's resize (`emit_grid_resize`). The grid layout constants,
//! allocation, scrollback and freeing live in `term/grid/term_grid/mod.rs`.

use super::*;

/// Detect a terminal resize at `term::sync` entry and reflow the grid (plan-35-C
/// Phase 2). Re-reads the terminal size (TIOCGWINSZ); if it differs from the
/// header dims, allocates a new block, copies the top-left overlap from the old
/// back buffer (so content is preserved, matching the app backends), clamps the
/// cursor, sets `dirty` for a full repaint, publishes the new base pointer in
/// slot 48, and frees the old block. A no-op when the ioctl fails (e.g. stdout is
/// not a tty), the size is unchanged, or a new allocation cannot be obtained.
/// Uses sp locals `[8,56)` for the winsize struct + parked dims across the
/// arena calls — disjoint in time from the present's decimal scratch (`[32,56)`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_grid_resize(
    symbol: &str,
    term_state_offset: usize,
    request: &str,
    platform: &dyn CodegenPlatform,
    platform_imports: &HashMap<String, String>,
    instrs: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    vregs: &mut Vregs,
) -> Result<(), String> {
    // sp scratch layout for the resize (temporally before the present loop).
    const WINSZ: usize = 8;
    const S_NEWR: usize = 16;
    const S_NEWC: usize = 24;
    const S_OLDR: usize = 32;
    const S_OLDC: usize = 40;
    const S_OLDGP: usize = 48;
    let sp = abi::stack_pointer();
    let newr: Reg = vregs.next().into();
    let newc: Reg = vregs.next().into();
    let oldr: Reg = vregs.next().into();
    let oldc: Reg = vregs.next().into();
    let gp: Reg = vregs.next().into();
    let ng: Reg = vregs.next().into();
    let t: Reg = vregs.next().into();
    let m: Reg = vregs.next().into();
    let wc: Reg = vregs.next().into();
    let zp: Reg = vregs.next().into();
    let zero: Reg = vregs.next().into();
    let minr: Reg = vregs.next().into();
    let minc: Reg = vregs.next().into();
    let rr: Reg = vregs.next().into();
    let ob: Reg = vregs.next().into();
    let nb: Reg = vregs.next().into();
    let cc: Reg = vregs.next().into();
    let sptr: Reg = vregs.next().into();
    let dptr: Reg = vregs.next().into();
    let word: Reg = vregs.next().into();
    // plan-70-B Phase 2: pool-slot copy across the reflow.
    let opb: Reg = vregs.next().into();
    let npb: Reg = vregs.next().into();
    let prloop = format!("{symbol}_rz_prloop");
    let prdone = format!("{symbol}_rz_prdone");
    let pcloop = format!("{symbol}_rz_pcloop");
    let pcdone = format!("{symbol}_rz_pcdone");
    let skip = format!("{symbol}_rz_skip");
    let do_rz = format!("{symbol}_rz_do");
    let zloop = format!("{symbol}_rz_zloop");
    let zdone = format!("{symbol}_rz_zdone");
    let crok = format!("{symbol}_rz_crok");
    let ccok = format!("{symbol}_rz_ccok");
    let mrok = format!("{symbol}_rz_mrok");
    let mcok = format!("{symbol}_rz_mcok");
    let rloop = format!("{symbol}_rz_rloop");
    let rdone = format!("{symbol}_rz_rdone");
    let cloop = format!("{symbol}_rz_cloop");
    let cdone = format!("{symbol}_rz_cdone");
    // ioctl(1, TIOCGWINSZ, &winsize) — a failure leaves the grid unchanged.
    instrs.extend([
        abi::move_immediate(abi::return_register(), "Integer", "1"),
        abi::move_immediate(abi::c_arg(1), "Integer", request),
        abi::add_immediate(abi::c_arg(2), sp, WINSZ),
    ]);
    platform.emit_terminal_size(symbol, platform_imports, instrs, relocations)?;
    instrs.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&skip),
        abi::load_u16(newr, sp, WINSZ),
        abi::load_u16(newc, sp, WINSZ + 2),
        abi::compare_immediate(newr, "0"),
        abi::branch_eq(&skip),
        abi::compare_immediate(newc, "0"),
        abi::branch_eq(&skip),
        abi::load_u64(
            gp,
            ARENA_STATE_REGISTER,
            term_state_offset + TERM_STATE_GRID_OFFSET,
        ),
        abi::compare_immediate(gp, "0"),
        abi::branch_eq(&skip),
        abi::load_u64(oldr, gp, H_ROWS),
        abi::load_u64(oldc, gp, H_COLS),
        // Unchanged size (both dims equal) → nothing to do.
        abi::compare_registers(newr, oldr),
        abi::branch_ne(&do_rz),
        abi::compare_registers(newc, oldc),
        abi::branch_eq(&skip),
        abi::label(&do_rz),
        // planning/term.md #11: a genuine terminal size change was detected — latch
        // the resize flag so `term::didResize()` reports it. `term::didResize`
        // read-and-clears it. `t` is a scratch immediately overwritten below.
        abi::move_immediate(t, "Integer", "1"),
        abi::store_u64(
            t,
            ARENA_STATE_REGISTER,
            term_state_offset + TERM_STATE_DID_RESIZE_OFFSET,
        ),
        // Park dims + old base across the arena calls.
        abi::store_u64(newr, sp, S_NEWR),
        abi::store_u64(newc, sp, S_NEWC),
        abi::store_u64(oldr, sp, S_OLDR),
        abi::store_u64(oldc, sp, S_OLDC),
        abi::store_u64(gp, sp, S_OLDGP),
        // new block = arena_alloc(HDR + newR*newC*(2*CELL+OUTBUF), 8)
        abi::multiply_registers(t, newr, newc),
        abi::move_immediate(
            m,
            "Integer",
            &(2 * CELL_SIZE + OUTBUF_PER_CELL + POOL_BYTES_PER_CELL).to_string(),
        ),
        abi::multiply_registers(t, t, m),
        abi::add_immediate(t, t, HDR_SIZE + TRAILER_SLACK),
        abi::move_register(abi::return_register(), t),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
    ]);
    relocations.push(internal_branch(symbol, ARENA_ALLOC_SYMBOL));
    instrs.extend([
        // Allocation failed → keep the old grid (skip the resize this frame).
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_ne(&skip),
        abi::move_register(ng, RESULT_VALUE_REGISTER),
        // Zero-fill the new block.
        abi::load_u64(newr, sp, S_NEWR),
        abi::load_u64(newc, sp, S_NEWC),
        abi::multiply_registers(t, newr, newc),
        abi::move_immediate(
            m,
            "Integer",
            &(2 * CELL_SIZE + OUTBUF_PER_CELL + POOL_BYTES_PER_CELL).to_string(),
        ),
        abi::multiply_registers(t, t, m),
        abi::add_immediate(t, t, HDR_SIZE + TRAILER_SLACK),
        abi::shift_right_immediate(wc, t, 3),
        abi::move_register(zp, ng),
        abi::move_immediate(zero, "Integer", "0"),
        abi::label(&zloop),
        abi::compare_immediate(wc, "0"),
        abi::branch_eq(&zdone),
        abi::store_u64(zero, zp, 0),
        abi::add_immediate(zp, zp, 8),
        abi::subtract_immediate(wc, wc, 1),
        abi::branch(&zloop),
        abi::label(&zdone),
        // New header: rows/cols, clamped cursor, dirty = 1.
        abi::load_u64(oldr, sp, S_OLDR),
        abi::load_u64(oldc, sp, S_OLDC),
        abi::load_u64(gp, sp, S_OLDGP),
        abi::store_u64(newr, ng, H_ROWS),
        abi::store_u64(newc, ng, H_COLS),
        abi::load_u64(t, gp, H_CUR_ROW),
        abi::compare_registers(t, newr),
        abi::branch_lt(&crok),
        abi::subtract_immediate(t, newr, 1),
        abi::label(&crok),
        abi::store_u64(t, ng, H_CUR_ROW),
        abi::load_u64(t, gp, H_CUR_COL),
        abi::compare_registers(t, newc),
        abi::branch_lt(&ccok),
        abi::subtract_immediate(t, newc, 1),
        abi::label(&ccok),
        abi::store_u64(t, ng, H_CUR_COL),
        abi::move_immediate(t, "Integer", "1"),
        abi::store_u64(t, ng, H_DIRTY),
        // Copy the top-left overlap: minR = min(oldR,newR), minC = min(oldC,newC).
        abi::move_register(minr, oldr),
        abi::compare_registers(oldr, newr),
        abi::branch_le(&mrok),
        abi::move_register(minr, newr),
        abi::label(&mrok),
        abi::move_register(minc, oldc),
        abi::compare_registers(oldc, newc),
        abi::branch_le(&mcok),
        abi::move_register(minc, newc),
        abi::label(&mcok),
        abi::move_immediate(rr, "Integer", "0"),
        abi::label(&rloop),
        abi::compare_registers(rr, minr),
        abi::branch_ge(&rdone),
        // ob = oldback + rr*oldC*CELL ; nb = newback + rr*newC*CELL
        abi::multiply_registers(t, rr, oldc),
        abi::shift_left_immediate(t, t, 4),
        abi::add_immediate(ob, gp, HDR_SIZE),
        abi::add_registers(ob, ob, t),
        abi::multiply_registers(t, rr, newc),
        abi::shift_left_immediate(t, t, 4),
        abi::add_immediate(nb, ng, HDR_SIZE),
        abi::add_registers(nb, nb, t),
        // copy minC cells (minC*2 words) ob -> nb
        abi::shift_left_immediate(cc, minc, 1),
        abi::move_register(sptr, ob),
        abi::move_register(dptr, nb),
        abi::label(&cloop),
        abi::compare_immediate(cc, "0"),
        abi::branch_eq(&cdone),
        abi::load_u64(word, sptr, 0),
        abi::store_u64(word, dptr, 0),
        abi::add_immediate(sptr, sptr, 8),
        abi::add_immediate(dptr, dptr, 8),
        abi::subtract_immediate(cc, cc, 1),
        abi::branch(&cloop),
        abi::label(&cdone),
        abi::add_immediate(rr, rr, 1),
        abi::branch(&rloop),
        abi::label(&rdone),
        // plan-70-B Phase 2: copy the per-cell EGC pool slots for the same overlap,
        // so a pooled cluster survives the reflow (else its copied tag would point
        // at the new block's zeroed slot and emit NUL bytes). Pool bases follow the
        // cells: opb = gp + HDR + oldR*oldC*(2*CELL+OUTBUF); npb likewise for ng.
        abi::multiply_registers(t, oldr, oldc),
        abi::move_immediate(m, "Integer", &(2 * CELL_SIZE + OUTBUF_PER_CELL).to_string()),
        abi::multiply_registers(t, t, m),
        abi::add_immediate(t, t, HDR_SIZE),
        abi::add_registers(opb, gp, t),
        abi::multiply_registers(t, newr, newc),
        abi::move_immediate(m, "Integer", &(2 * CELL_SIZE + OUTBUF_PER_CELL).to_string()),
        abi::multiply_registers(t, t, m),
        abi::add_immediate(t, t, HDR_SIZE),
        abi::add_registers(npb, ng, t),
        abi::move_immediate(rr, "Integer", "0"),
        abi::label(&prloop),
        abi::compare_registers(rr, minr),
        abi::branch_ge(&prdone),
        // sptr = opb + rr*oldC*POOL ; dptr = npb + rr*newC*POOL
        abi::multiply_registers(t, rr, oldc),
        abi::move_immediate(m, "Integer", &POOL_BYTES_PER_CELL.to_string()),
        abi::multiply_registers(t, t, m),
        abi::add_registers(sptr, opb, t),
        abi::multiply_registers(t, rr, newc),
        abi::move_immediate(m, "Integer", &POOL_BYTES_PER_CELL.to_string()),
        abi::multiply_registers(t, t, m),
        abi::add_registers(dptr, npb, t),
        // copy minC slots = minC*(POOL/8) words
        abi::move_immediate(m, "Integer", &(POOL_BYTES_PER_CELL / 8).to_string()),
        abi::multiply_registers(cc, minc, m),
        abi::label(&pcloop),
        abi::compare_immediate(cc, "0"),
        abi::branch_eq(&pcdone),
        abi::load_u64(word, sptr, 0),
        abi::store_u64(word, dptr, 0),
        abi::add_immediate(sptr, sptr, 8),
        abi::add_immediate(dptr, dptr, 8),
        abi::subtract_immediate(cc, cc, 1),
        abi::branch(&pcloop),
        abi::label(&pcdone),
        abi::add_immediate(rr, rr, 1),
        abi::branch(&prloop),
        abi::label(&prdone),
        // Publish the new block, then free the old one.
        abi::store_u64(
            ng,
            ARENA_STATE_REGISTER,
            term_state_offset + TERM_STATE_GRID_OFFSET,
        ),
        abi::load_u64(oldr, sp, S_OLDR),
        abi::load_u64(oldc, sp, S_OLDC),
        abi::load_u64(gp, sp, S_OLDGP),
        abi::multiply_registers(t, oldr, oldc),
        abi::move_immediate(
            m,
            "Integer",
            &(2 * CELL_SIZE + OUTBUF_PER_CELL + POOL_BYTES_PER_CELL).to_string(),
        ),
        abi::multiply_registers(t, t, m),
        abi::add_immediate(t, t, HDR_SIZE + TRAILER_SLACK),
        abi::move_register(abi::return_register(), gp),
        abi::move_register(abi::c_arg(1), t),
        abi::branch_link(ARENA_FREE_SYMBOL),
    ]);
    relocations.push(internal_branch(symbol, ARENA_FREE_SYMBOL));
    instrs.push(abi::label(&skip));
    Ok(())
}
