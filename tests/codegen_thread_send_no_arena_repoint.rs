//! bug-498: no user function may repoint the pinned arena register at another
//! thread's arena state.
//!
//! The only place user-level code ever wrote `ARENA_STATE_REGISTER` (`x19` on
//! AArch64) was the `thread::send` / `thread.emit` / `thread::transfer` lowering,
//! which swapped in the DESTINATION thread's arena to deep-copy the message there
//! (an unlocked allocation racing that thread's own allocator) and restored it
//! afterwards — including on the error path. After the fix the copy lands in the
//! sender's own arena, so a compiled program's functions never write `x19`; only
//! the hand-written runtime helpers (`program.entry`, `runtime.shutdown`,
//! `runtime.thread.trampoline`, the app/graphics thread entries) pin it.
//!
//! The runtime symptom is a data race (`tests/rt_thread_send_cross_arena.rs`);
//! this pins the deterministic codegen invariant behind it. Pinned to
//! `macos-aarch64` so the register spelling is fixed regardless of host.

mod common;
use common::{build_ncode, temp_project};

const SOURCE: &str = "\
IMPORT io\n\
IMPORT thread\n\
\n\
ISOLATED FUNC churn(w AS ThreadWorker OF String TO Integer, seed AS String) AS Integer\n\
  MUT n AS Integer = 0\n\
  FOR i = 1 TO 3\n\
    LET m AS String = thread::receive(w)\n\
    thread::send(w, seed & m)\n\
    n = n + len(m)\n\
  NEXT\n\
  RETURN n\n\
END FUNC\n\
\n\
FUNC main AS Integer\n\
  LET t AS Thread OF String TO Integer = thread::start(churn, \"abc\")\n\
  FOR i = 1 TO 3\n\
    thread::send(t, \"message-\" & toString(i))\n\
    io::print(thread::receive(t))\n\
  NEXT\n\
  RETURN thread::waitFor(t)\n\
END FUNC\n";

#[test]
fn compiled_functions_never_write_the_arena_register() {
    let project = temp_project("codegen_thread_send_no_arena_repoint", SOURCE);
    let ncode = build_ncode(
        &project,
        "macos-aarch64",
        "codegen_thread_send_no_arena_repoint",
    );
    let functions = ncode["functions"]
        .as_array()
        .expect("ncode has a functions array");

    let mut saw_send_site = false;
    let mut offenders = Vec::new();
    for func in functions {
        let name = func["name"].as_str().unwrap_or("");
        // Hand-written runtime helpers pin their own thread's arena legitimately.
        if name == "program.entry" || name.starts_with("runtime.") {
            continue;
        }
        let Some(insts) = func["instructions"].as_array() else {
            continue;
        };
        if insts.iter().any(|inst| {
            inst["op"].as_str() == Some("bl")
                && inst["target"].as_str().is_some_and(|t| {
                    t.contains("thread") && (t.contains("send") || t.contains("emit"))
                })
        }) {
            saw_send_site = true;
        }
        for (idx, inst) in insts.iter().enumerate() {
            if inst["dst"].as_str() == Some("x19") {
                offenders.push(format!("{name}[{idx}]: {inst}"));
            }
        }
    }
    assert!(
        saw_send_site,
        "fixture no longer contains a thread send/emit call site; the inspection shape drifted"
    );
    assert!(
        offenders.is_empty(),
        "compiled functions write the arena register (bug-498 cross-thread arena repoint):\n{}",
        offenders.join("\n")
    );
    let _ = std::fs::remove_dir_all(&project);
}
