//! plan-76-D: the non-blocking http client drives a real HTTP exchange end to end
//! over the `Stream` resource union carrying plan-74 `PendingState`, and the
//! rewritten blocking `http::read` produces the same `Response` over the same core.
//!
//! - Async (Phase 2/3): `RES s AS http::Stream STATE PendingState =
//!   http::startRead(url)`, then `IF http::ready(s) THEN http::pump(s)` until
//!   `http::done(s)`, then `http::finish(s)`. Exercises all five entry points + the
//!   union STATE mutation (`state.raw` accumulated across MULTIPLE pumps — only
//!   expressible because plan-80 relocated the STATE slot, the D4 fix).
//! - Blocking parity (Phase 4): `http::read(url)` — now a thin wrapper over the
//!   same core — yields the identical status + body.
//!
//! The peer is a one-shot Python raw-socket HTTP/1.1 server (Connection: close, so
//! the terminator is peer EOF, matching what `__http_buildRequest` forces), which
//! splits the body across two writes so the async client must pump more than once.
//! Gated only on `python3`. Not macOS-specific — the plaintext client uses the
//! `net` Socket variant, supported on every backend.

mod common;

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Child, Command, Output, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const PORT_ASYNC: u16 = 18473;
const PORT_BLOCKING: u16 = 18474;
const EXPECT_BODY: &str = "hello-async-stream-BODY";
const PROCESS_TIMEOUT: Duration = Duration::from_secs(30);
static DRIVE_LOCK: Mutex<()> = Mutex::new(());

fn nonce() -> String {
    common::unique_nonce()
}

fn have_python() -> bool {
    Command::new("python3")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// One-shot HTTP/1.1 server: accept one connection, drain the request headers,
/// reply with a fixed body sent in TWO writes (a small flush, a brief pause, then
/// the rest) so an async client accumulates `state.raw` across pumps. Prints
/// `READY` once listening.
const SERVER_PY: &str = r#"
import socket, sys, time
port = int(sys.argv[1])
srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
srv.bind(("127.0.0.1", port))
srv.listen(1)
print("READY", flush=True)
conn, _ = srv.accept()
data = b""
while b"\r\n\r\n" not in data:
    chunk = conn.recv(4096)
    if not chunk:
        break
    data += chunk
body = b"hello-async-stream-BODY"
head = b"HTTP/1.1 200 OK\r\nContent-Length: %d\r\nConnection: close\r\n\r\n" % len(body)
conn.sendall(head + body[:5])
time.sleep(0.2)
conn.sendall(body[5:])
conn.close()
srv.close()
"#;

fn output_with_timeout(command: &mut Command, phase: &str) -> Output {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .unwrap_or_else(|err| panic!("{phase}: {err}"));
    let deadline = Instant::now() + PROCESS_TIMEOUT;
    loop {
        match child.try_wait().expect("poll child") {
            Some(_) => return child.wait_with_output().expect("collect child output"),
            None if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            None => {
                let _ = child.kill();
                let output = child
                    .wait_with_output()
                    .expect("collect timed-out child output");
                panic!(
                    "plan-76-D: {phase} did not finish within 30s\nstdout:\n{}\nstderr:\n{}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                );
            }
        }
    }
}

fn stop_child(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn build_client(root: &Path, source: &str) -> std::path::PathBuf {
    fs::create_dir_all(root.join("src")).expect("create src dir");
    fs::write(
        root.join("project.json"),
        "{\"name\":\"httpc\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"executable\",\"sources\":[{\"root\":\"src\",\"role\":\"main\",\"include\":[\"**/*.mfb\"]}],\"entry\":\"main\",\"targets\":[\"native\"]}\n",
    )
    .expect("write project.json");
    fs::write(root.join("src/main.mfb"), source).expect("write source");
    let output = output_with_timeout(
        Command::new(common::mfb_exe()).arg("build").arg(root),
        "mfb build",
    );
    assert!(
        output.status.success(),
        "client build failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("utf8 build output")
        .lines()
        .find_map(|line| line.strip_prefix("Wrote executable to "))
        .map(std::path::PathBuf::from)
        .expect("build output executable path")
}

/// Build `source`, start the one-shot server on `port`, run the client with a hard
/// deadline, and return its process output. Panics (not hangs) on timeout.
fn drive(port: u16, source: &str) -> Output {
    let _guard = DRIVE_LOCK.lock().unwrap_or_else(|err| err.into_inner());
    let root = std::env::temp_dir().join(format!("mfb_p76d_{}_{}", port, nonce()));
    fs::create_dir_all(&root).expect("create temp root");
    let py = root.join("server.py");
    fs::write(&py, SERVER_PY).expect("write server.py");
    let exe = build_client(&root, source);

    let mut server = Command::new("python3")
        .arg(&py)
        .arg(port.to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn python http server");
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    {
        let out = server.stdout.take().expect("server stdout");
        std::thread::spawn(move || {
            let mut rdr = BufReader::new(out);
            let mut line = String::new();
            let result = rdr.read_line(&mut line).map(|_| line);
            let _ = ready_tx.send(result);
        });
    }
    let ready = match ready_rx.recv_timeout(PROCESS_TIMEOUT) {
        Ok(Ok(line)) => line,
        Ok(Err(err)) => {
            stop_child(&mut server);
            panic!("failed to read server READY: {err}");
        }
        Err(_) => {
            stop_child(&mut server);
            panic!("plan-76-D: server did not become ready within 30s");
        }
    };
    assert!(
        ready.starts_with("READY"),
        "server did not report READY: {ready:?}"
    );

    let out = output_with_timeout(&mut Command::new(&exe), "http client");
    stop_child(&mut server);
    let _ = fs::remove_dir_all(&root);
    out
}

fn assert_ok_response(out: &Output) {
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "client exited non-zero: {:?}\nstdout:\n{stdout}\nstderr:\n{}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        stdout.contains("status=200"),
        "expected status=200; got:\n{stdout}"
    );
    assert!(
        stdout.contains(&format!("body={EXPECT_BODY}")),
        "expected the full body {EXPECT_BODY:?}; got:\n{stdout}"
    );
}

#[test]
fn async_stream_client_drives_a_full_exchange_over_the_union() {
    if !have_python() {
        eprintln!("skipping: python3 not available");
        return;
    }
    let source = format!(
        "IMPORT http\nIMPORT net\nIMPORT io\n\n\
         FUNC main AS Integer\n\
        \x20 LET u AS net::Url = net::toUrl(\"http://127.0.0.1:{PORT_ASYNC}/\")\n\
        \x20 RES s AS http::Stream STATE http::PendingState = http::startRead(u)\n\
        \x20 WHILE http::done(s) = FALSE\n\
        \x20   IF http::ready(s) THEN\n\
        \x20     http::pump(s)\n\
        \x20   END IF\n\
        \x20 END WHILE\n\
        \x20 LET resp AS http::Response = http::finish(s)\n\
        \x20 io::print(\"status=\" & toString(resp.status))\n\
        \x20 io::print(\"body=\" & toString(resp.body))\n\
        \x20 RETURN 0\n\
         END FUNC\n",
    );
    assert_ok_response(&drive(PORT_ASYNC, &source));
}

#[test]
fn blocking_read_over_the_async_core_yields_the_same_response() {
    if !have_python() {
        eprintln!("skipping: python3 not available");
        return;
    }
    // The rewritten blocking wrapper (Phase 4) drives the same core internally.
    let source = format!(
        "IMPORT http\nIMPORT net\nIMPORT io\n\n\
         FUNC main AS Integer\n\
        \x20 LET u AS net::Url = net::toUrl(\"http://127.0.0.1:{PORT_BLOCKING}/\")\n\
        \x20 LET resp AS http::Response = http::read(u)\n\
        \x20 io::print(\"status=\" & toString(resp.status))\n\
        \x20 io::print(\"body=\" & toString(resp.body))\n\
        \x20 RETURN 0\n\
         END FUNC\n",
    );
    assert_ok_response(&drive(PORT_BLOCKING, &source));
}
