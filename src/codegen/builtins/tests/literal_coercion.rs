//! The literal coercions a CONSTANT ARGUMENT enjoys, through the SOURCE CHECKER.
//!
//! `ir::shape::expression_compatible` accepts three things `compatible` does
//! not, and only for a literal written at the call site:
//!
//!   - an in-range integer literal where a `Byte` is expected
//!   - a numeric literal, NEGATED OR NOT, where a `Fixed` or `Money` is expected
//!   - a LIST LITERAL of such literals where a list of them is expected
//!
//! The last one classifies each element with `numeric_literal_type`, whose four
//! arms are Integer, Float, Fixed and Money, plus a fifth for a negated literal
//! — a `Unary` node wrapping a `Number`, and not a `Number` at all.
//!
//! **This is a `check_src` suite and not a fixture, and that distinction is the
//! whole reason it exists.** `tests/rt-behavior/lexical/
//! list-literal-numeric-coercion-rt` was written for these arms and moved
//! `ir/shape.rs` by nothing at all: the in-process corpus LOWERS its fixtures
//! (`try_code_for_src` -> `lower_augmented_project`) and never runs the source
//! checkers, so no rt-behavior fixture can reach `ir::shape` however carefully
//! it is written. The checkers are reached by the `tests/syntax` diagnostic
//! corpus and by `check_src` — this. The fixture stays, because the VALUES it
//! asserts are worth pinning and lowering is where a wrong coercion produces a
//! wrong number; this covers the decision that lets it compile.
//!
//! A missed arm here is a REFUSAL: `expression_compatible` returning false is
//! TYPE_CALL_ARGUMENT_MISMATCH on a program the language accepts.

use crate::testutil::{accepts, check_src};

/// The declarations every case calls into.
const PRELUDE: &str = "\
IMPORT collections
IMPORT io

FUNC totalFixed(xs AS List OF Fixed) AS Fixed
  RETURN collections::sum(xs)
END FUNC

FUNC oneFixed(x AS Fixed) AS Fixed
  RETURN x
END FUNC

FUNC oneMoney(x AS Money) AS Money
  RETURN x
END FUNC

FUNC oneByte(b AS Byte) AS Integer
  RETURN toInt(b)
END FUNC

";

fn program(body: &str) -> String {
    format!("{PRELUDE}FUNC main() AS Integer\n{body}  RETURN 0\nEND FUNC\n")
}

/// Every accepted literal shape, one call per row.
///
/// Each is a program the language accepts and a stricter `compatible` would
/// refuse. They are asserted one at a time rather than in one program because
/// the checker reports every mismatch it finds, so a single refusal in a
/// combined program would not say which row caused it.
#[test]
fn a_literal_argument_coerces_to_the_parameter_it_is_written_for() {
    for (what, call) in [
        ("an integer literal into Fixed", "oneFixed(2)"),
        ("a float literal into Fixed", "oneFixed(1.25)"),
        ("a NEGATED literal into Fixed", "oneFixed(-1.25)"),
        ("an integer literal into Money", "oneMoney(3)"),
        ("a float literal into Money", "oneMoney(0.25)"),
        ("a NEGATED literal into Money", "oneMoney(-0.25)"),
        ("zero into Byte", "oneByte(0)"),
        ("the top of the Byte range", "oneByte(255)"),
        (
            "a list of integer literals into List OF Fixed",
            "totalFixed([1, 2, 3])",
        ),
        (
            "a list of float literals into List OF Fixed",
            "totalFixed([1.5, 2.5])",
        ),
        (
            "a list mixing signs into List OF Fixed",
            "totalFixed([-1.25, 1.25, -2])",
        ),
    ] {
        let source = program(&format!("  io::print(toString({call}))\n"));
        assert!(
            accepts(&source),
            "{what} is a coercion the language allows at a call site; the \
             checker refused it with {:?}",
            check_src(&source)
        );
    }
}

/// The coercion is for a LITERAL, and only at the call site.
///
/// The guard that makes the rows above mean something: a stricter
/// `expression_compatible` would refuse the accepted shapes, and a looser one
/// would accept these — an `Integer`-typed VARIABLE is not a literal, and no
/// amount of context makes it a `Fixed`.
#[test]
fn a_variable_is_not_a_literal_and_does_not_coerce() {
    for (what, body) in [
        (
            "an Integer local into Fixed",
            "  LET n AS Integer = 2\n  io::print(toString(oneFixed(n)))\n",
        ),
        (
            "an Integer local into Byte",
            "  LET n AS Integer = 2\n  io::print(toString(oneByte(n)))\n",
        ),
        (
            "a list of Integer LOCALS into List OF Fixed",
            "  LET n AS Integer = 2\n  io::print(toString(totalFixed([n, n])))\n",
        ),
    ] {
        let source = program(body);
        let rules = check_src(&source);
        assert!(
            !rules.is_empty(),
            "{what} must be refused: the coercion is a property of the literal \
             written at the call site, not of the type it happens to have"
        );
    }
}

/// An integer literal past the top of the `Byte` range is refused.
///
/// The range check is the one part of the Byte arm that is not "accept it": a
/// coercion that took any integer literal would put 256 into a byte and lose
/// the 1.
#[test]
fn an_out_of_range_byte_literal_is_refused() {
    let source = program("  io::print(toString(oneByte(256)))\n");
    assert!(
        !check_src(&source).is_empty(),
        "256 does not fit in a Byte, so the literal coercion must not take it"
    );
}
