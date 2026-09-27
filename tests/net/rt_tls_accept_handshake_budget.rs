//! bug-736: `tls::accept(listener, timeoutMs)` bounds the wait for a connection
//! and the handshake SEPARATELY, each by the full `timeoutMs`.
//!
//! On macOS (Network.framework) `accept` used to compute one absolute deadline
//! when the call began and reuse it for the handshake. A client that connected
//! late in the wait inherited only what was left, so the server cancelled a
//! healthy handshake with `ErrTimeout` (`77050008`). Linux (OpenSSL) and Windows
//! (Schannel) never did this: they reload the raw `timeoutMs` for the handshake.
//! The fix computes a fresh deadline when the connection is taken off the
//! backlog, and `mfb man tls accept` now documents the two bounds.
//!
//! ## The shape: late connection, handshake past the first deadline
//!
//! A client that connects at 280 ms into a 300 ms accept reproduces the bug,
//! but only if its handshake takes longer than the time left, which depends on
//! the machine. This test uses wide margins instead, so the result does not
//! depend on timing:
//!
//! * the server calls `tls::accept(listener, ACCEPT_MS)` as soon as it has
//!   printed its port;
//! * the client (Python's `ssl`, so the peer is not MFB) opens the TCP
//!   connection `CONNECT_AT_MS` after it starts, well before `ACCEPT_MS`, and
//!   sends its ClientHello at `HELLO_AT_MS`, which is AFTER `ACCEPT_MS`.
//!
//! The client starts only after the server has printed its port, so its
//! ClientHello always goes out more than `ACCEPT_MS` after the accept began.
//! Under the old shared deadline that handshake cannot finish in time, so the
//! test fails on an unfixed build (measured: `late=FAILED 77050008`). Under a
//! fresh budget the handshake has until about `CONNECT_AT_MS + ACCEPT_MS`, which
//! leaves `CONNECT_AT_MS + ACCEPT_MS - HELLO_AT_MS` = 2.5 s of slack. The
//! connection itself has `ACCEPT_MS - CONNECT_AT_MS` = 1 s of slack against
//! client start-up delay. The test also asserts that the accept took longer
//! than `ACCEPT_MS`, which shows the handshake really ran past the first
//! deadline and the test is not passing without exercising the fix.
//!
//! ## `0` is still one immediate attempt, in each step
//!
//! The same program then calls `tls::accept(listener, 0)` twice. The first time
//! nothing is waiting, so the connection wait times out. The second time a raw
//! TCP connection that never sends a ClientHello is already queued, so the
//! connection is taken and the handshake step times out. Both must raise
//! `ErrTimeout` promptly; a `0` that blocked would hang on the silent peer.
//!
//! ## bug-739: the handshake bound covers the whole handshake
//!
//! On Linux and Windows the handshake bound used to be `SO_RCVTIMEO` on the
//! accepted socket, which limits each read, so a peer that sent its ClientHello
//! a few bytes at a time, each piece inside `timeoutMs`, held `accept` for as
//! long as it kept sending. `tls_accept_bounds_the_whole_handshake` sends the
//! ClientHello in pieces: quickly under a generous bound (the positive control,
//! which must succeed and which also covers a ClientHello that arrives in more
//! than one piece), then slowly under a short bound, which must raise
//! `ErrTimeout` near the bound instead of completing once the last piece
//! arrives. Measured on Linux (box 2226) against the unfixed compiler:
//! `trickle=OK` after the full trickle.
//!
//! macOS and Linux only, like `rt_tls_listener_local_address.rs`: the
//! `openssl` CLI that mints the identity is not dependable on Windows.

#![cfg(any(target_os = "macos", target_os = "linux"))]

#[path = "../common/mod.rs"]
mod common;

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

/// The server's accept bound for the late-client case, in milliseconds.
const ACCEPT_MS: u64 = 4000;
/// When the client opens its TCP connection, measured from its own start.
const CONNECT_AT_MS: u64 = 3000;
/// When the client sends its ClientHello. It must be later than `ACCEPT_MS`.
const HELLO_AT_MS: u64 = 4500;
/// What the server writes once the handshake is done. The client must read it.
const PAYLOAD: &str = "handshake-budget-ok";
/// `ErrTimeout`.
const ERR_TIMEOUT: &str = "77050008";
/// Upper bound on a `timeoutMs = 0` accept. That call should return within a
/// few milliseconds; this only separates "returned" from "blocked".
const ZERO_BOUND_MS: u64 = 5000;
/// A limit on each blocking step, so a hang fails with a named message.
/// It is not a performance check. It matches the 120 s the siblings use.
const DEADLINE: Duration = Duration::from_secs(120);

const CLIENT: &str = r#"
import socket, ssl, sys, time
port = int(sys.argv[1])
connect_at = float(sys.argv[2]) / 1000.0
hello_at = float(sys.argv[3]) / 1000.0
t0 = time.monotonic()
ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
ctx.check_hostname = False
ctx.verify_mode = ssl.CERT_NONE
time.sleep(max(0.0, t0 + connect_at - time.monotonic()))
raw = socket.create_connection(("127.0.0.1", port), timeout=60)
time.sleep(max(0.0, t0 + hello_at - time.monotonic()))
try:
    s = ctx.wrap_socket(raw, server_hostname="127.0.0.1")
    data = b""
    while not data.endswith(b"\n"):
        chunk = s.recv(256)
        if not chunk:
            break
        data += chunk
    print("client=GOT " + data.decode(errors="replace").strip(), flush=True)
except Exception as e:
    print("client=FAILED %s: %s" % (type(e).__name__, e), flush=True)
"#;

/// bug-739 client: send the ClientHello in `pieces` pieces of `chunk` bytes,
/// `step_ms` apart, then the rest, then finish the handshake and read a line.
const TRICKLE_CLIENT: &str = r#"
import socket, ssl, sys, time
port, step, pieces, chunk = int(sys.argv[1]), float(sys.argv[2]) / 1000.0, int(sys.argv[3]), int(sys.argv[4])
ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
ctx.check_hostname = False
ctx.verify_mode = ssl.CERT_NONE
raw = socket.create_connection(("127.0.0.1", port), timeout=60)
inc, out = ssl.MemoryBIO(), ssl.MemoryBIO()
obj = ctx.wrap_bio(inc, out, server_hostname="127.0.0.1")
try:
    try:
        obj.do_handshake()
    except ssl.SSLWantReadError:
        pass
    hello = out.read()
    for i in range(pieces):
        raw.sendall(hello[i * chunk:(i + 1) * chunk])
        time.sleep(step)
    raw.sendall(hello[pieces * chunk:])
    while True:
        try:
            obj.do_handshake()
            break
        except ssl.SSLWantReadError:
            pending = out.read()
            if pending:
                raw.sendall(pending)
            data = raw.recv(65536)
            if not data:
                raise Exception("server closed the connection")
            inc.write(data)
    pending = out.read()
    if pending:
        raw.sendall(pending)
    data = b""
    while not data.endswith(b"\n"):
        try:
            data += obj.read(256)
        except ssl.SSLWantReadError:
            got = raw.recv(65536)
            if not got:
                break
            inc.write(got)
    print("client=GOT " + data.decode(errors="replace").strip(), flush=True)
except Exception as e:
    print("client=FAILED %s: %s" % (type(e).__name__, e), flush=True)
"#;
/// The generous bound for the fast, fragmented control handshake.
const CONTROL_ACCEPT_MS: u64 = 20_000;
/// The short bound the slow trickle must be held to.
const TRICKLE_ACCEPT_MS: u64 = 1500;
/// The slow trickle: 20 pieces of 10 bytes, 400 ms apart, so it needs about
/// 8 s, more than five times `TRICKLE_ACCEPT_MS`, and every gap is well inside it.
const TRICKLE_STEP_MS: u64 = 400;
/// An expiry must land near `TRICKLE_ACCEPT_MS`; the trickle alone takes
/// about 8 s, so anything under this is the bound, not the peer finishing.
const TRICKLE_BOUND_MS: u64 = 6000;

fn have_tools() -> bool {
    let ok = |cmd: &str, arg: &str| {
        Command::new(cmd)
            .arg(arg)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    };
    ok("openssl", "version") && ok(common::python_exe(), "--version")
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
        "IMPORT datetime\nIMPORT io\nIMPORT net\nIMPORT tls\n\n\
         FUNC nowMs AS Integer\n\
        \x20 RETURN datetime::monotonicNanos() / 1000000\n\
         END FUNC\n\n\
         FUNC timedAccept(RES listener AS tls::Listener, timeoutMs AS Integer, started AS Integer) AS String\n\
        \x20 RES conn = tls::accept(listener, timeoutMs)\n\
        \x20 LET took AS Integer = nowMs() - started\n\
        \x20 tls::write(conn, \"{PAYLOAD}\" & toString([toByte(10)]))\n\
        \x20 tls::close(conn)\n\
        \x20 RETURN \"OK \" & toString(took)\n\
        \x20 TRAP(err)\n\
        \x20   RETURN \"FAILED \" & toString(err.code) & \" \" & toString(nowMs() - started)\n\
        \x20 END TRAP\n\
         END FUNC\n\n\
         FUNC main AS Integer\n\
        \x20 RES listener = tls::listen(\"127.0.0.1\", 0, \"{cert}\", \"{key}\")\n\
        \x20 LET at AS net::Address = tls::localAddress(listener)\n\
        \x20 io::print(\"port=\" & toString(at.port))\n\
        \x20 io::print(\"late=\" & timedAccept(listener, {ACCEPT_MS}, nowMs()))\n\
        \x20 io::print(\"zero-empty=\" & timedAccept(listener, 0, nowMs()))\n\
        \x20 LET queued AS String = io::input()\n\
        \x20 io::print(\"zero-silent=\" & timedAccept(listener, 0, nowMs()))\n\
        \x20 tls::close(listener)\n\
        \x20 RETURN 0\n\
         END FUNC\n",
        cert = common::mfb_path_literal(cert),
        key = common::mfb_path_literal(key),
    );
    let project = root.join("server");
    fs::create_dir_all(project.join("src")).expect("create src dir");
    fs::write(
        project.join("project.json"),
        "{\"name\":\"tlsacceptbudget\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"executable\",\"sources\":[{\"root\":\"src\",\"role\":\"main\",\"include\":[\"**/*.mfb\"]}],\"entry\":\"main\",\"targets\":[\"native\"]}\n",
    )
    .expect("write project.json");
    fs::write(project.join("src/main.mfb"), source).expect("write source");
    common::build_project(&project)
}

fn build_trickle_server(root: &Path, cert: &Path, key: &Path) -> PathBuf {
    let source = format!(
        "IMPORT datetime\nIMPORT io\nIMPORT net\nIMPORT tls\n\n\
         FUNC nowMs AS Integer\n\
        \x20 RETURN datetime::monotonicNanos() / 1000000\n\
         END FUNC\n\n\
         FUNC timedAccept(RES listener AS tls::Listener, timeoutMs AS Integer, started AS Integer) AS String\n\
        \x20 RES conn = tls::accept(listener, timeoutMs)\n\
        \x20 LET took AS Integer = nowMs() - started\n\
        \x20 tls::write(conn, \"{PAYLOAD}\" & toString([toByte(10)]))\n\
        \x20 tls::close(conn)\n\
        \x20 RETURN \"OK \" & toString(took)\n\
        \x20 TRAP(err)\n\
        \x20   RETURN \"FAILED \" & toString(err.code) & \" \" & toString(nowMs() - started)\n\
        \x20 END TRAP\n\
         END FUNC\n\n\
         FUNC main AS Integer\n\
        \x20 RES listener = tls::listen(\"127.0.0.1\", 0, \"{cert}\", \"{key}\")\n\
        \x20 LET at AS net::Address = tls::localAddress(listener)\n\
        \x20 io::print(\"port=\" & toString(at.port))\n\
        \x20 io::print(\"control=\" & timedAccept(listener, {CONTROL_ACCEPT_MS}, nowMs()))\n\
        \x20 io::print(\"trickle=\" & timedAccept(listener, {TRICKLE_ACCEPT_MS}, nowMs()))\n\
        \x20 tls::close(listener)\n\
        \x20 RETURN 0\n\
         END FUNC\n",
        cert = common::mfb_path_literal(cert),
        key = common::mfb_path_literal(key),
    );
    let project = root.join("trickle");
    fs::create_dir_all(project.join("src")).expect("create src dir");
    fs::write(
        project.join("project.json"),
        "{\"name\":\"tlsaccepttrickle\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\"kind\":\"executable\",\"sources\":[{\"root\":\"src\",\"role\":\"main\",\"include\":[\"**/*.mfb\"]}],\"entry\":\"main\",\"targets\":[\"native\"]}\n",
    )
    .expect("write project.json");
    fs::write(project.join("src/main.mfb"), source).expect("write source");
    common::build_project(&project)
}

/// Run the trickle client to completion and return its one result line.
fn run_trickle_client(pids: &[u32], root: &Path, port: u16, step_ms: u64) -> String {
    let client = Command::new(common::python_exe())
        .args([
            "-c",
            TRICKLE_CLIENT,
            &port.to_string(),
            &step_ms.to_string(),
            "20",
            "10",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn python TLS client");
    let client_pid = client.id();
    let (tx, done) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(client.wait_with_output());
    });
    let out = done
        .recv_timeout(DEADLINE)
        .unwrap_or_else(|_| {
            let mut all = pids.to_vec();
            all.push(client_pid);
            fail(&all, root, "the python client did not exit".to_string())
        })
        .expect("wait for python client");
    let mut said = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if said.is_empty() {
        said = String::from_utf8_lossy(&out.stderr).trim().to_string();
    }
    said
}

fn kill(pid: u32) {
    let _ = Command::new("kill").args(["-9", &pid.to_string()]).status();
}

/// Kill whatever is still running and fail with `why`.
fn fail(pids: &[u32], root: &Path, why: String) -> ! {
    for pid in pids {
        kill(*pid);
    }
    let _ = fs::remove_dir_all(root);
    panic!(
        "{why}\nNOTE: a CPU-starved run can print the same timeout; re-run this test ALONE \
         before treating a timeout as a TLS regression."
    );
}

/// Parse `OK <ms>` / `FAILED <code> <ms>` into (outcome words, elapsed ms).
fn outcome(value: &str) -> (Vec<&str>, u64) {
    let words: Vec<&str> = value.split_whitespace().collect();
    let ms = words
        .last()
        .and_then(|w| w.parse::<u64>().ok())
        .unwrap_or_else(|| panic!("no elapsed-ms field in {value:?}"));
    (words[..words.len() - 1].to_vec(), ms)
}

#[test]
fn tls_accept_gives_a_late_connection_a_full_handshake_budget() {
    if !have_tools() {
        eprintln!("skipping: the openssl CLI or python3 is not available");
        return;
    }
    let root = std::env::temp_dir().join(format!("mfb_bug736_{}", common::unique_nonce()));
    fs::create_dir_all(&root).expect("create temp root");
    let (cert, key) = write_cert(&root);
    let exe = build_server(&root, &cert, &key);

    let mut server = Command::new(&exe)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn mfb tls server");
    let server_pid = server.id();
    let mut server_stdin = server.stdin.take().expect("server stdin");
    let (tx, lines) = mpsc::channel::<String>();
    let stdout = server.stdout.take().expect("server stdout");
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    let next = |pids: &[u32], prefix: &str| -> String {
        let line = lines.recv_timeout(DEADLINE).unwrap_or_else(|_| {
            fail(
                pids,
                &root,
                format!("server printed nothing for `{prefix}`"),
            )
        });
        line.strip_prefix(prefix)
            .map(str::to_string)
            .unwrap_or_else(|| fail(pids, &root, format!("expected `{prefix}…`, got {line:?}")))
    };

    let port: u16 = {
        let value = next(&[server_pid], "port=");
        value
            .parse()
            .unwrap_or_else(|_| fail(&[server_pid], &root, format!("bad port {value:?}")))
    };

    // 1. The late client. The server is already inside `tls::accept`.
    let client = Command::new(common::python_exe())
        .args([
            "-c",
            CLIENT,
            &port.to_string(),
            &CONNECT_AT_MS.to_string(),
            &HELLO_AT_MS.to_string(),
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn python TLS client");
    let client_pid = client.id();
    let late = next(&[server_pid, client_pid], "late=");
    let (client_tx, client_done) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = client_tx.send(client.wait_with_output());
    });
    let client_out = client_done
        .recv_timeout(DEADLINE)
        .unwrap_or_else(|_| {
            fail(
                &[server_pid, client_pid],
                &root,
                "the python client did not exit".to_string(),
            )
        })
        .expect("wait for python client");
    let client_said = String::from_utf8_lossy(&client_out.stdout)
        .trim()
        .to_string();

    // 2. `0` with nothing queued: the connection wait gives up at once.
    let zero_empty = next(&[server_pid], "zero-empty=");

    // 3. `0` with a silent connection queued: the handshake gives up at once.
    //    Keep the stream open until the server has answered, so a blocking
    //    handshake would hang here instead of failing on our close.
    let silent = TcpStream::connect(("127.0.0.1", port))
        .unwrap_or_else(|e| fail(&[server_pid], &root, format!("raw connect failed: {e}")));
    // Give the listener time to queue the connection before the server looks.
    std::thread::sleep(Duration::from_millis(500));
    server_stdin
        .write_all(b"queued\n")
        .expect("release the server's third accept");
    drop(server_stdin);
    let zero_silent = next(&[server_pid], "zero-silent=");
    drop(silent);

    let (done_tx, done) = mpsc::channel();
    std::thread::spawn(move || {
        let status = server.wait();
        let mut err = String::new();
        if let Some(mut e) = server.stderr.take() {
            let _ = e.read_to_string(&mut err);
        }
        let _ = done_tx.send((status, err));
    });
    let (status, server_err) = done
        .recv_timeout(DEADLINE)
        .unwrap_or_else(|_| fail(&[server_pid], &root, "server did not exit".to_string()));
    let _ = fs::remove_dir_all(&root);

    let (late_words, late_ms) = outcome(&late);
    assert_eq!(
        late_words,
        ["OK"],
        "a client that connects before the accept deadline must get a full handshake budget; \
         the server reported {late:?} (FAILED {ERR_TIMEOUT} = the handshake was cut off by the \
         deadline the connection wait had used up), client said {client_said:?}\n\
         server stderr:\n{server_err}"
    );
    assert!(
        late_ms > ACCEPT_MS,
        "the late accept took {late_ms} ms, which is not more than {ACCEPT_MS} ms, so the \
         handshake did not run past the first deadline and this run did not test the fix"
    );
    assert_eq!(
        client_said,
        format!("client=GOT {PAYLOAD}"),
        "the client must finish the handshake and read the server's payload"
    );

    for (name, value) in [("zero-empty", &zero_empty), ("zero-silent", &zero_silent)] {
        let (words, ms) = outcome(value);
        assert_eq!(
            words,
            ["FAILED", ERR_TIMEOUT],
            "`tls::accept(listener, 0)` ({name}) must raise ErrTimeout, got {value:?}"
        );
        assert!(
            ms < ZERO_BOUND_MS,
            "`tls::accept(listener, 0)` ({name}) took {ms} ms; `0` is one immediate attempt"
        );
    }

    let status = status.expect("server wait");
    assert!(
        status.success(),
        "the server must exit 0: {}\nstderr:\n{server_err}",
        common::exit_description(&status)
    );
}

#[test]
fn tls_accept_bounds_the_whole_handshake() {
    if !have_tools() {
        eprintln!("skipping: the openssl CLI or python3 is not available");
        return;
    }
    let root = std::env::temp_dir().join(format!("mfb_bug739_{}", common::unique_nonce()));
    fs::create_dir_all(&root).expect("create temp root");
    let (cert, key) = write_cert(&root);
    let exe = build_trickle_server(&root, &cert, &key);

    let mut server = Command::new(&exe)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn mfb tls server");
    let server_pid = server.id();
    let (tx, lines) = mpsc::channel::<String>();
    let stdout = server.stdout.take().expect("server stdout");
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    let next = |prefix: &str| -> String {
        let line = lines.recv_timeout(DEADLINE).unwrap_or_else(|_| {
            fail(
                &[server_pid],
                &root,
                format!("server printed nothing for `{prefix}`"),
            )
        });
        line.strip_prefix(prefix)
            .map(str::to_string)
            .unwrap_or_else(|| {
                fail(
                    &[server_pid],
                    &root,
                    format!("expected `{prefix}…`, got {line:?}"),
                )
            })
    };
    let port: u16 = {
        let value = next("port=");
        value
            .parse()
            .unwrap_or_else(|_| fail(&[server_pid], &root, format!("bad port {value:?}")))
    };

    // 1. Positive control: the ClientHello arrives in 20 pieces 50 ms apart,
    //    under a generous bound. It must succeed.
    let control_client = run_trickle_client(&[server_pid], &root, port, 50);
    let control = next("control=");
    // 2. The slow trickle under a short bound.
    let trickle_client = run_trickle_client(&[server_pid], &root, port, TRICKLE_STEP_MS);
    let trickle = next("trickle=");

    let (done_tx, done) = mpsc::channel();
    std::thread::spawn(move || {
        let status = server.wait();
        let mut err = String::new();
        if let Some(mut e) = server.stderr.take() {
            let _ = e.read_to_string(&mut err);
        }
        let _ = done_tx.send((status, err));
    });
    let (status, server_err) = done
        .recv_timeout(DEADLINE)
        .unwrap_or_else(|_| fail(&[server_pid], &root, "server did not exit".to_string()));
    let _ = fs::remove_dir_all(&root);

    let (control_words, _) = outcome(&control);
    assert_eq!(
        control_words,
        ["OK"],
        "a ClientHello that arrives in pieces, quickly, must complete: server {control:?}, \
         client {control_client:?}\nserver stderr:\n{server_err}"
    );
    assert_eq!(control_client, format!("client=GOT {PAYLOAD}"));

    let (trickle_words, trickle_ms) = outcome(&trickle);
    assert_eq!(
        trickle_words,
        ["FAILED", ERR_TIMEOUT],
        "a peer that trickles its ClientHello must not outlast the {TRICKLE_ACCEPT_MS} ms \
         handshake bound; the server reported {trickle:?} (OK = each piece reset a per-read \
         timeout), client said {trickle_client:?}"
    );
    assert!(
        trickle_ms < TRICKLE_BOUND_MS,
        "the trickle was cut off after {trickle_ms} ms; the bound is {TRICKLE_ACCEPT_MS} ms"
    );
    let status = status.expect("server wait");
    assert!(
        status.success(),
        "the server must exit 0: {}\nstderr:\n{server_err}",
        common::exit_description(&status)
    );
}
