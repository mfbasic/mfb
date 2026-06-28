use super::*;


/// Namespace a decoded package's own functions and globals by its deterministic
/// identity prefix `<id>.<package>` (see `binary_repr::package_identity_id`),
/// rewriting every internal reference to match. Types are left unqualified.
///
/// The `<id>` segment makes the prefix content-addressed: identical packages
/// reached via two dependency paths collapse to one copy at merge time, while
/// two distinct packages that share a name stay separate instead of colliding.
pub fn prefix_package_symbols(pir: &mut IrProject, id: &str) {
    let prefix = format!("{id}.{}", pir.name);
    let own_fns: HashSet<String> = pir.functions.iter().map(|f| f.name.clone()).collect();
    let own_globals: HashSet<String> = pir.bindings.iter().map(|b| b.name.clone()).collect();

    for function in &mut pir.functions {
        for op in &mut function.body {
            rewrite_op_targets(op, &own_fns, &own_globals, &prefix);
        }
        for param in &mut function.params {
            if let Some(default) = &mut param.default {
                rewrite_value_targets(default, &own_fns, &own_globals, &prefix);
            }
        }
        function.name = format!("{prefix}.{}", function.name);
    }
    for binding in &mut pir.bindings {
        if let Some(value) = &mut binding.value {
            rewrite_value_targets(value, &own_fns, &own_globals, &prefix);
        }
        binding.name = format!("{prefix}.{}", binding.name);
    }
    if let Some(entry) = &mut pir.entry {
        entry.name = format!("{prefix}.{}", entry.name);
    }
}

/// The package-qualified names (`package.symbol`) by which a *consumer* and
/// other packages reference this package's functions and globals. Computed
/// *before* `prefix_package_symbols` rewrites the definitions into their
/// identity-prefixed `<id>.package.symbol` form, so `apply_package_identity`
/// can rewrite those external references to match.
pub fn package_qualified_reference_names(pir: &IrProject) -> (HashSet<String>, HashSet<String>) {
    let pkg = &pir.name;
    let fns = pir
        .functions
        .iter()
        .map(|f| format!("{pkg}.{}", f.name))
        .collect();
    let globals = pir
        .bindings
        .iter()
        .map(|b| format!("{pkg}.{}", b.name))
        .collect();
    (fns, globals)
}

/// Rewrite every *external* reference to a package's symbols — from the
/// consumer and from other packages — from `package.symbol` to the
/// identity-prefixed `<id>.package.symbol` produced by `prefix_package_symbols`.
/// The package's own internal references are already identity-prefixed and so
/// are not in `fns`/`globals`; they are left untouched.
pub fn apply_package_identity(
    project: &mut IrProject,
    fns: &HashSet<String>,
    globals: &HashSet<String>,
    id: &str,
) {
    for function in &mut project.functions {
        for op in &mut function.body {
            rewrite_op_targets(op, fns, globals, id);
        }
        for param in &mut function.params {
            if let Some(default) = &mut param.default {
                rewrite_value_targets(default, fns, globals, id);
            }
        }
    }
    for binding in &mut project.bindings {
        if let Some(value) = &mut binding.value {
            rewrite_value_targets(value, fns, globals, id);
        }
    }
}

/// Merge a namespaced package `IrProject` into `project`. Functions and globals
/// are de-duplicated by their (already namespaced) name; types by bare name.
/// Call `prefix_package_symbols` on `package` first.
pub fn merge_package(project: &mut IrProject, package: IrProject) {
    for ty in package.types {
        if !project
            .types
            .iter()
            .any(|existing| existing.name == ty.name)
        {
            project.types.push(ty);
        }
    }
    for binding in package.bindings {
        if !project
            .bindings
            .iter()
            .any(|existing| existing.name == binding.name)
        {
            project.bindings.push(binding);
        }
    }
    for function in package.functions {
        if !project
            .functions
            .iter()
            .any(|existing| existing.name == function.name)
        {
            project.functions.push(function);
        }
    }
    // Native `LINK` functions keep their package-internal `alias.func` routing
    // names (wrapper bodies reference them unprefixed), de-duplicated across
    // diamond imports (plan-linker.md §12).
    for link in package.link_functions {
        if !project
            .link_functions
            .iter()
            .any(|existing| existing.alias == link.alias && existing.name == link.name)
        {
            project.link_functions.push(link);
        }
    }
    // A re-export alias is reached by importers as `<package>.<alias>` (the IR
    // normalizes any `IMPORT … AS` binding to the package name), so qualify the
    // bare alias name with the package for routing (plan-link-update.md §5a).
    for (alias_name, target) in package.link_aliases {
        let qualified = format!("{}.{}", package.name, alias_name);
        if !project
            .link_aliases
            .iter()
            .any(|(existing, _)| existing == &qualified)
        {
            project.link_aliases.push((qualified, target));
        }
    }
}

fn qualify_target(name: &mut String, pkg: &str) {
    *name = format!("{pkg}.{name}");
}

fn rewrite_op_targets(op: &mut IrOp, fns: &HashSet<String>, globals: &HashSet<String>, pkg: &str) {
    match op {
        IrOp::Bind { value, .. } => {
            if let Some(v) = value {
                rewrite_value_targets(v, fns, globals, pkg);
            }
        }
        IrOp::Assign { value, .. }
        | IrOp::StateAssign { value, .. }
        | IrOp::Eval { value }
        | IrOp::Fail { error: value } => rewrite_value_targets(value, fns, globals, pkg),
        IrOp::AssignGlobal { name, value } => {
            if globals.contains(name) {
                qualify_target(name, pkg);
            }
            rewrite_value_targets(value, fns, globals, pkg);
        }
        IrOp::Return { value } => {
            if let Some(v) = value {
                rewrite_value_targets(v, fns, globals, pkg);
            }
        }
        IrOp::ExitLoop { .. } | IrOp::ContinueLoop { .. } => {}
        IrOp::ExitProgram { code } => rewrite_value_targets(code, fns, globals, pkg),
        IrOp::If {
            condition,
            then_body,
            else_body,
        } => {
            rewrite_value_targets(condition, fns, globals, pkg);
            for op in then_body.iter_mut().chain(else_body.iter_mut()) {
                rewrite_op_targets(op, fns, globals, pkg);
            }
        }
        IrOp::Match { value, cases } => {
            rewrite_value_targets(value, fns, globals, pkg);
            for case in cases {
                match &mut case.pattern {
                    IrMatchPattern::Else => {}
                    IrMatchPattern::Value(v) => rewrite_value_targets(v, fns, globals, pkg),
                    IrMatchPattern::OneOf(vs) => {
                        for v in vs {
                            rewrite_value_targets(v, fns, globals, pkg);
                        }
                    }
                }
                if let Some(guard) = &mut case.guard {
                    rewrite_value_targets(guard, fns, globals, pkg);
                }
                for op in &mut case.body {
                    rewrite_op_targets(op, fns, globals, pkg);
                }
            }
        }
        IrOp::While {
            condition, body, ..
        } => {
            rewrite_value_targets(condition, fns, globals, pkg);
            for op in body {
                rewrite_op_targets(op, fns, globals, pkg);
            }
        }
        IrOp::For {
            start,
            end,
            step,
            body,
            ..
        } => {
            rewrite_value_targets(start, fns, globals, pkg);
            rewrite_value_targets(end, fns, globals, pkg);
            rewrite_value_targets(step, fns, globals, pkg);
            for op in body {
                rewrite_op_targets(op, fns, globals, pkg);
            }
        }
        IrOp::DoUntil { body, condition } => {
            for op in body {
                rewrite_op_targets(op, fns, globals, pkg);
            }
            rewrite_value_targets(condition, fns, globals, pkg);
        }
        IrOp::ForEach { iterable, body, .. } => {
            rewrite_value_targets(iterable, fns, globals, pkg);
            for op in body {
                rewrite_op_targets(op, fns, globals, pkg);
            }
        }
        IrOp::Trap { body, .. } => {
            for op in body {
                rewrite_op_targets(op, fns, globals, pkg);
            }
        }
    }
}

fn rewrite_value_targets(
    value: &mut IrValue,
    fns: &HashSet<String>,
    globals: &HashSet<String>,
    pkg: &str,
) {
    match value {
        IrValue::Call { target, args, .. } | IrValue::CallResult { target, args, .. } => {
            if fns.contains(target) {
                qualify_target(target, pkg);
            }
            for arg in args {
                rewrite_value_targets(arg, fns, globals, pkg);
            }
        }
        IrValue::FunctionRef { name, .. } => {
            if fns.contains(name) {
                qualify_target(name, pkg);
            }
        }
        IrValue::Closure { name, captures, .. } => {
            if fns.contains(name) {
                qualify_target(name, pkg);
            }
            for capture in captures {
                rewrite_value_targets(capture, fns, globals, pkg);
            }
        }
        IrValue::Global(name) => {
            if globals.contains(name) {
                qualify_target(name, pkg);
            }
        }
        IrValue::Constructor { args, .. } => {
            for arg in args {
                rewrite_value_targets(arg, fns, globals, pkg);
            }
        }
        IrValue::UnionWrap { value, .. }
        | IrValue::UnionExtract { value, .. }
        | IrValue::ResultIsOk { value }
        | IrValue::ResultValue { value }
        | IrValue::ResultError { value }
        | IrValue::Unary { operand: value, .. }
        | IrValue::MemberAccess { target: value, .. } => {
            rewrite_value_targets(value, fns, globals, pkg)
        }
        IrValue::WithUpdate {
            target, updates, ..
        } => {
            rewrite_value_targets(target, fns, globals, pkg);
            for update in updates {
                rewrite_value_targets(&mut update.value, fns, globals, pkg);
            }
        }
        IrValue::ListLiteral { values, .. } => {
            for v in values {
                rewrite_value_targets(v, fns, globals, pkg);
            }
        }
        IrValue::MapLiteral { entries, .. } => {
            for (k, v) in entries {
                rewrite_value_targets(k, fns, globals, pkg);
                rewrite_value_targets(v, fns, globals, pkg);
            }
        }
        IrValue::Binary { left, right, .. } => {
            rewrite_value_targets(left, fns, globals, pkg);
            rewrite_value_targets(right, fns, globals, pkg);
        }
        IrValue::Const { .. }
        | IrValue::Local(_)
        | IrValue::LocalRef { .. }
        | IrValue::Capture { .. } => {}
    }
}

