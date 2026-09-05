//! Regression test for bug-391: a `thread::start` worker whose result is a
//! **recursive** value — a self-referential union like `Node` whose
//! `ElementNode.children` is `List OF Node`, bare or embedded in a record —
//! could not be deep-copied out of the worker arena. The record shape aborted
//! the build with `native thread transfer cannot copy value of type '…'`, and a
//! naive fix would have hung codegen (the transfer copier inlined its recursion
//! over the type with no cycle guard).
//!
//! The fix emits a per-type runtime deep-copy function for every recursive type
//! and routes a recursive edge to a call, so the copy recurses at run time over
//! the finite data. These tests build every package from source and prove a
//! recursive `Node` survives a thread transfer — read back AFTER the worker
//! arena is reclaimed, so a shallow copy would be a use-after-free.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn unique_root(name: &str) -> PathBuf {
    let nonce = common::unique_nonce();
    let root = std::env::temp_dir().join(format!("mfb_bug391_{name}_{nonce}"));
    fs::create_dir_all(&root).expect("create root");
    root
}

fn mfb() -> Command {
    let mut command = Command::new(common::mfb_exe());
    command.env("MFB_HOME", std::env::temp_dir().join("mfb_bug391_home"));
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
            "{{\"name\":\"{dep}\",\"version\":\"=0.1.0\",\"source\":\"file:packages/{dep}.mfp\"}}"
        ));
    }
    let entry_field = if entry {
        "\"entry\":\"main\",\"targets\":[\"native\"],"
    } else {
        ""
    };
    let manifest = format!(
        "{{\"name\":\"{name}\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"{kind}\",\
         \"description\":\"bug-391 fixture\",\
         \"sources\":[{{\"root\":\"src\",\"role\":\"{role}\",\"include\":[\"**/*.mfb\"]}}],\
         {entry_field}\"packages\":[{packages}]}}\n"
    );
    fs::write(dir.join("project.json"), manifest).expect("write manifest");
    let src_name = if entry { "main.mfb" } else { "lib.mfb" };
    fs::write(dir.join("src").join(src_name), source).expect("write source");
}

fn build_ok(root: &Path, name: &str) -> String {
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
    assert!(
        out.status.success(),
        "expected `{name}` to build, but it failed:\n{combined}"
    );
    combined
}

fn install(root: &Path, from_pkg: &str, into_pkg: &str, mfp: &str) {
    fs::copy(
        root.join(from_pkg).join(format!("{mfp}.mfp")),
        root.join(into_pkg)
            .join("packages")
            .join(format!("{mfp}.mfp")),
    )
    .unwrap_or_else(|e| panic!("install {mfp}.mfp into {into_pkg}: {e}"));
}

// A recursive Node union (the shape thread transfer used to reject), with an
// iterative pre-order text renderer that proves the whole tree survived.
const DOM_SRC: &str = "IMPORT collections\n\
EXPORT TYPE ElementNode\n  tag AS String\n  children AS List OF Node\nEND TYPE\n\
EXPORT TYPE TextNode\n  text AS String\nEND TYPE\n\
EXPORT UNION Node\n  ElementNode\n  TextNode\nEND UNION\n\
EXPORT FUNC element(tag AS String, kids AS List OF Node) AS Node\n  LET n AS Node = ElementNode[tag, kids]\n  RETURN n\nEND FUNC\n\
EXPORT FUNC textNode(s AS String) AS Node\n  LET n AS Node = TextNode[s]\n  RETURN n\nEND FUNC\n\
EXPORT FUNC render(root AS Node) AS String\n  MUT out AS String = \"\"\n  MUT stack AS List OF Node = [root]\n  DO WHILE len(stack) > 0\n    LET i AS Integer = len(stack) - 1\n    LET node AS Node = collections::get(stack, i)\n    stack = collections::removeAt(stack, i)\n    MATCH node\n      CASE ElementNode(e)\n        MUT j AS Integer = len(e.children) - 1\n        DO WHILE j >= 0\n          stack = collections::append(stack, collections::get(e.children, j))\n          j = j - 1\n        LOOP\n      CASE TextNode(t)\n        out = out & t.text\n      CASE ELSE\n    END MATCH\n  LOOP\n  RETURN out\nEND FUNC\n";

// A worker returning a recursive Node (bare) and one returning a record that
// embeds a Node — the two shapes bug-391 covers.
const WORKER_SRC: &str = "IMPORT dom391\n\
EXPORT ISOLATED FUNC buildNode(w AS ThreadWorker OF String TO Node, seed AS String) AS Node\n  RETURN dom391::element(\"ul\", [dom391::element(\"li\", [dom391::textNode(seed & \"A\")]), dom391::element(\"li\", [dom391::textNode(seed & \"B\")])])\nEND FUNC\n\
EXPORT TYPE Wrap\n  ok AS Boolean\n  root AS Node\n  label AS String\nEND TYPE\n\
EXPORT ISOLATED FUNC buildWrap(w AS ThreadWorker OF String TO Wrap, seed AS String) AS Wrap\n  RETURN Wrap[TRUE, dom391::element(\"p\", [dom391::textNode(seed & \"X\")]), \"L\" & seed]\nEND FUNC\n";

const APP_SRC: &str = "IMPORT io\nIMPORT dom391\nIMPORT worker391\nIMPORT thread\n\
FUNC main AS Integer\n\
  LET t1 AS Thread OF String TO Node = thread::start(worker391::buildNode, \"seed\")\n\
  LET n AS Node = thread::waitFor(t1)\n\
  io::print(\"bare=\" & dom391::render(n))\n\
  LET t2 AS Thread OF String TO Wrap = thread::start(worker391::buildWrap, \"seed\")\n\
  LET wr AS Wrap = thread::waitFor(t2)\n\
  io::print(\"wrap=\" & dom391::render(wr.root) & \"|\" & wr.label & \"|\" & toString(wr.ok))\n\
  RETURN 0\nEND FUNC\n";

#[test]
fn recursive_node_survives_a_thread_transfer_bare_and_in_a_record() {
    let root = unique_root("transfer");

    write_project(&root, "dom391", "package", &[], false, DOM_SRC);
    build_ok(&root, "dom391");

    write_project(
        &root,
        "worker391",
        "package",
        &["dom391"],
        false,
        WORKER_SRC,
    );
    install(&root, "dom391", "worker391", "dom391");
    build_ok(&root, "worker391");

    write_project(
        &root,
        "app391",
        "executable",
        &["dom391", "worker391"],
        true,
        APP_SRC,
    );
    install(&root, "dom391", "app391", "dom391");
    install(&root, "worker391", "app391", "worker391");

    let build = build_ok(&root, "app391");
    let exe = build
        .lines()
        .find_map(|line| line.strip_prefix("Wrote executable to "))
        .expect("app build reported no executable path")
        .trim()
        .to_string();

    let run = Command::new(&exe).output().expect("run app391");
    let stdout = String::from_utf8_lossy(&run.stdout).into_owned();
    assert!(run.status.success(), "app391 crashed:\n{stdout}");
    // The trees are read back after the worker arenas are reclaimed; correct
    // output proves the transfer was a real deep copy, not an aliasing shallow one.
    assert!(
        stdout.contains("bare=seedAseedB"),
        "bare Node transfer produced wrong text:\n{stdout}"
    );
    assert!(
        stdout.contains("wrap=seedX|Lseed|TRUE"),
        "record-embedded Node transfer produced wrong result:\n{stdout}"
    );
}
