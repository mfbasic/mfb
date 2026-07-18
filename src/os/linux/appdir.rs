//! Linux app-mode AppDir emission (plan-51-A).
//!
//! The Linux twin of `crate::os::macos::link::write_app_bundle`: encode the
//! executable once through the ordinary path, then lay a standard directory tree
//! around it. The result is both directly runnable
//! (`./build/<name>.AppDir/AppRun`) and the exact payload plan-51-C seals into a
//! single-file `.AppImage`.

use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};

use crate::os::icon::{render_png, HICOLOR_SIZES, ROOT_ICON_SIZE};
use crate::os::BUILD_DIR;
use crate::target::linux_gtk::gtk_app_id;

/// Write an app-mode AppDir (plan-51-A §4.1) into the project's `build/`
/// directory:
///
/// ```text
/// build/<name>.AppDir/
///   AppRun -> usr/bin/<name>
///   <name>.desktop
///   <name>.png
///   .DirIcon -> <name>.png
///   usr/bin/<name>
///   usr/share/{applications,icons/hicolor/<N>x<N>/apps}/…
/// ```
///
/// The inner ELF is byte-identical to the `<name>-glibc.out` the console path
/// produces from the same image — **unless the build vendors native libraries**
/// (§4.4), where the two carry different `DT_RUNPATH` strings because they load
/// from different places.
///
/// `usr/lib/` is deliberately **not** created here: `copy_vendor_libraries`
/// creates it iff the build vendors something, so a non-vendoring AppDir carries
/// no empty directory. `usr/share/` *is* created and then left alone —
/// `copy_resources` (plan-55-A §4.3) writes `usr/share/<name>/` into it after
/// this function returns, so this writer must never wipe the tree it just made.
///
/// Returns the path to the `build/<name>.AppDir` directory.
pub(crate) fn write_appdir(
    project_dir: &Path,
    project_name: &str,
    bytes: &[u8],
    app_icon: Option<&Path>,
    app_version: &str,
) -> Result<PathBuf, String> {
    let appdir = project_dir
        .join(BUILD_DIR)
        .join(format!("{project_name}.AppDir"));

    let bin_dir = appdir.join("usr").join("bin");
    create_dir_all(&bin_dir)?;
    let executable = bin_dir.join(project_name);
    write_file(&executable, bytes)?;
    set_executable(&executable)?;

    // `usr/share/` is the root of everything a desktop environment reads, and
    // also plan-55's project-resource root. Created unconditionally so the later
    // resource copy has somewhere to land.
    let share_dir = appdir.join("usr").join("share");
    create_dir_all(&share_dir)?;

    // Render the icon once per size. Sharing `normalize_source` with the macOS
    // `.icns` path means a project `icon` that is present but not 1024×1024 now
    // fails a Linux build that previously ignored it — intended: the icon was
    // always declared, just never honored.
    for size in HICOLOR_SIZES {
        let dir = share_dir
            .join("icons")
            .join("hicolor")
            .join(format!("{size}x{size}"))
            .join("apps");
        create_dir_all(&dir)?;
        write_file(
            &dir.join(format!("{project_name}.png")),
            &render_png(app_icon, size)?,
        )?;
    }

    // appimagetool looks only at the AppDir root for the icon and never at
    // `usr/share/icons` (verified: zero references in `appimagetool.c`), so the
    // root copy is required rather than redundant.
    let root_icon = appdir.join(format!("{project_name}.png"));
    write_file(&root_icon, &render_png(app_icon, ROOT_ICON_SIZE)?)?;

    let app_id = gtk_app_id(project_name);
    let desktop = desktop_entry(project_name, &app_id, app_version);
    write_file(
        &appdir.join(format!("{project_name}.desktop")),
        desktop.as_bytes(),
    )?;
    let applications_dir = share_dir.join("applications");
    create_dir_all(&applications_dir)?;
    write_file(
        &applications_dir.join(format!("{project_name}.desktop")),
        desktop.as_bytes(),
    )?;

    // The AppImage runtime requires exactly one thing: an executable `/AppRun`,
    // which it `execv`s. A symlink is enough — the RUNPATH is baked into the ELF
    // at link time (§4.4), so the loader finds `usr/lib/` with no environment
    // help and the AppDir gains no dependency on a shell.
    make_symlink(&format!("usr/bin/{project_name}"), &appdir.join("AppRun"))?;
    make_symlink(&format!("{project_name}.png"), &appdir.join(".DirIcon"))?;

    Ok(appdir)
}

/// The freedesktop `.desktop` entry for an AppDir (plan-51-A §4.3), the
/// structural sibling of `app_info_plist` (`src/os/macos/link/mod.rs`).
///
/// - `Type`/`Name` are the only two keys freedesktop marks `required=TRUE`.
/// - `Exec` is the bare binary name; desktop-integration tools rewrite it to the
///   absolute AppImage path when they install the entry, so the value here only
///   has to be present and non-empty.
/// - `Icon` is **extension-less and mandatory**: appimagetool appends the
///   extension itself (`g_strdup_printf("%s/%s.png", source, icon_name)`), so
///   `Icon=<name>.png` makes it search for `<name>.png.png` and fail.
/// - `Categories` is *not* required by freedesktop, but appimagetool
///   hard-`die()`s without it. One line to stay tool-compatible.
/// - `StartupWMClass` must equal the GTK application id (§4.5) or the desktop
///   cannot associate the window with the launcher and the app shows a generic
///   icon in the dock.
/// - `X-AppImage-Version` is the only `X-AppImage-*` key appimagetool itself
///   writes; it is sourced from the manifest `version` that bug-248 already
///   threads to the backend.
/// - There is deliberately **no `Terminal=` key**: `Terminal=true` *disables*
///   desktop integration in libappimage, and omitting it defaults to false —
///   which is what a GUI app wants.
fn desktop_entry(project_name: &str, app_id: &str, app_version: &str) -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name={name}\n\
         Exec={name}\n\
         Icon={name}\n\
         Categories=Utility;\n\
         StartupWMClass={app_id}\n\
         X-AppImage-Version={version}\n",
        name = desktop_escape(project_name),
        app_id = desktop_escape(app_id),
        version = desktop_escape(app_version),
    )
}

/// Escape a `.desktop` value per the freedesktop spec: `\` is reserved in
/// values, and `;` terminates an entry in a list-typed key.
///
/// Project names reaching here have already passed manifest validation, but the
/// escape is deliberately not conditional on that — a format string that is only
/// safe for validated input is a format string one refactor away from being
/// unsafe.
fn desktop_escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace(';', "\\;")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

fn create_dir_all(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path).map_err(|err| format!("failed to create '{}': {err}", path.display()))
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    fs::write(path, bytes).map_err(|err| format!("failed to write '{}': {err}", path.display()))
}

fn set_executable(path: &Path) -> Result<(), String> {
    let mut permissions = fs::metadata(path)
        .map_err(|err| format!("failed to read '{}': {err}", path.display()))?
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions)
        .map_err(|err| format!("failed to mark '{}' executable: {err}", path.display()))
}

/// Create `link` pointing at the AppDir-relative `target`, replacing any file a
/// previous build left there. The build host is macOS or Linux — the compiler
/// only builds on Unix — so `std::os::unix::fs::symlink` is unconditional.
fn make_symlink(target: &str, link: &Path) -> Result<(), String> {
    match fs::remove_file(link) {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(format!("failed to replace '{}': {err}", link.display())),
    }
    symlink(target, link)
        .map_err(|err| format!("failed to link '{}' -> '{target}': {err}", link.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_entry_carries_every_required_key() {
        let entry = desktop_entry("hello", "dev.mfbasic.hello", "1.2.3");
        assert!(entry.starts_with("[Desktop Entry]\n"), "{entry}");
        assert!(entry.contains("\nType=Application\n"), "{entry}");
        assert!(entry.contains("\nName=hello\n"), "{entry}");
        assert!(entry.contains("\nExec=hello\n"), "{entry}");
        assert!(entry.contains("\nX-AppImage-Version=1.2.3\n"), "{entry}");
    }

    #[test]
    fn desktop_entry_icon_is_extension_less() {
        // appimagetool appends `.png` itself; `Icon=hello.png` makes it look for
        // `hello.png.png` and fail.
        let entry = desktop_entry("hello", "dev.mfbasic.hello", "1.0.0");
        assert!(entry.contains("\nIcon=hello\n"), "{entry}");
        assert!(!entry.contains("Icon=hello.png"), "{entry}");
    }

    #[test]
    fn desktop_entry_has_categories_and_no_terminal_key() {
        let entry = desktop_entry("hello", "dev.mfbasic.hello", "1.0.0");
        // Required by appimagetool, which hard-die()s without it.
        assert!(entry.contains("\nCategories=Utility;\n"), "{entry}");
        // `Terminal=true` disables desktop integration in libappimage; omitting
        // the key defaults it to false.
        assert!(!entry.contains("Terminal="), "{entry}");
    }

    #[test]
    fn desktop_entry_startup_wm_class_equals_the_gtk_app_id() {
        for name in ["hello", "my-app", "3d"] {
            let id = gtk_app_id(name);
            let entry = desktop_entry(name, &id, "1.0.0");
            assert!(
                entry.contains(&format!("\nStartupWMClass={id}\n")),
                "{name}: {entry}"
            );
        }
    }

    #[test]
    fn desktop_escape_covers_the_reserved_characters() {
        assert_eq!(desktop_escape("a;b"), "a\\;b");
        assert_eq!(desktop_escape("a\\b"), "a\\\\b");
        assert_eq!(desktop_escape("a\nb"), "a\\nb");
        assert_eq!(desktop_escape("plain"), "plain");
    }

    #[test]
    fn write_appdir_emits_the_full_layout() {
        let dir = tempfile::tempdir().unwrap();
        let appdir = write_appdir(dir.path(), "hello", b"\x7fELF fake", None, "0.1.0")
            .expect("write appdir");
        assert_eq!(
            appdir,
            dir.path().join("build").join("hello.AppDir"),
            "the AppDir lands in the project's build/ directory"
        );

        let executable = appdir.join("usr/bin/hello");
        assert!(executable.is_file(), "the ELF is at usr/bin/<name>");
        assert_eq!(std::fs::read(&executable).unwrap(), b"\x7fELF fake");
        assert_eq!(
            std::fs::metadata(&executable).unwrap().permissions().mode() & 0o777,
            0o755,
            "the executable bit survives"
        );

        // The AppImage runtime execv's /AppRun; it must resolve to the real ELF
        // and must be a symlink, not a second copy of it.
        let apprun = appdir.join("AppRun");
        assert_eq!(
            std::fs::read_link(&apprun).unwrap(),
            Path::new("usr/bin/hello")
        );
        assert!(std::fs::symlink_metadata(&apprun)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(
            std::fs::read_link(appdir.join(".DirIcon")).unwrap(),
            Path::new("hello.png")
        );

        assert!(appdir.join("hello.desktop").is_file());
        assert!(appdir
            .join("usr/share/applications/hello.desktop")
            .is_file());
        assert_eq!(
            std::fs::read(appdir.join("hello.desktop")).unwrap(),
            std::fs::read(appdir.join("usr/share/applications/hello.desktop")).unwrap(),
            "the two .desktop copies are byte-identical"
        );

        for size in HICOLOR_SIZES {
            let icon = appdir.join(format!(
                "usr/share/icons/hicolor/{size}x{size}/apps/hello.png"
            ));
            assert!(icon.is_file(), "missing {}", icon.display());
            let decoded = image::load_from_memory(&std::fs::read(&icon).unwrap()).unwrap();
            assert_eq!(
                image::GenericImageView::dimensions(&decoded),
                (size, size),
                "{size}: hicolor icon is its own size"
            );
        }
        let root_icon = image::load_from_memory(&std::fs::read(appdir.join("hello.png")).unwrap())
            .expect("root icon decodes");
        assert_eq!(
            image::GenericImageView::dimensions(&root_icon),
            (ROOT_ICON_SIZE, ROOT_ICON_SIZE)
        );

        // `usr/share/` exists for plan-55's resources to land in; `usr/lib/` does
        // not, because this build vendors nothing.
        assert!(appdir.join("usr/share").is_dir());
        assert!(
            !appdir.join("usr/lib").exists(),
            "a non-vendoring AppDir carries no empty usr/lib/"
        );
    }

    #[test]
    fn write_appdir_is_idempotent_across_rebuilds() {
        // The symlinks must survive a second build into the same directory, which
        // `symlink(2)` would otherwise reject with EEXIST.
        let dir = tempfile::tempdir().unwrap();
        write_appdir(dir.path(), "hello", b"first", None, "0.1.0").expect("first build");
        let appdir =
            write_appdir(dir.path(), "hello", b"second", None, "0.2.0").expect("second build");
        assert_eq!(
            std::fs::read(appdir.join("usr/bin/hello")).unwrap(),
            b"second"
        );
        assert_eq!(
            std::fs::read_link(appdir.join("AppRun")).unwrap(),
            Path::new("usr/bin/hello")
        );
        assert!(std::fs::read_to_string(appdir.join("hello.desktop"))
            .unwrap()
            .contains("X-AppImage-Version=0.2.0"));
    }

    #[test]
    fn write_appdir_rejects_a_non_1024_icon() {
        let dir = tempfile::tempdir().unwrap();
        let icon = dir.path().join("small.png");
        image::RgbaImage::from_pixel(64, 64, image::Rgba([1, 2, 3, 255]))
            .save(&icon)
            .unwrap();
        let err = write_appdir(dir.path(), "hello", b"x", Some(&icon), "0.1.0")
            .expect_err("a non-1024 icon must fail the build");
        assert!(err.contains("must be 1024×1024"), "{err}");
    }
}
