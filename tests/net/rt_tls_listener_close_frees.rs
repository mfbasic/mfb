//! bug-984: on macOS, `tls::listen` + `tls::close` left three arena blocks behind.
//!
//! `tls::listen` reads both PEM files into arena buffers and marshals the host
//! into a C string the `Listener` record keeps for `tls::localAddress`. Nothing
//! freed any of the three: the PEM buffers were never released after
//! `CFDataCreate` copied them, and the host block had an owner that never freed
//! it. A server that opened and closed listeners grew by the size of its
//! certificate, its private key and its host on every cycle.
//!
//! Each case runs one `--debug` build at N and 2N cycles and requires the main
//! arena to end with `alloc_calls == free_calls`, `live_bytes 0` and no double
//! free. `tcp::listen` + `tcp::close` is the control. The value half checks the
//! thing the host block exists for: `tls::localAddress` still reports the bound
//! host while the listener is open, for a named host and for bind-all.
//!
//! macOS only: the leak is in the Network.framework lowering.

#![cfg(target_os = "macos")]

#[path = "../common/mod.rs"]
mod common;

use common::debug_report::{arena_lines, build_debug, counter};
use std::path::Path;
use std::process::{Command, Stdio};

fn write_cert(root: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
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
            "-addext",
            "subjectAltName=IP:127.0.0.1",
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

/// Run `exe` at 3 and 6 cycles; require a balanced arena at both and return the
/// 3-cycle stdout.
fn assert_balanced(name: &str, exe: &Path, envs: &[(&str, String)]) -> String {
    let mut first = None;
    for n in [3u32, 6] {
        let mut all = vec![("N", n.to_string())];
        all.extend(envs.iter().cloned());
        let (status, stdout, stderr) = common::run_capture_with_env(exe, &all);
        assert_eq!(status, 0, "{name} N={n} failed:\n{stdout}\n{stderr}");
        let lines = arena_lines(name, &stderr);
        let allocs = counter(name, &lines, 0, "alloc_calls");
        let frees = counter(name, &lines, 0, "free_calls");
        let live = counter(name, &lines, 0, "live_bytes");
        assert_eq!(
            counter(name, &lines, 0, "double_free_skips"),
            0,
            "{name} N={n}: the arena skipped a double free"
        );
        assert_eq!(
            (allocs, live),
            (frees, 0),
            "{name} N={n}: alloc_calls {allocs} free_calls {frees} live_bytes {live}"
        );
        first.get_or_insert(stdout);
    }
    first.expect("ran")
}

const PRELUDE: &str = r#"IMPORT io
IMPORT os
IMPORT tls
IMPORT tcp
IMPORT net

FUNC cycles() AS Integer
  RETURN toInt(os::getEnv("N"))
END FUNC
"#;

#[test]
fn a_listener_closed_explicitly_frees_every_block_listen_allocated() {
    let root = std::env::temp_dir().join(format!("mfb_b984_{}", common::unique_nonce()));
    std::fs::create_dir_all(&root).expect("root");
    let (cert, key) = write_cert(&root);
    let source = format!(
        r#"{PRELUDE}
FUNC main AS Integer
  MUT hosts AS String = ""
  FOR i = 1 TO cycles()
    RES s AS tls::Listener = tls::listen(os::getEnv("HOST"), 0, "{cert}", "{key}")
    LET a AS net::Address = tls::localAddress(s)
    hosts = hosts & a.host & " "
    tls::close(s)
  NEXT
  io::print(hosts)
  RETURN 0
END FUNC
"#,
        cert = common::mfb_path_literal(&cert),
        key = common::mfb_path_literal(&key),
    );
    let exe = build_debug("b984_close", &source);
    let out = assert_balanced("b984_named", &exe, &[("HOST", "127.0.0.1".to_string())]);
    assert_eq!(out.trim(), "127.0.0.1 127.0.0.1 127.0.0.1");
    // The bind-all spelling parks a copy of the static "0.0.0.0".
    let out = assert_balanced("b984_any", &exe, &[("HOST", String::new())]);
    assert_eq!(out.trim(), "0.0.0.0 0.0.0.0 0.0.0.0");
}

/// A listener nobody closes is closed by its scope's drop, which frees the same
/// blocks.
#[test]
fn a_listener_left_to_its_scope_frees_every_block_listen_allocated() {
    let root = std::env::temp_dir().join(format!("mfb_b984_{}", common::unique_nonce()));
    std::fs::create_dir_all(&root).expect("root");
    let (cert, key) = write_cert(&root);
    let source = format!(
        r#"{PRELUDE}
FUNC once() AS String
  RES s AS tls::Listener = tls::listen("127.0.0.1", 0, "{cert}", "{key}")
  LET a AS net::Address = tls::localAddress(s)
  RETURN a.host
END FUNC

FUNC main AS Integer
  MUT hosts AS String = ""
  FOR i = 1 TO cycles()
    hosts = hosts & once() & " "
  NEXT
  io::print(hosts)
  RETURN 0
END FUNC
"#,
        cert = common::mfb_path_literal(&cert),
        key = common::mfb_path_literal(&key),
    );
    let exe = build_debug("b984_drop", &source);
    let out = assert_balanced("b984_drop", &exe, &[]);
    assert_eq!(out.trim(), "127.0.0.1 127.0.0.1 127.0.0.1");
}

/// A `tls::listen` that fails builds no listener, so it must release what it had
/// allocated by then itself: an unreadable certificate (nothing but the path), a
/// file that is not a PEM (both buffers), and a host that does not resolve (both
/// buffers and the host block).
#[test]
fn a_failed_listen_frees_what_it_allocated() {
    let root = std::env::temp_dir().join(format!("mfb_b984_{}", common::unique_nonce()));
    std::fs::create_dir_all(&root).expect("root");
    let (cert, key) = write_cert(&root);
    let junk = root.join("junk.pem");
    std::fs::write(&junk, "not a certificate\n").expect("junk");
    let source = format!(
        r#"{PRELUDE}
FUNC attempt(host AS String, cert AS String) AS String
  RES s AS tls::Listener = tls::listen(host, 0, cert, "{key}") TRAP(e)
    RETURN "E" & toString(e.code)
  END TRAP
  RETURN "ok"
END FUNC

FUNC main AS Integer
  MUT seen AS String = ""
  FOR i = 1 TO cycles()
    seen = attempt("127.0.0.1", "{root}/missing.pem") & " " & attempt("127.0.0.1", "{junk}") & " " & attempt("no.such.host.invalid", "{cert}")
  NEXT
  io::print(seen)
  RETURN 0
END FUNC
"#,
        cert = common::mfb_path_literal(&cert),
        key = common::mfb_path_literal(&key),
        junk = common::mfb_path_literal(&junk),
        root = common::mfb_path_literal(&root),
    );
    let exe = build_debug("b984_fail", &source);
    let out = assert_balanced("b984_fail", &exe, &[]);
    let codes: Vec<&str> = out.split_whitespace().collect();
    assert_eq!(codes.len(), 3, "{out}");
    assert!(
        codes.iter().all(|code| code.starts_with('E')),
        "every attempt must fail: {out}"
    );
}

/// The control: the same loop over `tcp`.
#[test]
fn a_tcp_listener_is_the_flat_control() {
    let source = format!(
        r#"{PRELUDE}
FUNC main AS Integer
  FOR i = 1 TO cycles()
    RES s AS tcp::Listener = tcp::listen("127.0.0.1", 0)
    tcp::close(s)
  NEXT
  io::print("ok")
  RETURN 0
END FUNC
"#
    );
    let exe = build_debug("b984_tcp", &source);
    assert_eq!(assert_balanced("b984_tcp", &exe, &[]).trim(), "ok");
}
