//! A resolver diagnostic names a file-PRIVATE symbol the way its author wrote it.
//!
//! `scope_privates` renames every PRIVATE top-level declaration to the internal
//! `#<file hash>$name` before the resolver runs (plan-24-C), and the resolver
//! interpolated that internal spelling into its messages: a PUBLIC and a PRIVATE
//! `FUNC r` of one signature in one file (bug-912's duplicate) reported
//! "Top-level symbol `#0fd20d0660c720e8$r` was already declared", and the
//! in-file reference to the rejected declaration then reported "Identifier
//! `#0fd20d0660c720e8$r` is not declared". The sigil form is untypeable and
//! means nothing to the person reading it; `internal_name::display_name` is the
//! render every user-facing message of a possibly-PRIVATE name goes through.

use crate::rules::collecting_diagnostics;

const MANIFEST: &str = r#"{
  "name": "app",
  "version": "0.1.0",
  "mfb": "1.0",
  "kind": "executable",
  "entry": "main",
  "targets": ["native"],
  "sources": [{ "root": "src", "role": "main", "include": ["**/*.mfb"] }]
}
"#;

/// Parse, scope and resolve a project of `files` the way `mfb build` does, and
/// return every diagnostic the resolver reported.
fn resolve_files(files: &[(&str, &str)]) -> Vec<(String, String)> {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("project.json"), MANIFEST).expect("manifest");
    std::fs::create_dir_all(dir.path().join("src")).expect("src");
    for (name, source) in files {
        std::fs::write(dir.path().join("src").join(name), source).expect("source");
    }
    let manifest_path = dir.path().join("project.json");
    let manifest = crate::manifest::parse_project_json(MANIFEST, &manifest_path).expect("json");
    let (result, diagnostics) = collecting_diagnostics(|| {
        let mut ast = crate::ast::manifest::parse_project("app", dir.path(), &manifest)
            .expect("project parses");
        crate::ast::scope_privates::scope_privates(&mut ast);
        crate::resolver::resolve_project(dir.path(), &manifest, &ast)
    });
    assert!(result.is_err(), "every program here has a resolver error");
    diagnostics
}

fn assert_no_internal_spelling(diagnostics: &[(String, String)]) {
    for (rule, detail) in diagnostics {
        assert!(
            !detail.contains('#'),
            "{rule} leaks an internal name: {detail}"
        );
    }
}

#[test]
fn a_public_and_private_duplicate_is_reported_under_the_written_name() {
    let diagnostics = resolve_files(&[
        (
            "a.mfb",
            "\
FUNC r(x AS Integer) AS String
  RETURN \"public\"
END FUNC
PRIVATE FUNC r(x AS Integer) AS String
  RETURN \"private\"
END FUNC
FUNC user() AS String
  LET f AS FUNC(Integer) AS String = r
  RETURN r(1) & f(2)
END FUNC
",
        ),
        (
            "main.mfb",
            "FUNC main() AS Integer\n  RETURN len(user())\nEND FUNC\n",
        ),
    ]);
    let rules: Vec<&str> = diagnostics.iter().map(|(rule, _)| rule.as_str()).collect();
    assert_eq!(
        rules,
        [
            "SYMBOL_DUPLICATE_TOP_LEVEL",
            "SYMBOL_UNKNOWN_IDENTIFIER",
            "SYMBOL_UNKNOWN_IDENTIFIER"
        ],
        "{diagnostics:?}"
    );
    assert!(
        diagnostics[0]
            .1
            .starts_with("Top-level symbol `r` was already declared in src/a.mfb:1."),
        "{}",
        diagnostics[0].1
    );
    // The in-file references were renamed to the rejected PRIVATE declaration,
    // so they cascade; they still name what the author wrote.
    let cascades: Vec<&str> = diagnostics[1..].iter().map(|(_, d)| d.as_str()).collect();
    assert!(
        cascades.contains(&"Identifier `r` is not declared in this scope.")
            && cascades.contains(&"Callable `r` is not a top-level function."),
        "{cascades:?}"
    );
    assert_no_internal_spelling(&diagnostics);
}

#[test]
fn private_duplicates_of_every_kind_are_reported_under_the_written_name() {
    let diagnostics = resolve_files(&[
        (
            "a.mfb",
            "\
PRIVATE FUNC helper(x AS Integer) AS Integer
  RETURN x
END FUNC
PRIVATE FUNC helper(x AS Integer) AS Integer
  RETURN x + 1
END FUNC
PRIVATE LET limit AS Integer = 1
PRIVATE LET limit AS Integer = 2
TYPE Frame
  w AS Integer
END TYPE
PRIVATE TYPE Frame
  h AS Integer
END TYPE
FUNC user() AS Integer
  RETURN helper(limit)
END FUNC
",
        ),
        (
            "main.mfb",
            "FUNC main() AS Integer\n  RETURN user()\nEND FUNC\n",
        ),
    ]);
    let duplicates: Vec<&str> = diagnostics
        .iter()
        .filter(|(rule, _)| rule == "SYMBOL_DUPLICATE_TOP_LEVEL")
        .map(|(_, detail)| detail.as_str())
        .collect();
    assert_eq!(duplicates.len(), 3, "{diagnostics:?}");
    for name in ["helper", "limit", "Frame"] {
        let want = format!("Top-level symbol `{name}` was already declared");
        assert!(
            duplicates.iter().any(|detail| detail.starts_with(&want)),
            "no duplicate names `{name}`: {duplicates:?}"
        );
    }
    assert_no_internal_spelling(&diagnostics);
}
