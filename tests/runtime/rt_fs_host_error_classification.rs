//! Regression test for bug-701: an `fs` host refusal raises the same `errorCode`
//! whichever member met it and whichever host it ran on.
//!
//! One row per line of the bug's target table (measured before the fix, bug-694's
//! probe): reading a directory raised `ErrOutOfMemory` on Linux, `ErrReadFailed` on
//! macOS and `ErrAccessDenied` on Windows; the writers reported a missing directory
//! as `ErrNotFound` where the openers said `ErrPathNotFound`; a file used as a
//! directory, an over-long name and a directory where a file is required fell to
//! `ErrWriteFailed` on POSIX and to not-found or access-denied on Windows.
//!
//! The fixtures are built here with `std::fs`, so the same assertions run on every
//! host `cargo test` runs on. The link-loop row needs a symbolic link, which
//! Windows only grants with a privilege, so it is Unix-only; the whole matrix
//! (`tools/fs-error-matrix`) was also run on Windows by hand (see the bug doc).

#[path = "../common/mod.rs"]
mod common;

use std::fs;

/// `ErrPathNotFound`, `7-703-0001`.
const PATH_NOT_FOUND: &str = "77030001";
/// `ErrInvalidPath`, `7-703-0002`.
const INVALID_PATH: &str = "77030002";

#[test]
fn every_host_reports_a_refused_path_with_one_code() {
    let dir = std::env::temp_dir().join(format!("mfb_bug701_{}", common::unique_nonce()));
    fs::create_dir_all(dir.join("full").join("sub")).expect("create fixture dirs");
    fs::write(dir.join("full").join("f"), "x").expect("write fixture");
    fs::write(dir.join("plain.txt"), "file").expect("write fixture");
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("loopb", dir.join("loopa")).expect("symlink");
        std::os::unix::fs::symlink("loopa", dir.join("loopb")).expect("symlink");
    }
    let root = common::mfb_path_literal(&dir);
    let long = "n".repeat(300);
    // (label, statement kind, call)
    let mut rows: Vec<(&str, &str, String, &str)> = vec![
        (
            "readText dir",
            "let",
            format!(r#"fs::readText("{root}/full")"#),
            INVALID_PATH,
        ),
        (
            "readBytes dir",
            "let",
            format!(r#"fs::readBytes("{root}/full")"#),
            INVALID_PATH,
        ),
        (
            "writeText missing dir",
            "sub",
            format!(r#"fs::writeText("{root}/no/such/x", "x")"#),
            PATH_NOT_FOUND,
        ),
        (
            "openFile w missing dir",
            "res",
            format!(r#"fs::openFile("{root}/no/such/x", "w")"#),
            PATH_NOT_FOUND,
        ),
        (
            "readText too long",
            "let",
            format!(r#"fs::readText("{root}/{long}")"#),
            INVALID_PATH,
        ),
        (
            "writeText too long",
            "sub",
            format!(r#"fs::writeText("{root}/{long}", "x")"#),
            INVALID_PATH,
        ),
        (
            "readText file as dir",
            "let",
            format!(r#"fs::readText("{root}/plain.txt/x")"#),
            INVALID_PATH,
        ),
        (
            "writeText file as dir",
            "sub",
            format!(r#"fs::writeText("{root}/plain.txt/x", "x")"#),
            INVALID_PATH,
        ),
        (
            "writeText dir",
            "sub",
            format!(r#"fs::writeText("{root}/full", "x")"#),
            INVALID_PATH,
        ),
        (
            "openFile w dir",
            "res",
            format!(r#"fs::openFile("{root}/full", "w")"#),
            INVALID_PATH,
        ),
        (
            "openFile r dir",
            "res",
            format!(r#"fs::openFile("{root}/full", "r")"#),
            INVALID_PATH,
        ),
        (
            "deleteFile dir",
            "sub",
            format!(r#"fs::deleteFile("{root}/full")"#),
            INVALID_PATH,
        ),
        (
            "deleteDirectory file",
            "sub",
            format!(r#"fs::deleteDirectory("{root}/plain.txt")"#),
            INVALID_PATH,
        ),
        (
            "setCurrentDirectory file",
            "sub",
            format!(r#"fs::setCurrentDirectory("{root}/plain.txt")"#),
            INVALID_PATH,
        ),
        (
            "canonicalPath file as dir",
            "let",
            format!(r#"fs::canonicalPath("{root}/plain.txt/x")"#),
            INVALID_PATH,
        ),
        (
            "createDirectories through file",
            "sub",
            format!(r#"fs::createDirectories("{root}/plain.txt/x/y")"#),
            INVALID_PATH,
        ),
    ];
    if cfg!(unix) {
        rows.push((
            "canonicalPath loop",
            "let",
            format!(r#"fs::canonicalPath("{root}/loopa")"#),
            INVALID_PATH,
        ));
    }
    let mut source = String::from("IMPORT fs\nIMPORT io\n\n");
    for (index, (label, kind, call, _)) in rows.iter().enumerate() {
        let (open, handler) = match *kind {
            "let" => (format!("LET v = {call}"), "RETURN"),
            "res" => (format!("RES v = {call}"), "RETURN"),
            _ => (call.clone(), "RETURN"),
        };
        source.push_str(&format!(
            "FUNC c{index}() AS Nothing\n  {open} TRAP(e)\n    io::print(\"{label}: \" & toString(e.code))\n    {handler}\n  END TRAP\n  io::print(\"{label}: ok\")\nEND FUNC\n\n"
        ));
    }
    source.push_str("FUNC main() AS Nothing\n");
    for index in 0..rows.len() {
        source.push_str(&format!("  c{index}()\n"));
    }
    source.push_str("END FUNC\n");

    let project = common::temp_project("bug701_host_errors", &source);
    let executable = common::build_project(&project);
    let output = common::run_with_stdin(&executable, b"");
    let expected: Vec<String> = rows
        .iter()
        .map(|(label, _, _, code)| format!("{label}: {code}"))
        .collect();
    assert_eq!(
        output.lines().collect::<Vec<_>>(),
        expected,
        "each condition must raise its one code on every host"
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&project);
}
