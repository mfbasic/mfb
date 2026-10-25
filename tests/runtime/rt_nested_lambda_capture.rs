//! A lambda nested in a lambda captures a binding from two scopes out.
//!
//! `ir::lower`'s capture analysis (`collect_captured_locals`) did not look inside
//! a nested `LAMBDA`, so the enclosing lambda captured nothing the inner one
//! needed, and lowering the inner closure looked the binding up in a frame that
//! never had it: `LAMBDA(i) -> LAMBDA(v) -> v * i + k` failed to build with
//! "NIR local reference 'k' does not resolve", and an outer lambda whose body
//! passes such a lambda on was typed `FUNC(...) AS Unknown`, failing its binding
//! with `TYPE_BINDING_MISMATCH`. A nested lambda's free variables are now
//! captured by every lambda between it and their binding — and a `MUT` reached
//! that way is rejected exactly as a direct `MUT` capture is.

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

const RETURNS_A_CAPTURING_LAMBDA: &str = "\
IMPORT io
IMPORT collections

FUNC main() AS Integer
  LET xs AS List OF Integer = [1, 2, 3]
  MUT n AS Integer = 0
  FOR k = 1 TO 3
    LET fs AS List OF FUNC(Integer) AS Integer = collections::transform(xs, LAMBDA(i AS Integer) -> LAMBDA(v AS Integer) -> v * i + k)
    FOR EACH f IN fs
      n = n + f(2)
    NEXT
  NEXT
  io::print(toString(n))
  RETURN 0
END FUNC
";

const PASSES_A_CAPTURING_LAMBDA_ON: &str = "\
IMPORT io
IMPORT collections

FUNC main() AS Integer
  MUT n AS Integer = 0
  LET xs AS List OF Integer = [1, 2, 3]
  FOR k = 1 TO 5
    LET label AS String = \"pre\" & toString(k)
    LET outer AS FUNC(Integer) AS List OF Integer = LAMBDA(m AS Integer) -> collections::transform(xs, LAMBDA(x AS Integer) -> x * m + len(label))
    LET ys AS List OF Integer = outer(k)
    n = n + collections::reduce(ys, 0, LAMBDA(acc AS Integer, y AS Integer) -> acc + y)
  NEXT
  io::print(toString(n))
  RETURN 0
END FUNC
";

const NESTED_MUT_CAPTURE: &str = "\
IMPORT io
IMPORT collections

FUNC main() AS Integer
  MUT total AS Integer = 0
  LET xs AS List OF Integer = [1, 2, 3]
  LET fs AS List OF FUNC(Integer) AS Integer = collections::transform(xs, LAMBDA(i AS Integer) -> LAMBDA(v AS Integer) -> v + total)
  io::print(toString(len(fs)))
  RETURN 0
END FUNC
";

fn build(name: &str, source: &str) -> (bool, String, String) {
    let project = common::temp_project(name, source);
    let output = Command::new(common::mfb_exe())
        .arg("build")
        .arg("--debug")
        .arg(&project)
        .output()
        .expect("run mfb build");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    if !output.status.success() {
        let _ = std::fs::remove_dir_all(&project);
        return (false, stdout, stderr);
    }
    let exe = stdout
        .lines()
        .filter_map(|line| line.strip_prefix("Wrote executable to "))
        .find(|p| p.ends_with(common::host_executable_suffix()))
        .or_else(|| {
            stdout
                .lines()
                .find_map(|line| line.strip_prefix("Wrote executable to "))
        })
        .expect("an executable")
        .to_string();
    let out = Command::new(&exe).output().expect("run the program");
    let _ = std::fs::remove_dir_all(&project);
    assert!(out.status.success(), "{name}: the program failed");
    let report = String::from_utf8_lossy(&out.stderr).into_owned();
    let sum = |key: &str| -> u64 {
        report
            .lines()
            .filter_map(|line| line.strip_prefix("arena."))
            .filter_map(|rest| rest.split_once(key))
            .filter(|(arena, _)| !arena.contains('.'))
            .map(|(_, n)| n.trim().parse::<u64>().expect("a count"))
            .sum()
    };
    assert_eq!(
        (sum(".alloc_calls "), sum(".live_bytes ")),
        (sum(".free_calls "), 0),
        "{name}: every closure is freed"
    );
    (
        true,
        String::from_utf8_lossy(&out.stdout).into_owned(),
        report,
    )
}

#[test]
fn a_lambda_returned_from_a_lambda_captures_the_outer_binding() {
    // sum over k = 1..3, i = 1..3 of (2 * i + k)
    let (built, stdout, stderr) = build("nested_lambda_returned", RETURNS_A_CAPTURING_LAMBDA);
    assert!(built, "build failed:\n{stdout}\n{stderr}");
    assert_eq!(stdout, "54\n");
}

#[test]
fn a_lambda_passed_on_inside_a_lambda_captures_the_outer_bindings() {
    // sum over k = 1..5 of (6 * k + 3 * len("preK"))
    let (built, stdout, stderr) = build("nested_lambda_passed_on", PASSES_A_CAPTURING_LAMBDA_ON);
    assert!(built, "build failed:\n{stdout}\n{stderr}");
    assert_eq!(stdout, "150\n");
}

#[test]
fn a_nested_mut_capture_is_rejected_like_a_direct_one() {
    let (built, stdout, stderr) = build("nested_lambda_mut", NESTED_MUT_CAPTURE);
    assert!(
        !built,
        "a MUT reached through a nested lambda must not build"
    );
    assert!(
        format!("{stdout}{stderr}").contains("TYPE_LAMBDA_CAPTURE_UNSUPPORTED"),
        "{stdout}{stderr}"
    );
}
