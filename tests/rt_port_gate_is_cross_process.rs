//! bug-488: `common::PortGate` must exclude across PROCESSES, not just threads.
//!
//! The gate it replaced was a `static OnceLock<Mutex<()>>` in
//! `rt_tls_connect_allow_self_signed.rs`. That ordered the four cases inside one
//! test binary and nothing else — not another test binary in the same
//! `cargo test`, and not a second `cargo test` on the machine, which is routine
//! here. The window it guards is a real race: a port is bound to learn its
//! number, released so `openssl s_server` can take it, and anything on the
//! machine may claim it in between.
//!
//! **Why this test exists in this shape.** The end-to-end symptom — the TLS test
//! reaching a stranger's listener — could not be reproduced on demand: soaking
//! the *pre-fix* binary at 2-way and 4-way concurrency (26 runs total) produced
//! zero failures, while six sightings had occurred in the field. All six were
//! during full `cargo test` runs, where the port pressure comes from hundreds of
//! unrelated tests rather than from copies of one. Rather than assert a rate this
//! harness cannot produce, this pins the *property the fix supplies*: mutual
//! exclusion between processes. That is deterministic, fast, and fails loudly if
//! anyone swaps the gate back to something process-local.
//!
//! Children are copies of this test binary re-entered through an environment
//! variable, so no helper program has to be built or found.

mod common;

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

/// Set on a child copy; its value is the shared log path.
const CHILD_ENV: &str = "MFB_PORT_GATE_CHILD_LOG";

/// How long a child holds the gate. Long enough that an unserialized run would
/// certainly interleave (the children start within a few ms of each other), short
/// enough that the whole test is a couple of seconds.
const HOLD: Duration = Duration::from_millis(120);

fn append_line(path: &PathBuf, line: &str) {
    // O_APPEND: a write this small is atomic on POSIX, so the log records the
    // real order without a lock of its own — which matters, since a lock here
    // would mask the very thing under test.
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .expect("open the shared log");
    f.write_all(line.as_bytes()).expect("append to the shared log");
}

/// The child half: take the gate, mark the critical section, release it.
fn run_as_child(log: PathBuf) {
    let pid = std::process::id();
    let guard = common::PortGate::acquire();
    append_line(&log, &format!("IN {pid}\n"));
    std::thread::sleep(HOLD);
    append_line(&log, &format!("OUT {pid}\n"));
    drop(guard);
}

#[test]
fn the_port_gate_excludes_across_processes() {
    if let Ok(log) = std::env::var(CHILD_ENV) {
        run_as_child(PathBuf::from(log));
        return;
    }

    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock before epoch")
        .as_nanos();
    let log = std::env::temp_dir().join(format!("mfb_port_gate_{nonce}.log"));
    let _ = fs::remove_file(&log);

    let exe = std::env::current_exe().expect("locate this test binary");
    let children: Vec<_> = (0..4)
        .map(|_| {
            Command::new(&exe)
                .args(["the_port_gate_excludes_across_processes", "--exact", "--nocapture"])
                .env(CHILD_ENV, &log)
                .spawn()
                .expect("spawn a child copy of this test binary")
        })
        .collect();
    for mut child in children {
        let status = child.wait().expect("wait for a child");
        assert!(status.success(), "a child copy failed: {status:?}");
    }

    let text = fs::read_to_string(&log).expect("read the shared log");
    let lines: Vec<&str> = text.lines().filter(|l| !l.is_empty()).collect();
    assert_eq!(
        lines.len(),
        8,
        "expected one IN and one OUT per child; got:\n{text}"
    );

    // Serialized means every IN is closed by its OWN pid's OUT before the next
    // IN. An interleave (IN a, IN b, ...) is exactly the cross-process failure
    // the per-process mutex could not prevent.
    for pair in lines.chunks(2) {
        let (enter, leave) = (pair[0], pair[1]);
        let enter_pid = enter.strip_prefix("IN ").unwrap_or_else(|| {
            panic!("expected an IN line, got {enter:?} — the gate let two processes interleave:\n{text}")
        });
        let leave_pid = leave.strip_prefix("OUT ").unwrap_or_else(|| {
            panic!("expected an OUT line, got {leave:?} — a second process entered before the first left:\n{text}")
        });
        assert_eq!(
            enter_pid, leave_pid,
            "a different process closed the section that {enter_pid} opened:\n{text}"
        );
    }

    let _ = fs::remove_file(&log);
}
