mod mach_o;

use crate::ir::IrProject;
use std::path::{Path, PathBuf};

pub fn write_executable(project_dir: &Path, ir: &IrProject) -> Result<PathBuf, String> {
    mach_o::write_executable(project_dir, ir)
}
