//! `mfb build --debug` / `mfb test --debug` (plan-130).
//!
//! A `--debug` build carries a measurement report that `_mfb_shutdown` writes to
//! stderr as its last act (`_mfb_debug_shutdown`, [`shutdown`]). What the report
//! contains is decided by one ordered registry, [`DEBUG_FEATURES`]: every hook the
//! compiler has for debug output asks the registry, never `module.debug` directly,
//! so a measurement is added by adding one list entry.
//!
//! The switch travels on [`NirModule`], never as a process global: it is read only
//! in codegen, and a global would leak across the nested builds of source
//! dependencies.
//!
//! Report format (version 1): every line is `<key> <value>\n` on fd 2, keys
//! dot-separated, values a decimal integer or a single token without spaces,
//! bracketed by `mfb.debug.begin 1` … `mfb.debug.end 1`. A consumer takes the last
//! `mfb.debug.begin` block in stderr.

mod arena;
mod perf;
mod shutdown;
#[cfg(test)]
mod tests;
mod write;

pub(crate) use arena::{
    ARENA_KIND_GRAPHICS, ARENA_KIND_WORKER, ARENA_SECTION, DEBUG_ARENA_REGISTER_SYMBOL,
};
pub(crate) use perf::PERF_SECTION;
pub(crate) use shutdown::DEBUG_SHUTDOWN_SYMBOL;

use std::collections::HashMap;

use crate::codegen::engine::types::{
    CodeDataObject, CodeFunction, CodeInstruction, CodeRelocation, CodegenPlatform,
};
use crate::codegen::engine::util::{finalize_vreg_helper, Vregs};
use crate::target::shared::abi;
use crate::target::shared::nir::NirModule;

/// Per-build debug-report options, carried on `NirModule`.
///
/// A struct rather than a bare `bool` so a later per-feature selection
/// (`--debug=arena,perf`) is a new field, not another signature change across
/// every lowering entry point.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct DebugOptions {
    /// Whether this build carries the debug report at all.
    pub(crate) enabled: bool,
}

impl DebugOptions {
    /// A normal build: no report, no debug-only code or data.
    ///
    /// Test-only: production builds construct the value from the parsed
    /// `--debug` flag (`BuildOptions::debug`), so only fixtures that lower a
    /// module outside the CLI need a named "off".
    #[cfg(test)]
    pub(crate) const OFF: DebugOptions = DebugOptions { enabled: false };
}

/// The program entry's instruction stream, handed to each feature's
/// [`emit_entry_start`](DebugFeature::emit_entry_start).
///
/// The hook runs right after the entry publishes the main arena address. A `bl`
/// there clobbers only caller-saved registers: the entry's argc/argv are parked in
/// callee-saved scratch and re-materialized after it.
pub(crate) struct DebugEmitCtx<'a> {
    /// The entry function's symbol, the `from` of every relocation emitted here.
    pub(crate) entry_symbol: &'a str,
    pub(crate) instructions: &'a mut Vec<CodeInstruction>,
    pub(crate) relocations: &'a mut Vec<CodeRelocation>,
}

/// One section of the debug report and everything it needs emitted.
///
/// Every method is consulted only for a module whose report is on
/// ([`report_enabled`]) and for which [`applies`](DebugFeature::applies) holds, so
/// an implementation never re-checks the build flag.
pub(crate) trait DebugFeature: Sync {
    /// Stable section name; every report line this feature writes starts with it.
    fn name(&self) -> &'static str;
    /// Whether this feature emits anything for `module` (e.g. a target restriction).
    fn applies(&self, module: &NirModule) -> bool;
    /// Data objects the feature's code addresses (counters, key and line strings).
    fn data_objects(&self, module: &NirModule) -> Vec<CodeDataObject>;
    /// Helper functions the feature adds, its report section among them.
    fn code_functions(
        &self,
        module: &NirModule,
        platform_imports: &HashMap<String, String>,
        platform: &dyn CodegenPlatform,
    ) -> Result<Vec<CodeFunction>, String>;
    /// Runtime calls (`family.member`) whose spec-dispatched helper symbols this
    /// feature's code branches to without a NIR call the plan scan could see.
    fn runtime_calls(&self) -> &'static [&'static str];
    /// Runtime calls whose platform imports this feature's code needs.
    fn import_calls(&self) -> &'static [&'static str];
    /// Helper symbols of this feature that take the platform mutex (`pthread_mutex_lock`
    /// / `unlock`, an SRWLOCK on Windows) and so need exactly those imports.
    fn lock_helpers(&self) -> &'static [&'static str];
    /// Instructions emitted in the program entry after the main arena address is
    /// published. May emit nothing.
    fn emit_entry_start(&self, ctx: &mut DebugEmitCtx<'_>) -> Result<(), String>;
    /// This feature's report-section helper, called by `_mfb_debug_shutdown`.
    fn report_symbol(&self) -> Option<&'static str>;
}

/// The report's sections, in the order they print.
pub(crate) static DEBUG_FEATURES: &[&dyn DebugFeature] =
    &[&CoreSection, &perf::PerfFeature, &arena::ArenaFeature];

/// Whether `module` carries the report: a `--debug` build of a program with an
/// entry (a package or entry-less module has no `_mfb_shutdown` to report from).
pub(crate) fn report_enabled(module: &NirModule) -> bool {
    module.debug.enabled && module.entry.is_some()
}

/// The registry entries that emit for `module`, in report order. Empty for a
/// module without the report, so every caller is a no-op in a normal build.
pub(crate) fn active_features(
    module: &NirModule,
) -> impl Iterator<Item = &'static dyn DebugFeature> + '_ {
    DEBUG_FEATURES
        .iter()
        .copied()
        .filter(move |feature| report_enabled(module) && feature.applies(module))
}

/// Whether the section named `name` emits for `module` — for codegen outside the
/// report (e.g. the arena helpers the perf section times) that must match it.
pub(crate) fn feature_active(module: &NirModule, name: &str) -> bool {
    active_features(module).any(|feature| feature.name() == name)
}

/// Every debug-only data object for `module`: the report helper's own, then each
/// active feature's. Empty unless the report is on.
pub(crate) fn data_objects(module: &NirModule) -> Vec<CodeDataObject> {
    if !report_enabled(module) {
        return Vec::new();
    }
    let mut objects = shutdown::data_objects();
    for feature in active_features(module) {
        objects.extend(feature.data_objects(module));
    }
    objects
}

/// Every debug-only function for `module`: `_mfb_debug_shutdown`, then each active
/// feature's. Empty unless the report is on.
pub(crate) fn code_functions(
    module: &NirModule,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<Vec<CodeFunction>, String> {
    if !report_enabled(module) {
        return Ok(Vec::new());
    }
    let mut functions = vec![shutdown::lower_debug_shutdown(
        module,
        platform_imports,
        platform,
    )?];
    for feature in active_features(module) {
        functions.extend(feature.code_functions(module, platform_imports, platform)?);
    }
    Ok(functions)
}

/// The report's always-present first section: which target and build mode the
/// program was compiled for, so a collected report identifies its own run.
struct CoreSection;

const CORE_REPORT_SYMBOL: &str = "_mfb_debug_report_core";
const CORE_TARGET_LINE_SYMBOL: &str = "_mfb_rt_debug_line_target";
const CORE_BUILD_LINE_SYMBOL: &str = "_mfb_rt_debug_line_build";

impl CoreSection {
    /// `console` or `app` — the report names the axis, not the toolkit.
    fn build_token(module: &NirModule) -> &'static str {
        if module.build_mode.is_app() {
            "app"
        } else {
            "console"
        }
    }
}

impl DebugFeature for CoreSection {
    fn name(&self) -> &'static str {
        "mfb.debug"
    }

    fn applies(&self, _module: &NirModule) -> bool {
        true
    }

    fn data_objects(&self, module: &NirModule) -> Vec<CodeDataObject> {
        // Both values are known at compile time, so each line is one prebuilt
        // object written by a single `write` — it cannot interleave with another
        // thread's stderr output mid-line.
        vec![
            write::constant_line_object(
                CORE_TARGET_LINE_SYMBOL,
                &format!("{}.target", self.name()),
                &module.target,
            ),
            write::constant_line_object(
                CORE_BUILD_LINE_SYMBOL,
                &format!("{}.build", self.name()),
                Self::build_token(module),
            ),
        ]
    }

    fn code_functions(
        &self,
        _module: &NirModule,
        platform_imports: &HashMap<String, String>,
        platform: &dyn CodegenPlatform,
    ) -> Result<Vec<CodeFunction>, String> {
        let mut vregs = Vregs::new();
        let mut instructions = vec![abi::label("entry")];
        let mut relocations = Vec::new();
        for line in [CORE_TARGET_LINE_SYMBOL, CORE_BUILD_LINE_SYMBOL] {
            write::emit_debug_constant_line(
                CORE_REPORT_SYMBOL,
                line,
                platform_imports,
                platform,
                &mut instructions,
                &mut relocations,
                &mut vregs,
            )?;
        }
        instructions.push(abi::return_());
        Ok(vec![finalize_vreg_helper(
            "runtime.debug_report_core",
            CORE_REPORT_SYMBOL,
            "Nothing",
            instructions,
            relocations,
        )])
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

    fn emit_entry_start(&self, _ctx: &mut DebugEmitCtx<'_>) -> Result<(), String> {
        // The target and build mode are compile-time constants: nothing to record
        // while the program runs.
        Ok(())
    }

    fn report_symbol(&self) -> Option<&'static str> {
        Some(CORE_REPORT_SYMBOL)
    }
}
