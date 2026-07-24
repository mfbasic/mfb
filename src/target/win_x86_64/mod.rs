//! The `windows-x86_64` native backend (plan-47-B Phase 2 / plan-47-D).
//!
//! Registered and — as of the 47-D machine floor — **executable** for the
//! console runtime subset it advertises: a program using only integers, strings,
//! collections (and, as later sub-plans land, `io`/fs/…) builds to a PE32+
//! `.exe` that runs on Windows. The Win64 ABI + PE writer are plan-47-B/47-C; this
//! module wires them together and installs the Win32 machine floor
//! (`code.rs`/`plan.rs`).

use crate::ir::IrProject;
use crate::os;
use crate::target::shared::{lower, validate};
use crate::target::{BackendCapabilities, BuildTarget, NativeBackend, NativeBuildMode};
use std::path::{Path, PathBuf};

pub(crate) mod code;
pub(crate) mod plan;

pub(crate) static BACKEND: Backend = Backend;

pub(crate) struct Backend;

/// The runtime-call surface this backend supports. The console-output family
/// (47-D-full) rides `emit_write` (GetStdHandle + WriteFile). Any not-yet-listed
/// `io`/fs/net/… helper is rejected at `validate_capabilities` rather than
/// building a broken `.exe`; each later sub-plan (F, G, …) adds its calls here.
const RUNTIME_CALLS: &[&str] = &[
    "io.print",
    "io.write",
    "io.printError",
    "io.writeError",
    "fs.exists",
    "fs.fileExists",
    "fs.directoryExists",
    // File reads (emit_open_file/read/close/seek over CreateFileW/ReadFile/
    // CloseHandle/SetFilePointerEx, open_flag_set's Windows arm, the GetLastError
    // fs error mapping). Box-verified: readText round-trips a file's exact bytes
    // and interleaves with io::print. (The read/write byte-count out-params are
    // DWORDs — emit_write/emit_read_file must zero the slot before the call.)
    "fs.readText",
    "fs.readBytes",
    "fs.writeText",
    "fs.writeBytes",
    "fs.appendText",
    "fs.appendBytes",
    "fs.deleteFile",
    "fs.createDirectory",
    "fs.setCurrentDirectory",
    "fs.currentDirectory",
    "fs.tempDirectory",
    "fs.deleteDirectory",
    "fs.listDirectory",
];

impl NativeBackend for Backend {
    fn target(&self) -> BuildTarget {
        BuildTarget {
            os: "windows".to_string(),
            arch: "x86_64".to_string(),
        }
    }

    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities {
            executable: true,
            native_ir: true,
            native_plan: true,
            native_object_plan: true,
            native_code_plan: true,
            runtime_calls: RUNTIME_CALLS,
        }
    }

    fn validate(&self, ir: &IrProject, packages: &[PathBuf]) -> Result<(), String> {
        validate::validate_project(ir, packages)
    }

    fn supports_app_mode(&self) -> bool {
        // Console subsystem only — no Windows GUI/app mode (master §Non-goals).
        false
    }

    #[allow(clippy::too_many_arguments)]
    fn write_executable(
        &self,
        project_dir: &Path,
        ir: &IrProject,
        packages: &[PathBuf],
        signing_metadata: Option<&[u8]>,
        build_mode: NativeBuildMode,
        _app_icon: Option<&Path>,
        _app_version: Option<&str>,
        _vendors_native_libraries: bool,
        stdin_log_cap: Option<u64>,
    ) -> Result<Vec<PathBuf>, String> {
        let module = lower_validated_module(ir, &self.target(), packages, build_mode, stdin_log_cap)?;
        let native_plan = plan::lower_module(&module)?;
        native_plan.validate()?;
        os::windows::validate_native_object_plan(&native_plan)?;
        let native_code = code::lower_module(&module, &native_plan, packages)?;
        native_code.validate()?;
        let mut image = crate::arch::x86_64::encode::encode(&native_code)?;
        image.signing_metadata = signing_metadata.map(|m| m.to_vec());
        let path = os::windows::write_linked_executable(project_dir, &ir.name, &image)?;
        Ok(vec![path])
    }

    fn write_nir(
        &self,
        project_dir: &Path,
        ir: &IrProject,
        packages: &[PathBuf],
        build_mode: NativeBuildMode,
    ) -> Result<PathBuf, String> {
        let module = lower_validated_module(ir, &self.target(), packages, build_mode, None)?;
        let path = project_dir.join(format!("{}.nir", ir.name));
        std::fs::write(&path, module.to_json())
            .map_err(|err| format!("failed to write '{}': {err}", path.display()))?;
        Ok(path)
    }

    fn write_native_plan(
        &self,
        project_dir: &Path,
        ir: &IrProject,
        packages: &[PathBuf],
        build_mode: NativeBuildMode,
    ) -> Result<PathBuf, String> {
        let module = lower_validated_module(ir, &self.target(), packages, build_mode, None)?;
        let native_plan = plan::lower_module(&module)?;
        native_plan.validate()?;
        let path = project_dir.join(format!("{}.nplan", ir.name));
        std::fs::write(&path, native_plan.to_json())
            .map_err(|err| format!("failed to write '{}': {err}", path.display()))?;
        Ok(path)
    }

    fn write_native_object_plan(
        &self,
        project_dir: &Path,
        ir: &IrProject,
        packages: &[PathBuf],
        build_mode: NativeBuildMode,
    ) -> Result<PathBuf, String> {
        let module = lower_validated_module(ir, &self.target(), packages, build_mode, None)?;
        let native_plan = plan::lower_module(&module)?;
        os::windows::write_native_object_plan(project_dir, &ir.name, &native_plan)
    }

    fn write_native_code_plan(
        &self,
        project_dir: &Path,
        ir: &IrProject,
        packages: &[PathBuf],
        build_mode: NativeBuildMode,
    ) -> Result<PathBuf, String> {
        let module = lower_validated_module(ir, &self.target(), packages, build_mode, None)?;
        let native_plan = plan::lower_module(&module)?;
        native_plan.validate()?;
        let native_code = code::lower_module(&module, &native_plan, packages)?;
        native_code.validate()?;
        let path = project_dir.join(format!("{}.ncode", ir.name));
        std::fs::write(&path, native_code.to_json())
            .map_err(|err| format!("failed to write '{}': {err}", path.display()))?;
        Ok(path)
    }

    fn write_mir(
        &self,
        project_dir: &Path,
        ir: &IrProject,
        packages: &[PathBuf],
        build_mode: NativeBuildMode,
    ) -> Result<PathBuf, String> {
        let module = lower_validated_module(ir, &self.target(), packages, build_mode, None)?;
        let native_plan = plan::lower_module(&module)?;
        native_plan.validate()?;
        let mir = code::lower_module_mir(&module, &native_plan, packages)?;
        let path = project_dir.join(format!("{}.mir", ir.name));
        std::fs::write(&path, mir.to_json())
            .map_err(|err| format!("failed to write '{}': {err}", path.display()))?;
        Ok(path)
    }
}

fn lower_validated_module(
    ir: &IrProject,
    target: &BuildTarget,
    packages: &[PathBuf],
    build_mode: NativeBuildMode,
    stdin_log_cap: Option<u64>,
) -> Result<crate::target::shared::nir::NirModule, String> {
    validate::validate_target(target)?;
    validate::validate_project(ir, packages)?;
    if !matches!(build_mode, NativeBuildMode::Console) {
        return Err(format!(
            "windows-x86_64 native targets do not support the {} build mode",
            build_mode.as_str()
        ));
    }
    let module = lower::lower_project(ir, target.name(), packages, build_mode, stdin_log_cap)?;
    validate::validate_nir(&module)?;
    validate::validate_capabilities(&module, &BACKEND.capabilities())?;
    Ok(module)
}
