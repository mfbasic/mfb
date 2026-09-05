//! Regression test for bug-499: a child spawned by `process::spawn` must not
//! inherit the parent's open files and sockets — only the three stdio pipes
//! the spawn deliberately hands over.
//!
//! Before the fix `fs::open` built its `O_*` flag word without `O_CLOEXEC` and
//! the socket helpers created sockets bare (no `SOCK_CLOEXEC` / no
//! `FD_CLOEXEC`), so every fd the parent held crossed `execvp` into the child:
//! a secret file stayed readable through `/dev/fd/N`, and a listening or
//! connected socket (including a TLS socket's transport fd) could be used by
//! the child. `process::spawn` itself already closed the *pipe* ends it dup'd,
//! which is exactly why the leak went unnoticed — the child's own stdio was
//! correct while everything else leaked.
//!
//! The probe is a tiny C program the child runs: it `fstat`s every fd from 3
//! up and reports the kind of each open one. Two runs share it:
//!
//!   * **negative** — the parent opens a regular file and a TCP listener, then
//!     spawns the probe. The probe must see NO open fd above 2.
//!   * **positive** — the same spawn path must still deliver the child's
//!     intended stdin / stdout / stderr: the probe echoes stdin to stdout,
//!     writes a marker to stderr, and exits with a chosen code, all of which
//!     the parent reads back. A "fix" that closed too much (or set CLOEXEC on
//!     a pipe end *after* dup2'ing it onto 0/1/2) fails this half.

#![cfg(unix)]

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// `fdprobe probe` — one line `leaked=<fd>:<kind>,...` (or `leaked=none`).
/// `fdprobe stdio` — echoes stdin (newlines escaped) as `stdin=...` on stdout,
/// `stderr=ok` on stderr, exit code 7.
fn build_probe(root: &Path) -> PathBuf {
    let source = root.join("fdprobe.c");
    fs::write(
        &source,
        r#"
#include <stdio.h>
#include <string.h>
#include <unistd.h>
#include <sys/stat.h>
int main(int argc, char **argv) {
  if (argc > 1 && strcmp(argv[1], "stdio") == 0) {
    char buf[256]; int n = 0, total = 0;
    fputs("stdin=", stdout);
    while ((n = read(0, buf, sizeof buf)) > 0) {
      for (int i = 0; i < n; i++) {
        if (buf[i] == '\n') fputs("\\n", stdout); else fputc(buf[i], stdout);
      }
      total += n;
    }
    fputc('\n', stdout);
    fflush(stdout);
    fputs("stderr=ok\n", stderr);
    return 7;
  }
  int any = 0;
  fputs("leaked=", stdout);
  for (int fd = 3; fd < 1024; fd++) {
    struct stat st;
    if (fstat(fd, &st) != 0) continue;
    const char *kind = "other";
    if (S_ISREG(st.st_mode)) kind = "file";
    else if (S_ISSOCK(st.st_mode)) kind = "socket";
    else if (S_ISFIFO(st.st_mode)) kind = "fifo";
    else if (S_ISDIR(st.st_mode)) kind = "dir";
    else if (S_ISCHR(st.st_mode)) kind = "chr";
    printf("%s%d:%s", any ? "," : "", fd, kind);
    any = 1;
  }
  if (!any) fputs("none", stdout);
  fputc('\n', stdout);
  return 0;
}
"#,
    )
    .expect("write probe source");
    let probe = root.join("fdprobe");
    let output = Command::new("cc")
        .arg("-o")
        .arg(&probe)
        .arg(&source)
        .output()
        .expect("compile probe");
    assert!(
        output.status.success(),
        "cc failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    probe
}

fn scratch(name: &str) -> PathBuf {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("mfb_{name}_{nonce}"));
    fs::create_dir_all(&root).expect("create scratch dir");
    root
}

#[test]
fn spawned_child_sees_no_inherited_file_or_socket() {
    let root = scratch("bug499_neg");
    let probe = build_probe(&root);
    let secret = root.join("secret.txt");
    fs::write(&secret, "top secret\n").expect("write secret");

    // The parent holds a regular file AND a listening TCP socket open across the
    // spawn. Both are `RES` resources that live to the end of `main`, so they are
    // open when the child execs.
    let source = format!(
        r#"IMPORT fs
IMPORT tcp
IMPORT process
IMPORT io

FUNC main AS Integer
  RES secret = fs::openFile("{secret}", "r")
  RES server = tcp::listen("127.0.0.1", 0)
  RES p = process::spawn(["{probe}", "probe"])
  process::close(p)
  LET report = process::receive(p)
  LET code = process::waitFor(p)
  io::print(report)
  io::print("exit=" & toString(code))
  RETURN 0
END FUNC
"#,
        secret = secret.display(),
        probe = probe.display(),
    );
    let project = common::temp_project("bug499_neg", &source);
    let exe = common::build_project(&project);
    let (status, stdout) = common::run_bounded(
        &exe,
        Duration::from_secs(30),
        "bug-499: the fd-probe spawn did not finish",
    );
    assert!(status.success(), "parent exit {status:?}\nstdout:\n{stdout}");
    let report = stdout
        .lines()
        .find(|l| l.starts_with("leaked="))
        .unwrap_or_else(|| panic!("no leaked= line in:\n{stdout}"));
    assert!(
        stdout.contains("exit=0"),
        "probe exit code not 0:\n{stdout}"
    );
    // The only fds the child may hold are 0/1/2 (the pipes dup2'd onto them).
    assert_eq!(
        report, "leaked=none",
        "bug-499: the spawned child inherited the parent's descriptors:\n{stdout}"
    );
}

#[test]
fn spawned_child_still_receives_its_stdio() {
    let root = scratch("bug499_pos");
    let probe = build_probe(&root);
    let source = format!(
        r#"IMPORT process
IMPORT io

FUNC main AS Integer
  RES p = process::spawn(["{probe}", "stdio"])
  process::send(p, "hello")
  process::close(p)
  LET out = process::receive(p)
  LET err = process::receive(p, process::Stream.StdErr)
  LET code = process::waitFor(p)
  io::print(out)
  io::print(err)
  io::print("exit=" & toString(code))
  RETURN 0
END FUNC
"#,
        probe = probe.display(),
    );
    let project = common::temp_project("bug499_pos", &source);
    let exe = common::build_project(&project);
    let (status, stdout) = common::run_bounded(
        &exe,
        Duration::from_secs(30),
        "bug-499: the stdio spawn did not finish",
    );
    assert!(status.success(), "parent exit {status:?}\nstdout:\n{stdout}");
    // `process::send` appends the line terminator, so the child saw "hello\n".
    assert!(
        stdout.contains("stdin=hello\\n"),
        "child did not receive its stdin:\n{stdout}"
    );
    assert!(
        stdout.contains("stderr=ok"),
        "child's stderr did not reach the parent:\n{stdout}"
    );
    assert!(
        stdout.contains("exit=7"),
        "child's exit code was not reported:\n{stdout}"
    );
}
