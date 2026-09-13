//! The `process` report section (plan-130-D): the process's peak resident set size,
//! as the operating system counts it, read when the report runs.
//!
//! Arena counters say how much the allocator mapped; this says how much memory the
//! process actually touched. One line: `process.peak_rss_bytes <n>`, in bytes on
//! every target.
//!
//! - macOS and Linux: `getrusage(RUSAGE_SELF, &ru)`, `ru.ru_maxrss` at offset 32.
//!   Darwin counts bytes; Linux counts KiB, so the value is scaled. musl's
//!   `struct rusage` is 272 bytes (glibc and Darwin: 144), so the buffer is sized for
//!   musl.
//! - Windows: `K32GetProcessMemoryInfo(GetCurrentProcess(), &pmc, 72)`,
//!   `pmc.PeakWorkingSetSize` at offset 8.
//!
//! The buffer lives in the helper's own frame, above the line window: the report runs
//! after `_mfb_arena_destroy`. The peak word is zeroed before the call, so a failed
//! call reports 0 rather than stack contents. The imports come from each backend's
//! `NativePlanPlatform::peak_rss_imports`, attributed to the program entry.

use std::collections::HashMap;

use super::write::{emit_debug_key_value, key_object, DEBUG_LINE_BUFFER_SIZE};
use super::{DebugEmitCtx, DebugFeature};
use crate::codegen::engine::types::{
    CodeDataObject, CodeFunction, CodegenPlatform, PlatformFamily,
};
use crate::codegen::engine::util::{finalize_vreg_body_with_locals, Vregs};
use crate::target::shared::abi;
use crate::target::shared::nir::NirModule;
use crate::target::shared::plan::{NativePlanPlatform, PlatformImport};

/// The section's name: its report line starts with it.
pub(crate) const PROCESS_SECTION: &str = "process";

const PROCESS_REPORT_SYMBOL: &str = "_mfb_debug_report_process";
const PEAK_RSS_KEY_SYMBOL: &str = "_mfb_rt_debug_process_key_peak_rss";

/// Where the OS result buffer starts in the helper's frame: above the line window
/// `emit_debug_key_value` assembles at `[sp, sp + DEBUG_LINE_BUFFER_SIZE)`.
const RESULT_BUFFER_OFFSET: usize = DEBUG_LINE_BUFFER_SIZE;
/// `sizeof(struct rusage)` on musl, the largest of the Unix targets (glibc and
/// Darwin: 144). Measured by running on every box (plan-130-D Corrections).
const RUSAGE_SIZE: usize = 272;
/// `offsetof(struct rusage, ru_maxrss)` on every Unix target: two 16-byte `timeval`s.
const RU_MAXRSS_OFFSET: usize = 32;
/// `RUSAGE_SELF`.
const RUSAGE_SELF: &str = "0";
/// `sizeof(PROCESS_MEMORY_COUNTERS)` on x64 (the call rejects 64).
const PMC_SIZE: usize = 72;
/// `offsetof(PROCESS_MEMORY_COUNTERS, PeakWorkingSetSize)`, after `cb` and
/// `PageFaultCount`.
const PMC_PEAK_WORKING_SET_OFFSET: usize = 8;

pub(super) struct ProcessFeature;

fn lower_report(
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<CodeFunction, String> {
    let symbol = PROCESS_REPORT_SYMBOL;
    let mut vregs = Vregs::new();
    let zero = vregs.next();
    let peak = vregs.next();
    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    instructions.push(abi::move_immediate(&zero, "Integer", "0"));
    match platform.family() {
        PlatformFamily::Windows => {
            let size = vregs.next();
            instructions.push(abi::store_u64(
                &zero,
                abi::stack_pointer(),
                RESULT_BUFFER_OFFSET + PMC_PEAK_WORKING_SET_OFFSET,
            ));
            platform.emit_external_call(
                "GetCurrentProcess",
                symbol,
                platform_imports,
                &mut instructions,
                &mut relocations,
            )?;
            instructions.extend([
                abi::move_register(abi::c_arg(0), abi::c_return(0)),
                abi::move_immediate(&size, "Integer", &PMC_SIZE.to_string()),
                abi::store_u32(&size, abi::stack_pointer(), RESULT_BUFFER_OFFSET),
                abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), RESULT_BUFFER_OFFSET),
                abi::move_immediate(abi::c_arg(2), "Integer", &PMC_SIZE.to_string()),
            ]);
            platform.emit_external_call(
                "K32GetProcessMemoryInfo",
                symbol,
                platform_imports,
                &mut instructions,
                &mut relocations,
            )?;
            instructions.push(abi::load_u64(
                &peak,
                abi::stack_pointer(),
                RESULT_BUFFER_OFFSET + PMC_PEAK_WORKING_SET_OFFSET,
            ));
        }
        PlatformFamily::MacOS | PlatformFamily::Linux => {
            instructions.extend([
                abi::store_u64(
                    &zero,
                    abi::stack_pointer(),
                    RESULT_BUFFER_OFFSET + RU_MAXRSS_OFFSET,
                ),
                abi::move_immediate(abi::c_arg(0), "Integer", RUSAGE_SELF),
                abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), RESULT_BUFFER_OFFSET),
            ]);
            platform.emit_external_call(
                "getrusage",
                symbol,
                platform_imports,
                &mut instructions,
                &mut relocations,
            )?;
            instructions.push(abi::load_u64(
                &peak,
                abi::stack_pointer(),
                RESULT_BUFFER_OFFSET + RU_MAXRSS_OFFSET,
            ));
            if platform.family() == PlatformFamily::Linux {
                // Linux `ru_maxrss` is KiB.
                instructions.push(abi::shift_left_immediate(&peak, &peak, 10));
            }
        }
    }
    emit_debug_key_value(
        symbol,
        PEAK_RSS_KEY_SYMBOL,
        &peak,
        "peak_rss",
        platform_imports,
        platform,
        &mut instructions,
        &mut relocations,
        &mut vregs,
    )?;
    instructions.push(abi::return_());
    let (frame, stack_slots) = finalize_vreg_body_with_locals(
        &mut instructions,
        &[],
        RESULT_BUFFER_OFFSET + RUSAGE_SIZE.max(PMC_SIZE),
    );
    Ok(CodeFunction {
        name: symbol.to_string(),
        symbol: symbol.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame,
        instructions,
        relocations,
        stack_slots,
    })
}

impl DebugFeature for ProcessFeature {
    fn name(&self) -> &'static str {
        PROCESS_SECTION
    }

    fn applies(&self, _module: &NirModule) -> bool {
        true
    }

    fn data_objects(&self, _module: &NirModule) -> Vec<CodeDataObject> {
        vec![key_object(PEAK_RSS_KEY_SYMBOL, "process.peak_rss_bytes")]
    }

    fn code_functions(
        &self,
        _module: &NirModule,
        platform_imports: &HashMap<String, String>,
        platform: &dyn CodegenPlatform,
    ) -> Result<Vec<CodeFunction>, String> {
        Ok(vec![lower_report(platform_imports, platform)?])
    }

    fn runtime_calls(&self) -> &'static [&'static str] {
        &[]
    }

    fn import_calls(&self) -> &'static [&'static str] {
        &[]
    }

    fn lock_helpers(&self) -> &'static [&'static str] {
        &[]
    }

    fn os_imports(
        &self,
        platform: &dyn NativePlanPlatform,
        required_by: &str,
    ) -> Vec<PlatformImport> {
        platform.peak_rss_imports(required_by)
    }

    fn emit_entry_start(&self, _ctx: &mut DebugEmitCtx<'_>) -> Result<(), String> {
        Ok(())
    }

    fn report_symbol(&self) -> Option<&'static str> {
        Some(PROCESS_REPORT_SYMBOL)
    }
}
