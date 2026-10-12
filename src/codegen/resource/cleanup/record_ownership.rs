//! The two per-function facts the resource lowering reads before it starts.
//!
//! Ownership itself is not decided here. A resource is closed, and its record
//! freed, by the scope whose token its owner word holds; a value made of
//! references is plain data its one owner frees (`mfb spec language
//! resource-management`, "The owner of a resource"). Nothing in this module
//! proves a holder, and nothing downstream waits for such a proof.
//!
//! What is computed:
//!
//! * [`RecordOwnership::alias_sources`] — the bare-local alias chains a `RETURN`
//!   follows to the binding that owns a resource union's box.
//! * [`StoreFloats`] — the assignments that can put a resource owned INSIDE the
//!   assigned container's scope into that container, so the store must raise
//!   what it stores (rule 4). This is a statement about scopes, read off the
//!   declarations: a value built only from bindings declared at the container's
//!   scope or further out reaches nothing owned inside it, and needs no code.

use crate::target::shared::nir::visit::{walk_op, walk_value, NirVisitor};
use crate::target::shared::nir::{NirFunction, NirOp, NirValue};
use crate::types::ParameterType;
use std::collections::{HashMap, HashSet};

/// The facts one function's lowering consults.
pub(crate) struct RecordOwnership {
    /// Locals whose single store is a bare `Local(src)` — the alias chains a `RETURN`
    /// follows to the binding that actually owns a union's box.
    pub(crate) alias_sources: HashMap<String, String>,
    /// The assignments that must raise what they store — see [`StoreFloats`].
    pub(crate) store_floats: StoreFloats,
}

/// The assignments whose stored value can reach a resource owned by a scope
/// inside the assigned container's.
///
/// `mfb spec language resource-management` 15.6: adding a reference to a resource
/// to a container raises the resource's owning scope to the container's when that
/// scope outlives the current owner. The raise is done at the store, by the owner
/// word (`CodeBuilder::emit_stored_item_float`,
/// `CodeBuilder::emit_assigned_container_float`): every resource the stored value
/// reaches is pushed, adopting, onto the container's owned-list, and the push
/// takes a record only when its owner is below the container's token.
///
/// The code is emitted only where it can do something. A binding's value reaches
/// nothing owned inside the binding's own scope (its bind and every store into
/// it raised what it took), so a stored value is asked for the deepest scope
/// anything in it can be owned by:
///
/// * a local: the scope it was declared in (a parameter and a module-level
///   binding: outside every scope of this body);
/// * a `FOR EACH` variable: whatever its iterable can reach;
/// * a call to a function body, and a function value (a callback a builtin
///   runs): the statement itself, one scope inside the current one, because a
///   `RETURN` hands the statement what it releases (rule 5).
///
/// When that is deeper than the container's declaration, the assignment is listed.
#[derive(Default, Clone)]
pub(crate) struct StoreFloats {
    /// The assigned value nodes (by address) of the listed assignments.
    pub(crate) stores: HashSet<usize>,
    /// The containers such an assignment stores into: each needs its owned-lists
    /// from its bind, like a float target.
    pub(crate) containers: HashSet<String>,
}

/// The facts for `function`. `container` answers whether a binding of that type
/// reaches a resource without being one (a collection or a record).
pub(crate) fn record_ownership(
    function: &NirFunction,
    functions: &HashMap<String, &NirFunction>,
    container: &dyn Fn(&ParameterType) -> bool,
) -> RecordOwnership {
    RecordOwnership {
        alias_sources: alias_sources(function),
        store_floats: collect_store_floats(function, functions, container),
    }
}

/// The locals stored exactly once, by a bare `Local(src)`.
fn alias_sources(function: &NirFunction) -> HashMap<String, String> {
    #[derive(Default)]
    struct Stores {
        /// Per local: how often it is stored, and its last bare-local source.
        seen: HashMap<String, (usize, Option<String>)>,
    }
    impl Stores {
        fn note(&mut self, name: &str, value: Option<&NirValue>) {
            let entry = self.seen.entry(name.to_string()).or_default();
            entry.0 += 1;
            entry.1 = match value {
                Some(NirValue::Local(source)) => Some(source.clone()),
                _ => None,
            };
        }
    }
    impl NirVisitor for Stores {
        fn visit_op(&mut self, op: &NirOp) {
            match op {
                NirOp::Bind { name, value, .. } => self.note(name, value.as_ref()),
                NirOp::Assign { name, value } => self.note(name, Some(value)),
                _ => {}
            }
            walk_op(self, op);
        }
    }
    let mut stores = Stores::default();
    stores.visit_ops(&function.body);
    stores
        .seen
        .into_iter()
        .filter_map(|(name, (count, source))| match (count, source) {
            (1, Some(source)) => Some((name, source)),
            _ => None,
        })
        .collect()
}

/// Find the assignments [`StoreFloats`] describes.
fn collect_store_floats(
    function: &NirFunction,
    functions: &HashMap<String, &NirFunction>,
    container: &dyn Fn(&ParameterType) -> bool,
) -> StoreFloats {
    struct Scan<'p> {
        functions: &'p HashMap<String, &'p NirFunction>,
        container: &'p dyn Fn(&ParameterType) -> bool,
        depth: usize,
        /// The bindings in scope: the deepest scope that can own what each
        /// reaches, and its type.
        bound: HashMap<String, (usize, ParameterType)>,
        scopes: Vec<Vec<String>>,
        out: StoreFloats,
    }
    /// The deepest scope anything `value` reaches can be owned by, not counting
    /// what `skip` (the assigned container itself) already holds.
    struct Reach<'s, 'p> {
        scan: &'s Scan<'p>,
        skip: Option<&'s str>,
        depth: usize,
    }
    impl NirVisitor for Reach<'_, '_> {
        fn visit_value(&mut self, value: &NirValue) {
            let reached = match value {
                NirValue::Local(name) if self.skip != Some(name.as_str()) => {
                    self.scan.bound.get(name).map(|(depth, _)| *depth)
                }
                NirValue::Call { target, .. } | NirValue::CallResult { target, .. }
                    if self.scan.functions.contains_key(target)
                        || self.scan.bound.contains_key(target) =>
                {
                    Some(self.scan.depth + 1)
                }
                NirValue::Closure { .. } | NirValue::FunctionRef { .. } => {
                    Some(self.scan.depth + 1)
                }
                _ => None,
            };
            self.depth = self.depth.max(reached.unwrap_or(0));
            walk_value(self, value);
        }
    }
    impl Scan<'_> {
        fn reach(&self, value: &NirValue, skip: Option<&str>) -> usize {
            let mut reach = Reach {
                scan: self,
                skip,
                depth: 0,
            };
            reach.visit_value(value);
            reach.depth
        }

        fn declare(&mut self, name: &str, depth: usize, type_: &ParameterType) {
            self.bound.insert(name.to_string(), (depth, type_.clone()));
            if let Some(scope) = self.scopes.last_mut() {
                scope.push(name.to_string());
            }
        }
    }
    impl NirVisitor for Scan<'_> {
        fn visit_ops(&mut self, ops: &[NirOp]) {
            self.depth += 1;
            self.scopes.push(Vec::new());
            for op in ops {
                self.visit_op(op);
            }
            for name in self.scopes.pop().unwrap_or_default() {
                self.bound.remove(&name);
            }
            self.depth -= 1;
        }

        fn visit_op(&mut self, op: &NirOp) {
            match op {
                NirOp::Bind { name, type_, .. } => {
                    let depth = self.depth;
                    self.declare(name, depth, type_);
                }
                NirOp::Assign { name, value } => {
                    let target = self
                        .bound
                        .get(name)
                        .filter(|(_, type_)| (self.container)(type_))
                        .map(|(depth, _)| *depth);
                    if let Some(target_depth) = target {
                        if self.reach(value, Some(name)) > target_depth {
                            self.out.stores.insert(value as *const NirValue as usize);
                            self.out.containers.insert(name.clone());
                        }
                    }
                }
                NirOp::ForEach {
                    name,
                    type_,
                    iterable,
                    body,
                } => {
                    let reach = self.reach(iterable, None);
                    let shadowed = self.bound.insert(name.clone(), (reach, type_.clone()));
                    self.visit_ops(body);
                    match shadowed {
                        Some(previous) => self.bound.insert(name.clone(), previous),
                        None => self.bound.remove(name),
                    };
                    return;
                }
                _ => {}
            }
            walk_op(self, op);
        }
    }
    let mut scan = Scan {
        functions,
        container,
        depth: 0,
        bound: HashMap::new(),
        scopes: Vec::new(),
        out: StoreFloats::default(),
    };
    scan.visit_ops(&function.body);
    scan.out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::target::shared::nir::NirSourceLoc;

    fn function(body: Vec<NirOp>) -> NirFunction {
        NirFunction {
            name: "f".to_string(),
            visibility: String::new(),
            kind: "function".to_string(),
            isolated: false,
            params: Vec::new(),
            returns: ParameterType::Integer,
            body,
            file: String::new(),
            resource_owners: HashMap::new(),
        }
    }

    fn list_type() -> ParameterType {
        ParameterType::parse("List OF RES fs.File")
    }

    fn bind(name: &str, value: NirValue) -> NirOp {
        NirOp::Bind {
            mutable: true,
            name: name.to_string(),
            type_: list_type(),
            value: Some(value),
        }
    }

    fn local(name: &str) -> NirValue {
        NirValue::Local(name.to_string())
    }

    fn call(target: &str, args: Vec<NirValue>) -> NirValue {
        NirValue::Call {
            target: target.to_string(),
            args,
            loc: NirSourceLoc::default(),
        }
    }

    fn empty() -> NirValue {
        NirValue::ListLiteral {
            type_: list_type(),
            values: Vec::new(),
        }
    }

    fn floats(body: Vec<NirOp>, functions: &HashMap<String, &NirFunction>) -> StoreFloats {
        collect_store_floats(&function(body), functions, &|type_| {
            matches!(type_, ParameterType::ListOf(_))
        })
    }

    /// A value from a scope inside the container's is raised at the store; one
    /// from the container's own scope, or a parameter, is not.
    #[test]
    fn only_a_store_out_of_an_inner_scope_floats() {
        let inner_store = NirOp::Assign {
            name: "keep".to_string(),
            value: call("collections.append", vec![local("keep"), local("xs")]),
        };
        let body = vec![
            bind("keep", empty()),
            bind("same", empty()),
            NirOp::Assign {
                name: "keep".to_string(),
                value: call("collections.append", vec![local("keep"), local("same")]),
            },
            NirOp::Assign {
                name: "keep".to_string(),
                value: call("collections.append", vec![local("keep"), local("param")]),
            },
            NirOp::While {
                kind: crate::ast::LoopKind::While,
                condition: NirValue::Const {
                    type_: ParameterType::Boolean,
                    value: "true".to_string(),
                },
                body: vec![bind("xs", empty()), inner_store],
            },
        ];
        let found = floats(body, &HashMap::new());
        assert_eq!(found.stores.len(), 1, "only the store of the inner `xs`");
        assert!(found.containers.contains("keep"));
    }

    /// A call to a function body can hand the statement a resource, so its value
    /// is owned one scope inside the current one; a natively lowered builtin's
    /// answer reaches only what its arguments do.
    #[test]
    fn a_function_body_call_floats_and_a_builtin_over_outer_values_does_not() {
        let callee = function(Vec::new());
        let mut functions: HashMap<String, &NirFunction> = HashMap::new();
        functions.insert("mkList".to_string(), &callee);
        let body = vec![
            bind("keep", empty()),
            NirOp::Assign {
                name: "keep".to_string(),
                value: call("mkList", Vec::new()),
            },
            NirOp::Assign {
                name: "keep".to_string(),
                value: call("canvas.groupItems", vec![local("param")]),
            },
        ];
        let found = floats(body, &functions);
        assert_eq!(found.stores.len(), 1, "only the function-body call");
    }

    #[test]
    fn a_local_stored_once_by_a_bare_local_is_an_alias() {
        let body = vec![
            bind("a", empty()),
            bind("b", local("a")),
            bind("c", local("a")),
            NirOp::Assign {
                name: "c".to_string(),
                value: local("b"),
            },
        ];
        let aliases = alias_sources(&function(body));
        assert_eq!(aliases.get("b").map(String::as_str), Some("a"));
        assert!(!aliases.contains_key("c"), "stored twice");
        assert!(!aliases.contains_key("a"));
    }
}
