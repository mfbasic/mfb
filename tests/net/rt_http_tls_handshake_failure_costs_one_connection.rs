//! bug-859: a single non-TLS (or otherwise handshake-failing) connection to an
//! `http::serverSSL` listener must cost only that one connection, never the
//! accept loop.
//!
//! `http::handleRequest(tls::Listener, routes)` accepts a connection with
//! `tls::accept`, which performs the server-side TLS handshake. A peer that
//! sends plaintext (or any non-ClientHello bytes) fails that handshake, and
//! `tls::accept`'s own contract (closed connection, open listener,
//! `ErrTlsFailed`) means `handleRequest` raises `ErrTlsFailed` out to its
//! caller rather than swallowing it. The owner decision (2026-10-05) keeps
//! that raise — the caller can see and log the failure — but requires that it
//! cost only the one connection: the listener stays open, and the next
//! `handleRequest` call on it serves the next client normally.
//!
//! Before this was settled, the stdlib spec and `mfb man http handleRequest`
//! both claimed the accept loop was unconditionally "crash-proof" ("never
//! dies on a bad client") while in the same breath documenting that a
//! handshake failure raises `ErrTlsFailed` — and the spec's own worked
//! accept-loop example (`DO http::handleRequest(s, routes) LOOP UNTIL FALSE`)
//! has no `TRAP`, so following it verbatim lets one plaintext probe
//! (a port scanner, a health check, `curl http://` against an HTTPS port)
//! kill the whole HTTPS service — a single-packet remote DoS. This test pins
//! the corrected contract: a caller that traps `handleRequest` (as the fixed
//! examples now show) keeps serving after a failed handshake.
//!
//! The server binds port 0 and publishes the host-assigned port through an
//! atomically-written file (the `rt_http_handle_request_serves` pattern), so
//! it cannot collide with a concurrently running sibling test and never uses
//! bind-then-release.
//!
//! macOS and Linux only: cert minting shells out to `openssl req`, which the
//! other TLS tests in this directory already restrict the same way.

#![cfg(any(target_os = "macos", target_os = "linux"))]

#[path = "../common/mod.rs"]
mod common;

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn write_cert(root: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
    let cert = root.join("cert.pem");
    let key = root.join("key.pem");
    let status = Command::new("openssl")
        .args([
            "req",
            "-x509",
            "-newkey",
            "rsa:2048",
            "-keyout",
            key.to_str().unwrap(),
            "-out",
            cert.to_str().unwrap(),
            "-days",
            "2",
            "-nodes",
            "-subj",
            "/CN=127.0.0.1",
            "-addext",
            "subjectAltName=IP:127.0.0.1",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("run openssl req");
    assert!(
        status.success(),
        "openssl failed to generate a self-signed cert"
    );
    (cert, key)
}

/// A one-shot HTTPS GET over Python's `ssl` module (not MFB, so the proof
/// that request 3 is answered does not depend on the client under test).
const TLS_GET_CLIENT: &str = r#"
import socket, ssl, sys
port = int(sys.argv[1])
path = sys.argv[2]
ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
ctx.check_hostname = False
ctx.verify_mode = ssl.CERT_NONE
raw = socket.create_connection(("127.0.0.1", port), timeout=10)
s = ctx.wrap_socket(raw, server_hostname="127.0.0.1")
s.sendall(("GET %s HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n" % path).encode())
data = b""
while True:
    chunk = s.recv(65536)
    if not chunk:
        break
    data += chunk
sys.stdout.buffer.write(data)
"#;

fn tls_get(port: u16, path: &str) -> String {
    let out = Command::new(common::python_exe())
        .args(["-c", TLS_GET_CLIENT, &port.to_string(), path])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run python TLS client");
    if !out.stdout.is_empty() {
        String::from_utf8_lossy(&out.stdout).into_owned()
    } else {
        String::from_utf8_lossy(&out.stderr).into_owned()
    }
}

/// Builds the server exactly the way the corrected `handleRequest` example
/// now reads: every call is wrapped in a `TRAP` that logs and continues.
fn build_server(root: &Path, cert: &Path, key: &Path, port_file: &Path) -> std::path::PathBuf {
    let source = format!(
        r#"IMPORT io
IMPORT http
IMPORT tls
IMPORT net
IMPORT fs
IMPORT collections

FUNC home(req AS http::Request) AS http::Response
  RETURN http::ok("alive " & req.path)
END FUNC

FUNC main AS Integer
  RES s AS tls::Listener = http::serverSSL(0, "{cert}", "{key}", "127.0.0.1")
  LET at AS net::Address = tls::localAddress(s)
  fs::writeTextAtomic("{portfile}", toString(at.port))
  MUT rs AS List OF http::Route = []
  rs = collections::append(rs, http::route("/*", home))
  FOR i = 1 TO 3
    http::handleRequest(s, rs) TRAP(e)
      io::print("caught:" & toString(e.code) & "\n")
      RECOVER
    END TRAP
  NEXT
  io::print("loop-done\n")
  RETURN 0
END FUNC
"#,
        cert = common::mfb_path_literal(cert),
        key = common::mfb_path_literal(key),
        portfile = common::mfb_path_literal(port_file),
    );
    let project = root.join("server");
    std::fs::create_dir_all(project.join("src")).expect("create src dir");
    std::fs::write(
        project.join("project.json"),
        "{\"name\":\"httpstlshandshake\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"executable\",\"sources\":[{\"root\":\"src\",\"role\":\"main\",\"include\":[\"**/*.mfb\"]}],\"entry\":\"main\",\"targets\":[\"native\"]}\n",
    )
    .expect("write project.json");
    std::fs::write(project.join("src/main.mfb"), source).expect("write source");
    common::build_project(&project)
}

#[test]
fn handshake_failure_costs_one_connection_not_the_loop() {
    if !Command::new("openssl")
        .arg("version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
    {
        eprintln!("skipping: openssl not available");
        return;
    }

    let root = std::env::temp_dir().join(format!(
        "mfb_b859_{}_{}",
        std::process::id(),
        common::unique_nonce()
    ));
    std::fs::create_dir_all(&root).expect("create test root");
    let (cert, key) = write_cert(&root);
    let port_file = root.join("port.txt");
    let exe = build_server(&root, &cert, &key, &port_file);

    let mut server = Command::new(&exe)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn mfb https server");

    let deadline = Instant::now() + Duration::from_secs(20);
    let port = loop {
        if let Ok(text) = std::fs::read_to_string(&port_file) {
            if let Ok(port) = text.trim().parse::<u16>() {
                if port != 0 {
                    break port;
                }
            }
        }
        if Instant::now() > deadline {
            let _ = server.kill();
            let _ = server.wait();
            panic!("mfb https server never published its port to {port_file:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    };

    // Request 1: a real handshake, must succeed.
    let first = tls_get(port, "/one");
    assert!(
        first.starts_with("HTTP/1.1 200 OK\r\n"),
        "request 1 (good handshake) must succeed; got:\n{first:?}"
    );

    // Request 2: the exploit — plaintext bytes at a TLS listener. This is the
    // single packet that, under the unfixed "no TRAP" example, kills the
    // whole accept loop.
    {
        let mut stream =
            TcpStream::connect(("127.0.0.1", port)).expect("connect for garbage probe");
        stream
            .write_all(b"GARBAGE NOT TLS\r\n\r\n")
            .expect("write garbage bytes");
        let mut buf = [0u8; 64];
        let _ = stream.read(&mut buf); // server may close without replying; don't care
    }

    // Give the server a moment to run the failed accept and loop back.
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        server.try_wait().expect("poll server").is_none(),
        "the accept loop must survive a failed handshake, not exit the process"
    );

    // Request 3: a real handshake again, on the SAME listener, must still succeed.
    let third = tls_get(port, "/three");

    let status = server
        .wait()
        .expect("wait for server to finish its 3-iteration loop");
    let mut stdout = String::new();
    server
        .stdout
        .take()
        .expect("server stdout")
        .read_to_string(&mut stdout)
        .expect("read server stdout");

    let _ = std::fs::remove_dir_all(&root);

    assert!(
        third.starts_with("HTTP/1.1 200 OK\r\n"),
        "request 3, after the failed handshake, must still be served 200; got:\n{third:?}"
    );
    assert!(status.success(), "server process must exit 0, not crash");
    assert!(
        stdout.contains("caught:77070008"),
        "the TRAP must have observed ErrTlsFailed (77070008) for request 2; stdout:\n{stdout}"
    );
    assert!(
        stdout.contains("loop-done"),
        "the loop must run all 3 iterations, not die on request 2; stdout:\n{stdout}"
    );
}
