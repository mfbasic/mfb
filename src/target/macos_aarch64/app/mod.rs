//! macOS app-mode (`mfb build -app`) runtime bootstrap codegen.
//!
//! Implements the macOS app-mode runtime (see src/spec/app/01_macos-runtime.md): emit the app-mode `_main`
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
use crate::target::shared::code::{
    self, AppEntrySpec, CodeDataObject, CodeFrame, CodeFunction, CodeInstruction, CodeRelocation, RelocIntent,
};

const MAIN_SYMBOL: &str = "_main";
const WORKER_SYMBOL: &str = "_mfb_macapp_worker";

/// NSApplicationActivationPolicyRegular.
const ACTIVATION_POLICY_REGULAR: &str = "0";
/// NSWindowStyleMask: Titled(1) | Closable(2) | Miniaturizable(4) | Resizable(8).
const WINDOW_STYLE_MASK: &str = "15";
/// NSBackingStoreBuffered.
const BACKING_BUFFERED: &str = "2";

// Read-only C-string data symbols referenced by the bootstrap.
const SEL_SHARED_APPLICATION: (&str, &str) =
    ("_mfb_macapp_sel_sharedApplication", "sharedApplication");
const SEL_SET_ACTIVATION_POLICY: (&str, &str) = (
    "_mfb_macapp_sel_setActivationPolicy",
    "setActivationPolicy:",
);
const SEL_ALLOC: (&str, &str) = ("_mfb_macapp_sel_alloc", "alloc");
const SEL_INIT_WINDOW: (&str, &str) = (
    "_mfb_macapp_sel_initWindow",
    "initWithContentRect:styleMask:backing:defer:",
);
const SEL_STRING_WITH_UTF8: (&str, &str) = (
    "_mfb_macapp_sel_stringWithUTF8String",
    "stringWithUTF8String:",
);
const SEL_SET_TITLE: (&str, &str) = ("_mfb_macapp_sel_setTitle", "setTitle:");
const SEL_MAKE_KEY_AND_ORDER_FRONT: (&str, &str) = (
    "_mfb_macapp_sel_makeKeyAndOrderFront",
    "makeKeyAndOrderFront:",
);
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
const SEL_SET_DOCUMENT_VIEW: (&str, &str) = ("_mfb_macapp_sel_setDocumentView", "setDocumentView:");
const SEL_SET_HAS_VSCROLLER: (&str, &str) = (
    "_mfb_macapp_sel_setHasVerticalScroller",
    "setHasVerticalScroller:",
);
const SEL_SET_AUTORESIZING_MASK: (&str, &str) = (
    "_mfb_macapp_sel_setAutoresizingMask",
    "setAutoresizingMask:",
);
const SEL_ADD_SUBVIEW: (&str, &str) = ("_mfb_macapp_sel_addSubview", "addSubview:");

/// NSViewWidthSizable(2) | NSViewHeightSizable(16): the scroll view tracks the
/// window's content view on resize.
const AUTORESIZE_WIDTH_HEIGHT: &str = "18";
/// NSViewWidthSizable(2): the transcript text view widens with the scroll view.
const AUTORESIZE_WIDTH: &str = "2";
// Transcript append selectors.
const SEL_TEXT_STORAGE: (&str, &str) = ("_mfb_macapp_sel_textStorage", "textStorage");
const SEL_APPEND_ATTRIBUTED: (&str, &str) = (
    "_mfb_macapp_sel_appendAttributed",
    "appendAttributedString:",
);
const SEL_DICTIONARY_WITH_OBJECT: (&str, &str) = (
    "_mfb_macapp_sel_dictWithObject",
    "dictionaryWithObject:forKey:",
);
const SEL_INIT_WITH_STRING_ATTRS: (&str, &str) = (
    "_mfb_macapp_sel_initWithStringAttrs",
    "initWithString:attributes:",
);
const SEL_PERFORM_ON_MAIN: (&str, &str) = (
    "_mfb_macapp_sel_performOnMain",
    "performSelectorOnMainThread:withObject:waitUntilDone:",
);
const SEL_INIT_WITH_BYTES: (&str, &str) = (
    "_mfb_macapp_sel_initWithBytes",
    "initWithBytes:length:encoding:",
);
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
const STR_EXIT_PREFIX: (&str, &str) = ("_mfb_macapp_str_exitPrefix", "\nProgram exited with code ");

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
const SEL_CHAR_AT_INDEX: (&str, &str) = ("_mfb_macapp_sel_characterAtIndex", "characterAtIndex:");
const SEL_APPEND_STRING: (&str, &str) = ("_mfb_macapp_sel_appendString", "appendString:");
const SEL_SET_STRING: (&str, &str) = ("_mfb_macapp_sel_setString", "setString:");
const SEL_DELETE_RANGE: (&str, &str) = (
    "_mfb_macapp_sel_deleteCharsInRange",
    "deleteCharactersInRange:",
);
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
const SEL_DEFAULT_LINE_HEIGHT: (&str, &str) = (
    "_mfb_macapp_sel_defaultLineHeight",
    "defaultLineHeightForFont:",
);
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
const SEL_SET_CONTENT_VIEW: (&str, &str) = ("_mfb_macapp_sel_setContentView", "setContentView:");
const SEL_COLOR_WITH_RGBA: (&str, &str) = (
    "_mfb_macapp_sel_colorWithRGBA",
    "colorWithCalibratedRed:green:blue:alpha:",
);
const SEL_SET: (&str, &str) = ("_mfb_macapp_sel_set", "set");
const SEL_WHITE_COLOR: (&str, &str) = ("_mfb_macapp_sel_whiteColor", "whiteColor");
const SEL_BLACK_COLOR: (&str, &str) = ("_mfb_macapp_sel_blackColor", "blackColor");
const SEL_DRAW_AT_POINT: (&str, &str) =
    ("_mfb_macapp_sel_drawAtPoint", "drawAtPoint:withAttributes:");
const SEL_STRING_WITH_CHARS: (&str, &str) = (
    "_mfb_macapp_sel_stringWithChars",
    "stringWithCharacters:length:",
);
const SEL_DICTIONARY: (&str, &str) = ("_mfb_macapp_sel_dictionary", "dictionary");
const SEL_SET_OBJECT_FOR_KEY: (&str, &str) =
    ("_mfb_macapp_sel_setObjectForKey", "setObject:forKey:");
const SEL_REMOVE_OBJECT_FOR_KEY: (&str, &str) =
    ("_mfb_macapp_sel_removeObjectForKey", "removeObjectForKey:");
const SEL_NUMBER_WITH_INT: (&str, &str) = ("_mfb_macapp_sel_numberWithInt", "numberWithInt:");
const SEL_NUMBER_WITH_DOUBLE: (&str, &str) =
    ("_mfb_macapp_sel_numberWithDouble", "numberWithDouble:");
/// `NSUnderlineStyleAttributeName` (value `@(NSUnderlineStyleSingle)`) and
/// `NSStrokeWidthAttributeName` (a negative value = faux-bold fill stroke) — the
/// attributed-string keys used to render per-cell underline/bold (plan §4.3).
const NS_UNDERLINE_STYLE_ATTRIBUTE_NAME: &str = "_NSUnderlineStyleAttributeName";
const NS_STROKE_WIDTH_ATTRIBUTE_NAME: &str = "_NSStrokeWidthAttributeName";
const SEL_SET_NEEDS_DISPLAY: (&str, &str) = ("_mfb_macapp_sel_setNeedsDisplay", "setNeedsDisplay:");
const SEL_MFB_WRITE_STRING: (&str, &str) = ("_mfb_macapp_sel_mfbWriteString", "mfbWriteString:");
/// `NSForegroundColorAttributeName` — attributed-string key for the glyph colour.
const NS_FOREGROUND_COLOR_ATTRIBUTE_NAME: &str = "_NSForegroundColorAttributeName";
/// IMP for the TermView `mfbWriteString:` main-thread write entry point.
const MFB_WRITE_STRING_SYMBOL: &str = "_mfb_macapp_term_writeString";
/// `acceptsFirstResponder` — TermView must return YES so it can become the
/// window's first responder and receive `keyDown:` while TUI mode is active.
const SEL_ACCEPTS_FIRST_RESPONDER: (&str, &str) = (
    "_mfb_macapp_sel_acceptsFirstResponder",
    "acceptsFirstResponder",
);
/// IMP for TermView `acceptsFirstResponder` (returns YES).
const TERM_ACCEPTS_FR_SYMBOL: &str = "_mfb_macapp_term_acceptsFR";
/// IMP for TermView `keyDown:` (routes typed keys to the window input pipe).
const TERM_KEY_DOWN_SYMBOL: &str = "_mfb_macapp_term_keyDown";
/// Obj-C method type encoding for `void (id, SEL, id)`.
const STR_WRITE_STRING_TYPES: (&str, &str) = ("_mfb_macapp_str_writeStringTypes", "v@:@");
/// Class names for the synthesized surface and the AppKit drawing primitives it
/// uses.
const CLASS_NS_VIEW: &str = "_OBJC_CLASS_$_NSView";
const CLASS_NS_COLOR: &str = "_OBJC_CLASS_$_NSColor";
const CLASS_NS_MUTABLE_DICTIONARY: &str = "_OBJC_CLASS_$_NSMutableDictionary";
const CLASS_NS_NUMBER: &str = "_OBJC_CLASS_$_NSNumber";
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
        for kind in [RelocIntent::GotLoadHi, RelocIntent::GotLoadLo] {
            self.rel.push(CodeRelocation {
                from: self.from.clone(),
                to: symbol.to_string(),
                kind,
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
        emit_finish_helper(spec.uses_term),
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
        emit_term_accepts_first_responder(),
        emit_term_key_down_helper(),
    ])
}


mod app_io;
mod bootstrap;
mod term_view;

pub(crate) use app_io::*;
use bootstrap::*;
use term_view::*;

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
        SEL_REMOVE_OBJECT_FOR_KEY,
        SEL_NUMBER_WITH_INT,
        SEL_NUMBER_WITH_DOUBLE,
        SEL_SET_NEEDS_DISPLAY,
        SEL_MFB_WRITE_STRING,
        SEL_ACCEPTS_FIRST_RESPONDER,
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
