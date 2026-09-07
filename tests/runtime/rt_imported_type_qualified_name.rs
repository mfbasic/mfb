//! A package-qualified imported TYPE name must name the same type the bare
//! spelling does.
//!
//! `mfb spec language modules-and-packages` §13 says a name reached through an
//! `IMPORT` takes a prefix, "for every kind of name alike: variables and
//! constants, functions, **records**, unions, union variants, enums, enum
//! members, and resource types". For an imported user package's record that was
//! not true: the parser rewrote `pkg::Note` to `pkg.Note` and left it there,
//! while `resolver::packages::install_package_type_names` installs the type
//! under its BARE name on purpose and `merge_packages` carries the package's own
//! bare spelling into the merged IR. The qualified spelling therefore named a
//! type nothing downstream knew.
//!
//! It failed differently depending on where it was written, which is what kept
//! it hidden: a field read off a `pkg::Note`-annotated parameter typed as
//! `Unknown` and died at the use site with `TYPE_UNKNOWN_VALUE`, while a
//! `LET n AS pkg::Note = …` type-checked and died in native lowering with
//! `native code field access target 'pkg.Note' is not a record or variant`.
//! Only a fully INFERRED binding worked, because inference copies the callee's
//! own bare spelling — so an exported record was readable exactly when nobody
//! wrote its name.
//!
//! Built-in packages are the other way round: their value types ARE
//! package-qualified (bug-480 Phase 4b), so the last case here guards against a
//! fix that de-qualifies those too.

#[path = "../common/mod.rs"]
mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// A source package exporting one record, plus an importer with `source`, built
/// and run. Returns the program's combined output.
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

/// The same scaffolding, for an importer the compiler must REJECT: returns
/// `mfb build`'s combined output after asserting it failed.
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

/// Write the package and the importer, and return `(project root, app dir)`.
fn importer_project(name: &str, importer: &str) -> (PathBuf, PathBuf) {
    let root = std::env::temp_dir().join(format!("mfb_{name}_{}", common::unique_nonce()));
    let pkg = root.join("pkg");
    let app = root.join("app");
    fs::create_dir_all(pkg.join("src")).expect("create package dir");
    fs::create_dir_all(app.join("src")).expect("create app dir");

    fs::write(
        pkg.join("project.json"),
        "{\"name\":\"notes\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"package\",\
         \"description\":\"a record to import\",\
         \"sources\":[{\"root\":\"src\",\"role\":\"package\",\"include\":[\"**/*.mfb\"]}]}\n",
    )
    .expect("write package manifest");
    fs::write(
        pkg.join("src/lib.mfb"),
        "IMPORT collections\n\
         \n\
         EXPORT TYPE Note\n\
        \x20 label AS String\n\
         END TYPE\n\
         \n\
         EXPORT TYPE Bag\n\
        \x20 items AS List OF Integer\n\
         END TYPE\n\
         \n\
         EXPORT FUNC notes() AS List OF Note\n\
        \x20 MUT out AS List OF Note = []\n\
        \x20 out = collections::append(out, Note[label := \"hello\"])\n\
        \x20 RETURN out\n\
         END FUNC\n",
    )
    .expect("write package source");

    fs::write(
        app.join("project.json"),
        "{\"name\":\"noteapp\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"executable\",\
         \"description\":\"an importer\",\
         \"sources\":[{\"root\":\"src\",\"role\":\"main\",\"include\":[\"**/*.mfb\"]}],\
         \"packages\":[{\"name\":\"notes\",\"version\":\"=0.1.0\",\"source\":\"file:../pkg\"}],\
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

/// The three positions that name the type: a binding annotation, a parameter,
/// and a constructor. All three read a field off the value afterwards, which is
/// the operation the wrong spelling broke.
#[test]
fn a_qualified_imported_record_reads_its_fields() {
    let output = run_importer(
        "qualified_record",
        "IMPORT notes\n\
         IMPORT io\n\
         IMPORT collections\n\
         \n\
         PRIVATE FUNC show(note AS notes::Note) AS String\n\
        \x20 RETURN note.label\n\
         END FUNC\n\
         \n\
         FUNC main() AS Integer\n\
        \x20 LET all AS List OF notes::Note = notes::notes()\n\
        \x20 LET one AS notes::Note = collections::get(all, 0)\n\
        \x20 io::print(one.label)\n\
        \x20 io::print(show(one))\n\
        \x20 io::print(notes::Note[label := \"built\"].label)\n\
        \x20 RETURN 0\n\
         END FUNC\n",
    );
    assert_eq!(
        output.lines().collect::<Vec<_>>(),
        vec!["hello", "hello", "built"],
        "each spelling must reach the same record:\n{output}"
    );
}

/// The bare spelling is the established convention
/// (`resolver::packages::install_package_type_names`) and must keep working:
/// the fix normalizes the qualified form ONTO it, so a regression here would
/// mean the normalization went the other way.
#[test]
fn a_bare_imported_record_still_reads_its_fields() {
    let output = run_importer(
        "bare_record",
        "IMPORT notes\n\
         IMPORT io\n\
         IMPORT collections\n\
         \n\
         PRIVATE FUNC show(note AS Note) AS String\n\
        \x20 RETURN note.label\n\
         END FUNC\n\
         \n\
         FUNC main() AS Integer\n\
        \x20 LET one AS Note = collections::get(notes::notes(), 0)\n\
        \x20 io::print(show(one))\n\
        \x20 RETURN 0\n\
         END FUNC\n",
    );
    assert_eq!(
        output.trim(),
        "hello",
        "the bare spelling still resolves:\n{output}"
    );
}

/// A BUILT-IN package's value type is package-qualified by the registry, so the
/// qualified spelling is the canonical one there and must be left alone. This
/// is the guard against de-qualifying every dotted type name.
#[test]
fn a_qualified_builtin_value_type_is_unchanged() {
    let project = common::temp_project(
        "qualified_builtin_type",
        "IMPORT regex\n\
         IMPORT io\n\
         \n\
         PRIVATE FUNC text(found AS regex::MatchInfo) AS String\n\
        \x20 RETURN found.text\n\
         END FUNC\n\
         \n\
         FUNC main() AS Integer\n\
        \x20 LET found AS regex::MatchInfo = regex::findMatch(\"abc\", \"b\")\n\
        \x20 io::print(found.text)\n\
        \x20 io::print(text(found))\n\
        \x20 RETURN 0\n\
         END FUNC\n",
    );
    let executable = common::build_project(&project);
    let output = Command::new(&executable).output().expect("run");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let _ = fs::remove_dir_all(&project);
    assert_eq!(
        stdout.lines().collect::<Vec<_>>(),
        vec!["b", "b"],
        "a built-in package's value type keeps its qualified identity:\n{stdout}"
    );
}

/// A qualified name the package does NOT export must be reported against the
/// package, at the line that writes it.
///
/// This is bug-480's contract, and de-qualifying a type-position `pkg::Leaf`
/// unconditionally broke it: `notes::NoSuchType` reached name resolution as the
/// bare `NoSuchType` and came back as `SYMBOL_UNKNOWN_TYPE` — "not a built-in or
/// top-level project type" — which names neither the package nor the fact that
/// the name was reached through an import. The rewrite is now gated on the
/// package actually exporting the leaf, so an unexported one stays qualified and
/// keeps its attribution.
#[test]
fn an_unexported_qualified_type_names_its_package() {
    let output = build_error(
        "unexported_qualified_type",
        "IMPORT notes\n\
         \n\
         FUNC main() AS Integer\n\
        \x20 LET bad AS notes::NoSuchType = 0\n\
        \x20 RETURN 0\n\
         END FUNC\n",
    );
    assert!(
        output.contains("SYMBOL_UNKNOWN_IDENTIFIER"),
        "an unexported member is an unknown IDENTIFIER, not an unknown project type:\n{output}"
    );
    assert!(
        output.contains("Package `notes` does not export `NoSuchType`."),
        "the diagnostic must name the package and the member:\n{output}"
    );
}

/// An imported record is comparable exactly when a local record of the same
/// shape is: `Bag` holds a `List OF Integer`, so it is not a legal `Map` key and
/// not a legal `collections::find` needle.
///
/// The rule reads the type's FIELDS, and the source-path IR carries only the
/// importer's own type table — so `is_comparable` found no row for `Bag`, fell
/// through to its permissive "unknown user type" tail, and accepted both. The
/// byte-identical LOCAL record was refused all along
/// (`tests/syntax/types/types-map-key-comparable-invalid`), which is what makes
/// this a boundary defect rather than a rule disagreement.
///
/// The list is BOUND rather than written inline at the call. A list literal
/// lowers as `List OF Unknown` whatever its elements are, and
/// `check_builtin_comparability` skips an `Unknown` element on purpose (never a
/// false rejection) — so `collections::find([one], one)` is accepted for a local
/// record too. That is a separate, import-independent gap in list-literal
/// element inference; writing it that way here would test nothing.
#[test]
fn an_imported_record_is_no_more_comparable_than_a_local_one() {
    let output = build_error(
        "imported_record_comparable",
        "IMPORT notes\n\
         IMPORT collections\n\
         \n\
         FUNC main() AS Integer\n\
        \x20 LET one AS notes::Bag = notes::Bag[[1, 2]]\n\
        \x20 LET keyed = Map OF notes::Bag TO Integer { one := 1 }\n\
        \x20 LET bags AS List OF notes::Bag = [one]\n\
        \x20 LET found AS Integer = collections::find(bags, one)\n\
        \x20 RETURN len(keyed) + found\n\
         END FUNC\n",
    );
    assert!(
        output.contains("TYPE_REQUIRES_COMPARABLE"),
        "an imported record holding a List is not comparable:\n{output}"
    );
    assert!(
        output.contains("Map key type requires a comparable type, got `Bag`."),
        "the Map key must be refused:\n{output}"
    );
    assert!(
        output.contains("Call to `collections.find` requires a comparable type, got `Bag`."),
        "the `collections::find` needle must be refused:\n{output}"
    );
}
