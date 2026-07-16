//! Regression test for bug-187 (LNK-08), macOS half: a program's constant data
//! (string literals, error messages) must live in a read-only segment, not the
//! writable `__DATA` segment. Before the fix, all non-code data — constants AND
//! the mutable arena global — shared the R+W `__DATA` segment, so an arbitrary-
//! write primitive could rewrite a format string or a constant used in a security
//! check. The encoder now routes the read-only constant prefix into a `__const`
//! section inside `__DATA_CONST` (which dyld maps read-only via `SG_READ_ONLY`
//! after fixups), leaving only the arena global and other runtime globals in the
//! writable `__DATA` segment.
//!
//! Inspects the Mach-O load commands directly (no macOS host needed): the string
//! literal bytes must reside in `__DATA_CONST` (flagged `SG_READ_ONLY`) and must
//! NOT appear in the writable `__DATA` segment. Runtime behavior (a store to a
//! constant faults with EXC_BAD_ACCESS while a store to the arena global succeeds)
//! is validated on a macOS host with lldb / `memory region`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

// A program carrying a string-literal constant (so the read-only partition is
// non-empty) plus normal runtime work. The lowercase literal below is what lands
// in the constant region; `strings::upper` produces the runtime value.
const LITERAL: &str = "hello-rodata-constant";
const SOURCE: &str = "IMPORT io\nIMPORT strings\n\nFUNC main AS Integer\n  LET g = strings::upper(\"hello-rodata-constant\")\n  io::print(g)\n  RETURN 0\nEND FUNC\n";

const MH_MAGIC_64: u32 = 0xfeed_facf;
const LC_SEGMENT_64: u32 = 0x19;
const SG_READ_ONLY: u32 = 0x10;

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

fn build_macho(project: &Path, name: &str) -> Vec<u8> {
    let output = Command::new(env!("CARGO_BIN_EXE_mfb"))
        .arg("build")
        .arg("-q")
        .arg("-target")
        .arg("macos-aarch64")
        .arg(project)
        .output()
        .expect("run mfb build");
    assert!(
        output.status.success(),
        "mfb build -target macos-aarch64 failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let path = project.join(format!("{name}.out"));
    fs::read(&path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()))
}

fn u32le(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}
fn u64le(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}

fn segname(b: &[u8], at: usize) -> String {
    let raw = &b[at..at + 16];
    let end = raw.iter().position(|&c| c == 0).unwrap_or(16);
    String::from_utf8_lossy(&raw[..end]).into_owned()
}

struct Segment {
    name: String,
    fileoff: usize,
    filesize: usize,
    flags: u32,
}

/// Parse the `LC_SEGMENT_64` load commands into `(name, fileoff, filesize, flags)`.
fn segments(bytes: &[u8]) -> Vec<Segment> {
    assert_eq!(u32le(bytes, 0), MH_MAGIC_64, "not a 64-bit Mach-O");
    let ncmds = u32le(bytes, 16) as usize;
    let mut segments = Vec::new();
    let mut cursor = 32;
    for _ in 0..ncmds {
        let cmd = u32le(bytes, cursor);
        let cmdsize = u32le(bytes, cursor + 4) as usize;
        if cmd == LC_SEGMENT_64 {
            segments.push(Segment {
                name: segname(bytes, cursor + 8),
                fileoff: u64le(bytes, cursor + 40) as usize,
                filesize: u64le(bytes, cursor + 48) as usize,
                flags: u32le(bytes, cursor + 68),
            });
        }
        cursor += cmdsize;
    }
    segments
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

#[test]
fn macos_constants_live_in_read_only_data_const() {
    let name = "rodata_macos_aarch64";
    let project = temp_project(name);
    let bytes = build_macho(&project, name);
    let segments = segments(&bytes);

    let data_const = segments
        .iter()
        .find(|segment| segment.name == "__DATA_CONST")
        .expect("a __DATA_CONST segment must exist");
    let data = segments
        .iter()
        .find(|segment| segment.name == "__DATA")
        .expect("a writable __DATA segment (arena global) must exist");

    // __DATA_CONST is marked SG_READ_ONLY, so dyld maps it read-only after fixups.
    assert!(
        data_const.flags & SG_READ_ONLY != 0,
        "__DATA_CONST must carry SG_READ_ONLY; flags={:#x}",
        data_const.flags,
    );

    let literal = LITERAL.as_bytes();
    let dc_bytes = &bytes[data_const.fileoff..data_const.fileoff + data_const.filesize];
    let data_bytes = &bytes[data.fileoff..data.fileoff + data.filesize];

    // The constant lives in the read-only __DATA_CONST region...
    assert!(
        contains(dc_bytes, literal),
        "string constant must reside in read-only __DATA_CONST",
    );
    // ...and NOT in the writable __DATA segment (which holds only the arena global
    // and other runtime globals).
    assert!(
        !contains(data_bytes, literal),
        "string constant must not reside in writable __DATA",
    );

    let _ = fs::remove_dir_all(&project);
}
