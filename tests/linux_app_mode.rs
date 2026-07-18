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
fn linux_app_mode_emits_a_single_sealed_appimage() {
    // plan-51-C: `--app` emits one artifact — `build/<name>.AppImage` — matching
    // macOS `--app`'s single `.app`, and the intermediate AppDir is gone.
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
        "app mode is glibc-only and emits a single artifact, got: {written:?}"
    );
    let path = PathBuf::from(written[0]);
    assert_eq!(
        path.file_name().and_then(|n| n.to_str()),
        Some("linux_app_exe.AppImage"),
        "app mode emits <name>.AppImage, got {}",
        path.display()
    );
    assert!(
        !project.join("build/linux_app_exe.AppDir").exists(),
        "a plain --app build leaves no AppDir behind (plan-51-C §3.3)"
    );
    assert!(
        !project.join("build/linux_app_exe.out").exists(),
        "the pre-plan-51 bare <name>.out must be gone"
    );

    let bytes = fs::read(&path).expect("read AppImage");
    assert_eq!(&bytes[0..4], b"\x7fELF", "the runtime is an ELF image");
    // The magic external tools key off: hex 0x414902 at offset 8.
    assert_eq!(&bytes[8..11], b"AI\x02", "AppImage type-2 magic");

    // A valid squashfs superblock begins at the runtime's exact length, with no
    // padding — padding would be read as the superblock and fail the mount.
    let offset = squashfs_offset(&bytes);
    assert_eq!(&bytes[offset..offset + 4], b"hsqs", "squashfs magic");
    // And the inner ELF's GTK dependencies are inside the (uncompressed) image.
    for library in [b"libgtk-4.so.1".as_slice(), b"libgio-2.0.so.0".as_slice()] {
        assert!(
            bytes[offset..]
                .windows(library.len())
                .any(|window| window == library),
            "the sealed payload should record {} as DT_NEEDED",
            String::from_utf8_lossy(library)
        );
    }
}

/// The offset the AppImage runtime looks for its squashfs at: the end of its own
/// ELF, which for every published runtime equals the blob's length. Recomputed
/// here from the file rather than hardcoded so a blob bump does not silently
/// invalidate the test.
fn squashfs_offset(bytes: &[u8]) -> usize {
    let u16_at = |at: usize| u16::from_le_bytes(bytes[at..at + 2].try_into().unwrap()) as usize;
    let u64_at = |at: usize| u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap()) as usize;
    let shoff = u64_at(0x28);
    let shentsize = u16_at(0x3A);
    let shnum = u16_at(0x3C);
    let mut end = shoff + shentsize * shnum;
    for index in 0..shnum {
        let header = shoff + index * shentsize;
        let sh_type = u32::from_le_bytes(bytes[header + 4..header + 8].try_into().unwrap());
        if sh_type == 8 {
            continue; // SHT_NOBITS occupies no file space
        }
        end = end.max(u64_at(header + 0x18) + u64_at(header + 0x20));
    }
    end
}

#[test]
fn linux_app_debug_keeps_the_appdir_beside_the_appimage() {
    // plan-51-C §4.7: `--app-debug` implies `--app` and retains the payload the
    // seal consumed, so the AppDir can be inspected. plan-51-A §4.1's full layout
    // must be there.
    let project = temp_project("linux_app_dbg", APP_SOURCE);
    let (ok, stdout, stderr) = run_mfb(&project, &["--app-debug", "-target", TARGET]);
    assert!(ok, "build --app-debug failed:\n{stdout}\n{stderr}");

    let appimage = project.join("build/linux_app_dbg.AppImage");
    let appdir = project.join("build/linux_app_dbg.AppDir");
    assert!(appimage.is_file(), "--app-debug still emits the AppImage");
    assert!(appdir.is_dir(), "--app-debug keeps the AppDir");

    // Every path plan-51-A §4.1 promises.
    assert!(appdir.join("usr/bin/linux_app_dbg").is_file());
    assert!(appdir.join("linux_app_dbg.desktop").is_file());
    assert!(appdir
        .join("usr/share/applications/linux_app_dbg.desktop")
        .is_file());
    assert!(appdir.join("linux_app_dbg.png").is_file());
    for size in [16, 32, 48, 64, 128, 256, 512] {
        assert!(
            appdir
                .join(format!(
                    "usr/share/icons/hicolor/{size}x{size}/apps/linux_app_dbg.png"
                ))
                .is_file(),
            "missing the {size}x{size} hicolor icon"
        );
    }
    assert_eq!(
        fs::read_link(appdir.join("AppRun")).expect("AppRun is a symlink"),
        Path::new("usr/bin/linux_app_dbg"),
        "AppRun must be a symlink to the real ELF, not a second copy of it"
    );
    assert_eq!(
        fs::read_link(appdir.join(".DirIcon")).expect(".DirIcon is a symlink"),
        Path::new("linux_app_dbg.png")
    );
    // A non-vendoring build carries no empty usr/lib/.
    assert!(!appdir.join("usr/lib").exists());

    let desktop = fs::read_to_string(appdir.join("linux_app_dbg.desktop")).expect("desktop");
    assert!(desktop.contains("\nType=Application\n"), "{desktop}");
    assert!(
        desktop.contains("\nIcon=linux_app_dbg\n"),
        "Icon= must be extension-less: appimagetool appends `.png` itself\n{desktop}"
    );
    assert!(
        desktop.contains("\nStartupWMClass=dev.mfbasic.linux_app_dbg\n"),
        "StartupWMClass must equal the GTK app id\n{desktop}"
    );
    assert!(
        desktop.contains("\nX-AppImage-Version=0.1.0\n"),
        "the manifest version reaches the .desktop\n{desktop}"
    );
    assert!(!desktop.contains("Terminal="), "{desktop}");

    // The inner ELF is what got sealed.
    let elf = fs::read(appdir.join("usr/bin/linux_app_dbg")).expect("read ELF");
    assert_eq!(&elf[0..4], b"\x7fELF");
    let sealed = fs::read(&appimage).expect("read AppImage");
    let offset = squashfs_offset(&sealed);
    assert!(
        sealed[offset..]
            .windows(elf.len())
            .any(|window| window == elf),
        "the AppDir's ELF must appear verbatim inside the uncompressed image"
    );
}

#[test]
fn linux_app_mode_carries_the_per_project_gtk_identity() {
    // plan-51-A §4.5: the GApplication id and the window title were compile-time
    // constants shared by every MFBASIC app; they are now derived from the
    // project name, which is what lets a `.desktop` file find the window.
    let project = temp_project("linux_app_id", APP_SOURCE);
    let (ok, stdout, stderr) = run_mfb(&project, &["-app", "-target", TARGET, "-ncode"]);
    assert!(ok, "build -app -ncode failed:\n{stdout}\n{stderr}");
    let ncode = fs::read_to_string(project.join("linux_app_id.ncode")).expect("read ncode");

    let hex = |text: &str| -> String {
        let mut out = String::new();
        for byte in text.bytes() {
            out.push_str(&format!("{byte:02x}"));
        }
        out.push_str("00");
        out
    };
    assert!(
        ncode.contains(&hex("dev.mfbasic.linux_app_id")),
        "the app id must be namespaced under the project name"
    );
    assert!(
        !ncode.contains(&hex("dev.mfbasic.app")),
        "the shared pre-plan-51 app id must be gone"
    );
    assert!(
        !ncode.contains(&hex("MFBASIC App")),
        "the shared pre-plan-51 window title must be gone"
    );
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
