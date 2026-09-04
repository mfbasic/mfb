//! Regression test for bug-500: `process::spawn(..., envReplace=TRUE)` must
//! terminate when the parent's own `environ` carries an entry that `unsetenv`
//! cannot remove.
//!
//! The fork child clears the inherited environment by deriving each
//! `environ[0]` entry's name (scan to `=` or NUL) and calling `unsetenv(name)`,
//! then restarting from `environ[0]`. That terminates only if every call
//! actually shrinks the array. Two entry shapes a launcher can hand a process
//! break it — the kernel does not validate `envp` strings:
//!
//!   * `"BOGUS_NO_EQUALS"` — no `=`, so the name is the whole string and
//!     `unsetenv` matches nothing;
//!   * `"=C:=C:\\"` — a leading `=`, so the name is empty and `unsetenv("")`
//!     fails `EINVAL` on both glibc and Darwin.
//!
//! Either way `environ[0]` never changes, the child spins forever, and each
//! iteration arena-allocates a name buffer that is (by design) never freed in
//! the fork child — the parent blocks in `read()` on the exec self-pipe while
//! the child's RSS climbs at ~1 GB/s. The fix advances by index and skips an
//! entry that has nothing to unset.
//!
//! `std::process::Command` can only build `KEY=VALUE` entries, so the program is
//! exec'd through a tiny C launcher that passes a raw `envp`. The run is bounded
//! and, on timeout, the whole process GROUP is killed — a bare `child.kill()`
//! would take the parent and leave the spinning fork child alive.

#![cfg(unix)]

mod common;

use std::fs;
use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The child checks that the map entry arrived and that the replace really
/// cleared the launcher-supplied marker variable that FOLLOWS the bogus entry
/// (so the clear loop ran to completion, not just past the bogus entry).
/// `/bin/sh` is an absolute path and `[` is a builtin, so the child needs no
/// `PATH` — and `PATH` itself is deliberately not asserted on: a shell started
/// without one synthesizes a default, so `[ -z "$PATH" ]` is false even when
/// the environment really was cleared.
const SOURCE: &str = r#"IMPORT process
IMPORT io

FUNC main AS Integer
  RES p = process::spawn(["/bin/sh", "-c", "[ x$FOO = xbar ] && [ -z \"$MFB_BUG500_INHERITED\" ]"], "", Map OF String TO String { "FOO" := "bar" }, TRUE)
  io::print("child=" & toString(process::waitFor(p)))
  RETURN 0
END FUNC
"#;

/// `launcher <envp-entry> <exe>` — execs `exe` with
/// `envp = { entry, "MFB_BUG500_INHERITED=1", NULL }`.
fn build_launcher(root: &Path) -> PathBuf {
    let source = root.join("launcher.c");
    fs::write(
        &source,
        r#"
#include <stdio.h>
#include <unistd.h>
int main(int argc, char **argv) {
  if (argc < 3) { return 2; }
  char *envp[] = { argv[1], "MFB_BUG500_INHERITED=1", NULL };
  execve(argv[2], argv + 2, envp);
  perror("execve");
  return 127;
}
"#,
    )
    .expect("write launcher source");
    let launcher = root.join("launcher");
    let output = Command::new("cc")
        .arg("-o")
        .arg(&launcher)
        .arg(&source)
        .output()
        .expect("compile launcher");
    assert!(
        output.status.success(),
        "launcher build failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    launcher
}

/// Run `launcher entry exe` in its own process group; on timeout kill the whole
/// group (parent AND the wedged fork child) and panic.
fn run_group_bounded(launcher: &Path, entry: &str, exe: &Path, timeout: Duration) -> (i32, String) {
    let mut child = Command::new(launcher)
        .arg(entry)
        .arg(exe)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .expect("spawn launcher");
    let pid = child.id();
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().expect("try_wait") {
            let mut stdout = String::new();
            if let Some(mut pipe) = child.stdout.take() {
                pipe.read_to_string(&mut stdout).ok();
            }
            return (status.code().unwrap_or(-1), stdout);
        }
        if start.elapsed() > timeout {
            // Negative pid = the process group the launcher started.
            let _ = Command::new("kill")
                .args(["-9", &format!("-{pid}")])
                .status();
            let _ = child.wait();
            panic!(
                "bug-500: process::spawn(envReplace=TRUE) did not finish within {timeout:?} \
                 with envp entry {entry:?} — the env-clear loop spun on an entry unsetenv \
                 cannot remove"
            );
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn spawn_env_replace_terminates_with(entry: &str) {
    let project = common::temp_project("bug500_env_clear", SOURCE);
    let exe = common::build_project(&project);
    let launcher = build_launcher(&project);
    let (code, stdout) = run_group_bounded(&launcher, entry, &exe, Duration::from_secs(5));
    assert_eq!(code, 0, "program exited {code}; stdout:\n{stdout}");
    assert_eq!(
        stdout, "child=0\n",
        "envReplace must still clear the inherited marker variable and apply FOO=bar"
    );
}

#[test]
fn env_replace_terminates_on_an_environ_entry_with_no_equals() {
    spawn_env_replace_terminates_with("BOGUS_NO_EQUALS");
}

#[test]
fn env_replace_terminates_on_an_environ_entry_with_a_leading_equals() {
    spawn_env_replace_terminates_with("=C:=C:\\");
}
