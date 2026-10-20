//! plan-176-T: the Windows `net.ping` backend (`IcmpSendEcho2`) — its status codes, reply layout, frame
//! and `lower_ping_windows`. The shared result helpers live in `builtins/net/gen_ping/mod.rs`.

use super::*;

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

const W_APORT: usize = 288; // the port the Address is built with

const W_ASIZE: usize = 296; // the built Address's byte size

const W_ACURSOR: usize = 304; // record marshaller scratch

const W_ABLOCK: usize = 312; // record marshaller scratch

const W_PSIZE: usize = 320; // the PingResult's byte size

const W_PRESULT: usize = 328; // the built PingResult

/// The Windows ICMP backend — `iphlpapi`, not a socket.
#[allow(clippy::too_many_lines)]
pub(super) fn lower_ping_windows(
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
    let host_scratch_win = HelperScratch::declare(&mut vregs, &mut instructions);
    // plan-132 C8: the request payload and the reply buffer are this helper's
    // scratch and were never freed. Declared before the first branch that reaches
    // `done`, where they are released with the host C-string.
    let request_scratch_win = HelperScratch::declare(&mut vregs, &mut instructions);
    let reply_scratch_win = HelperScratch::declare(&mut vregs, &mut instructions);
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
        // The flat `net::Address` (plan-132): the host is `x0 + [x0]`.
        instructions.extend([
            abi::load_u64(&v9, abi::return_register(), 0),
            abi::add_registers(&v9, abi::return_register(), &v9),
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
        &host_scratch_win,
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
        abi::move_register(&request_scratch_win.pointer, abi::mfb_return(1)),
        abi::load_u64(&request_scratch_win.size, abi::stack_pointer(), W_SIZE),
        abi::add_immediate(&request_scratch_win.size, &request_scratch_win.size, 1),
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
        abi::move_register(&reply_scratch_win.pointer, abi::mfb_return(1)),
        abi::load_u64(&reply_scratch_win.size, abi::stack_pointer(), W_REPLY_LEN),
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
        W_DST,
        &AddressSlots {
            len: W_HOSTLEN,
            host: W_AHOST,
            port: W_APORT,
            record: RecordBuildScratch {
                size: W_ASIZE,
                result: W_ADDRREC,
                cursor: W_ACURSOR,
                block_size: W_ABLOCK,
            },
        },
        &alloc_fail,
        &addr_fail,
        &mut vregs,
    )?;
    instructions.extend([
        abi::load_u64(&v9, abi::stack_pointer(), W_ADDRREC),
        // ICMP has no transport port (plan-110-A §C3).
        abi::store_u64(abi::ZERO, &v9, ADDRESS_OFFSET_PORT),
        abi::load_u64(abi::return_register(), abi::stack_pointer(), W_RES),
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
            status: W_STATUS,
            address: W_ADDRREC,
            address_size: W_ASIZE,
            rtt: W_RTT,
            ttl: W_RTTL,
            size: W_RSIZE,
            result_size: W_PSIZE,
            result: W_PRESULT,
            cursor: W_ACURSOR,
            block: W_ABLOCK,
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
    instructions.push(abi::label(&done));
    // bug-574: release the marshalled host C-string; the host call consumed
    // it and nothing on the MFBASIC side of this call can see it. plan-132 C8:
    // the request payload and the reply buffer are released with it, on every exit.
    emit_helper_scratch_release(
        symbol,
        &[host_scratch_win, request_scratch_win, reply_scratch_win],
        &mut vregs,
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::return_());
    Ok((instructions, relocations, WIN_FRAME_SIZE))
}
