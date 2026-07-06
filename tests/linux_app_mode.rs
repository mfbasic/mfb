//! Linux GTK4 app-mode build regression tests (plan-05-linux-app.md).
//!
//! These drive the real `mfb` CLI for a `linux-aarch64` target and inspect the
//! produced artifacts. They never execute the produced ELF (the dev/CI host is
//! macOS and cannot run a Linux+GTK aarch64 binary; see plan-05 §9), so they lock
//! the cross-compilation behavior — build mode, GTK import surface, single glibc
//! output flavor — rather than runtime behavior.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const TARGET: &str = "linux-aarch64";

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

fn run_mfb(project: &Path, args: &[&str]) -> (bool, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_mfb"))
        .arg("build")
        .args(args)
        .arg(project)
        .output()
        .expect("run mfb build");
    (
        output.status.success(),
        String::from_utf8(output.stdout).expect("utf8 stdout"),
        String::from_utf8(output.stderr).expect("utf8 stderr"),
    )
}

const APP_SOURCE: &str = "IMPORT io\n\nSUB main()\n  io::print(\"App mode started\")\n  LET name AS String = io::readLine()\n  io::print(\"Hello, \" & name)\nEND SUB\n";

#[test]
fn linux_app_mode_nir_records_build_mode() {
    let project = temp_project("linux_app_nir", APP_SOURCE);
    let (ok, stdout, stderr) = run_mfb(&project, &["-app", "-target", TARGET, "-nir"]);
    assert!(ok, "build -app -nir failed:\n{stdout}\n{stderr}");
    let nir = fs::read_to_string(project.join("linux_app_nir.nir")).expect("read nir");
    assert!(
        nir.contains("\"buildMode\": \"linux-app\""),
        "NIR should record the linux-app build mode, got:\n{nir}"
    );
}

#[test]
fn linux_app_mode_plan_declares_gtk_libraries() {
    let project = temp_project("linux_app_nplan", APP_SOURCE);
    let (ok, stdout, stderr) = run_mfb(&project, &["-app", "-target", TARGET, "-nplan"]);
    assert!(ok, "build -app -nplan failed:\n{stdout}\n{stderr}");
    let nplan = fs::read_to_string(project.join("linux_app_nplan.nplan")).expect("read nplan");
    for library in [
        "libgtk-4.so.1",
        "libgobject-2.0.so.0",
        "libglib-2.0.so.0",
        "libgio-2.0.so.0",
    ] {
        assert!(
            nplan.contains(library),
            "nplan should declare {library} as a GTK app-mode dependency"
        );
    }
    for symbol in [
        "gtk_application_new",
        "g_application_run",
        "g_signal_connect_data",
    ] {
        assert!(
            nplan.contains(symbol),
            "nplan should import the GTK bootstrap symbol {symbol}"
        );
    }
    // App mode omits the console SIGINT/SIGTERM handler import (plan-05 §6.1).
    assert!(
        !nplan.contains("\"signal\""),
        "app mode should not import the console signal handler"
    );
}

#[test]
fn linux_app_mode_emits_single_glibc_executable() {
    let project = temp_project("linux_app_exe", APP_SOURCE);
    let (ok, stdout, stderr) = run_mfb(&project, &["-app", "-target", TARGET]);
    assert!(ok, "build -app failed:\n{stdout}\n{stderr}");
    let written: Vec<&str> = stdout
        .lines()
        .filter_map(|line| line.strip_prefix("Wrote executable to "))
        .collect();
    assert_eq!(
        written.len(),
        1,
        "app mode is glibc-only and emits a single executable, got: {written:?}"
    );
    let path = PathBuf::from(written[0]);
    assert!(
        path.file_name().and_then(|n| n.to_str()) == Some("linux_app_exe.out"),
        "app executable should be <name>.out with no flavor suffix, got {}",
        path.display()
    );
    let bytes = fs::read(&path).expect("read app executable");
    assert_eq!(&bytes[0..4], b"\x7fELF", "output should be an ELF image");
    for library in [b"libgtk-4.so.1".as_slice(), b"libgio-2.0.so.0".as_slice()] {
        assert!(
            bytes.windows(library.len()).any(|window| window == library),
            "linked ELF should record {} as DT_NEEDED",
            String::from_utf8_lossy(library)
        );
    }
}

#[test]
fn linux_console_mode_still_emits_both_flavors() {
    let project = temp_project("linux_console", APP_SOURCE);
    let (ok, stdout, stderr) = run_mfb(&project, &["-target", TARGET]);
    assert!(ok, "console build failed:\n{stdout}\n{stderr}");
    let written: Vec<&str> = stdout
        .lines()
        .filter_map(|line| line.strip_prefix("Wrote executable to "))
        .collect();
    assert_eq!(
        written.len(),
        2,
        "console mode emits glibc + musl flavors, got: {written:?}"
    );
    assert!(
        written
            .iter()
            .any(|p| p.ends_with("linux_console-glibc.out"))
            && written
                .iter()
                .any(|p| p.ends_with("linux_console-musl.out")),
        "console mode should emit -glibc.out and -musl.out, got: {written:?}"
    );
}
