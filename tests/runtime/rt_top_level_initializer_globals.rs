//! A top-level `LET`/`MUT` initializer may read a global — the program's own or
//! an imported package's — and sees that global's INITIALIZED value.
//!
//! bug-612: `ir::lower::lower_binding` lowered the initializer with the whole
//! global type table as its local scope, so every global name in it lowered as
//! `IrValue::Local`, which nothing defines. The build died with the unlocated
//! internal `NIR local reference '<name>' does not resolve` — and a PACKAGE
//! whose own `EXPORT LET B = A` built its `.mfp` with exit 0 and broke every
//! importer.
//!
//! bug-613: the merged initializer stored bindings in `IrProject::bindings`
//! order, and `merge_package` appended each package's bindings AFTER the
//! consumer's. A consumer initializer that reached a package global (directly,
//! once bug-612 let it, or through a package function) read the still-zero
//! slot: a silent wrong value. Packages must initialize before every importer,
//! including one package that imports another — hence the chain test, whose
//! manifest lists the dependent package FIRST so an order that merely follows
//! the manifest cannot pass it.

#[path = "../common/mod.rs"]
mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// A source package: its name, its `lib.mfb`, and the packages it imports.
struct Package<'a> {
    name: &'a str,
    source: &'a str,
    depends_on: &'a [&'a str],
}

/// The bug-551 package: a constant, mutable state and an accessor for it.
const LIMITS: Package<'static> = Package {
    name: "limits",
    source: "EXPORT LET Answer AS Integer = 42\n\
             EXPORT MUT Counter AS Integer = 7\n\
             \n\
             EXPORT FUNC counter() AS Integer\n\
            \x20 RETURN Counter\n\
             END FUNC\n",
    depends_on: &[],
};

/// The `packages[]` entries for `names`. bug-628: each records `requiredBy` — the
/// listed packages whose own `depends_on` names it.
fn package_entries(names: &[&str], packages: &[Package<'_>]) -> String {
    names
        .iter()
        .map(|name| {
            let required_by: Vec<String> = names
                .iter()
                .filter(|other| {
                    packages
                        .iter()
                        .any(|p| p.name == **other && p.depends_on.contains(name))
                })
                .map(|other| format!("\"{other}\""))
                .collect();
            format!(
                "{{\"name\":\"{name}\",\"version\":\"=0.1.0\",\"source\":\"file:../{name}\",\
                 \"direct\":true,\"requiredBy\":[{}]}}",
                required_by.join(",")
            )
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// Write every package plus an executable importing `app_packages` (in that
/// manifest order), build, run, and return the combined output.
fn run_app(test: &str, packages: &[Package<'_>], app_packages: &[&str], main: &str) -> String {
    let root = std::env::temp_dir().join(format!("mfb_{test}_{}", common::unique_nonce()));
    for package in packages {
        let dir = root.join(package.name);
        fs::create_dir_all(dir.join("src")).expect("create package dir");
        fs::write(
            dir.join("project.json"),
            format!(
                "{{\"name\":\"{}\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"package\",\
                 \"description\":\"a package\",\
                 \"sources\":[{{\"root\":\"src\",\"role\":\"package\",\"include\":[\"**/*.mfb\"]}}],\
                 \"packages\":[{}]}}\n",
                package.name,
                package_entries(package.depends_on, packages)
            ),
        )
        .expect("write package manifest");
        fs::write(dir.join("src/lib.mfb"), package.source).expect("write package source");
    }
    let app = root.join("app");
    fs::create_dir_all(app.join("src")).expect("create app dir");
    fs::write(
        app.join("project.json"),
        format!(
            "{{\"name\":\"initapp\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"executable\",\
             \"description\":\"an importer\",\
             \"sources\":[{{\"root\":\"src\",\"role\":\"main\",\"include\":[\"**/*.mfb\"]}}],\
             \"packages\":[{}],\
             \"entry\":\"main\",\"targets\":[\"native\"]}}\n",
            package_entries(app_packages, packages)
        ),
    )
    .expect("write app manifest");
    fs::write(app.join("src/main.mfb"), main).expect("write app source");

    let executable = build(&app);
    let output = Command::new(&executable).output().expect("run the app");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.status.success(),
        "the app exited {}:\n{combined}",
        common::exit_description(&output.status)
    );
    let _ = fs::remove_dir_all(&root);
    combined
}

fn build(project: &Path) -> PathBuf {
    let output = Command::new(common::mfb_exe())
        .arg("build")
        .arg(project)
        .output()
        .expect("run mfb build");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "build failed:\n{combined}");
    let path = combined
        .lines()
        .find_map(|line| line.strip_prefix("Wrote executable to "))
        .expect("build output names an executable");
    PathBuf::from(path)
}

fn lines(output: &str) -> Vec<&str> {
    output.lines().collect()
}

/// bug-612's own-global row: no package involved at all.
#[test]
fn a_top_level_initializer_reads_the_programs_own_global() {
    let output = run_app(
        "init_own_global",
        &[],
        &[],
        "IMPORT io\n\
         \n\
         LET own AS Integer = 7\n\
         MUT counted AS Integer = own + 1\n\
         LET top = own\n\
         LET after AS Integer = counted * 10\n\
         \n\
         FUNC main() AS Integer\n\
        \x20 io::print(toString(top))\n\
        \x20 io::print(toString(after))\n\
        \x20 RETURN 0\n\
         END FUNC\n",
    );
    assert_eq!(
        lines(&output),
        vec!["7", "80"],
        "an initializer must read the global's value, in declaration order:\n{output}"
    );
}

/// bug-612's reproduction and its typed / `MUT` / aliased rows. Every one of
/// these also needs bug-613: the read is of a PACKAGE slot, which must already
/// hold its initializer's value.
#[test]
fn a_top_level_initializer_reads_an_imported_package_global() {
    let output = run_app(
        "init_package_global",
        &[LIMITS],
        &["limits"],
        "IMPORT limits\n\
         IMPORT limits AS lim\n\
         IMPORT io\n\
         \n\
         LET inferred = limits::Answer\n\
         LET typed AS Integer = limits::Answer\n\
         MUT state AS Integer = limits::Counter\n\
         LET aliased = lim::Answer + 1\n\
         \n\
         FUNC main() AS Integer\n\
        \x20 io::print(toString(inferred))\n\
        \x20 io::print(toString(typed))\n\
        \x20 io::print(toString(state))\n\
        \x20 io::print(toString(aliased))\n\
        \x20 RETURN 0\n\
         END FUNC\n",
    );
    assert_eq!(
        lines(&output),
        vec!["42", "42", "7", "43"],
        "a top-level read of a package global must see its initialized value:\n{output}"
    );
}

/// bug-612's package row: the package's OWN `EXPORT LET B = A` must produce a
/// `.mfp` an importer can use, even one that only reads `B` inside a function.
#[test]
fn a_package_initializer_reads_the_packages_own_global() {
    let output = run_app(
        "init_package_own_global",
        &[Package {
            name: "chainlet",
            source: "EXPORT LET A AS Integer = 1\n\
                     EXPORT LET B AS Integer = A\n",
            depends_on: &[],
        }],
        &["chainlet"],
        "IMPORT chainlet\n\
         IMPORT io\n\
         \n\
         FUNC main() AS Integer\n\
        \x20 io::print(toString(chainlet::B))\n\
        \x20 RETURN 0\n\
         END FUNC\n",
    );
    assert_eq!(lines(&output), vec!["1"], "{output}");
}

/// bug-613's reproduction: the read goes through a package FUNCTION, which is
/// the form that built before bug-612 and silently printed `0`.
#[test]
fn a_top_level_initializer_calling_a_package_sees_its_initialized_globals() {
    let output = run_app(
        "init_package_call",
        &[LIMITS],
        &["limits"],
        "IMPORT limits\n\
         IMPORT io\n\
         \n\
         LET top = limits::counter()\n\
         \n\
         FUNC main() AS Integer\n\
        \x20 io::print(toString(top))\n\
        \x20 RETURN 0\n\
         END FUNC\n",
    );
    assert_eq!(
        lines(&output),
        vec!["7"],
        "the package's globals must be initialized before the importer's:\n{output}"
    );
}

/// bug-613's chain: app → `upper` → `base`, where `upper`'s own initializer
/// calls into `base`. The app's manifest lists `upper` BEFORE `base`, so an
/// order taken from the manifest (or from merge order) initializes `upper`
/// against a zero `base::Seed`.
#[test]
fn a_package_initializer_calling_another_package_sees_its_initialized_globals() {
    let output = run_app(
        "init_package_chain",
        &[
            Package {
                name: "base",
                source: "EXPORT MUT Seed AS Integer = 200\n\
                         \n\
                         EXPORT FUNC seed() AS Integer\n\
                        \x20 RETURN Seed\n\
                         END FUNC\n",
                depends_on: &[],
            },
            Package {
                name: "upper",
                source: "IMPORT base\n\
                         \n\
                         EXPORT LET Derived AS Integer = base::seed() + 1\n\
                         EXPORT LET Direct AS Integer = base::Seed + 2\n",
                depends_on: &["base"],
            },
        ],
        &["upper", "base"],
        "IMPORT upper\n\
         IMPORT io\n\
         \n\
         LET top AS Integer = upper::Derived + upper::Direct\n\
         \n\
         FUNC main() AS Integer\n\
        \x20 io::print(toString(upper::Derived))\n\
        \x20 io::print(toString(upper::Direct))\n\
        \x20 io::print(toString(top))\n\
        \x20 RETURN 0\n\
         END FUNC\n",
    );
    assert_eq!(
        lines(&output),
        vec!["201", "202", "403"],
        "a package must initialize before the package that imports it:\n{output}"
    );
}
