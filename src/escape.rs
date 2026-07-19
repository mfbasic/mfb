//! Resource escape analysis (mfbasic.md §15.6).
//!
//! A resource is owned by a *scope*. By default that is the scope where the
//! resource is produced. When a borrow of a `RES` binding is added to a
//! collection (a `List` element or `Map` value), ownership **floats up** to the
//! outermost scope that references the resource — the declaring scope of the
//! outermost collection it reaches — and, when such a collection is `RETURN`ed,
//! out to the caller.
//!
//! This module computes, per `RES` binding name in a function, where its close
//! obligation lives:
//!
//! * [`ResOwner::Local`] — owned at its own producing scope (the existing
//!   per-scope static cleanup is already correct).
//! * [`ResOwner::Float`] — ownership floats up to the named collection binding's
//!   scope; the obligation is drained from that scope's runtime owned-list (and
//!   transferred to the caller when that collection is `RETURN`ed).
//!
//! The analysis is purely syntactic over the AST and depends only on which local
//! names are `RES` bindings, so the type checker and IR lowering compute the
//! same answer independently. It is sound because a borrowed resource cannot
//! escape a callee (`TYPE_RESOURCE_BORROW_INVALIDATE`): a resource only ever
//! enters a collection inside the function that owns it, by direct insertion of
//! a `RES`-binding identifier.

use crate::ast::{CallArg, Expression, Function, Statement};
use std::collections::{HashMap, HashSet};

/// Where a `RES` binding's close obligation is discharged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResOwner {
    /// Closed at the binding's own producing scope (no float).
    Local,
    /// Ownership floats up to the scope of this collection binding; the
    /// obligation is drained from that scope's runtime owned-list. When that
    /// collection is `RETURN`ed (`List OF RES File`), the `RETURN` transfers the
    /// owned-list to the caller's scope instead of draining it (§15.6).
    Float(String),
    /// The resource flows into a collection that is `RETURN`ed, but the float
    /// cannot be honored: the collection is declared *after* the resource (or in
    /// an inner scope), so its runtime owned-list does not exist yet when the
    /// resource is produced.
    ///
    /// bug-291: this case previously collapsed to [`ResOwner::Local`], which is a
    /// silent miscompile -- the resource was closed at function exit while the
    /// returned collection still carried it, and the caller's adopted owned-list
    /// then closed it a second time. Modelling it separately lets `ir::verify`
    /// reject it with a diagnostic naming both bindings instead. Lowering never
    /// sees it, because verification rejects the program first.
    FloatBlocked(String),
}

/// Per-function resource ownership decisions, keyed by `RES` binding name.
#[derive(Clone, Debug, Default)]
pub struct FunctionEscape {
    owners: HashMap<String, ResOwner>,
}

impl FunctionEscape {
    /// The owner of a `RES` binding; [`ResOwner::Local`] when it does not float.
    #[cfg(test)]
    pub fn owner(&self, res_name: &str) -> ResOwner {
        self.owners
            .get(res_name)
            .cloned()
            .unwrap_or(ResOwner::Local)
    }

    /// Whether the binding's ownership has floated away from its own scope (into
    /// an outer collection, or out via return). Such a binding becomes
    /// borrow-only: it may not close, `RETURN`, or `thread::transfer`.
    #[cfg(test)]
    pub fn floats(&self, res_name: &str) -> bool {
        !matches!(self.owner(res_name), ResOwner::Local)
    }

    /// The full map of `RES` binding name to owner decision. Bindings absent
    /// from the map are [`ResOwner::Local`].
    pub fn owners(&self) -> &HashMap<String, ResOwner> {
        &self.owners
    }
}

/// The destination a collection value flows into.
enum Target {
    Var(String),
    Returned,
}

/// One "a collection value carrying resource borrows flows into `target`" fact.
struct Routing {
    target: Target,
    /// `RES`-binding names inserted directly as elements at this site.
    res_elems: Vec<String>,
    /// Collection bindings whose contents also flow into `target` (copy /
    /// `append(C, …)` / nesting).
    src_collections: Vec<String>,
}

struct Analyzer {
    res_names: HashSet<String>,
    /// Declaration depth (block nesting) of every local binding, by name.
    decl_depth: HashMap<String, usize>,
    /// Declaration order index of every local binding, for deterministic ties.
    decl_order: HashMap<String, usize>,
    /// Declared type of each binding, when it carried one. Used only to tell a
    /// collection that can actually *own* resources (`List OF RES File`) from a
    /// bare one, so the bug-291 rejection does not pile onto a program already
    /// rejected for the missing `RES` marker.
    decl_type: HashMap<String, String>,
    res_depth: HashMap<String, usize>,
    routings: Vec<Routing>,
    next_order: usize,
}

/// Analyze a function body, returning per-`RES`-binding ownership decisions.
pub fn analyze_function(function: &Function) -> FunctionEscape {
    let mut analyzer = Analyzer {
        res_names: HashSet::new(),
        decl_depth: HashMap::new(),
        decl_order: HashMap::new(),
        decl_type: HashMap::new(),
        res_depth: HashMap::new(),
        routings: Vec::new(),
        next_order: 0,
    };

    // `RES` parameters are resources owned at function-entry depth.
    for param in &function.params {
        if param.resource {
            analyzer.declare(&param.name, 0);
            analyzer.res_names.insert(param.name.clone());
            analyzer.res_depth.insert(param.name.clone(), 0);
        }
    }

    analyzer.walk(&function.body, 0);
    if let Some(trap) = &function.trap {
        analyzer.walk(&trap.body, 1);
    }

    analyzer.solve()
}

impl Analyzer {
    fn declare(&mut self, name: &str, depth: usize) {
        self.decl_depth.entry(name.to_string()).or_insert(depth);
        self.decl_order.entry(name.to_string()).or_insert_with(|| {
            let order = self.next_order;
            self.next_order += 1;
            order
        });
    }

    fn walk(&mut self, body: &[Statement], depth: usize) {
        for statement in body {
            self.walk_statement(statement, depth);
        }
    }

    fn walk_statement(&mut self, statement: &Statement, depth: usize) {
        match statement {
            Statement::Let {
                resource,
                name,
                type_name,
                value,
                ..
            } => {
                self.declare(name, depth);
                if let Some(type_name) = type_name {
                    self.decl_type
                        .entry(name.clone())
                        .or_insert_with(|| type_name.clone());
                }
                if *resource {
                    self.res_names.insert(name.clone());
                    self.res_depth.insert(name.clone(), depth);
                }
                if let Some(value) = value {
                    self.record_routing(Target::Var(name.clone()), value);
                }
            }
            Statement::Assign { name, value, .. } => {
                self.declare(name, depth);
                self.record_routing(Target::Var(name.clone()), value);
            }
            Statement::Return {
                value: Some(value), ..
            } => {
                self.record_routing(Target::Returned, value);
            }
            Statement::If {
                then_body,
                else_body,
                ..
            } => {
                self.walk(then_body, depth + 1);
                self.walk(else_body, depth + 1);
            }
            Statement::Match { cases, .. } => {
                for case in cases {
                    self.walk(&case.body, depth + 1);
                }
            }
            Statement::For { body, .. }
            | Statement::ForEach { body, .. }
            | Statement::While { body, .. }
            | Statement::DoUntil { body, .. } => {
                self.walk(body, depth + 1);
            }
            _ => {}
        }
    }

    fn record_routing(&mut self, target: Target, expr: &Expression) {
        let mut res_elems = Vec::new();
        let mut src_collections = Vec::new();
        self.scan_collection_expr(expr, &mut res_elems, &mut src_collections);
        if res_elems.is_empty() && src_collections.is_empty() {
            return;
        }
        self.routings.push(Routing {
            target,
            res_elems,
            src_collections,
        });
    }

    /// Collect the resources directly inserted, and source collections merged,
    /// by a collection-valued expression.
    fn scan_collection_expr(
        &self,
        expr: &Expression,
        res_elems: &mut Vec<String>,
        src_collections: &mut Vec<String>,
    ) {
        match expr {
            Expression::Identifier(name) => {
                // A bare resource identifier in value position is not a
                // collection (e.g. `RETURN f`, `LET g = f`); it only escapes when
                // it appears as a collection *element* (see `scan_element`). A
                // non-resource identifier is a plain collection copy `V = C`.
                if !self.res_names.contains(name) {
                    src_collections.push(name.clone());
                }
            }
            Expression::ListLiteral(values) => {
                for value in values {
                    self.scan_element(value, res_elems, src_collections);
                }
            }
            Expression::MapLiteral { entries, .. } => {
                for (_, value) in entries {
                    self.scan_element(value, res_elems, src_collections);
                }
            }
            Expression::Call {
                callee, arguments, ..
            } if is_insertion_builtin(callee) => {
                for (index, arg) in arguments.iter().enumerate() {
                    let value = call_arg_expr(arg);
                    if index == 0 {
                        // The collection being updated flows into the result.
                        self.scan_collection_expr(value, res_elems, src_collections);
                    } else {
                        self.scan_element(value, res_elems, src_collections);
                    }
                }
            }
            // bug-290: an inline `TRAP` wraps the expression it guards, and this
            // scan previously fell through to `_ => {}` for it -- so
            // `xs = insert(xs, 0, f) TRAP … END TRAP` routed no ownership at all,
            // `f` stayed `ResOwner::Local`, and it was closed at its own scope
            // while the collection still held it. Both arms of the trap produce
            // the same target, so both flow into it: the guarded expression on
            // success, and whatever the handler `RECOVER`s on failure.
            Expression::Trapped {
                expression,
                handler,
                ..
            } => {
                self.scan_collection_expr(expression, res_elems, src_collections);
                for statement in handler {
                    if let Statement::Recover {
                        value: Some(value), ..
                    } = statement
                    {
                        self.scan_collection_expr(value, res_elems, src_collections);
                    }
                }
            }
            _ => {}
        }
    }

    /// An element position: a `RES` identifier is a direct insertion; a nested
    /// collection expression contributes its own reachable resources.
    fn scan_element(
        &self,
        expr: &Expression,
        res_elems: &mut Vec<String>,
        src_collections: &mut Vec<String>,
    ) {
        if let Expression::Identifier(name) = expr {
            if self.res_names.contains(name) {
                res_elems.push(name.clone());
                return;
            }
        }
        self.scan_collection_expr(expr, res_elems, src_collections);
    }

    fn solve(&self) -> FunctionEscape {
        // Collections that are `RETURN`ed: a resource flowing into one transfers
        // its scope-ownership to the caller (§15.6).
        let returned_collections: HashSet<String> = self
            .routings
            .iter()
            .filter(|routing| matches!(routing.target, Target::Returned))
            .flat_map(|routing| routing.src_collections.iter().cloned())
            .collect();
        // Propagate resource membership along collection-flow edges to a
        // fixpoint: `membership[c]` is the set of resources reachable from
        // collection binding `c`.
        let mut membership: HashMap<String, HashSet<String>> = HashMap::new();
        loop {
            let mut changed = false;
            for routing in &self.routings {
                // A `Target::Returned` routing contributes nothing to membership —
                // the caller-transfer decision below reads `returned_collections`,
                // computed once above — so only `Target::Var` edges propagate.
                let Target::Var(name) = &routing.target else {
                    continue;
                };
                let mut incoming: HashSet<String> = routing.res_elems.iter().cloned().collect();
                for source in &routing.src_collections {
                    if let Some(members) = membership.get(source) {
                        incoming.extend(members.iter().cloned());
                    }
                }
                let slot = membership.entry(name.clone()).or_default();
                for resource in incoming {
                    if slot.insert(resource) {
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }

        let mut owners = HashMap::new();
        for resource in &self.res_names {
            // A resource that flows into a returned collection still floats to
            // that collection's scope (so it is closed on error exits); the
            // `RETURN` of the collection transfers its owned-list to the caller
            // (§15.6).
            let res_depth = *self.res_depth.get(resource).unwrap_or(&0);
            let res_order = *self.decl_order.get(resource).unwrap_or(&0);
            // 1) A returned collection declared before the resource forces a
            //    float to it — even at the same scope depth — so the resource's
            //    close obligation rides the collection's owned-list and transfers
            //    to the caller on `RETURN`, instead of closing here.
            let mut best: Option<(usize, usize, String)> = None;
            for collection in &returned_collections {
                let Some(members) = membership.get(collection) else {
                    continue;
                };
                if !members.contains(resource) {
                    continue;
                }
                let order = *self.decl_order.get(collection).unwrap_or(&usize::MAX);
                if order >= res_order {
                    // The collection must be live before the resource so the
                    // owned-list exists when the resource is produced.
                    continue;
                }
                let depth = *self.decl_depth.get(collection).unwrap_or(&0);
                let candidate = (depth, order, collection.clone());
                best = match best {
                    Some(current) if (current.0, current.1) <= (depth, order) => Some(current),
                    _ => Some(candidate),
                };
            }
            // bug-291: remember whether phase 1 had a *candidate* it had to skip
            // purely because of declaration order -- that is the unsupportable
            // case, and it must not silently degrade to `Local`.
            let mut blocked_by_order: Option<String> = None;
            if best.is_none() {
                for collection in &returned_collections {
                    if !membership
                        .get(collection)
                        .is_some_and(|members| members.contains(resource))
                    {
                        continue;
                    }
                    // Only a RES-marked collection can own a resource at all. A
                    // bare `List OF File` is already rejected for the missing
                    // marker, and telling its author to reorder declarations
                    // would be advice that does not fix their program.
                    if !self
                        .decl_type
                        .get(collection)
                        .is_some_and(|type_| is_res_marked_resource_collection(type_))
                    {
                        continue;
                    }
                    blocked_by_order = Some(collection.clone());
                    break;
                }
            }
            // 2) Otherwise, float to the outermost strictly-outer collection.
            if best.is_none() {
                for (collection, members) in &membership {
                    if !members.contains(resource) {
                        continue;
                    }
                    let Some(&depth) = self.decl_depth.get(collection) else {
                        continue;
                    };
                    if depth >= res_depth {
                        // Same-or-inner scope: ownership does not float.
                        continue;
                    }
                    let order = *self.decl_order.get(collection).unwrap_or(&usize::MAX);
                    let candidate = (depth, order, collection.clone());
                    best = match best {
                        Some(current) if (current.0, current.1) <= (depth, order) => Some(current),
                        _ => Some(candidate),
                    };
                }
            }
            match best {
                Some((_, _, collection)) => {
                    owners.insert(resource.clone(), ResOwner::Float(collection));
                }
                // bug-291: phase 2 found no outer collection either. If phase 1 had
                // skipped a *returned* collection that genuinely holds this
                // resource, the program is the unsupportable ordering, not an
                // ordinary local: report it so verification can reject it.
                None => match blocked_by_order {
                    Some(collection) => {
                        owners.insert(resource.clone(), ResOwner::FloatBlocked(collection));
                    }
                    None => {
                        owners.insert(resource.clone(), ResOwner::Local);
                    }
                },
            }
        }

        FunctionEscape { owners }
    }
}

/// Does this declared type mark its element with the `RES` ownership axis, i.e.
/// can the collection actually take ownership of resources (§15.6)? Mirrors
/// `builder_codegen_primitives::is_res_marked_resource_collection`, which lives in
/// the target layer and is not reachable from here.
fn is_res_marked_resource_collection(type_: &str) -> bool {
    type_
        .strip_prefix("List OF ")
        .is_some_and(|element| element.starts_with("RES "))
        || type_
            .strip_prefix("Map OF ")
            .and_then(|rest| rest.split_once(" TO "))
            .is_some_and(|(_, value)| value.starts_with("RES "))
}

/// Collection-update builtins whose first argument is the collection being
/// updated and whose remaining arguments may insert resource elements.
fn is_insertion_builtin(callee: &str) -> bool {
    // The collection ops moved to `collections::` arrive qualified
    // (`collections.append`, ...); map back to the bare op so a freed bare name
    // in user code is never treated as a collection insertion
    // (plan-01-functions.md §5).
    matches!(
        crate::builtins::collections::native_member_bare(callee),
        Some("append" | "prepend" | "insert" | "set" | "mid" | "removeAt" | "filter" | "reduce")
    )
}

fn call_arg_expr(arg: &CallArg) -> &Expression {
    match arg {
        CallArg::Positional(expr) => expr,
        CallArg::Named { value, .. } => value,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Function, FunctionKind, Visibility};

    fn func(body: Vec<Statement>) -> Function {
        Function {
            kind: FunctionKind::Func,
            visibility: Visibility::Private,
            isolated: false,
            name: "f".to_string(),
            template_params: Vec::new(),
            params: Vec::new(),
            return_type: Some("Integer".to_string()),
            return_resource: false,
            return_state_type: None,
            body,
            trap: None,
            line: 1,
        }
    }

    fn res(name: &str, value: Expression) -> Statement {
        Statement::Let {
            mutable: false,
            resource: true,
            state_type: None,
            name: name.to_string(),
            type_name: Some("File".to_string()),
            value: Some(value),
            line: 1,
        }
    }

    fn list(name: &str, value: Expression) -> Statement {
        Statement::Let {
            mutable: true,
            resource: false,
            state_type: None,
            name: name.to_string(),
            type_name: Some("List OF File".to_string()),
            value: Some(value),
            line: 1,
        }
    }

    fn open() -> Expression {
        Expression::Call {
            callee: "fs.openFile".to_string(),
            arguments: vec![CallArg::Positional(Expression::String("p".to_string()))],
            line: 1,
            column: 1,
        }
    }

    fn append(collection: &str, element: &str) -> Expression {
        Expression::Call {
            callee: "collections.append".to_string(),
            arguments: vec![
                CallArg::Positional(Expression::Identifier(collection.to_string())),
                CallArg::Positional(Expression::Identifier(element.to_string())),
            ],
            line: 1,
            column: 1,
        }
    }

    fn ident(name: &str) -> Expression {
        Expression::Identifier(name.to_string())
    }

    #[test]
    fn same_scope_collection_does_not_float() {
        // RES f; LET xs = [f] — f and xs share a scope, so ownership stays local.
        let result = analyze_function(&func(vec![
            res("f", open()),
            list("xs", Expression::ListLiteral(vec![ident("f")])),
        ]));
        assert_eq!(result.owner("f"), ResOwner::Local);
        assert!(!result.floats("f"));
    }

    #[test]
    fn inner_resource_floats_to_outer_collection() {
        // MUT xs = []; WHILE { RES f; xs = append(xs, f) } — f floats to xs.
        let result = analyze_function(&func(vec![
            list("xs", Expression::ListLiteral(vec![])),
            Statement::While {
                kind: crate::ast::LoopKind::While,
                condition: Expression::Boolean(true),
                body: vec![
                    res("f", open()),
                    Statement::Assign {
                        name: "xs".to_string(),
                        value: append("xs", "f"),
                        line: 1,
                    },
                ],
                line: 1,
            },
        ]));
        assert_eq!(result.owner("f"), ResOwner::Float("xs".to_string()));
        assert!(result.floats("f"));
    }

    #[test]
    fn resource_in_returned_collection_floats_to_it() {
        // MUT xs = []; RES f; xs = append(xs, f); RETURN xs — f floats to xs even
        // at the same scope depth, because xs is declared first and is returned;
        // the `RETURN` transfers xs's owned-list to the caller (§15.6).
        let result = analyze_function(&func(vec![
            list("xs", Expression::ListLiteral(vec![])),
            res("f", open()),
            Statement::Assign {
                name: "xs".to_string(),
                value: append("xs", "f"),
                line: 1,
            },
            Statement::Return {
                value: Some(ident("xs")),
                line: 1,
            },
        ]));
        assert_eq!(result.owner("f"), ResOwner::Float("xs".to_string()));
        assert!(result.floats("f"));
    }

    #[test]
    fn bare_resource_return_does_not_float() {
        // RES f; RETURN f — a direct resource return is an ordinary move, not a
        // collection escape.
        let result = analyze_function(&func(vec![
            res("f", open()),
            Statement::Return {
                value: Some(ident("f")),
                line: 1,
            },
        ]));
        assert_eq!(result.owner("f"), ResOwner::Local);
    }

    #[test]
    fn float_follows_collection_copy_chain() {
        // Outer ys; inner { RES f; xs = [f]; ys = xs } — f reaches ys (outermost).
        let result = analyze_function(&func(vec![
            list("ys", Expression::ListLiteral(vec![])),
            Statement::While {
                kind: crate::ast::LoopKind::While,
                condition: Expression::Boolean(true),
                body: vec![
                    res("f", open()),
                    list("xs", Expression::ListLiteral(vec![ident("f")])),
                    Statement::Assign {
                        name: "ys".to_string(),
                        value: ident("xs"),
                        line: 1,
                    },
                ],
                line: 1,
            },
        ]));
        // ys is the outermost referencing collection.
        assert_eq!(result.owner("f"), ResOwner::Float("ys".to_string()));
    }
}
