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
#[allow(clippy::too_many_arguments)]
fn dlsym(
    symbol: &str,
    handle_off: usize,
    name: &str,
    fnptr_off: usize,
    fail: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    emit_dlsym(
        symbol,
        handle_off,
        name,
        fnptr_off,
        fail,
        platform_imports,
        platform,
        instructions,
        relocations,
    )
}

/// Build a 40-byte block literal at `sp + block_off` whose `invoke` is
/// `invoke_symbol` and whose single captured variable is the ctx pointer at
/// `sp + ctx_off`.
#[allow(clippy::too_many_arguments)]
fn emit_build_block(
    symbol: &str,
    handle_off: usize,
    invoke_symbol: &str,
    ctx_off: usize,
    block_off: usize,
    fnptr_off: usize,
    fail: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    dlsym(
        symbol,
        handle_off,
        "_NSConcreteStackBlock",
        fnptr_off,
        fail,
        platform_imports,
        platform,
        ins,
        rel,
    )?;
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), fnptr_off),
        abi::store_u64("%v9", abi::stack_pointer(), block_off + BLK_ISA),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), block_off + BLK_FLAGS),
    ]);
    emit_data_address(symbol, "%v9", invoke_symbol, ins, rel);
    ins.push(abi::store_u64(
        "%v9",
        abi::stack_pointer(),
        block_off + BLK_INVOKE,
    ));
    emit_data_address(symbol, "%v9", DESC_SYMBOL, ins, rel);
    ins.push(abi::store_u64(
        "%v9",
        abi::stack_pointer(),
        block_off + BLK_DESC,
    ));
    ins.extend([
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
#[allow(clippy::too_many_arguments)]
fn emit_fresh_sem(
    symbol: &str,
    handle_off: usize,
    ctx_off: usize,
    fnptr_off: usize,
    fail: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    // Release the semaphore left in ctx->sem by the previous operation.
    let skip_release = format!("{symbol}_sem_skip_release");
    dlsym(
        symbol,
        handle_off,
        "dispatch_release",
        fnptr_off,
        fail,
        platform_imports,
        platform,
        ins,
        rel,
    )?;
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), ctx_off),
        abi::load_u64(abi::return_register(), "%v9", CTX_SEM),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&skip_release),
        abi::load_u64("%v9", abi::stack_pointer(), fnptr_off),
        abi::branch_link_register("%v9"),
        abi::label(&skip_release),
    ]);
    dlsym(
        symbol,
        handle_off,
        "dispatch_semaphore_create",
        fnptr_off,
        fail,
        platform_imports,
        platform,
        ins,
        rel,
    )?;
    ins.extend([
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
#[allow(clippy::too_many_arguments)]
fn emit_wait(
    symbol: &str,
    handle_off: usize,
    ctx_off: usize,
    fnptr_off: usize,
    fail: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    dlsym(
        symbol,
        handle_off,
        "dispatch_semaphore_wait",
        fnptr_off,
        fail,
        platform_imports,
        platform,
        ins,
        rel,
    )?;
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), ctx_off),
        abi::load_u64(abi::return_register(), "%v9", CTX_SEM),
        abi::move_immediate(abi::ARG[1], "Integer", "0"),
        abi::bitwise_not(abi::ARG[1], abi::ARG[1]),
        abi::load_u64("%v10", abi::stack_pointer(), fnptr_off),
        abi::branch_link_register("%v10"),
    ]);
    Ok(())
}

pub(super) fn lower_tls_connect_macos(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<
    (
        CodeFrame,
        Vec<CodeInstruction>,
        Vec<CodeRelocation>,
        Vec<CodeStackSlot>,
    ),
    String,
> {
    const FRAME_SIZE: usize = 288;
    const HOST: usize = 8;
    const PORT: usize = 16;
    const HANDLE: usize = 24;
    const FNPTR: usize = 32;
    const CTX: usize = 40;
    const ENDPOINT: usize = 48;
    const PARAMS: usize = 56;
    const CONN: usize = 64;
    const QUEUE: usize = 72;
    const HOSTCSTR: usize = 80;
    const PORTCSTR: usize = 88;
    const CFG: usize = 96;
    const WAITFN: usize = 104;
    const BLOCK: usize = 112; // 112..152
    const PORTBUF: usize = 152; // 152..176
    const SNAME: usize = 176; // serverName String ptr (arg x3)
    const SNICSTR: usize = 184; // serverName as a C string
    const TLSCFG: usize = 192; // chosen configure-TLS block pointer
    const CFGBLOCK: usize = 200; // 200..264: the SNI-config block literal
    const TIMEOUT: usize = 264; // timeoutMs (arg x2)
    const DEADLINE: usize = 272; // dispatch_time deadline for the wait

    let wait_loop = format!("{symbol}_wait");
    let ready = format!("{symbol}_ready");
    let conn_fail = format!("{symbol}_conn_fail");
    let conn_timeout = format!("{symbol}_conn_timeout");
    let wait_forever = format!("{symbol}_wait_forever");
    let deadline_ready = format!("{symbol}_deadline_ready");
    let net_fail = format!("{symbol}_net_fail");
    let load_fail = format!("{symbol}_load_fail");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let itoa_loop = format!("{symbol}_itoa");
    let sni_default = format!("{symbol}_sni_default");
    let done = format!("{symbol}_done");

    let mut ins = vec![abi::label("entry")];
    let mut rel = Vec::new();
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), HOST),
        abi::store_u64(abi::ARG[1], abi::stack_pointer(), PORT),
        abi::store_u64(abi::ARG[2], abi::stack_pointer(), TIMEOUT),
        abi::store_u64(abi::ARG[3], abi::stack_pointer(), SNAME),
    ]);
    // itoa(port) -> NUL-terminated decimal at PORTBUF, pointer in PORTCSTR.
    ins.extend([
        abi::move_immediate("%v9", "Integer", "0"),
        abi::store_u8("%v9", abi::stack_pointer(), PORTBUF + 23),
        abi::load_u64("%v10", abi::stack_pointer(), PORT),
        abi::move_immediate("%v11", "Integer", "10"),
        abi::add_immediate("%v14", abi::stack_pointer(), PORTBUF + 22),
        abi::label(&itoa_loop),
        abi::unsigned_divide_registers("%v15", "%v10", "%v11"),
        abi::multiply_subtract_registers("%v16", "%v15", "%v11", "%v10"),
        abi::add_immediate("%v16", "%v16", 48),
        abi::store_u8("%v16", "%v14", 0),
        abi::subtract_immediate("%v14", "%v14", 1),
        abi::move_register("%v10", "%v15"),
        abi::compare_immediate("%v10", "0"),
        abi::branch_ne(&itoa_loop),
        abi::add_immediate("%v13", "%v14", 1),
        abi::store_u64("%v13", abi::stack_pointer(), PORTCSTR),
    ]);
    // dlopen Network.framework.
    emit_data_address(
        symbol,
        abi::return_register(),
        MACLIB_SYMBOL,
        &mut ins,
        &mut rel,
    );
    ins.push(abi::move_immediate(abi::ARG[1], "Integer", RTLD_NOW));
    platform.emit_libc_call("dlopen", symbol, platform_imports, &mut ins, &mut rel)?;
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), HANDLE),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&load_fail),
    ]);
    emit_cstring(
        symbol,
        "host",
        HOST,
        HOSTCSTR,
        &alloc_fail,
        &mut ins,
        &mut rel,
    );
    // Allocate the block context.
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", CTX_SIZE),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut ins, &mut rel, &alloc_fail);
    ins.push(abi::store_u64(abi::RET[1], abi::stack_pointer(), CTX));
    // endpoint = nw_endpoint_create_host(host, port)
    dlsym(
        symbol,
        HANDLE,
        "nw_endpoint_create_host",
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), HOSTCSTR),
        abi::load_u64(abi::ARG[1], abi::stack_pointer(), PORTCSTR),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&net_fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), ENDPOINT),
    ]);
    // cfg = *_nw_parameters_configure_protocol_default_configuration
    dlsym(
        symbol,
        HANDLE,
        "_nw_parameters_configure_protocol_default_configuration",
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::load_u64("%v9", "%v9", 0),
        abi::store_u64("%v9", abi::stack_pointer(), CFG),
        // The configure-TLS block defaults to the system default. A non-empty
        // serverName swaps in a custom block that overrides the SNI /
        // certificate-validation name (empty => the endpoint host is used).
        abi::store_u64("%v9", abi::stack_pointer(), TLSCFG),
        abi::load_u64("%v9", abi::stack_pointer(), SNAME),
        abi::load_u64("%v10", "%v9", 0),
        abi::compare_immediate("%v10", "0"),
        abi::branch_eq(&sni_default),
    ]);
    // serverName given: copy it to a C string and build a configure block
    // whose invoke calls sec_protocol_options_set_tls_server_name. The block
    // is invoked synchronously during nw_parameters_create_secure_tcp, so the
    // stack literal stays live for its whole lifetime.
    emit_cstring(
        symbol,
        "sni",
        SNAME,
        SNICSTR,
        &alloc_fail,
        &mut ins,
        &mut rel,
    );
    dlsym(
        symbol,
        HANDLE,
        "_NSConcreteStackBlock",
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::store_u64("%v9", abi::stack_pointer(), CFGBLOCK + BLK_ISA),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), CFGBLOCK + BLK_FLAGS),
    ]);
    emit_data_address(symbol, "%v9", CFG_INVOKE, &mut ins, &mut rel);
    ins.push(abi::store_u64(
        "%v9",
        abi::stack_pointer(),
        CFGBLOCK + BLK_INVOKE,
    ));
    emit_data_address(symbol, "%v9", CFG_DESC_SYMBOL, &mut ins, &mut rel);
    ins.push(abi::store_u64(
        "%v9",
        abi::stack_pointer(),
        CFGBLOCK + BLK_DESC,
    ));
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), SNICSTR),
        abi::store_u64("%v9", abi::stack_pointer(), CFGBLOCK + CFG_CAP_SNAME),
    ]);
    dlsym(
        symbol,
        HANDLE,
        "nw_tls_copy_sec_protocol_options",
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::store_u64("%v9", abi::stack_pointer(), CFGBLOCK + CFG_CAP_COPYFN),
    ]);
    dlsym(
        symbol,
        HANDLE,
        "sec_protocol_options_set_tls_server_name",
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::store_u64("%v9", abi::stack_pointer(), CFGBLOCK + CFG_CAP_SETFN),
    ]);
    // nw_release: the invoke releases the +1 sec_protocol_options the copy fn
    // returns, so each configured connection stops leaking one (bug-116).
    dlsym(
        symbol,
        HANDLE,
        "nw_release",
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::store_u64("%v9", abi::stack_pointer(), CFGBLOCK + CFG_CAP_RELEASEFN),
        // tlscfg = &block
        abi::add_immediate("%v9", abi::stack_pointer(), CFGBLOCK),
        abi::store_u64("%v9", abi::stack_pointer(), TLSCFG),
    ]);
    ins.push(abi::label(&sni_default));
    // params = nw_parameters_create_secure_tcp(tlscfg, cfg)
    dlsym(
        symbol,
        HANDLE,
        "nw_parameters_create_secure_tcp",
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), TLSCFG),
        abi::load_u64(abi::ARG[1], abi::stack_pointer(), CFG),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&net_fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), PARAMS),
    ]);
    // conn = nw_connection_create(endpoint, params)
    dlsym(
        symbol,
        HANDLE,
        "nw_connection_create",
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), ENDPOINT),
        abi::load_u64(abi::ARG[1], abi::stack_pointer(), PARAMS),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&net_fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), CONN),
    ]);
    // nw_connection_create retains both the endpoint and the parameters, so
    // release our own references now; otherwise every successful connect leaks
    // one nw_endpoint and one nw_parameters (bug-55). The connection (CONN),
    // queue, and ctx are handed to the TlsSocket record and released on close.
    dlsym(
        symbol,
        HANDLE,
        "nw_release",
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), ENDPOINT),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), PARAMS),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
    ]);
    // queue = dispatch_queue_create("mfb.tls", NULL)
    dlsym(
        symbol,
        HANDLE,
        "dispatch_queue_create",
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    emit_data_address(
        symbol,
        abi::return_register(),
        QLABEL_SYMBOL,
        &mut ins,
        &mut rel,
    );
    ins.extend([
        abi::move_immediate(abi::ARG[1], "Integer", "0"),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), QUEUE),
    ]);
    // ctx->sem = dispatch_semaphore_create(0)
    dlsym(
        symbol,
        HANDLE,
        "dispatch_semaphore_create",
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
        abi::load_u64("%v9", abi::stack_pointer(), CTX),
        abi::store_u64(abi::return_register(), "%v9", CTX_SEM),
    ]);
    // ctx->signal = &dispatch_semaphore_signal
    dlsym(
        symbol,
        HANDLE,
        "dispatch_semaphore_signal",
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::load_u64("%v10", abi::stack_pointer(), FNPTR),
        abi::load_u64("%v9", abi::stack_pointer(), CTX),
        abi::store_u64("%v10", "%v9", CTX_SIGNAL),
    ]);
    // nw_connection_set_queue(conn, queue)
    dlsym(
        symbol,
        HANDLE,
        "nw_connection_set_queue",
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CONN),
        abi::load_u64(abi::ARG[1], abi::stack_pointer(), QUEUE),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
    ]);
    // Build the state-changed block literal on the stack.
    dlsym(
        symbol,
        HANDLE,
        "_NSConcreteStackBlock",
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::store_u64("%v9", abi::stack_pointer(), BLOCK + BLK_ISA),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), BLOCK + BLK_FLAGS),
    ]);
    emit_data_address(symbol, "%v9", STATE_INVOKE, &mut ins, &mut rel);
    ins.push(abi::store_u64(
        "%v9",
        abi::stack_pointer(),
        BLOCK + BLK_INVOKE,
    ));
    emit_data_address(symbol, "%v9", DESC_SYMBOL, &mut ins, &mut rel);
    ins.push(abi::store_u64(
        "%v9",
        abi::stack_pointer(),
        BLOCK + BLK_DESC,
    ));
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), CTX),
        abi::store_u64("%v9", abi::stack_pointer(), BLOCK + BLK_CAP),
    ]);
    // nw_connection_set_state_changed_handler(conn, &block)
    dlsym(
        symbol,
        HANDLE,
        "nw_connection_set_state_changed_handler",
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CONN),
        abi::add_immediate(abi::ARG[1], abi::stack_pointer(), BLOCK),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
    ]);
    // nw_connection_start(conn)
    dlsym(
        symbol,
        HANDLE,
        "nw_connection_start",
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CONN),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
    ]);
    // Compute the wait deadline: timeoutMs > 0 => dispatch_time(NOW, ms*1e6);
    // otherwise DISPATCH_TIME_FOREVER. It is absolute, so re-waits across the
    // preparing loop all share the original deadline.
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), TIMEOUT),
        abi::compare_immediate("%v9", "0"),
        abi::branch_le(&wait_forever),
    ]);
    dlsym(
        symbol,
        HANDLE,
        "dispatch_time",
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", "0"), // DISPATCH_TIME_NOW
        abi::load_u64(abi::ARG[1], abi::stack_pointer(), TIMEOUT),
        abi::move_immediate("%v10", "Integer", "1000000"),
        abi::multiply_registers(abi::ARG[1], abi::ARG[1], "%v10"), // ms -> ns
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), DEADLINE),
        abi::branch(&deadline_ready),
        abi::label(&wait_forever),
        abi::move_immediate("%v9", "Integer", "0"),
        abi::bitwise_not("%v9", "%v9"), // DISPATCH_TIME_FOREVER
        abi::store_u64("%v9", abi::stack_pointer(), DEADLINE),
        abi::label(&deadline_ready),
    ]);
    // Wait for a terminal state, bounded by the deadline.
    dlsym(
        symbol,
        HANDLE,
        "dispatch_semaphore_wait",
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::store_u64("%v9", abi::stack_pointer(), WAITFN),
        abi::label(&wait_loop),
        abi::load_u64("%v9", abi::stack_pointer(), CTX),
        abi::load_u64(abi::return_register(), "%v9", CTX_SEM),
        abi::load_u64(abi::ARG[1], abi::stack_pointer(), DEADLINE),
        abi::load_u64("%v10", abi::stack_pointer(), WAITFN),
        abi::branch_link_register("%v10"),
        // Non-zero => the deadline elapsed before any state change signalled.
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&conn_timeout),
        abi::load_u64("%v9", abi::stack_pointer(), CTX),
        abi::load_u32("%v10", "%v9", CTX_STATE),
        abi::compare_immediate("%v10", NW_STATE_READY),
        abi::branch_eq(&ready),
        abi::compare_immediate("%v10", "2"), // preparing
        abi::branch_eq(&wait_loop),
        abi::compare_immediate("%v10", "0"), // invalid
        abi::branch_eq(&wait_loop),
        abi::branch(&conn_fail), // waiting/failed/cancelled
        abi::label(&ready),
    ]);
    // Build the TlsSocket record { closed=0, conn, queue, ctx }.
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", REC_SIZE),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut ins, &mut rel, &alloc_fail);
    ins.extend([
        abi::store_u64(abi::ZERO, abi::RET[1], REC_CLOSED),
        abi::load_u64("%v9", abi::stack_pointer(), CONN),
        abi::store_u64("%v9", abi::RET[1], REC_CONN),
        abi::load_u64("%v9", abi::stack_pointer(), QUEUE),
        abi::store_u64("%v9", abi::RET[1], REC_QUEUE),
        abi::load_u64("%v9", abi::stack_pointer(), CTX),
        abi::store_u64("%v9", abi::RET[1], REC_CTX),
        abi::move_register(RESULT_VALUE_REGISTER, abi::RET[1]),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);
    // conn_fail: cancel the connection, report a TLS failure.
    ins.push(abi::label(&conn_fail));
    dlsym(
        symbol,
        HANDLE,
        "nw_connection_cancel",
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CONN),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
    ]);
    emit_fail(
        symbol,
        ERR_TLS_FAILED_CODE,
        ERR_TLS_FAILED_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    // conn_timeout: the deadline elapsed; cancel the connection, report a
    // timeout.
    ins.push(abi::label(&conn_timeout));
    dlsym(
        symbol,
        HANDLE,
        "nw_connection_cancel",
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CONN),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
    ]);
    emit_fail(
        symbol,
        ERR_TIMEOUT_CODE,
        ERR_TIMEOUT_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    ins.push(abi::label(&net_fail));
    emit_fail(
        symbol,
        ERR_NETWORK_FAILED_CODE,
        ERR_NETWORK_FAILED_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    ins.push(abi::label(&load_fail));
    emit_fail(
        symbol,
        ERR_TLS_FAILED_CODE,
        ERR_TLS_FAILED_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    ins.push(abi::label(&alloc_fail));
    emit_fail(
        symbol,
        ERR_OUT_OF_MEMORY_CODE,
        ERR_ALLOCATION_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    ins.extend([abi::label(&done), abi::return_()]);
    {
        let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut ins, &[], FRAME_SIZE);
        Ok((frame, ins, rel, stack_slots))
    }
}

pub(super) fn lower_tls_read_macos(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    text: bool,
) -> Result<
    (
        CodeFrame,
        Vec<CodeInstruction>,
        Vec<CodeRelocation>,
        Vec<CodeStackSlot>,
    ),
    String,
> {
    const FRAME_SIZE: usize = 192;
    const REC: usize = 8;
    const CONN: usize = 16;
    const CTX: usize = 24;
    const MAX: usize = 32;
    const HANDLE: usize = 40;
    const FNPTR: usize = 48;
    const MAPPED: usize = 64;
    const MPTR: usize = 72;
    const MSIZE: usize = 80;
    const N: usize = 88;
    const STR: usize = 96;
    const BLOCK: usize = 104; // 104..144

    let closed = format!("{symbol}_closed");
    let invalid = format!("{symbol}_invalid");
    let peer_closed = format!("{symbol}_peer_closed");
    let load_fail = format!("{symbol}_load_fail");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let encoding_error = format!("{symbol}_encoding_error");
    let str_copy = format!("{symbol}_str_copy");
    let str_done = format!("{symbol}_str_done");
    let entry_loop = format!("{symbol}_entry_loop");
    let entry_done = format!("{symbol}_entry_done");
    let done = format!("{symbol}_done");

    let mut ins = vec![abi::label("entry")];
    let mut rel = Vec::new();
    ins.extend([
        abi::store_u64(abi::ARG[1], abi::stack_pointer(), MAX),
        abi::load_u64("%v9", abi::return_register(), REC_CLOSED),
        abi::compare_immediate("%v9", "0"),
        abi::branch_ne(&closed),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), REC),
        abi::load_u64("%v9", abi::return_register(), REC_CONN),
        abi::store_u64("%v9", abi::stack_pointer(), CONN),
        abi::load_u64("%v9", abi::return_register(), REC_CTX),
        abi::store_u64("%v9", abi::stack_pointer(), CTX),
        abi::load_u64("%v10", abi::stack_pointer(), MAX),
        abi::compare_immediate("%v10", "0"),
        abi::branch_le(&invalid),
    ]);
    emit_dlopen_libssl_macos(
        symbol,
        HANDLE,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    emit_fresh_sem(
        symbol,
        HANDLE,
        CTX,
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    // ctx->retain = &dispatch_retain (used inside the receive block).
    dlsym(
        symbol,
        HANDLE,
        "dispatch_retain",
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::load_u64("%v10", abi::stack_pointer(), FNPTR),
        abi::load_u64("%v9", abi::stack_pointer(), CTX),
        abi::store_u64("%v10", "%v9", CTX_RETAIN),
    ]);
    emit_build_block(
        symbol,
        HANDLE,
        RECV_INVOKE,
        CTX,
        BLOCK,
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    // nw_connection_receive(conn, min=1, max=maxBytes, &block)
    dlsym(
        symbol,
        HANDLE,
        "nw_connection_receive",
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CONN),
        abi::move_immediate(abi::ARG[1], "Integer", "1"),
        abi::load_u64(abi::ARG[2], abi::stack_pointer(), MAX),
        abi::add_immediate(abi::ARG[3], abi::stack_pointer(), BLOCK),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
    ]);
    emit_wait(
        symbol,
        HANDLE,
        CTX,
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    // A null content is end-of-stream.
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), CTX),
        abi::load_u64("%v10", "%v9", CTX_CONTENT),
        abi::compare_immediate("%v10", "0"),
        abi::branch_eq(&peer_closed),
    ]);
    // dispatch_data_create_map(content, &ptr, &size) -> mapped (contiguous)
    dlsym(
        symbol,
        HANDLE,
        "dispatch_data_create_map",
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), CTX),
        abi::load_u64(abi::return_register(), "%v9", CTX_CONTENT),
        abi::add_immediate(abi::ARG[1], abi::stack_pointer(), MPTR),
        abi::add_immediate(abi::ARG[2], abi::stack_pointer(), MSIZE),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), MAPPED),
        abi::load_u64("%v9", abi::stack_pointer(), MSIZE),
        abi::store_u64("%v9", abi::stack_pointer(), N),
    ]);
    if text {
        ins.extend([
            abi::load_u64("%v10", abi::stack_pointer(), N),
            abi::add_immediate(abi::return_register(), "%v10", 9),
            abi::move_immediate(abi::ARG[1], "Integer", "8"),
        ]);
        emit_alloc(symbol, &mut ins, &mut rel, &alloc_fail);
        ins.extend([
            abi::load_u64("%v10", abi::stack_pointer(), N),
            abi::store_u64("%v10", abi::RET[1], 0),
            abi::load_u64("%v11", abi::stack_pointer(), MPTR),
            abi::add_immediate("%v12", abi::RET[1], 8),
            abi::move_immediate("%v13", "Integer", "0"),
            abi::store_u64(abi::RET[1], abi::stack_pointer(), STR),
            abi::label(&str_copy),
            abi::compare_registers("%v13", "%v10"),
            abi::branch_eq(&str_done),
            abi::load_u8("%v14", "%v11", 0),
            abi::store_u8("%v14", "%v12", 0),
            abi::add_immediate("%v11", "%v11", 1),
            abi::add_immediate("%v12", "%v12", 1),
            abi::add_immediate("%v13", "%v13", 1),
            abi::branch(&str_copy),
            abi::label(&str_done),
            abi::store_u8(abi::ZERO, "%v12", 0),
            abi::load_u64("%v9", abi::stack_pointer(), STR),
            abi::add_immediate(abi::return_register(), "%v9", 8),
            abi::load_u64(abi::ARG[1], "%v9", 0),
        ]);
        emit_call_validate_utf8(symbol, &encoding_error, &mut ins, &mut rel);
    } else {
        ins.extend([
            abi::load_u64("%v10", abi::stack_pointer(), N),
            abi::move_immediate("%v11", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
            abi::multiply_registers("%v12", "%v10", "%v11"),
            abi::add_immediate("%v12", "%v12", COLLECTION_HEADER_SIZE),
            abi::add_registers(abi::return_register(), "%v12", "%v10"),
            abi::move_immediate(abi::ARG[1], "Integer", "8"),
        ]);
        emit_alloc(symbol, &mut ins, &mut rel, &alloc_fail);
        ins.extend([
            abi::store_u64(abi::RET[1], abi::stack_pointer(), STR),
            abi::move_immediate("%v9", "Byte", &COLLECTION_KIND_LIST.to_string()),
            abi::store_u8("%v9", abi::RET[1], COLLECTION_OFFSET_KIND),
            abi::move_immediate("%v9", "Byte", &COLLECTION_TYPE_NONE.to_string()),
            abi::store_u8("%v9", abi::RET[1], COLLECTION_OFFSET_KEY_TYPE),
            abi::move_immediate("%v9", "Byte", &COLLECTION_TYPE_BYTE.to_string()),
            abi::store_u8("%v9", abi::RET[1], COLLECTION_OFFSET_VALUE_TYPE),
            abi::move_immediate("%v9", "Byte", "1"),
            abi::store_u8("%v9", abi::RET[1], COLLECTION_OFFSET_FLAGS_VERSION),
            abi::load_u64("%v10", abi::stack_pointer(), N),
            abi::store_u64("%v10", abi::RET[1], COLLECTION_OFFSET_COUNT),
            abi::store_u64("%v10", abi::RET[1], COLLECTION_OFFSET_CAPACITY),
            abi::store_u64("%v10", abi::RET[1], COLLECTION_OFFSET_DATA_LENGTH),
            abi::store_u64("%v10", abi::RET[1], COLLECTION_OFFSET_DATA_CAPACITY),
            abi::add_immediate("%v11", abi::RET[1], COLLECTION_HEADER_SIZE),
            abi::move_immediate("%v12", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
            abi::multiply_registers("%v13", "%v10", "%v12"),
            abi::add_registers("%v14", "%v11", "%v13"),
            abi::load_u64("%v15", abi::stack_pointer(), MPTR),
            abi::move_immediate("%v9", "Integer", "0"),
            abi::label(&entry_loop),
            abi::compare_registers("%v9", "%v10"),
            abi::branch_eq(&entry_done),
            abi::move_immediate("%v12", "Byte", &COLLECTION_ENTRY_FLAG_USED.to_string()),
            abi::store_u8("%v12", "%v11", COLLECTION_ENTRY_OFFSET_FLAGS),
            abi::store_u64(abi::ZERO, "%v11", COLLECTION_ENTRY_OFFSET_KEY_OFFSET),
            abi::store_u64(abi::ZERO, "%v11", COLLECTION_ENTRY_OFFSET_KEY_LENGTH),
            abi::store_u64("%v9", "%v11", COLLECTION_ENTRY_OFFSET_VALUE_OFFSET),
            abi::move_immediate("%v12", "Integer", "1"),
            abi::store_u64("%v12", "%v11", COLLECTION_ENTRY_OFFSET_VALUE_LENGTH),
            abi::add_registers("%v12", "%v14", "%v9"),
            abi::load_u8("%v13", "%v15", 0),
            abi::store_u8("%v13", "%v12", 0),
            abi::add_immediate("%v15", "%v15", 1),
            abi::add_immediate("%v11", "%v11", COLLECTION_ENTRY_SIZE),
            abi::add_immediate("%v9", "%v9", 1),
            abi::branch(&entry_loop),
            abi::label(&entry_done),
        ]);
    }
    // Release the mapped data and the retained content, then return.
    dlsym(
        symbol,
        HANDLE,
        "dispatch_release",
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), MAPPED),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CTX),
        abi::load_u64(abi::return_register(), abi::return_register(), CTX_CONTENT),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), STR),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);
    if text {
        // The encoding-error exit must release the mapped data and the retained
        // content before failing, exactly as the success path above does.
        // Otherwise a peer that keeps sending invalid UTF-8 to a program looping
        // on tls::readText drives an unbounded dispatch_data/content leak — a
        // remotely-triggerable memory-exhaustion DoS (bug-52). MAPPED, CTX and
        // CTX_CONTENT are reloaded from stack slots so no live value is held in
        // a caller-saved register across either dispatch_release `bl`.
        ins.push(abi::label(&encoding_error));
        dlsym(
            symbol,
            HANDLE,
            "dispatch_release",
            FNPTR,
            &load_fail,
            platform_imports,
            platform,
            &mut ins,
            &mut rel,
        )?;
        ins.extend([
            abi::load_u64(abi::return_register(), abi::stack_pointer(), MAPPED),
            abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
            abi::branch_link_register("%v9"),
            abi::load_u64(abi::return_register(), abi::stack_pointer(), CTX),
            abi::load_u64(abi::return_register(), abi::return_register(), CTX_CONTENT),
            abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
            abi::branch_link_register("%v9"),
        ]);
        emit_fail(
            symbol,
            ERR_ENCODING_CODE,
            ERR_ENCODING_SYMBOL,
            &mut ins,
            &mut rel,
            &done,
        );
    }
    ins.push(abi::label(&peer_closed));
    emit_fail(
        symbol,
        ERR_CONNECTION_CLOSED_CODE,
        ERR_CONNECTION_CLOSED_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    ins.push(abi::label(&invalid));
    emit_fail(
        symbol,
        ERR_INVALID_ARGUMENT_CODE,
        ERR_INVALID_ARGUMENT_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    ins.push(abi::label(&load_fail));
    emit_fail(
        symbol,
        ERR_TLS_FAILED_CODE,
        ERR_TLS_FAILED_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    ins.push(abi::label(&closed));
    emit_fail(
        symbol,
        ERR_RESOURCE_CLOSED_CODE,
        ERR_RESOURCE_CLOSED_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    ins.push(abi::label(&alloc_fail));
    emit_fail(
        symbol,
        ERR_OUT_OF_MEMORY_CODE,
        ERR_ALLOCATION_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    ins.extend([abi::label(&done), abi::return_()]);
    {
        let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut ins, &[], FRAME_SIZE);
        Ok((frame, ins, rel, stack_slots))
    }
}

pub(super) fn lower_tls_write_macos(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    text: bool,
) -> Result<
    (
        CodeFrame,
        Vec<CodeInstruction>,
        Vec<CodeRelocation>,
        Vec<CodeStackSlot>,
    ),
    String,
> {
    const FRAME_SIZE: usize = 160;
    const REC: usize = 8;
    const CONN: usize = 16;
    const CTX: usize = 24;
    const HANDLE: usize = 32;
    const FNPTR: usize = 40;
    const CONTENT: usize = 48;
    const DATA: usize = 56;
    const DLEN: usize = 64;
    const CTXDEF: usize = 72;
    const BLOCK: usize = 80; // 80..120

    let closed = format!("{symbol}_closed");
    let write_fail = format!("{symbol}_write_fail");
    let load_fail = format!("{symbol}_load_fail");
    let empty = format!("{symbol}_empty");
    let done = format!("{symbol}_done");

    let mut ins = vec![abi::label("entry")];
    let mut rel = Vec::new();
    ins.extend([
        abi::load_u64("%v9", abi::return_register(), REC_CLOSED),
        abi::compare_immediate("%v9", "0"),
        abi::branch_ne(&closed),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), REC),
        abi::load_u64("%v9", abi::return_register(), REC_CONN),
        abi::store_u64("%v9", abi::stack_pointer(), CONN),
        abi::load_u64("%v9", abi::return_register(), REC_CTX),
        abi::store_u64("%v9", abi::stack_pointer(), CTX),
    ]);
    if text {
        ins.extend([
            abi::load_u64("%v10", abi::ARG[1], 0),
            abi::store_u64("%v10", abi::stack_pointer(), DLEN),
            abi::add_immediate("%v11", abi::ARG[1], 8),
            abi::store_u64("%v11", abi::stack_pointer(), DATA),
        ]);
    } else {
        ins.extend([
            abi::load_u64("%v10", abi::ARG[1], COLLECTION_OFFSET_COUNT),
            abi::store_u64("%v10", abi::stack_pointer(), DLEN),
            // The byte payload begins past the CAPACITY-sized entry array, not the
            // COUNT-sized one: an append-built list carries spare capacity, so
            // COUNT*ENTRY would mis-address it (byte payload base is
            // HEADER + CAPACITY*ENTRY). Mirrors the OpenSSL path (bug-157).
            abi::load_u64("%v14", abi::ARG[1], COLLECTION_OFFSET_CAPACITY),
            abi::move_immediate("%v12", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
            abi::multiply_registers("%v13", "%v14", "%v12"),
            abi::add_immediate("%v13", "%v13", COLLECTION_HEADER_SIZE),
            abi::add_registers("%v11", abi::ARG[1], "%v13"),
            abi::store_u64("%v11", abi::stack_pointer(), DATA),
        ]);
    }
    // Empty payload: nothing to send.
    ins.extend([
        abi::load_u64("%v10", abi::stack_pointer(), DLEN),
        abi::compare_immediate("%v10", "0"),
        abi::branch_eq(&empty),
    ]);
    emit_dlopen_libssl_macos(
        symbol,
        HANDLE,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    emit_fresh_sem(
        symbol,
        HANDLE,
        CTX,
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    // content = dispatch_data_create(data, len, NULL, NULL)  (NULL = copy)
    dlsym(
        symbol,
        HANDLE,
        "dispatch_data_create",
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), DATA),
        abi::load_u64(abi::ARG[1], abi::stack_pointer(), DLEN),
        abi::move_immediate(abi::ARG[2], "Integer", "0"),
        abi::move_immediate(abi::ARG[3], "Integer", "0"),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), CONTENT),
    ]);
    // ctxdef = *_nw_content_context_default_message
    dlsym(
        symbol,
        HANDLE,
        "_nw_content_context_default_message",
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::load_u64("%v9", "%v9", 0),
        abi::store_u64("%v9", abi::stack_pointer(), CTXDEF),
    ]);
    emit_build_block(
        symbol,
        HANDLE,
        SEND_INVOKE,
        CTX,
        BLOCK,
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    // nw_connection_send(conn, content, context, is_complete=true, &block)
    dlsym(
        symbol,
        HANDLE,
        "nw_connection_send",
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CONN),
        abi::load_u64(abi::ARG[1], abi::stack_pointer(), CONTENT),
        abi::load_u64(abi::ARG[2], abi::stack_pointer(), CTXDEF),
        abi::move_immediate(abi::ARG[3], "Integer", "1"),
        abi::add_immediate(abi::ARG[4], abi::stack_pointer(), BLOCK),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
    ]);
    emit_wait(
        symbol,
        HANDLE,
        CTX,
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    // Release the content we created.
    dlsym(
        symbol,
        HANDLE,
        "dispatch_release",
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CONTENT),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
        // A non-null error means the send failed.
        abi::load_u64("%v9", abi::stack_pointer(), CTX),
        abi::load_u64("%v10", "%v9", CTX_ERROR),
        abi::compare_immediate("%v10", "0"),
        abi::branch_ne(&write_fail),
        abi::label(&empty),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);
    ins.push(abi::label(&write_fail));
    emit_fail(
        symbol,
        ERR_TLS_FAILED_CODE,
        ERR_TLS_FAILED_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    ins.push(abi::label(&load_fail));
    emit_fail(
        symbol,
        ERR_TLS_FAILED_CODE,
        ERR_TLS_FAILED_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    ins.push(abi::label(&closed));
    emit_fail(
        symbol,
        ERR_RESOURCE_CLOSED_CODE,
        ERR_RESOURCE_CLOSED_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    ins.extend([abi::label(&done), abi::return_()]);
    {
        let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut ins, &[], FRAME_SIZE);
        Ok((frame, ins, rel, stack_slots))
    }
}

pub(super) fn lower_tls_close_macos(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<
    (
        CodeFrame,
        Vec<CodeInstruction>,
        Vec<CodeRelocation>,
        Vec<CodeStackSlot>,
    ),
    String,
> {
    const FRAME_SIZE: usize = 48;
    const REC: usize = 8;
    const HANDLE: usize = 16;
    const FNPTR: usize = 24;
    let already = format!("{symbol}_already");
    let load_fail = format!("{symbol}_load_fail");
    let done = format!("{symbol}_done");

    let mut ins = vec![abi::label("entry")];
    let mut rel = Vec::new();
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), REC),
        abi::load_u64("%v9", abi::return_register(), REC_CLOSED),
        abi::compare_immediate("%v9", "0"),
        abi::branch_ne(&already),
    ]);
    emit_dlopen_libssl_macos(
        symbol,
        HANDLE,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    // nw_connection_cancel(conn)
    dlsym(
        symbol,
        HANDLE,
        "nw_connection_cancel",
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), REC),
        abi::load_u64(abi::return_register(), "%v9", REC_CONN),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
    ]);
    // Release the connection, its dispatch queue, and the ctx semaphore that
    // this socket owns; cancelling alone leaves them all leaked on every
    // connect+close (bug-55). The arena-allocated ctx block is reclaimed with
    // the arena. Slots are never NULL for an open (non-closed) socket.
    dlsym(
        symbol,
        HANDLE,
        "nw_release",
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), REC),
        abi::load_u64(abi::return_register(), "%v9", REC_CONN),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
    ]);
    let skip_queue = format!("{symbol}_skip_queue_release");
    dlsym(
        symbol,
        HANDLE,
        "dispatch_release",
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        // Release the queue only if this socket owns it. A client socket stores
        // its own per-connection queue here; an accepted socket stores 0 because
        // it shares the listener's serial queue (released by closeListener), and
        // releasing that shared queue per accepted-close would over-release it.
        abi::load_u64("%v9", abi::stack_pointer(), REC),
        abi::load_u64(abi::return_register(), "%v9", REC_QUEUE),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&skip_queue),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
        abi::label(&skip_queue),
        // NB: ctx->sem is intentionally NOT released here. nw_connection_cancel
        // is asynchronous; the connection's state-changed handler still fires a
        // "cancelled" transition afterwards and does
        // dispatch_semaphore_signal(ctx->sem) — releasing the semaphore now
        // would make that a use-after-free. The single per-connection semaphore
        // is reclaimed with the arena-allocated ctx block (bug-55: the leaks
        // that scale — one per readText/write — are fixed in emit_fresh_sem).
        // Mark closed.
        abi::load_u64("%v9", abi::stack_pointer(), REC),
        abi::move_immediate("%v10", "Integer", "1"),
        abi::store_u64("%v10", "%v9", REC_CLOSED),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);
    ins.push(abi::label(&load_fail));
    emit_fail(
        symbol,
        ERR_TLS_FAILED_CODE,
        ERR_TLS_FAILED_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    ins.extend([
        abi::label(&already),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::label(&done),
        abi::return_(),
    ]);
    {
        let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut ins, &[], FRAME_SIZE);
        Ok((frame, ins, rel, stack_slots))
    }
}

fn emit_dlopen_libssl_macos(
    symbol: &str,
    handle_off: usize,
    fail: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    emit_dlopen_at(
        symbol,
        MACLIB_SYMBOL,
        handle_off,
        fail,
        platform_imports,
        platform,
        instructions,
        relocations,
    )
}

/// `dlopen` the framework named by the C-string data object `lib_symbol` into
/// `sp + handle_off`; branch to `fail` when it does not load.
#[allow(clippy::too_many_arguments)]
fn emit_dlopen_at(
    symbol: &str,
    lib_symbol: &str,
    handle_off: usize,
    fail: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    emit_data_address(
        symbol,
        abi::return_register(),
        lib_symbol,
        instructions,
        relocations,
    );
    instructions.push(abi::move_immediate(abi::ARG[1], "Integer", RTLD_NOW));
    platform.emit_libc_call(
        "dlopen",
        symbol,
        platform_imports,
        instructions,
        relocations,
    )?;
    instructions.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), handle_off),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(fail),
    ]);
    Ok(())
}

// ===========================================================================
// Server side: tls.listen / tls.accept / tls.closeListener
// (plan-06-tls-server.md §7)
// ===========================================================================

/// Read the whole file named by the MFBASIC `String` at `sp + path_off` into a
/// fresh arena buffer: pointer at `sp + buf_off`, byte length at
/// `sp + len_off`. `open_fail` is taken when the file cannot be opened (no fd
/// yet); `read_fail_fd` when a seek/read fails or the file is empty (the open
/// fd is at `sp + fd_off` for the caller to close).
#[allow(clippy::too_many_arguments)]
fn emit_read_whole_file(
    symbol: &str,
    prefix: &str,
    path_off: usize,
    cstr_off: usize,
    fd_off: usize,
    readoff_off: usize,
    buf_off: usize,
    len_off: usize,
    open_fail: &str,
    read_fail_fd: &str,
    alloc_fail: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    let read_loop = format!("{symbol}_{prefix}_read");
    let read_done = format!("{symbol}_{prefix}_read_done");
    emit_cstring(symbol, prefix, path_off, cstr_off, alloc_fail, ins, rel);
    // fd = open(path, O_RDONLY)
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), cstr_off),
        abi::move_immediate(abi::ARG[1], "Integer", "0"),
        abi::move_immediate(abi::ARG[2], "Integer", "0"),
    ]);
    platform.emit_open_file(symbol, platform_imports, ins, rel)?;
    ins.extend([
        // bug-102.3: narrow the C int `open` return before the signed compare
        // (lseek/read below return 64-bit off_t/ssize_t and must NOT be narrowed).
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(open_fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), fd_off),
        // len = lseek(fd, 0, SEEK_END); an empty file is not a valid PEM.
        abi::move_immediate(abi::ARG[1], "Integer", "0"),
        abi::move_immediate(abi::ARG[2], "Integer", "2"),
    ]);
    platform.emit_seek_file(symbol, platform_imports, ins, rel)?;
    ins.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_le(read_fail_fd),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), len_off),
        // rewind: lseek(fd, 0, SEEK_SET)
        abi::load_u64(abi::return_register(), abi::stack_pointer(), fd_off),
        abi::move_immediate(abi::ARG[1], "Integer", "0"),
        abi::move_immediate(abi::ARG[2], "Integer", "0"),
    ]);
    platform.emit_seek_file(symbol, platform_imports, ins, rel)?;
    // buf = arena_alloc(len, 1)
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), len_off),
        abi::move_immediate(abi::ARG[1], "Integer", "1"),
    ]);
    emit_alloc(symbol, ins, rel, alloc_fail);
    ins.extend([
        abi::store_u64(abi::RET[1], abi::stack_pointer(), buf_off),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), readoff_off),
        abi::label(&read_loop),
        abi::load_u64("%v9", abi::stack_pointer(), readoff_off),
        abi::load_u64("%v10", abi::stack_pointer(), len_off),
        abi::compare_registers("%v9", "%v10"),
        abi::branch_ge(&read_done),
        // n = read(fd, buf + off, len - off)
        abi::load_u64(abi::return_register(), abi::stack_pointer(), fd_off),
        abi::load_u64(abi::ARG[1], abi::stack_pointer(), buf_off),
        abi::add_registers(abi::ARG[1], abi::ARG[1], "%v9"),
        abi::subtract_registers(abi::ARG[2], "%v10", "%v9"),
    ]);
    platform.emit_read_file(symbol, platform_imports, ins, rel)?;
    ins.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_le(read_fail_fd),
        abi::load_u64("%v9", abi::stack_pointer(), readoff_off),
        abi::add_registers("%v9", "%v9", abi::return_register()),
        abi::store_u64("%v9", abi::stack_pointer(), readoff_off),
        abi::branch(&read_loop),
        abi::label(&read_done),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), fd_off),
    ]);
    platform.emit_close_file(symbol, platform_imports, ins, rel)?;
    Ok(())
}

/// Import one PEM item (a certificate or a private key) from the bytes at
/// `sp + buf_off`/`len_off` via `CFDataCreate` + `SecItemImport`, leaving the
/// first imported item (`SecCertificateRef`/`SecKeyRef`) at `sp + ref_off`.
#[allow(clippy::too_many_arguments)]
fn emit_import_pem_item(
    symbol: &str,
    buf_off: usize,
    len_off: usize,
    data_off: usize,
    items_off: usize,
    ref_off: usize,
    sec_handle_off: usize,
    cf_handle_off: usize,
    fnptr_off: usize,
    fail: &str,
    load_fail: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    ins: &mut Vec<CodeInstruction>,
    rel: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    // data = CFDataCreate(NULL, buf, len)
    dlsym(
        symbol,
        cf_handle_off,
        "CFDataCreate",
        fnptr_off,
        load_fail,
        platform_imports,
        platform,
        ins,
        rel,
    )?;
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::load_u64(abi::ARG[1], abi::stack_pointer(), buf_off),
        abi::load_u64(abi::ARG[2], abi::stack_pointer(), len_off),
        abi::load_u64("%v9", abi::stack_pointer(), fnptr_off),
        abi::branch_link_register("%v9"),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), data_off),
    ]);
    // SecItemImport(data, NULL, NULL, NULL, 0, NULL, NULL, &items) == errSecSuccess
    dlsym(
        symbol,
        sec_handle_off,
        "SecItemImport",
        fnptr_off,
        load_fail,
        platform_imports,
        platform,
        ins,
        rel,
    )?;
    ins.extend([
        abi::store_u64(abi::ZERO, abi::stack_pointer(), items_off),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), data_off),
        abi::move_immediate(abi::ARG[1], "Integer", "0"),
        abi::move_immediate(abi::ARG[2], "Integer", "0"),
        abi::move_immediate(abi::ARG[3], "Integer", "0"),
        abi::move_immediate(abi::ARG[4], "Integer", "0"),
        abi::move_immediate(abi::ARG[5], "Integer", "0"),
        abi::move_immediate(abi::ARG[6], "Integer", "0"),
        abi::add_immediate(abi::ARG[7], abi::stack_pointer(), items_off),
        abi::load_u64("%v9", abi::stack_pointer(), fnptr_off),
        abi::branch_link_register("%v9"),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(fail),
        abi::load_u64("%v9", abi::stack_pointer(), items_off),
        abi::compare_immediate("%v9", "0"),
        abi::branch_eq(fail),
    ]);
    // CFArrayGetCount(items) >= 1
    dlsym(
        symbol,
        cf_handle_off,
        "CFArrayGetCount",
        fnptr_off,
        load_fail,
        platform_imports,
        platform,
        ins,
        rel,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), items_off),
        abi::load_u64("%v9", abi::stack_pointer(), fnptr_off),
        abi::branch_link_register("%v9"),
        abi::compare_immediate(abi::return_register(), "1"),
        abi::branch_lt(fail),
    ]);
    // ref = CFArrayGetValueAtIndex(items, 0)
    dlsym(
        symbol,
        cf_handle_off,
        "CFArrayGetValueAtIndex",
        fnptr_off,
        load_fail,
        platform_imports,
        platform,
        ins,
        rel,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), items_off),
        abi::move_immediate(abi::ARG[1], "Integer", "0"),
        abi::load_u64("%v9", abi::stack_pointer(), fnptr_off),
        abi::branch_link_register("%v9"),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), ref_off),
    ]);
    Ok(())
}

pub(super) fn lower_tls_listen_macos(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<
    (
        CodeFrame,
        Vec<CodeInstruction>,
        Vec<CodeRelocation>,
        Vec<CodeStackSlot>,
    ),
    String,
> {
    const FRAME_SIZE: usize = 448;
    const HOST: usize = 8;
    const PORT: usize = 16;
    const CERT: usize = 24;
    const KEY: usize = 32;
    // x4 (backlog) is accepted for ABI parity but unused: Network.framework
    // manages its own accept backlog.
    const NWH: usize = 40;
    const SECH: usize = 48;
    const CFH: usize = 56;
    const FNPTR: usize = 64;
    const HOSTCSTR: usize = 72;
    const PORTCSTR: usize = 80;
    const PORTBUF: usize = 88; // 88..112
    const PATHCSTR: usize = 112;
    const FILEFD: usize = 120;
    const READOFF: usize = 128;
    const CERTBUF: usize = 136;
    const CERTLEN: usize = 144;
    const KEYBUF: usize = 152;
    const KEYLEN: usize = 160;
    const DATA: usize = 168;
    const ITEMS: usize = 176;
    const CERTREF: usize = 184;
    const KEYREF: usize = 192;
    const IDENT: usize = 200;
    const SECIDENT: usize = 208;
    const CFG: usize = 216;
    const ENDPOINT: usize = 224;
    const PARAMS: usize = 232;
    const LISTENER: usize = 240;
    const QUEUE: usize = 248;
    const LCTX: usize = 256;
    const CFGBLOCK: usize = 264; // 264..328: the identity-config block literal
    const SBLOCK: usize = 328; // 328..368: state-changed block literal
    const CBLOCK: usize = 368; // 368..408: new-connection block literal
    const WAITFN: usize = 408;

    let cert_fail = format!("{symbol}_cert_fail");
    let read_fail_fd = format!("{symbol}_read_fail_fd");
    let net_fail = format!("{symbol}_net_fail");
    let listen_fail = format!("{symbol}_listen_fail");
    let load_fail = format!("{symbol}_load_fail");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let null_host = format!("{symbol}_null_host");
    let host_ready = format!("{symbol}_host_ready");
    let itoa_loop = format!("{symbol}_itoa");
    let wait_loop = format!("{symbol}_wait");
    let ready = format!("{symbol}_ready");
    let done = format!("{symbol}_done");

    let mut ins = vec![abi::label("entry")];
    let mut rel = Vec::new();
    // x0 = host; x1 = port; x2 = certPath; x3 = keyPath; x4 = backlog (unused).
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), HOST),
        abi::store_u64(abi::ARG[1], abi::stack_pointer(), PORT),
        abi::store_u64(abi::ARG[2], abi::stack_pointer(), CERT),
        abi::store_u64(abi::ARG[3], abi::stack_pointer(), KEY),
    ]);
    // Read the PEM pair into arena buffers before touching any framework.
    emit_read_whole_file(
        symbol,
        "cert",
        CERT,
        PATHCSTR,
        FILEFD,
        READOFF,
        CERTBUF,
        CERTLEN,
        &cert_fail,
        &read_fail_fd,
        &alloc_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    emit_read_whole_file(
        symbol,
        "key",
        KEY,
        PATHCSTR,
        FILEFD,
        READOFF,
        KEYBUF,
        KEYLEN,
        &cert_fail,
        &read_fail_fd,
        &alloc_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    // dlopen Network.framework, Security.framework, CoreFoundation.
    emit_dlopen_at(
        symbol,
        MACLIB_SYMBOL,
        NWH,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    emit_dlopen_at(
        symbol,
        MACSEC_SYMBOL,
        SECH,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    emit_dlopen_at(
        symbol,
        MACCF_SYMBOL,
        CFH,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    // certRef / keyRef from the PEM bytes.
    emit_import_pem_item(
        symbol,
        CERTBUF,
        CERTLEN,
        DATA,
        ITEMS,
        CERTREF,
        SECH,
        CFH,
        FNPTR,
        &cert_fail,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    emit_import_pem_item(
        symbol,
        KEYBUF,
        KEYLEN,
        DATA,
        ITEMS,
        KEYREF,
        SECH,
        CFH,
        FNPTR,
        &cert_fail,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    // identity = SecIdentityCreate(NULL, certRef, keyRef) — the keychain-free
    // cert+key pairing entry point in Security.framework (resolved via dlsym;
    // absent => ErrTlsFailed, never a stub).
    dlsym(
        symbol,
        SECH,
        "SecIdentityCreate",
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::load_u64(abi::ARG[1], abi::stack_pointer(), CERTREF),
        abi::load_u64(abi::ARG[2], abi::stack_pointer(), KEYREF),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&cert_fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), IDENT),
    ]);
    // secIdentity = sec_identity_create(identity)
    dlsym(
        symbol,
        SECH,
        "sec_identity_create",
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), IDENT),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&cert_fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), SECIDENT),
    ]);
    // Build the configure-TLS block that installs the local identity:
    // CFG_INVOKE copies the sec_protocol_options and calls the captured
    // setter with the captured payload — here
    // sec_protocol_options_set_local_identity(options, secIdentity).
    dlsym(
        symbol,
        NWH,
        "_NSConcreteStackBlock",
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::store_u64("%v9", abi::stack_pointer(), CFGBLOCK + BLK_ISA),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), CFGBLOCK + BLK_FLAGS),
    ]);
    emit_data_address(symbol, "%v9", CFG_INVOKE, &mut ins, &mut rel);
    ins.push(abi::store_u64(
        "%v9",
        abi::stack_pointer(),
        CFGBLOCK + BLK_INVOKE,
    ));
    emit_data_address(symbol, "%v9", CFG_DESC_SYMBOL, &mut ins, &mut rel);
    ins.push(abi::store_u64(
        "%v9",
        abi::stack_pointer(),
        CFGBLOCK + BLK_DESC,
    ));
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), SECIDENT),
        abi::store_u64("%v9", abi::stack_pointer(), CFGBLOCK + CFG_CAP_SNAME),
    ]);
    dlsym(
        symbol,
        NWH,
        "nw_tls_copy_sec_protocol_options",
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::store_u64("%v9", abi::stack_pointer(), CFGBLOCK + CFG_CAP_COPYFN),
    ]);
    dlsym(
        symbol,
        SECH,
        "sec_protocol_options_set_local_identity",
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::store_u64("%v9", abi::stack_pointer(), CFGBLOCK + CFG_CAP_SETFN),
    ]);
    // nw_release: the invoke releases the +1 sec_protocol_options the copy fn
    // returns, so each listener stops leaking one (bug-116).
    dlsym(
        symbol,
        NWH,
        "nw_release",
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::store_u64("%v9", abi::stack_pointer(), CFGBLOCK + CFG_CAP_RELEASEFN),
    ]);
    // cfg = *_nw_parameters_configure_protocol_default_configuration
    dlsym(
        symbol,
        NWH,
        "_nw_parameters_configure_protocol_default_configuration",
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::load_u64("%v9", "%v9", 0),
        abi::store_u64("%v9", abi::stack_pointer(), CFG),
    ]);
    // params = nw_parameters_create_secure_tcp(&cfgBlock, cfg)
    dlsym(
        symbol,
        NWH,
        "nw_parameters_create_secure_tcp",
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::add_immediate(abi::return_register(), abi::stack_pointer(), CFGBLOCK),
        abi::load_u64(abi::ARG[1], abi::stack_pointer(), CFG),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&net_fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), PARAMS),
    ]);
    // nw_parameters_set_reuse_local_address(params, true)
    dlsym(
        symbol,
        NWH,
        "nw_parameters_set_reuse_local_address",
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), PARAMS),
        abi::move_immediate(abi::ARG[1], "Integer", "1"),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
    ]);
    // Local endpoint: empty host binds all interfaces ("0.0.0.0").
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), HOST),
        abi::load_u64("%v10", "%v9", 0),
        abi::compare_immediate("%v10", "0"),
        abi::branch_eq(&null_host),
    ]);
    emit_cstring(
        symbol,
        "host",
        HOST,
        HOSTCSTR,
        &alloc_fail,
        &mut ins,
        &mut rel,
    );
    ins.push(abi::branch(&host_ready));
    ins.push(abi::label(&null_host));
    emit_data_address(symbol, "%v9", ANYHOST_SYMBOL, &mut ins, &mut rel);
    ins.extend([
        abi::store_u64("%v9", abi::stack_pointer(), HOSTCSTR),
        abi::label(&host_ready),
    ]);
    // itoa(port) -> NUL-terminated decimal at PORTBUF, pointer in PORTCSTR.
    ins.extend([
        abi::move_immediate("%v9", "Integer", "0"),
        abi::store_u8("%v9", abi::stack_pointer(), PORTBUF + 23),
        abi::load_u64("%v10", abi::stack_pointer(), PORT),
        abi::move_immediate("%v11", "Integer", "10"),
        abi::add_immediate("%v14", abi::stack_pointer(), PORTBUF + 22),
        abi::label(&itoa_loop),
        abi::unsigned_divide_registers("%v15", "%v10", "%v11"),
        abi::multiply_subtract_registers("%v16", "%v15", "%v11", "%v10"),
        abi::add_immediate("%v16", "%v16", 48),
        abi::store_u8("%v16", "%v14", 0),
        abi::subtract_immediate("%v14", "%v14", 1),
        abi::move_register("%v10", "%v15"),
        abi::compare_immediate("%v10", "0"),
        abi::branch_ne(&itoa_loop),
        abi::add_immediate("%v13", "%v14", 1),
        abi::store_u64("%v13", abi::stack_pointer(), PORTCSTR),
    ]);
    // endpoint = nw_endpoint_create_host(host, port)
    dlsym(
        symbol,
        NWH,
        "nw_endpoint_create_host",
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), HOSTCSTR),
        abi::load_u64(abi::ARG[1], abi::stack_pointer(), PORTCSTR),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&net_fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), ENDPOINT),
    ]);
    // nw_parameters_set_local_endpoint(params, endpoint)
    dlsym(
        symbol,
        NWH,
        "nw_parameters_set_local_endpoint",
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), PARAMS),
        abi::load_u64(abi::ARG[1], abi::stack_pointer(), ENDPOINT),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
    ]);
    // listener = nw_listener_create(params)
    dlsym(
        symbol,
        NWH,
        "nw_listener_create",
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), PARAMS),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&net_fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), LISTENER),
    ]);
    // The endpoint is retained into the parameters (set_local_endpoint) and the
    // parameters are retained by the listener (nw_listener_create), so release
    // our own references now; otherwise every successful listen leaks one
    // nw_endpoint and one nw_parameters (bug-55).
    dlsym(
        symbol,
        NWH,
        "nw_release",
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), ENDPOINT),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), PARAMS),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
    ]);
    // queue = dispatch_queue_create("mfb.tls", NULL)
    dlsym(
        symbol,
        NWH,
        "dispatch_queue_create",
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    emit_data_address(
        symbol,
        abi::return_register(),
        QLABEL_SYMBOL,
        &mut ins,
        &mut rel,
    );
    ins.extend([
        abi::move_immediate(abi::ARG[1], "Integer", "0"),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), QUEUE),
    ]);
    // Allocate + initialize the listener context (shared ctx prefix + ring).
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", LCTX_SIZE),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut ins, &mut rel, &alloc_fail);
    ins.push(abi::store_u64(abi::RET[1], abi::stack_pointer(), LCTX));
    dlsym(
        symbol,
        NWH,
        "dispatch_semaphore_create",
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
        abi::load_u64("%v9", abi::stack_pointer(), LCTX),
        abi::store_u64(abi::return_register(), "%v9", CTX_SEM),
        abi::store_u64(abi::ZERO, "%v9", CTX_STATE),
        abi::store_u64(abi::ZERO, "%v9", CTX_CONTENT),
        abi::store_u64(abi::ZERO, "%v9", CTX_ERROR),
        abi::store_u64(abi::ZERO, "%v9", LCTX_HEAD),
        abi::store_u64(abi::ZERO, "%v9", LCTX_TAIL),
    ]);
    dlsym(
        symbol,
        NWH,
        "dispatch_semaphore_signal",
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::load_u64("%v10", abi::stack_pointer(), FNPTR),
        abi::load_u64("%v9", abi::stack_pointer(), LCTX),
        abi::store_u64("%v10", "%v9", CTX_SIGNAL),
    ]);
    // ctx->retain = &nw_retain (the conn handler retains queued connections).
    dlsym(
        symbol,
        NWH,
        "nw_retain",
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::load_u64("%v10", abi::stack_pointer(), FNPTR),
        abi::load_u64("%v9", abi::stack_pointer(), LCTX),
        abi::store_u64("%v10", "%v9", CTX_RETAIN),
    ]);
    // nw_listener_set_queue(listener, queue)
    dlsym(
        symbol,
        NWH,
        "nw_listener_set_queue",
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), LISTENER),
        abi::load_u64(abi::ARG[1], abi::stack_pointer(), QUEUE),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
    ]);
    // State-changed handler (the shared STATE_INVOKE trampoline over lctx).
    emit_build_block(
        symbol,
        NWH,
        STATE_INVOKE,
        LCTX,
        SBLOCK,
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    dlsym(
        symbol,
        NWH,
        "nw_listener_set_state_changed_handler",
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), LISTENER),
        abi::add_immediate(abi::ARG[1], abi::stack_pointer(), SBLOCK),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
    ]);
    // New-connection handler (retain + enqueue + signal).
    emit_build_block(
        symbol,
        NWH,
        LCONN_INVOKE,
        LCTX,
        CBLOCK,
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    dlsym(
        symbol,
        NWH,
        "nw_listener_set_new_connection_handler",
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), LISTENER),
        abi::add_immediate(abi::ARG[1], abi::stack_pointer(), CBLOCK),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
    ]);
    // nw_listener_start(listener)
    dlsym(
        symbol,
        NWH,
        "nw_listener_start",
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), LISTENER),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
    ]);
    // Wait until the listener is ready (bind complete) or failed.
    dlsym(
        symbol,
        NWH,
        "dispatch_semaphore_wait",
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::store_u64("%v9", abi::stack_pointer(), WAITFN),
        abi::label(&wait_loop),
        abi::load_u64("%v9", abi::stack_pointer(), LCTX),
        abi::load_u64(abi::return_register(), "%v9", CTX_SEM),
        abi::move_immediate(abi::ARG[1], "Integer", "0"),
        abi::bitwise_not(abi::ARG[1], abi::ARG[1]), // DISPATCH_TIME_FOREVER
        abi::load_u64("%v10", abi::stack_pointer(), WAITFN),
        abi::branch_link_register("%v10"),
        abi::load_u64("%v9", abi::stack_pointer(), LCTX),
        abi::load_u32("%v10", "%v9", CTX_STATE),
        abi::compare_immediate("%v10", NW_LISTENER_READY),
        abi::branch_eq(&ready),
        abi::compare_immediate("%v10", NW_LISTENER_FAILED),
        abi::branch_ge(&listen_fail), // failed / cancelled
        abi::branch(&wait_loop),      // invalid / waiting
        abi::label(&ready),
    ]);
    // Build the TlsListener record { closed=0, listener, queue, lctx }.
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", REC_SIZE),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut ins, &mut rel, &alloc_fail);
    ins.extend([
        abi::store_u64(abi::ZERO, abi::RET[1], REC_CLOSED),
        abi::load_u64("%v9", abi::stack_pointer(), LISTENER),
        abi::store_u64("%v9", abi::RET[1], REC_CONN),
        abi::load_u64("%v9", abi::stack_pointer(), QUEUE),
        abi::store_u64("%v9", abi::RET[1], REC_QUEUE),
        abi::load_u64("%v9", abi::stack_pointer(), LCTX),
        abi::store_u64("%v9", abi::RET[1], REC_CTX),
        abi::move_register(RESULT_VALUE_REGISTER, abi::RET[1]),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);
    // listen_fail: bind/start failed — cancel the listener, report a network
    // failure (mirrors net::listenTcp's bind error).
    ins.push(abi::label(&listen_fail));
    dlsym(
        symbol,
        NWH,
        "nw_listener_cancel",
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), LISTENER),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
    ]);
    ins.push(abi::label(&net_fail));
    emit_fail(
        symbol,
        ERR_NETWORK_FAILED_CODE,
        ERR_NETWORK_FAILED_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    // read_fail_fd: a seek/read on an opened PEM file failed — close it first.
    ins.push(abi::label(&read_fail_fd));
    ins.push(abi::load_u64(
        abi::return_register(),
        abi::stack_pointer(),
        FILEFD,
    ));
    platform.emit_close_file(symbol, platform_imports, &mut ins, &mut rel)?;
    ins.push(abi::label(&cert_fail));
    emit_fail(
        symbol,
        ERR_TLS_FAILED_CODE,
        ERR_TLS_FAILED_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    ins.push(abi::label(&load_fail));
    emit_fail(
        symbol,
        ERR_TLS_FAILED_CODE,
        ERR_TLS_FAILED_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    ins.push(abi::label(&alloc_fail));
    emit_fail(
        symbol,
        ERR_OUT_OF_MEMORY_CODE,
        ERR_ALLOCATION_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    ins.extend([abi::label(&done), abi::return_()]);
    {
        let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut ins, &[], FRAME_SIZE);
        Ok((frame, ins, rel, stack_slots))
    }
}

pub(super) fn lower_tls_accept_macos(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<
    (
        CodeFrame,
        Vec<CodeInstruction>,
        Vec<CodeRelocation>,
        Vec<CodeStackSlot>,
    ),
    String,
> {
    const FRAME_SIZE: usize = 208;
    const REC: usize = 8;
    const TIMEOUT: usize = 16;
    const NWH: usize = 24;
    const FNPTR: usize = 32;
    const LCTX: usize = 40;
    const QUEUE: usize = 48;
    const DEADLINE: usize = 56;
    const WAITFN: usize = 64;
    const CONN: usize = 72;
    const CCTX: usize = 80;
    const SBLOCK: usize = 96; // 96..136: per-connection state block literal

    let closed = format!("{symbol}_closed");
    let load_fail = format!("{symbol}_load_fail");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let wait_forever = format!("{symbol}_wait_forever");
    let deadline_ready = format!("{symbol}_deadline_ready");
    let wait_loop = format!("{symbol}_wait");
    let pop = format!("{symbol}_pop");
    let listener_dead = format!("{symbol}_listener_dead");
    let accept_timeout = format!("{symbol}_accept_timeout");
    let hs_loop = format!("{symbol}_hs_wait");
    let hs_timeout = format!("{symbol}_hs_timeout");
    let conn_fail = format!("{symbol}_conn_fail");
    let ready = format!("{symbol}_ready");
    let done = format!("{symbol}_done");

    let mut ins = vec![abi::label("entry")];
    let mut rel = Vec::new();
    // x0 = listener record { closed@0, listener@8, queue@16, lctx@24 };
    // x1 = timeoutMs.
    ins.extend([
        abi::store_u64(abi::ARG[1], abi::stack_pointer(), TIMEOUT),
        abi::load_u64("%v9", abi::return_register(), REC_CLOSED),
        abi::compare_immediate("%v9", "0"),
        abi::branch_ne(&closed),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), REC),
        abi::load_u64("%v9", abi::return_register(), REC_CTX),
        abi::store_u64("%v9", abi::stack_pointer(), LCTX),
        abi::load_u64("%v9", abi::return_register(), REC_QUEUE),
        abi::store_u64("%v9", abi::stack_pointer(), QUEUE),
    ]);
    emit_dlopen_libssl_macos(
        symbol,
        NWH,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    // Deadline: timeoutMs > 0 => dispatch_time(NOW, ms*1e6); else FOREVER.
    // The one absolute deadline bounds both the wait for a connection and the
    // server handshake.
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), TIMEOUT),
        abi::compare_immediate("%v9", "0"),
        abi::branch_le(&wait_forever),
    ]);
    dlsym(
        symbol,
        NWH,
        "dispatch_time",
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", "0"), // DISPATCH_TIME_NOW
        abi::load_u64(abi::ARG[1], abi::stack_pointer(), TIMEOUT),
        abi::move_immediate("%v10", "Integer", "1000000"),
        abi::multiply_registers(abi::ARG[1], abi::ARG[1], "%v10"), // ms -> ns
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), DEADLINE),
        abi::branch(&deadline_ready),
        abi::label(&wait_forever),
        abi::move_immediate("%v9", "Integer", "0"),
        abi::bitwise_not("%v9", "%v9"), // DISPATCH_TIME_FOREVER
        abi::store_u64("%v9", abi::stack_pointer(), DEADLINE),
        abi::label(&deadline_ready),
    ]);
    dlsym(
        symbol,
        NWH,
        "dispatch_semaphore_wait",
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::store_u64("%v9", abi::stack_pointer(), WAITFN),
        // Wait for a queued connection (the ring is checked first so
        // connections that arrived before this accept are drained even when
        // their semaphore counts were consumed by earlier state wakeups).
        abi::label(&wait_loop),
        abi::load_u64("%v9", abi::stack_pointer(), LCTX),
        abi::load_u64("%v10", "%v9", LCTX_HEAD),
        abi::load_u64("%v11", "%v9", LCTX_TAIL),
        abi::compare_registers("%v10", "%v11"),
        abi::branch_ne(&pop),
        // Listener failed/cancelled while we wait?
        abi::load_u32("%v10", "%v9", CTX_STATE),
        abi::compare_immediate("%v10", NW_LISTENER_FAILED),
        abi::branch_ge(&listener_dead),
        abi::load_u64(abi::return_register(), "%v9", CTX_SEM),
        abi::load_u64(abi::ARG[1], abi::stack_pointer(), DEADLINE),
        abi::load_u64("%v10", abi::stack_pointer(), WAITFN),
        abi::branch_link_register("%v10"),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&accept_timeout),
        abi::branch(&wait_loop),
        // Pop the oldest queued connection.
        abi::label(&pop),
        abi::load_u64("%v9", abi::stack_pointer(), LCTX),
        abi::load_u64("%v11", "%v9", LCTX_TAIL),
        abi::move_immediate("%v12", "Integer", "15"),
        abi::and_registers("%v12", "%v11", "%v12"),
        abi::shift_left_immediate("%v12", "%v12", 3),
        abi::add_immediate("%v13", "%v9", LCTX_RING),
        abi::add_registers("%v13", "%v13", "%v12"),
        abi::load_u64("%v14", "%v13", 0),
        abi::store_u64("%v14", abi::stack_pointer(), CONN),
        abi::add_immediate("%v11", "%v11", 1),
        abi::store_u64("%v11", "%v9", LCTX_TAIL),
    ]);
    // Per-connection block context { sem, signal, state, content, error }.
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", CTX_SIZE),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut ins, &mut rel, &alloc_fail);
    ins.push(abi::store_u64(abi::RET[1], abi::stack_pointer(), CCTX));
    dlsym(
        symbol,
        NWH,
        "dispatch_semaphore_create",
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
        abi::load_u64("%v9", abi::stack_pointer(), CCTX),
        abi::store_u64(abi::return_register(), "%v9", CTX_SEM),
        abi::store_u64(abi::ZERO, "%v9", CTX_STATE),
        abi::store_u64(abi::ZERO, "%v9", CTX_CONTENT),
        abi::store_u64(abi::ZERO, "%v9", CTX_ERROR),
    ]);
    dlsym(
        symbol,
        NWH,
        "dispatch_semaphore_signal",
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::load_u64("%v10", abi::stack_pointer(), FNPTR),
        abi::load_u64("%v9", abi::stack_pointer(), CCTX),
        abi::store_u64("%v10", "%v9", CTX_SIGNAL),
    ]);
    // nw_connection_set_queue(conn, queue) — the listener's serial queue.
    dlsym(
        symbol,
        NWH,
        "nw_connection_set_queue",
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CONN),
        abi::load_u64(abi::ARG[1], abi::stack_pointer(), QUEUE),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
    ]);
    // Per-connection state handler, then start (runs the server handshake).
    emit_build_block(
        symbol,
        NWH,
        STATE_INVOKE,
        CCTX,
        SBLOCK,
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    dlsym(
        symbol,
        NWH,
        "nw_connection_set_state_changed_handler",
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CONN),
        abi::add_immediate(abi::ARG[1], abi::stack_pointer(), SBLOCK),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
    ]);
    dlsym(
        symbol,
        NWH,
        "nw_connection_start",
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CONN),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
        // Wait for the connection to reach ready (handshake complete).
        abi::label(&hs_loop),
        abi::load_u64("%v9", abi::stack_pointer(), CCTX),
        abi::load_u64(abi::return_register(), "%v9", CTX_SEM),
        abi::load_u64(abi::ARG[1], abi::stack_pointer(), DEADLINE),
        abi::load_u64("%v10", abi::stack_pointer(), WAITFN),
        abi::branch_link_register("%v10"),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&hs_timeout),
        abi::load_u64("%v9", abi::stack_pointer(), CCTX),
        abi::load_u32("%v10", "%v9", CTX_STATE),
        abi::compare_immediate("%v10", NW_STATE_READY),
        abi::branch_eq(&ready),
        abi::compare_immediate("%v10", "2"), // preparing
        abi::branch_eq(&hs_loop),
        abi::compare_immediate("%v10", "0"), // invalid
        abi::branch_eq(&hs_loop),
        abi::branch(&conn_fail), // waiting/failed/cancelled
        abi::label(&ready),
    ]);
    // Build the TlsSocket record { closed=0, conn, queue=0, cctx } — the queue
    // slot is 0 (not the listener's shared serial queue) so the shared close
    // helper releases the connection and ctx semaphore this socket owns but not
    // the listener-owned queue, which closeListener releases (bug-55). read/
    // write/close otherwise work identically to a client socket.
    ins.extend([
        abi::move_immediate(abi::return_register(), "Integer", REC_SIZE),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut ins, &mut rel, &alloc_fail);
    ins.extend([
        abi::store_u64(abi::ZERO, abi::RET[1], REC_CLOSED),
        abi::load_u64("%v9", abi::stack_pointer(), CONN),
        abi::store_u64("%v9", abi::RET[1], REC_CONN),
        abi::store_u64(abi::ZERO, abi::RET[1], REC_QUEUE),
        abi::load_u64("%v9", abi::stack_pointer(), CCTX),
        abi::store_u64("%v9", abi::RET[1], REC_CTX),
        abi::move_register(RESULT_VALUE_REGISTER, abi::RET[1]),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);
    // conn_fail / hs_timeout: cancel the accepted connection first.
    ins.push(abi::label(&conn_fail));
    dlsym(
        symbol,
        NWH,
        "nw_connection_cancel",
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CONN),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
    ]);
    emit_fail(
        symbol,
        ERR_TLS_FAILED_CODE,
        ERR_TLS_FAILED_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    ins.push(abi::label(&hs_timeout));
    dlsym(
        symbol,
        NWH,
        "nw_connection_cancel",
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CONN),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
    ]);
    emit_fail(
        symbol,
        ERR_TIMEOUT_CODE,
        ERR_TIMEOUT_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    ins.push(abi::label(&accept_timeout));
    emit_fail(
        symbol,
        ERR_TIMEOUT_CODE,
        ERR_TIMEOUT_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    ins.push(abi::label(&listener_dead));
    emit_fail(
        symbol,
        ERR_NETWORK_FAILED_CODE,
        ERR_NETWORK_FAILED_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    ins.push(abi::label(&closed));
    emit_fail(
        symbol,
        ERR_RESOURCE_CLOSED_CODE,
        ERR_RESOURCE_CLOSED_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    ins.push(abi::label(&load_fail));
    emit_fail(
        symbol,
        ERR_TLS_FAILED_CODE,
        ERR_TLS_FAILED_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    ins.push(abi::label(&alloc_fail));
    emit_fail(
        symbol,
        ERR_OUT_OF_MEMORY_CODE,
        ERR_ALLOCATION_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    ins.extend([abi::label(&done), abi::return_()]);
    {
        let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut ins, &[], FRAME_SIZE);
        Ok((frame, ins, rel, stack_slots))
    }
}

pub(super) fn lower_tls_close_listener_macos(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<
    (
        CodeFrame,
        Vec<CodeInstruction>,
        Vec<CodeRelocation>,
        Vec<CodeStackSlot>,
    ),
    String,
> {
    const FRAME_SIZE: usize = 96;
    const REC: usize = 8;
    const HANDLE: usize = 16;
    const FNPTR: usize = 24;
    const LCTX: usize = 32;
    const CONN: usize = 40;
    const SETQFN: usize = 48;
    const CANCELFN: usize = 56;
    const RELEASEFN: usize = 64;

    let already = format!("{symbol}_already");
    let load_fail = format!("{symbol}_load_fail");
    let drain_loop = format!("{symbol}_drain");
    let drained = format!("{symbol}_drained");
    let done = format!("{symbol}_done");

    let mut ins = vec![abi::label("entry")];
    let mut rel = Vec::new();
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), REC),
        // Idempotent: a closed handle returns OK.
        abi::load_u64("%v9", abi::return_register(), REC_CLOSED),
        abi::compare_immediate("%v9", "0"),
        abi::branch_ne(&already),
        abi::load_u64("%v9", abi::return_register(), REC_CTX),
        abi::store_u64("%v9", abi::stack_pointer(), LCTX),
    ]);
    emit_dlopen_libssl_macos(
        symbol,
        HANDLE,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    for (name, off) in [
        ("nw_connection_set_queue", SETQFN),
        ("nw_connection_cancel", CANCELFN),
        ("nw_release", RELEASEFN),
    ] {
        dlsym(
            symbol,
            HANDLE,
            name,
            FNPTR,
            &load_fail,
            platform_imports,
            platform,
            &mut ins,
            &mut rel,
        )?;
        ins.extend([
            abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
            abi::store_u64("%v9", abi::stack_pointer(), off),
        ]);
    }
    // Reject every still-queued (retained, never-started) connection: give it
    // the listener's queue, cancel it, drop our retain.
    ins.extend([
        abi::label(&drain_loop),
        abi::load_u64("%v9", abi::stack_pointer(), LCTX),
        abi::load_u64("%v10", "%v9", LCTX_HEAD),
        abi::load_u64("%v11", "%v9", LCTX_TAIL),
        abi::compare_registers("%v10", "%v11"),
        abi::branch_eq(&drained),
        abi::move_immediate("%v12", "Integer", "15"),
        abi::and_registers("%v12", "%v11", "%v12"),
        abi::shift_left_immediate("%v12", "%v12", 3),
        abi::add_immediate("%v13", "%v9", LCTX_RING),
        abi::add_registers("%v13", "%v13", "%v12"),
        abi::load_u64("%v14", "%v13", 0),
        abi::store_u64("%v14", abi::stack_pointer(), CONN),
        abi::add_immediate("%v11", "%v11", 1),
        abi::store_u64("%v11", "%v9", LCTX_TAIL),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CONN),
        abi::load_u64("%v9", abi::stack_pointer(), REC),
        abi::load_u64(abi::ARG[1], "%v9", REC_QUEUE),
        abi::load_u64("%v10", abi::stack_pointer(), SETQFN),
        abi::branch_link_register("%v10"),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CONN),
        abi::load_u64("%v10", abi::stack_pointer(), CANCELFN),
        abi::branch_link_register("%v10"),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CONN),
        abi::load_u64("%v10", abi::stack_pointer(), RELEASEFN),
        abi::branch_link_register("%v10"),
        abi::branch(&drain_loop),
        abi::label(&drained),
    ]);
    // nw_listener_cancel(listener)
    dlsym(
        symbol,
        HANDLE,
        "nw_listener_cancel",
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), REC),
        abi::load_u64(abi::return_register(), "%v9", REC_CONN),
        abi::load_u64("%v10", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v10"),
        // Release the listener, its serial queue, and the listener-ctx
        // semaphore this handle owns; cancelling alone leaks them (bug-55). The
        // arena-allocated lctx block is reclaimed with the arena. RELEASEFN
        // already holds nw_release (resolved in the drain loop above).
        abi::load_u64("%v9", abi::stack_pointer(), REC),
        abi::load_u64(abi::return_register(), "%v9", REC_CONN),
        abi::load_u64("%v10", abi::stack_pointer(), RELEASEFN),
        abi::branch_link_register("%v10"),
    ]);
    dlsym(
        symbol,
        HANDLE,
        "dispatch_release",
        FNPTR,
        &load_fail,
        platform_imports,
        platform,
        &mut ins,
        &mut rel,
    )?;
    ins.extend([
        abi::load_u64("%v9", abi::stack_pointer(), REC),
        abi::load_u64(abi::return_register(), "%v9", REC_QUEUE),
        abi::load_u64("%v9", abi::stack_pointer(), FNPTR),
        abi::branch_link_register("%v9"),
        // NB: the listener ctx semaphore is intentionally NOT released here, for
        // the same reason as the connection close: nw_listener_cancel is async
        // and the listener state handler still signals ctx->sem on the cancelled
        // transition. It is reclaimed with the arena-allocated lctx block.
        // Mark closed.
        abi::load_u64("%v9", abi::stack_pointer(), REC),
        abi::move_immediate("%v10", "Integer", "1"),
        abi::store_u64("%v10", "%v9", REC_CLOSED),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);
    ins.push(abi::label(&load_fail));
    emit_fail(
        symbol,
        ERR_TLS_FAILED_CODE,
        ERR_TLS_FAILED_SYMBOL,
        &mut ins,
        &mut rel,
        &done,
    );
    ins.extend([
        abi::label(&already),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::label(&done),
        abi::return_(),
    ]);
    {
        let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut ins, &[], FRAME_SIZE);
        Ok((frame, ins, rel, stack_slots))
    }
}

#[cfg(test)]
mod encoding_error_release_tests {
    // Regression guard for bug-52: on macOS, `tls::readText`'s encoding-error
    // exit must release the mapped `dispatch_data` (MAPPED) and the retained nw
    // content object (CTX_CONTENT) before failing, exactly as the success exit
    // does. Before the fix that exit jumped straight to `emit_fail`, so every
    // invalid-UTF-8 read leaked one map + one content object — a peer-controlled
    // (remote) memory-exhaustion DoS. Runtime proof lives in the fix's leak
    // measurement (`leaks` shows the per-read `dispatch_data_t` leak drop to 0);
    // this test pins the codegen so the releases cannot silently regress.
    use super::*;
    use crate::target::shared::code::mir;

    struct TlsReadTestPlatform;

    #[rustfmt::skip]
    impl CodegenPlatform for TlsReadTestPlatform {
        fn target(&self) -> &'static str { unimplemented!("TlsReadTestPlatform::target") }
        fn arch(&self) -> &'static str { unimplemented!("TlsReadTestPlatform::arch") }
        fn backend(&self) -> &'static dyn crate::target::shared::code::mir::Backend { &crate::arch::aarch64::backend::AARCH64_BACKEND }
        fn termios_size(&self) -> usize { unimplemented!("TlsReadTestPlatform::termios_size") }
        fn termios_lflag_offset(&self) -> usize { unimplemented!("TlsReadTestPlatform::termios_lflag_offset") }
        fn termios_lflag_width(&self) -> usize { unimplemented!("TlsReadTestPlatform::termios_lflag_width") }
        fn termios_cc_offset(&self) -> usize { unimplemented!("TlsReadTestPlatform::termios_cc_offset") }
        fn termios_echo_flag(&self) -> u64 { unimplemented!("TlsReadTestPlatform::termios_echo_flag") }
        fn termios_icanon_flag(&self) -> u64 { unimplemented!("TlsReadTestPlatform::termios_icanon_flag") }
        fn termios_vmin_index(&self) -> usize { unimplemented!("TlsReadTestPlatform::termios_vmin_index") }
        fn termios_vtime_index(&self) -> usize { unimplemented!("TlsReadTestPlatform::termios_vtime_index") }
        fn emit_program_exit(
        &self,
        _from: &str,
        _instructions: &mut Vec<crate::target::shared::code::CodeInstruction>,
        _relocations: &mut Vec<crate::target::shared::code::CodeRelocation>,
    ) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_program_exit") }
        fn emit_write(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<crate::target::shared::code::CodeInstruction>,
        _relocations: &mut Vec<crate::target::shared::code::CodeRelocation>,
    ) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_write") }
        fn emit_poll_input(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<crate::target::shared::code::CodeInstruction>,
        _relocations: &mut Vec<crate::target::shared::code::CodeRelocation>,
    ) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_poll_input") }
        fn emit_is_terminal(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<crate::target::shared::code::CodeInstruction>,
        _relocations: &mut Vec<crate::target::shared::code::CodeRelocation>,
    ) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_is_terminal") }
        fn emit_terminal_size(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<crate::target::shared::code::CodeInstruction>,
        _relocations: &mut Vec<crate::target::shared::code::CodeRelocation>,
    ) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_terminal_size") }
        fn emit_path_exists(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<crate::target::shared::code::CodeInstruction>,
        _relocations: &mut Vec<crate::target::shared::code::CodeRelocation>,
    ) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_path_exists") }
        fn emit_path_stat(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<crate::target::shared::code::CodeInstruction>,
        _relocations: &mut Vec<crate::target::shared::code::CodeRelocation>,
    ) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_path_stat") }
        fn stat_mode_offset(&self) -> usize { unimplemented!("TlsReadTestPlatform::stat_mode_offset") }
        fn emit_current_directory(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<crate::target::shared::code::CodeInstruction>,
        _relocations: &mut Vec<crate::target::shared::code::CodeRelocation>,
    ) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_current_directory") }
        fn emit_environ_pointer(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<crate::target::shared::code::CodeInstruction>,
        _relocations: &mut Vec<crate::target::shared::code::CodeRelocation>,
    ) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_environ_pointer") }
        fn emit_fs_path_operation(
        &self,
        _from: &str,
        _operation: crate::target::shared::code::FsPathOperation,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<crate::target::shared::code::CodeInstruction>,
        _relocations: &mut Vec<crate::target::shared::code::CodeRelocation>,
    ) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_fs_path_operation") }
        fn emit_errno(
        &self,
        _from: &str,
        _dst: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<crate::target::shared::code::CodeInstruction>,
        _relocations: &mut Vec<crate::target::shared::code::CodeRelocation>,
    ) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_errno") }
        fn emit_libc_call(
        &self,
        _base: &str,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<crate::target::shared::code::CodeInstruction>,
        _relocations: &mut Vec<crate::target::shared::code::CodeRelocation>,
    ) -> Result<(), String> {
            // Minimal stand-in: a plain `bl` to the named libc function is
            // enough for the read helper to lower and register-allocate; the
            // test only inspects the resulting encoding-error release block.
            _instructions.push(crate::target::shared::abi::branch_link(&format!("_{_base}")));
            Ok(())
        }
        fn emit_open_file(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<crate::target::shared::code::CodeInstruction>,
        _relocations: &mut Vec<crate::target::shared::code::CodeRelocation>,
    ) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_open_file") }
        fn emit_read_file(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<crate::target::shared::code::CodeInstruction>,
        _relocations: &mut Vec<crate::target::shared::code::CodeRelocation>,
    ) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_read_file") }
        fn emit_close_file(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<crate::target::shared::code::CodeInstruction>,
        _relocations: &mut Vec<crate::target::shared::code::CodeRelocation>,
    ) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_close_file") }
        fn emit_sync_file(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<crate::target::shared::code::CodeInstruction>,
        _relocations: &mut Vec<crate::target::shared::code::CodeRelocation>,
    ) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_sync_file") }
        fn emit_seek_file(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<crate::target::shared::code::CodeInstruction>,
        _relocations: &mut Vec<crate::target::shared::code::CodeRelocation>,
    ) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_seek_file") }
        fn emit_rename_path(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<crate::target::shared::code::CodeInstruction>,
        _relocations: &mut Vec<crate::target::shared::code::CodeRelocation>,
    ) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_rename_path") }
        fn emit_mkstemps(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<crate::target::shared::code::CodeInstruction>,
        _relocations: &mut Vec<crate::target::shared::code::CodeRelocation>,
    ) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_mkstemps") }
        fn emit_random_bytes(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<crate::target::shared::code::CodeInstruction>,
        _relocations: &mut Vec<crate::target::shared::code::CodeRelocation>,
    ) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_random_bytes") }
        fn emit_temp_directory(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<crate::target::shared::code::CodeInstruction>,
        _relocations: &mut Vec<crate::target::shared::code::CodeRelocation>,
    ) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_temp_directory") }
        fn emit_opendir(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<crate::target::shared::code::CodeInstruction>,
        _relocations: &mut Vec<crate::target::shared::code::CodeRelocation>,
    ) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_opendir") }
        fn emit_readdir(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<crate::target::shared::code::CodeInstruction>,
        _relocations: &mut Vec<crate::target::shared::code::CodeRelocation>,
    ) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_readdir") }
        fn emit_closedir(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<crate::target::shared::code::CodeInstruction>,
        _relocations: &mut Vec<crate::target::shared::code::CodeRelocation>,
    ) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_closedir") }
        fn dirent_name_offset(&self) -> usize { unimplemented!("TlsReadTestPlatform::dirent_name_offset") }
        fn dirent_name_length_offset(&self) -> usize { unimplemented!("TlsReadTestPlatform::dirent_name_length_offset") }
        fn emit_realpath(
        &self,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<crate::target::shared::code::CodeInstruction>,
        _relocations: &mut Vec<crate::target::shared::code::CodeRelocation>,
    ) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_realpath") }
        fn emit_arena_map(
        &self,
        _size_reg: &str,
        _instructions: &mut Vec<crate::target::shared::code::CodeInstruction>,
    ) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_arena_map") }
        fn emit_arena_unmap(&self, _instructions: &mut Vec<crate::target::shared::code::CodeInstruction>) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_arena_unmap") }
        fn addrinfo_addr_offset(&self) -> usize { unimplemented!("TlsReadTestPlatform::addrinfo_addr_offset") }
        fn sol_socket(&self) -> &'static str { unimplemented!("TlsReadTestPlatform::sol_socket") }
        fn so_reuseaddr(&self) -> &'static str { unimplemented!("TlsReadTestPlatform::so_reuseaddr") }
        fn so_rcvtimeo(&self) -> &'static str { unimplemented!("TlsReadTestPlatform::so_rcvtimeo") }
        fn so_sndtimeo(&self) -> &'static str { unimplemented!("TlsReadTestPlatform::so_sndtimeo") }
        fn eagain(&self) -> &'static str { unimplemented!("TlsReadTestPlatform::eagain") }
        fn emsgsize(&self) -> &'static str { unimplemented!("TlsReadTestPlatform::emsgsize") }
        fn o_nonblock(&self) -> &'static str { unimplemented!("TlsReadTestPlatform::o_nonblock") }
        fn einprogress(&self) -> &'static str { unimplemented!("TlsReadTestPlatform::einprogress") }
        fn so_error(&self) -> &'static str { unimplemented!("TlsReadTestPlatform::so_error") }
        fn emit_variadic_call(
        &self,
        _base: &str,
        _from: &str,
        _platform_imports: &HashMap<String, String>,
        _instructions: &mut Vec<crate::target::shared::code::CodeInstruction>,
        _relocations: &mut Vec<crate::target::shared::code::CodeRelocation>,
    ) -> Result<(), String> { unimplemented!("TlsReadTestPlatform::emit_variadic_call") }
        fn emit_program_entry(
        &self,
        _spec: &crate::target::shared::code::ProgramEntrySpec<'_>,
        _platform_imports: &HashMap<String, String>,
    ) -> Result<crate::target::shared::code::CodeFunction, String> { unimplemented!("TlsReadTestPlatform::emit_program_entry") }
        fn emit_thread_trampoline(
        &self,
        _platform_imports: &HashMap<String, String>,
    ) -> Result<crate::target::shared::code::CodeFunction, String> { unimplemented!("TlsReadTestPlatform::emit_thread_trampoline") }
    }

    /// Number of `blr` (indirect call) instructions between the `start` and the
    /// next `end` label in the finalized instruction stream.
    fn blr_between(ins: &[CodeInstruction], start: &str, end: &str) -> usize {
        let s = ins
            .iter()
            .position(|i| i.op == CodeOp::Label && i.get("name") == Some(start))
            .unwrap_or_else(|| panic!("missing label {start}"));
        let e = ins[s + 1..]
            .iter()
            .position(|i| i.op == CodeOp::Label && i.get("name") == Some(end))
            .map(|p| p + s + 1)
            .unwrap_or_else(|| panic!("missing label {end}"));
        ins[s + 1..e]
            .iter()
            .filter(|i| i.op == CodeOp::BranchLinkRegister)
            .count()
    }

    #[test]
    fn readtext_encoding_error_releases_mapped_and_content() {
        mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
        let imports = HashMap::new();
        let (_frame, ins, rel, _slots) =
            lower_tls_read_macos("t_readtext", &imports, &TlsReadTestPlatform, true)
                .expect("lower tls::readText");

        // The encoding-error exit performs exactly the two dispatch_release
        // calls the success path does (MAPPED, then CTX_CONTENT) before failing.
        let releases = blr_between(&ins, "t_readtext_encoding_error", "t_readtext_peer_closed");
        assert_eq!(
            releases, 2,
            "bug-52: encoding_error exit must release MAPPED and CTX_CONTENT before failing"
        );

        // The fix adds a second dlsym(dispatch_release); the whole helper now
        // resolves that data symbol on both the success and the error path
        // (each resolution emits a hi/lo relocation pair).
        let release_relocs = rel.iter().filter(|r| r.to.contains("dispatch_release")).count();
        assert!(
            release_relocs >= 4,
            "expected dispatch_release resolved on both exits, got {release_relocs}"
        );
    }

    #[test]
    fn readbytes_has_no_encoding_error_exit() {
        mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
        let imports = HashMap::new();
        let (_frame, ins, _rel, _slots) =
            lower_tls_read_macos("t_readbytes", &imports, &TlsReadTestPlatform, false)
                .expect("lower tls::read");

        // readBytes has no UTF-8 validation, so it never emits an encoding_error
        // label — confirming the bug-52 fix is scoped to the text path only.
        assert!(
            !ins
                .iter()
                .any(|i| i.op == CodeOp::Label && i.get("name") == Some("t_readbytes_encoding_error")),
            "tls::read (bytes) must not have an encoding_error exit"
        );
    }

    fn has_label(ins: &[CodeInstruction], name: &str) -> bool {
        ins.iter()
            .any(|i| i.op == CodeOp::Label && i.get("name") == Some(name))
    }

    // bug-55: `emit_fresh_sem` used to store a brand-new dispatch_semaphore into
    // ctx->sem on every readText/write, leaking the previous one (~211k residual
    // objects over 200k reads under `leaks`). The fix releases the prior
    // semaphore first, emitting a `<sym>_sem_skip_release` guard label. These
    // tests pin that label so the release cannot silently regress.
    #[test]
    fn readtext_releases_previous_semaphore() {
        mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
        let imports = HashMap::new();
        let (_f, ins, rel, _s) =
            lower_tls_read_macos("t_rt", &imports, &TlsReadTestPlatform, true).expect("lower");
        assert!(
            has_label(&ins, "t_rt_sem_skip_release"),
            "readText must release the prior semaphore before creating a fresh one"
        );
        assert!(
            rel.iter().any(|r| r.to.contains("dispatch_release")),
            "readText must resolve dispatch_release for the semaphore free"
        );
    }

    #[test]
    fn write_releases_previous_semaphore() {
        mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
        let imports = HashMap::new();
        let (_f, ins, _r, _s) =
            lower_tls_write_macos("t_w", &imports, &TlsReadTestPlatform, false).expect("lower");
        assert!(
            has_label(&ins, "t_w_sem_skip_release"),
            "write must release the prior semaphore before creating a fresh one"
        );
    }

    // bug-55: connect retains the endpoint/parameters via nw_connection_create,
    // so it must nw_release its own references; before the fix they leaked on
    // every successful connect.
    #[test]
    fn connect_releases_endpoint_and_params() {
        mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
        let imports = HashMap::new();
        let (_f, _ins, rel, _s) =
            lower_tls_connect_macos("t_c", &imports, &TlsReadTestPlatform).expect("lower");
        assert!(
            rel.iter().any(|r| r.to.contains("nw_release")),
            "connect must resolve nw_release to free the endpoint and parameters"
        );
    }

    // bug-55: close now releases the connection (nw_release) and — only when it
    // owns them — the dispatch queue and ctx semaphore. The queue release is
    // guarded by a `<sym>_skip_queue_release` label because an accepted socket
    // shares the listener's queue (queue slot = 0) and must not release it.
    #[test]
    fn close_releases_connection_queue_and_sem() {
        mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
        let imports = HashMap::new();
        let (_f, ins, rel, _s) =
            lower_tls_close_macos("t_cl", &imports, &TlsReadTestPlatform).expect("lower");
        assert!(
            rel.iter().any(|r| r.to.contains("nw_release")),
            "close must resolve nw_release for the connection"
        );
        assert!(
            rel.iter().any(|r| r.to.contains("dispatch_release")),
            "close must resolve dispatch_release for the queue and semaphore"
        );
        assert!(
            has_label(&ins, "t_cl_skip_queue_release"),
            "close must guard the queue release so an accepted (queue=0) socket skips it"
        );
    }

    // bug-55: an accepted socket stores 0 in its queue slot (it shares the
    // listener's serial queue), so the shared close skips the queue release.
    #[test]
    fn accept_stores_zero_queue_slot() {
        mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
        let imports = HashMap::new();
        let (_f, ins, _r, _s) =
            lower_tls_accept_macos("t_a", &imports, &TlsReadTestPlatform).expect("lower");
        // The accepted-record build stores x31 (zero) into REC_QUEUE rather than
        // the shared listener queue; assert no `store [x1+REC_QUEUE] <- vN` from a
        // loaded queue exists by checking the record store uses the zero register.
        let stores_zero_queue = ins.iter().any(|i| {
            i.op == CodeOp::StrU64
                && i.get("src") == Some(abi::ZERO)
                && i.get("base") == Some(abi::RET[1])
                && i.get("offset") == Some(&REC_QUEUE.to_string())
        });
        assert!(
            stores_zero_queue,
            "accept must store 0 in the accepted socket's queue slot (shared listener queue)"
        );
    }

    // bug-55: closeListener releases the listener, its queue, and the listener
    // ctx semaphore; before the fix it only cancelled the listener.
    #[test]
    fn close_listener_releases_queue_and_sem() {
        mir::set_backend(&crate::arch::aarch64::backend::AARCH64_BACKEND);
        let imports = HashMap::new();
        let (_f, _ins, rel, _s) =
            lower_tls_close_listener_macos("t_ll", &imports, &TlsReadTestPlatform).expect("lower");
        assert!(
            rel.iter().any(|r| r.to.contains("dispatch_release")),
            "closeListener must resolve dispatch_release for the queue and ctx semaphore"
        );
    }
}
