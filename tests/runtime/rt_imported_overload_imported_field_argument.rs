//! Regression test for bug-631: a call to an *overloaded* function exported by a
//! package failed with `TYPE_OVERLOAD_AMBIGUOUS` when its argument was a field
//! of a record type the consumer imported (`ov::show(h.first)`, a `FOR EACH`
//! variable over `h.items`, `collections::get(h.items, 0)`, ...).
//!
//! The monomorphizer's `record_fields` read only the consumer's own type
//! declarations, so an imported record's field typed as `None` → `Unknown`, and
//! bug-36's (correct) `Unknown` wildcard in `resolve_imported_overload` matched
//! every nominal overload. The fix makes the imported record layouts visible to
//! the monomorphizer, so the field has its declared type and exactly one
//! overload matches. Every case builds a real package and a consumer from source.

#[path = "../common/mod.rs"]
mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn unique_root(name: &str) -> PathBuf {
    let nonce = common::unique_nonce();
    let root = std::env::temp_dir().join(format!("mfb_bug631_{name}_{nonce}"));
    fs::create_dir_all(&root).expect("create root");
    root
}

fn mfb() -> Command {
    let mut command = Command::new(common::mfb_exe());
    command.env("MFB_HOME", std::env::temp_dir().join("mfb_bug631_home"));
    command
}

fn write_project(root: &Path, name: &str, kind: &str, deps: &[&str], entry: bool, source: &str) {
    let dir = root.join(name);
    fs::create_dir_all(dir.join("src")).expect("src dir");
    fs::create_dir_all(dir.join("packages")).expect("packages dir");
    let role = if entry { "main" } else { "package" };
    let mut packages = String::new();
    for dep in deps {
        if !packages.is_empty() {
            packages.push(',');
        }
        packages.push_str(&format!(
            "{{\"name\":\"{dep}\",\"version\":\"=0.1.0\",\"source\":\"file:packages/{dep}.mfp\",\"direct\":true,\"requiredBy\":[]}}"
        ));
    }
    let entry_field = if entry {
        "\"entry\":\"main\",\"targets\":[\"native\"],"
    } else {
        ""
    };
    let manifest = format!(
        "{{\"name\":\"{name}\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"{kind}\",\
         \"description\":\"bug-631 fixture\",\
         \"sources\":[{{\"root\":\"src\",\"role\":\"{role}\",\"include\":[\"**/*.mfb\"]}}],\
         {entry_field}\"packages\":[{packages}]}}\n"
    );
    fs::write(dir.join("project.json"), manifest).expect("write manifest");
    let src_name = if entry { "main.mfb" } else { "lib.mfb" };
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

// The bug document's package: four same-arity overloads of `show` over nominal
// types (so an `Unknown` argument matches all of them), a non-overloaded `one`,
// and a `Holder` whose fields are a record, a list of records, and a list of a
// union.
const OV_SRC: &str = "EXPORT TYPE A\n  x AS Integer\nEND TYPE\n\
EXPORT TYPE B\n  y AS Integer\nEND TYPE\n\
EXPORT TYPE Leaf\n  s AS String\nEND TYPE\n\
EXPORT UNION U\n  Leaf\n  A\nEND UNION\n\
EXPORT TYPE Holder\n  items AS List OF A\n  nodes AS List OF U\n  first AS A\nEND TYPE\n\
EXPORT FUNC show(a AS A) AS String\n  RETURN \"A\"\nEND FUNC\n\
EXPORT FUNC show(b AS B) AS String\n  RETURN \"B\"\nEND FUNC\n\
EXPORT FUNC show(u AS U) AS String\n  RETURN \"U\"\nEND FUNC\n\
EXPORT FUNC show(h AS Holder) AS String\n  RETURN \"H\"\nEND FUNC\n\
EXPORT FUNC one(a AS A) AS String\n  RETURN \"one\"\nEND FUNC\n\
EXPORT FUNC holder() AS Holder\n  LET u AS U = Leaf[\"s\"]\n  RETURN Holder[[A[1], A[2]], [u], A[3]]\nEND FUNC\n";

/// Build `ov`, install it into a consumer whose `main` runs `body` (after
/// `LET h AS ov::Holder = ov::holder()`), and return the consumer's stdout lines.
/// `decls` are consumer-level declarations placed before `main`.
fn run_consumer(case: &str, decls: &str, body: &str) -> Vec<String> {
    let root = unique_root(case);
    write_project(&root, "ov", "package", &[], false, OV_SRC);
    let (ok, out) = build(&root, "ov");
    assert!(ok, "package `ov` failed to build:\n{out}");

    let app_src = format!(
        "IMPORT ov\nIMPORT io\nIMPORT collections\n{decls}\
         FUNC main() AS Integer\n  LET h AS ov::Holder = ov::holder()\n{body}  RETURN 0\nEND FUNC\n"
    );
    write_project(&root, "app", "executable", &["ov"], true, &app_src);
    fs::copy(
        root.join("ov").join("ov.mfp"),
        root.join("app").join("packages").join("ov.mfp"),
    )
    .expect("install ov.mfp");

    let (ok, out) = build(&root, "app");
    assert!(
        ok,
        "case `{case}`: consumer failed to build:\n{out}\n--- source ---\n{app_src}"
    );
    let exe = out
        .lines()
        .find_map(|line| line.strip_prefix("Wrote executable to "))
        .expect("app build reported no executable path")
        .trim()
        .to_string();
    let run = Command::new(&exe).output().expect("run app");
    let stdout = String::from_utf8_lossy(&run.stdout).into_owned();
    assert!(
        run.status.success(),
        "case `{case}`: app crashed:\n{stdout}"
    );
    stdout.lines().map(str::to_string).collect()
}

fn assert_prints(case: &str, decls: &str, body: &str, expected: &[&str]) {
    let lines = run_consumer(case, decls, body);
    assert_eq!(lines, expected, "case `{case}` printed the wrong lines");
}

// ---- ✗ rows of the reproduction table --------------------------------------

#[test]
fn field_read_directly() {
    assert_prints("direct", "", "  io::print(ov::show(h.first))\n", &["A"]);
}

#[test]
fn untyped_let_bound_to_field() {
    assert_prints(
        "let_field",
        "",
        "  LET f = h.first\n  io::print(ov::show(f))\n",
        &["A"],
    );
}

#[test]
fn for_each_over_record_list_field() {
    assert_prints(
        "each_items",
        "",
        "  FOR EACH a IN h.items\n    io::print(ov::show(a))\n  NEXT\n",
        &["A", "A"],
    );
}

#[test]
fn for_each_over_union_list_field() {
    assert_prints(
        "each_nodes",
        "",
        "  FOR EACH n IN h.nodes\n    io::print(ov::show(n))\n  NEXT\n",
        &["U"],
    );
}

#[test]
fn collections_get_of_list_field() {
    assert_prints(
        "get_items",
        "",
        "  io::print(ov::show(collections::get(h.items, 0)))\n",
        &["A"],
    );
}

#[test]
fn untyped_let_bound_to_collections_get() {
    assert_prints(
        "let_get",
        "",
        "  LET a = collections::get(h.items, 0)\n  io::print(ov::show(a))\n",
        &["A"],
    );
}

// ---- latent blast-radius sites ---------------------------------------------

#[test]
fn local_overload_set_with_imported_field() {
    assert_prints(
        "local_overload",
        "FUNC pick(a AS ov::A) AS String\n  RETURN \"la\"\nEND FUNC\n\
         FUNC pick(b AS ov::B) AS String\n  RETURN \"lb\"\nEND FUNC\n",
        "  io::print(pick(h.first))\n",
        &["la"],
    );
}

#[test]
fn generic_called_with_imported_field() {
    assert_prints(
        "generic",
        "FUNC ident OF T(x AS T) AS T\n  RETURN x\nEND FUNC\n",
        "  io::print(ov::show(ident(h.first)))\n",
        &["A"],
    );
}

// Builds on the unfixed compiler too: the untyped `[]` fields take their types
// from later passes. Kept as a guard on the constructor `record_fields` site.
#[test]
fn constructing_imported_record_with_untyped_list_fields() {
    assert_prints(
        "construct",
        "",
        "  LET g AS ov::Holder = ov::Holder[[], [], ov::A[1]]\n\
         \x20 io::print(ov::show(g))\n  io::print(toString(len(g.items)))\n",
        &["H", "0"],
    );
}

// ---- ✓ rows: guards that already work --------------------------------------

#[test]
fn guard_whole_record_and_non_overloaded_callee() {
    assert_prints(
        "guards",
        "",
        "  io::print(ov::show(h))\n  io::print(ov::one(h.first))\n\
         \x20 FOR EACH a IN h.items\n    io::print(ov::one(a))\n  NEXT\n",
        &["H", "one", "one", "one"],
    );
}

#[test]
fn guard_typed_locals() {
    assert_prints(
        "typed_locals",
        "",
        "  LET xs AS List OF ov::A = h.items\n  FOR EACH a IN xs\n    io::print(ov::show(a))\n  NEXT\n\
         \x20 LET us AS List OF ov::U = h.nodes\n  FOR EACH n IN us\n    io::print(ov::show(n))\n  NEXT\n\
         \x20 LET a0 = collections::get(xs, 0)\n  io::print(ov::show(a0))\n",
        &["A", "A", "U", "A"],
    );
}
