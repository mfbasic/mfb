//! The `thread` report section (plan-156-A): the process-global thread table, read when
//! the report runs.
//!
//! Three lines: `thread.table_capacity <n>` (slots the table's array holds — it starts
//! at none and doubles from 64 when full, so a value above 64 proves it grew),
//! `thread.table_high <n>` (slots ever handed out: the peak number of control blocks
//! alive at once), and `thread.table_live <n>` (control blocks not yet freed — each is
//! a thread whose handle side or worker has not let go).
//!
//! Present only for a program that starts threads, which is exactly when the table
//! (`_mfb_rt_thread_table`) is emitted. The words are read without the table lock: a
//! still-running worker may be releasing its block, and a word-sized snapshot is what
//! the report promises.

use std::collections::HashMap;

use super::write::{emit_debug_key_value, key_object, DEBUG_LINE_BUFFER_SIZE};
use super::{DebugEmitCtx, DebugFeature};
use crate::codegen::engine::analysis::module_analysis::module_uses_call;
use crate::codegen::engine::types::{CodeDataObject, CodeFunction, CodegenPlatform};
use crate::codegen::engine::util::{finalize_vreg_body_with_locals, Vregs};
use crate::codegen::memory::data::push_symbol_address;
use crate::codegen::runtime::thread::{
    THREAD_TABLE_CAPACITY, THREAD_TABLE_HIGH, THREAD_TABLE_SYMBOL, THREAD_TABLE_USED,
};
use crate::target::shared::abi;
use crate::target::shared::nir::NirModule;
use crate::target::shared::plan::{NativePlanPlatform, PlatformImport};

/// The section's name: its report lines start with it.
pub(crate) const THREAD_SECTION: &str = "thread";

const THREAD_REPORT_SYMBOL: &str = "_mfb_debug_report_thread";

/// `(key symbol, report key, table word)` for each line, in report order.
const THREAD_LINES: [(&str, &str, usize); 3] = [
    (
        "_mfb_rt_debug_thread_key_table_capacity",
        "thread.table_capacity",
        THREAD_TABLE_CAPACITY,
    ),
    (
        "_mfb_rt_debug_thread_key_table_high",
        "thread.table_high",
        THREAD_TABLE_HIGH,
    ),
    (
        "_mfb_rt_debug_thread_key_table_live",
        "thread.table_live",
        THREAD_TABLE_USED,
    ),
];

pub(super) struct ThreadFeature;

fn lower_report(
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<CodeFunction, String> {
    let symbol = THREAD_REPORT_SYMBOL;
    let mut vregs = Vregs::new();
    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    for (key_symbol, key, word) in THREAD_LINES {
        let table = vregs.next();
        let value = vregs.next();
        push_symbol_address(
            symbol,
            THREAD_TABLE_SYMBOL,
            &table,
            &mut instructions,
            &mut relocations,
        );
        instructions.push(abi::load_u64(&value, &table, word));
        emit_debug_key_value(
            symbol,
            key_symbol,
            &value,
            key.trim_start_matches("thread."),
            platform_imports,
            platform,
            &mut instructions,
            &mut relocations,
            &mut vregs,
        )?;
    }
    instructions.push(abi::return_());
    let (frame, stack_slots) =
        finalize_vreg_body_with_locals(&mut instructions, &[], DEBUG_LINE_BUFFER_SIZE);
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

impl DebugFeature for ThreadFeature {
    fn name(&self) -> &'static str {
        THREAD_SECTION
    }

    fn applies(&self, module: &NirModule) -> bool {
        module_uses_call(module, "thread.start")
    }

    fn data_objects(&self, _module: &NirModule) -> Vec<CodeDataObject> {
        THREAD_LINES
            .iter()
            .map(|(key_symbol, key, _)| key_object(key_symbol, key))
            .collect()
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
        _platform: &dyn NativePlanPlatform,
        _required_by: &str,
    ) -> Vec<PlatformImport> {
        Vec::new()
    }

    fn emit_entry_start(&self, _ctx: &mut DebugEmitCtx<'_>) -> Result<(), String> {
        Ok(())
    }

    fn report_symbol(&self) -> Option<&'static str> {
        Some(THREAD_REPORT_SYMBOL)
    }
}
