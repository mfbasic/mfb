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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn lockfile(present: bool, locked: bool, matches: Option<bool>) -> LockfileSummary {
        LockfileSummary {
            path: "mfb.lock".to_string(),
            present,
            locked,
            version: None,
            project_hash_matches: matches,
        }
    }

    fn inputs<'a>(
        ast: &'a ast::AstProject,
        manifest: &'a HashMap<String, JsonValue>,
    ) -> AuditInputs<'a> {
        AuditInputs {
            project_dir: Path::new("."),
            root_display: ".".to_string(),
            manifest,
            ast,
            kind: "program".to_string(),
            entry: None,
            locked: false,
        }
    }

    fn dependency(name: &str, status: &str, requested: &str) -> DependencyEntry {
        DependencyEntry {
            name: name.to_string(),
            ident: name.to_string(),
            requested_version: requested.to_string(),
            resolved_version: None,
            pin: false,
            source: "registry".to_string(),
            signature: None,
            content_hash: None,
            status: status.to_string(),
        }
    }

    fn empty_ast() -> ast::AstProject {
        ast::AstProject {
            name: "demo".to_string(),
            files: Vec::new(),
        }
    }

    #[test]
    fn locked_but_missing_lockfile_is_error() {
        let ast = empty_ast();
        let manifest = HashMap::new();
        let ins = inputs(&ast, &manifest);
        let mut findings = Vec::new();
        lockfile_findings(&lockfile(false, true, None), &[], &ins, &mut findings);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].code, "AUDIT-LOCK-MISSING");
        assert!(findings[0].severity == Severity::Error);
        assert_eq!(findings[0].path.as_deref(), Some("mfb.lock"));
    }

    #[test]
    fn stale_lock_is_warning_unlocked_error_locked() {
        let ast = empty_ast();
        let manifest = HashMap::new();
        let ins = inputs(&ast, &manifest);

        let mut warn = Vec::new();
        lockfile_findings(&lockfile(true, false, Some(false)), &[], &ins, &mut warn);
        assert_eq!(warn[0].code, "AUDIT-LOCK-STALE");
        assert!(warn[0].severity == Severity::Warning);

        let mut err = Vec::new();
        lockfile_findings(&lockfile(true, true, Some(false)), &[], &ins, &mut err);
        assert!(err[0].severity == Severity::Error);
    }

    #[test]
    fn matching_or_absent_lock_yields_no_findings() {
        let ast = empty_ast();
        let manifest = HashMap::new();
        let ins = inputs(&ast, &manifest);
        let mut findings = Vec::new();
        lockfile_findings(&lockfile(true, false, Some(true)), &[], &ins, &mut findings);
        lockfile_findings(&lockfile(false, false, None), &[], &ins, &mut findings);
        assert!(findings.is_empty());
    }

    #[test]
    fn dependency_findings_map_each_status() {
        let deps = vec![
            dependency("a", "missing", ""),
            dependency("b", "invalid", ""),
            dependency("c", "needs-update", "1.2.0"),
            dependency("d", "ok", "1.0.0"),
        ];
        let mut findings = Vec::new();
        dependency_findings(&deps, &mut findings);
        let codes: Vec<&str> = findings.iter().map(|f| f.code.as_str()).collect();
        assert_eq!(
            codes,
            vec![
                "AUDIT-DEP-MISSING",
                "AUDIT-DEP-INVALID",
                "AUDIT-DEP-OUTDATED"
            ]
        );
        assert!(findings[0].severity == Severity::Error);
        assert!(findings[1].severity == Severity::Error);
        assert!(findings[2].severity == Severity::Warning);
        assert!(findings[2].message.contains("1.2.0"));
    }

    #[test]
    fn package_findings_report_failed_and_unsigned() {
        let packages = vec![
            PackageEntry {
                name: "broken".to_string(),
                version: String::new(),
                path: "packages/broken.mfp".to_string(),
                signature: "unknown".to_string(),
                content_hash: String::new(),
                verifier: "failed".to_string(),
                exports: 0,
                imports: 0,
                cleanups: 0,
            },
            PackageEntry {
                name: "bare".to_string(),
                version: "1.0.0".to_string(),
                path: "packages/bare.mfp".to_string(),
                signature: "unsigned".to_string(),
                content_hash: "hash".to_string(),
                verifier: "ok".to_string(),
                exports: 1,
                imports: 0,
                cleanups: 0,
            },
        ];
        let manifest = HashMap::new();
        let mut findings = Vec::new();
        package_findings(Path::new("."), &manifest, &packages, &mut findings);
        let codes: Vec<&str> = findings.iter().map(|f| f.code.as_str()).collect();
        assert!(codes.contains(&"AUDIT-PKG-VERIFY-FAILED"));
        assert!(codes.contains(&"AUDIT-PKG-UNSIGNED"));
        // failed package short-circuits before the unsigned check
        assert_eq!(
            findings
                .iter()
                .filter(|f| f.package.as_deref() == Some("broken"))
                .count(),
            1
        );
    }

    #[test]
    fn package_findings_signed_ok_package_has_no_findings() {
        let packages = vec![PackageEntry {
            name: "good".to_string(),
            version: "1.0.0".to_string(),
            path: "packages/good.mfp".to_string(),
            signature: "signed".to_string(),
            content_hash: "hash".to_string(),
            verifier: "ok".to_string(),
            exports: 1,
            imports: 0,
            cleanups: 0,
        }];
        let manifest = HashMap::new();
        let mut findings = Vec::new();
        package_findings(Path::new("."), &manifest, &packages, &mut findings);
        assert!(findings.is_empty());
    }

    #[test]
    fn resource_findings_only_flag_close_may_fail() {
        let resources = vec![
            ResourceEntry {
                function: "f".to_string(),
                name: "file".to_string(),
                resource_type: "File".to_string(),
                close_op: "fs.close".to_string(),
                path: "main.mfb".to_string(),
                line: 3,
                native: false,
                close_may_fail: true,
            },
            ResourceEntry {
                function: "f".to_string(),
                name: "handle".to_string(),
                resource_type: "Native".to_string(),
                close_op: "pkg.close".to_string(),
                path: "main.mfb".to_string(),
                line: 4,
                native: true,
                close_may_fail: false,
            },
        ];
        let mut findings = Vec::new();
        resource_findings(&resources, &mut findings);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].code, "AUDIT-RESOURCE-CLOSE-MAY-FAIL");
        assert_eq!(findings[0].line, Some(3));
        assert!(findings[0].message.contains("fs.close"));
    }

    #[test]
    fn permission_findings_dedup_by_capability_and_map_codes() {
        let permission = |cap: &str| PermissionEntry {
            capability: cap.to_string(),
            package: "pkg".to_string(),
            function: "f".to_string(),
            path: "main.mfb".to_string(),
            line: 1,
            kind: "standard".to_string(),
        };
        let permissions = vec![
            permission("filesystem"),
            permission("filesystem"),
            permission("network"),
            permission("terminal"),
            permission("threads"),
            permission("process"),
            permission("environment"),
            permission("clock"),
            permission("randomness"),
            permission("native"),
            permission("weird-cap"),
        ];
        let mut findings = Vec::new();
        permission_findings(&permissions, &mut findings);
        let codes: Vec<&str> = findings.iter().map(|f| f.code.as_str()).collect();
        assert_eq!(
            codes,
            vec![
                "AUDIT-PERM-FILESYSTEM",
                "AUDIT-PERM-NETWORK",
                "AUDIT-PERM-TERMINAL",
                "AUDIT-PERM-THREADS",
                "AUDIT-PERM-PROCESS",
                "AUDIT-PERM-ENVIRONMENT",
                "AUDIT-PERM-CLOCK",
                "AUDIT-PERM-RANDOMNESS",
                "AUDIT-PERM-NATIVE",
                "AUDIT-PERM-OTHER",
            ]
        );
        assert!(findings.iter().all(|f| f.severity == Severity::Info));
    }

    #[test]
    fn sort_findings_orders_by_category_then_code() {
        let mk = |code: &str, category: &str| Finding {
            code: code.to_string(),
            category: category.to_string(),
            severity: Severity::Info,
            message: String::new(),
            path: None,
            line: None,
            package: None,
        };
        let mut findings = vec![
            mk("Z", "resource"),
            mk("B", "lockfile"),
            mk("A", "lockfile"),
            mk("M", "dependency"),
        ];
        sort_findings(&mut findings);
        let order: Vec<&str> = findings.iter().map(|f| f.code.as_str()).collect();
        assert_eq!(order, vec!["A", "B", "M", "Z"]);
    }
}
