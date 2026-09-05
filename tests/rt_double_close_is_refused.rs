//! bug-525: closing an already-closed handle answers the same way in every
//! built-in package.
//!
//! `mfb spec language resource-management` §15 states the contract for the whole
//! language:
//!
//! > A resource is still closed **exactly once**. What changed is *who* may close
//! > it, never *how many times*: an already-closed record is flagged, and a second
//! > close is a defined no-op reported as `ErrResourceClosed` rather than an
//! > operation on a dead handle.
//!
//! `fs`, `tcp` and `udp` implemented it. `tls` and `audio` returned success
//! instead, and both pages documented their own answer, so the divergence was
//! ratified rather than noticed. This test measures the answer rather than
//! reading it, for every package a runtime fixture can reach.
//!
//! ## Why a second close needs a helper
//!
//! `close` moves its argument, so the literal
//!
//! ```text
//! tcp::close(l)
//! tcp::close(l)
//! ```
//!
//! is refused at compile time (`TYPE_USE_AFTER_MOVE`) and never reaches a
//! backend. The reachable shape is the one §15 describes: a `RES` parameter is
//! the same handle seen from a deeper scope, and any holder may close it — so a
//! `SUB` taking `RES` and closing it, called twice, is a real double close and is
//! how a program meets this in practice.
//!
//! ## Coverage
//!
//! | package | handle | before the fix |
//! | --- | --- | --- |
//! | `fs` | `fs::File` | raises — positive pin |
//! | `tcp` | `tcp::Listener` | raises — positive pin |
//! | `udp` | `udp::Socket` | raises — positive pin |
//! | `tls` | `tls::Listener` | **succeeded** — the RED case |
//!
//! `tls::Socket` needs a completed handshake and `audio` needs a device, so
//! neither is reachable from a self-contained fixture. Both are pinned at the
//! lowering instead, on all three backends, by
//! `codegen::resource::tests::every_builtin_close_refuses_an_already_closed_handle`.
//!
//! macOS and Linux only: the `openssl` CLI that mints the server identity is not
//! a dependable presence on Windows. The Schannel side is proven by the lowering
//! pin and on box 2230.

#![cfg(any(target_os = "macos", target_os = "linux"))]

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use common::{build_project, mfb_path_literal, run_bounded, temp_project, unique_nonce};

/// `errorCode::ErrResourceClosed` — `mfb spec diagnostics error-codes` row
/// `7-703-0004`. The programs print the raw code so the assertion cannot be
/// satisfied by a differently-shaped failure.
const RESOURCE_CLOSED: &str = "77030004";

/// Generous on purpose: the bound exists to turn a hang into a named failure,
/// not to police performance.
const RUN_TIMEOUT: Duration = Duration::from_secs(60);

fn have_openssl() -> bool {
    Command::new("openssl")
        .arg("version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// A self-signed `127.0.0.1` identity for `tls::listen`.
///
/// `-days 397` and `extendedKeyUsage=serverAuth` mirror
/// `rt_tls_connect_allow_self_signed`: Apple enforces a certificate *shape*
/// policy that OpenSSL does not, and a longer-lived certificate is rejected on
/// macOS for reasons unrelated to this test. Nothing here completes a handshake,
/// but `tls::listen` still loads and inspects the identity.
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
        "openssl failed to generate a self-signed identity (exit {:?})\nstderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    (cert, key)
}

/// Build and run `source`, returning its stdout.
fn run(name: &str, source: &str) -> String {
    let project = temp_project(name, source);
    let executable = build_project(&project);
    let (status, stdout) = run_bounded(
        &executable,
        RUN_TIMEOUT,
        &format!("{name} double-close program"),
    );
    assert!(
        status.success(),
        "{name} exited {:?}\nstdout:\n{stdout}",
        status.code()
    );
    stdout
}

/// The shared shape: open a handle, close it through a `RES` parameter, close it
/// again the same way, and print the second call's outcome.
///
/// `TRAP` has to sit at the bottom of a function, so the second close lives in
/// its own — which also keeps the two calls textually distinct and immune to any
/// future peephole that might notice a repeated statement.
fn program(imports: &str, open: &str, type_: &str, close_call: &str) -> String {
    format!(
        "IMPORT io\n\
         IMPORT errorCode\n\
         {imports}\n\
         \n\
         SUB closeIt(RES h AS {type_})\n\
        \x20 {close_call}\n\
         END SUB\n\
         \n\
         FUNC closeAgain(RES h AS {type_}) AS Integer\n\
        \x20 closeIt(h)\n\
        \x20 RETURN 0\n\
        \x20 TRAP(e)\n\
        \x20   RETURN e.code\n\
        \x20 END TRAP\n\
         END FUNC\n\
         \n\
         FUNC main AS Integer\n\
        \x20 RES h AS {type_} = {open}\n\
        \x20 closeIt(h)\n\
        \x20 io::print(\"first=ok\")\n\
        \x20 io::print(\"second=\" & toString(closeAgain(h)))\n\
        \x20 RETURN 0\n\
         END FUNC\n"
    )
}

fn assert_second_close_raises(package: &str, stdout: &str) {
    assert!(
        stdout.contains("first=ok"),
        "{package}: the FIRST close must succeed — a test where nothing opened \
         would pass the negative assertion alone.\nstdout:\n{stdout}"
    );
    assert!(
        stdout.contains(&format!("second={RESOURCE_CLOSED}")),
        "{package}: a second close must be refused with ErrResourceClosed \
         ({RESOURCE_CLOSED}), per `mfb spec language resource-management` §15 — \
         \"a second close is a defined no-op reported as ErrResourceClosed\". \
         `second=0` means the close reported success on a dead handle \
         (bug-525).\nstdout:\n{stdout}"
    );
}

#[test]
fn fs_close_refuses_an_already_closed_file() {
    let source = program(
        "IMPORT fs",
        "fs::createTempFile()",
        "fs::File",
        "fs::close(h)",
    );
    assert_second_close_raises("fs", &run("dclose_fs", &source));
}

#[test]
fn tcp_close_refuses_an_already_closed_listener() {
    let source = program(
        "IMPORT tcp",
        "tcp::listen(\"127.0.0.1\", 0)",
        "tcp::Listener",
        "tcp::close(h)",
    );
    assert_second_close_raises("tcp", &run("dclose_tcp", &source));
}

#[test]
fn udp_close_refuses_an_already_closed_socket() {
    let source = program(
        "IMPORT udp",
        "udp::bind(\"127.0.0.1\", 0)",
        "udp::Socket",
        "udp::close(h)",
    );
    assert_second_close_raises("udp", &run("dclose_udp", &source));
}

/// The RED case. Before the fix this printed `second=0`: `tls::close` set the
/// closed flag once and then reported success forever after, which
/// `tls/func_close.rs` documented as a deliberate difference from `tcp`.
#[test]
fn tls_close_refuses_an_already_closed_listener() {
    if !have_openssl() {
        eprintln!("skipping: no openssl CLI to mint a server identity");
        return;
    }
    let root = std::env::temp_dir().join(format!("mfb_dclose_tls_id_{}", unique_nonce()));
    std::fs::create_dir_all(&root).expect("create the identity directory");
    let (cert, key) = write_cert(&root);
    let source = program(
        "IMPORT tls",
        &format!(
            "tls::listen(\"127.0.0.1\", 0, \"{}\", \"{}\")",
            mfb_path_literal(&cert),
            mfb_path_literal(&key)
        ),
        "tls::Listener",
        "tls::close(h)",
    );
    assert_second_close_raises("tls", &run("dclose_tls", &source));
}

/// The positive pin the negative one needs.
///
/// Making a second close raise must not make the FIRST one — or the automatic
/// close at the end of the binding's scope — behave differently. `mfb spec
/// language resource-management` §15 says a drop-close discards its failure, so
/// the ordinary idiom (open, use, close once, let the scope end) must still exit
/// cleanly on every transport, and the handle must still be closed exactly once:
/// the port has to be free for an immediate rebind afterwards.
#[test]
fn closing_once_and_letting_the_scope_end_is_unchanged() {
    if !have_openssl() {
        eprintln!("skipping: no openssl CLI to mint a server identity");
        return;
    }
    let root = std::env::temp_dir().join(format!("mfb_dclose_ok_id_{}", unique_nonce()));
    std::fs::create_dir_all(&root).expect("create the identity directory");
    let (cert, key) = write_cert(&root);
    let source = format!(
        "IMPORT io\n\
         IMPORT net\n\
         IMPORT tcp\n\
         IMPORT tls\n\
         \n\
         FUNC main AS Integer\n\
        \x20 RES plain AS tcp::Listener = tcp::listen(\"127.0.0.1\", 0)\n\
        \x20 LET plainAt AS net::Address = tcp::localAddress(plain)\n\
        \x20 io::print(\"tcp-bound=\" & toString(plainAt.port > 0))\n\
        \x20 tcp::close(plain)\n\
        \x20 io::print(\"tcp-closed-once\")\n\
        \x20 RES secure AS tls::Listener = tls::listen(\"127.0.0.1\", 0, \"{cert}\", \"{key}\")\n\
        \x20 LET secureAt AS net::Address = tls::localAddress(secure)\n\
        \x20 io::print(\"tls-bound=\" & toString(secureAt.port > 0))\n\
        \x20 tls::close(secure)\n\
        \x20 io::print(\"tls-closed-once\")\n\
        \x20 RES rebind AS tcp::Listener = tcp::listen(\"127.0.0.1\", plainAt.port)\n\
        \x20 io::print(\"rebound=\" & toString(tcp::localAddress(rebind).port = plainAt.port))\n\
        \x20 RETURN 0\n\
         END FUNC\n",
        cert = mfb_path_literal(&cert),
        key = mfb_path_literal(&key)
    );
    let stdout = run("dclose_ok", &source);
    for line in [
        "tcp-bound=TRUE",
        "tcp-closed-once",
        "tls-bound=TRUE",
        "tls-closed-once",
        "rebound=TRUE",
    ] {
        assert!(
            stdout.contains(line),
            "the ordinary close-once idiom must be unchanged, and each listener \
             must still be closed exactly once (the rebind proves the port was \
             released). Missing `{line}`.\nstdout:\n{stdout}"
        );
    }
    // Reaching here at all is the other half: `main` returns with `plain`,
    // `secure` and `rebind` still in scope, so the automatic close runs on two
    // already-closed handles. A drop-close discards its failure (§15); if it did
    // not, `run` would have failed on the exit status.
}
