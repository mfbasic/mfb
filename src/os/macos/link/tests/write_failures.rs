//! A Mach-O executable or `.app` bundle that cannot be written fails the build
//! with the path that could not be created or written — never a panic and
//! never a half-reported success. Each case blocks one step by putting a file
//! where a directory must go, or a directory where a file must go.

use super::*;

/// The smallest image that links: `_main` is one `ret`.
fn ret_image() -> EncodedImage {
    EncodedImage {
        text: vec![0xc0, 0x03, 0x5f, 0xd6],
        symbols: vec![text_symbol("_main", 0)],
        ..none()
    }
}

fn refusal(result: Result<PathBuf, String>) -> String {
    result.expect_err("the write must fail")
}

#[test]
fn a_console_executable_that_cannot_be_written_names_the_path() {
    let dir = tempfile::tempdir().unwrap();
    // The project directory is a file: `build/` cannot be created under it.
    let not_a_dir = dir.path().join("project");
    fs::write(&not_a_dir, "x").unwrap();
    let build = not_a_dir.join(BUILD_DIR);
    let err = refusal(write_executable(&not_a_dir, "app", &ret_image()));
    assert!(
        err.starts_with(&format!("failed to create '{}': ", build.display())),
        "{err}"
    );

    // `build/app.out` is a directory: the executable cannot be written there.
    let out = dir.path().join(BUILD_DIR).join("app.out");
    fs::create_dir_all(&out).unwrap();
    let err = refusal(write_executable(dir.path(), "app", &ret_image()));
    assert!(
        err.starts_with(&format!("failed to write '{}': ", out.display())),
        "{err}"
    );
}

#[test]
fn an_app_bundle_that_cannot_be_written_names_the_path() {
    let bundle = |root: &Path| root.join(BUILD_DIR).join("app.app").join("Contents");
    let write = |root: &Path| write_app_bundle(root, "app", &ret_image(), None, "1.0.0");

    // The project directory is a file: `Contents/MacOS` cannot be created.
    let dir = tempfile::tempdir().unwrap();
    let not_a_dir = dir.path().join("project");
    fs::write(&not_a_dir, "x").unwrap();
    let err = refusal(write(&not_a_dir));
    assert!(
        err.starts_with(&format!(
            "failed to create '{}': ",
            bundle(&not_a_dir).join("MacOS").display()
        )),
        "{err}"
    );

    // `Contents/Resources` is a file.
    let dir = tempfile::tempdir().unwrap();
    let resources = bundle(dir.path()).join(crate::os::MACOS_APP_RESOURCES_DIR);
    fs::create_dir_all(bundle(dir.path())).unwrap();
    fs::write(&resources, "x").unwrap();
    let err = refusal(write(dir.path()));
    assert!(
        err.starts_with(&format!("failed to create '{}': ", resources.display())),
        "{err}"
    );

    // `AppIcon.icns` is a directory.
    let dir = tempfile::tempdir().unwrap();
    let icns = bundle(dir.path())
        .join(crate::os::MACOS_APP_RESOURCES_DIR)
        .join("AppIcon.icns");
    fs::create_dir_all(&icns).unwrap();
    let err = refusal(write(dir.path()));
    assert!(
        err.starts_with(&format!("failed to write '{}': ", icns.display())),
        "{err}"
    );

    // `Info.plist` is a directory.
    let dir = tempfile::tempdir().unwrap();
    let plist = bundle(dir.path()).join("Info.plist");
    fs::create_dir_all(&plist).unwrap();
    let err = refusal(write(dir.path()));
    assert!(
        err.starts_with(&format!("failed to write '{}': ", plist.display())),
        "{err}"
    );
}
