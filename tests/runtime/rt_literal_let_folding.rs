//! plan-161-A: the Level-1 literal-`LET` folding row replaces a read of a
//! module-level `LET` whose initializer is a scalar literal with that literal.
//!
//! What must hold, and what each case pins:
//!
//! - the folded read computes the same value as the load it replaced, for every
//!   scalar type the row folds (`every_scalar_type_reads_the_same`);
//! - arithmetic through a folded `LET` raises the same error — code, message,
//!   file, line and column — at `-O0` and `-O1` (`overflow_through_a_folded_let`);
//! - the fold actually happens: at `-O1` the reading function no longer loads the
//!   globals block, and at `-O0` it still does (`the_read_becomes_an_immediate`),
//!   so the behavioral cases above are not vacuous.
//!
//! The initialization-order guard the row shares with the Level-2 row is pinned
//! by `rt_global_init_order_fold` (bug-747).

#[path = "../common/mod.rs"]
mod common;
use common::{mfb_exe, temp_project};
use std::path::Path;
use std::process::Command;

fn build_at(project: &Path, level: u8, extra: &[&str]) -> std::process::Output {
    let output = Command::new(mfb_exe())
        .arg("build")
        .arg(format!("-O{level}"))
        .args(extra)
        .arg(project)
        .output()
        .expect("run mfb build");
    assert!(
        output.status.success(),
        "-O{level} build failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

/// Build at `-O<level>` and run: `(exit code, stdout, stderr)`.
fn run_at(project: &Path, level: u8) -> (i32, String, String) {
    let output = build_at(project, level, &[]);
    let stdout = String::from_utf8(output.stdout).expect("utf8 build output");
    let executable = stdout
        .lines()
        .find_map(|line| line.strip_prefix("Wrote executable to "))
        .expect("build output names the executable");
    let run = Command::new(executable).output().expect("run program");
    (
        run.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&run.stdout).into_owned(),
        String::from_utf8_lossy(&run.stderr).into_owned(),
    )
}

#[test]
fn every_scalar_type_reads_the_same() {
    for (type_, literal, expected) in [
        ("Integer", "29", "29"),
        ("Byte", "200", "200"),
        ("Boolean", "TRUE", "TRUE"),
        ("Float", "1.5", "1.5"),
        ("Fixed", "2.25", "2.25"),
        ("Money", "3.10", "3.10"),
    ] {
        let source = format!(
            "\
IMPORT io\n\
LET V AS {type_} = {literal}\n\
\n\
FUNC shown() AS String\n\
  RETURN toString(V)\n\
END FUNC\n\
\n\
FUNC main() AS Nothing\n\
  io::print(shown())\n\
END FUNC\n"
        );
        let project = temp_project(&format!("literal_let_{}", type_.to_lowercase()), &source);
        let unoptimized = run_at(&project, 0);
        assert_eq!(
            unoptimized.0, 0,
            "{type_}: -O0 run failed: {}",
            unoptimized.2
        );
        assert!(
            unoptimized.1.trim().eq_ignore_ascii_case(expected)
                || unoptimized.1.trim().starts_with(expected),
            "{type_}: -O0 printed {:?}",
            unoptimized.1
        );
        assert_eq!(
            run_at(&project, 1),
            unoptimized,
            "{type_}: -O1 differs from -O0"
        );
    }
}

#[test]
fn overflow_through_a_folded_let() {
    let source = "\
IMPORT io\n\
LET BIG AS Integer = 9223372036854775807\n\
\n\
FUNC bump(x AS Integer) AS Integer\n\
  RETURN BIG + x\n\
END FUNC\n\
\n\
FUNC main() AS Nothing\n\
  LET n AS Integer = bump(1) TRAP(e)\n\
    io::print(\"code=\" & toString(e.code) & \" msg=\" & e.message & \" file=\" & e.source.filename & \" line=\" & toString(e.source.line) & \" char=\" & toString(e.source.char))\n\
    RETURN\n\
  END TRAP\n\
  io::print(toString(n))\n\
END FUNC\n";
    let project = temp_project("literal_let_overflow", source);
    let unoptimized = run_at(&project, 0);
    assert_eq!(unoptimized.0, 0, "-O0 run failed: {}", unoptimized.2);
    assert!(
        unoptimized.1.starts_with("code=77050010 ") && unoptimized.1.contains(" line=5 "),
        "BIG + 1 overflows at line 5: {:?}",
        unoptimized.1
    );
    assert_eq!(
        run_at(&project, 1),
        unoptimized,
        "the folded read raises the same error at the same place"
    );
}

/// `add_imm _, x19, _` is a globals-block address on AArch64 (`x19` holds the
/// arena/globals base).
fn loads_globals(project: &Path, level: u8, function: &str) -> bool {
    build_at(
        project,
        level,
        &["-q", "-ncode", "-target", "macos-aarch64"],
    );
    let dump = std::fs::read_dir(project)
        .expect("project dir")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.extension().is_some_and(|ext| ext == "ncode"))
        .expect("an .ncode dump");
    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dump).expect("read dump")).expect("json");
    let body = json["functions"]
        .as_array()
        .expect("functions")
        .iter()
        .find(|f| f["name"] == function)
        .unwrap_or_else(|| panic!("no function {function}"));
    body["instructions"]
        .as_array()
        .expect("instructions")
        .iter()
        .any(|i| i["op"] == "add_imm" && i["src"] == "x19")
}

#[test]
fn the_read_becomes_an_immediate() {
    let source = "\
IMPORT io\n\
LET ROWS AS Integer = 29\n\
\n\
FUNC cell(x AS Integer, y AS Integer) AS Integer\n\
  RETURN x * ROWS + y\n\
END FUNC\n\
\n\
FUNC main() AS Nothing\n\
  io::print(toString(cell(2, 3)))\n\
END FUNC\n";
    let project = temp_project("literal_let_immediate", source);
    assert!(
        loads_globals(&project, 0, "cell"),
        "control: at -O0 `cell` loads ROWS from the globals block"
    );
    assert!(
        !loads_globals(&project, 1, "cell"),
        "at -O1 `cell` reads ROWS as an immediate"
    );
    assert_eq!(run_at(&project, 1), run_at(&project, 0));
    assert_eq!(run_at(&project, 1).1, "61\n");
}
