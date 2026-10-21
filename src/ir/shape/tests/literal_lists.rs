//! `ir::shape` on list literals and byte lists:
//!
//! - a list literal of suffixed decimals (`1.5F` Fixed, `1.25m` Money) passed
//!   where a list of the other decimal is expected is a mismatch, element by
//!   element — the literal's own type is read off its suffix;
//! - a `List OF Byte` is printable, so `expectEqual` over two of them reports
//!   no `TESTING_EXPECT_NOT_PRINTABLE`.

use crate::testutil::check_src_details;

fn rules_with(src: &str, rule: &str) -> Vec<String> {
    check_src_details(src)
        .into_iter()
        .filter(|d| d.rule == rule)
        .map(|d| d.detail)
        .collect()
}

#[test]
fn a_suffixed_decimal_list_literal_does_not_cross_decimal_kinds() {
    let src = "\
FUNC takeMoney(xs AS List OF Money) AS Integer
  RETURN len(xs)
END FUNC
FUNC takeFixed(xs AS List OF Fixed) AS Integer
  RETURN len(xs)
END FUNC
FUNC main() AS Integer
  RETURN takeMoney([1.5F, 2.5F]) + takeFixed([1.25m])
END FUNC
";
    let mismatches = rules_with(src, "TYPE_CALL_ARGUMENT_MISMATCH");
    assert_eq!(
        mismatches,
        [
            "Argument 1 for `takeMoney` has type List OF Fixed, expected List OF Money."
                .to_string(),
            "Argument 1 for `takeFixed` has type List OF Money, expected List OF Fixed."
                .to_string(),
        ]
    );
}

#[test]
fn a_byte_list_is_printable_for_an_equality_assertion() {
    let src = "\
FUNC same(a AS List OF Byte) AS Integer
  expectEqual(a, a)
  RETURN 0
END FUNC
FUNC other(m AS Map OF String TO Integer) AS Integer
  expectEqual(m, m)
  RETURN 0
END FUNC
FUNC main() AS Integer
  RETURN 0
END FUNC
";
    let not_printable = rules_with(src, "TESTING_EXPECT_NOT_PRINTABLE");
    assert_eq!(
        not_printable.len(),
        2,
        "the Map's two operands, never the byte list's: {not_printable:?}"
    );
    assert!(
        not_printable
            .iter()
            .all(|detail| detail.ends_with("got Map OF String TO Integer.")),
        "{not_printable:?}"
    );
}
