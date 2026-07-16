use super::*;

pub(super) fn collect_source(
    ast: &ast::AstProject,
) -> (Vec<FlowFunction>, Vec<PermissionEntry>, Vec<ResourceEntry>) {
    let fallible = fallible_functions(ast);
    let aliases = link_aliases(ast);

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
                    let capability = builtin_capability(callee, &aliases);
                    if let Some(capability) = capability {
                        permissions.push(PermissionEntry {
                            capability: capability.to_string(),
                            package: package_of(callee).to_string(),
                            function: callee.to_string(),
                            path: file.path.clone(),
                            line,
                            kind: if capability == "native" {
                                "native".to_string()
                            } else {
                                "standard".to_string()
                            },
                        });
                    }
                    if is_fallible_call(callee, &fallible.names) {
                        calls.push(CallSite {
                            callee: callee.to_string(),
                            line,
                            propagation: propagation.to_string(),
                            capability: capability.map(str::to_string),
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
                fallible: fallible
                    .declarations
                    .contains(&(file.path.clone(), function.line)),
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

/// The callee of a resource-acquiring value: a bare call, or the call wrapped in an
/// inline `TRAP` (`LET h = fs::open(p) TRAP(e) … END TRAP`). Matching only the bare
/// `Expression::Call` under-reported the Resources section (bug-211).
fn acquisition_callee(value: &Expression) -> Option<&str> {
    match value {
        Expression::Call { callee, .. } => Some(callee),
        Expression::Trapped { expression, .. } => acquisition_callee(expression),
        _ => None,
    }
}

fn collect_resources(function: &str, path: &str, body: &[Statement], out: &mut Vec<ResourceEntry>) {
    for statement in body {
        match statement {
            Statement::Let {
                name,
                value: Some(value),
                line,
                ..
            } => {
                if let Some((resource_type, close_op)) =
                    acquisition_callee(value).and_then(resource_producer)
                {
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
            // A resource acquired by reassignment (`h = fs::open(p)`) is an
            // acquisition too — previously missed entirely (bug-211).
            Statement::Assign { name, value, line } => {
                if let Some((resource_type, close_op)) =
                    acquisition_callee(value).and_then(resource_producer)
                {
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
        | Expression::Scalar(_)
        | Expression::Boolean(_)
        | Expression::Identifier(_) => {}
    }
}

/// Which user functions let errors escape to their callers.
///
/// Overloads share a name, so a verdict must be kept per *declaration*. A call
/// site carries no types before monomorphization, though, so it cannot be
/// resolved to one overload: `names` therefore unions the verdicts of every
/// overload of a name, and a call to any of them counts as fallible. That
/// over-approximates a caller of a pure overload whose sibling is fallible, and
/// never under-reports.
struct Fallibility {
    /// Names with at least one fallible overload; the call-site test.
    names: HashSet<String>,
    /// Declarations that are themselves fallible, keyed by `(path, line)`.
    declarations: HashSet<(String, usize)>,
}

/// The block whose escapes decide a function's fallibility: errors raised in the
/// body are routed to the trap when one exists, so only what escapes the trap
/// handler reaches the caller.
fn relevant_block(function: &Function) -> &[Statement] {
    match &function.trap {
        Some(trap) => &trap.body,
        None => &function.body,
    }
}

/// Computes which user functions let errors escape to callers, per declaration.
fn fallible_functions(ast: &ast::AstProject) -> Fallibility {
    // Every declaration, in source order — a name-keyed map would drop all but
    // one overload, analyzing a single body and broadcasting its verdict.
    let mut functions: Vec<(&str, &Function)> = Vec::new();
    for file in &ast.files {
        for item in &file.items {
            if let Item::Function(function) = item {
                functions.push((file.path.as_str(), function));
            }
        }
    }

    // Seed with the `LINK` functions carrying a `SUCCESS_ON` gate: a call to one
    // raises a trappable native error, so a user function whose only error source
    // is such a call is fallible. The fixpoint below then propagates that to its
    // callers (bug-211).
    let mut names: HashSet<String> = link_fallible_calls(ast);
    loop {
        let mut changed = false;
        for (_, function) in &functions {
            if names.contains(&function.name) {
                continue;
            }
            if block_escapes(relevant_block(function), &names) {
                names.insert(function.name.clone());
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    // `names` has converged, so a final pass decides each declaration on its own
    // body — including the overloads the loop above skipped once their name was
    // already marked by a sibling.
    let declarations = functions
        .iter()
        .filter(|(_, function)| block_escapes(relevant_block(function), &names))
        .map(|(path, function)| ((*path).to_string(), function.line))
        .collect();

    Fallibility {
        names,
        declarations,
    }
}

/// Returns true if the block can let an error escape: a `FAIL`, a `PROPAGATE`,
/// or a call to a fallible builtin or fallible user function. `fallible` is the
/// name-union set, so a call resolves conservatively across overloads.
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

/// The host capability a call discloses, or `None` for a pure call.
///
/// Packages whose every operation touches the same host surface map by package;
/// `os`, `math`, and `datetime` mix pure and host-touching builtins, so those map
/// by the specific builtin. A call through a `LINK` alias discloses `native`.
fn builtin_capability(callee: &str, link_aliases: &HashSet<String>) -> Option<&'static str> {
    let package = package_of(callee);
    if link_aliases.contains(package) {
        return Some("native");
    }
    match package {
        "fs" => Some("filesystem"),
        "io" => Some("terminal"),
        "thread" => Some("threads"),
        // Networking also flows through `tls::*` and `http::*` (audit runs on the
        // pre-monomorph AST, before the http→net source rewrite, so these must be
        // listed directly or a TLS/HTTP-only program discloses no network use —
        // bug-96).
        "net" | "tls" | "http" => Some("network"),
        // Secure-randomness surface: the entropy-drawing crypto builtins (the
        // rest of `crypto` is pure computation over caller-supplied bytes).
        "crypto" => match callee {
            "crypto.randomBytes"
            | "crypto.randomInt"
            | "crypto.uuid4"
            | "crypto.generateEd25519"
            | "crypto.generateP256"
            | "crypto.generateP384"
            | "crypto.generateP521" => Some("randomness"),
            _ => None,
        },
        "os" => match callee {
            "os.getEnv" | "os.getEnvOr" | "os.hasEnv" | "os.setEnv" | "os.unsetEnv"
            | "os.environ" => Some("environment"),
            "os.args" | "os.pid" | "os.name" | "os.arch" | "os.hostName" | "os.userName"
            | "os.cpuCount" | "os.executablePath" => Some("process"),
            _ => None,
        },
        "math" => match callee {
            "math.rand" | "math.seed" => Some("randomness"),
            _ => None,
        },
        // Only the builtins that read the host clock or timezone; the rest of
        // `datetime` is arithmetic over values the caller supplies.
        "datetime" => match callee {
            "datetime.now"
            | "datetime.nowNanos"
            | "datetime.monotonic"
            | "datetime.monotonicNanos"
            | "datetime.localOffset"
            | "datetime.local"
            | "datetime.toLocal" => Some("clock"),
            _ => None,
        },
        _ => None,
    }
}

/// The `LINK` aliases a project declares. A call qualified by one of these is a
/// native call, whatever the alias happens to be named.
fn link_aliases(ast: &ast::AstProject) -> HashSet<String> {
    let mut aliases = HashSet::new();
    for file in &ast.files {
        for item in &file.items {
            if let Item::Link(link) = item {
                aliases.insert(link.alias.clone());
            }
        }
    }
    aliases
}

/// `<alias>.<func>` for every `LINK` function carrying a `SUCCESS_ON` gate. Such a
/// call raises a trappable error when the gate fails, so a user function whose only
/// error source is one of these is fallible — previously it was reported pure and
/// its call omitted from the Control-flow section (bug-211).
fn link_fallible_calls(ast: &ast::AstProject) -> HashSet<String> {
    let mut names = HashSet::new();
    for file in &ast.files {
        for item in &file.items {
            if let Item::Link(link) = item {
                for function in &link.functions {
                    if function.success_on.is_some() {
                        names.insert(format!("{}.{}", link.alias, function.name));
                    }
                }
            }
        }
    }
    names
}

fn is_fallible_call(callee: &str, fallible: &HashSet<String>) -> bool {
    // Whole-package fallible surfaces — every call raises a trappable host error.
    // `tls`/`http` join the original set: they are network I/O like `net`
    // (bug-96).
    if matches!(
        package_of(callee),
        "fs" | "io" | "json" | "net" | "thread" | "tls" | "http"
    ) {
        return true;
    }
    if is_fallible_builtin(callee) {
        return true;
    }
    fallible.contains(callee)
}

/// The specific builtins in mixed pure/fallible packages (`crypto`, `datetime`)
/// that raise a trappable domain error. The rest of those packages are total
/// computation, so a coarse package match would over-report (bug-96).
fn is_fallible_builtin(callee: &str) -> bool {
    matches!(
        callee,
        // crypto — AEAD (auth-tag verification), signatures, key generation, KDFs,
        // MACs, and entropy draws can each fail; the SHA hashes and
        // constant-time compare are total.
        "crypto.aes256GcmSeal"
            | "crypto.aes256GcmOpen"
            | "crypto.chacha20Poly1305Seal"
            | "crypto.chacha20Poly1305Open"
            | "crypto.ed25519Sign"
            | "crypto.ed25519Verify"
            | "crypto.p256Sign"
            | "crypto.p256Verify"
            | "crypto.p384Sign"
            | "crypto.p384Verify"
            | "crypto.p521Sign"
            | "crypto.p521Verify"
            | "crypto.generateEd25519"
            | "crypto.generateP256"
            | "crypto.generateP384"
            | "crypto.generateP521"
            | "crypto.hkdfSha256"
            | "crypto.hkdfSha512"
            | "crypto.pbkdf2Sha256"
            | "crypto.pbkdf2Sha512"
            | "crypto.hmacSha256"
            | "crypto.hmacSha512"
            | "crypto.randomBytes"
            | "crypto.randomInt"
            | "crypto.uuid4"
            // datetime — the parsers and formatters and the range-checked
            // constructors raise; date/duration arithmetic is total. Derived from
            // the `FAIL` sites in datetime_package.mfb.
            | "datetime.date"
            | "datetime.time"
            | "datetime.fixedOffset"
            | "datetime.format"
            | "datetime.parse"
            | "datetime.parseIso"
    )
}

fn resource_producer(callee: &str) -> Option<(&'static str, &'static str)> {
    match callee {
        "fs.open" | "fs.openFile" | "fs.openFileNoFollow" | "fs.createTempFile" => {
            Some(("File", "fs.close"))
        }
        "thread.start" => Some(("Thread", "thread.waitFor")),
        "net.connectTcp" | "net.accept" => Some(("Socket", "net.close")),
        "net.listenTcp" => Some(("Listener", "net.close")),
        // bug-96: the tls/http/udp resource producers were missing, so those
        // handles never appeared in the Resources section or the
        // close-may-fail findings.
        "net.bindUdp" => Some(("UdpSocket", "net.close")),
        "tls.connect" | "tls.accept" => Some(("TlsSocket", "tls.close")),
        "tls.listen" | "http.serverSSL" => Some(("TlsListener", "tls.close")),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parse standalone MFBASIC source into a single-file project.
    fn project(source: &str) -> ast::AstProject {
        let path = std::path::Path::new("main.mfb");
        let file = crate::ast::parse_source(path, "main.mfb", source).expect("parse");
        ast::AstProject {
            name: "app".to_string(),
            files: vec![file],
        }
    }

    #[test]
    fn callee_qualified_name_uses_dot_separator() {
        // Sanity: the collector matches `fs.open`, and `fs::open` parses to it.
        let ast = project("FUNC f()\n  LET h = fs::open(\"p\")\nEND FUNC\n");
        let (_, _, resources) = collect_source(&ast);
        assert_eq!(resources.len(), 1);
        assert_eq!(resources[0].resource_type, "File");
        assert_eq!(resources[0].close_op, "fs.close");
    }

    #[test]
    fn overloads_get_their_own_fallibility_verdict() {
        // A name-keyed map analyzed one `parse` body and broadcast its verdict to
        // the other. Each declaration must be judged on its own body, while a
        // call site — which has no types yet — resolves across the overload set.
        let source = concat!(
            "FUNC parse(n AS Integer) AS Integer\n",
            "  RETURN n\n",
            "END FUNC\n",
            "FUNC parse(s AS String) AS Integer\n",
            "  RETURN len(fs::readText(s))\n",
            "END FUNC\n",
            "SUB main()\n",
            "  LET x = parse(\"f.txt\")\n",
            "END SUB\n",
        );
        let (flow, _, _) = collect_source(&project(source));
        let at = |line: usize| {
            flow.iter()
                .find(|entry| entry.line == line)
                .unwrap_or_else(|| panic!("no function declared at line {line}"))
        };
        assert!(!at(1).fallible, "the Integer overload is pure");
        assert!(
            at(4).fallible,
            "the String overload calls a fallible builtin"
        );
        assert!(at(7).fallible, "main calls the fallible overload");
    }

    #[test]
    fn every_host_capability_is_disclosed() {
        // net/os/math/datetime and native LINK calls each disclose a capability;
        // pure builtins in the same packages disclose nothing.
        let source = concat!(
            "LINK \"sqlite3\" AS sql\n",
            "  FUNC open(path AS String) AS Nothing\n",
            "    SYMBOL \"sqlite3_open\"\n",
            "    ABI (path CString) AS status CInt32\n",
            "    SUCCESS_ON status = 0\n",
            "  END FUNC\n",
            "END LINK\n",
            "FUNC f()\n",
            "  net::close(s)\n",
            "  os::getEnv(\"HOME\")\n",
            "  os::pid()\n",
            "  math::rand(1, 6)\n",
            "  math::floor(1.5)\n",
            "  datetime::now()\n",
            "  datetime::toIso(x)\n",
            "  sql::open(\":memory:\")\n",
            "END FUNC\n",
        );
        let (_, permissions, _) = collect_source(&project(source));
        let disclosed = permissions
            .iter()
            .map(|permission| (permission.capability.as_str(), permission.function.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(
            disclosed,
            vec![
                ("clock", "datetime.now"),
                ("environment", "os.getEnv"),
                ("native", "sql.open"),
                ("network", "net.close"),
                ("process", "os.pid"),
                ("randomness", "math.rand"),
            ]
        );
        // Native calls are tagged as such; builtin calls stay "standard".
        let native = permissions
            .iter()
            .find(|permission| permission.capability == "native")
            .expect("native permission");
        assert_eq!(native.kind, "native");
        assert_eq!(native.package, "sql");
    }

    #[test]
    fn permissions_are_collected_and_deduplicated() {
        let source = concat!(
            "FUNC f()\n",
            "  io::print(\"a\")\n",
            "  io::print(\"b\")\n",
            "  LET h = fs::open(\"p\")\n",
            "  LET t = thread::start(worker)\n",
            "END FUNC\n",
        );
        let (_, permissions, _) = collect_source(&project(source));
        let caps: Vec<&str> = permissions.iter().map(|p| p.capability.as_str()).collect();
        assert!(caps.contains(&"terminal"));
        assert!(caps.contains(&"filesystem"));
        assert!(caps.contains(&"threads"));
    }

    #[test]
    fn resources_found_across_nested_control_flow() {
        let source = concat!(
            "FUNC f(n AS Integer)\n",
            "  IF n > 0 THEN\n",
            "    LET a = fs::open(\"a\")\n",
            "  ELSE\n",
            "    LET b = fs::openFile(\"b\")\n",
            "  END IF\n",
            "  FOR i = 1 TO n\n",
            "    LET c = net::connectTcp(addr)\n",
            "  NEXT\n",
            "  FOR EACH x IN items\n",
            "    LET d = net::listenTcp(addr)\n",
            "  NEXT\n",
            "  WHILE n > 0\n",
            "    LET e = thread::start(worker)\n",
            "  WEND\n",
            "  DO\n",
            "    LET g = net::accept(listener)\n",
            "  LOOP UNTIL n < 0\n",
            "END FUNC\n",
        );
        let (_, _, resources) = collect_source(&project(source));
        let types: Vec<&str> = resources.iter().map(|r| r.resource_type.as_str()).collect();
        assert!(types.contains(&"File"));
        assert!(types.contains(&"Socket"));
        assert!(types.contains(&"Listener"));
        assert!(types.contains(&"Thread"));
    }

    #[test]
    fn resources_found_inside_match_case() {
        let source = concat!(
            "TYPE Circle\n  radius AS Integer\nEND TYPE\n",
            "UNION Shape\n  Circle\nEND UNION\n",
            "FUNC f(s AS Shape)\n",
            "  MATCH s\n",
            "    CASE Circle(c)\n",
            "      LET a = fs::createTempFile(\"t\")\n",
            "    CASE ELSE\n",
            "      RETURN\n",
            "  END MATCH\n",
            "END FUNC\n",
        );
        let (_, _, resources) = collect_source(&project(source));
        assert_eq!(resources.len(), 1);
    }

    #[test]
    fn trap_classification_propagates() {
        let source = concat!(
            "FUNC f() AS Integer\n",
            "  RETURN leaf()\n",
            "  TRAP(e)\n",
            "    PROPAGATE\n",
            "  END TRAP\n",
            "END FUNC\n",
        );
        let (flow, _, _) = collect_source(&project(source));
        let f = flow.iter().find(|f| f.function == "f").unwrap();
        assert_eq!(f.trap.as_ref().unwrap().classification, "propagates");
    }

    #[test]
    fn trap_classification_fails() {
        let source = concat!(
            "FUNC f() AS Integer\n",
            "  RETURN leaf()\n",
            "  TRAP(e)\n",
            "    FAIL error(2, \"x\")\n",
            "  END TRAP\n",
            "END FUNC\n",
        );
        let (flow, _, _) = collect_source(&project(source));
        let f = flow.iter().find(|f| f.function == "f").unwrap();
        assert_eq!(f.trap.as_ref().unwrap().classification, "fails");
    }

    #[test]
    fn trap_classification_returns_value() {
        let source = concat!(
            "FUNC f() AS Integer\n",
            "  RETURN leaf()\n",
            "  TRAP(e)\n",
            "    RETURN 7\n",
            "  END TRAP\n",
            "END FUNC\n",
        );
        let (flow, _, _) = collect_source(&project(source));
        let f = flow.iter().find(|f| f.function == "f").unwrap();
        assert_eq!(f.trap.as_ref().unwrap().classification, "returns value");
    }

    #[test]
    fn trap_classification_recovers() {
        let source = concat!(
            "FUNC f() AS Integer\n",
            "  RETURN leaf()\n",
            "  TRAP(e)\n",
            "    RECOVER 0\n",
            "  END TRAP\n",
            "END FUNC\n",
        );
        let (flow, _, _) = collect_source(&project(source));
        let f = flow.iter().find(|f| f.function == "f").unwrap();
        assert_eq!(f.trap.as_ref().unwrap().classification, "recovers");
    }

    #[test]
    fn user_function_becomes_fallible_by_calling_fallible_builtin() {
        let source = concat!(
            "FUNC reader() AS String\n",
            "  RETURN fs::read(\"p\")\n",
            "END FUNC\n",
            "FUNC caller() AS String\n",
            "  RETURN reader()\n",
            "END FUNC\n",
        );
        let (flow, _, _) = collect_source(&project(source));
        let reader = flow.iter().find(|f| f.function == "reader").unwrap();
        let caller = flow.iter().find(|f| f.function == "caller").unwrap();
        // `reader` calls a fallible builtin; `caller` calls fallible `reader`.
        assert!(reader.fallible);
        assert!(caller.fallible);
    }

    #[test]
    fn fail_statement_makes_function_fallible() {
        let source = concat!(
            "FUNC boom() AS Integer\n",
            "  FAIL error(1, \"x\")\n",
            "END FUNC\n",
        );
        let (flow, _, _) = collect_source(&project(source));
        assert!(flow.iter().find(|f| f.function == "boom").unwrap().fallible);
    }

    #[test]
    fn walk_visits_loop_control_and_named_and_lambda_and_trapped() {
        // Covers walk_statements' Continue/Exit/Recover arms, For step, ForEach,
        // and walk_expression's named-call-arg / lambda / constructor-named /
        // trapped-expression / member-access branches — each wrapping a fallible
        // builtin so a permission and call site are recorded.
        let source = concat!(
            "TYPE Point\n  x AS Integer\n  y AS Integer\nEND TYPE\n",
            "FUNC wrap(text AS String) AS String\n",
            "  RETURN text\n",
            "END FUNC\n",
            "FUNC f(n AS Integer) AS Integer\n",
            "  FOR i = 1 TO n STEP 2\n",
            "    io::print(toString(i))\n",
            "    CONTINUE FOR\n",
            "  NEXT\n",
            "  FOR EACH e IN items\n",
            "    EXIT FOR\n",
            "  NEXT\n",
            "  LET g AS FUNC(Integer) AS String = LAMBDA(v AS Integer) -> fs::read(\"p\")\n",
            "  LET named AS String = wrap(text := fs::read(\"n\"))\n",
            "  LET p AS Point = Point[x := fs::read(\"q\"), y := 1]\n",
            "  LET r AS String = fs::read(\"r\") TRAP(e)\n",
            "    RECOVER \"x\"\n",
            "  END TRAP\n",
            "  RETURN 0\n",
            "END FUNC\n",
        );
        let (flow, permissions, _) = collect_source(&project(source));
        assert!(flow.iter().any(|f| f.function == "f"));
        assert!(permissions.iter().any(|p| p.capability == "filesystem"));
    }

    #[test]
    fn match_guard_and_case_bodies_are_walked() {
        let source = concat!(
            "TYPE Circle\n  radius AS Integer\nEND TYPE\n",
            "UNION Shape\n  Circle\nEND UNION\n",
            "FUNC f(s AS Shape) AS Integer\n",
            "  MATCH s\n",
            "    CASE Circle(c) WHEN c.radius > 0\n",
            "      RETURN fs::read(\"p\") & \"\"\n",
            "    CASE ELSE\n",
            "      RETURN 0\n",
            "  END MATCH\n",
            "END FUNC\n",
        );
        let (flow, _, _) = collect_source(&project(source));
        // The case body raises via a fallible builtin, so `f` is fallible.
        assert!(flow.iter().find(|f| f.function == "f").unwrap().fallible);
    }

    #[test]
    fn nested_fail_and_propagate_inside_control_flow_is_detected() {
        // `statements_fail_or_propagate` / `statements_contain_*` recurse through
        // IF / FOR / MATCH; a FAIL nested in an IF still makes the function
        // fallible, and a PROPAGATE nested in a trap's FOR classifies it.
        let source = concat!(
            "FUNC boom(n AS Integer) AS Integer\n",
            "  IF n > 0 THEN\n",
            "    FAIL error(1, \"x\")\n",
            "  END IF\n",
            "  RETURN 0\n",
            "END FUNC\n",
            "FUNC classified() AS Integer\n",
            "  RETURN leaf()\n",
            "  TRAP(e)\n",
            "    FOR i = 1 TO 2\n",
            "      PROPAGATE\n",
            "    NEXT\n",
            "  END TRAP\n",
            "END FUNC\n",
        );
        let (flow, _, _) = collect_source(&project(source));
        assert!(flow.iter().find(|f| f.function == "boom").unwrap().fallible);
        let classified = flow.iter().find(|f| f.function == "classified").unwrap();
        assert_eq!(
            classified.trap.as_ref().unwrap().classification,
            "propagates"
        );
    }

    /// Classify the trap of the first function in `source`.
    fn trap_classification(source: &str) -> String {
        let (flow, _, _) = collect_source(&project(source));
        flow.iter()
            .find_map(|f| f.trap.as_ref().map(|t| t.classification.clone()))
            .expect("a trap")
    }

    #[test]
    fn classify_recurses_through_if_match_and_loop_for_propagate() {
        // PROPAGATE nested inside IF, MATCH, and a loop each classify as
        // "propagates", exercising every recursive arm of the propagate scanner.
        assert_eq!(
            trap_classification(concat!(
                "FUNC f() AS Integer\n  RETURN leaf()\n  TRAP(e)\n",
                "    IF TRUE THEN\n      PROPAGATE\n    END IF\n",
                "  END TRAP\nEND FUNC\n",
            )),
            "propagates"
        );
        assert_eq!(
            trap_classification(concat!(
                "TYPE Circle\n  radius AS Integer\nEND TYPE\n",
                "UNION Shape\n  Circle\nEND UNION\n",
                "FUNC f(s AS Shape) AS Integer\n  RETURN leaf()\n  TRAP(e)\n",
                "    MATCH s\n      CASE Circle(c)\n        PROPAGATE\n      CASE ELSE\n        RECOVER 0\n    END MATCH\n",
                "  END TRAP\nEND FUNC\n",
            )),
            "propagates"
        );
        assert_eq!(
            trap_classification(concat!(
                "FUNC f() AS Integer\n  RETURN leaf()\n  TRAP(e)\n",
                "    WHILE TRUE\n      PROPAGATE\n    WEND\n",
                "  END TRAP\nEND FUNC\n",
            )),
            "propagates"
        );
    }

    #[test]
    fn classify_recurses_through_if_match_and_loop_for_fail() {
        // FAIL nested inside IF / MATCH / loop classifies as "fails" and drives
        // the fail scanner's recursive arms (only reached when no PROPAGATE).
        assert_eq!(
            trap_classification(concat!(
                "FUNC f() AS Integer\n  RETURN leaf()\n  TRAP(e)\n",
                "    IF TRUE THEN\n      FAIL error(1, \"x\")\n    END IF\n",
                "  END TRAP\nEND FUNC\n",
            )),
            "fails"
        );
        assert_eq!(
            trap_classification(concat!(
                "TYPE Circle\n  radius AS Integer\nEND TYPE\n",
                "UNION Shape\n  Circle\nEND UNION\n",
                "FUNC f(s AS Shape) AS Integer\n  RETURN leaf()\n  TRAP(e)\n",
                "    MATCH s\n      CASE Circle(c)\n        FAIL error(1, \"x\")\n      CASE ELSE\n        RECOVER 0\n    END MATCH\n",
                "  END TRAP\nEND FUNC\n",
            )),
            "fails"
        );
        assert_eq!(
            trap_classification(concat!(
                "FUNC f() AS Integer\n  RETURN leaf()\n  TRAP(e)\n",
                "    FOR i = 1 TO 2\n      FAIL error(1, \"x\")\n    NEXT\n",
                "  END TRAP\nEND FUNC\n",
            )),
            "fails"
        );
    }

    #[test]
    fn classify_recurses_through_if_match_and_loop_for_return_value() {
        // A value RETURN nested in IF / MATCH / loop classifies as "returns value"
        // (reached only when neither PROPAGATE nor FAIL is present).
        assert_eq!(
            trap_classification(concat!(
                "FUNC f() AS Integer\n  RETURN leaf()\n  TRAP(e)\n",
                "    IF TRUE THEN\n      RETURN 1\n    END IF\n",
                "  END TRAP\nEND FUNC\n",
            )),
            "returns value"
        );
        assert_eq!(
            trap_classification(concat!(
                "TYPE Circle\n  radius AS Integer\nEND TYPE\n",
                "UNION Shape\n  Circle\nEND UNION\n",
                "FUNC f(s AS Shape) AS Integer\n  RETURN leaf()\n  TRAP(e)\n",
                "    MATCH s\n      CASE Circle(c)\n        RETURN 1\n      CASE ELSE\n        RECOVER 0\n    END MATCH\n",
                "  END TRAP\nEND FUNC\n",
            )),
            "returns value"
        );
        assert_eq!(
            trap_classification(concat!(
                "FUNC f() AS Integer\n  RETURN leaf()\n  TRAP(e)\n",
                "    FOR i = 1 TO 2\n      RETURN 1\n    NEXT\n",
                "  END TRAP\nEND FUNC\n",
            )),
            "returns value"
        );
    }

    #[test]
    fn walk_visits_diverse_expression_and_statement_forms() {
        // Exercises binary/unary/constructor/with-update/list/map/member access
        // plus assign / exit-program / expression statements, all containing a
        // fallible builtin call so the walker reaches every branch.
        let source = concat!(
            "TYPE Point\n  x AS Integer\n  y AS Integer\nEND TYPE\n",
            "FUNC f(n AS Integer) AS Integer\n",
            "  LET a AS String = fs::read(\"p\") & \"z\"\n",
            "  LET b AS Integer = -n\n",
            "  LET p AS Point = Point(fs::read(\"q\"), 2)\n",
            "  LET p2 AS Point = WITH p { x := fs::read(\"r\") }\n",
            "  LET list AS List OF String = [fs::read(\"s\")]\n",
            "  LET map AS Map OF String TO String = Map OF String TO String { \"k\" := fs::read(\"t\") }\n",
            "  LET m AS Integer = p.x\n",
            "  a = fs::read(\"u\")\n",
            "  io::print(toString(n))\n",
            "  EXIT PROGRAM n\n",
            "  RETURN 0\n",
            "END FUNC\n",
        );
        let (flow, permissions, _) = collect_source(&project(source));
        assert!(flow.iter().any(|f| f.function == "f"));
        assert!(permissions.iter().any(|p| p.capability == "filesystem"));
    }
}
