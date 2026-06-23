//! Native runtime tests for loop codegen correctness.
//!
//! These build a tiny program with the host `mfb`, run the produced executable,
//! and assert on its stdout — they exercise behavior that the AST/IR/native
//! acceptance goldens cannot catch (a miscompilation that still produces a
//! well-formed plan).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};
use std::{env, fs};

fn temp_project(name: &str, source: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock before epoch")
        .as_nanos();
    let root = env::temp_dir().join(format!("mfb_{name}_{nonce}"));
    fs::create_dir_all(root.join("src")).expect("create temp project");
    fs::write(
        root.join("project.json"),
        format!(
            "{{\"name\":\"{name}\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"executable\",\"sources\":[{{\"root\":\"src\",\"role\":\"main\",\"include\":[\"**/*.mfb\"]}}],\"entry\":\"main\",\"targets\":[\"native\"]}}\n"
        ),
    )
    .expect("write project.json");
    fs::write(root.join("src/main.mfb"), source).expect("write source");
    root
}

fn build_project(project: &Path) -> PathBuf {
    let output = Command::new(env!("CARGO_BIN_EXE_mfb"))
        .arg("build")
        .arg(project)
        .output()
        .expect("run mfb build");
    assert!(
        output.status.success(),
        "build failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("utf8 build output");
    let path = stdout
        .lines()
        .find_map(|line| line.strip_prefix("Wrote executable to "))
        .expect("build output executable path");
    PathBuf::from(path)
}

fn run(executable: &Path) -> String {
    let output = Command::new(executable)
        .output()
        .expect("run executable");
    assert!(
        output.status.success(),
        "program failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("utf8 stdout")
}

fn build_and_run(name: &str, source: &str) -> String {
    run(&build_project(&temp_project(name, source)))
}

/// Regression test for a `DO ... LOOP UNTIL` (post-test loop) miscompilation:
/// a `MUT String` initialized from a literal and appended to inside the body is
/// read as its stale *initial* value on every iteration, so only the final
/// append survives (`"ZX"` instead of `"ZXXXX"`). The `MUT Integer` loop counter
/// updates correctly, so the loop runs the right number of times — only the
/// reference-typed binding's reads are stale. The equivalent `WHILE`, `FOR`, and
/// `DO WHILE` forms (below) are correct, which points at the post-test loop's
/// handling of a constant-initialized `MUT` local rather than `&` itself.
///
#[test]
fn native_do_until_preserves_mut_string_across_iterations() {
    let out = build_and_run(
        "loop_do_until_string",
        r#"
IMPORT io

FUNC main AS Integer
  MUT s AS String = "Z"
  MUT n AS Integer = 0
  DO
    s = s & "X"
    n = n + 1
  LOOP UNTIL n >= 4
  io::print(s)
  RETURN 0
END FUNC
"#,
    );
    assert_eq!(out, "ZXXXX\n");
}

#[test]
fn native_while_accumulates_mut_string() {
    let out = build_and_run(
        "loop_while_string",
        r#"
IMPORT io

FUNC main AS Integer
  MUT s AS String = "Z"
  MUT n AS Integer = 0
  WHILE n < 4
    s = s & "X"
    n = n + 1
  WEND
  io::print(s)
  RETURN 0
END FUNC
"#,
    );
    assert_eq!(out, "ZXXXX\n");
}

#[test]
fn native_do_while_accumulates_mut_string() {
    let out = build_and_run(
        "loop_do_while_string",
        r#"
IMPORT io

FUNC main AS Integer
  MUT s AS String = "Z"
  MUT n AS Integer = 0
  DO WHILE n < 4
    s = s & "X"
    n = n + 1
  LOOP
  io::print(s)
  RETURN 0
END FUNC
"#,
    );
    assert_eq!(out, "ZXXXX\n");
}

#[test]
fn native_for_accumulates_mut_string() {
    let out = build_and_run(
        "loop_for_string",
        r#"
IMPORT io

FUNC main AS Integer
  MUT s AS String = "Z"
  FOR i = 1 TO 4
    s = s & "X"
  NEXT
  io::print(s)
  RETURN 0
END FUNC
"#,
    );
    assert_eq!(out, "ZXXXX\n");
}

/// The post-test loop updates a scalar `MUT Integer` correctly (it is only the
/// reference-typed binding's reads that go stale), so the counter reaches 4.
#[test]
fn native_do_until_updates_mut_integer_across_iterations() {
    let out = build_and_run(
        "loop_do_until_integer",
        r#"
IMPORT io

FUNC main AS Integer
  MUT total AS Integer = 0
  MUT n AS Integer = 0
  DO
    total = total + 10
    n = n + 1
  LOOP UNTIL n >= 4
  io::print(toString(total))
  RETURN 0
END FUNC
"#,
    );
    assert_eq!(out, "40\n");
}
