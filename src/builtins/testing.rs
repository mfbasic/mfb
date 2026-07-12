//! Assertion builtins for the built-in test framework (plan-18-testing.md §1).
//!
//! The assertion builtins are compiler-lowered: they are recognized here,
//! type-checked in `syntaxcheck`, and lowered directly in `src/ir/lower.rs`
//! (there is no runtime helper). They are valid only inside a `TCASE` body —
//! placement is enforced by `crate::testing` before any other front-end pass.

/// `expectEqual(actual, expected)` — pass iff `actual = expected`. Generic: any
/// `=`-comparable, printable operands.
pub(crate) const EXPECT_EQUAL: &str = "expectEqual";
/// `expectNEqual(actual, expected)` — pass iff `actual <> expected`. Generic.
pub(crate) const EXPECT_NEQUAL: &str = "expectNEqual";
/// `expectFloat(actual, expected)` — both operands must be `Float`; pass iff equal.
pub(crate) const EXPECT_FLOAT: &str = "expectFloat";
/// `expectInteger(actual, expected)` — both `Integer`; pass iff equal.
pub(crate) const EXPECT_INTEGER: &str = "expectInteger";
/// `expectFixed(actual, expected)` — both `Fixed`; pass iff equal.
pub(crate) const EXPECT_FIXED: &str = "expectFixed";
/// `expectString(actual, expected)` — both `String`; pass iff equal.
pub(crate) const EXPECT_STRING: &str = "expectString";
/// `expectNFloat(actual, expected)` — both `Float`; pass iff not equal.
pub(crate) const EXPECT_NFLOAT: &str = "expectNFloat";
/// `expectNInteger(actual, expected)` — both `Integer`; pass iff not equal.
pub(crate) const EXPECT_NINTEGER: &str = "expectNInteger";
/// `expectNFixed(actual, expected)` — both `Fixed`; pass iff not equal.
pub(crate) const EXPECT_NFIXED: &str = "expectNFixed";
/// `expectNString(actual, expected)` — both `String`; pass iff not equal.
pub(crate) const EXPECT_NSTRING: &str = "expectNString";
/// `expectTrap(expr)` / `expectTrap(expr, code)` — pass iff evaluating `expr`
/// traps (and, with `code`, the trap's `error.code = code`).
pub(crate) const EXPECT_TRAP: &str = "expectTrap";
/// `expectNTrap(expr)` — pass iff evaluating `expr` does not trap.
pub(crate) const EXPECT_NTRAP: &str = "expectNTrap";

/// The reserved internal error code a failed assertion raises. It sits in the
/// `7-706-*` (trap/failure) subsystem but is deliberately absent from the
/// `errorCode::` registry, so user code can neither name it nor — barring a
/// deliberate `FAIL error(77069001, …)` — collide with it. The synthesized driver
/// recognizes it to distinguish an assertion failure from a genuine runtime error
/// (plan-18-B §3.1).
pub(crate) const TEST_ABORT_CODE: i64 = 77069001;

/// Whether `name` is one of the assertion builtins.
pub(crate) fn is_expect_call(name: &str) -> bool {
    is_equality_assert(name) || is_inequality_assert(name) || matches!(name, EXPECT_TRAP | EXPECT_NTRAP)
}

/// An equality assertion (`actual = expected`): the generic `expectEqual` or a
/// typed `expectFloat`/`expectInteger`/`expectFixed`/`expectString`.
pub(crate) fn is_equality_assert(name: &str) -> bool {
    matches!(
        name,
        EXPECT_EQUAL | EXPECT_FLOAT | EXPECT_INTEGER | EXPECT_FIXED | EXPECT_STRING
    )
}

/// An inequality assertion (`actual <> expected`): the generic `expectNEqual` or
/// a typed `expectNFloat`/`expectNInteger`/`expectNFixed`/`expectNString`.
pub(crate) fn is_inequality_assert(name: &str) -> bool {
    matches!(
        name,
        EXPECT_NEQUAL | EXPECT_NFLOAT | EXPECT_NINTEGER | EXPECT_NFIXED | EXPECT_NSTRING
    )
}

/// The exact operand type a *typed* equality/inequality assertion requires, or
/// `None` for the generic `expectEqual`/`expectNEqual` (any comparable operands).
pub(crate) fn expect_operand_type(name: &str) -> Option<&'static str> {
    match name {
        EXPECT_FLOAT | EXPECT_NFLOAT => Some("Float"),
        EXPECT_INTEGER | EXPECT_NINTEGER => Some("Integer"),
        EXPECT_FIXED | EXPECT_NFIXED => Some("Fixed"),
        EXPECT_STRING | EXPECT_NSTRING => Some("String"),
        _ => None,
    }
}

/// The `(min, max)` argument count accepted by an assertion builtin.
pub(crate) fn expect_arity(name: &str) -> Option<(usize, usize)> {
    if is_equality_assert(name) || is_inequality_assert(name) {
        return Some((2, 2));
    }
    match name {
        EXPECT_TRAP => Some((1, 2)),
        EXPECT_NTRAP => Some((1, 1)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_every_assertion_family_as_an_expect_call() {
        // Equality, inequality, and the two trap families are all `expect*` calls;
        // anything else (an ordinary function name) is not.
        for name in [
            EXPECT_EQUAL,
            EXPECT_FLOAT,
            EXPECT_INTEGER,
            EXPECT_FIXED,
            EXPECT_STRING,
            EXPECT_NEQUAL,
            EXPECT_NFLOAT,
            EXPECT_NINTEGER,
            EXPECT_NFIXED,
            EXPECT_NSTRING,
            EXPECT_TRAP,
            EXPECT_NTRAP,
        ] {
            assert!(is_expect_call(name), "`{name}` should be an expect call");
        }
        assert!(!is_expect_call("print"));
        assert!(!is_expect_call("expectSomethingElse"));
    }

    #[test]
    fn classifies_equality_and_inequality_families() {
        for name in [
            EXPECT_EQUAL,
            EXPECT_FLOAT,
            EXPECT_INTEGER,
            EXPECT_FIXED,
            EXPECT_STRING,
        ] {
            assert!(is_equality_assert(name));
            assert!(!is_inequality_assert(name));
        }
        for name in [
            EXPECT_NEQUAL,
            EXPECT_NFLOAT,
            EXPECT_NINTEGER,
            EXPECT_NFIXED,
            EXPECT_NSTRING,
        ] {
            assert!(is_inequality_assert(name));
            assert!(!is_equality_assert(name));
        }
        // The trap families are neither an equality nor an inequality assertion.
        assert!(!is_equality_assert(EXPECT_TRAP));
        assert!(!is_inequality_assert(EXPECT_NTRAP));
    }

    #[test]
    fn typed_assertions_carry_their_operand_type() {
        assert_eq!(expect_operand_type(EXPECT_FLOAT), Some("Float"));
        assert_eq!(expect_operand_type(EXPECT_NFLOAT), Some("Float"));
        assert_eq!(expect_operand_type(EXPECT_INTEGER), Some("Integer"));
        assert_eq!(expect_operand_type(EXPECT_NINTEGER), Some("Integer"));
        assert_eq!(expect_operand_type(EXPECT_FIXED), Some("Fixed"));
        assert_eq!(expect_operand_type(EXPECT_NFIXED), Some("Fixed"));
        assert_eq!(expect_operand_type(EXPECT_STRING), Some("String"));
        assert_eq!(expect_operand_type(EXPECT_NSTRING), Some("String"));
        // The generic families and non-assertions have no fixed operand type.
        assert_eq!(expect_operand_type(EXPECT_EQUAL), None);
        assert_eq!(expect_operand_type(EXPECT_NEQUAL), None);
        assert_eq!(expect_operand_type("print"), None);
    }

    #[test]
    fn arity_matches_each_assertion_family() {
        assert_eq!(expect_arity(EXPECT_EQUAL), Some((2, 2)));
        assert_eq!(expect_arity(EXPECT_STRING), Some((2, 2)));
        assert_eq!(expect_arity(EXPECT_NEQUAL), Some((2, 2)));
        assert_eq!(expect_arity(EXPECT_NFIXED), Some((2, 2)));
        // `expectTrap` takes the expression plus an optional code; `expectNTrap`
        // takes exactly the expression.
        assert_eq!(expect_arity(EXPECT_TRAP), Some((1, 2)));
        assert_eq!(expect_arity(EXPECT_NTRAP), Some((1, 1)));
        assert_eq!(expect_arity("print"), None);
    }
}
