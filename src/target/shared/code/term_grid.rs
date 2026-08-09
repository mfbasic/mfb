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

use std::collections::HashMap;

use super::*;
use crate::target::shared::abi;

/// Term-state slot holding the console grid header-block base pointer (0 = off).
pub(super) const TERM_STATE_GRID_OFFSET: usize = 48;

// Header field offsets.
const H_ROWS: usize = 0;
const H_COLS: usize = 8;
const H_CUR_ROW: usize = 16;
const H_CUR_COL: usize = 24;
const H_DIRTY: usize = 32;
// Header size / cell field offsets are shared with `term.rs`'s drawing helpers
// (drawHLine/drawVLine stamp glyphs straight into the same back buffer), so they
// are `pub(super)` to keep a single source of truth for the layout.
pub(super) const HDR_SIZE: usize = 40;

// Cell layout (16 bytes).
pub(super) const CELL_SIZE: usize = 16;
pub(super) const C_GLYPH: usize = 0;
pub(super) const C_FG: usize = 4;
pub(super) const C_BG: usize = 8;
pub(super) const C_BOLD: usize = 12;
pub(super) const C_UN: usize = 13;
// plan-70-B: display width (0/1/2) of the cluster this cell holds, computed at
// write time from A's charwidth table and read by the presenter so a wide glyph
// advances the terminal cursor by 2 columns. Lives in a previously-unused pad
// byte, so `CELL_SIZE` is unchanged.
pub(super) const C_WIDTH: usize = 14;
// A wide cluster occupies its primary cell (`width` 2) followed by this
// **wide-trailing sentinel** in the next column: a reserved glyph value the
// presenter and draw helpers skip (it emits nothing) but that keeps the second
// column occupied for cursor math and diffing. 0xFFFF_FFFF is not a valid packed
// UTF-8 scalar (a lead byte 0xFF never appears), so it can never collide with a
// real glyph.
pub(super) const WIDE_TRAIL: &str = "4294967295";

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
pub(super) const OUTBUF_PER_CELL: usize = 136;

/// plan-70-B Phase 2: bytes reserved per cell for a pooled (multi-scalar) grapheme
/// cluster — a combining sequence, ZWJ emoji family, or regional-indicator flag
/// whose UTF-8 exceeds the 4 bytes that pack inline into the cell glyph word. Each
/// cell owns a dedicated slot at `pool_base + cell_index * POOL_BYTES_PER_CELL`, so
/// the pool needs no bump pointer, no reset, and no compaction (the cell's slot is
/// overwritten whenever the cell is): the lifecycle problem the umbrella flags is
/// sidestepped by fixed per-cell slots. 64 covers a base plus a long run of
/// combining marks / a 4-emoji ZWJ family (25 bytes); a cluster longer than this
/// stores as many whole leading scalars as fit (graceful cap).
pub(super) const POOL_BYTES_PER_CELL: usize = 64;

/// Glyph-word tag for a **pooled** cell: top byte `0xC0`, low 24 bits the cluster
/// byte length (its bytes live in the per-cell pool slot). The high bit ALONE is
/// NOT free — every 4-byte UTF-8 scalar packs its 4th (continuation) byte, always
/// `0x80..=0xBF`, into bits 24-31, so an inline astral glyph (e.g. `😀` =
/// `0x80989FF0`) has bit 31 set. But an inline glyph's top byte is only ever `0x00`
/// (1-3 byte scalars) or `0x80..=0xBF` (4-byte) — never `0xC0..=0xFE` — so `0xC0`
/// is a collision-free tag. `WIDE_TRAIL` (top byte `0xFF`) is distinct and is
/// handled before the pooled check. Length <= `POOL_BYTES_PER_CELL` fits the low
/// 24 bits with room to spare.
pub(super) const GLYPH_POOLED_TAG: i64 = 0xC000_0000;
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

/// Emit an append of `bytes` (a fixed ASCII/escape run) into the buffer at
/// cursor `buf`, advancing `buf` by `bytes.len()`. `scratch` is a throwaway vreg.
fn append_const(buf: &str, bytes: &[u8], scratch: &str, instrs: &mut Vec<CodeInstruction>) {
    for byte in bytes {
        instrs.push(abi::move_immediate(scratch, "Integer", &byte.to_string()));
        instrs.push(abi::store_u8(scratch, buf, 0));
        instrs.push(abi::add_immediate(buf, buf, 1));
    }
}

/// Append the unsigned decimal text of `val` into `buf`, advancing `buf`. Digits
/// are generated least-significant-first into the sp-relative scratch region
/// `[tmp_start, tmp_end)` (no call happens, so the region is stable), then copied
/// forward into the buffer. `tag` makes the loop labels unique.
fn append_decimal(
    buf: &str,
    val: &str,
    tmp_end: usize,
    tag: &str,
    instrs: &mut Vec<CodeInstruction>,
) {
    let q = "%v240";
    let ten = "%v241";
    let tp = "%v242";
    let d = "%v243";
    let r = "%v244";
    let e = "%v245";
    let c = "%v246";
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
    buf: &str,
    packed: &str,
    tmp_end: usize,
    tag: &str,
    instrs: &mut Vec<CodeInstruction>,
) {
    let ch = "%v247";
    let m = "%v248";
    instrs.push(abi::move_immediate(m, "Integer", "255"));
    instrs.push(abi::and_registers(ch, packed, m));
    append_decimal(buf, ch, tmp_end, &format!("{tag}_r"), instrs);
    append_const(buf, b";", ch, instrs);
    instrs.push(abi::shift_right_immediate(ch, packed, 8));
    instrs.push(abi::move_immediate(m, "Integer", "255"));
    instrs.push(abi::and_registers(ch, ch, m));
    append_decimal(buf, ch, tmp_end, &format!("{tag}_g"), instrs);
    append_const(buf, b";", ch, instrs);
    instrs.push(abi::shift_right_immediate(ch, packed, 16));
    instrs.push(abi::move_immediate(m, "Integer", "255"));
    instrs.push(abi::and_registers(ch, ch, m));
    append_decimal(buf, ch, tmp_end, &format!("{tag}_b"), instrs);
}

/// Append the glyph's raw UTF-8 bytes (packed little-endian in `glyph`), or a
/// single space when the cell is blank (glyph 0). Stops at the first zero byte.
fn append_glyph(buf: &str, glyph: &str, tag: &str, instrs: &mut Vec<CodeInstruction>) {
    let b = "%v249";
    let sp = "%v250";
    let blank = format!("{tag}_gblank");
    let done = format!("{tag}_gdone");
    let m = "%v251";
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

/// Scroll the back buffer up one row: shift rows 1..rows into 0..rows-1 and blank
/// the last row. `back` is the cell base, `rows`/`cols` the grid dims. No calls.
///
/// plan-70-B Phase 2: the per-cell EGC pool slots are position-indexed, so a
/// pooled cluster that scrolls would otherwise render the destination cell's stale
/// slot. This shifts the pool region (which follows the cells, at
/// `back + ncells*(2*CELL+OUTBUF)`) in lockstep and blanks the last row's slots.
/// `POOL_BYTES_PER_CELL` is a multiple of 8 so the shift stays word-aligned.
fn emit_scroll_back(
    back: &str,
    rows: &str,
    cols: &str,
    tag: &str,
    instrs: &mut Vec<CodeInstruction>,
) {
    let ncells = "%v320";
    let moved = "%v321";
    let src = "%v322";
    let dst = "%v323";
    let word = "%v324";
    let cnt = "%v325";
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
pub(super) fn emit_grid_alloc(
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
) -> Result<(), String> {
    let rowsv = "%v400";
    let colsv = "%v401";
    let t = "%v402";
    let m = "%v403";
    let gp = "%v404";
    let wc = "%v405";
    let zp = "%v406";
    let zero = "%v407";
    let one = "%v408";
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
pub(super) fn emit_grid_free(
    symbol: &str,
    term_state_offset: usize,
    instrs: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let gp = "%v450";
    let rowsv = "%v451";
    let colsv = "%v452";
    let t = "%v453";
    let m = "%v454";
    let zero = "%v455";
    let skip = format!("{symbol}_gf_skip");
    instrs.extend([
        abi::load_u64(
            gp,
            ARENA_STATE_REGISTER,
            term_state_offset + TERM_STATE_GRID_OFFSET,
        ),
        abi::compare_immediate(gp, "0"),
        abi::branch_eq(&skip),
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

/// Grid writer, inlined into the console `io::write` helper's TUI-active branch.
/// Reads the String object at `strobj` (`[len:u64 | bytes…]`), the current
/// attributes from the term-state global, and the header cursor, then stamps each
/// glyph into the back buffer honouring `\n`/`\r`, wrap, and scroll — never
/// emitting to the terminal. When `append_newline` is set (io::print) a trailing
/// newline advances the cursor too.
pub(super) fn emit_grid_write(
    symbol: &str,
    term_state_offset: usize,
    strobj: &str,
    append_newline: bool,
    instrs: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let gp = "%v300";
    let back = "%v301";
    let rows = "%v302";
    let cols = "%v303";
    let row = "%v304";
    let col = "%v305";
    let fg = "%v306";
    let bg = "%v307";
    let bold = "%v308";
    let un = "%v309";
    let ptr = "%v310";
    let rem = "%v311";
    let b0 = "%v312";
    let len = "%v313";
    let glyph = "%v314";
    let t = "%v315";
    let idx = "%v316";
    let cell = "%v317";
    // plan-70-B width path: codepoint decode + charwidth lookup + wide handling.
    let cp = "%v318";
    let width = "%v319";
    let sa = "%v320";
    let sb = "%v321";
    let trail = "%v322";
    // plan-70-B Phase 2 cluster walk + EGC pool.
    let prop = "%v323";
    let state_bc = "%v324";
    let state_icb = "%v325";
    let clen = "%v326"; // total cluster byte length
    let pptr = "%v327"; // peek: pointer at ptr + clen
    let pb0 = "%v328"; // peek: lead byte
    let plen = "%v329"; // peek: scalar byte length
    let pcp = "%v330"; // peek: codepoint
    let pprop = "%v331"; // peek: property ptr
    let pbc = "%v332"; // peek: boundclass
    let picb = "%v333"; // peek: indic-conjunct-break
    let pwidth = "%v334"; // peek: charwidth
    let sc = "%v335"; // scratch
    let sd = "%v336"; // scratch
    let sd2 = "%v337"; // scratch byte
    let cellidx = "%v338"; // linear cell index (row*cols+col) for the pool slot
    let ncells = "%v339";
    let poolbase = "%v340";
    let poolslot = "%v341";
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
    super::private::unicode::emit_utf8_codepoint_by_len(
        &format!("{symbol}_gwd"),
        ptr,
        len,
        b0,
        cp,
        sa,
        sb,
        instrs,
    );
    super::private::unicode::emit_unicode_property_ptr_free(
        symbol,
        &format!("{symbol}_gwb"),
        cp,
        prop,
        sa,
        instrs,
        relocations,
    );
    super::private::unicode::emit_read_boundclass_icb_charwidth_free(
        prop, state_bc, state_icb, width, sa, instrs,
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
    super::private::unicode::emit_utf8_codepoint_by_len(
        &format!("{symbol}_gwpd"),
        pptr,
        plen,
        pb0,
        pcp,
        sa,
        sb,
        instrs,
    );
    super::private::unicode::emit_unicode_property_ptr_free(
        symbol,
        &format!("{symbol}_gwpl"),
        pcp,
        pprop,
        sa,
        instrs,
        relocations,
    );
    super::private::unicode::emit_read_boundclass_icb_charwidth_free(
        pprop, pbc, picb, pwidth, sa, instrs,
    );
    super::private::unicode::emit_grapheme_break_branch_free(
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
    super::private::unicode::emit_grapheme_state_update_free(
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
    emit_scroll_back(back, rows, cols, &format!("{symbol}_gwp"), instrs);
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
    emit_scroll_back(back, rows, cols, &format!("{symbol}_gwn"), instrs);
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
        emit_scroll_back(back, rows, cols, &format!("{symbol}_gwt"), instrs);
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

/// Detect a terminal resize at `term::sync` entry and reflow the grid (plan-35-C
/// Phase 2). Re-reads the terminal size (TIOCGWINSZ); if it differs from the
/// header dims, allocates a new block, copies the top-left overlap from the old
/// back buffer (so content is preserved, matching the app backends), clamps the
/// cursor, sets `dirty` for a full repaint, publishes the new base pointer in
/// slot 48, and frees the old block. A no-op when the ioctl fails (e.g. stdout is
/// not a tty), the size is unchanged, or a new allocation cannot be obtained.
/// Uses sp locals `[8,56)` for the winsize struct + parked dims across the
/// arena calls — disjoint in time from the present's decimal scratch (`[32,56)`).
pub(super) fn emit_grid_resize(
    symbol: &str,
    term_state_offset: usize,
    request: &str,
    platform: &dyn CodegenPlatform,
    platform_imports: &HashMap<String, String>,
    instrs: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    // sp scratch layout for the resize (temporally before the present loop).
    const WINSZ: usize = 8;
    const S_NEWR: usize = 16;
    const S_NEWC: usize = 24;
    const S_OLDR: usize = 32;
    const S_OLDC: usize = 40;
    const S_OLDGP: usize = 48;
    let sp = abi::stack_pointer();
    let newr = "%v500";
    let newc = "%v501";
    let oldr = "%v502";
    let oldc = "%v503";
    let gp = "%v504";
    let ng = "%v505";
    let t = "%v506";
    let m = "%v507";
    let wc = "%v508";
    let zp = "%v509";
    let zero = "%v510";
    let minr = "%v511";
    let minc = "%v512";
    let rr = "%v513";
    let ob = "%v514";
    let nb = "%v515";
    let cc = "%v516";
    let sptr = "%v517";
    let dptr = "%v518";
    let word = "%v519";
    // plan-70-B Phase 2: pool-slot copy across the reflow.
    let opb = "%v520";
    let npb = "%v521";
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

#[allow(clippy::too_many_arguments)]
/// Console present (plan-35-C): diff the back buffer against the front buffer and
/// emit only the changed cells (minimal CUP + coalesced SGR + glyphs) as one
/// batched `write(2)` into fd 1, then copy back→front and restore the cursor.
/// A set `dirty` flag forces a full repaint (first present after on/resize).
/// Inlined into the `term::sync` helper; `term::off` reaches it via `bl`.
pub(super) fn emit_grid_present(
    symbol: &str,
    term_state_offset: usize,
    tmp_end: usize,
    request: &str,
    platform: &dyn CodegenPlatform,
    platform_imports: &HashMap<String, String>,
    instrs: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    let gp = "%v200";
    let rows = "%v201";
    let cols = "%v202";
    let dirty = "%v203";
    let ncells = "%v204";
    let back = "%v205";
    let front = "%v206";
    let outbuf = "%v207";
    let buf = "%v208";
    let idx = "%v209";
    let row = "%v210";
    let col = "%v211";
    let off = "%v212";
    let bc = "%v213";
    let fc = "%v214";
    let last_valid = "%v215";
    let last_row = "%v216";
    let last_col = "%v217";
    let last_fg = "%v218";
    let last_bg = "%v219";
    let last_bold = "%v220";
    let last_un = "%v221";
    let cfg = "%v222";
    let cbg = "%v223";
    let cbold = "%v224";
    let cun = "%v225";
    let glyph = "%v226";
    let a0 = "%v227";
    let b0 = "%v228";
    let tmp = "%v229";
    let rowp1 = "%v230";
    let colp1 = "%v231";
    // bug-314 H3: the present-write loop's cursor / remaining / result.
    let wcur = "%v232";
    let wrem = "%v233";
    let wres = "%v234";
    // plan-70-B: this cell's stored display width, for the cursor-advance model.
    let cwidth = "%v235";
    // plan-70-B Phase 2: pooled-cluster emission.
    let pe_len = "%v236";
    let pe_base = "%v237";
    let pe_slot = "%v238";
    let pe_i = "%v239";
    let pe_byte = "%v240";
    let pe_sc = "%v241";
    let pe_dst = "%v242";

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
    append_decimal(buf, rowp1, tmp_end, &format!("{symbol}_prcur"), instrs);
    append_const(buf, b";", tmp, instrs);
    instrs.push(abi::add_immediate(colp1, col, 1));
    append_decimal(buf, colp1, tmp_end, &format!("{symbol}_prcuc"), instrs);
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
    append_rgb(buf, cfg, tmp_end, &format!("{symbol}_prfg"), instrs);
    append_const(buf, b"m", tmp, instrs);
    append_const(buf, b"\x1b[48;2;", tmp, instrs);
    append_rgb(buf, cbg, tmp_end, &format!("{symbol}_prbg"), instrs);
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
    append_glyph(buf, glyph, &format!("{symbol}_prg"), instrs);
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
    append_decimal(buf, rowp1, tmp_end, &format!("{symbol}_prrr"), instrs);
    append_const(buf, b";", tmp, instrs);
    instrs.push(abi::load_u64(tmp, gp, H_CUR_COL));
    instrs.push(abi::add_immediate(colp1, tmp, 1));
    append_decimal(buf, colp1, tmp_end, &format!("{symbol}_prrc"), instrs);
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
            &crate::target::shared::code::test_support::TestPlatform,
            &platform_imports,
            &mut instrs,
            &mut relocations,
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
