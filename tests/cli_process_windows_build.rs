//! Windows `process` backend emit-inspection gate (plan-90-D).
//!
//! There is no Windows console/kernel in CI (the dev/CI host is macOS — these
//! tests cross-compile and never execute the PE), so the Windows `process`
//! surface is proven two-sided and host-independently:
//!   1. a lifecycle program compiles for `windows-x86_64` and its plan imports the
//!      Win32 process primitives (`CreateProcessA`/`CreatePipe`/`WaitForSingleObject`),
//!   2. the same program compiles for a POSIX target WITHOUT those imports (the
//!      Unix backend uses `fork`/`execvp`/`pipe`).
//! On-box execution on box 2230 is the runtime gate (recorded in the plan).

mod common;
use common::mfb_exe;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Exercises the full Windows process surface: spawn (argv → CreateProcessA + 3
/// pipes), pid/isRunning/waitFor/close (lifecycle), send/receive/receiveBytes/poll
/// (I/O over WriteFile/ReadFile/PeekNamedPipe), and signal/didSignal/detach
/// (TerminateProcess/CloseHandle).
const LIFECYCLE_SOURCE: &str = "IMPORT process\nIMPORT io\n\nFUNC main AS Integer\n  RES p = process::spawn([\"C:\\\\Windows\\\\System32\\\\sort.exe\"])\n  process::send(p, \"hello\")\n  process::close(p)\n  LET up = process::isRunning(p)\n  io::print(toString(process::pid(p) > 0))\n  IF process::poll(p, 100) THEN\n    LET line = process::receive(p)\n    io::print(line)\n    LET raw = process::receiveBytes(p)\n    io::print(toString(len(raw)))\n  END IF\n  process::signal(p, process::Signal.Kill)\n  IF process::didSignal(p) = process::Signal.None THEN\n    io::print(\"none\")\n  END IF\n  LET code = process::waitFor(p)\n  io::print(toString(code))\n  RES d = process::spawn([\"C:\\\\Windows\\\\System32\\\\cmd.exe\", \"/c\", \"exit 0\"])\n  process::detach(d)\n  RETURN 0\nEND FUNC\n";

fn temp_project(name: &str, source: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("mfb_proc_win_{name}"));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("src")).expect("create temp project");
    std::fs::write(
        root.join("project.json"),
        format!(
            "{{\"name\":\"{name}\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"executable\",\"sources\":[{{\"root\":\"src\",\"role\":\"main\",\"include\":[\"**/*.mfb\"]}}],\"entry\":\"main\",\"targets\":[\"native\"]}}\n"
        ),
    )
    .expect("write project.json");
    std::fs::write(root.join("src/main.mfb"), source).expect("write source");
    root
}

fn run_mfb(project: &Path, args: &[&str]) -> (bool, String, String) {
    let output = Command::new(mfb_exe())
        .arg("build")
        .args(args)
        .arg(project)
        .output()
        .expect("run mfb build");
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn windows_process_lifecycle_compiles_and_imports_win32() {
    let project = temp_project("lifecycle", LIFECYCLE_SOURCE);
    let (ok, stdout, stderr) = run_mfb(&project, &["-target", "windows-x86_64", "-nplan"]);
    assert!(ok, "windows process build failed:\n{stdout}\n{stderr}");
    let nplan = std::fs::read_to_string(project.join("lifecycle.nplan")).expect("read nplan");
    for symbol in [
        "CreateProcessA",
        "CreatePipe",
        "SetHandleInformation",
        "WriteFile",
        "ReadFile",
        "PeekNamedPipe",
        "WaitForSingleObject",
        "GetExitCodeProcess",
        "TerminateProcess",
        "CloseHandle",
    ] {
        assert!(
            nplan.contains(&format!("\"symbol\": \"{symbol}\"")),
            "the Windows process backend must import {symbol}; nplan:\n{nplan}"
        );
    }
}

#[test]
fn posix_process_backend_does_not_import_win32() {
    // The same program on a POSIX target uses fork/execvp/pipe — none of the Win32
    // process primitives may appear.
    let project = temp_project("posix", LIFECYCLE_SOURCE);
    let (ok, stdout, stderr) = run_mfb(&project, &["-target", "linux-x86_64", "-nplan"]);
    assert!(ok, "linux process build failed:\n{stdout}\n{stderr}");
    let nplan = std::fs::read_to_string(project.join("posix.nplan")).expect("read nplan");
    assert!(
        !nplan.contains("CreateProcessA") && !nplan.contains("CreatePipe"),
        "a POSIX process backend must NOT import the Win32 process calls; nplan:\n{nplan}"
    );
}
