//! Parse-time refusals, asserted by RULE for the first time.
//!
//! Every diagnostic in `src/ast/**` is emitted by `rules::show_diagnostic` and
//! then thrown away: `parse_source` returns `Result<AstFile, ()>`, where the
//! `()` means "diagnostics were printed". Nothing in process could read them —
//! `testutil::check_src` panics outright on a program that does not parse, so
//! the diagnostic corpus contains only programs that DO parse and fail later.
//! The whole parse-time refusal surface was reachable only through
//! `test-accept.sh`, whose coverage lands in a child process (Finding F1).
//!
//! `rules::collecting_diagnostics` is the affordance: a `#[cfg(test)]`
//! thread-local that takes the diagnostic instead of rendering it. Scoped, so
//! one test collecting cannot silence another beside it, and `cfg(test)` so a
//! release build has neither the slot nor the branch.
//!
//! Asserting the RULE and not just `is_err()` is the point. "It did not parse"
//! is true of a typo and of a program the grammar deliberately refuses, and only
//! the second is a contract. These are the refusals whose wording is the whole
//! value — a doc comment with no closing fence, a header the renderer cannot
//! place, an argument list with a name a callable type cannot bind.

use crate::ast::parse_source;
use crate::rules::collecting_diagnostics;
use std::path::Path;

/// Parse `source` with its diagnostics collected, and return their rule codes.
fn refusal(what: &str, source: &str) -> Vec<String> {
    let (parsed, diagnostics) =
        collecting_diagnostics(|| parse_source(Path::new("main.mfb"), "main.mfb", source));
    assert!(
        parsed.is_err(),
        "{what} must be refused; the parser accepted it"
    );
    assert!(
        !diagnostics.is_empty(),
        "{what} was refused with NO diagnostic, which leaves the author nothing \
         to act on"
    );
    diagnostics.into_iter().map(|(rule, _)| rule).collect()
}

/// A malformed program is refused with the rule that names what is wrong.
#[test]
fn each_parse_refusal_names_its_rule() {
    for (what, rule, source) in [
        (
            "a binding with no name",
            "MFB_PARSE_INVALID_IDENTIFIER",
            "FUNC main() AS Integer\n  LET 5 = 1\n  RETURN 0\nEND FUNC\n",
        ),
        (
            "a block that never closes",
            "MFB_PARSE_UNTERMINATED_BLOCK",
            "FUNC main() AS Integer\n  IF TRUE THEN\n  RETURN 0\nEND FUNC\n",
        ),
        (
            "an expression that is not there",
            "MFB_PARSE_EXPECTED_EXPRESSION",
            "FUNC main() AS Integer\n  LET a AS Integer = \n  RETURN 0\nEND FUNC\n",
        ),
        (
            // `GOTO` is an identifier to this grammar, so `GOTO 10` is an
            // expression followed by a token that cannot continue it -- an
            // UNEXPECTED_TOKEN, not an unexpected STATEMENT. Recorded as the
            // parser says it rather than as I first guessed.
            "a token that cannot continue the statement",
            "MFB_PARSE_UNEXPECTED_TOKEN",
            "FUNC main() AS Integer\n  GOTO 10\n  RETURN 0\nEND FUNC\n",
        ),
        (
            // The top level accepts declarations only, so a bare statement
            // there is a STATEMENT the grammar does not have.
            "a statement at the top level",
            "MFB_PARSE_UNEXPECTED_STATEMENT",
            "IMPORT io\n\nio::print(\"hello\")\n\nFUNC main() AS Integer\n  RETURN 0\nEND FUNC\n",
        ),
        (
            "a record field written with `=`",
            "MFB_PARSE_RECORD_FIELD_ASSIGNMENT",
            "TYPE P\n  x AS Integer\nEND TYPE\n\nFUNC main() AS Integer\n  MUT p AS P = P[1]\n  p.x = 2\n  RETURN 0\nEND FUNC\n",
        ),
    ] {
        let rules = refusal(what, source);
        assert!(
            rules.iter().any(|r| r == rule),
            "{what} must be refused as {rule}; it said {rules:?}"
        );
    }
}

/// A well-formed program parses and reports nothing.
///
/// The row that makes the rest mean something: every assertion above is a
/// refusal, and all of them would hold against a parser that refused
/// everything. It also pins the collector itself — a collector that swallowed
/// diagnostics it should not have would show up here as a program that "parsed"
/// while reporting.
#[test]
fn a_well_formed_program_parses_and_reports_nothing() {
    let (parsed, diagnostics) = collecting_diagnostics(|| {
        parse_source(
            Path::new("main.mfb"),
            "main.mfb",
            "IMPORT io\n\nFUNC main() AS Integer\n  io::print(\"ok\")\n  RETURN 0\nEND FUNC\n",
        )
    });
    assert!(parsed.is_ok(), "a well-formed program must parse");
    assert!(
        diagnostics.is_empty(),
        "a well-formed program must report nothing; it said {diagnostics:?}"
    );
}

/// The collector is scoped: what it takes inside, it does not take outside.
///
/// Not a formality. The slot is a thread-local, and a collector that leaked
/// would silence every later diagnostic on the same thread — including the ones
/// another test is asserting on, which is the kind of failure that shows up as
/// an unrelated suite going green.
#[test]
fn the_collector_only_collects_inside_its_scope() {
    let bad = "FUNC main() AS Integer\n  LET 5 = 1\n  RETURN 0\nEND FUNC\n";
    let (_, inside) =
        collecting_diagnostics(|| parse_source(Path::new("main.mfb"), "main.mfb", bad));
    assert!(!inside.is_empty(), "the scope must collect");

    // A second scope starts empty rather than inheriting the first's.
    let (_, again) =
        collecting_diagnostics(|| parse_source(Path::new("main.mfb"), "main.mfb", bad));
    assert_eq!(
        inside.len(),
        again.len(),
        "each scope collects its own run, not the accumulation of every run \
         before it"
    );
}
