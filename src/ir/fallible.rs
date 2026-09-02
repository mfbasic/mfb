//! Which calls can raise an error the caller has to handle (bug-457).
//!
//! `ir::lower`'s inline-`TRAP` desugar has to know, for every call inside the
//! trapped expression, whether that call can fail: a fallible one needs its own
//! `CallResult` + `If ResultIsOk` check routed to the handler, while an
//! infallible one must stay a plain `Call` so the overwhelmingly common
//! single-call shape lowers byte-identically. `ir::shape` asks the same question
//! to reject the one shape the desugar cannot cover (a fallible call in a
//! short-circuited operand).
//!
//! The verdict is a **safe over-approximation**: a call is fallible unless it is
//! *proven* otherwise. Two proofs exist.
//!
//! * A built-in whose inline lowering can raise no domain error —
//!   [`builtins::inline_builtin_is_infallible`], the same census that backs the
//!   `TYPE_INLINE_TRAP_DEAD_HANDLER` warning (`len`, `toString`, `typeName`,
//!   the total `bits::*` ops, and the pure-query / default-returning /
//!   growth-only collection and string members).
//! * A function declared in this project whose body cannot let an error escape,
//!   decided by the fixpoint in [`analyze`].
//!
//! Everything else — an imported package's export, a native built-in with a
//! runtime failure seam (`io::print` raises `ErrOutput`), a call through a
//! `FUNC`-typed value — is treated as fallible. Over-approximating only ever
//! adds a check whose error branch is dead; under-approximating would silently
//! drop the error on the floor, which is exactly the bug this exists for.
//!
//! This is the *lowering* oracle. `audit/collect/source.rs:fallible_functions`
//! answers a related question over the AST for `mfb audit`'s reporting, where a
//! hand-curated per-package census is tuned to avoid over-reporting to a human.
//! The two are deliberately separate: a report that over-reports is noisy, while
//! a desugar that under-reports miscompiles.

use crate::codegen::builtins;
use crate::hir::{HirCallArg, HirConstructorArg, HirExpression, HirItem, HirProject, HirStatement};
use crate::operators::{BinaryOp, UnaryOp};
use crate::types::ParameterType;
use std::collections::HashSet;

/// The project's fallibility verdicts, consulted by target name.
#[derive(Default)]
pub(super) struct Fallibility {
    /// Names of project functions that can let an error reach their caller.
    /// Overloads share a name and a call site carries no types here, so a name
    /// is fallible when *any* overload of it is — conservative, never missing.
    fallible: HashSet<String>,
    /// Every function name declared in this project, so a name that is absent
    /// from `fallible` can be told apart from a name this analysis never saw
    /// (an import, a built-in) and therefore cannot vouch for.
    declared: HashSet<String>,
}

impl Fallibility {
    /// Whether a call to `target` can raise an error its caller must handle.
    ///
    /// `target` is the LOWERED callee spelling, not the source one: dot-qualified
    /// for a package member (`strings.mid`, `io.print`), bare for a project
    /// function or a general built-in (`inner`, `len`), and **overload-mangled**
    /// for a project function that overloads another (`len$Ring`). That last
    /// point is why the built-in census can be asked first: a user
    /// `FUNC len(r AS Ring)` overloading the general `len`
    /// (`tests/rt-behavior/functions/func_override_len_user`) reaches here as
    /// `len$Ring`, never as bare `len`, so a *failing* override cannot be
    /// mistaken for the infallible built-in of the same source name. Verified by
    /// dumping the `-ir` of that shape, not assumed.
    pub(super) fn call_is_fallible(&self, target: &str) -> bool {
        if builtins::inline_builtin_is_infallible(target) {
            return false;
        }
        if self.declared.contains(target) {
            return self.fallible.contains(target);
        }
        true
    }
}

/// Compute the fallibility verdicts for `hir`.
///
/// A function is fallible when its *relevant block* — the function-level `TRAP`
/// handler when it has one, else the body, since a trapped body's errors are
/// routed to that handler — can `FAIL`, `PROPAGATE`, or call something fallible.
/// The rule is applied to a fixpoint so fallibility propagates up call chains,
/// then re-applied once per declaration so an overload whose name a sibling
/// already marked is still judged on its own body.
pub(super) fn analyze(hir: &HirProject) -> Fallibility {
    let functions: Vec<&crate::hir::HirFunction> = hir
        .files
        .iter()
        .flat_map(|file| file.items.iter())
        .filter_map(|item| match item {
            HirItem::Function(function) => Some(function),
            _ => None,
        })
        .collect();

    let declared: HashSet<String> = functions
        .iter()
        .map(|function| function.name.clone())
        .collect();
    let mut verdicts = Fallibility {
        fallible: HashSet::new(),
        declared,
    };

    loop {
        let mut changed = false;
        for function in &functions {
            if verdicts.fallible.contains(&function.name) {
                continue;
            }
            let block = match &function.trap {
                Some(trap) => &trap.body,
                None => &function.body,
            };
            if block_escapes(block, &verdicts) {
                verdicts.fallible.insert(function.name.clone());
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    verdicts
}

/// The arithmetic operators (`mfb spec language operators` §11). Every one of
/// them is *checked* on an integer-family result — the spec's "checked numeric
/// failures from operators are ordinary failures and therefore auto-propagate
/// unless handled by a `TRAP`" — so each is a raise site the inline-`TRAP`
/// desugar has to cover (bug-471). The logical (`AND`/`OR`/`XOR`/`NOT`),
/// comparison and string-concatenation operators produce no domain error and are
/// deliberately absent.
const ARITHMETIC_OPERATORS: [BinaryOp; 7] = [
    BinaryOp::Add,
    BinaryOp::Subtract,
    BinaryOp::Multiply,
    BinaryOp::Divide,
    BinaryOp::IntDiv,
    BinaryOp::Mod,
    BinaryOp::Power,
];

/// The unary half of [`ARITHMETIC_OPERATORS`]: `-` is the only unary operator
/// with a numeric domain, so the only one that can raise. `NOT` is
/// `Boolean`-only and `SIZEOF` folds to a constant before lowering.
const ARITHMETIC_UNARY_OPERATORS: [UnaryOp; 1] = [UnaryOp::Negate];

/// Whether an operator node can raise a domain error while the expression it
/// sits in is being evaluated (bug-471).
///
/// This is the operator twin of [`Fallibility::call_is_fallible`] and, like it,
/// a deliberate **over-approximation**: it answers from the operator spelling
/// and the node's own result type rather than arm-by-arm against
/// `codegen::engine::operators`. A recogniser kept in lockstep with a per-arm
/// census is exactly the shape that silently loses an arm when the census grows
/// (`Money`'s dispatcher, `Byte`'s underflow, `Fixed`'s `MOD` divisor check are
/// three separate code paths already), and here an over-approximation costs one
/// redundant always-`Ok` check inside a trapped expression while an
/// under-approximation is the miscompile this exists to close.
///
/// The result type is the discriminator because arithmetic only type-checks on
/// numeric operands (`AND`/`OR` are `Boolean`-only, `&` is `String`-only — §11),
/// so an arithmetic node's result type is numeric exactly when its operands
/// were. `Float` counts: plan-17 moved `+`/`-`/`*`/`/`'s check from the operator
/// to the observation boundary, but that boundary still sits *inside* the
/// trapped expression (a `Float` overflow nested in a trapped call escaped the
/// handler exactly like an integer one before this fix), and `MOD`/`^` raise
/// `ErrFloatDomain` at the operator itself.
pub(super) fn operator_can_raise(op: BinaryOp, result_type: &ParameterType) -> bool {
    ARITHMETIC_OPERATORS.contains(&op)
        && matches!(
            result_type,
            ParameterType::Byte
                | ParameterType::Integer
                | ParameterType::Fixed
                | ParameterType::Money
                | ParameterType::Float
        )
}

/// The one exemption to [`operator_can_raise`]: a unary `-` whose operand is a
/// numeric **literal** is the spelling of a negative literal, not a computed
/// negation, and cannot raise. Callers apply it only once they have established
/// that the operand is a constant.
///
/// The parser produces `Unary(-, Const n)` for every negative numeric literal
/// (unary `-` binds at the unary tier — `mfb spec language operators`), so this
/// is by far the commonest operator inside a trapped expression: `f(-1) TRAP(e)`
/// would otherwise pay a whole `Checked` + `Result` materialization to check a
/// negation that provably succeeds.
///
/// Why it provably succeeds, per the codegen arm
/// (`builder_numeric.rs:lower_numeric_unary_negation`):
///
/// * `Integer`/`Fixed`/`Money` negate through `emit_min_i64_negation_check`,
///   which raises `ErrOverflow` only on exactly `i64::MIN`. `n` here is the
///   *non-negative* half of a negative literal, and the one spelling that would
///   produce `i64::MIN` — `-9223372036854775808` — never reaches this shape:
///   lowering folds it to a single `Const "-9223372036854775808"` (measured by
///   dumping its `-ir`, since `9223372036854775808` has no positive `Integer`
///   representation to hold).
/// * `Float` negation flips the sign bit and emits no check at all.
/// * `Byte` is **excluded** rather than reasoned about: its negation raises
///   `ErrUnderflow` for any non-zero operand. A negative literal types as
///   `Integer` and is rejected against a `Byte` parameter
///   (`TYPE_CALL_ARGUMENT_MISMATCH`), so the shape is unreachable from source —
///   but "unreachable" is a weaker guarantee than "cannot raise", and this is
///   the side of the line where being wrong is a miscompile.
pub(super) fn is_total_literal_negation(op: UnaryOp, result_type: &ParameterType) -> bool {
    op == UnaryOp::Negate && !matches!(result_type, ParameterType::Byte)
}

/// [`operator_can_raise`] for a unary node. Split from the binary form because
/// the two arities are separate vocabularies: before the split both shared one
/// `&str` list in which `"-"` stood for subtraction *and* negation at once.
pub(super) fn unary_operator_can_raise(op: UnaryOp, result_type: &ParameterType) -> bool {
    ARITHMETIC_UNARY_OPERATORS.contains(&op)
        && matches!(
            result_type,
            ParameterType::Byte
                | ParameterType::Integer
                | ParameterType::Fixed
                | ParameterType::Money
                | ParameterType::Float
        )
}

/// Whether a block can let an error escape to the caller.
fn block_escapes(body: &[HirStatement], verdicts: &Fallibility) -> bool {
    body.iter()
        .any(|statement| statement_escapes(statement, verdicts))
}

fn statement_escapes(statement: &HirStatement, verdicts: &Fallibility) -> bool {
    match statement {
        HirStatement::Fail { .. } | HirStatement::Propagate { .. } => true,
        HirStatement::Let { value, .. } => value
            .as_ref()
            .is_some_and(|v| expression_escapes(v, verdicts)),
        HirStatement::Return { value, .. } | HirStatement::Recover { value, .. } => value
            .as_ref()
            .is_some_and(|v| expression_escapes(v, verdicts)),
        HirStatement::Exit { code, .. } => code
            .as_ref()
            .is_some_and(|v| expression_escapes(v, verdicts)),
        HirStatement::Continue { .. } => false,
        HirStatement::Assign { value, .. }
        | HirStatement::StateAssign { value, .. }
        | HirStatement::Expression {
            expression: value, ..
        } => expression_escapes(value, verdicts),
        HirStatement::If {
            condition,
            then_body,
            else_body,
            ..
        } => {
            expression_escapes(condition, verdicts)
                || block_escapes(then_body, verdicts)
                || block_escapes(else_body, verdicts)
        }
        HirStatement::Match {
            expression, cases, ..
        } => {
            expression_escapes(expression, verdicts)
                || cases.iter().any(|case| {
                    case.guard
                        .as_ref()
                        .is_some_and(|guard| expression_escapes(guard, verdicts))
                        || block_escapes(&case.body, verdicts)
                })
        }
        HirStatement::For {
            start,
            end,
            step,
            body,
            ..
        } => {
            expression_escapes(start, verdicts)
                || expression_escapes(end, verdicts)
                || step
                    .as_ref()
                    .is_some_and(|step| expression_escapes(step, verdicts))
                || block_escapes(body, verdicts)
        }
        HirStatement::ForEach { iterable, body, .. } => {
            expression_escapes(iterable, verdicts) || block_escapes(body, verdicts)
        }
        HirStatement::While {
            condition, body, ..
        }
        | HirStatement::DoUntil {
            body, condition, ..
        } => expression_escapes(condition, verdicts) || block_escapes(body, verdicts),
    }
}

/// Whether evaluating `expression` can let an error escape.
///
/// An inline `TRAP` (`HirExpression::Trapped`) is a barrier in exactly one
/// direction: the trapped expression's own errors are routed to the handler, so
/// only what the *handler* can raise escapes (bug-280's rule, applied to the
/// nested-call shape this analysis now has to see through).
fn expression_escapes(expression: &HirExpression, verdicts: &Fallibility) -> bool {
    match expression {
        HirExpression::String(_)
        | HirExpression::Number(_)
        | HirExpression::Scalar(_)
        | HirExpression::Boolean(_)
        | HirExpression::Identifier(_) => false,
        // bug-471: a RAISING OPERATOR lets an error escape exactly as a failing
        // call does — `FUNC fltDiv(a AS Float, b AS Float) AS Float / RETURN a / b`
        // fails with `ErrFloatOverflow`, and every caller must be told so. Judged
        // by the operator spelling alone: this walk has no types, and an
        // arithmetic operator only type-checks on numeric operands, so the
        // spelling already implies the type (`AND`/`OR`/`XOR` are `Boolean`-only,
        // `&` is `String`-only, comparisons yield `Boolean` — none are listed).
        //
        // The `Unary` arm keeps the negative-literal exemption for the same
        // reason `lower::trap_hoist_kind` does: `RETURN -1` must not make a
        // function fallible. Its `Number` operand is the parser's spelling of the
        // literal, not a computed negation
        // (see `is_total_literal_negation`).
        HirExpression::Binary {
            left,
            operator,
            right,
            ..
        } => {
            ARITHMETIC_OPERATORS.contains(operator)
                || expression_escapes(left, verdicts)
                || expression_escapes(right, verdicts)
        }
        HirExpression::Unary {
            operand, operator, ..
        } => {
            (*operator == UnaryOp::Negate && !matches!(operand.as_ref(), HirExpression::Number(_)))
                || expression_escapes(operand, verdicts)
        }
        HirExpression::Call {
            callee, arguments, ..
        } => {
            verdicts.call_is_fallible(callee)
                || arguments.iter().any(|argument| match argument {
                    HirCallArg::Positional(value) | HirCallArg::Named { value, .. } => {
                        expression_escapes(value, verdicts)
                    }
                })
        }
        // A lambda body runs at the callback's call site, not here; the member
        // that invokes it is judged on its own name.
        HirExpression::Lambda { .. } => false,
        HirExpression::Constructor { arguments, .. } => {
            arguments.iter().any(|argument| match argument {
                HirConstructorArg::Positional(value) | HirConstructorArg::Named { value, .. } => {
                    expression_escapes(value, verdicts)
                }
            })
        }
        HirExpression::WithUpdate { target, updates } => {
            expression_escapes(target, verdicts)
                || updates
                    .iter()
                    .any(|update| expression_escapes(&update.value, verdicts))
        }
        HirExpression::ListLiteral(values) => {
            values.iter().any(|v| expression_escapes(v, verdicts))
        }
        HirExpression::SetLiteral { elements, .. } => {
            elements.iter().any(|v| expression_escapes(v, verdicts))
        }
        HirExpression::MapLiteral { entries, .. } => entries
            .iter()
            .any(|(k, v)| expression_escapes(k, verdicts) || expression_escapes(v, verdicts)),
        HirExpression::MemberAccess { target, .. } => expression_escapes(target, verdicts),
        HirExpression::Trapped { handler, .. } => block_escapes(handler, verdicts),
    }
}
