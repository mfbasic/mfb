//! Runs the execution-free byte-identity gate (`scripts/artifact-gate.sh`) under
//! `cargo test` so the full artifact-golden sweep participates in the normal test
//! run instead of only being reachable through the standalone shell harness.
//!
//! The gate regenerates every deterministic codegen artifact dump and diffs it
//! against its committed golden. It needs the release `mfb` binary (provided by
//! `common::mfb_exe()`, which builds it on demand) and must run with the repo
//! root as its working directory, because the script derives `REPO` from `pwd`.

mod common;

use std::path::Path;
use std::process::Command;

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

    assert!(
        status.success(),
        "artifact-gate.sh failed (exit {:?}); see output above",
        status.code()
    );
}
