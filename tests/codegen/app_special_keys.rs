//! bug-699: every `--app` key handler injects a non-character key in ONE
//! encoding — the kitty keyboard protocol form — so one decoder serves every
//! surface.
//!
//! Today each handler forwards "the characters of this key" as the OS reports
//! them, and the OSes disagree: AppKit reports Left as the private-use code point
//! U+F702 (written verbatim, `EF 9C 82`), GTK's `gdk_keyval_to_unicode` answers
//! 0 for Left and the handler drops it, and Windows only ever looks at
//! `WM_CHAR`, which the arrow keys never produce. No handler reports a release.
//!
//! # How these tests observe a handler
//!
//! They RUN it. Each test builds a small `--app` program for the target with
//! `mfb build -ncode` (execution-free, so every target builds on any host),
//! finds the handler the program registers, and executes it in the `.ncode`
//! interpreter (`tests/common/ncode_emu.rs`) with a platform model standing in
//! for AppKit, GTK or Win32: the model hands the handler the event a real key
//! press delivers (the same `NSEvent` fields, keyval, or window message) and
//! records the bytes written to the window input pipe.
//!
//! That makes the assertion about behavior — "Left injects `ESC [ D`" — and not
//! about which instructions produce it, so any correct Phase 3 implementation
//! passes. If a Phase 3 handler calls a platform API the model does not answer,
//! the run panics naming the call: extend the MODEL to answer it the way the real
//! API does. Never change an EXPECTED byte sequence to match a handler.
//!
//! # The expected bytes
//!
//! Exactly what a kitty-protocol terminal sends with the progressive-enhancement
//! flags `term::on` pushes (bug-699 Goal: disambiguate `0b1`, report event types
//! `0b10`, report all keys as escape codes `0b1000`), per the protocol spec
//! (<https://sw.kovidgoyal.net/kitty/keyboard-protocol/>, "Legacy functional
//! keys", "Event types", "Modifiers"):
//!
//! | Event | Bytes |
//! | --- | --- |
//! | Left press | `CSI D` — the `1;modifier` part is omitted when there are no modifiers |
//! | Forward Delete press | `CSI 3 ~` |
//! | F5 press | `CSI 15 ~` |
//! | Left release | `CSI 1 ; 1 : 3 D` — no modifiers is `1`, release is event type `3` |
//! | Shift+Left press | `CSI 1 ; 2 D` — modifier value is `1 + shift(1)` |
//!
//! Injecting what a kitty terminal sends is the point: a console under
//! `term::on` and every `--app` surface then reach the decoder in the same form.
//!
//! These cases fail today for the reasons above and are `#[ignore]`d until
//! bug-699 Phase 3 (the handler rewrite); each has a positive control that runs
//! the SAME handler through the SAME model with a plain letter and must pass
//! now, so a red case means the handler, not the harness.

#[path = "../common/mod.rs"]
mod common;

use common::ncode_emu::{data_cstr, find_calls, CallSite, Externals, Machine, Sym};
use common::{build_ncode_app, temp_project};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// A program that reaches every surface's key handler: a canvas, the TUI
/// (`term::on`), and raw reads on the transcript. Used for GTK and Windows,
/// whose handlers are installed whatever the start mode.
const PROGRAM: &str = "\
IMPORT app
IMPORT canvas
IMPORT color
IMPORT io
IMPORT term

FUNC main AS Integer
  app::setMode(app::Mode.Canvas)
  canvas::present([canvas::Rectangle[x := 0.0, y := 0.0, w := 200.0, h := 200.0, paint := canvas::fill(color::rgb(20, 40, 80))]])
  LET b AS Byte = io::readByte()
  app::setMode(app::Mode.Console)
  term::on()
  LET c AS Byte = io::readByte()
  term::off()
  io::print(toString(b) & toString(c))
  RETURN 0
END FUNC
";

/// macOS installs the transcript's and the TUI's `keyDown:` only in a program
/// that STARTS in `Console` (a program that calls `app::setMode` starts
/// windowless and never registers them — see the bug-699 doc, "New bug found"),
/// so the macOS surfaces come from two programs: this one for the transcript and
/// the TUI, [`PROGRAM`] for the canvas.
const CONSOLE_PROGRAM: &str = "\
IMPORT io
IMPORT term

FUNC main AS Integer
  LET b AS Byte = io::readByte()
  term::on()
  LET c AS Byte = io::readByte()
  term::off()
  io::print(toString(b) & toString(c))
  RETURN 0
END FUNC
";

const LEFT_PRESS: &[u8] = b"\x1b[D";
const DELETE_PRESS: &[u8] = b"\x1b[3~";
const F5_PRESS: &[u8] = b"\x1b[15~";
const LEFT_RELEASE: &[u8] = b"\x1b[1;1:3D";
const SHIFT_LEFT_PRESS: &[u8] = b"\x1b[1;2D";

/// The pipe descriptor/handle the models hand out; writes to anything else are
/// not input and are ignored.
const PIPE: u64 = 0x99;

/// The dump for `target`, built once per test binary: every case of a target
/// reads the same program, and an `--app` build is the slow part.
fn ncode_for(target: &'static str) -> &'static Value {
    ncode_of(target, PROGRAM)
}

fn ncode_of(target: &'static str, program: &'static str) -> &'static Value {
    type Cache = Mutex<HashMap<(&'static str, &'static str), &'static Value>>;
    static CACHE: OnceLock<Cache> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut cache = cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    cache.entry((target, program)).or_insert_with(|| {
        let project = temp_project("app_special_keys", program);
        let ncode = build_ncode_app(&project, target, "app_special_keys");
        let _ = std::fs::remove_dir_all(&project);
        Box::leak(Box::new(ncode))
    })
}

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn assert_injects(surface: &str, key: &str, got: &[u8], want: &[u8]) {
    assert_eq!(
        hex(got),
        hex(want),
        "{surface}: {key} must inject {:?} (kitty form); the handler wrote {:?}",
        String::from_utf8_lossy(want),
        String::from_utf8_lossy(got)
    );
}

// ---------------------------------------------------------------------------
// macOS (AppKit)
// ---------------------------------------------------------------------------

/// AppKit's function-key code points (`NSEvent.h`) and virtual key codes
/// (`Events.h`, `kVK_*`), and the modifier flag bits (`NSEventModifierFlag*`).
mod appkit {
    pub const LEFT_ARROW: u16 = 0xF702; // NSLeftArrowFunctionKey
    pub const F5: u16 = 0xF708; // NSF5FunctionKey
    pub const DELETE: u16 = 0xF728; // NSDeleteFunctionKey (forward delete)
    pub const KVK_ANSI_A: u64 = 0x00;
    pub const KVK_LEFT_ARROW: u64 = 0x7B;
    pub const KVK_F5: u64 = 0x60;
    pub const KVK_FORWARD_DELETE: u64 = 0x75;
    pub const SHIFT: u64 = 1 << 17;
    pub const NUMERIC_PAD: u64 = 1 << 21;
    pub const FUNCTION: u64 = 1 << 23;
    pub const KEY_DOWN: u64 = 10;
    pub const KEY_UP: u64 = 11;
}

#[derive(Clone)]
struct MacKey {
    characters: Vec<u16>,
    key_code: u64,
    modifier_flags: u64,
}

impl MacKey {
    fn letter_a() -> MacKey {
        MacKey {
            characters: vec![u16::from(b'a')],
            key_code: appkit::KVK_ANSI_A,
            modifier_flags: 0,
        }
    }
    fn left() -> MacKey {
        MacKey {
            characters: vec![appkit::LEFT_ARROW],
            key_code: appkit::KVK_LEFT_ARROW,
            modifier_flags: appkit::FUNCTION | appkit::NUMERIC_PAD,
        }
    }
    fn shift_left() -> MacKey {
        MacKey {
            modifier_flags: appkit::FUNCTION | appkit::NUMERIC_PAD | appkit::SHIFT,
            ..MacKey::left()
        }
    }
    fn forward_delete() -> MacKey {
        MacKey {
            characters: vec![appkit::DELETE],
            key_code: appkit::KVK_FORWARD_DELETE,
            modifier_flags: appkit::FUNCTION,
        }
    }
    fn f5() -> MacKey {
        MacKey {
            characters: vec![appkit::F5],
            key_code: appkit::KVK_F5,
            modifier_flags: appkit::FUNCTION,
        }
    }
}

enum MacObject {
    Event {
        key: MacKey,
        kind: u64,
    },
    String(Vec<u16>),
    Application,
    /// The view whose `keyDown:`/`keyUp:` runs (`self`).
    View,
    /// An object the handler only passes along (a text storage).
    Opaque,
}

/// AppKit as a key handler sees it: one `NSEvent`, the shared application, the
/// associated objects the bootstrap stashes (pipe fd, input mode, line buffer),
/// and `write(2)`.
struct MacModel {
    objects: HashMap<u64, MacObject>,
    selectors: HashMap<u64, String>,
    selector_ids: HashMap<String, u64>,
    written: Vec<u8>,
}

impl MacModel {
    fn new() -> MacModel {
        MacModel {
            objects: HashMap::new(),
            selectors: HashMap::new(),
            selector_ids: HashMap::new(),
            written: Vec::new(),
        }
    }

    fn object(&mut self, machine: &mut Machine, object: MacObject) -> u64 {
        let id = machine.alloc(16);
        self.objects.insert(id, object);
        id
    }

    fn event(&mut self, machine: &mut Machine, key: &MacKey, kind: u64) -> u64 {
        self.object(
            machine,
            MacObject::Event {
                key: key.clone(),
                kind,
            },
        )
    }
}

impl Externals for MacModel {
    fn call(&mut self, m: &mut Machine, name: &str) -> Option<u64> {
        match name {
            "_sel_registerName" => {
                let text = String::from_utf8(m.read_cstr(m.arg(0))).expect("selector name");
                if let Some(id) = self.selector_ids.get(&text) {
                    return Some(*id);
                }
                let id = m.alloc(8);
                self.selectors.insert(id, text.clone());
                self.selector_ids.insert(text, id);
                Some(id)
            }
            "_objc_getAssociatedObject" => {
                let key = m.symbol_at(m.arg(1))?.to_string();
                match key.as_str() {
                    "_mfb_macapp_pipe_key" => Some(PIPE),
                    // INPUT_MODE_RAW_NO_ECHO: the mode `io::readByte` sets.
                    "_mfb_macapp_inputmode_key" => Some(2),
                    "_mfb_macapp_inputline_key" => {
                        Some(self.object(m, MacObject::String(Vec::new())))
                    }
                    _ => None,
                }
            }
            "_strlen" => Some(m.read_cstr(m.arg(0)).len() as u64),
            "_write" => {
                let (fd, buf, len) = (m.arg(0), m.arg(1), m.arg(2) as usize);
                if fd == PIPE {
                    self.written.extend(m.read_bytes(buf, len));
                }
                Some(len as u64)
            }
            "_objc_msgSend" => {
                let receiver = m.arg(0);
                let selector = self.selectors.get(&m.arg(1))?.clone();
                let result = match (self.objects.get(&receiver), selector.as_str()) {
                    (_, "sharedApplication") => {
                        return Some(self.object(m, MacObject::Application));
                    }
                    (Some(MacObject::Event { key, .. }), "characters")
                    | (Some(MacObject::Event { key, .. }), "charactersIgnoringModifiers") => {
                        let text = key.characters.clone();
                        return Some(self.object(m, MacObject::String(text)));
                    }
                    (Some(MacObject::Event { key, .. }), "keyCode") => key.key_code,
                    (Some(MacObject::Event { key, .. }), "modifierFlags") => key.modifier_flags,
                    (Some(MacObject::Event { kind, .. }), "type") => *kind,
                    (Some(MacObject::Event { .. }), "isARepeat") => 0,
                    (Some(MacObject::View), "textStorage") => {
                        return Some(self.object(m, MacObject::Opaque));
                    }
                    (Some(MacObject::String(units)), "length") => units.len() as u64,
                    (Some(MacObject::String(units)), "characterAtIndex:") => {
                        u64::from(*units.get(m.arg(2) as usize)?)
                    }
                    (Some(MacObject::String(units)), "UTF8String") => {
                        let mut bytes = String::from_utf16(units).ok()?.into_bytes();
                        bytes.push(0);
                        let address = m.alloc(bytes.len());
                        m.write_bytes(address, &bytes);
                        address
                    }
                    _ => panic!(
                        "the AppKit model does not answer -{selector} sent to {}",
                        match self.objects.get(&receiver) {
                            Some(MacObject::Event { .. }) => "an NSEvent",
                            Some(MacObject::String(_)) => "an NSString",
                            Some(MacObject::Application) => "NSApp",
                            Some(MacObject::View) => "the view",
                            Some(MacObject::Opaque) => "an opaque object",
                            None => "an unknown object",
                        }
                    ),
                };
                Some(result)
            }
            _ => None,
        }
    }
}

/// One macOS surface: the class, and the IMPs its `keyDown:`/`keyUp:` resolve to.
struct MacSurface {
    ncode: &'static Value,
    class: String,
    key_down: String,
    key_up: Option<String>,
}

/// Every class that registers a `keyDown:` IMP, with its `keyUp:` IMP if any —
/// found from the program's own `class_addMethod(cls, sel, imp, types)` calls.
fn mac_surfaces_in(ncode: &'static Value) -> Vec<MacSurface> {
    let sites = find_calls(ncode, "_class_addMethod");
    let selector = |site: &CallSite| -> Option<String> {
        match &site.args[1] {
            Sym::Call(callee, args) if callee == "_sel_registerName" => match &args[0] {
                Sym::Addr(symbol) => data_cstr(ncode, symbol),
                _ => None,
            },
            _ => None,
        }
    };
    let imp = |site: &CallSite| -> Option<String> {
        match &site.args[2] {
            Sym::Addr(symbol) => Some(symbol.clone()),
            _ => None,
        }
    };
    let mut surfaces = Vec::new();
    for site in &sites {
        if selector(site).as_deref() != Some("keyDown:") {
            continue;
        }
        let key_up = sites
            .iter()
            .find(|other| {
                other.args[0] == site.args[0]
                    && other.function == site.function
                    && selector(other).as_deref() == Some("keyUp:")
            })
            .and_then(imp);
        surfaces.push(MacSurface {
            ncode,
            class: class_name(ncode, &site.args[0]),
            key_down: imp(site).expect("keyDown: IMP is a function address"),
            key_up,
        });
    }
    surfaces
}

/// The class a `class_addMethod` adds to, by the name it was allocated with.
fn class_name(ncode: &Value, class: &Sym) -> String {
    match class {
        Sym::Call(callee, args) if callee == "_objc_allocateClassPair" => match &args[1] {
            Sym::Addr(symbol) => data_cstr(ncode, symbol).unwrap_or_else(|| symbol.clone()),
            other => format!("{other:?}"),
        },
        other => format!("{other:?}"),
    }
}

/// Every macOS surface across the Console-start and the canvas program.
fn mac_surfaces() -> Vec<MacSurface> {
    let mut surfaces = mac_surfaces_in(ncode_of("macos-aarch64", CONSOLE_PROGRAM));
    for surface in mac_surfaces_in(ncode_of("macos-aarch64", PROGRAM)) {
        if !surfaces.iter().any(|s| s.key_down == surface.key_down) {
            surfaces.push(surface);
        }
    }
    surfaces
}

/// Run one IMP (`void imp(id self, SEL _cmd, NSEvent *event)`) on one event.
fn run_mac(ncode: &Value, imp: &str, key: &MacKey, kind: u64) -> Vec<u8> {
    let mut machine = Machine::new(ncode);
    let mut model = MacModel::new();
    let view = model.object(&mut machine, MacObject::View);
    let event = model.event(&mut machine, key, kind);
    machine.call(imp, &[view, 0, event], &mut model);
    model.written
}

fn mac_press(key: &MacKey) -> Vec<(String, Vec<u8>)> {
    mac_surfaces()
        .into_iter()
        .map(|s| {
            let bytes = run_mac(s.ncode, &s.key_down, key, appkit::KEY_DOWN);
            (format!("macOS {} ({})", s.class, s.key_down), bytes)
        })
        .collect()
}

#[test]
fn macos_every_surface_registers_a_key_down_handler_that_the_model_drives() {
    // Control: the harness finds all three surfaces (transcript, TUI, canvas)
    // and a plain letter reaches the pipe through each. If this fails, the red
    // cases below say nothing about the handlers.
    let surfaces = mac_surfaces();
    assert_eq!(
        surfaces.len(),
        3,
        "expected keyDown: on the transcript, TUI and canvas views; found {:?}",
        surfaces.iter().map(|s| &s.key_down).collect::<Vec<_>>()
    );
    for (surface, bytes) in mac_press(&MacKey::letter_a()) {
        assert!(
            !bytes.is_empty(),
            "{surface}: `a` must reach the input pipe"
        );
    }
}

/// bug-699 (found by this harness): a program that calls `app::setMode` starts
/// windowless, and the bootstrap built the transcript's `MFBTextView` class and
/// the TUI's `TermView` only on the `Console`-start path — so after
/// `setMode(Console)` the window took no keys at all (measured on macOS 15.7.7:
/// nothing delivered, while the same program without `setMode` got every key),
/// and `term::on` had no TermView to show. A mode-switching program must register
/// the same `keyDown:` handlers a Console-start one does.
#[test]
fn macos_a_mode_switching_program_registers_every_key_down_handler() {
    let surfaces = mac_surfaces_in(ncode_of("macos-aarch64", PROGRAM));
    for imp in [
        "_mfb_macapp_key_down",
        "_mfb_macapp_term_keyDown",
        "_mfb_macapp_canvas_keyDown",
    ] {
        let surface = surfaces
            .iter()
            .find(|s| s.key_down == imp)
            .unwrap_or_else(|| {
                panic!(
                    "a setMode program registers no keyDown: whose IMP is {imp}; found {:?}",
                    surfaces.iter().map(|s| &s.key_down).collect::<Vec<_>>()
                )
            });
        let bytes = run_mac(surface.ncode, imp, &MacKey::letter_a(), appkit::KEY_DOWN);
        assert_injects(
            &format!("macOS {} ({imp})", surface.class),
            "a",
            &bytes,
            b"a",
        );
    }
}

#[test]
#[ignore = "bug-699 Phase 3: macOS keyDown: writes the AppKit private-use code point verbatim"]
fn macos_left_injects_kitty_left() {
    for (surface, bytes) in mac_press(&MacKey::left()) {
        assert_injects(&surface, "Left", &bytes, LEFT_PRESS);
    }
}

#[test]
#[ignore = "bug-699 Phase 3: macOS keyDown: writes the AppKit private-use code point verbatim"]
fn macos_forward_delete_injects_kitty_delete() {
    for (surface, bytes) in mac_press(&MacKey::forward_delete()) {
        assert_injects(&surface, "Delete", &bytes, DELETE_PRESS);
    }
}

#[test]
#[ignore = "bug-699 Phase 3: macOS keyDown: writes the AppKit private-use code point verbatim"]
fn macos_f5_injects_kitty_f5() {
    for (surface, bytes) in mac_press(&MacKey::f5()) {
        assert_injects(&surface, "F5", &bytes, F5_PRESS);
    }
}

#[test]
#[ignore = "bug-699 Phase 3: no macOS view registers a keyUp: handler"]
fn macos_left_release_injects_kitty_release() {
    for surface in mac_surfaces() {
        let name = format!("macOS {} ({})", surface.class, surface.key_down);
        let key_up = surface
            .key_up
            .unwrap_or_else(|| panic!("{name}: no keyUp: IMP, so a release is never observable"));
        let bytes = run_mac(surface.ncode, &key_up, &MacKey::left(), appkit::KEY_UP);
        assert_injects(&name, "Left release", &bytes, LEFT_RELEASE);
    }
}

#[test]
#[ignore = "bug-699 Phase 3: macOS keyDown: ignores modifierFlags"]
fn macos_shift_left_injects_kitty_shift_left() {
    for (surface, bytes) in mac_press(&MacKey::shift_left()) {
        assert_injects(&surface, "Shift+Left", &bytes, SHIFT_LEFT_PRESS);
    }
}

// ---------------------------------------------------------------------------
// Linux (GTK 4)
// ---------------------------------------------------------------------------

/// GDK keyvals (`gdk/gdkkeysyms.h`) and modifier masks (`GdkModifierType`).
mod gdk {
    pub const KEY_A: u64 = 0x061;
    pub const KEY_LEFT: u64 = 0xff51;
    pub const KEY_DELETE: u64 = 0xffff;
    pub const KEY_F5: u64 = 0xffc2;
    pub const SHIFT_MASK: u64 = 1 << 0;
    /// X11 hardware keycodes (evdev + 8) for the same keys.
    pub const HW_A: u64 = 38;
    pub const HW_LEFT: u64 = 113;
    pub const HW_DELETE: u64 = 119;
    pub const HW_F5: u64 = 71;
}

/// `gdk_keyval_to_unicode` as GDK implements it for the keys used here: Latin-1
/// keyvals map to themselves, the `0x01000000` range carries a code point, the
/// TTY function keys map to their ASCII controls (`Delete` to `0x7F`), and every
/// other function key — arrows, Home/End, F-keys — maps to 0.
fn gdk_keyval_to_unicode(keyval: u64) -> u64 {
    match keyval {
        0x20..=0x7e | 0xa0..=0xff => keyval,
        0x0100_0000..=0x0110_ffff => keyval - 0x0100_0000,
        0xff08 => 0x08, // BackSpace
        0xff09 => 0x09, // Tab
        0xff0d => 0x0d, // Return
        0xff1b => 0x1b, // Escape
        0xffff => 0x7f, // Delete
        _ => 0,
    }
}

/// `_mfb_gtkapp_state` field offsets and the raw input mode
/// (`src/target/linux_gtk/mod.rs`: `ST_PIPE_WRITE_FD`, `ST_INPUT_MODE`,
/// `MODE_RAW`). A layout change there makes the control test below fail.
const GTK_ST_PIPE_WRITE_FD: u64 = 48;
const GTK_ST_INPUT_MODE: u64 = 72;
const GTK_MODE_RAW: u64 = 2;

struct GtkModel {
    written: Vec<u8>,
}

impl Externals for GtkModel {
    fn call(&mut self, m: &mut Machine, name: &str) -> Option<u64> {
        match name {
            "gdk_keyval_to_unicode" => Some(gdk_keyval_to_unicode(m.arg(0))),
            "g_unichar_to_utf8" => {
                let c = char::from_u32(m.arg(0) as u32)?;
                let mut buf = [0u8; 4];
                let bytes = c.encode_utf8(&mut buf).as_bytes().to_vec();
                m.write_bytes(m.arg(1), &bytes);
                Some(bytes.len() as u64)
            }
            "write" => {
                let (fd, buf, len) = (m.arg(0), m.arg(1), m.arg(2) as usize);
                if fd == PIPE {
                    self.written.extend(m.read_bytes(buf, len));
                }
                Some(len as u64)
            }
            _ => None,
        }
    }
}

fn gtk_ncode() -> &'static Value {
    ncode_for("linux-aarch64")
}

/// The handlers the program connects to a GTK signal, by signal name.
fn gtk_handlers(ncode: &Value, signal: &str) -> Vec<String> {
    let mut handlers: Vec<String> = find_calls(ncode, "g_signal_connect_data")
        .into_iter()
        .filter(|site| matches!(&site.args[1], Sym::Addr(s) if data_cstr(ncode, s).as_deref() == Some(signal)))
        .filter_map(|site| match &site.args[2] {
            Sym::Addr(handler) => Some(handler.clone()),
            _ => None,
        })
        .collect();
    handlers.sort();
    handlers.dedup();
    handlers
}

/// Run a GTK key handler (`gboolean (*)(GtkEventControllerKey*, guint keyval,
/// guint keycode, GdkModifierType state, gpointer)`) in raw input mode.
fn run_gtk(ncode: &Value, handler: &str, keyval: u64, keycode: u64, state: u64) -> Vec<u8> {
    let mut machine = Machine::new(ncode);
    let base = machine.address_of("_mfb_gtkapp_state");
    machine.write_uint(base + GTK_ST_INPUT_MODE, 8, GTK_MODE_RAW);
    machine.write_uint(base + GTK_ST_PIPE_WRITE_FD, 8, PIPE);
    let mut model = GtkModel {
        written: Vec::new(),
    };
    let controller = machine.alloc(16);
    machine.call(
        handler,
        &[controller, keyval, keycode, state, 0],
        &mut model,
    );
    model.written
}

fn gtk_press(ncode: &Value, keyval: u64, keycode: u64, state: u64) -> Vec<(String, Vec<u8>)> {
    let handlers = gtk_handlers(ncode, "key-pressed");
    assert!(!handlers.is_empty(), "no key-pressed handler is connected");
    handlers
        .into_iter()
        .map(|h| {
            let bytes = run_gtk(ncode, &h, keyval, keycode, state);
            (format!("GTK key-pressed ({h})"), bytes)
        })
        .collect()
}

#[test]
fn gtk_key_pressed_handler_is_driven_by_the_model() {
    let ncode = gtk_ncode();
    for (surface, bytes) in gtk_press(ncode, gdk::KEY_A, gdk::HW_A, 0) {
        assert_injects(&surface, "a", &bytes, b"a");
    }
}

#[test]
#[ignore = "bug-699 Phase 3: GTK raw arm drops keys gdk_keyval_to_unicode maps to 0"]
fn gtk_left_injects_kitty_left() {
    let ncode = gtk_ncode();
    for (surface, bytes) in gtk_press(ncode, gdk::KEY_LEFT, gdk::HW_LEFT, 0) {
        assert_injects(&surface, "Left", &bytes, LEFT_PRESS);
    }
}

#[test]
#[ignore = "bug-699 Phase 3: GTK raw arm writes gdk_keyval_to_unicode(GDK_KEY_Delete) = 0x7F"]
fn gtk_delete_injects_kitty_delete() {
    let ncode = gtk_ncode();
    for (surface, bytes) in gtk_press(ncode, gdk::KEY_DELETE, gdk::HW_DELETE, 0) {
        assert_injects(&surface, "Delete", &bytes, DELETE_PRESS);
    }
}

#[test]
#[ignore = "bug-699 Phase 3: GTK raw arm drops keys gdk_keyval_to_unicode maps to 0"]
fn gtk_f5_injects_kitty_f5() {
    let ncode = gtk_ncode();
    for (surface, bytes) in gtk_press(ncode, gdk::KEY_F5, gdk::HW_F5, 0) {
        assert_injects(&surface, "F5", &bytes, F5_PRESS);
    }
}

#[test]
#[ignore = "bug-699 Phase 3: no key-released handler is connected"]
fn gtk_left_release_injects_kitty_release() {
    let ncode = gtk_ncode();
    let handlers = gtk_handlers(ncode, "key-released");
    assert!(
        !handlers.is_empty(),
        "no key-released handler is connected, so a release is never observable"
    );
    for handler in handlers {
        let bytes = run_gtk(ncode, &handler, gdk::KEY_LEFT, gdk::HW_LEFT, 0);
        assert_injects(
            &format!("GTK key-released ({handler})"),
            "Left release",
            &bytes,
            LEFT_RELEASE,
        );
    }
}

#[test]
#[ignore = "bug-699 Phase 3: GTK raw arm drops Left and ignores the modifier state"]
fn gtk_shift_left_injects_kitty_shift_left() {
    let ncode = gtk_ncode();
    for (surface, bytes) in gtk_press(ncode, gdk::KEY_LEFT, gdk::HW_LEFT, gdk::SHIFT_MASK) {
        assert_injects(&surface, "Shift+Left", &bytes, SHIFT_LEFT_PRESS);
    }
}

// ---------------------------------------------------------------------------
// Windows (Win32)
// ---------------------------------------------------------------------------

/// Window messages and virtual-key codes (`WinUser.h`).
mod win32 {
    pub const WM_KEYDOWN: u64 = 0x0100;
    pub const WM_KEYUP: u64 = 0x0101;
    pub const WM_CHAR: u64 = 0x0102;
    pub const VK_SHIFT: u64 = 0x10;
    pub const VK_LEFT: u64 = 0x25;
    pub const VK_DELETE: u64 = 0x2E;
    pub const VK_F5: u64 = 0x74;
    /// `lParam` for a key message: repeat count 1, the scan code, the extended
    /// bit (24) for the cursor block, and for a release the previous-state (30)
    /// and transition (31) bits.
    pub fn lparam(scan: u64, extended: bool, release: bool) -> u64 {
        let mut value = 1 | (scan << 16);
        if extended {
            value |= 1 << 24;
        }
        if release {
            value |= (1 << 30) | (1 << 31);
        }
        value
    }
    pub const SCAN_LEFT: u64 = 0x4B;
    pub const SCAN_DELETE: u64 = 0x53;
    pub const SCAN_F5: u64 = 0x3F;
}

/// Win32 as a window procedure sees it: `WriteFile` to the input pipe, the
/// default/chained procedures, and the keyboard state for the modifier keys.
struct WinModel {
    written: Vec<u8>,
    held: Vec<u64>,
}

impl WinModel {
    fn key_state(&self, vk: u64) -> u64 {
        // SHORT with the high bit set when the key is down, sign-extended as the
        // x64 ABI returns it.
        if self.held.contains(&vk) {
            0xFFFF_FFFF_FFFF_8000
        } else {
            0
        }
    }
}

impl Externals for WinModel {
    fn call(&mut self, m: &mut Machine, name: &str) -> Option<u64> {
        match name {
            "WriteFile" => {
                let (handle, buf, len, written) = (m.arg(0), m.arg(1), m.arg(2), m.arg(3));
                if handle == PIPE {
                    self.written.extend(m.read_bytes(buf, len as usize));
                }
                if written != 0 {
                    m.write_uint(written, 4, len);
                }
                Some(1)
            }
            "CallWindowProcW" | "DefWindowProcW" => Some(0),
            "GetKeyState" | "GetAsyncKeyState" => Some(self.key_state(m.arg(0))),
            "GetKeyboardState" => {
                let buf = m.arg(0);
                for vk in 0..256u64 {
                    let down = if self.held.contains(&vk) { 0x80 } else { 0 };
                    m.write_uint(buf + vk, 1, down);
                }
                Some(1)
            }
            _ => None,
        }
    }
}

fn win_ncode() -> &'static Value {
    ncode_for("windows-x86_64")
}

/// The two window procedures that feed the input pipe: the top-level window's
/// (canvas surface) and the transcript EDIT's subclass.
const WNDPROC: &str = "_mfb_winapp_wndproc";
const EDITPROC: &str = "_mfb_winapp_editproc";

/// Deliver the messages to one procedure and return the bytes written to the
/// pipe. The canvas surface is marked presented so `wndproc`'s canvas input arm
/// is live, exactly as `app::setMode(Canvas)` leaves it.
fn run_win(
    ncode: &Value,
    proc_symbol: &str,
    messages: &[(u64, u64, u64)],
    held: &[u64],
) -> Vec<u8> {
    let mut machine = Machine::new(ncode);
    assert!(
        machine.has_function(proc_symbol),
        "no {proc_symbol} in the dump"
    );
    let stdin_write = machine.address_of("_mfb_winapp_stdin_write");
    machine.write_uint(stdin_write, 8, PIPE);
    let canvas = machine.address_of("_mfb_winapp_canvas_hwnd");
    machine.write_uint(canvas, 8, 0x4242);
    let mut model = WinModel {
        written: Vec::new(),
        held: held.to_vec(),
    };
    let hwnd = 0x4242;
    for (msg, wparam, lparam) in messages {
        machine.call(proc_symbol, &[hwnd, *msg, *wparam, *lparam], &mut model);
    }
    model.written
}

fn win_each_proc(messages: &[(u64, u64, u64)], held: &[u64]) -> Vec<(String, Vec<u8>)> {
    let ncode = win_ncode();
    [WNDPROC, EDITPROC]
        .into_iter()
        .map(|p| (format!("Windows {p}"), run_win(ncode, p, messages, held)))
        .collect()
}

fn key_down(vk: u64, scan: u64, extended: bool) -> (u64, u64, u64) {
    (win32::WM_KEYDOWN, vk, win32::lparam(scan, extended, false))
}

#[test]
fn windows_procs_are_driven_by_the_model() {
    // Control: `a` arrives as WM_CHAR 'a' (TranslateMessage runs in the app's
    // own message loop, so a procedure sees WM_KEYDOWN and then WM_CHAR).
    let messages = [
        key_down(0x41, 0x1E, false),
        (
            win32::WM_CHAR,
            u64::from(b'a'),
            win32::lparam(0x1E, false, false),
        ),
    ];
    for (surface, bytes) in win_each_proc(&messages, &[]) {
        assert!(
            !bytes.is_empty(),
            "{surface}: `a` must reach the input pipe"
        );
    }
}

#[test]
#[ignore = "bug-699 Phase 3: the window procedures handle WM_CHAR only; Left sends no WM_CHAR"]
fn windows_left_injects_kitty_left() {
    let messages = [key_down(win32::VK_LEFT, win32::SCAN_LEFT, true)];
    for (surface, bytes) in win_each_proc(&messages, &[]) {
        assert_injects(&surface, "Left", &bytes, LEFT_PRESS);
    }
}

#[test]
#[ignore = "bug-699 Phase 3: the window procedures handle WM_CHAR only; Delete sends no WM_CHAR"]
fn windows_delete_injects_kitty_delete() {
    let messages = [key_down(win32::VK_DELETE, win32::SCAN_DELETE, true)];
    for (surface, bytes) in win_each_proc(&messages, &[]) {
        assert_injects(&surface, "Delete", &bytes, DELETE_PRESS);
    }
}

#[test]
#[ignore = "bug-699 Phase 3: the window procedures handle WM_CHAR only; F5 sends no WM_CHAR"]
fn windows_f5_injects_kitty_f5() {
    let messages = [key_down(win32::VK_F5, win32::SCAN_F5, false)];
    for (surface, bytes) in win_each_proc(&messages, &[]) {
        assert_injects(&surface, "F5", &bytes, F5_PRESS);
    }
}

#[test]
#[ignore = "bug-699 Phase 3: the window procedures ignore WM_KEYUP"]
fn windows_left_release_injects_kitty_release() {
    let messages = [(
        win32::WM_KEYUP,
        win32::VK_LEFT,
        win32::lparam(win32::SCAN_LEFT, true, true),
    )];
    for (surface, bytes) in win_each_proc(&messages, &[]) {
        assert_injects(&surface, "Left release", &bytes, LEFT_RELEASE);
    }
}

#[test]
#[ignore = "bug-699 Phase 3: the window procedures handle WM_CHAR only; Left sends no WM_CHAR"]
fn windows_shift_left_injects_kitty_shift_left() {
    let messages = [key_down(win32::VK_LEFT, win32::SCAN_LEFT, true)];
    for (surface, bytes) in win_each_proc(&messages, &[win32::VK_SHIFT]) {
        assert_injects(&surface, "Shift+Left", &bytes, SHIFT_LEFT_PRESS);
    }
}

/// The sibling defect on the Windows arms (bug-699 Blast Radius): `WM_CHAR`
/// carries a UTF-16 code unit, and both arms write `wParam & 0xFF`. `é`
/// (U+00E9) enters the pipe as the lone byte `E9` — invalid UTF-8 — and a
/// character outside the BMP arrives as two surrogate halves, each truncated.
#[test]
#[ignore = "bug-699 Phase 3: WM_CHAR is written as the low byte of the UTF-16 unit"]
fn windows_non_ascii_wm_char_is_written_as_utf8() {
    let lp = win32::lparam(0, false, false);
    let e_acute = [(win32::WM_CHAR, 0x00E9, lp)];
    for (surface, bytes) in win_each_proc(&e_acute, &[]) {
        assert_eq!(
            hex(&bytes),
            hex("é".as_bytes()),
            "{surface}: WM_CHAR U+00E9"
        );
    }
    // U+1F600 arrives as the surrogate pair D83D DE00, one WM_CHAR each.
    let grinning = [(win32::WM_CHAR, 0xD83D, lp), (win32::WM_CHAR, 0xDE00, lp)];
    for (surface, bytes) in win_each_proc(&grinning, &[]) {
        assert_eq!(
            hex(&bytes),
            hex("\u{1F600}".as_bytes()),
            "{surface}: WM_CHAR surrogate pair D83D DE00"
        );
    }
}

#[test]
fn windows_ascii_wm_char_is_written_verbatim() {
    // Control for the case above: ASCII is already correct, so the red case
    // is about the encoding, not about reaching the pipe.
    let messages = [(
        win32::WM_CHAR,
        u64::from(b'z'),
        win32::lparam(0x2C, false, false),
    )];
    for (surface, bytes) in win_each_proc(&messages, &[]) {
        assert_eq!(hex(&bytes), hex(b"z"), "{surface}: WM_CHAR 'z'");
    }
}
