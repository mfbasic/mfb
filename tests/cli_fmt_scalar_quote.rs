//! bug-293: `mfb fmt`'s line scanner did not treat a backtick `Scalar` literal as
//! a literal, so a `` `"` `` or `` `'` `` scalar desynchronized it — an apostrophe
//! scalar turned the rest of the line into a "comment" (dropping keyword casing /
//! corrupting the tail), and a `"` scalar left the following string mis-scanned.
//! The formatter now recognizes scalar literals, so such a line is left intact.
//!
//! This drives the real `mfb fmt` in place over a file with an apostrophe scalar
//! followed by real code and asserts the file is unchanged (idempotent) — the
//! keyword-cased `"if then"` string and the trailing statements survive.

use std::process::Command;

mod common;
use common::*;

#[test]
fn fmt_leaves_scalar_quote_literals_and_their_line_intact() {
    // Already in canonical form: a double-quote Scalar literal, then a second
    // statement whose String contains lowercase keywords. Pre-fix the `` `"` ``
    // scalar opened string mode and the following string's body was scanned as
    // code, uppercasing `if`/`then` inside the string literal.
    let source =
        "FUNC main AS Integer\n  LET c AS Scalar = `\"` : LET s AS String = \"print if then\"\n  RETURN 0\nEND FUNC\n";
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("a.mfb");
    std::fs::write(&file, source).expect("write source");

    let output = Command::new(mfb_exe())
        .arg("fmt")
        .arg(&file)
        .output()
        .expect("run mfb fmt");
    assert!(
        output.status.success(),
        "mfb fmt failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let after = std::fs::read_to_string(&file).expect("read back");
    assert_eq!(
        after, source,
        "mfb fmt corrupted a scalar-quote line (bug-293 regressed):\n{after}"
    );
    // Belt and suspenders: the lowercase-keyword string must not have been
    // keyword-cased (which is what the desync did to the mis-scanned tail).
    assert!(
        after.contains("\"print if then\""),
        "the String after the scalar was keyword-cased (scanner desync):\n{after}"
    );
}
