use std::env;
use std::path::{Path, PathBuf};

use crate::bytecode::BytecodeMetadata;
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
    ) -> Result<Vec<PathBuf>, String>;
    fn write_nir(
        &self,
        project_dir: &Path,
        ir: &IrProject,
        packages: &[PathBuf],
    ) -> Result<PathBuf, String>;
    fn write_native_plan(
        &self,
        project_dir: &Path,
        ir: &IrProject,
        packages: &[PathBuf],
    ) -> Result<PathBuf, String>;
    fn write_native_object_plan(
        &self,
        project_dir: &Path,
        ir: &IrProject,
        packages: &[PathBuf],
    ) -> Result<PathBuf, String>;
    fn write_native_code_plan(
        &self,
        project_dir: &Path,
        ir: &IrProject,
        packages: &[PathBuf],
    ) -> Result<PathBuf, String>;
}

static NATIVE_BACKENDS: &[&dyn NativeBackend] = &[&macos_aarch64::BACKEND, &linux_aarch64::BACKEND];

fn backend_for(target: &BuildTarget) -> Result<&'static dyn NativeBackend, String> {
    NATIVE_BACKENDS
        .iter()
        .copied()
        .find(|backend| backend.target() == *target)
        .ok_or_else(|| format!("native output does not support {} yet", target.name()))
}

pub fn write_executable(
    project_dir: &Path,
    ir: &IrProject,
    target: &BuildTarget,
    packages: &[PathBuf],
) -> Result<Vec<PathBuf>, String> {
    let backend = backend_for(target)?;
    if !backend.capabilities().executable {
        return Err(format!(
            "native executable output does not support {} yet",
            target.name()
        ));
    }
    backend.validate(ir, packages)?;
    backend.write_executable(project_dir, ir, packages)
}

pub fn write_nir(
    project_dir: &Path,
    ir: &IrProject,
    target: &BuildTarget,
    packages: &[PathBuf],
) -> Result<PathBuf, String> {
    let backend = backend_for(target)?;
    if !backend.capabilities().native_ir {
        return Err(format!(
            "native IR output does not support {} yet",
            target.name()
        ));
    }
    backend.validate(ir, packages)?;
    backend.write_nir(project_dir, ir, packages)
}

pub fn write_native_plan(
    project_dir: &Path,
    ir: &IrProject,
    target: &BuildTarget,
    packages: &[PathBuf],
) -> Result<PathBuf, String> {
    let backend = backend_for(target)?;
    if !backend.capabilities().native_plan {
        return Err(format!(
            "native plan output does not support {} yet",
            target.name()
        ));
    }
    backend.validate(ir, packages)?;
    backend.write_native_plan(project_dir, ir, packages)
}

pub fn write_native_object_plan(
    project_dir: &Path,
    ir: &IrProject,
    target: &BuildTarget,
    packages: &[PathBuf],
) -> Result<PathBuf, String> {
    let backend = backend_for(target)?;
    if !backend.capabilities().native_object_plan {
        return Err(format!(
            "native object plan output does not support {} yet",
            target.name()
        ));
    }
    backend.validate(ir, packages)?;
    backend.write_native_object_plan(project_dir, ir, packages)
}

pub fn write_native_code_plan(
    project_dir: &Path,
    ir: &IrProject,
    target: &BuildTarget,
    packages: &[PathBuf],
) -> Result<PathBuf, String> {
    let backend = backend_for(target)?;
    if !backend.capabilities().native_code_plan {
        return Err(format!(
            "native code plan output does not support {} yet",
            target.name()
        ));
    }
    backend.validate(ir, packages)?;
    backend.write_native_code_plan(project_dir, ir, packages)
}

pub fn write_package(
    project_dir: &Path,
    ir: &IrProject,
    metadata: &BytecodeMetadata,
    packages: &[PathBuf],
) -> Result<PathBuf, String> {
    package_mfp::write_package(project_dir, ir, metadata, packages)
}
