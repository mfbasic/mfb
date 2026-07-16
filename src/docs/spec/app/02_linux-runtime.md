# Linux App Runtime (GTK4)

The Linux counterpart of the macOS AppKit app runtime: when `mfb build -app`
targets `linux-aarch64`, the backend emits a GTK4 `_main` bootstrap, a language
worker thread, the transcript/input widgets, a `GtkDrawingArea`+Cairo `term::`
surface, and the app-mode `io::*`/`term::*` helper bodies. Every GTK / GObject /
GLib / GIO / Cairo call is an ordinary imported C function reached by `bl
<symbol>` against the imports declared in `app_mode_imports` — there is no
`objc_msgSend`-style message layer. The container itself (glibc-only ELF, library
names) is the linker's concern (`./mfb spec linker static-and-dynamic-output`).
[[src/target/linux_gtk/mod.rs:emit_app_program_entry]]
[[src/target/linux_aarch64/plan.rs:app_mode_imports]]

> The Linux app backend mirrors the macOS app structure and is code-plan-valid
> and ELF-encodable. Several runtime-bound behaviors are simplified relative to
> the macOS runtime; the divergences documented below are the observable contract
> of this backend, and callers must not assume parity with the macOS runtime.
> [[src/target/linux_gtk/mod.rs:emit_app_program_entry]]

## Emitted functions

`emit_app_program_entry` returns the bootstrap/UI/worker/term set. The standard
program entry runs separately on the worker thread under
`code::MACAPP_PROGRAM_SYMBOL`.
[[src/target/linux_gtk/mod.rs:emit_app_program_entry]]

| Symbol | Role |
| --- | --- |
| `_main` | ELF `e_entry`; trampoline into `__libc_start_main` |
| `_mfb_gtkapp_main` | real C `main(argc,argv,envp)`; builds + runs the GTK app |
| `_mfb_gtkapp_activate` | `activate` handler; builds window, pipe, worker |
| `_mfb_gtkapp_worker` | pthread start routine running the language program |
| `_mfb_gtkapp_key_pressed` | window `key-pressed` handler (terminal-style input) |
| `_mfb_gtkapp_window_closed` | `close-request`; quits the GApplication |
| `_mfb_gtkapp_finish` | program-completion handler (`FINISH_SYMBOL`) |
| `_mfb_gtkapp_append` | main-thread `GtkTextBuffer` insert + auto-scroll |
| `_mfb_gtkapp_append_idle` | `g_idle_add` callback draining a marshaled chunk |
| `_mfb_gtkapp_term_*` | term:: grid draw / write / scroll / init / idle swaps |

[[src/target/linux_gtk/mod.rs:FINISH_SYMBOL]]
[[src/target/linux_gtk/mod.rs:MAIN_SYMBOL]]

## `_main` → `__libc_start_main` bootstrap

`_main` is the ELF entry point, reached with the stack exactly as the kernel/loader
left it (`sp → argc, argv, NULL, envp...`). The built-in linker pulls in no host
objects (no `crt1.o`), so `_main` hands off to `__libc_start_main`, passing
`_mfb_gtkapp_main` as `main`, `[sp]` as `argc`, `sp+8` as `argv`, and zeros for
`init`/`fini`/`rtld_fini`. `__libc_start_main` runs the C runtime init — including
every loaded shared library's `DT_INIT_ARRAY` constructors, which boot the
GLib/GObject type system GTK requires — then calls `main`. It never returns
(`branch_self` guards the tail). [[src/target/linux_gtk/bootstrap.rs:emit_libc_start_trampoline]]

`_mfb_gtkapp_main` then:

1. `setenv("GTK_A11Y","none",1)` and `setenv("GTK_IM_MODULE","none",1)` — disables
   the a11y + input-method layers before GTK initializes; their
   `g_variant_new_string` path crashes when the worker inserts transcript text.
2. `app = gtk_application_new("dev.mfbasic.app", G_APPLICATION_DEFAULT_FLAGS=0)`,
   stored at `ST_APPLICATION`.
3. `g_signal_connect_data(app, "activate", on_activate, …)`.
4. `g_application_run(app, argc, argv)` — forwards the real `argv` so GApplication
   platform-data is valid UTF-8. The loop owns the process until the window closes.
5. returns 0 → `__libc_start_main` calls `exit(0)`.

[[src/target/linux_gtk/bootstrap.rs:emit_main_bootstrap]]

## `activate`: window, widgets, pipe, worker

`on_activate(GtkApplication *app, gpointer)` constructs the UI and spawns the
worker (frame 32: `lr@0`, `pthread_t@8`, pipe fds@16, controller@24):
[[src/target/linux_gtk/bootstrap.rs:emit_activate_handler]]

- `gtk_application_window_new(app)` → `ST_WINDOW`; title `"MFBASIC App"`; default
  size 900×640.
- `scrolled = gtk_scrolled_window_new()` then `g_object_ref_sink` → `ST_SCROLLED`
  (an extra ref so swapping the window child to the term:: surface and back does
  not destroy it).
- `term_area = gtk_drawing_area_new()` + `g_object_ref_sink` → `ST_TERM_AREA`,
  draw func set to `_mfb_gtkapp_term_draw`; then `call_internal(TERM_INIT_SYMBOL)`
  derives the grid geometry on the main thread before the worker can use it.
- `text_view = gtk_text_view_new()`, editable=FALSE, monospace=TRUE; the view is
  deliberately left **non-focusable** (focusing a `GtkTextView` activates the
  IM/a11y machinery that crashes on worker inserts). `buffer =
  gtk_text_view_get_buffer(view)` → `ST_TEXT_BUFFER`. The scrolled window's child
  is the text view; the window's child is the scrolled window.
- Input is captured terminal-style by a `GtkEventControllerKey` added to the
  **window** (not a focusable input widget): `key-pressed` → `_mfb_gtkapp_key_pressed`,
  controller added via `gtk_widget_add_controller` (takes ownership).
- `close-request` → `_mfb_gtkapp_window_closed`; `gtk_window_present(window)`.
- `pipe(fds)`: read fd → `ST_PIPE_READ_FD`, write fd → `ST_PIPE_WRITE_FD`, and
  `dup2(read, 0)` so the reused console read helpers consume committed input on fd 0.
- `pthread_create(&thread, NULL, _mfb_gtkapp_worker, NULL)` then `pthread_detach`.

## Worker thread

`_mfb_gtkapp_worker(void *arg)` is the pthread start routine. It calls
`code::MACAPP_PROGRAM_SYMBOL` (the standard program entry). If
`spec.language_entry_accepts_args`, it passes `argc=0/argv=NULL`; argv is not
plumbed through to the worker. The program normally ends via `FINISH_SYMBOL`,
so the function tail (`return NULL`) is only reached defensively.
[[src/target/linux_gtk/bootstrap.rs:emit_worker_shim]]

## `_mfb_gtkapp_state` writable global

One writable runtime-state global holds every widget handle, the input-pipe fds,
the input-mode/line buffer, and the entire term:: grid backing store, so every
helper reaches them without register preservation. The data object is emitted
zero-initialized with `align: 8`, layout label
`mfb.runtime.gtkapp_state.v1 { u64 handles[7]; u64 argc; u64 argv; u64 mode; u64 lineLen; u8 lineBuf[] }`,
`size = STATE_SIZE`. [[src/target/linux_gtk/mod.rs:STATE_SYMBOL]]
[[src/target/linux_gtk/mod.rs:app_mode_data_objects]]

`Asm::state_array`/`load_state`/`store_state` materialize a field address (adrp/add
of `STATE_SYMBOL` + offset; offsets ≥ 4096 add via `x9`) — `x9` is the address
scratch, a recurring "load cellH/cur_fg before forming a value in x9" hazard noted
throughout the term code. [[src/target/linux_gtk/mod.rs:state_array]]

### Header + input fields

| Offset | Symbol | Field |
| --- | --- | --- |
| 0 | `ST_APPLICATION` | `GtkApplication*` |
| 8 | `ST_WINDOW` | `GtkWindow*` |
| 16 | `ST_SCROLLED` | `GtkScrolledWindow*` (held by ref) |
| 24 | `ST_TEXT_VIEW` | `GtkTextView*` (transcript) |
| 32 | `ST_TEXT_BUFFER` | `GtkTextBuffer*` |
| 40 | `ST_PIPE_READ_FD` | input pipe read fd (dup2'd onto 0) |
| 48 | `ST_PIPE_WRITE_FD` | input pipe write fd |
| 56 | `ST_ARGC` | process `argc`, for an arg-accepting entry |
| 64 | `ST_ARGV` | process `argv`, for an arg-accepting entry |
| 72 | `ST_INPUT_MODE` | `MODE_*` (line-noecho / line-echo / raw) |
| 80 | `ST_LINE_LEN` | pending uncommitted line length |
| 88 | `ST_LINE_BUF` | pending input bytes, `LINE_BUF_CAP = 1024` |

`_mfb_gtkapp_main` publishes `ST_ARGC`/`ST_ARGV` before `g_application_run`, and
the worker shim loads them to call an arg-accepting language entry. They live in
the state rather than riding `pthread_create`'s `arg` (as the macOS worker does)
because the GTK worker is created from the transient `activate` callback, which
cannot reach `_mfb_gtkapp_main`'s locals.

[[src/target/linux_gtk/mod.rs:ST_APPLICATION]]
[[src/target/linux_gtk/mod.rs:LINE_BUF_CAP]]

### term:: surface state and grid

The term:: section starts at `ST_TERM_AREA = ST_LINE_BUF + LINE_BUF_CAP = 1112`.
Cursor/cell/geometry fields are 8-byte slots; the parallel per-cell grids use a
fixed `TERM_MAX_COLS = 160` stride and `TERM_MAX_ROWS = 48` rows (storage is
static — active `cols`×`rows` are derived from window size + cell metrics and never
exceed the bounds). Each of the three grids is stored twice: the live copy the
worker mutates, and the draw-owned snapshot a present copies it into on the main
loop before `queue_draw`, so a draw can never observe a half-written frame
(plan-35-E). [[src/target/linux_gtk/mod.rs:ST_TERM_AREA]]
[[src/target/linux_gtk/mod.rs:TERM_MAX_COLS]]

| Offset | Symbol | Field |
| --- | --- | --- |
| 1112 | `ST_TERM_AREA` | `GtkDrawingArea*` (held by ref) |
| 1120 | `ST_TERM_ACTIVE` | 1 while term:: is on |
| 1128 | `ST_TERM_ROW` | cursor row |
| 1136 | `ST_TERM_COL` | cursor col |
| 1144 | `ST_TERM_CUR_FG` | current fg (packed `| COLOR_SET`) |
| 1152 | `ST_TERM_CUR_BG` | current bg (packed `| COLOR_SET`) |
| 1160 | `ST_TERM_CUR_BOLD` | current bold flag |
| 1168 | `ST_TERM_CUR_UNDERLINE` | current underline flag |
| 1176 | `ST_TERM_CURSOR_VISIBLE` | cursor visibility |
| 1184 | `ST_TERM_COLS` | active columns (derived) |
| 1192 | `ST_TERM_ROWS` | active rows (derived) |
| 1200 | `ST_TERM_CELL_W` | cell width (px) |
| 1208 | `ST_TERM_CELL_H` | cell height (px) |
| 1216 | `ST_TERM_CHARS` | live char grid, `u8[160*48]` = 7680 B |
| 8896 | `ST_TERM_FG` | live fg grid, `u32[160*48]` = 30720 B |
| 39616 | `ST_TERM_BG` | live bg grid, `u32[160*48]` = 30720 B |
| 70336 | `ST_TERM_SNAP_CHARS` | snapshot char grid, 7680 B |
| 78016 | `ST_TERM_SNAP_FG` | snapshot fg grid, 30720 B |
| 108736 | `ST_TERM_SNAP_BG` | snapshot bg grid, 30720 B |
| 139456 | `STATE_SIZE` | total |

[[src/target/linux_gtk/mod.rs:ST_TERM_CHARS]]
[[src/target/linux_gtk/mod.rs:STATE_SIZE]]

```text
_mfb_gtkapp_state layout (139456 bytes, align 8)
     0 ..    56  handles[7]  GtkApplication,Window,Scrolled,TextView,TextBuffer,
                             pipeRead,pipeWrite
    56 ..    72  argc (u64), argv (u64)
    72 ..    88  mode (u64), lineLen (u64)
    88 ..  1112  lineBuf[1024]
  1112 ..  1216  term cursor/cell/geometry scalars (13 u64 slots)
  1216 ..  8896  chars      u8[160*48]   (row stride = 160)
  8896 .. 39616  fg         u32[160*48]
 39616 .. 70336  bg         u32[160*48]
 70336 .. 78016  snapChars  u8[160*48]
 78016 ..108736  snapFg     u32[160*48]
108736 ..139456  snapBg     u32[160*48]
```

### Cell color/attribute encoding

fg/bg cells pack RGB in the low 24 bits (`r | g<<8 | b<<16`, the console
convention so the arena getters agree). `COLOR_SET = 1<<24` marks an explicit
color (so 0 means "use default" and explicit black stays distinct). `BOLD_FLAG =
1<<25` and `UNDERLINE_FLAG = 1<<26` ride in the fg word.
`TERM_DEFAULT_FG = "16777215"` (0xFFFFFF white). Font is `"monospace"` at
`TERM_FONT_SIZE = "16"`. [[src/target/linux_gtk/mod.rs:COLOR_SET]]

### Input modes / special keys

`ST_INPUT_MODE` selects: `MODE_LINE_NOECHO = "0"` (default / `io::readLine`),
`MODE_LINE_ECHO = "1"` (`io::input`), `MODE_RAW = "2"`
(`io::readChar`/`readByte`). `_mfb_gtkapp_key_pressed` (main thread) handles
RAW (write the key's UTF-8 bytes to the pipe immediately), LINE modes (accumulate
into `ST_LINE_BUF`; Enter commits `line + '\n'`; Backspace drops the last byte,
byte-granular and ASCII-only; printable keys append and
echo in LINE_ECHO). Special keyvals: `GDK_KEY_BACKSPACE = 65288`,
`GDK_KEY_RETURN = 65293`, `GDK_KEY_KP_ENTER = 65421`. Returns TRUE for consumed
keys, FALSE otherwise. [[src/target/linux_gtk/bootstrap.rs:emit_key_pressed_handler]]
[[src/target/linux_gtk/mod.rs:MODE_RAW]]

## term:: drawing surface (GtkDrawingArea + Cairo)

`_mfb_gtkapp_term_draw(area, cr, w, h, gpointer)` is the render callback (main
thread): paints black, then for each non-space cell draws an optional background
rect, then the glyph in its fg color and weight, then an optional 2px underline;
finally a 2px white cursor caret at `(ST_TERM_ROW, ST_TERM_COL)` when
`ST_TERM_CURSOR_VISIBLE`. [[src/target/linux_gtk/term_draw.rs:emit_term_draw_helper]]

- `_mfb_gtkapp_term_init` (main thread, at activate) measures the monospace cell
  from Cairo `font_extents.height` (cell H) and `text_extents("M").x_advance`
  (cell W) via a throwaway image surface, then `cols = clamp(900/cellW, 1, 160)`,
  `rows = clamp(640/cellH, 1, 48)` (`TERM_AREA_W=900`, `TERM_AREA_H=640`), and
  blanks the char grid to spaces. [[src/target/linux_gtk/term_draw.rs:emit_term_init_helper]]
- `_mfb_gtkapp_term_write(string, newline)` is the worker-side grid writer the io
  helpers call when term:: is active: pure grid mutation (safe off the main
  thread), advancing the cursor, wrapping at `cols`, scrolling via
  `_mfb_gtkapp_term_scroll` at the bottom, then `g_idle_add(redraw_idle)`.
  [[src/target/linux_gtk/term_draw.rs:emit_term_write_helper]]
- `_mfb_gtkapp_term_scroll` shifts each grid up one row (memmove) and blanks the
  last (memset). [[src/target/linux_gtk/term_draw.rs:emit_term_scroll_helper]]
- The `term_show_idle` / `term_hide_idle` / `term_redraw_idle` callbacks (each
  `G_SOURCE_REMOVE`) swap the window child to the drawing area / back to the
  scrolled transcript / queue a redraw — all on the main loop.
  [[src/target/linux_gtk/term_draw.rs:emit_term_show_idle_helper]]

`emit_app_term_helper` dispatches the `term::*` calls. `term::on`/`off` reset
attributes and toggle `ST_TERM_ACTIVE` plus the arena term-state
(`code::TERM_STATE_*_OFFSET`, so the console-backed getters agree) and schedule
the child swap; `setForeground`/`setBackground` write the arena (no flags) and the
app current-color field (with `COLOR_SET`); `setBold`/`setUnderline` mirror the
flag both places; `moveTo` clamps to the grid; `clear` blanks the backing store
and homes the cursor. The pinned arena base is `ARENA_REG = "x19"` (term helpers
run on the worker thread). [[src/target/linux_gtk/app_io.rs:emit_app_term_helper]]
[[src/target/linux_gtk/mod.rs:ARENA_REG]]

> `term::terminalSize` **is** implemented here (`OK({columns@0, rows@8})` from the
> derived grid size), unlike the `io::terminalSize` divergence below.
> [[src/target/linux_gtk/app_io.rs:emit_app_term_terminal_size]]

## io:: redirection

`emit_app_io_write_helper` (print/write/printError/writeError) takes the MFB
string in `x0` (`[x0]`=len, `x0+8`=UTF-8 bytes). Three paths, in order:
[[src/target/linux_gtk/app_io.rs:emit_app_io_write_helper]]

1. `ST_TERM_ACTIVE` set → `_mfb_gtkapp_term_write` (grid render); return OK.
2. else `ST_TEXT_BUFFER` non-nil → transcript path: copy the bytes (plus a
   `"[stderr] "` prefix for the error variants, plus a trailing `'\n'` for the
   newline variants) into a `malloc` chunk `[0]=len(u64), [16..]=bytes`, then
   `g_idle_add(_mfb_gtkapp_append_idle, chunk)`.
3. else (headless / pre-window) fd fallback: `write(fd, bytes, len)` to fd 1/2.

`_mfb_gtkapp_append_idle` (main thread) calls `_mfb_gtkapp_append` (insert at the
end iter + auto-scroll via a temporary mark) and frees the chunk.
`emit_app_io_input_helper` sets `MODE_LINE_ECHO`, writes the prompt via the io
write helper, then reads a committed line via `_mfb_rt_io_io_readLine` (which reads
fd 0). The app-mode `io` flush helper returns OK immediately without a marshaled
drain. The three `is*Terminal` helpers return `OK(TRUE)`.
`emit_set_raw_input_mode` (inlined into readChar/readByte) sets `MODE_RAW`.
[[src/target/linux_gtk/app_io.rs:emit_app_io_input_helper]]
[[src/target/linux_gtk/app_io.rs:emit_set_raw_input_mode]]

## Documented divergences from macOS

These are the observable behaviors of the Linux app backend that differ from the
macOS app runtime:
[[src/target/linux_gtk/app_io.rs:emit_app_io_write_helper]]

- **No main-thread marshal for the transcript-active path:** the fd fallback is
  the path exercised when no buffer is attached. The emitted code builds the
  `g_idle_add` chunk path, but term:: grid writes mutate the
  `GtkTextBuffer`/grid directly from the worker thread without the macOS-style
  main-thread hop for the grid itself.
- **The fd fallback** writes to stdout/stderr; the GTK transcript path is
  structurally present but not exercised on that path.
- **`finish` hard-exits.** `_mfb_gtkapp_finish` takes the exit code in `x0`; with
  no transcript attached it `_exit(code)`s, and the GUI path parks the worker in
  `pause()` (it must not `_exit` in GUI mode or the window dies). There is no
  "keep window open" path.
  [[src/target/linux_gtk/bootstrap.rs:emit_finish_helper]]
- **`io::printError` styling.** stderr runs *are* prefixed with `"[stderr] "`
  (`STR_STDERR_PREFIX`) in the transcript chunk; no distinct `GtkTextTag` styling
  is applied.
- **Interactive resize is implemented**: the drawing area's `resize`
  signal recomputes the active `cols`/`rows` and forces a full redraw, so
  `term::terminalSize` tracks the live window.

[[src/target/linux_gtk/mod.rs:STR_STDERR_PREFIX]]

## Libraries

App mode is glibc-only. `lib_for` maps each import to its shared object:
`libgtk-4.so.1` (gtk_* and GDK), `libgobject-2.0.so.0`, `libglib-2.0.so.0`,
`libgio-2.0.so.0`, `libc.so.6`, `libpthread.so.0`, `libcairo.so.2`. The relocation
library field is cosmetic (the linker binds by symbol name) but kept accurate for
artifact debugging. [[src/target/linux_gtk/mod.rs:lib_for]]

## See Also

- ./mfb spec app macos-runtime — the AppKit counterpart this backend mirrors
- ./mfb spec app console-io — the io:: window-redirection contract shared with macOS
- ./mfb spec app term-backend — the GUI term:: grid/cell model
- ./mfb spec memory program-startup — the console-mode entry/teardown sequence
- ./mfb spec linker static-and-dynamic-output — glibc-only ELF and app-mode imports
- ./mfb spec threading os-integration — the worker pthread the window drives
- ./mfb spec architecture commands — the `-app` build flag and `buildMode`
