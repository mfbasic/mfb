//! Regression test for bug-902: a program started with fd 0, 1 or 2 closed handed
//! those numbers to its own `fs::open`, because the kernel gives the lowest free
//! number to the next open. `io::readLine` then read the program's file as "stdin",
//! and `io::print`/`io::printError` wrote console text into a file it had opened.
//!
//! The program entry now reserves every closed standard descriptor with `/dev/null`
//! before anything can open one (`O_RDONLY` for 0, `O_WRONLY` for 1 and 2), as Rust
//! std and Go do. A closed stdin reads as end-of-input, a closed stdout or stderr
//! swallows writes, and no file the program opens can be numbered 0, 1 or 2.
//!
//! Each case closes the descriptor in the forked child right before `exec`, which is
//! exactly the state `prog <&-` / `prog >&-` / `prog 2>&-` leaves a process in.

#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Output, Stdio};

/// Run `executable` with each descriptor in `closed` closed at exec.
fn run_with_closed(executable: &Path, closed: &'static [i32]) -> Output {
    let mut command = Command::new(executable);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // SAFETY: the closure calls only `close`, which is async-signal-safe, and
    // allocates nothing.
    unsafe {
        command.pre_exec(move || {
            for &fd in closed {
                libc::close(fd);
            }
            Ok(())
        });
    }
    command.output().expect("run the program")
}

#[test]
fn a_closed_stdin_is_never_the_programs_own_file() {
    // A scratch directory for the file the program opens.
    let dir = common::temp_project("bug902_stdin_secret", "FUNC main() AS Nothing\nEND FUNC\n");
    let secret = dir.join("secret.txt");
    std::fs::write(&secret, "SECRET-LINE\n").expect("write the secret file");
    let source = format!(
        r#"IMPORT io
IMPORT fs

FUNC main() AS Integer
  RES f AS fs::File = fs::open("{}", "r")
  LET l AS String = io::readLine()
  io::print("stdin gave [" & l & "]")
  RETURN 0
END FUNC
"#,
        secret.display()
    );
    let project = common::temp_project("bug902_stdin", &source);
    let executable = common::build_project(&project);

    let output = run_with_closed(&executable, &[0]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stdout.contains("SECRET") && !stderr.contains("SECRET"),
        "with stdin closed, io::readLine read the file the program opened: \
         status {:?}\nstdout: {stdout}\nstderr: {stderr}",
        output.status
    );
}

#[test]
fn a_closed_stdout_or_stderr_never_writes_into_the_programs_file() {
    // A scratch directory for the file the program opens.
    let dir = common::temp_project("bug902_out_dir", "FUNC main() AS Nothing\nEND FUNC\n");
    let data = dir.join("data.txt");
    let source = format!(
        r#"IMPORT io
IMPORT fs

FUNC main() AS Integer
  RES g AS fs::File = fs::open("{}", "w")
  fs::writeAll(g, "file-data;")
  io::print("this went to stdout")
  io::printError("this went to stderr")
  RETURN 0
END FUNC
"#,
        data.display()
    );
    let project = common::temp_project("bug902_out", &source);
    let executable = common::build_project(&project);

    let cases: [&'static [i32]; 3] = [&[1], &[2], &[1, 2]];
    for closed in cases {
        let _ = std::fs::remove_file(&data);
        let output = run_with_closed(&executable, closed);
        let written = std::fs::read_to_string(&data).unwrap_or_default();
        assert_eq!(
            written,
            "file-data;",
            "with fd {closed:?} closed, console output landed in the file the program \
             opened: status {:?}, stdout {:?}, stderr {:?}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            output.status.success(),
            "with fd {closed:?} closed the program must still run to its own exit: {:?}",
            output.status
        );
    }
}
