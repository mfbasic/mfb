//! `_mfb_debug_shutdown`: the debug report, run as the last call of
//! `_mfb_shutdown` (plan-130-A §4.5).
//!
//! `_mfb_shutdown` calls it after its `shutdown_done` label, so both of its paths
//! reach it — the full teardown and the early return a second entry takes (a
//! SIGINT/SIGTERM arriving during normal cleanup). The once-guard here makes that
//! second arrival print nothing. The helper reads only its own globals: no arena
//! access (the arena is already destroyed), no read of the arena register, and it
//! leaves the exit code parked in the entry frame untouched.

use std::collections::HashMap;

use super::write::{emit_debug_key_value, key_object, DEBUG_LINE_BUFFER_SIZE};
use crate::codegen::engine::builder::internal_branch;
use crate::codegen::engine::types::{CodeDataObject, CodeFunction, CodegenPlatform};
use crate::codegen::engine::util::{finalize_vreg_body_with_locals, Vregs};
use crate::codegen::memory::data::push_symbol_address;
use crate::target::shared::abi;
use crate::target::shared::nir::NirModule;

pub(crate) const DEBUG_SHUTDOWN_SYMBOL: &str = "_mfb_debug_shutdown";

/// Writable once-guard: 0 until the report has started printing.
const DEBUG_REPORTED_SYMBOL: &str = "_mfb_rt_debug_reported";
const DEBUG_BEGIN_KEY_SYMBOL: &str = "_mfb_rt_debug_key_begin";
const DEBUG_END_KEY_SYMBOL: &str = "_mfb_rt_debug_key_end";

/// The report format version printed after `begin` and `end`.
const DEBUG_FORMAT_VERSION: &str = "1";

pub(super) fn data_objects() -> Vec<CodeDataObject> {
    vec![
        CodeDataObject {
            symbol: DEBUG_REPORTED_SYMBOL.to_string(),
            kind: "raw".to_string(),
            layout: "mfb.runtime.debug_reported.v1 { u64 reported }".to_string(),
            align: 8,
            size: 8,
            value: "0000000000000000".to_string(),
        },
        key_object(DEBUG_BEGIN_KEY_SYMBOL, "mfb.debug.begin"),
        key_object(DEBUG_END_KEY_SYMBOL, "mfb.debug.end"),
    ]
}

pub(super) fn lower_debug_shutdown(
    module: &NirModule,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<CodeFunction, String> {
    let symbol = DEBUG_SHUTDOWN_SYMBOL;
    let done = "debug_shutdown_done";
    let mut vregs = Vregs::new();
    let guard = vregs.next();
    let reported = vregs.next();
    let marker = vregs.next();
    let begin_version = vregs.next();
    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    push_symbol_address(
        symbol,
        DEBUG_REPORTED_SYMBOL,
        &guard,
        &mut instructions,
        &mut relocations,
    );
    // The guard is set before anything prints, so an entry that interrupts the
    // report (a signal during the normal exit's report) prints nothing rather than
    // a second, nested block.
    instructions.extend([
        abi::load_u64(&reported, &guard, 0),
        abi::compare_immediate(&reported, "0"),
        abi::branch_ne(done),
        abi::move_immediate(&marker, "Integer", "1"),
        abi::store_u64(&marker, &guard, 0),
        abi::move_immediate(&begin_version, "Integer", DEBUG_FORMAT_VERSION),
    ]);
    emit_debug_key_value(
        symbol,
        DEBUG_BEGIN_KEY_SYMBOL,
        &begin_version,
        "begin",
        platform_imports,
        platform,
        &mut instructions,
        &mut relocations,
        &mut vregs,
    )?;
    for feature in super::active_features(module) {
        if let Some(report) = feature.report_symbol() {
            instructions.push(abi::branch_link(report));
            relocations.push(internal_branch(symbol, report));
        }
    }
    let end_version = vregs.next();
    instructions.push(abi::move_immediate(
        &end_version,
        "Integer",
        DEBUG_FORMAT_VERSION,
    ));
    emit_debug_key_value(
        symbol,
        DEBUG_END_KEY_SYMBOL,
        &end_version,
        "end",
        platform_imports,
        platform,
        &mut instructions,
        &mut relocations,
        &mut vregs,
    )?;
    instructions.extend([abi::label(done), abi::return_()]);
    let (frame, stack_slots) =
        finalize_vreg_body_with_locals(&mut instructions, &[], DEBUG_LINE_BUFFER_SIZE);
    Ok(CodeFunction {
        name: "runtime.debug_shutdown".to_string(),
        symbol: symbol.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame,
        instructions,
        relocations,
        stack_slots,
    })
}
