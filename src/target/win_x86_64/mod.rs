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

pub(crate) mod app;
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
    // plan-66 A/B/C/D advertising, dropped by the same stale main→P-66 merge that
    // dropped the Phase-E fs block (the code.rs/shared impls survived, so these
    // box-proven calls were being rejected at validate_capabilities); restored.
    // Phase A — datetime.
    "datetime.monotonicNanos",
    "datetime.nowNanos",
    "datetime.localOffset",
    // Phase B — os.
    "os.name",
    "os.arch",
    "os.pid",
    "os.sleep",
    "os.cpuCount",
    "os.getEnv",
    "os.getEnvOr",
    "os.hasEnv",
    "os.setEnv",
    "os.unsetEnv",
    "os.environ",
    "os.args",
    "os.hostName",
    "os.userName",
    "os.executablePath",
    "os.version",
    "os.uptime",
    "os.isAdmin",
    // Phase C — io console input + buffering.
    "io.input",
    "io.readLine",
    "io.readChar",
    "io.readByte",
    "io.pollInput",
    "io.flush",
    "io.isBuffered",
    "io.setBuffered",
    // Phase D — term styling / TUI.
    "term.on",
    "term.off",
    "term.isOn",
    "term.setForeground",
    "term.setBackground",
    "term.setBold",
    "term.setUnderline",
    "term.getForeground",
    "term.getBackground",
    "term.getBold",
    "term.getUnderline",
    "term.showCursor",
    "term.hideCursor",
    "term.clear",
    "term.moveTo",
    "term.sync",
    // The draw helpers lower through the shared console cell-grid path (they only
    // mutate the back buffer; `term.sync` presents it), so console builds get them
    // for free. In app/GUI mode the GDI grid has no cell buffer, so the app backend
    // raises `ErrUnsupported` instead (see `app::emit_app_term_helper`).
    "term.drawHLine",
    "term.drawVLine",
    "term.drawBox",
    "term.fillRect",
    "term.drawText",
    "term.drawGlyph",
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
    "fs.canonicalPath",
    // File-resource surface.
    "fs.openFile",
    "fs.close",
    "fs.readAll",
    "fs.readAllBytes",
    "fs.readLine",
    "fs.eof",
    "fs.writeAll",
    "fs.writeAllBytes",
    "fs.flush",
    // plan-66-E fs extras. NOTE: this block was dropped by a stale-merge conflict
    // resolution into the P-66 integration branch (the advertising vanished while
    // the plan.rs/code.rs/shared impls survived); restored here. `openFileNoFollow`
    // and `openWithin` are advertised in the no-symlink block below (they need the
    // GetFinalPathNameByHandleW verify).
    "fs.open",
    "fs.createDirectories",
    "fs.createTempFile",
    "fs.writeTextAtomic",
    "fs.writeBytesAtomic",
    "fs.setBuffered",
    "fs.isBuffered",
    "fs.isWithin",
    // plan-66-E whole-path no-symlink surface. CreateFileW follows reparse points,
    // so both enforce the no-symlink / containment guarantee AFTER the open via a
    // GetFinalPathNameByHandleW verify (emit_verify_nofollow / emit_verify_within).
    "fs.openFileNoFollow",
    "fs.openWithin",
    // Terminal queries (47-G).
    "io.isInputTerminal",
    "io.isOutputTerminal",
    "io.isErrorTerminal",
    "term.terminalSize",
    "term.didResize",
    "thread.start",
    "thread.waitFor",
    "thread.send",
    "thread.receive",
    "thread.poll",
    "thread.isRunning",
    "thread.drop",
    "thread.cancel",
    "thread.isCancelled",
    "thread.emit",
    "thread.read",
    "thread.openStdIn",
    "thread.closeStdIn",
    "thread.transferResource",
    "thread.acceptResource",
    "thread.emitResource",
    "thread.readResource",
    // Networking (plan-47-I): the full net:: surface over Winsock2.
    "net.lookup",
    "net.connectTcp",
    "net.connectTcpAddr",
    "net.listenTcp",
    "net.accept",
    "net.bindUdp",
    "net.close",
    "net.read",
    "net.readText",
    "net.write",
    "net.writeText",
    "net.sendTo",
    "net.sendTextTo",
    "net.receiveFrom",
    "net.receiveTextFrom",
    "net.poll",
    "net.localAddress",
    "net.remoteAddress",
    "net.setReadTimeout",
    "net.setWriteTimeout",
    // Crypto (plan-47-J): randomBytes over BCryptGenRandom, NIST-EC over CNG.
    "crypto.randomBytes",
    "crypto.generate",
    "crypto.sign",
    "crypto.verify",
    "crypto.hash",
    "crypto.seal",
    "crypto.open",
    // TLS client + server over Schannel (plan-47-J).
    "tls.connect",
    "tls.read",
    "tls.readText",
    "tls.write",
    "tls.writeText",
    "tls.poll",
    "tls.pollList",
    "tls.close",
    "tls.listen",
    "tls.accept",
    "tls.closeListener",
    // WASAPI audio (plan-66 G+H): the full audio:: surface over COM.
    "audio.devices",
    "audio.openOutput",
    "audio.openOutputDevice",
    "audio.openInput",
    "audio.openInputDevice",
    "audio.write",
    "audio.read",
    "audio.readTimeout",
    "audio.poll",
    "audio.pollTimeout",
    "audio.available",
    "audio.xruns",
    "audio.closeInput",
    "audio.closeOutput",
    // plan-90-D: the Windows process surface — lifecycle (CreateProcessA + 3 pipes),
    // I/O (WriteFile/ReadFile/PeekNamedPipe), signals & detach (TerminateProcess/
    // CloseHandle). shell/spawnEnv remain Unix-only for now.
    "process.spawn",
    "process.pid",
    "process.isRunning",
    "process.waitFor",
    "process.close",
    "process.send",
    "process.sendTimeout",
    "process.sendBytes",
    "process.sendBytesTimeout",
    "process.receive",
    "process.receiveFrom",
    "process.receiveBytes",
    "process.receiveBytesFrom",
    "process.poll",
    "process.pollFrom",
    "process.signal",
    "process.didSignal",
    "process.detach",
    "process.__drop",
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
        // plan-66-I/J: the Win32 transcript window (GUI-subsystem PE hosting the
        // program's console I/O).
        true
    }

    #[allow(clippy::too_many_arguments)]
    fn write_executable(
        &self,
        project_dir: &Path,
        ir: &IrProject,
        packages: &[PathBuf],
        signing_metadata: Option<&[u8]>,
        build_mode: NativeBuildMode,
        app_icon: Option<&Path>,
        app_version: Option<&str>,
        _vendors_native_libraries: bool,
        stdin_log_cap: Option<u64>,
        progress: &dyn Fn(&str),
    ) -> Result<Vec<PathBuf>, String> {
        progress("lowering module");
        let module =
            lower_validated_module(ir, &self.target(), packages, build_mode, stdin_log_cap)?;
        progress("planning + regalloc");
        let native_plan = plan::lower_module(&module)?;
        native_plan.validate()?;
        os::windows::validate_native_object_plan(&native_plan)?;
        progress("emitting native code");
        let native_code = code::lower_module(&module, &native_plan, packages)?;
        native_code.validate()?;
        progress("encoding image");
        let mut image = crate::arch::x86_64::encode::encode(&native_code)?;
        image.signing_metadata = signing_metadata.map(|m| m.to_vec());
        progress("linking executable");
        // plan-66-I: app mode emits a GUI-subsystem PE; icon/version are threaded
        // toward the writer for the plan-66-K `.rsrc` resource section.
        let path = os::windows::write_linked_executable(
            project_dir,
            &ir.name,
            &image,
            build_mode.is_app(),
            app_icon,
            app_version,
        )?;
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
    // plan-66-I: Windows supports Console and its own app mode (the Win32
    // transcript window). Any other mode (macOS/Linux toolkits) is a misroute.
    if !matches!(
        build_mode,
        NativeBuildMode::Console | NativeBuildMode::WindowsApp
    ) {
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
