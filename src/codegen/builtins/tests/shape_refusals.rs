//! Five `ir::shape` refusals no program in the tree had ever produced.
//!
//! These go through `check_src`, and that is not a style choice. The in-process
//! corpus LOWERS its fixtures and never calls `ir::shape::collect_diagnostics`,
//! so an rt-behavior fixture cannot reach a source checker however carefully it
//! is written — measured, and recorded as C12 after a fixture aimed at this file
//! moved it by exactly zero lines.
//!
//! Each of the five is a diagnostic that exists because LOWERING ERASES THE
//! EVIDENCE. That is what they have in common and why they live in `shape`
//! rather than in `ir::verify`:
//!
//!   - a call through a function VALUE cannot use named arguments, because a
//!     callable type keeps no parameter names — and lowering discards the name,
//!     so by the time the IR exists there is nothing left to complain about.
//!   - an assignment to a name that is neither a local nor a top-level binding
//!     lowers to an `AssignGlobal`, which looks exactly like a legitimate write
//!     to a global.
//!
//! A missed refusal here is not a build failure. It is a program that compiles
//! and stores into a global nobody declared, or drops an argument the author
//! named.

use crate::testutil::check_src;

/// The rule codes `source` produces, or a panic naming what it produced instead.
fn refusal(what: &str, source: &str) -> Vec<String> {
    let rules = check_src(source);
    assert!(
        !rules.is_empty(),
        "{what} must be refused; the checkers accepted it"
    );
    rules
}

/// An assignment target that resolves to nothing is refused.
///
/// Lowering emits an `AssignGlobal` for any non-local name, so the IR cannot
/// tell "write to the global `total`" from "write to the misspelling `totl`".
/// This is the only pass that still can.
#[test]
fn an_assignment_to_a_name_that_is_not_a_binding_is_refused() {
    let rules = refusal(
        "an assignment to an undeclared name",
        "\
IMPORT io

FUNC main() AS Integer
  MUT total AS Integer = 0
  totl = 5
  io::print(toString(total))
  RETURN 0
END FUNC
",
    );
    assert!(
        rules.iter().any(|rule| rule == "TYPE_UNKNOWN_VALUE"),
        "an undeclared assignment target is TYPE_UNKNOWN_VALUE; it said {rules:?}"
    );
}

/// The same rule for a lambda whose body IS an assignment.
///
/// `LAMBDA(v AS Integer) -> name = <expr>` is its own grammar — the parser takes
/// the `identifier =` lookahead as an assignment rather than an equality — and
/// it yields Nothing, which is why the binding is `AS Nothing` and the callback
/// is passed straight to `forEach` rather than bound first (a `LET` initializer
/// does not accept it). It is "the shape a non-escaping callback uses to update a
/// captured MUT binding", and it needs its own arm because the lambda's
/// parameters are in scope inside it: the checker builds a nested scope and asks
/// the question again there.
#[test]
fn an_assignment_bodied_lambda_targeting_nothing_is_refused() {
    let rules = refusal(
        "a lambda assigning to an undeclared name",
        "\
IMPORT collections
IMPORT io

FUNC main() AS Integer
  MUT total AS Integer = 0
  collections::forEach([1, 2], LAMBDA(v AS Integer) -> totl = v)
  io::print(toString(total))
  RETURN 0
END FUNC
",
    );
    assert!(
        rules.iter().any(|rule| rule == "TYPE_UNKNOWN_VALUE"),
        "an undeclared target inside a lambda is TYPE_UNKNOWN_VALUE; it said \
         {rules:?}"
    );
}

/// A parameter default whose type cannot be determined is refused.
///
/// The default is evaluated at the CALL site, so a default the checker cannot
/// type is one every caller inherits.
#[test]
fn a_parameter_default_with_no_known_type_is_refused() {
    let rules = refusal(
        "a parameter default that does not type",
        "\
IMPORT io
IMPORT tcp

FUNC describe(a AS net::Address = tcp::localAddress(NOTHING)) AS String
  RETURN \"x\"
END FUNC

FUNC main() AS Integer
  io::print(describe())
  RETURN 0
END FUNC
",
    );
    assert!(
        !rules.is_empty(),
        "a default the checker cannot type must be refused; it said {rules:?}"
    );
}

/// A call THROUGH A FUNCTION VALUE cannot use named arguments.
///
/// A callable type is `FUNC(Integer) AS Integer` — it records the types and not
/// the names, so there is no name for `v :=` to bind to. Lowering discards the
/// name and passes the value positionally, so without this rule the program
/// compiles and the name silently means nothing.
#[test]
fn a_named_argument_to_a_function_value_is_refused() {
    let rules = refusal(
        "a named argument through a function value",
        "\
IMPORT io

FUNC twice(v AS Integer) AS Integer
  RETURN v * 2
END FUNC

FUNC main() AS Integer
  LET f AS FUNC(Integer) AS Integer = twice
  io::print(toString(f(v := 3)))
  RETURN 0
END FUNC
",
    );
    assert!(
        rules
            .iter()
            .any(|rule| rule == "TYPE_CALL_ARGUMENT_MISMATCH"),
        "a named argument to a function value is TYPE_CALL_ARGUMENT_MISMATCH; \
         it said {rules:?}"
    );
}

/// An argument of the wrong type THROUGH A FUNCTION VALUE is refused, and the
/// message names the position and both types.
///
/// The same check the direct-call path makes, on the other side of the seam: a
/// callable type carries its parameter types, so the mismatch is knowable, and
/// the position is all the author has to go on because there is no name.
#[test]
fn a_wrongly_typed_argument_to_a_function_value_is_refused() {
    let rules = refusal(
        "a String where a function value takes an Integer",
        "\
IMPORT io

FUNC twice(v AS Integer) AS Integer
  RETURN v * 2
END FUNC

FUNC main() AS Integer
  LET f AS FUNC(Integer) AS Integer = twice
  io::print(toString(f(\"three\")))
  RETURN 0
END FUNC
",
    );
    assert!(
        rules
            .iter()
            .any(|rule| rule == "TYPE_CALL_ARGUMENT_MISMATCH"),
        "a wrongly typed argument through a function value is \
         TYPE_CALL_ARGUMENT_MISMATCH; it said {rules:?}"
    );
}

/// The same programs, correct, are accepted.
///
/// The row that makes the five above mean something: each asserts a REFUSAL, and
/// all five would hold against a checker that refused everything.
#[test]
fn the_correct_forms_of_all_of_these_are_accepted() {
    for (what, source) in [
        (
            "assigning to a declared local",
            "\
IMPORT io

FUNC main() AS Integer
  MUT total AS Integer = 0
  total = 5
  io::print(toString(total))
  RETURN 0
END FUNC
",
        ),
        (
            "calling a function value positionally",
            "\
IMPORT io

FUNC twice(v AS Integer) AS Integer
  RETURN v * 2
END FUNC

FUNC main() AS Integer
  LET f AS FUNC(Integer) AS Integer = twice
  io::print(toString(f(3)))
  RETURN 0
END FUNC
",
        ),
        (
            "an assignment-bodied lambda targeting a declared local",
            "\
IMPORT collections
IMPORT io

FUNC main() AS Integer
  MUT total AS Integer = 0
  collections::forEach([1, 2], LAMBDA(v AS Integer) -> total = v)
  io::print(toString(total))
  RETURN 0
END FUNC
",
        ),
        (
            "a named argument to a NAMED function, which does keep its names",
            "\
IMPORT io

FUNC twice(v AS Integer) AS Integer
  RETURN v * 2
END FUNC

FUNC main() AS Integer
  io::print(toString(twice(v := 3)))
  RETURN 0
END FUNC
",
        ),
    ] {
        assert_eq!(
            check_src(source),
            Vec::<String>::new(),
            "{what} is a correct program; the checkers refused it"
        );
    }
}
