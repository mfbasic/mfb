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
                        signature = Some(crate::cli::pkg::signature_type_name(header.signature_type));
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
