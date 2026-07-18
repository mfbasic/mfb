//! Linux GTK4 app-mode codegen (plan-05-linux-app.md Phases 3-6).
//!
//! This is the Linux counterpart of `macos_aarch64/app.rs`. It emits the GTK4
//! `_main` bootstrap, the language worker thread, the transcript/input widgets,
//! and the app-mode `io::*` helper bodies. Every GTK/GObject/GLib/GIO call is an
//! ordinary imported C function (no `objc_msgSend` layer), so the emitted code is
//! plain `bl <symbol>` against the imports declared in
//! `linux_aarch64/plan.rs::app_mode_imports`.
//!
//! SCAFFOLD STATUS (plan-05): the structure below mirrors the macOS backend and is
//! code-plan-valid + ELF-encodable. It is exercised on Linux+GTK (Debian/Ubuntu
//! GTK VMs); the notes below describe the **implemented** main-thread contract:
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

use std::collections::HashMap;

use crate::arch::aarch64::abi;
use crate::target::shared::code::{
    self, AppEntrySpec, CodeDataObject, CodeFrame, CodeFunction, CodeInstruction, CodeRelocation,
    RelocIntent,
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
const ST_TERM_CHARS: usize = ST_TERM_CELL_H + 8;
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
const STATE_SIZE: usize = ST_TERM_SNAP_BG + TERM_MAX_COLS * TERM_MAX_ROWS * 4;

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
/// `GtkDrawingArea::resize` handler (plan-35-E): recomputes the active cols/rows from
/// the new allocation + cell metrics so `term::terminalSize` tracks the live window
/// and forces a full redraw. Runs on the GTK main loop.
const TERM_RESIZE_SYMBOL: &str = "_mfb_gtkapp_term_resize";
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
fn lib_for(symbol: &str) -> Result<&'static str, String> {
    Ok(match symbol {
        "g_application_run" | "g_application_quit" => GIO,
        "g_signal_connect_data" => GOBJECT,
        "g_idle_add" => GLIB,
        "pthread_create" | "pthread_detach" => LIBPTHREAD,
        "pipe" | "dup2" | "close" | "setenv" | "write" | "fcntl" | "_exit"
        | "__libc_start_main" | "malloc" | "free" | "memcpy" | "memset" | "memmove" | "pause" => {
            LIBC
        }
        // GDK is part of libgtk-4.so.1 in GTK4 (no separate libgdk).
        "gdk_keyval_to_unicode" => GTK,
        "g_object_ref_sink" => GOBJECT,
        sym if sym.starts_with("cairo_") => CAIRO,
        sym if sym.starts_with("gtk_") => GTK,
        sym if sym.starts_with("g_") => GLIB,
        // bug-176 D: an unmapped symbol is a codegen bug, but surface it as a
        // plan-level error rather than a `panic!` that aborts the process.
        other => {
            return Err(format!(
                "linux app-mode codegen referenced unmapped symbol '{other}'"
            ))
        }
    })
}

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
    fn call_external(&mut self, symbol: &str) {
        // bug-176 D: an unmapped symbol is a codegen bug; record it (first wins) and
        // fall back to libc so codegen can continue, then `finish` returns the error
        // rather than aborting the process with a `panic!`.
        let library = match lib_for(symbol) {
            Ok(library) => library,
            Err(message) => {
                if self.err.is_none() {
                    self.err = Some(message);
                }
                LIBC
            }
        };
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

    /// Store `src` into runtime-state field `offset` (clobbers the first
    /// scratch-pool register, realized `x9`). Spelled with the neutral token
    /// because some callers' sequences are injected into shared helper bodies,
    /// which the plan-34-D stream guard requires to be token-pure.
    fn store_state(&mut self, src: &str, offset: usize) {
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
/// entry runs separately on the worker thread under [`code::MACAPP_PROGRAM_SYMBOL`].
pub(crate) fn emit_app_program_entry(
    spec: &AppEntrySpec,
    _platform_imports: &HashMap<String, String>,
) -> Result<Vec<CodeFunction>, String> {
    Ok(vec![
        emit_libc_start_trampoline()?,
        emit_main_bootstrap()?,
        emit_activate_handler()?,
        emit_worker_shim(spec)?,
        emit_key_pressed_handler()?,
        emit_window_closed_handler()?,
        emit_finish_helper()?,
        emit_append_helper()?,
        emit_append_idle_helper()?,
        // term:: TUI surface support (plan-01-term.md §6.3).
        emit_term_draw_helper()?,
        emit_term_show_idle_helper()?,
        emit_term_hide_idle_helper()?,
        emit_term_redraw_idle_helper()?,
        emit_term_write_helper()?,
        emit_term_scroll_helper()?,
        emit_term_init_helper()?,
        emit_term_resize_helper()?,
    ])
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
        emit_activate_handler()?,
        emit_worker_shim(spec)?,
        emit_key_pressed_handler()?,
        emit_window_closed_handler()?,
        emit_finish_helper()?,
        emit_append_helper()?,
        emit_append_idle_helper()?,
        emit_term_draw_helper()?,
        emit_term_show_idle_helper()?,
        emit_term_hide_idle_helper()?,
        emit_term_redraw_idle_helper()?,
        emit_term_write_helper()?,
        emit_term_scroll_helper()?,
        emit_term_init_helper()?,
        emit_term_resize_helper()?,
    ];
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
    // The x86 selection maps x0..x5 to rdi/rsi/rdx/rcx/r8/r9 at the call.
    asm.local_address("x0", GTK_MAIN_SYMBOL); // main
    asm.push(abi::load_u64("x1", abi::stack_pointer(), 0)); // argc
    asm.push(abi::add_immediate("x2", abi::stack_pointer(), 8)); // argv
    asm.push(abi::move_immediate("x3", "Integer", "0")); // init
    asm.push(abi::move_immediate("x4", "Integer", "0")); // fini
    asm.push(abi::move_immediate("x5", "Integer", "0")); // rtld_fini
                                                         // stack_end = the entry sp, passed as the 7th (stack) argument.
    asm.push(abi::add_immediate("x9", abi::stack_pointer(), 0));
    asm.push(abi::subtract_stack(16));
    asm.push(abi::store_u64("x9", abi::stack_pointer(), 0));
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
    use crate::target::shared::code::{mir, regalloc};

    stage_result_reuse_x86(instructions);

    // Rename the AArch64 scratch/parking registers to per-function vregs (one
    // per distinct register, preserving each def/use chain — the same mapping
    // the retired vregify pass used).
    let is_scratch = |name: &str| -> bool {
        name.strip_prefix('x')
            .and_then(|rest| rest.parse::<u32>().ok())
            .is_some_and(|n| (9..=17).contains(&n) || (20..=28).contains(&n))
    };
    let mut order: Vec<String> = Vec::new();
    for instruction in instructions.iter() {
        for (_, value) in &instruction.fields {
            if is_scratch(value) && !order.contains(value) {
                order.push(value.clone());
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
            if let Some(vreg) = rename.get(value) {
                *value = vreg.clone();
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
        abi::store_u64("rbx", abi::stack_pointer(), 0),
        abi::store_u64("r12", abi::stack_pointer(), 8),
        abi::store_u64("r13", abi::stack_pointer(), 16),
        abi::store_u64("r14", abi::stack_pointer(), 24),
        abi::exclusive_or_registers("r14", "r14", "r14"),
    ];
    instructions.splice(entry_at..entry_at, prologue);
    let mut index = entry_at + 6;
    while index < instructions.len() {
        if instructions[index].op == CodeOp::Ret {
            let epilogue = vec![
                abi::load_u64("rbx", abi::stack_pointer(), 0),
                abi::load_u64("r12", abi::stack_pointer(), 8),
                abi::load_u64("r13", abi::stack_pointer(), 16),
                abi::load_u64("r14", abi::stack_pointer(), 24),
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
    let neutral = mir::lower_to_mir(instructions);
    let backend = mir::active_backend();
    *instructions = backend.select(&neutral);
    let spill_base = inner_frame + X86_WRAP_BYTES;
    let outcome = regalloc::allocate(
        regalloc::RegallocKind::LinearScan,
        instructions,
        &[],
        &[],
        backend.register_model(),
        spill_base,
        &[],
    );
    let spill_bytes = outcome.spill_slots.len() * backend.register_model().spill_slot_bytes();
    // Round to 16 so the bracket keeps the interior alignment parity.
    let spill_bytes = (spill_bytes + 15) & !15;
    if spill_bytes > 0 {
        let sentinel = X86_WRAP_BYTES.to_string();
        let bumped = (X86_WRAP_BYTES + spill_bytes).to_string();
        for instruction in instructions.iter_mut() {
            if matches!(instruction.op, CodeOp::SubSp | CodeOp::AddSp) {
                for (key, value) in instruction.fields.iter_mut() {
                    if *key == "imm" && *value == sentinel {
                        *value = bumped.clone();
                    }
                }
            }
        }
    }
}

/// x86 flavor of an app io/term helper body triple (see
/// [`finalize_x86_app_function`]).
pub(crate) fn wrap_x86_helper(
    triple: (CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>),
) -> (CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>) {
    let (frame, mut instructions, relocations) = triple;
    finalize_x86_app_function(&mut instructions);
    (frame, instructions, relocations)
}

/// Make the AArch64 "call result feeds the next call's first argument" idiom
/// explicit for x86. Hand-built sequences like
/// `bl gtk_scrolled_window_new; bl g_object_ref_sink` rely on the result
/// register doubling as the first argument register — true on AArch64 (both are
/// `x0`), false on SysV x86-64 (`rax` vs `rdi`). Before every call whose `x0`
/// was last defined by a *previous call's return* (no intervening def), insert
/// `mov x0, x0`: the x86 role remap colors the destination as the upcoming
/// call's first argument (`rdi`) and the source as the prior call's result
/// (`rax`), producing exactly the missing `mov rdi, rax`. Conservative at
/// labels (unknown provenance across merges — the hand-built bodies stage
/// explicitly across branches).
fn stage_result_reuse_x86(instructions: &mut Vec<CodeInstruction>) {
    use crate::arch::ops::CodeOp;
    let mut result_live = false;
    let mut index = 0;
    while index < instructions.len() {
        match instructions[index].op {
            CodeOp::Label => result_live = false,
            CodeOp::BranchLink | CodeOp::BranchLinkRegister => {
                if result_live {
                    instructions.insert(index, abi::move_register("x0", "x0"));
                    index += 1;
                }
                result_live = true;
            }
            _ => {
                let defines_x0 = instructions[index]
                    .fields
                    .iter()
                    .any(|(key, value)| *key == "dst" && value == "x0");
                if defines_x0 {
                    result_live = false;
                }
            }
        }
        index += 1;
    }
}

/// The app-mode platform import set, shared by the aarch64 and x86-64 Linux
/// plans (plan-05-linux-app.md §6.4). App mode is glibc-only (§1.1), so the
/// library names are fixed: GTK is plain C and every call is an ordinary
/// imported function; `__libc_start_main` runs the C runtime init before the
/// real `main`; pthread spawns the language worker; the pipe primitives feed
/// window input to the reused fd-0 console readers.
pub(crate) fn app_mode_imports() -> Vec<crate::target::shared::plan::PlatformImport> {
    use crate::target::shared::plan::PlatformImport;
    let gtk: &[(&str, &str)] = &[
        // Application + window lifecycle.
        (GIO, "g_application_run"),
        (GIO, "g_application_quit"),
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
        (GOBJECT, "g_object_ref_sink"),
        (CAIRO, "cairo_set_source_rgb"),
        (CAIRO, "cairo_paint"),
        (CAIRO, "cairo_rectangle"),
        (CAIRO, "cairo_fill"),
        (CAIRO, "cairo_select_font_face"),
        (CAIRO, "cairo_set_font_size"),
        (CAIRO, "cairo_move_to"),
        (CAIRO, "cairo_show_text"),
        // Font-metric measurement at init (sizes the grid from cell extents).
        (CAIRO, "cairo_font_extents"),
        (CAIRO, "cairo_text_extents"),
        (CAIRO, "cairo_image_surface_create"),
        (CAIRO, "cairo_create"),
        (CAIRO, "cairo_destroy"),
        (CAIRO, "cairo_surface_destroy"),
        // GObject signal wiring (non-variadic form; §6.4) + main-thread marshal.
        (GOBJECT, "g_signal_connect_data"),
        (GLIB, "g_idle_add"),
        // The worker thread and the window-input pipe come from libc/libpthread,
        // exactly as the console runtime resolves them on glibc.
        (LIBPTHREAD, "pthread_create"),
        (LIBPTHREAD, "pthread_detach"),
        // `__libc_start_main` runs the C runtime + shared-library constructors
        // (the GLib/GObject type system) before calling our real `main`; the
        // entry can't link crt1.o, so it calls this directly (plan-05 §6.1).
        (LIBC, "__libc_start_main"),
        (LIBC, "pipe"),
        (LIBC, "dup2"),
        // The activate handler dup2's the pipe read end onto fd 0, then closes
        // the redundant original descriptor so stdin EOF works (bug-59).
        (LIBC, "close"),
        (LIBC, "setenv"),
        (LIBC, "write"),
        // The activate handler sets the pipe write end O_NONBLOCK so a full pipe
        // makes the key handler's write() return EAGAIN instead of blocking the
        // GTK main thread (bug-114).
        (LIBC, "fcntl"),
        // Output marshaling to the GTK main thread + the worker park-on-finish.
        (LIBC, "malloc"),
        (LIBC, "free"),
        (LIBC, "memcpy"),
        (LIBC, "memset"),
        (LIBC, "memmove"),
        (LIBC, "pause"),
        // The finish helper's hard-exit fallback. The x86-64 console exit is a
        // raw `exit_group` syscall, so unlike aarch64 nothing else declares it.
        (LIBC, "_exit"),
        // The app `io::input` helper delegates to the console readLine body
        // (reading the fd-0 window pipe), which imports the terminal probes —
        // no-ops on a pipe (isatty(0) = 0 skips the termios calls), but the
        // symbols must bind. The plan's per-call rows only declare them for a
        // program that calls io.readLine directly.
        (LIBC, "read"),
        (LIBC, "isatty"),
        (LIBC, "tcgetattr"),
        (LIBC, "tcsetattr"),
    ];
    gtk.iter()
        .map(|(library, symbol)| PlatformImport {
            library: (*library).to_string(),
            symbol: (*symbol).to_string(),
            required_by: "_main".to_string(),
        })
        .collect()
}

mod app_io;
mod bootstrap;
mod term_draw;

pub(crate) use app_io::*;
use bootstrap::*;
use term_draw::*;

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

    /// bug-59: the GTK backend never calls `getenv`, so it must not be declared as
    /// an import; and the activate handler now calls `close`, which must be. Guard
    /// the import plan against reintroducing the dead symbol or dropping `close`.
    #[test]
    fn app_mode_imports_drop_getenv_add_close() {
        let symbols: Vec<String> = app_mode_imports()
            .into_iter()
            .map(|import| import.symbol)
            .collect();
        assert!(
            !symbols.iter().any(|s| s == "getenv"),
            "getenv is dead in the GTK backend and must not be imported (bug-59)"
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

    /// `lib_for` maps every symbol the backend references; `close` must resolve to
    /// libc, and an unmapped symbol now returns a plan-level `Err` (surfacing any
    /// accidental reintroduction) instead of panicking.
    #[test]
    fn lib_for_maps_close_to_libc() {
        assert_eq!(lib_for("close").unwrap(), LIBC);
        assert!(lib_for("getenv").is_err());
    }
}
