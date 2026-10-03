//! Regression test for bug-723: `io::setBuffered(FALSE)` dropped the buffered
//! standard-output bytes when draining them failed.
//!
//! Turning buffering off drains the pending bytes and then clears the thread's
//! enabled flag. The drain's result was ignored, so a failed write still cleared
//! the flag, and every later drain (`io::flush`, program exit) is a no-op with
//! buffering off: the bytes were lost and nothing reported it. `io::flush` keeps
//! the bytes and raises `ErrWriteFailed` on the same failure; `setBuffered` now
//! does too, leaving buffering on so the bytes can still be flushed — the
//! contract `fs::setBuffered` already has (bug-694 C7).
//!
//! Standard output is a descriptor opened only for reading, so the host refuses
//! every write and the failing drain is deterministic. Results go to stderr.

#[path = "../common/mod.rs"]
mod common;

use std::process::{Command, Stdio};

#[test]
fn turning_stdout_buffering_off_reports_a_failed_drain_and_keeps_the_bytes() {
    let source = r#"IMPORT io

FUNC main() AS Nothing
  io::setBuffered(TRUE)
  io::write("pending")
  io::setBuffered(FALSE) TRAP(e)
    io::printError("setBuffered " & toString(e.code))
    RECOVER
  END TRAP
  io::printError("buffered " & toString(io::isBuffered()))
  io::flush() TRAP(e)
    io::printError("flush " & toString(e.code))
    RECOVER
  END TRAP
  io::printError("done")
END FUNC
"#;
    let project = common::temp_project("bug723_io_set_buffered", source);
    let executable = common::build_project(&project);
    let unwritable = std::fs::File::open(&executable).expect("open a read-only stdout");
    let output = Command::new(&executable)
        .stdin(Stdio::null())
        .stdout(Stdio::from(unwritable))
        .stderr(Stdio::piped())
        .output()
        .expect("run the program");
    let stderr = String::from_utf8_lossy(&output.stderr);

    // ErrWriteFailed is 77020002. Buffering stays on with the bytes still
    // pending, so the following flush retries them, fails the same way, and
    // reports it.
    assert_eq!(
        stderr.lines().collect::<Vec<_>>(),
        vec![
            "setBuffered 77020002",
            "buffered TRUE",
            "flush 77020002",
            "done"
        ],
        "a failed drain while turning stdout buffering off must raise ErrWriteFailed \
         and keep the bytes pending; status {:?}",
        output.status
    );
}
