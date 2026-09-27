//! bug-699 Phase 3: the GTK key translator shared by `key-pressed` and
//! `key-released`.
//!
//! [`emit_key_inject_helper`] turns one GDK key event into the kitty
//! keyboard-protocol bytes (through the shared encoder,
//! `codegen::io::input::encode`) and writes them to the window input pipe. The
//! `key-pressed` handler's raw arm calls it; [`emit_key_released_handler`], the
//! new `key-released` handler, calls it for the release.
//!
//! What it reads: the keyval — the GDK table's native code for a non-text key,
//! and through `gdk_keyval_to_lower` + `gdk_keyval_to_unicode` the unshifted code
//! point of a text key — the typed text (`gdk_keyval_to_unicode` of the keyval
//! itself), and the `GdkModifierType` state. It runs BEFORE any "is this a
//! character" test, which is what makes the arrows, Home/End, Page Up/Down and
//! the F-keys arrive at all: `gdk_keyval_to_unicode` maps every one of them to 0,
//! and the handler used to drop a 0.
//!
//! GTK 4 reports no auto-repeat flag, so the helper keeps which hardware keys are
//! down ([`KEYS_HELD_SYMBOL`], one byte per keycode): a press of a key already
//! down is a repeat.

use super::*;
use crate::codegen::io::input::encode::{
    EVENT_PRESS, EVENT_RELEASE, EVENT_REPEAT, INPUT_ENCODE_SYMBOL, PACK_EVENT_SHIFT,
    PACK_MODS_SHIFT, PACK_TABLE_SHIFT,
};

/// `void _mfb_gtkapp_key_inject(guint keyval, guint keycode, GdkModifierType
/// state, long kind)`; `kind` is [`KIND_PRESS`], [`KIND_RELEASE`] or
/// [`KIND_FORGET`].
pub(super) const KEY_INJECT_SYMBOL: &str = "_mfb_gtkapp_key_inject";
/// The `key-released` handler.
pub(super) const KEY_RELEASED_SYMBOL: &str = "_mfb_gtkapp_key_released";
/// One byte per hardware keycode: non-zero while that key is down.
pub(super) const KEYS_HELD_SYMBOL: &str = "_mfb_gtkapp_keys_held";
pub(super) const KEYS_HELD_BYTES: usize = 256;
pub(super) const STR_KEY_RELEASED: (&str, &str) = ("_mfb_gtkapp_str_key_released", "key-released");

/// A press (or, for a key already down, a repeat).
pub(super) const KIND_PRESS: &str = "0";
/// A release, injected.
pub(super) const KIND_RELEASE: &str = "1";
/// A release in line input mode: the key is no longer down, and nothing is
/// injected (line mode has no key events).
pub(super) const KIND_FORGET: &str = "2";

/// `GTK_PHASE_CAPTURE`: the window's key controller sees a key before any child
/// widget. In the bubble phase the transcript's `GtkTextView` consumed Backspace
/// and Delete before they reached the window (measured on box 2226).
pub(super) const GTK_PHASE_CAPTURE: &str = "1";

// Frame of the translator.
const FRAME: usize = 96;
const OFF_KEYVAL: usize = 8;
const OFF_KEYCODE: usize = 16;
const OFF_STATE: usize = 24;
const OFF_EVENT: usize = 32;
const OFF_BASE: usize = 40;
const OFF_TEXT: usize = 48;
const OFF_BUF: usize = 56;
const _: () = assert!(OFF_BUF + crate::codegen::io::input::encode::ENCODE_OUT_BYTES <= FRAME);

/// The key translator (see the module doc).
pub(super) fn emit_key_inject_helper() -> Result<CodeFunction, String> {
    let mut asm = Asm::new(KEY_INJECT_SYMBOL);
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(FRAME));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        0,
    ));
    asm.push(abi::store_u64(
        abi::c_arg(0),
        abi::stack_pointer(),
        OFF_KEYVAL,
    ));
    asm.push(abi::store_u64(
        abi::c_arg(1),
        abi::stack_pointer(),
        OFF_KEYCODE,
    ));
    asm.push(abi::store_u64(
        abi::c_arg(2),
        abi::stack_pointer(),
        OFF_STATE,
    ));
    asm.push(abi::move_register(abi::SCRATCH[3], abi::c_arg(3))); // kind

    // held = &keys_held[keycode]; a keycode past the table is never a repeat.
    asm.push(abi::load_u64(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        OFF_KEYCODE,
    ));
    asm.push(abi::compare_immediate(
        abi::SCRATCH[1],
        &(KEYS_HELD_BYTES - 1).to_string(),
    ));
    asm.push(abi::branch_hi("kinj_untracked"));
    asm.local_address(abi::SCRATCH[2], KEYS_HELD_SYMBOL);
    asm.push(abi::add_registers(
        abi::SCRATCH[2],
        abi::SCRATCH[2],
        abi::SCRATCH[1],
    ));
    asm.push(abi::compare_immediate(abi::SCRATCH[3], KIND_PRESS));
    asm.push(abi::branch_ne("kinj_up"));
    asm.push(abi::load_u8(abi::SCRATCH[1], abi::SCRATCH[2], 0));
    asm.push(abi::compare_immediate(abi::SCRATCH[1], "0"));
    asm.push(abi::branch_ne("kinj_repeat"));
    asm.push(abi::move_immediate(abi::SCRATCH[1], "Integer", "1"));
    asm.push(abi::store_u8(abi::SCRATCH[1], abi::SCRATCH[2], 0));
    asm.push(abi::branch("kinj_press"));
    asm.push(abi::label("kinj_up"));
    asm.push(abi::move_immediate(abi::SCRATCH[1], "Integer", "0"));
    asm.push(abi::store_u8(abi::SCRATCH[1], abi::SCRATCH[2], 0));
    asm.push(abi::label("kinj_untracked"));
    asm.push(abi::compare_immediate(abi::SCRATCH[3], KIND_FORGET));
    asm.push(abi::branch_eq("kinj_done"));
    asm.push(abi::compare_immediate(abi::SCRATCH[3], KIND_RELEASE));
    asm.push(abi::branch_ne("kinj_press"));
    asm.push(abi::move_immediate(
        abi::SCRATCH[1],
        "Integer",
        &EVENT_RELEASE.to_string(),
    ));
    asm.push(abi::branch("kinj_have_event"));
    asm.push(abi::label("kinj_repeat"));
    asm.push(abi::move_immediate(
        abi::SCRATCH[1],
        "Integer",
        &EVENT_REPEAT.to_string(),
    ));
    asm.push(abi::branch("kinj_have_event"));
    asm.push(abi::label("kinj_press"));
    asm.push(abi::move_immediate(
        abi::SCRATCH[1],
        "Integer",
        &EVENT_PRESS.to_string(),
    ));
    asm.push(abi::label("kinj_have_event"));
    asm.push(abi::store_u64(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        OFF_EVENT,
    ));

    // base = gdk_keyval_to_unicode(gdk_keyval_to_lower(keyval))
    asm.push(abi::load_u64(
        abi::c_arg(0),
        abi::stack_pointer(),
        OFF_KEYVAL,
    ));
    asm.call_external("gdk_keyval_to_lower");
    asm.push(abi::move_register(abi::c_arg(0), abi::c_return(0)));
    asm.call_external("gdk_keyval_to_unicode");
    asm.push(abi::store_u64(
        abi::c_return(0),
        abi::stack_pointer(),
        OFF_BASE,
    ));
    // text = gdk_keyval_to_unicode(keyval) for a press or repeat.
    asm.push(abi::move_immediate(abi::SCRATCH[1], "Integer", "0"));
    asm.push(abi::store_u64(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        OFF_TEXT,
    ));
    asm.push(abi::load_u64(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        OFF_EVENT,
    ));
    asm.push(abi::compare_immediate(
        abi::SCRATCH[1],
        &EVENT_RELEASE.to_string(),
    ));
    asm.push(abi::branch_eq("kinj_encode"));
    asm.push(abi::load_u64(
        abi::c_arg(0),
        abi::stack_pointer(),
        OFF_KEYVAL,
    ));
    asm.call_external("gdk_keyval_to_unicode");
    asm.push(abi::store_u64(
        abi::c_return(0),
        abi::stack_pointer(),
        OFF_TEXT,
    ));

    // encode(buf, keyval | GDK table << 32, base | event << 24 | state << 32, text)
    asm.push(abi::label("kinj_encode"));
    asm.push(abi::load_u64(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        OFF_KEYVAL,
    ));
    asm.push(abi::move_immediate(
        abi::SCRATCH[2],
        "Integer",
        &((crate::codegen::io::input::encode::KeyTable::Gdk as u64) << PACK_TABLE_SHIFT)
            .to_string(),
    ));
    asm.push(abi::or_registers(
        abi::SCRATCH[1],
        abi::SCRATCH[1],
        abi::SCRATCH[2],
    ));
    asm.push(abi::store_u64(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        OFF_KEYVAL,
    ));
    asm.push(abi::load_u64(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        OFF_STATE,
    ));
    asm.push(abi::shift_left_immediate(
        abi::SCRATCH[1],
        abi::SCRATCH[1],
        PACK_MODS_SHIFT as u8,
    ));
    asm.push(abi::load_u64(
        abi::SCRATCH[2],
        abi::stack_pointer(),
        OFF_EVENT,
    ));
    asm.push(abi::shift_left_immediate(
        abi::SCRATCH[2],
        abi::SCRATCH[2],
        PACK_EVENT_SHIFT as u8,
    ));
    asm.push(abi::or_registers(
        abi::SCRATCH[1],
        abi::SCRATCH[1],
        abi::SCRATCH[2],
    ));
    asm.push(abi::load_u64(
        abi::SCRATCH[2],
        abi::stack_pointer(),
        OFF_BASE,
    ));
    asm.push(abi::or_registers(
        abi::SCRATCH[1],
        abi::SCRATCH[1],
        abi::SCRATCH[2],
    ));
    asm.push(abi::add_immediate(
        abi::c_arg(0),
        abi::stack_pointer(),
        OFF_BUF,
    ));
    asm.push(abi::load_u64(
        abi::c_arg(1),
        abi::stack_pointer(),
        OFF_KEYVAL,
    ));
    asm.push(abi::move_register(abi::c_arg(2), abi::SCRATCH[1]));
    asm.push(abi::load_u64(abi::c_arg(3), abi::stack_pointer(), OFF_TEXT));
    asm.call_internal(INPUT_ENCODE_SYMBOL);

    // write(pipe, buf + 1, len) when the event encodes to anything.
    asm.push(abi::load_u8(abi::c_arg(2), abi::stack_pointer(), OFF_BUF));
    asm.push(abi::compare_immediate(abi::c_arg(2), "0"));
    asm.push(abi::branch_eq("kinj_done"));
    asm.load_state(abi::c_arg(0), ST_PIPE_WRITE_FD);
    asm.push(abi::add_immediate(
        abi::c_arg(1),
        abi::stack_pointer(),
        OFF_BUF + 1,
    ));
    asm.call_external("write");

    asm.push(abi::label("kinj_done"));
    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::add_stack(FRAME));
    asm.push(abi::return_());
    asm.finish(KEY_INJECT_SYMBOL, "Nothing")
}

/// `void _mfb_gtkapp_key_released(GtkEventControllerKey *ctrl, guint keyval,
/// guint keycode, GdkModifierType state, gpointer user_data)`: report the
/// release in raw input mode; in line mode only note the key is up.
pub(super) fn emit_key_released_handler() -> Result<CodeFunction, String> {
    let mut asm = Asm::new(KEY_RELEASED_SYMBOL);
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(32));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        0,
    ));
    asm.push(abi::store_u64(abi::c_arg(1), abi::stack_pointer(), 8)); // keyval
    asm.push(abi::store_u64(abi::c_arg(2), abi::stack_pointer(), 16)); // keycode
    asm.push(abi::store_u64(abi::c_arg(3), abi::stack_pointer(), 24)); // state
    asm.push(abi::move_immediate(abi::SCRATCH[2], "Integer", KIND_FORGET));
    asm.load_state(abi::SCRATCH[1], ST_INPUT_MODE);
    asm.push(abi::compare_immediate(abi::SCRATCH[1], MODE_RAW));
    asm.push(abi::branch_ne("kr_inject"));
    asm.push(abi::move_immediate(
        abi::SCRATCH[2],
        "Integer",
        KIND_RELEASE,
    ));
    asm.push(abi::label("kr_inject"));
    asm.push(abi::load_u64(abi::c_arg(0), abi::stack_pointer(), 8));
    asm.push(abi::load_u64(abi::c_arg(1), abi::stack_pointer(), 16));
    asm.push(abi::load_u64(abi::c_arg(2), abi::stack_pointer(), 24));
    asm.push(abi::move_register(abi::c_arg(3), abi::SCRATCH[2]));
    asm.call_internal(KEY_INJECT_SYMBOL);
    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::add_stack(32));
    asm.push(abi::return_());
    asm.finish(KEY_RELEASED_SYMBOL, "Nothing")
}

/// Wire a new window key controller: `key-pressed`, `key-released`, the capture
/// phase, and hand it to the window. The controller is in `abi::LOCAL[0]`.
pub(super) fn emit_connect_key_controller(asm: &mut Asm) {
    for (signal, handler) in [
        (STR_KEY_PRESSED.0, KEY_PRESSED_SYMBOL),
        (STR_KEY_RELEASED.0, KEY_RELEASED_SYMBOL),
    ] {
        asm.local_address(abi::c_arg(1), signal);
        asm.local_address(abi::c_arg(2), handler);
        asm.push(abi::move_immediate(abi::c_arg(3), "Integer", "0"));
        asm.push(abi::move_immediate(abi::c_arg(4), "Integer", "0"));
        asm.push(abi::move_immediate(abi::c_arg(5), "Integer", "0"));
        asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[0]));
        asm.call_external("g_signal_connect_data");
    }
    asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[0]));
    asm.push(abi::move_immediate(
        abi::c_arg(1),
        "Integer",
        GTK_PHASE_CAPTURE,
    ));
    asm.call_external("gtk_event_controller_set_propagation_phase");
    asm.load_state(abi::c_arg(0), ST_WINDOW);
    asm.push(abi::move_register(abi::c_arg(1), abi::LOCAL[0]));
    asm.call_external("gtk_widget_add_controller"); // takes ownership
}

/// The held-keys table.
pub(super) fn keys_held_data_object() -> CodeDataObject {
    CodeDataObject {
        symbol: KEYS_HELD_SYMBOL.to_string(),
        kind: "raw".to_string(),
        layout: "mfb.runtime.gtkapp_keys_held.v1 { u8 down[256] }".to_string(),
        align: 8,
        size: KEYS_HELD_BYTES,
        value: "00".repeat(KEYS_HELD_BYTES),
    }
}
