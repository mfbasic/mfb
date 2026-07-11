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
use crate::target::shared::abi;

const AF_INET: &str = "2";
const SOCK_STREAM: &str = "1";
const SOCK_DGRAM: &str = "2";
// hints `u64` at offset 0 packs `ai_flags` (low 32) and `ai_family` (high 32).
// `AF_INET (2) << 32`.
const HINTS_FAMILY_WORD: &str = "8589934592"; // ai_flags = 0
const HINTS_FAMILY_WORD_PASSIVE: &str = "8589934593"; // ai_flags = AI_PASSIVE (1)
const SOCKADDR_STORAGE_SIZE: usize = 128;
const ADDR_STR_CAP: usize = 64;
const POLLIN: &str = "1";
/// `EINTR` errno (Linux/macOS both use 4): a `poll` interrupted by a signal
/// returns `-1`/`EINTR` and must be re-issued rather than treated as a hard
/// connect failure (bug-115).
const EINTR_ERRNO: &str = "4";

fn internal_reloc(symbol: &str, target: &str) -> CodeRelocation {
    CodeRelocation {
        from: symbol.to_string(),
        to: target.to_string(),
        kind: RelocIntent::Call,
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
        abi::load_u64("%v9", abi::stack_pointer(), str_off),
        abi::load_u64("%v10", "%v9", 0),
        abi::add_immediate(abi::return_register(), "%v10", 1),
        abi::move_immediate(abi::ARG[1], "Integer", "1"),
    ]);
    emit_alloc(symbol, instructions, relocations, alloc_fail);
    instructions.extend([
        abi::store_u64(abi::RET[1], abi::stack_pointer(), out_off),
        abi::load_u64("%v9", abi::stack_pointer(), str_off),
        abi::load_u64("%v10", "%v9", 0),
        abi::add_immediate("%v11", "%v9", 8),
        abi::move_register("%v12", abi::RET[1]),
        abi::move_immediate("%v13", "Integer", "0"),
        abi::label(&copy_loop),
        abi::compare_registers("%v13", "%v10"),
        abi::branch_eq(&copy_done),
        abi::load_u8("%v14", "%v11", 0),
        abi::store_u8("%v14", "%v12", 0),
        abi::add_immediate("%v11", "%v11", 1),
        abi::add_immediate("%v12", "%v12", 1),
        abi::add_immediate("%v13", "%v13", 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::store_u8(abi::ZERO, "%v12", 0),
    ]);
}

/// Zero a 48-byte `addrinfo` hints block at `sp + hints_off` and set
/// `ai_family = AF_INET`, `ai_socktype = socktype` (and `AI_PASSIVE` when
/// `passive`). Clobbers `x9`.
fn emit_hints(
    hints_off: usize,
    passive: bool,
    socktype: &str,
    instructions: &mut Vec<CodeInstruction>,
) {
    for offset in (0..48).step_by(8) {
        instructions.push(abi::store_u64(
            abi::ZERO,
            abi::stack_pointer(),
            hints_off + offset,
        ));
    }
    let family_word = if passive {
        HINTS_FAMILY_WORD_PASSIVE
    } else {
        HINTS_FAMILY_WORD
    };
    instructions.extend([
        abi::move_immediate("%v9", "Integer", family_word),
        abi::store_u64("%v9", abi::stack_pointer(), hints_off),
        abi::move_immediate("%v9", "Integer", socktype),
        abi::store_u64("%v9", abi::stack_pointer(), hints_off + 8),
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
        abi::move_immediate(abi::ARG[1], "Integer", "1"),
    ]);
    emit_alloc(symbol, instructions, relocations, alloc_fail);
    instructions.extend([
        abi::store_u64(abi::RET[1], abi::stack_pointer(), dst_off),
        // inet_ntop(AF_INET, sockaddr + 4, dst, ADDR_STR_CAP)
        abi::move_immediate(abi::return_register(), "Integer", AF_INET),
        abi::load_u64("%v9", abi::stack_pointer(), sockaddr_off),
        abi::add_immediate(abi::ARG[1], "%v9", 4),
        abi::load_u64(abi::ARG[2], abi::stack_pointer(), dst_off),
        abi::move_immediate(abi::ARG[3], "Integer", &ADDR_STR_CAP.to_string()),
    ]);
    platform.emit_libc_call(
        "inet_ntop",
        symbol,
        platform_imports,
        instructions,
        relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(addr_fail),
        // Count the NUL-terminated host string length.
        abi::load_u64("%v9", abi::stack_pointer(), dst_off),
        abi::move_immediate("%v10", "Integer", "0"),
        abi::label(&count_loop),
        abi::load_u8("%v11", "%v9", 0),
        abi::compare_immediate("%v11", "0"),
        abi::branch_eq(&count_done),
        abi::add_immediate("%v9", "%v9", 1),
        abi::add_immediate("%v10", "%v10", 1),
        abi::branch(&count_loop),
        abi::label(&count_done),
        abi::store_u64("%v10", abi::stack_pointer(), len_off),
        // Allocate the host String: [u64 len][bytes][nul].
        abi::add_immediate(abi::return_register(), "%v10", 9),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
    ]);
    emit_alloc(symbol, instructions, relocations, alloc_fail);
    instructions.extend([
        abi::move_register("%v15", abi::RET[1]), // alloc result → vreg (plan-34-B Phase 3)
        abi::load_u64("%v10", abi::stack_pointer(), len_off),
        abi::store_u64("%v10", "%v15", 0),
        abi::store_u64("%v15", abi::stack_pointer(), host_off),
        abi::load_u64("%v11", abi::stack_pointer(), dst_off),
        abi::add_immediate("%v12", "%v15", 8),
        abi::move_immediate("%v13", "Integer", "0"),
        abi::label(&copy_loop),
        abi::compare_registers("%v13", "%v10"),
        abi::branch_eq(&copy_done),
        abi::load_u8("%v14", "%v11", 0),
        abi::store_u8("%v14", "%v12", 0),
        abi::add_immediate("%v11", "%v11", 1),
        abi::add_immediate("%v12", "%v12", 1),
        abi::add_immediate("%v13", "%v13", 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::store_u8(abi::ZERO, "%v12", 0),
        // Allocate the Address record: [host ptr][port].
        abi::move_immediate(abi::return_register(), "Integer", "16"),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
    ]);
    emit_alloc(symbol, instructions, relocations, alloc_fail);
    instructions.extend([
        abi::move_register("%v16", abi::RET[1]), // alloc result → vreg (plan-34-B Phase 3)
        abi::load_u64("%v9", abi::stack_pointer(), host_off),
        abi::store_u64("%v9", "%v16", 0),
        // port = (sockaddr[2] << 8) | sockaddr[3]
        abi::load_u64("%v9", abi::stack_pointer(), sockaddr_off),
        abi::load_u8("%v10", "%v9", 2),
        abi::load_u8("%v11", "%v9", 3),
        abi::shift_left_immediate("%v10", "%v10", 8),
        abi::or_registers("%v10", "%v10", "%v11"),
        abi::store_u64("%v10", "%v16", 8),
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
        abi::move_immediate(abi::return_register(), "Integer", RESOURCE_RECORD_SIZE),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
    ]);
    emit_alloc(symbol, instructions, relocations, alloc_fail);
    instructions.extend([
        abi::move_register("%v10", abi::RET[1]), // alloc result → vreg base; x1 stays the returned ptr
        abi::load_u64("%v9", abi::stack_pointer(), fd_off),
        abi::store_u64("%v9", "%v10", FILE_OFFSET_FD),
        abi::store_u64(abi::ZERO, "%v10", FILE_OFFSET_CLOSED),
        abi::store_u64(abi::ZERO, "%v10", FILE_OFFSET_STATE),
    ]);
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
    // getaddrinfo `service` pointer (NULL for a resolved host; the `"0"` C string
    // below for a NULL/bind-all host, since getaddrinfo rejects node==service==NULL
    // — the real port is patched into sin_port afterward). bug-113.
    const SERVICE_OFFSET: usize = 144;
    const SERVICE_STR_OFFSET: usize = 152; // holds the bytes "0\0…"

    let null_host = format!("{symbol}_null_host");
    let resolved = format!("{symbol}_resolved");
    let resolve_fail = format!("{symbol}_resolve_fail");
    let socket_fail = format!("{symbol}_socket_fail");
    let op_fail = format!("{symbol}_op_fail");
    let blocking_connect = format!("{symbol}_blocking_connect");
    let nb_connected = format!("{symbol}_nb_connected");
    let connect_poll_retry = format!("{symbol}_connect_poll_retry");
    let connect_poll_ready = format!("{symbol}_connect_poll_ready");
    let connect_timeout = format!("{symbol}_connect_timeout");
    let connected_done = format!("{symbol}_connected_done");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let done = format!("{symbol}_done");

    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    if address {
        // x0 = Address record { host String ptr @0, port @8 }; x1 = timeoutMs.
        instructions.extend([
            abi::load_u64("%v9", abi::return_register(), 0),
            abi::store_u64("%v9", abi::stack_pointer(), HOST_OFFSET),
            abi::load_u64("%v9", abi::return_register(), 8),
            abi::store_u64("%v9", abi::stack_pointer(), PORT_OFFSET),
            abi::store_u64(abi::ARG[1], abi::stack_pointer(), EXTRA_OFFSET),
        ]);
    } else {
        instructions.extend([
            abi::store_u64(abi::return_register(), abi::stack_pointer(), HOST_OFFSET),
            abi::store_u64(abi::ARG[1], abi::stack_pointer(), PORT_OFFSET),
            abi::store_u64(abi::ARG[2], abi::stack_pointer(), EXTRA_OFFSET),
        ]);
    }
    emit_hints(HINTS_OFFSET, listen, SOCK_STREAM, &mut instructions);
    // Default getaddrinfo service = NULL (valid whenever the host is non-NULL).
    instructions.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), SERVICE_OFFSET));
    // Choose host C string. An empty host on a listener binds all interfaces
    // (NULL host + AI_PASSIVE).
    instructions.extend([
        abi::load_u64("%v9", abi::stack_pointer(), HOST_OFFSET),
        abi::load_u64("%v9", "%v9", 0),
        abi::compare_immediate("%v9", "0"),
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
            abi::store_u64(abi::ZERO, abi::stack_pointer(), CSTR_OFFSET),
            // Bind-all: node is NULL, so service must be non-NULL. Stage the C
            // string "0" (0x30 then a zero terminator) and point service at it,
            // so getaddrinfo(NULL, "0", &hints|AI_PASSIVE, …) returns the wildcard
            // address instead of EAI_NONAME (bug-113). The real port overwrites
            // sin_port afterward.
            abi::move_immediate("%v9", "Integer", "48"),
            abi::store_u64("%v9", abi::stack_pointer(), SERVICE_STR_OFFSET),
            abi::add_immediate("%v9", abi::stack_pointer(), SERVICE_STR_OFFSET),
            abi::store_u64("%v9", abi::stack_pointer(), SERVICE_OFFSET),
        ]);
    }
    instructions.extend([
        abi::label(&resolved),
        // getaddrinfo(host, service, &hints, &res)
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CSTR_OFFSET),
        abi::load_u64(abi::ARG[1], abi::stack_pointer(), SERVICE_OFFSET),
        abi::add_immediate(abi::ARG[2], abi::stack_pointer(), HINTS_OFFSET),
        abi::add_immediate(abi::ARG[3], abi::stack_pointer(), RES_OFFSET),
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
        abi::load_u64("%v9", abi::stack_pointer(), RES_OFFSET),
        abi::load_u32(abi::return_register(), "%v9", 4),
        abi::load_u32(abi::ARG[1], "%v9", 8),
        abi::load_u32(abi::ARG[2], "%v9", 12),
    ]);
    platform.emit_libc_call(
        "socket",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&socket_fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        // Overwrite sin_port at ai_addr + 2/3 with the requested port (network
        // byte order).
        abi::load_u64("%v9", abi::stack_pointer(), RES_OFFSET),
        abi::load_u64("%v9", "%v9", platform.addrinfo_addr_offset()),
        abi::load_u64("%v10", abi::stack_pointer(), PORT_OFFSET),
        abi::shift_right_immediate("%v11", "%v10", 8),
        abi::store_u8("%v11", "%v9", 2),
        abi::store_u8("%v10", "%v9", 3),
    ]);
    if listen {
        // setsockopt(fd, SOL_SOCKET, SO_REUSEADDR, &one, 4) - best effort.
        instructions.extend([
            abi::move_immediate("%v9", "Integer", "1"),
            abi::store_u64("%v9", abi::stack_pointer(), ONE_OFFSET),
            abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
            abi::move_immediate(abi::ARG[1], "Integer", platform.sol_socket()),
            abi::move_immediate(abi::ARG[2], "Integer", platform.so_reuseaddr()),
            abi::add_immediate(abi::ARG[3], abi::stack_pointer(), ONE_OFFSET),
            abi::move_immediate(abi::ARG[4], "Integer", "4"),
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
            abi::load_u64("%v9", abi::stack_pointer(), RES_OFFSET),
            abi::load_u64(abi::ARG[1], "%v9", platform.addrinfo_addr_offset()),
            abi::load_u32(abi::ARG[2], "%v9", 16),
        ]);
        platform.emit_libc_call(
            "bind",
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.extend([
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_lt(&op_fail),
            // listen(fd, backlog)
            abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
            abi::load_u64(abi::ARG[1], abi::stack_pointer(), EXTRA_OFFSET),
        ]);
        platform.emit_libc_call(
            "listen",
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.extend([
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_lt(&op_fail),
        ]);
    } else {
        // `timeoutMs <= 0` uses the implementation default: a plain blocking
        // connect. `timeoutMs > 0` performs a non-blocking connect bounded by a
        // `poll`, then restores blocking mode.
        instructions.extend([
            abi::load_u64("%v9", abi::stack_pointer(), EXTRA_OFFSET),
            abi::compare_immediate("%v9", "0"),
            abi::branch_le(&blocking_connect),
            // flags = fcntl(fd, F_GETFL, 0)
            abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
            abi::move_immediate(abi::ARG[1], "Integer", "3"),
            abi::move_immediate(abi::ARG[2], "Integer", "0"),
        ]);
        platform.emit_variadic_call(
            "fcntl",
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.extend([
            abi::store_u64(abi::return_register(), abi::stack_pointer(), FLAGS_OFFSET),
            // fcntl(fd, F_SETFL, flags | O_NONBLOCK)
            abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
            abi::move_immediate(abi::ARG[1], "Integer", "4"),
            abi::load_u64(abi::ARG[2], abi::stack_pointer(), FLAGS_OFFSET),
            abi::move_immediate("%v9", "Integer", platform.o_nonblock()),
            abi::or_registers(abi::ARG[2], abi::ARG[2], "%v9"),
        ]);
        platform.emit_variadic_call(
            "fcntl",
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        // connect(fd, ai_addr, ai_addrlen)
        instructions.extend([
            abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
            abi::load_u64("%v9", abi::stack_pointer(), RES_OFFSET),
            abi::load_u64(abi::ARG[1], "%v9", platform.addrinfo_addr_offset()),
            abi::load_u32(abi::ARG[2], "%v9", 16),
        ]);
        platform.emit_libc_call(
            "connect",
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.extend([
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_eq(&nb_connected),
        ]);
        // In progress? Anything other than EINPROGRESS is a hard failure.
        platform.emit_errno(
            symbol,
            "%v9",
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.extend([
            abi::compare_immediate("%v9", platform.einprogress()),
            abi::branch_ne(&op_fail),
            // poll(&pollfd { fd, POLLOUT }, 1, timeoutMs); connect_poll_retry
            // re-runs the pollfd rebuild + poll on an EINTR (bug-115).
            abi::label(&connect_poll_retry),
            abi::load_u64("%v9", abi::stack_pointer(), FD_OFFSET),
            abi::store_u64("%v9", abi::stack_pointer(), POLLFD_OFFSET),
            abi::move_immediate("%v10", "Integer", "4"), // POLLOUT
            abi::store_u8("%v10", abi::stack_pointer(), POLLFD_OFFSET + 4),
            abi::store_u8(abi::ZERO, abi::stack_pointer(), POLLFD_OFFSET + 5),
            abi::store_u8(abi::ZERO, abi::stack_pointer(), POLLFD_OFFSET + 6),
            abi::store_u8(abi::ZERO, abi::stack_pointer(), POLLFD_OFFSET + 7),
            abi::add_immediate(abi::return_register(), abi::stack_pointer(), POLLFD_OFFSET),
            abi::move_immediate(abi::ARG[1], "Integer", "1"),
            abi::load_u64(abi::ARG[2], abi::stack_pointer(), EXTRA_OFFSET),
        ]);
        platform.emit_libc_call(
            "poll",
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.extend([
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_eq(&connect_timeout),
            abi::branch_gt(&connect_poll_ready),
        ]);
        // bug-115: a negative poll return is either EINTR (re-issue the poll) or a
        // genuine failure. poll goes through libc here, so read errno.
        platform.emit_errno(
            symbol,
            "%v9",
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.extend([
            abi::compare_immediate("%v9", EINTR_ERRNO),
            abi::branch_eq(&connect_poll_retry),
            abi::branch(&op_fail),
            abi::label(&connect_poll_ready),
            // getsockopt(fd, SOL_SOCKET, SO_ERROR, &err, &len)
            abi::move_immediate("%v9", "Integer", "4"),
            abi::store_u64("%v9", abi::stack_pointer(), SOLEN_OFFSET),
            abi::store_u64(abi::ZERO, abi::stack_pointer(), SOERR_OFFSET),
            abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
            abi::move_immediate(abi::ARG[1], "Integer", platform.sol_socket()),
            abi::move_immediate(abi::ARG[2], "Integer", platform.so_error()),
            abi::add_immediate(abi::ARG[3], abi::stack_pointer(), SOERR_OFFSET),
            abi::add_immediate(abi::ARG[4], abi::stack_pointer(), SOLEN_OFFSET),
        ]);
        platform.emit_libc_call(
            "getsockopt",
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.extend([
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_lt(&op_fail),
            abi::load_u32("%v9", abi::stack_pointer(), SOERR_OFFSET),
            abi::compare_immediate("%v9", "0"),
            abi::branch_ne(&op_fail),
            // Connected: restore blocking mode with fcntl(fd, F_SETFL, flags).
            abi::label(&nb_connected),
            abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
            abi::move_immediate(abi::ARG[1], "Integer", "4"),
            abi::load_u64(abi::ARG[2], abi::stack_pointer(), FLAGS_OFFSET),
        ]);
        platform.emit_variadic_call(
            "fcntl",
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.extend([
            abi::branch(&connected_done),
            // Blocking connect path (timeoutMs <= 0).
            abi::label(&blocking_connect),
            abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
            abi::load_u64("%v9", abi::stack_pointer(), RES_OFFSET),
            abi::load_u64(abi::ARG[1], "%v9", platform.addrinfo_addr_offset()),
            abi::load_u32(abi::ARG[2], "%v9", 16),
        ]);
        platform.emit_libc_call(
            "connect",
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.extend([
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_lt(&op_fail),
            abi::label(&connected_done),
        ]);
    }
    // freeaddrinfo(res)
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
    emit_make_handle(
        symbol,
        FD_OFFSET,
        &mut instructions,
        &mut relocations,
        &alloc_fail,
    );
    instructions.extend([
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);
    // op_fail / socket_fail: free resources then report network failure. The
    // socket fd (if any) leaks on these rare error paths; the process-level
    // failure is surfaced to the caller as a network error.
    instructions.push(abi::label(&op_fail));
    instructions.push(abi::load_u64(
        abi::return_register(),
        abi::stack_pointer(),
        FD_OFFSET,
    ));
    platform.emit_libc_call(
        "close",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.push(abi::label(&socket_fail));
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
    platform.emit_libc_call(
        "close",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
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
    instructions.extend([abi::label(&done), abi::return_()]);
    {
        let (frame, stack_slots) =
            finalize_vreg_body_with_locals(&mut instructions, &[], FRAME_SIZE);
        Ok((frame, instructions, relocations, stack_slots))
    }
}

pub(super) fn lower_net_connect_tcp_helper(
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
    lower_net_endpoint_helper(symbol, platform_imports, platform, false, false)
}

pub(super) fn lower_net_connect_tcp_addr_helper(
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
    lower_net_endpoint_helper(symbol, platform_imports, platform, false, true)
}

pub(super) fn lower_net_listen_tcp_helper(
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
    lower_net_endpoint_helper(symbol, platform_imports, platform, true, false)
}

mod io;
mod poll;

pub(super) use io::*;
pub(super) use poll::*;
