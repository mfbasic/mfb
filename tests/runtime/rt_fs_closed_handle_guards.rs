//! Regression test for bug-694 C3: `fs::isBuffered`, `fs::setBuffered` and
//! `fs::flush` had no closed-handle guard.
//!
//! Every other `fs` operation on a `File` refuses a closed handle with
//! `ErrResourceClosed`, as `mfb spec` §15 requires ("an operation reaching such a
//! value raises the resource-closed error"). These three read the record as if it
//! were open: `isBuffered` returned the stale flag, `setBuffered` rewrote it, and
//! `flush` drained through the fd number the close had already released — a
//! number the host may since have handed to an unrelated file.
//!
//! The handle is closed through a resource parameter, which leaves the caller's
//! binding usable — the one source-level path that reaches a closed `File`.

#[path = "../common/mod.rs"]
mod common;

use std::fs;

#[test]
fn buffering_operations_refuse_a_closed_file() {
    let out_dir = std::env::temp_dir().join(format!("mfb_bug694_c3_{}", common::unique_nonce()));
    fs::create_dir_all(&out_dir).expect("create output dir");
    let path = common::mfb_path_literal(&out_dir.join("closed.txt"));
    let source = format!(
        r#"IMPORT fs
IMPORT io

FUNC closeIt(x AS fs::File) AS Nothing
  fs::close(x)
END FUNC

FUNC main() AS Nothing
  RES f = fs::openFile("{path}", "w")
  fs::setBuffered(f, TRUE)
  fs::writeAll(f, "kept")
  closeIt(f)
  LET buffered = fs::isBuffered(f) TRAP(e)
    io::print("isBuffered " & toString(e.code))
    RECOVER FALSE
  END TRAP
  IF buffered THEN io::print("isBuffered ok")
  fs::setBuffered(f, FALSE) TRAP(e)
    io::print("setBuffered " & toString(e.code))
    RECOVER
  END TRAP
  fs::flush(f) TRAP(e)
    io::print("flush " & toString(e.code))
    RECOVER
  END TRAP
  io::print("done")
END FUNC
"#
    );
    let project = common::temp_project("bug694_closed_guards", &source);
    let executable = common::build_project(&project);
    let output = common::run_with_stdin(&executable, b"");

    // ErrResourceClosed is 77030004.
    assert_eq!(
        output.lines().collect::<Vec<_>>(),
        vec![
            "isBuffered 77030004",
            "setBuffered 77030004",
            "flush 77030004",
            "done"
        ],
        "every buffering operation on a closed File must raise ErrResourceClosed"
    );
    // The close drained the buffer before releasing the fd, so the text is on disk.
    assert_eq!(
        fs::read_to_string(out_dir.join("closed.txt")).expect("read written file"),
        "kept"
    );
    let _ = fs::remove_dir_all(&out_dir);
    let _ = fs::remove_dir_all(&project);
}
