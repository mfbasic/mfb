//! The `net.ping` OS seam — real ICMP echo (plan-110-A).
//!
//! Three backends, not the two the plan first assumed. `tools/net-probes/icmp-capability-probe.c`
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
use crate::target::shared::abi::Reg;

use crate::codegen::memory::arena::{emit_helper_scratch_release, HelperScratch};
use crate::codegen::memory::marshal::{
    emit_build_inlined_record_sized, MarshalRegs, RecordBuildScratch,
};
use crate::codegen::os::socket::shared::{
    emit_address_from_sockaddr, emit_cstring, emit_fd_cloexec_fallback, emit_hints,
    emit_pollfd_events, emit_socket_type_cloexec, net_symbol, AddressSlots, NetBodyParts,
    NetSymbol, AF_INET, SOCKADDR_STORAGE_SIZE, SOCK_DGRAM,
};
use crate::types::ParameterType;

mod posix;
mod windows;

use posix::lower_ping_posix;
use windows::lower_ping_windows;

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

/// The `Address` record's port slot, forced to 0 because ICMP has no transport port.
const ADDRESS_OFFSET_PORT: usize = 8;

/// The frame slots a backend hands [`emit_ping_result`]: the five field values in
/// declaration order — `address` is the built `net::Address` block, with its byte
/// size beside it — and the record marshaller's scratch.
struct PingResultSlots {
    status: usize,
    address: usize,
    address_size: usize,
    rtt: usize,
    ttl: usize,
    size: usize,
    result_size: usize,
    result: usize,
    cursor: usize,
    block: usize,
}

/// Build the `net::PingResult` both backends return, through the record marshaller
/// (plan-132): `address` is inlined from the backend's built `net::Address`, whose
/// size the address builder left, and the other four fields are slot values
/// (`rttMs` carries raw f64 bits). The `Address` block is then scratch — the result
/// holds its own copy, and nothing outside this helper ever held the pointer — so it
/// is freed with its size. The result is left in `RESULT_VALUE_REGISTER`.
fn emit_ping_result(
    symbol: &str,
    slots: &PingResultSlots,
    alloc_fail: &str,
    vregs: &mut Vregs,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    emit_build_inlined_record_sized(
        symbol,
        "ping_result",
        &ParameterType::named(super::PING_RESULT_TYPE_ID),
        TypeModel::builtin_records(),
        &[
            slots.status,
            slots.address,
            slots.rtt,
            slots.ttl,
            slots.size,
        ],
        &[None, Some(slots.address_size), None, None, None],
        &RecordBuildScratch {
            size: slots.result_size,
            result: slots.result,
            cursor: slots.cursor,
            block_size: slots.block,
        },
        &MarshalRegs::fresh(vregs),
        abi::mfb_return(1),
        alloc_fail,
        instructions,
        relocations,
    )?;
    instructions.extend([
        abi::load_u64(abi::c_arg(0), abi::stack_pointer(), slots.address),
        abi::load_u64(abi::c_arg(1), abi::stack_pointer(), slots.address_size),
    ]);
    emit_arena_free(symbol, instructions, relocations);
    instructions.push(abi::load_u64(
        RESULT_VALUE_REGISTER,
        abi::stack_pointer(),
        slots.result,
    ));
    Ok(())
}

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

    /// Both backends build `PingResult` through the record marshaller, handing it
    /// five frame slots in exactly this order: the status, the built `Address`, the
    /// round-trip time, the TTL and the size. The marshaller lays fields out in
    /// declaration order, so inserting or reordering a field would silently assign
    /// each value to the wrong one.
    ///
    /// plan-132: `address` is a flat `net::Address` inlined into the result — the
    /// backends pass its block's known size — and the other four stay slot values.
    #[test]
    fn ping_result_fields_match_what_the_backends_hand_the_marshaller() {
        let record = net_package()
            .records()
            .iter()
            .find(|r| r.name == super::super::PING_RESULT_TYPE)
            .expect("PingResult record");
        let names: Vec<&str> = record.props.iter().map(|p| p.name).collect();
        assert_eq!(names, ["status", "address", "rttMs", "ttl", "size"]);
        let model = crate::codegen::engine::builder::TypeModel::builtin_records();
        let result = crate::types::ParameterType::declared(super::super::PING_RESULT_TYPE_ID);
        let fields = model.record_fields.get(&result).expect("PingResult layout");
        let inlined: Vec<bool> = fields
            .iter()
            .map(|(_, ty)| crate::codegen::collection::layout::record_field_is_inlined(model, ty))
            .collect();
        assert_eq!(inlined, [false, true, false, false, false]);
        // Both backends zero the built Address's port slot: ICMP has no port.
        let address = model
            .record_fields
            .get(&crate::types::ParameterType::declared(
                super::super::ADDRESS_TYPE_ID,
            ))
            .expect("Address layout");
        assert_eq!(address[ADDRESS_OFFSET_PORT / 8].0, "port");
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
