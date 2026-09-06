//! An imported package's exported `LET`/`MUT` is usable by the importer.
//!
//! bug-551. `mfb spec language modules-and-packages` §13 lets a top-level `LET`
//! and `MUT` be `EXPORT`ed, calls an exported `MUT` "package state visible to
//! importers", and heads its list of prefixable name kinds with "variables and
//! constants". The SYMBOL half already worked —
//! `resolver::packages::install_package_type_names` unions the `.mfp` GLOBAL
//! table into the package's visible surface, so `pkg::NotEvenThere` was refused
//! by name — but nothing carried the declared TYPE, so `pkg::Answer` typed as
//! `Unknown` and died at its use site with `TYPE_UNKNOWN_VALUE`, or reached an
//! unrelated call as an `(Unknown)` argument. Every package with a public
//! constant had to publish it as a `FUNC` instead.
//!
//! The bug report flagged initialization order as the thing to measure before
//! fixing: if an imported package's global initializers did not run before
//! `main`, typing the read would produce a zeroed value, which is worse than the
//! compile error. They do — `a_package_global_is_initialized_before_main` is
//! that measurement, and it is the reason this is a type-installation fix and
//! not a lowering one. The merge already re-points a `package.Name` reference at
//! the single `<id>.package.Name` definition
//! (`ir::package::apply_package_identity`), for reads and writes alike.
//!
//! The WRITE tests are the containment: the read fix makes `pkg::Name` resolve,
//! and `pkg::Name = value` in statement position previously fell through to
//! `parse_expression`, where `=` binds as EQUALITY — the silently-discarded
//! comparison bug-468 closed for `a.b = c`, reached by the `::` spelling. Left
//! alone it would have turned a loud internal error into a write that compiles
//! and vanishes.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// A source package exporting a constant, a string constant, mutable state and
/// a private binding, plus an importer with `source`, built and run.
fn run_importer(name: &str, importer: &str) -> String {
    let (root, app) = importer_project(name, importer);
    let executable = build(&app);
    let output = Command::new(&executable)
        .output()
        .expect("run the importer");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.status.success(),
        "the importer exited {}:\n{combined}",
        common::exit_description(&output.status)
    );
    let _ = fs::remove_dir_all(&root);
    combined
}

/// The same scaffolding, for an importer the compiler must REJECT.
fn build_error(name: &str, importer: &str) -> String {
    let (root, app) = importer_project(name, importer);
    let output = Command::new(common::mfb_exe())
        .arg("build")
        .arg(&app)
        .output()
        .expect("run mfb build");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !output.status.success(),
        "the importer was accepted, but must be rejected:\n{combined}"
    );
    let _ = fs::remove_dir_all(&root);
    combined
}

fn importer_project(name: &str, importer: &str) -> (PathBuf, PathBuf) {
    let root = std::env::temp_dir().join(format!("mfb_{name}_{}", common::unique_nonce()));
    let pkg = root.join("pkg");
    let app = root.join("app");
    fs::create_dir_all(pkg.join("src")).expect("create package dir");
    fs::create_dir_all(app.join("src")).expect("create app dir");

    fs::write(
        pkg.join("project.json"),
        "{\"name\":\"limits\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"package\",\
         \"description\":\"constants to import\",\
         \"sources\":[{\"root\":\"src\",\"role\":\"package\",\"include\":[\"**/*.mfb\"]}]}\n",
    )
    .expect("write package manifest");
    // `counter()` is the package's own view of `Counter`: it reads the same slot
    // the importer writes, which is what makes a write test a SEMANTICS test
    // rather than a "the assignment compiled" test.
    fs::write(
        pkg.join("src/lib.mfb"),
        "EXPORT LET Answer AS Integer = 42\n\
         EXPORT LET Greeting AS String = \"hi\"\n\
         EXPORT MUT Counter AS Integer = 7\n\
         PRIVATE LET Secret AS Integer = 5\n\
         \n\
         EXPORT FUNC counter() AS Integer\n\
        \x20 RETURN Counter\n\
         END FUNC\n\
         \n\
         EXPORT FUNC secret() AS Integer\n\
        \x20 RETURN Secret\n\
         END FUNC\n",
    )
    .expect("write package source");

    fs::write(
        app.join("project.json"),
        "{\"name\":\"limitapp\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"executable\",\
         \"description\":\"an importer\",\
         \"sources\":[{\"root\":\"src\",\"role\":\"main\",\"include\":[\"**/*.mfb\"]}],\
         \"packages\":[{\"name\":\"limits\",\"version\":\"=0.1.0\",\"source\":\"file:../pkg\"}],\
         \"entry\":\"main\",\"targets\":[\"native\"]}\n",
    )
    .expect("write app manifest");
    fs::write(app.join("src/main.mfb"), importer).expect("write importer source");

    (root, app)
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

/// The bug's own reproduction: an `EXPORT LET` read through an explicitly typed
/// binding, plus the two positions the missing type surfaced in — an inferred
/// binding and a call argument.
#[test]
fn an_exported_package_constant_is_readable() {
    let output = run_importer(
        "package_constant_read",
        "IMPORT limits\n\
         IMPORT io\n\
         \n\
         FUNC main() AS Integer\n\
        \x20 LET n AS Integer = limits::Answer\n\
        \x20 io::print(toString(n))\n\
        \x20 io::print(limits::Greeting)\n\
        \x20 io::print(toString(limits::Answer + 1))\n\
        \x20 RETURN 0\n\
         END FUNC\n",
    );
    assert_eq!(
        output.lines().collect::<Vec<_>>(),
        vec!["42", "hi", "43"],
        "an exported constant must carry its declared type:\n{output}"
    );
}

/// The measurement the bug report asked for FIRST: an imported package's global
/// initializer runs in the consumer binary before `main`. If it did not, typing
/// the read would hand back a zero — a wrong answer where there used to be a
/// compile error. `7` is the declaration's value, read through the package's own
/// accessor and directly, so both views agree.
#[test]
fn a_package_global_is_initialized_before_main() {
    let output = run_importer(
        "package_global_initialized",
        "IMPORT limits\n\
         IMPORT io\n\
         \n\
         FUNC main() AS Integer\n\
        \x20 io::print(toString(limits::counter()))\n\
        \x20 io::print(toString(limits::Counter))\n\
        \x20 RETURN 0\n\
         END FUNC\n",
    );
    assert_eq!(
        output.lines().collect::<Vec<_>>(),
        vec!["7", "7"],
        "the initializer must have run, and both views must see one slot:\n{output}"
    );
}

/// `EXPORT MUT` is "package state visible to importers" (§13), and there is one
/// slot: the importer's write is what the PACKAGE's own function reads back.
/// That round trip is the semantics claim — `99` coming back out of
/// `limits::counter()` cannot be produced by a private copy.
#[test]
fn an_exported_package_mut_is_one_shared_slot() {
    let output = run_importer(
        "package_mut_shared",
        "IMPORT limits\n\
         IMPORT io\n\
         \n\
         FUNC main() AS Integer\n\
        \x20 io::print(toString(limits::counter()))\n\
        \x20 limits::Counter = 99\n\
        \x20 io::print(toString(limits::counter()))\n\
        \x20 io::print(toString(limits::Counter))\n\
        \x20 RETURN 0\n\
         END FUNC\n",
    );
    assert_eq!(
        output.lines().collect::<Vec<_>>(),
        vec!["7", "99", "99"],
        "the importer's write must be the package's own state:\n{output}"
    );
}

/// `IMPORT … AS` binds the same package under another name, and the globals are
/// keyed by the CANONICAL `package.Name`, so both a read and a write have to
/// resolve the alias. The write half had its own resolution site (the shape
/// pass's assignment-target rule) and failed there after the read already worked.
#[test]
fn an_aliased_import_reaches_the_same_globals() {
    let output = run_importer(
        "package_global_aliased",
        "IMPORT limits AS lim\n\
         IMPORT io\n\
         \n\
         FUNC main() AS Integer\n\
        \x20 io::print(toString(lim::Answer))\n\
        \x20 lim::Counter = 5\n\
        \x20 io::print(toString(lim::counter()))\n\
        \x20 RETURN 0\n\
         END FUNC\n",
    );
    assert_eq!(
        output.lines().collect::<Vec<_>>(),
        vec!["42", "5"],
        "an aliased import must reach the same slots:\n{output}"
    );
}

/// The read fix must not make an `EXPORT LET` writable. This is the rule that
/// already refuses a write to the project's OWN `LET`, reached across the
/// boundary because `global_muts` is seeded with the `.mfp`'s mutability bit.
#[test]
fn a_write_to_an_exported_package_constant_is_refused() {
    let output = build_error(
        "package_constant_write",
        "IMPORT limits\n\
         \n\
         FUNC main() AS Integer\n\
        \x20 limits::Answer = 1\n\
        \x20 RETURN 0\n\
         END FUNC\n",
    );
    assert!(
        output.contains("TYPE_ASSIGN_REQUIRES_MUT"),
        "an EXPORT LET must not be assignable:\n{output}"
    );
    assert!(
        output.contains("`limits.Answer` is immutable"),
        "the diagnostic must name the binding, which needs its mutability bit \
         off the `.mfp` — without it the write parsed as a discarded comparison \
         and compiled clean:\n{output}"
    );
}

/// The type installed is the DECLARED one, not `Unknown`: a mismatched write is
/// caught. A permissive `Unknown` would pass this silently.
#[test]
fn a_write_of_the_wrong_type_to_a_package_mut_is_refused() {
    let output = build_error(
        "package_mut_wrong_type",
        "IMPORT limits\n\
         \n\
         FUNC main() AS Integer\n\
        \x20 limits::Counter = \"not an integer\"\n\
        \x20 RETURN 0\n\
         END FUNC\n",
    );
    assert!(
        output.contains("TYPE_ASSIGNMENT_MISMATCH") && output.contains("expected Integer"),
        "the declared type must be enforced:\n{output}"
    );
}

/// The same for a read: `Integer` in, `String` expected.
#[test]
fn a_package_constant_read_is_type_checked() {
    let output = build_error(
        "package_constant_read_typed",
        "IMPORT limits\n\
         \n\
         FUNC main() AS Integer\n\
        \x20 LET s AS String = limits::Answer\n\
        \x20 RETURN 0\n\
         END FUNC\n",
    );
    assert!(
        output.contains("TYPE_BINDING_MISMATCH") && output.contains("expected String"),
        "the installed type must be the declared one:\n{output}"
    );
}

/// Only `EXPORT` crosses the boundary. A `PRIVATE` binding is in the same `.mfp`
/// GLOBAL table (the writer records visibility in the entry flags rather than
/// omitting the row), so seeding without the visibility filter would publish it.
#[test]
fn a_private_package_global_is_not_visible() {
    let output = build_error(
        "package_global_private",
        "IMPORT limits\n\
         \n\
         FUNC main() AS Integer\n\
        \x20 LET n AS Integer = limits::Secret\n\
        \x20 RETURN 0\n\
         END FUNC\n",
    );
    assert!(
        output.contains("Package `limits` does not export `Secret`"),
        "a PRIVATE binding must stay private:\n{output}"
    );
}

/// bug-480's attribution, on a global that does not exist at all — the symbol
/// half that already worked and must keep working.
#[test]
fn an_unexported_global_name_still_names_its_package() {
    let output = build_error(
        "package_global_unknown",
        "IMPORT limits\n\
         \n\
         FUNC main() AS Integer\n\
        \x20 LET n AS Integer = limits::NotEvenThere\n\
        \x20 RETURN 0\n\
         END FUNC\n",
    );
    assert!(
        output.contains("Package `limits` does not export `NotEvenThere`"),
        "an unknown member must still be reported against its package:\n{output}"
    );
}

/// `=` in EXPRESSION position is still the equality operator. The statement-position
/// parse of `pkg::Name = value` as an assignment must not reach into a condition
/// or an initializer.
#[test]
fn a_qualified_comparison_in_expression_position_is_still_a_comparison() {
    let output = run_importer(
        "package_global_comparison",
        "IMPORT limits\n\
         IMPORT io\n\
         \n\
         FUNC main() AS Integer\n\
        \x20 IF limits::Answer = 42 THEN\n\
        \x20   io::print(\"equal\")\n\
        \x20 END IF\n\
        \x20 LET b AS Boolean = limits::Answer = 43\n\
        \x20 io::print(toString(b))\n\
        \x20 RETURN 0\n\
         END FUNC\n",
    );
    assert_eq!(
        output.lines().collect::<Vec<_>>(),
        vec!["equal", "FALSE"],
        "`=` in expression position is equality:\n{output}"
    );
}
