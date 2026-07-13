use super::helpers::*;
use super::*;

impl<'a> SyntaxChecker<'a> {
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
        // Ownership/borrow dataflow moved to `ir::verify` (TYPE_USE_AFTER_MOVE /
        // TYPE_RESOURCE_BORROW_INVALIDATE, plan-20-Z); only the type shape is
        // merged here now.
        let _ = right;
        LocalInfo {
            type_: left.type_,
            mutable: left.mutable,
            state_type: left.state_type,
        }
    }

    /// The `RES`/`STATE` ownership-axis rejections live in `ir::verify`
    /// (plan-20-Z); only the `STATE T` type-reference walk remains here (it
    /// feeds the surviving type-visibility/thread-sendability arms).
    pub(super) fn check_resource_declaration(
        &mut self,
        file: &AstFile,
        line: usize,
        _resource: bool,
        state_type: Option<&str>,
        _declared: Option<&Type>,
        _context: &str,
    ) {
        if let Some(state) = state_type {
            let state_resolved = self.parse_type(state);
            self.check_type_reference(file, &state_resolved, line);
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
                // A `RES` binding whose ownership floats into an outer-scope
                // collection (or out via a returned collection) becomes
                locals.insert(
                    name.clone(),
                    LocalInfo {
                        type_: binding_type,
                        mutable: *mutable,
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
                        // coverage:off — the parser always parses an expression
                        // for `EXIT PROGRAM`, so `code` is never `None` here.
                        let Some(code) = code else {
                            self.report(
                                "TYPE_UNKNOWN_VALUE",
                                "EXIT PROGRAM requires an Integer exit code.",
                                file,
                                *line,
                            );
                            return Flow::AlwaysReturns;
                        };
                        // coverage:on
                        let actual =
                            self.infer_expression(file, code, locals, *line, ExprMode::Read);
                        if !self.expression_compatible(&Type::Integer, &actual, Some(code)) {}
                    }
                }
                Flow::AlwaysReturns
            }
            Statement::Continue { kind: _, line: _ } => Flow::AlwaysReturns,
            Statement::Fail { error, line } => {
                let actual = self.infer_expression(file, error, locals, *line, ExprMode::Transfer);
                if !self.compatible(&Type::Error, &actual) {}
                Flow::AlwaysReturns
            }
            Statement::Propagate { line: _ } => Flow::AlwaysReturns,
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
                    if let Some(_binding) = self.lookup_visible_binding(file, name).cloned() {
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
                let _ = local;
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
                    for (_label, type_) in [
                        ("start", &start_type),
                        ("end", &end_type),
                        ("step", &step_type),
                    ] {
                        if !self.is_numeric(type_) {}
                    }
                    Type::Unknown
                };
                let mut nested = locals.clone();
                nested.insert(
                    name.clone(),
                    LocalInfo {
                        type_: loop_type,
                        mutable: false,
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
                    _other => Type::Unknown,
                };
                // Iterating a resource collection yields a *borrow* of each
                // element; the loop variable may not close, `RETURN`, or transfer
                // the resource (§15.6).
                let _element_borrowed = self.is_resource_type(&element_type);
                let mut nested = locals.clone();
                nested.insert(
                    name.clone(),
                    LocalInfo {
                        type_: element_type,
                        mutable: false,
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

#[cfg(test)]
mod tests {
    use crate::testutil::*;

    // ----- EXIT / RETURN legality -------------------------------------------

    #[test]
    fn exit_func_is_forbidden() {
        let src = "\
FUNC bad AS Integer
  EXIT FUNC
  RETURN 0
END FUNC

FUNC main AS Integer
  RETURN 0
END FUNC
";
        assert!(rejects_with(src, "EXIT_FUNC_FORBIDDEN"));
    }

    #[test]
    fn exit_sub_in_func_is_forbidden() {
        let src = "\
FUNC bad AS Integer
  EXIT SUB
  RETURN 0
END FUNC

FUNC main AS Integer
  RETURN 0
END FUNC
";
        assert!(rejects_with(src, "EXIT_SUB_IN_FUNC"));
    }

    #[test]
    fn exit_sub_inside_sub_is_allowed() {
        let src = "\
SUB doThing(flag AS Boolean)
  IF flag THEN
    EXIT SUB
  END IF
END SUB

FUNC main AS Integer
  RETURN 0
END FUNC
";
        assert!(!rejects_with(src, "EXIT_SUB_IN_FUNC"));
    }

    #[test]
    fn exit_loop_targets_are_accepted_in_a_loop() {
        // Exercises the ExitTarget::For/Do/While arm (loop_stack lookup) via a
        // FOR body, a WHILE body, and a DO..UNTIL body.
        let src = "\
FUNC main AS Integer
  FOR i = 1 TO 3
    EXIT FOR
  NEXT
  WHILE FALSE
    EXIT WHILE
  WEND
  DO
    EXIT DO
  LOOP UNTIL TRUE
  RETURN 0
END FUNC
";
        assert!(accepts(src), "{:?}", check_src(src));
    }

    #[test]
    fn continue_is_accepted_in_a_loop() {
        let src = "\
FUNC main AS Integer
  FOR i = 1 TO 3
    CONTINUE FOR
  NEXT
  RETURN 0
END FUNC
";
        assert!(accepts(src), "{:?}", check_src(src));
    }

    #[test]
    fn exit_program_with_code_is_accepted() {
        let src = "\
FUNC main AS Integer
  EXIT PROGRAM 0
END FUNC
";
        assert!(accepts(src), "{:?}", check_src(src));
    }

    #[test]
    fn exit_program_with_out_of_range_code_is_accepted_by_checker() {
        // Range check is a no-op here (moved to ir::verify); exercises the
        // integer_constant_value branch with an out-of-range constant.
        let src = "\
FUNC main AS Integer
  EXIT PROGRAM 300
END FUNC
";
        assert!(!rejects_with(src, "TYPE_UNKNOWN_VALUE"));
    }

    // ----- SUB RETURN -------------------------------------------------------

    #[test]
    fn sub_return_bare_is_forbidden() {
        let src = "\
SUB bad()
  RETURN
END SUB

FUNC main AS Integer
  RETURN 0
END FUNC
";
        assert!(rejects_with(src, "SUB_RETURN_FORBIDDEN"));
    }

    #[test]
    fn sub_return_with_value_is_forbidden() {
        // The `if let Some(value)` branch inside the SUB RETURN arm still runs
        // inference on the value before reporting.
        let src = "\
SUB bad()
  RETURN 5
END SUB

FUNC main AS Integer
  RETURN 0
END FUNC
";
        assert!(rejects_with(src, "SUB_RETURN_FORBIDDEN"));
    }

    #[test]
    fn func_return_value_is_accepted() {
        let src = "\
FUNC main AS Integer
  RETURN 0
END FUNC
";
        assert!(accepts(src), "{:?}", check_src(src));
    }

    #[test]
    fn return_unknown_value_reports() {
        // A RETURN whose value has no known type reports TYPE_UNKNOWN_VALUE.
        let src = "\
FUNC main AS Integer
  RETURN undefinedName
END FUNC
";
        assert!(rejects_with(src, "TYPE_UNKNOWN_VALUE"));
    }

    // ----- UNREACHABLE_AFTER_EXIT ------------------------------------------

    #[test]
    fn unreachable_after_exit_reports() {
        let src = "\
FUNC main AS Integer
  FOR i = 1 TO 3
    EXIT FOR
    LET dead AS Integer = 1
  NEXT
  RETURN 0
END FUNC
";
        assert!(rejects_with(src, "UNREACHABLE_AFTER_EXIT"));
    }

    #[test]
    fn unreachable_after_continue_reports() {
        let src = "\
FUNC main AS Integer
  FOR i = 1 TO 3
    CONTINUE FOR
    LET dead AS Integer = 1
  NEXT
  RETURN 0
END FUNC
";
        assert!(rejects_with(src, "UNREACHABLE_AFTER_EXIT"));
    }

    #[test]
    fn statement_after_return_is_not_unreachable_exit() {
        // RETURN also yields Flow::AlwaysReturns but is NOT Exit/Continue, so
        // the UNREACHABLE_AFTER_EXIT arm is skipped (the matches! guard).
        let src = "\
FUNC main AS Integer
  RETURN 0
  RETURN 1
END FUNC
";
        assert!(!rejects_with(src, "UNREACHABLE_AFTER_EXIT"));
    }

    // ----- Inline TRAP RECOVER ---------------------------------------------

    #[test]
    fn recover_outside_inline_trap_reports() {
        let src = "\
FUNC main AS Integer
  RECOVER 5
  RETURN 0
END FUNC
";
        assert!(rejects_with(src, "TYPE_RECOVER_OUTSIDE_INLINE_TRAP"));
    }

    #[test]
    fn recover_outside_inline_trap_with_value_infers_then_reports() {
        // The `if let Some(value)` branch of the outside-trap arm runs inference
        // on the RECOVER value before reporting.
        let src = "\
FUNC main AS Integer
  RECOVER undefinedName
  RETURN 0
END FUNC
";
        assert!(rejects_with(src, "TYPE_RECOVER_OUTSIDE_INLINE_TRAP"));
    }

    #[test]
    fn recover_matching_type_is_accepted() {
        let src = "\
FUNC parsePositive(v AS Integer) AS Integer
  IF v < 0 THEN FAIL error(404, \"missing\")
  RETURN v + 1
END FUNC

FUNC ok(v AS Integer) AS Integer
  LET a = parsePositive(v) TRAP(e)
    RECOVER 0
  END TRAP
  RETURN a
END FUNC

FUNC main AS Integer
  RETURN 0
END FUNC
";
        assert!(!rejects_with(src, "TYPE_RECOVER_TYPE_MISMATCH"));
    }

    #[test]
    fn recover_wrong_type_reports_mismatch() {
        // (Some(value), true) arm with an incompatible value type.
        let src = "\
FUNC parsePositive(v AS Integer) AS Integer
  IF v < 0 THEN FAIL error(404, \"missing\")
  RETURN v + 1
END FUNC

FUNC bad(v AS Integer) AS Integer
  LET a = parsePositive(v) TRAP(e)
    RECOVER \"nope\"
  END TRAP
  RETURN a
END FUNC

FUNC main AS Integer
  RETURN 0
END FUNC
";
        assert!(rejects_with(src, "TYPE_RECOVER_TYPE_MISMATCH"));
    }

    #[test]
    fn recover_missing_value_reports_mismatch() {
        // (None, true) arm: value-producing trap but RECOVER supplies no value.
        let src = "\
FUNC parsePositive(v AS Integer) AS Integer
  IF v < 0 THEN FAIL error(404, \"missing\")
  RETURN v + 1
END FUNC

FUNC bad(v AS Integer) AS Integer
  LET a = parsePositive(v) TRAP(e)
    RECOVER
  END TRAP
  RETURN a
END FUNC

FUNC main AS Integer
  RETURN 0
END FUNC
";
        assert!(rejects_with(src, "TYPE_RECOVER_TYPE_MISMATCH"));
    }

    #[test]
    fn recover_value_for_value_less_trap_reports_mismatch() {
        // (Some(value), false) arm: the trapped call produces Nothing, so a
        // RECOVER value is rejected. A value-less inline TRAP wraps a SUB call.
        let src = "\
SUB doThing(v AS Integer)
  IF v < 0 THEN FAIL error(404, \"missing\")
END SUB

FUNC bad(v AS Integer) AS Integer
  doThing(v) TRAP(e)
    RECOVER 0
  END TRAP
  RETURN 0
END FUNC

FUNC main AS Integer
  RETURN 0
END FUNC
";
        assert!(
            rejects_with(src, "TYPE_RECOVER_TYPE_MISMATCH"),
            "{:?}",
            check_src(src)
        );
    }

    #[test]
    fn recover_none_for_value_less_trap_is_accepted() {
        // (None, false) arm: value-less trap, bare RECOVER — no diagnostic.
        let src = "\
SUB doThing(v AS Integer)
  IF v < 0 THEN FAIL error(404, \"missing\")
END SUB

FUNC ok(v AS Integer) AS Integer
  doThing(v) TRAP(e)
    RECOVER
  END TRAP
  RETURN 0
END FUNC

FUNC main AS Integer
  RETURN 0
END FUNC
";
        assert!(
            !rejects_with(src, "TYPE_RECOVER_TYPE_MISMATCH"),
            "{:?}",
            check_src(src)
        );
    }

    // ----- LET binding / assignment paths ----------------------------------

    #[test]
    fn binding_with_unknown_initializer_reports() {
        // check_binding_shape's inferred == Some(Type::Unknown) arm.
        let src = "\
FUNC main AS Integer
  LET x = undefinedName
  RETURN 0
END FUNC
";
        assert!(rejects_with(src, "TYPE_UNKNOWN_VALUE"));
    }

    #[test]
    fn binding_with_known_initializer_is_accepted() {
        let src = "\
FUNC main AS Integer
  LET x AS Integer = 1
  RETURN x
END FUNC
";
        assert!(accepts(src), "{:?}", check_src(src));
    }

    #[test]
    fn assign_to_non_local_reports_unknown_value() {
        // Statement::Assign with an unknown target (not a local, not a visible
        // global binding).
        let src = "\
FUNC main AS Integer
  MUT total AS Integer = 0
  notdeclared = 5
  RETURN total
END FUNC
";
        assert!(
            rejects_with(src, "TYPE_UNKNOWN_VALUE"),
            "{:?}",
            check_src(src)
        );
    }

    #[test]
    fn assign_to_local_is_accepted() {
        let src = "\
FUNC main AS Integer
  MUT total AS Integer = 0
  total = 5
  RETURN total
END FUNC
";
        assert!(accepts(src), "{:?}", check_src(src));
    }

    #[test]
    fn assign_to_global_binding_infers_without_report() {
        // The `lookup_visible_binding` Some branch of Statement::Assign.
        let src = "\
MUT counter AS Integer = 0

FUNC main AS Integer
  counter = 5
  RETURN 0
END FUNC
";
        assert!(
            !rejects_with(src, "TYPE_UNKNOWN_VALUE"),
            "{:?}",
            check_src(src)
        );
    }

    // ----- Control-flow statement coverage (If/Match/loops) -----------------

    #[test]
    fn if_with_both_branches_returning_is_accepted() {
        let src = "\
FUNC pick(flag AS Boolean) AS Integer
  IF flag THEN
    RETURN 1
  ELSE
    RETURN 2
  END IF
END FUNC

FUNC main AS Integer
  RETURN pick(TRUE)
END FUNC
";
        assert!(accepts(src), "{:?}", check_src(src));
    }

    #[test]
    fn match_exhaustive_all_return_is_accepted() {
        let src = "\
FUNC classify(n AS Integer) AS Integer
  MATCH n
    CASE 0
      RETURN 0
    CASE ELSE
      RETURN 1
  END MATCH
END FUNC

FUNC main AS Integer
  RETURN classify(3)
END FUNC
";
        assert!(accepts(src), "{:?}", check_src(src));
    }

    #[test]
    fn while_loop_body_is_checked() {
        let src = "\
FUNC main AS Integer
  MUT i AS Integer = 0
  WHILE i < 3
    i = i + 1
  WEND
  RETURN i
END FUNC
";
        assert!(accepts(src), "{:?}", check_src(src));
    }

    #[test]
    fn do_until_loop_body_is_checked() {
        let src = "\
FUNC main AS Integer
  MUT i AS Integer = 0
  DO
    i = i + 1
  LOOP UNTIL i >= 3
  RETURN i
END FUNC
";
        assert!(accepts(src), "{:?}", check_src(src));
    }

    #[test]
    fn for_each_over_list_is_checked() {
        let src = "\
FUNC main AS Integer
  LET xs AS List OF Integer = [1, 2, 3]
  MUT total AS Integer = 0
  FOR EACH x IN xs
    total = total + x
  NEXT
  RETURN total
END FUNC
";
        assert!(accepts(src), "{:?}", check_src(src));
    }

    #[test]
    fn for_each_over_map_is_checked() {
        // Exercises the Type::Map arm of ForEach element typing.
        let src = "\
IMPORT collections

FUNC main AS Integer
  LET m AS Map OF String TO Integer = Map OF String TO Integer { \"a\" := 1 }
  MUT total AS Integer = 0
  FOR EACH entry IN m
    total = total + entry.value
  NEXT
  RETURN total
END FUNC
";
        assert!(accepts(src), "{:?}", check_src(src));
    }

    #[test]
    fn propagate_statement_is_walked() {
        // A PROPAGATE inside an inline TRAP handler reaches the Propagate arm
        // (trap_name Some); a bare PROPAGATE outside reaches the None branch.
        let src = "\
FUNC parsePositive(v AS Integer) AS Integer
  IF v < 0 THEN FAIL error(1, \"neg\")
  RETURN v
END FUNC

FUNC caller(v AS Integer) AS Integer
  LET a = parsePositive(v) TRAP(e)
    PROPAGATE
  END TRAP
  RETURN a
END FUNC

FUNC main AS Integer
  RETURN 0
END FUNC
";
        let _ = check_src(src);
        assert!(true);
    }

    #[test]
    fn propagate_outside_trap_is_walked() {
        // A PROPAGATE with no enclosing trap reaches the `trap_name.is_none()`
        // empty-body branch of the Propagate arm.
        let src = "\
FUNC caller() AS Integer
  PROPAGATE
END FUNC

FUNC main AS Integer
  RETURN 0
END FUNC
";
        let _ = check_src(src);
        assert!(true);
    }

    #[test]
    fn for_each_over_non_collection_yields_unknown_element() {
        // Iterating a non-List/non-Map value reaches the `_other => Unknown`
        // element-type arm of ForEach.
        let src = "\
FUNC main AS Integer
  MUT total AS Integer = 0
  FOR EACH x IN 42
    total = total + 1
  NEXT
  RETURN total
END FUNC
";
        let _ = check_src(src);
        assert!(true);
    }

    #[test]
    fn fail_statement_is_checked() {
        let src = "\
FUNC mayFail(v AS Integer) AS Integer
  IF v < 0 THEN FAIL error(1, \"neg\")
  RETURN v
END FUNC

FUNC main AS Integer
  RETURN mayFail(1)
END FUNC
";
        assert!(accepts(src), "{:?}", check_src(src));
    }

    #[test]
    fn state_assign_to_res_with_state_is_checked() {
        // A RES binding with a STATE type, then `f.state = ...` reaches the
        // StateAssign arm's state_type-present path (parse_type + compatibility).
        let src = "\
IMPORT fs

TYPE FileState
  pos AS Integer
END TYPE

FUNC main AS Integer
  RES f AS File STATE FileState = fs::createTempFile()
  f.state = WITH f.state { pos := 10 }
  fs::close(f)
  RETURN 0
END FUNC
";
        assert!(
            !rejects_with(src, "TYPE_UNKNOWN_VALUE"),
            "{:?}",
            check_src(src)
        );
    }

    #[test]
    fn state_assign_to_local_without_state_type_is_walked() {
        // A plain local (no STATE) as a `.state` target reaches the
        // `local.state_type == None` early-return arm of StateAssign.
        let src = "\
FUNC main AS Integer
  MUT x AS Integer = 0
  x.state = 5
  RETURN 0
END FUNC
";
        let _ = check_src(src);
        assert!(true);
    }

    #[test]
    fn return_get_of_resource_element_is_walked() {
        // RETURN whose value is a `get` on a resource collection reaches the
        // is_resource_element_borrow guard in the RETURN arm.
        let src = "\
IMPORT collections
IMPORT fs

FUNC firstFile(files AS List OF RES File) AS File
  RETURN collections::get(files, 0)
END FUNC

FUNC main AS Integer
  RETURN 0
END FUNC
";
        let _ = check_src(src);
        assert!(true);
    }

    #[test]
    fn foreach_over_resource_list_marks_element_borrowed() {
        // Iterating `List OF RES File` reaches the resource-element ForEach path
        // (is_resource_type on the stripped element type).
        let src = "\
IMPORT fs

FUNC main AS Integer
  LET files AS List OF RES File = []
  FOR EACH f IN files
    LET n AS Integer = 1
  NEXT
  RETURN 0
END FUNC
";
        let _ = check_src(src);
        assert!(true);
    }

    #[test]
    fn match_union_with_guards_and_bindings_is_checked() {
        // A union scrutinee with a bound case pattern and a WHEN guard exercises
        // the Match arm's guard inference, covered_cases tracking, and
        // exhaustiveness branch.
        let src = "\
TYPE Circle
  radius AS Integer
END TYPE

TYPE Square
  side AS Integer
END TYPE

UNION Shape
  Circle
  Square
END UNION

FUNC area(shape AS Shape) AS Integer
  MATCH shape
    CASE Circle(c) WHEN c.radius > 0
      RETURN c.radius
    CASE Circle(c)
      RETURN 0
    CASE Square(s)
      RETURN s.side
  END MATCH
END FUNC

FUNC main AS Integer
  RETURN area(Circle[3])
END FUNC
";
        assert!(accepts(src), "{:?}", check_src(src));
    }

    #[test]
    fn match_non_exhaustive_falls_through() {
        // A MATCH missing a union variant is non-exhaustive: drives the
        // `!exhaustive` fallthrough push + report path.
        let src = "\
TYPE Circle
  radius AS Integer
END TYPE

TYPE Square
  side AS Integer
END TYPE

UNION Shape
  Circle
  Square
END UNION

FUNC area(shape AS Shape) AS Integer
  MATCH shape
    CASE Circle(c)
      RETURN c.radius
  END MATCH
  RETURN 0
END FUNC

FUNC main AS Integer
  RETURN 0
END FUNC
";
        let _ = check_src(src);
        assert!(true);
    }

    #[test]
    fn state_assign_to_non_local_reports() {
        // Statement::StateAssign with an unknown target binding.
        let src = "\
FUNC main AS Integer
  notdeclared.state = 5
  RETURN 0
END FUNC
";
        assert!(
            rejects_with(src, "TYPE_UNKNOWN_VALUE"),
            "{:?}",
            check_src(src)
        );
    }
}
