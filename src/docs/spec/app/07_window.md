# Window Title and Fullscreen

The `app` window members — `app::setTitle(String)`, `app::getTitle() AS String`,
`app::setFullscreen(Boolean)`, `app::getFullscreen() AS Boolean` — make an `--app`
program's window title and fullscreen state program-controlled. This topic
specifies the state model, the worker-to-UI-thread sync, and each backend's
mechanics. The per-function API is `./mfb man app setTitle` (and siblings).

## Process-global state

The window is one per process, so its state is process-global rather than
per-arena like the presentation-mode word (`./mfb spec app presentation-mode`):
a `thread::` worker's `setTitle` is seen by the main program's `getTitle`, and a
backend's UI thread — which has no arena state — reads the same values. Four
writable data objects hold it, emitted only when the program references one of
the four members, so every other app program keeps its exact data-object set.
[[src/codegen/builtins/app/gen_window.rs:app_window_data_objects]]
[[src/codegen/builtins/app/gen_window.rs:module_uses_app_window]]

| Symbol | Layout | Meaning |
|---|---|---|
| `_mfb_rt_app_fullscreen` | `u64` | `0` windowed, `1` fullscreen |
| `_mfb_rt_app_title` | `u64` pointer | a process-heap block in `String` layout, or `0` = default |
| `_mfb_rt_app_title_lock` | `u8[64]` | the mutex guarding the title pointer |
| `_mfb_rt_app_default_title` | `String` layout | the title the window is built with |

[[src/codegen/builtins/app/gen_window.rs:APP_FULLSCREEN_SYMBOL]]
[[src/codegen/builtins/app/gen_window.rs:APP_TITLE_SYMBOL]]
[[src/codegen/builtins/app/gen_window.rs:APP_TITLE_LOCK_SYMBOL]]
[[src/codegen/builtins/app/gen_window.rs:APP_DEFAULT_TITLE_SYMBOL]]

The title blocks use the standalone `String` layout (`[u64 length][bytes][NUL]`,
`./mfb spec memory heap-values`), so `getTitle` copies one block into the arena and
a backend passes `block + 8` to its toolkit as a C string. The lock's static bytes
are the platform mutex initializer the `os::` env lock uses — the
`_PTHREAD_MUTEX_SIG_init` signature on macOS, all-zero on Linux (a
`pthread_mutex_t`) and on Windows (an `SRWLOCK`) — so no initializer runs.
[[src/codegen/builtins/os/gen_env.rs:os_env_lock_init_hex]]
[[src/codegen/builtins/app/gen_window.rs:title_lock_fns]]

The default title is the backend's own window title, so `getTitle` before any
`setTitle` returns exactly what the title bar shows: `MFBASIC App` on macOS, the
project name on Linux, and the project name on Windows (`MFBASIC App` when the
name is empty). [[src/codegen/engine/types/types.rs:app_default_window_title]]
[[src/target/macos_aarch64/app/mod.rs:DEFAULT_WINDOW_TITLE]]
[[src/target/win_x86_64/app/mod.rs:default_window_title]]

## The members

- `getFullscreen` is a plain load of the fullscreen word — no marshal.
  [[src/codegen/builtins/app/func_get_fullscreen.rs:lower_get_fullscreen]]
- `setFullscreen` stores the word, then runs the window sync.
  [[src/codegen/builtins/app/func_set_fullscreen.rs:lower_set_fullscreen]]
- `setTitle` copies the argument block (`length + 9` bytes) into a fresh
  process-heap block — the argument lives in the caller's per-thread arena, but the
  title outlives the call and is read on other threads — swaps it into the title
  pointer under the lock, frees the replaced block after the unlock, then runs the
  window sync. A failed heap allocation raises `ErrOutOfMemory` and leaves the title
  unchanged. [[src/codegen/builtins/app/func_set_title.rs:lower_set_title]]
- `getTitle` holds the lock across choosing the block (the pointer, or the default
  when it is `0`) and copying it into a fresh arena `String`, because a concurrent
  `setTitle` frees the block it replaces. Every exit, including arena
  out-of-memory, passes the one unlock.
  [[src/codegen/builtins/app/func_get_title.rs:lower_get_title]]

**Lock discipline.** No holder of the title lock ever waits on another thread: the
worker marshals to the UI thread only after it has unlocked. That is what lets
each backend's UI thread take the lock while a worker is blocked in a synchronous
marshal without deadlocking.

## The window sync

The window sync is the `CodegenPlatform::emit_app_window_sync` seam, appended to
the `setTitle`/`setFullscreen` helpers after the state update. It asks the UI
thread to make the window match the state: the title from the title data, and —
while the window is **visible** — the fullscreen state from the word. A hidden
window (`Mode.None`) is not fullscreened; instead, each backend's mode reconcile
calls the same UI-thread sync right after it shows the window. That is how a title
or fullscreen request made while windowless lands when the window appears, and how
the state survives a `Console`/`Canvas` switch. Headless there is no window and no
event loop, so the seam skips the marshal and the members are state-only.
[[src/codegen/engine/types/types.rs:emit_app_window_sync]]

Each backend's fullscreen tracking writes the fullscreen word from the UI thread
when the **user** changes the window's state, so `getFullscreen` reports the
window rather than the last request.

The backend helpers are emitted only for a program with `AppEntrySpec::uses_window`,
because each names the process-global data above.
[[src/codegen/engine/types/types.rs:AppEntrySpec]]

### macOS

- Worker: `_mfb_macapp_window_sync_marshal` sends `mfbSyncWindow:` to the app
  delegate with `performSelectorOnMainThread:withObject:nil waitUntilDone:YES`. It
  reads the delegate from the `_mfb_macapp_delegate` global rather than
  `[NSApp delegate]`, so any program thread may call it; the global is nil headless
  and the marshal is skipped.
- Main thread: `_mfb_macapp_window_sync` finds the window under the
  `WINDOW_ASSOC_KEY` associated object (none yet → return), builds the title with
  `alloc`/`initWithUTF8String:` under the lock, `setTitle:`s and `release`s it (a
  nil string — bytes that are not UTF-8 — leaves the title unchanged rather than
  raising in `setTitle:nil`). If `isVisible` (low byte of the `BOOL`), it compares
  `styleMask` bit 14 (`NSWindowStyleMaskFullScreen`) with the word and sends
  `toggleFullScreen:` when they differ. The transition is animated; the call
  returns once it has started.
- Tracking: the delegate observes `NSWindowDidEnterFullScreenNotification` and
  `NSWindowDidExitFullScreenNotification` (`object:nil`) with two IMPs that store
  `1`/`0`.

[[src/target/macos_aarch64/app/window.rs:emit_window_sync]]
[[src/target/macos_aarch64/app/window.rs:emit_window_observe]]

### Linux (GTK4)

- Worker: `g_idle_add(_mfb_gtkapp_window_sync, NULL)` — fire-and-forget, like the
  mode reconcile, and ordered after an earlier `setMode`'s reconcile idle. Skipped
  when `ST_APPLICATION` is `0` (headless never creates the application), because an
  idle nothing runs would be a leak per call.
- Main loop: `_mfb_gtkapp_window_sync` returns if `ST_WINDOW` is `0`, else sets
  the title with `gtk_window_set_title` under the lock (GTK copies it) and, if
  `gtk_widget_get_visible`, calls `gtk_window_fullscreen` or
  `gtk_window_unfullscreen` (both idempotent window-manager requests). Returns
  `G_SOURCE_REMOVE`. A `gboolean` return is masked to its low 32 bits.
- Tracking: each built window connects `notify::fullscreened` to a handler that
  stores `gtk_window_is_fullscreen(window) != 0`.

[[src/target/linux_gtk/window.rs:emit_window_sync]]
[[src/target/linux_gtk/window.rs:emit_track_fullscreen]]

### Windows

- Worker: `SendMessageW(main, WM_APP + 3, 0, 0)` — synchronous; skipped when the
  main HWND global is `0` (headless).
- UI thread: the `WndProc` arm calls `_mfb_winapp_window_sync(hwnd)`. The title is
  widened with `MultiByteToWideChar(CP_UTF8, …)` — a counting call, a
  `HeapAlloc`'d buffer, the converting call, all under the SRW lock — then set with
  `SetWindowTextW` and freed. Win32 has no fullscreen window state, so fullscreen is
  **borderless**: entering saves the style (`GWL_STYLE`) and `WINDOWPLACEMENT`,
  clears `WS_OVERLAPPEDWINDOW`, and covers the window's monitor
  (`MonitorFromWindow`/`GetMonitorInfoW`, `SetWindowPos` with
  `SWP_NOOWNERZORDER | SWP_FRAMECHANGED`); leaving restores the saved style and
  placement. `_mfb_winapp_fs_applied` records which state the window is in, so a
  sync that already matches does nothing and a fullscreen placement is never saved
  over the windowed one. Only a visible window (`IsWindowVisible`) is changed.
- Tracking: none is needed — nothing but the program changes a borderless window's
  state.

[[src/target/win_x86_64/app/window.rs:emit_window_sync]]
[[src/target/win_x86_64/app/window.rs:WM_APP_SYNC_WINDOW]]

## See Also

* ./mfb spec app presentation-mode — the mode word and the reconcile that shows the window
* ./mfb spec app macos-runtime — the AppKit bootstrap and delegate
* ./mfb spec app linux-runtime — the GTK4 bootstrap and `_mfb_gtkapp_state`
* ./mfb man app — the member API
