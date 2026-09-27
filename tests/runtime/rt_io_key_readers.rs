//! bug-699 Phase 4: the `io` keystroke readers (`readRaw`, `readScalar`,
//! `readGrapheme`, `readKey`) and the line readers' timeouts, run as real
//! programs on a pipe and on a terminal.
//!
//! The pipe cases pin the decoding on data (no timer: a sequence split across
//! writes waits for its rest); the terminal cases, driven through a real pty,
//! pin the keyboard behaviour — one key per read with no Return and no echo, an
//! escape sequence read as one key, a lone Escape decided after the delay, and
//! a half-typed line surviving a timed-out `readLine`.

#[path = "../common/mod.rs"]
mod common;
use common::*;
use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Reads keys until end of input or a timeout, one line per key:
/// `name grapheme-hex event s c a m`.
const KEY_DUMP: &str = r#"
IMPORT encoding
IMPORT errorCode
IMPORT io
IMPORT strings

FUNC hexOf(s AS String) AS String
  RETURN encoding::hexEncode(strings::toBytes(s))
END FUNC

FUNC flag(b AS Boolean) AS String
  IF b THEN
    RETURN "1"
  END IF
  RETURN "0"
END FUNC

FUNC main AS Integer
  WHILE TRUE
    LET k AS io::Key = io::readKey(TIMEOUT) TRAP(e)
      IF e.code = errorCode::ErrEndOfFile THEN
        io::print("eof")
        RETURN 0
      END IF
      IF e.code = errorCode::ErrTimeout THEN
        io::print("timeout")
        RETURN 0
      END IF
      FAIL e
    END TRAP
    io::print(toString(k.key) & " " & hexOf(k.grapheme) & " " & toString(k.event) & " " & flag(k.shift) & flag(k.ctrl) & flag(k.alt) & flag(k.meta))
  END WHILE
  RETURN 0
END FUNC
"#;

fn key_dump(name: &str, timeout: &str) -> std::path::PathBuf {
    build_project(&temp_project(name, &KEY_DUMP.replace("TIMEOUT", timeout)))
}

#[test]
fn read_key_decodes_every_encoding_on_a_pipe() {
    let exe = key_dump("rt_io_read_key_pipe", "5000");
    let stdout = run_with_stdin(
        &exe,
        b"a\x1b[D\x1bOA\x1b[1;2C\x1b[3~\x1b[15~\x1b[97;5u\x1b[1;1:3D\x1b[97;1:2;97u\
          \x01\x7f\t\r\n\xc3\xa9\x1bx\xff\x1b[200~\x1b",
    );
    assert_eq!(
        stdout,
        "Unicode 61 NotSupported 0000\n\
         Left  NotSupported 0000\n\
         Up  NotSupported 0000\n\
         Right  NotSupported 1000\n\
         Delete  NotSupported 0000\n\
         F5  NotSupported 0000\n\
         Unicode 61 NotSupported 0100\n\
         Left  Release 0000\n\
         Unicode 61 Repeat 0000\n\
         Unicode 61 NotSupported 0100\n\
         Backspace 7f NotSupported 0000\n\
         Tab 09 NotSupported 0000\n\
         Enter 0a NotSupported 0000\n\
         Unicode c3a9 NotSupported 0000\n\
         Unicode 78 NotSupported 0010\n\
         Escape 1b NotSupported 0000\n\
         eof\n",
        "one key per encoding; a kitty sequence with no event field reports \
         NotSupported on a pipe (no source of events), the invalid byte and the \
         paste marker are dropped, and the lone ESC at the end of the input is the \
         Escape key"
    );
}

#[test]
fn the_text_readers_return_text_only() {
    let exe = build_project(&temp_project(
        "rt_io_text_readers",
        r#"
IMPORT io

FUNC main AS Integer
  LET a AS Scalar = io::readScalar()
  LET b AS Scalar = io::readScalar()
  LET g AS String = io::readGrapheme()
  LET h AS String = io::readGrapheme()
  io::print(toString(toInt(a)) & " " & toString(toInt(b)) & " [" & g & "] [" & h & "]")
  RETURN 0
END FUNC
"#,
    ));
    // Up and a release are skipped; Ctrl+C is U+0003; `e` + U+0301 is one
    // grapheme; the next is `z`.
    let stdout = run_with_stdin(&exe, b"\x1b[A\x1b[99;1:3u\x03qe\xcc\x81z");
    assert_eq!(stdout, "3 113 [e\u{301}] [z]\n");
}

#[test]
fn read_raw_is_byte_exact_on_a_pipe() {
    let exe = build_project(&temp_project(
        "rt_io_read_raw",
        r#"
IMPORT errorCode
IMPORT io

FUNC main AS Integer
  MUT out AS String = ""
  WHILE TRUE
    LET b AS Byte = io::readRaw() TRAP(e)
      io::print(out)
      RETURN 0
    END TRAP
    out = out & toString(b) & " "
  END WHILE
  RETURN 0
END FUNC
"#,
    ));
    let stdout = run_with_stdin(&exe, b"\x1b[D\xff\r\n\x00");
    assert_eq!(stdout, "27 91 68 255 13 10 0 \n");
}

/// Spawn `exe` with a stdin the test writes to in timed pieces.
fn run_with_timed_stdin(exe: &std::path::Path, pieces: &[(u64, &[u8])]) -> String {
    let mut child = Command::new(exe)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    let mut stdin = child.stdin.take().expect("stdin");
    for (delay_ms, bytes) in pieces {
        std::thread::sleep(Duration::from_millis(*delay_ms));
        stdin.write_all(bytes).expect("write stdin");
        stdin.flush().expect("flush stdin");
    }
    drop(stdin);
    let output = child.wait_with_output().expect("wait");
    assert!(
        output.status.success(),
        "exit {:?}; stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("utf8 stdout")
}

#[test]
fn a_timed_read_line_keeps_the_part_of_the_line_it_had_read() {
    let exe = build_project(&temp_project(
        "rt_io_read_line_timeout",
        r#"
IMPORT errorCode
IMPORT io

FUNC main AS Integer
  ' Bounded by timeouts only: how many there are depends on how soon the
  ' program starts, and the lines and the end must all be read.
  MUT timeouts AS Integer = 0
  WHILE timeouts < 10
    LET line AS String = io::readLine(300) TRAP(e)
      IF e.code = errorCode::ErrTimeout THEN
        io::print("timeout")
        timeouts = timeouts + 1
        CONTINUE WHILE
      END IF
      io::print("end")
      RETURN 0
    END TRAP
    io::print("[" & line & "]")
  END WHILE
  RETURN 0
END FUNC
"#,
    ));
    // `ab` arrives, then nothing for longer than the timeout, then `cd\n`: a read
    // times out (how many depends on how soon the program starts), and the next
    // returns the WHOLE line — the `ab` it had read was kept.
    let stdout = run_with_timed_stdin(&exe, &[(0, b"ab"), (900, b"cd\nx\n")]);
    let lines: Vec<&str> = stdout.lines().collect();
    let first_line = lines
        .iter()
        .position(|line| *line != "timeout")
        .expect("a line after the timeouts");
    assert!(first_line >= 1, "the idle pipe timed out first: {stdout:?}");
    assert_eq!(&lines[first_line..], ["[abcd]", "[x]", "end"], "{stdout:?}");
}

#[test]
fn a_timed_read_waits_its_timeout_and_no_longer() {
    let exe = key_dump("rt_io_read_key_timeout", "400");
    let mut child = Command::new(&exe)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn");
    // Hold stdin open (an idle pipe, not end of input) until the child is done.
    let stdin = child.stdin.take().expect("stdin");
    let start = Instant::now();
    let output = child.wait_with_output().expect("wait");
    let elapsed = start.elapsed();
    drop(stdin);
    assert_eq!(String::from_utf8_lossy(&output.stdout), "timeout\n");
    assert!(
        elapsed >= Duration::from_millis(350) && elapsed < Duration::from_millis(5000),
        "readKey(400) on an idle pipe returned after {elapsed:?}"
    );
}

#[test]
fn a_timeout_of_zero_is_one_immediate_attempt() {
    let exe = build_project(&temp_project(
        "rt_io_read_key_zero",
        r#"
IMPORT errorCode
IMPORT io
IMPORT os

FUNC main AS Integer
  ' Let the input arrive first: `0` reads what is already waiting ...
  os::sleep(500)
  LET b AS Byte = io::readRaw(0)
  io::print(toString(b))
  ' ... and, on an idle pipe, times out at once. (On a pipe a grapheme is
  ' complete only once the next character, or the end, has arrived — there is
  ' no timer — so `readKey(0)` would time out on the `q` alone too.)
  LET again AS io::Key = io::readKey(0) TRAP(e)
    IF e.code = errorCode::ErrTimeout THEN
      io::print("timeout")
      RETURN 0
    END IF
    FAIL e
  END TRAP
  io::print("unexpected " & again.grapheme)
  RETURN 0
END FUNC
"#,
    ));
    let mut child = Command::new(&exe)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn");
    let mut stdin = child.stdin.take().expect("stdin");
    stdin.write_all(b"q").expect("write");
    stdin.flush().expect("flush");
    let start = Instant::now();
    let output = child.wait_with_output().expect("wait");
    drop(stdin);
    assert_eq!(String::from_utf8_lossy(&output.stdout), "113\ntimeout\n");
    assert!(start.elapsed() < Duration::from_millis(3000));
}

/// Drive `exe` on a pty: wait for `ready` and for echo to be off, then write each
/// `(pause_ms, bytes)` piece in turn; return everything the child printed.
#[cfg(unix)]
fn run_on_terminal(exe: &std::path::Path, ready: &str, pieces: &[(u64, &[u8])]) -> String {
    let script = r#"import os, pty, select, subprocess, sys, termios, time
ready = sys.argv[2].encode()
pieces = [(int(p.split(":")[0]), bytes.fromhex(p.split(":")[1])) for p in sys.argv[3:]]
master, slave = pty.openpty()
probe = os.dup(slave)
proc = subprocess.Popen([sys.argv[1]], stdin=slave, stdout=slave, stderr=slave, close_fds=True)
os.close(slave)
out = b""
def drain(t):
    global out
    end = time.time() + t
    while time.time() < end:
        r, _, _ = select.select([master], [], [], 0.01)
        if r:
            try:
                out += os.read(master, 4096)
            except OSError:
                return
deadline = time.time() + 10
while ready not in out and time.time() < deadline:
    drain(0.02)
while termios.tcgetattr(probe)[3] & termios.ECHO and time.time() < deadline:
    time.sleep(0.001)
for pause, data in pieces:
    drain(pause / 1000.0)
    os.write(master, data)
deadline = time.time() + 10
while proc.poll() is None and time.time() < deadline:
    drain(0.05)
drain(0.1)
if proc.poll() is None:
    proc.kill()
    sys.exit(124)
os.close(probe)
sys.stdout.buffer.write(out)
sys.exit(proc.wait())"#;
    let mut command = Command::new("python3");
    command.arg("-c").arg(script).arg(exe).arg(ready);
    for (pause, bytes) in pieces {
        let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        command.arg(format!("{pause}:{hex}"));
    }
    let output = command.output().expect("run pty driver");
    assert!(
        output.status.success(),
        "pty run failed ({:?}): {}\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n")
}

#[cfg(unix)]
#[test]
fn a_terminal_reads_one_key_at_a_time_without_echo() {
    let exe = build_project(&temp_project(
        "rt_io_terminal_keys",
        r#"
IMPORT io

FUNC main AS Integer
  io::write("ready> ")
  MUT n AS Integer = 0
  WHILE n < 4
    LET k AS io::Key = io::readKey()
    io::print("<" & toString(k.key) & ">")
    n = n + 1
  END WHILE
  RETURN 0
END FUNC
"#,
    ));
    // `w` with no Return; Left as one key; a lone Esc decided after the delay
    // (the next key comes 300 ms later); Alt+x as Esc then `x` in one burst.
    let out = run_on_terminal(
        &exe,
        "ready> ",
        &[(0, b"w"), (100, b"\x1b[D"), (100, b"\x1b"), (300, b"\x1bx")],
    );
    assert!(
        out.contains("ready> <Unicode>\n<Left>\n<Escape>\n<Unicode>\n"),
        "{out:?}"
    );
    assert!(!out.contains('w'), "the key was echoed: {out:?}");
}

#[cfg(unix)]
#[test]
fn a_timed_read_line_on_a_terminal_leaves_the_half_typed_line() {
    let exe = build_project(&temp_project(
        "rt_io_terminal_read_line_timeout",
        r#"
IMPORT errorCode
IMPORT io

FUNC main AS Integer
  io::write("ready> ")
  MUT n AS Integer = 0
  WHILE n < 3
    LET line AS String = io::readLine(300) TRAP(e)
      IF e.code = errorCode::ErrTimeout THEN
        io::print("timeout")
        n = n + 1
        CONTINUE WHILE
      END IF
      FAIL e
    END TRAP
    io::print("[" & line & "]")
    RETURN 0
  END WHILE
  RETURN 0
END FUNC
"#,
    ));
    // Half a line, a pause past the timeout, then the rest and Return.
    let out = run_on_terminal(&exe, "ready> ", &[(0, b"ha"), (500, b"lf\r")]);
    assert!(out.contains("timeout\n[half]\n"), "{out:?}");
}

/// Drive `exe` on a pty that answers `term::on`'s keyboard-protocol query: wait
/// for the query (`ESC [ ? u`), write `reply` (what the terminal answers, plus
/// any keys "typed" before the answer), then write each piece 100 ms apart;
/// return everything the child printed, escapes included.
#[cfg(unix)]
fn run_on_replying_terminal(exe: &std::path::Path, reply: &[u8], pieces: &[&[u8]]) -> String {
    let (status, out) = run_on_replying_terminal_status(exe, reply, pieces);
    assert_eq!(status, 0, "the program failed: {out:?}");
    out
}

/// [`run_on_replying_terminal`], returning the child's exit status (128 + the
/// signal for a signal death, as a shell reports it) instead of requiring 0.
/// The child runs on the pty as its controlling terminal (`pty.fork`), so the
/// terminal's own signals reach it as they would from a keyboard.
#[cfg(unix)]
fn run_on_replying_terminal_status(
    exe: &std::path::Path,
    reply: &[u8],
    pieces: &[&[u8]],
) -> (i32, String) {
    let script = r#"import os, pty, select, sys, time
reply = bytes.fromhex(sys.argv[2])
pieces = [bytes.fromhex(p) for p in sys.argv[3:]]
pid, master = pty.fork()
if pid == 0:
    os.execv(sys.argv[1], [sys.argv[1]])
out = b""
def drain(t):
    global out
    end = time.time() + t
    while time.time() < end:
        r, _, _ = select.select([master], [], [], 0.01)
        if r:
            try:
                out += os.read(master, 4096)
            except OSError:
                return
deadline = time.time() + 10
while b"\x1b[?u" not in out and time.time() < deadline:
    drain(0.02)
os.write(master, reply)
drain(0.1)
for data in pieces:
    os.write(master, data)
    drain(0.1)
deadline = time.time() + 10
status = None
while status is None and time.time() < deadline:
    drain(0.05)
    done, raw = os.waitpid(pid, os.WNOHANG)
    if done:
        status = raw
drain(0.1)
if status is None:
    os.kill(pid, 9)
    sys.exit(124)
sys.stdout.buffer.write(out)
code = 128 + os.WTERMSIG(status) if os.WIFSIGNALED(status) else os.WEXITSTATUS(status)
sys.stderr.write("status %d" % code)
sys.exit(0)"#;
    let hex = |bytes: &[u8]| -> String { bytes.iter().map(|b| format!("{b:02x}")).collect() };
    let mut command = Command::new("python3");
    command.arg("-c").arg(script).arg(exe).arg(hex(reply));
    for piece in pieces {
        command.arg(hex(piece));
    }
    let output = command.output().expect("run pty driver");
    let out = String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n");
    let err = String::from_utf8_lossy(&output.stderr);
    let status = err
        .strip_prefix("status ")
        .and_then(|code| code.trim().parse::<i32>().ok())
        .unwrap_or_else(|| panic!("pty run failed ({:?}): {out:?}\n{err}", output.status));
    (status, out)
}

/// `term::on`, four keys, `term::off`, then the keys printed (after `term::off`,
/// so the alternate screen does not swallow them).
#[cfg(unix)]
const TERM_KEYS: &str = r#"
IMPORT io
IMPORT term

FUNC main AS Integer
  term::on()
  MUT out AS String = ""
  MUT n AS Integer = 0
  WHILE n < 4
    LET k AS io::Key = io::readKey()
    out = out & "<" & toString(k.key) & " " & k.grapheme & " " & toString(k.event) & " " & toString(k.shift) & ">"
    n = n + 1
  END WHILE
  term::off()
  io::print(out)
  RETURN 0
END FUNC
"#;

#[cfg(unix)]
#[test]
fn term_on_switches_a_kitty_terminal_to_key_events_and_off_restores_it() {
    let exe = build_project(&temp_project("rt_io_term_kitty", TERM_KEYS));
    // The terminal reports its flags, then its device attributes; then `a` is
    // pressed and released, Shift+A pressed, Left pressed — all in the kitty form.
    let out = run_on_replying_terminal(
        &exe,
        b"\x1b[?0u\x1b[?62;c",
        &[b"\x1b[97;1:1u", b"\x1b[97;1:3u", b"\x1b[65;2:1u", b"\x1b[1;1:1D"],
    );
    let push = out.find("\x1b[>31u").unwrap_or_else(|| panic!("no push: {out:?}"));
    let pop = out.find("\x1b[<u").unwrap_or_else(|| panic!("no pop: {out:?}"));
    assert!(push < pop, "{out:?}");
    assert!(
        out.contains("<Unicode a Press FALSE><Unicode a Release FALSE><Unicode A Press TRUE><Left  Press FALSE>"),
        "{out:?}"
    );
}

#[cfg(unix)]
#[test]
fn term_on_leaves_a_legacy_terminal_alone_and_keeps_keys_typed_meanwhile() {
    let exe = build_project(&temp_project("rt_io_term_legacy", TERM_KEYS));
    // No flags reply — only device attributes, with `z` and Left typed before
    // it arrived. Neither key is lost, and nothing is pushed or popped.
    let out = run_on_replying_terminal(&exe, b"z\x1b[D\x1b[?62;c", &[b"q", b"\x1bx"]);
    assert!(!out.contains("\x1b[>31u"), "{out:?}");
    assert!(!out.contains("\x1b[<u"), "{out:?}");
    assert!(
        out.contains("<Unicode z NotSupported FALSE><Left  NotSupported FALSE><Unicode q NotSupported FALSE><Unicode x NotSupported FALSE>"),
        "{out:?}"
    );
}

#[cfg(unix)]
#[test]
fn read_line_under_term_on_reads_the_line_in_the_terminals_ordinary_encoding() {
    let exe = build_project(&temp_project(
        "rt_io_term_kitty_read_line",
        r#"
IMPORT io
IMPORT term

FUNC main AS Integer
  term::on()
  LET line AS String = io::readLine()
  LET k AS io::Key = io::readKey()
  term::off()
  io::print("[" & line & "] <" & toString(k.key) & " " & toString(k.event) & ">")
  RETURN 0
END FUNC
"#,
    ));
    // A kitty terminal: the line is typed after the pop (ordinary bytes), the key
    // after the push again (kitty form, a release).
    let out = run_on_replying_terminal(
        &exe,
        b"\x1b[?0u\x1b[?62;c",
        &[b"hi\r", b"\x1b[97;1:3u"],
    );
    let first_pop = out.find("\x1b[<u").unwrap_or_else(|| panic!("no pop: {out:?}"));
    assert!(
        out[first_pop..].contains("\x1b[>31u"),
        "the flags were not pushed again after the line: {out:?}"
    );
    assert!(out.contains("[hi] <Unicode Release>"), "{out:?}");
}

#[cfg(unix)]
#[test]
fn ctrl_c_under_the_kitty_protocol_still_interrupts_the_program() {
    // A kitty terminal sends Ctrl+C as a key, not the byte the line discipline
    // turns into SIGINT; the reader raises it instead. The teardown still runs:
    // the flags are popped and the exit status is the signal's.
    let exe = build_project(&temp_project("rt_io_term_kitty_ctrl_c", TERM_KEYS));
    let (status, out) =
        run_on_replying_terminal_status(&exe, b"\x1b[?0u\x1b[?62;c", &[b"\x1b[99;5u"]);
    assert_eq!(status, 130, "{out:?}");
    let push = out.find("\x1b[>31u").unwrap_or_else(|| panic!("no push: {out:?}"));
    assert!(out[push..].contains("\x1b[<u"), "not popped: {out:?}");
    assert!(!out.contains("<Unicode"), "Ctrl+C was returned as a key: {out:?}");
}

#[test]
fn every_reader_builds_and_reads_in_a_mouse_program() {
    // A program that uses the mouse wraps each reader's source read in the mouse
    // decoder, whose escape wait asks the stdin log for readiness — as the
    // reader's own timed wait does. The two checks once shared their labels, so
    // any timed reader in a mouse program failed to assemble ("duplicate label
    // `…_stdin_poll_ready_unlock`"). `term::on` carries the same pair (its
    // keyboard-protocol probe). Build every reader, and `term::on`, into one
    // mouse program and read a key, a line and a byte through it.
    let exe = build_project(&temp_project(
        "rt_io_mouse_readers",
        r#"
IMPORT errorCode
IMPORT io
IMPORT strings
IMPORT term

FUNC main AS Integer
  term::enableMouse(TRUE)
  LET k AS io::Key = io::readKey(1000)
  LET s AS Scalar = io::readScalar(1000)
  LET g AS String = io::readGrapheme(1000)
  LET line AS String = io::readLine(1000)
  LET b AS Byte = io::readRaw(1000)
  io::print(k.grapheme & "|" & strings::fromScalars([s]) & "|" & g & "|" & line & "|" & toString(b))
  LET more AS io::Key = io::readKey(0) TRAP(e)
    IF e.code = errorCode::ErrEndOfFile THEN
      io::print("eof")
      RETURN 0
    END IF
    FAIL e
  END TRAP
  IF FALSE THEN
    term::on()
  END IF
  RETURN 0
END FUNC
"#,
    ));
    // (The mouse-mode escapes around it are `enableMouse`'s own output.) The
    // report between `b` and `c` is swallowed by the decoder, not read as keys.
    let out = run_with_timed_stdin(&exe, &[(0, b"ab\x1b[<0;4;2Mc line\nZ")]);
    assert!(out.contains("a|b|c| line|90\neof\n"), "{out:?}");
}
