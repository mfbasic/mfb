//! bug-172 (finding B): `mfb pkg install a b` / `pkg update a b` fell through to
//! the catch-all "unknown pkg command `install`" because install/update had no
//! `[command, ..]` arity arm (unlike validate/verify/check-abi/transfer). They
//! now report a clear at-most-one usage error.
//!
//! Drives the real `mfb` binary with a too-many-arguments invocation and asserts
//! a bounded arity/usage error (exit 2), never the misleading "unknown pkg
//! command" fall-through.

use std::process::Command;

mod common;
use common::*;

fn run_pkg(args: &[&str]) -> std::process::Output {
    Command::new(mfb_exe())
        .arg("pkg")
        .args(args)
        .output()
        .expect("run mfb pkg")
}

#[test]
fn pkg_install_rejects_extra_arguments_with_arity_error() {
    let output = run_pkg(&["install", "a", "b", "c"]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(2), "stderr:\n{stderr}");
    assert!(
        stderr.contains("at most one"),
        "expected an at-most-one arity error, got:\n{stderr}"
    );
    assert!(
        !stderr.contains("unknown pkg command"),
        "install fell through to the catch-all (bug-172 regressed):\n{stderr}"
    );
}
