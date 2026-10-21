//! The input decoder's malformed and truncated sequences.
//!
//! `model.rs` is the executable specification of `_mfb_rt_input_decode`, and the corpus
//! and the random cross-check pin the emitted decoder against it. Both reach the
//! sequences a terminal sends; the decisions that matter in a real terminal are the
//! ones for what it SHOULD NOT send or has not finished sending: a carriage return
//! with nothing after it yet, a Linux-console function key cut short, a parameter list
//! longer than any key's, a modify-other-keys report with no key in it, a text field
//! past the grapheme cap or naming a surrogate. For each, a program must neither hang
//! waiting for bytes that will never come nor treat garbage as a key, so each is
//! stated: ask for more, drop exactly these bytes, or report this key.

use crate::codegen::io::input::keys::KeyName;
use crate::codegen::io::input::model::{decode, Outcome, FINAL};

/// A CR with nothing after it is a line ending only once no more is coming.
#[test]
fn a_lone_carriage_return_waits_for_a_possible_line_feed() {
    assert_eq!(decode(b"\r", 0), Outcome::NeedMore);
    let Outcome::Token(token) = decode(b"\r", FINAL) else {
        panic!("a final CR is Enter");
    };
    assert_eq!((token.consumed, token.key), (1, KeyName::Enter));
}

/// `ESC [ [` is the Linux console's F1-F5 prefix: cut short it drops three bytes when
/// nothing more is coming, and a letter that is no function key drops the same three.
#[test]
fn a_cut_short_or_unknown_linux_console_key_drops_the_prefix() {
    assert_eq!(decode(b"\x1b[[", 0), Outcome::NeedMore);
    assert_eq!(decode(b"\x1b[[", FINAL), Outcome::Skip(3));
    assert_eq!(decode(b"\x1b[[Z", 0), Outcome::Skip(3));
}

/// A CSI whose parameter list outgrows the cap is dropped at the cap, not scanned to
/// its end: a stream of digits cannot make the decoder buffer without bound.
#[test]
fn an_overlong_parameter_list_is_dropped_at_the_cap() {
    let mut sequence = b"\x1b[".to_vec();
    sequence.extend(std::iter::repeat_n(b'1', 80));
    let Outcome::Skip(dropped) = decode(&sequence, 0) else {
        panic!("a CSI with 80 parameter digits must be dropped");
    };
    assert!(
        (60..=70).contains(&dropped),
        "dropped at the parameter cap, not at the end of the input: {dropped}"
    );
}

/// A modify-other-keys report (`CSI 27 ; mods ; code ~`) with no key code names nothing,
/// and neither does a key code below space or in the private-use block.
#[test]
fn a_modify_other_keys_report_with_no_usable_code_is_dropped() {
    assert_eq!(decode(b"\x1b[27;5~", 0), Outcome::Skip(7));
    assert_eq!(decode(b"\x1b[27;5;1~", 0), Outcome::Skip(9));
    assert_eq!(decode(b"\x1b[5u", 0), Outcome::Skip(4));
    assert_eq!(decode(b"\x1b[0u", 0), Outcome::Skip(4));
}

/// A kitty report's associated text is refused when it names a surrogate, and ignored
/// for a named key (Enter keeps being Enter).
#[test]
fn a_kitty_text_field_is_refused_or_ignored() {
    // A surrogate is not a scalar.
    assert_eq!(
        decode(b"\x1b[97;;55296u", 0),
        Outcome::Skip(12),
        "associated text naming a surrogate is dropped"
    );
    // Enter with associated text is still Enter.
    let Outcome::Token(token) = decode(b"\x1b[13;;65u", 0) else {
        panic!("a named key with associated text is that key");
    };
    assert_eq!(token.key, KeyName::Enter);
}

/// An SS3 sequence with digits and nothing after, when no more is coming, drops what
/// it read; a final byte that is no SS3 key drops through the final.
#[test]
fn a_truncated_or_unknown_ss3_sequence_is_dropped() {
    assert_eq!(decode(b"\x1bO5", FINAL), Outcome::Skip(3));
    assert_eq!(decode(b"\x1bO~", 0), Outcome::Skip(3));
}

/// A lone C0 byte is a named key or Ctrl+<key>, and anything else is not a C0 key:
/// `c0_key` answers every C0 byte but ESC (which starts a sequence and never reaches
/// it), and a caller that hands it a printable byte is a decoder bug it refuses to
/// paper over with an invented key.
#[test]
fn a_lone_c0_byte_names_its_key_and_nothing_else_is_one() {
    use crate::codegen::io::input::keys::{c0_key, MOD_CTRL};
    assert_eq!(c0_key(0x08), (KeyName::Backspace, 0x7F, 0));
    assert_eq!(c0_key(0x7F), (KeyName::Backspace, 0x7F, 0));
    assert_eq!(c0_key(0x0D), (KeyName::Enter, b'\n', 0));
    assert_eq!(c0_key(0x01), (KeyName::Unicode, b'a', MOD_CTRL));
    assert_eq!(c0_key(0x1F), (KeyName::Unicode, b'_', MOD_CTRL));
    let refused = crate::testutil::silence_panics(|| std::panic::catch_unwind(|| c0_key(b'A')));
    assert!(refused.is_err(), "`A` is not a C0 byte");
}
