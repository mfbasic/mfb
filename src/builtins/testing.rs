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
