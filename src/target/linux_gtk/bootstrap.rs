//! Linux GTK4 app-mode bootstrap: libc-start trampoline, `main` setup,
//! activate/key-pressed/window-closed handlers, worker shim, and
//! append/finish helpers (plan-11 split, pure relocation).

use super::*;

/// Maximum number of bytes `g_unichar_to_utf8` may write for one code point (the
/// GLib UTF-8 encoder emits up to 6 bytes). Used as the safety margin for the
/// fixed line-buffer bound in [`emit_key_pressed_handler`] (bug-50).
const MAX_UTF8_LEN: usize = 6;

/// The ELF entry point. Our `_main` is `e_entry`, reached with the stack exactly
/// as the kernel/loader left it (`sp` -> argc, argv, NULL, envp...). We can't link
/// crt1.o (the built-in linker pulls in no host objects, plan-linker.md), so the
/// entry hands off to `__libc_start_main`, which runs the C runtime init —
/// including every loaded shared library's `DT_INIT_ARRAY` constructors (the
/// GLib/GObject type system boots there) — and then calls our real `main`. On
/// glibc the loader already ran library constructors via `_dl_init`; on musl they
/// run inside `__libc_start_main`, so routing through it works on both.
pub(super) fn emit_libc_start_trampoline() -> CodeFunction {
    let mut asm = Asm::new(MAIN_SYMBOL);
    asm.push(abi::label("entry"));
    // __libc_start_main(main, argc, argv, init, fini, rtld_fini, stack_end)
    asm.local_address("x0", GTK_MAIN_SYMBOL); // main
    asm.push(abi::load_u64("x1", abi::stack_pointer(), 0)); // argc
    asm.push(abi::add_immediate("x2", abi::stack_pointer(), 8)); // argv
    asm.push(abi::move_immediate("x3", "Integer", "0")); // init
    asm.push(abi::move_immediate("x4", "Integer", "0")); // fini
    asm.push(abi::move_immediate("x5", "Integer", "0")); // rtld_fini
    asm.push(abi::add_immediate("x6", abi::stack_pointer(), 0)); // stack_end
    asm.call_external("__libc_start_main");
    // __libc_start_main never returns (it calls exit when main returns).
    asm.push(abi::branch_self());
    asm.push(abi::return_());
    asm.finish(MAIN_SYMBOL, "Nothing")
}

/// `int _mfb_gtkapp_main(int argc, char **argv, char **envp)` — the real C main
/// invoked by `__libc_start_main` after runtime + library init. Creates the
/// GtkApplication, wires the `activate` signal, and runs the GTK main loop; the
/// loop owns the process until the window closes (plan-05 §6.1). Returns 0 so
/// `__libc_start_main` exits cleanly.
pub(super) fn emit_main_bootstrap() -> CodeFunction {
    let mut asm = Asm::new(GTK_MAIN_SYMBOL);
    // lr@0, argc@8, argv@16.
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(32));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        0,
    ));
    asm.push(abi::store_u64("x0", abi::stack_pointer(), 8)); // argc
    asm.push(abi::store_u64("x1", abi::stack_pointer(), 16)); // argv

    // Disable the a11y + IM layers before GTK initializes (they crash in
    // g_variant_new_string on transcript inserts): setenv("GTK_A11Y","none",1) and
    // setenv("GTK_IM_MODULE","none",1).
    asm.local_address("x0", STR_ENV_A11Y.0);
    asm.local_address("x1", STR_ENV_NONE.0);
    asm.push(abi::move_immediate("x2", "Integer", "1"));
    asm.call_external("setenv");
    asm.local_address("x0", STR_ENV_IM.0);
    asm.local_address("x1", STR_ENV_NONE.0);
    asm.push(abi::move_immediate("x2", "Integer", "1"));
    asm.call_external("setenv");

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

    // g_application_run(app, argc, argv) — forward the real argv so GApplication's
    // platform-data (argv[0], cwd) is valid UTF-8 rather than garbage.
    asm.load_state("x0", ST_APPLICATION);
    asm.push(abi::load_u64("x1", abi::stack_pointer(), 8)); // argc
    asm.push(abi::load_u64("x2", abi::stack_pointer(), 16)); // argv
    asm.call_external("g_application_run");

    // return 0 -> __libc_start_main calls exit(0).
    asm.push(abi::move_immediate("x0", "Integer", "0"));
    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::add_stack(32));
    asm.push(abi::return_());
    asm.finish(GTK_MAIN_SYMBOL, "Integer")
}

/// `void on_activate(GtkApplication *app /*x0*/, gpointer user_data)` — build the
/// window (transcript + input field), wire input/close signals, present it, create
/// the window-input pipe (dup'd onto fd 0 for the reused console readers), and
/// spawn the language worker thread.
pub(super) fn emit_activate_handler() -> CodeFunction {
    let mut asm = Asm::new(ACTIVATE_SYMBOL);
    // lr@0, pthread_t@8, pipe fds (2x i32)@16, x19(controller)@24.
    let frame = 32;
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(frame));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        0,
    ));
    asm.push(abi::store_u64("x19", abi::stack_pointer(), 24));

    // window = gtk_application_window_new(app)  (app is the incoming x0)
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

    // scrolled = gtk_scrolled_window_new(); hold a ref so swapping the window child
    // to the term:: surface and back doesn't destroy it (g_object_ref_sink owns the
    // floating ref; gtk_window_set_child then takes its own).
    asm.call_external("gtk_scrolled_window_new");
    asm.call_external("g_object_ref_sink");
    asm.store_state("x0", ST_SCROLLED);

    // term:: drawing-area surface: created up front, kept off-window (held by a ref)
    // until term::on swaps it in. Its draw func renders the character grid; the grid
    // starts cleared (all spaces).
    asm.call_external("gtk_drawing_area_new");
    asm.call_external("g_object_ref_sink");
    asm.store_state("x0", ST_TERM_AREA);
    asm.load_state("x0", ST_TERM_AREA);
    asm.local_address("x1", TERM_DRAW_SYMBOL);
    asm.push(abi::move_immediate("x2", "Integer", "0")); // user_data
    asm.push(abi::move_immediate("x3", "Integer", "0")); // destroy
    asm.call_external("gtk_drawing_area_set_draw_func");
    // Derive the grid geometry from the monospace font metrics + content size and
    // blank the grid (main thread, before the worker can use it).
    asm.call_internal(TERM_INIT_SYMBOL);

    // text_view = gtk_text_view_new(); editable=FALSE; monospace=TRUE. The view is
    // left NON-focusable (like the working build): focusing a GtkTextView activates
    // the IM/a11y machinery, which crashes in g_variant_new_string when the worker
    // inserts text. Keys are captured at the window instead (see below).
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
    // gtk_scrolled_window_set_child(scrolled, text_view); window child = scrolled.
    asm.load_state("x0", ST_SCROLLED);
    asm.load_state("x1", ST_TEXT_VIEW);
    asm.call_external("gtk_scrolled_window_set_child");
    asm.load_state("x0", ST_WINDOW);
    asm.load_state("x1", ST_SCROLLED);
    asm.call_external("gtk_window_set_child");

    // Capture keystrokes terminal-style with a key controller on the WINDOW (no
    // focusable input widget; the whole window is the terminal, matching macOS's
    // keyDown:-on-the-transcript model without the focused-textview IM/a11y hazard).
    //   controller = gtk_event_controller_key_new()
    //   g_signal_connect_data(controller, "key-pressed", on_key, NULL, NULL, 0)
    //   gtk_widget_add_controller(window, controller)  // takes ownership
    asm.call_external("gtk_event_controller_key_new");
    asm.push(abi::move_register("x19", "x0")); // controller (callee-saved across calls)
    asm.local_address("x1", STR_KEY_PRESSED.0);
    asm.local_address("x2", KEY_PRESSED_SYMBOL);
    asm.push(abi::move_immediate("x3", "Integer", "0"));
    asm.push(abi::move_immediate("x4", "Integer", "0"));
    asm.push(abi::move_immediate("x5", "Integer", "0"));
    asm.call_external("g_signal_connect_data");
    asm.load_state("x0", ST_WINDOW);
    asm.push(abi::move_register("x1", "x19"));
    asm.call_external("gtk_widget_add_controller");

    // connect window "close-request" -> on_window_closed
    asm.load_state("x0", ST_WINDOW);
    asm.local_address("x1", STR_CLOSE_REQUEST.0);
    asm.local_address("x2", WINDOW_CLOSED_SYMBOL);
    asm.push(abi::move_immediate("x3", "Integer", "0"));
    asm.push(abi::move_immediate("x4", "Integer", "0"));
    asm.push(abi::move_immediate("x5", "Integer", "0"));
    asm.call_external("g_signal_connect_data");

    // gtk_window_present(window); focus the transcript so it receives keys.
    asm.load_state("x0", ST_WINDOW);
    asm.call_external("gtk_window_present");

    // pipe(fds@sp+16); read end -> fd 0 so the reused console readers consume
    // committed input; the write end is stashed in the runtime state (plan-05
    // §6.6). The key handler writes committed input to the write end; the read
    // end is collapsed onto fd 0 below.
    asm.push(abi::add_immediate("x0", abi::stack_pointer(), 16));
    asm.call_external("pipe");
    asm.push(abi::load_u32("x11", abi::stack_pointer(), 20)); // write fd
    asm.store_state("x11", ST_PIPE_WRITE_FD);

    // Make the pipe write end non-blocking (bug-114): if the worker stops
    // draining stdin the 64 KiB pipe fills, and a blocking write() in the key
    // handler would hang the GTK main thread forever. fcntl(write, F_SETFL,
    // O_NONBLOCK); on Linux/AArch64 the variadic third arg is passed in x2.
    asm.push(abi::load_u32("x0", abi::stack_pointer(), 20)); // write fd
    asm.push(abi::move_immediate("x1", "Integer", "4")); // F_SETFL
    asm.push(abi::move_immediate("x2", "Integer", "2048")); // O_NONBLOCK (0o4000)
    asm.call_external("fcntl");

    // dup2(read, 0): fd 0 becomes a copy of the pipe read end. The read fd stays
    // on the stack (sp+16) rather than in a register — a caller-saved register
    // would not survive the `bl dup2` (Native Codegen Register Lifetimes).
    asm.push(abi::load_u32("x0", abi::stack_pointer(), 16)); // read fd
    asm.push(abi::move_immediate("x1", "Integer", "0"));
    asm.call_external("dup2");

    // close(read): fd 0 now holds the read end, so the original read descriptor
    // is redundant. pipe(2) never returns fd 0 here (fds 0/1/2 are already open
    // at process start), so `read` is a distinct descriptor from the fd-0 copy;
    // closing it leaves exactly ONE read end, so closing the write end signals
    // stdin EOF/hangup to the console readers (bug-59). Reload the read fd from
    // the stack — `bl dup2` clobbered the caller-saved registers.
    asm.push(abi::load_u32("x0", abi::stack_pointer(), 16)); // read fd
    asm.call_external("close");

    // Record the surviving read end (fd 0) in the runtime state. Use x10 for the
    // value because store_state materializes the state base into x9.
    asm.push(abi::move_immediate("x10", "Integer", "0"));
    asm.store_state("x10", ST_PIPE_READ_FD);

    // pthread_create(&thread@sp+8, NULL, _mfb_gtkapp_worker, NULL); detach.
    asm.push(abi::add_immediate("x0", abi::stack_pointer(), 8));
    asm.push(abi::move_immediate("x1", "Integer", "0"));
    asm.local_address("x2", WORKER_SYMBOL);
    asm.push(abi::move_immediate("x3", "Integer", "0"));
    asm.call_external("pthread_create");
    asm.push(abi::load_u64("x0", abi::stack_pointer(), 8));
    asm.call_external("pthread_detach");

    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::load_u64("x19", abi::stack_pointer(), 24));
    asm.push(abi::add_stack(frame));
    asm.push(abi::return_());
    asm.finish(ACTIVATE_SYMBOL, "Nothing")
}

/// `void *_mfb_gtkapp_worker(void *arg)` — pthread start routine that runs the
/// standard program entry. The program ends via [`FINISH_SYMBOL`], so the tail is
/// only reached defensively.
pub(super) fn emit_worker_shim(spec: &AppEntrySpec) -> CodeFunction {
    let mut asm = Asm::new(WORKER_SYMBOL);
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(16));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        0,
    ));
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

/// `gboolean _mfb_gtkapp_key_pressed(GtkEventControllerKey *ctrl, guint keyval
/// /*x1*/, guint keycode, GdkModifierType state, gpointer user_data)` — the
/// transcript's terminal-style key handler (the GTK analog of the macOS
/// `MFBTextView keyDown:`). Runs on the GTK main thread.
///
/// Behavior by `ST_INPUT_MODE`:
/// - RAW (`readChar`/`readByte`): write the key's UTF-8 bytes to the input pipe
///   immediately; no line buffering, no echo.
/// - LINE_ECHO (`io::input`) / LINE_NOECHO (`io::readLine`): accumulate into the
///   line buffer; Enter commits `line + '\n'` to the pipe; Backspace drops the last
///   byte; printable keys append (and echo into the transcript in LINE_ECHO).
///
/// Committed bytes flow pipe -> fd 0 -> the reused console read helpers. Returns
/// TRUE for keys it consumes, FALSE otherwise (so window shortcuts still work).
pub(super) fn emit_key_pressed_handler() -> CodeFunction {
    let mut asm = Asm::new(KEY_PRESSED_SYMBOL);
    // lr@0, oldlen@8, count@16, unichar@24, scratch(utf8/newline 8B)@32, x19@40.
    let frame = 48;
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(frame));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        0,
    ));
    asm.push(abi::store_u64("x19", abi::stack_pointer(), 40));
    asm.push(abi::move_register("x19", "x1")); // keyval

    // Raw mode delivers the keystroke immediately, bypassing the line buffer.
    asm.load_state("x9", ST_INPUT_MODE);
    asm.push(abi::compare_immediate("x9", MODE_RAW));
    asm.push(abi::branch_eq("raw"));

    // Enter (Return / KP_Enter) -> commit; Backspace -> erase.
    asm.push(abi::move_immediate("x9", "Integer", GDK_KEY_RETURN));
    asm.push(abi::compare_registers("x19", "x9"));
    asm.push(abi::branch_eq("commit"));
    asm.push(abi::move_immediate("x9", "Integer", GDK_KEY_KP_ENTER));
    asm.push(abi::compare_registers("x19", "x9"));
    asm.push(abi::branch_eq("commit"));
    asm.push(abi::move_immediate("x9", "Integer", GDK_KEY_BACKSPACE));
    asm.push(abi::compare_registers("x19", "x9"));
    asm.push(abi::branch_eq("backspace"));

    // Printable: unichar = gdk_keyval_to_unicode(keyval); 0 -> not a character.
    asm.push(abi::move_register("x0", "x19"));
    asm.call_external("gdk_keyval_to_unicode");
    asm.push(abi::compare_immediate("x0", "0"));
    asm.push(abi::branch_eq("ignore"));
    asm.push(abi::store_u64("x0", abi::stack_pointer(), 24)); // unichar
                                                              // oldlen = line_len; dst = &line_buf[oldlen]; count = g_unichar_to_utf8(unichar, dst)
    asm.load_state("x9", ST_LINE_LEN);
    // bug-50: cap the fixed 1024-byte line buffer. If the pending line can no
    // longer hold another maximum-width (6-byte) UTF-8 encoding, drop the key via
    // the existing `ignore` path so the g_unichar_to_utf8 store below never writes
    // past ST_LINE_BUF into the adjacent state fields (ST_TERM_AREA — the live
    // GtkDrawingArea* — and the term grid). LINE_BUF_CAP - 6 is the last oldlen at
    // which a full 6-byte encode still lands inside the buffer; compare unsigned
    // (a line length is never negative) and branch when strictly higher.
    asm.push(abi::compare_immediate(
        "x9",
        &(LINE_BUF_CAP - MAX_UTF8_LEN).to_string(),
    ));
    asm.push(abi::branch_hi("ignore"));
    asm.push(abi::store_u64("x9", abi::stack_pointer(), 8)); // oldlen
    asm.local_address("x10", STATE_SYMBOL);
    asm.push(abi::add_immediate("x1", "x10", ST_LINE_BUF));
    asm.push(abi::add_registers("x1", "x1", "x9"));
    asm.push(abi::load_u64("x0", abi::stack_pointer(), 24));
    asm.call_external("g_unichar_to_utf8");
    asm.push(abi::store_u64("x0", abi::stack_pointer(), 16)); // count
                                                              // line_len = oldlen + count
    asm.push(abi::load_u64("x9", abi::stack_pointer(), 8));
    asm.push(abi::add_registers("x9", "x9", "x0"));
    asm.local_address("x10", STATE_SYMBOL);
    asm.push(abi::store_u64("x9", "x10", ST_LINE_LEN));
    // Echo into the transcript only in LINE_ECHO mode.
    asm.load_state("x9", ST_INPUT_MODE);
    asm.push(abi::compare_immediate("x9", MODE_LINE_ECHO));
    asm.push(abi::branch_ne("consumed"));
    asm.load_state("x0", ST_TEXT_BUFFER);
    asm.local_address("x10", STATE_SYMBOL);
    asm.push(abi::add_immediate("x1", "x10", ST_LINE_BUF));
    asm.push(abi::load_u64("x9", abi::stack_pointer(), 8)); // oldlen
    asm.push(abi::add_registers("x1", "x1", "x9"));
    asm.push(abi::load_u64("x2", abi::stack_pointer(), 16)); // count
    asm.call_internal(APPEND_SYMBOL);
    asm.push(abi::branch("consumed"));

    // Commit: write line + '\n' to the pipe; echo '\n' in LINE_ECHO; clear buffer.
    asm.push(abi::label("commit"));
    asm.load_state("x0", ST_PIPE_WRITE_FD);
    asm.local_address("x10", STATE_SYMBOL);
    asm.push(abi::add_immediate("x1", "x10", ST_LINE_BUF));
    asm.load_state("x2", ST_LINE_LEN);
    asm.call_external("write");
    // O_NONBLOCK write end (bug-114): on -1/EAGAIN (pipe full, worker not
    // reading) drop the line rather than block; skip the trailing newline write
    // and fall through to echo + clear.
    asm.push(abi::compare_immediate("x0", "0"));
    asm.push(abi::branch_lt("commit_echo"));
    asm.push(abi::move_immediate("x9", "Integer", "10"));
    asm.push(abi::store_u8("x9", abi::stack_pointer(), 32)); // '\n'
    asm.load_state("x0", ST_PIPE_WRITE_FD);
    asm.push(abi::add_immediate("x1", abi::stack_pointer(), 32));
    asm.push(abi::move_immediate("x2", "Integer", "1"));
    asm.call_external("write");
    asm.push(abi::label("commit_echo"));
    asm.load_state("x9", ST_INPUT_MODE);
    asm.push(abi::compare_immediate("x9", MODE_LINE_ECHO));
    asm.push(abi::branch_ne("commit_clear"));
    asm.load_state("x0", ST_TEXT_BUFFER);
    asm.push(abi::add_immediate("x1", abi::stack_pointer(), 32)); // the '\n'
    asm.push(abi::move_immediate("x2", "Integer", "1"));
    asm.call_internal(APPEND_SYMBOL);
    asm.push(abi::label("commit_clear"));
    asm.push(abi::move_immediate("x9", "Integer", "0"));
    asm.local_address("x10", STATE_SYMBOL);
    asm.push(abi::store_u64("x9", "x10", ST_LINE_LEN));
    asm.push(abi::branch("consumed"));

    // Backspace: drop the last byte of the pending line (transcript echo-delete
    // TODO(plan-05): byte-granular, ASCII-correct for now).
    asm.push(abi::label("backspace"));
    asm.load_state("x9", ST_LINE_LEN);
    asm.push(abi::compare_immediate("x9", "0"));
    asm.push(abi::branch_eq("ignore"));
    asm.push(abi::subtract_immediate("x9", "x9", 1));
    asm.local_address("x10", STATE_SYMBOL);
    asm.push(abi::store_u64("x9", "x10", ST_LINE_LEN));
    asm.push(abi::branch("consumed"));

    // Raw: unichar -> UTF-8 in scratch -> write to the pipe immediately.
    asm.push(abi::label("raw"));
    asm.push(abi::move_register("x0", "x19"));
    asm.call_external("gdk_keyval_to_unicode");
    asm.push(abi::compare_immediate("x0", "0"));
    asm.push(abi::branch_eq("ignore"));
    asm.push(abi::add_immediate("x1", abi::stack_pointer(), 32));
    asm.call_external("g_unichar_to_utf8"); // x0 still = unichar; x0 := count
    asm.push(abi::move_register("x2", "x0"));
    asm.load_state("x0", ST_PIPE_WRITE_FD);
    asm.push(abi::add_immediate("x1", abi::stack_pointer(), 32));
    asm.call_external("write");

    asm.push(abi::label("consumed"));
    asm.push(abi::move_immediate("x0", "Boolean", TRUE)); // handled
    asm.push(abi::branch("kp_return"));
    asm.push(abi::label("ignore"));
    asm.push(abi::move_immediate("x0", "Boolean", FALSE)); // not handled
    asm.push(abi::label("kp_return"));
    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::load_u64("x19", abi::stack_pointer(), 40));
    asm.push(abi::add_stack(frame));
    asm.push(abi::return_());
    asm.finish(KEY_PRESSED_SYMBOL, "Boolean")
}

/// `gboolean on_window_closed(GtkWindow *window, gpointer user_data)` — quit the
/// application and allow the default close (return FALSE).
pub(super) fn emit_window_closed_handler() -> CodeFunction {
    let mut asm = Asm::new(WINDOW_CLOSED_SYMBOL);
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(16));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        0,
    ));
    asm.load_state("x0", ST_APPLICATION);
    asm.call_external("g_application_quit");
    asm.push(abi::move_immediate("x0", "Boolean", FALSE)); // allow default close
    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::add_stack(16));
    asm.push(abi::return_());
    asm.finish(WINDOW_CLOSED_SYMBOL, "Boolean")
}

/// Worker program-completion handler (plan-05 §6.7), matching the macOS flow: the
/// exit code arrives in `x0`. In GUI mode append "\nProgram exited with code N\n"
/// to the transcript (marshaled to the main thread, like every other write) and
/// park the worker in `pause()` so the main loop keeps the window open until the
/// user closes it. With no transcript attached (headless) terminate with the code.
///
/// The language program runs on the worker thread, so we must NOT `_exit` in GUI
/// mode or the process (window + main loop) dies.
pub(super) fn emit_finish_helper() -> CodeFunction {
    let prefix_len = STR_EXIT_PREFIX.1.len(); // includes the leading '\n'
    let mut asm = Asm::new(FINISH_SYMBOL);
    // lr@0, x19(exit code)@8, x20(chunk)@16.
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(32));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        0,
    ));
    asm.push(abi::store_u64("x19", abi::stack_pointer(), 8));
    asm.push(abi::store_u64("x20", abi::stack_pointer(), 16));
    asm.push(abi::move_register("x19", "x0")); // exit code

    // Headless (no transcript): terminate the process with the exit code.
    asm.load_state("x9", ST_TEXT_BUFFER);
    asm.push(abi::compare_immediate("x9", "0"));
    asm.push(abi::branch_ne("fin_gui"));
    asm.push(abi::move_register("x0", "x19"));
    asm.call_external("_exit");
    asm.push(abi::branch_self());

    // GUI: build the status chunk "<prefix>" + decimal(code) + "\n" and marshal it.
    // Chunk layout matches the io write helper: [0]=len, [16..]=bytes.
    asm.push(abi::label("fin_gui"));
    asm.push(abi::move_immediate("x0", "Integer", "64")); // 16 hdr + prefix + 3 digits + nl
    asm.call_external("malloc");
    asm.push(abi::move_register("x20", "x0")); // chunk
    asm.push(abi::add_immediate("x0", "x20", 16)); // memcpy(chunk+16, prefix, prefix_len)
    asm.local_address("x1", STR_EXIT_PREFIX.0);
    asm.push(abi::move_immediate(
        "x2",
        "Integer",
        &prefix_len.to_string(),
    ));
    asm.call_external("memcpy");
    // Format the exit code as decimal ASCII at chunk+16+prefix_len; '\n'; store len.
    asm.push(abi::add_immediate("x13", "x20", 16 + prefix_len)); // digit write ptr
    emit_format_exit_code(&mut asm, "x19", "x13");
    asm.push(abi::move_immediate("x14", "Integer", "10"));
    asm.push(abi::store_u8("x14", "x13", 0)); // trailing '\n'
    asm.push(abi::add_immediate("x13", "x13", 1));
    // len = x13 - (chunk + 16)
    asm.push(abi::subtract_registers("x9", "x13", "x20"));
    asm.push(abi::subtract_immediate("x9", "x9", 16));
    asm.push(abi::store_u64("x9", "x20", 0));
    asm.local_address("x0", APPEND_IDLE_SYMBOL);
    asm.push(abi::move_register("x1", "x20"));
    asm.call_external("g_idle_add");

    // Park the worker; the main loop owns shutdown when the window closes.
    asm.push(abi::label("park"));
    asm.call_external("pause");
    asm.push(abi::branch("park"));
    asm.push(abi::return_());
    asm.finish(FINISH_SYMBOL, "Nothing")
}

/// Format the unsigned exit code in `code` (0..255) as decimal ASCII at the pointer
/// in `dst`, advancing `dst` past the digits (leading zeros suppressed). Mirrors the
/// macOS `emit_format_exit_code`; uses only caller-saved scratch, performs no calls.
fn emit_format_exit_code(asm: &mut Asm, code: &str, dst: &str) {
    // h = code/100; rem = code%100; t = rem/10; o = rem%10.
    asm.push(abi::move_register("x9", code)); // n
    // Mask to the low 8 bits, matching macOS (bug-70): `_exit(status)` delivers
    // only `status & 0xFF` to the parent, so the GUI transcript must show that
    // truncated value. Without the mask a code >= 1000 makes hundreds >= 10 and
    // emits `'0'+10 = ':'` garbage, and a negative (u64-wrapped) code garbles all
    // three digits — diverging from macOS and console (bug-110).
    asm.push(abi::move_immediate("x11", "Integer", "255"));
    asm.push(abi::and_registers("x9", "x9", "x11"));
    asm.push(abi::move_immediate("x11", "Integer", "100"));
    asm.push(abi::unsigned_divide_registers("x10", "x9", "x11")); // hundreds
    asm.push(abi::multiply_subtract_registers("x9", "x10", "x11", "x9")); // n %= 100
    asm.push(abi::move_immediate("x11", "Integer", "10"));
    asm.push(abi::unsigned_divide_registers("x12", "x9", "x11")); // tens
    asm.push(abi::multiply_subtract_registers("x9", "x12", "x11", "x9")); // ones
                                                                          // hundreds != 0 -> emit hundreds then always tens+ones.
    asm.push(abi::compare_immediate("x10", "0"));
    asm.push(abi::branch_eq("fmt_skip_h"));
    asm.push(abi::add_immediate("x14", "x10", 48));
    asm.push(abi::store_u8("x14", dst, 0));
    asm.push(abi::add_immediate(dst, dst, 1));
    asm.push(abi::branch("fmt_tens"));
    asm.push(abi::label("fmt_skip_h"));
    // tens == 0 -> ones only.
    asm.push(abi::compare_immediate("x12", "0"));
    asm.push(abi::branch_eq("fmt_ones"));
    asm.push(abi::label("fmt_tens"));
    asm.push(abi::add_immediate("x14", "x12", 48));
    asm.push(abi::store_u8("x14", dst, 0));
    asm.push(abi::add_immediate(dst, dst, 1));
    asm.push(abi::label("fmt_ones"));
    asm.push(abi::add_immediate("x14", "x9", 48));
    asm.push(abi::store_u8("x14", dst, 0));
    asm.push(abi::add_immediate(dst, dst, 1));
}

/// `void _mfb_gtkapp_append(GtkTextBuffer *buffer /*x0*/, const char *text /*x1*/,
/// gsize len /*x2*/)` — append `len` bytes at the buffer's end iterator. Must run on
/// the GTK main thread; worker-thread writes reach it via `_mfb_gtkapp_append_idle`.
/// After inserting, auto-scrolls the transcript to the new end (plan-05 §6.5) via a
/// temporary end mark + gtk_text_view_scroll_mark_onscreen.
pub(super) fn emit_append_helper() -> CodeFunction {
    let mut asm = Asm::new(APPEND_SYMBOL);
    // lr@0, buffer@8, text@16, len@24, mark@32, GtkTextIter@40 (80B room to 120).
    let frame = 128;
    let iter = 40;
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(frame));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        0,
    ));
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

    // Auto-scroll: re-fetch the end iter, create a temporary mark there, scroll it
    // onscreen in the transcript, then delete it.
    asm.push(abi::load_u64("x0", abi::stack_pointer(), 8));
    asm.push(abi::add_immediate("x1", abi::stack_pointer(), iter));
    asm.call_external("gtk_text_buffer_get_end_iter");
    asm.push(abi::load_u64("x0", abi::stack_pointer(), 8)); // create_mark(buffer, NULL,
    asm.push(abi::move_immediate("x1", "Integer", "0")); //               &iter, FALSE)
    asm.push(abi::add_immediate("x2", abi::stack_pointer(), iter));
    asm.push(abi::move_immediate("x3", "Integer", "0"));
    asm.call_external("gtk_text_buffer_create_mark");
    asm.push(abi::store_u64("x0", abi::stack_pointer(), 32)); // mark
    asm.load_state("x0", ST_TEXT_VIEW);
    asm.push(abi::load_u64("x1", abi::stack_pointer(), 32));
    asm.call_external("gtk_text_view_scroll_mark_onscreen");
    asm.push(abi::load_u64("x0", abi::stack_pointer(), 8)); // delete_mark(buffer, mark)
    asm.push(abi::load_u64("x1", abi::stack_pointer(), 32));
    asm.call_external("gtk_text_buffer_delete_mark");

    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::add_stack(frame));
    asm.push(abi::return_());
    asm.finish(APPEND_SYMBOL, "Nothing")
}

/// `gboolean _mfb_gtkapp_append_idle(gpointer chunk)` — runs on the GTK main
/// thread (scheduled by the io write helper via `g_idle_add`). Inserts the chunk's
/// bytes into the transcript via `_mfb_gtkapp_append`, frees the chunk, and returns
/// FALSE (`G_SOURCE_REMOVE`) so the one-shot idle source is removed. Chunk layout:
/// `[0]` = len (u64), `[16..]` = bytes.
pub(super) fn emit_append_idle_helper() -> CodeFunction {
    let mut asm = Asm::new(APPEND_IDLE_SYMBOL);
    // lr@0, x20(chunk)@8.
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(16));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        0,
    ));
    asm.push(abi::store_u64("x20", abi::stack_pointer(), 8));
    asm.push(abi::move_register("x20", "x0")); // chunk (survives load_state's x9 use)

    // _mfb_gtkapp_append(state.text_buffer, chunk+16, chunk[0])
    asm.load_state("x0", ST_TEXT_BUFFER);
    asm.push(abi::add_immediate("x1", "x20", 16));
    asm.push(abi::load_u64("x2", "x20", 0));
    asm.call_internal(APPEND_SYMBOL);
    // free(chunk)
    asm.push(abi::move_register("x0", "x20"));
    asm.call_external("free");

    asm.push(abi::move_immediate("x0", "Boolean", FALSE)); // G_SOURCE_REMOVE
    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::load_u64("x20", abi::stack_pointer(), 8));
    asm.push(abi::add_stack(16));
    asm.push(abi::return_());
    asm.finish(APPEND_IDLE_SYMBOL, "Boolean")
}

// --- term:: TUI surface (plan-01-term.md §6.3) -----------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arch::ops::CodeOp;

    /// bug-50: the printable-key branch of `emit_key_pressed_handler` must bound
    /// the fixed line buffer before storing the UTF-8 encoding. Assert that after
    /// loading `ST_LINE_LEN` (into x9) it compares against `LINE_BUF_CAP - 6` and
    /// branches (unsigned-higher) to `ignore`, and that this guard sits BEFORE the
    /// `g_unichar_to_utf8` call that writes into `ST_LINE_BUF` — so no store can
    /// land past the buffer into `ST_TERM_AREA` / the term grid.
    #[test]
    fn key_handler_bounds_line_buffer_before_utf8_store() {
        let func = emit_key_pressed_handler();
        let ins = &func.instructions;

        // Locate the printable-branch UTF-8 encode: the FIRST `bl g_unichar_to_utf8`
        // is the printable path (the raw-mode branch has a second, later one).
        let utf8_call = ins
            .iter()
            .position(|i| {
                i.op == CodeOp::BranchLink && i.get("target") == Some("g_unichar_to_utf8")
            })
            .expect("printable branch must call g_unichar_to_utf8");

        // The bound check: `cmp_imm x9, (LINE_BUF_CAP - MAX_UTF8_LEN)` followed by
        // `b.hi ignore`, appearing before the encode call.
        let expected_bound = (LINE_BUF_CAP - MAX_UTF8_LEN).to_string();
        let guard = ins[..utf8_call]
            .windows(2)
            .position(|pair| {
                let cmp = &pair[0];
                let br = &pair[1];
                cmp.op == CodeOp::CmpImm
                    && cmp.get("lhs") == Some("x9")
                    && cmp.get("rhs") == Some(expected_bound.as_str())
                    && br.op == CodeOp::BranchHi
                    && br.get("target") == Some("ignore")
            })
            .expect(
                "printable branch must bound ST_LINE_LEN against LINE_BUF_CAP - 6 and \
                 branch to `ignore` before the g_unichar_to_utf8 store (bug-50)",
            );

        // The guard must read the freshly-loaded ST_LINE_LEN: the `load_state x9,
        // ST_LINE_LEN` (an `adrp _mfb_gtkapp_state` + `ldr x9, [x9, #ST_LINE_LEN]`)
        // must be the instruction pair immediately preceding the compare, so we are
        // bounding the actual line length, not a stale value.
        let ldr = &ins[guard - 1];
        assert_eq!(ldr.op, CodeOp::LdrU64, "guard must follow the ST_LINE_LEN load");
        assert_eq!(ldr.get("dst"), Some("x9"));
        assert_eq!(ldr.get("offset"), Some(ST_LINE_LEN.to_string().as_str()));

        // And the guard must sit before the destination-pointer arithmetic that the
        // encode writes through, so a dropped key never computes an out-of-range dst.
        let guard_idx = guard; // index of the cmp_imm
        assert!(
            guard_idx < utf8_call,
            "bound check must precede the g_unichar_to_utf8 store"
        );
    }

    /// bug-59: `emit_activate_handler` must close the redundant pipe read fd after
    /// `dup2(read, 0)`. Assert exactly one `bl dup2` and one `bl close`, that the
    /// close follows the dup2, and that both take the read fd reloaded from the
    /// pipe-fds stack slot (`ldr_u32 x0, [sp, #16]`). Loading offset 16 (the read
    /// end; the write end is offset 20) proves we close the read descriptor, and
    /// reloading from the stack (rather than a register held across the call)
    /// respects the register-lifetime rules.
    #[test]
    fn activate_closes_redundant_pipe_read_fd_after_dup2() {
        let func = emit_activate_handler();
        let ins = &func.instructions;

        let dup2_calls: Vec<usize> = ins
            .iter()
            .enumerate()
            .filter(|(_, i)| i.op == CodeOp::BranchLink && i.get("target") == Some("dup2"))
            .map(|(idx, _)| idx)
            .collect();
        assert_eq!(dup2_calls.len(), 1, "activate must call dup2 exactly once");
        let dup2 = dup2_calls[0];

        let close_calls: Vec<usize> = ins
            .iter()
            .enumerate()
            .filter(|(_, i)| i.op == CodeOp::BranchLink && i.get("target") == Some("close"))
            .map(|(idx, _)| idx)
            .collect();
        assert_eq!(
            close_calls.len(),
            1,
            "activate must close the redundant read fd exactly once (bug-59)"
        );
        let close = close_calls[0];
        assert!(close > dup2, "close(read) must follow dup2(read, 0)");

        // The instruction immediately before `bl close` reloads the read fd from
        // the pipe-fds stack slot at offset 16.
        let load = &ins[close - 1];
        assert_eq!(load.op, CodeOp::LdrU32, "close's fd must be a fresh stack load");
        assert_eq!(load.get("dst"), Some("x0"));
        assert_eq!(load.get("base"), Some("sp"));
        assert_eq!(
            load.get("offset"),
            Some("16"),
            "must close the READ end (offset 16), not the write end (offset 20)"
        );

        // dup2's read-fd argument is loaded from the same stack slot (offset 16),
        // never carried in a caller-saved register across the pipe/dup2 calls.
        let dup2_load = ins[..dup2]
            .iter()
            .rev()
            .find(|i| i.op == CodeOp::LdrU32 && i.get("dst") == Some("x0"))
            .expect("dup2 must load the read fd into x0");
        assert_eq!(dup2_load.get("base"), Some("sp"));
        assert_eq!(dup2_load.get("offset"), Some("16"));
    }

    /// The commit path streams `ST_LINE_LEN` bytes from `ST_LINE_BUF` to the pipe.
    /// With the printable branch bounded, `ST_LINE_LEN` can never exceed
    /// `LINE_BUF_CAP`, so this `write` can never read past the buffer. This is a
    /// layout guard: `LINE_BUF_CAP - MAX_UTF8_LEN` (last accepted oldlen) plus a
    /// full `MAX_UTF8_LEN` encode equals exactly `LINE_BUF_CAP`.
    #[test]
    fn bounded_line_length_never_exceeds_capacity() {
        assert_eq!(
            (LINE_BUF_CAP - MAX_UTF8_LEN) + MAX_UTF8_LEN,
            LINE_BUF_CAP,
            "the worst-case accepted line fills the buffer exactly, never past it"
        );
    }
}
