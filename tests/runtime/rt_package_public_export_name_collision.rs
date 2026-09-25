//! bug-653: a package's PUBLIC function sharing a name with an EXPORT function
//! must not change what an importer sees.
//!
//! Overloads may be declared across a package's files and visibility is not part
//! of a callable's identity, so `EXPORT FUNC f(String)` plus
//! `PUBLIC FUNC f(Integer, Integer)` is a legal overload set. But importers see
//! only the EXPORTs. The monomorphizer mangled every member of the set
//! (`f$String`) because it counted the PUBLIC sibling, and the `.mfp` writer then
//! dropped that sibling from the export table — leaving one mangled row that the
//! importer's overload collector (which needs two or more rows) never mapped back
//! to `f`. Every `pk::f(...)` failed to type or failed to link, while the package
//! itself built and tested green.
//!
//! Each runtime case is built from the package's source form and its `.mfp`
//! form, since the `.mfp` is what the importer actually decodes.

#[path = "../common/mod.rs"]
mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The package's public API, plus an export proving both overloads resolve
/// inside the package.
const LIB: &str = "EXPORT FUNC f(x AS String) AS String\n\
\x20 RETURN \"export:\" & x\n\
END FUNC\n\
\n\
EXPORT FUNC both() AS String\n\
\x20 RETURN f(\"a\") & \"|\" & f(1, 2)\n\
END FUNC\n";

/// A package-internal overload of `f`, in another file.
const PUBLIC_HELPER: &str = "PUBLIC FUNC f(a AS Integer, b AS Integer) AS String\n\
\x20 RETURN \"public:\" & toString(a + b)\n\
END FUNC\n";

/// The same overload, exported — the case that always worked.
const EXPORT_HELPER: &str = "EXPORT FUNC f(a AS Integer, b AS Integer) AS String\n\
\x20 RETURN \"public:\" & toString(a + b)\n\
END FUNC\n";

const PROGRAM: &str = "IMPORT io\n\
IMPORT pk\n\
\n\
SUB main()\n\
\x20 LET s AS String = pk::f(\"x\")\n\
\x20 io::print(s)\n\
\x20 io::print(toString(len(pk::f(\"x\"))))\n\
\x20 io::print(pk::f(\"y\"))\n\
\x20 io::print(pk::both())\n\
END SUB\n";

const EXPECTED: &str = "export:x\n8\nexport:y\nexport:a|public:3";

/// Return-type overloads: a PUBLIC sibling with the same parameters and a
/// different result type.
const RETURN_LIB: &str = "EXPORT FUNC g(x AS String) AS String\n\
\x20 RETURN \"s:\" & x\n\
END FUNC\n\
\n\
EXPORT FUNC both() AS String\n\
\x20 LET a AS String = g(\"ab\")\n\
\x20 LET n AS Integer = g(\"abc\")\n\
\x20 RETURN a & \"|\" & toString(n)\n\
END FUNC\n";

const RETURN_HELPER: &str = "PUBLIC FUNC g(x AS String) AS Integer\n\
\x20 RETURN len(x)\n\
END FUNC\n";

const RETURN_PROGRAM: &str = "IMPORT io\n\
IMPORT pk\n\
\n\
SUB main()\n\
\x20 io::print(pk::g(\"x\"))\n\
\x20 io::print(pk::both())\n\
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

/// Write a package `pk` from `files` (`(file name, source)`) into `dir`.
fn write_package(dir: &Path, files: &[(&str, &str)]) {
    fs::create_dir_all(dir.join("src")).expect("create package dir");
    fs::write(
        dir.join("project.json"),
        "{\"name\":\"pk\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"package\",\
         \"description\":\"a package with an overloaded name\",\
         \"sources\":[{\"root\":\"src\",\"role\":\"package\",\"include\":[\"**/*.mfb\"]}]}\n",
    )
    .expect("write package manifest");
    for (name, source) in files {
        fs::write(dir.join("src").join(name), source).expect("write package source");
    }
}

/// Build the package in `dir`; return the path of the `.mfp` it wrote.
fn build_package(dir: &Path) -> PathBuf {
    let output = Command::new(common::mfb_exe())
        .arg("build")
        .arg(dir)
        .output()
        .expect("run mfb build on the package");
    let text = combined(&output);
    assert!(
        output.status.success(),
        "the package failed to build:\n{text}"
    );
    PathBuf::from(
        text.lines()
            .find_map(|line| line.strip_prefix("Wrote package to "))
            .expect("build output names the package")
            .trim(),
    )
}

/// Write `files` as `pk` and `program` as its importer, in `form`; build the
/// importer and return `(root, build output, executable path if built)`.
fn build_importer(
    name: &str,
    files: &[(&str, &str)],
    program: &str,
    form: Form,
) -> (PathBuf, String, Option<PathBuf>) {
    let root = std::env::temp_dir().join(format!("mfb_{name}_{}", common::unique_nonce()));
    let app = root.join("app");
    fs::create_dir_all(app.join("src")).expect("create app dir");
    let source = match form {
        Form::Source => {
            write_package(&app.join("packages/pk"), files);
            "file:packages/pk"
        }
        Form::Mfp => {
            let pkg = root.join("pk");
            write_package(&pkg, files);
            let built = build_package(&pkg);
            fs::create_dir_all(app.join("packages")).expect("create app packages dir");
            fs::copy(built, app.join("packages/pk.mfp")).expect("install pk.mfp");
            "file:packages/pk.mfp"
        }
    };
    fs::write(
        app.join("project.json"),
        format!(
            "{{\"name\":\"app\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"executable\",\
             \"sources\":[{{\"root\":\"src\",\"role\":\"main\",\"include\":[\"**/*.mfb\"]}}],\
             \"packages\":[{{\"name\":\"pk\",\"version\":\"=0.1.0\",\"source\":\"{source}\",\"direct\":true,\"requiredBy\":[]}}],\
             \"entry\":\"main\",\"targets\":[\"native\"]}}\n"
        ),
    )
    .expect("write app manifest");
    fs::write(app.join("src/main.mfb"), program).expect("write program source");
    let output = Command::new(common::mfb_exe())
        .arg("build")
        .arg(&app)
        .output()
        .expect("run mfb build");
    let text = combined(&output);
    let exe = output
        .status
        .success()
        .then(|| {
            text.lines()
                .find_map(|line| line.strip_prefix("Wrote executable to "))
                .map(|path| PathBuf::from(path.trim()))
        })
        .flatten();
    (root, text, exe)
}

/// Build and run the importer; return its trimmed stdout.
fn run(name: &str, files: &[(&str, &str)], program: &str, form: Form) -> String {
    let (root, text, exe) = build_importer(name, files, program, form);
    let exe = exe.unwrap_or_else(|| {
        panic!("{name}: the importer must build — the export is the importer's to call:\n{text}")
    });
    let ran = Command::new(&exe).output().expect("run the importer");
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
fn an_export_overloaded_by_a_public_sibling_is_callable_from_source() {
    assert_eq!(
        run(
            "pub_export_src",
            &[("lib.mfb", LIB), ("helper.mfb", PUBLIC_HELPER)],
            PROGRAM,
            Form::Source
        ),
        EXPECTED
    );
}

#[test]
fn an_export_overloaded_by_a_public_sibling_is_callable_from_mfp() {
    assert_eq!(
        run(
            "pub_export_mfp",
            &[("lib.mfb", LIB), ("helper.mfb", PUBLIC_HELPER)],
            PROGRAM,
            Form::Mfp
        ),
        EXPECTED
    );
}

#[test]
fn an_export_with_a_public_return_type_sibling_is_callable() {
    for (name, form) in [
        ("pub_return_src", Form::Source),
        ("pub_return_mfp", Form::Mfp),
    ] {
        assert_eq!(
            run(
                name,
                &[("lib.mfb", RETURN_LIB), ("helper.mfb", RETURN_HELPER)],
                RETURN_PROGRAM,
                form
            ),
            "s:x\ns:ab|3"
        );
    }
}

/// Guard: two EXPORT overloads of one name keep resolving by argument types.
#[test]
fn two_export_overloads_still_resolve_by_argument_types() {
    const BOTH: &str = "IMPORT io\n\
IMPORT pk\n\
\n\
SUB main()\n\
\x20 io::print(pk::f(\"x\"))\n\
\x20 io::print(pk::f(1, 2))\n\
END SUB\n";
    for (name, form) in [
        ("two_export_src", Form::Source),
        ("two_export_mfp", Form::Mfp),
    ] {
        assert_eq!(
            run(
                name,
                &[("lib.mfb", LIB), ("helper.mfb", EXPORT_HELPER)],
                BOTH,
                form
            ),
            "export:x\npublic:3"
        );
    }
}

/// The PUBLIC overload is hidden from importers: calling it is a located
/// compile error, never the unlocated `NIR call target … does not resolve`.
#[test]
fn calling_the_public_overload_from_an_importer_is_a_located_error() {
    const CALLS_PUBLIC: &str = "IMPORT io\n\
IMPORT pk\n\
\n\
SUB main()\n\
\x20 io::print(pk::f(1, 2))\n\
END SUB\n";
    for (name, form) in [
        ("call_public_src", Form::Source),
        ("call_public_mfp", Form::Mfp),
    ] {
        let (root, text, exe) = build_importer(
            name,
            &[("lib.mfb", LIB), ("helper.mfb", PUBLIC_HELPER)],
            CALLS_PUBLIC,
            form,
        );
        assert!(
            exe.is_none(),
            "{name}: an importer must not reach a PUBLIC overload:\n{text}"
        );
        assert!(
            !text.contains("NIR call target"),
            "{name}: the rejection must be a located diagnostic:\n{text}"
        );
        assert!(
            text.contains("main.mfb:5 error["),
            "{name}: the rejection must name main.mfb line 5 with an error code:\n{text}"
        );
        assert!(
            text.contains("Call to `pk.f` has 2 argument(s), expected 1 to 1."),
            "{name}: only the EXPORT `f(x AS String)` is visible to the importer:\n{text}"
        );
        let _ = fs::remove_dir_all(&root);
    }
}

/// The `Exports:` section `mfb pkg info` prints for the package built from `files`.
fn exports_of(name: &str, files: &[(&str, &str)]) -> String {
    let dir = std::env::temp_dir().join(format!("mfb_{name}_{}", common::unique_nonce()));
    write_package(&dir, files);
    let mfp = build_package(&dir);
    let output = Command::new(common::mfb_exe())
        .args(["pkg", "info"])
        .arg(&mfp)
        .output()
        .expect("run mfb pkg info");
    let text = combined(&output);
    assert!(output.status.success(), "mfb pkg info failed:\n{text}");
    let _ = fs::remove_dir_all(&dir);
    text.split("\nExports:\n")
        .nth(1)
        .and_then(|rest| rest.split("\n\n").next())
        .unwrap_or_else(|| panic!("mfb pkg info prints an Exports section:\n{text}"))
        .to_string()
}

/// A PUBLIC declaration is hidden from importers, so adding one must not rename
/// an exported symbol — the export table and its ABI hashes are what
/// `mfb repo check-abi` compares between releases.
#[test]
fn a_public_sibling_does_not_change_the_package_exports() {
    const ONLY_EXPORT: &str = "EXPORT FUNC f(x AS String) AS String\n\
\x20 RETURN \"export:\" & x\n\
END FUNC\n";
    let plain = exports_of("exports_plain", &[("lib.mfb", ONLY_EXPORT)]);
    assert_eq!(
        exports_of(
            "exports_with_public",
            &[("lib.mfb", ONLY_EXPORT), ("helper.mfb", PUBLIC_HELPER)]
        ),
        plain,
        "a PUBLIC overload changed the export table"
    );
    const ONLY_RETURN_EXPORT: &str = "EXPORT FUNC g(x AS String) AS String\n\
\x20 RETURN \"s:\" & x\n\
END FUNC\n";
    let plain = exports_of("exports_return_plain", &[("lib.mfb", ONLY_RETURN_EXPORT)]);
    assert_eq!(
        exports_of(
            "exports_return_with_public",
            &[
                ("lib.mfb", ONLY_RETURN_EXPORT),
                ("helper.mfb", RETURN_HELPER)
            ]
        ),
        plain,
        "a PUBLIC return-type overload changed the export table"
    );
}
