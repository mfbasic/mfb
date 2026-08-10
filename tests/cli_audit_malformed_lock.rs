//! bug-281: an `mfb.lock` that cannot be read as a JSON object was treated as
//! "absent" — under `--locked` (which demands a verified lockfile) that let a
//! corrupt lockfile pass silently instead of failing. `mfb audit` now emits an
//! `AUDIT-LOCK-MALFORMED` finding: an Error under `--locked`, a Warning otherwise.
//!
//! This drives the real `mfb audit --locked` against a project with a malformed
//! lockfile and asserts the error surfaces (exit non-zero).

use std::process::Command;

mod common;
use common::*;

const SOURCE: &str = "IMPORT io\n\nFUNC main AS Integer\n  io::print(\"hi\")\n  RETURN 0\nEND FUNC\n";

#[test]
fn audit_locked_reports_a_malformed_lockfile_as_an_error() {
    let project = temp_project("bug281_audit_malformed_lock", SOURCE);
    // A lockfile that is not a JSON object.
    std::fs::write(project.join("mfb.lock"), b"this is not valid json {{{\n").expect("write lock");

    let output = Command::new(mfb_exe())
        .args(["audit", "--locked"])
        .arg(&project)
        .output()
        .expect("run mfb audit --locked");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "mfb audit --locked must fail on a malformed lockfile.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stdout.contains("AUDIT-LOCK-MALFORMED"),
        "expected an AUDIT-LOCK-MALFORMED finding, got:\nstdout:\n{stdout}"
    );

    let _ = std::fs::remove_dir_all(&project);
}
