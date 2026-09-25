//! GTK4 window title + fullscreen for the `app` window members
//! (`setTitle`/`getTitle`/`setFullscreen`/`getFullscreen`).
//!
//! The members keep their state in the process-global window data
//! (`builtins::app::gen_window`); this module makes the `GtkWindow` follow it.
//! Everything here is emitted only for a program that uses a window member
//! (`AppEntrySpec::uses_window`).
//!
//! * **Worker → main loop.** `setTitle`/`setFullscreen` end in
//!   [`emit_window_sync_seam`]: `g_idle_add(_mfb_gtkapp_window_sync, NULL)`, the same
//!   fire-and-forget marshal the mode reconcile uses. Idles of one priority run in
//!   the order they were added, so a `setMode` followed by a `setTitle` builds the
//!   window before syncing it. Skipped headless (`ST_APPLICATION` is never set
//!   there): no main loop runs, so every queued idle would be a leak — and a
//!   program that retitles every frame would queue one per frame.
//! * **The sync** ([`WINDOW_SYNC_SYMBOL`]) runs on the main loop: it sets the title
//!   from the title data (read under the title lock, which no holder keeps while
//!   waiting on the main loop), then — only while the window is visible —
//!   `gtk_window_fullscreen`/`unfullscreen` to match the fullscreen word. Both are
//!   idempotent requests to the window manager, so no "already fullscreen?" test is
//!   needed. The reconcile calls it after presenting the window, so a request made
//!   in `Mode.None` lands when the window appears.
//! * **User changes.** Each built window connects `notify::fullscreened` to
//!   [`FS_NOTIFY_SYMBOL`], which stores `gtk_window_is_fullscreen` into the
//!   fullscreen word, so `getFullscreen` follows a window-manager shortcut too.

use super::*;
use crate::codegen::builtins::app::gen_window::{
    APP_DEFAULT_TITLE_SYMBOL, APP_FULLSCREEN_SYMBOL, APP_TITLE_LOCK_SYMBOL, APP_TITLE_SYMBOL,
};

/// The `g_idle_add` callback (also called directly by the reconcile) that makes
/// the window follow the window data. Returns `G_SOURCE_REMOVE`.
pub(super) const WINDOW_SYNC_SYMBOL: &str = "_mfb_gtkapp_window_sync";
/// `notify::fullscreened` handler: stores the window's fullscreen state.
const FS_NOTIFY_SYMBOL: &str = "_mfb_gtkapp_fullscreen_notify";
const STR_NOTIFY_FULLSCREENED: (&str, &str) = (
    "_mfb_gtkapp_str_notify_fullscreened",
    "notify::fullscreened",
);
/// `gboolean` is a C `int`: only the low 32 bits of the return register are
/// defined (x86-64 leaves the upper half of `rax` unspecified).
const GBOOLEAN_MASK: &str = "4294967295";

/// The signal-name string the window helpers connect under.
pub(crate) fn window_data_objects() -> Vec<CodeDataObject> {
    let (symbol, text) = STR_NOTIFY_FULLSCREENED;
    vec![CodeDataObject {
        symbol: symbol.to_string(),
        kind: "raw".to_string(),
        layout: "C string (NUL-terminated)".to_string(),
        align: 1,
        size: text.len() + 1,
        value: hex_cstring(text),
    }]
}

/// The toolkit + lock imports the window helpers call. `libpthread` is the
/// flavor's pthread soname (glibc's `libpthread.so.0`, musl's libc).
pub(crate) fn app_window_imports(
    libc_names: AppLibcNames,
) -> Vec<crate::target::shared::plan::PlatformImport> {
    use crate::target::shared::plan::PlatformImport;
    [
        (GTK, "gtk_window_fullscreen"),
        (GTK, "gtk_window_unfullscreen"),
        (GTK, "gtk_window_is_fullscreen"),
        (GTK, "gtk_widget_get_visible"),
        (libc_names.libpthread, "pthread_mutex_lock"),
        (libc_names.libpthread, "pthread_mutex_unlock"),
    ]
    .iter()
    .map(|(library, symbol)| PlatformImport {
        library: (*library).to_string(),
        symbol: (*symbol).to_string(),
        required_by: "_main".to_string(),
    })
    .collect()
}

/// The window functions, for a program with `uses_window`.
pub(super) fn emit_window_functions() -> Result<Vec<CodeFunction>, String> {
    Ok(vec![emit_window_sync()?, emit_fullscreen_notify()?])
}

/// The worker-side seam `setTitle`/`setFullscreen` append. Emitted into a shared
/// vreg-lowered helper, so it names only argument-role tokens (plan-34-D) and
/// relies on the allocator treating the external call as clobbering everything.
/// `call_idle_add` emits the `g_idle_add` call itself, through the platform's
/// import-resolving external call, so an undeclared import fails the build.
pub(crate) fn emit_window_sync_seam(
    from_symbol: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    call_idle_add: impl FnOnce(
        &mut Vec<CodeInstruction>,
        &mut Vec<CodeRelocation>,
    ) -> Result<(), String>,
) -> Result<(), String> {
    let skip = format!("{from_symbol}_window_sync_skip");
    let mut asm = Asm::new(from_symbol);
    asm.local_address(abi::mfb_arg(0), STATE_SYMBOL);
    asm.push(abi::load_u64(
        abi::mfb_arg(0),
        abi::mfb_arg(0),
        ST_APPLICATION,
    ));
    asm.push(abi::compare_immediate(abi::mfb_arg(0), "0"));
    asm.push(abi::branch_eq(&skip)); // headless: no main loop to run the idle
    asm.local_address(abi::mfb_arg(0), WINDOW_SYNC_SYMBOL); // GSourceFunc
    asm.push(abi::move_immediate(abi::mfb_arg(1), "Integer", "0")); // user-data
    instructions.extend(asm.ins);
    relocations.extend(asm.rel);
    call_idle_add(instructions, relocations)?;
    instructions.push(abi::label(&skip));
    Ok(())
}

/// Connect the just-built `ST_WINDOW`'s `notify::fullscreened` to the tracking
/// handler (main loop, at each window build).
pub(super) fn emit_track_fullscreen(asm: &mut Asm) {
    asm.load_state(abi::c_arg(0), ST_WINDOW);
    asm.local_address(abi::c_arg(1), STR_NOTIFY_FULLSCREENED.0);
    asm.local_address(abi::c_arg(2), FS_NOTIFY_SYMBOL);
    asm.push(abi::move_immediate(abi::c_arg(3), "Integer", "0"));
    asm.push(abi::move_immediate(abi::c_arg(4), "Integer", "0"));
    asm.push(abi::move_immediate(abi::c_arg(5), "Integer", "0"));
    asm.call_external("g_signal_connect_data");
}

/// Call the sync directly (the reconcile's show arms, already on the main loop).
pub(super) fn emit_sync_call(asm: &mut Asm) {
    asm.push(abi::move_immediate(abi::c_arg(0), "Integer", "0"));
    asm.call_internal(WINDOW_SYNC_SYMBOL);
}

/// `gboolean _mfb_gtkapp_window_sync(gpointer)` (main loop).
fn emit_window_sync() -> Result<CodeFunction, String> {
    let mut asm = Asm::new(WINDOW_SYNC_SYMBOL);
    let frame = 16; // lr@0, local0 (window)@8
    let done = format!("{WINDOW_SYNC_SYMBOL}_done");
    let have_title = format!("{WINDOW_SYNC_SYMBOL}_have_title");
    let windowed = format!("{WINDOW_SYNC_SYMBOL}_windowed");
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(frame));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        0,
    ));
    asm.push(abi::store_u64(abi::LOCAL[0], abi::stack_pointer(), 8));
    // No window yet (a `None`-start program before its first `setMode`).
    asm.load_state(abi::LOCAL[0], ST_WINDOW);
    asm.push(abi::compare_immediate(abi::LOCAL[0], "0"));
    asm.push(abi::branch_eq(&done));

    // --- title: gtk_window_set_title copies the string, so the lock covers only
    // the read and the call.
    asm.local_address(abi::c_arg(0), APP_TITLE_LOCK_SYMBOL);
    asm.call_external("pthread_mutex_lock");
    asm.local_address(abi::c_arg(1), APP_TITLE_SYMBOL);
    asm.push(abi::load_u64(abi::c_arg(1), abi::c_arg(1), 0));
    asm.push(abi::compare_immediate(abi::c_arg(1), "0"));
    asm.push(abi::branch_ne(&have_title));
    asm.local_address(abi::c_arg(1), APP_DEFAULT_TITLE_SYMBOL);
    asm.push(abi::label(&have_title));
    asm.push(abi::add_immediate(abi::c_arg(1), abi::c_arg(1), 8)); // the bytes
    asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[0]));
    asm.call_external("gtk_window_set_title");
    asm.local_address(abi::c_arg(0), APP_TITLE_LOCK_SYMBOL);
    asm.call_external("pthread_mutex_unlock");

    // --- fullscreen, only for a shown window (a hidden one is synced again when
    // the reconcile presents it).
    asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[0]));
    asm.call_external("gtk_widget_get_visible");
    asm.push(abi::move_immediate(
        abi::SCRATCH[1],
        "Integer",
        GBOOLEAN_MASK,
    ));
    asm.push(abi::and_registers(
        abi::c_return(0),
        abi::c_return(0),
        abi::SCRATCH[1],
    ));
    asm.push(abi::compare_immediate(abi::c_return(0), "0"));
    asm.push(abi::branch_eq(&done));
    asm.local_address(abi::SCRATCH[1], APP_FULLSCREEN_SYMBOL);
    asm.push(abi::load_u64(abi::SCRATCH[1], abi::SCRATCH[1], 0));
    asm.push(abi::move_register(abi::c_arg(0), abi::LOCAL[0]));
    asm.push(abi::compare_immediate(abi::SCRATCH[1], "0"));
    asm.push(abi::branch_eq(&windowed));
    asm.call_external("gtk_window_fullscreen");
    asm.push(abi::branch(&done));
    asm.push(abi::label(&windowed));
    asm.call_external("gtk_window_unfullscreen");

    asm.push(abi::label(&done));
    asm.push(abi::move_immediate(abi::c_return(0), "Integer", "0")); // G_SOURCE_REMOVE
    asm.push(abi::load_u64(abi::LOCAL[0], abi::stack_pointer(), 8));
    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::add_stack(frame));
    asm.push(abi::return_());
    asm.finish(WINDOW_SYNC_SYMBOL, "Integer")
}

/// `void _mfb_gtkapp_fullscreen_notify(GObject *window, GParamSpec *, gpointer)`:
/// store `gtk_window_is_fullscreen(window) ? 1 : 0` into the fullscreen word.
fn emit_fullscreen_notify() -> Result<CodeFunction, String> {
    let mut asm = Asm::new(FS_NOTIFY_SYMBOL);
    let frame = 16; // lr@0
    let store = format!("{FS_NOTIFY_SYMBOL}_store");
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(frame));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        0,
    ));
    asm.call_external("gtk_window_is_fullscreen"); // window is already arg 0
    asm.push(abi::move_immediate(
        abi::SCRATCH[1],
        "Integer",
        GBOOLEAN_MASK,
    ));
    asm.push(abi::and_registers(
        abi::c_return(0),
        abi::c_return(0),
        abi::SCRATCH[1],
    ));
    asm.push(abi::compare_immediate(abi::c_return(0), "0"));
    asm.push(abi::branch_eq(&store));
    asm.push(abi::move_immediate(abi::c_return(0), "Integer", "1"));
    asm.push(abi::label(&store));
    asm.local_address(abi::SCRATCH[1], APP_FULLSCREEN_SYMBOL);
    asm.push(abi::store_u64(abi::c_return(0), abi::SCRATCH[1], 0));
    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::add_stack(frame));
    asm.push(abi::return_());
    asm.finish(FS_NOTIFY_SYMBOL, "Nothing")
}
