//! bug-885: macOS `tls::listen` never validated `host`.
//!
//! `nw_endpoint_create_host(host, port)` was handed the caller's `host`
//! string verbatim. Network.framework treats anything that is not already a
//! usable local-bind endpoint — `"localhost"`, a typo, an IPv6 literal, a
//! link-local address with no scope id — as unusable for a LOCAL bind, and
//! silently listens on the WILDCARD address on an OS-chosen EPHEMERAL port
//! instead of failing. Measured (bug doc, `lsof`): `tls::listen("localhost",
//! 19571, ...)` reported `local localhost:54211` and bound `*:54211` — every
//! interface, a port the caller never asked for, with `tls::localAddress`
//! echoing the caller's string back so the program could not see what
//! happened. `tls::listen("fe80::1", ...)` never returned at all.
//!
//! The fix resolves a non-empty, non-numeric `host` with `getaddrinfo` under
//! an `AF_INET` hint before building the Network.framework endpoint — exactly
//! as `tcp::listen` resolves it — so `"localhost"` binds `127.0.0.1` on the
//! EXACT requested port, and anything that does not resolve to an IPv4
//! address (a malformed host, every IPv6-only literal, since the AF_INET
//! hint makes those unresolvable) raises `ErrAddressInvalid` (`77070001`)
//! promptly instead of binding anywhere or hanging.
//!
//! This test's two distinguishing, CI-safe observables (no LAN reachability
//! probe, which would need an unsafe/unavailable second interface):
//! 1. The port `tls::localAddress` reports after a successful `listen` is
//!    EXACTLY the port requested — under the bug, Network.framework silently
//!    substituted a totally different ephemeral port, so a mismatch is
//!    exactly what the unfixed build produces.
//! 2. A real TLS client (Python's `ssl`, not MFB) completes a handshake
//!    against `127.0.0.1:<that port>` and reads the byte the server wrote.
//!
//! And for the malformed-host cases, that `tls::listen` raises
//! `ErrAddressInvalid` within a short bounded wait (the unfixed build hung
//! indefinitely on `"fe80::1"`, measured: killed by a 4 s alarm, one run
//! stayed alive over 100 s).
//!
//! macOS only: this is the Network.framework-specific code path (bug-885).
//! `tcp::listen`/`udp::bind` and the Linux/Windows TLS backends already
//! resolve `host` correctly (measured in the bug doc) and are unaffected.

#![cfg(target_os = "macos")]

#[path = "../common/mod.rs"]
mod common;

use std::io::Read;
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

fn build_server(root: &Path, cert: &Path, key: &Path) -> std::path::PathBuf {
    let source = format!(
        r#"IMPORT io
IMPORT tls
IMPORT net
IMPORT os
IMPORT collections

FUNC main AS Integer
  LET args AS List OF String = os::args()
  LET h AS String = collections::get(args, 0)
  LET p AS Integer = toInt(collections::get(args, 1))
  RES l AS tls::Listener = tls::listen(h, p, "{cert}", "{key}") TRAP(e)
    io::print("listen ERR " & toString(e.code))
    RETURN 1
  END TRAP
  LET la AS net::Address = tls::localAddress(l)
  io::print("listen ok " & la.host & " " & toString(la.port))
  RES c = tls::accept(l, 20000) TRAP(e)
    io::print("accept ERR " & toString(e.code))
    RETURN 2
  END TRAP
  tls::write(c, "hi")
  tls::close(c)
  RETURN 0
END FUNC
"#,
        cert = common::mfb_path_literal(cert),
        key = common::mfb_path_literal(key),
    );
    let project = root.join("server");
    std::fs::create_dir_all(project.join("src")).expect("create src dir");
    std::fs::write(
        project.join("project.json"),
        "{\"name\":\"tlslistenhost\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"executable\",\"sources\":[{\"root\":\"src\",\"role\":\"main\",\"include\":[\"**/*.mfb\"]}],\"entry\":\"main\",\"targets\":[\"native\"]}\n",
    )
    .expect("write project.json");
    std::fs::write(project.join("src/main.mfb"), source).expect("write source");
    common::build_project(&project)
}

/// A one-shot TLS GET-nothing client: connect, handshake, read whatever the
/// server sends, return it as text. Python's `ssl`, not MFB.
const TLS_PEEK_CLIENT: &str = r#"
import socket, ssl, sys
port = int(sys.argv[1])
ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
ctx.check_hostname = False
ctx.verify_mode = ssl.CERT_NONE
raw = socket.create_connection(("127.0.0.1", port), timeout=10)
s = ctx.wrap_socket(raw, server_hostname="127.0.0.1")
data = s.recv(64)
sys.stdout.write(data.decode(errors="replace"))
"#;

fn tls_peek(port: u16) -> String {
    let out = Command::new(common::python_exe())
        .args(["-c", TLS_PEEK_CLIENT, &port.to_string()])
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

/// A port picked from the test's own PID so two runs of this test binary
/// rarely collide; this is a fixed REQUESTED port (not bind-then-release —
/// nothing else is handed this number), chosen deliberately because the bug
/// this test pins is specifically about whether the requested port is honored.
fn requested_port(tag: u16) -> u16 {
    20000 + ((std::process::id() as u16).wrapping_add(tag) % 9000)
}

fn have_openssl() -> bool {
    Command::new("openssl")
        .arg("version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[test]
fn localhost_binds_loopback_on_the_exact_requested_port() {
    if !have_openssl() {
        eprintln!("skipping: openssl not available");
        return;
    }
    let root = std::env::temp_dir().join(format!(
        "mfb_b885_localhost_{}_{}",
        std::process::id(),
        common::unique_nonce()
    ));
    std::fs::create_dir_all(&root).expect("create test root");
    let (cert, key) = write_cert(&root);
    let exe = build_server(&root, &cert, &key);
    let port = requested_port(1);

    let mut child = Command::new(&exe)
        .arg("localhost")
        .arg(port.to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn mfb tls listen server");

    // Read the "listen ok <host> <port>" line, bounded.
    let mut stdout = child.stdout.take().expect("child stdout");
    let mut line = Vec::new();
    let mut buf = [0u8; 1];
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if stdout.read(&mut buf).unwrap_or(0) == 0 {
            break;
        }
        line.push(buf[0]);
        if buf[0] == b'\n' || Instant::now() > deadline {
            break;
        }
    }
    let said = String::from_utf8_lossy(&line).into_owned();
    assert!(
        said.starts_with("listen ok"),
        "listen must succeed for \"localhost\"; got:\n{said}"
    );
    let reported_port: u16 = said
        .trim()
        .rsplit(' ')
        .next()
        .and_then(|p| p.parse().ok())
        .unwrap_or_else(|| panic!("could not parse reported port from {said:?}"));
    assert_eq!(
        reported_port, port,
        "the port actually bound must be the EXACT requested port, not a \
         substituted ephemeral one (the bug: Network.framework silently \
         fell back to `*:<ephemeral>` for a non-numeric host); got:\n{said:?}"
    );

    let peeked = tls_peek(port);
    let status = child.wait().expect("wait for server");
    let _ = std::fs::remove_dir_all(&root);

    assert_eq!(
        peeked, "hi",
        "a real TLS client must complete a handshake against 127.0.0.1:{port} \
         and read what the server wrote"
    );
    assert!(status.success(), "server must exit 0, not crash or hang");
}

#[test]
fn malformed_or_ipv6_hosts_raise_address_invalid_promptly() {
    if !have_openssl() {
        eprintln!("skipping: openssl not available");
        return;
    }
    let root = std::env::temp_dir().join(format!(
        "mfb_b885_bad_{}_{}",
        std::process::id(),
        common::unique_nonce()
    ));
    std::fs::create_dir_all(&root).expect("create test root");
    let (cert, key) = write_cert(&root);
    let exe = build_server(&root, &cert, &key);

    for (tag, host) in [
        (2u16, "not-an-ip"),
        (3, "999.1.1.1"),
        (4, "garbage::zz"),
        (5, "::1"),
        (6, "fe80::1"),
    ] {
        let port = requested_port(tag);
        let mut child = Command::new(&exe)
            .arg(host)
            .arg(port.to_string())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn mfb tls listen server");

        // Bounded wait: the unfixed build hung forever on "fe80::1" (one
        // measured run stayed alive over 100 s under a 4 s alarm).
        let deadline = Instant::now() + Duration::from_secs(10);
        let status = loop {
            if let Some(status) = child.try_wait().expect("poll child") {
                break status;
            }
            if Instant::now() > deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!(
                    "tls::listen({host:?}, ...) did not return within 10s \
                     (the bug-885 hang, e.g. for a link-local literal with no scope id)"
                );
            }
            std::thread::sleep(Duration::from_millis(20));
        };

        let mut out = String::new();
        child
            .stdout
            .take()
            .expect("child stdout")
            .read_to_string(&mut out)
            .expect("read child stdout");

        assert!(
            out.contains("listen ERR 77070001"),
            "host {host:?} must raise ErrAddressInvalid (77070001); got stdout:\n{out:?}"
        );
        assert!(
            !status.success(),
            "host {host:?}: the program must report the error, not exit 0"
        );
    }

    let _ = std::fs::remove_dir_all(&root);
}
