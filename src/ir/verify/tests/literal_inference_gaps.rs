//! `ir::verify`'s value rules where the checker cannot type an operand, and at
//! the Float literal range edges.
//!
//! An element, key or value the checker cannot type is `Unknown`, and
//! `Unknown` never rejects: a collection literal holding one reports the
//! binding it cannot type, never an element/key/value mismatch on top (no
//! cascade). A Float literal past the Float range is refused with the rule for
//! its sign; one too small to represent is not an error.

use crate::testutil::check_src_details;

fn rules_and_details(src: &str) -> Vec<(String, String)> {
    check_src_details(src)
        .into_iter()
        .map(|d| (d.rule, d.detail))
        .collect()
}

#[test]
fn an_untypeable_collection_element_is_not_a_mismatch() {
    let found = rules_and_details(
        "\
FUNC main() AS Integer
  LET xs AS List OF Integer = [nope, 1]
  LET ys AS Set OF Integer = Set OF Integer { nope2 }
  LET m AS Map OF String TO Integer = Map OF String TO Integer { nope3 := nope4 }
  RETURN 0
END FUNC
",
    );
    assert!(
        !found.iter().any(|(rule, _)| rule.ends_with("_MISMATCH")),
        "an Unknown element, key or value must not be reported as a mismatch: {found:?}"
    );
}

#[test]
fn a_float_literal_past_the_range_is_refused_by_sign() {
    let found = rules_and_details(
        "\
FUNC main() AS Integer
  LET a AS Float = 1e400
  LET b AS Float = -1e400
  LET c AS Float = 1e-400
  RETURN 0
END FUNC
",
    );
    assert_eq!(
        found,
        [
            (
                "TYPE_FLOAT_LITERAL_OVERFLOW".to_string(),
                "Numeric literal `1e400` is outside the Float range.".to_string()
            ),
            (
                "TYPE_FLOAT_LITERAL_UNDERFLOW".to_string(),
                "Numeric literal `-1e400` is outside the Float range.".to_string()
            ),
        ],
        "a literal too small to represent (1e-400) is not an error"
    );
}
