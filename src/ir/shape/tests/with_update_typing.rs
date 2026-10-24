//! What the shape pass makes of a `WITH` whose target is not a plain record:
//! a map entry keeps its type (only `ir::verify` refuses the update, as
//! read-only), while a target that is no nominal at all leaves the
//! initializer untyped.

use crate::testutil::check_src_details;

fn found(src: &str) -> Vec<(String, usize)> {
    check_src_details(src)
        .into_iter()
        .map(|d| (d.rule, d.line))
        .collect()
}

#[test]
fn a_with_on_a_map_entry_is_typed_and_refused_only_as_read_only() {
    let src = "\
FUNC main() AS Integer
  LET m = Map OF String TO Integer { \"a\" := 1 }
  FOR EACH e IN m
    LET f = WITH e { value := 2 }
  NEXT
  RETURN 0
END FUNC
";
    assert_eq!(
        found(src),
        [("TYPE_READ_ONLY_RECORD_UPDATE".to_string(), 4)]
    );
}

#[test]
fn a_with_on_a_value_that_is_no_nominal_leaves_the_binding_untyped() {
    let src = "\
FUNC main() AS Integer
  LET q = WITH 5 { x := 1 }
  RETURN 0
END FUNC
";
    assert_eq!(found(src), [("TYPE_UNKNOWN_VALUE".to_string(), 2)]);
}
