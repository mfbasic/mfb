//! bug-726: `term::drawText`'s `AttributedString` overload never splits a grapheme
//! cluster. It draws in same-style runs, and a run boundary used to fall inside a
//! cluster whenever the styling changed there: the decomposed `e` + U+0301 with
//! bold on the combining mark only was drawn as `e`, an attribute switch, and the
//! lone mark in the NEXT cell. A cluster now takes the style of its first scalar,
//! so it is stamped whole, exactly as the `String` overload stamps it.

#[path = "../common/mod.rs"]
mod common;
use common::*;

const PROGRAM: &str = "IMPORT term\nIMPORT astrings\n\nFUNC main AS Integer\n  \
LET plain AS String = \"e\\u{301}\"\n  \
LET a AS AttributedString = astrings::fromString(plain)\n  \
LET styled AS AttributedString = astrings::addAttribute(a, 1, 1, astrings::bold())\n  \
term::on()\n  term::clear()\n  term::drawText(0, 0, styled)\n  \
term::drawText(0, 1, \"|\")\n  term::sync()\n  term::off()\n  RETURN 0\nEND FUNC\n";

#[test]
fn a_style_change_inside_a_cluster_does_not_split_it() {
    let exe = build_project(&temp_project("bug726_attr_grapheme", PROGRAM));
    let bytes = run_under_pty_bytes(&exe);
    let cluster = b"e\xcc\x81";
    assert!(
        bytes.windows(cluster.len()).any(|w| w == cluster),
        "the cluster e+U+0301 must reach the terminal as adjacent bytes (one cell)"
    );
    // The `|` drawn at column 1 must follow the whole cluster in the same row, not
    // overwrite a cell the combining mark was pushed into.
    let text = String::from_utf8_lossy(&bytes);
    assert!(
        text.contains("e\u{301}|"),
        "the cluster must occupy exactly one cell, with `|` in the next"
    );
}
