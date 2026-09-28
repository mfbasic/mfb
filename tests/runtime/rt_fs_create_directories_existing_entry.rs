//! Regression test for bug-694 C5: `fs::createDirectories` reported success when
//! `path` already existed as a regular file.
//!
//! The walk accepts "already exists" from each `mkdir` so that re-running it on a
//! directory that is already there is a quiet success. It took the final
//! component's "already exists" the same way without checking what exists, so a
//! file at `path` also returned success — and the caller's next write into the
//! "directory" failed somewhere else. `fs::createDirectory` raises
//! `ErrAlreadyExists` for any existing entry; `createDirectories` now does the same
//! for an existing entry that is not a directory.

#[path = "../common/mod.rs"]
mod common;

use std::fs;

#[test]
fn create_directories_refuses_an_existing_file_and_accepts_an_existing_directory() {
    let out_dir = std::env::temp_dir().join(format!("mfb_bug694_c5_{}", common::unique_nonce()));
    fs::create_dir_all(out_dir.join("dir")).expect("create fixture directory");
    fs::write(out_dir.join("file"), "not a directory").expect("create fixture file");
    let file = common::mfb_path_literal(&out_dir.join("file"));
    let dir = common::mfb_path_literal(&out_dir.join("dir"));
    let source = format!(
        r#"IMPORT fs
IMPORT io

FUNC main() AS Nothing
  fs::createDirectories("{file}") TRAP(e)
    io::print("file " & toString(e.code))
    RECOVER
  END TRAP
  fs::createDirectories("{dir}") TRAP(e)
    io::print("dir " & toString(e.code))
    RECOVER
  END TRAP
  io::print("done")
END FUNC
"#
    );
    let project = common::temp_project("bug694_create_directories", &source);
    let executable = common::build_project(&project);
    let output = common::run_with_stdin(&executable, b"");

    // ErrAlreadyExists is 77050005; the existing directory is still a success.
    assert_eq!(
        output.lines().collect::<Vec<_>>(),
        vec!["file 77050005", "done"],
        "createDirectories over an existing file must raise ErrAlreadyExists"
    );
    assert!(
        out_dir.join("file").is_file(),
        "the file must be left untouched"
    );
    let _ = fs::remove_dir_all(&out_dir);
    let _ = fs::remove_dir_all(&project);
}
