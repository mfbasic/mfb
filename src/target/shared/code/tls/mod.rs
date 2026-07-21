//! Native code generation for the built-in `tls` package (transport-layer
//! security). The Linux backend drives the system OpenSSL via `dlopen`/`dlsym`
//! so one binary spans OpenSSL 1.1.1 and 3.x (plan-03-net.md §4). The macOS
//! backend (see the `macos` submodule) drives Network.framework through a
//! dispatch-semaphore synchronous bridge.
//!
//! On Linux a `TlsSocket` handle is a 32-byte arena record: `fd` at 0, a
//! `closed` flag at 8, the `SSL*` at 16, and the `SSL_CTX*` at 24. Each helper
//! re-`dlopen`s `libssl` (cheap once loaded — it just bumps the refcount) and
//! `dlsym`s the `SSL_*` symbols it needs; `dlsym` resolves the library's default
//! symbol version, which is why a single binary works against both OpenSSL
//! series. The macOS record layout differs and is documented in `macos`.

use std::collections::HashMap;

use super::*;
use crate::target::shared::abi;

// OpenSSL handles share this fixed record layout (distinct from the `File`
// layout used by `Socket`/`UdpSocket`). An accepted (server-side) `TlsSocket`
// stores 0 in the `SSL_CTX*` slot: the marker that it points at the listener's
// shared server context and must not free it (plan-06-tls-server.md §5.1).
pub(super) const TLS_OFFSET_FD: usize = 0;
pub(super) const TLS_OFFSET_CLOSED: usize = 8;
pub(super) const TLS_OFFSET_SSL: usize = 16;
pub(super) const TLS_OFFSET_CTX: usize = 24;
pub(super) const TLS_RECORD_SIZE: &str = "32";

// The `TlsListener` record (Linux/OpenSSL): the listening fd plus the server
// `SSL_CTX*` it owns (freed exactly once, when the listener closes). The
// fourth slot is reserved (plan-06-tls-server.md §5.1).
pub(super) const TLS_LISTENER_OFFSET_FD: usize = 0;
pub(super) const TLS_LISTENER_OFFSET_CLOSED: usize = 8;
pub(super) const TLS_LISTENER_OFFSET_CTX: usize = 16;

// Both OpenSSL records place the `closed` flag at the canonical resource
// closed-flag offset (plan-38), so the backend-independent closed-default sets
// exactly the byte these guards read. The macOS Network.framework backend
// carries its own `REC_CLOSED` assert in `macos.rs`.
const _: () = assert!(TLS_OFFSET_CLOSED == RESOURCE_OFFSET_CLOSED);
const _: () = assert!(TLS_LISTENER_OFFSET_CLOSED == RESOURCE_OFFSET_CLOSED);

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

pub(super) fn hex_encode_cstring(text: &str) -> String {
    let mut hex = String::new();
    for byte in text.bytes() {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex.push_str("00"); // NUL terminator
    hex
}

/// Read-only C-string data objects (library sonames + symbol names) referenced
/// by the TLS helpers. Emitted once when a module uses any `tls` call; the
/// server-only symbol names are appended only when a server helper
/// (`tls.listen`/`tls.accept`/`tls.closeListener`) is in the plan.
pub(super) fn tls_cstring_data_objects(server: bool) -> Vec<CodeDataObject> {
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

/// Load the address of a read-only data symbol into `dst` (adrp + add).
pub(super) fn emit_data_address(
    from: &str,
    dst: &str,
    data_symbol: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    instructions.push(
        CodeInstruction::new("adrp")
            .field("dst", dst)
            .field("symbol", data_symbol),
    );
    instructions.push(
        CodeInstruction::new("add_pageoff")
            .field("dst", dst)
            .field("src", dst)
            .field("symbol", data_symbol),
    );
    relocations.extend([
        CodeRelocation {
            from: from.to_string(),
            to: data_symbol.to_string(),
            kind: RelocIntent::DataAddrHi,
            binding: "data".to_string(),
            library: None,
        },
        CodeRelocation {
            from: from.to_string(),
            to: data_symbol.to_string(),
            kind: RelocIntent::DataAddrLo,
            binding: "data".to_string(),
            library: None,
        },
    ]);
}

/// `bl _mfb_arena_alloc` with size in `x0` and alignment in `x1`; block pointer
/// left in `x1`. Branches to `fail` on allocation failure.

/// `bl _mfb_arena_free` returning a single compiler-sized block to the arena.
/// The caller stages the block pointer in the return register (`x0`) and its
/// original allocation size in `ARG[1]` (`x1`).
pub(super) fn emit_arena_free(
    symbol: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    instructions.push(abi::branch_link(ARENA_FREE_SYMBOL));
    relocations.push(super::internal_branch(symbol, ARENA_FREE_SYMBOL));
}

pub(super) fn emit_fail(
    symbol: &str,
    code: &str,
    message_symbol: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    done: &str,
) {
    instructions.extend([
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", code),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    push_error_message_address(symbol, message_symbol, instructions, relocations);
    instructions.push(abi::branch(done));
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
        abi::move_immediate(abi::ARG[1], "Integer", "1"),
    ]);
    emit_alloc(symbol, instructions, relocations, alloc_fail);
    instructions.extend([
        abi::store_u64(abi::RET[1], abi::stack_pointer(), out_off),
        abi::load_u64("%v17", abi::stack_pointer(), str_off),
        abi::load_u64("%v18", "%v17", 0),
        abi::add_immediate("%v19", "%v17", 8),
        abi::move_register("%v20", abi::RET[1]),
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
        abi::ARG[1],
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

    for opt in [platform.so_rcvtimeo(), platform.so_sndtimeo()] {
        ctx.instructions.extend([
            abi::load_u64(abi::return_register(), abi::stack_pointer(), fd_off),
            abi::move_immediate(abi::ARG[1], "Integer", platform.sol_socket()),
            abi::move_immediate(abi::ARG[2], "Integer", opt),
            abi::add_immediate(abi::ARG[3], abi::stack_pointer(), tv_off),
            abi::move_immediate(abi::ARG[4], "Integer", "16"),
        ]);
        platform.emit_libc_call(
            "setsockopt",
            symbol,
            platform_imports,
            ctx.instructions,
            ctx.relocations,
        )?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// tls.connect
// ---------------------------------------------------------------------------

pub(crate) mod macos;
mod openssl;

pub(super) use openssl::{
    lower_tls_accept_helper, lower_tls_close_helper, lower_tls_close_listener_helper,
    lower_tls_connect_helper, lower_tls_listen_helper, lower_tls_read_helper,
    lower_tls_write_helper,
};

// ===========================================================================
// macOS backend: Network.framework over a dispatch-semaphore synchronous bridge
// ===========================================================================

pub(super) fn macos_tls_data_objects(server: bool) -> Vec<CodeDataObject> {
    macos::data_objects(server)
}
