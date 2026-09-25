//! Regression test for bug-694 C7: `fs::setBuffered(file, FALSE)` dropped the
//! buffered bytes when draining them failed.
//!
//! Turning buffering off drains the pending bytes and then clears the handle's
//! buffered flag. The drain's result was ignored, so a failed write still cleared
//! the flag — and every later drain (`fs::flush`, `fs::close`) is a no-op on an
//! unbuffered handle, so the bytes were silently lost. `fs::flush` keeps the bytes
//! and raises `ErrWriteFailed` on the same failure; `setBuffered` now does too,
//! leaving the handle buffered so the bytes can still be flushed.
//!
//! A write to a handle opened only for reading is refused by the host, which makes
//! the failing drain deterministic.

#[path = "../common/mod.rs"]
mod common;

use std::fs;

#[test]
fn turning_buffering_off_reports_a_failed_drain_and_keeps_the_bytes() {
    let out_dir = std::env::temp_dir().join(format!("mfb_bug694_c7_{}", common::unique_nonce()));
    fs::create_dir_all(&out_dir).expect("create output dir");
    fs::write(out_dir.join("input.txt"), "read only").expect("create fixture file");
    let path = common::mfb_path_literal(&out_dir.join("input.txt"));
    let source = format!(
        r#"IMPORT fs
IMPORT io

SUB main()
  RES h = fs::openFile("{path}", "r")
  fs::setBuffered(h, TRUE)
  fs::writeAll(h, "pending")
  fs::setBuffered(h, FALSE) TRAP(e)
    io::print("setBuffered " & toString(e.code))
    RECOVER
  END TRAP
  io::print("buffered " & toString(fs::isBuffered(h)))
  fs::close(h) TRAP(e)
    io::print("close " & toString(e.code))
    RECOVER
  END TRAP
  io::print("done")
END SUB
"#
    );
    let project = common::temp_project("bug694_set_buffered", &source);
    let executable = common::build_project(&project);
    let output = common::run_with_stdin(&executable, b"");

    // ErrWriteFailed is 77020002. The handle stays buffered with the bytes still
    // pending, so the close's own drain fails the same way and reports it.
    assert_eq!(
        output.lines().collect::<Vec<_>>(),
        vec![
            "setBuffered 77020002",
            "buffered TRUE",
            "close 77020002",
            "done"
        ],
        "a failed drain while turning buffering off must raise ErrWriteFailed \
         and keep the pending bytes"
    );
    let _ = fs::remove_dir_all(&out_dir);
    let _ = fs::remove_dir_all(&project);
}
