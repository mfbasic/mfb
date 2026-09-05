//! bug-507: an `http::handleRequest` server must survive any anonymous client.
//!
//! - OS-51: a malformed chunk-size line raised out of the untrapped read loop and
//!   aborted the whole process (exit 255, "invalid chunk size").
//! - OS-52: the per-connection read had no deadline, so one idle connection
//!   wedged the single-threaded accept loop forever (slowloris).
//! - OS-56: the head was re-scanned from offset 0 on every read (quadratic), and
//!   nothing capped the head size, the header count, or a header line's length.
//!
//! Each test drives a real server over a raw socket. The server publishes its
//! host-assigned port through an atomically-written file.

mod common;

use std::io::{ErrorKind, Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const SERVER: &str = r#"IMPORT http
IMPORT tcp
IMPORT net
IMPORT fs
IMPORT collections

FUNC echo(req AS http::Request) AS http::Response
  RETURN http::ok("method=" & req.method & " bodyLen=" & toString(len(req.body)) & " nh=" & toString(len(req.headers)))
END FUNC

FUNC main AS Integer
  MUT routes AS List OF http::Route = []
  routes = collections::append(routes, http::route("/", echo))
  RES s AS tcp::Listener = http::server(0, "127.0.0.1")
  LET bound AS net::Address = tcp::localAddress(s)
  fs::writeTextAtomic("@PORTFILE@", toString(bound.port))
  MUT n AS Integer = 0
  WHILE n < 100
    http::handleRequest(s, routes)
    n = n + 1
  END WHILE
  RETURN 0
END FUNC
"#;

struct Server {
    child: Child,
    port: u16,
    project: PathBuf,
    port_file: PathBuf,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.port_file);
        let _ = std::fs::remove_dir_all(&self.project);
    }
}

fn nonce() -> String {
    common::unique_nonce()
}

fn spawn_server(tag: &str) -> Server {
    let port_file = std::env::temp_dir().join(format!(
        "mfb_b507_{tag}_port_{}_{}",
        std::process::id(),
        nonce()
    ));
    let _ = std::fs::remove_file(&port_file);
    let source = SERVER.replace("@PORTFILE@", &common::mfb_path_literal(&port_file));
    let project = common::temp_project(&format!("b507_{tag}"), &source);
    let exe = common::build_project(&project);
    let child = Command::new(&exe)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn mfb http server");
    let deadline = Instant::now() + Duration::from_secs(20);
    let port = loop {
        if let Ok(text) = std::fs::read_to_string(&port_file) {
            if let Ok(port) = text.trim().parse::<u16>() {
                if port != 0 {
                    break port;
                }
            }
        }
        assert!(
            Instant::now() < deadline,
            "mfb http server never published its port to {port_file:?}"
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    Server {
        child,
        port,
        project,
        port_file,
    }
}

/// Read until the peer closes or `timeout` passes without a byte.
fn read_reply(stream: &mut TcpStream, timeout: Duration) -> Vec<u8> {
    stream
        .set_read_timeout(Some(timeout))
        .expect("set read timeout");
    let mut reply = Vec::new();
    let mut buf = [0u8; 65536];
    loop {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => reply.extend_from_slice(&buf[..n]),
            Err(err)
                if err.kind() == ErrorKind::WouldBlock || err.kind() == ErrorKind::TimedOut =>
            {
                break
            }
            Err(err) if err.kind() == ErrorKind::ConnectionReset => break,
            Err(err) => panic!("read reply: {err}"),
        }
    }
    reply
}

/// Send `request` verbatim (without closing the write side — the server must
/// answer on its own) and return what came back.
fn send_raw(port: u16, request: &[u8], timeout: Duration) -> Vec<u8> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect to mfb http server");
    stream.write_all(request).expect("send request");
    read_reply(&mut stream, timeout)
}

fn status_line(reply: &[u8]) -> String {
    String::from_utf8_lossy(reply)
        .split("\r\n")
        .next()
        .unwrap_or_default()
        .to_string()
}

fn assert_still_serving(port: u16) {
    let reply = send_raw(
        port,
        b"GET / HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
        Duration::from_secs(20),
    );
    assert_eq!(
        status_line(&reply),
        "HTTP/1.1 200 OK",
        "the server must still serve a well-formed request afterwards; got:\n{:?}",
        String::from_utf8_lossy(&reply)
    );
}

// ---------------------------------------------------------------------------
// OS-51: a malformed chunk size is a 400 for that connection, not a process abort.
// ---------------------------------------------------------------------------

#[test]
fn malformed_chunk_size_is_answered_400_and_the_server_survives() {
    let srv = spawn_server("badchunk");
    let reply = send_raw(
        srv.port,
        b"POST / HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\n\r\nZZ\r\n",
        Duration::from_secs(20),
    );
    assert_eq!(
        status_line(&reply),
        "HTTP/1.1 400 Bad Request",
        "a malformed chunk size must be refused on that connection; got:\n{:?}",
        String::from_utf8_lossy(&reply)
    );
    assert_still_serving(srv.port);
}

// ---------------------------------------------------------------------------
// OS-52: an idle connection is dropped on the read deadline; the next client is served.
// ---------------------------------------------------------------------------

#[test]
fn idle_connection_is_dropped_on_the_read_deadline_and_the_next_client_is_served() {
    let srv = spawn_server("idle");
    // Sends a partial head and then goes silent — the slowloris shape.
    let mut idle = TcpStream::connect(("127.0.0.1", srv.port)).expect("connect idle client");
    idle.write_all(b"GET / HTTP/1.1\r\nHost: x\r\n")
        .expect("send partial head");
    std::thread::sleep(Duration::from_millis(300));

    let started = Instant::now();
    let mut second = TcpStream::connect(("127.0.0.1", srv.port)).expect("connect second client");
    second
        .write_all(b"GET / HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
        .expect("send second request");
    // Longer than the server's idle deadline, far shorter than "forever".
    let reply = read_reply(&mut second, Duration::from_secs(40));
    assert_eq!(
        status_line(&reply),
        "HTTP/1.1 200 OK",
        "the second client was not served within {:?} — the idle peer wedged the accept loop; got:\n{:?}",
        started.elapsed(),
        String::from_utf8_lossy(&reply)
    );
    // The idle peer was answered with 408 and closed, not silently abandoned mid-head.
    let idle_reply = read_reply(&mut idle, Duration::from_secs(5));
    assert_eq!(
        status_line(&idle_reply),
        "HTTP/1.1 408 Request Timeout",
        "the timed-out connection must be told why; got:\n{:?}",
        String::from_utf8_lossy(&idle_reply)
    );
}

// ---------------------------------------------------------------------------
// OS-56: caps on the head size, the header count, and a header line's length.
// ---------------------------------------------------------------------------

#[test]
fn oversized_head_is_rejected_with_431_before_the_terminator_arrives() {
    let srv = spawn_server("bighead");
    // 96 KiB of head with no terminator, write side left open: an uncapped
    // server keeps waiting for more; a capped one answers at once.
    let mut request = b"GET / HTTP/1.1\r\nHost: x\r\n".to_vec();
    for i in 0..(96 * 1024 / 1024) {
        request.extend_from_slice(format!("X-Pad-{i}: ").as_bytes());
        request.extend_from_slice(&vec![b'a'; 1000]);
        request.extend_from_slice(b"\r\n");
    }
    let started = Instant::now();
    let reply = send_raw(srv.port, &request, Duration::from_secs(20));
    assert_eq!(
        status_line(&reply),
        "HTTP/1.1 431 Request Header Fields Too Large",
        "an over-cap head must be refused without waiting for its terminator (waited {:?}); got:\n{:?}",
        started.elapsed(),
        String::from_utf8_lossy(&reply)
    );
    assert_still_serving(srv.port);
}

#[test]
fn too_many_header_fields_is_rejected_with_431() {
    let srv = spawn_server("manyheaders");
    let mut request = b"GET / HTTP/1.1\r\nHost: x\r\n".to_vec();
    for i in 0..200 {
        request.extend_from_slice(format!("X-H-{i}: v\r\n").as_bytes());
    }
    request.extend_from_slice(b"\r\n");
    let reply = send_raw(srv.port, &request, Duration::from_secs(20));
    assert_eq!(
        status_line(&reply),
        "HTTP/1.1 431 Request Header Fields Too Large",
        "200 header fields must be refused; got:\n{:?}",
        String::from_utf8_lossy(&reply)
    );
    assert_still_serving(srv.port);
}

#[test]
fn overlong_header_line_is_rejected_with_431() {
    let srv = spawn_server("longline");
    let mut request = b"GET / HTTP/1.1\r\nHost: x\r\nX-Long: ".to_vec();
    request.extend_from_slice(&vec![b'a'; 12 * 1024]);
    request.extend_from_slice(b"\r\n\r\n");
    let reply = send_raw(srv.port, &request, Duration::from_secs(20));
    assert_eq!(
        status_line(&reply),
        "HTTP/1.1 431 Request Header Fields Too Large",
        "a 12 KiB header line must be refused; got:\n{:?}",
        String::from_utf8_lossy(&reply)
    );
    assert_still_serving(srv.port);
}

/// A well-formed request just under every cap is still served — the caps must
/// not reject legitimate traffic.
#[test]
fn request_under_the_caps_is_still_served() {
    let srv = spawn_server("undercaps");
    let mut request = b"GET / HTTP/1.1\r\nHost: x\r\n".to_vec();
    for i in 0..40 {
        request.extend_from_slice(format!("X-H-{i}: ").as_bytes());
        request.extend_from_slice(&vec![b'a'; 1000]);
        request.extend_from_slice(b"\r\n");
    }
    request.extend_from_slice(b"\r\n");
    let reply = send_raw(srv.port, &request, Duration::from_secs(20));
    let t = String::from_utf8_lossy(&reply).into_owned();
    assert_eq!(status_line(&reply), "HTTP/1.1 200 OK", "got:\n{t:?}");
    assert!(t.ends_with("method=GET bodyLen=0 nh=41"), "got:\n{t:?}");
}

// ---------------------------------------------------------------------------
// OS-56: the frame scan is incremental — a body of many tiny chunks is linear.
// ---------------------------------------------------------------------------

#[test]
fn tiny_chunk_body_is_framed_in_linear_time() {
    let srv = spawn_server("tinychunks");
    // 4 MiB of one-byte chunks ("1\r\nA\r\n" each): ~700k chunk boundaries. A
    // scan that re-walks every chunk after each 64 KiB read does ~23M chunk
    // steps; an incremental one does ~700k.
    let chunks = 4 * 1024 * 1024 / 6;
    let mut request = b"POST / HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
    request.reserve(chunks * 6 + 8);
    for _ in 0..chunks {
        request.extend_from_slice(b"1\r\nA\r\n");
    }
    request.extend_from_slice(b"0\r\n\r\n");
    let started = Instant::now();
    let reply = send_raw(srv.port, &request, Duration::from_secs(120));
    let elapsed = started.elapsed();
    let t = String::from_utf8_lossy(&reply).into_owned();
    assert_eq!(status_line(&reply), "HTTP/1.1 200 OK", "got:\n{t:?}");
    assert!(
        t.ends_with(&format!("method=POST bodyLen={chunks} nh=2")),
        "the de-chunked body must be every chunk's byte; got:\n{t:?}"
    );
    assert!(
        elapsed < Duration::from_secs(30),
        "framing {chunks} one-byte chunks took {elapsed:?} — the chunk walk is being re-run from the body start on every read"
    );
}
