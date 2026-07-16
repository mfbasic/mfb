//! Regression test for bug-186 (LNK-01): Linux executables must be emitted as
//! position-independent (`ET_DYN`) with a load base of 0 and a `PT_GNU_STACK`
//! (non-executable stack) program header, on every Linux arch. Before the fix
//! they were `ET_EXEC` at a fixed `0x400000`, so the main image (code, data, GOT)
//! loaded at the same address on every run — no ASLR slide for an attacker's ROP
//! or GOT-overwrite. macOS was already PIE; this brings Linux in line.
//!
//! The ELF header is inspected directly (no Linux host needed); runtime PIE
//! behavior and ASLR randomization are validated on the Linux remotes.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const SOURCE: &str =
    "IMPORT io\n\nFUNC main AS Integer\n  io::print(\"pie\")\n  RETURN 0\nEND FUNC\n";

const PT_GNU_STACK: u32 = 0x6474_e551;

fn temp_project(name: &str) -> PathBuf {
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
    fs::write(root.join("src/main.mfb"), SOURCE).expect("write source");
    root
}

fn build_linux_elf(project: &Path, target: &str, name: &str) -> Vec<u8> {
    let output = Command::new(env!("CARGO_BIN_EXE_mfb"))
        .arg("build")
        .arg("-q")
        .arg("-target")
        .arg(target)
        .arg(project)
        .output()
        .expect("run mfb build");
    assert!(
        output.status.success(),
        "mfb build -target {target} failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    // Console builds emit one flavored executable per libc world; either is fine
    // for a header check (they share the ELF layout).
    let glibc = project.join(format!("{name}-glibc.out"));
    let musl = project.join(format!("{name}-musl.out"));
    let path = if glibc.exists() { glibc } else { musl };
    fs::read(&path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()))
}

fn u16le(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

fn u64le(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap())
}

fn has_gnu_stack(bytes: &[u8]) -> bool {
    let phoff = u64le(bytes, 32) as usize;
    let phnum = u16le(bytes, 56) as usize;
    (0..phnum).any(|i| {
        let base = phoff + i * 56;
        u32::from_le_bytes(bytes[base..base + 4].try_into().unwrap()) == PT_GNU_STACK
    })
}

fn assert_pie(target: &str) {
    let name = format!("pie_hdr_{}", target.replace('-', "_"));
    let project = temp_project(&name);
    let bytes = build_linux_elf(&project, target, &name);
    assert_eq!(&bytes[0..4], b"\x7fELF", "{target}: not an ELF image");
    // e_type == ET_DYN (3): a position-independent executable.
    assert_eq!(
        u16le(&bytes, 16),
        3,
        "{target}: e_type must be ET_DYN (PIE)"
    );
    // Base 0: the entry point is a small file-relative address, not 0x400000+.
    let entry = u64le(&bytes, 24);
    assert!(
        entry < 0x10_000,
        "{target}: entry {entry:#x} is not base-0 (PIE)",
    );
    assert!(
        has_gnu_stack(&bytes),
        "{target}: PT_GNU_STACK must be present"
    );
    let _ = fs::remove_dir_all(&project);
}

#[test]
fn linux_aarch64_is_pie_with_gnu_stack() {
    assert_pie("linux-aarch64");
}

#[test]
fn linux_x86_64_is_pie_with_gnu_stack() {
    assert_pie("linux-x86_64");
}

#[test]
fn linux_riscv64_is_pie_with_gnu_stack() {
    assert_pie("linux-riscv64");
}
