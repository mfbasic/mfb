//! The `perf` report section (plan-130-B): plan-67's runtime timings of the whole
//! program, `_mfb_arena_alloc`, and `_mfb_arena_free`, switched on by `--debug`.
//!
//! The timing helpers themselves (`perf.init/start/end/done`) are unchanged and
//! live in `codegen::builtins::perf`. This section owns when they are emitted and
//! called: the region is mapped and the `program` span opened at entry
//! ([`DebugFeature::emit_entry_start`]); the span is closed and the table printed
//! by `_mfb_debug_report_perf`, which `_mfb_debug_shutdown` calls. The arena
//! helpers bracket their bodies with `perf.start`/`perf.end` when
//! `feature_active(module, PERF_SECTION)` holds.
//!
//! macOS only: the timing helpers read the clock through the Darwin
//! `clock_gettime` sequence and are inert bodies elsewhere.

use std::collections::HashMap;

use super::{DebugEmitCtx, DebugFeature};
use crate::codegen::engine::builder::internal_branch;
use crate::codegen::engine::types::{CodeDataObject, CodeFunction, CodegenPlatform};
use crate::codegen::engine::util::finalize_vreg_helper;
use crate::codegen::error::constants::{
    PERF_HEADER_SYMBOL, PERF_NAME_MFB_ALLOC_SYMBOL, PERF_NAME_MFB_FREE_SYMBOL,
    PERF_NAME_MISMATCH_SYMBOL, PERF_NAME_OVERFLOW_SYMBOL, PERF_NAME_PROGRAM_SYMBOL,
    PERF_STATE_SYMBOL,
};
use crate::codegen::memory::data::{push_symbol_address, string_data_object};
use crate::target::shared::abi;
use crate::target::shared::nir::NirModule;
use crate::target::shared::runtime::{symbol_for_call, RuntimeHelper};

/// The section's name: its report lines start with it, and the arena helpers ask
/// `feature_active(module, PERF_SECTION)` to decide whether to time themselves.
pub(crate) const PERF_SECTION: &str = "perf";

const PERF_REPORT_SYMBOL: &str = "_mfb_debug_report_perf";

/// The four timing helpers, in the order a run reaches them.
const PERF_CALLS: &[&str] = &["perf.init", "perf.start", "perf.end", "perf.done"];

pub(super) struct PerfFeature;

/// The emitted symbol of a perf runtime call (`_mfb_rt_perf_perf_init`, …).
fn perf_symbol(call: &str) -> String {
    symbol_for_call(RuntimeHelper::Perf, call)
}

impl DebugFeature for PerfFeature {
    fn name(&self) -> &'static str {
        PERF_SECTION
    }

    fn applies(&self, module: &NirModule) -> bool {
        module.target == "macos-aarch64"
    }

    fn data_objects(&self, _module: &NirModule) -> Vec<CodeDataObject> {
        vec![
            // The region base (0 until `perf.init` maps it; every helper treats 0 as
            // "perf inert").
            CodeDataObject {
                symbol: PERF_STATE_SYMBOL.to_string(),
                kind: "raw".to_string(),
                layout: "mfb.runtime.perf_state.v1 { u64 regionBase }".to_string(),
                align: 8,
                size: 8,
                value: "0000000000000000".to_string(),
            },
            string_data_object(
                PERF_HEADER_SYMBOL,
                "name count avg median min max sum\n".to_string(),
            ),
            // Span names (compared by pointer) and the diagnostic counter rows.
            string_data_object(PERF_NAME_PROGRAM_SYMBOL, "program".to_string()),
            string_data_object(PERF_NAME_MISMATCH_SYMBOL, "mismatch".to_string()),
            string_data_object(PERF_NAME_OVERFLOW_SYMBOL, "overflow".to_string()),
            string_data_object(PERF_NAME_MFB_ALLOC_SYMBOL, "mfb_alloc".to_string()),
            string_data_object(PERF_NAME_MFB_FREE_SYMBOL, "mfb_free".to_string()),
        ]
    }

    fn code_functions(
        &self,
        _module: &NirModule,
        _platform_imports: &HashMap<String, String>,
        _platform: &dyn CodegenPlatform,
    ) -> Result<Vec<CodeFunction>, String> {
        // `_mfb_debug_report_perf`: close the whole-program span, then print the
        // table. Both helpers are arena-free (they read only `_mfb_rt_perf_state`),
        // which is what lets them run after `_mfb_arena_destroy`.
        let symbol = PERF_REPORT_SYMBOL;
        let perf_end = perf_symbol("perf.end");
        let perf_done = perf_symbol("perf.done");
        let mut instructions = vec![abi::label("entry")];
        let mut relocations = Vec::new();
        push_symbol_address(
            symbol,
            PERF_NAME_PROGRAM_SYMBOL,
            abi::c_arg(0),
            &mut instructions,
            &mut relocations,
        );
        instructions.push(abi::branch_link(&perf_end));
        relocations.push(internal_branch(symbol, &perf_end));
        instructions.push(abi::branch_link(&perf_done));
        relocations.push(internal_branch(symbol, &perf_done));
        instructions.push(abi::return_());
        Ok(vec![finalize_vreg_helper(
            "runtime.debug_report_perf",
            symbol,
            "Nothing",
            instructions,
            relocations,
        )])
    }

    fn runtime_calls(&self) -> &'static [&'static str] {
        PERF_CALLS
    }

    fn import_calls(&self) -> &'static [&'static str] {
        // `perf.start`/`perf.end` read the monotonic clock through `_clock_gettime`;
        // `perf.init`/`perf.done` need no libc import (mmap and write ride the
        // platform's own seams).
        &["perf.start"]
    }

    fn emit_entry_start(&self, ctx: &mut DebugEmitCtx<'_>) -> Result<(), String> {
        // Map the region (arena-free) and open the whole-program span.
        let perf_init = perf_symbol("perf.init");
        ctx.instructions.push(abi::branch_link(&perf_init));
        ctx.relocations
            .push(internal_branch(ctx.entry_symbol, &perf_init));
        let perf_start = perf_symbol("perf.start");
        push_symbol_address(
            ctx.entry_symbol,
            PERF_NAME_PROGRAM_SYMBOL,
            abi::c_arg(0),
            ctx.instructions,
            ctx.relocations,
        );
        ctx.instructions.push(abi::branch_link(&perf_start));
        ctx.relocations
            .push(internal_branch(ctx.entry_symbol, &perf_start));
        Ok(())
    }

    fn report_symbol(&self) -> Option<&'static str> {
        Some(PERF_REPORT_SYMBOL)
    }
}
