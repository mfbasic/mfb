//! Regression test for bug-184 (audit-2 OS-01): file-creating `fs` builtins must
//! create their files owner-only (mode `0o600`), not world-readable/writable
//! `0o666`. `fs::open` (with a create flag), `fs::writeText`, and `fs::writeBytes`
//! all previously passed mode `438` (`0o666`) to `open`, so with a typical
//! `umask 022` the resulting file was `0o644` — world-readable — exposing any
//! secret a program writes. The fix passes `384` (`0o600`), matching
//! `createTempFile`/`atomicWrite`.
//!
//! The mode is a runtime effect invisible to the golden harness, so this builds
//! and runs a program natively and stats the created files' permission bits.

#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_project(name: &str, out_dir: &Path) -> PathBuf {
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
    // Exercise all three file-creating paths: fs::open (create), fs::writeBytes,
    // and fs::writeText (the atomic path).
    let open_path = out_dir.join("open.txt");
    let bytes_path = out_dir.join("bytes.bin");
    let text_path = out_dir.join("text.txt");
    let source = format!(
        "IMPORT fs\nIMPORT io\n\nFUNC main AS Integer\n  RES f = fs::open(\"{open}\", \"write\")\n  LET b AS List OF Byte = [65, 66]\n  fs::writeBytes(\"{bytes}\", b)\n  fs::writeText(\"{text}\", \"secret\")\n  io::print(\"done\")\n  RETURN 0\nEND FUNC\n",
        open = open_path.display(),
        bytes = bytes_path.display(),
        text = text_path.display(),
    );
    fs::write(root.join("src/main.mfb"), source).expect("write source");
    root
}

fn mode_bits(path: &Path) -> u32 {
    fs::metadata(path)
        .unwrap_or_else(|err| panic!("stat {}: {err}", path.display()))
        .permissions()
        .mode()
        & 0o777
}

#[test]
fn file_creating_builtins_create_owner_only_0600() {
    let out_dir = std::env::temp_dir().join(format!(
        "mfb_bug184_out_{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&out_dir).expect("create output dir");
    let project = temp_project("bug184_mode", &out_dir);

    let output = Command::new(env!("CARGO_BIN_EXE_mfb"))
        .arg("build")
        .arg("-q")
        .arg(&project)
        .output()
        .expect("run mfb build");
    assert!(
        output.status.success(),
        "mfb build failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    // plan-46-D §4.1: the build emits into its own `<name>/` directory.
    let exe = project.join("bug184_mode").join("bug184_mode.out");
    let run = Command::new(&exe).output().expect("run built executable");
    assert!(
        run.status.success(),
        "program exited non-zero:\n{}\n{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr),
    );

    for name in ["open.txt", "bytes.bin", "text.txt"] {
        let path = out_dir.join(name);
        assert_eq!(
            mode_bits(&path),
            0o600,
            "{name}: file-creating builtin left mode {:o}, expected 0o600 (bug-184)",
            mode_bits(&path),
        );
    }

    let _ = fs::remove_dir_all(&project);
    let _ = fs::remove_dir_all(&out_dir);
}
