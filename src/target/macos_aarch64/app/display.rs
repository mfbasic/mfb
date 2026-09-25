//! The display refresh rate, published by the main thread (plan-155-A Phase 2).
//!
//! `[[window screen] maximumFramesPerSecond] × 1000` goes into
//! `GRAPHICS_OFFSET_DISPLAY_MHZ` at two moments:
//!
//! * every entry into canvas mode, from the canvas reconcile, after the view is the
//!   window's content view; and
//! * `NSWindowDidChangeScreenNotification`, which AppKit posts when the window moves
//!   to another screen. `MFBCanvasView` carries the observing method
//!   (`mfbScreenChanged:`), and the reconcile registers the view the one time it
//!   builds it.
//!
//! **The rate is the screen's maximum,** not its current adaptive rate: a ProMotion
//! panel lowers its rate for static content, and a pacing floor wants the fastest
//! rate the display can take. **`maximumFramesPerSecond` is macOS 12+** and the
//! binary's minimum is 11.0 (`build_version`), so the send is guarded with
//! `respondsToSelector:` and an older system publishes 0 ("unknown"). A window not
//! yet on a screen answers nil for `screen`; the main screen stands in until the
//! screen-change notification names the real one.
//!
//! Publishing signals nothing (`.ai/canvas-threading.md` §4): a new rate changes no
//! pixel.

use super::*;
use crate::codegen::runtime::canvas::emit_publish_display_rate;

/// Main-thread helper: `x0` = the window (nil allowed). Publishes the rate.
pub(super) const DISPLAY_RATE_SYMBOL: &str = "_mfb_macapp_display_rate";
/// IMP of `-[MFBCanvasView mfbScreenChanged:]`: republish for `[note object]`.
pub(super) const SCREEN_CHANGED_SYMBOL: &str = "_mfb_macapp_screen_changed";

pub(crate) const CLASS_NS_SCREEN: &str = "_OBJC_CLASS_$_NSScreen";
/// The `NSString * const` AppKit posts when a window changes screen.
pub(crate) const NS_WINDOW_DID_CHANGE_SCREEN: &str = "_NSWindowDidChangeScreenNotification";

pub(super) const SEL_MFB_SCREEN_CHANGED: (&str, &str) = (
    "_mfb_macapp_sel_mfbScreenChanged",
    "mfbScreenChanged:",
);
const SEL_OBJECT: (&str, &str) = ("_mfb_macapp_sel_display_object", "object");
const SEL_MAIN_SCREEN: (&str, &str) = ("_mfb_macapp_sel_mainScreen", "mainScreen");
const SEL_RESPONDS_TO_SELECTOR: (&str, &str) = (
    "_mfb_macapp_sel_respondsToSelector",
    "respondsToSelector:",
);
const SEL_MAXIMUM_FRAMES_PER_SECOND: (&str, &str) = (
    "_mfb_macapp_sel_maximumFramesPerSecond",
    "maximumFramesPerSecond",
);
/// Distinct symbols from `window.rs`'s pair: those are emitted only for a program
/// using a window member, and a program can use both.
const SEL_DEFAULT_CENTER: (&str, &str) = ("_mfb_macapp_sel_display_defaultCenter", "defaultCenter");
const SEL_ADD_OBSERVER: (&str, &str) = (
    "_mfb_macapp_sel_display_addObserver",
    "addObserver:selector:name:object:",
);

/// The selector strings, part of the reconcile data-object set.
pub(super) fn display_data_objects() -> Vec<(&'static str, &'static str)> {
    vec![
        SEL_MFB_SCREEN_CHANGED,
        SEL_OBJECT,
        SEL_MAIN_SCREEN,
        SEL_RESPONDS_TO_SELECTOR,
        SEL_MAXIMUM_FRAMES_PER_SECOND,
        SEL_DEFAULT_CENTER,
        SEL_ADD_OBSERVER,
    ]
}

/// The two functions, for a canvas program.
pub(super) fn emit_display_functions() -> Vec<CodeFunction> {
    vec![emit_display_rate(), emit_screen_changed()]
}

/// `class_addMethod(cls, @selector(mfbScreenChanged:), imp, "v@:@")`, for the canvas
/// view class being built in `class_reg` (before `objc_registerClassPair`).
pub(super) fn emit_install_screen_changed_method(asm: &mut Asm, class_reg: &str) {
    asm.load_selector(SEL_MFB_SCREEN_CHANGED.0);
    asm.local_address("x2", SCREEN_CHANGED_SYMBOL);
    asm.local_address("x3", STR_INPUT_TYPES.0); // "v@:@"
    asm.push(abi::move_register(abi::c_arg(0), class_reg));
    asm.call_external("_class_addMethod", LIB_OBJC);
}

/// `[[NSNotificationCenter defaultCenter] addObserver:view
/// selector:@selector(mfbScreenChanged:) name:NSWindowDidChangeScreenNotification
/// object:nil]`. `object:nil` because the program has one window and a rebuilt one
/// needs no re-registration — `window.rs`'s fullscreen observers make the same choice.
///
/// All three registers must be callee-saved and owned by the caller: every send below
/// is a call, and `load_selector` itself calls `sel_registerName`, so neither the
/// center nor the observing SEL may wait in an argument register.
pub(super) fn emit_observe_screen_changes(
    asm: &mut Asm,
    view_reg: &str,
    center_reg: &str,
    sel_reg: &str,
) {
    asm.external_data(center_reg, CLASS_NS_NOTIFICATION_CENTER, LIB_FOUNDATION);
    asm.load_selector(SEL_DEFAULT_CENTER.0);
    asm.push(abi::move_register(abi::c_arg(0), center_reg));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register(center_reg, abi::c_return(0))); // center
    asm.load_selector(SEL_MFB_SCREEN_CHANGED.0);
    asm.push(abi::move_register(sel_reg, abi::c_arg(1)));
    asm.load_selector(SEL_ADD_OBSERVER.0);
    asm.push(abi::move_register(abi::c_arg(2), view_reg)); // observer
    asm.push(abi::move_register(abi::c_arg(3), sel_reg)); // selector
    // The name is an `NSString * const` global: external_data yields the variable's
    // address, so dereference once more for the string.
    asm.external_data("x4", NS_WINDOW_DID_CHANGE_SCREEN, LIB_APPKIT);
    asm.push(abi::load_u64(abi::c_arg(4), abi::c_arg(4), 0));
    asm.push(abi::move_immediate(abi::c_arg(5), "Integer", "0")); // object: nil
    asm.push(abi::move_register(abi::c_arg(0), center_reg));
    asm.call_external("_objc_msgSend", LIB_OBJC);
}

/// `_mfb_macapp_display_rate(window)` — publish the window's screen's maximum rate.
fn emit_display_rate() -> CodeFunction {
    let mut asm = Asm::new(DISPLAY_RATE_SYMBOL);
    let have_screen = format!("{DISPLAY_RATE_SYMBOL}_have_screen");
    let unknown = format!("{DISPLAY_RATE_SYMBOL}_unknown");
    let publish = format!("{DISPLAY_RATE_SYMBOL}_publish");
    // lr@0, LOCAL[0]@8 (the screen), LOCAL[1]@16 (the maximumFramesPerSecond SEL).
    let frame = 32;
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(frame));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        0,
    ));
    asm.push(abi::store_u64(abi::LOCAL[0], abi::stack_pointer(), 8));
    asm.push(abi::store_u64(abi::LOCAL[1], abi::stack_pointer(), 16));
    asm.push(abi::move_register(abi::LOCAL[0], abi::c_arg(0))); // window

    // screen = window ? [window screen] : nil
    asm.push(abi::compare_immediate(abi::LOCAL[0], "0"));
    asm.push(abi::branch_eq(&format!("{DISPLAY_RATE_SYMBOL}_no_window")));
    asm.load_selector(metal::SEL_SCREEN.0);
    asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[0]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[0], abi::c_return(0)));
    asm.push(abi::compare_immediate(abi::LOCAL[0], "0"));
    asm.push(abi::branch_ne(&have_screen));
    asm.push(abi::label(&format!("{DISPLAY_RATE_SYMBOL}_no_window")));
    // Not on a screen yet: the main screen stands in.
    asm.external_data(abi::LOCAL[0], CLASS_NS_SCREEN, LIB_APPKIT);
    asm.load_selector(SEL_MAIN_SCREEN.0);
    asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[0]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[0], abi::c_return(0)));
    asm.push(abi::compare_immediate(abi::LOCAL[0], "0"));
    asm.push(abi::branch_eq(&unknown));
    asm.push(abi::label(&have_screen));

    // if (![screen respondsToSelector:@selector(maximumFramesPerSecond)]) → unknown
    asm.load_selector(SEL_MAXIMUM_FRAMES_PER_SECOND.0);
    asm.push(abi::move_register(abi::LOCAL[1], abi::c_arg(1)));
    asm.load_selector(SEL_RESPONDS_TO_SELECTOR.0);
    asm.push(abi::move_register(abi::c_arg(2), abi::LOCAL[1]));
    asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[0]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    // A BOOL is one byte; the upper bits of x0 are unspecified.
    asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", "255"));
    asm.push(abi::and_registers(
        abi::c_return(0),
        abi::c_return(0),
        abi::SCRATCH[0],
    ));
    asm.push(abi::compare_immediate(abi::c_return(0), "0"));
    asm.push(abi::branch_eq(&unknown));

    // mHz = [screen maximumFramesPerSecond] × 1000 (an NSInteger)
    asm.push(abi::move_register(abi::c_arg(1), abi::LOCAL[1]));
    asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[0]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", "1000"));
    asm.push(abi::multiply_registers(
        abi::SCRATCH[1],
        abi::c_return(0),
        abi::SCRATCH[0],
    ));
    asm.push(abi::branch(&publish));

    asm.push(abi::label(&unknown));
    asm.push(abi::move_immediate(abi::SCRATCH[1], "Integer", "0"));

    asm.push(abi::label(&publish));
    emit_publish_display_rate(
        DISPLAY_RATE_SYMBOL,
        abi::SCRATCH[0],
        abi::SCRATCH[1],
        &mut asm.ins,
        &mut asm.rel,
    );
    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::load_u64(abi::LOCAL[0], abi::stack_pointer(), 8));
    asm.push(abi::load_u64(abi::LOCAL[1], abi::stack_pointer(), 16));
    asm.push(abi::add_stack(frame));
    asm.push(abi::return_());
    display_code_function(DISPLAY_RATE_SYMBOL, asm)
}

/// `-[MFBCanvasView mfbScreenChanged:(NSNotification *)note]` — `x2` is the note,
/// whose `object` is the window that moved.
fn emit_screen_changed() -> CodeFunction {
    let mut asm = Asm::new(SCREEN_CHANGED_SYMBOL);
    let frame = 16;
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(frame));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        0,
    ));
    asm.push(abi::store_u64(abi::LOCAL[0], abi::stack_pointer(), 8));
    asm.push(abi::move_register(abi::LOCAL[0], abi::c_arg(2))); // note
    asm.load_selector(SEL_OBJECT.0);
    asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[0]));
    asm.call_external("_objc_msgSend", LIB_OBJC); // x0 = the window
    asm.call_internal(DISPLAY_RATE_SYMBOL);
    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::load_u64(abi::LOCAL[0], abi::stack_pointer(), 8));
    asm.push(abi::add_stack(frame));
    asm.push(abi::return_());
    display_code_function(SCREEN_CHANGED_SYMBOL, asm)
}

fn display_code_function(symbol: &str, asm: Asm) -> CodeFunction {
    CodeFunction {
        name: symbol.to_string(),
        symbol: symbol.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame: CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        stack_slots: Vec::new(),
        instructions: asm.ins,
        relocations: asm.rel,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn referenced(func: &CodeFunction) -> Vec<&str> {
        func.relocations.iter().map(|r| r.to.as_str()).collect()
    }

    /// The canvas reconcile registers the screen-change observer and publishes the
    /// rate on entry — and a program that never draws does neither, because both
    /// name symbols only a canvas program emits.
    #[test]
    fn the_canvas_reconcile_publishes_and_observes_only_for_a_canvas_program() {
        let canvas = emit_reconcile_canvas_helper(true, false);
        let names = referenced(&canvas);
        assert!(names.contains(&NS_WINDOW_DID_CHANGE_SCREEN));
        assert!(names.contains(&SEL_ADD_OBSERVER.0));
        assert!(names.contains(&DISPLAY_RATE_SYMBOL));
        let plain = emit_reconcile_canvas_helper(false, false);
        let names = referenced(&plain);
        assert!(!names.contains(&NS_WINDOW_DID_CHANGE_SCREEN));
        assert!(!names.contains(&DISPLAY_RATE_SYMBOL));
        assert!(!names.contains(&SCREEN_CHANGED_SYMBOL));
    }

    /// `maximumFramesPerSecond` is macOS 12+ and the binary's minimum is 11.0, so the
    /// helper must ask `respondsToSelector:` before it sends it, and must publish into
    /// the graphics state.
    #[test]
    fn the_rate_send_is_guarded_and_published() {
        let func = emit_display_rate();
        let names = referenced(&func);
        let responds = names
            .iter()
            .position(|n| *n == SEL_RESPONDS_TO_SELECTOR.0)
            .expect("the respondsToSelector: guard");
        let sends = names
            .iter()
            .filter(|n| **n == "_objc_msgSend")
            .count();
        assert_eq!(sends, 4, "screen, mainScreen, respondsToSelector:, the rate");
        assert!(names[responds..].contains(&"_objc_msgSend"));
        assert!(names.contains(&crate::codegen::runtime::canvas::GRAPHICS_STATE_SYMBOL));
    }

    /// The IMP reads the window off the notification and hands it to the helper.
    #[test]
    fn the_screen_change_imp_republishes_for_the_notifying_window() {
        let names_owned = emit_screen_changed();
        let names = referenced(&names_owned);
        assert!(names.contains(&SEL_OBJECT.0));
        assert!(names.contains(&DISPLAY_RATE_SYMBOL));
    }
}
