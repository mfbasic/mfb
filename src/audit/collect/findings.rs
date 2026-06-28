use super::*;

pub(super) fn lockfile_findings(
    lockfile: &LockfileSummary,
    _dependencies: &[DependencyEntry],
    inputs: &AuditInputs,
    findings: &mut Vec<Finding>,
) {
    if lockfile.locked && !lockfile.present {
        findings.push(Finding {
            code: "AUDIT-LOCK-MISSING".to_string(),
            category: "lockfile".to_string(),
            severity: Severity::Error,
            message: "mfb.lock is required by --locked but was not found".to_string(),
            path: Some(lockfile.path.clone()),
            line: None,
            package: None,
        });
        return;
    }

    if lockfile.present && lockfile.project_hash_matches == Some(false) {
        let severity = if lockfile.locked {
            Severity::Error
        } else {
            Severity::Warning
        };
        findings.push(Finding {
            code: "AUDIT-LOCK-STALE".to_string(),
            category: "lockfile".to_string(),
            severity,
            message: "mfb.lock projectHash does not match project.json packages".to_string(),
            path: Some(lockfile.path.clone()),
            line: None,
            package: None,
        });
    }

    let _ = inputs;
}

pub(super) fn dependency_findings(dependencies: &[DependencyEntry], findings: &mut Vec<Finding>) {
    for dependency in dependencies {
        match dependency.status.as_str() {
            "missing" => findings.push(Finding {
                code: "AUDIT-DEP-MISSING".to_string(),
                category: "dependency".to_string(),
                severity: Severity::Error,
                message: format!(
                    "declared package `{}` is not installed under packages/",
                    dependency.name
                ),
                path: None,
                line: None,
                package: Some(dependency.name.clone()),
            }),
            "invalid" => findings.push(Finding {
                code: "AUDIT-DEP-INVALID".to_string(),
                category: "dependency".to_string(),
                severity: Severity::Error,
                message: format!("package `{}` is invalid or unreadable", dependency.name),
                path: None,
                line: None,
                package: Some(dependency.name.clone()),
            }),
            "needs-update" => findings.push(Finding {
                code: "AUDIT-DEP-OUTDATED".to_string(),
                category: "dependency".to_string(),
                severity: Severity::Warning,
                message: format!(
                    "package `{}` does not satisfy requested version {}",
                    dependency.name, dependency.requested_version
                ),
                path: None,
                line: None,
                package: Some(dependency.name.clone()),
            }),
            _ => {}
        }
    }
}

pub(super) fn package_findings(
    project_dir: &Path,
    manifest: &HashMap<String, JsonValue>,
    packages: &[PackageEntry],
    findings: &mut Vec<Finding>,
) {
    for package in packages {
        if package.verifier == "failed" {
            findings.push(Finding {
                code: "AUDIT-PKG-VERIFY-FAILED".to_string(),
                category: "package".to_string(),
                severity: Severity::Error,
                message: format!("package `{}` failed to verify", package.name),
                path: Some(package.path.clone()),
                line: None,
                package: Some(package.name.clone()),
            });
            continue;
        }
        if package.signature == "unsigned" {
            findings.push(Finding {
                code: "AUDIT-PKG-UNSIGNED".to_string(),
                category: "package".to_string(),
                severity: Severity::Info,
                message: format!("package `{}` is unsigned", package.name),
                path: Some(package.path.clone()),
                line: None,
                package: Some(package.name.clone()),
            });
        }
    }

    // Exported mutable state and secondary-close cleanup metadata come from the
    // detailed package info, which we re-read for the audit notes.
    let Some(declared) = manifest
        .get("packages")
        .and_then(|value| value.get::<Vec<JsonValue>>())
    else {
        return;
    };
    for dependency in declared
        .iter()
        .filter_map(crate::manifest::package::project_package_dependency)
    {
        let package_file = project_dir
            .join("packages")
            .join(format!("{}.mfp", dependency.name));
        let Ok(info) = crate::binary_repr::read_package_info(&package_file) else {
            continue;
        };
        let display = format!("packages/{}.mfp", dependency.name);
        for global in &info.globals {
            if global.mutable && global.visibility == "export" {
                findings.push(Finding {
                    code: "AUDIT-PKG-STATE-EXPORTED-MUT".to_string(),
                    category: "package".to_string(),
                    severity: Severity::Warning,
                    message: format!(
                        "package `{}` exports mutable state `{}`",
                        info.manifest_name, global.name
                    ),
                    path: Some(display.clone()),
                    line: None,
                    package: Some(info.manifest_name.clone()),
                });
            }
        }
        for cleanup in &info.cleanups {
            if cleanup.records_secondary_close_failure {
                findings.push(Finding {
                    code: "AUDIT-RESOURCE-SECONDARY-CLOSE".to_string(),
                    category: "resource".to_string(),
                    severity: Severity::Info,
                    message: format!(
                        "package `{}` cleanup in `{}` records secondary close failures",
                        info.manifest_name, cleanup.function
                    ),
                    path: Some(display.clone()),
                    line: None,
                    package: Some(info.manifest_name.clone()),
                });
            }
        }
    }
}

pub(super) fn resource_findings(resources: &[ResourceEntry], findings: &mut Vec<Finding>) {
    for resource in resources {
        if resource.close_may_fail {
            findings.push(Finding {
                code: "AUDIT-RESOURCE-CLOSE-MAY-FAIL".to_string(),
                category: "resource".to_string(),
                severity: Severity::Info,
                message: format!(
                    "resource `{}` ({}) is closed by lexical drop; explicit `{}` is required to observe a close failure",
                    resource.name, resource.resource_type, resource.close_op
                ),
                path: Some(resource.path.clone()),
                line: Some(resource.line),
                package: None,
            });
        }
    }
}

pub(super) fn permission_findings(permissions: &[PermissionEntry], findings: &mut Vec<Finding>) {
    let mut seen = HashSet::new();
    for permission in permissions {
        if !seen.insert(permission.capability.clone()) {
            continue;
        }
        let code = match permission.capability.as_str() {
            "filesystem" => "AUDIT-PERM-FILESYSTEM",
            "network" => "AUDIT-PERM-NETWORK",
            "terminal" => "AUDIT-PERM-TERMINAL",
            "threads" => "AUDIT-PERM-THREADS",
            "process" => "AUDIT-PERM-PROCESS",
            "environment" => "AUDIT-PERM-ENVIRONMENT",
            "clock" => "AUDIT-PERM-CLOCK",
            "randomness" => "AUDIT-PERM-RANDOMNESS",
            "native" => "AUDIT-PERM-NATIVE",
            _ => "AUDIT-PERM-OTHER",
        };
        findings.push(Finding {
            code: code.to_string(),
            category: "permission".to_string(),
            severity: Severity::Info,
            message: format!("project uses host capability: {}", permission.capability),
            path: None,
            line: None,
            package: None,
        });
    }
}

pub(super) fn sort_findings(findings: &mut [Finding]) {
    findings.sort_by(|a, b| {
        AuditReport::category_rank(&a.category)
            .cmp(&AuditReport::category_rank(&b.category))
            .then(a.code.cmp(&b.code))
            .then(a.path.cmp(&b.path))
            .then(a.line.cmp(&b.line))
            .then(a.message.cmp(&b.message))
    });
}
