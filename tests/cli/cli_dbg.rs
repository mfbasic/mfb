//! `mfb dbg`: the command line on every host, and on macOS and on Linux on
//! AArch64 and riscv64 real sessions — a launched program paused before it runs
//! and killed on `q`, and a running one attached to, paused, and left running
//! on `q`. [`cpu`] names the host processor's registers and instructions, so one
//! session script serves both processors.
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

#[cfg(any(
    target_os = "macos",
    all(
        target_os = "linux",
        any(target_arch = "aarch64", target_arch = "riscv64")
    )
))]
#[path = "cli_dbg/session_launch_and_list.rs"]
mod session_launch_and_list;
#[cfg(any(
    target_os = "macos",
    all(
        target_os = "linux",
        any(target_arch = "aarch64", target_arch = "riscv64")
    )
))]
#[path = "cli_dbg/threads_watch_and_run.rs"]
mod threads_watch_and_run;

#[cfg(any(
    target_os = "macos",
    all(
        target_os = "linux",
        any(target_arch = "aarch64", target_arch = "riscv64")
    )
))]
mod cpu {
    //! The host processor's spellings the sessions assert: on AArch64 each is
    //! the literal the tests have always used.

    /// The first and second argument registers: MFBASIC's error code and
    /// value after a return.
    #[cfg(target_arch = "aarch64")]
    pub const ARG0: &str = "x0";
    #[cfg(target_arch = "aarch64")]
    pub const ARG1: &str = "x1";
    #[cfg(target_arch = "riscv64")]
    pub const ARG0: &str = "a0";
    #[cfg(target_arch = "riscv64")]
    pub const ARG1: &str = "a1";

    /// The register a call leaves its return address in.
    #[cfg(target_arch = "aarch64")]
    pub const LINK: &str = "lr";
    #[cfg(target_arch = "riscv64")]
    pub const LINK: &str = "ra";

    /// How `dis` shows a function's first instruction, the frame's
    /// allocation, up to its size.
    #[cfg(target_arch = "aarch64")]
    pub const PROLOGUE: &str = " sub    sp, sp, #0x";
    #[cfg(target_arch = "riscv64")]
    pub const PROLOGUE: &str = " addi   sp,sp,-";

    /// The bare `list reg` table: each role register and its role.
    #[cfg(target_arch = "aarch64")]
    pub const ROLES: &[(&str, &str)] = &[
        ("x0", "MFB arg, MFB result, C arg, C result"),
        ("x1", "MFB arg, MFB result, C arg, C result"),
        ("x2", "MFB arg, MFB result, C arg"),
        ("x3", "MFB arg, MFB result, C arg"),
        ("x4", "MFB arg, C arg"),
        ("x5", "MFB arg, C arg"),
        ("x6", "MFB arg, C arg"),
        ("x7", "MFB arg, C arg"),
        ("x19", "Arena"),
        ("x20", "Current Thread"),
        ("x28", "Closure"),
    ];
    #[cfg(target_arch = "riscv64")]
    pub const ROLES: &[(&str, &str)] = &[
        ("a0", "MFB arg, MFB result, C arg, C result"),
        ("a1", "MFB arg, MFB result, C arg, C result"),
        ("a2", "MFB arg, MFB result, C arg"),
        ("a3", "MFB arg, MFB result, C arg"),
        ("a4", "MFB arg, C arg"),
        ("a5", "MFB arg, C arg"),
        ("a6", "MFB arg, C arg"),
        ("a7", "MFB arg, C arg"),
        ("s2", "Current Thread"),
        ("s10", "Closure"),
        ("s11", "Arena"),
    ];

    /// Every register `list reg gpr sys simd` prints, in order.
    #[cfg(target_arch = "aarch64")]
    pub fn names() -> Vec<String> {
        let mut names = (0..29).map(|index| format!("x{index}")).collect::<Vec<_>>();
        names.extend(["fp", "lr", "sp", "pc", "nzcv", "fpsr", "fpcr"].map(String::from));
        names.extend((0..32).map(|index| format!("v{index}")));
        names
    }
    #[cfg(target_arch = "riscv64")]
    pub fn names() -> Vec<String> {
        let mut names = [
            "ra", "gp", "tp", "t0", "t1", "t2", "s0", "s1", "a0", "a1", "a2", "a3", "a4", "a5",
            "a6", "a7", "s2", "s3", "s4", "s5", "s6", "s7", "s8", "s9", "s10", "s11", "t3", "t4",
            "t5", "t6", "sp", "pc", "fcsr",
        ]
        .map(String::from)
        .to_vec();
        names.extend(
            [
                "ft0", "ft1", "ft2", "ft3", "ft4", "ft5", "ft6", "ft7", "fs0", "fs1", "fa0", "fa1",
                "fa2", "fa3", "fa4", "fa5", "fa6", "fa7", "fs2", "fs3", "fs4", "fs5", "fs6", "fs7",
                "fs8", "fs9", "fs10", "fs11", "ft8", "ft9", "ft10", "ft11",
            ]
            .map(String::from),
        );
        names
    }

    /// The hex digits register `name`'s value prints with: 32 for an AArch64
    /// vector register, else 16.
    pub fn digits(name: &str) -> usize {
        if cfg!(target_arch = "aarch64") && name.starts_with('v') {
            32
        } else {
            16
        }
    }
}

use common::*;
#[cfg(any(
    target_os = "macos",
    all(
        target_os = "linux",
        any(target_arch = "aarch64", target_arch = "riscv64")
    )
))]
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

#[cfg(not(any(
    target_os = "macos",
    all(
        target_os = "linux",
        any(target_arch = "aarch64", target_arch = "riscv64")
    )
)))]
#[test]
fn the_debugger_is_not_supported_off_its_three_hosts() {
    let project = temp_project("dbg_probe", "FUNC main AS Integer\n  RETURN 0\nEND FUNC\n");
    let binary = build_project(&project);
    let (code, stdout, stderr) = dbg(&PathBuf::from(mfb_exe()), &[binary.to_str().unwrap()]);
    assert_eq!(code, Some(1), "{stdout}");
    assert_eq!(
        stderr,
        format!(
            "error: mfb dbg is not supported on {} {}; it runs on macOS, and on Linux on \
             AArch64 and riscv64\n",
            std::env::consts::OS,
            std::env::consts::ARCH
        )
    );
}
