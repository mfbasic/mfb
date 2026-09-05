//! Regression test for bug-465 finding 3: `tls::localAddress` accepted only a
//! `Socket`, so a TLS server that bound port `0` had no way to learn which port
//! the OS gave it.
//!
//! `tcp::localAddress` registers Socket *and* Listener overloads and its own
//! documentation calls the port-`0` read-back "the only race-free way to bind".
//! `tls` registered the Socket form alone (`expected_arguments: Some("Socket")`),
//! so the identical program failed to compile:
//!
//! ```text
//! error[2-203-0021 TYPE_CALL_ARGUMENT_MISMATCH]: Call to `tls.localAddress`
//!   has argument type(s) (tls.Listener), expected Socket.
//! ```
//!
//! That is not hypothetical: `tls::remoteAddress`'s own man example binds
//! `tls::listen("127.0.0.1", 0, …)`, and nothing in that program — or in any
//! client of it — could discover the resulting port.
//!
//! ## Why this is a Rust integration test and not an rt-behavior fixture
//!
//! An MFBASIC TLS server needs a certificate/key pair. A static golden fixture
//! cannot carry one (it would expire, and committing a private key is its own
//! problem), so the identity is generated here at run time, exactly as
//! `rt_macos_tls_write_capacity.rs` does. The plaintext half of the same
//! contract needs no identity and *is* a fixture:
//! `tests/rt-behavior/tcp/tcp-read-eof-raises-rt` covers the `tcp` side of
//! bug-465.
//!
//! ## Why it connects rather than just printing the port
//!
//! Reading back a plausible-looking non-zero integer proves nothing: a stub that
//! returned a constant would satisfy it. The test dials the reported port with
//! `openssl s_client` and requires the server's payload to come back, so the
//! number is proven to be the port the listener is actually bound to.
//!
//! macOS and Linux only. Both run a TLS server (Network.framework and OpenSSL
//! respectively) and both reach the two lowerings this fix touches; Windows is
//! excluded because the `openssl` CLI this test uses as the peer is not a
//! dependable presence there.

#![cfg(any(target_os = "macos", target_os = "linux"))]

mod common;

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

/// What the server writes to the accepted peer. Receiving it back through a real
/// handshake is what proves the reported port is the listener's.
const PAYLOAD: &str = "listener-port-ok";

/// Wall-clock bound on each blocking interaction with the server, and the mfb
/// side's `tls::accept` deadline.
///
/// Generous on purpose. These bounds exist to turn a *hang* into a named failure
/// so a stalled TLS server cannot wedge `cargo test` — they are not performance
/// assertions, and every second here is free unless something is already broken.
/// The sibling `rt_macos_tls_write_capacity` was measured failing its 30s bound
/// purely from CPU starvation (three heavy jobs on one machine) and passing in
/// 67s when re-run alone. That failure is far more dangerous than a slow test,
/// because a starved timeout and a real TLS stall print the identical message —
/// so the number is set well clear of the contended case rather than close to
/// the observed-good one.
const DEADLINE: Duration = Duration::from_secs(120);
/// The mfb-side accept bound, in milliseconds, kept under [`DEADLINE`] so a
/// server that gives up reports `ErrTimeout` through the program rather than
/// tripping the harness's outer kill.
const ACCEPT_MS: u64 = 90_000;

fn nonce() -> String {
    common::unique_nonce()
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

fn write_cert(root: &Path) -> (PathBuf, PathBuf) {
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

/// Bind port 0, report what the OS chose through `tls::localAddress(listener)`,
/// then serve exactly one peer so the reported port can be verified.
fn build_project(root: &Path, cert: &Path, key: &Path) -> PathBuf {
    fs::create_dir_all(root.join("src")).expect("create src dir");
    fs::write(
        root.join("project.json"),
        "{\"name\":\"tlsbound\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"executable\",\"sources\":[{\"root\":\"src\",\"role\":\"main\",\"include\":[\"**/*.mfb\"]}],\"entry\":\"main\",\"targets\":[\"native\"]}\n",
    )
    .expect("write project.json");
    let source = format!(
        "IMPORT io\nIMPORT net\nIMPORT tls\n\n\
         FUNC main AS Integer\n\
        \x20 RES server = tls::listen(\"127.0.0.1\", 0, \"{cert}\", \"{key}\")\n\
        \x20 LET bound = tls::localAddress(server)\n\
        \x20 io::print(\"bound \" & bound.host & \" \" & toString(bound.port))\n\
        \x20 RES conn = tls::accept(server, {ACCEPT_MS})\n\
        \x20 tls::write(conn, \"{PAYLOAD}\")\n\
        \x20 tls::close(conn)\n\
        \x20 tls::close(server)\n\
        \x20 RETURN 0\n\
         END FUNC\n",
        cert = cert.display(),
        key = key.display(),
        ACCEPT_MS = ACCEPT_MS,
    );
    fs::write(root.join("src/main.mfb"), source).expect("write source");

    let output = Command::new(common::mfb_exe())
        .arg("build")
        .arg(root)
        .output()
        .expect("run mfb build");
    assert!(
        output.status.success(),
        "bug-465: a program calling tls::localAddress(listener) must build:\nstdout:\n{}\nstderr:\n{}",
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

/// Read the server's `bound <host> <port>` line, bounded so a wedged server
/// fails the test instead of hanging `cargo test`.
fn read_bound_line(server: &mut Child) -> (String, u16) {
    let stdout = server.stdout.take().expect("server stdout");
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        let _ = BufReader::new(stdout).read_line(&mut line);
        let _ = tx.send(line);
    });
    let line = match rx.recv_timeout(DEADLINE) {
        Ok(line) => line,
        Err(_) => {
            let _ = server.kill();
            panic!(
                "TLS server printed no bound address within {}s",
                DEADLINE.as_secs()
            );
        }
    };
    let mut parts = line.split_whitespace();
    assert_eq!(
        parts.next(),
        Some("bound"),
        "unexpected first line from the TLS server: {line:?}"
    );
    let host = parts.next().unwrap_or_default().to_string();
    let port = parts
        .next()
        .unwrap_or_default()
        .parse::<u16>()
        .unwrap_or_else(|_| panic!("could not parse a port out of {line:?}"));
    (host, port)
}

#[test]
fn tls_local_address_reports_the_port_a_listener_bound_to() {
    if !have_openssl() {
        eprintln!("skipping: openssl CLI not available");
        return;
    }
    let root = std::env::temp_dir().join(format!("mfb_bug465_{}", nonce()));
    fs::create_dir_all(&root).expect("create temp root");
    let (cert, key) = write_cert(&root);
    let exe = build_project(&root, &cert, &key);

    let mut server = Command::new(&exe)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn mfb tls server");
    let (host, port) = read_bound_line(&mut server);

    assert_eq!(
        host, "127.0.0.1",
        "tls::localAddress(listener) reported the wrong bound host"
    );
    assert_ne!(
        port, 0,
        "tls::localAddress(listener) still reports the requested port 0 rather \
         than the one the OS assigned — the read-back is the whole point"
    );

    // Prove the number is the real bound port: complete a handshake against it
    // and require the server's payload. A stubbed constant would fail here.
    let mut client = Command::new("openssl")
        .args([
            "s_client",
            "-connect",
            &format!("127.0.0.1:{port}"),
            "-quiet",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn openssl s_client");

    let server_pid = server.id();
    let client_pid = client.id();
    let (tx, rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        // Close s_client's stdin and send NOTHING through it. `-quiet` implies
        // `-ign_eof`, so the client exits when the *server* closes rather than on
        // its own EOF — the close alone is enough, and a byte written here is not.
        //
        // It is worse than unnecessary: the server never reads, so that byte sits
        // unread in its receive queue, and closing a socket with unread data makes
        // the kernel send RST instead of FIN. The RST then discards the payload
        // this test is about. Measured in an `ubuntu:24.04` container (OpenSSL
        // 3.0.13, the CI runner's version): writing "\n" gives `read:errno=104`
        // and empty stdout every run; sending nothing gives `listener-port-ok` and
        // exit 0. It passed locally only because OpenSSL 3.5.6 surfaces the
        // already-received record before reporting the reset.
        drop(client.stdin.take());
        let out = client.wait_with_output().expect("wait s_client");
        let _ = server.wait();
        let _ = tx.send(out);
    });

    let out = match rx.recv_timeout(DEADLINE) {
        Ok(out) => {
            let _ = worker.join();
            out
        }
        Err(_) => {
            for pid in [server_pid, client_pid] {
                let _ = Command::new("kill").args(["-9", &pid.to_string()]).status();
            }
            let _ = fs::remove_dir_all(&root);
            panic!(
                "TLS server did not serve the reported port {port} within {}s",
                DEADLINE.as_secs()
            );
        }
    };

    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains(PAYLOAD),
        "nothing was served on the port tls::localAddress reported ({port}); \
         s_client exited {:?} and saw {text:?}\ns_client stderr:\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );

    let _ = fs::remove_dir_all(&root);
}
