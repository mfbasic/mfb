use super::*;

mod compiled;
use std::fs;

fn write(path: &Path, contents: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

fn manifest(packages: &str) -> String {
    format!(
        "{{\n  \"name\": \"p\",\n  \"version\": \"0.1.0\",\n  \"mfb\": \"1.0\",\n  \
         \"kind\": \"package\",\n  \"packages\": [{packages}]\n}}\n"
    )
}

/// root/base (no deps), root/user (declares base), root/app (declares user).
fn fixture(app_packages: &str) -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    write(&root.path().join("base/project.json"), &manifest(""));
    write(
        &root.path().join("user/project.json"),
        &manifest(
            "{\"name\": \"base\", \"version\": \"=0.1.0\", \"source\": \"file:../base\", \
             \"direct\": true, \"requiredBy\": []}",
        ),
    );
    write(
        &root.path().join("app/project.json"),
        &manifest(app_packages),
    );
    root
}

fn check_app(root: &Path) -> Vec<Issue> {
    let app = root.join("app");
    let contents = fs::read_to_string(app.join("project.json")).unwrap();
    let manifest = parse_project_json(&contents, &app.join("project.json")).unwrap();
    check(&app, &manifest).unwrap()
}

const USER_ONLY: &str = "{\"name\": \"user\", \"version\": \"=0.1.0\", \
                         \"source\": \"file:../user\", \"direct\": true, \"requiredBy\": []}";

#[test]
fn an_undeclared_dependency_of_a_declared_package_is_an_issue() {
    let root = fixture(USER_ONLY);
    assert_eq!(
        check_app(root.path()),
        vec![Issue::Undeclared {
            requirer: "user".to_string(),
            name: "base".to_string()
        }]
    );
}

#[test]
fn a_closed_manifest_has_no_issues() {
    let root = fixture(&format!(
        "{USER_ONLY}, {{\"name\": \"base\", \"version\": \"=0.1.0\", \
         \"source\": \"file:../base\", \"direct\": false, \"requiredBy\": [\"user\"]}}"
    ));
    assert_eq!(check_app(root.path()), Vec::<Issue>::new());
}

#[test]
fn missing_stale_and_orphaned_fields_are_each_reported() {
    let root = fixture(
        "{\"name\": \"user\", \"version\": \"=0.1.0\", \"source\": \"file:../user\", \
         \"requiredBy\": []}, \
         {\"name\": \"base\", \"version\": \"=0.1.0\", \"source\": \"file:../base\", \
         \"direct\": false, \"requiredBy\": [\"nobody\"]}, \
         {\"name\": \"extra\", \"version\": \"=0.1.0\", \"source\": \"file:../base\", \
         \"direct\": false}",
    );
    assert_eq!(
        check_app(root.path()),
        vec![
            Issue::MissingDirect {
                name: "user".to_string()
            },
            Issue::RequiredBy {
                name: "base".to_string(),
                declared: vec!["nobody".to_string()],
                expected: vec!["user".to_string()],
            },
            Issue::MissingRequiredBy {
                name: "extra".to_string(),
                expected: vec![],
            },
            Issue::Orphan {
                name: "extra".to_string()
            },
        ]
    );
}

#[test]
fn reconcile_declares_the_closure_with_a_rebased_source() {
    let root = fixture(
        "\n    {\n      \"name\": \"user\",\n      \"version\": \"=0.1.0\",\n      \
         \"source\": \"file:../user\"\n    }\n  ",
    );
    let app = root.path().join("app");
    let contents = fs::read_to_string(app.join("project.json")).unwrap();
    let result = reconcile(&app, &contents).unwrap();
    assert_eq!(result.added, vec![("base".to_string(), "user".to_string())]);
    assert!(result.unresolved.is_empty());
    let manifest = parse_project_json(&result.contents, &app.join("project.json")).unwrap();
    let declared = declared_dependencies(&manifest);
    let user = declared.iter().find(|d| d.name == "user").unwrap();
    assert_eq!(user.direct, Some(true));
    assert_eq!(user.required_by, Some(vec![]));
    let base = declared.iter().find(|d| d.name == "base").unwrap();
    assert_eq!(base.direct, Some(false));
    assert_eq!(base.required_by, Some(vec!["user".to_string()]));
    assert_eq!(base.source, "file:../base");
    // Reconciling a closed manifest changes nothing.
    let again = reconcile(&app, &result.contents).unwrap();
    assert_eq!(again.contents, result.contents);
    assert!(again.added.is_empty() && again.removed.is_empty());
}

#[test]
fn reconcile_drops_orphans_transitively_and_keeps_direct_entries() {
    // `mid` (indirect) requires `base` (indirect); nothing requires `mid`.
    let root = fixture("");
    write(
        &root.path().join("mid/project.json"),
        &manifest(
            "{\"name\": \"base\", \"version\": \"=0.1.0\", \"source\": \"file:../base\", \
             \"direct\": true, \"requiredBy\": []}",
        ),
    );
    let app = root.path().join("app");
    let contents = manifest(
        "{\"name\": \"mid\", \"version\": \"=0.1.0\", \"source\": \"file:../mid\", \
         \"direct\": false, \"requiredBy\": []}, \
         {\"name\": \"base\", \"version\": \"=0.1.0\", \"source\": \"file:../base\", \
         \"direct\": false, \"requiredBy\": [\"mid\"]}, \
         {\"name\": \"user\", \"version\": \"=0.1.0\", \"source\": \"file:../user\", \
         \"direct\": true, \"requiredBy\": []}",
    );
    let result = reconcile(&app, &contents).unwrap();
    assert_eq!(result.removed, vec!["mid".to_string()]);
    // `base` is still required, by `user`.
    let manifest = parse_project_json(&result.contents, &app.join("project.json")).unwrap();
    let names: Vec<String> = declared_dependencies(&manifest)
        .into_iter()
        .map(|d| format!("{}:{:?}", d.name, d.required_by))
        .collect();
    assert_eq!(
        names,
        vec![
            "base:Some([\"user\"])".to_string(),
            "user:Some([])".to_string()
        ]
    );
}

#[test]
fn a_requirers_source_is_rebased_onto_the_importer() {
    let root = tempfile::tempdir().unwrap();
    let app = root.path().join("app");
    let user = root.path().join("libs/user");
    fs::create_dir_all(&app).unwrap();
    fs::create_dir_all(user.join("packages")).unwrap();
    let entry = |name: &str, source: &str| ProjectPackageDependency {
        name: name.to_string(),
        ident: name.to_string(),
        version: "=0.1.0".to_string(),
        pin: false,
        source: source.to_string(),
        ident_key: String::new(),
        direct: Some(true),
        required_by: Some(Vec::new()),
    };
    // A source directory, relative to the requirer.
    assert_eq!(
        rebase_source(&app, &user, &entry("base", "file:../base")),
        ("file:../libs/base".to_string(), None)
    );
    // The conventional `packages/<name>/` directory.
    assert_eq!(
        rebase_source(&app, &user, &entry("base", "")),
        ("file:../libs/user/packages/base".to_string(), None)
    );
    // Absolute and registry sources are the same everywhere.
    assert_eq!(
        rebase_source(&app, &user, &entry("base", "local:///opt/base")),
        ("local:///opt/base".to_string(), None)
    );
    assert_eq!(
        rebase_source(&app, &user, &entry("base", "ada#base")),
        ("ada#base".to_string(), None)
    );
    // An installed .mfp wins over the source beside it, and is installed.
    let installed = user.join("packages/base.mfp");
    fs::write(&installed, b"mfp").unwrap();
    assert_eq!(
        rebase_source(&app, &user, &entry("base", "file:../base")),
        ("file:packages/base.mfp".to_string(), Some(installed))
    );
}

#[test]
fn relative_paths_walk_up_to_the_common_ancestor() {
    let root = tempfile::tempdir().unwrap();
    let app = root.path().join("app");
    let base = root.path().join("libs/base");
    fs::create_dir_all(&app).unwrap();
    fs::create_dir_all(&base).unwrap();
    assert_eq!(relative_path(&app, &base).as_deref(), Some("../libs/base"));
    assert_eq!(relative_path(&app, &app).as_deref(), Some("."));
    assert_eq!(
        relative_path(&app, &app.join("packages/x.mfp")).as_deref(),
        Some("packages/x.mfp")
    );
}
