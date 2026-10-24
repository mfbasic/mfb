//! The 256-level IR nesting cap (`crate::ir::MAX_IR_NESTING_DEPTH`) bounds the
//! recursion of every IR walker. Lowering can add levels the source does not
//! spell — each argument union-wrapped into a parameter is a `UnionWrap` around
//! the value — so a source expression the parser admits (its own cap counts
//! source levels) can lower past the cap. That program must be refused at its
//! source line with a source rule by the checks `mfb build` runs first, never
//! later by the package verifier's internal `PACKAGE_BINARY_REPRESENTATION_*`
//! refusal.

use crate::testutil::check_src_details;

const PRELUDE: &str = "\
IMPORT io

TYPE Circle
  r AS Float
END TYPE

TYPE Square
  side AS Float
END TYPE

UNION Shape
  Circle
  Square
END UNION

FUNC wrapf(s AS Shape) AS Square
  RETURN Square[1.0]
END FUNC

FUNC area(s AS Shape) AS Integer
  RETURN 1
END FUNC

";

/// `main`, whose line 4 binds `d` to `depth` union-wrapped calls.
fn nested(depth: usize) -> String {
    format!(
        "{PRELUDE}FUNC main() AS Integer\n  LET d = area({}Square[2.0]{})\n  io::print(toString(d))\n  RETURN 0\nEND FUNC\n",
        "wrapf(".repeat(depth),
        ")".repeat(depth)
    )
}

/// The line of `LET d = ...` in [`nested`].
fn let_line() -> usize {
    PRELUDE.lines().count() + 2
}

fn found(src: &str) -> Vec<(String, usize)> {
    check_src_details(src)
        .into_iter()
        .map(|d| (d.rule, d.line))
        .collect()
}

#[test]
fn an_expression_that_lowers_past_the_ir_nesting_cap_is_refused_at_its_line() {
    // 140 source levels: inside the parser's 256, but each level is a call
    // plus a union wrap once lowered — about 280 IR levels.
    assert_eq!(
        found(&nested(140)),
        [("TYPE_EXPRESSION_TOO_DEEP".to_string(), let_line())]
    );
}

#[test]
fn an_expression_that_lowers_inside_the_cap_is_accepted() {
    assert_eq!(found(&nested(100)), []);
}

/// The build itself stops at the source rule: the user sees a diagnostic at
/// their line, and the package verifier's internal refusal never runs.
#[test]
fn mfb_build_refuses_the_too_deep_expression_with_the_source_rule() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("project.json"),
        "{\"name\": \"deep\", \"version\": \"0.1.0\", \"mfb\": \"1.0\", \"kind\": \
         \"executable\", \"entry\": \"main\", \"sources\": [{ \"root\": \"src\", \
         \"role\": \"main\", \"include\": [\"**/*.mfb\"] }]}",
    )
    .unwrap();
    std::fs::write(dir.path().join("src/main.mfb"), nested(140)).unwrap();
    let options = crate::cli::build::parse_build_options(vec![
        "-target".to_string(),
        "linux-x86_64".to_string(),
        dir.path().to_str().unwrap().to_string(),
    ])
    .unwrap();
    let (result, diagnostics) =
        crate::rules::collecting_diagnostics(|| crate::cli::build::build_project(&options));
    assert!(result.is_err(), "the build must fail");
    let rules: Vec<&str> = diagnostics.iter().map(|(rule, _)| rule.as_str()).collect();
    assert_eq!(rules, ["TYPE_EXPRESSION_TOO_DEEP"], "{diagnostics:?}");
}
