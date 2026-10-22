//! Tree-level UCE (`optimizer::opt1::uce`, Level 2) kept a function's final
//! `RETURN` after truncating everything after the first always-terminal
//! statement, even when that `RETURN` read a local whose `LET` sat in the
//! dropped range. NIR validation then rejected the build with
//! `NIR local reference '<name>' does not resolve` at `-O2` and `-O3`.
//!
//! Found through `examples/sorts`: every `mode: app` program imports the
//! canvas package, whose `__canvas_damageFor` begins with
//! `IF NOT __canvas_damageEnabled() THEN RETURN …`. At `-O2` that test folds to
//! an unconditional `RETURN`, and the kept final `RETURN [x0, y0, x1, y1]`
//! named the dropped `x0`.
//!
//! Both terminal shapes are covered: a constant-folded early `RETURN`, and an
//! `IF` whose both arms return. Each level must print the literal expected
//! output, so no two levels can agree on a wrong answer.

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::path::Path;
use std::process::Command;

const SOURCE: &str = r#"IMPORT io

LET ENABLED AS Boolean = FALSE

FUNC folded(n AS Integer) AS Integer
  IF NOT ENABLED THEN
    RETURN n
  END IF
  LET doubled AS Integer = n * 2
  RETURN doubled
END FUNC

FUNC bothArms(n AS Integer) AS Integer
  IF n > 0 THEN
    RETURN 1
  ELSE
    RETURN 2
  END IF
  LET tripled AS Integer = n * 3
  RETURN tripled
END FUNC

FUNC main() AS Integer
  io::print(toString(folded(21)))
  io::print(toString(bothArms(5)))
  io::print(toString(bothArms(-5)))
  RETURN 0
END FUNC
"#;

const EXPECTED: &str = "21\n1\n2\n";

fn build_and_run(project: &Path, level: &str) -> String {
    let output = Command::new(common::mfb_exe())
        .arg("build")
        .arg(format!("-O{level}"))
        .arg(project)
        .output()
        .expect("run mfb build");
    assert!(
        output.status.success(),
        "-O{level} build failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("utf8 build output");
    let executable = stdout
        .lines()
        .find_map(|line| line.strip_prefix("Wrote executable to "))
        .expect("build output names the executable")
        .to_string();
    let run = Command::new(&executable)
        .current_dir(project)
        .output()
        .expect("run program");
    assert!(
        run.status.success(),
        "-O{level} program failed:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
    String::from_utf8(run.stdout).expect("utf8 program output")
}

#[test]
fn a_final_return_after_a_terminal_builds_and_agrees_at_every_level() {
    for level in ["0", "1", "2", "3"] {
        let project = common::temp_project("uce_final_return", SOURCE);
        let got = build_and_run(&project, level);
        let _ = std::fs::remove_dir_all(&project);
        assert_eq!(got, EXPECTED, "-O{level} printed the wrong output");
    }
}
