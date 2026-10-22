// ---------------------------------------------------------------------------
// audit/mod.rs `run` — the exit code of each front-end gate an audit passes
// through before it reports, and of a report carrying an error finding.
// ---------------------------------------------------------------------------

use super::*;

/// Audit an executable project whose `src/main.mfb` is `source` (and whose
/// manifest adds `extra` fields); answer the exit code and the rules of the
/// diagnostics it reported.
fn audit_exit(source: &str, extra: &str, locked: bool) -> (i32, Vec<String>) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("project.json"),
        format!(
            "{{\"name\": \"app\", \"version\": \"0.1.0\", \"mfb\": \"1.0\", \"kind\": \
             \"executable\", {extra}\"sources\": [{{ \"root\": \"src\", \"role\": \"main\", \
             \"include\": [\"**/*.mfb\"] }}]}}"
        ),
    )
    .unwrap();
    std::fs::write(dir.path().join("src/main.mfb"), source).unwrap();
    let (code, diagnostics) = crate::rules::collecting_diagnostics(|| {
        run(&AuditOptions {
            location: dir.path().to_path_buf(),
            format: AuditFormat::Text,
            locked,
        })
    });
    (
        code,
        diagnostics.into_iter().map(|(rule, _)| rule).collect(),
    )
}

const CLEAN: &str = "IMPORT io\n\nFUNC main() AS Nothing\n  io::print(\"hi\")\nEND FUNC\n";

#[test]
fn a_clean_project_exits_zero() {
    assert_eq!(
        audit_exit(CLEAN, "\"entry\": \"main\", ", false),
        (0, Vec::new())
    );
}

/// A call to a function nothing declares fails name resolution: exit 3.
#[test]
fn an_unresolved_name_exits_three() {
    assert_eq!(
        audit_exit(
            "FUNC main() AS Nothing\n  nowhere()\nEND FUNC\n",
            "\"entry\": \"main\", ",
            false
        ),
        (3, vec!["SYMBOL_UNKNOWN_IDENTIFIER".to_string()])
    );
}

/// An entry naming no declared function fails entry-point validation: exit 3.
#[test]
fn a_missing_entry_point_exits_three() {
    assert_eq!(
        audit_exit(CLEAN, "\"entry\": \"start\", ", false),
        (3, vec!["PROJECT_ENTRY_INVALID".to_string()])
    );
}

/// A named argument the callee does not declare fails the shape pass: exit 3.
#[test]
fn an_unknown_argument_name_exits_three() {
    assert_eq!(
        audit_exit(
            "IMPORT io\n\nFUNC helper(n AS Integer) AS Integer\n  RETURN n\nEND FUNC\n\
             FUNC main() AS Nothing\n  io::print(toString(helper(m := 1)))\nEND FUNC\n",
            "\"entry\": \"main\", ",
            false
        ),
        (
            3,
            vec![
                "TYPE_UNKNOWN_ARGUMENT_NAME".to_string(),
                "TYPE_CALL_ARITY_MISMATCH".to_string()
            ]
        )
    );
}

/// `--locked` without a lockfile is an error-severity finding: the report is
/// printed and the audit exits 1.
#[test]
fn an_error_finding_exits_one() {
    assert_eq!(
        audit_exit(CLEAN, "\"entry\": \"main\", ", true),
        (1, Vec::new())
    );
}

/// Two templates equally specific for one call fail monomorphization: exit 3.
#[test]
fn an_ambiguous_template_call_exits_three() {
    assert_eq!(
        audit_exit(
            "FUNC f OF T(x AS T, y AS Integer) AS String\n  RETURN \"a\"\nEND FUNC\n\
             FUNC f OF T(x AS Integer, y AS T) AS String\n  RETURN \"b\"\nEND FUNC\n\
             FUNC main() AS Nothing\n  LET s = f(1, 2)\nEND FUNC\n",
            "\"entry\": \"main\", ",
            false
        ),
        (3, vec!["TYPE_OVERLOAD_AMBIGUOUS".to_string()])
    );
}
