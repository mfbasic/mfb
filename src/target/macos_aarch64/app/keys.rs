//! bug-699 Phase 3: the macOS key translator shared by every view's `keyDown:`
//! and `keyUp:`.
//!
//! One helper, [`emit_key_inject_helper`], turns an `NSEvent` into the kitty
//! keyboard-protocol bytes (through the shared encoder,
//! `codegen::io::input::encode`) and writes them to the window input pipe. The
//! transcript's and the TUI's `keyDown:` call it in raw input mode, the canvas
//! view's always; the `keyUp:` IMPs call it for the release.
//!
//! What the helper reads from the event:
//! - `charactersIgnoringModifiers` — the key: its first code point is both the
//!   AppKit table's native code (`NSLeftArrowFunctionKey`, U+F700, …; Return,
//!   Tab, Escape, Delete as their C0 characters) and, for a text key, kitty's
//!   key number (AppKit leaves Shift applied here; the encoder lower-cases);
//! - `characters` — the text the key typed (not read for a release);
//! - `modifierFlags` — Shift, Control, Option (Alt), Command (Meta);
//! - `isARepeat` — press or repeat.
//!
//! `flagsChanged:` is not overridden: a modifier key alone names no `io::KeyName`
//! (the decoder drops kitty's modifier-key events), and every key event already
//! carries the modifier state in `modifierFlags`.

use super::*;
use crate::codegen::io::input::encode::{
    ENCODE_OUT_BYTES, EVENT_PRESS, EVENT_RELEASE, EVENT_REPEAT, INPUT_ENCODE_SYMBOL,
    PACK_EVENT_SHIFT, PACK_MODS_SHIFT,
};

/// The shared translator: `void _mfb_macapp_key_inject(NSEvent *event /*x0*/,
/// long release /*x1*/)`.
pub(super) const KEY_INJECT_SYMBOL: &str = "_mfb_macapp_key_inject";
/// `keyUp:` IMP for the transcript and the TUI view: injects the release in raw
/// input mode only (line mode has no key events to report).
pub(super) const KEY_UP_SYMBOL: &str = "_mfb_macapp_key_up";
/// `keyUp:` IMP for the canvas view: always injects (a canvas is always raw).
pub(super) const CANVAS_KEY_UP_SYMBOL: &str = "_mfb_macapp_canvas_keyUp";

pub(super) const SEL_KEY_UP: (&str, &str) = ("_mfb_macapp_sel_keyUp", "keyUp:");
pub(super) const SEL_CHARACTERS_IGNORING_MODIFIERS: (&str, &str) = (
    "_mfb_macapp_sel_charactersIgnoringModifiers",
    "charactersIgnoringModifiers",
);
pub(super) const SEL_MODIFIER_FLAGS: (&str, &str) =
    ("_mfb_macapp_sel_modifierFlags", "modifierFlags");
pub(super) const SEL_IS_A_REPEAT: (&str, &str) = ("_mfb_macapp_sel_isARepeat", "isARepeat");

/// The selector strings this module adds to the app's C-string table.
pub(super) const KEY_SELECTORS: [(&str, &str); 4] = [
    SEL_KEY_UP,
    SEL_CHARACTERS_IGNORING_MODIFIERS,
    SEL_MODIFIER_FLAGS,
    SEL_IS_A_REPEAT,
];

// Frame of the translator.
const FRAME: usize = 96;
const OFF_FLAGS: usize = 48;
const OFF_LEN: usize = 56;
const OFF_BUF: usize = 64;
const _: () = assert!(OFF_BUF + ENCODE_OUT_BYTES <= FRAME);

/// `dst = code point of [string characterAtIndex:0]` (joining a surrogate pair),
/// or 0 for an empty string. `string` must be callee-saved; clobbers
/// `abi::LOCAL[2]` and the `OFF_LEN` slot.
fn emit_first_code_point(asm: &mut Asm, string: Reg, dst: Reg, tag: &str) {
    let empty = format!("kinj_{tag}_empty");
    let done = format!("kinj_{tag}_done");
    asm.load_selector(SEL_LENGTH.0);
    asm.push(abi::move_register(abi::c_arg(0), string));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::store_u64(abi::c_arg(0), abi::stack_pointer(), OFF_LEN));
    asm.push(abi::compare_immediate(abi::c_arg(0), "0"));
    asm.push(abi::branch_eq(&empty));
    asm.load_selector(SEL_CHAR_AT_INDEX.0);
    asm.push(abi::move_immediate(abi::c_arg(2), "Integer", "0"));
    asm.push(abi::move_register(abi::c_arg(0), string));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register(dst, abi::c_arg(0)));
    // A high surrogate with a unit after it: join the pair.
    asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", "55296")); // 0xD800
    asm.push(abi::compare_registers(dst, abi::SCRATCH[0]));
    asm.push(abi::branch_lo(&done));
    asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", "56319")); // 0xDBFF
    asm.push(abi::compare_registers(dst, abi::SCRATCH[0]));
    asm.push(abi::branch_hi(&done));
    asm.push(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        OFF_LEN,
    ));
    asm.push(abi::compare_immediate(abi::SCRATCH[0], "2"));
    asm.push(abi::branch_lo(&done));
    asm.load_selector(SEL_CHAR_AT_INDEX.0);
    asm.push(abi::move_immediate(abi::c_arg(2), "Integer", "1"));
    asm.push(abi::move_register(abi::c_arg(0), string));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    // cp = 0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00)
    asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", "55296"));
    asm.push(abi::subtract_registers(dst, dst, abi::SCRATCH[0]));
    asm.push(abi::shift_left_immediate(dst, dst, 10));
    asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", "56320"));
    asm.push(abi::subtract_registers(
        abi::c_arg(0),
        abi::c_arg(0),
        abi::SCRATCH[0],
    ));
    asm.push(abi::add_registers(dst, dst, abi::c_arg(0)));
    asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", "65536"));
    asm.push(abi::add_registers(dst, dst, abi::SCRATCH[0]));
    asm.push(abi::branch(&done));
    asm.push(abi::label(&empty));
    asm.push(abi::move_immediate(dst, "Integer", "0"));
    asm.push(abi::label(&done));
}

/// `void _mfb_macapp_key_inject(NSEvent *event, long release)`.
pub(super) fn emit_key_inject_helper() -> CodeFunction {
    let mut asm = Asm::new(KEY_INJECT_SYMBOL);
    // lr@0, event@8, release/event number@16, scratch object@24, key@32,
    // text@40, modifier flags@48, length@56, encoder buffer@64.
    let event = abi::LOCAL[0];
    let kind = abi::LOCAL[1];
    let object = abi::LOCAL[2];
    let key = abi::LOCAL[3];
    let text = abi::LOCAL[4];
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(FRAME));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        0,
    ));
    for (i, reg) in [event, kind, object, key, text].iter().enumerate() {
        asm.push(abi::store_u64(*reg, abi::stack_pointer(), 8 + 8 * i));
    }
    asm.push(abi::move_register(event, abi::c_arg(0)));
    asm.push(abi::move_register(kind, abi::c_arg(1)));

    // key = first code point of [event charactersIgnoringModifiers]; a key with
    // none (a dead key's own press, a modifier) is nothing to report.
    asm.load_selector(SEL_CHARACTERS_IGNORING_MODIFIERS.0);
    asm.push(abi::move_register(abi::c_arg(0), event));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register(object, abi::c_arg(0)));
    emit_first_code_point(&mut asm, object, key, "key");
    asm.push(abi::compare_immediate(key, "0"));
    asm.push(abi::branch_eq("kinj_done"));

    // text = first code point of [event characters], for a press or repeat.
    asm.push(abi::move_immediate(text, "Integer", "0"));
    asm.push(abi::compare_immediate(kind, "0"));
    asm.push(abi::branch_ne("kinj_have_text"));
    asm.load_selector(SEL_CHARACTERS.0);
    asm.push(abi::move_register(abi::c_arg(0), event));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register(object, abi::c_arg(0)));
    emit_first_code_point(&mut asm, object, text, "text");
    asm.push(abi::label("kinj_have_text"));

    // The modifier flags.
    asm.load_selector(SEL_MODIFIER_FLAGS.0);
    asm.push(abi::move_register(abi::c_arg(0), event));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::store_u64(
        abi::c_arg(0),
        abi::stack_pointer(),
        OFF_FLAGS,
    ));

    // The event: a release, else a repeat when AppKit says so, else a press.
    asm.push(abi::compare_immediate(kind, "0"));
    asm.push(abi::branch_eq("kinj_down"));
    asm.push(abi::move_immediate(
        kind,
        "Integer",
        &EVENT_RELEASE.to_string(),
    ));
    asm.push(abi::branch("kinj_encode"));
    asm.push(abi::label("kinj_down"));
    asm.load_selector(SEL_IS_A_REPEAT.0);
    asm.push(abi::move_register(abi::c_arg(0), event));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    // `BOOL` is one byte; the upper bits of the register are unspecified.
    asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", "255"));
    asm.push(abi::and_registers(
        abi::c_arg(0),
        abi::c_arg(0),
        abi::SCRATCH[0],
    ));
    asm.push(abi::move_immediate(
        kind,
        "Integer",
        &EVENT_PRESS.to_string(),
    ));
    asm.push(abi::compare_immediate(abi::c_arg(0), "0"));
    asm.push(abi::branch_eq("kinj_encode"));
    asm.push(abi::move_immediate(
        kind,
        "Integer",
        &EVENT_REPEAT.to_string(),
    ));

    // encode(buf, key | AppKit table (0), key | event << 24 | flags << 32, text)
    asm.push(abi::label("kinj_encode"));
    asm.push(abi::add_immediate(
        abi::c_arg(0),
        abi::stack_pointer(),
        OFF_BUF,
    ));
    asm.push(abi::move_register(abi::c_arg(1), key));
    asm.push(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        OFF_FLAGS,
    ));
    asm.push(abi::shift_left_immediate(
        abi::SCRATCH[0],
        abi::SCRATCH[0],
        PACK_MODS_SHIFT as u8,
    ));
    asm.push(abi::shift_left_immediate(
        abi::SCRATCH[1],
        kind,
        PACK_EVENT_SHIFT as u8,
    ));
    asm.push(abi::or_registers(
        abi::SCRATCH[0],
        abi::SCRATCH[0],
        abi::SCRATCH[1],
    ));
    asm.push(abi::or_registers(abi::c_arg(2), abi::SCRATCH[0], key));
    asm.push(abi::move_register(abi::c_arg(3), text));
    asm.call_internal(INPUT_ENCODE_SYMBOL);
    asm.push(abi::load_u8(object, abi::stack_pointer(), OFF_BUF));
    asm.push(abi::compare_immediate(object, "0"));
    asm.push(abi::branch_eq("kinj_done"));

    // write(objc_getAssociatedObject(NSApp, PIPE_ASSOC_KEY), buf + 1, len)
    asm.load_selector(SEL_SHARED_APPLICATION.0);
    asm.external_data(abi::c_arg(0), CLASS_NS_APPLICATION, LIB_APPKIT);
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.local_address(abi::c_arg(1), PIPE_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::add_immediate(
        abi::c_arg(1),
        abi::stack_pointer(),
        OFF_BUF + 1,
    ));
    asm.push(abi::move_register(abi::c_arg(2), object));
    asm.call_external("_write", LIB_SYSTEM);

    asm.push(abi::label("kinj_done"));
    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    for (i, reg) in [event, kind, object, key, text].iter().enumerate() {
        asm.push(abi::load_u64(*reg, abi::stack_pointer(), 8 + 8 * i));
    }
    asm.push(abi::add_stack(FRAME));
    asm.push(abi::return_());
    code_function("macapp.keyInject", KEY_INJECT_SYMBOL, "Nothing", asm)
}

/// A `keyUp:` IMP (`void keyUp:(id self, SEL, NSEvent *event /*x2*/)`). With
/// `raw_only`, the release is reported only while the window is in raw input
/// mode — the transcript and the TUI view, whose line mode has no key events.
fn emit_key_up(symbol: &str, name: &str, raw_only: bool) -> CodeFunction {
    let mut asm = Asm::new(symbol);
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(16));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        0,
    ));
    asm.push(abi::store_u64(abi::LOCAL[0], abi::stack_pointer(), 8));
    asm.push(abi::move_register(abi::LOCAL[0], abi::c_arg(2))); // event
    if raw_only {
        // mode = objc_getAssociatedObject([NSApplication sharedApplication],
        // INPUT_MODE_KEY). The selector first: resolving it calls out.
        asm.load_selector(SEL_SHARED_APPLICATION.0);
        asm.external_data(abi::c_arg(0), CLASS_NS_APPLICATION, LIB_APPKIT);
        asm.call_external("_objc_msgSend", LIB_OBJC);
        asm.local_address(abi::c_arg(1), INPUT_MODE_KEY);
        asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
        asm.push(abi::compare_immediate(
            abi::c_arg(0),
            INPUT_MODE_RAW_NO_ECHO,
        ));
        asm.push(abi::branch_ne("kup_done"));
    }
    asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[0]));
    asm.push(abi::move_immediate(abi::c_arg(1), "Integer", "1")); // release
    asm.call_internal(KEY_INJECT_SYMBOL);
    asm.push(abi::label("kup_done"));
    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::load_u64(abi::LOCAL[0], abi::stack_pointer(), 8));
    asm.push(abi::add_stack(16));
    asm.push(abi::return_());
    code_function(name, symbol, "Nothing", asm)
}

pub(super) fn emit_key_up_helper() -> CodeFunction {
    emit_key_up(KEY_UP_SYMBOL, "macapp.keyUp", true)
}

pub(super) fn emit_canvas_key_up_helper() -> CodeFunction {
    emit_key_up(CANVAS_KEY_UP_SYMBOL, "macapp.canvas.keyUp", false)
}

/// `[class addMethod keyUp: → imp]`, for a class pair being built in `class`.
pub(super) fn emit_install_key_up(asm: &mut Asm, class: Reg, imp: &str) {
    asm.load_selector(SEL_KEY_UP.0);
    asm.local_address(abi::c_arg(2), imp);
    asm.local_address(abi::c_arg(3), STR_INPUT_TYPES.0); // "v@:@"
    asm.push(abi::move_register(abi::c_arg(0), class));
    asm.call_external("_class_addMethod", LIB_OBJC);
}

/// Call the translator for a key press from a `keyDown:` IMP: `event` is the
/// register holding the `NSEvent`.
pub(super) fn emit_inject_press(asm: &mut Asm, event: Reg) {
    asm.push(abi::move_register(abi::c_arg(0), event));
    asm.push(abi::move_immediate(abi::c_arg(1), "Integer", "0"));
    asm.call_internal(KEY_INJECT_SYMBOL);
}

/// In line input mode a function key (AppKit reports it as a Private Use Area
/// character, U+F700–U+F8FF) is not text: branch to `ignore` for one, so the
/// line buffer never takes an arrow key's code point.
pub(super) fn emit_skip_function_key(asm: &mut Asm, code: Reg, ignore: &str, tag: &str) {
    let text = format!("{tag}_not_function_key");
    asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", "63232")); // 0xF700
    asm.push(abi::compare_registers(code, abi::SCRATCH[0]));
    asm.push(abi::branch_lo(&text));
    asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", "63743")); // 0xF8FF
    asm.push(abi::compare_registers(code, abi::SCRATCH[0]));
    asm.push(abi::branch_ls(ignore));
    asm.push(abi::label(&text));
}

fn code_function(name: &str, symbol: &str, returns: &str, asm: Asm) -> CodeFunction {
    CodeFunction {
        name: name.to_string(),
        symbol: symbol.to_string(),
        params: Vec::new(),
        returns: returns.to_string(),
        frame: CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        stack_slots: Vec::new(),
        instructions: asm.ins,
        relocations: asm.rel,
    }
}
