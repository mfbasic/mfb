//! Regression test for bug-483 sub-issue C: the members overloaded on "is the
//! first argument a `net::Address`?" must still recognise one.
//!
//! `tcp::connect`, `tls::connect` and `net::ping` each have two argument shapes
//! that do NOT share a positional layout — `tcp::connect(host, port)` versus
//! `tcp::connect(address)`, and with a trailing `timeoutMs` on both. Arity alone
//! cannot separate the two-argument cases (`String, Integer` versus
//! `net::Address, Integer`), so `builder_values` picks the code form off the
//! first argument's **static type**.
//!
//! That question was asked by bare name. bug-480 Phase 4b package-qualified
//! builtin value types, so the argument started arriving as `net.Address`, the
//! test answered `false`, and `tcp::connect(bound, 5000)` lowered through the
//! `host, port` form — which reads argument 0 as a `String` block. It is a
//! record, so the length word is a pointer and the connect died with `SIGSEGV`.
//!
//! **The one-argument form cannot catch this** and neither can a compile: arity
//! decides the single-argument case unconditionally, so `tcp::connect(bound)`
//! kept working throughout, and both shapes type-check either way — only the
//! emitted body differs. The two-argument form has to actually run.
//!
//! Loopback and plaintext, so there is no certificate and no peer to arrange:
//! the program listens, reads its own bound address back, and dials it.

#![cfg(any(target_os = "macos", target_os = "linux"))]

mod common;

use std::process::Command;

const SOURCE: &str = r#"IMPORT io
IMPORT net
IMPORT tcp

FUNC main AS Integer
  RES server = tcp::listen("127.0.0.1", 0)
  LET bound = tcp::localAddress(server)

  ' Selected by ARITY - this form never regressed.
  RES one = tcp::connect(bound)
  io::print("address form ok")
  tcp::close(one)

  ' Selected by the first argument's STATIC TYPE - this is the regressed one.
  RES two = tcp::connect(bound, 5000)
  io::print("address+timeout form ok")
  tcp::close(two)

  RETURN 0
END FUNC
"#;

#[test]
fn the_two_argument_address_overload_still_selects_the_address_form() {
    let project = common::temp_project("bug483_address_overload", SOURCE);
    let exe = common::build_project(&project);

    let output = Command::new(&exe).output().expect("run the overload probe");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    assert!(
        output.status.success(),
        "bug-483: `tcp::connect(address, timeoutMs)` did not select the Address \
         code form (status {:?}) — it lowered through `host, port` and read the \
         record as a String.\nstdout:\n{stdout}\nstderr:\n{stderr}",
        output.status
    );
    assert!(
        stdout.contains("address+timeout form ok"),
        "bug-483: the two-argument Address overload never completed.\n\
         stdout:\n{stdout}\nstderr:\n{stderr}"
    );

    let _ = std::fs::remove_dir_all(&project);
}
