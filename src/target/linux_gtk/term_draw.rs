//! Linux GTK4 app-mode Cairo TermView draw path: draw/scroll/init/write
//! emitters, cell-geometry helpers, and idle show/hide/redraw (plan-11 split).

use super::*;

/// Decode the UTF-8 code point at `ptr` (= `&text[index]`), whose lead byte is
/// already in `glyph` (bug-203).
///
/// On return `glyph` holds the code point's bytes packed little-endian — lead
/// byte in the low byte, zero-padded — which is exactly the layout a `str_u32`
/// into a 5-byte buffer needs to hand `cairo_show_text` one NUL-terminated
/// glyph. `len` holds its byte length (1-4), which the caller uses to advance.
///
/// The lead byte gives the length; a byte that is not a lead byte (a stray
/// continuation, `10xxxxxx`) decodes as length 1, and a sequence running past
/// `count` is clamped to 1. Malformed input therefore still advances one byte at
/// a time and renders per-byte instead of hanging or reading out of bounds.
///
/// No calls, so it uses caller-saved scratch (x11/x12) only; `len` must be a
/// register that survives the caller's own calls.
fn emit_utf8_decode_at(asm: &mut Asm, glyph: &str, ptr: &str, len: &str, index: &str, count: &str) {
    // Length from the lead byte: <0xC0 -> 1, <0xE0 -> 2, <0xF0 -> 3, else 4.
    asm.push(abi::move_immediate(len, "Integer", "1"));
    asm.push(abi::compare_immediate(glyph, "192"));
    asm.push(abi::branch_lt("u8_len_done"));
    asm.push(abi::move_immediate(len, "Integer", "2"));
    asm.push(abi::compare_immediate(glyph, "224"));
    asm.push(abi::branch_lt("u8_len_done"));
    asm.push(abi::move_immediate(len, "Integer", "3"));
    asm.push(abi::compare_immediate(glyph, "240"));
    asm.push(abi::branch_lt("u8_len_done"));
    asm.push(abi::move_immediate(len, "Integer", "4"));
    asm.push(abi::label("u8_len_done"));
    // Clamp to the bytes that remain, so a truncated tail cannot read past the
    // text: consume one byte instead.
    asm.push(abi::subtract_registers("x11", count, index));
    asm.push(abi::compare_registers(len, "x11"));
    asm.push(abi::branch_ls("u8_len_ok"));
    asm.push(abi::move_immediate(len, "Integer", "1"));
    asm.push(abi::label("u8_len_ok"));
    // Pack the continuation bytes with fixed shifts (len is 1-4, so unrolling
    // avoids needing a variable shift).
    for (byte, shift) in [(1usize, 8u8), (2, 16), (3, 24)] {
        asm.push(abi::compare_immediate(len, &(byte + 1).to_string()));
        asm.push(abi::branch_lt("u8_pack_done"));
        asm.push(abi::load_u8("x12", ptr, byte));
        asm.push(abi::shift_left_immediate("x12", "x12", shift));
        asm.push(abi::or_registers(glyph, glyph, "x12"));
    }
    asm.push(abi::label("u8_pack_done"));
}

/// Emit `cairo_set_source_rgb(cr, r/255, g/255, b/255)` from a packed RGB value in
/// `packed` (low 24 bits). Clobbers x0/x9-x13 and d0-d3.
fn emit_cairo_color(asm: &mut Asm, cr: &str, packed: &str) {
    asm.push(abi::move_register("x0", cr));
    asm.push(abi::move_immediate("x9", "Integer", "255")); // mask + divisor
    asm.push(abi::and_registers("x10", packed, "x9")); // r = packed & 0xFF
    asm.push(abi::shift_right_immediate("x11", packed, 8)); // g
    asm.push(abi::and_registers("x11", "x11", "x9"));
    asm.push(abi::shift_right_immediate("x12", packed, 16)); // b
    asm.push(abi::and_registers("x12", "x12", "x9"));
    asm.push(abi::signed_convert_to_float_d("d3", "x9")); // 255.0
    asm.push(abi::signed_convert_to_float_d("d0", "x10"));
    asm.push(abi::float_divide_d("d0", "d0", "d3"));
    asm.push(abi::signed_convert_to_float_d("d1", "x11"));
    asm.push(abi::float_divide_d("d1", "d1", "d3"));
    asm.push(abi::signed_convert_to_float_d("d2", "x12"));
    asm.push(abi::float_divide_d("d2", "d2", "d3"));
    asm.call_external("cairo_set_source_rgb");
}

/// `void term_draw(GtkDrawingArea *area, cairo_t *cr /*x1*/, int w, int h, gpointer)`
/// — the drawing-area render callback (main thread). Paints black, then renders each
/// non-space cell: an optional background rect, then the glyph in its fg color and
/// weight (bold). Monospace; cursor rendering is still deferred.
pub(super) fn emit_term_draw_helper() -> Result<CodeFunction, String> {
    let mut asm = Asm::new(TERM_DRAW_SYMBOL);
    // lr@0, x19(cr)@8, x20(row)@16, x21(col)@24, x22(lastBold)@32, x23(charsBase)@40,
    // x24(fgBase)@48, x25(bgBase)@56, x26(cols)@64, x27(rows)@72, fg@80, bg@88,
    // charbuf@96 (5B: up to 4 UTF-8 bytes + NUL).
    let frame = 112;
    let (off_fg, off_bg, off_buf) = (80usize, 88usize, 96usize);
    let saved = [
        ("x19", 8),
        ("x20", 16),
        ("x21", 24),
        ("x22", 32),
        ("x23", 40),
        ("x24", 48),
        ("x25", 56),
        ("x26", 64),
        ("x27", 72),
    ];
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(frame));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        0,
    ));
    for (reg, off) in saved {
        asm.push(abi::store_u64(reg, abi::stack_pointer(), off));
    }
    asm.push(abi::move_register("x19", "x1")); // cr

    // Black background.
    asm.push(abi::move_register("x0", "x19"));
    asm.push(abi::move_immediate("x9", "Integer", "0"));
    asm.push(abi::float_move_d_from_x("d0", "x9"));
    asm.push(abi::float_move_d_from_x("d1", "x9"));
    asm.push(abi::float_move_d_from_x("d2", "x9"));
    asm.call_external("cairo_set_source_rgb");
    asm.push(abi::move_register("x0", "x19"));
    asm.call_external("cairo_paint");
    // Normal-weight monospace at TERM_FONT_SIZE; lastBold tracks the selected weight.
    emit_term_select_font(&mut asm, "x19", false);
    asm.push(abi::move_immediate("x22", "Integer", "0"));
    // Render the draw-owned SNAPSHOT arrays (plan-35-E): a present copies the live
    // worker arrays here on the main loop before queue_draw, so this callback never
    // reads a half-written frame. The active extent (cols/rows) + cursor are read
    // live — a torn single u64 there is benign and self-corrects next present.
    asm.state_array("x23", ST_TERM_SNAP_CHARS);
    asm.state_array("x24", ST_TERM_SNAP_FG);
    asm.state_array("x25", ST_TERM_SNAP_BG);
    asm.load_state("x26", ST_TERM_COLS);
    asm.load_state("x27", ST_TERM_ROWS);

    asm.push(abi::move_immediate("x20", "Integer", "0")); // row
    asm.push(abi::label("d_row"));
    asm.push(abi::compare_registers("x20", "x27")); // row < rows?
    asm.push(abi::branch_ge("d_done"));
    asm.push(abi::move_immediate("x21", "Integer", "0")); // col
    asm.push(abi::label("d_col"));
    asm.push(abi::compare_registers("x21", "x26")); // col < cols?
    asm.push(abi::branch_ge("d_row_next"));
    // idx = row*MAX_COLS + col (fixed backing stride)
    asm.push(abi::move_immediate(
        "x9",
        "Integer",
        &TERM_MAX_COLS.to_string(),
    ));
    asm.push(abi::multiply_registers("x10", "x20", "x9"));
    asm.push(abi::add_registers("x10", "x10", "x21")); // idx
    asm.push(abi::shift_left_immediate("x11", "x10", 2)); // idx*4
                                                          // char -> charbuf; fg, bg -> stack (survive cairo calls). The cell holds one
                                                          // code point's UTF-8 bytes packed little-endian, so storing the u32 lays them
                                                          // out in order; the NUL after it terminates the 1-4 byte sequence for
                                                          // `cairo_show_text` (bug-203 — this used to store a single byte, which cut a
                                                          // multi-byte glyph into invalid fragments).
    asm.push(abi::add_registers("x12", "x23", "x11"));
    asm.push(abi::load_u32("x13", "x12", 0));
    asm.push(abi::store_u32("x13", abi::stack_pointer(), off_buf));
    asm.push(abi::move_immediate("x9", "Integer", "0"));
    asm.push(abi::store_u8("x9", abi::stack_pointer(), off_buf + 4));
    asm.push(abi::add_registers("x12", "x24", "x11"));
    asm.push(abi::load_u32("x13", "x12", 0));
    asm.push(abi::store_u64("x13", abi::stack_pointer(), off_fg));
    asm.push(abi::add_registers("x12", "x25", "x11"));
    asm.push(abi::load_u32("x13", "x12", 0));
    asm.push(abi::store_u64("x13", abi::stack_pointer(), off_bg));

    // Background rect when an explicit bg is set.
    asm.push(abi::load_u64("x14", abi::stack_pointer(), off_bg));
    asm.push(abi::move_immediate("x9", "Integer", &COLOR_SET.to_string()));
    asm.push(abi::and_registers("x9", "x14", "x9"));
    asm.push(abi::compare_immediate("x9", "0"));
    asm.push(abi::branch_eq("d_no_bg"));
    emit_cairo_color(&mut asm, "x19", "x14");
    asm.push(abi::move_register("x0", "x19")); // rectangle(cr, col*W, row*H, W, H)
    emit_cell_dim_to_d(&mut asm, "d0", "x21", ST_TERM_CELL_W);
    emit_cell_dim_to_d(&mut asm, "d1", "x20", ST_TERM_CELL_H);
    emit_cell_to_d(&mut asm, "d2", ST_TERM_CELL_W);
    emit_cell_to_d(&mut asm, "d3", ST_TERM_CELL_H);
    asm.call_external("cairo_rectangle");
    asm.push(abi::move_register("x0", "x19"));
    asm.call_external("cairo_fill");
    asm.push(abi::label("d_no_bg"));

    // Glyph (skip blanks). A never-written cell is 0 (the blanking memsets clear
    // whole u32 cells) and an explicitly written space is 32; both render
    // nothing. Compares the whole cell, so a multi-byte glyph whose lead byte
    // happens to be 0x20 could never be mistaken for a space.
    asm.push(abi::load_u32("x13", abi::stack_pointer(), off_buf));
    asm.push(abi::compare_immediate("x13", "0"));
    asm.push(abi::branch_eq("d_next"));
    asm.push(abi::compare_immediate("x13", "32"));
    asm.push(abi::branch_eq("d_next"));
    // Re-select font weight if bold changed.
    asm.push(abi::load_u64("x14", abi::stack_pointer(), off_fg));
    asm.push(abi::move_immediate("x9", "Integer", &BOLD_FLAG.to_string()));
    asm.push(abi::and_registers("x9", "x14", "x9")); // 0 or BOLD_FLAG
    asm.push(abi::compare_registers("x9", "x22"));
    asm.push(abi::branch_eq("d_bold_ok"));
    asm.push(abi::move_register("x22", "x9"));
    asm.push(abi::compare_immediate("x9", "0"));
    asm.push(abi::branch_eq("d_sel_normal"));
    emit_term_select_font(&mut asm, "x19", true);
    asm.push(abi::branch("d_bold_ok"));
    asm.push(abi::label("d_sel_normal"));
    emit_term_select_font(&mut asm, "x19", false);
    asm.push(abi::label("d_bold_ok"));
    // Foreground color: explicit or white.
    asm.push(abi::load_u64("x14", abi::stack_pointer(), off_fg));
    asm.push(abi::move_immediate("x9", "Integer", &COLOR_SET.to_string()));
    asm.push(abi::and_registers("x9", "x14", "x9"));
    asm.push(abi::compare_immediate("x9", "0"));
    asm.push(abi::branch_eq("d_fg_white"));
    emit_cairo_color(&mut asm, "x19", "x14");
    asm.push(abi::branch("d_fg_done"));
    asm.push(abi::label("d_fg_white"));
    asm.push(abi::move_register("x0", "x19"));
    asm.push(abi::move_immediate("x9", "Integer", "1"));
    asm.push(abi::signed_convert_to_float_d("d0", "x9"));
    asm.push(abi::signed_convert_to_float_d("d1", "x9"));
    asm.push(abi::signed_convert_to_float_d("d2", "x9"));
    asm.call_external("cairo_set_source_rgb");
    asm.push(abi::label("d_fg_done"));
    // move_to(col*cellW, (row+1)*cellH - 4); show_text(charbuf). Load cellH BEFORE
    // forming row+1 in x9 — load_state clobbers x9 as its address scratch.
    asm.push(abi::move_register("x0", "x19"));
    emit_cell_dim_to_d(&mut asm, "d0", "x21", ST_TERM_CELL_W);
    asm.load_state("x10", ST_TERM_CELL_H);
    asm.push(abi::add_immediate("x9", "x20", 1));
    asm.push(abi::multiply_registers("x9", "x9", "x10"));
    asm.push(abi::subtract_immediate("x9", "x9", 4));
    asm.push(abi::signed_convert_to_float_d("d1", "x9"));
    asm.call_external("cairo_move_to");
    asm.push(abi::move_register("x0", "x19"));
    asm.push(abi::add_immediate("x1", abi::stack_pointer(), off_buf));
    asm.call_external("cairo_show_text");
    // Underline: a 2px rect at the cell bottom in the (already-set) fg color.
    asm.push(abi::load_u64("x14", abi::stack_pointer(), off_fg));
    asm.push(abi::move_immediate(
        "x9",
        "Integer",
        &UNDERLINE_FLAG.to_string(),
    ));
    asm.push(abi::and_registers("x9", "x14", "x9"));
    asm.push(abi::compare_immediate("x9", "0"));
    asm.push(abi::branch_eq("d_next"));
    emit_term_cell_rect(&mut asm, "x19", "x21", "x20");

    asm.push(abi::label("d_next"));
    asm.push(abi::add_immediate("x21", "x21", 1));
    asm.push(abi::branch("d_col"));
    asm.push(abi::label("d_row_next"));
    asm.push(abi::add_immediate("x20", "x20", 1));
    asm.push(abi::branch("d_row"));

    asm.push(abi::label("d_done"));
    // Cursor caret: a 2px bar at the cursor cell bottom in white, if visible.
    asm.load_state("x9", ST_TERM_CURSOR_VISIBLE);
    asm.push(abi::compare_immediate("x9", "0"));
    asm.push(abi::branch_eq("d_no_cursor"));
    asm.push(abi::move_register("x0", "x19"));
    asm.push(abi::move_immediate("x9", "Integer", "1"));
    asm.push(abi::signed_convert_to_float_d("d0", "x9"));
    asm.push(abi::signed_convert_to_float_d("d1", "x9"));
    asm.push(abi::signed_convert_to_float_d("d2", "x9"));
    asm.call_external("cairo_set_source_rgb");
    asm.load_state("x20", ST_TERM_ROW);
    asm.load_state("x21", ST_TERM_COL);
    emit_term_cell_rect(&mut asm, "x19", "x21", "x20");
    asm.push(abi::label("d_no_cursor"));
    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    for (reg, off) in saved {
        asm.push(abi::load_u64(reg, abi::stack_pointer(), off));
    }
    asm.push(abi::add_stack(frame));
    asm.push(abi::return_());
    asm.finish(TERM_DRAW_SYMBOL, "Nothing")
}

/// cairo_select_font_face(cr, "monospace", NORMAL, weight) + set_font_size.
fn emit_term_select_font(asm: &mut Asm, cr: &str, bold: bool) {
    asm.push(abi::move_register("x0", cr));
    asm.local_address("x1", STR_MONOSPACE.0);
    asm.push(abi::move_immediate("x2", "Integer", "0")); // CAIRO_FONT_SLANT_NORMAL
    asm.push(abi::move_immediate(
        "x3",
        "Integer",
        if bold { "1" } else { "0" },
    ));
    asm.call_external("cairo_select_font_face");
    asm.push(abi::move_register("x0", cr));
    asm.push(abi::move_immediate("x9", "Integer", TERM_FONT_SIZE));
    asm.push(abi::signed_convert_to_float_d("d0", "x9"));
    asm.call_external("cairo_set_font_size");
}

/// `dst (d-reg) = index * cellSize` as a double, where the cell size (px) is read
/// from the runtime-state field `cell_off`. Clobbers x9.
fn emit_cell_dim_to_d(asm: &mut Asm, dst: &str, index: &str, cell_off: usize) {
    asm.load_state("x9", cell_off);
    asm.push(abi::multiply_registers("x9", index, "x9"));
    asm.push(abi::signed_convert_to_float_d(dst, "x9"));
}

/// `dst (d-reg) = the cell size (px) at runtime-state field `cell_off`.
fn emit_cell_to_d(asm: &mut Asm, dst: &str, cell_off: usize) {
    asm.load_state("x9", cell_off);
    asm.push(abi::signed_convert_to_float_d(dst, "x9"));
}

/// `dst (d-reg) = constant` as a double.
fn emit_const_to_d(asm: &mut Asm, dst: &str, value: usize) {
    asm.push(abi::move_immediate("x9", "Integer", &value.to_string()));
    asm.push(abi::signed_convert_to_float_d(dst, "x9"));
}

/// Fill a 2px-tall rect at the bottom of cell (col,row) in the current source color
/// (used for the underline run and the cursor caret).
fn emit_term_cell_rect(asm: &mut Asm, cr: &str, col: &str, row: &str) {
    asm.push(abi::move_register("x0", cr));
    emit_cell_dim_to_d(asm, "d0", col, ST_TERM_CELL_W); // x = col*cellW
                                                        // Load cellH before forming row+1 in x9 (load_state clobbers x9).
    asm.load_state("x10", ST_TERM_CELL_H);
    asm.push(abi::add_immediate("x9", row, 1)); // y = (row+1)*cellH - 2
    asm.push(abi::multiply_registers("x9", "x9", "x10"));
    asm.push(abi::subtract_immediate("x9", "x9", 2));
    asm.push(abi::signed_convert_to_float_d("d1", "x9"));
    emit_cell_to_d(asm, "d2", ST_TERM_CELL_W); // w
    emit_const_to_d(asm, "d3", 2); // h
    asm.call_external("cairo_rectangle");
    asm.push(abi::move_register("x0", cr));
    asm.call_external("cairo_fill");
}

/// `void _mfb_gtkapp_term_scroll(void)` — shift the grid up one row (chars/fg/bg)
/// and blank the last row. Worker-side data mutation (no GTK calls). Like
/// [`emit_term_write_helper`], this runs unsynchronized against the main-thread
/// draw callback: a concurrent redraw during the memmove/memset can paint a torn
/// row. Benign (fixed static buffers, no memory unsafety, corrected next frame);
/// the marshaling fix is deferred (bug-117.3).
pub(super) fn emit_term_scroll_helper() -> Result<CodeFunction, String> {
    let mut asm = Asm::new(TERM_SCROLL_SYMBOL);
    // lr@0, x19(cells = (rows-1)*MAX_COLS, the chars to move / last-row offset)@8.
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(16));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        0,
    ));
    asm.push(abi::store_u64("x19", abi::stack_pointer(), 8));
    asm.load_state("x19", ST_TERM_ROWS);
    asm.push(abi::subtract_immediate("x19", "x19", 1)); // rows-1
    asm.push(abi::move_immediate(
        "x9",
        "Integer",
        &TERM_MAX_COLS.to_string(),
    ));
    asm.push(abi::multiply_registers("x19", "x19", "x9")); // cells = (rows-1)*MAX_COLS
                                                           // memmove each array up one (fixed-stride) row: 4B per cell for all three
                                                           // (chars became u32 in bug-203, matching fg/bg).
    for (base, shift) in [(ST_TERM_CHARS, 2u8), (ST_TERM_FG, 2), (ST_TERM_BG, 2)] {
        asm.state_array("x0", base); // dst = row 0
        asm.state_array("x1", base + TERM_MAX_COLS * (1 << shift)); // src = row 1
        asm.push(abi::shift_left_immediate("x2", "x19", shift)); // cells * elemSize
        asm.call_external("memmove");
    }
    // Blank the last active row (offset = cells*4): all three arrays to 0. chars
    // clears to 0 rather than ' ' — `memset` writes whole bytes, so ' ' over u32
    // cells would pack FOUR spaces per cell; the draw skips 0 (bug-203).
    for base in [ST_TERM_CHARS, ST_TERM_FG, ST_TERM_BG] {
        asm.state_array("x0", base);
        asm.push(abi::shift_left_immediate("x9", "x19", 2)); // cells*4
        asm.push(abi::add_registers("x0", "x0", "x9"));
        asm.push(abi::move_immediate("x1", "Integer", "0"));
        asm.push(abi::move_immediate(
            "x2",
            "Integer",
            &(TERM_MAX_COLS * 4).to_string(),
        ));
        asm.call_external("memset");
    }
    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::load_u64("x19", abi::stack_pointer(), 8));
    asm.push(abi::add_stack(16));
    asm.push(abi::return_());
    asm.finish(TERM_SCROLL_SYMBOL, "Nothing")
}

/// `void _mfb_gtkapp_term_init(void)` — derive the grid geometry (main thread):
/// measure the monospace cell from Cairo font extents (via a throwaway 1x1 image
/// surface), then cols = floor(W/cellW), rows = floor(H/cellH) clamped to the
/// backing-store bounds, and blank the char grid. Mirrors the macOS term_init,
/// which sizes cols/rows from the font's advance + line height and the view frame.
pub(super) fn emit_term_init_helper() -> Result<CodeFunction, String> {
    let mut asm = Asm::new(TERM_INIT_SYMBOL);
    // lr@0, x19(cr)@8, x20(surf)@16, extents buffer@24 (48B, fits both font_extents
    // and the larger text_extents). cr/surf are callee-saved so they survive the
    // cairo calls.
    let frame = 80;
    let fe = 24usize;
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(frame));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        0,
    ));
    asm.push(abi::store_u64("x19", abi::stack_pointer(), 8));
    asm.push(abi::store_u64("x20", abi::stack_pointer(), 16));
    // surf = cairo_image_surface_create(CAIRO_FORMAT_ARGB32=0, 8, 8); cr = create(surf)
    asm.push(abi::move_immediate("x0", "Integer", "0"));
    asm.push(abi::move_immediate("x1", "Integer", "8"));
    asm.push(abi::move_immediate("x2", "Integer", "8"));
    asm.call_external("cairo_image_surface_create");
    asm.push(abi::move_register("x20", "x0")); // surf
    asm.push(abi::move_register("x0", "x20"));
    asm.call_external("cairo_create");
    asm.push(abi::move_register("x19", "x0")); // cr
                                               // select monospace at TERM_FONT_SIZE.
    emit_term_select_font(&mut asm, "x19", false);
    // cell_h = ceil(font_extents.height @ +16). font_extents_t: ascent,descent,
    // height,max_x_advance,max_y_advance.
    asm.push(abi::move_register("x0", "x19"));
    asm.push(abi::add_immediate("x1", abi::stack_pointer(), fe));
    asm.call_external("cairo_font_extents");
    asm.push(abi::load_u64("x9", abi::stack_pointer(), fe + 16));
    asm.push(abi::float_move_d_from_x("d0", "x9"));
    asm.push(abi::float_ceil_to_signed_x("x10", "d0"));
    emit_clamp_low(&mut asm, "x10", 1, "ch");
    asm.store_state("x10", ST_TERM_CELL_H);
    // cell_w = ceil(text_extents("M").x_advance @ +32). Using a real glyph's advance
    // (not font_extents.max_x_advance, which is the widest glyph in the whole font).
    // text_extents_t: x_bearing,y_bearing,width,height,x_advance,y_advance.
    asm.push(abi::move_register("x0", "x19"));
    asm.local_address("x1", STR_M.0);
    asm.push(abi::add_immediate("x2", abi::stack_pointer(), fe));
    asm.call_external("cairo_text_extents");
    asm.push(abi::load_u64("x9", abi::stack_pointer(), fe + 32));
    asm.push(abi::float_move_d_from_x("d0", "x9"));
    asm.push(abi::float_ceil_to_signed_x("x10", "d0"));
    emit_clamp_low(&mut asm, "x10", 1, "cw");
    asm.store_state("x10", ST_TERM_CELL_W);
    // cleanup
    asm.push(abi::move_register("x0", "x19"));
    asm.call_external("cairo_destroy");
    asm.push(abi::move_register("x0", "x20"));
    asm.call_external("cairo_surface_destroy");
    // cols = clamp(AREA_W / cell_w, 1, MAX_COLS); rows likewise.
    asm.load_state("x10", ST_TERM_CELL_W);
    asm.push(abi::move_immediate(
        "x9",
        "Integer",
        &TERM_AREA_W.to_string(),
    ));
    asm.push(abi::unsigned_divide_registers("x11", "x9", "x10"));
    emit_clamp_range(&mut asm, "x11", 1, TERM_MAX_COLS, "cols");
    asm.store_state("x11", ST_TERM_COLS);
    asm.load_state("x10", ST_TERM_CELL_H);
    asm.push(abi::move_immediate(
        "x9",
        "Integer",
        &TERM_AREA_H.to_string(),
    ));
    asm.push(abi::unsigned_divide_registers("x11", "x9", "x10"));
    emit_clamp_range(&mut asm, "x11", 1, TERM_MAX_ROWS, "rows");
    asm.store_state("x11", ST_TERM_ROWS);
    // Blank the whole char backing store (fg/bg stay 0 = defaults). Cells clear
    // to 0, not ' ' — see the scroll blank (bug-203).
    asm.state_array("x0", ST_TERM_CHARS);
    asm.push(abi::move_immediate("x1", "Integer", "0"));
    asm.push(abi::move_immediate(
        "x2",
        "Integer",
        &(TERM_MAX_COLS * TERM_MAX_ROWS * 4).to_string(),
    ));
    asm.call_external("memset");
    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::load_u64("x19", abi::stack_pointer(), 8));
    asm.push(abi::load_u64("x20", abi::stack_pointer(), 16));
    asm.push(abi::add_stack(frame));
    asm.push(abi::return_());
    asm.finish(TERM_INIT_SYMBOL, "Nothing")
}

/// Clamp `reg = max(reg, low)` (clobbers x9). `tag` makes the label unique.
fn emit_clamp_low(asm: &mut Asm, reg: &str, low: usize, tag: &str) {
    let label = format!("clamp_{tag}");
    asm.push(abi::compare_immediate(reg, &low.to_string()));
    asm.push(abi::branch_ge(&label));
    asm.push(abi::move_immediate(reg, "Integer", &low.to_string()));
    asm.push(abi::label(&label));
}

/// Clamp `reg` to `[low, high]`. `tag` makes the labels unique within a function.
fn emit_clamp_range(asm: &mut Asm, reg: &str, low: usize, high: usize, tag: &str) {
    let lo = format!("clo_{tag}");
    let hi = format!("chi_{tag}");
    asm.push(abi::compare_immediate(reg, &high.to_string()));
    asm.push(abi::branch_le(&hi));
    asm.push(abi::move_immediate(reg, "Integer", &high.to_string()));
    asm.push(abi::label(&hi));
    asm.push(abi::compare_immediate(reg, &low.to_string()));
    asm.push(abi::branch_ge(&lo));
    asm.push(abi::move_immediate(reg, "Integer", &low.to_string()));
    asm.push(abi::label(&lo));
}

/// Main-thread idle: swap the window child to the term:: surface and redraw it.
pub(super) fn emit_term_show_idle_helper() -> Result<CodeFunction, String> {
    let mut asm = Asm::new(TERM_SHOW_IDLE_SYMBOL);
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(16));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        0,
    ));
    asm.load_state("x0", ST_WINDOW);
    asm.load_state("x1", ST_TERM_AREA);
    asm.call_external("gtk_window_set_child");
    // Present the initial (cleared) grid: snapshot the live arrays before drawing so
    // the first frame after `term::on` matches the live grid (plan-35-E).
    emit_term_snapshot_copy(&mut asm);
    asm.load_state("x0", ST_TERM_AREA);
    asm.call_external("gtk_widget_queue_draw");
    asm.push(abi::move_immediate("x0", "Boolean", FALSE)); // G_SOURCE_REMOVE
    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::add_stack(16));
    asm.push(abi::return_());
    asm.finish(TERM_SHOW_IDLE_SYMBOL, "Boolean")
}

/// Main-thread idle: restore the transcript as the window child.
pub(super) fn emit_term_hide_idle_helper() -> Result<CodeFunction, String> {
    let mut asm = Asm::new(TERM_HIDE_IDLE_SYMBOL);
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(16));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        0,
    ));
    asm.load_state("x0", ST_WINDOW);
    asm.load_state("x1", ST_SCROLLED);
    asm.call_external("gtk_window_set_child");
    asm.push(abi::move_immediate("x0", "Boolean", FALSE));
    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::add_stack(16));
    asm.push(abi::return_());
    asm.finish(TERM_HIDE_IDLE_SYMBOL, "Boolean")
}

/// Copy the live worker-written grid arrays (chars/fg/bg) into the draw-owned
/// snapshot arrays with three `memcpy`s (plan-35-E). MUST run on the GTK main loop
/// (it is only ever reached from an idle callback). A raw byte copy preserves the
/// COLOR_SET/bold/underline bit-packing in the fg/bg words. Clobbers x0/x1/x2/x9.
fn emit_term_snapshot_copy(asm: &mut Asm) {
    // chars: 4 bytes/cell (one code point's UTF-8 bytes packed LE — bug-203).
    asm.state_array("x0", ST_TERM_SNAP_CHARS);
    asm.state_array("x1", ST_TERM_CHARS);
    asm.push(abi::move_immediate(
        "x2",
        "Integer",
        &(TERM_MAX_COLS * TERM_MAX_ROWS * 4).to_string(),
    ));
    asm.call_external("memcpy");
    // fg / bg: 4 bytes/cell (packed RGB | flags — copied verbatim).
    for (snap, live) in [(ST_TERM_SNAP_FG, ST_TERM_FG), (ST_TERM_SNAP_BG, ST_TERM_BG)] {
        asm.state_array("x0", snap);
        asm.state_array("x1", live);
        asm.push(abi::move_immediate(
            "x2",
            "Integer",
            &(TERM_MAX_COLS * TERM_MAX_ROWS * 4).to_string(),
        ));
        asm.call_external("memcpy");
    }
}

/// Main-thread idle: PRESENT the term:: surface (plan-35-E). Marshal a consistent
/// snapshot of the live grid on the main loop, then `queue_draw`. This is the single
/// coalesced present scheduled by `term::sync` / `io::flush` / `term::off` (and the
/// explicit terminal ops `clear` / cursor-visibility); the per-write redraw was
/// removed so a program that draws without a following present shows nothing new.
pub(super) fn emit_term_redraw_idle_helper() -> Result<CodeFunction, String> {
    let mut asm = Asm::new(TERM_REDRAW_IDLE_SYMBOL);
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(16));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        0,
    ));
    emit_term_snapshot_copy(&mut asm);
    asm.load_state("x0", ST_TERM_AREA);
    asm.call_external("gtk_widget_queue_draw");
    asm.push(abi::move_immediate("x0", "Boolean", FALSE));
    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::add_stack(16));
    asm.push(abi::return_());
    asm.finish(TERM_REDRAW_IDLE_SYMBOL, "Boolean")
}

/// `void _mfb_gtkapp_term_resize(GtkDrawingArea *area, int width /*x1*/,
/// int height /*x2*/, gpointer user_data)` — the drawing area's `resize` signal
/// handler (plan-35-E). Runs on the GTK main loop: recompute the active cols/rows
/// from the new allocation and the (font-fixed) cell metrics, update the extent in
/// `_mfb_gtkapp_state` so `term::terminalSize` tracks the live window, and force a
/// full redraw. The backing arrays keep their fixed stride (no realloc); only the
/// active top-left cols×rows change. Signal args arrive zero-extended in w1/w2.
pub(super) fn emit_term_resize_helper() -> Result<CodeFunction, String> {
    let mut asm = Asm::new(TERM_RESIZE_SYMBOL);
    // lr@0. width (x1) / height (x2) are consumed before the single queue_draw call,
    // so no callee-saved parking is needed.
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(16));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        0,
    ));
    // Stage width/height in scratch registers BEFORE dividing (like emit_term_init):
    // the x86 div lowering wants a renamable dividend, and x86 `div` clobbers the
    // second arg register (rdx = x2 = height), so both must be captured up front.
    asm.push(abi::move_register("x11", "x1")); // width
    asm.push(abi::move_register("x12", "x2")); // height
                                               // cols = clamp(width / cell_w, 1, MAX_COLS).
    asm.load_state("x10", ST_TERM_CELL_W);
    asm.push(abi::unsigned_divide_registers("x11", "x11", "x10"));
    emit_clamp_range(&mut asm, "x11", 1, TERM_MAX_COLS, "rz_cols");
    asm.store_state("x11", ST_TERM_COLS);
    // rows = clamp(height / cell_h, 1, MAX_ROWS).
    asm.load_state("x10", ST_TERM_CELL_H);
    asm.push(abi::unsigned_divide_registers("x12", "x12", "x10"));
    emit_clamp_range(&mut asm, "x12", 1, TERM_MAX_ROWS, "rz_rows");
    asm.store_state("x12", ST_TERM_ROWS);
    // Force a full redraw at the new extent.
    asm.load_state("x0", ST_TERM_AREA);
    asm.call_external("gtk_widget_queue_draw");
    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::add_stack(16));
    asm.push(abi::return_());
    asm.finish(TERM_RESIZE_SYMBOL, "Nothing")
}

/// `void _mfb_gtkapp_term_write(string obj /*x0*/, gboolean newline /*x1*/)` — the
/// worker-side grid writer the io write helpers call when term:: is active. It
/// mutates the fixed grid arrays (chars/fg/bg) from the worker thread. Bytes advance
/// the cursor; '\n' (and the trailing newline for print) move to the next row; when
/// the cursor passes the last row the grid scrolls up one line.
///
/// Concurrency (plan-35-E): the write does NOT schedule a redraw — it only touches
/// the LIVE grid arrays. The render callback ([`emit_term_draw_helper`]) reads the
/// separate draw-owned SNAPSHOT arrays, and a present (`term::sync`/`io::flush`/
/// `term::off`) copies live→snapshot on the GTK main loop before `queue_draw`
/// ([`emit_term_snapshot_copy`]). So a queued draw can no longer observe a
/// half-written frame — the former worker/draw tearing race is closed. The grids are
/// fixed-size static buffers (no reallocation, no dangling pointer, no memory
/// unsafety). Do not reintroduce a per-write redraw or a lock the worker holds across
/// the draw callback (either the mandatory-present contract breaks or the UI stalls).
pub(super) fn emit_term_write_helper() -> Result<CodeFunction, String> {
    let mut asm = Asm::new(TERM_WRITE_SYMBOL);
    // lr@0, x20(newline)@8, x21(i)@16, x22(len)@24, x23(ptr)@32, x24(charsBase)@40,
    // x25(row)@48, x26(col)@56, x27(fgBase)@64, x28(bgBase)@72, fgval@80, bgval@88,
    // x19(code-point byte length)@96.
    //
    // The length must be callee-saved: `tw_clamp` can call TERM_SCROLL_SYMBOL
    // between the decode and the `i += len` advance, so a caller-saved scratch
    // would not survive it (bug-203).
    let frame = 112;
    let (off_fgval, off_bgval) = (80usize, 88usize);
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(frame));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        0,
    ));
    for (reg, off) in [
        ("x19", 96),
        ("x20", 8),
        ("x21", 16),
        ("x22", 24),
        ("x23", 32),
        ("x24", 40),
        ("x25", 48),
        ("x26", 56),
        ("x27", 64),
        ("x28", 72),
    ] {
        asm.push(abi::store_u64(reg, abi::stack_pointer(), off));
    }
    asm.push(abi::move_register("x20", "x1")); // newline flag
    asm.push(abi::load_u64("x22", "x0", 0)); // text len
    asm.push(abi::add_immediate("x23", "x0", 8)); // text ptr
    asm.state_array("x24", ST_TERM_CHARS);
    asm.state_array("x27", ST_TERM_FG);
    asm.state_array("x28", ST_TERM_BG);
    asm.load_state("x25", ST_TERM_ROW);
    asm.load_state("x26", ST_TERM_COL);
    // fgval = cur_fg | (bold ? BOLD_FLAG : 0) | (underline ? UNDERLINE_FLAG : 0).
    // Hold cur_fg in x11 — load_state clobbers x9 as its address scratch.
    asm.load_state("x11", ST_TERM_CUR_FG);
    asm.load_state("x10", ST_TERM_CUR_BOLD);
    asm.push(abi::compare_immediate("x10", "0"));
    asm.push(abi::branch_eq("tw_no_bold"));
    asm.push(abi::move_immediate("x9", "Integer", &BOLD_FLAG.to_string()));
    asm.push(abi::or_registers("x11", "x11", "x9"));
    asm.push(abi::label("tw_no_bold"));
    asm.load_state("x10", ST_TERM_CUR_UNDERLINE);
    asm.push(abi::compare_immediate("x10", "0"));
    asm.push(abi::branch_eq("tw_no_ul"));
    asm.push(abi::move_immediate(
        "x9",
        "Integer",
        &UNDERLINE_FLAG.to_string(),
    ));
    asm.push(abi::or_registers("x11", "x11", "x9"));
    asm.push(abi::label("tw_no_ul"));
    asm.push(abi::store_u64("x11", abi::stack_pointer(), off_fgval));
    asm.load_state("x11", ST_TERM_CUR_BG);
    asm.push(abi::store_u64("x11", abi::stack_pointer(), off_bgval));
    asm.push(abi::move_immediate("x21", "Integer", "0")); // i

    asm.push(abi::label("tw_loop"));
    asm.push(abi::compare_registers("x21", "x22"));
    asm.push(abi::branch_ge("tw_after"));
    asm.push(abi::add_registers("x9", "x23", "x21"));
    asm.push(abi::load_u8("x10", "x9", 0)); // byte = ptr[i]
    asm.push(abi::compare_immediate("x10", "10")); // '\n'
    asm.push(abi::branch_eq("tw_newline"));
    // Decode ONE code point into x10 as its UTF-8 bytes packed little-endian,
    // with its length in x19 (bug-203). Storing a byte per cell split a
    // multi-byte glyph across cells: each cell held a lone fragment, the cursor
    // advanced by the byte count instead of one column, and the draw handed
    // cairo invalid UTF-8 (tofu). The lead byte gives the length:
    //   0xxxxxxx -> 1   110xxxxx -> 2   1110xxxx -> 3   11110xxx -> 4
    // A bare continuation byte (10xxxxxx) or a truncated tail is not a lead
    // byte; those fall through as length 1, so malformed input still advances
    // and renders per-byte rather than hanging or over-reading.
    emit_utf8_decode_at(&mut asm, "x10", "x9", "x19", "x21", "x22");
    // idx = row*MAX_COLS + col; chars[idx]=glyph; fg[idx]=fgval; bg[idx]=bgval.
    asm.push(abi::move_immediate(
        "x11",
        "Integer",
        &TERM_MAX_COLS.to_string(),
    ));
    asm.push(abi::multiply_registers("x12", "x25", "x11"));
    asm.push(abi::add_registers("x12", "x12", "x26")); // idx
    asm.push(abi::shift_left_immediate("x13", "x12", 2)); // idx*4
    asm.push(abi::add_registers("x9", "x24", "x13"));
    asm.push(abi::store_u32("x10", "x9", 0));
    asm.push(abi::load_u64("x9", abi::stack_pointer(), off_fgval));
    asm.push(abi::add_registers("x14", "x27", "x13"));
    asm.push(abi::store_u32("x9", "x14", 0));
    asm.push(abi::load_u64("x9", abi::stack_pointer(), off_bgval));
    asm.push(abi::add_registers("x14", "x28", "x13"));
    asm.push(abi::store_u32("x9", "x14", 0));
    // col++; wrap to next row at the active cols.
    asm.push(abi::add_immediate("x26", "x26", 1));
    asm.load_state("x9", ST_TERM_COLS);
    asm.push(abi::compare_registers("x26", "x9"));
    asm.push(abi::branch_lt("tw_next"));
    asm.push(abi::move_immediate("x26", "Integer", "0"));
    asm.push(abi::add_immediate("x25", "x25", 1));
    asm.push(abi::branch("tw_clamp"));
    asm.push(abi::label("tw_newline"));
    asm.push(abi::move_immediate("x19", "Integer", "1")); // '\n' is one byte
    asm.push(abi::move_immediate("x26", "Integer", "0"));
    asm.push(abi::add_immediate("x25", "x25", 1));
    asm.push(abi::label("tw_clamp"));
    // Scroll the grid up when the cursor passes the bottom (matches macOS).
    asm.load_state("x9", ST_TERM_ROWS);
    asm.push(abi::compare_registers("x25", "x9"));
    asm.push(abi::branch_lt("tw_next"));
    asm.call_internal(TERM_SCROLL_SYMBOL);
    asm.load_state("x25", ST_TERM_ROWS);
    asm.push(abi::subtract_immediate("x25", "x25", 1));
    asm.push(abi::label("tw_next"));
    // Advance by the code point's byte length (x19), set to 1 on the '\n' path
    // below. The cursor moved one column per glyph above (bug-203).
    asm.push(abi::add_registers("x21", "x21", "x19"));
    asm.push(abi::branch("tw_loop"));

    asm.push(abi::label("tw_after"));
    // print's trailing newline.
    asm.push(abi::compare_immediate("x20", "0"));
    asm.push(abi::branch_eq("tw_store"));
    asm.push(abi::move_immediate("x26", "Integer", "0"));
    asm.push(abi::add_immediate("x25", "x25", 1));
    asm.load_state("x9", ST_TERM_ROWS);
    asm.push(abi::compare_registers("x25", "x9"));
    asm.push(abi::branch_lt("tw_store"));
    asm.call_internal(TERM_SCROLL_SYMBOL);
    asm.load_state("x25", ST_TERM_ROWS);
    asm.push(abi::subtract_immediate("x25", "x25", 1));
    asm.push(abi::label("tw_store"));
    asm.store_state("x25", ST_TERM_ROW);
    asm.store_state("x26", ST_TERM_COL);
    // plan-35-E: NO per-write redraw. Writing only mutates the live grid; a present
    // (`term::sync`/`io::flush`/`term::off`) snapshots + queue_draws on the main loop.

    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    for (reg, off) in [
        ("x19", 96),
        ("x20", 8),
        ("x21", 16),
        ("x22", 24),
        ("x23", 32),
        ("x24", 40),
        ("x25", 48),
        ("x26", 56),
        ("x27", 64),
        ("x28", 72),
    ] {
        asm.push(abi::load_u64(reg, abi::stack_pointer(), off));
    }
    asm.push(abi::add_stack(frame));
    asm.push(abi::return_());
    asm.finish(TERM_WRITE_SYMBOL, "Nothing")
}
