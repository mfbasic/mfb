//! `mfb pkg remove` over installed packages with real import tables: the
//! cascade to every package importing the target, a lone package, and an
//! installed file whose import table cannot be read.

use super::*;
use crate::binary_repr::{BinaryReprDependency, BinaryReprMetadata};
use crate::ir::{IrFunction, IrOp, IrProject, IrSourceLoc, IrValue};
use crate::types::ParameterType;

fn function(name: &str, calls: Option<&str>) -> IrFunction {
    let body = calls
        .map(|target| {
            vec![IrOp::Return {
                value: Some(IrValue::Call {
                    target: target.to_string(),
                    args: vec![],
                    loc: IrSourceLoc::default(),
                    type_: ParameterType::parse("Integer"),
                }),
                loc: IrSourceLoc::default(),
            }]
        })
        .unwrap_or_default();
    IrFunction {
        name: name.to_string(),
        visibility: "export".to_string(),
        kind: "function".to_string(),
        isolated: false,
        params: vec![],
        returns: ParameterType::parse("Integer"),
        body,
        file: "src/main.mfb".to_string(),
        resource_owners: std::collections::HashMap::new(),
        loc: IrSourceLoc::default(),
    }
}

/// Compile `<owner>#<name>` into `dir/<name>.mfp`, importing `imports`
/// (each `(name, ident, compiled file)`) and calling each one's `helper`.
pub(super) fn package(dir: &Path, ident: &str, imports: &[(&str, &str, &Path)]) -> PathBuf {
    let name = ident.split('#').nth(1).unwrap();
    std::fs::create_dir_all(dir).unwrap();
    let mut functions = vec![function("helper", None)];
    for (index, (import, _, _)) in imports.iter().enumerate() {
        functions.push(function(
            &format!("use{index}"),
            Some(&format!("{import}.helper")),
        ));
    }
    let ir = IrProject {
        name: name.to_string(),
        entry: None,
        bindings: vec![],
        types: vec![],
        functions,
        native_resources: vec![],
        link_functions: vec![],
        link_cstructs: Vec::new(),
        link_aliases: vec![],
        docs: crate::ir::ProjectDocs::default(),
        native_libraries: Default::default(),
        max_buffer_bytes: crate::manifest::DEFAULT_MAX_BUFFER_MIB * 1024 * 1024,
        canvas_auto_paint_ms: crate::manifest::DEFAULT_CANVAS_AUTO_PAINT_MS,
        arena_memory: crate::ir::ArenaMemoryLimits::default(),
        selected_config: Default::default(),
    };
    let mut metadata = BinaryReprMetadata::new(name.to_string(), "1.0.0".to_string());
    metadata.ident = ident.to_string();
    metadata.dependencies = imports
        .iter()
        .map(|(name, ident, _)| BinaryReprDependency {
            name: name.to_string(),
            ident: ident.to_string(),
            version: "1.0.0".to_string(),
            pin: false,
            flags: 0,
        })
        .collect();
    let files: Vec<PathBuf> = imports
        .iter()
        .map(|(_, _, file)| file.to_path_buf())
        .collect();
    crate::target::package_mfp::write_package(dir, &ir, &metadata, &files, None).unwrap()
}

fn entry(name: &str, ident: &str, required_by: &str) -> String {
    format!(
        "{{\"name\": \"{name}\", \"ident\": \"{ident}\", \"version\": \"1.0.0\", \
         \"source\": \"{ident}\", \"direct\": true, \"requiredBy\": [{required_by}]}}"
    )
}

pub(super) fn project(dir: &Path, entries: &[String]) {
    std::fs::write(
        dir.join("project.json"),
        format!(
            "{{\n  \"name\": \"app\",\n  \"version\": \"0.1.0\",\n  \"mfb\": \"1.0\",\n  \
             \"sources\": [{{\"root\": \"src\"}}],\n  \"packages\": [{}]\n}}\n",
            entries.join(", ")
        ),
    )
    .unwrap();
}

fn declared(dir: &Path) -> Vec<String> {
    let path = dir.join("project.json");
    let manifest = parse_project_json(&std::fs::read_to_string(&path).unwrap(), &path).unwrap();
    crate::manifest::closure::declared_dependencies(&manifest)
        .into_iter()
        .map(|dependency| dependency.ident)
        .collect()
}

#[test]
fn removing_a_package_removes_every_package_that_imports_it() {
    let dir = tempfile::tempdir().unwrap();
    let packages = dir.path().join("packages");
    let base = package(&packages, "ada#base", &[]);
    package(&packages, "ada#user", &[("base", "ada#base", &base)]);
    let vendor = crate::manifest::libraries::imported_vendor_dir(dir.path(), "base");
    std::fs::create_dir_all(&vendor).unwrap();
    std::fs::write(vendor.join("libbase.so"), b"blob").unwrap();
    project(
        dir.path(),
        &[
            entry("base", "ada#base", "\"ada#user\""),
            entry("user", "ada#user", ""),
        ],
    );
    run_remove(dir.path(), "ada#base", true).unwrap();
    assert!(declared(dir.path()).is_empty());
    assert!(!packages.join("base.mfp").exists() && !packages.join("user.mfp").exists());
    assert!(!vendor.exists());
}

#[test]
fn a_package_nothing_imports_is_removed_alone() {
    let dir = tempfile::tempdir().unwrap();
    let packages = dir.path().join("packages");
    package(&packages, "ada#solo", &[]);
    project(dir.path(), &[entry("solo", "ada#solo", "")]);
    run_remove(dir.path(), "ada#solo", false).unwrap();
    assert!(declared(dir.path()).is_empty());
    assert!(!packages.join("solo.mfp").exists());
}

#[test]
fn an_installed_package_whose_import_table_cannot_be_read_stops_the_removal() {
    let dir = tempfile::tempdir().unwrap();
    let packages = dir.path().join("packages");
    std::fs::create_dir_all(&packages).unwrap();
    std::fs::write(packages.join("solo.mfp"), b"not a package").unwrap();
    project(dir.path(), &[entry("solo", "ada#solo", "")]);
    assert!(run_remove(dir.path(), "ada#solo", true).is_err());
    assert_eq!(declared(dir.path()), ["ada#solo"]);
    assert!(packages.join("solo.mfp").exists());
}

/// A package file or vendor directory that cannot be deleted is warned
/// about and left: the manifest change has already been made.
#[test]
#[cfg(unix)]
fn installed_files_that_cannot_be_deleted_are_left_with_a_warning() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let packages = dir.path().join("packages");
    let vendor = crate::manifest::libraries::imported_vendor_dir(dir.path(), "base");
    std::fs::create_dir_all(&vendor).unwrap();
    std::fs::write(vendor.join("libbase.so"), b"blob").unwrap();
    std::fs::write(packages.join("base.mfp"), b"mfp").unwrap();
    // Probe: root deletes from a read-only directory anyway.
    let mode = |path: &Path, mode| {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap()
    };
    mode(&packages, 0o555);
    let probe = std::fs::remove_file(packages.join("base.mfp")).is_ok();
    if probe {
        mode(&packages, 0o755);
        return;
    }
    remove_installed_package_files(dir.path(), "base");
    mode(&packages, 0o755);
    assert!(packages.join("base.mfp").exists());
    assert!(vendor.exists());
    // A package with nothing installed has nothing to delete.
    remove_installed_package_files(dir.path(), "never-installed");
}

/// A project with no `project.json` is refused before anything is read or
/// removed, naming the manifest path.
#[test]
fn removing_from_a_directory_without_a_manifest_names_it() {
    let dir = tempfile::tempdir().unwrap();
    let manifest = dir.path().join("project.json");
    let err = run_remove(dir.path(), "ada#solo", true).unwrap_err();
    assert!(
        err.starts_with(&format!("failed to read '{}': ", manifest.display())),
        "{err}"
    );
}
