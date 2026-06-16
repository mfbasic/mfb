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

    assert_eq!(run_with_stdin(&build_project(&read_line), b""), "77020003\n");
    assert_eq!(run_with_stdin(&build_project(&read_char), b"\x80"), "77020004\n");
    assert_eq!(run_with_stdin(&build_project(&input), b"\xff\n"), "77020004\n");
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
