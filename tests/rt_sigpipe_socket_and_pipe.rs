//! bug-467: a peer that closes its end of a TCP socket must not be able to kill
//! the MFBASIC process with `SIGPIPE`.
//!
//! POSIX delivers `SIGPIPE` to a process that writes to a socket whose peer has
//! sent an RST, and the default disposition terminates it. Nothing in a generated
//! program used to override that, so the *second* `tcp::write` after a peer's
//! close ended the whole process with signal 13 — no `TRAP` ran, no scope drop
//! ran, `main` never returned, and the shell reported 141. Since a server's peer
//! is untrusted input, that is a remote denial of service: any client that
//! connects and immediately disconnects ends the server, taking every other
//! in-flight connection with it.
//!
//! Two halves, deliberately in one file because they are two directions of the
//! SAME decision (`signal(SIGPIPE, SIG_IGN)` at program entry):
//!
//! * [`tcp_write_to_closed_peer_raises_instead_of_killing_the_process`] is the
//!   bug: the write must surface as `ErrConnectionClosed` and the process must
//!   survive to run its `TRAP`.
//! * [`stdout_write_to_a_closed_pipe_still_dies_by_sigpipe`] is the part the fix
//!   must NOT change. `prog | head` is supposed to end when `head` exits, and it
//!   ends because the writer dies by `SIGPIPE`. A process-wide `SIG_IGN` would
//!   silently convert that into an `ErrWriteFailed` raise (stderr noise and a
//!   wrong exit status in every pipeline), so the `io::` stdout path restores
//!   `SIG_DFL` and re-raises when — and only when — its write fails with `EPIPE`.
//!   This test passed before the fix and must keep passing after it.

#![cfg(unix)]

mod common;

use std::io::Read;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// SIGPIPE. 13 on Linux, macOS and every other POSIX host this compiler targets.
const SIGPIPE: i32 = 13;

fn build_project(name: &str, source: &str) -> PathBuf {
    let nonce = common::unique_nonce();
    let root = std::env::temp_dir().join(format!("mfb_{name}_{nonce}"));
    std::fs::create_dir_all(root.join("src")).expect("create temp project");
    std::fs::write(
        root.join("project.json"),
        format!(
            "{{\"name\":\"{name}\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"executable\",\"sources\":[{{\"root\":\"src\",\"role\":\"main\",\"include\":[\"**/*.mfb\"]}}],\"entry\":\"main\",\"targets\":[\"native\"]}}\n"
        ),
    )
    .expect("write project.json");
    std::fs::write(root.join("src/main.mfb"), source).expect("write source");

    let output = Command::new(common::mfb_exe())
        .arg("build")
        .arg("-q")
        .arg(&root)
        .output()
        .expect("run mfb build");
    assert!(
        output.status.success(),
        "mfb build failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    executable_in(&root.join("build"))
}

/// The console executable this host can run.
///
/// macOS and Windows emit exactly one. A Linux console build emits one per libc
/// world — `<name>-glibc.out` *and* `<name>-musl.out` (plan-56-B) — and only the
/// one matching this host's loader will start, so narrow to it before insisting
/// on a single answer.
fn executable_in(dir: &Path) -> PathBuf {
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|err| panic!("read {}: {err}", dir.display()))
        .map(|entry| entry.expect("dir entry").path())
        .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("out"))
        .collect();
    if found.len() > 1 {
        let flavor = format!("-{}", common::host_libc_flavor());
        found.retain(|path| {
            path.file_stem()
                .is_some_and(|stem| stem.to_string_lossy().ends_with(&flavor))
        });
    }
    assert_eq!(
        found.len(),
        1,
        "expected one runnable .out in {}",
        dir.display()
    );
    found.pop().expect("one executable")
}

/// The bug. The peer closes, then the program writes repeatedly. The first write
/// is accepted by the local stack; the peer answers with an RST; a later write
/// must return `EPIPE` so the emitter's existing errno classification can raise
/// `ErrConnectionClosed`, instead of the process being killed before `write`
/// returns at all.
///
/// A single write after the close SUCCEEDS, so a one-write probe cannot see this
/// — the loop below writes up to 200 times.
#[test]
fn tcp_write_to_closed_peer_raises_instead_of_killing_the_process() {
    let source = "\
IMPORT net
IMPORT tcp
IMPORT io

FUNC probe AS String
  RES server = tcp::listen(\"127.0.0.1\", 0)
  LET bound = tcp::localAddress(server)
  RES client = tcp::connect(\"127.0.0.1\", bound.port)
  RES conn = tcp::accept(server)
  tcp::close(client)
  MUT n = 0
  FOR i = 1 TO 200
    tcp::write(conn, \"xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\")
    n = n + 1
  NEXT
  RETURN \"no-raise-after-\" & toString(n) & \"-writes\"
  TRAP(e)
    RETURN \"raised-after-\" & toString(n) & \"-writes\"
  END TRAP
END FUNC

FUNC main AS Integer
  io::print(probe())
  RETURN 0
END FUNC
";
    let exe = build_project("bug467_tcp_write", source);

    // Run it repeatedly, and require EVERY run to survive. One run is not enough
    // to gate this: whether a given run dies depends on the peer's RST racing the
    // write loop. Measured against a compiler built from clean `main` -- the bug
    // still alive -- this shape is killed by SIGPIPE on about 8 runs in 10; the
    // other 2 surface `ECONNRESET` instead, which raises without a signal and is
    // indistinguishable from a pass. A single-run assertion therefore misses the
    // regression roughly a fifth of the time. Ten runs cut that to ~1e-7.
    //
    // The small 32-byte write is deliberate and is the opposite choice from
    // `rt-behavior/tcp/tcp-write-peer-closed-raises-rt`, which writes 64 KiB
    // chunks so that its output is stable enough to pin in a golden. Small writes
    // are the shape MORE likely to take the signal per run, so they belong here,
    // where the assertion is "no run was ever signalled" rather than a fixed
    // string.
    const RUNS: usize = 10;
    let mut raised = 0;
    for run in 1..=RUNS {
        let output = Command::new(&exe).output().expect("run program");
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();

        // The contract, asserted on EVERY run: a peer's behaviour never delivers a
        // signal, and the program gets to finish.
        assert_eq!(
            output.status.signal(),
            None,
            "run {run}/{RUNS}: killed by a signal ({:?}) instead of raising; stdout:\n{stdout}",
            output.status.signal(),
        );
        assert_eq!(
            output.status.code(),
            Some(0),
            "run {run}/{RUNS}: exited {:?}; stdout:\n{stdout}",
            output.status.code(),
        );
        if stdout.starts_with("raised-after-") {
            raised += 1;
        }
    }

    // Whether any individual run raises is NOT assertable: with writes this small
    // every one can be absorbed by the local send buffer before the peer's RST
    // arrives, and then the loop completes normally. That is a legitimate outcome
    // of a correct build -- demanding a raise per run is what made the sibling
    // golden fixture go red under the load of a full acceptance sweep.
    //
    // What must hold is that the failure path is exercised at all, so that these
    // ten runs cannot silently degenerate into ten runs that never touch it and
    // pass for the wrong reason.
    assert!(
        raised > 0,
        "none of {RUNS} runs saw the write fail, so the raise path was never \
         exercised; the probe has stopped reproducing the condition",
    );
}

/// The half the fix must not break: a CLI whose stdout pipe closes still dies by
/// `SIGPIPE`, which is what makes `prog | head` terminate.
#[test]
fn stdout_write_to_a_closed_pipe_still_dies_by_sigpipe() {
    let source = "\
IMPORT io

FUNC main AS Integer
  FOR i = 1 TO 500000
    io::print(\"line \" & toString(i))
  NEXT
  RETURN 0
END FUNC
";
    let exe = build_project("bug467_stdout_pipe", source);
    let mut child = Command::new(&exe)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn program");

    // Read a little, then close the read end — the writer's next write hits a
    // pipe with no reader.
    {
        let mut out = child.stdout.take().expect("piped stdout");
        let mut buf = [0u8; 64];
        let _ = out.read(&mut buf).expect("read from program");
    }
    let mut stderr = String::new();
    if let Some(mut handle) = child.stderr.take() {
        let _ = handle.read_to_string(&mut stderr);
    }
    let status = child.wait().expect("wait for program");
    assert_eq!(
        status.signal(),
        Some(SIGPIPE),
        "writing to a closed stdout pipe must still terminate by SIGPIPE \
         (status {status:?}); stderr:\n{stderr}",
    );
}

/// A spawned child must NOT inherit the parent's ignored SIGPIPE.
///
/// POSIX resets *caught* signals across `exec` but carries an **ignored**
/// disposition through unchanged — measured here, not assumed:
/// `sh -c 'trap "" PIPE; perl -e "print \$SIG{PIPE}"'` prints `IGNORE`. So without
/// an explicit reset in the fork child, every program an MFBASIC process spawns
/// would silently lose its own `prog | head` behaviour. `process::spawn` puts
/// SIGPIPE back to `SIG_DFL` between `fork` and `execvp`.
#[test]
fn a_spawned_child_does_not_inherit_the_ignored_sigpipe() {
    if which("perl").is_none() {
        eprintln!("skipping: perl is not on PATH");
        return;
    }
    let source = "\
IMPORT io
IMPORT process

FUNC main AS Integer
  RES p = process::spawn([\"perl\", \"-e\", \"print(($SIG{PIPE} // qq(DEFAULT)))\"])
  LET out AS String = process::receive(p)
  io::print(out)
  RETURN 0
END FUNC
";
    let exe = build_project("bug467_spawn_sigpipe", source);
    let output = Command::new(&exe).output().expect("run program");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    assert_eq!(
        stdout.trim(),
        "DEFAULT",
        "spawned child inherited a non-default SIGPIPE disposition; stdout:\n{stdout}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
}

fn which(program: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|dir| dir.join(program))
            .find(|candidate| candidate.is_file())
    })
}
