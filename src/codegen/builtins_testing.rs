//! Assertion builtins for the built-in test framework (plan-18-testing.md §1).
//!
//! The assertion builtins are compiler-lowered: they are recognized here,
//! type-checked in the former source checker, and lowered directly in `src/ir/lower.rs`
//! (there is no runtime helper). They are valid only inside a `TCASE` body —
//! placement is enforced by `crate::testing` before any other front-end pass.
//!
//! Membership (`is_testing_call`) is answered by the clean-room registry's
//! **unqualified-global** `testing` package (`crate::codegen::builtins::testing`):
//! the assertions are bare names (`expectEqual`, never `testing::expectEqual`), so
//! the descriptor lives under a real package name but the calls stay unqualified.
//! The three remaining hand helpers (`is_equality_assert` / `is_inequality_assert` /
//! `expect_operand_type`) and `expect_arity` are front-end classification the desugar
//! consults directly and are not descriptor-derived.

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

/// Whether `name` is one of the assertion builtins. Queries the clean-room
/// registry's unqualified-global `testing` package by bare member name — the
/// assertions are registered there under the real package name `"testing"` even
/// though the calls stay unqualified (`crate::codegen::builtins::testing`).
pub(crate) fn is_testing_call(name: &str) -> bool {
    crate::codegen::registry::registry()
        .resolve_package("testing")
        .is_some_and(|package| package.function(name).is_some())
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
            assert!(is_testing_call(name), "`{name}` should be an expect call");
        }
        assert!(!is_testing_call("print"));
        assert!(!is_testing_call("expectSomethingElse"));
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

    // The clean-room `testing` package is the membership authority; cross-check that
    // `is_testing_call` (which queries it) and `expect_arity` (the surviving hand
    // helper) agree on every assertion name and its arity, and that the package is
    // resolvable and unqualified-global.
    #[test]
    fn registry_backs_membership_and_arity() {
        const NAMES: &[&str] = &[
            EXPECT_EQUAL,
            EXPECT_NEQUAL,
            EXPECT_FLOAT,
            EXPECT_INTEGER,
            EXPECT_FIXED,
            EXPECT_STRING,
            EXPECT_NFLOAT,
            EXPECT_NINTEGER,
            EXPECT_NFIXED,
            EXPECT_NSTRING,
            EXPECT_TRAP,
            EXPECT_NTRAP,
        ];

        let pkg = crate::codegen::registry::registry()
            .resolve_package("testing")
            .expect("clean-room testing package is registered");
        assert!(pkg.is_unqualified_global());
        assert_eq!(pkg.functions().len(), NAMES.len());

        for &name in NAMES {
            assert!(is_testing_call(name), "`{name}` is a testing call");
            let func = pkg
                .function(name)
                .unwrap_or_else(|| panic!("`{name}` missing"));
            assert_eq!(
                func.arity(),
                expect_arity(name),
                "registry arity == expect_arity for `{name}`",
            );
        }
        assert!(!is_testing_call("print"));

        // `expectTrap`'s optional `code` widens arity to (1, 2) but never pads.
        assert_eq!(
            pkg.function(EXPECT_TRAP).and_then(|f| f.arity()),
            Some((1, 2)),
        );
    }
}
