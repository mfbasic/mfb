//! plan-176-N: IR lowering of statements, statement blocks, and `MATCH` cases and expressions.
//! The lowering context and entry points live in `ir/lower/mod.rs`.

use super::*;

/// plan-121-G: lower the statement, then rewrite any `collections::reduce` /
/// `reduceRight` with a **String accumulator and a recognized self-concat
/// reducer** into the loop it is sugar for.
///
/// `reduce` with a concatenating reducer is O(N²): the reducer is called N times
/// and each call returns a fresh tight `len(acc) + len(x)` string. The identical
/// fold written as a hand loop is O(N), because `a = a & x` on a `MUT String`
/// local is matched by `try_inplace_concat_assign` and appends into a grown
/// buffer. At N = 8000 the two spellings measured **790× apart for the same
/// answer**.
///
/// The cost is *inside* the reducer, so no change to how the fold threads its
/// accumulator can remove it (Correction G1) — the only sound fix is to stop
/// calling the reducer and emit the loop, which the existing concat arm then
/// optimizes with machinery that is already proven.
///
/// This is a POST-pass over the lowered ops rather than a hook in
/// `lower_expression_with_expected`, for the same reason `hoist_trap_calls` is:
/// the expression lowerer returns an `IrValue` and has no statement sink, and
/// threading one through it would touch every recursive call in the core
/// lowering path.
pub(super) fn lower_statement(
    statement: &HirStatement,
    locals: &mut HashMap<String, ParameterType>,
    context: &mut LowerContext<'_>,
    trap_name: Option<&str>,
) -> Vec<IrOp> {
    let ops = lower_statement_inner(statement, locals, context, trap_name);
    rewrite_concat_folds(ops, locals, context)
}

fn lower_statement_inner(
    statement: &HirStatement,
    locals: &mut HashMap<String, ParameterType>,
    context: &mut LowerContext<'_>,
    trap_name: Option<&str>,
) -> Vec<IrOp> {
    // The statement's own span: captured locally (nested blocks re-set
    // `context.current_loc`, so the context copy cannot be reread after
    // lowering a child block) and published for expression-lowering helpers
    // that synthesize ops mid-expression.
    let loc = statement_loc(statement);
    context.current_loc = loc;
    match statement {
        HirStatement::Let {
            mutable,
            name,
            type_,
            explicit_type,
            value,
            state_type,
            ..
        } => {
            // The explicit `AS T` annotation as the AST's `Option<String>`
            // (`None` when unannotated), reconstructed byte-exact from the HIR
            // `type_`/`explicit_type` pair.
            let declared_type: Option<ParameterType> = explicit_type.then(|| type_.clone());
            if let Some(HirExpression::Trapped {
                expression,
                binding,
                handler,
                ..
            }) = value
            {
                let success_type = declared_type
                    .clone()
                    .or_else(|| expression_type(expression, locals, context))
                    .unwrap_or(ParameterType::Unknown);
                // A `RES` binding's `STATE T` rides in the lowered type exactly as
                // on the non-trap path below. `expression_type` already carries it,
                // so only an explicit `AS T` needs it reattached — without this,
                // writing the annotation *caused* the TYPE_STATE_MISMATCH that
                // omitting it avoided (bug-372).
                let success_type = match (&declared_type, state_type) {
                    (Some(declared_type), Some(state)) => declared_type.with_state(state),
                    _ => success_type,
                };
                return lower_inline_trap(
                    expression,
                    binding,
                    handler,
                    InlineTrapTarget::Bind {
                        mutable: *mutable,
                        name: name.clone(),
                        type_: success_type,
                        explicit_type: *explicit_type,
                    },
                    locals,
                    context,
                );
            }
            let lowered_type = declared_type.clone().unwrap_or_else(|| {
                value
                    .as_ref()
                    .and_then(|value| expression_type(value, locals, context))
                    .unwrap_or(ParameterType::Unknown)
            });
            // A resource (or data) variant value bound to a union-typed binding is
            // wrapped into its union by `lower_expression_with_expected`, so a
            // `RES s AS Stream = <a File>` carries the variant tag for
            // tag-dispatched drop.
            let lowered_value = value.as_ref().map(|value| {
                lower_expression_with_expected(value, Some(&lowered_type), locals, context)
            });
            // A `RES` binding's `STATE T` rides in the lowered type
            // (`File STATE T`) so codegen can default-initialize and address the
            // state payload; the bare resource name is recovered for recognition.
            let lowered_type = match state_type {
                Some(state) => lowered_type.with_state(state),
                None => lowered_type,
            };
            locals.insert(name.clone(), lowered_type.clone());
            // Track `MUT` bindings so a non-escaping callback can capture them by
            // slot rather than copy them by value.
            if *mutable {
                context.mutable_locals.insert(name.clone());
            }
            vec![IrOp::Bind {
                mutable: *mutable,
                name: name.clone(),
                type_: lowered_type,
                value: lowered_value,
                explicit_type: *explicit_type,
                loc,
            }]
        }
        HirStatement::Return { value, .. } => vec![IrOp::Return {
            value: value.as_ref().map(|value| {
                // Coerce a bare numeric literal to the declared return type,
                // exactly as `LET`/constructor-arg lowering does — otherwise an
                // unsuffixed literal returned from a `Fixed`/`Money`/`Float`
                // function is classified as `Integer` and its raw bits are
                // reinterpreted as the destination type (bug-156).
                // A returned variant is wrapped into the function's declared union
                // return type there too, so the wrap is explicit in the IR (and
                // faithfully serialized into Binary Representation) rather than
                // re-derived during native codegen.
                let expected = context.current_return_type.clone();
                lower_expression_with_expected(value, expected.as_ref(), locals, context)
            }),
            loc,
        }],
        HirStatement::Exit { target, .. } => match target {
            ExitTarget::For => vec![IrOp::ExitLoop {
                kind: LoopKind::For,
                loc,
            }],
            ExitTarget::Do => vec![IrOp::ExitLoop {
                kind: LoopKind::Do,
                loc,
            }],
            ExitTarget::While => vec![IrOp::ExitLoop {
                kind: LoopKind::While,
                loc,
            }],
            ExitTarget::Func => Vec::new(),
        },
        HirStatement::Continue { kind, .. } => vec![IrOp::ContinueLoop { kind: *kind, loc }],
        HirStatement::Fail { error, .. } => vec![IrOp::Fail {
            error: lower_expression(error, locals, context),
            loc,
        }],
        HirStatement::Propagate { .. } => vec![IrOp::Fail {
            // Typecheck rejects PROPAGATE outside a trap body; total lowering
            // (plan-20-D) stamps a sentinel error local when the guard is
            // absent so it never panics on ill-typed input.
            error: IrValue::Local(trap_name.unwrap_or("$error").to_string()),
            loc,
        }],
        HirStatement::Recover { value, .. } => {
            // Typecheck rejects RECOVER outside an inline-TRAP handler
            // (TYPE_RECOVER_OUTSIDE_INLINE_TRAP); total lowering binds the stray
            // value to a `$recover_stray` temp rather than panicking. The temp —
            // not a discard `Eval` — because the statement's one surviving fact
            // must stay readable: the front end's flow analysis treats ANY
            // RECOVER as diverging, and `ir::verify`'s divergence rules
            // (TYPE_TRAP_FALLTHROUGH, TYPE_FUNC_MISSING_RETURN) key on the
            // temp's name to agree. Never reaches codegen: the program is
            // rejected.
            let Some(target) = context.recover_targets.last().cloned() else {
                let name = make_temp_local_name(context, "recover_stray");
                let value = value
                    .as_ref()
                    .map(|value| lower_expression(value, locals, context));
                return vec![IrOp::Bind {
                    mutable: false,
                    name,
                    type_: ParameterType::Unknown,
                    value,
                    explicit_type: false,
                    loc,
                }];
            };
            match (target.slot, value) {
                (Some(slot), Some(value)) => {
                    // bug-648 E: a union-typed slot (the trapped producer returns the
                    // union) receiving a VARIANT needs the explicit wrap every
                    // expected-type position gets from `lower_expression_with_expected`
                    // — the trailing `Bind name = $trap_val` is union-to-union and adds
                    // none, so an unwrapped variant reached the binding with no tag.
                    let lowered =
                        lower_expression_with_expected(value, Some(&target.type_), locals, context);
                    vec![IrOp::Assign {
                        name: slot,
                        value: lowered,
                        loc,
                    }]
                }
                (None, Some(value)) => vec![IrOp::Eval {
                    value: lower_expression_with_expected(
                        value,
                        Some(&target.type_),
                        locals,
                        context,
                    ),
                    loc,
                }],
                (_, None) => Vec::new(),
            }
        }
        HirStatement::Assign { name, value, .. } => {
            if let HirExpression::Trapped {
                expression,
                binding,
                handler,
                ..
            } = value
            {
                return lower_inline_trap(
                    expression,
                    binding,
                    handler,
                    InlineTrapTarget::Assign { name: name.clone() },
                    locals,
                    context,
                );
            }
            // bug-551: an assignment to an imported package's `EXPORT MUT` is
            // written `pkg::Name` and reaches here as `pkg.Name`; under
            // `IMPORT pkg AS p` it reaches here as `p.Name`. The CANONICAL
            // spelling is what `ir::package::apply_package_identity` rewrites to
            // the merged definition, so resolve the alias before emitting.
            let canonical_name = canonical_import_name(name, context);
            let target = if locals.contains_key(name) || context.binding_types.contains_key(name) {
                name.clone()
            } else if context.binding_types.contains_key(&canonical_name) {
                canonical_name
            } else {
                name.clone()
            };
            let expected = locals
                .get(&target)
                .or_else(|| context.binding_types.get(&target))
                .cloned();
            let lowered = lower_expression_with_expected(value, expected.as_ref(), locals, context);
            if locals.contains_key(&target) {
                vec![IrOp::Assign {
                    name: target,
                    value: lowered,
                    loc,
                }]
            } else {
                vec![IrOp::AssignGlobal {
                    name: target,
                    value: lowered,
                    loc,
                }]
            }
        }
        HirStatement::StateAssign {
            resource, value, ..
        } => {
            // bug-551, the `resource.state = <expr> TRAP …` form: the trap
            // covers the whole replacement, so `RECOVER` supplies a STATE
            // record and the assignment happens once, after the branch.
            if let HirExpression::Trapped {
                expression: inner,
                binding,
                handler,
                ..
            } = value
            {
                return lower_inline_trap(
                    inner,
                    binding,
                    handler,
                    InlineTrapTarget::StateAssign {
                        resource: resource.clone(),
                    },
                    locals,
                    context,
                );
            }
            // And the `resource.state.field = <expr> TRAP …` form, which
            // `ast::stmt` has already turned into a single-field `WITH` update
            // with the trap still on the field's value. The trap covers what the
            // author wrote -- the FIELD's expression -- so its result is bound to
            // a temporary and the update is applied to that. Recovering into the
            // field rather than into a rebuilt state record is the whole reason
            // the trap is not hoisted at parse time.
            if let HirExpression::WithUpdate { target, updates } = value {
                if let [update] = updates.as_slice() {
                    if let HirExpression::Trapped {
                        expression: inner,
                        binding,
                        handler,
                        ..
                    } = &update.value
                    {
                        let field_type = expression_type(inner, locals, context)
                            .unwrap_or(ParameterType::Unknown);
                        let temp = make_temp_local_name(context, "trap_state_field");
                        let mut ops = lower_inline_trap(
                            inner,
                            binding,
                            handler,
                            InlineTrapTarget::Bind {
                                mutable: false,
                                name: temp.clone(),
                                type_: field_type,
                                explicit_type: false,
                            },
                            locals,
                            context,
                        );
                        let updated = HirExpression::WithUpdate {
                            target: target.clone(),
                            updates: vec![crate::hir::HirRecordUpdate {
                                field: update.field.clone(),
                                value: HirExpression::Identifier(temp),
                                line: update.line,
                            }],
                        };
                        ops.push(IrOp::StateAssign {
                            resource: resource.clone(),
                            value: lower_expression(&updated, locals, context),
                            loc,
                        });
                        return ops;
                    }
                }
            }
            let resource_type = locals
                .get(resource)
                .or_else(|| context.binding_types.get(resource))
                .cloned();
            // plan-106-C: `ParameterType::state` splits the ` STATE T` clause out
            // structurally, so this no longer renders the type to read it back.
            let state_type = resource_type.as_ref().and_then(ParameterType::state);
            let lowered =
                lower_expression_with_expected(value, state_type.as_ref(), locals, context);
            vec![IrOp::StateAssign {
                resource: resource.clone(),
                value: lowered,
                loc,
            }]
        }
        HirStatement::Expression { expression, .. } => {
            // Assertion builtins (plan-18-B) desugar to ordinary statements —
            // comparisons + FAIL, or a trap-guarded evaluation — which are then
            // lowered through the normal path. Doing it here (post-typecheck)
            // sidesteps the source-level RECOVER-typing constraint on a
            // value-producing trapped expression.
            if let HirExpression::Call {
                callee,
                arguments,
                line: call_line,
                ..
            } = expression
            {
                if crate::codegen::builtins_testing::is_testing_call(callee) {
                    let uid = context.next_temp_id;
                    context.next_temp_id += 1;
                    let expanded =
                        crate::testing::expand_expect(callee, arguments, uid, *call_line);
                    return lower_statement_block(&expanded, locals, context, trap_name);
                }
            }
            // plan-180-A: a bare `os::exit(code)` statement is the program-exit op
            // (what `os::exit` lowered to). Every other position stays an
            // ordinary call and reaches the `os::exit` rewrite helper.
            if let Some(code) = os_exit_statement_code(expression, context) {
                return vec![IrOp::ExitProgram {
                    code: lower_expression(code, locals, context),
                    loc,
                }];
            }
            if let HirExpression::Trapped {
                expression: inner,
                binding,
                handler,
                ..
            } = expression
            {
                return lower_inline_trap(
                    inner,
                    binding,
                    handler,
                    InlineTrapTarget::Discard,
                    locals,
                    context,
                );
            }
            vec![IrOp::Eval {
                value: lower_expression(expression, locals, context),
                loc,
            }]
        }
        HirStatement::If {
            condition,
            then_body,
            else_body,
            ..
        } => vec![IrOp::If {
            condition: lower_expression(condition, locals, context),
            then_body: lower_statement_block(then_body, locals, context, trap_name),
            else_body: lower_statement_block(else_body, locals, context, trap_name),
            loc,
        }],
        HirStatement::Match {
            expression, cases, ..
        } => {
            let matched_type = match_expression_type(expression, locals, context)
                .unwrap_or(ParameterType::Unknown);
            let matched_name = make_temp_local_name(context, "match");
            let mut ops = vec![IrOp::Bind {
                mutable: false,
                name: matched_name.clone(),
                type_: matched_type.clone(),
                value: Some(lower_match_expression(
                    expression,
                    &matched_type,
                    locals,
                    context,
                )),
                loc,
                explicit_type: false,
            }];
            let mut match_locals = locals.clone();
            match_locals.insert(matched_name.clone(), matched_type);
            // coverage:off -- a call scrutinee auto-unwraps to its value and no
            // user spelling names the internal `#Result OF ...` (bug-945), so this
            // Result-flag branch is unreachable from source; kept for plan-20 total
            // lowering.
            let match_value = if matches!(match_locals[&matched_name], ParameterType::ResultOf(_)) {
                let match_flag_name = make_temp_local_name(context, "match_ok");
                ops.push(IrOp::Bind {
                    mutable: false,
                    name: match_flag_name.clone(),
                    type_: ParameterType::Boolean,
                    value: Some(IrValue::ResultIsOk {
                        value: Box::new(IrValue::Local(matched_name.clone())),
                    }),
                    loc,
                    explicit_type: false,
                });
                match_locals.insert(match_flag_name.clone(), ParameterType::Boolean);
                IrValue::Local(match_flag_name)
            } else {
                IrValue::Local(matched_name.clone())
            };
            // coverage:on
            ops.push(IrOp::Match {
                value: match_value,
                cases: cases
                    .iter()
                    .map(|case| {
                        lower_match_case(case, &matched_name, &match_locals, context, trap_name)
                    })
                    .collect(),
                loc,
            });
            ops
        }
        HirStatement::For {
            name,
            start,
            end,
            step,
            body,
            line,
        } => {
            let start_type =
                expression_type(start, locals, context).unwrap_or(ParameterType::Unknown);
            let end_type = expression_type(end, locals, context).unwrap_or(ParameterType::Unknown);
            let step_type = step
                .as_ref()
                .and_then(|value| expression_type(value, locals, context))
                .unwrap_or(ParameterType::Integer);
            let loop_type =
                numeric::typed_promote_loop_numeric_type(&start_type, &end_type, &step_type);
            let iter_name = make_temp_local_name(context, "for_iter");
            let end_name = make_temp_local_name(context, "for_end");
            let step_name = make_temp_local_name(context, "for_step");

            let start_value =
                lower_expression_with_expected(start, Some(&loop_type), locals, context);
            let end_value = lower_expression_with_expected(end, Some(&loop_type), locals, context);
            let step_value = step
                .as_ref()
                .map(|value| {
                    lower_expression_with_expected(value, Some(&loop_type), locals, context)
                })
                .unwrap_or_else(|| numeric_constant_for_type(&loop_type, "1"));

            // `$for_end`/`$for_step` keep their operand's own numeric type. The
            // loop's bound and step tests and its increment then promote that
            // operand exactly as the source comparison or `+` would (§4.1;
            // bug-816). Declaring them the loop type made the native bind store
            // the operand's bits unconverted, so a `Float` bound of a `Fixed`
            // loop was read as a raw Fixed and the loop ran ~10^9 times.
            let bound_type =
                |value: &IrValue, source: &HirExpression, context: &LowerContext<'_>| {
                    value
                        .annotated_parameter_type()
                        .or_else(|| expression_type(source, locals, context))
                        .filter(numeric::is_numeric)
                        .unwrap_or_else(|| loop_type.clone())
                };
            let end_bind_type = bound_type(&end_value, end, context);
            let step_bind_type = match step {
                Some(source) => bound_type(&step_value, source, context),
                None => loop_type.clone(),
            };

            locals.insert(iter_name.clone(), loop_type.clone());
            locals.insert(end_name.clone(), end_bind_type.clone());
            locals.insert(step_name.clone(), step_bind_type.clone());

            let step_local = IrValue::Local(step_name.clone());
            let iter_local = IrValue::Local(iter_name.clone());
            let end_local = IrValue::Local(end_name.clone());

            let mut nested = locals.clone();
            nested.insert(name.clone(), loop_type.clone());
            let mut loop_body = vec![IrOp::Bind {
                mutable: false,
                name: name.clone(),
                type_: loop_type.clone(),
                value: Some(iter_local.clone()),
                loc,
                explicit_type: false,
            }];
            loop_body.extend(lower_statement_block(body, &nested, context, trap_name));

            vec![
                IrOp::Bind {
                    mutable: false,
                    name: end_name,
                    type_: end_bind_type,
                    value: Some(end_value),
                    loc,
                    explicit_type: false,
                },
                IrOp::Bind {
                    mutable: false,
                    name: step_name,
                    type_: step_bind_type,
                    value: Some(step_value),
                    loc,
                    explicit_type: false,
                },
                IrOp::For {
                    name: iter_name,
                    type_: loop_type.clone(),
                    start: start_value,
                    end: end_local,
                    step: step_local,
                    body: loop_body,
                    loc: IrSourceLoc {
                        line: *line as u32,
                        column: 1,
                    },
                },
            ]
        }
        HirStatement::ForEach {
            name,
            iterable,
            body,
            ..
        } => {
            let iterable_type =
                expression_type(iterable, locals, context).unwrap_or(ParameterType::Unknown);
            let element_type =
                collection_iteration_type(&iterable_type).unwrap_or(ParameterType::Unknown);
            let mut nested = locals.clone();
            nested.insert(name.clone(), element_type.clone());
            vec![IrOp::ForEach {
                name: name.clone(),
                type_: element_type.clone(),
                iterable: lower_expression(iterable, locals, context),
                body: lower_statement_block(body, &nested, context, trap_name),
                loc,
            }]
        }
        HirStatement::While {
            kind,
            condition,
            body,
            ..
        } => vec![IrOp::While {
            kind: *kind,
            condition: lower_expression(condition, locals, context),
            body: lower_statement_block(body, locals, context, trap_name),
            loc,
        }],
        HirStatement::DoUntil {
            body, condition, ..
        } => {
            let body = lower_statement_block(body, locals, context, trap_name);
            // The trailing condition belongs to this statement, not the last
            // body statement: restore the loop's own span for any ops the
            // condition lowering synthesizes.
            context.current_loc = loc;
            vec![IrOp::DoUntil {
                body,
                condition: lower_expression(condition, locals, context),
                loc,
            }]
        }
    }
}

pub(super) fn lower_statement_block(
    body: &[HirStatement],
    locals: &HashMap<String, ParameterType>,
    context: &mut LowerContext<'_>,
    trap_name: Option<&str>,
) -> Vec<IrOp> {
    let mut nested = locals.clone();
    body.iter()
        .flat_map(|statement| lower_statement(statement, &mut nested, context, trap_name))
        .collect()
}

/// The loop-variable type `FOR EACH x IN <collection>` binds, or `None` when the
/// value is not iterable.
///
/// plan-106-A: a structural match on the collection's [`ParameterType`], replacing
/// the `strip_prefix("List OF ")` / `strip_prefix("Set OF ")` / `parse_map_type`
/// cascade this grew from — `ir::lower` holds no copy of the type grammar.
pub(in crate::ir) fn collection_iteration_type(type_: &ParameterType) -> Option<ParameterType> {
    match type_ {
        // Iterating `List OF RES File` yields a pointer to each element; the loop
        // variable's type is the bare resource (`File`), not `RES File` (§15.6).
        ParameterType::ListOf(element) => Some(strip_res(element)),
        // `FOR EACH x IN set` yields the element type `T` (plan-63); a Set element
        // is always comparable, so it never carries a `RES` marker.
        ParameterType::SetOf(element) => Some((**element).clone()),
        ParameterType::MapOf(key, value) => Some(ParameterType::map_entry_of(
            (**key).clone(),
            strip_res(value),
        )),
        _ => None,
    }
}

/// A collection element with its `RES ` ownership marker removed, if it carries
/// one — the structural form of the `strip_prefix("RES ")` this replaced.
/// plan-180-A: the `code` argument of a bare `os::exit(code)` call statement
/// (positional or `code :=`), or `None` for any other expression. Only a plain
/// `Call` matches: a `TRAP`'d call is a `Trapped` expression and keeps its trap.
fn os_exit_statement_code<'a>(
    expression: &'a HirExpression,
    context: &LowerContext<'_>,
) -> Option<&'a HirExpression> {
    let HirExpression::Call {
        callee, arguments, ..
    } = expression
    else {
        return None;
    };
    if canonical_import_name(callee, context) != "os.exit" {
        return None;
    }
    match arguments.as_slice() {
        [HirCallArg::Positional(code)] => Some(code),
        [HirCallArg::Named { name, value, .. }] if name == "code" => Some(value),
        _ => None,
    }
}

fn strip_res(type_: &ParameterType) -> ParameterType {
    match type_ {
        ParameterType::Res(inner) => (**inner).clone(),
        other => other.clone(),
    }
}

/// The `AttributedString` type (plan-89-D's attributed-text nominal).
///
/// Deliberately a NOMINAL and not [`ParameterType::AttributeString`]: that
/// variant renders `"AttributeString"` — no `d` — which is a different spelling
/// the language's attributed-text type never uses, so `parse("AttributedString")`
/// yields a `Named` and every structural comparison must build the same thing.
/// A function rather than a `const` because [`ParameterType::named`] interns.
pub(super) fn attributed_string_type() -> ParameterType {
    ParameterType::named("AttributedString")
}

pub(in crate::ir) fn make_temp_local_name(context: &mut LowerContext<'_>, prefix: &str) -> String {
    let name = format!("${prefix}{}", context.next_temp_id);
    context.next_temp_id += 1;
    name
}

fn numeric_constant_for_type(type_: &ParameterType, value: &str) -> IrValue {
    IrValue::Const {
        type_: type_.clone(),
        value: value.to_string(),
    }
}

fn lower_match_case(
    case: &HirMatchCase,
    matched_local: &str,
    locals: &HashMap<String, ParameterType>,
    context: &mut LowerContext<'_>,
    trap_name: Option<&str>,
) -> IrMatchCase {
    // The case arm's own span (the former source checker reports match-arm rules at the case
    // line); captured locally since the body block re-sets the context copy.
    let loc = IrSourceLoc {
        line: case.line as u32,
        column: 1,
    };
    context.current_loc = loc;
    let matched_type = locals
        .get(matched_local)
        .cloned()
        .unwrap_or(ParameterType::Unknown);
    // An unsuffixed numeric literal pattern coerces to a `Fixed` or `Money`
    // scrutinee exactly as it does to a `LET` of that type, so the arm compares
    // two values of the scrutinee's representation. Lowered without the expected
    // type it stayed an `Integer`/`Float` constant, and the match compared its
    // bits with the scrutinee's raw carrier: `MATCH 1F / CASE 1` never matched.
    // `Byte` is left alone: its literal range check is the verifier's.
    let pattern_expected = matches!(matched_type, ParameterType::Fixed | ParameterType::Money)
        .then_some(&matched_type);
    let pattern = match &case.pattern {
        HirMatchPattern::Else => IrMatchPattern::Else,
        HirMatchPattern::Literal(expression) => IrMatchPattern::Value(
            lower_expression_with_expected(expression, pattern_expected, locals, context),
        ),
        // coverage:off -- reachable only for an internal `#Result OF ...`
        // scrutinee, which no source MATCH has (bug-945); kept for plan-20 total
        // lowering.
        HirMatchPattern::Union { type_, .. }
            if matches!(matched_type, ParameterType::ResultOf(_)) =>
        {
            // plan-111-B: `#Ok`/`Error` are nominals, so this asks the interned
            // `Symbol` rather than matching a rendered spelling. Everything that
            // is not `Ok` is `false`, exactly as the `_` arm said.
            let matched = if type_.is_named(crate::types::RESULT_SUCCESS_MEMBER) {
                "true"
            } else {
                "false"
            };
            IrMatchPattern::Value(IrValue::Const {
                type_: ParameterType::Boolean,
                value: matched.to_string(),
            })
        }
        // coverage:on
        HirMatchPattern::Union { type_, .. } => {
            IrMatchPattern::Value(IrValue::Local(type_.name().into_owned()))
        }
        HirMatchPattern::OneOf(expressions) => IrMatchPattern::OneOf(
            expressions
                .iter()
                .map(|expression| {
                    lower_expression_with_expected(expression, pattern_expected, locals, context)
                })
                .collect(),
        ),
    };
    let mut case_locals = locals.clone();
    let mut body = Vec::new();
    if let Some((binding, binding_type, value)) =
        match_case_binding(&case.pattern, matched_local, &matched_type)
    {
        case_locals.insert(binding.clone(), binding_type.clone());
        body.push(IrOp::Bind {
            mutable: false,
            name: binding,
            type_: binding_type,
            value: Some(value),
            loc,
            explicit_type: false,
        });
    }
    body.extend(lower_statement_block(
        &case.body,
        &case_locals,
        context,
        trap_name,
    ));
    // The guard belongs to the case arm, not the last body statement: restore
    // the arm's span for any ops the guard lowering synthesizes.
    context.current_loc = loc;
    IrMatchCase {
        pattern,
        guard: case
            .guard
            .as_ref()
            .map(|guard| lower_expression(guard, &case_locals, context)),
        body,
        loc,
    }
}

pub(in crate::ir) fn match_case_binding(
    pattern: &HirMatchPattern,
    matched_local: &str,
    matched_type: &ParameterType,
) -> Option<(String, ParameterType, IrValue)> {
    match pattern {
        HirMatchPattern::Union { type_, binding } => {
            // coverage:off -- no source MATCH has an internal `#Result OF ...`
            // scrutinee (bug-945); this `#Ok`/Error case binding is kept only for
            // plan-20 total lowering.
            if let ParameterType::ResultOf(success) = matched_type {
                let success = (**success).clone();
                // plan-111-B: `#Ok`/`Error` are nominals — interned-`Symbol`
                // compares, not spelling matches. Same three outcomes, same order.
                if type_.is_named(crate::types::RESULT_SUCCESS_MEMBER) {
                    return Some((
                        binding.clone(),
                        success.clone(),
                        IrValue::ResultValue {
                            type_: success,
                            value: Box::new(IrValue::Local(matched_local.to_string())),
                        },
                    ));
                }
                if type_.is_named("Error") {
                    return Some((
                        binding.clone(),
                        ParameterType::named("Error"),
                        IrValue::ResultError {
                            value: Box::new(IrValue::Local(matched_local.to_string())),
                        },
                    ));
                }
                return None;
            }
            // coverage:on
            // A stateful resource union's STATE is uniform across variants, so the
            // extracted variant binding carries the same STATE suffix as the
            // scrutinee (plan-74) — this is what lets `f.state` on `CASE File(f)`
            // resolve and lower through the concrete-record path. The `UnionExtract`
            // itself stays keyed on the bare variant type (it loads the variant
            // record pointer at `+8`).
            // The scrutinee's `STATE` clause is split out structurally and
            // re-attached to the variant (plan-106-C).
            let binding_type = match matched_type.state() {
                Some(state) => type_.with_state(&state),
                None => type_.clone(),
            };
            Some((
                binding.clone(),
                binding_type,
                IrValue::UnionExtract {
                    type_: type_.clone(),
                    value: Box::new(IrValue::Local(matched_local.to_string())),
                },
            ))
        }
        _ => None,
    }
}

fn lower_match_expression(
    expression: &HirExpression,
    matched_type: &ParameterType,
    locals: &HashMap<String, ParameterType>,
    context: &mut LowerContext<'_>,
) -> IrValue {
    // A `MATCH` scrutinee that is a call auto-unwraps like any other call site
    // (local error handling now uses an inline `TRAP`), so the scrutinee lowers
    // to its ordinary value. No source value carries the internal `#Result OF …`
    // type (bug-945), so a scrutinee is never a result.
    lower_expression_with_expected(expression, Some(matched_type), locals, context)
}

pub(in crate::ir) fn match_expression_type(
    expression: &HirExpression,
    locals: &HashMap<String, ParameterType>,
    context: &LowerContext<'_>,
) -> Option<ParameterType> {
    // Call scrutinees auto-unwrap, so the scrutinee has its value's type.
    expression_type(expression, locals, context)
}
