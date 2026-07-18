/// Platform-neutral app-icon decode/validate/render, shared by the macOS `.icns`
/// pipeline and the Linux AppDir PNG set (plan-51-A §4.2).
pub(crate) mod icon;
pub(crate) mod linux;
pub(crate) mod macos;
pub(crate) mod note;

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
