use super::*;

// ---------------------------------------------------------------------------
// Placement validation (expect* only inside a TCASE)
// ---------------------------------------------------------------------------

/// Report every assertion builtin used outside a `TCASE` body. The `TESTING`
/// blocks themselves are validated in-place (their case bodies are the only legal
/// home for `expect*`); every other item — top-level bindings and ordinary
/// FUNC/SUB bodies — is walked for stray assertion calls. Returns `true` iff a
/// misplacement was reported. Runs on the parsed AST before any lowering, in both
/// build and test mode.
pub(crate) fn validate_expect_placement(ast: &AstProject) -> bool {
    let mut found = false;
    for file in &ast.files {
        for item in &file.items {
            match item {
                Item::Function(function) => {
                    walk_statements(&function.body, &file.path, &mut found);
                    if let Some(trap) = &function.trap {
                        walk_statements(&trap.body, &file.path, &mut found);
                    }
                }
                Item::Binding(binding) => {
                    if let Some(value) = &binding.value {
                        walk_expression(value, binding.line, &file.path, &mut found);
                    }
                }
                // TESTING case bodies are the one legal home for `expect*`.
                Item::Testing(_) => {}
                _ => {}
            }
        }
    }
    found
}

fn walk_statements(statements: &[Statement], path: &str, found: &mut bool) {
    for statement in statements {
        walk_statement(statement, path, found);
    }
}

fn walk_statement(statement: &Statement, path: &str, found: &mut bool) {
    match statement {
        Statement::Let { value, line, .. } => {
            if let Some(value) = value {
                walk_expression(value, *line, path, found);
            }
        }
        Statement::Return { value, line } | Statement::Recover { value, line } => {
            if let Some(value) = value {
                walk_expression(value, *line, path, found);
            }
        }
        Statement::Exit { code, line, .. } => {
            if let Some(code) = code {
                walk_expression(code, *line, path, found);
            }
        }
        Statement::Fail { error, line } => walk_expression(error, *line, path, found),
        Statement::Assign { value, line, .. } | Statement::StateAssign { value, line, .. } => {
            walk_expression(value, *line, path, found);
        }
        Statement::Expression { expression, line } => {
            walk_expression(expression, *line, path, found);
        }
        Statement::If {
            condition,
            then_body,
            else_body,
            line,
        } => {
            walk_expression(condition, *line, path, found);
            walk_statements(then_body, path, found);
            walk_statements(else_body, path, found);
        }
        Statement::Match {
            expression,
            cases,
            line,
        } => {
            walk_expression(expression, *line, path, found);
            for case in cases {
                if let Some(guard) = &case.guard {
                    walk_expression(guard, case.line, path, found);
                }
                walk_statements(&case.body, path, found);
            }
        }
        Statement::For {
            start,
            end,
            step,
            body,
            line,
            ..
        } => {
            walk_expression(start, *line, path, found);
            walk_expression(end, *line, path, found);
            if let Some(step) = step {
                walk_expression(step, *line, path, found);
            }
            walk_statements(body, path, found);
        }
        Statement::ForEach {
            iterable,
            body,
            line,
            ..
        } => {
            walk_expression(iterable, *line, path, found);
            walk_statements(body, path, found);
        }
        Statement::While {
            condition,
            body,
            line,
            ..
        } => {
            walk_expression(condition, *line, path, found);
            walk_statements(body, path, found);
        }
        Statement::DoUntil {
            body,
            condition,
            line,
        } => {
            walk_statements(body, path, found);
            walk_expression(condition, *line, path, found);
        }
        Statement::Continue { .. } | Statement::Propagate { .. } => {}
    }
}

fn walk_expression(expression: &Expression, line: usize, path: &str, found: &mut bool) {
    match expression {
        Expression::Call {
            callee,
            arguments,
            line: call_line,
            ..
        } => {
            if is_expect_call(callee) {
                *found = true;
                crate::rules::show_diagnostic(
                    "TESTING_EXPECT_OUTSIDE_TCASE",
                    &format!(
                        "`{callee}` is a test assertion and is valid only inside a TCASE body."
                    ),
                    std::path::Path::new(path),
                    *call_line,
                    1,
                    1,
                );
            }
            for argument in arguments {
                walk_expression(call_arg_value(argument), *call_line, path, found);
            }
        }
        Expression::Binary { left, right, .. } => {
            walk_expression(left, line, path, found);
            walk_expression(right, line, path, found);
        }
        Expression::Unary { operand, .. } => walk_expression(operand, line, path, found),
        Expression::Lambda { body, .. } => walk_expression(body, line, path, found),
        Expression::Constructor { arguments, .. } => {
            for argument in arguments {
                walk_expression(constructor_arg_value(argument), line, path, found);
            }
        }
        Expression::WithUpdate { target, updates } => {
            walk_expression(target, line, path, found);
            for update in updates {
                walk_expression(&update.value, update.line, path, found);
            }
        }
        Expression::ListLiteral(values) => {
            for value in values {
                walk_expression(value, line, path, found);
            }
        }
        Expression::MapLiteral { entries, .. } => {
            for (key, value) in entries {
                walk_expression(key, line, path, found);
                walk_expression(value, line, path, found);
            }
        }
        Expression::MemberAccess { target, .. } => walk_expression(target, line, path, found),
        Expression::Trapped {
            expression,
            handler,
            ..
        } => {
            walk_expression(expression, line, path, found);
            walk_statements(handler, path, found);
        }
        Expression::String(_)
        | Expression::Number(_)
        | Expression::Scalar(_)
        | Expression::Boolean(_)
        | Expression::Identifier(_) => {}
    }
}
