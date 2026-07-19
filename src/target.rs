use std::env;
use std::path::{Path, PathBuf};

use crate::binary_repr::BinaryReprMetadata;
use crate::ir::IrProject;

pub mod linux_aarch64;
/// The Linux-invariant backend layer shared by all three Linux targets
/// (bug-321). Not a `NativeBackend` itself — the three registered backends keep
/// their own identities, targets, and capability reporting.
pub(crate) mod linux_common;
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
    /// correct point: a sealed artifact cannot gain files afterwards. Returns
    /// every artifact that replaces what [`NativeBackend::write_executable`]
    /// reported, or an empty vec to keep those.
    ///
    /// macOS returns empty — a `.app` is a directory and is already complete.
    /// The Linux backends seal **one AppImage per libc flavor** (plan-56-B §4.4)
    /// and, unless `keep_intermediate` (`--app-debug`), delete each AppDir.
    fn finalize_app_bundle(
        &self,
        project_dir: &Path,
        project_name: &str,
        keep_intermediate: bool,
    ) -> Result<Vec<PathBuf>, String> {
        let _ = (project_dir, project_name, keep_intermediate);
        Ok(Vec::new())
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
/// reject `-app` for targets whose backend lacks app mode, before any lowering
/// happens. (Not "non-macOS": Linux app mode exists as `NativeBuildMode::LinuxApp`
/// over GTK4. bug-93(3) corrected the same claim on the trait method and missed
/// this one.)
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
/// image before it closes. A non-empty result replaces the paths
/// `write_executable` reported; an empty one keeps them.
///
/// A no-op for console builds and for macOS, whose `.app` is a directory and is
/// already complete when `write_executable` returns.
pub fn finalize_app_bundle(
    project_dir: &Path,
    project_name: &str,
    target: &BuildTarget,
    build_mode: NativeBuildMode,
    keep_intermediate: bool,
) -> Result<Vec<PathBuf>, String> {
    if !build_mode.is_app() {
        return Ok(Vec::new());
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The app-mode-capable targets, as a `(name, supports_app_mode)` table.
    /// Kept explicit rather than derived so that registering a backend, or
    /// flipping one's `supports_app_mode`, fails this table loudly instead of
    /// silently agreeing with itself.
    const APP_MODE_MATRIX: &[(&str, bool)] = &[
        ("macos-aarch64", true),
        ("linux-aarch64", true),
        ("linux-x86_64", true),
        // rv64 is console-only: the GTK4 toolkit (`target::linux_gtk`) has not
        // been ported, so `-app` is rejected at the CLI (plan-99).
        ("linux-riscv64", false),
    ];

    #[test]
    fn native_build_mode_as_str() {
        assert_eq!(NativeBuildMode::Console.as_str(), "console");
        assert_eq!(NativeBuildMode::MacApp.as_str(), "macos-app");
        assert_eq!(NativeBuildMode::LinuxApp.as_str(), "linux-app");
    }

    #[test]
    fn native_build_mode_is_app() {
        assert!(!NativeBuildMode::Console.is_app());
        assert!(NativeBuildMode::MacApp.is_app());
        assert!(NativeBuildMode::LinuxApp.is_app());
    }

    #[test]
    fn build_target_host_is_nonempty() {
        let host = BuildTarget::host();
        assert!(!host.os.is_empty());
        assert!(!host.arch.is_empty());
    }

    #[test]
    fn build_target_name_joins_os_and_arch() {
        let target = BuildTarget {
            os: "linux".to_string(),
            arch: "x86_64".to_string(),
        };
        assert_eq!(target.name(), "linux-x86_64");
    }

    #[test]
    fn is_host_matches_the_running_machine() {
        assert!(BuildTarget::host().is_host());
        let other = BuildTarget {
            os: "plan9".to_string(),
            arch: "sparc".to_string(),
        };
        assert!(!other.is_host());
    }

    #[test]
    fn parse_accepts_every_registered_target() {
        for target in registered_targets() {
            let name = target.name();
            assert_eq!(BuildTarget::parse(&name), Ok(target), "parsing {name}");
        }
    }

    #[test]
    fn parse_splits_os_and_arch() {
        assert_eq!(
            BuildTarget::parse("macos-aarch64"),
            Ok(BuildTarget {
                os: "macos".to_string(),
                arch: "aarch64".to_string(),
            })
        );
    }

    #[test]
    fn parse_rejects_missing_dash() {
        let err = BuildTarget::parse("macos").unwrap_err();
        assert!(err.contains("os-arch format"), "unexpected message: {err}");
    }

    #[test]
    fn parse_rejects_empty_os() {
        assert!(BuildTarget::parse("-aarch64").is_err());
    }

    #[test]
    fn parse_rejects_empty_arch() {
        assert!(BuildTarget::parse("macos-").is_err());
    }

    #[test]
    fn parse_rejects_triple_component() {
        // A third `-v7` component fails the `arch.contains('-')` guard.
        assert!(BuildTarget::parse("linux-arm-v7").is_err());
    }

    #[test]
    fn parse_round_trips_name() {
        for target in registered_targets() {
            let name = target.name();
            let parsed = BuildTarget::parse(&name).expect("parse");
            assert_eq!(parsed.name(), name);
        }
    }

    #[test]
    fn backend_for_resolves_every_registered_target() {
        for target in registered_targets() {
            match backend_for(&target) {
                Ok(backend) => assert_eq!(backend.target(), target),
                Err(err) => panic!("expected a backend for {}: {err}", target.name()),
            }
        }
    }

    #[test]
    fn backend_for_unknown_target_errors() {
        let target = BuildTarget {
            os: "plan9".to_string(),
            arch: "sparc".to_string(),
        };
        match backend_for(&target) {
            Ok(_) => panic!("unexpected backend for plan9-sparc"),
            Err(err) => assert!(err.contains("plan9-sparc"), "unexpected message: {err}"),
        }
    }

    #[test]
    fn app_mode_support_matches_the_documented_matrix() {
        for (name, expected) in APP_MODE_MATRIX {
            let target = BuildTarget::parse(name).expect("parse");
            assert_eq!(
                target_supports_app_mode(&target),
                *expected,
                "{name} app-mode support",
            );
        }
    }

    /// The matrix above must stay in step with the registry: a newly registered
    /// backend has to be given a row rather than defaulting silently.
    #[test]
    fn app_mode_matrix_covers_every_registered_target() {
        for target in registered_targets() {
            let name = target.name();
            assert!(
                APP_MODE_MATRIX.iter().any(|(n, _)| *n == name),
                "{name} is registered but missing from APP_MODE_MATRIX",
            );
        }
        assert_eq!(APP_MODE_MATRIX.len(), registered_targets().len());
    }

    #[test]
    fn unknown_target_does_not_support_app_mode() {
        let target = BuildTarget {
            os: "plan9".to_string(),
            arch: "sparc".to_string(),
        };
        assert!(!target_supports_app_mode(&target));
    }

    #[test]
    fn registered_oses_and_arches_are_deduplicated() {
        let oses = registered_target_oses();
        let arches = registered_target_arches();
        assert!(!oses.is_empty() && !arches.is_empty());
        for (index, os) in oses.iter().enumerate() {
            assert!(!oses[..index].contains(os), "duplicate os {os}");
        }
        for (index, arch) in arches.iter().enumerate() {
            assert!(!arches[..index].contains(arch), "duplicate arch {arch}");
        }
        // Every registered target's tokens appear in the two vocabularies.
        for target in registered_targets() {
            assert!(oses.contains(&target.os), "missing os {}", target.os);
            assert!(
                arches.contains(&target.arch),
                "missing arch {}",
                target.arch
            );
        }
    }

    #[test]
    fn registered_targets_are_unique() {
        let targets = registered_targets();
        for (index, target) in targets.iter().enumerate() {
            assert!(
                !targets[..index].contains(target),
                "duplicate registered target {}",
                target.name(),
            );
        }
    }
}
