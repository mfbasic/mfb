//! Regression test for bug-247: a macOS app-mode build of a program that calls
//! `io::input` but never `io::readLine` aborted codegen with
//!
//! ```text
//! error: runtime helper requires _isatty import
//! ```
//!
//! App-mode `io.input` composes the *unchanged console* `io.readLine` body
//! (`src/target/shared/code/mod.rs`, the `build_mode.is_app()` force-emit) to
//! read the window input pipe. That body probes the tty via `isatty`/`tcgetattr`,
//! but the macOS plan only declared those two symbols in the import row for
//! programs calling `io.readLine`/`readChar`/`readByte` *directly* — so an
//! `io::input`-only program emitted code referencing symbols absent from the
//! platform-imports map.
//!
//! The `io::input`-without-`io::readLine` shape is the whole point of these
//! fixtures; adding an `io::readLine` call would fire the readLine import row
//! and mask the bug.
//!
//! Gated to macOS: only there does the build take the `macos_aarch64` app-mode
//! plan and produce a `.app` bundle.

#![cfg(target_os = "macos")]

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// The terminal probes the composed console readLine body calls. They are
/// no-ops against the fd-0 window pipe (`isatty(0)` = 0 skips the termios
/// calls), but the symbols must still bind.
const TERMINAL_PROBES: &[&str] = &["_isatty", "_tcgetattr"];

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

/// A program that prompts with `io::input` and never calls `io::readLine`.
const INPUT_ONLY_SOURCE: &str = "IMPORT io\n\n\
     FUNC main AS Integer\n\
    \x20 LET name AS String = io::input(\"Name > \")\n\
    \x20 io::print(\"Hi \" & name)\n\
    \x20 RETURN 0\n\
     END FUNC\n";

fn build_app(name: &str, source: &str) -> PathBuf {
    let project = temp_project(name, source);
    let output = Command::new(env!("CARGO_BIN_EXE_mfb"))
        .arg("build")
        .arg("-app")
        .arg(&project)
        .output()
        .expect("run mfb build -app");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        output.status.success(),
        "app build of an io::input-only program should succeed:\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    project
}

#[test]
fn macos_app_io_input_only_program_builds() {
    let project = build_app("macos_app_input", INPUT_ONLY_SOURCE);
    let _ = fs::remove_dir_all(&project);
}

/// The emitted binary must actually bind the probes as undefined imports —
/// asserting on the build's success alone would still pass if a future change
/// dodged the import by altering the shared readLine body (explicitly forbidden
/// by the bug's non-goals).
#[test]
fn macos_app_io_input_binds_terminal_probe_symbols() {
    let project = build_app("macos_app_input_syms", INPUT_ONLY_SOURCE);
    // plan-46-D §4.1: every build emits into the project's `build/` directory.
    let exe = project.join("build/macos_app_input_syms.app/Contents/MacOS/macos_app_input_syms");
    assert!(
        exe.is_file(),
        "expected app executable at {}",
        exe.display()
    );

    let nm = Command::new("nm")
        .arg("-u")
        .arg(&exe)
        .output()
        .expect("run nm -u");
    assert!(nm.status.success(), "nm -u failed on {}", exe.display());
    let undefined = String::from_utf8_lossy(&nm.stdout);
    let bound: Vec<&str> = undefined.lines().map(str::trim).collect();

    for probe in TERMINAL_PROBES {
        assert!(
            bound.contains(probe),
            "{probe} should be an undefined import of the app binary; got:\n{undefined}"
        );
        assert_eq!(
            bound.iter().filter(|s| *s == probe).count(),
            1,
            "{probe} should be imported exactly once (no duplicate import entries)"
        );
    }

    let _ = fs::remove_dir_all(&project);
}
