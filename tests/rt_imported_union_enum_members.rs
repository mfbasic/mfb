//! An imported package's UNION and ENUM keep their MEMBERS across the boundary.
//!
//! bug-554. `mfb spec language modules-and-packages` §13 lets a top-level
//! `UNION` and `ENUM` be `EXPORT`ed and says the qualified-name rule covers
//! "unions, union variants, enums, enum members". The TYPE names crossed —
//! `resolver::packages::install_package_type_names` inserts the union, each
//! variant and the enum — but the MEMBERSHIP did not, and it is membership the
//! exhaustiveness checker reads.
//!
//! Two independent defects, one per direction of the boundary:
//!
//! 1. `ir::verify`'s `TypeEnv` is built from `project.types`, which on the
//!    source path holds only the importer's own declarations. An imported union
//!    and enum were therefore in neither `unions` nor `enums`, and
//!    `check_match_exhaustive` classifies a type in neither as an OPEN type: a
//!    `MATCH` covering every variant the package declares was refused with
//!    "MATCH on open type `Item` requires an unguarded CASE ELSE". `CASE ELSE`
//!    was the only spelling that compiled, and it cannot distinguish the
//!    variants the match was written to distinguish. Both the bare and the
//!    qualified spelling failed, so this half is not about qualification.
//!
//! 2. A package-qualified enum-member READ (`pkg::Colour.Red`) is a VALUE, so
//!    the parser's type-position normalizer never sees it, and
//!    `expression_type`'s enum-member arm looked the target up under the name as
//!    written. The bare `Colour.Red` typed as `Colour` while the prefixed form
//!    §13 asks for typed as `Unknown` and died with `TYPE_UNKNOWN_VALUE`. (The
//!    bug report guessed this was the same missing lookup as bug-551's package
//!    constants; it is not — the bare spelling already worked.)
//!
//! The negative tests are the point: the fix must teach the checker the real
//! membership, not make it permissive. A `MATCH` missing an arm, and a name that
//! is not a member of the enum, must still be refused — and the diagnostic must
//! name the variant/member that is missing, which only a checker that actually
//! has the member set can do.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// A source package exporting a union, its two variant records and an enum,
/// plus an importer with `source`, built and run. Returns the program's output.
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
        "{\"name\":\"shapes\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"package\",\
         \"description\":\"a union and an enum to import\",\
         \"sources\":[{\"root\":\"src\",\"role\":\"package\",\"include\":[\"**/*.mfb\"]}]}\n",
    )
    .expect("write package manifest");
    fs::write(
        pkg.join("src/lib.mfb"),
        "EXPORT TYPE Note\n\
        \x20 label AS String\n\
         END TYPE\n\
         \n\
         EXPORT TYPE Tally\n\
        \x20 count AS Integer\n\
         END TYPE\n\
         \n\
         EXPORT UNION Item\n\
        \x20 Note\n\
        \x20 Tally\n\
         END UNION\n\
         \n\
         EXPORT ENUM Colour\n\
        \x20 Red, Green\n\
         END ENUM\n\
         \n\
         EXPORT FUNC anItem() AS Item\n\
        \x20 RETURN Note[label := \"in a union\"]\n\
         END FUNC\n\
         \n\
         EXPORT FUNC aTally() AS Item\n\
        \x20 RETURN Tally[count := 7]\n\
         END FUNC\n\
         \n\
         EXPORT FUNC aColour() AS Colour\n\
        \x20 RETURN Colour.Green\n\
         END FUNC\n",
    )
    .expect("write package source");

    fs::write(
        app.join("project.json"),
        "{\"name\":\"shapeapp\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"executable\",\
         \"description\":\"an importer\",\
         \"sources\":[{\"root\":\"src\",\"role\":\"main\",\"include\":[\"**/*.mfb\"]}],\
         \"packages\":[{\"name\":\"shapes\",\"version\":\"=0.1.0\",\"source\":\"file:../pkg\"}],\
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

/// Defect 1, qualified spelling: an exhaustive `MATCH` on an imported UNION
/// compiles, and each arm selects on the real variant — the second call proves
/// the arms are discriminating and not just that one of them happened to run.
#[test]
fn a_qualified_imported_union_matches_exhaustively() {
    let output = run_importer(
        "qualified_union_match",
        "IMPORT shapes\n\
         IMPORT io\n\
         \n\
         PRIVATE FUNC show(item AS shapes::Item) AS String\n\
        \x20 MATCH item\n\
        \x20   CASE shapes::Note(n)  : RETURN n.label\n\
        \x20   CASE shapes::Tally(t) : RETURN toString(t.count)\n\
        \x20 END MATCH\n\
         END FUNC\n\
         \n\
         FUNC main() AS Integer\n\
        \x20 io::print(show(shapes::anItem()))\n\
        \x20 io::print(show(shapes::aTally()))\n\
        \x20 RETURN 0\n\
         END FUNC\n",
    );
    assert_eq!(
        output.lines().collect::<Vec<_>>(),
        vec!["in a union", "7"],
        "each arm must select its own variant:\n{output}"
    );
}

/// Defect 1, bare spelling. Both spellings failed identically before the fix, so
/// the bare form is not a duplicate of the test above — it proves the membership
/// seed, not the name normalization, is what carries this.
#[test]
fn a_bare_imported_union_matches_exhaustively() {
    let output = run_importer(
        "bare_union_match",
        "IMPORT shapes\n\
         IMPORT io\n\
         \n\
         FUNC main() AS Integer\n\
        \x20 LET item AS Item = shapes::aTally()\n\
        \x20 MATCH item\n\
        \x20   CASE Note(n)  : io::print(n.label)\n\
        \x20   CASE Tally(t) : io::print(toString(t.count))\n\
        \x20 END MATCH\n\
        \x20 RETURN 0\n\
         END FUNC\n",
    );
    assert_eq!(
        output.trim(),
        "7",
        "the bare spelling reaches the same membership:\n{output}"
    );
}

/// Defect 1 for the ENUM half, and defect 2 in the same program: the scrutinee
/// is produced by a package function and the arms are written in the qualified
/// `pkg::Enum.Member` form that only defect 2's fix types.
#[test]
fn a_qualified_imported_enum_matches_exhaustively() {
    let output = run_importer(
        "qualified_enum_match",
        "IMPORT shapes\n\
         IMPORT io\n\
         \n\
         FUNC main() AS Integer\n\
        \x20 LET c AS shapes::Colour = shapes::aColour()\n\
        \x20 MATCH c\n\
        \x20   CASE shapes::Colour.Red   : io::print(\"red\")\n\
        \x20   CASE shapes::Colour.Green : io::print(\"green\")\n\
        \x20 END MATCH\n\
        \x20 RETURN 0\n\
         END FUNC\n",
    );
    assert_eq!(
        output.trim(),
        "green",
        "the enum's members must cross the boundary:\n{output}"
    );
}

/// Defect 2 on its own: a qualified enum member READ, with the value's identity
/// checked against a BARE-spelled match so the two spellings are proved to name
/// the same member and not two lookups that merely both succeed.
#[test]
fn a_qualified_imported_enum_member_is_a_value() {
    let output = run_importer(
        "qualified_enum_member_value",
        "IMPORT shapes\n\
         IMPORT io\n\
         \n\
         FUNC main() AS Integer\n\
        \x20 LET c AS shapes::Colour = shapes::Colour.Red\n\
        \x20 MATCH c\n\
        \x20   CASE Colour.Red   : io::print(\"red\")\n\
        \x20   CASE Colour.Green : io::print(\"green\")\n\
        \x20 END MATCH\n\
        \x20 RETURN 0\n\
         END FUNC\n",
    );
    assert_eq!(
        output.trim(),
        "red",
        "`pkg::Colour.Red` must be the same member `Colour.Red` is:\n{output}"
    );
}

/// The bare spelling must keep working — defect 2's fix normalizes the qualified
/// form onto it, so a regression here would mean the normalization ran backwards.
#[test]
fn a_bare_imported_enum_member_is_still_a_value() {
    let output = run_importer(
        "bare_enum_member_value",
        "IMPORT shapes\n\
         IMPORT io\n\
         \n\
         FUNC main() AS Integer\n\
        \x20 LET c AS Colour = Colour.Green\n\
        \x20 MATCH c\n\
        \x20   CASE Colour.Red   : io::print(\"red\")\n\
        \x20   CASE Colour.Green : io::print(\"green\")\n\
        \x20 END MATCH\n\
        \x20 RETURN 0\n\
         END FUNC\n",
    );
    assert_eq!(
        output.trim(),
        "green",
        "the bare spelling still resolves:\n{output}"
    );
}

/// The guard on defect 1: seeding the membership must make the checker CORRECT,
/// not permissive. A `MATCH` missing a variant is still refused — and it names
/// `Tally`, which only a checker holding the real variant set can do.
#[test]
fn an_imported_union_match_missing_a_variant_is_still_refused() {
    let output = build_error(
        "union_match_missing_variant",
        "IMPORT shapes\n\
         IMPORT io\n\
         \n\
         FUNC main() AS Integer\n\
        \x20 LET item AS shapes::Item = shapes::anItem()\n\
        \x20 MATCH item\n\
        \x20   CASE shapes::Note(n) : io::print(n.label)\n\
        \x20 END MATCH\n\
        \x20 RETURN 0\n\
         END FUNC\n",
    );
    assert!(
        output.contains("TYPE_MATCH_NOT_EXHAUSTIVE"),
        "a missing variant must still be refused:\n{output}"
    );
    assert!(
        output.contains("MATCH on UNION `Item` does not cover Tally"),
        "the diagnostic must name the missing variant, which needs the real \
         variant set — the pre-fix message called `Item` an open type:\n{output}"
    );
}

/// The same guard for the ENUM half.
#[test]
fn an_imported_enum_match_missing_a_member_is_still_refused() {
    let output = build_error(
        "enum_match_missing_member",
        "IMPORT shapes\n\
         IMPORT io\n\
         \n\
         FUNC main() AS Integer\n\
        \x20 LET c AS shapes::Colour = shapes::aColour()\n\
        \x20 MATCH c\n\
        \x20   CASE shapes::Colour.Red : io::print(\"red\")\n\
        \x20 END MATCH\n\
        \x20 RETURN 0\n\
         END FUNC\n",
    );
    assert!(
        output.contains("MATCH on enum `Colour` does not cover Colour.Green"),
        "the diagnostic must name the missing member:\n{output}"
    );
}

/// The guard on defect 2: the qualified enum lookup fails CLOSED. `Purple` is
/// not one of `Colour`'s members, so the read must stay unresolved rather than
/// be admitted by a normalizer that only checked the type name.
#[test]
fn a_qualified_enum_name_that_is_not_a_member_is_still_refused() {
    let output = build_error(
        "enum_member_not_a_member",
        "IMPORT shapes\n\
         IMPORT io\n\
         \n\
         FUNC main() AS Integer\n\
        \x20 LET c AS shapes::Colour = shapes::Colour.Purple\n\
        \x20 io::print(\"unreachable\")\n\
        \x20 RETURN 0\n\
         END FUNC\n",
    );
    assert!(
        output.contains("TYPE_UNKNOWN_VALUE"),
        "a name that is not a member must not resolve:\n{output}"
    );
}

/// The guard on the union-variant seed: registering each variant as a record
/// must not make every type a variant. A `CASE` naming a type the union does not
/// include is still a pattern mismatch.
#[test]
fn a_case_on_a_type_the_union_does_not_include_is_still_refused() {
    let output = build_error(
        "union_case_not_a_variant",
        "IMPORT shapes\n\
         IMPORT io\n\
         \n\
         FUNC main() AS Integer\n\
        \x20 LET item AS shapes::Item = shapes::anItem()\n\
        \x20 MATCH item\n\
        \x20   CASE shapes::Note(n)   : io::print(n.label)\n\
        \x20   CASE shapes::Tally(t)  : io::print(toString(t.count))\n\
        \x20   CASE shapes::Colour(c) : io::print(\"no\")\n\
        \x20 END MATCH\n\
        \x20 RETURN 0\n\
         END FUNC\n",
    );
    assert!(
        output.contains("CASE `Colour` is not a member of UNION `Item`"),
        "a non-variant CASE must still be refused:\n{output}"
    );
}
