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
//! 256 KiB is comfortably past the pipe buffer on every supported Unix and
//! small enough that the drained bytes are read back in four `receiveBytes`
//! chunks.

mod common;

use std::time::Duration;

/// `yes hello` writes `"hello\n"` forever; `head -c 262144` cuts it at 256 KiB.
/// 262144 is not a multiple of 6, so the final line is a partial `"hel"` — the
/// exact total is what the byte count below pins.
const SOURCE: &str = r#"IMPORT process
IMPORT io

FUNC main AS Integer
  RES p = process::shell("yes hello | head -c 262144")
  LET code = process::waitFor(p)
  io::print("exit=" & toString(code))

  ' The drained output must still be there, in order, line by line.
  LET l1 = process::receive(p)
  IF l1 = "hello\n" THEN
    io::print("line1-ok")
  ELSE
    io::print("line1-bad:" & toString(len(l1)))
  END IF
  LET l2 = process::receive(p)
  IF l2 = "hello\n" THEN
    io::print("line2-ok")
  ELSE
    io::print("line2-bad:" & toString(len(l2)))
  END IF

  ' ...and the rest of it, to the byte.
  MUT total = 12
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

#[test]
fn waitfor_drains_a_chatty_child_without_losing_its_output() {
    let project = common::temp_project("bug475_waitfor_drain", SOURCE);
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
        stdout, "exit=0\nline1-ok\nline2-ok\nbytes=262144\n",
        "waitFor must return the child's exit code and leave its full 256 KiB of \
         output readable through receive/receiveBytes"
    );
}
