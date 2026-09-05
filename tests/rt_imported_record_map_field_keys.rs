//! Regression test for the imported-record-Map-field bug: a consumer package
//! that iterated a field of a type it *imported* from a dependency — e.g.
//! `collections::keys(f.props)` where `props AS Map OF String TO String` — could
//! not be built. IR lowering built its `TypeIndex` from the consumer's own AST
//! only, so an imported type carried no field layout and every `record.field` on
//! it lowered to `Unknown`. `getOr`/`len`/`hasKey` tolerate `Unknown`, but
//! `collections::keys`/`values` need the concrete element type, so the build
//! failed (a `TYPE_CALL_ARGUMENT_MISMATCH`, or `native plan has no storage class
//! for type Unknown` once a downstream consumer forced codegen).
//!
//! The fix decodes each imported (non-builtin) package's `.mfp` type exports and
//! folds their record/union/enum layouts into the lowering `TypeIndex`, so an
//! imported `record.field` types exactly as a local one would. These tests build
//! every package from source and prove a consumer can iterate both a standalone
//! imported record's Map field and an imported *union variant's* Map field.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn unique_root(name: &str) -> PathBuf {
    let nonce = common::unique_nonce();
    let root = std::env::temp_dir().join(format!("mfb_imported_map_{name}_{nonce}"));
    fs::create_dir_all(&root).expect("create root");
    root
}

fn mfb() -> Command {
    let mut command = Command::new(common::mfb_exe());
    command.env(
        "MFB_HOME",
        std::env::temp_dir().join("mfb_imported_map_home"),
    );
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
         \"description\":\"imported-map-field fixture\",\
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

// A dependency exporting BOTH a standalone record with a Map field and a union
// whose variant carries a Map field — the two ways an imported field is named.
const DOM_SRC: &str = "IMPORT collections\n\
EXPORT TYPE Style\n  props AS Map OF String TO String\nEND TYPE\n\
EXPORT TYPE Rule\n  selector AS String\n  props AS Map OF String TO String\nEND TYPE\n\
EXPORT TYPE Bare\n  label AS String\nEND TYPE\n\
EXPORT UNION Node\n  Rule\n  Bare\nEND UNION\n\
EXPORT FUNC makeStyle() AS Style\n  MUT m AS Map OF String TO String = Map OF String TO String {}\n  m = collections::set(m, \"color\", \"red\")\n  m = collections::set(m, \"size\", \"big\")\n  LET s AS Style = Style[m]\n  RETURN s\nEND FUNC\n\
EXPORT FUNC makeRule() AS Node\n  MUT m AS Map OF String TO String = Map OF String TO String {}\n  m = collections::set(m, \"weight\", \"bold\")\n  LET n AS Node = Rule[\"h1\", m]\n  RETURN n\nEND FUNC\n";

// The consumer names the imported types and iterates their Map fields directly:
// `collections::keys` over a standalone imported record's field, and over an
// imported union variant's field reached through a MATCH binding.
const APP_SRC: &str = "IMPORT io\nIMPORT collections\nIMPORT strings\nIMPORT dom0\n\
FUNC styleKeys(s AS Style) AS String\n  MUT out AS List OF String = []\n  FOR EACH k IN collections::keys(s.props)\n    out = collections::append(out, k & \"=\" & collections::getOr(s.props, k, \"\"))\n  NEXT\n  RETURN strings::join(out, \",\")\nEND FUNC\n\
FUNC ruleKeys(n AS Node) AS String\n  MATCH n\n    CASE Rule(r)\n      MUT out AS List OF String = []\n      FOR EACH k IN collections::keys(r.props)\n        out = collections::append(out, k & \"=\" & collections::getOr(r.props, k, \"\"))\n      NEXT\n      RETURN r.selector & \":\" & strings::join(out, \",\")\n    CASE ELSE\n      RETURN \"?\"\n  END MATCH\nEND FUNC\n\
FUNC main AS Integer\n  LET s AS Style = dom0::makeStyle()\n  io::print(\"style=\" & styleKeys(s))\n  LET n AS Node = dom0::makeRule()\n  io::print(\"rule=\" & ruleKeys(n))\n  RETURN 0\nEND FUNC\n";

#[test]
fn consumer_iterates_an_imported_records_map_field() {
    let root = unique_root("keys");

    write_project(&root, "dom0", "package", &[], false, DOM_SRC);
    build_ok(&root, "dom0");

    write_project(&root, "app0", "executable", &["dom0"], true, APP_SRC);
    install(&root, "dom0", "app0", "dom0");

    let build = build_ok(&root, "app0");
    let exe = build
        .lines()
        .find_map(|line| line.strip_prefix("Wrote executable to "))
        .expect("app build reported no executable path")
        .trim()
        .to_string();

    let run = Command::new(&exe).output().expect("run app0");
    let stdout = String::from_utf8_lossy(&run.stdout).into_owned();
    assert!(run.status.success(), "app0 crashed:\n{stdout}");
    // keys() over the standalone imported record's Map field visited both
    // entries (order is not asserted — the point is that the field typed as a
    // concrete Map instead of Unknown, so keys() lowered at all).
    assert!(
        stdout.contains("style=") && stdout.contains("color=red") && stdout.contains("size=big"),
        "iterating a standalone imported record's Map field produced wrong text:\n{stdout}"
    );
    // keys() over an imported union variant's Map field (via a MATCH binding).
    assert!(
        stdout.contains("rule=h1:") && stdout.contains("weight=bold"),
        "iterating an imported union variant's Map field produced wrong text:\n{stdout}"
    );
}
