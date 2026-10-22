// ---------------------------------------------------------------------------
// cli/build/packages.rs `verify_and_report_packages` — a pinned dependency
// whose installed version matches its pin, an installed package that breaks
// what an importer was compiled against, and a dependency closure that cannot
// be read.
// ---------------------------------------------------------------------------

use super::*;

/// Build package `name` `version` at `dir` from `source`, with `packages` as
/// its manifest's dependency list; answer the `.mfp`.
fn build_package(dir: &Path, name: &str, version: &str, source: &str, packages: &str) -> PathBuf {
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("project.json"),
        format!(
            "{{\"name\": \"{name}\", \"version\": \"{version}\", \"mfb\": \"1.0\", \"kind\": \
             \"package\", \"description\": \"d\", \"packages\": [{packages}], \"sources\": [{{ \
             \"root\": \"src\", \"role\": \"lib\", \"include\": [\"**/*.mfb\"] }}]}}"
        ),
    )
    .unwrap();
    std::fs::write(dir.join("src/lib.mfb"), source).unwrap();
    let options = parse_build_options(vec![dir.to_str().unwrap().to_string()]).unwrap();
    build_project(&options).expect("the package builds");
    dir.join(format!("{name}.mfp"))
}

/// `app/` declaring `user` (built against `base` 1.0.0, pinned) and `base`
/// (`base_pin` in the app's own entry), with `base` `installed` installed.
fn app_with(root: &Path, installed: &str, base_pin: &str) -> (PathBuf, HashMap<String, JsonValue>) {
    let base1 = build_package(
        &root.join("base1"),
        "base",
        "1.0.0",
        "EXPORT FUNC helper() AS Integer\n  RETURN 1\nEND FUNC\n",
        "",
    );
    let base2 = build_package(
        &root.join("base2"),
        "base",
        "2.0.0",
        "EXPORT FUNC helper() AS String\n  RETURN \"one\"\nEND FUNC\n",
        "",
    );
    let user_dir = root.join("user");
    std::fs::create_dir_all(user_dir.join("packages")).unwrap();
    std::fs::copy(&base1, user_dir.join("packages/base.mfp")).unwrap();
    let user = build_package(
        &user_dir,
        "user",
        "1.0.0",
        "IMPORT base\n\nEXPORT FUNC run() AS Integer\n  RETURN base::helper()\nEND FUNC\n",
        "{\"name\": \"base\", \"version\": \"1.0.0\", \"pin\": true, \"source\": \
         \"file:packages/base.mfp\", \"direct\": true, \"requiredBy\": []}",
    );
    let app = root.join("app");
    std::fs::create_dir_all(app.join("packages")).unwrap();
    std::fs::copy(&user, app.join("packages/user.mfp")).unwrap();
    let base = if installed == "2.0.0" { base2 } else { base1 };
    std::fs::copy(&base, app.join("packages/base.mfp")).unwrap();
    std::fs::write(
        app.join("project.json"),
        format!(
            "{{\"name\": \"app\", \"version\": \"0.1.0\", \"mfb\": \"1.0\", \"kind\": \
             \"package\", \"description\": \"d\", \"packages\": [\
             {{\"name\": \"user\", \"version\": \"1.0.0\", \"source\": \
             \"file:packages/user.mfp\", \"direct\": true, \"requiredBy\": []}}, \
             {{\"name\": \"base\", {base_pin}\"source\": \"file:packages/base.mfp\", \
             \"direct\": false, \"requiredBy\": [\"user\"]}}], \"sources\": [{{ \"root\": \
             \"src\", \"role\": \"lib\", \"include\": [\"**/*.mfb\"] }}]}}"
        ),
    )
    .unwrap();
    let manifest =
        crate::manifest::validate_project_manifest(&app.join("project.json")).expect("manifest");
    (app, manifest)
}

/// A pinned dependency installed at exactly its pinned version passes the pin
/// gate; the closure agrees, so the packages verify.
#[test]
fn a_pinned_dependency_at_its_pinned_version_verifies() {
    let root = tempfile::tempdir().unwrap();
    let (app, manifest) = app_with(
        root.path(),
        "1.0.0",
        "\"version\": \"1.0.0\", \"pin\": true, ",
    );
    let (result, diagnostics) =
        crate::rules::collecting_diagnostics(|| verify_and_report_packages(&app, &manifest, false));
    assert_eq!(result, Ok(()));
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
}

/// An installed `base` that breaks `user`'s pin and the ABI `user` was built
/// against refuses the build, each as a version conflict.
#[test]
fn an_installed_package_that_breaks_an_importer_is_a_version_conflict() {
    let root = tempfile::tempdir().unwrap();
    let (app, manifest) = app_with(root.path(), "2.0.0", "\"version\": \"2.0.0\", ");
    let (result, diagnostics) =
        crate::rules::collecting_diagnostics(|| verify_and_report_packages(&app, &manifest, false));
    assert_eq!(result, Err(()));
    assert_eq!(
        diagnostics,
        vec![
            (
                "PACKAGE_VERSION_CONFLICT".to_string(),
                "package `user` pins `base` 1.0.0, but the installed `base` is 2.0.0; run `mfb \
                 pkg verify` for details"
                    .to_string()
            ),
            (
                "PACKAGE_VERSION_CONFLICT".to_string(),
                "package `user` was built against a `base` incompatible with the installed one: \
                 its `helper` differs from `base` 2.0.0; run `mfb pkg verify` for details"
                    .to_string()
            ),
        ]
    );
}

/// A declared source dependency whose `project.json` cannot be parsed leaves
/// the closure unreadable: the build refuses and says so.
#[test]
fn an_unreadable_dependency_closure_refuses_the_build() {
    let root = tempfile::tempdir().unwrap();
    let app = root.path().join("app");
    std::fs::create_dir_all(app.join("broken")).unwrap();
    std::fs::write(app.join("broken/project.json"), "{ not json").unwrap();
    std::fs::write(
        app.join("project.json"),
        "{\"name\": \"app\", \"version\": \"0.1.0\", \"mfb\": \"1.0\", \"kind\": \"package\", \
         \"description\": \"d\", \"packages\": [{\"name\": \"broken\", \"version\": \"1.0.0\", \
         \"source\": \"file:broken\", \"direct\": true, \"requiredBy\": []}], \"sources\": [{ \
         \"root\": \"src\", \"role\": \"lib\", \"include\": [\"**/*.mfb\"] }]}",
    )
    .unwrap();
    let manifest =
        crate::manifest::validate_project_manifest(&app.join("project.json")).expect("manifest");
    let (result, diagnostics) =
        crate::rules::collecting_diagnostics(|| verify_and_report_packages(&app, &manifest, false));
    assert_eq!(result, Err(()));
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_eq!(diagnostics[0].0, "PACKAGE_DEPENDENCIES_INCONSISTENT");
    assert!(
        diagnostics[0]
            .1
            .starts_with("cannot read the dependency closure: "),
        "{}",
        diagnostics[0].1
    );
}
