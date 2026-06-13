use crate::ir::IrProject;
use std::path::PathBuf;

use super::nir::{self, NirModule};
use super::runtime;

pub fn lower_project(
    ir: &IrProject,
    target_name: String,
    packages: &[PathBuf],
) -> Result<NirModule, String> {
    let helpers = runtime::required_helpers(ir);
    nir::lower_module(ir, target_name, helpers, packages)
}
