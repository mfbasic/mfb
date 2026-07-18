use std::env;
use std::path::{Path, PathBuf};

use crate::binary_repr::BinaryReprMetadata;
use crate::ir::IrProject;

pub mod linux_aarch64;
/// Linux GTK4 app-mode codegen shared by the aarch64 and x86-64 Linux targets.
pub(crate) mod linux_gtk;
pub mod linux_riscv64;
pub mod linux_x86_64;
pub mod macos_aarch64;
pub mod package_mfp;
pub(crate) mod shared;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildTarget {
    pub os: String,
    pub arch: String,
}

/// Selects which native runtime/output shape a native backend produces.
///
/// `Console` is the standard terminal/file-descriptor executable. `MacApp` is the
/// macOS GUI app-mode output (`mfb build -app`) whose `io::*` built-ins target an
/// AppKit window instead of the terminal (see src/docs/spec/app/01_macos-runtime.md).
/// `LinuxApp` is the Linux counterpart whose `io::*` built-ins target a GTK4 window
/// (see src/docs/spec/app/02_linux-runtime.md). The shared lowering treats both app
/// modes uniformly via [`NativeBuildMode::is_app`]; the target OS selects the
/// toolkit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeBuildMode {
    Console,
    MacApp,
    LinuxApp,
}

impl NativeBuildMode {
    /// Stable identifier recorded in NIR / native plan / native code plan metadata
    /// and used by goldens and validation.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            NativeBuildMode::Console => "console",
            NativeBuildMode::MacApp => "macos-app",
            NativeBuildMode::LinuxApp => "linux-app",
        }
    }

    /// Whether this is a GUI app-mode build (`mfb build -app`), regardless of the
    /// target OS / toolkit. Shared lowering branches on this so console behavior is
    /// shared by every target and app behavior is shared by every app toolkit.
    pub(crate) fn is_app(self) -> bool {
        matches!(self, NativeBuildMode::MacApp | NativeBuildMode::LinuxApp)
    }
}

impl BuildTarget {
    pub fn host() -> Self {
        Self {
            os: env::consts::OS.to_string(),
            arch: env::consts::ARCH.to_string(),
        }
    }

    pub fn name(&self) -> String {
        format!("{}-{}", self.os, self.arch)
    }

    /// Whether this target matches the machine running the compiler, so a
    /// freshly built executable can be run directly (used by `mfb test`).
    pub fn is_host(&self) -> bool {
        self.os == env::consts::OS && self.arch == env::consts::ARCH
    }

    pub fn parse(value: &str) -> Result<Self, String> {
        let Some((os, arch)) = value.split_once('-') else {
            return Err(format!("target '{value}' must use os-arch format"));
        };
        if os.is_empty() || arch.is_empty() || arch.contains('-') {
            return Err(format!("target '{value}' must use os-arch format"));
        }
        Ok(Self {
            os: os.to_string(),
            arch: arch.to_string(),
        })
    }
}

pub(crate) struct BackendCapabilities {
    pub(crate) executable: bool,
    pub(crate) native_ir: bool,
    pub(crate) native_plan: bool,
    pub(crate) native_object_plan: bool,
    pub(crate) native_code_plan: bool,
    pub(crate) runtime_calls: &'static [&'static str],
}

pub(crate) trait NativeBackend: Sync {
    fn target(&self) -> BuildTarget;
    fn capabilities(&self) -> BackendCapabilities;
    fn validate(&self, ir: &IrProject, packages: &[PathBuf]) -> Result<(), String>;
    #[allow(clippy::too_many_arguments)]
    fn write_executable(
        &self,
        project_dir: &Path,
        ir: &IrProject,
        packages: &[PathBuf],
        signing_metadata: Option<&[u8]>,
        build_mode: NativeBuildMode,
        app_icon: Option<&Path>,
        // bug-248: the manifest `version`, published as the macOS app bundle's
        // `CFBundleShortVersionString`/`CFBundleVersion`. Required in app mode;
        // ignored by console builds and by backends without a bundle format.
        app_version: Option<&str>,
        // plan-46-D §4.2/§4.3: whether this build resolved any `vendor` native
        // library, so the backend emits an RPATH pointing at the vendor directory
        // beside the executable. The *string* is the backend's choice, because it
        // is per output shape (`$ORIGIN/vendor`, `@loader_path/vendor`, or
        // `@executable_path/../Frameworks` for a macOS `.app`); the caller only
        // knows whether there is anything to point at.
        vendors_native_libraries: bool,
        // plan-15 D3: stdin broadcast-log backpressure cap from the manifest
        // `"config"` section, or `None` to bake `STDIN_LOG_CAP_DEFAULT`.
        stdin_log_cap: Option<u64>,
    ) -> Result<Vec<PathBuf>, String>;
    fn write_nir(
        &self,
        project_dir: &Path,
        ir: &IrProject,
        packages: &[PathBuf],
        build_mode: NativeBuildMode,
    ) -> Result<PathBuf, String>;
    fn write_native_plan(
        &self,
        project_dir: &Path,
        ir: &IrProject,
        packages: &[PathBuf],
        build_mode: NativeBuildMode,
    ) -> Result<PathBuf, String>;
    fn write_native_object_plan(
        &self,
        project_dir: &Path,
        ir: &IrProject,
        packages: &[PathBuf],
        build_mode: NativeBuildMode,
    ) -> Result<PathBuf, String>;
    fn write_native_code_plan(
        &self,
        project_dir: &Path,
        ir: &IrProject,
        packages: &[PathBuf],
        build_mode: NativeBuildMode,
    ) -> Result<PathBuf, String>;
    /// Write the target-neutral MIR dump (`-mir`, plan-00-A §12a). Shares the
    /// `native_code_plan` capability (same lowering, captured before register
    /// allocation and instruction selection).
    fn write_mir(
        &self,
        project_dir: &Path,
        ir: &IrProject,
        packages: &[PathBuf],
        build_mode: NativeBuildMode,
    ) -> Result<PathBuf, String>;
    /// Whether this backend supports app mode (`mfb build -app`). macOS backends
    /// advertise the AppKit runtime and Linux backends the GTK4 one; the CLI
    /// rejects `-app` for any backend returning false.
    fn supports_app_mode(&self) -> bool {
        false
    }

    /// Finalize an app-mode build after vendored libraries and resources are in
    /// place (plan-51-C §3.2).
    ///
    /// Runs after `copy_vendor_libraries`/`copy_resources`, which is the only
    /// correct point: a sealed artifact cannot gain files afterwards. Returns the
    /// path that replaces what [`NativeBackend::write_executable`] reported, or
    /// `None` to keep it.
    ///
    /// macOS returns `None` — a `.app` is a directory and is already complete.
    /// The Linux backends seal the AppDir into a single `.AppImage` and, unless
    /// `keep_intermediate` (`--app-debug`), delete the AppDir.
    fn finalize_app_bundle(
        &self,
        project_dir: &Path,
        project_name: &str,
        keep_intermediate: bool,
    ) -> Result<Option<PathBuf>, String> {
        let _ = (project_dir, project_name, keep_intermediate);
        Ok(None)
    }
}

static NATIVE_BACKENDS: &[&dyn NativeBackend] = &[
    &macos_aarch64::BACKEND,
    &linux_aarch64::BACKEND,
    &linux_x86_64::BACKEND,
    &linux_riscv64::BACKEND,
];

/// The `os` token of every registered native backend, deduplicated, in registry
/// order.
///
/// This is the canonical `os` vocabulary for native-library locators (plan-46-A
/// §4.1). It is derived from [`NATIVE_BACKENDS`] rather than hardcoded so that
/// registering a backend widens the accepted set — and the plan-46-B coverage
/// matrix — for free.
pub fn registered_target_oses() -> Vec<String> {
    let mut oses: Vec<String> = Vec::new();
    for backend in NATIVE_BACKENDS {
        let os = backend.target().os;
        if !oses.contains(&os) {
            oses.push(os);
        }
    }
    oses
}

/// The `arch` token of every registered native backend, deduplicated, in registry
/// order. The canonical `arch` vocabulary for native-library locators
/// (plan-46-A §4.1).
pub fn registered_target_arches() -> Vec<String> {
    let mut arches: Vec<String> = Vec::new();
    for backend in NATIVE_BACKENDS {
        let arch = backend.target().arch;
        if !arches.contains(&arch) {
            arches.push(arch);
        }
    }
    arches
}

/// The `(os, arch)` pair of every registered native backend, in registry order.
///
/// Crossed with the libc axis (linux only) this yields the plan-46-B §4.2
/// supported-target coverage matrix.
pub fn registered_targets() -> Vec<BuildTarget> {
    NATIVE_BACKENDS
        .iter()
        .map(|backend| backend.target())
        .collect()
}

fn backend_for(target: &BuildTarget) -> Result<&'static dyn NativeBackend, String> {
    NATIVE_BACKENDS
        .iter()
        .copied()
        .find(|backend| backend.target() == *target)
        .ok_or_else(|| format!("native output does not support {} yet", target.name()))
}

/// Whether the resolved target supports `mfb build -app`. Used by the CLI to
/// reject `-app` for non-macOS targets before any lowering happens.
pub fn target_supports_app_mode(target: &BuildTarget) -> bool {
    backend_for(target)
        .map(|backend| backend.supports_app_mode())
        .unwrap_or(false)
}

#[allow(clippy::too_many_arguments)]
pub fn write_executable(
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
    let backend = backend_for(target)?;
    if !backend.capabilities().executable {
        return Err(format!(
            "native executable output does not support {} yet",
            target.name()
        ));
    }
    backend.validate(ir, packages)?;
    backend.write_executable(
        project_dir,
        ir,
        packages,
        signing_metadata,
        build_mode,
        app_icon,
        app_version,
        vendors_native_libraries,
        stdin_log_cap,
    )
}

/// Finalize an app-mode build once every file that belongs inside the artifact
/// is in place (plan-51-C §3.2/§4.5).
///
/// Called from the CLI *after* `copy_vendor_libraries` and `copy_resources`,
/// because an AppImage is a sealed file: the libraries have to be inside the
/// image before it closes. `Some(path)` replaces the paths `write_executable`
/// reported; `None` keeps them.
///
/// A no-op for console builds and for macOS, whose `.app` is a directory and is
/// already complete when `write_executable` returns.
pub fn finalize_app_bundle(
    project_dir: &Path,
    project_name: &str,
    target: &BuildTarget,
    build_mode: NativeBuildMode,
    keep_intermediate: bool,
) -> Result<Option<PathBuf>, String> {
    if !build_mode.is_app() {
        return Ok(None);
    }
    backend_for(target)?.finalize_app_bundle(project_dir, project_name, keep_intermediate)
}

pub fn write_nir(
    project_dir: &Path,
    ir: &IrProject,
    target: &BuildTarget,
    packages: &[PathBuf],
    build_mode: NativeBuildMode,
) -> Result<PathBuf, String> {
    let backend = backend_for(target)?;
    if !backend.capabilities().native_ir {
        return Err(format!(
            "native IR output does not support {} yet",
            target.name()
        ));
    }
    backend.validate(ir, packages)?;
    backend.write_nir(project_dir, ir, packages, build_mode)
}

pub fn write_native_plan(
    project_dir: &Path,
    ir: &IrProject,
    target: &BuildTarget,
    packages: &[PathBuf],
    build_mode: NativeBuildMode,
) -> Result<PathBuf, String> {
    let backend = backend_for(target)?;
    if !backend.capabilities().native_plan {
        return Err(format!(
            "native plan output does not support {} yet",
            target.name()
        ));
    }
    backend.validate(ir, packages)?;
    backend.write_native_plan(project_dir, ir, packages, build_mode)
}

pub fn write_native_object_plan(
    project_dir: &Path,
    ir: &IrProject,
    target: &BuildTarget,
    packages: &[PathBuf],
    build_mode: NativeBuildMode,
) -> Result<PathBuf, String> {
    let backend = backend_for(target)?;
    if !backend.capabilities().native_object_plan {
        return Err(format!(
            "native object plan output does not support {} yet",
            target.name()
        ));
    }
    backend.validate(ir, packages)?;
    backend.write_native_object_plan(project_dir, ir, packages, build_mode)
}

pub fn write_native_code_plan(
    project_dir: &Path,
    ir: &IrProject,
    target: &BuildTarget,
    packages: &[PathBuf],
    build_mode: NativeBuildMode,
) -> Result<PathBuf, String> {
    let backend = backend_for(target)?;
    if !backend.capabilities().native_code_plan {
        return Err(format!(
            "native code plan output does not support {} yet",
            target.name()
        ));
    }
    backend.validate(ir, packages)?;
    backend.write_native_code_plan(project_dir, ir, packages, build_mode)
}

pub fn write_mir(
    project_dir: &Path,
    ir: &IrProject,
    target: &BuildTarget,
    packages: &[PathBuf],
    build_mode: NativeBuildMode,
) -> Result<PathBuf, String> {
    let backend = backend_for(target)?;
    // The MIR dump runs the same lowering as `-ncode`, so it shares that
    // capability gate.
    if !backend.capabilities().native_code_plan {
        return Err(format!("MIR output does not support {} yet", target.name()));
    }
    backend.validate(ir, packages)?;
    backend.write_mir(project_dir, ir, packages, build_mode)
}

pub fn write_package(
    project_dir: &Path,
    ir: &IrProject,
    metadata: &BinaryReprMetadata,
    packages: &[PathBuf],
    signing: Option<&package_mfp::PackageSigning>,
) -> Result<PathBuf, String> {
    package_mfp::write_package(project_dir, ir, metadata, packages, signing)
}
