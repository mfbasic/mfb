//! bug-477: `tls::connect(..., allowSelfSigned := TRUE)` accepts a chain whose
//! *only* defect is an untrusted root — and still rejects everything else.
//!
//! ## Why this is a Rust test and not an `rt-behavior` golden fixture
//!
//! The behaviour needs three different certificates (a good self-signed one, a
//! name-mismatched one, and an expired one) and a private key for each. A static
//! golden fixture cannot carry them: the key would be committed, and the expiry
//! cases are defined *relative to now*, so any committed pair rots. The sibling
//! `rt_tls_listener_thread_transfer.rs` hit the same wall from the server side
//! and resolved it the same way — generate the identity at run time.
//!
//! ## Why the peer is `openssl s_server` and not `tls::listen`
//!
//! Quoting the sibling test, "a proof where our client and our server agree with
//! each other cannot tell a working TLS server from two matching bugs". Here the
//! asymmetry matters even more: the whole point of the flag is to relax *client*
//! trust, so the peer must be an implementation that has no idea this flag
//! exists.
//!
//! ## What each case pins, and why the negatives are the important ones
//!
//! A test proving a self-signed certificate is accepted proves nothing about
//! safety — an implementation that simply disables verification passes it. The
//! four cases are therefore a matched set:
//!
//! | case | flag | peer certificate | required outcome |
//! | --- | --- | --- | --- |
//! | `accepts_a_self_signed_peer` | `TRUE` | self-signed, name matches | **connects** |
//! | `still_rejects_a_name_mismatch` | `TRUE` | self-signed, `CN=wrong.example` | **raises** |
//! | `still_rejects_an_expired_certificate` | `TRUE` | self-signed, expired 2020 | **raises** |
//! | `defaults_to_rejecting_a_self_signed_peer` | omitted | self-signed, name matches | **raises** |
//!
//! The last one is the non-goal guard: omitting the argument must be exactly
//! today's handshake, because `http::`'s HTTPS path reaches this member with the
//! argument padded and must keep verifying.
//!
//! macOS and Linux only, matching `rt_tls_listener_thread_transfer.rs`: both run
//! a real TLS client (Network.framework and OpenSSL) and Windows is excluded
//! because the `openssl` CLI this uses as the peer is not a dependable presence
//! there. The Windows/Schannel side of the flag is proven separately on the
//! remote box (see the bug document's validation matrix).

#![cfg(any(target_os = "macos", target_os = "linux"))]

mod common;

use std::fs;
use std::io::{BufRead, BufReader};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

/// Wall-clock bound on the whole client run. Generous on purpose: this turns a
/// *hang* into a named failure so a wedged handshake cannot stall `cargo test`.
/// It is not a performance assertion.
const DEADLINE: Duration = Duration::from_secs(120);

/// The name the client validates against, and the name the good certificate
/// carries. Deliberately a DNS name rather than the `127.0.0.1` literal the
/// connection actually dials: `SSL_set1_host` matches DNS-name SANs only
/// (`gen_openssl.rs`'s note on `X509_VERIFY_PARAM_set1_ip`), so a test that
/// validated against the IP would be asserting an unsupported path.
const EXPECT_NAME: &str = "localhost";

/// bug-488, the half the port gate does not cover: this file's four cases run
/// concurrently and each names its scratch root `mfb_bug477_<nonce>`. A
/// nanosecond timestamp does NOT make that unique — measured on the macOS host,
/// 698 577 of 800 000 stamps taken from four threads were duplicates — so two
/// cases can share a root. `write_cert` puts `cert.pem`/`key.pem` at fixed names
/// inside it, so a colliding pair serves the WRONG IDENTITY and a negative case
/// reports `result=connected`; and whichever finishes first `remove_dir_all`s
/// the other's client out from under it (`spawn the mfb tls client: NotFound`,
/// the macOS CI row). `common::unique_nonce` is a pid + counter, which cannot
/// repeat.
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

/// Which certificate the peer serves. Each is self-signed — the *only* axis
/// under test is what else is wrong with it besides its root.
#[derive(Clone, Copy)]
enum Peer {
    /// Well-formed, name-correct, in date. Fails trust and nothing else.
    Good,
    /// In date, but `CN=wrong.example` — the name check must still reject it.
    NameMismatch,
    /// Name-correct, but expired in 2020 — the date check must still reject it.
    Expired,
}

impl Peer {
    fn subject(self) -> &'static str {
        match self {
            Peer::NameMismatch => "/CN=wrong.example",
            _ => "/CN=localhost",
        }
    }

    fn san(self) -> &'static str {
        match self {
            Peer::NameMismatch => "subjectAltName=DNS:wrong.example",
            _ => "subjectAltName=DNS:localhost,IP:127.0.0.1",
        }
    }
}

/// Run `openssl` with `args`, and on failure report the exit code and the CLI's
/// own stderr.
///
/// bug-485: these spawns used to discard both streams, so an *unsupported
/// option* — the CLI printing a usage block and exiting 1 — was indistinguishable
/// from a broken install, a bad subject, or an unwritable path. The whole cost of
/// diagnosing that bug was paid here. Keep the streams captured.
fn run_openssl(args: &[&str], what: &str) {
    let output = Command::new("openssl")
        .args(args)
        .output()
        .unwrap_or_else(|err| panic!("run openssl to {what}: {err}"));
    assert!(
        output.status.success(),
        "openssl failed to {what} (exit {:?})\nargv: openssl {}\nstderr:\n{}",
        output.status.code(),
        args.join(" "),
        String::from_utf8_lossy(&output.stderr),
    );
}

/// Generate the self-signed pair for `peer` under `root`.
///
/// The expired case is pinned to a fixed 2019–2020 window rather than a negative
/// `-days`, so the certificate is unambiguously expired no matter when the suite
/// runs and the test never becomes time-dependent. A negative `-days` is *not* a
/// shortcut to the same place: `openssl req -x509 -days -400` is accepted by
/// LibreSSL 3.3.6 and emits a certificate valid from now to now+30d — a live
/// certificate, which would silently invert `still_rejects_an_expired_certificate`
/// into asserting the opposite of what it reads as.
///
/// **`-days 397` and `extendedKeyUsage=serverAuth` are load-bearing on macOS.**
/// Apple enforces a certificate *shape* policy that OpenSSL does not: a TLS
/// server certificate must carry `serverAuth` and a validity window under ~398
/// days, or `SecTrustEvaluateWithError` rejects it as "not standards compliant"
/// no matter what the anchors are. A 10-year certificate here would fail the
/// positive case on macOS for a reason that has nothing to do with this bug.
fn write_cert(root: &Path, peer: Peer) -> (PathBuf, PathBuf) {
    let cert = root.join("cert.pem");
    let key = root.join("key.pem");
    match peer {
        Peer::Expired => write_expired_cert(root, peer, &cert, &key),
        _ => run_openssl(
            &[
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
                peer.subject(),
                "-addext",
                peer.san(),
                "-addext",
                "extendedKeyUsage=serverAuth",
                "-days",
                "397",
            ],
            "generate an in-date self-signed cert",
        ),
    }
    (cert, key)
}

/// Build the expired identity through a CSR and `openssl ca -selfsign`, rather
/// than the one-shot `openssl req -x509` the in-date peers use.
///
/// **This detour is required, not stylistic (bug-485).** Setting an explicit
/// window on `req` needs `-not_before`/`-not_after`, which exist only in OpenSSL
/// **3.5+** — they are absent from the 3.0 `req(1)` manpage. CI runs
/// ubuntu-24.04, whose `openssl` is 3.0.13, so there the CLI printed a usage
/// block and exited 1 and `still_rejects_an_expired_certificate` died in
/// `write_cert` before a byte of TLS was exchanged. The negative guard that gives
/// `allowSelfSigned` its meaning therefore ran nowhere except a developer box
/// with a new enough OpenSSL first on `PATH`.
///
/// `ca -startdate`/`-enddate` has been there since OpenSSL 1.x and is documented
/// in the 3.0 `ca(1)` manpage; OpenSSL 3.6 reports `-not_before` as *an alias
/// for* `-startdate`. The recipe below is measured to produce the identical
/// certificate on LibreSSL 3.3.6, OpenSSL 3.5.6, 3.6.2 and 3.6.3 — which brackets
/// CI's 3.0.13 from both sides. Do not "simplify" this back to the one-shot `req`
/// form.
///
/// `ca` takes its extensions from the config's `x509_extensions` section and
/// ignores `-addext`, so the SAN and `serverAuth` EKU move into the generated
/// `.cnf`. [`Peer::san`] stays the single source of truth for both paths because
/// an `-addext` argument and a config line have the same `key=value` spelling.
fn write_expired_cert(root: &Path, peer: Peer, cert: &Path, key: &Path) {
    let cnf = root.join("ca.cnf");
    let csr = root.join("csr.pem");
    let index = root.join("index.txt");
    let serial = root.join("serial");
    // `ca` refuses to run without its bookkeeping: an (empty) index database and
    // a serial to start from. `rand_serial = no` is what makes it read the file.
    fs::write(&index, "").expect("write the ca index");
    fs::write(&serial, "01\n").expect("write the ca serial");
    fs::write(
        &cnf,
        format!(
            "[ ca ]\n\
             default_ca = CA_default\n\
             \n\
             [ CA_default ]\n\
             database = {index}\n\
             serial = {serial}\n\
             new_certs_dir = {dir}\n\
             default_md = sha256\n\
             policy = pol\n\
             x509_extensions = ext\n\
             email_in_dn = no\n\
             rand_serial = no\n\
             unique_subject = no\n\
             \n\
             [ pol ]\n\
             commonName = supplied\n\
             \n\
             [ ext ]\n\
             {san}\n\
             extendedKeyUsage = serverAuth\n\
             basicConstraints = critical,CA:TRUE\n\
             \n\
             [ req ]\n\
             distinguished_name = dn\n\
             \n\
             [ dn ]\n",
            index = index.display(),
            serial = serial.display(),
            dir = root.display(),
            san = peer.san(),
        ),
    )
    .expect("write the ca config");

    run_openssl(
        &[
            "req",
            "-new",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-keyout",
            key.to_str().unwrap(),
            "-out",
            csr.to_str().unwrap(),
            "-subj",
            peer.subject(),
            "-config",
            cnf.to_str().unwrap(),
        ],
        "generate the expired peer's key and CSR",
    );
    run_openssl(
        &[
            "ca",
            "-batch",
            "-selfsign",
            "-notext",
            "-config",
            cnf.to_str().unwrap(),
            "-keyfile",
            key.to_str().unwrap(),
            "-in",
            csr.to_str().unwrap(),
            "-out",
            cert.to_str().unwrap(),
            "-startdate",
            "190101000000Z",
            "-enddate",
            "200101000000Z",
        ],
        "self-sign the expired peer certificate",
    );
    assert_expired_cert_shape(cert);
}

/// Read the expired certificate back and pin the three properties the case
/// depends on.
///
/// This is not belt-and-braces. `still_rejects_an_expired_certificate` asserts
/// only that the handshake *raised*, so it passes for **any** rejection — and the
/// `ca` path above can fail quietly in exactly the way that produces a wrong
/// pass: `openssl ca` silently drops the SAN if `x509_extensions` is missing or
/// misspelled, and a certificate with no SAN is rejected for its **name**, not
/// its date. That is `still_rejects_a_name_mismatch`'s job, not this one's.
/// Without this check, a typo in the config above turns the expiry guard into a
/// duplicate of its neighbour and nothing goes red.
fn assert_expired_cert_shape(cert: &Path) {
    let text = Command::new("openssl")
        .args(["x509", "-in", cert.to_str().unwrap(), "-noout", "-text"])
        .output()
        .expect("read back the expired certificate");
    assert!(
        text.status.success(),
        "openssl could not parse the certificate `ca -selfsign` just wrote:\n{}",
        String::from_utf8_lossy(&text.stderr),
    );
    let text = String::from_utf8_lossy(&text.stdout);
    for needle in ["DNS:localhost", "TLS Web Server Authentication"] {
        assert!(
            text.contains(needle),
            "the expired certificate is missing `{needle}`, so this case would be \
             rejected for the wrong reason (name, not date) and pass anyway:\n{text}"
        );
    }
    // `-checkend 0` exits non-zero when the certificate is already expired, which
    // is a semantic assertion rather than a match on the CLI's date formatting
    // (LibreSSL and OpenSSL do not agree on spacing).
    let checkend = Command::new("openssl")
        .args([
            "x509",
            "-in",
            cert.to_str().unwrap(),
            "-noout",
            "-checkend",
            "0",
        ])
        .output()
        .expect("check the expired certificate's validity window");
    assert!(
        !checkend.status.success(),
        "the certificate meant to be expired is still valid — this case would \
         assert the opposite of what it reads as:\n{}",
        String::from_utf8_lossy(&checkend.stdout),
    );
}

/// Serializes "pick a port" through "`s_server` has that port bound".
///
/// `free_port` has to *release* the port before `s_server` can bind it, and the four
/// cases in this file run concurrently — so without this gate two of them can be
/// handed the same ephemeral port inside that window. See `start_peer` for why that
/// is not merely a flake.
// bug-488: the gate is `common::PortGate`, a `flock(2)` on a file in the temp
// dir, so it serializes every process on the machine — not just this binary.
// The `static OnceLock<Mutex<()>>` that used to live here was per-PROCESS: it
// ordered the four cases in this file and nothing else, so a second `cargo test`
// (routine, with work running in several worktrees at once) could take our port
// inside the release window. Six false failures came from that, in BOTH
// directions — a verdict that flips either way is two runs sharing state, not
// one stale value leaking.

/// A port nothing is listening on yet. `s_server` is then told to bind it.
///
/// Inherently a bind-then-release: the listener has to be dropped so `s_server` can
/// take the port, so the port is free for a moment. `port_gate` narrows that window
/// to one case at a time and `start_peer` detects a loser, which together is what
/// makes this safe — see the note there. (Parsing `s_server`'s banner instead is
/// markedly less stable across OpenSSL versions.)
fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("bind an ephemeral port")
        .local_addr()
        .expect("read the bound address")
        .port()
}

/// Serve `peer` on a fresh port and return the child once it is accepting.
///
/// **The readiness probe cannot be the only check, and that is the whole point of the
/// retry.** If two cases are handed the same port, one `s_server` wins the bind and
/// the other exits immediately — and the loser's probe then connects to the *winner's*
/// server, reports ready, and the loser's client completes a handshake against the
/// wrong identity. That does not fail loudly: it silently returns the other case's
/// verification outcome, so `still_rejects_a_name_mismatch` can report `connected` and
/// `accepts_a_self_signed_peer` can report `raised`. Both were observed on this file
/// under load, in the same session, on different runs.
///
/// So a live child is part of the readiness condition. A child that has already
/// exited lost the bind; take a new port and try again rather than probing a server
/// that is not ours.
fn start_peer(root: &Path, peer: Peer) -> (Child, u16) {
    let (cert, key) = write_cert(root, peer);
    let log = root.join("s_server.err");
    for _ in 0..10 {
        let guard = common::PortGate::acquire();
        let port = free_port();
        let port_arg = port.to_string();
        let args = [
            "s_server",
            "-quiet",
            "-accept",
            &port_arg,
            "-cert",
            cert.to_str().unwrap(),
            "-key",
            key.to_str().unwrap(),
            "-naccept",
            "4",
        ];
        let mut child = Command::new("openssl")
            .args(&args)
            .stdout(Stdio::null())
            .stderr(Stdio::from(
                fs::File::create(&log).expect("create the s_server log"),
            ))
            .spawn()
            .expect("spawn openssl s_server");
        let mut lost = false;
        for _ in 0..200 {
            match child.try_wait().expect("poll openssl s_server") {
                // Exited before accepting: it could not bind, so whatever is listening
                // on that port belongs to another case.
                Some(_) => {
                    // ...unless it never got as far as bind(2). bug-485: an
                    // unusable *argument* also exits immediately, and this loop
                    // read that as a lost race, retried it ten times, and then
                    // blamed the ports — naming a cause that had nothing to do
                    // with the failure. Retrying cannot fix an argument, so stop
                    // on the spot and quote what the CLI actually said.
                    //
                    // In practice this fires on macOS `/usr/bin/openssl`, which
                    // is LibreSSL and has no `-naccept`. **Do not "fix" that by
                    // dropping the option.** It was tried: LibreSSL then starts,
                    // but its `s_server` stops serving after the bare TCP
                    // readiness probe below, so the client never connects and
                    // the three cases that assert `result=raised` pass without
                    // verifying anything — measured, `3 passed; 1 failed` on a
                    // peer that had done nothing. LibreSSL cannot be this file's
                    // peer; failing here, loudly, is the intended outcome.
                    let said = fs::read_to_string(&log).unwrap_or_default();
                    assert!(
                        !said.contains("unknown option") && !said.contains("usage:"),
                        "openssl s_server rejected an argument, so this is not a \
                         lost bind and retrying will not help:\nargv: openssl {}\n\
                         stderr:\n{said}",
                        args.join(" "),
                    );
                    lost = true;
                    break;
                }
                None => {
                    if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
                        drop(guard);
                        return (child, port);
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        drop(guard);
        if !lost {
            let _ = child.kill();
            let _ = child.wait();
            panic!("openssl s_server never began accepting on port {port}");
        }
        let _ = child.wait();
    }
    panic!("openssl s_server lost the bind on ten consecutive ports");
}

/// Build a client that connects to `port` and prints exactly one line:
/// `result=connected` or `result=raised`.
///
/// `allow` selects whether the new argument is passed at all — `None` is the
/// today's-behaviour guard, and must stay a strict handshake.
fn build_client(root: &Path, port: u16, allow: Option<bool>) -> PathBuf {
    fs::create_dir_all(root.join("src")).expect("create src dir");
    fs::write(
        root.join("project.json"),
        "{\"name\":\"tlsselfsigned\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\
         \"kind\":\"executable\",\
         \"sources\":[{\"root\":\"src\",\"role\":\"main\",\"include\":[\"**/*.mfb\"]}],\
         \"entry\":\"main\",\"targets\":[\"native\"]}\n",
    )
    .expect("write project.json");

    let arg = match allow {
        Some(value) => format!(
            ", allowSelfSigned := {}",
            if value { "TRUE" } else { "FALSE" }
        ),
        None => String::new(),
    };
    // An inline `TRAP` on the `RES` binding reduces the outcome to one stable
    // token: whether the handshake completed. The assertion is then just which
    // token appeared, so nothing depends on the peer's bytes or on the wording
    // of the error. The handler diverges rather than `RECOVER`ing because there
    // is no second `tls::Socket` to recover with.
    let source = format!(
        "IMPORT io\n\
         IMPORT tls\n\n\
         FUNC main AS Integer\n\
        \x20 RES conn = tls::connect(\"127.0.0.1\", {port}, 5000, \"{EXPECT_NAME}\"{arg}) TRAP(e)\n\
        \x20   io::print(\"result=raised code=\" & toString(e.code))\n\
        \x20   RETURN 0\n\
        \x20 END TRAP\n\
        \x20 io::print(\"result=connected\")\n\
        \x20 tls::close(conn)\n\
        \x20 RETURN 0\n\
         END FUNC\n",
    );
    fs::write(root.join("src/main.mfb"), source).expect("write source");

    let output = Command::new(common::mfb_exe())
        .arg("build")
        .arg(root)
        .output()
        .expect("run mfb build");
    assert!(
        output.status.success(),
        "bug-477: a program passing `allowSelfSigned` to tls::connect must build:\n\
         stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("utf8 build output")
        .lines()
        .find_map(|line| line.strip_prefix("Wrote executable to "))
        .map(PathBuf::from)
        .expect("build output executable path")
}

/// Run the client and return its `result=` token, bounded by [`DEADLINE`].
fn run_client(exe: &Path) -> String {
    let mut child = Command::new(exe)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn the mfb tls client");
    let stdout = child.stdout.take().expect("client stdout");
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        let _ = BufReader::new(stdout).read_line(&mut line);
        let _ = tx.send(line);
    });
    match rx.recv_timeout(DEADLINE) {
        Ok(line) => {
            let _ = child.wait();
            line.trim().to_string()
        }
        Err(_) => {
            let _ = child.kill();
            panic!(
                "the tls client printed no result within {}s",
                DEADLINE.as_secs()
            );
        }
    }
}

/// The whole harness for one case: serve `peer`, connect with `allow`, return
/// the token.
fn outcome(peer: Peer, allow: Option<bool>) -> String {
    let root = std::env::temp_dir().join(format!("mfb_bug477_{}", nonce()));
    fs::create_dir_all(&root).expect("create temp root");
    let (mut server, port) = start_peer(&root, peer);
    let exe = build_client(&root, port, allow);
    let token = run_client(&exe);
    let _ = server.kill();
    let _ = server.wait();
    let _ = fs::remove_dir_all(&root);
    token
}

#[test]
fn accepts_a_self_signed_peer() {
    if !have_openssl() {
        eprintln!("skipping: openssl CLI not available");
        return;
    }
    assert_eq!(
        outcome(Peer::Good, Some(true)),
        "result=connected",
        "bug-477: with `allowSelfSigned := TRUE`, a chain whose only defect is an \
         untrusted root must complete the handshake"
    );
}

#[test]
fn still_rejects_a_name_mismatch() {
    if !have_openssl() {
        eprintln!("skipping: openssl CLI not available");
        return;
    }
    {
        let got = outcome(Peer::NameMismatch, Some(true));
        assert!(
            got.starts_with("result=raised"),
            "bug-477: `allowSelfSigned` relaxes the trust anchor and NOTHING else — a \
         certificate whose name does not match the expected server name must still \
         raise, or the flag is a blanket verification bypass; observed: {got}"
        );
    }
}

#[test]
fn still_rejects_an_expired_certificate() {
    if !have_openssl() {
        eprintln!("skipping: openssl CLI not available");
        return;
    }
    {
        let got = outcome(Peer::Expired, Some(true));
        assert!(
            got.starts_with("result=raised"),
            "bug-477: `allowSelfSigned` relaxes the trust anchor and NOTHING else — an \
         expired certificate must still raise, or the flag is a blanket \
         verification bypass; observed: {got}"
        );
    }
}

#[test]
fn defaults_to_rejecting_a_self_signed_peer() {
    if !have_openssl() {
        eprintln!("skipping: openssl CLI not available");
        return;
    }
    {
        let got = outcome(Peer::Good, None);
        assert!(
            got.starts_with("result=raised"),
            "bug-477 non-goal: omitting `allowSelfSigned` must be exactly today's \
         handshake. `http::`'s HTTPS path reaches tls::connect with this argument \
         padded, so a padded default of anything but FALSE silently turns every \
         HTTPS client in the language into a MITM target; observed: {got}"
        );
    }
}
