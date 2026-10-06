//! bug-967: a re-entered console `term::on` must not orphan the shadow grid.
//!
//! `term::on` allocates the console shadow-grid block (back + front + output buffer
//! + EGC pool, sized from the terminal) and stores it in term-state slot 48. A
//! second `term::on` while TUI mode was already on allocated ANOTHER block and
//! overwrote the slot, so the previous one was unreachable; `term::off` freed only
//! the last. `mfb spec app term-backend`, "Console shadow-grid header block":
//! `term::off` frees the block — so after one `off`, however many `on`s ran, the
//! `--debug` report must show no live arena bytes.
//!
//! The program runs on a real pty (the grid is a TUI-mode structure; `term::on`
//! sizes it from the terminal).

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::path::PathBuf;
use std::process::Command;

fn build_debug(name: &str, source: &str) -> PathBuf {
    let project = common::temp_project(name, source);
    let output = Command::new(common::mfb_exe())
        .arg("build")
        .arg("--debug")
        .arg(&project)
        .output()
        .expect("run mfb build");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success(),
        "{name} failed to build:\nstdout:\n{stdout}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let flavored = format!("-{}.out", common::host_libc_flavor());
    let written: Vec<&str> = stdout
        .lines()
        .filter_map(|line| line.strip_prefix("Wrote executable to "))
        .collect();
    let chosen = written
        .iter()
        .find(|path| path.ends_with(&flavored))
        .or_else(|| written.first())
        .unwrap_or_else(|| panic!("{name}: no executable in build output:\n{stdout}"));
    PathBuf::from(chosen)
}

/// The value of the debug report's `arena.0.<key>` line in `report`.
fn arena_stat(report: &str, key: &str) -> Option<u64> {
    let prefix = format!("arena.0.{key} ");
    report
        .split(['\r', '\n'])
        .find_map(|line| line.trim().strip_prefix(prefix.as_str()))
        .and_then(|value| value.trim().parse().ok())
}

#[test]
fn n_term_on_calls_then_one_off_leave_no_live_grid() {
    let mut failures = Vec::new();
    for calls in [1, 10, 100] {
        let source = format!(
            "IMPORT term

FUNC main() AS Nothing
  MUT i AS Integer = 0
  WHILE i < {calls}
    term::on()
    i = i + 1
  END WHILE
  term::off()
END FUNC
"
        );
        let exe = build_debug(&format!("term_on_reentry_grid_{calls}"), &source);
        let tty = String::from_utf8_lossy(&common::run_under_pty_bytes(&exe)).into_owned();
        match arena_stat(&tty, "live_bytes") {
            Some(0) => {}
            other => failures.push(format!(
                "{calls} x term::on: want arena.0.live_bytes 0, got {other:?} (alloc_calls {:?}, free_calls {:?})",
                arena_stat(&tty, "alloc_calls"),
                arena_stat(&tty, "free_calls")
            )),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
