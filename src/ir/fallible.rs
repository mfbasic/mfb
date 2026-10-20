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
//!   `TYPE_INLINE_TRAP_DEAD_HANDLER` warning (`len`, `typeName`, the total
//!   `bits::*` ops, and the non-allocating pure-query / default-returning
//!   collection members). `toString` is not among them: every overload builds a
//!   `String` and can raise `ErrOutOfMemory` (owner decision 2026-10-04). That
//!   census is keyed on the callee name **and** the call site's argument types:
//!
//!   - `replace` is fallible on its `String` overload (an empty needle) and on
//!     its `List` one (it allocates); an untyped site answers fallible.
//!   - `toInt` runs the other way (bug-679). The name is fallible — a bad parse,
//!     an overflow — and exactly one overload is total: over an enum it is the
//!     ordinal the value already holds, a register move that declares no error
//!     (plan-140-B). That one overload is granted.
//!
//!   Every question asked here therefore carries argument types, and the `toInt`
//!   half additionally carries a `TypeKinds` oracle, since an enum is not its own
//!   `ParameterType`. A site that cannot supply either falls back to the
//!   name-keyed answer, which for both is the over-approximating side and merely
//!   imprecise.
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
//! a desugar that under-reports miscompiles. For the built-ins this census
//! answers, audit reads the same fact it does — the member's declared errors —
//! so there they agree by construction (bug-811).
//!
//! Separate also means a rule about one overload has to be taught to BOTH, in
//! each one's own terms. bug-679 is the worked example: fixing the census here
//! silenced the dead-handler warning's half of it and left `mfb audit` reporting
//! the same `toInt(<enum>)` chain as fallible, because that census reads its own
//! hand-curated name list over the AST and had never been given the call's
//! arguments at all. A change to one is a prompt to check the other.

use super::lower::{expression_type, LowerContext};
use crate::codegen::builtins;
use crate::hir::{HirCallArg, HirConstructorArg, HirExpression, HirItem, HirProject, HirStatement};
use crate::operators::{BinaryOp, UnaryOp};
use crate::types::ParameterType;
use std::collections::{HashMap, HashSet};

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
    ///
    /// `arg_types` carries the call site's argument types so the built-in census
    /// can answer for an overload rather than a name, for the names listed in
    /// `builtins::inline_builtin_fallibility_depends_on_args` (`replace`, bug-533;
    /// `toInt`, bug-679). `toString` is no longer one of them: bug-486 made its
    /// `List OF Byte` overload fallible (`ErrEncoding`), and since bug-807 every
    /// overload is fallible by name — each builds a new `String` and can raise
    /// `ErrOutOfMemory` (owner decision 2026-10-04). A site that cannot type its
    /// arguments passes an empty slice and gets the name-keyed verdict, which for
    /// both listed names is the fallible (over-approximating) side.
    ///
    /// `kinds` is the declared-kind oracle (plan-140-A), needed because one
    /// overload's verdict turns on the argument being an `ENUM` rather than on its
    /// `ParameterType`, which spells every declared type the same: `toInt(<enum>)`
    /// is the ordinal the value already holds and cannot fail, while every other
    /// `toInt` can (bug-679). `NoTypeKinds` answers the over-approximating side.
    pub(super) fn call_is_fallible(
        &self,
        target: &str,
        arg_types: &[ParameterType],
        kinds: &dyn builtins::TypeKinds,
    ) -> bool {
        if builtins::inline_builtin_is_infallible(target, arg_types, kinds) {
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
///
/// `context` is the type oracle. It is needed because the built-in census answers
/// per *overload* for a few names (`builtins::inline_builtin_fallibility_depends_on_args`):
/// `toInt(<enum>)` is total where every other `toInt` can fail (bug-679), and a
/// comparison raises only on the operand types `comparison_can_raise` names
/// (bug-816), so this pass has to type what it walks. The
/// context is positioned on each function's own file before its body is walked,
/// so a package-qualified name resolves through that file's import bindings.
pub(super) fn analyze(hir: &HirProject, context: &mut LowerContext<'_>) -> Fallibility {
    // Paired with the owning file index so the oracle can be positioned on it.
    let functions: Vec<(usize, &crate::hir::HirFunction)> = hir
        .files
        .iter()
        .enumerate()
        .flat_map(|(index, file)| {
            file.items.iter().filter_map(move |item| match item {
                HirItem::Function(function) => Some((index, function)),
                _ => None,
            })
        })
        .collect();

    let declared: HashSet<String> = functions
        .iter()
        .map(|(_, function)| function.name.clone())
        .collect();
    let mut verdicts = Fallibility {
        fallible: HashSet::new(),
        declared,
    };

    loop {
        let mut changed = false;
        for (file_index, function) in &functions {
            if verdicts.fallible.contains(&function.name) {
                continue;
            }
            let block = match &function.trap {
                Some(trap) => &trap.body,
                None => &function.body,
            };
            context.current_imports = hir.files[*file_index].import_bindings();
            // The parameters are the seed of the local type map; `statement_escapes`
            // extends it as it walks. An argument the map cannot type answers
            // `Unknown`, which lands on the name-keyed verdict.
            let mut scope = EscapeScope {
                verdicts: &verdicts,
                locals: function
                    .params
                    .iter()
                    .map(|param| (param.name.clone(), param.type_.clone()))
                    .collect(),
                context,
            };
            if block_escapes(block, &mut scope) {
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

/// The walk state [`analyze`]'s escape rules carry: the verdicts so far, the
/// local type map built as the body is walked, and the type oracle that reads it.
///
/// The map exists only so the built-in census can be asked per *overload*
/// (bug-486). It is deliberately partial — a name it never saw types as `Unknown`
/// and the census falls back to its name-keyed answer, which is what this walk
/// did before types entered it at all.
struct EscapeScope<'a, 'c> {
    verdicts: &'a Fallibility,
    locals: HashMap<String, ParameterType>,
    context: &'a LowerContext<'c>,
}

impl EscapeScope<'_, '_> {
    /// The type this walk can prove for `expression`, `Unknown` when it cannot.
    fn type_of(&self, expression: &HirExpression) -> ParameterType {
        expression_type(expression, &self.locals, self.context).unwrap_or(ParameterType::Unknown)
    }

    /// [`Fallibility::call_is_fallible`] with the call site's argument types
    /// resolved through the local map.
    ///
    /// Gated: this runs on every call node of every function on every pass of the
    /// fixpoint, while only the handful of names in
    /// `inline_builtin_fallibility_depends_on_args` can act on the types. For
    /// every other callee the verdict is name-decided, so the typing is skipped.
    fn call_is_fallible(&self, callee: &str, arguments: &[HirCallArg]) -> bool {
        if !builtins::inline_builtin_fallibility_depends_on_args(callee) {
            return self
                .verdicts
                .call_is_fallible(callee, &[], self.context.type_index());
        }
        let arg_types: Vec<ParameterType> = arguments
            .iter()
            .map(|argument| match argument {
                HirCallArg::Positional(value) | HirCallArg::Named { value, .. } => {
                    self.type_of(value)
                }
            })
            .collect();
        self.verdicts
            .call_is_fallible(callee, &arg_types, self.context.type_index())
    }

    /// Record a binding this walk just introduced, so a later `toString(name)`
    /// can be typed. An explicit annotation wins; otherwise the initializer is
    /// typed with what is already known.
    fn bind(&mut self, name: &str, declared: &ParameterType, value: Option<&HirExpression>) {
        let type_ = if *declared == ParameterType::Unknown {
            value
                .map(|v| self.type_of(v))
                .unwrap_or(ParameterType::Unknown)
        } else {
            declared.clone()
        };
        self.locals.insert(name.to_string(), type_);
    }
}

/// The arithmetic operators (`mfb spec language operators` §11). Every one of
/// them is *checked* on an integer-family result — the spec's "checked numeric
/// failures from operators are ordinary failures and therefore auto-propagate
/// unless handled by a `TRAP`" — so each is a raise site the inline-`TRAP`
/// desugar has to cover (bug-471). The logical (`AND`/`OR`/`XOR`/`NOT`) and
/// string-concatenation operators produce no domain error and are deliberately
/// absent; a comparison raises only while promoting into `Fixed`, which
/// [`comparison_can_raise`] judges from the operand types.
///
/// `&` (string concatenation) is the one case worth spelling out: it allocates
/// a new `String` and so CAN raise `ErrOutOfMemory` at runtime, same as any
/// other allocation. It is excluded from this census anyway, on purpose, not
/// by oversight — this is the operator twin of `toString`'s fallibility
/// (`mfb spec language error-model` rule 11; `src/codegen/builtins/mod.rs`'s
/// `inline_builtin_is_infallible`), except decided the OTHER way: owner
/// decision 2026-10-04 (bug-814's open question) was that making every
/// concatenation fallible would force an error path through nearly all
/// text-building code, so `&` stays out of TRAP-handler-liveness,
/// function-fallibility, and declared-errors accounting. A failed allocation
/// inside `&` still raises `ErrOutOfMemory` at runtime — the exception is in
/// what the compiler counts as a raise site, not in what the operator can
/// actually do — so a `TRAP` on an expression whose only "fallible" node is a
/// bare `&` is flagged `TYPE_INLINE_TRAP_DEAD_HANDLER` same as a provably-total
/// call, and `mfb audit`'s source-level census does not walk into a `&` node
/// looking for a raise site either (`src/audit/collect/source.rs`'s
/// `Expression::Binary` handling recurses for nested *calls* only). No other
/// operator allocates: MFBASIC has no infix operator on `List`/`Map`/`Set` (all
/// collection construction goes through `collections::*` calls), so `&` is the
/// entire exception, not an example of a larger class.
const ARITHMETIC_OPERATORS: [BinaryOp; 7] = [
    BinaryOp::Add,
    BinaryOp::Subtract,
    BinaryOp::Multiply,
    BinaryOp::Divide,
    BinaryOp::IntDiv,
    BinaryOp::Mod,
    BinaryOp::Power,
];

/// The six comparison operators. They yield `Boolean` and have no domain of
/// their own, but they promote their operands exactly as arithmetic does (§4.1),
/// and promotion into `Fixed` is checked — see [`comparison_can_raise`].
const COMPARISON_OPERATORS: [BinaryOp; 6] = [
    BinaryOp::Equal,
    BinaryOp::NotEqual,
    BinaryOp::Less,
    BinaryOp::LessEqual,
    BinaryOp::Greater,
    BinaryOp::GreaterEqual,
];

/// Whether a comparison can raise while promoting its operands (bug-816).
///
/// A comparison converts its operands to the arithmetic promotion type (§4.1),
/// so a `Fixed` compared with an `Integer` or a `Float` converts that operand to
/// `Fixed` first, and the conversion is checked: `ErrOverflow` outside the
/// `Fixed` range, `ErrInvalidFormat` for a non-finite `Float`. `Byte` always
/// fits, and every other pairing compares without converting into `Fixed`.
///
/// The operand types are whatever the caller can prove; `Unknown` is read as
/// "could be any numeric type", the over-approximating side, exactly as
/// [`operator_can_raise`] over-approximates — a redundant check is harmless,
/// an omitted one lets the error escape an inline `TRAP`.
pub(crate) fn comparison_can_raise(
    op: BinaryOp,
    left: &ParameterType,
    right: &ParameterType,
) -> bool {
    let may_be_fixed =
        |type_: &ParameterType| matches!(type_, ParameterType::Fixed | ParameterType::Unknown);
    let checked_into_fixed = |type_: &ParameterType| {
        matches!(
            type_,
            ParameterType::Integer | ParameterType::Float | ParameterType::Unknown
        )
    };
    COMPARISON_OPERATORS.contains(&op)
        && ((may_be_fixed(left) && checked_into_fixed(right))
            || (may_be_fixed(right) && checked_into_fixed(left)))
}

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
/// (`builder_numeric/operator_lowering.rs:lower_numeric_unary_negation`):
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

/// Whether a `Float` operator's only failure is at the *observation boundary*
/// (bug-798): `+`, `-`, `*`, `/` and `DIV` never raise when evaluated, and a
/// transient non-finite result is allowed to recover before anything observes it
/// (`mfb spec language types`). Float negation is the unary member. `MOD` and `^`
/// are not in it: they raise `ErrFloatDomain` at the operator itself.
pub(super) fn float_operator_raises_only_when_observed(
    op: BinaryOp,
    result_type: &ParameterType,
) -> bool {
    matches!(result_type, ParameterType::Float)
        && matches!(
            op,
            BinaryOp::Add
                | BinaryOp::Subtract
                | BinaryOp::Multiply
                | BinaryOp::Divide
                | BinaryOp::IntDiv
        )
}

/// [`float_operator_raises_only_when_observed`] for a unary node: `Float`
/// negation flips the sign bit and has no check of its own.
pub(super) fn float_unary_raises_only_when_observed(
    op: UnaryOp,
    result_type: &ParameterType,
) -> bool {
    op == UnaryOp::Negate && matches!(result_type, ParameterType::Float)
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
///
/// A plain loop, not `.any()`: each statement may extend `scope`'s local type map
/// for the ones after it, so the walk has to reach them. Short-circuiting on the
/// first escape is still correct — the verdict is already decided.
fn block_escapes(body: &[HirStatement], scope: &mut EscapeScope<'_, '_>) -> bool {
    for statement in body {
        if statement_escapes(statement, scope) {
            return true;
        }
    }
    false
}

fn statement_escapes(statement: &HirStatement, scope: &mut EscapeScope<'_, '_>) -> bool {
    match statement {
        HirStatement::Fail { .. } | HirStatement::Propagate { .. } => true,
        HirStatement::Let {
            name, type_, value, ..
        } => {
            let escapes = value.as_ref().is_some_and(|v| expression_escapes(v, scope));
            // Bound after the initializer is judged: the initializer cannot see
            // the name it defines.
            scope.bind(name, type_, value.as_ref());
            escapes
        }
        HirStatement::Return { value, .. } | HirStatement::Recover { value, .. } => {
            value.as_ref().is_some_and(|v| expression_escapes(v, scope))
        }
        HirStatement::Exit { code, .. } => {
            code.as_ref().is_some_and(|v| expression_escapes(v, scope))
        }
        HirStatement::Continue { .. } => false,
        HirStatement::Assign { value, .. }
        | HirStatement::StateAssign { value, .. }
        | HirStatement::Expression {
            expression: value, ..
        } => expression_escapes(value, scope),
        HirStatement::If {
            condition,
            then_body,
            else_body,
            ..
        } => {
            expression_escapes(condition, scope)
                || block_escapes(then_body, scope)
                || block_escapes(else_body, scope)
        }
        HirStatement::Match {
            expression, cases, ..
        } => {
            if expression_escapes(expression, scope) {
                return true;
            }
            for case in cases {
                // A union pattern binds a name at a known type — the last binder
                // this walk would otherwise leave `Unknown`, so a byte decode over
                // a matched variant is typed like one over a parameter.
                if let crate::hir::HirMatchPattern::Union { type_, binding } = &case.pattern {
                    scope.locals.insert(binding.clone(), type_.clone());
                }
                if case
                    .guard
                    .as_ref()
                    .is_some_and(|guard| expression_escapes(guard, scope))
                    || block_escapes(&case.body, scope)
                {
                    return true;
                }
            }
            false
        }
        // A counted loop can always raise: its increment is checked arithmetic
        // (`ErrOverflow` past the counter type's range, a non-finite `Float`
        // counter at its assignment boundary), and its bound and step tests
        // promote like any comparison (bug-816). The IR census answers the same
        // (`trap_init`'s `IrOp::For`).
        HirStatement::For { .. } => true,
        HirStatement::ForEach {
            name,
            iterable,
            body,
            ..
        } => {
            let escapes = expression_escapes(iterable, scope);
            // The element binding, so an overload-sensitive call or a raising
            // comparison over it inside the loop is typed like one over a
            // parameter.
            let element = match scope.type_of(iterable) {
                ParameterType::ListOf(element) | ParameterType::SetOf(element) => *element,
                _ => ParameterType::Unknown,
            };
            scope.locals.insert(name.clone(), element);
            escapes || block_escapes(body, scope)
        }
        HirStatement::While {
            condition, body, ..
        }
        | HirStatement::DoUntil {
            body, condition, ..
        } => expression_escapes(condition, scope) || block_escapes(body, scope),
    }
}

/// Whether evaluating `expression` can let an error escape.
///
/// An inline `TRAP` (`HirExpression::Trapped`) is a barrier in exactly one
/// direction: the trapped expression's own errors are routed to the handler, so
/// only what the *handler* can raise escapes (bug-280's rule, applied to the
/// nested-call shape this analysis now has to see through).
fn expression_escapes(expression: &HirExpression, scope: &mut EscapeScope<'_, '_>) -> bool {
    match expression {
        HirExpression::String(_)
        | HirExpression::Number(_)
        | HirExpression::Scalar(_)
        | HirExpression::Boolean(_)
        | HirExpression::Identifier(_) => false,
        // bug-471: a RAISING OPERATOR lets an error escape exactly as a failing
        // call does — `FUNC fltDiv(a AS Float, b AS Float) AS Float / RETURN a / b`
        // fails with `ErrFloatOverflow`, and every caller must be told so. An
        // arithmetic operator is judged by its spelling alone: it only
        // type-checks on numeric operands, so the spelling already implies the
        // type (`AND`/`OR`/`XOR` are `Boolean`-only, `&` is `String`-only). A
        // comparison raises only while promoting an `Integer`/`Float` operand
        // into `Fixed` (bug-816), so it is judged on its operand types.
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
                || comparison_can_raise(*operator, &scope.type_of(left), &scope.type_of(right))
                || expression_escapes(left, scope)
                || expression_escapes(right, scope)
        }
        HirExpression::Unary {
            operand, operator, ..
        } => {
            (*operator == UnaryOp::Negate && !matches!(operand.as_ref(), HirExpression::Number(_)))
                || expression_escapes(operand, scope)
        }
        HirExpression::Call {
            callee, arguments, ..
        } => {
            scope.call_is_fallible(callee, arguments)
                || arguments.iter().any(|argument| match argument {
                    HirCallArg::Positional(value) | HirCallArg::Named { value, .. } => {
                        expression_escapes(value, scope)
                    }
                })
        }
        // A lambda body runs at the callback's call site, not here; the member
        // that invokes it is judged on its own name.
        HirExpression::Lambda { .. } => false,
        HirExpression::Constructor { arguments, .. } => {
            arguments.iter().any(|argument| match argument {
                HirConstructorArg::Positional(value) | HirConstructorArg::Named { value, .. } => {
                    expression_escapes(value, scope)
                }
            })
        }
        HirExpression::WithUpdate { target, updates } => {
            expression_escapes(target, scope)
                || updates
                    .iter()
                    .any(|update| expression_escapes(&update.value, scope))
        }
        HirExpression::ListLiteral(values) => values.iter().any(|v| expression_escapes(v, scope)),
        HirExpression::SetLiteral { elements, .. } => {
            elements.iter().any(|v| expression_escapes(v, scope))
        }
        HirExpression::MapLiteral { entries, .. } => entries
            .iter()
            .any(|(k, v)| expression_escapes(k, scope) || expression_escapes(v, scope)),
        HirExpression::MemberAccess { target, .. } => expression_escapes(target, scope),
        HirExpression::Trapped { handler, .. } => block_escapes(handler, scope),
    }
}
