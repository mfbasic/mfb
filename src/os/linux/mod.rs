/// App-mode AppDir emission (plan-51-A): the launchable directory tree a Linux
/// `--app` build produces, and the payload plan-51-C seals into an `.AppImage`.
pub(crate) mod appdir;
/// AppImage sealing (plan-51-C): the embedded type-2 runtime plus the
/// AppDir → squashfs → single-file concatenation.
mod appimage;
pub(crate) mod flavor;
mod link;
mod object;
/// SquashFS 4.0 image writer (plan-51-B), the second half of an AppImage.
pub(crate) mod squashfs;

use crate::arch::aarch64::encode::EncodedImage;
use crate::target::shared::plan::NativePlan;
use flavor::LinuxFlavor;
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) fn write_native_object_plan(
    project_dir: &Path,
    project_name: &str,
    plan: &NativePlan,
) -> Result<PathBuf, String> {
    let object_plan = object::lower_plan(plan)?;
    object_plan.validate()?;
    let object_path = project_dir.join(format!("{project_name}.nobj"));
    fs::write(&object_path, object_plan.to_json())
        .map_err(|err| format!("failed to write '{}': {err}", object_path.display()))?;
    Ok(object_path)
}

pub(crate) fn validate_native_object_plan(plan: &NativePlan) -> Result<(), String> {
    object::lower_plan(plan)?.validate()
}

pub(crate) fn write_linked_executable(
    project_dir: &Path,
    project_name: &str,
    arch: &str,
    flavor: LinuxFlavor,
    image: &EncodedImage,
) -> Result<PathBuf, String> {
    link::write_executable(project_dir, project_name, arch, flavor, image)
}

/// Link `image` and write it as an app-mode AppDir (plan-51-A §4.1), returning
/// the path to the `build/<name>.AppDir` directory. The Linux sibling of
/// `crate::os::macos::write_linked_app_bundle`.
///
/// `app_icon` is the resolved project `icon` source (plan-22-A); `None` uses the
/// compiler's embedded default. `app_version` is the manifest `version`,
/// published as the `.desktop` entry's `X-AppImage-Version`.
pub(crate) fn write_linked_appdir(
    project_dir: &Path,
    project_name: &str,
    arch: &str,
    flavor: LinuxFlavor,
    image: &EncodedImage,
    app_icon: Option<&Path>,
    app_version: &str,
) -> Result<PathBuf, String> {
    link::write_appdir(
        project_dir,
        project_name,
        arch,
        flavor,
        image,
        app_icon,
        app_version,
    )
}

/// Seal `build/<name>.AppDir` into `build/<name>.AppImage` (plan-51-C §4.4),
/// returning the AppImage path. Must run **after** vendored libraries are copied
/// into the AppDir: a sealed artifact cannot gain files afterwards.
pub(crate) fn seal_appimage(
    project_dir: &Path,
    project_name: &str,
    flavor: LinuxFlavor,
    arch: &str,
) -> Result<PathBuf, String> {
    appimage::seal(project_dir, project_name, flavor.suffix(), arch)
}

/// Remove the intermediate AppDir the seal consumed (plan-51-C §3.3). Skipped by
/// `--app-debug`, which keeps it for inspection.
pub(crate) fn remove_appdir(
    project_dir: &Path,
    project_name: &str,
    flavor: LinuxFlavor,
) -> Result<(), String> {
    appimage::remove_appdir(project_dir, project_name, flavor.suffix())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arch::aarch64::encode::{EncodedSection, EncodedSymbol};
    use crate::target::shared::plan::{
        NativePlan, PlanCall, PlannedFunction, StorageClass, StorageType,
    };

    fn plan() -> NativePlan {
        NativePlan {
            target: "linux-aarch64".to_string(),
            build_mode: crate::target::NativeBuildMode::Console,
            project: "hello".to_string(),
            entry_symbol: Some("_mfb_fn_main".to_string()),
            runtime_symbols: Vec::new(),
            external_symbols: Vec::new(),
            platform_imports: Vec::new(),
            functions: vec![PlannedFunction {
                name: "main".to_string(),
                symbol: "_mfb_fn_main".to_string(),
                returns: StorageType {
                    name: "Nothing".to_string(),
                    class: StorageClass::Void,
                    size: 0,
                    align: 1,
                },
                params: Vec::new(),
                local_slots: Vec::new(),
                labels: Vec::new(),
                operations: vec!["ret".to_string()],
                calls: Vec::<PlanCall>::new(),
            }],
            link_symbols: Vec::new(),
        }
    }

    #[test]
    fn writes_native_object_plan_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_native_object_plan(dir.path(), "hello", &plan()).expect("write nobj");
        assert_eq!(path, dir.path().join("hello.nobj"));
        let json = std::fs::read_to_string(&path).unwrap();
        assert!(json.contains("\"container\": \"elf\""));
    }

    #[test]
    fn write_native_object_plan_propagates_lowering_error() {
        let mut plan = plan();
        plan.target = "windows".to_string();
        let dir = tempfile::tempdir().unwrap();
        assert!(write_native_object_plan(dir.path(), "hello", &plan).is_err());
    }

    #[test]
    fn validate_native_object_plan_accepts_and_rejects() {
        validate_native_object_plan(&plan()).expect("valid plan");
        let mut bad = plan();
        bad.target = "solaris".to_string();
        assert!(validate_native_object_plan(&bad).is_err());
    }

    #[test]
    fn writes_linked_executable_static_elf() {
        // A raw `ret` aarch64 program with no imports links to a static ELF; the
        // wrapper writes `<name>-<flavor>.out` and marks it executable.
        let image = EncodedImage {
            text: vec![0xc0, 0x03, 0x5f, 0xd6],
            data: Vec::new(),
            rodata_size: 0,
            symbols: vec![EncodedSymbol {
                name: "_main".to_string(),
                section: EncodedSection::Text,
                offset: 0,
            }],
            relocations: Vec::new(),
            imports: Vec::new(),
            entry: "_main".to_string(),
            initializers: Vec::new(),
            signing_metadata: None,
            rpaths: Vec::new(),
        };
        let dir = tempfile::tempdir().unwrap();
        let path =
            write_linked_executable(dir.path(), "prog", "aarch64", LinuxFlavor::Glibc, &image)
                .expect("write executable");
        // plan-46-D §4.1: the build emits into the project's `build/` directory.
        assert_eq!(path, dir.path().join("build").join("prog-glibc.out"));
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[0..4], b"\x7fELF");
    }

    /// plan-51-A §4.1: the Linux sibling of `os::macos::tests::
    /// writes_linked_app_bundle_layout` — every path the AppDir promises exists,
    /// through the same public wrapper the backends call.
    #[test]
    fn writes_linked_appdir_layout() {
        let image = EncodedImage {
            text: vec![0xc0, 0x03, 0x5f, 0xd6],
            data: Vec::new(),
            rodata_size: 0,
            symbols: vec![EncodedSymbol {
                name: "_main".to_string(),
                section: EncodedSection::Text,
                offset: 0,
            }],
            relocations: Vec::new(),
            imports: Vec::new(),
            entry: "_main".to_string(),
            initializers: Vec::new(),
            signing_metadata: None,
            rpaths: Vec::new(),
        };
        let dir = tempfile::tempdir().unwrap();
        let appdir = write_linked_appdir(
            dir.path(),
            "windowed",
            "aarch64",
            LinuxFlavor::Glibc,
            &image,
            None,
            "0.1.0",
        )
        .expect("appdir");
        assert_eq!(
            appdir,
            dir.path().join("build").join("windowed-glibc.AppDir")
        );
        assert!(appdir.join("usr/bin/windowed").is_file());
        assert!(appdir.join("windowed.desktop").is_file());
        assert!(appdir
            .join("usr/share/applications/windowed.desktop")
            .is_file());
        assert!(appdir.join("windowed.png").is_file());
        assert!(appdir
            .join("usr/share/icons/hicolor/256x256/apps/windowed.png")
            .is_file());

        // The runtime `execv`s `/AppRun`; it must resolve to the real ELF and
        // stay a symlink rather than becoming a second copy of it.
        assert_eq!(
            std::fs::read_link(appdir.join("AppRun")).unwrap(),
            Path::new("usr/bin/windowed")
        );
        assert_eq!(
            std::fs::read_link(appdir.join(".DirIcon")).unwrap(),
            Path::new("windowed.png")
        );
        assert_eq!(
            std::fs::read(appdir.join("usr/bin/windowed")).unwrap()[0..4],
            *b"\x7fELF"
        );
        // 0755, and no empty `usr/lib/` for a build that vendors nothing.
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(appdir.join("usr/bin/windowed"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o755
        );
        assert!(!appdir.join("usr/lib").exists());
    }
}
