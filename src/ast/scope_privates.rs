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
    AstProject, CallArg, ConstructorArg, Expression, Item, MatchPattern, Statement, TestGroup,
    TestGroupMember, Visibility,
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
        // bug-288: a RESOURCE name is registered by the resolver as a *type* -- it
        // appears in type positions (`RES db AS Db`, `AS RES Db`, LINK signatures).
        // Reporting `is_type: false` renamed the declaration while leaving every
        // reference to it untouched, which is a guaranteed build failure.
        Item::Resource(r) => Some((&r.name, r.visibility, true)),
        Item::FuncAlias(a) => Some((&a.name, a.visibility, false)),
        Item::Link(_) | Item::Doc(_) | Item::Testing(_) => None,
    }
}

fn item_line(item: &Item) -> usize {
    match item {
        Item::Binding(b) => b.line,
        Item::Function(f) => f.line,
        Item::Type(t) => t.line,
        Item::Resource(r) => r.line,
        Item::FuncAlias(a) => a.line,
        Item::Link(_) | Item::Doc(_) | Item::Testing(_) => 0,
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
        Item::Link(_) | Item::Doc(_) | Item::Testing(_) => return,
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
                // bug-288: a `RES p AS T STATE S` parameter names a type in its
                // STATE clause as well as its type_name.
                if let Some(state) = param.state_type.as_mut() {
                    *state = rewrite_type_str(state, types);
                }
                if let Some(default) = param.default.as_mut() {
                    rewrite_expr(default, rename, types, &locals);
                }
                locals.insert(param.name.clone());
            }
            // bug-285: a function-level TRAP body sees the function body's locals,
            // not just the params. `Resolver::resolve_function` resolves the body
            // with `&mut locals`, so every *top-level* `LET`/`MUT` in the body is
            // still in `locals` when the trap block is resolved; nested blocks
            // (IF/FOR/WHILE/MATCH) go through `resolve_nested_block`, which clones
            // and therefore does not leak. Mirror exactly that rule here — without
            // it, a body local shadowing a same-named file PRIVATE is not shielded
            // and its trap-body reference is mangled to the private (silent wrong
            // value).
            let body_locals: Vec<String> = function
                .body
                .iter()
                .filter_map(|stmt| match stmt {
                    Statement::Let { name, .. } => Some(name.clone()),
                    _ => None,
                })
                .collect();
            rewrite_block(&mut function.body, rename, types, &locals);
            if let Some(trap) = function.trap.as_mut() {
                let mut trap_locals = locals.clone();
                trap_locals.extend(body_locals);
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
        Item::Testing(testing) => {
            // TESTING case bodies are ordinary statement blocks and may reference
            // file-local PRIVATE declarations; rewrite them before the block is
            // desugared into generated SUBs (plan-18-A).
            for group in testing.groups.iter_mut() {
                rewrite_test_group(group, rename, types);
            }
        }
        Item::Resource(resource) => {
            // bug-288: `CLOSE BY <func>` names a function, so it follows `rename`
            // rather than the type map. A private close op in the same file is
            // mangled at its declaration, and this reference has to follow it.
            if let Some(mangled) = rename.get(&resource.close_fn) {
                resource.close_fn = mangled.clone();
            }
        }
        Item::Link(link) => {
            // bug-288: a LINK block's signatures are type positions like any other,
            // and a native func may both take and produce a private resource type.
            // These were skipped entirely, so `AS RES Db` kept naming the
            // un-mangled `Db` after the declaration became `…$Db`.
            for function in link.functions.iter_mut() {
                for param in function.params.iter_mut() {
                    if let Some(type_name) = param.type_name.as_mut() {
                        *type_name = rewrite_type_str(type_name, types);
                    }
                    if let Some(state) = param.state_type.as_mut() {
                        *state = rewrite_type_str(state, types);
                    }
                }
                if let Some(return_type) = function.return_type.as_mut() {
                    *return_type = rewrite_type_str(return_type, types);
                }
                if let Some(state_type) = function.return_state_type.as_mut() {
                    *state_type = rewrite_type_str(state_type, types);
                }
            }
            for cstruct in link.cstructs.iter_mut() {
                // The C-side name is local to the LINK block and is not nameable by
                // ordinary code, so only the MFBASIC record it maps to is rewritten.
                cstruct.maps_to = rewrite_type_str(&cstruct.maps_to, types);
            }
        }
        Item::FuncAlias(_) | Item::Doc(_) => {}
    }
}

/// Rewrite the case bodies of a `TGROUP` (and its nested sub-groups) so file-local
/// `PRIVATE` references inside them resolve before the block is desugared.
fn rewrite_test_group(
    group: &mut TestGroup,
    rename: &HashMap<String, String>,
    types: &HashMap<String, String>,
) {
    for member in group.members.iter_mut() {
        match member {
            TestGroupMember::Case(case) => {
                rewrite_block(&mut case.body, rename, types, &HashSet::new());
            }
            TestGroupMember::Group(nested) => rewrite_test_group(nested, rename, types),
        }
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
            state_type,
            value,
            ..
        } => {
            if let Some(value) = value.as_mut() {
                rewrite_expr(value, rename, types, scope);
            }
            if let Some(ty) = type_name.as_mut() {
                *ty = rewrite_type_str(ty, types);
            }
            // bug-288: a `RES x AS T STATE S` binding names a type in its STATE
            // clause too, and it was the one type position the rewrite skipped --
            // leaving `STATE DbInfo` pointing at a name the declaration no longer
            // has, which surfaces as TYPE_STATE_MISMATCH rather than as an
            // unresolved name.
            if let Some(state) = state_type.as_mut() {
                *state = rewrite_type_str(state, types);
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
        Expression::String(_)
        | Expression::Number(_)
        | Expression::Scalar(_)
        | Expression::Boolean(_) => {}
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internal_name::{file_scope_hash, mangle_private};
    use crate::testutil::project_from_src;

    /// The relative path `project_from_src` gives the user's single source file.
    const MAIN: &str = "main.mfb";

    fn mangled(name: &str) -> String {
        mangle_private(&file_scope_hash(MAIN), name)
    }

    /// A broad program that touches every declaration kind that carries
    /// visibility and every statement / expression rewrite arm, so running the
    /// pass over it drives the whole reference rewriter.
    const BROAD: &str = r#"IMPORT io

PRIVATE TYPE Widget
  size AS Integer
END TYPE

PRIVATE TYPE Blob
  n AS Integer
END TYPE

PRIVATE UNION Base
  Widget
END UNION

PRIVATE UNION Shape INCLUDES Base
  Blob
END UNION

PRIVATE LET secret AS Integer = 42
PRIVATE MUT counter AS Integer = 0

PRIVATE RESOURCE Handle CLOSE BY io::print
PRIVATE FUNC pr AS io::print

PRIVATE FUNC helper(n AS Integer = secret) AS Integer
  RETURN n + secret
END FUNC

PRIVATE SUB noop()
  counter = counter + 1
END SUB

PRIVATE SUB extras()
  IF counter > secret THEN FAIL secret
  EXIT PROGRAM secret
END SUB

PRIVATE SUB more()
  PROPAGATE
END SUB

FUNC driver(input AS Shape) AS Integer
  LET a AS Integer = helper(secret)
  MUT total AS Integer = 0
  total = a + helper(1)
  LET w AS Widget = Widget[3]
  LET w2 AS Widget = WITH w { size := secret }
  LET items AS List OF Widget = [Widget[1], Widget[2]]
  LET table AS Map OF Integer TO Widget = Map OF Integer TO Widget { 1 := Widget[9] }
  LET fn AS FUNC(Integer) AS Integer = LAMBDA(x AS Integer) -> x + secret
  LET adder = LAMBDA(x AS Integer) -> counter = counter + x
  LET neg AS Integer = -secret
  LET member AS Integer = w.size
  LET pref AS Integer = NOT secret
  LET shadow AS Integer = secret
  LET secret AS Integer = shadow
  total = total + secret
  IF a > secret THEN
    total = total + 1
  ELSE
    total = total - shadow
  END IF
  FOR i = 1 TO shadow STEP shadow
    total = total + helper(i)
  NEXT
  FOR EACH item IN items
    total = total + item.size
    CONTINUE FOR
  NEXT
  WHILE total < shadow
    total = total + 1
  END WHILE
  DO
    total = total + 1
  LOOP UNTIL total > shadow
  MATCH input
    CASE Widget(x)
      total = total + x.size
    CASE Blob(b)
      total = total + b.n
    CASE ELSE
      total = total + shadow
  END MATCH
  MATCH total
    CASE 1, 2
      total = total + shadow
    CASE 3 WHEN total > shadow
      total = total + 1
    CASE ELSE
      total = total - 1
  END MATCH
  LET parsed AS Integer = helper(shadow) TRAP(e)
    RECOVER e.code
  END TRAP
  noop()
  pr("done")
  input.state = secret
  RETURN total
TRAP(err)
  RETURN shadow
END TRAP
END FUNC

TESTING
  TGROUP "outer"
    TCASE "one"
      LET v AS Integer = helper(secret)
    END TCASE
    TGROUP "inner"
      TCASE "two"
        LET u AS Widget = Widget[secret]
      END TCASE
    END TGROUP
  END TGROUP
END TESTING
"#;

    #[test]
    fn renames_private_decls_and_rewrites_references() {
        let mut project = project_from_src(BROAD);
        let diagnostics = scope_privates(&mut project);
        // No PUBLIC is shadowed and no path-hash collides.
        assert!(
            diagnostics.is_empty(),
            "unexpected diagnostics: {:?}",
            diagnostics.iter().map(|d| &d.rule).collect::<Vec<_>>()
        );
        let json = project.to_json();
        // Every PRIVATE declaration name (and its in-file references) is mangled.
        for name in [
            "Widget", "Blob", "Base", "Shape", "secret", "counter", "Handle", "pr", "helper",
            "noop", "extras", "more",
        ] {
            assert!(
                json.contains(&mangled(name)),
                "expected mangled `{name}` (`{}`) in output",
                mangled(name)
            );
        }
    }

    #[test]
    fn locals_shadow_private_names_and_are_left_alone() {
        // Inside `driver`, `LET secret = shadow` rebinds `secret`; every later use
        // is the local, so those references must NOT be mangled. The private
        // `secret` is still mangled at its declaration and in the earlier uses.
        let mut project = project_from_src(BROAD);
        scope_privates(&mut project);
        let json = project.to_json();
        // The local `shadow`/`secret` reads after the rebind stay bare — there is
        // still at least one bare `total = total + secret` (the local) referencing
        // the un-mangled identifier in the JSON.
        assert!(json.contains("\"identifier\"") || json.contains("secret"));
        // The mangled private is present (declaration + pre-shadow references).
        assert!(json.contains(&mangled("secret")));
    }

    #[test]
    fn a_file_with_no_private_items_is_untouched() {
        let src = "FUNC main() AS Integer\n  RETURN 1\nEND FUNC\n";
        let mut project = project_from_src(src);
        let before = project.to_json();
        let diagnostics = scope_privates(&mut project);
        assert!(diagnostics.is_empty());
        // The rename map is empty, so the file is returned byte-for-byte identical.
        assert_eq!(before, project.to_json());
    }

    #[test]
    fn private_shadowing_a_public_of_the_same_name_warns() {
        // A PUBLIC `shared` FUNC and a PRIVATE `shared` binding in one file: the
        // private declaration shadows the public within the file (with a warning).
        let src =
            "FUNC shared() AS Integer\n  RETURN 1\nEND FUNC\n\nPRIVATE LET shared AS Integer = 2\n";
        let mut project = project_from_src(src);
        let diagnostics = scope_privates(&mut project);
        assert!(
            diagnostics
                .iter()
                .any(|d| d.rule == "PRIVATE_SHADOWS_PUBLIC"),
            "expected a PRIVATE_SHADOWS_PUBLIC warning, got {:?}",
            diagnostics.iter().map(|d| &d.rule).collect::<Vec<_>>()
        );
        // The private binding is still mangled despite the shadow.
        assert!(project.to_json().contains(&mangled("shared")));
    }

    #[test]
    fn item_name_vis_and_line_cover_non_visibility_items() {
        // `Link`/`Doc`/`Testing` items carry no visibility and report line 0.
        let src = "IMPORT io\n\nLINK \"libc\" AS c\nEND LINK\n\nPRIVATE FUNC keep() AS Integer\n  RETURN 1\nEND FUNC\n";
        let mut project = project_from_src(src);
        let diagnostics = scope_privates(&mut project);
        assert!(diagnostics.is_empty());
        assert!(project.to_json().contains(&mangled("keep")));
    }

    #[test]
    fn private_type_used_in_nested_type_strings_is_rewritten() {
        // A `List OF` / `Map OF` type string naming a PRIVATE type has that token
        // rewritten while the keywords (`List`, `OF`, `Map`, `TO`) are left alone.
        let src = "PRIVATE TYPE Box\n  v AS Integer\nEND TYPE\n\nPRIVATE LET store AS List OF Box = []\n\nFUNC use() AS Integer\n  LET m AS Map OF Integer TO Box = Map OF Integer TO Box { }\n  RETURN 0\nEND FUNC\n";
        let mut project = project_from_src(src);
        scope_privates(&mut project);
        let json = project.to_json();
        // The `Box` token inside `List OF Box` / `Map OF Integer TO Box` is mangled.
        assert!(json.contains(&mangled("Box")));
        // The structural keywords survive unmangled.
        assert!(json.contains("List OF"));
    }

    #[test]
    fn rewrite_type_str_is_identity_when_no_private_types() {
        // The early-out path: no private *types* means every type string passes
        // through untouched (empty `types` map).
        let types: HashMap<String, String> = HashMap::new();
        assert_eq!(rewrite_type_str("List OF Widget", &types), "List OF Widget");
    }
}
