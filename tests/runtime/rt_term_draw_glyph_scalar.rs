//! bug-725: `term::drawGlyph` writes only well-formed UTF-8. A code point above
//! U+10FFFF or in the surrogate band U+D800–U+DFFF is not a Unicode scalar and is
//! skipped, exactly as a control value below U+0020 is; it used to be encoded
//! as-is (`f4 90 80 80`, `ed a0 80`), putting invalid UTF-8 on the terminal.

#[path = "../common/mod.rs"]
mod common;
use common::*;

const PROGRAM: &str = "IMPORT term\n\nFUNC main AS Integer\n  term::on()\n  \
term::drawGlyph(0, 0, 1114112)\n  term::drawGlyph(1, 0, 55296)\n  \
term::drawGlyph(2, 0, 57343)\n  term::drawGlyph(3, 0, 65)\n  term::sync()\n  \
term::off()\n  RETURN 0\nEND FUNC\n";

#[test]
fn non_scalar_code_points_reach_the_terminal_as_nothing() {
    let exe = build_project(&temp_project("bug725_draw_glyph", PROGRAM));
    let bytes = run_under_pty_bytes(&exe);
    if let Err(e) = std::str::from_utf8(&bytes) {
        let at = e.valid_up_to();
        panic!(
            "drawGlyph put invalid UTF-8 on the terminal at byte {at}: {:02x?}",
            &bytes[at..(at + 8).min(bytes.len())]
        );
    }
    for bad in [
        &[0xf4u8, 0x90, 0x80, 0x80][..],
        &[0xed, 0xa0, 0x80],
        &[0xed, 0xbf, 0xbf],
    ] {
        assert!(
            !bytes.windows(bad.len()).any(|w| w == bad),
            "the encoding {bad:02x?} of a non-scalar reached the terminal"
        );
    }
    assert!(
        bytes.contains(&b'A'),
        "the valid glyph U+0041 must still be drawn"
    );
}
