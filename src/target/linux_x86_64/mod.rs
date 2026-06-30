use crate::arch;
use crate::ir::IrProject;
use crate::os;
use crate::os::linux::flavor::LinuxFlavor;
use crate::target::shared::{lower, validate};
use crate::target::{BackendCapabilities, BuildTarget, NativeBackend, NativeBuildMode};
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) mod code;
pub(crate) mod plan;

pub(crate) static BACKEND: Backend = Backend;

pub(crate) struct Backend;

impl NativeBackend for Backend {
    fn target(&self) -> BuildTarget {
        BuildTarget {
            os: "linux".to_string(),
            arch: "x86_64".to_string(),
        }
    }

    fn capabilities(&self) -> BackendCapabilities {
        // Phase 1 (plan-00-H): the x86-64 backend brings up the integer-only
        // entry + arena machine floor. The runtime-helper surface (io/fs/net/...)
        // is not wired yet — those OS methods return a Phase-1 error — so the
        // backend advertises no runtime calls. Programs that only run integer
        // language code go through the entry + arena path alone.
        BackendCapabilities {
            executable: true,
            native_ir: true,
            native_plan: true,
            native_object_plan: true,
            native_code_plan: true,
            // Phase 1 wires the integer core + io OUTPUT (write/print via raw
            // `write` syscalls). io input, fs, net, term, thread, tls remain
            // Phase 2+ (their OS methods still return a Phase-1 error).
            runtime_calls: &[
                "io.print",
                "io.write",
                "io.printError",
                "io.writeError",
            ],
        }
    }

    fn validate(&self, ir: &IrProject, packages: &[PathBuf]) -> Result<(), String> {
        validate::validate_project(ir, packages)
    }

    fn supports_app_mode(&self) -> bool {
        false
    }

    fn write_executable(
        &self,
        project_dir: &Path,
        ir: &IrProject,
        packages: &[PathBuf],
        signing_metadata: Option<&[u8]>,
        build_mode: NativeBuildMode,
    ) -> Result<Vec<PathBuf>, String> {
        write_executable(
            project_dir,
            ir,
            &self.target(),
            packages,
            signing_metadata,
            build_mode,
        )
    }

    fn write_nir(
        &self,
        project_dir: &Path,
        ir: &IrProject,
        packages: &[PathBuf],
        build_mode: NativeBuildMode,
    ) -> Result<PathBuf, String> {
        write_nir(project_dir, ir, &self.target(), packages, build_mode)
    }

    fn write_native_plan(
        &self,
        project_dir: &Path,
        ir: &IrProject,
        packages: &[PathBuf],
        build_mode: NativeBuildMode,
    ) -> Result<PathBuf, String> {
        write_native_plan(project_dir, ir, &self.target(), packages, build_mode)
    }

    fn write_native_object_plan(
        &self,
        project_dir: &Path,
        ir: &IrProject,
        packages: &[PathBuf],
        build_mode: NativeBuildMode,
    ) -> Result<PathBuf, String> {
        write_native_object_plan(project_dir, ir, &self.target(), packages, build_mode)
    }

    fn write_native_code_plan(
        &self,
        project_dir: &Path,
        ir: &IrProject,
        packages: &[PathBuf],
        build_mode: NativeBuildMode,
    ) -> Result<PathBuf, String> {
        write_native_code_plan(project_dir, ir, &self.target(), packages, build_mode)
    }

    fn write_mir(
        &self,
        project_dir: &Path,
        ir: &IrProject,
        packages: &[PathBuf],
        build_mode: NativeBuildMode,
    ) -> Result<PathBuf, String> {
        write_mir(project_dir, ir, &self.target(), packages, build_mode)
    }
}

fn write_executable(
    project_dir: &Path,
    ir: &IrProject,
    target: &BuildTarget,
    packages: &[PathBuf],
    signing_metadata: Option<&[u8]>,
    build_mode: NativeBuildMode,
) -> Result<Vec<PathBuf>, String> {
    let module = lower_validated_module(ir, target, packages, build_mode)?;
    let mut paths = Vec::new();
    // Phase 1: a single static musl-flavored executable. The Linux x86-64 OS
    // methods use raw syscalls (no libc), so only the musl flavor is emitted.
    let flavor = LinuxFlavor::Musl;
    let native_plan = plan_lower(&module, flavor)?;
    native_plan.validate()?;
    os::linux::validate_native_object_plan(&native_plan)?;
    let native_code = code::lower_module(&module, &native_plan, packages, flavor)?;
    native_code.validate()?;
    let mut image = arch::x86_64::encode::encode(&native_code)?;
    image.signing_metadata = signing_metadata.map(|metadata| metadata.to_vec());
    paths.push(os::linux::write_linked_executable(
        project_dir,
        &ir.name,
        flavor,
        false,
        &image,
    )?);
    Ok(paths)
}

fn write_nir(
    project_dir: &Path,
    ir: &IrProject,
    target: &BuildTarget,
    packages: &[PathBuf],
    build_mode: NativeBuildMode,
) -> Result<PathBuf, String> {
    let module = lower_validated_module(ir, target, packages, build_mode)?;
    let nir_path = project_dir.join(format!("{}.nir", ir.name));
    fs::write(&nir_path, module.to_json())
        .map_err(|err| format!("failed to write '{}': {err}", nir_path.display()))?;
    Ok(nir_path)
}

fn write_native_plan(
    project_dir: &Path,
    ir: &IrProject,
    target: &BuildTarget,
    packages: &[PathBuf],
    build_mode: NativeBuildMode,
) -> Result<PathBuf, String> {
    let module = lower_validated_module(ir, target, packages, build_mode)?;
    let native_plan = plan_lower(&module, LinuxFlavor::Musl)?;
    native_plan.validate()?;
    let plan_path = project_dir.join(format!("{}.nplan", ir.name));
    fs::write(&plan_path, native_plan.to_json())
        .map_err(|err| format!("failed to write '{}': {err}", plan_path.display()))?;
    Ok(plan_path)
}

fn write_native_object_plan(
    project_dir: &Path,
    ir: &IrProject,
    target: &BuildTarget,
    packages: &[PathBuf],
    build_mode: NativeBuildMode,
) -> Result<PathBuf, String> {
    let module = lower_validated_module(ir, target, packages, build_mode)?;
    let native_plan = plan_lower(&module, LinuxFlavor::Musl)?;
    native_plan.validate()?;
    os::linux::write_native_object_plan(project_dir, &ir.name, &native_plan)
}

fn write_native_code_plan(
    project_dir: &Path,
    ir: &IrProject,
    target: &BuildTarget,
    packages: &[PathBuf],
    build_mode: NativeBuildMode,
) -> Result<PathBuf, String> {
    let module = lower_validated_module(ir, target, packages, build_mode)?;
    let native_plan = plan_lower(&module, LinuxFlavor::Musl)?;
    native_plan.validate()?;
    os::linux::validate_native_object_plan(&native_plan)?;
    let native_code = code::lower_module(&module, &native_plan, packages, LinuxFlavor::Musl)?;
    native_code.validate()?;
    let code_path = project_dir.join(format!("{}.ncode", ir.name));
    fs::write(&code_path, native_code.to_json())
        .map_err(|err| format!("failed to write '{}': {err}", code_path.display()))?;
    Ok(code_path)
}

fn write_mir(
    project_dir: &Path,
    ir: &IrProject,
    target: &BuildTarget,
    packages: &[PathBuf],
    build_mode: NativeBuildMode,
) -> Result<PathBuf, String> {
    let module = lower_validated_module(ir, target, packages, build_mode)?;
    let native_plan = plan_lower(&module, LinuxFlavor::Musl)?;
    native_plan.validate()?;
    os::linux::validate_native_object_plan(&native_plan)?;
    let mir = code::lower_module_mir(&module, &native_plan, packages, LinuxFlavor::Musl)?;
    let mir_path = project_dir.join(format!("{}.mir", ir.name));
    fs::write(&mir_path, mir.to_json())
        .map_err(|err| format!("failed to write '{}': {err}", mir_path.display()))?;
    Ok(mir_path)
}

/// The Linux native plan is ISA-independent (it is the object plan), so the
/// x86-64 backend reuses the AArch64 backend's `plan` lowering verbatim.
fn plan_lower(
    module: &crate::target::shared::nir::NirModule,
    flavor: LinuxFlavor,
) -> Result<crate::target::shared::plan::NativePlan, String> {
    plan::lower_module(module, flavor)
}

fn lower_validated_module(
    ir: &IrProject,
    target: &BuildTarget,
    packages: &[PathBuf],
    build_mode: NativeBuildMode,
) -> Result<crate::target::shared::nir::NirModule, String> {
    validate::validate_target(target)?;
    validate::validate_project(ir, packages)?;
    if !matches!(build_mode, NativeBuildMode::Console) {
        return Err(format!(
            "Linux x86-64 native targets do not support the {} build mode",
            build_mode.as_str()
        ));
    }
    let module = lower::lower_project(ir, target.name(), packages, build_mode)?;
    validate::validate_nir(&module)?;
    validate::validate_capabilities(&module, &BACKEND.capabilities())?;
    Ok(module)
}
