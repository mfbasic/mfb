//! `fs::pathJoin` / `fs::pathNormalize` return a String block of exactly the size
//! it is freed at.
//!
//! A `String` block is `byteLength + 9` bytes and is freed at that size (`mfb spec
//! memory heap-values`). `pathJoin` allocated `sum(parts) + count + 9` and
//! `pathNormalize` `input + 10`, so every call whose result came out shorter than
//! that — a joined part that already ends in a separator, a `.` / `..` segment
//! normalized away — was freed short and lost the difference to the arena:
//! `alloc_calls == free_calls` yet `live_bytes` grew with the call count. Found
//! while checking bug-768's reader probe for `live_bytes 0`.

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

const SOURCE: &str = "\
IMPORT io
IMPORT fs

FUNC main() AS Integer
  MUT total AS Integer = 0
  FOR i = 1 TO 500
    LET a AS String = fs::pathJoin([\"base/\", \"dir/\", \"file\" & toString(i)])
    LET b AS String = fs::pathNormalize(\"a/./b/../c/./d\" & toString(i))
    LET c AS String = fs::pathJoin([fs::tempDirectory(), \"name\" & toString(i)])
    total = total + len(a) + len(b) + len(c)
  NEXT
  io::print(toString(total > 0))
  RETURN 0
END FUNC
";

#[test]
fn path_join_and_normalize_free_exactly_what_they_allocate() {
    let project = common::temp_project("fs_path_exact_size", SOURCE);
    let output = Command::new(common::mfb_exe())
        .arg("build")
        .arg("--debug")
        .arg(&project)
        .output()
        .expect("run mfb build");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(output.status.success(), "build failed:\n{stdout}");
    let exe = stdout
        .lines()
        .filter_map(|line| line.strip_prefix("Wrote executable to "))
        .find(|p| p.ends_with(common::host_executable_suffix()))
        .or_else(|| {
            stdout
                .lines()
                .find_map(|line| line.strip_prefix("Wrote executable to "))
        })
        .expect("an executable")
        .to_string();
    let run = Command::new(&exe).output().expect("run the program");
    let _ = std::fs::remove_dir_all(&project);
    assert!(run.status.success());
    assert_eq!(String::from_utf8_lossy(&run.stdout), "TRUE\n");
    let report = String::from_utf8_lossy(&run.stderr);
    let live: u64 = report
        .lines()
        .filter_map(|line| line.strip_prefix("arena."))
        .filter_map(|rest| rest.split_once(".live_bytes "))
        .filter(|(arena, _)| !arena.contains('.'))
        .map(|(_, n)| n.trim().parse::<u64>().expect("a count"))
        .sum();
    assert_eq!(
        live, 0,
        "pathJoin/pathNormalize freed their result short of its allocation: {live} B lost \
         over 500 iterations"
    );
}
