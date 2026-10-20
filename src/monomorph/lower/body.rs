//! plan-176-M: lowering function bodies — statements, expressions and constructor arguments
//! — onto their concrete instantiations. The driver lives in `monomorph/lower/mod.rs`.

use super::*;

impl<'a> Monomorphizer<'a> {
    pub(super) fn lower_statements(
        &mut self,
        statements: &[HirStatement],
        substitutions: &HashMap<crate::intern::Symbol, crate::types::ParameterType>,
        context: &mut FunctionContext,
    ) -> Vec<HirStatement> {
        statements
            .iter()
            .map(|statement| self.lower_statement(statement, substitutions, context))
            .collect()
    }

    fn lower_statement(
        &mut self,
        statement: &HirStatement,
        substitutions: &HashMap<crate::intern::Symbol, crate::types::ParameterType>,
        context: &mut FunctionContext,
    ) -> HirStatement {
        match statement {
            HirStatement::Let {
                mutable,
                resource,
                state_type,
                name,
                type_,
                explicit_type,
                value,
                line,
            } => {
                // Mirror the AST `Option<String>` the pre-D3 walk consumed: an
                // plan-111-B: an explicit `AS T` annotation substitutes
                // directly; an inferred `LET` carries `None`. `concrete_type` is
                // the typed form `concrete_type_name` wrapped, so nothing
                // renders and nothing re-parses.
                let lowered_type = explicit_type.then(|| self.concrete_type(type_, substitutions));
                let lowered_state = state_type
                    .as_ref()
                    .map(|state_type| self.concrete_type(state_type, substitutions));
                // The declared type with this instantiation's template params
                // substituted. `type_` IS the parse of `type_name` (they round-trip
                // byte-exact), so substitute the HIR node directly rather than
                // re-parsing a render of it.
                let expected_source_type =
                    explicit_type.then(|| substitute_type_params(type_, substitutions));
                let lowered_value = value.as_ref().map(|value| {
                    self.lower_expression(
                        value,
                        substitutions,
                        context,
                        expected_source_type.as_ref(),
                        *line,
                    )
                });
                let binding_type = lowered_type.clone().or_else(|| {
                    lowered_value
                        .as_ref()
                        .and_then(|value| self.expression_type(value, context))
                });
                // bug-671: an explicit `AS T` binding with `STATE S` records the
                // payload too (an inferred one already carries it from the value).
                let binding_type = match (binding_type, &lowered_state) {
                    (Some(binding_type), Some(state)) if binding_type.state().is_none() => {
                        Some(binding_type.with_state(state))
                    }
                    (binding_type, _) => binding_type,
                };
                if let Some(binding_type) = binding_type {
                    context.locals.insert(name.clone(), binding_type);
                }
                context.bound.insert(name.clone());
                HirStatement::Let {
                    mutable: *mutable,
                    resource: *resource,
                    state_type: lowered_state,
                    name: name.clone(),
                    type_: lowered_type.unwrap_or(ParameterType::Unknown),
                    explicit_type: *explicit_type,
                    value: lowered_value,
                    line: *line,
                }
            }
            HirStatement::Return { value, line } => HirStatement::Return {
                value: value.as_ref().map(|value| {
                    // A `RETURN` of a call propagates the enclosing function's
                    // declared return type as the expected type so a return-type
                    // overload set resolves (plan-01-overload.md §F.2); a bare
                    // name selects an overloaded function's member by it (bug-822).
                    let expected = consults_expected(value)
                        .then(|| context.enclosing_return.clone())
                        .flatten();
                    self.lower_expression(value, substitutions, context, expected.as_ref(), *line)
                }),
                line: *line,
            },
            HirStatement::Exit { kind, line } => HirStatement::Exit {
                kind: *kind,
                line: *line,
            },
            HirStatement::Continue { kind, line } => HirStatement::Continue {
                kind: *kind,
                line: *line,
            },
            HirStatement::Fail { error, line } => HirStatement::Fail {
                error: self.lower_expression(error, substitutions, context, None, *line),
                line: *line,
            },
            HirStatement::Propagate { line } => HirStatement::Propagate { line: *line },
            HirStatement::Recover { value, line } => HirStatement::Recover {
                value: value
                    .as_ref()
                    .map(|value| self.lower_expression(value, substitutions, context, None, *line)),
                line: *line,
            },
            HirStatement::Assign { name, value, line } => {
                // Pass the target local's declared type as the RHS expected type so
                // a return-type-overloaded call disambiguates, exactly like the
                // `LET … AS T = call()` form (bug-197). Only consulted by call
                // overload resolution; other RHS expressions ignore it.
                let expected = context.locals.get(name).cloned();
                HirStatement::Assign {
                    name: name.clone(),
                    value: self.lower_expression(
                        value,
                        substitutions,
                        context,
                        expected.as_ref(),
                        *line,
                    ),
                    line: *line,
                }
            }
            HirStatement::StateAssign {
                resource,
                value,
                line,
            } => {
                // The value of `h.state = …` is the payload, not the handle.
                let expected = context.locals.get(resource).and_then(ParameterType::state);
                HirStatement::StateAssign {
                    resource: resource.clone(),
                    value: self.lower_expression(
                        value,
                        substitutions,
                        context,
                        expected.as_ref(),
                        *line,
                    ),
                    line: *line,
                }
            }
            HirStatement::Expression { expression, line } => HirStatement::Expression {
                expression: self.lower_expression(expression, substitutions, context, None, *line),
                line: *line,
            },
            HirStatement::If {
                condition,
                then_body,
                else_body,
                line,
            } => {
                let mut then_context = context.clone();
                let mut else_context = context.clone();
                HirStatement::If {
                    condition: self.lower_expression(
                        condition,
                        substitutions,
                        context,
                        None,
                        *line,
                    ),
                    then_body: self.lower_statements(then_body, substitutions, &mut then_context),
                    else_body: self.lower_statements(else_body, substitutions, &mut else_context),
                    line: *line,
                }
            }
            HirStatement::Match {
                expression,
                cases,
                line,
            } => {
                let expression =
                    self.lower_expression(expression, substitutions, context, None, *line);
                let scrutinee = self.expression_type(&expression, context);
                HirStatement::Match {
                    expression,
                    cases: cases
                        .iter()
                        .map(|case| {
                            let mut case_context = context.clone();
                            if let HirMatchPattern::Union { binding, .. } = &case.pattern {
                                case_context.bound.insert(binding.clone());
                            }
                            // bug-680: a CASE names a template-instantiated variant by
                            // its template's name (`CASE Some(s)` over `Opt OF Integer`),
                            // so the pattern takes the concrete variant the scrutinee's
                            // union actually carries.
                            let union_pattern_type = match &case.pattern {
                                HirMatchPattern::Union { type_, .. } => {
                                    Some(self.union_pattern_type(
                                        scrutinee.as_ref(),
                                        type_,
                                        substitutions,
                                        case.line,
                                    ))
                                }
                                _ => None,
                            };
                            if let (HirMatchPattern::Union { binding, .. }, Some(type_)) =
                                (&case.pattern, &union_pattern_type)
                            {
                                case_context.locals.insert(binding.clone(), type_.clone());
                            }
                            HirMatchCase {
                                pattern: match &case.pattern {
                                    HirMatchPattern::Else => HirMatchPattern::Else,
                                    HirMatchPattern::Literal(expression) => {
                                        HirMatchPattern::Literal(self.lower_expression(
                                            expression,
                                            substitutions,
                                            &mut case_context,
                                            None,
                                            case.line,
                                        ))
                                    }
                                    HirMatchPattern::Union { binding, .. } => {
                                        HirMatchPattern::Union {
                                            type_: union_pattern_type
                                                .clone()
                                                .expect("a union pattern resolves its type above"),
                                            binding: binding.clone(),
                                        }
                                    }
                                    HirMatchPattern::OneOf(expressions) => HirMatchPattern::OneOf(
                                        expressions
                                            .iter()
                                            .map(|expression| {
                                                self.lower_expression(
                                                    expression,
                                                    substitutions,
                                                    &mut case_context,
                                                    None,
                                                    case.line,
                                                )
                                            })
                                            .collect(),
                                    ),
                                },
                                guard: case.guard.as_ref().map(|guard| {
                                    self.lower_expression(
                                        guard,
                                        substitutions,
                                        &mut case_context,
                                        None,
                                        case.line,
                                    )
                                }),
                                body: self.lower_statements(
                                    &case.body,
                                    substitutions,
                                    &mut case_context,
                                ),
                                line: case.line,
                            }
                        })
                        .collect(),
                    line: *line,
                }
            }
            HirStatement::For {
                name,
                start,
                end,
                step,
                body,
                line,
            } => {
                let lowered_start =
                    self.lower_expression(start, substitutions, context, None, *line);
                let lowered_end = self.lower_expression(end, substitutions, context, None, *line);
                let lowered_step = step
                    .as_ref()
                    .map(|value| self.lower_expression(value, substitutions, context, None, *line));
                let mut nested = context.clone();
                if let Some(loop_type) = self
                    .expression_type(&lowered_start, context)
                    .zip(self.expression_type(&lowered_end, context))
                    .map(|(start_type, end_type)| {
                        let step_type = lowered_step
                            .as_ref()
                            .and_then(|value| self.expression_type(value, context))
                            .unwrap_or(ParameterType::Integer);
                        crate::numeric::typed_promote_loop_numeric_type(
                            &start_type,
                            &end_type,
                            &step_type,
                        )
                    })
                {
                    nested.locals.insert(name.clone(), loop_type);
                }
                nested.bound.insert(name.clone());
                HirStatement::For {
                    name: name.clone(),
                    start: lowered_start,
                    end: lowered_end,
                    step: lowered_step,
                    body: self.lower_statements(body, substitutions, &mut nested),
                    line: *line,
                }
            }
            HirStatement::ForEach {
                name,
                iterable,
                body,
                line,
            } => {
                let lowered_iterable =
                    self.lower_expression(iterable, substitutions, context, None, *line);
                let mut nested = context.clone();
                if let Some(iterable_type) = self.expression_type(&lowered_iterable, context) {
                    // plan-105-B: the iterable's element type, off the canonical
                    // grammar. Iterating a `Map` yields a `MapEntry` over the same
                    // key/value pair, which is now built structurally instead of by
                    // splicing the raw `K TO V` text behind a `MapEntry OF ` prefix.
                    // plan-106-A: the iterable's type arrives already typed, so the
                    // match is on the value itself rather than on a re-parse of its
                    // rendered name.
                    let loop_type = match iterable_type {
                        ParameterType::ListOf(element) | ParameterType::SetOf(element) => *element,
                        ParameterType::MapOf(key, value) => ParameterType::MapEntryOf(key, value),
                        _ => ParameterType::Unknown,
                    };
                    nested.locals.insert(name.clone(), loop_type);
                }
                nested.bound.insert(name.clone());
                HirStatement::ForEach {
                    name: name.clone(),
                    iterable: lowered_iterable,
                    body: self.lower_statements(body, substitutions, &mut nested),
                    line: *line,
                }
            }
            HirStatement::While {
                kind,
                condition,
                body,
                line,
            } => HirStatement::While {
                kind: *kind,
                condition: self.lower_expression(condition, substitutions, context, None, *line),
                body: self.lower_statements(body, substitutions, &mut context.clone()),
                line: *line,
            },
            HirStatement::DoUntil {
                body,
                condition,
                line,
            } => HirStatement::DoUntil {
                body: self.lower_statements(body, substitutions, &mut context.clone()),
                condition: self.lower_expression(condition, substitutions, context, None, *line),
                line: *line,
            },
        }
    }

    pub(super) fn lower_expression(
        &mut self,
        expression: &HirExpression,
        substitutions: &HashMap<crate::intern::Symbol, crate::types::ParameterType>,
        context: &mut FunctionContext,
        expected_type: Option<&ParameterType>,
        line: usize,
    ) -> HirExpression {
        match expression {
            HirExpression::Call {
                callee,
                arguments,
                line: call_line,
                column,
            } => {
                // A call through a local (a parameter, `LET`, loop or lambda
                // binding) that shares its name with a top-level function calls
                // the LOCAL: no instantiation, override routing or overload
                // rewrite may claim it. Rewriting it to an overload member ran
                // the wrong function silently (bug-822).
                let local_callee = context.bound.contains(callee.as_str());
                // When the callee names exactly one user function, propagate each
                // parameter type as the expected type of its argument slot, but
                // only for an argument that consults it — a nested call, where a
                // return-type overload set needs the context to resolve
                // (plan-01-overload.md §F.2), or a bare name, which may select a
                // member of an overloaded function referenced as a value
                // (bug-822). Literals keep their own inferred typing.
                let sig_params = if local_callee {
                    None
                } else {
                    self.single_signature_params(callee)
                };
                let lowered_args = arguments
                    .iter()
                    .enumerate()
                    .map(|(index, argument)| match argument {
                        HirCallArg::Positional(value) => {
                            let expected =
                                arg_slot_expected(value, sig_params.as_deref(), |params| {
                                    params.get(index)
                                });
                            HirCallArg::Positional(self.lower_expression(
                                value,
                                substitutions,
                                context,
                                expected.as_ref(),
                                line,
                            ))
                        }
                        HirCallArg::Named { name, value, line } => {
                            let expected =
                                arg_slot_expected(value, sig_params.as_deref(), |params| {
                                    params.iter().find(|param| param.name == *name)
                                });
                            HirCallArg::Named {
                                name: name.clone(),
                                value: self.lower_expression(
                                    value,
                                    substitutions,
                                    context,
                                    expected.as_ref(),
                                    *line,
                                ),
                                line: *line,
                            }
                        }
                    })
                    .collect::<Vec<_>>();
                // Keep the vector aligned with the argument list: an argument
                // whose type cannot be inferred becomes the `"Unknown"` wildcard
                // (which `params_match`/`resolve_overload` accept for any
                // parameter) rather than being dropped, which would shorten the
                // vector and shift the remaining types into wrong positions.
                let arg_types = lowered_args
                    .iter()
                    .map(|argument| {
                        self.expression_type(call_arg_value(argument), context)
                            .unwrap_or(ParameterType::Unknown)
                    })
                    .collect::<Vec<_>>();
                // The public callee (`encoding.utf8Decode`, `collections.sort`)
                // before the internal-name rewrite below. A call that fails overload
                // resolution must keep this resolvable, dotted name rather than the
                // mangled `#name`: otherwise the post-monomorph resolver pass reports
                // SYMBOL_UNKNOWN_IDENTIFIER on an unresolvable `#encoding_utf8Decode`
                // and aborts before the former source checker can emit the real
                // TYPE_CALL_ARITY_MISMATCH / TYPE_CALL_ARGUMENT_MISMATCH (bug-443).
                let public_callee = callee.clone();
                // Rewrite the public overloaded `encoding::utf8Encode`/`utf8Decode`
                // onto their internal `__encoding_*` overload sets, and a
                // `collections::` call onto its internal generic implementation —
                // so the native overload / instantiation machinery below resolves
                // and mangles them like any user overload (`resolve_overload`).
                let encoding_internal = match callee.as_str() {
                    "encoding.utf8Encode" => Some("#encoding_utf8Encode".to_string()),
                    "encoding.utf8Decode" => Some("#encoding_utf8Decode".to_string()),
                    _ => None,
                };
                let callee = &encoding_internal
                    .or_else(|| self.collections_internal_callee(callee, arguments.len()))
                    .unwrap_or_else(|| callee.clone());
                // Named arguments can reorder the call's values relative to the
                // template's declared parameters. `arg_types` is built in source
                // order, so bind template type-params against types reordered into
                // declared-parameter order (mirroring the value reorder IR lowering
                // performs) — otherwise a type-param binds to the wrong argument
                // type and instantiation emits a wrong concrete symbol (bug-196).
                let target = if local_callee {
                    public_callee.clone()
                } else if let Some(target) =
                    self.instantiate_function(callee, arguments, &arg_types, line)
                {
                    target
                } else if let Some(target) =
                    self.resolve_general_builtin_override(callee, &arg_types)
                {
                    target
                } else if let Some(target) =
                    self.resolve_overload(callee, &public_callee, &arg_types, expected_type, line)
                {
                    target
                } else if let Some(target) =
                    self.resolve_imported_overload(callee, &arg_types, line)
                {
                    target
                } else {
                    // Overload resolution failed (wrong arity/argument types): keep
                    // the PUBLIC callee, not the mangled `#name`, so the second
                    // resolver pass resolves it and the former source checker emits the proper
                    // argument diagnostic naming the public call (bug-443).
                    public_callee.clone()
                };
                HirExpression::Call {
                    callee: target,
                    arguments: lowered_args,
                    line: *call_line,
                    column: *column,
                }
            }
            HirExpression::Constructor { type_, arguments } => {
                let type_name = type_.name().into_owned();
                let mut concrete_type = None;
                // plan-105-B: read the expected type's template head/arguments off
                // `UserOf` instead of re-splitting its rendered name.
                if let Some(ParameterType::UserOf(expected_name, expected_args)) = expected_type {
                    if expected_name.resolve() == type_name {
                        // Each type argument goes through `concrete_type_name` — the
                        // same walk `instantiate_type`'s other caller uses — so a
                        // nested user generic is instantiated to its MANGLED name
                        // before it becomes part of this instantiation's key.
                        // Passing the raw spelling instead made the two callers
                        // disagree: `Holder OF Holder OF Integer` mangled as
                        // `Holder$Holder$Integer` at the declaration and
                        // `Holder$Holder$OF$Integer` here, so the constructor's type
                        // failed to match its own binding (TYPE_BINDING_MISMATCH).
                        let mut args = Vec::with_capacity(expected_args.len());
                        for arg in expected_args {
                            args.push(self.concrete_type(arg, substitutions));
                        }
                        concrete_type = Some(self.instantiate_type(expected_name.resolve(), &args));
                    }
                }
                let field_types = concrete_type
                    .clone()
                    .or_else(|| Some(ParameterType::declared(&type_name)))
                    .and_then(|type_| self.record_fields(&type_).cloned());
                let lowered_args = arguments
                    .iter()
                    .enumerate()
                    .map(|(index, argument)| {
                        let expected_arg_type =
                            constructor_arg_field_type(argument, index, field_types.as_deref());
                        self.lower_constructor_arg(
                            argument,
                            substitutions,
                            context,
                            line,
                            expected_arg_type.as_ref(),
                        )
                    })
                    .collect::<Vec<_>>();
                let template_key = ParameterType::declared(&type_name);
                if concrete_type.is_none() && self.type_templates.contains_key(&template_key) {
                    let Some(template) = self.type_templates.get(&template_key).cloned() else {
                        unreachable!();
                    };
                    let mut inferred = HashMap::new();
                    let param_set: std::collections::HashSet<crate::intern::Symbol> = template
                        .template_params
                        .iter()
                        .map(|param| crate::intern::Symbol::intern(param))
                        .collect();
                    let fields = match template.kind {
                        TypeDeclKind::Type => template.fields.clone(),
                        // A union is never built by its own name — a member
                        // constructor builds it — so there is nothing to infer.
                        // Reported here with the verifier's own rule for the
                        // non-template union (`Shape[…]`), because the bare
                        // template name would otherwise reach the post-monomorph
                        // resolve and be misreported as an unknown type (bug-680).
                        TypeDeclKind::Union => {
                            self.report(
                                "TYPE_CONSTRUCTOR_REQUIRES_RECORD",
                                &format!("`{type_name}` is a UNION, not a record TYPE."),
                                line,
                            );
                            Vec::new()
                        }
                        TypeDeclKind::Enum => Vec::new(),
                    };
                    for (field, argument) in fields.iter().zip(lowered_args.iter()) {
                        if let Some(actual) =
                            self.expression_type(constructor_arg_value(argument), context)
                        {
                            unify_type(&field.type_, &actual, &param_set, &mut inferred);
                        }
                    }
                    let args = template
                        .template_params
                        .iter()
                        .map(|param| inferred.get(&crate::intern::Symbol::intern(param)).cloned())
                        .collect::<Option<Vec<_>>>();
                    if let Some(args) = args {
                        concrete_type = Some(self.instantiate_type(&type_name, &args));
                    }
                }
                HirExpression::Constructor {
                    type_: concrete_type.unwrap_or_else(|| ParameterType::declared(&type_name)),
                    arguments: lowered_args,
                }
            }
            HirExpression::WithUpdate { target, updates } => {
                let lowered_target =
                    self.lower_expression(target, substitutions, context, None, line);
                // An updated field's declared type is the expected type of its new
                // value, as it is for a constructor argument (bug-822).
                let field_types = self
                    .expression_type(&lowered_target, context)
                    .and_then(|type_| self.record_fields(&type_).cloned());
                HirExpression::WithUpdate {
                    target: Box::new(lowered_target),
                    updates: updates
                        .iter()
                        .map(|update| {
                            let expected = consults_expected(&update.value)
                                .then(|| {
                                    field_types.as_ref().and_then(|fields| {
                                        fields
                                            .iter()
                                            .find(|field| field.name == update.field)
                                            .map(|field| field.type_.clone())
                                    })
                                })
                                .flatten();
                            HirRecordUpdate {
                                field: update.field.clone(),
                                value: self.lower_expression(
                                    &update.value,
                                    substitutions,
                                    context,
                                    expected.as_ref(),
                                    update.line,
                                ),
                                line: update.line,
                            }
                        })
                        .collect(),
                }
            }
            HirExpression::ListLiteral(values) => HirExpression::ListLiteral(
                values
                    .iter()
                    .map(|value| {
                        let expected_element = expected_element_type(expected_type, false);
                        self.lower_expression(
                            value,
                            substitutions,
                            context,
                            expected_element.as_ref(),
                            line,
                        )
                    })
                    .collect(),
            ),
            HirExpression::SetLiteral {
                element_type,
                elements,
            } => HirExpression::SetLiteral {
                element_type: self.concrete_type(element_type, substitutions),
                elements: elements
                    .iter()
                    .map(|value| {
                        let expected_element = expected_element_type(expected_type, true);
                        self.lower_expression(
                            value,
                            substitutions,
                            context,
                            expected_element.as_ref(),
                            line,
                        )
                    })
                    .collect(),
            },
            HirExpression::MapLiteral {
                key_type,
                value_type,
                entries,
            } => HirExpression::MapLiteral {
                key_type: self.concrete_type(key_type, substitutions),
                value_type: self.concrete_type(value_type, substitutions),
                entries: entries
                    .iter()
                    .map(|(key, value)| {
                        // The literal spells its key and value types, so they are
                        // the expected types of every entry's operands (bug-822).
                        let expected_key = consults_expected(key)
                            .then(|| opt_type(&substitute_type_params(key_type, substitutions)))
                            .flatten();
                        let expected_value = consults_expected(value)
                            .then(|| opt_type(&substitute_type_params(value_type, substitutions)))
                            .flatten();
                        (
                            self.lower_expression(
                                key,
                                substitutions,
                                context,
                                expected_key.as_ref(),
                                line,
                            ),
                            self.lower_expression(
                                value,
                                substitutions,
                                context,
                                expected_value.as_ref(),
                                line,
                            ),
                        )
                    })
                    .collect(),
            },
            HirExpression::MemberAccess { target, member } => HirExpression::MemberAccess {
                target: Box::new(self.lower_expression(target, substitutions, context, None, line)),
                member: member.clone(),
            },
            HirExpression::Binary {
                left,
                operator,
                right,
                line: op_line,
                column,
            } => HirExpression::Binary {
                left: Box::new(self.lower_expression(left, substitutions, context, None, line)),
                operator: operator.clone(),
                right: Box::new(self.lower_expression(right, substitutions, context, None, line)),
                line: *op_line,
                column: *column,
            },
            HirExpression::Unary {
                operator,
                operand,
                line: op_line,
                column,
            } => HirExpression::Unary {
                operator: operator.clone(),
                operand: Box::new(self.lower_expression(
                    operand,
                    substitutions,
                    context,
                    None,
                    line,
                )),
                line: *op_line,
                column: *column,
            },
            HirExpression::Lambda {
                params,
                body,
                assign_target,
            } => {
                let mut nested = context.clone();
                let lowered_params = params
                    .iter()
                    .map(|param| {
                        nested.bound.insert(param.name.clone());
                        let mut lowered = param.clone();
                        if !matches!(param.type_, ParameterType::Unknown) {
                            let concrete = self.concrete_type(&param.type_, substitutions);
                            nested.locals.insert(param.name.clone(), concrete.clone());
                            lowered.type_ = concrete;
                        }
                        lowered
                    })
                    .collect::<Vec<_>>();
                HirExpression::Lambda {
                    params: lowered_params,
                    body: Box::new(self.lower_expression(
                        body,
                        substitutions,
                        &mut nested,
                        None,
                        line,
                    )),
                    assign_target: assign_target.clone(),
                }
            }
            HirExpression::Trapped {
                expression,
                binding,
                handler,
                line: trap_line,
            } => {
                let lowered_expression =
                    Box::new(self.lower_expression(expression, substitutions, context, None, line));
                let mut handler_context = context.clone();
                handler_context.bound.insert(binding.clone());
                handler_context
                    .locals
                    .insert(binding.clone(), ParameterType::named("Error"));
                let lowered_handler =
                    self.lower_statements(handler, substitutions, &mut handler_context);
                HirExpression::Trapped {
                    expression: lowered_expression,
                    binding: binding.clone(),
                    handler: lowered_handler,
                    line: *trap_line,
                }
            }
            HirExpression::Identifier(value) => HirExpression::Identifier(
                self.resolve_function_reference(value, expected_type, substitutions, context, line)
                    .unwrap_or_else(|| value.clone()),
            ),
            HirExpression::String(value) => HirExpression::String(value.clone()),
            HirExpression::Number(value) => HirExpression::Number(value.clone()),
            HirExpression::Scalar(code_point) => HirExpression::Scalar(*code_point),
            HirExpression::Boolean(value) => HirExpression::Boolean(*value),
        }
    }

    fn lower_constructor_arg(
        &mut self,
        argument: &HirConstructorArg,
        substitutions: &HashMap<crate::intern::Symbol, crate::types::ParameterType>,
        context: &mut FunctionContext,
        line: usize,
        expected_type: Option<&ParameterType>,
    ) -> HirConstructorArg {
        match argument {
            HirConstructorArg::Positional(value) => HirConstructorArg::Positional(
                self.lower_expression(value, substitutions, context, expected_type, line),
            ),
            HirConstructorArg::Named {
                name,
                value,
                line: arg_line,
            } => HirConstructorArg::Named {
                name: name.clone(),
                value: self.lower_expression(
                    value,
                    substitutions,
                    context,
                    expected_type,
                    *arg_line,
                ),
                line: *arg_line,
            },
        }
    }
}
