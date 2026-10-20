use super::*;

fn lockfile(present: bool, locked: bool, matches: Option<bool>) -> LockfileSummary {
    LockfileSummary {
        path: "mfb.lock".to_string(),
        present,
        locked,
        parsed: true,
        version: Some(1),
        project_hash_matches: matches,
    }
}

/// A present-but-undecodable lockfile (bug-281).
fn malformed_lockfile(locked: bool) -> LockfileSummary {
    LockfileSummary {
        path: "mfb.lock".to_string(),
        present: true,
        locked,
        parsed: false,
        version: None,
        project_hash_matches: None,
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

/// A present-but-undecodable lockfile is reported, and is an error under
/// `--locked` (bug-281).
///
/// Previously all three decode failures collapsed to
/// `project_hash_matches: None`, which matched no arm — so `mfb audit
/// --locked` printed "Lockfile: present", zero errors, and exited 0 on a
/// lockfile whose hash could not be checked at all. That ranked a
/// cannot-verify lockfile *better* than a stale one.
#[test]
fn malformed_lockfile_is_a_finding_and_errors_under_locked() {
    let dir = std::env::temp_dir();
    let manifest = HashMap::new();
    let ast = ast::AstProject {
        name: "a".to_string(),
        files: Vec::new(),
    };
    let ins = inputs(&dir, &manifest, &ast);

    let mut findings = Vec::new();
    lockfile_findings(&malformed_lockfile(true), &[], &ins, &mut findings);
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].code, "AUDIT-LOCK-MALFORMED");
    assert_eq!(findings[0].severity, Severity::Error);

    // Without --locked it is a warning, matching AUDIT-LOCK-STALE's shape.
    let mut findings = Vec::new();
    lockfile_findings(&malformed_lockfile(false), &[], &ins, &mut findings);
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].code, "AUDIT-LOCK-MALFORMED");
    assert_eq!(findings[0].severity, Severity::Warning);

    // A decodable but stale lockfile still reports STALE, not MALFORMED.
    let mut findings = Vec::new();
    lockfile_findings(&lockfile(true, true, Some(false)), &[], &ins, &mut findings);
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].code, "AUDIT-LOCK-STALE");
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
            resource_type: "fs.File".to_string(),
            close_op: "fs.close".to_string(),
            path: "main.mfb".to_string(),
            line: 3,
            native: false,
            close_may_fail: true,
        },
        ResourceEntry {
            function: "g".to_string(),
            name: "s".to_string(),
            resource_type: "tcp.Socket".to_string(),
            close_op: "tcp.close".to_string(),
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
