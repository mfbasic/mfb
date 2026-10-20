//! plan-176-N: control-flow facts the resource checks need — an inline trap's scrutinee,
//! whether a value can raise, whether a block always returns and whether a match is exhaustive.
//! Resource-move checking lives in `ir/verify/resources/mod.rs`.

use super::*;

impl TypeEnv {
    /// The inline-TRAP scrutinee rules on the lowered shape: `Bind $trap_resN =
    /// <scrutinee>` then `If ResultIsOk($trap_resN)`. The scrutinee survives as
    /// the temp's bind value — a `CallResult` when the source trapped a call, a
    /// `Checked` for any other expression (bug-471, plan-165-D). A scrutinee
    /// that cannot fail draws the advisory `TYPE_INLINE_TRAP_DEAD_HANDLER`. Source
    /// lowering produces nothing else since plan-165-D; a `CallResult` of a
    /// package constant or any other bind value (`TYPE_INLINE_TRAP_REQUIRES_FALLIBLE`)
    /// can only come from a crafted or stale package, whose `$trap_res` would
    /// otherwise not be a `Result` at all.
    ///
    /// `expectTrap`/`expectNTrap` desugar (`testing::desugar`) into this same
    /// shape, but the former source checker saw them as assertion calls with their own rule
    /// (`TESTING_EXPECT_TRAP_REQUIRES_FALLIBLE`), so a handler that sets an
    /// `$expect_` temp is skipped.
    pub(in crate::ir::verify) fn check_inline_trap_scrutinee(
        &self,
        condition: &IrValue,
        handler: &[IrOp],
        temp_consts: &HashMap<&str, &IrValue>,
        locals: &HashMap<String, ParameterType>,
    ) {
        let IrValue::ResultIsOk { value } = condition else {
            return;
        };
        let IrValue::Local(res) = value.as_ref() else {
            return;
        };
        if !res.starts_with("$trap_res") {
            return;
        }
        // Every assertion desugar's handler touches an `$expect_` temp: it sets
        // `$expect_trapped` (expectTrap) or reads the `$expect_err` binding the
        // lowering then binds (expectNTrap's `FAIL error(…, err.message)`).
        if handler.iter().any(|op| {
            matches!(op, IrOp::Assign { name, .. } | IrOp::Bind { name, .. } if name.starts_with("$expect_"))
        }) {
            return;
        }
        let Some(scrutinee) = temp_consts.get(res.as_str()) else {
            return;
        };
        match scrutinee {
            // bug-471: a `Checked` scrutinee is a non-call expression the desugar
            // wraps so its domain-error exits join the capture. plan-165-D (Option
            // 1): any well-typed expression may be trapped, so one that holds no
            // raising operator — `a & b`, `n = 3`, a plain read — cannot fail and
            // its handler is dead code, exactly as on a provably total call.
            IrValue::Checked { value, .. } => {
                if !self.checked_value_can_raise(value, locals) {
                    self.emit(
                        "TYPE_INLINE_TRAP_DEAD_HANDLER",
                        "Inline TRAP handler is unreachable — this expression cannot fail, so the handler is dead code."
                            .to_string(),
                    );
                }
            }
            IrValue::CallResult { target, args, .. } | IrValue::Call { target, args, .. } => {
                if builtins::is_package_constant(target) {
                    self.emit(
                        "TYPE_INLINE_TRAP_REQUIRES_FALLIBLE",
                        "Inline TRAP requires a fallible call; a package constant is not a call."
                            .to_string(),
                    );
                } else if builtins::inline_builtin_is_infallible(
                    target,
                    // bug-486: with the argument types, so an overload whose
                    // verdict differs from its name's is judged as the overload it
                    // is — advising the author to delete a live handler is advising
                    // them to delete the only thing between the error and a dead
                    // process. Typed only for the names whose verdict can turn on
                    // them.
                    &if builtins::inline_builtin_fallibility_depends_on_args(target) {
                        args.iter()
                            .map(|arg| {
                                self.infer_type(arg, locals)
                                    .unwrap_or(ParameterType::Unknown)
                            })
                            .collect::<Vec<_>>()
                    } else {
                        Vec::new()
                    },
                    // bug-679: `self` IS the declared-kind oracle (`impl TypeKinds
                    // for TypeEnv`, plan-140-A), so `toInt(<enum>)` is recognised
                    // as the one infallible `toInt` overload and its dead handler
                    // is reported. Every other `toInt`
                    // can really fail and keeps its handler.
                    self,
                ) {
                    // A provably-infallible inline built-in (`len`, `typeName`,
                    // every total `bits::*`, …) under a TRAP compiles and runs; its
                    // handler is dead code — an advisory warning, not an error
                    // (plan-26-A).
                    self.emit(
                        "TYPE_INLINE_TRAP_DEAD_HANDLER",
                        format!(
                            "Inline TRAP handler is unreachable — `{target}` cannot fail, so the handler is dead code."
                        ),
                    );
                }
            }
            _ => self.emit(
                "TYPE_INLINE_TRAP_REQUIRES_FALLIBLE",
                "Inline TRAP requires a call to trap; this expression is not a call.".to_string(),
            ),
        }
    }

    /// Whether a `Checked` value (call-free, `check_checked_has_no_call`) holds
    /// an operator that can raise: the same per-operator census `ir::lower`
    /// lifts on (`operator_can_raise`, `comparison_can_raise` on the operand
    /// types, `unary_operator_can_raise` minus a negative literal). `&` is never
    /// a raise site (owner decision 2026-10-04). An operand of unproven type
    /// counts as raising for a comparison, as the census does.
    fn checked_value_can_raise(
        &self,
        value: &IrValue,
        locals: &HashMap<String, ParameterType>,
    ) -> bool {
        use crate::ir::fallible::{
            comparison_can_raise, is_total_literal_negation, operator_can_raise,
            unary_operator_can_raise,
        };
        let mut raises = false;
        crate::ir::value::visit_value(value, &mut |node| match node {
            IrValue::Binary {
                op,
                type_,
                left,
                right,
                ..
            } => {
                let operand = |operand: &IrValue| {
                    self.infer_type(operand, locals)
                        .unwrap_or(ParameterType::Unknown)
                };
                raises |= operator_can_raise(*op, type_)
                    || comparison_can_raise(*op, &operand(left), &operand(right));
            }
            IrValue::Unary {
                op, type_, operand, ..
            } => {
                let literal = matches!(operand.as_ref(), IrValue::Const { .. })
                    && is_total_literal_negation(*op, type_);
                raises |= unary_operator_can_raise(*op, type_) && !literal;
            }
            _ => {}
        });
        raises
    }

    /// Whether every path through `ops` leaves the function (mirrors
    /// the former source checker's `Flow::AlwaysReturns`): a Return/Fail/ExitProgram op, an If
    /// whose both branches return, a MATCH with an unguarded CASE ELSE whose
    /// every arm returns, or a TRAP whose body returns. Loops never count
    /// (they may run zero times).
    pub(in crate::ir::verify) fn block_always_returns(
        &self,
        ops: &[IrOp],
        locals: &HashMap<String, ParameterType>,
    ) -> bool {
        let mut locals = locals.clone();
        for op in ops {
            match op {
                IrOp::Return { .. } | IrOp::Fail { .. } | IrOp::ExitProgram { .. } => return true,
                // A stray RECOVER (outside any inline-TRAP handler — an error the
                // shape pass reports) lowers to a `$recover_stray` bind; the front
                // end's flow analysis treats every RECOVER as diverging, so the
                // rules built on this predicate must too.
                IrOp::Bind { name, .. } if name.starts_with("$recover_stray") => return true,
                IrOp::If {
                    then_body,
                    else_body,
                    ..
                } => {
                    if self.block_always_returns(then_body, &locals)
                        && self.block_always_returns(else_body, &locals)
                    {
                        return true;
                    }
                }
                IrOp::Match { value, cases, .. } => {
                    // Exhaustive = an unguarded CASE ELSE, or full enum/union
                    // coverage by unguarded arms (mirroring the relocated
                    // exhaustiveness rule, which rejects anything else).
                    let has_else = cases.iter().any(|case| {
                        case.guard.is_none()
                            && matches!(case.pattern, super::super::super::IrMatchPattern::Else)
                    });
                    let exhaustive = has_else || self.match_covers_all(value, cases, &locals);
                    if exhaustive
                        && cases
                            .iter()
                            .all(|case| self.block_always_returns(&case.body, &locals))
                    {
                        return true;
                    }
                }
                // A function-level `TRAP` is the error *handler* for the
                // preceding statements; on the success path control falls
                // through it without executing the handler. So a trailing
                // `Trap` never makes the block always-return — only the ops
                // *before* it (a success-path `RETURN`) can. The handler
                // returning is irrelevant to fall-through.
                IrOp::Trap { .. } => {}
                IrOp::Bind { name, type_, .. } => {
                    locals.insert(name.clone(), type_.clone());
                }
                _ => {}
            }
        }
        false
    }

    /// Whether the unguarded arms of a MATCH cover every member/variant of its
    /// enum/union scrutinee (the coverage half of `check_match_exhaustive`).
    pub(in crate::ir::verify) fn match_covers_all(
        &self,
        value: &IrValue,
        cases: &[super::super::super::IrMatchCase],
        locals: &HashMap<String, ParameterType>,
    ) -> bool {
        let Some(ty) = self.infer_type(value, locals) else {
            return false;
        };
        let ty = resource_base_type(&ty);
        let all = if let Some(variants) = self.union_variants(&ty) {
            variants
        } else if let Some(members) = self.enums.get(&ty) {
            members.clone()
        } else {
            return false;
        };
        let (covered, has_unguarded_else) = super::super::fold_match_coverage(cases);
        if has_unguarded_else {
            return true;
        }
        all.difference(&covered).next().is_none()
    }
}
