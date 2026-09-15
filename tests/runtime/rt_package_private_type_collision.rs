//! bug-624: a package's PRIVATE type and a program type of the same name are two
//! different types.
//!
//! `merge_packages` (`src/target/shared/nir/lower.rs`) identity-prefixes a
//! package's functions and globals (`ir::prefix_package_symbols`) but left its
//! types unqualified, and `ir::merge_package` merges types with a first-wins
//! bare-name dedup. So when the importer declared a `TYPE` whose name matched a
//! non-`EXPORT` type inside the package, the program's definition won, and the
//! package's own code was checked against it: the build failed with
//! `TYPE_CONSTRUCTOR_ARGUMENT_MISMATCH` (flat types) or
//! `PACKAGE_BINARY_REPRESENTATION_VERIFY_TYPE` (a recursive one — the browser's
//! `dom` package and its private `Frame`). A private type is not part of the
//! package's surface, so its name must be invisible to the consumer.
//!
//! Each case is built from the package's source form and from its `.mfp` form,
//! since the `.mfp` is what the merge actually decodes.

#[path = "../common/mod.rs"]
mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// A package with a PRIVATE, flat `Frame` and an exported function that builds
/// a list of them.
const FLAT_PACKAGE: &str = "IMPORT collections\n\
\n\
' A PRIVATE record: never exported.\n\
TYPE Frame\n\
\x20 tag AS String\n\
\x20 kids AS List OF Integer\n\
END TYPE\n\
\n\
EXPORT FUNC depth(n AS Integer) AS Integer\n\
\x20 MUT stack AS List OF Frame = [Frame[\"root\", []]]\n\
\x20 MUT i AS Integer = 0\n\
\x20 WHILE i < n\n\
\x20   stack = collections::append(stack, Frame[\"x\", [i]])\n\
\x20   i = i + 1\n\
\x20 END WHILE\n\
\x20 RETURN len(stack)\n\
END FUNC\n";

/// A package with a PRIVATE, recursive `Frame` (the browser `dom` shape).
const RECURSIVE_PACKAGE: &str = "IMPORT collections\n\
\n\
TYPE Frame\n\
\x20 tag AS String\n\
\x20 kids AS List OF Node\n\
END TYPE\n\
\n\
TYPE Leaf\n\
\x20 text AS String\n\
END TYPE\n\
\n\
UNION Node\n\
\x20 Frame\n\
\x20 Leaf\n\
END UNION\n\
\n\
EXPORT FUNC depth(n AS Integer) AS Integer\n\
\x20 MUT kids AS List OF Node = []\n\
\x20 MUT i AS Integer = 0\n\
\x20 WHILE i < n\n\
\x20   LET leaf AS Node = Leaf[\"x\"]\n\
\x20   kids = collections::append(kids, leaf)\n\
\x20   i = i + 1\n\
\x20 END WHILE\n\
\x20 LET root AS Frame = Frame[\"root\", kids]\n\
\x20 RETURN len(root.kids) + 1\n\
END FUNC\n";

/// The program's own flat `Frame`: same name as the package's private one,
/// different fields.
const FLAT_PROGRAM: &str = "IMPORT io\n\
IMPORT pk\n\
\n\
TYPE Frame\n\
\x20 tag AS String\n\
\x20 kids AS List OF String\n\
END TYPE\n\
\n\
SUB main()\n\
\x20 LET f AS Frame = Frame[\"mine\", [\"a\"]]\n\
\x20 io::print(f.tag & \" \" & toString(pk::depth(3)))\n\
END SUB\n";

/// The program's own recursive `Frame`.
const RECURSIVE_PROGRAM: &str = "IMPORT io\n\
IMPORT pk\n\
\n\
TYPE Frame\n\
\x20 tag AS String\n\
\x20 kids AS List OF Node\n\
END TYPE\n\
\n\
TYPE Leaf\n\
\x20 text AS String\n\
END TYPE\n\
\n\
UNION Node\n\
\x20 Frame\n\
\x20 Leaf\n\
END UNION\n\
\n\
SUB main()\n\
\x20 LET f AS Frame = Frame[\"mine\", [Leaf[\"a\"]]]\n\
\x20 io::print(f.tag & \" \" & toString(pk::depth(3)))\n\
END SUB\n";

/// How the importer names its dependency.
#[derive(Clone, Copy)]
enum Form {
    /// `file:packages/pk` — the package directory, built as part of the app.
    Source,
    /// `file:packages/pk.mfp` — the package prebuilt to its binary form.
    Mfp,
}

fn combined(output: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// Write `package` as `pk` and `program` as its importer, in `form`, and return
/// `(root, app dir)`.
fn project(name: &str, package: &str, program: &str, form: Form) -> (PathBuf, PathBuf) {
    let root = std::env::temp_dir().join(format!("mfb_{name}_{}", common::unique_nonce()));
    let app = root.join("app");
    let pkg = match form {
        Form::Source => app.join("packages/pk"),
        Form::Mfp => root.join("pk"),
    };
    fs::create_dir_all(pkg.join("src")).expect("create package dir");
    fs::create_dir_all(app.join("src")).expect("create app dir");
    fs::write(
        pkg.join("project.json"),
        "{\"name\":\"pk\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"package\",\
         \"description\":\"a package with a private type\",\
         \"sources\":[{\"root\":\"src\",\"role\":\"package\",\"include\":[\"**/*.mfb\"]}]}\n",
    )
    .expect("write package manifest");
    fs::write(pkg.join("src/lib.mfb"), package).expect("write package source");

    let source = match form {
        Form::Source => "file:packages/pk",
        Form::Mfp => {
            let output = Command::new(common::mfb_exe())
                .arg("build")
                .arg(&pkg)
                .output()
                .expect("run mfb build on the package");
            let text = combined(&output);
            assert!(output.status.success(), "the package failed to build:\n{text}");
            let built = text
                .lines()
                .find_map(|line| line.strip_prefix("Wrote package to "))
                .expect("build output names the package");
            fs::create_dir_all(app.join("packages")).expect("create app packages dir");
            fs::copy(built.trim(), app.join("packages/pk.mfp")).expect("install pk.mfp");
            "file:packages/pk.mfp"
        }
    };
    fs::write(
        app.join("project.json"),
        format!(
            "{{\"name\":\"app\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"executable\",\
             \"sources\":[{{\"root\":\"src\",\"role\":\"main\",\"include\":[\"**/*.mfb\"]}}],\
             \"packages\":[{{\"name\":\"pk\",\"version\":\"=0.1.0\",\"source\":\"{source}\"}}],\
             \"entry\":\"main\",\"targets\":[\"native\"]}}\n"
        ),
    )
    .expect("write app manifest");
    fs::write(app.join("src/main.mfb"), program).expect("write program source");
    (root, app)
}

/// Build and run the importer; return its trimmed stdout.
fn run(name: &str, package: &str, program: &str, form: Form) -> String {
    let (root, app) = project(name, package, program, form);
    let output = Command::new(common::mfb_exe())
        .arg("build")
        .arg(&app)
        .output()
        .expect("run mfb build");
    let text = combined(&output);
    assert!(
        output.status.success(),
        "{name}: the importer must build — the package's private `Frame` is not the \
         program's `Frame`:\n{text}"
    );
    let exe = text
        .lines()
        .find_map(|line| line.strip_prefix("Wrote executable to "))
        .expect("build output names an executable");
    let ran = Command::new(Path::new(exe.trim()))
        .output()
        .expect("run the importer");
    let out = combined(&ran);
    assert!(
        ran.status.success(),
        "{name}: the importer exited {}:\n{out}",
        common::exit_description(&ran.status)
    );
    let _ = fs::remove_dir_all(&root);
    String::from_utf8_lossy(&ran.stdout).trim().to_string()
}

#[test]
fn a_private_flat_package_type_and_a_flat_program_type_are_distinct() {
    assert_eq!(
        run("private_flat_src", FLAT_PACKAGE, FLAT_PROGRAM, Form::Source),
        "mine 4"
    );
}

#[test]
fn a_private_flat_mfp_type_and_a_flat_program_type_are_distinct() {
    assert_eq!(
        run("private_flat_mfp", FLAT_PACKAGE, FLAT_PROGRAM, Form::Mfp),
        "mine 4"
    );
}

#[test]
fn a_private_recursive_package_type_and_a_flat_program_type_are_distinct() {
    assert_eq!(
        run("private_rec_src", RECURSIVE_PACKAGE, FLAT_PROGRAM, Form::Source),
        "mine 4"
    );
}

#[test]
fn a_private_recursive_mfp_type_and_a_flat_program_type_are_distinct() {
    assert_eq!(
        run("private_rec_mfp", RECURSIVE_PACKAGE, FLAT_PROGRAM, Form::Mfp),
        "mine 4"
    );
}

#[test]
fn a_private_flat_package_type_and_a_recursive_program_type_are_distinct() {
    assert_eq!(
        run("private_swap_src", FLAT_PACKAGE, RECURSIVE_PROGRAM, Form::Source),
        "mine 4"
    );
}

#[test]
fn a_private_flat_mfp_type_and_a_recursive_program_type_are_distinct() {
    assert_eq!(
        run("private_swap_mfp", FLAT_PACKAGE, RECURSIVE_PROGRAM, Form::Mfp),
        "mine 4"
    );
}
