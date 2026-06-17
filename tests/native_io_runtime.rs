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
  TRAP err
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
  TRAP err
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
fn native_io_input_reports_prompt_flush_failure() {
    let project = temp_project(
        "native_io_input_prompt_failure",
        r#"
IMPORT io

FUNC main AS Integer
  io::input()
  RETURN 17
  TRAP err
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
  TRAP err
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
  TRAP err
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
  TRAP err
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

FUNC printTerminalSize AS Nothing
  LET size AS TerminalSize = io::terminalSize()
  io::print(toString(size.columns))
  io::print(toString(size.rows))
  RETURN NOTHING
  TRAP err
    io::print("ERR:" & toString(err.code))
    RETURN NOTHING
  END TRAP
END FUNC

FUNC main AS Integer
  io::print(toString(io::isInputTerminal()))
  io::print(toString(io::isOutputTerminal()))
  io::print(toString(io::isErrorTerminal()))
  printTerminalSize()
  RETURN 0
END FUNC
"#,
    );
    let executable = build_project(&project);

    let direct = run_with_stdin(&executable, b"");
    assert_eq!(direct, "FALSE\nFALSE\nFALSE\nERR:77050007\n");

    let pty = run_under_pty(&executable);
    let lines = pty
        .replace("\r\n", "\n")
        .lines()
        .map(str::to_string)
        .collect::<Vec<_>>();
    assert!(lines.len() >= 5, "expected tty output, got {lines:?}");
    assert_eq!(lines[0], "TRUE");
    assert_eq!(lines[1], "TRUE");
    assert_eq!(lines[2], "TRUE");
    assert!(lines[3].parse::<i64>().expect("columns") > 0);
    assert!(lines[4].parse::<i64>().expect("rows") > 0);
}
