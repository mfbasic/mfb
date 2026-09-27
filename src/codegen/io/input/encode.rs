//! `_mfb_rt_input_encode_key` — the `--app` key encoder (bug-699 Fix Design
//! layer 3).
//!
//! Every `--app` window key handler (macOS `keyDown:`/`keyUp:`, GTK
//! `key-pressed`/`key-released`, the Windows window procedures) turns one
//! platform key event into the bytes a kitty-protocol terminal sends for it under
//! the flags `term::on` pushes, and writes them into the window input pipe. The
//! input decoder (`super::decode`) then reads an `--app` window and a terminal
//! the same way: one encoding, one decoder, on every surface.
//!
//! The handlers differ only in how they learn the key, so each one gathers its
//! platform's facts and calls this one encoder, which owns the translation:
//!
//! - the platform's code for the key (an AppKit `charactersIgnoringModifiers`
//!   code unit, a GDK keyval, a Win32 virtual-key code) and which platform's
//!   table it belongs to — the tables below map every non-text key to its kitty
//!   number and final byte;
//! - the key's UNSHIFTED code point, for a text key (kitty's key number);
//! - the platform's modifier flags;
//! - the event: press, repeat or release;
//! - the text the key typed, if any (kitty's associated-text field).
//!
//! It is a pure function: no I/O, no allocation, no arena. It runs on the UI
//! thread, which has no arena state.
//!
//! # Interface
//!
//! `c_arg(0)` = the output buffer ([`ENCODE_OUT_BYTES`] bytes: the length at
//! byte 0, the sequence from byte 1); `c_arg(1)` = the native code in the low 32
//! bits and the [`KeyTable`] above them; `c_arg(2)` = the unshifted code point in
//! bits 0–23, the event (1 press, 2 repeat, 3 release) in bits 24–31 and the
//! platform's modifier flags in bits 32–63; `c_arg(3)` = the typed code point,
//! or 0. A length of 0 means the event names nothing a program can receive (a
//! modifier key alone, a key with no code) and nothing is to be written.
//!
//! The length is returned in the buffer, not in a register, because the callers
//! are hand-written handlers on four ABIs whose MFB and C result registers
//! differ.
//!
//! # What it writes
//!
//! Exactly kitty's `CSI … u` / legacy-functional serialization with the
//! disambiguate, event-type, all-keys-as-escape-codes and associated-text flags
//! (<https://sw.kovidgoyal.net/kitty/keyboard-protocol/>): `CSI number ; mods :
//! event ; text final`, where the number is omitted for a letter-final key with
//! nothing after it (`CSI D`), the second field is omitted for an unmodified
//! press, the event is omitted for a press, and the text field is present only
//! when the key typed text. Alternate keys are not sent: the typed text is
//! always sent instead, which is what the decoder prefers.

use crate::codegen::engine::operand::VirtualRegister;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::target::shared::abi;

pub(crate) const INPUT_ENCODE_SYMBOL: &str = "_mfb_rt_input_encode_key";

/// Bytes the caller's output buffer must hold: the length byte and the
/// longest sequence (`ESC [ 1114111 ; 16 : 3 ; 1114111 u` is 24 bytes).
pub(crate) const ENCODE_OUT_BYTES: usize = 32;

/// Event numbers, as kitty spells them.
pub(crate) const EVENT_PRESS: u64 = 1;
pub(crate) const EVENT_REPEAT: u64 = 2;
pub(crate) const EVENT_RELEASE: u64 = 3;

/// Bit positions of the packed second argument.
pub(crate) const PACK_EVENT_SHIFT: u64 = 24;
pub(crate) const PACK_MODS_SHIFT: u64 = 32;
pub(crate) const PACK_TABLE_SHIFT: u64 = 32;

/// Which platform's native codes and modifier flags an event carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum KeyTable {
    /// AppKit: the first UTF-16 unit of `charactersIgnoringModifiers`, and
    /// `NSEventModifierFlags`.
    AppKit = 0,
    /// GDK: the keyval, and `GdkModifierType`.
    Gdk = 1,
    /// Win32: the virtual-key code; the modifiers are already kitty bits (the
    /// window procedure builds them from `GetKeyState`).
    Win32 = 2,
}

/// kitty modifier bits (the parameter is `1 +` these).
pub(crate) const KITTY_SHIFT: u64 = 1;
pub(crate) const KITTY_ALT: u64 = 2;
pub(crate) const KITTY_CTRL: u64 = 4;
pub(crate) const KITTY_SUPER: u64 = 8;

/// A non-text key: kitty's number and final byte.
type Functional = (u64, u64, u8);

const fn f(native: u64, number: u64, fin: u8) -> Functional {
    (native, number, fin)
}

/// AppKit: `NSUpArrowFunctionKey` (U+F700) and friends (`NSEvent.h`), and the
/// C0 characters AppKit reports for Return, keypad Enter (U+0003), Tab,
/// Shift+Tab (U+0019, backtab), Escape, Delete (U+007F) and Ctrl+H-style
/// Backspace (U+0008).
pub(crate) const APPKIT_KEYS: &[Functional] = &[
    f(0xF700, 1, b'A'),
    f(0xF701, 1, b'B'),
    f(0xF702, 1, b'D'),
    f(0xF703, 1, b'C'),
    f(0xF704, 1, b'P'),
    f(0xF705, 1, b'Q'),
    f(0xF706, 13, b'~'),
    f(0xF707, 1, b'S'),
    f(0xF708, 15, b'~'),
    f(0xF709, 17, b'~'),
    f(0xF70A, 18, b'~'),
    f(0xF70B, 19, b'~'),
    f(0xF70C, 20, b'~'),
    f(0xF70D, 21, b'~'),
    f(0xF70E, 23, b'~'),
    f(0xF70F, 24, b'~'),
    f(0xF727, 2, b'~'),
    f(0xF728, 3, b'~'),
    f(0xF729, 1, b'H'),
    f(0xF72B, 1, b'F'),
    f(0xF72C, 5, b'~'),
    f(0xF72D, 6, b'~'),
    f(0x0D, 13, b'u'),
    f(0x03, 13, b'u'),
    f(0x09, 9, b'u'),
    f(0x19, 9, b'u'),
    f(0x1B, 27, b'u'),
    f(0x7F, 127, b'u'),
    f(0x08, 127, b'u'),
];

/// GDK keyvals (`gdk/gdkkeysyms.h`): the cursor block, its keypad twins (Num
/// Lock off), Tab and `ISO_Left_Tab` (Shift+Tab), Return and `KP_Enter`, and
/// F1–F12.
pub(crate) const GDK_KEYS: &[Functional] = &[
    f(0xFF51, 1, b'D'),
    f(0xFF52, 1, b'A'),
    f(0xFF53, 1, b'C'),
    f(0xFF54, 1, b'B'),
    f(0xFF50, 1, b'H'),
    f(0xFF57, 1, b'F'),
    f(0xFF55, 5, b'~'),
    f(0xFF56, 6, b'~'),
    f(0xFF63, 2, b'~'),
    f(0xFFFF, 3, b'~'),
    f(0xFF96, 1, b'D'),
    f(0xFF97, 1, b'A'),
    f(0xFF98, 1, b'C'),
    f(0xFF99, 1, b'B'),
    f(0xFF95, 1, b'H'),
    f(0xFF9C, 1, b'F'),
    f(0xFF9A, 5, b'~'),
    f(0xFF9B, 6, b'~'),
    f(0xFF9E, 2, b'~'),
    f(0xFF9F, 3, b'~'),
    f(0xFF08, 127, b'u'),
    f(0xFF09, 9, b'u'),
    f(0xFE20, 9, b'u'),
    f(0xFF0D, 13, b'u'),
    f(0xFF8D, 13, b'u'),
    f(0xFF1B, 27, b'u'),
    f(0xFFBE, 1, b'P'),
    f(0xFFBF, 1, b'Q'),
    f(0xFFC0, 13, b'~'),
    f(0xFFC1, 1, b'S'),
    f(0xFFC2, 15, b'~'),
    f(0xFFC3, 17, b'~'),
    f(0xFFC4, 18, b'~'),
    f(0xFFC5, 19, b'~'),
    f(0xFFC6, 20, b'~'),
    f(0xFFC7, 21, b'~'),
    f(0xFFC8, 23, b'~'),
    f(0xFFC9, 24, b'~'),
];

/// Win32 virtual-key codes (`WinUser.h`).
pub(crate) const WIN32_KEYS: &[Functional] = &[
    f(0x25, 1, b'D'),
    f(0x26, 1, b'A'),
    f(0x27, 1, b'C'),
    f(0x28, 1, b'B'),
    f(0x24, 1, b'H'),
    f(0x23, 1, b'F'),
    f(0x21, 5, b'~'),
    f(0x22, 6, b'~'),
    f(0x2D, 2, b'~'),
    f(0x2E, 3, b'~'),
    f(0x08, 127, b'u'),
    f(0x09, 9, b'u'),
    f(0x0D, 13, b'u'),
    f(0x1B, 27, b'u'),
    f(0x70, 1, b'P'),
    f(0x71, 1, b'Q'),
    f(0x72, 13, b'~'),
    f(0x73, 1, b'S'),
    f(0x74, 15, b'~'),
    f(0x75, 17, b'~'),
    f(0x76, 18, b'~'),
    f(0x77, 19, b'~'),
    f(0x78, 20, b'~'),
    f(0x79, 21, b'~'),
    f(0x7A, 23, b'~'),
    f(0x7B, 24, b'~'),
];

pub(crate) fn table_rows(table: KeyTable) -> &'static [Functional] {
    match table {
        KeyTable::AppKit => APPKIT_KEYS,
        KeyTable::Gdk => GDK_KEYS,
        KeyTable::Win32 => WIN32_KEYS,
    }
}

/// `(platform flag, kitty bit)` for every modifier a platform reports. Caps
/// Lock and Num Lock are not modifiers (kitty does not report them either).
pub(crate) fn modifier_rows(table: KeyTable) -> &'static [(u64, u64)] {
    match table {
        // NSEventModifierFlagShift/Control/Option/Command.
        KeyTable::AppKit => &[
            (1 << 17, KITTY_SHIFT),
            (1 << 18, KITTY_CTRL),
            (1 << 19, KITTY_ALT),
            (1 << 20, KITTY_SUPER),
        ],
        // GDK_SHIFT_MASK, GDK_CONTROL_MASK, GDK_ALT_MASK, GDK_SUPER_MASK,
        // GDK_META_MASK.
        KeyTable::Gdk => &[
            (1 << 0, KITTY_SHIFT),
            (1 << 2, KITTY_CTRL),
            (1 << 3, KITTY_ALT),
            (1 << 26, KITTY_SUPER),
            (1 << 28, KITTY_SUPER),
        ],
        KeyTable::Win32 => &[
            (KITTY_SHIFT, KITTY_SHIFT),
            (KITTY_ALT, KITTY_ALT),
            (KITTY_CTRL, KITTY_CTRL),
            (KITTY_SUPER, KITTY_SUPER),
        ],
    }
}

/// Whether `cp` is text a key can type: not a C0 control or DEL, not a
/// surrogate, not the Private Use Area block AppKit reports function keys in
/// (U+E000–U+F8FF), not past Unicode. The model's; the emitter's
/// [`emit_unless_typable`] is its transliteration.
#[cfg(test)]
fn is_typable(cp: u64) -> bool {
    cp >= 0x20 && cp != 0x7F && !(0xD800..=0xF8FF).contains(&cp) && cp <= 0x10FFFF
}

/// The reference encoder the emitted one is a transliteration of.
#[cfg(test)]
pub(crate) fn model(
    table: KeyTable,
    native: u64,
    base: u64,
    native_mods: u64,
    event: u64,
    text: u64,
) -> Vec<u8> {
    let mut mods = 0;
    for (flag, bit) in modifier_rows(table) {
        if native_mods & flag != 0 {
            mods |= bit;
        }
    }
    let (number, fin, text) = match table_rows(table).iter().find(|row| row.0 == native) {
        Some(&(_, number, fin)) => (number, fin, 0),
        None => {
            let base = if (u64::from(b'A')..=u64::from(b'Z')).contains(&base) {
                base + 32
            } else {
                base
            };
            if base == 0 || !is_typable(base) {
                return Vec::new();
            }
            let text = if event == EVENT_RELEASE
                || mods & (KITTY_CTRL | KITTY_SUPER) != 0
                || !is_typable(text)
            {
                0
            } else {
                text
            };
            (base, b'u', text)
        }
    };
    let second = mods != 0 || event != EVENT_PRESS;
    let mut out = b"\x1b[".to_vec();
    if number != 1 || second || text != 0 {
        out.extend(number.to_string().bytes());
    }
    if second || text != 0 {
        out.push(b';');
        if second {
            out.extend((mods + 1).to_string().bytes());
            if event != EVENT_PRESS {
                out.push(b':');
                out.extend(event.to_string().bytes());
            }
        }
    }
    if text != 0 {
        out.push(b';');
        out.extend(text.to_string().bytes());
    }
    out.push(fin);
    out
}

/// Emission state.
struct Enc<'a> {
    ins: &'a mut Vec<CodeInstruction>,
    v: Vregs,
    labels: usize,
}

impl Enc<'_> {
    fn r(&mut self) -> VirtualRegister {
        self.v.next()
    }
    fn label(&mut self, name: &str) -> String {
        self.labels += 1;
        format!("{INPUT_ENCODE_SYMBOL}_{name}_{}", self.labels)
    }
    fn push(&mut self, instruction: CodeInstruction) {
        self.ins.push(instruction);
    }
    fn imm(&mut self, dst: VirtualRegister, value: u64) {
        self.push(abi::move_immediate(dst, "Integer", &value.to_string()));
    }
    fn at(&mut self, name: &str) {
        self.push(abi::label(name));
    }
    fn goto(&mut self, name: &str) {
        self.push(abi::branch(name));
    }
    fn beq(&mut self, reg: VirtualRegister, value: u64, target: &str) {
        self.push(abi::compare_immediate(reg, &value.to_string()));
        self.push(abi::branch_eq(target));
    }
    fn bne(&mut self, reg: VirtualRegister, value: u64, target: &str) {
        self.push(abi::compare_immediate(reg, &value.to_string()));
        self.push(abi::branch_ne(target));
    }
    /// Unsigned `if reg < value goto target`.
    fn blo(&mut self, reg: VirtualRegister, value: u64, target: &str) {
        self.push(abi::compare_immediate(reg, &value.to_string()));
        self.push(abi::branch_lo(target));
    }
    /// Unsigned `if reg > value goto target`.
    fn bhi(&mut self, reg: VirtualRegister, value: u64, target: &str) {
        self.push(abi::compare_immediate(reg, &value.to_string()));
        self.push(abi::branch_hi(target));
    }
    /// `if reg & mask != 0 goto target`.
    fn bit_set(&mut self, reg: VirtualRegister, mask: u64, target: &str) {
        let m = self.r();
        let t = self.r();
        self.imm(m, mask);
        self.push(abi::and_registers(t, reg, m));
        self.bne(t, 0, target);
    }
    /// `*p++ = byte`.
    fn put(&mut self, p: VirtualRegister, byte: u8) {
        let b = self.r();
        self.imm(b, u64::from(byte));
        self.push(abi::store_u8(b, p, 0));
        self.push(abi::add_immediate(p, p, 1));
    }
    /// `*p++ = reg` (the low byte).
    fn put_reg(&mut self, p: VirtualRegister, reg: VirtualRegister) {
        self.push(abi::store_u8(reg, p, 0));
        self.push(abi::add_immediate(p, p, 1));
    }
    /// Append `n` in decimal (no leading zeros; `0` is "0"). Counts each digit
    /// by repeated subtraction of falling powers of ten, so it needs neither a
    /// divide nor a scratch buffer: every value written is at most 1114111, seven
    /// digits.
    fn put_decimal(&mut self, p: VirtualRegister, n: VirtualRegister) {
        let value = self.r();
        let started = self.r();
        let digit = self.r();
        let pow = self.r();
        self.push(abi::move_register(value, n));
        self.imm(started, 0);
        for power in [1_000_000u64, 100_000, 10_000, 1_000, 100, 10, 1] {
            let count = self.label("dec_count");
            let counted = self.label("dec_counted");
            let skip = self.label("dec_skip");
            let emit = self.label("dec_emit");
            self.imm(pow, power);
            self.imm(digit, 0);
            self.at(&count);
            self.push(abi::compare_registers(value, pow));
            self.push(abi::branch_lo(&counted));
            self.push(abi::subtract_registers(value, value, pow));
            self.push(abi::add_immediate(digit, digit, 1));
            self.goto(&count);
            self.at(&counted);
            if power != 1 {
                self.bne(digit, 0, &emit);
                self.beq(started, 0, &skip);
            }
            self.at(&emit);
            self.imm(started, 1);
            self.push(abi::add_immediate(digit, digit, usize::from(b'0')));
            self.put_reg(p, digit);
            self.at(&skip);
        }
    }
}

/// Lower `_mfb_rt_input_encode_key` (see the module doc for the interface).
pub(crate) fn lower_input_encode_key() -> Result<CodeFunction, String> {
    let mut instructions = vec![abi::label("entry")];
    let mut e = Enc {
        ins: &mut instructions,
        v: Vregs::new(),
        labels: 0,
    };
    let out = e.r();
    let packed_native = e.r();
    let packed = e.r();
    let text = e.r();
    e.push(abi::move_register(out, abi::c_arg(0)));
    e.push(abi::move_register(packed_native, abi::c_arg(1)));
    e.push(abi::move_register(packed, abi::c_arg(2)));
    e.push(abi::move_register(text, abi::c_arg(3)));

    let native = e.r();
    let table = e.r();
    let base = e.r();
    let event = e.r();
    let native_mods = e.r();
    let mask = e.r();
    e.imm(mask, 0xFFFF_FFFF);
    e.push(abi::and_registers(native, packed_native, mask));
    e.push(abi::shift_right_immediate(
        table,
        packed_native,
        PACK_TABLE_SHIFT as u8,
    ));
    e.imm(mask, 0xFF_FFFF);
    e.push(abi::and_registers(base, packed, mask));
    e.push(abi::shift_right_immediate(
        event,
        packed,
        PACK_EVENT_SHIFT as u8,
    ));
    e.imm(mask, 0xFF);
    e.push(abi::and_registers(event, event, mask));
    e.push(abi::shift_right_immediate(
        native_mods,
        packed,
        PACK_MODS_SHIFT as u8,
    ));

    let number = e.r();
    let fin = e.r();
    let mods = e.r();
    let empty = e.label("empty");
    let functional = e.label("functional");
    let text_key = e.label("text_key");
    let serialize = e.label("serialize");

    // The per-table blocks: modifiers, then the functional-key lookup.
    e.imm(mods, 0);
    let tables = [KeyTable::AppKit, KeyTable::Gdk, KeyTable::Win32];
    let table_labels: Vec<String> = tables
        .iter()
        .map(|t| e.label(&format!("table_{}", *t as u64)))
        .collect();
    for (t, label) in tables.iter().zip(&table_labels) {
        e.beq(table, *t as u64, label);
    }
    e.goto(&empty);
    for (t, label) in tables.iter().zip(&table_labels) {
        e.at(label);
        for (flag, bit) in modifier_rows(*t) {
            let unset = e.label("mod_unset");
            let t0 = e.r();
            let m = e.r();
            e.imm(m, *flag);
            e.push(abi::and_registers(t0, native_mods, m));
            e.beq(t0, 0, &unset);
            e.imm(m, *bit);
            e.push(abi::or_registers(mods, mods, m));
            e.at(&unset);
        }
        for &(code, num, final_byte) in table_rows(*t) {
            let next = e.label("row");
            e.bne(native, code, &next);
            e.imm(number, num);
            e.imm(fin, u64::from(final_byte));
            e.goto(&functional);
            e.at(&next);
        }
        e.goto(&text_key);
    }

    // A text key: kitty's number is the unshifted code point (lower-cased, since
    // AppKit leaves Shift applied to `charactersIgnoringModifiers`), and the
    // text is sent when the key typed some.
    e.at(&text_key);
    let not_upper = e.label("not_upper");
    e.blo(base, u64::from(b'A'), &not_upper);
    e.bhi(base, u64::from(b'Z'), &not_upper);
    e.push(abi::add_immediate(base, base, 32));
    e.at(&not_upper);
    emit_unless_typable(&mut e, base, &empty);
    e.push(abi::move_register(number, base));
    e.imm(fin, u64::from(b'u'));
    let no_text = e.label("no_text");
    let keep_text = e.label("keep_text");
    e.beq(event, EVENT_RELEASE, &no_text);
    e.bit_set(mods, KITTY_CTRL | KITTY_SUPER, &no_text);
    emit_unless_typable(&mut e, text, &no_text);
    e.goto(&keep_text);
    e.at(&no_text);
    e.imm(text, 0);
    e.at(&keep_text);
    e.goto(&serialize);

    e.at(&functional);
    e.imm(text, 0);

    // CSI [number] [; [mods+1][:event]] [; text] final
    e.at(&serialize);
    let p = e.r();
    e.push(abi::add_immediate(p, out, 1));
    e.put(p, 0x1B);
    e.put(p, b'[');
    let second = e.r();
    let has_second = e.label("has_second");
    e.imm(second, 1);
    e.bne(mods, 0, &has_second);
    e.bne(event, EVENT_PRESS, &has_second);
    e.imm(second, 0);
    e.at(&has_second);
    let write_number = e.label("write_number");
    let number_done = e.label("number_done");
    e.bne(number, 1, &write_number);
    e.bne(second, 0, &write_number);
    e.bne(text, 0, &write_number);
    e.goto(&number_done);
    e.at(&write_number);
    e.put_decimal(p, number);
    e.at(&number_done);
    let fields_done = e.label("fields_done");
    let second_field = e.label("second_field");
    e.bne(second, 0, &second_field);
    e.beq(text, 0, &fields_done);
    e.at(&second_field);
    e.put(p, b';');
    let second_written = e.label("second_written");
    e.beq(second, 0, &second_written);
    let value = e.r();
    e.push(abi::add_immediate(value, mods, 1));
    e.put_decimal(p, value);
    e.beq(event, EVENT_PRESS, &second_written);
    e.put(p, b':');
    e.put_decimal(p, event);
    e.at(&second_written);
    e.beq(text, 0, &fields_done);
    e.put(p, b';');
    e.put_decimal(p, text);
    e.at(&fields_done);
    e.put_reg(p, fin);
    // out[0] = p - (out + 1)
    let len = e.r();
    e.push(abi::subtract_registers(len, p, out));
    e.push(abi::subtract_immediate(len, len, 1));
    e.push(abi::store_u8(len, out, 0));
    let done = e.label("done");
    e.goto(&done);

    e.at(&empty);
    let zero = e.r();
    e.imm(zero, 0);
    e.push(abi::store_u8(zero, out, 0));
    e.at(&done);
    e.push(abi::return_());

    Ok(finalize_vreg_helper(
        "runtime.input.encodeKey",
        INPUT_ENCODE_SYMBOL,
        "Nothing",
        instructions,
        Vec::new(),
    ))
}

/// `goto target` unless `cp` passes [`is_typable`].
fn emit_unless_typable(e: &mut Enc, cp: VirtualRegister, target: &str) {
    let ok = e.label("typable");
    e.blo(cp, 0x20, target);
    e.beq(cp, 0x7F, target);
    e.bhi(cp, 0x10FFFF, target);
    e.blo(cp, 0xD800, &ok);
    e.bhi(cp, 0xF8FF, &ok);
    e.goto(target);
    e.at(&ok);
}
