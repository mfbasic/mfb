//! bug-398: the compiler-side decode of untrusted build files (`project.json`,
//! `mfb.lock`, dependency manifests) went through `tinyjson`'s recursive-descent
//! parser with no depth limit, so a deeply nested document overflowed the native
//! thread stack and aborted `mfb` (SIGABRT) before any validation ran — a DoS at
//! the package trust boundary.
//!
//! These tests drive the real `mfb` binary against pathologically nested build
//! files and assert it exits with a *bounded* error (a normal non-zero exit
//! code), never dies by signal.

use std::os::unix::process::ExitStatusExt;
use std::process::Command;

mod common;
use common::*;

/// Nesting depth that reliably overflows the recursive `tinyjson` parser; the
/// documented reproduction used the same shape.
const OVERFLOW_DEPTH: usize = 120_000;

fn deeply_nested_json() -> String {
    "[".repeat(OVERFLOW_DEPTH) + &"]".repeat(OVERFLOW_DEPTH)
}

/// A process aborted by `stack overflow` is killed by a signal, so its
/// `ExitStatus` carries no exit code. A cleanly-reported error exits normally.
fn assert_bounded_not_aborted(output: &std::process::Output, context: &str) {
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.code().is_some(),
        "{context}: mfb was killed by signal {:?} (stack overflow?) instead of \
         reporting a bounded error.\nstderr:\n{stderr}",
        output.status.signal()
    );
    assert!(
        !stderr.contains("stack overflow"),
        "{context}: mfb reported a stack overflow.\nstderr:\n{stderr}"
    );
    assert!(
        !output.status.success(),
        "{context}: a malformed manifest must still be an error, not success"
    );
}

#[test]
fn pkg_verify_rejects_deeply_nested_project_json() {
    let project = tempfile::tempdir().unwrap();
    std::fs::write(project.path().join("project.json"), deeply_nested_json()).unwrap();

    let output = Command::new(mfb_exe())
        .args(["pkg", "verify"])
        .current_dir(project.path())
        .output()
        .expect("run mfb pkg verify");

    assert_bounded_not_aborted(&output, "project.json");
}

#[test]
fn pkg_install_rejects_deeply_nested_mfb_lock() {
    let workdir = tempfile::tempdir().unwrap();
    let project = workdir.path().join("app");

    // `init` writes a fully-valid project.json so decode advances past manifest
    // validation and reaches the lockfile read in `pkg install`.
    let init = Command::new(mfb_exe())
        .args(["init", project.to_str().unwrap()])
        .output()
        .expect("run mfb init");
    assert!(
        init.status.success(),
        "init failed: {}",
        String::from_utf8_lossy(&init.stderr)
    );
    std::fs::write(project.join("mfb.lock"), deeply_nested_json()).unwrap();

    let output = Command::new(mfb_exe())
        .args(["pkg", "install"])
        .current_dir(&project)
        .output()
        .expect("run mfb pkg install");

    assert_bounded_not_aborted(&output, "mfb.lock");
}
