use std::path::{Path, PathBuf};

use super::{check_debuggable, Program};
use crate::arch::image::{EncodedImage, EncodedImport, EncodedSection, EncodedSymbol, ImportKind};
use crate::os::linux::flavor::LinuxFlavor;

fn image(text: Vec<u8>, entry: &str) -> EncodedImage {
    EncodedImage {
        text,
        data: Vec::new(),
        rodata_size: 0,
        symbols: vec![EncodedSymbol {
            name: entry.to_string(),
            section: EncodedSection::Text,
            offset: 0,
        }],
        relocations: Vec::new(),
        imports: Vec::new(),
        entry: entry.to_string(),
        initializers: Vec::new(),
        signing_metadata: None,
        rpaths: Vec::new(),
        debug_symbols: false,
    }
}

/// One tiny MFBASIC executable per container format and architecture the
/// in-tree linkers write, as `(os, arch, path)` with `arch` spelled as
/// `std::env::consts::ARCH` spells it. Nothing runs them; only their headers and
/// provenance notes are read.
pub(crate) fn linked_probes(dir: &Path) -> Vec<(&'static str, &'static str, PathBuf)> {
    let x86_ret = vec![0xc3];
    let arm_ret = vec![0xc0, 0x03, 0x5f, 0xd6];
    let riscv_ret = vec![0x67, 0x80, 0x00, 0x00];
    let linux = |name: &str, arch: &str, text: Vec<u8>| {
        crate::os::linux::write_linked_executable(
            &dir.join(name),
            "probe",
            arch,
            LinuxFlavor::Glibc,
            &image(text, "_start"),
        )
        .expect("link ELF")
    };
    vec![
        (
            "linux",
            "x86_64",
            linux("elf-x86", "x86_64", x86_ret.clone()),
        ),
        (
            "linux",
            "aarch64",
            linux("elf-arm", "aarch64", arm_ret.clone()),
        ),
        ("linux", "riscv64", linux("elf-riscv", "riscv64", riscv_ret)),
        (
            "windows",
            "x86_64",
            crate::os::windows::write_linked_executable(
                &dir.join("pe"),
                "probe",
                &image(x86_ret, "_start"),
                false,
                None,
                None,
            )
            .expect("link PE"),
        ),
        (
            "macos",
            "aarch64",
            crate::os::macos::write_linked_executable(
                &dir.join("macho"),
                "probe",
                &image(arm_ret, "_main"),
            )
            .expect("link Mach-O"),
        ),
    ]
}

/// The probe this host runs natively.
pub(crate) fn native_probe(dir: &Path) -> PathBuf {
    linked_probes(dir)
        .into_iter()
        .find(|(os, arch, _)| (*os, *arch) == (std::env::consts::OS, std::env::consts::ARCH))
        .map(|(_, _, path)| path)
        .expect("the in-tree linkers write an executable for this host")
}

#[test]
fn only_the_host_s_own_format_and_architecture_are_debuggable() {
    let dir = tempfile::tempdir().unwrap();
    let host = (std::env::consts::OS, std::env::consts::ARCH);
    for (os, arch, path) in linked_probes(dir.path()) {
        let verdict = check_debuggable(&path);
        if (os, arch) == host {
            assert_eq!(verdict, Ok(Program { app: false }), "{os}-{arch}");
        } else {
            let message = verdict.expect_err(&format!("{os}-{arch} is not native"));
            assert!(
                message.starts_with(&format!("'{}' is built for ", path.display()))
                    && message.contains("; this debugger runs "),
                "{os}-{arch}: {message}"
            );
        }
    }
}

#[test]
fn a_mismatch_names_both_the_file_s_and_the_host_s_format() {
    let dir = tempfile::tempdir().unwrap();
    let probes = linked_probes(dir.path());
    // Some probe is foreign on every host; the PE and Mach-O never share one.
    let (_, _, foreign) = probes
        .iter()
        .find(|(os, _, _)| *os != std::env::consts::OS)
        .unwrap();
    let message = check_debuggable(foreign).unwrap_err();
    let (format, arch) = super::host_image();
    assert!(
        message.ends_with(&format!("this debugger runs {format} {arch} programs")),
        "{message}"
    );
}

#[test]
fn a_file_without_the_provenance_note_is_not_an_mfbasic_executable() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("plain.txt");
    std::fs::write(&path, "not a program").unwrap();
    assert_eq!(
        check_debuggable(&path),
        Err(format!("'{}' is not an MFBASIC executable", path.display()))
    );
}

#[test]
fn an_unreadable_path_says_why() {
    let dir = tempfile::tempdir().unwrap();
    let message = check_debuggable(dir.path()).unwrap_err();
    assert!(
        message.starts_with(&format!("cannot read '{}': ", dir.path().display())),
        "{message}"
    );
}

#[test]
fn a_path_with_control_codes_is_printed_escaped() {
    let shown = super::shown(Path::new("/tmp/a\u{1b}[2Jb"));
    assert!(!shown.contains('\u{1b}'), "{shown:?}");
}

#[test]
fn the_host_image_matches_how_inspect_spells_formats_and_architectures() {
    let (format, arch) = super::host_image();
    let expected = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => ("Mach-O", "aarch64"),
        ("linux", "x86_64") => ("ELF", "x86-64"),
        ("linux", "aarch64") => ("ELF", "aarch64"),
        ("linux", "riscv64") => ("ELF", "riscv64"),
        ("windows", "x86_64") => ("PE", "x86-64"),
        other => panic!("no MFBASIC target for host {other:?}"),
    };
    assert_eq!((format, arch), expected);
}

#[test]
fn the_backend_exists_exactly_where_mfb_dbg_is_supported() {
    let host = (std::env::consts::OS, std::env::consts::ARCH);
    assert_eq!(
        super::host_backend().is_some(),
        host.0 == "macos" || host == ("linux", "aarch64")
    );
}

/// A program that loads AppKit is an app: its initial thread runs the window
/// system, not `main`.
#[test]
fn a_program_that_loads_appkit_is_an_app() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = image(vec![0xc0, 0x03, 0x5f, 0xd6], "_main");
    app.imports.push(EncodedImport {
        library: "AppKit".to_string(),
        symbol: "_NSApplicationMain".to_string(),
        kind: ImportKind::Function,
        version: None,
    });
    let path = crate::os::macos::write_linked_executable(dir.path(), "probe", &app).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let binary = crate::os::inspect::inspect(&bytes).unwrap();
    assert!(binary
        .libraries
        .iter()
        .any(|library| library.contains("AppKit.framework")));
    if (std::env::consts::OS, std::env::consts::ARCH) == ("macos", "aarch64") {
        assert_eq!(check_debuggable(&path), Ok(Program { app: true }));
    }
}
