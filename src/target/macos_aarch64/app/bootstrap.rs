//! macOS app-mode bootstrap: `_main` AppKit setup, worker shim, and the
//! launch/terminate/append/finish delegate helpers (plan-11 split, pure relocation).

use super::*;

pub(super) fn emit_main_bootstrap() -> CodeFunction {
    let mut asm = Asm::new(MAIN_SYMBOL);
    asm.push(abi::label("entry"));
    // Reserve the frame and stash argc/argv (passed in x0/x1 by the kernel) before
    // any external call clobbers them; the worker reads them from this block.
    asm.push(abi::subtract_stack(FRAME_SIZE));
    asm.push(abi::store_u64("x0", abi::stack_pointer(), OFF_ARGC));
    asm.push(abi::store_u64("x1", abi::stack_pointer(), OFF_ARGV));

    // app = [NSApplication sharedApplication]
    asm.external_data(REG_APP, CLASS_NS_APPLICATION, LIB_APPKIT);
    asm.load_selector(SEL_SHARED_APPLICATION.0);
    asm.push(abi::move_register("x0", REG_APP));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register(REG_APP, "x0"));

    // [app setActivationPolicy:NSApplicationActivationPolicyRegular]
    asm.load_selector(SEL_SET_ACTIVATION_POLICY.0);
    asm.push(abi::move_immediate(
        "x2",
        "Integer",
        ACTIVATION_POLICY_REGULAR,
    ));
    asm.push(abi::move_register("x0", REG_APP));
    asm.call_external("_objc_msgSend", LIB_OBJC);

    // window = [[NSWindow alloc] initWithContentRect:styleMask:backing:defer:]
    asm.external_data(REG_WINDOW, CLASS_NS_WINDOW, LIB_APPKIT);
    asm.load_selector(SEL_ALLOC.0);
    asm.push(abi::move_register("x0", REG_WINDOW));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register(REG_WINDOW, "x0"));

    asm.load_selector(SEL_INIT_WINDOW.0);
    // contentRect = NSMakeRect(100, 100, 900, 640) -> d0..d3 (HFA of 4 doubles).
    emit_double_immediate(&mut asm, "d0", 100);
    emit_double_immediate(&mut asm, "d1", 100);
    emit_double_immediate(&mut asm, "d2", 900);
    emit_double_immediate(&mut asm, "d3", 640);
    asm.push(abi::move_immediate("x2", "Integer", WINDOW_STYLE_MASK));
    asm.push(abi::move_immediate("x3", "Integer", BACKING_BUFFERED));
    asm.push(abi::move_immediate("x4", "Integer", "0")); // defer: NO
    asm.push(abi::move_register("x0", REG_WINDOW));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register(REG_WINDOW, "x0"));

    // title = [NSString stringWithUTF8String:"MFBASIC App"]; [window setTitle:title]
    asm.external_data(REG_SCRATCH_OBJ, CLASS_NS_STRING, LIB_FOUNDATION);
    asm.load_selector(SEL_STRING_WITH_UTF8.0);
    asm.local_address("x2", STR_TITLE.0);
    asm.push(abi::move_register("x0", REG_SCRATCH_OBJ));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register(REG_SCRATCH_OBJ, "x0"));
    asm.load_selector(SEL_SET_TITLE.0);
    asm.push(abi::move_register("x2", REG_SCRATCH_OBJ));
    asm.push(abi::move_register("x0", REG_WINDOW));
    asm.call_external("_objc_msgSend", LIB_OBJC);

    // headless = getenv("MFB_MACAPP_HEADLESS")
    asm.local_address("x0", STR_HEADLESS_ENV.0);
    asm.call_external("_getenv", LIB_SYSTEM);
    asm.push(abi::move_register(REG_HEADLESS, "x0"));

    // In GUI mode, build the transcript view + show/activate the window. Headless
    // test mode skips all of this, leaving no associated NSTextView so the io
    // helpers fall back to the file descriptor sink (plan §7.2 Strategy A).
    asm.push(abi::compare_immediate(REG_HEADLESS, "0"));
    asm.push(abi::branch_ne("after_show"));

    // content = [window contentView]
    asm.load_selector(SEL_CONTENT_VIEW.0);
    asm.push(abi::move_register("x0", REG_WINDOW));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register("x24", "x0")); // content view (callee-saved)

    // scroll = [[NSScrollView alloc] initWithFrame:NSMakeRect(0,0,900,640)]
    asm.external_data("x23", CLASS_NS_SCROLL_VIEW, LIB_APPKIT);
    asm.load_selector(SEL_ALLOC.0);
    asm.push(abi::move_register("x0", "x23"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register("x23", "x0"));
    asm.load_selector(SEL_INIT_FRAME.0);
    emit_double_immediate(&mut asm, "d0", 0);
    emit_double_immediate(&mut asm, "d1", 0);
    emit_double_immediate(&mut asm, "d2", 900);
    emit_double_immediate(&mut asm, "d3", 640);
    asm.push(abi::move_register("x0", "x23"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register("x23", "x0")); // scroll view
                                               // [scroll setAutoresizingMask:NSViewWidthSizable|NSViewHeightSizable] -- track
                                               // the window content view so the transcript fills the window on resize.
    asm.load_selector(SEL_SET_AUTORESIZING_MASK.0);
    asm.push(abi::move_immediate(
        "x2",
        "Integer",
        AUTORESIZE_WIDTH_HEIGHT,
    ));
    asm.push(abi::move_register("x0", "x23"));
    asm.call_external("_objc_msgSend", LIB_OBJC);

    // Synthesize MFBTextView : NSTextView overriding keyDown: so the transcript
    // view itself receives typed keys (terminal-style input echoed into the
    // view), instead of a separate input field.
    // cls = objc_allocateClassPair(NSTextView, "MFBTextView", 0)
    asm.external_data("x25", CLASS_NS_TEXT_VIEW, LIB_APPKIT);
    asm.local_address("x1", STR_TEXTVIEW_CLASS.0);
    asm.push(abi::move_immediate("x2", "Integer", "0"));
    asm.push(abi::move_register("x0", "x25"));
    asm.call_external("_objc_allocateClassPair", LIB_OBJC);
    asm.push(abi::move_register("x25", "x0")); // new class
                                               // class_addMethod(cls, @selector(keyDown:), imp, "v@:@")
    asm.load_selector(SEL_KEY_DOWN.0);
    asm.local_address("x2", KEY_DOWN_SYMBOL);
    asm.local_address("x3", STR_INPUT_TYPES.0);
    asm.push(abi::move_register("x0", "x25"));
    asm.call_external("_class_addMethod", LIB_OBJC);
    // objc_registerClassPair(cls)
    asm.push(abi::move_register("x0", "x25"));
    asm.call_external("_objc_registerClassPair", LIB_OBJC);
    // tv = [[MFBTextView alloc] initWithFrame:NSMakeRect(0,0,900,640)]
    asm.load_selector(SEL_ALLOC.0);
    asm.push(abi::move_register("x0", "x25"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register(REG_SCRATCH_OBJ, "x0"));
    asm.load_selector(SEL_INIT_FRAME.0);
    emit_double_immediate(&mut asm, "d0", 0);
    emit_double_immediate(&mut asm, "d1", 0);
    emit_double_immediate(&mut asm, "d2", 900);
    emit_double_immediate(&mut asm, "d3", 640);
    asm.push(abi::move_register("x0", REG_SCRATCH_OBJ));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register(REG_SCRATCH_OBJ, "x0")); // transcript text view (x21)

    // [tv setAutoresizingMask:NSViewWidthSizable] -- widen with the scroll view.
    asm.load_selector(SEL_SET_AUTORESIZING_MASK.0);
    asm.push(abi::move_immediate("x2", "Integer", AUTORESIZE_WIDTH));
    asm.push(abi::move_register("x0", REG_SCRATCH_OBJ));
    asm.call_external("_objc_msgSend", LIB_OBJC);

    // [tv setFont:[NSFont userFixedPitchFontOfSize:13]] -- monospaced (plan §5.5)
    asm.external_data("x25", CLASS_NS_FONT, LIB_APPKIT);
    asm.load_selector(SEL_USER_FIXED_FONT.0);
    emit_double_immediate(&mut asm, "d0", TRANSCRIPT_FONT_SIZE);
    asm.push(abi::move_register("x0", "x25"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register("x25", "x0")); // fixed-pitch font
    asm.load_selector(SEL_SET_FONT.0);
    asm.push(abi::move_register("x2", "x25"));
    asm.push(abi::move_register("x0", REG_SCRATCH_OBJ));
    asm.call_external("_objc_msgSend", LIB_OBJC);

    // [tv setEditable:NO]; [tv setSelectable:YES]
    asm.load_selector(SEL_SET_EDITABLE.0);
    asm.push(abi::move_immediate("x2", "Integer", "0"));
    asm.push(abi::move_register("x0", REG_SCRATCH_OBJ));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.load_selector(SEL_SET_SELECTABLE.0);
    asm.push(abi::move_immediate("x2", "Integer", "1"));
    asm.push(abi::move_register("x0", REG_SCRATCH_OBJ));
    asm.call_external("_objc_msgSend", LIB_OBJC);

    // [scroll setDocumentView:tv]; [scroll setHasVerticalScroller:YES]
    asm.load_selector(SEL_SET_DOCUMENT_VIEW.0);
    asm.push(abi::move_register("x2", REG_SCRATCH_OBJ));
    asm.push(abi::move_register("x0", "x23"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.load_selector(SEL_SET_HAS_VSCROLLER.0);
    asm.push(abi::move_immediate("x2", "Integer", "1"));
    asm.push(abi::move_register("x0", "x23"));
    asm.call_external("_objc_msgSend", LIB_OBJC);

    // [content addSubview:scroll]
    asm.load_selector(SEL_ADD_SUBVIEW.0);
    asm.push(abi::move_register("x2", "x23"));
    asm.push(abi::move_register("x0", "x24"));
    asm.call_external("_objc_msgSend", LIB_OBJC);

    // Stash the text view on NSApp so the io helpers (worker thread) can reach it:
    // objc_setAssociatedObject(app, &ASSOC_KEY, tv, OBJC_ASSOCIATION_ASSIGN)
    asm.push(abi::move_register("x0", REG_APP));
    asm.local_address("x1", ASSOC_KEY);
    asm.push(abi::move_register("x2", REG_SCRATCH_OBJ));
    asm.push(abi::move_immediate("x3", "Integer", "0"));
    asm.call_external("_objc_setAssociatedObject", LIB_OBJC);

    // --- term:: TermView surface (plan-01-term.md §6.3, Phase 4) ------------
    // Stash the window + transcript scroll view on NSApp so the worker-thread
    // term:: helpers can swap the content view in/out and restore it.
    asm.push(abi::move_register("x0", REG_APP));
    asm.local_address("x1", WINDOW_ASSOC_KEY);
    asm.push(abi::move_register("x2", REG_WINDOW));
    asm.push(abi::move_immediate("x3", "Integer", "0")); // OBJC_ASSOCIATION_ASSIGN
    asm.call_external("_objc_setAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register("x0", REG_APP));
    asm.local_address("x1", SCROLLVIEW_ASSOC_KEY);
    asm.push(abi::move_register("x2", "x23")); // scroll view
    asm.push(abi::move_immediate("x3", "Integer", "0"));
    asm.call_external("_objc_setAssociatedObject", LIB_OBJC);

    // Synthesize TermView : NSView. The grid-state struct is calloc'd and
    // attached as an associated object (see term_init), so the class needs no
    // extra instance bytes.
    // cls = objc_allocateClassPair(NSView, "TermView", 0)
    asm.external_data("x25", CLASS_NS_VIEW, LIB_APPKIT);
    asm.local_address("x1", STR_TERMVIEW_CLASS_NAME.0);
    asm.push(abi::move_immediate("x2", "Integer", "0"));
    asm.push(abi::move_register("x0", "x25"));
    asm.call_external("_objc_allocateClassPair", LIB_OBJC);
    asm.push(abi::move_register("x25", "x0")); // new class
                                               // class_addMethod(cls, @selector(drawRect:), imp, "v@:{CGRect=dddd}")
    asm.load_selector(SEL_DRAW_RECT.0);
    asm.local_address("x2", TERM_VIEW_DRAW_RECT_SYMBOL);
    asm.local_address("x3", STR_DRAW_RECT_TYPES.0);
    asm.push(abi::move_register("x0", "x25"));
    asm.call_external("_class_addMethod", LIB_OBJC);
    // class_addMethod(cls, @selector(isFlipped), imp, "c@:")
    asm.load_selector(SEL_IS_FLIPPED.0);
    asm.local_address("x2", TERM_VIEW_IS_FLIPPED_SYMBOL);
    asm.local_address("x3", STR_IS_FLIPPED_TYPES.0);
    asm.push(abi::move_register("x0", "x25"));
    asm.call_external("_class_addMethod", LIB_OBJC);
    // class_addMethod(cls, @selector(mfbWriteString:), imp, "v@:@")
    asm.load_selector(SEL_MFB_WRITE_STRING.0);
    asm.local_address("x2", MFB_WRITE_STRING_SYMBOL);
    asm.local_address("x3", STR_WRITE_STRING_TYPES.0);
    asm.push(abi::move_register("x0", "x25"));
    asm.call_external("_class_addMethod", LIB_OBJC);
    // class_addMethod(cls, @selector(mfbClear:), imp, "v@:@") — main-thread grid
    // clear (bug-165). The IMP is the existing TERM_CLEAR_SYMBOL helper, which
    // reads only `self` (x0 = the TermView) and ignores `_cmd`/the object arg.
    asm.load_selector(SEL_MFB_CLEAR.0);
    asm.local_address("x2", TERM_CLEAR_SYMBOL);
    asm.local_address("x3", STR_WRITE_STRING_TYPES.0);
    asm.push(abi::move_register("x0", "x25"));
    asm.call_external("_class_addMethod", LIB_OBJC);
    // class_addMethod(cls, @selector(acceptsFirstResponder), imp, "c@:") — so the
    // TermView can become first responder and receive keyDown: in TUI mode.
    asm.load_selector(SEL_ACCEPTS_FIRST_RESPONDER.0);
    asm.local_address("x2", TERM_ACCEPTS_FR_SYMBOL);
    asm.local_address("x3", STR_IS_FLIPPED_TYPES.0); // "c@:"
    asm.push(abi::move_register("x0", "x25"));
    asm.call_external("_class_addMethod", LIB_OBJC);
    // class_addMethod(cls, @selector(keyDown:), imp, "v@:@")
    asm.load_selector(SEL_KEY_DOWN.0);
    asm.local_address("x2", TERM_KEY_DOWN_SYMBOL);
    asm.local_address("x3", STR_INPUT_TYPES.0); // "v@:@"
    asm.push(abi::move_register("x0", "x25"));
    asm.call_external("_class_addMethod", LIB_OBJC);
    // class_addMethod(cls, @selector(setFrameSize:), imp, "v@:{CGSize=dd}") — the
    // live-window-resize hook: recompute rows/cols and realloc the grid (plan-35-D).
    asm.load_selector(SEL_SET_FRAME_SIZE.0);
    asm.local_address("x2", TERM_SET_FRAME_SIZE_SYMBOL);
    asm.local_address("x3", STR_SET_FRAME_SIZE_TYPES.0);
    asm.push(abi::move_register("x0", "x25"));
    asm.call_external("_class_addMethod", LIB_OBJC);
    // objc_registerClassPair(cls)
    asm.push(abi::move_register("x0", "x25"));
    asm.call_external("_objc_registerClassPair", LIB_OBJC);
    // tv = [[TermView alloc] initWithFrame:NSMakeRect(0,0,W,H)]
    asm.load_selector(SEL_ALLOC.0);
    asm.push(abi::move_register("x0", "x25"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register("x26", "x0"));
    asm.load_selector(SEL_INIT_FRAME.0);
    emit_double_immediate(&mut asm, "d0", 0);
    emit_double_immediate(&mut asm, "d1", 0);
    emit_double_immediate(&mut asm, "d2", TERM_VIEW_WIDTH);
    emit_double_immediate(&mut asm, "d3", TERM_VIEW_HEIGHT);
    asm.push(abi::move_register("x0", "x26"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register("x26", "x0")); // TermView instance
                                               // [tv setAutoresizingMask:NSViewWidthSizable|NSViewHeightSizable]
    asm.load_selector(SEL_SET_AUTORESIZING_MASK.0);
    asm.push(abi::move_immediate(
        "x2",
        "Integer",
        AUTORESIZE_WIDTH_HEIGHT,
    ));
    asm.push(abi::move_register("x0", "x26"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    // Size + allocate the cell grid from the font metrics.
    asm.push(abi::move_register("x0", "x26"));
    asm.call_internal(TERM_INIT_SYMBOL);
    // Stash the TermView on NSApp (its alloc +1 keeps it alive; ASSIGN).
    asm.push(abi::move_register("x0", REG_APP));
    asm.local_address("x1", TERMVIEW_ASSOC_KEY);
    asm.push(abi::move_register("x2", "x26"));
    asm.push(abi::move_immediate("x3", "Integer", "0"));
    asm.call_external("_objc_setAssociatedObject", LIB_OBJC);
    // --- end term:: TermView surface ---------------------------------------

    // Synthesize an NSApplication delegate so closing the window quits the app
    // (plan §5.7): a runtime MFBAppDelegate : NSObject whose
    // applicationShouldTerminateAfterLastWindowClosed: returns YES.
    // cls = objc_allocateClassPair(NSObject, "MFBAppDelegate", 0)
    asm.external_data("x23", CLASS_NS_OBJECT, LIB_OBJC);
    asm.local_address("x1", STR_DELEGATE_CLASS.0);
    asm.push(abi::move_immediate("x2", "Integer", "0"));
    asm.push(abi::move_register("x0", "x23"));
    asm.call_external("_objc_allocateClassPair", LIB_OBJC);
    asm.push(abi::move_register("x23", "x0")); // new class
                                               // class_addMethod(cls, @selector(applicationShouldTerminate...), imp, "c@:@")
    asm.load_selector(SEL_APP_SHOULD_TERMINATE.0);
    asm.local_address("x2", SHOULD_TERMINATE_SYMBOL);
    asm.local_address("x3", STR_DELEGATE_TYPES.0);
    asm.push(abi::move_register("x0", "x23"));
    asm.call_external("_class_addMethod", LIB_OBJC);
    // class_addMethod(cls, @selector(applicationDidFinishLaunching:), imp, "v@:@")
    // — spawns the worker once launch is complete (plan-01-term.md §6.4).
    asm.load_selector(SEL_APP_DID_FINISH_LAUNCHING.0);
    asm.local_address("x2", DID_FINISH_LAUNCHING_SYMBOL);
    asm.local_address("x3", STR_INPUT_TYPES.0); // "v@:@"
    asm.push(abi::move_register("x0", "x23"));
    asm.call_external("_class_addMethod", LIB_OBJC);
    // objc_registerClassPair(cls)
    asm.push(abi::move_register("x0", "x23"));
    asm.call_external("_objc_registerClassPair", LIB_OBJC);
    // delegate = [[cls alloc] init]; [app setDelegate:delegate]
    asm.load_selector(SEL_ALLOC.0);
    asm.push(abi::move_register("x0", "x23"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register("x23", "x0"));
    asm.load_selector(SEL_INIT.0);
    asm.push(abi::move_register("x0", "x23"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register("x23", "x0")); // delegate instance
    asm.load_selector(SEL_SET_DELEGATE.0);
    asm.push(abi::move_register("x2", "x23"));
    asm.push(abi::move_register("x0", REG_APP));
    asm.call_external("_objc_msgSend", LIB_OBJC);

    // Input-line buffer: an NSMutableString accumulating typed characters until
    // Return; stashed (retained) on NSApp so the keyDown: handler can reach it.
    asm.external_data("x23", CLASS_NS_MUTABLE_STRING, LIB_FOUNDATION);
    asm.load_selector(SEL_STRING.0);
    asm.push(abi::move_register("x0", "x23"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register("x23", "x0")); // input line buffer
    asm.push(abi::move_register("x0", REG_APP));
    asm.local_address("x1", INPUT_LINE_KEY);
    asm.push(abi::move_register("x2", "x23"));
    asm.push(abi::move_immediate("x3", "Integer", "1")); // OBJC_ASSOCIATION_RETAIN_NONATOMIC
    asm.call_external("_objc_setAssociatedObject", LIB_OBJC);

    // Wire the input pipe: pipe(fds); dup2(fds[0], 0) so the console read helpers
    // consume window input; stash fds[1] (write end) on NSApp for the keyDown:
    // handler.
    asm.push(abi::add_immediate("x0", abi::stack_pointer(), OFF_PIPE));
    asm.call_external("_pipe", LIB_SYSTEM);
    // Make the pipe write end (fds[1]) non-blocking so the keyDown: commit
    // write() returns -1/EAGAIN instead of blocking the UI thread forever when
    // the worker stops draining stdin and the 64 KiB pipe fills (bug-114). The
    // third `fcntl` argument is variadic, so on Apple AArch64 it is passed on
    // the stack (mirrors emit_variadic_call).
    asm.push(abi::load_u32("x0", abi::stack_pointer(), OFF_PIPE + 4)); // fds[1] (write)
    asm.push(abi::move_immediate("x1", "Integer", "4")); // F_SETFL
    asm.push(abi::move_immediate("x2", "Integer", "4")); // O_NONBLOCK (0x0004 on Darwin)
    asm.push(abi::subtract_stack(16));
    asm.push(abi::store_u64("x2", abi::stack_pointer(), 0));
    asm.call_external("_fcntl", LIB_SYSTEM);
    asm.push(abi::add_stack(16));
    asm.push(abi::load_u32("x0", abi::stack_pointer(), OFF_PIPE)); // fds[0] (read)
    asm.push(abi::move_immediate("x1", "Integer", "0")); // newfd: stdin
    asm.call_external("_dup2", LIB_SYSTEM);
    // fd 0 now names the read end, so the original fds[0] is a redundant
    // duplicate that would otherwise stay open for the process lifetime
    // (bug-241). Two cases must NOT close it: a failed dup2 (fds[0] is then the
    // only read end), and fds[0] already being fd 0 (only reachable if stdin was
    // closed before `pipe`, which makes dup2 a no-op — closing would leave the
    // program with no stdin at all).
    asm.push(abi::compare_immediate("x0", "0"));
    asm.push(abi::branch_lt("input_pipe_wired"));
    asm.push(abi::load_u32("x0", abi::stack_pointer(), OFF_PIPE)); // fds[0] (read)
    asm.push(abi::compare_immediate("x0", "0"));
    asm.push(abi::branch_eq("input_pipe_wired"));
    asm.call_external("_close", LIB_SYSTEM);
    asm.push(abi::label("input_pipe_wired"));
    asm.push(abi::load_u32("x2", abi::stack_pointer(), OFF_PIPE + 4)); // fds[1] (write)
    asm.push(abi::move_register("x0", REG_APP));
    asm.local_address("x1", PIPE_ASSOC_KEY);
    asm.push(abi::move_immediate("x3", "Integer", "0")); // OBJC_ASSOCIATION_ASSIGN
    asm.call_external("_objc_setAssociatedObject", LIB_OBJC);

    // Application menu with the standard Quit item (Cmd-Q -> [NSApp terminate:]):
    //   mainMenu -> appMenuItem -> appMenu -> "Quit" item.
    // mainMenu = [[NSMenu alloc] init]
    asm.external_data("x23", CLASS_NS_MENU, LIB_APPKIT);
    asm.load_selector(SEL_ALLOC.0);
    asm.push(abi::move_register("x0", "x23"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register("x23", "x0"));
    asm.load_selector(SEL_INIT.0);
    asm.push(abi::move_register("x0", "x23"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register("x23", "x0")); // main menu
                                               // appMenuItem = [[NSMenuItem alloc] init]
    asm.external_data("x24", CLASS_NS_MENU_ITEM, LIB_APPKIT);
    asm.load_selector(SEL_ALLOC.0);
    asm.push(abi::move_register("x0", "x24"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register("x24", "x0"));
    asm.load_selector(SEL_INIT.0);
    asm.push(abi::move_register("x0", "x24"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register("x24", "x0")); // app menu item
                                               // [mainMenu addItem:appMenuItem]
    asm.load_selector(SEL_ADD_ITEM.0);
    asm.push(abi::move_register("x2", "x24"));
    asm.push(abi::move_register("x0", "x23"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    // appMenu = [[NSMenu alloc] init]
    asm.external_data("x25", CLASS_NS_MENU, LIB_APPKIT);
    asm.load_selector(SEL_ALLOC.0);
    asm.push(abi::move_register("x0", "x25"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register("x25", "x0"));
    asm.load_selector(SEL_INIT.0);
    asm.push(abi::move_register("x0", "x25"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register("x25", "x0")); // app submenu
                                               // quitItem = [[NSMenuItem alloc] init]
    asm.external_data("x26", CLASS_NS_MENU_ITEM, LIB_APPKIT);
    asm.load_selector(SEL_ALLOC.0);
    asm.push(abi::move_register("x0", "x26"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register("x26", "x0"));
    asm.load_selector(SEL_INIT.0);
    asm.push(abi::move_register("x0", "x26"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register("x26", "x0")); // quit item
                                               // [quitItem setTitle:@"Quit"]
    build_nsstring_from_cstring(&mut asm, "x27", STR_QUIT.0);
    asm.push(abi::move_register("x27", "x0"));
    asm.load_selector(SEL_SET_TITLE.0);
    asm.push(abi::move_register("x2", "x27"));
    asm.push(abi::move_register("x0", "x26"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    // [quitItem setAction:@selector(terminate:)]
    asm.load_selector(SEL_TERMINATE.0);
    asm.push(abi::move_register("x27", "x1")); // terminate: SEL
    asm.load_selector(SEL_SET_ACTION.0);
    asm.push(abi::move_register("x2", "x27"));
    asm.push(abi::move_register("x0", "x26"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    // [quitItem setKeyEquivalent:@"q"]
    build_nsstring_from_cstring(&mut asm, "x27", STR_QUIT_KEY.0);
    asm.push(abi::move_register("x27", "x0"));
    asm.load_selector(SEL_SET_KEY_EQUIVALENT.0);
    asm.push(abi::move_register("x2", "x27"));
    asm.push(abi::move_register("x0", "x26"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    // [appMenu addItem:quitItem]
    asm.load_selector(SEL_ADD_ITEM.0);
    asm.push(abi::move_register("x2", "x26"));
    asm.push(abi::move_register("x0", "x25"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    // [appMenuItem setSubmenu:appMenu]
    asm.load_selector(SEL_SET_SUBMENU.0);
    asm.push(abi::move_register("x2", "x25"));
    asm.push(abi::move_register("x0", "x24"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    // [app setMainMenu:mainMenu]
    asm.load_selector(SEL_SET_MAIN_MENU.0);
    asm.push(abi::move_register("x2", "x23"));
    asm.push(abi::move_register("x0", REG_APP));
    asm.call_external("_objc_msgSend", LIB_OBJC);

    asm.load_selector(SEL_MAKE_KEY_AND_ORDER_FRONT.0);
    asm.push(abi::move_immediate("x2", "Integer", "0"));
    asm.push(abi::move_register("x0", REG_WINDOW));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.load_selector(SEL_ACTIVATE.0);
    asm.push(abi::move_immediate("x2", "Integer", "1")); // ignoreOtherApps: YES
    asm.push(abi::move_register("x0", REG_APP));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    // [window makeFirstResponder:textview] -- keypresses go to the transcript.
    asm.load_selector(SEL_MAKE_FIRST_RESPONDER.0);
    asm.push(abi::move_register("x2", REG_SCRATCH_OBJ)); // transcript text view
    asm.push(abi::move_register("x0", REG_WINDOW));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::label("after_show"));

    // The GUI worker must NOT run during -[NSApplication finishLaunching]:
    // touching AppKit / the Obj-C runtime from the worker while the main thread
    // is still launching (e.g. lazily loading category-bearing frameworks)
    // corrupts the runtime and aborts (plan-01-term.md §6.4). So in GUI mode the
    // worker is spawned from applicationDidFinishLaunching: instead of here;
    // stash the {argc, argv} block pointer for that handler. Headless has no run
    // loop / delegate callback, so it spawns the worker inline.
    asm.push(abi::compare_immediate(REG_HEADLESS, "0"));
    asm.push(abi::branch_eq("gui_defer_worker"));

    // Headless: pthread_create(&tid, NULL, worker, &argblock); then spin while the
    // worker runs the program and exits the process.
    asm.push(abi::add_immediate("x0", abi::stack_pointer(), OFF_TID));
    asm.push(abi::move_immediate("x1", "Integer", "0"));
    asm.local_address("x2", WORKER_SYMBOL);
    asm.push(abi::add_immediate("x3", abi::stack_pointer(), OFF_ARGC));
    asm.call_external("_pthread_create", LIB_SYSTEM);
    // Park the main thread (block in pause() forever) instead of busy-spinning at
    // 100% CPU; the worker runs the program and exits the process.
    asm.push(abi::label("spin"));
    asm.call_external("_pause", LIB_SYSTEM);
    asm.push(abi::branch("spin"));

    // GUI: stash &argblock so applicationDidFinishLaunching: can spawn the worker
    // once launch is complete. _main blocks forever in [NSApp run], so this stack
    // pointer stays valid for the lifetime of the process.
    asm.push(abi::label("gui_defer_worker"));
    asm.push(abi::move_register("x0", REG_APP));
    asm.local_address("x1", ARG_ASSOC_KEY);
    asm.push(abi::add_immediate("x2", abi::stack_pointer(), OFF_ARGC));
    asm.push(abi::move_immediate("x3", "Integer", "0")); // OBJC_ASSOCIATION_ASSIGN
    asm.call_external("_objc_setAssociatedObject", LIB_OBJC);

    // GUI: run the AppKit event loop on the main thread. [NSApp run] does not
    // return under normal operation; if it ever does, exit cleanly.
    asm.push(abi::label("run_event_loop"));
    asm.load_selector(SEL_RUN.0);
    asm.push(abi::move_register("x0", REG_APP));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_immediate("x0", "Integer", "0"));
    asm.call_external("_exit", LIB_SYSTEM);
    asm.push(abi::branch_self());
    asm.push(abi::return_());

    CodeFunction {
        name: "macapp.bootstrap".to_string(),
        symbol: MAIN_SYMBOL.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame: CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        stack_slots: Vec::new(),
        instructions: asm.ins,
        relocations: asm.rel,
    }
}

/// `void *_mfb_macapp_worker(void *arg)` pthread start routine: establishes an
/// autorelease pool for this Cocoa-calling thread, unpacks the `{argc, argv}`
/// block (when the entry accepts args) into `x0`/`x1`, then tail calls the
/// standard program entry, which never returns (it ends in `_exit` or, in GUI
/// mode, `pthread_exit`).
///
/// The autorelease pool is mandatory: the worker creates autoreleased Cocoa
/// objects (NSString/NSFont/...), and on the GUI keep-open path it `pthread_exit`s
/// — without a real pool in place, the thread-exit autorelease-pool cleanup
/// drains improperly-pooled objects and crashes (SIGSEGV in objc_msgSend release).
pub(super) fn emit_worker_shim(spec: &AppEntrySpec) -> CodeFunction {
    let mut asm = Asm::new(WORKER_SYMBOL);
    asm.push(abi::label("entry"));
    // objc_autoreleasePoolPush(), preserving the pthread arg in x0 across it.
    asm.push(abi::subtract_stack(16));
    asm.push(abi::store_u64("x0", abi::stack_pointer(), 0));
    asm.call_external("_objc_autoreleasePoolPush", LIB_OBJC);
    asm.push(abi::load_u64("x0", abi::stack_pointer(), 0));
    asm.push(abi::add_stack(16));
    if spec.language_entry_accepts_args {
        // arg (x0) points at { i64 argc; char **argv }.
        asm.push(abi::load_u64("x1", "x0", OFF_ARGV));
        asm.push(abi::load_u64("x0", "x0", OFF_ARGC));
    }
    asm.call_internal(code::MACAPP_PROGRAM_SYMBOL);
    asm.push(abi::branch_self());
    asm.push(abi::return_());

    CodeFunction {
        name: "macapp.worker".to_string(),
        symbol: WORKER_SYMBOL.to_string(),
        params: Vec::new(),
        returns: "Pointer".to_string(),
        frame: CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        stack_slots: Vec::new(),
        instructions: asm.ins,
        relocations: asm.rel,
    }
}

/// `void _mfb_macapp_append(id textView /*x0*/, id nsString /*x1*/)`: append
/// `nsString` to the text view's transcript, styled with the monospaced font, on
/// the main thread.
///
/// Builds `[[NSAttributedString alloc] initWithString:nsString
/// attributes:@{NSFontAttributeName: [NSFont userFixedPitchFontOfSize:N]}]` and
/// appends it to the text storage via `performSelectorOnMainThread:` (AppKit
/// stays single-threaded; waitUntilDone makes the write synchronous so
/// `io::flush` is a no-op, plan §5.4). Appending an explicitly-attributed run is
/// required: plain `mutableString.appendString:` ignores the view's font and
/// renders in the default proportional system font (plan §5.5).
pub(super) fn emit_append_helper() -> CodeFunction {
    let mut asm = Asm::new(APPEND_SYMBOL);
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(48));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        0,
    ));
    asm.push(abi::store_u64("x19", abi::stack_pointer(), 8));
    asm.push(abi::store_u64("x20", abi::stack_pointer(), 16));
    asm.push(abi::store_u64("x21", abi::stack_pointer(), 24));
    asm.push(abi::store_u64("x22", abi::stack_pointer(), 32));
    asm.push(abi::move_register("x19", "x0")); // text view
    asm.push(abi::move_register("x20", "x1")); // nsstring

    // Per-append autorelease pool. The worker's process-lifetime pool
    // (emit_worker_shim) is never drained (the worker parks forever), so the
    // autoreleased font and attributes NSDictionary created below would
    // accumulate for the process lifetime — RSS grows without bound with output
    // volume (bug-112). Push a fresh pool here and pop it before returning so
    // this call's autoreleases are reclaimed immediately. Token saved at sp+40
    // (the frame's spare slot). The owned attributed string is still released
    // explicitly (bug-53); waitUntilDone:YES means the main thread has finished
    // consuming it before the pop.
    asm.call_external("_objc_autoreleasePoolPush", LIB_OBJC);
    asm.push(abi::store_u64("x0", abi::stack_pointer(), 40)); // pool token

    // font = [NSFont userFixedPitchFontOfSize:N]
    asm.external_data("x21", CLASS_NS_FONT, LIB_APPKIT);
    asm.load_selector(SEL_USER_FIXED_FONT.0);
    emit_double_immediate(&mut asm, "d0", TRANSCRIPT_FONT_SIZE);
    asm.push(abi::move_register("x0", "x21"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register("x21", "x0")); // fixed-pitch font

    // attrs = [NSDictionary dictionaryWithObject:font forKey:NSFontAttributeName]
    asm.external_data("x22", CLASS_NS_DICTIONARY, LIB_FOUNDATION);
    asm.load_selector(SEL_DICTIONARY_WITH_OBJECT.0);
    asm.push(abi::move_register("x2", "x21")); // object: font
                                               // NSFontAttributeName is a `NSString * const` global: external_data yields the
                                               // address of that variable, so dereference once more to get the NSString key.
    asm.external_data("x3", NS_FONT_ATTRIBUTE_NAME, LIB_APPKIT);
    asm.push(abi::load_u64("x3", "x3", 0));
    asm.push(abi::move_register("x0", "x22"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register("x22", "x0")); // attributes dictionary

    // attr = [[NSAttributedString alloc] initWithString:nsstring attributes:attrs]
    asm.external_data("x21", CLASS_NS_ATTRIBUTED_STRING, LIB_FOUNDATION);
    asm.load_selector(SEL_ALLOC.0);
    asm.push(abi::move_register("x0", "x21"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register("x21", "x0")); // allocated attributed string
    asm.load_selector(SEL_INIT_WITH_STRING_ATTRS.0);
    asm.push(abi::move_register("x2", "x20")); // string
    asm.push(abi::move_register("x3", "x22")); // attributes
    asm.push(abi::move_register("x0", "x21"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register("x20", "x0")); // attributed string

    // storage = [textView textStorage]
    asm.load_selector(SEL_TEXT_STORAGE.0);
    asm.push(abi::move_register("x0", "x19"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register("x19", "x0")); // text storage

    // [storage performSelectorOnMainThread:@selector(appendAttributedString:)
    //          withObject:attr waitUntilDone:YES]
    asm.load_selector(SEL_APPEND_ATTRIBUTED.0);
    asm.push(abi::move_register("x21", "x1")); // appendAttributedString: SEL
    asm.load_selector(SEL_PERFORM_ON_MAIN.0);
    asm.push(abi::move_register("x2", "x21"));
    asm.push(abi::move_register("x3", "x20"));
    asm.push(abi::move_immediate("x4", "Integer", "1")); // waitUntilDone YES
    asm.push(abi::move_register("x0", "x19")); // receiver: text storage
    asm.call_external("_objc_msgSend", LIB_OBJC);

    // [attr release] — the attributed string was created owned (alloc +
    // initWithString:attributes:, retain count 1) and appendAttributedString:
    // copies its contents rather than taking ownership, so we hold the sole
    // reference and must release it (bug-53). x20 (the attributed string) is
    // callee-saved and survives the appendAttributedString: msgSend above.
    asm.load_selector(SEL_RELEASE.0);
    asm.push(abi::move_register("x0", "x20")); // attributed string
    asm.call_external("_objc_msgSend", LIB_OBJC);

    // Drain this call's autoreleased objects (font, attributes dictionary).
    asm.push(abi::load_u64("x0", abi::stack_pointer(), 40)); // pool token
    asm.call_external("_objc_autoreleasePoolPop", LIB_OBJC);

    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::load_u64("x19", abi::stack_pointer(), 8));
    asm.push(abi::load_u64("x20", abi::stack_pointer(), 16));
    asm.push(abi::load_u64("x21", abi::stack_pointer(), 24));
    asm.push(abi::load_u64("x22", abi::stack_pointer(), 32));
    asm.push(abi::add_stack(48));
    asm.push(abi::return_());

    CodeFunction {
        name: "macapp.append".to_string(),
        symbol: APPEND_SYMBOL.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame: CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        stack_slots: Vec::new(),
        instructions: asm.ins,
        relocations: asm.rel,
    }
}

/// `void _mfb_macapp_program_finish(int code /*x0*/)`: the worker thread's
/// program-completion handler (plan §5.7). macOS `emit_program_exit` routes the
/// worker program's exit here instead of `_exit`.
///
/// Headless (no transcript view attached): `_exit(code)` — preserves the
/// console-like behavior the runtime tests rely on. GUI: append
/// `Program exited with code <N>` to the transcript and `pthread_exit` the worker
/// so the process keeps running with the window open; the app quits when the
/// window is closed (the synthesized delegate's
/// applicationShouldTerminateAfterLastWindowClosed: returns YES).
pub(super) fn emit_finish_helper(uses_term: bool) -> CodeFunction {
    let mut asm = Asm::new(FINISH_SYMBOL);
    // Frame: lr@0, x19(code)@8, x20(scratch/nsstring)@16, x21(textview)@24,
    // x22(digit count)@32, decimal digit buffer@40 (<=3 digits for 0..255).
    let frame = 48;
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(frame));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        0,
    ));
    asm.push(abi::store_u64("x19", abi::stack_pointer(), 8));
    asm.push(abi::store_u64("x20", abi::stack_pointer(), 16));
    asm.push(abi::store_u64("x21", abi::stack_pointer(), 24));
    asm.push(abi::store_u64("x22", abi::stack_pointer(), 32));
    // Auto-restore the transcript if the program left TUI mode active (plan §6.5).
    // `x19` is still the pinned arena-state base here, which `_mfb_rt_term_term_off`
    // reads; it gates on the active flag (a no-op when already off / headless), so
    // this is safe unconditionally. Preserve the exit code across the call.
    if uses_term {
        asm.push(abi::store_u64("x0", abi::stack_pointer(), 40));
        asm.call_internal("_mfb_rt_term_term_off");
        asm.push(abi::load_u64("x0", abi::stack_pointer(), 40));
    }
    asm.push(abi::move_register("x19", "x0")); // exit code

    // view = objc_getAssociatedObject([NSApplication sharedApplication], &KEY)
    asm.external_data("x21", CLASS_NS_APPLICATION, LIB_APPKIT);
    asm.load_selector(SEL_SHARED_APPLICATION.0);
    asm.push(abi::move_register("x0", "x21"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.local_address("x1", ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register("x21", "x0")); // transcript view or nil
    asm.push(abi::compare_immediate("x21", "0"));
    asm.push(abi::branch_eq("headless_exit"));

    // --- GUI: append the completion status line, then end the worker thread ---
    build_nsstring_from_cstring(&mut asm, "x20", STR_EXIT_PREFIX.0);
    asm.push(abi::move_register("x1", "x0"));
    asm.push(abi::move_register("x0", "x21"));
    asm.call_internal(APPEND_SYMBOL);

    // Format the exit code (0..255) as decimal ASCII into the stack buffer at +40,
    // leaving the digit count in x22. Pure register arithmetic, no calls.
    emit_format_exit_code(&mut asm, frame);

    // number = [[NSString alloc] initWithBytes:&buf length:x22 encoding:UTF8]
    asm.external_data("x20", CLASS_NS_STRING, LIB_FOUNDATION);
    asm.load_selector(SEL_ALLOC.0);
    asm.push(abi::move_register("x0", "x20"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register("x20", "x0")); // allocated NSString
    asm.load_selector(SEL_INIT_WITH_BYTES.0);
    asm.push(abi::add_immediate("x2", abi::stack_pointer(), 40));
    asm.push(abi::move_register("x3", "x22"));
    asm.push(abi::move_immediate("x4", "Integer", NS_UTF8_ENCODING));
    asm.push(abi::move_register("x0", "x20"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register("x1", "x0"));
    asm.push(abi::move_register("x0", "x21"));
    asm.call_internal(APPEND_SYMBOL);

    build_nsstring_from_cstring(&mut asm, "x20", STR_NEWLINE.0);
    asm.push(abi::move_register("x1", "x0"));
    asm.push(abi::move_register("x0", "x21"));
    asm.call_internal(APPEND_SYMBOL);

    // Park the worker thread (block in pause() forever); the main thread's event
    // loop keeps the window open until the user closes it, at which point the app
    // terminates the whole process. We must NOT pthread_exit here: the worker has
    // made Cocoa calls, and the thread-exit autorelease-pool cleanup crashes
    // draining them (SIGSEGV in objc release). Parking avoids any per-thread exit
    // cleanup.
    asm.push(abi::label("park"));
    asm.call_external("_pause", LIB_SYSTEM);
    asm.push(abi::branch("park"));

    // --- headless: terminate the process with the program's exit code ---
    asm.push(abi::label("headless_exit"));
    asm.push(abi::move_register("x0", "x19"));
    asm.call_external("_exit", LIB_SYSTEM);
    asm.push(abi::branch_self());
    asm.push(abi::return_());

    CodeFunction {
        name: "macapp.finish".to_string(),
        symbol: FINISH_SYMBOL.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame: CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        stack_slots: Vec::new(),
        instructions: asm.ins,
        relocations: asm.rel,
    }
}

/// Format the exit code in `x19` (0..255) as decimal ASCII into the stack buffer
/// at `sp+40`, leaving the digit count in `x22`. Leading zeros are suppressed.
/// Uses only caller-saved scratch registers and performs no calls.
fn emit_format_exit_code(asm: &mut Asm, _frame: usize) {
    // h = code/100; rem = code%100; t = rem/10; o = rem%10.
    asm.push(abi::move_register("x9", "x19")); // n
    // Mask to the low 8 bits: `_exit(status)` (the headless path) delivers only
    // `status & 0xFF` to the parent, so the GUI transcript must show that same
    // truncated value — never the raw code. Without this, a code > 255 (e.g.
    // 300 or 1000) formatted garbage digits (`'0' + 10 = ':'`) since only
    // hundreds/tens/ones are computed (bug-70). Masking bounds n to 0..255, so
    // hundreds <= 2 and every digit is valid.
    asm.push(abi::move_immediate("x11", "Integer", "255"));
    asm.push(abi::and_registers("x9", "x9", "x11"));
    asm.push(abi::move_immediate("x11", "Integer", "100"));
    asm.push(abi::unsigned_divide_registers("x10", "x9", "x11")); // hundreds
    asm.push(abi::multiply_subtract_registers("x9", "x10", "x11", "x9")); // n %= 100
    asm.push(abi::move_immediate("x11", "Integer", "10"));
    asm.push(abi::unsigned_divide_registers("x12", "x9", "x11")); // tens
    asm.push(abi::multiply_subtract_registers("x9", "x12", "x11", "x9")); // ones
                                                                          // write pointer x13 = sp+40; start x16 = x13.
    asm.push(abi::add_immediate("x13", abi::stack_pointer(), 40));
    asm.push(abi::move_register("x16", "x13"));
    // if hundreds != 0: emit hundreds, then always emit tens + ones.
    asm.push(abi::compare_immediate("x10", "0"));
    asm.push(abi::branch_eq("fmt_skip_h"));
    asm.push(abi::add_immediate("x14", "x10", 48));
    asm.push(abi::store_u8("x14", "x13", 0));
    asm.push(abi::add_immediate("x13", "x13", 1));
    asm.push(abi::branch("fmt_tens"));
    asm.push(abi::label("fmt_skip_h"));
    // else if tens == 0: skip tens (ones only).
    asm.push(abi::compare_immediate("x12", "0"));
    asm.push(abi::branch_eq("fmt_ones"));
    asm.push(abi::label("fmt_tens"));
    asm.push(abi::add_immediate("x14", "x12", 48));
    asm.push(abi::store_u8("x14", "x13", 0));
    asm.push(abi::add_immediate("x13", "x13", 1));
    asm.push(abi::label("fmt_ones"));
    asm.push(abi::add_immediate("x14", "x9", 48));
    asm.push(abi::store_u8("x14", "x13", 0));
    asm.push(abi::add_immediate("x13", "x13", 1));
    // x22 = digit count = x13 - x16.
    asm.push(abi::subtract_registers("x22", "x13", "x16"));
}

/// IMP for `applicationDidFinishLaunching:` — spawns the worker thread now that
/// AppKit has finished launching (plan-01-term.md §6.4). Runs on the main thread.
/// `void applicationDidFinishLaunching:(id self, SEL _cmd, id notification)`.
pub(super) fn emit_did_finish_launching_helper() -> CodeFunction {
    let mut asm = Asm::new(DID_FINISH_LAUNCHING_SYMBOL);
    // Frame: lr@0, pthread_t@8 (thrown away; the worker is never joined).
    let frame = 32;
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(frame));
    asm.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        0,
    ));

    // app = [NSApplication sharedApplication]
    asm.load_selector(SEL_SHARED_APPLICATION.0);
    asm.external_data("x0", CLASS_NS_APPLICATION, LIB_APPKIT);
    asm.call_external("_objc_msgSend", LIB_OBJC);
    // argblock = objc_getAssociatedObject(app, &ARG_ASSOC_KEY)
    asm.local_address("x1", ARG_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register("x3", "x0")); // arg = &argblock

    // pthread_create(&tid, NULL, _mfb_macapp_worker, &argblock)
    asm.push(abi::add_immediate("x0", abi::stack_pointer(), 8)); // &tid
    asm.push(abi::move_immediate("x1", "Integer", "0"));
    asm.local_address("x2", WORKER_SYMBOL);
    asm.call_external("_pthread_create", LIB_SYSTEM);

    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::add_stack(frame));
    asm.push(abi::return_());

    CodeFunction {
        name: "macapp.didFinishLaunching".to_string(),
        symbol: DID_FINISH_LAUNCHING_SYMBOL.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame: CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        stack_slots: Vec::new(),
        instructions: asm.ins,
        relocations: asm.rel,
    }
}

/// IMP for `applicationShouldTerminateAfterLastWindowClosed:` — returns YES so
/// closing the transcript window quits the app (plan §5.7).
pub(super) fn emit_should_terminate_helper() -> CodeFunction {
    let mut asm = Asm::new(SHOULD_TERMINATE_SYMBOL);
    asm.push(abi::label("entry"));
    asm.push(abi::move_immediate("x0", "Integer", "1")); // YES
    asm.push(abi::return_());
    CodeFunction {
        name: "macapp.shouldTerminate".to_string(),
        symbol: SHOULD_TERMINATE_SYMBOL.to_string(),
        params: Vec::new(),
        returns: "Boolean".to_string(),
        frame: CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        stack_slots: Vec::new(),
        instructions: asm.ins,
        relocations: asm.rel,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // bug-70: `emit_format_exit_code` only computed hundreds/tens/ones, so a
    // program exit code > 255 (e.g. 300 or 1000) rendered garbage digits in the
    // GUI transcript. It must first mask the code to its low 8 bits — the value
    // `_exit(status)` actually delivers to the parent — so the printed code is
    // always 0..255 and matches the headless path.
    #[test]
    fn exit_code_formatter_masks_to_low_8_bits() {
        let mut asm = Asm::new("test");
        emit_format_exit_code(&mut asm, 0);
        let field = |ins: &CodeInstruction, key: &str| -> Option<String> {
            ins.fields
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| v.clone())
        };
        // A `mov_imm <r>, 255` immediately followed by `and x9, x9, <r>` masks
        // the code copied into x9 before any digit is extracted.
        let has_mask = asm.ins.windows(2).any(|w| {
            let mask_reg = match (field(&w[0], "value"), field(&w[0], "dst")) {
                (Some(v), Some(reg)) if v == "255" => reg,
                _ => return false,
            };
            field(&w[1], "dst").as_deref() == Some("x9")
                && field(&w[1], "lhs").as_deref() == Some("x9")
                && field(&w[1], "rhs") == Some(mask_reg)
        });
        assert!(
            has_mask,
            "emit_format_exit_code must mask the exit code to 0..255 before formatting"
        );
    }
}
