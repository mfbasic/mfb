use crate::ir::IrProject;
use std::path::PathBuf;

use super::nir::{self, NirModule};
use super::runtime;

pub fn lower_project(
    ir: &IrProject,
    target_name: String,
    packages: &[PathBuf],
) -> Result<NirModule, String> {
    // Merge imported packages' Binary IR into the project up front so runtime
    // helper detection and codegen both see the complete, unified function set.
    let merged = nir::merge_packages(ir, packages)?;
    let helpers = runtime::required_helpers(&merged);
    nir::lower_module(&merged, target_name, helpers)
}
