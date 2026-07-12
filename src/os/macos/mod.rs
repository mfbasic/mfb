pub(crate) mod icon;
mod link;
mod object;

use crate::arch::aarch64::encode::EncodedImage;
use crate::target::shared::plan::NativePlan;
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) fn write_native_object_plan(
    project_dir: &Path,
    project_name: &str,
    native_plan: &NativePlan,
) -> Result<PathBuf, String> {
    let object_plan = object::lower_plan(native_plan)?;
    object_plan.validate()?;
    let object_path = project_dir.join(format!("{project_name}.nobj"));
    fs::write(&object_path, object_plan.to_json())
        .map_err(|err| format!("failed to write '{}': {err}", object_path.display()))?;
    Ok(object_path)
}

pub(crate) fn validate_native_object_plan(native_plan: &NativePlan) -> Result<(), String> {
    let object_plan = object::lower_plan(native_plan)?;
    object_plan.validate()
}

pub(crate) fn write_linked_executable(
    project_dir: &Path,
    project_name: &str,
    image: &EncodedImage,
) -> Result<PathBuf, String> {
    link::write_executable(project_dir, project_name, image)
}

/// Link `image` and write it as a macOS app-mode `.app` bundle (Info.plist +
/// `Contents/MacOS/<name>` + `Contents/Resources/AppIcon.icns`), returning the
/// path to the `.app` directory. `app_icon` is the resolved project `icon` source
/// (plan-22-A); `None` uses the compiler's embedded default icon (plan-22-B).
pub(crate) fn write_linked_app_bundle(
    project_dir: &Path,
    project_name: &str,
    image: &EncodedImage,
    app_icon: Option<&Path>,
) -> Result<PathBuf, String> {
    link::write_app_bundle(project_dir, project_name, image, app_icon)
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
            target: "macos-aarch64".to_string(),
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

    fn image() -> EncodedImage {
        EncodedImage {
            text: vec![0xc0, 0x03, 0x5f, 0xd6],
            data: Vec::new(),
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
        }
    }

    #[test]
    fn writes_native_object_plan_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_native_object_plan(dir.path(), "hello", &plan()).expect("write nobj");
        assert_eq!(path, dir.path().join("hello.nobj"));
        let json = std::fs::read_to_string(&path).unwrap();
        assert!(json.contains("\"container\": \"mach-o\""));
    }

    #[test]
    fn write_native_object_plan_propagates_lowering_error() {
        let mut plan = plan();
        plan.target = "linux-aarch64".to_string();
        let dir = tempfile::tempdir().unwrap();
        assert!(write_native_object_plan(dir.path(), "hello", &plan).is_err());
    }

    #[test]
    fn validate_native_object_plan_accepts_and_rejects() {
        validate_native_object_plan(&plan()).expect("valid plan");
        let mut bad = plan();
        bad.target = "linux-aarch64".to_string();
        assert!(validate_native_object_plan(&bad).is_err());
    }

    #[test]
    fn writes_linked_executable_mach_o() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_linked_executable(dir.path(), "prog", &image()).expect("write exe");
        assert_eq!(path, dir.path().join("prog.out"));
        let bytes = std::fs::read(&path).unwrap();
        // Mach-O 64 magic (little-endian 0xfeedfacf).
        assert_eq!(&bytes[0..4], &[0xcf, 0xfa, 0xed, 0xfe]);
    }

    #[test]
    fn writes_linked_app_bundle_layout() {
        let dir = tempfile::tempdir().unwrap();
        let bundle =
            write_linked_app_bundle(dir.path(), "windowed", &image(), None).expect("bundle");
        assert_eq!(bundle, dir.path().join("windowed.app"));
        assert!(bundle.join("Contents/MacOS/windowed").is_file());
        assert!(bundle.join("Contents/Info.plist").is_file());
        assert!(bundle.join("Contents/Resources/AppIcon.icns").is_file());
    }
}
