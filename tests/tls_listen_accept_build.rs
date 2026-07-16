//! Regression test for bug-249: a `tls::listen` + `tls::accept` program failed
//! to build with
//!
//! ```text
//! error: native code data relocation target '_mfb_str_error_tls_failed' is not a data object or defined symbol
//! ```
//!
//! The error-message data-object gate
//! (`src/target/shared/code/data_objects.rs`) keyed the `ErrTlsFailed` string
//! set on the client-side calls only (`tls.connect`/`read`/`write`/`close`).
//! The server-side helpers raise the same errors, and a listen+accept program
//! that lets scope-drop close its resources issues no NIR `tls.close` call at
//! all — so nothing fired the gate, the string was never emitted, and the
//! relocation the emitted helper bodies carry had no target.
//!
//! The drop-only close is the load-bearing part of these fixtures: the
//! pre-existing `tests/syntax/tls/accept_valid` fixture calls `tls::close`
//! explicitly, which fires the gate through the client-side row and hides the
//! bug. Do not "tidy" these by adding explicit closes.
//!
//! Build-only and target-independent (the gate is shared codegen), so these run
//! on any host via a cross-target build.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const TARGET: &str = "linux-x86_64";

fn temp_project(name: &str, source: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock before epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("mfb_{name}_{nonce}"));
    fs::create_dir_all(root.join("src")).expect("create temp project");
    fs::write(
        root.join("project.json"),
        format!(
            "{{\"name\":\"{name}\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"executable\",\"sources\":[{{\"root\":\"src\",\"role\":\"main\",\"include\":[\"**/*.mfb\"]}}],\"entry\":\"main\",\"targets\":[\"native\"]}}\n"
        ),
    )
    .expect("write project.json");
    fs::write(root.join("src/main.mfb"), source).expect("write source");
    root
}

fn assert_builds(name: &str, source: &str) {
    let project = temp_project(name, source);
    let output = Command::new(env!("CARGO_BIN_EXE_mfb"))
        .arg("build")
        .args(["-target", TARGET])
        .arg(&project)
        .output()
        .expect("run mfb build");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        output.status.success(),
        "{name} should build for {TARGET}:\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    let _ = fs::remove_dir_all(&project);
}

/// The bug-249 reproduction: listen + accept, both resources closed by
/// scope-drop rather than an explicit `tls::close` call.
#[test]
fn tls_listen_accept_with_dropped_resources_builds() {
    assert_builds(
        "tls_listen_accept",
        "IMPORT io\nIMPORT tls\n\n\
         FUNC main AS Integer\n\
        \x20 RES l = tls::listen(\"127.0.0.1\", 18443, \"/tmp/cert.pem\", \"/tmp/key.pem\")\n\
        \x20 RES s = tls::accept(l, 1500) TRAP(e)\n\
        \x20   RETURN 0\n\
        \x20 END TRAP\n\
        \x20 io::print(\"accepted\")\n\
        \x20 RETURN 0\n\
         END FUNC\n",
    );
}

/// `tls::listen` alone raises `ErrTlsFailed` too, so a listener-only program is
/// broken by the same gate gap and must build on its own.
#[test]
fn tls_listen_only_with_dropped_listener_builds() {
    assert_builds(
        "tls_listen_only",
        "IMPORT io\nIMPORT tls\n\n\
         FUNC main AS Integer\n\
        \x20 RES l = tls::listen(\"127.0.0.1\", 18443, \"/tmp/cert.pem\", \"/tmp/key.pem\")\n\
        \x20 io::print(\"listening\")\n\
        \x20 RETURN 0\n\
         END FUNC\n",
    );
}

/// Closing the listener explicitly goes through `tls::closeListener`, the third
/// server-side call the gate omitted.
#[test]
fn tls_listen_with_explicit_close_listener_builds() {
    assert_builds(
        "tls_close_listener",
        "IMPORT io\nIMPORT tls\n\n\
         FUNC main AS Integer\n\
        \x20 RES l = tls::listen(\"127.0.0.1\", 18443, \"/tmp/cert.pem\", \"/tmp/key.pem\")\n\
        \x20 tls::close(l)\n\
        \x20 io::print(\"closed\")\n\
        \x20 RETURN 0\n\
         END FUNC\n",
    );
}
