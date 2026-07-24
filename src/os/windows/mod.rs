//! Windows PE/COFF container writer (plan-47-C) — the third sibling of
//! `src/os/{linux,macos}/`, emitting a PE32+ console `.exe` from the same
//! [`crate::arch::aarch64::encode::EncodedImage`] the ELF and Mach-O writers
//! consume.
//!
//! **Staged landing (plan-47-C C1).** This module and its object plan land
//! before the backend that selects them: `windows-x86_64` is not registered in
//! `NATIVE_BACKENDS` until plan-47-B, and the compiler-driven build is not wired
//! until plan-47-D. Until then the writer is exercised only from its own tests,
//! so its public surface is unreferenced by non-test code — hence the
//! module-scoped `dead_code` allow below. **plan-47-D removes it** when it wires
//! `write_native_object_plan` into the target dispatch and `write_executable`
//! into the linker seam.
#![allow(dead_code)]

mod object;

use crate::target::shared::plan::NativePlan;
use std::fs;
use std::path::{Path, PathBuf};

/// Lower `plan` to a `container:"pe"` object plan and write it as `<name>.nobj`
/// (the `-nobj` artifact). Mirrors `crate::os::linux::write_native_object_plan`.
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

/// Validate that `plan` lowers to a well-formed PE object plan, without writing
/// anything. Mirrors `crate::os::linux::validate_native_object_plan`.
pub(crate) fn validate_native_object_plan(plan: &NativePlan) -> Result<(), String> {
    object::lower_plan(plan)?.validate()
}
