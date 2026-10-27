//! `term::drawText` and `term::drawGlyph` skip every control character, not only
//! the C0 range below U+0020: DEL (U+007F) and the C1 controls U+0080–U+009F are
//! skipped too, each still taking its one column. They used to reach the terminal
//! raw (`7f`, `c2 9b`), and a C1 control such as U+009B (CSI) can start an escape
//! sequence in the presented frame.

#[path = "../common/mod.rs"]
mod common;
use common::*;

const PROGRAM: &str = "IMPORT term\n\nFUNC main AS Integer\n  term::on()\n  \
term::drawText(0, 0, \"a\\u{7f}b\\u{9b}c\\u{85}d\")\n  term::drawGlyph(1, 0, 127)\n  \
term::drawGlyph(1, 1, 155)\n  term::drawGlyph(1, 2, 159)\n  term::drawGlyph(1, 3, 65)\n  \
term::drawGlyph(1, 4, 160)\n  term::sync()\n  term::off()\n  RETURN 0\nEND FUNC\n";

#[test]
fn del_and_c1_controls_reach_the_terminal_as_nothing() {
    let exe = build_project(&temp_project("draw_control_skip", PROGRAM));
    let bytes = run_under_pty_bytes(&exe);
    for bad in [&[0x7fu8][..], &[0xc2, 0x9b], &[0xc2, 0x85], &[0xc2, 0x9f]] {
        assert!(
            !bytes.windows(bad.len()).any(|w| w == bad),
            "the control {bad:02x?} reached the terminal: {:?}",
            String::from_utf8_lossy(&bytes)
        );
    }
    let raw = String::from_utf8_lossy(&bytes);
    // The frame's styling and cursor sequences sit between cells; drop them.
    let mut text = String::new();
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for n in chars.by_ref() {
                if n.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            text.push(c);
        }
    }
    // Each skipped control still takes its column, so the letters keep their
    // places: `a` at column 0, then a blank where DEL was, `b` at column 2.
    assert!(
        text.contains("a b c d"),
        "drawText must leave one blank column per skipped control: {text:?}"
    );
    assert!(text.contains('A'), "the valid glyph U+0041 must still be drawn");
    assert!(
        text.contains('\u{a0}'),
        "U+00A0 is not a control and must still be drawn"
    );
}
