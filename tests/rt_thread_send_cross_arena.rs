//! bug-498: a cross-thread message copy must never allocate from the PEER
//! thread's arena.
//!
//! `thread::send` / `thread.emit` used to repoint the pinned arena register at the
//! destination thread's arena state and deep-copy the message there — with no
//! lock, while the destination was itself allocating from that arena. The arena
//! allocator's quick-bin pop is a plain load/store of a free-list head, so two
//! threads popping the same bin hand out one block twice or dereference a torn
//! `next` link: both threads fault at the same PC in `_mfb_arena_alloc` on a
//! garbage pointer (observed `EXC_BAD_ACCESS address=0x23`, 5/5 runs).
//!
//! The fix copies the message into the SENDER's own arena and hands the block
//! across; the receiver adopts it (frees it into its own bins later). Every arena
//! but the main one lives for the whole process and even the main one is only
//! torn down at `_mfb_shutdown`, so the handed-over block stays mapped, and a free
//! only ever touches the freeing thread's own arena state — no shared state, no
//! lock needed on the ordinary per-thread allocation path.
//!
//! Both queue directions are exercised with the receiver DRAINING (so the sender
//! never blocks on a full queue) while both threads allocate for the whole run.
//! Each program is a deterministic sum, so a corrupted block is a wrong total, a
//! crash, or a hang — all three fail this test.

mod common;

use std::time::Duration;

const MESSAGES: i64 = 200_000;

fn digits(i: i64) -> i64 {
    i.to_string().len() as i64
}

/// Parent → worker (`thread::send` into the inbound queue). The worker receives
/// every message and allocates its own strings in the same loop.
const PARENT_TO_WORKER: &str = "\
IMPORT io\n\
IMPORT thread\n\
\n\
ISOLATED FUNC churn(w AS ThreadWorker OF String TO Integer, seed AS String) AS Integer\n\
  MUT n AS Integer = 0\n\
  FOR i = 1 TO 200000\n\
    LET m AS String = thread::receive(w)\n\
    LET s AS String = seed & toString(i) & \"-padding-padding-padding\"\n\
    n = n + len(s) + len(m)\n\
  NEXT\n\
  RETURN n\n\
END FUNC\n\
\n\
FUNC main AS Integer\n\
  LET t AS Thread OF String TO Integer = thread::start(churn, \"abc\")\n\
  FOR i = 1 TO 200000\n\
    thread::send(t, \"message-\" & toString(i) & \"-padding-padding-padding\")\n\
  NEXT\n\
  LET n AS Integer = thread::waitFor(t)\n\
  io::print(\"worker returned \" & toString(n))\n\
  RETURN 0\n\
END FUNC\n";

/// Worker → parent (`thread.emit` into the outbound queue). The parent receives
/// every message and allocates its own strings in the same loop.
const WORKER_TO_PARENT: &str = "\
IMPORT io\n\
IMPORT thread\n\
\n\
ISOLATED FUNC producer(w AS ThreadWorker OF String TO Integer, seed AS String) AS Integer\n\
  MUT n AS Integer = 0\n\
  FOR i = 1 TO 200000\n\
    LET s AS String = seed & toString(i) & \"-padding-padding-padding\"\n\
    n = n + len(s)\n\
    thread::send(w, s)\n\
  NEXT\n\
  RETURN n\n\
END FUNC\n\
\n\
FUNC main AS Integer\n\
  LET t AS Thread OF String TO Integer = thread::start(producer, \"abc\")\n\
  MUT total AS Integer = 0\n\
  FOR i = 1 TO 200000\n\
    LET m AS String = thread::receive(t)\n\
    LET s AS String = \"local-\" & toString(i) & \"-padding-padding-padding\"\n\
    total = total + len(m) + len(s)\n\
  NEXT\n\
  LET n AS Integer = thread::waitFor(t)\n\
  io::print(\"total \" & toString(total) & \" worker \" & toString(n))\n\
  RETURN 0\n\
END FUNC\n";

fn run_expecting(name: &str, source: &str, expected_line: &str) {
    let project = common::temp_project(name, source);
    let executable = common::build_project(&project);
    // The race is timing-dependent (5/5 on the unfixed compiler, but a fixed
    // compiler must be clean on every run): run the program a few times.
    for attempt in 1..=3 {
        let (status, stdout) = common::run_bounded(
            &executable,
            Duration::from_secs(120),
            "bug-498: a cross-thread arena race can also deadlock on a torn free list",
        );
        assert!(
            status.success(),
            "attempt {attempt}: program {} (bug-498 cross-thread arena race)\nstdout:\n{stdout}",
            common::exit_description(&status),
        );
        assert_eq!(
            stdout.trim(),
            expected_line,
            "attempt {attempt}: wrong total — a message block was corrupted"
        );
    }
    let _ = std::fs::remove_dir_all(&project);
}

#[test]
fn parent_to_worker_send_survives_a_busy_receiver_arena() {
    // len("abc" & i & "-padding-padding-padding") + len("message-" & i & "-padding-padding-padding")
    let n: i64 = (1..=MESSAGES)
        .map(|i| (3 + digits(i) + 24) + (8 + digits(i) + 24))
        .sum();
    run_expecting(
        "rt_thread_send_cross_arena_p2w",
        PARENT_TO_WORKER,
        &format!("worker returned {n}"),
    );
}

#[test]
fn worker_to_parent_emit_survives_a_busy_receiver_arena() {
    let worker: i64 = (1..=MESSAGES).map(|i| 3 + digits(i) + 24).sum();
    // len(m) + len("local-" & i & "-padding-padding-padding")
    let total: i64 = (1..=MESSAGES)
        .map(|i| (3 + digits(i) + 24) + (6 + digits(i) + 24))
        .sum();
    run_expecting(
        "rt_thread_send_cross_arena_w2p",
        WORKER_TO_PARENT,
        &format!("total {total} worker {worker}"),
    );
}
