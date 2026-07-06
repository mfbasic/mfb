//! plan-24-C: file-scope PRIVATE top-level names.
//!
//! `PRIVATE` is a file-local boundary: two files may each declare a `PRIVATE`
//! symbol with the same name without colliding, and a file-local `PRIVATE`
//! shadows a project `PUBLIC` of the same name *within its file* (with a
//! warning). This pass runs once, right after parsing and before the first
//! resolve, and renames every `PRIVATE` top-level declaration to an untypeable,
//! file-unique internal name `#<hash>$<name>` (see
//! [`crate::internal_name::mangle_private`]), rewriting the references *inside the
//! same file* to match. After it, private names are globally unique, so every
//! later stage (resolve, monomorph, IR, NIR, native symbols) sees distinct names
//! and needs no per-file visibility bookkeeping.
//!
//! Reference rewriting is scope-aware: a reference shadowed by a local binding
//! (param, `LET`/`MUT`, `FOR`/`FOR EACH` variable, lambda param, `MATCH` binding,
//! or `TRAP` binding) is left alone.

use crate::ast::{
    AstProject, CallArg, ConstructorArg, Expression, Item, MatchPattern, Statement, Visibility,
};
use crate::internal_name::{file_scope_hash, mangle_private};
use crate::rules::PendingDiagnostic;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

/// Rename file-local `PRIVATE` top-level declarations to `#<hash>$name` and
/// rewrite their in-file references. Returns shadow warnings and (should never
/// fire) file-hash collisions.
pub fn scope_privates(project: &mut AstProject) -> Vec<PendingDiagnostic> {
    let mut diagnostics = Vec::new();

    // 1. Per-file hash + collision guard (distinct paths must hash distinctly).
    let mut seen_hash: HashMap<String, String> = HashMap::new();
    let file_hashes: Vec<String> = project
        .files
        .iter()
        .map(|file| {
            let hash = file_scope_hash(&file.path);
            match seen_hash.get(&hash) {
                Some(prev) if prev != &file.path => diagnostics.push(PendingDiagnostic {
                    rule: "PRIVATE_PATH_HASH_COLLISION".to_string(),
                    detail: format!(
                        "file-scope hash collision between `{prev}` and `{}`",
                        file.path
                    ),
                    path: PathBuf::from(&file.path),
                    line: 1,
                }),
                Some(_) => {}
                None => {
                    seen_hash.insert(hash.clone(), file.path.clone());
                }
            }
            hash
        })
        .collect();

    // 2. Project-wide non-PRIVATE names, for shadow detection.
    let mut public_names: HashSet<String> = HashSet::new();
    for file in &project.files {
        for item in &file.items {
            if let Some((name, visibility, _)) = item_name_vis(item) {
                if !matches!(visibility, Visibility::Private) {
                    public_names.insert(name.to_string());
                }
            }
        }
    }

    // 3. Per file: build the rename maps, warn on shadowing, rename decls, rewrite refs.
    for (index, file) in project.files.iter_mut().enumerate() {
        // Toolchain-provided source (injected builtin packages, the `<builtin …>`
        // prelude) is left untouched.
        if file.internal || file.path.starts_with('<') {
            continue;
        }
        let hash = &file_hashes[index];
        let mut rename: HashMap<String, String> = HashMap::new();
        let mut private_types: HashMap<String, String> = HashMap::new();
        for item in &file.items {
            if let Some((name, Visibility::Private, is_type)) = item_name_vis(item) {
                let mangled = mangle_private(hash, name);
                if public_names.contains(name) {
                    diagnostics.push(PendingDiagnostic {
                        rule: "PRIVATE_SHADOWS_PUBLIC".to_string(),
                        detail: format!(
                            "PRIVATE `{name}` shadows a PUBLIC declaration of the same name \
                             within this file."
                        ),
                        path: PathBuf::from(&file.path),
                        line: item_line(item),
                    });
                }
                if is_type {
                    private_types.insert(name.to_string(), mangled.clone());
                }
                rename.insert(name.to_string(), mangled);
            }
        }
        if rename.is_empty() {
            continue;
        }
        for item in &mut file.items {
            rename_decl(item, &rename);
            rewrite_item_refs(item, &rename, &private_types);
        }
    }

    diagnostics
}

/// `(name, visibility, is_type)` for a top-level item that carries visibility.
fn item_name_vis(item: &Item) -> Option<(&str, Visibility, bool)> {
    match item {
        Item::Binding(b) => Some((&b.name, b.visibility, false)),
        Item::Function(f) => Some((&f.name, f.visibility, false)),
        Item::Type(t) => Some((&t.name, t.visibility, true)),
        Item::Resource(r) => Some((&r.name, r.visibility, false)),
        Item::FuncAlias(a) => Some((&a.name, a.visibility, false)),
        Item::Link(_) | Item::Doc(_) => None,
    }
}

fn item_line(item: &Item) -> usize {
    match item {
        Item::Binding(b) => b.line,
        Item::Function(f) => f.line,
        Item::Type(t) => t.line,
        Item::Resource(r) => r.line,
        Item::FuncAlias(a) => a.line,
        Item::Link(_) | Item::Doc(_) => 0,
    }
}

/// Rename the declaration's own name to its mangled form.
fn rename_decl(item: &mut Item, rename: &HashMap<String, String>) {
    let name = match item {
        Item::Binding(b) => &mut b.name,
        Item::Function(f) => &mut f.name,
        Item::Type(t) => &mut t.name,
        Item::Resource(r) => &mut r.name,
        Item::FuncAlias(a) => &mut a.name,
        Item::Link(_) | Item::Doc(_) => return,
    };
    if let Some(mangled) = rename.get(name) {
        *name = mangled.clone();
    }
}

/// Rewrite references (calls, identifiers, constructors, type annotations) inside
/// a declaration to the mangled private names.
fn rewrite_item_refs(
    item: &mut Item,
    rename: &HashMap<String, String>,
    types: &HashMap<String, String>,
) {
    match item {
        Item::Function(function) => {
            if let Some(ret) = function.return_type.as_mut() {
                *ret = rewrite_type_str(ret, types);
            }
            let mut locals: HashSet<String> = HashSet::new();
            for param in function.params.iter_mut() {
                if let Some(ty) = param.type_name.as_mut() {
                    *ty = rewrite_type_str(ty, types);
                }
                if let Some(default) = param.default.as_mut() {
                    rewrite_expr(default, rename, types, &locals);
                }
                locals.insert(param.name.clone());
            }
            rewrite_block(&mut function.body, rename, types, &locals);
            if let Some(trap) = function.trap.as_mut() {
                let mut trap_locals = locals.clone();
                trap_locals.insert(trap.name.clone());
                rewrite_block(&mut trap.body, rename, types, &trap_locals);
            }
        }
        Item::Type(type_decl) => {
            for include in type_decl.includes.iter_mut() {
                *include = rewrite_type_str(include, types);
            }
            for field in type_decl.fields.iter_mut() {
                field.type_name = rewrite_type_str(&field.type_name, types);
            }
        }
        Item::Binding(binding) => {
            if let Some(ty) = binding.type_name.as_mut() {
                *ty = rewrite_type_str(ty, types);
            }
            if let Some(value) = binding.value.as_mut() {
                rewrite_expr(value, rename, types, &HashSet::new());
            }
        }
        Item::Resource(_) | Item::FuncAlias(_) | Item::Link(_) | Item::Doc(_) => {}
    }
}

/// Process a statement block in its own scope (a clone of the enclosing locals,
/// so inner bindings do not leak out).
fn rewrite_block(
    stmts: &mut [Statement],
    rename: &HashMap<String, String>,
    types: &HashMap<String, String>,
    locals: &HashSet<String>,
) {
    let mut scope = locals.clone();
    for stmt in stmts.iter_mut() {
        rewrite_stmt(stmt, rename, types, &mut scope);
    }
}

fn rewrite_stmt(
    stmt: &mut Statement,
    rename: &HashMap<String, String>,
    types: &HashMap<String, String>,
    scope: &mut HashSet<String>,
) {
    match stmt {
        Statement::Let {
            name,
            type_name,
            value,
            ..
        } => {
            if let Some(value) = value.as_mut() {
                rewrite_expr(value, rename, types, scope);
            }
            if let Some(ty) = type_name.as_mut() {
                *ty = rewrite_type_str(ty, types);
            }
            // The binding is in scope only after its initializer.
            scope.insert(name.clone());
        }
        Statement::Assign { name, value, .. } => {
            rewrite_expr(value, rename, types, scope);
            if !scope.contains(name) {
                if let Some(mangled) = rename.get(name) {
                    *name = mangled.clone();
                }
            }
        }
        Statement::StateAssign { value, .. } => rewrite_expr(value, rename, types, scope),
        Statement::Return { value, .. } | Statement::Recover { value, .. } => {
            if let Some(value) = value.as_mut() {
                rewrite_expr(value, rename, types, scope);
            }
        }
        Statement::Exit { code, .. } => {
            if let Some(code) = code.as_mut() {
                rewrite_expr(code, rename, types, scope);
            }
        }
        Statement::Fail { error, .. } => rewrite_expr(error, rename, types, scope),
        Statement::Expression { expression, .. } => rewrite_expr(expression, rename, types, scope),
        Statement::If {
            condition,
            then_body,
            else_body,
            ..
        } => {
            rewrite_expr(condition, rename, types, scope);
            rewrite_block(then_body, rename, types, scope);
            rewrite_block(else_body, rename, types, scope);
        }
        Statement::Match {
            expression, cases, ..
        } => {
            rewrite_expr(expression, rename, types, scope);
            for case in cases.iter_mut() {
                let mut case_scope = scope.clone();
                match &mut case.pattern {
                    MatchPattern::Literal(expr) => rewrite_expr(expr, rename, types, scope),
                    MatchPattern::OneOf(exprs) => {
                        for expr in exprs.iter_mut() {
                            rewrite_expr(expr, rename, types, scope);
                        }
                    }
                    MatchPattern::Union { type_name, binding } => {
                        *type_name = rewrite_type_str(type_name, types);
                        case_scope.insert(binding.clone());
                    }
                    MatchPattern::Else => {}
                }
                if let Some(guard) = case.guard.as_mut() {
                    rewrite_expr(guard, rename, types, &case_scope);
                }
                rewrite_block(&mut case.body, rename, types, &case_scope);
            }
        }
        Statement::For {
            name,
            start,
            end,
            step,
            body,
            ..
        } => {
            rewrite_expr(start, rename, types, scope);
            rewrite_expr(end, rename, types, scope);
            if let Some(step) = step.as_mut() {
                rewrite_expr(step, rename, types, scope);
            }
            let mut body_scope = scope.clone();
            body_scope.insert(name.clone());
            rewrite_block(body, rename, types, &body_scope);
        }
        Statement::ForEach {
            name,
            iterable,
            body,
            ..
        } => {
            rewrite_expr(iterable, rename, types, scope);
            let mut body_scope = scope.clone();
            body_scope.insert(name.clone());
            rewrite_block(body, rename, types, &body_scope);
        }
        Statement::While {
            condition, body, ..
        } => {
            rewrite_expr(condition, rename, types, scope);
            rewrite_block(body, rename, types, scope);
        }
        Statement::DoUntil {
            body, condition, ..
        } => {
            rewrite_block(body, rename, types, scope);
            rewrite_expr(condition, rename, types, scope);
        }
        Statement::Continue { .. } | Statement::Propagate { .. } => {}
    }
}

fn rewrite_expr(
    expr: &mut Expression,
    rename: &HashMap<String, String>,
    types: &HashMap<String, String>,
    locals: &HashSet<String>,
) {
    match expr {
        Expression::Binary { left, right, .. } => {
            rewrite_expr(left, rename, types, locals);
            rewrite_expr(right, rename, types, locals);
        }
        Expression::Unary { operand, .. } => rewrite_expr(operand, rename, types, locals),
        Expression::Call {
            callee, arguments, ..
        } => {
            if !callee.contains('.') && !locals.contains(callee.as_str()) {
                if let Some(mangled) = rename.get(callee) {
                    *callee = mangled.clone();
                }
            }
            for arg in arguments.iter_mut() {
                match arg {
                    CallArg::Positional(value) | CallArg::Named { value, .. } => {
                        rewrite_expr(value, rename, types, locals)
                    }
                }
            }
        }
        Expression::Lambda {
            params,
            body,
            assign_target,
        } => {
            let mut inner = locals.clone();
            for param in params.iter() {
                inner.insert(param.name.clone());
            }
            if let Some(target) = assign_target.as_mut() {
                if !inner.contains(target.as_str()) {
                    if let Some(mangled) = rename.get(target) {
                        *target = mangled.clone();
                    }
                }
            }
            rewrite_expr(body, rename, types, &inner);
        }
        Expression::Constructor {
            type_name,
            arguments,
        } => {
            if !locals.contains(type_name.as_str()) {
                *type_name = rewrite_type_str(type_name, types);
            }
            for arg in arguments.iter_mut() {
                match arg {
                    ConstructorArg::Positional(value) | ConstructorArg::Named { value, .. } => {
                        rewrite_expr(value, rename, types, locals)
                    }
                }
            }
        }
        Expression::WithUpdate { target, updates } => {
            rewrite_expr(target, rename, types, locals);
            for update in updates.iter_mut() {
                rewrite_expr(&mut update.value, rename, types, locals);
            }
        }
        Expression::ListLiteral(items) => {
            for item in items.iter_mut() {
                rewrite_expr(item, rename, types, locals);
            }
        }
        Expression::MapLiteral {
            key_type,
            value_type,
            entries,
        } => {
            *key_type = rewrite_type_str(key_type, types);
            *value_type = rewrite_type_str(value_type, types);
            for (key, value) in entries.iter_mut() {
                rewrite_expr(key, rename, types, locals);
                rewrite_expr(value, rename, types, locals);
            }
        }
        Expression::MemberAccess { target, .. } => rewrite_expr(target, rename, types, locals),
        Expression::Trapped {
            expression,
            binding,
            handler,
            ..
        } => {
            rewrite_expr(expression, rename, types, locals);
            let mut inner = locals.clone();
            inner.insert(binding.clone());
            rewrite_block(handler, rename, types, &inner);
        }
        Expression::Identifier(name) => {
            if !locals.contains(name.as_str()) {
                if let Some(mangled) = rename.get(name) {
                    *expr = Expression::Identifier(mangled.clone());
                }
            }
        }
        Expression::String(_) | Expression::Number(_) | Expression::Boolean(_) => {}
    }
}

/// Rewrite each identifier token of a type string that names a file-local PRIVATE
/// type to its mangled name. Type strings are token sequences (`List OF Foo`,
/// `Map OF K TO V`, `FUNC(Foo) AS Bar`), so replacing whole identifier tokens is
/// safe — type keywords (`OF`, `TO`, `AS`, `FUNC`) can never be a user type name.
fn rewrite_type_str(type_str: &str, types: &HashMap<String, String>) -> String {
    if types.is_empty() {
        return type_str.to_string();
    }
    let mut out = String::with_capacity(type_str.len());
    let mut ident = String::new();
    let flush = |ident: &mut String, out: &mut String| {
        if !ident.is_empty() {
            out.push_str(types.get(ident.as_str()).map_or(ident.as_str(), |m| m));
            ident.clear();
        }
    };
    for ch in type_str.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            ident.push(ch);
        } else {
            flush(&mut ident, &mut out);
            out.push(ch);
        }
    }
    flush(&mut ident, &mut out);
    out
}
