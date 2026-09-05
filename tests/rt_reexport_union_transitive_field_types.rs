//! Regression test for bug-435: a package that re-exports a dependency's union
//! was not self-contained when one of that union's variant fields referenced
//! another user type (a record or enum) from the owner package. The reader
//! (`read_package_type_exports_resolved`) filled the re-exported union's own
//! variants from the owner `.mfp` but never walked those variants' field types
//! to pull in the *other* user types they reference, so an importer of the
//! intermediary rejected the package with:
//!
//! ```text
//! error[6-605-0001 PACKAGE_INVALID]: ... exported union `Node`
//!   that references unknown type `Meta`.
//! ```
//!
//! The model mirrors the browser example that first surfaced it:
//!
//! ```text
//! leaf : ENUM Kind ; TYPE Meta{kind AS Kind} ; TYPE Box{meta AS Meta, kids AS List OF Node}
//!        TYPE Leaf ; UNION Node{Box, Leaf}      (Meta/Kind reachable only through Box)
//! mid  : IMPORT leaf ; EXPORT FUNC describe(n AS Node) AS String   (re-exports Node)
//! app  : IMPORT mid   (NOT leaf) — must build even though it never names the owner
//! ```
//!
//! Everything is built from source (no committed binary goldens to churn).

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn unique_root(name: &str) -> PathBuf {
    let nonce = common::unique_nonce();
    let root = std::env::temp_dir().join(format!("mfb_bug435_{name}_{nonce}"));
    fs::create_dir_all(&root).expect("create root");
    root
}

fn mfb() -> Command {
    let mut command = Command::new(common::mfb_exe());
    // An empty per-run key store keeps `file:` local-dependency builds unsigned
    // and independent of whatever registry the dev machine authed against.
    command.env("MFB_HOME", std::env::temp_dir().join("mfb_bug435_home"));
    command
}

/// Scaffold a package/executable project. `deps` are dependency package names;
/// the caller installs each `<name>.mfp` into `packages/`.
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
         \"description\":\"bug-435 fixture\",\
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

fn build_ok(root: &Path, name: &str) -> String {
    let (ok, combined) = build(root, name);
    assert!(ok, "expected `{name}` to build, but it failed:\n{combined}");
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

// `Meta` and its enum `Kind` are reachable ONLY through `Box.meta`, a field of a
// union variant — exactly the transitive edge the resolver used to drop.
const LEAF_SRC: &str = "\
EXPORT ENUM Kind
  Alpha
  Beta
END ENUM
EXPORT TYPE Meta
  kind AS Kind
  n AS Integer
END TYPE
EXPORT TYPE Box
  meta AS Meta
  kids AS List OF Node
END TYPE
EXPORT TYPE Leaf
  text AS String
END TYPE
EXPORT UNION Node
  Box
  Leaf
END UNION
EXPORT FUNC makeNode() AS Node
  RETURN Box[Meta[Kind.Alpha, 1], []]
END FUNC
";

const MID_SRC: &str = "\
IMPORT leaf435

EXPORT FUNC describe(n AS Node) AS String
  RETURN \"node\"
END FUNC
";

/// The core bug: `app` imports only `mid435` (never the owner `leaf435`), so the
/// re-exported `Node`'s closure must already carry `Meta`/`Kind` or the build is
/// rejected with `PACKAGE_INVALID ... references unknown type Meta`.
#[test]
fn importer_of_reexported_union_resolves_transitive_field_types() {
    let root = unique_root("useonly_mid");
    write_project(&root, "leaf435", "package", &[], false, LEAF_SRC);
    build_ok(&root, "leaf435");

    write_project(&root, "mid435", "package", &["leaf435"], false, MID_SRC);
    install(&root, "leaf435", "mid435", "leaf435");
    build_ok(&root, "mid435");

    let app_src = "\
IMPORT io AS console
IMPORT mid435

FUNC main AS Integer
  console::print(\"ok\")
  RETURN 0
END FUNC
";
    write_project(&root, "app435", "executable", &["mid435"], true, app_src);
    install(&root, "mid435", "app435", "mid435");
    // The owner `.mfp` is present as a sibling for the resolver to read, but the
    // app never IMPORTs it — the pre-fix failure occurs regardless.
    install(&root, "leaf435", "app435", "leaf435");

    let (ok, combined) = build(&root, "app435");
    assert!(
        ok,
        "app435 must build: the re-exported `Node` closure should carry the \
         transitively-referenced `Meta`/`Kind`, but the build failed:\n{combined}"
    );

    // Prove the merged package is actually runnable, not merely accepted.
    let exe = combined
        .lines()
        .find_map(|line| line.strip_prefix("Wrote executable to "))
        .expect("app build reported no executable path")
        .trim()
        .to_string();
    let run = Command::new(&exe).output().expect("run app435");
    let stdout = String::from_utf8_lossy(&run.stdout).into_owned();
    assert!(run.status.success(), "app435 crashed:\n{stdout}");
    assert!(stdout.contains("ok"), "expected `ok`, got:\n{stdout}");
}

// bug-436: the same re-export, but the intermediary names the imported union in
// its **package-qualified** form (`leaf435::Node`) in the exported signature.
// The parser lowers this to the dotted IR type name `leaf435.Node`, which the ABI
// writer's `TypeTable::type_id` failed to resolve — the bare `Node` foreign-type
// entry is keyed under `Node`, so the dotted name missed and degraded to an
// empty-record placeholder that failed its own read-back with
// `truncated binary representation`, writing no `.mfp`.
const MID_SRC_QUALIFIED: &str = "\
IMPORT leaf435

EXPORT FUNC describe(n AS leaf435::Node) AS String
  RETURN \"node\"
END FUNC
";

/// A package-qualified imported type in an exported signature must build a valid
/// `.mfp` equivalent to the unqualified spelling — never emit a corrupt/no `.mfp`
/// under a `Building …` line (bug-436). The whole three-package chain must build
/// and run just as the unqualified form does.
#[test]
fn reexported_union_qualified_type_reference_builds_equivalent_package() {
    let root = unique_root("qualified_mid");
    write_project(&root, "leaf435", "package", &[], false, LEAF_SRC);
    build_ok(&root, "leaf435");

    write_project(
        &root,
        "mid435",
        "package",
        &["leaf435"],
        false,
        MID_SRC_QUALIFIED,
    );
    install(&root, "leaf435", "mid435", "leaf435");
    // Pre-fix: this build printed `Building mid435 …` then
    // `error: truncated binary representation` and wrote no `.mfp`.
    build_ok(&root, "mid435");
    assert!(
        root.join("mid435").join("mid435.mfp").is_file(),
        "mid435.mfp must be written for the qualified spelling"
    );

    let app_src = "\
IMPORT io AS console
IMPORT mid435

FUNC main AS Integer
  console::print(\"ok\")
  RETURN 0
END FUNC
";
    write_project(&root, "app435", "executable", &["mid435"], true, app_src);
    install(&root, "mid435", "app435", "mid435");
    install(&root, "leaf435", "app435", "leaf435");

    let (ok, combined) = build(&root, "app435");
    assert!(
        ok,
        "app435 must build against the qualified-form mid435:\n{combined}"
    );

    let exe = combined
        .lines()
        .find_map(|line| line.strip_prefix("Wrote executable to "))
        .expect("app build reported no executable path")
        .trim()
        .to_string();
    let run = Command::new(&exe).output().expect("run app435");
    let stdout = String::from_utf8_lossy(&run.stdout).into_owned();
    assert!(run.status.success(), "app435 crashed:\n{stdout}");
    assert!(stdout.contains("ok"), "expected `ok`, got:\n{stdout}");
}
