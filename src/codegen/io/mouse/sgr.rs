//! Formatting an SGR mouse report, for the app backends to inject
//! (plan-94-C/D/E).
//!
//! The backends do not build their own event queues. Each converts a native
//! mouse event to surface coordinates, formats the report a terminal would have
//! sent, and writes those bytes into the window input pipe the backend already
//! uses for keystrokes — where plan-94-B's decoder, sitting at the single stdin
//! read choke point, picks them up. One decoder, one ring, three backends that
//! only have to know how to spell an event.
//!
//! This module is the spelling. It is deliberately **pure arithmetic and stores**
//! — no external calls, no platform branches — so the same emitter serves the
//! macOS ObjC IMPs, the GTK controller callbacks and the Win32 `WndProc` arms,
//! each of which then writes the bytes with its own platform's call.
//!
//! **It names physical registers, not vregs**, because every one of its three
//! callers is a hand-written backend body the vreg allocator never sees — an
//! emitted `%v0` reaches the assembler verbatim and is rejected ("unknown AArch64
//! register '%v0'"). The caller supplies the scratch set it knows is free.
//!
//! The wire form is the 1006 "SGR extended" one:
//!
//! ```text
//! ESC [ < b ; x ; y M      press
//! ESC [ < b ; x ; y m      release
//! ```
//!
//! with `x`/`y` **one-based**, matching what a real terminal sends and what
//! `decode.rs` expects. Coordinates are plain decimals with no ceiling, which is
//! what lets a canvas backend put a 1920-pixel x through the identical path a
//! terminal's 80-column one takes.

use crate::codegen::engine::types::CodeInstruction;
use crate::target::shared::abi;

/// Bytes a caller must reserve for [`emit_format_report`]'s output.
///
/// Worst case is `ESC [ < 255 ; 99999 ; 99999 M` — 21 bytes. 32 is the next
/// power of two and leaves the buffer aligned without arithmetic at the call
/// site.
pub(crate) const SGR_REPORT_BYTES: usize = 32;

/// Bytes of stack scratch [`emit_format_report`] uses beyond the report buffer,
/// for the reversed digits of one field.
///
/// A `u64` is at most 20 decimal digits; 32 keeps the buffer aligned and the
/// arithmetic trivial.
pub(crate) const SGR_SCRATCH_BYTES: usize = 32;

/// Scratch registers the formatter may clobber.
///
/// All seven are dead on return and none is live across a call — the formatter
/// makes none. Named by role rather than numbered so a caller reading its own
/// register budget can see what each is for.
pub(crate) struct SgrScratch<'a> {
    /// The value being divided down, digit by digit.
    pub(crate) value: &'a str,
    /// Holds the constant 10.
    pub(crate) ten: &'a str,
    /// The quotient of each division step.
    pub(crate) quotient: &'a str,
    /// One digit's byte.
    pub(crate) digit: &'a str,
    /// How many digits the current field has produced.
    pub(crate) count: &'a str,
    /// A computed byte address.
    pub(crate) addr: &'a str,
    /// The base of the reversed-digit scratch area.
    pub(crate) scratch_base: &'a str,
}

/// Append `value_reg` to `buf` at `cursor` as unsigned decimal, advancing
/// `cursor`.
///
/// Digits come out least-significant-first, so they are written backwards into a
/// scratch area and then copied forward. That is two short loops rather than one
/// long one, and it avoids having to count the digits before starting — which is
/// the thing that makes the forward version awkward.
#[allow(clippy::too_many_arguments)]
fn emit_decimal(
    label_base: &str,
    value_reg: &str,
    buf: &str,
    cursor: &str,
    scratch_offset: usize,
    s: &SgrScratch,
    ins: &mut Vec<CodeInstruction>,
) {
    let split = format!("{label_base}_split");
    let copy = format!("{label_base}_copy");
    let copy_done = format!("{label_base}_copy_done");

    ins.extend([
        abi::move_register(s.value, value_reg),
        abi::move_immediate(s.ten, "Integer", "10"),
        abi::move_immediate(s.count, "Integer", "0"),
        abi::add_immediate(s.scratch_base, abi::stack_pointer(), scratch_offset),
        // A do-while, so a value of 0 still produces one digit rather than an
        // empty field — `ESC[<0;1;1M` is a real report and `ESC[<;1;1M` is not.
        abi::label(&split),
        abi::unsigned_divide_registers(s.quotient, s.value, s.ten),
        abi::multiply_subtract_registers(s.digit, s.quotient, s.ten, s.value),
        abi::add_immediate(s.digit, s.digit, 48),
        abi::add_registers(s.addr, s.scratch_base, s.count),
        abi::store_u8(s.digit, s.addr, 0),
        abi::add_immediate(s.count, s.count, 1),
        abi::move_register(s.value, s.quotient),
        abi::compare_immediate(s.value, "0"),
        abi::branch_ne(&split),
        // Copy back to front into the report buffer.
        abi::label(&copy),
        abi::compare_immediate(s.count, "0"),
        abi::branch_eq(&copy_done),
        abi::subtract_immediate(s.count, s.count, 1),
        abi::add_registers(s.addr, s.scratch_base, s.count),
        abi::load_u8(s.digit, s.addr, 0),
        abi::add_registers(s.addr, buf, cursor),
        abi::store_u8(s.digit, s.addr, 0),
        abi::add_immediate(cursor, cursor, 1),
        abi::branch(&copy),
        abi::label(&copy_done),
    ]);
}

/// Append one literal byte.
fn emit_literal(
    byte: u64,
    buf: &str,
    cursor: &str,
    s: &SgrScratch,
    ins: &mut Vec<CodeInstruction>,
) {
    ins.extend([
        abi::move_immediate(s.digit, "Integer", &byte.to_string()),
        abi::add_registers(s.addr, buf, cursor),
        abi::store_u8(s.digit, s.addr, 0),
        abi::add_immediate(cursor, cursor, 1),
    ]);
}

/// The registers describing the report to format. All hold plain integers.
pub(crate) struct SgrReport<'a> {
    /// The SGR button/motion/modifier code — the `b` field. The caller composes
    /// it from the native event: low two bits the button, bit 5 motion, bits 2/3/4
    /// shift/alt/ctrl, or 64/65 for the wheel.
    pub(crate) button_code: &'a str,
    /// The **one-based** horizontal coordinate: a column in cells, or an x in
    /// pixels.
    pub(crate) x: &'a str,
    /// The **one-based** vertical coordinate.
    pub(crate) y: &'a str,
    /// The terminator byte: `77` (`'M'`, a press) or `109` (`'m'`, a release).
    /// Motion and wheel reports use `'M'`.
    pub(crate) terminator: &'a str,
}

/// Format the report into the buffer at `buf`, leaving its length in `out_len`.
///
/// `scratch_offset` is an sp-relative byte offset of [`SGR_SCRATCH_BYTES`] the
/// caller owns; `buf` is a register holding the address of a
/// [`SGR_REPORT_BYTES`] buffer. `out_len` doubles as the write cursor while the
/// report is built, so it must alias nothing in `report` or `s`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_format_report(
    report: &SgrReport,
    buf: &str,
    out_len: &str,
    scratch_offset: usize,
    label_base: &str,
    s: &SgrScratch,
    ins: &mut Vec<CodeInstruction>,
) {
    let cursor = out_len;
    ins.push(abi::move_immediate(cursor, "Integer", "0"));
    emit_literal(0x1b, buf, cursor, s, ins); // ESC
    emit_literal(b'[' as u64, buf, cursor, s, ins);
    emit_literal(b'<' as u64, buf, cursor, s, ins);
    emit_decimal(
        &format!("{label_base}_b"),
        report.button_code,
        buf,
        cursor,
        scratch_offset,
        s,
        ins,
    );
    emit_literal(b';' as u64, buf, cursor, s, ins);
    emit_decimal(
        &format!("{label_base}_x"),
        report.x,
        buf,
        cursor,
        scratch_offset,
        s,
        ins,
    );
    emit_literal(b';' as u64, buf, cursor, s, ins);
    emit_decimal(
        &format!("{label_base}_y"),
        report.y,
        buf,
        cursor,
        scratch_offset,
        s,
        ins,
    );
    // The terminator is a register, not a literal, because press and release
    // differ only here and every caller has it as a value already.
    ins.extend([
        abi::add_registers(s.addr, buf, cursor),
        abi::store_u8(report.terminator, s.addr, 0),
        abi::add_immediate(cursor, cursor, 1),
    ]);
}
