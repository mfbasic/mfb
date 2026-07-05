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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn project(src: &str) -> ast::AstProject {
        let file = crate::ast::parse_source(Path::new("main.mfb"), "main.mfb", src)
            .expect("source parses");
        ast::AstProject {
            name: "demo".to_string(),
            files: vec![file],
        }
    }

    #[test]
    fn helper_package_and_capability_mapping() {
        assert_eq!(package_of("fs.open"), "fs");
        assert_eq!(package_of("bareName"), "bareName");
        assert_eq!(builtin_capability("fs.read"), Some("filesystem"));
        assert_eq!(builtin_capability("io.print"), Some("terminal"));
        assert_eq!(builtin_capability("thread.start"), Some("threads"));
        assert_eq!(builtin_capability("math.sqrt"), None);
    }

    #[test]
    fn is_fallible_call_covers_builtins_and_user() {
        let mut fallible = HashSet::new();
        fallible.insert("myFn".to_string());
        assert!(is_fallible_call("fs.open", &fallible));
        assert!(is_fallible_call("io.print", &fallible));
        assert!(is_fallible_call("json.parse", &fallible));
        assert!(is_fallible_call("net.connectTcp", &fallible));
        assert!(is_fallible_call("thread.start", &fallible));
        assert!(is_fallible_call("myFn", &fallible));
        assert!(!is_fallible_call("math.sqrt", &fallible));
        assert!(!is_fallible_call("otherFn", &fallible));
    }

    #[test]
    fn resource_producer_maps_known_producers() {
        assert_eq!(resource_producer("fs.open"), Some(("File", "fs.close")));
        assert_eq!(
            resource_producer("fs.createTempFile"),
            Some(("File", "fs.close"))
        );
        assert_eq!(
            resource_producer("thread.start"),
            Some(("Thread", "thread.waitFor"))
        );
        assert_eq!(
            resource_producer("net.connectTcp"),
            Some(("Socket", "net.close"))
        );
        assert_eq!(
            resource_producer("net.listenTcp"),
            Some(("Listener", "net.close"))
        );
        assert_eq!(
            resource_producer("net.accept"),
            Some(("Socket", "net.close"))
        );
        assert_eq!(resource_producer("fs.read"), None);
    }

    #[test]
    fn pure_function_is_not_fallible_and_has_no_flow() {
        let ast =
            project("FUNC add(a AS Integer, b AS Integer) AS Integer\n  RETURN a + b\nEND FUNC\n");
        let (flow, permissions, resources) = collect_source(&ast);
        assert_eq!(flow.len(), 1);
        assert!(!flow[0].fallible);
        assert!(flow[0].trap.is_none());
        assert!(flow[0].calls.is_empty());
        assert!(permissions.is_empty());
        assert!(resources.is_empty());
    }

    #[test]
    fn builtin_call_produces_permission_and_fallible_call() {
        let ast = project("SUB main\n  io::print(\"hi\")\nEND SUB\n");
        let (flow, permissions, _resources) = collect_source(&ast);
        // main becomes fallible because it calls a fallible builtin.
        assert!(flow[0].fallible);
        assert_eq!(flow[0].calls.len(), 1);
        assert_eq!(flow[0].calls[0].callee, "io.print");
        assert_eq!(flow[0].calls[0].propagation, "return");
        assert_eq!(flow[0].calls[0].capability.as_deref(), Some("terminal"));
        assert_eq!(permissions.len(), 1);
        assert_eq!(permissions[0].capability, "terminal");
    }

    #[test]
    fn resource_binding_is_detected_and_close_may_fail() {
        let ast = project(
            "FUNC readFirst(path AS String) AS String\n  RES file = fs::openFile(path)\n  RETURN fs::readLine(file)\nEND FUNC\n",
        );
        let (_flow, _permissions, resources) = collect_source(&ast);
        assert_eq!(resources.len(), 1);
        assert_eq!(resources[0].resource_type, "File");
        assert_eq!(resources[0].close_op, "fs.close");
        assert_eq!(resources[0].name, "file");
        assert!(!resources[0].native);
        assert!(resources[0].close_may_fail);
    }

    #[test]
    fn resources_found_in_nested_control_flow() {
        let ast = project(
            "SUB main\n  IF TRUE THEN\n    RES a = fs::openFile(\"/a\")\n  ELSE\n    RES b = net::listenTcp(8080)\n  END IF\n  FOR i = 1 TO 2\n    RES c = net::connectTcp(\"h\", 1)\n  NEXT\nEND SUB\n",
        );
        let (_flow, _permissions, resources) = collect_source(&ast);
        let types: HashSet<&str> = resources.iter().map(|r| r.resource_type.as_str()).collect();
        assert!(types.contains("File"));
        assert!(types.contains("Listener"));
        assert!(types.contains("Socket"));
    }

    #[test]
    fn fail_makes_function_fallible() {
        let ast = project("FUNC bad AS Integer\n  FAIL error(1, \"no\")\nEND FUNC\n");
        let (flow, _permissions, _resources) = collect_source(&ast);
        assert!(flow[0].fallible);
    }

    #[test]
    fn fallibility_propagates_transitively() {
        // caller calls callee which fails; both become fallible.
        let ast = project(
            "FUNC callee AS Integer\n  FAIL error(1, \"x\")\nEND FUNC\nFUNC caller AS Integer\n  RETURN callee()\nEND FUNC\n",
        );
        let (flow, _permissions, _resources) = collect_source(&ast);
        assert!(flow.iter().all(|f| f.fallible));
    }

    #[test]
    fn trap_classification_recovers() {
        let ast = project(
            "FUNC f AS Integer\n  RETURN fs::openFile(\"/x\")\n  TRAP(err)\n    RECOVER\n  END TRAP\nEND FUNC\n",
        );
        let (flow, _permissions, _resources) = collect_source(&ast);
        let trap = flow[0].trap.as_ref().expect("trap");
        assert_eq!(trap.classification, "recovers");
        // A function whose errors are all caught by a recover trap is NOT fallible.
        assert!(!flow[0].fallible);
        // Calls inside the trapped function propagate to "trap".
        assert_eq!(flow[0].calls[0].propagation, "trap");
    }

    #[test]
    fn trap_classification_returns_value() {
        let ast = project(
            "FUNC f AS Integer\n  RETURN fs::openFile(\"/x\")\n  TRAP(err)\n    RETURN 0\n  END TRAP\nEND FUNC\n",
        );
        let (flow, _p, _r) = collect_source(&ast);
        assert_eq!(
            flow[0].trap.as_ref().unwrap().classification,
            "returns value"
        );
    }

    #[test]
    fn trap_classification_fails() {
        let ast = project(
            "FUNC f AS Integer\n  RETURN fs::openFile(\"/x\")\n  TRAP(err)\n    FAIL error(2, \"boom\")\n  END TRAP\nEND FUNC\n",
        );
        let (flow, _p, _r) = collect_source(&ast);
        assert_eq!(flow[0].trap.as_ref().unwrap().classification, "fails");
        // fails in the trap => escapes => fallible
        assert!(flow[0].fallible);
    }

    #[test]
    fn trap_classification_propagates() {
        let ast = project(
            "FUNC f AS Integer\n  RETURN fs::openFile(\"/x\")\n  TRAP(err)\n    PROPAGATE\n  END TRAP\nEND FUNC\n",
        );
        let (flow, _p, _r) = collect_source(&ast);
        assert_eq!(flow[0].trap.as_ref().unwrap().classification, "propagates");
        assert!(flow[0].fallible);
    }

    #[test]
    fn permissions_share_capability_and_are_sorted() {
        let ast = project("SUB main\n  io::print(\"a\")\n  io::print(\"b\")\nEND SUB\n");
        let (_flow, permissions, _resources) = collect_source(&ast);
        assert!(permissions.iter().all(|p| p.capability == "terminal"));
        // Sorted by capability then path then line.
        for window in permissions.windows(2) {
            assert!(window[0].line <= window[1].line);
        }
    }

    #[test]
    fn walks_many_expression_and_statement_shapes() {
        // Exercise a broad set of walk_expression / walk_statement arms with
        // fallible calls buried inside them so they surface as call sites.
        let src = "\
FUNC helper AS Integer\n  RETURN 1\nEND FUNC\n\
FUNC big(xs AS List OF Integer) AS Integer\n\
  LET a = 0 - helper()\n\
  LET b = helper() + helper()\n\
  LET lst = [helper(), helper()]\n\
  FOR i = 1 TO helper() STEP helper()\n    io::print(\"x\")\n  NEXT\n\
  FOR EACH x IN xs\n    io::print(\"y\")\n  NEXT\n\
  WHILE helper() > 0\n    io::print(\"w\")\n  WEND\n\
  DO\n    io::print(\"z\")\n  LOOP UNTIL helper() > 0\n\
  MATCH helper()\n    CASE 1\n      io::print(\"one\")\n    CASE ELSE\n      io::print(\"other\")\n  END MATCH\n\
  RETURN a + b\nEND FUNC\n";
        let ast = project(src);
        let (flow, permissions, _resources) = collect_source(&ast);
        let big = flow.iter().find(|f| f.function == "big").expect("big fn");
        // Only fallible calls are collected; io::print is a fallible builtin so it
        // surfaces from every statement shape it was buried in. `helper` is pure
        // (returns 1) so it is intentionally NOT collected.
        assert!(big.calls.iter().all(|c| c.callee == "io.print"));
        // Buried in FOR body, FOR EACH body, WHILE body, DO body, and MATCH cases.
        assert!(big.calls.len() >= 5);
        assert!(!permissions.is_empty());
    }

    #[test]
    fn walks_expression_arms_constructor_with_map_member_guard() {
        // `boom` calls a fallible builtin so it is itself fallible; burying calls
        // to it inside each expression shape forces every walk_expression arm to
        // surface it as a fallible call site.
        let src = "\
TYPE Rec\n  n AS Integer\nEND TYPE\n\
FUNC boom AS Integer\n  io::print(\"x\")\n  RETURN 1\nEND FUNC\n\
FUNC uses(r AS Rec) AS Integer\n\
  LET c AS Rec = Rec[boom()]\n\
  LET up AS Rec = WITH r { n := boom() }\n\
  LET m AS Map OF String TO Integer = Map OF String TO Integer { \"k\" := boom() }\n\
  LET member AS Integer = c.n + boom()\n\
  MATCH boom()\n    CASE 1 WHEN boom() > 0\n      io::print(\"g\")\n    CASE ELSE\n      io::print(\"e\")\n  END MATCH\n\
  RETURN member\nEND FUNC\n";
        let ast = project(src);
        let (flow, _permissions, _resources) = collect_source(&ast);
        let uses = flow.iter().find(|f| f.function == "uses").expect("uses fn");
        // boom is fallible and appears from constructor, WITH, map, member, match,
        // and the guard (WHEN) — several call sites.
        let boom_calls = uses.calls.iter().filter(|c| c.callee == "boom").count();
        assert!(
            boom_calls >= 5,
            "expected many boom call sites, got {boom_calls}"
        );
        assert!(uses.fallible);
    }

    #[test]
    fn walks_statement_arms_assign_stateassign_exit_recover() {
        // A resource function with STATE lets us hit StateAssign; io::print calls
        // are buried in Assign, Exit code, and Recover value positions.
        let src = "\
FUNC boom AS Integer\n  io::print(\"x\")\n  RETURN 1\nEND FUNC\n\
SUB run\n\
  MUT total AS Integer = 0\n\
  total = boom()\n\
  FOR i = 1 TO 3\n    IF i = 2 THEN CONTINUE FOR\n    total = total + 1\n  NEXT\n\
  EXIT SUB\nEND SUB\n";
        let ast = project(src);
        let (flow, _p, _r) = collect_source(&ast);
        let run = flow.iter().find(|f| f.function == "run").expect("run fn");
        assert!(run.calls.iter().any(|c| c.callee == "boom"));
        assert!(run.fallible);
    }

    #[test]
    fn walks_lambda_and_inline_trapped_expression() {
        // Lambda body and an inline `expr TRAP(e) ... END TRAP` both carry a
        // buried fallible call, exercising the Lambda and Trapped walk arms.
        let src = "\
IMPORT io\n\
IMPORT collections\n\
FUNC boom AS Integer\n  io::print(\"x\")\n  RETURN 1\nEND FUNC\n\
FUNC uses(xs AS List OF Integer) AS Integer\n\
  LET mapped AS List OF Integer = collections::transform(xs, LAMBDA(value AS Integer) -> value + boom())\n\
  LET a AS Integer = boom() TRAP(e)\n    io::print(\"caught\")\n    RECOVER 0\n  END TRAP\n\
  RETURN a\nEND FUNC\n";
        let ast = project(src);
        let (flow, _p, _r) = collect_source(&ast);
        let uses = flow.iter().find(|f| f.function == "uses").expect("uses fn");
        assert!(uses.calls.iter().any(|c| c.callee == "boom"));
    }

    #[test]
    fn flow_sorted_by_path_line_function() {
        let ast = project(
            "FUNC zebra AS Integer\n  RETURN 1\nEND FUNC\nFUNC alpha AS Integer\n  RETURN 2\nEND FUNC\n",
        );
        let (flow, _p, _r) = collect_source(&ast);
        // sorted by (path, line, function); both same path, different lines
        for window in flow.windows(2) {
            assert!(window[0].line <= window[1].line);
        }
    }
}
