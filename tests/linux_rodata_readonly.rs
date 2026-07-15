//! Regression test for bug-187 (LNK-08): a program's constant data (string
//! literals, error messages) must live in a read-only segment, not the writable
//! data segment. Before the fix, all non-code data — constants AND the mutable
//! arena global — shared one R+W `PT_LOAD`, so an arbitrary-write primitive could
//! rewrite a format string or a constant used in a security check. The encoder
//! now partitions data into a read-only constant prefix and a writable suffix
//! (arena global + runtime globals), and the Linux linker maps the prefix in its
//! own R `PT_LOAD`.
//!
//! Inspects the ELF program headers directly (no Linux host needed): a program
//! with string literals must have a data-region `PT_LOAD` that is readable but
//! NOT writable and NOT executable. Runtime behavior (constants readable, a write
//! faults) is validated on the Linux remotes.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

// A program carrying string-literal constants (so the read-only partition is
// non-empty) plus normal runtime work.
const SOURCE: &str = "IMPORT io\nIMPORT strings\n\nFUNC main AS Integer\n  LET g = strings::upper(\"hello-rodata-constant\")\n  io::print(g)\n  RETURN 0\nEND FUNC\n";

const PT_LOAD: u32 = 1;
const PF_X: u32 = 1;
const PF_W: u32 = 2;
const PF_R: u32 = 4;

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
    let glibc = project.join(format!("{name}-glibc.out"));
    let musl = project.join(format!("{name}-musl.out"));
    let path = if glibc.exists() { glibc } else { musl };
    fs::read(&path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()))
}

fn u16le(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}
fn u32le(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}
fn u64le(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}

/// Program headers as `(p_type, p_flags, p_vaddr)`.
fn program_headers(bytes: &[u8]) -> Vec<(u32, u32, u64)> {
    let phoff = u64le(bytes, 32) as usize;
    let phnum = u16le(bytes, 56) as usize;
    (0..phnum)
        .map(|i| {
            let base = phoff + i * 56;
            (
                u32le(bytes, base),
                u32le(bytes, base + 4),
                u64le(bytes, base + 16),
            )
        })
        .collect()
}

fn assert_has_readonly_data_load(target: &str) {
    let name = format!("rodata_{}", target.replace('-', "_"));
    let project = temp_project(&name);
    let bytes = build_linux_elf(&project, target, &name);
    let phdrs = program_headers(&bytes);
    // The R-X text load is lowest; a read-only data load must exist above it —
    // readable, not writable, not executable — holding the string constants.
    let has_ro_data = phdrs.iter().any(|&(kind, flags, vaddr)| {
        kind == PT_LOAD
            && vaddr > 0x1000
            && flags & PF_R != 0
            && flags & PF_W == 0
            && flags & PF_X == 0
    });
    // And the writable data load (arena global) must still exist, distinct from it.
    let has_rw_data = phdrs
        .iter()
        .any(|&(kind, flags, _)| kind == PT_LOAD && flags & PF_W != 0 && flags & PF_X == 0);
    assert!(
        has_ro_data,
        "{target}: expected a read-only (R, !W, !X) data PT_LOAD for constants; phdrs={phdrs:?}",
    );
    assert!(
        has_rw_data,
        "{target}: expected a writable data PT_LOAD for the arena global; phdrs={phdrs:?}",
    );
    let _ = fs::remove_dir_all(&project);
}

#[test]
fn linux_aarch64_constants_are_read_only() {
    assert_has_readonly_data_load("linux-aarch64");
}

#[test]
fn linux_x86_64_constants_are_read_only() {
    assert_has_readonly_data_load("linux-x86_64");
}

#[test]
fn linux_riscv64_constants_are_read_only() {
    assert_has_readonly_data_load("linux-riscv64");
}
