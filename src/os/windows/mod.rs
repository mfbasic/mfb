//! Windows PE/COFF container writer (plan-47-C) — the third sibling of
//! `src/os/{linux,macos}/`, emitting a PE32+ console `.exe` from the same
//! [`crate::arch::image::EncodedImage`] the ELF and Mach-O writers
//! consume.

mod link;
mod object;

use crate::target::shared::plan::NativePlan;
use std::fs;
use std::path::{Path, PathBuf};

/// Lower `plan` to a `container:"pe"` object plan and write it as `<name>.nobj`
/// (the `-nobj` artifact). Mirrors `crate::os::linux::write_native_object_plan`.
pub(crate) fn write_native_object_plan(
    project_dir: &Path,
    project_name: &str,
    plan: &NativePlan,
) -> Result<PathBuf, String> {
    crate::os::validate_output_name(project_name)?;
    let object_plan = object::lower_plan(plan)?;
    object_plan.validate()?;
    let object_path = project_dir.join(format!("{project_name}.nobj"));
    fs::write(&object_path, object_plan.to_json())
        .map_err(|err| format!("failed to write '{}': {err}", object_path.display()))?;
    Ok(object_path)
}

/// Validate that `plan` lowers to a well-formed PE object plan, without writing
/// anything. Mirrors `crate::os::linux::validate_native_object_plan`.
pub(crate) fn validate_native_object_plan(plan: &NativePlan) -> Result<(), String> {
    object::lower_plan(plan)?.validate()
}

/// Link `image` into a PE32+ `.exe` and write it as `build/<name>.exe` (plan-47-D).
/// One file, no flavor suffix — the Windows sibling of
/// `crate::os::linux::write_linked_executable`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn write_linked_executable(
    project_dir: &Path,
    project_name: &str,
    image: &crate::arch::image::EncodedImage,
    // plan-66-I: app mode links a GUI-subsystem PE (Subsystem=2) instead of the
    // console subsystem (3).
    gui: bool,
    // plan-66-K: the app icon + version, packaged into a `.rsrc` resource section.
    app_icon: Option<&Path>,
    app_version: Option<&str>,
) -> Result<PathBuf, String> {
    crate::os::validate_output_name(project_name)?;
    let bytes = link::write_executable(image, gui, app_icon, app_version)?;
    let build_dir = project_dir.join(crate::os::BUILD_DIR);
    fs::create_dir_all(&build_dir)
        .map_err(|err| format!("failed to create '{}': {err}", build_dir.display()))?;
    let exe_path = build_dir.join(format!("{project_name}.exe"));
    fs::write(&exe_path, &bytes)
        .map_err(|err| format!("failed to write '{}': {err}", exe_path.display()))?;
    Ok(exe_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arch::image::{EncodedImage, EncodedSection, EncodedSymbol};
    use crate::target::shared::plan::{
        NativePlan, PlanCall, PlannedFunction, StorageClass, StorageType,
    };

    fn plan(target: &str) -> NativePlan {
        NativePlan {
            target: target.to_string(),
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

    fn ret_image() -> EncodedImage {
        EncodedImage {
            text: vec![0xc3], // ret
            data: Vec::new(),
            rodata_size: 0,
            symbols: vec![EncodedSymbol {
                name: "_start".to_string(),
                section: EncodedSection::Text,
                offset: 0,
            }],
            relocations: Vec::new(),
            imports: Vec::new(),
            entry: "_start".to_string(),
            initializers: Vec::new(),
            signing_metadata: None,
            rpaths: Vec::new(),
        }
    }

    #[test]
    fn writes_native_object_plan_file() {
        let dir = tempfile::tempdir().unwrap();
        let path =
            write_native_object_plan(dir.path(), "hello", &plan("windows-x86_64")).expect("write");
        assert_eq!(path, dir.path().join("hello.nobj"));
        let written = std::fs::read_to_string(&path).unwrap();
        let expected = object::lower_plan(&plan("windows-x86_64"))
            .unwrap()
            .to_json();
        assert_eq!(written, expected);
        assert!(written.contains("\"container\": \"pe\""));
    }

    #[test]
    fn write_native_object_plan_propagates_a_lowering_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(write_native_object_plan(dir.path(), "hello", &plan("linux-x86_64")).is_err());
    }

    #[test]
    fn validate_native_object_plan_accepts_and_rejects() {
        validate_native_object_plan(&plan("windows-x86_64")).expect("valid plan");
        assert!(validate_native_object_plan(&plan("linux-x86_64")).is_err());
    }

    #[test]
    fn writes_linked_console_and_gui_executables() {
        let dir = tempfile::tempdir().unwrap();
        let console = write_linked_executable(dir.path(), "prog", &ret_image(), false, None, None)
            .expect("console exe");
        assert_eq!(console, dir.path().join("build").join("prog.exe"));
        assert_eq!(&std::fs::read(&console).unwrap()[0..2], b"MZ");

        let gui = write_linked_executable(dir.path(), "windowed", &ret_image(), true, None, None)
            .expect("gui exe");
        assert_eq!(gui, dir.path().join("build").join("windowed.exe"));
        assert_eq!(&std::fs::read(&gui).unwrap()[0..2], b"MZ");
    }

    /// bug-503: the project name is `Path::join`ed into `build/<name>.exe` (and
    /// `<name>.nobj`), so a name carrying a path separator or `..` must be
    /// refused before any byte is written — never resolved to a path outside
    /// `build/`.
    #[test]
    fn refuses_to_write_artifacts_under_a_traversing_name() {
        let dir = tempfile::tempdir().unwrap();
        let error = write_linked_executable(dir.path(), "../evil", &ret_image(), false, None, None)
            .expect_err("a traversing project name must be rejected");
        assert!(error.contains("not a valid path component"), "{error}");
        assert!(
            !dir.path().join("evil.exe").exists(),
            "an executable escaped build/ under a traversing name"
        );
        assert!(
            !dir.path().join("build").exists(),
            "nothing may be created on refusal"
        );
        let error = write_native_object_plan(dir.path(), "../evil", &plan("windows-x86_64"))
            .expect_err("a traversing project name must be rejected");
        assert!(error.contains("not a valid path component"), "{error}");
        assert!(!dir.path().parent().unwrap().join("evil.nobj").exists());
        for name in [".hidden", "/tmp/evil", "a\\b", "a/b"] {
            assert!(
                write_linked_executable(dir.path(), name, &ret_image(), false, None, None).is_err(),
                "{name}"
            );
        }
    }

    #[test]
    fn object_plan_reports_an_unwritable_destination() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("not-a-directory");
        std::fs::write(&file, b"occupied").unwrap();
        let error = write_native_object_plan(&file, "hello", &plan("windows-x86_64"))
            .expect_err("a file cannot contain an object plan");
        assert!(error.contains("failed to write"));
    }

    #[test]
    fn linked_executable_reports_an_uncreatable_build_directory() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("not-a-directory");
        std::fs::write(&file, b"occupied").unwrap();
        let error = write_linked_executable(&file, "prog", &ret_image(), false, None, None)
            .expect_err("a file cannot contain a build directory");
        assert!(error.contains("failed to create"));
    }

    #[test]
    fn linked_executable_reports_an_unwritable_output_path() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("build").join("prog.exe");
        std::fs::create_dir_all(&output).unwrap();
        let error = write_linked_executable(dir.path(), "prog", &ret_image(), false, None, None)
            .expect_err("a directory cannot be overwritten by an executable");
        assert!(error.contains("failed to write"));
    }
}
