//! Regression tests for bug-543: `process::spawn` must hand the child ONLY the
//! descriptors the spawn deliberately gives it — on every platform, and
//! including descriptors MFBASIC never opened.
//!
//! bug-499 made every fd the MFBASIC *runtime* opens close-on-exec, which covers
//! everything MFBASIC opened and nothing else. A descriptor the MFB program's own
//! launcher left inheritable (a CI runner's pipes at fds 142/145 were the real
//! case) therefore passed straight through `process::spawn` into the child on
//! Unix, while the Windows path — `bInheritHandles = FALSE` plus an explicit
//! `PROC_THREAD_ATTRIBUTE_LIST` — already refused it. The guarantee is now the
//! same shape everywhere:
//!
//!   * **Linux** — `close_range(4, ~0u, 0)` in the fork child, after the self-pipe
//!     write end has been moved to fd 3.
//!   * **macOS** — `posix_spawn` with `POSIX_SPAWN_CLOEXEC_DEFAULT`: the kernel
//!     hands over exactly the descriptors named in the file-actions.
//!   * **Windows** — unchanged; it was already exhaustive.
//!
//! Three cases, and the last two are the ones that catch a careless fix:
//!
//!   * **ambient** — the parent is launched holding two inheritable descriptors
//!     it never opened. The child must see none of them. (RED before the fix.)
//!   * **positive** — the same spawn must still deliver the child's stdin,
//!     stdout, stderr and exit code. A fix that closes everything is trivially
//!     "secure" and useless.
//!
//! Both of those run over all three routes into the shared spawn tail —
//! `process::spawn(argv)`, the four-argument `process::spawn` (which emits extra
//! child-side `chdir`/`setenv` work between the handover and the exec), and
//! `process::shell` — because they are three separate emissions of it.
//!   * **signal disposition** — an *ignored* signal survives `exec` (only caught
//!     ones are reset), and MFBASIC's entry installs `signal(SIGPIPE, SIG_IGN)`
//!     process-wide (bug-467). Moving macOS off `fork`/`exec` deletes the
//!     "between" where the child-side reset to `SIG_DFL` lived, so the reset has
//!     to move into `POSIX_SPAWN_SETSIGDEF`. Miss it and the child silently
//!     inherits an ignored `SIGPIPE`: `prog | head` hangs forever instead of the
//!     writer dying. Both halves are pinned — the disposition the child reads
//!     back, and the pipeline actually terminating.

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

/// The probe the spawned child runs.
///
/// * `fds`     — `leaked=<fd>:<kind>,…` (or `leaked=none`) for every open fd from
///               3 up, then echoes stdin as `stdin=…`, writes `stderr=ok` to
///               stderr, and exits 7.
/// * `sigpipe` — reports the inherited disposition of SIGPIPE/SIGINT/SIGTERM.
/// * `spam`    — writes to stdout forever, ignoring write errors. With SIGPIPE at
///               its default disposition a closed reader kills it; with an
///               inherited `SIG_IGN` it spins until something kills it.
fn build_probe(root: &Path) -> PathBuf {
    let source = root.join("fdprobe543.c");
    fs::write(
        &source,
        r#"
#include <stdio.h>
#include <string.h>
#include <unistd.h>
#include <signal.h>
#include <sys/stat.h>

static const char *disposition(int signo) {
  struct sigaction sa;
  if (sigaction(signo, NULL, &sa) != 0) return "error";
  if (sa.sa_handler == SIG_IGN) return "ignored";
  if (sa.sa_handler == SIG_DFL) return "default";
  return "handler";
}

int main(int argc, char **argv) {
  const char *mode = argc > 1 ? argv[1] : "fds";
  if (strcmp(mode, "sigpipe") == 0) {
    printf("sigpipe=%s sigint=%s sigterm=%s\n",
           disposition(SIGPIPE), disposition(SIGINT), disposition(SIGTERM));
    fflush(stdout);
    return 0;
  }
  if (strcmp(mode, "spam") == 0) {
    char buf[1024];
    memset(buf, 'x', sizeof buf);
    for (;;) {
      if (write(1, buf, sizeof buf) < 0) { /* EPIPE swallowed on purpose */ }
    }
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
  char in[256];
  int n = 0;
  fputs("stdin=", stdout);
  while ((n = read(0, in, sizeof in)) > 0) {
    for (int i = 0; i < n; i++) {
      if (in[i] == '\n') fputs("\\n", stdout); else fputc(in[i], stdout);
    }
  }
  fputc('\n', stdout);
  fflush(stdout);
  fputs("stderr=ok\n", stderr);
  return 7;
}
"#,
    )
    .expect("write probe source");
    let probe = root.join("fdprobe543");
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
    let nonce = common::unique_nonce();
    let root = std::env::temp_dir().join(format!("mfb_{name}_{nonce}"));
    fs::create_dir_all(&root).expect("create scratch dir");
    root
}

/// Run `executable` while handing it two **ambient inheritable** descriptors at
/// fds 142 and 145 — descriptors MFBASIC never opened and never marked
/// close-on-exec, exactly the shape a CI runner leaks into every process it
/// starts. This is the inverse of `common::run_bounded_without_inherited_fds`,
/// which scrubs them: here the leak is the input to the measurement.
fn run_with_ambient_fds(
    executable: &Path,
    donor: &Path,
    timeout: Duration,
    hang_context: &str,
) -> (ExitStatus, String) {
    use std::os::unix::io::AsRawFd;
    use std::os::unix::process::CommandExt;

    let donor = fs::File::open(donor).expect("open ambient donor file");
    let donor_fd = donor.as_raw_fd();
    let mut command = Command::new(executable);
    if let Some(dir) = executable.parent() {
        command.current_dir(dir);
    }
    // SAFETY: the closure calls only `dup2` and `fcntl`, both async-signal-safe,
    // and allocates nothing. `dup2` clears FD_CLOEXEC on the new descriptor, so
    // 142/145 are inheritable in the child by construction.
    unsafe {
        command.pre_exec(move || {
            // Scrub whatever this test binary itself was handed first, so the
            // measurement sees only the two descriptors this test injects.
            for fd in 3..1024 {
                let flags = libc::fcntl(fd, libc::F_GETFD);
                if flags >= 0 && flags & libc::FD_CLOEXEC == 0 && fd != donor_fd {
                    libc::close(fd);
                }
            }
            if libc::dup2(donor_fd, 142) < 0 || libc::dup2(donor_fd, 145) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            libc::close(donor_fd);
            Ok(())
        });
    }
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn executable");
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().expect("try_wait") {
            let mut stdout = String::new();
            if let Some(mut pipe) = child.stdout.take() {
                use std::io::Read;
                pipe.read_to_string(&mut stdout).ok();
            }
            return (status, stdout);
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            panic!(
                "executable {} did not finish within {timeout:?} — {hang_context}",
                executable.display(),
            );
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// The three ways an MFBASIC program reaches the shared spawn tail. They are three
/// separate emissions of it — `spawn` and `shell` are different helpers, and the
/// four-argument `spawn` (`process.spawnEnv`) emits extra child-side work between
/// the descriptor handover and the exec — so each is measured, not just the first.
#[derive(Clone, Copy)]
enum SpawnForm {
    Argv,
    ArgvCwdEnv,
    Shell,
}

impl SpawnForm {
    fn tag(self) -> &'static str {
        match self {
            SpawnForm::Argv => "argv",
            SpawnForm::ArgvCwdEnv => "argvenv",
            SpawnForm::Shell => "shell",
        }
    }

    /// The MFBASIC line that starts the probe, binding it to `p`.
    fn start_line(self, probe: &Path) -> String {
        let probe = probe.display();
        match self {
            SpawnForm::Argv => format!(r#"  RES p = process::spawn(["{probe}", "fds"])"#),
            SpawnForm::ArgvCwdEnv => format!(
                r#"  RES p = process::spawn(["{probe}", "fds"], "", Map OF String TO String {{ "MFB543" := "x" }}, FALSE)"#
            ),
            SpawnForm::Shell => format!(r#"  RES p = process::shell("'{probe}' fds")"#),
        }
    }
}

/// The parent program for the ambient/positive halves: it starts the probe, feeds
/// it a line on stdin, and reports everything the child produced. One program
/// proves both directions — what the child must NOT have (fds 142/145) and what
/// it MUST have (stdin, stdout, stderr, exit code).
fn probe_parent_source(probe: &Path, form: SpawnForm) -> String {
    format!(
        r#"IMPORT process
IMPORT io

FUNC main AS Integer
{start}
  process::send(p, "hello")
  process::closeInput(p)
  LET fds = process::receive(p)
  LET echo = process::receive(p)
  LET err = process::receive(p, process::Stream.StdErr)
  LET code = process::waitFor(p)
  io::print(fds)
  io::print(echo)
  io::print(err)
  io::print("exit=" & toString(code))
  RETURN 0
END FUNC
"#,
        start = form.start_line(probe),
    )
}

fn assert_child_got_its_stdio(stdout: &str) {
    assert!(
        stdout.contains("stdin=hello\\n"),
        "the child did not receive its stdin:\n{stdout}"
    );
    assert!(
        stdout.contains("stderr=ok"),
        "the child did not receive its stderr:\n{stdout}"
    );
    assert!(
        stdout.contains("exit=7"),
        "the child's exit code did not come back:\n{stdout}"
    );
}

fn leaked_line(stdout: &str) -> &str {
    stdout
        .lines()
        .find(|l| l.starts_with("leaked="))
        .unwrap_or_else(|| panic!("no leaked= line in:\n{stdout}"))
}

/// bug-543: two descriptors the launcher left inheritable must NOT reach the
/// child. RED before the fix (`leaked=142:file,145:file`).
#[test]
fn spawned_child_sees_no_ambient_inherited_fd() {
    let root = scratch("bug543_ambient");
    let probe = build_probe(&root);
    let donor = root.join("ambient.txt");
    fs::write(&donor, "ambient\n").expect("write donor");
    for form in [SpawnForm::Argv, SpawnForm::ArgvCwdEnv, SpawnForm::Shell] {
        let name = format!("bug543_ambient_{}", form.tag());
        let project = common::temp_project(&name, &probe_parent_source(&probe, form));
        let exe = common::build_project(&project);

        let (status, stdout) = run_with_ambient_fds(
            &exe,
            &donor,
            Duration::from_secs(30),
            "bug-543: the ambient-fd spawn did not finish",
        );
        assert!(
            status.success(),
            "{name}: parent exit {status:?}\nstdout:\n{stdout}"
        );
        assert_eq!(
            leaked_line(&stdout),
            "leaked=none",
            "bug-543 [{name}]: the child inherited descriptors MFBASIC never opened:\n{stdout}"
        );
        // A fix that simply closes everything is useless — the child must still
        // have the three stdio pipes the spawn deliberately handed it.
        assert_child_got_its_stdio(&stdout);
    }
}

/// The positive pin without the ambient descriptors: the ordinary spawn still
/// delivers exactly stdin/stdout/stderr and nothing above 2.
#[test]
fn spawned_child_still_gets_exactly_its_stdio() {
    let root = scratch("bug543_stdio");
    let probe = build_probe(&root);
    for form in [SpawnForm::Argv, SpawnForm::ArgvCwdEnv, SpawnForm::Shell] {
        let name = format!("bug543_stdio_{}", form.tag());
        let project = common::temp_project(&name, &probe_parent_source(&probe, form));
        let exe = common::build_project(&project);

        let (status, stdout) = common::run_bounded_without_inherited_fds(
            &exe,
            Duration::from_secs(30),
            "bug-543: the stdio spawn did not finish",
        );
        assert!(
            status.success(),
            "{name}: parent exit {status:?}\nstdout:\n{stdout}"
        );
        assert_eq!(
            leaked_line(&stdout),
            "leaked=none",
            "{name} stdout:\n{stdout}"
        );
        assert_child_got_its_stdio(&stdout);
    }
}

/// An IGNORED disposition survives `exec`. MFBASIC's entry ignores SIGPIPE
/// process-wide, so without an explicit reset — `signal(SIGPIPE, SIG_DFL)` in the
/// fork child on Linux, `POSIX_SPAWN_SETSIGDEF` in the spawn attributes on macOS
/// where there is no fork child any more — every spawned child would read back
/// `sigpipe=ignored`.
#[test]
fn spawned_child_gets_default_signal_dispositions() {
    let root = scratch("bug543_sigdisp");
    let probe = build_probe(&root);
    let source = format!(
        r#"IMPORT process
IMPORT io

FUNC main AS Integer
  RES p = process::spawn(["{probe}", "sigpipe"])
  process::closeInput(p)
  LET out = process::receive(p)
  LET code = process::waitFor(p)
  io::print(out)
  io::print("exit=" & toString(code))
  RETURN 0
END FUNC
"#,
        probe = probe.display(),
    );
    let project = common::temp_project("bug543_sigdisp", &source);
    let exe = common::build_project(&project);
    let (status, stdout) = common::run_bounded(
        &exe,
        Duration::from_secs(30),
        "bug-543: the signal-disposition spawn did not finish",
    );
    assert!(
        status.success(),
        "parent exit {status:?}\nstdout:\n{stdout}"
    );
    assert!(
        stdout.contains("sigpipe=default sigint=default sigterm=default"),
        "bug-543: the spawned child inherited a non-default signal disposition:\n{stdout}"
    );
    assert!(stdout.contains("exit=0"), "stdout:\n{stdout}");
}

/// The behavioural half of the same trap, and the symptom a user would report: a
/// spawned `writer | head` must END. With an inherited `SIG_IGN` the writer takes
/// EPIPE forever instead of dying, the shell never returns, and `process::waitFor`
/// blocks until this test's timeout fires.
#[test]
fn spawned_child_dies_on_a_closed_pipe() {
    let root = scratch("bug543_sigpipe");
    let probe = build_probe(&root);
    let source = format!(
        r#"IMPORT process
IMPORT io

FUNC main AS Integer
  RES p = process::spawn(["/bin/sh", "-c", "'{probe}' spam | head -c 8 > /dev/null; echo pipeline-done"])
  process::closeInput(p)
  LET out = process::receive(p)
  LET code = process::waitFor(p)
  io::print(out)
  io::print("exit=" & toString(code))
  RETURN 0
END FUNC
"#,
        probe = probe.display(),
    );
    let project = common::temp_project("bug543_sigpipe", &source);
    let exe = common::build_project(&project);
    let (status, stdout) = common::run_bounded(
        &exe,
        Duration::from_secs(30),
        "bug-543: the spawned `writer | head` pipeline never terminated, so the \
         child inherited an ignored SIGPIPE",
    );
    assert!(
        status.success(),
        "parent exit {status:?}\nstdout:\n{stdout}"
    );
    assert!(
        stdout.contains("pipeline-done"),
        "bug-543: the pipeline did not run to completion:\n{stdout}"
    );
}
