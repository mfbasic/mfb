use crate::arch;
use crate::ir::IrProject;
use crate::os;
use crate::os::linux::flavor::LinuxFlavor;
use crate::target::shared::{lower, validate};
use crate::target::{BackendCapabilities, BuildTarget, NativeBackend};
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
            arch: "aarch64".to_string(),
        }
    }

    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities {
            executable: true,
            native_ir: true,
            native_plan: true,
            native_object_plan: true,
            native_code_plan: true,
            runtime_calls: &[
                "io.print",
                "io.write",
                "io.printError",
                "io.writeError",
                "io.flush",
                "io.flushError",
                "io.input",
                "io.readLine",
                "io.readChar",
                "io.readByte",
                "io.pollInput",
                "io.isInputTerminal",
                "io.isOutputTerminal",
                "io.isErrorTerminal",
                "io.terminalSize",
                "fs.fileExists",
                "fs.directoryExists",
                "fs.exists",
                "fs.currentDirectory",
                "fs.tempDirectory",
                "fs.setCurrentDirectory",
                "fs.deleteFile",
                "fs.createDirectory",
                "fs.createDirectories",
                "fs.deleteDirectory",
                "fs.listDirectory",
                "fs.open",
                "fs.openFile",
                "fs.openFileNoFollow",
                "fs.createTempFile",
                "fs.close",
                "fs.writeAll",
                "fs.writeAllBytes",
                "fs.readText",
                "fs.readBytes",
                "fs.writeText",
                "fs.writeTextAtomic",
                "fs.writeBytes",
                "fs.writeBytesAtomic",
                "fs.appendText",
                "fs.appendBytes",
                "fs.readLine",
                "fs.readAll",
                "fs.readAllBytes",
                "fs.eof",
                "fs.canonicalPath",
                "fs.isWithin",
                "thread.start",
                "thread.isRunning",
                "thread.waitFor",
                "thread.cancel",
                "thread.send",
                "thread.poll",
                "thread.receive",
                "thread.isCancelled",
            ],
        }
    }

    fn validate(&self, ir: &IrProject, packages: &[PathBuf]) -> Result<(), String> {
        validate::validate_project(ir, packages)
    }

    fn write_executable(
        &self,
        project_dir: &Path,
        ir: &IrProject,
        packages: &[PathBuf],
    ) -> Result<Vec<PathBuf>, String> {
        write_executable(project_dir, ir, &self.target(), packages)
    }

    fn write_nir(
        &self,
        project_dir: &Path,
        ir: &IrProject,
        packages: &[PathBuf],
    ) -> Result<PathBuf, String> {
        write_nir(project_dir, ir, &self.target(), packages)
    }

    fn write_native_plan(
        &self,
        project_dir: &Path,
        ir: &IrProject,
        packages: &[PathBuf],
    ) -> Result<PathBuf, String> {
        write_native_plan(project_dir, ir, &self.target(), packages)
    }

    fn write_native_object_plan(
        &self,
        project_dir: &Path,
        ir: &IrProject,
        packages: &[PathBuf],
    ) -> Result<PathBuf, String> {
        write_native_object_plan(project_dir, ir, &self.target(), packages)
    }

    fn write_native_code_plan(
        &self,
        project_dir: &Path,
        ir: &IrProject,
        packages: &[PathBuf],
    ) -> Result<PathBuf, String> {
        write_native_code_plan(project_dir, ir, &self.target(), packages)
    }
}

fn write_executable(
    project_dir: &Path,
    ir: &IrProject,
    target: &BuildTarget,
    packages: &[PathBuf],
) -> Result<Vec<PathBuf>, String> {
    let module = lower_validated_module(ir, target, packages)?;
    let mut paths = Vec::new();
    for flavor in LinuxFlavor::ALL {
        let native_plan = plan::lower_module(&module, flavor)?;
        native_plan.validate()?;
        os::linux::validate_native_object_plan(&native_plan)?;
        let native_code = code::lower_module(&module, &native_plan, packages, flavor)?;
        native_code.validate()?;
        let image = arch::aarch64::encode::encode(&native_code)?;
        paths.push(os::linux::write_linked_executable(
            project_dir,
            &ir.name,
            flavor,
            &image,
        )?);
    }
    Ok(paths)
}

fn write_nir(
    project_dir: &Path,
    ir: &IrProject,
    target: &BuildTarget,
    packages: &[PathBuf],
) -> Result<PathBuf, String> {
    let module = lower_validated_module(ir, target, packages)?;
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
) -> Result<PathBuf, String> {
    let module = lower_validated_module(ir, target, packages)?;
    let native_plan = plan::lower_module(&module, LinuxFlavor::Glibc)?;
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
) -> Result<PathBuf, String> {
    let module = lower_validated_module(ir, target, packages)?;
    let native_plan = plan::lower_module(&module, LinuxFlavor::Glibc)?;
    native_plan.validate()?;
    os::linux::write_native_object_plan(project_dir, &ir.name, &native_plan)
}

fn write_native_code_plan(
    project_dir: &Path,
    ir: &IrProject,
    target: &BuildTarget,
    packages: &[PathBuf],
) -> Result<PathBuf, String> {
    let module = lower_validated_module(ir, target, packages)?;
    let native_plan = plan::lower_module(&module, LinuxFlavor::Glibc)?;
    native_plan.validate()?;
    os::linux::validate_native_object_plan(&native_plan)?;
    let native_code = code::lower_module(&module, &native_plan, packages, LinuxFlavor::Glibc)?;
    native_code.validate()?;
    let code_path = project_dir.join(format!("{}.ncode", ir.name));
    fs::write(&code_path, native_code.to_json())
        .map_err(|err| format!("failed to write '{}': {err}", code_path.display()))?;
    Ok(code_path)
}

fn lower_validated_module(
    ir: &IrProject,
    target: &BuildTarget,
    packages: &[PathBuf],
) -> Result<crate::target::shared::nir::NirModule, String> {
    validate::validate_target(target)?;
    validate::validate_project(ir, packages)?;
    let module = lower::lower_project(ir, target.name(), packages)?;
    validate::validate_nir(&module)?;
    validate::validate_capabilities(&module, &BACKEND.capabilities())?;
    Ok(module)
}
