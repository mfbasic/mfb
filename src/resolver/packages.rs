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
