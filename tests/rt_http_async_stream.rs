//! plan-76-D Phase 2/3: the non-blocking http client drives a real HTTP exchange
//! end to end over the `Stream` resource union carrying plan-74 `PendingState`.
//!
//! A program binds `RES s AS http::Stream STATE PendingState = http::startRead(url)`,
//! then loops `IF http::ready(s) THEN http::pump(s)` until `http::done(s)`, and
//! `http::finish(s)` yields the `Response`. This exercises all five entry points +
//! the union STATE mutation (`state.raw` accumulation across pumps), which is only
//! expressible because plan-80 relocated the STATE slot (the D4 fix).
//!
//! The peer is a one-shot Python raw-socket HTTP/1.1 server (Connection: close, so
//! the terminator is peer EOF, matching what `__http_buildRequest` forces). Gated
//! only on `python3` being available (skipped otherwise). Not macOS-specific — the
//! plaintext client uses the `net` Socket variant, which every backend supports.

mod common;

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const PORT: u16 = 18473;

fn nonce() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock before epoch")
        .as_nanos()
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

/// A one-shot HTTP/1.1 server: accept one connection, drain the request headers,
/// reply with a fixed body sent in TWO writes (a small flush, then the rest after
/// a brief pause) so the client must `pump` more than once to accumulate the full
/// `state.raw` — proving multi-pump accumulation. Prints `READY` once listening.
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
# Split the response so the client accumulates across pumps.
conn.sendall(head + body[:5])
time.sleep(0.2)
conn.sendall(body[5:])
conn.close()
srv.close()
"#;

fn build_client(root: &Path) -> PathBuf {
    fs::create_dir_all(root.join("src")).expect("create src dir");
    fs::write(
        root.join("project.json"),
        "{\"name\":\"asyncstream\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"executable\",\"sources\":[{\"root\":\"src\",\"role\":\"main\",\"include\":[\"**/*.mfb\"]}],\"entry\":\"main\",\"targets\":[\"native\"]}\n",
    )
    .expect("write project.json");
    let source = format!(
        "IMPORT http\nIMPORT net\nIMPORT io\n\n\
         FUNC main AS Integer\n\
        \x20 LET u AS net::Url = net::toUrl(\"http://127.0.0.1:{PORT}/\")\n\
        \x20 RES s AS http::Stream STATE PendingState = http::startRead(u)\n\
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
    fs::write(root.join("src/main.mfb"), source).expect("write source");

    let output = Command::new(common::mfb_exe())
        .arg("build")
        .arg(root)
        .output()
        .expect("run mfb build");
    assert!(
        output.status.success(),
        "client build failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("utf8 build output");
    stdout
        .lines()
        .find_map(|line| line.strip_prefix("Wrote executable to "))
        .map(PathBuf::from)
        .expect("build output executable path")
}

#[test]
fn async_stream_client_drives_a_full_exchange_over_the_union() {
    if !have_python() {
        eprintln!("skipping: python3 not available");
        return;
    }
    let root = std::env::temp_dir().join(format!("mfb_p76d_async_{}", nonce()));
    fs::create_dir_all(&root).expect("create temp root");
    let py = root.join("server.py");
    fs::write(&py, SERVER_PY).expect("write server.py");
    let exe = build_client(&root);

    // Start the one-shot server; wait for its READY line before connecting.
    let mut server = Command::new("python3")
        .arg(&py)
        .arg(PORT.to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn python http server");
    {
        let out = server.stdout.take().expect("server stdout");
        let mut rdr = BufReader::new(out);
        let mut line = String::new();
        rdr.read_line(&mut line).expect("read server READY");
        assert!(line.starts_with("READY"), "server did not report READY: {line:?}");
    }

    // Drive the client with a hard deadline so a hang (e.g. `ready` never firing)
    // fails fast instead of wedging the suite.
    let server_pid = server.id();
    let (tx, rx) = std::sync::mpsc::channel();
    let exe2 = exe.clone();
    let worker = std::thread::spawn(move || {
        let out = Command::new(&exe2).output().expect("run async client");
        let _ = tx.send(out);
    });

    let out = match rx.recv_timeout(Duration::from_secs(30)) {
        Ok(out) => {
            let _ = worker.join();
            let _ = server.wait();
            out
        }
        Err(_) => {
            let _ = Command::new("kill")
                .args(["-9", &server_pid.to_string()])
                .status();
            let _ = fs::remove_dir_all(&root);
            panic!(
                "plan-76-D: async client did not finish within 30s — startRead/ready/\
                 pump/done/finish loop hung (ready never fired, or done never true)"
            );
        }
    };

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "async client exited non-zero: {:?}\nstdout:\n{stdout}\nstderr:\n{}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        stdout.contains("status=200"),
        "expected status=200 from the async exchange; got:\n{stdout}"
    );
    assert!(
        stdout.contains("body=hello-async-stream-BODY"),
        "expected the full accumulated body (multi-pump); got:\n{stdout}"
    );

    let _ = fs::remove_dir_all(&root);
}
