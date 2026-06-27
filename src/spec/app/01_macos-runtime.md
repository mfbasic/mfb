# macOS App Runtime (AppKit)

The macOS `mfb build -app` runtime: the AppKit `_main` bootstrap, the worker
pthread that runs the MFBASIC program, and the per-process state scheme. The
bootstrap and its helpers are emitted as hand-written aarch64 by
`emit_app_program_entry`, which returns the bootstrap (`_main`), the worker shim,
and the transcript/finish/delegate/input/term helper functions. The standard
program entry runs separately on the worker under `_mfb_macapp_program`
(`MACAPP_PROGRAM_SYMBOL`). [[src/target/macos_aarch64/app.rs:emit_app_program_entry]]

All Objective-C interaction goes through the public runtime
(`objc_msgSend`/`sel_registerName`/`objc_allocateClassPair`/...); classes are
obtained by loading the `_OBJC_CLASS_$_*` data symbols through the GOT, which also
force-loads AppKit/Foundation. No private API is used.
[[src/target/macos_aarch64/app.rs:emit_main_bootstrap]]

## `_main` bootstrap

`_main` runs on the process main thread (AppKit's required home). It reserves a
32-byte frame and stashes `argc`/`argv` (passed in `x0`/`x1` by the kernel)
before any external call clobbers them. Persistent objects are held in
callee-saved registers across the `objc_msgSend` sequence.

| Register | Holds |
|----------|-------|
| `x19` (`REG_APP`) | `NSApplication` instance |
| `x20` (`REG_WINDOW`) | `NSWindow` instance |
| `x21` (`REG_SCRATCH_OBJ`) | transient object (class / NSString / transcript view) |
| `x22` (`REG_HEADLESS`) | `getenv("MFB_MACAPP_HEADLESS")` result |

```text
_main stack frame (FRAME_SIZE = 32):
  [sp+0]   OFF_ARGC   i64 argc        }
  [sp+8]   OFF_ARGV   char **argv     } worker arg block {argc, argv}
  [sp+16]  OFF_TID    pthread_t       (headless path only)
  [sp+24]  OFF_PIPE   int fds[2]      input pipe (read @+24, write @+28)
```

[[src/target/macos_aarch64/app.rs:emit_main_bootstrap]]

The unconditional bootstrap prefix builds the application and window:

- `app = [NSApplication sharedApplication]` (referencing `_OBJC_CLASS_$_NSApplication` binds AppKit).
- `[app setActivationPolicy:0]` — `NSApplicationActivationPolicyRegular` (`ACTIVATION_POLICY_REGULAR`).
- `window = [[NSWindow alloc] initWithContentRect:styleMask:backing:defer:]` with
  `contentRect = NSMakeRect(100, 100, 900, 640)` passed as an HFA of four doubles
  in `d0..d3`, `styleMask = 15` (`WINDOW_STYLE_MASK` = Titled|Closable|Miniaturizable|Resizable),
  `backing = 2` (`BACKING_BUFFERED`, NSBackingStoreBuffered), `defer = NO`.
- `[window setTitle:[NSString stringWithUTF8String:"MFBASIC App"]]` (`STR_TITLE`).
- `headless = getenv("MFB_MACAPP_HEADLESS")` (`STR_HEADLESS_ENV`); the result gates
  all GUI construction below.

[[src/target/macos_aarch64/app.rs:emit_main_bootstrap]]

### GUI construction (skipped when headless != 0)

A `compare REG_HEADLESS, 0` / `branch_ne after_show` guards the GUI section. When
the env var is set the bootstrap branches to `after_show`, building no view and
showing no window — the io helpers then find no associated NSTextView and fall
back to the fd sink (see [Headless path](#headless-path-mfb_macapp_headless)).
[[src/target/macos_aarch64/app.rs:emit_main_bootstrap]]

The GUI path constructs, in order:

1. **Transcript view.** `content = [window contentView]`; an `NSScrollView`
   (`initWithFrame:NSMakeRect(0,0,900,640)`, `setAutoresizingMask:18` =
   `AUTORESIZE_WIDTH_HEIGHT`, NSViewWidthSizable|NSViewHeightSizable). The document
   view is an **`MFBTextView : NSTextView`** synthesized at runtime via
   `objc_allocateClassPair`/`class_addMethod(keyDown:, ..., "v@:@")`/
   `objc_registerClassPair`, so the transcript view itself receives typed keys.
   The text view is given `setAutoresizingMask:2` (`AUTORESIZE_WIDTH`),
   `setFont:[NSFont userFixedPitchFontOfSize:13]` (`TRANSCRIPT_FONT_SIZE`),
   `setEditable:NO`, `setSelectable:YES`, then installed as the scroll view's
   document view with `setHasVerticalScroller:YES` and added to the content view.
   The text view is stashed on NSApp under `ASSOC_KEY`. [[src/target/macos_aarch64/app.rs:emit_main_bootstrap]]
2. **TermView surface.** The window and scroll view are stashed under
   `WINDOW_ASSOC_KEY`/`SCROLLVIEW_ASSOC_KEY`. A **`TermView : NSView`** class is
   synthesized with methods `drawRect:` (`"v@:{CGRect=dddd}"`), `isFlipped`
   (`"c@:"`), `mfbWriteString:` (`"v@:@"`), `acceptsFirstResponder` (`"c@:"`), and
   `keyDown:` (`"v@:@"`). A `TermView` instance is allocated at
   `NSMakeRect(0,0,900,640)` (`TERM_VIEW_WIDTH`/`TERM_VIEW_HEIGHT`),
   auto-sized to fill, initialized by the internal `_mfb_macapp_term_init`, and
   stashed under `TERMVIEW_ASSOC_KEY`. See `./mfb spec app term-backend`.
   [[src/target/macos_aarch64/app.rs:emit_main_bootstrap]]
3. **App delegate.** An **`MFBAppDelegate : NSObject`** (`STR_DELEGATE_CLASS`) is
   synthesized with two methods and set as `[app setDelegate:]`:
   - `applicationShouldTerminateAfterLastWindowClosed:` (type `"c@:@"`,
     `STR_DELEGATE_TYPES`), IMP `_mfb_macapp_should_terminate` — returns YES so
     closing the window quits the app.
   - `applicationDidFinishLaunching:` (type `"v@:@"`, `STR_INPUT_TYPES`), IMP
     `_mfb_macapp_did_finish_launching` — spawns the worker thread (below).

   [[src/target/macos_aarch64/app.rs:emit_main_bootstrap]] [[src/target/macos_aarch64/app.rs:emit_should_terminate_helper]]
4. **Input plumbing.** An `NSMutableString` line buffer (`[NSMutableString string]`)
   is stashed under `INPUT_LINE_KEY` with `OBJC_ASSOCIATION_RETAIN_NONATOMIC` (3rd
   arg `1`). Then `pipe(fds)`; `dup2(fds[0], 0)` redirects the program's stdin
   reads onto the window input pipe; the write end `fds[1]` is stashed under
   `PIPE_ASSOC_KEY`. See `./mfb spec app console-io`. [[src/target/macos_aarch64/app.rs:emit_main_bootstrap]]
5. **Application menu.** `mainMenu` (NSMenu) → `appMenuItem` (NSMenuItem) →
   `appMenu` (NSMenu) → a **Quit** item titled `"Quit"` (`STR_QUIT`) with
   `setAction:@selector(terminate:)` (the standard `[NSApp terminate:]`) and
   `setKeyEquivalent:@"q"` (`STR_QUIT_KEY`, Cmd-Q). `[app setMainMenu:mainMenu]`.
   [[src/target/macos_aarch64/app.rs:emit_main_bootstrap]]
6. **Show & activate.** `[window makeKeyAndOrderFront:nil]`,
   `[app activateIgnoringOtherApps:YES]`, `[window makeFirstResponder:textview]`
   so keypresses reach the transcript. The `after_show` label follows.
   [[src/target/macos_aarch64/app.rs:emit_main_bootstrap]]

### Worker spawn and event loop

After `after_show`, the bootstrap re-tests `REG_HEADLESS`:

- **GUI** (`branch_eq gui_defer_worker`): the worker is **not** spawned inline —
  touching AppKit / the Obj-C runtime from the worker during
  `-[NSApplication finishLaunching]` corrupts the runtime and aborts. Instead the
  `&argblock` stack pointer (`sp+OFF_ARGC`) is stashed under `ARG_ASSOC_KEY`, and
  `applicationDidFinishLaunching:` later spawns the worker. The bootstrap then runs
  the AppKit event loop with `[NSApp run]` (`SEL_RUN`), which does not return under
  normal operation; if it ever does, `_exit(0)`. The `&argblock` pointer stays
  valid because `_main` blocks forever in `[NSApp run]`. [[src/target/macos_aarch64/app.rs:emit_main_bootstrap]]
- **Headless**: there is no run loop or delegate callback, so the worker is spawned
  inline — `pthread_create(&tid, NULL, _mfb_macapp_worker, &argblock)` — and `_main`
  spins (`branch_self`) while the worker runs the program and exits the process.
  [[src/target/macos_aarch64/app.rs:emit_main_bootstrap]]

`applicationDidFinishLaunching:` (`_mfb_macapp_did_finish_launching`, main thread)
re-fetches `[NSApplication sharedApplication]`, reads the `&argblock` pointer back
out of `ARG_ASSOC_KEY`, and `pthread_create`s the worker (the `pthread_t` is
discarded; the worker is never joined). [[src/target/macos_aarch64/app.rs:emit_did_finish_launching_helper]]

## Worker pthread shim

`void *_mfb_macapp_worker(void *arg)` is the pthread start routine. It pushes an
autorelease pool (`objc_autoreleasePoolPush`, preserving the arg pointer across the
call on a 16-byte temp frame), then, when the language entry accepts args
(`AppEntrySpec.language_entry_accepts_args`), unpacks the `{argc, argv}` block:
`argv` from `[arg+OFF_ARGV]` into `x1`, `argc` from `[arg+OFF_ARGC]` into `x0`. It
then tail-calls the standard program entry `_mfb_macapp_program`
(`MACAPP_PROGRAM_SYMBOL`), which never returns. [[src/target/macos_aarch64/app.rs:emit_worker_shim]]

The autorelease pool is mandatory: the worker creates autoreleased Cocoa objects
(NSString/NSFont/...), and on the GUI keep-open path the thread parks rather than
exits — but were it to exit, the thread-exit autorelease-pool cleanup would crash
draining improperly-pooled objects. [[src/target/macos_aarch64/app.rs:emit_worker_shim]]

## Program-finish path (window stays open)

macOS `emit_program_exit` diverges from the console `_exit`: when the emitting
function is `_mfb_macapp_program` (the worker's program entry), the program's exit
is routed to `bl _mfb_macapp_program_finish` (`FINISH_SYMBOL`) instead of
`_exit`, so the window can stay open. Any other function (console programs, plus
the headless fallback inside the finish helper itself) still terminates via
`_exit`. [[src/target/macos_aarch64/code.rs:emit_program_exit]]

`void _mfb_macapp_program_finish(int code /*x0*/)` runs on the worker thread:

- When the program uses `term::` (`AppEntrySpec.uses_term`), it first calls
  `_mfb_rt_term_term_off` to auto-restore the transcript if TUI mode is still
  active (it gates on the active flag, so it is a safe no-op otherwise);
  the exit code is preserved across the call via `sp+40`. `x19` is still the
  pinned arena-state base that `term_off` reads. [[src/target/macos_aarch64/app.rs:emit_finish_helper]]
- It fetches the transcript view via `objc_getAssociatedObject(NSApp, &ASSOC_KEY)`.
  **nil** (headless / no window) → `headless_exit`: `_exit(code)`, preserving the
  console-like behavior the runtime tests rely on. [[src/target/macos_aarch64/app.rs:emit_finish_helper]]
- **GUI**: it appends `"\nProgram exited with code "` (`STR_EXIT_PREFIX`), then the
  decimal exit code formatted in-register by `emit_format_exit_code` (0..255 →
  ASCII into the stack buffer at `sp+40`, leading zeros suppressed, count in `x22`)
  rendered via `[[NSString alloc] initWithBytes:length:encoding:NSUTF8StringEncoding]`,
  then `"\n"`. Each append goes through `_mfb_macapp_append`. The worker then
  **parks** in `pause()` forever (`park` loop): it must **not** `pthread_exit`,
  because the worker has made Cocoa calls and the thread-exit autorelease-pool
  cleanup would SIGSEGV draining them. The main thread's event loop keeps the
  window open until the user closes it, at which point the delegate terminates the
  whole process. [[src/target/macos_aarch64/app.rs:emit_finish_helper]] [[src/target/macos_aarch64/app.rs:emit_format_exit_code]]

## Transcript append helper

`void _mfb_macapp_append(id textView /*x0*/, id nsString /*x1*/)` styles and
appends text on the main thread. It builds
`[[NSAttributedString alloc] initWithString:nsString attributes:@{NSFontAttributeName: [NSFont userFixedPitchFontOfSize:13]}]`
— `NSFontAttributeName` is read as external data (an AppKit `NSString * const`
global) and dereferenced once for the key. The run is appended to
`[textView textStorage]` via
`performSelectorOnMainThread:@selector(appendAttributedString:) withObject:attr waitUntilDone:YES`.
Appending an explicitly-attributed run is required: a plain `appendString:`
ignores the view's font and renders in the default proportional system font.
`waitUntilDone:YES` makes the write synchronous (so `io::flush` is a no-op).
[[src/target/macos_aarch64/app.rs:emit_append_helper]]

## Per-process state: associated objects on NSApp

App mode keeps no writable data segment for runtime state; instead, per-process
state is stored as Objective-C **associated objects** on the shared
`NSApplication`, keyed by the **unique address of a 1-byte read-only data symbol**.
Each key symbol is emitted by `app_mode_data_objects` as a `raw` 1-byte object
(`align 1`, value `00`, layout "associated-object key (unique address)"); only the
symbol's address is meaningful. Worker-thread helpers reach the stashed objects by
fetching `[NSApplication sharedApplication]` and calling
`objc_getAssociatedObject(app, &KEY)`. [[src/target/macos_aarch64/app.rs:app_mode_data_objects]]

| Key symbol | Stored object | Association |
|------------|---------------|-------------|
| `ASSOC_KEY` (`_mfb_macapp_textview_key`) | transcript NSTextView (nil ⇒ headless ⇒ fd sink) | ASSIGN |
| `WINDOW_ASSOC_KEY` (`_mfb_macapp_window_key`) | NSWindow | ASSIGN |
| `SCROLLVIEW_ASSOC_KEY` (`_mfb_macapp_scrollview_key`) | transcript NSScrollView | ASSIGN |
| `TERMVIEW_ASSOC_KEY` (`_mfb_macapp_termview_key`) | TermView grid surface | ASSIGN |
| `TVSTATE_ASSOC_KEY` (`_mfb_macapp_termstate_key`) | TermView `calloc`'d grid-state struct (plain C buffer) | ASSIGN |
| `PIPE_ASSOC_KEY` (`_mfb_macapp_pipe_key`) | input pipe write fd | ASSIGN |
| `INPUT_LINE_KEY` (`_mfb_macapp_inputline_key`) | input-line NSMutableString | RETAIN_NONATOMIC |
| `INPUT_MODE_KEY` (`_mfb_macapp_inputmode_key`) | app input mode (1=line/echo, 2=raw/no-echo) | (set by io helpers) |
| `ARG_ASSOC_KEY` (`_mfb_macapp_argblock_key`) | `&{argc, argv}` block pointer | ASSIGN |

[[src/target/macos_aarch64/app.rs:app_mode_data_objects]]

The same scheme stores the AppKit data the SEL/string constants reference: all
selector and C-string constants are emitted as NUL-terminated `raw` C strings
(`align 1`) by `app_mode_data_objects`; selectors are interned at runtime through
`sel_registerName`. [[src/target/macos_aarch64/app.rs:app_mode_data_objects]]

## Headless path (`MFB_MACAPP_HEADLESS`)

When the `MFB_MACAPP_HEADLESS` env var (`STR_HEADLESS_ENV`) is set, the bootstrap
skips the entire GUI construction (transcript view, TermView, delegate, input pipe,
menu, show/activate) and the AppKit event loop, spawning the worker inline and
spinning `_main`. With no transcript view associated under `ASSOC_KEY`, the io
helpers fall back to the file-descriptor sink and the finish helper takes the
`headless_exit` → `_exit(code)` branch. This drives the automated runtime tests
through the same construction-free worker and program-exit code the GUI path uses,
while preserving console-like exit semantics. [[src/target/macos_aarch64/app.rs:emit_main_bootstrap]] [[src/target/macos_aarch64/app.rs:emit_finish_helper]]

## See Also

* ./mfb spec memory program-startup — the console-mode entry/teardown sequence (where program exit is `_exit`)
* ./mfb spec app console-io — `io::` redirected over the window (input pipe dup2'd onto fd 0, line vs raw key handling)
* ./mfb spec app term-backend — the GUI `term::` TermView grid/cell model and content-view swap
* ./mfb spec app linux-runtime — the GTK4 counterpart bootstrap and state global
* ./mfb spec threading os-integration — the worker pthread the window drives
* ./mfb spec linker static-and-dynamic-output — app-mode entry-bootstrap import differences
