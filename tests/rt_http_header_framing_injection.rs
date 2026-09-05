//! bug-506: the `http` package must not let a control byte frame an extra request
//! or response, and its server-side head parser must refuse the request-smuggling
//! primitives.
//!
//! - OS-54 (client): `__http_normalizeMethod` rejected only `""` and a space, so a
//!   method carrying `\r\n` injected a second request line on the wire.
//! - OS-55 (server): `__http_serializeHead` interpolated `reason` and every
//!   header name/value raw, so an echoed value split the response.
//! - OS-53 (server): duplicate `Content-Length` was last-wins, `Content-Length`
//!   together with `Transfer-Encoding` was accepted, whitespace before `:` and
//!   obs-fold continuation lines were promoted to headers, `chunked` was matched
//!   as a substring, a bad `Content-Length` became 0, and the body was not
//!   truncated to `Content-Length`.
//!
//! Every server case drives a real `http::handleRequest` loop over a raw socket;
//! the well-formed exchange is pinned byte-for-byte so the fix cannot move it.

mod common;

use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// An echo server: the body reports what the parser produced, and three query
/// parameters let a request steer a header value, a header name, and the reason
/// phrase into the response — the reflection shapes OS-55 is about.
const SERVER: &str = r#"IMPORT http
IMPORT tcp
IMPORT net
IMPORT fs
IMPORT collections

FUNC echo(req AS http::Request) AS http::Response
  MUT r AS http::Response = http::ok("method=" & req.method & " bodyLen=" & toString(len(req.body)) & " nh=" & toString(len(req.headers)))
  LET dest AS String = collections::getOr(req.query, "to", "")
  IF dest <> "" THEN
    r = http::withHeader(r, "Location", dest)
  END IF
  LET hname AS String = collections::getOr(req.query, "hname", "")
  IF hname <> "" THEN
    r = http::withHeader(r, hname, "1")
  END IF
  LET why AS String = collections::getOr(req.query, "reason", "")
  IF why <> "" THEN
    r = WITH r { reason := why }
  END IF
  RETURN r
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
        "mfb_b506_{tag}_port_{}_{}",
        std::process::id(),
        nonce()
    ));
    let _ = std::fs::remove_file(&port_file);
    let source = SERVER.replace("@PORTFILE@", &port_file.to_string_lossy());
    let project = common::temp_project(&format!("b506_{tag}"), &source);
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

/// Send `request` verbatim and return everything the server sent before it
/// closed the connection (or the read deadline passed).
fn send_raw(port: u16, request: &[u8]) -> Vec<u8> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect to mfb http server");
    stream
        .set_read_timeout(Some(Duration::from_secs(20)))
        .expect("set read timeout");
    stream.write_all(request).expect("send request");
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
            Err(err) => panic!("read reply: {err}"),
        }
    }
    reply
}

fn text(reply: &[u8]) -> String {
    String::from_utf8_lossy(reply).into_owned()
}

fn status_line(reply: &[u8]) -> String {
    text(reply)
        .split("\r\n")
        .next()
        .unwrap_or_default()
        .to_string()
}

/// The reply's head must be exactly one status line plus `name: value` lines,
/// terminated by a blank line, with no control byte inside any line.
fn assert_well_framed(reply: &[u8]) {
    let t = text(reply);
    let (head, _) = t
        .split_once("\r\n\r\n")
        .unwrap_or_else(|| panic!("reply has no head terminator: {t:?}"));
    for (i, line) in head.split("\r\n").enumerate() {
        assert!(
            !line.bytes().any(|b| b < 0x20 || b == 0x7f),
            "head line {i} carries a control byte: {line:?}\nfull reply: {t:?}"
        );
        if i > 0 {
            assert!(
                line.contains(':'),
                "head line {i} is not a header field: {line:?}\nfull reply: {t:?}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// The pin: a well-formed exchange stays byte-identical.
// ---------------------------------------------------------------------------

#[test]
fn well_formed_request_reply_is_byte_identical() {
    let srv = spawn_server("pin");
    let reply = send_raw(
        srv.port,
        b"POST /?x=1 HTTP/1.1\r\nHost: localhost\r\nContent-Length: 4\r\nX-Custom: yes\r\n\r\nabcd",
    );
    assert_eq!(
        text(&reply),
        "HTTP/1.1 200 OK\r\ncontent-type: text/plain; charset=utf-8\r\nContent-Length: 26\r\nConnection: close\r\n\r\nmethod=POST bodyLen=4 nh=3"
    );
    // A handler-set header survives verbatim, and the reason phrase is honoured.
    let reply = send_raw(
        srv.port,
        b"GET /?to=/next&reason=Fine HTTP/1.1\r\nHost: localhost\r\n\r\n",
    );
    assert_eq!(
        text(&reply),
        "HTTP/1.1 200 Fine\r\ncontent-type: text/plain; charset=utf-8\r\nLocation: /next\r\nContent-Length: 25\r\nConnection: close\r\n\r\nmethod=GET bodyLen=0 nh=1"
    );
}

// ---------------------------------------------------------------------------
// OS-55: response splitting through a reflected header value / name / reason.
// ---------------------------------------------------------------------------

#[test]
fn crlf_in_a_reflected_header_value_does_not_split_the_response() {
    let srv = spawn_server("split_value");
    let reply = send_raw(
        srv.port,
        b"GET /?to=/x%0d%0aSet-Cookie:%20evil=1%0d%0a%0d%0a<html>injected HTTP/1.1\r\nHost: x\r\n\r\n",
    );
    let t = text(&reply);
    assert!(
        !t.contains("Set-Cookie: evil=1") && !t.contains("<html>injected"),
        "the reflected value split the response:\n{t:?}"
    );
    assert_well_framed(&reply);
    assert_eq!(
        status_line(&reply),
        "HTTP/1.1 500 Internal Server Error",
        "a handler response that cannot be serialized safely is a handler failure; got:\n{t:?}"
    );
}

#[test]
fn control_byte_in_a_header_name_or_reason_does_not_split_the_response() {
    let srv = spawn_server("split_name_reason");
    let by_name = send_raw(
        srv.port,
        b"GET /?hname=X-A%0d%0aSet-Cookie:%20evil=1%0d%0aX-B HTTP/1.1\r\nHost: x\r\n\r\n",
    );
    let t = text(&by_name);
    assert!(
        !t.contains("Set-Cookie: evil=1"),
        "the reflected header NAME split the response:\n{t:?}"
    );
    assert_well_framed(&by_name);
    assert_eq!(status_line(&by_name), "HTTP/1.1 500 Internal Server Error");

    let by_reason = send_raw(
        srv.port,
        b"GET /?reason=OK%0d%0aSet-Cookie:%20evil=1 HTTP/1.1\r\nHost: x\r\n\r\n",
    );
    let t = text(&by_reason);
    assert!(
        !t.contains("Set-Cookie: evil=1"),
        "the reflected REASON split the response:\n{t:?}"
    );
    assert_well_framed(&by_reason);
    assert_eq!(
        status_line(&by_reason),
        "HTTP/1.1 500 Internal Server Error"
    );

    // NUL is a control byte too (not just CR/LF).
    let by_nul = send_raw(srv.port, b"GET /?to=/x%00y HTTP/1.1\r\nHost: x\r\n\r\n");
    assert!(
        !by_nul.contains(&0u8),
        "a NUL reached the wire:\n{:?}",
        text(&by_nul)
    );
    assert_eq!(status_line(&by_nul), "HTTP/1.1 500 Internal Server Error");
}

// ---------------------------------------------------------------------------
// OS-53: the smuggling primitives are refused with 400.
// ---------------------------------------------------------------------------

#[test]
fn server_refuses_the_request_smuggling_primitives() {
    let srv = spawn_server("smuggle");
    let cases: &[(&str, &[u8])] = &[
        (
            "duplicate Content-Length",
            b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: 5\r\nContent-Length: 10\r\n\r\nabcdefghij",
        ),
        (
            "Content-Length together with Transfer-Encoding",
            b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: 4\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n",
        ),
        (
            "whitespace before the colon",
            b"GET / HTTP/1.1\r\nHost : x\r\n\r\n",
        ),
        (
            "obs-fold continuation line",
            b"GET / HTTP/1.1\r\nHost: x\r\n X-Fold: y\r\n\r\n",
        ),
        (
            "chunked is not the final transfer coding",
            b"POST / HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked, gzip\r\n\r\n0\r\n\r\n",
        ),
        (
            "a transfer coding other than chunked",
            b"POST / HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: gzip\r\n\r\n",
        ),
        (
            "non-numeric Content-Length",
            b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: abc\r\n\r\n",
        ),
        (
            "signed Content-Length",
            b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: +3\r\n\r\nabc",
        ),
    ];
    for (what, request) in cases {
        let reply = send_raw(srv.port, request);
        assert_eq!(
            status_line(&reply),
            "HTTP/1.1 400 Bad Request",
            "{what}: must be refused; got:\n{:?}",
            text(&reply)
        );
    }
}

#[test]
fn server_truncates_the_body_to_content_length() {
    let srv = spawn_server("truncate");
    let reply = send_raw(
        srv.port,
        b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: 3\r\n\r\nabcdefghij",
    );
    let t = text(&reply);
    assert_eq!(status_line(&reply), "HTTP/1.1 200 OK", "got:\n{t:?}");
    assert!(
        t.ends_with("method=POST bodyLen=3 nh=2"),
        "the body must be exactly Content-Length bytes; got:\n{t:?}"
    );
}

// ---------------------------------------------------------------------------
// OS-54: a client method carrying CRLF is rejected before anything is sent.
// ---------------------------------------------------------------------------

#[test]
fn client_method_with_crlf_is_rejected_before_connecting() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind peer");
    let port = listener.local_addr().expect("peer addr").port();
    let source = format!(
        "IMPORT http\nIMPORT net\nIMPORT io\n\n\
         FUNC main AS Integer\n\
        \x20 LET u AS net::Url = net::toUrl(\"http://127.0.0.1:{port}/\")\n\
        \x20 LET h AS Map OF String TO String = Map OF String TO String {{}}\n\
        \x20 LET r AS http::Response = http::read(u, h, \"GET\\r\\nX-Injected:1\\r\\nGET\") TRAP(e)\n\
        \x20   io::print(\"client error \" & toString(e.code) & \" \" & e.message)\n\
        \x20   RETURN 2\n\
        \x20 END TRAP\n\
        \x20 io::print(\"status=\" & toString(r.status))\n\
        \x20 RETURN 0\n\
         END FUNC\n"
    );
    let project = common::temp_project("b506_client_method", &source);
    let exe = common::build_project(&project);

    // The peer answers anything it receives, so a client that DOES connect
    // completes normally and the test can inspect what reached the wire.
    let peer = std::thread::spawn(move || {
        listener
            .set_nonblocking(true)
            .expect("nonblocking listener");
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            match listener.accept() {
                Ok((mut conn, _)) => {
                    let _ = conn.set_read_timeout(Some(Duration::from_secs(2)));
                    let mut got = Vec::new();
                    let mut buf = [0u8; 4096];
                    while !got.windows(4).any(|w| w == b"\r\n\r\n") {
                        match conn.read(&mut buf) {
                            Ok(0) => break,
                            Ok(n) => got.extend_from_slice(&buf[..n]),
                            Err(_) => break,
                        }
                    }
                    let _ = conn.write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    );
                    return Some(got);
                }
                Err(err) if err.kind() == ErrorKind::WouldBlock => {
                    if Instant::now() > deadline {
                        return None;
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(err) => panic!("peer accept: {err}"),
            }
        }
    });

    let out = Command::new(&exe).output().expect("run client");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    // Unblock the peer if the client never connected (the fixed behaviour).
    let _ = TcpStream::connect(("127.0.0.1", port));
    let got = peer.join().expect("peer thread");
    let _ = std::fs::remove_dir_all(&project);

    let wire = got.map(|g| String::from_utf8_lossy(&g).into_owned());
    assert!(
        !wire.as_deref().unwrap_or("").contains("X-INJECTED"),
        "the CRLF method reached the wire and injected a header:\n{wire:?}"
    );
    assert!(
        stdout.contains("client error 77050002"),
        "the client must reject the method with ErrInvalidArgument (77050002); got stdout:\n{stdout}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
