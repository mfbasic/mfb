//! macOS Network.framework TLS block trampolines (aarch64).
//!
//! These are the fixed-ABI dispatch/objc block `invoke` functions the
//! Network.framework / libdispatch runtime calls back into: the block pointer
//! arrives in `x0` and the remaining arguments in `x1..` per each block's C
//! signature, so their register layout is dictated by Apple's runtime, not by
//! us — the allocator cannot place it. They are the macOS counterpart of the
//! program-entry / thread-trampoline "machine floor": per-(OS, ISA) emitters,
//! reached through [`CodegenPlatform::emit_tls_block_trampolines`]. A future
//! macOS-x86 backend supplies its own here, reusing the ISA-neutral block/ctx
//! layout constants that stay in `shared/code/tls/macos.rs`.

use crate::arch::aarch64::abi;
use crate::codegen::builtins::tls::gen_macos::{
    verify_fn_slot, BLK_CAP, BLK_INVOKE, CFG_CAP_COPYFN, CFG_CAP_QUEUE, CFG_CAP_RELEASEFN,
    CFG_CAP_SETFN, CFG_CAP_SETVERIFYFN, CFG_CAP_SNAME, CFG_CAP_VBLOCK, CFG_INVOKE, CTX_ERROR,
    CTX_PCONTENT, CTX_PERROR, CTX_PSEM, CTX_RETAIN, CTX_SEM, CTX_SIGNAL, CTX_STATE, LCONN_INVOKE,
    LCTX_HEAD, LCTX_RING, LCTX_RING_CAP, LCTX_TAIL, RECV_POLL_INVOKE, SEND_INVOKE, STATE_INVOKE,
    VERIFY_CAP_SNAME, VERIFY_FNS_SYMBOL, VERIFY_INVOKE,
};
use crate::codegen::engine::types::CodeFrame;
use crate::codegen::engine::types::CodeFunction;

/// A leaf frame that only saves the link register (these trampolines call
/// captured function pointers, so they are not true leaves).
fn frame(stack_size: usize) -> CodeFrame {
    CodeFrame {
        stack_size,
        callee_saved: vec![abi::link_register().to_string()],
    }
}

/// A block invoke `void(block, ...)` that stores its argument registers into
/// the captured ctx slots, then calls the captured signal fn on the
/// semaphore. `stores` is a list of `(arg_register, ctx_offset)`.
fn invoke_function(symbol: &str, stores: &[(&str, usize)]) -> CodeFunction {
    let mut instructions = vec![
        abi::label("entry"),
        abi::subtract_stack(16),
        abi::store_u64(abi::link_register(), abi::stack_pointer(), 0),
        abi::load_u64(abi::SCRATCH[0], abi::c_arg(0), BLK_CAP), // ctx = block->captured pointer
    ];
    for (reg, off) in stores {
        instructions.push(abi::store_u64(reg, abi::SCRATCH[0], *off));
    }
    instructions.extend([
        abi::load_u64(abi::SCRATCH[1], abi::SCRATCH[0], CTX_SIGNAL),
        abi::load_u64(abi::c_arg(0), abi::SCRATCH[0], CTX_SEM),
        abi::branch_link_register(abi::SCRATCH[1]),
        abi::load_u64(abi::link_register(), abi::stack_pointer(), 0),
        abi::add_stack(16),
        abi::return_(),
    ]);
    CodeFunction {
        name: format!("runtime.{symbol}"),
        symbol: symbol.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame: frame(16),
        stack_slots: Vec::new(),
        instructions,
        relocations: Vec::new(),
    }
}

/// The receive completion `(content @x1, context @x2, is_complete @x3,
/// error @x4)`. The `content` dispatch_data is only valid for the block's
/// duration, so it is retained before being stashed for the helper to map.
///
/// Parameterized over the ctx slots it writes/signals so the poll readiness
/// receive (plan-76-B Phase 4) gets an isolated block over `CTX_P*` — identical
/// body, different offsets — that never touches the read/write `CTX_SEM`.
fn recv_invoke_impl(
    symbol: &str,
    content_off: usize,
    error_off: usize,
    sem_off: usize,
) -> CodeFunction {
    let sig = format!("{symbol}_sig");
    let instructions = vec![
        abi::label("entry"),
        abi::subtract_stack(32),
        abi::store_u64(abi::link_register(), abi::stack_pointer(), 0),
        abi::store_u64(abi::LOCAL[0], abi::stack_pointer(), 8),
        abi::move_register(abi::LOCAL[0], abi::c_arg(0)), // x19 = block; reload ctx below
        abi::load_u64(abi::LOCAL[0], abi::LOCAL[0], BLK_CAP), // x19 = ctx (callee-saved across calls)
        abi::store_u64(abi::c_arg(4), abi::LOCAL[0], error_off),
        abi::compare_immediate(abi::c_arg(1), "0"),
        abi::branch_eq(&sig),
        abi::store_u64(abi::c_arg(1), abi::LOCAL[0], content_off),
        // dispatch_retain(content) so it survives past this block.
        abi::load_u64(abi::SCRATCH[3], abi::LOCAL[0], CTX_RETAIN),
        abi::move_register(abi::c_arg(0), abi::c_arg(1)),
        abi::branch_link_register(abi::SCRATCH[3]),
        abi::label(&sig),
        abi::load_u64(abi::SCRATCH[1], abi::LOCAL[0], CTX_SIGNAL),
        abi::load_u64(abi::c_arg(0), abi::LOCAL[0], sem_off),
        abi::branch_link_register(abi::SCRATCH[1]),
        abi::load_u64(abi::LOCAL[0], abi::stack_pointer(), 8),
        abi::load_u64(abi::link_register(), abi::stack_pointer(), 0),
        abi::add_stack(32),
        abi::return_(),
    ];
    CodeFunction {
        name: format!("runtime.{symbol}"),
        symbol: symbol.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame: frame(32),
        stack_slots: Vec::new(),
        instructions,
        relocations: Vec::new(),
    }
}

fn recv_poll_invoke_function() -> CodeFunction {
    recv_invoke_impl(RECV_POLL_INVOKE, CTX_PCONTENT, CTX_PERROR, CTX_PSEM)
}

/// The configure-TLS block `void(block @x0, nw_protocol_options_t tls @x1)`.
/// It copies the TLS protocol's `sec_protocol_options`, then overrides the
/// server name used for SNI and certificate validation. The server-name C
/// string and the two framework functions are captured in the block (the
/// invoke is a static aux function and cannot embed per-call `dlsym`
/// results). Defaults still apply for everything it does not touch.
/// bug-477 added the verify-block install. The two overrides are **independent**
/// null-checks, not an if/else: `allowSelfSigned` may be set with no
/// `serverName` (validation then defaults to `host`, matching the other two
/// backends), and `serverName` may be set with the flag off. The connect side
/// takes this custom block whenever either applies and NULLs the capture for
/// whichever does not.
fn cfg_invoke_function() -> CodeFunction {
    let skip_sname = format!("{CFG_INVOKE}_no_sname");
    let skip_verify = format!("{CFG_INVOKE}_no_verify");
    let instructions = vec![
        abi::label("entry"),
        abi::subtract_stack(64),
        abi::store_u64(abi::link_register(), abi::stack_pointer(), 0),
        abi::store_u64(abi::LOCAL[0], abi::stack_pointer(), 8),
        abi::store_u64(abi::LOCAL[1], abi::stack_pointer(), 16),
        abi::store_u64(abi::LOCAL[2], abi::stack_pointer(), 40),
        // x0 = block, x1 = tls_options. Preserve server name + setter across
        // the copy call (x0/x1 are clobbered by it). The release fn is stashed
        // to a stack slot now because the block pointer (x0) is clobbered too.
        // The block pointer itself is kept in LOCAL[2] for the verify install.
        abi::move_register(abi::LOCAL[2], abi::c_arg(0)),
        abi::load_u64(abi::LOCAL[0], abi::c_arg(0), CFG_CAP_SNAME), // server name (cstr), may be NULL
        abi::load_u64(abi::LOCAL[1], abi::c_arg(0), CFG_CAP_SETFN), // sec_protocol_options_set_tls_server_name
        abi::load_u64(abi::SCRATCH[0], abi::c_arg(0), CFG_CAP_RELEASEFN), // nw_release
        abi::store_u64(abi::SCRATCH[0], abi::stack_pointer(), 32),
        abi::load_u64(abi::SCRATCH[0], abi::c_arg(0), CFG_CAP_COPYFN), // nw_tls_copy_sec_protocol_options
        abi::move_register(abi::c_arg(0), abi::c_arg(1)),
        abi::branch_link_register(abi::SCRATCH[0]), // x0 = sec_options (+1)
        abi::store_u64(abi::c_arg(0), abi::stack_pointer(), 24), // survive the setter calls
        // set_tls_server_name(sec_options, name) — only when a name was captured.
        abi::compare_immediate(abi::LOCAL[0], "0"),
        abi::branch_eq(&skip_sname),
        abi::load_u64(abi::c_arg(0), abi::stack_pointer(), 24),
        abi::move_register(abi::c_arg(1), abi::LOCAL[0]),
        abi::branch_link_register(abi::LOCAL[1]),
        abi::label(&skip_sname),
        // set_verify_block(sec_options, verify_block, queue) — only when the
        // verify block was captured, i.e. only when `allowSelfSigned` is set.
        abi::load_u64(abi::LOCAL[0], abi::LOCAL[2], CFG_CAP_VBLOCK),
        abi::compare_immediate(abi::LOCAL[0], "0"),
        abi::branch_eq(&skip_verify),
        abi::load_u64(abi::LOCAL[1], abi::LOCAL[2], CFG_CAP_SETVERIFYFN),
        abi::load_u64(abi::c_arg(2), abi::LOCAL[2], CFG_CAP_QUEUE),
        abi::load_u64(abi::c_arg(0), abi::stack_pointer(), 24),
        abi::move_register(abi::c_arg(1), abi::LOCAL[0]),
        abi::branch_link_register(abi::LOCAL[1]),
        abi::label(&skip_verify),
        // Balance the copy fn's +1 retain: nw_release(sec_options). The setters
        // are getter-style config and do not consume the ref (bug-116).
        abi::load_u64(abi::c_arg(0), abi::stack_pointer(), 24), // sec_options
        abi::load_u64(abi::SCRATCH[0], abi::stack_pointer(), 32), // nw_release
        abi::branch_link_register(abi::SCRATCH[0]),
        abi::load_u64(abi::LOCAL[2], abi::stack_pointer(), 40),
        abi::load_u64(abi::LOCAL[1], abi::stack_pointer(), 16),
        abi::load_u64(abi::LOCAL[0], abi::stack_pointer(), 8),
        abi::load_u64(abi::link_register(), abi::stack_pointer(), 0),
        abi::add_stack(64),
        abi::return_(),
    ];
    CodeFunction {
        name: format!("runtime.{CFG_INVOKE}"),
        symbol: CFG_INVOKE.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame: frame(64),
        stack_slots: Vec::new(),
        instructions,
        relocations: Vec::new(),
    }
}

/// bug-477: the `sec_protocol_options_set_verify_block` block
/// `void(block @x0, sec_protocol_metadata_t @x1, sec_trust_t @x2,
/// sec_protocol_verify_complete_t @x3)`.
///
/// This is the whole macOS half of `allowSelfSigned`, and it is the one place
/// the bug is most likely to ship broken — an unconditional `complete(true)`
/// here would drop the hostname check on macOS only, silently, while every
/// positive fixture still passed. It does not do that. It re-runs the **full**
/// SSL trust evaluation with one thing changed: the anchor set is the
/// certificate the peer itself presented as its root, instead of the host trust
/// store. Hostname, `notBefore`/`notAfter` and the chain signatures are all
/// still evaluated by `SecTrustEvaluateWithError` under a
/// `SecPolicyCreateSSL(true, name)` policy, so a name mismatch and an expiry
/// still complete `false` and still fail the handshake.
///
/// ```c
/// SecTrustRef t = sec_trust_copy_ref(trust);
/// CFStringRef n = CFStringCreateWithCString(NULL, sname, kCFStringEncodingUTF8);
/// SecPolicyRef p = SecPolicyCreateSSL(true, n);
/// SecTrustSetPolicies(t, p);
/// CFArrayRef chain = SecTrustCopyCertificateChain(t);
/// bool ok = false;
/// if (chain && CFArrayGetCount(chain) > 0) {
///     const void *root = CFArrayGetValueAtIndex(chain, CFArrayGetCount(chain) - 1);
///     CFArrayRef a = CFArrayCreate(NULL, &root, 1, &kCFTypeArrayCallBacks);
///     SecTrustSetAnchorCertificates(t, a);
///     SecTrustSetAnchorCertificatesOnly(t, true);
///     ok = SecTrustEvaluateWithError(t, NULL);
///     CFRelease(a);
/// }
/// ... releases ...
/// complete(ok);
/// ```
///
/// It runs on the connection's dispatch queue — a **different thread** from the
/// MFB worker — so it touches no arena state (which is per-thread). Everything
/// it needs is either a block capture (the server name) or a process-global slot
/// read (the framework entry points).
fn verify_invoke_function() -> CodeFunction {
    // kCFStringEncodingUTF8.
    const UTF8: &str = "134217984";
    // Frame slots.
    const LR: usize = 0;
    const SAVE0: usize = 8;
    const SAVE1: usize = 16;
    const TRUST: usize = 24;
    const NAME: usize = 32;
    const POLICY: usize = 40;
    const CHAIN: usize = 48;
    const ROOT: usize = 56; // also the `&root` argument to CFArrayCreate
    const ANCHORS: usize = 64;
    const OK: usize = 72;
    const COMPLETE: usize = 80;
    const SAVE2: usize = 88;
    const FRAME: usize = 96;

    let no_chain = format!("{VERIFY_INVOKE}_no_chain");
    let release = format!("{VERIFY_INVOKE}_release");
    let mut relocations = Vec::new();
    let mut ins = vec![
        abi::label("entry"),
        abi::subtract_stack(FRAME),
        abi::store_u64(abi::link_register(), abi::stack_pointer(), LR),
        abi::store_u64(abi::LOCAL[0], abi::stack_pointer(), SAVE0),
        abi::store_u64(abi::LOCAL[1], abi::stack_pointer(), SAVE1),
        abi::store_u64(abi::LOCAL[2], abi::stack_pointer(), SAVE2),
        abi::store_u64(abi::c_arg(3), abi::stack_pointer(), COMPLETE),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), OK),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), CHAIN),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), ANCHORS),
        // LOCAL[1] = the captured server-name C string; LOCAL[2] = trust arg.
        abi::load_u64(abi::LOCAL[1], abi::c_arg(0), VERIFY_CAP_SNAME),
        abi::move_register(abi::LOCAL[2], abi::c_arg(2)),
    ];
    // LOCAL[0] = the resolved-entry-point table (callee-saved across every call).
    crate::codegen::memory::arena::emit_data_address(
        VERIFY_INVOKE,
        abi::LOCAL[0],
        VERIFY_FNS_SYMBOL,
        &mut ins,
        &mut relocations,
    );
    let call = |ins: &mut Vec<_>, name: &str| {
        ins.push(abi::load_u64(
            abi::SCRATCH[0],
            abi::LOCAL[0],
            verify_fn_slot(name),
        ));
        ins.push(abi::branch_link_register(abi::SCRATCH[0]));
    };
    // t = sec_trust_copy_ref(trust)
    ins.push(abi::move_register(abi::c_arg(0), abi::LOCAL[2]));
    call(&mut ins, "sec_trust_copy_ref");
    ins.push(abi::store_u64(
        abi::c_return(0),
        abi::stack_pointer(),
        TRUST,
    ));
    // n = CFStringCreateWithCString(NULL, sname, kCFStringEncodingUTF8)
    ins.extend([
        abi::move_immediate(abi::c_arg(0), "Integer", "0"),
        abi::move_register(abi::c_arg(1), abi::LOCAL[1]),
        abi::move_immediate(abi::c_arg(2), "Integer", UTF8),
    ]);
    call(&mut ins, "CFStringCreateWithCString");
    ins.push(abi::store_u64(abi::c_return(0), abi::stack_pointer(), NAME));
    // p = SecPolicyCreateSSL(true, n)   -- `true` = evaluate as a CLIENT checking
    // a SERVER, which is what makes the hostname argument load-bearing.
    ins.extend([
        abi::move_immediate(abi::c_arg(0), "Integer", "1"),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), NAME),
    ]);
    call(&mut ins, "SecPolicyCreateSSL");
    ins.push(abi::store_u64(
        abi::c_return(0),
        abi::stack_pointer(),
        POLICY,
    ));
    // SecTrustSetPolicies(t, p)
    ins.extend([
        abi::load_u64(abi::c_arg(0), abi::stack_pointer(), TRUST),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), POLICY),
    ]);
    call(&mut ins, "SecTrustSetPolicies");
    // chain = SecTrustCopyCertificateChain(t)
    ins.push(abi::load_u64(abi::c_arg(0), abi::stack_pointer(), TRUST));
    call(&mut ins, "SecTrustCopyCertificateChain");
    ins.extend([
        abi::store_u64(abi::c_return(0), abi::stack_pointer(), CHAIN),
        abi::compare_immediate(abi::c_return(0), "0"),
        abi::branch_eq(&no_chain),
    ]);
    // n = CFArrayGetCount(chain); if (n <= 0) skip
    ins.push(abi::load_u64(abi::c_arg(0), abi::stack_pointer(), CHAIN));
    call(&mut ins, "CFArrayGetCount");
    ins.extend([
        abi::move_register(abi::LOCAL[1], abi::c_return(0)),
        abi::compare_immediate(abi::LOCAL[1], "0"),
        abi::branch_le(&no_chain),
        // root = CFArrayGetValueAtIndex(chain, n - 1) -- the LAST element is what
        // the peer offered as its root. For a self-signed leaf that is the leaf;
        // for a private-CA chain it is the CA. Pinning the leaf instead would
        // accept a chain whose intermediate signatures do not check out.
        abi::load_u64(abi::c_arg(0), abi::stack_pointer(), CHAIN),
        abi::subtract_immediate(abi::c_arg(1), abi::LOCAL[1], 1),
    ]);
    call(&mut ins, "CFArrayGetValueAtIndex");
    ins.push(abi::store_u64(abi::c_return(0), abi::stack_pointer(), ROOT));
    // anchors = CFArrayCreate(NULL, &root, 1, &kCFTypeArrayCallBacks)
    ins.extend([
        abi::move_immediate(abi::c_arg(0), "Integer", "0"),
        abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), ROOT),
        abi::move_immediate(abi::c_arg(2), "Integer", "1"),
        abi::load_u64(
            abi::c_arg(3),
            abi::LOCAL[0],
            verify_fn_slot("kCFTypeArrayCallBacks"),
        ),
    ]);
    call(&mut ins, "CFArrayCreate");
    ins.push(abi::store_u64(
        abi::c_return(0),
        abi::stack_pointer(),
        ANCHORS,
    ));
    // SecTrustSetAnchorCertificates(t, anchors)
    ins.extend([
        abi::load_u64(abi::c_arg(0), abi::stack_pointer(), TRUST),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), ANCHORS),
    ]);
    call(&mut ins, "SecTrustSetAnchorCertificates");
    // SecTrustSetAnchorCertificatesOnly(t, true) -- without this the SYSTEM
    // anchors stay in play as well, which would be harmless but pointless; with
    // it the evaluation is exactly "does this chain stand up on its own terms".
    ins.extend([
        abi::load_u64(abi::c_arg(0), abi::stack_pointer(), TRUST),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
    ]);
    call(&mut ins, "SecTrustSetAnchorCertificatesOnly");
    // ok = SecTrustEvaluateWithError(t, NULL)
    ins.extend([
        abi::load_u64(abi::c_arg(0), abi::stack_pointer(), TRUST),
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),
    ]);
    call(&mut ins, "SecTrustEvaluateWithError");
    ins.push(abi::store_u64(abi::c_return(0), abi::stack_pointer(), OK));
    ins.push(abi::label(&no_chain));
    // Release anchors / chain / policy / name / trust, each only if non-NULL.
    for slot in [ANCHORS, CHAIN, POLICY, NAME, TRUST] {
        let skip = format!("{release}_{slot}");
        ins.extend([
            abi::load_u64(abi::c_arg(0), abi::stack_pointer(), slot),
            abi::compare_immediate(abi::c_arg(0), "0"),
            abi::branch_eq(&skip),
        ]);
        call(&mut ins, "CFRelease");
        ins.push(abi::label(&skip));
    }
    // complete(ok): a block invocation is (block->invoke)(block, arg).
    ins.extend([
        abi::load_u64(abi::LOCAL[1], abi::stack_pointer(), COMPLETE),
        abi::load_u64(abi::SCRATCH[0], abi::LOCAL[1], BLK_INVOKE),
        abi::move_register(abi::c_arg(0), abi::LOCAL[1]),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), OK),
        abi::branch_link_register(abi::SCRATCH[0]),
        abi::load_u64(abi::LOCAL[2], abi::stack_pointer(), SAVE2),
        abi::load_u64(abi::LOCAL[1], abi::stack_pointer(), SAVE1),
        abi::load_u64(abi::LOCAL[0], abi::stack_pointer(), SAVE0),
        abi::load_u64(abi::link_register(), abi::stack_pointer(), LR),
        abi::add_stack(FRAME),
        abi::return_(),
    ]);
    CodeFunction {
        name: format!("runtime.{VERIFY_INVOKE}"),
        symbol: VERIFY_INVOKE.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame: frame(FRAME),
        stack_slots: Vec::new(),
        instructions: ins,
        relocations,
    }
}

/// The listener new-connection block `void(block @x0, nw_connection_t @x1)`.
/// Single producer (the listener's serial dispatch queue) into the listener
/// context's ring: retain the connection, store it at `ring[head & mask]`,
/// bump `head`, and signal the semaphore. When the ring is full the
/// connection is neither retained nor signalled — the framework releases it
/// after the callback returns, refusing the connection (backpressure).
fn lconn_invoke_function() -> CodeFunction {
    let full = format!("{LCONN_INVOKE}_full");
    let mask = (LCTX_RING_CAP - 1).to_string();
    let instructions = vec![
        abi::label("entry"),
        abi::subtract_stack(32),
        abi::store_u64(abi::link_register(), abi::stack_pointer(), 0),
        abi::store_u64(abi::LOCAL[0], abi::stack_pointer(), 8),
        abi::store_u64(abi::LOCAL[1], abi::stack_pointer(), 16),
        abi::load_u64(abi::LOCAL[0], abi::c_arg(0), BLK_CAP), // x19 = lctx
        abi::move_register(abi::LOCAL[1], abi::c_arg(1)),     // x20 = connection
        // Full? head - tail >= capacity => drop (no retain, no signal).
        abi::load_u64(abi::SCRATCH[0], abi::LOCAL[0], LCTX_HEAD),
        abi::load_u64(abi::SCRATCH[1], abi::LOCAL[0], LCTX_TAIL),
        abi::subtract_registers(abi::SCRATCH[2], abi::SCRATCH[0], abi::SCRATCH[1]),
        abi::compare_immediate(abi::SCRATCH[2], &LCTX_RING_CAP.to_string()),
        abi::branch_ge(&full),
        // nw_retain(conn) so it survives past this callback.
        abi::load_u64(abi::SCRATCH[3], abi::LOCAL[0], CTX_RETAIN),
        abi::move_register(abi::c_arg(0), abi::LOCAL[1]),
        abi::branch_link_register(abi::SCRATCH[3]),
        // ring[head & mask] = conn; head += 1.
        abi::load_u64(abi::SCRATCH[0], abi::LOCAL[0], LCTX_HEAD),
        abi::move_immediate(abi::SCRATCH[3], "Integer", &mask),
        abi::and_registers(abi::SCRATCH[2], abi::SCRATCH[0], abi::SCRATCH[3]),
        abi::shift_left_immediate(abi::SCRATCH[2], abi::SCRATCH[2], 3),
        abi::add_immediate(abi::SCRATCH[3], abi::LOCAL[0], LCTX_RING),
        abi::add_registers(abi::SCRATCH[3], abi::SCRATCH[3], abi::SCRATCH[2]),
        abi::store_u64(abi::LOCAL[1], abi::SCRATCH[3], 0),
        abi::add_immediate(abi::SCRATCH[0], abi::SCRATCH[0], 1),
        abi::store_u64(abi::SCRATCH[0], abi::LOCAL[0], LCTX_HEAD),
        // signal(sem)
        abi::load_u64(abi::SCRATCH[1], abi::LOCAL[0], CTX_SIGNAL),
        abi::load_u64(abi::c_arg(0), abi::LOCAL[0], CTX_SEM),
        abi::branch_link_register(abi::SCRATCH[1]),
        abi::label(&full),
        abi::load_u64(abi::LOCAL[1], abi::stack_pointer(), 16),
        abi::load_u64(abi::LOCAL[0], abi::stack_pointer(), 8),
        abi::load_u64(abi::link_register(), abi::stack_pointer(), 0),
        abi::add_stack(32),
        abi::return_(),
    ];
    CodeFunction {
        name: format!("runtime.{LCONN_INVOKE}"),
        symbol: LCONN_INVOKE.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame: frame(32),
        stack_slots: Vec::new(),
        instructions,
        relocations: Vec::new(),
    }
}

/// The macOS Network.framework block trampolines, in the order the linker
/// expects (state, send, receive, configure, and — server only — the
/// listener's new-connection handler). Emitted only when the program uses
/// TLS; reached via `CodegenPlatform::emit_tls_block_trampolines`.
pub(crate) fn block_trampolines(server: bool) -> Vec<CodeFunction> {
    let mut trampolines = vec![
        // state_changed(state @x1, error @x2)
        invoke_function(STATE_INVOKE, &[("x1", CTX_STATE), ("x2", CTX_ERROR)]),
        // send_completion(error @x1)
        invoke_function(SEND_INVOKE, &[("x1", CTX_ERROR)]),
        recv_poll_invoke_function(),
        cfg_invoke_function(),
        verify_invoke_function(),
    ];
    if server {
        trampolines.push(lconn_invoke_function());
    }
    trampolines
}
