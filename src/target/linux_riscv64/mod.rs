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
            arch: "riscv64".to_string(),
        }
    }

    fn capabilities(&self) -> BackendCapabilities {
        // plan-99 complete: the riscv64 backend advertises the full console
        // runtime-call surface — identical to linux-aarch64 (io/fs/net/term/
        // datetime/thread/tls) — all served by the shared helpers through the
        // MIR seam and the x86 remap.
        BackendCapabilities {
            executable: true,
            native_ir: true,
            native_plan: true,
            native_object_plan: true,
            native_code_plan: true,
            // The runtime-helper OS methods are wired via libc (mirroring
            // AArch64), so the same call surface is supported — including
            // thread.* (the shared pthread trampoline; alias-free x13/x14/x20
            // scratch) and tls.* (the shared OpenSSL dlopen backend).
            runtime_calls: &[
                "crypto.randomBytes",
                "crypto.generateP256Raw",
                "crypto.generateP384Raw",
                "crypto.generateP521Raw",
                "crypto.p256Sign",
                "crypto.p384Sign",
                "crypto.p521Sign",
                "crypto.p256Verify",
                "crypto.p384Verify",
                "crypto.p521Verify",
                "datetime.nowNanos",
                "datetime.monotonicNanos",
                "datetime.localOffset",
                "os.getEnv",
                "os.getEnvOr",
                "os.hasEnv",
                "os.setEnv",
                "os.unsetEnv",
                "os.environ",
                "os.args",
                "os.pid",
                "os.executablePath",
                "os.resourcePath",
                "os.name",
                "os.arch",
                "os.hostName",
                "os.userName",
                "os.cpuCount",
                "io.print",
                "io.write",
                "io.printError",
                "io.writeError",
                "io.flush",
                "io.isBuffered",
                "io.setBuffered",
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
                "term.sync",
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
                "fs.openWithin",
                "fs.createTempFile",
                "fs.close",
                "fs.writeAll",
                "fs.setBuffered",
                "fs.isBuffered",
                "fs.flush",
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
                "tls.listen",
                "tls.accept",
                "tls.read",
                "tls.readText",
                "tls.write",
                "tls.writeText",
                "tls.close",
                "tls.closeListener",
                "audio.devices",
                "audio.openInput",
                "audio.openInputDevice",
                "audio.openOutput",
                "audio.openOutputDevice",
                "audio.read",
                "audio.readTimeout",
                "audio.write",
                "audio.poll",
                "audio.pollTimeout",
                "audio.available",
                "audio.xruns",
                "audio.closeInput",
                "audio.closeOutput",
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
                "thread.openStdIn",
                "thread.closeStdIn",
            ],
        }
    }

    fn validate(&self, ir: &IrProject, packages: &[PathBuf]) -> Result<(), String> {
        validate::validate_project(ir, packages)
    }

    fn supports_app_mode(&self) -> bool {
        // Console only for now (plan-99): the GTK4 app-mode toolkit
        // (`target::linux_gtk`) has not been ported to rv64, so `-app` is
        // rejected at the CLI for this target.
        false
    }

    fn write_executable(
        &self,
        project_dir: &Path,
        ir: &IrProject,
        packages: &[PathBuf],
        signing_metadata: Option<&[u8]>,
        build_mode: NativeBuildMode,
        app_icon: Option<&Path>,
        app_version: Option<&str>,
        vendors_native_libraries: bool,
        stdin_log_cap: Option<u64>,
    ) -> Result<Vec<PathBuf>, String> {
        // App icons are macOS-only (plan-22); the Linux/GTK backend ignores it.
        let _ = app_icon;
        // Bundle version keys are macOS-only (bug-248); Linux has no bundle.
        let _ = app_version;
        write_executable(
            project_dir,
            ir,
            &self.target(),
            packages,
            signing_metadata,
            build_mode,
            vendors_native_libraries,
            stdin_log_cap,
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

#[allow(clippy::too_many_arguments)]
fn write_executable(
    project_dir: &Path,
    ir: &IrProject,
    target: &BuildTarget,
    packages: &[PathBuf],
    signing_metadata: Option<&[u8]>,
    build_mode: NativeBuildMode,
    vendors_native_libraries: bool,
    stdin_log_cap: Option<u64>,
) -> Result<Vec<PathBuf>, String> {
    let module = lower_validated_module(ir, target, packages, build_mode, stdin_log_cap)?;
    // The console build emits one executable per libc world — `<name>-glibc.out`
    // (libc.so.6, /lib64/ld-linux-riscv64.so.2) and `<name>-musl.out`
    // (libc.musl-riscv64.so.1, /lib/ld-musl-riscv64.so.1) — exactly like
    // linux-aarch64. The whole lowering is flavor-parameterized (the plan's
    // `Platform::libc()` names the library each import binds to); on riscv64 the
    // two worlds share every kernel struct layout the codegen bakes in
    // (stat/dirent/termios, pthread object sizes), so only the import library
    // names and the interpreter differ.
    //
    // App mode never reaches here: `supports_app_mode()` is false (bug-117.1 —
    // the GTK entry was never ported), and plan-51-A §3.3 records why that is now
    // permanent rather than pending, since AppImage/type2-runtime publishes no
    // riscv64 runtime to seal an AppDir with. Reject rather than quietly emitting
    // a console-shaped binary for an app build.
    if build_mode.is_app() {
        return Err("linux-riscv64 does not support app mode".to_string());
    }
    let flavors: &[LinuxFlavor] = &LinuxFlavor::ALL;
    let mut paths = Vec::new();
    for &flavor in flavors {
        let native_plan = plan_lower(&module, flavor)?;
        native_plan.validate()?;
        os::linux::validate_native_object_plan(&native_plan)?;
        let native_code = code::lower_module(&module, &native_plan, packages, flavor)?;
        native_code.validate()?;
        let mut image = arch::riscv64::encode::encode(&native_code)?;
        image.signing_metadata = signing_metadata.map(|metadata| metadata.to_vec());
        // plan-46-D §4.2: point the loader at the `vendor/` directory beside the
        // executable, so a bare-filename `dlopen` of a vendored library resolves
        // and keeps resolving after the whole `build/` directory is moved. Emitted
        // only when the build vendors something, so every other binary stays
        // byte-identical.
        if vendors_native_libraries {
            image.rpaths = vec![crate::os::ELF_VENDOR_RPATH.to_string()];
        }
        paths.push(os::linux::write_linked_executable(
            project_dir,
            &ir.name,
            "riscv64",
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
    build_mode: NativeBuildMode,
) -> Result<PathBuf, String> {
    let module = lower_validated_module(ir, target, packages, build_mode, None)?;
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
    let module = lower_validated_module(ir, target, packages, build_mode, None)?;
    // The single-flavor diagnostic dumps (.nplan/.nobj/.ncode/.mir) use the glibc
    // flavor, matching linux-aarch64, so a cross-target dump diff stays consistent.
    // `write_executable` still builds both flavors for the shipped binaries.
    let native_plan = plan_lower(&module, LinuxFlavor::Glibc)?;
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
    let module = lower_validated_module(ir, target, packages, build_mode, None)?;
    let native_plan = plan_lower(&module, LinuxFlavor::Glibc)?;
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
    let module = lower_validated_module(ir, target, packages, build_mode, None)?;
    let native_plan = plan_lower(&module, LinuxFlavor::Glibc)?;
    native_plan.validate()?;
    os::linux::validate_native_object_plan(&native_plan)?;
    let native_code = code::lower_module(&module, &native_plan, packages, LinuxFlavor::Glibc)?;
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
    let module = lower_validated_module(ir, target, packages, build_mode, None)?;
    let native_plan = plan_lower(&module, LinuxFlavor::Glibc)?;
    native_plan.validate()?;
    os::linux::validate_native_object_plan(&native_plan)?;
    let mir = code::lower_module_mir(&module, &native_plan, packages, LinuxFlavor::Glibc)?;
    let mir_path = project_dir.join(format!("{}.mir", ir.name));
    fs::write(&mir_path, mir.to_json())
        .map_err(|err| format!("failed to write '{}': {err}", mir_path.display()))?;
    Ok(mir_path)
}

/// The Linux native plan is ISA-independent (it is the object plan), so the
/// riscv64 backend reuses the AArch64 backend's `plan` lowering verbatim.
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
    stdin_log_cap: Option<u64>,
) -> Result<crate::target::shared::nir::NirModule, String> {
    validate::validate_target(target)?;
    validate::validate_project(ir, packages)?;
    if !matches!(build_mode, NativeBuildMode::Console) {
        // Console output only: riscv64 `supports_app_mode()` is false, so admitting
        // `LinuxApp` here would reach `code.rs`'s `unimplemented!("rv64 app mode not
        // ported")` and abort the process instead of returning a clean error
        // (bug-223). The CLI rejects `-app` for this target first, but a
        // non-CLI/API caller could construct a `LinuxApp` module directly.
        return Err(format!(
            "Linux riscv64 native targets do not support the {} build mode",
            build_mode.as_str()
        ));
    }
    let module = lower::lower_project(ir, target.name(), packages, build_mode, stdin_log_cap)?;
    validate::validate_nir(&module)?;
    validate::validate_capabilities(&module, &BACKEND.capabilities())?;
    Ok(module)
}
