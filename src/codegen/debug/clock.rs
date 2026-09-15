//! A monotonic clock reading for `--debug` helpers (plan-133-C): the arena memory
//! series stamps each sample with it.
//!
//! - macOS and Linux: `clock_gettime(CLOCK_MONOTONIC, &ts)` with the platform's clock
//!   id (Darwin 6, Linux 1), then `tv_sec * 1e9 + tv_nsec`. Both entries already import
//!   `clock_gettime` for the arena-fill seed.
//! - Windows: `QueryPerformanceCounter` and `QueryPerformanceFrequency`, then the
//!   overflow-safe fold `(counter / freq) * 1e9 + ((counter % freq) * 1e9) / freq`
//!   (`datetime::monotonicNanos` does the same; `counter * 1e9` alone overflows u64
//!   within about 21 s at a 10 MHz frequency).
//!
//! The reading lands in a vreg, never a physical register, so a helper finalized by
//! the vreg allocator can use it. The caller reserves 16 bytes of frame locals at
//! `buffer_offset` for the `timespec` (or the two Windows words).

use std::collections::HashMap;

use crate::codegen::engine::types::{
    CodeInstruction, CodeRelocation, CodegenPlatform, PlatformFamily,
};
use crate::codegen::engine::util::Vregs;
use crate::target::shared::abi;

/// Frame bytes [`emit_debug_monotonic_nanos`] needs at its `buffer_offset`.
pub(super) const CLOCK_BUFFER_SIZE: usize = 16;

const NANOS_PER_SECOND: &str = "1000000000";

/// `dst = monotonic nanoseconds`. Clobbers the call's registers; `dst` and every live
/// vreg survive (the allocator spills them across the external calls).
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_debug_monotonic_nanos(
    from: &str,
    dst: &str,
    buffer_offset: usize,
    platform: &dyn CodegenPlatform,
    platform_imports: &HashMap<String, String>,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    vregs: &mut Vregs,
) -> Result<(), String> {
    if platform.family() == PlatformFamily::Windows {
        instructions.push(abi::add_immediate(
            abi::c_arg(0),
            abi::stack_pointer(),
            buffer_offset,
        ));
        platform.emit_external_call(
            "QueryPerformanceCounter",
            from,
            platform_imports,
            instructions,
            relocations,
        )?;
        instructions.push(abi::add_immediate(
            abi::c_arg(0),
            abi::stack_pointer(),
            buffer_offset + 8,
        ));
        platform.emit_external_call(
            "QueryPerformanceFrequency",
            from,
            platform_imports,
            instructions,
            relocations,
        )?;
        let counter = vregs.next();
        let freq = vregs.next();
        let quotient = vregs.next();
        let remainder = vregs.next();
        let scale = vregs.next();
        instructions.extend([
            abi::load_u64(&counter, abi::stack_pointer(), buffer_offset),
            abi::load_u64(&freq, abi::stack_pointer(), buffer_offset + 8),
            abi::unsigned_divide_registers(&quotient, &counter, &freq),
            abi::multiply_subtract_registers(&remainder, &quotient, &freq, &counter),
            abi::move_immediate(&scale, "Integer", NANOS_PER_SECOND),
            abi::multiply_registers(&quotient, &quotient, &scale),
            abi::multiply_registers(&remainder, &remainder, &scale),
            abi::unsigned_divide_registers(&remainder, &remainder, &freq),
            abi::add_registers(dst, &quotient, &remainder),
        ]);
        return Ok(());
    }
    instructions.extend([
        abi::move_immediate(abi::c_arg(0), "Integer", platform.clock_monotonic()),
        abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), buffer_offset),
    ]);
    platform.emit_external_call(
        "clock_gettime",
        from,
        platform_imports,
        instructions,
        relocations,
    )?;
    let seconds = vregs.next();
    let nanos = vregs.next();
    let scale = vregs.next();
    instructions.extend([
        abi::load_u64(&seconds, abi::stack_pointer(), buffer_offset),
        abi::load_u64(&nanos, abi::stack_pointer(), buffer_offset + 8),
        abi::move_immediate(&scale, "Integer", NANOS_PER_SECOND),
        abi::multiply_registers(&seconds, &seconds, &scale),
        abi::add_registers(dst, &seconds, &nanos),
    ]);
    Ok(())
}
