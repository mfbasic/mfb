//! The `net.ping` OS seam — real ICMP echo (plan-110-A).
//!
//! Three backends, not the two the plan first assumed. `scripts/icmp-capability-probe.c`
//! was run on macOS AArch64, Alpine x86_64/riscv64 (musl), Debian x86_64 and Kali
//! AArch64 (glibc); macOS and Linux disagree on every fact the parser depends on
//! (plan-110-A §Corrections C1):
//!
//! | | macOS | Linux |
//! |---|---|---|
//! | reply buffer | IPv4 header **present** | bare ICMP message |
//! | reply TTL | IP header byte 8 | `IP_RECVTTL` cmsg (`recvmsg`) |
//! | echo id | preserved | **rewritten by the kernel** |
//! | socket demux | **promiscuous** — every ICMP socket sees every reply | per-socket |
//!
//! Consequences encoded below:
//!
//! * macOS matches a reply on **id *and* sequence**, because another process's ping
//!   is delivered to our socket too. Linux cannot match on the id (the kernel
//!   replaced it) and does not need to (the kernel already demultiplexed), so it
//!   matches on sequence alone.
//! * macOS uses `recvfrom`; Linux uses `recvmsg` purely to reach the TTL control
//!   message. macOS supports `IP_RECVTTL` as well, but a shared `recvmsg` path would
//!   not be simpler — `msghdr`/`cmsghdr` have different layouts on the two systems,
//!   so it would need per-family offsets anyway, and macOS has the TTL in the buffer
//!   for free.
//! * Windows has no ICMP socket at all without Administrator; it goes through
//!   `iphlpapi`'s `IcmpCreateFile`/`IcmpSendEcho`, which builds and matches the packet
//!   itself. That backend is [`lower_ping_windows`].
//!
//! An unmatched packet does **not** end the call: the receive loop keeps polling
//! against the deadline, which is what makes the macOS promiscuity survivable.
//!
//! Every numeric socket option and clock id here comes from a `CodegenPlatform`
//! accessor rather than a literal, because all of them differ between macOS and
//! Linux (`IP_TTL` 4 vs 2, `IP_RECVTTL` 24 vs 12, `CLOCK_MONOTONIC` 6 vs 1) and a
//! wrong value fails silently on a target this host cannot execute.
//!
//! Register discipline follows the rest of `net`: a value that must survive an
//! external call lives in a stack slot, never in a vreg.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::error::emission::*;
use crate::codegen::os::syscall::*;

use std::collections::HashMap;

use crate::target::shared::abi;

use crate::codegen::os::socket::shared::{
    emit_address_from_sockaddr, emit_cstring, emit_fd_cloexec_fallback, emit_hints,
    emit_pollfd_events, emit_socket_type_cloexec, net_symbol, NetBodyParts, NetSymbol, AF_INET,
    SOCKADDR_STORAGE_SIZE, SOCK_DGRAM,
};

/// `IPPROTO_ICMP` — 1 on every supported platform (measured).
const IPPROTO_ICMP: &str = "1";

/// ICMP message types the parser recognizes.
const ICMP_ECHO_REQUEST: &str = "8";
const ICMP_ECHO_REPLY: &str = "0";
const ICMP_DEST_UNREACHABLE: &str = "3";
const ICMP_TIME_EXCEEDED: &str = "11";

/// `PingStatus` ordinals — the enum's declaration order in `net::register`. An enum
/// value is its ordinal `Integer` at run time.
const STATUS_OK: &str = "0";
const STATUS_TIMEOUT: &str = "1";
const STATUS_UNREACHABLE: &str = "2";
const STATUS_TTL_EXCEEDED: &str = "3";

/// The largest `size` the public contract accepts. macOS caps an ICMP datagram at
/// `net.inet.raw.maxdgram` (8192) minus the 8-byte header = 8184; Linux allows
/// 65507. The contract publishes the MINIMUM so one documented limit is true
/// everywhere (plan-110-A §C3).
pub(crate) const PING_MAX_PAYLOAD: i64 = 8184;

/// Receive buffer: the largest reply is our own payload echoed back plus an IPv4
/// header; an ICMP error quotes at most the original header plus 8 bytes. One fixed
/// allocation avoids sizing this at run time.
const RECV_CAPACITY: &str = "8320";

/// `PingResult { status, address, rttMs, ttl, size }` — five 8-byte slots.
const PING_RESULT_SIZE: &str = "40";
const RESULT_OFFSET_STATUS: usize = 0;
const RESULT_OFFSET_ADDRESS: usize = 8;
const RESULT_OFFSET_RTT: usize = 16;
const RESULT_OFFSET_TTL: usize = 24;
const RESULT_OFFSET_SIZE: usize = 32;

/// The `Address` record's port slot, forced to 0 because ICMP has no transport port.
const ADDRESS_OFFSET_PORT: usize = 8;

/// Nanoseconds per millisecond — the `f64` divisor for the round-trip time.
const NANOS_PER_MILLI: &str = "1000000";
const NANOS_PER_SECOND: &str = "1000000000";

/// Dispatch the ping lowering by platform family. `address_form` selects the
/// `ping(net::Address, …)` overload, whose first argument is an `Address` record
/// rather than a `String`.
pub(crate) fn lower_net_ping_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    address_form: bool,
) -> Result<NetBodyParts, String> {
    match platform.family() {
        PlatformFamily::Windows => {
            lower_ping_windows(symbol, platform_imports, platform, address_form)
        }
        family => lower_ping_posix(
            symbol,
            platform_imports,
            platform,
            address_form,
            family == PlatformFamily::MacOS,
        ),
    }
}

// ---------------------------------------------------------------------------
// Frame layout (POSIX)
// ---------------------------------------------------------------------------

const FRAME_SIZE: usize = 640;

const HOST_OFFSET: usize = 8; // String ptr (after unwrapping the Address form)
const TIMEOUT_OFFSET: usize = 16;
const TTL_OFFSET: usize = 24;
const SIZE_OFFSET: usize = 32;
const CSTR_OFFSET: usize = 40;
const RES_OFFSET: usize = 48; // addrinfo*
const FD_OFFSET: usize = 56;
const PKT_OFFSET: usize = 64; // echo request buffer
const BUF_OFFSET: usize = 72; // receive buffer
const START_OFFSET: usize = 80; // monotonic ns captured just before sendto
const DEADLINE_OFFSET: usize = 88; // monotonic ns, or -1 for unbounded
const ID_OFFSET: usize = 96;
const SEQ_OFFSET: usize = 104;
const STATUS_OFFSET: usize = 112;
const RTT_OFFSET: usize = 120; // f64 bits
const RTTL_OFFSET: usize = 128; // reply TTL
const RSIZE_OFFSET: usize = 136; // reply payload size
const SADDR_PTR_OFFSET: usize = 144; // sockaddr to render into the Address
const ADDRLEN_OFFSET: usize = 152;
const HOSTLEN_OFFSET: usize = 160; // scratch for emit_address_from_sockaddr
const DST_OFFSET: usize = 168; // scratch
const AHOST_OFFSET: usize = 176; // scratch
const NRECV_OFFSET: usize = 184;
const OPTVAL_OFFSET: usize = 192; // setsockopt optval (int)
const POLLFD_OFFSET: usize = 200; // pollfd { fd; events; revents }
const TS_OFFSET: usize = 216; // struct timespec (16)
const REMAIN_OFFSET: usize = 232; // poll timeout in ms for this iteration
const ADDRREC_OFFSET: usize = 240; // the built Address record
const MSG_OFFSET: usize = 248; // struct msghdr (Linux, 56 bytes) 248..304
const IOV_OFFSET: usize = 304; // struct iovec (16) 304..320
const HINTS_OFFSET: usize = 320; // addrinfo hints (48) 320..368
const FROM_OFFSET: usize = 384; // sockaddr_storage (128) 384..512
const CMSG_OFFSET: usize = 512; // control buffer (Linux) 512..576

// Linux `msghdr` / `cmsghdr` offsets. Measured identical on x86_64/aarch64/riscv64
// and on glibc/musl (plan-110-A §C5) — only the declared width of `msg_iovlen` /
// `msg_controllen` / `cmsg_len` varies, which is invisible to 8-byte little-endian
// stores and loads at these 8-aligned offsets.
const MSGHDR_NAME: usize = 0;
const MSGHDR_NAMELEN: usize = 8;
const MSGHDR_IOV: usize = 16;
const MSGHDR_IOVLEN: usize = 24;
const MSGHDR_CONTROL: usize = 32;
const MSGHDR_CONTROLLEN: usize = 40;
const MSGHDR_FLAGS: usize = 48;
const CMSGHDR_LEVEL: usize = 8;
const CMSGHDR_TYPE: usize = 12;
const CMSGHDR_DATA: usize = 16;
/// Control buffer handed to `recvmsg`. One `CMSG_SPACE(sizeof(int))` is 24 bytes,
/// but the kernel may attach more than the message we asked for, so leave room
/// rather than risk `MSG_CTRUNC` dropping the TTL.
const CMSG_CAPACITY: usize = 64;

/// The POSIX ICMP-echo backend. `darwin` selects the macOS reply shape (IPv4 header
/// in the buffer, echo id preserved, promiscuous socket) over the Linux one (bare
/// ICMP message, kernel-rewritten id, TTL via control message).
#[allow(clippy::too_many_lines)]
fn lower_ping_posix(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    address_form: bool,
    darwin: bool,
) -> Result<NetBodyParts, String> {
    let invalid = format!("{symbol}_invalid");
    let timeout_ok = format!("{symbol}_timeout_ok");
    let resolve_fail = format!("{symbol}_resolve_fail");
    let socket_fail = format!("{symbol}_socket_fail");
    let op_fail = format!("{symbol}_op_fail");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let addr_fail = format!("{symbol}_addr_fail");
    let fill_loop = format!("{symbol}_fill");
    let fill_done = format!("{symbol}_fill_done");
    let sum_loop = format!("{symbol}_sum");
    let sum_done = format!("{symbol}_sum_done");
    let sum_odd = format!("{symbol}_sum_odd");
    let deadline_unbounded = format!("{symbol}_deadline_unbounded");
    let deadline_ok = format!("{symbol}_deadline_ok");
    let recv_loop = format!("{symbol}_recv_loop");
    let poll_infinite = format!("{symbol}_poll_infinite");
    let remain_ok = format!("{symbol}_remain_ok");
    let remain_clamped = format!("{symbol}_remain_clamped");
    let poll_issue = format!("{symbol}_poll_issue");
    let poll_fail = format!("{symbol}_poll_fail");
    let status_timeout = format!("{symbol}_status_timeout");
    let recv_fail = format!("{symbol}_recv_fail");
    let no_ip_header = format!("{symbol}_no_ip_header");
    let have_header = format!("{symbol}_have_header");
    let try_error_reply = format!("{symbol}_try_error");
    let error_reply = format!("{symbol}_error_reply");
    let error_status_set = format!("{symbol}_error_status_set");
    let cmsg_scan = format!("{symbol}_cmsg_scan");
    let cmsg_next = format!("{symbol}_cmsg_next");
    let cmsg_done = format!("{symbol}_cmsg_done");
    let build = format!("{symbol}_build");
    let cleanup_fail = format!("{symbol}_cleanup_fail");
    let done = format!("{symbol}_done");

    let mut instructions: Vec<CodeInstruction> = Vec::new();
    let mut relocations: Vec<CodeRelocation> = Vec::new();
    let mut vregs = Vregs::new();
    let v9 = vregs.next();
    let v10 = vregs.next();
    let v11 = vregs.next();
    let v12 = vregs.next();
    let v13 = vregs.next();
    let v14 = vregs.next();
    let v15 = vregs.next();

    // --- incoming arguments -------------------------------------------------
    // arg0 = host String / Address record, arg1 = timeoutMs, arg2 = ttl, arg3 = size.
    // `builder_values` pads every omitted trailing argument, so all four are present.
    instructions.extend([
        abi::store_u64(abi::c_arg(1), abi::stack_pointer(), TIMEOUT_OFFSET),
        abi::store_u64(abi::c_arg(2), abi::stack_pointer(), TTL_OFFSET),
        abi::store_u64(abi::c_arg(3), abi::stack_pointer(), SIZE_OFFSET),
    ]);
    if address_form {
        // Address { host String ptr @0, port @8 }. The port is deliberately ignored:
        // ICMP has no transport port (plan-110-A §C3).
        instructions.extend([
            abi::load_u64(&v9, abi::return_register(), 0),
            abi::store_u64(&v9, abi::stack_pointer(), HOST_OFFSET),
        ]);
    } else {
        instructions.push(abi::store_u64(
            abi::return_register(),
            abi::stack_pointer(),
            HOST_OFFSET,
        ));
    }

    // --- validate before touching the network -------------------------------
    // Every rejection happens before the resolver or the socket exists, so a bad
    // argument leaks nothing.
    instructions.extend([
        // ttl must be 1..=255 (it is written into a one-byte IP header field).
        abi::load_u64(&v9, abi::stack_pointer(), TTL_OFFSET),
        abi::compare_immediate(&v9, "1"),
        abi::branch_lt(&invalid),
        abi::compare_immediate(&v9, "255"),
        abi::branch_gt(&invalid),
        // size must be 0..=PING_MAX_PAYLOAD. 0 is valid and sends a bare 8-byte ICMP
        // message (measured working on macOS and all three permitted Linux boxes).
        abi::load_u64(&v9, abi::stack_pointer(), SIZE_OFFSET),
        abi::compare_immediate(&v9, "0"),
        abi::branch_lt(&invalid),
        abi::move_immediate(&v10, "Integer", &PING_MAX_PAYLOAD.to_string()),
        abi::compare_registers(&v9, &v10),
        abi::branch_gt(&invalid),
        // timeoutMs: the unbounded sentinel is allowed; any other negative is not.
        abi::load_u64(&v9, abi::stack_pointer(), TIMEOUT_OFFSET),
        abi::move_immediate(&v10, "Integer", TIMEOUT_UNBOUNDED_SENTINEL),
        abi::compare_registers(&v9, &v10),
        abi::branch_eq(&timeout_ok),
        abi::compare_immediate(&v9, "0"),
        abi::branch_lt(&invalid),
        abi::label(&timeout_ok),
        // Nothing is allocated yet; mark the cleanup slots empty so a failure tail
        // can run unconditionally.
        abi::store_u64(abi::ZERO, abi::stack_pointer(), RES_OFFSET),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), FD_OFFSET),
    ]);

    // --- resolve the destination --------------------------------------------
    emit_hints(
        HINTS_OFFSET,
        false,
        SOCK_DGRAM,
        &mut instructions,
        &mut vregs,
    );
    emit_cstring(
        symbol,
        "host",
        HOST_OFFSET,
        CSTR_OFFSET,
        &alloc_fail,
        &mut instructions,
        &mut relocations,
        &mut vregs,
    );
    instructions.extend([
        // getaddrinfo(host, NULL, &hints, &res)
        abi::load_u64(abi::return_register(), abi::stack_pointer(), CSTR_OFFSET),
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),
        abi::add_immediate(abi::c_arg(2), abi::stack_pointer(), HINTS_OFFSET),
        abi::add_immediate(abi::c_arg(3), abi::stack_pointer(), RES_OFFSET),
    ]);
    platform.emit_external_call(
        net_symbol(platform, NetSymbol::GetAddrInfo),
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&resolve_fail),
        // socket(AF_INET, SOCK_DGRAM, IPPROTO_ICMP). This is the call the OS refuses
        // when ICMP is not permitted for the caller (EACCES on Linux when the
        // caller's gid falls outside ping_group_range); the contract turns that into
        // an Error, never a PingStatus (plan-110-A §C3).
        abi::move_immediate(abi::return_register(), "Integer", AF_INET),
        abi::move_immediate(abi::c_arg(1), "Integer", SOCK_DGRAM),
        abi::move_immediate(abi::c_arg(2), "Integer", IPPROTO_ICMP),
    ]);
    // ... | SOCK_CLOEXEC on Linux; macOS sets FD_CLOEXEC just below (bug-499).
    emit_socket_type_cloexec(platform, &mut instructions);
    platform.emit_external_call(
        net_symbol(platform, NetSymbol::Socket),
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        // C `int` return — sign-extend before the signed compare (bug-04/bug-170).
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&socket_fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
    ]);
    emit_fd_cloexec_fallback(
        platform,
        symbol,
        FD_OFFSET,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    // Raise the receive buffer BEFORE sending: with macOS's default, a reply at the
    // documented maximum payload is dropped by the socket layer and the call reports
    // a bogus `Timeout` (see PING_RECV_BUFFER).
    emit_int_sockopt(
        symbol,
        platform,
        platform_imports,
        &mut instructions,
        &mut relocations,
        &mut vregs,
        SockOptSource::RecvBuffer,
        platform.so_rcvbuf(),
        &op_fail,
    )?;
    emit_int_sockopt(
        symbol,
        platform,
        platform_imports,
        &mut instructions,
        &mut relocations,
        &mut vregs,
        SockOptSource::Ttl,
        platform.ip_ttl(),
        &op_fail,
    )?;
    if !darwin {
        // Linux strips the IPv4 header, so the reply TTL is only reachable as a
        // control message. Ask for it; note the cmsg that arrives is typed IP_TTL,
        // not IP_RECVTTL (plan-110-A §C5 trap 2).
        emit_int_sockopt(
            symbol,
            platform,
            platform_imports,
            &mut instructions,
            &mut relocations,
            &mut vregs,
            SockOptSource::One,
            platform.ip_recvttl(),
            &op_fail,
        )?;
    }

    // --- allocate the packet and receive buffers ----------------------------
    instructions.extend([
        abi::load_u64(&v9, abi::stack_pointer(), SIZE_OFFSET),
        abi::add_immediate(abi::return_register(), &v9, 8),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.extend([
        abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), PKT_OFFSET),
        abi::move_immediate(abi::return_register(), "Integer", RECV_CAPACITY),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.push(abi::store_u64(
        abi::mfb_return(1),
        abi::stack_pointer(),
        BUF_OFFSET,
    ));

    // --- derive a per-call echo identifier ----------------------------------
    // Taken from the monotonic clock rather than a constant: on macOS every ICMP
    // socket on the host receives every reply, so two concurrent pings sharing an
    // id and sequence would each accept the other's answer.
    emit_monotonic_nanos(
        symbol,
        platform,
        platform_imports,
        &mut instructions,
        &mut relocations,
        &mut vregs,
        &v9,
    )?;
    instructions.extend([
        abi::move_immediate(&v10, "Integer", "65535"),
        abi::and_registers(&v11, &v9, &v10),
        abi::store_u64(&v11, abi::stack_pointer(), ID_OFFSET),
        abi::shift_right_immediate(&v11, &v9, 16),
        abi::and_registers(&v11, &v11, &v10),
        abi::store_u64(&v11, abi::stack_pointer(), SEQ_OFFSET),
    ]);

    // --- build the echo request ---------------------------------------------
    instructions.extend([
        abi::load_u64(&v9, abi::stack_pointer(), PKT_OFFSET),
        // type = 8 (echo request), code = 0, checksum = 0 (filled in below).
        abi::move_immediate(&v10, "Integer", ICMP_ECHO_REQUEST),
        abi::store_u8(&v10, &v9, 0),
        abi::store_u8(abi::ZERO, &v9, 1),
        abi::store_u8(abi::ZERO, &v9, 2),
        abi::store_u8(abi::ZERO, &v9, 3),
        // id and sequence, network byte order.
        abi::load_u64(&v10, abi::stack_pointer(), ID_OFFSET),
        abi::shift_right_immediate(&v11, &v10, 8),
        abi::store_u8(&v11, &v9, 4),
        abi::store_u8(&v10, &v9, 5),
        abi::load_u64(&v10, abi::stack_pointer(), SEQ_OFFSET),
        abi::shift_right_immediate(&v11, &v10, 8),
        abi::store_u8(&v11, &v9, 6),
        abi::store_u8(&v10, &v9, 7),
        // payload[i] = i & 0xff
        abi::load_u64(&v11, abi::stack_pointer(), SIZE_OFFSET),
        abi::move_immediate(&v12, "Integer", "0"),
        abi::add_immediate(&v13, &v9, 8),
        abi::label(&fill_loop),
        abi::compare_registers(&v12, &v11),
        abi::branch_eq(&fill_done),
        abi::store_u8(&v12, &v13, 0),
        abi::add_immediate(&v13, &v13, 1),
        abi::add_immediate(&v12, &v12, 1),
        abi::branch(&fill_loop),
        abi::label(&fill_done),
    ]);
    emit_icmp_checksum(
        &mut instructions,
        &mut vregs,
        &v9,
        &sum_loop,
        &sum_done,
        &sum_odd,
    );

    // --- deadline ------------------------------------------------------------
    emit_monotonic_nanos(
        symbol,
        platform,
        platform_imports,
        &mut instructions,
        &mut relocations,
        &mut vregs,
        &v9,
    )?;
    instructions.extend([
        abi::store_u64(&v9, abi::stack_pointer(), START_OFFSET),
        abi::load_u64(&v10, abi::stack_pointer(), TIMEOUT_OFFSET),
        abi::move_immediate(&v11, "Integer", TIMEOUT_UNBOUNDED_SENTINEL),
        abi::compare_registers(&v10, &v11),
        abi::branch_eq(&deadline_unbounded),
        // deadline = start + timeoutMs * 1e6
        abi::move_immediate(&v11, "Integer", NANOS_PER_MILLI),
        abi::multiply_registers(&v10, &v10, &v11),
        abi::add_registers(&v10, &v9, &v10),
        abi::store_u64(&v10, abi::stack_pointer(), DEADLINE_OFFSET),
        abi::branch(&deadline_ok),
        abi::label(&deadline_unbounded),
        // -1 marks "no deadline"; the poll below passes -1 straight through.
        abi::bitwise_not(&v10, abi::ZERO),
        abi::store_u64(&v10, abi::stack_pointer(), DEADLINE_OFFSET),
        abi::label(&deadline_ok),
        // sendto(fd, pkt, 8 + size, 0, ai_addr, ai_addrlen)
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), PKT_OFFSET),
        abi::load_u64(&v9, abi::stack_pointer(), SIZE_OFFSET),
        abi::add_immediate(abi::c_arg(2), &v9, 8),
        abi::move_immediate(abi::c_arg(3), "Integer", "0"),
        abi::load_u64(&v9, abi::stack_pointer(), RES_OFFSET),
        abi::load_u64(abi::c_arg(4), &v9, platform.addrinfo_addr_offset()),
        abi::load_u32(abi::c_arg(5), &v9, 16),
    ]);
    // `sendto` takes SIX int args; on Win64 args 5/6 are stack arguments above the
    // shadow (bug-384). This path is POSIX-only but routes through the shared helper
    // so it stays correct by construction.
    crate::codegen::os::ffi::emit_external_int_call(
        platform,
        net_symbol(platform, NetSymbol::SendTo),
        symbol,
        6,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&op_fail),
    ]);

    // --- receive loop ---------------------------------------------------------
    // Every iteration recomputes the remaining time, so an unmatched packet (on
    // macOS, possibly another process's ping) costs a re-poll rather than the whole
    // deadline.
    instructions.extend([
        abi::label(&recv_loop),
        abi::load_u64(&v9, abi::stack_pointer(), DEADLINE_OFFSET),
        abi::compare_immediate(&v9, "0"),
        abi::branch_lt(&poll_infinite),
    ]);
    emit_monotonic_nanos(
        symbol,
        platform,
        platform_imports,
        &mut instructions,
        &mut relocations,
        &mut vregs,
        &v10,
    )?;
    instructions.extend([
        abi::load_u64(&v9, abi::stack_pointer(), DEADLINE_OFFSET),
        abi::subtract_registers(&v9, &v9, &v10),
        abi::compare_immediate(&v9, "0"),
        abi::branch_gt(&remain_ok),
        // Already expired. Poll once with 0 so a reply that landed while the previous
        // packet was being parsed is still collected, then report Timeout.
        abi::move_immediate(&v9, "Integer", "0"),
        abi::branch(&remain_clamped),
        abi::label(&remain_ok),
        abi::move_immediate(&v10, "Integer", NANOS_PER_MILLI),
        abi::unsigned_divide_registers(&v9, &v9, &v10),
        // poll() takes a C `int`: clamp so a huge deadline cannot be read back as a
        // negative (infinite) timeout (bug-239).
        abi::move_immediate(&v10, "Integer", "2147483647"),
        abi::compare_registers(&v9, &v10),
        abi::branch_le(&remain_clamped),
        abi::move_register(&v9, &v10),
        abi::label(&remain_clamped),
        abi::store_u64(&v9, abi::stack_pointer(), REMAIN_OFFSET),
        abi::branch(&poll_issue),
        abi::label(&poll_infinite),
        abi::bitwise_not(&v9, abi::ZERO),
        abi::store_u64(&v9, abi::stack_pointer(), REMAIN_OFFSET),
        abi::label(&poll_issue),
        // pollfd { fd, events = POLLIN, revents }
        abi::load_u64(&v9, abi::stack_pointer(), FD_OFFSET),
        abi::store_u64(&v9, abi::stack_pointer(), POLLFD_OFFSET),
    ]);
    emit_pollfd_events(platform, POLLFD_OFFSET, &mut instructions, &mut vregs);
    instructions.extend([
        abi::add_immediate(abi::return_register(), abi::stack_pointer(), POLLFD_OFFSET),
        abi::move_immediate(abi::c_arg(1), "Integer", "1"),
        abi::load_u64(abi::c_arg(2), abi::stack_pointer(), REMAIN_OFFSET),
    ]);
    platform.emit_external_call(
        net_symbol(platform, NetSymbol::Poll),
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&poll_fail),
        abi::branch_eq(&status_timeout),
    ]);

    // --- receive one packet ---------------------------------------------------
    if darwin {
        instructions.extend([
            abi::move_immediate(&v9, "Integer", &SOCKADDR_STORAGE_SIZE.to_string()),
            abi::store_u64(&v9, abi::stack_pointer(), ADDRLEN_OFFSET),
            // recvfrom(fd, buf, RECV_CAPACITY, 0, &from, &fromlen)
            abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
            abi::load_u64(abi::c_arg(1), abi::stack_pointer(), BUF_OFFSET),
            abi::move_immediate(abi::c_arg(2), "Integer", RECV_CAPACITY),
            abi::move_immediate(abi::c_arg(3), "Integer", "0"),
            abi::add_immediate(abi::c_arg(4), abi::stack_pointer(), FROM_OFFSET),
            abi::add_immediate(abi::c_arg(5), abi::stack_pointer(), ADDRLEN_OFFSET),
        ]);
        crate::codegen::os::ffi::emit_external_int_call(
            platform,
            net_symbol(platform, NetSymbol::RecvFrom),
            symbol,
            6,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
    } else {
        instructions.extend([
            // struct msghdr { name, namelen, iov, iovlen, control, controllen, flags }
            abi::add_immediate(&v9, abi::stack_pointer(), FROM_OFFSET),
            abi::store_u64(&v9, abi::stack_pointer(), MSG_OFFSET + MSGHDR_NAME),
            abi::move_immediate(&v9, "Integer", &SOCKADDR_STORAGE_SIZE.to_string()),
            abi::store_u32(&v9, abi::stack_pointer(), MSG_OFFSET + MSGHDR_NAMELEN),
            abi::add_immediate(&v9, abi::stack_pointer(), IOV_OFFSET),
            abi::store_u64(&v9, abi::stack_pointer(), MSG_OFFSET + MSGHDR_IOV),
            // A u64 store is right for both libcs: glibc declares `size_t`, musl an
            // `int` followed by padding, and the value is 1 either way on
            // little-endian (plan-110-A §C5).
            abi::move_immediate(&v9, "Integer", "1"),
            abi::store_u64(&v9, abi::stack_pointer(), MSG_OFFSET + MSGHDR_IOVLEN),
            abi::add_immediate(&v9, abi::stack_pointer(), CMSG_OFFSET),
            abi::store_u64(&v9, abi::stack_pointer(), MSG_OFFSET + MSGHDR_CONTROL),
            // Safe as a u64 store on Linux ONLY: `msg_flags` sits at 48 here but at
            // 44 on macOS, where this would clobber it (plan-110-A §C5 trap 1).
            abi::move_immediate(&v9, "Integer", &CMSG_CAPACITY.to_string()),
            abi::store_u64(&v9, abi::stack_pointer(), MSG_OFFSET + MSGHDR_CONTROLLEN),
            abi::store_u32(abi::ZERO, abi::stack_pointer(), MSG_OFFSET + MSGHDR_FLAGS),
            // struct iovec { base = buf, len = RECV_CAPACITY }
            abi::load_u64(&v9, abi::stack_pointer(), BUF_OFFSET),
            abi::store_u64(&v9, abi::stack_pointer(), IOV_OFFSET),
            abi::move_immediate(&v9, "Integer", RECV_CAPACITY),
            abi::store_u64(&v9, abi::stack_pointer(), IOV_OFFSET + 8),
            // recvmsg(fd, &msg, 0)
            abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
            abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), MSG_OFFSET),
            abi::move_immediate(abi::c_arg(2), "Integer", "0"),
        ]);
        platform.emit_external_call(
            "recvmsg",
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
    }
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&recv_fail),
        abi::store_u64(abi::return_register(), abi::stack_pointer(), NRECV_OFFSET),
    ]);

    // --- locate the ICMP message and the reply TTL ---------------------------
    // v13 = ICMP message pointer, v14 = ICMP message length, v15 = reply TTL.
    instructions.extend([
        abi::load_u64(&v9, abi::stack_pointer(), BUF_OFFSET),
        abi::load_u64(&v14, abi::stack_pointer(), NRECV_OFFSET),
        abi::move_immediate(&v15, "Integer", "0"),
        // An IPv4 header is present iff the high nibble of byte 0 is 4. macOS always
        // attaches one, Linux never does. Sniffing rather than assuming keeps this
        // parser correct on both, and on a raw socket, which also carries a header.
        abi::load_u8(&v10, &v9, 0),
        abi::shift_right_immediate(&v11, &v10, 4),
        abi::compare_immediate(&v11, "4"),
        abi::branch_ne(&no_ip_header),
        abi::move_immediate(&v11, "Integer", "15"),
        abi::and_registers(&v11, &v10, &v11),
        abi::shift_left_immediate(&v11, &v11, 2), // ihl words -> bytes
        // The IPv4 total-length field is NOT usable: BSD hands it up in host byte
        // order with the header length already subtracted, so the recv return length
        // is the only trustworthy size (plan-110-A §C2).
        abi::load_u8(&v15, &v9, 8), // reply TTL
        abi::add_registers(&v13, &v9, &v11),
        abi::subtract_registers(&v14, &v14, &v11),
        abi::branch(&have_header),
        abi::label(&no_ip_header),
        abi::move_register(&v13, &v9),
        abi::label(&have_header),
    ]);
    if !darwin {
        // Linux: walk the control messages for the IP_TTL one. There is normally
        // exactly one, but scan rather than assume so an extra cmsg cannot hide it.
        instructions.extend([
            abi::add_immediate(&v10, abi::stack_pointer(), CMSG_OFFSET),
            abi::load_u64(&v11, abi::stack_pointer(), MSG_OFFSET + MSGHDR_CONTROLLEN),
            abi::add_registers(&v11, &v10, &v11), // end of the control buffer
            abi::label(&cmsg_scan),
            // A whole cmsghdr plus its 4-byte payload must fit to read anything.
            abi::add_immediate(&v12, &v10, CMSGHDR_DATA + 4),
            abi::compare_registers(&v12, &v11),
            abi::branch_gt(&cmsg_done),
            abi::load_u32(&v12, &v10, CMSGHDR_LEVEL),
            abi::compare_immediate(&v12, platform.ipproto_ip()),
            abi::branch_ne(&cmsg_next),
            abi::load_u32(&v12, &v10, CMSGHDR_TYPE),
            // IP_TTL, not IP_RECVTTL — the option used to enable this and the message
            // that arrives carry different numbers (plan-110-A §C5 trap 2).
            abi::compare_immediate(&v12, platform.cmsg_ip_ttl_type()),
            abi::branch_ne(&cmsg_next),
            abi::load_u32(&v15, &v10, CMSGHDR_DATA),
            abi::branch(&cmsg_done),
            abi::label(&cmsg_next),
            // cmsg_len is padded up to the next 8-byte boundary between messages.
            abi::load_u32(&v12, &v10, 0),
            abi::add_immediate(&v12, &v12, 7),
            abi::shift_right_immediate(&v12, &v12, 3),
            abi::shift_left_immediate(&v12, &v12, 3),
            // A zero-length cmsg would spin forever; treat it as the end.
            abi::compare_immediate(&v12, "0"),
            abi::branch_le(&cmsg_done),
            abi::add_registers(&v10, &v10, &v12),
            abi::branch(&cmsg_scan),
            abi::label(&cmsg_done),
        ]);
    }

    // --- match the reply -------------------------------------------------------
    instructions.extend([
        // A truncated message cannot be classified; wait for a better one.
        abi::compare_immediate(&v14, "8"),
        abi::branch_lt(&recv_loop),
        abi::load_u8(&v10, &v13, 0), // ICMP type — must survive to the error arm
        abi::compare_immediate(&v10, ICMP_ECHO_REPLY),
        abi::branch_ne(&try_error_reply),
        // Echo reply: the sequence must be ours.
        abi::load_u8(&v11, &v13, 6),
        abi::shift_left_immediate(&v11, &v11, 8),
        abi::load_u8(&v12, &v13, 7),
        abi::or_registers(&v11, &v11, &v12),
        abi::load_u64(&v12, abi::stack_pointer(), SEQ_OFFSET),
        abi::compare_registers(&v11, &v12),
        abi::branch_ne(&recv_loop),
    ]);
    if darwin {
        // macOS preserves the echo id AND delivers every host ICMP reply to every
        // ICMP socket, so the id is both checkable and necessary. Linux rewrites it
        // (not checkable) and demultiplexes per socket (not necessary).
        instructions.extend([
            abi::load_u8(&v11, &v13, 4),
            abi::shift_left_immediate(&v11, &v11, 8),
            abi::load_u8(&v12, &v13, 5),
            abi::or_registers(&v11, &v11, &v12),
            abi::load_u64(&v12, abi::stack_pointer(), ID_OFFSET),
            abi::compare_registers(&v11, &v12),
            abi::branch_ne(&recv_loop),
        ]);
    }
    instructions.extend([
        // Matched. status = Ok, size = ICMP length - 8, ttl = the observed reply TTL.
        abi::move_immediate(&v11, "Integer", STATUS_OK),
        abi::store_u64(&v11, abi::stack_pointer(), STATUS_OFFSET),
        abi::subtract_immediate(&v11, &v14, 8),
        abi::store_u64(&v11, abi::stack_pointer(), RSIZE_OFFSET),
        abi::store_u64(&v15, abi::stack_pointer(), RTTL_OFFSET),
        // The responder is whoever answered, not who we aimed at.
        abi::add_immediate(&v11, abi::stack_pointer(), FROM_OFFSET),
        abi::store_u64(&v11, abi::stack_pointer(), SADDR_PTR_OFFSET),
    ]);
    emit_monotonic_nanos(
        symbol,
        platform,
        platform_imports,
        &mut instructions,
        &mut relocations,
        &mut vregs,
        &v9,
    )?;
    instructions.extend([
        abi::load_u64(&v10, abi::stack_pointer(), START_OFFSET),
        abi::subtract_registers(&v9, &v9, &v10),
        // rttMs is a Float: a loopback round trip is tens of microseconds and would
        // truncate to 0 in whole milliseconds, which is exactly why the contract
        // types this field Float rather than Integer (plan-110-A §C3).
        abi::signed_convert_to_float_d(abi::FP_SCRATCH[0], &v9),
        abi::move_immediate(&v10, "Integer", NANOS_PER_MILLI),
        abi::signed_convert_to_float_d(abi::FP_SCRATCH[1], &v10),
        abi::float_divide_d(abi::FP_SCRATCH[0], abi::FP_SCRATCH[0], abi::FP_SCRATCH[1]),
        abi::float_move_x_from_d(&v9, abi::FP_SCRATCH[0]),
        abi::store_u64(&v9, abi::stack_pointer(), RTT_OFFSET),
        abi::branch(&build),
    ]);

    // --- ICMP error replies ----------------------------------------------------
    instructions.extend([
        abi::label(&try_error_reply),
        abi::compare_immediate(&v10, ICMP_TIME_EXCEEDED),
        abi::branch_eq(&error_reply),
        abi::compare_immediate(&v10, ICMP_DEST_UNREACHABLE),
        abi::branch_ne(&recv_loop),
        abi::label(&error_reply),
        // An ICMP error quotes the original IPv4 header plus its first 8 bytes,
        // starting 8 bytes into the error message. That quoted echo header is the
        // only way to tell our datagram's error from another process's.
        abi::compare_immediate(&v14, "36"), // 8 + 20 + 8: the smallest usable quote
        abi::branch_lt(&recv_loop),
        abi::add_immediate(&v11, &v13, 8), // quoted IPv4 header
        abi::load_u8(&v12, &v11, 0),
        abi::move_immediate(&v9, "Integer", "15"),
        abi::and_registers(&v12, &v12, &v9),
        abi::shift_left_immediate(&v12, &v12, 2), // quoted ihl in bytes
        // The quote must also be long enough to contain the 8-byte echo header.
        abi::add_immediate(&v9, &v12, 16), // 8 (error hdr) + ihl + 8 (echo hdr)
        abi::compare_registers(&v14, &v9),
        abi::branch_lt(&recv_loop),
        abi::add_registers(&v11, &v11, &v12), // quoted ICMP echo header
        abi::load_u8(&v9, &v11, 0),
        abi::compare_immediate(&v9, ICMP_ECHO_REQUEST),
        abi::branch_ne(&recv_loop),
        // The sequence survives the quote on every platform; the id does not (Linux
        // rewrote it on the way out), so match on sequence alone here.
        abi::load_u8(&v9, &v11, 6),
        abi::shift_left_immediate(&v9, &v9, 8),
        abi::load_u8(&v12, &v11, 7),
        abi::or_registers(&v9, &v9, &v12),
        abi::load_u64(&v12, abi::stack_pointer(), SEQ_OFFSET),
        abi::compare_registers(&v9, &v12),
        abi::branch_ne(&recv_loop),
        // Ours. `v10` still holds the ICMP type, which selects the status.
        abi::move_immediate(&v9, "Integer", STATUS_UNREACHABLE),
        abi::compare_immediate(&v10, ICMP_TIME_EXCEEDED),
        abi::branch_ne(&error_status_set),
        abi::move_immediate(&v9, "Integer", STATUS_TTL_EXCEEDED),
        abi::label(&error_status_set),
        abi::store_u64(&v9, abi::stack_pointer(), STATUS_OFFSET),
        // Every non-Ok status zeroes rttMs, ttl and size (plan-110-A §C3).
        abi::store_u64(abi::ZERO, abi::stack_pointer(), RTT_OFFSET),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), RTTL_OFFSET),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), RSIZE_OFFSET),
        abi::add_immediate(&v9, abi::stack_pointer(), FROM_OFFSET),
        abi::store_u64(&v9, abi::stack_pointer(), SADDR_PTR_OFFSET),
        abi::branch(&build),
    ]);

    // --- timeout ---------------------------------------------------------------
    instructions.extend([
        abi::label(&status_timeout),
        abi::move_immediate(&v9, "Integer", STATUS_TIMEOUT),
        abi::store_u64(&v9, abi::stack_pointer(), STATUS_OFFSET),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), RTT_OFFSET),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), RTTL_OFFSET),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), RSIZE_OFFSET),
        // Nobody answered, so report the address we aimed at — an empty `address`
        // would tell the caller less than the destination does. The resolver results
        // are still alive here; `build` renders the Address before freeing them.
        abi::load_u64(&v9, abi::stack_pointer(), RES_OFFSET),
        abi::load_u64(&v9, &v9, platform.addrinfo_addr_offset()),
        abi::store_u64(&v9, abi::stack_pointer(), SADDR_PTR_OFFSET),
    ]);

    // --- build the PingResult ----------------------------------------------------
    instructions.extend([
        abi::label(&build),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
    ]);
    platform.emit_external_call(
        net_symbol(platform, NetSymbol::Close),
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    // The Address is rendered BEFORE `freeaddrinfo`, because on the timeout path
    // `SADDR_PTR` points into the resolver's own results.
    emit_address_from_sockaddr(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "ping",
        SADDR_PTR_OFFSET,
        HOSTLEN_OFFSET,
        DST_OFFSET,
        AHOST_OFFSET,
        &alloc_fail,
        &addr_fail,
        &mut vregs,
    )?;
    instructions.extend([
        abi::move_register(&v9, abi::mfb_return(1)),
        // ICMP has no transport port: whatever the sockaddr carried, publish 0
        // (plan-110-A §C3).
        abi::store_u64(abi::ZERO, &v9, ADDRESS_OFFSET_PORT),
        abi::store_u64(&v9, abi::stack_pointer(), ADDRREC_OFFSET),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), RES_OFFSET),
    ]);
    platform.emit_external_call(
        net_symbol(platform, NetSymbol::FreeAddrInfo),
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", PING_RESULT_SIZE),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.extend([
        abi::move_register(&v9, abi::mfb_return(1)),
        abi::load_u64(&v10, abi::stack_pointer(), STATUS_OFFSET),
        abi::store_u64(&v10, &v9, RESULT_OFFSET_STATUS),
        abi::load_u64(&v10, abi::stack_pointer(), ADDRREC_OFFSET),
        abi::store_u64(&v10, &v9, RESULT_OFFSET_ADDRESS),
        abi::load_u64(&v10, abi::stack_pointer(), RTT_OFFSET),
        abi::store_u64(&v10, &v9, RESULT_OFFSET_RTT),
        abi::load_u64(&v10, abi::stack_pointer(), RTTL_OFFSET),
        abi::store_u64(&v10, &v9, RESULT_OFFSET_TTL),
        abi::load_u64(&v10, abi::stack_pointer(), RSIZE_OFFSET),
        abi::store_u64(&v10, &v9, RESULT_OFFSET_SIZE),
        abi::move_register(RESULT_VALUE_REGISTER, &v9),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);

    // --- failure tails -----------------------------------------------------------
    // `poll` and the receive can be interrupted by a signal; re-issue rather than
    // reporting a spurious failure (bug-115).
    instructions.push(abi::label(&poll_fail));
    platform.emit_errno(
        symbol,
        (&v9).into(),
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(&v9, EINTR_ERRNO),
        abi::branch_eq(&recv_loop),
        abi::branch(&op_fail),
        abi::label(&recv_fail),
    ]);
    platform.emit_errno(
        symbol,
        (&v9).into(),
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(&v9, EINTR_ERRNO),
        abi::branch_eq(&recv_loop),
        // op_fail: the socket is open and the resolver results are live.
        abi::label(&op_fail),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
    ]);
    platform.emit_external_call(
        net_symbol(platform, NetSymbol::Close),
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    // socket_fail: the resolver results are live but no socket was opened.
    instructions.extend([
        abi::label(&socket_fail),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), RES_OFFSET),
    ]);
    platform.emit_external_call(
        net_symbol(platform, NetSymbol::FreeAddrInfo),
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.push(abi::label(&cleanup_fail));
    emit_fail(
        symbol,
        "ErrNetworkFailed",
        &mut instructions,
        &mut relocations,
        &done,
    );
    // addr_fail: `inet_ntop` failed after the socket and resolver were released.
    instructions.push(abi::label(&addr_fail));
    emit_fail(
        symbol,
        "ErrNetworkFailed",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&resolve_fail));
    emit_fail(
        symbol,
        "ErrAddressInvalid",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&invalid));
    emit_fail(
        symbol,
        "ErrInvalidArgument",
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
    Ok((instructions, relocations, FRAME_SIZE))
}

/// Which value an [`emit_int_sockopt`] call writes into the `optval` slot.
enum SockOptSource {
    /// The validated `ttl` argument.
    Ttl,
    /// A literal `1`, to enable a boolean option.
    One,
    /// The receive-buffer size [`PING_RECV_BUFFER`].
    RecvBuffer,
}

/// The receive buffer `net::ping` asks for.
///
/// macOS's default raw receive space is `net.inet.raw.maxdgram` = 8192, and the
/// socket-buffer accounting charges per-datagram overhead on top of the payload —
/// so with the default buffer the largest echo that actually comes BACK is 8132
/// payload bytes, well under the 8184 that `sendto` accepts. The failure is silent:
/// the request goes out, the reply is dropped by the socket layer, and the call
/// reports `Timeout` as though the host were down. Measured with
/// `/tmp/p110-probe/rcvbuf.c`: requesting 32768 or more restores the full 8184.
/// 65536 leaves headroom without being extravagant, and Linux (whose default is
/// already far larger) is unaffected.
const PING_RECV_BUFFER: &str = "65536";

/// `setsockopt(fd, level, option, &optval, 4)` with a 4-byte integer `optval`,
/// branching to `fail` when it reports an error. `SO_RCVBUF` is socket-level; the
/// TTL options are IP-level.
#[allow(clippy::too_many_arguments)]
fn emit_int_sockopt(
    symbol: &str,
    platform: &dyn CodegenPlatform,
    platform_imports: &HashMap<String, String>,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    vregs: &mut Vregs,
    source: SockOptSource,
    option: &str,
    fail: &str,
) -> Result<(), String> {
    let scratch = vregs.next();
    let level = match source {
        SockOptSource::Ttl => {
            instructions.push(abi::load_u64(&scratch, abi::stack_pointer(), TTL_OFFSET));
            platform.ipproto_ip()
        }
        SockOptSource::One => {
            instructions.push(abi::move_immediate(&scratch, "Integer", "1"));
            platform.ipproto_ip()
        }
        SockOptSource::RecvBuffer => {
            instructions.push(abi::move_immediate(&scratch, "Integer", PING_RECV_BUFFER));
            platform.sol_socket()
        }
    };
    instructions.extend([
        abi::store_u64(&scratch, abi::stack_pointer(), OPTVAL_OFFSET),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), FD_OFFSET),
        abi::move_immediate(abi::c_arg(1), "Integer", level),
        abi::move_immediate(abi::c_arg(2), "Integer", option),
        abi::add_immediate(abi::c_arg(3), abi::stack_pointer(), OPTVAL_OFFSET),
        abi::move_immediate(abi::c_arg(4), "Integer", "4"),
    ]);
    // `setsockopt` takes FIVE int args; the fifth is a stack argument on Win64
    // (bug-384). POSIX-only here, but the shared helper keeps it correct anyway.
    crate::codegen::os::ffi::emit_external_int_call(
        platform,
        net_symbol(platform, NetSymbol::SetSockOpt),
        symbol,
        5,
        platform_imports,
        instructions,
        relocations,
    )?;
    instructions.extend([
        abi::sign_extend_word(abi::return_register(), abi::return_register()),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(fail),
    ]);
    Ok(())
}

/// Read `CLOCK_MONOTONIC` into `dst` as nanoseconds. The clock id differs by
/// platform (macOS 6, Linux 1), so it comes from the platform rather than a literal.
fn emit_monotonic_nanos(
    symbol: &str,
    platform: &dyn CodegenPlatform,
    platform_imports: &HashMap<String, String>,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    vregs: &mut Vregs,
    dst: &str,
) -> Result<(), String> {
    let sec = vregs.next();
    let nsec = vregs.next();
    let scale = vregs.next();
    instructions.extend([
        abi::move_immediate(abi::c_arg(0), "Integer", platform.clock_monotonic()),
        abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), TS_OFFSET),
    ]);
    platform.emit_external_call(
        "clock_gettime",
        symbol,
        platform_imports,
        instructions,
        relocations,
    )?;
    instructions.extend([
        abi::load_u64(&sec, abi::stack_pointer(), TS_OFFSET),
        abi::load_u64(&nsec, abi::stack_pointer(), TS_OFFSET + 8),
        abi::move_immediate(&scale, "Integer", NANOS_PER_SECOND),
        abi::multiply_registers(&sec, &sec, &scale),
        abi::add_registers(dst, &sec, &nsec),
    ]);
    Ok(())
}

/// Fill the ICMP checksum field of the packet based at `pkt`.
///
/// The internet checksum is the one's complement of the one's-complement sum of the
/// message read as 16-bit big-endian words. Bytes 2..3 — the checksum field itself —
/// are already zero when this runs. Accumulating into a 64-bit register cannot
/// overflow for any payload the contract allows, so the carry fold is two fixed
/// steps rather than a loop.
fn emit_icmp_checksum(
    instructions: &mut Vec<CodeInstruction>,
    vregs: &mut Vregs,
    pkt: &str,
    sum_loop: &str,
    sum_done: &str,
    sum_odd: &str,
) {
    let cursor = vregs.next();
    let remaining = vregs.next();
    let sum = vregs.next();
    let hi = vregs.next();
    let lo = vregs.next();
    let mask = vregs.next();
    instructions.extend([
        abi::load_u64(&remaining, abi::stack_pointer(), SIZE_OFFSET),
        abi::add_immediate(&remaining, &remaining, 8),
        abi::move_register(&cursor, pkt),
        abi::move_immediate(&sum, "Integer", "0"),
        abi::label(sum_loop),
        abi::compare_immediate(&remaining, "2"),
        abi::branch_lt(sum_odd),
        abi::load_u8(&hi, &cursor, 0),
        abi::shift_left_immediate(&hi, &hi, 8),
        abi::load_u8(&lo, &cursor, 1),
        abi::or_registers(&hi, &hi, &lo),
        abi::add_registers(&sum, &sum, &hi),
        abi::add_immediate(&cursor, &cursor, 2),
        abi::subtract_immediate(&remaining, &remaining, 2),
        abi::branch(sum_loop),
        abi::label(sum_odd),
        // A trailing odd byte is padded on the right with zero, i.e. it is the HIGH
        // half of the final word.
        abi::compare_immediate(&remaining, "0"),
        abi::branch_eq(sum_done),
        abi::load_u8(&hi, &cursor, 0),
        abi::shift_left_immediate(&hi, &hi, 8),
        abi::add_registers(&sum, &sum, &hi),
        abi::label(sum_done),
        // Fold the carries into 16 bits; twice is enough for a 64-bit accumulation.
        abi::move_immediate(&mask, "Integer", "65535"),
        abi::shift_right_immediate(&hi, &sum, 16),
        abi::and_registers(&lo, &sum, &mask),
        abi::add_registers(&sum, &hi, &lo),
        abi::shift_right_immediate(&hi, &sum, 16),
        abi::and_registers(&lo, &sum, &mask),
        abi::add_registers(&sum, &hi, &lo),
        // checksum = ~sum, stored big-endian into bytes 2..3.
        abi::bitwise_not(&sum, &sum),
        abi::and_registers(&sum, &sum, &mask),
        abi::shift_right_immediate(&hi, &sum, 8),
        abi::store_u8(&hi, pkt, 2),
        abi::store_u8(&sum, pkt, 3),
    ]);
}

// ---------------------------------------------------------------------------
// Windows (iphlpapi)
// ---------------------------------------------------------------------------

// Windows has no unprivileged ICMP socket — Winsock's `SOCK_RAW`/`IPPROTO_ICMP`
// requires Administrator — so ping goes through `iphlpapi`'s ICMP API, which
// builds, matches, and times the echo itself. That makes the Windows backend a
// different shape from POSIX rather than a different set of constants.
//
// `IP_STATUS` values from `ipexport.h` (plan-110-A §C3 fixes the mapping).
const IP_SUCCESS: &str = "0";
const IP_DEST_NET_UNREACHABLE: &str = "11002";
const IP_DEST_HOST_UNREACHABLE: &str = "11003";
const IP_DEST_PROT_UNREACHABLE: &str = "11004";
const IP_DEST_PORT_UNREACHABLE: &str = "11005";
const IP_REQ_TIMED_OUT: &str = "11010";
const IP_BAD_ROUTE: &str = "11012";
const IP_TTL_EXPIRED_TRANSIT: &str = "11013";
const IP_TTL_EXPIRED_REASSEM: &str = "11014";

// `ICMP_ECHO_REPLY` field offsets (x64).
const REPLY_ADDRESS: usize = 0; // IPAddr, 4 bytes, network order
const REPLY_STATUS: usize = 4; // ULONG IP_STATUS
const REPLY_DATA_SIZE: usize = 12; // USHORT
const REPLY_OPTIONS_TTL: usize = 24; // IP_OPTION_INFORMATION.Ttl (UCHAR)
/// `sizeof(ICMP_ECHO_REPLY)` on x64 — the reply buffer must hold this plus the
/// echoed payload plus 8 bytes of slack, per the API contract.
const REPLY_STRUCT_SIZE: usize = 40;

// `IP_OPTION_INFORMATION` (x64): Ttl@0, Tos@1, Flags@2, OptionsSize@3, OptionsData@8.
const OPTINFO_TTL: usize = 0;

/// How long one `IcmpSendEcho` attempt waits when the caller asked for an
/// unbounded ping. The API takes a `DWORD` and has no infinite value, so an
/// omitted `timeoutMs` re-issues the echo until something other than
/// `IP_REQ_TIMED_OUT` comes back — which is what a real ping does, and the honest
/// reading of "wait indefinitely" (plan-110-A §C3).
const WIN_UNBOUNDED_ATTEMPT_MS: &str = "60000";

const WIN_FRAME_SIZE: usize = 512;
const W_HOST: usize = 8;
const W_TIMEOUT: usize = 16;
const W_TTL: usize = 24;
const W_SIZE: usize = 32;
const W_CSTR: usize = 40;
const W_RES: usize = 48; // addrinfo*
const W_HANDLE: usize = 56; // IcmpCreateFile handle
const W_REQ: usize = 64; // request payload buffer
const W_REPLY: usize = 72; // reply buffer
const W_REPLY_LEN: usize = 80;
const W_STATUS: usize = 88; // PingStatus ordinal
const W_RTT: usize = 96; // f64 bits
const W_RTTL: usize = 104;
const W_RSIZE: usize = 112;
const W_DEST: usize = 120; // destination IPAddr (4 bytes, network order)
const W_QPC_START: usize = 128;
const W_QPC_END: usize = 136;
const W_QPC_FREQ: usize = 144;
const W_ATTEMPT_MS: usize = 152;
const W_IPSTATUS: usize = 160;
const W_SADDR_PTR: usize = 168;
const W_HOSTLEN: usize = 176; // scratch for emit_address_from_sockaddr
const W_DST: usize = 184; // scratch
const W_AHOST: usize = 192; // scratch
const W_ADDRREC: usize = 200;
const W_HINTS: usize = 208; // addrinfo hints (48) 208..256
const W_SOCKADDR: usize = 256; // synthesized sockaddr_in (16) for the responder
const W_OPTINFO: usize = 272; // IP_OPTION_INFORMATION (16)

/// The Windows ICMP backend — `iphlpapi`, not a socket.
#[allow(clippy::too_many_lines)]
fn lower_ping_windows(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    address_form: bool,
) -> Result<NetBodyParts, String> {
    let invalid = format!("{symbol}_invalid");
    let timeout_ok = format!("{symbol}_timeout_ok");
    let resolve_fail = format!("{symbol}_resolve_fail");
    let handle_fail = format!("{symbol}_handle_fail");
    let alloc_fail = format!("{symbol}_alloc_fail");
    let addr_fail = format!("{symbol}_addr_fail");
    let op_fail = format!("{symbol}_op_fail");
    let fill_loop = format!("{symbol}_fill");
    let fill_done = format!("{symbol}_fill_done");
    let attempt = format!("{symbol}_attempt");
    let bounded = format!("{symbol}_bounded");
    let attempt_low = format!("{symbol}_attempt_low");
    let attempt_set = format!("{symbol}_attempt_set");
    let echo_failed = format!("{symbol}_echo_failed");
    let have_status = format!("{symbol}_have_status");
    let classify = format!("{symbol}_classify");
    let status_ok = format!("{symbol}_status_ok");
    let status_timeout = format!("{symbol}_status_timeout");
    let status_unreachable = format!("{symbol}_status_unreachable");
    let status_ttl = format!("{symbol}_status_ttl");
    let non_ok = format!("{symbol}_non_ok");
    let build = format!("{symbol}_build");
    let done = format!("{symbol}_done");

    let mut instructions: Vec<CodeInstruction> = Vec::new();
    let mut relocations: Vec<CodeRelocation> = Vec::new();
    let mut vregs = Vregs::new();
    let v9 = vregs.next();
    let v10 = vregs.next();
    let v11 = vregs.next();
    let v12 = vregs.next();
    let v13 = vregs.next();

    instructions.extend([
        abi::store_u64(abi::c_arg(1), abi::stack_pointer(), W_TIMEOUT),
        abi::store_u64(abi::c_arg(2), abi::stack_pointer(), W_TTL),
        abi::store_u64(abi::c_arg(3), abi::stack_pointer(), W_SIZE),
    ]);
    if address_form {
        instructions.extend([
            abi::load_u64(&v9, abi::return_register(), 0),
            abi::store_u64(&v9, abi::stack_pointer(), W_HOST),
        ]);
    } else {
        instructions.push(abi::store_u64(
            abi::return_register(),
            abi::stack_pointer(),
            W_HOST,
        ));
    }

    // Identical validation to POSIX: the contract's ranges are platform-independent.
    instructions.extend([
        abi::load_u64(&v9, abi::stack_pointer(), W_TTL),
        abi::compare_immediate(&v9, "1"),
        abi::branch_lt(&invalid),
        abi::compare_immediate(&v9, "255"),
        abi::branch_gt(&invalid),
        abi::load_u64(&v9, abi::stack_pointer(), W_SIZE),
        abi::compare_immediate(&v9, "0"),
        abi::branch_lt(&invalid),
        abi::move_immediate(&v10, "Integer", &PING_MAX_PAYLOAD.to_string()),
        abi::compare_registers(&v9, &v10),
        abi::branch_gt(&invalid),
        abi::load_u64(&v9, abi::stack_pointer(), W_TIMEOUT),
        abi::move_immediate(&v10, "Integer", TIMEOUT_UNBOUNDED_SENTINEL),
        abi::compare_registers(&v9, &v10),
        abi::branch_eq(&timeout_ok),
        abi::compare_immediate(&v9, "0"),
        abi::branch_lt(&invalid),
        abi::label(&timeout_ok),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), W_RES),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), W_HANDLE),
    ]);

    // Resolve with Winsock's getaddrinfo, exactly as the other net members do.
    emit_hints(W_HINTS, false, SOCK_DGRAM, &mut instructions, &mut vregs);
    emit_cstring(
        symbol,
        "host",
        W_HOST,
        W_CSTR,
        &alloc_fail,
        &mut instructions,
        &mut relocations,
        &mut vregs,
    );
    instructions.extend([
        abi::load_u64(abi::return_register(), abi::stack_pointer(), W_CSTR),
        abi::move_immediate(abi::c_arg(1), "Integer", "0"),
        abi::add_immediate(abi::c_arg(2), abi::stack_pointer(), W_HINTS),
        abi::add_immediate(abi::c_arg(3), abi::stack_pointer(), W_RES),
    ]);
    platform.emit_external_call(
        net_symbol(platform, NetSymbol::GetAddrInfo),
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne(&resolve_fail),
        // The destination is an IPAddr: the 4-byte in_addr inside the sockaddr_in.
        abi::load_u64(&v9, abi::stack_pointer(), W_RES),
        abi::load_u64(&v9, &v9, platform.addrinfo_addr_offset()),
        abi::load_u32(&v10, &v9, 4),
        abi::store_u64(&v10, abi::stack_pointer(), W_DEST),
        // IcmpCreateFile() -> handle. Unlike the POSIX socket, this does not need
        // any privilege; a failure here is a genuine system error.
    ]);
    platform.emit_external_call(
        "IcmpCreateFile",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::move_register(&v9, abi::c_return(0)),
        // INVALID_HANDLE_VALUE is -1, which the encoder will not take as a compare
        // immediate; materialize it instead. A NULL handle is refused too — the API
        // documents -1, but a zero handle is equally unusable.
        abi::bitwise_not(&v10, abi::ZERO),
        abi::compare_registers(&v9, &v10),
        abi::branch_eq(&handle_fail),
        abi::compare_immediate(&v9, "0"),
        abi::branch_eq(&handle_fail),
        abi::store_u64(&v9, abi::stack_pointer(), W_HANDLE),
        // Request payload: same `i & 0xff` filling as POSIX so a capture looks the
        // same on every platform.
        abi::load_u64(&v9, abi::stack_pointer(), W_SIZE),
        abi::add_immediate(abi::return_register(), &v9, 1), // never a zero-size alloc
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.extend([
        abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), W_REQ),
        abi::load_u64(&v10, abi::stack_pointer(), W_SIZE),
        abi::move_register(&v11, abi::mfb_return(1)),
        abi::move_immediate(&v12, "Integer", "0"),
        abi::label(&fill_loop),
        abi::compare_registers(&v12, &v10),
        abi::branch_eq(&fill_done),
        abi::store_u8(&v12, &v11, 0),
        abi::add_immediate(&v11, &v11, 1),
        abi::add_immediate(&v12, &v12, 1),
        abi::branch(&fill_loop),
        abi::label(&fill_done),
        // Reply buffer: sizeof(ICMP_ECHO_REPLY) + payload + 8, per the API contract.
        abi::load_u64(&v9, abi::stack_pointer(), W_SIZE),
        abi::add_immediate(&v9, &v9, REPLY_STRUCT_SIZE + 8),
        abi::store_u64(&v9, abi::stack_pointer(), W_REPLY_LEN),
        abi::move_register(abi::return_register(), &v9),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.extend([
        abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), W_REPLY),
        // IP_OPTION_INFORMATION { Ttl, Tos, Flags, OptionsSize, OptionsData }
        abi::store_u64(abi::ZERO, abi::stack_pointer(), W_OPTINFO),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), W_OPTINFO + 8),
        abi::load_u64(&v9, abi::stack_pointer(), W_TTL),
        abi::store_u8(&v9, abi::stack_pointer(), W_OPTINFO + OPTINFO_TTL),
        // Per-attempt timeout: the caller's value, or a bounded slice when unbounded.
        abi::load_u64(&v9, abi::stack_pointer(), W_TIMEOUT),
        abi::move_immediate(&v10, "Integer", TIMEOUT_UNBOUNDED_SENTINEL),
        abi::compare_registers(&v9, &v10),
        abi::branch_ne(&bounded),
        abi::move_immediate(&v9, "Integer", WIN_UNBOUNDED_ATTEMPT_MS),
        abi::branch(&attempt_set),
        abi::label(&bounded),
        // The API takes a DWORD; clamp so a huge deadline cannot wrap.
        abi::move_immediate(&v10, "Integer", "4294967294"),
        abi::compare_registers(&v9, &v10),
        abi::branch_le(&attempt_low),
        abi::move_register(&v9, &v10),
        abi::label(&attempt_low),
        // `IcmpSendEcho` does not accept a 0 timeout — it fails the call outright
        // rather than performing one immediate check — so the convention's
        // "`0` is one immediate attempt" becomes the smallest expressible wait.
        // This is the same accommodation `lower_net_set_timeout_helper` already
        // makes for Winsock's `SO_RCVTIMEO`, where 0 means "infinite" instead of
        // "don't wait" (plan-73-C).
        abi::compare_immediate(&v9, "0"),
        abi::branch_ne(&attempt_set),
        abi::move_immediate(&v9, "Integer", "1"),
        abi::label(&attempt_set),
        abi::store_u64(&v9, abi::stack_pointer(), W_ATTEMPT_MS),
        // Time the exchange ourselves rather than using the API's whole-millisecond
        // RoundTripTime: a loopback ping is far under 1 ms and would report 0.0,
        // breaking the contract's measured-rttMs guarantee (plan-110-A §C3).
        abi::add_immediate(abi::c_arg(0), abi::stack_pointer(), W_QPC_FREQ),
    ]);
    platform.emit_external_call(
        "QueryPerformanceFrequency",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::label(&attempt),
        abi::add_immediate(abi::c_arg(0), abi::stack_pointer(), W_QPC_START),
    ]);
    platform.emit_external_call(
        "QueryPerformanceCounter",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    // IcmpSendEcho(handle, dest, reqData, reqSize, reqOptions, replyBuf, replySize,
    // timeout) — EIGHT integer arguments. This deliberately does NOT go through
    // `emit_external_int_call`: that stages argument n in `abi::c_arg(n)`, and the
    // x86 call bank puts `c_arg(7)` on `rbp`, the frame pointer, so staging through
    // it corrupts the frame before the call (plan-110-A §C2). Arguments 0..3 go in
    // the register bank; 4..7 are written straight to the outgoing-args area.
    instructions.extend([
        abi::load_u64(abi::c_arg(0), abi::stack_pointer(), W_HANDLE),
        abi::load_u32(abi::c_arg(1), abi::stack_pointer(), W_DEST),
        abi::load_u64(abi::c_arg(2), abi::stack_pointer(), W_REQ),
        abi::load_u64(abi::c_arg(3), abi::stack_pointer(), W_SIZE),
        abi::add_immediate(&v9, abi::stack_pointer(), W_OPTINFO),
        abi::outgoing_stack_arg_store(&v9, 0),
        abi::load_u64(&v9, abi::stack_pointer(), W_REPLY),
        abi::outgoing_stack_arg_store(&v9, 1),
        abi::load_u64(&v9, abi::stack_pointer(), W_REPLY_LEN),
        abi::outgoing_stack_arg_store(&v9, 2),
        abi::load_u64(&v9, abi::stack_pointer(), W_ATTEMPT_MS),
        abi::outgoing_stack_arg_store(&v9, 3),
    ]);
    platform.emit_external_call(
        "IcmpSendEcho",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::move_register(&v13, abi::c_return(0)),
        abi::add_immediate(abi::c_arg(0), abi::stack_pointer(), W_QPC_END),
    ]);
    platform.emit_external_call(
        "QueryPerformanceCounter",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        // A zero reply count means the whole call failed; GetLastError carries the
        // IP_STATUS. A non-zero count means the reply struct carries it.
        abi::compare_immediate(&v13, "0"),
        abi::branch_eq(&echo_failed),
        abi::load_u64(&v9, abi::stack_pointer(), W_REPLY),
        abi::load_u32(&v10, &v9, REPLY_STATUS),
        abi::store_u64(&v10, abi::stack_pointer(), W_IPSTATUS),
        abi::branch(&have_status),
        abi::label(&echo_failed),
    ]);
    platform.emit_external_call(
        "GetLastError",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::move_register(&v10, abi::c_return(0)),
        abi::store_u64(&v10, abi::stack_pointer(), W_IPSTATUS),
        abi::label(&have_status),
        // An unbounded ping re-issues the echo instead of reporting Timeout.
        abi::load_u64(&v9, abi::stack_pointer(), W_TIMEOUT),
        abi::move_immediate(&v10, "Integer", TIMEOUT_UNBOUNDED_SENTINEL),
        abi::compare_registers(&v9, &v10),
        abi::branch_ne(&classify),
        abi::load_u64(&v9, abi::stack_pointer(), W_IPSTATUS),
        abi::compare_immediate(&v9, IP_REQ_TIMED_OUT),
        abi::branch_eq(&attempt),
        abi::label(&classify),
        abi::load_u64(&v9, abi::stack_pointer(), W_IPSTATUS),
        abi::compare_immediate(&v9, IP_SUCCESS),
        abi::branch_eq(&status_ok),
        abi::compare_immediate(&v9, IP_REQ_TIMED_OUT),
        abi::branch_eq(&status_timeout),
        abi::compare_immediate(&v9, IP_TTL_EXPIRED_TRANSIT),
        abi::branch_eq(&status_ttl),
        abi::compare_immediate(&v9, IP_TTL_EXPIRED_REASSEM),
        abi::branch_eq(&status_ttl),
        abi::compare_immediate(&v9, IP_DEST_NET_UNREACHABLE),
        abi::branch_eq(&status_unreachable),
        abi::compare_immediate(&v9, IP_DEST_HOST_UNREACHABLE),
        abi::branch_eq(&status_unreachable),
        abi::compare_immediate(&v9, IP_DEST_PROT_UNREACHABLE),
        abi::branch_eq(&status_unreachable),
        abi::compare_immediate(&v9, IP_DEST_PORT_UNREACHABLE),
        abi::branch_eq(&status_unreachable),
        abi::compare_immediate(&v9, IP_BAD_ROUTE),
        abi::branch_eq(&status_unreachable),
        // Every other IP_STATUS is a system failure, not an answer about the peer.
        abi::branch(&op_fail),
        // --- Ok: measured values, and the responder is the reply's Address ---
        abi::label(&status_ok),
        abi::move_immediate(&v9, "Integer", STATUS_OK),
        abi::store_u64(&v9, abi::stack_pointer(), W_STATUS),
        abi::load_u64(&v9, abi::stack_pointer(), W_REPLY),
        abi::load_u16(&v10, &v9, REPLY_DATA_SIZE),
        abi::store_u64(&v10, abi::stack_pointer(), W_RSIZE),
        abi::load_u8(&v10, &v9, REPLY_OPTIONS_TTL),
        abi::store_u64(&v10, abi::stack_pointer(), W_RTTL),
        // Synthesize a sockaddr_in { AF_INET, port 0, reply.Address } so the shared
        // Address builder renders it exactly as the POSIX path's `from` sockaddr.
        abi::store_u64(abi::ZERO, abi::stack_pointer(), W_SOCKADDR),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), W_SOCKADDR + 8),
        abi::move_immediate(&v11, "Integer", AF_INET),
        abi::store_u16(&v11, abi::stack_pointer(), W_SOCKADDR),
        abi::load_u32(&v11, &v9, REPLY_ADDRESS),
        abi::store_u32(&v11, abi::stack_pointer(), W_SOCKADDR + 4),
        abi::add_immediate(&v11, abi::stack_pointer(), W_SOCKADDR),
        abi::store_u64(&v11, abi::stack_pointer(), W_SADDR_PTR),
        // rttMs = (end - start) * 1000 / frequency, as a Float.
        abi::load_u64(&v10, abi::stack_pointer(), W_QPC_END),
        abi::load_u64(&v11, abi::stack_pointer(), W_QPC_START),
        abi::subtract_registers(&v10, &v10, &v11),
        abi::load_u64(&v11, abi::stack_pointer(), W_QPC_FREQ),
        abi::signed_convert_to_float_d(abi::FP_SCRATCH[0], &v10),
        abi::signed_convert_to_float_d(abi::FP_SCRATCH[1], &v11),
        abi::float_divide_d(abi::FP_SCRATCH[0], abi::FP_SCRATCH[0], abi::FP_SCRATCH[1]),
        abi::move_immediate(&v12, "Integer", "1000"),
        abi::signed_convert_to_float_d(abi::FP_SCRATCH[1], &v12),
        abi::float_multiply_d(abi::FP_SCRATCH[0], abi::FP_SCRATCH[0], abi::FP_SCRATCH[1]),
        abi::float_move_x_from_d(&v10, abi::FP_SCRATCH[0]),
        abi::store_u64(&v10, abi::stack_pointer(), W_RTT),
        abi::branch(&build),
        // --- the three non-Ok statuses ---
        abi::label(&status_timeout),
        abi::move_immediate(&v9, "Integer", STATUS_TIMEOUT),
        abi::branch(&non_ok),
        abi::label(&status_unreachable),
        abi::move_immediate(&v9, "Integer", STATUS_UNREACHABLE),
        abi::branch(&non_ok),
        abi::label(&status_ttl),
        abi::move_immediate(&v9, "Integer", STATUS_TTL_EXCEEDED),
        abi::label(&non_ok),
        abi::store_u64(&v9, abi::stack_pointer(), W_STATUS),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), W_RTT),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), W_RTTL),
        abi::store_u64(abi::ZERO, abi::stack_pointer(), W_RSIZE),
        // No usable responder: report the destination that was aimed at, matching
        // POSIX. `ai_addr` is still alive; `build` renders before freeing it.
        abi::load_u64(&v9, abi::stack_pointer(), W_RES),
        abi::load_u64(&v9, &v9, platform.addrinfo_addr_offset()),
        abi::store_u64(&v9, abi::stack_pointer(), W_SADDR_PTR),
        // --- build the PingResult ---
        abi::label(&build),
        abi::load_u64(abi::c_arg(0), abi::stack_pointer(), W_HANDLE),
    ]);
    platform.emit_external_call(
        "IcmpCloseHandle",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    emit_address_from_sockaddr(
        &mut EmitCtx {
            symbol,
            platform_imports,
            platform,
            instructions: &mut instructions,
            relocations: &mut relocations,
        },
        "ping",
        W_SADDR_PTR,
        W_HOSTLEN,
        W_DST,
        W_AHOST,
        &alloc_fail,
        &addr_fail,
        &mut vregs,
    )?;
    instructions.extend([
        abi::move_register(&v9, abi::mfb_return(1)),
        abi::store_u64(abi::ZERO, &v9, ADDRESS_OFFSET_PORT),
        abi::store_u64(&v9, abi::stack_pointer(), W_ADDRREC),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), W_RES),
    ]);
    platform.emit_external_call(
        net_symbol(platform, NetSymbol::FreeAddrInfo),
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::move_immediate(abi::return_register(), "Integer", PING_RESULT_SIZE),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.extend([
        abi::move_register(&v9, abi::mfb_return(1)),
        abi::load_u64(&v10, abi::stack_pointer(), W_STATUS),
        abi::store_u64(&v10, &v9, RESULT_OFFSET_STATUS),
        abi::load_u64(&v10, abi::stack_pointer(), W_ADDRREC),
        abi::store_u64(&v10, &v9, RESULT_OFFSET_ADDRESS),
        abi::load_u64(&v10, abi::stack_pointer(), W_RTT),
        abi::store_u64(&v10, &v9, RESULT_OFFSET_RTT),
        abi::load_u64(&v10, abi::stack_pointer(), W_RTTL),
        abi::store_u64(&v10, &v9, RESULT_OFFSET_TTL),
        abi::load_u64(&v10, abi::stack_pointer(), W_RSIZE),
        abi::store_u64(&v10, &v9, RESULT_OFFSET_SIZE),
        abi::move_register(RESULT_VALUE_REGISTER, &v9),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
    ]);

    // --- failure tails ---
    // op_fail: the ICMP handle is open and the resolver results are live.
    instructions.extend([
        abi::label(&op_fail),
        abi::load_u64(abi::c_arg(0), abi::stack_pointer(), W_HANDLE),
    ]);
    platform.emit_external_call(
        "IcmpCloseHandle",
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::label(&handle_fail),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), W_RES),
    ]);
    platform.emit_external_call(
        net_symbol(platform, NetSymbol::FreeAddrInfo),
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    emit_fail(
        symbol,
        "ErrNetworkFailed",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&addr_fail));
    emit_fail(
        symbol,
        "ErrNetworkFailed",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&resolve_fail));
    emit_fail(
        symbol,
        "ErrAddressInvalid",
        &mut instructions,
        &mut relocations,
        &done,
    );
    instructions.push(abi::label(&invalid));
    emit_fail(
        symbol,
        "ErrInvalidArgument",
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
    Ok((instructions, relocations, WIN_FRAME_SIZE))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::registry::registry;

    fn net_package() -> &'static crate::codegen::registry::RegistryPackage {
        registry().resolve_package("net").expect("net package")
    }

    /// The emitters write `PingStatus` ordinals as literals, while the declaration
    /// order in `net::register` is what actually gives each variant its ordinal.
    /// Nothing connects the two at compile time, so reordering the enum — a natural
    /// thing to do while editing docs — would silently change what every ping
    /// reports, with no test failing and no diagnostic. This pins them together.
    #[test]
    fn ping_status_literals_match_the_declared_variant_order() {
        let status = net_package()
            .enums()
            .iter()
            .find(|e| e.name == super::super::PING_STATUS_TYPE)
            .expect("PingStatus enum");
        let ordinal = |name: &str| {
            status
                .variants
                .iter()
                .position(|v| v.name == name)
                .unwrap_or_else(|| panic!("PingStatus has no `{name}` variant"))
                .to_string()
        };
        assert_eq!(STATUS_OK, ordinal("Ok"));
        assert_eq!(STATUS_TIMEOUT, ordinal("Timeout"));
        assert_eq!(STATUS_UNREACHABLE, ordinal("Unreachable"));
        assert_eq!(STATUS_TTL_EXCEEDED, ordinal("TtlExceeded"));
        // Exactly four: a fifth variant would need an emitter arm to ever be
        // produced, and an unproducible status is worse than none.
        assert_eq!(status.variants.len(), 4);
    }

    /// Both backends build `PingResult` by storing five consecutive 8-byte slots at
    /// hardcoded offsets. Those offsets are only correct while the record's declared
    /// field order matches; inserting or reordering a field would silently write
    /// each value into the wrong one.
    #[test]
    fn ping_result_offsets_match_the_declared_field_order() {
        let record = net_package()
            .records()
            .iter()
            .find(|r| r.name == super::super::PING_RESULT_TYPE)
            .expect("PingResult record");
        let names: Vec<&str> = record.props.iter().map(|p| p.name).collect();
        assert_eq!(names, ["status", "address", "rttMs", "ttl", "size"]);
        for (index, offset) in [
            RESULT_OFFSET_STATUS,
            RESULT_OFFSET_ADDRESS,
            RESULT_OFFSET_RTT,
            RESULT_OFFSET_TTL,
            RESULT_OFFSET_SIZE,
        ]
        .into_iter()
        .enumerate()
        {
            assert_eq!(offset, index * 8, "field {} offset", names[index]);
        }
        // The allocation must cover exactly the declared fields.
        assert_eq!(
            PING_RESULT_SIZE.parse::<usize>().unwrap(),
            record.props.len() * 8
        );
        // `rttMs` is Float by contract, not Integer: a loopback round trip is tens
        // of microseconds and would truncate to zero (plan-110-A §C3). The emitters
        // store raw f64 bits into that slot, so an Integer field here would render
        // the bit pattern as a huge number.
        let rtt = record
            .props
            .iter()
            .find(|p| p.name == "rttMs")
            .expect("rttMs field");
        assert_eq!(rtt.ty, crate::types::ParameterType::Float);
    }

    /// The documented maximum is quoted in the member's parameter documentation and
    /// enforced by the emitters from this constant; keep them from drifting apart.
    #[test]
    fn documented_payload_maximum_matches_the_enforced_one() {
        assert_eq!(PING_MAX_PAYLOAD, 8184);
        let ping = net_package()
            .functions()
            .iter()
            .find(|f| f.name == "ping")
            .expect("ping member");
        let size_doc = ping
            .implementations
            .iter()
            .flat_map(|i| i.params.iter())
            .find(|p| p.name == "size")
            .expect("size parameter")
            .desc;
        assert!(
            size_doc.contains(&PING_MAX_PAYLOAD.to_string()),
            "the `size` documentation must quote the enforced maximum {PING_MAX_PAYLOAD}"
        );
    }

    /// Both overloads must return `PingResult`, and the second must take an
    /// `Address` — the `net.pingAddr` alias is selected purely by that argument
    /// type, so a change here silently routes to the wrong backend.
    #[test]
    fn ping_overloads_are_host_and_address() {
        let ping = net_package()
            .functions()
            .iter()
            .find(|f| f.name == "ping")
            .expect("ping member");
        assert_eq!(ping.implementations.len(), 2);
        let expected = crate::types::ParameterType::named(super::super::PING_RESULT_TYPE_ID);
        for implementation in &ping.implementations {
            assert_eq!(implementation.return_type, expected);
            // One required parameter plus three optional ones, on both overloads.
            assert_eq!(implementation.params.len(), 4);
        }
        assert_eq!(
            ping.implementations[0].params[0].ty,
            crate::types::ParameterType::String
        );
        assert_eq!(
            ping.implementations[1].params[0].ty,
            crate::types::ParameterType::named(super::super::ADDRESS_TYPE_ID)
        );
    }
}
