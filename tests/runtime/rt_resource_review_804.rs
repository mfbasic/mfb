//! Regression test for bug-804: a successful `thread::transfer` moves the sent
//! resource, so a later read of the sender's binding is a compile-time
//! `TYPE_USE_AFTER_MOVE` — while an inline `TRAP` failure handler keeps the
//! untransferred resource and may still use it.
//!
//! `mfb spec language memory-semantics` §14.9 and `resource-management` §15
//! ("Transfer outcome and cleanup routing"): the successful continuation sees the
//! resource moved; the inline failure arm sees it in its pre-transfer state; the
//! continuation after inline recovery is moved on *some* path and so unusable.
//!
//! Before the fix the verifier tracked no transfer effect at all, so every
//! rejected program below built and only refused the read at run time
//! (`ErrResourceMoved`).
//!
//! The accepted programs are run: each uses the full capacity-one resource queue
//! (the worker waits on the data plane) to force the second transfer's failure,
//! and proves the handler read the untransferred file.

#[path = "../common/mod.rs"]
mod common;

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use common::{build_project, mfb_exe, mfb_path_literal, run_bounded, temp_project, unique_nonce};

/// The shared prelude: a producer of a fresh file and a reader that reports a
/// refused read as text rather than failing.
fn prelude(data: &str) -> String {
    format!(
        "IMPORT fs\n\
         IMPORT io\n\
         IMPORT thread\n\
         FUNC fresh() AS RES fs::File\n  RETURN fs::openFile(\"{data}\")\nEND FUNC\n\
         FUNC read(f AS fs::File) AS String\n  LET line = fs::readLine(f) TRAP(e)\n    RECOVER e.message\n  END TRAP\n  RETURN line\nEND FUNC\n\
         ISOLATED FUNC worker(t AS ThreadWorker OF String RES fs::File TO Integer, seed AS String) AS Integer\n  LET signal = thread::receive(t)\n  RETURN 0\nEND FUNC\n\
         FUNC same(f AS fs::File) AS RES fs::File\n  RETURN f\nEND FUNC\n\
         FUNC wait(text AS String) AS Integer\n  IF text = \"\" THEN\n    FAIL error(77050002, \"no wait\")\n  END IF\n  RETURN toInt(text)\nEND FUNC\n"
    )
}

fn data_file() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mfb_bug804_{}", unique_nonce()));
    fs::create_dir_all(&dir).expect("create data dir");
    let data = dir.join("data.txt");
    fs::write(&data, "first\nsecond\n").expect("write data file");
    data
}

/// Build `body` (appended to the prelude) and return the program text plus the
/// 1-based line of the first line of `body` containing `marker`.
fn program(name: &str, body: &str, marker: &str) -> (PathBuf, usize) {
    let data = data_file();
    let source = format!("{}{body}", prelude(&mfb_path_literal(&data)));
    let line = source
        .lines()
        .position(|l| l.contains(marker))
        .map(|i| i + 1)
        .unwrap_or_else(|| panic!("marker `{marker}` not in source"));
    (temp_project(name, &source), line)
}

/// The build must fail with exactly one `TYPE_USE_AFTER_MOVE`, at `line`.
fn assert_use_after_move(name: &str, body: &str, marker: &str) {
    let (project, line) = program(name, body, marker);
    let output = Command::new(mfb_exe())
        .arg("build")
        .arg(&project)
        .output()
        .expect("run mfb build");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !output.status.success(),
        "{name}: a read after a successful transfer built:\n{text}"
    );
    let reports: Vec<&str> = text
        .lines()
        .filter(|l| l.contains("TYPE_USE_AFTER_MOVE"))
        .collect();
    assert_eq!(
        reports.len(),
        1,
        "{name}: expected one TYPE_USE_AFTER_MOVE:\n{text}"
    );
    assert!(
        reports[0].contains(&format!("main.mfb:{line} ")),
        "{name}: TYPE_USE_AFTER_MOVE not reported at line {line}:\n{text}"
    );
    assert!(
        text.lines()
            .all(|l| !l.contains(" error[") || l.contains("TYPE_USE_AFTER_MOVE")),
        "{name}: unexpected additional diagnostic:\n{text}"
    );
}

fn run_accepted(name: &str, body: &str) -> String {
    let (project, _) = program(name, body, "FUNC main");
    let exe = build_project(&project);
    let (status, stdout) = run_bounded(&exe, Duration::from_secs(30), name);
    assert!(status.success(), "{name}: program failed: {stdout}");
    stdout
}

#[test]
fn a_read_after_a_successful_transfer_is_a_use_after_move() {
    assert_use_after_move(
        "b804_success",
        "FUNC main() AS Nothing\n  RES t = thread::start(worker, \"\", 1, 1)\n  RES f = fresh()\n  thread::transfer(t, f)\n  io::print(read(f)) ' READ\n  thread::send(t, \"done\")\nEND FUNC\n",
        "' READ",
    );
}

#[test]
fn a_worker_read_after_its_own_transfer_is_a_use_after_move() {
    // The worker direction lowers to the same `thread.transferResource` target and
    // is split to `emitResource` only in codegen; the move is the same.
    assert_use_after_move(
        "b804_worker",
        "ISOLATED FUNC giver(w AS ThreadWorker OF String RES fs::File TO Integer, seed AS String) AS Integer\n  RES f = fresh()\n  thread::transfer(w, f)\n  io::print(read(f)) ' READ\n  RETURN 0\nEND FUNC\n\
         FUNC main() AS Nothing\n  RES t = thread::start(giver, \"\", 1, 1)\n  RES g AS fs::File = thread::accept(t)\n  io::print(read(g))\nEND FUNC\n",
        "' READ",
    );
}

#[test]
fn transferring_one_name_moves_its_possible_alias() {
    // `RES g = same(f)` may hand back `f` itself, so `g` may denote the transferred
    // resource; the alias set moves with it, exactly as for a close.
    assert_use_after_move(
        "b804_alias",
        "FUNC main() AS Nothing\n  RES t = thread::start(worker, \"\", 1, 1)\n  RES f = fresh()\n  RES g = same(f)\n  thread::transfer(t, f)\n  io::print(read(g)) ' READ\n  thread::send(t, \"done\")\nEND FUNC\n",
        "' READ",
    );
}

#[test]
fn a_read_after_an_inline_recovery_join_is_a_use_after_move() {
    // The handler's read is legal; the read after the join is not, because the
    // transfer may have succeeded.
    assert_use_after_move(
        "b804_join",
        "FUNC main() AS Nothing\n  RES t = thread::start(worker, \"\", 1, 1)\n  RES f = fresh()\n  thread::transfer(t, f, 0) TRAP(e)\n    io::print(read(f))\n    RECOVER\n  END TRAP\n  io::print(read(f)) ' READ\n  thread::send(t, \"done\")\nEND FUNC\n",
        "' READ",
    );
}

#[test]
fn a_nested_fallible_argument_keeps_the_failure_handler_legal_but_not_the_join() {
    // A fallible call inside the transfer's arguments lowers to a check chain whose
    // single handler runs after the checks, behind a failure flag.
    assert_use_after_move(
        "b804_chain_join",
        "FUNC main() AS Nothing\n  RES t = thread::start(worker, \"\", 1, 1)\n  RES f = fresh()\n  thread::transfer(t, f, wait(\"0\")) TRAP(e)\n    io::print(read(f))\n    RECOVER\n  END TRAP\n  io::print(read(f)) ' READ\n  thread::send(t, \"done\")\nEND FUNC\n",
        "' READ",
    );
}

#[test]
fn an_inline_failure_handler_may_use_the_untransferred_resource() {
    let stdout = run_accepted(
        "b804_failure",
        "FUNC main() AS Nothing\n  RES t = thread::start(worker, \"\", 1, 1)\n  RES fill = fresh()\n  thread::transfer(t, fill)\n  RES f = fresh()\n  thread::transfer(t, f, 0) TRAP(e)\n    io::print(read(f))\n    RECOVER\n  END TRAP\n  thread::send(t, \"done\")\nEND FUNC\n",
    );
    assert_eq!(stdout, "first\n");
}

#[test]
fn a_check_chain_failure_handler_may_use_the_untransferred_resource() {
    // First chain: the argument succeeds and the full queue fails the transfer.
    // Second chain: the argument itself fails, so no transfer is attempted.
    let stdout = run_accepted(
        "b804_chain_failure",
        "FUNC main() AS Nothing\n  RES t = thread::start(worker, \"\", 1, 1)\n  RES fill = fresh()\n  thread::transfer(t, fill)\n  RES f = fresh()\n  thread::transfer(t, f, wait(\"0\")) TRAP(e)\n    io::print(read(f))\n    RECOVER\n  END TRAP\n  RES h = fresh()\n  thread::transfer(t, h, wait(\"\")) TRAP(e)\n    io::print(e.message & \" \" & read(h))\n    RECOVER\n  END TRAP\n  thread::send(t, \"done\")\nEND FUNC\n",
    );
    assert_eq!(stdout, "first\nno wait first\n");
}
