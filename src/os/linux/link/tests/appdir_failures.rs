//! What `write_appdir` reports when a path it must write is not writable.
//!
//! The AppDir is laid out file by file; a build that leaves a stale directory (or
//! anything else) where one of its files belongs must fail with the path named, not
//! produce a half-written AppDir the AppImage seal would then package.

use super::*;
use crate::os::icon::HICOLOR_SIZES;

fn appdir_path(root: &std::path::Path) -> std::path::PathBuf {
    root.join(crate::os::BUILD_DIR).join("failing-glibc.AppDir")
}

/// Write the AppDir with `obstacle` (a path relative to the AppDir) pre-created as a
/// non-empty directory, so whatever the writer puts there cannot be a file or a link.
fn write_with_obstacle(obstacle: &str) -> String {
    let root = tempfile::tempdir().unwrap();
    let blocked = appdir_path(root.path()).join(obstacle);
    std::fs::create_dir_all(blocked.join("inner")).unwrap();
    write_appdir(
        root.path(),
        "failing",
        "x86_64",
        LinuxFlavor::Glibc,
        &x86_static_image(),
        None,
        "0.1.0",
    )
    .err()
    .expect("a blocked path fails the build")
}

#[test]
fn a_directory_where_an_icon_belongs_names_the_icon_path() {
    let size = HICOLOR_SIZES[0];
    let error = write_with_obstacle(&format!(
        "usr/share/icons/hicolor/{size}x{size}/apps/failing.png"
    ));
    assert!(error.starts_with("failed to write '"), "{error}");
    assert!(error.contains("failing.png"), "{error}");
}

#[test]
fn a_directory_where_the_root_desktop_entry_belongs_names_it() {
    let error = write_with_obstacle("failing.desktop");
    assert!(error.starts_with("failed to write '"), "{error}");
    assert!(error.contains("failing.desktop"), "{error}");
}

#[test]
fn a_directory_where_the_applications_desktop_entry_belongs_names_it() {
    let error = write_with_obstacle("usr/share/applications/failing.desktop");
    assert!(error.starts_with("failed to write '"), "{error}");
    assert!(error.contains("applications"), "{error}");
}

#[test]
fn a_directory_where_the_apprun_link_belongs_fails_the_replace() {
    let error = write_with_obstacle("AppRun");
    assert!(error.starts_with("failed to replace '"), "{error}");
    assert!(error.contains("AppRun"), "{error}");
}

#[test]
fn a_file_where_an_appdir_directory_belongs_names_the_directory() {
    let root = tempfile::tempdir().unwrap();
    let appdir = appdir_path(root.path());
    std::fs::create_dir_all(&appdir).unwrap();
    std::fs::write(appdir.join("usr"), b"not a directory").unwrap();
    let error = write_appdir(
        root.path(),
        "failing",
        "x86_64",
        LinuxFlavor::Glibc,
        &x86_static_image(),
        None,
        "0.1.0",
    )
    .err()
    .expect("a file in place of usr/ fails the build");
    assert!(error.starts_with("failed to create '"), "{error}");
    assert!(error.contains("usr"), "{error}");
}
