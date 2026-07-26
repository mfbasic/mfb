//! Regression test for bug-186 (LNK-01): Linux executables must be emitted as
//! position-independent (`ET_DYN`) with a load base of 0 and a `PT_GNU_STACK`
//! (non-executable stack) program header, on every Linux arch. Before the fix
//! they were `ET_EXEC` at a fixed `0x400000`, so the main image (code, data, GOT)
//! loaded at the same address on every run — no ASLR slide for an attacker's ROP
//! or GOT-overwrite. macOS was already PIE; this brings Linux in line.
//!
//! The ELF header is inspected directly (no Linux host needed); runtime PIE
//! behavior and ASLR randomization are validated on the Linux remotes.

mod common;
use common::build_linux_elf;
use std::fs;
use std::path::PathBuf;

const SOURCE: &str =
    "IMPORT io\n\nFUNC main AS Integer\n  io::print(\"pie\")\n  RETURN 0\nEND FUNC\n";

const PT_GNU_STACK: u32 = 0x6474_e551;

fn temp_project(name: &str) -> PathBuf {
    common::temp_project(name, SOURCE)
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
