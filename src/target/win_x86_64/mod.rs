//! The `windows-x86_64` native backend (plan-47-B Phase 2).
//!
//! Registered so `BuildTarget::parse("windows-x86_64")` resolves through
//! `backend_for` instead of erroring "native output does not support …", but
//! deliberately **non-executable**: every capability is `false`, so each dispatch
//! entry point rejects the target at its capability gate with a clear message
//! before any lowering runs. 47-C wires the PE writer and 47-D the Win32 runtime
//! floor; the codegen machinery (the Win64 ABI + `Win64Backend`, plan-47-B A1)
//! is already in place and is activated by the `CodegenPlatform` A2 lands.

use crate::ir::IrProject;
use crate::target::shared::validate;
use crate::target::{BackendCapabilities, BuildTarget, NativeBackend, NativeBuildMode};
use std::path::{Path, PathBuf};

pub(crate) static BACKEND: Backend = Backend;

pub(crate) struct Backend;

/// The message every unsupported dispatch reports. The capability gates in
/// `target::{write_executable,write_nir,…}` fire first with their own per-artifact
/// message, so these method bodies are unreachable in practice; they name the
/// owning sub-plans for anyone who reaches them directly.
fn not_yet() -> String {
    "windows-x86_64 native output is not yet supported (plan-47-C wires the PE \
     writer; plan-47-D the Win32 runtime floor)"
        .to_string()
}

impl NativeBackend for Backend {
    fn target(&self) -> BuildTarget {
        BuildTarget {
            os: "windows".to_string(),
            arch: "x86_64".to_string(),
        }
    }

    fn capabilities(&self) -> BackendCapabilities {
        // All false: the target is resolvable but produces no artifacts yet.
        BackendCapabilities {
            executable: false,
            native_ir: false,
            native_plan: false,
            native_object_plan: false,
            native_code_plan: false,
            runtime_calls: &[],
        }
    }

    fn validate(&self, ir: &IrProject, packages: &[PathBuf]) -> Result<(), String> {
        // Target-neutral project/IR validation (identical to every other backend),
        // so a *valid* project reaches the capability gate and fails there with the
        // expected "…does not support windows-x86_64 yet" message rather than a
        // spurious validation error.
        validate::validate_project(ir, packages)
    }

    #[allow(clippy::too_many_arguments)]
    fn write_executable(
        &self,
        _project_dir: &Path,
        _ir: &IrProject,
        _packages: &[PathBuf],
        _signing_metadata: Option<&[u8]>,
        _build_mode: NativeBuildMode,
        _app_icon: Option<&Path>,
        _app_version: Option<&str>,
        _vendors_native_libraries: bool,
        _stdin_log_cap: Option<u64>,
    ) -> Result<Vec<PathBuf>, String> {
        Err(not_yet())
    }

    fn write_nir(
        &self,
        _project_dir: &Path,
        _ir: &IrProject,
        _packages: &[PathBuf],
        _build_mode: NativeBuildMode,
    ) -> Result<PathBuf, String> {
        Err(not_yet())
    }

    fn write_native_plan(
        &self,
        _project_dir: &Path,
        _ir: &IrProject,
        _packages: &[PathBuf],
        _build_mode: NativeBuildMode,
    ) -> Result<PathBuf, String> {
        Err(not_yet())
    }

    fn write_native_object_plan(
        &self,
        _project_dir: &Path,
        _ir: &IrProject,
        _packages: &[PathBuf],
        _build_mode: NativeBuildMode,
    ) -> Result<PathBuf, String> {
        Err(not_yet())
    }

    fn write_native_code_plan(
        &self,
        _project_dir: &Path,
        _ir: &IrProject,
        _packages: &[PathBuf],
        _build_mode: NativeBuildMode,
    ) -> Result<PathBuf, String> {
        Err(not_yet())
    }

    fn write_mir(
        &self,
        _project_dir: &Path,
        _ir: &IrProject,
        _packages: &[PathBuf],
        _build_mode: NativeBuildMode,
    ) -> Result<PathBuf, String> {
        Err(not_yet())
    }

    fn supports_app_mode(&self) -> bool {
        // Console subsystem only — no Windows GUI/app mode (master §Non-goals).
        false
    }
}
