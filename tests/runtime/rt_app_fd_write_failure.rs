//! bug-799: an `--app` build's headless fd fallback for `io::write`/`io::print`
//! issued one `write()` and ignored its result, so a failed write returned `Ok` and
//! a short write dropped the tail. It now runs the console write loop: every byte,
//! `EINTR` retried, stdout `EPIPE` re-raising SIGPIPE, otherwise `ErrWriteFailed`.
//!
//! Run headless (`MFB_HEADLESS=1`) with stdout bound to a read-only file:
//! the write fails, and the program's `TRAP` must see `ErrWriteFailed` (77020002).
//! The same program was verified on Windows (box 206, `MFB_HEADLESS`) and
//! Linux GTK (box 2226, `MFB_HEADLESS`); see the bug doc.

#![cfg(target_os = "macos")]

#[path = "../common/mod.rs"]
mod common;

use std::process::{Command, Stdio};

const SOURCE: &str = r#"IMPORT io

FUNC main() AS Nothing
  io::write("lost") TRAP(e)
    io::printError("write " & toString(e.code))
    RECOVER
  END TRAP
  io::printError("done")
END FUNC
"#;

fn build(name: &str) -> std::path::PathBuf {
    let project = common::temp_project(name, SOURCE);
    let built = Command::new(common::mfb_exe())
        .arg("build")
        .arg("--app")
        .arg(&project)
        .output()
        .expect("run mfb build --app");
    assert!(
        built.status.success(),
        "build failed:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );
    project
        .join("build")
        .join(format!("{name}.app"))
        .join("Contents/MacOS")
        .join(name)
}

#[test]
fn a_failed_headless_app_write_raises_write_failed() {
    let exe = build("bug799_app_fd_write");
    let read_only = std::fs::File::open(&exe).expect("open a read-only descriptor");
    let output = Command::new(&exe)
        .env("MFB_HEADLESS", "1")
        .stdout(Stdio::from(read_only))
        .output()
        .expect("run the app headless");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "exit {:?}:\n{stderr}",
        output.status
    );
    assert_eq!(
        stderr.lines().collect::<Vec<_>>(),
        ["write 77020002", "done"],
        "the failed write must reach the TRAP"
    );
}

#[test]
fn a_headless_app_write_still_reaches_stdout() {
    let exe = build("bug799_app_fd_write_ok");
    let output = Command::new(&exe)
        .env("MFB_HEADLESS", "1")
        .output()
        .expect("run the app headless");
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "lost");
    assert_eq!(String::from_utf8_lossy(&output.stderr).trim(), "done");
}
