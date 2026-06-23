use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_project(name: &str, source: &str) -> PathBuf {
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

fn build_project(project: &Path) -> PathBuf {
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

fn run_capture_with_env(executable: &Path, envs: &[(&str, String)]) -> (i32, String, String) {
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

fn build_close_interposer(root: &Path) -> PathBuf {
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

fn run_with_stdin(executable: &Path, stdin: &[u8]) -> String {
    let mut child = Command::new(executable)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
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

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    out
}

fn decode_hex(value: &str) -> Vec<u8> {
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

fn run_with_closed_fd(executable: &Path, closed_fd: u8, stdin: &[u8]) -> (i32, String, String) {
    let output = Command::new("python3")
        .arg("-c")
        .arg(
            r#"import binascii, os, subprocess, sys
closed_fd = int(sys.argv[2])
stdin_data = bytes.fromhex(sys.argv[3])

def close_requested_fd():
    try:
        os.close(closed_fd)
    except OSError:
        pass

proc = subprocess.Popen(
    [sys.argv[1]],
    stdin=subprocess.PIPE,
    stdout=subprocess.PIPE,
    stderr=subprocess.PIPE,
    preexec_fn=close_requested_fd,
)
out, err = proc.communicate(stdin_data)
sys.stdout.write(str(proc.returncode) + "\n")
sys.stdout.write(binascii.hexlify(out).decode("ascii") + "\n")
sys.stdout.write(binascii.hexlify(err).decode("ascii") + "\n")"#,
        )
        .arg(executable)
        .arg(closed_fd.to_string())
        .arg(hex(stdin))
        .output()
        .expect("run closed-fd helper");

    assert!(
        output.status.success(),
        "closed-fd helper failed:\nstdout:\n{}\nstderr:\n{}",
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

fn run_under_pty(executable: &Path) -> String {
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

fn run_pty_prompt_interaction(executable: &Path, prompt: &str, input: &[u8]) -> String {
    let output = Command::new("python3")
        .arg("-c")
        .arg(
            r#"import fcntl, os, pty, select, subprocess, sys, time
prompt = sys.argv[2].encode()
reply = bytes.fromhex(sys.argv[3])
master, slave = pty.openpty()
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

#[test]
fn native_io_reads_from_stdin_and_preserves_input_semantics() {
    let project = temp_project(
        "native_io_reads_success",
        r#"
IMPORT io

FUNC main AS Integer
  io::print(io::input("name> "))
  io::print(io::readLine())
  io::print(io::readChar())
  io::print(toString(io::readByte()))
  RETURN 0
END FUNC
"#,
    );
    let executable = build_project(&project);
    let stdout = run_with_stdin(&executable, b"Ada\nBeta\n\xc3\xa9Z");
    assert_eq!(stdout, "name> Ada\nBeta\né\n90\n");
}

#[test]
fn native_io_flush_reports_standard_stream_failures() {
    let flush_stdout = temp_project(
        "native_io_flush_stdout_failure",
        r#"
IMPORT io

FUNC main AS Integer
  io::flush()
  RETURN 17
  TRAP(err)
    io::printError(toString(err.code))
    RETURN 0
  END TRAP
END FUNC
"#,
    );
    let flush_stderr = temp_project(
        "native_io_flush_stderr_failure",
        r#"
IMPORT io

FUNC main AS Integer
  io::flushError()
  RETURN 17
  TRAP(err)
    io::print(toString(err.code))
    RETURN 0
  END TRAP
END FUNC
"#,
    );

    let (status, stdout, stderr) = run_with_closed_fd(&build_project(&flush_stdout), 1, b"");
    assert_eq!(status, 0);
    assert_eq!(stdout, "");
    assert_eq!(stderr, "77020002\n");

    let (status, stdout, stderr) = run_with_closed_fd(&build_project(&flush_stderr), 2, b"");
    assert_eq!(status, 0);
    assert_eq!(stdout, "77020002\n");
    assert_eq!(stderr, "");
}

#[test]
fn native_resource_cleanup_reports_secondary_close_failure_metadata() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock before epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("mfb_native_resource_cleanup_{nonce}"));
    fs::create_dir_all(root.join("src")).expect("create temp project");
    let target_file = root.join("data.txt");
    fs::write(&target_file, "data").expect("write target file");
    fs::write(
        root.join("project.json"),
        "{\"name\":\"native_resource_cleanup_failure\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"executable\",\"sources\":[{\"root\":\"src\",\"role\":\"main\",\"include\":[\"**/*.mfb\"]}],\"entry\":\"main\",\"targets\":[\"native\"]}\n",
    )
    .expect("write project.json");
    fs::write(
        root.join("src/main.mfb"),
        format!(
            r#"
IMPORT fs

FUNC main AS Integer
  RES file = fs::openFile("{}")
  FAIL error(1234, "body failed")
END FUNC
"#,
            target_file.display()
        ),
    )
    .expect("write source");

    let executable = build_project(&root);
    let interposer = build_close_interposer(&root);
    let marker = root.join("interposer.loaded");
    let mut envs = vec![("MFB_FAIL_CLOSE_PATH", "*".to_string())];
    envs.push(("MFB_INTERPOSER_MARKER", marker.display().to_string()));
    if cfg!(target_os = "macos") {
        envs.push(("DYLD_INSERT_LIBRARIES", interposer.display().to_string()));
        envs.push(("DYLD_FORCE_FLAT_NAMESPACE", "1".to_string()));
    } else {
        envs.push(("LD_PRELOAD", interposer.display().to_string()));
    }

    let (status, stdout, stderr) = run_capture_with_env(&executable, &envs);
    assert!(marker.exists(), "interposer was not loaded");
    let marker_text = fs::read_to_string(&marker).expect("read marker");
    assert!(
        marker_text.contains("fail\n"),
        "interposer did not fail a close call; marker: {marker_text:?}"
    );
    assert_eq!(status, 255, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert_eq!(stdout, "");
    assert_eq!(
        stderr,
        "Code: 1234 Message: body failed\nCleanup failure: Code: 77030006 Message: close failed\n"
    );
}

#[test]
fn native_exit_program_runs_caller_resource_cleanup() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock before epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("mfb_native_exit_program_cleanup_{nonce}"));
    fs::create_dir_all(root.join("src")).expect("create temp project");
    let target_file = root.join("data.txt");
    fs::write(&target_file, "data").expect("write target file");
    fs::write(
        root.join("project.json"),
        "{\"name\":\"native_exit_program_cleanup\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"executable\",\"sources\":[{\"root\":\"src\",\"role\":\"main\",\"include\":[\"**/*.mfb\"]}],\"entry\":\"main\",\"targets\":[\"native\"]}\n",
    )
    .expect("write project.json");
    fs::write(
        root.join("src/main.mfb"),
        format!(
            r#"
IMPORT fs

FUNC leave AS Nothing
  EXIT PROGRAM 7
END FUNC

FUNC main AS Integer
  RES file = fs::openFile("{}")
  leave()
  RETURN 99
END FUNC
"#,
            target_file.display()
        ),
    )
    .expect("write source");

    let executable = build_project(&root);
    let interposer = build_close_interposer(&root);
    let marker = root.join("interposer.loaded");
    let mut envs = vec![("MFB_FAIL_CLOSE_PATH", "*".to_string())];
    envs.push(("MFB_INTERPOSER_MARKER", marker.display().to_string()));
    if cfg!(target_os = "macos") {
        envs.push(("DYLD_INSERT_LIBRARIES", interposer.display().to_string()));
        envs.push(("DYLD_FORCE_FLAT_NAMESPACE", "1".to_string()));
    } else {
        envs.push(("LD_PRELOAD", interposer.display().to_string()));
    }

    let (status, stdout, stderr) = run_capture_with_env(&executable, &envs);
    assert_eq!(status, 7, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    assert!(marker.exists(), "interposer was not loaded");
    let marker_text = fs::read_to_string(&marker).expect("read marker");
    assert!(
        marker_text.contains("fail\n"),
        "caller resource cleanup did not call close before EXIT PROGRAM; marker: {marker_text:?}"
    );
}

#[test]
fn native_loop_exit_and_continue_run_body_resource_cleanup() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock before epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("mfb_native_loop_cleanup_{nonce}"));
    fs::create_dir_all(root.join("src")).expect("create temp project");
    let target_file = root.join("data.txt");
    fs::write(&target_file, "data").expect("write target file");
    fs::write(
        root.join("project.json"),
        "{\"name\":\"native_loop_cleanup\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"executable\",\"sources\":[{\"root\":\"src\",\"role\":\"main\",\"include\":[\"**/*.mfb\"]}],\"entry\":\"main\",\"targets\":[\"native\"]}\n",
    )
    .expect("write project.json");
    fs::write(
        root.join("src/main.mfb"),
        format!(
            r#"
IMPORT fs

FUNC main AS Integer
  FOR i = 1 TO 1
    RES exitFile = fs::openFile("{}")
    EXIT FOR
  NEXT

  FOR j = 1 TO 1
    RES continueFile = fs::openFile("{}")
    CONTINUE FOR
  NEXT

  RETURN 0
END FUNC
"#,
            target_file.display(),
            target_file.display()
        ),
    )
    .expect("write source");

    let executable = build_project(&root);
    let interposer = build_close_interposer(&root);
    let marker = root.join("interposer.loaded");
    let mut envs = vec![("MFB_FAIL_CLOSE_PATH", "*".to_string())];
    envs.push(("MFB_INTERPOSER_MARKER", marker.display().to_string()));
    if cfg!(target_os = "macos") {
        envs.push(("DYLD_INSERT_LIBRARIES", interposer.display().to_string()));
        envs.push(("DYLD_FORCE_FLAT_NAMESPACE", "1".to_string()));
    } else {
        envs.push(("LD_PRELOAD", interposer.display().to_string()));
    }

    let (status, stdout, stderr) = run_capture_with_env(&executable, &envs);
    assert_eq!(status, 0, "stdout:\n{stdout}\nstderr:\n{stderr}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    assert!(marker.exists(), "interposer was not loaded");
    let marker_text = fs::read_to_string(&marker).expect("read marker");
    let close_failures = marker_text.matches("fail\n").count();
    assert!(
        close_failures >= 2,
        "loop EXIT/CONTINUE cleanup did not close both files; marker: {marker_text:?}"
    );
}

#[test]
fn native_io_input_flushes_stdout_before_reading() {
    let project = temp_project(
        "native_io_input_prompt_flush",
        r#"
IMPORT io

FUNC main AS Integer
  LET name AS String = io::input("name> ")
  io::print("hello " & name)
  RETURN 0
END FUNC
"#,
    );
    let transcript = run_pty_prompt_interaction(&build_project(&project), "name> ", b"Ada\n");
    let normalized = transcript.replace("\r\n", "\n");
    assert!(
        normalized.starts_with("name> "),
        "prompt was not visible before input: {normalized:?}"
    );
    assert!(normalized.contains("hello Ada\n"), "got {normalized:?}");
}

#[test]
fn native_io_input_echoes_terminal_line_input() {
    let project = temp_project(
        "native_io_input_echoes",
        r#"
IMPORT io

FUNC main AS Integer
  LET name AS String = io::input("name> ")
  io::print(toString(len(name)))
  RETURN 0
END FUNC
"#,
    );
    let transcript = run_pty_prompt_interaction(&build_project(&project), "name> ", b"Ada\n");
    let normalized = transcript.replace("\r\n", "\n");
    assert!(
        normalized.contains("name> Ada\n3\n"),
        "input did not echo terminal line input: {normalized:?}"
    );
}

#[test]
fn native_io_readline_suppresses_terminal_echo_until_newline() {
    let project = temp_project(
        "native_io_readline_no_echo",
        r#"
IMPORT io

FUNC main AS Integer
  io::write("line> ")
  LET line AS String = io::readLine()
  io::print(toString(len(line)))
  RETURN 0
END FUNC
"#,
    );
    let transcript = run_pty_prompt_interaction(&build_project(&project), "line> ", b"secret\n");
    let normalized = transcript.replace("\r\n", "\n");
    assert!(
        normalized.contains("line> 6\n"),
        "readLine did not return submitted line: {normalized:?}"
    );
    assert!(
        !normalized.contains("secret"),
        "readLine echoed terminal input: {normalized:?}"
    );
}

#[test]
fn native_io_readchar_reads_terminal_key_without_newline_or_echo() {
    let project = temp_project(
        "native_io_readchar_keypress",
        r#"
IMPORT io

FUNC main AS Integer
  io::write("ready> ")
  LET ch AS String = io::readChar()
  IF ch = "w" THEN
    io::print("up")
  ELSE
    io::print("wrong")
  END IF
  RETURN 0
END FUNC
"#,
    );
    let transcript = run_pty_prompt_interaction(&build_project(&project), "ready> ", b"w");
    let normalized = transcript.replace("\r\n", "\n");
    assert!(
        normalized.contains("ready> up\n"),
        "readChar did not return after one keypress: {normalized:?}"
    );
    assert!(
        !normalized.contains("ready> w"),
        "readChar echoed terminal keypress: {normalized:?}"
    );
}

#[test]
fn native_io_readbyte_reads_terminal_key_without_newline_or_echo() {
    let project = temp_project(
        "native_io_readbyte_keypress",
        r#"
IMPORT io

FUNC main AS Integer
  io::write("ready> ")
  LET byte AS Byte = io::readByte()
  io::print(toString(byte))
  RETURN 0
END FUNC
"#,
    );
    let transcript = run_pty_prompt_interaction(&build_project(&project), "ready> ", b"A");
    let normalized = transcript.replace("\r\n", "\n");
    assert!(
        normalized.contains("ready> 65\n"),
        "readByte did not return after one keypress: {normalized:?}"
    );
    assert!(
        !normalized.contains("ready> A"),
        "readByte echoed terminal keypress: {normalized:?}"
    );
}

#[test]
fn native_io_input_reports_prompt_flush_failure() {
    let project = temp_project(
        "native_io_input_prompt_failure",
        r#"
IMPORT io

FUNC main AS Integer
  io::input()
  RETURN 17
  TRAP(err)
    io::printError(toString(err.code))
    RETURN 0
  END TRAP
END FUNC
"#,
    );
    let (status, stdout, stderr) = run_with_closed_fd(&build_project(&project), 1, b"Ada\n");
    assert_eq!(status, 0);
    assert_eq!(stdout, "");
    assert_eq!(stderr, "77020002\n");
}

#[test]
fn native_io_reports_eof_and_encoding_failures() {
    let read_line = temp_project(
        "native_io_readline_eof",
        r#"
IMPORT io

FUNC readLineWithTrap AS Nothing
  io::print(io::readLine())
  RETURN NOTHING
  TRAP(err)
    io::print(toString(err.code))
    RETURN NOTHING
  END TRAP
END FUNC

FUNC main AS Integer
  readLineWithTrap()
  RETURN 0
END FUNC
"#,
    );
    let read_char = temp_project(
        "native_io_readchar_encoding",
        r#"
IMPORT io

FUNC readCharWithTrap AS Nothing
  io::print(io::readChar())
  RETURN NOTHING
  TRAP(err)
    io::print(toString(err.code))
    RETURN NOTHING
  END TRAP
END FUNC

FUNC main AS Integer
  readCharWithTrap()
  RETURN 0
END FUNC
"#,
    );
    let input = temp_project(
        "native_io_input_encoding",
        r#"
IMPORT io

FUNC inputWithTrap AS Nothing
  io::print(io::input())
  RETURN NOTHING
  TRAP(err)
    io::print(toString(err.code))
    RETURN NOTHING
  END TRAP
END FUNC

FUNC main AS Integer
  inputWithTrap()
  RETURN 0
END FUNC
"#,
    );

    assert_eq!(
        run_with_stdin(&build_project(&read_line), b""),
        "77020003\n"
    );
    assert_eq!(
        run_with_stdin(&build_project(&read_char), b"\x80"),
        "77020004\n"
    );
    assert_eq!(
        run_with_stdin(&build_project(&input), b"\xff\n"),
        "77020004\n"
    );
}

#[test]
fn native_io_terminal_helpers_cover_pipe_and_tty_execution() {
    let project = temp_project(
        "native_io_terminal_helpers",
        r#"
IMPORT io

FUNC main AS Integer
  io::print(toString(io::isInputTerminal()))
  io::print(toString(io::isOutputTerminal()))
  io::print(toString(io::isErrorTerminal()))
  RETURN 0
END FUNC
"#,
    );
    let executable = build_project(&project);

    let direct = run_with_stdin(&executable, b"");
    assert_eq!(direct, "FALSE\nFALSE\nFALSE\n");

    let pty = run_under_pty(&executable);
    let lines = pty
        .replace("\r\n", "\n")
        .lines()
        .map(str::to_string)
        .collect::<Vec<_>>();
    assert!(lines.len() >= 3, "expected tty output, got {lines:?}");
    assert_eq!(lines[0], "TRUE");
    assert_eq!(lines[1], "TRUE");
    assert_eq!(lines[2], "TRUE");
}

#[test]
fn native_term_size_reports_unsupported_off_and_size_when_active() {
    // `term::terminalSize` errors with ERR_UNSUPPORTED_OPERATION while TUI mode
    // is off (plan-01-term.md §4.7) and returns the live window size once active.
    let project = temp_project(
        "native_term_size",
        r#"
IMPORT io
IMPORT term

FUNC sizeWhileOff AS Nothing
  LET size AS TermSize = term::terminalSize()
  io::print("OFF-COLS:" & toString(size.columns))
  RETURN NOTHING
  TRAP(err)
    io::print("OFF-ERR:" & toString(err.code))
    RETURN NOTHING
  END TRAP
END FUNC

FUNC printSize AS Nothing
  LET size AS TermSize = term::terminalSize()
  io::print("SIZE:" & toString(size.columns) & "x" & toString(size.rows))
  RETURN NOTHING
  TRAP(err)
    io::print("SIZE-ERR:" & toString(err.code))
    RETURN NOTHING
  END TRAP
END FUNC

FUNC main AS Integer
  sizeWhileOff()
  term::on()
  printSize()
  term::off()
  RETURN 0
END FUNC
"#,
    );
    let executable = build_project(&project);

    // Piped (no tty): the off-path errors, and the active path also reports
    // unsupported because the ioctl fails on a pipe (program still exits 0
    // because both reads are trapped).
    let direct = run_with_stdin(&executable, b"");
    assert!(
        direct.contains("OFF-ERR:77050007"),
        "expected off-path unsupported error, got {direct:?}"
    );
    assert!(
        direct.contains("SIZE-ERR:77050007"),
        "expected non-tty active terminalSize to report unsupported, got {direct:?}"
    );

    // Under a pty with a known window size, the active path reports it.
    let pty = run_under_pty(&executable).replace("\r\n", "\n");
    assert!(
        pty.contains("OFF-ERR:77050007"),
        "expected off-path unsupported error under pty, got {pty:?}"
    );
    assert!(
        pty.contains("SIZE:100x40"),
        "expected live window size under pty, got {pty:?}"
    );
}

#[test]
fn native_term_console_emits_expected_escape_sequences() {
    // The console backend writes the documented ANSI escapes to stdout while TUI
    // mode is active (plan-01-term.md §10.2). Driven into a pipe (no tty needed).
    let project = temp_project(
        "native_term_escapes",
        r#"
IMPORT term

FUNC main AS Integer
  term::on()
  term::setForeground(0, 255, 0)
  term::setBackground(0, 0, 0)
  term::setBold(TRUE)
  term::setUnderline(TRUE)
  term::moveTo(2, 4)
  term::showCursor()
  term::hideCursor()
  term::clear()
  term::off()
  RETURN 0
END FUNC
"#,
    );
    let executable = build_project(&project);
    let out = run_with_stdin(&executable, b"");
    for needle in [
        "\x1b[?1049h",          // on(): enter the alternate screen
        "\x1b[38;2;0;255;0m",   // setForeground(0,255,0)
        "\x1b[48;2;0;0;0m",     // setBackground(0,0,0)
        "\x1b[1m",              // setBold(TRUE)
        "\x1b[4m",              // setUnderline(TRUE)
        "\x1b[3;5H",            // moveTo(2,4) -> 1-based 3;5
        "\x1b[?25h",            // showCursor()
        "\x1b[?25l",            // hideCursor()
        "\x1b[2J\x1b[H",        // clear()
        "\x1b[?1049l",          // off(): leave the alternate screen
    ] {
        assert!(
            out.contains(needle),
            "missing escape {:?} in output {:?}",
            needle,
            hex(out.as_bytes())
        );
    }
}

#[test]
fn native_term_gate_no_ops_while_inactive() {
    // Every term:: call except on()/isOn() is inert while TUI mode is off
    // (plan-01-term.md §4.2.1): setters/surface calls emit nothing, getters return
    // the inert default, isOn() is FALSE.
    let project = temp_project(
        "native_term_gate",
        r#"
IMPORT io
IMPORT term

FUNC main AS Integer
  term::setForeground(1, 2, 3)
  term::setBackground(4, 5, 6)
  term::setBold(TRUE)
  term::setUnderline(TRUE)
  term::moveTo(5, 5)
  term::clear()
  term::showCursor()
  term::hideCursor()
  LET on AS Boolean = term::isOn()
  LET fg AS TermColor = term::getForeground()
  LET bg AS TermColor = term::getBackground()
  LET bold AS Boolean = term::getBold()
  LET ul AS Boolean = term::getUnderline()
  io::print("ON:" & toString(on))
  io::print("FG:" & toString(fg.r) & "," & toString(fg.g) & "," & toString(fg.b))
  io::print("BG:" & toString(bg.r) & "," & toString(bg.g) & "," & toString(bg.b))
  io::print("BOLD:" & toString(bold))
  io::print("UL:" & toString(ul))
  RETURN 0
END FUNC
"#,
    );
    let executable = build_project(&project);
    let out = run_with_stdin(&executable, b"");
    assert!(
        !out.contains('\x1b'),
        "inactive term:: leaked escape bytes: {:?}",
        hex(out.as_bytes())
    );
    assert!(out.contains("ON:FALSE"), "isOn should be FALSE while off: {out:?}");
    assert!(out.contains("FG:255,255,255"), "inert fg should be white: {out:?}");
    assert!(out.contains("BG:0,0,0"), "inert bg should be black: {out:?}");
    assert!(out.contains("BOLD:FALSE"), "inert bold should be FALSE: {out:?}");
    assert!(out.contains("UL:FALSE"), "inert underline should be FALSE: {out:?}");
}

#[test]
fn native_term_on_resets_state_to_defaults() {
    // on() resets all state to defaults every time it is called (plan-01-term.md
    // §4.2): set non-defaults, off(), on() again, read the defaults back.
    let project = temp_project(
        "native_term_reset",
        r#"
IMPORT io
IMPORT term

FUNC main AS Integer
  term::on()
  term::setForeground(10, 20, 30)
  term::setBackground(40, 50, 60)
  term::setBold(TRUE)
  term::setUnderline(TRUE)
  term::off()
  term::on()
  LET fg AS TermColor = term::getForeground()
  LET bg AS TermColor = term::getBackground()
  LET bold AS Boolean = term::getBold()
  LET ul AS Boolean = term::getUnderline()
  LET on AS Boolean = term::isOn()
  term::off()
  io::print("FG:" & toString(fg.r) & "," & toString(fg.g) & "," & toString(fg.b))
  io::print("BG:" & toString(bg.r) & "," & toString(bg.g) & "," & toString(bg.b))
  io::print("BOLD:" & toString(bold))
  io::print("UL:" & toString(ul))
  io::print("ON:" & toString(on))
  RETURN 0
END FUNC
"#,
    );
    let executable = build_project(&project);
    let out = run_with_stdin(&executable, b"");
    assert!(out.contains("FG:255,255,255"), "on() should reset fg to white: {out:?}");
    assert!(out.contains("BG:0,0,0"), "on() should reset bg to black: {out:?}");
    assert!(out.contains("BOLD:FALSE"), "on() should reset bold: {out:?}");
    assert!(out.contains("UL:FALSE"), "on() should reset underline: {out:?}");
    assert!(out.contains("ON:TRUE"), "isOn should be TRUE while active: {out:?}");
}
