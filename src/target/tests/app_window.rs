//! The `app` window members (`setTitle`/`getTitle`/`setFullscreen`/`getFullscreen`)
//! lowered in `-app` mode on every backend that has a window.
//!
//! Each toolkit makes its native window follow the process-global window data
//! (`builtins::app::gen_window`) with its own emitted helpers, and none of the
//! corpus fixtures call a window member, so those helpers were lowered by
//! nothing. A program that uses them must emit every helper the call sites name
//! and import every system function those helpers call: a missing helper is an
//! undefined symbol at link time, a missing import is a crash at the first
//! `setTitle`.

use crate::codegen::engine::types::{CodeFunction, NativeCodePlan};
use crate::testutil::{app_code_cached, CodeTarget};

pub(super) const WINDOW_PROGRAM: &str = "\
IMPORT app
IMPORT io

FUNC main() AS Integer
  app::setTitle(\"Demo\")
  app::setFullscreen(TRUE)
  io::print(app::getTitle())
  io::print(toString(app::getFullscreen()))
  RETURN 0
END FUNC
";

pub(super) fn function<'a>(plan: &'a NativeCodePlan, symbol: &str) -> &'a CodeFunction {
    plan.functions
        .iter()
        .find(|function| function.symbol == symbol)
        .unwrap_or_else(|| panic!("`{symbol}` was not emitted for a window program"))
}

/// Every symbol `function` reaches through a relocation.
pub(super) fn targets(function: &CodeFunction) -> Vec<&str> {
    function
        .relocations
        .iter()
        .map(|relocation| relocation.to.as_str())
        .collect()
}

pub(super) fn mnemonics(function: &CodeFunction) -> Vec<&'static str> {
    function
        .instructions
        .iter()
        .map(|instruction| instruction.op.mnemonic())
        .collect()
}

fn cstring_hex(text: &str) -> String {
    let mut hex: String = text.bytes().map(|byte| format!("{byte:02x}")).collect();
    hex.push_str("00");
    hex
}

/// macOS: the sync IMP titles the `NSWindow`, then toggles fullscreen only when
/// the style-mask bit disagrees with the fullscreen word; the two notification
/// IMPs are single stores of `1` / `0` into that word; the observer registers the
/// delegate for both notifications; the marshal performs the sync on the main
/// thread.
#[test]
fn macos_window_members_emit_the_sync_marshal_and_fullscreen_observers() {
    let plan = app_code_cached(WINDOW_PROGRAM, CodeTarget::MacosAarch64);

    let sync = function(plan, "_mfb_macapp_window_sync");
    let reached = targets(sync);
    for symbol in [
        "_objc_getAssociatedObject",
        "_pthread_mutex_lock",
        "_pthread_mutex_unlock",
        "_objc_msgSend",
    ] {
        assert!(
            reached.contains(&symbol),
            "the window sync must call {symbol}; it reaches {reached:?}"
        );
    }
    // The lock is released before any window send: lock, unlock, then sends.
    let lock = reached.iter().position(|s| *s == "_pthread_mutex_lock");
    let unlock = reached.iter().position(|s| *s == "_pthread_mutex_unlock");
    assert!(
        lock < unlock,
        "the title lock is taken before it is dropped"
    );

    for (symbol, value) in [
        ("_mfb_macapp_fullscreen_entered", "1"),
        ("_mfb_macapp_fullscreen_exited", "0"),
    ] {
        let imp = function(plan, symbol);
        assert!(
            targets(imp)
                .iter()
                .all(|target| !target.starts_with("_objc") && !target.starts_with("_pthread")),
            "{symbol} is a leaf: it only stores into the fullscreen word"
        );
        assert!(
            mnemonics(imp).contains(&"str_u64"),
            "{symbol} must store the fullscreen word; has {:?}",
            mnemonics(imp)
        );
        let stored = imp.instructions.iter().find_map(|instruction| {
            (instruction.op.mnemonic() == "mov_imm").then(|| instruction.get("value"))
        });
        assert_eq!(stored.flatten().as_deref(), Some(value), "{symbol}");
    }

    let observe = function(plan, "_mfb_macapp_window_observe");
    let observed = targets(observe);
    for notification in [
        "_NSWindowDidEnterFullScreenNotification",
        "_NSWindowDidExitFullScreenNotification",
    ] {
        assert!(
            observed.contains(&notification),
            "the observer must register for {notification}; reaches {observed:?}"
        );
    }

    let marshal = function(plan, "_mfb_macapp_window_sync_marshal");
    assert!(targets(marshal).contains(&"_objc_msgSend"));

    for selector in [
        "mfbSyncWindow:",
        "mfbFullscreenEntered:",
        "toggleFullScreen:",
    ] {
        let wanted = cstring_hex(selector);
        assert!(
            plan.data_objects
                .iter()
                .any(|object| object.value == wanted),
            "the selector string `{selector}` must be a data object"
        );
    }
}

fn imported(plan: &NativeCodePlan, symbol: &str) -> bool {
    plan.imports.iter().any(|import| import.symbol == symbol)
}

fn has_data(plan: &NativeCodePlan, symbol: &str) -> bool {
    plan.data_objects
        .iter()
        .any(|object| object.symbol == symbol)
}

/// Linux (both ISAs): the sync callback fullscreens/unfullscreens the GTK window
/// under the title lock, the `notify::fullscreened` handler stores the toolkit's
/// own state back, and the signal-name string it connects under is a data object.
#[test]
fn linux_window_members_emit_the_gtk_sync_and_the_fullscreen_notify_handler() {
    for target in [CodeTarget::LinuxX86_64, CodeTarget::LinuxAarch64] {
        let plan = app_code_cached(WINDOW_PROGRAM, target);
        let name = target.name();

        for symbol in [
            "gtk_window_fullscreen",
            "gtk_window_unfullscreen",
            "gtk_window_is_fullscreen",
            "gtk_widget_get_visible",
            "pthread_mutex_lock",
            "pthread_mutex_unlock",
        ] {
            assert!(imported(plan, symbol), "{name}: must import {symbol}");
        }

        let sync = targets(function(plan, "_mfb_gtkapp_window_sync"));
        for symbol in ["gtk_window_fullscreen", "gtk_window_unfullscreen"] {
            assert!(
                sync.contains(&symbol),
                "{name}: the sync must call {symbol}; it reaches {sync:?}"
            );
        }
        let notify = targets(function(plan, "_mfb_gtkapp_fullscreen_notify"));
        assert!(
            notify.contains(&"gtk_window_is_fullscreen"),
            "{name}: the handler stores gtk_window_is_fullscreen; it reaches {notify:?}"
        );

        let signal = plan
            .data_objects
            .iter()
            .find(|object| object.symbol == "_mfb_gtkapp_str_notify_fullscreened")
            .unwrap_or_else(|| panic!("{name}: the notify signal name must be a data object"));
        assert_eq!(signal.value, cstring_hex("notify::fullscreened"), "{name}");
        assert_eq!(signal.size, "notify::fullscreened".len() + 1, "{name}");
    }
}

/// Windows: the UI-thread sync strips and restores the window style around a
/// monitor-covering `SetWindowPos`, and the three fullscreen bookkeeping globals
/// (applied flag, saved style, the 44-byte `WINDOWPLACEMENT` rounded to 48) exist.
#[test]
fn windows_window_members_emit_the_win32_sync_and_its_bookkeeping_globals() {
    let plan = app_code_cached(WINDOW_PROGRAM, CodeTarget::WindowsX86_64);

    let sync = targets(function(plan, "_mfb_winapp_window_sync"));
    for symbol in ["SetWindowPos", "GetWindowPlacement", "SetWindowPlacement"] {
        assert!(
            sync.contains(&symbol),
            "the Win32 sync must call {symbol}; it reaches {sync:?}"
        );
    }
    for symbol in [
        "_mfb_winapp_fs_applied",
        "_mfb_winapp_fs_saved_style",
        "_mfb_winapp_fs_placement",
    ] {
        assert!(has_data(plan, symbol), "{symbol} must be a data object");
    }
    let placement = plan
        .data_objects
        .iter()
        .find(|object| object.symbol == "_mfb_winapp_fs_placement")
        .unwrap();
    assert_eq!(placement.size, 48);
    assert_eq!(placement.align, 8);
    assert_eq!(placement.value, "00".repeat(48));
}
