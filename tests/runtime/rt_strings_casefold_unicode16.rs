//! bug-905: `strings::caseFold` (and regex `(?i)`, which calls it) applies the
//! C+F mappings of Unicode 16.0.0 CaseFolding.txt, both when the compiler folds a
//! literal and when the generated runtime tables fold a run-time value. The data
//! used to come from the `unicode-casefold` crate (Unicode 9.0.0), so 184 later
//! capital letters were copied through unfolded.

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

/// (scalar, its CaseFolding.txt C/F mapping), all added after Unicode 9 except the
/// last four, which pin the full (multi-scalar) and long-standing mappings.
const FOLDS: &[(&str, &str)] = &[
    ("1C90", "10D0"),           // Georgian Mtavruli (Unicode 11)
    ("1C89", "1C8A"),           // Cyrillic Tje (Unicode 16)
    ("2C2F", "2C5F"),           // Glagolitic Caudate Chrivi (Unicode 14)
    ("A7C0", "A7C1"),           // Latin (Unicode 14)
    ("A7DC", "019B"),           // Latin Lambda With Stroke (Unicode 16)
    ("10570", "10597"),         // Vithkuqi (Unicode 14)
    ("10D50", "10D70"),         // Garay (Unicode 16)
    ("16E40", "16E60"),         // Medefaidrin (Unicode 11)
    ("00DF", "0073 0073"),      // sharp s: full folding
    ("0130", "0069 0307"),      // dotted capital I: full folding
    ("FB03", "0066 0066 0069"), // ffi ligature
    ("10400", "10428"),         // Deseret
];

fn lit(hexes: &str) -> String {
    hexes
        .split(' ')
        .map(|h| format!("\\u{{{h}}}"))
        .collect::<String>()
}

#[test]
fn case_fold_matches_unicode_16_for_literals_and_run_time_values() {
    let mut body = String::new();
    for (i, (from, to)) in FOLDS.iter().enumerate() {
        // Compile-time fold of a literal, and run-time fold of the same value.
        body.push_str(&format!(
            "  io::print(\"{i} const \" & toString(strings::caseFold(\"{f}\") = \"{t}\"))\n",
            f = lit(from),
            t = lit(to)
        ));
        body.push_str(&format!(
            "  io::print(\"{i} run \" & toString(strings::caseFold(d(\"{f}\")) = \"{t}\"))\n",
            f = lit(from),
            t = lit(to)
        ));
    }
    body.push_str(
        "  io::print(\"regex \" & toString(regex::match(d(\"\\u{1C90}\"), \"(?i)^\\u{10D0}$\")))\n",
    );
    let source = format!(
        "IMPORT io
IMPORT strings
IMPORT regex

FUNC d(s AS String) AS String
  RETURN s & \"\"
END FUNC

FUNC main() AS Integer
{body}  RETURN 0
END FUNC
"
    );
    let project = common::temp_project("casefold_unicode16", &source);
    let exe = common::build_project(&project);
    let out = Command::new(&exe).output().expect("run the program");
    let _ = std::fs::remove_dir_all(&project);
    assert!(out.status.success(), "program failed");
    let mut expected = String::new();
    for i in 0..FOLDS.len() {
        expected.push_str(&format!("{i} const TRUE\n{i} run TRUE\n"));
    }
    expected.push_str("regex TRUE\n");
    assert_eq!(String::from_utf8_lossy(&out.stdout), expected);
}
