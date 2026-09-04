/// Platform-neutral app-icon decode/validate/render, shared by the macOS `.icns`
/// pipeline and the Linux AppDir PNG set (plan-51-A §4.2).
pub(crate) mod icon;
/// AArch64 relocation encoders and bounds-checked byte emit/patch helpers shared
/// by the Mach-O and ELF linkers (bug-335 A4/A5). The instruction-encoding
/// constants are ISA facts; only per-target relocation dispatch stays per platform.
mod link_encode;
pub(crate) mod linux;
pub(crate) mod macos;
pub(crate) mod note;
/// ISA-neutral native object-plan model (the plan structs, their JSON rendering,
/// and the dedup/align helpers) shared by the Mach-O and ELF object writers
/// (bug-335 A1). The format-specific `lower_plan`/validation stay per platform.
mod object_plan;
/// Windows PE/COFF container writer (plan-47-C). A leaf sibling of `linux`/`macos`
/// that lands before the `windows-x86_64` backend (plan-47-B) selects it.
pub(crate) mod windows;

/// Refuse a project name that cannot be a single path component before it is
/// `Path::join`ed into an artifact path (bug-503, audit-3 LNK-12).
///
/// Every writer in this module forms its output path as `<dir>/<name><suffix>`
/// and the executables are then made `0755`. A `../x` name escapes `build/`, a
/// leading `/` makes the target absolute, a leading `.` hides the file. The
/// manifest gate (`manifest::validate_name`) rejects such a name first; this is
/// the defence in depth at the boundary that actually touches the filesystem, so
/// no future caller can bypass the gate by constructing an `IrProject` directly.
/// Same charset as a `.mfp` package name (`validate_package_name`).
pub(crate) fn validate_output_name(name: &str) -> Result<(), String> {
    crate::manifest::package::validate_package_name(name).map_err(|_| {
        format!(
            "refusing to write a build artifact: project name `{name}` is not a valid path \
             component (expected [A-Za-z0-9_][A-Za-z0-9_.-]*)"
        )
    })
}

/// The per-project directory every build artifact is written into (plan-46-D
/// §4.1): `<project dir>/build/<name>.out`, `<project dir>/build/<name>.app`,
/// and the `vendor/` directory an RPATH-bearing build points at.
///
/// One fixed name rather than the project name, so a single `.gitignore` line
/// (`build/`) covers every project's output. The directory is also the unit of
/// relocation: the executable and its `vendor/` move together.
pub(crate) const BUILD_DIR: &str = "build";

/// The directory, inside [`BUILD_DIR`], holding the native libraries a build
/// vendors (plan-46-D §4.5). Flat: one filename means one file.
pub(crate) const VENDOR_DIR: &str = "vendor";

/// ELF `DT_RUNPATH` for a vendored build (plan-46-D §4.2). `$ORIGIN` is expanded
/// by the loader, not the build — take care that no format string interpolates it.
pub(crate) const ELF_VENDOR_RPATH: &str = "$ORIGIN/vendor";

/// ELF `DT_RUNPATH` for a vendored **AppDir** build (plan-51-A §4.4): the
/// executable sits at `usr/bin/<name>` and its libraries at `usr/lib/`, the
/// layout every AppDir-consuming tool expects. `$ORIGIN` is expanded by the
/// loader, not the build — take care that no format string interpolates it.
pub(crate) const ELF_APPDIR_VENDOR_RPATH: &str = "$ORIGIN/../lib";

/// Mach-O `LC_RPATH` for a vendored **console** build (plan-46-D §4.4): the
/// executable sits at `build/<name>.out` and its libraries at `build/vendor/`.
pub(crate) const MACHO_CONSOLE_VENDOR_RPATH: &str = "@loader_path/vendor";

/// Mach-O `LC_RPATH` for a vendored **`.app` bundle** (plan-46-D §4.4): dylibs go
/// in the platform-standard `Contents/Frameworks/`, which is where Apple specifies
/// private shared libraries live and where every bundle-inspecting tool expects
/// them. `@executable_path` matches the string Xcode emits for app targets
/// (`@loader_path` would be equivalent here, since the loader *is* the
/// executable).
pub(crate) const MACHO_APP_VENDOR_RPATH: &str = "@executable_path/../Frameworks";

/// The `.app` bundle subdirectory holding vendored dylibs (plan-46-D §4.4).
pub(crate) const MACOS_APP_FRAMEWORKS_DIR: &str = "Frameworks";

/// The `.app` bundle subdirectory holding project resources and `AppIcon.icns`
/// (plan-55-A §4.3), where Apple specifies bundle resources live. `os::resourcePath`
/// (plan-55-B) resolves against `Contents/Resources/` in an app build.
pub(crate) const MACOS_APP_RESOURCES_DIR: &str = "Resources";
