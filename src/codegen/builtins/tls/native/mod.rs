//! Native code generation for the built-in `tls` package (transport-layer
//! security). The Linux backend drives the system OpenSSL via `dlopen`/`dlsym`
//! so one binary spans OpenSSL 1.1.1 and 3.x (plan-03-net.md §4). The macOS
//! backend (see the `macos` submodule) drives Network.framework through a
//! dispatch-semaphore synchronous bridge.
//!
//! On Linux a `TlsSocket` handle is an arena record with the canonical plan-80
//! header (`tag`@0, `fd`@8, `closed`@16, `STATE`@24) and a TLS tail: the
//! `SSL_CTX*` at 32 and the `SSL*` at 40. Each helper
//! re-`dlopen`s `libssl` (cheap once loaded — it just bumps the refcount) and
//! `dlsym`s the `SSL_*` symbols it needs; `dlsym` resolves the library's default
//! symbol version, which is why a single binary works against both OpenSSL
//! series. The macOS record layout differs and is documented in `macos`.

use std::collections::HashMap;

use crate::target::shared::abi;
use crate::target::shared::code::native_helpers::{
    emit_data_address, emit_fail, hex_encode_cstring,
};
use crate::target::shared::code::*;

// TLS handles share the canonical resource-record header (plan-80): tag@0,
// fd (handle)@8, closed@16, STATE@24 — then the TLS-specific `SSL_CTX*`/`SSL*`
// tail at 32+. Before plan-80 this record was 32 bytes with `SSL*` at 16, which
// collided with the generic `STATE` slot and SIGSEGV'd a `Stream STATE` union
// over a `TlsSocket` (plan-76-D D4). An accepted (server-side) `TlsSocket`
// stores 0 in the `SSL_CTX*` slot: the marker that it points at the listener's
// shared server context and must not free it (plan-06-tls-server.md §5.1).
pub(super) const TLS_OFFSET_FD: usize = RESOURCE_OFFSET_HANDLE;
pub(super) const TLS_OFFSET_CLOSED: usize = RESOURCE_OFFSET_CLOSED;
pub(super) const TLS_OFFSET_STATE: usize = RESOURCE_OFFSET_STATE;
pub(super) const TLS_OFFSET_CTX: usize = 32;
pub(super) const TLS_OFFSET_SSL: usize = 40;
pub(super) const TLS_RECORD_SIZE: &str = RESOURCE_RECORD_SIZE;

// The schannel (Windows) backend stores a pointer to its separate SSPI
// credential/context arena block in the record. Before plan-80 that pointer sat
// at offset 16 (a bare literal); the header now owns 0..32, so it moves to the
// TLS-specific tail at 40 (there is no `SSL*` slot on Windows — SSPI keeps its
// handles in the block this points at).
pub(super) const TLS_SCHANNEL_OFFSET_BLOCK: usize = 40;

// The `TlsListener` record: the listening fd plus the server `SSL_CTX*` it owns
// (freed exactly once, when the listener closes). Shares the canonical header;
// the `SSL_CTX*` moves to the type-specific tail at 32 (plan-80).
pub(super) const TLS_LISTENER_OFFSET_FD: usize = RESOURCE_OFFSET_HANDLE;
pub(super) const TLS_LISTENER_OFFSET_CLOSED: usize = RESOURCE_OFFSET_CLOSED;
pub(super) const TLS_LISTENER_OFFSET_CTX: usize = 32;

// Both OpenSSL records place `closed`/`STATE` at the canonical resource offsets
// (plan-38, plan-80), so the backend-independent closed-default and union STATE
// land on the right bytes. The macOS Network.framework backend carries its own
// `REC_CLOSED`/`REC_STATE` asserts in `macos.rs`.
const _: () = assert!(TLS_OFFSET_CLOSED == RESOURCE_OFFSET_CLOSED);
const _: () = assert!(TLS_LISTENER_OFFSET_CLOSED == RESOURCE_OFFSET_CLOSED);
const _: () = assert!(TLS_OFFSET_STATE == RESOURCE_OFFSET_STATE);
const _: () = assert!(TLS_OFFSET_FD == RESOURCE_OFFSET_HANDLE);
// The widest TLS tail (`SSL*`@40) fits inside the shared envelope.
const _: () = assert!(TLS_OFFSET_SSL + 8 <= RESOURCE_RECORD_SIZE_BYTES);
const _: () = assert!(TLS_SCHANNEL_OFFSET_BLOCK + 8 <= RESOURCE_RECORD_SIZE_BYTES);

pub(super) const SOCK_STREAM: &str = "1";
pub(super) const HINTS_FAMILY_WORD: &str = "8589934592"; // ai_family = AF_INET (2 << 32), ai_flags = 0
pub(super) const HINTS_FAMILY_WORD_PASSIVE: &str = "8589934593"; // ai_flags = AI_PASSIVE (1)
pub(super) const RTLD_NOW: &str = "2";

// OpenSSL constants (stable across 1.1.1 and 3.x).
pub(super) const SSL_VERIFY_PEER: &str = "1";
pub(super) const SSL_CTRL_SET_TLSEXT_HOSTNAME: &str = "55";
pub(super) const TLSEXT_NAMETYPE_HOST_NAME: &str = "0";
pub(super) const SSL_CTRL_SET_MIN_PROTO_VERSION: &str = "123";
pub(super) const TLS1_2_VERSION: &str = "771"; // 0x0303

/// Candidate `libssl` sonames, tried in order at load time. `.so.3` first
/// (OpenSSL 3.x), then `.so.1.1` (OpenSSL 1.1.1).
pub(super) const TLS_LIB_NAMES: &[&str] = &["libssl.so.3", "libssl.so.1.1"];

/// Every OpenSSL symbol the client-side helpers `dlsym`. Each gets a read-only
/// C-string data object so the load can name it.
pub(super) const TLS_SYMBOLS: &[&str] = &[
    "TLS_client_method",
    "SSL_CTX_new",
    "SSL_CTX_set_default_verify_paths",
    "SSL_new",
    "SSL_set_fd",
    "SSL_set_verify",
    "SSL_set1_host",
    "SSL_ctrl",
    "SSL_connect",
    "SSL_get_verify_result",
    "SSL_read",
    // plan-76-B: non-consuming count of already-decrypted, buffered app bytes —
    // the readiness fast-path for `tls::poll` (a TLS record can hold app bytes with
    // the fd idle, so an fd-only poll would under-report).
    "SSL_pending",
    "SSL_write",
    "SSL_shutdown",
    "SSL_free",
    "SSL_CTX_free",
];

/// The additional server-side entry points (`tls::listen`/`tls::accept`).
/// Their name strings are emitted only when a module uses a server helper, so
/// client-only programs stay byte-identical (plan-06-tls-server.md §1).
pub(super) const TLS_SERVER_SYMBOLS: &[&str] = &[
    "TLS_server_method",
    "SSL_CTX_ctrl",
    "SSL_CTX_use_certificate_chain_file",
    "SSL_CTX_use_PrivateKey_file",
    "SSL_CTX_check_private_key",
    "SSL_accept",
];

fn lib_data_symbol(index: usize) -> String {
    format!("_mfb_tls_lib_{index}")
}

pub(super) fn sym_data_symbol(name: &str) -> String {
    format!("_mfb_tls_sym_{name}")
}

/// Read-only C-string data objects (library sonames + symbol names) referenced
/// by the TLS helpers. Emitted once when a module uses any `tls` call; the
/// server-only symbol names are appended only when a server helper
/// (`tls.listen`/`tls.accept`/`tls.closeListener`) is in the plan.
pub(crate) fn tls_cstring_data_objects(server: bool) -> Vec<CodeDataObject> {
    let mut objects = Vec::new();
    for (index, name) in TLS_LIB_NAMES.iter().enumerate() {
        objects.push(CodeDataObject {
            symbol: lib_data_symbol(index),
            kind: "raw".to_string(),
            layout: "C string (NUL-terminated)".to_string(),
            align: 1,
            size: name.len() + 1,
            value: hex_encode_cstring(name),
        });
    }
    let symbols: Box<dyn Iterator<Item = &&str>> = if server {
        Box::new(TLS_SYMBOLS.iter().chain(TLS_SERVER_SYMBOLS.iter()))
    } else {
        Box::new(TLS_SYMBOLS.iter())
    };
    for name in symbols {
        objects.push(CodeDataObject {
            symbol: sym_data_symbol(name),
            kind: "raw".to_string(),
            layout: "C string (NUL-terminated)".to_string(),
            align: 1,
            size: name.len() + 1,
            value: hex_encode_cstring(name),
        });
    }
    objects
}

/// Copy a NUL-free MFBASIC `String` (pointer at `sp + str_off`) into a freshly
/// allocated NUL-terminated C string, storing the result pointer at
/// `sp + out_off`. Branches to `alloc_fail` on allocation failure.
pub(super) fn emit_cstring(
    symbol: &str,
    prefix: &str,
    str_off: usize,
    out_off: usize,
    alloc_fail: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    let copy_loop = format!("{symbol}_{prefix}_cstr_copy");
    let copy_done = format!("{symbol}_{prefix}_cstr_done");
    instructions.extend([
        abi::load_u64("%v17", abi::stack_pointer(), str_off),
        abi::load_u64("%v18", "%v17", 0),
        abi::add_immediate(abi::return_register(), "%v18", 1),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
    ]);
    emit_alloc(symbol, instructions, relocations, alloc_fail);
    instructions.extend([
        abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), out_off),
        abi::load_u64("%v17", abi::stack_pointer(), str_off),
        abi::load_u64("%v18", "%v17", 0),
        abi::add_immediate("%v19", "%v17", 8),
        abi::move_register("%v20", abi::mfb_return(1)),
        abi::move_immediate("%v21", "Integer", "0"),
        abi::label(&copy_loop),
        abi::compare_registers("%v21", "%v18"),
        abi::branch_eq(&copy_done),
        abi::load_u8("%v22", "%v19", 0),
        abi::store_u8("%v22", "%v20", 0),
        abi::add_immediate("%v19", "%v19", 1),
        abi::add_immediate("%v20", "%v20", 1),
        abi::add_immediate("%v21", "%v21", 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::store_u8(abi::ZERO, "%v20", 0),
    ]);
}

/// `dlopen` `libssl.so.3`, falling back to `libssl.so.1.1`; the handle is stored
/// at `sp + handle_off`. Branches to `fail` when neither loads.
pub(super) fn emit_dlopen_libssl(
    ctx: &mut EmitCtx,
    handle_off: usize,
    fail: &str,
) -> Result<(), String> {
    let symbol = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    let loaded = format!("{symbol}_dlopen_done");
    emit_data_address(
        symbol,
        abi::return_register(),
        &lib_data_symbol(0),
        ctx.instructions,
        ctx.relocations,
    );
    ctx.instructions
        .push(abi::move_immediate(abi::c_arg(1), "Integer", RTLD_NOW));
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
        abi::branch_ne(&loaded),
    ]);
    emit_data_address(
        symbol,
        abi::return_register(),
        &lib_data_symbol(1),
        ctx.instructions,
        ctx.relocations,
    );
    ctx.instructions
        .push(abi::move_immediate(abi::c_arg(1), "Integer", RTLD_NOW));
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
        abi::label(&loaded),
    ]);
    Ok(())
}

/// `dlsym(handle, name)` into `sp + fnptr_off`. Branches to `fail` if missing.
pub(super) fn emit_dlsym(
    ctx: &mut EmitCtx,
    handle_off: usize,
    name: &str,
    fnptr_off: usize,
    fail: &str,
) -> Result<(), String> {
    let symbol = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    ctx.instructions.push(abi::load_u64(
        abi::return_register(),
        abi::stack_pointer(),
        handle_off,
    ));
    emit_data_address(
        symbol,
        abi::c_arg(1),
        &sym_data_symbol(name),
        ctx.instructions,
        ctx.relocations,
    );
    platform.emit_libc_call(
        "dlsym",
        symbol,
        platform_imports,
        ctx.instructions,
        ctx.relocations,
    )?;
    ctx.instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), fnptr_off),
    ]);
    Ok(())
}

/// Emit `setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO/SO_SNDTIMEO, &tv, 16)` for the
/// `timeval` already stored at `sp + tv_off`. Used on Linux to bound the
/// blocking TLS handshake by `timeoutMs` (and, with a zero `timeval`, to clear
/// the bound afterwards so `read`/`write` stay unbounded). Best effort: a
/// `setsockopt` failure is ignored — the handshake just falls back to blocking.
pub(super) fn emit_set_sock_timeouts(
    ctx: &mut EmitCtx,
    fd_off: usize,
    tv_off: usize,
) -> Result<(), String> {
    let symbol = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;

    // plan-73-D: Winsock `SO_RCVTIMEO`/`SO_SNDTIMEO` optval is a DWORD of
    // milliseconds (4 bytes), NOT a POSIX `struct timeval` (16 bytes). Callers store
    // the matching shape at `tv_off` per platform; pass the right optlen here (a
    // 16-byte optval to Winsock silently fails to install the timeout, so the socket
    // op then blocks forever).
    let optlen = if platform.family() == PlatformFamily::Windows {
        "4"
    } else {
        "16"
    };
    for opt in [platform.so_rcvtimeo(), platform.so_sndtimeo()] {
        ctx.instructions.extend([
            abi::load_u64(abi::return_register(), abi::stack_pointer(), fd_off),
            abi::move_immediate(abi::c_arg(1), "Integer", platform.sol_socket()),
            abi::move_immediate(abi::c_arg(2), "Integer", opt),
            abi::add_immediate(abi::c_arg(3), abi::stack_pointer(), tv_off),
            abi::move_immediate(abi::c_arg(4), "Integer", optlen),
        ]);
        // plan-73-D: setsockopt has 5 int args; on Win64 the 5th (optlen) is a stack
        // argument above the shadow, not a register (bug-384) — a garbage optlen makes
        // SO_*TIMEO silently fail to install and the later recv blocks forever. Route
        // through emit_external_int_call, which spills the overflow arg on Win64 and
        // is byte-identical on POSIX (all 5 fit in registers → plain emit_libc_call).
        crate::target::shared::code::native_helpers::emit_external_int_call(
            platform,
            "setsockopt",
            symbol,
            5,
            platform_imports,
            ctx.instructions,
            ctx.relocations,
        )?;
    }
    Ok(())
}

pub(crate) mod macos;
mod openssl;
pub(crate) mod schannel;

/// The single family-generic OS-seam entry for every `tls::` member — the twin
/// idiom (`Body::native_os_seam(Some(lower_tls_helper), Some(lower_tls_helper),
/// …)` in each `func_*.rs`): the generic runtime-call dispatch
/// (`crate::codegen::os::dispatch_runtime_helper` → `registry::os_helper`) routes a
/// `tls.*` runtime call here by member name / os-alias, and this matches the call
/// to its per-helper family dispatcher (which each branch on `platform.family()`
/// for openssl / schannel / macos). Covers the descriptor members plus the two
/// code-form aliases (`tls.pollList`, `tls.closeListener`).
pub(crate) fn lower_tls_helper(
    call: &str,
    symbol: &str,
    _ctx: &crate::codegen::registry::OsLowerCtx,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    match call {
        "tls.connect" => lower_tls_connect_helper(symbol, platform_imports, platform),
        "tls.listen" => lower_tls_listen_helper(symbol, platform_imports, platform),
        "tls.accept" => lower_tls_accept_helper(symbol, platform_imports, platform),
        "tls.read" => lower_tls_read_helper(symbol, platform_imports, platform, false),
        "tls.readText" => lower_tls_read_helper(symbol, platform_imports, platform, true),
        "tls.write" => lower_tls_write_helper(symbol, platform_imports, platform, false),
        "tls.writeText" => lower_tls_write_helper(symbol, platform_imports, platform, true),
        "tls.poll" => lower_tls_poll_helper(symbol, platform_imports, platform),
        "tls.pollList" => lower_tls_poll_list_helper(symbol, platform_imports, platform),
        "tls.close" => lower_tls_close_helper(symbol, platform_imports, platform),
        "tls.closeListener" => lower_tls_close_listener_helper(symbol, platform_imports, platform),
        other => Err(format!(
            "native code plan does not emit runtime call '{other}'"
        )),
    }
}

// Per-helper platform dispatch, done once here in the package parent — mirroring
// `crypto_ec::lower_crypto_ec_helper` — so neither backend is the entry point
// that owns the other's dispatch (bug-330). Each backend file is a pure
// `*_openssl` / `*_macos` implementation; the macOS decision lives only here.
pub(super) fn lower_tls_connect_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    match platform.family() {
        PlatformFamily::MacOS => macos::lower_tls_connect_macos(symbol, platform_imports, platform),
        PlatformFamily::Linux => {
            openssl::lower_tls_connect_openssl(symbol, platform_imports, platform)
        }
        PlatformFamily::Windows => schannel::lower_tls_connect(symbol, platform_imports, platform),
    }
}

pub(super) fn lower_tls_listen_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    match platform.family() {
        PlatformFamily::MacOS => macos::lower_tls_listen_macos(symbol, platform_imports, platform),
        PlatformFamily::Linux => {
            openssl::lower_tls_listen_openssl(symbol, platform_imports, platform)
        }
        PlatformFamily::Windows => schannel::lower_tls_listen(symbol, platform_imports, platform),
    }
}

pub(super) fn lower_tls_accept_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    match platform.family() {
        PlatformFamily::MacOS => macos::lower_tls_accept_macos(symbol, platform_imports, platform),
        PlatformFamily::Linux => {
            openssl::lower_tls_accept_openssl(symbol, platform_imports, platform)
        }
        PlatformFamily::Windows => schannel::lower_tls_accept(symbol, platform_imports, platform),
    }
}

pub(super) fn lower_tls_read_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    text: bool,
) -> HelperResult {
    match platform.family() {
        PlatformFamily::MacOS => {
            macos::lower_tls_read_macos(symbol, platform_imports, platform, text)
        }
        PlatformFamily::Linux => {
            openssl::lower_tls_read_openssl(symbol, platform_imports, platform, text)
        }
        PlatformFamily::Windows => {
            schannel::lower_tls_read(symbol, platform_imports, platform, text)
        }
    }
}

pub(super) fn lower_tls_write_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    text: bool,
) -> HelperResult {
    match platform.family() {
        PlatformFamily::MacOS => {
            macos::lower_tls_write_macos(symbol, platform_imports, platform, text)
        }
        PlatformFamily::Linux => {
            openssl::lower_tls_write_openssl(symbol, platform_imports, platform, text)
        }
        PlatformFamily::Windows => {
            schannel::lower_tls_write(symbol, platform_imports, platform, text)
        }
    }
}

/// plan-76-B: `tls::poll(sock[, timeoutMs]) AS Boolean` — the TLS readiness query,
/// per-backend (openssl `SSL_pending`+`poll`, schannel carry-over+`WSAPoll`, macOS
/// the outstanding-receive model).
pub(super) fn lower_tls_poll_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    match platform.family() {
        PlatformFamily::MacOS => macos::lower_tls_poll_macos(symbol, platform_imports, platform),
        PlatformFamily::Linux => {
            openssl::lower_tls_poll_openssl(symbol, platform_imports, platform)
        }
        PlatformFamily::Windows => schannel::lower_tls_poll(symbol, platform_imports, platform),
    }
}

/// plan-76-C: `tls::poll(List OF RES TlsSocket[, timeoutMs]) AS TlsSocket` — the TLS
/// readiness multiplex. Blocks until one socket in the list is readable, then returns
/// a BORROWED pointer to the first ready one (lowest index); the list keeps ownership
/// and closes each socket on scope exit (§15.6). Empty list → `ErrInvalidArgument`;
/// expiry with none ready → `ErrTimeout` (producing call).
///
/// **Portable, backend-uniform**: rather than a per-backend fd/ring multiplex, it
/// reuses the per-backend scalar readiness predicate — it scans each socket with
/// `_mfb_rt_tls_tls_poll(rec, 0)` (non-blocking) and, when none is ready, waits a
/// bounded slice on the first socket (also via the scalar helper) before rescanning,
/// so every backend's buffered + raw readiness is honoured with no new native code.
/// The scalar helper also propagates any per-socket error (e.g. a closed socket).
/// `x0` = list ptr, `x1` = timeoutMs.
pub(super) fn lower_tls_poll_list_helper(
    symbol: &str,
    _platform_imports: &HashMap<String, String>,
    _platform: &dyn CodegenPlatform,
) -> HelperResult {
    const FRAME_SIZE: usize = 64;
    const LIST_OFF: usize = 8;
    const COUNT_OFF: usize = 16;
    const DATABASE_OFF: usize = 24;
    const I_OFF: usize = 32;
    const ROUNDS_OFF: usize = 40;
    const INFINITE_OFF: usize = 48; // 1 = block until ready (omitted timeout)
    const SLICE_MS: &str = "20";
    const SCALAR: &str = "_mfb_rt_tls_tls_poll";

    let invalid = format!("{symbol}_invalid");
    let timeout_lbl = format!("{symbol}_timeout");
    let mode_infinite = format!("{symbol}_mode_infinite");
    let mode_zero = format!("{symbol}_mode_zero");
    let rounds_done = format!("{symbol}_rounds_done");
    let round_loop = format!("{symbol}_round_loop");
    let scan_loop = format!("{symbol}_scan_loop");
    let scan_done = format!("{symbol}_scan_done");
    let do_wait = format!("{symbol}_do_wait");
    let found = format!("{symbol}_found");
    let done = format!("{symbol}_done");

    let mut ins = vec![abi::label("entry")];
    let mut rel = Vec::new();
    // Loads socks[i]'s record ptr into `dst`: entry = list+HEADER+i*ENTRY_SIZE;
    // rec = load(data_base + load(entry+VALUE_OFFSET)). Uses %v13/%v14 as scratch.
    // (list ptr in LIST_OFF, data_base in DATABASE_OFF, index reg in `idx`.)
    let load_elem = |ins: &mut Vec<CodeInstruction>, dst: Operand, idx: &str| {
        ins.extend([
            abi::load_u64("%v13", abi::stack_pointer(), LIST_OFF),
            abi::move_immediate("%v14", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
            abi::multiply_registers("%v14", idx, "%v14"),
            abi::add_immediate("%v13", "%v13", COLLECTION_HEADER_SIZE),
            abi::add_registers("%v13", "%v13", "%v14"),
            abi::load_u64("%v13", "%v13", COLLECTION_ENTRY_OFFSET_VALUE_OFFSET),
            abi::load_u64("%v14", abi::stack_pointer(), DATABASE_OFF),
            abi::add_registers("%v13", "%v14", "%v13"),
            abi::load_u64(dst, "%v13", 0),
        ]);
    };
    ins.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), LIST_OFF),
        // count = socks.count; reject empty.
        abi::load_u64("%v9", abi::return_register(), COLLECTION_OFFSET_COUNT),
        abi::compare_immediate("%v9", "0"),
        abi::branch_eq(&invalid),
        abi::store_u64("%v9", abi::stack_pointer(), COUNT_OFF),
        // data_base = list + HEADER + capacity * ENTRY_SIZE (kind-0 resource list).
        abi::load_u64("%v10", abi::return_register(), COLLECTION_OFFSET_CAPACITY),
        abi::move_immediate("%v11", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers("%v10", "%v10", "%v11"),
        abi::add_immediate("%v11", abi::return_register(), COLLECTION_HEADER_SIZE),
        abi::add_registers("%v11", "%v11", "%v10"),
        abi::store_u64("%v11", abi::stack_pointer(), DATABASE_OFF),
        // Timeout → mode: sentinel = infinite (block); <0 = invalid; 0 = one scan,
        // no wait; >0 = ceil(t / SLICE) rounds.
        abi::store_u64(abi::ZERO, abi::stack_pointer(), INFINITE_OFF),
        abi::move_immediate("%v10", "Integer", TIMEOUT_UNBOUNDED_SENTINEL),
        abi::compare_registers(abi::c_arg(1), "%v10"),
        abi::branch_eq(&mode_infinite),
        abi::compare_immediate(abi::c_arg(1), "0"),
        abi::branch_lt(&invalid),
        abi::branch_eq(&mode_zero),
        // rounds = (t + SLICE - 1) / SLICE
        abi::move_immediate("%v10", "Integer", SLICE_MS),
        abi::add_immediate(abi::c_arg(1), abi::c_arg(1), 19),
        abi::unsigned_divide_registers("%v9", abi::c_arg(1), "%v10"),
        abi::store_u64("%v9", abi::stack_pointer(), ROUNDS_OFF),
        abi::branch(&rounds_done),
        abi::label(&mode_infinite),
        abi::move_immediate("%v9", "Integer", "1"),
        abi::store_u64("%v9", abi::stack_pointer(), INFINITE_OFF),
        abi::branch(&rounds_done),
        abi::label(&mode_zero),
        abi::move_immediate("%v9", "Integer", "1"),
        abi::store_u64("%v9", abi::stack_pointer(), ROUNDS_OFF),
        abi::label(&rounds_done),
        abi::label(&round_loop),
        // Scan every socket non-blocking.
        abi::move_immediate("%v9", "Integer", "0"),
        abi::store_u64("%v9", abi::stack_pointer(), I_OFF),
        abi::label(&scan_loop),
        abi::load_u64("%v9", abi::stack_pointer(), I_OFF),
        abi::load_u64("%v10", abi::stack_pointer(), COUNT_OFF),
        abi::compare_registers("%v9", "%v10"),
        abi::branch_ge(&scan_done),
    ]);
    load_elem(&mut ins, abi::return_register(), "%v9");
    ins.extend([
        abi::move_immediate(abi::c_arg(1), "Integer", "0"), // non-blocking check
        abi::branch_link(SCALAR),
        // Propagate any scalar error (closed socket etc.).
        abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG),
        abi::branch_ne(&done),
        abi::compare_immediate(RESULT_VALUE_REGISTER, "1"),
        abi::branch_eq(&found),
        abi::load_u64("%v9", abi::stack_pointer(), I_OFF),
        abi::add_immediate("%v9", "%v9", 1),
        abi::store_u64("%v9", abi::stack_pointer(), I_OFF),
        abi::branch(&scan_loop),
        abi::label(&scan_done),
        // None ready. Infinite mode always waits; otherwise decrement rounds and
        // raise ErrTimeout when exhausted (zero mode: rounds 1 → 0 → timeout, no wait).
        abi::load_u64("%v9", abi::stack_pointer(), INFINITE_OFF),
        abi::compare_immediate("%v9", "1"),
        abi::branch_eq(&do_wait),
        abi::load_u64("%v9", abi::stack_pointer(), ROUNDS_OFF),
        abi::subtract_immediate("%v9", "%v9", 1),
        abi::store_u64("%v9", abi::stack_pointer(), ROUNDS_OFF),
        abi::compare_immediate("%v9", "0"),
        abi::branch_le(&timeout_lbl),
        abi::label(&do_wait),
        // Wait a bounded slice on socket 0 (via the scalar helper) before rescanning.
        abi::move_immediate("%v9", "Integer", "0"),
    ]);
    load_elem(&mut ins, abi::return_register(), "%v9");
    ins.extend([
        abi::move_immediate(abi::c_arg(1), "Integer", SLICE_MS),
        abi::branch_link(SCALAR),
        abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG),
        abi::branch_ne(&done), // propagate a wait-time error
        abi::branch(&round_loop),
        abi::label(&found),
        // Return socks[i] (borrowed) — the list still owns/closes it.
        abi::load_u64("%v9", abi::stack_pointer(), I_OFF),
    ]);
    load_elem(&mut ins, RESULT_VALUE_REGISTER, "%v9");
    ins.extend([
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);
    rel.push(internal_branch(symbol, SCALAR));
    ins.push(abi::label(&invalid));
    emit_fail(symbol, "ErrInvalidArgument", &mut ins, &mut rel, &done);
    ins.push(abi::label(&timeout_lbl));
    emit_fail(symbol, "ErrTimeout", &mut ins, &mut rel, &done);
    ins.extend([abi::label(&done), abi::return_()]);
    let (frame, stack_slots) = finalize_vreg_body_with_locals(&mut ins, &[], FRAME_SIZE);
    Ok((frame, ins, rel, stack_slots))
}

pub(super) fn lower_tls_close_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    match platform.family() {
        PlatformFamily::MacOS => macos::lower_tls_close_macos(symbol, platform_imports, platform),
        PlatformFamily::Linux => {
            openssl::lower_tls_close_openssl(symbol, platform_imports, platform)
        }
        PlatformFamily::Windows => schannel::lower_tls_close(symbol, platform_imports, platform),
    }
}

pub(super) fn lower_tls_close_listener_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    match platform.family() {
        PlatformFamily::MacOS => {
            macos::lower_tls_close_listener_macos(symbol, platform_imports, platform)
        }
        PlatformFamily::Linux => {
            openssl::lower_tls_close_listener_openssl(symbol, platform_imports, platform)
        }
        PlatformFamily::Windows => {
            schannel::lower_tls_close_listener(symbol, platform_imports, platform)
        }
    }
}

// ===========================================================================
// macOS backend: Network.framework over a dispatch-semaphore synchronous bridge
// ===========================================================================

pub(crate) fn macos_tls_data_objects(server: bool) -> Vec<CodeDataObject> {
    macos::data_objects(server)
}
