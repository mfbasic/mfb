//! Regression tests for bug-390: a package `.mfp` could not serialize a type it
//! imported from a dependency when that type appeared in its own exported API —
//! the build aborted with the opaque `truncated binary representation`, because
//! `TypeTable::type_id`'s fallback degraded the foreign type to an empty-record
//! placeholder that later failed the field-count read.
//!
//! The fix adds a foreign-type-reference type-table kind carrying the owning
//! dependency's name, the type's original name, and the owning package's ABI
//! hash. These tests exercise the full acceptance model from source:
//!
//! ```text
//! pA : EXPORT TYPE A ; TYPE B (private) ; EXPORT TYPE C
//! pB : IMPORT pA ; EXPORT FUNC takesA(a AS A) AS Integer   (A in an exported ARG)
//! pC : IMPORT pA ; EXPORT FUNC makesA() AS A               (A in an exported RESULT)
//! app: IMPORT pB, pC ; wires pC::makesA() -> pB::takesA(...)
//! ```
//!
//! They build every package from source (no committed binary goldens to churn)
//! and assert: pB/pC build; app links and runs with the value round-tripping to
//! 42 (both foreign references resolve to one `pA::A`); pB re-exports only the
//! surfaced `A` (never the private `B` nor the unused `C`); the consumer can name
//! `A` but not the private `B`; and a dependency set whose intermediaries were
//! built against ABI-incompatible versions of `pA` is rejected at build.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn unique_root(name: &str) -> PathBuf {
    let nonce = common::unique_nonce();
    let root = std::env::temp_dir().join(format!("mfb_bug390_{name}_{nonce}"));
    fs::create_dir_all(&root).expect("create root");
    root
}

fn mfb() -> Command {
    let mut command = Command::new(common::mfb_exe());
    // An empty per-run key store: package builds with `file:`/local dependencies
    // are permitted unsigned, and this keeps the result independent of whatever
    // registry the developer machine has authed against (see test-accept.sh).
    command.env("MFB_HOME", std::env::temp_dir().join("mfb_bug390_home"));
    command
}

/// Scaffold a package/executable project. `deps` are `(name, relative source)`
/// entries recorded in `packages[]`; the caller installs each `<name>.mfp`.
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
         \"description\":\"bug-390 fixture\",\
         \"sources\":[{{\"root\":\"src\",\"role\":\"{role}\",\"include\":[\"**/*.mfb\"]}}],\
         {entry_field}\"packages\":[{packages}]}}\n"
    );
    fs::write(dir.join("project.json"), manifest).expect("write manifest");
    let src_name = if entry { "main.mfb" } else { "lib.mfb" };
    fs::write(dir.join("src").join(src_name), source).expect("write source");
}

/// `mfb build <dir>`; returns the full combined output. Panics if the build
/// fails (use `build_expect_failure` for the negative cases).
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

fn build_expect_failure(root: &Path, name: &str) -> String {
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
        !out.status.success(),
        "expected `{name}` build to FAIL, but it succeeded:\n{combined}"
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

const PA_SRC: &str = "EXPORT TYPE A\n  n AS Integer\n  label AS String\nEND TYPE\n\
                      TYPE B\n  secret AS Integer\nEND TYPE\n\
                      EXPORT TYPE C\n  flag AS Boolean\nEND TYPE\n\
                      EXPORT FUNC makeB() AS Integer\n  LET b AS B = B[42]\n  RETURN b.secret\nEND FUNC\n";
const PB_SRC: &str =
    "IMPORT pa390\nEXPORT FUNC takesA(a AS A) AS Integer\n  RETURN a.n * 2\nEND FUNC\n";
const PC_SRC: &str =
    "IMPORT pa390\nEXPORT FUNC makesA() AS A\n  RETURN A[21, \"made\"]\nEND FUNC\n";

/// The full acceptance model: pB/pC build, app runs, value round-trips to 42.
#[test]
fn foreign_type_reexport_round_trips_through_two_packages() {
    let root = unique_root("roundtrip");
    write_project(&root, "pa390", "package", &[], false, PA_SRC);
    build_ok(&root, "pa390");

    write_project(&root, "pb390", "package", &["pa390"], false, PB_SRC);
    install(&root, "pa390", "pb390", "pa390");
    build_ok(&root, "pb390");

    write_project(&root, "pc390", "package", &["pa390"], false, PC_SRC);
    install(&root, "pa390", "pc390", "pa390");
    build_ok(&root, "pc390");

    // pB surfaces only the type it names in its own API: `A`. It must not
    // re-export the private `B` (never in pA's ABI) nor the unused `C` (pA
    // exports it, but pB names no `C`).
    let info = mfb()
        .args(["pkg", "info"])
        .arg(root.join("pb390/pb390.mfp"))
        .output()
        .expect("pkg info");
    let info = String::from_utf8_lossy(&info.stdout).into_owned();
    assert!(info.contains("TYPE A"), "pB should re-export A:\n{info}");
    assert!(
        !info.contains("TYPE C"),
        "pB must not re-export unused C:\n{info}"
    );
    assert!(
        !info.contains("TYPE B"),
        "pB must not re-export private B:\n{info}"
    );

    // app declares only pB and pC; pA is a transitive dependency whose `.mfp`
    // must be present for the merge and the foreign-type resolution.
    let app_src = concat!(
        "IMPORT io AS console\n",
        "IMPORT pb390\n",
        "IMPORT pc390\n\n",
        "FUNC main AS Integer\n",
        "  LET v AS A = pc390::makesA()\n",
        "  LET r AS Integer = pb390::takesA(v)\n",
        "  console::print(toString(r))\n",
        "  RETURN 0\n",
        "END FUNC\n",
    );
    write_project(
        &root,
        "app390",
        "executable",
        &["pb390", "pc390"],
        true,
        app_src,
    );
    install(&root, "pb390", "app390", "pb390");
    install(&root, "pc390", "app390", "pc390");
    install(&root, "pa390", "app390", "pa390"); // transitive

    let build = build_ok(&root, "app390");
    let exe = build
        .lines()
        .find_map(|line| line.strip_prefix("Wrote executable to "))
        .expect("app build reported no executable path")
        .trim()
        .to_string();
    let run = Command::new(&exe).output().expect("run app390");
    let stdout = String::from_utf8_lossy(&run.stdout).into_owned();
    assert!(run.status.success(), "app390 crashed:\n{stdout}");
    assert!(
        stdout.contains("42"),
        "expected the pC->pB round-trip to print 42, got:\n{stdout}"
    );
}

/// The consumer may name the re-exported `A` (proven above) but not pA's private
/// `B`, which is in no ABI surface.
#[test]
fn consumer_cannot_name_a_dependencys_private_type() {
    let root = unique_root("private");
    write_project(&root, "pa390", "package", &[], false, PA_SRC);
    build_ok(&root, "pa390");
    write_project(&root, "pb390", "package", &["pa390"], false, PB_SRC);
    install(&root, "pa390", "pb390", "pa390");
    build_ok(&root, "pb390");

    // `B` is private in pA and re-exported by no one, so naming it must fail.
    let app_src = "IMPORT pb390\nFUNC main AS Integer\n  LET x AS B = B[1]\n  RETURN 0\nEND FUNC\n";
    write_project(&root, "app390", "executable", &["pb390"], true, app_src);
    install(&root, "pb390", "app390", "pb390");
    install(&root, "pa390", "app390", "pa390");
    let out = build_expect_failure(&root, "app390");
    assert!(
        out.contains("B"),
        "the private-type error should mention `B`:\n{out}"
    );
}

/// Two intermediaries built against ABI-incompatible versions of pA must not
/// silently interoperate — the consumer build is rejected.
#[test]
fn abi_incompatible_dependency_versions_are_rejected() {
    let root = unique_root("incompat");
    // pA-v1: A has two fields; pA-v2: A has one — same name/version, different ABI.
    write_project(
        &root,
        "pa1",
        "package",
        &[],
        false,
        "EXPORT TYPE A\n  n AS Integer\n  label AS String\nEND TYPE\n",
    );
    write_project(
        &root,
        "pa2",
        "package",
        &[],
        false,
        "EXPORT TYPE A\n  n AS Integer\nEND TYPE\n",
    );
    // Both are the package `pa390` (same manifest name), just different shapes.
    for (dir, fields) in [
        ("pa1", "n AS Integer\n  label AS String"),
        ("pa2", "n AS Integer"),
    ] {
        fs::write(
            root.join(dir).join("project.json"),
            "{\"name\":\"pa390\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"package\",\
             \"description\":\"bug-390\",\
             \"sources\":[{\"root\":\"src\",\"role\":\"package\",\"include\":[\"**/*.mfb\"]}],\
             \"packages\":[]}\n",
        )
        .unwrap();
        fs::write(
            root.join(dir).join("src/lib.mfb"),
            format!("EXPORT TYPE A\n  {fields}\nEND TYPE\n"),
        )
        .unwrap();
    }
    build_ok(&root, "pa1");
    build_ok(&root, "pa2");

    write_project(&root, "pb390", "package", &["pa390"], false, PB_SRC);
    fs::copy(
        root.join("pa1/pa390.mfp"),
        root.join("pb390/packages/pa390.mfp"),
    )
    .unwrap();
    build_ok(&root, "pb390");

    write_project(&root, "pc390", "package", &["pa390"], false, PC_SRC);
    fs::copy(
        root.join("pa2/pa390.mfp"),
        root.join("pc390/packages/pa390.mfp"),
    )
    .unwrap();
    build_ok(&root, "pc390");

    let app_src = "IMPORT pb390\nIMPORT pc390\n\
                   FUNC main AS Integer\n  RETURN pb390::takesA(pc390::makesA())\nEND FUNC\n";
    write_project(
        &root,
        "app390",
        "executable",
        &["pb390", "pc390"],
        true,
        app_src,
    );
    install(&root, "pb390", "app390", "pb390");
    install(&root, "pc390", "app390", "pc390");
    let out = build_expect_failure(&root, "app390");
    assert!(
        out.contains("ABI")
            || out.to_lowercase().contains("incompatible")
            || out.contains("disagree"),
        "expected an ABI-incompatibility error, got:\n{out}"
    );
}
