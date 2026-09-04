//! bug-497 / bug-508: the bytes-vs-text lowering of `tcp::write`, `udp::send`
//! and `tls::write` must be chosen by the payload's TYPE, and the byte form
//! must verify it was handed a collection block before it reads a length.
//!
//! The two payload layouts differ — a `String` is `[u64 len][bytes]`, a
//! `List OF Byte` is a collection block with `count` at +8 and its bytes past
//! the header — so a mis-selected form is an out-of-bounds read whose length
//! the payload's own first bytes dictate. Over a socket those bytes are the
//! peer's: `spikes/audit-3/OS-50` sent 22 bytes to an echo server built on
//! `tcp::write(sock, reply(txt))` and read back 1024 bytes of process memory,
//! because the selector could not type a call result and fell OPEN to the byte
//! form. On Windows the Schannel `tls::write` ignored the form altogether and
//! read a byte list as a `String` (bug-508).
//!
//! Two invariants, each pinned against the `-ncode` dump (execution-free, so
//! every target is checked on this host):
//!
//! * [`write_form_is_selected_by_the_payload_type_for_every_call_shape`] — the
//!   call site takes the text form for a `String` however it is produced (a
//!   named FUNC, a FUNC-typed value, a field of a call result) and the byte form
//!   for a `List OF Byte`. Measured RED on the unfixed compiler for the named
//!   FUNC, and RED after the first (named-function-only) fix for the other two.
//! * [`byte_form_helpers_verify_the_collection_header_before_reading_a_length`] —
//!   every byte-form runtime helper reads the payload's `kind`/`keyType`/
//!   `valueType` header bytes and branches to `<sym>_bad_payload` on a mismatch,
//!   and takes its length from the collection `count` (+8), never from +0; the
//!   text form reads +0 and never touches header bytes. Windows `tls::write` is
//!   the bug-508 witness: it read +0 for both forms.

mod common;

use common::{build_ncode, temp_project};
use serde_json::Value;

/// Every write member, both forms, with the three call shapes the selector had
/// to learn to type. `tcp::connect` to port 1 is never executed — the dump is
/// execution-free — it only puts a `Socket` in scope.
const SOURCE: &str = r#"IMPORT tcp
IMPORT udp
IMPORT tls
IMPORT net
IMPORT strings

TYPE Rec
  body AS String
END TYPE

TYPE Pend
  raw AS List OF Byte
  note AS String
END TYPE

FUNC head(n AS Integer) AS String
  RETURN "value=" & toString(n) & ";"
END FUNC

FUNC makeRec() AS Rec
  RETURN Rec["b"]
END FUNC

FUNC main() AS Integer
  RES sock AS tcp::Socket = tcp::connect("127.0.0.1", 1)
  LET f AS FUNC(Integer) AS String = head
  LET raw AS List OF Byte = strings::toBytes("bytes")
  tcp::write(sock, head(7))
  tcp::write(sock, f(8))
  tcp::write(sock, makeRec().body)
  tcp::write(sock, raw)
  RES st AS tcp::Socket STATE Pend = tcp::connect("127.0.0.1", 2)
  tcp::write(st, st.state.raw)
  tcp::write(st, st.state.note)
  RES u AS udp::Socket = udp::bind("127.0.0.1", 0)
  LET at AS net::Address = udp::localAddress(u)
  udp::send(u, at, head(1))
  udp::send(u, at, raw)
  RES t AS tls::Socket = tls::connect("example.com", 443)
  tls::write(t, head(2))
  tls::write(t, raw)
  RETURN 0
END FUNC
"#;

const TARGETS: [&str; 5] = [
    "macos-aarch64",
    "linux-aarch64",
    "linux-x86_64",
    "linux-riscv64",
    "windows-x86_64",
];

const BYTE_FORMS: [&str; 3] = [
    "_mfb_rt_tcp_tcp_write",
    "_mfb_rt_udp_udp_send",
    "_mfb_rt_tls_tls_write",
];
const TEXT_FORMS: [&str; 3] = [
    "_mfb_rt_tcp_tcp_writeText",
    "_mfb_rt_udp_udp_sendText",
    "_mfb_rt_tls_tls_writeText",
];

fn function<'a>(ncode: &'a Value, symbol: &str) -> &'a [Value] {
    ncode["functions"]
        .as_array()
        .expect("ncode has a functions array")
        .iter()
        .find(|f| f["symbol"].as_str() == Some(symbol))
        .unwrap_or_else(|| panic!("no function `{symbol}` in the dump"))["instructions"]
        .as_array()
        .expect("function has an instructions array")
}

fn field<'a>(inst: &'a Value, name: &str) -> &'a str {
    inst[name].as_str().unwrap_or("")
}

#[test]
fn write_form_is_selected_by_the_payload_type_for_every_call_shape() {
    let project = temp_project("codegen_net_write_select", SOURCE);
    let ncode = build_ncode(&project, "macos-aarch64", "codegen_net_write_select");
    let calls: Vec<&str> = function(&ncode, "_mfb_fn_main")
        .iter()
        .filter(|i| field(i, "op") == "bl")
        .map(|i| field(i, "target"))
        .filter(|t| {
            t.starts_with("_mfb_rt_tcp_tcp_write")
                || t.starts_with("_mfb_rt_udp_udp_send")
                || t.starts_with("_mfb_rt_tls_tls_write")
        })
        .collect();
    assert_eq!(
        calls,
        vec![
            "_mfb_rt_tcp_tcp_writeText", // head(7): a named FUNC returning String
            "_mfb_rt_tcp_tcp_writeText", // f(8): a call through a FUNC-typed value
            "_mfb_rt_tcp_tcp_writeText", // makeRec().body: a String field of a call result
            "_mfb_rt_tcp_tcp_write",     // raw: List OF Byte
            // A payload reached THROUGH a resource's STATE block. The fix's
            // fail-closed selector refused these two outright until
            // `overload_arg_type` learned `res.state` — a build error on a
            // program that had always been valid
            // (`tests/rt_macos_d4_union_state_tls.rs`).
            "_mfb_rt_tcp_tcp_write", // st.state.raw: List OF Byte via STATE
            "_mfb_rt_tcp_tcp_writeText", // st.state.note: String via STATE
            "_mfb_rt_udp_udp_sendText",
            "_mfb_rt_udp_udp_send",
            "_mfb_rt_tls_tls_writeText",
            "_mfb_rt_tls_tls_write",
        ],
        "bug-497: a String payload must take the text form whatever produced it, \
         and a List OF Byte the byte form; a String read through the byte form is a \
         peer-controlled out-of-bounds read"
    );
    let _ = std::fs::remove_dir_all(&project);
}

#[test]
fn byte_form_helpers_verify_the_collection_header_before_reading_a_length() {
    for target in TARGETS {
        let name = format!("codegen_net_write_view_{}", target.replace('-', "_"));
        let project = temp_project(&name, SOURCE);
        let ncode = build_ncode(&project, target, &name);
        for sym in BYTE_FORMS {
            let insts = function(&ncode, sym);
            let bad = format!("{sym}_bad_payload");
            // The header check: an `ldr_u8` off the payload register, a compare,
            // and a `b.ne` to the helper's own `_bad_payload` label — three of
            // them, at offsets 0/1/2 (kind, keyType, valueType). Identified by
            // the branch, not by "any byte load": Schannel's plaintext copy loop
            // also loads bytes.
            let checks: Vec<(usize, &Value)> = insts
                .iter()
                .enumerate()
                .filter(|(k, i)| {
                    // The first branch after the load decides. The Win64
                    // allocator spills between the load and its compare, and
                    // RISC-V fuses compare and branch into `rv.br … cond: ne`.
                    field(i, "op") == "ldr_u8"
                        && insts[k + 1..]
                            .iter()
                            .take(8)
                            .find(|n| !field(n, "target").is_empty())
                            .is_some_and(|b| {
                                field(b, "target") == bad
                                    && (field(b, "op") == "b.ne" || field(b, "cond") == "ne")
                            })
                })
                .collect();
            let mut offsets: Vec<&str> = checks.iter().map(|(_, i)| field(i, "offset")).collect();
            offsets.sort_unstable();
            assert_eq!(
                offsets,
                vec!["0", "1", "2"],
                "{target} {sym}: the byte form must read the block's kind/keyType/valueType \
                 header bytes and branch to `{bad}` before trusting its count (bug-497 sink \
                 hardening)"
            );
            let payload_reg = field(checks[0].1, "base");
            assert!(
                payload_reg != "sp" && checks.iter().all(|(_, i)| field(i, "base") == payload_reg),
                "{target} {sym}: header bytes read off more than one register"
            );
            assert!(
                insts
                    .iter()
                    .any(|i| field(i, "op") == "label" && field(i, "name") == bad),
                "{target} {sym}: `{bad}` is branched to but never emitted"
            );
            // The length is the collection COUNT (+8), read right after the
            // checks, and the word at +0 is never read as a length — that was the
            // bug-508 shape on Windows `tls::write`. Bounded to the view itself
            // (entry through a few instructions past the last check) so a later
            // reuse of the argument register cannot alias into the assertion.
            let view_end = checks
                .last()
                .map(|(k, _)| k + 12)
                .unwrap_or(0)
                .min(insts.len());
            let u64_offsets: Vec<&str> = insts[..view_end]
                .iter()
                .filter(|i| field(i, "op") == "ldr_u64" && field(i, "base") == payload_reg)
                .map(|i| field(i, "offset"))
                .collect();
            assert!(
                u64_offsets.contains(&"8"),
                "{target} {sym}: byte form never loads the collection count (+8) off the \
                 payload after its header check: {u64_offsets:?}"
            );
            assert!(
                !u64_offsets.contains(&"0"),
                "{target} {sym}: byte form reads the word at +0 of the payload — that is the \
                 String layout; on a List OF Byte it is the header (bug-508)"
            );
        }
        for sym in TEXT_FORMS {
            let insts = function(&ncode, sym);
            let bad = format!("{sym}_bad_payload");
            // The text form carries no header check at all: its payload is a
            // String, and its emitted bytes must stay exactly as they were.
            assert!(
                !insts
                    .iter()
                    .any(|i| field(i, "target") == bad || field(i, "name") == bad),
                "{target} {sym}: the text form must not reference `{bad}`"
            );
        }
        let _ = std::fs::remove_dir_all(&project);
    }
}
