//! Runs the execution-free byte-identity gate (`scripts/artifact-gate.sh`) under
//! `cargo test` so the full artifact-golden sweep participates in the normal test
//! run instead of only being reachable through the standalone shell harness.
//!
//! The gate regenerates every deterministic codegen artifact dump and diffs it
//! against its committed golden. It needs the release `mfb` binary (provided by
//! `common::mfb_exe()`, which builds it on demand) and must run with the repo
//! root as its working directory, because the script derives `REPO` from `pwd`.

#[cfg(not(windows))]
mod common;

#[cfg(not(windows))]
use std::path::Path;
#[cfg(not(windows))]
use std::process::Command;

#[cfg(not(windows))]
#[test]
fn artifact_gate_all() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = repo.join("scripts").join("artifact-gate.sh");
    let mfb = common::mfb_exe();

    let status = Command::new("bash")
        .arg(&script)
        .arg(&mfb)
        .arg("all")
        .current_dir(repo)
        .status()
        .expect("spawn scripts/artifact-gate.sh");

    // Exit 98 is the harness's "a rival run holds the lock" refusal, not a gate
    // result: the gate never started, so nothing was checked. Distinguishing it
    // matters because the two are otherwise indistinguishable from here, and the
    // refusal is fast -- a 0.2s "failure" reads like a golden regression and
    // sends the reader looking for a codegen change that does not exist.
    if status.code() == Some(98) {
        panic!(
            "artifact-gate.sh could not START: another gate run holds the lock. \
             This is NOT a golden regression -- nothing was checked. Re-run \
             `cargo test --test golden` once the other run finishes, or run \
             `scripts/artifact-gate.sh <mfb> all` standalone."
        );
    }
    assert!(
        status.success(),
        "artifact-gate.sh reported diffs or failed (exit {:?}); see output above",
        status.code()
    );
}
