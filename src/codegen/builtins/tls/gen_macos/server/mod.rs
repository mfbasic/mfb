// --- codegen tier imports (migration) ---
use super::*;
use crate::codegen::engine::builder::emit_arena_free;
use crate::target::shared::abi;
use std::collections::HashMap;

mod accept;
mod close_listener;
mod identity;
mod listen;

pub(crate) use accept::lower_tls_accept_macos;
pub(crate) use close_listener::lower_tls_close_listener_macos;
pub(crate) use listen::lower_tls_listen_macos;
// ===========================================================================
// Server side: tls.listen / tls.accept / tls.closeListener
// (plan-06-tls-server.md §7)
// ===========================================================================

/// bug-412: block until the arena ctx at `sp + ctx_off` reaches its terminal
/// `cancelled` state (`cancelled_state`) before a cancel-and-return exit hands
/// control back to the program.
///
/// `nw_connection_cancel` / `nw_listener_cancel` transition asynchronously, and
/// the state-changed handler (STATE_INVOKE) dereferences the arena-allocated ctx
/// on *every* invocation. Letting an exit return before the object's final
/// `cancelled` transition fires lets that queued handler run against a freed ctx
/// after the program exits and the arena is torn down → EXC_BAD_ACCESS on the
/// shared `mfb.tls` serial queue (intermittent, load-dependent). `cancelled` is
/// terminal — nothing transitions after it — so spinning on the semaphore until
/// `ctx->state == cancelled_state` guarantees no handler runs afterward, no
/// matter how many transitions `cancel` produced or whether a leftover signal is
/// consumed first. The state constant differs by object: a connection reaches
/// `nw_connection_state_cancelled` = 5, a listener `nw_listener_state_cancelled`
/// = 4; both share the ctx prefix (`CTX_SEM` / `CTX_STATE`). `wait_off` must hold
/// a resolved `dispatch_semaphore_wait`; the wait is DISPATCH_TIME_FOREVER. This
/// mirrors the connect-path drain (`client/connect.rs`, bug-380).
pub(super) fn emit_cancel_drain(
    ins: &mut Vec<CodeInstruction>,
    ctx_off: usize,
    wait_off: usize,
    drain_label: &str,
    cancelled_state: &str,
    vregs: &mut Vregs,
) {
    let v9 = vregs.next();
    let v10 = vregs.next();
    ins.extend([
        abi::label(drain_label),
        abi::load_u64(&v9, abi::stack_pointer(), ctx_off),
        abi::load_u64(abi::return_register(), &v9, CTX_SEM),
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),
        abi::bitwise_not(abi::c_arg(1), abi::c_arg(1)), // DISPATCH_TIME_FOREVER
        abi::load_u64(&v10, abi::stack_pointer(), wait_off),
        abi::branch_link_register(&v10),
        abi::load_u64(&v9, abi::stack_pointer(), ctx_off),
        abi::load_u32(&v10, &v9, CTX_STATE),
        abi::compare_immediate(&v10, cancelled_state),
        abi::branch_ne(drain_label),
    ]);
}
