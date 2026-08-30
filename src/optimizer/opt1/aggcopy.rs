//! Object / aggregate copy propagation — a Level-3 Opt1 catalog row
//! (`planning/optimizations.md`): forward a whole value-semantic aggregate
//! instead of copying it.
//!
//! `LET b = a` on a record, union, String or collection is not a register
//! move — MFB has value semantics, so codegen emits a real deep copy of the
//! block (`copy_collection_tight` for a collection, `copy_flat_block`
//! otherwise) and `b` owns the copy. When neither name is ever written again,
//! that copy is a duplicate of a value nothing can tell apart from the
//! original, so the row deletes the binding and points `b`'s readers at `a`.
//!
//! Scalar copy propagation does not reach this: it works on registers, and an
//! aggregate is a block. NRVO does not reach it either: that elides the copy
//! at a `RETURN`, not at a binding.
//!
//! **The gates, and why each one is load-bearing.** Getting this wrong is a
//! double free or a use-after-free, not a missed optimization, so the row
//! declines on anything it cannot prove:
//!
//! - **Both names immutable and never assigned.** A write to either would
//!   make the two values genuinely different from that point on. This is
//!   checked scope-blind across the whole function, so shadowing anywhere
//!   blocks the rewrite.
//! - **Neither name address-taken.** A `LocalRef` hands a callee the slot
//!   itself, which is the one way a local can change without an `Assign`.
//! - **Neither name captured by a closure.** A capture deep-copies the local
//!   into the environment; re-pointing which local is copied is sound, but the
//!   environment's own free is decided from the capture's static type, and
//!   that is a second ownership story this row does not need to enter.
//! - **Neither name a resource owner.** Resource handles are unique by
//!   construction (§15); aliasing one is exactly the thing the ownership
//!   model forbids.
//! - **The source is bound in this function, not a parameter.** A parameter's
//!   block belongs to the caller and has no owned-value cleanup here, so
//!   `RETURN b` (which the return-move elision turns into a move of the
//!   source) must keep seeing a local it may actually move.
//!
//! With those, exactly one block exists where two did, exactly one cleanup
//! frees it, and every reader sees the same bytes it saw before. The value's
//! lifetime can only get *longer* (the source outlives the binding it
//! replaced), which is safe in an arena and is the memory-lifetime row's
//! problem, not a correctness one.

use std::collections::{HashMap, HashSet};

use crate::target::shared::nir::visit::{walk_op, walk_value, NirVisitor};
use crate::target::shared::nir::{NirFunction, NirModule, NirOp, NirValue};
use crate::types::ParameterType;

use super::plans::shape::{nested_bodies_mut, own_values_mut};

/// Apply the row to the whole module. Self-guarded on its catalog level (3).
pub(crate) fn propagate(module: &mut NirModule) {
    if !crate::optimizer::level_enabled(3) {
        return;
    }
    let mut fired = 0u64;
    for function in &mut module.functions {
        loop {
            let facts = Facts::census(function);
            let Some((binding, source)) = pick(function, &facts) else {
                break;
            };
            rewrite_reads(&mut function.body, &binding, &source);
            drop_bind(&mut function.body, &binding);
            fired += 1;
        }
    }
    crate::optimizer::stats::count_aggregate_copies_forwarded(fired);
}

/// Per-name occurrence facts, scope-blind on purpose: NIR keeps source names
/// under shadowing, so a per-scope census could confuse two bindings of the
/// same name. Counting every occurrence anywhere is the conservative choice.
#[derive(Default)]
struct Facts {
    binds: HashMap<String, usize>,
    writes: HashSet<String>,
    address_taken: HashSet<String>,
    captured: HashSet<String>,
}

impl Facts {
    fn census(function: &NirFunction) -> Facts {
        let mut facts = Facts::default();
        for param in &function.params {
            // A parameter is not a local binding this row may move from, so
            // record it as bound elsewhere.
            *facts.binds.entry(param.name.clone()).or_insert(0) += 2;
        }
        // `visit_ops`, not a bare `walk_op` per statement: `walk_op` descends
        // into an op's *children*, so driving it directly skips `visit_op` on
        // the top-level statements themselves — which is exactly where the
        // `Assign` that blocks a forward lives.
        facts.visit_ops(&function.body);
        facts
    }

    /// Whether the name is safe to alias: written once, never re-written,
    /// never address-taken, never captured.
    fn stable(&self, name: &str) -> bool {
        self.binds.get(name).copied().unwrap_or(0) <= 1
            && !self.writes.contains(name)
            && !self.address_taken.contains(name)
            && !self.captured.contains(name)
    }
}

impl NirVisitor for Facts {
    fn visit_op(&mut self, op: &NirOp) {
        match op {
            NirOp::Bind { name, .. } => {
                *self.binds.entry(name.clone()).or_insert(0) += 1;
            }
            NirOp::Assign { name, .. } => {
                self.writes.insert(name.clone());
            }
            NirOp::StateAssign { resource, .. } => {
                self.writes.insert(resource.clone());
            }
            // A loop or handler binder re-introduces the name each iteration.
            NirOp::For { name, .. } | NirOp::ForEach { name, .. } | NirOp::Trap { name, .. } => {
                *self.binds.entry(name.clone()).or_insert(0) += 2;
            }
            _ => {}
        }
        walk_op(self, op);
    }

    fn visit_value(&mut self, value: &NirValue) {
        match value {
            NirValue::LocalRef { name, .. } => {
                self.address_taken.insert(name.clone());
            }
            NirValue::Closure { captures, .. } => {
                for capture in captures {
                    if let NirValue::Local(name) = capture {
                        self.captured.insert(name.clone());
                    }
                }
            }
            _ => {}
        }
        walk_value(self, value);
    }
}

/// The types whose binding copies a block rather than a register.
fn is_aggregate(type_: &ParameterType) -> bool {
    let name = type_.name();
    name == "String"
        || crate::codegen::engine::types::is_collection_type(&name)
        // A record or union spells as a bare user type name; the primitives
        // and the function/thread types do not copy a block.
        || !matches!(
            name.as_ref(),
            "Integer" | "Float" | "Boolean" | "Byte" | "Nothing" | "Error" | "ErrorLoc"
        ) && !name.contains("FUNC")
            && !name.contains("Thread")
            && !name.starts_with("RES")
}

/// The first `LET b = a` this row may forward, as `(b, a)`.
fn pick(function: &NirFunction, facts: &Facts) -> Option<(String, String)> {
    let mut found = None;
    find_candidate(&function.body, function, facts, &mut found);
    found
}

fn find_candidate(
    ops: &[NirOp],
    function: &NirFunction,
    facts: &Facts,
    found: &mut Option<(String, String)>,
) {
    for op in ops {
        if found.is_some() {
            return;
        }
        if let NirOp::Bind {
            mutable: false,
            name,
            type_,
            value: Some(NirValue::Local(source)),
        } = op
        {
            if is_aggregate(type_)
                && name != source
                && facts.stable(name)
                && facts.stable(source)
                && !function.resource_owners.contains_key(name)
                && !function.resource_owners.contains_key(source)
                && function.params.iter().all(|param| param.name != *source)
            {
                *found = Some((name.clone(), source.clone()));
                return;
            }
        }
        for body in super::plans::shape::nested_bodies(op) {
            find_candidate(body, function, facts, found);
            if found.is_some() {
                return;
            }
        }
    }
}

/// Repoint every read of `binding` at `source`.
fn rewrite_reads(ops: &mut Vec<NirOp>, binding: &str, source: &str) {
    for op in ops.iter_mut() {
        for value in own_values_mut(op) {
            rewrite_value(value, binding, source);
        }
        if let NirOp::Match { cases, .. } = op {
            for case in cases.iter_mut() {
                if let Some(guard) = case.guard.as_mut() {
                    rewrite_value(guard, binding, source);
                }
            }
        }
        for body in nested_bodies_mut(op) {
            rewrite_reads(body, binding, source);
        }
    }
}

fn rewrite_value(value: &mut NirValue, binding: &str, source: &str) {
    if matches!(value, NirValue::Local(name) if name == binding) {
        *value = NirValue::Local(source.to_string());
        return;
    }
    for child in children_mut(value) {
        rewrite_value(child, binding, source);
    }
}

fn children_mut(value: &mut NirValue) -> Vec<&mut NirValue> {
    match value {
        NirValue::Call { args, .. }
        | NirValue::CallResult { args, .. }
        | NirValue::RuntimeCall { args, .. }
        | NirValue::Constructor { args, .. } => args.iter_mut().collect(),
        NirValue::Closure { captures, .. } => captures.iter_mut().collect(),
        NirValue::UnionWrap { value, .. }
        | NirValue::UnionExtract { value, .. }
        | NirValue::ResultIsOk { value }
        | NirValue::ResultValue { value }
        | NirValue::ResultError { value }
        | NirValue::MemberAccess { target: value, .. }
        | NirValue::Unary { operand: value, .. } => vec![&mut **value],
        NirValue::WithUpdate {
            target, updates, ..
        } => std::iter::once(&mut **target)
            .chain(updates.iter_mut().map(|update| &mut update.value))
            .collect(),
        NirValue::ListLiteral { values, .. } | NirValue::SetLiteral { values, .. } => {
            values.iter_mut().collect()
        }
        NirValue::MapLiteral { entries, .. } => entries
            .iter_mut()
            .flat_map(|(key, value)| [key, value])
            .collect(),
        NirValue::Binary { left, right, .. } => vec![&mut **left, &mut **right],
        _ => Vec::new(),
    }
}

/// Remove the (now readerless) binding.
fn drop_bind(ops: &mut Vec<NirOp>, binding: &str) -> bool {
    if let Some(index) = ops
        .iter()
        .position(|op| matches!(op, NirOp::Bind { name, .. } if name == binding))
    {
        ops.remove(index);
        return true;
    }
    for op in ops.iter_mut() {
        for body in nested_bodies_mut(op) {
            if drop_bind(body, binding) {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::super::local_rewrites::testutil::*;
    use super::*;
    use crate::optimizer::{with_opt_level, OptLevel};
    use crate::target::shared::nir::NirSourceLoc;

    fn function(body: Vec<NirOp>) -> NirFunction {
        NirFunction {
            name: "f".to_string(),
            visibility: "private".to_string(),
            kind: "function".to_string(),
            isolated: false,
            params: vec![],
            returns: ParameterType::String,
            body,
            file: "main.mfb".to_string(),
            resource_owners: std::collections::HashMap::new(),
        }
    }

    fn bind(name: &str, type_: ParameterType, value: NirValue) -> NirOp {
        NirOp::Bind {
            mutable: false,
            name: name.to_string(),
            type_,
            value: Some(value),
        }
    }

    fn run(module: &mut NirModule, level: u8) {
        with_opt_level(OptLevel(level), || propagate(module));
    }

    /// `LET b = a` on a String is a whole-block copy; with neither name ever
    /// written, the copy goes and `b`'s reader reads `a`.
    #[test]
    fn a_stable_aggregate_copy_is_forwarded() {
        let mut module = test_module(vec![function(vec![
            bind(
                "a",
                ParameterType::String,
                typed_const(ParameterType::String, "hi"),
            ),
            bind("b", ParameterType::String, local("a")),
            NirOp::Return {
                value: Some(local("b")),
            },
        ])]);
        run(&mut module, 3);
        let body = &module.functions[0].body;
        assert_eq!(body.len(), 2, "the copy binding is gone");
        assert!(
            matches!(&body[1], NirOp::Return { value: Some(NirValue::Local(name)) } if name == "a"),
            "the reader now reads the source"
        );
    }

    /// A scalar copy is a register move, not a block copy — not this row's
    /// business, and left for scalar propagation.
    #[test]
    fn a_scalar_copy_is_left_alone() {
        let mut module = test_module(vec![function(vec![
            bind("a", ParameterType::Integer, int_const("1")),
            bind("b", ParameterType::Integer, local("a")),
            NirOp::Return {
                value: Some(local("b")),
            },
        ])]);
        run(&mut module, 3);
        assert_eq!(module.functions[0].body.len(), 3);
    }

    /// Writing the source makes the two values genuinely different from that
    /// point on, so the copy stays.
    #[test]
    fn a_written_source_keeps_the_copy() {
        let mut module = test_module(vec![function(vec![
            bind(
                "a",
                ParameterType::String,
                typed_const(ParameterType::String, "hi"),
            ),
            bind("b", ParameterType::String, local("a")),
            NirOp::Assign {
                name: "a".to_string(),
                value: typed_const(ParameterType::String, "bye"),
            },
            NirOp::Return {
                value: Some(local("b")),
            },
        ])]);
        run(&mut module, 3);
        assert_eq!(module.functions[0].body.len(), 4);
    }

    /// A slot reference hands a callee the local itself, which is the one way
    /// it can change without an assignment.
    #[test]
    fn an_address_taken_source_keeps_the_copy() {
        let mut module = test_module(vec![function(vec![
            bind(
                "a",
                ParameterType::String,
                typed_const(ParameterType::String, "hi"),
            ),
            bind("b", ParameterType::String, local("a")),
            NirOp::Eval {
                value: NirValue::Call {
                    target: "helper".to_string(),
                    args: vec![NirValue::LocalRef {
                        name: "a".to_string(),
                        type_: ParameterType::String,
                    }],
                    loc: NirSourceLoc::default(),
                },
            },
            NirOp::Return {
                value: Some(local("b")),
            },
        ])]);
        run(&mut module, 3);
        assert_eq!(module.functions[0].body.len(), 4);
    }

    /// A parameter's block belongs to the caller and has no owned-value
    /// cleanup here, so it is never the source of a forward.
    #[test]
    fn a_parameter_source_keeps_the_copy() {
        let mut f = function(vec![
            bind("b", ParameterType::String, local("s")),
            NirOp::Return {
                value: Some(local("b")),
            },
        ]);
        f.params = vec![crate::target::shared::nir::NirParam {
            name: "s".to_string(),
            type_: ParameterType::String,
            default: None,
        }];
        let mut module = test_module(vec![f]);
        run(&mut module, 3);
        assert_eq!(module.functions[0].body.len(), 2);
    }

    /// The row is off below `-O3`.
    #[test]
    fn level_two_disables_the_row() {
        let mut module = test_module(vec![function(vec![
            bind(
                "a",
                ParameterType::String,
                typed_const(ParameterType::String, "hi"),
            ),
            bind("b", ParameterType::String, local("a")),
            NirOp::Return {
                value: Some(local("b")),
            },
        ])]);
        run(&mut module, 2);
        assert_eq!(module.functions[0].body.len(), 3);
    }
}
