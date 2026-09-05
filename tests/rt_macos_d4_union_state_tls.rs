//! plan-80 D4 proof: a resource union carrying plan-74 `STATE` binds over a
//! **live** `tls::Socket` and drives real TLS I/O without corrupting the handle.
//!
//! plan-76-D Corrections D4 (the core-premise defect plan-80 exists to fix):
//! plan-74 stored the `STATE` pointer at record offset 16, which is free in the
//! File/Socket layout but held `SSL*` (openssl) / the dispatch queue (macOS
//! Network.framework) in a `tls::Socket` record. Binding a `Stream STATE …` union
//! to a `tls::Socket` therefore clobbered the live TLS handle, and the next `tls::*`
//! op dereferenced garbage — `http::read("https://…")` SIGSEGV'd (exit 139).
//!
//! plan-80 relocates `STATE` to offset 24, free in every backend layout. This
//! test stands up an mfb TLS server whose accepted client is bound into a
//! `Stream STATE PendingState` union: it default-inits the STATE, mutates it
//! (`state.sentAll`, `state.raw` appends), then reads the peer greeting and
//! writes the accumulated STATE bytes back — all via the MATCH-extracted
//! `tls::Socket` variant. If the STATE write still clobbered the macOS dispatch
//! queue (the pre-plan-80 bug), that `tls::read`/`tls::write` would crash or
//! stall; instead the peer receives the exact bytes and the union drops cleanly
//! (union cleanup closes the tls::Socket variant + frees the STATE block).
//!
//! Gated to macOS — only there does the binary embed the `macos.rs` TLS record
//! whose offset-16 `REC_QUEUE` was the collision. The openssl path (offset-16
//! `SSL*`) is proven the same way on a Linux openssl box (plan-80 Validation) and
//! by the regenerated codegen goldens. Mirrors the loopback harness of the
//! sibling `rt_macos_tls_write_capacity.rs`.

#![cfg(target_os = "macos")]

mod common;

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

/// The server accumulates these into `state.raw` (one append each) and writes
/// them back through the MATCH-extracted `tls::Socket` variant.
const EXPECTED: &[u8] = &[65, 66, 67, 68, 69]; // "ABCDE"
const PORT: u16 = 18461;

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
        "{\"name\":\"d4tls\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"executable\",\"sources\":[{\"root\":\"src\",\"role\":\"main\",\"include\":[\"**/*.mfb\"]}],\"entry\":\"main\",\"targets\":[\"native\"]}\n",
    )
    .expect("write project.json");
    // The accepted `tls::Socket` is widened into a `Stream STATE PendingState`
    // union; STATE is default-inited at the bind, then mutated, then the live
    // handle is used for real TLS I/O — the exact sequence that SIGSEGV'd
    // pre-plan-80.
    let source = format!(
        "IMPORT collections\nIMPORT tcp\nIMPORT tls\n\n\
         TYPE PendingState\n\
        \x20 raw AS List OF Byte\n\
        \x20 sentAll AS Boolean\n\
         END TYPE\n\n\
         UNION Stream\n\
        \x20 tcp::Socket\n\
        \x20 tls::Socket\n\
         END UNION\n\n\
         FUNC serveOnce(RES listener AS tls::Listener) AS Integer\n\
        \x20 RES client AS Stream STATE PendingState = tls::accept(listener)\n\
        \x20 client.state.sentAll = TRUE\n\
        \x20 client.state.raw = collections::append(client.state.raw, toByte(65))\n\
        \x20 client.state.raw = collections::append(client.state.raw, toByte(66))\n\
        \x20 client.state.raw = collections::append(client.state.raw, toByte(67))\n\
        \x20 client.state.raw = collections::append(client.state.raw, toByte(68))\n\
        \x20 client.state.raw = collections::append(client.state.raw, toByte(69))\n\
        \x20 MATCH client\n\
        \x20   CASE tls::Socket(t)\n\
        \x20     tls::write(t, client.state.raw)\n\
        \x20   CASE tcp::Socket(p)\n\
        \x20     tcp::write(p, client.state.raw)\n\
        \x20 END MATCH\n\
        \x20 RETURN 0\n\
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
fn macos_union_state_over_live_tls_socket_does_not_corrupt_the_handle() {
    if !have_openssl() {
        eprintln!("skipping: openssl CLI not available");
        return;
    }
    let root = std::env::temp_dir().join(format!("mfb_plan80_d4_{}", nonce()));
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

    // Bound the peer interaction with a hard deadline so a regression that
    // reintroduces the crash/stall fails fast instead of wedging cargo test
    // (mirrors the bug-386 guard in the sibling write-capacity test).
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

    let stdout = match rx.recv_timeout(Duration::from_secs(30)) {
        Ok(stdout) => {
            let _ = worker.join();
            stdout
        }
        Err(_) => {
            for pid in [server_pid, client_pid] {
                let _ = Command::new("kill").args(["-9", &pid.to_string()]).status();
            }
            let _ = fs::remove_dir_all(&root);
            panic!(
                "plan-80 D4: TLS server did not respond within 30s — a STATE-on-union \
                 write over a live tls::Socket corrupted the handle (queue/SSL* stall) \
                 or crashed; killed server+peer and failed instead of hanging"
            );
        }
    };

    assert!(
        stdout.windows(EXPECTED.len()).any(|w| w == EXPECTED),
        "peer did not receive the STATE bytes {EXPECTED:?}; the union's STATE write \
         over a live tls::Socket corrupted the TLS handle (plan-80 D4). got {:x?}",
        stdout
    );

    let _ = fs::remove_dir_all(&root);
}
