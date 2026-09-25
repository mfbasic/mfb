//! plan-156-B: a worker's arena is returned to the OS when the worker finishes.
//!
//! Every value a finished worker produced that another thread can still read — its
//! result, its messages, its transferred resources — was copied into the thread's
//! control block (plan-156-A), so nothing points into the worker's own arena once its
//! entry returns, and the worker unmaps it on the way out. Without that, every worker
//! arena's chunks stayed mapped for the life of the process, so a program that ran
//! many short workers grew without bound.

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::time::Duration;

/// 1,000 workers, one after another, each building (and dropping) a 10 MB string in
/// its own arena and returning a small number.
const PROGRAM: &str = r#"IMPORT io
IMPORT strings
IMPORT thread

ISOLATED FUNC churn(w AS ThreadWorker OF String TO Integer, seed AS Integer) AS Integer
  LET big AS String = strings::repeat("x", 10000000)
  RETURN len(big) MOD 1000 + seed
END FUNC

FUNC main AS Integer
  MUT total AS Integer = 0
  MUT i AS Integer = 0
  WHILE i < 1000
    RES t AS Thread OF String TO Integer = thread::start(churn, 1)
    total = total + thread::waitFor(t)
    i = i + 1
  END WHILE
  io::print(toString(total))
  RETURN 0
END FUNC
"#;

#[test]
fn a_thousand_ten_megabyte_workers_stay_under_a_hundred_megabytes() {
    let project = common::temp_project("worker_arena_release", PROGRAM);
    let exe = common::build_project(&project);
    let (status, stdout, rss) = common::run_bounded_with_rss(
        &exe,
        Duration::from_secs(600),
        "the worker-arena release probe did not finish",
    );
    assert!(
        status.success(),
        "{}\n{stdout}",
        common::exit_description(&status)
    );
    assert_eq!(stdout, "1000\n", "every worker ran");
    let rss = rss.expect("unix reports ru_maxrss");
    let _ = std::fs::remove_dir_all(&project);
    assert!(
        rss < 100 * 1024 * 1024,
        "peak RSS was {} MB: finished workers' arenas are not being returned to the OS",
        rss / (1024 * 1024)
    );
}

/// A worker's buffered standard output lives in the worker's arena, so the worker's
/// end drains it before that arena goes away: output a worker buffered and never
/// flushed still appears, ahead of anything the parent prints after collecting it.
const BUFFERED: &str = r#"IMPORT io
IMPORT thread

ISOLATED FUNC chatty(w AS ThreadWorker OF String TO Integer, seed AS Integer) AS Integer
  io::setBuffered(TRUE)
  io::print("from the worker")
  RETURN seed
END FUNC

FUNC main AS Integer
  RES t AS Thread OF String TO Integer = thread::start(chatty, 7)
  io::print("result " & toString(thread::waitFor(t)))
  RETURN 0
END FUNC
"#;

#[test]
fn a_workers_unflushed_buffered_output_is_drained_when_it_finishes() {
    let project = common::temp_project("worker_buffered_output", BUFFERED);
    let exe = common::build_project(&project);
    let output = std::process::Command::new(&exe)
        .output()
        .expect("run the program");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let _ = std::fs::remove_dir_all(&project);
    assert!(output.status.success(), "{stdout}");
    assert_eq!(stdout, "from the worker\nresult 7\n");
}
