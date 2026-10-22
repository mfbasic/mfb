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

/// A thread handle's resource plane is part of an export's ABI hash: the same
/// function over a thread type without it hashes differently.
#[test]
fn a_thread_type_s_resource_plane_is_part_of_the_abi_hash() {
    let hash = |thread: &str| {
        let (_dir, path) = built(
            "threads",
            &format!(
                "IMPORT fs\nIMPORT thread\n\nEXPORT FUNC watch(t AS {thread}) AS Integer\n  \
                 RETURN 1\nEND FUNC\n"
            ),
            "",
        );
        read_package_info(&path)
            .expect("info")
            .exports
            .into_iter()
            .find(|export| export.name == "watch")
            .expect("watch is exported")
            .sig_hash
    };
    assert_ne!(
        hash("Thread OF Integer RES fs::File TO Integer"),
        hash("Thread OF Integer TO Integer")
    );
}

/// A composite type used twice is one type-table entry: a function type, a
/// thread and a thread-worker handle, a map entry.
#[test]
fn a_composite_type_used_twice_is_one_type_table_entry() {
    let (_dir, path) = built(
        "twice",
        "IMPORT thread\n\nEXPORT FUNC rep(a AS FUNC(Integer) AS Integer, \
         b AS FUNC(Integer) AS Integer, t AS Thread OF Integer TO Integer, \
         u AS Thread OF Integer TO Integer, w AS ThreadWorker OF Integer TO Integer, \
         x AS ThreadWorker OF Integer TO Integer, e AS MapEntry OF String TO Integer, \
         f AS MapEntry OF String TO Integer) AS Integer\n  RETURN 1\nEND FUNC\n",
        "",
    );
    let package = read_package_binary_repr(&path).expect("decodes");
    let names = type_entry_names(&package.project.types, &package.project.strings.values)
        .expect("type names");
    let mut kinds: std::collections::BTreeMap<u16, usize> = std::collections::BTreeMap::new();
    for entry in &package.project.types.entries {
        *kinds.entry(entry.kind).or_default() += 1;
    }
    // Each composite named twice in the signature (a FUNC type, a Thread, a
    // ThreadWorker, a MapEntry) is written once: one entry of each kind.
    assert_eq!(names.len(), 4, "{names:?}");
    assert_eq!(kinds.len(), 4, "{kinds:?}");
    assert!(
        kinds.values().all(|count| *count == 1),
        "{kinds:?} {names:?}"
    );
}
