// ---------------------------------------------------------------------------
// writer/mod.rs + sections/mod.rs — sections a package compiled from source
// carries only for some programs: a built-in resource type in its API (the
// resource table's standard rows) and a LINK block with locators (the native
// library table, section 10).
// ---------------------------------------------------------------------------

use super::*;

/// Build package `name` from `source` (and `extra` manifest fields); answer
/// the `.mfp` path, the directory kept alive beside it.
fn built(name: &str, source: &str, extra: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("project.json"),
        format!(
            "{{\"name\": \"{name}\", \"version\": \"1.0.0\", \"mfb\": \"1.0\", \"kind\": \
             \"package\", \"description\": \"d\", {extra}\"sources\": [{{ \"root\": \"src\", \
             \"role\": \"lib\", \"include\": [\"**/*.mfb\"] }}]}}"
        ),
    )
    .unwrap();
    std::fs::write(dir.path().join("src/lib.mfb"), source).unwrap();
    let options =
        crate::cli::build::parse_build_options(vec![dir.path().to_str().unwrap().to_string()])
            .unwrap();
    crate::cli::build::build_project(&options).expect("the package builds");
    let path = dir.path().join(format!("{name}.mfp"));
    (dir, path)
}

#[test]
fn a_built_in_resource_in_a_package_api_gets_a_standard_resource_row() {
    let (_dir, path) = built(
        "files",
        "IMPORT fs\nIMPORT udp\n\nEXPORT FUNC keep(f AS fs::File, s AS udp::Socket) AS Integer\n  RETURN 1\nEND FUNC\n",
        "",
    );
    let names: Vec<String> = read_package_resources(&path)
        .expect("resource table")
        .into_iter()
        .map(|resource| resource.type_name)
        .collect();
    // fs.File has its own row; any other built-in resource (udp.Socket) is
    // added as a standard "other" row.
    assert!(names.iter().any(|name| name.ends_with("File")), "{names:?}");
    assert!(
        names.iter().any(|name| name.ends_with("Socket")),
        "{names:?}"
    );
}

#[test]
fn a_link_block_with_locators_writes_the_native_library_table() {
    let (_dir, path) = built(
        "trig",
        "LINK \"libm\" AS libm\n  FUNC cosine(x AS Float) AS Float\n    SYMBOL \"cos\"\n    \
         ABI (x CDouble) AS r CDouble\n    RETURN r\n  END FUNC\nEND LINK\n\n\
         EXPORT FUNC c(x AS Float) AS Float\n  RETURN libm::cosine(x)\nEND FUNC\n",
        "\"libraries\": {\"libm\": [{\"os\": \"linux\", \"type\": \"system\", \"source\": \
         \"libm.so.6\"}, {\"os\": \"macos\", \"type\": \"system\", \"source\": \
         \"libSystem.B.dylib\"}, {\"os\": \"windows\", \"type\": \"system\", \"source\": \
         \"msvcrt.dll\"}]}, ",
    );
    let (package, table) = read_package_native_libraries(&path).expect("native library table");
    assert_eq!(package, "trig");
    let sources: Vec<&str> = table
        .entries
        .iter()
        .flat_map(|entry| entry.locators.iter().map(|locator| locator.source.as_str()))
        .collect();
    assert!(sources.contains(&"libm.so.6"), "{sources:?}");
    assert!(sources.contains(&"msvcrt.dll"), "{sources:?}");
}
