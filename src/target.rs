use std::env;
use std::path::{Path, PathBuf};

use crate::binary_repr::BinaryReprMetadata;
use crate::ir::IrProject;

pub mod linux_aarch64;
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
/// AppKit window instead of the terminal (see src/spec/app/01_macos-runtime.md).
/// `LinuxApp` is the Linux counterpart whose `io::*` built-ins target a GTK4 window
/// (see src/spec/app/02_linux-runtime.md). The shared lowering treats both app
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
    fn write_executable(
        &self,
        project_dir: &Path,
        ir: &IrProject,
        packages: &[PathBuf],
        signing_metadata: Option<&[u8]>,
        build_mode: NativeBuildMode,
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
    /// Whether this backend supports app mode (`mfb build -app`). Only macOS
    /// backends advertise this; the CLI rejects `-app` for any other target.
    fn supports_app_mode(&self) -> bool {
        false
    }
}

static NATIVE_BACKENDS: &[&dyn NativeBackend] = &[&macos_aarch64::BACKEND, &linux_aarch64::BACKEND];

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

pub fn write_executable(
    project_dir: &Path,
    ir: &IrProject,
    target: &BuildTarget,
    packages: &[PathBuf],
    signing_metadata: Option<&[u8]>,
    build_mode: NativeBuildMode,
) -> Result<Vec<PathBuf>, String> {
    let backend = backend_for(target)?;
    if !backend.capabilities().executable {
        return Err(format!(
            "native executable output does not support {} yet",
            target.name()
        ));
    }
    backend.validate(ir, packages)?;
    backend.write_executable(project_dir, ir, packages, signing_metadata, build_mode)
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

pub fn write_package(
    project_dir: &Path,
    ir: &IrProject,
    metadata: &BinaryReprMetadata,
    packages: &[PathBuf],
    signing_key: Option<&[u8]>,
) -> Result<PathBuf, String> {
    package_mfp::write_package(project_dir, ir, metadata, packages, signing_key)
}
