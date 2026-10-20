use super::*;
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::operators::{BinaryOp, UnaryOp};
#[cfg(unix)]
use std::os::unix::fs::symlink;

mod expressions_and_types;
mod parse_errors;
mod serialize_and_signatures;

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
fn file_root_ignores_include_patterns() {
    let root = test_temp_dir("file_root_ignores_include_patterns");
    let project_dir = root.join("project");
    fs::create_dir_all(project_dir.join("src")).expect("project src");
    fs::write(
        project_dir.join("src/main.mfb"),
        "FUNC main AS Nothing\nEND FUNC\n",
    )
    .expect("write main");
    fs::write(
        project_dir.join("src/other.mfb"),
        "FUNC other AS Nothing\nEND FUNC\n",
    )
    .expect("write other");

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
    fs::write(
        project_dir.join("src/main.mfb"),
        "FUNC main AS Nothing\nEND FUNC\n",
    )
    .expect("write main");
    fs::write(
        project_dir.join("src/pkg/keep.mfb"),
        "FUNC keep AS Nothing\nEND FUNC\n",
    )
    .expect("write keep");
    fs::write(
        project_dir.join("src/pkg/skip_test.mfb"),
        "FUNC skip_test AS Nothing\nEND FUNC\n",
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
    fs::write(
        project_dir.join("src/main.mfb"),
        "FUNC main AS Nothing\nEND FUNC\n",
    )
    .expect("write main");

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
    fs::write(
        outside_dir.join("escape.mfb"),
        "FUNC escape AS Nothing\nEND FUNC\n",
    )
    .expect("write escape");
    symlink(&outside_dir, project_dir.join("src")).expect("symlink src");

    let manifest = manifest_with_sources(vec![source_entry("src", Some(vec!["**/*.mfb"]), None)]);
    let canonical_project_dir = fs::canonicalize(&project_dir).expect("canonical project dir");

    assert!(
        collect_selected_source_files(&project_dir, &canonical_project_dir, &manifest).is_err()
    );

    fs::remove_dir_all(root).expect("remove temp dir");
}

// ---------------------------------------------------------------------------
// Expression-parser coverage (src/ast/expr.rs): primary/operator/lambda/with
// forms, parse_type_name variants, and remaining argument/edge/error paths.
// ---------------------------------------------------------------------------

/// Parse a whole program, expecting success.
fn parse_ok(src: &str) -> AstFile {
    parse_source(Path::new("main.mfb"), "main.mfb", src).expect("expected parse to succeed")
}

/// Parse a whole program, expecting a parse error.
fn parse_err(src: &str) {
    assert!(
        parse_source(Path::new("main.mfb"), "main.mfb", src).is_err(),
        "expected parse error for: {src:?}"
    );
}

/// Wrap an expression in a `FUNC main` whose body returns it.
fn ret(expr: &str) -> String {
    format!("FUNC main AS Integer\n  RETURN {expr}\nEND FUNC\n")
}

/// Extract the first return-expression of `main`.
fn first_return_expr(file: &AstFile) -> &Expression {
    let Item::Function(function) = &file.items[0] else {
        panic!("expected a function item");
    };
    let Statement::Return {
        value: Some(expr), ..
    } = &function.body[0]
    else {
        panic!("expected a return with a value");
    };
    expr
}

/// Parse `RETURN <expr>` and return the resulting expression.
fn expr_of(expr: &str) -> Expression {
    let file = parse_ok(&ret(expr));
    first_return_expr(&file).clone()
}

fn type_err(type_name: &str) {
    let src = format!("FUNC main AS Nothing\n  LET v AS {type_name} = NOTHING\nEND FUNC\n");
    parse_err(&src);
}

// ---------------------------------------------------------------------------
// Project / manifest assembly coverage (src/ast/manifest.rs): parse_project /
// write_ast / selected_source_paths, nested directory walk and escape checks,
// glob edge cases, and canonicalize/read failure diagnostic paths.
// ---------------------------------------------------------------------------

#[test]
fn parse_project_reads_files_and_appends_prelude() {
    let root = test_temp_dir("parse_project_reads_files_and_appends_prelude");
    let project_dir = root.join("project");
    fs::create_dir_all(project_dir.join("src")).expect("src dir");
    fs::write(
        project_dir.join("src/main.mfb"),
        "FUNC main AS Nothing\nEND FUNC\n",
    )
    .expect("write main");

    let manifest = manifest_with_sources(vec![source_entry("src", None, None)]);
    let project = parse_project("demo", &project_dir, &manifest).expect("parse project");

    assert_eq!(project.name, "demo");
    // The user's source is files[0]; the compiler-owned prelude is appended.
    assert_eq!(project.files[0].path, "src/main.mfb");
    assert!(
        project
            .files
            .iter()
            .any(|file| file.path == BUILTIN_PRELUDE_PATH),
        "prelude file must be appended"
    );

    fs::remove_dir_all(root).expect("remove temp dir");
}

#[test]
fn parse_project_rejects_missing_project_dir() {
    let missing = std::env::temp_dir().join("mfb_ast_parse_project_missing_dir_zzz_nope");
    let _ = fs::remove_dir_all(&missing);
    let manifest = manifest_with_sources(vec![source_entry("src", None, None)]);
    assert!(parse_project("demo", &missing, &manifest).is_err());
}

#[test]
fn selected_source_paths_are_sorted_actual_paths() {
    let root = test_temp_dir("selected_source_paths_are_sorted_actual_paths");
    let project_dir = root.join("project");
    fs::create_dir_all(project_dir.join("src")).expect("src dir");
    fs::write(
        project_dir.join("src/a.mfb"),
        "FUNC a AS Nothing\nEND FUNC\n",
    )
    .expect("write a");
    fs::write(
        project_dir.join("src/b.mfb"),
        "FUNC b AS Nothing\nEND FUNC\n",
    )
    .expect("write b");

    let manifest = manifest_with_sources(vec![source_entry("src", None, None)]);
    let paths = selected_source_paths(&project_dir, &manifest).expect("paths");
    assert_eq!(paths.len(), 2);
    assert!(paths[0].ends_with("a.mfb"));
    assert!(paths[1].ends_with("b.mfb"));

    fs::remove_dir_all(root).expect("remove temp dir");
}

#[test]
fn selected_source_paths_rejects_missing_project_dir() {
    let missing = std::env::temp_dir().join("mfb_ast_selected_paths_missing_zzz_nope");
    let _ = fs::remove_dir_all(&missing);
    let manifest = manifest_with_sources(vec![source_entry("src", None, None)]);
    assert!(selected_source_paths(&missing, &manifest).is_err());
}

#[test]
fn write_ast_writes_ast_json_file() {
    let root = test_temp_dir("write_ast_writes_ast_json_file");
    let project_dir = root.join("project");
    fs::create_dir_all(project_dir.join("src")).expect("src dir");
    fs::write(
        project_dir.join("src/main.mfb"),
        "FUNC main AS Nothing\nEND FUNC\n",
    )
    .expect("write main");

    let manifest = manifest_with_sources(vec![source_entry("src", None, None)]);
    let project = parse_project("demo", &project_dir, &manifest).expect("parse project");
    let ast_path = write_ast(&project_dir, &project).expect("write ast");
    assert_eq!(ast_path, project_dir.join("demo.ast"));
    let contents = fs::read_to_string(&ast_path).expect("read ast");
    assert!(contents.contains("\"files\""), "{contents}");

    fs::remove_dir_all(root).expect("remove temp dir");
}

#[test]
fn write_ast_reports_write_failure() {
    let root = test_temp_dir("write_ast_reports_write_failure");
    // Point at a non-existent directory so the write fails.
    let bogus_dir = root.join("does_not_exist");
    let project = AstProject {
        name: "demo".to_string(),
        files: Vec::new(),
    };
    assert!(write_ast(&bogus_dir, &project).is_err());

    fs::remove_dir_all(root).expect("remove temp dir");
}

#[test]
fn parse_source_internal_marks_file_internal() {
    let file = parse_source_internal(
        Path::new("pkg.mfb"),
        "pkg.mfb",
        "FUNC main AS Nothing\nEND FUNC\n",
    )
    .expect("parse internal");
    assert!(file.internal);
    assert_eq!(file.path, "pkg.mfb");
}

#[test]
fn source_root_missing_is_rejected() {
    let root = test_temp_dir("source_root_missing_is_rejected");
    let project_dir = root.join("project");
    fs::create_dir_all(&project_dir).expect("project dir");

    let manifest = manifest_with_sources(vec![source_entry("missing_root", None, None)]);
    let canonical = fs::canonicalize(&project_dir).expect("canonical");
    assert!(collect_selected_source_files(&project_dir, &canonical, &manifest).is_err());

    fs::remove_dir_all(root).expect("remove temp dir");
}

#[test]
fn empty_source_root_is_rejected() {
    let root = test_temp_dir("empty_source_root_is_rejected");
    let project_dir = root.join("project");
    fs::create_dir_all(project_dir.join("src")).expect("src dir");
    // No .mfb files under src.
    fs::write(project_dir.join("src/readme.txt"), "not source\n").expect("write txt");

    let manifest = manifest_with_sources(vec![source_entry("src", None, None)]);
    let canonical = fs::canonicalize(&project_dir).expect("canonical");
    assert!(collect_selected_source_files(&project_dir, &canonical, &manifest).is_err());

    fs::remove_dir_all(root).expect("remove temp dir");
}

#[test]
fn source_entry_without_root_is_ignored() {
    let root = test_temp_dir("source_entry_without_root_is_ignored");
    let project_dir = root.join("project");
    fs::create_dir_all(project_dir.join("src")).expect("src dir");
    fs::write(
        project_dir.join("src/main.mfb"),
        "FUNC main AS Nothing\nEND FUNC\n",
    )
    .expect("write main");

    // One entry has no `root` key (filtered out by source_entries), the other is valid.
    let bad = JsonValue::Object(HashMap::from([(
        "include".to_string(),
        JsonValue::Array(vec![JsonValue::String("**/*.mfb".to_string())]),
    )]));
    let good = source_entry("src", None, None);
    let manifest = manifest_with_sources(vec![bad, good]);
    let canonical = fs::canonicalize(&project_dir).expect("canonical");
    let files = collect_selected_source_files(&project_dir, &canonical, &manifest).expect("files");
    assert_eq!(files.len(), 1);

    fs::remove_dir_all(root).expect("remove temp dir");
}

#[test]
fn file_root_ignores_non_mfb_extension() {
    let root = test_temp_dir("file_root_ignores_non_mfb_extension");
    let project_dir = root.join("project");
    fs::create_dir_all(project_dir.join("src")).expect("src dir");
    fs::write(
        project_dir.join("src/main.mfb"),
        "FUNC main AS Nothing\nEND FUNC\n",
    )
    .expect("write main");
    fs::write(project_dir.join("src/notes.txt"), "notes\n").expect("write notes");

    // A file root pointing at a non-.mfb file selects nothing, so the entry is
    // empty and rejected.
    let manifest = manifest_with_sources(vec![source_entry("src/notes.txt", None, None)]);
    let canonical = fs::canonicalize(&project_dir).expect("canonical");
    assert!(collect_selected_source_files(&project_dir, &canonical, &manifest).is_err());

    fs::remove_dir_all(root).expect("remove temp dir");
}

#[cfg(unix)]
#[test]
fn directory_cycle_via_symlink_is_tolerated() {
    let root = test_temp_dir("directory_cycle_via_symlink_is_tolerated");
    let project_dir = root.join("project");
    fs::create_dir_all(project_dir.join("src/sub")).expect("src/sub");
    fs::write(
        project_dir.join("src/main.mfb"),
        "FUNC main AS Nothing\nEND FUNC\n",
    )
    .expect("write main");
    // A symlink pointing back to src forms a directory cycle; visited-dir
    // tracking must break it and still return the real file.
    symlink(project_dir.join("src"), project_dir.join("src/sub/loop")).expect("symlink loop");

    let manifest = manifest_with_sources(vec![source_entry("src", None, None)]);
    let canonical = fs::canonicalize(&project_dir).expect("canonical");
    let files = collect_selected_source_files(&project_dir, &canonical, &manifest).expect("files");
    assert_eq!(files.len(), 1);
    assert!(files[0].actual_path.ends_with("main.mfb"));

    fs::remove_dir_all(root).expect("remove temp dir");
}

#[cfg(unix)]
#[test]
fn nested_symlink_escape_is_rejected() {
    let root = test_temp_dir("nested_symlink_escape_is_rejected");
    let project_dir = root.join("project");
    let outside_dir = root.join("outside");
    fs::create_dir_all(project_dir.join("src")).expect("src dir");
    fs::create_dir_all(&outside_dir).expect("outside dir");
    fs::write(
        project_dir.join("src/main.mfb"),
        "FUNC main AS Nothing\nEND FUNC\n",
    )
    .expect("write main");
    fs::write(
        outside_dir.join("escape.mfb"),
        "FUNC escape AS Nothing\nEND FUNC\n",
    )
    .expect("write escape");
    // A file symlink inside a scanned directory pointing outside the project.
    symlink(
        outside_dir.join("escape.mfb"),
        project_dir.join("src/escape.mfb"),
    )
    .expect("symlink escape");

    let manifest = manifest_with_sources(vec![source_entry("src", None, None)]);
    let canonical = fs::canonicalize(&project_dir).expect("canonical");
    assert!(collect_selected_source_files(&project_dir, &canonical, &manifest).is_err());

    fs::remove_dir_all(root).expect("remove temp dir");
}

#[test]
fn glob_star_and_question_components_match() {
    // Exercise `*` (with backtracking) and `?` single-char wildcards.
    assert!(glob_matches("f*o", "foo"));
    assert!(glob_matches("f*o", "fabco"));
    assert!(glob_matches("a?c", "abc"));
    assert!(!glob_matches("a?c", "ac"));
    assert!(!glob_matches("abc", "abd"));
    // Trailing star matches the remaining suffix (including empty).
    assert!(glob_matches("ab*", "ab"));
    assert!(glob_matches("ab*", "abcdef"));
    // `**` matches across an empty path tail.
    assert!(glob_matches("src/**", "src"));
    // A literal segment that does not match fails immediately.
    assert!(!glob_matches("src/*.mfb", "lib/main.mfb"));
}

#[test]
fn parse_project_propagates_collection_failure() {
    // A missing source root makes collection fail; the error propagates out of
    // `parse_project` (the `?` on `collect_selected_source_files`).
    let root = test_temp_dir("parse_project_propagates_collection_failure");
    let project_dir = root.join("project");
    fs::create_dir_all(&project_dir).expect("project dir");
    let manifest = manifest_with_sources(vec![source_entry("missing_root", None, None)]);
    assert!(parse_project("demo", &project_dir, &manifest).is_err());
    fs::remove_dir_all(root).expect("remove temp dir");
}

#[test]
fn selected_source_paths_propagates_collection_failure() {
    let root = test_temp_dir("selected_source_paths_propagates_collection_failure");
    let project_dir = root.join("project");
    fs::create_dir_all(&project_dir).expect("project dir");
    let manifest = manifest_with_sources(vec![source_entry("missing_root", None, None)]);
    assert!(selected_source_paths(&project_dir, &manifest).is_err());
    fs::remove_dir_all(root).expect("remove temp dir");
}

#[cfg(unix)]
#[test]
fn broken_symlink_entry_reports_read_failure() {
    // A dangling symlink with an `.mfb` name exists() as a directory entry but
    // fails to canonicalize, exercising the per-entry canonicalize `?`.
    let root = test_temp_dir("broken_symlink_entry_reports_read_failure");
    let project_dir = root.join("project");
    fs::create_dir_all(project_dir.join("src")).expect("src dir");
    fs::write(
        project_dir.join("src/main.mfb"),
        "FUNC main AS Nothing\nEND FUNC\n",
    )
    .expect("write main");
    symlink(
        project_dir.join("src/nonexistent_target.mfb"),
        project_dir.join("src/dangling.mfb"),
    )
    .expect("symlink dangling");

    let manifest = manifest_with_sources(vec![source_entry("src", None, None)]);
    let canonical = fs::canonicalize(&project_dir).expect("canonical");
    assert!(collect_selected_source_files(&project_dir, &canonical, &manifest).is_err());

    fs::remove_dir_all(root).expect("remove temp dir");
}

#[test]
fn parse_source_reports_lex_failure() {
    // A stray control/invalid byte makes the lexer fail before parsing.
    assert!(parse_source(
        Path::new("main.mfb"),
        "main.mfb",
        "FUNC main AS Nothing\n  LET x = `\nEND FUNC\n"
    )
    .is_err());
    assert!(parse_source_internal(
        Path::new("pkg.mfb"),
        "pkg.mfb",
        "FUNC main AS Nothing\n  LET x = `\nEND FUNC\n"
    )
    .is_err());
}

#[cfg(unix)]
#[test]
fn parse_project_reports_unreadable_source_file() {
    use std::os::unix::fs::PermissionsExt;

    let root = test_temp_dir("parse_project_reports_unreadable_source_file");
    let project_dir = root.join("project");
    fs::create_dir_all(project_dir.join("src")).expect("src dir");
    let file = project_dir.join("src/main.mfb");
    fs::write(&file, "FUNC main AS Nothing\nEND FUNC\n").expect("write main");
    // Strip all read permission so parse_file's read_to_string fails.
    fs::set_permissions(&file, fs::Permissions::from_mode(0o000)).expect("chmod");

    let manifest = manifest_with_sources(vec![source_entry("src", None, None)]);
    let result = parse_project("demo", &project_dir, &manifest);

    // Restore permission before assertions so cleanup can run.
    fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).expect("restore chmod");
    assert!(result.is_err());

    fs::remove_dir_all(root).expect("remove temp dir");
}

#[cfg(unix)]
#[test]
fn unreadable_directory_reports_read_failure() {
    use std::os::unix::fs::PermissionsExt;

    let root = test_temp_dir("unreadable_directory_reports_read_failure");
    let project_dir = root.join("project");
    let locked = project_dir.join("src/locked");
    fs::create_dir_all(&locked).expect("locked dir");
    fs::write(
        project_dir.join("src/main.mfb"),
        "FUNC main AS Nothing\nEND FUNC\n",
    )
    .expect("write main");
    fs::write(
        locked.join("inner.mfb"),
        "FUNC inner AS Nothing\nEND FUNC\n",
    )
    .expect("write inner");
    // Remove read/exec permission so read_dir on the nested directory fails
    // with a non-PermissionDenied-mapped error path.
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).expect("chmod");

    let manifest = manifest_with_sources(vec![source_entry("src", None, None)]);
    let canonical = fs::canonicalize(&project_dir).expect("canonical");
    let result = collect_selected_source_files(&project_dir, &canonical, &manifest);

    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).expect("restore chmod");
    assert!(result.is_err());

    fs::remove_dir_all(root).expect("remove temp dir");
}

fn manifest_with_sources(sources: Vec<JsonValue>) -> HashMap<String, JsonValue> {
    HashMap::from([("sources".to_string(), JsonValue::Array(sources))])
}

fn source_entry(root: &str, include: Option<Vec<&str>>, exclude: Option<Vec<&str>>) -> JsonValue {
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

#[test]
fn parse_project_propagates_parse_errors() {
    let root = test_temp_dir("parse_project_propagates_parse_errors");
    let project_dir = root.join("project");
    fs::create_dir_all(project_dir.join("src")).expect("src");
    fs::write(project_dir.join("src/main.mfb"), "garbage nonsense\n").expect("main");

    let manifest = manifest_with_sources(vec![source_entry("src", None, None)]);
    assert!(parse_project("demo", &project_dir, &manifest).is_err());

    fs::remove_dir_all(root).expect("remove");
}

#[test]
fn parse_project_missing_directory_is_error() {
    let root = test_temp_dir("parse_project_missing_directory_is_error");
    let manifest = manifest_with_sources(vec![source_entry("src", None, None)]);
    // The project directory itself does not exist -> canonicalize fails.
    assert!(parse_project("demo", &root.join("nope"), &manifest).is_err());
    fs::remove_dir_all(root).expect("remove");
}

#[test]
fn collect_reports_missing_and_empty_roots() {
    let root = test_temp_dir("collect_reports_missing_and_empty_roots");
    let project_dir = root.join("project");
    fs::create_dir_all(project_dir.join("src")).expect("src");
    let canonical = fs::canonicalize(&project_dir).expect("canonical");

    // A source root that does not exist.
    let missing = manifest_with_sources(vec![source_entry("nope", None, None)]);
    assert!(collect_selected_source_files(&project_dir, &canonical, &missing).is_err());

    // A source root that has no matching .mfb files.
    let empty = manifest_with_sources(vec![source_entry("src", None, None)]);
    assert!(collect_selected_source_files(&project_dir, &canonical, &empty).is_err());

    fs::remove_dir_all(root).expect("remove");
}

#[test]
fn collect_handles_single_file_and_non_mfb_root() {
    let root = test_temp_dir("collect_handles_single_file_and_non_mfb_root");
    let project_dir = root.join("project");
    fs::create_dir_all(&project_dir).expect("dir");
    fs::write(
        project_dir.join("main.mfb"),
        "FUNC main AS Nothing\nEND FUNC\n",
    )
    .expect("main");
    fs::write(project_dir.join("notes.txt"), "hi\n").expect("notes");
    let canonical = fs::canonicalize(&project_dir).expect("canonical");

    // A direct file root that is an .mfb file is selected.
    let file_manifest = manifest_with_sources(vec![source_entry("main.mfb", None, None)]);
    let files =
        collect_selected_source_files(&project_dir, &canonical, &file_manifest).expect("files");
    assert_eq!(files.len(), 1);

    // A direct file root that is not .mfb yields an empty selection -> error.
    let txt_manifest = manifest_with_sources(vec![source_entry("notes.txt", None, None)]);
    assert!(collect_selected_source_files(&project_dir, &canonical, &txt_manifest).is_err());

    fs::remove_dir_all(root).expect("remove");
}

#[test]
fn source_entries_default_include_when_absent() {
    // A manifest with no `sources` key yields no entries -> empty selection error.
    let root = test_temp_dir("source_entries_default_include_when_absent");
    let project_dir = root.join("project");
    fs::create_dir_all(&project_dir).expect("dir");
    let canonical = fs::canonicalize(&project_dir).expect("canonical");
    let manifest: HashMap<String, JsonValue> = HashMap::new();
    let files =
        collect_selected_source_files(&project_dir, &canonical, &manifest).expect("empty ok");
    assert!(files.is_empty());
    fs::remove_dir_all(root).expect("remove");
}

#[test]
fn collect_walks_nested_directories() {
    let root = test_temp_dir("collect_walks_nested_directories");
    let project_dir = root.join("project");
    fs::create_dir_all(project_dir.join("src/a/b")).expect("dirs");
    fs::write(
        project_dir.join("src/top.mfb"),
        "FUNC top AS Nothing\nEND FUNC\n",
    )
    .expect("top");
    fs::write(
        project_dir.join("src/a/mid.mfb"),
        "FUNC mid AS Nothing\nEND FUNC\n",
    )
    .expect("mid");
    fs::write(
        project_dir.join("src/a/b/deep.mfb"),
        "FUNC deep AS Nothing\nEND FUNC\n",
    )
    .expect("deep");
    fs::write(project_dir.join("src/a/skip.txt"), "x\n").expect("txt");
    let canonical = fs::canonicalize(&project_dir).expect("canonical");
    let manifest = manifest_with_sources(vec![source_entry("src", None, None)]);
    let files = collect_selected_source_files(&project_dir, &canonical, &manifest).expect("files");
    assert_eq!(files.len(), 3);
    fs::remove_dir_all(root).expect("remove");
}

#[test]
fn selected_source_paths_missing_dir_is_error() {
    let root = test_temp_dir("selected_source_paths_missing_dir_is_error");
    let manifest = manifest_with_sources(vec![source_entry("src", None, None)]);
    assert!(selected_source_paths(&root.join("nope"), &manifest).is_err());
    fs::remove_dir_all(root).expect("remove");
}

#[cfg(unix)]
#[test]
fn nested_symlink_escaping_project_is_rejected() {
    let root = test_temp_dir("nested_symlink_escaping_project_is_rejected");
    let project_dir = root.join("project");
    let outside = root.join("outside");
    fs::create_dir_all(project_dir.join("src")).expect("src");
    fs::create_dir_all(&outside).expect("outside");
    fs::write(
        outside.join("escape.mfb"),
        "FUNC escape AS Nothing\nEND FUNC\n",
    )
    .expect("escape");
    // A symlink *inside* a walked subdirectory pointing outside the project.
    symlink(&outside, project_dir.join("src/link")).expect("symlink");
    let canonical = fs::canonicalize(&project_dir).expect("canonical");
    let manifest = manifest_with_sources(vec![source_entry("src", None, None)]);
    assert!(collect_selected_source_files(&project_dir, &canonical, &manifest).is_err());
    fs::remove_dir_all(root).expect("remove");
}

#[test]
fn glob_component_edge_cases() {
    // Trailing `*` after the value is consumed.
    assert!(glob_matches("a*", "a"));
    // `?` matches exactly one character.
    assert!(glob_matches("a?c", "abc"));
    assert!(!glob_matches("a?c", "ac"));
    // A `*` in the middle backtracks.
    assert!(glob_matches("a*c", "abbbc"));
    assert!(!glob_matches("a*c", "abbb"));
    // `**` matches zero segments.
    assert!(glob_matches("**/x", "x"));
    // A literal mismatch fails fast.
    assert!(!glob_matches("abc", "xbc"));
}

#[cfg(unix)]
#[test]
fn broken_symlink_root_reports_canonicalize_failure() {
    let root = test_temp_dir("broken_symlink_root_reports_canonicalize_failure");
    let project_dir = root.join("project");
    fs::create_dir_all(&project_dir).expect("dir");
    // A source root that is a dangling symlink: exists() is true (the link),
    // but canonicalize() fails on the missing target.
    symlink(project_dir.join("missing_target"), project_dir.join("src")).expect("symlink");
    let canonical = fs::canonicalize(&project_dir).expect("canonical");
    let manifest = manifest_with_sources(vec![source_entry("src", None, None)]);
    assert!(collect_selected_source_files(&project_dir, &canonical, &manifest).is_err());
    fs::remove_dir_all(root).expect("remove");
}

#[cfg(unix)]
#[test]
fn unreadable_source_file_reports_read_failure() {
    use std::os::unix::fs::PermissionsExt;
    let root = test_temp_dir("unreadable_source_file_reports_read_failure");
    let project_dir = root.join("project");
    fs::create_dir_all(project_dir.join("src")).expect("src");
    let file = project_dir.join("src/main.mfb");
    fs::write(&file, "FUNC main AS Nothing\nEND FUNC\n").expect("write");
    // Remove all read permission so parse_file's read_to_string fails.
    fs::set_permissions(&file, fs::Permissions::from_mode(0o000)).expect("chmod");
    let manifest = manifest_with_sources(vec![source_entry("src", None, None)]);
    let result = parse_project("demo", &project_dir, &manifest);
    // Restore permissions so cleanup can proceed regardless of the outcome.
    let _ = fs::set_permissions(&file, fs::Permissions::from_mode(0o644));
    assert!(result.is_err());
    fs::remove_dir_all(root).expect("remove");
}

#[cfg(unix)]
#[test]
fn symlink_cycle_within_project_is_skipped() {
    let root = test_temp_dir("symlink_cycle_within_project_is_skipped");
    let project_dir = root.join("project");
    fs::create_dir_all(project_dir.join("src")).expect("src");
    fs::write(
        project_dir.join("src/main.mfb"),
        "FUNC main AS Nothing\nEND FUNC\n",
    )
    .expect("main");
    // A symlink pointing back at its own parent creates a cycle; the visited-dirs
    // guard must break it rather than recursing forever.
    symlink(project_dir.join("src"), project_dir.join("src/loop")).expect("symlink");
    let canonical = fs::canonicalize(&project_dir).expect("canonical");
    let manifest = manifest_with_sources(vec![source_entry("src", None, None)]);
    let files = collect_selected_source_files(&project_dir, &canonical, &manifest).expect("files");
    assert_eq!(files.len(), 1);
    fs::remove_dir_all(root).expect("remove");
}

// ---------------------------------------------------------------------------
// plan-12 coverage: source-driven parse + serialize (src/ast/serialize.rs).
// One rich fixture over every AST node kind, signature_line + placeholder
// helpers, and the null/None default branches of each renderer.
// ---------------------------------------------------------------------------

/// Parse a single source string into an `AstFile`, panicking on parse error.
fn parse_file(src: &str) -> AstFile {
    parse_source(Path::new("main.mfb"), "main.mfb", src).expect("parse source")
}

/// Attempt to parse a source string; `Ok(file)` on success, `Err(())` on a
/// reported diagnostic.
fn try_parse(src: &str) -> Result<AstFile, ()> {
    parse_source(Path::new("main.mfb"), "main.mfb", src)
}

/// Wrap a parsed file in a project and render its `.ast` JSON dump.
fn project_json(src: &str) -> String {
    let file = parse_file(src);
    let project = AstProject {
        name: "demo".to_string(),
        files: vec![file],
    };
    project.to_json()
}

fn function<'a>(file: &'a AstFile, name: &str) -> &'a Function {
    file.items
        .iter()
        .find_map(|item| match item {
            Item::Function(function) if function.name == name => Some(function),
            _ => None,
        })
        .expect("function item")
}

fn link_fn(body: &str) -> String {
    format!("LINK \"x\" AS l\n  FUNC f() AS Integer\n{body}  END FUNC\nEND LINK\n")
}
