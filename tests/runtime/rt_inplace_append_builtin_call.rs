//! bug-626: `list = collections::append(list, <builtin call>)` appends in place.
//!
//! `try_inplace_append_assign` takes the in-place path only when
//! `static_item_type` names the appended value's type and it equals the list's
//! element type (gate G11). A builtin call's type came only from
//! `static_type_name`'s hand-written table of a dozen names, so
//! `append(keep, fs::readText(path))` answered `None`, declined, and took the
//! general path — one allocation as large as the whole list on every append.
//! Output is identical either way; only the allocated bytes show it.
//!
//! **The measure.** The main arena's `alloc_bytes` (every byte ever requested,
//! freed or not) at `N` and `2N` appends of a 5,000-byte file. In place, the
//! list's own geometric growth plus one payload read per append doubles with
//! `N` (the bound `LET` form measured ×2.26 for 100 → 200). The copying path
//! allocates Σ i × 5,000 B, which measured ×3.90. The ×3 bound sits between.

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

const PAYLOAD_BYTES: usize = 5_000;
const N: u64 = 100;

/// Build `source` with `--debug` and return the executable (the host glibc one on
/// Linux), as `rt_list_append_growth_bounds` does.
fn build_debug(name: &str, source: &str) -> PathBuf {
    let project = common::temp_project(name, source);
    let output = Command::new(common::mfb_exe())
        .arg("build")
        .arg("--debug")
        .arg(&project)
        .output()
        .expect("run mfb build --debug");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success(),
        "{name} failed to build:\n{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let written: Vec<&str> = stdout
        .lines()
        .filter_map(|line| line.strip_prefix("Wrote executable to "))
        .collect();
    let chosen = written
        .iter()
        .find(|path| path.ends_with("-glibc.out"))
        .or_else(|| written.first())
        .unwrap_or_else(|| panic!("{name}: no executable in build output:\n{stdout}"));
    PathBuf::from(chosen)
}

/// Run a `--debug` program, require it to print `expected`, and return the main
/// arena's `alloc_bytes`.
fn alloc_bytes(name: &str, exe: &Path, expected: &str) -> u64 {
    let output = Command::new(exe).output().expect("run the program");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "{name} failed:\n{stdout}\n{stderr}"
    );
    assert_eq!(
        stdout.lines().next(),
        Some(expected),
        "{name}: wrong output"
    );
    stderr
        .lines()
        .find_map(|line| line.strip_prefix("arena.0.alloc_bytes "))
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| panic!("{name}: no arena.0.alloc_bytes in:\n{stderr}"))
}

/// A 5,000-byte input file, printable ASCII so `fs::readText` accepts it.
fn payload_file(tag: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("mfb_{tag}_{}.txt", std::process::id()));
    let bytes: Vec<u8> = (0..PAYLOAD_BYTES).map(|i| b'a' + (i % 26) as u8).collect();
    std::fs::write(&path, bytes).expect("write payload file");
    path
}

/// `n` appends of `call` (a builtin reading `path`) into a `MUT` list of `element`.
fn program(element: &str, call: &str, path: &Path, n: u64) -> String {
    format!(
        "IMPORT collections\nIMPORT fs\nIMPORT io\n\n\
         FUNC main AS Integer\n  \
         MUT keep AS List OF {element} = []\n  \
         FOR i = 1 TO {n}\n    \
         keep = collections::append(keep, {call}(\"{}\"))\n  \
         NEXT\n  \
         io::print(toString(len(keep)))\n  \
         RETURN 0\n\
         END FUNC\n",
        path.display()
    )
}

fn assert_linear(tag: &str, element: &str, call: &str) {
    let path = payload_file(tag);
    let measure = |n: u64| {
        let name = format!("{tag}_{n}");
        let exe = build_debug(&name, &program(element, call, &path, n));
        alloc_bytes(&name, &exe, &n.to_string())
    };
    let once = measure(N);
    let twice = measure(2 * N);
    let _ = std::fs::remove_file(&path);
    assert!(
        twice * 10 <= once * 30,
        "bug-626: `keep = collections::append(keep, {call}(…))` into a List OF {element} \
         requested {once} bytes for {N} appends and {twice} for {}: ×{:.2}. An in-place \
         append doubles (×≤3); a copy of the whole list per append quadruples.",
        2 * N,
        twice as f64 / once as f64
    );
}

#[test]
fn appending_fs_read_text_into_a_string_list_is_in_place() {
    assert_linear("b626_read_text", "String", "fs::readText");
}

#[test]
fn appending_fs_read_bytes_into_a_nested_byte_list_is_in_place() {
    assert_linear("b626_read_bytes", "List OF Byte", "fs::readBytes");
}
