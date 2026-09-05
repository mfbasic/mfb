//! bug-477 non-goal guard: `http::`'s HTTPS path must keep verifying fully.
//!
//! `helper_start_exchange.rs` reaches HTTPS through
//! `tls::connect(url.host, url.port, __HTTP_CONNECT_TIMEOUT_MS, url.host)` — four
//! arguments, so `allowSelfSigned` is supplied by `ir/lower.rs`'s trailing-optional
//! padding rather than written in the source. That padding is the single seam
//! where a mistake in this bug turns every HTTPS client in the language into a
//! MITM target, with **no compile error and no behavioural symptom on a
//! well-configured host** — a padded `TRUE` still talks to real servers perfectly.
//! Only a deliberately untrusted peer distinguishes the two.
//!
//! Two independent checks, because each catches a different mistake:
//!
//! 1. `the_padded_flag_is_false` reads the lowered IR of an `http::read` and
//!    asserts the padded constant. This catches the pad table being given the
//!    wrong value, which is the mistake most likely to be made and least likely
//!    to be noticed.
//! 2. `https_to_a_self_signed_peer_still_fails` is the behavioural proof: point
//!    `http::read` at a self-signed TLS server and require it to fail. If the pad
//!    ever became `TRUE`, this connects.
//!
//! The second is macOS/Linux-only for the same reason as
//! `rt_tls_connect_allow_self_signed.rs`: it needs the `openssl` CLI as a peer.

mod common;

use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn nonce() -> String {
    common::unique_nonce()
}

fn write_project(root: &Path, source: &str) {
    fs::create_dir_all(root.join("src")).expect("create src dir");
    fs::write(
        root.join("project.json"),
        "{\"name\":\"httpsguard\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\
         \"kind\":\"executable\",\
         \"sources\":[{\"root\":\"src\",\"role\":\"main\",\"include\":[\"**/*.mfb\"]}],\
         \"entry\":\"main\",\"targets\":[\"native\"]}\n",
    )
    .expect("write project.json");
    fs::write(root.join("src/main.mfb"), source).expect("write source");
}

/// The padded `allowSelfSigned` on `http::`'s own `tls::connect` must be `false`.
///
/// Asserted against the lowered IR rather than the descriptor, because the
/// descriptor default and the value that actually reaches codegen are two
/// different things — `default_argument_padding` is what builds the constant.
#[test]
fn the_padded_flag_is_false() {
    let root = std::env::temp_dir().join(format!("mfb_bug477_httpsir_{}", nonce()));
    write_project(
        &root,
        "IMPORT http\nIMPORT net\n\nFUNC main AS Integer\n\
        \x20 LET r = http::read(net::toUrl(\"https://example.com/\"))\n\
        \x20 RETURN 0\n\
         END FUNC\n",
    );
    let output = Command::new(common::mfb_exe())
        .arg("build")
        .arg(&root)
        .arg("-ir")
        .output()
        .expect("run mfb build -ir");
    assert!(
        output.status.success(),
        "an http::read program must build:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let ir_path = String::from_utf8(output.stdout)
        .expect("utf8 build output")
        .lines()
        .find_map(|line| line.strip_prefix("Wrote IR to "))
        .map(PathBuf::from)
        .expect("build reported an IR path");
    let ir = fs::read_to_string(&ir_path).expect("read the lowered IR");

    // Every tls.connect the http package lowers, with its argument list.
    let calls: Vec<&str> = ir
        .match_indices("\"target\": \"tls.connect\"")
        .map(|(i, _)| &ir[i..])
        .collect();
    assert!(
        !calls.is_empty(),
        "bug-477: http::read(https://…) must still lower a tls.connect — if this \
         fires, the HTTPS path moved and this guard is watching nothing"
    );
    for call in calls {
        let args = &call[..call
            .find("] }")
            .map(|e| e + 3)
            .unwrap_or(call.len().min(2000))];
        assert!(
            args.contains(r#"{ "kind": "const", "type": "Boolean", "value": "false" }"#),
            "bug-477 non-goal: http::'s HTTPS path must pad allowSelfSigned as \
             Boolean `false`. A padded `true` here silently turns every HTTPS \
             client in the language into a MITM target — no compile error, and no \
             symptom against a correctly-configured server. Lowered args were:\n{args}"
        );
    }
    let _ = fs::remove_dir_all(&root);
}

/// The behavioural half: `http::read` against a self-signed TLS peer must fail.
#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn https_to_a_self_signed_peer_still_fails() {
    if Command::new("openssl")
        .arg("version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| !s.success())
        .unwrap_or(true)
    {
        eprintln!("skipping: openssl CLI not available");
        return;
    }
    let root = std::env::temp_dir().join(format!("mfb_bug477_httpsrt_{}", nonce()));
    fs::create_dir_all(&root).expect("create temp root");
    let cert = root.join("cert.pem");
    let key = root.join("key.pem");
    assert!(
        Command::new("openssl")
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
                "397",
                "-nodes",
                "-subj",
                "/CN=localhost",
                "-addext",
                "subjectAltName=DNS:localhost,IP:127.0.0.1",
                "-addext",
                "extendedKeyUsage=serverAuth",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("run openssl req")
            .success(),
        "openssl failed to generate a self-signed cert"
    );
    let port = TcpListener::bind("127.0.0.1:0")
        .expect("bind an ephemeral port")
        .local_addr()
        .expect("read the bound address")
        .port();
    let mut server = Command::new("openssl")
        .args([
            "s_server",
            "-quiet",
            "-accept",
            &port.to_string(),
            "-cert",
            cert.to_str().unwrap(),
            "-key",
            key.to_str().unwrap(),
            "-naccept",
            "2",
            "-www",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn openssl s_server");
    for _ in 0..200 {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }

    write_project(
        &root,
        &format!(
            "IMPORT http\nIMPORT io\nIMPORT net\n\nFUNC main AS Integer\n\
            \x20 LET r = http::read(net::toUrl(\"https://localhost:{port}/\")) TRAP\n\
            \x20   io::print(\"result=raised\")\n\
            \x20   RETURN 0\n\
            \x20 END TRAP\n\
            \x20 io::print(\"result=connected\")\n\
            \x20 RETURN 0\n\
             END FUNC\n"
        ),
    );
    let build = Command::new(common::mfb_exe())
        .arg("build")
        .arg(&root)
        .output()
        .expect("run mfb build");
    assert!(
        build.status.success(),
        "the https guard program must build:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let exe = String::from_utf8(build.stdout)
        .expect("utf8 build output")
        .lines()
        .find_map(|line| line.strip_prefix("Wrote executable to "))
        .map(PathBuf::from)
        .expect("build reported an executable path");
    let run = Command::new(&exe).output().expect("run the https guard");
    let stdout = String::from_utf8_lossy(&run.stdout);
    let _ = server.kill();
    let _ = server.wait();
    let _ = fs::remove_dir_all(&root);

    assert!(
        stdout.contains("result=raised"),
        "bug-477 non-goal: http:: must NOT expose allowSelfSigned, so an HTTPS \
         request to a self-signed peer must still fail. It reported {stdout:?} — \
         if that is `result=connected`, the padded flag reached tls::connect as \
         TRUE and every http:: HTTPS caller has silently lost certificate \
         verification"
    );
}
