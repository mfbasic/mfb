pub(crate) mod flavor;
mod link;
mod object;

use crate::arch::aarch64::encode::EncodedImage;
use crate::target::shared::plan::NativePlan;
use flavor::LinuxFlavor;
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) fn write_native_object_plan(
    project_dir: &Path,
    project_name: &str,
    plan: &NativePlan,
) -> Result<PathBuf, String> {
    let object_plan = object::lower_plan(plan)?;
    object_plan.validate()?;
    let object_path = project_dir.join(format!("{project_name}.nobj"));
    fs::write(&object_path, object_plan.to_json())
        .map_err(|err| format!("failed to write '{}': {err}", object_path.display()))?;
    Ok(object_path)
}

pub(crate) fn validate_native_object_plan(plan: &NativePlan) -> Result<(), String> {
    object::lower_plan(plan)?.validate()
}

pub(crate) fn write_linked_executable(
    project_dir: &Path,
    project_name: &str,
    arch: &str,
    flavor: LinuxFlavor,
    app_mode: bool,
    image: &EncodedImage,
) -> Result<PathBuf, String> {
    link::write_executable(project_dir, project_name, arch, flavor, app_mode, image)
}
