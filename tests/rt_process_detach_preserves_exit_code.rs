//! Regression test for bug-474: `process::detach` must not destroy
//! `process::waitFor`'s exit code for every OTHER child.
//!
//! The Unix `detach` helper used to reap by installing a **process-wide** signal
//! disposition — `signal(SIGCHLD, SIG_IGN)` — which tells the kernel to reap
//! *every* child of the process immediately, not just the detached one. Any
//! later `waitpid` then failed with `ECHILD`, and `func_wait_for.rs`'s POSIX
//! helper treats `ECHILD` as "already reaped" and returns the handle's cached
//! exit code, which for a never-waited child is its initialised default `0`. So
//! a program that detached one child silently read `0` out of `waitFor` for
//! every unrelated child, including one that failed.
//!
//! The fix reaps the detached child with a per-child reaper thread
//! (`_mfb_rt_process_reaper`, a `waitpid` on exactly that pid) and leaves the
//! process-wide `SIGCHLD` disposition alone.
//!
//! Two halves, both runtime effects the golden harness cannot see:
//!   1. `first=7` — the real exit code of a child that was never detached,
//!      observed after an unrelated `detach`. This is the bug.
//!   2. `probe=.` — the detached child is still reaped, leaving no zombie.
//!      `detach`'s other contract, which the fix must not trade away. (A zombie
//!      renders as `probe=Z.`; verified by removing the `detach`.)
//!
//! Both children are identified by their own pids, never by "the only child",
//! so the test is unaffected by anything else the suite runs concurrently.

#![cfg(unix)]

mod common;

use std::time::Duration;

const SOURCE: &str = r#"IMPORT process
IMPORT io
IMPORT os

FUNC main AS Integer
  ' bug-474: `other` is detached; `first` is not, and is unrelated to it.
  ' `first` exits 7, so waitFor(first) must report 7 — not 0.
  RES first = process::shell("exit 7")
  RES other = process::shell("sleep 5")
  process::detach(other)
  io::print("first=" & toString(process::waitFor(first)))

  ' detach must still reap the child it detached: a second later there must be
  ' no zombie for `quick`'s pid. `ps -o stat=` prints nothing once the pid is
  ' gone, so the probe line is "." when reaped and "Z." when a zombie remains.
  RES quick = process::shell("exit 0")
  LET quickPid = process::pid(quick)
  process::detach(quick)
  os::sleep(1500)
  RES probe = process::shell("ps -o stat= -p " & toString(quickPid) & " | tr -d ' \n'; echo .")
  io::print("probe=" & process::receive(probe))
  RETURN 0
END FUNC
"#;

#[test]
fn detach_preserves_wait_for_exit_code_of_other_children() {
    let project = common::temp_project("bug474_detach_exitcode", SOURCE);
    let exe = common::build_project(&project);
    let (status, stdout) = common::run_bounded(
        &exe,
        Duration::from_secs(60),
        "process::detach / waitFor deadlock",
    );
    assert!(
        status.success(),
        "program exited non-zero ({status:?}):\n{stdout}"
    );

    assert!(
        stdout.contains("first=7"),
        "bug-474: waitFor on a child that was never detached returned the wrong \
         exit code after an unrelated process::detach. Expected `first=7`, got:\n{stdout}"
    );
    assert!(
        stdout.contains("probe=."),
        "process::detach left the detached child unreaped (a zombie). Expected \
         `probe=.`, got:\n{stdout}"
    );

    let _ = std::fs::remove_dir_all(&project);
}
