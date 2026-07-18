//! Data model for the `mfb audit` report.
//!
//! The report is a plain data structure assembled by [`super::collect`] and
//! rendered deterministically by [`super::text`] and [`super::json`]. Nothing in
//! this module performs IO so the same report can drive both output formats.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Severity {
    Error,
    Warning,
    Info,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Info => "info",
        }
    }
}

pub struct AuditReport {
    pub project: ProjectSummary,
    pub lockfile: LockfileSummary,
    pub dependencies: Vec<DependencyEntry>,
    pub packages: Vec<PackageEntry>,
    pub source_flow: Vec<FlowFunction>,
    pub resources: Vec<ResourceEntry>,
    pub native_links: Vec<NativeLinkEntry>,
    /// Native `LINK` resource *types* declared by this package (plan-link-update.md §13).
    pub native_resources: Vec<NativeResourceEntry>,
    pub permissions: Vec<PermissionEntry>,
    pub findings: Vec<Finding>,
}

pub struct ProjectSummary {
    pub name: String,
    pub ident: String,
    pub version: String,
    pub kind: String,
    pub entry: Option<String>,
    pub root: String,
    pub language_version: String,
}

pub struct LockfileSummary {
    pub path: String,
    pub present: bool,
    pub locked: bool,
    /// False when the file exists but could not be read or parsed as a JSON
    /// object (bug-281). Distinguishing this from a merely-stale lockfile is the
    /// point: a hash that *cannot* be checked is strictly worse than one that
    /// does not match, but with only `project_hash_matches: None` to go on it
    /// produced no finding at all and silently satisfied `--locked`.
    pub parsed: bool,
    pub version: Option<i64>,
    pub project_hash_matches: Option<bool>,
}

pub struct DependencyEntry {
    pub name: String,
    pub ident: String,
    pub requested_version: String,
    pub resolved_version: Option<String>,
    pub pin: bool,
    pub source: String,
    pub signature: Option<String>,
    pub content_hash: Option<String>,
    pub status: String,
}

pub struct PackageEntry {
    pub name: String,
    pub version: String,
    pub path: String,
    pub signature: String,
    pub content_hash: String,
    pub verifier: String,
    pub exports: usize,
    pub imports: usize,
    pub cleanups: usize,
}

pub struct FlowFunction {
    pub function: String,
    pub path: String,
    pub line: usize,
    pub fallible: bool,
    pub trap: Option<TrapInfo>,
    pub calls: Vec<CallSite>,
}

pub struct TrapInfo {
    pub name: String,
    pub line: usize,
    pub classification: String,
}

pub struct CallSite {
    pub callee: String,
    pub line: usize,
    pub propagation: String,
    pub capability: Option<String>,
}

pub struct ResourceEntry {
    pub function: String,
    pub name: String,
    pub resource_type: String,
    pub close_op: String,
    pub path: String,
    pub line: usize,
    pub native: bool,
    pub close_may_fail: bool,
}

pub struct NativeLinkEntry {
    pub package: String,
    pub symbol: String,
    pub close_function: String,
    pub may_fail: bool,
}

/// A native `LINK` resource type declaration (plan-link-update.md §13). Reports
/// the declaring package, the close op, whether close may fail, and thread
/// sendability — the same facts a standard resource exposes.
pub struct NativeResourceEntry {
    pub package: String,
    pub resource_type: String,
    pub close_op: String,
    pub close_may_fail: bool,
    pub sendable: bool,
    pub exported: bool,
    pub path: String,
    pub line: usize,
}

pub struct PermissionEntry {
    pub capability: String,
    pub package: String,
    pub function: String,
    pub path: String,
    pub line: usize,
    pub kind: String,
}

pub struct Finding {
    pub code: String,
    pub category: String,
    pub severity: Severity,
    pub message: String,
    pub path: Option<String>,
    pub line: Option<usize>,
    pub package: Option<String>,
}

pub struct Counts {
    pub errors: usize,
    pub warnings: usize,
    pub infos: usize,
}

impl AuditReport {
    pub fn counts(&self) -> Counts {
        let mut counts = Counts {
            errors: 0,
            warnings: 0,
            infos: 0,
        };
        for finding in &self.findings {
            match finding.severity {
                Severity::Error => counts.errors += 1,
                Severity::Warning => counts.warnings += 1,
                Severity::Info => counts.infos += 1,
            }
        }
        counts
    }

    /// Rank used to order findings deterministically by category.
    pub fn category_rank(category: &str) -> u8 {
        match category {
            "lockfile" => 0,
            "dependency" => 1,
            "package" => 2,
            "sourceFlow" => 3,
            "resource" => 4,
            "native" => 5,
            "permission" => 6,
            "lint" => 7,
            "policy" => 8,
            _ => 9,
        }
    }
}

#[cfg(test)]
pub(super) mod testsupport {
    use super::*;

    pub(in crate::audit) fn finding(code: &str, category: &str, severity: Severity) -> Finding {
        Finding {
            code: code.to_string(),
            category: category.to_string(),
            severity,
            message: format!("message for {code}"),
            path: None,
            line: None,
            package: None,
        }
    }

    /// A report exercising every optional field and non-empty section, used by
    /// the text/json renderers to hit their full branch matrix.
    pub(in crate::audit) fn full_report() -> AuditReport {
        AuditReport {
            project: ProjectSummary {
                name: "demo".to_string(),
                ident: "demo.ident".to_string(),
                version: "2.1.0".to_string(),
                kind: "program".to_string(),
                entry: Some("main".to_string()),
                root: "path/to/root".to_string(),
                language_version: "1".to_string(),
            },
            lockfile: LockfileSummary {
                path: "mfb.lock".to_string(),
                present: true,
                parsed: true,
                locked: true,
                version: Some(1),
                project_hash_matches: Some(false),
            },
            dependencies: vec![
                DependencyEntry {
                    name: "alpha".to_string(),
                    ident: "alpha.id".to_string(),
                    requested_version: "1.2.0".to_string(),
                    resolved_version: Some("1.2.3".to_string()),
                    pin: true,
                    source: "registry".to_string(),
                    signature: Some("signed".to_string()),
                    content_hash: Some("abcd".to_string()),
                    status: "ok".to_string(),
                },
                DependencyEntry {
                    name: "beta".to_string(),
                    ident: "beta.id".to_string(),
                    requested_version: String::new(),
                    resolved_version: None,
                    pin: false,
                    source: "path".to_string(),
                    signature: None,
                    content_hash: None,
                    status: "missing".to_string(),
                },
            ],
            packages: vec![PackageEntry {
                name: "alpha".to_string(),
                version: "1.2.3".to_string(),
                path: "packages/alpha.mfp".to_string(),
                signature: "signed".to_string(),
                content_hash: "abcd".to_string(),
                verifier: "ok".to_string(),
                exports: 3,
                imports: 2,
                cleanups: 1,
            }],
            source_flow: vec![
                FlowFunction {
                    function: "doWork".to_string(),
                    path: "main.mfb".to_string(),
                    line: 10,
                    fallible: true,
                    trap: Some(TrapInfo {
                        name: "err".to_string(),
                        line: 15,
                        classification: "recovers".to_string(),
                    }),
                    calls: vec![CallSite {
                        callee: "fs.open".to_string(),
                        line: 12,
                        propagation: "trap".to_string(),
                        capability: Some("filesystem".to_string()),
                    }],
                },
                FlowFunction {
                    function: "pure".to_string(),
                    path: "main.mfb".to_string(),
                    line: 30,
                    fallible: false,
                    trap: None,
                    calls: Vec::new(),
                },
            ],
            resources: vec![
                ResourceEntry {
                    function: "doWork".to_string(),
                    name: "file".to_string(),
                    resource_type: "File".to_string(),
                    close_op: "fs.close".to_string(),
                    path: "main.mfb".to_string(),
                    line: 11,
                    native: false,
                    close_may_fail: true,
                },
                ResourceEntry {
                    function: "doWork".to_string(),
                    name: "handle".to_string(),
                    resource_type: "Native".to_string(),
                    close_op: "pkg.close".to_string(),
                    path: "main.mfb".to_string(),
                    line: 20,
                    native: true,
                    close_may_fail: false,
                },
            ],
            native_links: vec![NativeLinkEntry {
                package: "pkg".to_string(),
                symbol: "sym".to_string(),
                close_function: "closeFn".to_string(),
                may_fail: true,
            }],
            native_resources: vec![
                NativeResourceEntry {
                    package: "pkg".to_string(),
                    resource_type: "Db".to_string(),
                    close_op: "pkg.close".to_string(),
                    close_may_fail: true,
                    sendable: true,
                    exported: true,
                    path: "lib.mfb".to_string(),
                    line: 5,
                },
                NativeResourceEntry {
                    package: "pkg".to_string(),
                    resource_type: "Cursor".to_string(),
                    close_op: "pkg.free".to_string(),
                    close_may_fail: false,
                    sendable: false,
                    exported: false,
                    path: "lib.mfb".to_string(),
                    line: 9,
                },
            ],
            permissions: vec![
                PermissionEntry {
                    capability: "filesystem".to_string(),
                    package: "fs".to_string(),
                    function: "fs.open".to_string(),
                    path: "main.mfb".to_string(),
                    line: 12,
                    kind: "standard".to_string(),
                },
                PermissionEntry {
                    capability: "filesystem".to_string(),
                    package: "fs".to_string(),
                    function: "fs.read".to_string(),
                    path: "main.mfb".to_string(),
                    line: 13,
                    kind: "standard".to_string(),
                },
                PermissionEntry {
                    capability: "terminal".to_string(),
                    package: "io".to_string(),
                    function: "io.print".to_string(),
                    path: "main.mfb".to_string(),
                    line: 14,
                    kind: "standard".to_string(),
                },
            ],
            findings: vec![
                Finding {
                    code: "AUDIT-LOCK-STALE".to_string(),
                    category: "lockfile".to_string(),
                    severity: Severity::Warning,
                    message: "stale lock".to_string(),
                    path: Some("mfb.lock".to_string()),
                    line: None,
                    package: None,
                },
                Finding {
                    code: "AUDIT-RESOURCE-CLOSE-MAY-FAIL".to_string(),
                    category: "resource".to_string(),
                    severity: Severity::Info,
                    message: "resource close may fail".to_string(),
                    path: Some("main.mfb".to_string()),
                    line: Some(11),
                    package: None,
                },
                Finding {
                    code: "AUDIT-DEP-MISSING".to_string(),
                    category: "dependency".to_string(),
                    severity: Severity::Error,
                    message: "dep missing".to_string(),
                    path: None,
                    line: None,
                    package: Some("beta".to_string()),
                },
            ],
        }
    }

    pub(in crate::audit) fn empty_report() -> AuditReport {
        AuditReport {
            project: ProjectSummary {
                name: "demo".to_string(),
                ident: "demo".to_string(),
                version: "1.0.0".to_string(),
                kind: "program".to_string(),
                entry: Some("main".to_string()),
                root: ".".to_string(),
                language_version: "1".to_string(),
            },
            lockfile: LockfileSummary {
                path: "mfb.lock".to_string(),
                present: false,
                parsed: false,
                locked: false,
                version: None,
                project_hash_matches: None,
            },
            dependencies: Vec::new(),
            packages: Vec::new(),
            source_flow: Vec::new(),
            resources: Vec::new(),
            native_links: Vec::new(),
            native_resources: Vec::new(),
            permissions: Vec::new(),
            findings: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testsupport::*;
    use super::*;

    #[test]
    fn severity_as_str_covers_all_variants() {
        assert_eq!(Severity::Error.as_str(), "error");
        assert_eq!(Severity::Warning.as_str(), "warning");
        assert_eq!(Severity::Info.as_str(), "info");
    }

    #[test]
    fn counts_tallies_each_severity() {
        let mut report = empty_report();
        report.findings = vec![
            finding("A", "lockfile", Severity::Error),
            finding("B", "lockfile", Severity::Error),
            finding("C", "dependency", Severity::Warning),
            finding("D", "resource", Severity::Info),
        ];
        let counts = report.counts();
        assert_eq!(counts.errors, 2);
        assert_eq!(counts.warnings, 1);
        assert_eq!(counts.infos, 1);
    }

    #[test]
    fn counts_empty_report_is_zero() {
        let counts = empty_report().counts();
        assert_eq!(counts.errors, 0);
        assert_eq!(counts.warnings, 0);
        assert_eq!(counts.infos, 0);
    }

    #[test]
    fn category_rank_orders_known_and_unknown() {
        assert_eq!(AuditReport::category_rank("lockfile"), 0);
        assert_eq!(AuditReport::category_rank("dependency"), 1);
        assert_eq!(AuditReport::category_rank("package"), 2);
        assert_eq!(AuditReport::category_rank("sourceFlow"), 3);
        assert_eq!(AuditReport::category_rank("resource"), 4);
        assert_eq!(AuditReport::category_rank("native"), 5);
        assert_eq!(AuditReport::category_rank("permission"), 6);
        assert_eq!(AuditReport::category_rank("lint"), 7);
        assert_eq!(AuditReport::category_rank("policy"), 8);
        assert_eq!(AuditReport::category_rank("something-else"), 9);
    }
}
