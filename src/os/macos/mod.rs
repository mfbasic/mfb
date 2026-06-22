mod link;
mod object;

use crate::arch::aarch64::encode::EncodedImage;
use crate::target::shared::plan::NativePlan;
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) fn write_native_object_plan(
    project_dir: &Path,
    project_name: &str,
    native_plan: &NativePlan,
) -> Result<PathBuf, String> {
    let object_plan = object::lower_plan(native_plan)?;
    object_plan.validate()?;
    let object_path = project_dir.join(format!("{project_name}.nobj"));
    fs::write(&object_path, object_plan.to_json())
        .map_err(|err| format!("failed to write '{}': {err}", object_path.display()))?;
    Ok(object_path)
}

pub(crate) fn validate_native_object_plan(native_plan: &NativePlan) -> Result<(), String> {
    let object_plan = object::lower_plan(native_plan)?;
    object_plan.validate()
}

pub(crate) fn write_linked_executable(
    project_dir: &Path,
    project_name: &str,
    image: &EncodedImage,
) -> Result<PathBuf, String> {
    link::write_executable(project_dir, project_name, image)
}

/// Link `image` and write it as a macOS app-mode `.app` bundle (Info.plist +
/// `Contents/MacOS/<name>`), returning the path to the `.app` directory.
///
/// Wired into the macOS backend's app-mode executable path in Phase 3 once the
/// app runtime bootstrap lands; until then the executable path reports a blocker
/// and this writer is exercised only by its end-to-end tests.
#[allow(dead_code)]
pub(crate) fn write_linked_app_bundle(
    project_dir: &Path,
    project_name: &str,
    image: &EncodedImage,
) -> Result<PathBuf, String> {
    link::write_app_bundle(project_dir, project_name, image)
}
