//! macOS window title + fullscreen for the `app` window members
//! (`setTitle`/`getTitle`/`setFullscreen`/`getFullscreen`).
//!
//! The members keep their state in the process-global window data
//! (`builtins::app::gen_window`); this module makes the `NSWindow` follow it.
//! Everything here is emitted only for a program that uses a window member
//! (`AppEntrySpec::uses_window`).
//!
//! * **Worker → main.** `setTitle`/`setFullscreen` end in a call to
//!   [`WINDOW_SYNC_MARSHAL_SYMBOL`], which sends `mfbSyncWindow:` to the app
//!   delegate with `performSelectorOnMainThread:…waitUntilDone:YES`. It reads the
//!   delegate from `DELEGATE_GLOBAL_SYM` rather than `[NSApp delegate]`, so it is
//!   safe from any program thread (a `thread::` worker has no autorelease pool and
//!   must not touch `NSApp`). The delegate is nil headless — no run loop to drain
//!   the perform — so the marshal is skipped there.
//! * **The sync** ([`WINDOW_SYNC_SYMBOL`], the `mfbSyncWindow:` IMP) runs on the
//!   main thread: it sets the window's title from the title data (read under the
//!   title lock, which no holder ever keeps while waiting on the main thread), and,
//!   when the window is on screen, sends `toggleFullScreen:` if the window's
//!   `NSWindowStyleMaskFullScreen` bit disagrees with the fullscreen word. The
//!   reconcile calls the same function after it shows the window, so a title or
//!   fullscreen request made in `Mode.None` lands when the window appears.
//! * **User changes.** The delegate observes
//!   `NSWindowDidEnterFullScreenNotification` / `…DidExit…` and writes `1`/`0`
//!   into the fullscreen word, so `getFullscreen` follows the green button too.

use super::*;
use crate::codegen::builtins::app::gen_window::{
    APP_DEFAULT_TITLE_SYMBOL, APP_FULLSCREEN_SYMBOL, APP_TITLE_LOCK_SYMBOL, APP_TITLE_SYMBOL,
};

/// Worker-side marshal: `performSelectorOnMainThread:@selector(mfbSyncWindow:)`.
pub(super) const WINDOW_SYNC_MARSHAL_SYMBOL: &str = "_mfb_macapp_window_sync_marshal";
/// Main-thread IMP of `mfbSyncWindow:`, also called directly by the reconcile.
pub(super) const WINDOW_SYNC_SYMBOL: &str = "_mfb_macapp_window_sync";
/// IMP of `mfbFullscreenEntered:` — stores `1` into the fullscreen word.
const FS_ENTERED_SYMBOL: &str = "_mfb_macapp_fullscreen_entered";
/// IMP of `mfbFullscreenExited:` — stores `0` into the fullscreen word.
const FS_EXITED_SYMBOL: &str = "_mfb_macapp_fullscreen_exited";
/// Main-thread helper that registers the delegate for the two fullscreen
/// notifications. `x0` = the delegate.
const WINDOW_OBSERVE_SYMBOL: &str = "_mfb_macapp_window_observe";

const SEL_MFB_SYNC_WINDOW: (&str, &str) = ("_mfb_macapp_sel_mfbSyncWindow", "mfbSyncWindow:");
const SEL_MFB_FS_ENTERED: (&str, &str) = (
    "_mfb_macapp_sel_mfbFullscreenEntered",
    "mfbFullscreenEntered:",
);
const SEL_MFB_FS_EXITED: (&str, &str) = (
    "_mfb_macapp_sel_mfbFullscreenExited",
    "mfbFullscreenExited:",
);
const SEL_IS_VISIBLE: (&str, &str) = ("_mfb_macapp_sel_isVisible", "isVisible");
const SEL_STYLE_MASK: (&str, &str) = ("_mfb_macapp_sel_styleMask", "styleMask");
const SEL_TOGGLE_FULL_SCREEN: (&str, &str) =
    ("_mfb_macapp_sel_toggleFullScreen", "toggleFullScreen:");
const SEL_INIT_WITH_UTF8: (&str, &str) = (
    "_mfb_macapp_sel_win_initWithUTF8String",
    "initWithUTF8String:",
);
const SEL_DEFAULT_CENTER: (&str, &str) = ("_mfb_macapp_sel_defaultCenter", "defaultCenter");
const SEL_ADD_OBSERVER: (&str, &str) = (
    "_mfb_macapp_sel_addObserver",
    "addObserver:selector:name:object:",
);

/// `NSNotificationCenter`, for the fullscreen observers.
pub(crate) const CLASS_NS_NOTIFICATION_CENTER: &str = "_OBJC_CLASS_$_NSNotificationCenter";
/// The two `NSString * const` notification names (AppKit globals).
pub(crate) const NS_WINDOW_DID_ENTER_FULL_SCREEN: &str = "_NSWindowDidEnterFullScreenNotification";
pub(crate) const NS_WINDOW_DID_EXIT_FULL_SCREEN: &str = "_NSWindowDidExitFullScreenNotification";

/// `NSWindowStyleMaskFullScreen` is `1 << 14`.
const STYLE_MASK_FULL_SCREEN_SHIFT: u8 = 14;

/// The selector strings the window helpers name.
pub(crate) fn window_data_objects() -> Vec<CodeDataObject> {
    [
        SEL_MFB_SYNC_WINDOW,
        SEL_MFB_FS_ENTERED,
        SEL_MFB_FS_EXITED,
        SEL_IS_VISIBLE,
        SEL_STYLE_MASK,
        SEL_TOGGLE_FULL_SCREEN,
        SEL_INIT_WITH_UTF8,
        SEL_DEFAULT_CENTER,
        SEL_ADD_OBSERVER,
    ]
    .into_iter()
    .map(|(symbol, text)| CodeDataObject {
        symbol: symbol.to_string(),
        kind: "raw".to_string(),
        layout: "C string (NUL-terminated)".to_string(),
        align: 1,
        size: text.len() + 1,
        value: hex_cstring(text),
    })
    .collect()
}

/// The window functions, for a program with `uses_window`.
pub(super) fn emit_window_functions() -> Vec<CodeFunction> {
    vec![
        emit_window_sync_marshal(),
        emit_window_sync(),
        emit_fullscreen_flag_imp(FS_ENTERED_SYMBOL, 1),
        emit_fullscreen_flag_imp(FS_EXITED_SYMBOL, 0),
        emit_window_observe(),
    ]
}

/// The worker-side seam `setTitle`/`setFullscreen` append: a plain call to the
/// marshal, which takes no arguments and preserves nothing but what the PCS
/// requires (the vreg allocator already treats the call as clobbering every
/// caller-saved register).
pub(crate) fn emit_window_sync_seam(
    from_symbol: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    instructions.push(abi::branch_link(WINDOW_SYNC_MARSHAL_SYMBOL));
    relocations.push(CodeRelocation {
        from: from_symbol.to_string(),
        to: WINDOW_SYNC_MARSHAL_SYMBOL.to_string(),
        kind: RelocIntent::Call,
        binding: "internal".to_string(),
        library: None,
    });
}

/// Add the three window methods to the delegate class being built in
/// `class_reg` (before `objc_registerClassPair`).
pub(super) fn emit_install_window_methods(asm: &mut Asm, class_reg: &str) {
    for (selector, imp) in [
        (SEL_MFB_SYNC_WINDOW.0, WINDOW_SYNC_SYMBOL),
        (SEL_MFB_FS_ENTERED.0, FS_ENTERED_SYMBOL),
        (SEL_MFB_FS_EXITED.0, FS_EXITED_SYMBOL),
    ] {
        asm.load_selector(selector);
        asm.local_address("x2", imp);
        asm.local_address("x3", STR_INPUT_TYPES.0); // "v@:@"
        asm.push(abi::move_register(abi::c_arg(0), class_reg));
        asm.call_external("_class_addMethod", LIB_OBJC);
    }
}

/// Register the delegate instance in `delegate_reg` for the fullscreen
/// notifications (main thread, once, after the delegate is created).
pub(super) fn emit_observe_call(asm: &mut Asm, delegate_reg: &str) {
    asm.push(abi::move_register(abi::c_arg(0), delegate_reg));
    asm.call_internal(WINDOW_OBSERVE_SYMBOL);
}

/// Call the main-thread sync directly (the reconcile's show arms, already on the
/// main thread). The sync ignores its receiver and arguments.
pub(super) fn emit_sync_call(asm: &mut Asm) {
    asm.call_internal(WINDOW_SYNC_SYMBOL);
}

fn code_function(symbol: &str, asm: Asm) -> CodeFunction {
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

/// `_mfb_macapp_window_sync_marshal` (any program thread): if the delegate has
/// been published, `[delegate performSelectorOnMainThread:@selector(mfbSyncWindow:)
/// withObject:nil waitUntilDone:YES]`. Waiting means a `getFullscreen`/`getTitle`
/// after the call, and the program's next frame, see a window that has at least
/// begun the change.
fn emit_window_sync_marshal() -> CodeFunction {
    let mut asm = Asm::new(WINDOW_SYNC_MARSHAL_SYMBOL);
    let frame = 32;
    let skip = format!("{WINDOW_SYNC_MARSHAL_SYMBOL}_skip");
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(frame));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        0,
    ));
    asm.push(abi::store_u64(abi::LOCAL[0], abi::stack_pointer(), 8));
    asm.push(abi::store_u64(abi::LOCAL[1], abi::stack_pointer(), 16));
    asm.local_address(abi::LOCAL[0], DELEGATE_GLOBAL_SYM);
    asm.push(abi::load_u64(abi::LOCAL[0], abi::LOCAL[0], 0));
    asm.push(abi::compare_immediate(abi::LOCAL[0], "0"));
    asm.push(abi::branch_eq(&skip)); // headless: no delegate, no run loop
    asm.load_selector(SEL_MFB_SYNC_WINDOW.0);
    asm.push(abi::move_register(abi::LOCAL[1], abi::c_arg(1)));
    asm.load_selector(SEL_PERFORM_ON_MAIN.0);
    asm.push(abi::move_register(abi::c_arg(2), abi::LOCAL[1]));
    asm.push(abi::move_immediate(abi::c_arg(3), "Integer", "0")); // withObject: nil
    asm.push(abi::move_immediate(abi::c_arg(4), "Integer", "1")); // waitUntilDone: YES
    asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[0]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::label(&skip));
    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::load_u64(abi::LOCAL[0], abi::stack_pointer(), 8));
    asm.push(abi::load_u64(abi::LOCAL[1], abi::stack_pointer(), 16));
    asm.push(abi::add_stack(frame));
    asm.push(abi::return_());
    code_function(WINDOW_SYNC_MARSHAL_SYMBOL, asm)
}

/// `_mfb_macapp_window_sync` (main thread): make the window match the window
/// data. No window yet (a `None`-start program before its first `setMode`) → do
/// nothing; the reconcile calls this again once it has built and shown one.
///
/// The title NSString is built with `alloc`/`initWithUTF8String:` and released
/// after `setTitle:` (which copies it) rather than autoreleased, so a program that
/// retitles every frame does not grow the main thread's pool between drains. It is
/// built under the title lock — the block can be freed by a `setTitle` on another
/// thread the moment the lock drops — and the lock is released before any window
/// send. `initWithUTF8String:` answers nil for bytes that are not UTF-8; the title
/// is then left as it was rather than sending `setTitle:nil`, which raises.
///
/// Fullscreen is synced only while the window is visible: a hidden (`Mode.None`)
/// window cannot enter a fullscreen space, and the reconcile re-runs this after it
/// shows the window.
fn emit_window_sync() -> CodeFunction {
    let mut asm = Asm::new(WINDOW_SYNC_SYMBOL);
    let frame = 48;
    let done = format!("{WINDOW_SYNC_SYMBOL}_done");
    let have_title = format!("{WINDOW_SYNC_SYMBOL}_have_title");
    let skip_title = format!("{WINDOW_SYNC_SYMBOL}_skip_title");
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(frame));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        0,
    ));
    asm.push(abi::store_u64(abi::LOCAL[0], abi::stack_pointer(), 8));
    asm.push(abi::store_u64(abi::LOCAL[1], abi::stack_pointer(), 16));
    asm.push(abi::store_u64(abi::LOCAL[2], abi::stack_pointer(), 24));
    asm.push(abi::store_u64(abi::LOCAL[3], abi::stack_pointer(), 32));
    // app = [NSApplication sharedApplication]; window = assoc(app, WINDOW_ASSOC_KEY)
    asm.external_data(abi::LOCAL[0], CLASS_NS_APPLICATION, LIB_APPKIT);
    asm.load_selector(SEL_SHARED_APPLICATION.0);
    asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[0]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.local_address("x1", WINDOW_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[1], abi::c_arg(0))); // window (or nil)
    asm.push(abi::compare_immediate(abi::LOCAL[1], "0"));
    asm.push(abi::branch_eq(&done));

    // --- title ---
    asm.local_address(abi::c_arg(0), APP_TITLE_LOCK_SYMBOL);
    asm.call_external("_pthread_mutex_lock", LIB_SYSTEM);
    asm.local_address(abi::LOCAL[2], APP_TITLE_SYMBOL);
    asm.push(abi::load_u64(abi::LOCAL[2], abi::LOCAL[2], 0));
    asm.push(abi::compare_immediate(abi::LOCAL[2], "0"));
    asm.push(abi::branch_ne(&have_title));
    asm.local_address(abi::LOCAL[2], APP_DEFAULT_TITLE_SYMBOL);
    asm.push(abi::label(&have_title));
    // str = [[NSString alloc] initWithUTF8String:block + 8]
    asm.external_data(abi::LOCAL[3], CLASS_NS_STRING, LIB_FOUNDATION);
    asm.load_selector(SEL_ALLOC.0);
    asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[3]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[3], abi::c_arg(0)));
    asm.load_selector(SEL_INIT_WITH_UTF8.0);
    asm.push(abi::add_immediate(abi::c_arg(2), abi::LOCAL[2], 8));
    asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[3]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[3], abi::c_arg(0))); // str (or nil)
    asm.local_address(abi::c_arg(0), APP_TITLE_LOCK_SYMBOL);
    asm.call_external("_pthread_mutex_unlock", LIB_SYSTEM);
    asm.push(abi::compare_immediate(abi::LOCAL[3], "0"));
    asm.push(abi::branch_eq(&skip_title));
    asm.load_selector(SEL_SET_TITLE.0);
    asm.push(abi::move_register(abi::c_arg(2), abi::LOCAL[3]));
    asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[1]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.load_selector(SEL_RELEASE.0);
    asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[3]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::label(&skip_title));

    // --- fullscreen ---
    // visible = [window isVisible]; a BOOL is only defined in its low byte.
    asm.load_selector(SEL_IS_VISIBLE.0);
    asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[1]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", "255"));
    asm.push(abi::and_registers(
        abi::c_arg(0),
        abi::c_arg(0),
        abi::SCRATCH[0],
    ));
    asm.push(abi::compare_immediate(abi::c_arg(0), "0"));
    asm.push(abi::branch_eq(&done));
    // now = ([window styleMask] >> 14) & 1
    asm.load_selector(SEL_STYLE_MASK.0);
    asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[1]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::shift_right_immediate(
        abi::c_arg(0),
        abi::c_arg(0),
        STYLE_MASK_FULL_SCREEN_SHIFT,
    ));
    asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", "1"));
    asm.push(abi::and_registers(
        abi::c_arg(0),
        abi::c_arg(0),
        abi::SCRATCH[0],
    ));
    asm.local_address(abi::SCRATCH[1], APP_FULLSCREEN_SYMBOL);
    asm.push(abi::load_u64(abi::SCRATCH[1], abi::SCRATCH[1], 0));
    asm.push(abi::compare_registers(abi::c_arg(0), abi::SCRATCH[1]));
    asm.push(abi::branch_eq(&done));
    asm.load_selector(SEL_TOGGLE_FULL_SCREEN.0);
    asm.push(abi::move_immediate(abi::c_arg(2), "Integer", "0")); // sender: nil
    asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[1]));
    asm.call_external("_objc_msgSend", LIB_OBJC);

    asm.push(abi::label(&done));
    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::load_u64(abi::LOCAL[0], abi::stack_pointer(), 8));
    asm.push(abi::load_u64(abi::LOCAL[1], abi::stack_pointer(), 16));
    asm.push(abi::load_u64(abi::LOCAL[2], abi::stack_pointer(), 24));
    asm.push(abi::load_u64(abi::LOCAL[3], abi::stack_pointer(), 32));
    asm.push(abi::add_stack(frame));
    asm.push(abi::return_());
    code_function(WINDOW_SYNC_SYMBOL, asm)
}

/// A fullscreen-notification IMP: store `value` into the fullscreen word. A leaf
/// — it touches only scratch registers and makes no call.
fn emit_fullscreen_flag_imp(symbol: &str, value: u32) -> CodeFunction {
    let mut asm = Asm::new(symbol);
    asm.push(abi::label("entry"));
    asm.local_address(abi::SCRATCH[0], APP_FULLSCREEN_SYMBOL);
    asm.push(abi::move_immediate(
        abi::SCRATCH[1],
        "Integer",
        &value.to_string(),
    ));
    asm.push(abi::store_u64(abi::SCRATCH[1], abi::SCRATCH[0], 0));
    asm.push(abi::return_());
    code_function(symbol, asm)
}

/// `_mfb_macapp_window_observe(delegate)` (main thread): register `delegate` with
/// the default notification center for the window's enter/exit-fullscreen
/// notifications (`object:nil` — the program has one window, and a rebuilt window
/// needs no re-registration).
fn emit_window_observe() -> CodeFunction {
    let mut asm = Asm::new(WINDOW_OBSERVE_SYMBOL);
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
    asm.push(abi::store_u64(abi::LOCAL[2], abi::stack_pointer(), 24));
    asm.push(abi::move_register(abi::LOCAL[0], abi::c_arg(0))); // delegate
    asm.external_data(abi::LOCAL[1], CLASS_NS_NOTIFICATION_CENTER, LIB_FOUNDATION);
    asm.load_selector(SEL_DEFAULT_CENTER.0);
    asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[1]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[1], abi::c_arg(0))); // center
    for (selector, name) in [
        (SEL_MFB_FS_ENTERED.0, NS_WINDOW_DID_ENTER_FULL_SCREEN),
        (SEL_MFB_FS_EXITED.0, NS_WINDOW_DID_EXIT_FULL_SCREEN),
    ] {
        asm.load_selector(selector);
        asm.push(abi::move_register(abi::LOCAL[2], abi::c_arg(1)));
        asm.load_selector(SEL_ADD_OBSERVER.0);
        asm.push(abi::move_register(abi::c_arg(2), abi::LOCAL[0])); // observer
        asm.push(abi::move_register(abi::c_arg(3), abi::LOCAL[2])); // selector
                                                                    // The name is an `NSString * const` global: external_data yields the
                                                                    // variable's address, so dereference once more for the string.
        asm.external_data("x4", name, LIB_APPKIT);
        asm.push(abi::load_u64(abi::c_arg(4), abi::c_arg(4), 0));
        asm.push(abi::move_immediate(abi::c_arg(5), "Integer", "0")); // object: nil
        asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[1]));
        asm.call_external("_objc_msgSend", LIB_OBJC);
    }
    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::load_u64(abi::LOCAL[0], abi::stack_pointer(), 8));
    asm.push(abi::load_u64(abi::LOCAL[1], abi::stack_pointer(), 16));
    asm.push(abi::load_u64(abi::LOCAL[2], abi::stack_pointer(), 24));
    asm.push(abi::add_stack(frame));
    asm.push(abi::return_());
    code_function(WINDOW_OBSERVE_SYMBOL, asm)
}
