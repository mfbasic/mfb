//! bug-802: a macOS executable lists each imported libSystem symbol once.
//!
//! The native plan lists an import once per runtime helper that needs it —
//! `fs.canonicalPath` and `fs.isWithin` each list `_realpath$DARWIN_EXTSN` and
//! `___error` — and the image used to keep every copy: `nm -u` showed `___error`
//! three times and `_realpath` twice, each with its own stub and GOT slot. The
//! encoder now keeps the first of each `(library, symbol)`, as a system linker does.

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
fn each_import_appears_once_in_the_symbol_table() {
    let project = common::temp_project("link_unique_imports", SOURCE);
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
    let executable = project.join("build").join("link_unique_imports.out");
    let undefined = Command::new("nm")
        .arg("-u")
        .arg(&executable)
        .output()
        .expect("run nm");
    let undefined = String::from_utf8_lossy(&undefined.stdout);
    let mut symbols: Vec<&str> = undefined.lines().map(str::trim).collect();
    let listed = symbols.len();
    symbols.sort_unstable();
    symbols.dedup();
    assert_eq!(
        listed,
        symbols.len(),
        "duplicate undefined symbols:\n{undefined}"
    );
    assert!(symbols.contains(&"___error"), "nm -u:\n{undefined}");

    let run = Command::new(&executable)
        .current_dir(&project)
        .output()
        .expect("run the program");
    assert!(run.status.success());
    assert_eq!(
        String::from_utf8_lossy(&run.stdout).lines().nth(1),
        Some("TRUE")
    );
}
