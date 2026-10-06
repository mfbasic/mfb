//! bug-858: `http::read` (and the `startRead`/`pump`/`done`/`finish` drive) must
//! read a close-delimited response body — one whose head carries neither
//! `Content-Length` nor `Transfer-Encoding: chunked` — all the way to the peer's
//! EOF (RFC 9112 §6.3 rule 8). `__http_framingLength` returned `0` for "no
//! framing header", and `__http_frameComplete`'s `have >= framing` then read as
//! `have >= 0`, which is true the instant the head's `\r\n\r\n` is seen — so the
//! read loop (`WHILE __http_done(s) = FALSE: __http_pump(s)`) exits before any
//! body byte that arrives on a LATER `tcp::read` is ever pumped in. A peer that
//! flushes head and body in the same TCP segment happened to work by accident;
//! one that pauses between them (exactly RFC 9112's intended shape, and any
//! `Connection: close` reply from a slow handler) did not.
//!
//! The second half of the same bug: a `Content-Length: N` response is not
//! truncated to N bytes — `__http_decodeBody` returns the whole `bodySection` it
//! is handed — so bytes past the declared length that happen to share the same
//! read as the head leak into `body`.
//!
//! Both peers here are a real `std::net::TcpListener` bound once (`127.0.0.1:0`)
//! and used directly for the one connection each test drives — never
//! bind-then-release-then-rebind, which races a concurrent test for the same
//! ephemeral port.

#[path = "../common/mod.rs"]
mod common;

use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::Duration;

fn nonce() -> String {
    common::unique_nonce()
}

/// Read bytes from `stream` until the `\r\n\r\n` head terminator has been seen
/// (the client sends a body-less GET, so nothing past the head is expected).
fn read_request_head(stream: &mut std::net::TcpStream) {
    stream
        .set_read_timeout(Some(Duration::from_secs(20)))
        .expect("set read timeout");
    let mut got = Vec::new();
    let mut buf = [0u8; 4096];
    while !got.windows(4).any(|w| w == b"\r\n\r\n") {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => got.extend_from_slice(&buf[..n]),
            Err(err) => panic!("read request head: {err}"),
        }
    }
}

fn build_and_run_client(tag: &str, port: u16, extra: &str) -> std::process::Output {
    let source = format!(
        "IMPORT http\nIMPORT net\nIMPORT io\nIMPORT encoding\n\n\
         FUNC main AS Integer\n\
        \x20 LET u AS net::Url = net::toUrl(\"http://127.0.0.1:{port}/\")\n\
        \x20 LET r AS http::Response = http::read(u)\n\
        \x20 io::print(\"status=\" & toString(r.status))\n\
        \x20 io::print(\"bodylen=\" & toString(len(r.body)))\n\
        \x20 io::print(\"bodyhex=\" & encoding::hexEncode(r.body))\n\
        {extra}\
        \x20 RETURN 0\n\
         END FUNC\n"
    );
    let project = common::temp_project(&format!("b858_{tag}_{}", nonce()), &source);
    let exe = common::build_project(&project);
    let out = std::process::Command::new(&exe)
        .output()
        .expect("run http client");
    let _ = std::fs::remove_dir_all(&project);
    out
}

fn hex(bytes: &[u8]) -> String {
    common::hex(bytes)
}

// ---------------------------------------------------------------------------
// Close-delimited: no Content-Length, no chunked — read to EOF.
// ---------------------------------------------------------------------------

#[test]
fn close_delimited_body_split_across_reads_is_not_dropped() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind peer");
    let port = listener.local_addr().expect("peer addr").port();
    let body: &[u8] = b"hello-close-delimited";

    let peer = std::thread::spawn(move || {
        let (mut conn, _) = listener.accept().expect("accept");
        read_request_head(&mut conn);
        // Head only, no Content-Length, no Transfer-Encoding: the response is
        // close-delimited. Flushed separately from the body so a client that
        // stops reading at the head terminator (the bug) sees an empty body.
        conn.write_all(b"HTTP/1.1 200 OK\r\nServer: x\r\n\r\n")
            .expect("write head");
        conn.flush().expect("flush head");
        std::thread::sleep(Duration::from_millis(300));
        conn.write_all(body).expect("write body");
        conn.flush().expect("flush body");
        // Dropping `conn` here closes the socket — the EOF that delimits the
        // body per RFC 9112 §6.3 rule 8.
    });

    let out = build_and_run_client("close_delim", port, "");
    peer.join().expect("peer thread");

    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.success(),
        "client failed:\nstdout:\n{stdout}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        stdout.contains("status=200"),
        "expected status=200; got:\n{stdout}"
    );
    assert!(
        stdout.contains(&format!("bodylen={}", body.len())),
        "expected bodylen={} (the close-delimited body must be read to EOF, \
         not dropped at the head terminator); got:\n{stdout}",
        body.len()
    );
    assert!(
        stdout.contains(&format!("bodyhex={}", hex(body))),
        "expected bodyhex={}; got:\n{stdout}",
        hex(body)
    );
}

// ---------------------------------------------------------------------------
// Content-Length: body is exactly N bytes, even when more bytes share the read.
// ---------------------------------------------------------------------------

#[test]
fn content_length_body_is_truncated_to_the_declared_length() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind peer");
    let port = listener.local_addr().expect("peer addr").port();

    let peer = std::thread::spawn(move || {
        let (mut conn, _) = listener.accept().expect("accept");
        read_request_head(&mut conn);
        // Content-Length: 2, but six body bytes are sent in the SAME write as
        // the head — the extra four bytes must never reach `body`.
        conn.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nabcdef")
            .expect("write head+body");
        conn.flush().expect("flush");
        // The peer deliberately does not close immediately: a correct client
        // stops at Content-Length and never blocks waiting for EOF.
        std::thread::sleep(Duration::from_millis(200));
    });

    let out = build_and_run_client("content_length_trunc", port, "");
    peer.join().expect("peer thread");

    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.success(),
        "client failed:\nstdout:\n{stdout}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        stdout.contains("status=200"),
        "expected status=200; got:\n{stdout}"
    );
    assert!(
        stdout.contains("bodylen=2"),
        "expected bodylen=2 (truncated to Content-Length); got:\n{stdout}"
    );
    assert!(
        stdout.contains(&format!("bodyhex={}", hex(b"ab"))),
        "expected bodyhex={} (\"ab\", not the trailing \"cdef\"); got:\n{stdout}",
        hex(b"ab")
    );
}

// ---------------------------------------------------------------------------
// The non-blocking drive (startRead/ready/pump/done/finish) must agree.
// ---------------------------------------------------------------------------

#[test]
fn close_delimited_body_is_not_dropped_by_the_non_blocking_drive() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind peer");
    let port = listener.local_addr().expect("peer addr").port();
    let body: &[u8] = b"hello-close-delimited-async";

    let peer = std::thread::spawn(move || {
        let (mut conn, _) = listener.accept().expect("accept");
        read_request_head(&mut conn);
        conn.write_all(b"HTTP/1.1 200 OK\r\nServer: x\r\n\r\n")
            .expect("write head");
        conn.flush().expect("flush head");
        std::thread::sleep(Duration::from_millis(300));
        conn.write_all(body).expect("write body");
        conn.flush().expect("flush body");
    });

    let source = format!(
        "IMPORT http\nIMPORT net\nIMPORT io\nIMPORT encoding\nIMPORT datetime\n\n\
         FUNC main AS Integer\n\
        \x20 LET u AS net::Url = net::toUrl(\"http://127.0.0.1:{port}/\")\n\
        \x20 RES s AS http::Stream STATE http::PendingState = http::startRead(u)\n\
        \x20 LET startNs AS Integer = datetime::monotonicNanos()\n\
        \x20 WHILE http::done(s) = FALSE\n\
        \x20\x20 IF http::ready(s) THEN\n\
        \x20\x20\x20 http::pump(s)\n\
        \x20\x20 END IF\n\
        \x20\x20 IF datetime::monotonicNanos() - startNs > 20000000000 THEN\n\
        \x20\x20\x20 io::print(\"TIMEOUT\")\n\
        \x20\x20\x20 RETURN 1\n\
        \x20\x20 END IF\n\
        \x20 END WHILE\n\
        \x20 LET r AS http::Response = http::finish(s)\n\
        \x20 io::print(\"status=\" & toString(r.status))\n\
        \x20 io::print(\"bodylen=\" & toString(len(r.body)))\n\
        \x20 io::print(\"bodyhex=\" & encoding::hexEncode(r.body))\n\
        \x20 RETURN 0\n\
         END FUNC\n"
    );
    let project = common::temp_project(&format!("b858_async_{}", nonce()), &source);
    let exe = common::build_project(&project);
    let out = std::process::Command::new(&exe)
        .output()
        .expect("run http client");
    let _ = std::fs::remove_dir_all(&project);
    peer.join().expect("peer thread");

    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.success(),
        "client failed:\nstdout:\n{stdout}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        stdout.contains(&format!("bodylen={}", body.len())),
        "non-blocking drive dropped the close-delimited body; got:\n{stdout}"
    );
    assert!(
        stdout.contains(&format!("bodyhex={}", hex(body))),
        "expected bodyhex={}; got:\n{stdout}",
        hex(body)
    );
}

// ---------------------------------------------------------------------------
// A categorically bodyless status (RFC 9112 §6.3 rules 1-2: 1xx, 204, 304)
// must complete at the end of the head, before any Content-Length/
// Transfer-Encoding/close-delimited arm -- even when the peer holds the
// connection open afterward. Regression found in review: the first fix for
// this bug introduced a close-delimited (-2) sentinel that made EVERY
// response with no Content-Length and no chunked framing wait for EOF,
// which wrongly included 204/304 (no Content-Length is the common case for
// both). A 204/304 from a server that then sits on the socket must still
// return promptly.
// ---------------------------------------------------------------------------

/// Run the blocking `http::read` client against a peer that sends `head`,
/// then holds the connection open for `hold` before closing it. Returns how
/// long the client took, its stdout, and its exit status.
fn run_against_held_open_peer(
    head: &[u8],
    hold: Duration,
) -> (Duration, String, std::process::ExitStatus) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind peer");
    let port = listener.local_addr().expect("peer addr").port();
    let head = head.to_vec();
    let peer = std::thread::spawn(move || {
        let (mut conn, _) = listener.accept().expect("accept");
        read_request_head(&mut conn);
        conn.write_all(&head).expect("write head");
        conn.flush().expect("flush head");
        std::thread::sleep(hold);
        // `conn` drops here, closing the socket -- the correct client must
        // already be done long before this point.
    });

    let source = format!(
        "IMPORT http\nIMPORT net\nIMPORT io\n\n\
         FUNC main AS Integer\n\
        \x20 LET u AS net::Url = net::toUrl(\"http://127.0.0.1:{port}/\")\n\
        \x20 LET r AS http::Response = http::read(u)\n\
        \x20 io::print(\"status=\" & toString(r.status))\n\
        \x20 io::print(\"bodylen=\" & toString(len(r.body)))\n\
        \x20 RETURN 0\n\
         END FUNC\n"
    );
    let project = common::temp_project(&format!("b858_held_{}", nonce()), &source);
    let exe = common::build_project(&project);
    let start = std::time::Instant::now();
    let out = std::process::Command::new(&exe)
        .output()
        .expect("run http client");
    let elapsed = start.elapsed();
    let _ = std::fs::remove_dir_all(&project);
    peer.join().expect("peer thread");
    (
        elapsed,
        String::from_utf8_lossy(&out.stdout).into_owned(),
        out.status,
    )
}

#[test]
fn status_204_completes_without_waiting_for_the_peer_to_close() {
    let (elapsed, stdout, status) =
        run_against_held_open_peer(b"HTTP/1.1 204 No Content\r\n\r\n", Duration::from_secs(6));
    assert!(status.success(), "client failed:\n{stdout}");
    assert!(stdout.contains("status=204"), "got:\n{stdout}");
    assert!(stdout.contains("bodylen=0"), "got:\n{stdout}");
    assert!(
        elapsed < Duration::from_secs(3),
        "a 204 is categorically bodyless (RFC 9112 §6.3 rule 2) and must \
         complete at the end of the head, not wait for the peer's 6s-delayed \
         close; took {elapsed:?}"
    );
}

#[test]
fn status_304_completes_without_waiting_for_the_peer_to_close() {
    let (elapsed, stdout, status) =
        run_against_held_open_peer(b"HTTP/1.1 304 Not Modified\r\n\r\n", Duration::from_secs(6));
    assert!(status.success(), "client failed:\n{stdout}");
    assert!(stdout.contains("status=304"), "got:\n{stdout}");
    assert!(stdout.contains("bodylen=0"), "got:\n{stdout}");
    assert!(
        elapsed < Duration::from_secs(3),
        "a 304 is categorically bodyless (RFC 9112 §6.3 rule 2) and must \
         complete at the end of the head, not wait for the peer's 6s-delayed \
         close; took {elapsed:?}"
    );
}
