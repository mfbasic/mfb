//! Data model for the `mfb audit` report.
//!
//! The report is a plain data structure assembled by [`super::collect`] and
//! rendered deterministically by [`super::text`] and [`super::json`]. Nothing in
//! this module performs IO so the same report can drive both output formats.

#[derive(Clone, Copy, PartialEq, Eq)]
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
