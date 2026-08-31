use crate::ir::IrProject;
use crate::target::NativeBuildMode;
use std::path::PathBuf;

use super::nir::{self, NirModule};
use super::runtime;

pub fn lower_project(
    ir: &IrProject,
    target_name: String,
    packages: &[PathBuf],
    build_mode: NativeBuildMode,
    // plan-15 D3: stdin broadcast-log backpressure cap from the manifest `"config"`
    // section, or `None` to bake the default (used by every non-executable / dump path).
    stdin_log_cap: Option<u64>,
) -> Result<NirModule, String> {
    // Merge imported packages' Binary Representation into the project up front so runtime
    // helper detection and codegen both see the complete, unified function set.
    //
    // The `-vv` spans (`crate::trace`) split the `codegen: lowering module`
    // stage into its four steps. This is the sole `NirModule` producer, so one
    // set of spans here covers all five targets.
    let merged = crate::trace::timed("merge packages", || nir::merge_packages(ir, packages))?;
    let helpers = crate::trace::timed("required helpers", || runtime::required_helpers(&merged));
    let module = crate::trace::timed("IR -> NIR", || {
        nir::lower_module(&merged, target_name, build_mode, stdin_log_cap, helpers)
    })?;
    crate::trace::count("NIR functions", module.functions.len() as u64);
    crate::trace::count(
        "NIR statements",
        module
            .functions
            .iter()
            .map(|function| function.body.len() as u64)
            .sum(),
    );
    // plan-100 §3: the Opt1 seam. This is the sole `NirModule` producer, so one
    // wrap here covers all five targets. Occupied by the Level-1 local-rewrite
    // rows — constant folding, algebraic simplification, strength reduction
    // (`optimizer::opt1`).
    Ok(crate::trace::timed("opt1 (NIR)", || {
        crate::optimizer::opt1::optimize_nir(module, crate::optimizer::active_opt_level())
    }))
}
