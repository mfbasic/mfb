use super::*;

pub(super) fn collect_dependencies(
    project_dir: &Path,
    manifest: &HashMap<String, JsonValue>,
) -> Vec<DependencyEntry> {
    let Some(packages) = manifest
        .get("packages")
        .and_then(|value| value.get::<Vec<JsonValue>>())
    else {
        return Vec::new();
    };

    let mut entries: Vec<DependencyEntry> = packages
        .iter()
        .filter_map(crate::manifest::package::project_package_dependency)
        .map(|dependency| {
            let package_file = project_dir
                .join("packages")
                .join(format!("{}.mfp", dependency.name));
            let mut resolved_version = None;
            let mut content_hash = None;
            let mut signature = None;
            let status;

            if package_file.is_file() {
                match crate::manifest::package::read_mfp_header(&package_file) {
                    Ok(header) => {
                        resolved_version = Some(header.version.clone());
                        signature =
                            Some(crate::cli::pkg::signature_type_name(header.signature_type));
                        content_hash =
                            crate::target::package_mfp::package_content_hash_file(&package_file)
                                .ok()
                                .map(|hash| crate::cli::pkg::hex_bytes(&hash));
                        status = verify_status_label(crate::cli::pkg::package_dependency_status(
                            &dependency,
                            &header.name,
                            &header.ident,
                            &header.version,
                        ));
                    }
                    Err(_) => status = "invalid".to_string(),
                }
            } else {
                status = "missing".to_string();
            }

            DependencyEntry {
                name: dependency.name,
                ident: dependency.ident,
                requested_version: dependency.version,
                resolved_version,
                pin: dependency.pin,
                source: dependency.source,
                signature,
                content_hash,
                status,
            }
        })
        .collect();

    entries.sort_by(|a, b| a.name.cmp(&b.name));
    entries
}

fn verify_status_label(status: crate::cli::pkg::PackageVerifyStatus) -> String {
    match status {
        crate::cli::pkg::PackageVerifyStatus::Ok => "ok",
        crate::cli::pkg::PackageVerifyStatus::NeedsUpdate => "needs-update",
        crate::cli::pkg::PackageVerifyStatus::InvalidPackage => "invalid",
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    /// Absolute path to a real, valid `.mfp` fixture in the repository.
    fn valid_mfp_fixture() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/rt-behavior/project/project-with-package-import-as/packages/package_import_as.mfp")
    }

    /// Build a manifest declaring one package named `name`.
    fn manifest_with(name: &str) -> HashMap<String, JsonValue> {
        let json = format!(
            "{{ \"packages\": [ {{ \"name\": \"{name}\", \"version\": \"1.0.0\", \"pin\": true, \"source\": \"file:x\" }} ] }}"
        );
        json.parse::<JsonValue>()
            .unwrap()
            .get::<HashMap<String, JsonValue>>()
            .unwrap()
            .clone()
    }

    #[test]
    fn no_packages_key_yields_empty() {
        let dir = tempdir().unwrap();
        assert!(collect_dependencies(dir.path(), &HashMap::new()).is_empty());
        assert!(collect_packages(dir.path(), &HashMap::new()).is_empty());
    }

    #[test]
    fn missing_mfp_is_status_missing() {
        let dir = tempdir().unwrap();
        let manifest = manifest_with("shape");
        let deps = collect_dependencies(dir.path(), &manifest);
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].status, "missing");
        // collect_packages skips a missing file entirely.
        assert!(collect_packages(dir.path(), &manifest).is_empty());
    }

    #[test]
    fn garbage_mfp_is_status_invalid_and_verifier_failed() {
        let dir = tempdir().unwrap();
        let packages = dir.path().join("packages");
        std::fs::create_dir_all(&packages).unwrap();
        std::fs::write(packages.join("shape.mfp"), b"not a package").unwrap();
        let manifest = manifest_with("shape");

        let deps = collect_dependencies(dir.path(), &manifest);
        assert_eq!(deps[0].status, "invalid");

        let pkgs = collect_packages(dir.path(), &manifest);
        assert_eq!(pkgs.len(), 1);
        assert_eq!(pkgs[0].verifier, "failed");
        assert_eq!(pkgs[0].signature, "unknown");
    }

    #[test]
    fn valid_mfp_reads_header_and_info() {
        let fixture = valid_mfp_fixture();
        if !fixture.exists() {
            return;
        }
        let dir = tempdir().unwrap();
        let packages = dir.path().join("packages");
        std::fs::create_dir_all(&packages).unwrap();
        // Install the fixture under the declared name so the header/content
        // read paths execute end-to-end.
        std::fs::copy(&fixture, packages.join("shape.mfp")).unwrap();
        let manifest = manifest_with("shape");

        let deps = collect_dependencies(dir.path(), &manifest);
        assert_eq!(deps.len(), 1);
        // A readable header populates the resolved version, signature and hash.
        assert!(deps[0].resolved_version.is_some());
        assert!(deps[0].signature.is_some());
        assert!(deps[0].content_hash.is_some());

        let pkgs = collect_packages(dir.path(), &manifest);
        assert_eq!(pkgs.len(), 1);
        assert_eq!(pkgs[0].verifier, "ok");
        assert!(!pkgs[0].content_hash.is_empty());
    }
}

pub(super) fn collect_packages(
    project_dir: &Path,
    manifest: &HashMap<String, JsonValue>,
) -> Vec<PackageEntry> {
    let Some(packages) = manifest
        .get("packages")
        .and_then(|value| value.get::<Vec<JsonValue>>())
    else {
        return Vec::new();
    };

    let mut entries = Vec::new();
    for dependency in packages
        .iter()
        .filter_map(crate::manifest::package::project_package_dependency)
    {
        let package_file = project_dir
            .join("packages")
            .join(format!("{}.mfp", dependency.name));
        if !package_file.is_file() {
            continue;
        }
        let display = format!("packages/{}.mfp", dependency.name);
        let header = crate::manifest::package::read_mfp_header(&package_file);
        let info = crate::binary_repr::read_package_info(&package_file);
        let content_hash = crate::target::package_mfp::package_content_hash_file(&package_file)
            .ok()
            .map(|hash| crate::cli::pkg::hex_bytes(&hash))
            .unwrap_or_default();

        match (header, info) {
            (Ok(header), Ok(info)) => entries.push(PackageEntry {
                name: header.name.clone(),
                version: header.version.clone(),
                path: display,
                signature: crate::cli::pkg::signature_type_name(header.signature_type),
                content_hash,
                verifier: "ok".to_string(),
                exports: info.export_count,
                imports: info.import_count,
                cleanups: info.cleanup_count,
            }),
            _ => entries.push(PackageEntry {
                name: dependency.name.clone(),
                version: String::new(),
                path: display,
                signature: "unknown".to_string(),
                content_hash,
                verifier: "failed".to_string(),
                exports: 0,
                imports: 0,
                cleanups: 0,
            }),
        }
    }

    entries.sort_by(|a, b| a.name.cmp(&b.name));
    entries
}
