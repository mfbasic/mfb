//! bug-497: a `String` payload that is a call THROUGH a FUNC-typed value, or a
//! field read off a call result, must reach the socket as exactly its bytes.
//!
//! `tcp::write` is one name with two lowerings chosen from the payload's static
//! type. The first fix for the call-result case (bug-476/bug-483) typed a call to
//! a NAMED function; these two shapes were measured still selecting the
//! `List OF Byte` form afterwards (`tests/codegen_net_write_payload_view.rs`
//! pins the selection itself). Read through the byte layout, a `String`'s first
//! eight bytes become the write length and the bytes start 40 past the header —
//! against a network peer, a peer-chosen amount of process memory goes back on
//! the wire (`spikes/audit-3/OS-50`). This is the end-to-end witness: the peer
//! must receive precisely the three payloads, nothing more.
//!
//! In-process over loopback with a port-0 bind, so it cannot collide with a
//! sibling test. The reader drains to EOF, because three small writes may arrive
//! in more than one segment.

mod common;

use std::time::Duration;

const SOURCE: &str = r#"IMPORT tcp
IMPORT net
IMPORT collections
IMPORT io

TYPE Rec
  body AS String
END TYPE

FUNC head(n AS Integer) AS String
  RETURN "value=" & toString(n) & ";"
END FUNC

FUNC makeRec() AS Rec
  RETURN Rec["field;"]
END FUNC

FUNC main AS Integer
  RES server AS tcp::Listener = tcp::listen("127.0.0.1", 0, 8)
  LET bound AS net::Address = tcp::localAddress(server)
  RES client AS tcp::Socket = tcp::connect("127.0.0.1", bound.port)
  RES conn AS tcp::Socket = tcp::accept(server)
  LET f AS FUNC(Integer) AS String = head
  tcp::write(conn, head(7)) TRAP(e)
    io::print("named write raised " & toString(e.code))
    RECOVER
  END TRAP
  tcp::write(conn, f(8)) TRAP(e)
    io::print("func-value write raised " & toString(e.code))
    RECOVER
  END TRAP
  tcp::write(conn, makeRec().body) TRAP(e)
    io::print("field write raised " & toString(e.code))
    RECOVER
  END TRAP
  tcp::close(conn)
  MUT got AS List OF Byte = []
  MUT open AS Boolean = TRUE
  WHILE open
    MUT chunk AS List OF Byte = []
    chunk = tcp::read(client, 4096) TRAP(e)
      open = FALSE
      RECOVER []
    END TRAP
    got = collections::append(got, chunk)
  END WHILE
  io::print("got=" & toString(got) & "|len=" & toString(len(got)))
  RETURN 0
END FUNC
"#;

#[test]
fn string_payloads_from_a_func_value_and_a_call_field_arrive_verbatim() {
    let project = common::temp_project("b497_write_shapes", SOURCE);
    let exe = common::build_project(&project);
    let (status, stdout) = common::run_bounded(
        &exe,
        Duration::from_secs(30),
        "bug-497: the reader never reached EOF — a write took the wrong form",
    );
    assert!(
        status.success(),
        "exited non-zero: {status:?}\nstdout:\n{stdout}"
    );
    assert!(
        !stdout.contains("raised"),
        "no write here may raise; got:\n{stdout}"
    );
    // 8 + 8 + 6 bytes: exactly the three payloads, no header bytes and no
    // process memory.
    assert!(
        stdout.contains("got=value=7;value=8;field;|len=22"),
        "expected exactly the three String payloads on the wire; got:\n{stdout}"
    );
    let _ = std::fs::remove_dir_all(&project);
}
