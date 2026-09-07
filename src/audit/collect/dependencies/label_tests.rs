//! The three `verify` statuses, as the audit report spells them.
//!
//! `verify_status_label` maps `PackageVerifyStatus` to the string that goes into
//! the audit JSON, and only `InvalidPackage` had ever been produced — reaching
//! the other two needs a package that verifies, which needs a registry.
//!
//! **The strings are an interface, not a rendering.** The audit report is read by
//! whatever consumes it, so `"ok"` and `"needs-update"` are values something
//! downstream compares against. Swap them and a report says every current
//! package needs updating; collapse `needs-update` into `invalid` and a stale
//! dependency reads as a corrupt one. Neither shows up as a build failure, and
//! neither is visible in the enum: the mapping is the only place the names
//! exist.
//!
//! Asserted per variant AND as a set, because two variants mapped to the same
//! string satisfies every per-variant check that spells that string.

use super::verify_status_label;
use crate::cli::pkg::PackageVerifyStatus;

#[test]
fn each_verify_status_has_its_own_label() {
    assert_eq!(verify_status_label(PackageVerifyStatus::Ok), "ok");
    assert_eq!(
        verify_status_label(PackageVerifyStatus::NeedsUpdate),
        "needs-update"
    );
    assert_eq!(
        verify_status_label(PackageVerifyStatus::InvalidPackage),
        "invalid"
    );

    let labels = [
        verify_status_label(PackageVerifyStatus::Ok),
        verify_status_label(PackageVerifyStatus::NeedsUpdate),
        verify_status_label(PackageVerifyStatus::InvalidPackage),
    ];
    let distinct: std::collections::BTreeSet<&String> = labels.iter().collect();
    assert_eq!(
        distinct.len(),
        3,
        "the three statuses must be distinguishable in the report; two sharing a \
         label means a reader cannot tell a stale dependency from a corrupt one, \
         and every per-variant assertion above still passes: {labels:?}"
    );
}
