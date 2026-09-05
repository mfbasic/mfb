//! Regression test for bug-475: `process::waitFor` must not deadlock against a
//! child whose output outruns the pipe buffer — and must not "unblock" it by
//! throwing that output away.
//!
//! Before the fix `waitFor` called `waitpid` and nothing else. The parent held
//! the child's stdout read end open and never read it, so a child writing more
//! than a pipeful (16–64 KiB on macOS, 64 KiB on Linux) blocked forever in its
//! own `write` while the parent blocked forever in `waitpid`. Neither side could
//! move; the program simply stopped, with no diagnostic.
//!
//! The test is deliberately a *both-directions* gate (the shape bug-467 used):
//!
//!   * the run must FINISH — that is the deadlock half, enforced by
//!     `run_bounded`; and
//!   * every byte the child wrote must still be delivered by
//!     `process::receive` / `process::receiveBytes` afterwards — that is the
//!     half a drain-and-discard "fix" would fail. `waitFor` reading the pipe and
//!     dropping the bytes would unblock the child while silently destroying the
//!     output the package is contracted to hand back, turning a hang into a
//!     wrong answer.
//!
//! 256 KiB is comfortably past the pipe buffer on every supported platform and
//! small enough that the drained bytes are read back in a few `receiveBytes`
//! chunks.
//!
//! **The producer is per-platform, and the test is not.** `yes hello | head -c`
//! is a Unix shell pipeline: on Windows `cmd.exe` has neither tool, so the child
//! exited 255 having written nothing and the case failed on a fixture detail
//! while saying nothing about `waitFor`. The tempting fix — `#![cfg(unix)]` on
//! the file — is exactly the mistake that let bug-544 ship: a platform-gated
//! runtime test is ZERO coverage on the platform it excludes, and bug-475's
//! property (drain the pipe, keep every byte) is not a Unix property. So the
//! producer is chosen per platform and the assertions follow it. Measured on box
//! 2230 before writing this: the Windows producer overruns the pipe and
//! `waitFor` returns 0 with the output intact, so the property does hold there —
//! this pins it.

mod common;

use std::time::Duration;

/// Unix: `yes hello` writes `"hello\n"` forever; `head -c 262144` cuts it at
/// 256 KiB. 262144 is not a multiple of 6, so the final line is a partial
/// `"hel"` — the exact total is what the byte count below pins.
#[cfg(unix)]
const PRODUCER: &str = "yes hello | head -c 262144";
/// Windows: `cmd.exe`'s `for /L` counting loop, which needs no external tool.
/// `echo hello` emits CRLF, so each line is 7 bytes and the total is exact —
/// there is no partial final line to account for. 37 449 lines is 262 143 bytes,
/// the same order as the Unix case and far past the pipe buffer.
#[cfg(windows)]
const PRODUCER: &str = "for /L %i in (1,1,37449) do @echo hello";

/// One line as `process::receive` returns it — the terminator is NOT stripped,
/// and it is the host's.
#[cfg(unix)]
const LINE: &str = "hello\\n";
#[cfg(windows)]
const LINE: &str = "hello\\r\\n";

/// Bytes the child writes in total, and the width of the two lines read back
/// individually before the `receiveBytes` loop takes over.
#[cfg(unix)]
const TOTAL_BYTES: usize = 262_144;
#[cfg(windows)]
const TOTAL_BYTES: usize = 37_449 * 7;

const SOURCE_TEMPLATE: &str = r#"IMPORT process
IMPORT io

FUNC main AS Integer
  RES p = process::shell("@PRODUCER@")
  LET code = process::waitFor(p)
  io::print("exit=" & toString(code))

  ' The drained output must still be there, in order, line by line.
  LET l1 = process::receive(p)
  IF l1 = "@LINE@" THEN
    io::print("line1-ok")
  ELSE
    io::print("line1-bad:" & toString(len(l1)))
  END IF
  LET l2 = process::receive(p)
  IF l2 = "@LINE@" THEN
    io::print("line2-ok")
  ELSE
    io::print("line2-bad:" & toString(len(l2)))
  END IF

  ' ...and the rest of it, to the byte.
  MUT total = @TWO_LINES@
  WHILE TRUE
    LET n = len(process::receiveBytes(p)) TRAP(e)
      RECOVER 0
    END TRAP
    IF n = 0 THEN EXIT WHILE
    total = total + n
  END WHILE
  io::print("bytes=" & toString(total))
  RETURN 0
END FUNC
"#;

/// The MFB program, with the host's producer, line shape and seed substituted in.
fn source() -> String {
    SOURCE_TEMPLATE
        .replace("@PRODUCER@", PRODUCER)
        .replace("@LINE@", LINE)
        .replace("@TWO_LINES@", &(2 * unescaped_len(LINE)).to_string())
}

/// Byte length of one received line on this host (`"hello\n"` = 6,
/// `"hello\r\n"` = 7) — the two lines read individually before the
/// `receiveBytes` loop seeds the running total.
fn unescaped_len(line: &str) -> usize {
    line.replace("\\n", "\n").replace("\\r", "\r").len()
}

#[test]
fn waitfor_drains_a_chatty_child_without_losing_its_output() {
    let project = common::temp_project("bug475_waitfor_drain", &source());
    let exe = common::build_project(&project);
    let (status, stdout) = common::run_bounded(
        &exe,
        Duration::from_secs(60),
        "bug-475: process::waitFor deadlocked against a child that filled the stdout pipe",
    );
    assert!(
        status.success(),
        "program exited {status:?}; stdout:\n{stdout}"
    );
    assert_eq!(
        stdout,
        format!("exit=0\nline1-ok\nline2-ok\nbytes={TOTAL_BYTES}\n"),
        "waitFor must return the child's exit code and leave the child's full \
         output readable through receive/receiveBytes"
    );
}
