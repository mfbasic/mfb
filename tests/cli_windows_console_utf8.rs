//! Windows console UTF-8 code-page regression (bug-392).
//!
//! A compiled MFB program that prints non-ASCII UTF-8 to an interactive Windows
//! console mojibaked, because the runtime writes correct UTF-8 bytes verbatim
//! (`WriteFile`) but the fresh console decodes them with its legacy OEM code
//! page (437/850), not UTF-8 (65001). The fix sets the console output/input code
//! page to 65001 once in the `_start` entry stub (`SetConsoleOutputCP` /
//! `SetConsoleCP`), so the console decodes the same bytes as the intended glyphs.
//!
//! The console *decode* itself is not observable in CI (there is no Windows
//! console here, and the dev/CI host is macOS — these tests cross-compile and
//! never execute the PE). So the gate is two-sided and host-independent:
//!   1. the Windows entry now imports and calls `SetConsoleOutputCP`/`SetConsoleCP`
//!      (the fix), and
//!   2. the emitted output bytes stay byte-identical raw UTF-8 — the em-dash
//!      `E2 80 94` appears verbatim in the PE (the non-goal: a fix must NOT
//!      transcode file/pipe output), and no other backend gains the imports.

mod common;
use common::mfb_exe;
use std::path::{Path, PathBuf};
use std::process::Command;

/// A program printing a multi-byte UTF-8 string (em-dash U+2014 = E2 80 94).
const SOURCE: &str =
    "IMPORT io\n\nSUB main()\n  io::print(\"browser — a tiny terminal web viewer\")\nEND SUB\n";

fn temp_project(name: &str) -> PathBuf {
    // A fixed-name project dir under the OS temp dir; the test target string keeps
    // parallel test runs from colliding.
    let root = std::env::temp_dir().join(format!("mfb_bug392_{name}"));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("src")).expect("create temp project");
    std::fs::write(
        root.join("project.json"),
        format!(
            "{{\"name\":\"{name}\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"executable\",\"sources\":[{{\"root\":\"src\",\"role\":\"main\",\"include\":[\"**/*.mfb\"]}}],\"entry\":\"main\",\"targets\":[\"native\"]}}\n"
        ),
    )
    .expect("write project.json");
    std::fs::write(root.join("src/main.mfb"), SOURCE).expect("write source");
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
fn windows_entry_sets_console_utf8_code_page() {
    // The fix: the `_start` entry imports and calls SetConsoleOutputCP (and
    // SetConsoleCP for symmetric input), so the console decodes the verbatim
    // UTF-8 bytes as glyphs instead of OEM mojibake.
    let project = temp_project("cp_win");
    let (ok, stdout, stderr) = run_mfb(&project, &["-target", "windows-x86_64", "-nplan"]);
    assert!(ok, "windows -nplan build failed:\n{stdout}\n{stderr}");
    let nplan = std::fs::read_to_string(project.join("cp_win.nplan")).expect("read nplan");
    assert!(
        nplan.contains("\"symbol\": \"SetConsoleOutputCP\""),
        "the Windows entry must import SetConsoleOutputCP to set the console code \
         page to UTF-8 (bug-392); nplan:\n{nplan}"
    );
    assert!(
        nplan.contains("\"symbol\": \"SetConsoleCP\""),
        "the Windows entry must import SetConsoleCP for symmetric UTF-8 console \
         input (bug-392); nplan:\n{nplan}"
    );
    // The code-page setup rides `_start`, not a runtime surface — a plain `print`
    // program (no term::) must get it, since bare `print \"—\"` mojibaked too.
    assert!(
        nplan.contains("{ \"library\": \"kernel32.dll\", \"symbol\": \"SetConsoleOutputCP\", \"requiredBy\": \"_start\" }"),
        "SetConsoleOutputCP must be required by _start (the entry), not a term:: arm; nplan:\n{nplan}"
    );
}

#[test]
fn windows_output_bytes_stay_raw_utf8() {
    // Non-goal guard: the fix must NOT transcode the program's output. The em-dash
    // is stored and written as raw UTF-8 (E2 80 94); redirected-to-file output is
    // therefore byte-identical. Assert the exact bytes survive into the PE.
    let project = temp_project("bytes_win");
    let (ok, stdout, stderr) = run_mfb(&project, &["-target", "windows-x86_64"]);
    assert!(ok, "windows build failed:\n{stdout}\n{stderr}");
    let exe = project.join("build/bytes_win.exe");
    let bytes = std::fs::read(&exe).expect("read PE");
    assert_eq!(&bytes[0..2], b"MZ", "the artifact is a PE image");
    let emdash = b"\xe2\x80\x94";
    let count = bytes.windows(emdash.len()).filter(|w| *w == emdash).count();
    assert_eq!(
        count, 1,
        "the em-dash must appear exactly once as raw UTF-8 (E2 80 94) — a fix that \
         transcodes output would corrupt or duplicate it"
    );
}

#[test]
fn non_windows_backends_do_not_gain_the_code_page_call() {
    // Non-goal guard: macOS/Linux/riscv are untouched — a POSIX terminal decodes
    // UTF-8 unconditionally, so the SetConsoleOutputCP seam is a Windows-only
    // override (default trait impl emits nothing). No other target's entry may
    // grow the import.
    let project = temp_project("cp_linux");
    let (ok, stdout, stderr) = run_mfb(&project, &["-target", "linux-aarch64", "-nplan"]);
    assert!(ok, "linux -nplan build failed:\n{stdout}\n{stderr}");
    let nplan = std::fs::read_to_string(project.join("cp_linux.nplan")).expect("read nplan");
    assert!(
        !nplan.contains("SetConsoleOutputCP") && !nplan.contains("SetConsoleCP"),
        "the Linux entry must NOT import the Windows console code-page calls; nplan:\n{nplan}"
    );
}
