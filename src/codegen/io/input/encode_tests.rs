//! The `--app` key encoder (bug-699 Phase 3).
//!
//! Three layers, each against an oracle the encoder does not share:
//! - hand-written kitty sequences for representative events, from the protocol
//!   document (the same table the handler tests in
//!   `tests/codegen/app_special_keys.rs` expect);
//! - every table row through the DECODER's reference model: whatever the encoder
//!   writes for a key, the decoder must read back as that key with that event
//!   and those modifiers — the property the whole design rests on;
//! - the emitted encoder, run in the `.ncode` interpreter as every app backend
//!   lowers it, against the model on the whole input grid.

use super::encode::*;
use super::keys::{KeyEvent, KeyName, MOD_ALT, MOD_CTRL, MOD_META, MOD_SHIFT};
use super::model::{decode, Outcome, Text, EVENTS, FINAL};
use super::ncode_emu::{Externals, Machine};

const APPKIT_SHIFT: u64 = 1 << 17;
const APPKIT_CTRL: u64 = 1 << 18;
const APPKIT_OPTION: u64 = 1 << 19;
const APPKIT_COMMAND: u64 = 1 << 20;
const GDK_SHIFT: u64 = 1;
const GDK_CTRL: u64 = 1 << 2;

fn enc(table: KeyTable, native: u64, base: u64, mods: u64, event: u64, text: u64) -> Vec<u8> {
    model(table, native, base, mods, event, text)
}

#[test]
fn the_encoder_writes_what_a_kitty_terminal_sends() {
    use KeyTable::*;
    let cases: &[(&str, Vec<u8>, &[u8])] = &[
        ("Left", enc(AppKit, 0xF702, 0, 0, EVENT_PRESS, 0), b"\x1b[D"),
        (
            "Delete",
            enc(AppKit, 0xF728, 0, 0, EVENT_PRESS, 0),
            b"\x1b[3~",
        ),
        ("F5", enc(AppKit, 0xF708, 0, 0, EVENT_PRESS, 0), b"\x1b[15~"),
        ("F3", enc(Gdk, 0xFFC0, 0, 0, EVENT_PRESS, 0), b"\x1b[13~"),
        ("F1", enc(Win32, 0x70, 0, 0, EVENT_PRESS, 0), b"\x1b[P"),
        (
            "Left release",
            enc(AppKit, 0xF702, 0, 0, EVENT_RELEASE, 0),
            b"\x1b[1;1:3D",
        ),
        (
            "Shift+Left",
            enc(AppKit, 0xF702, 0, APPKIT_SHIFT, EVENT_PRESS, 0),
            b"\x1b[1;2D",
        ),
        (
            "Ctrl+Delete repeat",
            enc(Gdk, 0xFFFF, 0, GDK_CTRL, EVENT_REPEAT, 0),
            b"\x1b[3;5:2~",
        ),
        (
            "Ctrl+Option+Left",
            enc(
                AppKit,
                0xF702,
                0,
                APPKIT_CTRL | APPKIT_OPTION,
                EVENT_PRESS,
                0,
            ),
            b"\x1b[1;7D",
        ),
        ("Enter", enc(Win32, 0x0D, 0, 0, EVENT_PRESS, 0), b"\x1b[13u"),
        (
            "Shift+Tab (backtab)",
            enc(AppKit, 0x19, 0, APPKIT_SHIFT, EVENT_PRESS, 0),
            b"\x1b[9;2u",
        ),
        (
            "a",
            enc(AppKit, u64::from(b'a'), u64::from(b'a'), 0, EVENT_PRESS, 97),
            b"\x1b[97;;97u",
        ),
        (
            "a repeat",
            enc(Gdk, 0x61, u64::from(b'a'), 0, EVENT_REPEAT, 97),
            b"\x1b[97;1:2;97u",
        ),
        (
            "a release carries no text",
            enc(
                AppKit,
                u64::from(b'a'),
                u64::from(b'a'),
                0,
                EVENT_RELEASE,
                97,
            ),
            b"\x1b[97;1:3u",
        ),
        (
            "Shift+a: AppKit's charactersIgnoringModifiers keeps Shift",
            enc(
                AppKit,
                u64::from(b'A'),
                u64::from(b'A'),
                APPKIT_SHIFT,
                EVENT_PRESS,
                65,
            ),
            b"\x1b[97;2;65u",
        ),
        (
            "Ctrl+a types no text",
            enc(Gdk, 0x61, u64::from(b'a'), GDK_CTRL, EVENT_PRESS, 97),
            b"\x1b[97;5u",
        ),
        (
            "Cmd+a types no text",
            enc(
                AppKit,
                u64::from(b'a'),
                u64::from(b'a'),
                APPKIT_COMMAND,
                EVENT_PRESS,
                97,
            ),
            b"\x1b[97;9u",
        ),
        (
            "Option+e types its composed text",
            enc(
                AppKit,
                u64::from(b'e'),
                u64::from(b'e'),
                APPKIT_OPTION,
                EVENT_PRESS,
                0xE9,
            ),
            b"\x1b[101;3;233u",
        ),
        (
            "a control character is not text",
            enc(Win32, 0, u64::from(b'a'), KITTY_CTRL, EVENT_PRESS, 1),
            b"\x1b[97;5u",
        ),
        (
            "a supplementary-plane character",
            enc(Win32, 0, 0x1F600, 0, EVENT_PRESS, 0x1F600),
            b"\x1b[128512;;128512u",
        ),
        (
            "a modifier key alone",
            enc(Gdk, 0xFFE1, 0, GDK_SHIFT, EVENT_PRESS, 0),
            b"",
        ),
        (
            "an AppKit function key with no row (Help)",
            enc(AppKit, 0xF746, 0xF746, 0, EVENT_PRESS, 0),
            b"",
        ),
    ];
    for (name, got, want) in cases {
        assert_eq!(
            String::from_utf8_lossy(got),
            String::from_utf8_lossy(want),
            "{name}"
        );
    }
}

/// kitty modifier bits -> `io::Key` modifier bits.
fn key_mods(kitty: u64) -> u64 {
    let mut m = 0;
    if kitty & KITTY_SHIFT != 0 {
        m |= MOD_SHIFT;
    }
    if kitty & KITTY_ALT != 0 {
        m |= MOD_ALT;
    }
    if kitty & KITTY_CTRL != 0 {
        m |= MOD_CTRL;
    }
    if kitty & KITTY_SUPER != 0 {
        m |= MOD_META;
    }
    m
}

fn key_event(event: u64) -> KeyEvent {
    match event {
        EVENT_PRESS => KeyEvent::Press,
        EVENT_REPEAT => KeyEvent::Repeat,
        EVENT_RELEASE => KeyEvent::Release,
        other => panic!("event {other}"),
    }
}

/// The key a table row is, written out from the platform headers — not read from
/// the encoder's or the decoder's tables.
fn expected_key(table: KeyTable, native: u64) -> KeyName {
    use KeyName::*;
    match (table, native) {
        (KeyTable::AppKit, 0xF700) => Up,
        (KeyTable::AppKit, 0xF701) => Down,
        (KeyTable::AppKit, 0xF702) => Left,
        (KeyTable::AppKit, 0xF703) => Right,
        (KeyTable::AppKit, n @ 0xF704..=0xF70F) => FUNCTION_KEYS[(n - 0xF704) as usize],
        (KeyTable::AppKit, 0xF727) => Insert,
        (KeyTable::AppKit, 0xF728) => Delete,
        (KeyTable::AppKit, 0xF729) => Home,
        (KeyTable::AppKit, 0xF72B) => End,
        (KeyTable::AppKit, 0xF72C) => PageUp,
        (KeyTable::AppKit, 0xF72D) => PageDown,
        (KeyTable::AppKit, 0x0D | 0x03) => Enter,
        (KeyTable::AppKit, 0x09 | 0x19) => Tab,
        (KeyTable::AppKit, 0x1B) => Escape,
        (KeyTable::AppKit, 0x7F | 0x08) => Backspace,
        (KeyTable::Gdk, 0xFF51 | 0xFF96) => Left,
        (KeyTable::Gdk, 0xFF52 | 0xFF97) => Up,
        (KeyTable::Gdk, 0xFF53 | 0xFF98) => Right,
        (KeyTable::Gdk, 0xFF54 | 0xFF99) => Down,
        (KeyTable::Gdk, 0xFF50 | 0xFF95) => Home,
        (KeyTable::Gdk, 0xFF57 | 0xFF9C) => End,
        (KeyTable::Gdk, 0xFF55 | 0xFF9A) => PageUp,
        (KeyTable::Gdk, 0xFF56 | 0xFF9B) => PageDown,
        (KeyTable::Gdk, 0xFF63 | 0xFF9E) => Insert,
        (KeyTable::Gdk, 0xFFFF | 0xFF9F) => Delete,
        (KeyTable::Gdk, 0xFF08) => Backspace,
        (KeyTable::Gdk, 0xFF09 | 0xFE20) => Tab,
        (KeyTable::Gdk, 0xFF0D | 0xFF8D) => Enter,
        (KeyTable::Gdk, 0xFF1B) => Escape,
        (KeyTable::Gdk, n @ 0xFFBE..=0xFFC9) => FUNCTION_KEYS[(n - 0xFFBE) as usize],
        (KeyTable::Win32, 0x25) => Left,
        (KeyTable::Win32, 0x26) => Up,
        (KeyTable::Win32, 0x27) => Right,
        (KeyTable::Win32, 0x28) => Down,
        (KeyTable::Win32, 0x24) => Home,
        (KeyTable::Win32, 0x23) => End,
        (KeyTable::Win32, 0x21) => PageUp,
        (KeyTable::Win32, 0x22) => PageDown,
        (KeyTable::Win32, 0x2D) => Insert,
        (KeyTable::Win32, 0x2E) => Delete,
        (KeyTable::Win32, 0x08) => Backspace,
        (KeyTable::Win32, 0x09) => Tab,
        (KeyTable::Win32, 0x0D) => Enter,
        (KeyTable::Win32, 0x1B) => Escape,
        (KeyTable::Win32, n @ 0x70..=0x7B) => FUNCTION_KEYS[(n - 0x70) as usize],
        other => panic!("no expected key for {other:?}"),
    }
}

const FUNCTION_KEYS: [KeyName; 12] = [
    KeyName::F1,
    KeyName::F2,
    KeyName::F3,
    KeyName::F4,
    KeyName::F5,
    KeyName::F6,
    KeyName::F7,
    KeyName::F8,
    KeyName::F9,
    KeyName::F10,
    KeyName::F11,
    KeyName::F12,
];

const TABLES: [KeyTable; 3] = [KeyTable::AppKit, KeyTable::Gdk, KeyTable::Win32];

/// Every modifier combination, as the platform spells it.
fn native_mod_sets(table: KeyTable) -> Vec<(u64, u64)> {
    let rows = modifier_rows(table);
    let mut sets = Vec::new();
    for combo in 0..(1u64 << rows.len()) {
        let mut native = 0;
        let mut kitty = 0;
        for (i, (flag, bit)) in rows.iter().enumerate() {
            if combo & (1 << i) != 0 {
                native |= flag;
                kitty |= bit;
            }
        }
        sets.push((native, kitty));
    }
    sets
}

#[test]
fn the_decoder_reads_every_encoded_key_back() {
    let mut failures = Vec::new();
    for table in TABLES {
        for &(native, _, _) in table_rows(table) {
            for (native_mods, kitty_mods) in native_mod_sets(table) {
                for event in [EVENT_PRESS, EVENT_REPEAT, EVENT_RELEASE] {
                    let bytes = model(table, native, 0, native_mods, event, 0);
                    let got = decode(&bytes, EVENTS | FINAL);
                    let ok = matches!(&got, Outcome::Token(t)
                        if t.consumed == bytes.len()
                            && t.key == expected_key(table, native)
                            && t.event == key_event(event)
                            && t.mods == key_mods(kitty_mods));
                    if !ok {
                        failures.push(format!(
                            "{table:?} {native:#x} mods {native_mods:#x} event {event}: {:?} -> {got:?}",
                            String::from_utf8_lossy(&bytes)
                        ));
                    }
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} encoded key(s) do not decode back:\n{}",
        failures.len(),
        failures[..failures.len().min(20)].join("\n")
    );
}

#[test]
fn the_decoder_reads_every_encoded_text_key_back() {
    // (base, typed text, the grapheme a reader gets) — the text wins when there is
    // some; a release and a Ctrl/Super chord type none.
    let keys: &[(u64, u64)] = &[
        (u64::from(b'a'), u64::from(b'a')),
        (u64::from(b'a'), u64::from(b'A')),
        (u64::from(b'1'), u64::from(b'!')),
        (u64::from(b'e'), 0xE9),
        (0xE9, 0xE9),
        (0x4E2D, 0x4E2D),
        (0x1F600, 0x1F600),
        (u64::from(b' '), u64::from(b' ')),
    ];
    let mut failures = Vec::new();
    for &(base, text) in keys {
        for (native_mods, kitty_mods) in native_mod_sets(KeyTable::Win32) {
            for event in [EVENT_PRESS, EVENT_REPEAT, EVENT_RELEASE] {
                let bytes = model(KeyTable::Win32, 0, base, native_mods, event, text);
                let got = decode(&bytes, EVENTS | FINAL);
                let typed = event != EVENT_RELEASE && kitty_mods & (KITTY_CTRL | KITTY_SUPER) == 0;
                let ok = match &got {
                    Outcome::Token(t) => {
                        t.consumed == bytes.len()
                            && t.key == KeyName::Unicode
                            && t.event == key_event(event)
                            && t.mods == key_mods(kitty_mods)
                            && (!typed
                                || (t.grapheme
                                    == char::from_u32(text as u32)
                                        .unwrap()
                                        .to_string()
                                        .into_bytes()
                                    && t.text == Text::Grapheme))
                    }
                    _ => false,
                };
                if !ok {
                    failures.push(format!(
                        "base {base:#x} text {text:#x} mods {kitty_mods} event {event}: {:?} -> {got:?}",
                        String::from_utf8_lossy(&bytes)
                    ));
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} text key(s) do not decode back:\n{}",
        failures.len(),
        failures[..failures.len().min(20)].join("\n")
    );
}

// --- The emitted encoder ------------------------------------------------------

struct NoPlatform;
impl Externals for NoPlatform {
    fn call(&mut self, _: &mut Machine, _: &str) -> Option<u64> {
        None
    }
}

fn app_dump(target: crate::testutil::CodeTarget) -> serde_json::Value {
    let plan = crate::testutil::app_code_cached(
        "IMPORT io\n\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n",
        target,
    );
    serde_json::from_str(&plan.to_json()).expect("the ncode dump is JSON")
}

/// The input grid: every table row and a spread of text keys, under a spread
/// of modifiers and every event.
fn grid() -> Vec<(KeyTable, u64, u64, u64, u64, u64)> {
    let mut inputs = Vec::new();
    for table in TABLES {
        let sets = native_mod_sets(table);
        for &(native, _, _) in table_rows(table) {
            for (i, (native_mods, _)) in sets.iter().enumerate() {
                if i % 3 != 0 && i != sets.len() - 1 {
                    continue;
                }
                for event in [EVENT_PRESS, EVENT_REPEAT, EVENT_RELEASE] {
                    inputs.push((table, native, 0, *native_mods, event, 0));
                }
            }
        }
        for &(base, text) in &[
            (0u64, 0u64),
            (u64::from(b'a'), u64::from(b'a')),
            (u64::from(b'A'), u64::from(b'A')),
            (u64::from(b'z'), 1),
            (0xE9, 0xE9),
            (0x1F600, 0x1F600),
            (0xF746, 0),
            (0x10, 0),
            (u64::from(b'q'), 0xD800),
            (u64::from(b'q'), 0x110000),
        ] {
            for (native_mods, _) in &sets {
                for event in [EVENT_PRESS, EVENT_REPEAT, EVENT_RELEASE] {
                    // A native code outside every table, so this is a text key.
                    inputs.push((table, 0x7FFF_0000, base, *native_mods, event, text));
                }
            }
        }
    }
    inputs
}

#[test]
fn the_emitted_encoder_matches_the_model_on_every_app_backend() {
    use crate::testutil::CodeTarget;
    for target in [
        CodeTarget::MacosAarch64,
        CodeTarget::LinuxAarch64,
        CodeTarget::LinuxX86_64,
        CodeTarget::WindowsX86_64,
    ] {
        let dump: &'static serde_json::Value = Box::leak(Box::new(app_dump(target)));
        let mut machine = Machine::new(dump);
        assert!(
            machine.has_function(INPUT_ENCODE_SYMBOL),
            "{}: the app build emits no {INPUT_ENCODE_SYMBOL}",
            target.name()
        );
        let out = machine.alloc(ENCODE_OUT_BYTES);
        let mut failures = Vec::new();
        for (table, native, base, mods, event, text) in grid() {
            machine.write_bytes(out, &[0xAA; ENCODE_OUT_BYTES]);
            machine.call(
                INPUT_ENCODE_SYMBOL,
                &[
                    out,
                    native | (table as u64) << PACK_TABLE_SHIFT,
                    base | event << PACK_EVENT_SHIFT | mods << PACK_MODS_SHIFT,
                    text,
                ],
                &mut NoPlatform,
            );
            let len = machine.read_uint(out, 1) as usize;
            let got = machine.read_bytes(out + 1, len);
            let want = model(table, native, base, mods, event, text);
            if got != want && failures.len() < 20 {
                failures.push(format!(
                    "{table:?} native {native:#x} base {base:#x} mods {mods:#x} event {event} text {text:#x}:\n    want {:?}\n    got  {:?}",
                    String::from_utf8_lossy(&want),
                    String::from_utf8_lossy(&got)
                ));
            }
        }
        assert!(
            failures.is_empty(),
            "{}: the emitted encoder disagrees with the model:\n{}",
            target.name(),
            failures.join("\n")
        );
    }
}
