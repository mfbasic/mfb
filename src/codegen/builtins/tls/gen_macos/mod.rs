use super::gen_shared::*;
use crate::codegen::collection::layout::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::error::emission::*;
use crate::codegen::memory::arena::*;
use crate::codegen::memory::marshal::push_write_payload_view;
use crate::codegen::string::util::*;
use crate::target::shared::abi;
pub(crate) fn emit_port_itoa(
    symbol: &str,
    port_off: usize,
    portbuf_off: usize,
    portcstr_off: usize,
    ins: &mut Vec<CodeInstruction>,
    vregs: &mut Vregs,
) {
    let v9 = vregs.next();
    let v10 = vregs.next();
    let v11 = vregs.next();
    let v14 = vregs.next();
    let v15 = vregs.next();
    let v16 = vregs.next();
    let v13 = vregs.next();
    let itoa_loop = format!("{symbol}_itoa");
    ins.extend([
        abi::move_immediate(&v9, "Integer", "0"),
        abi::store_u8(&v9, abi::stack_pointer(), portbuf_off + 23),
        abi::load_u64(&v10, abi::stack_pointer(), port_off),
        abi::move_immediate(&v11, "Integer", "10"),
        abi::add_immediate(&v14, abi::stack_pointer(), portbuf_off + 22),
        abi::label(&itoa_loop),
        abi::unsigned_divide_registers(&v15, &v10, &v11),
        abi::multiply_subtract_registers(&v16, &v15, &v11, &v10),
        abi::add_immediate(&v16, &v16, 48),
        abi::store_u8(&v16, &v14, 0),
        abi::subtract_immediate(&v14, &v14, 1),
        abi::move_register(&v10, &v15),
        abi::compare_immediate(&v10, "0"),
        abi::branch_ne(&itoa_loop),
        abi::add_immediate(&v13, &v14, 1),
        abi::store_u64(&v13, abi::stack_pointer(), portcstr_off),
    ]);
}

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
// plan-76-B Phase 4: the poll readiness receive's completion block. Identical to
// RECV_INVOKE but writes the dedicated CTX_PCONTENT/CTX_PERROR slots and signals
// CTX_PSEM, so an outstanding poll receive never collides with the read/write
// CTX_SEM/CTX_CONTENT the per-op `emit_fresh_sem` recycles.
pub(crate) const RECV_POLL_INVOKE: &str = "_mfb_tls_nw_recv_poll_invoke";
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

// The handle record shares the canonical plan-80 header: tag@0, handle
// (nw_connection)@8, closed@16, STATE@24 — then the macOS tail { ctx@32,
// dispatch-queue@40 }. The `closed` flag sits at the canonical resource
// closed-flag offset (plan-38 F7) so the backend-independent closed-default
// (which zeroes the record and sets the closed byte) marks this record closed
// too. STATE@24 is the generic union payload slot (plan-74/80), null until the
// bind lazy-inits it. All record accesses go through these named constants, so
// the re-slot is transparent.
const REC_TAG: usize = RESOURCE_OFFSET_TAG;
const REC_CONN: usize = RESOURCE_OFFSET_HANDLE;
const REC_CLOSED: usize = RESOURCE_OFFSET_CLOSED;
const REC_STATE: usize = RESOURCE_OFFSET_STATE;
pub(crate) const REC_CTX: usize = 32;
pub(crate) const REC_QUEUE: usize = 40;
/// Listener-only tail slot (bug-465): the NUL-terminated host the listener was
/// bound to. `tls::localAddress(listener)` needs a host and a port, and macOS can
/// supply only the port — a `Listener`'s handle slot holds an `nw_listener`, not a
/// descriptor, and Network.framework exposes `nw_listener_get_port` but nothing
/// that answers the bound address. So `tls::listen` parks the C string it already
/// built for `nw_endpoint_create_host` here. It is either an arena copy of the
/// caller's host or the static `_mfb_tls_anyhost` (`"0.0.0.0"`), so the record
/// borrows it with nothing to release.
///
/// The arena copy is process-lifetime only for a handle that never leaves its
/// thread. Since bug-464 a TLS handle can be *transferred*, and an arena is
/// per-thread: a verbatim move would leave the receiver's
/// `tls::localAddress` reading a string the sender's teardown released. The
/// registry therefore declares this slot `SlotTransfer::ArenaCString` so the
/// transfer copy duplicates the string into the receiver's arena.
pub(crate) const REC_LHOST: usize = 48;
const REC_SIZE: &str = RESOURCE_RECORD_SIZE;

const _: () = assert!(REC_CLOSED == RESOURCE_OFFSET_CLOSED);
const _: () = assert!(REC_STATE == RESOURCE_OFFSET_STATE);
const _: () = assert!(REC_CONN == RESOURCE_OFFSET_HANDLE);
const _: () = assert!(REC_QUEUE + 8 <= RESOURCE_RECORD_SIZE_BYTES);
const _: () = assert!(REC_LHOST + 8 <= RESOURCE_RECORD_SIZE_BYTES);

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
                                         // plan-76-B Phase 4 (outstanding-receive model for tls::poll): a DEDICATED
                                         // semaphore + content/error slots for the poll readiness receive, isolated from
                                         // the per-op CTX_SEM/CTX_CONTENT that read/write recycle via `emit_fresh_sem`. The
                                         // isolation is what keeps `tls::write`/`tls::close` byte-identical — an outstanding
                                         // poll receive never touches CTX_SEM, so their fresh-sem invariant (bug-52/55) is
                                         // unaffected. `CTX_PSEM` is created once at connection-ctx setup and reused (at most
                                         // one poll receive is ever outstanding). `CTX_PEND_*` is the stashed decrypted
                                         // plaintext (a plain arena buffer, so no NW object lifetime is held across a
                                         // poll→read boundary); `CTX_ARMED` is 1 while a poll receive is in flight. These
                                         // slots live only in the CONNECTION ctx (a separate allocation from the listener
                                         // LCTX, whose ring starts at offset 48), so they do not collide with LCTX.
pub(crate) const CTX_PSEM: usize = 48;
pub(crate) const CTX_PCONTENT: usize = 56;
pub(crate) const CTX_PERROR: usize = 64;
pub(crate) const CTX_PEND_BUF: usize = 72; // stashed plaintext arena buffer (0 = none)
pub(crate) const CTX_PEND_LEN: usize = 80; // total bytes in CTX_PEND_BUF
pub(crate) const CTX_PEND_OFF: usize = 88; // consume cursor into CTX_PEND_BUF
pub(crate) const CTX_ARMED: usize = 96; // 1 while a poll receive is outstanding
                                        // plan-110-D Phase 2: the per-socket read/write deadlines `tls::setReadTimeout`
                                        // and `tls::setWriteTimeout` install. Linux and Windows push these down to the
                                        // OS as SO_RCVTIMEO/SO_SNDTIMEO; Network.framework owns the socket and has no
                                        // such knob, so macOS carries the policy here and bounds its own semaphore waits
                                        // with it. Both hold milliseconds, or `TIMEOUT_UNBOUNDED_SENTINEL` for "no
                                        // deadline" — the state a fresh connection starts in, which is what keeps an
                                        // unconfigured socket's wait FOREVER exactly as before.
pub(crate) const CTX_RTO: usize = 104;
pub(crate) const CTX_WTO: usize = 112;
// 1 while a send completion is outstanding. A write that hits its deadline
// cannot cancel the posted send — the completion block will still signal
// CTX_SEM later — so the next write must consume that stale signal instead of
// mistaking it for its own. This is the same outstanding-operation model
// plan-76-B gave the receive side with `CTX_ARMED`.
pub(crate) const CTX_WARMED: usize = 120;
const CTX_SIZE: &str = "128";

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
// bug-477: the verify block invokes the completion block it is handed, and a
// block invocation reads the target's `invoke` pointer at this offset.
pub(crate) const BLK_INVOKE: usize = 16;
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
// bug-477: three more captures so the configure block can also install the
// verify block. `CFG_CAP_VBLOCK` is NULL when `allowSelfSigned` is off, and
// `CFG_CAP_SNAME` is independently NULL when `serverName` is empty — the two
// decisions are separate, because the flag may be set with no `serverName` (the
// name then defaults to `host`, as on the other two backends). Total size 88.
pub(crate) const CFG_CAP_VBLOCK: usize = 64;
pub(crate) const CFG_CAP_SETVERIFYFN: usize = 72;
pub(crate) const CFG_CAP_QUEUE: usize = 80;
pub(crate) const CFG_BLOCK_SIZE: usize = 88;

// --- bug-477 `allowSelfSigned` (client-side verify block) -------------------
//
// The block Network.framework calls to decide the peer's chain. It captures only
// the server-name C string (block size 40, the same shape as the state/send/recv
// blocks); the framework entry points it calls live in a process-global slot
// table instead, because there are thirteen of them and a block capture list
// that long would need its own descriptor size.
//
// The block runs on the connection's dispatch queue — a DIFFERENT thread from
// the MFB worker — so it must not touch arena state (which is per-thread). It
// does not: it reads the global table, calls C, and invokes the completion block.
pub(crate) const VERIFY_INVOKE: &str = "_mfb_tls_nw_verify_invoke";
pub(crate) const VERIFY_FNS_SYMBOL: &str = "_mfb_tls_verify_fns";
pub(crate) const VERIFY_CAP_SNAME: usize = 32;

/// Slot offsets into [`VERIFY_FNS_SYMBOL`], in the order [`VERIFY_FN_NAMES`]
/// lists them.
pub(crate) const VFN_SLOT_BYTES: usize = 8;
pub(crate) const VERIFY_FNS_SIZE: usize = VERIFY_FN_NAMES.len() * VFN_SLOT_BYTES;

/// Every entry point the verify block calls, resolved once during `connect` and
/// published to the global table. Order defines the slot offsets.
///
/// `sec_trust_copy_ref` is Network.framework; the `Sec*` are Security.framework;
/// the `CF*` are CoreFoundation. `kCFTypeArrayCallBacks` is a DATA symbol (the
/// callbacks struct), not a function — the block passes its address to
/// `CFArrayCreate`, so it is resolved and stored the same way.
pub(crate) const VERIFY_FN_NAMES: &[(&str, Framework)] = &[
    ("sec_trust_copy_ref", Framework::Network),
    ("CFStringCreateWithCString", Framework::CoreFoundation),
    ("SecPolicyCreateSSL", Framework::Security),
    ("SecTrustSetPolicies", Framework::Security),
    ("SecTrustCopyCertificateChain", Framework::Security),
    ("CFArrayGetCount", Framework::CoreFoundation),
    ("CFArrayGetValueAtIndex", Framework::CoreFoundation),
    ("CFArrayCreate", Framework::CoreFoundation),
    ("SecTrustSetAnchorCertificates", Framework::Security),
    ("SecTrustSetAnchorCertificatesOnly", Framework::Security),
    ("SecTrustEvaluateWithError", Framework::Security),
    ("CFRelease", Framework::CoreFoundation),
    ("kCFTypeArrayCallBacks", Framework::CoreFoundation),
];

/// Which dlopen handle a [`VERIFY_FN_NAMES`] entry is resolved from.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Framework {
    Network,
    Security,
    CoreFoundation,
}

/// Slot offset of `name` in the global table.
pub(crate) fn verify_fn_slot(name: &str) -> usize {
    VERIFY_FN_NAMES
        .iter()
        .position(|(candidate, _)| *candidate == name)
        .map(|index| index * VFN_SLOT_BYTES)
        .unwrap_or_else(|| panic!("bug-477: `{name}` is not a verify-block entry point"))
}

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
    // bug-477 `allowSelfSigned`: installs the client verify block. Listed
    // unconditionally (like `nw_listener_get_port` above) because the configure
    // block captures the resolved pointer whether or not this call passes the
    // flag — a NULL capture is what turns it off, not a missing symbol.
    "sec_protocol_options_set_verify_block",
    // plan-110-D: the endpoint queries behind `tls::localAddress` /
    // `tls::remoteAddress`. Network.framework owns the socket and exposes no fd,
    // so these are how macOS answers what Linux/Windows answer with
    // getsockname/getpeername.
    "nw_connection_copy_current_path",
    "nw_path_copy_effective_local_endpoint",
    "nw_path_copy_effective_remote_endpoint",
    "nw_endpoint_get_address",
    // bug-465: the whole address surface an `nw_listener` exposes — it has no
    // descriptor for `getsockname`, so `tls::localAddress(listener)` reads the
    // port from here and the host from `REC_LHOST`.
    //
    // Listed with the CLIENT symbols even though only the server path can hold a
    // listener. The `localAddress` overload split is resolved at emission from
    // the argument's type, so the code layer force-emits the listener body
    // whenever `tls.localAddress` is present — including in a client-only module,
    // which would then relocate against a name the server-gated table had not
    // written. Gating the *synthesis* instead does not close it: a module can
    // take a `Listener` as a parameter without ever calling `listen`/`accept`.
    // One extra C string is the honest price; it is the only server-side symbol
    // the listener-address body touches.
    "nw_listener_get_port",
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

pub(crate) fn data_objects(server: bool) -> Vec<CodeDataObject> {
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
            layout: "Block_descriptor { u64 reserved=0; u64 size=88 }".to_string(),
            align: 8,
            size: 16,
            // reserved = 0, size = 88 (0x58), little-endian u64s (bug-477 added
            // the verify-block / set-verify-fn / queue captures)
            value: "00000000000000005800000000000000".to_string(),
        },
    ];
    for name in SYMBOLS {
        objects.push(raw_cstr(&sym_data_symbol(name), name));
    }
    // bug-477: the verify block's entry-point table, and the two frameworks the
    // client now needs for it. Security/CoreFoundation used to be server-only
    // (`sec_identity_create` and the PEM import); the client verify block calls
    // `Sec*`/`CF*` too, so their library names and symbol names move onto the
    // unconditional path. A `raw` object is writable, which the table needs —
    // `connect` fills it after `dlsym`, the block reads it.
    objects.push(CodeDataObject {
        symbol: VERIFY_FNS_SYMBOL.to_string(),
        kind: "raw".to_string(),
        layout: "void *[13] — the verify block's resolved entry points".to_string(),
        align: 8,
        size: VERIFY_FNS_SIZE,
        value: "0".repeat(VERIFY_FNS_SIZE * 2),
    });
    for (name, _) in VERIFY_FN_NAMES {
        if !SYMBOLS.contains(name) {
            objects.push(raw_cstr(&sym_data_symbol(name), name));
        }
    }
    if !server {
        objects.push(raw_cstr(MACSEC_SYMBOL, MACSEC));
        objects.push(raw_cstr(MACCF_SYMBOL, MACCF));
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
    vregs: &mut Vregs,
) -> Result<(), String> {
    let v9 = vregs.next();
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
            abi::load_u64(&v9, abi::stack_pointer(), fnptr_off),
            abi::branch_link_register(&v9),
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
    vregs: &mut Vregs,
) -> Result<(), String> {
    let v9 = vregs.next();
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
        abi::load_u64(&v9, abi::stack_pointer(), fnptr_off),
        abi::branch_link_register(&v9),
    ]);
    Ok(())
}

/// Build a 40-byte block literal at `sp + block_off` whose `invoke` is
/// `invoke_symbol` and whose single captured variable is the ctx pointer at
/// `sp + ctx_off`.
#[allow(clippy::too_many_arguments)]
fn emit_build_block(
    ctx: &mut EmitCtx,
    handle_off: usize,
    invoke_symbol: &str,
    ctx_off: usize,
    block_off: usize,
    fnptr_off: usize,
    fail: &str,
    vregs: &mut Vregs,
) -> Result<(), String> {
    let v9 = vregs.next();
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
        abi::load_u64(&v9, abi::stack_pointer(), fnptr_off),
        abi::store_u64(&v9, abi::stack_pointer(), block_off + BLK_ISA),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), block_off + BLK_FLAGS),
    ]);
    emit_data_address(
        symbol,
        &v9,
        invoke_symbol,
        ctx.instructions,
        ctx.relocations,
    );
    ctx.instructions.push(abi::store_u64(
        &v9,
        abi::stack_pointer(),
        block_off + BLK_INVOKE,
    ));
    emit_data_address(symbol, &v9, DESC_SYMBOL, ctx.instructions, ctx.relocations);
    ctx.instructions.push(abi::store_u64(
        &v9,
        abi::stack_pointer(),
        block_off + BLK_DESC,
    ));
    ctx.instructions.extend([
        abi::load_u64(&v9, abi::stack_pointer(), ctx_off),
        abi::store_u64(&v9, abi::stack_pointer(), block_off + BLK_CAP),
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
    vregs: &mut Vregs,
) -> Result<(), String> {
    let v9 = vregs.next();
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
        abi::load_u64(&v9, abi::stack_pointer(), ctx_off),
        abi::load_u64(abi::return_register(), &v9, CTX_SEM),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&skip_release),
        abi::load_u64(&v9, abi::stack_pointer(), fnptr_off),
        abi::branch_link_register(&v9),
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
        abi::load_u64(&v9, abi::stack_pointer(), fnptr_off),
        abi::branch_link_register(&v9),
        abi::load_u64(&v9, abi::stack_pointer(), ctx_off),
        abi::store_u64(abi::return_register(), &v9, CTX_SEM),
        abi::store_u64(abi::ZERO, &v9, CTX_CONTENT),
        abi::store_u64(abi::ZERO, &v9, CTX_ERROR),
    ]);
    Ok(())
}

/// Wait on one of the ctx semaphores, bounded by a deadline held in a ctx slot,
/// branching to `timeout` when the deadline elapses first.
///
/// This is [`emit_wait`] with a deadline instead of `DISPATCH_TIME_FOREVER`.
/// Network.framework owns the socket, so macOS cannot express a read/write
/// deadline as `SO_RCVTIMEO`/`SO_SNDTIMEO` the way Linux and Windows do; the
/// deadline lives in `CTX_RTO`/`CTX_WTO` and is applied here, at the only place
/// the operation actually blocks.
///
/// The timeout convention (`.ai/net-tls.md`) maps straight onto `dispatch_time`:
/// the unbounded sentinel is `DISPATCH_TIME_FOREVER`, `0` is `DISPATCH_TIME_NOW`
/// (one immediate attempt), and a positive value is `dispatch_time(NOW, ms*1e6)`.
/// `tag` distinguishes this call site's labels; `deadline_off` is a scratch stack
/// slot in the caller's frame.
#[allow(clippy::too_many_arguments)]
fn emit_wait_bounded(
    ctx: &mut EmitCtx,
    handle_off: usize,
    ctx_off: usize,
    fnptr_off: usize,
    deadline_off: usize,
    sem_slot: usize,
    timeout_slot: usize,
    tag: &str,
    timeout: &str,
    fail: &str,
    vregs: &mut Vregs,
) -> Result<(), String> {
    let v9 = vregs.next();
    let v10 = vregs.next();
    let symbol = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;
    let wait_now = format!("{symbol}_{tag}_wait_now");
    let wait_forever = format!("{symbol}_{tag}_wait_forever");
    let ready = format!("{symbol}_{tag}_deadline_ready");

    ctx.instructions.extend([
        abi::load_u64(&v9, abi::stack_pointer(), ctx_off),
        abi::load_u64(&v9, &v9, timeout_slot),
        abi::move_immediate(&v10, "Integer", TIMEOUT_UNBOUNDED_SENTINEL),
        abi::compare_registers(&v9, &v10),
        abi::branch_eq(&wait_forever),
        abi::compare_immediate(&v9, "0"),
        abi::branch_eq(&wait_now),
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
        "dispatch_time",
        fnptr_off,
        fail,
    )?;
    ctx.instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", "0"), // DISPATCH_TIME_NOW
        abi::load_u64(&v9, abi::stack_pointer(), ctx_off),
        abi::load_u64(abi::c_arg(1), &v9, timeout_slot),
        abi::move_immediate(&v10, "Integer", "1000000"),
        abi::multiply_registers(abi::c_arg(1), abi::c_arg(1), &v10),
        abi::load_u64(&v9, abi::stack_pointer(), fnptr_off),
        abi::branch_link_register(&v9),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), deadline_off),
        abi::branch(&ready),
        abi::label(&wait_now),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), deadline_off),
        abi::branch(&ready),
        abi::label(&wait_forever),
        abi::move_immediate(&v9, "Integer", "0"),
        abi::bitwise_not(&v9, &v9), // DISPATCH_TIME_FOREVER
        abi::store_u64(&v9, abi::stack_pointer(), deadline_off),
        abi::label(&ready),
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
        "dispatch_semaphore_wait",
        fnptr_off,
        fail,
    )?;
    ctx.instructions.extend([
        abi::load_u64(&v9, abi::stack_pointer(), ctx_off),
        abi::load_u64(abi::return_register(), &v9, sem_slot),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), deadline_off),
        abi::load_u64(&v10, abi::stack_pointer(), fnptr_off),
        abi::branch_link_register(&v10),
        // Non-zero => the deadline elapsed before the operation signalled.
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(timeout),
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
        .push(abi::move_immediate(abi::c_arg(1), "Integer", RTLD_NOW));
    platform.emit_external_call(
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

mod address;
mod client;
mod server;
#[cfg(test)]
mod tests;
mod timeout;

pub(crate) use address::{lower_tls_address_macos, lower_tls_listener_address_macos};
pub(crate) use client::{
    lower_tls_close_macos, lower_tls_connect_macos, lower_tls_poll_macos, lower_tls_read_macos,
    lower_tls_write_macos,
};
pub(crate) use server::{
    lower_tls_accept_macos, lower_tls_close_listener_macos, lower_tls_listen_macos,
};
pub(crate) use timeout::lower_tls_set_timeout_macos;
