//! The `timezones` package's own gates, run under `cargo test`, not only by hand:
//!
//! - its MFBASIC test suite (`mfb test packages/timezones`), which pins the rules;
//! - its consumer smoke test, which builds a program against the compiled `.mfp`
//!   and checks what it prints;
//! - its doc-example check, which builds and runs every `EXAMPLE` and every
//!   whole-program README block.
//!
//! The two scripts are bash, so they run on Unix only, as `script_selftests` does.
//! The suite runs everywhere `mfb` does. Each uses the release compiler from
//! `common::mfb_exe()`.

#[path = "../common/mod.rs"]
mod common;

use std::path::PathBuf;
use std::process::Command;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn assert_success(what: &str, out: &std::process::Output) {
    assert!(
        out.status.success(),
        "{what} failed ({})\n--- stdout\n{}\n--- stderr\n{}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn timezones_package_test_suite_passes() {
    let mfb = common::mfb_exe();
    let out = Command::new(&mfb)
        .args(["test", "packages/timezones"])
        .current_dir(root())
        .output()
        .expect("could not run mfb test packages/timezones");
    assert_success("mfb test packages/timezones", &out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("Fail: 0"),
        "timezones tests reported failures:\n{stdout}"
    );
}

#[cfg(unix)]
fn run_script(script: &str) {
    let mfb = common::mfb_exe();
    let out = Command::new("bash")
        .arg(root().join("packages").join("timezones").join(script))
        .arg(&mfb)
        .current_dir(root())
        .output()
        .unwrap_or_else(|e| panic!("could not spawn bash for {script}: {e}"));
    assert_success(&format!("packages/timezones/{script}"), &out);
}

#[cfg(unix)]
#[test]
fn timezones_runtime_smoke_passes() {
    run_script("runtime-smoke.sh");
}

#[cfg(unix)]
#[test]
fn timezones_doc_examples_build_and_run() {
    run_script("check-doc-examples.sh");
}
