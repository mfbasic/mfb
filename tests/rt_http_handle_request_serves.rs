//! bug-476: `http::handleRequest` must actually write the response it built.
//!
//! The documented server accepted a connection, parsed the request, dispatched it
//! and produced a correct `Response` — and then wrote nothing at all, closing the
//! socket so every client saw an empty reply (`curl`: "Empty reply from server").
//! Eight of the `http` page's 38 examples are server-shaped and all eight failed
//! on it.
//!
//! The cause was not in `http`: `tcp::write(sock, __http_serializeHead(resp))`
//! passes a String produced by a CALL, and the byte-vs-text code-form selection in
//! `builder_values::lower_runtime_helper_call` could not name a call result's
//! type, so it fell to the bytes lowering and marshalled the `String*` as a
//! collection. The write raised `ErrConnectionClosed` into `handleRequest`'s
//! silent `EXIT SUB`. See `rt_native_write_overload_call_argument.rs` for the
//! mechanism in isolation; this test pins the user-visible contract.
//!
//! The server binds port 0 and publishes the host-assigned port through an
//! atomically-written file, so it cannot collide with a concurrently running
//! sibling test.

mod common;

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

const SOURCE: &str = r#"IMPORT http
IMPORT tcp
IMPORT net
IMPORT fs
IMPORT collections

FUNC home(req AS http::Request) AS http::Response
  RETURN http::ok("hello from " & req.path)
END FUNC

FUNC main AS Integer
  MUT routes AS List OF http::Route = []
  routes = collections::append(routes, http::route("/", home))
  RES s AS tcp::Listener = http::server(0)
  LET bound AS net::Address = tcp::localAddress(s)
  fs::writeTextAtomic("@PORTFILE@", toString(bound.port))
  http::handleRequest(s, routes)
  http::handleRequest(s, routes)
  RETURN 0
END FUNC
"#;

/// One request/response exchange with `Connection: close`, returning everything
/// the server sent before it closed.
fn exchange(port: u16, path: &str) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect to mfb http server");
    stream
        .set_read_timeout(Some(Duration::from_secs(20)))
        .expect("set read timeout");
    stream
        .write_all(
            format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
                .as_bytes(),
        )
        .expect("send request");
    let mut reply = Vec::new();
    stream.read_to_end(&mut reply).expect("read reply");
    String::from_utf8_lossy(&reply).into_owned()
}

#[test]
fn handle_request_writes_the_response_it_built() {
    let port_file = std::env::temp_dir().join(format!(
        "mfb_b476_port_{}_{}",
        std::process::id(),
        common::unique_nonce()
    ));
    let _ = std::fs::remove_file(&port_file);
    let source = SOURCE.replace("@PORTFILE@", &port_file.to_string_lossy());
    let project = common::temp_project("b476_handle_request", &source);
    let exe = common::build_project(&project);

    let mut server = std::process::Command::new(&exe)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn mfb http server");

    // Wait for the server to publish its host-assigned port.
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
            panic!("mfb http server never published its port to {port_file:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    };

    let matched = exchange(port, "/");
    let unmatched = exchange(port, "/no-such-route");

    let _ = server.wait();
    let _ = std::fs::remove_file(&port_file);
    let _ = std::fs::remove_dir_all(&project);

    assert!(
        matched.starts_with("HTTP/1.1 200 OK\r\n"),
        "a matched route must be answered 200; got:\n{matched:?}"
    );
    assert!(
        matched.contains("Content-Length: 12\r\n") && matched.ends_with("hello from /"),
        "the handler's body must reach the wire; got:\n{matched:?}"
    );
    assert!(
        unmatched.starts_with("HTTP/1.1 404 Not Found\r\n"),
        "a path matching no route must be answered 404; got:\n{unmatched:?}"
    );
}
