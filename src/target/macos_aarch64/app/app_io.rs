//! macOS app-mode IO ops: `emit_app_io_*` and `emit_app_term_*` emitters
//! (write/flush/input/terminal-size/set-color/attr/move/clear/cursor) (plan-11 split).

use super::*;
use crate::target::shared::code::AppHookBody;

/// App-mode body for `io.print`/`io.write`/`io.printError`/`io.writeError`. The
/// runtime helper receives the MFBASIC string object in `x0` (`{u64 len; bytes}`)
/// and returns a `Result` (tag in `x0`). When TUI mode is active the text is
/// written into the TermView surface (plan-01-term.md §4.8); otherwise, when a
/// transcript view is attached (GUI), append to it; else (headless) write to fd.
/// `term_state_offset` is the writable term-state slot base (None when the
/// program never uses `term::`).
pub(crate) fn emit_app_io_write_helper(
    symbol: &str,
    stderr: bool,
    newline: bool,
    term_state_offset: Option<usize>,
) -> AppHookBody {
    let mut asm = Asm::new(symbol);
    let fd = if stderr { "2" } else { "1" };
    // lr@0, x19(string)@8, x20(view)@16, x21(scratch)@24, nl byte@32, x22(sel)@40,
    // autorelease-pool token@48, string arg@56.
    let frame = 64;
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(frame));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        0,
    ));
    asm.push(abi::store_u64(abi::LOCAL[0], abi::stack_pointer(), 8));
    asm.push(abi::store_u64(abi::LOCAL[1], abi::stack_pointer(), 16));
    asm.push(abi::store_u64(abi::LOCAL[2], abi::stack_pointer(), 24));
    asm.push(abi::store_u64(abi::LOCAL[3], abi::stack_pointer(), 40));
    // Per-write autorelease pool. The worker's process-lifetime pool
    // (emit_worker_shim) is never drained, so the autoreleased NSStrings this
    // helper builds for the "[stderr] " prefix and the trailing newline would
    // accumulate for the process lifetime (bug-112). Save the string arg first
    // (poolPush clobbers x0); `objc_autoreleasePoolPush` preserves x19-x28, so
    // the pinned arena-state base in x19 survives for the TUI-active check below.
    asm.push(abi::store_u64("x0", abi::stack_pointer(), 56)); // string arg
    asm.call_external("_objc_autoreleasePoolPush", LIB_OBJC);
    asm.push(abi::store_u64("x0", abi::stack_pointer(), 48)); // pool token
                                                              // While TUI mode is active, route to the TermView surface (x19 is the pinned
                                                              // arena-state base on entry, before it is reused for the string object).
    if let Some(off) = term_state_offset {
        asm.push(abi::load_u64(
            abi::SCRATCH[0],
            TERM_ARENA_STATE_REG,
            off + code::TERM_STATE_ACTIVE_OFFSET,
        ));
        asm.push(abi::load_u64(abi::LOCAL[0], abi::stack_pointer(), 56)); // string object
        asm.push(abi::compare_immediate(abi::SCRATCH[0], "0"));
        asm.push(abi::branch_ne("term_surface_path"));
    } else {
        asm.push(abi::load_u64(abi::LOCAL[0], abi::stack_pointer(), 56)); // string object
    }

    // app = [NSApplication sharedApplication]; view = objc_getAssociatedObject(app, &KEY)
    asm.external_data(abi::LOCAL[1], CLASS_NS_APPLICATION, LIB_APPKIT);
    asm.load_selector(SEL_SHARED_APPLICATION.0);
    asm.push(abi::move_register("x0", abi::LOCAL[1]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.local_address("x1", ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[1], "x0")); // transcript view or nil
    asm.push(abi::compare_immediate(abi::LOCAL[1], "0"));
    asm.push(abi::branch_eq("fd_path"));

    // --- GUI transcript path ---
    if stderr {
        // Visually distinguish stderr with a "[stderr] " marker (plan §5.4).
        build_nsstring_from_cstring(&mut asm, abi::LOCAL[2], STR_STDERR_PREFIX.0);
        asm.push(abi::move_register("x1", "x0"));
        asm.push(abi::move_register("x0", abi::LOCAL[1]));
        asm.call_internal(APPEND_SYMBOL);
    }
    // text = [[NSString alloc] initWithBytes:(str+8) length:str[0] encoding:UTF8]
    asm.external_data(abi::LOCAL[2], CLASS_NS_STRING, LIB_FOUNDATION);
    asm.load_selector(SEL_ALLOC.0);
    asm.push(abi::move_register("x0", abi::LOCAL[2]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[2], "x0")); // allocated NSString
    asm.load_selector(SEL_INIT_WITH_BYTES.0);
    asm.push(abi::add_immediate("x2", abi::LOCAL[0], 8)); // bytes
    asm.push(abi::load_u64("x3", abi::LOCAL[0], 0)); // length
    asm.push(abi::move_immediate("x4", "Integer", NS_UTF8_ENCODING));
    asm.push(abi::move_register("x0", abi::LOCAL[2]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[3], "x0")); // owned NSString (save across append)
    asm.push(abi::move_register("x1", "x0")); // text nsstring
    asm.push(abi::move_register("x0", abi::LOCAL[1]));
    asm.call_internal(APPEND_SYMBOL);
    // [text release] — the NSString was created owned (alloc +
    // initWithBytes:length:encoding:, retain count 1) and _mfb_macapp_append
    // copies it into the text storage, so we hold the sole reference (bug-53).
    // x22 is callee-saved and preserved by _mfb_macapp_append (it saves x19-x22).
    asm.load_selector(SEL_RELEASE.0);
    asm.push(abi::move_register("x0", abi::LOCAL[3]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    if newline {
        build_nsstring_from_cstring(&mut asm, abi::LOCAL[2], STR_NEWLINE.0);
        asm.push(abi::move_register("x1", "x0"));
        asm.push(abi::move_register("x0", abi::LOCAL[1]));
        asm.call_internal(APPEND_SYMBOL);
    }
    asm.push(abi::move_immediate("x0", "Integer", "0")); // RESULT_OK_TAG
    asm.push(abi::branch("done"));

    // --- headless / no-window path: write to the file descriptor ---
    asm.push(abi::label("fd_path"));
    asm.push(abi::move_immediate("x0", "Integer", fd));
    asm.push(abi::add_immediate("x1", abi::LOCAL[0], 8));
    asm.push(abi::load_u64("x2", abi::LOCAL[0], 0));
    asm.call_external("_write", LIB_SYSTEM);
    if newline {
        asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", "10")); // '\n'
        asm.push(abi::store_u8(abi::SCRATCH[0], abi::stack_pointer(), 32));
        asm.push(abi::move_immediate("x0", "Integer", fd));
        asm.push(abi::add_immediate("x1", abi::stack_pointer(), 32));
        asm.push(abi::move_immediate("x2", "Integer", "1"));
        asm.call_external("_write", LIB_SYSTEM);
    }
    asm.push(abi::move_immediate("x0", "Integer", "0")); // RESULT_OK_TAG
    asm.push(abi::branch("done"));

    // --- TUI surface path: write into the TermView grid on the main thread ---
    if term_state_offset.is_some() {
        asm.push(abi::label("term_surface_path"));
        // tv = objc_getAssociatedObject([NSApplication sharedApplication], &TERMVIEW_KEY)
        asm.external_data(abi::LOCAL[1], CLASS_NS_APPLICATION, LIB_APPKIT);
        asm.load_selector(SEL_SHARED_APPLICATION.0);
        asm.push(abi::move_register("x0", abi::LOCAL[1]));
        asm.call_external("_objc_msgSend", LIB_OBJC);
        asm.local_address("x1", TERMVIEW_ASSOC_KEY);
        asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
        asm.push(abi::move_register(abi::LOCAL[1], "x0")); // termview or nil
        asm.push(abi::compare_immediate(abi::LOCAL[1], "0"));
        asm.push(abi::branch_eq("fd_path")); // headless: no surface -> fd
                                             // text = [[NSString alloc] initWithBytes:(str+8) length:str[0] encoding:UTF8]
        asm.external_data(abi::LOCAL[2], CLASS_NS_STRING, LIB_FOUNDATION);
        asm.load_selector(SEL_ALLOC.0);
        asm.push(abi::move_register("x0", abi::LOCAL[2]));
        asm.call_external("_objc_msgSend", LIB_OBJC);
        asm.push(abi::move_register(abi::LOCAL[2], "x0"));
        asm.load_selector(SEL_INIT_WITH_BYTES.0);
        asm.push(abi::add_immediate("x2", abi::LOCAL[0], 8));
        asm.push(abi::load_u64("x3", abi::LOCAL[0], 0));
        asm.push(abi::move_immediate("x4", "Integer", NS_UTF8_ENCODING));
        asm.push(abi::move_register("x0", abi::LOCAL[2]));
        asm.call_external("_objc_msgSend", LIB_OBJC);
        asm.push(abi::move_register(abi::LOCAL[2], "x0")); // text nsstring
                                                           // [tv performSelectorOnMainThread:@selector(mfbWriteString:) withObject:text waitUntilDone:YES]
        asm.load_selector(SEL_MFB_WRITE_STRING.0);
        asm.push(abi::move_register(abi::LOCAL[3], "x1")); // mfbWriteString: sel
        asm.load_selector(SEL_PERFORM_ON_MAIN.0);
        asm.push(abi::move_register("x2", abi::LOCAL[3]));
        asm.push(abi::move_register("x3", abi::LOCAL[2]));
        asm.push(abi::move_immediate("x4", "Integer", "1")); // waitUntilDone: YES
        asm.push(abi::move_register("x0", abi::LOCAL[1]));
        asm.call_external("_objc_msgSend", LIB_OBJC);
        // [text release] — the NSString was created owned (alloc +
        // initWithBytes:length:encoding:); mfbWriteString: only reads its glyphs
        // (synchronous, waitUntilDone:YES) and does not retain it, so we hold the
        // sole reference and must release it (bug-53). x21 is callee-saved and
        // survives the performSelectorOnMainThread: msgSend above.
        asm.load_selector(SEL_RELEASE.0);
        asm.push(abi::move_register("x0", abi::LOCAL[2]));
        asm.call_external("_objc_msgSend", LIB_OBJC);
        if newline {
            build_nsstring_from_cstring(&mut asm, abi::LOCAL[2], STR_NEWLINE.0);
            asm.push(abi::move_register(abi::LOCAL[2], "x0")); // "\n" nsstring
            asm.load_selector(SEL_MFB_WRITE_STRING.0);
            asm.push(abi::move_register(abi::LOCAL[3], "x1"));
            asm.load_selector(SEL_PERFORM_ON_MAIN.0);
            asm.push(abi::move_register("x2", abi::LOCAL[3]));
            asm.push(abi::move_register("x3", abi::LOCAL[2]));
            asm.push(abi::move_immediate("x4", "Integer", "1"));
            asm.push(abi::move_register("x0", abi::LOCAL[1]));
            asm.call_external("_objc_msgSend", LIB_OBJC);
        }
        asm.push(abi::move_immediate("x0", "Integer", "0")); // RESULT_OK_TAG
    }

    asm.push(abi::label("done"));
    // Drain this write's autoreleased NSStrings, then re-establish the OK result
    // (poolPop clobbers x0). Every path here returns RESULT_OK_TAG.
    asm.push(abi::load_u64("x0", abi::stack_pointer(), 48)); // pool token
    asm.call_external("_objc_autoreleasePoolPop", LIB_OBJC);
    asm.push(abi::move_immediate("x0", "Integer", "0")); // RESULT_OK_TAG
    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::load_u64(abi::LOCAL[0], abi::stack_pointer(), 8));
    asm.push(abi::load_u64(abi::LOCAL[1], abi::stack_pointer(), 16));
    asm.push(abi::load_u64(abi::LOCAL[2], abi::stack_pointer(), 24));
    asm.push(abi::load_u64(abi::LOCAL[3], abi::stack_pointer(), 40));
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

/// App-mode body for `io.flush`. Transcript writes are already synchronous (see
/// [`emit_append_helper`]), but in TUI mode grid writes are retained and only
/// presented on demand, so `io::flush` drives the same coalesced present as
/// `term::sync` — a marshaled `setNeedsDisplay:` on the TermView (plan-35-D §3).
/// Headless / no-surface runs skip the present and return OK.
pub(crate) fn emit_app_io_flush_helper(symbol: &str) -> AppHookBody {
    let mut asm = Asm::new(symbol);
    let frame = 32; // lr@0, x20(termView)@8, x21(sel)@16
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(frame));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        0,
    ));
    asm.push(abi::store_u64(abi::LOCAL[1], abi::stack_pointer(), 8));
    asm.push(abi::store_u64(abi::LOCAL[2], abi::stack_pointer(), 16));
    emit_present_needs_display(&mut asm, "flush_done");
    asm.push(abi::label("flush_done"));
    emit_term_ok_return(&mut asm, frame, &[(abi::LOCAL[1], 8), (abi::LOCAL[2], 16)]);
    (
        CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        asm.ins,
        asm.rel,
    )
}

/// App-mode body for `io.input` (plan §5.4): render the prompt to the transcript
/// via the `io.write` helper, then read a committed line via the `io.readLine`
/// helper (which reads fd 0 — the window input pipe in app mode). The prompt
/// string is passed in `x0`; `io.readLine` takes no arguments, so its result
/// (`x0`/`x1`/`x2`) becomes this helper's result.
pub(crate) fn emit_app_io_input_helper(symbol: &str) -> AppHookBody {
    let mut asm = Asm::new(symbol);
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(16));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        0,
    ));
    asm.call_internal(IO_WRITE_SYMBOL); // x0 = prompt; renders it, result ignored
    emit_set_input_mode_instructions(&mut asm, INPUT_MODE_LINE_ECHO);
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
pub(crate) fn emit_app_io_is_terminal_helper(symbol: &str) -> AppHookBody {
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
pub(crate) fn emit_app_term_on_helper(symbol: &str, term_state_offset: usize) -> AppHookBody {
    let mut asm = Asm::new(symbol);
    // Frame: lr@0, x20(app)@8, x21(window)@16, x22(termview)@24, x23(sel)@32.
    let frame = 48;
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(frame));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        0,
    ));
    asm.push(abi::store_u64(abi::LOCAL[1], abi::stack_pointer(), 8));
    asm.push(abi::store_u64(abi::LOCAL[2], abi::stack_pointer(), 16));
    asm.push(abi::store_u64(abi::LOCAL[3], abi::stack_pointer(), 24));
    asm.push(abi::store_u64(abi::LOCAL[4], abi::stack_pointer(), 32));

    // Reset all term state to defaults (active on, fg white, bg black, bold and
    // underline off, cursor visible). x19 is the pinned arena-state base.
    store_term_state(
        &mut asm,
        term_state_offset,
        code::TERM_STATE_ACTIVE_OFFSET,
        "1",
    );
    store_term_state(
        &mut asm,
        term_state_offset,
        code::TERM_STATE_FG_OFFSET,
        "16777215",
    );
    store_term_state(&mut asm, term_state_offset, code::TERM_STATE_BG_OFFSET, "0");
    store_term_state(
        &mut asm,
        term_state_offset,
        code::TERM_STATE_BOLD_OFFSET,
        "0",
    );
    store_term_state(
        &mut asm,
        term_state_offset,
        code::TERM_STATE_UNDERLINE_OFFSET,
        "0",
    );
    store_term_state(
        &mut asm,
        term_state_offset,
        code::TERM_STATE_CURSOR_VISIBLE_OFFSET,
        "1",
    );

    // bug-150: entering TUI mode flips the window into immediate single-key
    // delivery once, from the moment `term::on` runs — set INPUT_MODE_KEY =
    // RAW_NO_ECHO so both keyDown IMPs (transcript `_mfb_macapp_key_down` and TUI
    // `_mfb_macapp_term_keyDown`) route each keystroke straight to the input pipe
    // instead of buffering until Return. The initial mode is nil (0) at startup;
    // this is the one-time flip. `io::input`/`io::readLine` still switch to
    // LINE_ECHO for their own read (emit_app_io_input_helper).
    emit_set_input_mode_instructions(&mut asm, INPUT_MODE_RAW_NO_ECHO);

    // app = [NSApplication sharedApplication]
    asm.external_data(abi::LOCAL[1], CLASS_NS_APPLICATION, LIB_APPKIT);
    asm.load_selector(SEL_SHARED_APPLICATION.0);
    asm.push(abi::move_register("x0", abi::LOCAL[1]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[1], "x0")); // app

    // window = objc_getAssociatedObject(app, &WINDOW_ASSOC_KEY); nil -> headless.
    asm.push(abi::move_register("x0", abi::LOCAL[1]));
    asm.local_address("x1", WINDOW_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[2], "x0")); // window or nil
    asm.push(abi::compare_immediate(abi::LOCAL[2], "0"));
    asm.push(abi::branch_eq("term_on_done"));

    // termview = objc_getAssociatedObject(app, &TERMVIEW_ASSOC_KEY)
    asm.push(abi::move_register("x0", abi::LOCAL[1]));
    asm.local_address("x1", TERMVIEW_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[3], "x0")); // termview

    // Clear the grid + home the cursor before the surface is displayed.
    asm.push(abi::move_register("x0", abi::LOCAL[3]));
    asm.call_internal(TERM_CLEAR_SYMBOL);

    // [window performSelectorOnMainThread:@selector(setContentView:)
    //         withObject:termview waitUntilDone:YES]  (AppKit is main-thread only)
    asm.load_selector(SEL_SET_CONTENT_VIEW.0);
    asm.push(abi::move_register(abi::LOCAL[4], "x1")); // setContentView: sel
    asm.load_selector(SEL_PERFORM_ON_MAIN.0);
    asm.push(abi::move_register("x2", abi::LOCAL[4]));
    asm.push(abi::move_register("x3", abi::LOCAL[3]));
    asm.push(abi::move_immediate("x4", "Integer", "1")); // waitUntilDone: YES
    asm.push(abi::move_register("x0", abi::LOCAL[2]));
    asm.call_external("_objc_msgSend", LIB_OBJC);

    // [window performSelectorOnMainThread:@selector(makeFirstResponder:)
    //         withObject:termview waitUntilDone:YES] — route keys to the surface.
    asm.load_selector(SEL_MAKE_FIRST_RESPONDER.0);
    asm.push(abi::move_register(abi::LOCAL[4], "x1"));
    asm.load_selector(SEL_PERFORM_ON_MAIN.0);
    asm.push(abi::move_register("x2", abi::LOCAL[4]));
    asm.push(abi::move_register("x3", abi::LOCAL[3])); // termview
    asm.push(abi::move_immediate("x4", "Integer", "1"));
    asm.push(abi::move_register("x0", abi::LOCAL[2])); // window
    asm.call_external("_objc_msgSend", LIB_OBJC);

    asm.push(abi::label("term_on_done"));
    asm.push(abi::move_immediate("x0", "Integer", "0")); // RESULT_OK_TAG
    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::load_u64(abi::LOCAL[1], abi::stack_pointer(), 8));
    asm.push(abi::load_u64(abi::LOCAL[2], abi::stack_pointer(), 16));
    asm.push(abi::load_u64(abi::LOCAL[3], abi::stack_pointer(), 24));
    asm.push(abi::load_u64(abi::LOCAL[4], abi::stack_pointer(), 32));
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

/// App-mode body for `term::off` (plan-01-term.md §4.2 / §6.3). No-op when
/// already off; otherwise restores the transcript scroll view as the window
/// content view on the main thread (GUI) and clears the active flag. Headless
/// runs update only the state global.
pub(crate) fn emit_app_term_off_helper(symbol: &str, term_state_offset: usize) -> AppHookBody {
    let mut asm = Asm::new(symbol);
    // Frame: lr@0, x20(app)@8, x21(window)@16, x22(scrollview)@24, x23(sel)@32.
    let frame = 48;
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(frame));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        0,
    ));
    asm.push(abi::store_u64(abi::LOCAL[1], abi::stack_pointer(), 8));
    asm.push(abi::store_u64(abi::LOCAL[2], abi::stack_pointer(), 16));
    asm.push(abi::store_u64(abi::LOCAL[3], abi::stack_pointer(), 24));
    asm.push(abi::store_u64(abi::LOCAL[4], abi::stack_pointer(), 32));

    // Gate: already off -> no-op (plan §4.2). x19 is the pinned arena-state base.
    asm.push(abi::load_u64(
        abi::SCRATCH[0],
        TERM_ARENA_STATE_REG,
        term_state_offset + code::TERM_STATE_ACTIVE_OFFSET,
    ));
    asm.push(abi::compare_immediate(abi::SCRATCH[0], "0"));
    asm.push(abi::branch_eq("term_off_done"));

    // app = [NSApplication sharedApplication]
    asm.external_data(abi::LOCAL[1], CLASS_NS_APPLICATION, LIB_APPKIT);
    asm.load_selector(SEL_SHARED_APPLICATION.0);
    asm.push(abi::move_register("x0", abi::LOCAL[1]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[1], "x0")); // app

    // window = objc_getAssociatedObject(app, &WINDOW_ASSOC_KEY); nil -> headless.
    asm.push(abi::move_register("x0", abi::LOCAL[1]));
    asm.local_address("x1", WINDOW_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[2], "x0")); // window or nil
    asm.push(abi::compare_immediate(abi::LOCAL[2], "0"));
    asm.push(abi::branch_eq("term_off_inactive"));

    // Final present (plan-35-D §3): force the TermView to draw synchronously
    // before the content-view swap, so the last drawn frame is shown (the
    // mandatory-present contract — a program that draws then `term::off`s without
    // a trailing `term::sync` still shows its final frame). `display` marks the
    // whole view dirty and repaints it immediately; marshaled waitUntilDone:YES so
    // it completes before the transcript swap below.
    asm.push(abi::move_register("x0", abi::LOCAL[1])); // app
    asm.local_address("x1", TERMVIEW_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[3], "x0")); // termview or nil
    asm.push(abi::compare_immediate(abi::LOCAL[3], "0"));
    asm.push(abi::branch_eq("term_off_presented"));
    asm.load_selector(SEL_DISPLAY.0);
    asm.push(abi::move_register(abi::LOCAL[4], "x1")); // display sel
    asm.load_selector(SEL_PERFORM_ON_MAIN.0);
    asm.push(abi::move_register("x2", abi::LOCAL[4]));
    asm.push(abi::move_immediate("x3", "Integer", "0")); // withObject: nil (display takes no arg)
    asm.push(abi::move_immediate("x4", "Integer", "1")); // waitUntilDone: YES
    asm.push(abi::move_register("x0", abi::LOCAL[3]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::label("term_off_presented"));

    // scroll = objc_getAssociatedObject(app, &SCROLLVIEW_ASSOC_KEY)
    asm.push(abi::move_register("x0", abi::LOCAL[1]));
    asm.local_address("x1", SCROLLVIEW_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[3], "x0")); // scroll view

    // [window performSelectorOnMainThread:@selector(setContentView:)
    //         withObject:scrollView waitUntilDone:YES]
    asm.load_selector(SEL_SET_CONTENT_VIEW.0);
    asm.push(abi::move_register(abi::LOCAL[4], "x1"));
    asm.load_selector(SEL_PERFORM_ON_MAIN.0);
    asm.push(abi::move_register("x2", abi::LOCAL[4]));
    asm.push(abi::move_register("x3", abi::LOCAL[3]));
    asm.push(abi::move_immediate("x4", "Integer", "1")); // waitUntilDone: YES
    asm.push(abi::move_register("x0", abi::LOCAL[2]));
    asm.call_external("_objc_msgSend", LIB_OBJC);

    // Restore the transcript as first responder so window input returns to it.
    asm.push(abi::move_register("x0", abi::LOCAL[1])); // app
    asm.local_address("x1", ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[3], "x0")); // transcript view
    asm.load_selector(SEL_MAKE_FIRST_RESPONDER.0);
    asm.push(abi::move_register(abi::LOCAL[4], "x1"));
    asm.load_selector(SEL_PERFORM_ON_MAIN.0);
    asm.push(abi::move_register("x2", abi::LOCAL[4]));
    asm.push(abi::move_register("x3", abi::LOCAL[3]));
    asm.push(abi::move_immediate("x4", "Integer", "1"));
    asm.push(abi::move_register("x0", abi::LOCAL[2])); // window
    asm.call_external("_objc_msgSend", LIB_OBJC);

    asm.push(abi::label("term_off_inactive"));
    // bug-150: leaving TUI mode returns the window to line input so subsequent
    // reads commit on Return again (symmetric with the console `term::off`
    // cooked-mode restore).
    emit_set_input_mode_instructions(&mut asm, INPUT_MODE_LINE_ECHO);
    store_term_state(
        &mut asm,
        term_state_offset,
        code::TERM_STATE_ACTIVE_OFFSET,
        "0",
    );

    asm.push(abi::label("term_off_done"));
    asm.push(abi::move_immediate("x0", "Integer", "0")); // RESULT_OK_TAG
    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::load_u64(abi::LOCAL[1], abi::stack_pointer(), 8));
    asm.push(abi::load_u64(abi::LOCAL[2], abi::stack_pointer(), 16));
    asm.push(abi::load_u64(abi::LOCAL[3], abi::stack_pointer(), 24));
    asm.push(abi::load_u64(abi::LOCAL[4], abi::stack_pointer(), 32));
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

/// App-mode dispatcher for the `term::` runtime helpers (plan-01-term.md §6.3,
/// Phase 5). Returns `None` for calls that keep the shared console backend
/// (`isOn` and the attribute getters, which read the term-state global the app
/// setters keep updated).
pub(crate) fn emit_app_term_helper(
    call: &str,
    symbol: &str,
    term_state_offset: usize,
) -> Option<AppHookBody> {
    let helper = match call {
        "term.on" => emit_app_term_on_helper(symbol, term_state_offset),
        "term.off" => emit_app_term_off_helper(symbol, term_state_offset),
        "term.setForeground" => emit_app_set_color(
            symbol,
            term_state_offset,
            code::TERM_STATE_FG_OFFSET,
            TV_CUR_FG_OFFSET,
        ),
        "term.setBackground" => emit_app_set_color(
            symbol,
            term_state_offset,
            code::TERM_STATE_BG_OFFSET,
            TV_CUR_BG_OFFSET,
        ),
        "term.setBold" => emit_app_set_attr(
            symbol,
            term_state_offset,
            code::TERM_STATE_BOLD_OFFSET,
            TV_CUR_BOLD_OFFSET,
        ),
        "term.setUnderline" => emit_app_set_attr(
            symbol,
            term_state_offset,
            code::TERM_STATE_UNDERLINE_OFFSET,
            TV_CUR_UNDERLINE_OFFSET,
        ),
        "term.moveTo" => emit_app_move_to(symbol, term_state_offset),
        "term.drawHLine" => emit_app_draw_line(symbol, term_state_offset, true),
        "term.drawVLine" => emit_app_draw_line(symbol, term_state_offset, false),
        "term.drawBox" => emit_app_draw_box(symbol, term_state_offset),
        "term.fillRect" => emit_app_fill_rect(symbol, term_state_offset),
        "term.drawGlyph" => emit_app_draw_glyph(symbol, term_state_offset),
        "term.drawText" => emit_app_draw_text(symbol, term_state_offset),
        "term.clear" => emit_app_clear(symbol, term_state_offset),
        "term.sync" => emit_app_term_sync(symbol, term_state_offset),
        "term.showCursor" => emit_app_set_cursor_visible(symbol, term_state_offset, "1"),
        "term.hideCursor" => emit_app_set_cursor_visible(symbol, term_state_offset, "0"),
        "term.terminalSize" => emit_app_terminal_size(symbol, term_state_offset),
        "term.didResize" => emit_app_did_resize(symbol),
        _ => return None,
    };
    Some(helper)
}

/// `term::didResize()` app arm (planning/term.md #11): OK(Boolean) = the cached
/// resize flag on TVSTATE, read-and-cleared so it latches from a genuine window
/// resize (set by the `setFrameSize:` IMP) until the program observes it. Headless
/// (no attached surface) reads false. Result ABI x0=tag, x1=value.
fn emit_app_did_resize(symbol: &str) -> AppHookBody {
    let mut asm = Asm::new(symbol);
    // Frame: lr@0, x20(state)@8.
    let frame = 16;
    let false_label = format!("{symbol}_false");
    let done = format!("{symbol}_done");
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(frame));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        0,
    ));
    asm.push(abi::store_u64(abi::LOCAL[1], abi::stack_pointer(), 8));
    // Load TVSTATE (clobbers x0/x1, not x19); nil (headless) → false.
    emit_get_tv_state(&mut asm, abi::LOCAL[1], &false_label);
    // value = TVSTATE.didResize, then clear it (read-and-clear).
    asm.push(abi::load_u64("x1", abi::LOCAL[1], TV_DID_RESIZE_OFFSET));
    asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
    asm.push(abi::store_u64(
        abi::SCRATCH[0],
        abi::LOCAL[1],
        TV_DID_RESIZE_OFFSET,
    ));
    asm.push(abi::move_immediate("x0", "Integer", "0")); // OK tag
    asm.push(abi::branch(&done));
    asm.push(abi::label(&false_label));
    asm.push(abi::move_immediate("x1", "Integer", "0")); // false
    asm.push(abi::move_immediate("x0", "Integer", "0")); // OK tag
    asm.push(abi::label(&done));
    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::load_u64(abi::LOCAL[1], abi::stack_pointer(), 8));
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

/// `term::sync()` app arm (plan-35-D §3). The single present: marshal a
/// `setNeedsDisplay:` onto the TermView so the coalesced frame is drawn once. A
/// clean no-op while TUI mode is off (the active gate) or when no surface is
/// attached (headless). This is the *only* redraw trigger for grid writes —
/// `mfbWriteString:`/`clear` no longer request their own redraw (mandatory
/// present, plan-35 D1).
fn emit_app_term_sync(symbol: &str, term_state_offset: usize) -> AppHookBody {
    let mut asm = Asm::new(symbol);
    let frame = 32; // lr@0, x20(termView)@8, x21(sel)@16
    let done = format!("{symbol}_done");
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(frame));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        0,
    ));
    asm.push(abi::store_u64(abi::LOCAL[1], abi::stack_pointer(), 8));
    asm.push(abi::store_u64(abi::LOCAL[2], abi::stack_pointer(), 16));
    emit_term_active_gate(&mut asm, term_state_offset, &done);
    emit_present_needs_display(&mut asm, &done);
    asm.push(abi::label(&done));
    emit_term_ok_return(&mut asm, frame, &[(abi::LOCAL[1], 8), (abi::LOCAL[2], 16)]);
    (
        CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        asm.ins,
        asm.rel,
    )
}

/// Marshal a `setNeedsDisplay:` present onto the TermView on the main thread —
/// the coalesced single redraw shared by `term::sync` and app-mode `io::flush`.
/// Loads the TermView off NSApp; branches to `done` when no surface is attached
/// (headless). Uses `x20` (termView) and `x21` (sel), which the caller must have
/// spilled. Does not touch `x19` (the pinned arena-state base). The present is
/// marshaled `waitUntilDone:YES` the same way grid writes are, so a `sync` cannot
/// race ahead of the writes it should show (plan-35-D §3).
fn emit_present_needs_display(asm: &mut Asm, done: &str) {
    // tv = objc_getAssociatedObject([NSApplication sharedApplication], &TERMVIEW_KEY)
    asm.external_data(abi::LOCAL[1], CLASS_NS_APPLICATION, LIB_APPKIT);
    asm.load_selector(SEL_SHARED_APPLICATION.0);
    asm.push(abi::move_register("x0", abi::LOCAL[1]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.local_address("x1", TERMVIEW_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[1], "x0")); // termView or nil
    asm.push(abi::compare_immediate(abi::LOCAL[1], "0"));
    asm.push(abi::branch_eq(done));
    // [tv performSelectorOnMainThread:@selector(setNeedsDisplay:) withObject:tv
    //  waitUntilDone:YES] — any non-nil withObject reads as BOOL YES.
    asm.load_selector(SEL_SET_NEEDS_DISPLAY.0);
    asm.push(abi::move_register(abi::LOCAL[2], "x1"));
    asm.load_selector(SEL_PERFORM_ON_MAIN.0);
    asm.push(abi::move_register("x2", abi::LOCAL[2]));
    asm.push(abi::move_register("x3", abi::LOCAL[1]));
    asm.push(abi::move_immediate("x4", "Integer", "1")); // waitUntilDone: YES
    asm.push(abi::move_register("x0", abi::LOCAL[1]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
}

/// Branch to `done` when TUI mode is inactive (the §4.2.1 no-op gate). `x19` is
/// the pinned arena-state base holding the term-state global.
fn emit_term_active_gate(asm: &mut Asm, term_state_offset: usize, done: &str) {
    asm.push(abi::load_u64(
        abi::SCRATCH[0],
        TERM_ARENA_STATE_REG,
        term_state_offset + code::TERM_STATE_ACTIVE_OFFSET,
    ));
    asm.push(abi::compare_immediate(abi::SCRATCH[0], "0"));
    asm.push(abi::branch_eq(done));
}

/// Load the TermView's grid-state struct into `state_reg` (callee-saved); branch
/// to `nil_label` when no surface is attached (headless). Clobbers x0/x1 and the
/// objc call-clobbered registers, but not `x19` (the arena-state base).
fn emit_get_tv_state(asm: &mut Asm, state_reg: &str, nil_label: &str) {
    asm.external_data(state_reg, CLASS_NS_APPLICATION, LIB_APPKIT);
    asm.load_selector(SEL_SHARED_APPLICATION.0);
    asm.push(abi::move_register("x0", state_reg));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.local_address("x1", TERMVIEW_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::compare_immediate("x0", "0"));
    asm.push(abi::branch_eq(nil_label));
    asm.local_address("x1", TVSTATE_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register(state_reg, "x0"));
    asm.push(abi::compare_immediate(state_reg, "0"));
    asm.push(abi::branch_eq(nil_label));
}

/// Standard term runtime-helper epilogue: `x0 = RESULT_OK_TAG`, restore lr + the
/// listed callee-saved registers, pop the frame, return.
fn emit_term_ok_return(asm: &mut Asm, frame: usize, saved: &[(&str, usize)]) {
    asm.push(abi::move_immediate("x0", "Integer", "0")); // RESULT_OK_TAG
    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    for (reg, off) in saved {
        asm.push(abi::load_u64(reg, abi::stack_pointer(), *off));
    }
    asm.push(abi::add_stack(frame));
    asm.push(abi::return_());
}

/// `term::setForeground`/`setBackground` app body: pack r/g/b and store it to the
/// term-state global (so the console-backed getters stay correct) and to the
/// TermView's current-attribute field (so the write path tags cells with it).
fn emit_app_set_color(
    symbol: &str,
    term_state_offset: usize,
    global_field: usize,
    tv_field: usize,
) -> AppHookBody {
    let mut asm = Asm::new(symbol);
    // Frame: lr@0, x20(state)@8, r/g/b (then packed) @16/@24/@32.
    let frame = 48;
    let done = format!("{symbol}_done");
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(frame));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        0,
    ));
    asm.push(abi::store_u64(abi::LOCAL[1], abi::stack_pointer(), 8));
    asm.push(abi::store_u64("x0", abi::stack_pointer(), 16)); // r
    asm.push(abi::store_u64("x1", abi::stack_pointer(), 24)); // g
    asm.push(abi::store_u64("x2", abi::stack_pointer(), 32)); // b
    emit_term_active_gate(&mut asm, term_state_offset, &done);
    // packed = r | g<<8 | b<<16
    asm.push(abi::load_u64(abi::SCRATCH[0], abi::stack_pointer(), 16));
    asm.push(abi::load_u64(abi::SCRATCH[1], abi::stack_pointer(), 24));
    asm.push(abi::load_u64(abi::SCRATCH[2], abi::stack_pointer(), 32));
    asm.push(abi::shift_left_immediate(
        abi::SCRATCH[1],
        abi::SCRATCH[1],
        8,
    ));
    asm.push(abi::shift_left_immediate(
        abi::SCRATCH[2],
        abi::SCRATCH[2],
        16,
    ));
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
    asm.push(abi::store_u64(abi::SCRATCH[0], abi::stack_pointer(), 16)); // keep packed across calls
    emit_get_tv_state(&mut asm, abi::LOCAL[1], &done);
    asm.push(abi::load_u64(abi::SCRATCH[0], abi::stack_pointer(), 16));
    asm.push(abi::store_u64(abi::SCRATCH[0], abi::LOCAL[1], tv_field));
    asm.push(abi::label(&done));
    emit_term_ok_return(&mut asm, frame, &[(abi::LOCAL[1], 8)]);
    (
        CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        asm.ins,
        asm.rel,
    )
}

/// `term::setBold`/`setUnderline` app body: store the flag to the term-state
/// global and the TermView current-attribute field.
fn emit_app_set_attr(
    symbol: &str,
    term_state_offset: usize,
    global_field: usize,
    tv_field: usize,
) -> AppHookBody {
    let mut asm = Asm::new(symbol);
    let frame = 32; // lr@0, x20(state)@8, enabled@16
    let done = format!("{symbol}_done");
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(frame));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        0,
    ));
    asm.push(abi::store_u64(abi::LOCAL[1], abi::stack_pointer(), 8));
    asm.push(abi::store_u64("x0", abi::stack_pointer(), 16)); // enabled
    emit_term_active_gate(&mut asm, term_state_offset, &done);
    asm.push(abi::load_u64(abi::SCRATCH[0], abi::stack_pointer(), 16));
    asm.push(abi::store_u64(
        abi::SCRATCH[0],
        TERM_ARENA_STATE_REG,
        term_state_offset + global_field,
    ));
    emit_get_tv_state(&mut asm, abi::LOCAL[1], &done);
    asm.push(abi::load_u64(abi::SCRATCH[0], abi::stack_pointer(), 16));
    asm.push(abi::store_u64(abi::SCRATCH[0], abi::LOCAL[1], tv_field));
    asm.push(abi::label(&done));
    emit_term_ok_return(&mut asm, frame, &[(abi::LOCAL[1], 8)]);
    (
        CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        asm.ins,
        asm.rel,
    )
}

/// `term::moveTo(row, col)` app body: clamp to `[0, rows-1] x [0, cols-1]` and
/// store into the TermView cursor (plan §4.5).
fn emit_app_move_to(symbol: &str, term_state_offset: usize) -> AppHookBody {
    let mut asm = Asm::new(symbol);
    let frame = 32; // lr@0, x20(state)@8, row@16, col@24
    let done = format!("{symbol}_done");
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(frame));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        0,
    ));
    asm.push(abi::store_u64(abi::LOCAL[1], abi::stack_pointer(), 8));
    asm.push(abi::store_u64("x0", abi::stack_pointer(), 16)); // row
    asm.push(abi::store_u64("x1", abi::stack_pointer(), 24)); // col
    emit_term_active_gate(&mut asm, term_state_offset, &done);
    emit_get_tv_state(&mut asm, abi::LOCAL[1], &done);
    // row = clamp(row, 0, rows-1)
    asm.push(abi::load_u64(abi::SCRATCH[0], abi::stack_pointer(), 16));
    let row_lo = format!("{symbol}_row_lo");
    let row_hi = format!("{symbol}_row_hi");
    asm.push(abi::compare_immediate(abi::SCRATCH[0], "0"));
    asm.push(abi::branch_ge(&row_lo));
    asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
    asm.push(abi::label(&row_lo));
    asm.push(abi::load_u64(
        abi::SCRATCH[1],
        abi::LOCAL[1],
        TV_ROWS_OFFSET,
    ));
    asm.push(abi::subtract_immediate(abi::SCRATCH[1], abi::SCRATCH[1], 1));
    asm.push(abi::compare_registers(abi::SCRATCH[0], abi::SCRATCH[1]));
    asm.push(abi::branch_le(&row_hi));
    asm.push(abi::move_register(abi::SCRATCH[0], abi::SCRATCH[1]));
    asm.push(abi::label(&row_hi));
    asm.push(abi::store_u64(
        abi::SCRATCH[0],
        abi::LOCAL[1],
        TV_CURSOR_ROW_OFFSET,
    ));
    // col = clamp(col, 0, cols-1)
    asm.push(abi::load_u64(abi::SCRATCH[0], abi::stack_pointer(), 24));
    let col_lo = format!("{symbol}_col_lo");
    let col_hi = format!("{symbol}_col_hi");
    asm.push(abi::compare_immediate(abi::SCRATCH[0], "0"));
    asm.push(abi::branch_ge(&col_lo));
    asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
    asm.push(abi::label(&col_lo));
    asm.push(abi::load_u64(
        abi::SCRATCH[1],
        abi::LOCAL[1],
        TV_COLS_OFFSET,
    ));
    asm.push(abi::subtract_immediate(abi::SCRATCH[1], abi::SCRATCH[1], 1));
    asm.push(abi::compare_registers(abi::SCRATCH[0], abi::SCRATCH[1]));
    asm.push(abi::branch_le(&col_hi));
    asm.push(abi::move_register(abi::SCRATCH[0], abi::SCRATCH[1]));
    asm.push(abi::label(&col_hi));
    asm.push(abi::store_u64(
        abi::SCRATCH[0],
        abi::LOCAL[1],
        TV_CURSOR_COL_OFFSET,
    ));
    asm.push(abi::label(&done));
    emit_term_ok_return(&mut asm, frame, &[(abi::LOCAL[1], 8)]);
    (
        CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        asm.ins,
        asm.rel,
    )
}

/// `term::clear` app body: clear the grid + home the cursor (worker side). The
/// surface is repainted only on the next present (`term::sync`/`io::flush`), not
/// per clear — redraw is present-driven (plan-35-D §3, mandatory present).
fn emit_app_clear(symbol: &str, term_state_offset: usize) -> AppHookBody {
    let mut asm = Asm::new(symbol);
    let frame = 32; // lr@0, x20(termView)@8, x21(mfbClear: sel)@16
    let done = format!("{symbol}_done");
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(frame));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        0,
    ));
    asm.push(abi::store_u64(abi::LOCAL[1], abi::stack_pointer(), 8));
    asm.push(abi::store_u64(abi::LOCAL[2], abi::stack_pointer(), 16));
    emit_term_active_gate(&mut asm, term_state_offset, &done);
    // tv = objc_getAssociatedObject([NSApplication sharedApplication], &TERMVIEW_KEY)
    asm.external_data(abi::LOCAL[1], CLASS_NS_APPLICATION, LIB_APPKIT);
    asm.load_selector(SEL_SHARED_APPLICATION.0);
    asm.push(abi::move_register("x0", abi::LOCAL[1]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.local_address("x1", TERMVIEW_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[1], "x0")); // termView or nil
    asm.push(abi::compare_immediate(abi::LOCAL[1], "0"));
    asm.push(abi::branch_eq(&done));
    // Marshal the grid clear onto the main thread (bug-165): the cell buffer is
    // realloc/free'd by `setFrameSize:` on the main thread during a live window
    // resize, so mutating it directly from the worker is a use-after-free. Run it
    // through the `mfbClear:` selector like `mfbWriteString:` does — no redraw, the
    // repaint is present-driven (plan-35-D §3).
    // [tv performSelectorOnMainThread:@selector(mfbClear:) withObject:nil waitUntilDone:YES]
    asm.load_selector(SEL_MFB_CLEAR.0);
    asm.push(abi::move_register(abi::LOCAL[2], "x1")); // mfbClear: sel
    asm.load_selector(SEL_PERFORM_ON_MAIN.0);
    asm.push(abi::move_register("x2", abi::LOCAL[2]));
    asm.push(abi::move_immediate("x3", "Integer", "0")); // withObject: nil
    asm.push(abi::move_immediate("x4", "Integer", "1")); // waitUntilDone: YES
    asm.push(abi::move_register("x0", abi::LOCAL[1]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::label(&done));
    emit_term_ok_return(&mut asm, frame, &[(abi::LOCAL[1], 8), (abi::LOCAL[2], 16)]);
    (
        CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        asm.ins,
        asm.rel,
    )
}

/// `term::drawHLine`/`drawVLine` app body: resolve the `LineStyle` ordinal to a
/// `unichar` glyph, park it plus the fixed line and span endpoints in the TermView
/// state, then marshal `mfbDrawLine:` onto the main thread (waitUntilDone:YES) so
/// the cell buffer is mutated there (bug-165), matching `mfbClear:`. `is_horizontal`
/// selects the glyph table at emit time; the main-thread IMP clamps the span to the
/// current grid. The repaint is present-driven (plan-35-D §3).
fn emit_app_draw_line(symbol: &str, term_state_offset: usize, is_horizontal: bool) -> AppHookBody {
    let mut asm = Asm::new(symbol);
    // Frame: lr@0, x20(tv)@8, x21(state)@16, x22(sel)@24, ord@32, fixed@40, lo@48,
    // hi@56.
    let frame = 64;
    let done = format!("{symbol}_done");
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(frame));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        0,
    ));
    asm.push(abi::store_u64(abi::LOCAL[1], abi::stack_pointer(), 8));
    asm.push(abi::store_u64(abi::LOCAL[2], abi::stack_pointer(), 16));
    asm.push(abi::store_u64(abi::LOCAL[3], abi::stack_pointer(), 24));
    asm.push(abi::store_u64("x0", abi::stack_pointer(), 32)); // ordinal
    asm.push(abi::store_u64("x1", abi::stack_pointer(), 40)); // fixed
    asm.push(abi::store_u64("x2", abi::stack_pointer(), 48)); // lo endpoint
    asm.push(abi::store_u64("x3", abi::stack_pointer(), 56)); // hi endpoint
    emit_term_active_gate(&mut asm, term_state_offset, &done);
    // tv = objc_getAssociatedObject([NSApplication sharedApplication], &TERMVIEW_KEY)
    asm.external_data(abi::LOCAL[1], CLASS_NS_APPLICATION, LIB_APPKIT);
    asm.load_selector(SEL_SHARED_APPLICATION.0);
    asm.push(abi::move_register("x0", abi::LOCAL[1]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.local_address("x1", TERMVIEW_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[1], "x0")); // termView or nil
    asm.push(abi::compare_immediate(abi::LOCAL[1], "0"));
    asm.push(abi::branch_eq(&done));
    // state = objc_getAssociatedObject(tv, &TVSTATE_KEY)
    asm.push(abi::move_register("x0", abi::LOCAL[1]));
    asm.local_address("x1", TVSTATE_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[2], "x0")); // state or nil
    asm.push(abi::compare_immediate(abi::LOCAL[2], "0"));
    asm.push(abi::branch_eq(&done));
    // Resolve the glyph from the parked ordinal; ordinal 0 (Light) is the
    // fall-through default. The table is chosen at emit time (this body is emitted
    // separately for drawHLine and drawVLine).
    let table: &[u32; 7] = if is_horizontal {
        &code::TERM_HLINE_CODEPOINTS
    } else {
        &code::TERM_VLINE_CODEPOINTS
    };
    let gdone = format!("{symbol}_gdone");
    asm.push(abi::load_u64(abi::SCRATCH[0], abi::stack_pointer(), 32)); // ordinal
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
        abi::LOCAL[2],
        TV_DRAW_GLYPH_OFFSET,
    ));
    asm.push(abi::load_u64(abi::SCRATCH[0], abi::stack_pointer(), 40));
    asm.push(abi::store_u64(
        abi::SCRATCH[0],
        abi::LOCAL[2],
        TV_DRAW_FIXED_OFFSET,
    ));
    asm.push(abi::load_u64(abi::SCRATCH[0], abi::stack_pointer(), 48));
    asm.push(abi::store_u64(
        abi::SCRATCH[0],
        abi::LOCAL[2],
        TV_DRAW_LO_OFFSET,
    ));
    asm.push(abi::load_u64(abi::SCRATCH[0], abi::stack_pointer(), 56));
    asm.push(abi::store_u64(
        abi::SCRATCH[0],
        abi::LOCAL[2],
        TV_DRAW_HI_OFFSET,
    ));
    asm.push(abi::move_immediate(
        abi::SCRATCH[0],
        "Integer",
        if is_horizontal { "1" } else { "0" },
    ));
    asm.push(abi::store_u64(
        abi::SCRATCH[0],
        abi::LOCAL[2],
        TV_DRAW_HORIZ_OFFSET,
    ));
    // [tv performSelectorOnMainThread:@selector(mfbDrawLine:) withObject:nil
    //     waitUntilDone:YES]
    asm.load_selector(SEL_MFB_DRAW_LINE.0);
    asm.push(abi::move_register(abi::LOCAL[3], "x1")); // mfbDrawLine: sel
    asm.load_selector(SEL_PERFORM_ON_MAIN.0);
    asm.push(abi::move_register("x2", abi::LOCAL[3]));
    asm.push(abi::move_immediate("x3", "Integer", "0")); // withObject: nil
    asm.push(abi::move_immediate("x4", "Integer", "1")); // waitUntilDone: YES
    asm.push(abi::move_register("x0", abi::LOCAL[1]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::label(&done));
    emit_term_ok_return(
        &mut asm,
        frame,
        &[(abi::LOCAL[1], 8), (abi::LOCAL[2], 16), (abi::LOCAL[3], 24)],
    );
    (
        CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        asm.ins,
        asm.rel,
    )
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
fn emit_app_draw_box(symbol: &str, term_state_offset: usize) -> AppHookBody {
    let mut asm = Asm::new(symbol);
    // Frame: lr@0, x20(tv)@8, x21(state)@16, x22(sel)@24, ord@32, x1@40, y1@48,
    // x2@56, y2@64.
    let frame = 80;
    let done = format!("{symbol}_done");
    let ord = abi::SCRATCH[0];
    let dst = abi::SCRATCH[1];
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(frame));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        0,
    ));
    asm.push(abi::store_u64(abi::LOCAL[1], abi::stack_pointer(), 8));
    asm.push(abi::store_u64(abi::LOCAL[2], abi::stack_pointer(), 16));
    asm.push(abi::store_u64(abi::LOCAL[3], abi::stack_pointer(), 24));
    asm.push(abi::store_u64("x0", abi::stack_pointer(), 32)); // ordinal
    asm.push(abi::store_u64("x1", abi::stack_pointer(), 40)); // x1
    asm.push(abi::store_u64("x2", abi::stack_pointer(), 48)); // y1
    asm.push(abi::store_u64("x3", abi::stack_pointer(), 56)); // x2
    asm.push(abi::store_u64("x4", abi::stack_pointer(), 64)); // y2
    emit_term_active_gate(&mut asm, term_state_offset, &done);
    // tv = objc_getAssociatedObject([NSApplication sharedApplication], &TERMVIEW_KEY)
    asm.external_data(abi::LOCAL[1], CLASS_NS_APPLICATION, LIB_APPKIT);
    asm.load_selector(SEL_SHARED_APPLICATION.0);
    asm.push(abi::move_register("x0", abi::LOCAL[1]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.local_address("x1", TERMVIEW_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[1], "x0")); // termView or nil
    asm.push(abi::compare_immediate(abi::LOCAL[1], "0"));
    asm.push(abi::branch_eq(&done));
    // state = objc_getAssociatedObject(tv, &TVSTATE_KEY)
    asm.push(abi::move_register("x0", abi::LOCAL[1]));
    asm.local_address("x1", TVSTATE_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[2], "x0")); // state or nil
    asm.push(abi::compare_immediate(abi::LOCAL[2], "0"));
    asm.push(abi::branch_eq(&done));
    // Resolve the six glyphs from the parked ordinal (no calls until the marshal,
    // so `ord`/`dst` stay stable).
    asm.push(abi::load_u64(ord, abi::stack_pointer(), 32));
    let glyphs: [(&[u32; 7], usize, &str); 6] = [
        (&code::TERM_HLINE_CODEPOINTS, TV_BOX_HG_OFFSET, "hg"),
        (&code::TERM_VLINE_CODEPOINTS, TV_BOX_VG_OFFSET, "vg"),
        (&code::TERM_CORNER_TL_CODEPOINTS, TV_BOX_CTL_OFFSET, "tl"),
        (&code::TERM_CORNER_TR_CODEPOINTS, TV_BOX_CTR_OFFSET, "tr"),
        (&code::TERM_CORNER_BL_CODEPOINTS, TV_BOX_CBL_OFFSET, "bl"),
        (&code::TERM_CORNER_BR_CODEPOINTS, TV_BOX_CBR_OFFSET, "br"),
    ];
    for (table, off, tag) in glyphs {
        emit_app_select_unichar(&mut asm, ord, dst, table, &format!("{symbol}_box_{tag}"));
        asm.push(abi::store_u64(dst, abi::LOCAL[2], off));
    }
    // Park the two raw corner points (normalised/clamped on the main thread).
    for (slot, off) in [
        (40usize, TV_BOX_X1_OFFSET),
        (48, TV_BOX_Y1_OFFSET),
        (56, TV_BOX_X2_OFFSET),
        (64, TV_BOX_Y2_OFFSET),
    ] {
        asm.push(abi::load_u64(dst, abi::stack_pointer(), slot));
        asm.push(abi::store_u64(dst, abi::LOCAL[2], off));
    }
    // [tv performSelectorOnMainThread:@selector(mfbDrawBox:) withObject:nil
    //     waitUntilDone:YES]
    asm.load_selector(SEL_MFB_DRAW_BOX.0);
    asm.push(abi::move_register(abi::LOCAL[3], "x1")); // mfbDrawBox: sel
    asm.load_selector(SEL_PERFORM_ON_MAIN.0);
    asm.push(abi::move_register("x2", abi::LOCAL[3]));
    asm.push(abi::move_immediate("x3", "Integer", "0")); // withObject: nil
    asm.push(abi::move_immediate("x4", "Integer", "1")); // waitUntilDone: YES
    asm.push(abi::move_register("x0", abi::LOCAL[1]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::label(&done));
    emit_term_ok_return(
        &mut asm,
        frame,
        &[(abi::LOCAL[1], 8), (abi::LOCAL[2], 16), (abi::LOCAL[3], 24)],
    );
    (
        CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        asm.ins,
        asm.rel,
    )
}

/// `term::fillRect` app body: resolve the `FillStyle` ordinal to a unichar, park
/// it plus the two raw corner points in the TermView state, then marshal
/// `mfbFillRect:` onto the main thread (waitUntilDone:YES), which normalises,
/// clamps, and fills. Present-driven.
fn emit_app_fill_rect(symbol: &str, term_state_offset: usize) -> AppHookBody {
    let mut asm = Asm::new(symbol);
    // Frame: lr@0, x20(tv)@8, x21(state)@16, x22(sel)@24, ord@32, x1@40, y1@48,
    // x2@56, y2@64.
    let frame = 80;
    let done = format!("{symbol}_done");
    let ord = abi::SCRATCH[0];
    let dst = abi::SCRATCH[1];
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(frame));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        0,
    ));
    asm.push(abi::store_u64(abi::LOCAL[1], abi::stack_pointer(), 8));
    asm.push(abi::store_u64(abi::LOCAL[2], abi::stack_pointer(), 16));
    asm.push(abi::store_u64(abi::LOCAL[3], abi::stack_pointer(), 24));
    asm.push(abi::store_u64("x0", abi::stack_pointer(), 32)); // ordinal
    asm.push(abi::store_u64("x1", abi::stack_pointer(), 40)); // x1
    asm.push(abi::store_u64("x2", abi::stack_pointer(), 48)); // y1
    asm.push(abi::store_u64("x3", abi::stack_pointer(), 56)); // x2
    asm.push(abi::store_u64("x4", abi::stack_pointer(), 64)); // y2
    emit_term_active_gate(&mut asm, term_state_offset, &done);
    asm.external_data(abi::LOCAL[1], CLASS_NS_APPLICATION, LIB_APPKIT);
    asm.load_selector(SEL_SHARED_APPLICATION.0);
    asm.push(abi::move_register("x0", abi::LOCAL[1]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.local_address("x1", TERMVIEW_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[1], "x0")); // termView or nil
    asm.push(abi::compare_immediate(abi::LOCAL[1], "0"));
    asm.push(abi::branch_eq(&done));
    asm.push(abi::move_register("x0", abi::LOCAL[1]));
    asm.local_address("x1", TVSTATE_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[2], "x0")); // state or nil
    asm.push(abi::compare_immediate(abi::LOCAL[2], "0"));
    asm.push(abi::branch_eq(&done));
    // Resolve the fill glyph from the parked ordinal.
    asm.push(abi::load_u64(ord, abi::stack_pointer(), 32));
    emit_app_select_unichar(
        &mut asm,
        ord,
        dst,
        &code::TERM_FILL_CODEPOINTS,
        &format!("{symbol}_fill"),
    );
    asm.push(abi::store_u64(dst, abi::LOCAL[2], TV_FILL_GLYPH_OFFSET));
    for (slot, off) in [
        (40usize, TV_FILL_X1_OFFSET),
        (48, TV_FILL_Y1_OFFSET),
        (56, TV_FILL_X2_OFFSET),
        (64, TV_FILL_Y2_OFFSET),
    ] {
        asm.push(abi::load_u64(dst, abi::stack_pointer(), slot));
        asm.push(abi::store_u64(dst, abi::LOCAL[2], off));
    }
    // [tv performSelectorOnMainThread:@selector(mfbFillRect:) withObject:nil
    //     waitUntilDone:YES]
    asm.load_selector(SEL_MFB_FILL_RECT.0);
    asm.push(abi::move_register(abi::LOCAL[3], "x1")); // mfbFillRect: sel
    asm.load_selector(SEL_PERFORM_ON_MAIN.0);
    asm.push(abi::move_register("x2", abi::LOCAL[3]));
    asm.push(abi::move_immediate("x3", "Integer", "0")); // withObject: nil
    asm.push(abi::move_immediate("x4", "Integer", "1")); // waitUntilDone: YES
    asm.push(abi::move_register("x0", abi::LOCAL[1]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::label(&done));
    emit_term_ok_return(
        &mut asm,
        frame,
        &[(abi::LOCAL[1], 8), (abi::LOCAL[2], 16), (abi::LOCAL[3], 24)],
    );
    (
        CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        asm.ins,
        asm.rel,
    )
}

/// `term::drawGlyph` app body: park the code point (as a unichar) + cell in the
/// TermView state and marshal `mfbDrawGlyph:` onto the main thread. Control code
/// points (< 0x20) are skipped (they would corrupt the surface). Present-driven.
fn emit_app_draw_glyph(symbol: &str, term_state_offset: usize) -> AppHookBody {
    let mut asm = Asm::new(symbol);
    // Frame: lr@0, x20(tv)@8, x21(state)@16, x22(sel)@24, x@32, y@40, cp@48.
    let frame = 64;
    let done = format!("{symbol}_done");
    let v = abi::SCRATCH[0];
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(frame));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        0,
    ));
    asm.push(abi::store_u64(abi::LOCAL[1], abi::stack_pointer(), 8));
    asm.push(abi::store_u64(abi::LOCAL[2], abi::stack_pointer(), 16));
    asm.push(abi::store_u64(abi::LOCAL[3], abi::stack_pointer(), 24));
    asm.push(abi::store_u64("x0", abi::stack_pointer(), 32)); // x
    asm.push(abi::store_u64("x1", abi::stack_pointer(), 40)); // y
    asm.push(abi::store_u64("x2", abi::stack_pointer(), 48)); // codepoint
    emit_term_active_gate(&mut asm, term_state_offset, &done);
    // Skip control code points.
    asm.push(abi::load_u64(v, abi::stack_pointer(), 48));
    asm.push(abi::compare_immediate(v, "32"));
    asm.push(abi::branch_lt(&done));
    asm.external_data(abi::LOCAL[1], CLASS_NS_APPLICATION, LIB_APPKIT);
    asm.load_selector(SEL_SHARED_APPLICATION.0);
    asm.push(abi::move_register("x0", abi::LOCAL[1]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.local_address("x1", TERMVIEW_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[1], "x0"));
    asm.push(abi::compare_immediate(abi::LOCAL[1], "0"));
    asm.push(abi::branch_eq(&done));
    asm.push(abi::move_register("x0", abi::LOCAL[1]));
    asm.local_address("x1", TVSTATE_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[2], "x0"));
    asm.push(abi::compare_immediate(abi::LOCAL[2], "0"));
    asm.push(abi::branch_eq(&done));
    for (slot, off) in [
        (48usize, TV_GLYPH_G_OFFSET),
        (32, TV_GLYPH_X_OFFSET),
        (40, TV_GLYPH_Y_OFFSET),
    ] {
        asm.push(abi::load_u64(v, abi::stack_pointer(), slot));
        asm.push(abi::store_u64(v, abi::LOCAL[2], off));
    }
    asm.load_selector(SEL_MFB_DRAW_GLYPH.0);
    asm.push(abi::move_register(abi::LOCAL[3], "x1"));
    asm.load_selector(SEL_PERFORM_ON_MAIN.0);
    asm.push(abi::move_register("x2", abi::LOCAL[3]));
    asm.push(abi::move_immediate("x3", "Integer", "0"));
    asm.push(abi::move_immediate("x4", "Integer", "1"));
    asm.push(abi::move_register("x0", abi::LOCAL[1]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::label(&done));
    emit_term_ok_return(
        &mut asm,
        frame,
        &[(abi::LOCAL[1], 8), (abi::LOCAL[2], 16), (abi::LOCAL[3], 24)],
    );
    (
        CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        asm.ins,
        asm.rel,
    )
}

/// `term::drawText` app body: park the start cell in the TermView state, build an
/// NSString from the mfb string, marshal `mfbDrawText:` onto the main thread (the
/// NSString as the object argument, like `mfbWriteString:`), then release it.
/// Present-driven.
fn emit_app_draw_text(symbol: &str, term_state_offset: usize) -> AppHookBody {
    let mut asm = Asm::new(symbol);
    // Frame: lr@0, x20(tv)@8, x21(state/nsstr)@16, x22(sel)@24, x@32, y@40,
    // strobj@48.
    let frame = 64;
    let done = format!("{symbol}_done");
    let v = abi::SCRATCH[0];
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(frame));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        0,
    ));
    asm.push(abi::store_u64(abi::LOCAL[1], abi::stack_pointer(), 8));
    asm.push(abi::store_u64(abi::LOCAL[2], abi::stack_pointer(), 16));
    asm.push(abi::store_u64(abi::LOCAL[3], abi::stack_pointer(), 24));
    asm.push(abi::store_u64("x0", abi::stack_pointer(), 32)); // x
    asm.push(abi::store_u64("x1", abi::stack_pointer(), 40)); // y
    asm.push(abi::store_u64("x2", abi::stack_pointer(), 48)); // strobj (mfb String)
    emit_term_active_gate(&mut asm, term_state_offset, &done);
    asm.external_data(abi::LOCAL[1], CLASS_NS_APPLICATION, LIB_APPKIT);
    asm.load_selector(SEL_SHARED_APPLICATION.0);
    asm.push(abi::move_register("x0", abi::LOCAL[1]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.local_address("x1", TERMVIEW_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[1], "x0")); // tv
    asm.push(abi::compare_immediate(abi::LOCAL[1], "0"));
    asm.push(abi::branch_eq(&done));
    asm.push(abi::move_register("x0", abi::LOCAL[1]));
    asm.local_address("x1", TVSTATE_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[2], "x0")); // state
    asm.push(abi::compare_immediate(abi::LOCAL[2], "0"));
    asm.push(abi::branch_eq(&done));
    for (slot, off) in [(32usize, TV_TEXT_X_OFFSET), (40, TV_TEXT_Y_OFFSET)] {
        asm.push(abi::load_u64(v, abi::stack_pointer(), slot));
        asm.push(abi::store_u64(v, abi::LOCAL[2], off));
    }
    // nsstr = [[NSString alloc] initWithBytes:(strobj+8) length:strobj[0] encoding:UTF8]
    asm.external_data(abi::LOCAL[2], CLASS_NS_STRING, LIB_FOUNDATION);
    asm.load_selector(SEL_ALLOC.0);
    asm.push(abi::move_register("x0", abi::LOCAL[2]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[2], "x0"));
    asm.load_selector(SEL_INIT_WITH_BYTES.0);
    asm.push(abi::load_u64(v, abi::stack_pointer(), 48)); // strobj
    asm.push(abi::add_immediate("x2", v, 8));
    asm.push(abi::load_u64("x3", v, 0));
    asm.push(abi::move_immediate("x4", "Integer", NS_UTF8_ENCODING));
    asm.push(abi::move_register("x0", abi::LOCAL[2]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register(abi::LOCAL[2], "x0")); // nsstr
                                                       // [tv performSelectorOnMainThread:mfbDrawText: withObject:nsstr waitUntilDone:YES]
    asm.load_selector(SEL_MFB_DRAW_TEXT.0);
    asm.push(abi::move_register(abi::LOCAL[3], "x1"));
    asm.load_selector(SEL_PERFORM_ON_MAIN.0);
    asm.push(abi::move_register("x2", abi::LOCAL[3]));
    asm.push(abi::move_register("x3", abi::LOCAL[2]));
    asm.push(abi::move_immediate("x4", "Integer", "1"));
    asm.push(abi::move_register("x0", abi::LOCAL[1]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    // [nsstr release] — owned (alloc+init); mfbDrawText: only read it (synchronous).
    asm.load_selector(SEL_RELEASE.0);
    asm.push(abi::move_register("x0", abi::LOCAL[2]));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::label(&done));
    emit_term_ok_return(
        &mut asm,
        frame,
        &[(abi::LOCAL[1], 8), (abi::LOCAL[2], 16), (abi::LOCAL[3], 24)],
    );
    (
        CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        asm.ins,
        asm.rel,
    )
}

/// `term::showCursor`/`hideCursor` app body: store the cursor-visible flag into
/// the TermView state (and the term-state global). Cursor glyph rendering is a
/// later refinement, so no redraw is needed yet.
fn emit_app_set_cursor_visible(symbol: &str, term_state_offset: usize, value: &str) -> AppHookBody {
    let mut asm = Asm::new(symbol);
    let frame = 16; // lr@0, x20(state)@8
    let done = format!("{symbol}_done");
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(frame));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        0,
    ));
    asm.push(abi::store_u64(abi::LOCAL[1], abi::stack_pointer(), 8));
    emit_term_active_gate(&mut asm, term_state_offset, &done);
    asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", value));
    asm.push(abi::store_u64(
        abi::SCRATCH[0],
        TERM_ARENA_STATE_REG,
        term_state_offset + code::TERM_STATE_CURSOR_VISIBLE_OFFSET,
    ));
    emit_get_tv_state(&mut asm, abi::LOCAL[1], &done);
    asm.push(abi::move_immediate(abi::SCRATCH[0], "Integer", value));
    asm.push(abi::store_u64(
        abi::SCRATCH[0],
        abi::LOCAL[1],
        TV_CURSOR_VISIBLE_OFFSET,
    ));
    asm.push(abi::label(&done));
    emit_term_ok_return(&mut asm, frame, &[(abi::LOCAL[1], 8)]);
    (
        CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        asm.ins,
        asm.rel,
    )
}

/// `term::terminalSize` app body: return a `TermSize { columns, rows }` record
/// from the TermView grid, or `ERR_UNSUPPORTED` when inactive / no surface.
fn emit_app_terminal_size(symbol: &str, term_state_offset: usize) -> AppHookBody {
    let mut asm = Asm::new(symbol);
    // Frame: lr@0, x20(state)@8, columns@16, rows@24.
    let frame = 48;
    let unsupported = format!("{symbol}_unsupported");
    let done = format!("{symbol}_done");
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(frame));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        0,
    ));
    asm.push(abi::store_u64(abi::LOCAL[1], abi::stack_pointer(), 8));
    // Requires active TUI mode (plan §4.7).
    asm.push(abi::load_u64(
        abi::SCRATCH[0],
        TERM_ARENA_STATE_REG,
        term_state_offset + code::TERM_STATE_ACTIVE_OFFSET,
    ));
    asm.push(abi::compare_immediate(abi::SCRATCH[0], "0"));
    asm.push(abi::branch_eq(&unsupported));
    emit_get_tv_state(&mut asm, abi::LOCAL[1], &unsupported);
    asm.push(abi::load_u64(
        abi::SCRATCH[1],
        abi::LOCAL[1],
        TV_COLS_OFFSET,
    ));
    asm.push(abi::load_u64(
        abi::SCRATCH[2],
        abi::LOCAL[1],
        TV_ROWS_OFFSET,
    ));
    asm.push(abi::store_u64(abi::SCRATCH[1], abi::stack_pointer(), 16));
    asm.push(abi::store_u64(abi::SCRATCH[2], abi::stack_pointer(), 24));
    // record = arena_alloc(16, 8); columns@0, rows@8.
    asm.push(abi::move_immediate("x0", "Integer", "16"));
    asm.push(abi::move_immediate("x1", "Integer", "8"));
    asm.call_internal(ARENA_ALLOC_SYMBOL);
    asm.push(abi::compare_immediate("x0", "0")); // RESULT_OK_TAG
    asm.push(abi::branch_ne(&unsupported));
    asm.push(abi::load_u64(abi::SCRATCH[1], abi::stack_pointer(), 16));
    asm.push(abi::load_u64(abi::SCRATCH[2], abi::stack_pointer(), 24));
    asm.push(abi::store_u64(abi::SCRATCH[1], "x1", 0)); // columns
    asm.push(abi::store_u64(abi::SCRATCH[2], "x1", 8)); // rows
    asm.push(abi::move_immediate("x0", "Integer", "0")); // OK; x1 = record
    asm.push(abi::branch(&done));
    // plan-88-C: code + message symbol from `ERRORCODE_CONSTANTS`. This app helper
    // returns via x0/x1/x2 directly (not the shared `RESULT_*` registers), so the
    // shared `raise_error_into` emitter does not fit; the values are table-sourced
    // and loaded here, byte-identical to the former local `ERR_UNSUPPORTED_*` consts.
    let (unsupported_code, unsupported_symbol) =
        crate::codegen::registry::runtime_error_emission("ErrUnsupported")
            .expect("ErrUnsupported is an errorCode constant");
    asm.push(abi::label(&unsupported));
    asm.push(abi::move_immediate("x1", "Integer", unsupported_code));
    asm.push(abi::move_immediate("x0", "Integer", "1")); // ERR tag
    asm.push(
        CodeInstruction::new("adrp")
            .field("dst", "x2")
            .field("symbol", unsupported_symbol),
    );
    asm.push(
        CodeInstruction::new("add_pageoff")
            .field("dst", "x2")
            .field("src", "x2")
            .field("symbol", unsupported_symbol),
    );
    for kind in [RelocIntent::DataAddrHi, RelocIntent::DataAddrLo] {
        asm.rel.push(CodeRelocation {
            from: symbol.to_string(),
            to: unsupported_symbol.to_string(),
            kind,
            binding: "data".to_string(),
            library: None,
        });
    }
    asm.push(abi::label(&done));
    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::load_u64(abi::LOCAL[1], abi::stack_pointer(), 8));
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
