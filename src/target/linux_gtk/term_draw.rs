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
    asm.push(abi::subtract_registers(abi::SCRATCH[2], count, index));
    asm.push(abi::compare_registers(len, abi::SCRATCH[2]));
    asm.push(abi::branch_ls("u8_len_ok"));
    asm.push(abi::move_immediate(len, "Integer", "1"));
    asm.push(abi::label("u8_len_ok"));
    // Pack the continuation bytes with fixed shifts (len is 1-4, so unrolling
    // avoids needing a variable shift).
    for (byte, shift) in [(1usize, 8u8), (2, 16), (3, 24)] {
        asm.push(abi::compare_immediate(len, &(byte + 1).to_string()));
        asm.push(abi::branch_lt("u8_pack_done"));
        asm.push(abi::load_u8(abi::SCRATCH[3], ptr, byte));
        asm.push(abi::shift_left_immediate(
            abi::SCRATCH[3],
            abi::SCRATCH[3],
            shift,
        ));
        asm.push(abi::or_registers(glyph, glyph, abi::SCRATCH[3]));
    }
    asm.push(abi::label("u8_pack_done"));
}

/// plan-70-E: `out` = display width (1 or 2) of the grapheme whose UTF-8 bytes are
/// packed little-endian in `packed` (the char-cell form) with byte length `len`.
/// First decodes the Unicode scalar from those bytes, then looks up A's charwidth via
/// the two-stage property trie (`(flags@16 >> 4) & 3`, width 0 folded to 1). Uses
/// `s1`/`s2`/`s3` as scratch and does not clobber `packed`/`len`; `tag` makes the
/// internal labels unique. The `_mfb_unicode_*` relocations these `local_address`
/// loads emit make the shared build embed the ~1.5 MB property table — call ONLY when
/// the app uses `term::` (the writer gates on `uses_term`).
fn emit_gtk_charwidth(
    asm: &mut Asm,
    packed: &str,
    len: &str,
    out: &str,
    s1: &str,
    s2: &str,
    s3: &str,
    tag: &str,
) {
    use crate::codegen::error::constants::UNICODE_PROPERTIES_SYMBOL;
    use crate::codegen::error::constants::UNICODE_STAGE1_SYMBOL;
    use crate::codegen::error::constants::UNICODE_STAGE2_SYMBOL;
    let n2 = format!("{tag}_n2");
    let n3 = format!("{tag}_n3");
    let n4 = format!("{tag}_n4");
    let sdone = format!("{tag}_sdone");
    let lookup = format!("{tag}_lk");
    let done = format!("{tag}_done");
    // --- decode the scalar from the packed UTF-8 bytes into `out` ---
    // b0=packed&0xFF, b1=(packed>>8)&0x3F, b2=(packed>>16)&0x3F, b3=(packed>>24)&0x3F.
    asm.push(abi::compare_immediate(len, "1"));
    asm.push(abi::branch_ne(&n2));
    asm.push(abi::move_immediate(s1, "Integer", "127"));
    asm.push(abi::and_registers(out, packed, s1)); // ASCII
    asm.push(abi::branch(&sdone));
    asm.push(abi::label(&n2));
    asm.push(abi::compare_immediate(len, "2"));
    asm.push(abi::branch_ne(&n3));
    asm.push(abi::move_immediate(s1, "Integer", "31"));
    asm.push(abi::and_registers(out, packed, s1)); // b0 & 0x1F
    asm.push(abi::shift_left_immediate(out, out, 6));
    asm.push(abi::shift_right_immediate(s2, packed, 8));
    asm.push(abi::move_immediate(s1, "Integer", "63"));
    asm.push(abi::and_registers(s2, s2, s1)); // b1 & 0x3F
    asm.push(abi::or_registers(out, out, s2));
    asm.push(abi::branch(&sdone));
    asm.push(abi::label(&n3));
    asm.push(abi::compare_immediate(len, "3"));
    asm.push(abi::branch_ne(&n4));
    asm.push(abi::move_immediate(s1, "Integer", "15"));
    asm.push(abi::and_registers(out, packed, s1)); // b0 & 0x0F
    asm.push(abi::shift_left_immediate(out, out, 12));
    asm.push(abi::move_immediate(s1, "Integer", "63"));
    asm.push(abi::shift_right_immediate(s2, packed, 8));
    asm.push(abi::and_registers(s2, s2, s1));
    asm.push(abi::shift_left_immediate(s2, s2, 6));
    asm.push(abi::or_registers(out, out, s2));
    asm.push(abi::shift_right_immediate(s2, packed, 16));
    asm.push(abi::and_registers(s2, s2, s1));
    asm.push(abi::or_registers(out, out, s2));
    asm.push(abi::branch(&sdone));
    asm.push(abi::label(&n4));
    // len 4 (or any other): b0 & 0x07.
    asm.push(abi::move_immediate(s1, "Integer", "7"));
    asm.push(abi::and_registers(out, packed, s1));
    asm.push(abi::shift_left_immediate(out, out, 18));
    asm.push(abi::move_immediate(s1, "Integer", "63"));
    asm.push(abi::shift_right_immediate(s2, packed, 8));
    asm.push(abi::and_registers(s2, s2, s1));
    asm.push(abi::shift_left_immediate(s2, s2, 12));
    asm.push(abi::or_registers(out, out, s2));
    asm.push(abi::shift_right_immediate(s2, packed, 16));
    asm.push(abi::and_registers(s2, s2, s1));
    asm.push(abi::shift_left_immediate(s2, s2, 6));
    asm.push(abi::or_registers(out, out, s2));
    asm.push(abi::shift_right_immediate(s2, packed, 24));
    asm.push(abi::and_registers(s2, s2, s1));
    asm.push(abi::or_registers(out, out, s2));
    asm.push(abi::label(&sdone));
    // --- two-stage trie lookup: `out` (scalar) -> width ---
    asm.push(abi::move_immediate(s1, "Integer", "1114112")); // 0x110000
    asm.push(abi::compare_registers(out, s1));
    asm.push(abi::branch_lt(&lookup));
    asm.push(abi::move_immediate(out, "Integer", "1"));
    asm.push(abi::branch(&done));
    asm.push(abi::label(&lookup));
    asm.push(abi::shift_right_immediate(s1, out, 8));
    asm.push(abi::shift_left_immediate(s1, s1, 1));
    asm.local_address(s2, UNICODE_STAGE1_SYMBOL);
    asm.push(abi::add_registers(s2, s2, s1));
    asm.push(abi::load_u16(s1, s2, 0));
    asm.push(abi::move_immediate(s3, "Integer", "255"));
    asm.push(abi::and_registers(s3, out, s3));
    asm.push(abi::add_registers(s1, s1, s3));
    asm.push(abi::shift_left_immediate(s1, s1, 1));
    asm.local_address(s2, UNICODE_STAGE2_SYMBOL);
    asm.push(abi::add_registers(s2, s2, s1));
    asm.push(abi::load_u16(s1, s2, 0));
    // Property record: 6 live u16 fields = 12 bytes, flags @ offset 6 (plan-77 U1
    // repacked it from 24 bytes / flags @ 16). Must match the shared reader's
    // UNICODE_PROPERTY_OFFSET_FLAGS in target/shared/code/private/unicode.rs.
    asm.push(abi::move_immediate(s3, "Integer", "12")); // property record size
    asm.push(abi::multiply_registers(s1, s1, s3));
    asm.local_address(s2, UNICODE_PROPERTIES_SYMBOL);
    asm.push(abi::add_registers(s2, s2, s1));
    asm.push(abi::load_u16(out, s2, 6)); // flags @ 6
    asm.push(abi::shift_right_immediate(out, out, 4));
    asm.push(abi::move_immediate(s3, "Integer", "3"));
    asm.push(abi::and_registers(out, out, s3)); // (flags>>4)&3 -> raw width 0/1/2
    asm.push(abi::label(&done));
    // NOTE: width 0 (a combining mark / zero-width scalar) is returned raw; the
    // writer folds it into the previous cell's EGC pool (plan-70-E Phase 3), or, if
    // there is no base to attach to, treats it as a lone width-1 cell.
}

/// plan-70-E Phase 3: fold a combining mark (UTF-8 bytes packed LE in `mark`, length
/// `marklen`) into the previous base cell's EGC pool slot. `charoff` is the base
/// cell's byte offset into the CHAR array (`idx*4`); the parallel pool slot is at
/// `idx*32 == charoff*8`. The slot is length-prefixed: `pool[0]` = total byte length,
/// `pool[1..]` = the cluster's UTF-8 bytes. On the first mark the base's own bytes seed
/// the slot and its CHAR word becomes `GTK_POOL_TAG`; a cluster that would exceed the
/// slot drops the tail (rare, long ZWJ). Uses caller-saved x11-x17; preserves
/// `mark`/`marklen`/`charoff` and the callee-saved loop registers.
fn emit_gtk_pool_append(asm: &mut Asm, charoff: &str, mark: &str, marklen: &str, tag: &str) {
    let pooled = format!("{tag}_pooled");
    let bl = format!("{tag}_bl");
    let write = format!("{tag}_write");
    let skip = format!("{tag}_skip");
    // pool slot addr (x11) = ST_TERM_POOL + charoff*8; base char addr (x12).
    asm.state_array(abi::SCRATCH[2], ST_TERM_POOL);
    asm.push(abi::shift_left_immediate(abi::SCRATCH[8], charoff, 3));
    asm.push(abi::add_registers(
        abi::SCRATCH[2],
        abi::SCRATCH[2],
        abi::SCRATCH[8],
    ));
    asm.state_array(abi::SCRATCH[3], ST_TERM_CHARS);
    asm.push(abi::add_registers(
        abi::SCRATCH[3],
        abi::SCRATCH[3],
        charoff,
    ));
    asm.push(abi::load_u32(abi::SCRATCH[4], abi::SCRATCH[3], 0)); // base word (packed bytes OR POOL_TAG)
    asm.push(abi::move_immediate(
        abi::SCRATCH[5],
        "Integer",
        GTK_POOL_TAG,
    ));
    asm.push(abi::compare_registers(abi::SCRATCH[4], abi::SCRATCH[5]));
    asm.push(abi::branch_eq(&pooled));
    // First mark: seed the slot with the base's UTF-8 bytes. blen from the lead byte.
    asm.push(abi::move_immediate(abi::SCRATCH[5], "Integer", "255"));
    asm.push(abi::and_registers(
        abi::SCRATCH[6],
        abi::SCRATCH[4],
        abi::SCRATCH[5],
    )); // lead byte
    asm.push(abi::move_immediate(abi::SCRATCH[8], "Integer", "1"));
    asm.push(abi::compare_immediate(abi::SCRATCH[6], "192"));
    asm.push(abi::branch_lt(&bl));
    asm.push(abi::move_immediate(abi::SCRATCH[8], "Integer", "2"));
    asm.push(abi::compare_immediate(abi::SCRATCH[6], "224"));
    asm.push(abi::branch_lt(&bl));
    asm.push(abi::move_immediate(abi::SCRATCH[8], "Integer", "3"));
    asm.push(abi::compare_immediate(abi::SCRATCH[6], "240"));
    asm.push(abi::branch_lt(&bl));
    asm.push(abi::move_immediate(abi::SCRATCH[8], "Integer", "4"));
    asm.push(abi::label(&bl));
    // base bytes at pool[1..] — via an address register (offset 1 is an unaligned u32).
    asm.push(abi::add_immediate(abi::SCRATCH[6], abi::SCRATCH[2], 1));
    asm.push(abi::store_u32(abi::SCRATCH[4], abi::SCRATCH[6], 0));
    asm.push(abi::store_u8(abi::SCRATCH[8], abi::SCRATCH[2], 0)); // pool[0] = blen
    asm.push(abi::move_immediate(
        abi::SCRATCH[5],
        "Integer",
        GTK_POOL_TAG,
    ));
    asm.push(abi::store_u32(abi::SCRATCH[5], abi::SCRATCH[3], 0)); // CHAR = POOL_TAG
    asm.push(abi::branch(&write));
    asm.push(abi::label(&pooled));
    asm.push(abi::load_u8(abi::SCRATCH[8], abi::SCRATCH[2], 0)); // current length
    asm.push(abi::label(&write));
    // Guard the 4-byte store against the 32-byte slot end (curlen <= 27 leaves room).
    asm.push(abi::compare_immediate(abi::SCRATCH[8], "27"));
    asm.push(abi::branch_gt(&skip));
    // write pos = pool + 1 + curlen; store the mark's ≤4 bytes; pool[0] += marklen.
    asm.push(abi::add_immediate(abi::SCRATCH[4], abi::SCRATCH[2], 1));
    asm.push(abi::add_registers(
        abi::SCRATCH[4],
        abi::SCRATCH[4],
        abi::SCRATCH[8],
    ));
    asm.push(abi::store_u32(mark, abi::SCRATCH[4], 0));
    asm.push(abi::add_registers(
        abi::SCRATCH[5],
        abi::SCRATCH[8],
        marklen,
    ));
    asm.push(abi::store_u8(abi::SCRATCH[5], abi::SCRATCH[2], 0));
    asm.push(abi::label(&skip));
}

/// Emit `cairo_set_source_rgb(cr, r/255, g/255, b/255)` from a packed RGB value in
/// `packed` (low 24 bits). Clobbers x0/x9-x13 and d0-d3.
fn emit_cairo_color(asm: &mut Asm, cr: &str, packed: &str) {
    asm.push(abi::move_register(abi::c_arg(0), cr));
    asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", "255")); // mask + divisor
    asm.push(abi::and_registers(abi::SCRATCH[1], packed, abi::SCRATCH[0])); // r = packed & 0xFF
    asm.push(abi::shift_right_immediate(abi::SCRATCH[2], packed, 8)); // g
    asm.push(abi::and_registers(
        abi::SCRATCH[2],
        abi::SCRATCH[2],
        abi::SCRATCH[0],
    ));
    asm.push(abi::shift_right_immediate(abi::SCRATCH[3], packed, 16)); // b
    asm.push(abi::and_registers(
        abi::SCRATCH[3],
        abi::SCRATCH[3],
        abi::SCRATCH[0],
    ));
    asm.push(abi::signed_convert_to_float_d(
        abi::FP_SCRATCH[3],
        abi::SCRATCH[0],
    )); // 255.0
    asm.push(abi::signed_convert_to_float_d(
        abi::FP_SCRATCH[0],
        abi::SCRATCH[1],
    ));
    asm.push(abi::float_divide_d(
        abi::FP_SCRATCH[0],
        abi::FP_SCRATCH[0],
        abi::FP_SCRATCH[3],
    ));
    asm.push(abi::signed_convert_to_float_d(
        abi::FP_SCRATCH[1],
        abi::SCRATCH[2],
    ));
    asm.push(abi::float_divide_d(
        abi::FP_SCRATCH[1],
        abi::FP_SCRATCH[1],
        abi::FP_SCRATCH[3],
    ));
    asm.push(abi::signed_convert_to_float_d(
        abi::FP_SCRATCH[2],
        abi::SCRATCH[3],
    ));
    asm.push(abi::float_divide_d(
        abi::FP_SCRATCH[2],
        abi::FP_SCRATCH[2],
        abi::FP_SCRATCH[3],
    ));
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
    // charbuf@96 (5B: up to 4 UTF-8 bytes + NUL). plan-70-E: pango layout@104,
    // desc@112 — created once, reused per cell (Pango draws with font fallback).
    let frame = 128;
    let (off_fg, off_bg, off_buf) = (80usize, 88usize, 96usize);
    let (off_layout, off_desc) = (104usize, 112usize);
    let saved = [
        (abi::LOCAL[0], 8),
        (abi::LOCAL[1], 16),
        (abi::LOCAL[2], 24),
        (abi::LOCAL[3], 32),
        (abi::LOCAL[4], 40),
        (abi::LOCAL[5], 48),
        (abi::LOCAL[6], 56),
        (abi::LOCAL[7], 64),
        (abi::LOCAL[8], 72),
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
    asm.push(abi::move_register(abi::LOCAL[0], abi::c_arg(1))); // cr

    // Black background.
    asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[0]));
    asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
    asm.push(abi::float_move_d_from_x(
        abi::FP_SCRATCH[0],
        abi::SCRATCH[0],
    ));
    asm.push(abi::float_move_d_from_x(
        abi::FP_SCRATCH[1],
        abi::SCRATCH[0],
    ));
    asm.push(abi::float_move_d_from_x(
        abi::FP_SCRATCH[2],
        abi::SCRATCH[0],
    ));
    asm.call_external("cairo_set_source_rgb");
    asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[0]));
    asm.call_external("cairo_paint");
    // plan-70-E: one Pango layout + monospace font description for the whole frame,
    // reused per cell (set_text + show_layout). Pango cascades fonts, so CJK/emoji
    // render instead of tofu. desc weight starts normal; lastBold tracks it.
    asm.local_address(abi::c_arg(0), STR_MONO_DESC.0);
    asm.call_external("pango_font_description_from_string");
    asm.push(abi::store_u64(
        abi::c_return(0),
        abi::stack_pointer(),
        off_desc,
    ));
    asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[0]));
    asm.call_external("pango_cairo_create_layout");
    asm.push(abi::store_u64(
        abi::c_return(0),
        abi::stack_pointer(),
        off_layout,
    ));
    asm.push(abi::load_u64(abi::c_arg(1), abi::stack_pointer(), off_desc));
    asm.push(abi::move_register(abi::c_arg(0), abi::c_return(0)));
    asm.call_external("pango_layout_set_font_description");
    asm.push(abi::move_immediate(abi::LOCAL[3], "Integer", "0"));
    // Render the draw-owned SNAPSHOT arrays (plan-35-E): a present copies the live
    // worker arrays here on the main loop before queue_draw, so this callback never
    // reads a half-written frame. The active extent (cols/rows) + cursor are read
    // live — a torn single u64 there is benign and self-corrects next present.
    asm.state_array(abi::LOCAL[4], ST_TERM_SNAP_CHARS);
    asm.state_array(abi::LOCAL[5], ST_TERM_SNAP_FG);
    asm.state_array(abi::LOCAL[6], ST_TERM_SNAP_BG);
    asm.load_state(abi::LOCAL[7], ST_TERM_COLS);
    asm.load_state(abi::LOCAL[8], ST_TERM_ROWS);

    asm.push(abi::move_immediate(abi::LOCAL[1], "Integer", "0")); // row
    asm.push(abi::label("d_row"));
    asm.push(abi::compare_registers(abi::LOCAL[1], abi::LOCAL[8])); // row < rows?
    asm.push(abi::branch_ge("d_done"));
    asm.push(abi::move_immediate(abi::LOCAL[2], "Integer", "0")); // col
    asm.push(abi::label("d_col"));
    asm.push(abi::compare_registers(abi::LOCAL[2], abi::LOCAL[7])); // col < cols?
    asm.push(abi::branch_ge("d_row_next"));
    // idx = row*MAX_COLS + col (fixed backing stride)
    asm.push(abi::move_immediate(
        abi::SCRATCH[0],
        "Integer",
        &TERM_MAX_COLS.to_string(),
    ));
    asm.push(abi::multiply_registers(
        abi::SCRATCH[1],
        abi::LOCAL[1],
        abi::SCRATCH[0],
    ));
    asm.push(abi::add_registers(
        abi::SCRATCH[1],
        abi::SCRATCH[1],
        abi::LOCAL[2],
    )); // idx
    asm.push(abi::shift_left_immediate(
        abi::SCRATCH[2],
        abi::SCRATCH[1],
        2,
    )); // idx*4
        // char -> charbuf; fg, bg -> stack (survive cairo calls). The cell holds one
        // code point's UTF-8 bytes packed little-endian, so storing the u32 lays them
        // out in order; the NUL after it terminates the 1-4 byte sequence for
        // `cairo_show_text` (bug-203 — this used to store a single byte, which cut a
        // multi-byte glyph into invalid fragments).
    asm.push(abi::add_registers(
        abi::SCRATCH[3],
        abi::LOCAL[4],
        abi::SCRATCH[2],
    ));
    asm.push(abi::load_u32(abi::SCRATCH[4], abi::SCRATCH[3], 0));
    asm.push(abi::store_u32(
        abi::SCRATCH[4],
        abi::stack_pointer(),
        off_buf,
    ));
    asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
    asm.push(abi::store_u8(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_buf + 4,
    ));
    asm.push(abi::add_registers(
        abi::SCRATCH[3],
        abi::LOCAL[5],
        abi::SCRATCH[2],
    ));
    asm.push(abi::load_u32(abi::SCRATCH[4], abi::SCRATCH[3], 0));
    asm.push(abi::store_u64(
        abi::SCRATCH[4],
        abi::stack_pointer(),
        off_fg,
    ));
    asm.push(abi::add_registers(
        abi::SCRATCH[3],
        abi::LOCAL[6],
        abi::SCRATCH[2],
    ));
    asm.push(abi::load_u32(abi::SCRATCH[4], abi::SCRATCH[3], 0));
    asm.push(abi::store_u64(
        abi::SCRATCH[4],
        abi::stack_pointer(),
        off_bg,
    ));

    // Background rect when an explicit bg is set.
    asm.push(abi::load_u64(abi::SCRATCH[5], abi::stack_pointer(), off_bg));
    asm.push(abi::move_immediate(
        abi::SCRATCH[0],
        "Integer",
        &COLOR_SET.to_string(),
    ));
    asm.push(abi::and_registers(
        abi::SCRATCH[0],
        abi::SCRATCH[5],
        abi::SCRATCH[0],
    ));
    asm.push(abi::compare_immediate(abi::SCRATCH[0], "0"));
    asm.push(abi::branch_eq("d_no_bg"));
    emit_cairo_color(&mut asm, abi::LOCAL[0], abi::SCRATCH[5]);
    asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[0])); // rectangle(cr, col*W, row*H, W, H)
    emit_cell_dim_to_d(&mut asm, "d0", abi::LOCAL[2], ST_TERM_CELL_W);
    emit_cell_dim_to_d(&mut asm, "d1", abi::LOCAL[1], ST_TERM_CELL_H);
    emit_cell_to_d(&mut asm, "d2", ST_TERM_CELL_W);
    emit_cell_to_d(&mut asm, "d3", ST_TERM_CELL_H);
    asm.call_external("cairo_rectangle");
    asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[0]));
    asm.call_external("cairo_fill");
    asm.push(abi::label("d_no_bg"));

    // Glyph (skip blanks). A never-written cell is 0 (the blanking memsets clear
    // whole u32 cells) and an explicitly written space is 32; both render
    // nothing. Compares the whole cell, so a multi-byte glyph whose lead byte
    // happens to be 0x20 could never be mistaken for a space.
    asm.push(abi::load_u32(
        abi::SCRATCH[4],
        abi::stack_pointer(),
        off_buf,
    ));
    asm.push(abi::compare_immediate(abi::SCRATCH[4], "0"));
    asm.push(abi::branch_eq("d_next"));
    asm.push(abi::compare_immediate(abi::SCRATCH[4], "32"));
    asm.push(abi::branch_eq("d_next"));
    // plan-70-E: a WIDE_TRAIL sentinel draws nothing — the wide primary's Pango glyph
    // already spans this column (its background was filled above). Skip it.
    asm.push(abi::move_immediate(
        abi::SCRATCH[0],
        "Integer",
        GTK_WIDE_TRAIL,
    ));
    asm.push(abi::compare_registers(abi::SCRATCH[4], abi::SCRATCH[0]));
    asm.push(abi::branch_eq("d_next"));
    // plan-70-E: re-weight the Pango font description if bold changed (700 bold /
    // 400 normal), then reapply it to the layout.
    asm.push(abi::load_u64(abi::SCRATCH[5], abi::stack_pointer(), off_fg));
    asm.push(abi::move_immediate(
        abi::SCRATCH[0],
        "Integer",
        &BOLD_FLAG.to_string(),
    ));
    asm.push(abi::and_registers(
        abi::SCRATCH[0],
        abi::SCRATCH[5],
        abi::SCRATCH[0],
    )); // 0 or BOLD_FLAG
    asm.push(abi::compare_registers(abi::SCRATCH[0], abi::LOCAL[3]));
    asm.push(abi::branch_eq("d_bold_ok"));
    asm.push(abi::move_register(abi::LOCAL[3], abi::SCRATCH[0]));
    asm.push(abi::load_u64(abi::c_arg(0), abi::stack_pointer(), off_desc));
    asm.push(abi::compare_immediate(abi::SCRATCH[0], "0"));
    asm.push(abi::branch_eq("d_sel_normal"));
    asm.push(abi::move_immediate(abi::c_arg(1), "Integer", "700")); // PANGO_WEIGHT_BOLD
    asm.push(abi::branch("d_sel_apply"));
    asm.push(abi::label("d_sel_normal"));
    asm.push(abi::move_immediate(abi::c_arg(1), "Integer", "400")); // PANGO_WEIGHT_NORMAL
    asm.push(abi::label("d_sel_apply"));
    asm.call_external("pango_font_description_set_weight");
    asm.push(abi::load_u64(
        abi::c_arg(0),
        abi::stack_pointer(),
        off_layout,
    ));
    asm.push(abi::load_u64(abi::c_arg(1), abi::stack_pointer(), off_desc));
    asm.call_external("pango_layout_set_font_description");
    asm.push(abi::label("d_bold_ok"));
    // Foreground color: explicit or white.
    asm.push(abi::load_u64(abi::SCRATCH[5], abi::stack_pointer(), off_fg));
    asm.push(abi::move_immediate(
        abi::SCRATCH[0],
        "Integer",
        &COLOR_SET.to_string(),
    ));
    asm.push(abi::and_registers(
        abi::SCRATCH[0],
        abi::SCRATCH[5],
        abi::SCRATCH[0],
    ));
    asm.push(abi::compare_immediate(abi::SCRATCH[0], "0"));
    asm.push(abi::branch_eq("d_fg_white"));
    emit_cairo_color(&mut asm, abi::LOCAL[0], abi::SCRATCH[5]);
    asm.push(abi::branch("d_fg_done"));
    asm.push(abi::label("d_fg_white"));
    asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[0]));
    asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", "1"));
    asm.push(abi::signed_convert_to_float_d(
        abi::FP_SCRATCH[0],
        abi::SCRATCH[0],
    ));
    asm.push(abi::signed_convert_to_float_d(
        abi::FP_SCRATCH[1],
        abi::SCRATCH[0],
    ));
    asm.push(abi::signed_convert_to_float_d(
        abi::FP_SCRATCH[2],
        abi::SCRATCH[0],
    ));
    asm.call_external("cairo_set_source_rgb");
    asm.push(abi::label("d_fg_done"));
    // plan-70-E: pango_layout_set_text; move_to(col*cellW, row*cellH) — Pango draws
    // the layout from the current point as its TOP-LEFT (not a baseline) — then
    // pango_cairo_show_layout in the fg colour set above. A pooled cluster
    // (char == POOL_TAG) feeds the length-prefixed EGC slot (Pango composes the
    // combining marks); a lone scalar feeds the NUL-terminated 4-byte charbuf.
    asm.push(abi::load_u32(
        abi::SCRATCH[4],
        abi::stack_pointer(),
        off_buf,
    ));
    asm.push(abi::move_immediate(
        abi::SCRATCH[0],
        "Integer",
        GTK_POOL_TAG,
    ));
    asm.push(abi::compare_registers(abi::SCRATCH[4], abi::SCRATCH[0]));
    asm.push(abi::branch_ne("d_inline_text"));
    asm.push(abi::move_immediate(
        abi::SCRATCH[2],
        "Integer",
        &TERM_MAX_COLS.to_string(),
    ));
    asm.push(abi::multiply_registers(
        abi::SCRATCH[3],
        abi::LOCAL[1],
        abi::SCRATCH[2],
    ));
    asm.push(abi::add_registers(
        abi::SCRATCH[3],
        abi::SCRATCH[3],
        abi::LOCAL[2],
    ));
    asm.push(abi::shift_left_immediate(
        abi::SCRATCH[3],
        abi::SCRATCH[3],
        5,
    )); // idx*32 (pool stride)
    asm.state_array(abi::SCRATCH[0], ST_TERM_SNAP_POOL);
    asm.push(abi::add_registers(
        abi::SCRATCH[0],
        abi::SCRATCH[0],
        abi::SCRATCH[3],
    )); // pool slot
    asm.push(abi::load_u8(abi::c_arg(2), abi::SCRATCH[0], 0)); // length prefix
    asm.push(abi::add_immediate(abi::c_arg(1), abi::SCRATCH[0], 1)); // cluster bytes
    asm.push(abi::branch("d_set_text"));
    asm.push(abi::label("d_inline_text"));
    asm.push(abi::add_immediate(
        abi::c_arg(1),
        abi::stack_pointer(),
        off_buf,
    ));
    asm.push(abi::move_immediate(abi::c_arg(2), "Integer", "0"));
    asm.push(abi::bitwise_not(abi::c_arg(2), abi::c_arg(2))); // length -1 => NUL-terminated (no neg immediate)
    asm.push(abi::label("d_set_text"));
    asm.push(abi::load_u64(
        abi::c_arg(0),
        abi::stack_pointer(),
        off_layout,
    ));
    asm.call_external("pango_layout_set_text");
    asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[0]));
    emit_cell_dim_to_d(&mut asm, "d0", abi::LOCAL[2], ST_TERM_CELL_W); // x = col*cellW
    emit_cell_dim_to_d(&mut asm, "d1", abi::LOCAL[1], ST_TERM_CELL_H); // y = row*cellH (top)
    asm.call_external("cairo_move_to");
    asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[0]));
    asm.push(abi::load_u64(
        abi::c_arg(1),
        abi::stack_pointer(),
        off_layout,
    ));
    asm.call_external("pango_cairo_show_layout");
    // Underline: a 2px rect at the cell bottom in the (already-set) fg color.
    asm.push(abi::load_u64(abi::SCRATCH[5], abi::stack_pointer(), off_fg));
    asm.push(abi::move_immediate(
        abi::SCRATCH[0],
        "Integer",
        &UNDERLINE_FLAG.to_string(),
    ));
    asm.push(abi::and_registers(
        abi::SCRATCH[0],
        abi::SCRATCH[5],
        abi::SCRATCH[0],
    ));
    asm.push(abi::compare_immediate(abi::SCRATCH[0], "0"));
    asm.push(abi::branch_eq("d_next"));
    emit_term_cell_rect(&mut asm, abi::LOCAL[0], abi::LOCAL[2], abi::LOCAL[1]);

    asm.push(abi::label("d_next"));
    asm.push(abi::add_immediate(abi::LOCAL[2], abi::LOCAL[2], 1));
    asm.push(abi::branch("d_col"));
    asm.push(abi::label("d_row_next"));
    asm.push(abi::add_immediate(abi::LOCAL[1], abi::LOCAL[1], 1));
    asm.push(abi::branch("d_row"));

    asm.push(abi::label("d_done"));
    // Cursor caret: a 2px bar at the cursor cell bottom in white, if visible.
    asm.load_state(abi::SCRATCH[0], ST_TERM_CURSOR_VISIBLE);
    asm.push(abi::compare_immediate(abi::SCRATCH[0], "0"));
    asm.push(abi::branch_eq("d_no_cursor"));
    asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[0]));
    asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", "1"));
    asm.push(abi::signed_convert_to_float_d(
        abi::FP_SCRATCH[0],
        abi::SCRATCH[0],
    ));
    asm.push(abi::signed_convert_to_float_d(
        abi::FP_SCRATCH[1],
        abi::SCRATCH[0],
    ));
    asm.push(abi::signed_convert_to_float_d(
        abi::FP_SCRATCH[2],
        abi::SCRATCH[0],
    ));
    asm.call_external("cairo_set_source_rgb");
    asm.load_state(abi::LOCAL[1], ST_TERM_ROW);
    asm.load_state(abi::LOCAL[2], ST_TERM_COL);
    emit_term_cell_rect(&mut asm, abi::LOCAL[0], abi::LOCAL[2], abi::LOCAL[1]);
    asm.push(abi::label("d_no_cursor"));
    // plan-70-E: release the per-frame Pango layout + font description.
    asm.push(abi::load_u64(
        abi::c_arg(0),
        abi::stack_pointer(),
        off_layout,
    ));
    asm.call_external("g_object_unref");
    asm.push(abi::load_u64(abi::c_arg(0), abi::stack_pointer(), off_desc));
    asm.call_external("pango_font_description_free");
    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    for (reg, off) in saved {
        asm.push(abi::load_u64(reg, abi::stack_pointer(), off));
    }
    asm.push(abi::add_stack(frame));
    asm.push(abi::return_());
    asm.finish(TERM_DRAW_SYMBOL, "Nothing")
}

/// `dst (d-reg) = index * cellSize` as a double, where the cell size (px) is read
/// from the runtime-state field `cell_off`. Clobbers x9.
fn emit_cell_dim_to_d(asm: &mut Asm, dst: &str, index: &str, cell_off: usize) {
    asm.load_state(abi::SCRATCH[0], cell_off);
    asm.push(abi::multiply_registers(
        abi::SCRATCH[0],
        index,
        abi::SCRATCH[0],
    ));
    asm.push(abi::signed_convert_to_float_d(dst, abi::SCRATCH[0]));
}

/// `dst (d-reg) = the cell size (px) at runtime-state field `cell_off`.
fn emit_cell_to_d(asm: &mut Asm, dst: &str, cell_off: usize) {
    asm.load_state(abi::SCRATCH[0], cell_off);
    asm.push(abi::signed_convert_to_float_d(dst, abi::SCRATCH[0]));
}

/// `dst (d-reg) = constant` as a double.
fn emit_const_to_d(asm: &mut Asm, dst: &str, value: usize) {
    asm.push(abi::move_immediate(
        abi::SCRATCH[0],
        "Integer",
        &value.to_string(),
    ));
    asm.push(abi::signed_convert_to_float_d(dst, abi::SCRATCH[0]));
}

/// Fill a 2px-tall rect at the bottom of cell (col,row) in the current source color
/// (used for the underline run and the cursor caret).
fn emit_term_cell_rect(asm: &mut Asm, cr: &str, col: &str, row: &str) {
    asm.push(abi::move_register(abi::c_arg(0), cr));
    emit_cell_dim_to_d(asm, "d0", col, ST_TERM_CELL_W); // x = col*cellW
                                                        // Load cellH before forming row+1 in x9 (load_state clobbers x9).
    asm.load_state(abi::SCRATCH[1], ST_TERM_CELL_H);
    asm.push(abi::add_immediate(abi::SCRATCH[0], row, 1)); // y = (row+1)*cellH - 2
    asm.push(abi::multiply_registers(
        abi::SCRATCH[0],
        abi::SCRATCH[0],
        abi::SCRATCH[1],
    ));
    asm.push(abi::subtract_immediate(abi::SCRATCH[0], abi::SCRATCH[0], 2));
    asm.push(abi::signed_convert_to_float_d(
        abi::FP_SCRATCH[1],
        abi::SCRATCH[0],
    ));
    emit_cell_to_d(asm, "d2", ST_TERM_CELL_W); // w
    emit_const_to_d(asm, "d3", 2); // h
    asm.call_external("cairo_rectangle");
    asm.push(abi::move_register(abi::c_arg(0), cr));
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
    asm.push(abi::store_u64(abi::LOCAL[0], abi::stack_pointer(), 8));
    asm.load_state(abi::LOCAL[0], ST_TERM_ROWS);
    asm.push(abi::subtract_immediate(abi::LOCAL[0], abi::LOCAL[0], 1)); // rows-1
    asm.push(abi::move_immediate(
        abi::SCRATCH[0],
        "Integer",
        &TERM_MAX_COLS.to_string(),
    ));
    asm.push(abi::multiply_registers(
        abi::LOCAL[0],
        abi::LOCAL[0],
        abi::SCRATCH[0],
    )); // cells = (rows-1)*MAX_COLS
        // memmove each array up one (fixed-stride) row: 4B per cell for all three
        // (chars became u32 in bug-203, matching fg/bg).
        // plan-70-E Phase 3: the EGC pool (GTK_POOL_BYTES=32=1<<5 per cell) shifts with
        // the char/fg/bg arrays so a scrolled pooled cluster keeps its slot.
    for (base, shift) in [
        (ST_TERM_CHARS, 2u8),
        (ST_TERM_FG, 2),
        (ST_TERM_BG, 2),
        (ST_TERM_POOL, 5),
    ] {
        asm.state_array(abi::c_arg(0), base); // dst = row 0
        asm.state_array(abi::c_arg(1), base + TERM_MAX_COLS * (1 << shift)); // src = row 1
        asm.push(abi::shift_left_immediate(
            abi::c_arg(2),
            abi::LOCAL[0],
            shift,
        )); // cells * elemSize
        asm.call_external("memmove");
    }
    // Blank the last active row (offset = cells*4): all three arrays to 0. chars
    // clears to 0 rather than ' ' — `memset` writes whole bytes, so ' ' over u32
    // cells would pack FOUR spaces per cell; the draw skips 0 (bug-203).
    for (base, shift, rowbytes) in [
        (ST_TERM_CHARS, 2u8, TERM_MAX_COLS * 4),
        (ST_TERM_FG, 2, TERM_MAX_COLS * 4),
        (ST_TERM_BG, 2, TERM_MAX_COLS * 4),
        (ST_TERM_POOL, 5, TERM_MAX_COLS * GTK_POOL_BYTES),
    ] {
        asm.state_array(abi::c_arg(0), base);
        asm.push(abi::shift_left_immediate(
            abi::SCRATCH[0],
            abi::LOCAL[0],
            shift,
        )); // cells*elemSize
        asm.push(abi::add_registers(
            abi::c_arg(0),
            abi::c_arg(0),
            abi::SCRATCH[0],
        ));
        asm.push(abi::move_immediate(abi::c_arg(1), "Integer", "0"));
        asm.push(abi::move_immediate(
            abi::c_arg(2),
            "Integer",
            &rowbytes.to_string(),
        ));
        asm.call_external("memset");
    }
    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::load_u64(abi::LOCAL[0], abi::stack_pointer(), 8));
    asm.push(abi::add_stack(16));
    asm.push(abi::return_());
    asm.finish(TERM_SCROLL_SYMBOL, "Nothing")
}

/// `void _mfb_gtkapp_term_init(void)` — derive the grid geometry (main thread):
/// measure the monospace cell from Cairo font extents (via a throwaway 8x8 image
/// surface), then cols = floor(W/cellW), rows = floor(H/cellH) clamped to the
/// backing-store bounds, and blank the char grid. Mirrors the macOS term_init,
/// which sizes cols/rows from the font's advance + line height and the view frame.
pub(super) fn emit_term_init_helper() -> Result<CodeFunction, String> {
    let mut asm = Asm::new(TERM_INIT_SYMBOL);
    // lr@0, x19(cr)@8, x20(surf)@16, extents buffer@24 (48B, fits both font_extents
    // and the larger text_extents; plan-70-E also holds Pango's 16B PangoRectangle).
    // cr/surf are callee-saved so they survive the cairo calls. plan-70-E:
    // layout@72, desc@80 hold the throwaway Pango layout + font description used to
    // measure the cell from the same font that draws it.
    let frame = 96;
    let fe = 24usize;
    let (off_layout, off_desc) = (72usize, 80usize);
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(frame));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        0,
    ));
    asm.push(abi::store_u64(abi::LOCAL[0], abi::stack_pointer(), 8));
    asm.push(abi::store_u64(abi::LOCAL[1], abi::stack_pointer(), 16));
    // surf = cairo_image_surface_create(CAIRO_FORMAT_ARGB32=0, 8, 8); cr = create(surf)
    asm.push(abi::move_immediate(abi::c_arg(0), "Integer", "0"));
    asm.push(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
    asm.push(abi::move_immediate(abi::c_arg(2), "Integer", "8"));
    asm.call_external("cairo_image_surface_create");
    asm.push(abi::move_register(abi::LOCAL[1], abi::c_return(0))); // surf
    asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[1]));
    asm.call_external("cairo_create");
    asm.push(abi::move_register(abi::LOCAL[0], abi::c_return(0))); // cr
                                                                   // plan-70-E: measure the cell from the SAME Pango monospace font that draws it
                                                                   // (so the grid geometry matches the rendered glyphs). desc = "monospace 16";
                                                                   // layout of "M"; its logical PangoRectangle gives {width, height} in pixels.
    asm.local_address(abi::c_arg(0), STR_MONO_DESC.0);
    asm.call_external("pango_font_description_from_string");
    asm.push(abi::store_u64(
        abi::c_return(0),
        abi::stack_pointer(),
        off_desc,
    ));
    asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[0]));
    asm.call_external("pango_cairo_create_layout");
    asm.push(abi::store_u64(
        abi::c_return(0),
        abi::stack_pointer(),
        off_layout,
    ));
    asm.push(abi::load_u64(abi::c_arg(1), abi::stack_pointer(), off_desc));
    asm.push(abi::move_register(abi::c_arg(0), abi::c_return(0)));
    asm.call_external("pango_layout_set_font_description");
    asm.push(abi::load_u64(
        abi::c_arg(0),
        abi::stack_pointer(),
        off_layout,
    ));
    asm.local_address(abi::c_arg(1), STR_M.0);
    asm.push(abi::move_immediate(abi::c_arg(2), "Integer", "1")); // "M" is 1 byte
    asm.call_external("pango_layout_set_text");
    // pango_layout_get_pixel_extents(layout, ink=NULL, logical=&rect@fe). The
    // PangoRectangle is {x@0, y@4, width@8, height@12} (i32 each).
    asm.push(abi::load_u64(
        abi::c_arg(0),
        abi::stack_pointer(),
        off_layout,
    ));
    asm.push(abi::move_immediate(abi::c_arg(1), "Integer", "0"));
    asm.push(abi::add_immediate(abi::c_arg(2), abi::stack_pointer(), fe));
    asm.call_external("pango_layout_get_pixel_extents");
    asm.push(abi::load_u32(abi::SCRATCH[1], abi::stack_pointer(), fe + 8)); // logical.width
    emit_clamp_low(&mut asm, abi::SCRATCH[1], 1, "cw");
    asm.store_state(abi::SCRATCH[1], ST_TERM_CELL_W);
    asm.push(abi::load_u32(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        fe + 12,
    )); // logical.height
    emit_clamp_low(&mut asm, abi::SCRATCH[1], 1, "ch");
    asm.store_state(abi::SCRATCH[1], ST_TERM_CELL_H);
    // Pango cleanup before the cairo teardown (the layout holds a ref on cr).
    asm.push(abi::load_u64(
        abi::c_arg(0),
        abi::stack_pointer(),
        off_layout,
    ));
    asm.call_external("g_object_unref");
    asm.push(abi::load_u64(abi::c_arg(0), abi::stack_pointer(), off_desc));
    asm.call_external("pango_font_description_free");
    // cleanup
    asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[0]));
    asm.call_external("cairo_destroy");
    asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[1]));
    asm.call_external("cairo_surface_destroy");
    // cols = clamp(AREA_W / cell_w, 1, MAX_COLS); rows likewise.
    asm.load_state(abi::SCRATCH[1], ST_TERM_CELL_W);
    asm.push(abi::move_immediate(
        abi::SCRATCH[0],
        "Integer",
        &TERM_AREA_W.to_string(),
    ));
    asm.push(abi::unsigned_divide_registers(
        abi::SCRATCH[2],
        abi::SCRATCH[0],
        abi::SCRATCH[1],
    ));
    emit_clamp_range(&mut asm, abi::SCRATCH[2], 1, TERM_MAX_COLS, "cols");
    asm.store_state(abi::SCRATCH[2], ST_TERM_COLS);
    asm.load_state(abi::SCRATCH[1], ST_TERM_CELL_H);
    asm.push(abi::move_immediate(
        abi::SCRATCH[0],
        "Integer",
        &TERM_AREA_H.to_string(),
    ));
    asm.push(abi::unsigned_divide_registers(
        abi::SCRATCH[2],
        abi::SCRATCH[0],
        abi::SCRATCH[1],
    ));
    emit_clamp_range(&mut asm, abi::SCRATCH[2], 1, TERM_MAX_ROWS, "rows");
    asm.store_state(abi::SCRATCH[2], ST_TERM_ROWS);
    // Blank the whole char backing store (fg/bg stay 0 = defaults). Cells clear
    // to 0, not ' ' — see the scroll blank (bug-203).
    asm.state_array(abi::c_arg(0), ST_TERM_CHARS);
    asm.push(abi::move_immediate(abi::c_arg(1), "Integer", "0"));
    asm.push(abi::move_immediate(
        abi::c_arg(2),
        "Integer",
        &(TERM_MAX_COLS * TERM_MAX_ROWS * 4).to_string(),
    ));
    asm.call_external("memset");
    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::load_u64(abi::LOCAL[0], abi::stack_pointer(), 8));
    asm.push(abi::load_u64(abi::LOCAL[1], abi::stack_pointer(), 16));
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
    asm.load_state(abi::c_arg(0), ST_WINDOW);
    asm.load_state(abi::c_arg(1), ST_TERM_AREA);
    asm.call_external("gtk_window_set_child");
    // Present the initial (cleared) grid: snapshot the live arrays before drawing so
    // the first frame after `term::on` matches the live grid (plan-35-E).
    emit_term_snapshot_copy(&mut asm);
    asm.load_state(abi::c_arg(0), ST_TERM_AREA);
    asm.call_external("gtk_widget_queue_draw");
    asm.push(abi::move_immediate(abi::c_return(0), "Boolean", FALSE)); // G_SOURCE_REMOVE
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
    asm.load_state(abi::c_arg(0), ST_WINDOW);
    asm.load_state(abi::c_arg(1), ST_SCROLLED);
    asm.call_external("gtk_window_set_child");
    asm.push(abi::move_immediate(abi::c_return(0), "Boolean", FALSE));
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
    asm.state_array(abi::c_arg(0), ST_TERM_SNAP_CHARS);
    asm.state_array(abi::c_arg(1), ST_TERM_CHARS);
    asm.push(abi::move_immediate(
        abi::c_arg(2),
        "Integer",
        &(TERM_MAX_COLS * TERM_MAX_ROWS * 4).to_string(),
    ));
    asm.call_external("memcpy");
    // fg / bg: 4 bytes/cell (packed RGB | flags — copied verbatim).
    for (snap, live) in [(ST_TERM_SNAP_FG, ST_TERM_FG), (ST_TERM_SNAP_BG, ST_TERM_BG)] {
        asm.state_array(abi::c_arg(0), snap);
        asm.state_array(abi::c_arg(1), live);
        asm.push(abi::move_immediate(
            abi::c_arg(2),
            "Integer",
            &(TERM_MAX_COLS * TERM_MAX_ROWS * 4).to_string(),
        ));
        asm.call_external("memcpy");
    }
    // plan-70-E Phase 3: the EGC pool (GTK_POOL_BYTES/cell) rides the same snapshot so
    // the draw callback rebuilds pooled clusters from a consistent frame.
    asm.state_array(abi::c_arg(0), ST_TERM_SNAP_POOL);
    asm.state_array(abi::c_arg(1), ST_TERM_POOL);
    asm.push(abi::move_immediate(
        abi::c_arg(2),
        "Integer",
        &(TERM_MAX_COLS * TERM_MAX_ROWS * GTK_POOL_BYTES).to_string(),
    ));
    asm.call_external("memcpy");
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
    asm.load_state(abi::c_arg(0), ST_TERM_AREA);
    asm.call_external("gtk_widget_queue_draw");
    asm.push(abi::move_immediate(abi::c_return(0), "Boolean", FALSE));
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
    asm.push(abi::move_register(abi::SCRATCH[2], abi::c_arg(1))); // width
    asm.push(abi::move_register(abi::SCRATCH[3], abi::c_arg(2))); // height
                                                                  // cols = clamp(width / cell_w, 1, MAX_COLS).
    asm.load_state(abi::SCRATCH[1], ST_TERM_CELL_W);
    asm.push(abi::unsigned_divide_registers(
        abi::SCRATCH[2],
        abi::SCRATCH[2],
        abi::SCRATCH[1],
    ));
    emit_clamp_range(&mut asm, abi::SCRATCH[2], 1, TERM_MAX_COLS, "rz_cols");
    // rows = clamp(height / cell_h, 1, MAX_ROWS).
    asm.load_state(abi::SCRATCH[1], ST_TERM_CELL_H);
    asm.push(abi::unsigned_divide_registers(
        abi::SCRATCH[3],
        abi::SCRATCH[3],
        abi::SCRATCH[1],
    ));
    emit_clamp_range(&mut asm, abi::SCRATCH[3], 1, TERM_MAX_ROWS, "rz_rows");
    // planning/term.md #11: latch the resize flag only on a GENUINE extent change
    // (the "resize" signal also fires at initial allocation, where the freshly
    // clamped cols/rows equal the activate-computed values → no spurious flag).
    // Capture the old extent in x13/x14 before overwriting, compare, set 1 on diff.
    asm.load_state(abi::SCRATCH[4], ST_TERM_COLS); // old cols
    asm.load_state(abi::SCRATCH[5], ST_TERM_ROWS); // old rows
    asm.store_state(abi::SCRATCH[2], ST_TERM_COLS);
    asm.store_state(abi::SCRATCH[3], ST_TERM_ROWS);
    asm.push(abi::compare_registers(abi::SCRATCH[4], abi::SCRATCH[2]));
    asm.push(abi::branch_ne("rz_changed"));
    asm.push(abi::compare_registers(abi::SCRATCH[5], abi::SCRATCH[3]));
    asm.push(abi::branch_eq("rz_nochange"));
    asm.push(abi::label("rz_changed"));
    asm.push(abi::move_immediate(abi::SCRATCH[4], "Integer", "1"));
    asm.store_state(abi::SCRATCH[4], ST_TERM_DID_RESIZE);
    asm.push(abi::label("rz_nochange"));
    // Force a full redraw at the new extent.
    asm.load_state(abi::c_arg(0), ST_TERM_AREA);
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
pub(super) fn emit_term_write_helper(uses_term: bool) -> Result<CodeFunction, String> {
    let mut asm = Asm::new(TERM_WRITE_SYMBOL);
    // lr@0, x20(newline)@8, x21(i)@16, x22(len)@24, x23(ptr)@32, x24(charsBase)@40,
    // x25(row)@48, x26(col)@56, x27(fgBase)@64, x28(bgBase)@72, fgval@80, bgval@88,
    // x19(code-point byte length)@96. plan-70-E: glyph@104, width@112 spill the
    // decoded glyph + display width across the wide-at-edge scroll.
    //
    // The length must be callee-saved: `tw_clamp` can call TERM_SCROLL_SYMBOL
    // between the decode and the `i += len` advance, so a caller-saved scratch
    // would not survive it (bug-203).
    let frame = 128;
    let (off_fgval, off_bgval) = (80usize, 88usize);
    let (off_glyph, off_width) = (104usize, 112usize);
    // plan-70-E Phase 3: byte offset (idx*4) of the last base cell written, or -1 —
    // a following combining mark (width 0) folds into its pool slot.
    let off_lastbase = 120usize;
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(frame));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        0,
    ));
    for (reg, off) in [
        (abi::LOCAL[0], 96),
        (abi::LOCAL[1], 8),
        (abi::LOCAL[2], 16),
        (abi::LOCAL[3], 24),
        (abi::LOCAL[4], 32),
        (abi::LOCAL[5], 40),
        (abi::LOCAL[6], 48),
        (abi::LOCAL[7], 56),
        (abi::LOCAL[8], 64),
        (abi::LOCAL[9], 72),
    ] {
        asm.push(abi::store_u64(reg, abi::stack_pointer(), off));
    }
    asm.push(abi::move_register(abi::LOCAL[1], abi::c_arg(1))); // newline flag
    asm.push(abi::load_u64(abi::LOCAL[3], abi::c_arg(0), 0)); // text len
    asm.push(abi::add_immediate(abi::LOCAL[4], abi::c_arg(0), 8)); // text ptr
    asm.state_array(abi::LOCAL[5], ST_TERM_CHARS);
    asm.state_array(abi::LOCAL[8], ST_TERM_FG);
    asm.state_array(abi::LOCAL[9], ST_TERM_BG);
    asm.load_state(abi::LOCAL[6], ST_TERM_ROW);
    asm.load_state(abi::LOCAL[7], ST_TERM_COL);
    // fgval = cur_fg | (bold ? BOLD_FLAG : 0) | (underline ? UNDERLINE_FLAG : 0).
    // Hold cur_fg in x11 — load_state clobbers x9 as its address scratch.
    asm.load_state(abi::SCRATCH[2], ST_TERM_CUR_FG);
    asm.load_state(abi::SCRATCH[1], ST_TERM_CUR_BOLD);
    asm.push(abi::compare_immediate(abi::SCRATCH[1], "0"));
    asm.push(abi::branch_eq("tw_no_bold"));
    asm.push(abi::move_immediate(
        abi::SCRATCH[0],
        "Integer",
        &BOLD_FLAG.to_string(),
    ));
    asm.push(abi::or_registers(
        abi::SCRATCH[2],
        abi::SCRATCH[2],
        abi::SCRATCH[0],
    ));
    asm.push(abi::label("tw_no_bold"));
    asm.load_state(abi::SCRATCH[1], ST_TERM_CUR_UNDERLINE);
    asm.push(abi::compare_immediate(abi::SCRATCH[1], "0"));
    asm.push(abi::branch_eq("tw_no_ul"));
    asm.push(abi::move_immediate(
        abi::SCRATCH[0],
        "Integer",
        &UNDERLINE_FLAG.to_string(),
    ));
    asm.push(abi::or_registers(
        abi::SCRATCH[2],
        abi::SCRATCH[2],
        abi::SCRATCH[0],
    ));
    asm.push(abi::label("tw_no_ul"));
    asm.push(abi::store_u64(
        abi::SCRATCH[2],
        abi::stack_pointer(),
        off_fgval,
    ));
    asm.load_state(abi::SCRATCH[2], ST_TERM_CUR_BG);
    asm.push(abi::store_u64(
        abi::SCRATCH[2],
        abi::stack_pointer(),
        off_bgval,
    ));
    asm.push(abi::move_immediate(abi::LOCAL[2], "Integer", "0")); // i
                                                                  // plan-70-E Phase 3: last base cell = none (-1) — a combining mark folds into it.
    asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
    asm.push(abi::bitwise_not(abi::SCRATCH[0], abi::SCRATCH[0])); // -1
    asm.push(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_lastbase,
    ));

    asm.push(abi::label("tw_loop"));
    asm.push(abi::compare_registers(abi::LOCAL[2], abi::LOCAL[3]));
    asm.push(abi::branch_ge("tw_after"));
    asm.push(abi::add_registers(
        abi::SCRATCH[0],
        abi::LOCAL[4],
        abi::LOCAL[2],
    ));
    asm.push(abi::load_u8(abi::SCRATCH[1], abi::SCRATCH[0], 0)); // byte = ptr[i]
    asm.push(abi::compare_immediate(abi::SCRATCH[1], "10")); // '\n'
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
    emit_utf8_decode_at(
        &mut asm,
        abi::SCRATCH[1],
        abi::SCRATCH[0],
        abi::LOCAL[0],
        abi::LOCAL[2],
        abi::LOCAL[3],
    );
    // plan-70-E: display width (0/1/2 -> 1/1/2) of this scalar, in x15. Gated on
    // uses_term so a non-term GTK app never references the property table.
    if uses_term {
        emit_gtk_charwidth(
            &mut asm,
            abi::SCRATCH[1],
            abi::LOCAL[0],
            abi::SCRATCH[6],
            abi::SCRATCH[7],
            abi::SCRATCH[8],
            abi::SCRATCH[5],
            "tw_cw",
        );
        // plan-70-E Phase 3: width 0 = combining mark. Fold it into the previous base
        // cell's EGC pool (so Pango composes the cluster), advancing i but not the
        // column. With no base to attach to, fall through as a lone width-1 cell.
        asm.push(abi::compare_immediate(abi::SCRATCH[6], "0"));
        asm.push(abi::branch_ne("tw_not_combine"));
        asm.push(abi::load_u64(
            abi::SCRATCH[7],
            abi::stack_pointer(),
            off_lastbase,
        ));
        asm.push(abi::compare_immediate(abi::SCRATCH[7], "0"));
        asm.push(abi::branch_lt("tw_lone_combine")); // -1 => no base
        emit_gtk_pool_append(
            &mut asm,
            abi::SCRATCH[7],
            abi::SCRATCH[1],
            abi::LOCAL[0],
            "tw_pa",
        );
        asm.push(abi::branch("tw_next")); // fold complete; advance i, keep the column
        asm.push(abi::label("tw_lone_combine"));
        asm.push(abi::move_immediate(abi::SCRATCH[6], "Integer", "1"));
        asm.push(abi::label("tw_not_combine"));
    } else {
        asm.push(abi::move_immediate(abi::SCRATCH[6], "Integer", "1"));
    }
    // Wide-at-edge: a width-2 glyph that would straddle the right edge wraps to the
    // next row first (spill glyph+width across the scroll, then reload).
    asm.push(abi::compare_immediate(abi::SCRATCH[6], "2"));
    asm.push(abi::branch_ne("tw_edge_ok"));
    asm.push(abi::add_immediate(abi::SCRATCH[0], abi::LOCAL[7], 1));
    asm.load_state(abi::SCRATCH[2], ST_TERM_COLS);
    asm.push(abi::compare_registers(abi::SCRATCH[0], abi::SCRATCH[2]));
    asm.push(abi::branch_lt("tw_edge_ok"));
    asm.push(abi::store_u64(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        off_glyph,
    ));
    asm.push(abi::store_u64(
        abi::SCRATCH[6],
        abi::stack_pointer(),
        off_width,
    ));
    asm.push(abi::move_immediate(abi::LOCAL[7], "Integer", "0"));
    asm.push(abi::add_immediate(abi::LOCAL[6], abi::LOCAL[6], 1));
    asm.load_state(abi::SCRATCH[0], ST_TERM_ROWS);
    asm.push(abi::compare_registers(abi::LOCAL[6], abi::SCRATCH[0]));
    asm.push(abi::branch_lt("tw_edge_noscroll"));
    asm.call_internal(TERM_SCROLL_SYMBOL);
    asm.load_state(abi::LOCAL[6], ST_TERM_ROWS);
    asm.push(abi::subtract_immediate(abi::LOCAL[6], abi::LOCAL[6], 1));
    asm.push(abi::label("tw_edge_noscroll"));
    asm.push(abi::load_u64(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        off_glyph,
    ));
    asm.push(abi::load_u64(
        abi::SCRATCH[6],
        abi::stack_pointer(),
        off_width,
    ));
    asm.push(abi::label("tw_edge_ok"));
    // idx = row*MAX_COLS + col; chars[idx]=glyph; fg[idx]=fgval|(width<<27); bg=bgval.
    asm.push(abi::move_immediate(
        abi::SCRATCH[2],
        "Integer",
        &TERM_MAX_COLS.to_string(),
    ));
    asm.push(abi::multiply_registers(
        abi::SCRATCH[3],
        abi::LOCAL[6],
        abi::SCRATCH[2],
    ));
    asm.push(abi::add_registers(
        abi::SCRATCH[3],
        abi::SCRATCH[3],
        abi::LOCAL[7],
    )); // idx
    asm.push(abi::shift_left_immediate(
        abi::SCRATCH[4],
        abi::SCRATCH[3],
        2,
    )); // idx*4
    asm.push(abi::add_registers(
        abi::SCRATCH[0],
        abi::LOCAL[5],
        abi::SCRATCH[4],
    ));
    asm.push(abi::store_u32(abi::SCRATCH[1], abi::SCRATCH[0], 0));
    asm.push(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_fgval,
    ));
    asm.push(abi::shift_left_immediate(
        abi::SCRATCH[2],
        abi::SCRATCH[6],
        WIDTH_SHIFT as u8,
    )); // width<<27
    asm.push(abi::or_registers(
        abi::SCRATCH[0],
        abi::SCRATCH[0],
        abi::SCRATCH[2],
    ));
    asm.push(abi::add_registers(
        abi::SCRATCH[5],
        abi::LOCAL[8],
        abi::SCRATCH[4],
    ));
    asm.push(abi::store_u32(abi::SCRATCH[0], abi::SCRATCH[5], 0));
    asm.push(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_bgval,
    ));
    asm.push(abi::add_registers(
        abi::SCRATCH[5],
        abi::LOCAL[9],
        abi::SCRATCH[4],
    ));
    asm.push(abi::store_u32(abi::SCRATCH[0], abi::SCRATCH[5], 0));
    // plan-70-E Phase 3: this cell is now the base a following combining mark folds
    // into (x13 = idx*4, the CHAR/pool byte offset).
    asm.push(abi::store_u64(
        abi::SCRATCH[4],
        abi::stack_pointer(),
        off_lastbase,
    ));
    // Wide (width 2): the next cell is a WIDE_TRAIL sentinel (col+1 is on the grid
    // after the wide-at-edge wrap above). char[idx+1]=0xFFFFFFFF, fg/bg copied, width 0.
    asm.push(abi::compare_immediate(abi::SCRATCH[6], "2"));
    asm.push(abi::branch_ne("tw_no_trail"));
    asm.push(abi::add_registers(
        abi::SCRATCH[0],
        abi::LOCAL[5],
        abi::SCRATCH[4],
    ));
    asm.push(abi::move_immediate(
        abi::SCRATCH[2],
        "Integer",
        GTK_WIDE_TRAIL,
    ));
    asm.push(abi::store_u32(abi::SCRATCH[2], abi::SCRATCH[0], 4));
    asm.push(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_fgval,
    ));
    asm.push(abi::add_registers(
        abi::SCRATCH[5],
        abi::LOCAL[8],
        abi::SCRATCH[4],
    ));
    asm.push(abi::store_u32(abi::SCRATCH[0], abi::SCRATCH[5], 4));
    asm.push(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_bgval,
    ));
    asm.push(abi::add_registers(
        abi::SCRATCH[5],
        abi::LOCAL[9],
        abi::SCRATCH[4],
    ));
    asm.push(abi::store_u32(abi::SCRATCH[0], abi::SCRATCH[5], 4));
    asm.push(abi::label("tw_no_trail"));
    // col += width; wrap to next row at the active cols.
    asm.push(abi::add_registers(
        abi::LOCAL[7],
        abi::LOCAL[7],
        abi::SCRATCH[6],
    ));
    asm.load_state(abi::SCRATCH[0], ST_TERM_COLS);
    asm.push(abi::compare_registers(abi::LOCAL[7], abi::SCRATCH[0]));
    asm.push(abi::branch_lt("tw_next"));
    asm.push(abi::move_immediate(abi::LOCAL[7], "Integer", "0"));
    asm.push(abi::add_immediate(abi::LOCAL[6], abi::LOCAL[6], 1));
    asm.push(abi::branch("tw_clamp"));
    asm.push(abi::label("tw_newline"));
    asm.push(abi::move_immediate(abi::LOCAL[0], "Integer", "1")); // '\n' is one byte
    asm.push(abi::move_immediate(abi::LOCAL[7], "Integer", "0"));
    asm.push(abi::add_immediate(abi::LOCAL[6], abi::LOCAL[6], 1));
    asm.push(abi::label("tw_clamp"));
    // Scroll the grid up when the cursor passes the bottom (matches macOS).
    asm.load_state(abi::SCRATCH[0], ST_TERM_ROWS);
    asm.push(abi::compare_registers(abi::LOCAL[6], abi::SCRATCH[0]));
    asm.push(abi::branch_lt("tw_next"));
    asm.call_internal(TERM_SCROLL_SYMBOL);
    asm.load_state(abi::LOCAL[6], ST_TERM_ROWS);
    asm.push(abi::subtract_immediate(abi::LOCAL[6], abi::LOCAL[6], 1));
    asm.push(abi::label("tw_next"));
    // Advance by the code point's byte length (x19), set to 1 on the '\n' path
    // below. The cursor moved one column per glyph above (bug-203).
    asm.push(abi::add_registers(
        abi::LOCAL[2],
        abi::LOCAL[2],
        abi::LOCAL[0],
    ));
    asm.push(abi::branch("tw_loop"));

    asm.push(abi::label("tw_after"));
    // print's trailing newline.
    asm.push(abi::compare_immediate(abi::LOCAL[1], "0"));
    asm.push(abi::branch_eq("tw_store"));
    asm.push(abi::move_immediate(abi::LOCAL[7], "Integer", "0"));
    asm.push(abi::add_immediate(abi::LOCAL[6], abi::LOCAL[6], 1));
    asm.load_state(abi::SCRATCH[0], ST_TERM_ROWS);
    asm.push(abi::compare_registers(abi::LOCAL[6], abi::SCRATCH[0]));
    asm.push(abi::branch_lt("tw_store"));
    asm.call_internal(TERM_SCROLL_SYMBOL);
    asm.load_state(abi::LOCAL[6], ST_TERM_ROWS);
    asm.push(abi::subtract_immediate(abi::LOCAL[6], abi::LOCAL[6], 1));
    asm.push(abi::label("tw_store"));
    asm.store_state(abi::LOCAL[6], ST_TERM_ROW);
    asm.store_state(abi::LOCAL[7], ST_TERM_COL);
    // plan-35-E: NO per-write redraw. Writing only mutates the live grid; a present
    // (`term::sync`/`io::flush`/`term::off`) snapshots + queue_draws on the main loop.

    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    for (reg, off) in [
        (abi::LOCAL[0], 96),
        (abi::LOCAL[1], 8),
        (abi::LOCAL[2], 16),
        (abi::LOCAL[3], 24),
        (abi::LOCAL[4], 32),
        (abi::LOCAL[5], 40),
        (abi::LOCAL[6], 48),
        (abi::LOCAL[7], 56),
        (abi::LOCAL[8], 64),
        (abi::LOCAL[9], 72),
    ] {
        asm.push(abi::load_u64(reg, abi::stack_pointer(), off));
    }
    asm.push(abi::add_stack(frame));
    asm.push(abi::return_());
    asm.finish(TERM_WRITE_SYMBOL, "Nothing")
}
