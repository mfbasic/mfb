//! bug-699 Phase 3: the Windows key translator shared by the top-level window
//! procedure and the transcript EDIT's subclass.
//!
//! Windows reports a key in pieces: `WM_KEYDOWN`/`WM_SYSKEYDOWN` with the
//! virtual-key code, then — once `TranslateMessage` has run in `_main`'s loop —
//! `WM_CHAR`/`WM_SYSCHAR` with the UTF-16 unit(s) it typed, and `WM_KEYUP`/
//! `WM_SYSKEYUP` on release. [`emit_key_inject_helper`] takes each of them and
//! writes to the window input pipe:
//!
//! - a non-text key (the arrows, Home/End, Page Up/Down, Insert, Delete, the
//!   F-keys, Enter, Tab, Backspace, Escape) is injected at its key-down, and its
//!   release at its key-up, in the kitty keyboard-protocol form, through the
//!   shared encoder (`codegen::io::input::encode`). Its `WM_CHAR` (Enter's CR,
//!   Tab, Backspace's BS, Escape) is then not delivered a second time;
//! - a text key depends on the input mode ([`INPUT_MODE_SYM`]). In RAW mode —
//!   set by the key readers and `term::on`, as on macOS and GTK — every event is
//!   kitty-form: the `WM_CHAR` carries the text, the key's unshifted character
//!   (`MapVirtualKeyW`) is kitty's key number, and a Ctrl chord (which types a
//!   control character, or nothing) is injected at its key-down. In the default
//!   mode the text is written as UTF-8 (CR as LF), which is what `io::readLine`
//!   and `io::input` read, and a text key's release is not reported.
//!
//! `WM_CHAR` carries one UTF-16 code unit; a character outside the Basic
//! Multilingual Plane arrives as two, a high then a low surrogate, and is joined
//! before it is encoded ([`HIGH_SURROGATE_SYM`]). Before bug-699 both procedures
//! wrote `wParam & 0xFF`, the low byte of the unit, so `é` entered the pipe as
//! the invalid UTF-8 byte `0xE9`.
//!
//! The modifiers come from `GetKeyState`, not from the message kind: a window
//! with no focused child receives every key as `WM_SYSKEYDOWN`, so `WM_SYS*`
//! does not mean Alt. AltGr (reported by Windows as Ctrl+Alt) typing a
//! printable character is that character, not a Ctrl+Alt chord.

use super::*;
use crate::codegen::io::input::encode::{
    EVENT_PRESS, EVENT_RELEASE, EVENT_REPEAT, INPUT_ENCODE_SYMBOL, KITTY_ALT, KITTY_CTRL,
    KITTY_SHIFT, KITTY_SUPER, PACK_EVENT_SHIFT, PACK_MODS_SHIFT, PACK_TABLE_SHIFT,
};

/// `void _mfb_winapp_key_inject(UINT msg, WPARAM wParam, LPARAM lParam)`.
pub(super) const KEY_INJECT_SYMBOL: &str = "_mfb_winapp_key_inject";
/// The input mode: 0 (the default — text as UTF-8) or [`INPUT_MODE_RAW`].
pub(super) const INPUT_MODE_SYM: &str = "_mfb_winapp_input_mode";
pub(super) const INPUT_MODE_RAW: &str = "2";
/// The virtual-key code of the last key-down, which its `WM_CHAR` follows.
pub(super) const LAST_VK_SYM: &str = "_mfb_winapp_last_vk";
/// Non-zero when the last key-down was already injected in RAW mode (a Ctrl
/// chord), so its `WM_CHAR` is not delivered again.
pub(super) const DOWN_INJECTED_SYM: &str = "_mfb_winapp_down_injected";
/// A high surrogate waiting for its low half.
pub(super) const HIGH_SURROGATE_SYM: &str = "_mfb_winapp_high_surrogate";

pub(super) const WM_KEYDOWN: &str = "256"; // 0x0100
pub(super) const WM_KEYUP: &str = "257"; // 0x0101
pub(super) const WM_SYSKEYDOWN: &str = "260"; // 0x0104
pub(super) const WM_SYSKEYUP: &str = "261"; // 0x0105
pub(super) const WM_SYSCHAR: &str = "262"; // 0x0106
/// `MAPVK_VK_TO_CHAR`.
const MAPVK_VK_TO_CHAR: &str = "2";

// Frame: shadow [0x00,0x20), WriteFile's 5th argument @0x20, `written` @0x28,
// the arguments @0x30/0x38/0x40, working slots, the encoder's buffer, and the
// nonvolatile save area on top. FRAME ≡ 8 (mod 16): entered at sp%16==8.
const OFF_OVERLAPPED: usize = 0x20;
const OFF_WRITTEN: usize = 0x28;
const OFF_MSG: usize = 0x30;
const OFF_WPARAM: usize = 0x38;
const OFF_LPARAM: usize = 0x40;
const OFF_MODS: usize = 0x48;
const OFF_EVENT: usize = 0x50;
const OFF_CP: usize = 0x58;
const OFF_BASE: usize = 0x60;
const OFF_NATIVE: usize = 0x68;
const OFF_BUF: usize = 0x70;
const OFF_SAVE: usize = 0x90;
const FRAME: usize = 0xD8;
const _: () = assert!(OFF_BUF + crate::codegen::io::input::encode::ENCODE_OUT_BYTES <= OFF_SAVE);
const _: () = assert!(FRAME % 16 == 8);

struct Emit<'a> {
    ins: &'a mut Vec<CodeInstruction>,
    rel: &'a mut Vec<CodeRelocation>,
}

impl Emit<'_> {
    fn push(&mut self, instruction: CodeInstruction) {
        self.ins.push(instruction);
    }
    fn label(&mut self, name: &str) {
        self.push(abi::label(name));
    }
    fn imm(&mut self, reg: Reg, value: &str) {
        self.push(abi::move_immediate(reg, "Integer", value));
    }
    fn load(&mut self, reg: Reg, offset: usize) {
        self.push(abi::load_u64(reg, abi::stack_pointer(), offset));
    }
    fn store(&mut self, reg: Reg, offset: usize) {
        self.push(abi::store_u64(reg, abi::stack_pointer(), offset));
    }
    fn global_addr(&mut self, reg: Reg, symbol: &str) {
        load_addr(reg, symbol, KEY_INJECT_SYMBOL, self.ins, self.rel);
    }
    fn load_global(&mut self, reg: Reg, symbol: &str) {
        self.global_addr(reg, symbol);
        self.push(abi::load_u64(reg, reg, 0));
    }
    /// `*symbol = value` through `addr` (`value` must not be `addr`).
    fn store_global(&mut self, value: Reg, addr: Reg, symbol: &str) {
        self.global_addr(addr, symbol);
        self.push(abi::store_u64(value, addr, 0));
    }
    fn external(&mut self, symbol: &str, library: &str) {
        call_external(KEY_INJECT_SYMBOL, symbol, library, self.ins, self.rel);
    }
    /// `if reg == value goto target` (large values go through `scratch`).
    fn beq(&mut self, reg: Reg, value: &str, target: &str) {
        self.push(abi::compare_immediate(reg, value));
        self.push(abi::branch_eq(target));
    }
    fn bne(&mut self, reg: Reg, value: &str, target: &str) {
        self.push(abi::compare_immediate(reg, value));
        self.push(abi::branch_ne(target));
    }
}

/// The key translator (see the module doc).
pub(super) fn emit_key_inject_helper() -> CodeFunction {
    let mut ins: Vec<CodeInstruction> = Vec::new();
    let mut rel: Vec<CodeRelocation> = Vec::new();
    let mut e = Emit {
        ins: &mut ins,
        rel: &mut rel,
    };
    let (a0, a1, a2, a3) = (
        abi::mfb_arg(0),
        abi::mfb_arg(1),
        abi::mfb_arg(2),
        abi::mfb_arg(3),
    );
    let ret = abi::c_return(0);
    e.label("entry");
    e.push(abi::subtract_stack(FRAME));
    e.store(a0, OFF_MSG);
    e.store(a1, OFF_WPARAM);
    e.store(a2, OFF_LPARAM);

    // mods = kitty bits of the modifier keys that are down now.
    e.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), OFF_MODS));
    for (vk, bit) in [
        ("16", KITTY_SHIFT), // VK_SHIFT
        ("18", KITTY_ALT),   // VK_MENU
        ("17", KITTY_CTRL),  // VK_CONTROL
        ("91", KITTY_SUPER), // VK_LWIN
        ("92", KITTY_SUPER), // VK_RWIN
    ] {
        let up = format!("kinj_mod_{vk}_up");
        e.imm(a0, vk);
        e.external("GetKeyState", USER32);
        // SHORT: the high bit is "down"; the rest of the register is unspecified.
        e.imm(a1, "32768");
        e.push(abi::and_registers(a1, ret, a1));
        e.beq(a1, "0", &up);
        e.load(a1, OFF_MODS);
        e.imm(a2, &bit.to_string());
        e.push(abi::or_registers(a1, a1, a2));
        e.store(a1, OFF_MODS);
        e.label(&up);
    }
    // event = lParam bit 30 (the key was already down) ? repeat : press
    e.load(a1, OFF_LPARAM);
    e.push(abi::shift_right_immediate(a1, a1, 30));
    e.imm(a2, "1");
    e.push(abi::and_registers(a1, a1, a2));
    e.imm(a2, &EVENT_PRESS.to_string());
    e.beq(a1, "0", "kinj_event_set");
    e.imm(a2, &EVENT_REPEAT.to_string());
    e.label("kinj_event_set");
    e.store(a2, OFF_EVENT);

    e.load(a0, OFF_MSG);
    e.beq(a0, WM_KEYDOWN, "kinj_keydown");
    e.beq(a0, WM_SYSKEYDOWN, "kinj_keydown");
    e.beq(a0, WM_KEYUP, "kinj_keyup");
    e.beq(a0, WM_SYSKEYUP, "kinj_keyup");
    e.beq(a0, super::WM_CHAR, "kinj_char");
    e.beq(a0, WM_SYSCHAR, "kinj_char");
    e.push(abi::branch("kinj_done"));

    // ---- key-down: a non-text key now; a text key waits for its WM_CHAR ------
    e.label("kinj_keydown");
    e.load(a1, OFF_WPARAM);
    e.imm(a2, "65535");
    e.push(abi::and_registers(a1, a1, a2));
    e.store(a1, OFF_NATIVE);
    e.store_global(a1, a2, LAST_VK_SYM);
    e.push(abi::move_immediate(a1, "Integer", "0"));
    e.store_global(a1, a2, DOWN_INJECTED_SYM);
    e.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), OFF_BASE));
    e.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), OFF_CP));
    emit_encode(&mut e);
    e.push(abi::load_u8(a2, abi::stack_pointer(), OFF_BUF));
    e.bne(a2, "0", "kinj_write");
    // Not a non-text key. In RAW mode a Ctrl chord (Ctrl held, Alt not — Ctrl+Alt
    // is AltGr) is injected now: it types a control character or nothing, never
    // text, so waiting for a WM_CHAR could wait for one that does not come.
    e.load_global(a1, INPUT_MODE_SYM);
    e.bne(a1, INPUT_MODE_RAW, "kinj_done");
    e.load(a1, OFF_MODS);
    e.imm(a2, &(KITTY_CTRL | KITTY_ALT).to_string());
    e.push(abi::and_registers(a1, a1, a2));
    e.bne(a1, &KITTY_CTRL.to_string(), "kinj_done");
    e.load(a0, OFF_NATIVE);
    emit_vk_base(&mut e);
    e.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), OFF_NATIVE));
    emit_encode(&mut e);
    e.push(abi::load_u8(a2, abi::stack_pointer(), OFF_BUF));
    e.beq(a2, "0", "kinj_done");
    e.imm(a1, "1");
    e.store_global(a1, a0, DOWN_INJECTED_SYM);
    e.push(abi::branch("kinj_write"));

    // ---- key-up ---------------------------------------------------------------
    e.label("kinj_keyup");
    e.imm(a1, &EVENT_RELEASE.to_string());
    e.store(a1, OFF_EVENT);
    e.load(a1, OFF_WPARAM);
    e.imm(a2, "65535");
    e.push(abi::and_registers(a1, a1, a2));
    e.store(a1, OFF_NATIVE);
    e.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), OFF_BASE));
    e.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), OFF_CP));
    emit_encode(&mut e);
    e.push(abi::load_u8(a2, abi::stack_pointer(), OFF_BUF));
    e.bne(a2, "0", "kinj_write");
    // A text key's release: RAW mode only.
    e.load_global(a1, INPUT_MODE_SYM);
    e.bne(a1, INPUT_MODE_RAW, "kinj_done");
    e.load(a0, OFF_NATIVE);
    emit_vk_base(&mut e);
    e.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), OFF_NATIVE));
    emit_encode(&mut e);
    e.push(abi::load_u8(a2, abi::stack_pointer(), OFF_BUF));
    e.bne(a2, "0", "kinj_write");
    e.push(abi::branch("kinj_done"));

    // ---- a character ----------------------------------------------------------
    e.label("kinj_char");
    e.load(a1, OFF_WPARAM);
    e.imm(a2, "65535");
    e.push(abi::and_registers(a1, a1, a2));
    // A high surrogate waits for its low half.
    e.imm(a2, "55296"); // 0xD800
    e.push(abi::compare_registers(a1, a2));
    e.push(abi::branch_lo("kinj_char_bmp"));
    e.imm(a2, "56319"); // 0xDBFF
    e.push(abi::compare_registers(a1, a2));
    e.push(abi::branch_hi("kinj_char_low"));
    e.store_global(a1, a2, HIGH_SURROGATE_SYM);
    e.push(abi::branch("kinj_done"));
    e.label("kinj_char_low");
    e.imm(a2, "57343"); // 0xDFFF
    e.push(abi::compare_registers(a1, a2));
    e.push(abi::branch_hi("kinj_char_bmp"));
    // cp = 0x10000 + ((high - 0xD800) << 10) + (low - 0xDC00); a lone low
    // surrogate is dropped.
    e.load_global(a0, HIGH_SURROGATE_SYM);
    e.beq(a0, "0", "kinj_done");
    e.imm(a2, "55296");
    e.push(abi::subtract_registers(a0, a0, a2));
    e.push(abi::shift_left_immediate(a0, a0, 10));
    e.imm(a2, "56320");
    e.push(abi::subtract_registers(a1, a1, a2));
    e.push(abi::add_registers(a1, a1, a0));
    e.imm(a2, "65536");
    e.push(abi::add_registers(a1, a1, a2));
    e.label("kinj_char_bmp");
    e.store(a1, OFF_CP);
    e.push(abi::move_immediate(a1, "Integer", "0"));
    e.store_global(a1, a2, HIGH_SURROGATE_SYM);
    // The key that typed it: a non-text key was delivered at its key-down, and a
    // RAW-mode Ctrl chord too.
    e.load_global(a1, LAST_VK_SYM);
    e.store(a1, OFF_NATIVE);
    e.push(abi::move_immediate(a0, "Integer", "0"));
    e.store_global(a0, a2, LAST_VK_SYM);
    e.load_global(a0, DOWN_INJECTED_SYM);
    e.bne(a0, "0", "kinj_char_skip");
    e.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), OFF_BASE));
    e.load(a0, OFF_CP);
    e.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), OFF_CP));
    e.store(a0, OFF_WRITTEN); // the character, parked while the lookup encodes
    emit_encode(&mut e);
    e.load(a0, OFF_WRITTEN);
    e.store(a0, OFF_CP);
    e.push(abi::load_u8(a2, abi::stack_pointer(), OFF_BUF));
    e.bne(a2, "0", "kinj_done");
    e.load_global(a1, INPUT_MODE_SYM);
    e.beq(a1, INPUT_MODE_RAW, "kinj_char_raw");

    // Default mode: the character as UTF-8, CR as LF. Alt+key (WM_SYSCHAR with
    // Alt down) types nothing into the transcript, so it types nothing here.
    e.load(a0, OFF_MSG);
    e.bne(a0, WM_SYSCHAR, "kinj_char_text");
    e.load(a0, OFF_MODS);
    e.imm(a2, &KITTY_ALT.to_string());
    e.push(abi::and_registers(a0, a0, a2));
    e.bne(a0, "0", "kinj_done");
    e.label("kinj_char_text");
    e.load(a0, OFF_CP);
    e.bne(a0, "13", "kinj_char_utf8");
    e.imm(a0, "10");
    e.label("kinj_char_utf8");
    emit_utf8(&mut e);
    e.push(abi::branch("kinj_write"));

    // RAW mode: the kitty form, the character as its text.
    e.label("kinj_char_raw");
    // AltGr: Windows reports it as Ctrl+Alt; a printable character it types is
    // plain text.
    e.load(a1, OFF_MODS);
    e.imm(a2, &(KITTY_CTRL | KITTY_ALT).to_string());
    e.push(abi::and_registers(a0, a1, a2));
    e.bne(a0, &(KITTY_CTRL | KITTY_ALT).to_string(), "kinj_char_mods");
    e.load(a0, OFF_CP);
    e.imm(a2, "32");
    e.push(abi::compare_registers(a0, a2));
    e.push(abi::branch_lo("kinj_char_mods"));
    e.imm(a2, &(!(KITTY_CTRL | KITTY_ALT) & 0xFF).to_string());
    e.push(abi::and_registers(a1, a1, a2));
    e.store(a1, OFF_MODS);
    e.label("kinj_char_mods");
    // base = the key's unshifted character, or the character itself when the
    // WM_CHAR follows no key-down (posted input).
    e.load(a0, OFF_NATIVE);
    e.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), OFF_BASE));
    e.beq(a0, "0", "kinj_char_base");
    emit_vk_base(&mut e);
    e.label("kinj_char_base");
    e.load(a0, OFF_BASE);
    e.bne(a0, "0", "kinj_char_encode");
    e.load(a0, OFF_CP);
    e.store(a0, OFF_BASE);
    e.label("kinj_char_encode");
    e.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), OFF_NATIVE));
    emit_encode(&mut e);
    e.push(abi::load_u8(a2, abi::stack_pointer(), OFF_BUF));
    e.bne(a2, "0", "kinj_write");
    e.push(abi::branch("kinj_done"));
    e.label("kinj_char_skip");
    e.push(abi::move_immediate(a0, "Integer", "0"));
    e.store_global(a0, a2, DOWN_INJECTED_SYM);
    e.push(abi::branch("kinj_done"));

    // ---- WriteFile(pipe, buf + 1, len, &written, NULL) ------------------------
    e.label("kinj_write");
    e.load_global(a0, STDIN_WRITE_SYM);
    e.beq(a0, "0", "kinj_done"); // the pipe was never wired (headless)
    e.push(abi::load_u8(a2, abi::stack_pointer(), OFF_BUF));
    e.push(abi::add_immediate(a1, abi::stack_pointer(), OFF_BUF + 1));
    e.push(abi::add_immediate(a3, abi::stack_pointer(), OFF_WRITTEN));
    e.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), OFF_WRITTEN));
    e.push(abi::store_u64(
        abi::ZERO,
        abi::stack_pointer(),
        OFF_OVERLAPPED,
    ));
    e.external("WriteFile", KERNEL32);

    e.label("kinj_done");
    e.push(abi::add_stack(FRAME));
    e.push(abi::return_());
    let ins = super::preserve_win64_nonvolatile(ins, OFF_SAVE);
    code_function("winapp.keyInject", KEY_INJECT_SYMBOL, ins, rel)
}

/// `encode(buf, native | Win32 table, base | event << 24 | mods << 32, cp)`
/// from the frame slots; the encoded length lands at `buf[0]`.
fn emit_encode(e: &mut Emit) {
    let (a0, a1, a2, a3) = (
        abi::mfb_arg(0),
        abi::mfb_arg(1),
        abi::mfb_arg(2),
        abi::mfb_arg(3),
    );
    e.load(a1, OFF_NATIVE);
    e.imm(
        a2,
        &((crate::codegen::io::input::encode::KeyTable::Win32 as u64) << PACK_TABLE_SHIFT)
            .to_string(),
    );
    e.push(abi::or_registers(a1, a1, a2));
    e.load(a2, OFF_MODS);
    e.push(abi::shift_left_immediate(a2, a2, PACK_MODS_SHIFT as u8));
    e.load(a3, OFF_EVENT);
    e.push(abi::shift_left_immediate(a3, a3, PACK_EVENT_SHIFT as u8));
    e.push(abi::or_registers(a2, a2, a3));
    e.load(a3, OFF_BASE);
    e.push(abi::or_registers(a2, a2, a3));
    e.load(a3, OFF_CP);
    e.push(abi::add_immediate(a0, abi::stack_pointer(), OFF_BUF));
    call_internal(KEY_INJECT_SYMBOL, INPUT_ENCODE_SYMBOL, e.ins, e.rel);
}

/// `OFF_BASE = MapVirtualKeyW(vk /* a0 */, MAPVK_VK_TO_CHAR)` without the
/// dead-key flag (bit 31): the character the key types with no modifiers.
fn emit_vk_base(e: &mut Emit) {
    let (a1, ret) = (abi::mfb_arg(1), abi::c_return(0));
    e.imm(a1, MAPVK_VK_TO_CHAR);
    e.external("MapVirtualKeyW", USER32);
    e.imm(a1, "2147483647");
    e.push(abi::and_registers(a1, ret, a1));
    e.store(a1, OFF_BASE);
}

/// The code point in `a0` as UTF-8 at `buf + 1`, its length at `buf[0]`.
fn emit_utf8(e: &mut Emit) {
    let (a0, a1, a2) = (abi::mfb_arg(0), abi::mfb_arg(1), abi::mfb_arg(2));
    let byte = |e: &mut Emit, index: usize| {
        e.push(abi::store_u8(a1, abi::stack_pointer(), OFF_BUF + 1 + index));
    };
    // cont(shift) = 0x80 | ((cp >> shift) & 0x3F), into a1.
    let cont = |e: &mut Emit, shift: u8| {
        e.push(abi::shift_right_immediate(a1, a0, shift));
        e.imm(a2, "63");
        e.push(abi::and_registers(a1, a1, a2));
        e.imm(a2, "128");
        e.push(abi::or_registers(a1, a1, a2));
    };
    let lead = |e: &mut Emit, shift: u8, marker: &str| {
        e.push(abi::shift_right_immediate(a1, a0, shift));
        e.imm(a2, marker);
        e.push(abi::or_registers(a1, a1, a2));
    };
    let length = |e: &mut Emit, n: &str| {
        e.imm(a1, n);
        e.push(abi::store_u8(a1, abi::stack_pointer(), OFF_BUF));
    };
    e.imm(a2, "128");
    e.push(abi::compare_registers(a0, a2));
    e.push(abi::branch_lo("kinj_utf8_1"));
    e.imm(a2, "2048");
    e.push(abi::compare_registers(a0, a2));
    e.push(abi::branch_lo("kinj_utf8_2"));
    e.imm(a2, "65536");
    e.push(abi::compare_registers(a0, a2));
    e.push(abi::branch_lo("kinj_utf8_3"));
    lead(e, 18, "240");
    byte(e, 0);
    cont(e, 12);
    byte(e, 1);
    cont(e, 6);
    byte(e, 2);
    cont(e, 0);
    byte(e, 3);
    length(e, "4");
    e.push(abi::branch("kinj_utf8_done"));
    e.label("kinj_utf8_3");
    lead(e, 12, "224");
    byte(e, 0);
    cont(e, 6);
    byte(e, 1);
    cont(e, 0);
    byte(e, 2);
    length(e, "3");
    e.push(abi::branch("kinj_utf8_done"));
    e.label("kinj_utf8_2");
    lead(e, 6, "192");
    byte(e, 0);
    cont(e, 0);
    byte(e, 1);
    length(e, "2");
    e.push(abi::branch("kinj_utf8_done"));
    e.label("kinj_utf8_1");
    e.push(abi::move_register(a1, a0));
    byte(e, 0);
    length(e, "1");
    e.label("kinj_utf8_done");
}

/// The globals the translator keeps between messages.
pub(super) fn key_data_objects() -> Vec<CodeDataObject> {
    [
        INPUT_MODE_SYM,
        LAST_VK_SYM,
        DOWN_INJECTED_SYM,
        HIGH_SURROGATE_SYM,
    ]
    .into_iter()
    .map(writable_qword)
    .collect()
}

/// `*_mfb_winapp_input_mode = mode`, from any thread (a plain aligned store).
/// Uses the first two MFB argument registers.
pub(super) fn emit_set_input_mode(
    from: &str,
    mode: &str,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
) {
    load_addr(abi::mfb_arg(0), INPUT_MODE_SYM, from, ins, rel);
    ins.push(abi::move_immediate(abi::mfb_arg(1), "Integer", mode));
    ins.push(abi::store_u64(abi::mfb_arg(1), abi::mfb_arg(0), 0));
}
