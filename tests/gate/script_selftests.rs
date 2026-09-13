//! plan-131-E: the shell selftests run under `cargo test`, not only by hand.
//!
//! `scripts/test-accept-selftest.sh` guards the acceptance harness's own machinery,
//! and `scripts/remote-common-selftest.sh` guards the helpers every platform proof
//! sources. A selftest nobody runs is a selftest that has already rotted, so each is
//! spawned here and must exit 0.
//!
//! Unix only. Both need POSIX tooling a Windows runner does not reliably provide:
//! `perl` (the watchdog), `ssh` (the closed-port case), `python3` (the RGBA compare)
//! and `pgrep`/process groups (the acceptance watchdog). The scripts they test are
//! bash harnesses that only ever run on macOS and Linux.

#[cfg(unix)]
fn run_selftest(script: &str) {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let out = std::process::Command::new("bash")
        .arg(root.join("scripts").join(script))
        .current_dir(&root)
        .output()
        .unwrap_or_else(|e| panic!("could not spawn bash for {script}: {e}"));
    assert!(
        out.status.success(),
        "scripts/{script} failed ({})\n--- stdout\n{}\n--- stderr\n{}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[cfg(unix)]
#[test]
fn test_accept_selftest_passes() {
    run_selftest("test-accept-selftest.sh");
}

#[cfg(unix)]
#[test]
fn remote_common_selftest_passes() {
    run_selftest("remote-common-selftest.sh");
}
