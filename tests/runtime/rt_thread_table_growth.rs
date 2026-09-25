//! plan-156-A Phase 2: the process-global thread table grows without a limit.
//!
//! Every `thread::start` takes a slot in the table and the block's final release
//! returns it; the slot array starts at 64 and doubles when full. The `--debug`
//! report's `thread.` section reads the table at exit: `table_capacity` (the array's
//! size), `table_high` (slots ever handed out, i.e. the peak number of live control
//! blocks) and `table_live` (blocks not yet freed).

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use common::debug_report::{arena_lines, build_debug, counter, run_ok};

fn thread_value(case: &str, stderr: &str, key: &str) -> u64 {
    let at = stderr
        .rfind("mfb.debug.begin ")
        .unwrap_or_else(|| panic!("{case}: no report block on stderr:\n{stderr}"));
    let prefix = format!("{key} ");
    stderr[at..]
        .lines()
        .take_while(|line| *line != "mfb.debug.end 1")
        .find_map(|line| line.strip_prefix(&prefix))
        .and_then(|value| value.parse().ok())
        .unwrap_or_else(|| panic!("{case}: no `{key}` line:\n{stderr}"))
}

/// `{SEQ}` threads started and waited one at a time, then 200 held live at once (each
/// recursion level keeps its handle until the levels below it return).
const PROGRAM: &str = r#"IMPORT io
IMPORT thread

ISOLATED FUNC square(w AS ThreadWorker OF String TO Integer, seed AS Integer) AS Integer
  RETURN seed * seed
END FUNC

FUNC hold(n AS Integer) AS Integer
  IF n = 0 THEN
    RETURN 0
  END IF
  LET t AS Thread OF String TO Integer = thread::start(square, n)
  LET below AS Integer = hold(n - 1)
  RETURN below + thread::waitFor(t)
END FUNC

FUNC main AS Integer
  MUT total AS Integer = 0
  MUT i AS Integer = 0
  WHILE i < {SEQ}
    LET t AS Thread OF String TO Integer = thread::start(square, i MOD 100)
    total = total + thread::waitFor(t)
    i = i + 1
  END WHILE
  io::print(toString(total) & " " & toString(hold(200)))
  RETURN 0
END FUNC
"#;

/// The main arena's `live_bytes` at exit for `seq` sequential threads, after checking
/// the program's output and the table's shape.
fn run(seq: u64) -> u64 {
    let case = format!("growth_{seq}");
    let source = PROGRAM.replace("{SEQ}", &seq.to_string());
    let exe = build_debug(&format!("thread_table_growth_{seq}"), &source);
    let (stdout, stderr) = run_ok(&case, &exe);
    let sequential: u64 = (0..seq).map(|i| (i % 100) * (i % 100)).sum();
    let held: u64 = (1..=200u64).map(|n| n * n).sum();
    assert_eq!(stdout, format!("{sequential} {held}\n"), "{case}: wrong totals");
    let capacity = thread_value(&case, &stderr, "thread.table_capacity");
    assert!(
        capacity > 64,
        "{case}: 200 live blocks must have grown the table past 64 slots, capacity \
         is {capacity}"
    );
    assert!(
        thread_value(&case, &stderr, "thread.table_high") >= 200,
        "{case}: the 200 held threads did not each take a slot:\n{stderr}"
    );
    assert_eq!(
        thread_value(&case, &stderr, "thread.table_live"),
        0,
        "{case}: a control block was never freed:\n{stderr}"
    );
    let lines = arena_lines(&case, &stderr);
    counter(&case, &lines, 0, "live_bytes")
}

#[test]
fn the_table_grows_and_every_block_is_freed() {
    // The parent's arena holds nothing per thread: its live bytes at exit do not
    // move with the number of threads started.
    let small = run(1000);
    let large = run(5000);
    assert_eq!(
        small, large,
        "the main arena's live bytes grew with the thread count ({small} -> {large})"
    );
}
