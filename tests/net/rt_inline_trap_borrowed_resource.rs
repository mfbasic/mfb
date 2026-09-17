//! bug-648: an inline `TRAP` took ownership of a resource it was only lent.
//!
//! `tcp::poll`, `udp::poll` and `tls::poll` over a `List OF RES …`, and
//! `collections::get` of one, return a **borrowed** pointer to an element the list
//! still owns and closes (§15.6). Without a `TRAP` the bind is an alias and closes
//! nothing. With an inline `TRAP` the desugar routes the value through a
//! compiler temp,
//!
//! ```text
//! bind $trap_resN = callResult tcp.poll(socks, …)
//! bind MUT $trap_valN : tcp.Socket            ← no initializer
//! if resultIsOk($trap_resN) { $trap_valN = resultValue($trap_resN) }
//! else                      { …handler…; $trap_valN = <RECOVER value> }
//! bind ready = $trap_valN
//! ```
//!
//! and the resource-typed `$trap_valN` registered a close obligation on whatever
//! it was assigned. At its scope exit it closed the list's live element, and
//! freed its record too (`frees_record` is set for the net handles), so the next
//! loop iteration's `poll` found a closed socket. `collections::get` of a
//! sendable handle was worse: the `Result` wrap deep-copied the element through
//! the thread hand-over lowering, which tombstones its source `moved|closed` on
//! the spot, and the list's own drop then reported a cleanup failure.
//!
//! The same temp also took ownership of a `RECOVER` value that only NAMES a live
//! resource (`RECOVER outer`), closing `outer` while its own binding still held
//! it. Ownership is a property of each assignment, not of the temp: a trap whose
//! success value is borrowed can recover an owned one, and the reverse. The
//! `mixed_*` cases pin both directions, under a low descriptor limit so that
//! "never own anything" (a leak) fails as loudly as "own everything" (a premature
//! close).
//!
//! macOS and Linux only: the descriptor limit is set through `sh`'s `ulimit`, and
//! the TLS case needs the `openssl` CLI as its peer.

#![cfg(any(target_os = "macos", target_os = "linux"))]

#[path = "../common/mod.rs"]
mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::{build_project, mfb_path_literal, temp_project};

/// Low enough that 100 leaked sockets exhaust it, high enough for the programs'
/// own handful of descriptors.
const FD_LIMIT: u32 = 64;

/// Build `source`, run it under [`FD_LIMIT`], and return `(exit, stdout, stderr)`.
fn run(name: &str, source: &str) -> (i32, String, String) {
    let project = temp_project(name, source);
    let executable = build_project(&project);
    let output = Command::new("sh")
        .arg("-c")
        .arg(format!("ulimit -n {FD_LIMIT} && exec \"$0\""))
        .arg(&executable)
        .current_dir(executable.parent().expect("executable directory"))
        .output()
        .expect("run the program under sh");
    let _ = std::fs::remove_dir_all(&project);
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn assert_clean_run(name: &str, source: &str, expected_stdout: &str) {
    let (code, stdout, stderr) = run(name, source);
    assert!(
        code == 0 && stdout == expected_stdout && stderr.is_empty(),
        "{name}: expected exit 0, no stderr, and stdout\n{expected_stdout}\n\
         got exit {code}\nstdout:\n{stdout}\nstderr:\n{stderr}\n\
         `7-703-0004` (already closed) on a later use means the TRAP closed a \
         handle it did not own; `Cleanup failure: 7-703-0009` means it tombstoned \
         one; a bind/accept failure near the descriptor limit means an owned \
         handle was never closed."
    );
}

#[test]
fn a_trapped_tcp_and_udp_poll_leaves_the_list_element_open() {
    let source = "\
IMPORT encoding
IMPORT net
IMPORT tcp
IMPORT udp
IMPORT io
IMPORT collections

FUNC main AS Integer
  RES server = tcp::listen(\"127.0.0.1\", 0)
  LET bound = tcp::localAddress(server)
  RES clientA = tcp::connect(\"127.0.0.1\", bound.port)
  RES connA = tcp::accept(server)
  RES clientB = tcp::connect(\"127.0.0.1\", bound.port)
  RES connB = tcp::accept(server)
  MUT socks AS List OF RES tcp::Socket = []
  socks = collections::append(socks, connA)
  socks = collections::append(socks, connB)
  FOR i = 1 TO 3
    tcp::write(clientA, \"t\")
    RES ready AS tcp::Socket = tcp::poll(socks, 5000) TRAP(e)
      io::print(\"tcp poll trapped \" & e.message)
      RETURN 1
    END TRAP
    io::print(\"tcp \" & toString(i) & \" \" & encoding::utf8Decode(tcp::read(ready, 8)))
  NEXT

  RES receiver = udp::bind(\"127.0.0.1\", 0)
  LET at AS net::Address = udp::localAddress(receiver)
  RES sender = udp::bind(\"127.0.0.1\", 0)
  MUT usocks AS List OF RES udp::Socket = []
  usocks = collections::append(usocks, receiver)
  FOR i = 1 TO 3
    udp::send(sender, at, \"u\")
    RES uready AS udp::Socket = udp::poll(usocks, 5000) TRAP(e)
      io::print(\"udp poll trapped \" & e.message)
      RETURN 1
    END TRAP
    LET got = udp::receive(uready, 16)
    io::print(\"udp \" & toString(i) & \" \" & encoding::utf8Decode(got.bytes))
  NEXT
  RETURN 0
END FUNC
";
    assert_clean_run(
        "bug648_poll_tcp_udp",
        source,
        "tcp 1 t\ntcp 2 t\ntcp 3 t\nudp 1 u\nudp 2 u\nudp 3 u\n",
    );
}

#[test]
fn a_trapped_collections_get_neither_tombstones_nor_closes_the_element() {
    // The poll after the loop is untrapped: it proves the list still holds a
    // live, un-tombstoned element once every trapped `get` binding has dropped.
    let source = "\
IMPORT encoding
IMPORT net
IMPORT tcp
IMPORT io
IMPORT collections

FUNC main AS Integer
  RES server = tcp::listen(\"127.0.0.1\", 0)
  LET bound = tcp::localAddress(server)
  RES client = tcp::connect(\"127.0.0.1\", bound.port)
  RES conn = tcp::accept(server)
  MUT socks AS List OF RES tcp::Socket = []
  socks = collections::append(socks, conn)
  FOR i = 1 TO 3
    tcp::write(client, \"g\")
    RES got AS tcp::Socket = collections::get(socks, 0) TRAP(e)
      io::print(\"get trapped \" & e.message)
      RETURN 1
    END TRAP
    io::print(\"get \" & toString(i) & \" \" & encoding::utf8Decode(tcp::read(got, 8)))
  NEXT
  tcp::write(client, \"p\")
  RES ready AS tcp::Socket = tcp::poll(socks, 5000)
  io::print(\"poll \" & encoding::utf8Decode(tcp::read(ready, 8)))
  RETURN 0
END FUNC
";
    assert_clean_run(
        "bug648_get_tcp",
        source,
        "get 1 g\nget 2 g\nget 3 g\npoll p\n",
    );
}

#[test]
fn mixed_an_owned_success_with_an_aliasing_recover_closes_only_what_it_opened() {
    // Odd iterations fail to bind and RECOVER the outer socket (an alias: the
    // temp must not close it). Even iterations bind a fresh socket (owned: the
    // temp must close it, or 100 of them exhaust the descriptor limit).
    let source = "\
IMPORT net
IMPORT udp
IMPORT io

FUNC main AS Integer
  RES outer = udp::bind(\"127.0.0.1\", 0)
  MUT recovered AS Integer = 0
  FOR i = 1 TO 200
    MUT host AS String = \"127.0.0.1\"
    IF i MOD 2 = 1 THEN
      host = \"999.999.1.1\"
    END IF
    RES c AS udp::Socket = udp::bind(host, 0) TRAP(e)
      recovered = recovered + 1
      RECOVER outer
    END TRAP
    LET a = udp::localAddress(c)
    IF a.port <= 0 THEN
      io::print(\"bad port at \" & toString(i))
      RETURN 1
    END IF
  NEXT
  LET b = udp::localAddress(outer)
  io::print(\"recovered=\" & toString(recovered) & \" outerOpen=\" & toString(b.port > 0))
  RETURN 0
END FUNC
";
    assert_clean_run(
        "bug648_recover_alias",
        source,
        "recovered=100 outerOpen=TRUE\n",
    );
}

#[test]
fn mixed_a_borrowed_success_with_an_owned_recover_closes_only_what_it_opened() {
    // Even iterations poll a pending datagram out of the list (borrowed: the temp
    // must not close the element). Odd iterations time out and RECOVER a fresh
    // socket (owned: the temp must close it, or 100 of them exhaust the limit).
    let source = "\
IMPORT encoding
IMPORT net
IMPORT udp
IMPORT io
IMPORT collections

FUNC main AS Integer
  RES receiver = udp::bind(\"127.0.0.1\", 0)
  LET at AS net::Address = udp::localAddress(receiver)
  RES sender = udp::bind(\"127.0.0.1\", 0)
  MUT socks AS List OF RES udp::Socket = []
  socks = collections::append(socks, receiver)
  MUT borrowed AS Integer = 0
  MUT fresh AS Integer = 0
  FOR i = 1 TO 200
    MUT wait AS Integer = 0
    IF i MOD 2 = 0 THEN
      udp::send(sender, at, \"p\")
      wait = 5000
    END IF
    MUT recovered AS Boolean = FALSE
    RES ready AS udp::Socket = udp::poll(socks, wait) TRAP(e)
      recovered = TRUE
      RECOVER udp::bind(\"127.0.0.1\", 0)
    END TRAP
    IF recovered THEN
      LET a = udp::localAddress(ready)
      fresh = fresh + 1
    ELSE
      LET got = udp::receive(ready, 16)
      IF encoding::utf8Decode(got.bytes) <> \"p\" THEN
        io::print(\"bad payload at \" & toString(i))
        RETURN 1
      END IF
      borrowed = borrowed + 1
    END IF
  NEXT
  io::print(\"borrowed=\" & toString(borrowed) & \" fresh=\" & toString(fresh))
  RETURN 0
END FUNC
";
    assert_clean_run(
        "bug648_poll_recover_owned",
        source,
        "borrowed=100 fresh=100\n",
    );
}

fn have_openssl() -> bool {
    Command::new("openssl")
        .arg("version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// A self-signed `127.0.0.1` identity for `tls::listen`, in the shape
/// `rt_double_close_is_refused` uses (Apple's certificate policy rejects a
/// longer-lived or EKU-less leaf for reasons unrelated to this test).
fn write_cert(root: &Path) -> (PathBuf, PathBuf) {
    let cert = root.join("cert.pem");
    let key = root.join("key.pem");
    let output = Command::new("openssl")
        .args([
            "req",
            "-x509",
            "-newkey",
            "rsa:2048",
            "-keyout",
            key.to_str().unwrap(),
            "-out",
            cert.to_str().unwrap(),
            "-nodes",
            "-subj",
            "/CN=localhost",
            "-addext",
            "subjectAltName=IP:127.0.0.1,DNS:localhost",
            "-addext",
            "extendedKeyUsage=serverAuth",
            "-days",
            "397",
        ])
        .output()
        .expect("run openssl to generate a self-signed identity");
    assert!(
        output.status.success(),
        "openssl failed to generate a self-signed identity\nstderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    (cert, key)
}

#[test]
fn a_trapped_tls_poll_leaves_the_list_element_open() {
    // `tls::Socket` is not thread-sendable, so its trapped result was never
    // copied — this case isolates the ownership half of the bug. The peer is
    // `openssl s_client`, fed one line per iteration through its stdin.
    if !have_openssl() {
        eprintln!("skipping: the openssl CLI is not on PATH");
        return;
    }
    let identity = std::env::temp_dir().join(format!(
        "mfb_bug648_tls_identity_{}",
        common::unique_nonce()
    ));
    std::fs::create_dir_all(&identity).expect("create identity directory");
    let (cert, key) = write_cert(&identity);
    let source = format!(
        "\
IMPORT encoding
IMPORT io
IMPORT net
IMPORT process
IMPORT tls
IMPORT collections

FUNC main AS Integer
  RES server = tls::listen(\"127.0.0.1\", 0, \"{cert}\", \"{key}\")
  LET bound = tls::localAddress(server)
  RES peer = process::spawn([\"openssl\", \"s_client\", \"-quiet\", \"-connect\", \"127.0.0.1:\" & toString(bound.port)])
  RES conn = tls::accept(server)
  MUT socks AS List OF RES tls::Socket = []
  socks = collections::append(socks, conn)
  LET newlineBytes AS List OF Byte = [toByte(10)]
  LET newline AS String = toString(newlineBytes)
  FOR i = 1 TO 3
    process::send(peer, \"s\" & newline)
    RES ready AS tls::Socket = tls::poll(socks, 10000) TRAP(e)
      io::print(\"tls poll trapped \" & e.message)
      RETURN 1
    END TRAP
    LET got AS String = encoding::utf8Decode(tls::read(ready, 64))
    io::print(\"tls \" & toString(i) & \" \" & toString(len(got) > 0))
  NEXT
  process::signal(peer, process::Signal.Kill)
  LET reaped AS Integer = process::waitFor(peer)
  RETURN 0
END FUNC
",
        cert = mfb_path_literal(&cert),
        key = mfb_path_literal(&key),
    );
    let (code, stdout, stderr) = run("bug648_poll_tls", &source);
    let _ = std::fs::remove_dir_all(&identity);
    // `openssl s_client` writes its handshake chatter to the inherited stderr, so
    // only a cleanup failure is disqualifying there.
    assert!(
        code == 0
            && stdout == "tls 1 TRUE\ntls 2 TRUE\ntls 3 TRUE\n"
            && !stderr.contains("Cleanup failure"),
        "bug648_poll_tls: expected exit 0 and three reads\n\
         got exit {code}\nstdout:\n{stdout}\nstderr:\n{stderr}\n\
         `tls poll trapped Resource handle is already closed.` on iteration 2 \
         means the TRAP closed the list's element at the end of iteration 1."
    );
}
