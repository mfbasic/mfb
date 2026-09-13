//! `mfb info <binary>` against executables `mfb build` produces for every target,
//! and against files that are not MFBasic binaries.
//!
//! The signed half — a chain that verifies against a pinned registry key, one
//! that cannot reach it, and a forged claim — needs a registry, and lives in
//! `cli_repo_publish`.

use std::path::Path;
use std::process::Command;

#[path = "../common/mod.rs"]
mod common;
use common::*;

const SOURCE: &str = "FUNC main AS Integer\n  RETURN 0\nEND FUNC\n";

fn info(args: &[&str]) -> (Option<i32>, String, String) {
    let output = Command::new(mfb_exe())
        .arg("info")
        .args(args)
        .output()
        .expect("run mfb info");
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn build(project: &Path, target: &str) {
    let output = Command::new(mfb_exe())
        .args(["build", "-q", "-target", target])
        .arg(project)
        .output()
        .expect("run mfb build");
    assert!(
        output.status.success(),
        "mfb build -target {target} failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

/// Every target's executable reports its format, architecture and this
/// compiler's version, and — built without `--sign` — `Signed: no`.
#[test]
fn every_target_executable_reports_what_the_linker_recorded() {
    let version = env!("CARGO_PKG_VERSION");
    for (target, file, format, arch) in [
        ("linux-x86_64", "info_probe-glibc.out", "ELF", "x86-64"),
        ("linux-x86_64", "info_probe-musl.out", "ELF", "x86-64"),
        ("linux-aarch64", "info_probe-glibc.out", "ELF", "aarch64"),
        ("linux-riscv64", "info_probe-glibc.out", "ELF", "riscv64"),
        ("macos-aarch64", "info_probe.out", "Mach-O", "aarch64"),
        ("windows-x86_64", "info_probe.exe", "PE", "x86-64"),
    ] {
        let project = temp_project("info_probe", SOURCE);
        build(&project, target);
        let binary = project.join("build").join(file);
        let (code, stdout, stderr) = info(&[binary.to_str().unwrap()]);
        assert_eq!(code, Some(0), "{target}: {stderr}");
        let lines = stdout.lines().collect::<Vec<_>>();
        let expected_head = [
            format!("File: {}", binary.display()),
            format!("Format: {format}"),
            format!("Architecture: {arch}"),
        ];
        assert!(lines.len() >= 6, "{target}:\n{stdout}");
        assert_eq!(
            lines[..3],
            expected_head.each_ref().map(String::as_str),
            "{target}:\n{stdout}"
        );
        let mut rest = &lines[3..];
        if format == "ELF" {
            assert!(rest[0].starts_with("Linking: "), "{target}:\n{stdout}");
            rest = &rest[1..];
        }
        // The libraries section: `Libraries: none`, or a header and one indented
        // name per line, optionally followed by search paths the same way.
        assert!(rest[0].starts_with("Libraries:"), "{target}:\n{stdout}");
        let compiler_at = rest
            .iter()
            .position(|line| line.starts_with("Compiler: "))
            .unwrap_or_else(|| panic!("{target}: no Compiler line:\n{stdout}"));
        assert_eq!(
            &rest[compiler_at..],
            [format!("Compiler: mfb {version}").as_str(), "Signed: no"],
            "{target}:\n{stdout}"
        );
        let _ = std::fs::remove_dir_all(&project);
    }
}

/// A file that is not an MFBasic binary — text, or a native executable some
/// other toolchain linked — says so in one line and exits 0.
#[test]
fn a_foreign_file_is_not_a_mfbasic_binary() {
    let this_test = std::env::current_exe().expect("test binary path");
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    for path in [this_test, manifest] {
        let (code, stdout, stderr) = info(&[path.to_str().unwrap()]);
        assert_eq!(code, Some(0), "{}: {stderr}", path.display());
        assert_eq!(stdout, "Not a MFBasic binary\n", "{}", path.display());
    }
}

#[test]
fn an_unreadable_file_is_reported_and_exits_zero() {
    let (code, stdout, stderr) = info(&["/definitely/not/a/real/binary.out"]);
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "");
    assert!(stderr.starts_with("error: failed to read '"), "{stderr}");
}

#[test]
fn info_takes_exactly_one_binary() {
    for args in [&[][..], &["a", "b"]] {
        let (code, _, stderr) = info(args);
        assert_eq!(code, Some(2), "{args:?}: {stderr}");
        assert!(
            stderr.contains("mfb info accepts exactly one <binary>"),
            "{stderr}"
        );
    }
}
