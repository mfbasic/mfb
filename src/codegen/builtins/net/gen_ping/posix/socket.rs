//! plan-176-T: the POSIX `net.ping` socket helpers — socket options, the monotonic clock and the ICMP
//! checksum. They read the frame offsets in `builtins/net/gen_ping/posix/mod.rs`.

use super::*;

const NANOS_PER_SECOND: &str = "1000000000";

/// Which value an [`emit_int_sockopt`] call writes into the `optval` slot.
pub(super) enum SockOptSource {
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
pub(super) fn emit_int_sockopt(
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
pub(super) fn emit_monotonic_nanos(
    symbol: &str,
    platform: &dyn CodegenPlatform,
    platform_imports: &HashMap<String, String>,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    vregs: &mut Vregs,
    dst: impl Into<Reg>,
) -> Result<(), String> {
    let dst: Reg = dst.into();
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
pub(super) fn emit_icmp_checksum(
    instructions: &mut Vec<CodeInstruction>,
    vregs: &mut Vregs,
    pkt: impl Into<Reg>,
    sum_loop: &str,
    sum_done: &str,
    sum_odd: &str,
) {
    let pkt: Reg = pkt.into();
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
