use super::*;
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::fs::symlink;

#[test]
fn glob_patterns_match_nested_and_root_files() {
    assert!(glob_matches("**/*.mfb", "main.mfb"));
    assert!(glob_matches("**/*.mfb", "pkg/main.mfb"));
    assert!(glob_matches("pkg/*.mfb", "pkg/main.mfb"));
    assert!(!glob_matches("pkg/*.mfb", "pkg/nested/main.mfb"));
    assert!(glob_matches("**/*_test.mfb", "pkg/math_test.mfb"));
    assert!(!glob_matches("**/*_test.mfb", "pkg/math.mfb"));
}

#[test]
fn parse_import_aliases() {
    let file = parse_source(
        Path::new("main.mfb"),
        "main.mfb",
        "IMPORT io AS term\nIMPORT math\n",
    )
    .expect("parse source");

    assert_eq!(file.imports.len(), 2);
    assert_eq!(file.imports[0].module, "io");
    assert_eq!(file.imports[0].alias.as_deref(), Some("term"));
    assert_eq!(file.imports[0].binding_name(), "term");
    assert_eq!(file.imports[0].package_name(), "io");
    assert_eq!(file.imports[1].module, "math");
    assert_eq!(file.imports[1].alias, None);
    assert_eq!(file.imports[1].binding_name(), "math");
    assert_eq!(file.imports[1].package_name(), "math");
}

#[test]
fn string_concat_has_lower_precedence_than_addition() {
    let file = parse_source(
        Path::new("main.mfb"),
        "main.mfb",
        "FUNC main AS String\n  RETURN a & b + c\nEND FUNC\n",
    )
    .expect("parse source");

    let Item::Function(function) = &file.items[0] else {
        panic!("expected function item");
    };
    let Statement::Return {
        value: Some(expression),
        ..
    } = &function.body[0]
    else {
        panic!("expected return expression");
    };

    let Expression::Binary {
        left,
        operator,
        right,
        ..
    } = expression
    else {
        panic!("expected binary expression");
    };
    assert_eq!(operator, "&");
    assert!(matches!(&**left, Expression::Identifier(name) if name == "a"));

    let Expression::Binary {
        left: add_left,
        operator: add_operator,
        right: add_right,
        ..
    } = &**right
    else {
        panic!("expected addition on concat right side");
    };
    assert_eq!(add_operator, "+");
    assert!(matches!(&**add_left, Expression::Identifier(name) if name == "b"));
    assert!(matches!(&**add_right, Expression::Identifier(name) if name == "c"));
}

#[test]
fn file_root_ignores_include_patterns() {
    let root = test_temp_dir("file_root_ignores_include_patterns");
    let project_dir = root.join("project");
    fs::create_dir_all(project_dir.join("src")).expect("project src");
    fs::write(project_dir.join("src/main.mfb"), "SUB main\nEND SUB\n").expect("write main");
    fs::write(project_dir.join("src/other.mfb"), "SUB other\nEND SUB\n").expect("write other");

    let manifest = manifest_with_sources(vec![source_entry(
        "src/main.mfb",
        Some(vec!["missing/**/*.mfb"]),
        None,
    )]);
    let canonical_project_dir = fs::canonicalize(&project_dir).expect("canonical project dir");
    let files = collect_selected_source_files(&project_dir, &canonical_project_dir, &manifest)
        .expect("files");

    assert_eq!(
        files,
        vec![SelectedSource {
            actual_path: canonical_project_dir.join("src/main.mfb"),
            display_path: project_dir.join("src/main.mfb"),
        }]
    );

    fs::remove_dir_all(root).expect("remove temp dir");
}

#[test]
fn directory_root_applies_include_and_exclude_patterns() {
    let root = test_temp_dir("directory_root_applies_include_and_exclude_patterns");
    let project_dir = root.join("project");
    fs::create_dir_all(project_dir.join("src/pkg")).expect("project pkg");
    fs::write(project_dir.join("src/main.mfb"), "SUB main\nEND SUB\n").expect("write main");
    fs::write(project_dir.join("src/pkg/keep.mfb"), "SUB keep\nEND SUB\n").expect("write keep");
    fs::write(
        project_dir.join("src/pkg/skip_test.mfb"),
        "SUB skip_test\nEND SUB\n",
    )
    .expect("write skip");

    let manifest = manifest_with_sources(vec![source_entry(
        "src",
        Some(vec!["pkg/**/*.mfb"]),
        Some(vec!["**/*_test.mfb"]),
    )]);
    let canonical_project_dir = fs::canonicalize(&project_dir).expect("canonical project dir");
    let files = collect_selected_source_files(&project_dir, &canonical_project_dir, &manifest)
        .expect("files");

    assert_eq!(
        files,
        vec![SelectedSource {
            actual_path: canonical_project_dir.join("src/pkg/keep.mfb"),
            display_path: project_dir.join("src/pkg/keep.mfb"),
        }]
    );

    fs::remove_dir_all(root).expect("remove temp dir");
}

#[test]
fn overlapping_source_entries_are_rejected() {
    let root = test_temp_dir("overlapping_source_entries_are_rejected");
    let project_dir = root.join("project");
    fs::create_dir_all(project_dir.join("src")).expect("project src");
    fs::write(project_dir.join("src/main.mfb"), "SUB main\nEND SUB\n").expect("write main");

    let manifest = manifest_with_sources(vec![
        source_entry("src", Some(vec!["**/*.mfb"]), None),
        source_entry("src/main.mfb", None, None),
    ]);
    let canonical_project_dir = fs::canonicalize(&project_dir).expect("canonical project dir");

    assert!(
        collect_selected_source_files(&project_dir, &canonical_project_dir, &manifest).is_err()
    );

    fs::remove_dir_all(root).expect("remove temp dir");
}

#[cfg(unix)]
#[test]
fn symlinked_source_paths_must_stay_inside_project() {
    let root = test_temp_dir("symlinked_source_paths_must_stay_inside_project");
    let project_dir = root.join("project");
    let outside_dir = root.join("outside");
    fs::create_dir_all(&project_dir).expect("project dir");
    fs::create_dir_all(&outside_dir).expect("outside dir");
    fs::write(outside_dir.join("escape.mfb"), "SUB escape\nEND SUB\n").expect("write escape");
    symlink(&outside_dir, project_dir.join("src")).expect("symlink src");

    let manifest =
        manifest_with_sources(vec![source_entry("src", Some(vec!["**/*.mfb"]), None)]);
    let canonical_project_dir = fs::canonicalize(&project_dir).expect("canonical project dir");

    assert!(
        collect_selected_source_files(&project_dir, &canonical_project_dir, &manifest).is_err()
    );

    fs::remove_dir_all(root).expect("remove temp dir");
}

fn manifest_with_sources(sources: Vec<JsonValue>) -> HashMap<String, JsonValue> {
    HashMap::from([("sources".to_string(), JsonValue::Array(sources))])
}

fn source_entry(
    root: &str,
    include: Option<Vec<&str>>,
    exclude: Option<Vec<&str>>,
) -> JsonValue {
    let mut source = HashMap::from([("root".to_string(), JsonValue::String(root.to_string()))]);
    if let Some(include) = include {
        source.insert(
            "include".to_string(),
            JsonValue::Array(
                include
                    .into_iter()
                    .map(|pattern| JsonValue::String(pattern.to_string()))
                    .collect(),
            ),
        );
    }
    if let Some(exclude) = exclude {
        source.insert(
            "exclude".to_string(),
            JsonValue::Array(
                exclude
                    .into_iter()
                    .map(|pattern| JsonValue::String(pattern.to_string()))
                    .collect(),
            ),
        );
    }
    JsonValue::Object(source)
}

fn test_temp_dir(name: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("timestamp")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("mfb_ast_{name}_{stamp}"));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("temp dir");
    root
}
