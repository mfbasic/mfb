//! bug-29: `mfb init` wrote its scaffold files (`project.json`, `src/main.mfb`)
//! with a plain `fs::write` guarded only by an `exists()` check — a TOCTOU that
//! followed a symlink planted at the target onto its victim, clobbering it (and a
//! *dangling* symlink slipped past `exists()` entirely). `write_new_file` now uses
//! `create_new` (`O_EXCL`), which never follows a symlink and refuses any existing
//! target.
//!
//! This drives the real `mfb` binary against a symlinked `project.json` and
//! asserts init refuses and leaves the victim file untouched.

#![cfg(unix)]

use std::process::Command;

mod common;
use common::*;

#[test]
fn init_refuses_to_follow_a_dangling_symlinked_target() {
    // A *dangling* symlink is the case that distinguishes the fix: the pre-fix
    // `exists()` guard returns false for it (nothing there), so the follow-up
    // `fs::write` followed the symlink and CREATED its victim target. `create_new`
    // (O_EXCL) refuses the symlink outright.
    let dir = tempfile::tempdir().expect("temp dir");
    let victim = dir.path().join("victim-target"); // does NOT exist yet
    let project = dir.path().join("app");
    std::fs::create_dir_all(&project).expect("mkdir app");
    std::os::unix::fs::symlink(&victim, project.join("project.json")).expect("symlink");

    let output = Command::new(mfb_exe())
        .arg("init")
        .arg(&project)
        .output()
        .expect("run mfb init");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "mfb init must refuse a dangling-symlinked target, not succeed.\nstderr:\n{stderr}"
    );
    assert!(
        stderr.contains("refusing to overwrite"),
        "expected a refusing-to-overwrite error, got:\n{stderr}"
    );
    // The dangling symlink must NOT have been followed to create the victim.
    assert!(
        !victim.exists(),
        "the dangling symlink was followed and its target was created (bug-29 regressed)"
    );
}
