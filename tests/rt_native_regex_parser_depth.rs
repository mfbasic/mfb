//! bug-423: the regex *parser* (`__regex_compile` → recursive descent over
//! nested groups) had no nesting-depth cap. A deeply-nested-group pattern
//! recursed once per `(` and overflowed the native stack of the produced
//! executable, killing it with an uncatchable SIGSEGV during *compile* — before
//! any matching happened. bug-315 had capped only the *matcher*.
//!
//! These build a program that hands the engine a pathologically deep pattern
//! (as untrusted runtime data, so no compile-time escaping is involved), TRAP
//! the resulting error, and assert it is the ordinary `ErrInvalidFormat`
//! (77050003) — a catchable failure, not a crash. Without the parser depth cap
//! the produced executable SIGSEGVs, `run()` sees a non-success exit, and the
//! test fails.

mod common;
use common::temp_project;
use std::path::Path;
use std::process::Command;

fn build_project(project: &Path) -> std::path::PathBuf {
    let output = Command::new(common::mfb_exe())
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
    std::path::PathBuf::from(path)
}

fn run(executable: &Path) -> String {
    let output = Command::new(executable).output().expect("run executable");
    assert!(
        output.status.success(),
        "program crashed or exited non-zero (bug-423 SIGSEGV?): status {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("utf8 stdout")
}

fn build_and_run(name: &str, source: &str) -> String {
    run(&build_project(&temp_project(name, source)))
}

/// A pattern of 2000 unbalanced `(` — well past the ~350-deep native-stack
/// overflow threshold — must reach the parser depth cap and fail cleanly with
/// `ErrInvalidFormat`, never SIGSEGV the process.
#[test]
fn native_regex_deep_unbalanced_parens_is_clean_error_not_sigsegv() {
    let out = build_and_run(
        "regex_parser_depth_unbalanced",
        r#"IMPORT regex
IMPORT io
IMPORT errorCode

FUNC deep() AS Integer
  MUT s AS String = ""
  MUT i AS Integer = 0
  WHILE i < 2000
    s = s & "("
    i = i + 1
  END WHILE
  LET m AS Boolean = regex::match("x", s)
  RETURN 0
  TRAP(err)
    io::print("code=" & toString(err.code = errorCode::ErrInvalidFormat))
    RETURN 1
  END TRAP
END FUNC

SUB main()
  io::print("caught=" & toString(deep()))
END SUB
"#,
    );
    assert!(
        out.contains("code=TRUE"),
        "expected a clean ErrInvalidFormat, got:\n{out}"
    );
    assert!(
        out.contains("caught=1"),
        "expected the TRAP to fire (caught=1), got:\n{out}"
    );
}

/// A *balanced* deeply-nested group `(((…x…)))` (2000 deep) is a syntactically
/// valid but pathologically nested pattern. It too must be rejected by the depth
/// cap with a clean error rather than crashing the compile.
#[test]
fn native_regex_deep_balanced_groups_is_clean_error_not_sigsegv() {
    let out = build_and_run(
        "regex_parser_depth_balanced",
        r#"IMPORT regex
IMPORT io
IMPORT errorCode

FUNC deep() AS Integer
  MUT s AS String = ""
  MUT i AS Integer = 0
  WHILE i < 2000
    s = s & "("
    i = i + 1
  END WHILE
  s = s & "x"
  i = 0
  WHILE i < 2000
    s = s & ")"
    i = i + 1
  END WHILE
  LET m AS Boolean = regex::match("x", s)
  RETURN 0
  TRAP(err)
    io::print("code=" & toString(err.code = errorCode::ErrInvalidFormat))
    RETURN 1
  END TRAP
END FUNC

SUB main()
  io::print("caught=" & toString(deep()))
END SUB
"#,
    );
    assert!(
        out.contains("code=TRUE"),
        "expected a clean ErrInvalidFormat, got:\n{out}"
    );
    assert!(
        out.contains("caught=1"),
        "expected the TRAP to fire (caught=1), got:\n{out}"
    );
}

/// A legitimately-but-modestly nested pattern (well under the cap) must still
/// compile and match — the fix must not reject ordinary nested groups.
#[test]
fn native_regex_modest_nesting_still_compiles() {
    let out = build_and_run(
        "regex_parser_depth_ok",
        r#"IMPORT regex
IMPORT io

SUB main()
  ' 40 nested capturing groups around a literal — deep, but ordinary.
  MUT s AS String = ""
  MUT i AS Integer = 0
  WHILE i < 40
    s = s & "("
    i = i + 1
  END WHILE
  s = s & "abc"
  i = 0
  WHILE i < 40
    s = s & ")"
    i = i + 1
  END WHILE
  io::print("match=" & toString(regex::match("abc", s)))
END SUB
"#,
    );
    assert!(
        out.contains("match=TRUE"),
        "modest nesting should compile and match, got:\n{out}"
    );
}
