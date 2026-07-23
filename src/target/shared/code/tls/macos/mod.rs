use super::*;

const MACLIB: &str = "/System/Library/Frameworks/Network.framework/Network";
const MACLIB_SYMBOL: &str = "_mfb_tls_maclib";
// The server identity is built from the PEM pair via Security.framework
// (SecItemImport + SecIdentityCreate) and CoreFoundation (CFData/CFArray);
// both are dlopen'd only by the server path (plan-06-tls-server.md §7).
const MACSEC: &str = "/System/Library/Frameworks/Security.framework/Security";
const MACSEC_SYMBOL: &str = "_mfb_tls_macsec";
const MACCF: &str = "/System/Library/Frameworks/CoreFoundation.framework/CoreFoundation";
const MACCF_SYMBOL: &str = "_mfb_tls_maccf";
// An empty listen host binds all interfaces.
const ANYHOST: &str = "0.0.0.0";
const ANYHOST_SYMBOL: &str = "_mfb_tls_anyhost";
const QLABEL: &str = "mfb.tls";
const QLABEL_SYMBOL: &str = "_mfb_tls_qlabel";
const DESC_SYMBOL: &str = "_mfb_tls_block_desc";
// Descriptor for the larger SNI-config block (three captured pointers).
const CFG_DESC_SYMBOL: &str = "_mfb_tls_cfg_block_desc";
// The block `invoke` symbols. The block-building setup (this module) references
// them when filling each block's invoke field; the aarch64 backend defines their
// bodies (`target/macos_aarch64/tls.rs`) — hence `pub(crate)`.
pub(crate) const STATE_INVOKE: &str = "_mfb_tls_nw_state_invoke";
pub(crate) const SEND_INVOKE: &str = "_mfb_tls_nw_send_invoke";
pub(crate) const RECV_INVOKE: &str = "_mfb_tls_nw_recv_invoke";
// Configure-TLS block invoke: overrides the SNI / certificate-validation
// server name when `serverName` is supplied. The server path reuses the same
// trampoline shape to install the local identity: it captures
// (sec_identity, nw_tls_copy_sec_protocol_options,
// sec_protocol_options_set_local_identity) instead.
pub(crate) const CFG_INVOKE: &str = "_mfb_tls_nw_cfg_invoke";
// New-connection handler invoke for `tls::listen`: retains the inbound
// nw_connection into the listener context's ring and signals the semaphore.
pub(crate) const LCONN_INVOKE: &str = "_mfb_tls_nw_lconn_invoke";

// nw_connection_state_t
const NW_STATE_READY: &str = "3";
// nw_listener_state_t (distinct numbering from connection states)
const NW_LISTENER_READY: &str = "2";
const NW_LISTENER_FAILED: &str = "3";

// The handle record: nw_connection, closed flag, dispatch queue, ctx pointer.
// The `closed` flag sits at the canonical resource closed-flag offset 8
// (plan-38 F7) so the backend-independent closed-default (which zeroes the
// record and sets offset 8) marks this record closed too. Before plan-38 the
// closed flag was at offset 0 and offset 8 held `REC_CONN`; a closed-default
// record then read as *open* and `close` dereferenced offset 8 (=1) as the
// connection pointer → `nw_connection_cancel((void*)0x1)` SIGSEGV on the drop
// path. Swapping the two offsets fixes it and satisfies the shared assert. All
// record accesses go through these named constants, so the swap is transparent.
const REC_CONN: usize = 0;
const REC_CLOSED: usize = 8;
const REC_QUEUE: usize = 16;
const REC_CTX: usize = 24;
const REC_SIZE: &str = "32";

const _: () = assert!(REC_CLOSED == RESOURCE_OFFSET_CLOSED);

// The shared block context (arena): semaphore, the captured signal fn, and
// the slots each block writes before signaling.
// The ctx-slot layout is the shared contract between the block-building setup
// here and the trampoline bodies in the aarch64 backend — `pub(crate)` so both
// sides read one definition.
pub(crate) const CTX_SEM: usize = 0;
pub(crate) const CTX_SIGNAL: usize = 8;
pub(crate) const CTX_STATE: usize = 16;
pub(crate) const CTX_CONTENT: usize = 24;
pub(crate) const CTX_ERROR: usize = 32;
pub(crate) const CTX_RETAIN: usize = 40; // dispatch_retain, used by the receive block
const CTX_SIZE: &str = "48";

// The listener context extends the shared ctx prefix (the listener's
// state-changed handler is the plain STATE_INVOKE trampoline over the same
// slots) with a single-producer/single-consumer ring of pending retained
// nw_connections. The serial dispatch queue is the only producer; `tls::accept`
// on the owning thread is the only consumer; the semaphore signal/wait pair
// orders the slot writes. CTX_RETAIN holds `nw_retain` here (the conn handler
// retains each connection so it survives past the callback).
pub(crate) const LCTX_HEAD: usize = 48; // producer count (trampoline-owned)
pub(crate) const LCTX_TAIL: usize = 56; // consumer count (accept-owned)
pub(crate) const LCTX_RING: usize = 64; // LCTX_RING_CAP pointer slots
pub(crate) const LCTX_RING_CAP: usize = 16; // power of two (index mask 15)
const LCTX_SIZE: &str = "192"; // 64 + 16*8

// Block literal: isa, flags, invoke, descriptor, one captured ctx pointer.
const BLK_ISA: usize = 0;
const BLK_FLAGS: usize = 8;
const BLK_INVOKE: usize = 16;
const BLK_DESC: usize = 24;
pub(crate) const BLK_CAP: usize = 32;

// The SNI-config block captures four plain pointers after the 32-byte
// header: the server-name C string, the two resolved framework functions its
// invoke calls, and `nw_release` used to balance the `sec_protocol_options`
// the copy fn returns (+1). Total size 64 (see CFG_DESC_SYMBOL).
pub(crate) const CFG_CAP_SNAME: usize = 32;
pub(crate) const CFG_CAP_COPYFN: usize = 40;
pub(crate) const CFG_CAP_SETFN: usize = 48;
pub(crate) const CFG_CAP_RELEASEFN: usize = 56;

const SYMBOLS: &[&str] = &[
    "nw_endpoint_create_host",
    "nw_parameters_create_secure_tcp",
    "nw_connection_create",
    "nw_connection_set_queue",
    "nw_connection_set_state_changed_handler",
    "nw_connection_start",
    "nw_connection_send",
    "nw_connection_receive",
    "nw_connection_cancel",
    "nw_release",
    "dispatch_queue_create",
    "dispatch_semaphore_create",
    "dispatch_semaphore_signal",
    "dispatch_semaphore_wait",
    "dispatch_time",
    "dispatch_data_create",
    "dispatch_data_create_map",
    "dispatch_release",
    "dispatch_retain",
    "_NSConcreteStackBlock",
    "_nw_parameters_configure_protocol_default_configuration",
    "_nw_content_context_default_message",
    "nw_tls_copy_sec_protocol_options",
    "sec_protocol_options_set_tls_server_name",
];

/// The additional server-side entry points (`tls::listen`/`tls::accept`).
/// Their name strings are emitted only when a module uses a server helper, so
/// client-only programs stay byte-identical (plan-06-tls-server.md §1).
const SERVER_SYMBOLS: &[&str] = &[
    "nw_listener_create",
    "nw_listener_set_queue",
    "nw_listener_set_new_connection_handler",
    "nw_listener_set_state_changed_handler",
    "nw_listener_start",
    "nw_listener_cancel",
    "nw_parameters_set_local_endpoint",
    "nw_parameters_set_reuse_local_address",
    "nw_retain",
    "sec_identity_create",
    "sec_protocol_options_set_local_identity",
    "SecItemImport",
    "SecIdentityCreate",
    "CFDataCreate",
    "CFArrayGetCount",
    "CFArrayGetValueAtIndex",
    // bug-236: balance the +1 CFData/CFArray the PEM import creates, and own the
    // extracted cert/key ref across the array's release.
    "CFRetain",
    "CFRelease",
];

fn raw_cstr(symbol: &str, text: &str) -> CodeDataObject {
    CodeDataObject {
        symbol: symbol.to_string(),
        kind: "raw".to_string(),
        layout: "C string (NUL-terminated)".to_string(),
        align: 1,
        size: text.len() + 1,
        value: hex_encode_cstring(text),
    }
}

pub(super) fn data_objects(server: bool) -> Vec<CodeDataObject> {
    let mut objects = vec![
        raw_cstr(MACLIB_SYMBOL, MACLIB),
        raw_cstr(QLABEL_SYMBOL, QLABEL),
        CodeDataObject {
            symbol: DESC_SYMBOL.to_string(),
            kind: "raw".to_string(),
            layout: "Block_descriptor { u64 reserved=0; u64 size=40 }".to_string(),
            align: 8,
            size: 16,
            // reserved = 0, size = 40 (0x28), little-endian u64s
            value: "00000000000000002800000000000000".to_string(),
        },
        CodeDataObject {
            symbol: CFG_DESC_SYMBOL.to_string(),
            kind: "raw".to_string(),
            layout: "Block_descriptor { u64 reserved=0; u64 size=64 }".to_string(),
            align: 8,
            size: 16,
            // reserved = 0, size = 64 (0x40), little-endian u64s
            value: "00000000000000004000000000000000".to_string(),
        },
    ];
    for name in SYMBOLS {
        objects.push(raw_cstr(&sym_data_symbol(name), name));
    }
    if server {
        objects.push(raw_cstr(MACSEC_SYMBOL, MACSEC));
        objects.push(raw_cstr(MACCF_SYMBOL, MACCF));
        objects.push(raw_cstr(ANYHOST_SYMBOL, ANYHOST));
        for name in SERVER_SYMBOLS {
            objects.push(raw_cstr(&sym_data_symbol(name), name));
        }
    }
    objects
}

// The block-`invoke` trampoline bodies (STATE/SEND/RECV/CFG) are the
// foreign-runtime callback ABI realized as aarch64 instructions, so they live in
// the per-(OS, ISA) backend: `target/macos_aarch64/tls.rs`, reached via
// `CodegenPlatform::emit_tls_block_trampolines`. They consume the `pub(crate)`
// block/ctx layout above. A macOS-x86 backend supplies its own.

/// Emit a `dlsym(handle, name)` into `fnptr_off` (delegates to the parent).
fn dlsym(
    ctx: &mut EmitCtx,
    handle_off: usize,
    name: &str,
    fnptr_off: usize,
    fail: &str,
) -> Result<(), String> {
    let symbol = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    emit_dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: ctx.instructions,
            relocations: ctx.relocations,
        },
        handle_off,
        name,
        fnptr_off,
        fail,
    )
}

/// `nw_connection_cancel(conn)` then `nw_release(conn)` for the connection held
/// at `sp + conn_off`.
///
/// Cancelling stops the connection's network activity but does not drop the
/// caller's `+1` retain, so an error exit that only cancels leaks the
/// `nw_connection` object. Every connect/accept failure exit that owns a
/// connection uses this so its teardown matches the success/close path
/// (bug-317). `conn_off` is only reached once the slot holds a non-NULL
/// connection, so no null guard is needed.
fn emit_cancel_and_release_conn(
    ctx: &mut EmitCtx,
    handle_off: usize,
    conn_off: usize,
    fnptr_off: usize,
    fail: &str,
) -> Result<(), String> {
    let symbol = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    for name in ["nw_connection_cancel", "nw_release"] {
        dlsym(
            &mut EmitCtx {
                symbol,
                platform_imports,
                platform,
                instructions: ctx.instructions,
                relocations: ctx.relocations,
            },
            handle_off,
            name,
            fnptr_off,
            fail,
        )?;
        ctx.instructions.extend([
            abi::load_u64(abi::return_register(), abi::stack_pointer(), conn_off),
            abi::load_u64("%v9", abi::stack_pointer(), fnptr_off),
            abi::branch_link_register("%v9"),
        ]);
    }
    Ok(())
}

/// `dispatch_release(queue)` for the dispatch queue held at `sp + queue_off`.
///
/// Only for a queue this frame owns. An accepted socket shares the listener's
/// serial queue (released by `closeListener`), so its failure exits must not
/// call this or they would over-release a queue still in use.
fn emit_release_queue(
    ctx: &mut EmitCtx,
    handle_off: usize,
    queue_off: usize,
    fnptr_off: usize,
    fail: &str,
) -> Result<(), String> {
    let symbol = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: ctx.instructions,
            relocations: ctx.relocations,
        },
        handle_off,
        "dispatch_release",
        fnptr_off,
        fail,
    )?;
    ctx.instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), queue_off),
        abi::load_u64("%v9", abi::stack_pointer(), fnptr_off),
        abi::branch_link_register("%v9"),
    ]);
    Ok(())
}

/// Build a 40-byte block literal at `sp + block_off` whose `invoke` is
/// `invoke_symbol` and whose single captured variable is the ctx pointer at
/// `sp + ctx_off`.
fn emit_build_block(
    ctx: &mut EmitCtx,
    handle_off: usize,
    invoke_symbol: &str,
    ctx_off: usize,
    block_off: usize,
    fnptr_off: usize,
    fail: &str,
) -> Result<(), String> {
    let symbol = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: ctx.instructions,
            relocations: ctx.relocations,
        },
        handle_off,
        "_NSConcreteStackBlock",
        fnptr_off,
        fail,
    )?;
    ctx.instructions.extend([
        abi::load_u64("%v9", abi::stack_pointer(), fnptr_off),
        abi::store_u64("%v9", abi::stack_pointer(), block_off + BLK_ISA),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), block_off + BLK_FLAGS),
    ]);
    emit_data_address(
        symbol,
        "%v9",
        invoke_symbol,
        ctx.instructions,
        ctx.relocations,
    );
    ctx.instructions.push(abi::store_u64(
        "%v9",
        abi::stack_pointer(),
        block_off + BLK_INVOKE,
    ));
    emit_data_address(
        symbol,
        "%v9",
        DESC_SYMBOL,
        ctx.instructions,
        ctx.relocations,
    );
    ctx.instructions.push(abi::store_u64(
        "%v9",
        abi::stack_pointer(),
        block_off + BLK_DESC,
    ));
    ctx.instructions.extend([
        abi::load_u64("%v9", abi::stack_pointer(), ctx_off),
        abi::store_u64("%v9", abi::stack_pointer(), block_off + BLK_CAP),
    ]);
    Ok(())
}

/// Create a fresh semaphore into `ctx->sem` (so leftover signals from a prior
/// operation can't satisfy this wait), then `dispatch_semaphore_wait` is
/// emitted separately by the caller after the async op is launched. Resets the
/// ctx output slots.
///
/// The previous `ctx->sem` (created by connect/accept and replaced on every
/// prior readText/write) is `dispatch_release`d before the replacement is
/// stored. Without that release each read/write leaked one `dispatch_semaphore`
/// on both the success and error paths — `leaks` showed ~211k residual objects
/// over 200k reads (bug-55 follow-up to bug-52). The release is safe: every
/// operation performs exactly one `dispatch_semaphore_wait` (FOREVER) balanced
/// by exactly one signal from its completion block, so between operations the
/// semaphore's count is back at its initial 0 and disposing it cannot trip
/// libdispatch's "deallocated while in use" assertion. The slot is non-NULL
/// from connect onward, but the store is null-guarded for defence in depth
/// (`dispatch_release(NULL)` would crash).
fn emit_fresh_sem(
    ctx: &mut EmitCtx,
    handle_off: usize,
    ctx_off: usize,
    fnptr_off: usize,
    fail: &str,
) -> Result<(), String> {
    let symbol = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    // Release the semaphore left in ctx->sem by the previous operation.
    let skip_release = format!("{symbol}_sem_skip_release");
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: ctx.instructions,
            relocations: ctx.relocations,
        },
        handle_off,
        "dispatch_release",
        fnptr_off,
        fail,
    )?;
    ctx.instructions.extend([
        abi::load_u64("%v9", abi::stack_pointer(), ctx_off),
        abi::load_u64(abi::return_register(), "%v9", CTX_SEM),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&skip_release),
        abi::load_u64("%v9", abi::stack_pointer(), fnptr_off),
        abi::branch_link_register("%v9"),
        abi::label(&skip_release),
    ]);
    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: ctx.instructions,
            relocations: ctx.relocations,
        },
        handle_off,
        "dispatch_semaphore_create",
        fnptr_off,
        fail,
    )?;
    ctx.instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::load_u64("%v9", abi::stack_pointer(), fnptr_off),
        abi::branch_link_register("%v9"),
        abi::load_u64("%v9", abi::stack_pointer(), ctx_off),
        abi::store_u64(abi::return_register(), "%v9", CTX_SEM),
        abi::store_u64(abi::ZERO, "%v9", CTX_CONTENT),
        abi::store_u64(abi::ZERO, "%v9", CTX_ERROR),
    ]);
    Ok(())
}

/// Emit `dispatch_semaphore_wait(ctx->sem, FOREVER)`.
fn emit_wait(
    ctx: &mut EmitCtx,
    handle_off: usize,
    ctx_off: usize,
    fnptr_off: usize,
    fail: &str,
) -> Result<(), String> {
    let symbol = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    dlsym(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: ctx.instructions,
            relocations: ctx.relocations,
        },
        handle_off,
        "dispatch_semaphore_wait",
        fnptr_off,
        fail,
    )?;
    ctx.instructions.extend([
        abi::load_u64("%v9", abi::stack_pointer(), ctx_off),
        abi::load_u64(abi::return_register(), "%v9", CTX_SEM),
        abi::move_immediate(abi::ARG[1], "Integer", "0"),
        abi::bitwise_not(abi::ARG[1], abi::ARG[1]),
        abi::load_u64("%v10", abi::stack_pointer(), fnptr_off),
        abi::branch_link_register("%v10"),
    ]);
    Ok(())
}

fn emit_dlopen_maclib(ctx: &mut EmitCtx, handle_off: usize, fail: &str) -> Result<(), String> {
    let symbol = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    emit_dlopen_at(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: ctx.instructions,
            relocations: ctx.relocations,
        },
        MACLIB_SYMBOL,
        handle_off,
        fail,
    )
}

/// `dlopen` the framework named by the C-string data object `lib_symbol` into
/// `sp + handle_off`; branch to `fail` when it does not load.
fn emit_dlopen_at(
    ctx: &mut EmitCtx,
    lib_symbol: &str,
    handle_off: usize,
    fail: &str,
) -> Result<(), String> {
    let symbol = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    emit_data_address(
        symbol,
        abi::return_register(),
        lib_symbol,
        ctx.instructions,
        ctx.relocations,
    );
    ctx.instructions
        .push(abi::move_immediate(abi::ARG[1], "Integer", RTLD_NOW));
    platform.emit_libc_call(
        "dlopen",
        symbol,
        platform_imports,
        ctx.instructions,
        ctx.relocations,
    )?;
    ctx.instructions.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), handle_off),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(fail),
    ]);
    Ok(())
}

mod client;
mod server;
#[cfg(test)]
mod tests;

pub(super) use client::{
    lower_tls_close_macos, lower_tls_connect_macos, lower_tls_read_macos, lower_tls_write_macos,
};
pub(super) use server::{
    lower_tls_accept_macos, lower_tls_close_listener_macos, lower_tls_listen_macos,
};
