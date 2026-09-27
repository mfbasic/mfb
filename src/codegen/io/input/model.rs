//! The input decoder, written in Rust (test only).
//!
//! This is the specification of `_mfb_rt_input_decode` in executable form: the
//! emitted decoder ([`super::decode`]) is a transliteration of it, and the tests
//! run BOTH against the corpus and then against each other on random input, so a
//! transliteration slip shows up as a disagreement on bytes nobody thought to
//! write a case for. (`.ai/codegen-invariants.md`: "write the algorithm in Rust
//! under `cfg(test)`, pin it against a real oracle, then transliterate".)
//!
//! Grapheme clustering here is `unicode-segmentation`, NOT the UAX #29 rules the
//! emitted decoder carries — two independent implementations, so a disagreement
//! on a cluster is a real finding in one of them.

use super::keys::*;
use unicode_segmentation::UnicodeSegmentation;

/// `flags` bit: no more input will arrive for this decision — the escape delay
/// expired, the stream ended, or (pipe/file) the read-ahead found nothing more.
pub(crate) const FINAL: u64 = 1;
/// `flags` bit: the source reports key events (an `--app` surface, or a console
/// with the kitty protocol on), so a sequence with no event field is a Press.
pub(crate) const EVENTS: u64 = 2;
/// `flags` bit: return a whole grapheme cluster for text (`readGrapheme`,
/// `readKey`), not one scalar (`readScalar`).
pub(crate) const CLUSTER: u64 = 4;

/// What a text reader receives from a token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Text {
    /// The token's grapheme, as is.
    Grapheme,
    /// One scalar that differs from the grapheme: Ctrl+A's U+0001.
    Scalar(u8),
    /// Nothing: a key with no Unicode value, or a release.
    None,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Token {
    pub(crate) consumed: usize,
    pub(crate) key: KeyName,
    pub(crate) event: KeyEvent,
    pub(crate) mods: u64,
    pub(crate) grapheme: Vec<u8>,
    pub(crate) text: Text,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// The bytes so far are a prefix of something longer; ask again with more
    /// (or with `FINAL`).
    NeedMore,
    Token(Token),
    /// Drop this many bytes: invalid UTF-8, or a well-formed sequence that names
    /// nothing a program can receive (a mouse report, a device response, F13).
    Skip(usize),
}

use super::{CSI_PARAM_CAP, GRAPHEME_CAP};

fn event_default(flags: u64) -> KeyEvent {
    if flags & EVENTS != 0 {
        KeyEvent::Press
    } else {
        KeyEvent::NotSupported
    }
}

fn named(consumed: usize, key: KeyName, event: KeyEvent, mods: u64) -> Token {
    let text = named_key_text(key);
    Token {
        consumed,
        key,
        event,
        mods,
        grapheme: text.map(|b| vec![b]).unwrap_or_default(),
        text: if text.is_some() && event != KeyEvent::Release {
            Text::Grapheme
        } else {
            Text::None
        },
    }
}

/// A Unicode-key token for `grapheme`, with the text a text reader gets.
fn unicode(consumed: usize, grapheme: Vec<u8>, event: KeyEvent, mods: u64) -> Token {
    let text = if event == KeyEvent::Release {
        Text::None
    } else if mods & MOD_CTRL != 0 && grapheme.len() == 1 {
        match ctrl_code(grapheme[0]) {
            Some(code) => Text::Scalar(code),
            None => Text::Grapheme,
        }
    } else {
        Text::Grapheme
    };
    Token {
        consumed,
        key: KeyName::Unicode,
        event,
        mods,
        grapheme,
        text,
    }
}

/// The length of the UTF-8 scalar starting at `bytes[0]`: `Ok(len)` when whole
/// and valid, `Err(Some(n))` for an invalid maximal subpart of `n` bytes (drop
/// it), `Err(None)` for a valid but truncated prefix.
fn scalar_len(bytes: &[u8]) -> Result<usize, Option<usize>> {
    let lead = bytes[0];
    let (len, lo, hi) = match lead {
        0x00..=0x7F => return Ok(1),
        0xC2..=0xDF => (2, 0x80, 0xBF),
        0xE0 => (3, 0xA0, 0xBF),
        0xE1..=0xEC | 0xEE..=0xEF => (3, 0x80, 0xBF),
        0xED => (3, 0x80, 0x9F),
        0xF0 => (4, 0x90, 0xBF),
        0xF1..=0xF3 => (4, 0x80, 0xBF),
        0xF4 => (4, 0x80, 0x8F),
        _ => return Err(Some(1)),
    };
    for i in 1..len {
        let Some(&b) = bytes.get(i) else {
            return Err(None);
        };
        let (l, h) = if i == 1 { (lo, hi) } else { (0x80, 0xBF) };
        if b < l || b > h {
            return Err(Some(i));
        }
    }
    Ok(len)
}

/// Decode one token from the front of `buf`.
pub(crate) fn decode(buf: &[u8], flags: u64) -> Outcome {
    if buf.is_empty() {
        return Outcome::NeedMore;
    }
    let b0 = buf[0];
    match b0 {
        0x1B => escape(buf, flags),
        0x0D => {
            if buf.len() >= 2 {
                let n = if buf[1] == 0x0A { 2 } else { 1 };
                Outcome::Token(named(n, KeyName::Enter, KeyEvent::NotSupported, 0))
            } else if flags & FINAL != 0 {
                Outcome::Token(named(1, KeyName::Enter, KeyEvent::NotSupported, 0))
            } else {
                Outcome::NeedMore
            }
        }
        0x00..=0x1F | 0x7F => {
            let (key, grapheme, mods) = c0_key(b0);
            if key == KeyName::Unicode {
                let mut token = unicode(1, vec![grapheme], KeyEvent::NotSupported, mods);
                // The C0 byte IS the text (Decision 2), whatever the key was.
                token.text = Text::Scalar(b0);
                Outcome::Token(token)
            } else {
                Outcome::Token(named(1, key, KeyEvent::NotSupported, 0))
            }
        }
        _ => text(buf, flags),
    }
}

/// Plain UTF-8 text: one scalar, or (CLUSTER) one grapheme cluster.
fn text(buf: &[u8], flags: u64) -> Outcome {
    let len = match scalar_len(buf) {
        Ok(len) => len,
        Err(Some(n)) => return Outcome::Skip(n),
        Err(None) if flags & FINAL != 0 => return Outcome::Skip(buf.len()),
        Err(None) => return Outcome::NeedMore,
    };
    if flags & CLUSTER == 0 {
        return Outcome::Token(unicode(len, buf[..len].to_vec(), KeyEvent::NotSupported, 0));
    }
    // The run a cluster can come from: the valid, non-control text that has
    // arrived. It stops at a control byte (every cluster breaks before one,
    // UAX #29 GB5), at an invalid byte, or at a truncated scalar.
    let mut run_end = len;
    let mut truncated = false;
    while run_end < buf.len() {
        let b = buf[run_end];
        if b < 0x20 || b == 0x7F {
            break;
        }
        match scalar_len(&buf[run_end..]) {
            Ok(n) => run_end += n,
            Err(Some(_)) => break,
            Err(None) => {
                truncated = true;
                break;
            }
        }
    }
    let run = std::str::from_utf8(&buf[..run_end]).expect("validated run");
    let first = run.graphemes(true).next().expect("non-empty run");
    let mut end = len;
    for c in first.chars().skip(1) {
        let n = c.len_utf8();
        if end + n > GRAPHEME_CAP {
            // Cut at the cap: the rest is the next token.
            return Outcome::Token(unicode(end, buf[..end].to_vec(), KeyEvent::NotSupported, 0));
        }
        end += n;
    }
    let open_ended = end == run_end && (run_end == buf.len() || truncated);
    if open_ended && flags & FINAL == 0 {
        // The cluster runs to the end of what has arrived: more of it may follow.
        return Outcome::NeedMore;
    }
    Outcome::Token(unicode(end, buf[..end].to_vec(), KeyEvent::NotSupported, 0))
}

fn escape(buf: &[u8], flags: u64) -> Outcome {
    if buf.len() == 1 {
        return if flags & FINAL != 0 {
            Outcome::Token(named(1, KeyName::Escape, KeyEvent::NotSupported, 0))
        } else {
            Outcome::NeedMore
        };
    }
    match buf[1] {
        b'[' => csi(buf, flags),
        b'O' => ss3(buf, flags),
        _ => alt_prefixed(buf, flags),
    }
}

/// `ESC <token>`: the token with Alt (a legacy terminal's Meta-sends-Escape).
fn alt_prefixed(buf: &[u8], flags: u64) -> Outcome {
    match decode(&buf[1..], flags) {
        Outcome::NeedMore => Outcome::NeedMore,
        Outcome::Token(mut token) => {
            token.consumed += 1;
            token.mods |= MOD_ALT;
            Outcome::Token(token)
        }
        // ESC before bytes that are not a key: the ESC is the Escape key.
        Outcome::Skip(_) => Outcome::Token(named(1, KeyName::Escape, KeyEvent::NotSupported, 0)),
    }
}

/// `ESC [` or `ESC O` with nothing after it and no more coming: Alt+`[` / Alt+`O`.
fn alt_introducer(buf: &[u8]) -> Outcome {
    Outcome::Token(unicode(2, vec![buf[1]], KeyEvent::NotSupported, MOD_ALT))
}

/// The parsed parameters of a CSI: `;`-separated fields of `:`-separated
/// decimal subfields, each `None` when empty.
fn parse_params(params: &[u8]) -> Vec<Vec<Option<u64>>> {
    params
        .split(|b| *b == b';')
        .map(|field| {
            field
                .split(|b| *b == b':')
                .map(|sub| {
                    if sub.is_empty() {
                        None
                    } else {
                        Some(sub.iter().fold(0u64, |acc, d| {
                            (acc * 10 + u64::from(d - b'0')).min(1_000_000_000)
                        }))
                    }
                })
                .collect()
        })
        .collect()
}

fn field(params: &[Vec<Option<u64>>], i: usize, j: usize) -> Option<u64> {
    params.get(i).and_then(|f| f.get(j)).copied().flatten()
}

/// The event a modifier field's `:event` subfield names; `Err` for a value the
/// protocol does not define.
fn event_of(params: &[Vec<Option<u64>>], flags: u64) -> Result<KeyEvent, ()> {
    match field(params, 1, 1) {
        None => Ok(event_default(flags)),
        Some(1) => Ok(KeyEvent::Press),
        Some(2) => Ok(KeyEvent::Repeat),
        Some(3) => Ok(KeyEvent::Release),
        Some(_) => Err(()),
    }
}

fn csi(buf: &[u8], flags: u64) -> Outcome {
    // Linux console F1–F5: `ESC [ [ A`–`E`.
    if buf.get(2) == Some(&b'[') {
        return match buf.get(3) {
            None if flags & FINAL != 0 => Outcome::Skip(3),
            None => Outcome::NeedMore,
            Some(b) => match LINUX_CONSOLE_F_KEYS.iter().find(|(c, _)| c == b) {
                Some((_, key)) => Outcome::Token(named(4, *key, KeyEvent::NotSupported, 0)),
                None => Outcome::Skip(3),
            },
        };
    }
    let mut j = 2;
    while j < buf.len() && (0x30..=0x3F).contains(&buf[j]) {
        j += 1;
        if j - 2 > CSI_PARAM_CAP {
            return Outcome::Skip(j);
        }
    }
    let params_end = j;
    // Intermediates, except `$`: rxvt ends a Shift+key sequence with it.
    while j < buf.len() && (0x20..=0x2F).contains(&buf[j]) && buf[j] != b'$' {
        j += 1;
    }
    if j == buf.len() {
        if flags & FINAL == 0 {
            return Outcome::NeedMore;
        }
        return if j == 2 {
            alt_introducer(buf)
        } else {
            Outcome::Skip(j)
        };
    }
    let fin = buf[j];
    if !(0x40..=0x7E).contains(&fin) && fin != b'$' {
        // Not a CSI after all: drop what was read, resume at the stray byte.
        return Outcome::Skip(j);
    }
    let consumed = j + 1;
    let raw = &buf[2..params_end];
    if params_end != j || raw.iter().any(|b| (0x3C..=0x3F).contains(b)) {
        // Intermediates, or a private marker: a device response or a report the
        // mouse decoder did not claim. Nothing a program types.
        return Outcome::Skip(consumed);
    }
    let params = parse_params(raw);
    let mods = modifier_bits(field(&params, 1, 0).unwrap_or(1));
    let Ok(event) = event_of(&params, flags) else {
        return Outcome::Skip(consumed);
    };
    if let Some((_, key)) = CSI_LETTER_KEYS.iter().find(|(c, _)| *c == fin) {
        return Outcome::Token(named(consumed, *key, event, mods));
    }
    if let Some((_, key)) = RXVT_ARROWS.iter().find(|(c, _)| *c == fin) {
        return Outcome::Token(named(consumed, *key, KeyEvent::NotSupported, MOD_SHIFT));
    }
    match fin {
        b'Z' => Outcome::Token(named(consumed, KeyName::Tab, event, mods | MOD_SHIFT)),
        b'~' | b'$' | b'^' | b'@' => {
            let n = field(&params, 0, 0).unwrap_or(0);
            if fin == b'~' && n == TILDE_MODIFY_OTHER_KEYS {
                let m = modifier_bits(field(&params, 1, 0).unwrap_or(1));
                return match field(&params, 2, 0) {
                    Some(code) => code_token(consumed, code, None, event, m),
                    None => Outcome::Skip(consumed),
                };
            }
            let Some((_, key)) = TILDE_KEYS.iter().find(|(k, _)| *k == n) else {
                return Outcome::Skip(consumed);
            };
            let (mods, event) = match fin {
                b'~' => (mods, event),
                b'$' => (MOD_SHIFT, KeyEvent::NotSupported),
                b'^' => (MOD_CTRL, KeyEvent::NotSupported),
                _ => (MOD_CTRL | MOD_SHIFT, KeyEvent::NotSupported),
            };
            Outcome::Token(named(consumed, *key, event, mods))
        }
        b'u' => {
            let Some(code) = field(&params, 0, 0) else {
                return Outcome::Skip(consumed);
            };
            let shifted = field(&params, 0, 1);
            let text: Vec<u64> = params
                .get(2)
                .map(|f| f.iter().flatten().copied().collect())
                .unwrap_or_default();
            kitty_token(consumed, code, shifted, &text, event, mods)
        }
        _ => Outcome::Skip(consumed),
    }
}

/// The UTF-8 of `cp`, or `None` for a surrogate or a value past U+10FFFF.
fn utf8(cp: u64) -> Option<Vec<u8>> {
    let c = char::from_u32(u32::try_from(cp).ok()?)?;
    Some(c.to_string().into_bytes())
}

/// A token for the Unicode key whose (unshifted) code point is `code`.
fn code_token(
    consumed: usize,
    code: u64,
    shifted: Option<u64>,
    event: KeyEvent,
    mods: u64,
) -> Outcome {
    if let Some((_, what)) = KITTY_CODES.iter().find(|(c, _)| *c == code) {
        return match what {
            KittyCode::Named(key) => Outcome::Token(named(consumed, *key, event, mods)),
            KittyCode::Text(b) => Outcome::Token(unicode(consumed, vec![*b], event, mods)),
        };
    }
    if (KITTY_PUA_FIRST..=KITTY_PUA_LAST).contains(&code) || code < 0x20 {
        return Outcome::Skip(consumed);
    }
    // The grapheme is what the key types. With Shift that is the shifted key:
    // the alternate-key subfield when the source sent one, else the ASCII
    // upper case (a source that sends neither text nor alternates gives no
    // better answer).
    let cp = if mods & MOD_SHIFT != 0 {
        match shifted {
            Some(s) => s,
            None if (u64::from(b'a')..=u64::from(b'z')).contains(&code) => code - 32,
            None => code,
        }
    } else {
        code
    };
    match utf8(cp) {
        Some(g) => Outcome::Token(unicode(consumed, g, event, mods)),
        None => Outcome::Skip(consumed),
    }
}

fn kitty_token(
    consumed: usize,
    code: u64,
    shifted: Option<u64>,
    text: &[u64],
    event: KeyEvent,
    mods: u64,
) -> Outcome {
    if !text.is_empty() {
        let mut grapheme = Vec::new();
        for cp in text {
            match utf8(*cp) {
                Some(g) if grapheme.len() + g.len() <= GRAPHEME_CAP => grapheme.extend(g),
                Some(_) => break,
                None => return Outcome::Skip(consumed),
            }
        }
        // Associated text names a Unicode key even for a keypad digit.
        if code == 0
            || !KITTY_CODES
                .iter()
                .any(|(c, k)| *c == code && matches!(k, KittyCode::Named(_)))
        {
            return Outcome::Token(unicode(consumed, grapheme, event, mods));
        }
    }
    if code == 0 {
        return Outcome::Skip(consumed);
    }
    code_token(consumed, code, shifted, event, mods)
}

fn ss3(buf: &[u8], flags: u64) -> Outcome {
    let mut j = 2;
    while j < buf.len() && buf[j].is_ascii_digit() {
        j += 1;
    }
    if j == buf.len() {
        if flags & FINAL == 0 {
            return Outcome::NeedMore;
        }
        return if j == 2 {
            alt_introducer(buf)
        } else {
            Outcome::Skip(j)
        };
    }
    let fin = buf[j];
    let consumed = j + 1;
    let digits = &buf[2..j];
    let mods = if digits.is_empty() {
        0
    } else {
        modifier_bits(
            digits
                .iter()
                .fold(0u64, |a, d| (a * 10 + u64::from(d - b'0')).min(1_000_000)),
        )
    };
    if let Some((_, key)) = SS3_KEYS.iter().find(|(c, _)| *c == fin) {
        return Outcome::Token(named(consumed, *key, KeyEvent::NotSupported, mods));
    }
    if let Some((_, key)) = RXVT_ARROWS.iter().find(|(c, _)| *c == fin) {
        return Outcome::Token(named(consumed, *key, KeyEvent::NotSupported, MOD_CTRL));
    }
    if let Some((_, b)) = SS3_KEYPAD_TEXT.iter().find(|(c, _)| *c == fin) {
        return Outcome::Token(unicode(consumed, vec![*b], KeyEvent::NotSupported, mods));
    }
    if (0x40..=0x7E).contains(&fin) {
        Outcome::Skip(consumed)
    } else {
        // `ESC O` then something that is no SS3 final: Alt+O, then that byte.
        alt_introducer(buf)
    }
}
