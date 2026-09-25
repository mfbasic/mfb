//! The key vocabulary the input decoder speaks (bug-699), and the tables that
//! map every accepted escape-sequence spelling onto it.
//!
//! These tables are the single source of truth for "which bytes mean which
//! key": the emitted decoder ([`super::decode`]) is generated from them, and the
//! reference model the corpus test runs is written against them too. The corpus
//! itself (`super::corpus`) does NOT read them — its expected values are written
//! out by hand from the protocol documents, which is what makes it an oracle
//! rather than an echo.
//!
//! Sources, for every row below:
//! - kitty keyboard protocol, <https://sw.kovidgoyal.net/kitty/keyboard-protocol/>
//!   ("Legacy functional keys", "Functional key definitions", "Modifiers",
//!   "Event types");
//! - xterm "Control Sequences" (PC-style function keys, `modifyOtherKeys`);
//! - the rxvt/VT220 and Linux-console spellings the bug doc's Goal table names.

/// The key a token names. The ORDER is the `io::KeyName` enum's declaration
/// order (bug-699 Decision 4), because the decoder stores the ordinal and the
/// reader that builds an `io::Key` record (Phase 4) stores it unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum KeyName {
    Unicode = 0,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Delete,
    Backspace,
    Enter,
    Tab,
    Escape,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
}

/// The event a token reports, in `io::KeyEvent` declaration order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum KeyEvent {
    Press = 0,
    Repeat,
    Release,
    NotSupported,
}

/// Modifier bits in a token (`io::Key`'s four Booleans).
pub(crate) const MOD_SHIFT: u64 = 1;
pub(crate) const MOD_ALT: u64 = 2;
pub(crate) const MOD_CTRL: u64 = 4;
pub(crate) const MOD_META: u64 = 8;

/// `CSI [1;m] <letter>` — the arrow/Home/End/F1–F4 form shared by xterm and
/// kitty. `R` is F3 only here: kitty dropped `CSI R` because it collides with a
/// cursor-position report, but xterm still sends `CSI 1;m R` for a modified F3,
/// and the runtime never requests a position report, so nothing else can
/// produce it.
pub(crate) const CSI_LETTER_KEYS: &[(u8, KeyName)] = &[
    (b'A', KeyName::Up),
    (b'B', KeyName::Down),
    (b'C', KeyName::Right),
    (b'D', KeyName::Left),
    (b'H', KeyName::Home),
    (b'F', KeyName::End),
    (b'P', KeyName::F1),
    (b'Q', KeyName::F2),
    (b'R', KeyName::F3),
    (b'S', KeyName::F4),
];

/// `CSI n [;m] ~` (VT220/xterm/kitty) — and the rxvt `$`/`^`/`@` finals, which
/// use the same numbers. 1/4 are VT220 Home/End, 7/8 rxvt's; 11–14 are the
/// VT220/rxvt F1–F4 (kitty's alternates for F1/F2/F4, and its only F3).
pub(crate) const TILDE_KEYS: &[(u64, KeyName)] = &[
    (1, KeyName::Home),
    (2, KeyName::Insert),
    (3, KeyName::Delete),
    (4, KeyName::End),
    (5, KeyName::PageUp),
    (6, KeyName::PageDown),
    (7, KeyName::Home),
    (8, KeyName::End),
    (11, KeyName::F1),
    (12, KeyName::F2),
    (13, KeyName::F3),
    (14, KeyName::F4),
    (15, KeyName::F5),
    (17, KeyName::F6),
    (18, KeyName::F7),
    (19, KeyName::F8),
    (20, KeyName::F9),
    (21, KeyName::F10),
    (23, KeyName::F11),
    (24, KeyName::F12),
];

/// xterm `modifyOtherKeys`: `CSI 27 ; m ; code ~` is the key `code` with
/// modifier `m`.
pub(crate) const TILDE_MODIFY_OTHER_KEYS: u64 = 27;

/// `ESC O <letter>` (SS3): cursor-key application mode, and F1–F4.
pub(crate) const SS3_KEYS: &[(u8, KeyName)] = &[
    (b'A', KeyName::Up),
    (b'B', KeyName::Down),
    (b'C', KeyName::Right),
    (b'D', KeyName::Left),
    (b'H', KeyName::Home),
    (b'F', KeyName::End),
    (b'P', KeyName::F1),
    (b'Q', KeyName::F2),
    (b'R', KeyName::F3),
    (b'S', KeyName::F4),
    (b'M', KeyName::Enter),
    (b'I', KeyName::Tab),
];

/// `ESC O <letter>` in keypad application mode: the keypad's text keys. The
/// letter is the keypad key; the byte is the text it types.
pub(crate) const SS3_KEYPAD_TEXT: &[(u8, u8)] = &[
    (b'j', b'*'),
    (b'k', b'+'),
    (b'l', b','),
    (b'm', b'-'),
    (b'n', b'.'),
    (b'o', b'/'),
    (b'p', b'0'),
    (b'q', b'1'),
    (b'r', b'2'),
    (b's', b'3'),
    (b't', b'4'),
    (b'u', b'5'),
    (b'v', b'6'),
    (b'w', b'7'),
    (b'x', b'8'),
    (b'y', b'9'),
    (b'X', b'='),
];

/// rxvt: `ESC [ a`–`d` is Shift+arrow; `ESC O a`–`d` is Ctrl+arrow.
pub(crate) const RXVT_ARROWS: &[(u8, KeyName)] = &[
    (b'a', KeyName::Up),
    (b'b', KeyName::Down),
    (b'c', KeyName::Right),
    (b'd', KeyName::Left),
];

/// Linux console: `ESC [ [ A`–`E` are F1–F5.
pub(crate) const LINUX_CONSOLE_F_KEYS: &[(u8, KeyName)] = &[
    (b'A', KeyName::F1),
    (b'B', KeyName::F2),
    (b'C', KeyName::F3),
    (b'D', KeyName::F4),
    (b'E', KeyName::F5),
];

/// What a kitty `CSI code … u` key code names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum KittyCode {
    Named(KeyName),
    /// A keypad key that types this ASCII text.
    Text(u8),
}

/// kitty's `CSI … u` numbers that name a key rather than carry a code point:
/// the four C0 keys, and the keypad block of the Private Use Area. Every other
/// Private Use Area number (57344–63743) — lock keys, modifier keys, media keys,
/// F13 and up — names a key `io::KeyName` has no member for, and its event is
/// dropped.
pub(crate) const KITTY_CODES: &[(u64, KittyCode)] = &[
    (9, KittyCode::Named(KeyName::Tab)),
    (13, KittyCode::Named(KeyName::Enter)),
    (27, KittyCode::Named(KeyName::Escape)),
    (127, KittyCode::Named(KeyName::Backspace)),
    (57399, KittyCode::Text(b'0')),
    (57400, KittyCode::Text(b'1')),
    (57401, KittyCode::Text(b'2')),
    (57402, KittyCode::Text(b'3')),
    (57403, KittyCode::Text(b'4')),
    (57404, KittyCode::Text(b'5')),
    (57405, KittyCode::Text(b'6')),
    (57406, KittyCode::Text(b'7')),
    (57407, KittyCode::Text(b'8')),
    (57408, KittyCode::Text(b'9')),
    (57409, KittyCode::Text(b'.')),
    (57410, KittyCode::Text(b'/')),
    (57411, KittyCode::Text(b'*')),
    (57412, KittyCode::Text(b'-')),
    (57413, KittyCode::Text(b'+')),
    (57414, KittyCode::Named(KeyName::Enter)),
    (57415, KittyCode::Text(b'=')),
    (57416, KittyCode::Text(b',')),
    (57417, KittyCode::Named(KeyName::Left)),
    (57418, KittyCode::Named(KeyName::Right)),
    (57419, KittyCode::Named(KeyName::Up)),
    (57420, KittyCode::Named(KeyName::Down)),
    (57421, KittyCode::Named(KeyName::PageUp)),
    (57422, KittyCode::Named(KeyName::PageDown)),
    (57423, KittyCode::Named(KeyName::Home)),
    (57424, KittyCode::Named(KeyName::End)),
    (57425, KittyCode::Named(KeyName::Insert)),
    (57426, KittyCode::Named(KeyName::Delete)),
];

/// kitty's Private Use Area for functional keys.
pub(crate) const KITTY_PUA_FIRST: u64 = 57344;
pub(crate) const KITTY_PUA_LAST: u64 = 63743;

/// The grapheme — and the text a text reader sees — of a named key that has
/// one (bug-699 Decision 2). Every other named key has neither.
pub(crate) fn named_key_text(key: KeyName) -> Option<u8> {
    match key {
        KeyName::Enter => Some(b'\n'),
        KeyName::Tab => Some(b'\t'),
        KeyName::Backspace => Some(0x7F),
        KeyName::Escape => Some(0x1B),
        _ => None,
    }
}

/// The C0 code Ctrl turns an ASCII key into (bug-699 Decision 2: Ctrl+A…Ctrl+Z
/// arrive as U+0001…U+001A). Also the rest of the legacy Ctrl block — `@`/space
/// (NUL), `[`, `\`, `]`, `^`, `_`, `?` (DEL) — because a terminal sends those
/// bytes for those keys and a text reader must see the same scalar whether the
/// key came from a terminal or a kitty-form event.
pub(crate) fn ctrl_code(ascii: u8) -> Option<u8> {
    match ascii {
        b'a'..=b'z' => Some(ascii - b'a' + 1),
        b'A'..=b'Z' => Some(ascii - b'A' + 1),
        b'@' | b' ' => Some(0),
        b'[' => Some(0x1B),
        b'\\' => Some(0x1C),
        b']' => Some(0x1D),
        b'^' => Some(0x1E),
        b'_' => Some(0x1F),
        b'?' => Some(0x7F),
        _ => None,
    }
}

/// The key a terminal meant by a lone C0 byte (other than ESC, which starts a
/// sequence): the four named keys, and Ctrl+<key> for the rest. `0x08` is
/// Backspace (some terminals send it instead of `0x7F`, and Ctrl+Backspace is
/// `0x08` in kitty's legacy table); `CR` is Enter.
pub(crate) fn c0_key(byte: u8) -> (KeyName, u8, u64) {
    match byte {
        0x08 | 0x7F => (KeyName::Backspace, 0x7F, 0),
        0x09 => (KeyName::Tab, b'\t', 0),
        0x0A | 0x0D => (KeyName::Enter, b'\n', 0),
        0x00 => (KeyName::Unicode, b' ', MOD_CTRL),
        0x01..=0x1A => (KeyName::Unicode, byte - 1 + b'a', MOD_CTRL),
        0x1C => (KeyName::Unicode, b'\\', MOD_CTRL),
        0x1D => (KeyName::Unicode, b']', MOD_CTRL),
        0x1E => (KeyName::Unicode, b'^', MOD_CTRL),
        0x1F => (KeyName::Unicode, b'_', MOD_CTRL),
        _ => unreachable!("c0_key called with {byte:#x}"),
    }
}

/// Modifier bits from an xterm/kitty modifier parameter (the model's; the
/// emitter's `decode::emit_modifier_bits` is its transliteration) (`1 + bits`; absent or
/// 0 is none). kitty's `super` (8) and `meta` (32) both land on `io::Key.meta`
/// — the Cmd/Windows key; `hyper`, Caps Lock and Num Lock are not reported.
#[cfg(test)]
pub(crate) fn modifier_bits(parameter: u64) -> u64 {
    let bits = parameter.saturating_sub(1);
    let mut mods = bits & (MOD_SHIFT | MOD_ALT | MOD_CTRL);
    if bits & (8 | 32) != 0 {
        mods |= MOD_META;
    }
    mods
}
