// Regression guard for bug-414 item (2): the Schannel `tls::read`/`tls::readText`
// entry must reject `maxBytes <= 0` with `ErrInvalidArgument`, matching the
// OpenSSL backend (`openssl.rs`, which branches to an `_invalid` exit emitting
// `ERR_INVALID_ARGUMENT`). Before the fix the Schannel read had no such guard:
// `maxBytes == 0` ran a full blocking recv+DecryptMessage then served 0 bytes as
// OK, and a negative `maxBytes` routed to `alloc_fail`/`ErrOutOfMemory` — a
// cross-platform divergence from Linux/macOS. This lowers the read helper and
// pins the presence of the ErrInvalidArgument exit so it cannot silently
// regress. Runtime proof of the Schannel path is Windows-only (box 2230).
use super::*;
use crate::target::shared::code::mir;
use crate::target::shared::code::test_support::TestPlatform;

/// The Schannel read helper emits an `ErrInvalidArgument` failure exit, produced
/// only by `emit_fail(ERR_INVALID_ARGUMENT_*)` — which relocates the error
/// message data symbol. bug-414 (2): before the fix no such exit existed.
fn reads_reject_invalid_maxbytes(text: bool) {
    mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
    let imports = HashMap::new();
    let (_frame, _ins, rel, _slots) =
        lower_tls_read("t_read", &imports, &TestPlatform, text).expect("lower schannel tls::read");
    assert!(
        rel.iter().any(|r| r.to == ERR_INVALID_ARGUMENT_SYMBOL),
        "bug-414: schannel tls::read must reject maxBytes <= 0 with ErrInvalidArgument \
         (text={text})"
    );
}

#[test]
fn read_bytes_rejects_nonpositive_maxbytes() {
    reads_reject_invalid_maxbytes(false);
}

#[test]
fn read_text_rejects_nonpositive_maxbytes() {
    reads_reject_invalid_maxbytes(true);
}
