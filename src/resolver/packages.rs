use super::*;

impl Resolver<'_> {
    pub(super) fn resolve_imported_package(&mut self, file: &AstFile, name: &str, line: usize) {
        if is_builtin_import(name) {
            return;
        }

        let Some(dependency) = self.dependency_packages.get(name).cloned() else {
            self.report(
                "IMPORT_PACKAGE_NOT_DECLARED",
                &format!(
                    "Package `{name}` is not built in and is not declared in project.json packages."
                ),
                file,
                line,
            );
            return;
        };

        if let Some(source) = dependency.source.as_deref() {
            if source.starts_with("local://") {
                self.resolve_local_dependency(file, name, source, line);
                return;
            }
        }

        let package_file = self
            .project_dir
            .join("packages")
            .join(format!("{name}.mfp"));
        if package_file.is_file() {
            self.install_package_type_names(file, name, &package_file, line);
            return;
        }

        let package_manifest = self
            .project_dir
            .join("packages")
            .join(name)
            .join("project.json");
        if package_manifest.is_file() {
            self.validate_source_package_manifest(file, name, &package_manifest, line);
            return;
        }

        self.report(
            "IMPORT_PACKAGE_NOT_INSTALLED",
            &format!(
                "Declared package `{name}` was not found at `{}` or `{}`.",
                package_file.display(),
                package_manifest.display()
            ),
            file,
            line,
        );
    }

    fn install_package_type_names(
        &mut self,
        file: &AstFile,
        name: &str,
        package_file: &Path,
        line: usize,
    ) {
        let exports = match binary_repr::read_package_type_exports(package_file) {
            Ok(exports) => exports,
            Err(err) => {
                self.report(
                    "IMPORT_PACKAGE_INVALID",
                    &format!("Package `{name}` type exports could not be read: {err}"),
                    file,
                    line,
                );
                return;
            }
        };
        for export in exports {
            self.types.insert(export.name);
            for variant in export.variants {
                self.types.insert(variant.name);
            }
        }
    }

    fn resolve_local_dependency(&mut self, file: &AstFile, name: &str, source: &str, line: usize) {
        let Some(path) = source.strip_prefix("local://") else {
            unreachable!("checked local scheme");
        };
        let path = PathBuf::from(path);
        if !path.is_absolute() {
            self.report(
                "IMPORT_LOCAL_PATH_INVALID",
                &format!("Local package source for `{name}` must use `local:///absolute/path`."),
                file,
                line,
            );
            return;
        }

        let manifest_path = path.join("project.json");
        if !manifest_path.is_file() {
            self.report(
                "IMPORT_PACKAGE_NOT_INSTALLED",
                &format!(
                    "Local package `{name}` does not have a project.json at `{}`.",
                    manifest_path.display()
                ),
                file,
                line,
            );
            return;
        }

        self.validate_source_package_manifest(file, name, &manifest_path, line);
    }

    fn validate_source_package_manifest(
        &mut self,
        file: &AstFile,
        expected_name: &str,
        manifest_path: &Path,
        line: usize,
    ) {
        let Some(manifest) = read_manifest(manifest_path) else {
            self.report(
                "IMPORT_PACKAGE_MANIFEST_INVALID",
                &format!(
                    "Could not read package manifest `{}`.",
                    manifest_path.display()
                ),
                file,
                line,
            );
            return;
        };

        let actual_name = manifest.get("name").and_then(|value| value.get::<String>());
        if actual_name.map(String::as_str) != Some(expected_name) {
            self.report(
                "IMPORT_PACKAGE_NAME_MISMATCH",
                &format!(
                    "Imported package `{expected_name}` must have matching `name` in `{}`.",
                    manifest_path.display()
                ),
                file,
                line,
            );
            return;
        }

        let kind = manifest.get("kind").and_then(|value| value.get::<String>());
        if kind.map(String::as_str) != Some("package") {
            self.report(
                "IMPORT_PACKAGE_KIND_INVALID",
                &format!(
                    "Imported source package `{expected_name}` must declare `\"kind\": \"package\"` in `{}`.",
                    manifest_path.display()
                ),
                file,
                line,
            );
        }
    }
}

#[derive(Clone)]
pub(super) struct DependencyPackage {
    source: Option<String>,
}

pub(super) fn dependency_packages(
    manifest: &HashMap<String, JsonValue>,
) -> HashMap<String, DependencyPackage> {
    manifest
        .get("packages")
        .and_then(|value| value.get::<Vec<JsonValue>>())
        .into_iter()
        .flatten()
        .filter_map(|package| package.get::<HashMap<String, JsonValue>>())
        .filter_map(|package| {
            let name = package.get("name")?.get::<String>()?.clone();
            let source = package
                .get("source")
                .and_then(|value| value.get::<String>())
                .cloned();
            Some((name, DependencyPackage { source }))
        })
        .collect()
}

pub(super) fn read_manifest(path: &Path) -> Option<HashMap<String, JsonValue>> {
    let contents = fs::read_to_string(path).ok()?;
    let json = contents.parse::<JsonValue>().ok()?;
    json.get::<HashMap<String, JsonValue>>().cloned()
}

pub(super) fn qualify_package_name(name: &str, binding: &str, package: &str) -> String {
    if binding == package {
        return name.to_string();
    }
    format!("{package}.{}", &name[binding.len() + 1..])
}

fn is_builtin_import(name: &str) -> bool {
    builtins::is_builtin_import(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::AstFile;
    use tempfile::tempdir;

    fn empty_file() -> AstFile {
        AstFile {
            path: "main.mfb".to_string(),
            imports: Vec::new(),
            items: Vec::new(),
            internal: false,
        }
    }

    /// Build a project manifest declaring one package with the given optional
    /// `source`.
    fn manifest_with_package(name: &str, source: Option<&str>) -> HashMap<String, JsonValue> {
        let mut pkg: HashMap<String, JsonValue> = HashMap::new();
        pkg.insert("name".to_string(), JsonValue::String(name.to_string()));
        if let Some(source) = source {
            pkg.insert("source".to_string(), JsonValue::String(source.to_string()));
        }
        let mut root: HashMap<String, JsonValue> = HashMap::new();
        root.insert(
            "packages".to_string(),
            JsonValue::Array(vec![JsonValue::Object(pkg)]),
        );
        root
    }

    /// Run `resolve_imported_package` for `name` against a fresh resolver rooted at
    /// `project_dir` with the given manifest, returning whether it reported an
    /// error.
    fn resolve_import(
        project_dir: &Path,
        manifest: &HashMap<String, JsonValue>,
        name: &str,
    ) -> bool {
        let ast = AstProject {
            name: "app".to_string(),
            files: vec![empty_file()],
        };
        let mut resolver = Resolver::new(project_dir, manifest, &ast);
        resolver.resolve_imported_package(&ast.files[0], name, 1);
        resolver.had_error
    }

    #[test]
    fn dependency_packages_parses_name_and_optional_source() {
        let manifest = manifest_with_package("shape", Some("local:///abs"));
        let deps = dependency_packages(&manifest);
        assert_eq!(deps.len(), 1);
        assert_eq!(deps["shape"].source.as_deref(), Some("local:///abs"));

        let manifest = manifest_with_package("shape", None);
        let deps = dependency_packages(&manifest);
        assert!(deps["shape"].source.is_none());
    }

    #[test]
    fn dependency_packages_empty_when_no_packages_key() {
        let deps = dependency_packages(&HashMap::new());
        assert!(deps.is_empty());
    }

    #[test]
    fn dependency_packages_skips_entries_without_name() {
        let mut pkg: HashMap<String, JsonValue> = HashMap::new();
        pkg.insert("source".to_string(), JsonValue::String("x".to_string()));
        let mut root: HashMap<String, JsonValue> = HashMap::new();
        root.insert(
            "packages".to_string(),
            JsonValue::Array(vec![JsonValue::Object(pkg)]),
        );
        assert!(dependency_packages(&root).is_empty());
    }

    #[test]
    fn read_manifest_ok_and_error_paths() {
        let dir = tempdir().unwrap();
        let good = dir.path().join("good.json");
        fs::write(&good, "{ \"name\": \"x\" }").unwrap();
        let manifest = read_manifest(&good).expect("valid manifest");
        assert_eq!(
            manifest.get("name").and_then(|v| v.get::<String>()),
            Some(&"x".to_string())
        );

        // Missing file → None.
        assert!(read_manifest(&dir.path().join("missing.json")).is_none());

        // Invalid JSON → None.
        let bad = dir.path().join("bad.json");
        fs::write(&bad, "not json").unwrap();
        assert!(read_manifest(&bad).is_none());

        // Valid JSON but not an object → None.
        let arr = dir.path().join("arr.json");
        fs::write(&arr, "[1, 2, 3]").unwrap();
        assert!(read_manifest(&arr).is_none());
    }

    #[test]
    fn qualify_package_name_both_branches() {
        // Same binding and package: name is returned verbatim.
        assert_eq!(qualify_package_name("draw", "shape", "shape"), "draw");
        // Rebinding: the binding prefix is swapped for the real package name.
        assert_eq!(
            qualify_package_name("geo.draw", "geo", "shape"),
            "shape.draw"
        );
    }

    #[test]
    fn builtin_import_short_circuits() {
        let dir = tempdir().unwrap();
        // `io` is built in: no error even with an empty manifest.
        assert!(!resolve_import(dir.path(), &HashMap::new(), "io"));
    }

    #[test]
    fn undeclared_package_is_reported() {
        let dir = tempdir().unwrap();
        assert!(resolve_import(dir.path(), &HashMap::new(), "shape"));
    }

    #[test]
    fn declared_but_not_installed_is_reported() {
        let dir = tempdir().unwrap();
        let manifest = manifest_with_package("shape", None);
        // No packages/shape.mfp and no packages/shape/project.json.
        assert!(resolve_import(dir.path(), &manifest, "shape"));
    }

    #[test]
    fn present_mfp_that_is_garbage_is_reported() {
        let dir = tempdir().unwrap();
        let packages = dir.path().join("packages");
        fs::create_dir_all(&packages).unwrap();
        fs::write(packages.join("shape.mfp"), b"not a real package").unwrap();
        let manifest = manifest_with_package("shape", None);
        assert!(resolve_import(dir.path(), &manifest, "shape"));
    }

    #[test]
    fn present_valid_mfp_installs_type_names() {
        let dir = tempdir().unwrap();
        let packages = dir.path().join("packages");
        fs::create_dir_all(&packages).unwrap();
        // A real, valid package binary-representation fixture exercises the
        // success loop that inserts exported type/variant names.
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/rt-behavior/project/project-with-package-import-as/packages/package_import_as.mfp");
        fs::copy(&fixture, packages.join("shape.mfp")).unwrap();
        let manifest = manifest_with_package("shape", None);
        // Reading valid exports must not report an error.
        assert!(!resolve_import(dir.path(), &manifest, "shape"));
    }

    #[test]
    fn source_package_dir_valid_manifest_passes() {
        let dir = tempdir().unwrap();
        let pkg_dir = dir.path().join("packages").join("shape");
        fs::create_dir_all(&pkg_dir).unwrap();
        fs::write(
            pkg_dir.join("project.json"),
            "{ \"name\": \"shape\", \"kind\": \"package\" }",
        )
        .unwrap();
        let manifest = manifest_with_package("shape", None);
        assert!(!resolve_import(dir.path(), &manifest, "shape"));
    }

    #[test]
    fn source_package_dir_unreadable_manifest_is_reported() {
        let dir = tempdir().unwrap();
        let pkg_dir = dir.path().join("packages").join("shape");
        fs::create_dir_all(&pkg_dir).unwrap();
        fs::write(pkg_dir.join("project.json"), "not json").unwrap();
        let manifest = manifest_with_package("shape", None);
        assert!(resolve_import(dir.path(), &manifest, "shape"));
    }

    #[test]
    fn source_package_dir_name_mismatch_is_reported() {
        let dir = tempdir().unwrap();
        let pkg_dir = dir.path().join("packages").join("shape");
        fs::create_dir_all(&pkg_dir).unwrap();
        fs::write(
            pkg_dir.join("project.json"),
            "{ \"name\": \"other\", \"kind\": \"package\" }",
        )
        .unwrap();
        let manifest = manifest_with_package("shape", None);
        assert!(resolve_import(dir.path(), &manifest, "shape"));
    }

    #[test]
    fn source_package_dir_wrong_kind_is_reported() {
        let dir = tempdir().unwrap();
        let pkg_dir = dir.path().join("packages").join("shape");
        fs::create_dir_all(&pkg_dir).unwrap();
        fs::write(
            pkg_dir.join("project.json"),
            "{ \"name\": \"shape\", \"kind\": \"executable\" }",
        )
        .unwrap();
        let manifest = manifest_with_package("shape", None);
        assert!(resolve_import(dir.path(), &manifest, "shape"));
    }

    #[test]
    fn local_source_relative_path_is_reported() {
        let dir = tempdir().unwrap();
        let manifest = manifest_with_package("shape", Some("local://relative/path"));
        assert!(resolve_import(dir.path(), &manifest, "shape"));
    }

    #[test]
    fn local_source_missing_manifest_is_reported() {
        let dir = tempdir().unwrap();
        let absent = dir.path().join("absent-pkg");
        let source = format!("local://{}", absent.display());
        let manifest = manifest_with_package("shape", Some(&source));
        assert!(resolve_import(dir.path(), &manifest, "shape"));
    }

    #[test]
    fn local_source_valid_manifest_passes() {
        let dir = tempdir().unwrap();
        let pkg_dir = dir.path().join("external-shape");
        fs::create_dir_all(&pkg_dir).unwrap();
        fs::write(
            pkg_dir.join("project.json"),
            "{ \"name\": \"shape\", \"kind\": \"package\" }",
        )
        .unwrap();
        let source = format!("local://{}", pkg_dir.display());
        let manifest = manifest_with_package("shape", Some(&source));
        assert!(!resolve_import(dir.path(), &manifest, "shape"));
    }
}
