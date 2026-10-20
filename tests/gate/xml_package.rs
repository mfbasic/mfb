//! The `xml` package's own gates, run under `cargo test`, not only by hand:
//!
//! - its MFBASIC test suite (`mfb test packages/xml`), which pins the rules;
//! - the compiler's audit (`mfb audit packages/xml`), which must report no errors;
//! - its doc-example check, which builds and runs every `EXAMPLE` in `lib.mfb`.
//!
//! The differential oracles (`packages/xml/oracle/diff.mjs`) need Node and a Rust
//! toolchain, and the W3C suite is fetched, so they run by hand, as the oracle
//! README describes. The doc-example check is bash and runs on Unix only, as
//! `script_selftests` does. Each uses the release compiler from
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
fn xml_package_test_suite_passes() {
    let mfb = common::mfb_exe();
    let out = Command::new(&mfb)
        .args(["test", "packages/xml"])
        .current_dir(root())
        .output()
        .expect("could not run mfb test packages/xml");
    assert_success("mfb test packages/xml", &out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("Fail: 0"),
        "xml tests reported failures:\n{stdout}"
    );
}

#[test]
fn xml_package_audit_reports_no_errors() {
    let mfb = common::mfb_exe();
    let out = Command::new(&mfb)
        .args(["audit", "packages/xml"])
        .current_dir(root())
        .output()
        .expect("could not run mfb audit packages/xml");
    assert_success("mfb audit packages/xml", &out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("errors: 0"),
        "xml audit reported errors:\n{stdout}"
    );
}

#[cfg(unix)]
#[test]
fn xml_doc_examples_build_and_run() {
    let mfb = common::mfb_exe();
    let out = Command::new("bash")
        .arg(
            root()
                .join("packages")
                .join("xml")
                .join("check-doc-examples.sh"),
        )
        .arg(&mfb)
        .current_dir(root())
        .output()
        .expect("could not spawn bash for packages/xml/check-doc-examples.sh");
    assert_success("packages/xml/check-doc-examples.sh", &out);
}
