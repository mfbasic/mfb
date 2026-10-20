//! The shared `net` OS-seam emitters: the DNS-lookup / TCP-endpoint / connect /
//! listen helpers and the socket-call symbol/emitter primitives the sibling
//! [`super::gen_io`] / [`super::gen_poll`] backends share. Each `lower_net_*_helper`
//! emits a self-contained runtime function that marshals libc socket calls and
//! returns the standard `(tag, value)` result in `x0`/`x1`; each `net::` member owns
//! its `Body::abi_function` body in its own `func_*.rs`, which calls the matching
//! `lower_net_*_helper` (with any bool/alias discriminant) and finalizes.
//!
//! Socket and listener handles share the `File` record layout (`fd` at offset
//! 8, a `closed` flag at offset 16). Platform `sockaddr` structures are produced
//! by `getaddrinfo` so the helpers never hand-build a `sockaddr_in`; the only
//! field written directly is `sin_port` at offset 2, which is consistent across
//! platforms for `AF_INET`.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::error::emission::*;
use crate::codegen::memory::arena::{emit_helper_scratch_release, HelperScratch};
use crate::codegen::memory::marshal::{
    emit_build_inlined_record_sized, MarshalRegs, RecordBuildScratch,
};
use crate::codegen::os::syscall::*;
use crate::codegen::string::validate::*;

use std::collections::HashMap;

use crate::target::shared::abi;
use crate::types::ParameterType;

mod endpoint;
pub(crate) use endpoint::{
    lower_net_connect_tcp_addr_helper, lower_net_connect_tcp_helper, lower_net_listen_tcp_helper,
};
/// The socket-call symbols the shared `net` lowering issues. Every hardcoded
/// libc symbol literal routes through [`net_symbol`] so a platform whose socket
/// ABI diverges from POSIX (Windows/Winsock) can rename it in one place instead
/// of at 35 call sites. Names mirror the POSIX symbol they map to on every
/// non-Windows target (plan-47-I I1).
#[derive(Clone, Copy)]
pub(crate) enum NetSymbol {
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
pub(crate) fn net_symbol(platform: &dyn CodegenPlatform, intent: NetSymbol) -> &'static str {
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

/// Write the `events` and zeroed `revents` fields of a pollfd whose fd (8 bytes)
/// has already been stored at `sp + pollfd_offset`. POSIX `struct pollfd` is
/// `{ int fd; short events; short revents }` (events at +4, POLLIN = 1,
/// POLLOUT = 4); Windows `WSAPOLLFD` is `{ SOCKET fd; SHORT events; SHORT
/// revents }` — an 8-byte fd, so events sit at +8, readability is `POLLRDNORM`
/// (0x0100) rather than POSIX `POLLIN`, and writability is `POLLWRNORM`
/// (0x0010) rather than POSIX `POLLOUT` (plan-47-I). The POSIX arms are
/// byte-identical to the pre-seam inline sequences.
pub(crate) fn emit_pollfd_events(
    platform: &dyn CodegenPlatform,
    pollfd_offset: usize,
    instructions: &mut Vec<CodeInstruction>,
    vregs: &mut Vregs,
) {
    emit_pollfd_events_for(platform, pollfd_offset, false, instructions, vregs)
}

/// [`emit_pollfd_events`] with an explicit direction: `writable` selects
/// POLLOUT/`POLLWRNORM` (the non-blocking connect wait) over POLLIN/`POLLRDNORM`
/// (a readability query).
pub(crate) fn emit_pollfd_events_for(
    platform: &dyn CodegenPlatform,
    pollfd_offset: usize,
    writable: bool,
    instructions: &mut Vec<CodeInstruction>,
    vregs: &mut Vregs,
) {
    let v10 = vregs.next();
    if platform.family() == PlatformFamily::Windows {
        // POLLRDNORM = 0x0100 (high byte 1), POLLWRNORM = 0x0010 (low byte 0x10).
        let (low, high) = if writable { ("16", "0") } else { ("0", "1") };
        instructions.extend([
            abi::move_immediate(&v10, "Integer", low),
            abi::store_u8(&v10, abi::stack_pointer(), pollfd_offset + 8),
            abi::move_immediate(&v10, "Integer", high),
            abi::store_u8(&v10, abi::stack_pointer(), pollfd_offset + 9),
            abi::store_u8(abi::ZERO, abi::stack_pointer(), pollfd_offset + 10),
            abi::store_u8(abi::ZERO, abi::stack_pointer(), pollfd_offset + 11),
        ]);
    } else {
        instructions.extend([
            abi::move_immediate(&v10, "Integer", if writable { POLLOUT } else { POLLIN }),
            abi::store_u8(&v10, abi::stack_pointer(), pollfd_offset + 4),
            abi::store_u8(abi::ZERO, abi::stack_pointer(), pollfd_offset + 5),
            abi::store_u8(abi::ZERO, abi::stack_pointer(), pollfd_offset + 6),
            abi::store_u8(abi::ZERO, abi::stack_pointer(), pollfd_offset + 7),
        ]);
    }
}

pub(crate) const AF_INET: &str = "2";
pub(crate) const SOCK_STREAM: &str = "1";
pub(crate) const SOCK_DGRAM: &str = "2";
/// Linux `SOCK_CLOEXEC` (== `O_CLOEXEC`, 0x80000 — the same bit on x86-64,
/// AArch64 and RISC-V), bug-499. Also the flag `accept4(2)` takes.
pub(crate) const LINUX_SOCK_CLOEXEC: &str = "524288";

/// bug-499: make the socket about to be created close-on-exec, so a
/// `process::spawn` child never inherits it. Call with the `socket(2)` arguments
/// already staged (`type` in `c_arg(1)`): on Linux this ORs `SOCK_CLOEXEC` into
/// the type, setting the flag atomically with creation (`c_arg(3)`, unused by the
/// three-argument call, is the scratch). macOS has no `SOCK_CLOEXEC`, so this emits
/// nothing there and [`emit_fd_cloexec_fallback`] sets `FD_CLOEXEC` right after
/// the call. Windows emits nothing: a SOCKET is a handle, and the Windows spawn
/// hands the child an explicit handle list carrying only its stdio.
pub(crate) fn emit_socket_type_cloexec(
    platform: &dyn CodegenPlatform,
    instructions: &mut Vec<CodeInstruction>,
) {
    if platform.family() == PlatformFamily::Linux {
        instructions.extend([
            abi::move_immediate(abi::c_arg(3), "Integer", LINUX_SOCK_CLOEXEC),
            abi::or_registers(abi::c_arg(1), abi::c_arg(1), abi::c_arg(3)),
        ]);
    }
}

/// bug-499: the macOS half of close-on-exec for a descriptor whose creating call
/// cannot set it atomically — `fcntl(fd, F_SETFD, FD_CLOEXEC)` on the fd stored at
/// `sp + fd_slot`. Emits nothing on Linux (`SOCK_CLOEXEC` / `accept4` already set
/// it) or Windows (handle list). Clobbers the C argument/return registers; every
/// caller reloads the fd from its slot afterwards, as the sites already do.
pub(crate) fn emit_fd_cloexec_fallback(
    platform: &dyn CodegenPlatform,
    symbol: &str,
    fd_slot: usize,
    platform_imports: &HashMap<String, String>,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    if platform.family() != PlatformFamily::MacOS {
        return Ok(());
    }
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), fd_slot),
        abi::move_immediate(abi::c_arg(1), "Integer", "2"), // F_SETFD
        abi::move_immediate(abi::c_arg(2), "Integer", "1"), // FD_CLOEXEC
    ]);
    platform.emit_variadic_external_call(
        "fcntl",
        symbol,
        platform_imports,
        instructions,
        relocations,
    )
}

/// bug-499: emit the accept call for a listener fd already staged as the first
/// argument with NULL address arguments: Linux `accept4(fd, NULL, NULL,
/// SOCK_CLOEXEC)` so the accepted socket is close-on-exec from birth; every other
/// platform the plain `accept(fd, NULL, NULL)` (macOS follows up with
/// [`emit_fd_cloexec_fallback`] once the fd is stored; Windows relies on the
/// spawn's handle list). The C `int` result is left where `accept` leaves it.
pub(crate) fn emit_accept_call(
    platform: &dyn CodegenPlatform,
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    if platform.family() == PlatformFamily::Linux {
        instructions.push(abi::move_immediate(
            abi::c_arg(3),
            "Integer",
            LINUX_SOCK_CLOEXEC,
        ));
        return platform.emit_external_call(
            "accept4",
            symbol,
            platform_imports,
            instructions,
            relocations,
        );
    }
    platform.emit_external_call(
        net_symbol(platform, NetSymbol::Accept),
        symbol,
        platform_imports,
        instructions,
        relocations,
    )
}
// hints `u64` at offset 0 packs `ai_flags` (low 32) and `ai_family` (high 32).
// `AF_INET (2) << 32`.
const HINTS_FAMILY_WORD: &str = "8589934592"; // ai_flags = 0
const HINTS_FAMILY_WORD_PASSIVE: &str = "8589934593"; // ai_flags = AI_PASSIVE (1)
pub(crate) const SOCKADDR_STORAGE_SIZE: usize = 128;
const ADDR_STR_CAP: usize = 64;
pub(crate) const POLLIN: &str = "1";
/// POSIX `POLLOUT` — the writability bit the non-blocking connect wait polls for.
pub(crate) const POLLOUT: &str = "4";
/// `EINTR` errno (Linux/macOS both use 4): a `poll` interrupted by a signal
/// returns `-1`/`EINTR` and must be re-issued rather than treated as a hard
/// connect failure (bug-115).

// `emit_alloc` (used below) is the shared arena allocator emitter reached via the
// `*` glob; it emits `bl _mfb_arena_alloc` with the size
// in `x0`/alignment in `x1` and leaves the block pointer in `x1` on success.

/// Emit the shared "build a String result" body (bug-331 §H): allocate `N + 9`
/// bytes, copy the `N` received bytes from `sp + buf_offset` into the new String's
/// data region, NUL-terminate, and call the UTF-8 validator (branching to
/// `encoding_error` on failure). The new String pointer is stored at
/// `sp + str_offset` and left in `%v9`. Offsets and labels are caller-supplied so
/// the emitted bytes match each call site exactly. Clobbers `x0`/`x1`/`%v9`..`%v15`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_string_result_build(
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
    // Self-contained scratch minter (plan-00-G): this emitter has non-net callers
    // (process/tls) so its signature stays fixed; no minted vreg here is held live
    // across the internal emit_alloc / validate_utf8 calls (each reloads from the
    // stack), so an independent minter cannot collide with a caller's live vregs.
    let mut vregs = Vregs::new();
    let v9 = vregs.next();
    let v10 = vregs.next();
    let v11 = vregs.next();
    let v12 = vregs.next();
    let v13 = vregs.next();
    let v14 = vregs.next();
    let v15 = vregs.next();
    instructions.extend([
        abi::load_u64(&v10, abi::stack_pointer(), n_offset),
        abi::add_immediate(abi::return_register(), &v10, 9),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, instructions, relocations, alloc_fail);
    instructions.extend([
        abi::move_register(&v15, abi::mfb_return(1)), // alloc result -> vreg base (plan-34-B Phase 3)
        abi::load_u64(&v10, abi::stack_pointer(), n_offset),
        abi::store_u64(&v10, &v15, 0),
        abi::load_u64(&v11, abi::stack_pointer(), buf_offset),
        abi::add_immediate(&v12, &v15, 8),
        abi::move_immediate(&v13, "Integer", "0"),
        abi::store_u64(&v15, abi::stack_pointer(), str_offset),
        abi::label(str_copy),
        abi::compare_registers(&v13, &v10),
        abi::branch_eq(str_done),
        abi::load_u8(&v14, &v11, 0),
        abi::store_u8(&v14, &v12, 0),
        abi::add_immediate(&v11, &v11, 1),
        abi::add_immediate(&v12, &v12, 1),
        abi::add_immediate(&v13, &v13, 1),
        abi::branch(str_copy),
        abi::label(str_done),
        abi::store_u8(abi::ZERO, &v12, 0),
        // validate_utf8(bytes, len)
        abi::load_u64(&v9, abi::stack_pointer(), str_offset),
        abi::add_immediate(abi::return_register(), &v9, 8),
        abi::load_u64(abi::c_arg(1), &v9, 0),
    ]);
    emit_call_validate_utf8(symbol, encoding_error, instructions, relocations);
}

/// Copy a NUL-free MFBASIC `String` (pointer at `sp + str_off`) into a freshly
/// allocated NUL-terminated C string, storing the result pointer at
/// `sp + out_off`. Branches to `alloc_fail` on allocation failure. Clobbers
/// `x0`, `x1`, `x9`..`x14`.
///
/// bug-574: the block is the CALLER's to release. It is the helper's own
/// marshalling scratch — handed to `getaddrinfo`/`bind`/`sendto` and never
/// returned to MFBASIC — so no caller-side ownership analysis can see it, and
/// every `net::`/`udp::`/`tcp::` call leaked its host argument, proportionally to
/// that argument's length. The returned [`HelperScratch`] is what
/// `emit_helper_scratch_release` frees at the helper's `done`; the null-init that
/// makes that free safe on the `alloc_fail` path is emitted HERE, ahead of the
/// allocation, so it cannot be forgotten at a call site.
///
/// The pointer is ALSO left in `sp + out_off` exactly as before — the release
/// reads the vreg pair, so the existing stack-slot readers are untouched.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_cstring(
    symbol: &str,
    prefix: &str,
    str_off: usize,
    out_off: usize,
    alloc_fail: &str,
    scratch: &HelperScratch,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    vregs: &mut Vregs,
) {
    let v9 = vregs.next();
    let v10 = vregs.next();
    let v11 = vregs.next();
    let v12 = vregs.next();
    let v13 = vregs.next();
    let v14 = vregs.next();
    let size = &scratch.size;
    let copy_loop = format!("{symbol}_{prefix}_cstr_copy");
    let copy_done = format!("{symbol}_{prefix}_cstr_done");
    instructions.extend([
        abi::load_u64(&v9, abi::stack_pointer(), str_off),
        abi::load_u64(&v10, &v9, 0),
        abi::add_immediate(size, &v10, 1),
        abi::move_register(abi::return_register(), size),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
    ]);
    emit_alloc(symbol, instructions, relocations, alloc_fail);
    instructions.extend([
        abi::move_register(&scratch.pointer, abi::mfb_return(1)),
        abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), out_off),
        abi::load_u64(&v9, abi::stack_pointer(), str_off),
        abi::load_u64(&v10, &v9, 0),
        abi::add_immediate(&v11, &v9, 8),
        abi::move_register(&v12, abi::mfb_return(1)),
        abi::move_immediate(&v13, "Integer", "0"),
        abi::label(&copy_loop),
        abi::compare_registers(&v13, &v10),
        abi::branch_eq(&copy_done),
        abi::load_u8(&v14, &v11, 0),
        abi::store_u8(&v14, &v12, 0),
        abi::add_immediate(&v11, &v11, 1),
        abi::add_immediate(&v12, &v12, 1),
        abi::add_immediate(&v13, &v13, 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::store_u8(abi::ZERO, &v12, 0),
    ]);
}

/// bug-876: a port is an unsigned 16-bit number. Load the `Integer` port saved at
/// `sp + port_off` and branch to `invalid` when it lies outside `0..=65535`
/// (negative values have the sign bit set, so they fail the same `>> 16` test).
/// Every host/port endpoint calls this BEFORE `getaddrinfo`, so the later
/// two-byte `sin_port` store never discards high bits. Clobbers two vregs.
pub(crate) fn emit_port_range_check(
    port_off: usize,
    invalid: &str,
    instructions: &mut Vec<CodeInstruction>,
    vregs: &mut Vregs,
) {
    let port = vregs.next();
    let high = vregs.next();
    instructions.extend([
        abi::load_u64(&port, abi::stack_pointer(), port_off),
        abi::shift_right_immediate(&high, &port, 16),
        abi::compare_immediate(&high, "0"),
        abi::branch_ne(invalid),
    ]);
}

/// Zero a 48-byte `addrinfo` hints block at `sp + hints_off` and set
/// `ai_family = AF_INET`, `ai_socktype = socktype` (and `AI_PASSIVE` when
/// `passive`). Clobbers `x9`.
pub(crate) fn emit_hints(
    hints_off: usize,
    passive: bool,
    socktype: &str,
    instructions: &mut Vec<CodeInstruction>,
    vregs: &mut Vregs,
) {
    let v9 = vregs.next();
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
        abi::move_immediate(&v9, "Integer", family_word),
        abi::store_u64(&v9, abi::stack_pointer(), hints_off),
        abi::move_immediate(&v9, "Integer", socktype),
        abi::store_u64(&v9, abi::stack_pointer(), hints_off + 8),
    ]);
}

/// The seven scratch vregs the two `net::Address` builders share, allocated in
/// one place and in one order so both name the same register for the same role.
struct AddrVregs {
    /// Walking cursor while measuring the host string. The `sockaddr` builder
    /// reuses it to hold the `sockaddr` pointer while decoding the port.
    cursor: VirtualRegister,
    /// The measured host length; reused as the port's high byte.
    len: VirtualRegister,
    /// Copy source cursor; reused as the port's low byte.
    src: VirtualRegister,
    /// Copy destination cursor.
    dst: VirtualRegister,
    /// Copy loop index.
    idx: VirtualRegister,
    /// The byte in flight.
    byte: VirtualRegister,
    /// The scratch MFBASIC `String` block the record copies the host from.
    string: VirtualRegister,
}

impl AddrVregs {
    fn new(vregs: &mut Vregs) -> Self {
        Self {
            cursor: vregs.next(),
            len: vregs.next(),
            src: vregs.next(),
            dst: vregs.next(),
            idx: vregs.next(),
            byte: vregs.next(),
            string: vregs.next(),
        }
    }
}

/// The frame slots a `net::Address` builder writes — all the caller's, all
/// distinct.
///
/// After a build, `record.result` holds the record pointer (also left in `x1`) and
/// `record.size` its byte size, which a caller that inlines the record into a
/// `udp::Datagram` or a `net::PingResult`, embeds it in a list, or frees it, needs.
pub(crate) struct AddressSlots {
    /// The measured host length.
    pub(crate) len: usize,
    /// The scratch host `String`, freed once the record holds its own copy.
    pub(crate) host: usize,
    /// The port the record is built with.
    pub(crate) port: usize,
    /// The record marshaller's scratch.
    pub(crate) record: RecordBuildScratch,
}

/// The type every `net::Address` builder constructs.
pub(crate) fn address_type() -> ParameterType {
    ParameterType::named(crate::codegen::builtins::net::ADDRESS_TYPE_ID)
}

/// The shared tail of both `net::Address` builders: measure the NUL-terminated
/// host string whose pointer sits at `sp + src_off`, copy it into a scratch
/// MFBASIC `String`, build the spec-canonical `Address` record from that `String`
/// and the port at `slots.port` — the host inlined into the record's data region,
/// exactly as a source-built record (plan-132) — and free the scratch `String`.
///
/// The record is left in `x1`, its pointer in `slots.record.result` and its byte
/// size in `slots.record.size`.
fn emit_address_host_and_record(
    ctx: &mut EmitCtx,
    prefix: &str,
    src_off: usize,
    slots: &AddressSlots,
    alloc_fail: &str,
    v: &AddrVregs,
    vregs: &mut Vregs,
) -> Result<(), String> {
    let symbol = ctx.symbol;
    let count_loop = format!("{symbol}_{prefix}_addr_count");
    let count_done = format!("{symbol}_{prefix}_addr_count_done");
    let copy_loop = format!("{symbol}_{prefix}_addr_copy");
    let copy_done = format!("{symbol}_{prefix}_addr_copy_done");
    ctx.instructions.extend([
        // Count the NUL-terminated host string length.
        abi::load_u64(&v.cursor, abi::stack_pointer(), src_off),
        abi::move_immediate(&v.len, "Integer", "0"),
        abi::label(&count_loop),
        abi::load_u8(&v.src, &v.cursor, 0),
        abi::compare_immediate(&v.src, "0"),
        abi::branch_eq(&count_done),
        abi::add_immediate(&v.cursor, &v.cursor, 1),
        abi::add_immediate(&v.len, &v.len, 1),
        abi::branch(&count_loop),
        abi::label(&count_done),
        abi::store_u64(&v.len, abi::stack_pointer(), slots.len),
        // Allocate the scratch host String: [u64 len][bytes][nul].
        abi::add_immediate(abi::return_register(), &v.len, 9),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, ctx.instructions, ctx.relocations, alloc_fail);
    ctx.instructions.extend([
        abi::move_register(&v.string, abi::mfb_return(1)), // alloc result → vreg (plan-34-B Phase 3)
        abi::load_u64(&v.len, abi::stack_pointer(), slots.len),
        abi::store_u64(&v.len, &v.string, 0),
        abi::store_u64(&v.string, abi::stack_pointer(), slots.host),
        abi::load_u64(&v.src, abi::stack_pointer(), src_off),
        abi::add_immediate(&v.dst, &v.string, 8),
        abi::move_immediate(&v.idx, "Integer", "0"),
        abi::label(&copy_loop),
        abi::compare_registers(&v.idx, &v.len),
        abi::branch_eq(&copy_done),
        abi::load_u8(&v.byte, &v.src, 0),
        abi::store_u8(&v.byte, &v.dst, 0),
        abi::add_immediate(&v.src, &v.src, 1),
        abi::add_immediate(&v.dst, &v.dst, 1),
        abi::add_immediate(&v.idx, &v.idx, 1),
        abi::branch(&copy_loop),
        abi::label(&copy_done),
        abi::store_u8(abi::ZERO, &v.dst, 0),
    ]);
    emit_build_inlined_record_sized(
        symbol,
        &format!("{prefix}_addr"),
        &address_type(),
        TypeModel::builtin_records(),
        &[slots.host, slots.port],
        &[None, None],
        &slots.record,
        &MarshalRegs::fresh(vregs),
        abi::mfb_return(1),
        alloc_fail,
        ctx.instructions,
        ctx.relocations,
    )?;
    // The record now holds its own copy of the host bytes, so the scratch `String`
    // is garbage, and nothing but this builder ever held it. `len + 9` is the size
    // it was allocated with. `_mfb_arena_free` clobbers `x1`; reload the record.
    ctx.instructions.extend([
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), slots.len),
        abi::add_immediate(abi::c_arg(1), abi::c_arg(1), 9),
        abi::load_u64(abi::c_arg(0), abi::stack_pointer(), slots.host),
    ]);
    emit_arena_free(symbol, ctx.instructions, ctx.relocations);
    ctx.instructions.push(abi::load_u64(
        abi::mfb_return(1),
        abi::stack_pointer(),
        slots.record.result,
    ));
    Ok(())
}

/// Build an `Address` record from a host name and port the caller already holds:
/// a NUL-terminated `char *` at `sp + host_cstr_off` and a host-order port at
/// `sp + slots.port`. The `Address` pointer is left in `x1`, and in
/// `slots.record.result` with its size in `slots.record.size`.
///
/// The sibling of [`emit_address_from_sockaddr`], for the one handle that has no
/// descriptor to ask: a macOS TLS `Listener` holds a Network.framework
/// `nw_listener`, which `getsockname` cannot see, so its bound address is
/// assembled from the host it was created with plus `nw_listener_get_port`
/// (bug-465). Both builders emit the identical record, so every package renders
/// an endpoint the same way.
pub(crate) fn emit_address_from_host_and_port(
    ctx: &mut EmitCtx,
    prefix: &str,
    host_cstr_off: usize,
    slots: &AddressSlots,
    alloc_fail: &str,
    vregs: &mut Vregs,
) -> Result<(), String> {
    let v = AddrVregs::new(vregs);
    emit_address_host_and_record(ctx, prefix, host_cstr_off, slots, alloc_fail, &v, vregs)
}

#[allow(clippy::too_many_arguments)]
/// Build an `Address` record from a `sockaddr` whose pointer lives at
/// `sp + sockaddr_off`. The observed port is read from `sockaddr + 2/3`.
/// `dst_off` is a scratch slot for the `inet_ntop` buffer. Leaves the `Address`
/// pointer in `x1` and in `slots.record.result`, its size in `slots.record.size`;
/// branches to `alloc_fail` on allocation failure or `addr_fail` when `inet_ntop`
/// fails. Everything persists on the stack so no callee-saved registers are
/// clobbered.
pub(crate) fn emit_address_from_sockaddr(
    ctx: &mut EmitCtx,
    prefix: &str,
    sockaddr_off: usize,
    dst_off: usize,
    slots: &AddressSlots,
    alloc_fail: &str,
    addr_fail: &str,
    vregs: &mut Vregs,
) -> Result<(), String> {
    // The three shared refs are `&'a` fields, so reading them out is
    // independent of the `&mut ctx` reference — only the two streams need `ctx.`.
    let symbol = ctx.symbol;
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;
    let v = AddrVregs::new(vregs);
    // Temp dst buffer for the numeric host string.
    ctx.instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", &ADDR_STR_CAP.to_string()),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
    ]);
    emit_alloc(symbol, ctx.instructions, ctx.relocations, alloc_fail);
    ctx.instructions.extend([
        abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), dst_off),
        // inet_ntop(AF_INET, sockaddr + 4, dst, ADDR_STR_CAP)
        abi::move_immediate(abi::return_register(), "Integer", AF_INET),
        abi::load_u64(&v.cursor, abi::stack_pointer(), sockaddr_off),
        abi::add_immediate(abi::c_arg(1), &v.cursor, 4),
        abi::load_u64(abi::c_arg(2), abi::stack_pointer(), dst_off),
        abi::move_immediate(abi::c_arg(3), "Integer", &ADDR_STR_CAP.to_string()),
    ]);
    platform.emit_external_call(
        "inet_ntop",
        symbol,
        platform_imports,
        ctx.instructions,
        ctx.relocations,
    )?;
    ctx.instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_eq(addr_fail),
    ]);
    // port = (sockaddr[2] << 8) | sockaddr[3], parked in its slot for the build.
    ctx.instructions.extend([
        abi::load_u64(&v.cursor, abi::stack_pointer(), sockaddr_off),
        abi::load_u8(&v.len, &v.cursor, 2),
        abi::load_u8(&v.src, &v.cursor, 3),
        abi::shift_left_immediate(&v.len, &v.len, 8),
        abi::or_registers(&v.len, &v.len, &v.src),
        abi::store_u64(&v.len, abi::stack_pointer(), slots.port),
    ]);
    emit_address_host_and_record(ctx, prefix, dst_off, slots, alloc_fail, &v, vregs)?;
    // bug-599: the `inet_ntop` buffer is this builder's own scratch. Its bytes were
    // copied into the host `String`, and nothing — not the record, not any caller
    // (every `dst_off` is declared a scratch slot and read nowhere else) — holds its
    // pointer, so it was orphaned on every address built: 64 bytes per
    // `net::lookup` element, `tcp`/`udp`/`tls::localAddress`, `udp::receive` and
    // `net::ping`. Free it with the size it was allocated with.
    ctx.instructions.extend([
        abi::load_u64(abi::c_arg(0), abi::stack_pointer(), dst_off),
        abi::move_immediate(abi::c_arg(1), "Integer", &ADDR_STR_CAP.to_string()),
    ]);
    emit_arena_free(symbol, ctx.instructions, ctx.relocations);
    // Every caller reads the record from `x1`, which the free clobbered.
    ctx.instructions.push(abi::load_u64(
        abi::mfb_return(1),
        abi::stack_pointer(),
        slots.record.result,
    ));
    Ok(())
}

/// Allocate a socket/listener handle record (the canonical plan-80 envelope)
/// from the file descriptor in `x9`, leaving the record pointer in `x1`. Writes
/// the plan-80 header { tag, fd (handle), closed=0, STATE=0 }; `tag` is the
/// caller's `RESOURCE_TAG_*` (Socket / UdpSocket / Listener). Branches to
/// `alloc_fail` on failure.
pub(crate) fn emit_make_handle(
    symbol: &str,
    fd_off: usize,
    tag: &str,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    alloc_fail: &str,
    vregs: &mut Vregs,
) {
    let v9 = vregs.next();
    let v10 = vregs.next();
    instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", RESOURCE_RECORD_SIZE),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, instructions, relocations, alloc_fail);
    instructions.extend([
        abi::move_register(&v10, abi::mfb_return(1)), // alloc result → vreg base; x1 stays the returned ptr
        abi::move_immediate(&v9, "Integer", tag),
        abi::store_u64(&v9, &v10, RESOURCE_OFFSET_TAG),
        abi::load_u64(&v9, abi::stack_pointer(), fd_off),
        abi::store_u64(&v9, &v10, FILE_OFFSET_FD),
        abi::store_u64(abi::ZERO, &v10, FILE_OFFSET_CLOSED),
        // bug-755: a fresh record is unowned until a binding claims it.
        abi::store_u64(abi::ZERO, &v10, RESOURCE_OFFSET_OWNER),
        abi::store_u64(abi::ZERO, &v10, FILE_OFFSET_STATE),
    ]);
}

/// The `(instructions, relocations, stack_size)` a `net` OS-seam body emits before the
/// `abi_function` wrapper finalizes it — the successor to the finalized `HelperResult`
/// tuple (see `fs`'s `FsBodyParts` / `audio`'s `AudioBodyParts`). `stack_size` is the
/// sp-relative locals region the body reserves; the wrapper passes it to
/// `finalize_vreg_body_with_locals`, byte-identical to the body's former self-finalize.
pub(crate) type NetBodyParts = (Vec<CodeInstruction>, Vec<CodeRelocation>, usize);

/// The `void` result every native `net.*` member returns from its per-member
/// `abi_function` body: every net body emits its own fallible ABI, so the wrapper
/// appends no epilogue. `type_` is `Nothing`; `text` carries the runtime-call name.
pub(crate) fn void_result(call: &str) -> ValueResult {
    ValueResult {
        origin: None,
        type_: ParameterType::Nothing,
        location: Operand::from("void"),
        text: call.to_string(),
    }
}

// --- moved from builtins/net/gen_io.rs by plan-110-E Phase 3 ---------------
// `lower_net_address_helper` serves tcp, udp AND tls, so it belongs with the
// other shared primitives rather than in any one package.

pub(crate) fn lower_net_address_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    remote: bool,
) -> Result<NetBodyParts, String> {
    const FRAME_SIZE: usize = 256;
    const FD_OFFSET: usize = 8;
    const LEN_OFFSET: usize = 16;
    const DST_OFFSET: usize = 24;
    const HOST_OFFSET: usize = 32;
    const SADDR_PTR_OFFSET: usize = 40;
    const HOSTLEN_OFFSET: usize = 48;
    const ADDR_OFFSET: usize = 64; // 64..192 sockaddr_storage
    const APORT_OFFSET: usize = 192; // the port the Address is built with
    const ASIZE_OFFSET: usize = 200; // the Address's byte size
    const ARESULT_OFFSET: usize = 208; // the built Address
    const ACURSOR_OFFSET: usize = 216; // record marshaller scratch
    const ABLOCK_OFFSET: usize = 224; // record marshaller scratch

    let closed = format!("{symbol}_closed");
    let closed_moved = format!("{symbol}_closed_moved");
    let name_fail = format!("{symbol}_name_fail");
    let addr_fail = format!("{symbol}_addr_fail");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let done = format!("{symbol}_done");

    let mut instructions: Vec<CodeInstruction> = Vec::new();
    let mut relocations = Vec::new();
    let mut vregs = Vregs::new();
    let v9 = vregs.next();
    let v10 = vregs.next();
    instructions.extend([abi::load_u64(
        &v9,
        abi::return_register(),
        FILE_OFFSET_CLOSED,
    )]);
    instructions.extend(crate::codegen::resource::closed_or_moved_guard(
        &v9,
        &closed,
        &closed_moved,
    ));
    instructions.extend([
        abi::load_u64(&v9, abi::return_register(), FILE_OFFSET_FD),
        abi::store_u64(&v9, abi::stack_pointer(), FD_OFFSET),
        abi::move_immediate(&v10, "Integer", &SOCKADDR_STORAGE_SIZE.to_string()),
        abi::store_u64(&v10, abi::stack_pointer(), LEN_OFFSET),
        abi::move_register(abi::return_register(), &v9),
        abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), ADDR_OFFSET),
        abi::add_immediate(abi::c_arg(2), abi::stack_pointer(), LEN_OFFSET),
    ]);
    let call = if remote { "getpeername" } else { "getsockname" };
    platform.emit_external_call(
        call,
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        // C `int` return (getpeername/getsockname) — sign-extend before the signed
        // compare (bug-04/bug-170).
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&name_fail),
        abi::add_immediate(&v9, abi::stack_pointer(), ADDR_OFFSET),
        abi::store_u64(&v9, abi::stack_pointer(), SADDR_PTR_OFFSET),
    ]);
    emit_address_from_sockaddr(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "addr",
        SADDR_PTR_OFFSET,
        DST_OFFSET,
        &AddressSlots {
            len: HOSTLEN_OFFSET,
            host: HOST_OFFSET,
            port: APORT_OFFSET,
            record: RecordBuildScratch {
                size: ASIZE_OFFSET,
                result: ARESULT_OFFSET,
                cursor: ACURSOR_OFFSET,
                block_size: ABLOCK_OFFSET,
            },
        },
        &alloc_fail,
        &addr_fail,
        &mut vregs,
    )?;
    instructions.extend([
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&name_fail),
    ]);
    emit_fail(
        symbol,
        "ErrResourceClosed",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&addr_fail));
    emit_fail(
        symbol,
        "ErrAddressInvalid",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&closed));
    emit_fail(
        symbol,
        "ErrResourceClosed",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&closed_moved));
    emit_fail(
        symbol,
        "ErrResourceMoved",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&alloc_fail));
    emit_fail(
        symbol,
        "ErrOutOfMemory",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.extend([abi::label(&done), abi::return_()]);
    {
        Ok((instructions, relocations, FRAME_SIZE))
    }
}

// ---------------------------------------------------------------------------
// net.read / net.readText
// ---------------------------------------------------------------------------
