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
            runtime_calls: linux_common::RUNTIME_CALLS,
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
        app_icon: Option<&Path>,
        app_version: Option<&str>,
        vendors_native_libraries: bool,
        stdin_log_cap: Option<u64>,
    ) -> Result<Vec<PathBuf>, String> {
        write_executable(
            project_dir,
            ir,
            &self.target(),
            packages,
            signing_metadata,
            build_mode,
            app_icon,
            app_version,
            vendors_native_libraries,
            stdin_log_cap,
        )
    }

    /// plan-51-C §4.5: seal the AppDir plan-51-A wrote — now complete with its
    /// vendored libraries and resources — into a single `build/<name>.AppImage`,
    /// then drop the intermediate unless `--app-debug` asked to keep it.
    fn finalize_app_bundle(
        &self,
        project_dir: &Path,
        project_name: &str,
        keep_intermediate: bool,
    ) -> Result<Vec<PathBuf>, String> {
        // One AppImage per libc flavor (plan-56-B §4.4), in the same order the
        // AppDirs were written.
        let mut sealed = Vec::new();
        for &flavor in &LinuxFlavor::ALL {
            sealed.push(os::linux::seal_appimage(
                project_dir,
                project_name,
                flavor,
                "aarch64",
            )?);
            if !keep_intermediate {
                os::linux::remove_appdir(project_dir, project_name, flavor)?;
            }
        }
        Ok(sealed)
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
    app_icon: Option<&Path>,
    app_version: Option<&str>,
    vendors_native_libraries: bool,
    stdin_log_cap: Option<u64>,
) -> Result<Vec<PathBuf>, String> {
    let module = lower_validated_module(ir, target, packages, build_mode, stdin_log_cap)?;
    let app_mode = build_mode.is_app();
    // plan-56-B §4.1: app mode is no longer glibc-only. GTK4 exists in the musl
    // world (Alpine's `gtk4.0`), and plan-56-A made the import surface
    // flavor-correct, so `--app` emits one AppImage per libc exactly as the
    // console build emits one `.out` per libc.
    let flavors: &[LinuxFlavor] = &LinuxFlavor::ALL;
    let mut paths = Vec::new();
    for &flavor in flavors {
        let native_plan = plan::lower_module(&module, flavor)?;
        native_plan.validate()?;
        os::linux::validate_native_object_plan(&native_plan)?;
        let native_code = code::lower_module(&module, &native_plan, packages, flavor)?;
        native_code.validate()?;
        let mut image = arch::aarch64::encode::encode(&native_code)?;
        image.signing_metadata = signing_metadata.map(|metadata| metadata.to_vec());
        // plan-46-D §4.2: point the loader at the `vendor/` directory beside the
        // executable, so a bare-filename `dlopen` of a vendored library resolves
        // and keeps resolving after the whole `build/` directory is moved. Emitted
        // only when the build vendors something, so every other binary stays
        // byte-identical.
        if vendors_native_libraries {
            // plan-51-A §4.4: the two output shapes load from different places —
            // `build/vendor/` beside a console `.out`, `usr/lib/` inside an
            // AppDir whose executable sits at `usr/bin/<name>` — so each carries
            // its own RUNPATH. Must stay in lockstep with `vendor_output_dirs`
            // (`src/cli/build.rs`): the loader looks exactly there and nowhere
            // else.
            image.rpaths = vec![if app_mode {
                crate::os::ELF_APPDIR_VENDOR_RPATH.to_string()
            } else {
                crate::os::ELF_VENDOR_RPATH.to_string()
            }];
        }
        paths.push(if app_mode {
            // bug-248's `app_version` gains its second consumer here (the
            // `.desktop` `X-AppImage-Version`), so a missing one is an internal
            // error rather than a silently empty key — mirroring
            // `macos_aarch64/mod.rs`.
            let version =
                app_version.ok_or("internal error: app mode requires the manifest version")?;
            os::linux::write_linked_appdir(
                project_dir,
                &ir.name,
                "aarch64",
                flavor,
                &image,
                app_icon,
                version,
            )?
        } else {
            os::linux::write_linked_executable(project_dir, &ir.name, "aarch64", flavor, &image)?
        });
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
    let module = lower::lower_project(ir, target.name(), packages, build_mode, stdin_log_cap)?;
    validate::validate_nir(&module)?;
    validate::validate_capabilities(&module, &BACKEND.capabilities())?;
    Ok(module)
}
