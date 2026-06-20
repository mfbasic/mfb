//! Assembles an [`AuditReport`] from the project manifest, parsed source, and
//! installed packages. All collection is offline and reuses the same project,
//! package, and `.mfp` helpers that builds use (via `crate::`).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use tinyjson::JsonValue;

use super::report::*;
use crate::ast::{self, CallArg, ConstructorArg, Expression, Function, Item, Statement};

/// Inputs handed to the collector after the front-end pipeline has run.
pub struct AuditInputs<'a> {
    pub project_dir: &'a Path,
    pub root_display: String,
    pub manifest: &'a HashMap<String, JsonValue>,
    pub ast: &'a ast::AstProject,
    pub kind: String,
    pub entry: Option<String>,
    pub locked: bool,
}

pub fn collect(inputs: &AuditInputs) -> AuditReport {
    let project = project_summary(inputs);
    let dependencies = collect_dependencies(inputs.project_dir, inputs.manifest);
    let packages = collect_packages(inputs.project_dir, inputs.manifest);
    let (source_flow, permissions, resources) = collect_source(inputs.ast);
    let lockfile = collect_lockfile(inputs.project_dir, inputs.manifest, inputs.locked);

    let mut findings = Vec::new();
    lockfile_findings(&lockfile, &dependencies, inputs, &mut findings);
    dependency_findings(&dependencies, &mut findings);
    package_findings(inputs.project_dir, inputs.manifest, &packages, &mut findings);
    resource_findings(&resources, &mut findings);
    permission_findings(&permissions, &mut findings);
    sort_findings(&mut findings);

    AuditReport {
        project,
        lockfile,
        dependencies,
        packages,
        source_flow,
        resources,
        native_links: Vec::new(),
        permissions,
        findings,
    }
}

fn project_summary(inputs: &AuditInputs) -> ProjectSummary {
    let manifest = inputs.manifest;
    let name = manifest_string(manifest, "name").unwrap_or_default();
    let ident = manifest_string(manifest, "ident").unwrap_or_else(|| name.clone());
    let version = manifest_string(manifest, "version").unwrap_or_default();
    let language_version = manifest_string(manifest, "mfb").unwrap_or_default();
    ProjectSummary {
        name,
        ident,
        version,
        kind: inputs.kind.clone(),
        entry: inputs.entry.clone(),
        root: inputs.root_display.clone(),
        language_version,
    }
}

// ---------------------------------------------------------------------------
// Dependencies and packages
// ---------------------------------------------------------------------------

fn collect_dependencies(
    project_dir: &Path,
    manifest: &HashMap<String, JsonValue>,
) -> Vec<DependencyEntry> {
    let Some(packages) = manifest
        .get("packages")
        .and_then(|value| value.get::<Vec<JsonValue>>())
    else {
        return Vec::new();
    };

    let mut entries: Vec<DependencyEntry> = packages
        .iter()
        .filter_map(crate::project_package_dependency)
        .map(|dependency| {
            let package_file = project_dir
                .join("packages")
                .join(format!("{}.mfp", dependency.name));
            let mut resolved_version = None;
            let mut content_hash = None;
            let mut signature = None;
            let status;

            if package_file.is_file() {
                match crate::read_mfp_header(&package_file) {
                    Ok(header) => {
                        resolved_version = Some(header.version.clone());
                        signature = Some(crate::signature_type_name(header.signature_type));
                        content_hash = std::fs::read(&package_file)
                            .ok()
                            .and_then(|bytes| {
                                crate::target::package_mfp::package_content_hash(&bytes).ok()
                            })
                            .map(|hash| crate::hex_bytes(&hash));
                        status = verify_status_label(crate::package_dependency_status(
                            &dependency,
                            &header.name,
                            &header.ident,
                            &header.version,
                        ));
                    }
                    Err(_) => status = "invalid".to_string(),
                }
            } else {
                status = "missing".to_string();
            }

            DependencyEntry {
                name: dependency.name,
                ident: dependency.ident,
                requested_version: dependency.version,
                resolved_version,
                pin: dependency.pin,
                source: dependency.source,
                signature,
                content_hash,
                status,
            }
        })
        .collect();

    entries.sort_by(|a, b| a.name.cmp(&b.name));
    entries
}

fn verify_status_label(status: crate::PackageVerifyStatus) -> String {
    match status {
        crate::PackageVerifyStatus::Ok => "ok",
        crate::PackageVerifyStatus::NeedsUpdate => "needs-update",
        crate::PackageVerifyStatus::InvalidPackage => "invalid",
    }
    .to_string()
}

fn collect_packages(
    project_dir: &Path,
    manifest: &HashMap<String, JsonValue>,
) -> Vec<PackageEntry> {
    let Some(packages) = manifest
        .get("packages")
        .and_then(|value| value.get::<Vec<JsonValue>>())
    else {
        return Vec::new();
    };

    let mut entries = Vec::new();
    for dependency in packages.iter().filter_map(crate::project_package_dependency) {
        let package_file = project_dir
            .join("packages")
            .join(format!("{}.mfp", dependency.name));
        if !package_file.is_file() {
            continue;
        }
        let display = format!("packages/{}.mfp", dependency.name);
        let header = crate::read_mfp_header(&package_file);
        let info = crate::binary_repr::read_package_info(&package_file);
        let content_hash = std::fs::read(&package_file)
            .ok()
            .and_then(|bytes| crate::target::package_mfp::package_content_hash(&bytes).ok())
            .map(|hash| crate::hex_bytes(&hash))
            .unwrap_or_default();

        match (header, info) {
            (Ok(header), Ok(info)) => entries.push(PackageEntry {
                name: header.name.clone(),
                version: header.version.clone(),
                path: display,
                signature: crate::signature_type_name(header.signature_type),
                content_hash,
                verifier: "ok".to_string(),
                exports: info.export_count,
                imports: info.import_count,
                cleanups: info.cleanup_count,
            }),
            _ => entries.push(PackageEntry {
                name: dependency.name.clone(),
                version: String::new(),
                path: display,
                signature: "unknown".to_string(),
                content_hash,
                verifier: "failed".to_string(),
                exports: 0,
                imports: 0,
                cleanups: 0,
            }),
        }
    }

    entries.sort_by(|a, b| a.name.cmp(&b.name));
    entries
}

// ---------------------------------------------------------------------------
// Lockfile
// ---------------------------------------------------------------------------

fn collect_lockfile(
    project_dir: &Path,
    manifest: &HashMap<String, JsonValue>,
    locked: bool,
) -> LockfileSummary {
    let lock_path = project_dir.join("mfb.lock");
    let display = "mfb.lock".to_string();
    if !lock_path.is_file() {
        return LockfileSummary {
            path: display,
            present: false,
            locked,
            version: None,
            project_hash_matches: None,
        };
    }

    let mut version = None;
    let mut project_hash_matches = None;
    if let Ok(contents) = std::fs::read_to_string(&lock_path) {
        if let Ok(value) = contents.parse::<JsonValue>() {
            if let Some(object) = value.get::<HashMap<String, JsonValue>>() {
                version = object
                    .get("lockfileVersion")
                    .and_then(|value| value.get::<f64>())
                    .map(|value| *value as i64);
                let stored = object
                    .get("projectHash")
                    .and_then(|value| value.get::<String>())
                    .cloned()
                    .unwrap_or_default();
                project_hash_matches = Some(stored == project_hash(manifest));
            }
        }
    }

    LockfileSummary {
        path: display,
        present: true,
        locked,
        version,
        project_hash_matches,
    }
}

/// Lowercase hex SHA-256 over a canonical, sorted serialization of the
/// `project.json` `packages[]` request tuples.
pub fn project_hash(manifest: &HashMap<String, JsonValue>) -> String {
    use sha2::{Digest, Sha256};

    let mut tuples: Vec<String> = manifest
        .get("packages")
        .and_then(|value| value.get::<Vec<JsonValue>>())
        .into_iter()
        .flatten()
        .filter_map(crate::project_package_dependency)
        .map(|dependency| {
            format!(
                "{}\u{0}{}\u{0}{}\u{0}{}\u{0}{}\n",
                dependency.name,
                dependency.ident,
                dependency.version,
                dependency.pin,
                dependency.source
            )
        })
        .collect();
    tuples.sort();

    let mut hasher = Sha256::new();
    for tuple in tuples {
        hasher.update(tuple.as_bytes());
    }
    crate::hex_bytes(hasher.finalize().as_slice())
}

// ---------------------------------------------------------------------------
// Source flow, permissions, resources
// ---------------------------------------------------------------------------

fn collect_source(
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

fn collect_resources(
    function: &str,
    path: &str,
    body: &[Statement],
    out: &mut Vec<ResourceEntry>,
) {
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
            Statement::Fail { error, line } => walk_expression(error, *line, visit),
            Statement::Propagate { .. } => {}
            Statement::Recover { value, line } => {
                if let Some(expr) = value {
                    walk_expression(expr, *line, visit);
                }
            }
            Statement::Assign { value, line, .. } => walk_expression(value, *line, visit),
            Statement::Expression { expression, line } => {
                walk_expression(expression, *line, visit)
            }
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
        Expression::Call { callee, arguments } => {
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
        Statement::Match { cases, .. } => {
            cases.iter().any(|case| statements_contain_propagate(&case.body))
        }
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
            statements_contain_return_value(then_body)
                || statements_contain_return_value(else_body)
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

// ---------------------------------------------------------------------------
// Builtin capability and fallibility tables
// ---------------------------------------------------------------------------

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
    if matches!(package_of(callee), "fs" | "io" | "json" | "thread") {
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
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Findings
// ---------------------------------------------------------------------------

fn lockfile_findings(
    lockfile: &LockfileSummary,
    _dependencies: &[DependencyEntry],
    inputs: &AuditInputs,
    findings: &mut Vec<Finding>,
) {
    if lockfile.locked && !lockfile.present {
        findings.push(Finding {
            code: "AUDIT-LOCK-MISSING".to_string(),
            category: "lockfile".to_string(),
            severity: Severity::Error,
            message: "mfb.lock is required by --locked but was not found".to_string(),
            path: Some(lockfile.path.clone()),
            line: None,
            package: None,
        });
        return;
    }

    if lockfile.present && lockfile.project_hash_matches == Some(false) {
        let severity = if lockfile.locked {
            Severity::Error
        } else {
            Severity::Warning
        };
        findings.push(Finding {
            code: "AUDIT-LOCK-STALE".to_string(),
            category: "lockfile".to_string(),
            severity,
            message: "mfb.lock projectHash does not match project.json packages".to_string(),
            path: Some(lockfile.path.clone()),
            line: None,
            package: None,
        });
    }

    let _ = inputs;
}

fn dependency_findings(dependencies: &[DependencyEntry], findings: &mut Vec<Finding>) {
    for dependency in dependencies {
        match dependency.status.as_str() {
            "missing" => findings.push(Finding {
                code: "AUDIT-DEP-MISSING".to_string(),
                category: "dependency".to_string(),
                severity: Severity::Error,
                message: format!(
                    "declared package `{}` is not installed under packages/",
                    dependency.name
                ),
                path: None,
                line: None,
                package: Some(dependency.name.clone()),
            }),
            "invalid" => findings.push(Finding {
                code: "AUDIT-DEP-INVALID".to_string(),
                category: "dependency".to_string(),
                severity: Severity::Error,
                message: format!("package `{}` is invalid or unreadable", dependency.name),
                path: None,
                line: None,
                package: Some(dependency.name.clone()),
            }),
            "needs-update" => findings.push(Finding {
                code: "AUDIT-DEP-OUTDATED".to_string(),
                category: "dependency".to_string(),
                severity: Severity::Warning,
                message: format!(
                    "package `{}` does not satisfy requested version {}",
                    dependency.name, dependency.requested_version
                ),
                path: None,
                line: None,
                package: Some(dependency.name.clone()),
            }),
            _ => {}
        }
    }
}

fn package_findings(
    project_dir: &Path,
    manifest: &HashMap<String, JsonValue>,
    packages: &[PackageEntry],
    findings: &mut Vec<Finding>,
) {
    for package in packages {
        if package.verifier == "failed" {
            findings.push(Finding {
                code: "AUDIT-PKG-VERIFY-FAILED".to_string(),
                category: "package".to_string(),
                severity: Severity::Error,
                message: format!("package `{}` failed to verify", package.name),
                path: Some(package.path.clone()),
                line: None,
                package: Some(package.name.clone()),
            });
            continue;
        }
        if package.signature == "unsigned" {
            findings.push(Finding {
                code: "AUDIT-PKG-UNSIGNED".to_string(),
                category: "package".to_string(),
                severity: Severity::Info,
                message: format!("package `{}` is unsigned", package.name),
                path: Some(package.path.clone()),
                line: None,
                package: Some(package.name.clone()),
            });
        }
    }

    // Exported mutable state and secondary-close cleanup metadata come from the
    // detailed package info, which we re-read for the audit notes.
    let Some(declared) = manifest
        .get("packages")
        .and_then(|value| value.get::<Vec<JsonValue>>())
    else {
        return;
    };
    for dependency in declared.iter().filter_map(crate::project_package_dependency) {
        let package_file = project_dir
            .join("packages")
            .join(format!("{}.mfp", dependency.name));
        let Ok(info) = crate::binary_repr::read_package_info(&package_file) else {
            continue;
        };
        let display = format!("packages/{}.mfp", dependency.name);
        for global in &info.globals {
            if global.mutable && global.visibility == "export" {
                findings.push(Finding {
                    code: "AUDIT-PKG-STATE-EXPORTED-MUT".to_string(),
                    category: "package".to_string(),
                    severity: Severity::Warning,
                    message: format!(
                        "package `{}` exports mutable state `{}`",
                        info.manifest_name, global.name
                    ),
                    path: Some(display.clone()),
                    line: None,
                    package: Some(info.manifest_name.clone()),
                });
            }
        }
        for cleanup in &info.cleanups {
            if cleanup.records_secondary_close_failure {
                findings.push(Finding {
                    code: "AUDIT-RESOURCE-SECONDARY-CLOSE".to_string(),
                    category: "resource".to_string(),
                    severity: Severity::Info,
                    message: format!(
                        "package `{}` cleanup in `{}` records secondary close failures",
                        info.manifest_name, cleanup.function
                    ),
                    path: Some(display.clone()),
                    line: None,
                    package: Some(info.manifest_name.clone()),
                });
            }
        }
    }
}

fn resource_findings(resources: &[ResourceEntry], findings: &mut Vec<Finding>) {
    for resource in resources {
        if resource.close_may_fail {
            findings.push(Finding {
                code: "AUDIT-RESOURCE-CLOSE-MAY-FAIL".to_string(),
                category: "resource".to_string(),
                severity: Severity::Info,
                message: format!(
                    "resource `{}` ({}) is closed by lexical drop; explicit `{}` is required to observe a close failure",
                    resource.name, resource.resource_type, resource.close_op
                ),
                path: Some(resource.path.clone()),
                line: Some(resource.line),
                package: None,
            });
        }
    }
}

fn permission_findings(permissions: &[PermissionEntry], findings: &mut Vec<Finding>) {
    let mut seen = HashSet::new();
    for permission in permissions {
        if !seen.insert(permission.capability.clone()) {
            continue;
        }
        let code = match permission.capability.as_str() {
            "filesystem" => "AUDIT-PERM-FILESYSTEM",
            "network" => "AUDIT-PERM-NETWORK",
            "terminal" => "AUDIT-PERM-TERMINAL",
            "threads" => "AUDIT-PERM-THREADS",
            "process" => "AUDIT-PERM-PROCESS",
            "environment" => "AUDIT-PERM-ENVIRONMENT",
            "clock" => "AUDIT-PERM-CLOCK",
            "randomness" => "AUDIT-PERM-RANDOMNESS",
            "native" => "AUDIT-PERM-NATIVE",
            _ => "AUDIT-PERM-OTHER",
        };
        findings.push(Finding {
            code: code.to_string(),
            category: "permission".to_string(),
            severity: Severity::Info,
            message: format!("project uses host capability: {}", permission.capability),
            path: None,
            line: None,
            package: None,
        });
    }
}

fn sort_findings(findings: &mut [Finding]) {
    findings.sort_by(|a, b| {
        AuditReport::category_rank(&a.category)
            .cmp(&AuditReport::category_rank(&b.category))
            .then(a.code.cmp(&b.code))
            .then(a.path.cmp(&b.path))
            .then(a.line.cmp(&b.line))
            .then(a.message.cmp(&b.message))
    });
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn manifest_string(manifest: &HashMap<String, JsonValue>, key: &str) -> Option<String> {
    manifest
        .get(key)
        .and_then(|value| value.get::<String>())
        .cloned()
}
