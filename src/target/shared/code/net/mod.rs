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
//!
//! Visibility here is spelled `pub(in crate::target::shared::code)` in full,
//! matching `io.rs` and `poll.rs`, rather than the shorter `pub(super)`. In this
//! file the two happen to mean the same thing; in the child modules they do not
//! (`pub(super)` there would mean `pub(in ...::net)`), so the long form is the
//! only spelling that is correct in all three and can be copied between them.

mod io;
mod poll;

pub(in crate::target::shared::code) use io::*;
pub(in crate::target::shared::code) use poll::*;

use std::collections::HashMap;

use super::*;
use crate::target::shared::abi;

/// The socket-call symbols the shared `net` lowering issues. Every hardcoded
/// libc symbol literal routes through [`net_symbol`] so a platform whose socket
/// ABI diverges from POSIX (Windows/Winsock) can rename it in one place instead
/// of at 35 call sites. Names mirror the POSIX symbol they map to on every
/// non-Windows target (plan-47-I I1).
#[derive(Clone, Copy)]
pub(in crate::target::shared::code) enum NetSymbol {
    Socket,
    Connect,
    Bind,
    Listen,
    Accept,
    Recv,
    Send,
    RecvFrom,
    SendTo,
    Close,
    Fcntl,
    Poll,
    GetAddrInfo,
    FreeAddrInfo,
    SetSockOpt,
    GetSockOpt,
}

/// Map a [`NetSymbol`] intent to the concrete libc/Winsock symbol for `platform`.
/// On every non-Windows target this returns the POSIX name unchanged, so the
/// four existing backends stay byte-identical (I1's proof). Winsock's three
/// renames (`close`→`closesocket`, `poll`→`WSAPoll`, and the `fcntl` non-blocking
/// toggle, which is rewritten to `ioctlsocket` at the call site) land in I2.
pub(in crate::target::shared::code) fn net_symbol(
    platform: &dyn CodegenPlatform,
    intent: NetSymbol,
) -> &'static str {
    if platform.family() == PlatformFamily::Windows {
        match intent {
            // A SOCKET is not a file descriptor; close() on it is undefined.
            NetSymbol::Close => return "closesocket",
            NetSymbol::Poll => return "WSAPoll",
            // Fcntl never reaches here on Windows: both call sites branch to
            // ioctlsocket (emit_set_nonblocking / emit_restore_blocking) instead.
            _ => {}
        }
    }
    match intent {
        NetSymbol::Socket => "socket",
        NetSymbol::Connect => "connect",
        NetSymbol::Bind => "bind",
        NetSymbol::Listen => "listen",
        NetSymbol::Accept => "accept",
        NetSymbol::Recv => "recv",
        NetSymbol::Send => "send",
        NetSymbol::RecvFrom => "recvfrom",
        NetSymbol::SendTo => "sendto",
        NetSymbol::Close => "close",
        NetSymbol::Fcntl => "fcntl",
        NetSymbol::Poll => "poll",
        NetSymbol::GetAddrInfo => "getaddrinfo",
        NetSymbol::FreeAddrInfo => "freeaddrinfo",
        NetSymbol::SetSockOpt => "setsockopt",
        NetSymbol::GetSockOpt => "getsockopt",
    }
}

/// Write the `events = POLLIN` and zeroed `revents` fields of a pollfd whose fd
/// (8 bytes) has already been stored at `sp + pollfd_offset`. POSIX `struct pollfd`
/// is `{ int fd; short events; short revents }` (events at +4, POLLIN = 1); Windows
/// `WSAPOLLFD` is `{ SOCKET fd; SHORT events; SHORT revents }` — an 8-byte fd, so
/// events sit at +8 and readability is `POLLRDNORM` (0x0100), not POSIX `POLLIN`
/// (plan-47-I). The POSIX arm is byte-identical to the pre-seam inline sequence.
pub(in crate::target::shared::code) fn emit_pollfd_events(
    platform: &dyn CodegenPlatform,
    pollfd_offset: usize,
    instructions: &mut Vec<CodeInstruction>,
) {
    if platform.family() == PlatformFamily::Windows {
        instructions.extend([
            // events (SHORT) @ +8 = POLLRDNORM (0x0100).
            abi::store_u8(abi::ZERO, abi::stack_pointer(), pollfd_offset + 8),
            abi::move_immediate("%v10", "Integer", "1"),
            abi::store_u8("%v10", abi::stack_pointer(), pollfd_offset + 9),
            abi::store_u8(abi::ZERO, abi::stack_pointer(), pollfd_offset + 10),
            abi::store_u8(abi::ZERO, abi::stack_pointer(), pollfd_offset + 11),
        ]);
    } else {
        instructions.extend([
            abi::move_immediate("%v10", "Integer", POLLIN),
            abi::store_u8("%v10", abi::stack_pointer(), pollfd_offset + 4),
            abi::store_u8(abi::ZERO, abi::stack_pointer(), pollfd_offset + 5),
            abi::store_u8(abi::ZERO, abi::stack_pointer(), pollfd_offset + 6),
            abi::store_u8(abi::ZERO, abi::stack_pointer(), pollfd_offset + 7),
        ]);
    }
}

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

/// Emit `bl _mfb_arena_alloc` with the size in `x0` and alignment in `x1`
/// (preset by the caller), then branch to `fail` when allocation fails. On
/// success the block pointer is left in `x1` (`RESULT_VALUE_REGISTER`).

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

/// Emit the shared "build a String result" body (bug-331 §H): allocate `N + 9`
/// bytes, copy the `N` received bytes from `sp + buf_offset` into the new String's
/// data region, NUL-terminate, and call the UTF-8 validator (branching to
/// `encoding_error` on failure). The new String pointer is stored at
/// `sp + str_offset` and left in `%v9`. Offsets and labels are caller-supplied so
/// the emitted bytes match each call site exactly. Clobbers `x0`/`x1`/`%v9`..`%v15`.
#[allow(clippy::too_many_arguments)]
fn emit_string_result_build(
    symbol: &str,
    buf_offset: usize,
    n_offset: usize,
    str_offset: usize,
    str_copy: &str,
    str_done: &str,
    alloc_fail: &str,
    encoding_error: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    instructions.extend([
        abi::load_u64("%v10", abi::stack_pointer(), n_offset),
        abi::add_immediate(abi::return_register(), "%v10", 9),
        abi::move_immediate(abi::ARG[1], "Integer", "8"),
    ]);
    emit_alloc(symbol, instructions, relocations, alloc_fail);
    instructions.extend([
        abi::move_register("%v15", abi::RET[1]), // alloc result -> vreg base (plan-34-B Phase 3)
        abi::load_u64("%v10", abi::stack_pointer(), n_offset),
        abi::store_u64("%v10", "%v15", 0),
        abi::load_u64("%v11", abi::stack_pointer(), buf_offset),
        abi::add_immediate("%v12", "%v15", 8),
        abi::move_immediate("%v13", "Integer", "0"),
        abi::store_u64("%v15", abi::stack_pointer(), str_offset),
        abi::label(str_copy),
        abi::compare_registers("%v13", "%v10"),
        abi::branch_eq(str_done),
        abi::load_u8("%v14", "%v11", 0),
        abi::store_u8("%v14", "%v12", 0),
        abi::add_immediate("%v11", "%v11", 1),
        abi::add_immediate("%v12", "%v12", 1),
        abi::add_immediate("%v13", "%v13", 1),
        abi::branch(str_copy),
        abi::label(str_done),
        abi::store_u8(abi::ZERO, "%v12", 0),
        // validate_utf8(bytes, len)
        abi::load_u64("%v9", abi::stack_pointer(), str_offset),
        abi::add_immediate(abi::return_register(), "%v9", 8),
        abi::load_u64(abi::ARG[1], "%v9", 0),
    ]);
    emit_call_validate_utf8(symbol, encoding_error, instructions, relocations);
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

#[allow(clippy::too_many_arguments)]
/// Build an `Address` record from a `sockaddr` whose pointer lives at
/// `sp + sockaddr_off`. The observed port is read from `sockaddr + 2/3`.
/// `len_off`, `dst_off`, and `host_off` are scratch stack slots. Leaves the
/// `Address` pointer in `x1`, branches to `alloc_fail` on allocation failure or
/// `addr_fail` when `inet_ntop` fails. Everything persists on the stack so no
/// callee-saved registers are clobbered.
fn emit_address_from_sockaddr(
    ctx: &mut EmitCtx,
    prefix: &str,
    sockaddr_off: usize,
    len_off: usize,
    dst_off: usize,
    host_off: usize,
    alloc_fail: &str,
    addr_fail: &str,
) -> Result<(), String> {
    // The three shared refs are `&'a` fields, so reading them out is
    // independent of the `&mut ctx` reference — only the two streams need `ctx.`.
    let symbol = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;
    let count_loop = format!("{symbol}_{prefix}_addr_count");
    let count_done = format!("{symbol}_{prefix}_addr_count_done");
    let copy_loop = format!("{symbol}_{prefix}_addr_copy");
    let copy_done = format!("{symbol}_{prefix}_addr_copy_done");
    // Temp dst buffer for the numeric host string.
    ctx.instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", &ADDR_STR_CAP.to_string()),
        abi::move_immediate(abi::ARG[1], "Integer", "1"),
    ]);
    emit_alloc(symbol, ctx.instructions, ctx.relocations, alloc_fail);
    ctx.instructions.extend([
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
        ctx.instructions,
        ctx.relocations,
    )?;
    ctx.instructions.extend([
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
    emit_alloc(symbol, ctx.instructions, ctx.relocations, alloc_fail);
    ctx.instructions.extend([
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
    emit_alloc(symbol, ctx.instructions, ctx.relocations, alloc_fail);
    ctx.instructions.extend([
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
) -> HelperResult {
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
                                           // bug-261: a non-positive `timeoutMs` no longer blocks indefinitely. Instead
                                           // of a plain blocking connect (which a black-holed peer or a firewall dropping
                                           // SYNs wedges past any reasonable deadline), the non-positive case falls
                                           // through to the same non-blocking-connect + `poll` machinery the positive
                                           // case uses, seeded with this bounded default. 120 s comfortably exceeds any
                                           // real TCP handshake while still bounding the wedge (docs already state the
                                           // default "is not guaranteed to be unbounded").
    const DEFAULT_CONNECT_TIMEOUT_MS: &str = "120000";

    let null_host = format!("{symbol}_null_host");
    let resolved = format!("{symbol}_resolved");
    let resolve_fail = format!("{symbol}_resolve_fail");
    let socket_fail = format!("{symbol}_socket_fail");
    let op_fail = format!("{symbol}_op_fail");
    let connect_use_timeout = format!("{symbol}_connect_use_timeout");
    let nb_connected = format!("{symbol}_nb_connected");
    let connect_poll_retry = format!("{symbol}_connect_poll_retry");
    let connect_poll_ready = format!("{symbol}_connect_poll_ready");
    let connect_timeout = format!("{symbol}_connect_timeout");
    let connect_timeout_ok = format!("{symbol}_connect_timeout_ok");
    let listen_backlog_ok = format!("{symbol}_listen_backlog_ok");
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
    instructions.push(abi::store_u64(
        abi::ZERO,
        abi::stack_pointer(),
        SERVICE_OFFSET,
    ));
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
        net_symbol(platform, NetSymbol::GetAddrInfo),
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
        net_symbol(platform, NetSymbol::Socket),
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        // C `int` return (socket fd) — sign-extend before the signed compare
        // (bug-04/bug-170).
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
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
            net_symbol(platform, NetSymbol::SetSockOpt),
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
            net_symbol(platform, NetSymbol::Bind),
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.extend([
            // C `int` return (bind) — sign-extend before the signed compare
            // (bug-04/bug-170).
            abi::sign_extend_word(abi::return_register(), abi::return_register()),
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_lt(&op_fail),
            // listen(fd, backlog)
            abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
            abi::load_u64(abi::ARG[1], abi::stack_pointer(), EXTRA_OFFSET),
            // Clamp backlog to INT_MAX: listen() takes a C `int`, so a 64-bit value
            // with bit 31 set would be passed as a negative backlog (bug-239).
            abi::move_immediate("%v9", "Integer", "2147483647"),
            abi::compare_registers(abi::ARG[1], "%v9"),
            abi::branch_le(&listen_backlog_ok),
            abi::move_register(abi::ARG[1], "%v9"),
            abi::label(&listen_backlog_ok),
        ]);
        platform.emit_libc_call(
            net_symbol(platform, NetSymbol::Listen),
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.extend([
            // C `int` return (listen) — sign-extend before the signed compare
            // (bug-04/bug-170).
            abi::sign_extend_word(abi::return_register(), abi::return_register()),
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_lt(&op_fail),
        ]);
    } else {
        // Every connect now takes the non-blocking-connect + `poll` path: a
        // positive `timeoutMs` is honored as-is; a non-positive one (including the
        // omitted-argument overload, which passes 0) is replaced with the bounded
        // `DEFAULT_CONNECT_TIMEOUT_MS` so it can no longer wedge the thread forever
        // (bug-261). Blocking mode is restored on success.
        instructions.extend([
            abi::load_u64("%v9", abi::stack_pointer(), EXTRA_OFFSET),
            abi::compare_immediate("%v9", "0"),
            abi::branch_gt(&connect_use_timeout),
            // Non-positive: seed the bounded default deadline, then fall through.
            abi::move_immediate("%v9", "Integer", DEFAULT_CONNECT_TIMEOUT_MS),
            abi::store_u64("%v9", abi::stack_pointer(), EXTRA_OFFSET),
            abi::label(&connect_use_timeout),
        ]);
        if platform.family() != PlatformFamily::Windows {
            // flags = fcntl(fd, F_GETFL, 0). Winsock's ioctlsocket(FIONBIO) is
            // stateless, so Windows skips the read and emit_set_nonblocking ignores
            // FLAGS_OFFSET.
            instructions.extend([
                abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
                abi::move_immediate(abi::ARG[1], "Integer", "3"),
                abi::move_immediate(abi::ARG[2], "Integer", "0"),
            ]);
            platform.emit_variadic_call(
                net_symbol(platform, NetSymbol::Fcntl),
                symbol,
                platform_imports,
                &mut instructions,
                &mut relocations,
            )?;
            instructions.push(abi::store_u64(
                abi::return_register(),
                abi::stack_pointer(),
                FLAGS_OFFSET,
            ));
        }
        // fcntl(fd, F_SETFL, flags | O_NONBLOCK) — Windows: ioctlsocket(fd, FIONBIO, &1)
        platform.emit_set_nonblocking(
            FD_OFFSET,
            FLAGS_OFFSET,
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
            net_symbol(platform, NetSymbol::Connect),
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.extend([
            // C `int` return (connect) — sign-extend before comparing so a success
            // 0 with dirty upper x0 bits is still recognized (bug-04/bug-170).
            abi::sign_extend_word(abi::return_register(), abi::return_register()),
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
            abi::compare_immediate("%v9", platform.socket_in_progress_code()),
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
            // Clamp the connect timeout to INT_MAX: poll() takes a C `int`, so a
            // 64-bit value with bit 31 set would block forever (bug-239).
            abi::move_immediate("%v11", "Integer", "2147483647"),
            abi::compare_registers(abi::ARG[2], "%v11"),
            abi::branch_le(&connect_timeout_ok),
            abi::move_register(abi::ARG[2], "%v11"),
            abi::label(&connect_timeout_ok),
        ]);
        platform.emit_libc_call(
            net_symbol(platform, NetSymbol::Poll),
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.extend([
            // C `int` return (poll) — sign-extend before the signed compares; a -1
            // poll error read as large-positive would wrongly take branch_gt
            // (connect_poll_ready) and treat the socket as writable (bug-04/bug-170).
            abi::sign_extend_word(abi::return_register(), abi::return_register()),
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
            net_symbol(platform, NetSymbol::GetSockOpt),
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.extend([
            // C `int` return (getsockopt) — sign-extend before the signed compare
            // (bug-04/bug-170).
            abi::sign_extend_word(abi::return_register(), abi::return_register()),
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_lt(&op_fail),
            abi::load_u32("%v9", abi::stack_pointer(), SOERR_OFFSET),
            abi::compare_immediate("%v9", "0"),
            abi::branch_ne(&op_fail),
            // Connected: restore blocking mode with fcntl(fd, F_SETFL, flags).
            abi::label(&nb_connected),
        ]);
        if platform.family() == PlatformFamily::Windows {
            // Winsock: ioctlsocket(fd, FIONBIO, &0) — no flags word to restore.
            platform.emit_restore_blocking(
                FD_OFFSET,
                FLAGS_OFFSET,
                symbol,
                platform_imports,
                &mut instructions,
                &mut relocations,
            )?;
        } else {
            instructions.extend([
                abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
                abi::move_immediate(abi::ARG[1], "Integer", "4"),
                abi::load_u64(abi::ARG[2], abi::stack_pointer(), FLAGS_OFFSET),
            ]);
            platform.emit_variadic_call(
                net_symbol(platform, NetSymbol::Fcntl),
                symbol,
                platform_imports,
                &mut instructions,
                &mut relocations,
            )?;
        }
        // Both the caller-timeout and default-timeout connects converge here after
        // restoring blocking mode; the old unbounded blocking-connect path (taken
        // when timeoutMs <= 0) is gone — that case now uses the bounded default
        // deadline above (bug-261).
        instructions.push(abi::label(&connected_done));
    }
    // freeaddrinfo(res)
    instructions.push(abi::load_u64(
        abi::return_register(),
        abi::stack_pointer(),
        RES_OFFSET,
    ));
    platform.emit_libc_call(
        net_symbol(platform, NetSymbol::FreeAddrInfo),
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
    // op_fail / socket_fail: free resources then report network failure. op_fail
    // closes the socket fd (loaded from FD_OFFSET below) before falling through to
    // socket_fail, which frees the addrinfo — so no fd or addrinfo leaks on the
    // error paths (bug-268 / OS-06: the earlier "fd leaks" note was stale).
    instructions.push(abi::label(&op_fail));
    instructions.push(abi::load_u64(
        abi::return_register(),
        abi::stack_pointer(),
        FD_OFFSET,
    ));
    platform.emit_libc_call(
        net_symbol(platform, NetSymbol::Close),
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
        net_symbol(platform, NetSymbol::FreeAddrInfo),
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
        net_symbol(platform, NetSymbol::Close),
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
        net_symbol(platform, NetSymbol::FreeAddrInfo),
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

pub(in crate::target::shared::code) fn lower_net_connect_tcp_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    lower_net_endpoint_helper(symbol, platform_imports, platform, false, false)
}

pub(in crate::target::shared::code) fn lower_net_connect_tcp_addr_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    lower_net_endpoint_helper(symbol, platform_imports, platform, false, true)
}

pub(in crate::target::shared::code) fn lower_net_listen_tcp_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> HelperResult {
    lower_net_endpoint_helper(symbol, platform_imports, platform, true, false)
}
