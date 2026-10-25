//! `module_uses_unicode_runtime_tables` (`engine/analysis/module_analysis/unicode_tables.rs`)
//! decides whether the module carries the Unicode runtime tables (a sizeable data
//! object). It must find a case-mapping or grapheme call in EVERY statement and value
//! position — a call it misses is a program that links without the tables its
//! `strings::upper` reads, which fails at run time — and must not find one in a
//! program that has none, or whose call folds at compile time.

use crate::codegen::engine::analysis::module_uses_unicode_runtime_tables;
use crate::target::NativeBuildMode::Console;
use crate::testutil::{nir_for_src, CodeTarget};

const HEADER: &str = "\
IMPORT fs
IMPORT io
IMPORT os
IMPORT strings

TYPE Note
  name AS String
END TYPE

MUT sink AS String = \"\"
";

fn uses_tables(body: &str, name: &str) -> bool {
    let source = format!(
        "{HEADER}\nFUNC probe(s AS String, f AS fs::File STATE Note, n AS Integer) AS Integer\n{body}\n  RETURN 0\nEND FUNC\n\nFUNC main() AS Integer\n  LET keep = probe\n  RETURN 0\nEND FUNC\n"
    );
    let module = nir_for_src(&source, CodeTarget::LinuxX86_64, Console)
        .unwrap_or_else(|error| panic!("{name}: the probe lowers to NIR: {error}"));
    module_uses_unicode_runtime_tables(&module)
}

#[test]
fn a_case_mapping_call_is_found_in_every_statement_and_value_position() {
    let up = "strings::upper(s)";
    let positions: &[(&str, String)] = &[
        ("bind", format!("  LET u AS String = {up}")),
        (
            "assign",
            format!("  MUT u AS String = \"\"\n  u = {up}"),
        ),
        ("store global", format!("  sink = {up}")),
        ("state assign", format!("  f.state.name = {up}")),
        ("eval", format!("  io::print({up})")),
        (
            "fail",
            format!("  IF n > 1 THEN\n    FAIL error(1, {up})\n  END IF"),
        ),
        ("return", format!("  RETURN len({up})")),
        (
            "exit program",
            format!("  IF n > 1 THEN\n    os::exit(len({up}))\n  END IF"),
        ),
        (
            "if condition",
            format!("  IF {up} = \"A\" THEN\n    io::print(\"a\")\n  END IF"),
        ),
        (
            "if then body",
            format!("  IF n > 1 THEN\n    io::print({up})\n  END IF"),
        ),
        (
            "if else body",
            format!("  IF n > 1 THEN\n    io::print(\"a\")\n  ELSE\n    io::print({up})\n  END IF"),
        ),
        (
            "match value",
            format!("  MATCH {up}\n    CASE \"A\"\n      io::print(\"a\")\n    CASE ELSE\n      io::print(\"b\")\n  END MATCH"),
        ),
        (
            "match guard",
            format!("  MATCH n\n    CASE 1 WHEN {up} = \"A\"\n      io::print(\"a\")\n    CASE ELSE\n      io::print(\"b\")\n  END MATCH"),
        ),
        (
            "match case body",
            format!("  MATCH n\n    CASE 1\n      io::print({up})\n    CASE ELSE\n      io::print(\"b\")\n  END MATCH"),
        ),
        (
            "while condition",
            format!("  WHILE len({up}) < 0\n    io::print(\"a\")\n  END WHILE"),
        ),
        (
            "while body",
            format!("  WHILE n < 0\n    io::print({up})\n  END WHILE"),
        ),
        (
            "for bound",
            format!("  FOR i = 0 TO len({up})\n    io::print(\"a\")\n  NEXT"),
        ),
        (
            "for body",
            format!("  FOR i = 0 TO 1\n    io::print({up})\n  NEXT"),
        ),
        (
            "do until condition",
            format!("  DO\n    io::print(\"a\")\n  LOOP UNTIL len({up}) > 0"),
        ),
        (
            "do until body",
            format!("  DO\n    io::print({up})\n  LOOP UNTIL TRUE"),
        ),
        (
            "for each iterable",
            "  FOR EACH c IN strings::graphemes(s)\n    io::print(c)\n  NEXT".to_string(),
        ),
        (
            "for each body",
            format!("  FOR EACH c IN [1, 2]\n    io::print({up})\n  NEXT"),
        ),
    ];
    let missed: Vec<&str> = positions
        .iter()
        .filter(|(name, body)| !uses_tables(body, name))
        .map(|(name, _)| *name)
        .collect();
    assert!(
        missed.is_empty(),
        "a call in these positions was not found: {missed:?}"
    );
}

#[test]
fn a_program_without_a_case_mapping_call_needs_no_tables() {
    assert!(!uses_tables("  io::print(s)", "none"));
    // A call over a compile-time constant folds, so the tables are not read at run time.
    assert!(!uses_tables(
        "  io::print(strings::upper(\"abc\"))",
        "folded"
    ));
    // `lower` maps case too.
    assert!(uses_tables("  io::print(strings::lower(s))", "lower"));
}
