pub mod aarch64;

use crate::ir::IrProject;
use crate::target::BuildTarget;
use std::path::{Path, PathBuf};

pub fn write_binary_dump(
    project_dir: &Path,
    ir: &IrProject,
    target: &BuildTarget,
    packages: &[PathBuf],
) -> Result<PathBuf, String> {
    match target.arch.as_str() {
        "aarch64" => aarch64::write_binary_dump(project_dir, ir, target, packages),
        arch => Err(format!("binary output does not support {arch} yet")),
    }
}
