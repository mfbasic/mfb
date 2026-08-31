// `tls::setReadTimeout` / `tls::setWriteTimeout` on macOS (plan-110-D Phase 2).
//
// Linux and Windows implement these with `setsockopt(SO_RCVTIMEO/SO_SNDTIMEO)`
// on the descriptor their TLS record already holds, sharing `net`'s emitter
// verbatim. macOS has no descriptor to set an option on — Network.framework owns
// the socket — so the deadline is recorded on the connection context and applied
// where the operation actually blocks: the `dispatch_semaphore_wait` in
// `tls::read`'s drain (CTX_RTO) and `tls::write`'s send wait (CTX_WTO), both via
// `emit_wait_bounded`.
//
// This is a pure store. It touches no Network.framework object, so it needs no
// dlopen and cannot fail except on a closed socket or a negative argument.

use super::*;
use crate::target::shared::abi;
use std::collections::HashMap;

/// `write` selects `CTX_WTO` (the send deadline) over `CTX_RTO` (the receive
/// deadline).
pub(crate) fn lower_tls_set_timeout_macos(
    symbol: &str,
    _platform_imports: &HashMap<String, String>,
    _platform: &dyn CodegenPlatform,
    write: bool,
) -> Result<TlsBodyParts, String> {
    const FRAME_SIZE: usize = 32;

    let closed = format!("{symbol}_closed");
    let invalid = format!("{symbol}_invalid");
    let done = format!("{symbol}_done");

    let mut ins: Vec<CodeInstruction> = Vec::new();
    let mut rel: Vec<CodeRelocation> = Vec::new();
    let mut vregs = Vregs::new();
    let v9 = vregs.next();
    let v10 = vregs.next();

    ins.extend([
        // x0 = the TLS socket record, x1 = timeoutMs. A negative that is not the
        // unbounded sentinel is rejected before anything is stored, matching the
        // convention `net`/`tcp` enforce in their setsockopt path.
        abi::move_register(&v10, abi::c_arg(1)),
        abi::move_immediate(&v9, "Integer", TIMEOUT_UNBOUNDED_SENTINEL),
        abi::compare_registers(&v10, &v9),
        abi::branch_eq(&format!("{symbol}_ts_ok")),
        abi::compare_immediate(&v10, "0"),
        abi::branch_lt(&invalid),
        abi::label(&format!("{symbol}_ts_ok")),
        abi::load_u64(&v9, abi::return_register(), REC_CLOSED),
        abi::compare_immediate(&v9, "0"),
        abi::branch_ne(&closed),
        // The deadline lives on the per-connection ctx, not on the record: the
        // waits that consume it already hold the ctx pointer, and an accepted
        // socket and a connected one share the same ctx shape.
        abi::load_u64(&v9, abi::return_register(), REC_CTX),
        abi::store_u64(&v10, &v9, if write { CTX_WTO } else { CTX_RTO }),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", "0"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);
    ins.push(abi::label(&invalid));
    emit_fail(symbol, "ErrInvalidArgument", &mut ins, &mut rel, &done);
    ins.push(abi::label(&closed));
    emit_fail(symbol, "ErrResourceClosed", &mut ins, &mut rel, &done);
    ins.extend([abi::label(&done), abi::return_()]);
    Ok((ins, rel, FRAME_SIZE))
}
