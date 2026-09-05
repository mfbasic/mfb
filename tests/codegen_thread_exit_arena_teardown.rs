//! bug-547: a program that can still have a LIVE worker at `main`'s return must
//! not free the main arena on the way out — on any platform family.
//!
//! `thread.start` `arena_alloc`s the worker's thread control block AND the
//! worker's whole arena-state block (arena state + the per-arena globals region)
//! out of the SPAWNING thread's arena, so a running worker's pinned arena and
//! current-thread registers both point into the MAIN arena's blocks. Dropping a
//! `Thread` handle only cancels and broadcasts — a detached worker keeps running —
//! so `_mfb_shutdown` is reached with those blocks live. If it calls
//! `_mfb_arena_destroy`, the `munmap`/`VirtualFree` pulls the worker's own arena
//! state out from under it and the worker faults on its next allocation or TCB
//! read. On box 2230 (Win11 x86_64) that was `0xC0000005` in 2 of 25 runs of a
//! program whose worker merely spins; macOS carried the identical latent
//! use-after-free. Only Linux deferred the teardown.
//!
//! No box is needed to pin the decision: it is a codegen one, taken per target in
//! `builder::lower_module_for_platform`, so this reads the `-ncode` dump of
//! `runtime.shutdown` (`_mfb_shutdown`) for each target from any host.
//!
//! The scope half matters as much as the fix: a program with NO `thread.` runtime
//! call cannot have a worker outlive it, so it must still destroy the arena. That
//! keeps the change byte-identical for every threadless program on every target.

mod common;
use common::{build_ncode, temp_project};

/// The bug's own shape: start a worker and return from `main` WITHOUT joining it.
const THREADED: &str = "\
IMPORT io\n\
IMPORT thread\n\
\n\
ISOLATED FUNC worker(t AS ThreadWorker OF RES Integer TO Integer, n AS Integer) AS Integer\n\
  RETURN n\n\
END FUNC\n\
\n\
FUNC main AS Integer\n\
  LET a AS Thread OF RES Integer TO Integer = thread::start(worker, 0)\n\
  io::print(\"started\")\n\
  RETURN 0\n\
END FUNC\n";

/// The same program, joined. It still embeds `thread.` runtime calls, so it takes
/// the same deferral — the gate is "can this program have a worker at all", not a
/// dataflow proof that one is live.
const JOINED: &str = "\
IMPORT io\n\
IMPORT thread\n\
\n\
ISOLATED FUNC worker(t AS ThreadWorker OF RES Integer TO Integer, n AS Integer) AS Integer\n\
  RETURN n\n\
END FUNC\n\
\n\
FUNC main AS Integer\n\
  LET a AS Thread OF RES Integer TO Integer = thread::start(worker, 0)\n\
  io::print(toString(thread::waitFor(a)))\n\
  RETURN 0\n\
END FUNC\n";

/// No `thread.` runtime call anywhere: nothing can outlive `main`.
const THREADLESS: &str = "\
IMPORT io\n\
\n\
FUNC main AS Integer\n\
  io::print(\"started\")\n\
  RETURN 0\n\
END FUNC\n";

/// Every target `mfb build -ncode` accepts that has a thread runtime. The bug is
/// Windows-visible, but the decision is per-family and was wrong on two of the
/// three families, so all of them are pinned here.
const TARGETS: [&str; 5] = [
    "windows-x86_64",
    "macos-aarch64",
    "linux-x86_64",
    "linux-aarch64",
    "linux-riscv64",
];

/// The `bl` targets of `runtime.shutdown` in an `-ncode` dump.
fn shutdown_calls(project: &std::path::Path, target: &str, name: &str) -> Vec<String> {
    let ncode = build_ncode(project, target, name);
    let shutdown = ncode["functions"]
        .as_array()
        .expect("ncode has a functions array")
        .iter()
        .find(|f| f["name"].as_str() == Some("runtime.shutdown"))
        .unwrap_or_else(|| panic!("{target}: ncode has no runtime.shutdown"));
    assert_eq!(
        shutdown["symbol"].as_str(),
        Some("_mfb_shutdown"),
        "{target}: runtime.shutdown is the `_mfb_shutdown` teardown"
    );
    shutdown["instructions"]
        .as_array()
        .expect("shutdown has instructions")
        .iter()
        .filter(|inst| inst["op"].as_str() == Some("bl"))
        .filter_map(|inst| inst["target"].as_str().map(str::to_string))
        .collect()
}

#[test]
fn a_program_that_can_leave_a_worker_running_does_not_free_the_arena_at_exit() {
    let project = temp_project("b547_threaded", THREADED);
    for target in TARGETS {
        let calls = shutdown_calls(&project, target, "b547_threaded");
        assert!(
            !calls.iter().any(|c| c == "_mfb_arena_destroy"),
            "{target}: `_mfb_shutdown` must NOT destroy the main arena while a worker \
             can still be running — the worker's own arena state and thread control \
             block live in the blocks it would unmap. Calls emitted: {calls:?}"
        );
    }
}

#[test]
fn joining_the_worker_takes_the_same_deferral() {
    let project = temp_project("b547_joined", JOINED);
    for target in TARGETS {
        let calls = shutdown_calls(&project, target, "b547_joined");
        assert!(
            !calls.iter().any(|c| c == "_mfb_arena_destroy"),
            "{target}: the deferral is gated on the program embedding a `thread.` \
             runtime call, not on a liveness proof. Calls emitted: {calls:?}"
        );
    }
}

#[test]
fn a_threadless_program_still_destroys_the_arena_at_exit() {
    let project = temp_project("b547_threadless", THREADLESS);
    for target in TARGETS {
        let calls = shutdown_calls(&project, target, "b547_threadless");
        assert!(
            calls.iter().any(|c| c == "_mfb_arena_destroy"),
            "{target}: a program with no worker must still free the main arena — the \
             bug-547 deferral is scoped to programs that use `thread.`, so every \
             threadless program stays byte-identical. Calls emitted: {calls:?}"
        );
    }
}

/// The teardown steps that DO have to keep running are untouched: the deferral
/// removes exactly one call. `io::print` with the stdout buffer off emits no
/// drain, so the assertion is on what is absent, plus that the threadless control
/// on the same target emits the destroy and nothing else new.
#[test]
fn the_deferral_removes_exactly_the_arena_destroy_call() {
    let threaded = temp_project("b547_delta_threaded", THREADED);
    let threadless = temp_project("b547_delta_threadless", THREADLESS);
    for target in TARGETS {
        let mut with_worker = shutdown_calls(&threaded, target, "b547_delta_threaded");
        let mut without = shutdown_calls(&threadless, target, "b547_delta_threadless");
        without.retain(|c| c != "_mfb_arena_destroy");
        with_worker.sort();
        without.sort();
        assert_eq!(
            with_worker, without,
            "{target}: the only teardown difference a worker makes is the skipped \
             `_mfb_arena_destroy`"
        );
    }
}
