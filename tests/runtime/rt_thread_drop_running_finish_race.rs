//! bug-897: dropping a `Thread` whose worker is finishing must not free memory the
//! worker still uses.
//!
//! A parent view's drop of a RUNNING worker closes and detaches it and gives up the
//! handle's reference. The worker's `_mfb_rt_thread_finish` gave up ITS reference
//! under the block lock and only then unmapped its arena (`_mfb_arena_destroy`,
//! which runs on the worker's arena-state block through the arena register). When
//! the drop's release came second it was the last one, and it freed the control
//! block — and the worker's arena-state block with it — while the worker was still
//! inside `_mfb_arena_destroy`: `malloc` aborted (`free_list_checksum_botch`, exit
//! 134), a later start read a zeroed control block (exit 139), or the destroy
//! faulted. The repro hit it in about half of its runs on macOS AArch64.
//!
//! `mfb spec threading control-block` § Lifetime: whichever side releases last, the
//! block is freed only after the worker no longer touches it.

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

const RUNS: usize = 20;

/// 32 drivers each start 400 workers that block in `receive` and drop each view at
/// once, so every drop races its worker's exit.
const SOURCE: &str = "\
IMPORT io
IMPORT thread
IMPORT collections

ISOLATED FUNC blocked(w AS ThreadWorker OF Integer TO Integer, n AS Integer) AS Integer
  LET v AS Integer = thread::receive(w)
  RETURN v
  TRAP(e)
    RETURN -1
  END TRAP
END FUNC

ISOLATED FUNC driver(w AS ThreadWorker OF Nothing TO Integer, n AS Integer) AS Integer
  MUT i AS Integer = 0
  WHILE i < n
    RES t AS Thread OF Integer TO Integer = thread::start(blocked, i)
    i = i + 1
  END WHILE
  RETURN i
END FUNC

FUNC main AS Integer
  MUT pool AS List OF RES Thread OF Nothing TO Integer = []
  MUT i AS Integer = 0
  WHILE i < 32
    RES t AS Thread OF Nothing TO Integer = thread::start(driver, 400)
    pool = collections::append(pool, t)
    i = i + 1
  END WHILE
  MUT s AS Integer = 0
  FOR EACH t IN pool
    s = s + thread::waitFor(t)
  NEXT
  io::print(toString(s))
  RETURN 0
END FUNC
";

#[test]
fn dropping_a_finishing_worker_never_frees_its_arena_state_early() {
    let project = common::temp_project("thread_drop_finish_race", SOURCE);
    let exe = common::build_project(&project);
    let mut failures = Vec::new();
    for run in 0..RUNS {
        let output = Command::new(&exe).output().expect("run program");
        let stdout = String::from_utf8_lossy(&output.stdout);
        if !output.status.success() || stdout != "12800\n" {
            failures.push(format!(
                "run {run}: want exit 0 and \"12800\\n\", got {} stdout {stdout:?} stderr {:?}",
                common::exit_description(&output.status),
                String::from_utf8_lossy(&output.stderr)
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {RUNS} runs failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
