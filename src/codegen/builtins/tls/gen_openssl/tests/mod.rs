// Regression guards for bug-55 on the OpenSSL/Linux TLS backend. These paths
// cannot execute on this macOS host; the assertions pin the emitted release
// sequence so a post-handshake OOM cannot silently leak the fd + SSL(+CTX),
// and so the alloc_fail cleanup is null/-1-guarded.
use super::*;
use crate::arch::ops::CodeOp;
use crate::codegen::engine::mir;
use crate::codegen::engine::tests::{has_label, TestPlatform};

fn reloc_count(rel: &[CodeRelocation], needle: &str) -> usize {
    rel.iter().filter(|r| r.to.contains(needle)).count()
}

#[test]
fn connect_alloc_fail_frees_ssl_ctx_and_fd() {
    mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
    let imports = HashMap::new();
    let (ins, rel, _s) =
        lower_tls_connect_helper("c", &imports, &TestPlatform, false).expect("lower connect");
    for label in [
        "c_af_skip_ssl",
        "c_af_skip_ctx",
        "c_af_skip_fd",
        "c_alloc_fail_raw",
    ] {
        assert!(
            has_label(&ins, label),
            "missing alloc_fail cleanup label {label}"
        );
    }
    // alloc_fail resolves SSL_free and SSL_CTX_free (neither was referenced by
    // connect before the fix — SSL_CTX_free was close-only).
    assert!(
        reloc_count(&rel, "sym_SSL_free") >= 1,
        "connect must free the SSL on OOM"
    );
    assert!(
        reloc_count(&rel, "sym_SSL_CTX_free") >= 1,
        "connect must free the SSL_CTX on OOM"
    );
}

/// bug-597: `tls::listen("")` must hand `getaddrinfo` a non-NULL service.
/// `getaddrinfo(NULL, NULL, …)` is `EAI_NONAME` on glibc and musl, so a
/// literal-0 service made the documented bind-all form raise
/// `ErrInvalidAddress` on Linux. The bind-all path stages the C string "0"
/// (0x30); nothing between `resolved` and the call may zero the service.
#[test]
fn listen_bind_all_passes_a_non_null_service() {
    mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
    let imports = HashMap::new();
    let (ins, _rel, _s) =
        lower_tls_listen_openssl("l", &imports, &TestPlatform).expect("lower listen");
    let at = |label: &str| {
        ins.iter()
            .position(|i| i.op == CodeOp::Label && i.get("name").as_deref() == Some(label))
            .unwrap_or_else(|| panic!("missing label {label}"))
    };
    let (null_host, resolved) = (at("l_null_host"), at("l_resolved"));
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

/// Whether `dlsym(<name>)` is emitted between labels `start` and `end`.
///
/// `emit_dlsym` materialises the symbol's data address with an `adrp`
/// carrying `_mfb_tls_sym_<name>`, so a resolution is visible positionally.
/// A whole-function reloc count cannot substitute: `connect` already frees
/// SSL/SSL_CTX in `alloc_fail`, so only a windowed check proves `tls_fail`
/// frees them too.
fn resolves_between(ins: &[CodeInstruction], start: &str, end: &str, name: &str) -> bool {
    let at = |label: &str| {
        ins.iter()
            .position(|i| i.op == CodeOp::Label && i.get("name").as_deref() == Some(label))
            .unwrap_or_else(|| panic!("missing label {label}"))
    };
    let (from, to) = (at(start), at(end));
    assert!(from < to, "expected {start} to precede {end}");
    let want = sym_data_symbol(name);
    ins[from..to]
        .iter()
        .any(|i| i.get("symbol").as_deref() == Some(&want))
}

// bug-317 T2: `tls_fail` is branched to from SSL_new onward — SSL_set_fd,
// SSL_set1_host, the min-proto ctrl, SSL_connect, SSL_get_verify_result — at
// every one of which this frame owns the SSL session and the per-connection
// SSL_CTX. It used to close only the fd, so a reconnect loop against an
// expired- or untrusted-cert host leaked several KB of OpenSSL heap per
// attempt, while the sibling alloc_fail and the accept-side ssl_fail freed
// both. The frees are null-guarded (slots are sentinel-initialised to 0).
#[test]
fn connect_tls_fail_frees_ssl_and_ctx() {
    mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
    let imports = HashMap::new();
    let (ins, _r, _s) =
        lower_tls_connect_helper("c", &imports, &TestPlatform, false).expect("lower connect");
    for label in ["c_tf_skip_ssl", "c_tf_skip_ctx", "c_tls_fail_raw"] {
        assert!(
            has_label(&ins, label),
            "missing tls_fail cleanup label {label} (the frees must be null-guarded)"
        );
    }
    assert!(
        resolves_between(&ins, "c_tls_fail", "c_tls_fail_raw", "SSL_free"),
        "a handshake failure must free the SSL session, not just close the fd"
    );
    assert!(
        resolves_between(&ins, "c_tls_fail", "c_tls_fail_raw", "SSL_CTX_free"),
        "a handshake failure must free the per-connection SSL_CTX"
    );
}

#[test]
fn accept_alloc_fail_frees_ssl_and_fd() {
    mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
    let imports = HashMap::new();
    let (_ins, rel, _s) =
        lower_tls_accept_helper("a", &imports, &TestPlatform).expect("lower accept");
    // ssl_fail resolves SSL_free once (2 data relocs); alloc_fail now adds a
    // second resolution, so the count roughly doubles.
    assert!(
        reloc_count(&rel, "sym_SSL_free") > 2,
        "accept alloc_fail must free the SSL session in addition to ssl_fail"
    );
}

// A `List OF Byte` is a PACKED fixed-width block (kind 2, plan-57-D): its
// entry stride is zero and the payload starts immediately after the header,
// with no entry-descriptor array. The OpenSSL `tls::read` byte path was
// missed by that change and kept writing 40-byte descriptors, so element 0
// landed on an entry-flags byte and every read reported a first byte of
// COLLECTION_ENTRY_FLAG_USED (1) instead of the wire byte. Measured on
// Alpine x86_64 (box 2227) before the fix: `first byte=1` for an HTTP reply
// whose first byte is 'H' (72); after: `first byte=72`.
//
// The guard is on the fill loop: with a zero stride its body may only copy
// the payload, never store an entry descriptor. Windowed between the loop's
// own labels because the block header legitimately stores at the same
// offsets (COLLECTION_OFFSET_CAPACITY == COLLECTION_ENTRY_OFFSET_KEY_LENGTH
// == 16) before the loop begins.
#[test]
fn byte_read_fill_loop_writes_no_entry_descriptor() {
    mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
    let imports = HashMap::new();
    let (ins, _rel, _s) = lower_tls_read_openssl("r", &imports, &TestPlatform).expect("lower read");
    let start = ins
        .iter()
        .position(|i| i.op == CodeOp::Label && i.get("name").as_deref() == Some("r_entry_loop"))
        .expect("byte read must emit the fill loop");
    let end = ins
        .iter()
        .position(|i| i.op == CodeOp::Label && i.get("name").as_deref() == Some("r_entry_done"))
        .expect("byte read must emit the fill loop terminator");
    let body = &ins[start..end];
    if byte_list_entry_stride() == 0 {
        for off in [
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ] {
            assert!(
                !body.iter().any(|i| {
                    i.op == CodeOp::StrU64 && i.get("offset").as_deref() == Some(&off.to_string())
                }),
                "packed byte list has no entry array, but the fill loop stores an \
                     entry descriptor field at offset {off}"
            );
        }
    }
    // Either way the cursor must advance by the real stride, never by a
    // hardcoded COLLECTION_ENTRY_SIZE.
    assert!(
        body.iter().any(|i| {
            i.op == CodeOp::AddImm
                && i.get("imm").as_deref() == Some(&byte_list_entry_stride().to_string())
        }),
        "the fill loop must advance its entry cursor by byte_list_entry_stride()"
    );
}

#[test]
fn listen_lowers_with_min_proto_check() {
    // The SSL_CTX_ctrl(SET_MIN_PROTO_VERSION) return is now checked; the
    // helper must still lower cleanly and resolve the ctrl symbol.
    mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
    let imports = HashMap::new();
    let (_ins, rel, _s) =
        lower_tls_listen_helper("l", &imports, &TestPlatform).expect("lower listen");
    assert!(reloc_count(&rel, "sym_SSL_CTX_ctrl") >= 1);
}

// bug-564 (owner decision 2026-09-25): once `tls::read` has reported the
// peer's close, every later `tls::write` raises `ErrConnectionClosed`. On
// OpenSSL a half-closed peer (close_notify sent, still reading) otherwise
// takes every write. The read records the close in the record's
// `TLS_OFFSET_PEER_CLOSED`, the write checks it before any call, and both
// constructors zero it (the record comes from the arena, which does not).
#[test]
fn a_read_that_reports_the_close_makes_every_later_write_raise() {
    mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
    let imports = HashMap::new();
    let flag = TLS_OFFSET_PEER_CLOSED.to_string();
    let off_record = |i: &CodeInstruction| {
        i.get("offset").as_deref() == Some(flag.as_str()) && i.get("base").as_deref() != Some("sp")
    };

    let (ins, _rel, _s) = lower_tls_read_openssl("r", &imports, &TestPlatform).expect("lower read");
    let from = ins
        .iter()
        .position(|i| i.op == CodeOp::Label && i.get("name").as_deref() == Some("r_peer_closed"))
        .expect("tls::read has a peer_closed exit");
    let sets = ins[from + 1..]
        .iter()
        .take_while(|i| i.op != CodeOp::Label)
        .any(|i| {
            i.op == CodeOp::StrU64
                && off_record(i)
                && i.get("src").as_deref() != Some(abi::ZERO_SPELLING)
        });
    assert!(
        sets,
        "tls::read's peer_closed exit must set TLS_OFFSET_PEER_CLOSED"
    );

    for text in [false, true] {
        let (ins, _rel, _s) =
            lower_tls_write_openssl("w", &imports, &TestPlatform, text).expect("lower write");
        let check = ins
            .iter()
            .position(|i| i.op == CodeOp::LdrU64 && off_record(i))
            .unwrap_or_else(|| panic!("tls::write (text={text}) must read the flag"));
        assert!(
            ins[check..(check + 4).min(ins.len())]
                .iter()
                .any(|i| i.op == CodeOp::BranchNe
                    && i.get("target").as_deref() == Some("w_peer_closed")),
            "tls::write (text={text}): a set flag must raise ErrConnectionClosed"
        );
        let first_call = ins
            .iter()
            .position(|i| matches!(i.op, CodeOp::BranchLink | CodeOp::BranchLinkRegister))
            .expect("tls::write calls something");
        assert!(
            check < first_call,
            "tls::write (text={text}): the flag check (index {check}) must precede the \
                 first call ({first_call})"
        );
    }

    let zeroes = |ins: &[CodeInstruction]| {
        ins.iter().any(|i| {
            i.op == CodeOp::StrU64
                && off_record(i)
                && i.get("src").as_deref() == Some(abi::ZERO_SPELLING)
        })
    };
    let (ins, _rel, _s) =
        lower_tls_connect_helper("c", &imports, &TestPlatform, false).expect("lower connect");
    assert!(
        zeroes(&ins),
        "tls::connect must zero TLS_OFFSET_PEER_CLOSED"
    );
    let (ins, _rel, _s) =
        lower_tls_accept_openssl("a", &imports, &TestPlatform).expect("lower accept");
    assert!(zeroes(&ins), "tls::accept must zero TLS_OFFSET_PEER_CLOSED");
}

// bug-564 (found while pinning the write rule, owner rule "all errors in all
// targets should be the same"): a peer that went away without close_notify
// (reset, or killed) made the OpenSSL `tls::read` raise ErrTlsFailed, where
// macOS raises ErrConnectionClosed for the same event. OpenSSL reports it as
// SSL_ERROR_SYSCALL (5), as SSL_ERROR_ZERO_RETURN (6), or, on 3.x, as
// SSL_ERROR_SSL (1) with reason SSL_R_UNEXPECTED_EOF_WHILE_READING (294).
// All three are the peer's close; any other SSL_ERROR_SSL stays ErrTlsFailed.
#[test]
fn read_names_a_departed_peer_connection_closed() {
    mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
    let imports = HashMap::new();
    let (ins, rel, _s) = lower_tls_read_openssl("r", &imports, &TestPlatform).expect("lower read");
    let routed = |value: &str, target: &str| {
        ins.windows(2).any(|w| {
            w[0].op == CodeOp::CmpImm
                && w[0].get("rhs").as_deref() == Some(value)
                && w[1].op == CodeOp::BranchEq
                && w[1].get("target").as_deref() == Some(target)
        })
    };
    assert!(
        routed("5", "r_peer_closed"),
        "SSL_ERROR_SYSCALL must raise ErrConnectionClosed"
    );
    assert!(
        routed("6", "r_peer_closed"),
        "SSL_ERROR_ZERO_RETURN must raise ErrConnectionClosed"
    );
    assert!(
        routed("294", "r_peer_closed"),
        "SSL_R_UNEXPECTED_EOF_WHILE_READING must raise ErrConnectionClosed"
    );
    assert!(
        reloc_count(&rel, "sym_ERR_peek_last_error") >= 1,
        "the unexpected-EOF check reads the error queue"
    );
    let tls_failed = crate::codegen::registry::runtime_error_emission("ErrTlsFailed")
        .expect("errorCode name")
        .1;
    assert!(
        rel.iter().any(|r| r.to == tls_failed),
        "any other failure is still ErrTlsFailed"
    );
}

/// bug-739: `SO_RCVTIMEO` bounds each read, so a peer trickling its
/// ClientHello held `SSL_accept` indefinitely. A bounded handshake must
/// run non-blocking against one deadline: the loop polls for the time
/// LEFT (a `clock_gettime` read before every `poll`) and no per-socket
/// timeout is installed any more.
#[test]
fn accept_bounds_the_whole_handshake_with_one_deadline() {
    mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
    let imports = HashMap::new();
    let (ins, _rel, _s) = lower_tls_accept_openssl("t_a", &imports, &TestPlatform)
        .expect("lower openssl tls::accept");
    let calls = |from: usize, to: usize, name: &str| {
        ins[from..to]
            .iter()
            .any(|i| i.op == CodeOp::BranchLink && i.get("target").as_deref() == Some(name))
    };
    let at = |name: &str| {
        ins.iter()
            .position(|i| i.op == CodeOp::Label && i.get("name").as_deref() == Some(name))
            .unwrap_or_else(|| panic!("missing label {name}"))
    };
    assert!(
        has_label(&ins, "t_a_hs_loop"),
        "the handshake must loop on SSL_accept"
    );
    let poll_at = at("t_a_hs_poll");
    let poll_call = ins[poll_at..]
        .iter()
        .position(|i| i.op == CodeOp::BranchLink && i.get("target").as_deref() == Some("_poll"))
        .map(|n| n + poll_at)
        .expect("the handshake loop must poll the connection");
    assert!(
        calls(poll_at, poll_call, "_clock_gettime"),
        "each wait must be for the time left, read from the monotonic clock"
    );
    assert!(
        !calls(0, ins.len(), "_setsockopt"),
        "accept must not bound the handshake with a per-read SO_*TIMEO any more"
    );
}
