//! Regression tests for bug-744: a call to a `FUNC … AS Nothing` used as a VALUE.
//!
//! `Nothing` is a normal concrete unit type whose one value is `NOTHING`
//! (`mfb spec language types` §4.6), and only a `SUB` call is barred from value
//! position (`TYPE_SUB_HAS_NO_VALUE`, §7). So `LET y = f(1)` for a `FUNC f … AS
//! Nothing` is a legal program that binds `NOTHING`. Native codegen lowered every
//! `Nothing`-typed call to a `void` location — no register — because a statement
//! call has no value to carry. Any consumer that needed the value (a binding
//! store, an argument, an operand, a literal element, a field, a map value)
//! handed `void` to a register-operand constructor, and the compiler panicked
//! with "`void` is not a register operand (bug-721)" (before bug-721 it failed
//! the build with "unknown AArch64 register 'void'").
//!
//! One test per value position. Each program prints `eq` only when the value it
//! read back really is `NOTHING`, so a lowering that stores a garbage register
//! instead of panicking still fails here. The two contrast tests pin what already
//! worked: a statement-position call, and the `SUB` value-position diagnostic.

#[path = "../common/mod.rs"]
mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// A `FUNC … AS Nothing` with an observable effect, a `SUB` of the same shape,
/// a consumer taking a `Nothing` parameter, and a record with a `Nothing` field.
const PRELUDE: &str = concat!(
    "IMPORT io\n\n",
    "FUNC f(x AS Integer) AS Nothing\n",
    "  io::print(\"f\" & toString(x))\n",
    "  RETURN NOTHING\n",
    "END FUNC\n\n",
    "SUB sb(x AS Integer)\n",
    "  io::print(\"sb\" & toString(x))\n",
    "END SUB\n\n",
    "FUNC takesN(n AS Nothing) AS Integer\n",
    "  IF n = NOTHING THEN RETURN 7\n",
    "  RETURN 0\n",
    "END FUNC\n\n",
    "TYPE Holder\n",
    "  value AS Nothing\n",
    "END TYPE\n\n",
);

fn project(name: &str, body: &str) -> PathBuf {
    let nonce = common::unique_nonce();
    let root = std::env::temp_dir().join(format!("mfb_nothingval_{name}_{nonce}"));
    fs::create_dir_all(root.join("src")).expect("create root");
    fs::write(
        root.join("project.json"),
        "{\"name\":\"nothingval\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"executable\",\
         \"sources\":[{\"root\":\"src\",\"role\":\"main\",\"include\":[\"**/*.mfb\"]}],\
         \"entry\":\"main\",\"targets\":[\"native\"]}\n",
    )
    .expect("write manifest");
    fs::write(root.join("src/main.mfb"), format!("{PRELUDE}{body}")).expect("write source");
    root
}

fn mfb_build(root: &Path) -> (bool, String) {
    let out = Command::new(common::mfb_exe())
        .arg("build")
        .arg(root)
        .output()
        .expect("run mfb build");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), combined)
}

/// Build `body` (after the prelude), run it, and return its stdout. Panics with
/// the compiler output when the build fails or panics.
fn build_and_run(name: &str, body: &str) -> String {
    let root = project(name, body);
    let (ok, combined) = mfb_build(&root);
    assert!(
        ok && !combined.contains("panicked"),
        "expected the `{name}` program to build, but it failed:\n{combined}"
    );
    let exe = combined
        .lines()
        .find_map(|line| line.strip_prefix("Wrote executable to "))
        .expect("build reported no executable path")
        .trim()
        .to_string();
    let exe = root.join(exe.strip_prefix("./").unwrap_or(&exe));
    let out = Command::new(&exe).output().expect("run built program");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.success(),
        "the `{name}` program failed (exit {:?}):\nstdout:\n{stdout}\nstderr:\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = fs::remove_dir_all(&root);
    stdout
}

fn main_sub(lines: &str) -> String {
    format!("FUNC main() AS Nothing\n{lines}  io::print(\"done\")\nEND FUNC\n")
}

#[test]
fn let_binds_the_result_of_a_nothing_func() {
    let out = build_and_run(
        "let",
        &main_sub("  LET y = f(1)\n  IF y = NOTHING THEN io::print(\"eq\")\n"),
    );
    assert_eq!(out, "f1\neq\ndone\n");
}

#[test]
fn let_with_an_explicit_nothing_type_binds_the_result() {
    let out = build_and_run(
        "let_typed",
        &main_sub("  LET y AS Nothing = f(1)\n  IF y = NOTHING THEN io::print(\"eq\")\n"),
    );
    assert_eq!(out, "f1\neq\ndone\n");
}

#[test]
fn mut_binds_and_reassigns_the_result_of_a_nothing_func() {
    let out = build_and_run(
        "mut",
        &main_sub("  MUT y = f(1)\n  y = f(2)\n  IF y = NOTHING THEN io::print(\"eq\")\n"),
    );
    assert_eq!(out, "f1\nf2\neq\ndone\n");
}

#[test]
fn a_nothing_func_call_is_an_argument() {
    let out = build_and_run("arg", &main_sub("  io::print(toString(takesN(f(1))))\n"));
    assert_eq!(out, "f1\n7\ndone\n");
}

/// The owner's example (2026-09-26): `LET a = NOTHING`, then a `Nothing` value
/// passed to a `Nothing` function whose result is bound again.
#[test]
fn a_nothing_value_round_trips_through_a_nothing_func() {
    let out = build_and_run(
        "roundtrip",
        "FUNC same(n AS Nothing) AS Nothing\n  RETURN n\nEND FUNC\n\n\
         FUNC main() AS Nothing\n  LET a = NOTHING\n  LET b = same(a)\n  LET c = same(same(b))\n  \
         IF c = NOTHING THEN io::print(\"eq\")\n  io::print(\"done\")\nEND FUNC\n",
    );
    assert_eq!(out, "eq\ndone\n");
}

#[test]
fn a_nothing_func_call_is_a_comparison_operand() {
    let out = build_and_run(
        "operand",
        &main_sub("  IF f(1) = NOTHING THEN io::print(\"eq\")\n"),
    );
    assert_eq!(out, "f1\neq\ndone\n");
}

#[test]
fn a_nothing_func_returns_another_nothing_func_call() {
    let out = build_and_run(
        "return",
        "FUNC g() AS Nothing\n  RETURN f(1)\nEND FUNC\n\n\
         FUNC main() AS Nothing\n  g()\n  LET y = g()\n  IF y = NOTHING THEN io::print(\"eq\")\n  \
         io::print(\"done\")\nEND FUNC\n",
    );
    assert_eq!(out, "f1\nf1\neq\ndone\n");
}

#[test]
fn a_nothing_func_call_initializes_a_record_field() {
    let out = build_and_run(
        "field",
        &main_sub(
            "  LET h = Holder[value := f(1)]\n  IF h.value = NOTHING THEN io::print(\"eq\")\n",
        ),
    );
    assert_eq!(out, "f1\neq\ndone\n");
}

#[test]
fn a_lambda_whose_body_is_a_nothing_func_call_yields_nothing() {
    let out = build_and_run(
        "lambda",
        &main_sub(
            "  LET l = LAMBDA(x AS Integer) -> f(x)\n  LET y = l(3)\n  \
             IF y = NOTHING THEN io::print(\"eq\")\n",
        ),
    );
    assert_eq!(out, "f3\neq\ndone\n");
}

#[test]
fn a_nothing_func_called_through_a_func_value_yields_nothing() {
    let out = build_and_run(
        "funcvalue",
        &main_sub("  LET g = f\n  LET y = g(4)\n  IF y = NOTHING THEN io::print(\"eq\")\n"),
    );
    assert_eq!(out, "f4\neq\ndone\n");
}

/// §7: naming a `SUB` yields a `FUNC(...) AS Nothing` value, and a call through
/// that value is an ordinary `FUNC` call — it yields `NOTHING`.
#[test]
fn a_sub_called_through_its_func_value_yields_nothing() {
    let out = build_and_run(
        "subvalue",
        &main_sub("  LET g = sb\n  LET y = g(5)\n  IF y = NOTHING THEN io::print(\"eq\")\n"),
    );
    assert_eq!(out, "sb5\neq\ndone\n");
}

/// A built-in declared `AS Nothing` (`io::print`) is a `FUNC`, not a `SUB`.
#[test]
fn a_nothing_builtin_call_is_a_value() {
    let out = build_and_run(
        "builtin",
        &main_sub("  LET y = io::print(\"x\")\n  IF y = NOTHING THEN io::print(\"eq\")\n"),
    );
    assert_eq!(out, "x\neq\ndone\n");
}

/// §8 rule 13: `RECOVER` takes no operand for a `Nothing` trapped expression, and
/// the binding still holds `NOTHING` — on the success path here.
#[test]
fn an_inline_trapped_nothing_func_call_binds_nothing() {
    let out = build_and_run(
        "trap",
        &main_sub(
            "  LET y = f(1) TRAP(e)\n    RECOVER\n  END TRAP\n  \
             IF y = NOTHING THEN io::print(\"eq\")\n",
        ),
    );
    assert_eq!(out, "f1\neq\ndone\n");
}

/// Contrast: the statement-position call has always worked.
#[test]
fn a_statement_position_nothing_func_call_still_runs() {
    let out = build_and_run("statement", &main_sub("  f(1)\n"));
    assert_eq!(out, "f1\ndone\n");
}

/// Contrast: a direct `SUB` call in value position stays a compile error.
#[test]
fn a_sub_call_in_value_position_is_still_rejected() {
    let root = project("subdiag", &main_sub("  LET y = sb(1)\n"));
    let (ok, combined) = mfb_build(&root);
    let _ = fs::remove_dir_all(&root);
    assert!(
        !ok,
        "a SUB call in value position must not build:\n{combined}"
    );
    assert!(
        combined.contains("TYPE_SUB_HAS_NO_VALUE") && !combined.contains("panicked"),
        "expected TYPE_SUB_HAS_NO_VALUE, got:\n{combined}"
    );
}
