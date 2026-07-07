//! Assertion builtins for the built-in test framework (plan-18-testing.md §1).
//!
//! `expectEQ`/`expectNQ`/`expectTrap`/`expectNTrap` are compiler-lowered: they are
//! recognized here, type-checked in `syntaxcheck`, and lowered directly in
//! `src/ir/lower.rs` (there is no runtime helper). They are valid only inside a
//! `TCASE` body — placement is enforced by `crate::testing` before any other
//! front-end pass.

/// `expectEQ(actual, expected)` — pass iff `actual == expected`.
pub(crate) const EXPECT_EQ: &str = "expectEQ";
/// `expectNQ(actual, expected)` — pass iff `actual != expected`.
pub(crate) const EXPECT_NQ: &str = "expectNQ";
/// `expectTrap(expr)` / `expectTrap(expr, code)` — pass iff evaluating `expr`
/// traps (and, with `code`, the trap's `error.code == code`).
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

/// Whether `name` is one of the four assertion builtins.
pub(crate) fn is_expect_call(name: &str) -> bool {
    matches!(name, EXPECT_EQ | EXPECT_NQ | EXPECT_TRAP | EXPECT_NTRAP)
}

/// The `(min, max)` argument count accepted by an assertion builtin.
pub(crate) fn expect_arity(name: &str) -> Option<(usize, usize)> {
    match name {
        EXPECT_EQ | EXPECT_NQ => Some((2, 2)),
        EXPECT_TRAP => Some((1, 2)),
        EXPECT_NTRAP => Some((1, 1)),
        _ => None,
    }
}
