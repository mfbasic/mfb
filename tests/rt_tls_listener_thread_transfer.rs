//! bug-464: a `tls::Listener` transferred to another thread completes
//! `tls::accept` there, against a real foreign TLS client.
//!
//! `tcp::Listener`, `tls::Socket` and `tls::Listener` were all refused on a
//! thread plane with `2-203-0063 TYPE_THREAD_NOT_SENDABLE`. The first two are
//! proven by acceptance fixtures
//! (`tests/rt-behavior/threads/thread-transfer-tcp-listener-rt`,
//! `.../thread-transfer-tls-socket-rt`). This one cannot be: an MFBASIC TLS
//! *server* needs a certificate/key pair, and a static golden fixture cannot
//! carry one (it would expire, and committing a private key is its own problem).
//! So the identity is generated here at run time, exactly as the sibling
//! `rt_tls_listener_local_address.rs` and `rt_macos_tls_write_capacity.rs` do.
//!
//! ## What it actually proves
//!
//! The listener is bound, and its identity loaded, on the MAIN thread; it is
//! then moved with `thread::transfer` and every part of serving — the handshake
//! and the write — happens on the worker. A `tls::Listener`'s server context
//! lives in the record tail past the canonical header (`SSL_CTX*`@32 on OpenSSL,
//! an arena SSPI WORK block@40 on Schannel, ctx@32 / queue@40 / bound-host@48 on
//! Network.framework), and the thread-transfer copy used to zero that whole
//! region. A listener that arrived with a zeroed context could not complete a
//! handshake, so requiring the payload to come back through `openssl s_client`
//! is what distinguishes a carried context from a truncated one.
//!
//! The peer is `openssl s_client` rather than another MFBASIC program on
//! purpose: a proof where our client and our server agree with each other cannot
//! tell a working TLS server from two matching bugs.
//!
//! macOS and Linux only, matching `rt_tls_listener_local_address.rs`: both run a
//! real TLS server (Network.framework and OpenSSL), and Windows is excluded
//! because the `openssl` CLI this uses as the peer is not a dependable presence
//! there.

#![cfg(any(target_os = "macos", target_os = "linux"))]

mod common;

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

/// What the worker writes to the accepted peer. Receiving it back through a real
/// handshake driven entirely on the receiving thread is the proof.
const PAYLOAD: &str = "transferred-listener-ok";

/// Wall-clock bound on each blocking interaction. Generous on purpose — these
/// bounds turn a *hang* into a named failure so a stalled TLS server cannot
/// wedge `cargo test`; they are not performance assertions. See the sibling
/// test's note on a starved 30s bound passing in 67s when re-run alone.
/// The worker's own `tls::accept` bound is 90_000 ms, set in the committed
/// package source, and deliberately under this so a server that gives up reports
/// `ErrTimeout` through the program rather than tripping the harness's outer kill.
const DEADLINE: Duration = Duration::from_secs(120);

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

/// The worker package that receives the listener. Its source lives in
/// `tools/thread-package-sources/xfer_tls_listener_worker` and its `.mfp` is
/// committed beside it (rebuilt by `scripts/sync-package-mfp.sh`), so this test
/// copies the artifact rather than compiling a package itself.
fn worker_mfp() -> PathBuf {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
    repo.join("tools/thread-package-sources/xfer_tls_listener_worker/xfer_tls_listener_worker.mfp")
}

/// Bind and load the identity on the main thread, transfer the listener, and let
/// the worker do all the serving.
fn build_project(root: &Path, cert: &Path, key: &Path) -> PathBuf {
    fs::create_dir_all(root.join("src")).expect("create src dir");
    fs::create_dir_all(root.join("packages")).expect("create packages dir");
    fs::copy(
        worker_mfp(),
        root.join("packages/xfer_tls_listener_worker.mfp"),
    )
    .expect("copy worker .mfp");
    fs::write(
        root.join("project.json"),
        "{\"name\":\"tlsxfer\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"executable\",\
         \"sources\":[{\"root\":\"src\",\"role\":\"main\",\"include\":[\"**/*.mfb\"]}],\
         \"packages\":[{\"name\":\"xfer_tls_listener_worker\",\"version\":\"=0.1.0\",\
         \"source\":\"file:packages/xfer_tls_listener_worker.mfp\"}],\
         \"entry\":\"main\",\"targets\":[\"native\"]}\n",
    )
    .expect("write project.json");
    let source = format!(
        "IMPORT io\nIMPORT net\nIMPORT tls\nIMPORT thread\nIMPORT xfer_tls_listener_worker\n\n\
         FUNC main AS Integer\n\
        \x20 RES server = tls::listen(\"127.0.0.1\", 0, \"{cert}\", \"{key}\")\n\
        \x20 LET bound = tls::localAddress(server)\n\
        \x20 io::print(\"bound \" & bound.host & \" \" & toString(bound.port))\n\
        \x20 LET t AS Thread OF RES tls::Listener TO Integer = \
             thread::start(xfer_tls_listener_worker::serveOnTransferredTlsListener, \"{PAYLOAD}\")\n\
        \x20 thread::transfer(t, server)\n\
        \x20 io::print(\"served \" & toString(thread::waitFor(t)))\n\
        \x20 RETURN 0\n\
         END FUNC\n",
        cert = cert.display(),
        key = key.display(),
        PAYLOAD = PAYLOAD,
    );
    fs::write(root.join("src/main.mfb"), source).expect("write source");

    let output = Command::new(common::mfb_exe())
        .arg("build")
        .arg(root)
        .output()
        .expect("run mfb build");
    assert!(
        output.status.success(),
        "bug-464: a program transferring a tls::Listener to a thread must build:\nstdout:\n{}\nstderr:\n{}",
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
fn read_bound_line(server: &mut Child) -> u16 {
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
    let _host = parts.next();
    parts
        .next()
        .unwrap_or_default()
        .parse::<u16>()
        .unwrap_or_else(|_| panic!("could not parse a port out of {line:?}"))
}

#[test]
fn a_transferred_tls_listener_accepts_on_the_receiving_thread() {
    if !have_openssl() {
        eprintln!("skipping: openssl CLI not available");
        return;
    }
    let root = std::env::temp_dir().join(format!("mfb_bug464_{}", nonce()));
    fs::create_dir_all(&root).expect("create temp root");
    let (cert, key) = write_cert(&root);
    let exe = build_project(&root, &cert, &key);

    let mut server = Command::new(&exe)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn mfb tls server");
    let port = read_bound_line(&mut server);
    assert_ne!(port, 0, "the listener did not report a real bound port");

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
        // Close s_client's stdin and send NOTHING through it — see the same
        // sequence in `rt_tls_listener_local_address` for the measurement. In
        // short: `-quiet` implies `-ign_eof`, so the close alone makes the client
        // exit when the server does, and a byte written here would sit unread in
        // the server's receive queue and turn its close into an RST that discards
        // the payload.
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
                "the transferred listener did not serve port {port} within {}s — \
                 a listener whose server context did not survive the move cannot \
                 complete a handshake",
                DEADLINE.as_secs()
            );
        }
    };

    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains(PAYLOAD),
        "the transferred tls::Listener completed no TLS exchange on the receiving \
         thread; s_client exited {:?} and saw {text:?}\ns_client stderr:\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );

    let _ = fs::remove_dir_all(&root);
}
