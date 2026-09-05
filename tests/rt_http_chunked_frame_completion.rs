//! bug-439: HTTP chunked-response completion must be detected by walking the chunk
//! framing, NOT by a naive `0\r\n\r\n` substring search. A chunked body larger than
//! one 64 KiB read whose data contains the literal bytes `0\r\n\r\n` before the real
//! terminating zero-length chunk used to make `__http_frameComplete` declare the
//! response complete early: the read loop stopped mid-body and `__http_dechunkBytes`
//! then overran the truncated buffer and failed with `truncated chunk data`.
//!
//! The peer here is a one-shot Python raw-socket HTTP/1.1 server that replies with a
//! single chunked body whose FIRST chunk is ~70 KiB and contains `0\r\n\r\n` a few
//! bytes in, followed by the genuine `0\r\n\r\n` terminator. Because the whole
//! response exceeds one 65536-byte client read, the stray sequence lands in an early
//! pump while the real terminator is still unread — exactly the shape that tripped
//! the old substring check. A correct framing walk keeps reading until the real
//! zero-length chunk arrives and returns the full body. Gated only on `python3`.

mod common;

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::Duration;

const PORT_CHUNKED: u16 = 18475;
// filler_a (100) + stray "0\r\n\r\n" (5) + filler_b (70000) = 70105 de-chunked bytes.
const EXPECT_BODY_LEN: usize = 70105;

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

/// One-shot HTTP/1.1 server: accept one connection, drain the request headers, then
/// reply with a Transfer-Encoding: chunked body whose first ~70 KiB chunk contains
/// the literal bytes `0\r\n\r\n` early in its data, followed by the real zero-length
/// terminator. The whole response is sent in one `sendall`, but it is larger than a
/// single 65536-byte client read, so the stray sequence is seen before the true
/// terminator. Prints `READY` once listening.
const SERVER_PY: &str = r#"
import socket, sys
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
stray = b"0\r\n\r\n"
chunk_data = b"A" * 100 + stray + b"B" * 70000
size_line = ("%x\r\n" % len(chunk_data)).encode()
body = size_line + chunk_data + b"\r\n" + b"0\r\n\r\n"
head = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n"
try:
    conn.sendall(head + body)
except OSError:
    pass
try:
    conn.close()
except OSError:
    pass
srv.close()
"#;

fn build_client(root: &Path, source: &str) -> std::path::PathBuf {
    fs::create_dir_all(root.join("src")).expect("create src dir");
    fs::write(
        root.join("project.json"),
        "{\"name\":\"httpc\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"executable\",\"sources\":[{\"root\":\"src\",\"role\":\"main\",\"include\":[\"**/*.mfb\"]}],\"entry\":\"main\",\"targets\":[\"native\"]}\n",
    )
    .expect("write project.json");
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
    let root = std::env::temp_dir().join(format!("mfb_b439_{}_{}", port, nonce()));
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
    {
        let out = server.stdout.take().expect("server stdout");
        let mut rdr = BufReader::new(out);
        let mut line = String::new();
        rdr.read_line(&mut line).expect("read server READY");
        assert!(
            line.starts_with("READY"),
            "server did not report READY: {line:?}"
        );
    }

    let server_pid = server.id();
    let (tx, rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let out = Command::new(&exe).output().expect("run http client");
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
            panic!("bug-439: http client did not finish within 30s (loop hung)");
        }
    };
    let _ = fs::remove_dir_all(&root);
    out
}

#[test]
fn chunked_body_containing_the_terminator_bytes_reads_the_whole_body() {
    if !have_python() {
        eprintln!("skipping: python3 not available");
        return;
    }
    let source = format!(
        "IMPORT http\nIMPORT net\nIMPORT io\n\n\
         FUNC main AS Integer\n\
        \x20 LET u AS net::Url = net::toUrl(\"http://127.0.0.1:{PORT_CHUNKED}/\")\n\
        \x20 LET resp AS http::Response = http::read(u)\n\
        \x20 io::print(\"status=\" & toString(resp.status))\n\
        \x20 io::print(\"bodyLen=\" & toString(len(resp.body)))\n\
        \x20 RETURN 0\n\
         END FUNC\n",
    );
    let out = drive(PORT_CHUNKED, &source);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "client exited non-zero (chunked completion mis-detected mid-body): {:?}\nstdout:\n{stdout}\nstderr:\n{}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        stdout.contains("status=200"),
        "expected status=200; got:\n{stdout}"
    );
    assert!(
        stdout.contains(&format!("bodyLen={EXPECT_BODY_LEN}")),
        "expected the full de-chunked body of {EXPECT_BODY_LEN} bytes; got:\n{stdout}"
    );
}
