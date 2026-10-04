//! bug-808: `http::finish` raises only codes from its own Errors table.
//!
//! `http::PendingState.err` is a plain `Integer` field a program can write, and
//! `finish` used to re-raise whatever it held — `ErrBadPixelCount` (a `canvas`
//! code) or a number that is no error code at all — under "http stream transport
//! failed". `finish` now re-raises a captured code only when it is one a drive can
//! actually record (`tcp::read`/`tls::read`'s failures minus the `closed` EOF,
//! plus the deadline's `ErrTimeout` and the 64 MiB cap's `ErrOverflow`), and
//! raises `ErrInvalidArgument` for any other non-zero `state.err`.
//!
//! The peer is a `TcpListener` this test binds on an ephemeral loopback port and
//! never accepts from: the kernel completes the handshake from the backlog, so
//! `http::startRead`'s connect and request write succeed, and the program then
//! overwrites `s.state.err` before any read. No network and no Python needed.

#[path = "../common/mod.rs"]
mod common;

use std::net::TcpListener;
use std::path::PathBuf;
use std::time::Duration;

const SOURCE: &str = r#"IMPORT net
IMPORT http
IMPORT io
IMPORT os
IMPORT strings

FUNC finishWith(port AS String, code AS Integer) AS String
  RES s AS http::Stream STATE http::PendingState = http::startRead(net::toUrl("http://127.0.0.1:" & port & "/"))
  s.state.err = code
  LET r AS http::Response = http::finish(s) TRAP(e)
    RETURN toString(e.code) & " " & e.message
  END TRAP
  RETURN "status " & toString(r.status)
END FUNC

FUNC main() AS Nothing
  LET port AS String = os::getEnvOr("PORT", "0")
  LET codes AS List OF String = strings::split(os::getEnvOr("CODES", ""), ",")
  FOR EACH text IN codes
    io::print(text & " -> " & finishWith(port, toInt(text)))
  NEXT
END FUNC
"#;

const ERR_INVALID_ARGUMENT: i64 = 77050002;

/// Every code `__http_pump`/`__http_waitReadable` can store in `state.err`:
/// `tcp::read` ∪ `tls::read` declared errors minus `ErrConnectionClosed`, plus
/// `ErrTimeout` and `ErrOverflow`.
const RECORDABLE: &[(&str, i64)] = &[
    ("ErrInvalidArgument", 77050002),
    ("ErrNetworkFailed", 77070003),
    ("ErrOutOfMemory", 77010001),
    ("ErrOverflow", 77050010),
    ("ErrResourceClosed", 77030004),
    ("ErrResourceMoved", 77030009),
    ("ErrTimeout", 77050008),
    ("ErrTlsFailed", 77070008),
];

struct Project(PathBuf);

impl Drop for Project {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Run the program once over `codes` and return its `code -> outcome` lines.
fn run(codes: &[i64]) -> Vec<String> {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback listener");
    let port = listener.local_addr().expect("local addr").port();
    let project = Project(common::temp_project("http_finish_state_err", SOURCE));
    let binary = common::build_project(&project.0);
    let list = codes
        .iter()
        .map(|code| code.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let mut command = std::process::Command::new(&binary);
    command.env("PORT", port.to_string()).env("CODES", list);
    let output = command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn program");
    let output = wait_bounded(output, Duration::from_secs(60));
    drop(listener);
    assert!(
        output.status.success(),
        "program {}:\nstdout:\n{}\nstderr:\n{}",
        common::exit_description(&output.status),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8(output.stdout)
        .expect("utf8 stdout")
        .lines()
        .map(str::to_string)
        .collect()
}

fn wait_bounded(mut child: std::process::Child, timeout: Duration) -> std::process::Output {
    let start = std::time::Instant::now();
    loop {
        if child.try_wait().expect("try_wait").is_some() {
            return child.wait_with_output().expect("collect output");
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            panic!("program still running after {timeout:?}");
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn a_recordable_code_is_re_raised_unchanged() {
    let codes: Vec<i64> = RECORDABLE.iter().map(|(_, code)| *code).collect();
    let lines = run(&codes);
    assert_eq!(lines.len(), codes.len(), "one line per code: {lines:?}");
    for ((name, code), line) in RECORDABLE.iter().zip(&lines) {
        assert_eq!(
            line,
            &format!("{code} -> {code} http stream transport failed"),
            "{name} must be re-raised as itself",
        );
    }
}

#[test]
fn any_other_code_raises_err_invalid_argument() {
    // `ErrBadPixelCount` (a `canvas` code), `ErrConnectionClosed` (diverted to
    // `closed`, never stored), a number that is no error code, and a negative one.
    let codes = [77050021, 77070004, 123456789, -1];
    let lines = run(&codes);
    assert_eq!(lines.len(), codes.len(), "one line per code: {lines:?}");
    for (code, line) in codes.iter().zip(&lines) {
        let prefix = format!("{code} -> {ERR_INVALID_ARGUMENT} ");
        assert!(
            line.starts_with(&prefix),
            "state.err = {code} is no code a drive records; expected ErrInvalidArgument, got: {line}",
        );
    }
}
