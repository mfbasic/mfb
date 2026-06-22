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

const CLASS_NS_APPLICATION: &str = "_OBJC_CLASS_$_NSApplication";
const CLASS_NS_WINDOW: &str = "_OBJC_CLASS_$_NSWindow";
const CLASS_NS_STRING: &str = "_OBJC_CLASS_$_NSString";

const LIB_OBJC: &str = "libobjc";
const LIB_APPKIT: &str = "AppKit";
const LIB_FOUNDATION: &str = "Foundation";
const LIB_SYSTEM: &str = "libSystem";

/// Persistent (callee-saved) registers held across the external calls in `_main`.
const REG_APP: &str = "x19"; // NSApplication instance
const REG_WINDOW: &str = "x20"; // NSWindow instance
const REG_SCRATCH_OBJ: &str = "x21"; // transient object (class / NSString)
const REG_HEADLESS: &str = "x22"; // getenv("MFB_MACAPP_HEADLESS") result

// `_main` stack frame: [sp+0]=argc, [sp+8]=argv (worker arg block), [sp+16]=pthread_t.
const FRAME_SIZE: usize = 32;
const OFF_ARGC: usize = 0;
const OFF_ARGV: usize = 8;
const OFF_TID: usize = 16;

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
    Ok(vec![emit_main_bootstrap(), emit_worker_shim(spec)])
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

    // In GUI mode, show + activate the window. Headless test mode skips this.
    asm.push(abi::compare_immediate(REG_HEADLESS, "0"));
    asm.push(abi::branch_ne("after_show"));
    asm.load_selector(SEL_MAKE_KEY_AND_ORDER_FRONT.0);
    asm.push(abi::move_immediate("x2", "Integer", "0"));
    asm.push(abi::move_register("x0", REG_WINDOW));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.load_selector(SEL_ACTIVATE.0);
    asm.push(abi::move_immediate("x2", "Integer", "1")); // ignoreOtherApps: YES
    asm.push(abi::move_register("x0", REG_APP));
    asm.call_external("_objc_msgSend", LIB_OBJC);
    asm.push(abi::label("after_show"));

    // pthread_create(&tid, NULL, _mfb_macapp_worker, &argblock)
    asm.push(abi::add_immediate("x0", abi::stack_pointer(), OFF_TID));
    asm.push(abi::move_immediate("x1", "Integer", "0"));
    asm.local_address("x2", WORKER_SYMBOL);
    asm.push(abi::add_immediate("x3", abi::stack_pointer(), OFF_ARGC));
    asm.call_external("_pthread_create", LIB_SYSTEM);

    // Headless: spin while the worker runs the program and exits the process.
    asm.push(abi::compare_immediate(REG_HEADLESS, "0"));
    asm.push(abi::branch_eq("run_event_loop"));
    asm.push(abi::label("spin"));
    asm.push(abi::branch_self());

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

/// `void *_mfb_macapp_worker(void *arg)` pthread start routine: unpacks the
/// `{argc, argv}` block (when the entry accepts args) into `x0`/`x1`, then tail
/// calls the standard program entry, which never returns (it ends in `_exit`).
fn emit_worker_shim(spec: &AppEntrySpec) -> CodeFunction {
    let mut asm = Asm::new(WORKER_SYMBOL);
    asm.push(abi::label("entry"));
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

/// Materialize a small non-negative integer as a double in `dst` (an FP
/// register): `movz` the value into a scratch GPR, then `scvtf`.
fn emit_double_immediate(asm: &mut Asm, dst: &str, value: u32) {
    asm.push(abi::move_immediate("x9", "Integer", &value.to_string()));
    asm.push(abi::signed_convert_to_float_d(dst, "x9"));
}

/// Read-only C-string data objects (selectors, window title, env-var name) the
/// bootstrap references. NUL-terminated raw bytes, mirroring the TLS helpers.
pub(crate) fn app_mode_data_objects() -> Vec<CodeDataObject> {
    [
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
    .collect()
}

fn hex_cstring(text: &str) -> String {
    let mut hex = String::new();
    for byte in text.bytes() {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex.push_str("00");
    hex
}
