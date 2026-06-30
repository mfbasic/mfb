//! Linux GTK4 app-mode codegen (plan-05-linux-app.md Phases 3-6).
//!
//! This is the Linux counterpart of `macos_aarch64/app.rs`. It emits the GTK4
//! `_main` bootstrap, the language worker thread, the transcript/input widgets,
//! and the app-mode `io::*` helper bodies. Every GTK/GObject/GLib/GIO call is an
//! ordinary imported C function (no `objc_msgSend` layer), so the emitted code is
//! plain `bl <symbol>` against the imports declared in
//! `linux_aarch64/plan.rs::app_mode_imports`.
//!
//! SCAFFOLD STATUS (plan-05 Phase 3): the structure below mirrors the macOS
//! backend and is code-plan-valid + ELF-encodable, but it has **not** been run on
//! a Linux+GTK aarch64 machine (the dev host is macOS, which cannot execute the
//! produced ELF). Several runtime-bound behaviors are intentionally simplified and
//! marked `TODO(plan-05)`:
//!   * output `io::print`/`io::write` append to the `GtkTextBuffer` directly from
//!     the worker thread; the main-thread marshal (`g_idle_add` / condvar) that
//!     §6.4 requires is not yet wired, and the fd fallback (write to stdout/stderr)
//!     is the only path exercised when no buffer is attached.
//!   * `io::printError` is not yet visually distinguished with a `GtkTextTag`.
//!   * the finish path hard-exits via `_exit` instead of keeping the window open
//!     (§6.7) and `io::terminalSize` / interactive resize (§6, Phase 6) are absent.
//! These are completed-and-verified on-device in a later pass; until then the
//! glibc executable links GTK and is structurally complete but unverified.

use std::collections::HashMap;

use crate::arch::aarch64::abi;
use crate::target::shared::code::{
    self, AppEntrySpec, CodeDataObject, CodeFrame, CodeFunction, CodeInstruction, CodeRelocation, RelocIntent,
};

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
/// Main-thread idle callback that drains one marshaled output chunk into the
/// transcript (scheduled from the worker thread via `g_idle_add`, plan-05 §6.4).
const APPEND_IDLE_SYMBOL: &str = "_mfb_gtkapp_append_idle";
/// Worker program-completion handler (referenced by `emit_program_exit`).
pub(crate) const FINISH_SYMBOL: &str = "_mfb_gtkapp_finish";

/// Writable runtime-state global. One pointer/handle per slot; the GTK widgets
/// and the window-input pipe fds live here so every helper can reach them without
/// register preservation (plan-05-linux-app.md §6.2, simplified for the scaffold).
const STATE_SYMBOL: &str = "_mfb_gtkapp_state";
const ST_APPLICATION: usize = 0;
const ST_WINDOW: usize = 8;
const ST_SCROLLED: usize = 16;
const ST_TEXT_VIEW: usize = 24;
const ST_TEXT_BUFFER: usize = 32;
const ST_PIPE_READ_FD: usize = 40;
const ST_PIPE_WRITE_FD: usize = 48;
/// Current input mode (see `MODE_*`): selects echo / raw key handling, exactly like
/// the macOS `INPUT_MODE_*` associated object.
const ST_INPUT_MODE: usize = 56;
/// Length of the pending (uncommitted) input line in `ST_LINE_BUF`.
const ST_LINE_LEN: usize = 64;
/// Accumulated bytes of the line being typed (committed to the pipe on Enter).
const ST_LINE_BUF: usize = 72;
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
                                                  // Parallel per-cell grids: chars (1B), fg (u32 packed | flags), bg (u32 packed).
                                                  // Row stride is TERM_MAX_COLS; only the top-left cols x rows are active.
const ST_TERM_CHARS: usize = ST_TERM_CELL_H + 8;
const ST_TERM_FG: usize = ST_TERM_CHARS + TERM_MAX_COLS * TERM_MAX_ROWS;
const ST_TERM_BG: usize = ST_TERM_FG + TERM_MAX_COLS * TERM_MAX_ROWS * 4;
const STATE_SIZE: usize = ST_TERM_BG + TERM_MAX_COLS * TERM_MAX_ROWS * 4;

// fg/bg cell encoding: low 24 bits = packed RGB (r|g<<8|b<<16, the console
// convention so the arena getters agree); bit 24 marks an explicit color (so 0 =
// "use default", letting black be set distinctly); bit 25 (fg) = bold, bit 26 (fg)
// = underline.
const COLOR_SET: usize = 1 << 24;
const BOLD_FLAG: usize = 1 << 25;
const UNDERLINE_FLAG: usize = 1 << 26;
const TERM_DEFAULT_FG: &str = "16777215"; // 0xFFFFFF white (matches console default)

// Backing-store bounds for the grid (a fixed stride keeps storage static). The
// active cols/rows are derived from the window size and font cell metrics and never
// exceed these.
const TERM_MAX_COLS: usize = 160;
const TERM_MAX_ROWS: usize = 48;
const TERM_FONT_SIZE: &str = "16";
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
/// Pinned arena-state base register (term helpers run on the worker thread, where
/// x19 holds the arena base; the shared console term-state lives at tso + field).
const ARENA_REG: &str = "x19";

// Input modes (mirror macOS app.rs INPUT_MODE_*): line-buffered without echo is the
// default (`io::readLine`), line-buffered with echo is `io::input`, and raw delivers
// each keystroke's bytes to the pipe immediately (`io::readChar`/`readByte`).
/// Default mode: line-buffered, no echo (the zero-initialized state value).
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

const STR_APP_ID: (&str, &str) = ("_mfb_gtkapp_str_app_id", "dev.mfbasic.app");
const STR_TITLE: (&str, &str) = ("_mfb_gtkapp_str_title", "MFBASIC App");
const STR_ACTIVATE: (&str, &str) = ("_mfb_gtkapp_str_activate", "activate");
const STR_CLOSE_REQUEST: (&str, &str) = ("_mfb_gtkapp_str_close_request", "close-request");
const STR_KEY_PRESSED: (&str, &str) = ("_mfb_gtkapp_str_key_pressed", "key-pressed");
/// Completion status line appended to the transcript when the program ends
/// (matches macOS app.rs STR_EXIT_PREFIX): leading newline + "...code " + N + "\n".
const STR_EXIT_PREFIX: (&str, &str) =
    ("_mfb_gtkapp_str_exit_prefix", "\nProgram exited with code ");
/// Marker prepended to `printError`/`writeError` transcript runs (matches macOS
/// app.rs STR_STDERR_PREFIX), visually distinguishing stderr (plan-05 §5.4).
const STR_STDERR_PREFIX: (&str, &str) = ("_mfb_gtkapp_str_stderr_prefix", "[stderr] ");
/// Cairo font family for the term:: grid.
const STR_MONOSPACE: (&str, &str) = ("_mfb_gtkapp_str_monospace", "monospace");
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

// --- Library names (app mode is glibc-only, plan-05 §1.1) -------------------

const GTK: &str = "libgtk-4.so.1";
const GOBJECT: &str = "libgobject-2.0.so.0";
const GLIB: &str = "libglib-2.0.so.0";
const GIO: &str = "libgio-2.0.so.0";
const LIBC: &str = "libc.so.6";
const LIBPTHREAD: &str = "libpthread.so.0";
const CAIRO: &str = "libcairo.so.2";

/// Library that exports `symbol`, matching `app_mode_imports`. The relocation's
/// library field is cosmetic (the linker binds by symbol name), but keeping it
/// accurate aids artifact debugging.
fn lib_for(symbol: &str) -> &'static str {
    match symbol {
        "g_application_run" | "g_application_quit" => GIO,
        "g_signal_connect_data" => GOBJECT,
        "g_idle_add" => GLIB,
        "pthread_create" | "pthread_detach" => LIBPTHREAD,
        "pipe" | "dup2" | "getenv" | "setenv" | "write" | "_exit" | "__libc_start_main"
        | "malloc" | "free" | "memcpy" | "memset" | "memmove" | "pause" => LIBC,
        // GDK is part of libgtk-4.so.1 in GTK4 (no separate libgdk).
        "gdk_keyval_to_unicode" => GTK,
        "g_object_ref_sink" => GOBJECT,
        sym if sym.starts_with("cairo_") => CAIRO,
        sym if sym.starts_with("gtk_") => GTK,
        sym if sym.starts_with("g_") => GLIB,
        other => panic!("linux app-mode codegen referenced unmapped symbol '{other}'"),
    }
}

// --- Tiny assembler over CodeInstruction/CodeRelocation --------------------

struct Asm {
    from: String,
    ins: Vec<CodeInstruction>,
    rel: Vec<CodeRelocation>,
}

impl Asm {
    fn new(from: &str) -> Self {
        Asm {
            from: from.to_string(),
            ins: Vec::new(),
            rel: Vec::new(),
        }
    }

    fn push(&mut self, instruction: CodeInstruction) {
        self.ins.push(instruction);
    }

    /// `bl <symbol>` to an imported C function.
    fn call_external(&mut self, symbol: &str) {
        self.ins.push(abi::branch_link(symbol));
        self.rel.push(CodeRelocation {
            from: self.from.clone(),
            to: symbol.to_string(),
            kind: RelocIntent::Call,
            binding: "external".to_string(),
            library: Some(lib_for(symbol).to_string()),
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
    fn local_address(&mut self, dst: &str, symbol: &str) {
        self.push(
            CodeInstruction::new("adrp")
                .field("dst", dst)
                .field("symbol", symbol),
        );
        self.push(
            CodeInstruction::new("add_pageoff")
                .field("dst", dst)
                .field("src", dst)
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
    /// `dst` (clobbers `x9` for large offsets past the 12-bit add immediate).
    fn state_array(&mut self, dst: &str, offset: usize) {
        self.local_address(dst, STATE_SYMBOL);
        if offset < 4096 {
            self.push(abi::add_immediate(dst, dst, offset));
        } else {
            self.push(abi::move_immediate("x9", "Integer", &offset.to_string()));
            self.push(abi::add_registers(dst, dst, "x9"));
        }
    }

    /// Load runtime-state field `offset` into `dst` (clobbers `x9`).
    fn load_state(&mut self, dst: &str, offset: usize) {
        self.local_address("x9", STATE_SYMBOL);
        self.push(abi::load_u64(dst, "x9", offset));
    }

    /// Store `src` into runtime-state field `offset` (clobbers `x9`).
    fn store_state(&mut self, src: &str, offset: usize) {
        self.local_address("x9", STATE_SYMBOL);
        self.push(abi::store_u64(src, "x9", offset));
    }

    fn finish(self, symbol: &str, returns: &str) -> CodeFunction {
        CodeFunction {
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
        }
    }
}

// --- Bootstrap + UI + worker -----------------------------------------------

/// Emit the GTK4 `_main` bootstrap and supporting functions. The standard program
/// entry runs separately on the worker thread under [`code::MACAPP_PROGRAM_SYMBOL`].
pub(crate) fn emit_app_program_entry(
    spec: &AppEntrySpec,
    _platform_imports: &HashMap<String, String>,
) -> Result<Vec<CodeFunction>, String> {
    Ok(vec![
        emit_libc_start_trampoline(),
        emit_main_bootstrap(),
        emit_activate_handler(),
        emit_worker_shim(spec),
        emit_key_pressed_handler(),
        emit_window_closed_handler(),
        emit_finish_helper(),
        emit_append_helper(),
        emit_append_idle_helper(),
        // term:: TUI surface support (plan-01-term.md §6.3).
        emit_term_draw_helper(),
        emit_term_show_idle_helper(),
        emit_term_hide_idle_helper(),
        emit_term_redraw_idle_helper(),
        emit_term_write_helper(),
        emit_term_scroll_helper(),
        emit_term_init_helper(),
    ])
}


mod app_io;
mod bootstrap;
mod term_draw;

pub(crate) use app_io::*;
use bootstrap::*;
use term_draw::*;

/// Read-only C-string data symbols + the writable runtime-state global.
pub(crate) fn app_mode_data_objects() -> Vec<CodeDataObject> {
    let mut objects: Vec<CodeDataObject> = [
        STR_APP_ID,
        STR_TITLE,
        STR_ACTIVATE,
        STR_CLOSE_REQUEST,
        STR_KEY_PRESSED,
        STR_EXIT_PREFIX,
        STR_STDERR_PREFIX,
        STR_MONOSPACE,
        STR_M,
        STR_ENV_A11Y,
        STR_ENV_IM,
        STR_ENV_NONE,
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
        layout:
            "mfb.runtime.gtkapp_state.v1 { u64 handles[7]; u64 mode; u64 lineLen; u8 lineBuf[] }"
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
