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
    let merged = nir::merge_packages(ir, packages)?;
    let helpers = runtime::required_helpers(&merged);
    let module = nir::lower_module(&merged, target_name, build_mode, stdin_log_cap, helpers)?;
    // plan-100 §3: the Opt1 seam. This is the sole `NirModule` producer, so one
    // wrap here covers all five targets. Identity today -- no catalog row
    // occupies Opt1 yet.
    Ok(crate::optimizer::opt1::optimize_nir(
        module,
        crate::optimizer::active_opt_level(),
    ))
}
