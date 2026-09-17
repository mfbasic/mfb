//! A built-in member's value-type parameter accepts only that type (bug-638).
//!
//! `datetime::toMillis(at AS datetime::Instant)` compiled when handed a
//! `datetime::DateTime`, and then read the `DateTime`'s first two words as an
//! `Instant`'s `seconds` and `nanos`: `32000` for noon on any day. A user
//! function with the same parameter rejected the same argument. The registry's
//! strict matcher treated every non-resource nominal parameter as accepting
//! every nominal argument, so no built-in record, enum or union parameter was
//! ever checked by identity.
//!
//! These go through `check_src`, which runs `ir::shape` and `ir::verify` in the
//! build's order — the diagnostic the source path reports.

use crate::testutil::check_src;

/// Whether `source` is refused with `TYPE_CALL_ARGUMENT_MISMATCH`.
fn assert_argument_mismatch(what: &str, source: &str) {
    let rules = check_src(source);
    assert!(
        rules
            .iter()
            .any(|rule| rule == "TYPE_CALL_ARGUMENT_MISMATCH"),
        "{what} must be TYPE_CALL_ARGUMENT_MISMATCH; the checkers said {rules:?}"
    );
}

/// Whether `source` is accepted with no diagnostic at all.
fn assert_accepted(what: &str, source: &str) {
    let rules = check_src(source);
    assert!(rules.is_empty(), "{what} must be accepted; it said {rules:?}");
}

/// The bug's own reproduction: a `DateTime`, a `Date` and a `Duration` each
/// passed where `toMillis` takes an `Instant`.
#[test]
fn a_different_datetime_record_is_refused_where_an_instant_is_taken() {
    for (what, argument) in [
        (
            "a DateTime passed to datetime::toMillis",
            "datetime::civil(datetime::date(2026, 3, 7), datetime::time(12, 0), datetime::utc())",
        ),
        (
            "a Date passed to datetime::toMillis",
            "datetime::date(2026, 3, 7)",
        ),
        (
            "a Duration passed to datetime::toMillis",
            "datetime::duration(90)",
        ),
    ] {
        assert_argument_mismatch(
            what,
            &format!(
                "\
IMPORT io
IMPORT datetime

FUNC main() AS Integer
  io::print(toString(datetime::toMillis({argument})))
  RETURN 0
END FUNC
"
            ),
        );
    }
}

/// A user record whose name and fields match a built-in record is still a
/// different type.
#[test]
fn a_user_record_shaped_like_a_builtin_record_is_refused() {
    assert_argument_mismatch(
        "a user `Instant` passed to datetime::toMillis",
        "\
IMPORT io
IMPORT datetime

TYPE Instant
  seconds AS Integer
  nanos AS Integer
END TYPE

FUNC main() AS Integer
  LET at AS Instant = Instant[seconds := 1, nanos := 0]
  io::print(toString(datetime::toMillis(at)))
  RETURN 0
END FUNC
",
    );
}

/// An enum parameter takes only its own enum.
#[test]
fn a_different_builtin_enum_is_refused() {
    assert_argument_mismatch(
        "a crypto::SymmetricCipher passed where crypto::hash takes a crypto::Hash",
        "\
IMPORT io
IMPORT crypto

FUNC main() AS Integer
  LET digest AS List OF Byte = crypto::hash(crypto::SymmetricCipher.AES256GCM, \"abc\")
  io::print(toString(len(digest)))
  RETURN 0
END FUNC
",
    );
}

/// A union parameter takes its variants, and nothing else.
#[test]
fn a_record_that_is_not_a_variant_is_refused_where_a_union_is_taken() {
    assert_argument_mismatch(
        "a datetime::Instant passed where json::stringify takes a json::Json",
        "\
IMPORT io
IMPORT json
IMPORT datetime

FUNC main() AS Integer
  io::print(json::stringify(datetime::now()))
  RETURN 0
END FUNC
",
    );
}

/// `canvas` takes `color::Color`; a `datetime::Instant` is not one.
#[test]
fn a_different_record_is_refused_by_a_canvas_member() {
    assert_argument_mismatch(
        "a datetime::Instant passed where canvas::fill takes a color::Color",
        "\
IMPORT app
IMPORT canvas
IMPORT datetime

FUNC main() AS Integer
  LET paint AS canvas::Paint = canvas::fill(datetime::now())
  RETURN 0
END FUNC
",
    );
}

/// The row that makes the refusals above mean something: the well-typed forms
/// of every call above are accepted, so none of them holds against a checker
/// that refuses everything.
#[test]
fn the_well_typed_forms_are_accepted() {
    assert_accepted(
        "an Instant passed to datetime::toMillis",
        "\
IMPORT io
IMPORT datetime

FUNC main() AS Integer
  LET at AS datetime::DateTime = datetime::civil(datetime::date(2026, 3, 7), datetime::time(12, 0), datetime::utc())
  io::print(toString(datetime::toMillis(datetime::resolve(at))))
  RETURN 0
END FUNC
",
    );
    assert_accepted(
        "a crypto::Hash passed to crypto::hash",
        "\
IMPORT io
IMPORT crypto

FUNC main() AS Integer
  LET digest AS List OF Byte = crypto::hash(crypto::Hash.SHA2_256, \"abc\")
  io::print(toString(len(digest)))
  RETURN 0
END FUNC
",
    );
    assert_accepted(
        "a json::JsonBool variant passed to json::stringify",
        "\
IMPORT io
IMPORT json

FUNC main() AS Integer
  LET value AS json::JsonBool = json::JsonBool[value := TRUE]
  io::print(json::stringify(value))
  RETURN 0
END FUNC
",
    );
    assert_accepted(
        "a color::Color passed to canvas::fill",
        "\
IMPORT app
IMPORT canvas
IMPORT color

FUNC main() AS Integer
  LET paint AS canvas::Paint = canvas::fill(color::rgb(1, 2, 3))
  RETURN 0
END FUNC
",
    );
}
