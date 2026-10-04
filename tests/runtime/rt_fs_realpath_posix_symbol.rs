//! bug-800: `fs::canonicalPath` / `fs::isWithin` / `fs::openWithin` call libSystem's
//! POSIX `realpath` (`_realpath$DARWIN_EXTSN`, the symbol `<stdlib.h>` binds), not
//! the bare legacy `_realpath`.
//!
//! The legacy entry point resolves a relative path through a `getcwd` that scans
//! every parent directory. With the working directory under a `$TMPDIR` holding
//! 353 832 entries, one `fs::canonicalPath(".")` cost ~0.17 s of system time (50
//! calls: 8.4 s), and the man-examples gate's `canonicalPath`/`isWithin` examples
//! timed out at 60 s. The POSIX entry point answers the same call in microseconds and,
//! with a caller-supplied buffer, returns, fails and sets `errno` identically.
//!
//! The timing depends on the machine's `$TMPDIR`, so the test pins the cause: the
//! imported symbol.

#![cfg(target_os = "macos")]

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

const SOURCE: &str = "\
IMPORT io
IMPORT fs

FUNC main() AS Integer
  io::print(fs::canonicalPath(\".\"))
  io::print(toString(fs::isWithin(\".\", \".\")))
  RETURN 0
END FUNC
";

#[test]
fn realpath_users_import_the_posix_entry_point() {
    let project = common::temp_project("fs_realpath_symbol", SOURCE);
    let built = Command::new(common::mfb_exe())
        .arg("build")
        .arg(&project)
        .output()
        .expect("run mfb build");
    assert!(
        built.status.success(),
        "build failed:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let executable = project.join("build").join("fs_realpath_symbol.out");
    let undefined = Command::new("nm")
        .arg("-u")
        .arg(&executable)
        .output()
        .expect("run nm");
    let undefined = String::from_utf8_lossy(&undefined.stdout);
    // A set: the import table lists a symbol once per importing helper.
    let realpaths: std::collections::BTreeSet<&str> = undefined
        .lines()
        .map(str::trim)
        .filter(|symbol| symbol.starts_with("_realpath"))
        .collect();
    assert_eq!(
        realpaths.into_iter().collect::<Vec<_>>(),
        ["_realpath$DARWIN_EXTSN"],
        "nm -u:\n{undefined}"
    );

    let run = Command::new(&executable)
        .current_dir(&project)
        .output()
        .expect("run the probe");
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(run.status.success(), "probe failed:\n{stdout}");
    let canonical = std::fs::canonicalize(&project).expect("canonicalize the project");
    assert_eq!(
        stdout.lines().collect::<Vec<_>>(),
        [canonical.to_str().expect("utf-8 path"), "TRUE"]
    );
}
