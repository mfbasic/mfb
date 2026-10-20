//! Console shadow-grid backend for the retained, double-buffered `term::` surface
//! (plan-35-B / plan-35-C). Neutral `abi::` codegen (plan-35 D3): one
//! implementation shared across aarch64/x86/riscv, emitting only virtual
//! registers (`%vN`) and sp-relative locals so it stays byte-deterministic and
//! satisfies the zero-physical-register invariant (plan-34-D).
//!
//! While TUI mode is on, `term::on` allocates one arena header block sized to the
//! terminal; `io::write` and the `term::` drawing calls mutate a **back** cell
//! buffer (never the terminal); and `term::sync` presents by diffing the back
//! buffer against the last-presented **front** buffer, emitting only the changed
//! cells as a single batched `write(2)`. The block also carries a scratch output
//! buffer the present builds the escape stream into, so there are no per-cell
//! syscalls and no values held live across a call inside the present loop.
//!
//! Block layout (base pointer stored in term-state slot 48):
//! ```text
//!   0   rows       u64
//!   8   cols       u64
//!  16   cursorRow  u64
//!  24   cursorCol  u64
//!  32   dirty      u64      (1 forces a full repaint: first present after on/resize)
//!  40   back cells    rows*cols * 16
//!  ...  front cells   rows*cols * 16
//!  ...  out buffer    rows*cols * OUTBUF_PER_CELL   (escape-stream scratch)
//! ```
//! A **cell** is 16 bytes: glyph (u32, raw UTF-8 bytes packed little-endian; 0 =
//! blank), fg (u32 packed `r|g<<8|b<<16`), bg (u32), bold (u8), underline (u8).

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::Vregs;
use crate::codegen::error::constants::*;
use crate::codegen::os::syscall::*;
use crate::target::shared::abi;
use crate::target::shared::abi::Reg;
use std::collections::HashMap;

mod present;
mod resize;
mod write;
pub(crate) use present::emit_grid_present;
pub(crate) use resize::emit_grid_resize;
pub(crate) use write::emit_grid_write;

/// Term-state slot holding the console grid header-block base pointer (0 = off).
pub(crate) const TERM_STATE_GRID_OFFSET: usize = 48;

// Header field offsets.
const H_ROWS: usize = 0;
const H_COLS: usize = 8;
const H_CUR_ROW: usize = 16;
const H_CUR_COL: usize = 24;
const H_DIRTY: usize = 32;
// Header size / cell field offsets are shared with `term/core/term/`'s drawing helpers
// (drawHLine/drawVLine stamp glyphs straight into the same back buffer), so they
// are `pub(crate)` to keep a single source of truth for the layout.
pub(crate) const HDR_SIZE: usize = 40;

// Cell layout (16 bytes).
pub(crate) const CELL_SIZE: usize = 16;
pub(crate) const C_GLYPH: usize = 0;
pub(crate) const C_FG: usize = 4;
pub(crate) const C_BG: usize = 8;
pub(crate) const C_BOLD: usize = 12;
pub(crate) const C_UN: usize = 13;
// plan-70-B: display width (0/1/2) of the cluster this cell holds, computed at
// write time from A's charwidth table and read by the presenter so a wide glyph
// advances the terminal cursor by 2 columns. Lives in a previously-unused pad
// byte, so `CELL_SIZE` is unchanged.
pub(crate) const C_WIDTH: usize = 14;
// A wide cluster occupies its primary cell (`width` 2) followed by this
// **wide-trailing sentinel** in the next column: a reserved glyph value the
// presenter and draw helpers skip (it emits nothing) but that keeps the second
// column occupied for cursor math and diffing. 0xFFFF_FFFF is not a valid packed
// UTF-8 scalar (a lead byte 0xFF never appears), so it can never collide with a
// real glyph.
pub(crate) const WIDE_TRAIL: &str = "4294967295";

/// Worst-case escape bytes emitted per changed cell. Sizes the per-block output
/// buffer.
///
/// bug-313: this was 64, described as fitting "comfortably". It does not — 64 is
/// the worst case *exactly*, with zero margin, and only for coordinates below
/// 1000. Counted from the sequences this module actually emits, for a coordinate
/// of `d` digits:
///
/// ```text
///   CUP    ESC [ <row> ; <col> H      2 + d + 1 + d + 1  = 2d + 4
///   SGR    ESC [0m                    4
///          ESC [1m                    4   (bold)
///          ESC [4m                    4   (underline)
///          ESC [38;2;<r>;<g>;<b>m     7 + 11 + 1 = 19    (fg, 3 digits each)
///          ESC [48;2;<r>;<g>;<b>m     19                 (bg)
///   glyph  up to 4 UTF-8 bytes        4
///                                     -----------------
///                                     2d + 58
/// ```
///
/// So d=3 needs exactly 64, d=4 needs 66 and d=5 needs 68 — and `rows`/`cols`
/// come from raw `TIOCGWINSZ` `ws_row`/`ws_col` as u16 with no clamp, so d reaches
/// 5. Past d=3 each changed cell overran its share, and once the aggregate excess
/// passed `TRAILER_SLACK` the present loop wrote beyond the arena grid block into
/// adjacent arena memory.
///
/// plan-70-B Phase 2: a cell can now emit a pooled multi-scalar grapheme cluster
/// of up to `POOL_BYTES_PER_CELL` bytes (vs. one 4-byte scalar before), so the
/// per-cell worst case grows from `2d+58` to `2d+54+POOL_BYTES_PER_CELL`. At d=5
/// and a 64-byte pool slot that is 128; 136 covers it with margin. The diff
/// coalesces SGR, so the steady state remains far below this; the budget only has
/// to bound the pathological repaint. (`outbuf_per_cell_covers_the_worst_case_escape_run`.)
pub(crate) const OUTBUF_PER_CELL: usize = 136;

/// plan-70-B Phase 2: bytes reserved per cell for a pooled (multi-scalar) grapheme
/// cluster — a combining sequence, ZWJ emoji family, or regional-indicator flag
/// whose UTF-8 exceeds the 4 bytes that pack inline into the cell glyph word. Each
/// cell owns a dedicated slot at `pool_base + cell_index * POOL_BYTES_PER_CELL`, so
/// the pool needs no bump pointer, no reset, and no compaction (the cell's slot is
/// overwritten whenever the cell is): the lifecycle problem the umbrella flags is
/// sidestepped by fixed per-cell slots. 64 covers a base plus a long run of
/// combining marks / a 4-emoji ZWJ family (25 bytes); a cluster longer than this
/// stores as many whole leading scalars as fit (graceful cap).
pub(crate) const POOL_BYTES_PER_CELL: usize = 64;

/// Glyph-word tag for a **pooled** cell: top byte `0xC0`, low 24 bits the cluster
/// byte length (its bytes live in the per-cell pool slot). The high bit ALONE is
/// NOT free — every 4-byte UTF-8 scalar packs its 4th (continuation) byte, always
/// `0x80..=0xBF`, into bits 24-31, so an inline astral glyph (e.g. `😀` =
/// `0x80989FF0`) has bit 31 set. But an inline glyph's top byte is only ever `0x00`
/// (1-3 byte scalars) or `0x80..=0xBF` (4-byte) — never `0xC0..=0xFE` — so `0xC0`
/// is a collision-free tag. `WIDE_TRAIL` (top byte `0xFF`) is distinct and is
/// handled before the pooled check. Length <= `POOL_BYTES_PER_CELL` fits the low
/// 24 bits with room to spare.
pub(crate) const GLYPH_POOLED_TAG: i64 = 0xC000_0000;
/// Mask recovering a pooled cell's cluster byte length from its glyph word (clears
/// the `0xC0` tag byte).
const GLYPH_POOLED_LEN_MASK: i64 = 0x00FF_FFFF;

/// Extra bytes reserved past the exact `rows*cols*OUTBUF_PER_CELL` out-buffer so
/// the fixed trailing reset/CUP/cursor sequence (~24 bytes) appended after the
/// per-cell repaint loop has headroom even on a near-saturating repaint
/// (pathological geometry). The out buffer is the final region of the block, so
/// this slack lands at the block tail (bug-175 G).
const TRAILER_SLACK: usize = 64;

const DEFAULT_ROWS: &str = "24";
const DEFAULT_COLS: &str = "80";

/// The six throwaway vregs `emit_scroll_back` clobbers: the cell count, the moved
/// cell count, the copy source/destination cursors, the word being moved, and the
/// word counter.
#[derive(Clone, Copy)]
struct ScrollScratch {
    ncells: Reg,
    moved: Reg,
    src: Reg,
    dst: Reg,
    word: Reg,
    cnt: Reg,
}

/// Scroll the back buffer up one row: shift rows 1..rows into 0..rows-1 and blank
/// the last row. `back` is the cell base, `rows`/`cols` the grid dims. No calls.
///
/// plan-70-B Phase 2: the per-cell EGC pool slots are position-indexed, so a
/// pooled cluster that scrolls would otherwise render the destination cell's stale
/// slot. This shifts the pool region (which follows the cells, at
/// `back + ncells*(2*CELL+OUTBUF)`) in lockstep and blanks the last row's slots.
/// `POOL_BYTES_PER_CELL` is a multiple of 8 so the shift stays word-aligned.
///
/// `scratch` is six caller vregs this clobbers, all of which must be dead at the
/// call. `emit_grid_write` lends its per-glyph scratch rather than minting six
/// more: every one of them is rewritten before it is next read at each scroll
/// point.
fn emit_scroll_back(
    back: Reg,
    rows: Reg,
    cols: Reg,
    scratch: &ScrollScratch,
    tag: &str,
    instrs: &mut Vec<CodeInstruction>,
) {
    let ScrollScratch {
        ncells,
        moved,
        src,
        dst,
        word,
        cnt,
    } = *scratch;
    let copy = format!("{tag}_sc_copy");
    let copy_done = format!("{tag}_sc_cdone");
    let clr = format!("{tag}_sc_clr");
    let clr_done = format!("{tag}_sc_cldone");
    let pcopy = format!("{tag}_sc_pcopy");
    let pcopy_done = format!("{tag}_sc_pcdone");
    let pclr = format!("{tag}_sc_pclr");
    let pclr_done = format!("{tag}_sc_pcldone");
    instrs.extend([
        abi::multiply_registers(ncells, rows, cols),
        // moved cells = (ncells - cols); copy that many cells up by `cols`.
        abi::subtract_registers(moved, ncells, cols),
        // Word count = moved * CELL_SIZE / 8 = moved * 2.
        abi::shift_left_immediate(cnt, moved, 1),
        abi::move_register(dst, back),
        // src = back + cols*CELL_SIZE
        abi::shift_left_immediate(src, cols, 4),
        abi::add_registers(src, back, src),
        abi::label(&copy),
        abi::compare_immediate(cnt, "0"),
        abi::branch_eq(&copy_done),
        abi::load_u64(word, src, 0),
        abi::store_u64(word, dst, 0),
        abi::add_immediate(src, src, 8),
        abi::add_immediate(dst, dst, 8),
        abi::subtract_immediate(cnt, cnt, 1),
        abi::branch(&copy),
        abi::label(&copy_done),
        // Blank the last row: `cols` cells starting at dst, in 8-byte words.
        abi::shift_left_immediate(cnt, cols, 1),
        abi::move_immediate(word, "Integer", "0"),
        abi::label(&clr),
        abi::compare_immediate(cnt, "0"),
        abi::branch_eq(&clr_done),
        abi::store_u64(word, dst, 0),
        abi::add_immediate(dst, dst, 8),
        abi::subtract_immediate(cnt, cnt, 1),
        abi::branch(&clr),
        abi::label(&clr_done),
        // --- shift the per-cell EGC pool slots up one row too (plan-70-B P2) ---
        // pool_base = back + ncells*(2*CELL + OUTBUF); dst = pool_base.
        abi::move_immediate(
            word,
            "Integer",
            &(2 * CELL_SIZE + OUTBUF_PER_CELL).to_string(),
        ),
        abi::multiply_registers(dst, ncells, word),
        abi::add_registers(dst, back, dst),
        // src = pool_base + cols*POOL_BYTES_PER_CELL (row 1's slots).
        abi::move_immediate(word, "Integer", &POOL_BYTES_PER_CELL.to_string()),
        abi::multiply_registers(src, cols, word),
        abi::add_registers(src, dst, src),
        // words to move = moved slots * (POOL_BYTES_PER_CELL / 8).
        abi::move_immediate(word, "Integer", &(POOL_BYTES_PER_CELL / 8).to_string()),
        abi::multiply_registers(cnt, moved, word),
        abi::label(&pcopy),
        abi::compare_immediate(cnt, "0"),
        abi::branch_eq(&pcopy_done),
        abi::load_u64(word, src, 0),
        abi::store_u64(word, dst, 0),
        abi::add_immediate(src, src, 8),
        abi::add_immediate(dst, dst, 8),
        abi::subtract_immediate(cnt, cnt, 1),
        abi::branch(&pcopy),
        abi::label(&pcopy_done),
        // Blank the last row's pool slots: cols slots * (POOL/8) words at dst.
        abi::move_immediate(word, "Integer", &(POOL_BYTES_PER_CELL / 8).to_string()),
        abi::multiply_registers(cnt, cols, word),
        abi::move_immediate(word, "Integer", "0"),
        abi::label(&pclr),
        abi::compare_immediate(cnt, "0"),
        abi::branch_eq(&pclr_done),
        abi::store_u64(word, dst, 0),
        abi::add_immediate(dst, dst, 8),
        abi::subtract_immediate(cnt, cnt, 1),
        abi::branch(&pclr),
        abi::label(&pclr_done),
    ]);
}

#[allow(clippy::too_many_arguments)]
/// Allocate the console grid header block on `term::on`. Sizes it to the current
/// terminal (via the TIOCGWINSZ ioctl, defaulting to 24x80 when unavailable),
/// arena-allocates `HDR + rows*cols*(2*CELL + OUTBUF_PER_CELL)` bytes, zero-fills
/// it (a cleared grid), writes the header (dirty = 1 forces the first full
/// repaint), and stores the base pointer in term-state slot 48. On allocation
/// failure it branches to `fail_label` (the caller emits the OOM result). Parks
/// rows/cols in the caller's `rows_slot`/`cols_slot` sp locals across the arena
/// call. `winsize_off` is the sp offset of the `winsize` scratch struct.
pub(crate) fn emit_grid_alloc(
    symbol: &str,
    term_state_offset: usize,
    request: &str,
    winsize_off: usize,
    rows_slot: usize,
    cols_slot: usize,
    fail_label: &str,
    platform: &dyn CodegenPlatform,
    platform_imports: &HashMap<String, String>,
    instrs: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    vregs: &mut Vregs,
) -> Result<(), String> {
    let rowsv: Reg = vregs.next().into();
    let colsv: Reg = vregs.next().into();
    let t: Reg = vregs.next().into();
    let m: Reg = vregs.next().into();
    let gp: Reg = vregs.next().into();
    let wc: Reg = vregs.next().into();
    let zp: Reg = vregs.next().into();
    let zero: Reg = vregs.next().into();
    let one: Reg = vregs.next().into();
    let size_fail = format!("{symbol}_ga_sizefail");
    let size_ok = format!("{symbol}_ga_sizeok");
    let zloop = format!("{symbol}_ga_zloop");
    let zdone = format!("{symbol}_ga_zdone");
    // ioctl(1, TIOCGWINSZ, &winsize)
    instrs.extend([
        abi::move_immediate(abi::return_register(), "Integer", "1"),
        abi::move_immediate(abi::c_arg(1), "Integer", request),
        abi::add_immediate(abi::c_arg(2), abi::stack_pointer(), winsize_off),
    ]);
    platform.emit_terminal_size(symbol, platform_imports, instrs, relocations)?;
    instrs.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&size_fail),
        abi::load_u16(rowsv, abi::stack_pointer(), winsize_off),
        abi::load_u16(colsv, abi::stack_pointer(), winsize_off + 2),
        abi::compare_immediate(rowsv, "0"),
        abi::branch_eq(&size_fail),
        abi::compare_immediate(colsv, "0"),
        abi::branch_eq(&size_fail),
        abi::branch(&size_ok),
        abi::label(&size_fail),
        abi::move_immediate(rowsv, "Integer", DEFAULT_ROWS),
        abi::move_immediate(colsv, "Integer", DEFAULT_COLS),
        abi::label(&size_ok),
        abi::store_u64(rowsv, abi::stack_pointer(), rows_slot),
        abi::store_u64(colsv, abi::stack_pointer(), cols_slot),
        // size = HDR_SIZE + rows*cols*(2*CELL_SIZE + OUTBUF_PER_CELL)
        abi::multiply_registers(t, rowsv, colsv),
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
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_ne(fail_label),
        abi::move_register(gp, RESULT_VALUE_REGISTER),
        // Recompute the byte size from the parked dims and zero-fill the block.
        abi::load_u64(rowsv, abi::stack_pointer(), rows_slot),
        abi::load_u64(colsv, abi::stack_pointer(), cols_slot),
        abi::multiply_registers(t, rowsv, colsv),
        abi::move_immediate(
            m,
            "Integer",
            &(2 * CELL_SIZE + OUTBUF_PER_CELL + POOL_BYTES_PER_CELL).to_string(),
        ),
        abi::multiply_registers(t, t, m),
        abi::add_immediate(t, t, HDR_SIZE + TRAILER_SLACK),
        abi::shift_right_immediate(wc, t, 3),
        abi::move_register(zp, gp),
        abi::move_immediate(zero, "Integer", "0"),
        abi::label(&zloop),
        abi::compare_immediate(wc, "0"),
        abi::branch_eq(&zdone),
        abi::store_u64(zero, zp, 0),
        abi::add_immediate(zp, zp, 8),
        abi::subtract_immediate(wc, wc, 1),
        abi::branch(&zloop),
        abi::label(&zdone),
        // Header: rows, cols, cursor (0,0 already zeroed), dirty = 1.
        abi::store_u64(rowsv, gp, H_ROWS),
        abi::store_u64(colsv, gp, H_COLS),
        abi::move_immediate(one, "Integer", "1"),
        abi::store_u64(one, gp, H_DIRTY),
        abi::store_u64(
            gp,
            ARENA_STATE_REGISTER,
            term_state_offset + TERM_STATE_GRID_OFFSET,
        ),
    ]);
    Ok(())
}

/// Free the console grid block on `term::off` (and shutdown), returning it to the
/// arena and zeroing slot 48. A no-op when the slot is null.
pub(crate) fn emit_grid_free(
    symbol: &str,
    term_state_offset: usize,
    instrs: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    vregs: &mut Vregs,
) {
    let gp: Reg = vregs.next().into();
    let size_regs = GridSizeRegs::next(vregs);
    let zero: Reg = vregs.next().into();
    let skip = format!("{symbol}_gf_skip");
    instrs.extend([
        abi::load_u64(
            gp,
            ARENA_STATE_REGISTER,
            term_state_offset + TERM_STATE_GRID_OFFSET,
        ),
        abi::compare_immediate(gp, "0"),
        abi::branch_eq(&skip),
    ]);
    emit_grid_block_free(symbol, gp, &size_regs, instrs, relocations);
    instrs.extend([
        abi::move_immediate(zero, "Integer", "0"),
        abi::store_u64(
            zero,
            ARENA_STATE_REGISTER,
            term_state_offset + TERM_STATE_GRID_OFFSET,
        ),
        abi::label(&skip),
    ]);
}

/// bug-967: free the grid block a re-entered `term::on` replaced. `term::on`
/// parks the slot-48 pointer at `previous_slot` (an sp local) before it allocates
/// the new grid, and calls this after the new block is in slot 48. A no-op when
/// TUI mode was off (the parked pointer is null). Slot 48 is not touched: it
/// already names the new block.
pub(crate) fn emit_grid_free_previous(
    symbol: &str,
    previous_slot: usize,
    instrs: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    vregs: &mut Vregs,
) {
    let gp: Reg = vregs.next().into();
    let size_regs = GridSizeRegs::next(vregs);
    let skip = format!("{symbol}_gf_previous_skip");
    instrs.extend([
        abi::load_u64(gp, abi::stack_pointer(), previous_slot),
        abi::compare_immediate(gp, "0"),
        abi::branch_eq(&skip),
    ]);
    emit_grid_block_free(symbol, gp, &size_regs, instrs, relocations);
    instrs.push(abi::label(&skip));
}

/// The scratch registers [`emit_grid_block_free`] sizes a block with.
struct GridSizeRegs {
    rows: Reg,
    cols: Reg,
    total: Reg,
    per_cell: Reg,
}

impl GridSizeRegs {
    fn next(vregs: &mut Vregs) -> Self {
        Self {
            rows: vregs.next().into(),
            cols: vregs.next().into(),
            total: vregs.next().into(),
            per_cell: vregs.next().into(),
        }
    }
}

/// Return the non-null grid block `gp` to the arena, sized from its own header.
fn emit_grid_block_free(
    symbol: &str,
    gp: Reg,
    regs: &GridSizeRegs,
    instrs: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let GridSizeRegs {
        rows: rowsv,
        cols: colsv,
        total: t,
        per_cell: m,
    } = *regs;
    instrs.extend([
        abi::load_u64(rowsv, gp, H_ROWS),
        abi::load_u64(colsv, gp, H_COLS),
        abi::multiply_registers(t, rowsv, colsv),
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
}

#[cfg(test)]
mod tests {
    use super::*;

    /// bug-313: `OUTBUF_PER_CELL` must cover the worst-case escape run a single
    /// changed cell can emit, for the widest coordinate the geometry admits.
    ///
    /// It was 64 — the exact worst case for a 3-digit coordinate and short of it
    /// for anything wider. `rows`/`cols` come from raw `TIOCGWINSZ` as u16 with no
    /// clamp, so a coordinate can be 5 digits, and past 3 digits each changed cell
    /// overran its share until the aggregate excess passed `TRAILER_SLACK` and the
    /// present loop wrote outside the arena grid block.
    ///
    /// This recomputes the budget from the sequences the module emits rather than
    /// restating a number, so shrinking it or adding a longer escape fails here
    /// instead of silently overflowing a customer's terminal.
    #[test]
    fn outbuf_per_cell_covers_the_worst_case_escape_run() {
        /// Bytes a single changed cell can emit at a `digits`-wide coordinate with
        /// a glyph payload of `glyph` bytes.
        fn worst_case(digits: usize, glyph: usize) -> usize {
            let cup = 2 + digits + 1 + digits + 1; // ESC [ row ; col H
            let reset = "\x1b[0m".len();
            let bold = "\x1b[1m".len();
            let underline = "\x1b[4m".len();
            // ESC [38;2; r ; g ; b m — three components of up to three digits.
            let colour = "\x1b[38;2;".len() + 3 + 1 + 3 + 1 + 3 + "m".len();
            cup + reset + bold + underline + colour * 2 + glyph
        }

        // The historical bug-313 budget was exactly the 3-digit worst case for a
        // single 4-byte scalar, which is why the shortfall only appeared on large
        // terminals.
        assert_eq!(
            worst_case(3, 4),
            64,
            "the 3-digit / 4-byte-glyph worst case is what 64 encoded"
        );

        // u16 geometry means a coordinate is at most 65535 — five digits. plan-70-B
        // Phase 2: a cell can emit a pooled cluster of up to POOL_BYTES_PER_CELL
        // bytes, so that is the glyph payload the budget must now cover.
        let widest = u16::MAX.to_string().len();
        assert_eq!(widest, 5);
        assert!(
            OUTBUF_PER_CELL >= worst_case(widest, POOL_BYTES_PER_CELL),
            "OUTBUF_PER_CELL ({OUTBUF_PER_CELL}) must cover the {widest}-digit worst \
             case with a {POOL_BYTES_PER_CELL}-byte pooled glyph ({})",
            worst_case(widest, POOL_BYTES_PER_CELL)
        );
    }

    /// bug-410: the `term::sync` present-write loop copies each emitted cell
    /// back->front *before* the `write`, so a signal delivered mid-`write` (MFBASIC
    /// handlers are not `SA_RESTART`) must re-issue the write — else `front == back`
    /// claims a frame that only partially reached the terminal and the corruption
    /// is permanent. The loop used to give up on any negative return (`b.le
    /// write_done`), leaving the dead `write_retry` label with no incoming branch.
    ///
    /// This drives the present lowering with the errno accessor available (the
    /// libc-`write` convention, as on macOS / aarch64 / riscv64) and asserts the
    /// loop classifies EINTR and branches back to its own top to retry, exactly the
    /// tail every sibling write loop emits via `emit_transfer_loop_tail`.
    #[test]
    fn present_write_loop_retries_eintr() {
        let symbol = "test_term_sync";
        // The errno accessor being importable is what lets the libc-write backends
        // classify EINTR at all (bug-410's import half); supply it so the retry
        // path is exercised rather than the degenerate give-up.
        let mut platform_imports = HashMap::new();
        platform_imports.insert("__errno_location".to_string(), "libc".to_string());
        let mut instrs = Vec::new();
        let mut relocations = Vec::new();
        emit_grid_present(
            symbol,
            0,
            56, // SCRATCH_END; only baked into offsets, never executed here.
            "0",
            &crate::codegen::engine::tests::TestPlatform,
            &platform_imports,
            &mut instrs,
            &mut relocations,
            &mut Vregs::new(),
        )
        .expect("present lowers");

        let write_loop = format!("{symbol}_present_write_loop");
        // The EINTR retry edge: `cmp <ret>, EINTR` immediately followed by
        // `b.eq write_loop`. Before the fix no compare against EINTR and no
        // conditional branch back to the loop top existed — the negative return
        // fell straight through to `write_done`.
        let retries_eintr = instrs.windows(2).any(|w| {
            w[0].op.mnemonic() == "cmp_imm"
                && w[0].get("rhs").as_deref() == Some(EINTR_ERRNO)
                && w[1].op.mnemonic() == "b.eq"
                && w[1].get("target").as_deref() == Some(write_loop.as_str())
        });
        assert!(
            retries_eintr,
            "term::sync present-write loop must re-issue the write on EINTR \
             (bug-410): expected `cmp <ret>, {EINTR_ERRNO}; b.eq {write_loop}`"
        );
    }
}
