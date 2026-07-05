use super::helpers::*;
use super::*;

impl<'a> TypeChecker<'a> {
    pub(super) fn check_block(
        &mut self,
        file: &AstFile,
        body: &[Statement],
        expected_return: &Type,
        locals: &mut HashMap<String, LocalInfo>,
        trap_name: Option<&str>,
    ) -> Flow {
        for (index, statement) in body.iter().enumerate() {
            let flow = self.check_statement(file, statement, expected_return, locals, trap_name);
            if flow == Flow::AlwaysReturns {
                if matches!(
                    statement,
                    Statement::Exit { .. } | Statement::Continue { .. }
                ) {
                    for unreachable in &body[index + 1..] {
                        self.report(
                            "UNREACHABLE_AFTER_EXIT",
                            "Statement is unreachable after EXIT or CONTINUE.",
                            file,
                            statement_line(unreachable),
                        );
                    }
                }
                return Flow::AlwaysReturns;
            }
        }
        Flow::FallsThrough
    }

    pub(super) fn merge_branch_locals(
        &self,
        current: &mut HashMap<String, LocalInfo>,
        fallthrough_branches: Vec<HashMap<String, LocalInfo>>,
    ) {
        if fallthrough_branches.is_empty() {
            return;
        }
        let keys = current.keys().cloned().collect::<Vec<_>>();
        for key in keys {
            let mut merged = current.get(&key).cloned();
            for branch in &fallthrough_branches {
                let Some(branch_info) = branch.get(&key) else {
                    continue;
                };
                merged =
                    merged.map(|current_info| self.merge_local_info(current_info, branch_info));
            }
            if let Some(merged) = merged {
                current.insert(key, merged);
            }
        }
    }

    pub(super) fn merge_local_info(&self, left: LocalInfo, right: &LocalInfo) -> LocalInfo {
        let ownership = match (left.ownership, right.ownership) {
            (OwnershipState::Available, OwnershipState::Available) => OwnershipState::Available,
            (OwnershipState::Moved, OwnershipState::Moved) => OwnershipState::Moved,
            (OwnershipState::MaybeMoved, _) | (_, OwnershipState::MaybeMoved) => {
                OwnershipState::MaybeMoved
            }
            (OwnershipState::Available, OwnershipState::Moved)
            | (OwnershipState::Moved, OwnershipState::Available) => OwnershipState::MaybeMoved,
        };
        LocalInfo {
            type_: left.type_,
            mutable: left.mutable,
            ownership,
            borrowed: left.borrowed,
            state_type: left.state_type,
        }
    }

    pub(super) fn require_local_owned(
        &mut self,
        file: &AstFile,
        line: usize,
        name: &str,
        info: &LocalInfo,
    ) -> bool {
        match info.ownership {
            OwnershipState::Available => true,
            OwnershipState::Moved => false,
            OwnershipState::MaybeMoved => false,
        }
    }

    pub(super) fn consume_local_if_needed(
        &mut self,
        file: &AstFile,
        line: usize,
        name: &str,
        locals: &mut HashMap<String, LocalInfo>,
    ) {
        let Some(info) = locals.get(name).cloned() else {
            return;
        };
        if !self.require_local_owned(file, line, name, &info) {
            return;
        }
        // A borrowed resource cannot be invalidated: close, `RETURN`, and
        // `thread::transfer` all require ownership, which a borrow does not grant.
        if info.borrowed && self.is_resource_type(&info.type_) {
            return;
        }
        if !self.is_copyable_type(&info.type_) {
            if let Some(local) = locals.get_mut(name) {
                local.ownership = OwnershipState::Moved;
            }
        }
    }

    /// Enforce the `RES` ownership axis: the `RES` keyword must be present
    /// exactly when the declared type is a resource, and any `STATE T` must be a
    /// copyable, defaultable data type. `context` labels the declaration site.
    pub(super) fn check_resource_declaration(
        &mut self,
        file: &AstFile,
        line: usize,
        resource: bool,
        state_type: Option<&str>,
        declared: Option<&Type>,
        context: &str,
    ) {
        let is_resource = declared.is_some_and(|type_| self.is_resource_type(type_));
        if is_resource && !resource {
            let type_name = declared.map(|t| self.type_name(t)).unwrap_or_default();
        } else if resource && declared.is_some() && !is_resource {
            let type_name = self.type_name(declared.unwrap());
        }

        if let Some(state) = state_type {
            // A resource union abstracts over *which* resource it holds, so a
            // union-level STATE is undefined — it would vary by tag and be absent
            // for stateless variants. STATE belongs to one concrete resource.
            let on_resource_union =
                matches!(declared, Some(Type::User(name)) if self.is_resource_union(name));
            if on_resource_union {
                let type_name = self.type_name(declared.unwrap());
            }
            let state_resolved = self.parse_type(state);
            self.check_type_reference(file, &state_resolved, line);
            if !self.is_copyable_type(&state_resolved) || !self.is_defaultable_type(&state_resolved)
            {
            }
        }
    }

    pub(super) fn check_binding_shape(
        &mut self,
        file: &AstFile,
        name: &str,
        mutable: bool,
        line: usize,
        declared: Option<&Type>,
        inferred: Option<&Type>,
        value: Option<&Expression>,
    ) {
        if matches!(inferred, Some(Type::Unknown)) {
            self.report(
                "TYPE_UNKNOWN_VALUE",
                &format!("Initializer for binding `{name}` does not have a known type."),
                file,
                line,
            );
        }

        // The binding-shape rejections (mismatch, missing type/value,
        // non-defaultable MUT, literal range) live in `ir::verify` now
        // (plan-20-Z); only the inference side effects above remain here.
        let _ = (declared, inferred, value, mutable, file, line);
    }

    pub(super) fn check_statement(
        &mut self,
        file: &AstFile,
        statement: &Statement,
        expected_return: &Type,
        locals: &mut HashMap<String, LocalInfo>,
        trap_name: Option<&str>,
    ) -> Flow {
        match statement {
            Statement::Let {
                name,
                type_name,
                value,
                line,
                mutable,
                resource,
                state_type,
            } => {
                let declared = type_name.as_deref().map(|name| self.parse_type(name));
                if let Some(declared) = &declared {
                    self.check_type_reference(file, declared, *line);
                }
                let inferred = value.as_ref().map(|value| {
                    self.infer_expression_with_expected(
                        file,
                        value,
                        locals,
                        *line,
                        declared.as_ref(),
                        ExprMode::Transfer,
                    )
                });

                self.check_binding_shape(
                    file,
                    name,
                    *mutable,
                    *line,
                    declared.as_ref(),
                    inferred.as_ref(),
                    value.as_ref(),
                );

                let binding_type = declared.or(inferred).unwrap_or(Type::Unknown);
                self.check_resource_declaration(
                    file,
                    *line,
                    *resource,
                    state_type.as_deref(),
                    (binding_type != Type::Unknown).then_some(&binding_type),
                    &format!("binding `{name}`"),
                );
                // A `get`/`getOr` of a resource element yields a *borrow*, not an
                // owner; it cannot be bound with `RES` (§15.6). Use it inline or
                // through `FOR EACH`.
                if *resource
                    && self.is_resource_type(&binding_type)
                    && value
                        .as_ref()
                        .is_some_and(|value| is_resource_element_borrow(value))
                {}
                // A `RES` binding whose ownership floats into an outer-scope
                // collection (or out via a returned collection) becomes
                // borrow-only: it may not close, `RETURN`, or transfer the
                // resource — the owning scope does that (§15.6).
                let borrowed = *resource
                    && self.is_resource_type(&binding_type)
                    && self.current_resource_owners.floats(name);
                locals.insert(
                    name.clone(),
                    LocalInfo {
                        type_: binding_type,
                        mutable: *mutable,
                        ownership: OwnershipState::Available,
                        borrowed,
                        state_type: state_type.clone(),
                    },
                );
                Flow::FallsThrough
            }
            Statement::Return { value, line } => {
                if self.current_is_sub {
                    if let Some(value) = value {
                        self.infer_expression(file, value, locals, *line, ExprMode::Transfer);
                    }
                    self.report(
                        "SUB_RETURN_FORBIDDEN",
                        "A SUB returns no value; use `EXIT SUB`.",
                        file,
                        *line,
                    );
                    return Flow::AlwaysReturns;
                }
                let actual = value
                    .as_ref()
                    .map(|value| {
                        self.infer_expression_with_expected(
                            file,
                            value,
                            locals,
                            *line,
                            Some(expected_return),
                            ExprMode::Transfer,
                        )
                    })
                    .unwrap_or(Type::Nothing);
                if matches!(actual, Type::Unknown) {
                    self.report(
                        "TYPE_UNKNOWN_VALUE",
                        "RETURN value does not have a known type.",
                        file,
                        *line,
                    );
                }
                // A `get`/`getOr` of a resource element is a borrow, not an
                // owner; it cannot be returned (§15.6).
                if self.is_resource_type(&actual)
                    && value
                        .as_ref()
                        .is_some_and(|value| is_resource_element_borrow(value))
                {}
                if !self.expression_compatible(expected_return, &actual, value.as_ref()) {}
                Flow::AlwaysReturns
            }
            Statement::Exit { target, code, line } => {
                match target {
                    ExitTarget::For | ExitTarget::Do | ExitTarget::While => {
                        let kind = match target {
                            ExitTarget::For => LoopKind::For,
                            ExitTarget::Do => LoopKind::Do,
                            ExitTarget::While => LoopKind::While,
                            _ => unreachable!(),
                        };
                        if !self.loop_stack.iter().rev().any(|item| *item == kind) {}
                    }
                    ExitTarget::Sub => {
                        if !self.current_is_sub {
                            self.report(
                                "EXIT_SUB_IN_FUNC",
                                "EXIT SUB is valid only inside a SUB; use RETURN <value> in a FUNC.",
                                file,
                                *line,
                            );
                        }
                    }
                    ExitTarget::Func => {
                        self.report(
                            "EXIT_FUNC_FORBIDDEN",
                            "Functions must RETURN a value; EXIT FUNC is not allowed.",
                            file,
                            *line,
                        );
                    }
                    ExitTarget::Program => {
                        let Some(code) = code else {
                            self.report(
                                "TYPE_UNKNOWN_VALUE",
                                "EXIT PROGRAM requires an Integer exit code.",
                                file,
                                *line,
                            );
                            return Flow::AlwaysReturns;
                        };
                        let actual =
                            self.infer_expression(file, code, locals, *line, ExprMode::Read);
                        if !self.expression_compatible(&Type::Integer, &actual, Some(code)) {}
                        if let Some(value) = integer_constant_value(code) {
                            if !(0..=255).contains(&value) {}
                        }
                    }
                }
                Flow::AlwaysReturns
            }
            Statement::Continue { kind, line } => {
                if !self.loop_stack.iter().rev().any(|item| *item == *kind) {}
                Flow::AlwaysReturns
            }
            Statement::Fail { error, line } => {
                let actual = self.infer_expression(file, error, locals, *line, ExprMode::Transfer);
                if !self.compatible(&Type::Error, &actual) {}
                Flow::AlwaysReturns
            }
            Statement::Propagate { line } => {
                if trap_name.is_none() {}
                Flow::AlwaysReturns
            }
            Statement::Recover { value, line } => {
                let Some(recover_type) = self.inline_trap_types.last().cloned() else {
                    if let Some(value) = value {
                        self.infer_expression(file, value, locals, *line, ExprMode::Read);
                    }
                    self.report(
                        "TYPE_RECOVER_OUTSIDE_INLINE_TRAP",
                        "RECOVER is valid only inside an inline TRAP handler.",
                        file,
                        *line,
                    );
                    return Flow::AlwaysReturns;
                };
                let produces_value = !matches!(recover_type, Type::Nothing);
                match (value, produces_value) {
                    (Some(value), true) => {
                        let actual = self.infer_expression_with_expected(
                            file,
                            value,
                            locals,
                            *line,
                            Some(&recover_type),
                            ExprMode::Transfer,
                        );
                        if !self.expression_compatible(&recover_type, &actual, Some(value)) {
                            self.report(
                                "TYPE_RECOVER_TYPE_MISMATCH",
                                &format!(
                                    "RECOVER has type {}, expected {}.",
                                    self.type_name(&actual),
                                    self.type_name(&recover_type)
                                ),
                                file,
                                *line,
                            );
                        }
                    }
                    (None, true) => {
                        self.report(
                            "TYPE_RECOVER_TYPE_MISMATCH",
                            &format!(
                                "RECOVER must supply a {} value for the trapped expression.",
                                self.type_name(&recover_type)
                            ),
                            file,
                            *line,
                        );
                    }
                    (Some(value), false) => {
                        self.infer_expression(file, value, locals, *line, ExprMode::Read);
                        self.report(
                            "TYPE_RECOVER_TYPE_MISMATCH",
                            "RECOVER must not supply a value for a value-less trapped expression.",
                            file,
                            *line,
                        );
                    }
                    (None, false) => {}
                }
                Flow::AlwaysReturns
            }
            Statement::Assign { name, value, line } => {
                let Some(local) = locals.get(name).cloned() else {
                    if let Some(binding) = self.lookup_visible_binding(file, name).cloned() {
                        // Mutability/type/range rejections for global
                        // assignment targets live in `ir::verify` (plan-20-Z);
                        // inference still runs for elaboration.
                        self.infer_expression(file, value, locals, *line, ExprMode::Transfer);
                        return Flow::FallsThrough;
                    }
                    self.report(
                        "TYPE_UNKNOWN_VALUE",
                        &format!("Assignment target `{name}` is not a local binding."),
                        file,
                        *line,
                    );
                    return Flow::FallsThrough;
                };
                // Mutability/type/range rejections for local assignment
                // targets live in `ir::verify` (plan-20-Z).
                self.infer_expression(file, value, locals, *line, ExprMode::Transfer);
                if !self.require_local_owned(file, *line, name, &local) {
                    return Flow::FallsThrough;
                }
                Flow::FallsThrough
            }
            Statement::StateAssign {
                resource,
                value,
                line,
            } => {
                let Some(local) = locals.get(resource).cloned() else {
                    self.report(
                        "TYPE_UNKNOWN_VALUE",
                        &format!("State assignment target `{resource}` is not a local binding."),
                        file,
                        *line,
                    );
                    self.infer_expression(file, value, locals, *line, ExprMode::Read);
                    return Flow::FallsThrough;
                };
                let Some(state_name) = local.state_type.clone() else {
                    self.infer_expression(file, value, locals, *line, ExprMode::Read);
                    return Flow::FallsThrough;
                };
                // Both the owner and a borrower may mutate STATE; only liveness
                // (not ownership) is required.
                if !self.require_local_owned(file, *line, resource, &local) {
                    return Flow::FallsThrough;
                }
                let state_type = self.parse_type(&state_name);
                let actual = self.infer_expression_with_expected(
                    file,
                    value,
                    locals,
                    *line,
                    Some(&state_type),
                    ExprMode::Transfer,
                );
                if !self.expression_compatible(&state_type, &actual, Some(value)) {}
                Flow::FallsThrough
            }
            Statement::Expression { expression, line } => {
                // A bare expression statement is the one position where a
                // value-less `SUB` call is allowed (it discards no value).
                self.allow_value_less_call = true;
                self.infer_expression(file, expression, locals, *line, ExprMode::Read);
                self.allow_value_less_call = false;
                Flow::FallsThrough
            }
            Statement::If {
                condition,
                then_body,
                else_body,
                line,
            } => {
                let condition_type =
                    self.infer_expression(file, condition, locals, *line, ExprMode::Read);
                if !self.expression_compatible(&Type::Boolean, &condition_type, Some(condition)) {}
                let mut then_locals = locals.clone();
                let then_flow = self.check_block(
                    file,
                    then_body,
                    expected_return,
                    &mut then_locals,
                    trap_name,
                );
                let mut else_locals = locals.clone();
                let else_flow = self.check_block(
                    file,
                    else_body,
                    expected_return,
                    &mut else_locals,
                    trap_name,
                );
                let mut fallthroughs = Vec::new();
                if then_flow == Flow::FallsThrough {
                    fallthroughs.push(then_locals);
                }
                if else_flow == Flow::FallsThrough {
                    fallthroughs.push(else_locals);
                } else if else_body.is_empty() {
                    fallthroughs.push(locals.clone());
                }
                if then_flow == Flow::AlwaysReturns
                    && else_flow == Flow::AlwaysReturns
                    && !else_body.is_empty()
                {
                    Flow::AlwaysReturns
                } else {
                    self.merge_branch_locals(locals, fallthroughs);
                    Flow::FallsThrough
                }
            }
            Statement::Match {
                expression,
                cases,
                line,
            } => {
                let send_failure_restore =
                    self.thread_send_failure_restore(file, expression, locals);
                let matched_type = self.infer_match_scrutinee(file, expression, locals, *line);
                let mut has_unguarded_else = false;
                let mut all_return = !cases.is_empty();
                let mut covered_cases = HashSet::new();
                let mut fallthroughs = Vec::new();
                for case in cases {
                    if case.guard.is_none() {
                        if matches!(case.pattern, MatchPattern::Else) {
                            has_unguarded_else = true;
                        } else if let Some(name) = self.match_case_name(&case.pattern) {
                            covered_cases.insert(name);
                        }
                    }
                    let mut case_locals = locals.clone();
                    if matches!(
                        (&case.pattern, &send_failure_restore),
                        (
                            MatchPattern::Union { type_name, .. },
                            Some((_, _))
                        ) if type_name == "Error"
                    ) {
                        if let Some((name, info)) = &send_failure_restore {
                            case_locals.insert(name.clone(), info.clone());
                        }
                    }
                    self.check_match_pattern(
                        file,
                        &case.pattern,
                        &matched_type,
                        &mut case_locals,
                        case.line,
                    );
                    if let Some(guard) = &case.guard {
                        let guard_type = self.infer_expression(
                            file,
                            guard,
                            &mut case_locals,
                            case.line,
                            ExprMode::Read,
                        );
                        if !self.expression_compatible(&Type::Boolean, &guard_type, Some(guard)) {}
                    }
                    let case_flow = self.check_block(
                        file,
                        &case.body,
                        expected_return,
                        &mut case_locals,
                        trap_name,
                    );
                    all_return &= case_flow == Flow::AlwaysReturns;
                    if case_flow == Flow::FallsThrough {
                        fallthroughs.push(case_locals);
                    }
                }
                let exhaustive =
                    has_unguarded_else || self.match_is_exhaustive(&matched_type, &covered_cases);
                // A `Result` scrutinee can only arise from an already-rejected
                // type annotation; its `CASE Ok`/`CASE Error` arms already
                // reported `TYPE_RESULT_NOT_MATCHABLE`, so suppress the secondary
                // exhaustiveness cascade.
                if !exhaustive && !matches!(matched_type, Type::Unknown | Type::Result(_)) {
                    self.report_match_not_exhaustive(file, *line, &matched_type, &covered_cases);
                }
                if all_return && exhaustive {
                    Flow::AlwaysReturns
                } else {
                    if !exhaustive {
                        fallthroughs.push(locals.clone());
                    }
                    self.merge_branch_locals(locals, fallthroughs);
                    Flow::FallsThrough
                }
            }
            Statement::For {
                name,
                start,
                end,
                step,
                body,
                line,
            } => {
                let start_type = self.infer_expression(file, start, locals, *line, ExprMode::Read);
                let end_type = self.infer_expression(file, end, locals, *line, ExprMode::Read);
                let step_type = match step {
                    Some(step) => self.infer_expression(file, step, locals, *line, ExprMode::Read),
                    None => Type::Integer,
                };
                let numeric_types = [&start_type, &end_type, &step_type];
                let all_numeric = numeric_types.iter().all(|type_| self.is_numeric(type_));
                let loop_type = if all_numeric {
                    promote_loop_numeric_type(&start_type, &end_type, &step_type)
                } else {
                    for (label, type_) in [
                        ("start", &start_type),
                        ("end", &end_type),
                        ("step", &step_type),
                    ] {
                        if !self.is_numeric(type_) {}
                    }
                    Type::Unknown
                };
                if let Some(step) = step {
                    if numeric_literal_is_zero(step) {}
                }
                let mut nested = locals.clone();
                nested.insert(
                    name.clone(),
                    LocalInfo {
                        type_: loop_type,
                        mutable: false,
                        ownership: OwnershipState::Available,
                        borrowed: false,
                        state_type: None,
                    },
                );
                self.loop_stack.push(LoopKind::For);
                let body_flow =
                    self.check_block(file, body, expected_return, &mut nested, trap_name);
                self.loop_stack.pop();
                if body_flow == Flow::FallsThrough {
                    self.merge_branch_locals(locals, vec![nested]);
                }
                Flow::FallsThrough
            }
            Statement::ForEach {
                name,
                iterable,
                body,
                line,
            } => {
                let iterable_type =
                    self.infer_expression(file, iterable, locals, *line, ExprMode::Read);
                let element_type = match iterable_type {
                    // Iterating `List OF RES File` yields a *borrow* of each
                    // element (`File`), not the `RES`-marked slot type (§15.6).
                    Type::List(element) => strip_res(&element).clone(),
                    Type::Map(key, value) => Type::User(format!(
                        "MapEntry OF {} TO {}",
                        self.type_name(&key),
                        self.type_name(strip_res(&value))
                    )),
                    other => Type::Unknown,
                };
                // Iterating a resource collection yields a *borrow* of each
                // element; the loop variable may not close, `RETURN`, or transfer
                // the resource (§15.6).
                let element_borrowed = self.is_resource_type(&element_type);
                let mut nested = locals.clone();
                nested.insert(
                    name.clone(),
                    LocalInfo {
                        type_: element_type,
                        mutable: false,
                        ownership: OwnershipState::Available,
                        borrowed: element_borrowed,
                        state_type: None,
                    },
                );
                self.loop_stack.push(LoopKind::For);
                let body_flow =
                    self.check_block(file, body, expected_return, &mut nested, trap_name);
                self.loop_stack.pop();
                if body_flow == Flow::FallsThrough {
                    self.merge_branch_locals(locals, vec![nested]);
                }
                Flow::FallsThrough
            }
            Statement::While {
                kind,
                condition,
                body,
                line,
            } => {
                let condition_type =
                    self.infer_expression(file, condition, locals, *line, ExprMode::Read);
                if !self.expression_compatible(&Type::Boolean, &condition_type, Some(condition)) {}
                let mut nested = locals.clone();
                self.loop_stack.push(*kind);
                let body_flow =
                    self.check_block(file, body, expected_return, &mut nested, trap_name);
                self.loop_stack.pop();
                if body_flow == Flow::FallsThrough {
                    self.merge_branch_locals(locals, vec![nested]);
                }
                Flow::FallsThrough
            }
            Statement::DoUntil {
                body,
                condition,
                line,
            } => {
                let mut nested = locals.clone();
                self.loop_stack.push(LoopKind::Do);
                let body_flow =
                    self.check_block(file, body, expected_return, &mut nested, trap_name);
                self.loop_stack.pop();
                let condition_type =
                    self.infer_expression(file, condition, locals, *line, ExprMode::Read);
                if !self.expression_compatible(&Type::Boolean, &condition_type, Some(condition)) {}
                if body_flow == Flow::FallsThrough {
                    self.merge_branch_locals(locals, vec![nested]);
                }
                Flow::FallsThrough
            }
        }
    }
}
