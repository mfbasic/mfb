//! What an importer reads from packages compiled from MFBASIC source by the
//! real build: exported globals, a computed parameter default, the type names
//! a package exports, and a re-exported foreign type whose owner's ABI moved
//! (bug-390). Undecodable and undeclared packages are read lossily.

use super::*;
use crate::manifest::parse_project_json;

const GEO_MANIFEST: &str = "{\n  \"name\": \"geo\",\n  \"version\": \"1.0.0\",\n  \
    \"mfb\": \"1.0\",\n  \"kind\": \"package\",\n  \"description\": \"geo\",\n  \
    \"sources\": [{ \"root\": \"src\", \"role\": \"lib\", \"include\": [\"**/*.mfb\"] }]\n}\n";

/// `geo`: an exported record (with one more field in `v2`), two exported
/// globals and a private one, and a function whose default is computed.
fn geo_source(v2: bool) -> String {
    format!(
        "EXPORT TYPE Point\n  x AS Float\n{}END TYPE\n\n\
         EXPORT LET origin AS Point = Point[0.0{}]\n\
         EXPORT MUT count AS Integer = 0\n\
         PUBLIC LET shared AS Integer = 1\n\
         LET hidden AS Integer = 3\n\n\
         FUNC two() AS Integer\n  RETURN 2\nEND FUNC\n\n\
         EXPORT FUNC scaled(p AS Point, k AS Integer = two()) AS Point\n  \
         RETURN WITH p {{ x := p.x * toFloat(k + hidden) }}\nEND FUNC\n",
        if v2 { "  y AS Float\n" } else { "" },
        if v2 { ", 0.0" } else { "" },
    )
}

/// `<name>`: re-exports `geo::Point` through its own function.
fn mid_manifest(name: &str) -> String {
    format!(
        "{{\n  \"name\": \"{name}\",\n  \"version\": \"1.0.0\",\n  \"mfb\": \"1.0\",\n  \
         \"kind\": \"package\",\n  \"description\": \"{name}\",\n  \"packages\": [{{\"name\": \
         \"geo\", \"version\": \"1.0.0\", \"source\": \"file:../geo\", \"direct\": true, \
         \"requiredBy\": []}}],\n  \"sources\": [{{ \"root\": \"src\", \"role\": \"lib\", \
         \"include\": [\"**/*.mfb\"] }}]\n}}\n"
    )
}

const MID_SOURCE: &str = "IMPORT geo\n\nEXPORT FUNC shift(p AS geo::Point) AS geo::Point\n  \
    RETURN geo::scaled(p)\nEND FUNC\n";

fn write_project(dir: &Path, manifest: &str, source: &str) {
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(dir.join("project.json"), manifest).unwrap();
    fs::write(dir.join("src/lib.mfb"), source).unwrap();
}

fn build(dir: &Path) {
    let options =
        crate::cli::build::parse_build_options(vec![dir.to_str().unwrap().to_string()]).unwrap();
    crate::cli::build::build_project(&options).expect("the package builds");
}

/// Under `root/<version>/`: `geo/geo.mfp` and `<mid>/<mid>.mfp` built
/// against it. Answers the two files.
fn built(root: &Path, version: &str, mid: &str) -> (PathBuf, PathBuf) {
    let at = root.join(version);
    write_project(&at.join("geo"), GEO_MANIFEST, &geo_source(version == "v2"));
    write_project(&at.join(mid), &mid_manifest(mid), MID_SOURCE);
    build(&at.join("geo"));
    build(&at.join(mid));
    (
        at.join("geo/geo.mfp"),
        at.join(mid).join(format!("{mid}.mfp")),
    )
}

#[test]
fn an_importer_reads_exported_globals_defaults_and_type_names() {
    let root = tempfile::tempdir().unwrap();
    let (geo, _) = built(root.path(), "v1", "mid");
    let globals = imported_global_defs_from_files(std::slice::from_ref(&geo));
    assert_eq!(
        globals
            .iter()
            .map(|global| (
                global.name.as_str(),
                global.type_.to_string(),
                global.mutable
            ))
            .collect::<Vec<_>>(),
        [
            ("geo.origin", "geo.Point".to_string(), false),
            ("geo.count", "Integer".to_string(), true),
        ]
    );
    let signatures =
        external_package_function_types_from_files(std::slice::from_ref(&geo)).unwrap();
    let scaled = &signatures["geo.scaled"];
    assert_eq!(scaled.params[0].default, crate::ir::ExternalDefault::None);
    assert!(
        matches!(&scaled.params[1].default,
            crate::ir::ExternalDefault::Function(name) if name.starts_with("geo.")),
        "{:?}",
        scaled.params[1].default
    );
    // A function is no type name; the record is.
    let app = root.path().join("app");
    fs::create_dir_all(app.join("packages")).unwrap();
    fs::copy(&geo, app.join("packages/geo.mfp")).unwrap();
    let manifest = parse_project_json(
        "{\"name\": \"app\", \"version\": \"0.1.0\", \"mfb\": \"1.0\", \"packages\": \
         [{\"name\": \"geo\", \"version\": \"1.0.0\"}]}",
        &app.join("project.json"),
    )
    .unwrap();
    let names = imported_type_names(&app, &manifest);
    assert_eq!(
        names["geo"],
        ["Point".to_string()]
            .into_iter()
            .collect::<std::collections::HashSet<_>>()
    );
}

#[test]
fn a_re_exported_type_built_against_another_owner_abi_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let (_, mid) = built(root.path(), "v1", "mid");
    let (geo2, mid2) = built(root.path(), "v2", "mid2");
    // `mid` was built against geo 1; geo 2 installed beside it changed Point.
    let installed = root.path().join("installed");
    fs::create_dir_all(&installed).unwrap();
    fs::copy(&mid, installed.join("mid.mfp")).unwrap();
    fs::copy(&geo2, installed.join("geo.mfp")).unwrap();
    assert_eq!(
        verify_foreign_type_abi_consistency(&[installed.join("mid.mfp")]),
        Err(
            "type `Point` re-exported by an imported package was built against an \
             ABI-incompatible version of `geo`"
                .to_string()
        )
    );
    // Built against the geo installed beside it, and agreeing with another
    // intermediary built against the same geo, it is accepted.
    let (geo1, mid3) = built(root.path(), "v1-again", "mid3");
    fs::copy(&geo1, installed.join("geo.mfp")).unwrap();
    fs::copy(&mid3, installed.join("mid3.mfp")).unwrap();
    assert_eq!(
        verify_foreign_type_abi_consistency(&[
            installed.join("mid.mfp"),
            installed.join("mid3.mfp")
        ]),
        Ok(())
    );
    // Two intermediaries built against different geo versions disagree.
    assert_eq!(
        verify_foreign_type_abi_consistency(&[mid, mid2]),
        Err(
            "imported packages disagree on the ABI of type `Point` from `geo`: they were \
             built against incompatible versions of `geo`"
                .to_string()
        )
    );
}

#[test]
fn unreadable_or_undeclared_packages_contribute_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let packages = dir.path().join("packages");
    fs::create_dir_all(&packages).unwrap();
    // A sound header over a payload that is no binary representation.
    let junk = crate::target::package_mfp::build_package_bytes(
        &crate::binary_repr::BinaryReprMetadata::new("junk".to_string(), "1.0.0".to_string()),
        b"MFPC and nothing after it",
        None,
    )
    .unwrap();
    fs::write(packages.join("junk.mfp"), junk).unwrap();
    let files = [packages.join("junk.mfp")];
    assert!(imported_global_defs_from_files(&files).is_empty());
    assert!(imported_type_defs_from_files(&files).is_empty());
    let declared = parse_project_json(
        "{\"name\": \"app\", \"version\": \"0.1.0\", \"mfb\": \"1.0\", \"packages\": \
         [{\"name\": \"junk\", \"version\": \"1.0.0\"}]}",
        &dir.path().join("project.json"),
    )
    .unwrap();
    assert!(!imported_type_names(dir.path(), &declared).contains_key("junk"));
    assert!(external_package_function_types(dir.path(), &declared).is_empty());
    // A declared package that is not installed: nothing to read at all.
    let missing = parse_project_json(
        "{\"name\": \"app\", \"version\": \"0.1.0\", \"mfb\": \"1.0\", \"packages\": \
         [{\"name\": \"gone\", \"version\": \"1.0.0\"}]}",
        &dir.path().join("project.json"),
    )
    .unwrap();
    assert!(external_package_function_types(dir.path(), &missing).is_empty());
    assert!(imported_type_defs(dir.path(), &missing).is_empty());
    assert!(imported_global_defs(dir.path(), &missing).is_empty());
    assert!(imported_type_names(dir.path(), &missing).is_empty());
}
