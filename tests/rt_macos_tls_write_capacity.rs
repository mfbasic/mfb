//! Regression test for bug-157: on macOS, `tls::write` of a `List OF Byte`
//! whose CAPACITY exceeds its COUNT addressed the byte payload from
//! `HEADER + COUNT*ENTRY` instead of `HEADER + CAPACITY*ENTRY`, so it copied
//! `count` bytes from *inside* the entry array — silent wrong data over TLS.
//!
//! This is the same CAPACITY-vs-COUNT class as commit e7b48c0f (fixed in the
//! net/openssl path); the macOS `SecureTransport` write path was missed. The
//! OpenSSL sibling (`src/codegen/builtins/tls/native/openssl.rs`) is the correct
//! reference and loads `COLLECTION_OFFSET_CAPACITY` for the payload-base
//! multiply.
//!
//! The test stands up an mfb TLS server (the affected write path) and connects
//! with `openssl s_client` as the peer, asserting it receives the exact bytes.
//! An append-built list carries spare capacity (`count == 5`, capacity grown to
//! 8), which is precisely the case that mis-addressed pre-fix. Gated to macOS —
//! only there does the binary embed the `macos.rs` write path; on Linux/x86 the
//! already-correct OpenSSL path is used.

#![cfg(target_os = "macos")]

mod common;

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

/// The server writes these exact bytes back after the client's greeting. They
/// are appended one at a time so the `List OF Byte` ends with `count == 5` and a
/// grown capacity (spare slots), reproducing the mis-addressing condition.
const EXPECTED: &[u8] = &[65, 66, 67, 68, 69]; // "ABCDE"
const PORT: u16 = 18453;

/// Wall-clock bound on the peer exchange, guarding bug-386's
/// `dispatch_semaphore_wait(FOREVER)` stall so a regression fails this test
/// instead of wedging `cargo test` for 36+ minutes.
///
/// It was 30s, and 30s was measured FAILING here purely from CPU starvation
/// (three heavy jobs on one machine) while the same code PASSED in 67.24s run
/// alone. That is the worst kind of flake: a starved timeout and a genuine
/// FOREVER stall print the identical message, so the false red looks exactly
/// like the bug this bound exists to catch.
///
/// The bound only has to fire before CI's job-level limit does — it is not a
/// performance assertion, and every second is free unless something is already
/// broken. Set clear of the contended case rather than close to the
/// observed-good one. Matches the reasoning on
/// `rt_tls_listener_local_address.rs`'s `DEADLINE`.
const STALL_DEADLINE: Duration = Duration::from_secs(120);

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

fn build_project(root: &Path, cert: &Path, key: &Path) -> PathBuf {
    fs::create_dir_all(root.join("src")).expect("create src dir");
    fs::write(
        root.join("project.json"),
        "{\"name\":\"tlswrite\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"executable\",\"sources\":[{\"root\":\"src\",\"role\":\"main\",\"include\":[\"**/*.mfb\"]}],\"entry\":\"main\",\"targets\":[\"native\"]}\n",
    )
    .expect("write project.json");
    // Build the byte list via append so CAPACITY > COUNT (the bug condition).
    let source = format!(
        "IMPORT collections\nIMPORT encoding\nIMPORT tls\n\n\
         FUNC serveOnce(RES listener AS tls::Listener) AS Integer\n\
        \x20 RES client = tls::accept(listener)\n\
        \x20 LET greeting = encoding::utf8Decode(tls::read(client, 16))\n\
        \x20 MUT payload AS List OF Byte = [65]\n\
        \x20 payload = collections::append(payload, toByte(66))\n\
        \x20 payload = collections::append(payload, toByte(67))\n\
        \x20 payload = collections::append(payload, toByte(68))\n\
        \x20 payload = collections::append(payload, toByte(69))\n\
        \x20 tls::write(client, payload)\n\
        \x20 tls::close(client)\n\
        \x20 RETURN len(greeting)\n\
         END FUNC\n\n\
         FUNC main AS Integer\n\
        \x20 RES s = tls::listen(\"127.0.0.1\", {PORT}, \"{cert}\", \"{key}\")\n\
        \x20 LET n AS Integer = serveOnce(s)\n\
        \x20 tls::close(s)\n\
        \x20 RETURN 0\n\
         END FUNC\n",
        cert = cert.display(),
        key = key.display(),
    );
    fs::write(root.join("src/main.mfb"), source).expect("write source");

    let output = Command::new(common::mfb_exe())
        .arg("build")
        .arg(root)
        .output()
        .expect("run mfb build");
    assert!(
        output.status.success(),
        "build failed:\nstdout:\n{}\nstderr:\n{}",
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
fn macos_tls_write_sends_capacity_over_count_byte_list_exactly() {
    if !have_openssl() {
        eprintln!("skipping: openssl CLI not available");
        return;
    }
    let root = std::env::temp_dir().join(format!("mfb_bug157_{}", nonce()));
    fs::create_dir_all(&root).expect("create temp root");
    let (cert, key) = write_cert(&root);
    let exe = build_project(&root, &cert, &key);

    // Start the mfb TLS server; give it a moment to bind and listen.
    let mut server = Command::new(&exe)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn mfb tls server");
    std::thread::sleep(Duration::from_millis(1000));

    // Connect as the peer, send a greeting, capture whatever the server writes.
    let mut client = Command::new("openssl")
        .args([
            "s_client",
            "-connect",
            &format!("127.0.0.1:{PORT}"),
            "-quiet",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn openssl s_client");

    // bug-386: the mfb TLS server can (intermittently, under load) stall forever
    // on a `dispatch_semaphore_wait(FOREVER)` if a Network.framework completion
    // never fires. The runtime pre-check in the read/write paths is the real fix,
    // but this test must ALSO be unable to wedge `cargo test` for 36+ minutes if a
    // regression reintroduces the stall: run the peer interaction on a worker
    // thread and bound it with a hard wall-clock deadline, killing both processes
    // and failing (not hanging) on expiry.
    let server_pid = server.id();
    let client_pid = client.id();
    let (tx, rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        client
            .stdin
            .take()
            .expect("client stdin")
            .write_all(b"hi\n")
            .expect("write greeting");
        let out = client.wait_with_output().expect("wait s_client");
        let _ = server.wait();
        let _ = tx.send(out.stdout);
    });

    let stdout = match rx.recv_timeout(STALL_DEADLINE) {
        Ok(stdout) => {
            let _ = worker.join();
            stdout
        }
        Err(_) => {
            // The server (or peer) stalled. Kill both so nothing lingers, then
            // fail fast — previously this hung the whole suite indefinitely.
            for pid in [server_pid, client_pid] {
                let _ = Command::new("kill").args(["-9", &pid.to_string()]).status();
            }
            let _ = fs::remove_dir_all(&root);
            panic!(
                "bug-386: TLS server did not respond within {}s (a FOREVER \
                 semaphore-wait stall); killed server+peer and failed instead of \
                 hanging cargo test. NOTE: this message is also what a merely \
                 CPU-STARVED run prints -- re-run this test ALONE before treating \
                 it as a TLS regression.",
                STALL_DEADLINE.as_secs()
            );
        }
    };

    assert!(
        stdout.windows(EXPECTED.len()).any(|w| w == EXPECTED),
        "peer did not receive the exact byte payload {EXPECTED:?}; got {:x?}",
        stdout
    );

    let _ = fs::remove_dir_all(&root);
}
