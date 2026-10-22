// ---------------------------------------------------------------------------
// cli/build/source_packages.rs — source dependencies that are skipped or
// refused: an invalid declared name, a graph nested past the depth limit, a
// `vendor` that is not a directory, an uncreatable cache, and a `build/` that
// is not a directory.
// ---------------------------------------------------------------------------

use super::*;

/// Write package `name` at `dir`, depending on the sibling `next` (if any).
fn write_package(dir: &Path, name: &str, next: Option<&str>) {
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let packages = next
        .map(|next| {
            format!(
                "\"packages\": [{{ \"name\": \"{next}\", \"version\": \"=0.1.0\", \"source\": \
                 \"file:../{next}\", \"direct\": true, \"requiredBy\": [] }}], "
            )
        })
        .unwrap_or_default();
    std::fs::write(
        dir.join("project.json"),
        format!(
            "{{\"name\": \"{name}\", \"version\": \"0.1.0\", \"mfb\": \"1.0\", \"kind\": \
             \"package\", \"description\": \"d\", {packages}\"sources\": [{{ \"root\": \"src\", \
             \"role\": \"lib\", \"include\": [\"**/*.mfb\"] }}]}}"
        ),
    )
    .unwrap();
    std::fs::write(
        dir.join("src/lib.mfb"),
        "EXPORT FUNC one() AS Integer\n  RETURN 1\nEND FUNC\n",
    )
    .unwrap();
}

fn manifest_of(dir: &Path) -> HashMap<String, JsonValue> {
    crate::manifest::validate_project_manifest(&dir.join("project.json")).expect("manifest")
}

/// A declared dependency whose name is not a valid package name is left to
/// the resolver: nothing is compiled for it.
#[test]
fn a_dependency_with_an_invalid_name_is_not_compiled() {
    let root = tempfile::tempdir().unwrap();
    write_package(&root.path().join("dep"), "dep", None);
    let manifest = "{ \"packages\": [ { \"name\": \"Not Valid\", \"source\": \"file:dep\" } ] }"
        .parse::<JsonValue>()
        .unwrap()
        .get::<HashMap<String, JsonValue>>()
        .unwrap()
        .clone();
    assert_eq!(
        build_source_dependency_interfaces(root.path(), &manifest),
        Ok(())
    );
    assert!(!crate::manifest::package::source_package_cache_dir(root.path()).exists());
}

/// A chain of source packages nested past the limit is refused at the
/// manifest that goes one level too deep.
#[test]
fn a_dependency_chain_nested_past_the_limit_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let names: Vec<String> = (0..=33).map(|level| format!("d{level}")).collect();
    for (level, name) in names.iter().enumerate() {
        write_package(
            &root.path().join(name),
            name,
            names.get(level + 1).map(String::as_str),
        );
    }
    let top = root.path().join("d0");
    let (result, diagnostics) = crate::rules::collecting_diagnostics(|| {
        build_source_dependency_interfaces(&top, &manifest_of(&top))
    });
    assert_eq!(result, Err(()));
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_eq!(diagnostics[0].0, "IMPORT_PACKAGE_MANIFEST_INVALID");
    assert!(
        diagnostics[0]
            .1
            .starts_with("source package dependencies nest more than 32 levels deep at `"),
        "{}",
        diagnostics[0].1
    );
}

/// A dependency whose `vendor` is a file, not a directory, cannot be staged:
/// the build refuses rather than linking without its libraries.
#[test]
fn a_vendor_that_is_not_a_directory_refuses_the_build() {
    let root = tempfile::tempdir().unwrap();
    let app = root.path().join("app");
    write_package(&app, "app", Some("dep"));
    write_package(&root.path().join("dep"), "dep", None);
    std::fs::write(root.path().join("dep/vendor"), b"not a directory").unwrap();
    assert!(source_packages::stage_source_dependency_vendor(
        &root.path().join("dep"),
        root.path(),
        "dep"
    )
    .is_err());
    assert_eq!(
        build_source_dependency_interfaces(&app, &manifest_of(&app)),
        Err(())
    );
    assert!(!crate::manifest::package::source_package_cache_dir(&app)
        .join("dep.mfp")
        .exists());
}

/// A `build/` the cache cannot be created in refuses the build.
#[test]
#[cfg(unix)]
fn an_uncreatable_source_package_cache_refuses_the_build() {
    use std::os::unix::fs::PermissionsExt;
    if running_as_root() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let app = root.path().join("app");
    write_package(&app, "app", Some("dep"));
    write_package(&root.path().join("dep"), "dep", None);
    let build = app.join("build");
    std::fs::create_dir_all(&build).unwrap();
    std::fs::set_permissions(&build, std::fs::Permissions::from_mode(0o500)).unwrap();
    let result = build_source_dependency_interfaces(&app, &manifest_of(&app));
    std::fs::set_permissions(&build, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(result, Err(()));
}

/// Clearing a `build/` that is a file, not a directory, is an error.
#[test]
fn clearing_a_build_path_that_is_a_file_fails() {
    let root = tempfile::tempdir().unwrap();
    let build = root.path().join("build");
    std::fs::write(&build, b"x").unwrap();
    assert!(source_packages::clear_build_dir(&build).is_err());
}
