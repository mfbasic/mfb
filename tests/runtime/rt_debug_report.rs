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

/// Assert a report block's shape: the core section for `build`, then each registered
/// section's `<key> <value>` lines (the `arena` and `process` sections everywhere, with
/// exactly one positive `process.peak_rss_bytes`; the `perf` section only on macOS,
/// plan-130-B, where its lines must include the six statistics of the whole-program
/// span), then the end line. Returns the `perf.` lines (none off macOS).
fn assert_block(case: &str, block: &[String], build: &str) -> Vec<String> {
    let head = [
        "mfb.debug.begin 1".to_string(),
        format!("mfb.debug.target {}", host_target()),
        format!("mfb.debug.build {build}"),
    ];
    assert!(
        block.len() >= 4,
        "{case}: report block too short: {block:?}"
    );
    assert_eq!(&block[..3], &head[..], "{case}: report block head");
    assert_eq!(
        block.last().map(String::as_str),
        Some("mfb.debug.end 1"),
        "{case}: report block end"
    );
    let body = &block[3..block.len() - 1];
    for line in body {
        let (key, value) = line
            .split_once(' ')
            .unwrap_or_else(|| panic!("{case}: `{line}` is not `<key> <value>`"));
        assert!(
            ["perf.", "arena.", "process."]
                .iter()
                .any(|prefix| key.starts_with(prefix))
                && !value.is_empty()
                && !value.contains(' '),
            "{case}: `{line}` is not a `perf.`/`arena.`/`process.` section line"
        );
    }
    let peak_rss = body
        .iter()
        .filter_map(|line| line.strip_prefix("process.peak_rss_bytes "))
        .collect::<Vec<_>>();
    assert!(
        peak_rss.len() == 1 && peak_rss[0].parse::<u64>().is_ok_and(|bytes| bytes > 0),
        "{case}: expected one positive `process.peak_rss_bytes <n>` line in {body:?}"
    );
    let perf: Vec<String> = body
        .iter()
        .filter(|line| line.starts_with("perf."))
        .cloned()
        .collect();
    if !cfg!(target_os = "macos") {
        assert!(perf.is_empty(), "{case}: perf is macOS-only, got {perf:?}");
        return perf;
    }
    for line in &perf {
        let value = line
            .split_once(' ')
            .map(|(_, value)| value)
            .unwrap_or_default();
        assert!(
            value.parse::<u64>().is_ok(),
            "{case}: `{line}` is not a `perf.<key> <integer>` line"
        );
    }
    assert!(
        perf.iter().any(|line| line == "perf.program.count 1"),
        "{case}: the whole-program span must be counted once: {perf:?}"
    );
    for stat in ["avg", "median", "min", "max", "sum"] {
        let key = format!("perf.program.{stat} ");
        assert!(
            perf.iter().any(|line| line.starts_with(&key)),
            "{case}: missing `{key}<n>` in {perf:?}"
        );
    }
    perf
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
    assert_block(case, &block, "console");
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
    assert_block("dbg_sigterm", &block, "console");
}

/// Build `source` as project `name` for `target` with `--ncode` (and `--debug` when
/// asked), returning the parsed dump.
fn build_ncode(name: &str, source: &str, target: &str, debug: bool) -> serde_json::Value {
    let project = common::temp_project(name, source);
    let mut command = Command::new(common::mfb_exe());
    command
        .arg("build")
        .arg("--ncode")
        .arg("--target")
        .arg(target);
    if debug {
        command.arg("--debug");
    }
    let output = command
        .arg(&project)
        .output()
        .expect("run mfb build --ncode");
    assert!(
        output.status.success(),
        "{name}: mfb build --ncode --target {target} (debug={debug}) failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let text = std::fs::read_to_string(project.join(format!("{name}.ncode")))
        .unwrap_or_else(|err| panic!("{name}: read the ncode dump: {err}"));
    serde_json::from_str(&text).expect("parse the ncode dump")
}

fn functions(plan: &serde_json::Value) -> &Vec<serde_json::Value> {
    plan["functions"].as_array().expect("functions array")
}

/// The report's value of `key` (`<key> <u64>`), from a `--debug` run's stderr.
fn report_value(case: &str, stderr: &str, key: &str) -> u64 {
    let (_, block) = split_report(case, stderr);
    let prefix = format!("{key} ");
    block
        .iter()
        .find_map(|line| line.strip_prefix(&prefix))
        .and_then(|value| value.parse().ok())
        .unwrap_or_else(|| panic!("{case}: no `{key} <n>` in {block:?}"))
}

/// plan-130-D: a program that builds a 64 MiB string reports a peak resident set
/// size of at least 64 MiB — the value is bytes on every target (Linux's KiB are
/// scaled) and counts memory the process touched.
#[test]
fn a_64_mib_string_reports_at_least_64_mib_peak_rss() {
    let source = "IMPORT io\n\nFUNC main AS Integer\n  MUT s AS String = \"x\"\n  FOR i = 1 TO 26\n    s = s & s\n  NEXT\n  io::print(toString(len(s)))\n  RETURN 0\nEND FUNC\n";
    let run = run(&build("dbg_peak_rss", source, true));
    assert!(run.status.success(), "dbg_peak_rss failed: {}", run.stderr);
    assert_eq!(run.stdout.trim(), "67108864");
    let (_, block) = split_report("dbg_peak_rss", &run.stderr);
    assert_block("dbg_peak_rss", &block, "console");
    let peak = report_value("dbg_peak_rss", &run.stderr, "process.peak_rss_bytes");
    assert!(
        peak >= 64 * 1024 * 1024,
        "dbg_peak_rss: process.peak_rss_bytes {peak} is below 64 MiB"
    );
}

/// plan-130-D: only a `--debug` build references the peak-RSS call, on every
/// target — a normal build's import table gains nothing.
#[test]
fn only_a_debug_build_imports_the_peak_rss_call() {
    let source = "FUNC main() AS Integer\n  RETURN 0\nEND FUNC\n";
    for (target, calls) in [
        ("macos-aarch64", &["_getrusage"][..]),
        ("linux-aarch64", &["getrusage"][..]),
        ("linux-x86_64", &["getrusage"][..]),
        ("linux-riscv64", &["getrusage"][..]),
        (
            "windows-x86_64",
            &["GetCurrentProcess\"", "K32GetProcessMemoryInfo"][..],
        ),
    ] {
        let slug = target.replace('-', "_");
        let normal =
            build_ncode(&format!("dbgrss_{slug}_normal"), source, target, false).to_string();
        let debug = build_ncode(&format!("dbgrss_{slug}_debug"), source, target, true).to_string();
        for call in calls {
            assert!(
                !normal.contains(call),
                "{target}: a build without --debug references {call}"
            );
            assert!(
                debug.contains(call),
                "{target}: a --debug build does not reference {call}"
            );
        }
    }
}

/// plan-130-A Phase 3: the call site on the four targets this host cannot run.
///
/// A runtime test only exercises the host backend; the call inside
/// `_mfb_shutdown` is emitted per backend, so each one is inspected in its own
/// dump. In a `--debug` build the instruction right after the `shutdown_done`
/// label is the call to `_mfb_debug_shutdown` (after the label, so the
/// already-shut-down early return reaches it too); a normal build carries no
/// `_mfb_debug` symbol or reference at all.
#[test]
fn every_cross_target_calls_the_report_right_after_shutdown_done() {
    let source =
        "IMPORT io\n\nFUNC main() AS Integer\n  io::print(\"hello\")\n  RETURN 0\nEND FUNC\n";
    for target in [
        "linux-aarch64",
        "linux-x86_64",
        "linux-riscv64",
        "windows-x86_64",
    ] {
        let slug = target.replace('-', "_");

        let normal = build_ncode(&format!("dbgncode_{slug}_normal"), source, target, false);
        assert!(
            !normal.to_string().contains("_mfb_debug"),
            "{target}: a build without --debug must carry no _mfb_debug symbol or reference"
        );

        let debug = build_ncode(&format!("dbgncode_{slug}_debug"), source, target, true);
        let symbols: Vec<&str> = functions(&debug)
            .iter()
            .filter_map(|function| function["symbol"].as_str())
            .collect();
        assert!(
            !debug.to_string().contains("_mfb_rt_perf_"),
            "{target}: perf is macOS-only; a --debug build must emit no perf helper"
        );
        for expected in ["_mfb_debug_shutdown", "_mfb_debug_report_core"] {
            assert!(
                symbols.contains(&expected),
                "{target}: --debug build is missing {expected}"
            );
        }
        let shutdown = functions(&debug)
            .iter()
            .find(|function| function["symbol"].as_str() == Some("_mfb_shutdown"))
            .unwrap_or_else(|| panic!("{target}: no _mfb_shutdown in the --debug dump"));
        let instructions = shutdown["instructions"].as_array().expect("instructions");
        let done = instructions
            .iter()
            .position(|instruction| {
                instruction["op"].as_str() == Some("label")
                    && instruction["name"].as_str() == Some("shutdown_done")
            })
            .unwrap_or_else(|| panic!("{target}: _mfb_shutdown has no shutdown_done label"));
        let next = &instructions[done + 1];
        assert_eq!(
            next["target"].as_str(),
            Some("_mfb_debug_shutdown"),
            "{target}: the instruction after shutdown_done must call the report, got {next}"
        );
    }
}

/// plan-130-A Phase 3: an app build's worker finishes through `_mfb_shutdown` too,
/// so a headless macOS `--app --debug` run ends stderr with the report (build
/// token `app`) and otherwise behaves like the `--app` build.
///
/// `MFB_MACAPP_HEADLESS` skips all window construction, so this opens nothing.
#[cfg(target_os = "macos")]
#[test]
fn a_headless_app_finish_ends_stderr_with_the_report() {
    let source = "IMPORT io\n\nFUNC main() AS Integer\n  io::print(\"hi\")\n  RETURN 3\nEND FUNC\n";
    let run_app = |name: &str, debug: bool| {
        let project = common::temp_project(name, source);
        let mut command = Command::new(common::mfb_exe());
        command.arg("build").arg("--app");
        if debug {
            command.arg("--debug");
        }
        let output = command.arg(&project).output().expect("run mfb build --app");
        assert!(
            output.status.success(),
            "{name}: app build failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let output = Command::new(common::app_binary(&project, name))
            .env("MFB_MACAPP_HEADLESS", "1")
            .output()
            .expect("run the headless app");
        Run {
            status: output.status,
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        }
    };
    let normal = run_app("dbgapp_normal", false);
    let debug = run_app("dbgapp_debug", true);
    assert!(
        !normal.stderr.contains("mfb.debug."),
        "an --app build without --debug must print no report:\n{}",
        normal.stderr
    );
    assert_eq!(
        debug.status.code(),
        normal.status.code(),
        "--debug changed the exit code"
    );
    assert_eq!(debug.stdout, normal.stdout, "--debug changed stdout");
    let (before, block) = split_report("dbgapp", &debug.stderr);
    assert_eq!(
        before, normal.stderr,
        "--debug changed stderr before the report"
    );
    assert_block("dbgapp", &block, "app");
}

/// plan-130-B: on macOS the `perf` section also times the arena. A program that
/// allocates reports `perf.mfb_alloc.count` of at least one, and every arena span it
/// reports carries all six statistics.
#[cfg(target_os = "macos")]
#[test]
fn the_perf_section_times_the_arena() {
    let source = concat!(
        "IMPORT io\n",
        "\n",
        "FUNC main() AS Integer\n",
        "  LET a = \"abc\" & toString(12345)\n",
        "  LET b = a & a\n",
        "  io::print(b)\n",
        "  RETURN 0\n",
        "END FUNC\n",
    );
    let debug = run(&build("dbg_perf_arena", source, true));
    let (_, block) = split_report("dbg_perf_arena", &debug.stderr);
    let perf = assert_block("dbg_perf_arena", &block, "console");
    let alloc_count = perf
        .iter()
        .find_map(|line| line.strip_prefix("perf.mfb_alloc.count "))
        .and_then(|n| n.parse::<u64>().ok())
        .unwrap_or_else(|| panic!("no perf.mfb_alloc.count line: {perf:?}"));
    assert!(alloc_count >= 1, "the program allocates; got {alloc_count}");
    for span in ["mfb_alloc", "mfb_free"] {
        if perf
            .iter()
            .any(|line| line.starts_with(&format!("perf.{span}.count ")))
        {
            for stat in ["avg", "median", "min", "max", "sum"] {
                let key = format!("perf.{span}.{stat} ");
                assert!(
                    perf.iter().any(|line| line.starts_with(&key)),
                    "missing `{key}<n>` in {perf:?}"
                );
            }
        }
    }
}
