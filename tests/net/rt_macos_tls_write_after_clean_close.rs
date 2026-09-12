//! bug-564: the POSITIVE pins around the store-release fix in the macOS
//! Network.framework trampolines.
//!
//! bug-564 made the state and send handlers publish the error domain before the
//! gate that tells `tls::write` to read it, with a store-release (`stlr`) for the
//! gate and a load-acquire (`ldar`) for the writer's gate load. A change like
//! that can go wrong in two ways that the departed-peer fixture
//! (`rt-behavior/tls/tls-write-peer-closed-raises-rt`, a peer killed outright)
//! cannot see:
//!
//! * a write to a LIVE peer must still succeed, and the peer must receive the
//!   bytes. A gate that reads as terminal too early would fail it.
//! * a write after a CLEAN close must raise `ErrConnectionClosed`. Here the peer
//!   sends `close_notify` and exits, which is the ordinary way a TLS session
//!   ends. `mfb spec stdlib transports` (§17) and `mfb man tls write` require
//!   `ErrConnectionClosed` for a write to a peer that has gone away, and
//!   `ErrTlsFailed` is reserved for handshake, certificate and protocol failures.
//!
//! The peer is `openssl s_client` with `-msg`, and the test asserts that its
//! trace records the `close_notify` alert. So "clean close" is measured, not
//! assumed. As in `rt_macos_tls_write_capacity.rs`, the identity is minted at
//! run time and the server announces the port it bound.
//!
//! Gated to macOS: the trampolines under test exist only in the
//! Network.framework backend.

#![cfg(target_os = "macos")]

#[path = "../common/mod.rs"]
mod common;

use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

/// What the server writes to the live peer.
const PAYLOAD: &str = "live-write-ok";

/// Wall-clock bound on each blocking step. It turns a hang into a named failure
/// and is not a performance assertion. It matches the siblings' 120 s, which
/// was set after a starved 30 s bound failed a run that passed in 67 s alone.
const DEADLINE: Duration = Duration::from_secs(120);

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

fn build_server(root: &Path, cert: &Path, key: &Path) -> PathBuf {
    let source = format!(
        "IMPORT errorCode\nIMPORT io\nIMPORT net\nIMPORT strings\nIMPORT tls\n\n\
         FUNC liveWrite(RES conn AS tls::Socket) AS String\n\
        \x20 tls::write(conn, \"{PAYLOAD}\" & toString([toByte(10)]))\n\
        \x20 RETURN \"live=OK\"\n\
        \x20 TRAP(err)\n\
        \x20   RETURN \"live=RAISED \" & toString(err.code)\n\
        \x20 END TRAP\n\
         END FUNC\n\n\
         FUNC readUntilClosed(RES conn AS tls::Socket) AS String\n\
        \x20 WHILE TRUE\n\
        \x20   LET chunk AS List OF Byte = tls::read(conn, 4096)\n\
        \x20 END WHILE\n\
        \x20 RETURN \"eof=NONE\"\n\
        \x20 TRAP(err)\n\
        \x20   RETURN \"eof=\" & toString(err.code = errorCode::ErrConnectionClosed)\n\
        \x20 END TRAP\n\
         END FUNC\n\n\
         FUNC writeAfterClose(RES conn AS tls::Socket) AS String\n\
        \x20 LET chunk AS String = strings::repeat(\"x\", 65536)\n\
        \x20 FOR i = 1 TO 200\n\
        \x20   tls::write(conn, chunk)\n\
        \x20 NEXT\n\
        \x20 RETURN \"after=COMPLETED\"\n\
        \x20 TRAP(err)\n\
        \x20   RETURN \"after=\" & toString(err.code = errorCode::ErrConnectionClosed) & \" code=\" & toString(err.code)\n\
        \x20 END TRAP\n\
         END FUNC\n\n\
         FUNC main AS Integer\n\
        \x20 RES s = tls::listen(\"127.0.0.1\", 0, \"{cert}\", \"{key}\")\n\
        \x20 LET at AS net::Address = tls::localAddress(s)\n\
        \x20 io::print(\"port=\" & toString(at.port))\n\
        \x20 RES conn = tls::accept(s)\n\
        \x20 io::print(liveWrite(conn))\n\
        \x20 io::print(readUntilClosed(conn))\n\
        \x20 io::print(writeAfterClose(conn))\n\
        \x20 RETURN 0\n\
         END FUNC\n",
        cert = common::mfb_path_literal(cert),
        key = common::mfb_path_literal(key),
    );
    let project = root.join("server");
    fs::create_dir_all(project.join("src")).expect("create src dir");
    fs::write(
        project.join("project.json"),
        "{\"name\":\"tlscleanclose\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"executable\",\"sources\":[{\"root\":\"src\",\"role\":\"main\",\"include\":[\"**/*.mfb\"]}],\"entry\":\"main\",\"targets\":[\"native\"]}\n",
    )
    .expect("write project.json");
    fs::write(project.join("src/main.mfb"), source).expect("write source");
    common::build_project(&project)
}

/// Kill whatever is still running and fail with `why`.
fn fail(pids: &[u32], root: &Path, why: String) -> ! {
    for pid in pids {
        let _ = Command::new("kill").args(["-9", &pid.to_string()]).status();
    }
    let _ = fs::remove_dir_all(root);
    panic!(
        "{why}\nNOTE: a CPU-starved run prints the same timeout; re-run this test ALONE \
         before treating a timeout as a TLS regression."
    );
}

#[test]
fn macos_tls_write_succeeds_live_and_raises_connection_closed_after_close_notify() {
    if !have_openssl() {
        eprintln!("skipping: openssl CLI not available");
        return;
    }
    let root = std::env::temp_dir().join(format!("mfb_bug564_{}", common::unique_nonce()));
    fs::create_dir_all(&root).expect("create temp root");
    let (cert, key) = write_cert(&root);
    let exe = build_server(&root, &cert, &key);

    let mut server = Command::new(&exe)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn mfb tls server");
    let server_pid = server.id();
    let (server_tx, server_lines) = mpsc::channel::<String>();
    let server_stdout = server.stdout.take().expect("server stdout");
    std::thread::spawn(move || {
        for line in BufReader::new(server_stdout).lines().map_while(Result::ok) {
            if server_tx.send(line).is_err() {
                break;
            }
        }
    });
    let next_server_line = |pids: &[u32], step: &str| -> String {
        server_lines
            .recv_timeout(DEADLINE)
            .unwrap_or_else(|_| fail(pids, &root, format!("server printed nothing for `{step}`")))
    };

    let port: u16 = {
        let line = next_server_line(&[server_pid], "port");
        line.strip_prefix("port=")
            .and_then(|p| p.parse().ok())
            .unwrap_or_else(|| {
                fail(
                    &[server_pid],
                    &root,
                    format!("expected `port=<n>`, got {line:?}"),
                )
            })
    };

    // Not `-quiet`: that implies `-ign_eof`, and the stdin EOF below is what
    // makes s_client shut the session down with close_notify.
    let mut peer = Command::new("openssl")
        .args(["s_client", "-msg", "-connect", &format!("127.0.0.1:{port}")])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn openssl s_client");
    let peer_pid = peer.id();
    let pids = [server_pid, peer_pid];
    let peer_stdin = peer.stdin.take().expect("peer stdin");
    let (peer_tx, peer_chunks) = mpsc::channel::<Vec<u8>>();
    let mut peer_stdout = peer.stdout.take().expect("peer stdout");
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while let Ok(n) = peer_stdout.read(&mut buf) {
            if n == 0 || peer_tx.send(buf[..n].to_vec()).is_err() {
                break;
            }
        }
    });

    // 1. A write to a live peer succeeds, and the peer receives it.
    let live = next_server_line(&pids, "live write");
    assert_eq!(live, "live=OK", "a write to a live peer must succeed");
    let mut transcript: Vec<u8> = Vec::new();
    let payload = format!("{PAYLOAD}\n").into_bytes();
    while !transcript
        .windows(payload.len())
        .any(|w| w == payload.as_slice())
    {
        match peer_chunks.recv_timeout(DEADLINE) {
            Ok(chunk) => transcript.extend(chunk),
            Err(_) => fail(
                &pids,
                &root,
                format!(
                    "the live peer never received {PAYLOAD:?}; its output so far:\n{}",
                    String::from_utf8_lossy(&transcript)
                ),
            ),
        }
    }

    // 2. Close the peer cleanly: EOF on s_client's stdin makes it send
    //    close_notify and exit.
    drop(peer_stdin);
    let eof = next_server_line(&pids, "read until closed");
    assert_eq!(
        eof, "eof=TRUE",
        "the server must observe the clean close as ErrConnectionClosed on read"
    );
    let after = next_server_line(&pids, "write after close");

    let (done_tx, done) = mpsc::channel();
    std::thread::spawn(move || {
        let status = peer.wait();
        let server_status = server.wait();
        let mut server_err = String::new();
        if let Some(mut e) = server.stderr.take() {
            let _ = e.read_to_string(&mut server_err);
        }
        let _ = done_tx.send((status, server_status, server_err));
    });
    let (peer_status, server_status, server_err) = done
        .recv_timeout(DEADLINE)
        .unwrap_or_else(|_| fail(&pids, &root, "server or peer did not exit".to_string()));
    while let Ok(chunk) = peer_chunks.recv_timeout(Duration::from_millis(200)) {
        transcript.extend(chunk);
    }
    let transcript = String::from_utf8_lossy(&transcript).into_owned();
    let _ = fs::remove_dir_all(&root);

    assert!(
        transcript.contains("close_notify") || transcript.contains("close notify"),
        "the peer's -msg trace must show it sent close_notify, or this is not a clean close:\n{transcript}"
    );
    assert!(peer_status.is_ok(), "s_client exits: {peer_status:?}");
    assert!(
        after.starts_with("after=TRUE "),
        "a write after the peer's close_notify must raise ErrConnectionClosed \
         (`after=COMPLETED` means it never raised; `after=FALSE code=<c>` names what it \
         raised instead), got {after:?}\nserver stderr:\n{server_err}"
    );
    let server_status = server_status.expect("server wait");
    assert!(
        server_status.success(),
        "the server must exit 0: {}\nstderr:\n{server_err}",
        common::exit_description(&server_status)
    );
}
