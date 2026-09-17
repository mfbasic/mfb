//! Regression tests for bug-628: an executable that declared only `userpkg`, where
//! `userpkg` itself imports `basepkg`, failed to build with the internal, unlocated
//! `NIR call target 'basepkg.base' does not resolve`. The executable merge walks
//! only the packages its own `project.json` declares, so `basepkg` was never merged.
//!
//! The rule (spec `tooling project-manifest` §"Dependency Entries"): a project's
//! `packages[]` lists its whole dependency closure. Every entry carries `direct`
//! (the user added it) and `requiredBy` (the idents of the declared packages whose
//! import tables name it). `mfb pkg update` computes both; `mfb build` checks them
//! against the real import tables and refuses — never repairs — a manifest that
//! disagrees, and refuses a dependency that does not satisfy an importer's
//! recorded symbols.
//!
//! Every package is a source dependency built fresh, so no committed `.mfp` churns.

#[path = "../common/mod.rs"]
mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn unique_root(name: &str) -> PathBuf {
    let nonce = common::unique_nonce();
    let root = std::env::temp_dir().join(format!("mfb_bug628_{name}_{nonce}"));
    fs::create_dir_all(&root).expect("create root");
    root
}

fn mfb() -> Command {
    let mut command = Command::new(common::mfb_exe());
    // An empty per-run key store: local dependencies are permitted unsigned, and
    // this keeps the result independent of the developer machine's registry auth.
    command.env("MFB_HOME", std::env::temp_dir().join("mfb_bug628_home"));
    command
}

const BASE_SRC: &str = "EXPORT FUNC base() AS Integer\n  RETURN 5\nEND FUNC\n";
/// Same package name, incompatible `base` signature.
const BASE_V2_SRC: &str = "EXPORT FUNC base(x AS Integer) AS Integer\n  RETURN x\nEND FUNC\n";
const USER_SRC: &str =
    "IMPORT basepkg\n\nEXPORT FUNC g() AS Integer\n  RETURN basepkg::base()\nEND FUNC\n";
const APP_SRC: &str = "IMPORT userpkg\nIMPORT io\n\nFUNC main() AS Integer\n  \
                       io::print(toString(userpkg::g()))\n  RETURN 0\nEND FUNC\n";

/// Write `root/<dir>` as a project named `name`. `packages` is the literal JSON
/// body of the `packages` array.
fn write_project(root: &Path, dir: &str, name: &str, kind: &str, packages: &str, source: &str) {
    let project = root.join(dir);
    fs::create_dir_all(project.join("src")).expect("src dir");
    let (entry, file) = if kind == "executable" {
        ("\"entry\":\"main\",", "main.mfb")
    } else {
        ("", "lib.mfb")
    };
    let manifest = format!(
        "{{\"name\":\"{name}\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"{kind}\",\
         \"description\":\"bug-628 fixture\",{entry}\
         \"sources\":[{{\"root\":\"src\",\"role\":\"main\",\"include\":[\"**/*.mfb\"]}}],\
         \"packages\":[{packages}]}}\n"
    );
    fs::write(project.join("project.json"), manifest).expect("write manifest");
    fs::write(project.join("src").join(file), source).expect("write source");
}

/// `basepkg` and a `userpkg` that correctly declares it.
fn write_packages(root: &Path) {
    write_project(root, "basepkg", "basepkg", "package", "", BASE_SRC);
    write_project(
        root,
        "userpkg",
        "userpkg",
        "package",
        "{\"name\":\"basepkg\",\"version\":\"=0.1.0\",\"source\":\"file:../basepkg\",\
         \"direct\":true,\"requiredBy\":[]}",
        USER_SRC,
    );
}

fn run(command: &mut Command) -> (bool, String) {
    let out = command.output().expect("run mfb");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), combined)
}

fn build(root: &Path, dir: &str) -> (bool, String) {
    run(mfb().arg("build").arg(root.join(dir)))
}

fn built_executable(output: &str) -> String {
    output
        .lines()
        .find_map(|line| line.strip_prefix("Wrote executable to "))
        .unwrap_or_else(|| panic!("build reported no executable path:\n{output}"))
        .trim()
        .to_string()
}

fn assert_runs_and_prints_5(output: &str) {
    let exe = built_executable(output);
    let run = Command::new(&exe).output().expect("run app");
    let stdout = String::from_utf8_lossy(&run.stdout).into_owned();
    assert!(run.status.success(), "app crashed:\n{stdout}");
    assert_eq!(stdout.trim(), "5", "unexpected app output");
}

/// The bug's exact repro: the build must be refused by the manifest check, name
/// the undeclared package and who needs it, and point at `mfb pkg update` — not
/// reach the internal NIR error.
#[test]
fn undeclared_transitive_dependency_is_refused_before_lowering() {
    let root = unique_root("undeclared");
    write_packages(&root);
    write_project(
        &root,
        "app",
        "app",
        "executable",
        "{\"name\":\"userpkg\",\"version\":\"=0.1.0\",\"source\":\"file:../userpkg\",\
         \"direct\":true,\"requiredBy\":[]}",
        APP_SRC,
    );
    let (ok, output) = build(&root, "app");
    assert!(!ok, "the build must be refused:\n{output}");
    assert!(
        !output.contains("does not resolve"),
        "the internal NIR error must never be reached:\n{output}"
    );
    assert!(
        output.contains("PACKAGE_DEPENDENCIES_INCONSISTENT"),
        "expected the manifest diagnostic:\n{output}"
    );
    assert!(
        output.contains("`basepkg`") && output.contains("`userpkg`"),
        "the diagnostic must name the missing package and its requirer:\n{output}"
    );
    assert!(
        output.contains("mfb pkg update"),
        "the diagnostic must tell the user to run `mfb pkg update`:\n{output}"
    );
}

/// `mfb pkg update` writes the closure — the transitive entry with its source
/// rebased onto the app, `direct: false`, `requiredBy` the requirer's ident — and
/// the app then builds and runs.
#[test]
fn pkg_update_declares_the_closure_and_the_app_runs() {
    let root = unique_root("update");
    write_packages(&root);
    write_project(
        &root,
        "app",
        "app",
        "executable",
        "{\"name\":\"userpkg\",\"version\":\"=0.1.0\",\"source\":\"file:../userpkg\"}",
        APP_SRC,
    );
    let (ok, output) = run(mfb().args(["pkg", "update"]).current_dir(root.join("app")));
    assert!(ok, "`mfb pkg update` failed:\n{output}");

    let manifest = fs::read_to_string(root.join("app/project.json")).expect("read manifest");
    let value: serde_json::Value = serde_json::from_str(&manifest).expect("manifest is JSON");
    let packages = value["packages"].as_array().expect("packages array");
    let entry = |name: &str| {
        packages
            .iter()
            .find(|p| p["name"] == name)
            .unwrap_or_else(|| panic!("no `{name}` entry:\n{manifest}"))
    };
    assert_eq!(entry("userpkg")["direct"], true, "{manifest}");
    assert_eq!(
        entry("userpkg")["requiredBy"],
        serde_json::json!([]),
        "{manifest}"
    );
    assert_eq!(entry("basepkg")["direct"], false, "{manifest}");
    assert_eq!(
        entry("basepkg")["requiredBy"],
        serde_json::json!(["userpkg"]),
        "{manifest}"
    );
    assert_eq!(entry("basepkg")["source"], "file:../basepkg", "{manifest}");

    let (ok, output) = build(&root, "app");
    assert!(
        ok,
        "the app must build once the closure is declared:\n{output}"
    );
    assert_runs_and_prints_5(&output);
}

/// `mfb pkg add file://…/userpkg.mfp` declares and installs the `basepkg.mfp`
/// sitting beside it — the add a user performs by hand, closed like any other.
#[test]
fn pkg_add_of_a_compiled_package_declares_its_sibling_dependency() {
    let root = unique_root("add_file");
    write_packages(&root);
    let (ok, output) = build(&root, "userpkg");
    assert!(ok, "userpkg must build:\n{output}");
    let dist = root.join("dist");
    fs::create_dir_all(&dist).expect("dist dir");
    fs::copy(
        root.join("userpkg/build/packages/basepkg.mfp"),
        dist.join("basepkg.mfp"),
    )
    .expect("stage basepkg.mfp");
    fs::copy(root.join("userpkg/userpkg.mfp"), dist.join("userpkg.mfp")).expect("stage userpkg");
    write_project(&root, "app", "app", "executable", "", APP_SRC);

    let url = format!("file://{}", dist.join("userpkg.mfp").display());
    let (ok, output) = run(mfb()
        .args(["pkg", "add", &url])
        .current_dir(root.join("app")));
    assert!(ok, "`mfb pkg add` failed:\n{output}");
    assert!(
        output.contains("Declared package basepkg (required by userpkg)"),
        "the add must say what it declared:\n{output}"
    );
    assert!(
        root.join("app/packages/basepkg.mfp").is_file(),
        "basepkg.mfp must be installed"
    );
    let manifest = fs::read_to_string(root.join("app/project.json")).expect("read manifest");
    let value: serde_json::Value = serde_json::from_str(&manifest).expect("manifest is JSON");
    let base = value["packages"]
        .as_array()
        .expect("packages array")
        .iter()
        .find(|p| p["name"] == "basepkg")
        .unwrap_or_else(|| panic!("no basepkg entry:\n{manifest}"));
    assert_eq!(base["direct"], false, "{manifest}");
    assert_eq!(
        base["requiredBy"],
        serde_json::json!(["userpkg"]),
        "{manifest}"
    );

    let (ok, output) = build(&root, "app");
    assert!(ok, "the app must build:\n{output}");
    assert_runs_and_prints_5(&output);
}

/// A hand-written, correct closure builds and runs.
#[test]
fn a_declared_closure_builds_and_runs() {
    let root = unique_root("declared");
    write_packages(&root);
    write_project(
        &root,
        "app",
        "app",
        "executable",
        "{\"name\":\"userpkg\",\"version\":\"=0.1.0\",\"source\":\"file:../userpkg\",\
         \"direct\":true,\"requiredBy\":[]},\
         {\"name\":\"basepkg\",\"version\":\"=0.1.0\",\"source\":\"file:../basepkg\",\
         \"direct\":false,\"requiredBy\":[\"userpkg\"]}",
        APP_SRC,
    );
    let (ok, output) = build(&root, "app");
    assert!(ok, "a correct closure must build:\n{output}");
    assert_runs_and_prints_5(&output);
}

/// Every stale `requiredBy`, a missing field, and an orphaned indirect entry are
/// all listed in one refusal; nothing is repaired.
#[test]
fn stale_dependency_fields_are_listed_and_refused() {
    let root = unique_root("stale");
    write_packages(&root);
    write_project(
        &root,
        "orphan",
        "orphan",
        "package",
        "",
        BASE_SRC.replace("base", "o").as_str(),
    );
    write_project(
        &root,
        "app",
        "app",
        "executable",
        // userpkg: no `direct`; basepkg: requiredBy omits userpkg; orphan: indirect
        // but required by nothing.
        "{\"name\":\"userpkg\",\"version\":\"=0.1.0\",\"source\":\"file:../userpkg\",\
         \"requiredBy\":[]},\
         {\"name\":\"basepkg\",\"version\":\"=0.1.0\",\"source\":\"file:../basepkg\",\
         \"direct\":false,\"requiredBy\":[]},\
         {\"name\":\"orphan\",\"version\":\"=0.1.0\",\"source\":\"file:../orphan\",\
         \"direct\":false,\"requiredBy\":[]}",
        APP_SRC,
    );
    let before = fs::read_to_string(root.join("app/project.json")).expect("read manifest");
    let (ok, output) = build(&root, "app");
    assert!(!ok, "a stale manifest must be refused:\n{output}");
    for needle in [
        "PACKAGE_DEPENDENCIES_INCONSISTENT",
        "`userpkg`",
        "`direct`",
        "`basepkg`",
        "requiredBy",
        "`orphan`",
        "mfb pkg update",
    ] {
        assert!(output.contains(needle), "missing `{needle}` in:\n{output}");
    }
    let after = fs::read_to_string(root.join("app/project.json")).expect("read manifest");
    assert_eq!(before, after, "the build must never rewrite project.json");
}

/// The app declares a `basepkg` whose `base` is not the one `userpkg` was built
/// against: a version conflict, refused before compiling, pointing at
/// `mfb pkg verify` — which then names the requirer, the package, and the symbol.
#[test]
fn incompatible_dependency_version_is_refused_and_verify_explains_it() {
    let root = unique_root("conflict");
    write_packages(&root);
    write_project(&root, "basepkg-v2", "basepkg", "package", "", BASE_V2_SRC);
    write_project(
        &root,
        "app",
        "app",
        "executable",
        "{\"name\":\"userpkg\",\"version\":\"=0.1.0\",\"source\":\"file:../userpkg\",\
         \"direct\":true,\"requiredBy\":[]},\
         {\"name\":\"basepkg\",\"version\":\"=0.1.0\",\"source\":\"file:../basepkg-v2\",\
         \"direct\":false,\"requiredBy\":[\"userpkg\"]}",
        APP_SRC,
    );
    let (ok, output) = build(&root, "app");
    assert!(!ok, "an incompatible dependency must be refused:\n{output}");
    assert!(
        !output.contains("does not resolve") && !output.contains("Building app"),
        "the conflict must stop the build before compiling the app:\n{output}"
    );
    assert!(
        output.contains("PACKAGE_VERSION_CONFLICT") && output.contains("mfb pkg verify"),
        "expected the conflict diagnostic pointing at `mfb pkg verify`:\n{output}"
    );

    let (ok, output) = run(mfb().args(["pkg", "verify"]).current_dir(root.join("app")));
    assert!(!ok, "`mfb pkg verify` must fail on a conflict:\n{output}");
    for needle in ["`userpkg`", "`basepkg`", "`base`"] {
        assert!(
            output.contains(needle),
            "verify must name {needle}:\n{output}"
        );
    }
}
