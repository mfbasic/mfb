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

    fn lockfile(present: bool, locked: bool, matches: Option<bool>) -> LockfileSummary {
        LockfileSummary {
            path: "mfb.lock".to_string(),
            present,
            locked,
            version: Some(1),
            project_hash_matches: matches,
        }
    }

    fn inputs<'a>(
        dir: &'a Path,
        manifest: &'a HashMap<String, JsonValue>,
        ast: &'a ast::AstProject,
    ) -> AuditInputs<'a> {
        AuditInputs {
            project_dir: dir,
            root_display: ".".to_string(),
            manifest,
            ast,
            kind: "executable".to_string(),
            entry: None,
            locked: false,
        }
    }

    fn dependency(name: &str, status: &str) -> DependencyEntry {
        DependencyEntry {
            name: name.to_string(),
            ident: format!("std#{name}"),
            requested_version: "1.0.0".to_string(),
            resolved_version: None,
            pin: true,
            source: "file:x".to_string(),
            signature: None,
            content_hash: None,
            status: status.to_string(),
        }
    }

    fn package_entry(name: &str, verifier: &str, signature: &str) -> PackageEntry {
        PackageEntry {
            name: name.to_string(),
            version: "1.0.0".to_string(),
            path: format!("packages/{name}.mfp"),
            signature: signature.to_string(),
            content_hash: String::new(),
            verifier: verifier.to_string(),
            exports: 0,
            imports: 0,
            cleanups: 0,
        }
    }

    #[test]
    fn lock_missing_when_required() {
        let dir = std::path::PathBuf::from(".");
        let manifest = HashMap::new();
        let ast = ast::AstProject {
            name: "a".to_string(),
            files: Vec::new(),
        };
        let ins = inputs(&dir, &manifest, &ast);
        let mut findings = Vec::new();
        lockfile_findings(&lockfile(false, true, None), &[], &ins, &mut findings);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].code, "AUDIT-LOCK-MISSING");
        assert!(findings[0].severity == Severity::Error);
    }

    #[test]
    fn lock_stale_is_error_when_locked_warning_otherwise() {
        let dir = std::path::PathBuf::from(".");
        let manifest = HashMap::new();
        let ast = ast::AstProject {
            name: "a".to_string(),
            files: Vec::new(),
        };
        let ins = inputs(&dir, &manifest, &ast);

        let mut findings = Vec::new();
        lockfile_findings(&lockfile(true, true, Some(false)), &[], &ins, &mut findings);
        assert_eq!(findings[0].code, "AUDIT-LOCK-STALE");
        assert!(findings[0].severity == Severity::Error);

        let mut findings = Vec::new();
        lockfile_findings(
            &lockfile(true, false, Some(false)),
            &[],
            &ins,
            &mut findings,
        );
        assert_eq!(findings[0].code, "AUDIT-LOCK-STALE");
        assert!(findings[0].severity == Severity::Warning);
    }

    #[test]
    fn lock_matching_present_yields_no_finding() {
        let dir = std::path::PathBuf::from(".");
        let manifest = HashMap::new();
        let ast = ast::AstProject {
            name: "a".to_string(),
            files: Vec::new(),
        };
        let ins = inputs(&dir, &manifest, &ast);
        let mut findings = Vec::new();
        lockfile_findings(&lockfile(true, false, Some(true)), &[], &ins, &mut findings);
        assert!(findings.is_empty());
    }

    #[test]
    fn dependency_findings_cover_each_status() {
        let deps = vec![
            dependency("a", "missing"),
            dependency("b", "invalid"),
            dependency("c", "needs-update"),
            dependency("d", "ok"),
        ];
        let mut findings = Vec::new();
        dependency_findings(&deps, &mut findings);
        let codes: Vec<&str> = findings.iter().map(|f| f.code.as_str()).collect();
        assert!(codes.contains(&"AUDIT-DEP-MISSING"));
        assert!(codes.contains(&"AUDIT-DEP-INVALID"));
        assert!(codes.contains(&"AUDIT-DEP-OUTDATED"));
        // "ok" yields nothing.
        assert_eq!(findings.len(), 3);
    }

    #[test]
    fn package_findings_flag_failed_and_unsigned() {
        let dir = std::path::PathBuf::from(".");
        let manifest = HashMap::new();
        let packages = vec![
            package_entry("bad", "failed", "unknown"),
            package_entry("plain", "ok", "unsigned"),
            package_entry("good", "ok", "ed25519"),
        ];
        let mut findings = Vec::new();
        package_findings(&dir, &manifest, &packages, &mut findings);
        let codes: Vec<&str> = findings.iter().map(|f| f.code.as_str()).collect();
        assert!(codes.contains(&"AUDIT-PKG-VERIFY-FAILED"));
        assert!(codes.contains(&"AUDIT-PKG-UNSIGNED"));
        // The verified, signed package produces neither.
        assert_eq!(findings.len(), 2);
    }

    #[test]
    fn package_findings_flags_exported_mutable_state_from_mfp() {
        // Drives the second half of `package_findings`: it re-reads the declared
        // package's `.mfp` info and scans its exported globals for mutable state.
        // The `package-state-audit` golden `.mfp` exports a mutable `counter`, so
        // this exercises the AUDIT-PKG-STATE-EXPORTED-MUT emit path end-to-end.
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/syntax/packages/package-state-audit/golden/package_state_audit.mfp");
        if !fixture.exists() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let packages_dir = dir.path().join("packages");
        std::fs::create_dir_all(&packages_dir).unwrap();
        std::fs::copy(&fixture, packages_dir.join("package_state_audit.mfp")).unwrap();
        let manifest = "{ \"packages\": [ { \"name\": \"package_state_audit\" } ] }"
            .parse::<JsonValue>()
            .unwrap()
            .get::<HashMap<String, JsonValue>>()
            .unwrap()
            .clone();

        let mut findings = Vec::new();
        // No PackageEntry rows: exercises only the info-reading loop.
        package_findings(dir.path(), &manifest, &[], &mut findings);
        assert!(findings
            .iter()
            .any(|f| f.code == "AUDIT-PKG-STATE-EXPORTED-MUT"));
    }

    #[test]
    fn resource_findings_flag_lexical_close() {
        let resources = vec![
            ResourceEntry {
                function: "f".to_string(),
                name: "h".to_string(),
                resource_type: "File".to_string(),
                close_op: "fs.close".to_string(),
                path: "main.mfb".to_string(),
                line: 3,
                native: false,
                close_may_fail: true,
            },
            ResourceEntry {
                function: "g".to_string(),
                name: "s".to_string(),
                resource_type: "Socket".to_string(),
                close_op: "net.close".to_string(),
                path: "main.mfb".to_string(),
                line: 9,
                native: false,
                close_may_fail: false,
            },
        ];
        let mut findings = Vec::new();
        resource_findings(&resources, &mut findings);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].code, "AUDIT-RESOURCE-CLOSE-MAY-FAIL");
        assert_eq!(findings[0].line, Some(3));
    }

    fn permission(capability: &str) -> PermissionEntry {
        PermissionEntry {
            capability: capability.to_string(),
            package: capability.to_string(),
            function: "f".to_string(),
            path: "main.mfb".to_string(),
            line: 1,
            kind: "standard".to_string(),
        }
    }

    #[test]
    fn permission_findings_map_each_capability_and_dedup() {
        let permissions = vec![
            permission("filesystem"),
            permission("filesystem"), // duplicate collapses
            permission("network"),
            permission("terminal"),
            permission("threads"),
            permission("process"),
            permission("environment"),
            permission("clock"),
            permission("randomness"),
            permission("native"),
            permission("mystery"), // maps to AUDIT-PERM-OTHER
        ];
        let mut findings = Vec::new();
        permission_findings(&permissions, &mut findings);
        let codes: Vec<&str> = findings.iter().map(|f| f.code.as_str()).collect();
        for expected in [
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
        ] {
            assert!(codes.contains(&expected), "missing {expected}");
        }
        // Ten distinct capabilities → ten findings (the duplicate collapsed).
        assert_eq!(findings.len(), 10);
    }

    #[test]
    fn sort_findings_orders_by_category_then_code() {
        let mut findings = vec![
            Finding {
                code: "AUDIT-PERM-NETWORK".to_string(),
                category: "permission".to_string(),
                severity: Severity::Info,
                message: "b".to_string(),
                path: None,
                line: None,
                package: None,
            },
            Finding {
                code: "AUDIT-LOCK-STALE".to_string(),
                category: "lockfile".to_string(),
                severity: Severity::Warning,
                message: "a".to_string(),
                path: None,
                line: None,
                package: None,
            },
        ];
        sort_findings(&mut findings);
        // lockfile (rank 0) sorts before permission (rank 6).
        assert_eq!(findings[0].category, "lockfile");
    }
}
