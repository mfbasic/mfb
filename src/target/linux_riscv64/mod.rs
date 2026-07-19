use crate::arch;
use crate::ir::IrProject;
use crate::os;
use crate::os::linux::flavor::LinuxFlavor;
use crate::target::linux_common;
use crate::target::shared::{lower, validate};
use crate::target::{BackendCapabilities, BuildTarget, NativeBackend, NativeBuildMode};
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
            runtime_calls: linux_common::RUNTIME_CALLS,
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
        linux_common::write_nir(
            &DUMPS,
            project_dir,
            ir,
            &self.target(),
            packages,
            build_mode,
        )
    }

    fn write_native_plan(
        &self,
        project_dir: &Path,
        ir: &IrProject,
        packages: &[PathBuf],
        build_mode: NativeBuildMode,
    ) -> Result<PathBuf, String> {
        linux_common::write_native_plan(
            &DUMPS,
            project_dir,
            ir,
            &self.target(),
            packages,
            build_mode,
        )
    }

    fn write_native_object_plan(
        &self,
        project_dir: &Path,
        ir: &IrProject,
        packages: &[PathBuf],
        build_mode: NativeBuildMode,
    ) -> Result<PathBuf, String> {
        linux_common::write_native_object_plan(
            &DUMPS,
            project_dir,
            ir,
            &self.target(),
            packages,
            build_mode,
        )
    }

    fn write_native_code_plan(
        &self,
        project_dir: &Path,
        ir: &IrProject,
        packages: &[PathBuf],
        build_mode: NativeBuildMode,
    ) -> Result<PathBuf, String> {
        linux_common::write_native_code_plan(
            &DUMPS,
            project_dir,
            ir,
            &self.target(),
            packages,
            build_mode,
        )
    }

    fn write_mir(
        &self,
        project_dir: &Path,
        ir: &IrProject,
        packages: &[PathBuf],
        build_mode: NativeBuildMode,
    ) -> Result<PathBuf, String> {
        linux_common::write_mir(
            &DUMPS,
            project_dir,
            ir,
            &self.target(),
            packages,
            build_mode,
        )
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
        let native_plan = plan::lower_module(&module, flavor)?;
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

/// The five diagnostic dump writers are Linux-invariant (bug-321); only the
/// lowering entry points differ, and `lower_validated_module` stays here because
/// it carries this backend's build-mode policy.
const DUMPS: linux_common::DumpHooks = linux_common::DumpHooks {
    lower_validated_module,
    lower_plan: plan::lower_module,
    lower_code: code::lower_module,
    lower_mir: code::lower_module_mir,
};

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

#[cfg(test)]
mod tests {
    use super::*;

    /// bug-223, defense layer 3. `supports_app_mode()` is false so the CLI
    /// rejects `--app` for this target, but a non-CLI/API caller can construct a
    /// `LinuxApp` build directly. This guard must turn that into a clean `Err`
    /// **before** lowering reaches the `AppSupport::Unsupported` hard-stops in
    /// `linux_common::code`, which would abort the process instead.
    ///
    /// bug-321 moved the app-mode bodies into a shared layer, which is precisely
    /// the change that could have weakened this. It is per-backend and must stay
    /// so: aarch64 and x86-64 legitimately accept `NativeBuildMode::LinuxApp`.
    #[test]
    fn app_build_mode_is_rejected_before_lowering() {
        let ir = crate::testutil::lower_src("SUB main()\nEND SUB\n");
        let target = BACKEND.target();
        let Err(err) = lower_validated_module(&ir, &target, &[], NativeBuildMode::LinuxApp, None)
        else {
            panic!("riscv64 must reject an app build");
        };
        assert!(
            err.contains("do not support") && err.contains("riscv64"),
            "expected a clean rejection, got: {err}"
        );
    }

    /// The companion fact: `supports_app_mode()` stays false, which is what makes
    /// the CLI reject `--app` before it ever gets here (defense layer 2).
    #[test]
    fn app_mode_is_not_advertised() {
        assert!(!BACKEND.supports_app_mode());
    }

    /// Console builds must get PAST the build-mode guard — otherwise the
    /// rejection above would be passing for the wrong reason (a guard that
    /// rejects everything). This fixture has no entry point, so it fails later
    /// for an unrelated reason; what matters is that it is not the build-mode
    /// rejection.
    #[test]
    fn console_build_mode_passes_the_guard() {
        let ir = crate::testutil::lower_src("SUB main()\nEND SUB\n");
        let target = BACKEND.target();
        let err = match lower_validated_module(&ir, &target, &[], NativeBuildMode::Console, None) {
            Ok(_) => return,
            Err(err) => err,
        };
        assert!(
            !err.contains("do not support"),
            "console must not hit the app-mode build-mode guard, got: {err}"
        );
    }
}
