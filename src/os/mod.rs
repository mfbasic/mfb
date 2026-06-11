pub mod macos;

use crate::ir::IrProject;
use crate::target::BuildTarget;
use std::path::{Path, PathBuf};

pub fn write_executable(
    project_dir: &Path,
    ir: &IrProject,
    target: &BuildTarget,
) -> Result<PathBuf, String> {
    match target.os.as_str() {
        "macos" => macos::write_executable(project_dir, ir, target),
        os => Err(format!(
            "native executable output does not support {os} yet"
        )),
    }
}
