use super::*;

impl Resolver<'_> {
    pub(super) fn resolve_imported_package(&mut self, file: &HirFile, name: &str, line: usize) {
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

        // An installed `packages/<name>.mfp` IS the dependency, whatever the
        // `source` field says — it is what `resolved_package_file` picks and what
        // the build declines to recompile. Its own sources are not consulted.
        let installed = self
            .project_dir
            .join("packages")
            .join(format!("{name}.mfp"));
        if installed.is_file() {
            self.install_package_type_names(file, name, &installed, line);
            return;
        }

        // Otherwise the dependency is declared by SOURCE DIRECTORY (bug-480
        // Defect A): validate its own manifest, then load the interface from the
        // `.mfp` this build compiled into its package cache. Both dependency
        // forms therefore reach the front end through one reader.
        let source_dir =
            match source_dependency(self.project_dir, name, dependency.source.as_deref()) {
                SourceDependency::LocalPathNotAbsolute => {
                    self.report(
                        "IMPORT_LOCAL_PATH_INVALID",
                        &format!(
                            "Local package source for `{name}` must use `local:///absolute/path`."
                        ),
                        file,
                        line,
                    );
                    return;
                }
                SourceDependency::Directory(dir) => dir,
                // A `.mfp`/registry source with no installed file: there is
                // nothing else to look at.
                SourceDependency::Compiled => {
                    self.report(
                        "IMPORT_PACKAGE_NOT_INSTALLED",
                        &format!(
                            "Declared package `{name}` was not found at `{}`.",
                            installed.display()
                        ),
                        file,
                        line,
                    );
                    return;
                }
            };

        let package_manifest = source_dir.join("project.json");
        if !package_manifest.is_file() {
            self.report(
                "IMPORT_PACKAGE_NOT_INSTALLED",
                &format!(
                    "Declared package `{name}` was not found at `{}` or `{}`.",
                    installed.display(),
                    package_manifest.display()
                ),
                file,
                line,
            );
            return;
        }

        if !self.validate_source_package_manifest(file, name, &package_manifest, line) {
            return;
        }

        // The cache entry is absent only when the source package failed to
        // build, which `build_source_dependencies` already reported against the
        // dependency's own sources; do not blame the import line for it.
        if let Some(package_file) = resolved_package_file(self.project_dir, name) {
            self.install_package_type_names(file, name, &package_file, line);
        }
    }

    fn install_package_type_names(
        &mut self,
        file: &HirFile,
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
        // bug-301 G1 asked for these names to be inserted PREFIXED, so that only a
        // qualified `pkg.Type` reference resolves. That was investigated and
        // rejected: bare imported type names are the established convention, not a
        // leniency. `tests/rt-behavior/native/native-resource-state-import-rt`
        // writes `RES h AS Db STATE DbInfo` for types imported from a `.mfp`, and
        // it builds, links and runs. Prefixing here would break it and every
        // importer like it.
        //
        // The report's basis was `architecture/03_packages.md`'s
        // `packageName.exportName`, but that passage describes the *internal*
        // signature names lowering creates for imported FUNCTIONS, not the source
        // syntax for naming an imported type. Both `DbInfo` and `db::DbInfo`
        // resolve in a type position today (`db.DbInfo` is a parse error, since
        // dot is field access); `resolver::packages::tests` pins that.
        //
        // bug-480: the same names also form this package's visible surface, so a
        // `pkg::member` naming something the package does not export can be
        // refused where it is written instead of typing as `Unknown` and
        // surfacing as an argument-type error at an unrelated call.
        let mut visible: HashSet<String> = HashSet::new();
        for export in exports {
            self.types
                .insert(crate::types::ParameterType::declared(&export.name));
            visible.insert(export.name.clone());
            visible.extend(export.members.iter().cloned());
            for variant in export.variants {
                self.types
                    .insert(crate::types::ParameterType::declared(&variant.name));
                visible.insert(variant.name);
            }
        }
        let Ok(exports) = binary_repr::read_package_exports(package_file) else {
            // The type table read but the export table did not. Both come off one
            // container, so this is a corrupt `.mfp` the shape pass reports as
            // `PACKAGE_INVALID`; record no surface rather than a partial one that
            // would reject valid calls.
            return;
        };
        for export in exports {
            // Monomorphization rewrites a call to an overloaded import to the
            // mangled `base$signature` spelling, and both passes of the resolver
            // run over the same table, so accept either.
            if let Some((base, _)) = export.name.split_once('$') {
                visible.insert(base.to_string());
            }
            visible.insert(export.name);
        }
        // The ABI export table carries ONLY functions and record/union/enum
        // types (`AbiIndex::from_project`). A package's remaining top-level
        // surface lives in two other sections, and both have to be unioned in
        // here or a valid program would be refused:
        //
        //   - the RESOURCE table: each resource TYPE, and the close op a native
        //     resource re-exports as a bare alias (`EXPORT FUNC close AS
        //     link::op`, plan-link-update.md §5a) — `sqlite3::close` is exactly
        //     that, and it appears in no export row;
        //   - the GLOBAL table: `EXPORT MUT` / `EXPORT LET` package state, which
        //     `13_modules-and-packages.md` calls visible to importers.
        if let Ok(resources) = binary_repr::read_package_resources(package_file) {
            for resource in resources {
                visible.insert(resource.type_name);
                if let Some(close) = resource.close_function {
                    // Built-in close ops are dotted (`fs.close`); the importer
                    // writes only the member.
                    let member = close.rsplit('.').next().unwrap_or(&close).to_string();
                    visible.insert(member);
                    visible.insert(close);
                }
            }
        }
        if let Ok(info) = binary_repr::read_package_info(package_file) {
            for global in info.globals {
                visible.insert(global.name);
            }
        }
        self.package_exports.insert(name.to_string(), visible);
    }

    /// Validate a source-directory dependency's own manifest. Returns whether it
    /// is usable; a `false` return has already reported the reason.
    fn validate_source_package_manifest(
        &mut self,
        file: &HirFile,
        expected_name: &str,
        manifest_path: &Path,
        line: usize,
    ) -> bool {
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
            return false;
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
            return false;
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
            return false;
        }
        true
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
    let json = crate::json::parse_json_bounded(&contents).ok()?;
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
    use crate::hir::HirFile;
    use tempfile::tempdir;

    fn empty_file() -> HirFile {
        HirFile {
            path: "main.mfb".to_string(),
            imports: Vec::new(),
            own_imports: Vec::new(),
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
        let hir = crate::hir::HirProject {
            name: "app".to_string(),
            files: vec![empty_file()],
        };
        let mut resolver = Resolver::new(project_dir, manifest, &hir);
        resolver.resolve_imported_package(&hir.files[0], name, 1);
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

    /// A manifest declaring only `kind`, for the `IMPORT self` package/executable
    /// gate (plan-81-import-self.md §4.3).
    fn manifest_with_kind(kind: &str) -> HashMap<String, JsonValue> {
        let mut root: HashMap<String, JsonValue> = HashMap::new();
        root.insert("kind".to_string(), JsonValue::String(kind.to_string()));
        root
    }

    /// plan-115-B: `IMPORT self` is gone, so `self` is an ordinary package name
    /// and resolves through the normal order — i.e. it is undeclared like any
    /// other. This replaces `self_import_in_package_is_ok` /
    /// `self_import_in_executable_is_reported`, which pinned the reserved
    /// specifier's two outcomes; the feature they described no longer exists.
    #[test]
    fn self_is_an_ordinary_package_name() {
        let dir = tempdir().unwrap();
        // Reported in BOTH kinds now, and for the ordinary reason
        // (IMPORT_PACKAGE_NOT_DECLARED) rather than a reserved-specifier rule.
        assert!(resolve_import(
            dir.path(),
            &manifest_with_kind("package"),
            "self"
        ));
        assert!(resolve_import(
            dir.path(),
            &manifest_with_kind("executable"),
            "self"
        ));
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
        // bug-40: this path emits `IMPORT_PACKAGE_INVALID`, which must be a defined
        // rule. `resolve_import` -> `report` -> `show_diagnostic` -> `rule_for`
        // debug-asserts the name resolves, so this test panics if the emit site and
        // the rule table drift (previously it degraded to `0-000-0000 UNKNOWN_RULE`).
        assert!(resolve_import(dir.path(), &manifest, "shape"));
        // The emitted identity is defined and non-sentinel.
        assert_eq!(
            crate::rules::code_and_name("IMPORT_PACKAGE_INVALID"),
            ("2-201-0001", "IMPORT_PACKAGE_INVALID")
        );
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

    /// bug-480: a source-directory dependency is compiled into the build's
    /// package cache before resolution runs, so its exported TYPE names must
    /// install from there exactly as an installed `.mfp`'s do — and its manifest
    /// is still validated on the way through.
    #[test]
    fn source_package_dir_installs_type_names_from_the_build_cache() {
        let dir = tempdir().unwrap();
        let pkg_dir = dir.path().join("packages").join("shape");
        fs::create_dir_all(&pkg_dir).unwrap();
        fs::write(
            pkg_dir.join("project.json"),
            "{ \"name\": \"shape\", \"kind\": \"package\" }",
        )
        .unwrap();
        let cache = crate::manifest::package::source_package_cache_dir(dir.path());
        fs::create_dir_all(&cache).unwrap();
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/rt-behavior/project/project-with-package-import-as/packages/package_import_as.mfp");
        fs::copy(&fixture, cache.join("shape.mfp")).unwrap();

        let manifest = manifest_with_package("shape", Some("file:packages/shape"));
        let hir = crate::hir::HirProject {
            name: "app".to_string(),
            files: vec![empty_file()],
        };
        let mut resolver = Resolver::new(dir.path(), &manifest, &hir);
        resolver.resolve_imported_package(&hir.files[0], "shape", 1);
        assert!(!resolver.had_error);
        // The export surface was recorded, so an unknown member can be refused
        // by name rather than leaking `Unknown` (bug-480 Phase 2).
        let exports = resolver
            .package_exports
            .get("shape")
            .expect("source-package exports recorded");
        assert!(exports.contains("byteLenAlias"), "{exports:?}");
        assert!(!exports.contains("noSuchMember"));
    }

    /// A cache entry with no source manifest beside it is still a resolvable
    /// dependency — but a source-form entry whose manifest disagrees is not.
    #[test]
    fn source_package_dir_bad_manifest_reports_before_loading_exports() {
        let dir = tempdir().unwrap();
        let pkg_dir = dir.path().join("packages").join("shape");
        fs::create_dir_all(&pkg_dir).unwrap();
        fs::write(
            pkg_dir.join("project.json"),
            "{ \"name\": \"shape\", \"kind\": \"executable\" }",
        )
        .unwrap();
        let cache = crate::manifest::package::source_package_cache_dir(dir.path());
        fs::create_dir_all(&cache).unwrap();
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/rt-behavior/project/project-with-package-import-as/packages/package_import_as.mfp");
        fs::copy(&fixture, cache.join("shape.mfp")).unwrap();
        let manifest = manifest_with_package("shape", Some("file:packages/shape"));
        assert!(resolve_import(dir.path(), &manifest, "shape"));
        assert_eq!(
            crate::rules::code_and_name("IMPORT_PACKAGE_KIND_INVALID"),
            ("2-201-0007", "IMPORT_PACKAGE_KIND_INVALID")
        );
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
