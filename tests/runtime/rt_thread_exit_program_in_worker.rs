//! bug-898: `EXIT PROGRAM` inside a thread worker ends the process with its code.
//!
//! `mfb spec language control-flow`: "`EXIT PROGRAM <integer>` terminates the process
//! with the given exit code from any call depth". A worker's entry that comes back
//! with the program-exit tag (`2`) carries the exit code in the value register and
//! nothing in the message/origin registers. `_mfb_rt_thread_finish` split the parked
//! result on `tag == Ok` only, so tag 2 went down the error arm and deep-copied the
//! stale message/origin registers as a `String`/`ErrorLoc`: the process died with
//! SIGSEGV (exit 139) or printed a bogus `ErrOutOfMemory` (`Error: 7-701-0001`,
//! exit 255), nondeterministically, at every optimization level.
//!
//! The crash was a race on stale register contents, so each program runs several
//! times per optimization level.

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

const RUNS_PER_LEVEL: usize = 10;

/// Each case is a whole program that must exit 7 with the given stdout and no stderr.
const CASES: &[(&str, &str, &str)] = &[
    (
        "entry_exits_parent_waits",
        "\
IMPORT io
IMPORT thread

ISOLATED FUNC quitter(w AS ThreadWorker OF Integer TO Integer, n AS Integer) AS Integer
  IF n > 0 THEN EXIT PROGRAM n
  RETURN 0
END FUNC

FUNC main AS Integer
  RES a AS Thread OF Integer TO Integer = thread::start(quitter, 7)
  LET r AS Integer = thread::waitFor(a)
  io::print(\"after waitFor r=\" & toString(r))
  RETURN 3
END FUNC
",
        "",
    ),
    (
        "entry_exits_parent_sleeps",
        "\
IMPORT io
IMPORT os
IMPORT thread

ISOLATED FUNC quitter(w AS ThreadWorker OF Integer TO Integer, n AS Integer) AS Integer
  IF n > 0 THEN EXIT PROGRAM n
  RETURN 0
END FUNC

FUNC main AS Integer
  RES a AS Thread OF Integer TO Integer = thread::start(quitter, 7)
  os::sleep(5000)
  io::print(\"parent woke\")
  RETURN 3
END FUNC
",
        "",
    ),
    (
        "callee_exits_under_inline_trap",
        "\
IMPORT io
IMPORT thread

ISOLATED FUNC bye(n AS Integer) AS Integer
  EXIT PROGRAM n
END FUNC

ISOLATED FUNC quitter(w AS ThreadWorker OF Integer TO Integer, n AS Integer) AS Integer
  LET v AS Integer = bye(n) TRAP
    RECOVER 1
  END TRAP
  RETURN v
  TRAP(e)
    RETURN 2
  END TRAP
END FUNC

FUNC main AS Integer
  RES a AS Thread OF Integer TO Integer = thread::start(quitter, 7)
  LET r AS Integer = thread::waitFor(a)
  io::print(\"after waitFor r=\" & toString(r))
  RETURN 3
END FUNC
",
        "",
    ),
    (
        "worker_output_before_exit",
        "\
IMPORT io
IMPORT thread

ISOLATED FUNC quitter(w AS ThreadWorker OF Integer TO Integer, n AS Integer) AS Integer
  io::print(\"worker says bye\")
  EXIT PROGRAM n
END FUNC

FUNC main AS Integer
  RES a AS Thread OF Integer TO Integer = thread::start(quitter, 7)
  LET r AS Integer = thread::waitFor(a)
  io::print(\"after waitFor r=\" & toString(r))
  RETURN 3
END FUNC
",
        "worker says bye\n",
    ),
];

#[test]
fn exit_program_in_a_worker_ends_the_process_with_its_code() {
    let mut failures = Vec::new();
    for (name, source, want_stdout) in CASES {
        let project = common::temp_project(&format!("worker_exit_{name}"), source);
        for level in ["0", "3"] {
            let exe = common::build_project_opt(&project, level);
            for run in 0..RUNS_PER_LEVEL {
                let output = Command::new(&exe).output().expect("run program");
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                if output.status.code() != Some(7) || stdout != *want_stdout || !stderr.is_empty()
                {
                    failures.push(format!(
                        "{name} -O{level} run {run}: want exit 7 and stdout {want_stdout:?}, got {} stdout {stdout:?} stderr {stderr:?}",
                        common::exit_description(&output.status)
                    ));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
