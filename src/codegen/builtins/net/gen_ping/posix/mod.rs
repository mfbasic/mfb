//! plan-176-T: the POSIX (macOS and Linux) `net.ping` backend — its frame layout and `lower_ping_posix`.
//! The shared result helpers live in `builtins/net/gen_ping/mod.rs`; the socket helpers in `socket.rs`.

use super::*;

mod socket;

use socket::{emit_icmp_checksum, emit_int_sockopt, emit_monotonic_nanos, SockOptSource};

/// Nanoseconds per millisecond — the `f64` divisor for the round-trip time.
const NANOS_PER_MILLI: &str = "1000000";

// ---------------------------------------------------------------------------
// Frame layout (POSIX)
// ---------------------------------------------------------------------------

const FRAME_SIZE: usize = 704;

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

const APORT_OFFSET: usize = 576; // the port the Address is built with

const ASIZE_OFFSET: usize = 584; // the built Address's byte size

const ACURSOR_OFFSET: usize = 592; // record marshaller scratch

const ABLOCK_OFFSET: usize = 600; // record marshaller scratch

const PSIZE_OFFSET: usize = 608; // the PingResult's byte size

const PRESULT_OFFSET: usize = 616; // the built PingResult

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
pub(super) fn lower_ping_posix(
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
    let host_scratch = HelperScratch::declare(&mut vregs, &mut instructions);
    // plan-132 C8: the echo packet and the receive buffer are this helper's scratch
    // and were never freed. Declared before the first branch that reaches `done`,
    // where they are released with the host C-string.
    let packet_scratch = HelperScratch::declare(&mut vregs, &mut instructions);
    let receive_scratch = HelperScratch::declare(&mut vregs, &mut instructions);
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
        // The flat `net::Address` (plan-132): slot 0 holds the host `String`'s
        // block-relative offset, so the host is `x0 + [x0]`. The port is
        // deliberately ignored: ICMP has no transport port (plan-110-A §C3).
        instructions.extend([
            abi::load_u64(&v9, abi::return_register(), 0),
            abi::add_registers(&v9, abi::return_register(), &v9),
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
        &host_scratch,
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
        abi::move_register(&packet_scratch.pointer, abi::mfb_return(1)),
        abi::load_u64(&packet_scratch.size, abi::stack_pointer(), SIZE_OFFSET),
        abi::add_immediate(&packet_scratch.size, &packet_scratch.size, 8),
        abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), PKT_OFFSET),
        abi::move_immediate(abi::return_register(), "Integer", RECV_CAPACITY),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
    ]);
    emit_alloc(symbol, &mut instructions, &mut relocations, &alloc_fail);
    instructions.extend([
        abi::move_register(&receive_scratch.pointer, abi::mfb_return(1)),
        abi::move_immediate(&receive_scratch.size, "Integer", RECV_CAPACITY),
        abi::store_u64(abi::mfb_return(1), abi::stack_pointer(), BUF_OFFSET),
    ]);

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
        DST_OFFSET,
        &AddressSlots {
            len: HOSTLEN_OFFSET,
            host: AHOST_OFFSET,
            port: APORT_OFFSET,
            record: RecordBuildScratch {
                size: ASIZE_OFFSET,
                result: ADDRREC_OFFSET,
                cursor: ACURSOR_OFFSET,
                block_size: ABLOCK_OFFSET,
            },
        },
        &alloc_fail,
        &addr_fail,
        &mut vregs,
    )?;
    instructions.extend([
        abi::load_u64(&v9, abi::stack_pointer(), ADDRREC_OFFSET),
        // ICMP has no transport port: whatever the sockaddr carried, publish 0
        // (plan-110-A §C3).
        abi::store_u64(abi::ZERO, &v9, ADDRESS_OFFSET_PORT),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), RES_OFFSET),
    ]);
    platform.emit_external_call(
        net_symbol(platform, NetSymbol::FreeAddrInfo),
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    emit_ping_result(
        symbol,
        &PingResultSlots {
            status: STATUS_OFFSET,
            address: ADDRREC_OFFSET,
            address_size: ASIZE_OFFSET,
            rtt: RTT_OFFSET,
            ttl: RTTL_OFFSET,
            size: RSIZE_OFFSET,
            result_size: PSIZE_OFFSET,
            result: PRESULT_OFFSET,
            cursor: ACURSOR_OFFSET,
            block: ABLOCK_OFFSET,
        },
        &alloc_fail,
        &mut vregs,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
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
    instructions.push(abi::label(&done));
    // bug-574: release the marshalled host C-string; the host call consumed
    // it and nothing on the MFBASIC side of this call can see it. plan-132 C8:
    // the echo packet and the receive buffer are released with it, on every exit.
    emit_helper_scratch_release(
        symbol,
        &[host_scratch, packet_scratch, receive_scratch],
        &mut vregs,
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::return_());
    Ok((instructions, relocations, FRAME_SIZE))
}
