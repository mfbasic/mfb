//! The SGR mouse-report decoder (plan-94-B §3a).
//!
//! The grammar recognised is the 1006 "SGR extended" form and nothing else:
//!
//! ```text
//! ESC [ < b ; x ; y M      press
//! ESC [ < b ; x ; y m      release
//! ```
//!
//! **1006 is not one option among several — it is the only one that can work
//! here.** The older X10/1005 encodings put each coordinate in a single byte
//! biased by 32, capping them at 223. That is survivable for terminal cells and
//! useless for the canvas surface plan-94-C/D/E inject pixel coordinates through.
//! The 1006 form writes coordinates as plain decimal integers with no ceiling, so
//! one decoder serves both units with no private escape hatch.
//!
//! # The contract, and why it has two entry points
//!
//! This sits on the path every keystroke takes, so the property that matters most
//! is the negative one: **a byte that is not part of a recognised mouse report
//! reaches the program unchanged, in order** — including a bare `ESC` and
//! sequences like `ESC [ Z`.
//!
//! That is harder than "return the byte", because deciding whether `ESC` starts a
//! mouse report takes several more bytes, and by the time the answer is "no" the
//! decoder is holding bytes the program is owed but can only be handed back one
//! per read. So the buffer doubles as a replay queue and
//! [`MOUSE_STATE_DRAIN_POS_OFFSET`] walks it:
//!
//! - [`emit_drain_pending`] — hand back the next byte the program is owed, if any.
//! - [`emit_decode_byte`] — classify one freshly-read byte.
//!
//! and the reader loops:
//!
//! ```text
//! loop {
//!     if drain_pending() -> byte  { return byte }      // owed bytes first
//!     byte = read_one_from_os()                        // then, and only then, read
//!     match decode(byte) {
//!         Pass(b)  => return b,
//!         Buffered => continue,                        // might still become a report
//!         Event    => continue,                        // consumed; it was one
//!     }
//! }
//! ```
//!
//! **A fresh byte is never read while bytes are owed** — that ordering is the
//! whole contract, and it is why `decode` may assume it is not mid-drain.

use crate::codegen::engine::builder::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::target::shared::abi;

use super::ring::{emit_enqueue, MouseEventRegs};

/// The byte is not part of a mouse report; deliver it.
pub(crate) const DECODE_PASS: u64 = 0;
/// The byte was buffered as part of a possible report; deliver nothing.
pub(crate) const DECODE_BUFFERED: u64 = 1;
/// A complete report was decoded and enqueued; deliver nothing, read on.
pub(crate) const DECODE_EVENT: u64 = 2;

/// SGR button-code bits (`b` in `ESC [ < b ; x ; y M`).
const SGR_BUTTON_MASK: u64 = 0b11;
const SGR_MOTION_BIT: u64 = 32;
const SGR_SHIFT_BIT: u64 = 4;
const SGR_ALT_BIT: u64 = 8;
const SGR_CTRL_BIT: u64 = 16;
const SGR_WHEEL_UP: u64 = 64;
const SGR_WHEEL_DOWN: u64 = 65;
/// The low-two-bits value meaning "no button" — what a bare motion report carries.
const SGR_BUTTON_NONE: u64 = 3;

/// `branch if lhs >= rhs`, unsigned.
///
/// The instruction vocabulary has `lo`/`ls`/`hi` but no `hs`, so this compares the
/// operands the other way round: `rhs <= lhs` is the same predicate. Spelled once
/// here because getting it backwards is a silent off-by-one in a loop bound.
fn branch_unsigned_ge(lhs: &str, rhs: &str, target: &str, ctx: &mut EmitCtx) {
    ctx.instructions.push(abi::compare_registers(rhs, lhs));
    ctx.instructions.push(abi::branch_ls(target));
}

/// Hand back the next byte the program is owed from a flushed prefix.
///
/// Sets `out_have` to 1 and `out_byte` to that byte, or `out_have` to 0 when
/// nothing is owed. Resets the buffer once the last owed byte is handed over, so
/// the decoder returns to its resting state.
///
/// The cursor is one-based ([`MOUSE_STATE_DRAIN_POS_OFFSET`]): `0` is "nothing
/// owed", and `n` means the next byte is `buf[n - 1]`.
pub(crate) fn emit_drain_pending(
    out_have: &str,
    out_byte: &str,
    mouse_state_offset: usize,
    ctx: &mut EmitCtx,
    vregs: &mut Vregs,
) {
    let symbol = ctx.symbol;
    let done = format!("{symbol}_mouse_drain_done");
    let reset = format!("{symbol}_mouse_drain_reset");

    let pos = vregs.next();
    let len = vregs.next();
    let index = vregs.next();
    let buf = vregs.next();
    let addr = vregs.next();
    let zero = vregs.next();

    ctx.instructions.extend([
        abi::move_immediate(out_have, "Integer", "0"),
        abi::load_u64(
            &pos,
            ARENA_STATE_REGISTER,
            mouse_state_offset + MOUSE_STATE_DRAIN_POS_OFFSET,
        ),
        abi::compare_immediate(&pos, "0"),
        abi::branch_eq(&done),
        abi::subtract_immediate(&index, &pos, 1),
        abi::load_u64(
            &len,
            ARENA_STATE_REGISTER,
            mouse_state_offset + MOUSE_STATE_PARSE_LEN_OFFSET,
        ),
    ]);
    branch_unsigned_ge(&index, &len, &reset, ctx);

    ctx.instructions.extend([
        abi::add_immediate(
            &buf,
            ARENA_STATE_REGISTER,
            mouse_state_offset + MOUSE_STATE_PARSE_BUF_OFFSET,
        ),
        abi::add_registers(&addr, &buf, &index),
        abi::load_u8(out_byte, &addr, 0),
        abi::add_immediate(&pos, &pos, 1),
        abi::store_u64(
            &pos,
            ARENA_STATE_REGISTER,
            mouse_state_offset + MOUSE_STATE_DRAIN_POS_OFFSET,
        ),
        abi::move_immediate(out_have, "Integer", "1"),
        abi::branch(&done),
    ]);

    // Drained to the end: clear both cursors so the next byte starts fresh.
    ctx.instructions.extend([
        abi::label(&reset),
        abi::move_immediate(&zero, "Integer", "0"),
        abi::store_u64(
            &zero,
            ARENA_STATE_REGISTER,
            mouse_state_offset + MOUSE_STATE_DRAIN_POS_OFFSET,
        ),
        abi::store_u64(
            &zero,
            ARENA_STATE_REGISTER,
            mouse_state_offset + MOUSE_STATE_PARSE_LEN_OFFSET,
        ),
        abi::label(&done),
    ]);
}

/// Put one byte back, undelivered, so the next drain hands it over.
///
/// `io::pollInput` is the reason this exists. It has to answer "will a `readChar`
/// return without blocking", and with mouse reporting on it cannot tell from
/// readiness alone: the pending bytes may be a report the pump will swallow
/// whole, leaving the follow-up `readChar` to block on a terminal that has gone
/// quiet. The only honest way to know is to run the bytes through the decoder —
/// and then the one byte that turns out to be the program's must not be lost.
///
/// Safe to call only when nothing is already owed, which is exactly the state
/// `pollInput` establishes before it reads.
pub(crate) fn emit_pushback_byte(
    byte: &str,
    mouse_state_offset: usize,
    ctx: &mut EmitCtx,
    vregs: &mut Vregs,
) {
    let buf = vregs.next();
    let one = vregs.next();
    ctx.instructions.extend([
        abi::add_immediate(
            &buf,
            ARENA_STATE_REGISTER,
            mouse_state_offset + MOUSE_STATE_PARSE_BUF_OFFSET,
        ),
        abi::store_u8(byte, &buf, 0),
        abi::move_immediate(&one, "Integer", "1"),
        abi::store_u64(
            &one,
            ARENA_STATE_REGISTER,
            mouse_state_offset + MOUSE_STATE_PARSE_LEN_OFFSET,
        ),
        // One-based: `1` means "the next byte owed is `buf[0]`" — the state a
        // zero-based cursor could not distinguish from "nothing owed".
        abi::store_u64(
            &one,
            ARENA_STATE_REGISTER,
            mouse_state_offset + MOUSE_STATE_DRAIN_POS_OFFSET,
        ),
    ]);
}

/// Classify one freshly-read byte.
///
/// `out_action` receives a `DECODE_*` value; `out_byte` receives the byte to
/// deliver when that value is [`DECODE_PASS`]. May only be called when nothing is
/// owed (see the module doc).
pub(crate) fn emit_decode_byte(
    byte: &str,
    out_action: &str,
    out_byte: &str,
    mouse_state_offset: usize,
    clock_scratch: usize,
    ctx: &mut EmitCtx,
    vregs: &mut Vregs,
) -> Result<(), String> {
    let symbol = ctx.symbol;
    let l = |s: &str| format!("{symbol}_mouse_dec_{s}");
    let done = l("done");
    let pass_through = l("pass");
    let buffering = l("buffering");
    let append = l("append");
    let flush_prefix = l("flush");
    let complete = l("complete");
    let at1 = l("at1");
    let at2 = l("at2");
    let body = l("body");

    let buf = vregs.next();
    let len = vregs.next();
    ctx.instructions.extend([
        abi::move_register(out_byte, byte),
        abi::add_immediate(
            &buf,
            ARENA_STATE_REGISTER,
            mouse_state_offset + MOUSE_STATE_PARSE_BUF_OFFSET,
        ),
        abi::load_u64(
            &len,
            ARENA_STATE_REGISTER,
            mouse_state_offset + MOUSE_STATE_PARSE_LEN_OFFSET,
        ),
        abi::compare_immediate(&len, "0"),
        abi::branch_ne(&buffering),
        // Resting state: only ESC can begin a report. Every other byte is the
        // program's, untouched — the fast path, and it is two instructions.
        abi::compare_immediate(byte, "27"),
        abi::branch_eq(&append),
        abi::branch(&pass_through),
    ]);

    // Mid-sequence. Whether this byte can continue a report depends on position:
    //   [0] ESC (already buffered)   [1] '['   [2] '<'
    //   [3..] digits and ';' until the terminating 'M' or 'm'
    ctx.instructions.extend([
        abi::label(&buffering),
        abi::compare_immediate(&len, "1"),
        abi::branch_eq(&at1),
        abi::compare_immediate(&len, "2"),
        abi::branch_eq(&at2),
        abi::branch(&body),
        abi::label(&at1),
        abi::compare_immediate(byte, "91"), // '['
        abi::branch_eq(&append),
        abi::branch(&flush_prefix),
        // A CSI that is not `<` — `ESC [ Z`, a cursor-key report, anything — leaves
        // here and is replayed byte for byte.
        abi::label(&at2),
        abi::compare_immediate(byte, "60"), // '<'
        abi::branch_eq(&append),
        abi::branch(&flush_prefix),
        // Body: digits and ';' accumulate, 'M'/'m' terminate.
        abi::label(&body),
        abi::compare_immediate(byte, "77"), // 'M'
        abi::branch_eq(&complete),
        abi::compare_immediate(byte, "109"), // 'm'
        abi::branch_eq(&complete),
        abi::compare_immediate(byte, "59"), // ';'
        abi::branch_eq(&append),
        abi::compare_immediate(byte, "48"), // '0'
        abi::branch_lo(&flush_prefix),
        abi::compare_immediate(byte, "57"), // '9'
        abi::branch_hi(&flush_prefix),
    ]);

    // Append. A report longer than the buffer cannot be a real one (21 bytes worst
    // case against 32), so a full buffer means the stream is not what it claimed
    // and the prefix is replayed rather than silently truncated.
    let cap = vregs.next();
    let addr = vregs.next();
    let next_len = vregs.next();
    ctx.instructions.push(abi::label(&append));
    ctx.instructions.push(abi::move_immediate(
        &cap,
        "Integer",
        &MOUSE_STATE_PARSE_BUF_BYTES.to_string(),
    ));
    branch_unsigned_ge(&len, &cap, &flush_prefix, ctx);
    ctx.instructions.extend([
        abi::add_registers(&addr, &buf, &len),
        abi::store_u8(byte, &addr, 0),
        abi::add_immediate(&next_len, &len, 1),
        abi::store_u64(
            &next_len,
            ARENA_STATE_REGISTER,
            mouse_state_offset + MOUSE_STATE_PARSE_LEN_OFFSET,
        ),
        abi::move_immediate(out_action, "Integer", &DECODE_BUFFERED.to_string()),
        abi::branch(&done),
    ]);

    // Flush: this was not a mouse report after all. The breaking byte joins the
    // tail (it is the program's too, and it is the byte AFTER the prefix), then
    // `buf[0]` goes back now and the drain cursor takes care of the rest.
    //
    // No shifting: the cursor is why the buffer can be replayed in place.
    let skip_append = l("flush_no_room");
    let start_drain = l("flush_drain");
    let one = vregs.next();
    ctx.instructions.push(abi::label(&flush_prefix));
    ctx.instructions.push(abi::move_immediate(
        &cap,
        "Integer",
        &MOUSE_STATE_PARSE_BUF_BYTES.to_string(),
    ));
    branch_unsigned_ge(&len, &cap, &skip_append, ctx);
    ctx.instructions.extend([
        abi::add_registers(&addr, &buf, &len),
        abi::store_u8(byte, &addr, 0),
        abi::add_immediate(&len, &len, 1),
        abi::store_u64(
            &len,
            ARENA_STATE_REGISTER,
            mouse_state_offset + MOUSE_STATE_PARSE_LEN_OFFSET,
        ),
        abi::branch(&start_drain),
        abi::label(&skip_append),
        abi::label(&start_drain),
        // Deliver buf[0] now, and point the cursor at buf[1] for the next read.
        // One-based, so "next is buf[1]" is 2.
        abi::load_u8(out_byte, &buf, 0),
        abi::move_immediate(&one, "Integer", "2"),
        abi::store_u64(
            &one,
            ARENA_STATE_REGISTER,
            mouse_state_offset + MOUSE_STATE_DRAIN_POS_OFFSET,
        ),
        abi::move_immediate(out_action, "Integer", &DECODE_PASS.to_string()),
        abi::branch(&done),
    ]);

    // Complete: parse `b;x;y`, enqueue, reset.
    ctx.instructions.push(abi::label(&complete));
    emit_parse_and_enqueue(
        byte,
        &buf,
        &len,
        mouse_state_offset,
        clock_scratch,
        ctx,
        vregs,
    )?;
    let zero = vregs.next();
    ctx.instructions.extend([
        abi::move_immediate(&zero, "Integer", "0"),
        abi::store_u64(
            &zero,
            ARENA_STATE_REGISTER,
            mouse_state_offset + MOUSE_STATE_PARSE_LEN_OFFSET,
        ),
        abi::store_u64(
            &zero,
            ARENA_STATE_REGISTER,
            mouse_state_offset + MOUSE_STATE_DRAIN_POS_OFFSET,
        ),
        abi::move_immediate(out_action, "Integer", &DECODE_EVENT.to_string()),
        abi::branch(&done),
    ]);

    ctx.instructions.extend([
        abi::label(&pass_through),
        abi::move_immediate(out_action, "Integer", &DECODE_PASS.to_string()),
        abi::label(&done),
    ]);
    Ok(())
}

/// Parse the buffered `ESC [ < b ; x ; y` and enqueue what `terminator` describes.
///
/// `terminator` is `'M'` (press) or `'m'` (release) — the byte that completed the
/// report, which is not itself in the buffer.
fn emit_parse_and_enqueue(
    terminator: &str,
    buf: &str,
    len: &str,
    mouse_state_offset: usize,
    clock_scratch: usize,
    ctx: &mut EmitCtx,
    vregs: &mut Vregs,
) -> Result<(), String> {
    let symbol = ctx.symbol;
    let l = |s: &str| format!("{symbol}_mouse_parse_{s}");

    // Three semicolon-separated decimals starting at index 3 (past `ESC [ <`).
    let cursor = vregs.next();
    ctx.instructions
        .push(abi::move_immediate(&cursor, "Integer", "3"));
    let b = emit_parse_decimal(&l("b"), buf, len, &cursor, ctx, vregs);
    let x = emit_parse_decimal(&l("x"), buf, len, &cursor, ctx, vregs);
    let y = emit_parse_decimal(&l("y"), buf, len, &cursor, ctx, vregs);

    // Coordinates are 1-based on the wire, in both units. Subtract before the ring
    // sees them, so every consumer reads the 0-based convention the rest of
    // `term::` and `canvas::` already use.
    ctx.instructions.extend([
        abi::subtract_immediate(&x, &x, 1),
        abi::subtract_immediate(&y, &y, 1),
    ]);

    // Modifiers: SGR bits 2/3/4, repacked into the ring's own bit order so the
    // ring never has to know the SGR encoding.
    let mods = vregs.next();
    let bit = vregs.next();
    let tmp = vregs.next();
    ctx.instructions
        .push(abi::move_immediate(&mods, "Integer", "0"));
    for (sgr_bit, ring_bit) in [
        (SGR_SHIFT_BIT, MOUSE_MOD_SHIFT),
        (SGR_ALT_BIT, MOUSE_MOD_ALT),
        (SGR_CTRL_BIT, MOUSE_MOD_CTRL),
    ] {
        let skip = l(&format!("mod{sgr_bit}"));
        ctx.instructions.extend([
            abi::move_immediate(&bit, "Integer", &sgr_bit.to_string()),
            abi::and_registers(&tmp, &b, &bit),
            abi::compare_immediate(&tmp, "0"),
            abi::branch_eq(&skip),
            abi::move_immediate(&bit, "Integer", &ring_bit.to_string()),
            abi::or_registers(&mods, &mods, &bit),
            abi::label(&skip),
        ]);
    }

    // Kind and button.
    //
    // Order matters: the wheel codes (64/65) reuse the low two bits that would
    // otherwise name a button, so the wheel is tested BEFORE the button is read.
    // Then bit 5 (motion) separates Move from Drag by whether a button is held;
    // only if it is clear does the terminator decide Down vs Up.
    let kind = vregs.next();
    let button = vregs.next();
    let wheel_up = l("wheel_up");
    let wheel_down = l("wheel_down");
    let not_wheel = l("not_wheel");
    let motion = l("motion");
    let press = l("press");
    let plain_move = l("move");
    let kind_done = l("kind_done");
    let no_button = l("no_button");

    ctx.instructions.extend([
        abi::move_immediate(&button, "Integer", &MOUSE_BUTTON_NONE.to_string()),
        abi::move_immediate(&kind, "Integer", &MOUSE_KIND_NONE.to_string()),
        abi::compare_immediate(&b, &SGR_WHEEL_UP.to_string()),
        abi::branch_eq(&wheel_up),
        abi::compare_immediate(&b, &SGR_WHEEL_DOWN.to_string()),
        abi::branch_eq(&wheel_down),
        abi::branch(&not_wheel),
        abi::label(&wheel_up),
        abi::move_immediate(&kind, "Integer", &MOUSE_KIND_SCROLL_UP.to_string()),
        abi::branch(&kind_done),
        abi::label(&wheel_down),
        abi::move_immediate(&kind, "Integer", &MOUSE_KIND_SCROLL_DOWN.to_string()),
        abi::branch(&kind_done),
        abi::label(&not_wheel),
    ]);

    let low = vregs.next();
    let mask = vregs.next();
    ctx.instructions.extend([
        abi::move_immediate(&mask, "Integer", &SGR_BUTTON_MASK.to_string()),
        abi::and_registers(&low, &b, &mask),
        abi::compare_immediate(&low, &SGR_BUTTON_NONE.to_string()),
        abi::branch_eq(&no_button),
        // `MouseButton` is declared None, Left, Middle, Right — so the SGR code
        // plus one IS the ordinal. Checked rather than merely asserted in prose,
        // because this is the one place the enum's declaration order is
        // load-bearing beyond `None = 0`, and reordering the variants in either
        // package's `add_enum` would silently remap every button.
        abi::add_immediate(&button, &low, 1),
        abi::label(&no_button),
    ]);
    const _: () = {
        assert!(MOUSE_BUTTON_LEFT == 0 + 1, "SGR button 0 must map to Left");
        assert!(
            MOUSE_BUTTON_MIDDLE == 1 + 1,
            "SGR button 1 must map to Middle"
        );
        assert!(
            MOUSE_BUTTON_RIGHT == 2 + 1,
            "SGR button 2 must map to Right"
        );
    };

    let bit5 = vregs.next();
    let moved = vregs.next();
    ctx.instructions.extend([
        abi::move_immediate(&bit5, "Integer", &SGR_MOTION_BIT.to_string()),
        abi::and_registers(&moved, &b, &bit5),
        abi::compare_immediate(&moved, "0"),
        abi::branch_ne(&motion),
        abi::compare_immediate(terminator, "77"), // 'M'
        abi::branch_eq(&press),
        abi::move_immediate(&kind, "Integer", &MOUSE_KIND_UP.to_string()),
        abi::branch(&kind_done),
        abi::label(&press),
        abi::move_immediate(&kind, "Integer", &MOUSE_KIND_DOWN.to_string()),
        abi::branch(&kind_done),
        abi::label(&motion),
        abi::compare_immediate(&button, &MOUSE_BUTTON_NONE.to_string()),
        abi::branch_eq(&plain_move),
        abi::move_immediate(&kind, "Integer", &MOUSE_KIND_DRAG.to_string()),
        abi::branch(&kind_done),
        abi::label(&plain_move),
        abi::move_immediate(&kind, "Integer", &MOUSE_KIND_MOVE.to_string()),
        abi::label(&kind_done),
    ]);

    // The wire order is `b;x;y` where x is the COLUMN and y is the ROW. The ring's
    // `coord_a`/`coord_b` are row-then-column in cells and x-then-y in pixels,
    // matching `term::MouseEvent { row, column }` and `canvas::Point { x, y }`
    // respectively — so in cells the pair swaps here and in pixels it does not.
    // Both consumers read `coord_a` first, so the swap belongs in exactly one
    // place: here, where the wire order is known.
    //
    // For pixels the producer (a backend's mouse handler, plan-94-C/D/E) formats
    // `x` into the wire's x field, so the same swap puts `y` in `coord_a` — which
    // is why those backends must read `coord_a` as y. That asymmetry is recorded
    // on `MOUSE_SLOT_COORD_A_OFFSET`.
    emit_enqueue(
        &MouseEventRegs {
            kind: &kind,
            button: &button,
            coord_a: &y,
            coord_b: &x,
            mods: &mods,
        },
        mouse_state_offset,
        clock_scratch,
        ctx,
        vregs,
    )
}

/// Parse one decimal field out of `buf` starting at `cursor`, advancing `cursor`
/// past the field and its separator. Returns the value's vreg.
fn emit_parse_decimal(
    label_base: &str,
    buf: &str,
    len: &str,
    cursor: &str,
    ctx: &mut EmitCtx,
    vregs: &mut Vregs,
) -> String {
    let loop_head = format!("{label_base}_loop");
    let loop_done = format!("{label_base}_done");
    let value = vregs.next();
    let addr = vregs.next();
    let digit = vregs.next();
    let ten = vregs.next();

    ctx.instructions.extend([
        abi::move_immediate(&value, "Integer", "0"),
        abi::move_immediate(&ten, "Integer", "10"),
        abi::label(&loop_head),
    ]);
    branch_unsigned_ge(cursor, len, &loop_done, ctx);
    ctx.instructions.extend([
        abi::add_registers(&addr, buf, cursor),
        abi::load_u8(&digit, &addr, 0),
        abi::compare_immediate(&digit, "48"), // '0'
        abi::branch_lo(&loop_done),
        abi::compare_immediate(&digit, "57"), // '9'
        abi::branch_hi(&loop_done),
        abi::subtract_immediate(&digit, &digit, 48),
        abi::multiply_registers(&value, &value, &ten),
        abi::add_registers(&value, &value, &digit),
        abi::add_immediate(cursor, cursor, 1),
        abi::branch(&loop_head),
        abi::label(&loop_done),
        // Step past the ';'. Past the end is harmless: the next field's loop
        // re-tests the bound before it reads anything.
        abi::add_immediate(cursor, cursor, 1),
    ]);
    value
}
