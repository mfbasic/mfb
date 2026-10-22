use super::*;

mod appimage;
mod options;
mod outputs;
mod package_gate;
mod refusals;
mod signing;
mod source_deps;

fn s(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
}

fn build_err(args: &[&str]) -> String {
    match parse_build_options(s(args)) {
        Ok(_) => panic!("expected an error for {args:?}"),
        Err(message) => message,
    }
}

fn write_executable_project(dir: &Path) {
    std::fs::write(
        dir.join("project.json"),
        concat!(
            "{\n",
            "  \"name\": \"app\",\n",
            "  \"version\": \"0.1.0\",\n",
            "  \"mfb\": \"1.0\",\n",
            "  \"kind\": \"executable\",\n",
            "  \"entry\": \"main\",\n",
            "  \"targets\": [\"native\"],\n",
            "  \"sources\": [{ \"root\": \"src\", \"role\": \"main\", \"include\": [\"**/*.mfb\"] }]\n",
            "}\n"
        ),
    )
    .expect("manifest");
    std::fs::create_dir_all(dir.join("src")).expect("src dir");
    std::fs::write(
        dir.join("src").join("main.mfb"),
        "IMPORT io\n\nFUNC main() AS Nothing\n  io::print(\"hi\")\nEND FUNC\n",
    )
    .expect("source");
}

fn copy_test_project(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("copy destination");
    for entry in std::fs::read_dir(from).expect("read fixture directory") {
        let entry = entry.expect("fixture entry");
        let source = entry.path();
        let destination = to.join(entry.file_name());
        if source.is_dir() {
            if entry.file_name() == "golden" || entry.file_name() == "build" {
                continue;
            }
            copy_test_project(&source, &destination);
        } else {
            std::fs::copy(&source, &destination).expect("copy fixture file");
        }
    }
}

/// Keep every compile-only byte-identity corpus in-process so cargo-llvm-cov
/// credits the clean-room `gen_*` files. The artifact gate runs these same
/// projects through a child compiler, whose profile cargo-llvm-cov does not
/// associate with the test target's object list. Targets are derived from the
/// committed ncodesum names, matching artifact-gate.sh rather than widening a
/// fixture onto a backend it intentionally does not support.
#[test]
fn builtin_codegen_corpora_lower_in_process() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/byte-identity");
    for package in std::fs::read_dir(&root).expect("byte-identity root") {
        let fixture = package.expect("package directory").path();
        if !fixture.join("project.json").is_file() {
            continue;
        }
        let dir = tempfile::tempdir().expect("temp dir");
        copy_test_project(&fixture, dir.path());
        let project = dir.path().to_str().expect("utf8 temp path");
        let golden = fixture.join("golden");
        for target in crate::target::registered_targets() {
            let target_name = target.name();
            let has_golden = std::fs::read_dir(&golden)
                .expect("fixture goldens")
                .filter_map(Result::ok)
                .any(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .ends_with(&format!(".{target_name}.ncodesum"))
                });
            if !has_golden {
                continue;
            }
            let options = parse_build_options(s(&["-ncode", "-target", &target_name, project]))
                .expect("options");
            build_project(&options)
                .unwrap_or_else(|()| panic!("{} lowers for {target_name}", fixture.display()));
        }
    }
}

#[test]
fn build_project_validates_a_bad_manifest() {
    let dir = tempfile::tempdir().expect("temp dir");
    // No project.json at all -> validate fails, Err(()).
    let options =
        parse_build_options(vec![dir.path().to_str().unwrap().to_string()]).expect("options");
    assert!(build_project(&options).is_err());
}

#[test]
fn build_project_rejects_app_mode_for_non_app_target() {
    let dir = tempfile::tempdir().expect("temp dir");
    write_executable_project(dir.path());
    // -app against a non-app target (a bare custom os) is rejected.
    let options = parse_build_options(s(&[
        "-app",
        "-target",
        "freebsd-riscv",
        dir.path().to_str().unwrap(),
    ]))
    .expect("options");
    assert!(build_project(&options).is_err());
}

#[test]
fn build_project_builds_a_host_executable() {
    let dir = tempfile::tempdir().expect("temp dir");
    write_executable_project(dir.path());
    let options =
        parse_build_options(vec![dir.path().to_str().unwrap().to_string()]).expect("options");
    // Full front-end + native writer for the host target; no network.
    build_project(&options).expect("build should succeed");
}

/// bug-503 (audit-3 LNK-12): a `project.json` `name` of `../evil` used to be
/// `Path::join`ed into `build/../evil.out` and written 0755 — an arbitrary
/// executable write from merely *building* an untrusted project. The build
/// must refuse the name and write nothing outside the project.
#[test]
fn build_project_rejects_a_traversing_project_name() {
    let root = tempfile::tempdir().expect("temp dir");
    let dir = root.path().join("proj");
    std::fs::create_dir_all(&dir).expect("project dir");
    write_executable_project(&dir);
    let manifest = std::fs::read_to_string(dir.join("project.json")).expect("manifest");
    std::fs::write(
        dir.join("project.json"),
        manifest.replace("\"name\": \"app\"", "\"name\": \"../evil\""),
    )
    .expect("rewrite manifest");
    let options = parse_build_options(vec![dir.to_str().unwrap().to_string()]).expect("options");
    assert!(
        build_project(&options).is_err(),
        "a traversing project name must fail the build"
    );
    // `proj/build/../evil.out` is `<root>/evil.out`; `-ast` would land
    // `<root>/evil.ast`. Neither — nor anything else — may appear beside the
    // project.
    let escaped: Vec<_> = std::fs::read_dir(root.path())
        .expect("read root")
        .map(|entry| entry.expect("entry").file_name())
        .filter(|name| name != "proj")
        .collect();
    assert!(escaped.is_empty(), "files escaped the project: {escaped:?}");
    assert!(
        !dir.join(crate::os::BUILD_DIR).exists(),
        "a refused build must not create build/"
    );
}

#[test]
fn build_project_clears_stale_build_dir() {
    // plan-55-A §4.2: a real build removes `build/` at the start, so a file a
    // previous build left there is gone afterward while the freshly written
    // executable exists.
    let dir = tempfile::tempdir().expect("temp dir");
    write_executable_project(dir.path());
    let build_dir = dir.path().join(crate::os::BUILD_DIR);
    std::fs::create_dir_all(&build_dir).expect("build dir");
    std::fs::write(build_dir.join("stale.txt"), b"stale").expect("stale");
    let options =
        parse_build_options(vec![dir.path().to_str().unwrap().to_string()]).expect("options");
    build_project(&options).expect("build should succeed");
    assert!(
        !build_dir.join("stale.txt").exists(),
        "stale file must be cleared by the build"
    );
    assert!(build_dir.exists(), "build dir is recreated by the writer");
}

#[test]
fn mfb_test_host_run_leaves_project_build_dir_untouched() {
    // plan-55-A §4.2: a `mfb test` host run links into a private temp dir and
    // must never clear the project's own `build/`. Seed one, run the tests, and
    // confirm the seeded file survives.
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(
        dir.path().join("project.json"),
        concat!(
            "{\n",
            "  \"name\": \"app\",\n",
            "  \"version\": \"0.1.0\",\n",
            "  \"mfb\": \"1.0\",\n",
            "  \"kind\": \"executable\",\n",
            "  \"entry\": \"main\",\n",
            "  \"targets\": [\"native\"],\n",
            "  \"sources\": [{ \"root\": \"src\", \"role\": \"main\", \"include\": [\"**/*.mfb\"] }]\n",
            "}\n"
        ),
    )
    .expect("manifest");
    std::fs::create_dir_all(dir.path().join("src")).expect("src dir");
    std::fs::write(
        dir.path().join("src").join("main.mfb"),
        concat!(
            "FUNC main AS Integer\n",
            "  RETURN 0\n",
            "END FUNC\n",
            "\n",
            "TESTING\n",
            "  TGROUP \"g\"\n",
            "    TCASE \"c\"\n",
            "      expectInteger(1, 1)\n",
            "    END TCASE\n",
            "  END TGROUP\n",
            "END TESTING\n"
        ),
    )
    .expect("source");
    let build_dir = dir.path().join(crate::os::BUILD_DIR);
    std::fs::create_dir_all(&build_dir).expect("build dir");
    std::fs::write(build_dir.join("keep.txt"), b"keep").expect("keep");
    let options =
        parse_test_options(vec![dir.path().to_str().unwrap().to_string()]).expect("options");
    build_project(&options).expect("mfb test should pass");
    assert!(
        build_dir.join("keep.txt").exists(),
        "mfb test host run must not clear the project build/"
    );
}

#[test]
fn build_project_writes_ast_and_ir_dumps() {
    let dir = tempfile::tempdir().expect("temp dir");
    write_executable_project(dir.path());
    let options = parse_build_options(s(&["-ast", "-ir", "-br", dir.path().to_str().unwrap()]))
        .expect("options");
    build_project(&options).expect("dump build should succeed");
}

fn write_package_project(dir: &Path) {
    std::fs::write(
        dir.join("project.json"),
        concat!(
            "{\n",
            "  \"name\": \"lib\",\n",
            "  \"version\": \"0.1.0\",\n",
            "  \"mfb\": \"1.0\",\n",
            "  \"kind\": \"package\",\n",
            // Required for `kind: "package"` since plan-61-F. Without it
            // this helper's manifest fails validation — which would also
            // make `build_project_rejects_native_output_for_a_package`
            // pass for the wrong reason, since it only asserts `is_err()`.
            "  \"description\": \"Test fixture package for the build CLI.\",\n",
            "  \"sources\": [{ \"root\": \"src\", \"role\": \"package\", \"include\": [\"**/*.mfb\"] }]\n",
            "}\n"
        ),
    )
    .expect("manifest");
    std::fs::create_dir_all(dir.join("src")).expect("src dir");
    std::fs::write(
        dir.join("src").join("lib.mfb"),
        "EXPORT FUNC answer() AS Integer\n  RETURN 42\nEND FUNC\n",
    )
    .expect("source");
}

#[test]
fn build_project_builds_a_package() {
    let dir = tempfile::tempdir().expect("temp dir");
    write_package_project(dir.path());
    let options =
        parse_build_options(vec![dir.path().to_str().unwrap().to_string()]).expect("options");
    build_project(&options).expect("package build should succeed");
    assert!(dir.path().join("lib.mfp").is_file());
}

#[test]
fn build_project_rejects_native_output_for_a_package() {
    let dir = tempfile::tempdir().expect("temp dir");
    write_package_project(dir.path());
    // A native code dump is unsupported for package projects.
    let options =
        parse_build_options(s(&["-ncode", dir.path().to_str().unwrap()])).expect("options");
    assert!(build_project(&options).is_err());
}

#[test]
fn build_project_reports_a_source_error() {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(
        dir.path().join("project.json"),
        concat!(
            "{\n",
            "  \"name\": \"app\",\n",
            "  \"version\": \"0.1.0\",\n",
            "  \"mfb\": \"1.0\",\n",
            "  \"kind\": \"executable\",\n",
            "  \"entry\": \"main\",\n",
            "  \"sources\": [{ \"root\": \"src\", \"role\": \"main\", \"include\": [\"**/*.mfb\"] }]\n",
            "}\n"
        ),
    )
    .expect("manifest");
    std::fs::create_dir_all(dir.path().join("src")).expect("src dir");
    // References an unknown package -> resolver/verify error.
    std::fs::write(
        dir.path().join("src").join("main.mfb"),
        "FUNC main() AS Nothing\n  nope::bogus()\nEND FUNC\n",
    )
    .expect("source");
    let options =
        parse_build_options(vec![dir.path().to_str().unwrap().to_string()]).expect("options");
    assert!(build_project(&options).is_err());
}

// ---- vendored native library copy (plan-46-D §4.5) ----

use crate::binary_repr::NativeLibraryLocator;
use crate::manifest::libraries::{LibType, Libc};

// Some CI environments run as root, where a `0o000` permission is ignored
// (root bypasses the check), so a permission-denied test would spuriously
// succeed at the very operation it means to fail. Probe once and skip those
// tests there rather than asserting a failure that cannot happen.
#[cfg(unix)]
fn running_as_root() -> bool {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let probe = dir.path().join("probe");
    std::fs::write(&probe, b"x").unwrap();
    let mut perm = std::fs::metadata(&probe).unwrap().permissions();
    perm.set_mode(0o000);
    std::fs::set_permissions(&probe, perm).unwrap();
    // A 0o000 file is still readable by root.
    std::fs::read(&probe).is_ok()
}

/// An `expect*` assertion outside a `TCASE` body is rejected before lowering.
#[test]
fn build_project_rejects_expect_outside_a_test_case() {
    let dir = tempfile::tempdir().unwrap();
    write_executable_project(dir.path());
    std::fs::write(
        dir.path().join("src").join("main.mfb"),
        "FUNC main() AS Nothing\n  expectInteger(1, 1)\nEND FUNC\n",
    )
    .unwrap();
    let options =
        parse_build_options(vec![dir.path().to_str().unwrap().to_string()]).expect("options");
    assert!(build_project(&options).is_err());
}

/// A `mfb test --coverage` host run writes the coverage map, runs the driver,
/// and folds the counts into a coverage report.
#[test]
fn build_project_coverage_test_writes_a_report() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("project.json"),
        concat!(
            "{\n",
            "  \"name\": \"app\",\n",
            "  \"version\": \"0.1.0\",\n",
            "  \"mfb\": \"1.0\",\n",
            "  \"kind\": \"executable\",\n",
            "  \"entry\": \"main\",\n",
            "  \"targets\": [\"native\"],\n",
            "  \"sources\": [{ \"root\": \"src\", \"role\": \"main\", \"include\": [\"**/*.mfb\"] }]\n",
            "}\n"
        ),
    )
    .unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("src").join("main.mfb"),
        concat!(
            "FUNC main AS Integer\n",
            "  RETURN 0\n",
            "END FUNC\n\n",
            "TESTING\n",
            "  TGROUP \"g\"\n",
            "    TCASE \"c\"\n",
            "      expectInteger(1, 1)\n",
            "    END TCASE\n",
            "  END TGROUP\n",
            "END TESTING\n"
        ),
    )
    .unwrap();
    let options =
        parse_test_options(s(&["--coverage", dir.path().to_str().unwrap()])).expect("options");
    build_project(&options).expect("coverage test passes");
    assert!(dir.path().join(crate::testing::COVMAP_FILE).is_file());
    assert!(dir.path().join(crate::testing::COVERAGE_HTML).is_file());
}

/// A coverage map that cannot be written (a directory holds its path) is a
/// warning, not a failure: the cases still build, run and pass.
#[test]
fn an_unwritable_coverage_map_does_not_fail_the_test_run() {
    let dir = tempfile::tempdir().unwrap();
    write_executable_project(dir.path());
    std::fs::write(
        dir.path().join("src").join("main.mfb"),
        concat!(
            "FUNC main AS Integer\n",
            "  RETURN 0\n",
            "END FUNC\n\n",
            "TESTING\n",
            "  TGROUP \"g\"\n",
            "    TCASE \"c\"\n",
            "      expectInteger(1, 1)\n",
            "    END TCASE\n",
            "  END TGROUP\n",
            "END TESTING\n"
        ),
    )
    .unwrap();
    std::fs::create_dir(dir.path().join(crate::testing::COVMAP_FILE)).unwrap();
    let options =
        parse_test_options(s(&["--coverage", dir.path().to_str().unwrap()])).expect("options");
    build_project(&options).expect("the cases still pass without a coverage map");
    assert!(dir.path().join(crate::testing::COVMAP_FILE).is_dir());
}

/// An unknown project `kind` is a warning, not an error: the build validates
/// and returns Ok having produced no artifact (bug-300 E8).
#[test]
fn build_project_unknown_kind_validates_and_builds_nothing() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("project.json"),
        concat!(
            "{\n",
            "  \"name\": \"app\",\n",
            "  \"version\": \"0.1.0\",\n",
            "  \"mfb\": \"1.0\",\n",
            "  \"kind\": \"program\",\n",
            "  \"entry\": \"main\",\n",
            "  \"sources\": [{ \"root\": \"src\", \"role\": \"main\", \"include\": [\"**/*.mfb\"] }]\n",
            "}\n"
        ),
    )
    .unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("src").join("main.mfb"),
        "FUNC main() AS Nothing\nEND FUNC\n",
    )
    .unwrap();
    let options =
        parse_build_options(vec![dir.path().to_str().unwrap().to_string()]).expect("options");
    build_project(&options).expect("unknown kind validates");
    assert!(!dir.path().join("app.mfp").exists());
}

/// Every native artifact dump writer runs for an executable project.
#[test]
fn build_project_writes_every_native_dump() {
    for flag in ["-nir", "-nplan", "-nobj", "-ncode", "-mir"] {
        let dir = tempfile::tempdir().unwrap();
        write_executable_project(dir.path());
        let options =
            parse_build_options(s(&[flag, dir.path().to_str().unwrap()])).expect("options");
        build_project(&options).unwrap_or_else(|_| panic!("{flag} dump should succeed"));
    }
}

/// A native dump is unsupported for a package project (each native flavor).
#[test]
fn build_project_rejects_native_dumps_for_a_package() {
    for flag in ["-nir", "-mir"] {
        let dir = tempfile::tempdir().unwrap();
        write_package_project(dir.path());
        let options =
            parse_build_options(s(&[flag, dir.path().to_str().unwrap()])).expect("options");
        assert!(
            build_project(&options).is_err(),
            "{flag} must be rejected for a package"
        );
    }
}

/// `--sign` combined with an artifact dump flag is rejected (signing is only
/// for a full package/executable build).
#[test]
fn build_project_rejects_sign_with_output_flags() {
    let dir = tempfile::tempdir().unwrap();
    write_executable_project(dir.path());
    let options = parse_build_options(s(&["--sign", "ada", "--ast", dir.path().to_str().unwrap()]))
        .expect("options");
    assert!(build_project(&options).is_err());
}

/// A `--sign` build with no local ident key fails fast at signing-info load
/// (no network), exercising the version/ident extraction and the load call
/// site (the registry request itself is the signing.rs boundary A excepts).
#[test]
fn build_project_sign_without_a_local_ident_key_fails() {
    let _lock = crate::cli::tests::ENV_LOCK.lock().unwrap();
    let home = tempfile::tempdir().unwrap();
    let _mfb = crate::cli::tests::EnvVarGuard::set("MFB_HOME", home.path().to_str().unwrap());
    let dir = tempfile::tempdir().unwrap();
    write_executable_project(dir.path());
    let options =
        parse_build_options(s(&["--sign", "ada", dir.path().to_str().unwrap()])).expect("options");
    assert!(build_project(&options).is_err());
}

/// A `LINK` naming a library with no `libraries` entry is a hard error: the
/// native-library table cannot be assembled, aborting the executable build
/// (and driving `assemble_native_library_table`'s error-finding loop).
#[test]
fn build_project_rejects_a_link_without_a_libraries_entry() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("project.json"),
        concat!(
            "{\n",
            "  \"name\": \"app\",\n",
            "  \"version\": \"0.1.0\",\n",
            "  \"mfb\": \"1.0\",\n",
            "  \"kind\": \"executable\",\n",
            "  \"entry\": \"main\",\n",
            "  \"targets\": [\"native\"],\n",
            "  \"sources\": [{ \"root\": \"src\", \"role\": \"main\", \"include\": [\"**/*.mfb\"] }]\n",
            "}\n"
        ),
    )
    .unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("src").join("main.mfb"),
        concat!(
            "LINK \"foo\" AS fooLib\n",
            "  FUNC ping() AS Nothing\n",
            "    SYMBOL \"foo_ping\"\n",
            "    ABI () AS status CInt32\n",
            "    SUCCESS_ON status = 0\n",
            "  END FUNC\n",
            "END LINK\n\n",
            "FUNC main() AS Nothing\n",
            "  fooLib::ping()\n",
            "END FUNC\n"
        ),
    )
    .unwrap();
    let options =
        parse_build_options(vec![dir.path().to_str().unwrap().to_string()]).expect("options");
    assert!(build_project(&options).is_err());
}

/// `--sign` with a manifest `ident` that is not `<owner>#<package>` fails at
/// `signing_ident` — before any registry/key access — exercising the signing
/// error arm.
#[test]
fn build_project_sign_rejects_a_malformed_manifest_ident() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("project.json"),
        concat!(
            "{\n",
            "  \"name\": \"app\",\n",
            "  \"version\": \"0.1.0\",\n",
            "  \"mfb\": \"1.0\",\n",
            "  \"kind\": \"executable\",\n",
            "  \"entry\": \"main\",\n",
            "  \"ident\": \"ada-app\",\n",
            "  \"targets\": [\"native\"],\n",
            "  \"sources\": [{ \"root\": \"src\", \"role\": \"main\", \"include\": [\"**/*.mfb\"] }]\n",
            "}\n"
        ),
    )
    .unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("src").join("main.mfb"),
        "IMPORT io\n\nFUNC main() AS Nothing\n  io::print(\"hi\")\nEND FUNC\n",
    )
    .unwrap();
    // `ada-app` has no `#`, so signing_ident rejects it.
    let options =
        parse_build_options(s(&["--sign", "zed", dir.path().to_str().unwrap()])).expect("options");
    assert!(build_project(&options).is_err());
}

/// A top-level `EXPORT` in an executable resolves cleanly but is rejected by
/// the export-in-executable diagnostic, driving the `had_error` return after
/// the verify phase (as distinct from a resolver name error).
#[test]
fn build_project_export_in_an_executable_is_a_verify_error() {
    let dir = tempfile::tempdir().unwrap();
    write_executable_project(dir.path());
    std::fs::write(
        dir.path().join("src").join("main.mfb"),
        "EXPORT FUNC leak() AS Integer\n  RETURN 1\nEND FUNC\n\nFUNC main() AS Nothing\nEND FUNC\n",
    )
    .unwrap();
    let options =
        parse_build_options(vec![dir.path().to_str().unwrap().to_string()]).expect("options");
    assert!(build_project(&options).is_err());
}

/// A `build/` path that already exists as a regular file (not a directory)
/// makes the start-of-build clear fail with a non-`NotFound` error, aborting
/// the executable build.
#[test]
fn build_project_errors_when_build_path_is_a_file() {
    let dir = tempfile::tempdir().unwrap();
    write_executable_project(dir.path());
    // A regular file where the build directory should be: remove_dir_all fails
    // with NotADirectory (not NotFound), which is the fatal arm.
    std::fs::write(dir.path().join(crate::os::BUILD_DIR), b"not a directory").unwrap();
    let options =
        parse_build_options(vec![dir.path().to_str().unwrap().to_string()]).expect("options");
    assert!(build_project(&options).is_err());
}

/// An executable declaring a local package dependency whose `.mfp` is not
/// installed passes package verification (a missing local dep is skipped there)
/// then fails at the strict install check inside the executable build.
#[test]
fn build_project_executable_missing_dependency_is_a_hard_error() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("project.json"),
        concat!(
            "{\n",
            "  \"name\": \"app\",\n",
            "  \"version\": \"0.1.0\",\n",
            "  \"mfb\": \"1.0\",\n",
            "  \"kind\": \"executable\",\n",
            "  \"entry\": \"main\",\n",
            "  \"targets\": [\"native\"],\n",
            "  \"packages\": [{ \"name\": \"ghost\", \"ident\": \"ada#ghost\", \"version\": \"0.1.0\", \"pin\": true, \"source\": \"file:packages/ghost.mfp\" }],\n",
            "  \"sources\": [{ \"root\": \"src\", \"role\": \"main\", \"include\": [\"**/*.mfb\"] }]\n",
            "}\n"
        ),
    )
    .unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("src").join("main.mfb"),
        "IMPORT io\n\nFUNC main() AS Nothing\n  io::print(\"hi\")\nEND FUNC\n",
    )
    .unwrap();
    let options =
        parse_build_options(vec![dir.path().to_str().unwrap().to_string()]).expect("options");
    assert!(build_project(&options).is_err());
}

/// The same missing-dependency install check gates a package build (the
/// package arm's install-check error).
#[test]
fn build_project_package_missing_dependency_is_a_hard_error() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("project.json"),
        concat!(
            "{\n",
            "  \"name\": \"lib\",\n",
            "  \"version\": \"0.1.0\",\n",
            "  \"mfb\": \"1.0\",\n",
            "  \"kind\": \"package\",\n",
            "  \"description\": \"Fixture package with a missing dependency.\",\n",
            "  \"packages\": [{ \"name\": \"ghost\", \"ident\": \"ada#ghost\", \"version\": \"0.1.0\", \"pin\": true, \"source\": \"file:packages/ghost.mfp\" }],\n",
            "  \"sources\": [{ \"root\": \"src\", \"role\": \"package\", \"include\": [\"**/*.mfb\"] }]\n",
            "}\n"
        ),
    )
    .unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("src").join("lib.mfb"),
        "EXPORT FUNC answer() AS Integer\n  RETURN 42\nEND FUNC\n",
    )
    .unwrap();
    let options =
        parse_build_options(vec![dir.path().to_str().unwrap().to_string()]).expect("options");
    assert!(build_project(&options).is_err());
}

/// A build that reads an installed dependency's exported function signatures
/// drives the `returns_imported_resource` filter closure over every external
/// signature (bug-377), even for the common non-resource return.
#[test]
fn build_project_reads_installed_dependency_exports() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("project.json"),
        concat!(
            "{\n",
            "  \"name\": \"app\",\n",
            "  \"version\": \"0.1.0\",\n",
            "  \"mfb\": \"1.0\",\n",
            "  \"kind\": \"executable\",\n",
            "  \"entry\": \"main\",\n",
            "  \"targets\": [\"native\"],\n",
            "  \"packages\": [{ \"name\": \"trap_builtin_pkg\", \"ident\": \"tests#trap\", \"version\": \"0.1.0\", \"pin\": false, \"source\": \"file:packages/trap_builtin_pkg.mfp\", \"direct\": true, \"requiredBy\": [] }],\n",
            "  \"sources\": [{ \"root\": \"src\", \"role\": \"main\", \"include\": [\"**/*.mfb\"] }]\n",
            "}\n"
        ),
    )
    .unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("src").join("main.mfb"),
        "IMPORT io\n\nFUNC main() AS Nothing\n  io::print(\"hi\")\nEND FUNC\n",
    )
    .unwrap();
    let packages = dir.path().join("packages");
    std::fs::create_dir_all(&packages).unwrap();
    std::fs::copy(
        "tests/syntax/packages/package-trap-builtin/golden/trap_builtin_pkg.mfp",
        packages.join("trap_builtin_pkg.mfp"),
    )
    .expect("copy fixture");
    let options =
        parse_build_options(vec![dir.path().to_str().unwrap().to_string()]).expect("options");
    // The build succeeds; the point is that the external-signature filter ran.
    build_project(&options).expect("build with an installed dependency succeeds");
}

/// Write a `kind: "package"` project at `dir` exporting `answer()`.
fn write_source_package(dir: &Path, name: &str) {
    std::fs::create_dir_all(dir.join("src")).expect("package src dir");
    std::fs::write(
        dir.join("project.json"),
        format!(
            concat!(
                "{{\n",
                "  \"name\": \"{name}\",\n",
                "  \"version\": \"0.1.0\",\n",
                "  \"mfb\": \"1.0\",\n",
                "  \"kind\": \"package\",\n",
                "  \"description\": \"A source-directory dependency.\",\n",
                "  \"targets\": [\"native\"],\n",
                "  \"sources\": [{{ \"root\": \"src\", \"role\": \"lib\", \"include\": [\"**/*.mfb\"] }}]\n",
                "}}\n"
            ),
            name = name
        ),
    )
    .expect("package manifest");
    std::fs::write(
        dir.join("src").join("lib.mfb"),
        "EXPORT FUNC answer() AS Integer\n  RETURN 42\nEND FUNC\n",
    )
    .expect("package source");
}

/// bug-480 Defect A: a dependency declared by SOURCE DIRECTORY is compiled
/// into `build/packages/<name>.mfp` and its exported signatures resolve, so
/// the call types as `Integer` instead of `Unknown`.
#[test]
fn build_project_compiles_a_source_directory_dependency() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("project.json"),
        concat!(
            "{\n",
            "  \"name\": \"app\",\n",
            "  \"version\": \"0.1.0\",\n",
            "  \"mfb\": \"1.0\",\n",
            "  \"kind\": \"executable\",\n",
            "  \"entry\": \"main\",\n",
            "  \"targets\": [\"native\"],\n",
            "  \"packages\": [{ \"name\": \"tiny\", \"version\": \"=0.1.0\", \"source\": \"file:packages/tiny\", \"direct\": true, \"requiredBy\": [] }],\n",
            "  \"sources\": [{ \"root\": \"src\", \"role\": \"main\", \"include\": [\"**/*.mfb\"] }]\n",
            "}\n"
        ),
    )
    .unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("src").join("main.mfb"),
        "IMPORT io\nIMPORT tiny\n\nFUNC main() AS Nothing\n  io::print(toString(tiny::answer()))\nEND FUNC\n",
    )
    .unwrap();
    write_source_package(&dir.path().join("packages").join("tiny"), "tiny");

    let options =
        parse_build_options(vec![dir.path().to_str().unwrap().to_string()]).expect("options");
    build_project(&options).expect("source-directory dependency builds");

    // The compiled interface lands in the cache, never beside the sources.
    let cache = crate::manifest::package::source_package_cache_dir(dir.path());
    assert!(
        cache.join("tiny.mfp").is_file(),
        "the dependency's .mfp belongs in build/packages/"
    );
    assert!(
        !dir.path()
            .join("packages")
            .join("tiny")
            .join("tiny.mfp")
            .exists(),
        "nothing is written into the dependency's source tree"
    );
    // The executable-branch build-dir clear must not take the cache with it:
    // its paths are what `write_executable` was handed.
    assert!(
        dir.path()
            .join(crate::os::BUILD_DIR)
            .join("app.out")
            .exists()
            || dir.path().join(crate::os::BUILD_DIR).exists()
    );
}

/// A source dependency that depends on itself must be a located, coded
/// diagnostic — not an infinite recursion.
#[test]
fn build_project_rejects_a_source_dependency_cycle() {
    let dir = tempfile::tempdir().unwrap();
    let package_dir = dir.path().join("packages").join("tiny");
    write_source_package(&package_dir, "tiny");
    // Point the package at ITSELF, by absolute path, so the two entries are
    // the same directory rather than two copies of one.
    let absolute = std::fs::canonicalize(&package_dir).expect("canonical package dir");
    std::fs::write(
        package_dir.join("project.json"),
        format!(
            concat!(
                "{{\n",
                "  \"name\": \"tiny\",\n",
                "  \"version\": \"0.1.0\",\n",
                "  \"mfb\": \"1.0\",\n",
                "  \"kind\": \"package\",\n",
                "  \"description\": \"A source package that depends on itself.\",\n",
                "  \"targets\": [\"native\"],\n",
                "  \"packages\": [{{ \"name\": \"tiny\", \"version\": \"=0.1.0\", \"source\": \"local://{}\" }}],\n",
                "  \"sources\": [{{ \"root\": \"src\", \"role\": \"lib\", \"include\": [\"**/*.mfb\"] }}]\n",
                "}}\n"
            ),
            absolute.display()
        ),
    )
    .unwrap();
    std::fs::write(
        dir.path().join("project.json"),
        concat!(
            "{\n",
            "  \"name\": \"app\",\n",
            "  \"version\": \"0.1.0\",\n",
            "  \"mfb\": \"1.0\",\n",
            "  \"kind\": \"executable\",\n",
            "  \"entry\": \"main\",\n",
            "  \"targets\": [\"native\"],\n",
            "  \"packages\": [{ \"name\": \"tiny\", \"version\": \"=0.1.0\", \"source\": \"file:packages/tiny\", \"direct\": true, \"requiredBy\": [] }],\n",
            "  \"sources\": [{ \"root\": \"src\", \"role\": \"main\", \"include\": [\"**/*.mfb\"] }]\n",
            "}\n"
        ),
    )
    .unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("src").join("main.mfb"),
        "IMPORT io\nIMPORT tiny\n\nFUNC main() AS Nothing\n  io::print(toString(tiny::answer()))\nEND FUNC\n",
    )
    .unwrap();

    let options =
        parse_build_options(vec![dir.path().to_str().unwrap().to_string()]).expect("options");
    assert!(
        build_project(&options).is_err(),
        "a dependency cycle must fail the build"
    );
    // The emitted identity is defined and non-sentinel.
    assert_eq!(
        crate::rules::code_and_name("IMPORT_PACKAGE_MANIFEST_INVALID"),
        ("2-201-0005", "IMPORT_PACKAGE_MANIFEST_INVALID")
    );
}

/// A cross-target `mfb test` build cannot run the produced binary on the host,
/// so it writes the test executable and reports the artifact instead of
/// executing it — driving the cross-target test-report arm.
#[test]
fn build_project_cross_target_test_reports_the_artifact() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("project.json"),
        concat!(
            "{\n",
            "  \"name\": \"app\",\n",
            "  \"version\": \"0.1.0\",\n",
            "  \"mfb\": \"1.0\",\n",
            "  \"kind\": \"executable\",\n",
            "  \"entry\": \"main\",\n",
            "  \"targets\": [\"native\"],\n",
            "  \"sources\": [{ \"root\": \"src\", \"role\": \"main\", \"include\": [\"**/*.mfb\"] }]\n",
            "}\n"
        ),
    )
    .unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("src").join("main.mfb"),
        concat!(
            "FUNC main AS Integer\n",
            "  RETURN 0\n",
            "END FUNC\n\n",
            "TESTING\n",
            "  TGROUP \"g\"\n",
            "    TCASE \"c\"\n",
            "      expectInteger(1, 1)\n",
            "    END TCASE\n",
            "  END TGROUP\n",
            "END TESTING\n"
        ),
    )
    .unwrap();
    let options = parse_test_options(s(&[
        "--target",
        "windows-x86_64",
        dir.path().to_str().unwrap(),
    ]))
    .expect("options");
    // A cross target cannot be run on the host; the driver is written and
    // reported (no execution, no host `build/` clobber).
    build_project(&options).expect("cross-target test build writes an artifact");
}

/// A package build whose output location is read-only fails when
/// `write_package` cannot create the `.mfp`, exercising that error arm.
#[test]
#[cfg(unix)]
fn build_project_package_write_to_a_readonly_location_fails() {
    use std::os::unix::fs::PermissionsExt;
    if running_as_root() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    write_package_project(dir.path());
    // Read + execute, but not write: sources still read, the .mfp cannot be
    // written.
    let mut perm = std::fs::metadata(dir.path()).unwrap().permissions();
    perm.set_mode(0o555);
    std::fs::set_permissions(dir.path(), perm).unwrap();
    let options =
        parse_build_options(vec![dir.path().to_str().unwrap().to_string()]).expect("options");
    let result = build_project(&options);
    // Restore write so the temp dir can be cleaned up.
    let mut restore = std::fs::metadata(dir.path()).unwrap().permissions();
    restore.set_mode(0o755);
    std::fs::set_permissions(dir.path(), restore).unwrap();
    assert!(
        result.is_err(),
        "write_package into a read-only dir must fail"
    );
}

/// Every artifact-dump writer surfaces its write error when the output location
/// is read-only, driving each dump's error arm (AST/IR/BR and the shared native
/// dump writer).
#[test]
#[cfg(unix)]
fn build_project_dump_writers_report_a_readonly_location() {
    use std::os::unix::fs::PermissionsExt;
    if running_as_root() {
        return;
    }
    for flag in [
        "-ast", "-ir", "-br", "-nir", "-nplan", "-nobj", "-ncode", "-mir",
    ] {
        let dir = tempfile::tempdir().unwrap();
        write_executable_project(dir.path());
        let mut perm = std::fs::metadata(dir.path()).unwrap().permissions();
        perm.set_mode(0o555);
        std::fs::set_permissions(dir.path(), perm).unwrap();
        let options =
            parse_build_options(s(&[flag, dir.path().to_str().unwrap()])).expect("options");
        let result = build_project(&options);
        let mut restore = std::fs::metadata(dir.path()).unwrap().permissions();
        restore.set_mode(0o755);
        std::fs::set_permissions(dir.path(), restore).unwrap();
        assert!(
            result.is_err(),
            "{flag}: a read-only location must fail the dump"
        );
    }
}
