//! Regression test for bug-702: an operation on a handle that `thread::transfer`
//! moved to another thread reports `ErrResourceMoved`, not `ErrResourceClosed`.
//!
//! `mfb spec language resource-management` §15: "where a handle nonetheless
//! reaches an operation moved (an alias those rules do not track), the operation
//! is refused with `ErrResourceMoved` rather than `ErrResourceClosed`". A `RES`
//! parameter is such an alias: the callee transfers it, and the caller's binding
//! stays usable to the compiler. Before the fix only the `close` members split
//! the two; every other guard tested the flag word for non-zero and raised
//! `ErrResourceClosed` for a moved handle too.
//!
//! One case per guarded helper family: every `File` member, the `tcp` listener
//! address and accept helpers, the `udp` timeout and poll helpers, a `Thread`
//! view, and (on hosts with an `openssl` CLI to mint a certificate) a
//! `tls::Listener`.

#[path = "../common/mod.rs"]
mod common;

use std::fs;

/// `ErrResourceMoved`, `7-703-0009`.
const MOVED: &str = "77030009";

#[test]
fn file_socket_and_thread_operations_report_a_moved_handle() {
    let dir = std::env::temp_dir().join(format!("mfb_bug702_{}", common::unique_nonce()));
    fs::create_dir_all(&dir).expect("create fixture dir");
    let data = dir.join("data.txt");
    fs::write(&data, "hello\nworld\n").expect("write fixture");
    let path = common::mfb_path_literal(&data);
    let source = format!(
        r#"IMPORT io
IMPORT fs
IMPORT tcp
IMPORT udp
IMPORT thread

ISOLATED FUNC takeFile(t AS ThreadWorker OF RES fs::File TO Integer, n AS Integer) AS Integer
  RES g AS fs::File = thread::accept(t, 5000)
  RETURN 1
END FUNC

ISOLATED FUNC takeListener(t AS ThreadWorker OF RES tcp::Listener TO Integer, n AS Integer) AS Integer
  RES g AS tcp::Listener = thread::accept(t, 5000)
  RETURN 2
END FUNC

ISOLATED FUNC takeUdp(t AS ThreadWorker OF RES udp::Socket TO Integer, n AS Integer) AS Integer
  RES g AS udp::Socket = thread::accept(t, 5000)
  RETURN 3
END FUNC

ISOLATED FUNC idle(t AS ThreadWorker OF String TO Integer, n AS Integer) AS Integer
  RETURN 4
END FUNC

ISOLATED FUNC takeThread(t AS ThreadWorker OF RES Thread OF String TO Integer TO Integer, n AS Integer) AS Integer
  RES g AS Thread OF String TO Integer = thread::accept(t, 5000)
  RETURN thread::waitFor(g)
END FUNC

FUNC giveFile(RES x AS fs::File, t AS Thread OF RES fs::File TO Integer) AS Nothing
  thread::transfer(t, x, 5000)
END FUNC

FUNC giveListener(RES x AS tcp::Listener, t AS Thread OF RES tcp::Listener TO Integer) AS Nothing
  thread::transfer(t, x, 5000)
END FUNC

FUNC giveUdp(RES x AS udp::Socket, t AS Thread OF RES udp::Socket TO Integer) AS Nothing
  thread::transfer(t, x, 5000)
END FUNC

FUNC giveThread(RES x AS Thread OF String TO Integer, t AS Thread OF RES Thread OF String TO Integer TO Integer) AS Nothing
  thread::transfer(t, x, 5000)
END FUNC

FUNC probeFile() AS Nothing
  RES tf AS Thread OF RES fs::File TO Integer = thread::start(takeFile, 0)
  RES f AS fs::File = fs::openFile("{path}", "r")
  giveFile(f, tf)
  LET a = fs::readLine(f) TRAP(e)
    io::print("readLine " & toString(e.code))
    RECOVER ""
  END TRAP
  LET b = fs::readAll(f) TRAP(e)
    io::print("readAll " & toString(e.code))
    RECOVER ""
  END TRAP
  LET c = fs::readAllBytes(f) TRAP(e)
    io::print("readAllBytes " & toString(e.code))
    RECOVER []
  END TRAP
  LET d = fs::readBytesAt(f, 0, 2) TRAP(e)
    io::print("readBytesAt " & toString(e.code))
    RECOVER []
  END TRAP
  fs::writeAll(f, "x") TRAP(e)
    io::print("writeAll " & toString(e.code))
    RECOVER
  END TRAP
  fs::writeAllBytes(f, c) TRAP(e)
    io::print("writeAllBytes " & toString(e.code))
    RECOVER
  END TRAP
  LET g = fs::eof(f) TRAP(e)
    io::print("eof " & toString(e.code))
    RECOVER FALSE
  END TRAP
  LET h = fs::size(f) TRAP(e)
    io::print("size " & toString(e.code))
    RECOVER 0
  END TRAP
  fs::flush(f) TRAP(e)
    io::print("flush " & toString(e.code))
    RECOVER
  END TRAP
  fs::setBuffered(f, TRUE) TRAP(e)
    io::print("setBuffered " & toString(e.code))
    RECOVER
  END TRAP
  LET i = fs::isBuffered(f) TRAP(e)
    io::print("isBuffered " & toString(e.code))
    RECOVER FALSE
  END TRAP
  fs::close(f) TRAP(e)
    io::print("close " & toString(e.code))
    RECOVER
  END TRAP
  io::print("file worker " & toString(thread::waitFor(tf)))
END FUNC

FUNC probeListenerAddress() AS Nothing
  RES tl AS Thread OF RES tcp::Listener TO Integer = thread::start(takeListener, 0)
  RES l = tcp::listen("127.0.0.1", 0)
  giveListener(l, tl)
  LET addr = tcp::localAddress(l) TRAP(e)
    io::print("tcp localAddress " & toString(e.code))
    RETURN
  END TRAP
END FUNC

FUNC probeListenerAccept() AS Nothing
  RES tl AS Thread OF RES tcp::Listener TO Integer = thread::start(takeListener, 0)
  RES l = tcp::listen("127.0.0.1", 0)
  giveListener(l, tl)
  RES s = tcp::accept(l, 10) TRAP(e)
    io::print("tcp accept " & toString(e.code))
    RETURN
  END TRAP
END FUNC

FUNC probeUdp() AS Nothing
  RES tu AS Thread OF RES udp::Socket TO Integer = thread::start(takeUdp, 0)
  RES u = udp::bind("127.0.0.1", 0)
  giveUdp(u, tu)
  udp::setReadTimeout(u, 10) TRAP(e)
    io::print("udp setReadTimeout " & toString(e.code))
    RECOVER
  END TRAP
  LET ready = udp::poll(u, 0) TRAP(e)
    io::print("udp poll " & toString(e.code))
    RECOVER FALSE
  END TRAP
  io::print("udp worker " & toString(thread::waitFor(tu)))
END FUNC

FUNC probeThread() AS Nothing
  RES tt AS Thread OF RES Thread OF String TO Integer TO Integer = thread::start(takeThread, 0)
  RES inner AS Thread OF String TO Integer = thread::start(idle, 0)
  giveThread(inner, tt)
  LET running = thread::isRunning(inner) TRAP(e)
    io::print("thread isRunning " & toString(e.code))
    RECOVER FALSE
  END TRAP
  io::print("thread worker " & toString(thread::waitFor(tt)))
END FUNC

FUNC main() AS Nothing
  probeFile()
  probeListenerAddress()
  probeListenerAccept()
  probeUdp()
  probeThread()
END FUNC
"#
    );
    let project = common::temp_project("bug702_moved_guards", &source);
    let executable = common::build_project(&project);
    let output = common::run_with_stdin(&executable, b"");
    let expected: Vec<String> = [
        "readLine",
        "readAll",
        "readAllBytes",
        "readBytesAt",
        "writeAll",
        "writeAllBytes",
        "eof",
        "size",
        "flush",
        "setBuffered",
        "isBuffered",
        "close",
    ]
    .iter()
    .map(|op| format!("{op} {MOVED}"))
    .chain(["file worker 1".to_string()])
    .chain([
        format!("tcp localAddress {MOVED}"),
        format!("tcp accept {MOVED}"),
        format!("udp setReadTimeout {MOVED}"),
        format!("udp poll {MOVED}"),
        "udp worker 3".to_string(),
        format!("thread isRunning {MOVED}"),
        "thread worker 4".to_string(),
    ])
    .collect();
    assert_eq!(
        output.lines().collect::<Vec<_>>(),
        expected,
        "every operation on a moved handle must raise ErrResourceMoved"
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&project);
}

/// The `tls::Listener` helpers (accept, address, close) have their own guards on
/// every TLS backend. A listener needs a certificate, minted here as the sibling
/// `tests/net/rt_tls_listener_*` tests do; Windows is excluded for the same
/// reason they are (the `openssl` CLI is not a dependable presence there).
#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn tls_listener_operations_report_a_moved_handle() {
    use std::process::{Command, Stdio};
    let have_openssl = Command::new("openssl")
        .arg("version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !have_openssl {
        eprintln!("skipping: openssl CLI not available");
        return;
    }
    let dir = std::env::temp_dir().join(format!("mfb_bug702_tls_{}", common::unique_nonce()));
    fs::create_dir_all(&dir).expect("create fixture dir");
    let (cert, key) = (dir.join("cert.pem"), dir.join("key.pem"));
    let status = Command::new("openssl")
        .args(["req", "-x509", "-newkey", "rsa:2048", "-keyout"])
        .arg(&key)
        .arg("-out")
        .arg(&cert)
        .args(["-days", "2", "-nodes", "-subj", "/CN=127.0.0.1"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("run openssl req");
    assert!(status.success(), "openssl failed to generate a certificate");
    let (cert, key) = (
        common::mfb_path_literal(&cert),
        common::mfb_path_literal(&key),
    );
    let source = format!(
        r#"IMPORT io
IMPORT tls
IMPORT thread

ISOLATED FUNC takeListener(t AS ThreadWorker OF RES tls::Listener TO Integer, n AS Integer) AS Integer
  RES g AS tls::Listener = thread::accept(t, 5000)
  RETURN 5
END FUNC

FUNC giveListener(RES x AS tls::Listener, t AS Thread OF RES tls::Listener TO Integer) AS Nothing
  thread::transfer(t, x, 5000)
END FUNC

FUNC probeAddress() AS Nothing
  RES tl AS Thread OF RES tls::Listener TO Integer = thread::start(takeListener, 0)
  RES l = tls::listen("127.0.0.1", 0, "{cert}", "{key}")
  giveListener(l, tl)
  LET addr = tls::localAddress(l) TRAP(e)
    io::print("tls localAddress " & toString(e.code))
    RETURN
  END TRAP
END FUNC

FUNC probeAccept() AS Nothing
  RES tl AS Thread OF RES tls::Listener TO Integer = thread::start(takeListener, 0)
  RES l = tls::listen("127.0.0.1", 0, "{cert}", "{key}")
  giveListener(l, tl)
  RES s = tls::accept(l, 10) TRAP(e)
    io::print("tls accept " & toString(e.code))
    RETURN
  END TRAP
END FUNC

FUNC probeClose() AS Nothing
  RES tl AS Thread OF RES tls::Listener TO Integer = thread::start(takeListener, 0)
  RES l = tls::listen("127.0.0.1", 0, "{cert}", "{key}")
  giveListener(l, tl)
  tls::close(l) TRAP(e)
    io::print("tls close " & toString(e.code))
    RECOVER
  END TRAP
  io::print("tls worker " & toString(thread::waitFor(tl)))
END FUNC

FUNC main() AS Nothing
  probeAddress()
  probeAccept()
  probeClose()
END FUNC
"#
    );
    let project = common::temp_project("bug702_moved_tls", &source);
    let executable = common::build_project(&project);
    let output = common::run_with_stdin(&executable, b"");
    assert_eq!(
        output.lines().collect::<Vec<_>>(),
        vec![
            format!("tls localAddress {MOVED}"),
            format!("tls accept {MOVED}"),
            format!("tls close {MOVED}"),
            "tls worker 5".to_string(),
        ],
        "every operation on a moved tls::Listener must raise ErrResourceMoved"
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&project);
}
