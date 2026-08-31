//! Linux GTK4 app-mode codegen (plan-05-linux-app.md Phases 3-6).
//!
//! This is the Linux counterpart of `macos_aarch64/app/mod.rs`. It emits the GTK4
//! `_main` bootstrap, the language worker thread, the transcript/input widgets,
//! and the app-mode `io::*` helper bodies. Every GTK/GObject/GLib/GIO call is an
//! ordinary imported C function (no `objc_msgSend` layer), so the emitted code is
//! plain `bl <symbol>` against the imports declared in `app_mode_imports` below.
//!
//! The structure mirrors the macOS backend and is exercised on Linux+GTK
//! (Debian/Ubuntu GTK VMs). The notes below are the implemented main-thread
//! contract:
//!   * output `io::print`/`io::write` marshal every transcript write onto the main
//!     loop: the worker copies the bytes into a malloc'd chunk and posts it via
//!     `g_idle_add(APPEND_IDLE)`, so the `GtkTextBuffer` is only touched on the main
//!     thread (§6.4). The fd fallback (write to stdout/stderr) is used only when no
//!     window/buffer is attached (headless).
//!   * `io::printError` is prefix-distinguished on the marshaled path (not raw-
//!     appended).
//!   * the finish path parks the worker in `pause()` for the GUI case so the window
//!     stays open (§6.7); it `_exit`s only headless. `io::terminalSize` and
//!     interactive resize are wired — the grid reflows on the drawing area's
//!     `resize` signal (plan-35-E).

mod app_io;
mod bootstrap;
mod term_draw;

pub(crate) use app_io::*;
pub(crate) use bootstrap::*;
use term_draw::*;

use std::collections::HashMap;

use crate::arch::aarch64::abi;
use crate::codegen::engine::operand::Operand;
use crate::codegen::engine::types::AppEntrySpec;
use crate::codegen::engine::types::CodeDataObject;
use crate::codegen::engine::types::CodeFrame;
use crate::codegen::engine::types::CodeFunction;
use crate::codegen::engine::types::CodeInstruction;
use crate::codegen::engine::types::CodeRelocation;
use crate::codegen::engine::types::PresentationMode;
use crate::codegen::engine::types::RelocIntent;

// --- Emitted symbols -------------------------------------------------------

const MAIN_SYMBOL: &str = "_main";
/// The real C `main(argc, argv, envp)` the libc start path invokes after running
/// every loaded shared library's constructors (which boot the GLib/GObject type
/// system — GTK is unusable without them).
const GTK_MAIN_SYMBOL: &str = "_mfb_gtkapp_main";
const ACTIVATE_SYMBOL: &str = "_mfb_gtkapp_activate";
const WORKER_SYMBOL: &str = "_mfb_gtkapp_worker";
/// `key-pressed` handler on the transcript view (terminal-style input, no entry box).
const KEY_PRESSED_SYMBOL: &str = "_mfb_gtkapp_key_pressed";
const WINDOW_CLOSED_SYMBOL: &str = "_mfb_gtkapp_window_closed";
const APPEND_SYMBOL: &str = "_mfb_gtkapp_append";
/// bug-421: erase the final character (one whole UTF-8 code point) from the
/// transcript. Called by the LINE_ECHO Backspace path to keep the on-screen echo
/// in sync with the code-point-aware line buffer.
const DELETE_LAST_CHAR_SYMBOL: &str = "_mfb_gtkapp_delete_last_char";
/// Main-thread idle callback that drains one marshaled output chunk into the
/// transcript (scheduled from the worker thread via `g_idle_add`, plan-05 §6.4).
const APPEND_IDLE_SYMBOL: &str = "_mfb_gtkapp_append_idle";
/// Worker program-completion handler (referenced by `emit_program_exit`).
pub(crate) const FINISH_SYMBOL: &str = "_mfb_gtkapp_finish";
/// plan-62-D Phase 2: the GLib main-loop idle callback the worker's `setMode`
/// schedules (via `g_idle_add`) to build or tear down the transcript window to
/// match the new presentation mode.
const RECONCILE_IDLE_SYMBOL: &str = "_mfb_gtkapp_reconcile_idle";
/// plan-98-A Phase 3: builds the transcript window (window + scrolled + text view +
/// buffer) and installs the scrolled window as its child. Factored out of the
/// reconcile's `Console` arm because the `Canvas` arm needs the same window: a
/// canvas-first program has never presented a surface, so `ST_WINDOW` is still null
/// when it enters canvas mode, and both arms must be able to create one. Also gives
/// the canvas teardown something to restore the window's child *to*.
const RECONCILE_BUILD_SYMBOL: &str = "_mfb_gtkapp_reconcile_build";
/// plan-98-C Phase 3: the worker-side frame blit. Packs the rendered frame into a
/// `malloc` block (swizzling RGBA to the BGRX byte order Cairo's `RGB24` wants on
/// a little-endian host) and hands ownership to the main loop via `g_idle_add`.
const CANVAS_BLIT_SYMBOL: &str = "_mfb_gtkapp_canvas_blit";
/// The `g_idle_add` callback that takes ownership of a blitted frame: frees the
/// previous block, publishes the new one, and queues a redraw. Runs on the GTK main
/// loop, which is what makes `ST_CANVAS_PIXELS` single-threaded.
const CANVAS_COMMIT_SYMBOL: &str = "_mfb_gtkapp_canvas_commit";
/// The `GtkDrawingAreaDrawFunc` that paints the committed frame.
const CANVAS_DRAW_SYMBOL: &str = "_mfb_gtkapp_canvas_draw";

/// Writable runtime-state global. One pointer/handle per slot; the GTK widgets
/// and the window-input pipe fds live here so every helper can reach them without
/// register preservation (plan-05-linux-app.md §6.2).
const STATE_SYMBOL: &str = "_mfb_gtkapp_state";
/// plan-98-F: the headless gate's environment variable, the Linux twin of
/// `MFB_MACAPP_HEADLESS` / `MFB_WINAPP_HEADLESS`.
pub(super) const STR_HEADLESS_ENV: (&str, &str) =
    ("_mfb_gtkapp_str_headless", "MFB_GTKAPP_HEADLESS");

const ST_APPLICATION: usize = 0;
const ST_WINDOW: usize = 8;
const ST_SCROLLED: usize = 16;
const ST_TEXT_VIEW: usize = 24;
const ST_TEXT_BUFFER: usize = 32;
const ST_PIPE_READ_FD: usize = 40;
const ST_PIPE_WRITE_FD: usize = 48;
/// The process argc/argv, stashed by `_mfb_gtkapp_main` for the worker shim to
/// pass to an arg-accepting language entry (bug-240). They live here rather than
/// riding pthread_create's `arg` (the macOS approach) because the worker is
/// created from the transient `activate` callback, whose frame cannot host the
/// arg block; `_mfb_gtkapp_main`'s locals are not reachable from there.
const ST_ARGC: usize = 56;
const ST_ARGV: usize = 64;
/// Current input mode (see `MODE_*`): selects echo / raw key handling, exactly like
/// the macOS `INPUT_MODE_*` associated object.
const ST_INPUT_MODE: usize = 72;
/// Length of the pending (uncommitted) input line in `ST_LINE_BUF`.
const ST_LINE_LEN: usize = 80;
/// Accumulated bytes of the line being typed (committed to the pipe on Enter).
const ST_LINE_BUF: usize = 88;
// Kept modest so every state field stays within a 12-bit immediate add/ldr offset.
const LINE_BUF_CAP: usize = 1024;
// term:: TUI surface state (plan-01-term.md §6.3): a fixed character grid rendered
// by a GtkDrawingArea, the Linux analog of the macOS TermView.
const ST_TERM_AREA: usize = ST_LINE_BUF + LINE_BUF_CAP; // GtkDrawingArea*
const ST_TERM_ACTIVE: usize = ST_TERM_AREA + 8; // 1 while term:: is on
const ST_TERM_ROW: usize = ST_TERM_ACTIVE + 8; // cursor row
const ST_TERM_COL: usize = ST_TERM_ROW + 8; // cursor col
const ST_TERM_CUR_FG: usize = ST_TERM_COL + 8; // current fg (packed | COLOR_SET)
const ST_TERM_CUR_BG: usize = ST_TERM_CUR_FG + 8; // current bg (packed | COLOR_SET)
const ST_TERM_CUR_BOLD: usize = ST_TERM_CUR_BG + 8; // current bold flag
const ST_TERM_CUR_UNDERLINE: usize = ST_TERM_CUR_BOLD + 8; // current underline flag
const ST_TERM_CURSOR_VISIBLE: usize = ST_TERM_CUR_UNDERLINE + 8; // cursor visibility
                                                                 // Grid geometry DERIVED from the window size + monospace cell metrics (like macOS),
                                                                 // computed once at activate. cols/rows are the active extent; the backing arrays use
                                                                 // a fixed TERM_MAX_COLS stride so storage is static (no per-resize realloc).
const ST_TERM_COLS: usize = ST_TERM_CURSOR_VISIBLE + 8; // active columns
const ST_TERM_ROWS: usize = ST_TERM_COLS + 8; // active rows
const ST_TERM_CELL_W: usize = ST_TERM_ROWS + 8; // cell width in px
const ST_TERM_CELL_H: usize = ST_TERM_CELL_W + 8; // cell height in px
                                                  // Parallel per-cell grids: chars (u32), fg (u32 packed | flags), bg (u32 packed).
                                                  // Row stride is TERM_MAX_COLS; only the top-left cols x rows are active.
                                                  //
                                                  // A char cell holds ONE code point's UTF-8 bytes packed little-endian —
                                                  // lead byte in the low byte, zero-padded — so a `str_u32` into a 5-byte
                                                  // buffer lays the sequence out in order with a NUL terminator after it, and
                                                  // `cairo_show_text` gets the whole glyph. It was one byte per cell, which
                                                  // split a multi-byte glyph across cells and drew each fragment as tofu
                                                  // (bug-203). 4 bytes covers every code point (U+10FFFF encodes to 4).
                                                  //
                                                  // A blank cell is 0, not ' ': the blanking `memset`s write whole bytes, and
                                                  // a memset of ' ' over u32 cells would pack FOUR spaces per cell. The draw
                                                  // treats 0 and ' ' alike (both render nothing).
/// planning/term.md #11: cached "window was resized" flag. Set to 1 by
/// `_mfb_gtkapp_term_resize` on a genuine cols/rows change and read-and-cleared by
/// `term::didResize()`. Lives in the address-based GTK global (not the arena
/// term-state) so the main-loop resize callback and the worker-side getter both
/// reach it without the pinned arena register.
const ST_TERM_DID_RESIZE: usize = ST_TERM_CELL_H + 8;
const ST_TERM_CHARS: usize = ST_TERM_DID_RESIZE + 8;
const ST_TERM_FG: usize = ST_TERM_CHARS + TERM_MAX_COLS * TERM_MAX_ROWS * 4;
const ST_TERM_BG: usize = ST_TERM_FG + TERM_MAX_COLS * TERM_MAX_ROWS * 4;
// Draw-owned snapshot (front) copy of the three grid arrays (plan-35-E). The worker
// mutates the live arrays above; a present (`term::sync`/`io::flush`/`off`) copies the
// live arrays into this snapshot ON THE MAIN LOOP before `queue_draw`, and the draw
// callback reads the snapshot — so a draw can never observe a half-written frame
// (closing the former tearing caveat). Same fixed TERM_MAX_COLS×TERM_MAX_ROWS stride
// and COLOR_SET/bold/underline bit-packing as the live arrays (a raw memcpy preserves
// every packed bit).
const ST_TERM_SNAP_CHARS: usize = ST_TERM_BG + TERM_MAX_COLS * TERM_MAX_ROWS * 4;
const ST_TERM_SNAP_FG: usize = ST_TERM_SNAP_CHARS + TERM_MAX_COLS * TERM_MAX_ROWS * 4;
const ST_TERM_SNAP_BG: usize = ST_TERM_SNAP_FG + TERM_MAX_COLS * TERM_MAX_ROWS * 4;
// plan-70-E Phase 3: the EGC pool — a per-cell fixed slot of GTK_POOL_BYTES holding a
// multi-scalar grapheme cluster's UTF-8 bytes (NUL-terminated). A pooled cell's CHAR
// word is the GTK_POOL_TAG sentinel; the renderer rebuilds the cluster from the slot.
// Snapshot copy + scroll shift it in lockstep with the char/fg/bg arrays, just like
// the macOS TermView pool (per-cell slot, lifecycle-free).
const GTK_POOL_BYTES: usize = 32;
const GTK_POOL_TAG: &str = "4294967294"; // 0xFFFFFFFE (distinct from 0/32/WIDE_TRAIL)
const ST_TERM_POOL: usize = ST_TERM_SNAP_BG + TERM_MAX_COLS * TERM_MAX_ROWS * 4;
const ST_TERM_SNAP_POOL: usize = ST_TERM_POOL + TERM_MAX_COLS * TERM_MAX_ROWS * GTK_POOL_BYTES;
/// plan-62-D: 1 while a `g_application_hold` is in effect (windowless `None` mode),
/// 0 while a window owns the app's aliveness (`Console`). The reconcile keeps
/// exactly one aliveness source by toggling this: hold+set on entering `None`,
/// release+clear on entering `Console` (Open Decision 1).
const ST_HELD: usize = ST_TERM_SNAP_POOL + TERM_MAX_COLS * TERM_MAX_ROWS * GTK_POOL_BYTES;
/// plan-98-A Phase 3: the `Mode.Canvas` surface — a `GtkDrawingArea` swapped in as
/// the window's child in place of the transcript's scrolled window. `g_object_ref_sink`ed
/// on creation, exactly like [`ST_SCROLLED`], so `gtk_window_set_child` swapping it
/// away on mode exit unparents it without destroying it: enter → exit → re-enter
/// reuses the one widget rather than leaking a new one per cycle.
const ST_CANVAS_AREA: usize = ST_HELD + 8;
/// plan-98-A Phase 3: the window's `GdkSurface*`, read via `gtk_native_get_surface`
/// once canvas mode has presented the window. This is the display-server-agnostic
/// native handle plan-98-F turns into a `VkSurfaceKHR` (through
/// `gdk_x11_surface_get_xid` / `gdk_wayland_surface_get_wl_surface`, which are
/// backend-specific and so are *not* called here — retrieval only, no Vulkan).
/// Non-zero only while in canvas mode; cleared on exit, which is what makes
/// "released after exit" observable.
const ST_CANVAS_SURFACE: usize = ST_CANVAS_AREA + 8;
/// plan-98-C Phase 3: the committed frame, as one `malloc` block holding its own
/// width at +0, height at +8 and BGRX pixels from +16.
///
/// Width and height travel *inside* the block rather than in their own state slots
/// so that one pointer carries a whole frame. That is what makes the handoff
/// race-free without a lock: the worker builds a block nobody else can see, hands
/// the pointer to `g_idle_add`, and every read *and* write of this slot happens on
/// the GTK main loop. A separate width slot would have to be published separately,
/// and a frame could then be drawn with the previous frame's dimensions.
const ST_CANVAS_PIXELS: usize = ST_CANVAS_SURFACE + 8;
const STATE_SIZE: usize = ST_CANVAS_PIXELS + 8;

// fg/bg cell encoding: low 24 bits = packed RGB (r|g<<8|b<<16, the console
// convention so the arena getters agree); bit 24 marks an explicit color (so 0 =
// "use default", letting black be set distinctly); bit 25 (fg) = bold, bit 26 (fg)
// = underline.
const COLOR_SET: usize = 1 << 24;
const BOLD_FLAG: usize = 1 << 25;
const UNDERLINE_FLAG: usize = 1 << 26;
// plan-70-E: the display width (0/1/2) of the cell's grapheme rides in the fg word's
// free bits 27-28, so the snapshot memcpy + resize/scroll array shifts carry it for
// free (no separate width array). A wide (width-2) glyph reserves the next cell as a
// WIDE_TRAIL sentinel in the CHAR array — 0xFFFFFFFF is not valid UTF-8 (0xFF is not a
// lead byte), so the renderer's blank check distinguishes it, and it never collides
// with a real packed-UTF-8 cell.
const WIDTH_SHIFT: usize = 27;
const GTK_WIDE_TRAIL: &str = "4294967295"; // 0xFFFFFFFF
const TERM_DEFAULT_FG: &str = "16777215"; // 0xFFFFFF white (matches console default)

// Backing-store bounds for the grid (a fixed stride keeps storage static). The
// active cols/rows are derived from the window size and font cell metrics and never
// exceed these.
const TERM_MAX_COLS: usize = 160;
const TERM_MAX_ROWS: usize = 48;
// Window content area used to size the grid (matches the default window size, like
// macOS sizing from the TermView frame).
const TERM_AREA_W: usize = 900;
const TERM_AREA_H: usize = 640;
/// Drawing-area draw callback symbol.
const TERM_DRAW_SYMBOL: &str = "_mfb_gtkapp_term_draw";
/// Main-thread idle callbacks (GTK calls must run on the main loop): show the grid,
/// restore the transcript, and request a grid redraw.
const TERM_SHOW_IDLE_SYMBOL: &str = "_mfb_gtkapp_term_show_idle";
const TERM_HIDE_IDLE_SYMBOL: &str = "_mfb_gtkapp_term_hide_idle";
const TERM_REDRAW_IDLE_SYMBOL: &str = "_mfb_gtkapp_term_redraw_idle";
/// Worker-side grid writer shared by the io write helpers when term:: is active.
const TERM_WRITE_SYMBOL: &str = "_mfb_gtkapp_term_write";
/// Worker-side grid scroll-up (called from term_write at the bottom edge).
const TERM_SCROLL_SYMBOL: &str = "_mfb_gtkapp_term_scroll";
/// Computes grid geometry from font metrics + content size; run once on the main
/// thread at activate, before the worker can touch the grid.
const TERM_INIT_SYMBOL: &str = "_mfb_gtkapp_term_init";
/// `GtkDrawingArea::resize` handler (plan-35-E): recomputes the active cols/rows from
/// the new allocation + cell metrics so `term::terminalSize` tracks the live window
/// and forces a full redraw. Runs on the GTK main loop.
const TERM_RESIZE_SYMBOL: &str = "_mfb_gtkapp_term_resize";

// Input modes (mirror macOS `app/mod.rs` INPUT_MODE_*): line-buffered without echo is the
// default (`io::readLine`), line-buffered with echo is `io::input`, and raw delivers
// each keystroke's bytes to the pipe immediately (`io::readChar`/`readByte`).
/// Default mode: line-buffered, no echo (the zero-initialized state value).
///
/// Never assigned, and that is the point: it is the value the state word
/// already holds, so no code writes it. The specification documents it as
/// such (`app/03_console-io.md`: "default; never set explicitly"), and it
/// completes the enumeration its two live siblings belong to (bug-326-D6).
#[allow(dead_code)]
const MODE_LINE_NOECHO: &str = "0";
const MODE_LINE_ECHO: &str = "1";
const MODE_RAW: &str = "2";

// GDK keyvals for the keys the transcript handles specially.
const GDK_KEY_BACKSPACE: &str = "65288"; // 0xFF08
const GDK_KEY_RETURN: &str = "65293"; // 0xFF0D
const GDK_KEY_KP_ENTER: &str = "65421"; // 0xFF8D

// Reused runtime helper symbols (the console io::write / io::readLine bodies feed
// the transcript prompt + the fd-0 window-input pipe respectively).
const IO_WRITE_SYMBOL: &str = "_mfb_rt_io_io_write";
const IO_READ_LINE_SYMBOL: &str = "_mfb_rt_io_io_readLine";

// --- Read-only string data symbols -----------------------------------------

/// Symbol names for the two strings that carry the app's *identity* (plan-51-A
/// §4.5). Both were compile-time constants until plan-51: every MFBASIC GTK app
/// on a machine shared one D-Bus name and one window class, so no `.desktop`
/// file could associate its launcher with a window. Their values are now derived
/// from the project name by [`gtk_app_id`] and [`app_mode_data_objects`].
const SYM_APP_ID: &str = "_mfb_gtkapp_str_app_id";
const SYM_TITLE: &str = "_mfb_gtkapp_str_title";
const STR_ACTIVATE: (&str, &str) = ("_mfb_gtkapp_str_activate", "activate");
const STR_CLOSE_REQUEST: (&str, &str) = ("_mfb_gtkapp_str_close_request", "close-request");
const STR_KEY_PRESSED: (&str, &str) = ("_mfb_gtkapp_str_key_pressed", "key-pressed");
/// `GtkDrawingArea::resize` signal name (plan-35-E grid reflow on window resize).
const STR_RESIZE: (&str, &str) = ("_mfb_gtkapp_str_resize", "resize");
/// Completion status line appended to the transcript when the program ends
/// (matches macOS `app/mod.rs` STR_EXIT_PREFIX): leading newline + "...code " + N + "\n".
const STR_EXIT_PREFIX: (&str, &str) =
    ("_mfb_gtkapp_str_exit_prefix", "\nProgram exited with code ");
/// Marker prepended to `printError`/`writeError` transcript runs (matches macOS
/// `app/mod.rs` STR_STDERR_PREFIX), visually distinguishing stderr (plan-05 §5.4).
const STR_STDERR_PREFIX: (&str, &str) = ("_mfb_gtkapp_str_stderr_prefix", "[stderr] ");
/// Cairo font family for the term:: grid.
/// plan-70-E: the Pango font-description string ("family size"), parsed once by
/// `pango_font_description_from_string` into the layout's cached description.
const STR_MONO_DESC: (&str, &str) = ("_mfb_gtkapp_str_mono_desc", "monospace 16");
/// Representative glyph used to measure the monospace cell width.
const STR_M: (&str, &str) = ("_mfb_gtkapp_str_m", "M");

// In-process disable of the a11y + input-method layers, whose g_variant_new_string
// path crashes when the worker inserts transcript text. Set before GTK initializes.
const STR_ENV_A11Y: (&str, &str) = ("_mfb_gtkapp_env_a11y", "GTK_A11Y");
const STR_ENV_IM: (&str, &str) = ("_mfb_gtkapp_env_im", "GTK_IM_MODULE");
const STR_ENV_NONE: (&str, &str) = ("_mfb_gtkapp_env_none", "none");

// --- GTK / GObject enum immediates -----------------------------------------

const G_APPLICATION_DEFAULT_FLAGS: &str = "0";
const TRUE: &str = "1";
const FALSE: &str = "0";
const WINDOW_WIDTH: &str = "900";
const WINDOW_HEIGHT: &str = "640";

// --- Library names ---------------------------------------------------------
//
// The GTK/GLib/Cairo sonames are identical on both libc worlds. The C-library
// names are NOT: they are resolved by the calling backend's `Platform` and
// passed in as [`AppLibcNames`] (plan-56-A §4.1), because app mode is no longer
// no longer glibc-only (plan-56-B).

const GTK: &str = "libgtk-4.so.1";
const GOBJECT: &str = "libgobject-2.0.so.0";
const GLIB: &str = "libglib-2.0.so.0";
const GIO: &str = "libgio-2.0.so.0";
const CAIRO: &str = "libcairo.so.2";
// plan-70-E: the TUI grid draws through Pango (font cascade for CJK/emoji),
// replacing the Cairo toy font API which has no fallback.
const PANGO: &str = "libpango-1.0.so.0";
const PANGOCAIRO: &str = "libpangocairo-1.0.so.0";

/// The C-library sonames an app-mode build binds to (plan-56-A §4.1), resolved
/// by the calling backend's `Platform` so `linux_gtk` needs to know neither the
/// arch nor the libc-naming convention.
///
/// `libc` is `libc.so.6` on glibc and `libc.musl-<arch>.so.1` on musl;
/// `libpthread` is `libpthread.so.0` on glibc and the same string as `libc` on
/// musl, where pthread lives inside libc.
///
/// ⚠️ Getting these wrong is **invisible at runtime**: musl's loader absorbs
/// `libc.so.6` and `libpthread.so.0` into itself, so a musl binary carrying the
/// glibc names loads and runs identically to a correct one (verified on stock
/// Alpine x86_64 and aarch64, gcompat absent — plan-56-A §2.4). Only
/// `readelf -d` can tell them apart.
#[derive(Clone, Copy)]
pub(crate) struct AppLibcNames {
    pub libc: &'static str,
    pub libpthread: &'static str,
}

/// Placeholder written into a relocation's `library` at emit time, resolved by
/// `shared::crate::codegen::engine::builder::bind_deferred_relocation_libraries` (plan-56-A §4.2).
///
/// This replaced a `lib_for` symbol→library table that was a **second** copy of
/// `app_mode_imports`, obliged by its own doc comment to stay in sync with it —
/// the same two-derivations-of-one-value shape as the plan-46-D §1 `.dynstr`
/// bug. Binding from the import map instead makes disagreement unrepresentable
/// rather than merely discouraged, and makes the label flavor-correct for free.
///
/// The alternative — resolving here — would mean threading the libc flavor
/// through ~30 emitter signatures and 33 `Asm::new` sites, to duplicate a
/// mapping the native plan already owns.
const UNBOUND_LIBRARY: &str = "";

// --- Tiny assembler over CodeInstruction/CodeRelocation --------------------

struct Asm {
    from: String,
    ins: Vec<CodeInstruction>,
    rel: Vec<CodeRelocation>,
    /// bug-176 D: the first `lib_for` failure (an unmapped symbol) recorded here so
    /// `finish` can surface it as a plan-level error instead of `panic!`ing. Kept on
    /// the builder so the many infallible `call_external` sites need not change.
    err: Option<String>,
}

impl Asm {
    fn new(from: &str) -> Self {
        Asm {
            from: from.to_string(),
            ins: Vec::new(),
            rel: Vec::new(),
            err: None,
        }
    }

    fn push(&mut self, instruction: CodeInstruction) {
        self.ins.push(instruction);
    }

    /// `bl <symbol>` to an imported C function.
    ///
    /// The relocation's `library` is deferred ([`UNBOUND_LIBRARY`]) and filled
    /// in by `shared::crate::codegen::engine::builder::bind_deferred_relocation_libraries` from the
    /// flavor-correct import map, so no emitter needs to know the libc flavor or
    /// the arch. bug-176 D's "unmapped symbol is an error, never a panic" rule
    /// is preserved there: an undeclared symbol fails the build with a message
    /// naming it.
    fn call_external(&mut self, symbol: &str) {
        let library = UNBOUND_LIBRARY;
        self.ins.push(abi::branch_link(symbol));
        self.rel.push(CodeRelocation {
            from: self.from.clone(),
            to: symbol.to_string(),
            kind: RelocIntent::Call,
            binding: "external".to_string(),
            library: Some(library.to_string()),
        });
    }

    /// `bl <symbol>` to an internal text symbol.
    fn call_internal(&mut self, symbol: &str) {
        self.ins.push(abi::branch_link(symbol));
        self.rel.push(CodeRelocation {
            from: self.from.clone(),
            to: symbol.to_string(),
            kind: RelocIntent::Call,
            binding: "internal".to_string(),
            library: None,
        });
    }

    /// Materialize an internal data/text symbol's address into `dst` (adrp/add).
    fn local_address(&mut self, dst: impl Into<Operand>, symbol: &str) {
        let dst = dst.into();
        self.push(
            CodeInstruction::new("adrp")
                .field("dst", &dst)
                .field("symbol", symbol),
        );
        self.push(
            CodeInstruction::new("add_pageoff")
                .field("dst", &dst)
                .field("src", &dst)
                .field("symbol", symbol),
        );
        for kind in [RelocIntent::DataAddrHi, RelocIntent::DataAddrLo] {
            self.rel.push(CodeRelocation {
                from: self.from.clone(),
                to: symbol.to_string(),
                kind,
                binding: "data".to_string(),
                library: None,
            });
        }
    }

    /// Materialize the address of a runtime-state field/array at `offset` into
    /// `dst` (clobbers the first scratch-pool register, realized `x9`, for large
    /// offsets past the 12-bit add immediate). Spelled with the neutral scratch
    /// token — not raw `x9` — so a caller injected into a vreg-finalized
    /// `abi_function` body (plan-101 append shape) does not trip the plan-34-D
    /// zero-physical-register guard; realized to the same `x9` in a standalone body.
    fn state_array(&mut self, dst: impl Into<Operand>, offset: usize) {
        let dst = dst.into();
        self.local_address(dst.clone(), STATE_SYMBOL);
        if offset < 4096 {
            self.push(abi::add_immediate(dst.clone(), dst, offset));
        } else {
            self.push(abi::move_immediate(
                abi::SCRATCH[0],
                "Integer",
                &offset.to_string(),
            ));
            self.push(abi::add_registers(dst.clone(), dst, abi::SCRATCH[0]));
        }
    }

    /// Load runtime-state field `offset` into `dst` (clobbers the first
    /// scratch-pool register, realized `x9`). Spelled with the neutral scratch
    /// token — not raw `x9` — so a caller injected into a vreg-finalized
    /// `abi_function` body (plan-101 append shape) does not trip the plan-34-D
    /// zero-physical-register guard; realized to the same `x9` in a standalone body.
    fn load_state(&mut self, dst: impl Into<Operand>, offset: usize) {
        self.local_address(abi::SCRATCH[0], STATE_SYMBOL);
        self.push(abi::load_u64(dst, abi::SCRATCH[0], offset));
    }

    /// Store `src` into runtime-state field `offset` (clobbers the first
    /// scratch-pool register, realized `x9`). Spelled with the neutral token
    /// because some callers' sequences are injected into shared helper bodies,
    /// which the plan-34-D stream guard requires to be token-pure.
    fn store_state(&mut self, src: impl Into<Operand>, offset: usize) {
        self.local_address(abi::SCRATCH[0], STATE_SYMBOL);
        self.push(abi::store_u64(src, abi::SCRATCH[0], offset));
    }

    fn finish(self, symbol: &str, returns: &str) -> Result<CodeFunction, String> {
        // bug-176 D: surface a recorded `lib_for` failure (unmapped symbol) here as
        // a plan-level error instead of `panic!`ing at the call site.
        if let Some(message) = self.err {
            return Err(message);
        }
        Ok(CodeFunction {
            name: symbol.to_string(),
            symbol: symbol.to_string(),
            params: Vec::new(),
            returns: returns.to_string(),
            frame: CodeFrame {
                stack_size: 0,
                callee_saved: Vec::new(),
            },
            stack_slots: Vec::new(),
            instructions: self.ins,
            relocations: self.rel,
        })
    }
}

// --- Bootstrap + UI + worker -----------------------------------------------

/// Emit the GTK4 `_main` bootstrap and supporting functions. The standard program
/// entry runs separately on the worker thread under [`crate::codegen::error::constants::MACAPP_PROGRAM_SYMBOL`].
pub(crate) fn emit_app_program_entry(
    spec: &AppEntrySpec,
    _platform_imports: &HashMap<String, String>,
) -> Result<Vec<CodeFunction>, String> {
    let mut functions = vec![
        emit_libc_start_trampoline()?,
        emit_main_bootstrap()?,
        emit_activate_handler(spec.initial_mode)?,
        emit_worker_shim(spec)?,
        emit_key_pressed_handler()?,
        emit_window_closed_handler()?,
        emit_finish_helper()?,
        emit_append_helper()?,
        emit_delete_last_char_helper()?,
        emit_append_idle_helper()?,
        // term:: TUI surface support (plan-01-term.md §6.3).
        emit_term_draw_helper()?,
        emit_term_show_idle_helper()?,
        emit_term_hide_idle_helper()?,
        emit_term_redraw_idle_helper()?,
        emit_term_write_helper(spec.uses_term)?,
        emit_term_scroll_helper()?,
        emit_term_init_helper()?,
        emit_term_resize_helper()?,
    ];
    // plan-62-D Phase 2: the runtime setMode reconcile idle callback, only for a
    // program that can change mode (static default `None`) — a `Console`-default
    // program never reconciles and keeps its exact function set.
    if spec.initial_mode == PresentationMode::None {
        functions.push(emit_reconcile_build_helper()?);
        functions.push(emit_reconcile_idle_helper(spec.uses_canvas)?);
    }
    // plan-98-C Phase 3: the frame blit's worker side, its main-loop commit, and the
    // drawing area's paint callback. Gated on the program *drawing* rather than on
    // its start mode — see the macOS twin for why the two are not the same question.
    if spec.uses_canvas {
        functions.push(emit_canvas_blit_helper()?);
        functions.push(emit_canvas_commit_helper()?);
        functions.push(emit_canvas_draw_helper()?);
    }
    Ok(functions)
}

/// plan-98-C Phase 3: the worker-side `canvas::blitSurface` seam.
///
/// The caller has already staged the frame pointer, width and height in the MFB
/// argument registers, which is what [`bootstrap::emit_canvas_blit_helper`] expects,
/// so this is a plain call. Unlike the mode reconcile it does *not* go straight to
/// `g_idle_add`: the frame has to be copied out of the caller's block before the
/// worker returns, and only the blit helper can do that.
pub(crate) fn emit_canvas_blit_seam(
    from_symbol: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let mut asm = Asm::new(from_symbol);
    asm.call_internal(CANVAS_BLIT_SYMBOL);
    instructions.extend(asm.ins);
    relocations.extend(asm.rel);
}

/// The x86-64 flavor of [`emit_app_program_entry`]: the ELF-entry trampoline is
/// per-ISA (the SysV `__libc_start_main` call passes its 7th argument on the
/// stack), and every other function — GTK signal callbacks, GLib idle callbacks,
/// the pthread worker shim — is bracketed by [`wrap_x86_instructions`] so it
/// honors the SysV callee-saved contract and the runtime's zero-register
/// convention (see that function's doc).
pub(crate) fn emit_app_program_entry_x86(
    spec: &AppEntrySpec,
    _platform_imports: &HashMap<String, String>,
) -> Result<Vec<CodeFunction>, String> {
    let mut functions = vec![
        emit_main_bootstrap()?,
        emit_activate_handler(spec.initial_mode)?,
        emit_worker_shim(spec)?,
        emit_key_pressed_handler()?,
        emit_window_closed_handler()?,
        emit_finish_helper()?,
        emit_append_helper()?,
        emit_delete_last_char_helper()?,
        emit_append_idle_helper()?,
        emit_term_draw_helper()?,
        emit_term_show_idle_helper()?,
        emit_term_hide_idle_helper()?,
        emit_term_redraw_idle_helper()?,
        emit_term_write_helper(spec.uses_term)?,
        emit_term_scroll_helper()?,
        emit_term_init_helper()?,
        emit_term_resize_helper()?,
    ];
    // plan-62-D Phase 2: the reconcile idle callback (None-default programs only).
    if spec.initial_mode == PresentationMode::None {
        functions.push(emit_reconcile_build_helper()?);
        functions.push(emit_reconcile_idle_helper(spec.uses_canvas)?);
    }
    // plan-98-C Phase 3: the frame blit's worker side, its main-loop commit, and the
    // drawing area's paint callback. Gated on the program *drawing* rather than on
    // its start mode — see the macOS twin for why the two are not the same question.
    if spec.uses_canvas {
        functions.push(emit_canvas_blit_helper()?);
        functions.push(emit_canvas_commit_helper()?);
        functions.push(emit_canvas_draw_helper()?);
    }
    for function in &mut functions {
        finalize_x86_app_function(&mut function.instructions);
    }
    // The trampoline is the raw ELF entry (no caller, no callee-saved contract,
    // kernel-aligned stack) — unwrapped, first.
    functions.insert(0, emit_libc_start_trampoline_x86()?);
    Ok(functions)
}

/// x86-64 ELF entry: hand off to `__libc_start_main`, which initializes the C
/// runtime — crucially `environ` (GTK needs `DISPLAY`) — and then calls the real
/// `_mfb_gtkapp_main`. SysV passes the first six arguments in registers and the
/// seventh (`stack_end`) on the stack; the kernel enters `_main` with `rsp`
/// 16-aligned pointing at `argc`, so the 16-byte slot below keeps the call site
/// 16-aligned as the ABI requires. `__libc_start_main` never returns.
fn emit_libc_start_trampoline_x86() -> Result<CodeFunction, String> {
    let mut asm = Asm::new(MAIN_SYMBOL);
    asm.push(abi::label("entry"));
    // __libc_start_main's six C arguments: main, argc, argv, init, fini, rtld_fini.
    asm.local_address(abi::c_arg(0), GTK_MAIN_SYMBOL); // main
    asm.push(abi::load_u64(abi::c_arg(1), abi::stack_pointer(), 0)); // argc
    asm.push(abi::add_immediate(abi::c_arg(2), abi::stack_pointer(), 8)); // argv
    asm.push(abi::move_immediate(abi::c_arg(3), "Integer", "0")); // init
    asm.push(abi::move_immediate(abi::c_arg(4), "Integer", "0")); // fini
    asm.push(abi::move_immediate(abi::c_arg(5), "Integer", "0")); // rtld_fini
                                                                  // stack_end = the entry sp, passed as the 7th (stack) argument.
    asm.push(abi::add_immediate(abi::SCRATCH[0], abi::stack_pointer(), 0));
    asm.push(abi::subtract_stack(16));
    asm.push(abi::store_u64(abi::SCRATCH[0], abi::stack_pointer(), 0));
    asm.call_external("__libc_start_main");
    asm.push(abi::branch_self());
    asm.push(abi::return_());
    asm.finish(MAIN_SYMBOL, "Nothing")
}

/// The wrap bracket's base size: four callee-saved slots + padding that flips
/// the stack parity so a C-callee-entered body (rsp ≡ 8 mod 16) reaches its
/// interior call sites 16-aligned. No hand-built app frame is 56 bytes, so the
/// bracket's `sub`/`add` are identifiable by this immediate when the spill area
/// is folded in after allocation.
const X86_WRAP_BYTES: usize = 56;

/// The x86-64 callee-saved GPRs the wrap bracket saves/restores around a
/// hand-built app body: the general callee-saved bank the interior code (the
/// linear-scan allocator's output plus the residual scratch map) may land on,
/// minus `rbp` (the frame pointer) and `r15` (the pinned arena base). These are
/// listed as *data* — the physical save targets of the machine-floor bracket,
/// which by construction cannot be virtual or neutral-token operands (they wrap
/// the vreg-allocated body and must name the concrete callee-saved registers to
/// preserve). `r14` (index 3) additionally doubles as the runtime zero register
/// and is zeroed after being saved.
const X86_BRACKET_CALLEE_SAVED: [&str; 4] = ["rbx", "r12", "r13", "r14"];

/// Finalize a hand-built app function for x86-64. These bodies were written
/// against the AArch64 register conventions: `x9`–`x17` caller-saved scratch
/// and `x19`–`x28` callee-saved parking, 19 distinct registers. The x86
/// selection folds that space onto an 11-entry pool where `xN` and `xN+11`
/// alias (x9/x20 → rbx, …) and six of the pool's registers are SysV
/// **caller**-saved — so aliased pairs clobber each other and parked values die
/// across C calls. Instead of hand-auditing every body, this renames the whole
/// scratch/parking space to virtual registers and runs the shared linear-scan
/// allocator against the real x86 register model — exactly how the builder
/// path handles the same problem — with the spill area folded into the wrap
/// bracket below the function's own frame.
///
/// The bracket itself saves the callee-saved registers the allocator may hand
/// out (`rbx`/`r12`/`r13`) plus the pinned zero register `r14`, zeroes `r14`
/// for the runtime's zero-register convention (a GTK callback arrives with a
/// foreign value in it), restores all four at every return, and keeps the
/// interior 16-aligned. `x19` stays physical: the selection realizes it as the
/// pinned arena register, callee-saved either way, and the bodies that use it
/// as plain scratch save/restore it through their own frame slots.
pub(crate) fn finalize_x86_app_function(instructions: &mut Vec<CodeInstruction>) {
    use crate::arch::ops::CodeOp;
    use crate::codegen::engine::mir;
    use crate::codegen::engine::regalloc;

    // Rename the AArch64 scratch/parking registers to per-function vregs (one
    // per distinct register, preserving each def/use chain — the same mapping
    // the retired vregify pass used). The bodies now spell that space with the
    // neutral `abi::SCRATCH`/`abi::LOCAL` tokens (plan-34-D), so map each token
    // back to the AArch64 index it realizes to before applying the identical
    // `x9`–`x17` / `x20`–`x28` predicate — `%scratchK` = `x{9+K}`, `%localK` =
    // `x{19+K}`, and a raw `xN` (still emitted by a few bodies) verbatim. The set
    // is byte-identical to the raw-`xN` era: `x18` (`%scratch9`) and `x19`
    // (`%local0`, the pinned arena) stay physical, exactly as before.
    let scratch_index = |name: &str| -> Option<u32> {
        if let Some(n) = name
            .strip_prefix('x')
            .and_then(|rest| rest.parse::<u32>().ok())
        {
            return Some(n);
        }
        if let Some(k) = name
            .strip_prefix("%scratch")
            .and_then(|rest| rest.parse::<u32>().ok())
        {
            return Some(9 + k);
        }
        if let Some(k) = name
            .strip_prefix("%local")
            .and_then(|rest| rest.parse::<u32>().ok())
        {
            return Some(19 + k);
        }
        None
    };
    let is_scratch = |name: &str| -> bool {
        scratch_index(name).is_some_and(|n| (9..=17).contains(&n) || (20..=28).contains(&n))
    };
    let mut order: Vec<String> = Vec::new();
    for instruction in instructions.iter() {
        for (_, value) in &instruction.fields {
            let rendered = value.render();
            if is_scratch(&rendered) && !order.contains(&rendered) {
                order.push(rendered);
            }
        }
    }
    let rename: HashMap<String, String> = order
        .into_iter()
        .enumerate()
        .map(|(index, register)| (register, format!("%v{index}")))
        .collect();
    for instruction in instructions.iter_mut() {
        for (_, value) in instruction.fields.iter_mut() {
            if let Some(vreg) = rename.get(&value.render()) {
                *value = Operand::from(vreg.as_str());
            }
        }
    }

    // The function's own frame (its first sub_sp): the spill area sits above it
    // and above the wrap slots, all addressed from the frame-level sp.
    let inner_frame = instructions
        .iter()
        .find(|instruction| instruction.op == CodeOp::SubSp)
        .and_then(|instruction| instruction.get("imm"))
        .and_then(|imm| imm.parse::<usize>().ok())
        .unwrap_or(0);

    // Bracket: save/zero/restore + parity.
    let entry_at = usize::from(
        instructions
            .first()
            .is_some_and(|instruction| instruction.op == CodeOp::Label),
    );
    let prologue = vec![
        abi::subtract_stack(X86_WRAP_BYTES),
        abi::store_u64(X86_BRACKET_CALLEE_SAVED[0], abi::stack_pointer(), 0),
        abi::store_u64(X86_BRACKET_CALLEE_SAVED[1], abi::stack_pointer(), 8),
        abi::store_u64(X86_BRACKET_CALLEE_SAVED[2], abi::stack_pointer(), 16),
        abi::store_u64(X86_BRACKET_CALLEE_SAVED[3], abi::stack_pointer(), 24),
        abi::exclusive_or_registers(
            X86_BRACKET_CALLEE_SAVED[3],
            X86_BRACKET_CALLEE_SAVED[3],
            X86_BRACKET_CALLEE_SAVED[3],
        ),
    ];
    instructions.splice(entry_at..entry_at, prologue);
    let mut index = entry_at + 6;
    while index < instructions.len() {
        if instructions[index].op == CodeOp::Ret {
            let epilogue = vec![
                abi::load_u64(X86_BRACKET_CALLEE_SAVED[0], abi::stack_pointer(), 0),
                abi::load_u64(X86_BRACKET_CALLEE_SAVED[1], abi::stack_pointer(), 8),
                abi::load_u64(X86_BRACKET_CALLEE_SAVED[2], abi::stack_pointer(), 16),
                abi::load_u64(X86_BRACKET_CALLEE_SAVED[3], abi::stack_pointer(), 24),
                abi::add_stack(X86_WRAP_BYTES),
            ];
            let count = epilogue.len();
            instructions.splice(index..index, epilogue);
            index += count + 1;
        } else {
            index += 1;
        }
    }

    // Select to x86 ops (role remap + scratch map for anything left physical),
    // then color the vregs. The later plan-assembly MIR routing round-trips the
    // already-selected stream as an identity pass.
    let neutral = mir::lower_to_mir_owned(std::mem::take(instructions));
    let backend = mir::active_backend();
    *instructions = backend.select(neutral);
    let spill_base = inner_frame + X86_WRAP_BYTES;
    let outcome = regalloc::allocate(instructions, backend.register_model(), spill_base, &[]);
    let spill_bytes = outcome.spill_slots.len() * backend.register_model().spill_slot_bytes();
    // Round to 16 so the bracket keeps the interior alignment parity.
    let spill_bytes = (spill_bytes + 15) & !15;
    if spill_bytes > 0 {
        let sentinel = X86_WRAP_BYTES.to_string();
        let bumped = (X86_WRAP_BYTES + spill_bytes).to_string();
        for instruction in instructions.iter_mut() {
            if matches!(instruction.op, CodeOp::SubSp | CodeOp::AddSp) {
                for (key, value) in instruction.fields.iter_mut() {
                    if *key == "imm" && *value == sentinel.as_str() {
                        *value = Operand::from(bumped.as_str());
                    }
                }
            }
        }
    }
}

/// The app-mode platform import set, shared by the aarch64 and x86-64 Linux
/// plans (plan-05-linux-app.md §6.4). The C-library sonames are flavor-derived
/// (plan-56-A §4.1), so the
/// library names are fixed: GTK is plain C and every call is an ordinary
/// imported function; `__libc_start_main` runs the C runtime init before the
/// real `main`; pthread spawns the language worker; the pipe primitives feed
/// window input to the reused fd-0 console readers.
pub(crate) fn app_mode_imports(
    libc_names: AppLibcNames,
) -> Vec<crate::target::shared::plan::PlatformImport> {
    use crate::target::shared::plan::PlatformImport;
    let AppLibcNames { libc, libpthread } = libc_names;
    let gtk: &[(&str, &str)] = &[
        // Application + window lifecycle.
        (GIO, "g_application_run"),
        (GIO, "g_application_quit"),
        // plan-62-D: keep a windowless (`None`-mode) app alive, and balance the
        // hold when a window takes over aliveness on `setMode(Console)`.
        // (`g_idle_add`, used by the reconcile to marshal onto the main loop, is
        // already imported below for the transcript idle append.)
        (GIO, "g_application_hold"),
        (GIO, "g_application_release"),
        // plan-62-D reconcile: hide the window on `setMode(None)`.
        (GTK, "gtk_widget_set_visible"),
        (GTK, "gtk_application_new"),
        (GTK, "gtk_application_window_new"),
        (GTK, "gtk_window_set_title"),
        (GTK, "gtk_window_set_default_size"),
        (GTK, "gtk_window_set_child"),
        (GTK, "gtk_window_present"),
        // Scrolling container.
        (GTK, "gtk_scrolled_window_new"),
        (GTK, "gtk_scrolled_window_set_child"),
        // Read-only transcript (GtkTextView + GtkTextBuffer).
        (GTK, "gtk_text_view_new"),
        (GTK, "gtk_text_view_set_editable"),
        (GTK, "gtk_text_view_set_monospace"),
        (GTK, "gtk_text_view_get_buffer"),
        (GTK, "gtk_text_view_scroll_mark_onscreen"),
        (GTK, "gtk_text_buffer_create_mark"),
        (GTK, "gtk_text_buffer_delete_mark"),
        (GTK, "gtk_text_buffer_get_end_iter"),
        (GTK, "gtk_text_buffer_insert"),
        // bug-421: LINE_ECHO Backspace erases the last echoed character by moving a
        // GtkTextIter back one char (code-point granular) and deleting the range.
        (GTK, "gtk_text_iter_backward_char"),
        (GTK, "gtk_text_buffer_delete"),
        // Terminal-style key input captured at the window (no entry box; mirrors
        // the macOS NSTextView keyDown: override). GDK lives in libgtk-4.
        (GTK, "gtk_event_controller_key_new"),
        (GTK, "gtk_widget_add_controller"),
        (GTK, "gdk_keyval_to_unicode"),
        (GLIB, "g_unichar_to_utf8"),
        // term:: TUI surface: a GtkDrawingArea rendered with Cairo (libcairo).
        (GTK, "gtk_drawing_area_new"),
        (GTK, "gtk_drawing_area_set_draw_func"),
        (GTK, "gtk_widget_queue_draw"),
        // plan-98-A Phase 3: read the presented window's native `GdkSurface*` when
        // entering `Mode.Canvas`. Backend-agnostic on purpose — the X11/Wayland
        // getters that turn this into an XID / wl_surface are backend-specific
        // symbols plan-98-F imports, and naming either here would fail to bind on
        // the other display server.
        (GTK, "gtk_native_get_surface"),
        // plan-98-C Phase 3: the CPU-buffer blit. Cairo rather than a `GdkTexture`
        // because the surface plan-98-A built is a `GtkDrawingArea`, whose draw
        // callback hands out a `cairo_t` — displaying a texture through it would
        // mean downloading the texture back to CPU memory it just came from.
        (GTK, "gtk_drawing_area_set_draw_func"),
        (GTK, "gtk_widget_queue_draw"),
        (CAIRO, "cairo_image_surface_create_for_data"),
        (CAIRO, "cairo_set_source_surface"),
        (CAIRO, "cairo_surface_destroy"),
        (GOBJECT, "g_object_ref_sink"),
        (CAIRO, "cairo_set_source_rgb"),
        (CAIRO, "cairo_paint"),
        (CAIRO, "cairo_rectangle"),
        (CAIRO, "cairo_fill"),
        (CAIRO, "cairo_move_to"),
        // plan-70-E: Pango draws each cell's grapheme with font fallback (CJK/emoji)
        // that the Cairo toy API lacks; the layout is created once per draw/measure
        // and reused per cell (set_text + show_layout). This replaced the Cairo toy
        // font API (select_font_face / show_text / font_extents / text_extents).
        (PANGOCAIRO, "pango_cairo_create_layout"),
        (PANGOCAIRO, "pango_cairo_show_layout"),
        (PANGO, "pango_layout_set_text"),
        (PANGO, "pango_layout_set_font_description"),
        (PANGO, "pango_layout_get_pixel_extents"),
        (PANGO, "pango_font_description_from_string"),
        (PANGO, "pango_font_description_set_weight"),
        (PANGO, "pango_font_description_free"),
        (GOBJECT, "g_object_unref"),
        (CAIRO, "cairo_image_surface_create"),
        (CAIRO, "cairo_create"),
        (CAIRO, "cairo_destroy"),
        (CAIRO, "cairo_surface_destroy"),
        // GObject signal wiring (non-variadic form; §6.4) + main-thread marshal.
        (GOBJECT, "g_signal_connect_data"),
        (GLIB, "g_idle_add"),
        // The worker thread and the window-input pipe come from libc/libpthread,
        // exactly as the console runtime resolves them on glibc.
        (libpthread, "pthread_create"),
        (libpthread, "pthread_detach"),
        // `__libc_start_main` runs the C runtime + shared-library constructors
        // (the GLib/GObject type system) before calling our real `main`; the
        // entry can't link crt1.o, so it calls this directly (plan-05 §6.1).
        (libc, "__libc_start_main"),
        (libc, "pipe"),
        (libc, "dup2"),
        // The activate handler dup2's the pipe read end onto fd 0, then closes
        // the redundant original descriptor so stdin EOF works (bug-59).
        (libc, "close"),
        (libc, "setenv"),
        // plan-98-F: the headless gate reads its env name. (`pause`, which parks the
        // main thread afterwards, is already declared below for the existing park.)
        (libc, "getenv"),
        (libc, "write"),
        // The activate handler sets the pipe write end O_NONBLOCK so a full pipe
        // makes the key handler's write() return EAGAIN instead of blocking the
        // GTK main thread (bug-114).
        (libc, "fcntl"),
        // Output marshaling to the GTK main thread + the worker park-on-finish.
        (libc, "malloc"),
        (libc, "free"),
        (libc, "memcpy"),
        (libc, "memset"),
        (libc, "memmove"),
        (libc, "pause"),
        // The finish helper's hard-exit fallback. The x86-64 console exit is a
        // raw `exit_group` syscall, so unlike aarch64 nothing else declares it.
        (libc, "_exit"),
        // The app `io::input` helper delegates to the console readLine body
        // (reading the fd-0 window pipe), which imports the terminal probes —
        // no-ops on a pipe (isatty(0) = 0 skips the termios calls), but the
        // symbols must bind. The plan's per-call rows only declare them for a
        // program that calls io.readLine directly.
        (libc, "read"),
        (libc, "isatty"),
        (libc, "tcgetattr"),
        (libc, "tcsetattr"),
    ];
    gtk.iter()
        .map(|(library, symbol)| PlatformImport {
            library: (*library).to_string(),
            symbol: (*symbol).to_string(),
            required_by: "_main".to_string(),
        })
        .collect()
}

/// Read-only C-string data symbols + the writable runtime-state global.
pub(crate) fn app_mode_data_objects(project_name: &str) -> Vec<CodeDataObject> {
    let app_id = gtk_app_id(project_name);
    let mut objects: Vec<CodeDataObject> = [
        (SYM_APP_ID, app_id.as_str()),
        // The window title is the project name, matching the `.desktop` `Name=`
        // (plan-51-A §4.3) and the macOS `CFBundleName`.
        (SYM_TITLE, project_name),
        STR_ACTIVATE,
        STR_CLOSE_REQUEST,
        STR_KEY_PRESSED,
        STR_RESIZE,
        STR_EXIT_PREFIX,
        STR_STDERR_PREFIX,
        STR_MONO_DESC,
        STR_M,
        STR_ENV_A11Y,
        STR_ENV_IM,
        STR_ENV_NONE,
        // plan-98-F: the headless gate's env name.
        STR_HEADLESS_ENV,
    ]
    .iter()
    .map(|(symbol, text)| CodeDataObject {
        symbol: (*symbol).to_string(),
        kind: "raw".to_string(),
        layout: "C string (NUL-terminated)".to_string(),
        align: 1,
        size: text.len() + 1,
        value: hex_cstring(text),
    })
    .collect();
    objects.push(CodeDataObject {
        symbol: STATE_SYMBOL.to_string(),
        kind: "raw".to_string(),
        layout: "mfb.runtime.gtkapp_state.v1 { u64 handles[7]; u64 argc; u64 argv; u64 mode; \
                 u64 lineLen; u8 lineBuf[] }"
            .to_string(),
        align: 8,
        size: STATE_SIZE,
        value: "00".repeat(STATE_SIZE),
    });
    objects
}

fn hex_cstring(text: &str) -> String {
    let mut hex = String::new();
    for byte in text.bytes() {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex.push_str("00");
    hex
}

/// The GTK/GApplication id for `project_name` (plan-51-A §4.5), matching the
/// macOS `CFBundleIdentifier` (`src/os/macos/link/mod.rs:app_info_plist`).
///
/// The name is sanitized to `[A-Za-z0-9_]` with a `_` prefix ahead of a leading
/// digit. `g_application_new` does not tolerate an invalid id: it emits a
/// `g_critical` and the app dies before its first frame, with nothing at build
/// time to catch it. The accepted set here is deliberately narrower than
/// `g_application_id_is_valid` accepts — it is also valid under the stricter
/// `g_dbus_is_name`, so the id works as a bus name too, and a project named
/// `my-app` yields `dev.mfbasic.my_app` rather than a runtime abort.
///
/// The `.desktop` `StartupWMClass` (plan-51-A §4.3) must equal this exactly: GTK4
/// sets the window's `WM_CLASS` from the application id, and a mismatch makes the
/// desktop's launcher-to-window association silently fail.
pub(crate) fn gtk_app_id(project_name: &str) -> String {
    let mut sanitized = String::with_capacity(project_name.len() + 1);
    for ch in project_name.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            sanitized.push(ch);
        } else {
            // Every other byte — `-`, `.`, a space, or any non-ASCII scalar —
            // becomes `_`. Collapsing rather than dropping keeps two distinct
            // project names from colliding on one id.
            sanitized.push('_');
        }
    }
    // A GApplication id element may not start with a digit, and an empty element
    // is invalid outright.
    if sanitized.is_empty() {
        sanitized.push('_');
    } else if sanitized.starts_with(|ch: char| ch.is_ascii_digit()) {
        sanitized.insert(0, '_');
    }
    format!("dev.mfbasic.{sanitized}")
}

#[cfg(test)]
mod identity_tests {
    use super::*;

    #[test]
    fn gtk_app_id_passes_a_plain_name_through() {
        assert_eq!(gtk_app_id("hello"), "dev.mfbasic.hello");
        assert_eq!(gtk_app_id("my_app2"), "dev.mfbasic.my_app2");
    }

    #[test]
    fn gtk_app_id_replaces_every_invalid_character() {
        // A hyphen is legal in a project name and illegal in a bus-name element.
        assert_eq!(gtk_app_id("my-app"), "dev.mfbasic.my_app");
        // A dot would introduce a new element, changing the id's shape.
        assert_eq!(gtk_app_id("my.app"), "dev.mfbasic.my_app");
        assert_eq!(gtk_app_id("my app"), "dev.mfbasic.my_app");
        assert_eq!(gtk_app_id("café"), "dev.mfbasic.caf_");
    }

    #[test]
    fn gtk_app_id_prefixes_a_leading_digit() {
        assert_eq!(gtk_app_id("3d"), "dev.mfbasic._3d");
        assert_eq!(gtk_app_id("2048"), "dev.mfbasic._2048");
    }

    #[test]
    fn gtk_app_id_never_produces_an_empty_element() {
        assert_eq!(gtk_app_id(""), "dev.mfbasic._");
    }

    #[test]
    fn gtk_app_id_output_is_valid_under_g_dbus_is_name() {
        // The conservative set the doc comment promises: every element non-empty,
        // `[A-Za-z_][A-Za-z0-9_]*`, at least two elements, no leading digit.
        for name in ["hello", "my-app", "3d", "", "café", "a.b.c", "x  y"] {
            let id = gtk_app_id(name);
            let elements: Vec<&str> = id.split('.').collect();
            assert!(elements.len() >= 2, "{id}: needs at least two elements");
            for element in elements {
                assert!(!element.is_empty(), "{id}: empty element");
                assert!(
                    !element.starts_with(|ch: char| ch.is_ascii_digit()),
                    "{id}: element starts with a digit"
                );
                assert!(
                    element
                        .chars()
                        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_'),
                    "{id}: element has an invalid character"
                );
            }
        }
    }

    #[test]
    fn app_mode_data_objects_carry_the_derived_id_and_title() {
        let objects = app_mode_data_objects("my-app");
        let id = objects
            .iter()
            .find(|object| object.symbol == SYM_APP_ID)
            .expect("app id object");
        assert_eq!(id.value, hex_cstring("dev.mfbasic.my_app"));
        assert_eq!(id.size, "dev.mfbasic.my_app".len() + 1);
        let title = objects
            .iter()
            .find(|object| object.symbol == SYM_TITLE)
            .expect("title object");
        assert_eq!(title.value, hex_cstring("my-app"), "title is the raw name");
        // The pre-plan-51 constants must not survive anywhere in the data.
        let dead = hex_cstring("dev.mfbasic.app");
        assert!(
            objects.iter().all(|object| object.value != dead),
            "the shared `dev.mfbasic.app` id must be gone"
        );
    }
}

#[cfg(test)]
mod import_tests {
    use super::*;

    /// The glibc names, matching what the constants held before plan-56-A.
    const GLIBC_NAMES: AppLibcNames = AppLibcNames {
        libc: "libc.so.6",
        libpthread: "libpthread.so.0",
    };
    /// The musl names for x86_64; aarch64 differs only in the arch token.
    const MUSL_X86_NAMES: AppLibcNames = AppLibcNames {
        libc: "libc.musl-x86_64.so.1",
        libpthread: "libc.musl-x86_64.so.1",
    };

    /// plan-56-A §4.1: the glibc list is exactly what the hardcoded constants
    /// produced, so threading the names moves no byte.
    #[test]
    fn app_mode_imports_glibc_is_unchanged() {
        let libs: Vec<String> = app_mode_imports(GLIBC_NAMES)
            .into_iter()
            .map(|import| import.library)
            .collect();
        assert!(libs.iter().any(|l| l == "libc.so.6"));
        assert!(libs.iter().any(|l| l == "libpthread.so.0"));
        assert!(libs.iter().any(|l| l == "libgtk-4.so.1"));
        assert!(
            !libs.iter().any(|l| l.starts_with("libc.musl-")),
            "a glibc build must name no musl library"
        );
    }

    /// ⚠️ The assertion plan-56 exists for. A musl binary that *also* declares
    /// the glibc names runs fine — musl's loader absorbs `libc.so.6` and
    /// `libpthread.so.0` into itself (verified on stock Alpine x86_64 and
    /// aarch64 with gcompat absent, plan-56-A §2.4) — so no runtime signal can
    /// catch this. Assert on the ABSENCE of every glibc name, not merely the
    /// presence of the musl one.
    #[test]
    fn app_mode_imports_musl_names_no_glibc_library() {
        for (names, expected_libc) in [
            (MUSL_X86_NAMES, "libc.musl-x86_64.so.1"),
            (
                AppLibcNames {
                    libc: "libc.musl-aarch64.so.1",
                    libpthread: "libc.musl-aarch64.so.1",
                },
                "libc.musl-aarch64.so.1",
            ),
        ] {
            let imports = app_mode_imports(names);
            let libs: Vec<&str> = imports.iter().map(|i| i.library.as_str()).collect();
            for glibc_only in [
                "libc.so.6",
                "libpthread.so.0",
                "libdl.so.2",
                "librt.so.1",
                "libm.so.6",
            ] {
                assert!(
                    !libs.contains(&glibc_only),
                    "{expected_libc}: musl build must not declare {glibc_only}"
                );
            }
            assert!(
                libs.contains(&expected_libc),
                "musl build must declare {expected_libc}"
            );
            // pthread lives in libc on musl, so the pthread symbols follow it.
            for import in &imports {
                if import.symbol.starts_with("pthread_") {
                    assert_eq!(
                        import.library, expected_libc,
                        "{} must bind to the musl libc",
                        import.symbol
                    );
                }
            }
            // The toolkit libraries are libc-world-independent.
            assert!(libs.contains(&"libgtk-4.so.1"));
            assert!(libs.contains(&"libcairo.so.2"));
        }
    }

    /// The import *set* is flavor-independent — only attribution changes.
    #[test]
    fn app_mode_imports_symbol_set_is_flavor_independent() {
        let glibc: Vec<String> = app_mode_imports(GLIBC_NAMES)
            .into_iter()
            .map(|i| i.symbol)
            .collect();
        let musl: Vec<String> = app_mode_imports(MUSL_X86_NAMES)
            .into_iter()
            .map(|i| i.symbol)
            .collect();
        assert_eq!(glibc, musl);
    }

    /// bug-59: the GTK backend never calls `getenv`, so it must not be declared as
    /// an import; and the activate handler now calls `close`, which must be. Guard
    /// the import plan against reintroducing the dead symbol or dropping `close`.
    #[test]
    fn app_mode_imports_drop_getenv_add_close() {
        let symbols: Vec<String> = app_mode_imports(GLIBC_NAMES)
            .into_iter()
            .map(|import| import.symbol)
            .collect();
        // bug-59 removed `getenv` as a dead import and pinned its absence here.
        // plan-98-F made it live again: `emit_main_bootstrap` reads
        // `MFB_GTKAPP_HEADLESS` with it. The assertion is inverted rather than
        // deleted, because the property bug-59 cared about — no import without a
        // caller — is still the one worth holding, and the companion test
        // (`every_emitted_external_call_is_a_declared_import`) is what pins the
        // caller's existence.
        assert!(
            symbols.iter().any(|s| s == "getenv"),
            "the headless gate reads MFB_GTKAPP_HEADLESS, so `getenv` must be imported"
        );
        assert!(
            symbols.iter().any(|s| s == "close"),
            "the activate handler closes the redundant read fd, so `close` must be imported"
        );
        // The genuinely-used libc env call must remain.
        assert!(
            symbols.iter().any(|s| s == "setenv"),
            "setenv is still used"
        );
    }

    /// plan-56-A §4.2: `lib_for`'s job — every symbol the emitters reference is
    /// declared in `app_mode_imports` — is now enforced by
    /// `shared::crate::codegen::engine::builder::bind_deferred_relocation_libraries`, which errors on an
    /// undeclared symbol. Pin the half that lives here: the symbols the emitters
    /// actually call must all be in the import list, so the binding cannot fail
    /// at build time.
    #[test]
    fn every_emitted_external_call_is_a_declared_import() {
        let declared: std::collections::HashSet<String> = app_mode_imports(GLIBC_NAMES)
            .into_iter()
            .map(|import| import.symbol)
            .collect();
        // The symbols the old `lib_for` table enumerated by hand, plus the two
        // whose absence bug-59 pins.
        for symbol in [
            "close",
            "pipe",
            "dup2",
            "setenv",
            "write",
            "fcntl",
            "malloc",
            "free",
            "memcpy",
            "memset",
            "memmove",
            "pause",
            // plan-98-F: the MFB_GTKAPP_HEADLESS gate.
            "getenv",
            "_exit",
            "__libc_start_main",
            "pthread_create",
            "pthread_detach",
            "g_idle_add",
            "g_application_run",
            "gtk_window_present",
            // bug-421: the LINE_ECHO Backspace transcript-erase helper.
            "gtk_text_iter_backward_char",
            "gtk_text_buffer_delete",
        ] {
            assert!(
                declared.contains(symbol),
                "{symbol} is emitted but not declared in app_mode_imports, so \
                 relocation binding would fail the build"
            );
        }
        // Was `!declared.contains("getenv")` (bug-59, when nothing called it). The
        // headless gate does now, and it is in the emitted-call list above.
        assert!(
            declared.contains("getenv"),
            "the headless gate emits a getenv call, so it must be declared"
        );
    }
}

#[cfg(test)]
/// plan-98-A Phase 3: the `Mode.Canvas` arm of the GTK reconcile.
///
/// Structural, not behavioral: the dev/CI host is macOS and cannot execute a
/// Linux + GTK binary, so — exactly as the rest of this file's coverage does —
/// these inspect the emitted code rather than running it.
mod canvas_reconcile_tests {
    use super::*;

    fn externals(func: &CodeFunction, name: &str) -> usize {
        func.relocations
            .iter()
            .filter(|r| r.to.as_str() == name)
            .count()
    }

    /// The compare immediates the reconcile dispatch tests, in emitted order.
    fn compare_immediates(func: &CodeFunction) -> Vec<String> {
        func.instructions
            .iter()
            .filter(|i| i.op == crate::arch::ops::CodeOp::CmpImm)
            .filter_map(|i| {
                i.fields
                    .iter()
                    .find(|(name, _)| *name == "rhs")
                    .map(|(_, value)| value.to_string())
            })
            .collect()
    }

    /// `Canvas` (2) must be dispatched before the `Console`-or-not test. With a
    /// third variant "not Console" no longer implies `None`, so the old two-way
    /// shape would take the None arm for Canvas and hide the window the instant a
    /// program entered canvas mode.
    #[test]
    fn reconcile_dispatches_canvas_before_the_console_test() {
        let func = bootstrap::emit_reconcile_idle_helper(true).expect("reconcile idle");
        let immediates = compare_immediates(&func);
        let canvas = immediates
            .iter()
            .position(|value| value == "2")
            .expect("reconcile must test for the Canvas discriminant");
        let console = immediates
            .iter()
            .position(|value| value == "0")
            .expect("reconcile must test for the Console discriminant");
        assert!(
            canvas < console,
            "Canvas (2) must be dispatched before the Console/not-Console test, \
             else Canvas falls into the None arm; got {immediates:?}"
        );
    }

    /// The canvas surface is a `GtkDrawingArea` `g_object_ref_sink`ed like the
    /// transcript's scrolled window, so `gtk_window_set_child` swapping it away on
    /// mode exit unparents rather than destroys it — enter → exit → re-enter reuses
    /// the one widget instead of leaking a new one per cycle.
    #[test]
    fn canvas_area_is_created_and_ref_sunk() {
        let func = bootstrap::emit_reconcile_idle_helper(true).expect("reconcile idle");
        assert_eq!(
            externals(&func, "gtk_drawing_area_new"),
            1,
            "the Canvas arm must create the drawing area that is its surface"
        );
        assert!(
            externals(&func, "g_object_ref_sink") >= 1,
            "the canvas area must be ref_sunk or set_child would destroy it on exit"
        );
    }

    /// The drawing area must get its paint callback when it is created.
    ///
    /// A `GtkDrawingArea` with no draw func renders nothing at all, and the area is
    /// built on entering canvas mode — before any frame exists — so installing the
    /// callback at first present would leave the first exposes blank.
    #[test]
    fn canvas_area_gets_its_draw_func_when_created() {
        let func = bootstrap::emit_reconcile_idle_helper(true).expect("reconcile idle");
        let order: Vec<&str> = func
            .relocations
            .iter()
            .map(|r| r.to.as_str())
            .filter(|name| {
                *name == "gtk_drawing_area_new" || *name == "gtk_drawing_area_set_draw_func"
            })
            .collect();
        let created = order
            .iter()
            .position(|name| *name == "gtk_drawing_area_new")
            .expect("the Canvas arm must create the drawing area");
        let installed = order
            .iter()
            .position(|name| *name == "gtk_drawing_area_set_draw_func")
            .expect("the drawing area must be given a draw func or it paints nothing");
        assert!(
            created < installed,
            "the draw func must be installed on the area just created; got {order:?}"
        );
    }

    /// The blit copies the frame before handing it over.
    ///
    /// The caller's block belongs to the next frame the moment `canvas::blitSurface`
    /// returns, so a pointer handed to the main loop without a copy would be drawn
    /// after it had been overwritten. `malloc` before `g_idle_add` is what proves the
    /// copy happens on the worker's side of the handoff.
    #[test]
    fn blit_copies_the_frame_before_scheduling_it() {
        let func = bootstrap::emit_canvas_blit_helper().expect("canvas blit");
        let order: Vec<&str> = func
            .relocations
            .iter()
            .map(|r| r.to.as_str())
            .filter(|name| *name == "malloc" || *name == "g_idle_add")
            .collect();
        assert_eq!(
            order,
            vec!["malloc", "g_idle_add"],
            "the frame must be copied into its own block before the main loop is \
             given the pointer; got {order:?}"
        );
    }

    /// Ownership of a frame block passes to the main loop, and only the main loop
    /// frees it.
    ///
    /// This is what makes `ST_CANVAS_PIXELS` safe without a lock. If the *worker*
    /// freed the previous block, it would race the draw callback reading it.
    #[test]
    fn only_the_commit_callback_frees_a_frame() {
        let blit = bootstrap::emit_canvas_blit_helper().expect("canvas blit");
        assert_eq!(
            externals(&blit, "free"),
            0,
            "the worker must not free a frame block — the main loop owns it once \
             g_idle_add has the pointer"
        );
        let commit = bootstrap::emit_canvas_commit_helper().expect("canvas commit");
        assert_eq!(
            externals(&commit, "free"),
            1,
            "the commit callback must free exactly the block it replaces"
        );
        assert_eq!(
            externals(&commit, "gtk_widget_queue_draw"),
            1,
            "committing a frame must queue the redraw that paints it"
        );
    }

    /// The draw callback paints through a Cairo image surface and destroys it.
    ///
    /// Leaking one `cairo_surface_t` per expose would be unbounded — a window
    /// redraws on every resize step and every occlusion change.
    #[test]
    fn canvas_draw_paints_and_releases_its_surface() {
        let func = bootstrap::emit_canvas_draw_helper().expect("canvas draw");
        let order: Vec<&str> = func
            .relocations
            .iter()
            .map(|r| r.to.as_str())
            .filter(|name| name.starts_with("cairo_"))
            .collect();
        assert_eq!(
            order,
            vec![
                "cairo_image_surface_create_for_data",
                "cairo_set_source_surface",
                "cairo_paint",
                "cairo_surface_destroy",
            ],
            "the draw func must create, source, paint and then destroy its surface; \
             got {order:?}"
        );
    }

    /// The native `GdkSurface*` must be read AFTER `gtk_window_present`: an
    /// unrealized window has no surface, so reading it earlier stores null and
    /// plan-98-F would have nothing to build a VkSurfaceKHR from.
    #[test]
    fn native_surface_is_read_after_present() {
        let func = bootstrap::emit_reconcile_idle_helper(true).expect("reconcile idle");
        let order: Vec<&str> = func
            .relocations
            .iter()
            .map(|r| r.to.as_str())
            .filter(|name| *name == "gtk_window_present" || *name == "gtk_native_get_surface")
            .collect();
        let surface = order
            .iter()
            .position(|name| *name == "gtk_native_get_surface")
            .expect("the Canvas arm must read the native surface handle");
        let present = order
            .iter()
            .position(|name| *name == "gtk_window_present")
            .expect("the reconcile must present the window");
        assert!(
            present < surface,
            "gtk_native_get_surface must follow gtk_window_present; got {order:?}"
        );
    }

    /// The window build must be a shared helper both arms call, not code inlined
    /// into the `Console` arm: a canvas-first program has never presented a
    /// surface, so `ST_WINDOW` is null when it enters canvas mode.
    #[test]
    fn window_build_is_shared_by_the_console_and_canvas_arms() {
        let func = bootstrap::emit_reconcile_idle_helper(true).expect("reconcile idle");
        assert_eq!(
            externals(&func, RECONCILE_BUILD_SYMBOL),
            2,
            "Console and Canvas must each be able to build the window — a \
             canvas-first program has never presented a surface"
        );
        assert_eq!(
            externals(&func, "gtk_application_window_new"),
            0,
            "the window build must live in the shared helper, not be inlined into \
             an arm where the other arm cannot reach it"
        );
    }

    /// Both non-canvas arms must run the canvas teardown, so leaving canvas by
    /// either route restores the transcript child rather than leaving the canvas
    /// area installed on a re-shown transcript window or a hidden one.
    ///
    /// Counted through `gtk_window_set_child`, the teardown's observable call: the
    /// idle helper's three are the Canvas arm installing the canvas area plus one
    /// per teardown. (The build helper's own `set_child` is in a different
    /// function.) A missing teardown drops this to 2.
    #[test]
    fn both_non_canvas_arms_tear_the_canvas_area_down() {
        let func = bootstrap::emit_reconcile_idle_helper(true).expect("reconcile idle");
        assert_eq!(
            externals(&func, "gtk_window_set_child"),
            3,
            "expected 1 Canvas install + 2 teardowns (Console and None arms)"
        );
    }

    /// Every symbol the arm emits must be declared, or relocation binding fails
    /// the build.
    #[test]
    fn canvas_symbols_are_declared_imports() {
        let declared: Vec<String> = app_mode_imports(AppLibcNames {
            libc: "libc.so.6",
            libpthread: "libpthread.so.0",
        })
        .into_iter()
        .map(|import| import.symbol)
        .collect();
        for symbol in [
            "gtk_native_get_surface",
            "gtk_drawing_area_new",
            "g_object_ref_sink",
        ] {
            assert!(
                declared.iter().any(|s| s == symbol),
                "{symbol} is emitted by the Canvas arm but not declared"
            );
        }
    }

    // --- plan-98-A Phase 4: canvas keyboard input ---

    /// The key controller goes on the **window**, not on any child widget. That
    /// placement is what makes canvas mode inherit keyboard input for free —
    /// `gtk_window_set_child` swapping the transcript for the canvas area does not
    /// disturb a controller attached to the window itself. The reconcile-built
    /// window had none at all before this phase, so a `None`-default program's
    /// `io::` reads found nothing to read even after switching to `Console`.
    #[test]
    fn the_reconcile_built_window_gets_the_key_controller() {
        let build = bootstrap::emit_reconcile_build_helper().expect("reconcile build");
        assert_eq!(
            externals(&build, "gtk_event_controller_key_new"),
            1,
            "the reconcile-built window must get a key controller"
        );
        // The two callback addresses are taken with `local_address`, an adrp/add
        // page pair = 2 relocations each; the external calls above are 1 each.
        assert_eq!(
            externals(&build, KEY_PRESSED_SYMBOL),
            2,
            "the controller must be connected to the shared key handler"
        );
        assert_eq!(
            externals(&build, "gtk_widget_add_controller"),
            1,
            "the controller must be attached to the window"
        );
        assert_eq!(
            externals(&build, WINDOW_CLOSED_SYMBOL),
            2,
            "closing a reconcile-built window must end the program, as closing a \
             startup-built one does"
        );
    }

    /// The window input pipe must be wired for a `None`-default program too. A
    /// `None` default means the program references `app::setMode`, so it is the only
    /// kind that can ever reach `Console` or `Canvas` — and in both the window is
    /// the input source. It must happen at startup, before the worker spawns:
    /// `dup2`ing onto fd 0 after the worker has blocked in `read(0, …)` leaves that
    /// read waiting on the old file description forever.
    #[test]
    fn the_input_pipe_is_wired_for_a_none_default_program() {
        let none = bootstrap::emit_activate_handler(PresentationMode::None).expect("activate");
        assert_eq!(
            externals(&none, "pipe"),
            1,
            "a None-default activate must wire the window input pipe"
        );
        assert_eq!(
            externals(&none, "dup2"),
            1,
            "the pipe read end must become fd 0 before the worker spawns"
        );
        // The Console-default path keeps exactly one — the extraction must not have
        // duplicated it into the branch it came from.
        let console =
            bootstrap::emit_activate_handler(PresentationMode::Console).expect("activate");
        assert_eq!(externals(&console, "pipe"), 1);
    }
}
