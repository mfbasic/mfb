//! plan-176-E: `app_special_keys`'s macOS (AppKit) section: the AppKit model, its
//! surfaces and the 11 `macos_*` key tests. The key constants and the shared
//! `ncode_*`/`assert_injects` helpers live in the root, `app_special_keys.rs`.

use super::*;

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
    pub const CONTROL: u64 = 1 << 18;
    pub const NUMERIC_PAD: u64 = 1 << 21;
    pub const FUNCTION: u64 = 1 << 23;
    pub const KEY_DOWN: u64 = 10;
    pub const KEY_UP: u64 = 11;
}

#[derive(Clone)]
struct MacKey {
    characters: Vec<u16>,
    /// `charactersIgnoringModifiers`, when it differs from `characters` (AppKit
    /// ignores every modifier there but Shift: Ctrl+A is `"\u{1}"` / `"a"`).
    ignoring_modifiers: Option<Vec<u16>>,
    key_code: u64,
    modifier_flags: u64,
    repeat: bool,
}

impl MacKey {
    fn letter_a() -> MacKey {
        MacKey {
            characters: vec![u16::from(b'a')],
            ignoring_modifiers: None,
            key_code: appkit::KVK_ANSI_A,
            modifier_flags: 0,
            repeat: false,
        }
    }
    fn left() -> MacKey {
        MacKey {
            characters: vec![appkit::LEFT_ARROW],
            ignoring_modifiers: None,
            key_code: appkit::KVK_LEFT_ARROW,
            modifier_flags: appkit::FUNCTION | appkit::NUMERIC_PAD,
            repeat: false,
        }
    }
    fn shift_left() -> MacKey {
        MacKey {
            modifier_flags: appkit::FUNCTION | appkit::NUMERIC_PAD | appkit::SHIFT,
            ..MacKey::left()
        }
    }
    fn shift_a() -> MacKey {
        MacKey {
            characters: vec![u16::from(b'A')],
            modifier_flags: appkit::SHIFT,
            ..MacKey::letter_a()
        }
    }
    fn ctrl_a() -> MacKey {
        MacKey {
            characters: vec![0x01],
            ignoring_modifiers: Some(vec![u16::from(b'a')]),
            modifier_flags: appkit::CONTROL,
            ..MacKey::letter_a()
        }
    }
    fn forward_delete() -> MacKey {
        MacKey {
            characters: vec![appkit::DELETE],
            ignoring_modifiers: None,
            key_code: appkit::KVK_FORWARD_DELETE,
            modifier_flags: appkit::FUNCTION,
            repeat: false,
        }
    }
    fn f5() -> MacKey {
        MacKey {
            characters: vec![appkit::F5],
            ignoring_modifiers: None,
            key_code: appkit::KVK_F5,
            modifier_flags: appkit::FUNCTION,
            repeat: false,
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
    /// The window's input mode (the `INPUT_MODE_KEY` associated object).
    mode: u64,
    objects: HashMap<u64, MacObject>,
    selectors: HashMap<u64, String>,
    selector_ids: HashMap<String, u64>,
    written: Vec<u8>,
}

impl MacModel {
    fn new() -> MacModel {
        MacModel {
            mode: 2,
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
                    // INPUT_MODE_RAW_NO_ECHO (2) unless a test says otherwise: the
                    // mode the key readers set.
                    "_mfb_macapp_inputmode_key" => Some(self.mode),
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
                    (Some(MacObject::Event { key, .. }), "characters") => {
                        let text = key.characters.clone();
                        return Some(self.object(m, MacObject::String(text)));
                    }
                    (Some(MacObject::Event { key, .. }), "charactersIgnoringModifiers") => {
                        let text = key
                            .ignoring_modifiers
                            .clone()
                            .unwrap_or_else(|| key.characters.clone());
                        return Some(self.object(m, MacObject::String(text)));
                    }
                    (Some(MacObject::Event { key, .. }), "keyCode") => key.key_code,
                    (Some(MacObject::Event { key, .. }), "modifierFlags") => key.modifier_flags,
                    (Some(MacObject::Event { kind, .. }), "type") => *kind,
                    (Some(MacObject::Event { key, .. }), "isARepeat") => u64::from(key.repeat),
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
    run_mac_in_mode(ncode, imp, key, kind, 2)
}

/// [`run_mac`] with the window in input mode `mode` (1 line, 2 raw).
fn run_mac_in_mode(ncode: &Value, imp: &str, key: &MacKey, kind: u64, mode: u64) -> Vec<u8> {
    let mut machine = Machine::new(ncode);
    let mut model = MacModel::new();
    model.mode = mode;
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
            A_PRESS,
        );
    }
}

#[test]
fn macos_left_injects_kitty_left() {
    for (surface, bytes) in mac_press(&MacKey::left()) {
        assert_injects(&surface, "Left", &bytes, LEFT_PRESS);
    }
}

#[test]
fn macos_forward_delete_injects_kitty_delete() {
    for (surface, bytes) in mac_press(&MacKey::forward_delete()) {
        assert_injects(&surface, "Delete", &bytes, DELETE_PRESS);
    }
}

#[test]
fn macos_f5_injects_kitty_f5() {
    for (surface, bytes) in mac_press(&MacKey::f5()) {
        assert_injects(&surface, "F5", &bytes, F5_PRESS);
    }
}

#[test]
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
fn macos_shift_left_injects_kitty_shift_left() {
    for (surface, bytes) in mac_press(&MacKey::shift_left()) {
        assert_injects(&surface, "Shift+Left", &bytes, SHIFT_LEFT_PRESS);
    }
}

/// A held key's auto-repeat is a Repeat event (`isARepeat`), not another press.
#[test]
fn macos_auto_repeat_injects_a_repeat() {
    let key = MacKey {
        repeat: true,
        ..MacKey::left()
    };
    for (surface, bytes) in mac_press(&key) {
        assert_injects(&surface, "Left repeat", &bytes, b"\x1b[1;1:2D");
    }
}

/// A text key is a key event too: Shift+a carries the text it typed (`A`) as
/// kitty's associated text, and its key number is the unshifted `a`.
#[test]
fn macos_shift_a_injects_its_text() {
    for (surface, bytes) in mac_press(&MacKey::shift_a()) {
        assert_injects(&surface, "Shift+a", &bytes, b"\x1b[97;2;65u");
    }
}

/// Ctrl+A types the control character U+0001, which is not text: the event is
/// the key `a` with Ctrl, and no text field.
#[test]
fn macos_ctrl_a_injects_the_key_without_text() {
    for (surface, bytes) in mac_press(&MacKey::ctrl_a()) {
        assert_injects(&surface, "Ctrl+a", &bytes, b"\x1b[97;5u");
    }
}

/// In line input mode the transcript and the TUI view report no key events, so
/// their `keyUp:` writes nothing; the canvas view is always raw.
#[test]
fn macos_key_up_reports_nothing_in_line_mode_except_on_the_canvas() {
    for surface in mac_surfaces() {
        let key_up = surface.key_up.clone().expect("every surface has keyUp:");
        let bytes = run_mac_in_mode(surface.ncode, &key_up, &MacKey::left(), appkit::KEY_UP, 1);
        let canvas = surface.key_down == "_mfb_macapp_canvas_keyDown";
        let want: &[u8] = if canvas { LEFT_RELEASE } else { b"" };
        assert_injects(
            &format!("macOS {} ({key_up}) in line mode", surface.class),
            "Left release",
            &bytes,
            want,
        );
    }
}
