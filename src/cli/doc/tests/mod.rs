use super::*;
use tempfile::tempdir;

fn s(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| item.to_string()).collect()
}

const PROJECT_MANIFEST: &str = concat!(
    "{\n",
    "  \"name\": \"docapp\",\n",
    "  \"version\": \"0.1.0\",\n",
    "  \"mfb\": \"1.0\",\n",
    "  \"kind\": \"executable\",\n",
    "  \"sources\": [\n",
    "    { \"root\": \"src\", \"role\": \"main\", \"include\": [\"**/*.mfb\"] }\n",
    "  ],\n",
    "  \"entry\": \"main\",\n",
    "  \"targets\": [\"native\"]\n",
    "}\n"
);

#[test]
fn build_source_doc_page_single_file() {
    let dir = tempdir().unwrap();
    let file = dir.path().join("prog.mfb");
    fs::write(&file, "FUNC main() AS Nothing\nEND FUNC\n").unwrap();
    let (_, valid) = build_source_doc_page(&file).expect("page");
    assert!(valid);
}

#[test]
fn build_source_doc_page_unparseable_file_errors() {
    let dir = tempdir().unwrap();
    let file = dir.path().join("bad.mfb");
    fs::write(&file, "FUNC main( AS\n").unwrap();
    assert!(build_source_doc_page(&file).is_err());
}

#[test]
fn build_source_doc_page_missing_file_errors() {
    let dir = tempdir().unwrap();
    let missing = dir.path().join("nope.mfb");
    match build_source_doc_page(&missing) {
        Err(err) => assert!(err.contains("failed to read")),
        Ok(_) => panic!("missing file must error"),
    }
}

#[test]
fn build_source_doc_page_project_dir() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(dir.path().join("project.json"), PROJECT_MANIFEST).unwrap();
    fs::write(src.join("main.mfb"), "FUNC main() AS Nothing\nEND FUNC\n").unwrap();
    let (_, valid) = build_source_doc_page(dir.path()).expect("page");
    assert!(valid);
}

/// A project that imports a package declared by source directory documents
/// as valid even when `build/packages/` holds an unreadable `.mfp` for it
/// (an older compiler's output, say): `mfb build` rebuilds that cache before
/// resolving, so `mfb doc` must too.
#[test]
fn build_source_doc_page_rebuilds_a_stale_source_package() {
    let dir = tempdir().unwrap();
    let root = dir.path();
    let dep = root.join("packages").join("docdep");
    fs::create_dir_all(dep.join("src")).unwrap();
    fs::write(
        dep.join("project.json"),
        concat!(
            "{\n",
            "  \"name\": \"docdep\",\n",
            "  \"version\": \"0.1.0\",\n",
            "  \"mfb\": \"1.0\",\n",
            "  \"kind\": \"package\",\n",
            "  \"description\": \"mfb doc fixture package\",\n",
            "  \"sources\": [{ \"root\": \"src\", \"role\": \"lib\", \"include\": [\"**/*.mfb\"] }],\n",
            "  \"targets\": [\"native\"]\n",
            "}\n"
        ),
    )
    .unwrap();
    fs::write(
        dep.join("src").join("lib.mfb"),
        "EXPORT FUNC twice(n AS Integer) AS Integer\n  RETURN n * 2\nEND FUNC\n",
    )
    .unwrap();
    fs::write(
        root.join("project.json"),
        concat!(
            "{\n",
            "  \"name\": \"docapp\",\n",
            "  \"version\": \"0.1.0\",\n",
            "  \"mfb\": \"1.0\",\n",
            "  \"kind\": \"executable\",\n",
            "  \"sources\": [{ \"root\": \"src\", \"role\": \"main\", \"include\": [\"**/*.mfb\"] }],\n",
            "  \"packages\": [{ \"name\": \"docdep\", \"version\": \"=0.1.0\", \"source\": \"file:packages/docdep\", \"direct\": true, \"requiredBy\": [] }],\n",
            "  \"entry\": \"main\",\n",
            "  \"targets\": [\"native\"]\n",
            "}\n"
        ),
    )
    .unwrap();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("src").join("main.mfb"),
        "IMPORT docdep\n\nFUNC main() AS Nothing\n  LET n AS Integer = docdep::twice(2)\nEND FUNC\n",
    )
    .unwrap();
    let cache = root.join("build").join("packages");
    fs::create_dir_all(&cache).unwrap();
    fs::write(cache.join("docdep.mfp"), b"not a package").unwrap();

    let (_, valid) = build_source_doc_page(root).expect("page");
    assert!(valid, "a stale cached .mfp must be rebuilt, not reported");
}

#[test]
fn build_source_doc_page_bad_manifest_errors() {
    let dir = tempdir().unwrap();
    // A directory with no project.json fails validation.
    match build_source_doc_page(dir.path()) {
        Err(err) => assert!(err.contains("project validation failed")),
        Ok(_) => panic!("missing manifest must error"),
    }
}

#[test]
fn run_doc_command_writes_html_for_file() {
    let dir = tempdir().unwrap();
    let file = dir.path().join("prog.mfb");
    fs::write(&file, "FUNC main() AS Nothing\nEND FUNC\n").unwrap();
    let out = dir.path().join("out.html");
    let code = run_doc_command(&s(&[
        file.to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
    ]));
    assert_eq!(code, 0);
    assert!(out.is_file());
}

#[test]
fn run_doc_command_reports_build_error() {
    let dir = tempdir().unwrap();
    let file = dir.path().join("bad.mfb");
    fs::write(&file, "FUNC main( AS\n").unwrap();
    let out = dir.path().join("out.html");
    let code = run_doc_command(&s(&[
        file.to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
    ]));
    assert_eq!(code, 1);
}

#[test]
fn run_doc_command_out_requires_a_value() {
    assert_eq!(run_doc_command(&s(&["--out"])), 2);
}

#[test]
fn run_doc_command_rejects_unknown_flag() {
    assert_eq!(run_doc_command(&s(&["--bogus"])), 2);
}

#[test]
fn run_doc_command_rejects_two_paths() {
    assert_eq!(run_doc_command(&s(&["a.mfb", "b.mfb"])), 2);
}

#[test]
fn run_doc_command_write_failure_returns_one() {
    let dir = tempdir().unwrap();
    let file = dir.path().join("prog.mfb");
    fs::write(&file, "FUNC main() AS Nothing\nEND FUNC\n").unwrap();
    // Point --out at a path whose parent is not a directory.
    let bad_out = file.join("cannot").join("write.html");
    let code = run_doc_command(&s(&[
        file.to_str().unwrap(),
        "--out",
        bad_out.to_str().unwrap(),
    ]));
    assert_eq!(code, 1);
}

/// A DOC block that fails validation still writes the page, but the command
/// exits 1 (the diagnostics were already reported).
#[test]
fn run_doc_command_writes_the_page_and_exits_one_for_an_invalid_doc_block() {
    let dir = tempdir().unwrap();
    let file = dir.path().join("prog.mfb");
    fs::write(
        &file,
        "DOC\n  FUNC main\n  PROP p A property is not valid on a function.\nEND DOC\n\
         FUNC main() AS Nothing\nEND FUNC\n",
    )
    .unwrap();
    let out = dir.path().join("out.html");
    let (code, diagnostics) = crate::rules::collecting_diagnostics(|| {
        run_doc_command(&s(&[
            file.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
        ]))
    });
    assert_eq!(code, 1);
    let rules: Vec<&str> = diagnostics.iter().map(|(rule, _)| rule.as_str()).collect();
    assert_eq!(rules, vec!["DOC_PROP_INVALID_CONTEXT"]);
    assert!(out.is_file());
}
