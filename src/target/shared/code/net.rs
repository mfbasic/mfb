//! Native code generation for the built-in `net` package runtime helpers (DNS
//! lookup and TCP sockets). Each `lower_net_*_helper` emits a self-contained
//! AArch64 runtime function that marshals libc socket calls and returns the
//! standard `(tag, value)` result in `x0`/`x1`.
//!
//! Socket and listener handles share the `File` record layout (`fd` at offset
//! 0, a `closed` flag at offset 8). Platform `sockaddr` structures are produced
//! by `getaddrinfo` so the helpers never hand-build a `sockaddr_in`; the only
//! field written directly is `sin_port` at offset 2, which is consistent across
//! platforms for `AF_INET`.

use std::collections::HashMap;

use super::*;
use crate::arch::aarch64::abi;

const AF_INET: &str = "2";
const SOCK_STREAM: &str = "1";
// hints `u64` at offset 0 packs `ai_flags` (low 32) and `ai_family` (high 32).
// `AF_INET (2) << 32`.
const HINTS_FAMILY_WORD: &str = "8589934592"; // ai_flags = 0
const HINTS_FAMILY_WORD_PASSIVE: &str = "8589934593"; // ai_flags = AI_PASSIVE (1)
const SOCKADDR_STORAGE_SIZE: usize = 128;
const ADDR_STR_CAP: usize = 64;
const POLLIN: &str = "1";

fn internal_reloc(symbol: &str, target: &str) -> CodeRelocation {
    CodeRelocation {
        from: symbol.to_string(),
        to: target.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    }
}

/// Emit `bl _mfb_arena_alloc` with the size in `x0` and alignment in `x1`
/// (preset by the caller), then branch to `fail` when allocation fails. On
/// success the block pointer is left in `x1` (`RESULT_VALUE_REGISTER`).
fn emit_alloc(
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

/// Set the result registers to a failure `(code, message)` and branch to
/// `done`.
fn emit_fail(
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
/// `sp + out_off`. Branches to `alloc_fail` on allocation failure. Clobbers
/// `x0`, `x1`, `x9`..`x14`.
fn emit_cstring(
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

/// Zero a 48-byte `addrinfo` hints block at `sp + hints_off` and set
/// `ai_family = AF_INET`, `ai_socktype = SOCK_STREAM` (and `AI_PASSIVE` when
/// `passive`). Clobbers `x9`.
fn emit_hints(
    hints_off: usize,
    passive: bool,
    instructions: &mut Vec<CodeInstruction>,
) {
    for offset in (0..48).step_by(8) {
        instructions.push(abi::store_u64("x31", abi::stack_pointer(), hints_off + offset));
    }
    let family_word = if passive {
        HINTS_FAMILY_WORD_PASSIVE
    } else {
        HINTS_FAMILY_WORD
    };
    instructions.extend([
        abi::move_immediate("x9", "Integer", family_word),
        abi::store_u64("x9", abi::stack_pointer(), hints_off),
        abi::move_immediate("x9", "Integer", SOCK_STREAM),
        abi::store_u64("x9", abi::stack_pointer(), hints_off + 8),
    ]);
}

/// Build an `Address` record from a `sockaddr` whose pointer lives at
/// `sp + sockaddr_off`. The observed port is read from `sockaddr + 2/3`.
/// `len_off`, `dst_off`, and `host_off` are scratch stack slots. Leaves the
/// `Address` pointer in `x1`, branches to `alloc_fail` on allocation failure or
/// `addr_fail` when `inet_ntop` fails. Everything persists on the stack so no
/// callee-saved registers are clobbered.
#[allow(clippy::too_many_arguments)]
fn emit_address_from_sockaddr(
    symbol: &str,
    prefix: &str,
    sockaddr_off: usize,
    len_off: usize,
    dst_off: usize,
    host_off: usize,
    platform: &dyn CodegenPlatform,
    platform_imports: &HashMap<String, String>,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    alloc_fail: &str,
    addr_fail: &str,
) -> Result<(), String> {
    let count_loop = format!("{symbol}_{prefix}_addr_count");
    let count_done = format!("{symbol}_{prefix}_addr_count_done");
    let copy_loop = format!("{symbol}_{prefix}_addr_copy");
    let copy_done = format!("{symbol}_{prefix}_addr_copy_done");
    // Temp dst buffer for the numeric host string.
    instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", &ADDR_STR_CAP.to_string()),
        abi::move_immediate("x1", "Integer", "1"),
    ]);
    emit_alloc(symbol, instructions, relocations, alloc_fail);
    instructions.extend([
        abi::store_u64("x1", abi::stack_pointer(), dst_off),
        // inet_ntop(AF_INET, sockaddr + 4, dst, ADDR_STR_CAP)
        abi::move_immediate(abi::return_register(), "Integer", AF_INET),
        abi::load_u64("x9", abi::stack_pointer(), sockaddr_off),
        abi::add_immediate("x1", "x9", 4),
        abi::load_u64("x2", abi::stack_pointer(), dst_off),
        abi::move_immediate("x3", "Integer", &ADDR_STR_CAP.to_string()),
    ]);
    platform.emit_libc_call("inet_ntop", symbol, platform_imports, instructions, relocations)?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(addr_fail),
        // Count the NUL-terminated host string length.
        abi::load_u64("x9", abi::stack_pointer(), dst_off),
        abi::move_immediate("x10", "Integer", "0"),
        abi::label(&count_loop),
        abi::load_u8("x11", "x9", 0),
        abi::compare_immediate("x11", "0"),
        abi::branch_eq(&count_done),
        abi::add_immediate("x9", "x9", 1),
        abi::add_immediate("x10", "x10", 1),
        abi::branch(&count_loop),
        abi::label(&count_done),
        abi::store_u64("x10", abi::stack_pointer(), len_off),
        // Allocate the host String: [u64 len][bytes][nul].
        abi::add_immediate(abi::return_register(), "x10", 9),
        abi::move_immediate("x1", "Integer", "8"),
    ]);
    emit_alloc(symbol, instructions, relocations, alloc_fail);
    instructions.extend([
        abi::load_u64("x10", abi::stack_pointer(), len_off),
        abi::store_u64("x10", "x1", 0),
        abi::store_u64("x1", abi::stack_pointer(), host_off),
        abi::load_u64("x11", abi::stack_pointer(), dst_off),
        abi::add_immediate("x12", "x1", 8),
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
        // Allocate the Address record: [host ptr][port].
        abi::move_immediate(abi::return_register(), "Integer", "16"),
        abi::move_immediate("x1", "Integer", "8"),
    ]);
    emit_alloc(symbol, instructions, relocations, alloc_fail);
    instructions.extend([
        abi::load_u64("x9", abi::stack_pointer(), host_off),
        abi::store_u64("x9", "x1", 0),
        // port = (sockaddr[2] << 8) | sockaddr[3]
        abi::load_u64("x9", abi::stack_pointer(), sockaddr_off),
        abi::load_u8("x10", "x9", 2),
        abi::load_u8("x11", "x9", 3),
        abi::shift_left_immediate("x10", "x10", 8),
        abi::or_registers("x10", "x10", "x11"),
        abi::store_u64("x10", "x1", 8),
    ]);
    Ok(())
}

/// Allocate a 16-byte socket/listener handle record from the file descriptor in
/// `x9`, leaving the record pointer in `x1`. Branches to `alloc_fail` on
/// failure.
fn emit_make_handle(
    symbol: &str,
    fd_off: usize,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    alloc_fail: &str,
) {
    instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", "16"),
        abi::move_immediate("x1", "Integer", "8"),
    ]);
    emit_alloc(symbol, instructions, relocations, alloc_fail);
    instructions.extend([
        abi::load_u64("x9", abi::stack_pointer(), fd_off),
        abi::store_u64("x9", "x1", FILE_OFFSET_FD),
        abi::store_u64("x31", "x1", FILE_OFFSET_CLOSED),
    ]);
}

fn frame(stack_size: usize) -> CodeFrame {
    CodeFrame {
        stack_size,
        callee_saved: vec![abi::link_register().to_string()],
    }
}

// ---------------------------------------------------------------------------
// net.connectTcp / net.listenTcp
// ---------------------------------------------------------------------------

/// Shared lowering for `connectTcp` and `listenTcp`: both resolve the host with
/// `getaddrinfo`, create a socket, and then either `connect` or `bind`+`listen`.
fn lower_net_endpoint_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    listen: bool,
    address: bool,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 192;
    const LR_OFFSET: usize = 0;
    const HOST_OFFSET: usize = 8;
    const PORT_OFFSET: usize = 16;
    const EXTRA_OFFSET: usize = 24; // timeoutMs (connect) or backlog (listen)
    const RES_OFFSET: usize = 32;
    const FD_OFFSET: usize = 40;
    const CSTR_OFFSET: usize = 48;
    const ONE_OFFSET: usize = 56;
    const HINTS_OFFSET: usize = 64; // 64..112
    const FLAGS_OFFSET: usize = 112; // saved socket flags for non-blocking connect
    const POLLFD_OFFSET: usize = 120; // pollfd { fd; events; revents }
    const SOERR_OFFSET: usize = 128; // getsockopt SO_ERROR output
    const SOLEN_OFFSET: usize = 136; // getsockopt option length

    let null_host = format!("{symbol}_null_host");
    let resolved = format!("{symbol}_resolved");
    let resolve_fail = format!("{symbol}_resolve_fail");
    let socket_fail = format!("{symbol}_socket_fail");
    let op_fail = format!("{symbol}_op_fail");
    let blocking_connect = format!("{symbol}_blocking_connect");
    let nb_connected = format!("{symbol}_nb_connected");
    let connect_timeout = format!("{symbol}_connect_timeout");
    let connected_done = format!("{symbol}_connected_done");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    instructions.push(abi::store_u64(
        abi::link_register(),
        abi::stack_pointer(),
        LR_OFFSET,
    ));
    if address {
        // x0 = Address record { host String ptr @0, port @8 }; x1 = timeoutMs.
        instructions.extend([
            abi::load_u64("x9", abi::return_register(), 0),
            abi::store_u64("x9", abi::stack_pointer(), HOST_OFFSET),
            abi::load_u64("x9", abi::return_register(), 8),
            abi::store_u64("x9", abi::stack_pointer(), PORT_OFFSET),
            abi::store_u64("x1", abi::stack_pointer(), EXTRA_OFFSET),
        ]);
    } else {
        instructions.extend([
            abi::store_u64(abi::return_register(), abi::stack_pointer(), HOST_OFFSET),
            abi::store_u64("x1", abi::stack_pointer(), PORT_OFFSET),
            abi::store_u64("x2", abi::stack_pointer(), EXTRA_OFFSET),
        ]);
    }
    emit_hints(HINTS_OFFSET, listen, &mut instructions);
    // Choose host C string. An empty host on a listener binds all interfaces
    // (NULL host + AI_PASSIVE).
    instructions.extend([
        abi::load_u64("x9", abi::stack_pointer(), HOST_OFFSET),
        abi::load_u64("x9", "x9", 0),
        abi::compare_immediate("x9", "0"),
    ]);
    if listen {
        instructions.push(abi::branch_eq(&null_host));
    }
    emit_cstring(
        symbol,
        "host",
        HOST_OFFSET,
        CSTR_OFFSET,
        &alloc_fail,
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::branch(&resolved));
    if listen {
        instructions.extend([
            abi::label(&null_host),
            abi::store_u64("x31", abi::stack_pointer(), CSTR_OFFSET),
        ]);
    }
    instructions.extend([
        abi::label(&resolved),
        // getaddrinfo(host, NULL, &hints, &res)
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CSTR_OFFSET),
        abi::move_immediate("x1", "Integer", "0"),
        abi::add_immediate("x2", abi::stack_pointer(), HINTS_OFFSET),
        abi::add_immediate("x3", abi::stack_pointer(), RES_OFFSET),
    ]);
    platform.emit_libc_call(
        "getaddrinfo",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&resolve_fail),
        // socket(ai_family, ai_socktype, ai_protocol)
        abi::load_u64("x9", abi::stack_pointer(), RES_OFFSET),
        abi::load_u32(abi::return_register(), "x9", 4),
        abi::load_u32("x1", "x9", 8),
        abi::load_u32("x2", "x9", 12),
    ]);
    platform.emit_libc_call("socket", symbol, platform_imports, &mut instructions, &mut relocations)?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&socket_fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        // Overwrite sin_port at ai_addr + 2/3 with the requested port (network
        // byte order).
        abi::load_u64("x9", abi::stack_pointer(), RES_OFFSET),
        abi::load_u64("x9", "x9", platform.addrinfo_addr_offset()),
        abi::load_u64("x10", abi::stack_pointer(), PORT_OFFSET),
        abi::shift_right_immediate("x11", "x10", 8),
        abi::store_u8("x11", "x9", 2),
        abi::store_u8("x10", "x9", 3),
    ]);
    if listen {
        // setsockopt(fd, SOL_SOCKET, SO_REUSEADDR, &one, 4) - best effort.
        instructions.extend([
            abi::move_immediate("x9", "Integer", "1"),
            abi::store_u64("x9", abi::stack_pointer(), ONE_OFFSET),
            abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
            abi::move_immediate("x1", "Integer", platform.sol_socket()),
            abi::move_immediate("x2", "Integer", platform.so_reuseaddr()),
            abi::add_immediate("x3", abi::stack_pointer(), ONE_OFFSET),
            abi::move_immediate("x4", "Integer", "4"),
        ]);
        platform.emit_libc_call(
            "setsockopt",
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        // bind(fd, ai_addr, ai_addrlen)
        instructions.extend([
            abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
            abi::load_u64("x9", abi::stack_pointer(), RES_OFFSET),
            abi::load_u64("x1", "x9", platform.addrinfo_addr_offset()),
            abi::load_u32("x2", "x9", 16),
        ]);
        platform.emit_libc_call("bind", symbol, platform_imports, &mut instructions, &mut relocations)?;
        instructions.extend([
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_lt(&op_fail),
            // listen(fd, backlog)
            abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
            abi::load_u64("x1", abi::stack_pointer(), EXTRA_OFFSET),
        ]);
        platform.emit_libc_call("listen", symbol, platform_imports, &mut instructions, &mut relocations)?;
        instructions.extend([
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_lt(&op_fail),
        ]);
    } else {
        // `timeoutMs <= 0` uses the implementation default: a plain blocking
        // connect. `timeoutMs > 0` performs a non-blocking connect bounded by a
        // `poll`, then restores blocking mode.
        instructions.extend([
            abi::load_u64("x9", abi::stack_pointer(), EXTRA_OFFSET),
            abi::compare_immediate("x9", "0"),
            abi::branch_le(&blocking_connect),
            // flags = fcntl(fd, F_GETFL, 0)
            abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
            abi::move_immediate("x1", "Integer", "3"),
            abi::move_immediate("x2", "Integer", "0"),
        ]);
        platform.emit_variadic_call("fcntl", symbol, platform_imports, &mut instructions, &mut relocations)?;
        instructions.extend([
            abi::store_u64(abi::return_register(), abi::stack_pointer(), FLAGS_OFFSET),
            // fcntl(fd, F_SETFL, flags | O_NONBLOCK)
            abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
            abi::move_immediate("x1", "Integer", "4"),
            abi::load_u64("x2", abi::stack_pointer(), FLAGS_OFFSET),
            abi::move_immediate("x9", "Integer", platform.o_nonblock()),
            abi::or_registers("x2", "x2", "x9"),
        ]);
        platform.emit_variadic_call("fcntl", symbol, platform_imports, &mut instructions, &mut relocations)?;
        // connect(fd, ai_addr, ai_addrlen)
        instructions.extend([
            abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
            abi::load_u64("x9", abi::stack_pointer(), RES_OFFSET),
            abi::load_u64("x1", "x9", platform.addrinfo_addr_offset()),
            abi::load_u32("x2", "x9", 16),
        ]);
        platform.emit_libc_call("connect", symbol, platform_imports, &mut instructions, &mut relocations)?;
        instructions.extend([
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_eq(&nb_connected),
        ]);
        // In progress? Anything other than EINPROGRESS is a hard failure.
        platform.emit_errno(symbol, platform_imports, &mut instructions, &mut relocations)?;
        instructions.extend([
            abi::compare_immediate("x9", platform.einprogress()),
            abi::branch_ne(&op_fail),
            // poll(&pollfd { fd, POLLOUT }, 1, timeoutMs)
            abi::load_u64("x9", abi::stack_pointer(), FD_OFFSET),
            abi::store_u64("x9", abi::stack_pointer(), POLLFD_OFFSET),
            abi::move_immediate("x10", "Integer", "4"), // POLLOUT
            abi::store_u8("x10", abi::stack_pointer(), POLLFD_OFFSET + 4),
            abi::store_u8("x31", abi::stack_pointer(), POLLFD_OFFSET + 5),
            abi::store_u8("x31", abi::stack_pointer(), POLLFD_OFFSET + 6),
            abi::store_u8("x31", abi::stack_pointer(), POLLFD_OFFSET + 7),
            abi::add_immediate(abi::return_register(), abi::stack_pointer(), POLLFD_OFFSET),
            abi::move_immediate("x1", "Integer", "1"),
            abi::load_u64("x2", abi::stack_pointer(), EXTRA_OFFSET),
        ]);
        platform.emit_libc_call("poll", symbol, platform_imports, &mut instructions, &mut relocations)?;
        instructions.extend([
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_lt(&op_fail),
            abi::branch_eq(&connect_timeout),
            // getsockopt(fd, SOL_SOCKET, SO_ERROR, &err, &len)
            abi::move_immediate("x9", "Integer", "4"),
            abi::store_u64("x9", abi::stack_pointer(), SOLEN_OFFSET),
            abi::store_u64("x31", abi::stack_pointer(), SOERR_OFFSET),
            abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
            abi::move_immediate("x1", "Integer", platform.sol_socket()),
            abi::move_immediate("x2", "Integer", platform.so_error()),
            abi::add_immediate("x3", abi::stack_pointer(), SOERR_OFFSET),
            abi::add_immediate("x4", abi::stack_pointer(), SOLEN_OFFSET),
        ]);
        platform.emit_libc_call("getsockopt", symbol, platform_imports, &mut instructions, &mut relocations)?;
        instructions.extend([
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_lt(&op_fail),
            abi::load_u32("x9", abi::stack_pointer(), SOERR_OFFSET),
            abi::compare_immediate("x9", "0"),
            abi::branch_ne(&op_fail),
            // Connected: restore blocking mode with fcntl(fd, F_SETFL, flags).
            abi::label(&nb_connected),
            abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
            abi::move_immediate("x1", "Integer", "4"),
            abi::load_u64("x2", abi::stack_pointer(), FLAGS_OFFSET),
        ]);
        platform.emit_variadic_call("fcntl", symbol, platform_imports, &mut instructions, &mut relocations)?;
        instructions.extend([
            abi::branch(&connected_done),
            // Blocking connect path (timeoutMs <= 0).
            abi::label(&blocking_connect),
            abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
            abi::load_u64("x9", abi::stack_pointer(), RES_OFFSET),
            abi::load_u64("x1", "x9", platform.addrinfo_addr_offset()),
            abi::load_u32("x2", "x9", 16),
        ]);
        platform.emit_libc_call("connect", symbol, platform_imports, &mut instructions, &mut relocations)?;
        instructions.extend([
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_lt(&op_fail),
            abi::label(&connected_done),
        ]);
    }
    // freeaddrinfo(res)
    instructions.push(abi::load_u64(abi::return_register(), abi::stack_pointer(), RES_OFFSET));
    platform.emit_libc_call(
        "freeaddrinfo",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    emit_make_handle(symbol, FD_OFFSET, &mut instructions, &mut relocations, &alloc_fail);
    instructions.extend([
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);
    // op_fail / socket_fail: free resources then report network failure. The
    // socket fd (if any) leaks on these rare error paths; the process-level
    // failure is surfaced to the caller as a network error.
    instructions.push(abi::label(&op_fail));
    instructions.push(abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET));
    platform.emit_libc_call("close", symbol, platform_imports, &mut instructions, &mut relocations)?;
    instructions.push(abi::label(&socket_fail));
    instructions.push(abi::load_u64(abi::return_register(), abi::stack_pointer(), RES_OFFSET));
    platform.emit_libc_call(
        "freeaddrinfo",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    emit_fail(
        symbol,
        ERR_NETWORK_FAILED_CODE,
        ERR_NETWORK_FAILED_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    // A connect that did not complete before its deadline: close the pending
    // socket, release the resolver results, and report a timeout.
    instructions.push(abi::label(&connect_timeout));
    instructions.push(abi::load_u64(
        abi::return_register(),
        abi::stack_pointer(),
        FD_OFFSET,
    ));
    platform.emit_libc_call("close", symbol, platform_imports, &mut instructions, &mut relocations)?;
    instructions.push(abi::load_u64(
        abi::return_register(),
        abi::stack_pointer(),
        RES_OFFSET,
    ));
    platform.emit_libc_call(
        "freeaddrinfo",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    emit_fail(
        symbol,
        ERR_TIMEOUT_CODE,
        ERR_TIMEOUT_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&resolve_fail));
    if listen {
        emit_fail(
            symbol,
            ERR_ADDRESS_INVALID_CODE,
            ERR_ADDRESS_INVALID_SYMBOL,
            &mut instructions,
            &mut relocations,
            &done,
        );
    } else {
        emit_fail(
            symbol,
            ERR_ADDRESS_NOT_FOUND_CODE,
            ERR_ADDRESS_NOT_FOUND_SYMBOL,
            &mut instructions,
            &mut relocations,
            &done,
        );
    }
    instructions.push(abi::label(&alloc_fail));
    emit_fail(
        symbol,
        ERR_OUT_OF_MEMORY_CODE,
        ERR_ALLOCATION_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([
        abi::label(&done),
        abi::load_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::add_stack(FRAME_SIZE),
        abi::return_(),
    ]);
    Ok((frame(FRAME_SIZE), instructions, relocations))
}

pub(super) fn lower_net_connect_tcp_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    lower_net_endpoint_helper(symbol, platform_imports, platform, false, false)
}

pub(super) fn lower_net_connect_tcp_addr_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    lower_net_endpoint_helper(symbol, platform_imports, platform, false, true)
}

pub(super) fn lower_net_listen_tcp_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    lower_net_endpoint_helper(symbol, platform_imports, platform, true, false)
}

// ---------------------------------------------------------------------------
// net.accept
// ---------------------------------------------------------------------------

pub(super) fn lower_net_accept_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 64;
    const LR_OFFSET: usize = 0;
    const FD_OFFSET: usize = 8;
    const TIMEOUT_OFFSET: usize = 16;

    let closed = format!("{symbol}_closed");
    let accept_fail = format!("{symbol}_accept_fail");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::store_u64("x1", abi::stack_pointer(), TIMEOUT_OFFSET),
        abi::load_u64("x9", abi::return_register(), FILE_OFFSET_CLOSED),
        abi::compare_immediate("x9", "0"),
        abi::branch_ne(&closed),
        abi::load_u64("x9", abi::return_register(), FILE_OFFSET_FD),
        abi::store_u64("x9", abi::stack_pointer(), FD_OFFSET),
        // accept(fd, NULL, NULL)
        abi::move_register(abi::return_register(), "x9"),
        abi::move_immediate("x1", "Integer", "0"),
        abi::move_immediate("x2", "Integer", "0"),
    ]);
    platform.emit_libc_call("accept", symbol, platform_imports, &mut instructions, &mut relocations)?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&accept_fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
    ]);
    emit_make_handle(symbol, FD_OFFSET, &mut instructions, &mut relocations, &alloc_fail);
    instructions.extend([
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&accept_fail),
    ]);
    emit_fail(
        symbol,
        ERR_NETWORK_FAILED_CODE,
        ERR_NETWORK_FAILED_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&closed));
    emit_fail(
        symbol,
        ERR_RESOURCE_CLOSED_CODE,
        ERR_RESOURCE_CLOSED_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&alloc_fail));
    emit_fail(
        symbol,
        ERR_OUT_OF_MEMORY_CODE,
        ERR_ALLOCATION_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([
        abi::label(&done),
        abi::load_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::add_stack(FRAME_SIZE),
        abi::return_(),
    ]);
    Ok((frame(FRAME_SIZE), instructions, relocations))
}

// ---------------------------------------------------------------------------
// net.localAddress / net.remoteAddress
// ---------------------------------------------------------------------------

pub(super) fn lower_net_address_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    remote: bool,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 224;
    const LR_OFFSET: usize = 0;
    const FD_OFFSET: usize = 8;
    const LEN_OFFSET: usize = 16;
    const DST_OFFSET: usize = 24;
    const HOST_OFFSET: usize = 32;
    const SADDR_PTR_OFFSET: usize = 40;
    const HOSTLEN_OFFSET: usize = 48;
    const ADDR_OFFSET: usize = 64; // 64..192 sockaddr_storage

    let closed = format!("{symbol}_closed");
    let name_fail = format!("{symbol}_name_fail");
    let addr_fail = format!("{symbol}_addr_fail");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::load_u64("x9", abi::return_register(), FILE_OFFSET_CLOSED),
        abi::compare_immediate("x9", "0"),
        abi::branch_ne(&closed),
        abi::load_u64("x9", abi::return_register(), FILE_OFFSET_FD),
        abi::store_u64("x9", abi::stack_pointer(), FD_OFFSET),
        abi::move_immediate("x10", "Integer", &SOCKADDR_STORAGE_SIZE.to_string()),
        abi::store_u64("x10", abi::stack_pointer(), LEN_OFFSET),
        abi::move_register(abi::return_register(), "x9"),
        abi::add_immediate("x1", abi::stack_pointer(), ADDR_OFFSET),
        abi::add_immediate("x2", abi::stack_pointer(), LEN_OFFSET),
    ]);
    let call = if remote { "getpeername" } else { "getsockname" };
    platform.emit_libc_call(call, symbol, platform_imports, &mut instructions, &mut relocations)?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&name_fail),
        abi::add_immediate("x9", abi::stack_pointer(), ADDR_OFFSET),
        abi::store_u64("x9", abi::stack_pointer(), SADDR_PTR_OFFSET),
    ]);
    emit_address_from_sockaddr(
        symbol,
        "addr",
        SADDR_PTR_OFFSET,
        HOSTLEN_OFFSET,
        DST_OFFSET,
        HOST_OFFSET,
        platform,
        platform_imports,
        &mut instructions,
        &mut relocations,
        &alloc_fail,
        &addr_fail,
    )?;
    instructions.extend([
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&name_fail),
    ]);
    emit_fail(
        symbol,
        ERR_RESOURCE_CLOSED_CODE,
        ERR_RESOURCE_CLOSED_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&addr_fail));
    emit_fail(
        symbol,
        ERR_ADDRESS_INVALID_CODE,
        ERR_ADDRESS_INVALID_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&closed));
    emit_fail(
        symbol,
        ERR_RESOURCE_CLOSED_CODE,
        ERR_RESOURCE_CLOSED_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&alloc_fail));
    emit_fail(
        symbol,
        ERR_OUT_OF_MEMORY_CODE,
        ERR_ALLOCATION_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([
        abi::label(&done),
        abi::load_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::add_stack(FRAME_SIZE),
        abi::return_(),
    ]);
    Ok((frame(FRAME_SIZE), instructions, relocations))
}

// ---------------------------------------------------------------------------
// net.read / net.readText
// ---------------------------------------------------------------------------

pub(super) fn lower_net_read_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    text: bool,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 96;
    const LR_OFFSET: usize = 0;
    const FD_OFFSET: usize = 8;
    const MAX_OFFSET: usize = 16;
    const BUF_OFFSET: usize = 24;
    const N_OFFSET: usize = 32;
    const STR_OFFSET: usize = 40;

    let closed = format!("{symbol}_closed");
    let invalid = format!("{symbol}_invalid");
    let peer_closed = format!("{symbol}_peer_closed");
    let read_fail = format!("{symbol}_read_fail");
    let timeout = format!("{symbol}_timeout");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let encoding_error = format!("{symbol}_encoding_error");
    let build_list = format!("{symbol}_build_list");
    let entry_loop = format!("{symbol}_entry_loop");
    let entry_done = format!("{symbol}_entry_done");
    let str_copy = format!("{symbol}_str_copy");
    let str_done = format!("{symbol}_str_done");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::store_u64("x1", abi::stack_pointer(), MAX_OFFSET),
        abi::load_u64("x9", abi::return_register(), FILE_OFFSET_CLOSED),
        abi::compare_immediate("x9", "0"),
        abi::branch_ne(&closed),
        abi::load_u64("x9", abi::return_register(), FILE_OFFSET_FD),
        abi::store_u64("x9", abi::stack_pointer(), FD_OFFSET),
        abi::load_u64("x10", abi::stack_pointer(), MAX_OFFSET),
        abi::compare_immediate("x10", "0"),
        abi::branch_le(&invalid),
        // Allocate a temporary read buffer of maxBytes.
        abi::move_register(abi::return_register(), "x10"),
        abi::move_immediate("x1", "Integer", "1"),
    ]);
    emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.extend([
        abi::store_u64("x1", abi::stack_pointer(), BUF_OFFSET),
        // read(fd, buf, maxBytes)
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::load_u64("x1", abi::stack_pointer(), BUF_OFFSET),
        abi::load_u64("x2", abi::stack_pointer(), MAX_OFFSET),
    ]);
    platform.emit_read_file(symbol, platform_imports, &mut instructions, &mut relocations)?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(&peer_closed),
        abi::branch_lt(&read_fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), N_OFFSET),
    ]);
    if text {
        // Build a String: [u64 len][bytes][nul], validate UTF-8.
        instructions.extend([
            abi::load_u64("x10", abi::stack_pointer(), N_OFFSET),
            abi::add_immediate(abi::return_register(), "x10", 9),
            abi::move_immediate("x1", "Integer", "8"),
        ]);
        emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
        instructions.extend([
            abi::load_u64("x10", abi::stack_pointer(), N_OFFSET),
            abi::store_u64("x10", "x1", 0),
            abi::load_u64("x11", abi::stack_pointer(), BUF_OFFSET),
            abi::add_immediate("x12", "x1", 8),
            abi::move_immediate("x13", "Integer", "0"),
            abi::store_u64("x1", abi::stack_pointer(), STR_OFFSET),
            abi::label(&str_copy),
            abi::compare_registers("x13", "x10"),
            abi::branch_eq(&str_done),
            abi::load_u8("x14", "x11", 0),
            abi::store_u8("x14", "x12", 0),
            abi::add_immediate("x11", "x11", 1),
            abi::add_immediate("x12", "x12", 1),
            abi::add_immediate("x13", "x13", 1),
            abi::branch(&str_copy),
            abi::label(&str_done),
            abi::store_u8("x31", "x12", 0),
            // validate_utf8(bytes, len)
            abi::load_u64("x9", abi::stack_pointer(), STR_OFFSET),
            abi::add_immediate(abi::return_register(), "x9", 8),
            abi::load_u64("x1", "x9", 0),
        ]);
        emit_call_validate_utf8(symbol, &encoding_error, &mut instructions, &mut relocations);
        instructions.extend([
            abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), STR_OFFSET),
            abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
            abi::branch(&done),
            abi::label(&encoding_error),
        ]);
        emit_fail(
            symbol,
            ERR_ENCODING_CODE,
            ERR_ENCODING_SYMBOL,
            &mut instructions,
            &mut relocations,
            &done,
        );
    } else {
        // Build a List OF Byte with N elements.
        instructions.extend([
            abi::label(&build_list),
            abi::load_u64("x10", abi::stack_pointer(), N_OFFSET),
            abi::move_immediate("x11", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
            abi::multiply_registers("x12", "x10", "x11"),
            abi::add_immediate("x12", "x12", COLLECTION_HEADER_SIZE),
            abi::add_registers(abi::return_register(), "x12", "x10"),
            abi::move_immediate("x1", "Integer", "8"),
        ]);
        emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
        instructions.extend([
            abi::move_immediate("x9", "Byte", &COLLECTION_KIND_LIST.to_string()),
            abi::store_u8("x9", "x1", COLLECTION_OFFSET_KIND),
            abi::move_immediate("x9", "Byte", &COLLECTION_TYPE_NONE.to_string()),
            abi::store_u8("x9", "x1", COLLECTION_OFFSET_KEY_TYPE),
            abi::move_immediate("x9", "Byte", &COLLECTION_TYPE_BYTE.to_string()),
            abi::store_u8("x9", "x1", COLLECTION_OFFSET_VALUE_TYPE),
            abi::move_immediate("x9", "Byte", "1"),
            abi::store_u8("x9", "x1", COLLECTION_OFFSET_FLAGS_VERSION),
            abi::load_u64("x10", abi::stack_pointer(), N_OFFSET),
            abi::store_u64("x10", "x1", COLLECTION_OFFSET_COUNT),
            abi::store_u64("x10", "x1", COLLECTION_OFFSET_CAPACITY),
            abi::store_u64("x10", "x1", COLLECTION_OFFSET_DATA_LENGTH),
            abi::store_u64("x10", "x1", COLLECTION_OFFSET_DATA_CAPACITY),
            abi::add_immediate("x11", "x1", COLLECTION_HEADER_SIZE),
            abi::move_immediate("x12", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
            abi::multiply_registers("x13", "x10", "x12"),
            abi::add_registers("x14", "x11", "x13"),
            // x11 = entry cursor, x14 = data region, copy bytes into data.
            abi::load_u64("x15", abi::stack_pointer(), BUF_OFFSET),
            abi::move_immediate("x9", "Integer", "0"),
            abi::label(&entry_loop),
            abi::compare_registers("x9", "x10"),
            abi::branch_eq(&entry_done),
            abi::move_immediate("x12", "Byte", &COLLECTION_ENTRY_FLAG_USED.to_string()),
            abi::store_u8("x12", "x11", COLLECTION_ENTRY_OFFSET_FLAGS),
            abi::store_u64("x31", "x11", COLLECTION_ENTRY_OFFSET_KEY_OFFSET),
            abi::store_u64("x31", "x11", COLLECTION_ENTRY_OFFSET_KEY_LENGTH),
            abi::store_u64("x9", "x11", COLLECTION_ENTRY_OFFSET_VALUE_OFFSET),
            abi::move_immediate("x12", "Integer", "1"),
            abi::store_u64("x12", "x11", COLLECTION_ENTRY_OFFSET_VALUE_LENGTH),
            // data[i] = buf[i]
            abi::add_registers("x12", "x14", "x9"),
            abi::load_u8("x13", "x15", 0),
            abi::store_u8("x13", "x12", 0),
            abi::add_immediate("x15", "x15", 1),
            abi::add_immediate("x11", "x11", COLLECTION_ENTRY_SIZE),
            abi::add_immediate("x9", "x9", 1),
            abi::branch(&entry_loop),
            abi::label(&entry_done),
            abi::move_register(RESULT_VALUE_REGISTER, "x1"),
            abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
            abi::branch(&done),
        ]);
    }
    instructions.push(abi::label(&peer_closed));
    emit_fail(
        symbol,
        ERR_CONNECTION_CLOSED_CODE,
        ERR_CONNECTION_CLOSED_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    // read_fail: distinguish a read timeout (EAGAIN) from a closed connection.
    instructions.push(abi::label(&read_fail));
    platform.emit_errno(symbol, platform_imports, &mut instructions, &mut relocations)?;
    instructions.extend([
        abi::compare_immediate("x9", platform.eagain()),
        abi::branch_eq(&timeout),
    ]);
    emit_fail(
        symbol,
        ERR_CONNECTION_CLOSED_CODE,
        ERR_CONNECTION_CLOSED_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&timeout));
    emit_fail(
        symbol,
        ERR_READ_TIMEOUT_CODE,
        ERR_READ_TIMEOUT_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&invalid));
    emit_fail(
        symbol,
        ERR_INVALID_ARGUMENT_CODE,
        ERR_INVALID_ARGUMENT_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&closed));
    emit_fail(
        symbol,
        ERR_RESOURCE_CLOSED_CODE,
        ERR_RESOURCE_CLOSED_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&alloc_fail));
    emit_fail(
        symbol,
        ERR_OUT_OF_MEMORY_CODE,
        ERR_ALLOCATION_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([
        abi::label(&done),
        abi::load_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::add_stack(FRAME_SIZE),
        abi::return_(),
    ]);
    Ok((frame(FRAME_SIZE), instructions, relocations))
}

// ---------------------------------------------------------------------------
// net.write / net.writeText
// ---------------------------------------------------------------------------

pub(super) fn lower_net_write_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    text: bool,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 96;
    const LR_OFFSET: usize = 0;
    const FD_OFFSET: usize = 8;
    const SRC_OFFSET: usize = 16; // pointer to the next byte to write
    const REMAINING_OFFSET: usize = 24;

    let closed = format!("{symbol}_closed");
    let write_loop = format!("{symbol}_write_loop");
    let write_done = format!("{symbol}_write_done");
    let write_fail = format!("{symbol}_write_fail");
    let timeout = format!("{symbol}_timeout");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::load_u64("x9", abi::return_register(), FILE_OFFSET_CLOSED),
        abi::compare_immediate("x9", "0"),
        abi::branch_ne(&closed),
        abi::load_u64("x9", abi::return_register(), FILE_OFFSET_FD),
        abi::store_u64("x9", abi::stack_pointer(), FD_OFFSET),
    ]);
    if text {
        // x1 = String*: data at +8, length at +0.
        instructions.extend([
            abi::load_u64("x10", "x1", 0),
            abi::store_u64("x10", abi::stack_pointer(), REMAINING_OFFSET),
            abi::add_immediate("x11", "x1", 8),
            abi::store_u64("x11", abi::stack_pointer(), SRC_OFFSET),
        ]);
    } else {
        // x1 = List OF Byte collection: bytes live inline in the data region at
        // collection + HEADER + count * ENTRY_SIZE.
        instructions.extend([
            abi::load_u64("x10", "x1", COLLECTION_OFFSET_COUNT),
            abi::store_u64("x10", abi::stack_pointer(), REMAINING_OFFSET),
            abi::move_immediate("x12", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
            abi::multiply_registers("x13", "x10", "x12"),
            abi::add_immediate("x13", "x13", COLLECTION_HEADER_SIZE),
            abi::add_registers("x11", "x1", "x13"),
            abi::store_u64("x11", abi::stack_pointer(), SRC_OFFSET),
        ]);
    }
    instructions.extend([
        abi::label(&write_loop),
        abi::load_u64("x10", abi::stack_pointer(), REMAINING_OFFSET),
        abi::compare_immediate("x10", "0"),
        abi::branch_eq(&write_done),
        // write(fd, src, remaining)
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::load_u64("x1", abi::stack_pointer(), SRC_OFFSET),
        abi::move_register("x2", "x10"),
    ]);
    platform.emit_write(symbol, platform_imports, &mut instructions, &mut relocations)?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_le(&write_fail),
        abi::load_u64("x11", abi::stack_pointer(), SRC_OFFSET),
        abi::load_u64("x10", abi::stack_pointer(), REMAINING_OFFSET),
        abi::add_registers("x11", "x11", abi::return_register()),
        abi::subtract_registers("x10", "x10", abi::return_register()),
        abi::store_u64("x11", abi::stack_pointer(), SRC_OFFSET),
        abi::store_u64("x10", abi::stack_pointer(), REMAINING_OFFSET),
        abi::branch(&write_loop),
        abi::label(&write_done),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&write_fail),
    ]);
    platform.emit_errno(symbol, platform_imports, &mut instructions, &mut relocations)?;
    instructions.extend([
        abi::compare_immediate("x9", platform.eagain()),
        abi::branch_eq(&timeout),
    ]);
    emit_fail(
        symbol,
        ERR_CONNECTION_CLOSED_CODE,
        ERR_CONNECTION_CLOSED_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&timeout));
    emit_fail(
        symbol,
        ERR_WRITE_TIMEOUT_CODE,
        ERR_WRITE_TIMEOUT_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&closed));
    emit_fail(
        symbol,
        ERR_RESOURCE_CLOSED_CODE,
        ERR_RESOURCE_CLOSED_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([
        abi::label(&done),
        abi::load_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::add_stack(FRAME_SIZE),
        abi::return_(),
    ]);
    Ok((frame(FRAME_SIZE), instructions, relocations))
}

// ---------------------------------------------------------------------------
// net.poll
// ---------------------------------------------------------------------------

pub(super) fn lower_net_poll_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 48;
    const LR_OFFSET: usize = 0;
    const POLLFD_OFFSET: usize = 16;

    let closed = format!("{symbol}_closed");
    let invalid = format!("{symbol}_invalid");
    let poll_fail = format!("{symbol}_poll_fail");
    let not_ready = format!("{symbol}_not_ready");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        // x1 = timeoutMs; reject negative timeouts.
        abi::compare_immediate("x1", "0"),
        abi::branch_lt(&invalid),
        abi::move_register("x12", "x1"),
        abi::load_u64("x9", abi::return_register(), FILE_OFFSET_CLOSED),
        abi::compare_immediate("x9", "0"),
        abi::branch_ne(&closed),
        abi::load_u64("x9", abi::return_register(), FILE_OFFSET_FD),
        // pollfd { int fd; short events = POLLIN; short revents; }
        abi::store_u64("x9", abi::stack_pointer(), POLLFD_OFFSET),
        abi::move_immediate("x10", "Integer", POLLIN),
        abi::store_u8("x10", abi::stack_pointer(), POLLFD_OFFSET + 4),
        abi::store_u8("x31", abi::stack_pointer(), POLLFD_OFFSET + 5),
        abi::store_u8("x31", abi::stack_pointer(), POLLFD_OFFSET + 6),
        abi::store_u8("x31", abi::stack_pointer(), POLLFD_OFFSET + 7),
        // poll(&pollfd, 1, timeoutMs)
        abi::add_immediate(abi::return_register(), abi::stack_pointer(), POLLFD_OFFSET),
        abi::move_immediate("x1", "Integer", "1"),
        abi::move_register("x2", "x12"),
    ]);
    platform.emit_libc_call("poll", symbol, platform_imports, &mut instructions, &mut relocations)?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&poll_fail),
        abi::branch_eq(&not_ready),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "1"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&not_ready),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&poll_fail),
    ]);
    emit_fail(
        symbol,
        ERR_RESOURCE_CLOSED_CODE,
        ERR_RESOURCE_CLOSED_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&invalid));
    emit_fail(
        symbol,
        ERR_INVALID_ARGUMENT_CODE,
        ERR_INVALID_ARGUMENT_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&closed));
    emit_fail(
        symbol,
        ERR_RESOURCE_CLOSED_CODE,
        ERR_RESOURCE_CLOSED_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([
        abi::label(&done),
        abi::load_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::add_stack(FRAME_SIZE),
        abi::return_(),
    ]);
    Ok((frame(FRAME_SIZE), instructions, relocations))
}

// ---------------------------------------------------------------------------
// net.setReadTimeout / net.setWriteTimeout
// ---------------------------------------------------------------------------

pub(super) fn lower_net_set_timeout_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    write: bool,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 48;
    const LR_OFFSET: usize = 0;
    const FD_OFFSET: usize = 8;
    const TIMEVAL_OFFSET: usize = 16; // tv_sec (8) + tv_usec (8)

    let closed = format!("{symbol}_closed");
    let invalid = format!("{symbol}_invalid");
    let set_fail = format!("{symbol}_set_fail");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        // x1 = timeoutMs; reject negatives.
        abi::compare_immediate("x1", "0"),
        abi::branch_lt(&invalid),
        abi::load_u64("x9", abi::return_register(), FILE_OFFSET_CLOSED),
        abi::compare_immediate("x9", "0"),
        abi::branch_ne(&closed),
        abi::load_u64("x9", abi::return_register(), FILE_OFFSET_FD),
        abi::store_u64("x9", abi::stack_pointer(), FD_OFFSET),
        // tv_sec = ms / 1000, tv_usec = (ms % 1000) * 1000
        abi::move_immediate("x10", "Integer", "1000"),
        abi::unsigned_divide_registers("x11", "x1", "x10"),
        abi::multiply_subtract_registers("x12", "x11", "x10", "x1"),
        abi::move_immediate("x13", "Integer", "1000"),
        abi::multiply_registers("x12", "x12", "x13"),
        abi::store_u64("x11", abi::stack_pointer(), TIMEVAL_OFFSET),
        abi::store_u64("x12", abi::stack_pointer(), TIMEVAL_OFFSET + 8),
        // setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO/SO_SNDTIMEO, &tv, 16)
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::move_immediate("x1", "Integer", platform.sol_socket()),
        abi::move_immediate(
            "x2",
            "Integer",
            if write {
                platform.so_sndtimeo()
            } else {
                platform.so_rcvtimeo()
            },
        ),
        abi::add_immediate("x3", abi::stack_pointer(), TIMEVAL_OFFSET),
        abi::move_immediate("x4", "Integer", "16"),
    ]);
    platform.emit_libc_call(
        "setsockopt",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&set_fail),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&set_fail),
    ]);
    emit_fail(
        symbol,
        ERR_RESOURCE_CLOSED_CODE,
        ERR_RESOURCE_CLOSED_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&invalid));
    emit_fail(
        symbol,
        ERR_INVALID_ARGUMENT_CODE,
        ERR_INVALID_ARGUMENT_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&closed));
    emit_fail(
        symbol,
        ERR_RESOURCE_CLOSED_CODE,
        ERR_RESOURCE_CLOSED_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([
        abi::label(&done),
        abi::load_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::add_stack(FRAME_SIZE),
        abi::return_(),
    ]);
    Ok((frame(FRAME_SIZE), instructions, relocations))
}

// ---------------------------------------------------------------------------
// net.lookup
// ---------------------------------------------------------------------------

pub(super) fn lower_net_lookup_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const FRAME_SIZE: usize = 256;
    const LR_OFFSET: usize = 0;
    const HOST_OFFSET: usize = 8;
    const PORT_OFFSET: usize = 16;
    const RES_OFFSET: usize = 24;
    const CSTR_OFFSET: usize = 32;
    const COUNT_OFFSET: usize = 40;
    const NODE_OFFSET: usize = 48;
    const LIST_OFFSET: usize = 56;
    const ENTRY_OFFSET: usize = 64;
    const DATA_OFFSET: usize = 72;
    const INDEX_OFFSET: usize = 80;
    const DST_OFFSET: usize = 88;
    const ADDRHOST_OFFSET: usize = 96;
    const SADDR_PTR_OFFSET: usize = 152;
    const HOSTLEN_OFFSET: usize = 160;
    const HINTS_OFFSET: usize = 104; // 104..152

    let resolve_fail = format!("{symbol}_resolve_fail");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let addr_fail = format!("{symbol}_addr_fail");
    let count_loop = format!("{symbol}_count_loop");
    let count_skip = format!("{symbol}_count_skip");
    let count_done = format!("{symbol}_count_done");
    let fill_loop = format!("{symbol}_fill_loop");
    let fill_skip = format!("{symbol}_fill_skip");
    let fill_done = format!("{symbol}_fill_done");
    let done = format!("{symbol}_done");

    let addr_off = platform.addrinfo_addr_offset();
    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    instructions.extend([
        abi::store_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), HOST_OFFSET),
        abi::store_u64("x1", abi::stack_pointer(), PORT_OFFSET),
    ]);
    emit_hints(HINTS_OFFSET, false, &mut instructions);
    emit_cstring(
        symbol,
        "host",
        HOST_OFFSET,
        CSTR_OFFSET,
        &alloc_fail,
        &mut instructions,
        &mut relocations,
    );
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CSTR_OFFSET),
        abi::move_immediate("x1", "Integer", "0"),
        abi::add_immediate("x2", abi::stack_pointer(), HINTS_OFFSET),
        abi::add_immediate("x3", abi::stack_pointer(), RES_OFFSET),
    ]);
    platform.emit_libc_call(
        "getaddrinfo",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&resolve_fail),
        // Count AF_INET results.
        abi::load_u64("x9", abi::stack_pointer(), RES_OFFSET),
        abi::store_u64("x9", abi::stack_pointer(), NODE_OFFSET),
        abi::store_u64("x31", abi::stack_pointer(), COUNT_OFFSET),
        abi::label(&count_loop),
        abi::load_u64("x9", abi::stack_pointer(), NODE_OFFSET),
        abi::compare_immediate("x9", "0"),
        abi::branch_eq(&count_done),
        abi::load_u32("x10", "x9", 4),
        abi::compare_immediate("x10", AF_INET),
        abi::branch_ne(&count_skip),
        abi::load_u64("x11", abi::stack_pointer(), COUNT_OFFSET),
        abi::add_immediate("x11", "x11", 1),
        abi::store_u64("x11", abi::stack_pointer(), COUNT_OFFSET),
        abi::label(&count_skip),
        abi::load_u64("x9", abi::stack_pointer(), NODE_OFFSET),
        abi::load_u64("x9", "x9", 40),
        abi::store_u64("x9", abi::stack_pointer(), NODE_OFFSET),
        abi::branch(&count_loop),
        abi::label(&count_done),
        // Allocate List OF Address: count Address records (16 bytes) inline.
        abi::load_u64("x10", abi::stack_pointer(), COUNT_OFFSET),
        abi::move_immediate("x11", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers("x12", "x10", "x11"),
        abi::add_immediate("x12", "x12", COLLECTION_HEADER_SIZE),
        abi::move_immediate("x13", "Integer", "16"),
        abi::multiply_registers("x14", "x10", "x13"),
        abi::add_registers(abi::return_register(), "x12", "x14"),
        abi::move_immediate("x1", "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.extend([
        abi::store_u64("x1", abi::stack_pointer(), LIST_OFFSET),
        abi::move_immediate("x9", "Byte", &COLLECTION_KIND_LIST.to_string()),
        abi::store_u8("x9", "x1", COLLECTION_OFFSET_KIND),
        abi::move_immediate("x9", "Byte", &COLLECTION_TYPE_NONE.to_string()),
        abi::store_u8("x9", "x1", COLLECTION_OFFSET_KEY_TYPE),
        abi::move_immediate("x9", "Byte", &COLLECTION_TYPE_OBJECT.to_string()),
        abi::store_u8("x9", "x1", COLLECTION_OFFSET_VALUE_TYPE),
        abi::move_immediate("x9", "Byte", "1"),
        abi::store_u8("x9", "x1", COLLECTION_OFFSET_FLAGS_VERSION),
        abi::load_u64("x10", abi::stack_pointer(), COUNT_OFFSET),
        abi::store_u64("x10", "x1", COLLECTION_OFFSET_COUNT),
        abi::store_u64("x10", "x1", COLLECTION_OFFSET_CAPACITY),
        abi::move_immediate("x13", "Integer", "16"),
        abi::multiply_registers("x14", "x10", "x13"),
        abi::store_u64("x14", "x1", COLLECTION_OFFSET_DATA_LENGTH),
        abi::store_u64("x14", "x1", COLLECTION_OFFSET_DATA_CAPACITY),
        // entry cursor and data region.
        abi::add_immediate("x11", "x1", COLLECTION_HEADER_SIZE),
        abi::store_u64("x11", abi::stack_pointer(), ENTRY_OFFSET),
        abi::move_immediate("x12", "Integer", &COLLECTION_ENTRY_SIZE.to_string()),
        abi::multiply_registers("x13", "x10", "x12"),
        abi::add_registers("x14", "x11", "x13"),
        abi::store_u64("x14", abi::stack_pointer(), DATA_OFFSET),
        // Iterate results again, building one Address per AF_INET node.
        abi::load_u64("x9", abi::stack_pointer(), RES_OFFSET),
        abi::store_u64("x9", abi::stack_pointer(), NODE_OFFSET),
        abi::store_u64("x31", abi::stack_pointer(), INDEX_OFFSET),
        abi::label(&fill_loop),
        abi::load_u64("x9", abi::stack_pointer(), NODE_OFFSET),
        abi::compare_immediate("x9", "0"),
        abi::branch_eq(&fill_done),
        abi::load_u32("x10", "x9", 4),
        abi::compare_immediate("x10", AF_INET),
        abi::branch_ne(&fill_skip),
        // node->ai_addr; force the requested port into sin_port.
        abi::load_u64("x12", "x9", addr_off),
        abi::store_u64("x12", abi::stack_pointer(), SADDR_PTR_OFFSET),
        abi::load_u64("x10", abi::stack_pointer(), PORT_OFFSET),
        abi::shift_right_immediate("x11", "x10", 8),
        abi::store_u8("x11", "x12", 2),
        abi::store_u8("x10", "x12", 3),
    ]);
    emit_address_from_sockaddr(
        symbol,
        "node",
        SADDR_PTR_OFFSET,
        HOSTLEN_OFFSET,
        DST_OFFSET,
        ADDRHOST_OFFSET,
        platform,
        platform_imports,
        &mut instructions,
        &mut relocations,
        &alloc_fail,
        &addr_fail,
    )?;
    // x1 = Address pointer; copy its 16 bytes into the list data region and
    // record the entry descriptor.
    instructions.extend([
        abi::load_u64("x9", abi::stack_pointer(), INDEX_OFFSET),
        abi::move_immediate("x10", "Integer", "16"),
        abi::multiply_registers("x11", "x9", "x10"),
        abi::load_u64("x12", abi::stack_pointer(), DATA_OFFSET),
        abi::add_registers("x12", "x12", "x11"),
        abi::load_u64("x13", "x1", 0),
        abi::store_u64("x13", "x12", 0),
        abi::load_u64("x13", "x1", 8),
        abi::store_u64("x13", "x12", 8),
        // entry descriptor at ENTRY cursor.
        abi::load_u64("x14", abi::stack_pointer(), ENTRY_OFFSET),
        abi::move_immediate("x13", "Byte", &COLLECTION_ENTRY_FLAG_USED.to_string()),
        abi::store_u8("x13", "x14", COLLECTION_ENTRY_OFFSET_FLAGS),
        abi::store_u64("x31", "x14", COLLECTION_ENTRY_OFFSET_KEY_OFFSET),
        abi::store_u64("x31", "x14", COLLECTION_ENTRY_OFFSET_KEY_LENGTH),
        abi::store_u64("x11", "x14", COLLECTION_ENTRY_OFFSET_VALUE_OFFSET),
        abi::move_immediate("x13", "Integer", "16"),
        abi::store_u64("x13", "x14", COLLECTION_ENTRY_OFFSET_VALUE_LENGTH),
        abi::add_immediate("x14", "x14", COLLECTION_ENTRY_SIZE),
        abi::store_u64("x14", abi::stack_pointer(), ENTRY_OFFSET),
        abi::load_u64("x9", abi::stack_pointer(), INDEX_OFFSET),
        abi::add_immediate("x9", "x9", 1),
        abi::store_u64("x9", abi::stack_pointer(), INDEX_OFFSET),
        abi::label(&fill_skip),
        abi::load_u64("x9", abi::stack_pointer(), NODE_OFFSET),
        abi::load_u64("x9", "x9", 40),
        abi::store_u64("x9", abi::stack_pointer(), NODE_OFFSET),
        abi::branch(&fill_loop),
        abi::label(&fill_done),
        // freeaddrinfo(res)
        abi::load_u64(abi::return_register(), abi::stack_pointer(), RES_OFFSET),
    ]);
    platform.emit_libc_call(
        "freeaddrinfo",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64(RESULT_VALUE_REGISTER, abi::stack_pointer(), LIST_OFFSET),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&resolve_fail),
    ]);
    emit_fail(
        symbol,
        ERR_ADDRESS_NOT_FOUND_CODE,
        ERR_ADDRESS_NOT_FOUND_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&addr_fail));
    emit_fail(
        symbol,
        ERR_ADDRESS_INVALID_CODE,
        ERR_ADDRESS_INVALID_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&alloc_fail));
    emit_fail(
        symbol,
        ERR_OUT_OF_MEMORY_CODE,
        ERR_ALLOCATION_SYMBOL,
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([
        abi::label(&done),
        abi::load_u64(abi::link_register(), abi::stack_pointer(), LR_OFFSET),
        abi::add_stack(FRAME_SIZE),
        abi::return_(),
    ]);
    Ok((frame(FRAME_SIZE), instructions, relocations))
}
