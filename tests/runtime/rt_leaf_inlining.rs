//! plan-158-C (bug-740 mechanism (c)): the Level-1 leaf-function inlining row
//! replaces a call to a one-expression scalar helper with the helper's
//! expression. Every observable must be what the call produced:
//!
//! - values, including a short-circuit `AND` helper used as a condition;
//! - a runtime error raised inside the helper — code, message, and an
//!   `ErrorLoc` naming the HELPER's file, line and column, also when the helper
//!   lives in another source file than the caller;
//! - a failure inside the helper reaching the caller's function-level `TRAP`
//!   exactly as the call's failure did;
//! - a builtin inside the helper (`collections::get` on a module-level table)
//!   raising its own error with the helper's location.
//!
//! The control (`the_call_is_gone_at_level_one`) pins that the row actually
//! fires, so the equivalences above are not vacuous.

#[path = "../common/mod.rs"]
mod common;
use common::{mfb_exe, temp_project};
use std::path::{Path, PathBuf};
use std::process::Command;

const HELPERS: &str = "\
IMPORT collections\n\
\n\
PUBLIC LET ROWS AS Integer = 29\n\
PUBLIC LET DELTAS AS List OF Integer = [0, 1, -1]\n\
\n\
PUBLIC FUNC cell(x AS Integer, y AS Integer) AS Integer\n\
  RETURN x * ROWS + y\n\
END FUNC\n\
\n\
PUBLIC FUNC inMap(x AS Integer, y AS Integer) AS Boolean\n\
  RETURN x >= 0 AND x < 79 AND y >= 0 AND y < ROWS\n\
END FUNC\n\
\n\
PUBLIC FUNC delta(dir AS Integer) AS Integer\n\
  RETURN collections::get(DELTAS, dir)\n\
END FUNC\n";

const MAIN: &str = "\
IMPORT io\n\
\n\
FUNC report(e AS Error) AS String\n\
  RETURN \"code=\" & toString(e.code) & \" file=\" & e.source.filename & \" line=\" & toString(e.source.line) & \" char=\" & toString(e.source.char)\n\
END FUNC\n\
\n\
FUNC guarded(x AS Integer) AS String\n\
  LET c AS Integer = cell(x, 1)\n\
  RETURN \"guarded ok \" & toString(c)\n\
TRAP(e)\n\
  RETURN \"guarded trapped \" & report(e)\n\
END TRAP\n\
END FUNC\n\
\n\
SUB main()\n\
  LET a AS Integer = 2\n\
  LET b AS Integer = 3\n\
  io::print(toString(cell(a, b)))\n\
  LET big AS Integer = 9223372036854775807\n\
  LET one AS Integer = 1\n\
  MUT inside AS Integer = 0\n\
  FOR x = -1 TO 80\n\
    FOR y = -1 TO 30\n\
      IF inMap(x, y) THEN inside = inside + 1\n\
    NEXT\n\
  NEXT\n\
  io::print(\"inside=\" & toString(inside))\n\
  io::print(guarded(a))\n\
  io::print(guarded(big))\n\
  LET dir AS Integer = 7\n\
  LET s AS Integer = delta(dir) TRAP(e)\n\
    io::print(\"delta trapped \" & report(e))\n\
    EXIT SUB\n\
  END TRAP\n\
  io::print(toString(s + one))\n\
END SUB\n";

fn project() -> PathBuf {
    let project = temp_project("leaf_inlining", MAIN);
    std::fs::write(project.join("src/helpers.mfb"), HELPERS).expect("write helpers");
    project
}

fn build(project: &Path, level: u8, extra: &[&str]) -> String {
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
    String::from_utf8(output.stdout).expect("utf8 build output")
}

fn run(project: &Path, level: u8) -> (i32, String) {
    let stdout = build(project, level, &[]);
    let executable = stdout
        .lines()
        .find_map(|line| line.strip_prefix("Wrote executable to "))
        .expect("build output names the executable");
    let run = Command::new(executable).output().expect("run program");
    (
        run.status.code().unwrap_or(-1),
        String::from_utf8(run.stdout).expect("utf8 program output"),
    )
}

#[test]
fn inlining_changes_no_observable() {
    let project = project();
    let unoptimized = run(&project, 0);
    assert_eq!(unoptimized.0, 0, "-O0 run failed");
    let lines: Vec<&str> = unoptimized.1.lines().collect();
    assert_eq!(lines[0], "61");
    assert_eq!(lines[1], "inside=2291", "79 x 29 cells are in the map");
    assert_eq!(lines[2], "guarded ok 59");
    assert!(
        lines[3].starts_with("guarded trapped code=77050010 file=src/helpers.mfb line=7 "),
        "the overflow names the helper's file and line: {}",
        lines[3]
    );
    assert!(
        lines[4].starts_with("delta trapped code=77050001 file=src/helpers.mfb line=15 "),
        "the index error names the helper's file and line: {}",
        lines[4]
    );
    assert_eq!(run(&project, 1), unoptimized, "-O1 must match -O0 exactly");
}

/// `bl` targets in `function`'s `.ncode` body (AArch64).
fn calls_in(project: &Path, level: u8, function: &str) -> Vec<String> {
    build(
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
        .filter(|i| i["op"] == "bl")
        .map(|i| i["target"].as_str().unwrap_or_default().to_string())
        .collect()
}

#[test]
fn the_call_is_gone_at_level_one() {
    let project = project();
    let count = |level: u8, name: &str| {
        calls_in(&project, level, "main")
            .into_iter()
            .filter(|target| target.contains(name))
            .count()
    };
    assert!(
        count(0, "cell") >= 1 && count(0, "inMap") >= 1,
        "control: -O0 calls the helpers"
    );
    assert_eq!(count(1, "cell"), 0, "-O1 inlines `cell(a, b)`");
    assert_eq!(count(1, "inMap"), 0, "-O1 inlines `inMap(x, y)`");
    assert_eq!(
        count(1, "delta"),
        1,
        "an inline-TRAPped call is the desugar's CallResult and stays a call"
    );
}
