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
use crate::target::shared::code::tls::macos::{
    BLK_CAP, CFG_CAP_COPYFN, CFG_CAP_RELEASEFN, CFG_CAP_SETFN, CFG_CAP_SNAME, CFG_INVOKE,
    CTX_CONTENT, CTX_ERROR, CTX_RETAIN, CTX_SEM, CTX_SIGNAL, CTX_STATE, LCONN_INVOKE, LCTX_HEAD,
    LCTX_RING, LCTX_RING_CAP, LCTX_TAIL, RECV_INVOKE, SEND_INVOKE, STATE_INVOKE,
};
use crate::target::shared::code::{CodeFrame, CodeFunction};

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
        abi::load_u64("x9", "x0", BLK_CAP), // ctx = block->captured pointer
    ];
    for (reg, off) in stores {
        instructions.push(abi::store_u64(reg, "x9", *off));
    }
    instructions.extend([
        abi::load_u64("x10", "x9", CTX_SIGNAL),
        abi::load_u64("x0", "x9", CTX_SEM),
        abi::branch_link_register("x10"),
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
fn recv_invoke_function() -> CodeFunction {
    let sig = format!("{RECV_INVOKE}_sig");
    let instructions = vec![
        abi::label("entry"),
        abi::subtract_stack(32),
        abi::store_u64(abi::link_register(), abi::stack_pointer(), 0),
        abi::store_u64("x19", abi::stack_pointer(), 8),
        abi::move_register("x19", "x0"), // x19 = block; reload ctx below
        abi::load_u64("x19", "x19", BLK_CAP), // x19 = ctx (callee-saved across calls)
        abi::store_u64("x4", "x19", CTX_ERROR),
        abi::compare_immediate("x1", "0"),
        abi::branch_eq(&sig),
        abi::store_u64("x1", "x19", CTX_CONTENT),
        // dispatch_retain(content) so it survives past this block.
        abi::load_u64("x12", "x19", CTX_RETAIN),
        abi::move_register("x0", "x1"),
        abi::branch_link_register("x12"),
        abi::label(&sig),
        abi::load_u64("x10", "x19", CTX_SIGNAL),
        abi::load_u64("x0", "x19", CTX_SEM),
        abi::branch_link_register("x10"),
        abi::load_u64("x19", abi::stack_pointer(), 8),
        abi::load_u64(abi::link_register(), abi::stack_pointer(), 0),
        abi::add_stack(32),
        abi::return_(),
    ];
    CodeFunction {
        name: format!("runtime.{RECV_INVOKE}"),
        symbol: RECV_INVOKE.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame: frame(32),
        stack_slots: Vec::new(),
        instructions,
        relocations: Vec::new(),
    }
}

/// The configure-TLS block `void(block @x0, nw_protocol_options_t tls @x1)`.
/// It copies the TLS protocol's `sec_protocol_options`, then overrides the
/// server name used for SNI and certificate validation. The server-name C
/// string and the two framework functions are captured in the block (the
/// invoke is a static aux function and cannot embed per-call `dlsym`
/// results). Defaults still apply for everything it does not touch.
fn cfg_invoke_function() -> CodeFunction {
    let instructions = vec![
        abi::label("entry"),
        abi::subtract_stack(48),
        abi::store_u64(abi::link_register(), abi::stack_pointer(), 0),
        abi::store_u64("x19", abi::stack_pointer(), 8),
        abi::store_u64("x20", abi::stack_pointer(), 16),
        // x0 = block, x1 = tls_options. Preserve server name + setter across
        // the copy call (x0/x1 are clobbered by it). The release fn is stashed
        // to a stack slot now because the block pointer (x0) is clobbered too.
        abi::load_u64("x19", "x0", CFG_CAP_SNAME), // server name (cstr)
        abi::load_u64("x20", "x0", CFG_CAP_SETFN), // sec_protocol_options_set_tls_server_name
        abi::load_u64("x9", "x0", CFG_CAP_RELEASEFN), // nw_release
        abi::store_u64("x9", abi::stack_pointer(), 32),
        abi::load_u64("x9", "x0", CFG_CAP_COPYFN), // nw_tls_copy_sec_protocol_options
        abi::move_register("x0", "x1"),
        abi::branch_link_register("x9"), // x0 = sec_options (+1)
        abi::store_u64("x0", abi::stack_pointer(), 24), // survive the setter call
        abi::move_register("x1", "x19"),
        abi::branch_link_register("x20"), // set_tls_server_name(sec_options, name)
        // Balance the copy fn's +1 retain: nw_release(sec_options). The setter
        // is getter-style config and does not consume the ref (bug-116).
        abi::load_u64("x0", abi::stack_pointer(), 24), // sec_options
        abi::load_u64("x9", abi::stack_pointer(), 32), // nw_release
        abi::branch_link_register("x9"),
        abi::load_u64("x20", abi::stack_pointer(), 16),
        abi::load_u64("x19", abi::stack_pointer(), 8),
        abi::load_u64(abi::link_register(), abi::stack_pointer(), 0),
        abi::add_stack(48),
        abi::return_(),
    ];
    CodeFunction {
        name: format!("runtime.{CFG_INVOKE}"),
        symbol: CFG_INVOKE.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame: frame(48),
        stack_slots: Vec::new(),
        instructions,
        relocations: Vec::new(),
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
        abi::store_u64("x19", abi::stack_pointer(), 8),
        abi::store_u64("x20", abi::stack_pointer(), 16),
        abi::load_u64("x19", "x0", BLK_CAP), // x19 = lctx
        abi::move_register("x20", "x1"),     // x20 = connection
        // Full? head - tail >= capacity => drop (no retain, no signal).
        abi::load_u64("x9", "x19", LCTX_HEAD),
        abi::load_u64("x10", "x19", LCTX_TAIL),
        abi::subtract_registers("x11", "x9", "x10"),
        abi::compare_immediate("x11", &LCTX_RING_CAP.to_string()),
        abi::branch_ge(&full),
        // nw_retain(conn) so it survives past this callback.
        abi::load_u64("x12", "x19", CTX_RETAIN),
        abi::move_register("x0", "x20"),
        abi::branch_link_register("x12"),
        // ring[head & mask] = conn; head += 1.
        abi::load_u64("x9", "x19", LCTX_HEAD),
        abi::move_immediate("x12", "Integer", &mask),
        abi::and_registers("x11", "x9", "x12"),
        abi::shift_left_immediate("x11", "x11", 3),
        abi::add_immediate("x12", "x19", LCTX_RING),
        abi::add_registers("x12", "x12", "x11"),
        abi::store_u64("x20", "x12", 0),
        abi::add_immediate("x9", "x9", 1),
        abi::store_u64("x9", "x19", LCTX_HEAD),
        // signal(sem)
        abi::load_u64("x10", "x19", CTX_SIGNAL),
        abi::load_u64("x0", "x19", CTX_SEM),
        abi::branch_link_register("x10"),
        abi::label(&full),
        abi::load_u64("x20", abi::stack_pointer(), 16),
        abi::load_u64("x19", abi::stack_pointer(), 8),
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
        recv_invoke_function(),
        cfg_invoke_function(),
    ];
    if server {
        trampolines.push(lconn_invoke_function());
    }
    trampolines
}
