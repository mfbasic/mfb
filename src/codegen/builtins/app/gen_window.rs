//! Shared state and lowering pieces for the `app` window members —
//! `setTitle`/`getTitle`/`setFullscreen`/`getFullscreen`.
//!
//! The window is one per process, so its title and fullscreen state are
//! process-global, not per-arena like the presentation-mode slot: a `thread::`
//! worker that sets the title must be seen by the main program's `getTitle`, and
//! every backend's UI thread (which has no arena state at all) must read the same
//! values when it builds, shows or syncs the window. So they live in four
//! writable data objects, emitted only for a program that references one of the
//! four members:
//!
//! * [`APP_FULLSCREEN_SYMBOL`] — one word, `0`/`1`. Written by `setFullscreen` on
//!   a program thread and by a backend's UI thread when the *user* changes the
//!   window's fullscreen state (the macOS green button, a window-manager
//!   shortcut), so `getFullscreen` reports the window rather than the last
//!   request. One plain aligned word: a reader sees either the old or the new
//!   value, and either is an answer the window held a moment ago.
//! * [`APP_TITLE_SYMBOL`] — a pointer to a heap block laid out exactly like an
//!   MFBASIC `String` (`[u64 length][bytes][NUL]`), or `0` before the first
//!   `setTitle`, meaning "the default title" ([`APP_DEFAULT_TITLE_SYMBOL`]).
//!   Same layout so `getTitle` copies it with one loop and a backend hands
//!   `+8` to its toolkit as a C string.
//! * [`APP_TITLE_LOCK_SYMBOL`] — the mutex guarding that pointer. `setTitle`
//!   swaps the pointer under it and frees the old block after; `getTitle` and
//!   each backend's UI-thread sync read the block under it. Nothing ever waits
//!   on another thread while holding it (the worker-to-UI marshal happens after
//!   the unlock), so the UI thread taking it cannot deadlock against a worker
//!   blocked in `performSelectorOnMainThread:…waitUntilDone:YES` / `SendMessageW`.
//! * [`APP_DEFAULT_TITLE_SYMBOL`] — the title the window is built with, in the
//!   same `String` layout. It comes from the backend
//!   ([`CodegenPlatform::app_default_window_title`]) so `getTitle` returns
//!   exactly what the window shows before any `setTitle`.

use crate::codegen::engine::analysis::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::*;
use crate::codegen::memory::data::*;
use crate::target::shared::abi;
use crate::target::shared::nir::NirModule;

/// The four window members. Any one of them emits the whole window-state set:
/// the backend sync helper `setFullscreen` triggers reads the title too.
pub(crate) const APP_WINDOW_CALLS: &[&str] = &[
    "app.setTitle",
    "app.getTitle",
    "app.setFullscreen",
    "app.getFullscreen",
];

/// The fullscreen word: `0` windowed, `1` fullscreen. See the module doc.
pub(crate) const APP_FULLSCREEN_SYMBOL: &str = "_mfb_rt_app_fullscreen";
/// The title pointer: a heap `String`-layout block, or `0` for the default.
pub(crate) const APP_TITLE_SYMBOL: &str = "_mfb_rt_app_title";
/// The mutex guarding [`APP_TITLE_SYMBOL`].
pub(crate) const APP_TITLE_LOCK_SYMBOL: &str = "_mfb_rt_app_title_lock";
/// The default title, as a `String`-layout block.
pub(crate) const APP_DEFAULT_TITLE_SYMBOL: &str = "_mfb_rt_app_default_title";

/// Whether `module` references any window member, so the window state (and each
/// backend's window-sync helper) is emitted. A program that never touches the
/// window keeps its exact data-object, import and function set.
pub(crate) fn module_uses_app_window(module: &NirModule) -> bool {
    module_uses_any_call(module, APP_WINDOW_CALLS)
}

/// The four window-state data objects for a program that uses a window member.
/// The lock's bytes are the platform's static mutex initializer (the same one
/// the `os::` env lock uses), so no runtime initializer runs.
pub(crate) fn app_window_data_objects(
    family: PlatformFamily,
    default_title: &str,
) -> Vec<CodeDataObject> {
    let title = default_title.as_bytes();
    let mut block = Vec::with_capacity(title.len() + 9);
    block.extend_from_slice(&(title.len() as u64).to_le_bytes());
    block.extend_from_slice(title);
    block.push(0);
    vec![
        CodeDataObject {
            symbol: APP_FULLSCREEN_SYMBOL.to_string(),
            kind: "raw".to_string(),
            layout: "mfb.runtime.app_fullscreen.v1 { u64 fullscreen }".to_string(),
            align: 8,
            size: 8,
            value: "00".repeat(8),
        },
        CodeDataObject {
            symbol: APP_TITLE_SYMBOL.to_string(),
            kind: "raw".to_string(),
            layout: "mfb.runtime.app_title.v1 { u64 string_block_or_zero }".to_string(),
            align: 8,
            size: 8,
            value: "00".repeat(8),
        },
        CodeDataObject {
            symbol: APP_TITLE_LOCK_SYMBOL.to_string(),
            kind: "raw".to_string(),
            layout: "mfb.runtime.app_title_lock.v1 { u8 mutex[64] }".to_string(),
            align: 8,
            size: crate::codegen::builtins::os::OS_ENV_LOCK_SIZE,
            value: crate::codegen::builtins::os::os_env_lock_init_hex(family),
        },
        CodeDataObject {
            symbol: APP_DEFAULT_TITLE_SYMBOL.to_string(),
            kind: "raw".to_string(),
            layout: "mfb.runtime.app_default_title.v1 { u64 length; u8 bytes[length]; u8 nul }"
                .to_string(),
            align: 8,
            size: block.len(),
            value: block.iter().map(|byte| format!("{byte:02x}")).collect(),
        },
    ]
}

/// The title-lock acquire/release functions: the pthread mutex on POSIX, an
/// SRWLOCK on Windows (whose all-zero `SRWLOCK_INIT` is what
/// `os_env_lock_init_hex` writes there).
pub(crate) fn title_lock_fns(family: PlatformFamily) -> (&'static str, &'static str) {
    match family {
        PlatformFamily::Windows => ("AcquireSRWLockExclusive", "ReleaseSRWLockExclusive"),
        _ => ("pthread_mutex_lock", "pthread_mutex_unlock"),
    }
}

/// `lock(&_mfb_rt_app_title_lock)` (`unlock` when `acquire` is false). Clobbers
/// every caller-saved register; live values must be in vregs.
pub(crate) fn emit_title_lock(ctx: &mut EmitCtx, acquire: bool) -> Result<(), String> {
    push_symbol_address(
        ctx.symbol,
        APP_TITLE_LOCK_SYMBOL,
        abi::c_arg(0),
        ctx.instructions,
        ctx.relocations,
    );
    let (lock_fn, unlock_fn) = title_lock_fns(ctx.platform.family());
    ctx.platform.emit_external_call(
        if acquire { lock_fn } else { unlock_fn },
        ctx.symbol,
        ctx.platform_imports,
        ctx.instructions,
        ctx.relocations,
    )
}
