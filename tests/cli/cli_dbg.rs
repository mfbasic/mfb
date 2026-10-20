//! `mfb dbg`: the command line on every host, and on macOS and Linux on AArch64
//! real sessions — a launched program paused before it runs and killed on `q`,
//! and a running one attached to, paused, and left running on `q`.
//!
//! macOS hands a debugger another program's task port only when the debugger is
//! signed with the `com.apple.security.cs.debugger` entitlement. The cargo-built
//! `mfb` is not, so the live sessions there run a copy signed by
//! `scripts/sign-debugger.sh` — which is also the proof that script works.
//! Linux needs no signing; a system whose `kernel.yama.ptrace_scope` keeps a
//! debugger from attaching to a program it did not start has each attaching
//! test check the refusal instead.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

#[path = "../common/mod.rs"]
mod common;

#[cfg(any(target_os = "macos", all(target_os = "linux", target_arch = "aarch64")))]
#[path = "cli_dbg/session_launch_and_list.rs"]
mod session_launch_and_list;
#[cfg(any(target_os = "macos", all(target_os = "linux", target_arch = "aarch64")))]
#[path = "cli_dbg/threads_watch_and_run.rs"]
mod threads_watch_and_run;

use common::*;
#[cfg(any(target_os = "macos", all(target_os = "linux", target_arch = "aarch64")))]
use session_launch_and_list::*;

fn dbg(mfb: &Path, args: &[&str]) -> (Option<i32>, String, String) {
    let output = Command::new(mfb)
        .arg("dbg")
        .args(args)
        .stdin(Stdio::null())
        .output()
        .expect("run mfb dbg");
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn help_and_usage_errors_on_every_host() {
    let mfb = PathBuf::from(mfb_exe());
    let (code, stdout, _) = dbg(&mfb, &["--help"]);
    assert_eq!(code, Some(0));
    assert!(
        stdout.starts_with("Usage: mfb dbg <binary> [-- <args>...]"),
        "{stdout}"
    );

    let (code, _, stderr) = dbg(&mfb, &[]);
    assert_eq!(code, Some(2));
    assert!(
        stderr.starts_with("error: mfb dbg requires a <binary> or a <pid>"),
        "{stderr}"
    );

    // `--help` after `--` is the program's, not the debugger's.
    let (code, _, stderr) = dbg(&mfb, &["4242", "--", "--help"]);
    assert_eq!(code, Some(2));
    assert!(
        stderr.starts_with("error: program arguments after --"),
        "{stderr}"
    );
}

#[cfg(not(any(target_os = "macos", all(target_os = "linux", target_arch = "aarch64"))))]
#[test]
fn the_debugger_is_not_supported_off_macos_and_linux_aarch64() {
    let project = temp_project("dbg_probe", "FUNC main AS Integer\n  RETURN 0\nEND FUNC\n");
    let binary = build_project(&project);
    let (code, stdout, stderr) = dbg(&PathBuf::from(mfb_exe()), &[binary.to_str().unwrap()]);
    assert_eq!(code, Some(1), "{stdout}");
    assert_eq!(
        stderr,
        format!(
            "error: mfb dbg is not supported on {} {}; it runs on macOS and on Linux on \
             AArch64\n",
            std::env::consts::OS,
            std::env::consts::ARCH
        )
    );
}
