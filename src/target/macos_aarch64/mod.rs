use crate::arch;
use crate::ir::IrProject;
use crate::os;
use crate::target::shared::{lower, validate};
use crate::target::{BackendCapabilities, BuildTarget, NativeBackend, NativeBuildMode};
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) mod app;
pub(crate) mod code;
pub(crate) mod plan;

pub(crate) static BACKEND: Backend = Backend;

pub(crate) struct Backend;

impl NativeBackend for Backend {
    fn target(&self) -> BuildTarget {
        BuildTarget {
            os: "macos".to_string(),
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

    fn supports_app_mode(&self) -> bool {
        true
    }

    fn validate(&self, ir: &IrProject, packages: &[PathBuf]) -> Result<(), String> {
        validate::validate_project(ir, packages)
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
    validate::validate_target(target)?;
    validate::validate_project(ir, packages)?;
    let module = lower::lower_project(ir, target.name(), packages, build_mode)?;
    validate::validate_nir(&module)?;
    validate::validate_capabilities(&module, &BACKEND.capabilities())?;
    let native_plan = plan::lower_module(&module)?;
    native_plan.validate()?;
    os::macos::validate_native_object_plan(&native_plan)?;
    let native_code = code::lower_module(&module, &native_plan, packages)?;
    native_code.validate()?;
    let mut image = arch::aarch64::encode::encode(&native_code)?;
    image.signing_metadata = signing_metadata.map(|metadata| metadata.to_vec());
    match build_mode {
        // App mode (plan-04-macos-app.md §5.2) emits a `.app` bundle whose AppKit
        // `_main` bootstrap targets a window; console mode emits a plain `.out`.
        NativeBuildMode::MacApp => {
            os::macos::write_linked_app_bundle(project_dir, &ir.name, &image).map(|path| vec![path])
        }
        NativeBuildMode::Console => {
            os::macos::write_linked_executable(project_dir, &ir.name, &image).map(|path| vec![path])
        }
        // `LinuxApp` is a Linux toolkit selection; it never reaches the macOS
        // backend (the CLI picks the build mode from the target OS).
        NativeBuildMode::LinuxApp => {
            Err("internal error: macOS backend received a Linux app build mode".to_string())
        }
    }
}

fn write_nir(
    project_dir: &Path,
    ir: &IrProject,
    target: &BuildTarget,
    packages: &[PathBuf],
    build_mode: NativeBuildMode,
) -> Result<PathBuf, String> {
    validate::validate_target(target)?;
    validate::validate_project(ir, packages)?;
    let module = lower::lower_project(ir, target.name(), packages, build_mode)?;
    validate::validate_nir(&module)?;
    validate::validate_capabilities(&module, &BACKEND.capabilities())?;
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
    validate::validate_target(target)?;
    validate::validate_project(ir, packages)?;
    let module = lower::lower_project(ir, target.name(), packages, build_mode)?;
    validate::validate_nir(&module)?;
    validate::validate_capabilities(&module, &BACKEND.capabilities())?;
    let native_plan = plan::lower_module(&module)?;
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
    validate::validate_target(target)?;
    validate::validate_project(ir, packages)?;
    let module = lower::lower_project(ir, target.name(), packages, build_mode)?;
    validate::validate_nir(&module)?;
    validate::validate_capabilities(&module, &BACKEND.capabilities())?;
    let native_plan = plan::lower_module(&module)?;
    native_plan.validate()?;
    os::macos::write_native_object_plan(project_dir, &ir.name, &native_plan)
}

fn write_native_code_plan(
    project_dir: &Path,
    ir: &IrProject,
    target: &BuildTarget,
    packages: &[PathBuf],
    build_mode: NativeBuildMode,
) -> Result<PathBuf, String> {
    validate::validate_target(target)?;
    validate::validate_project(ir, packages)?;
    let module = lower::lower_project(ir, target.name(), packages, build_mode)?;
    validate::validate_nir(&module)?;
    validate::validate_capabilities(&module, &BACKEND.capabilities())?;
    let native_plan = plan::lower_module(&module)?;
    native_plan.validate()?;
    os::macos::validate_native_object_plan(&native_plan)?;
    let native_code = code::lower_module(&module, &native_plan, packages)?;
    native_code.validate()?;
    let code_path = project_dir.join(format!("{}.ncode", ir.name));
    fs::write(&code_path, native_code.to_json())
        .map_err(|err| format!("failed to write '{}': {err}", code_path.display()))?;
    Ok(code_path)
}
