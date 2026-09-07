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
