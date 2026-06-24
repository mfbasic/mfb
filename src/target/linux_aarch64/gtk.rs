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
    self, AppEntrySpec, CodeDataObject, CodeFrame, CodeFunction, CodeInstruction, CodeRelocation,
};

// --- Emitted symbols -------------------------------------------------------

const MAIN_SYMBOL: &str = "_main";
const ACTIVATE_SYMBOL: &str = "_mfb_gtkapp_activate";
const WORKER_SYMBOL: &str = "_mfb_gtkapp_worker";
const INPUT_COMMITTED_SYMBOL: &str = "_mfb_gtkapp_input_committed";
const WINDOW_CLOSED_SYMBOL: &str = "_mfb_gtkapp_window_closed";
const APPEND_SYMBOL: &str = "_mfb_gtkapp_append";
/// Worker program-completion handler (referenced by `emit_program_exit`).
pub(crate) const FINISH_SYMBOL: &str = "_mfb_gtkapp_finish";

/// Writable runtime-state global. One pointer/handle per slot; the GTK widgets
/// and the window-input pipe fds live here so every helper can reach them without
/// register preservation (plan-05-linux-app.md §6.2, simplified for the scaffold).
const STATE_SYMBOL: &str = "_mfb_gtkapp_state";
const ST_APPLICATION: usize = 0;
const ST_WINDOW: usize = 8;
const ST_BOX: usize = 16;
const ST_SCROLLED: usize = 24;
const ST_TEXT_VIEW: usize = 32;
const ST_TEXT_BUFFER: usize = 40;
const ST_INPUT_FIELD: usize = 48;
const ST_PIPE_READ_FD: usize = 56;
const ST_PIPE_WRITE_FD: usize = 64;
const STATE_SIZE: usize = 72;

// Reused runtime helper symbols (the console io::write / io::readLine bodies feed
// the transcript prompt + the fd-0 window-input pipe respectively).
const IO_WRITE_SYMBOL: &str = "_mfb_rt_io_io_write";
const IO_READ_LINE_SYMBOL: &str = "_mfb_rt_io_io_readLine";

// --- Read-only string data symbols -----------------------------------------

const STR_APP_ID: (&str, &str) = ("_mfb_gtkapp_str_app_id", "dev.mfbasic.app");
const STR_TITLE: (&str, &str) = ("_mfb_gtkapp_str_title", "MFBASIC App");
const STR_ACTIVATE: (&str, &str) = ("_mfb_gtkapp_str_activate", "activate");
const STR_CLOSE_REQUEST: (&str, &str) = ("_mfb_gtkapp_str_close_request", "close-request");
const STR_EMPTY: (&str, &str) = ("_mfb_gtkapp_str_empty", "");

// --- GTK / GObject enum immediates -----------------------------------------

const G_APPLICATION_DEFAULT_FLAGS: &str = "0";
const GTK_ORIENTATION_VERTICAL: &str = "1";
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

/// Library that exports `symbol`, matching `app_mode_imports`. The relocation's
/// library field is cosmetic (the linker binds by symbol name), but keeping it
/// accurate aids artifact debugging.
fn lib_for(symbol: &str) -> &'static str {
    match symbol {
        "g_application_run" | "g_application_quit" => GIO,
        "g_signal_connect_data" => GOBJECT,
        "g_idle_add" => GLIB,
        "pthread_create" | "pthread_detach" => LIBPTHREAD,
        "pipe" | "dup2" | "getenv" | "write" | "strlen" | "_exit" => LIBC,
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
            kind: "branch26".to_string(),
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
            kind: "branch26".to_string(),
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
        for kind in ["page21", "pageoff12"] {
            self.rel.push(CodeRelocation {
                from: self.from.clone(),
                to: symbol.to_string(),
                kind: kind.to_string(),
                binding: "data".to_string(),
                library: None,
            });
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
        emit_main_bootstrap(),
        emit_activate_handler(),
        emit_worker_shim(spec),
        emit_input_committed_handler(),
        emit_window_closed_handler(),
        emit_finish_helper(),
        emit_append_helper(),
    ])
}

/// `int main(void)` — the ELF entry. Creates the GtkApplication, wires the
/// `activate` signal, and runs the GTK main loop; the loop owns the process until
/// the window closes (plan-05 §6.1).
fn emit_main_bootstrap() -> CodeFunction {
    let mut asm = Asm::new(MAIN_SYMBOL);
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(16));

    // app = gtk_application_new("dev.mfbasic.app", G_APPLICATION_DEFAULT_FLAGS)
    asm.local_address("x0", STR_APP_ID.0);
    asm.push(abi::move_immediate(
        "x1",
        "Integer",
        G_APPLICATION_DEFAULT_FLAGS,
    ));
    asm.call_external("gtk_application_new");
    asm.store_state("x0", ST_APPLICATION);

    // g_signal_connect_data(app, "activate", on_activate, NULL, NULL, 0)
    asm.load_state("x0", ST_APPLICATION);
    asm.local_address("x1", STR_ACTIVATE.0);
    asm.local_address("x2", ACTIVATE_SYMBOL);
    asm.push(abi::move_immediate("x3", "Integer", "0"));
    asm.push(abi::move_immediate("x4", "Integer", "0"));
    asm.push(abi::move_immediate("x5", "Integer", "0"));
    asm.call_external("g_signal_connect_data");

    // g_application_run(app, 0, NULL). The scaffold passes argc=0/argv=NULL rather
    // than sourcing them from the process stack (TODO(plan-05): forward argv).
    asm.load_state("x0", ST_APPLICATION);
    asm.push(abi::move_immediate("x1", "Integer", "0"));
    asm.push(abi::move_immediate("x2", "Integer", "0"));
    asm.call_external("g_application_run");

    // _exit(0) — the loop returned, so the app is done.
    asm.push(abi::move_immediate("x0", "Integer", "0"));
    asm.call_external("_exit");
    asm.push(abi::branch_self());
    asm.push(abi::return_());
    asm.finish(MAIN_SYMBOL, "Nothing")
}

/// `void on_activate(GtkApplication *app /*x0*/, gpointer user_data)` — build the
/// window (transcript + input field), wire input/close signals, present it, create
/// the window-input pipe (dup'd onto fd 0 for the reused console readers), and
/// spawn the language worker thread.
fn emit_activate_handler() -> CodeFunction {
    let mut asm = Asm::new(ACTIVATE_SYMBOL);
    // lr@0, pthread_t@8, pipe fds (2x i32)@16.
    let frame = 32;
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(frame));
    asm.push(abi::store_u64(abi::link_register(), abi::stack_pointer(), 0));

    // window = gtk_application_window_new(app)
    asm.call_external("gtk_application_window_new");
    asm.store_state("x0", ST_WINDOW);
    // gtk_window_set_title(window, "MFBASIC App")
    asm.load_state("x0", ST_WINDOW);
    asm.local_address("x1", STR_TITLE.0);
    asm.call_external("gtk_window_set_title");
    // gtk_window_set_default_size(window, 900, 640)
    asm.load_state("x0", ST_WINDOW);
    asm.push(abi::move_immediate("x1", "Integer", WINDOW_WIDTH));
    asm.push(abi::move_immediate("x2", "Integer", WINDOW_HEIGHT));
    asm.call_external("gtk_window_set_default_size");

    // box = gtk_box_new(GTK_ORIENTATION_VERTICAL, 0)
    asm.push(abi::move_immediate("x0", "Integer", GTK_ORIENTATION_VERTICAL));
    asm.push(abi::move_immediate("x1", "Integer", "0"));
    asm.call_external("gtk_box_new");
    asm.store_state("x0", ST_BOX);

    // scrolled = gtk_scrolled_window_new(); gtk_widget_set_vexpand(scrolled, TRUE)
    asm.call_external("gtk_scrolled_window_new");
    asm.store_state("x0", ST_SCROLLED);
    asm.load_state("x0", ST_SCROLLED);
    asm.push(abi::move_immediate("x1", "Integer", TRUE));
    asm.call_external("gtk_widget_set_vexpand");

    // text_view = gtk_text_view_new(); editable=FALSE; monospace=TRUE
    asm.call_external("gtk_text_view_new");
    asm.store_state("x0", ST_TEXT_VIEW);
    asm.load_state("x0", ST_TEXT_VIEW);
    asm.push(abi::move_immediate("x1", "Integer", FALSE));
    asm.call_external("gtk_text_view_set_editable");
    asm.load_state("x0", ST_TEXT_VIEW);
    asm.push(abi::move_immediate("x1", "Integer", TRUE));
    asm.call_external("gtk_text_view_set_monospace");
    // buffer = gtk_text_view_get_buffer(text_view)
    asm.load_state("x0", ST_TEXT_VIEW);
    asm.call_external("gtk_text_view_get_buffer");
    asm.store_state("x0", ST_TEXT_BUFFER);
    // gtk_scrolled_window_set_child(scrolled, text_view)
    asm.load_state("x0", ST_SCROLLED);
    asm.load_state("x1", ST_TEXT_VIEW);
    asm.call_external("gtk_scrolled_window_set_child");

    // input = gtk_entry_new(); connect "activate" -> on_input_committed
    asm.call_external("gtk_entry_new");
    asm.store_state("x0", ST_INPUT_FIELD);
    asm.load_state("x0", ST_INPUT_FIELD);
    asm.local_address("x1", STR_ACTIVATE.0);
    asm.local_address("x2", INPUT_COMMITTED_SYMBOL);
    asm.push(abi::move_immediate("x3", "Integer", "0"));
    asm.push(abi::move_immediate("x4", "Integer", "0"));
    asm.push(abi::move_immediate("x5", "Integer", "0"));
    asm.call_external("g_signal_connect_data");

    // box_append(box, scrolled); box_append(box, input); window_set_child(window, box)
    asm.load_state("x0", ST_BOX);
    asm.load_state("x1", ST_SCROLLED);
    asm.call_external("gtk_box_append");
    asm.load_state("x0", ST_BOX);
    asm.load_state("x1", ST_INPUT_FIELD);
    asm.call_external("gtk_box_append");
    asm.load_state("x0", ST_WINDOW);
    asm.load_state("x1", ST_BOX);
    asm.call_external("gtk_window_set_child");

    // connect window "close-request" -> on_window_closed
    asm.load_state("x0", ST_WINDOW);
    asm.local_address("x1", STR_CLOSE_REQUEST.0);
    asm.local_address("x2", WINDOW_CLOSED_SYMBOL);
    asm.push(abi::move_immediate("x3", "Integer", "0"));
    asm.push(abi::move_immediate("x4", "Integer", "0"));
    asm.push(abi::move_immediate("x5", "Integer", "0"));
    asm.call_external("g_signal_connect_data");

    // gtk_window_present(window); gtk_widget_grab_focus(input)
    asm.load_state("x0", ST_WINDOW);
    asm.call_external("gtk_window_present");
    asm.load_state("x0", ST_INPUT_FIELD);
    asm.call_external("gtk_widget_grab_focus");

    // pipe(fds@sp+16); read end -> fd 0 so the reused console readers consume
    // committed input; both ends stashed in the runtime state (plan-05 §6.6).
    asm.push(abi::add_immediate("x0", abi::stack_pointer(), 16));
    asm.call_external("pipe");
    asm.push(abi::load_u32("x10", abi::stack_pointer(), 16)); // read fd
    asm.push(abi::load_u32("x11", abi::stack_pointer(), 20)); // write fd
    asm.store_state("x10", ST_PIPE_READ_FD);
    asm.store_state("x11", ST_PIPE_WRITE_FD);
    asm.push(abi::move_register("x0", "x10"));
    asm.push(abi::move_immediate("x1", "Integer", "0")); // dup2(read, 0)
    asm.call_external("dup2");

    // pthread_create(&thread@sp+8, NULL, _mfb_gtkapp_worker, NULL); detach.
    asm.push(abi::add_immediate("x0", abi::stack_pointer(), 8));
    asm.push(abi::move_immediate("x1", "Integer", "0"));
    asm.local_address("x2", WORKER_SYMBOL);
    asm.push(abi::move_immediate("x3", "Integer", "0"));
    asm.call_external("pthread_create");
    asm.push(abi::load_u64("x0", abi::stack_pointer(), 8));
    asm.call_external("pthread_detach");

    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::add_stack(frame));
    asm.push(abi::return_());
    asm.finish(ACTIVATE_SYMBOL, "Nothing")
}

/// `void *_mfb_gtkapp_worker(void *arg)` — pthread start routine that runs the
/// standard program entry. The program ends via [`FINISH_SYMBOL`], so the tail is
/// only reached defensively.
fn emit_worker_shim(spec: &AppEntrySpec) -> CodeFunction {
    let mut asm = Asm::new(WORKER_SYMBOL);
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(16));
    asm.push(abi::store_u64(abi::link_register(), abi::stack_pointer(), 0));
    if spec.language_entry_accepts_args {
        // Scaffold: no argv plumbing yet (TODO(plan-05)); pass argc=0/argv=NULL.
        asm.push(abi::move_immediate("x0", "Integer", "0"));
        asm.push(abi::move_immediate("x1", "Integer", "0"));
    }
    asm.call_internal(code::MACAPP_PROGRAM_SYMBOL);
    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::add_stack(16));
    asm.push(abi::move_immediate("x0", "Integer", "0"));
    asm.push(abi::return_());
    asm.finish(WORKER_SYMBOL, "Pointer")
}

/// `gboolean on_input_committed(GtkEntry *entry /*x0*/, gpointer user_data)` —
/// push the committed line + newline into the window-input pipe (so the fd-0
/// console readers observe it) and clear the entry. Echoing committed input into
/// the transcript is deferred (TODO(plan-05) §6.6).
fn emit_input_committed_handler() -> CodeFunction {
    let mut asm = Asm::new(INPUT_COMMITTED_SYMBOL);
    // lr@0, entry widget@8, text ptr@16, newline scratch byte@24.
    let frame = 32;
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(frame));
    asm.push(abi::store_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::store_u64("x0", abi::stack_pointer(), 8)); // entry widget

    // text = gtk_editable_get_text(entry)  (entry already in x0)
    asm.call_external("gtk_editable_get_text");
    asm.push(abi::store_u64("x0", abi::stack_pointer(), 16)); // text ptr
    // len = strlen(text)  (text already in x0)
    asm.call_external("strlen");
    asm.push(abi::move_register("x2", "x0")); // len
    asm.push(abi::load_u64("x1", abi::stack_pointer(), 16)); // text ptr
    asm.load_state("x0", ST_PIPE_WRITE_FD);
    asm.call_external("write");
    // write a trailing '\n' so readChar/readByte see the line boundary.
    asm.push(abi::move_immediate("x9", "Integer", "10"));
    asm.push(abi::store_u8("x9", abi::stack_pointer(), 24));
    asm.load_state("x0", ST_PIPE_WRITE_FD);
    asm.push(abi::add_immediate("x1", abi::stack_pointer(), 24));
    asm.push(abi::move_immediate("x2", "Integer", "1"));
    asm.call_external("write");

    // gtk_editable_set_text(entry, "")
    asm.push(abi::load_u64("x0", abi::stack_pointer(), 8));
    asm.local_address("x1", STR_EMPTY.0);
    asm.call_external("gtk_editable_set_text");

    asm.push(abi::move_immediate("x0", "Boolean", TRUE)); // handled
    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::add_stack(frame));
    asm.push(abi::return_());
    asm.finish(INPUT_COMMITTED_SYMBOL, "Boolean")
}

/// `gboolean on_window_closed(GtkWindow *window, gpointer user_data)` — quit the
/// application and allow the default close (return FALSE).
fn emit_window_closed_handler() -> CodeFunction {
    let mut asm = Asm::new(WINDOW_CLOSED_SYMBOL);
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(16));
    asm.push(abi::store_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.load_state("x0", ST_APPLICATION);
    asm.call_external("g_application_quit");
    asm.push(abi::move_immediate("x0", "Boolean", FALSE)); // allow default close
    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::add_stack(16));
    asm.push(abi::return_());
    asm.finish(WINDOW_CLOSED_SYMBOL, "Boolean")
}

/// Worker program-completion handler. SCAFFOLD: hard-exits the process; the
/// keep-window-open policy of §6.7 (schedule a `g_idle_add` notification, leave
/// the window up, show the exit status) is deferred.
fn emit_finish_helper() -> CodeFunction {
    let mut asm = Asm::new(FINISH_SYMBOL);
    asm.push(abi::label("entry"));
    asm.push(abi::move_immediate("x0", "Integer", "0"));
    asm.call_external("_exit");
    asm.push(abi::branch_self());
    asm.push(abi::return_());
    asm.finish(FINISH_SYMBOL, "Nothing")
}

/// `void _mfb_gtkapp_append(GtkTextBuffer *buffer /*x0*/, const char *text /*x1*/,
/// gsize len /*x2*/)` — append `len` bytes at the buffer's end iterator.
///
/// TODO(plan-05 §6.4): this runs on the worker thread; GTK requires it on the
/// main-loop thread, so the real implementation must marshal via `g_idle_add` (or
/// `g_main_context_invoke_full` for the synchronous flush). Batching + auto-scroll
/// to the end mark are likewise pending.
fn emit_append_helper() -> CodeFunction {
    let mut asm = Asm::new(APPEND_SYMBOL);
    // lr@0, buffer@8, text@16, len@24, GtkTextIter@40 (>=80 bytes room to 128).
    let frame = 128;
    let iter = 40;
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(frame));
    asm.push(abi::store_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::store_u64("x0", abi::stack_pointer(), 8));
    asm.push(abi::store_u64("x1", abi::stack_pointer(), 16));
    asm.push(abi::store_u64("x2", abi::stack_pointer(), 24));

    // gtk_text_buffer_get_end_iter(buffer, &iter)
    asm.push(abi::load_u64("x0", abi::stack_pointer(), 8));
    asm.push(abi::add_immediate("x1", abi::stack_pointer(), iter));
    asm.call_external("gtk_text_buffer_get_end_iter");
    // gtk_text_buffer_insert(buffer, &iter, text, len)
    asm.push(abi::load_u64("x0", abi::stack_pointer(), 8));
    asm.push(abi::add_immediate("x1", abi::stack_pointer(), iter));
    asm.push(abi::load_u64("x2", abi::stack_pointer(), 16));
    asm.push(abi::load_u64("x3", abi::stack_pointer(), 24));
    asm.call_external("gtk_text_buffer_insert");

    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::add_stack(frame));
    asm.push(abi::return_());
    asm.finish(APPEND_SYMBOL, "Nothing")
}

// --- io::* app-mode helper bodies ------------------------------------------

/// App-mode `io.print`/`io.write`/`io.printError`/`io.writeError`. The MFB string
/// object is in `x0` (`[x0]` = length, `x0+8` = UTF-8 bytes). When a transcript
/// buffer is attached, append to it; otherwise fall back to the stdout/stderr file
/// descriptor (the only path verified in headless runs). Returns `OK` (x0 = 0).
pub(crate) fn emit_app_io_write_helper(
    symbol: &str,
    stderr: bool,
    newline: bool,
) -> (CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>) {
    let fd = if stderr { "2" } else { "1" };
    let mut asm = Asm::new(symbol);
    // lr@0, x19(string)@8, newline scratch byte@16.
    let frame = 32;
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(frame));
    asm.push(abi::store_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::store_u64("x19", abi::stack_pointer(), 8));
    asm.push(abi::move_register("x19", "x0")); // preserve string object

    // buffer = state.text_buffer; nil => fd fallback.
    asm.load_state("x10", ST_TEXT_BUFFER);
    asm.push(abi::compare_immediate("x10", "0"));
    asm.push(abi::branch_eq("fd_path"));

    // --- transcript path (TODO(plan-05): marshal to the main thread) ---
    // TODO(plan-05 §5.4): style stderr runs with a GtkTextTag; appended plain here.
    asm.push(abi::move_register("x0", "x10"));
    asm.push(abi::add_immediate("x1", "x19", 8));
    asm.push(abi::load_u64("x2", "x19", 0));
    asm.call_internal(APPEND_SYMBOL);
    if newline {
        asm.push(abi::move_immediate("x9", "Integer", "10"));
        asm.push(abi::store_u8("x9", abi::stack_pointer(), 16));
        asm.load_state("x0", ST_TEXT_BUFFER);
        asm.push(abi::add_immediate("x1", abi::stack_pointer(), 16));
        asm.push(abi::move_immediate("x2", "Integer", "1"));
        asm.call_internal(APPEND_SYMBOL);
    }
    asm.push(abi::move_immediate("x0", "Integer", "0")); // RESULT_OK_TAG
    asm.push(abi::branch("done"));

    // --- fd fallback path ---
    asm.push(abi::label("fd_path"));
    asm.push(abi::move_immediate("x0", "Integer", fd));
    asm.push(abi::add_immediate("x1", "x19", 8));
    asm.push(abi::load_u64("x2", "x19", 0));
    asm.call_external("write");
    if newline {
        asm.push(abi::move_immediate("x9", "Integer", "10"));
        asm.push(abi::store_u8("x9", abi::stack_pointer(), 16));
        asm.push(abi::move_immediate("x0", "Integer", fd));
        asm.push(abi::add_immediate("x1", abi::stack_pointer(), 16));
        asm.push(abi::move_immediate("x2", "Integer", "1"));
        asm.call_external("write");
    }
    asm.push(abi::move_immediate("x0", "Integer", "0")); // RESULT_OK_TAG

    asm.push(abi::label("done"));
    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::load_u64("x19", abi::stack_pointer(), 8));
    asm.push(abi::add_stack(frame));
    asm.push(abi::return_());
    (
        CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        asm.ins,
        asm.rel,
    )
}

/// App-mode `io.flush`/`io.flushError`: returns `OK` immediately. SCAFFOLD: real
/// flush must drain the pending main-thread transcript update (§5.4) once
/// marshaling lands.
pub(crate) fn emit_app_io_flush_helper(
    symbol: &str,
) -> (CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>) {
    let mut asm = Asm::new(symbol);
    asm.push(abi::label("entry"));
    asm.push(abi::move_immediate("x0", "Integer", "0")); // RESULT_OK_TAG
    asm.push(abi::return_());
    (
        CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        asm.ins,
        asm.rel,
    )
}

/// App-mode `io.input` (plan-05 §5.4): render the prompt to the transcript via the
/// `io.write` helper, then read a committed line via the `io.readLine` helper
/// (which reads fd 0 — the window-input pipe). Prompt string is in `x0`.
pub(crate) fn emit_app_io_input_helper(
    symbol: &str,
) -> (CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>) {
    let mut asm = Asm::new(symbol);
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(16));
    asm.push(abi::store_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.call_internal(IO_WRITE_SYMBOL); // x0 = prompt; result ignored
    asm.call_internal(IO_READ_LINE_SYMBOL); // result in x0/x1/x2
    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::add_stack(16));
    asm.push(abi::return_());
    (
        CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        asm.ins,
        asm.rel,
    )
}

/// App-mode `io.isInputTerminal`/`io.isOutputTerminal`/`io.isErrorTerminal`
/// (plan-05 §5.4): the window is the interactive console, so all three return
/// `OK(TRUE)`. Result ABI: x0 = tag (0 = ok), x1 = value.
pub(crate) fn emit_app_io_is_terminal_helper(
    symbol: &str,
) -> (CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>) {
    let mut asm = Asm::new(symbol);
    asm.push(abi::label("entry"));
    asm.push(abi::move_immediate("x1", "Boolean", "1")); // value = TRUE
    asm.push(abi::move_immediate("x0", "Integer", "0")); // tag = OK
    asm.push(abi::return_());
    (
        CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        asm.ins,
        asm.rel,
    )
}

/// Read-only C-string data symbols + the writable runtime-state global.
pub(crate) fn app_mode_data_objects() -> Vec<CodeDataObject> {
    let mut objects: Vec<CodeDataObject> = [
        STR_APP_ID,
        STR_TITLE,
        STR_ACTIVATE,
        STR_CLOSE_REQUEST,
        STR_EMPTY,
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
        layout: "mfb.runtime.gtkapp_state.v1 { u64 handles[9] }".to_string(),
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
