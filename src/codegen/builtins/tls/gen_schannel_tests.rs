// Regression guard for bug-414 item (2): the Schannel `tls::read`/`tls::read`
// entry must reject `maxBytes <= 0` with `ErrInvalidArgument`, matching the
// OpenSSL backend (`openssl.rs`, which branches to an `_invalid` exit emitting
// `ERR_INVALID_ARGUMENT`). Before the fix the Schannel read had no such guard:
// `maxBytes == 0` ran a full blocking recv+DecryptMessage then served 0 bytes as
// OK, and a negative `maxBytes` routed to `alloc_fail`/`ErrOutOfMemory` — a
// cross-platform divergence from Linux/macOS. This lowers the read helper and
// pins the presence of the ErrInvalidArgument exit so it cannot silently
// regress. Runtime proof of the Schannel path is Windows-only (box 2230).
// --- codegen tier imports (migration) ---
use super::*;
use crate::arch::ops::CodeOp;
use crate::codegen::engine::mir;
use crate::codegen::engine::tests::TestPlatform;
use std::collections::HashMap;
/// The Schannel read helper emits an `ErrInvalidArgument` failure exit, produced
/// only by `emit_fail(ERR_INVALID_ARGUMENT_*)` — which relocates the error
/// message data symbol. bug-414 (2): before the fix no such exit existed.
fn reads_reject_invalid_maxbytes() {
    mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
    let imports = HashMap::new();
    let (_ins, rel, _slots) =
        lower_tls_read("t_read", &imports, &TestPlatform).expect("lower schannel tls::read");
    let invalid_argument_symbol =
        crate::codegen::registry::runtime_error_emission("ErrInvalidArgument")
            .expect("errorCode name")
            .1;
    assert!(
        rel.iter().any(|r| r.to == invalid_argument_symbol),
        "bug-414: schannel tls::read must reject maxBytes <= 0 with ErrInvalidArgument"
    );
}

// plan-110-D: one form now, `tls::read` being bytes-only.
#[test]
fn read_rejects_nonpositive_maxbytes() {
    reads_reject_invalid_maxbytes();
}

// bug-461: the Schannel `tls::listen` accepted only a PKCS#8 private key.
//
// The key load unwrapped PKCS#8 (`CryptDecodeObjectEx(PKCS_PRIVATE_KEY_INFO=44)`)
// and jumped to the shared failure exit when that returned FALSE -- which is
// exactly what a traditional PKCS#1 `-----BEGIN RSA PRIVATE KEY-----` does. That
// is the form `openssl rsa -traditional` emits, and the resulting `7-707-0008`
// says nothing about key encoding. macOS and the OpenSSL backend both accept
// either form, so this was the one platform where a portable PEM did not exist.
//
// The fallback is control flow, so it is pinned by its labels: the PKCS#8 unwrap
// must branch to a PKCS#1 path rather than to failure, and both must join at one
// `PKCS_RSA_PRIVATE_KEY` decode. Execution proof is Windows-only (box 2230).
#[test]
fn listen_accepts_a_pkcs1_key_not_only_pkcs8() {
    mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
    let imports = HashMap::new();
    let (ins, _rel, _slots) =
        lower_tls_listen("t_listen", &imports, &TestPlatform).expect("lower schannel tls::listen");
    let has_label = |name: &str| ins.iter().any(|i| i.get("name").as_deref() == Some(name));
    assert!(
        has_label("t_listen_key_pkcs1"),
        "the PKCS#8 unwrap must fall back to treating the DER as PKCS#1 rather than \
         failing the call (bug-461)"
    );
    assert!(
        has_label("t_listen_key_decoded"),
        "both key encodings must join at one PKCS_RSA_PRIVATE_KEY decode (bug-461)"
    );
}

// bug-597: Winsock's `getaddrinfo`, like glibc's, refuses a NULL node together
// with a NULL service, so the empty-host bind-all `tls::listen` could never
// resolve. The bind-all path stages the C string "0" (0x30) as the service, and
// nothing between `resolved` and the call may zero it. Execution proof is
// Windows-only (box 2230).
#[test]
fn listen_bind_all_passes_a_non_null_service() {
    mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
    let imports = HashMap::new();
    let (ins, _rel, _slots) =
        lower_tls_listen("t_listen", &imports, &TestPlatform).expect("lower schannel tls::listen");
    let at = |label: &str| {
        ins.iter()
            .position(|i| i.op == CodeOp::Label && i.get("name").as_deref() == Some(label))
            .unwrap_or_else(|| panic!("missing label {label}"))
    };
    let (null_host, resolved) = (at("t_listen_null_host"), at("t_listen_resolved"));
    assert!(
        ins[null_host..resolved]
            .iter()
            .any(|i| i.op == CodeOp::MovImm && i.get("value").as_deref() == Some("48")),
        "bug-597: the bind-all path must stage the service string \"0\""
    );
    let service_reg = abi::c_arg(1).to_string();
    let zeroed_service = ins[resolved..].iter().take(8).any(|i| {
        i.op == CodeOp::MovImm
            && i.get("dst").as_deref() == Some(service_reg.as_str())
            && i.get("value").as_deref() == Some("0")
    });
    assert!(
        !zeroed_service,
        "bug-597: getaddrinfo's service argument must not be a literal NULL"
    );
}

// bug-483: the Schannel `tls::write` collapsed every `send` failure into
// `ErrNetworkFailed`. Measured on box 2230 before the fix, an MFBASIC TLS server
// writing to a client that had gone away reported "Network operation failed
// before a connection was established" — for a connection that WAS established
// and had been used — where the OpenSSL backend reported `ErrConnectionClosed`,
// which is what `mfb man tls write` and `mfb man tcp write` both state.
//
// The same arm swallowed the `tls::setWriteTimeout` deadline: `SO_SNDTIMEO`
// expiry is `WSAETIMEDOUT` (10060) on Winsock, and `mfb man tls setWriteTimeout`
// says a write that reaches its deadline raises `ErrTimeout`. The read side was
// taught exactly this in plan-110-D; the write side never was.
//
// Windows codegen cannot be executed from this host, so this lowering test is
// the instrument that covers the change here (`.ncodesum` only says a hash
// moved). Runtime proof is box 2230, recorded in the bug document.
#[test]
fn write_classifies_the_winsock_error_behind_a_failed_send() {
    mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
    let imports = HashMap::new();
    for text in [false, true] {
        let (ins, rel, _slots) = lower_tls_write("t_write", &imports, &TestPlatform, text)
            .expect("lower schannel tls::write");
        let has_label = |name: &str| ins.iter().any(|i| i.get("name").as_deref() == Some(name));
        assert!(
            has_label("t_write_send_fail"),
            "`send` needs a failure arm of its own to classify (text={text})"
        );
        for arm in ["t_write_peer_closed", "t_write_timed_out"] {
            assert!(has_label(arm), "missing {arm} (text={text})");
        }
        // The positive half, and the one that matters: `EncryptMessage`'s own
        // negative return and any unrecognised Winsock error are NOT transport
        // failures and must keep the code they always had. A fix that routed
        // every write failure to `ErrConnectionClosed` would pass the runtime
        // proof and be a worse bug than the one it replaced.
        for code in [
            "ErrConnectionClosed",
            "ErrTimeout",
            "ErrNetworkFailed",
            "ErrResourceClosed",
        ] {
            let symbol = crate::codegen::registry::runtime_error_emission(code)
                .expect("errorCode name")
                .1;
            assert!(
                rel.iter().any(|r| r.to == symbol),
                "schannel tls::write must keep {code} reachable (text={text})"
            );
        }
    }
}

// bug-564 (found while measuring its OPEN finding on box 2230): a peer that sent
// close_notify and kept its socket open — a TLS half-close, which is what
// `openssl s_client` and .NET `SslStream.ShutdownAsync` both do — hung the
// Schannel `tls::read` forever. `DecryptMessage` reports close_notify as
// SEC_I_CONTEXT_EXPIRED (0x00090317), a SUCCESS status with no plaintext, and the
// read loop treated it as an empty record and went back to a `recv` that only the
// peer's FIN could end. OpenSSL and Network.framework both raise
// `ErrConnectionClosed` at that point.
//
// Pinned three ways: the status routes to an exit that records the close in the
// state block; a later read raises from that record before it can reach `recv`;
// and `tls::poll` reports such a socket readable, since the next read will not
// block. Execution proof is Windows-only (box 2230).
#[test]
fn a_close_notify_ends_the_read_and_stays_ended() {
    mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
    let imports = HashMap::new();
    let peer_closed = st::PEER_CLOSED.to_string();
    let position = |ins: &[CodeInstruction], what: &str, f: &dyn Fn(&CodeInstruction) -> bool| {
        ins.iter()
            .position(|i| f(i))
            .unwrap_or_else(|| panic!("missing {what}"))
    };
    let label_at = |ins: &[CodeInstruction], name: &str| {
        position(ins, name, &|i| {
            i.op == CodeOp::Label && i.get("name").as_deref() == Some(name)
        })
    };

    let (ins, _rel, _slots) =
        lower_tls_read("t_read", &imports, &TestPlatform).expect("lower schannel tls::read");
    let status_check = position(&ins, "a SEC_I_CONTEXT_EXPIRED arm", &|i| {
        i.op == CodeOp::CmpImm && i.get("rhs").as_deref() == Some(SEC_I_CONTEXT_EXPIRED)
    });
    assert!(
        ins[status_check + 1].op == CodeOp::BranchEq
            && ins[status_check + 1].get("target").as_deref() == Some("t_read_context_expired"),
        "DecryptMessage's SEC_I_CONTEXT_EXPIRED must leave the read loop"
    );
    let (expired, peer) = (
        label_at(&ins, "t_read_context_expired"),
        label_at(&ins, "t_read_peer"),
    );
    // The record is made at the COMMON `peer` exit, which the close_notify arm
    // falls into: since the 2026-09-25 owner decision, every read that raises
    // ErrConnectionClosed (close_notify, or the transport's own end of stream)
    // must leave the flag set for `tls::write`, not only the close_notify one.
    assert!(
        expired < peer
            && ins[expired + 1..peer].iter().all(|i| i.op != CodeOp::Label)
            && ins[peer + 1..]
                .iter()
                .take_while(|i| i.op != CodeOp::Label)
                .any(|i| i.op == CodeOp::StrU64
                    && i.get("offset").as_deref() == Some(&peer_closed)
                    && i.get("src").as_deref() != Some(abi::ZERO)),
        "the close_notify exit must fall into the ErrConnectionClosed exit, which records \
         PEER_CLOSED for every read that reports the close"
    );
    let sticky = position(&ins, "a PEER_CLOSED check at read entry", &|i| {
        i.op == CodeOp::LdrU64 && i.get("offset").as_deref() == Some(&peer_closed)
    });
    assert!(
        sticky < label_at(&ins, "t_read_dloop")
            && ins[sticky..sticky + 3]
                .iter()
                .any(|i| i.op == CodeOp::BranchNe
                    && i.get("target").as_deref() == Some("t_read_peer")),
        "a read after the close_notify must raise before it reaches recv"
    );

    let (ins, _rel, _slots) =
        lower_tls_poll("t_poll", &imports, &TestPlatform).expect("lower schannel tls::poll");
    let sticky = position(&ins, "a PEER_CLOSED check in poll", &|i| {
        i.op == CodeOp::LdrU64 && i.get("offset").as_deref() == Some(&peer_closed)
    });
    assert!(
        sticky < label_at(&ins, "t_poll_poll_infinite")
            && ins[sticky..sticky + 3]
                .iter()
                .any(|i| i.op == CodeOp::BranchNe
                    && i.get("target").as_deref() == Some("t_poll_ready")),
        "tls::poll must report a socket whose close_notify was consumed as readable"
    );
}

// bug-564 (found on box 2230 while measuring its OPEN finding): a second
// `tls::connect` in one Windows process crashed with 0xC0000005 inside
// `_mfb_arena_alloc` (`rt-behavior/tls/tls-poll-rt` never printed `loop=`,
// `tls-poll-list-rt` printed nothing). The connect body mints its vregs from
// `%v0`, so its host-C-string `HelperScratch` pointer was `%v9` — the very name
// `emit_free_buffer_guarded` hard-coded for the pointer it frees. The success
// path's free of the 64 KiB wide server name therefore overwrote the scratch
// pointer, and `done` freed that 64 KiB block AGAIN as an 8-byte one: it went
// onto the 16-byte quick bin while still linked in large bin 0, the next
// connect's host string was carved from it, and "8.8.8.8" became a bin link
// (captured at the fault: r10 = 0x382E382E382E38).
//
// Pinned on the connect body itself: across every path, the only writes to the
// vreg the scratch release frees are its null-init and the allocation capture.
#[test]
fn connect_keeps_its_host_scratch_pointer_intact_to_the_release() {
    mir::set_backend(&crate::arch::x86_64::backend::X86_64_BACKEND);
    // The real Windows platform, because the generic test platform cannot lower
    // this body. It refuses an import the plan did not declare, so declare each
    // one it names (the library is irrelevant to the vreg flow under test).
    let windows = crate::target::win_x86_64::code::Platform;
    let lower = |address: bool| {
        let mut imports: HashMap<String, String> = HashMap::new();
        loop {
            match lower_tls_connect("t_c", &imports, &windows, address) {
                Ok(parts) => return parts,
                Err(e) => {
                    let name = e
                        .strip_prefix("runtime helper requires ")
                        .and_then(|r| r.strip_suffix(" import"))
                        .unwrap_or_else(|| panic!("lower schannel tls::connect: {e}"));
                    assert!(
                        imports
                            .insert(name.to_string(), "ws2_32.dll".to_string())
                            .is_none(),
                        "{name} was already declared: {e}"
                    );
                }
            }
        }
    };
    for address in [false, true] {
        let (ins, _rel, _s) = lower(address);
        let kept = ins
            .iter()
            .position(|i| {
                i.op == CodeOp::Label && i.get("name").as_deref() == Some("t_c_scratch_kept_0")
            })
            .expect("connect releases its host scratch at `done`");
        // `emit_helper_scratch_release` ends `mov arg0, pointer; mov arg1, size;
        // <free>` before the `kept` label: the pointer is the second-to-last
        // vreg-to-argument move.
        let arg_moves: Vec<String> = ins[..kept]
            .iter()
            .filter(|i| {
                i.op == CodeOp::Mov
                    && i.get("src").is_some_and(|s| s.starts_with("%v"))
                    && i.get("dst").is_some_and(|d| !d.starts_with("%v"))
            })
            .filter_map(|i| i.get("src").map(|s| s.to_string()))
            .collect();
        let freed = arg_moves[arg_moves.len() - 2].clone();
        let writers: Vec<String> = ins
            .iter()
            .filter(|i| i.get("dst").as_deref() == Some(freed.as_str()))
            .map(|i| format!("{:?} {:?}", i.op, i.get("src").or(i.get("base"))))
            .collect();
        assert_eq!(
            writers.len(),
            2,
            "(address={address}) the host scratch pointer {freed} may be written only by its \
             null-init and its allocation capture, or the release frees whatever overwrote \
             it; writers: {writers:?}"
        );
    }
}

// bug-564 (owner decision 2026-09-25): once `tls::read` has reported the peer's
// close, every later `tls::write` raises `ErrConnectionClosed` on every target.
// Schannel's write checks the state block's PEER_CLOSED before any call, so it
// neither allocates its send buffer nor encrypts. It raises through an exit of
// its own, because the send-failure exit runs after the buffer exists.
#[test]
fn a_read_that_reports_the_close_makes_every_later_write_raise() {
    mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
    let imports = HashMap::new();
    let peer_closed = st::PEER_CLOSED.to_string();
    for text in [false, true] {
        let (ins, _rel, _s) = lower_tls_write("t_write", &imports, &TestPlatform, text)
            .expect("lower schannel tls::write");
        let check = ins
            .iter()
            .position(|i| {
                i.op == CodeOp::LdrU64 && i.get("offset").as_deref() == Some(&peer_closed)
            })
            .unwrap_or_else(|| panic!("tls::write (text={text}) must read PEER_CLOSED"));
        let target = ins[check..(check + 4).min(ins.len())]
            .iter()
            .find(|i| i.op == CodeOp::BranchNe)
            .and_then(|i| i.get("target"))
            .unwrap_or_else(|| panic!("tls::write (text={text}): a set flag must branch away"));
        let exit = ins
            .iter()
            .position(|i| {
                i.op == CodeOp::Label && i.get("name").as_deref() == Some(target.as_str())
            })
            .expect("the flag's exit exists");
        let raises_closed = ins[exit + 1..]
            .iter()
            .take_while(|i| i.op != CodeOp::Label)
            .any(|i| {
                i.op == CodeOp::BranchLink || i.op == CodeOp::Branch || i.op == CodeOp::MovImm
            });
        assert!(
            raises_closed,
            "tls::write (text={text}): the flag's exit must raise"
        );
        let first_call = ins
            .iter()
            .position(|i| matches!(i.op, CodeOp::BranchLink | CodeOp::BranchLinkRegister))
            .expect("tls::write calls something");
        assert!(
            check < first_call,
            "tls::write (text={text}): the PEER_CLOSED check (index {check}) must precede the \
             first call ({first_call}), so no send buffer is allocated"
        );
    }
}

// bug-564 (found on box 2230 while pinning the write rule): Schannel
// `tls::write` allocated its send buffer (stream header + max message + trailer,
// about 16 KiB) on every call and never released it, on success or failure. 20000
// five-byte writes to a live peer reached a 400 MB peak working set. Every exit
// reached after the allocation must release it; the exits before it must not.
#[test]
fn write_releases_its_send_buffer_on_every_exit_after_allocating_it() {
    mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
    let imports = HashMap::new();
    for text in [false, true] {
        let (ins, _rel, _s) = lower_tls_write("t_write", &imports, &TestPlatform, text)
            .expect("lower schannel tls::write");
        let frees_in = |label: &str| {
            let at = ins
                .iter()
                .position(|i| i.op == CodeOp::Label && i.get("name").as_deref() == Some(label))
                .unwrap_or_else(|| panic!("missing label {label}"));
            ins[at + 1..]
                .iter()
                .take_while(|i| {
                    i.op != CodeOp::Label
                        || i.get("name").is_some_and(|n| n.contains("buf_free_skip"))
                })
                .any(|i| {
                    i.op == CodeOp::BranchLink
                        && i.get("target").as_deref()
                            == Some(crate::codegen::error::constants::ARENA_FREE_SYMBOL)
                })
        };
        for exit in [
            "t_write_wdone",
            "t_write_peer_closed",
            "t_write_timed_out",
            "t_write_fail",
        ] {
            assert!(
                frees_in(exit),
                "tls::write (text={text}): `{exit}` runs after the send buffer exists and must release it"
            );
        }
        for exit in ["t_write_closed", "t_write_peer_gone", "t_write_alloc_fail"] {
            assert!(
                !frees_in(exit),
                "tls::write (text={text}): `{exit}` runs before the send buffer exists"
            );
        }
    }
}

// bug-564 (found while pinning the write rule, owner rule "all errors in all
// targets should be the same"): a peer that resets the connection made the
// Schannel `tls::read` raise ErrNetworkFailed ("failed before a connection was
// established"), where macOS raises ErrConnectionClosed for the same event and
// the write side on this backend already maps the same Winsock codes to it.
#[test]
fn read_names_a_reset_peer_connection_closed() {
    mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
    let imports = HashMap::new();
    let (ins, _rel, _s) =
        lower_tls_read("t_read", &imports, &TestPlatform).expect("lower schannel tls::read");
    for code in ["10054", "10053", "10058"] {
        let routed = ins.windows(2).any(|w| {
            w[0].op == CodeOp::CmpImm
                && w[0].get("rhs").as_deref() == Some(code)
                && w[1].op == CodeOp::BranchEq
                && w[1].get("target").as_deref() == Some("t_read_peer")
        });
        assert!(
            routed,
            "Winsock {code} after recv must raise ErrConnectionClosed"
        );
    }
}
