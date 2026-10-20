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
    /// The manifest's `libraries` locators (plan-46) and `resources` entries
    /// (plan-55). Both decide what a build pulls in from outside the source
    /// tree, so both belong in the audit (bug-283 A3).
    pub libraries: Vec<LibraryEntry>,
    pub resource_files: Vec<ResourceFileEntry>,
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

/// One declared native-library locator: which logical `LINK` name it satisfies,
/// on which target slot, and which file it binds to (bug-283 A3). A project
/// pointing a benign-looking `LINK "sqlite3"` at a vendored file used to audit
/// identically to one using the system library.
pub struct LibraryEntry {
    pub logical: String,
    pub os: String,
    pub arch: Option<String>,
    pub libc: Option<String>,
    /// `system` or `vendor`.
    pub lib_type: String,
    pub source: String,
}

/// One `resources` manifest entry: a file copied into the build output.
pub struct ResourceFileEntry {
    pub src: String,
    pub dst: String,
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

/// One `RES`/`LET` acquisition of a resource handle, as the audit sees it.
pub struct ResourceEntry {
    pub function: String,
    pub name: String,
    pub resource_type: String,
    pub close_op: String,
    pub path: String,
    pub line: usize,
    /// Always `false` in a real run — and correctly so, not by oversight
    /// (bug-326-A17). `collect::source` only builds a `ResourceEntry` when
    /// `resource_producer` recognizes the callee, and that table lists built-in
    /// producers exclusively (`fs.open`, `net.connectTcp`, `tls.accept`, …). A
    /// native `LINK` resource is reported through `AuditReport::native_resources`
    /// instead, so nothing here is ever native. The field and the `text`/`json`
    /// renderings of it stay because they are part of the audit's committed
    /// output shape, and because they are what a native producer would need if
    /// `resource_producer` ever grew one.
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

/// bug-477: one `tls::connect(..., allowSelfSigned := TRUE)` call site.
///
/// Reported as its own finding rather than folded into the `network`
/// capability, because "this program opens sockets" and "this program will
/// accept a certificate no one vouched for" are different facts and a reviewer
/// needs the second one by itself.
pub struct RelaxedTrustEntry {
    pub function: String,
    pub path: String,
    pub line: usize,
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
    ///
    /// `sourceFlow`, `native`, and `policy` have no rank-consumer in practice:
    /// `audit::collect` emits only dependency, lint, lockfile, package,
    /// permission, and resource findings. They are kept so the ordering is
    /// total and stable if a collector for one of them lands — adding a
    /// category later must not renumber the ones already in use, which is what
    /// would change committed audit output (bug-326-A16). `sourceFlow` is also
    /// a live *report section* name (`audit::json`), just never a finding
    /// category.
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
pub(super) mod testsupport;

#[cfg(test)]
mod tests;
