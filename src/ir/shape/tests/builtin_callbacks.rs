//! A bare general built-in passed as a `collections` callback (bug-368): its
//! callable type is derived from the list's element type, so
//!
//! - a collection argument that is not a list has no element to derive it
//!   from, and the call is a mismatch naming the predicate;
//! - a derived type the member still rejects (`isEmpty` returns a Boolean,
//!   `forEach` needs a `Nothing` callback) is a mismatch naming the derived
//!   types.

use crate::testutil::check_src_details;

fn found(src: &str) -> Vec<(String, String)> {
    check_src_details(src)
        .into_iter()
        .map(|d| (d.rule, d.detail))
        .collect()
}

fn mismatches(src: &str) -> Vec<String> {
    found(src)
        .into_iter()
        .filter(|(rule, _)| rule == "TYPE_CALL_ARGUMENT_MISMATCH")
        .map(|(_, detail)| detail)
        .collect()
}

#[test]
fn a_collection_that_is_not_a_list_names_the_predicate() {
    let src = "\
IMPORT collections
FUNC main() AS Integer
  LET a = collections::filter(5, isEmpty)
  RETURN 0
END FUNC
";
    assert_eq!(
        mismatches(src),
        [
            "Call to `collections.filter` has argument type(s) (Integer, isEmpty), \
          expected List OF T, FUNC(T) AS Boolean."
                .to_string()
        ]
    );
}

#[test]
fn a_derived_predicate_type_the_member_rejects_names_the_derived_types() {
    let src = "\
IMPORT collections
FUNC main() AS Integer
  collections::forEach([\"a\"], isEmpty)
  RETURN 0
END FUNC
";
    assert_eq!(
        mismatches(src),
        ["Call to `collections.forEach` has argument type(s) \
          (List OF String, FUNC(String) AS Boolean), expected List OF T, FUNC(T) AS Nothing."
            .to_string()]
    );
}

/// A call through a local that is not a function value has no signature to
/// check arguments against: only the unknown result is reported, never an
/// arity or argument rule.
#[test]
fn a_call_through_a_non_function_local_checks_no_arguments() {
    let src = "FUNC main() AS Integer\n  LET x = 1\n  RETURN x(2, 3)\nEND FUNC\n";
    assert_eq!(super::shape_codes(src), ["TYPE_UNKNOWN_VALUE"]);
}
