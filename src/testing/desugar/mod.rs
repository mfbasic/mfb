//! AST construction for the `mfb test` driver, plus assertion-placement
//! validation (plan-18-B §3.4–3.5).
//!
//! The driver is built directly as MFBASIC AST — an ordinary `FUNC … AS Integer`
//! entry point — so it flows through resolve/typecheck/lowering exactly like
//! hand-written source and needs no new runtime. Each `TCASE` becomes a
//! parameterless `SUB`; the driver calls each under an inline `TRAP`, reads the
//! error to tell an assertion failure (the reserved [`TEST_ABORT_CODE`]) from a
//! genuine runtime error, streams the pass/fail tree, and returns a non-zero exit
//! code iff any case failed.

use crate::ast::{
    AstProject, CallArg, Expression, Function, FunctionKind, Item, Statement, Visibility,
};

// Names of the generated coverage runtime helpers (plan-18-C). Plain (non-sigil)
// names so they lower as ordinary declarations; the `__mfb_cov` prefix makes a
// user collision vanishingly unlikely. Shared: `coverage` emits the helpers and
// `driver` calls the dump/fail entry points.
const COV_ARRAY: &str = "__mfb_cov";
const COV_FAILED: &str = "__mfb_cov_failed";
const COV_HIT: &str = "__mfb_cov_hit";
const COV_FAIL: &str = "__mfb_cov_fail";
const COV_DUMP: &str = "__mfb_cov_dump";
const COV_ZEROS: &str = "__mfb_cov_zeros";
const COV_EMPTY_STRINGS: &str = "__mfb_cov_empty_strings";
use crate::builtins::testing::{is_expect_call, TEST_ABORT_CODE};
use crate::coverage::CovSlot;
use std::path::Path;

mod coverage;
mod driver;
mod expect;
mod placement;

pub(crate) use coverage::instrument_coverage;
pub(crate) use driver::{build_driver, DriverStep};
pub(crate) use expect::{desugar_case_body, expand_expect};
pub(crate) use placement::validate_expect_placement;

fn call_arg_value(argument: &CallArg) -> &Expression {
    match argument {
        CallArg::Positional(value) => value,
        CallArg::Named { value, .. } => value,
    }
}

fn constructor_arg_value(argument: &crate::ast::ConstructorArg) -> &Expression {
    match argument {
        crate::ast::ConstructorArg::Positional(value) => value,
        crate::ast::ConstructorArg::Named { value, .. } => value,
    }
}

#[cfg(test)]
mod tests {
    use super::coverage::instrument_block;
    use super::*;

    #[test]
    fn instruments_inline_trap_handler_lines() {
        // A bare-expression statement carrying an inline `TRAP` whose handler runs
        // one statement at line 42. Instrumenting the block must emit a CovSlot for
        // the handler line so it is not rendered Neutral (bug-93.2).
        let handler = vec![Statement::Expression {
            expression: call("io.print", vec![ident("boom")]),
            line: 42,
        }];
        let mut block = vec![Statement::Expression {
            expression: Expression::Trapped {
                expression: Box::new(call("fs.readText", vec![ident("path")])),
                binding: "err".to_string(),
                handler,
                line: 7,
            },
            line: 7,
        }];
        let mut slots: Vec<CovSlot> = Vec::new();
        instrument_block(&mut block, "app.mfb", &mut slots);
        assert!(
            slots
                .iter()
                .any(|slot| slot.line == 42 && slot.file == "app.mfb"),
            "expected a CovSlot for the inline-TRAP handler line, got {slots:?}"
        );
    }

    use crate::ast::build::*;
    use crate::builtins::testing::{EXPECT_EQUAL, EXPECT_NEQUAL, EXPECT_NTRAP, EXPECT_TRAP};

    /// Positional call arguments from a list of expressions.
    fn pos(values: Vec<Expression>) -> Vec<CallArg> {
        values.into_iter().map(CallArg::Positional).collect()
    }

    #[test]
    fn expand_equality_lowers_to_two_lets_and_a_guarded_fail() {
        // `expectEqual(a, e)` binds both operands, then FAILs when they differ.
        let out = expand_expect(EXPECT_EQUAL, &pos(vec![num(1), num(2)]), 0, 9);
        assert_eq!(out.len(), 3);
        assert!(matches!(out[0], Statement::Let { .. }));
        assert!(matches!(out[1], Statement::Let { .. }));
        // The guard's condition is the *negated* equality (fail on mismatch).
        let Statement::If { condition, .. } = &out[2] else {
            panic!("expected an IF guard");
        };
        assert!(matches!(condition, Expression::Unary { .. }));
    }

    #[test]
    fn expand_inequality_fails_when_operands_are_equal() {
        // `expectNEqual(a, e)` FAILs on a *positive* equality (the two matched).
        let out = expand_expect(EXPECT_NEQUAL, &pos(vec![num(1), num(1)]), 3, 4);
        assert_eq!(out.len(), 3);
        let Statement::If { condition, .. } = &out[2] else {
            panic!("expected an IF guard");
        };
        assert!(matches!(condition, Expression::Binary { .. }));
    }

    #[test]
    fn expand_trap_without_a_code_guards_the_flag() {
        // No expected code → flag init, the guarded expression, and one FAIL when
        // the trap never fired.
        let out = expand_expect(EXPECT_TRAP, &pos(vec![ident("risky")]), 1, 5);
        assert_eq!(out.len(), 3);
        assert!(matches!(out[0], Statement::Let { .. }));
        assert!(matches!(out[1], Statement::Expression { .. }));
        assert!(matches!(out[2], Statement::If { .. }));
    }

    #[test]
    fn expand_trap_with_a_code_checks_both_presence_and_value() {
        // An expected code adds a code capture and two guards (missing + mismatch).
        let out = expand_expect(EXPECT_TRAP, &pos(vec![ident("risky"), num(42)]), 2, 6);
        assert_eq!(out.len(), 6);
        // Two of the emitted statements are the missing/mismatch FAIL guards.
        let guards = out
            .iter()
            .filter(|statement| matches!(statement, Statement::If { .. }))
            .count();
        assert_eq!(guards, 2);
    }

    #[test]
    fn expand_ntrap_is_a_single_trap_whose_handler_fails() {
        let out = expand_expect(EXPECT_NTRAP, &pos(vec![ident("safe")]), 0, 7);
        assert_eq!(out.len(), 1);
        assert!(matches!(out[0], Statement::Expression { .. }));
    }

    #[test]
    fn expand_expect_yields_nothing_for_unknown_or_argless_calls() {
        // An unrecognized callee produces no lowering.
        assert!(expand_expect("notAnAssertion", &pos(vec![num(1)]), 0, 0).is_empty());
        // Missing operands short-circuit every family.
        assert!(expand_expect(EXPECT_EQUAL, &[], 0, 0).is_empty());
        assert!(expand_expect(EXPECT_TRAP, &[], 0, 0).is_empty());
        assert!(expand_expect(EXPECT_NTRAP, &[], 0, 0).is_empty());
    }

    #[test]
    fn validate_flags_a_stray_assertion_outside_a_tcase() {
        // A broad program: `expectEqual` sits in an ordinary FUNC body (illegal) and
        // the walker also descends through every common statement/expression kind.
        let source = "\
TYPE Point
  x AS Integer
  y AS Integer
END TYPE

MUT g AS Integer = 0

FUNC f AS Integer
  MUT total AS Integer = 0
  LET a AS Integer = 1
  LET b AS Integer = 2
  IF total > 0 THEN
    expectEqual(a, b)
  ELSE
    total = 0
  END IF
  MATCH total
    CASE 0 WHEN a > 0
      total = 1
    CASE ELSE
      total = 2
  END MATCH
  FOR i = 1 TO 10 STEP 2
    total = total + i
  NEXT
  FOR EACH item IN [1, 2, 3]
    total = total + item
  NEXT
  WHILE total < 100
    total = total + 1
  END WHILE
  DO
    total = total + 1
  LOOP UNTIL total > 200
  LET lam AS Integer = LAMBDA(x AS Integer) -> x + 1
  LET neg AS Integer = -total
  LET truth AS Boolean = NOT (a = b)
  LET m AS Map OF String TO Integer = Map OF String TO Integer { \"k\" := 1 }
  LET lst AS List OF Integer = [1, 2]
  LET pt AS Point = Point[x := 1, y := 2]
  LET up AS Point = WITH pt { x := 9 }
  LET member AS Integer = pt.x
  LET trapped AS Integer = risky() TRAP(err)
    RECOVER 0
  END TRAP
  EXIT FOR
  CONTINUE FOR
  FAIL \"boom\"
  PROPAGATE
  EXIT PROGRAM 1
  RETURN total
END FUNC
";
        let project = crate::testutil::project_from_src(source);
        assert!(
            validate_expect_placement(&project),
            "an `expectEqual` outside a TCASE must be flagged"
        );
    }

    #[test]
    fn validate_accepts_a_project_with_no_stray_assertions() {
        // The same shape without any `expect*` call is clean; the walker visits the
        // FUNC body (and its function-level TRAP) and finds nothing to report.
        let source = "\
FUNC f AS Integer
  MUT total AS Integer = 0
  IF total > 0 THEN
    total = 1
  END IF
  RETURN total
TRAP(err)
  RETURN 0
END TRAP
END FUNC
";
        let project = crate::testutil::project_from_src(source);
        assert!(!validate_expect_placement(&project));
    }
}
