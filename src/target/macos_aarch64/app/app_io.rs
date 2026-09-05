//! macOS app-mode IO ops: `emit_app_io_*` and `emit_app_term_*` emitters
//! (write/flush/input/terminal-size/set-color/attr/move/clear/cursor) (plan-11 split).

use super::*;
use crate::codegen::engine::util::Vregs;
use crate::codegen::error::constants::RESULT_ERROR_MESSAGE_REGISTER;
use crate::codegen::error::constants::RESULT_TAG_REGISTER;
use crate::codegen::error::constants::RESULT_VALUE_REGISTER;

/// App-mode append for `io.print`/`io.write`/`io.printError`/`io.writeError`
/// (plan-101 append shape). The `abi_function` member receives the MFBASIC string
/// object in the first arg register (`{u64 len; bytes}`) and returns a `Result`
/// (tag in `mfb_return(0)`). When TUI mode is active the text is written into the
/// TermView surface (plan-01-term.md §4.8); otherwise, when a transcript view is
/// attached (GUI), append to it; else (headless) write to the file descriptor.
/// `term_state_offset` is the writable term-state slot base (None when the program
/// never uses `term::`), read off the pinned arena-state register (`abi::ARENA`).
///
/// Appends its vreg stream into the caller's stream; the `abi_function` wrapper
/// builds the frame and saves the callee-saved vregs held across the objc calls
/// (the old standalone helper managed its own frame + raw x19-x22 spills — those
/// physical registers cannot appear in a vreg-finalized body, plan-34-D). The
/// values live in allocator-managed vregs so the finalizer both colors them to
/// callee-saved registers and saves them; objc args go via `mfb_arg`, results via
/// `mfb_return`, each caller-saved and consumed at its call.
pub(crate) fn emit_app_io_write(
    symbol: &str,
    stderr: bool,
    newline: bool,
    term_state_offset: Option<usize>,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let mut asm = Asm::new(symbol);
    let mut vregs = Vregs::new();
    let fd = if stderr { "2" } else { "1" };
    let v_str = vregs.next(); // MFBASIC string object (the arg), live to the end
    let v_view = vregs.next(); // transcript view / termview
    let v_tmp = vregs.next(); // class temp / allocated NSString
    let v_owned = vregs.next(); // owned NSString / marshaled SEL (held across calls)
    let v_pool = vregs.next(); // autorelease-pool token

    // Per-write autorelease pool. The worker's process-lifetime pool
    // (emit_worker_shim) is never drained, so the autoreleased NSStrings this body
    // builds for the "[stderr] " prefix and the trailing newline would accumulate
    // for the process lifetime (bug-112). Save the string arg first (poolPush
    // clobbers the arg/return registers); it survives every call in a callee-saved
    // vreg.
    asm.push(abi::move_register(&v_str, abi::mfb_arg(0))); // string object
    asm.call_external("_objc_autoreleasePoolPush", LIB_OBJC);
    asm.push(abi::move_register(&v_pool, abi::mfb_return(0))); // pool token

    // While TUI mode is active, route to the TermView surface. The term-state
    // global is reached off the pinned arena-state base (`abi::ARENA`), not a
    // repurposed callee-saved register.
    if let Some(off) = term_state_offset {
        asm.push(abi::load_u64(
            abi::SCRATCH[0],
            abi::ARENA,
            off + crate::codegen::error::constants::TERM_STATE_ACTIVE_OFFSET,
        ));
        asm.push(abi::compare_immediate(abi::SCRATCH[0], "0"));
        asm.push(abi::branch_ne("term_surface_path"));
    }

    // app = [NSApplication sharedApplication]; view = objc_getAssociatedObject(app, &KEY)
    asm.external_data(&v_view, CLASS_NS_APPLICATION, LIB_APPKIT);
    asm.load_selector(SEL_SHARED_APPLICATION.0);
    asm.push(abi::move_register(abi::mfb_arg(0), &v_view));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.local_address(abi::mfb_arg(1), ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register(&v_view, abi::mfb_return(0))); // transcript view or nil
    asm.push(abi::compare_immediate(&v_view, "0"));
    asm.push(abi::branch_eq("fd_path"));

    // --- GUI transcript path ---
    if stderr {
        // Visually distinguish stderr with a "[stderr] " marker (plan §5.4).
        build_nsstring_from_cstring_vreg(&mut asm, &v_tmp, STR_STDERR_PREFIX.0);
        asm.push(abi::move_register(abi::mfb_arg(1), abi::mfb_return(0)));
        asm.push(abi::move_register(abi::mfb_arg(0), &v_view));
        asm.call_internal(APPEND_SYMBOL);
    }
    // text = [[NSString alloc] initWithBytes:(str+8) length:str[0] encoding:UTF8]
    asm.external_data(&v_tmp, CLASS_NS_STRING, LIB_FOUNDATION);
    asm.load_selector(SEL_ALLOC.0);
    asm.push(abi::move_register(abi::mfb_arg(0), &v_tmp));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register(&v_tmp, abi::mfb_return(0))); // allocated NSString
    asm.load_selector(SEL_INIT_WITH_BYTES.0);
    asm.push(abi::add_immediate(abi::mfb_arg(2), &v_str, 8)); // bytes
    asm.push(abi::load_u64(abi::mfb_arg(3), &v_str, 0)); // length
    asm.push(abi::move_immediate(
        abi::mfb_arg(4),
        "Integer",
        NS_UTF8_ENCODING,
    ));
    asm.push(abi::move_register(abi::mfb_arg(0), &v_tmp));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register(&v_owned, abi::mfb_return(0))); // owned NSString (across append)
    asm.push(abi::move_register(abi::mfb_arg(1), abi::mfb_return(0))); // text nsstring
    asm.push(abi::move_register(abi::mfb_arg(0), &v_view));
    asm.call_internal(APPEND_SYMBOL);
    // [text release] — the NSString was created owned (alloc +
    // initWithBytes:length:encoding:, retain count 1) and _mfb_macapp_append copies
    // it into the text storage, so we hold the sole reference (bug-53).
    asm.load_selector(SEL_RELEASE.0);
    asm.push(abi::move_register(abi::mfb_arg(0), &v_owned));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    if newline {
        build_nsstring_from_cstring_vreg(&mut asm, &v_tmp, STR_NEWLINE.0);
        asm.push(abi::move_register(abi::mfb_arg(1), abi::mfb_return(0)));
        asm.push(abi::move_register(abi::mfb_arg(0), &v_view));
        asm.call_internal(APPEND_SYMBOL);
    }
    asm.push(abi::branch("done"));

    // --- headless / no-window path: write to the file descriptor ---
    asm.push(abi::label("fd_path"));
    asm.push(abi::move_immediate(abi::mfb_arg(0), "Integer", fd));
    asm.push(abi::add_immediate(abi::mfb_arg(1), &v_str, 8));
    asm.push(abi::load_u64(abi::mfb_arg(2), &v_str, 0));
    asm.call_external("_write", LIB_SYSTEM);
    if newline {
        // The trailing LF comes from the shared read-only "\n" data object, so the
        // fd path needs no writable stack scratch (the finalizer keeps stack_size
        // at 0 — the body is frame-only for lr + the callee-saved vregs).
        asm.push(abi::move_immediate(abi::mfb_arg(0), "Integer", fd));
        asm.local_address(abi::mfb_arg(1), STR_NEWLINE.0);
        asm.push(abi::move_immediate(abi::mfb_arg(2), "Integer", "1"));
        asm.call_external("_write", LIB_SYSTEM);
    }
    asm.push(abi::branch("done"));

    // --- TUI surface path: write into the TermView grid on the main thread ---
    if term_state_offset.is_some() {
        asm.push(abi::label("term_surface_path"));
        // tv = objc_getAssociatedObject([NSApplication sharedApplication], &TERMVIEW_KEY)
        asm.external_data(&v_view, CLASS_NS_APPLICATION, LIB_APPKIT);
        asm.load_selector(SEL_SHARED_APPLICATION.0);
        asm.push(abi::move_register(abi::mfb_arg(0), &v_view));
        asm.call_external("_objc_msgSend", LIB_OBJC);
        asm.local_address(abi::mfb_arg(1), TERMVIEW_ASSOC_KEY);
        asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
        asm.push(abi::move_register(&v_view, abi::mfb_return(0))); // termview or nil
        asm.push(abi::compare_immediate(&v_view, "0"));
        asm.push(abi::branch_eq("fd_path")); // headless: no surface -> fd
                                             // text = [[NSString alloc] initWithBytes:(str+8) length:str[0] encoding:UTF8]
        asm.external_data(&v_tmp, CLASS_NS_STRING, LIB_FOUNDATION);
        asm.load_selector(SEL_ALLOC.0);
        asm.push(abi::move_register(abi::mfb_arg(0), &v_tmp));
        asm.call_external("_objc_msgSend", LIB_OBJC);
        asm.push(abi::move_register(&v_tmp, abi::mfb_return(0)));
        asm.load_selector(SEL_INIT_WITH_BYTES.0);
        asm.push(abi::add_immediate(abi::mfb_arg(2), &v_str, 8));
        asm.push(abi::load_u64(abi::mfb_arg(3), &v_str, 0));
        asm.push(abi::move_immediate(
            abi::mfb_arg(4),
            "Integer",
            NS_UTF8_ENCODING,
        ));
        asm.push(abi::move_register(abi::mfb_arg(0), &v_tmp));
        asm.call_external("_objc_msgSend", LIB_OBJC);
        asm.push(abi::move_register(&v_tmp, abi::mfb_return(0))); // text nsstring
                                                                  // [tv performSelectorOnMainThread:@selector(mfbWriteString:) withObject:text waitUntilDone:YES]
        asm.load_selector(SEL_MFB_WRITE_STRING.0);
        asm.push(abi::move_register(&v_owned, abi::mfb_arg(1))); // mfbWriteString: sel
        asm.load_selector(SEL_PERFORM_ON_MAIN.0);
        asm.push(abi::move_register(abi::mfb_arg(2), &v_owned));
        asm.push(abi::move_register(abi::mfb_arg(3), &v_tmp));
        asm.push(abi::move_immediate(abi::mfb_arg(4), "Integer", "1")); // waitUntilDone: YES
        asm.push(abi::move_register(abi::mfb_arg(0), &v_view));
        asm.call_external("_objc_msgSend", LIB_OBJC);
        // [text release] — created owned; mfbWriteString: only reads its glyphs
        // (synchronous, waitUntilDone:YES) and does not retain it, so we hold the
        // sole reference and must release it (bug-53).
        asm.load_selector(SEL_RELEASE.0);
        asm.push(abi::move_register(abi::mfb_arg(0), &v_tmp));
        asm.call_external("_objc_msgSend", LIB_OBJC);
        if newline {
            build_nsstring_from_cstring_vreg(&mut asm, &v_tmp, STR_NEWLINE.0);
            asm.push(abi::move_register(&v_tmp, abi::mfb_return(0))); // "\n" nsstring
            asm.load_selector(SEL_MFB_WRITE_STRING.0);
            asm.push(abi::move_register(&v_owned, abi::mfb_arg(1)));
            asm.load_selector(SEL_PERFORM_ON_MAIN.0);
            asm.push(abi::move_register(abi::mfb_arg(2), &v_owned));
            asm.push(abi::move_register(abi::mfb_arg(3), &v_tmp));
            asm.push(abi::move_immediate(abi::mfb_arg(4), "Integer", "1"));
            asm.push(abi::move_register(abi::mfb_arg(0), &v_view));
            asm.call_external("_objc_msgSend", LIB_OBJC);
        }
    }

    asm.push(abi::label("done"));
    // Drain this write's autoreleased NSStrings, then return OK (poolPop clobbers
    // the arg/return registers). Every path here returns RESULT_OK_TAG.
    asm.push(abi::move_register(abi::mfb_arg(0), &v_pool)); // pool token
    asm.call_external("_objc_autoreleasePoolPop", LIB_OBJC);
    asm.push(abi::move_immediate(abi::mfb_return(0), "Integer", "0")); // RESULT_OK_TAG
    asm.push(abi::return_());

    instructions.extend(asm.ins);
    relocations.extend(asm.rel);
}

/// App-mode body for `io.flush`. Transcript writes are already synchronous (see
/// [`emit_append_helper`]), but in TUI mode grid writes are retained and only
/// presented on demand, so `io::flush` drives the same coalesced present as
/// `term::sync` — a marshaled `setNeedsDisplay:` on the TermView (plan-35-D §3).
/// Headless / no-surface runs skip the present and return OK.
pub(crate) fn emit_app_io_flush(
    symbol: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    // Append shape (plan-101): no own frame — the `abi_function` vreg finalizer
    // builds the frame and saves the `LOCAL` callee-saved regs held across the
    // objc calls in `emit_present_needs_display`. io epilogue returns OK.
    let mut asm = Asm::new(symbol);
    let mut vregs = Vregs::new();
    emit_present_needs_display_vreg(&mut asm, &mut vregs, "flush_done");
    asm.push(abi::label("flush_done"));
    asm.push(abi::move_immediate(abi::mfb_return(0), "Integer", "0")); // RESULT_OK_TAG
    asm.push(abi::return_());
    instructions.extend(asm.ins);
    relocations.extend(asm.rel);
}

/// App-mode append for `io.input` (plan §5.4, plan-101 append shape): render the
/// prompt to the transcript via the `io.write` helper, then read a committed line
/// via the `io.readLine` helper (which reads fd 0 — the window input pipe in app
/// mode). The prompt string is already in the first arg register on entry;
/// `io.readLine` takes no arguments, so its result becomes this body's result.
/// Nothing is live across the two internal calls, so no vregs are needed — the
/// `abi_function` wrapper builds the frame and saves lr.
pub(crate) fn emit_app_io_input(
    symbol: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let mut asm = Asm::new(symbol);
    asm.call_internal(IO_WRITE_SYMBOL); // arg0 = prompt; renders it, result ignored
    emit_set_input_mode_instructions(&mut asm, INPUT_MODE_LINE_ECHO);
    asm.call_internal(IO_READ_LINE_SYMBOL); // result in the return registers
    asm.push(abi::return_());
    instructions.extend(asm.ins);
    relocations.extend(asm.rel);
}

pub(crate) fn emit_set_raw_input_mode(
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    from: &str,
) {
    let mut asm = Asm::new(from);
    emit_set_input_mode_instructions(&mut asm, INPUT_MODE_RAW_NO_ECHO);
    instructions.extend(asm.ins);
    relocations.extend(asm.rel);
}

fn emit_set_input_mode_instructions(asm: &mut Asm, mode: &str) {
    // C-call argument staging is spelled with role tokens, not physical
    // registers: this sequence is also injected into shared helper bodies
    // (`io_helpers::lower_io_read_char_helper` via `emit_set_raw_input_mode`),
    // which the plan-34-D stream guard requires to be token-pure. The tokens
    // realize to the same x0–x3 at the selection seam.
    asm.load_selector(SEL_SHARED_APPLICATION.0);
    asm.external_data(abi::mfb_arg(0), CLASS_NS_APPLICATION, LIB_APPKIT);
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.local_address(abi::mfb_arg(1), INPUT_MODE_KEY);
    asm.push(abi::move_immediate(abi::mfb_arg(2), "Integer", mode));
    asm.push(abi::move_immediate(abi::mfb_arg(3), "Integer", "0")); // OBJC_ASSOCIATION_ASSIGN
    asm.call_external("_objc_setAssociatedObject", LIB_OBJC);
}

/// App-mode body for `io.isInputTerminal`/`io.isOutputTerminal`/
/// `io.isErrorTerminal` (plan §5.4): the window is the interactive console, so
/// all three return `OK(TRUE)`. Result ABI: x0 = tag (0 = ok), x1 = value.
/// App-mode `io.is*Terminal`: the window is the interactive console, so return
/// TRUE. Appends into the caller's vreg stream (plan-101 append shape); the
/// `abi_function` wrapper finalizes.
pub(crate) fn emit_app_io_is_terminal(
    symbol: &str,
    instructions: &mut Vec<CodeInstruction>,
    _relocations: &mut Vec<CodeRelocation>,
) {
    let _ = symbol;
    // Result registers via the abstract ABI tokens (`mfb_return(1)` = value,
    // `mfb_return(0)` = tag) — NOT raw `x1`/`x0`, so the `abi_function` vreg
    // finalizer accepts the appended stream (plan-101).
    instructions.push(abi::move_immediate(abi::mfb_return(1), "Boolean", "1")); // value = TRUE
    instructions.push(abi::move_immediate(abi::mfb_return(0), "Integer", "0")); // tag = OK
    instructions.push(abi::return_());
}

/// Store an immediate into a term-state-global slot reached off the pinned
/// arena-state register (plan-01-term.md §6.2).
fn store_term_state(asm: &mut Asm, term_state_offset: usize, field_offset: usize, value: &str) {
    asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", value));
    asm.push(abi::store_u64(
        abi::SCRATCH[0],
        TERM_ARENA_STATE_REG,
        term_state_offset + field_offset,
    ));
}

/// App-mode body for `term::on` (plan-01-term.md §4.2 / §6.3). Resets the
/// term-state global to its defaults, then — when a window is attached (GUI) —
/// clears the TermView grid and swaps it in as the window content view on the
/// main thread. Headless runs (no window) update only the state global so
/// `isOn`/auto-restore stay correct.
pub(crate) fn emit_app_term_on_helper(
    symbol: &str,
    term_state_offset: usize,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let mut asm = Asm::new(symbol);
    let mut vregs = Vregs::new();
    // Values parked across the objc/internal calls (app, window, termview, the
    // marshaled selector) live in allocator vregs so the `abi_function` finalizer
    // colors them callee-saved AND saves them (plan-101 append shape).
    let v_app = vregs.next();
    let v_window = vregs.next();
    let v_termview = vregs.next();
    let v_sel = vregs.next();

    // Reset all term state to defaults (active on, fg white, bg black, bold and
    // underline off, cursor visible). x19 is the pinned arena-state base.
    store_term_state(
        &mut asm,
        term_state_offset,
        crate::codegen::error::constants::TERM_STATE_ACTIVE_OFFSET,
        "1",
    );
    store_term_state(
        &mut asm,
        term_state_offset,
        crate::codegen::error::constants::TERM_STATE_FG_OFFSET,
        "16777215",
    );
    store_term_state(
        &mut asm,
        term_state_offset,
        crate::codegen::error::constants::TERM_STATE_BG_OFFSET,
        "0",
    );
    store_term_state(
        &mut asm,
        term_state_offset,
        crate::codegen::error::constants::TERM_STATE_BOLD_OFFSET,
        "0",
    );
    store_term_state(
        &mut asm,
        term_state_offset,
        crate::codegen::error::constants::TERM_STATE_UNDERLINE_OFFSET,
        "0",
    );
    store_term_state(
        &mut asm,
        term_state_offset,
        crate::codegen::error::constants::TERM_STATE_CURSOR_VISIBLE_OFFSET,
        "1",
    );

    // bug-150: entering TUI mode flips the window into immediate single-key
    // delivery once, from the moment `term::on` runs — set INPUT_MODE_KEY =
    // RAW_NO_ECHO so both keyDown IMPs (transcript `_mfb_macapp_key_down` and TUI
    // `_mfb_macapp_term_keyDown`) route each keystroke straight to the input pipe
    // instead of buffering until Return. The initial mode is nil (0) at startup;
    // this is the one-time flip. `io::input`/`io::readLine` still switch to
    // LINE_ECHO for their own read (emit_app_io_input).
    emit_set_input_mode_instructions(&mut asm, INPUT_MODE_RAW_NO_ECHO);

    // app = [NSApplication sharedApplication]
    asm.external_data(&v_app, CLASS_NS_APPLICATION, LIB_APPKIT);
    asm.load_selector(SEL_SHARED_APPLICATION.0);
    asm.push(abi::move_register(abi::mfb_arg(0), &v_app));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register(&v_app, abi::mfb_return(0))); // app

    // window = objc_getAssociatedObject(app, &WINDOW_ASSOC_KEY); nil -> headless.
    asm.push(abi::move_register(abi::mfb_arg(0), &v_app));
    asm.local_address(abi::mfb_arg(1), WINDOW_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register(&v_window, abi::mfb_return(0))); // window or nil
    asm.push(abi::compare_immediate(&v_window, "0"));
    asm.push(abi::branch_eq("term_on_done"));

    // termview = objc_getAssociatedObject(app, &TERMVIEW_ASSOC_KEY)
    asm.push(abi::move_register(abi::mfb_arg(0), &v_app));
    asm.local_address(abi::mfb_arg(1), TERMVIEW_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register(&v_termview, abi::mfb_return(0))); // termview

    // Clear the grid + home the cursor before the surface is displayed.
    asm.push(abi::move_register(abi::mfb_arg(0), &v_termview));
    asm.call_internal(TERM_CLEAR_SYMBOL);

    // [window performSelectorOnMainThread:@selector(setContentView:)
    //         withObject:termview waitUntilDone:YES]  (AppKit is main-thread only)
    asm.load_selector(SEL_SET_CONTENT_VIEW.0);
    asm.push(abi::move_register(&v_sel, abi::mfb_arg(1))); // setContentView: sel
    asm.load_selector(SEL_PERFORM_ON_MAIN.0);
    asm.push(abi::move_register(abi::mfb_arg(2), &v_sel));
    asm.push(abi::move_register(abi::mfb_arg(3), &v_termview));
    asm.push(abi::move_immediate(abi::mfb_arg(4), "Integer", "1")); // waitUntilDone: YES
    asm.push(abi::move_register(abi::mfb_arg(0), &v_window));
    asm.call_external("_objc_msgSend", LIB_OBJC);

    // [window performSelectorOnMainThread:@selector(makeFirstResponder:)
    //         withObject:termview waitUntilDone:YES] — route keys to the surface.
    asm.load_selector(SEL_MAKE_FIRST_RESPONDER.0);
    asm.push(abi::move_register(&v_sel, abi::mfb_arg(1)));
    asm.load_selector(SEL_PERFORM_ON_MAIN.0);
    asm.push(abi::move_register(abi::mfb_arg(2), &v_sel));
    asm.push(abi::move_register(abi::mfb_arg(3), &v_termview)); // termview
    asm.push(abi::move_immediate(abi::mfb_arg(4), "Integer", "1"));
    asm.push(abi::move_register(abi::mfb_arg(0), &v_window)); // window
    asm.call_external("_objc_msgSend", LIB_OBJC);

    asm.push(abi::label("term_on_done"));
    asm.push(abi::move_immediate(RESULT_TAG_REGISTER, "Integer", "0")); // RESULT_OK_TAG
    asm.push(abi::return_());
    instructions.extend(asm.ins);
    relocations.extend(asm.rel);
}

/// App-mode body for `term::off` (plan-01-term.md §4.2 / §6.3). No-op when
/// already off; otherwise restores the transcript scroll view as the window
/// content view on the main thread (GUI) and clears the active flag. Headless
/// runs update only the state global.
pub(crate) fn emit_app_term_off_helper(
    symbol: &str,
    term_state_offset: usize,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let mut asm = Asm::new(symbol);
    let mut vregs = Vregs::new();
    // app / window survive to the end; v_view holds termview/scroll/transcript in
    // turn across the marshal calls, v_sel the marshaled selector — all in
    // allocator vregs so the `abi_function` finalizer saves them (plan-101).
    let v_app = vregs.next();
    let v_window = vregs.next();
    let v_view = vregs.next();
    let v_sel = vregs.next();

    // Gate: already off -> no-op (plan §4.2). x19 is the pinned arena-state base.
    asm.push(abi::load_u64(
        abi::SCRATCH[0],
        TERM_ARENA_STATE_REG,
        term_state_offset + crate::codegen::error::constants::TERM_STATE_ACTIVE_OFFSET,
    ));
    asm.push(abi::compare_immediate(abi::SCRATCH[0], "0"));
    asm.push(abi::branch_eq("term_off_done"));

    // app = [NSApplication sharedApplication]
    asm.external_data(&v_app, CLASS_NS_APPLICATION, LIB_APPKIT);
    asm.load_selector(SEL_SHARED_APPLICATION.0);
    asm.push(abi::move_register(abi::mfb_arg(0), &v_app));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register(&v_app, abi::mfb_return(0))); // app

    // window = objc_getAssociatedObject(app, &WINDOW_ASSOC_KEY); nil -> headless.
    asm.push(abi::move_register(abi::mfb_arg(0), &v_app));
    asm.local_address(abi::mfb_arg(1), WINDOW_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register(&v_window, abi::mfb_return(0))); // window or nil
    asm.push(abi::compare_immediate(&v_window, "0"));
    asm.push(abi::branch_eq("term_off_inactive"));

    // Final present (plan-35-D §3): force the TermView to draw synchronously
    // before the content-view swap, so the last drawn frame is shown (the
    // mandatory-present contract — a program that draws then `term::off`s without
    // a trailing `term::sync` still shows its final frame). `display` marks the
    // whole view dirty and repaints it immediately; marshaled waitUntilDone:YES so
    // it completes before the transcript swap below.
    asm.push(abi::move_register(abi::mfb_arg(0), &v_app)); // app
    asm.local_address(abi::mfb_arg(1), TERMVIEW_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register(&v_view, abi::mfb_return(0))); // termview or nil
    asm.push(abi::compare_immediate(&v_view, "0"));
    asm.push(abi::branch_eq("term_off_presented"));
    asm.load_selector(SEL_DISPLAY.0);
    asm.push(abi::move_register(&v_sel, abi::mfb_arg(1))); // display sel
    asm.load_selector(SEL_PERFORM_ON_MAIN.0);
    asm.push(abi::move_register(abi::mfb_arg(2), &v_sel));
    asm.push(abi::move_immediate(abi::mfb_arg(3), "Integer", "0")); // withObject: nil (display takes no arg)
    asm.push(abi::move_immediate(abi::mfb_arg(4), "Integer", "1")); // waitUntilDone: YES
    asm.push(abi::move_register(abi::mfb_arg(0), &v_view));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::label("term_off_presented"));

    // scroll = objc_getAssociatedObject(app, &SCROLLVIEW_ASSOC_KEY)
    asm.push(abi::move_register(abi::mfb_arg(0), &v_app));
    asm.local_address(abi::mfb_arg(1), SCROLLVIEW_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register(&v_view, abi::mfb_return(0))); // scroll view

    // [window performSelectorOnMainThread:@selector(setContentView:)
    //         withObject:scrollView waitUntilDone:YES]
    asm.load_selector(SEL_SET_CONTENT_VIEW.0);
    asm.push(abi::move_register(&v_sel, abi::mfb_arg(1)));
    asm.load_selector(SEL_PERFORM_ON_MAIN.0);
    asm.push(abi::move_register(abi::mfb_arg(2), &v_sel));
    asm.push(abi::move_register(abi::mfb_arg(3), &v_view));
    asm.push(abi::move_immediate(abi::mfb_arg(4), "Integer", "1")); // waitUntilDone: YES
    asm.push(abi::move_register(abi::mfb_arg(0), &v_window));
    asm.call_external("_objc_msgSend", LIB_OBJC);

    // Restore the transcript as first responder so window input returns to it.
    asm.push(abi::move_register(abi::mfb_arg(0), &v_app)); // app
    asm.local_address(abi::mfb_arg(1), ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register(&v_view, abi::mfb_return(0))); // transcript view
    asm.load_selector(SEL_MAKE_FIRST_RESPONDER.0);
    asm.push(abi::move_register(&v_sel, abi::mfb_arg(1)));
    asm.load_selector(SEL_PERFORM_ON_MAIN.0);
    asm.push(abi::move_register(abi::mfb_arg(2), &v_sel));
    asm.push(abi::move_register(abi::mfb_arg(3), &v_view));
    asm.push(abi::move_immediate(abi::mfb_arg(4), "Integer", "1"));
    asm.push(abi::move_register(abi::mfb_arg(0), &v_window)); // window
    asm.call_external("_objc_msgSend", LIB_OBJC);

    asm.push(abi::label("term_off_inactive"));
    // bug-150: leaving TUI mode returns the window to line input so subsequent
    // reads commit on Return again (symmetric with the console `term::off`
    // cooked-mode restore).
    emit_set_input_mode_instructions(&mut asm, INPUT_MODE_LINE_ECHO);
    store_term_state(
        &mut asm,
        term_state_offset,
        crate::codegen::error::constants::TERM_STATE_ACTIVE_OFFSET,
        "0",
    );

    asm.push(abi::label("term_off_done"));
    asm.push(abi::move_immediate(RESULT_TAG_REGISTER, "Integer", "0")); // RESULT_OK_TAG
    asm.push(abi::return_());
    instructions.extend(asm.ins);
    relocations.extend(asm.rel);
}

/// App-mode dispatcher for the `term::` runtime helpers (plan-01-term.md §6.3,
/// Phase 5). Returns `None` for calls that keep the shared console backend
/// (`isOn` and the attribute getters, which read the term-state global the app
/// setters keep updated).
pub(crate) fn emit_app_term_helper(
    call: &str,
    symbol: &str,
    term_state_offset: usize,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Option<Result<(), String>> {
    match call {
        "term.on" => emit_app_term_on_helper(symbol, term_state_offset, instructions, relocations),
        "term.off" => {
            emit_app_term_off_helper(symbol, term_state_offset, instructions, relocations)
        }
        "term.setForeground" => emit_app_set_color(
            symbol,
            term_state_offset,
            crate::codegen::error::constants::TERM_STATE_FG_OFFSET,
            TV_CUR_FG_OFFSET,
            instructions,
            relocations,
        ),
        "term.setBackground" => emit_app_set_color(
            symbol,
            term_state_offset,
            crate::codegen::error::constants::TERM_STATE_BG_OFFSET,
            TV_CUR_BG_OFFSET,
            instructions,
            relocations,
        ),
        "term.setBold" => emit_app_set_attr(
            symbol,
            term_state_offset,
            crate::codegen::error::constants::TERM_STATE_BOLD_OFFSET,
            TV_CUR_BOLD_OFFSET,
            instructions,
            relocations,
        ),
        "term.setUnderline" => emit_app_set_attr(
            symbol,
            term_state_offset,
            crate::codegen::error::constants::TERM_STATE_UNDERLINE_OFFSET,
            TV_CUR_UNDERLINE_OFFSET,
            instructions,
            relocations,
        ),
        "term.moveTo" => emit_app_move_to(symbol, term_state_offset, instructions, relocations),
        "term.drawHLine" => {
            emit_app_draw_line(symbol, term_state_offset, true, instructions, relocations)
        }
        "term.drawVLine" => {
            emit_app_draw_line(symbol, term_state_offset, false, instructions, relocations)
        }
        "term.drawBox" => emit_app_draw_box(symbol, term_state_offset, instructions, relocations),
        "term.fillRect" => emit_app_fill_rect(symbol, term_state_offset, instructions, relocations),
        "term.drawGlyph" => {
            emit_app_draw_glyph(symbol, term_state_offset, instructions, relocations)
        }
        "term.drawText" => emit_app_draw_text(symbol, term_state_offset, instructions, relocations),
        "term.clear" => emit_app_clear(symbol, term_state_offset, instructions, relocations),
        "term.sync" => emit_app_term_sync(symbol, term_state_offset, instructions, relocations),
        "term.showCursor" => {
            emit_app_set_cursor_visible(symbol, term_state_offset, "1", instructions, relocations)
        }
        "term.hideCursor" => {
            emit_app_set_cursor_visible(symbol, term_state_offset, "0", instructions, relocations)
        }
        "term.terminalSize" => {
            emit_app_terminal_size(symbol, term_state_offset, instructions, relocations)
        }
        "term.didResize" => emit_app_did_resize(symbol, instructions, relocations),
        _ => return None,
    }
    Some(Ok(()))
}

/// `term::didResize()` app arm (planning/term.md #11): OK(Boolean) = the cached
/// resize flag on TVSTATE, read-and-cleared so it latches from a genuine window
/// resize (set by the `setFrameSize:` IMP) until the program observes it. Headless
/// (no attached surface) reads false. Result ABI x0=tag, x1=value.
fn emit_app_did_resize(
    symbol: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let mut asm = Asm::new(symbol);
    let mut vregs = Vregs::new();
    let v_state = vregs.next(); // TVSTATE ptr, held across emit_get_tv_state's calls
    let false_label = format!("{symbol}_false");
    let done = format!("{symbol}_done");
    // Load TVSTATE (clobbers x0/x1, not x19); nil (headless) → false.
    emit_get_tv_state(&mut asm, &v_state, &false_label);
    // value = TVSTATE.didResize, then clear it (read-and-clear).
    asm.push(abi::load_u64(
        RESULT_VALUE_REGISTER,
        &v_state,
        TV_DID_RESIZE_OFFSET,
    ));
    asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
    asm.push(abi::store_u64(
        abi::SCRATCH[0],
        &v_state,
        TV_DID_RESIZE_OFFSET,
    ));
    asm.push(abi::move_immediate(RESULT_TAG_REGISTER, "Integer", "0")); // OK tag
    asm.push(abi::branch(&done));
    asm.push(abi::label(&false_label));
    asm.push(abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "0")); // false
    asm.push(abi::move_immediate(RESULT_TAG_REGISTER, "Integer", "0")); // OK tag
    asm.push(abi::label(&done));
    asm.push(abi::return_());
    instructions.extend(asm.ins);
    relocations.extend(asm.rel);
}

/// `term::sync()` app arm (plan-35-D §3). The single present: marshal a
/// `setNeedsDisplay:` onto the TermView so the coalesced frame is drawn once. A
/// clean no-op while TUI mode is off (the active gate) or when no surface is
/// attached (headless). This is the *only* redraw trigger for grid writes —
/// `mfbWriteString:`/`clear` no longer request their own redraw (mandatory
/// present, plan-35 D1).
fn emit_app_term_sync(
    symbol: &str,
    term_state_offset: usize,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let mut asm = Asm::new(symbol);
    let mut vregs = Vregs::new();
    let done = format!("{symbol}_done");
    emit_term_active_gate(&mut asm, term_state_offset, &done);
    // The finalized (vreg) present twin — the `LOCAL` version is not saved by the
    // `abi_function` vreg finalizer, so a value parked there would be clobbered
    // across the objc calls (plan-101).
    emit_present_needs_display_vreg(&mut asm, &mut vregs, &done);
    asm.push(abi::label(&done));
    asm.push(abi::move_immediate(RESULT_TAG_REGISTER, "Integer", "0")); // RESULT_OK_TAG
    asm.push(abi::return_());
    instructions.extend(asm.ins);
    relocations.extend(asm.rel);
}

/// Vreg-based present (plan-101): the coalesced single `setNeedsDisplay:` redraw
/// shared by `term::sync` and io's `abi_function` `flush` body. Identical
/// objc sequence, with the values held across the objc calls in allocator-managed
/// vregs so the `abi_function` finalizer colors them callee-saved AND saves them
/// (a raw `LOCAL` token is not saved by that finalizer, so it would be clobbered
/// across the call).
fn emit_present_needs_display_vreg(asm: &mut Asm, vregs: &mut Vregs, done: &str) {
    // tv = objc_getAssociatedObject([NSApplication sharedApplication], &TERMVIEW_KEY)
    // Values held across the objc calls (termView, the setNeedsDisplay SEL) go in
    // ALLOCATOR-MANAGED vregs — the `abi_function` finalizer colors them to
    // callee-saved registers AND saves them in the frame. (Raw `LOCAL` tokens are
    // NOT saved by the finalizer, so they'd be clobbered across the call — the
    // plan-101 append correctness fix.) objc args via `mfb_arg`, returns via
    // `mfb_return`; both caller-saved and consumed at each call.
    let tv = vregs.next();
    let sel = vregs.next();
    asm.external_data(&tv, CLASS_NS_APPLICATION, LIB_APPKIT);
    asm.load_selector(SEL_SHARED_APPLICATION.0);
    asm.push(abi::move_register(abi::mfb_arg(0), &tv));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.local_address(abi::mfb_arg(1), TERMVIEW_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register(&tv, abi::mfb_return(0))); // termView or nil
    asm.push(abi::compare_immediate(&tv, "0"));
    asm.push(abi::branch_eq(done));
    // [tv performSelectorOnMainThread:@selector(setNeedsDisplay:) withObject:tv
    //  waitUntilDone:YES] — any non-nil withObject reads as BOOL YES.
    asm.load_selector(SEL_SET_NEEDS_DISPLAY.0);
    asm.push(abi::move_register(&sel, abi::mfb_arg(1)));
    asm.load_selector(SEL_PERFORM_ON_MAIN.0);
    asm.push(abi::move_register(abi::mfb_arg(2), &sel));
    asm.push(abi::move_register(abi::mfb_arg(3), &tv));
    asm.push(abi::move_immediate(abi::mfb_arg(4), "Integer", "1")); // waitUntilDone: YES
    asm.push(abi::move_register(abi::mfb_arg(0), &tv));
    asm.call_external("_objc_msgSend", LIB_OBJC);
}

/// Branch to `done` when TUI mode is inactive (the §4.2.1 no-op gate). `x19` is
/// the pinned arena-state base holding the term-state global.
fn emit_term_active_gate(asm: &mut Asm, term_state_offset: usize, done: &str) {
    asm.push(abi::load_u64(
        abi::SCRATCH[0],
        TERM_ARENA_STATE_REG,
        term_state_offset + crate::codegen::error::constants::TERM_STATE_ACTIVE_OFFSET,
    ));
    asm.push(abi::compare_immediate(abi::SCRATCH[0], "0"));
    asm.push(abi::branch_eq(done));
}

/// Load the TermView's grid-state struct into `state_reg` (an allocator vreg the
/// caller supplies); branch to `nil_label` when no surface is attached (headless).
/// Clobbers the objc call-clobbered registers, but not `x19` (the arena-state
/// base). objc staging uses role tokens (`mfb_arg`/`mfb_return`, byte-identical to
/// x0/x1 on aarch64) so the `abi_function` vreg finalizer accepts the stream.
fn emit_get_tv_state(asm: &mut Asm, state_reg: &str, nil_label: &str) {
    asm.external_data(state_reg, CLASS_NS_APPLICATION, LIB_APPKIT);
    asm.load_selector(SEL_SHARED_APPLICATION.0);
    asm.push(abi::move_register(abi::mfb_arg(0), state_reg));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.local_address(abi::mfb_arg(1), TERMVIEW_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::compare_immediate(abi::mfb_return(0), "0"));
    asm.push(abi::branch_eq(nil_label));
    asm.local_address(abi::mfb_arg(1), TVSTATE_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register(state_reg, abi::mfb_return(0)));
    asm.push(abi::compare_immediate(state_reg, "0"));
    asm.push(abi::branch_eq(nil_label));
}

/// `term::setForeground`/`setBackground` app body: pack r/g/b and store it to the
/// term-state global (so the console-backed getters stay correct) and to the
/// TermView's current-attribute field (so the write path tags cells with it).
fn emit_app_set_color(
    symbol: &str,
    term_state_offset: usize,
    global_field: usize,
    tv_field: usize,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let mut asm = Asm::new(symbol);
    let mut vregs = Vregs::new();
    let v_packed = vregs.next(); // packed r|g|b, held across emit_get_tv_state's calls
    let v_state = vregs.next(); // TVSTATE ptr
    let done = format!("{symbol}_done");
    emit_term_active_gate(&mut asm, term_state_offset, &done);
    // packed = r | g<<8 | b<<16 (r/g/b arrive in mfb_arg(0..2); consumed before any
    // call, so they need no vreg — only the packed result crosses the objc calls).
    asm.push(abi::move_register(abi::SCRATCH[0], abi::mfb_arg(0))); // r
    asm.push(abi::shift_left_immediate(
        abi::SCRATCH[1],
        abi::mfb_arg(1),
        8,
    )); // g<<8
    asm.push(abi::shift_left_immediate(
        abi::SCRATCH[2],
        abi::mfb_arg(2),
        16,
    )); // b<<16
    asm.push(abi::or_registers(
        abi::SCRATCH[0],
        abi::SCRATCH[0],
        abi::SCRATCH[1],
    ));
    asm.push(abi::or_registers(
        abi::SCRATCH[0],
        abi::SCRATCH[0],
        abi::SCRATCH[2],
    ));
    asm.push(abi::store_u64(
        abi::SCRATCH[0],
        TERM_ARENA_STATE_REG,
        term_state_offset + global_field,
    ));
    asm.push(abi::move_register(&v_packed, abi::SCRATCH[0])); // keep packed across the call
    emit_get_tv_state(&mut asm, &v_state, &done);
    asm.push(abi::store_u64(&v_packed, &v_state, tv_field));
    asm.push(abi::label(&done));
    asm.push(abi::move_immediate(RESULT_TAG_REGISTER, "Integer", "0")); // RESULT_OK_TAG
    asm.push(abi::return_());
    instructions.extend(asm.ins);
    relocations.extend(asm.rel);
}

/// `term::setBold`/`setUnderline` app body: store the flag to the term-state
/// global and the TermView current-attribute field.
fn emit_app_set_attr(
    symbol: &str,
    term_state_offset: usize,
    global_field: usize,
    tv_field: usize,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let mut asm = Asm::new(symbol);
    let mut vregs = Vregs::new();
    let v_enabled = vregs.next(); // the flag, held across emit_get_tv_state's calls
    let v_state = vregs.next(); // TVSTATE ptr
    let done = format!("{symbol}_done");
    // `enabled` arrives in mfb_arg(0); park it in a vreg so it survives the objc
    // calls in emit_get_tv_state.
    asm.push(abi::move_register(&v_enabled, abi::mfb_arg(0)));
    emit_term_active_gate(&mut asm, term_state_offset, &done);
    asm.push(abi::store_u64(
        &v_enabled,
        TERM_ARENA_STATE_REG,
        term_state_offset + global_field,
    ));
    emit_get_tv_state(&mut asm, &v_state, &done);
    asm.push(abi::store_u64(&v_enabled, &v_state, tv_field));
    asm.push(abi::label(&done));
    asm.push(abi::move_immediate(RESULT_TAG_REGISTER, "Integer", "0")); // RESULT_OK_TAG
    asm.push(abi::return_());
    instructions.extend(asm.ins);
    relocations.extend(asm.rel);
}

/// `term::moveTo(row, col)` app body: clamp to `[0, rows-1] x [0, cols-1]` and
/// store into the TermView cursor (plan §4.5).
fn emit_app_move_to(
    symbol: &str,
    term_state_offset: usize,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let mut asm = Asm::new(symbol);
    let mut vregs = Vregs::new();
    // row/col arrive in mfb_arg(0)/mfb_arg(1) and are read AFTER emit_get_tv_state's
    // objc calls, so they live in vregs; v_state holds the TVSTATE ptr.
    let v_row = vregs.next();
    let v_col = vregs.next();
    let v_state = vregs.next();
    let done = format!("{symbol}_done");
    asm.push(abi::move_register(&v_row, abi::mfb_arg(0)));
    asm.push(abi::move_register(&v_col, abi::mfb_arg(1)));
    emit_term_active_gate(&mut asm, term_state_offset, &done);
    emit_get_tv_state(&mut asm, &v_state, &done);
    // row = clamp(row, 0, rows-1)
    asm.push(abi::move_register(abi::SCRATCH[0], &v_row));
    let row_lo = format!("{symbol}_row_lo");
    let row_hi = format!("{symbol}_row_hi");
    asm.push(abi::compare_immediate(abi::SCRATCH[0], "0"));
    asm.push(abi::branch_ge(&row_lo));
    asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
    asm.push(abi::label(&row_lo));
    asm.push(abi::load_u64(abi::SCRATCH[1], &v_state, TV_ROWS_OFFSET));
    asm.push(abi::subtract_immediate(abi::SCRATCH[1], abi::SCRATCH[1], 1));
    asm.push(abi::compare_registers(abi::SCRATCH[0], abi::SCRATCH[1]));
    asm.push(abi::branch_le(&row_hi));
    asm.push(abi::move_register(abi::SCRATCH[0], abi::SCRATCH[1]));
    asm.push(abi::label(&row_hi));
    asm.push(abi::store_u64(
        abi::SCRATCH[0],
        &v_state,
        TV_CURSOR_ROW_OFFSET,
    ));
    // col = clamp(col, 0, cols-1)
    asm.push(abi::move_register(abi::SCRATCH[0], &v_col));
    let col_lo = format!("{symbol}_col_lo");
    let col_hi = format!("{symbol}_col_hi");
    asm.push(abi::compare_immediate(abi::SCRATCH[0], "0"));
    asm.push(abi::branch_ge(&col_lo));
    asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
    asm.push(abi::label(&col_lo));
    asm.push(abi::load_u64(abi::SCRATCH[1], &v_state, TV_COLS_OFFSET));
    asm.push(abi::subtract_immediate(abi::SCRATCH[1], abi::SCRATCH[1], 1));
    asm.push(abi::compare_registers(abi::SCRATCH[0], abi::SCRATCH[1]));
    asm.push(abi::branch_le(&col_hi));
    asm.push(abi::move_register(abi::SCRATCH[0], abi::SCRATCH[1]));
    asm.push(abi::label(&col_hi));
    asm.push(abi::store_u64(
        abi::SCRATCH[0],
        &v_state,
        TV_CURSOR_COL_OFFSET,
    ));
    asm.push(abi::label(&done));
    asm.push(abi::move_immediate(RESULT_TAG_REGISTER, "Integer", "0")); // RESULT_OK_TAG
    asm.push(abi::return_());
    instructions.extend(asm.ins);
    relocations.extend(asm.rel);
}

/// `term::clear` app body: clear the grid + home the cursor (worker side). The
/// surface is repainted only on the next present (`term::sync`/`io::flush`), not
/// per clear — redraw is present-driven (plan-35-D §3, mandatory present).
fn emit_app_clear(
    symbol: &str,
    term_state_offset: usize,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let mut asm = Asm::new(symbol);
    let mut vregs = Vregs::new();
    let v_tv = vregs.next(); // termView, held across the marshal call
    let v_sel = vregs.next(); // mfbClear: sel, held across load_selector's bl
    let done = format!("{symbol}_done");
    emit_term_active_gate(&mut asm, term_state_offset, &done);
    // tv = objc_getAssociatedObject([NSApplication sharedApplication], &TERMVIEW_KEY)
    asm.external_data(&v_tv, CLASS_NS_APPLICATION, LIB_APPKIT);
    asm.load_selector(SEL_SHARED_APPLICATION.0);
    asm.push(abi::move_register(abi::mfb_arg(0), &v_tv));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.local_address(abi::mfb_arg(1), TERMVIEW_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register(&v_tv, abi::mfb_return(0))); // termView or nil
    asm.push(abi::compare_immediate(&v_tv, "0"));
    asm.push(abi::branch_eq(&done));
    // Marshal the grid clear onto the main thread (bug-165): the cell buffer is
    // realloc/free'd by `setFrameSize:` on the main thread during a live window
    // resize, so mutating it directly from the worker is a use-after-free. Run it
    // through the `mfbClear:` selector like `mfbWriteString:` does — no redraw, the
    // repaint is present-driven (plan-35-D §3).
    // [tv performSelectorOnMainThread:@selector(mfbClear:) withObject:nil waitUntilDone:YES]
    asm.load_selector(SEL_MFB_CLEAR.0);
    asm.push(abi::move_register(&v_sel, abi::mfb_arg(1))); // mfbClear: sel
    asm.load_selector(SEL_PERFORM_ON_MAIN.0);
    asm.push(abi::move_register(abi::mfb_arg(2), &v_sel));
    asm.push(abi::move_immediate(abi::mfb_arg(3), "Integer", "0")); // withObject: nil
    asm.push(abi::move_immediate(abi::mfb_arg(4), "Integer", "1")); // waitUntilDone: YES
    asm.push(abi::move_register(abi::mfb_arg(0), &v_tv));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::label(&done));
    asm.push(abi::move_immediate(RESULT_TAG_REGISTER, "Integer", "0")); // RESULT_OK_TAG
    asm.push(abi::return_());
    instructions.extend(asm.ins);
    relocations.extend(asm.rel);
}

/// `term::drawHLine`/`drawVLine` app body: resolve the `LineStyle` ordinal to a
/// `unichar` glyph, park it plus the fixed line and span endpoints in the TermView
/// state, then marshal `mfbDrawLine:` onto the main thread (waitUntilDone:YES) so
/// the cell buffer is mutated there (bug-165), matching `mfbClear:`. `is_horizontal`
/// selects the glyph table at emit time; the main-thread IMP clamps the span to the
/// current grid. The repaint is present-driven (plan-35-D §3).
fn emit_app_draw_line(
    symbol: &str,
    term_state_offset: usize,
    is_horizontal: bool,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let mut asm = Asm::new(symbol);
    let mut vregs = Vregs::new();
    // The four incoming args (ordinal + the run's coordinates) are read only AFTER
    // the tv/state objc fetches, so each lives in a vreg; v_tv/v_state/v_sel are held
    // across the objc calls too. Both members name a start point `(row, column)` then
    // the far end of the run — `drawHLine(line, row, columnA, columnB)` and
    // `drawVLine(line, rowA, column, rowB)` — so the fixed coordinate is arg 1 for the
    // horizontal form and arg 2 for the vertical one.
    let v_ord = vregs.next();
    let v_fixed = vregs.next();
    let v_lo = vregs.next();
    let v_hi = vregs.next();
    let v_tv = vregs.next();
    let v_state = vregs.next();
    let v_sel = vregs.next();
    let done = format!("{symbol}_done");
    let (fixed_arg, lo_arg, hi_arg) = if is_horizontal { (1, 2, 3) } else { (2, 1, 3) };
    asm.push(abi::move_register(&v_ord, abi::mfb_arg(0))); // ordinal
    asm.push(abi::move_register(&v_fixed, abi::mfb_arg(fixed_arg))); // fixed row/column
    asm.push(abi::move_register(&v_lo, abi::mfb_arg(lo_arg))); // start of the run
    asm.push(abi::move_register(&v_hi, abi::mfb_arg(hi_arg))); // far end of the run
    emit_term_active_gate(&mut asm, term_state_offset, &done);
    // tv = objc_getAssociatedObject([NSApplication sharedApplication], &TERMVIEW_KEY)
    asm.external_data(&v_tv, CLASS_NS_APPLICATION, LIB_APPKIT);
    asm.load_selector(SEL_SHARED_APPLICATION.0);
    asm.push(abi::move_register(abi::mfb_arg(0), &v_tv));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.local_address(abi::mfb_arg(1), TERMVIEW_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register(&v_tv, abi::mfb_return(0))); // termView or nil
    asm.push(abi::compare_immediate(&v_tv, "0"));
    asm.push(abi::branch_eq(&done));
    // state = objc_getAssociatedObject(tv, &TVSTATE_KEY)
    asm.push(abi::move_register(abi::mfb_arg(0), &v_tv));
    asm.local_address(abi::mfb_arg(1), TVSTATE_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register(&v_state, abi::mfb_return(0))); // state or nil
    asm.push(abi::compare_immediate(&v_state, "0"));
    asm.push(abi::branch_eq(&done));
    // Resolve the glyph from the parked ordinal; ordinal 0 (Light) is the
    // fall-through default. The table is chosen at emit time (this body is emitted
    // separately for drawHLine and drawVLine).
    let table: &[u32; 7] = if is_horizontal {
        &crate::codegen::error::constants::TERM_HLINE_CODEPOINTS
    } else {
        &crate::codegen::error::constants::TERM_VLINE_CODEPOINTS
    };
    let gdone = format!("{symbol}_gdone");
    asm.push(abi::move_register(abi::SCRATCH[0], &v_ord)); // ordinal
    asm.push(abi::move_immediate(
        abi::SCRATCH[1],
        "Integer",
        &table[0].to_string(),
    ));
    for (ordinal, codepoint) in table.iter().enumerate().skip(1) {
        let next = format!("{symbol}_g{ordinal}");
        asm.push(abi::compare_immediate(
            abi::SCRATCH[0],
            &ordinal.to_string(),
        ));
        asm.push(abi::branch_ne(&next));
        asm.push(abi::move_immediate(
            abi::SCRATCH[1],
            "Integer",
            &codepoint.to_string(),
        ));
        asm.push(abi::branch(&gdone));
        asm.push(abi::label(&next));
    }
    asm.push(abi::label(&gdone));
    asm.push(abi::store_u64(
        abi::SCRATCH[1],
        &v_state,
        TV_DRAW_GLYPH_OFFSET,
    ));
    asm.push(abi::store_u64(&v_fixed, &v_state, TV_DRAW_FIXED_OFFSET));
    asm.push(abi::store_u64(&v_lo, &v_state, TV_DRAW_LO_OFFSET));
    asm.push(abi::store_u64(&v_hi, &v_state, TV_DRAW_HI_OFFSET));
    asm.push(abi::move_immediate(
        abi::SCRATCH[0],
        "Integer",
        if is_horizontal { "1" } else { "0" },
    ));
    asm.push(abi::store_u64(
        abi::SCRATCH[0],
        &v_state,
        TV_DRAW_HORIZ_OFFSET,
    ));
    // [tv performSelectorOnMainThread:@selector(mfbDrawLine:) withObject:nil
    //     waitUntilDone:YES]
    asm.load_selector(SEL_MFB_DRAW_LINE.0);
    asm.push(abi::move_register(&v_sel, abi::mfb_arg(1))); // mfbDrawLine: sel
    asm.load_selector(SEL_PERFORM_ON_MAIN.0);
    asm.push(abi::move_register(abi::mfb_arg(2), &v_sel));
    asm.push(abi::move_immediate(abi::mfb_arg(3), "Integer", "0")); // withObject: nil
    asm.push(abi::move_immediate(abi::mfb_arg(4), "Integer", "1")); // waitUntilDone: YES
    asm.push(abi::move_register(abi::mfb_arg(0), &v_tv));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::label(&done));
    asm.push(abi::move_immediate(RESULT_TAG_REGISTER, "Integer", "0")); // RESULT_OK_TAG
    asm.push(abi::return_());
    instructions.extend(asm.ins);
    relocations.extend(asm.rel);
}

/// Emit `dst = table[ord]` (a unichar code point) as a select-by-ordinal chain,
/// defaulting to entry 0 for an out-of-range ordinal. `tag` uniquifies the labels.
fn emit_app_select_unichar(asm: &mut Asm, ord: &str, dst: &str, table: &[u32], tag: &str) {
    let done = format!("{tag}_done");
    asm.push(abi::move_immediate(dst, "Integer", &table[0].to_string()));
    for (ordinal, codepoint) in table.iter().enumerate().skip(1) {
        let next = format!("{tag}_{ordinal}");
        asm.push(abi::compare_immediate(ord, &ordinal.to_string()));
        asm.push(abi::branch_ne(&next));
        asm.push(abi::move_immediate(dst, "Integer", &codepoint.to_string()));
        asm.push(abi::branch(&done));
        asm.push(abi::label(&next));
    }
    asm.push(abi::label(&done));
}

/// `term::drawBox` app body: resolve the `LineStyle` ordinal to the six box glyphs
/// (H/V edges + four corners, as unichars — `*Dash`/`*Dot` reuse the Light/Heavy
/// corners), park them plus the two raw corner points in the TermView state, then
/// marshal `mfbDrawBox:` onto the main thread (waitUntilDone:YES, like
/// `mfbDrawLine:`), which normalises/clamps and stamps. Present-driven.
fn emit_app_draw_box(
    symbol: &str,
    term_state_offset: usize,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let mut asm = Asm::new(symbol);
    let mut vregs = Vregs::new();
    // The five incoming args (ordinal + the two raw corner points) are read only
    // AFTER the tv/state objc fetches, so each lives in a vreg; v_tv/v_state/v_sel
    // are held across the objc calls too.
    let v_ord = vregs.next();
    let v_x1 = vregs.next();
    let v_y1 = vregs.next();
    let v_x2 = vregs.next();
    let v_y2 = vregs.next();
    let v_tv = vregs.next();
    let v_state = vregs.next();
    let v_sel = vregs.next();
    let done = format!("{symbol}_done");
    let ord = abi::SCRATCH[0];
    let dst = abi::SCRATCH[1];
    // Corners arrive as `(rowA, columnA, rowB, columnB)` — every `term::` point is
    // written row before column — so the rows are args 1/3 and the columns args 2/4.
    asm.push(abi::move_register(&v_ord, abi::mfb_arg(0))); // ordinal
    asm.push(abi::move_register(&v_y1, abi::mfb_arg(1))); // rowA
    asm.push(abi::move_register(&v_x1, abi::mfb_arg(2))); // columnA
    asm.push(abi::move_register(&v_y2, abi::mfb_arg(3))); // rowB
    asm.push(abi::move_register(&v_x2, abi::mfb_arg(4))); // columnB
    emit_term_active_gate(&mut asm, term_state_offset, &done);
    // tv = objc_getAssociatedObject([NSApplication sharedApplication], &TERMVIEW_KEY)
    asm.external_data(&v_tv, CLASS_NS_APPLICATION, LIB_APPKIT);
    asm.load_selector(SEL_SHARED_APPLICATION.0);
    asm.push(abi::move_register(abi::mfb_arg(0), &v_tv));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.local_address(abi::mfb_arg(1), TERMVIEW_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register(&v_tv, abi::mfb_return(0))); // termView or nil
    asm.push(abi::compare_immediate(&v_tv, "0"));
    asm.push(abi::branch_eq(&done));
    // state = objc_getAssociatedObject(tv, &TVSTATE_KEY)
    asm.push(abi::move_register(abi::mfb_arg(0), &v_tv));
    asm.local_address(abi::mfb_arg(1), TVSTATE_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register(&v_state, abi::mfb_return(0))); // state or nil
    asm.push(abi::compare_immediate(&v_state, "0"));
    asm.push(abi::branch_eq(&done));
    // Resolve the six glyphs from the parked ordinal (no calls until the marshal,
    // so `ord`/`dst` stay stable in scratch).
    asm.push(abi::move_register(ord, &v_ord));
    let glyphs: [(&[u32; 7], usize, &str); 6] = [
        (
            &crate::codegen::error::constants::TERM_HLINE_CODEPOINTS,
            TV_BOX_HG_OFFSET,
            "hg",
        ),
        (
            &crate::codegen::error::constants::TERM_VLINE_CODEPOINTS,
            TV_BOX_VG_OFFSET,
            "vg",
        ),
        (
            &crate::codegen::error::constants::TERM_CORNER_TL_CODEPOINTS,
            TV_BOX_CTL_OFFSET,
            "tl",
        ),
        (
            &crate::codegen::error::constants::TERM_CORNER_TR_CODEPOINTS,
            TV_BOX_CTR_OFFSET,
            "tr",
        ),
        (
            &crate::codegen::error::constants::TERM_CORNER_BL_CODEPOINTS,
            TV_BOX_CBL_OFFSET,
            "bl",
        ),
        (
            &crate::codegen::error::constants::TERM_CORNER_BR_CODEPOINTS,
            TV_BOX_CBR_OFFSET,
            "br",
        ),
    ];
    for (table, off, tag) in glyphs {
        emit_app_select_unichar(&mut asm, ord, dst, table, &format!("{symbol}_box_{tag}"));
        asm.push(abi::store_u64(dst, &v_state, off));
    }
    // Park the two raw corner points (normalised/clamped on the main thread).
    asm.push(abi::store_u64(&v_x1, &v_state, TV_BOX_X1_OFFSET));
    asm.push(abi::store_u64(&v_y1, &v_state, TV_BOX_Y1_OFFSET));
    asm.push(abi::store_u64(&v_x2, &v_state, TV_BOX_X2_OFFSET));
    asm.push(abi::store_u64(&v_y2, &v_state, TV_BOX_Y2_OFFSET));
    // [tv performSelectorOnMainThread:@selector(mfbDrawBox:) withObject:nil
    //     waitUntilDone:YES]
    asm.load_selector(SEL_MFB_DRAW_BOX.0);
    asm.push(abi::move_register(&v_sel, abi::mfb_arg(1))); // mfbDrawBox: sel
    asm.load_selector(SEL_PERFORM_ON_MAIN.0);
    asm.push(abi::move_register(abi::mfb_arg(2), &v_sel));
    asm.push(abi::move_immediate(abi::mfb_arg(3), "Integer", "0")); // withObject: nil
    asm.push(abi::move_immediate(abi::mfb_arg(4), "Integer", "1")); // waitUntilDone: YES
    asm.push(abi::move_register(abi::mfb_arg(0), &v_tv));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::label(&done));
    asm.push(abi::move_immediate(RESULT_TAG_REGISTER, "Integer", "0")); // RESULT_OK_TAG
    asm.push(abi::return_());
    instructions.extend(asm.ins);
    relocations.extend(asm.rel);
}

/// `term::fillRect` app body: resolve the `FillStyle` ordinal to a unichar, park
/// it plus the two raw corner points in the TermView state, then marshal
/// `mfbFillRect:` onto the main thread (waitUntilDone:YES), which normalises,
/// clamps, and fills. Present-driven.
fn emit_app_fill_rect(
    symbol: &str,
    term_state_offset: usize,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let mut asm = Asm::new(symbol);
    let mut vregs = Vregs::new();
    // ordinal + the two raw corner points are read AFTER the tv/state objc fetches,
    // so each lives in a vreg; v_tv/v_state/v_sel are held across the objc calls too.
    let v_ord = vregs.next();
    let v_x1 = vregs.next();
    let v_y1 = vregs.next();
    let v_x2 = vregs.next();
    let v_y2 = vregs.next();
    let v_tv = vregs.next();
    let v_state = vregs.next();
    let v_sel = vregs.next();
    let done = format!("{symbol}_done");
    let ord = abi::SCRATCH[0];
    let dst = abi::SCRATCH[1];
    // Corners arrive as `(rowA, columnA, rowB, columnB)` — every `term::` point is
    // written row before column — so the rows are args 1/3 and the columns args 2/4.
    asm.push(abi::move_register(&v_ord, abi::mfb_arg(0))); // ordinal
    asm.push(abi::move_register(&v_y1, abi::mfb_arg(1))); // rowA
    asm.push(abi::move_register(&v_x1, abi::mfb_arg(2))); // columnA
    asm.push(abi::move_register(&v_y2, abi::mfb_arg(3))); // rowB
    asm.push(abi::move_register(&v_x2, abi::mfb_arg(4))); // columnB
    emit_term_active_gate(&mut asm, term_state_offset, &done);
    asm.external_data(&v_tv, CLASS_NS_APPLICATION, LIB_APPKIT);
    asm.load_selector(SEL_SHARED_APPLICATION.0);
    asm.push(abi::move_register(abi::mfb_arg(0), &v_tv));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.local_address(abi::mfb_arg(1), TERMVIEW_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register(&v_tv, abi::mfb_return(0))); // termView or nil
    asm.push(abi::compare_immediate(&v_tv, "0"));
    asm.push(abi::branch_eq(&done));
    asm.push(abi::move_register(abi::mfb_arg(0), &v_tv));
    asm.local_address(abi::mfb_arg(1), TVSTATE_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register(&v_state, abi::mfb_return(0))); // state or nil
    asm.push(abi::compare_immediate(&v_state, "0"));
    asm.push(abi::branch_eq(&done));
    // Resolve the fill glyph from the parked ordinal.
    asm.push(abi::move_register(ord, &v_ord));
    emit_app_select_unichar(
        &mut asm,
        ord,
        dst,
        &crate::codegen::error::constants::TERM_FILL_CODEPOINTS,
        &format!("{symbol}_fill"),
    );
    asm.push(abi::store_u64(dst, &v_state, TV_FILL_GLYPH_OFFSET));
    asm.push(abi::store_u64(&v_x1, &v_state, TV_FILL_X1_OFFSET));
    asm.push(abi::store_u64(&v_y1, &v_state, TV_FILL_Y1_OFFSET));
    asm.push(abi::store_u64(&v_x2, &v_state, TV_FILL_X2_OFFSET));
    asm.push(abi::store_u64(&v_y2, &v_state, TV_FILL_Y2_OFFSET));
    // [tv performSelectorOnMainThread:@selector(mfbFillRect:) withObject:nil
    //     waitUntilDone:YES]
    asm.load_selector(SEL_MFB_FILL_RECT.0);
    asm.push(abi::move_register(&v_sel, abi::mfb_arg(1))); // mfbFillRect: sel
    asm.load_selector(SEL_PERFORM_ON_MAIN.0);
    asm.push(abi::move_register(abi::mfb_arg(2), &v_sel));
    asm.push(abi::move_immediate(abi::mfb_arg(3), "Integer", "0")); // withObject: nil
    asm.push(abi::move_immediate(abi::mfb_arg(4), "Integer", "1")); // waitUntilDone: YES
    asm.push(abi::move_register(abi::mfb_arg(0), &v_tv));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::label(&done));
    asm.push(abi::move_immediate(RESULT_TAG_REGISTER, "Integer", "0")); // RESULT_OK_TAG
    asm.push(abi::return_());
    instructions.extend(asm.ins);
    relocations.extend(asm.rel);
}

/// `term::drawGlyph` app body: park the code point (as a unichar) + cell in the
/// TermView state and marshal `mfbDrawGlyph:` onto the main thread. Control code
/// points (< 0x20) are skipped (they would corrupt the surface). Present-driven.
fn emit_app_draw_glyph(
    symbol: &str,
    term_state_offset: usize,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let mut asm = Asm::new(symbol);
    let mut vregs = Vregs::new();
    // `drawGlyph(row, column, codepoint)`: the point is row-first. All three are read
    // AFTER the tv/state objc fetches, so each lives in a vreg; v_tv/v_state/v_sel are
    // held across the objc calls too.
    let v_x = vregs.next();
    let v_y = vregs.next();
    let v_cp = vregs.next();
    let v_tv = vregs.next();
    let v_state = vregs.next();
    let v_sel = vregs.next();
    let done = format!("{symbol}_done");
    asm.push(abi::move_register(&v_y, abi::mfb_arg(0))); // row
    asm.push(abi::move_register(&v_x, abi::mfb_arg(1))); // column
    asm.push(abi::move_register(&v_cp, abi::mfb_arg(2))); // codepoint
    emit_term_active_gate(&mut asm, term_state_offset, &done);
    // Skip control code points.
    asm.push(abi::compare_immediate(&v_cp, "32"));
    asm.push(abi::branch_lt(&done));
    asm.external_data(&v_tv, CLASS_NS_APPLICATION, LIB_APPKIT);
    asm.load_selector(SEL_SHARED_APPLICATION.0);
    asm.push(abi::move_register(abi::mfb_arg(0), &v_tv));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.local_address(abi::mfb_arg(1), TERMVIEW_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register(&v_tv, abi::mfb_return(0)));
    asm.push(abi::compare_immediate(&v_tv, "0"));
    asm.push(abi::branch_eq(&done));
    asm.push(abi::move_register(abi::mfb_arg(0), &v_tv));
    asm.local_address(abi::mfb_arg(1), TVSTATE_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register(&v_state, abi::mfb_return(0)));
    asm.push(abi::compare_immediate(&v_state, "0"));
    asm.push(abi::branch_eq(&done));
    asm.push(abi::store_u64(&v_cp, &v_state, TV_GLYPH_G_OFFSET));
    asm.push(abi::store_u64(&v_x, &v_state, TV_GLYPH_X_OFFSET));
    asm.push(abi::store_u64(&v_y, &v_state, TV_GLYPH_Y_OFFSET));
    asm.load_selector(SEL_MFB_DRAW_GLYPH.0);
    asm.push(abi::move_register(&v_sel, abi::mfb_arg(1)));
    asm.load_selector(SEL_PERFORM_ON_MAIN.0);
    asm.push(abi::move_register(abi::mfb_arg(2), &v_sel));
    asm.push(abi::move_immediate(abi::mfb_arg(3), "Integer", "0"));
    asm.push(abi::move_immediate(abi::mfb_arg(4), "Integer", "1"));
    asm.push(abi::move_register(abi::mfb_arg(0), &v_tv));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::label(&done));
    asm.push(abi::move_immediate(RESULT_TAG_REGISTER, "Integer", "0")); // RESULT_OK_TAG
    asm.push(abi::return_());
    instructions.extend(asm.ins);
    relocations.extend(asm.rel);
}

/// `term::drawText` app body: park the start cell in the TermView state, build an
/// NSString from the mfb string, marshal `mfbDrawText:` onto the main thread (the
/// NSString as the object argument, like `mfbWriteString:`), then release it.
/// Present-driven.
fn emit_app_draw_text(
    symbol: &str,
    term_state_offset: usize,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let mut asm = Asm::new(symbol);
    let mut vregs = Vregs::new();
    // `drawText(row, column, text)`: the point is row-first. All three are read AFTER
    // the tv/state objc fetches, so each lives in a vreg; v_tv/v_nsstr/v_sel are held
    // across the objc calls too.
    let v_x = vregs.next();
    let v_y = vregs.next();
    let v_strobj = vregs.next();
    let v_tv = vregs.next();
    let v_state = vregs.next();
    let v_nsstr = vregs.next();
    let v_sel = vregs.next();
    let done = format!("{symbol}_done");
    asm.push(abi::move_register(&v_y, abi::mfb_arg(0))); // row
    asm.push(abi::move_register(&v_x, abi::mfb_arg(1))); // column
    asm.push(abi::move_register(&v_strobj, abi::mfb_arg(2))); // strobj (mfb String)
    emit_term_active_gate(&mut asm, term_state_offset, &done);
    asm.external_data(&v_tv, CLASS_NS_APPLICATION, LIB_APPKIT);
    asm.load_selector(SEL_SHARED_APPLICATION.0);
    asm.push(abi::move_register(abi::mfb_arg(0), &v_tv));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.local_address(abi::mfb_arg(1), TERMVIEW_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register(&v_tv, abi::mfb_return(0))); // tv
    asm.push(abi::compare_immediate(&v_tv, "0"));
    asm.push(abi::branch_eq(&done));
    asm.push(abi::move_register(abi::mfb_arg(0), &v_tv));
    asm.local_address(abi::mfb_arg(1), TVSTATE_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register(&v_state, abi::mfb_return(0))); // state
    asm.push(abi::compare_immediate(&v_state, "0"));
    asm.push(abi::branch_eq(&done));
    asm.push(abi::store_u64(&v_x, &v_state, TV_TEXT_X_OFFSET));
    asm.push(abi::store_u64(&v_y, &v_state, TV_TEXT_Y_OFFSET));
    // nsstr = [[NSString alloc] initWithBytes:(strobj+8) length:strobj[0] encoding:UTF8]
    asm.external_data(&v_nsstr, CLASS_NS_STRING, LIB_FOUNDATION);
    asm.load_selector(SEL_ALLOC.0);
    asm.push(abi::move_register(abi::mfb_arg(0), &v_nsstr));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register(&v_nsstr, abi::mfb_return(0)));
    asm.load_selector(SEL_INIT_WITH_BYTES.0);
    asm.push(abi::add_immediate(abi::mfb_arg(2), &v_strobj, 8));
    asm.push(abi::load_u64(abi::mfb_arg(3), &v_strobj, 0));
    asm.push(abi::move_immediate(
        abi::mfb_arg(4),
        "Integer",
        NS_UTF8_ENCODING,
    ));
    asm.push(abi::move_register(abi::mfb_arg(0), &v_nsstr));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register(&v_nsstr, abi::mfb_return(0))); // nsstr
                                                                // [tv performSelectorOnMainThread:mfbDrawText: withObject:nsstr waitUntilDone:YES]
    asm.load_selector(SEL_MFB_DRAW_TEXT.0);
    asm.push(abi::move_register(&v_sel, abi::mfb_arg(1)));
    asm.load_selector(SEL_PERFORM_ON_MAIN.0);
    asm.push(abi::move_register(abi::mfb_arg(2), &v_sel));
    asm.push(abi::move_register(abi::mfb_arg(3), &v_nsstr));
    asm.push(abi::move_immediate(abi::mfb_arg(4), "Integer", "1"));
    asm.push(abi::move_register(abi::mfb_arg(0), &v_tv));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    // [nsstr release] — owned (alloc+init); mfbDrawText: only read it (synchronous).
    asm.load_selector(SEL_RELEASE.0);
    asm.push(abi::move_register(abi::mfb_arg(0), &v_nsstr));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::label(&done));
    asm.push(abi::move_immediate(RESULT_TAG_REGISTER, "Integer", "0")); // RESULT_OK_TAG
    asm.push(abi::return_());
    instructions.extend(asm.ins);
    relocations.extend(asm.rel);
}

/// `term::showCursor`/`hideCursor` app body: store the cursor-visible flag into
/// the TermView state (and the term-state global). Cursor glyph rendering is a
/// later refinement, so no redraw is needed yet.
fn emit_app_set_cursor_visible(
    symbol: &str,
    term_state_offset: usize,
    value: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let mut asm = Asm::new(symbol);
    let mut vregs = Vregs::new();
    // The TVSTATE pointer is produced by emit_get_tv_state's objc calls and used
    // immediately after, so it lives in an allocator vreg (plan-101).
    let v_state = vregs.next();
    let done = format!("{symbol}_done");
    emit_term_active_gate(&mut asm, term_state_offset, &done);
    asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", value));
    asm.push(abi::store_u64(
        abi::SCRATCH[0],
        TERM_ARENA_STATE_REG,
        term_state_offset + crate::codegen::error::constants::TERM_STATE_CURSOR_VISIBLE_OFFSET,
    ));
    emit_get_tv_state(&mut asm, &v_state, &done);
    asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", value));
    asm.push(abi::store_u64(
        abi::SCRATCH[0],
        &v_state,
        TV_CURSOR_VISIBLE_OFFSET,
    ));
    asm.push(abi::label(&done));
    asm.push(abi::move_immediate(RESULT_TAG_REGISTER, "Integer", "0")); // RESULT_OK_TAG
    asm.push(abi::return_());
    instructions.extend(asm.ins);
    relocations.extend(asm.rel);
}

/// `term::terminalSize` app body: return a `TermSize { columns, rows }` record
/// from the TermView grid, or `ERR_UNSUPPORTED` when inactive / no surface.
fn emit_app_terminal_size(
    symbol: &str,
    term_state_offset: usize,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let mut asm = Asm::new(symbol);
    let mut vregs = Vregs::new();
    // columns/rows must survive the arena_alloc internal call, and the TVSTATE ptr
    // survives emit_get_tv_state's objc calls, so all three live in allocator vregs
    // (plan-101 — the finalizer colors them callee-saved AND saves them).
    let v_cols = vregs.next();
    let v_rows = vregs.next();
    let v_state = vregs.next();
    let unsupported = format!("{symbol}_unsupported");
    let done = format!("{symbol}_done");
    // Requires active TUI mode (plan §4.7).
    asm.push(abi::load_u64(
        abi::SCRATCH[0],
        TERM_ARENA_STATE_REG,
        term_state_offset + crate::codegen::error::constants::TERM_STATE_ACTIVE_OFFSET,
    ));
    asm.push(abi::compare_immediate(abi::SCRATCH[0], "0"));
    asm.push(abi::branch_eq(&unsupported));
    emit_get_tv_state(&mut asm, &v_state, &unsupported);
    asm.push(abi::load_u64(&v_cols, &v_state, TV_COLS_OFFSET));
    asm.push(abi::load_u64(&v_rows, &v_state, TV_ROWS_OFFSET));
    // record = arena_alloc(16, 8); columns@0, rows@8.
    asm.push(abi::move_immediate(abi::mfb_arg(0), "Integer", "16"));
    asm.push(abi::move_immediate(abi::mfb_arg(1), "Integer", "8"));
    asm.call_internal(ARENA_ALLOC_SYMBOL);
    asm.push(abi::compare_immediate(RESULT_TAG_REGISTER, "0")); // RESULT_OK_TAG
    asm.push(abi::branch_ne(&unsupported));
    // arena_alloc returns the record pointer in RESULT_VALUE_REGISTER; the parked
    // columns/rows vregs survived the call, so stamp them into the record.
    asm.push(abi::store_u64(&v_cols, RESULT_VALUE_REGISTER, 0)); // columns
    asm.push(abi::store_u64(&v_rows, RESULT_VALUE_REGISTER, 8)); // rows
    asm.push(abi::move_immediate(RESULT_TAG_REGISTER, "Integer", "0")); // OK; value = record
    asm.push(abi::branch(&done));
    // plan-88-C: code + message symbol from `ERRORCODE_CONSTANTS`, loaded into the
    // shared `RESULT_*` result registers (tag/value/message), byte-identical to the
    // former local `ERR_UNSUPPORTED_*` consts.
    let (unsupported_code, unsupported_symbol) =
        crate::codegen::registry::runtime_error_emission("ErrUnsupported")
            .expect("ErrUnsupported is an errorCode constant");
    asm.push(abi::label(&unsupported));
    asm.push(abi::move_immediate(
        RESULT_VALUE_REGISTER,
        "Integer",
        unsupported_code,
    ));
    asm.push(abi::move_immediate(RESULT_TAG_REGISTER, "Integer", "1")); // ERR tag
    asm.local_address(RESULT_ERROR_MESSAGE_REGISTER, unsupported_symbol);
    asm.push(abi::label(&done));
    asm.push(abi::return_());
    instructions.extend(asm.ins);
    relocations.extend(asm.rel);
}
