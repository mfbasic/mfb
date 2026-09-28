//! Place copy forwarding — a Level-1 Opt1 catalog row (plan-149,
//! `planning/optimizations.md`): drop the deep copy `LET b = <place>` makes when
//! every read of `b` happens before the next write of the place's owner.
//!
//! `LET cur AS List OF Integer = rec.xs` copies the field, because MFB has value
//! semantics and `cur` owns its value. The update that follows,
//! `rec = WITH rec { xs := collections::set(cur, j, v) }`, then never matches the
//! in-place field seam (plan-145), which wants the field expression itself as the
//! operand, so the record is rebuilt too. When nothing writes `rec` between the
//! binding and `cur`'s last read, `cur` and `rec.xs` are the same value at every
//! read, so the row points the reads at `rec.xs` and deletes the binding. The NIR
//! is then exactly the direct form, and the existing seam takes it — no codegen
//! change.
//!
//! Keeping the copy is always correct, so this is Level 1 ("same operations,
//! less waste"), not Level 0. It differs from [`super::aggcopy`] (Level 3) in its
//! proof: aggcopy needs both names never written anywhere; this row allows the
//! owner to be written, and proves instead that the write comes after the last
//! read (the *window*).
//!
//! **A place** is `Local(o)`, or a `MemberAccess` chain rooted at `Local(o)`. No
//! call, union extract or other node sits in the chain, so evaluating the place
//! later than the binding did cannot trap or observe anything but `o`.
//!
//! **Function-wide gates** (a scope-blind census, as in aggcopy — NIR keeps
//! source names under shadowing, so counting every occurrence is the
//! conservative choice):
//!
//! - `b` is bound once, never assigned, never address-taken (`LocalRef`), never
//!   captured, not a parameter and not a resource owner.
//! - The owner `o` has exactly one binding (a `Bind`, or else it is a parameter
//!   with none), is never a loop or handler binder, never address-taken and
//!   never captured, and is not a `RES` handle. Those are the ways a local can
//!   change other than a visible `Assign{o}`.
//! - A bare `Local(o)` source is not a parameter: `RETURN b` becomes `RETURN o`,
//!   and the return-move elision must still see a local it may move (aggcopy's
//!   reason).
//!
//! **The window** — let the binding be `ops[i]` of its block; scope confines
//! every read of `b` to `ops[i+1..]`, possibly nested. Let `L` be the last op
//! holding a read.
//!
//! - **W1.** No op in `i+1..L` (exclusive) writes the owner, at any depth.
//! - **W2.** Op `L` writes the owner nowhere, **or** is itself the top-level
//!   owner write whose value holds the reads. A value is fully evaluated before
//!   its store, and `WITH` evaluates every right-hand side against the old
//!   record, so the forwarded reads still see the old value.
//! - **W3.** No read of `b` sits inside a `TRAP` handler (it runs after the
//!   failing statement, which may be past the window).
//! - Every read of `b` in the function lies in the window. Scope guarantees this
//!   for source programs; the check makes the rewrite total by construction
//!   rather than by a scope argument (a `DO … LOOP UNTIL` condition, for one, is
//!   evaluated by the loop op, not by its body).
//!
//! A binding inside a loop body is re-evaluated each iteration and its window
//! ends inside that body, so loops need no extra rule. A write in a branch not
//! taken still declines: the proof is structural, not path-sensitive.

use std::collections::{HashMap, HashSet};

use crate::target::shared::nir::visit::{walk_op, walk_value, NirVisitor};
use crate::target::shared::nir::{
    NirFunction, NirMatchPattern, NirModule, NirOp, NirType, NirValue,
};
use crate::types::ParameterType;

use super::aggcopy::is_aggregate;
use super::plans::shape::{nested_bodies, nested_bodies_mut, own_values_mut, value_children_mut};

/// Apply the row to the whole module. Self-guarded on its catalog level (1).
pub(crate) fn forward(module: &mut NirModule) {
    if !crate::optimizer::level_enabled(1) {
        return;
    }
    let mut fired = 0u64;
    let module_facts = ModuleFacts::of(module);
    for function in &mut module.functions {
        loop {
            let facts = Facts::census(function);
            let context = Context {
                function_params: function.params.iter().map(|p| p.name.clone()).collect(),
                resource_owners: function.resource_owners.keys().cloned().collect(),
                facts,
                module: &module_facts,
            };
            if !forward_one(&mut function.body, &context) {
                break;
            }
            fired += 1;
        }
    }
    crate::optimizer::stats::count_place_copies_forwarded(fired);
}

/// What the gates need about one function, computed once per forward.
struct Context<'m> {
    function_params: HashSet<String>,
    resource_owners: HashSet<String>,
    facts: Facts,
    module: &'m ModuleFacts,
}

/// Module-wide facts the call gate (W4) needs.
struct ModuleFacts {
    /// Every user function, lambda included: a call to one can write a global
    /// or reach a by-ref slot through another closure.
    user_functions: HashSet<String>,
    globals: HashMap<String, ParameterType>,
    /// Each user type's member types (record fields, union variant fields).
    type_members: HashMap<String, Vec<ParameterType>>,
}

impl ModuleFacts {
    fn of(module: &NirModule) -> ModuleFacts {
        let member_types = |t: &NirType| {
            t.fields
                .iter()
                .chain(t.variants.iter().flat_map(|variant| variant.fields.iter()))
                .map(|field| field.type_.clone())
                .collect()
        };
        ModuleFacts {
            user_functions: module.functions.iter().map(|f| f.name.clone()).collect(),
            globals: module
                .globals
                .iter()
                .map(|g| (g.name.clone(), g.type_.clone()))
                .collect(),
            type_members: module
                .types
                .iter()
                .map(|t| (t.name.clone(), member_types(t)))
                .collect(),
        }
    }

    /// Whether a value of `type_` can be, or can carry, a function value — the
    /// only way a builtin call can run user code. Conservative: an unresolved
    /// user nominal, a type variable and a thread handle all answer yes.
    fn may_hold_function(&self, type_: &ParameterType) -> bool {
        self.may_hold_function_in(type_, &mut HashSet::new())
    }

    fn may_hold_function_in(&self, type_: &ParameterType, seen: &mut HashSet<String>) -> bool {
        match type_ {
            ParameterType::Func(..)
            | ParameterType::ThreadHandle { .. }
            | ParameterType::Var(_)
            | ParameterType::Arg(_)
            | ParameterType::Unknown => true,
            ParameterType::ListOf(inner)
            | ParameterType::SetOf(inner)
            | ParameterType::ResultOf(inner)
            | ParameterType::Res(inner) => self.may_hold_function_in(inner, seen),
            ParameterType::MapOf(key, value) | ParameterType::MapEntryOf(key, value) => {
                self.may_hold_function_in(key, seen) || self.may_hold_function_in(value, seen)
            }
            ParameterType::Stateful { base, state } => {
                self.may_hold_function_in(base, seen) || self.may_hold_function_in(state, seen)
            }
            ParameterType::Named(_) | ParameterType::UserOf(..) => {
                let name = type_.name().into_owned();
                if !seen.insert(name.clone()) {
                    return false; // already being answered further up
                }
                match self.type_members.get(&name) {
                    Some(members) => members
                        .iter()
                        .any(|member| self.may_hold_function_in(member, seen)),
                    // A package's opaque type (`fs.File`) is the runtime's; it
                    // holds no user function. An unknown bare nominal might.
                    None => {
                        !name.contains('.')
                            && !type_.is_named("Error")
                            && !type_.is_named("ErrorLoc")
                    }
                }
            }
            ParameterType::AttributeString
            | ParameterType::Boolean
            | ParameterType::Byte
            | ParameterType::Integer
            | ParameterType::Fixed
            | ParameterType::Float
            | ParameterType::Money
            | ParameterType::Nothing
            | ParameterType::String
            | ParameterType::C(_) => false,
        }
    }
}

/// Per-name occurrence facts across the whole function, scope-blind.
#[derive(Default)]
struct Facts {
    /// Binding positions: `Bind`, loop and handler binders (counted twice, so a
    /// binder never passes a "bound once" test), parameters.
    binds: HashMap<String, usize>,
    /// The type and initializer class of each `Bind` (the last one seen; only
    /// consulted for a name bound exactly once).
    bind_types: HashMap<String, ParameterType>,
    by_ref_bound: HashSet<String>,
    writes: HashSet<String>,
    address_taken: HashSet<String>,
    captured: HashSet<String>,
    /// Every `Local(name)` read, anywhere, including match patterns and guards.
    reads: HashMap<String, usize>,
}

impl Facts {
    fn census(function: &NirFunction) -> Facts {
        let mut facts = Facts::default();
        for param in &function.params {
            *facts.binds.entry(param.name.clone()).or_insert(0) += 1;
            facts
                .bind_types
                .insert(param.name.clone(), param.type_.clone());
        }
        // `visit_ops`, not `walk_op` per statement: `walk_op` visits an op's
        // children only, which would skip the top-level writes themselves.
        facts.visit_ops(&function.body);
        facts
    }
}

impl NirVisitor for Facts {
    fn visit_op(&mut self, op: &NirOp) {
        match op {
            NirOp::Bind {
                name, type_, value, ..
            } => {
                *self.binds.entry(name.clone()).or_insert(0) += 1;
                self.bind_types.insert(name.clone(), type_.clone());
                if matches!(value, Some(NirValue::Capture { by_ref: true, .. })) {
                    self.by_ref_bound.insert(name.clone());
                }
            }
            NirOp::Assign { name, .. } => {
                self.writes.insert(name.clone());
            }
            NirOp::StateAssign { resource, .. } => {
                self.writes.insert(resource.clone());
            }
            NirOp::For { name, type_, .. } | NirOp::ForEach { name, type_, .. } => {
                *self.binds.entry(name.clone()).or_insert(0) += 2;
                self.bind_types.insert(name.clone(), type_.clone());
            }
            NirOp::Trap { name, .. } => {
                *self.binds.entry(name.clone()).or_insert(0) += 2;
                self.bind_types
                    .insert(name.clone(), ParameterType::named("Error"));
            }
            _ => {}
        }
        walk_op(self, op);
    }

    fn visit_value(&mut self, value: &NirValue) {
        match value {
            NirValue::Local(name) => {
                *self.reads.entry(name.clone()).or_insert(0) += 1;
            }
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

/// The owner of a place, and what counts as writing it.
#[derive(Clone)]
enum Owner {
    /// A local (or parameter) this function owns outright: only `Assign{o}`
    /// writes it, so no call can break its window.
    Local(String),
    /// A local bound from a by-ref capture (`Capture{by_ref: true}`, a
    /// `collections::forEach` lambda writing an outer `MUT`): its reads and
    /// writes go through the parent's slot.
    ByRef(String),
    /// A module global.
    Global(String),
}

impl Owner {
    /// Whether `op` itself (not its children) is a write of this owner.
    fn is_write(&self, op: &NirOp) -> bool {
        match self {
            Owner::Local(name) | Owner::ByRef(name) => {
                matches!(op, NirOp::Assign { name: written, .. } if written == name)
            }
            // `Assign` of the same name is a local that shadows the global, not
            // the global; declining on it costs nothing and needs no proof.
            Owner::Global(name) => matches!(
                op,
                NirOp::StoreGlobal { name: written, .. } | NirOp::Assign { name: written, .. }
                    if written == name
            ),
        }
    }

    /// Whether something other than a visible write could change the owner, so
    /// the window must also be call-safe (W4).
    fn needs_call_gate(&self) -> bool {
        !matches!(self, Owner::Local(_))
    }
}

/// The root of a place, or `None` when `value` is not a place.
fn place_root(value: &NirValue) -> Option<(&NirValue, usize)> {
    let mut depth = 0;
    let mut cursor = value;
    while let NirValue::MemberAccess { target, .. } = cursor {
        depth += 1;
        cursor = target;
    }
    matches!(cursor, NirValue::Local(_) | NirValue::Global { .. }).then_some((cursor, depth))
}

/// Find and apply the first forwardable binding in `ops` (or any nested body).
fn forward_one(ops: &mut Vec<NirOp>, context: &Context) -> bool {
    for index in 0..ops.len() {
        if let Some((binding, place, last)) = candidate(ops, index, context) {
            for op in &mut ops[index + 1..=last] {
                rewrite_op(op, &binding, &place);
            }
            ops.remove(index);
            return true;
        }
        for body in nested_bodies_mut(&mut ops[index]) {
            if forward_one(body, context) {
                return true;
            }
        }
    }
    false
}

/// `ops[index]` as `(b, place, L)` when every gate holds.
fn candidate(ops: &[NirOp], index: usize, context: &Context) -> Option<(String, NirValue, usize)> {
    let NirOp::Bind {
        mutable: false,
        name: binding,
        type_,
        value: Some(place),
    } = &ops[index]
    else {
        return None;
    };
    if !is_aggregate(type_) {
        return None;
    }
    let facts = &context.facts;
    let binding_stable = facts.binds.get(binding).copied() == Some(1)
        && !facts.writes.contains(binding)
        && !facts.address_taken.contains(binding)
        && !facts.captured.contains(binding)
        && !context.function_params.contains(binding)
        && !context.resource_owners.contains(binding);
    if !binding_stable {
        return None;
    }
    let owner = owner_of(place, binding, context)?;

    // Every read of `b` lies in `ops[index+1..]`; `L` is the last op with one.
    let total_reads = facts.reads.get(binding).copied().unwrap_or(0);
    let mut window_reads = 0;
    let mut last = None;
    for (offset, op) in ops[index + 1..].iter().enumerate() {
        let mut probe = ReadProbe::new(binding);
        probe.visit_op(op);
        if probe.in_handler {
            return None; // W3
        }
        if probe.reads > 0 {
            window_reads += probe.reads;
            last = Some(index + 1 + offset);
        }
    }
    let last = last?;
    if window_reads != total_reads {
        return None;
    }
    // W1: nothing strictly inside the window writes the owner.
    if ops[index + 1..last]
        .iter()
        .any(|op| writes_owner(op, &owner))
    {
        return None;
    }
    // W2: op L is the top-level owner write, or writes the owner nowhere.
    let last_op = &ops[last];
    if !owner.is_write(last_op) && writes_owner(last_op, &owner) {
        return None;
    }
    // W4: for an owner a call can reach, nothing in the window — op L's value
    // included — may run user code. Including op L keeps the proof free of
    // argument order: `g = WITH g { xs := set(cur, j, helper()) }` read `cur`
    // before `helper()` ran, but its forwarded `g.xs` is read when the argument
    // is evaluated.
    if owner.needs_call_gate() {
        let mut gate = CallGate {
            context,
            runs_user_code: false,
        };
        gate.visit_ops(&ops[index + 1..=last]);
        if gate.runs_user_code {
            return None;
        }
    }
    Some((binding.clone(), place.clone(), last))
}

/// The owner of `place` when it is one this row can prove a window for.
fn owner_of(place: &NirValue, binding: &str, context: &Context) -> Option<Owner> {
    let (root, depth) = place_root(place)?;
    let owner = match root {
        // A global cannot be address-taken, captured or rebound, so it needs
        // no census gate beyond W4.
        NirValue::Global { name, .. } => return Some(Owner::Global(name.clone())),
        NirValue::Local(owner) => owner,
        _ => return None,
    };
    if owner == binding {
        return None;
    }
    let facts = &context.facts;
    let is_param = context.function_params.contains(owner);
    // One binding: its own `Bind`, or the parameter (which never has a `Bind`
    // of the same name when the count is exactly one).
    let owner_stable = facts.binds.get(owner).copied() == Some(1)
        && !facts.address_taken.contains(owner)
        && !facts.captured.contains(owner)
        && !context.resource_owners.contains(owner)
        && !matches!(
            facts.bind_types.get(owner),
            Some(ParameterType::Stateful { .. } | ParameterType::Res(_)) | None
        );
    if !owner_stable {
        return None;
    }
    // A bare local copied from a parameter keeps the copy (return-move).
    if depth == 0 && is_param {
        return None;
    }
    if facts.by_ref_bound.contains(owner) {
        return Some(Owner::ByRef(owner.clone()));
    }
    Some(Owner::Local(owner.clone()))
}

/// W4's scan: does anything in the window run user code? A call to a user
/// function, a call through a function value (its target is a local, a
/// parameter or a global, not a function), and any function value at all —
/// a `FunctionRef`, a `Closure`, or a binding whose type can hold one — since
/// that is the only way a builtin call can call back into user code. Builtin
/// package calls are otherwise allowed: the runtime writes no user global, slot
/// or `STATE` payload.
struct CallGate<'c, 'm> {
    context: &'c Context<'m>,
    runs_user_code: bool,
}

impl CallGate<'_, '_> {
    fn calls_user_code(&self, target: &str) -> bool {
        let module = self.context.module;
        module.user_functions.contains(target)
            || module.globals.contains_key(target)
            || self.context.facts.bind_types.contains_key(target)
    }
}

impl NirVisitor for CallGate<'_, '_> {
    fn visit_value(&mut self, value: &NirValue) {
        let module = self.context.module;
        let found = match value {
            NirValue::Call { target, .. }
            | NirValue::CallResult { target, .. }
            | NirValue::RuntimeCall { target, .. } => self.calls_user_code(target),
            NirValue::FunctionRef { .. } | NirValue::Closure { .. } => true,
            NirValue::Local(name) => self
                .context
                .facts
                .bind_types
                .get(name)
                .is_none_or(|type_| module.may_hold_function(type_)),
            NirValue::Capture { type_, .. } | NirValue::Global { type_, .. } => {
                module.may_hold_function(type_)
            }
            _ => false,
        };
        if found {
            self.runs_user_code = true;
            return;
        }
        walk_value(self, value);
    }
}

/// Whether `op` or anything nested in it writes the owner.
fn writes_owner(op: &NirOp, owner: &Owner) -> bool {
    owner.is_write(op)
        || nested_bodies(op)
            .into_iter()
            .any(|body| body.iter().any(|inner| writes_owner(inner, owner)))
}

/// Counts reads of one name in an op, and whether any sits in a `TRAP` handler.
struct ReadProbe<'a> {
    name: &'a str,
    reads: usize,
    handler_depth: usize,
    in_handler: bool,
}

impl<'a> ReadProbe<'a> {
    fn new(name: &'a str) -> Self {
        ReadProbe {
            name,
            reads: 0,
            handler_depth: 0,
            in_handler: false,
        }
    }
}

impl NirVisitor for ReadProbe<'_> {
    fn visit_op(&mut self, op: &NirOp) {
        if let NirOp::Trap { .. } = op {
            self.handler_depth += 1;
            walk_op(self, op);
            self.handler_depth -= 1;
            return;
        }
        walk_op(self, op);
    }

    fn visit_value(&mut self, value: &NirValue) {
        if matches!(value, NirValue::Local(name) if name == self.name) {
            self.reads += 1;
            if self.handler_depth > 0 {
                self.in_handler = true;
            }
        }
        walk_value(self, value);
    }
}

/// Replace every read of `binding` in `op` with `place`: the op's own values,
/// its `MATCH` patterns and guards, and every nested body — the same reach as
/// [`NirVisitor`], so the census count above is exactly what gets rewritten.
fn rewrite_op(op: &mut NirOp, binding: &str, place: &NirValue) {
    for value in own_values_mut(op) {
        rewrite_value(value, binding, place);
    }
    if let NirOp::Match { cases, .. } = op {
        for case in cases.iter_mut() {
            match &mut case.pattern {
                NirMatchPattern::Else => {}
                NirMatchPattern::Value(value) => rewrite_value(value, binding, place),
                NirMatchPattern::OneOf(values) => {
                    for value in values {
                        rewrite_value(value, binding, place);
                    }
                }
            }
            if let Some(guard) = case.guard.as_mut() {
                rewrite_value(guard, binding, place);
            }
        }
    }
    for body in nested_bodies_mut(op) {
        for inner in body.iter_mut() {
            rewrite_op(inner, binding, place);
        }
    }
}

fn rewrite_value(value: &mut NirValue, binding: &str, place: &NirValue) {
    if matches!(value, NirValue::Local(name) if name == binding) {
        *value = place.clone();
        return;
    }
    for child in value_children_mut(value) {
        rewrite_value(child, binding, place);
    }
}

#[cfg(test)]
mod tests;
