#![allow(dead_code)]

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn temp_project(name: &str, source: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock before epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("mfb_{name}_{nonce}"));
    fs::create_dir_all(root.join("src")).expect("create temp project");
    fs::write(
        root.join("project.json"),
        format!(
            "{{\"name\":\"{name}\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"executable\",\"sources\":[{{\"root\":\"src\",\"role\":\"main\",\"include\":[\"**/*.mfb\"]}}],\"entry\":\"main\",\"targets\":[\"native\"]}}\n"
        ),
    )
    .expect("write project.json");
    fs::write(root.join("src/main.mfb"), source).expect("write source");
    root
}

pub fn build_project(project: &Path) -> PathBuf {
    let output = Command::new(env!("CARGO_BIN_EXE_mfb"))
        .arg("build")
        .arg(project)
        .output()
        .expect("run mfb build");
    assert!(
        output.status.success(),
        "build failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("utf8 build output");
    let path = stdout
        .lines()
        .find_map(|line| line.strip_prefix("Wrote executable to "))
        .expect("build output executable path");
    PathBuf::from(path)
}

pub fn run_capture_with_env(executable: &Path, envs: &[(&str, String)]) -> (i32, String, String) {
    let mut command = Command::new(executable);
    for (key, value) in envs {
        command.env(key, value);
    }
    let output = command.output().expect("run executable");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8(output.stdout).expect("utf8 stdout"),
        String::from_utf8(output.stderr).expect("utf8 stderr"),
    )
}

pub fn build_close_interposer(root: &Path) -> PathBuf {
    let source = root.join("fail_close.c");
    fs::write(
        &source,
        r#"
#include <errno.h>
#include <fcntl.h>
#include <stdlib.h>
#include <string.h>
#include <sys/syscall.h>
#include <unistd.h>
#if !defined(__APPLE__)
#include <stdio.h>
#endif

static void mfb_marker_text(const char *text) {
  const char *marker = getenv("MFB_INTERPOSER_MARKER");
  if (marker && marker[0]) {
    int fd = open(marker, O_WRONLY | O_CREAT | O_APPEND, 0600);
    if (fd >= 0) {
      write(fd, text, strlen(text));
      syscall(SYS_close, fd);
    }
  }
}

__attribute__((constructor)) static void mfb_marker(void) {
  const char *marker = getenv("MFB_INTERPOSER_MARKER");
  if (marker && marker[0]) {
    int fd = open(marker, O_WRONLY | O_CREAT | O_TRUNC, 0600);
    if (fd >= 0) {
      syscall(SYS_close, fd);
    }
  }
  mfb_marker_text("loaded\n");
}

static int should_fail_close(int fd) {
  const char *target = getenv("MFB_FAIL_CLOSE_PATH");
  if (target && target[0]) {
    if (strcmp(target, "*") == 0 && fd > 2) {
      mfb_marker_text("fail\n");
      errno = EIO;
      return 1;
    }
    char path[4096];
#if defined(__APPLE__)
    if (fcntl(fd, F_GETPATH, path) == 0 && strcmp(path, target) == 0) {
      errno = EIO;
      return 1;
    }
#else
    char link_path[64];
    snprintf(link_path, sizeof(link_path), "/proc/self/fd/%d", fd);
    ssize_t len = readlink(link_path, path, sizeof(path) - 1);
    if (len >= 0) {
      path[len] = '\0';
      if (strcmp(path, target) == 0) {
        errno = EIO;
        return 1;
      }
    }
#endif
  }
  return 0;
}

static long mfb_close(int fd) {
  if (should_fail_close(fd)) {
    return -1L;
  }
  return syscall(SYS_close, fd);
}

#if defined(__APPLE__)
typedef struct {
  const void *replacement;
  const void *replacee;
} interpose_t;
__attribute__((used)) static const interpose_t interposers[] __attribute__((section("__DATA,__interpose"))) = {
  { (const void *)mfb_close, (const void *)close }
};
#else
int close(int fd) {
  return (int)mfb_close(fd);
}
#endif
"#,
    )
    .expect("write close interposer source");

    let library = if cfg!(target_os = "macos") {
        root.join("libfail_close.dylib")
    } else {
        root.join("libfail_close.so")
    };
    let mut command = Command::new("cc");
    if cfg!(target_os = "macos") {
        command.args(["-dynamiclib", "-o"]);
    } else {
        command.args(["-shared", "-fPIC", "-o"]);
    }
    command.arg(&library).arg(&source);
    if !cfg!(target_os = "macos") {
        command.arg("-ldl");
    }
    let output = command.output().expect("compile close interposer");
    assert!(
        output.status.success(),
        "interposer build failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    library
}

pub fn run_with_stdin(executable: &Path, stdin: &[u8]) -> String {
    let mut child = Command::new(executable)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        // Pipe stderr too: otherwise the child inherits the test harness's fd 2,
        // which is a tty when `cargo test` runs in an interactive terminal, and
        // `io::isErrorTerminal()` then reports TRUE instead of the expected FALSE.
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn executable");
    let mut child_stdin = child.stdin.take().expect("stdin pipe");
    child_stdin.write_all(stdin).expect("write stdin");
    drop(child_stdin);
    let output = child.wait_with_output().expect("wait for executable");
    assert!(
        output.status.success(),
        "program failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("utf8 stdout")
}

pub fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    out
}

pub fn decode_hex(value: &str) -> Vec<u8> {
    fn nibble(byte: u8) -> u8 {
        match byte {
            b'0'..=b'9' => byte - b'0',
            b'a'..=b'f' => byte - b'a' + 10,
            b'A'..=b'F' => byte - b'A' + 10,
            _ => panic!("invalid hex byte {byte}"),
        }
    }

    let bytes = value.as_bytes();
    assert_eq!(bytes.len() % 2, 0, "hex output must have even length");
    bytes
        .chunks_exact(2)
        .map(|pair| (nibble(pair[0]) << 4) | nibble(pair[1]))
        .collect()
}

/// Run `executable` with stdout (fd 1) pointed at a **read-only** descriptor
/// (`/dev/null` opened `O_RDONLY`), then dup'd onto fd 1. Any real `write(1, …)`
/// then fails deterministically with `EBADF` on every platform/libc — unlike a
/// *closed* fd, a valid-but-read-only descriptor cannot be silently reopened or
/// replaced by the runtime/loader, so this is the portable way to exercise the
/// stdout-write failure path (bug-04: `io::flush`/`io::input` detect failures via
/// `write`, not `fsync`).
pub fn run_with_readonly_stdout(executable: &Path, stdin: &[u8]) -> (i32, String, String) {
    let output = Command::new("python3")
        .arg("-c")
        .arg(
            r#"import binascii, os, subprocess, sys
stdin_data = bytes.fromhex(sys.argv[2])

def make_stdout_readonly():
    fd = os.open(os.devnull, os.O_RDONLY)
    os.dup2(fd, 1)
    if fd != 1:
        os.close(fd)

proc = subprocess.Popen(
    [sys.argv[1]],
    stdin=subprocess.PIPE,
    stdout=subprocess.PIPE,
    stderr=subprocess.PIPE,
    preexec_fn=make_stdout_readonly,
)
out, err = proc.communicate(stdin_data)
sys.stdout.write(str(proc.returncode) + "\n")
sys.stdout.write(binascii.hexlify(out).decode("ascii") + "\n")
sys.stdout.write(binascii.hexlify(err).decode("ascii") + "\n")"#,
        )
        .arg(executable)
        .arg(hex(stdin))
        .output()
        .expect("run readonly-stdout helper");

    assert!(
        output.status.success(),
        "readonly-stdout helper failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("utf8 helper output");
    let mut lines = stdout.lines();
    let status = lines
        .next()
        .expect("status line")
        .parse::<i32>()
        .expect("status code");
    let child_stdout =
        String::from_utf8(decode_hex(lines.next().expect("stdout line"))).expect("utf8 stdout");
    let child_stderr =
        String::from_utf8(decode_hex(lines.next().expect("stderr line"))).expect("utf8 stderr");
    (status, child_stdout, child_stderr)
}

pub fn run_under_pty(executable: &Path) -> String {
    let output = Command::new("python3")
        .arg("-c")
        .arg(
            r#"import fcntl, os, pty, struct, subprocess, sys, termios
master, slave = pty.openpty()
fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 100, 0, 0))
proc = subprocess.Popen([sys.argv[1]], stdin=slave, stdout=slave, stderr=slave, close_fds=True)
os.close(slave)
chunks = []
while True:
    try:
        data = os.read(master, 4096)
    except OSError:
        break
    if not data:
        break
    chunks.append(data)
os.close(master)
sys.stdout.buffer.write(b"".join(chunks))
sys.exit(proc.wait())"#,
        )
        .arg(executable)
        .output()
        .expect("run pty helper");

    assert!(
        output.status.success(),
        "pty run failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("utf8 stdout")
}

pub fn run_pty_prompt_interaction(executable: &Path, prompt: &str, input: &[u8]) -> String {
    run_pty_prompt_interaction_inner(executable, prompt, input, false)
}

/// Like `run_pty_prompt_interaction`, but for the echo-suppressing reads
/// (`readChar`/`readByte`/`readLine`): after the prompt appears, wait until the
/// child has actually cleared the terminal `ECHO` flag before injecting input.
///
/// The child writes its prompt with echo still on (the prompt is a *separate*
/// `io::write` statement — the runtime can't know an echo-suppressed read
/// follows), then enters raw mode inside the read. Injecting the reply the
/// instant the prompt is visible races that `tcsetattr`: if the byte lands while
/// echo is still on, the kernel line discipline echoes it and the assertion
/// flakes (deterministically under `cargo llvm-cov`, which perturbs scheduling).
/// Gating the write on `ECHO` being cleared — the child is then blocked in
/// `read()` — closes the window without a timing hack.
pub fn run_pty_prompt_interaction_echo_off(
    executable: &Path,
    prompt: &str,
    input: &[u8],
) -> String {
    run_pty_prompt_interaction_inner(executable, prompt, input, true)
}

pub fn run_pty_prompt_interaction_inner(
    executable: &Path,
    prompt: &str,
    input: &[u8],
    wait_echo_off: bool,
) -> String {
    let output = Command::new("python3")
        .arg("-c")
        .arg(
            r#"import fcntl, os, pty, select, subprocess, sys, termios, time
prompt = sys.argv[2].encode()
reply = bytes.fromhex(sys.argv[3])
wait_echo_off = sys.argv[4] == "1"
master, slave = pty.openpty()
# Keep a spare handle to the slave so we can read its termios after the child
# closes its own copies; closed before the drain loop so `master` still sees EOF.
echo_probe = os.dup(slave)
proc = subprocess.Popen([sys.argv[1]], stdin=slave, stdout=slave, stderr=slave, close_fds=True)
os.close(slave)
chunks = []
seen = b""
deadline = time.time() + 5.0
while prompt not in seen:
    remaining = deadline - time.time()
    if remaining <= 0:
        proc.kill()
        sys.stderr.write("timed out waiting for prompt; saw %r\n" % seen)
        sys.exit(124)
    ready, _, _ = select.select([master], [], [], remaining)
    if not ready:
        continue
    data = os.read(master, 4096)
    if not data:
        break
    chunks.append(data)
    seen += data
if wait_echo_off:
    echo_deadline = time.time() + 5.0
    while True:
        try:
            lflag = termios.tcgetattr(echo_probe)[3]
        except OSError:
            break
        if not (lflag & termios.ECHO):
            break
        if time.time() > echo_deadline:
            proc.kill()
            sys.stderr.write("timed out waiting for child to disable echo\n")
            sys.exit(124)
        time.sleep(0.001)
os.close(echo_probe)
os.write(master, reply)
while True:
    ready, _, _ = select.select([master], [], [], 5.0)
    if not ready:
        proc.kill()
        sys.stderr.write("timed out waiting for process exit\n")
        sys.exit(124)
    try:
        data = os.read(master, 4096)
    except OSError:
        break
    if not data:
        break
    chunks.append(data)
os.close(master)
sys.stdout.buffer.write(b"".join(chunks))
sys.exit(proc.wait())"#,
        )
        .arg(executable)
        .arg(prompt)
        .arg(hex(input))
        .arg(if wait_echo_off { "1" } else { "0" })
        .output()
        .expect("run pty prompt helper");

    assert!(
        output.status.success(),
        "pty prompt run failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("utf8 pty prompt output")
}

/// Build a `write()` interposer that caps every call to 4096 bytes, forcing the
/// short *positive* returns that bug-51's output loops must survive. macOS routes
/// `write` through libSystem, so a `__DATA,__interpose` shim reaches the mfb
/// binary; linux-x86_64 issues a raw `write` syscall that no libc interposer can
/// hook, so this validation is macOS-only.
#[cfg(target_os = "macos")]
pub fn build_short_write_interposer(root: &Path) -> PathBuf {
    let source = root.join("short_write.c");
    fs::write(
        &source,
        r#"
#include <stdlib.h>
#include <sys/syscall.h>
#include <unistd.h>

static long mfb_short_write(int fd, const void *buf, unsigned long n) {
  unsigned long cap = n > 4096 ? 4096 : n;
  return syscall(SYS_write, fd, buf, (size_t)cap);
}

typedef struct {
  const void *replacement;
  const void *replacee;
} interpose_t;
__attribute__((used)) static const interpose_t interposers[] __attribute__((section("__DATA,__interpose"))) = {
  { (const void *)mfb_short_write, (const void *)write }
};
"#,
    )
    .expect("write short-write interposer source");
    let library = root.join("libshort_write.dylib");
    let output = Command::new("cc")
        .args(["-dynamiclib", "-o"])
        .arg(&library)
        .arg(&source)
        .output()
        .expect("compile short-write interposer");
    assert!(
        output.status.success(),
        "interposer build failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    library
}
