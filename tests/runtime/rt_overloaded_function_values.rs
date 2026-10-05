//! Regression test for bug-822: a declared OVERLOADED user function named as a
//! value was reported as an undeclared identifier (`SYMBOL_UNKNOWN_IDENTIFIER`),
//! because monomorphization mangled every declaration of the set while leaving
//! the reference's bare name unrevised.
//!
//! A reference must select the unique declaration whose complete callable type —
//! ordered parameter types, return type and isolation — matches the expected
//! `FUNC` type of its context, and the program must then RUN that declaration.
//! An ambiguous or incompatible reference gets a source-level overload/type
//! diagnostic, never an undeclared-name one. Every case builds and executes a
//! real program; the controls (single declaration, direct call, typed lambda)
//! already worked before the fix.

#[path = "../common/mod.rs"]
mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn unique_root(name: &str) -> PathBuf {
    let nonce = common::unique_nonce();
    let root = std::env::temp_dir().join(format!("mfb_bug822_{name}_{nonce}"));
    fs::create_dir_all(&root).expect("create root");
    root
}

fn mfb() -> Command {
    let mut command = Command::new(common::mfb_exe());
    command.env("MFB_HOME", std::env::temp_dir().join("mfb_bug822_home"));
    command
}

fn write_project(root: &Path, name: &str, kind: &str, deps: &[&str], source: &str) {
    let dir = root.join(name);
    fs::create_dir_all(dir.join("src")).expect("src dir");
    fs::create_dir_all(dir.join("packages")).expect("packages dir");
    let executable = kind == "executable";
    let role = if executable { "main" } else { "package" };
    let packages = deps
        .iter()
        .map(|dep| {
            format!(
                "{{\"name\":\"{dep}\",\"version\":\"=0.1.0\",\"source\":\"file:packages/{dep}.mfp\",\"direct\":true,\"requiredBy\":[]}}"
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let entry_field = if executable {
        "\"entry\":\"main\",\"targets\":[\"native\"],"
    } else {
        ""
    };
    let manifest = format!(
        "{{\"name\":\"{name}\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"{kind}\",\
         \"description\":\"bug-822 fixture\",\
         \"sources\":[{{\"root\":\"src\",\"role\":\"{role}\",\"include\":[\"**/*.mfb\"]}}],\
         {entry_field}\"packages\":[{packages}]}}\n"
    );
    fs::write(dir.join("project.json"), manifest).expect("write manifest");
    let src_name = if executable { "main.mfb" } else { "lib.mfb" };
    fs::write(dir.join("src").join(src_name), source).expect("write source");
}

fn build(root: &Path, name: &str) -> (bool, String) {
    let out = mfb()
        .arg("build")
        .arg(root.join(name))
        .output()
        .expect("run mfb build");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), combined)
}

/// Build `source` as an executable, run it, and return its stdout.
fn run_program(case: &str, root: &Path, source: &str) -> String {
    write_project(root, "app", "executable", &[], source);
    run_built(case, root, source)
}

fn run_built(case: &str, root: &Path, source: &str) -> String {
    let (ok, out) = build(root, "app");
    assert!(
        ok,
        "case `{case}`: build failed:\n{out}\n--- source ---\n{source}"
    );
    let exe = out
        .lines()
        .find_map(|line| line.strip_prefix("Wrote executable to "))
        .expect("build reported no executable path")
        .trim()
        .to_string();
    let run = Command::new(&exe).output().expect("run app");
    let stdout = String::from_utf8_lossy(&run.stdout).into_owned();
    assert!(
        run.status.success(),
        "case `{case}`: program failed ({:?}):\n{stdout}{}",
        run.status,
        String::from_utf8_lossy(&run.stderr)
    );
    stdout
}

fn assert_runs(case: &str, source: &str, expected: &str) {
    let root = unique_root(case);
    let stdout = run_program(case, &root, source);
    assert_eq!(stdout, expected, "case `{case}` printed the wrong output");
}

/// Build `source`, require failure with `code`, and require that the declared
/// overloaded name is never reported as undeclared.
fn assert_rejects(case: &str, source: &str, code: &str, detail: &str) {
    let root = unique_root(case);
    write_project(&root, "app", "executable", &[], source);
    let (ok, out) = build(&root, "app");
    assert!(!ok, "case `{case}`: build unexpectedly succeeded:\n{out}");
    assert!(
        out.contains(code),
        "case `{case}`: expected {code}, got:\n{out}"
    );
    assert!(
        out.contains(detail),
        "case `{case}`: expected detail `{detail}`, got:\n{out}"
    );
    assert!(
        !out.contains("SYMBOL_UNKNOWN_IDENTIFIER"),
        "case `{case}`: a declared overload was reported undeclared:\n{out}"
    );
}

const PICK_INT: &str = "FUNC pick(x AS Integer) AS Integer\n  RETURN x + 1\nEND FUNC\n";
const PICK_STRING: &str = "FUNC pick(x AS String) AS Integer\n  RETURN len(x)\nEND FUNC\n";
const PICK_RETURN: &str = "FUNC pick(x AS Integer) AS String\n  RETURN toString(x)\nEND FUNC\n";

fn program(decls: &str, body: &str) -> String {
    format!("IMPORT io\n{PICK_INT}{decls}FUNC main() AS Nothing\n{body}END FUNC\n")
}

// ---- the bug document's reproduction ---------------------------------------

#[test]
fn annotated_local_selects_parameter_type_overload() {
    assert_runs(
        "param",
        &program(
            PICK_STRING,
            "  LET f AS FUNC(Integer) AS Integer = pick\n  io::print(toString(f(4)))\n",
        ),
        "5\n",
    );
}

#[test]
fn annotated_local_selects_return_type_overload() {
    assert_runs(
        "return",
        &program(
            PICK_RETURN,
            "  LET f AS FUNC(Integer) AS Integer = pick\n  io::print(toString(f(4)))\n",
        ),
        "5\n",
    );
}

/// Both members of a return-type set are reachable, each by its own type.
#[test]
fn annotated_locals_select_each_return_type_member() {
    assert_runs(
        "return_both",
        &program(
            PICK_RETURN,
            "  LET f AS FUNC(Integer) AS Integer = pick\n  LET g AS FUNC(Integer) AS String = pick\n  \
             io::print(toString(f(4)))\n  io::print(g(4) & \"!\")\n",
        ),
        "5\n4!\n",
    );
}

// ---- controls (worked before the fix) --------------------------------------

#[test]
fn single_declaration_control() {
    assert_runs(
        "single",
        &program(
            "",
            "  LET f AS FUNC(Integer) AS Integer = pick\n  io::print(toString(f(4)))\n",
        ),
        "5\n",
    );
}

#[test]
fn direct_call_and_lambda_controls() {
    assert_runs(
        "direct_lambda",
        &program(
            PICK_STRING,
            "  io::print(toString(pick(4)))\n  \
             LET f AS FUNC(Integer) AS Integer = LAMBDA(x AS Integer) -> pick(x)\n  \
             io::print(toString(f(4)))\n",
        ),
        "5\n5\n",
    );
}

// ---- every value context that carries an expected callable type ------------

#[test]
fn annotated_global_selects_overload() {
    assert_runs(
        "global",
        &program(
            &format!("{PICK_STRING}LET chosen AS FUNC(String) AS Integer = pick\n"),
            "  io::print(toString(chosen(\"abc\")))\n",
        ),
        "3\n",
    );
}

#[test]
fn assignment_selects_overload() {
    assert_runs(
        "assign",
        &program(
            PICK_STRING,
            "  MUT f AS FUNC(String) AS Integer = LAMBDA(s AS String) -> 0\n  f = pick\n  \
             io::print(toString(f(\"abcd\")))\n",
        ),
        "4\n",
    );
}

#[test]
fn callback_argument_selects_overload() {
    assert_runs(
        "argument",
        &program(
            &format!(
                "{PICK_STRING}FUNC apply(f AS FUNC(Integer) AS Integer, x AS Integer) AS Integer\n  \
                 RETURN f(x)\nEND FUNC\n"
            ),
            "  io::print(toString(apply(pick, 4)))\n  io::print(toString(apply(f := pick, x := 9)))\n",
        ),
        "5\n10\n",
    );
}

#[test]
fn return_operand_selects_overload() {
    assert_runs(
        "return_operand",
        &program(
            &format!(
                "{PICK_RETURN}FUNC chooser() AS FUNC(Integer) AS String\n  RETURN pick\nEND FUNC\n"
            ),
            "  LET g AS FUNC(Integer) AS String = chooser()\n  io::print(g(7) & \"?\")\n",
        ),
        "7?\n",
    );
}

#[test]
fn typed_aggregates_select_overload() {
    assert_runs(
        "aggregate",
        &program(
            &format!("{PICK_STRING}TYPE Holder\n  op AS FUNC(String) AS Integer\nEND TYPE\n"),
            "  LET h AS Holder = Holder[pick]\n  LET hf = h.op\n  io::print(toString(hf(\"ab\")))\n  \
             LET named AS Holder = Holder[op := pick]\n  LET nf = named.op\n  \
             io::print(toString(nf(\"abc\")))\n  \
             LET fs AS List OF FUNC(Integer) AS Integer = [pick, pick]\n  \
             LET g = collections::get(fs, 1)\n  io::print(toString(g(10)))\n  \
             LET m = Map OF String TO FUNC(String) AS Integer { \"k\" := pick }\n  \
             LET mf = collections::get(m, \"k\")\n  io::print(toString(mf(\"hello\")))\n  \
             LET updated = WITH h { op := pick }\n  LET uf = updated.op\n  \
             io::print(toString(uf(\"abcdef\")))\n",
        )
        .replace("IMPORT io\n", "IMPORT io\nIMPORT collections\n"),
        "2\n3\n11\n5\n6\n",
    );
}

#[test]
fn isolated_annotation_selects_isolated_overload() {
    assert_runs(
        "isolated",
        &program(
            "ISOLATED FUNC pick(x AS String) AS Integer\n  RETURN len(x) * 10\nEND FUNC\n",
            "  LET f AS ISOLATED FUNC(String) AS Integer = pick\n  io::print(toString(f(\"ab\")))\n  \
             LET g AS FUNC(String) AS Integer = pick\n  io::print(toString(g(\"abc\")))\n",
        ),
        "20\n30\n",
    );
}

/// A local that shadows the overloaded name is the local, never the overload set.
#[test]
fn local_shadowing_the_name_is_not_rewritten() {
    assert_runs(
        "shadow",
        &program(
            PICK_STRING,
            "  LET pick = LAMBDA(x AS Integer) -> x * 100\n  LET f AS FUNC(Integer) AS Integer = pick\n  \
             io::print(toString(f(2)))\n",
        ),
        "200\n",
    );
}

// ---- diagnostics: ambiguous / incompatible references ----------------------

#[test]
fn untyped_reference_is_ambiguous() {
    assert_rejects(
        "untyped",
        &program(PICK_STRING, "  LET f = pick\n"),
        "TYPE_OVERLOAD_AMBIGUOUS",
        "`pick`",
    );
}

#[test]
fn reference_without_a_callable_context_is_ambiguous() {
    assert_rejects(
        "no_context",
        &program(PICK_STRING, "  io::print(toString(pick))\n"),
        "TYPE_OVERLOAD_AMBIGUOUS",
        "`pick`",
    );
}

#[test]
fn no_matching_parameter_types_is_rejected() {
    assert_rejects(
        "no_match_params",
        &program(PICK_STRING, "  LET f AS FUNC(Boolean) AS Integer = pick\n"),
        "TYPE_OVERLOAD_NO_MATCH",
        "FUNC(Boolean) AS Integer",
    );
}

#[test]
fn no_matching_return_type_is_rejected() {
    assert_rejects(
        "no_match_return",
        &program(PICK_RETURN, "  LET f AS FUNC(Integer) AS Boolean = pick\n"),
        "TYPE_OVERLOAD_NO_MATCH",
        "FUNC(Integer) AS Boolean",
    );
}

#[test]
fn isolation_mismatch_is_rejected() {
    assert_rejects(
        "no_match_isolated",
        &program(
            PICK_STRING,
            "  LET f AS ISOLATED FUNC(Integer) AS Integer = pick\n",
        ),
        "TYPE_OVERLOAD_NO_MATCH",
        "ISOLATED FUNC(Integer) AS Integer",
    );
}

#[test]
fn non_callable_expected_type_is_rejected() {
    assert_rejects(
        "no_match_scalar",
        &program(PICK_STRING, "  LET f AS Integer = pick\n"),
        "TYPE_OVERLOAD_NO_MATCH",
        "`pick`",
    );
}

// ---- imported overload sets --------------------------------------------------

const OV_SRC: &str = "EXPORT FUNC pick(x AS Integer) AS Integer\n  RETURN x + 1\nEND FUNC\n\
EXPORT FUNC pick(x AS String) AS Integer\n  RETURN len(x)\nEND FUNC\n\
EXPORT FUNC pick(x AS Integer, y AS Integer) AS Integer\n  RETURN x * y\nEND FUNC\n";

fn build_package(root: &Path) {
    write_project(root, "ov", "package", &[], OV_SRC);
    let (ok, out) = build(root, "ov");
    assert!(ok, "package `ov` failed to build:\n{out}");
}

fn consumer(root: &Path, import: &str, body: &str) -> String {
    let source = format!("{import}\nIMPORT io\nFUNC main() AS Nothing\n{body}END FUNC\n");
    write_project(root, "app", "executable", &["ov"], &source);
    fs::copy(
        root.join("ov").join("ov.mfp"),
        root.join("app").join("packages").join("ov.mfp"),
    )
    .expect("install ov.mfp");
    source
}

#[test]
fn imported_overload_selected_by_annotation() {
    let root = unique_root("imported");
    build_package(&root);
    let source = consumer(
        &root,
        "IMPORT ov",
        "  LET f AS FUNC(Integer) AS Integer = ov::pick\n  LET g AS FUNC(String) AS Integer = ov::pick\n  \
         LET h AS FUNC(Integer, Integer) AS Integer = ov::pick\n  \
         io::print(toString(f(4)))\n  io::print(toString(g(\"abc\")))\n  io::print(toString(h(6, 7)))\n",
    );
    assert_eq!(run_built("imported", &root, &source), "5\n3\n42\n");
}

#[test]
fn imported_overload_selected_through_an_alias() {
    let root = unique_root("imported_alias");
    build_package(&root);
    let source = consumer(
        &root,
        "IMPORT ov AS radio",
        "  LET f AS FUNC(String) AS Integer = radio::pick\n  io::print(toString(f(\"hello\")))\n",
    );
    assert_eq!(run_built("imported_alias", &root, &source), "5\n");
}

#[test]
fn imported_overload_without_a_match_is_rejected() {
    let root = unique_root("imported_no_match");
    build_package(&root);
    consumer(
        &root,
        "IMPORT ov",
        "  LET f AS FUNC(Boolean) AS Integer = ov::pick\n",
    );
    let (ok, out) = build(&root, "app");
    assert!(!ok, "build unexpectedly succeeded:\n{out}");
    assert!(out.contains("TYPE_OVERLOAD_NO_MATCH"), "got:\n{out}");
    assert!(!out.contains("SYMBOL_UNKNOWN_IDENTIFIER"), "got:\n{out}");
}

#[test]
fn imported_overload_untyped_is_ambiguous() {
    let root = unique_root("imported_untyped");
    build_package(&root);
    consumer(&root, "IMPORT ov", "  LET f = ov::pick\n");
    let (ok, out) = build(&root, "app");
    assert!(!ok, "build unexpectedly succeeded:\n{out}");
    assert!(out.contains("TYPE_OVERLOAD_AMBIGUOUS"), "got:\n{out}");
    assert!(!out.contains("SYMBOL_UNKNOWN_IDENTIFIER"), "got:\n{out}");
}

/// Inside a generic body the annotation is substituted per instantiation, so each
/// instantiation selects its own member.
#[test]
fn generic_body_selects_per_instantiation() {
    assert_runs(
        "generic",
        &program(
            &format!(
                "{PICK_STRING}FUNC via OF T(x AS T) AS Integer\n  LET f AS FUNC(T) AS Integer = pick\n  \
                 RETURN f(x)\nEND FUNC\n"
            ),
            "  io::print(toString(via(4)))\n  io::print(toString(via(\"abc\")))\n",
        ),
        "5\n3\n",
    );
}

// ---- a local that shadows the overloaded name is called, not the overload ---
//
// Found while fixing bug-822: the direct-call path rewrote a call through a
// LOCAL or PARAMETER named like an overloaded function to that overload, so the
// program silently ran the wrong function (`3`, not `200`). A single declaration
// was already correct.

#[test]
fn call_through_shadowing_local_runs_the_local() {
    assert_runs(
        "shadow_call_local",
        &program(
            PICK_STRING,
            "  LET pick = LAMBDA(x AS Integer) -> x * 100\n  io::print(toString(pick(2)))\n",
        ),
        "200\n",
    );
}

#[test]
fn call_through_shadowing_parameter_runs_the_parameter() {
    assert_runs(
        "shadow_call_param",
        &program(
            &format!(
                "{PICK_STRING}FUNC run(pick AS FUNC(Integer) AS Integer) AS Integer\n  RETURN pick(2)\nEND FUNC\n"
            ),
            "  io::print(toString(run(LAMBDA(x AS Integer) -> x * 100)))\n",
        ),
        "200\n",
    );
}

/// Found while fixing bug-822: inside a generic body a `FUNC(T) AS T`
/// annotation kept `T` in the expected type of its initializer, so a
/// return-type overload set returning callables could never be selected there
/// (`TYPE_OVERLOAD_AMBIGUOUS` on a valid program).
#[test]
fn generic_body_selects_return_overload_returning_a_callable() {
    assert_runs(
        "generic_return_callable",
        "IMPORT io\n\
         FUNC inc(x AS Integer) AS Integer\n  RETURN x + 1\nEND FUNC\n\
         FUNC shout(x AS String) AS String\n  RETURN x & \"!\"\nEND FUNC\n\
         FUNC make() AS FUNC(Integer) AS Integer\n  RETURN inc\nEND FUNC\n\
         FUNC make() AS FUNC(String) AS String\n  RETURN shout\nEND FUNC\n\
         FUNC via OF T(x AS T) AS T\n  LET f AS FUNC(T) AS T = make()\n  RETURN f(x)\nEND FUNC\n\
         FUNC main() AS Nothing\n  io::print(toString(via(4)))\n  io::print(via(\"hi\"))\nEND FUNC\n",
        "5\nhi!\n",
    );
}

/// A user override of an overridable built-in name is always mangled, so a value
/// reference to it reached the bare built-in name (`TYPE_UNKNOWN_VALUE`). The
/// gap-fill rule selects it exactly as it selects a call; a type the built-in
/// supports, and a built-in predicate reference, keep the built-in.
#[test]
fn builtin_named_override_selected_by_annotation() {
    assert_runs(
        "builtin_override",
        "IMPORT io\nIMPORT collections\n\
         TYPE Point\n  x AS Integer\nEND TYPE\n\
         FUNC toString(p AS Point) AS String\n  RETURN \"P\" & toString(p.x)\nEND FUNC\n\
         FUNC main() AS Nothing\n  LET f AS FUNC(Point) AS String = toString\n  io::print(f(Point[7]))\n  \
         LET p AS FUNC(Integer) AS Boolean = isEven\n  io::print(toString(p(4)))\n  \
         io::print(toString(len(collections::filter([1, 2, 3, 4], isEven))))\nEND FUNC\n",
        "P7\nTRUE\n2\n",
    );
}
