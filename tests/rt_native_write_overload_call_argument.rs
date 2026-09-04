//! bug-476: an overloaded native member whose *code form* is chosen at codegen
//! from an argument's static type must resolve that type when the argument is a
//! CALL RESULT.
//!
//! Several members collapse two overloads into one name and pick the lowering in
//! `builder_values::lower_runtime_helper_call` from an argument's static type:
//! `tcp::write`/`tls::write`/`udp::send` (bytes vs text),
//! `tcp::connect`/`tls::connect` (host/port vs `net::Address`), `net::ping`,
//! `tcp`/`udp`/`tls` `poll` (scalar vs list), `tls::localAddress`
//! (`Socket` vs `Listener`), and `io::print`/`io::write`'s `AttributedString`
//! rewrite. The probe was `CodeBuilder::static_type_name`, whose `NirValue::Call`
//! arm is a hand-written table of a dozen builtins — **every other call answered
//! `None`**, so each of those selectors silently took its fallback form whenever
//! the deciding argument was a call result.
//!
//! For `tcp::write` that meant `tcp::write(sock, buildHead(x))` marshalling a
//! `String*` through the collection path: a garbage element count out of the
//! string's data word, a failed `write(2)`, and `ErrConnectionClosed` (77070004)
//! raised with nothing on the wire. Binding the same expression to a `LET` first
//! worked, which is what made it look like a transport fault. That is the defect
//! `http::handleRequest` tripped over (see `rt_http_handle_request_serves.rs`).
//!
//! Coverage here is one representative per *selector shape*, since all of them
//! read the same one-line probe:
//!
//! | shape                        | pinned by                                    |
//! |------------------------------|----------------------------------------------|
//! | bytes vs text                | `tcp::write` (both forms) and `udp::send`    |
//! | scalar vs list               | `tcp::poll(list, timeoutMs)`                 |
//! | `AttributedString` rewrite   | `io::print(astrings::fromString(…))`         |
//!
//! Those four were each verified RED on the unfixed compiler.
//!
//! `tcp::connect`'s host/port-vs-`net::Address` shape is exercised below but did
//! NOT reproduce: a record-returning call is spilled to a temporary before the
//! selector runs, so it already sees a `Local` and resolves. It is kept as a
//! guard on a selector this change touched, not as a regression witness.
//!
//! Not pinned here at all, and sharing the identical probe: `tls::write`,
//! `tls::connect`, `tls::localAddress`, `tls::poll` (all need a TLS identity) and
//! `net::ping` (needs raw-socket permission).
//!
//! Every exchange is in-process over loopback with a port-0 bind, so nothing here
//! can collide with a concurrently running sibling test.

mod common;

use std::time::Duration;

/// Run `source` and return its stdout, failing the test on a non-zero exit or a
/// hang (both are how this bug shows up: nothing is written, so the peer's read
/// never completes).
fn run(name: &str, source: &str) -> String {
    let project = common::temp_project(name, source);
    let exe = common::build_project(&project);
    let (status, stdout) = common::run_bounded(
        &exe,
        Duration::from_secs(30),
        "bug-476: the peer's read never completed because the wrong code form was selected",
    );
    assert!(
        status.success(),
        "{name} exited non-zero: {status:?}\nstdout:\n{stdout}"
    );
    assert!(
        !stdout.contains("raised"),
        "{name}: nothing here may raise; got:\n{stdout}"
    );
    let _ = std::fs::remove_dir_all(&project);
    stdout
}

/// `tcp::write` — a String produced by a user FUNC must take the text form, and a
/// `List OF Byte` produced by a user FUNC must still take the bytes form. Both
/// have to arrive verbatim.
const WRITE_SOURCE: &str = r#"IMPORT tcp
IMPORT net
IMPORT strings
IMPORT io
IMPORT collections

FUNC head(n AS Integer) AS String
  RETURN "value=" & toString(n) & ";"
END FUNC

FUNC payload() AS List OF Byte
  RETURN strings::toBytes("bytes!")
END FUNC

FUNC main AS Integer
  RES server AS tcp::Listener = tcp::listen("127.0.0.1", 0, 8)
  LET bound AS net::Address = tcp::localAddress(server)
  RES client AS tcp::Socket = tcp::connect("127.0.0.1", bound.port)
  RES conn AS tcp::Socket = tcp::accept(server)
  tcp::write(conn, head(7)) TRAP(e)
    io::print("text write raised " & toString(e.code))
    RECOVER
  END TRAP
  tcp::write(conn, payload()) TRAP(e)
    io::print("bytes write raised " & toString(e.code))
    RECOVER
  END TRAP
  tcp::close(conn)
  ' Two separate writes are two segments: a single read returns whichever
  ' happened to arrive, so this drained only "value=7;" about two runs in three.
  ' Drain to EOF instead of reading once.
  MUT got AS List OF Byte = []
  MUT open AS Boolean = TRUE
  WHILE open
    MUT chunk AS List OF Byte = []
    chunk = tcp::read(client, 4096) TRAP(e)
      open = FALSE
      RECOVER []
    END TRAP
    IF len(chunk) = 0 THEN
      open = FALSE
    END IF
    got = collections::append(got, chunk)
  END WHILE
  io::print("got=" & toString(got))
  RETURN 0
END FUNC
"#;

#[test]
fn tcp_write_selects_the_text_form_for_a_call_returned_string() {
    let stdout = run("b476_tcp_write", WRITE_SOURCE);
    assert!(
        stdout.contains("got=value=7;bytes!"),
        "expected both call-returned payloads on the wire verbatim; got:\n{stdout}"
    );
}

/// `udp::send` — the same bytes-vs-text split, one argument further along
/// (socket, address, payload), so it also pins that the *right* argument is read.
const UDP_SOURCE: &str = r#"IMPORT udp
IMPORT net
IMPORT io

FUNC greeting(n AS Integer) AS String
  RETURN "udp=" & toString(n)
END FUNC

FUNC main AS Integer
  RES rx AS udp::Socket = udp::bind("127.0.0.1", 0)
  LET at AS net::Address = udp::localAddress(rx)
  RES tx AS udp::Socket = udp::bind("127.0.0.1", 0)
  udp::send(tx, at, greeting(3)) TRAP(e)
    io::print("udp send raised " & toString(e.code))
    RECOVER
  END TRAP
  LET d AS udp::Datagram = udp::receive(rx, 4096) TRAP(e)
    io::print("udp receive raised " & toString(e.code))
    RECOVER udp::Datagram[at, []]
  END TRAP
  io::print("got=" & toString(d.bytes))
  RETURN 0
END FUNC
"#;

#[test]
fn udp_send_selects_the_text_form_for_a_call_returned_string() {
    let stdout = run("b476_udp_send", UDP_SOURCE);
    assert!(
        stdout.contains("got=udp=3"),
        "expected the call-returned datagram payload verbatim; got:\n{stdout}"
    );
}

/// `tcp::connect` — the two-argument `(Address, timeoutMs)` form, where the
/// endpoint is a call result. Measured GREEN on the unfixed compiler too: a
/// record-returning call is spilled to a temporary first, so the selector sees a
/// `Local`. Kept as a guard on the `net_connect_is_address_form` probe this
/// change rewires, not as a witness for bug-476.
const CONNECT_SOURCE: &str = r#"IMPORT tcp
IMPORT net
IMPORT io

FUNC endpoint(RES l AS tcp::Listener) AS net::Address
  RETURN tcp::localAddress(l)
END FUNC

FUNC main AS Integer
  RES server AS tcp::Listener = tcp::listen("127.0.0.1", 0, 8)
  RES client AS tcp::Socket = tcp::connect(endpoint(server), 5000) TRAP(e)
    io::print("connect raised " & toString(e.code))
    PROPAGATE
  END TRAP
  RES conn AS tcp::Socket = tcp::accept(server)
  tcp::write(conn, "ok")
  tcp::close(conn)
  MUT got AS List OF Byte = []
  got = tcp::read(client, 64) TRAP(e)
    io::print("read raised " & toString(e.code))
    RECOVER []
  END TRAP
  io::print("got=" & toString(got))
  RETURN 0
END FUNC
"#;

#[test]
fn tcp_connect_selects_the_address_form_for_a_call_returned_address() {
    let stdout = run("b476_tcp_connect", CONNECT_SOURCE);
    assert!(
        stdout.contains("got=ok"),
        "expected the Address-form connect to reach the listener; got:\n{stdout}"
    );
}

/// `tcp::poll` — the list form. This one is doubly load-bearing: the same probe
/// picks the `tcp.pollList` lowering *and* the call's return type (a borrowed
/// `Socket` for the list form, `Boolean` for the scalar one), so an unresolved
/// argument type mis-typed the result as well as mis-selecting the helper.
const POLL_SOURCE: &str = r#"IMPORT tcp
IMPORT net
IMPORT io

FUNC watched(RES a AS tcp::Socket) AS List OF RES tcp::Socket
  RETURN [a]
END FUNC

FUNC main AS Integer
  RES server AS tcp::Listener = tcp::listen("127.0.0.1", 0, 8)
  LET bound AS net::Address = tcp::localAddress(server)
  RES client AS tcp::Socket = tcp::connect("127.0.0.1", bound.port)
  RES conn AS tcp::Socket = tcp::accept(server)
  tcp::write(client, "ping")
  RES ready AS tcp::Socket = tcp::poll(watched(conn), 5000) TRAP(e)
    io::print("poll raised " & toString(e.code))
    PROPAGATE
  END TRAP
  MUT got AS List OF Byte = []
  got = tcp::read(ready, 64) TRAP(e)
    io::print("read raised " & toString(e.code))
    RECOVER []
  END TRAP
  io::print("got=" & toString(got))
  RETURN 0
END FUNC
"#;

#[test]
fn tcp_poll_selects_the_list_form_for_a_call_returned_list() {
    let stdout = run("b476_tcp_poll", POLL_SOURCE);
    assert!(
        stdout.contains("got=ping"),
        "expected the list-form poll to return the ready socket; got:\n{stdout}"
    );
}

/// `io::print` — the `AttributedString` argument is rewritten to `toString(a)`
/// before the writer path runs. `astrings::fromString` is not in the hand-written
/// table either, so printing its result directly handed the writer a raw
/// `AttributedString` block instead of its visible text.
const ASTRINGS_SOURCE: &str = r#"IMPORT astrings
IMPORT io

FUNC main AS Integer
  io::print("got=" & toString(astrings::fromString("attr")))
  io::write("direct=")
  io::print(astrings::fromString("attr"))
  RETURN 0
END FUNC
"#;

#[test]
fn io_print_rewrites_a_call_returned_attributed_string() {
    let stdout = run("b476_io_print_attr", ASTRINGS_SOURCE);
    assert!(
        stdout.contains("got=attr") && stdout.contains("direct=attr"),
        "expected the visible text of a call-returned AttributedString; got:\n{stdout}"
    );
}
