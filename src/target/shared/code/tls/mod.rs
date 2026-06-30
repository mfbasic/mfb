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
use crate::arch::aarch64::abi;

// OpenSSL handles share this fixed record layout (distinct from the `File`
// layout used by `Socket`/`UdpSocket`).
pub(super) const TLS_OFFSET_FD: usize = 0;
pub(super) const TLS_OFFSET_CLOSED: usize = 8;
pub(super) const TLS_OFFSET_SSL: usize = 16;
pub(super) const TLS_OFFSET_CTX: usize = 24;
pub(super) const TLS_RECORD_SIZE: &str = "32";

pub(super) const SOCK_STREAM: &str = "1";
pub(super) const HINTS_FAMILY_WORD: &str = "8589934592"; // ai_family = AF_INET (2 << 32), ai_flags = 0
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

/// Every OpenSSL symbol the helpers `dlsym`. Each gets a read-only C-string data
/// object so the load can name it.
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
/// by the TLS helpers. Emitted once when a module uses any `tls` call.
pub(super) fn tls_cstring_data_objects() -> Vec<CodeDataObject> {
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
    for name in TLS_SYMBOLS {
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

fn internal_reloc(symbol: &str, target: &str) -> CodeRelocation {
    CodeRelocation {
        from: symbol.to_string(),
        to: target.to_string(),
        kind: RelocIntent::Call,
        binding: "internal".to_string(),
        library: None,
    }
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
pub(super) fn emit_alloc(
    symbol: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    fail: &str,
) {
    instructions.push(abi::branch_link(ARENA_ALLOC_SYMBOL));
    relocations.push(internal_reloc(symbol, ARENA_ALLOC_SYMBOL));
    instructions.extend([
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_ne(fail),
    ]);
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
        abi::load_u64("x9", abi::stack_pointer(), str_off),
        abi::load_u64("x10", "x9", 0),
        abi::add_immediate(abi::return_register(), "x10", 1),
        abi::move_immediate("x1", "Integer", "1"),
    ]);
    emit_alloc(symbol, instructions, relocations, alloc_fail);
    instructions.extend([
        abi::store_u64("x1", abi::stack_pointer(), out_off),
        abi::load_u64("x9", abi::stack_pointer(), str_off),
        abi::load_u64("x10", "x9", 0),
        abi::add_immediate("x11", "x9", 8),
        abi::move_register("x12", "x1"),
        abi::move_immediate("x13", "Integer", "0"),
        abi::label(&copy_loop),
        abi::compare_registers("x13", "x10"),
        abi::branch_eq(&copy_done),
        abi::load_u8("x14", "x11", 0),
        abi::store_u8("x14", "x12", 0),
        abi::add_immediate("x11", "x11", 1),
        abi::add_immediate("x12", "x12", 1),
        abi::add_immediate("x13", "x13", 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::store_u8("x31", "x12", 0),
    ]);
}

/// `dlopen` `libssl.so.3`, falling back to `libssl.so.1.1`; the handle is stored
/// at `sp + handle_off`. Branches to `fail` when neither loads.
pub(super) fn emit_dlopen_libssl(
    symbol: &str,
    handle_off: usize,
    fail: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    let loaded = format!("{symbol}_dlopen_done");
    emit_data_address(
        symbol,
        abi::return_register(),
        &lib_data_symbol(0),
        instructions,
        relocations,
    );
    instructions.push(abi::move_immediate("x1", "Integer", RTLD_NOW));
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
        abi::branch_ne(&loaded),
    ]);
    emit_data_address(
        symbol,
        abi::return_register(),
        &lib_data_symbol(1),
        instructions,
        relocations,
    );
    instructions.push(abi::move_immediate("x1", "Integer", RTLD_NOW));
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
        abi::label(&loaded),
    ]);
    Ok(())
}

/// `dlsym(handle, name)` into `sp + fnptr_off`. Branches to `fail` if missing.
pub(super) fn emit_dlsym(
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
    instructions.push(abi::load_u64(
        abi::return_register(),
        abi::stack_pointer(),
        handle_off,
    ));
    emit_data_address(
        symbol,
        "x1",
        &sym_data_symbol(name),
        instructions,
        relocations,
    );
    platform.emit_libc_call("dlsym", symbol, platform_imports, instructions, relocations)?;
    instructions.extend([
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
    symbol: &str,
    fd_off: usize,
    tv_off: usize,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    for opt in [platform.so_rcvtimeo(), platform.so_sndtimeo()] {
        instructions.extend([
            abi::load_u64(abi::return_register(), abi::stack_pointer(), fd_off),
            abi::move_immediate("x1", "Integer", platform.sol_socket()),
            abi::move_immediate("x2", "Integer", opt),
            abi::add_immediate("x3", abi::stack_pointer(), tv_off),
            abi::move_immediate("x4", "Integer", "16"),
        ]);
        platform.emit_libc_call(
            "setsockopt",
            symbol,
            platform_imports,
            instructions,
            relocations,
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
    lower_tls_close_helper, lower_tls_connect_helper, lower_tls_read_helper,
    lower_tls_write_helper,
};

// ===========================================================================
// macOS backend: Network.framework over a dispatch-semaphore synchronous bridge
// ===========================================================================

pub(super) fn macos_tls_data_objects() -> Vec<CodeDataObject> {
    macos::data_objects()
}
