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
                        content_hash = std::fs::read(&package_file)
                            .ok()
                            .and_then(|bytes| {
                                crate::target::package_mfp::package_content_hash(&bytes).ok()
                            })
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

    fn manifest_with_packages(entries: Vec<(&str, &str)>) -> HashMap<String, JsonValue> {
        let packages: Vec<JsonValue> = entries
            .into_iter()
            .map(|(name, version)| {
                let mut map = HashMap::new();
                map.insert("name".to_string(), JsonValue::String(name.to_string()));
                map.insert(
                    "version".to_string(),
                    JsonValue::String(version.to_string()),
                );
                JsonValue::Object(map)
            })
            .collect();
        let mut manifest = HashMap::new();
        manifest.insert("packages".to_string(), JsonValue::Array(packages));
        manifest
    }

    #[test]
    fn verify_status_label_maps_all_variants() {
        assert_eq!(
            verify_status_label(crate::cli::pkg::PackageVerifyStatus::Ok),
            "ok"
        );
        assert_eq!(
            verify_status_label(crate::cli::pkg::PackageVerifyStatus::NeedsUpdate),
            "needs-update"
        );
        assert_eq!(
            verify_status_label(crate::cli::pkg::PackageVerifyStatus::InvalidPackage),
            "invalid"
        );
    }

    #[test]
    fn no_packages_key_yields_empty_lists() {
        let dir = tempdir().unwrap();
        let manifest = HashMap::new();
        assert!(collect_dependencies(dir.path(), &manifest).is_empty());
        assert!(collect_packages(dir.path(), &manifest).is_empty());
    }

    #[test]
    fn declared_but_uninstalled_dependency_is_missing() {
        let dir = tempdir().unwrap();
        let manifest = manifest_with_packages(vec![("alpha", "1.0.0")]);
        let deps = collect_dependencies(dir.path(), &manifest);
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].name, "alpha");
        assert_eq!(deps[0].status, "missing");
        assert!(deps[0].resolved_version.is_none());
        assert!(deps[0].content_hash.is_none());
        assert!(deps[0].signature.is_none());
    }

    #[test]
    fn dependencies_are_sorted_by_name() {
        let dir = tempdir().unwrap();
        let manifest = manifest_with_packages(vec![("zeta", "1.0.0"), ("alpha", "1.0.0")]);
        let deps = collect_dependencies(dir.path(), &manifest);
        assert_eq!(deps[0].name, "alpha");
        assert_eq!(deps[1].name, "zeta");
    }

    #[test]
    fn collect_packages_skips_uninstalled_and_reports_invalid_file() {
        let dir = tempdir().unwrap();
        let manifest = manifest_with_packages(vec![("alpha", "1.0.0"), ("beta", "1.0.0")]);
        // alpha has no file -> skipped entirely. beta has a garbage .mfp -> verifier failed.
        std::fs::create_dir_all(dir.path().join("packages")).unwrap();
        std::fs::write(dir.path().join("packages/beta.mfp"), b"not a real package").unwrap();
        let packages = collect_packages(dir.path(), &manifest);
        assert_eq!(packages.len(), 1);
        assert_eq!(packages[0].name, "beta");
        assert_eq!(packages[0].verifier, "failed");
        assert_eq!(packages[0].signature, "unknown");
        assert_eq!(packages[0].path, "packages/beta.mfp");
    }

    #[test]
    fn collect_dependencies_reports_invalid_for_bad_file() {
        let dir = tempdir().unwrap();
        let manifest = manifest_with_packages(vec![("beta", "1.0.0")]);
        std::fs::create_dir_all(dir.path().join("packages")).unwrap();
        std::fs::write(dir.path().join("packages/beta.mfp"), b"garbage header").unwrap();
        let deps = collect_dependencies(dir.path(), &manifest);
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].status, "invalid");
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
        let content_hash = std::fs::read(&package_file)
            .ok()
            .and_then(|bytes| crate::target::package_mfp::package_content_hash(&bytes).ok())
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
