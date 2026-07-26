use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

mod common;
use common::*;

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
fn native_io_readline_grows_buffer_and_reuses_freed_chunks() {
    // A line far longer than the 32-byte initial buffer forces many doublings.
    // Each grow copies the live bytes into the new buffer and returns the old one
    // to the arena free-list (plan-01 §8.3), so this exercises arena_free /
    // coalescing at runtime and guards the register-lifetime reload across the
    // allocator call in the grow path (a stale length would run the copy off the
    // new buffer and segfault).
    let project = temp_project(
        "native_io_readline_grow",
        r#"
IMPORT io

FUNC main AS Integer
  LET line AS String = io::readLine()
  io::print(toString(len(line)))
  io::print(line)
  RETURN 0
END FUNC
"#,
    );
    let executable = build_project(&project);
    let long: String = "AB".repeat(5000); // 10000 bytes, dozens of grows
    let mut input = long.clone().into_bytes();
    input.push(b'\n');
    let stdout = run_with_stdin(&executable, &input);
    assert_eq!(stdout, format!("10000\n{long}\n"));
}

#[test]
fn native_io_flush_reports_standard_stream_failures() {
    // io::flush() surfaces a stdout *write* failure — deterministically, on every
    // platform. Buffer real bytes, then flush into a read-only stdout: the drain's
    // write(1, …) fails with EBADF and flush traps (bug-04). This exercises the
    // real detection path (write, not fsync), so it behaves identically
    // everywhere instead of depending on what fd 1 happens to be.
    let flush_stdout = temp_project(
        "native_io_flush_stdout_failure",
        r#"
IMPORT io

FUNC main AS Integer
  io::setBuffered(TRUE)
  io::write("data")
  io::flush()
  RETURN 17
  TRAP(err)
    io::printError(toString(err.code))
    RETURN 0
  END TRAP
END FUNC
"#,
    );
    let (status, stdout, stderr) = run_with_readonly_stdout(&build_project(&flush_stdout), b"");
    assert_eq!(status, 0);
    assert_eq!(stdout, "");
    assert_eq!(stderr, "77020002\n");
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
        "Error: 1234\nbody failed\nCleanup failure: 7-703-0006\nResource close operation failed.\n"
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
    let transcript =
        run_pty_prompt_interaction_echo_off(&build_project(&project), "line> ", b"secret\n");
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
    let transcript = run_pty_prompt_interaction_echo_off(&build_project(&project), "ready> ", b"w");
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
    let transcript = run_pty_prompt_interaction_echo_off(&build_project(&project), "ready> ", b"A");
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
    // io::input(prompt) writes the prompt before reading; a stdout write failure
    // there must trap. A *non-empty* prompt drives the write path (bug-04:
    // input no longer fsyncs), so with a read-only stdout the prompt write fails
    // EBADF and traps deterministically everywhere. An empty prompt writes
    // nothing and correctly cannot fail.
    let project = temp_project(
        "native_io_input_prompt_failure",
        r#"
IMPORT io

FUNC main AS Integer
  io::input("prompt> ")
  RETURN 17
  TRAP(err)
    io::printError(toString(err.code))
    RETURN 0
  END TRAP
END FUNC
"#,
    );
    let (status, stdout, stderr) = run_with_readonly_stdout(&build_project(&project), b"Ada\n");
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

/// bug-51: output paths that issue a single `write()` treated a short *positive*
/// return as a complete write, silently dropping the tail while reporting success.
/// Under a `write()` capped to 4096 bytes, a 300000-byte write returns dozens of
/// short counts; the fixed advance-and-loop must transfer every byte. Covers the
/// default `io::write` stdout path (`lower_io_write_helper`) and the `fs::writeAll`
/// buffered large-chunk path (`emit_append_to_file_buffer`). Before the fix each
/// path wrote a single 4096-byte chunk, reported OK, and dropped the remaining
/// ~296 KB.
#[cfg(target_os = "macos")]
#[test]
fn native_io_short_write_returns_do_not_truncate_output() {
    const N: usize = 300000;

    // Default stdout path: a large io::write must survive short writes.
    let io_project = temp_project(
        "native_io_short_write_stdout",
        r#"
IMPORT io
IMPORT strings

FUNC main AS Integer
  io::write(strings::repeat("y", 300000))
  RETURN 0
END FUNC
"#,
    );
    let io_exe = build_project(&io_project);
    let io_interposer = build_short_write_interposer(&io_project);
    let io_envs = vec![
        ("DYLD_INSERT_LIBRARIES", io_interposer.display().to_string()),
        ("DYLD_FORCE_FLAT_NAMESPACE", "1".to_string()),
    ];
    let (io_status, io_stdout, io_stderr) = run_capture_with_env(&io_exe, &io_envs);
    assert_eq!(
        io_status, 0,
        "io::write should succeed under short writes: {io_stderr}"
    );
    assert_eq!(
        io_stdout.len(),
        N,
        "io::write dropped output on short writes: got {} of {N} bytes",
        io_stdout.len()
    );
    assert!(
        io_stdout.bytes().all(|b| b == b'y'),
        "io::write payload corrupted under short writes"
    );

    // fs::writeAll buffered large-chunk path: write to a regular file, read it
    // back, and confirm the whole payload landed.
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock before epoch")
        .as_nanos();
    let out_path = std::env::temp_dir().join(format!("bug51_fs_{nonce}.bin"));
    let fs_source = format!(
        r#"
IMPORT fs
IMPORT strings

FUNC main AS Integer
  RES f AS File = fs::open("{}", "write")
  fs::setBuffered(f, TRUE)
  fs::writeAll(f, strings::repeat("z", 300000))
  fs::close(f)
  RETURN 0
END FUNC
"#,
        out_path.display()
    );
    let fs_project = temp_project("native_io_short_write_fs", &fs_source);
    let fs_exe = build_project(&fs_project);
    let fs_interposer = build_short_write_interposer(&fs_project);
    let fs_envs = vec![
        ("DYLD_INSERT_LIBRARIES", fs_interposer.display().to_string()),
        ("DYLD_FORCE_FLAT_NAMESPACE", "1".to_string()),
    ];
    let (fs_status, _fs_stdout, fs_stderr) = run_capture_with_env(&fs_exe, &fs_envs);
    assert_eq!(
        fs_status, 0,
        "fs::writeAll should succeed under short writes: {fs_stderr}"
    );
    let written = fs::read(&out_path).expect("read fs output");
    let _ = fs::remove_file(&out_path);
    assert_eq!(
        written.len(),
        N,
        "fs::writeAll buffered dropped output on short writes: got {} of {N} bytes",
        written.len()
    );
    assert!(
        written.iter().all(|&b| b == b'z'),
        "fs::writeAll payload corrupted under short writes"
    );
}
