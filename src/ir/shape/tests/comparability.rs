//! `ir::shape`'s comparability and argument-compatibility answers for record
//! fields of the built-in and kind-specific nominals, and for two nominals
//! that share (or do not share) a bare name.
//!
//! - A record holding an `Error` and an enum is comparable; one holding an
//!   `AttributedString` (a list overlay) or a union is not, and neither is a
//!   union as a Set element.
//! - A record passed where a different record is expected is a mismatch, and
//!   so is the built-in `net.Url` where the program's own `Url` is expected —
//!   one bare name, two declarations (bug-41).

use crate::testutil::check_src_details;

fn found(src: &str) -> Vec<(String, String)> {
    check_src_details(src)
        .into_iter()
        .map(|d| (d.rule, d.detail))
        .collect()
}

#[test]
fn comparability_follows_the_fields_kinds() {
    let src = "\
IMPORT collections
IMPORT astrings
ENUM Color
  Red, Green
END ENUM
TYPE WithError
  e AS Error
  c AS Color
END TYPE
TYPE WithAttr
  a AS AttributedString
END TYPE
TYPE Circle
  r AS Integer
END TYPE
TYPE Square
  s AS Integer
END TYPE
UNION Shape
  Circle
  Square
END UNION
TYPE WithShape
  s AS Shape
END TYPE
FUNC area(shapes AS Set OF Shape) AS Integer
  RETURN len(shapes)
END FUNC
FUNC main() AS Integer
  LET a AS List OF WithError = []
  LET b = collections::contains(a, WithError[error(1, \"x\"), Color.Red])
  LET c AS List OF WithAttr = []
  LET d = collections::contains(c, WithAttr[astrings::fromString(\"x\")])
  LET e AS List OF WithShape = []
  LET f = collections::contains(e, WithShape[Circle[1]])
  RETURN 0
END FUNC
";
    let comparable: Vec<String> = found(src)
        .into_iter()
        .filter(|(rule, _)| rule == "TYPE_REQUIRES_COMPARABLE")
        .map(|(_, detail)| detail)
        .collect();
    assert_eq!(
        comparable,
        [
            "Set element type requires a comparable type, got `Shape`.".to_string(),
            "Call to `collections.contains` requires a comparable type, got `WithAttr`."
                .to_string(),
            "Call to `collections.contains` requires a comparable type, got `WithShape`."
                .to_string(),
        ],
        "a record of an Error and an enum is comparable"
    );
}

#[test]
fn a_different_declaration_is_a_mismatch_even_under_one_bare_name() {
    let src = "\
IMPORT net
TYPE Circle
  r AS Integer
END TYPE
TYPE Square
  s AS Integer
END TYPE
TYPE Url
  host AS String
END TYPE
FUNC takeCircle(c AS Circle) AS Integer
  RETURN c.r
END FUNC
FUNC takeUrl(u AS Url) AS Integer
  RETURN 0
END FUNC
FUNC main() AS Integer
  LET a = takeCircle(Square[1])
  LET b = takeUrl(net::toUrl(\"http://x\"))
  RETURN 0
END FUNC
";
    let mismatches: Vec<String> = found(src)
        .into_iter()
        .filter(|(rule, _)| rule == "TYPE_CALL_ARGUMENT_MISMATCH")
        .map(|(_, detail)| detail)
        .collect();
    assert_eq!(
        mismatches,
        [
            "Argument 1 for `takeCircle` has type Square, expected Circle.".to_string(),
            "Argument 1 for `takeUrl` has type net.Url, expected Url.".to_string(),
        ]
    );
}

/// `expectEqual` accepts exactly the `=`-comparable operands: a record of an
/// `Error` and an enum is; records holding an `AttributedString` or a union
/// are not. The assertion builtins are recognized by name, so a plain FUNC body
/// reaches the check without the TESTING desugaring (`mfb test` only).
#[test]
fn an_equality_assertion_follows_the_same_comparability() {
    let src = "\
IMPORT astrings
ENUM Color
  Red, Green
END ENUM
TYPE WithError
  e AS Error
  c AS Color
END TYPE
TYPE WithAttr
  a AS AttributedString
END TYPE
TYPE Circle
  r AS Integer
END TYPE
TYPE Square
  s AS Integer
END TYPE
UNION Shape
  Circle
  Square
END UNION
TYPE WithShape
  s AS Shape
END TYPE
FUNC main AS Integer
  LET e = WithError[error(1, \"x\"), Color.Red]
  expectEqual(e, e)
  LET a = WithAttr[astrings::fromString(\"x\")]
  expectEqual(a, a)
  LET s = WithShape[Circle[1]]
  expectEqual(s, s)
  RETURN 0
END FUNC
";
    let diagnostics = super::collect_diagnostics(
        std::path::Path::new("/proj"),
        &super::hir_from(src),
        &[],
        &[],
        &std::collections::HashMap::new(),
        &[],
    );
    let incomparable: Vec<&str> = diagnostics
        .iter()
        .filter(|d| d.rule == "TESTING_EXPECT_INCOMPARABLE")
        .map(|d| d.detail.as_str())
        .collect();
    assert_eq!(
        incomparable,
        [
            "`expectEqual` operands must be comparable with `=`; got WithAttr and WithAttr.",
            "`expectEqual` operands must be comparable with `=`; got WithShape and WithShape.",
        ]
    );
}
