use super::*;

pub(super) fn collect_source(
    ast: &ast::AstProject,
) -> (Vec<FlowFunction>, Vec<PermissionEntry>, Vec<ResourceEntry>) {
    let fallible = fallible_functions(ast);

    let mut flow = Vec::new();
    let mut permissions = Vec::new();
    let mut resources = Vec::new();

    for file in &ast.files {
        for item in &file.items {
            let Item::Function(function) = item else {
                continue;
            };

            let has_trap = function.trap.is_some();
            let propagation = if has_trap { "trap" } else { "return" };

            let mut calls = Vec::new();
            {
                let mut visit = |callee: &str, line: usize| {
                    if let Some(capability) = builtin_capability(callee) {
                        permissions.push(PermissionEntry {
                            capability: capability.to_string(),
                            package: package_of(callee).to_string(),
                            function: callee.to_string(),
                            path: file.path.clone(),
                            line,
                            kind: "standard".to_string(),
                        });
                    }
                    if is_fallible_call(callee, &fallible) {
                        calls.push(CallSite {
                            callee: callee.to_string(),
                            line,
                            propagation: propagation.to_string(),
                            capability: builtin_capability(callee).map(str::to_string),
                        });
                    }
                };
                walk_statements(&function.body, &mut visit);
                if let Some(trap) = &function.trap {
                    walk_statements(&trap.body, &mut visit);
                }
            }

            collect_resources(&function.name, &file.path, &function.body, &mut resources);
            if let Some(trap) = &function.trap {
                collect_resources(&function.name, &file.path, &trap.body, &mut resources);
            }

            calls.sort_by(|a, b| a.line.cmp(&b.line).then(a.callee.cmp(&b.callee)));

            let trap_info = function.trap.as_ref().map(|trap| TrapInfo {
                name: trap.name.clone(),
                line: trap.line,
                classification: classify_trap(&trap.body),
            });

            flow.push(FlowFunction {
                function: function.name.clone(),
                path: file.path.clone(),
                line: function.line,
                fallible: fallible.contains(&function.name),
                trap: trap_info,
                calls,
            });
        }
    }

    flow.sort_by(|a, b| {
        a.path
            .cmp(&b.path)
            .then(a.line.cmp(&b.line))
            .then(a.function.cmp(&b.function))
    });
    permissions.sort_by(|a, b| {
        a.capability
            .cmp(&b.capability)
            .then(a.path.cmp(&b.path))
            .then(a.line.cmp(&b.line))
            .then(a.function.cmp(&b.function))
    });
    permissions.dedup_by(|a, b| {
        a.capability == b.capability
            && a.path == b.path
            && a.line == b.line
            && a.function == b.function
    });
    resources.sort_by(|a, b| {
        a.path
            .cmp(&b.path)
            .then(a.line.cmp(&b.line))
            .then(a.name.cmp(&b.name))
    });

    (flow, permissions, resources)
}

fn collect_resources(function: &str, path: &str, body: &[Statement], out: &mut Vec<ResourceEntry>) {
    for statement in body {
        match statement {
            Statement::Let {
                name,
                value: Some(Expression::Call { callee, .. }),
                line,
                ..
            } => {
                if let Some((resource_type, close_op)) = resource_producer(callee) {
                    out.push(ResourceEntry {
                        function: function.to_string(),
                        name: name.clone(),
                        resource_type: resource_type.to_string(),
                        close_op: close_op.to_string(),
                        path: path.to_string(),
                        line: *line,
                        native: false,
                        close_may_fail: true,
                    });
                }
            }
            Statement::If {
                then_body,
                else_body,
                ..
            } => {
                collect_resources(function, path, then_body, out);
                collect_resources(function, path, else_body, out);
            }
            Statement::Match { cases, .. } => {
                for case in cases {
                    collect_resources(function, path, &case.body, out);
                }
            }
            Statement::For { body, .. }
            | Statement::ForEach { body, .. }
            | Statement::While { body, .. }
            | Statement::DoUntil { body, .. } => {
                collect_resources(function, path, body, out);
            }
            _ => {}
        }
    }
}

/// Visits every statement and reports each call expression's callee and line.
fn walk_statements(body: &[Statement], visit: &mut impl FnMut(&str, usize)) {
    for statement in body {
        match statement {
            Statement::Let { value, line, .. } => {
                if let Some(expr) = value {
                    walk_expression(expr, *line, visit);
                }
            }
            Statement::Return { value, line } => {
                if let Some(expr) = value {
                    walk_expression(expr, *line, visit);
                }
            }
            Statement::Exit { code, line, .. } => {
                if let Some(expr) = code {
                    walk_expression(expr, *line, visit);
                }
            }
            Statement::Continue { .. } => {}
            Statement::Fail { error, line } => walk_expression(error, *line, visit),
            Statement::Propagate { .. } => {}
            Statement::Recover { value, line } => {
                if let Some(expr) = value {
                    walk_expression(expr, *line, visit);
                }
            }
            Statement::Assign { value, line, .. } => walk_expression(value, *line, visit),
            Statement::StateAssign { value, line, .. } => walk_expression(value, *line, visit),
            Statement::Expression { expression, line } => walk_expression(expression, *line, visit),
            Statement::If {
                condition,
                then_body,
                else_body,
                line,
            } => {
                walk_expression(condition, *line, visit);
                walk_statements(then_body, visit);
                walk_statements(else_body, visit);
            }
            Statement::Match {
                expression,
                cases,
                line,
            } => {
                walk_expression(expression, *line, visit);
                for case in cases {
                    if let Some(guard) = &case.guard {
                        walk_expression(guard, case.line, visit);
                    }
                    walk_statements(&case.body, visit);
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
                walk_expression(start, *line, visit);
                walk_expression(end, *line, visit);
                if let Some(step) = step {
                    walk_expression(step, *line, visit);
                }
                walk_statements(body, visit);
            }
            Statement::ForEach {
                iterable,
                body,
                line,
                ..
            } => {
                walk_expression(iterable, *line, visit);
                walk_statements(body, visit);
            }
            Statement::While {
                kind: _,
                condition,
                body,
                line,
            } => {
                walk_expression(condition, *line, visit);
                walk_statements(body, visit);
            }
            Statement::DoUntil {
                body,
                condition,
                line,
            } => {
                walk_statements(body, visit);
                walk_expression(condition, *line, visit);
            }
        }
    }
}

fn walk_expression(expression: &Expression, line: usize, visit: &mut impl FnMut(&str, usize)) {
    match expression {
        Expression::Call {
            callee, arguments, ..
        } => {
            for argument in arguments {
                match argument {
                    CallArg::Positional(value) => walk_expression(value, line, visit),
                    CallArg::Named { value, line, .. } => walk_expression(value, *line, visit),
                }
            }
            visit(callee, line);
        }
        Expression::Binary { left, right, .. } => {
            walk_expression(left, line, visit);
            walk_expression(right, line, visit);
        }
        Expression::Unary { operand, .. } => walk_expression(operand, line, visit),
        Expression::Lambda { body, .. } => walk_expression(body, line, visit),
        Expression::Constructor { arguments, .. } => {
            for argument in arguments {
                match argument {
                    ConstructorArg::Positional(value) => walk_expression(value, line, visit),
                    ConstructorArg::Named { value, line, .. } => {
                        walk_expression(value, *line, visit)
                    }
                }
            }
        }
        Expression::WithUpdate { target, updates } => {
            walk_expression(target, line, visit);
            for update in updates {
                walk_expression(&update.value, update.line, visit);
            }
        }
        Expression::ListLiteral(items) => {
            for item in items {
                walk_expression(item, line, visit);
            }
        }
        Expression::MapLiteral { entries, .. } => {
            for (key, value) in entries {
                walk_expression(key, line, visit);
                walk_expression(value, line, visit);
            }
        }
        Expression::MemberAccess { target, .. } => walk_expression(target, line, visit),
        Expression::Trapped {
            expression,
            handler,
            line: trap_line,
            ..
        } => {
            walk_expression(expression, *trap_line, visit);
            walk_statements(handler, visit);
        }
        Expression::String(_)
        | Expression::Number(_)
        | Expression::Boolean(_)
        | Expression::Identifier(_) => {}
    }
}

/// Computes the set of user function names whose errors escape to callers.
fn fallible_functions(ast: &ast::AstProject) -> HashSet<String> {
    let mut functions: BTreeMap<&str, &Function> = BTreeMap::new();
    for file in &ast.files {
        for item in &file.items {
            if let Item::Function(function) = item {
                functions.insert(function.name.as_str(), function);
            }
        }
    }

    let mut fallible: HashSet<String> = HashSet::new();
    loop {
        let mut changed = false;
        for (name, function) in &functions {
            if fallible.contains(*name) {
                continue;
            }
            // Errors raised in the body are routed to the trap when one exists,
            // so only what escapes the relevant block makes the function fallible.
            let body = match &function.trap {
                Some(trap) => &trap.body,
                None => &function.body,
            };
            if block_escapes(body, &fallible) {
                fallible.insert((*name).to_string());
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    fallible
}

/// Returns true if the block can let an error escape: a `FAIL`, a `PROPAGATE`,
/// or a call to a fallible builtin or fallible user function.
fn block_escapes(body: &[Statement], fallible: &HashSet<String>) -> bool {
    let mut escapes = false;
    let mut check = |callee: &str, _line: usize| {
        if is_fallible_call(callee, fallible) {
            escapes = true;
        }
    };
    if statements_fail_or_propagate(body) {
        return true;
    }
    walk_statements(body, &mut check);
    escapes
}

fn statements_fail_or_propagate(body: &[Statement]) -> bool {
    body.iter().any(|statement| match statement {
        Statement::Fail { .. } | Statement::Propagate { .. } => true,
        Statement::If {
            then_body,
            else_body,
            ..
        } => statements_fail_or_propagate(then_body) || statements_fail_or_propagate(else_body),
        Statement::Match { cases, .. } => cases
            .iter()
            .any(|case| statements_fail_or_propagate(&case.body)),
        Statement::For { body, .. }
        | Statement::ForEach { body, .. }
        | Statement::While { body, .. }
        | Statement::DoUntil { body, .. } => statements_fail_or_propagate(body),
        _ => false,
    })
}

fn classify_trap(body: &[Statement]) -> String {
    if statements_contain_propagate(body) {
        "propagates".to_string()
    } else if statements_contain_fail(body) {
        "fails".to_string()
    } else if statements_contain_return_value(body) {
        "returns value".to_string()
    } else {
        "recovers".to_string()
    }
}

fn statements_contain_propagate(body: &[Statement]) -> bool {
    body.iter().any(|statement| match statement {
        Statement::Propagate { .. } => true,
        Statement::If {
            then_body,
            else_body,
            ..
        } => statements_contain_propagate(then_body) || statements_contain_propagate(else_body),
        Statement::Match { cases, .. } => cases
            .iter()
            .any(|case| statements_contain_propagate(&case.body)),
        Statement::For { body, .. }
        | Statement::ForEach { body, .. }
        | Statement::While { body, .. }
        | Statement::DoUntil { body, .. } => statements_contain_propagate(body),
        _ => false,
    })
}

fn statements_contain_fail(body: &[Statement]) -> bool {
    body.iter().any(|statement| match statement {
        Statement::Fail { .. } => true,
        Statement::If {
            then_body,
            else_body,
            ..
        } => statements_contain_fail(then_body) || statements_contain_fail(else_body),
        Statement::Match { cases, .. } => {
            cases.iter().any(|case| statements_contain_fail(&case.body))
        }
        Statement::For { body, .. }
        | Statement::ForEach { body, .. }
        | Statement::While { body, .. }
        | Statement::DoUntil { body, .. } => statements_contain_fail(body),
        _ => false,
    })
}

fn statements_contain_return_value(body: &[Statement]) -> bool {
    body.iter().any(|statement| match statement {
        Statement::Return { value, .. } => value.is_some(),
        Statement::If {
            then_body,
            else_body,
            ..
        } => {
            statements_contain_return_value(then_body) || statements_contain_return_value(else_body)
        }
        Statement::Match { cases, .. } => cases
            .iter()
            .any(|case| statements_contain_return_value(&case.body)),
        Statement::For { body, .. }
        | Statement::ForEach { body, .. }
        | Statement::While { body, .. }
        | Statement::DoUntil { body, .. } => statements_contain_return_value(body),
        _ => false,
    })
}

fn package_of(callee: &str) -> &str {
    callee.split('.').next().unwrap_or(callee)
}

fn builtin_capability(callee: &str) -> Option<&'static str> {
    match package_of(callee) {
        "fs" => Some("filesystem"),
        "io" => Some("terminal"),
        "thread" => Some("threads"),
        _ => None,
    }
}

fn is_fallible_call(callee: &str, fallible: &HashSet<String>) -> bool {
    if matches!(package_of(callee), "fs" | "io" | "json" | "net" | "thread") {
        return true;
    }
    fallible.contains(callee)
}

fn resource_producer(callee: &str) -> Option<(&'static str, &'static str)> {
    match callee {
        "fs.open" | "fs.openFile" | "fs.openFileNoFollow" | "fs.createTempFile" => {
            Some(("File", "fs.close"))
        }
        "thread.start" => Some(("Thread", "thread.waitFor")),
        "net.connectTcp" | "net.accept" => Some(("Socket", "net.close")),
        "net.listenTcp" => Some(("Listener", "net.close")),
        _ => None,
    }
}
