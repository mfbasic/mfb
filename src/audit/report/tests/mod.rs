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
