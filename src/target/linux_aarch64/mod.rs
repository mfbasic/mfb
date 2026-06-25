use crate::arch;
use crate::ir::IrProject;
use crate::os;
use crate::os::linux::flavor::LinuxFlavor;
use crate::target::shared::{lower, validate};
use crate::target::{BackendCapabilities, BuildTarget, NativeBackend, NativeBuildMode};
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) mod code;
pub(crate) mod gtk;
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
                "datetime.nowNanos",
                "datetime.monotonicNanos",
                "datetime.localOffset",
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
                "term.on",
                "term.off",
                "term.isOn",
                "term.setForeground",
                "term.setBackground",
                "term.setBold",
                "term.setUnderline",
                "term.showCursor",
                "term.hideCursor",
                "term.clear",
                "term.moveTo",
                "term.getForeground",
                "term.getBackground",
                "term.getBold",
                "term.getUnderline",
                "term.terminalSize",
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
                "thread.transferResource",
                "thread.acceptResource",
                "thread.isCancelled",
                "net.lookup",
                "net.connectTcp",
                "net.listenTcp",
                "net.accept",
                "net.poll",
                "net.read",
                "net.readText",
                "net.write",
                "net.writeText",
                "net.close",
                "net.localAddress",
                "net.remoteAddress",
                "net.setReadTimeout",
                "net.setWriteTimeout",
                "net.bindUdp",
                "net.receiveFrom",
                "net.receiveTextFrom",
                "net.sendTo",
                "net.sendTextTo",
                "tls.connect",
                "tls.read",
                "tls.readText",
                "tls.write",
                "tls.writeText",
                "tls.close",
            ],
        }
    }

    fn validate(&self, ir: &IrProject, packages: &[PathBuf]) -> Result<(), String> {
        validate::validate_project(ir, packages)
    }

    fn supports_app_mode(&self) -> bool {
        true
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
    // App mode (plan-05-linux-app.md §1.1, §5.2) is glibc-only: GTK is a
    // glibc-world dependency, so app mode emits a single `<name>.out` instead of
    // the console build's `-glibc.out` + `-musl.out` pair.
    let app_mode = build_mode.is_app();
    let flavors: &[LinuxFlavor] = if app_mode {
        &[LinuxFlavor::Glibc]
    } else {
        &LinuxFlavor::ALL
    };
    let mut paths = Vec::new();
    for &flavor in flavors {
        let native_plan = plan::lower_module(&module, flavor)?;
        native_plan.validate()?;
        os::linux::validate_native_object_plan(&native_plan)?;
        let native_code = code::lower_module(&module, &native_plan, packages, flavor)?;
        native_code.validate()?;
        let mut image = arch::aarch64::encode::encode(&native_code)?;
        image.signing_metadata = signing_metadata.map(|metadata| metadata.to_vec());
        paths.push(os::linux::write_linked_executable(
            project_dir,
            &ir.name,
            flavor,
            app_mode,
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
    build_mode: NativeBuildMode,
) -> Result<PathBuf, String> {
    let module = lower_validated_module(ir, target, packages, build_mode)?;
    let native_plan = plan::lower_module(&module, LinuxFlavor::Glibc)?;
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
    build_mode: NativeBuildMode,
) -> Result<crate::target::shared::nir::NirModule, String> {
    validate::validate_target(target)?;
    validate::validate_project(ir, packages)?;
    if !matches!(
        build_mode,
        NativeBuildMode::Console | NativeBuildMode::LinuxApp
    ) {
        // The Linux backend only produces console or GTK4 app-mode output; the CLI
        // selects the build mode from the target OS, so `MacApp` never reaches here.
        return Err(format!(
            "Linux native targets do not support the {} build mode",
            build_mode.as_str()
        ));
    }
    let module = lower::lower_project(ir, target.name(), packages, build_mode)?;
    validate::validate_nir(&module)?;
    validate::validate_capabilities(&module, &BACKEND.capabilities())?;
    Ok(module)
}
