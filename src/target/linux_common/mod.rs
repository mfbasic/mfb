//! Material shared by every Linux native backend (bug-321).
//!
//! `linux_aarch64`, `linux_riscv64`, and `linux_x86_64` were three near-verbatim
//! copies of one backend — ~5,250 lines carrying roughly one backend's worth of
//! information. Any Linux ABI fact, import rule, or runtime-call registration had
//! to be edited in three places, and the copies had already drifted (the x86 copy
//! silently lost every explanatory comment on its socket/errno constants).
//!
//! The three backends remain three distinct [`crate::target::NativeBackend`]
//! registrations with their own targets, encoders, and capabilities — that seam
//! is load-bearing. What lives here is only what is invariant across Linux:
//!
//! - [`RUNTIME_CALLS`] — the runtime-call surface, previously a 150-entry array
//!   tripled verbatim.
//! - [`plan`] — the libc import rules, parameterized by each backend's
//!   raw-syscall policy.
//! - [`code`] — the [`crate::target::shared::code::CodegenPlatform`]
//!   implementation, parameterized by each backend's ISA delta.
//! - The five diagnostic dump writers below.
//!
//! What deliberately stays per-backend: `app_mode_imports`, `supports_app_mode`,
//! `write_executable`, and `lower_validated_module`. The last two carry the
//! riscv64 app-mode rejection (bug-223), which cannot be hoisted because the
//! aarch64 and x86-64 backends legitimately accept `NativeBuildMode::LinuxApp`.

pub(crate) mod code;
pub(crate) mod plan;

use crate::ir::IrProject;
use crate::os;
use crate::os::linux::flavor::LinuxFlavor;
use crate::target::shared::code::{MirPlan, NativeCodePlan};
use crate::target::shared::nir::NirModule;
use crate::target::shared::plan::NativePlan;
use crate::target::{BuildTarget, NativeBuildMode};
use std::fs;
use std::path::{Path, PathBuf};

/// The runtime calls every Linux backend serves.
///
/// All three advertised a set-identical 150-entry list; only the position of the
/// 12-entry `thread.*` block differed, which made a raw `diff` look like a
/// 24-line divergence carrying zero information. The sole consumer is
/// [`crate::target::shared::validate::validate_capabilities`], which does a
/// `contains` membership test — order is not semantically significant, but no
/// entry may be added or dropped without changing what the backends accept.
pub(crate) const RUNTIME_CALLS: &[&str] = &[
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
];

/// A backend's NIR lowering plus its own validation and build-mode policy.
type LowerValidatedModule = fn(
    &IrProject,
    &BuildTarget,
    &[PathBuf],
    NativeBuildMode,
    Option<u64>,
) -> Result<NirModule, String>;

/// The per-backend lowering entry points the diagnostic dump writers call.
///
/// `lower_validated_module` stays per-backend on purpose: it is where each
/// backend enforces its own build-mode policy (riscv64 rejects
/// `NativeBuildMode::LinuxApp` there, bug-223).
pub(crate) struct DumpHooks {
    pub(crate) lower_validated_module: LowerValidatedModule,
    pub(crate) lower_plan: fn(&NirModule, LinuxFlavor) -> Result<NativePlan, String>,
    pub(crate) lower_code:
        fn(&NirModule, &NativePlan, &[PathBuf], LinuxFlavor) -> Result<NativeCodePlan, String>,
    pub(crate) lower_mir:
        fn(&NirModule, &NativePlan, &[PathBuf], LinuxFlavor) -> Result<MirPlan, String>,
}

/// The single-flavor diagnostic dumps (`.nplan`/`.nobj`/`.ncode`/`.mir`) all use
/// the glibc flavor, so a cross-target dump diff stays comparable.
/// `write_executable` still builds both flavors for the shipped binaries.
const DUMP_FLAVOR: LinuxFlavor = LinuxFlavor::Glibc;

pub(crate) fn write_nir(
    hooks: &DumpHooks,
    project_dir: &Path,
    ir: &IrProject,
    target: &BuildTarget,
    packages: &[PathBuf],
    build_mode: NativeBuildMode,
) -> Result<PathBuf, String> {
    let module = (hooks.lower_validated_module)(ir, target, packages, build_mode, None)?;
    let nir_path = project_dir.join(format!("{}.nir", ir.name));
    fs::write(&nir_path, module.to_json())
        .map_err(|err| format!("failed to write '{}': {err}", nir_path.display()))?;
    Ok(nir_path)
}

pub(crate) fn write_native_plan(
    hooks: &DumpHooks,
    project_dir: &Path,
    ir: &IrProject,
    target: &BuildTarget,
    packages: &[PathBuf],
    build_mode: NativeBuildMode,
) -> Result<PathBuf, String> {
    let module = (hooks.lower_validated_module)(ir, target, packages, build_mode, None)?;
    let native_plan = (hooks.lower_plan)(&module, DUMP_FLAVOR)?;
    native_plan.validate()?;
    let plan_path = project_dir.join(format!("{}.nplan", ir.name));
    fs::write(&plan_path, native_plan.to_json())
        .map_err(|err| format!("failed to write '{}': {err}", plan_path.display()))?;
    Ok(plan_path)
}

pub(crate) fn write_native_object_plan(
    hooks: &DumpHooks,
    project_dir: &Path,
    ir: &IrProject,
    target: &BuildTarget,
    packages: &[PathBuf],
    build_mode: NativeBuildMode,
) -> Result<PathBuf, String> {
    let module = (hooks.lower_validated_module)(ir, target, packages, build_mode, None)?;
    let native_plan = (hooks.lower_plan)(&module, DUMP_FLAVOR)?;
    native_plan.validate()?;
    os::linux::write_native_object_plan(project_dir, &ir.name, &native_plan)
}

pub(crate) fn write_native_code_plan(
    hooks: &DumpHooks,
    project_dir: &Path,
    ir: &IrProject,
    target: &BuildTarget,
    packages: &[PathBuf],
    build_mode: NativeBuildMode,
) -> Result<PathBuf, String> {
    let module = (hooks.lower_validated_module)(ir, target, packages, build_mode, None)?;
    let native_plan = (hooks.lower_plan)(&module, DUMP_FLAVOR)?;
    native_plan.validate()?;
    os::linux::validate_native_object_plan(&native_plan)?;
    let native_code = (hooks.lower_code)(&module, &native_plan, packages, DUMP_FLAVOR)?;
    native_code.validate()?;
    let code_path = project_dir.join(format!("{}.ncode", ir.name));
    fs::write(&code_path, native_code.to_json())
        .map_err(|err| format!("failed to write '{}': {err}", code_path.display()))?;
    Ok(code_path)
}

pub(crate) fn write_mir(
    hooks: &DumpHooks,
    project_dir: &Path,
    ir: &IrProject,
    target: &BuildTarget,
    packages: &[PathBuf],
    build_mode: NativeBuildMode,
) -> Result<PathBuf, String> {
    let module = (hooks.lower_validated_module)(ir, target, packages, build_mode, None)?;
    let native_plan = (hooks.lower_plan)(&module, DUMP_FLAVOR)?;
    native_plan.validate()?;
    os::linux::validate_native_object_plan(&native_plan)?;
    let mir = (hooks.lower_mir)(&module, &native_plan, packages, DUMP_FLAVOR)?;
    let mir_path = project_dir.join(format!("{}.mir", ir.name));
    fs::write(&mir_path, mir.to_json())
        .map_err(|err| format!("failed to write '{}': {err}", mir_path.display()))?;
    Ok(mir_path)
}
