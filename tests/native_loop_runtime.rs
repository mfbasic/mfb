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
    let output = Command::new(executable).output().expect("run executable");
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

/// Regression test for bug-67: `op_requires_empty_string_constant`
/// (`src/target/shared/code/module_analysis.rs`) skipped the `FOR` and
/// `DO ... LOOP UNTIL` loop bodies, so an uninitialized `MUT String` declared
/// *only* inside those loops did not force emission of the shared
/// `_mfb_str_empty` data object — while the codegen for the default bind still
/// referenced it, producing a dangling relocation that failed plan validation
/// ("native code data relocation target '_mfb_str_empty' is not a data object").
/// The `WHILE` and top-level forms already worked; all loop forms must now build
/// and print empty lines for the uninitialized default.
#[test]
fn native_uninitialized_string_in_for_loop_builds() {
    let out = build_and_run(
        "loop_for_uninit_string",
        r#"
IMPORT io

SUB main()
  FOR i = 0 TO 2
    MUT s AS String
    io::print(s)
  NEXT
END SUB
"#,
    );
    assert_eq!(out, "\n\n\n");
}

#[test]
fn native_uninitialized_string_in_do_until_loop_builds() {
    let out = build_and_run(
        "loop_do_until_uninit_string",
        r#"
IMPORT io

SUB main()
  MUT n AS Integer = 0
  DO
    MUT s AS String
    io::print(s)
    n = n + 1
  LOOP UNTIL n >= 3
END SUB
"#,
    );
    assert_eq!(out, "\n\n\n");
}

/// Guard cases: the same uninitialized bind inside `WHILE` and at function top
/// level built correctly before the fix and must continue to.
#[test]
fn native_uninitialized_string_in_while_loop_builds() {
    let out = build_and_run(
        "loop_while_uninit_string",
        r#"
IMPORT io

SUB main()
  MUT n AS Integer = 0
  WHILE n < 3
    MUT s AS String
    io::print(s)
    n = n + 1
  WEND
END SUB
"#,
    );
    assert_eq!(out, "\n\n\n");
}

/// Regression test for bug-57: a `WHILE` condition that folds a loop-mutated
/// local through a string folder (`toString`) used the local's stale loop-entry
/// constant, frozen in the single emitted comparison above the back-edge, so the
/// loop never observed `c` changing and ran forever. The body's guard bounds the
/// iteration count and exits with a distinct marker so a buggy binary produces a
/// well-formed *wrong* result (exit 0) rather than hanging: on the buggy codegen
/// the frozen `toString(0) <> "3"` stays true, the guard fires, and it prints
/// "looping c=21"; the fixed codegen reads the live `c` each iteration and
/// terminates at `c = 3`.
#[test]
fn native_while_tostring_condition_reads_live_local() {
    let out = build_and_run(
        "loop_while_tostring_cond",
        r#"
IMPORT io

FUNC main AS Integer
  MUT c AS Integer = 0
  MUT guard AS Integer = 0
  WHILE toString(c) <> "3"
    c = c + 1
    guard = guard + 1
    IF guard > 20 THEN
      io::print("looping c=" & toString(c))
      RETURN 0
    END IF
  WEND
  io::print("terminated c=" & toString(c))
  RETURN 0
END FUNC
"#,
    );
    assert_eq!(out, "terminated c=3\n");
}

/// Companion for bug-57 covering the string-concat folder: a loop-invariant
/// `String` local (`tag`) concatenated with `toString(c)` in the condition. The
/// `&` folder collapses `tag & toString(c)` to the rodata `"step0"` at loop
/// entry (both operands folded), freezing the comparison. After the fix the
/// condition reads the live `c` (and the still-invariant `tag`) each iteration.
#[test]
fn native_while_concat_condition_reads_live_local() {
    let out = build_and_run(
        "loop_while_concat_cond",
        r#"
IMPORT io

FUNC main AS Integer
  MUT c AS Integer = 0
  MUT guard AS Integer = 0
  MUT tag AS String = "step"
  WHILE (tag & toString(c)) <> "step3"
    c = c + 1
    guard = guard + 1
    IF guard > 20 THEN
      io::print("looping c=" & toString(c))
      RETURN 0
    END IF
  WEND
  io::print("terminated c=" & toString(c))
  RETURN 0
END FUNC
"#,
    );
    assert_eq!(out, "terminated c=3\n");
}

/// Guard case: the integer relational form of the same loop already terminated
/// (numeric lowering never consults `local.constant`), and must continue to.
#[test]
fn native_while_integer_condition_terminates() {
    let out = build_and_run(
        "loop_while_int_cond",
        r#"
IMPORT io

FUNC main AS Integer
  MUT c AS Integer = 0
  WHILE c < 3
    c = c + 1
  WEND
  io::print("terminated c=" & toString(c))
  RETURN 0
END FUNC
"#,
    );
    assert_eq!(out, "terminated c=3\n");
}

/// Guard case: the `DO ... LOOP UNTIL` sibling clears loop-entry constants before
/// its condition already, so a `toString`-folding termination test was correct
/// before the fix and must remain so.
#[test]
fn native_do_until_tostring_condition_reads_live_local() {
    let out = build_and_run(
        "loop_do_until_tostring_cond",
        r#"
IMPORT io

FUNC main AS Integer
  MUT c AS Integer = 0
  MUT guard AS Integer = 0
  DO
    c = c + 1
    guard = guard + 1
    IF guard > 20 THEN
      io::print("looping c=" & toString(c))
      RETURN 0
    END IF
  LOOP UNTIL toString(c) = "3"
  io::print("terminated c=" & toString(c))
  RETURN 0
END FUNC
"#,
    );
    assert_eq!(out, "terminated c=3\n");
}
