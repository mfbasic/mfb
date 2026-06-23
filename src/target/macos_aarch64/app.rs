//! macOS app-mode (`mfb build -app`) runtime bootstrap codegen.
//!
//! Phase 3 of `specifications/plan-04-macos-app.md`: emit the app-mode `_main`
//! AppKit bootstrap and the pthread worker shim. `_main` runs on the process
//! main thread (AppKit's required home), creates the `NSApplication` and a
//! window via the Objective-C runtime, spawns a worker thread that runs the
//! standard MFBASIC program entry ([`code::MACAPP_PROGRAM_SYMBOL`]), then runs
//! the AppKit event loop.
//!
//! All Objective-C interaction goes through the public runtime functions
//! `objc_msgSend`/`sel_registerName`; classes are obtained by referencing the
//! `_OBJC_CLASS_$_*` data symbols through the GOT (which also force-loads
//! AppKit/Foundation). No private API is used (plan §4.4).

use crate::arch::aarch64::abi;
use crate::target::shared::code::{self, AppEntrySpec, CodeDataObject, CodeFrame, CodeFunction, CodeInstruction, CodeRelocation};

const MAIN_SYMBOL: &str = "_main";
const WORKER_SYMBOL: &str = "_mfb_macapp_worker";

/// NSApplicationActivationPolicyRegular.
const ACTIVATION_POLICY_REGULAR: &str = "0";
/// NSWindowStyleMask: Titled(1) | Closable(2) | Miniaturizable(4) | Resizable(8).
const WINDOW_STYLE_MASK: &str = "15";
/// NSBackingStoreBuffered.
const BACKING_BUFFERED: &str = "2";

// Read-only C-string data symbols referenced by the bootstrap.
const SEL_SHARED_APPLICATION: (&str, &str) = ("_mfb_macapp_sel_sharedApplication", "sharedApplication");
const SEL_SET_ACTIVATION_POLICY: (&str, &str) =
    ("_mfb_macapp_sel_setActivationPolicy", "setActivationPolicy:");
const SEL_ALLOC: (&str, &str) = ("_mfb_macapp_sel_alloc", "alloc");
const SEL_INIT_WINDOW: (&str, &str) = (
    "_mfb_macapp_sel_initWindow",
    "initWithContentRect:styleMask:backing:defer:",
);
const SEL_STRING_WITH_UTF8: (&str, &str) =
    ("_mfb_macapp_sel_stringWithUTF8String", "stringWithUTF8String:");
const SEL_SET_TITLE: (&str, &str) = ("_mfb_macapp_sel_setTitle", "setTitle:");
const SEL_MAKE_KEY_AND_ORDER_FRONT: (&str, &str) =
    ("_mfb_macapp_sel_makeKeyAndOrderFront", "makeKeyAndOrderFront:");
const SEL_ACTIVATE: (&str, &str) = (
    "_mfb_macapp_sel_activateIgnoringOtherApps",
    "activateIgnoringOtherApps:",
);
const SEL_RUN: (&str, &str) = ("_mfb_macapp_sel_run", "run");
const STR_TITLE: (&str, &str) = ("_mfb_macapp_str_title", "MFBASIC App");
/// When this environment variable is set the bootstrap skips showing the window
/// and the AppKit event loop, spawning the worker headlessly. This drives the
/// automated runtime tests (plan §7.2 Strategy A) through the same construction
/// and worker code the GUI path uses.
const STR_HEADLESS_ENV: (&str, &str) = ("_mfb_macapp_str_headless", "MFB_MACAPP_HEADLESS");

// Transcript view selectors (Phase 4 output path, plan §5.5).
const SEL_CONTENT_VIEW: (&str, &str) = ("_mfb_macapp_sel_contentView", "contentView");
const SEL_INIT_FRAME: (&str, &str) = ("_mfb_macapp_sel_initWithFrame", "initWithFrame:");
const SEL_SET_EDITABLE: (&str, &str) = ("_mfb_macapp_sel_setEditable", "setEditable:");
const SEL_SET_SELECTABLE: (&str, &str) = ("_mfb_macapp_sel_setSelectable", "setSelectable:");
const SEL_SET_DOCUMENT_VIEW: (&str, &str) =
    ("_mfb_macapp_sel_setDocumentView", "setDocumentView:");
const SEL_SET_HAS_VSCROLLER: (&str, &str) =
    ("_mfb_macapp_sel_setHasVerticalScroller", "setHasVerticalScroller:");
const SEL_SET_AUTORESIZING_MASK: (&str, &str) =
    ("_mfb_macapp_sel_setAutoresizingMask", "setAutoresizingMask:");
const SEL_ADD_SUBVIEW: (&str, &str) = ("_mfb_macapp_sel_addSubview", "addSubview:");

/// NSViewWidthSizable(2) | NSViewHeightSizable(16): the scroll view tracks the
/// window's content view on resize.
const AUTORESIZE_WIDTH_HEIGHT: &str = "18";
/// NSViewWidthSizable(2): the transcript text view widens with the scroll view.
const AUTORESIZE_WIDTH: &str = "2";
// Transcript append selectors.
const SEL_TEXT_STORAGE: (&str, &str) = ("_mfb_macapp_sel_textStorage", "textStorage");
const SEL_APPEND_ATTRIBUTED: (&str, &str) =
    ("_mfb_macapp_sel_appendAttributed", "appendAttributedString:");
const SEL_DICTIONARY_WITH_OBJECT: (&str, &str) =
    ("_mfb_macapp_sel_dictWithObject", "dictionaryWithObject:forKey:");
const SEL_INIT_WITH_STRING_ATTRS: (&str, &str) =
    ("_mfb_macapp_sel_initWithStringAttrs", "initWithString:attributes:");
const SEL_PERFORM_ON_MAIN: (&str, &str) = (
    "_mfb_macapp_sel_performOnMain",
    "performSelectorOnMainThread:withObject:waitUntilDone:",
);
const SEL_INIT_WITH_BYTES: (&str, &str) =
    ("_mfb_macapp_sel_initWithBytes", "initWithBytes:length:encoding:");
const STR_STDERR_PREFIX: (&str, &str) = ("_mfb_macapp_str_stderr_prefix", "[stderr] ");
const STR_NEWLINE: (&str, &str) = ("_mfb_macapp_str_newline", "\n");
/// The address of this 1-byte read-only symbol is the unique key under which the
/// transcript NSTextView is stored as an associated object on the shared
/// NSApplication (objc-runtime-managed per-process storage; avoids needing a
/// writable data segment). A nil result means "no window" (headless) -> fd sink.
const ASSOC_KEY: &str = "_mfb_macapp_textview_key";

// Phase 4a shutdown (plan §5.7): app delegate + completion status line.
const SEL_INIT: (&str, &str) = ("_mfb_macapp_sel_init", "init");
const SEL_SET_DELEGATE: (&str, &str) = ("_mfb_macapp_sel_setDelegate", "setDelegate:");
const SEL_APP_SHOULD_TERMINATE: (&str, &str) = (
    "_mfb_macapp_sel_appShouldTerminate",
    "applicationShouldTerminateAfterLastWindowClosed:",
);
const STR_DELEGATE_CLASS: (&str, &str) = ("_mfb_macapp_str_delegateClass", "MFBAppDelegate");
/// Objective-C method type encoding for `BOOL (id self, SEL _cmd, id sender)`.
const STR_DELEGATE_TYPES: (&str, &str) = ("_mfb_macapp_str_delegateTypes", "c@:@");
/// `applicationDidFinishLaunching:` — fired on the main thread once AppKit has
/// finished launching the app. The GUI worker thread is spawned from here (not
/// inline before `[NSApp run]`) so it never runs concurrently with
/// `-[NSApplication finishLaunching]`; touching AppKit/the Obj-C runtime from the
/// worker during launch corrupts the runtime and aborts (plan-01-term.md §6.4).
const SEL_APP_DID_FINISH_LAUNCHING: (&str, &str) = (
    "_mfb_macapp_sel_appDidFinishLaunching",
    "applicationDidFinishLaunching:",
);
/// IMP for the delegate's `applicationDidFinishLaunching:` (spawns the worker).
const DID_FINISH_LAUNCHING_SYMBOL: &str = "_mfb_macapp_did_finish_launching";
/// Associated-object key (its address) under which the bootstrap stashes the
/// `{argc, argv}` block pointer so the launch handler can pass it to the worker.
const ARG_ASSOC_KEY: &str = "_mfb_macapp_argblock_key";
const STR_EXIT_PREFIX: (&str, &str) =
    ("_mfb_macapp_str_exitPrefix", "\nProgram exited with code ");

// Monospaced transcript font (plan §5.5).
const SEL_USER_FIXED_FONT: (&str, &str) =
    ("_mfb_macapp_sel_userFixedFont", "userFixedPitchFontOfSize:");
const SEL_SET_FONT: (&str, &str) = ("_mfb_macapp_sel_setFont", "setFont:");
/// Point size for the fixed-pitch transcript font.
const TRANSCRIPT_FONT_SIZE: u32 = 13;
/// `NSFontAttributeName` — the attributed-string key carrying the transcript
/// font. Referenced as external data (an AppKit NSString global) via the GOT.
const NS_FONT_ATTRIBUTE_NAME: &str = "_NSFontAttributeName";

// Application menu with the standard Quit item.
const SEL_ADD_ITEM: (&str, &str) = ("_mfb_macapp_sel_addItem", "addItem:");
const SEL_SET_ACTION: (&str, &str) = ("_mfb_macapp_sel_setAction", "setAction:");
const SEL_SET_KEY_EQUIVALENT: (&str, &str) =
    ("_mfb_macapp_sel_setKeyEquivalent", "setKeyEquivalent:");
const SEL_SET_SUBMENU: (&str, &str) = ("_mfb_macapp_sel_setSubmenu", "setSubmenu:");
const SEL_SET_MAIN_MENU: (&str, &str) = ("_mfb_macapp_sel_setMainMenu", "setMainMenu:");
/// The standard NSApplication `terminate:` action wired to the Quit item.
const SEL_TERMINATE: (&str, &str) = ("_mfb_macapp_sel_terminate", "terminate:");
const STR_QUIT: (&str, &str) = ("_mfb_macapp_str_quit", "Quit");
const STR_QUIT_KEY: (&str, &str) = ("_mfb_macapp_str_quitKey", "q");

// Terminal-style input (plan §5.6): the transcript view (an MFBTextView subclass
// overriding keyDown:) receives typed keys directly. Line mode accumulates keys
// in a buffer until Return writes the buffered line to a pipe whose read end is
// dup2'd onto fd 0. Raw mode writes each key event's UTF-8 bytes to that pipe
// immediately so readChar/readByte do not wait for Return.
const SEL_UTF8_STRING: (&str, &str) = ("_mfb_macapp_sel_UTF8String", "UTF8String");
const SEL_MAKE_FIRST_RESPONDER: (&str, &str) =
    ("_mfb_macapp_sel_makeFirstResponder", "makeFirstResponder:");
const SEL_KEY_DOWN: (&str, &str) = ("_mfb_macapp_sel_keyDown", "keyDown:");
const SEL_CHARACTERS: (&str, &str) = ("_mfb_macapp_sel_characters", "characters");
const SEL_LENGTH: (&str, &str) = ("_mfb_macapp_sel_length", "length");
const SEL_CHAR_AT_INDEX: (&str, &str) =
    ("_mfb_macapp_sel_characterAtIndex", "characterAtIndex:");
const SEL_APPEND_STRING: (&str, &str) = ("_mfb_macapp_sel_appendString", "appendString:");
const SEL_SET_STRING: (&str, &str) = ("_mfb_macapp_sel_setString", "setString:");
const SEL_DELETE_RANGE: (&str, &str) =
    ("_mfb_macapp_sel_deleteCharsInRange", "deleteCharactersInRange:");
const SEL_STRING: (&str, &str) = ("_mfb_macapp_sel_string", "string");
const STR_TEXTVIEW_CLASS: (&str, &str) = ("_mfb_macapp_str_textviewClass", "MFBTextView");
/// Objective-C method type encoding for `void (id self, SEL _cmd, id arg)`.
const STR_INPUT_TYPES: (&str, &str) = ("_mfb_macapp_str_inputTypes", "v@:@");
const STR_EMPTY: (&str, &str) = ("_mfb_macapp_str_empty", "");
/// IMP for the transcript view's `keyDown:` override.
const KEY_DOWN_SYMBOL: &str = "_mfb_macapp_key_down";
/// Associated-object key (its address) for the input pipe's write fd on NSApp.
const PIPE_ASSOC_KEY: &str = "_mfb_macapp_pipe_key";
/// Associated-object key (its address) for the input-line NSMutableString buffer.
const INPUT_LINE_KEY: &str = "_mfb_macapp_inputline_key";
/// Associated-object key (its address) for the app input mode on NSApp.
const INPUT_MODE_KEY: &str = "_mfb_macapp_inputmode_key";
const INPUT_MODE_LINE_ECHO: &str = "1";
const INPUT_MODE_RAW_NO_ECHO: &str = "2";

/// NSUTF8StringEncoding.
const NS_UTF8_ENCODING: &str = "4";
/// The transcript NSTextView append helper emitted alongside the bootstrap.
const APPEND_SYMBOL: &str = "_mfb_macapp_append";
/// Console io.write / io.readLine runtime helpers that app-mode `io.input`
/// composes (force-emitted in app mode when io.input is used).
const IO_WRITE_SYMBOL: &str = "_mfb_rt_io_io_write";
const IO_READ_LINE_SYMBOL: &str = "_mfb_rt_io_io_readLine";

// io.terminalSize (plan §5.4): viewport columns/rows from the scroll view's
// content size and the monospaced font metrics.
const SEL_ENCLOSING_SCROLL_VIEW: (&str, &str) =
    ("_mfb_macapp_sel_enclosingScrollView", "enclosingScrollView");
const SEL_CONTENT_SIZE: (&str, &str) = ("_mfb_macapp_sel_contentSize", "contentSize");
const SEL_MAX_ADVANCEMENT: (&str, &str) =
    ("_mfb_macapp_sel_maximumAdvancement", "maximumAdvancement");
const SEL_LAYOUT_MANAGER: (&str, &str) = ("_mfb_macapp_sel_layoutManager", "layoutManager");
const SEL_DEFAULT_LINE_HEIGHT: (&str, &str) =
    ("_mfb_macapp_sel_defaultLineHeight", "defaultLineHeightForFont:");
/// The arena allocator (`lower_arena_alloc`): size in x0, align in x1; returns a
/// result tag in x0 and the block pointer in x1.
// Kept for plan-01-term.md Phase 5: the app-mode `term::terminalSize` retargets
// this transcript-viewport helper to read the `TermView` grid (§8.3). Unused
// until then now that `io::terminalSize` is removed (Phase 3).
#[allow(dead_code)]
const ARENA_ALLOC_SYMBOL: &str = "_mfb_arena_alloc";
/// `ERR_UNSUPPORTED` (`ERR_UNSUPPORTED_CODE` / `ERR_UNSUPPORTED_SYMBOL` in
/// src/target/shared/code/mod.rs): returned by the app terminal-size helper when
/// no transcript is attached. The `_mfb_str_error_unsupported` data object is
/// emitted by the shared lowering whenever `term::terminalSize` is used.
#[allow(dead_code)]
const ERR_UNSUPPORTED_CODE: &str = "77050007";
#[allow(dead_code)]
const ERR_UNSUPPORTED_SYMBOL: &str = "_mfb_str_error_unsupported";
/// Program-completion handler (plan §5.7): runs on the worker thread when the
/// MFBASIC program finishes. macOS `emit_program_exit` routes the worker
/// program's exit through this instead of `_exit` so the window can stay open.
pub(crate) const FINISH_SYMBOL: &str = "_mfb_macapp_program_finish";
/// IMP for the synthesized app delegate's
/// `applicationShouldTerminateAfterLastWindowClosed:` (returns YES so closing
/// the window quits the app).
const SHOULD_TERMINATE_SYMBOL: &str = "_mfb_macapp_should_terminate";

// `term::` macOS app backend (plan-01-term.md §6.3, Phase 4). A synthesized
// `TermView : NSView` grid surface, swapped in as the window content view while
// TUI mode is active and restored to the transcript scroll view on `term::off`.
const SEL_DRAW_RECT: (&str, &str) = ("_mfb_macapp_sel_drawRect", "drawRect:");
const SEL_IS_FLIPPED: (&str, &str) = ("_mfb_macapp_sel_isFlipped", "isFlipped");
const SEL_SET_CONTENT_VIEW: (&str, &str) =
    ("_mfb_macapp_sel_setContentView", "setContentView:");
const SEL_COLOR_WITH_RGBA: (&str, &str) = (
    "_mfb_macapp_sel_colorWithRGBA",
    "colorWithCalibratedRed:green:blue:alpha:",
);
const SEL_SET: (&str, &str) = ("_mfb_macapp_sel_set", "set");
const SEL_WHITE_COLOR: (&str, &str) = ("_mfb_macapp_sel_whiteColor", "whiteColor");
const SEL_BLACK_COLOR: (&str, &str) = ("_mfb_macapp_sel_blackColor", "blackColor");
const SEL_DRAW_AT_POINT: (&str, &str) =
    ("_mfb_macapp_sel_drawAtPoint", "drawAtPoint:withAttributes:");
const SEL_STRING_WITH_CHARS: (&str, &str) =
    ("_mfb_macapp_sel_stringWithChars", "stringWithCharacters:length:");
const SEL_DICTIONARY: (&str, &str) = ("_mfb_macapp_sel_dictionary", "dictionary");
const SEL_SET_OBJECT_FOR_KEY: (&str, &str) =
    ("_mfb_macapp_sel_setObjectForKey", "setObject:forKey:");
const SEL_SET_NEEDS_DISPLAY: (&str, &str) =
    ("_mfb_macapp_sel_setNeedsDisplay", "setNeedsDisplay:");
const SEL_MFB_WRITE_STRING: (&str, &str) = ("_mfb_macapp_sel_mfbWriteString", "mfbWriteString:");
/// `NSForegroundColorAttributeName` — attributed-string key for the glyph colour.
const NS_FOREGROUND_COLOR_ATTRIBUTE_NAME: &str = "_NSForegroundColorAttributeName";
/// IMP for the TermView `mfbWriteString:` main-thread write entry point.
const MFB_WRITE_STRING_SYMBOL: &str = "_mfb_macapp_term_writeString";
/// Obj-C method type encoding for `void (id, SEL, id)`.
const STR_WRITE_STRING_TYPES: (&str, &str) = ("_mfb_macapp_str_writeStringTypes", "v@:@");
/// Class names for the synthesized surface and the AppKit drawing primitives it
/// uses.
const CLASS_NS_VIEW: &str = "_OBJC_CLASS_$_NSView";
const CLASS_NS_COLOR: &str = "_OBJC_CLASS_$_NSColor";
const CLASS_NS_MUTABLE_DICTIONARY: &str = "_OBJC_CLASS_$_NSMutableDictionary";
const CLASS_NS_LAYOUT_MANAGER: &str = "_OBJC_CLASS_$_NSLayoutManager";
/// `void NSRectFill(NSRect)` — fills the rect (passed in d0..d3) with the current
/// graphics-context colour. An AppKit C function, not an Obj-C method.
const NS_RECT_FILL: &str = "_NSRectFill";
const STR_TERMVIEW_CLASS_NAME: (&str, &str) = ("_mfb_macapp_str_termviewClassName", "TermView");
/// Obj-C method type encodings: `drawRect:` is `void (id, SEL, NSRect)`,
/// `isFlipped` is `BOOL (id, SEL)`.
const STR_DRAW_RECT_TYPES: (&str, &str) = ("_mfb_macapp_str_drawRectTypes", "v@:{CGRect=dddd}");
const STR_IS_FLIPPED_TYPES: (&str, &str) = ("_mfb_macapp_str_isFlippedTypes", "c@:");

/// Associated-object keys (their unique addresses) under which the bootstrap
/// stashes the window, the transcript scroll view, and the TermView on NSApp so
/// the worker-thread `term::` helpers can reach them (plan-01-term.md §6.3).
const WINDOW_ASSOC_KEY: &str = "_mfb_macapp_window_key";
const SCROLLVIEW_ASSOC_KEY: &str = "_mfb_macapp_scrollview_key";
const TERMVIEW_ASSOC_KEY: &str = "_mfb_macapp_termview_key";
/// Associated-object key (its address) under which the TermView's `calloc`'d
/// grid-state struct is attached to the view (OBJC_ASSOCIATION_ASSIGN — the
/// runtime never messages it, it is a plain C buffer). This avoids the
/// `objc_allocateClassPair` extra-bytes / `object_getIndexedIvars` path, whose
/// storage is not reliably backed for these runtime-synthesized classes.
const TVSTATE_ASSOC_KEY: &str = "_mfb_macapp_termstate_key";

/// TermView grid-state struct (a `calloc`'d buffer attached via
/// [`TVSTATE_ASSOC_KEY`]). Twelve 8-byte fields = 96 bytes.
const TV_CELLS_OFFSET: usize = 0; // TermCell* heap grid (rows*cols cells)
const TV_ROWS_OFFSET: usize = 8; // i64 row count
const TV_COLS_OFFSET: usize = 16; // i64 column count
const TV_CURSOR_ROW_OFFSET: usize = 24; // i64 cursor row (0-based)
const TV_CURSOR_COL_OFFSET: usize = 32; // i64 cursor column (0-based)
const TV_CELL_W_OFFSET: usize = 40; // f64 cell width in points
const TV_CELL_H_OFFSET: usize = 48; // f64 cell height in points
const TV_CURSOR_VISIBLE_OFFSET: usize = 56; // i64 cursor-visible flag
// Current attributes applied to cells as they are written (the app-mode mirror of
// the term-state global; readable from the main-thread write/draw path).
const TV_CUR_FG_OFFSET: usize = 64; // u32 packed r|g<<8|b<<16 (default white)
const TV_CUR_BG_OFFSET: usize = 72; // u32 packed (default black = 0)
const TV_CUR_BOLD_OFFSET: usize = 80; // i64 bold flag
const TV_CUR_UNDERLINE_OFFSET: usize = 88; // i64 underline flag
const TV_STATE_SIZE: usize = 96;
/// Default foreground packed value (white), shared by term_init and the helpers.
const TERM_DEFAULT_FG_PACKED: &str = "16777215";

/// TermCell layout (16 bytes): a unichar glyph plus packed fg/bg colours and the
/// bold/underline flags. Mirrors the reference `.m` cell (plan §6.3).
const CELL_SIZE: usize = 16;
// Cell field offsets are consumed by the Phase 5 write/render path; declared now
// so the grid layout lives in one place alongside the Phase 4 grid allocation.
#[allow(dead_code)]
const CELL_GLYPH_OFFSET: usize = 0; // u32 unichar (0 / space = blank)
#[allow(dead_code)]
const CELL_FG_OFFSET: usize = 4; // u32 packed r|g<<8|b<<16
#[allow(dead_code)]
const CELL_BG_OFFSET: usize = 8; // u32 packed r|g<<8|b<<16
#[allow(dead_code)]
const CELL_BOLD_OFFSET: usize = 12; // u8 (Phase 5)
#[allow(dead_code)]
const CELL_UNDERLINE_OFFSET: usize = 13; // u8 (Phase 5)

/// Initial TermView frame (matches the window content rect set in the bootstrap).
const TERM_VIEW_WIDTH: u32 = 900;
const TERM_VIEW_HEIGHT: u32 = 640;

// Term-state global: the writable TUI-mode slots in the worker's program-entry
// frame, reached off the pinned arena-state register (plan-01-term.md §6.2). The
// app `term::on`/`term::off` helpers update the same slots the console backend
// does so `isOn`, the §4.2.1 gate, and auto-restore stay backend-uniform.
const TERM_ARENA_STATE_REG: &str = "x19";
/// Internal helper symbols for the synthesized surface.
const TERM_VIEW_DRAW_RECT_SYMBOL: &str = "_mfb_macapp_term_drawRect";
const TERM_VIEW_IS_FLIPPED_SYMBOL: &str = "_mfb_macapp_term_isFlipped";
const TERM_INIT_SYMBOL: &str = "_mfb_macapp_term_init";
const TERM_CLEAR_SYMBOL: &str = "_mfb_macapp_term_clear";
const TERM_SCROLL_SYMBOL: &str = "_mfb_macapp_term_scroll";

const CLASS_NS_OBJECT: &str = "_OBJC_CLASS_$_NSObject";
const CLASS_NS_APPLICATION: &str = "_OBJC_CLASS_$_NSApplication";
const CLASS_NS_WINDOW: &str = "_OBJC_CLASS_$_NSWindow";
const CLASS_NS_STRING: &str = "_OBJC_CLASS_$_NSString";
const CLASS_NS_MUTABLE_STRING: &str = "_OBJC_CLASS_$_NSMutableString";
const CLASS_NS_DICTIONARY: &str = "_OBJC_CLASS_$_NSDictionary";
const CLASS_NS_ATTRIBUTED_STRING: &str = "_OBJC_CLASS_$_NSAttributedString";
const CLASS_NS_SCROLL_VIEW: &str = "_OBJC_CLASS_$_NSScrollView";
const CLASS_NS_TEXT_VIEW: &str = "_OBJC_CLASS_$_NSTextView";
const CLASS_NS_FONT: &str = "_OBJC_CLASS_$_NSFont";
const CLASS_NS_MENU: &str = "_OBJC_CLASS_$_NSMenu";
const CLASS_NS_MENU_ITEM: &str = "_OBJC_CLASS_$_NSMenuItem";

const LIB_OBJC: &str = "libobjc";
const LIB_APPKIT: &str = "AppKit";
const LIB_FOUNDATION: &str = "Foundation";
const LIB_SYSTEM: &str = "libSystem";

/// Persistent (callee-saved) registers held across the external calls in `_main`.
const REG_APP: &str = "x19"; // NSApplication instance
const REG_WINDOW: &str = "x20"; // NSWindow instance
const REG_SCRATCH_OBJ: &str = "x21"; // transient object (class / NSString)
const REG_HEADLESS: &str = "x22"; // getenv("MFB_MACAPP_HEADLESS") result

// `_main` stack frame: [sp+0]=argc, [sp+8]=argv (worker arg block),
// [sp+16]=pthread_t, [sp+24..32]=input pipe fds (read, write).
const FRAME_SIZE: usize = 32;
const OFF_ARGC: usize = 0;
const OFF_ARGV: usize = 8;
const OFF_TID: usize = 16;
const OFF_PIPE: usize = 24;

/// Small instruction/relocation accumulator that records every relocation under
/// a single `from` symbol, keeping the objc_msgSend boilerplate compact.
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

    /// `bl <symbol>` to an external (imported) function.
    fn call_external(&mut self, symbol: &str, library: &str) {
        self.ins.push(abi::branch_link(symbol));
        self.rel.push(CodeRelocation {
            from: self.from.clone(),
            to: symbol.to_string(),
            kind: "branch26".to_string(),
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
            kind: "branch26".to_string(),
            binding: "internal".to_string(),
            library: None,
        });
    }

    /// Load the address of an internal data (or text) symbol into `dst`
    /// (`adrp`/`add`). The linker resolves the symbol's own vmaddr.
    fn local_address(&mut self, dst: &str, symbol: &str) {
        self.ins.push(
            CodeInstruction::new("adrp")
                .field("dst", dst)
                .field("symbol", symbol),
        );
        self.ins.push(
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

    /// Load an external data symbol's value into `dst`: `adrp`/`add` resolves the
    /// symbol's GOT slot address, then `ldr` dereferences it. Used for the
    /// `_OBJC_CLASS_$_*` class pointers (binds + force-loads the framework).
    fn external_data(&mut self, dst: &str, symbol: &str, library: &str) {
        self.ins.push(
            CodeInstruction::new("adrp")
                .field("dst", dst)
                .field("symbol", symbol),
        );
        self.ins.push(
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
                binding: "external".to_string(),
                library: Some(library.to_string()),
            });
        }
        self.ins.push(abi::load_u64(dst, dst, 0));
    }

    /// Resolve `selector_symbol`'s SEL via `sel_registerName`, leaving it in `x1`.
    /// Clobbers `x0`.
    fn load_selector(&mut self, selector_symbol: &str) {
        self.local_address("x0", selector_symbol);
        self.call_external("_sel_registerName", LIB_OBJC);
        self.push(abi::move_register("x1", "x0"));
    }
}

/// Emit the macOS app-mode `_main` bootstrap plus the worker shim. The standard
/// program entry is emitted separately by the shared lowering under
/// [`code::MACAPP_PROGRAM_SYMBOL`].
pub(crate) fn emit_app_program_entry(spec: &AppEntrySpec) -> Result<Vec<CodeFunction>, String> {
    Ok(vec![
        emit_main_bootstrap(),
        emit_worker_shim(spec),
        emit_append_helper(),
        emit_finish_helper(),
        emit_should_terminate_helper(),
        emit_did_finish_launching_helper(),
        emit_key_down_helper(),
        // term:: synthesized TermView surface (plan-01-term.md §6.3, Phase 4-5).
        emit_term_view_is_flipped(),
        emit_term_view_draw_rect(),
        emit_term_init_helper(),
        emit_term_clear_helper(),
        emit_term_scroll_helper(),
        emit_term_write_string_helper(),
    ])
}

fn emit_main_bootstrap() -> CodeFunction {
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
    asm.push(abi::move_immediate("x2", "Integer", ACTIVATION_POLICY_REGULAR));
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
    asm.push(abi::move_immediate("x2", "Integer", AUTORESIZE_WIDTH_HEIGHT));
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
    asm.push(abi::move_immediate("x2", "Integer", AUTORESIZE_WIDTH_HEIGHT));
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
    asm.push(abi::load_u32("x0", abi::stack_pointer(), OFF_PIPE)); // fds[0] (read)
    asm.push(abi::move_immediate("x1", "Integer", "0")); // newfd: stdin
    asm.call_external("_dup2", LIB_SYSTEM);
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
    asm.push(abi::label("spin"));
    asm.push(abi::branch_self());

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
fn emit_worker_shim(spec: &AppEntrySpec) -> CodeFunction {
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
fn emit_append_helper() -> CodeFunction {
    let mut asm = Asm::new(APPEND_SYMBOL);
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(48));
    asm.push(abi::store_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::store_u64("x19", abi::stack_pointer(), 8));
    asm.push(abi::store_u64("x20", abi::stack_pointer(), 16));
    asm.push(abi::store_u64("x21", abi::stack_pointer(), 24));
    asm.push(abi::store_u64("x22", abi::stack_pointer(), 32));
    asm.push(abi::move_register("x19", "x0")); // text view
    asm.push(abi::move_register("x20", "x1")); // nsstring

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
fn emit_finish_helper() -> CodeFunction {
    let mut asm = Asm::new(FINISH_SYMBOL);
    // Frame: lr@0, x19(code)@8, x20(scratch/nsstring)@16, x21(textview)@24,
    // x22(digit count)@32, decimal digit buffer@40 (<=3 digits for 0..255).
    let frame = 48;
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(frame));
    asm.push(abi::store_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::store_u64("x19", abi::stack_pointer(), 8));
    asm.push(abi::store_u64("x20", abi::stack_pointer(), 16));
    asm.push(abi::store_u64("x21", abi::stack_pointer(), 24));
    asm.push(abi::store_u64("x22", abi::stack_pointer(), 32));
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
fn emit_did_finish_launching_helper() -> CodeFunction {
    let mut asm = Asm::new(DID_FINISH_LAUNCHING_SYMBOL);
    // Frame: lr@0, pthread_t@8 (thrown away; the worker is never joined).
    let frame = 32;
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(frame));
    asm.push(abi::store_u64(abi::link_register(), abi::stack_pointer(), 0));

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
fn emit_should_terminate_helper() -> CodeFunction {
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

/// `void _mfb_macapp_key_down(id self /*x0 = MFBTextView*/, SEL _cmd, NSEvent
/// *event /*x2*/)`: terminal-style input (plan §5.6). The transcript view itself
/// receives keys; each printable key is echoed into the transcript and appended
/// to the input-line buffer, Backspace deletes the last character from both, and
/// Return commits the buffered line (UTF-8 bytes + newline) to the input pipe so
/// the program's reads on fd 0 receive it. Runs on the main thread, so the
/// synchronous transcript appends do not deadlock.
fn emit_key_down_helper() -> CodeFunction {
    let mut asm = Asm::new(KEY_DOWN_SYMBOL);
    // Frame: lr@0, x19(self)@8, x20(app)@16, x21(chars/cstr)@24,
    // x22(textStorage)@32, x23(event/scratch)@40, x24(char code)@48,
    // x25(input line)@56, x26(input mode)@64, newline byte@72.
    let frame = 96;
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(frame));
    asm.push(abi::store_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::store_u64("x19", abi::stack_pointer(), 8));
    asm.push(abi::store_u64("x20", abi::stack_pointer(), 16));
    asm.push(abi::store_u64("x21", abi::stack_pointer(), 24));
    asm.push(abi::store_u64("x22", abi::stack_pointer(), 32));
    asm.push(abi::store_u64("x23", abi::stack_pointer(), 40));
    asm.push(abi::store_u64("x24", abi::stack_pointer(), 48));
    asm.push(abi::store_u64("x25", abi::stack_pointer(), 56));
    asm.push(abi::store_u64("x26", abi::stack_pointer(), 64));
    asm.push(abi::move_register("x19", "x0")); // self (text view)
    asm.push(abi::move_register("x23", "x2")); // event

    // chars = [event characters]; if [chars length] == 0 (modifier-only) -> done
    asm.load_selector(SEL_CHARACTERS.0);
    asm.push(abi::move_register("x0", "x23"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register("x21", "x0")); // chars
    asm.load_selector(SEL_LENGTH.0);
    asm.push(abi::move_register("x0", "x21"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::compare_immediate("x0", "0"));
    asm.push(abi::branch_eq("kd_done"));
    // c = [chars characterAtIndex:0]
    asm.load_selector(SEL_CHAR_AT_INDEX.0);
    asm.push(abi::move_immediate("x2", "Integer", "0"));
    asm.push(abi::move_register("x0", "x21"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register("x24", "x0")); // char code

    // app, input line buffer, text storage.
    asm.external_data("x20", CLASS_NS_APPLICATION, LIB_APPKIT);
    asm.load_selector(SEL_SHARED_APPLICATION.0);
    asm.push(abi::move_register("x0", "x20"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register("x20", "x0")); // app
    asm.push(abi::move_register("x0", "x20"));
    asm.local_address("x1", INPUT_LINE_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register("x25", "x0")); // input line buffer
    asm.load_selector(SEL_TEXT_STORAGE.0);
    asm.push(abi::move_register("x0", "x19"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register("x22", "x0")); // text storage
    asm.push(abi::move_register("x0", "x20"));
    asm.local_address("x1", INPUT_MODE_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register("x26", "x0")); // input mode

    // Dispatch on the key.
    asm.push(abi::compare_immediate("x26", INPUT_MODE_RAW_NO_ECHO));
    asm.push(abi::branch_eq("kd_raw"));
    asm.push(abi::compare_immediate("x24", "13")); // CR
    asm.push(abi::branch_eq("kd_commit"));
    asm.push(abi::compare_immediate("x24", "10")); // LF
    asm.push(abi::branch_eq("kd_commit"));
    asm.push(abi::compare_immediate("x24", "3")); // Enter
    asm.push(abi::branch_eq("kd_commit"));
    asm.push(abi::compare_immediate("x24", "127")); // Delete
    asm.push(abi::branch_eq("kd_backspace"));
    asm.push(abi::compare_immediate("x24", "8")); // Backspace
    asm.push(abi::branch_eq("kd_backspace"));

    // Default: [inputLine appendString:chars]; echo only for io.input mode.
    asm.load_selector(SEL_APPEND_STRING.0);
    asm.push(abi::move_register("x2", "x21"));
    asm.push(abi::move_register("x0", "x25"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::compare_immediate("x26", INPUT_MODE_LINE_ECHO));
    asm.push(abi::branch_ne("kd_done"));
    asm.push(abi::move_register("x0", "x19"));
    asm.push(abi::move_register("x1", "x21"));
    asm.call_internal(APPEND_SYMBOL);
    asm.push(abi::branch("kd_done"));

    // Commit: deliver the buffered line + newline to the pipe, echo a newline,
    // clear the buffer.
    asm.push(abi::label("kd_commit"));
    asm.push(abi::move_register("x0", "x20"));
    asm.local_address("x1", PIPE_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register("x23", "x0")); // write fd
    asm.load_selector(SEL_UTF8_STRING.0);
    asm.push(abi::move_register("x0", "x25"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register("x21", "x0")); // UTF-8 bytes of the line
    asm.push(abi::move_register("x0", "x21"));
    asm.call_external("_strlen", LIB_SYSTEM);
    asm.push(abi::move_register("x2", "x0")); // length
    asm.push(abi::move_register("x0", "x23"));
    asm.push(abi::move_register("x1", "x21"));
    asm.call_external("_write", LIB_SYSTEM);
    asm.push(abi::move_immediate("x9", "Integer", "10"));
    asm.push(abi::store_u8("x9", abi::stack_pointer(), 72));
    asm.push(abi::move_register("x0", "x23"));
    asm.push(abi::add_immediate("x1", abi::stack_pointer(), 72));
    asm.push(abi::move_immediate("x2", "Integer", "1"));
    asm.call_external("_write", LIB_SYSTEM);
    asm.push(abi::compare_immediate("x26", INPUT_MODE_LINE_ECHO));
    asm.push(abi::branch_ne("kd_commit_clear"));
    build_nsstring_from_cstring(&mut asm, "x21", STR_NEWLINE.0);
    asm.push(abi::move_register("x1", "x0"));
    asm.push(abi::move_register("x0", "x19"));
    asm.call_internal(APPEND_SYMBOL);
    asm.push(abi::label("kd_commit_clear"));
    build_nsstring_from_cstring(&mut asm, "x21", STR_EMPTY.0);
    asm.push(abi::move_register("x24", "x0")); // empty string (callee-saved; survives
                                               // the sel_registerName in load_selector)
    asm.load_selector(SEL_SET_STRING.0);
    asm.push(abi::move_register("x2", "x24"));
    asm.push(abi::move_register("x0", "x25"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::branch("kd_done"));

    // Backspace: drop the last character from the buffer and the transcript.
    asm.push(abi::label("kd_backspace"));
    asm.load_selector(SEL_LENGTH.0);
    asm.push(abi::move_register("x0", "x25"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::compare_immediate("x0", "0"));
    asm.push(abi::branch_eq("kd_done"));
    asm.push(abi::move_register("x23", "x0")); // buffer length
    asm.load_selector(SEL_DELETE_RANGE.0);
    asm.push(abi::subtract_immediate("x2", "x23", 1)); // range.location = len - 1
    asm.push(abi::move_immediate("x3", "Integer", "1")); // range.length = 1
    asm.push(abi::move_register("x0", "x25"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::compare_immediate("x26", INPUT_MODE_LINE_ECHO));
    asm.push(abi::branch_ne("kd_done"));
    asm.load_selector(SEL_LENGTH.0);
    asm.push(abi::move_register("x0", "x22"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::compare_immediate("x0", "0"));
    asm.push(abi::branch_eq("kd_done"));
    asm.push(abi::move_register("x23", "x0")); // transcript length
    asm.load_selector(SEL_DELETE_RANGE.0);
    asm.push(abi::subtract_immediate("x2", "x23", 1));
    asm.push(abi::move_immediate("x3", "Integer", "1"));
    asm.push(abi::move_register("x0", "x22"));
    asm.call_external("_objc_msgSend", LIB_OBJC);

    // Raw read mode: write this key event's UTF-8 bytes to the input pipe now,
    // with no transcript echo and no line buffering.
    asm.push(abi::label("kd_raw"));
    asm.push(abi::move_register("x0", "x20"));
    asm.local_address("x1", PIPE_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register("x23", "x0")); // write fd
    asm.load_selector(SEL_UTF8_STRING.0);
    asm.push(abi::move_register("x0", "x21"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register("x21", "x0")); // UTF-8 bytes for chars
    asm.push(abi::move_register("x0", "x21"));
    asm.call_external("_strlen", LIB_SYSTEM);
    asm.push(abi::compare_immediate("x0", "0"));
    asm.push(abi::branch_eq("kd_done"));
    asm.push(abi::move_register("x2", "x0"));
    asm.push(abi::move_register("x0", "x23"));
    asm.push(abi::move_register("x1", "x21"));
    asm.call_external("_write", LIB_SYSTEM);
    asm.push(abi::branch("kd_done"));

    asm.push(abi::label("kd_done"));
    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::load_u64("x19", abi::stack_pointer(), 8));
    asm.push(abi::load_u64("x20", abi::stack_pointer(), 16));
    asm.push(abi::load_u64("x21", abi::stack_pointer(), 24));
    asm.push(abi::load_u64("x22", abi::stack_pointer(), 32));
    asm.push(abi::load_u64("x23", abi::stack_pointer(), 40));
    asm.push(abi::load_u64("x24", abi::stack_pointer(), 48));
    asm.push(abi::load_u64("x25", abi::stack_pointer(), 56));
    asm.push(abi::load_u64("x26", abi::stack_pointer(), 64));
    asm.push(abi::add_stack(frame));
    asm.push(abi::return_());

    CodeFunction {
        name: "macapp.keyDown".to_string(),
        symbol: KEY_DOWN_SYMBOL.to_string(),
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
) -> (CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>) {
    let mut asm = Asm::new(symbol);
    let fd = if stderr { "2" } else { "1" };
    // lr@0, x19(string)@8, x20(view)@16, x21(scratch)@24, nl byte@32, x22(sel)@40
    let frame = 48;
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(frame));
    asm.push(abi::store_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::store_u64("x19", abi::stack_pointer(), 8));
    asm.push(abi::store_u64("x20", abi::stack_pointer(), 16));
    asm.push(abi::store_u64("x21", abi::stack_pointer(), 24));
    asm.push(abi::store_u64("x22", abi::stack_pointer(), 40));
    // While TUI mode is active, route to the TermView surface (x19 is the pinned
    // arena-state base on entry, before it is reused for the string object).
    if let Some(off) = term_state_offset {
        asm.push(abi::load_u64(
            "x9",
            TERM_ARENA_STATE_REG,
            off + code::TERM_STATE_ACTIVE_OFFSET,
        ));
        asm.push(abi::move_register("x19", "x0")); // string object
        asm.push(abi::compare_immediate("x9", "0"));
        asm.push(abi::branch_ne("term_surface_path"));
    } else {
        asm.push(abi::move_register("x19", "x0")); // string object
    }

    // app = [NSApplication sharedApplication]; view = objc_getAssociatedObject(app, &KEY)
    asm.external_data("x20", CLASS_NS_APPLICATION, LIB_APPKIT);
    asm.load_selector(SEL_SHARED_APPLICATION.0);
    asm.push(abi::move_register("x0", "x20"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.local_address("x1", ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register("x20", "x0")); // transcript view or nil
    asm.push(abi::compare_immediate("x20", "0"));
    asm.push(abi::branch_eq("fd_path"));

    // --- GUI transcript path ---
    if stderr {
        // Visually distinguish stderr with a "[stderr] " marker (plan §5.4).
        build_nsstring_from_cstring(&mut asm, "x21", STR_STDERR_PREFIX.0);
        asm.push(abi::move_register("x1", "x0"));
        asm.push(abi::move_register("x0", "x20"));
        asm.call_internal(APPEND_SYMBOL);
    }
    // text = [[NSString alloc] initWithBytes:(str+8) length:str[0] encoding:UTF8]
    asm.external_data("x21", CLASS_NS_STRING, LIB_FOUNDATION);
    asm.load_selector(SEL_ALLOC.0);
    asm.push(abi::move_register("x0", "x21"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register("x21", "x0")); // allocated NSString
    asm.load_selector(SEL_INIT_WITH_BYTES.0);
    asm.push(abi::add_immediate("x2", "x19", 8)); // bytes
    asm.push(abi::load_u64("x3", "x19", 0)); // length
    asm.push(abi::move_immediate("x4", "Integer", NS_UTF8_ENCODING));
    asm.push(abi::move_register("x0", "x21"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register("x1", "x0")); // text nsstring
    asm.push(abi::move_register("x0", "x20"));
    asm.call_internal(APPEND_SYMBOL);
    if newline {
        build_nsstring_from_cstring(&mut asm, "x21", STR_NEWLINE.0);
        asm.push(abi::move_register("x1", "x0"));
        asm.push(abi::move_register("x0", "x20"));
        asm.call_internal(APPEND_SYMBOL);
    }
    asm.push(abi::move_immediate("x0", "Integer", "0")); // RESULT_OK_TAG
    asm.push(abi::branch("done"));

    // --- headless / no-window path: write to the file descriptor ---
    asm.push(abi::label("fd_path"));
    asm.push(abi::move_immediate("x0", "Integer", fd));
    asm.push(abi::add_immediate("x1", "x19", 8));
    asm.push(abi::load_u64("x2", "x19", 0));
    asm.call_external("_write", LIB_SYSTEM);
    if newline {
        asm.push(abi::move_immediate("x9", "Integer", "10")); // '\n'
        asm.push(abi::store_u8("x9", abi::stack_pointer(), 32));
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
        asm.external_data("x20", CLASS_NS_APPLICATION, LIB_APPKIT);
        asm.load_selector(SEL_SHARED_APPLICATION.0);
        asm.push(abi::move_register("x0", "x20"));
        asm.call_external("_objc_msgSend", LIB_OBJC);
        asm.local_address("x1", TERMVIEW_ASSOC_KEY);
        asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
        asm.push(abi::move_register("x20", "x0")); // termview or nil
        asm.push(abi::compare_immediate("x20", "0"));
        asm.push(abi::branch_eq("fd_path")); // headless: no surface -> fd
        // text = [[NSString alloc] initWithBytes:(str+8) length:str[0] encoding:UTF8]
        asm.external_data("x21", CLASS_NS_STRING, LIB_FOUNDATION);
        asm.load_selector(SEL_ALLOC.0);
        asm.push(abi::move_register("x0", "x21"));
        asm.call_external("_objc_msgSend", LIB_OBJC);
        asm.push(abi::move_register("x21", "x0"));
        asm.load_selector(SEL_INIT_WITH_BYTES.0);
        asm.push(abi::add_immediate("x2", "x19", 8));
        asm.push(abi::load_u64("x3", "x19", 0));
        asm.push(abi::move_immediate("x4", "Integer", NS_UTF8_ENCODING));
        asm.push(abi::move_register("x0", "x21"));
        asm.call_external("_objc_msgSend", LIB_OBJC);
        asm.push(abi::move_register("x21", "x0")); // text nsstring
        // [tv performSelectorOnMainThread:@selector(mfbWriteString:) withObject:text waitUntilDone:YES]
        asm.load_selector(SEL_MFB_WRITE_STRING.0);
        asm.push(abi::move_register("x22", "x1")); // mfbWriteString: sel
        asm.load_selector(SEL_PERFORM_ON_MAIN.0);
        asm.push(abi::move_register("x2", "x22"));
        asm.push(abi::move_register("x3", "x21"));
        asm.push(abi::move_immediate("x4", "Integer", "1")); // waitUntilDone: YES
        asm.push(abi::move_register("x0", "x20"));
        asm.call_external("_objc_msgSend", LIB_OBJC);
        if newline {
            build_nsstring_from_cstring(&mut asm, "x21", STR_NEWLINE.0);
            asm.push(abi::move_register("x21", "x0")); // "\n" nsstring
            asm.load_selector(SEL_MFB_WRITE_STRING.0);
            asm.push(abi::move_register("x22", "x1"));
            asm.load_selector(SEL_PERFORM_ON_MAIN.0);
            asm.push(abi::move_register("x2", "x22"));
            asm.push(abi::move_register("x3", "x21"));
            asm.push(abi::move_immediate("x4", "Integer", "1"));
            asm.push(abi::move_register("x0", "x20"));
            asm.call_external("_objc_msgSend", LIB_OBJC);
        }
        asm.push(abi::move_immediate("x0", "Integer", "0")); // RESULT_OK_TAG
    }

    asm.push(abi::label("done"));
    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::load_u64("x19", abi::stack_pointer(), 8));
    asm.push(abi::load_u64("x20", abi::stack_pointer(), 16));
    asm.push(abi::load_u64("x21", abi::stack_pointer(), 24));
    asm.push(abi::load_u64("x22", abi::stack_pointer(), 40));
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

/// App-mode body for `io.flush`/`io.flushError`: transcript writes are already
/// synchronous (see [`emit_append_helper`]), so flush just returns success.
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

/// App-mode body for `io.input` (plan §5.4): render the prompt to the transcript
/// via the `io.write` helper, then read a committed line via the `io.readLine`
/// helper (which reads fd 0 — the window input pipe in app mode). The prompt
/// string is passed in `x0`; `io.readLine` takes no arguments, so its result
/// (`x0`/`x1`/`x2`) becomes this helper's result.
pub(crate) fn emit_app_io_input_helper(
    symbol: &str,
) -> (CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>) {
    let mut asm = Asm::new(symbol);
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(16));
    asm.push(abi::store_u64(abi::link_register(), abi::stack_pointer(), 0));
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
    asm.load_selector(SEL_SHARED_APPLICATION.0);
    asm.external_data("x0", CLASS_NS_APPLICATION, LIB_APPKIT);
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.local_address("x1", INPUT_MODE_KEY);
    asm.push(abi::move_immediate("x2", "Integer", mode));
    asm.push(abi::move_immediate("x3", "Integer", "0")); // OBJC_ASSOCIATION_ASSIGN
    asm.call_external("_objc_setAssociatedObject", LIB_OBJC);
}

/// App-mode body for `io.isInputTerminal`/`io.isOutputTerminal`/
/// `io.isErrorTerminal` (plan §5.4): the window is the interactive console, so
/// all three return `OK(TRUE)`. Result ABI: x0 = tag (0 = ok), x1 = value.
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

/// App-mode body for `io.terminalSize` (plan §5.4): the transcript viewport size
/// in text columns/rows. columns = floor(contentWidth / charWidth), rows =
/// floor(contentHeight / lineHeight), where contentWidth/Height come from the
/// scroll view's `contentSize`, charWidth from the monospaced font's
/// `maximumAdvancement`, and lineHeight from the layout manager. Returns the
/// `{ columns, rows }` record, or `ERR_UNSUPPORTED` when no transcript is
/// attached (e.g. headless).
// Retained for plan-01-term.md Phase 5 (app-mode `term::terminalSize`, §8.3);
// unused since `io::terminalSize` was removed in Phase 3.
#[allow(dead_code)]
pub(crate) fn emit_app_io_terminal_size_helper(
    symbol: &str,
) -> (CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>) {
    // Frame: lr@0, x19(font)@8, x20(text view)@16, x21(scratch obj)@24,
    // width@32, height@40, charWidth@48, lineHeight@56, columns@64, rows@72.
    let frame = 80;
    let (off_w, off_h, off_cw, off_lh, off_col, off_row) = (32, 40, 48, 56, 64, 72);
    let mut asm = Asm::new(symbol);
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(frame));
    asm.push(abi::store_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::store_u64("x19", abi::stack_pointer(), 8));
    asm.push(abi::store_u64("x20", abi::stack_pointer(), 16));
    asm.push(abi::store_u64("x21", abi::stack_pointer(), 24));

    // app; textview = objc_getAssociatedObject(app, &ASSOC_KEY); require non-nil.
    asm.external_data("x21", CLASS_NS_APPLICATION, LIB_APPKIT);
    asm.load_selector(SEL_SHARED_APPLICATION.0);
    asm.push(abi::move_register("x0", "x21"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.local_address("x1", ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register("x20", "x0")); // text view (or nil when headless)
    asm.push(abi::compare_immediate("x20", "0"));
    asm.push(abi::branch_eq("ts_error"));

    // sv = [textview enclosingScrollView]; require non-nil.
    asm.load_selector(SEL_ENCLOSING_SCROLL_VIEW.0);
    asm.push(abi::move_register("x0", "x20"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::compare_immediate("x0", "0"));
    asm.push(abi::branch_eq("ts_error"));
    asm.push(abi::move_register("x21", "x0")); // scroll view

    // size = [sv contentSize] -> d0 = width, d1 = height; spill both.
    asm.load_selector(SEL_CONTENT_SIZE.0);
    asm.push(abi::move_register("x0", "x21"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::float_move_x_from_d("x9", "d0"));
    asm.push(abi::store_u64("x9", abi::stack_pointer(), off_w));
    asm.push(abi::float_move_x_from_d("x9", "d1"));
    asm.push(abi::store_u64("x9", abi::stack_pointer(), off_h));

    // font = [NSFont userFixedPitchFontOfSize:N]
    asm.external_data("x19", CLASS_NS_FONT, LIB_APPKIT);
    asm.load_selector(SEL_USER_FIXED_FONT.0);
    emit_double_immediate(&mut asm, "d0", TRANSCRIPT_FONT_SIZE);
    asm.push(abi::move_register("x0", "x19"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register("x19", "x0")); // font

    // charWidth = [font maximumAdvancement].width (d0); spill.
    asm.load_selector(SEL_MAX_ADVANCEMENT.0);
    asm.push(abi::move_register("x0", "x19"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::float_move_x_from_d("x9", "d0"));
    asm.push(abi::store_u64("x9", abi::stack_pointer(), off_cw));

    // lineHeight = [[textview layoutManager] defaultLineHeightForFont:font] (d0).
    asm.load_selector(SEL_LAYOUT_MANAGER.0);
    asm.push(abi::move_register("x0", "x20"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register("x21", "x0")); // layout manager
    asm.load_selector(SEL_DEFAULT_LINE_HEIGHT.0);
    asm.push(abi::move_register("x2", "x19")); // font
    asm.push(abi::move_register("x0", "x21"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::float_move_x_from_d("x9", "d0"));
    asm.push(abi::store_u64("x9", abi::stack_pointer(), off_lh));

    // columns = floor(width / charWidth); rows = floor(height / lineHeight).
    asm.push(abi::load_u64("x9", abi::stack_pointer(), off_w));
    asm.push(abi::float_move_d_from_x("d0", "x9"));
    asm.push(abi::load_u64("x9", abi::stack_pointer(), off_cw));
    asm.push(abi::float_move_d_from_x("d1", "x9"));
    asm.push(abi::float_divide_d("d0", "d0", "d1"));
    asm.push(abi::float_floor_to_signed_x("x10", "d0")); // columns
    asm.push(abi::load_u64("x9", abi::stack_pointer(), off_h));
    asm.push(abi::float_move_d_from_x("d0", "x9"));
    asm.push(abi::load_u64("x9", abi::stack_pointer(), off_lh));
    asm.push(abi::float_move_d_from_x("d1", "x9"));
    asm.push(abi::float_divide_d("d0", "d0", "d1"));
    asm.push(abi::float_floor_to_signed_x("x11", "d0")); // rows
    asm.push(abi::compare_immediate("x10", "0"));
    asm.push(abi::branch_le("ts_error"));
    asm.push(abi::compare_immediate("x11", "0"));
    asm.push(abi::branch_le("ts_error"));

    // Allocate the { columns, rows } record (16 bytes, 8-aligned). Spill
    // columns/rows first; _mfb_arena_alloc clobbers x10/x11/x20-x28.
    asm.push(abi::store_u64("x10", abi::stack_pointer(), off_col));
    asm.push(abi::store_u64("x11", abi::stack_pointer(), off_row));
    asm.push(abi::move_immediate("x0", "Integer", "16"));
    asm.push(abi::move_immediate("x1", "Integer", "8"));
    asm.call_internal(ARENA_ALLOC_SYMBOL);
    asm.push(abi::compare_immediate("x0", "0")); // RESULT_OK_TAG
    asm.push(abi::branch_ne("ts_error"));
    asm.push(abi::load_u64("x10", abi::stack_pointer(), off_col));
    asm.push(abi::load_u64("x11", abi::stack_pointer(), off_row));
    asm.push(abi::store_u64("x10", "x1", 0)); // columns @ 0
    asm.push(abi::store_u64("x11", "x1", 8)); // rows @ 8
    asm.push(abi::move_immediate("x0", "Integer", "0")); // tag = OK (x1 = record)
    asm.push(abi::branch("ts_done"));

    // No transcript / unusable size -> ERR_UNSUPPORTED.
    asm.push(abi::label("ts_error"));
    asm.push(abi::move_immediate("x1", "Integer", ERR_UNSUPPORTED_CODE)); // value = code
    asm.push(abi::move_immediate("x0", "Integer", "1")); // tag = ERR
    asm.push(
        CodeInstruction::new("adrp")
            .field("dst", "x2")
            .field("symbol", ERR_UNSUPPORTED_SYMBOL),
    );
    asm.push(
        CodeInstruction::new("add_pageoff")
            .field("dst", "x2")
            .field("src", "x2")
            .field("symbol", ERR_UNSUPPORTED_SYMBOL),
    );
    for kind in ["page21", "pageoff12"] {
        asm.rel.push(CodeRelocation {
            from: symbol.to_string(),
            to: ERR_UNSUPPORTED_SYMBOL.to_string(),
            kind: kind.to_string(),
            binding: "data".to_string(),
            library: None,
        });
    }

    asm.push(abi::label("ts_done"));
    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::load_u64("x19", abi::stack_pointer(), 8));
    asm.push(abi::load_u64("x20", abi::stack_pointer(), 16));
    asm.push(abi::load_u64("x21", abi::stack_pointer(), 24));
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

/// IMP for `TermView`'s `isFlipped` override — returns YES so row 0 is at the
/// top of the view and cell `(row, col)` maps to `(col*cellW, row*cellH)` in the
/// flipped coordinate space (plan-01-term.md §6.3).
fn emit_term_view_is_flipped() -> CodeFunction {
    let mut asm = Asm::new(TERM_VIEW_IS_FLIPPED_SYMBOL);
    asm.push(abi::label("entry"));
    asm.push(abi::move_immediate("x0", "Integer", "1")); // YES
    asm.push(abi::return_());
    CodeFunction {
        name: "macapp.term.isFlipped".to_string(),
        symbol: TERM_VIEW_IS_FLIPPED_SYMBOL.to_string(),
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

/// IMP for `TermView`'s `drawRect:` (`void drawRect:(NSRect dirty)`; self in x0,
/// `_cmd` in x1, the rect in d0..d3).
///
/// Fills the dirty rect black, then paints each non-blank cell glyph at
/// `(col*cellW, row*cellH)` in the monospaced font (plan-01-term.md §6.3). This
/// increment renders monochrome (white on black); per-cell fg/bg colour grows on
/// top in the next increment ("smallest correct `drawRect:` first", plan §9.1).
fn emit_term_view_draw_rect() -> CodeFunction {
    let mut asm = Asm::new(TERM_VIEW_DRAW_RECT_SYMBOL);
    // Frame: lr@0; callee-saved x19(state)@8, x20(cells)@16, x21(rows)@24,
    // x22(cols)@32, x23(row)@40, x24(col)@48, x25(font)@56, x26(attrs)@64,
    // x28(drawAtPoint sel)@72; rect x/y/w/h@80..104; stringWithChars sel@112;
    // glyph unichar buffer@120.
    let frame = 144;
    let (off_rx, off_ry, off_rw, off_rh) = (80, 88, 96, 104);
    let (off_swc_sel, off_glyph) = (112, 120);
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(frame));
    asm.push(abi::store_u64(abi::link_register(), abi::stack_pointer(), 0));
    for (reg, off) in [
        ("x19", 8),
        ("x20", 16),
        ("x21", 24),
        ("x22", 32),
        ("x23", 40),
        ("x24", 48),
        ("x25", 56),
        ("x26", 64),
        ("x28", 72),
    ] {
        asm.push(abi::store_u64(reg, abi::stack_pointer(), off));
    }
    // Spill the dirty rect (d0..d3) before any call clobbers the FP arg regs.
    for (reg, off) in [("d0", off_rx), ("d1", off_ry), ("d2", off_rw), ("d3", off_rh)] {
        asm.push(abi::float_move_x_from_d("x9", reg));
        asm.push(abi::store_u64("x9", abi::stack_pointer(), off));
    }

    // state = objc_getAssociatedObject(self, &TVSTATE_KEY)  (self in x0)
    asm.local_address("x1", TVSTATE_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register("x19", "x0")); // state (or nil)

    // Fill the dirty rect black: [[NSColor blackColor] set]; NSRectFill(rect).
    asm.load_selector(SEL_BLACK_COLOR.0);
    asm.external_data("x0", CLASS_NS_COLOR, LIB_APPKIT);
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register("x26", "x0")); // black colour (temp)
    asm.load_selector(SEL_SET.0);
    asm.push(abi::move_register("x0", "x26"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    for (reg, off) in [("d0", off_rx), ("d1", off_ry), ("d2", off_rw), ("d3", off_rh)] {
        asm.push(abi::load_u64("x9", abi::stack_pointer(), off));
        asm.push(abi::float_move_d_from_x(reg, "x9"));
    }
    asm.call_external(NS_RECT_FILL, LIB_APPKIT);

    // No state / no grid yet -> nothing more to paint.
    asm.push(abi::compare_immediate("x19", "0"));
    asm.push(abi::branch_eq("draw_done"));
    asm.push(abi::load_u64("x20", "x19", TV_CELLS_OFFSET)); // cells
    asm.push(abi::compare_immediate("x20", "0"));
    asm.push(abi::branch_eq("draw_done"));
    asm.push(abi::load_u64("x21", "x19", TV_ROWS_OFFSET));
    asm.push(abi::load_u64("x22", "x19", TV_COLS_OFFSET));

    // font = [NSFont userFixedPitchFontOfSize:N]
    asm.external_data("x25", CLASS_NS_FONT, LIB_APPKIT);
    asm.load_selector(SEL_USER_FIXED_FONT.0);
    emit_double_immediate(&mut asm, "d0", TRANSCRIPT_FONT_SIZE);
    asm.push(abi::move_register("x0", "x25"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register("x25", "x0")); // font

    // attrs = [NSMutableDictionary dictionary]
    asm.load_selector(SEL_DICTIONARY.0);
    asm.external_data("x0", CLASS_NS_MUTABLE_DICTIONARY, LIB_FOUNDATION);
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register("x26", "x0")); // attrs dict
    // [attrs setObject:font forKey:NSFontAttributeName]
    asm.load_selector(SEL_SET_OBJECT_FOR_KEY.0);
    asm.push(abi::move_register("x2", "x25"));
    asm.external_data("x3", NS_FONT_ATTRIBUTE_NAME, LIB_APPKIT);
    asm.push(abi::load_u64("x3", "x3", 0));
    asm.push(abi::move_register("x0", "x26"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    // white = [NSColor whiteColor]; [attrs setObject:white forKey:NSForegroundColorAttributeName]
    // (font is already retained by the dict, so x25 is free to hold white across
    // the following load_selector — whose sel_registerName call clobbers x0..x18).
    asm.load_selector(SEL_WHITE_COLOR.0);
    asm.external_data("x0", CLASS_NS_COLOR, LIB_APPKIT);
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register("x25", "x0")); // white (callee-saved)
    asm.load_selector(SEL_SET_OBJECT_FOR_KEY.0);
    asm.push(abi::move_register("x2", "x25"));
    asm.external_data("x3", NS_FOREGROUND_COLOR_ATTRIBUTE_NAME, LIB_APPKIT);
    asm.push(abi::load_u64("x3", "x3", 0));
    asm.push(abi::move_register("x0", "x26"));
    asm.call_external("_objc_msgSend", LIB_OBJC);

    // Pre-resolve the per-glyph selectors (drawAtPoint:withAttributes: into x28,
    // stringWithCharacters:length: spilled).
    asm.load_selector(SEL_DRAW_AT_POINT.0);
    asm.push(abi::move_register("x28", "x1"));
    asm.load_selector(SEL_STRING_WITH_CHARS.0);
    asm.push(abi::store_u64("x1", abi::stack_pointer(), off_swc_sel));

    // for row in 0..rows: for col in 0..cols
    asm.push(abi::move_immediate("x23", "Integer", "0"));
    asm.push(abi::label("draw_row"));
    asm.push(abi::compare_registers("x23", "x21"));
    asm.push(abi::branch_ge("draw_done"));
    asm.push(abi::move_immediate("x24", "Integer", "0"));
    asm.push(abi::label("draw_col"));
    asm.push(abi::compare_registers("x24", "x22"));
    asm.push(abi::branch_ge("draw_row_next"));

    // cell = cells + (row*cols + col) * CELL_SIZE; glyph = cell.glyph
    asm.push(abi::multiply_registers("x9", "x23", "x22"));
    asm.push(abi::add_registers("x9", "x9", "x24"));
    asm.push(abi::shift_left_immediate("x9", "x9", 4)); // * CELL_SIZE (16)
    asm.push(abi::add_registers("x9", "x20", "x9")); // cell ptr
    asm.push(abi::load_u32("x10", "x9", CELL_GLYPH_OFFSET));
    asm.push(abi::compare_immediate("x10", "0"));
    asm.push(abi::branch_eq("draw_col_next"));
    asm.push(abi::compare_immediate("x10", "32")); // space = blank
    asm.push(abi::branch_eq("draw_col_next"));

    // s = [NSString stringWithCharacters:&glyph length:1]
    asm.push(abi::store_u32("x10", abi::stack_pointer(), off_glyph));
    asm.push(abi::load_u64("x1", abi::stack_pointer(), off_swc_sel));
    asm.external_data("x0", CLASS_NS_STRING, LIB_FOUNDATION);
    asm.push(abi::add_immediate("x2", abi::stack_pointer(), off_glyph));
    asm.push(abi::move_immediate("x3", "Integer", "1"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    // [s drawAtPoint:(col*cellW, row*cellH) withAttributes:attrs]
    asm.push(abi::load_u64("x11", "x19", TV_CELL_W_OFFSET));
    asm.push(abi::float_move_d_from_x("d4", "x11"));
    asm.push(abi::load_u64("x11", "x19", TV_CELL_H_OFFSET));
    asm.push(abi::float_move_d_from_x("d5", "x11"));
    asm.push(abi::signed_convert_to_float_d("d6", "x24"));
    asm.push(abi::float_multiply_d("d0", "d6", "d4")); // px = col*cellW
    asm.push(abi::signed_convert_to_float_d("d7", "x23"));
    asm.push(abi::float_multiply_d("d1", "d7", "d5")); // py = row*cellH
    asm.push(abi::move_register("x2", "x26")); // attrs
    asm.push(abi::move_register("x1", "x28")); // drawAtPoint:withAttributes: sel
    asm.call_external("_objc_msgSend", LIB_OBJC);

    asm.push(abi::label("draw_col_next"));
    asm.push(abi::add_immediate("x24", "x24", 1));
    asm.push(abi::branch("draw_col"));
    asm.push(abi::label("draw_row_next"));
    asm.push(abi::add_immediate("x23", "x23", 1));
    asm.push(abi::branch("draw_row"));

    asm.push(abi::label("draw_done"));
    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    for (reg, off) in [
        ("x19", 8),
        ("x20", 16),
        ("x21", 24),
        ("x22", 32),
        ("x23", 40),
        ("x24", 48),
        ("x25", 56),
        ("x26", 64),
        ("x28", 72),
    ] {
        asm.push(abi::load_u64(reg, abi::stack_pointer(), off));
    }
    asm.push(abi::add_stack(frame));
    asm.push(abi::return_());

    CodeFunction {
        name: "macapp.term.drawRect".to_string(),
        symbol: TERM_VIEW_DRAW_RECT_SYMBOL.to_string(),
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

/// `void _mfb_macapp_term_init(id termView /*x0*/)`: size the TermView's cell
/// grid from the monospaced font metrics and the initial view frame, then
/// `calloc` the (zero-initialized = cleared) grid into the state struct held in
/// the view's extra bytes (plan-01-term.md §6.3). Called once from the bootstrap.
fn emit_term_init_helper() -> CodeFunction {
    let mut asm = Asm::new(TERM_INIT_SYMBOL);
    // Frame: lr@0, x19(termView)@8, x20(state)@16, x21(font)@24, x22(scratch)@32,
    // cellW bits@40, cellH bits@48.
    let frame = 64;
    let (off_cw, off_lh) = (40, 48);
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(frame));
    asm.push(abi::store_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::store_u64("x19", abi::stack_pointer(), 8));
    asm.push(abi::store_u64("x20", abi::stack_pointer(), 16));
    asm.push(abi::store_u64("x21", abi::stack_pointer(), 24));
    asm.push(abi::store_u64("x22", abi::stack_pointer(), 32));
    asm.push(abi::move_register("x19", "x0")); // termView

    // state = calloc(1, TV_STATE_SIZE) — zero-initialized grid state struct.
    asm.push(abi::move_immediate("x0", "Integer", "1"));
    asm.push(abi::move_immediate("x1", "Integer", &TV_STATE_SIZE.to_string()));
    asm.call_external("_calloc", LIB_SYSTEM);
    asm.push(abi::move_register("x20", "x0")); // state struct ptr

    // font = [NSFont userFixedPitchFontOfSize:N]
    asm.external_data("x21", CLASS_NS_FONT, LIB_APPKIT);
    asm.load_selector(SEL_USER_FIXED_FONT.0);
    emit_double_immediate(&mut asm, "d0", TRANSCRIPT_FONT_SIZE);
    asm.push(abi::move_register("x0", "x21"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register("x21", "x0")); // font

    // cellW = [font maximumAdvancement].width (d0); spill bits.
    asm.load_selector(SEL_MAX_ADVANCEMENT.0);
    asm.push(abi::move_register("x0", "x21"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::float_move_x_from_d("x9", "d0"));
    asm.push(abi::store_u64("x9", abi::stack_pointer(), off_cw));

    // lm = [[NSLayoutManager alloc] init]; cellH = [lm defaultLineHeightForFont:font].
    asm.external_data("x22", CLASS_NS_LAYOUT_MANAGER, LIB_APPKIT);
    asm.load_selector(SEL_ALLOC.0);
    asm.push(abi::move_register("x0", "x22"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register("x22", "x0"));
    asm.load_selector(SEL_INIT.0);
    asm.push(abi::move_register("x0", "x22"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register("x22", "x0")); // layout manager
    asm.load_selector(SEL_DEFAULT_LINE_HEIGHT.0);
    asm.push(abi::move_register("x2", "x21")); // font
    asm.push(abi::move_register("x0", "x22"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::float_move_x_from_d("x9", "d0"));
    asm.push(abi::store_u64("x9", abi::stack_pointer(), off_lh));

    // cols = floor(WIDTH / cellW); rows = floor(HEIGHT / cellH).
    asm.push(abi::load_u64("x9", abi::stack_pointer(), off_cw));
    asm.push(abi::float_move_d_from_x("d1", "x9"));
    asm.push(abi::move_immediate("x9", "Integer", &TERM_VIEW_WIDTH.to_string()));
    asm.push(abi::signed_convert_to_float_d("d0", "x9"));
    asm.push(abi::float_divide_d("d0", "d0", "d1"));
    asm.push(abi::float_floor_to_signed_x("x9", "d0"));
    asm.push(abi::store_u64("x9", "x20", TV_COLS_OFFSET));
    asm.push(abi::load_u64("x9", abi::stack_pointer(), off_lh));
    asm.push(abi::float_move_d_from_x("d1", "x9"));
    asm.push(abi::move_immediate("x9", "Integer", &TERM_VIEW_HEIGHT.to_string()));
    asm.push(abi::signed_convert_to_float_d("d0", "x9"));
    asm.push(abi::float_divide_d("d0", "d0", "d1"));
    asm.push(abi::float_floor_to_signed_x("x9", "d0"));
    asm.push(abi::store_u64("x9", "x20", TV_ROWS_OFFSET));

    // Persist the cell pixel dimensions for drawRect: / cursor positioning.
    asm.push(abi::load_u64("x9", abi::stack_pointer(), off_cw));
    asm.push(abi::store_u64("x9", "x20", TV_CELL_W_OFFSET));
    asm.push(abi::load_u64("x9", abi::stack_pointer(), off_lh));
    asm.push(abi::store_u64("x9", "x20", TV_CELL_H_OFFSET));

    // cells = calloc(rows*cols, CELL_SIZE) — zero-initialized = cleared grid.
    asm.push(abi::load_u64("x9", "x20", TV_COLS_OFFSET));
    asm.push(abi::load_u64("x10", "x20", TV_ROWS_OFFSET));
    asm.push(abi::multiply_registers("x0", "x9", "x10"));
    asm.push(abi::move_immediate("x1", "Integer", &CELL_SIZE.to_string()));
    asm.call_external("_calloc", LIB_SYSTEM);
    asm.push(abi::store_u64("x0", "x20", TV_CELLS_OFFSET));

    // cursor (0,0; calloc already zeroed); cursor visible; current fg = white
    // (bg/bold/underline default to 0 from calloc).
    asm.push(abi::move_immediate("x9", "Integer", "1"));
    asm.push(abi::store_u64("x9", "x20", TV_CURSOR_VISIBLE_OFFSET));
    asm.push(abi::move_immediate("x9", "Integer", TERM_DEFAULT_FG_PACKED));
    asm.push(abi::store_u64("x9", "x20", TV_CUR_FG_OFFSET));

    // objc_setAssociatedObject(termView, &TVSTATE_KEY, state, ASSIGN)
    asm.push(abi::move_register("x0", "x19"));
    asm.local_address("x1", TVSTATE_ASSOC_KEY);
    asm.push(abi::move_register("x2", "x20"));
    asm.push(abi::move_immediate("x3", "Integer", "0")); // OBJC_ASSOCIATION_ASSIGN
    asm.call_external("_objc_setAssociatedObject", LIB_OBJC);

    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::load_u64("x19", abi::stack_pointer(), 8));
    asm.push(abi::load_u64("x20", abi::stack_pointer(), 16));
    asm.push(abi::load_u64("x21", abi::stack_pointer(), 24));
    asm.push(abi::load_u64("x22", abi::stack_pointer(), 32));
    asm.push(abi::add_stack(frame));
    asm.push(abi::return_());

    CodeFunction {
        name: "macapp.term.init".to_string(),
        symbol: TERM_INIT_SYMBOL.to_string(),
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

/// `void _mfb_macapp_term_clear(id termView /*x0*/)`: zero every grid cell (the
/// cleared-to-background-black, blank-glyph state) and home the cursor. Pure data
/// mutation on our own heap, safe from the worker thread (plan-01-term.md §6.4).
fn emit_term_clear_helper() -> CodeFunction {
    let mut asm = Asm::new(TERM_CLEAR_SYMBOL);
    // Frame: lr@0, x19(state, after spilling the caller's arena base)@8.
    let frame = 32;
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(frame));
    asm.push(abi::store_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::store_u64("x19", abi::stack_pointer(), 8));

    // state = objc_getAssociatedObject(termView, &TVSTATE_KEY)  (x0 = termView)
    asm.local_address("x1", TVSTATE_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register("x19", "x0")); // state struct ptr
    asm.push(abi::compare_immediate("x19", "0"));
    asm.push(abi::branch_eq("clr_done")); // no state attached yet

    // bzero(cells, rows*cols*CELL_SIZE) when a grid is allocated.
    asm.push(abi::load_u64("x9", "x19", TV_CELLS_OFFSET));
    asm.push(abi::compare_immediate("x9", "0"));
    asm.push(abi::branch_eq("clr_cursor"));
    asm.push(abi::load_u64("x10", "x19", TV_ROWS_OFFSET));
    asm.push(abi::load_u64("x11", "x19", TV_COLS_OFFSET));
    asm.push(abi::multiply_registers("x10", "x10", "x11"));
    asm.push(abi::shift_left_immediate("x10", "x10", 4)); // * CELL_SIZE (16)
    asm.push(abi::move_register("x0", "x9"));
    asm.push(abi::move_register("x1", "x10"));
    asm.call_external("_bzero", LIB_SYSTEM);

    asm.push(abi::label("clr_cursor"));
    asm.push(abi::move_immediate("x9", "Integer", "0"));
    asm.push(abi::store_u64("x9", "x19", TV_CURSOR_ROW_OFFSET));
    asm.push(abi::store_u64("x9", "x19", TV_CURSOR_COL_OFFSET));

    asm.push(abi::label("clr_done"));
    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::load_u64("x19", abi::stack_pointer(), 8));
    asm.push(abi::add_stack(frame));
    asm.push(abi::return_());

    CodeFunction {
        name: "macapp.term.clear".to_string(),
        symbol: TERM_CLEAR_SYMBOL.to_string(),
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

/// `void _mfb_macapp_term_scroll(void *state /*x0*/)`: scroll the grid up one row
/// (memmove rows 1.. to 0.., then clear the new bottom row). Main-thread only.
fn emit_term_scroll_helper() -> CodeFunction {
    let mut asm = Asm::new(TERM_SCROLL_SYMBOL);
    // Frame: lr@0, x19(rowBytes)@8, x20(cells)@16, x21(rows)@24.
    let frame = 48;
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(frame));
    asm.push(abi::store_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::store_u64("x19", abi::stack_pointer(), 8));
    asm.push(abi::store_u64("x20", abi::stack_pointer(), 16));
    asm.push(abi::store_u64("x21", abi::stack_pointer(), 24));

    asm.push(abi::load_u64("x20", "x0", TV_CELLS_OFFSET)); // cells
    asm.push(abi::load_u64("x21", "x0", TV_ROWS_OFFSET)); // rows
    asm.push(abi::load_u64("x9", "x0", TV_COLS_OFFSET)); // cols
    asm.push(abi::shift_left_immediate("x19", "x9", 4)); // rowBytes = cols*CELL_SIZE

    // memmove(cells, cells + rowBytes, (rows-1)*rowBytes)
    asm.push(abi::subtract_immediate("x9", "x21", 1));
    asm.push(abi::multiply_registers("x2", "x9", "x19")); // len
    asm.push(abi::move_register("x0", "x20")); // dst
    asm.push(abi::add_registers("x1", "x20", "x19")); // src
    asm.call_external("_memmove", LIB_SYSTEM);

    // bzero(cells + (rows-1)*rowBytes, rowBytes) — clear the new bottom row.
    asm.push(abi::subtract_immediate("x9", "x21", 1));
    asm.push(abi::multiply_registers("x9", "x9", "x19"));
    asm.push(abi::add_registers("x0", "x20", "x9"));
    asm.push(abi::move_register("x1", "x19"));
    asm.call_external("_bzero", LIB_SYSTEM);

    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::load_u64("x19", abi::stack_pointer(), 8));
    asm.push(abi::load_u64("x20", abi::stack_pointer(), 16));
    asm.push(abi::load_u64("x21", abi::stack_pointer(), 24));
    asm.push(abi::add_stack(frame));
    asm.push(abi::return_());

    CodeFunction {
        name: "macapp.term.scroll".to_string(),
        symbol: TERM_SCROLL_SYMBOL.to_string(),
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

/// IMP for `TermView mfbWriteString:` (`void mfbWriteString:(id self, SEL _cmd,
/// NSString *str)`): write `str` into the grid at the cursor using the current
/// attributes, honouring `\n`/`\r`/`\t`, wrapping at the right edge and scrolling
/// at the bottom, then `setNeedsDisplay:` (plan-01-term.md §4.8). Main-thread only
/// (invoked via performSelectorOnMainThread), so grid mutation and redraw are
/// serialized in program order with the other surface ops (§6.4).
fn emit_term_write_string_helper() -> CodeFunction {
    let mut asm = Asm::new(MFB_WRITE_STRING_SYMBOL);
    // Frame: lr@0, x19(self)@8, x20(str)@16, x21(state)@24, x22(cells)@32,
    // x23(i)@40, x24(n)@48, x25(cols)@56, x26(rows)@64, x27(char)@72.
    let frame = 96;
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(frame));
    asm.push(abi::store_u64(abi::link_register(), abi::stack_pointer(), 0));
    for (reg, off) in [
        ("x19", 8),
        ("x20", 16),
        ("x21", 24),
        ("x22", 32),
        ("x23", 40),
        ("x24", 48),
        ("x25", 56),
        ("x26", 64),
        ("x27", 72),
    ] {
        asm.push(abi::store_u64(reg, abi::stack_pointer(), off));
    }
    asm.push(abi::move_register("x19", "x0")); // self
    asm.push(abi::move_register("x20", "x2")); // str

    // state = objc_getAssociatedObject(self, &TVSTATE_KEY)
    asm.local_address("x1", TVSTATE_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register("x21", "x0"));
    asm.push(abi::compare_immediate("x21", "0"));
    asm.push(abi::branch_eq("w_redraw"));
    asm.push(abi::load_u64("x22", "x21", TV_CELLS_OFFSET)); // cells
    asm.push(abi::compare_immediate("x22", "0"));
    asm.push(abi::branch_eq("w_redraw"));
    asm.push(abi::load_u64("x25", "x21", TV_COLS_OFFSET));
    asm.push(abi::load_u64("x26", "x21", TV_ROWS_OFFSET));

    // n = [str length]; i = 0
    asm.load_selector(SEL_LENGTH.0);
    asm.push(abi::move_register("x0", "x20"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register("x24", "x0"));
    asm.push(abi::move_immediate("x23", "Integer", "0"));

    asm.push(abi::label("w_loop"));
    asm.push(abi::compare_registers("x23", "x24"));
    asm.push(abi::branch_ge("w_redraw"));
    // c = [str characterAtIndex:i]
    asm.load_selector(SEL_CHAR_AT_INDEX.0);
    asm.push(abi::move_register("x2", "x23"));
    asm.push(abi::move_register("x0", "x20"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register("x27", "x0")); // char code

    asm.push(abi::compare_immediate("x27", "10")); // \n
    asm.push(abi::branch_eq("w_newline"));
    asm.push(abi::compare_immediate("x27", "13")); // \r
    asm.push(abi::branch_eq("w_cr"));
    asm.push(abi::compare_immediate("x27", "9")); // \t
    asm.push(abi::branch_eq("w_tab"));

    // printable: wrap if cursor_col >= cols
    asm.push(abi::load_u64("x9", "x21", TV_CURSOR_COL_OFFSET));
    asm.push(abi::compare_registers("x9", "x25"));
    asm.push(abi::branch_lt("w_col_ok"));
    asm.push(abi::move_immediate("x9", "Integer", "0"));
    asm.push(abi::store_u64("x9", "x21", TV_CURSOR_COL_OFFSET));
    asm.push(abi::load_u64("x10", "x21", TV_CURSOR_ROW_OFFSET));
    asm.push(abi::add_immediate("x10", "x10", 1));
    asm.push(abi::store_u64("x10", "x21", TV_CURSOR_ROW_OFFSET));
    asm.push(abi::label("w_col_ok"));
    // scroll if cursor_row >= rows
    asm.push(abi::load_u64("x10", "x21", TV_CURSOR_ROW_OFFSET));
    asm.push(abi::compare_registers("x10", "x26"));
    asm.push(abi::branch_lt("w_row_ok"));
    asm.push(abi::move_register("x0", "x21"));
    asm.call_internal(TERM_SCROLL_SYMBOL);
    asm.push(abi::subtract_immediate("x10", "x26", 1));
    asm.push(abi::store_u64("x10", "x21", TV_CURSOR_ROW_OFFSET));
    asm.push(abi::label("w_row_ok"));
    // cell = cells + (row*cols + col)*CELL_SIZE
    asm.push(abi::load_u64("x10", "x21", TV_CURSOR_ROW_OFFSET));
    asm.push(abi::load_u64("x11", "x21", TV_CURSOR_COL_OFFSET));
    asm.push(abi::multiply_registers("x12", "x10", "x25"));
    asm.push(abi::add_registers("x12", "x12", "x11"));
    asm.push(abi::shift_left_immediate("x12", "x12", 4));
    asm.push(abi::add_registers("x12", "x22", "x12")); // cell ptr
    asm.push(abi::store_u32("x27", "x12", CELL_GLYPH_OFFSET));
    asm.push(abi::load_u64("x13", "x21", TV_CUR_FG_OFFSET));
    asm.push(abi::store_u32("x13", "x12", CELL_FG_OFFSET));
    asm.push(abi::load_u64("x13", "x21", TV_CUR_BG_OFFSET));
    asm.push(abi::store_u32("x13", "x12", CELL_BG_OFFSET));
    asm.push(abi::load_u64("x13", "x21", TV_CUR_BOLD_OFFSET));
    asm.push(abi::store_u8("x13", "x12", CELL_BOLD_OFFSET));
    asm.push(abi::load_u64("x13", "x21", TV_CUR_UNDERLINE_OFFSET));
    asm.push(abi::store_u8("x13", "x12", CELL_UNDERLINE_OFFSET));
    // cursor_col++
    asm.push(abi::load_u64("x11", "x21", TV_CURSOR_COL_OFFSET));
    asm.push(abi::add_immediate("x11", "x11", 1));
    asm.push(abi::store_u64("x11", "x21", TV_CURSOR_COL_OFFSET));
    asm.push(abi::branch("w_next"));

    // \n: col = 0, row++ (scroll if needed)
    asm.push(abi::label("w_newline"));
    asm.push(abi::move_immediate("x9", "Integer", "0"));
    asm.push(abi::store_u64("x9", "x21", TV_CURSOR_COL_OFFSET));
    asm.push(abi::load_u64("x10", "x21", TV_CURSOR_ROW_OFFSET));
    asm.push(abi::add_immediate("x10", "x10", 1));
    asm.push(abi::store_u64("x10", "x21", TV_CURSOR_ROW_OFFSET));
    asm.push(abi::compare_registers("x10", "x26"));
    asm.push(abi::branch_lt("w_next"));
    asm.push(abi::move_register("x0", "x21"));
    asm.call_internal(TERM_SCROLL_SYMBOL);
    asm.push(abi::subtract_immediate("x10", "x26", 1));
    asm.push(abi::store_u64("x10", "x21", TV_CURSOR_ROW_OFFSET));
    asm.push(abi::branch("w_next"));

    // \r: col = 0
    asm.push(abi::label("w_cr"));
    asm.push(abi::move_immediate("x9", "Integer", "0"));
    asm.push(abi::store_u64("x9", "x21", TV_CURSOR_COL_OFFSET));
    asm.push(abi::branch("w_next"));

    // \t: col = (col & ~3) + 4, wrapping to a new line if it runs off the edge
    asm.push(abi::label("w_tab"));
    asm.push(abi::load_u64("x9", "x21", TV_CURSOR_COL_OFFSET));
    asm.push(abi::shift_right_immediate("x9", "x9", 2));
    asm.push(abi::shift_left_immediate("x9", "x9", 2));
    asm.push(abi::add_immediate("x9", "x9", 4));
    asm.push(abi::store_u64("x9", "x21", TV_CURSOR_COL_OFFSET));
    asm.push(abi::compare_registers("x9", "x25"));
    asm.push(abi::branch_lt("w_next"));
    asm.push(abi::move_immediate("x9", "Integer", "0"));
    asm.push(abi::store_u64("x9", "x21", TV_CURSOR_COL_OFFSET));
    asm.push(abi::load_u64("x10", "x21", TV_CURSOR_ROW_OFFSET));
    asm.push(abi::add_immediate("x10", "x10", 1));
    asm.push(abi::store_u64("x10", "x21", TV_CURSOR_ROW_OFFSET));
    asm.push(abi::compare_registers("x10", "x26"));
    asm.push(abi::branch_lt("w_next"));
    asm.push(abi::move_register("x0", "x21"));
    asm.call_internal(TERM_SCROLL_SYMBOL);
    asm.push(abi::subtract_immediate("x10", "x26", 1));
    asm.push(abi::store_u64("x10", "x21", TV_CURSOR_ROW_OFFSET));

    asm.push(abi::label("w_next"));
    asm.push(abi::add_immediate("x23", "x23", 1));
    asm.push(abi::branch("w_loop"));

    asm.push(abi::label("w_redraw"));
    asm.load_selector(SEL_SET_NEEDS_DISPLAY.0);
    asm.push(abi::move_immediate("x2", "Integer", "1")); // YES
    asm.push(abi::move_register("x0", "x19"));
    asm.call_external("_objc_msgSend", LIB_OBJC);

    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    for (reg, off) in [
        ("x19", 8),
        ("x20", 16),
        ("x21", 24),
        ("x22", 32),
        ("x23", 40),
        ("x24", 48),
        ("x25", 56),
        ("x26", 64),
        ("x27", 72),
    ] {
        asm.push(abi::load_u64(reg, abi::stack_pointer(), off));
    }
    asm.push(abi::add_stack(frame));
    asm.push(abi::return_());

    CodeFunction {
        name: "macapp.term.writeString".to_string(),
        symbol: MFB_WRITE_STRING_SYMBOL.to_string(),
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

/// Store an immediate into a term-state-global slot reached off the pinned
/// arena-state register (plan-01-term.md §6.2).
fn store_term_state(asm: &mut Asm, term_state_offset: usize, field_offset: usize, value: &str) {
    asm.push(abi::move_immediate("x9", "Integer", value));
    asm.push(abi::store_u64(
        "x9",
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
) -> (CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>) {
    let mut asm = Asm::new(symbol);
    // Frame: lr@0, x20(app)@8, x21(window)@16, x22(termview)@24, x23(sel)@32.
    let frame = 48;
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(frame));
    asm.push(abi::store_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::store_u64("x20", abi::stack_pointer(), 8));
    asm.push(abi::store_u64("x21", abi::stack_pointer(), 16));
    asm.push(abi::store_u64("x22", abi::stack_pointer(), 24));
    asm.push(abi::store_u64("x23", abi::stack_pointer(), 32));

    // Reset all term state to defaults (active on, fg white, bg black, bold and
    // underline off, cursor visible). x19 is the pinned arena-state base.
    store_term_state(&mut asm, term_state_offset, code::TERM_STATE_ACTIVE_OFFSET, "1");
    store_term_state(
        &mut asm,
        term_state_offset,
        code::TERM_STATE_FG_OFFSET,
        "16777215",
    );
    store_term_state(&mut asm, term_state_offset, code::TERM_STATE_BG_OFFSET, "0");
    store_term_state(&mut asm, term_state_offset, code::TERM_STATE_BOLD_OFFSET, "0");
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

    // app = [NSApplication sharedApplication]
    asm.external_data("x20", CLASS_NS_APPLICATION, LIB_APPKIT);
    asm.load_selector(SEL_SHARED_APPLICATION.0);
    asm.push(abi::move_register("x0", "x20"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register("x20", "x0")); // app

    // window = objc_getAssociatedObject(app, &WINDOW_ASSOC_KEY); nil -> headless.
    asm.push(abi::move_register("x0", "x20"));
    asm.local_address("x1", WINDOW_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register("x21", "x0")); // window or nil
    asm.push(abi::compare_immediate("x21", "0"));
    asm.push(abi::branch_eq("term_on_done"));

    // termview = objc_getAssociatedObject(app, &TERMVIEW_ASSOC_KEY)
    asm.push(abi::move_register("x0", "x20"));
    asm.local_address("x1", TERMVIEW_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register("x22", "x0")); // termview

    // Clear the grid + home the cursor before the surface is displayed.
    asm.push(abi::move_register("x0", "x22"));
    asm.call_internal(TERM_CLEAR_SYMBOL);

    // [window performSelectorOnMainThread:@selector(setContentView:)
    //         withObject:termview waitUntilDone:YES]  (AppKit is main-thread only)
    asm.load_selector(SEL_SET_CONTENT_VIEW.0);
    asm.push(abi::move_register("x23", "x1")); // setContentView: sel
    asm.load_selector(SEL_PERFORM_ON_MAIN.0);
    asm.push(abi::move_register("x2", "x23"));
    asm.push(abi::move_register("x3", "x22"));
    asm.push(abi::move_immediate("x4", "Integer", "1")); // waitUntilDone: YES
    asm.push(abi::move_register("x0", "x21"));
    asm.call_external("_objc_msgSend", LIB_OBJC);

    asm.push(abi::label("term_on_done"));
    asm.push(abi::move_immediate("x0", "Integer", "0")); // RESULT_OK_TAG
    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::load_u64("x20", abi::stack_pointer(), 8));
    asm.push(abi::load_u64("x21", abi::stack_pointer(), 16));
    asm.push(abi::load_u64("x22", abi::stack_pointer(), 24));
    asm.push(abi::load_u64("x23", abi::stack_pointer(), 32));
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
pub(crate) fn emit_app_term_off_helper(
    symbol: &str,
    term_state_offset: usize,
) -> (CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>) {
    let mut asm = Asm::new(symbol);
    // Frame: lr@0, x20(app)@8, x21(window)@16, x22(scrollview)@24, x23(sel)@32.
    let frame = 48;
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(frame));
    asm.push(abi::store_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::store_u64("x20", abi::stack_pointer(), 8));
    asm.push(abi::store_u64("x21", abi::stack_pointer(), 16));
    asm.push(abi::store_u64("x22", abi::stack_pointer(), 24));
    asm.push(abi::store_u64("x23", abi::stack_pointer(), 32));

    // Gate: already off -> no-op (plan §4.2). x19 is the pinned arena-state base.
    asm.push(abi::load_u64(
        "x9",
        TERM_ARENA_STATE_REG,
        term_state_offset + code::TERM_STATE_ACTIVE_OFFSET,
    ));
    asm.push(abi::compare_immediate("x9", "0"));
    asm.push(abi::branch_eq("term_off_done"));

    // app = [NSApplication sharedApplication]
    asm.external_data("x20", CLASS_NS_APPLICATION, LIB_APPKIT);
    asm.load_selector(SEL_SHARED_APPLICATION.0);
    asm.push(abi::move_register("x0", "x20"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::move_register("x20", "x0")); // app

    // window = objc_getAssociatedObject(app, &WINDOW_ASSOC_KEY); nil -> headless.
    asm.push(abi::move_register("x0", "x20"));
    asm.local_address("x1", WINDOW_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register("x21", "x0")); // window or nil
    asm.push(abi::compare_immediate("x21", "0"));
    asm.push(abi::branch_eq("term_off_inactive"));

    // scroll = objc_getAssociatedObject(app, &SCROLLVIEW_ASSOC_KEY)
    asm.push(abi::move_register("x0", "x20"));
    asm.local_address("x1", SCROLLVIEW_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register("x22", "x0")); // scroll view

    // [window performSelectorOnMainThread:@selector(setContentView:)
    //         withObject:scrollView waitUntilDone:YES]
    asm.load_selector(SEL_SET_CONTENT_VIEW.0);
    asm.push(abi::move_register("x23", "x1"));
    asm.load_selector(SEL_PERFORM_ON_MAIN.0);
    asm.push(abi::move_register("x2", "x23"));
    asm.push(abi::move_register("x3", "x22"));
    asm.push(abi::move_immediate("x4", "Integer", "1")); // waitUntilDone: YES
    asm.push(abi::move_register("x0", "x21"));
    asm.call_external("_objc_msgSend", LIB_OBJC);

    asm.push(abi::label("term_off_inactive"));
    store_term_state(&mut asm, term_state_offset, code::TERM_STATE_ACTIVE_OFFSET, "0");

    asm.push(abi::label("term_off_done"));
    asm.push(abi::move_immediate("x0", "Integer", "0")); // RESULT_OK_TAG
    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::load_u64("x20", abi::stack_pointer(), 8));
    asm.push(abi::load_u64("x21", abi::stack_pointer(), 16));
    asm.push(abi::load_u64("x22", abi::stack_pointer(), 24));
    asm.push(abi::load_u64("x23", abi::stack_pointer(), 32));
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
) -> Option<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>)> {
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
        "term.clear" => emit_app_clear(symbol, term_state_offset),
        "term.showCursor" => emit_app_set_cursor_visible(symbol, term_state_offset, "1"),
        "term.hideCursor" => emit_app_set_cursor_visible(symbol, term_state_offset, "0"),
        "term.terminalSize" => emit_app_terminal_size(symbol, term_state_offset),
        _ => return None,
    };
    Some(helper)
}

/// Branch to `done` when TUI mode is inactive (the §4.2.1 no-op gate). `x19` is
/// the pinned arena-state base holding the term-state global.
fn emit_term_active_gate(asm: &mut Asm, term_state_offset: usize, done: &str) {
    asm.push(abi::load_u64(
        "x9",
        TERM_ARENA_STATE_REG,
        term_state_offset + code::TERM_STATE_ACTIVE_OFFSET,
    ));
    asm.push(abi::compare_immediate("x9", "0"));
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
) -> (CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>) {
    let mut asm = Asm::new(symbol);
    // Frame: lr@0, x20(state)@8, r/g/b (then packed) @16/@24/@32.
    let frame = 48;
    let done = format!("{symbol}_done");
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(frame));
    asm.push(abi::store_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::store_u64("x20", abi::stack_pointer(), 8));
    asm.push(abi::store_u64("x0", abi::stack_pointer(), 16)); // r
    asm.push(abi::store_u64("x1", abi::stack_pointer(), 24)); // g
    asm.push(abi::store_u64("x2", abi::stack_pointer(), 32)); // b
    emit_term_active_gate(&mut asm, term_state_offset, &done);
    // packed = r | g<<8 | b<<16
    asm.push(abi::load_u64("x9", abi::stack_pointer(), 16));
    asm.push(abi::load_u64("x10", abi::stack_pointer(), 24));
    asm.push(abi::load_u64("x11", abi::stack_pointer(), 32));
    asm.push(abi::shift_left_immediate("x10", "x10", 8));
    asm.push(abi::shift_left_immediate("x11", "x11", 16));
    asm.push(abi::or_registers("x9", "x9", "x10"));
    asm.push(abi::or_registers("x9", "x9", "x11"));
    asm.push(abi::store_u64(
        "x9",
        TERM_ARENA_STATE_REG,
        term_state_offset + global_field,
    ));
    asm.push(abi::store_u64("x9", abi::stack_pointer(), 16)); // keep packed across calls
    emit_get_tv_state(&mut asm, "x20", &done);
    asm.push(abi::load_u64("x9", abi::stack_pointer(), 16));
    asm.push(abi::store_u64("x9", "x20", tv_field));
    asm.push(abi::label(&done));
    emit_term_ok_return(&mut asm, frame, &[("x20", 8)]);
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
) -> (CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>) {
    let mut asm = Asm::new(symbol);
    let frame = 32; // lr@0, x20(state)@8, enabled@16
    let done = format!("{symbol}_done");
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(frame));
    asm.push(abi::store_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::store_u64("x20", abi::stack_pointer(), 8));
    asm.push(abi::store_u64("x0", abi::stack_pointer(), 16)); // enabled
    emit_term_active_gate(&mut asm, term_state_offset, &done);
    asm.push(abi::load_u64("x9", abi::stack_pointer(), 16));
    asm.push(abi::store_u64(
        "x9",
        TERM_ARENA_STATE_REG,
        term_state_offset + global_field,
    ));
    emit_get_tv_state(&mut asm, "x20", &done);
    asm.push(abi::load_u64("x9", abi::stack_pointer(), 16));
    asm.push(abi::store_u64("x9", "x20", tv_field));
    asm.push(abi::label(&done));
    emit_term_ok_return(&mut asm, frame, &[("x20", 8)]);
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
fn emit_app_move_to(
    symbol: &str,
    term_state_offset: usize,
) -> (CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>) {
    let mut asm = Asm::new(symbol);
    let frame = 32; // lr@0, x20(state)@8, row@16, col@24
    let done = format!("{symbol}_done");
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(frame));
    asm.push(abi::store_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::store_u64("x20", abi::stack_pointer(), 8));
    asm.push(abi::store_u64("x0", abi::stack_pointer(), 16)); // row
    asm.push(abi::store_u64("x1", abi::stack_pointer(), 24)); // col
    emit_term_active_gate(&mut asm, term_state_offset, &done);
    emit_get_tv_state(&mut asm, "x20", &done);
    // row = clamp(row, 0, rows-1)
    asm.push(abi::load_u64("x9", abi::stack_pointer(), 16));
    let row_lo = format!("{symbol}_row_lo");
    let row_hi = format!("{symbol}_row_hi");
    asm.push(abi::compare_immediate("x9", "0"));
    asm.push(abi::branch_ge(&row_lo));
    asm.push(abi::move_immediate("x9", "Integer", "0"));
    asm.push(abi::label(&row_lo));
    asm.push(abi::load_u64("x10", "x20", TV_ROWS_OFFSET));
    asm.push(abi::subtract_immediate("x10", "x10", 1));
    asm.push(abi::compare_registers("x9", "x10"));
    asm.push(abi::branch_le(&row_hi));
    asm.push(abi::move_register("x9", "x10"));
    asm.push(abi::label(&row_hi));
    asm.push(abi::store_u64("x9", "x20", TV_CURSOR_ROW_OFFSET));
    // col = clamp(col, 0, cols-1)
    asm.push(abi::load_u64("x9", abi::stack_pointer(), 24));
    let col_lo = format!("{symbol}_col_lo");
    let col_hi = format!("{symbol}_col_hi");
    asm.push(abi::compare_immediate("x9", "0"));
    asm.push(abi::branch_ge(&col_lo));
    asm.push(abi::move_immediate("x9", "Integer", "0"));
    asm.push(abi::label(&col_lo));
    asm.push(abi::load_u64("x10", "x20", TV_COLS_OFFSET));
    asm.push(abi::subtract_immediate("x10", "x10", 1));
    asm.push(abi::compare_registers("x9", "x10"));
    asm.push(abi::branch_le(&col_hi));
    asm.push(abi::move_register("x9", "x10"));
    asm.push(abi::label(&col_hi));
    asm.push(abi::store_u64("x9", "x20", TV_CURSOR_COL_OFFSET));
    asm.push(abi::label(&done));
    emit_term_ok_return(&mut asm, frame, &[("x20", 8)]);
    (
        CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        asm.ins,
        asm.rel,
    )
}

/// `term::clear` app body: clear the grid + home the cursor (worker side), then
/// trigger a redraw on the main thread.
fn emit_app_clear(
    symbol: &str,
    term_state_offset: usize,
) -> (CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>) {
    let mut asm = Asm::new(symbol);
    let frame = 32; // lr@0, x20(termView)@8, x21(sel)@16
    let done = format!("{symbol}_done");
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(frame));
    asm.push(abi::store_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::store_u64("x20", abi::stack_pointer(), 8));
    asm.push(abi::store_u64("x21", abi::stack_pointer(), 16));
    emit_term_active_gate(&mut asm, term_state_offset, &done);
    // tv = objc_getAssociatedObject([NSApplication sharedApplication], &TERMVIEW_KEY)
    asm.external_data("x20", CLASS_NS_APPLICATION, LIB_APPKIT);
    asm.load_selector(SEL_SHARED_APPLICATION.0);
    asm.push(abi::move_register("x0", "x20"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.local_address("x1", TERMVIEW_ASSOC_KEY);
    asm.call_external("_objc_getAssociatedObject", LIB_OBJC);
    asm.push(abi::move_register("x20", "x0")); // termView or nil
    asm.push(abi::compare_immediate("x20", "0"));
    asm.push(abi::branch_eq(&done));
    // clear the grid + cursor (our heap, worker-safe)
    asm.push(abi::move_register("x0", "x20"));
    asm.call_internal(TERM_CLEAR_SYMBOL);
    // [tv performSelectorOnMainThread:@selector(setNeedsDisplay:) withObject:tv waitUntilDone:YES]
    // (any non-nil withObject reads as BOOL YES).
    asm.load_selector(SEL_SET_NEEDS_DISPLAY.0);
    asm.push(abi::move_register("x21", "x1"));
    asm.load_selector(SEL_PERFORM_ON_MAIN.0);
    asm.push(abi::move_register("x2", "x21"));
    asm.push(abi::move_register("x3", "x20"));
    asm.push(abi::move_immediate("x4", "Integer", "1"));
    asm.push(abi::move_register("x0", "x20"));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::label(&done));
    emit_term_ok_return(&mut asm, frame, &[("x20", 8), ("x21", 16)]);
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
fn emit_app_set_cursor_visible(
    symbol: &str,
    term_state_offset: usize,
    value: &str,
) -> (CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>) {
    let mut asm = Asm::new(symbol);
    let frame = 16; // lr@0, x20(state)@8
    let done = format!("{symbol}_done");
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(frame));
    asm.push(abi::store_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::store_u64("x20", abi::stack_pointer(), 8));
    emit_term_active_gate(&mut asm, term_state_offset, &done);
    asm.push(abi::move_immediate("x9", "Integer", value));
    asm.push(abi::store_u64(
        "x9",
        TERM_ARENA_STATE_REG,
        term_state_offset + code::TERM_STATE_CURSOR_VISIBLE_OFFSET,
    ));
    emit_get_tv_state(&mut asm, "x20", &done);
    asm.push(abi::move_immediate("x9", "Integer", value));
    asm.push(abi::store_u64("x9", "x20", TV_CURSOR_VISIBLE_OFFSET));
    asm.push(abi::label(&done));
    emit_term_ok_return(&mut asm, frame, &[("x20", 8)]);
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
fn emit_app_terminal_size(
    symbol: &str,
    term_state_offset: usize,
) -> (CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>) {
    let mut asm = Asm::new(symbol);
    // Frame: lr@0, x20(state)@8, columns@16, rows@24.
    let frame = 48;
    let unsupported = format!("{symbol}_unsupported");
    let done = format!("{symbol}_done");
    asm.push(abi::label("entry"));
    asm.push(abi::subtract_stack(frame));
    asm.push(abi::store_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::store_u64("x20", abi::stack_pointer(), 8));
    // Requires active TUI mode (plan §4.7).
    asm.push(abi::load_u64(
        "x9",
        TERM_ARENA_STATE_REG,
        term_state_offset + code::TERM_STATE_ACTIVE_OFFSET,
    ));
    asm.push(abi::compare_immediate("x9", "0"));
    asm.push(abi::branch_eq(&unsupported));
    emit_get_tv_state(&mut asm, "x20", &unsupported);
    asm.push(abi::load_u64("x10", "x20", TV_COLS_OFFSET));
    asm.push(abi::load_u64("x11", "x20", TV_ROWS_OFFSET));
    asm.push(abi::store_u64("x10", abi::stack_pointer(), 16));
    asm.push(abi::store_u64("x11", abi::stack_pointer(), 24));
    // record = arena_alloc(16, 8); columns@0, rows@8.
    asm.push(abi::move_immediate("x0", "Integer", "16"));
    asm.push(abi::move_immediate("x1", "Integer", "8"));
    asm.call_internal(ARENA_ALLOC_SYMBOL);
    asm.push(abi::compare_immediate("x0", "0")); // RESULT_OK_TAG
    asm.push(abi::branch_ne(&unsupported));
    asm.push(abi::load_u64("x10", abi::stack_pointer(), 16));
    asm.push(abi::load_u64("x11", abi::stack_pointer(), 24));
    asm.push(abi::store_u64("x10", "x1", 0)); // columns
    asm.push(abi::store_u64("x11", "x1", 8)); // rows
    asm.push(abi::move_immediate("x0", "Integer", "0")); // OK; x1 = record
    asm.push(abi::branch(&done));
    asm.push(abi::label(&unsupported));
    asm.push(abi::move_immediate("x1", "Integer", ERR_UNSUPPORTED_CODE));
    asm.push(abi::move_immediate("x0", "Integer", "1")); // ERR tag
    asm.push(
        CodeInstruction::new("adrp")
            .field("dst", "x2")
            .field("symbol", ERR_UNSUPPORTED_SYMBOL),
    );
    asm.push(
        CodeInstruction::new("add_pageoff")
            .field("dst", "x2")
            .field("src", "x2")
            .field("symbol", ERR_UNSUPPORTED_SYMBOL),
    );
    for kind in ["page21", "pageoff12"] {
        asm.rel.push(CodeRelocation {
            from: symbol.to_string(),
            to: ERR_UNSUPPORTED_SYMBOL.to_string(),
            kind: kind.to_string(),
            binding: "data".to_string(),
            library: None,
        });
    }
    asm.push(abi::label(&done));
    asm.push(abi::load_u64(abi::link_register(), abi::stack_pointer(), 0));
    asm.push(abi::load_u64("x20", abi::stack_pointer(), 8));
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

/// Build `[NSString stringWithUTF8String:<cstr>]` into `x0`. `class_tmp` is a
/// callee-saved scratch register (free at the call site) used for the class.
fn build_nsstring_from_cstring(asm: &mut Asm, class_tmp: &str, cstr_symbol: &str) {
    asm.external_data(class_tmp, CLASS_NS_STRING, LIB_FOUNDATION);
    asm.load_selector(SEL_STRING_WITH_UTF8.0);
    asm.local_address("x2", cstr_symbol);
    asm.push(abi::move_register("x0", class_tmp));
    asm.call_external("_objc_msgSend", LIB_OBJC);
}

/// Materialize a small non-negative integer as a double in `dst` (an FP
/// register): `movz` the value into a scratch GPR, then `scvtf`.
fn emit_double_immediate(asm: &mut Asm, dst: &str, value: u32) {
    asm.push(abi::move_immediate("x9", "Integer", &value.to_string()));
    asm.push(abi::signed_convert_to_float_d(dst, "x9"));
}

/// Read-only C-string data objects (selectors, window title, env-var name) the
/// bootstrap references. NUL-terminated raw bytes, mirroring the TLS helpers.
pub(crate) fn app_mode_data_objects() -> Vec<CodeDataObject> {
    let mut objects: Vec<CodeDataObject> = [
        SEL_SHARED_APPLICATION,
        SEL_SET_ACTIVATION_POLICY,
        SEL_ALLOC,
        SEL_INIT_WINDOW,
        SEL_STRING_WITH_UTF8,
        SEL_SET_TITLE,
        SEL_MAKE_KEY_AND_ORDER_FRONT,
        SEL_ACTIVATE,
        SEL_RUN,
        STR_TITLE,
        STR_HEADLESS_ENV,
        // Phase 4 transcript output.
        SEL_CONTENT_VIEW,
        SEL_INIT_FRAME,
        SEL_SET_EDITABLE,
        SEL_SET_SELECTABLE,
        SEL_SET_DOCUMENT_VIEW,
        SEL_SET_HAS_VSCROLLER,
        SEL_SET_AUTORESIZING_MASK,
        SEL_ADD_SUBVIEW,
        SEL_TEXT_STORAGE,
        SEL_APPEND_ATTRIBUTED,
        SEL_DICTIONARY_WITH_OBJECT,
        SEL_INIT_WITH_STRING_ATTRS,
        SEL_PERFORM_ON_MAIN,
        SEL_INIT_WITH_BYTES,
        STR_STDERR_PREFIX,
        STR_NEWLINE,
        // Phase 4a shutdown / app delegate.
        SEL_INIT,
        SEL_SET_DELEGATE,
        SEL_APP_SHOULD_TERMINATE,
        SEL_APP_DID_FINISH_LAUNCHING,
        STR_DELEGATE_CLASS,
        STR_DELEGATE_TYPES,
        STR_EXIT_PREFIX,
        // Monospaced font + application menu.
        SEL_USER_FIXED_FONT,
        SEL_SET_FONT,
        SEL_ADD_ITEM,
        SEL_SET_ACTION,
        SEL_SET_KEY_EQUIVALENT,
        SEL_SET_SUBMENU,
        SEL_SET_MAIN_MENU,
        SEL_TERMINATE,
        STR_QUIT,
        STR_QUIT_KEY,
        // Terminal-style input (keyDown: on the transcript view).
        SEL_UTF8_STRING,
        SEL_MAKE_FIRST_RESPONDER,
        SEL_KEY_DOWN,
        SEL_CHARACTERS,
        SEL_LENGTH,
        SEL_CHAR_AT_INDEX,
        SEL_APPEND_STRING,
        SEL_SET_STRING,
        SEL_DELETE_RANGE,
        SEL_STRING,
        STR_TEXTVIEW_CLASS,
        STR_INPUT_TYPES,
        STR_EMPTY,
        // Terminal size (io.terminalSize).
        SEL_ENCLOSING_SCROLL_VIEW,
        SEL_CONTENT_SIZE,
        SEL_MAX_ADVANCEMENT,
        SEL_LAYOUT_MANAGER,
        SEL_DEFAULT_LINE_HEIGHT,
        // term:: synthesized TermView surface (plan-01-term.md §6.3, Phase 4-5).
        SEL_DRAW_RECT,
        SEL_IS_FLIPPED,
        SEL_SET_CONTENT_VIEW,
        SEL_COLOR_WITH_RGBA,
        SEL_SET,
        SEL_WHITE_COLOR,
        SEL_BLACK_COLOR,
        SEL_DRAW_AT_POINT,
        SEL_STRING_WITH_CHARS,
        SEL_DICTIONARY,
        SEL_SET_OBJECT_FOR_KEY,
        SEL_SET_NEEDS_DISPLAY,
        SEL_MFB_WRITE_STRING,
        STR_TERMVIEW_CLASS_NAME,
        STR_DRAW_RECT_TYPES,
        STR_IS_FLIPPED_TYPES,
        STR_WRITE_STRING_TYPES,
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
    // The transcript NSTextView is stored as an associated object on NSApp keyed
    // by the address of this 1-byte read-only symbol (objc-runtime-managed
    // per-process storage; see ASSOC_KEY).
    for key in [
        ASSOC_KEY,
        PIPE_ASSOC_KEY,
        INPUT_LINE_KEY,
        INPUT_MODE_KEY,
        WINDOW_ASSOC_KEY,
        SCROLLVIEW_ASSOC_KEY,
        TERMVIEW_ASSOC_KEY,
        TVSTATE_ASSOC_KEY,
        ARG_ASSOC_KEY,
    ] {
        objects.push(CodeDataObject {
            symbol: key.to_string(),
            kind: "raw".to_string(),
            layout: "associated-object key (unique address)".to_string(),
            align: 1,
            size: 1,
            value: "00".to_string(),
        });
    }
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
