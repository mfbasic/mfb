//! bug-981: `http::read` must never write a raw SP, DEL, or non-ASCII byte
//! from a URL's path/query onto the request line.
//!
//! `__http_requestTarget` used to copy `url.path`/`url.query` verbatim into
//! the request line (`METHOD target HTTP/1.1`). Because `net::toUrl` keeps a
//! URL's path/query as the raw text it was given (bug-979 is the separate,
//! out-of-scope report that this diverges from WHATWG parsing), a path byte
//! the caller did not expect to be meaningful reached the wire unescaped:
//!
//! | `http::read(net::toUrl(...))` | request line sent (measured, oracle `/tmp/urlo/v4`) |
//! | --- | --- |
//! | `http://127.0.0.1:P/a b`        | `GET /a b HTTP/1.1` |
//! | `http://127.0.0.1:P/x HTTP/1.0` | `GET /x HTTP/1.0 HTTP/1.1` — a second,
//! attacker-chosen "HTTP version" token injected onto the request line |
//! | `http://127.0.0.1:P/é?q=a b`    | raw UTF-8 and a space |
//!
//! The owner decision (2026-10-05) is to percent-encode space, DEL and
//! non-ASCII bytes in the path/query before they are written — matching
//! Node's `new URL(...).pathname + search` — without double-encoding an
//! escape that is already present. Each case here plays the TCP peer itself
//! (bind port 0, no bind-then-release: the listener is held open by this
//! process and handed straight to the client, never released and
//! re-acquired by a second party) and asserts on the exact first line it
//! received — the "raw 127.0.0.1 server logs the request line" oracle the
//! bug doc specifies.

#[path = "../common/mod.rs"]
mod common;

use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

/// Runs an `http::read` against a URL built from `path_and_query` (appended
/// directly after `http://127.0.0.1:<peer-port>`), capturing the first line
/// the peer receives. The peer always answers a minimal 200 so the client
/// completes normally.
fn request_line_for(path_and_query: &str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind peer");
    let port = listener.local_addr().expect("peer addr").port();

    let source = format!(
        "IMPORT http\nIMPORT net\nIMPORT io\n\n\
         FUNC main AS Integer\n\
        \x20 LET u AS net::Url = net::toUrl(\"http://127.0.0.1:{port}{path_and_query}\")\n\
        \x20 LET r AS http::Response = http::read(u) TRAP(e)\n\
        \x20   io::print(\"client error \" & toString(e.code) & \" \" & e.message)\n\
        \x20   RETURN 2\n\
        \x20 END TRAP\n\
        \x20 io::print(\"status=\" & toString(r.status))\n\
        \x20 RETURN 0\n\
         END FUNC\n"
    );
    let project = common::temp_project(&format!("b981_{}", common::unique_nonce()), &source);
    let exe = common::build_project(&project);

    let peer = std::thread::spawn(move || {
        listener
            .set_nonblocking(true)
            .expect("nonblocking listener");
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            match listener.accept() {
                Ok((mut conn, _)) => {
                    let _ = conn.set_read_timeout(Some(Duration::from_secs(5)));
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
                    return got;
                }
                Err(err) if err.kind() == ErrorKind::WouldBlock => {
                    if Instant::now() > deadline {
                        return Vec::new();
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(err) => panic!("peer accept: {err}"),
            }
        }
    });

    let out = run_client(&exe);
    // Unblock the peer if the client somehow never connected.
    let _ = TcpStream::connect(("127.0.0.1", port));
    let got = peer.join().expect("peer thread");
    let _ = std::fs::remove_dir_all(&project);

    assert!(
        out.contains("status=200"),
        "client did not complete the exchange; stdout:\n{out}"
    );

    let wire = String::from_utf8_lossy(&got).into_owned();
    wire.split("\r\n").next().unwrap_or_default().to_string()
}

fn run_client(exe: &std::path::Path) -> String {
    let out = std::process::Command::new(exe)
        .output()
        .expect("run http client");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn space_in_path_is_percent_encoded() {
    let line = request_line_for("/a b");
    assert_eq!(
        line, "GET /a%20b HTTP/1.1",
        "a raw space must become %20, not a second wire token"
    );
}

#[test]
fn space_followed_by_text_cannot_inject_a_second_request_line_token() {
    // The exploit shape: a path that looks like it carries its own HTTP
    // version. Unescaped, this sends `GET /x HTTP/1.0 HTTP/1.1` — two
    // "HTTP/1.x" tokens on one line.
    let line = request_line_for("/x HTTP/1.0");
    assert_eq!(
        line, "GET /x%20HTTP/1.0 HTTP/1.1",
        "the path must stay one token; got:\n{line:?}"
    );
}

#[test]
fn non_ascii_path_byte_and_query_space_are_percent_encoded() {
    let line = request_line_for("/\u{e9}?q=a b");
    assert_eq!(
        line, "GET /%C3%A9?q=a%20b HTTP/1.1",
        "non-ASCII UTF-8 bytes and a query space must both be escaped; got:\n{line:?}"
    );
}

#[test]
fn an_already_percent_encoded_escape_is_not_double_encoded() {
    let line = request_line_for("/a%2Fb%20c");
    assert_eq!(
        line, "GET /a%2Fb%20c HTTP/1.1",
        "an existing %XX escape must be kept verbatim, not re-encoded to %25...; got:\n{line:?}"
    );
}

#[test]
fn a_lone_percent_sign_is_escaped_to_stay_well_formed() {
    let line = request_line_for("/100%off");
    assert_eq!(
        line, "GET /100%25off HTTP/1.1",
        "a bare % not starting a valid escape must become %25; got:\n{line:?}"
    );
}
