//! plan-130-A: `mfb build --debug` ends stderr with exactly one debug report block.
//!
//! The report is written by `_mfb_debug_shutdown` as the last act of
//! `_mfb_shutdown`, so it must appear on every path that reaches shutdown — a
//! normal return, `EXIT PROGRAM n`, an untrapped error, and SIGTERM on Unix —
//! without changing anything the program itself does: the same exit status, the
//! same stdout, and the same stderr up to the block. A build without `--debug`
//! must print no report at all.
//!
//! Each case builds the SAME source twice, with and without `--debug`, and
//! compares the two runs; nothing here hard-codes what the program prints.

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::io::{BufRead, BufReader, Read};
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

/// The report's target token for the host this suite builds for.
fn host_target() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos-aarch64"
    } else if cfg!(target_arch = "x86_64") {
        "linux-x86_64"
    } else if cfg!(target_arch = "riscv64") {
        "linux-riscv64"
    } else {
        "linux-aarch64"
    }
}

/// The §4.4 block a console build on this host prints.
fn expected_block() -> Vec<String> {
    vec![
        "mfb.debug.begin 1".to_string(),
        format!("mfb.debug.target {}", host_target()),
        "mfb.debug.build console".to_string(),
        "mfb.debug.end 1".to_string(),
    ]
}

/// Build `source` as its own project, with or without `--debug`, and return the
/// executable to run. On Linux the console build writes one executable per libc
/// world; the glibc one is the host's.
fn build(name: &str, source: &str, debug: bool) -> PathBuf {
    let project = common::temp_project(name, source);
    let mut command = Command::new(common::mfb_exe());
    command.arg("build");
    if debug {
        command.arg("--debug");
    }
    let output = command.arg(&project).output().expect("run mfb build");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success(),
        "{name} (debug={debug}) failed to build:\nstdout:\n{stdout}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let written: Vec<&str> = stdout
        .lines()
        .filter_map(|line| line.strip_prefix("Wrote executable to "))
        .collect();
    let chosen = written
        .iter()
        .find(|path| path.ends_with("-glibc.out"))
        .or_else(|| written.first())
        .unwrap_or_else(|| panic!("{name}: no executable in build output:\n{stdout}"));
    PathBuf::from(chosen)
}

struct Run {
    status: ExitStatus,
    stdout: String,
    stderr: String,
}

fn run(exe: &Path) -> Run {
    let output = Command::new(exe).output().expect("run built program");
    Run {
        status: output.status,
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// Split a `--debug` run's stderr into (everything before the block, the block).
///
/// Asserts the block is the LAST thing on stderr and appears exactly once.
fn split_report(case: &str, stderr: &str) -> (String, Vec<String>) {
    let begins = stderr.matches("mfb.debug.begin ").count();
    assert_eq!(
        begins, 1,
        "{case}: expected exactly one report block, found {begins}:\n{stderr}"
    );
    let at = stderr.find("mfb.debug.begin ").expect("counted above");
    assert!(
        at == 0 || stderr[..at].ends_with('\n'),
        "{case}: the block must start on its own line:\n{stderr}"
    );
    let block: Vec<String> = stderr[at..].lines().map(str::to_string).collect();
    assert!(
        stderr.ends_with("mfb.debug.end 1\n"),
        "{case}: the block must be the last thing on stderr:\n{stderr}"
    );
    (stderr[..at].to_string(), block)
}

/// Build `source` both ways, run both, and assert the plan-130-A contract.
fn assert_report_on_exit(case: &str, source: &str) {
    let normal = run(&build(&format!("{case}_normal"), source, false));
    let debug = run(&build(&format!("{case}_debug"), source, true));
    assert!(
        !normal.stderr.contains("mfb.debug."),
        "{case}: a build without --debug must print no report:\n{}",
        normal.stderr
    );
    assert_eq!(
        debug.status.code(),
        normal.status.code(),
        "{case}: --debug changed the exit code"
    );
    assert_eq!(
        debug.stdout, normal.stdout,
        "{case}: --debug changed stdout"
    );
    let (before, block) = split_report(case, &debug.stderr);
    assert_eq!(
        before, normal.stderr,
        "{case}: --debug changed stderr before the report"
    );
    assert_eq!(block, expected_block(), "{case}: report block");
}

#[test]
fn a_normal_return_ends_stderr_with_the_report() {
    assert_report_on_exit(
        "dbg_return",
        "IMPORT io\n\nFUNC main() AS Integer\n  io::print(\"hello\")\n  RETURN 0\nEND FUNC\n",
    );
}

#[test]
fn exit_program_ends_stderr_with_the_report() {
    assert_report_on_exit(
        "dbg_exit",
        concat!(
            "IMPORT io\n",
            "\n",
            "FUNC main() AS Integer\n",
            "  io::print(\"before\")\n",
            "  MUT code AS Integer = 3\n",
            "  IF code = 3 THEN\n",
            "    EXIT PROGRAM code\n",
            "  END IF\n",
            "  RETURN 0\n",
            "END FUNC\n",
        ),
    );
}

#[test]
fn an_untrapped_error_ends_stderr_with_the_report() {
    assert_report_on_exit(
        "dbg_error",
        concat!(
            "IMPORT io\n",
            "\n",
            "FUNC main() AS Integer\n",
            "  MUT z AS Integer = 0\n",
            "  io::print(\"before\")\n",
            "  io::print(toString(1 / z))\n",
            "  RETURN 0\n",
            "END FUNC\n",
        ),
    );
}

/// Spawn `exe`, wait for its first stdout line, send SIGTERM, and collect the rest.
fn run_until_sigterm(case: &str, exe: &Path) -> Run {
    let mut child = Command::new(exe)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn built program");
    let mut stdout = BufReader::new(child.stdout.take().expect("stdout pipe"));
    let mut first = String::new();
    stdout.read_line(&mut first).expect("read the ready line");
    assert_eq!(first, "ready\n", "{case}: the program did not start");
    // SAFETY: `kill` on the pid of a child this test owns and has not reaped.
    let sent = unsafe { libc::kill(child.id() as libc::pid_t, libc::SIGTERM) };
    assert_eq!(sent, 0, "{case}: kill(SIGTERM) failed");
    let deadline = Instant::now() + Duration::from_secs(30);
    let status = loop {
        if let Some(status) = child.try_wait().expect("poll child") {
            break status;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            panic!("{case}: the program did not exit after SIGTERM");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let mut rest = String::new();
    stdout.read_to_string(&mut rest).expect("read stdout");
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .expect("stderr pipe")
        .read_to_string(&mut stderr)
        .expect("read stderr");
    Run {
        status,
        stdout: first + &rest,
        stderr,
    }
}

#[test]
fn sigterm_mid_sleep_ends_stderr_with_the_report() {
    let source = concat!(
        "IMPORT io\n",
        "IMPORT os\n",
        "\n",
        "FUNC main() AS Integer\n",
        "  io::print(\"ready\")\n",
        "  os::sleep(30000)\n",
        "  RETURN 0\n",
        "END FUNC\n",
    );
    let normal = run_until_sigterm("dbg_sigterm", &build("dbg_sigterm_normal", source, false));
    let debug = run_until_sigterm("dbg_sigterm", &build("dbg_sigterm_debug", source, true));
    assert!(
        !normal.stderr.contains("mfb.debug."),
        "a build without --debug must print no report:\n{}",
        normal.stderr
    );
    // The handler exits 128 + SIGTERM; neither run may die by the signal itself.
    assert_eq!(
        normal.status.signal(),
        None,
        "normal build died by the signal"
    );
    assert_eq!(
        debug.status.code(),
        normal.status.code(),
        "--debug changed the exit code"
    );
    assert_eq!(debug.stdout, normal.stdout, "--debug changed stdout");
    let (before, block) = split_report("dbg_sigterm", &debug.stderr);
    assert_eq!(
        before, normal.stderr,
        "--debug changed stderr before the report"
    );
    assert_eq!(block, expected_block(), "SIGTERM report block");
}
