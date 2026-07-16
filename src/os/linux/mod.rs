pub(crate) mod flavor;
mod link;
mod object;

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
    app_mode: bool,
    image: &EncodedImage,
) -> Result<PathBuf, String> {
    link::write_executable(project_dir, project_name, arch, flavor, app_mode, image)
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
        let path = write_linked_executable(
            dir.path(),
            "prog",
            "aarch64",
            LinuxFlavor::Glibc,
            false,
            &image,
        )
        .expect("write executable");
        // plan-46-D §4.1: the build emits into the project's `build/` directory.
        assert_eq!(path, dir.path().join("build").join("prog-glibc.out"));
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[0..4], b"\x7fELF");
    }
}
