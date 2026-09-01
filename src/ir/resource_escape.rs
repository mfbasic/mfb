//! Resource escape analysis (15_resource-management.md §15.6).
//!
//! A resource is owned by a *scope*. By default that is the scope where the
//! resource is produced. When a pointer to a `RES` binding is added to a
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
//! The analysis is purely syntactic over the HIR and depends only on which local
//! names are `RES` bindings, so the type checker and IR lowering compute the
//! same answer independently.
//!
//! **Soundness (re-founded by plan-59-E).** This used to rest on
//! `TYPE_RESOURCE_INVALIDATE_NOT_OWNER`: a non-owning resource pointer could not
//! escape a callee, so a resource only ever entered a collection inside the
//! function that owned it. That rule is retired — a resource is now owned by the
//! outermost scope that touches it, and any holder of the pointer may close,
//! `RETURN`, or transfer it. The syntactic scan is therefore no longer a complete
//! account of where a resource can go.
//!
//! What carries the guarantee instead is a layered argument, and the layers
//! matter individually:
//!
//! 1. **Ownership hand-off is still decided statically where it is syntactically
//!    visible.** `emit_return_exit` deactivates the cleanup for a returned `RES`
//!    binding, resource union, or `List OF RES` (`deactivate_resource_cleanup` /
//!    `deactivate_owned_list`), so the common case emits no close at all in the
//!    escaping scope.
//! 2. **Runtime pointer identity backstops the cases (1) cannot see**
//!    (plan-59-D): at scope exit a cleanup whose record pointer equals the value
//!    escaping the scope skips both close and reclaim.
//! 3. **A second close is a defined no-op, not corruption** (plan-59-B): every
//!    resource record carries a closed/moved flag at offset 8, and both the
//!    built-in helpers and every native `LINK` thunk test it before acting.
//!
//! So this pass no longer needs to prove that a resource cannot escape; it needs
//! only to compute where ownership *floats*, with (2) and (3) ensuring that a
//! resource which escapes by a route the scan does not model is still closed
//! exactly once rather than twice or never.

use crate::hir::{HirCallArg, HirConstructorArg, HirExpression, HirFunction, HirStatement};
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
    /// non-owning: it may not close, `RETURN`, or `thread::transfer`.
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

/// One "a collection value carrying resource pointers flows into `target`" fact.
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
    decl_type: HashMap<String, crate::types::ParameterType>,
    /// Record types that carry a `RES` field, so the bug-291 ordering gate can
    /// tell a container that can actually own a resource from one that cannot.
    /// See [`analyze_function_with`] for why a type name alone cannot answer it.
    res_field_records: HashSet<crate::types::ParameterType>,
    /// `RES` bindings that are PARAMETERS. The caller owns these; this function
    /// never produces them, so the bug-291 ordering rule does not apply — there
    /// is no production point for a container's owned-list to be missing at.
    res_params: HashSet<String>,
    res_depth: HashMap<String, usize>,
    routings: Vec<Routing>,
    next_order: usize,
}

/// Analyze a function body, returning per-`RES`-binding ownership decisions.
///
/// Equivalent to [`analyze_function_with`] with no record-type knowledge, i.e.
/// the collection rules only.
///
/// `#[cfg(test)]`: **production always has the type table** (`ir::lower` passes
/// `TypeIndex::res_field_record_types`), and a caller that silently dropped it
/// would lose the record half of the bug-291 ordering gate — the failure mode
/// being a double close with no diagnostic. Keeping this convenience entry point
/// out of non-test builds means that mistake cannot be made by accident; the
/// tests that predate records use it because for them the empty set is the
/// truth, not a shortcut.
#[cfg(test)]
pub fn analyze_function(function: &HirFunction) -> FunctionEscape {
    analyze_function_with(function, &HashSet::new())
}

/// [`analyze_function`], plus the set of record types that carry a `RES` field.
///
/// plan-114-C: the bug-291 ordering gate skips a returned container that cannot
/// actually own a resource, so that it does not pile a `FloatBlocked` rejection
/// onto a program already refused for a missing `RES` marker. For a collection
/// that test is structural — `is_res_marked_resource_collection` reads
/// `List OF RES T` straight off the type. For a **record** it is not:
/// `Named("Holder")` does not reveal whether `Holder` has a `RES` field, so
/// without this set the gate would fall through to `ResOwner::Local` and
/// silently reproduce the exact bug-291 miscompile (the callee closes the
/// handle, then the caller's adopted list closes it again).
///
/// Empty is the safe default: it means "no record can own a resource", which is
/// what the `#[cfg(test)]` callers and the pre-record world both assume.
pub fn analyze_function_with(
    function: &HirFunction,
    res_field_records: &HashSet<crate::types::ParameterType>,
) -> FunctionEscape {
    let mut analyzer = Analyzer {
        res_names: HashSet::new(),
        decl_depth: HashMap::new(),
        decl_order: HashMap::new(),
        decl_type: HashMap::new(),
        res_field_records: res_field_records.clone(),
        res_params: HashSet::new(),
        res_depth: HashMap::new(),
        routings: Vec::new(),
        next_order: 0,
    };

    // `RES` parameters are resources owned at function-entry depth.
    for param in &function.params {
        if param.resource {
            analyzer.declare(&param.name, 0);
            analyzer.res_names.insert(param.name.clone());
            analyzer.res_params.insert(param.name.clone());
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

    fn walk(&mut self, body: &[HirStatement], depth: usize) {
        for statement in body {
            self.walk_statement(statement, depth);
        }
    }

    fn walk_statement(&mut self, statement: &HirStatement, depth: usize) {
        match statement {
            HirStatement::Let {
                resource,
                name,
                type_,
                explicit_type,
                value,
                ..
            } => {
                self.declare(name, depth);
                // Mirror the AST rule: only an explicit `AS T` annotation records a
                // declared type (an inferred binding carried `None` there).
                if *explicit_type {
                    self.decl_type
                        .entry(name.clone())
                        .or_insert_with(|| type_.clone());
                } else if let Some(HirExpression::Constructor { type_, .. }) = value {
                    // plan-114-C: a record CONSTRUCTOR names its own type, so an
                    // inferred binding still has a knowable one. Recording it is
                    // load-bearing, not tidiness — the bug-291 ordering gate reads
                    // `decl_type` to decide whether a returned container can own a
                    // resource at all, and with no entry it answers "no" and
                    // degrades to `ResOwner::Local`.
                    //
                    // That degradation is the silent double close bug-291 exists to
                    // prevent. Measured before the fix:
                    //
                    //   FUNC makeHolder(p AS String) AS Holder
                    //     RES f AS fs::File = fs::openFile(p, "w")
                    //     LET h = Holder["made", f]     ' inferred -> no decl_type
                    //     RETURN h
                    //   END FUNC
                    //
                    // compiled, and the caller's first write raised
                    // `7-703-0004 Resource handle is already closed` — the callee
                    // closed the handle it had just handed over.
                    //
                    // A collection literal is deliberately NOT covered here: a
                    // `List`/`Map` literal does not name its element type, so there
                    // is nothing to record, and the explicit-annotation rule is
                    // what those bindings have always relied on.
                    self.decl_type
                        .entry(name.clone())
                        .or_insert_with(|| type_.clone());
                }
                if *resource {
                    self.res_names.insert(name.clone());
                    self.res_depth.insert(name.clone(), depth);
                }
                if let Some(value) = value {
                    self.record_routing(Target::Var(name.clone()), value);
                }
            }
            HirStatement::Assign { name, value, .. } => {
                self.declare(name, depth);
                self.record_routing(Target::Var(name.clone()), value);
            }
            HirStatement::Return {
                value: Some(value), ..
            } => {
                self.record_routing(Target::Returned, value);
            }
            HirStatement::If {
                then_body,
                else_body,
                ..
            } => {
                self.walk(then_body, depth + 1);
                self.walk(else_body, depth + 1);
            }
            HirStatement::Match { cases, .. } => {
                for case in cases {
                    self.walk(&case.body, depth + 1);
                }
            }
            HirStatement::For { body, .. }
            | HirStatement::ForEach { body, .. }
            | HirStatement::While { body, .. }
            | HirStatement::DoUntil { body, .. } => {
                self.walk(body, depth + 1);
            }
            _ => {}
        }
    }

    fn record_routing(&mut self, target: Target, expr: &HirExpression) {
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
        expr: &HirExpression,
        res_elems: &mut Vec<String>,
        src_collections: &mut Vec<String>,
    ) {
        match expr {
            HirExpression::Identifier(name) => {
                // A bare resource identifier in value position is not a
                // collection (e.g. `RETURN f`, `LET g = f`); it only escapes when
                // it appears as a collection *element* (see `scan_element`). A
                // non-resource identifier is a plain collection copy `V = C`.
                if !self.res_names.contains(name) {
                    src_collections.push(name.clone());
                }
            }
            HirExpression::ListLiteral(values) => {
                for value in values {
                    self.scan_element(value, res_elems, src_collections);
                }
            }
            HirExpression::MapLiteral { entries, .. } => {
                for (_, value) in entries {
                    self.scan_element(value, res_elems, src_collections);
                }
            }
            // plan-114-C: a record is a container in exactly the sense this scan
            // means — a value that holds handle pointers and whose binding has a
            // scope. `Holder[handle := f]` routes `f` into the constructed
            // record's binding just as `[f]` routes it into a list's, so a
            // constructor argument is an element position.
            HirExpression::Constructor { arguments, .. } => {
                for argument in arguments {
                    let value = match argument {
                        HirConstructorArg::Positional(value) => value,
                        HirConstructorArg::Named { value, .. } => value,
                    };
                    self.scan_element(value, res_elems, src_collections);
                }
            }
            // `WITH v { field := expr }` produces a NEW record carrying both the
            // updated value and everything `v` already held, so both flow into
            // the result — `target` exactly as insertion argument 0 does, and
            // each update as an element. Records have no field assignment, so
            // this is the only mutation-shaped edge there is (§4.2).
            HirExpression::WithUpdate { target, updates } => {
                self.scan_collection_expr(target, res_elems, src_collections);
                for update in updates {
                    self.scan_element(&update.value, res_elems, src_collections);
                }
            }
            HirExpression::Call {
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
            HirExpression::Trapped {
                expression,
                handler,
                ..
            } => {
                self.scan_collection_expr(expression, res_elems, src_collections);
                for statement in handler {
                    if let HirStatement::Recover {
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
        expr: &HirExpression,
        res_elems: &mut Vec<String>,
        src_collections: &mut Vec<String>,
    ) {
        if let HirExpression::Identifier(name) = expr {
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
            // plan-114-C: a `RES` PARAMETER is never blocked by ordering. The
            // rule's hazard is that the container's owned-list does not exist yet
            // *when the resource is produced* — and a parameter is not produced in
            // this function at all. The caller owns it and closes it; returning a
            // container that carries it transfers nothing.
            //
            // Without this, `FUNC wrap(RES f AS fs::File) AS Holder` with
            // `LET h = Holder[…, f]; RETURN h` is rejected, and it is a correct
            // program: measured working (the caller writes through the returned
            // record after `wrap` returns, and the handle is still open).
            if best.is_none() && !self.res_params.contains(resource) {
                for collection in &returned_collections {
                    if !membership
                        .get(collection)
                        .is_some_and(|members| members.contains(resource))
                    {
                        continue;
                    }
                    // Only a container that can actually own a resource counts. A
                    // bare `List OF File` is already rejected for the missing
                    // marker, and telling its author to reorder declarations
                    // would be advice that does not fix their program.
                    //
                    // plan-114-C: a record qualifies the same way, but its type
                    // name does not reveal whether it has a `RES` field, so the
                    // record case is decided by the table threaded in through
                    // `analyze_function_with`. Without it the gate would fall
                    // through to `Local` and silently reproduce bug-291.
                    if !self.decl_type.get(collection).is_some_and(|type_| {
                        is_res_marked_resource_collection(type_)
                            || self.res_field_records.contains(type_)
                    }) {
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
fn is_res_marked_resource_collection(type_: &crate::types::ParameterType) -> bool {
    use crate::types::ParameterType;
    match type_ {
        ParameterType::ListOf(element) => matches!(element.as_ref(), ParameterType::Res(_)),
        ParameterType::MapOf(_, value) => matches!(value.as_ref(), ParameterType::Res(_)),
        _ => false,
    }
}

/// Collection-update builtins whose first argument is the collection being
/// updated and whose remaining arguments may insert resource elements.
fn is_insertion_builtin(callee: &str) -> bool {
    // The collection ops moved to `collections::` arrive qualified
    // (`collections.append`, ...); map back to the bare op so a freed bare name
    // in user code is never treated as a collection insertion
    // (plan-01-functions.md §5).
    // The bare native member name for a `collections.<member>` call (the collections
    // guard excludes the shared `strings.{find,mid,replace}` overloads that
    // `native_builtin_target` also matches).
    let member = callee
        .starts_with("collections.")
        .then(|| crate::codegen::builtins::native_builtin_target(callee))
        .flatten();
    matches!(
        member,
        Some("append" | "prepend" | "insert" | "set" | "mid" | "removeAt" | "filter" | "reduce")
    )
}

fn call_arg_expr(arg: &HirCallArg) -> &HirExpression {
    match arg {
        HirCallArg::Positional(expr) => expr,
        HirCallArg::Named { value, .. } => value,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{FunctionKind, Visibility};
    use crate::types::ParameterType;

    fn func(body: Vec<HirStatement>) -> HirFunction {
        HirFunction {
            kind: FunctionKind::Func,
            visibility: Visibility::Private,
            isolated: false,
            name: "f".to_string(),
            template_params: Vec::new(),
            params: Vec::new(),
            returns: ParameterType::parse("Integer"),
            return_resource: false,
            return_state_type: None,
            body,
            trap: None,
            line: 1,
        }
    }

    fn res(name: &str, value: HirExpression) -> HirStatement {
        HirStatement::Let {
            mutable: false,
            resource: true,
            state_type: None,
            name: name.to_string(),
            type_: ParameterType::parse("fs.File"),
            explicit_type: true,
            value: Some(value),
            line: 1,
        }
    }

    fn list(name: &str, value: HirExpression) -> HirStatement {
        HirStatement::Let {
            mutable: true,
            resource: false,
            state_type: None,
            name: name.to_string(),
            type_: ParameterType::parse("List OF File"),
            explicit_type: true,
            value: Some(value),
            line: 1,
        }
    }

    fn open() -> HirExpression {
        HirExpression::Call {
            callee: "fs.openFile".to_string(),
            arguments: vec![HirCallArg::Positional(HirExpression::String(
                "p".to_string(),
            ))],
            line: 1,
            column: 1,
        }
    }

    fn append(collection: &str, element: &str) -> HirExpression {
        HirExpression::Call {
            callee: "collections.append".to_string(),
            arguments: vec![
                HirCallArg::Positional(HirExpression::Identifier(collection.to_string())),
                HirCallArg::Positional(HirExpression::Identifier(element.to_string())),
            ],
            line: 1,
            column: 1,
        }
    }

    fn ident(name: &str) -> HirExpression {
        HirExpression::Identifier(name.to_string())
    }

    // -----------------------------------------------------------------------
    // plan-114-C Phase 2 — the record edges.
    //
    // A record is a container in the same sense a collection is: a value that
    // holds handle pointers and whose binding has a scope. These mirror the
    // collection cases above one for one, so a divergence between the two
    // container kinds shows up as a test that passes for lists and fails for
    // records.
    // -----------------------------------------------------------------------

    /// A record binding, `MUT`/`LET` per `mutable`.
    fn holder(name: &str, mutable: bool, value: HirExpression) -> HirStatement {
        HirStatement::Let {
            mutable,
            resource: false,
            state_type: None,
            name: name.to_string(),
            type_: ParameterType::declared("Holder"),
            explicit_type: true,
            value: Some(value),
            line: 1,
        }
    }

    /// `Holder[handle := <element>]`
    fn construct(element: &str) -> HirExpression {
        HirExpression::Constructor {
            type_: ParameterType::declared("Holder"),
            arguments: vec![HirConstructorArg::Named {
                name: "handle".to_string(),
                value: ident(element),
                line: 1,
            }],
        }
    }

    /// `WITH <target> { handle := <element> }`
    fn with_update(target: &str, element: &str) -> HirExpression {
        HirExpression::WithUpdate {
            target: Box::new(ident(target)),
            updates: vec![crate::hir::HirRecordUpdate {
                field: "handle".to_string(),
                value: ident(element),
                line: 1,
            }],
        }
    }

    #[test]
    fn inner_resource_floats_to_an_outer_record_via_with() {
        // MUT h = Holder[...]; WHILE { RES f; h = WITH h { handle := f } }
        // The record twin of `inner_resource_floats_to_outer_collection`.
        let result = analyze_function(&func(vec![
            holder("h", true, construct("nothing")),
            HirStatement::While {
                kind: crate::ast::LoopKind::While,
                condition: HirExpression::Boolean(true),
                body: vec![
                    res("f", open()),
                    HirStatement::Assign {
                        name: "h".to_string(),
                        value: with_update("h", "f"),
                        line: 1,
                    },
                ],
                line: 1,
            },
        ]));
        assert_eq!(result.owner("f"), ResOwner::Float("h".to_string()));
        assert!(result.floats("f"));
    }

    #[test]
    fn same_scope_record_does_not_float() {
        // RES f; LET h = Holder[handle := f] — same scope, so ownership stays
        // local, exactly as for a list literal at the same depth.
        let result = analyze_function(&func(vec![
            res("f", open()),
            holder("h", false, construct("f")),
        ]));
        assert_eq!(result.owner("f"), ResOwner::Local);
        assert!(!result.floats("f"));
    }

    #[test]
    fn a_constructor_nested_in_a_list_routes_to_the_list() {
        // MUT xs = []; WHILE { RES f; xs = append(xs, Holder[handle := f]) }
        // No extra scan arm is needed for this: `scan_element` falls through to
        // `scan_collection_expr`, which now has a Constructor arm, so the
        // resource reaches the LIST binding (the outermost container).
        let result = analyze_function(&func(vec![
            list("xs", HirExpression::ListLiteral(vec![])),
            HirStatement::While {
                kind: crate::ast::LoopKind::While,
                condition: HirExpression::Boolean(true),
                body: vec![
                    res("f", open()),
                    HirStatement::Assign {
                        name: "xs".to_string(),
                        value: HirExpression::Call {
                            callee: "collections.append".to_string(),
                            arguments: vec![
                                HirCallArg::Positional(ident("xs")),
                                HirCallArg::Positional(construct("f")),
                            ],
                            line: 1,
                            column: 1,
                        },
                        line: 1,
                    },
                ],
                line: 1,
            },
        ]));
        assert_eq!(result.owner("f"), ResOwner::Float("xs".to_string()));
    }

    #[test]
    fn a_record_with_no_resource_argument_routes_nothing() {
        // The regression guard: an ordinary record must not acquire an
        // owned-list just because the Constructor arm now exists.
        let result = analyze_function(&func(vec![
            res("f", open()),
            holder("h", false, construct("unrelated")),
        ]));
        assert_eq!(result.owner("f"), ResOwner::Local);
        assert!(!result.floats("f"));
    }

    #[test]
    fn a_positional_constructor_argument_routes_the_same_as_a_named_one() {
        // `Holder[f]` and `Holder[handle := f]` are the same edge; the scan must
        // not see only the by-field spelling (§4.1 reads both arms).
        let result = analyze_function(&func(vec![
            list("xs", HirExpression::ListLiteral(vec![])),
            HirStatement::While {
                kind: crate::ast::LoopKind::While,
                condition: HirExpression::Boolean(true),
                body: vec![
                    res("f", open()),
                    HirStatement::Assign {
                        name: "xs".to_string(),
                        value: HirExpression::Call {
                            callee: "collections.append".to_string(),
                            arguments: vec![
                                HirCallArg::Positional(ident("xs")),
                                HirCallArg::Positional(HirExpression::Constructor {
                                    type_: ParameterType::declared("Holder"),
                                    arguments: vec![HirConstructorArg::Positional(ident("f"))],
                                }),
                            ],
                            line: 1,
                            column: 1,
                        },
                        line: 1,
                    },
                ],
                line: 1,
            },
        ]));
        assert_eq!(result.owner("f"), ResOwner::Float("xs".to_string()));
    }

    #[test]
    fn with_update_carries_the_targets_existing_contents() {
        // `h2 = WITH h1 { … }` must route h1's contents into h2 the way
        // insertion argument 0 does, or a resource already held by h1 would be
        // lost when the updated copy outlives it.
        let result = analyze_function(&func(vec![
            holder("outer", true, construct("nothing")),
            HirStatement::While {
                kind: crate::ast::LoopKind::While,
                condition: HirExpression::Boolean(true),
                body: vec![
                    res("f", open()),
                    holder("inner", false, construct("f")),
                    HirStatement::Assign {
                        name: "outer".to_string(),
                        value: with_update("inner", "f"),
                        line: 1,
                    },
                ],
                line: 1,
            },
        ]));
        // `f` reaches `outer`, the outermost container that references it.
        assert_eq!(result.owner("f"), ResOwner::Float("outer".to_string()));
    }

    /// bug-291's record twin: a RETURNED record declared *after* the resource it
    /// carries cannot honour the float (its owned-list does not exist yet when
    /// the resource is produced), so it must be reported rather than degraded.
    ///
    /// This is the case that needs the type table. Degrading it to `Local` is a
    /// silent miscompile — the callee closes the handle at its own scope while
    /// the returned record still carries it, and the caller's adopted list
    /// closes it a second time.

    /// plan-114-C, found by running the returned-record shape rather than
    /// reasoning about it: an **inferred** record binding had no `decl_type`, so
    /// the bug-291 ordering gate could not see the container and degraded to
    /// `ResOwner::Local`. That is the silent double close the gate exists to
    /// prevent -- measured before the fix as a caller write raising
    /// `7-703-0004 Resource handle is already closed`.
    ///
    /// A record CONSTRUCTOR names its own type, so an inferred binding still has
    /// a knowable one; recording it from the initializer is what makes the gate
    /// fire.
    #[test]
    fn an_inferred_record_binding_is_still_seen_by_the_ordering_gate() {
        let mut records = HashSet::new();
        records.insert(ParameterType::declared("Holder"));
        // `RES f = …; LET h = Holder[handle := f]; RETURN h` -- note `h` carries
        // NO `AS Holder` annotation, which is the whole point.
        let inferred = HirStatement::Let {
            mutable: false,
            resource: false,
            state_type: None,
            name: "h".to_string(),
            type_: ParameterType::declared("Holder"),
            explicit_type: false,
            value: Some(construct("f")),
            line: 1,
        };
        let result = analyze_function_with(
            &func(vec![
                res("f", open()),
                inferred,
                HirStatement::Return {
                    value: Some(ident("h")),
                    line: 1,
                },
            ]),
            &records,
        );
        assert_eq!(
            result.owner("f"),
            ResOwner::FloatBlocked("h".to_string()),
            "an inferred `LET h = Holder[…]` must still be recognised as the \
             container; degrading to Local is a double close"
        );
    }

    /// The other half of the same fix, and the guard against over-rejecting it:
    /// a `RES` **parameter** is never blocked by ordering. The rule's hazard is
    /// that the container's owned-list does not exist yet *when the resource is
    /// produced* -- and a parameter is not produced here at all. The caller owns
    /// and closes it.
    ///
    /// Measured working end to end: `FUNC wrap(RES f AS fs::File) AS Holder`
    /// returning `Holder[…, f]`, with the caller writing through the returned
    /// record's handle after `wrap` returns.
    #[test]
    fn a_res_parameter_is_never_blocked_by_return_order() {
        let mut records = HashSet::new();
        records.insert(ParameterType::declared("Holder"));
        let mut f = func(vec![
            HirStatement::Let {
                mutable: false,
                resource: false,
                state_type: None,
                name: "h".to_string(),
                type_: ParameterType::declared("Holder"),
                explicit_type: false,
                value: Some(construct("p")),
                line: 1,
            },
            HirStatement::Return {
                value: Some(ident("h")),
                line: 1,
            },
        ]);
        f.params = vec![crate::hir::HirParam {
            name: "p".to_string(),
            type_: ParameterType::parse("fs.File"),
            resource: true,
            state_type: None,
            default: None,
            line: 1,
        }];
        let result = analyze_function_with(&f, &records);
        assert_ne!(
            result.owner("p"),
            ResOwner::FloatBlocked("h".to_string()),
            "a RES parameter is owned by the caller; returning a record that \
             carries it transfers nothing and must not be refused"
        );
    }

    #[test]
    fn a_returned_record_declared_after_its_resource_is_blocked() {
        let body = vec![
            res("f", open()),
            holder("h", false, construct("f")),
            HirStatement::Return {
                value: Some(ident("h")),
                line: 1,
            },
        ];
        let mut records = HashSet::new();
        records.insert(ParameterType::declared("Holder"));

        let blocked = analyze_function_with(&func(body.clone()), &records);
        assert_eq!(
            blocked.owner("f"),
            ResOwner::FloatBlocked("h".to_string()),
            "a returned record declared after its resource must be reported, \
             not degraded to Local"
        );

        // And the reason the table is needed at all: with no record-type
        // knowledge the very same program degrades to `Local`, which is the
        // bug-291 double close. This half is what makes the test prove the
        // threading, not just the gate.
        let unaware = analyze_function_with(&func(body), &HashSet::new());
        assert_eq!(
            unaware.owner("f"),
            ResOwner::Local,
            "without the record table the gate cannot see the container — this \
             is why ir::lower must pass TypeIndex::res_field_record_types"
        );
    }

    /// The guard on the guard: a returned record with NO `RES` field must not be
    /// blocked. Reporting an ordering problem for a container that cannot own a
    /// resource would be advice that does not fix the program — the same reason
    /// a bare `List OF File` is skipped.
    #[test]
    fn a_returned_record_without_a_res_field_is_not_blocked() {
        let result = analyze_function_with(
            &func(vec![
                res("f", open()),
                holder("h", false, construct("unrelated")),
                HirStatement::Return {
                    value: Some(ident("h")),
                    line: 1,
                },
            ]),
            &HashSet::new(),
        );
        assert_eq!(result.owner("f"), ResOwner::Local);
    }

    /// A record declared BEFORE its resource and returned floats normally — the
    /// ordering gate must not fire just because a record is involved.
    #[test]
    fn a_returned_record_declared_before_its_resource_floats() {
        let mut records = HashSet::new();
        records.insert(ParameterType::declared("Holder"));
        let result = analyze_function_with(
            &func(vec![
                holder("h", true, construct("nothing")),
                res("f", open()),
                HirStatement::Assign {
                    name: "h".to_string(),
                    value: with_update("h", "f"),
                    line: 1,
                },
                HirStatement::Return {
                    value: Some(ident("h")),
                    line: 1,
                },
            ]),
            &records,
        );
        assert_eq!(result.owner("f"), ResOwner::Float("h".to_string()));
    }

    #[test]
    fn same_scope_collection_does_not_float() {
        // RES f; LET xs = [f] — f and xs share a scope, so ownership stays local.
        let result = analyze_function(&func(vec![
            res("f", open()),
            list("xs", HirExpression::ListLiteral(vec![ident("f")])),
        ]));
        assert_eq!(result.owner("f"), ResOwner::Local);
        assert!(!result.floats("f"));
    }

    #[test]
    fn inner_resource_floats_to_outer_collection() {
        // MUT xs = []; WHILE { RES f; xs = append(xs, f) } — f floats to xs.
        let result = analyze_function(&func(vec![
            list("xs", HirExpression::ListLiteral(vec![])),
            HirStatement::While {
                kind: crate::ast::LoopKind::While,
                condition: HirExpression::Boolean(true),
                body: vec![
                    res("f", open()),
                    HirStatement::Assign {
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
            list("xs", HirExpression::ListLiteral(vec![])),
            res("f", open()),
            HirStatement::Assign {
                name: "xs".to_string(),
                value: append("xs", "f"),
                line: 1,
            },
            HirStatement::Return {
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
            HirStatement::Return {
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
            list("ys", HirExpression::ListLiteral(vec![])),
            HirStatement::While {
                kind: crate::ast::LoopKind::While,
                condition: HirExpression::Boolean(true),
                body: vec![
                    res("f", open()),
                    list("xs", HirExpression::ListLiteral(vec![ident("f")])),
                    HirStatement::Assign {
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
