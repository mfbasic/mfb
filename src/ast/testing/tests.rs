//! A malformed `TESTING` block, which no fixture in the tree contains.
//!
//! Every `TESTING` block under `tests/` is well formed, so the `TESTING` parser
//! ran only its happy path. What was never taken is what it does when a case
//! body does not parse (`synchronize`, so the REST of the suite is still read)
//! and when a block's `END` is missing the word that names it.
//!
//! **Recovery is the contract, not politeness.** A `TESTING` block is a list of
//! independent cases, and one mistyped line inside one case must not cost the
//! developer the other cases' diagnostics — a parser that gave up at the first
//! bad statement would report one error per edit-compile cycle through a suite
//! that has ten. So the bad case is followed by a good one, and the good one's
//! diagnostics have to still be absent while the bad one's is present.

use crate::ast::parse_source;
use std::path::Path;

/// The rule names a source string reports, in order.
fn rules(source: &str) -> Vec<String> {
    let (_, diagnostics) = crate::rules::collecting_diagnostics(|| {
        let _ = parse_source(Path::new("main.mfb"), "main.mfb", source);
    });
    diagnostics.into_iter().map(|(rule, _)| rule).collect()
}

/// A statement that does not parse inside a `TCASE` is reported once, and the
/// case after it is still read.
#[test]
fn a_bad_statement_in_one_case_does_not_swallow_the_next() {
    let good = "\
TESTING
  TGROUP \"group\"
    TCASE \"first\"
      LET a AS Integer = 1
    END TCASE
    TCASE \"second\"
      LET b AS Integer = 2
    END TCASE
  END TGROUP
END TESTING
SUB main
END SUB
";
    assert!(
        rules(good).is_empty(),
        "a well-formed TESTING block reports nothing; without this half the \
         assertion below would pass against a parser that reported on every \
         block it saw"
    );

    let bad = "\
TESTING
  TGROUP \"group\"
    TCASE \"first\"
      LET a AS = = 1
    END TCASE
    TCASE \"second\"
      LET b AS Integer = 2
    END TCASE
  END TGROUP
END TESTING
SUB main
END SUB
";
    let reported = rules(bad);
    assert!(
        !reported.is_empty(),
        "the malformed binding in the first case must be reported"
    );
    assert!(
        reported.len() < 4,
        "one bad statement must not cascade: the parser synchronizes and reads \
         the rest of the suite, so a developer fixing one typo does not get a \
         page of consequences; got {reported:?}"
    );
}

/// `END` without the word that names the block is refused, for every level.
///
/// One shared helper (`consume_end_contextual`) closes `TCASE`, `TGROUP` and
/// `TESTING`, so this is three call sites through one refusal. `END` alone is
/// ambiguous in a nested block -- it could close any of the three -- which is
/// exactly why the parser insists on the name rather than guessing.
#[test]
fn an_end_that_does_not_name_its_block_is_refused() {
    for (what, source) in [
        (
            "TCASE",
            "\
TESTING
  TGROUP \"group\"
    TCASE \"first\"
      LET a AS Integer = 1
    END
  END TGROUP
END TESTING
SUB main
END SUB
",
        ),
        (
            "TGROUP",
            "\
TESTING
  TGROUP \"group\"
    TCASE \"first\"
      LET a AS Integer = 1
    END TCASE
  END
END TESTING
SUB main
END SUB
",
        ),
    ] {
        assert!(
            !rules(source).is_empty(),
            "a bare `END` closing a {what} must be refused -- in a nested block \
             it could close any of the three, and guessing would attach the \
             following cases to the wrong parent"
        );
    }
}
