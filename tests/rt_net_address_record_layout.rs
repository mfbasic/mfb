//! Regression test for bug-483: every `net::Address`-valued endpoint query, and
//! the `udp::Datagram` that nests one, must come back with a *readable* `host`.
//!
//! ## What broke
//!
//! `net::Address`, `udp::Datagram` and `audio::AudioDevice` are the three records
//! built by bespoke runtime helpers rather than by the codegen `Constructor`
//! path, and those helpers write a `String` field as an **absolute pointer** to a
//! separate allocation. Every other record inlines its `String` blocks into a
//! trailing data region and stores a **block-relative offset** in the slot. The
//! two layouts are distinguished by exactly one predicate,
//! `builder_collection_layout::is_pointer_string_record`, which matched the
//! record's name.
//!
//! bug-480 Phase 4b made a builtin value type's declared identity
//! package-qualified (`Address` -> `net.Address`), so that name match stopped
//! firing. Readers then took the absolute pointer the helper had written as an
//! offset from the record's own base, producing a wild pointer: reading
//! `addr.host` died with `SIGSEGV`, or silently reported an empty/garbage host
//! when the arithmetic happened to land on mapped memory.
//!
//! ## Why a run, and why all four members in one program
//!
//! Nothing about this is visible at compile time — the program builds, and the
//! *port* (an ordinary `Integer` slot) reads back correctly, which is what made
//! the original report look like a TLS listener bug. Only reading the `String`
//! field of a value a helper produced shows it, so the test runs the program and
//! requires the host text.
//!
//! The four members are one program deliberately: they reach the shared layout
//! predicate by four different routes — a bare returned record
//! (`tcp::localAddress`), a `List OF` it (`net::lookup`), the same record under
//! another package's member (`udp::localAddress`), and a *nested* one
//! (`udp::Datagram.from`). A fix that repaired only the direct return would still
//! leave the list and the nested field broken, and one process proves all four
//! for the cost of one build.
//!
//! Loopback only — no DNS, no peer, no certificate — so it is deterministic and
//! needs no network. macOS and Linux; Windows is excluded only because these
//! endpoint queries are exercised there by `cli_*` build tests rather than by a
//! run, matching the sibling `rt_tls_listener_local_address`.

#![cfg(any(target_os = "macos", target_os = "linux"))]

mod common;

use std::process::Command;

/// Bind loopback ports, read every address-valued endpoint query back, and print
/// each `host` so the harness can require the text.
const SOURCE: &str = r#"IMPORT io
IMPORT net
IMPORT tcp
IMPORT udp

FUNC main AS Integer
  ' A List OF net::Address, built by the lookup helper.
  LET found = net::lookup("127.0.0.1", 80)
  FOR EACH a IN found
    io::print("lookup " & a.host & " " & toString(a.port))
  NEXT

  ' A bare returned net::Address.
  RES server = tcp::listen("127.0.0.1", 0)
  LET bound = tcp::localAddress(server)
  io::print("tcp " & bound.host & " " & toString(bound.port > 0))

  ' The same record reached through another package's member...
  RES sock = udp::bind("127.0.0.1", 0)
  LET at = udp::localAddress(sock)
  io::print("udp " & at.host & " " & toString(at.port > 0))

  ' ...and nested inside udp::Datagram, which is itself pointer-string.
  RES peer = udp::bind("127.0.0.1", 0)
  udp::send(peer, at, "ping")
  LET dg = udp::receive(sock, 5000)
  io::print("datagram " & dg.from.host & " " & toString(len(dg.bytes)))

  RETURN 0
END FUNC
"#;

#[test]
fn address_valued_endpoint_queries_report_a_readable_host() {
    let project = common::temp_project("bug483_address_layout", SOURCE);
    let exe = common::build_project(&project);

    let output = Command::new(&exe).output().expect("run the address probe");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    assert!(
        output.status.success(),
        "bug-483: reading `host` off a helper-built net::Address crashed \
         (status {:?}). The record's String fields are absolute pointers, so a \
         reader that treats the slot as a block-relative offset dereferences \
         garbage.\nstdout:\n{stdout}\nstderr:\n{stderr}",
        output.status
    );

    // Each line names which route produced the address, so a partial fix says
    // which one it missed rather than just "wrong output".
    for expected in [
        "lookup 127.0.0.1 80",
        "tcp 127.0.0.1 TRUE",
        "udp 127.0.0.1 TRUE",
        "datagram 127.0.0.1 4",
    ] {
        assert!(
            stdout.lines().any(|line| line == expected),
            "bug-483: expected a line {expected:?}; a wrong or empty host here \
             means that route still reads the String slot as an offset.\n\
             stdout:\n{stdout}\nstderr:\n{stderr}"
        );
    }

    let _ = std::fs::remove_dir_all(&project);
}
