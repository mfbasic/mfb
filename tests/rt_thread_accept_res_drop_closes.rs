//! bug-535, runtime half: a resource taken off a thread's resource channel is a
//! live handle on the receiving thread, and the scope drop that the compiler
//! refused to account for really closes it — exactly once.
//!
//! `validate_nir` used to reject `RES s AS tcp::Socket = thread::accept(t, ...)`
//! outright ("NIR declares unused runtime helper 'tcp'") because a plain
//! resource bind drops through a codegen-emitted close rather than an NIR call,
//! so nothing counted the package's helper as used. The accounting fix makes
//! that program build. That alone proves nothing about the program: a fix that
//! stops the error while handing the worker a dead handle would pass a
//! build-only test.
//!
//! So this drives the drop from outside the process. The host binds a listener,
//! the program connects to it and `thread::transfer`s the socket to a worker
//! whose body is the reproduction — accept, then let the binding fall out of
//! scope with no `tcp::` call of its own. The host then requires:
//!
//! * the accepted connection to reach end of stream, which can only be the
//!   worker's scope drop closing the handle;
//! * the program to still be **running** when that happens — it parks on
//!   `io::readLine` after joining the worker — so process exit cannot be
//!   mistaken for the close;
//! * a clean exit afterwards. A resource closed twice raises on the second
//!   close, so exit 0 with the worker's value on stdout is the "exactly once"
//!   half.
//!
//! `tests/cli_thread_accept_res_bind.rs` carries the build half (the per-package
//! sweep, the `STATE` form, and the alias-only pin).

mod common;

use std::io::{ErrorKind, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Wall-clock bound on each blocking interaction. Generous on purpose: these
/// turn a hang into a named failure rather than wedging `cargo test`, and are
/// not performance assertions.
const DEADLINE: Duration = Duration::from_secs(30);

/// What the worker returns once it has accepted the transferred socket. Seeing
/// it on stdout proves the accept completed rather than timing out.
const WORKER_RESULT: &str = "7";

fn temp_project(name: &str, source: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock before epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("mfb_{name}_{nonce}"));
    std::fs::create_dir_all(root.join("src")).expect("create temp project");
    std::fs::write(
        root.join("project.json"),
        "{\"name\":\"mfb_project\",\"version\":\"0.1.0\",\"mfb\":\"1.0\",\
         \"kind\":\"executable\",\
         \"sources\":[{\"root\":\"src\",\"role\":\"main\",\"include\":[\"**/*.mfb\"]}],\
         \"entry\":\"main\",\"targets\":[\"native\"]}\n",
    )
    .expect("write project.json");
    std::fs::write(root.join("src/main.mfb"), source).expect("write source");
    root
}

/// The program. `worker` is the bug-535 shape verbatim: its only reference to
/// `tcp` is the `RES` bind, and the binding's only consumer is the scope drop.
/// `main` connects and hands the socket over, then parks on standard input so
/// the drop is observable while the process is alive.
fn source(port: u16) -> String {
    format!(
        "IMPORT io\n\
         IMPORT thread\n\
         IMPORT tcp\n\n\
         ISOLATED FUNC worker(t AS ThreadWorker OF RES tcp::Socket TO Integer, n AS Integer) AS Integer\n\
        \x20 RES s AS tcp::Socket = thread::accept(t, 20000)\n\
        \x20 RETURN {WORKER_RESULT}\n\
         END FUNC\n\n\
         FUNC main AS Integer\n\
        \x20 LET a AS Thread OF RES tcp::Socket TO Integer = thread::start(worker, 0)\n\
        \x20 RES c AS tcp::Socket = tcp::connect(\"127.0.0.1\", {port}, 20000)\n\
        \x20 thread::transfer(a, c)\n\
        \x20 LET r AS Integer = thread::waitFor(a)\n\
        \x20 io::print(\"worker returned \" & toString(r))\n\
        \x20 LET line AS String = io::readLine()\n\
        \x20 io::print(\"bye\")\n\
        \x20 RETURN 0\n\
         END FUNC\n"
    )
}

fn build(project: &PathBuf) -> PathBuf {
    let output = Command::new(common::mfb_exe())
        .arg("build")
        .arg(project)
        .output()
        .expect("run mfb build");
    assert!(
        output.status.success(),
        "the bug-535 worker program must build:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    PathBuf::from(
        stdout
            .lines()
            .find_map(|line| line.strip_prefix("Wrote executable to "))
            .unwrap_or_else(|| panic!("no executable path in build output:\n{stdout}")),
    )
}

/// Kill `child` and panic with `message` — used for every failure past spawn so
/// a wedged program never outlives the test.
fn fail(child: &mut Child, message: String) -> ! {
    let _ = child.kill();
    let _ = child.wait();
    panic!("{message}");
}

#[test]
fn an_accepted_resource_is_live_and_its_scope_drop_closes_it() {
    let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .expect("bind a loopback listener");
    let port = listener.local_addr().expect("listener address").port();
    listener
        .set_nonblocking(false)
        .expect("blocking accept on the host listener");

    let exe = build(&temp_project("b535_drop_closes", &source(port)));

    let mut child = Command::new(&exe)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn the built program");

    // The program's `tcp::connect`. `accept` here is blocking; the build above
    // already happened, so the connect follows within milliseconds.
    let (mut peer, _addr): (TcpStream, _) = match listener.accept() {
        Ok(pair) => pair,
        Err(err) => fail(&mut child, format!("the program never connected: {err}")),
    };
    peer.set_read_timeout(Some(DEADLINE))
        .expect("set a read deadline on the accepted connection");

    // The proof. The worker's `RES s AS tcp::Socket` binding leaves scope the
    // instant `worker` returns, and its codegen-emitted close is the only thing
    // that can end this stream. A dead handle would close nothing and this read
    // would sit until the deadline.
    let mut buf = [0u8; 64];
    let closed = match peer.read(&mut buf) {
        Ok(0) => true,
        // A close can surface as a reset rather than an orderly FIN; both are
        // the socket going away, which is what is being asserted.
        Err(err) if matches!(err.kind(), ErrorKind::ConnectionReset) => true,
        Ok(n) => fail(
            &mut child,
            format!("the worker wrote {n} unexpected byte(s) instead of closing"),
        ),
        Err(err) => fail(
            &mut child,
            format!(
                "the accepted connection was never closed ({err}) — the resource the worker \
                 took off the thread channel is not a live handle, or its scope drop emitted \
                 no close"
            ),
        ),
    };
    assert!(closed);

    // ...and the program is still alive, parked on `io::readLine`. Without this
    // the assertion above would also pass if the socket had simply died with the
    // process.
    match child.try_wait().expect("poll the child") {
        None => {}
        Some(status) => fail(
            &mut child,
            format!(
                "the program had already exited ({status}) when the connection closed, so the \
                 close is not attributable to the worker's scope drop"
            ),
        ),
    }

    child
        .stdin
        .as_mut()
        .expect("piped stdin")
        .write_all(b"\n")
        .expect("release the parked program");

    let deadline = Instant::now() + DEADLINE;
    loop {
        match child.try_wait().expect("poll the child") {
            Some(_) => break,
            None if Instant::now() >= deadline => {
                fail(&mut child, "the program did not exit after stdin".to_string())
            }
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    }
    let output = child.wait_with_output().expect("collect program output");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        output.status.success(),
        "the program must exit cleanly — a second close of the transferred socket raises, so a \
         non-zero exit here is the 'exactly once' half failing.\nstatus: {:?}\nstdout:\n{stdout}\n\
         stderr:\n{stderr}",
        output.status,
    );
    assert!(
        stdout.contains(&format!("worker returned {WORKER_RESULT}")),
        "the worker must have accepted the transferred socket and returned normally;\
         \nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stdout.contains("bye"),
        "the program must run to completion;\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}
