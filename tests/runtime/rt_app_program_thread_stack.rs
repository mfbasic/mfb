//! bug-803: an `--app` build ran the program on a thread created with default
//! pthread attributes, so it got the platform's default thread stack (512 KiB on
//! macOS, 128 KiB on musl) instead of the `THREAD_STACK_BYTES` (8 MiB) every
//! console program and `thread::start` worker gets. A recursion a console build
//! completes raised `ErrStackOverflow` (before bug-789: died by SIGSEGV).
//!
//! The bootstrap now creates the program thread with an explicit
//! `THREAD_STACK_BYTES` stack. Run headless (`MFB_MACAPP_HEADLESS=1`): a
//! 20000-deep recursion must return its value. The Linux GTK bootstrap was
//! verified on box 2226 under `ulimit -s 256`; see the bug doc.

#![cfg(target_os = "macos")]

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

const SOURCE: &str = r#"IMPORT io

FUNC recur(n AS Integer) AS Integer
  IF n = 0 THEN
    RETURN 0
  END IF
  RETURN recur(n - 1) + 1
END FUNC

FUNC main() AS Nothing
  io::print(toString(recur(20000)))
END FUNC
"#;

#[test]
fn a_headless_app_program_gets_the_console_stack() {
    let name = "bug803_app_program_stack";
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
    let exe = project
        .join("build")
        .join(format!("{name}.app"))
        .join("Contents/MacOS")
        .join(name);
    let output = Command::new(&exe)
        .env("MFB_MACAPP_HEADLESS", "1")
        .output()
        .expect("run the app headless");
    assert!(
        output.status.success(),
        "exit {:?}:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "20000");
}
