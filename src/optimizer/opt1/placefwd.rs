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
//! **A place** is `Local(o)` or `Global{g}`, or a `MemberAccess` chain rooted at
//! one. No call, union extract or other node sits in the chain, so evaluating
//! the place later than the binding did cannot trap or observe anything but the
//! owner.
//!
//! **Owners**, and what writes each: an owned local (`Assign{o}` only), a local
//! bound from a by-ref capture (`Assign{o}`, plus anything that reaches the
//! parent's slot), a global (`StoreGlobal{g}`, plus any user code), and a
//! resource's `STATE` (`StateAssign{f}`, plus anything the handle is passed to).
//!
//! **Scope.** A binding's scope is the rest of its block: a name cannot shadow
//! an enclosing one (`SYMBOL_DUPLICATE_LOCAL`), but it can be reused by a later,
//! disjoint scope (the benchmark binds `cur` inside a loop and again after it),
//! and a `DO … LOOP UNTIL` condition cannot see its body's bindings. So every
//! `Local(b)` in `ops[i+1..]` of the binding's block is a read of *this* `b`,
//! provided nothing there binds `b` again — the row declines if anything does.
//! The owner is resolved the same way, through the bindings visible at `ops[i]`.
//!
//! **Gates on `b`**, within `ops[i+1..]`: not bound again, not assigned, not
//! address-taken (`LocalRef`), not captured by a closure; and `b` is not a
//! parameter or a resource.
//!
//! **Gates on a local owner `o`**: never address-taken or captured anywhere in
//! the function (by name, so conservatively), not bound again inside the window,
//! and a bare `Local(o)` source is not a parameter — `RETURN b` becomes
//! `RETURN o`, and the return-move elision must still see a local it may move
//! (aggcopy's reason).
//!
//! **The window** — let `L` be the last op of `ops[i+1..]` holding a read of `b`.
//!
//! - **W1.** No op in `i+1..L` (exclusive) writes the owner, at any depth.
//! - **W2.** Op `L` writes the owner nowhere, **or** is itself the top-level
//!   owner write whose value holds the reads. A value is fully evaluated before
//!   its store, and `WITH` evaluates every right-hand side against the old
//!   record, so the forwarded reads still see the old value.
//! - **W3.** No read of `b` sits inside a `TRAP` handler (it runs after the
//!   failing statement, which may be past the window).
//! - **W4** (by-ref, global, `STATE` owners). Nothing in `i+1..=L`, op L's value
//!   included, can run user code: no user or indirect call and no function value.
//! - **W5** (`STATE` owners). The resource itself appears in the window only as
//!   the root of a member chain; handing it anywhere declines.
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
            let context = Context {
                function_params: function.params.iter().map(|p| p.name.clone()).collect(),
                resource_owners: function.resource_owners.keys().cloned().collect(),
                facts: Facts::census(function),
                module: &module_facts,
            };
            let mut scope = Scope::default();
            scope.open();
            for param in &function.params {
                scope.declare(
                    &param.name,
                    Visible {
                        type_: param.type_.clone(),
                        param: true,
                        by_ref: false,
                    },
                );
            }
            if !forward_one(&mut function.body, &context, &mut scope) {
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

/// Per-name facts across the whole function, scope-blind: used only where
/// merging every binding of a name errs toward declining.
#[derive(Default)]
struct Facts {
    /// Every type any binding of the name has (a `Bind`, a loop or handler
    /// binder, a parameter): a reused name must not hide a function-typed one.
    name_types: HashMap<String, Vec<ParameterType>>,
    address_taken: HashSet<String>,
    captured: HashSet<String>,
}

impl Facts {
    fn census(function: &NirFunction) -> Facts {
        let mut facts = Facts::default();
        for param in &function.params {
            facts.note_type(&param.name, &param.type_);
        }
        facts.visit_ops(&function.body);
        facts
    }

    fn note_type(&mut self, name: &str, type_: &ParameterType) {
        self.name_types
            .entry(name.to_string())
            .or_default()
            .push(type_.clone());
    }
}

impl NirVisitor for Facts {
    fn visit_op(&mut self, op: &NirOp) {
        if let Some((name, type_)) = binder(op) {
            self.note_type(name, &type_);
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

/// The name an op binds, and its type: a `Bind`, a `FOR` / `FOR EACH` loop
/// variable, or a `TRAP`'s error binding.
fn binder(op: &NirOp) -> Option<(&str, ParameterType)> {
    match op {
        NirOp::Bind { name, type_, .. }
        | NirOp::For { name, type_, .. }
        | NirOp::ForEach { name, type_, .. } => Some((name, type_.clone())),
        NirOp::Trap { name, .. } => Some((name, ParameterType::named("Error"))),
        _ => None,
    }
}

/// A binding visible at the op being examined.
#[derive(Clone)]
struct Visible {
    type_: ParameterType,
    param: bool,
    /// Bound from `Capture{by_ref: true}`: reads and writes go through the
    /// parent's slot.
    by_ref: bool,
}

/// The bindings visible at a point in the body, innermost frame last.
#[derive(Default)]
struct Scope {
    frames: Vec<HashMap<String, Visible>>,
}

impl Scope {
    fn open(&mut self) {
        self.frames.push(HashMap::new());
    }

    fn close(&mut self) {
        self.frames.pop();
    }

    fn declare(&mut self, name: &str, visible: Visible) {
        if let Some(frame) = self.frames.last_mut() {
            frame.insert(name.to_string(), visible);
        }
    }

    fn lookup(&self, name: &str) -> Option<&Visible> {
        self.frames.iter().rev().find_map(|frame| frame.get(name))
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
    /// The `STATE` payload of a `RES` binding (`f.state.…`): written by
    /// `StateAssign{f}`, and reachable by anything the handle is passed to.
    State(String),
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
            Owner::State(name) => matches!(
                op,
                NirOp::StateAssign { resource: written, .. } | NirOp::Assign { name: written, .. }
                    if written == name
            ),
        }
    }

    /// The local name the owner is reached through, if any.
    fn local_name(&self) -> Option<&str> {
        match self {
            Owner::Local(name) | Owner::ByRef(name) | Owner::State(name) => Some(name),
            Owner::Global(_) => None,
        }
    }

    /// Whether something other than a visible write could change the owner, so
    /// the window must also be call-safe (W4).
    fn needs_call_gate(&self) -> bool {
        !matches!(self, Owner::Local(_))
    }
}

/// The root of a place, its depth, and the member taken directly on the root
/// (`state` in `f.state.xs`), or `None` when `value` is not a place.
fn place_root(value: &NirValue) -> Option<(&NirValue, usize, Option<&str>)> {
    let mut depth = 0;
    let mut cursor = value;
    let mut first_member = None;
    while let NirValue::MemberAccess { target, member } = cursor {
        depth += 1;
        first_member = Some(member.as_str());
        cursor = target;
    }
    matches!(cursor, NirValue::Local(_) | NirValue::Global { .. }).then_some((
        cursor,
        depth,
        first_member,
    ))
}

/// Find and apply the first forwardable binding in `ops` (or any nested body),
/// tracking the bindings visible at each op in `scope`.
fn forward_one(ops: &mut Vec<NirOp>, context: &Context, scope: &mut Scope) -> bool {
    scope.open();
    let fired = forward_in_block(ops, context, scope);
    scope.close();
    fired
}

fn forward_in_block(ops: &mut Vec<NirOp>, context: &Context, scope: &mut Scope) -> bool {
    for index in 0..ops.len() {
        if let Some((binding, place, last)) = candidate(ops, index, context, scope) {
            for op in &mut ops[index + 1..=last] {
                rewrite_op(op, &binding, &place);
            }
            ops.remove(index);
            return true;
        }
        // A loop variable or a handler's error binding is visible in the body
        // only; a `Bind` is visible to the rest of this block.
        let body_binder = match &ops[index] {
            NirOp::For { .. } | NirOp::ForEach { .. } | NirOp::Trap { .. } => {
                binder(&ops[index]).map(|(name, type_)| (name.to_string(), type_))
            }
            _ => None,
        };
        for body in nested_bodies_mut(&mut ops[index]) {
            scope.open();
            if let Some((name, type_)) = &body_binder {
                scope.declare(
                    name,
                    Visible {
                        type_: type_.clone(),
                        param: false,
                        by_ref: false,
                    },
                );
            }
            let fired = forward_in_block(body, context, scope);
            scope.close();
            if fired {
                return true;
            }
        }
        if let NirOp::Bind {
            name, type_, value, ..
        } = &ops[index]
        {
            scope.declare(
                name,
                Visible {
                    type_: type_.clone(),
                    param: false,
                    by_ref: matches!(value, Some(NirValue::Capture { by_ref: true, .. })),
                },
            );
        }
    }
    false
}

/// `ops[index]` as `(b, place, L)` when every gate holds.
fn candidate(
    ops: &[NirOp],
    index: usize,
    context: &Context,
    scope: &Scope,
) -> Option<(String, NirValue, usize)> {
    let NirOp::Bind {
        mutable: false,
        name: binding,
        type_,
        value: Some(place),
    } = &ops[index]
    else {
        return None;
    };
    if !is_aggregate(type_)
        || context.function_params.contains(binding)
        || context.resource_owners.contains(binding)
    {
        return None;
    }
    let owner = owner_of(place, binding, context, scope)?;

    // Every `Local(b)` in the rest of the block is a read of this `b` unless
    // something there binds `b` again; `L` is the last op with a read.
    let mut last = None;
    for (offset, op) in ops[index + 1..].iter().enumerate() {
        let mut probe = BindingProbe::new(binding);
        probe.visit_op(op);
        if probe.disqualified() {
            return None; // rebound, assigned, address-taken, captured, or W3
        }
        if probe.reads > 0 {
            last = Some(index + 1 + offset);
        }
    }
    let last = last?;
    let window = &ops[index + 1..=last];
    // The owner's name must mean the same binding throughout the window.
    if let Some(name) = owner.local_name() {
        if window.iter().any(|op| binds_name(op, name)) {
            return None;
        }
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
        gate.visit_ops(window);
        if gate.runs_user_code {
            return None;
        }
    }
    // W5: the resource itself goes nowhere in the window. Handing it to
    // anything — `fs::close(f)`, a user `SUB` — can change or end its `STATE`.
    if let Owner::State(resource) = &owner {
        let mut bare = BareUse {
            name: resource,
            found: false,
        };
        bare.visit_ops(window);
        if bare.found {
            return None;
        }
    }
    Some((binding.clone(), place.clone(), last))
}

/// The owner of `place` when it is one this row can prove a window for.
fn owner_of(place: &NirValue, binding: &str, context: &Context, scope: &Scope) -> Option<Owner> {
    let (root, depth, first_member) = place_root(place)?;
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
    let visible = scope.lookup(owner)?;
    let facts = &context.facts;
    if facts.address_taken.contains(owner) || facts.captured.contains(owner) {
        return None;
    }
    let is_resource = context.resource_owners.contains(owner)
        || matches!(
            visible.type_,
            ParameterType::Stateful { .. } | ParameterType::Res(_)
        );
    if is_resource {
        // A resource's only place is its `STATE` payload.
        return (first_member == Some("state")).then(|| Owner::State(owner.clone()));
    }
    // A bare local copied from a parameter keeps the copy (return-move).
    if depth == 0 && visible.param {
        return None;
    }
    if visible.by_ref {
        return Some(Owner::ByRef(owner.clone()));
    }
    Some(Owner::Local(owner.clone()))
}

/// W5's scan: is the resource `name` used other than as the root of a
/// `MemberAccess` chain (`f.state.xs`)?
struct BareUse<'a> {
    name: &'a str,
    found: bool,
}

impl NirVisitor for BareUse<'_> {
    fn visit_value(&mut self, value: &NirValue) {
        match value {
            NirValue::MemberAccess { .. } => {
                let mut cursor = value;
                while let NirValue::MemberAccess { target, .. } = cursor {
                    cursor = target;
                }
                if matches!(cursor, NirValue::Local(root) if root == self.name) {
                    return; // a read through the handle, not the handle itself
                }
                walk_value(self, value);
            }
            NirValue::Local(name) if name == self.name => self.found = true,
            NirValue::LocalRef { name, .. } if name == self.name => self.found = true,
            _ => walk_value(self, value),
        }
    }
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
            || self.context.facts.name_types.contains_key(target)
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
                .name_types
                .get(name)
                .is_none_or(|types| types.iter().any(|type_| module.may_hold_function(type_))),
            NirValue::Capture { type_, .. } => module.may_hold_function(type_),
            // A `Global` read carries no type of its own (lowering leaves it
            // empty); the declaration in `module.globals` has it.
            NirValue::Global { name, .. } => module
                .globals
                .get(name)
                .is_none_or(|type_| module.may_hold_function(type_)),
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

/// Whether `op` or anything nested in it binds `name`.
fn binds_name(op: &NirOp, name: &str) -> bool {
    binder(op).is_some_and(|(bound, _)| bound == name)
        || nested_bodies(op)
            .into_iter()
            .any(|body| body.iter().any(|inner| binds_name(inner, name)))
}

/// What one op does with the binding `b`: how often it reads it, and whether
/// it does anything that disqualifies the forward.
struct BindingProbe<'a> {
    name: &'a str,
    reads: usize,
    handler_depth: usize,
    /// A read inside a `TRAP` handler (W3).
    in_handler: bool,
    /// `b` bound again, assigned, address-taken, or captured.
    misused: bool,
}

impl<'a> BindingProbe<'a> {
    fn new(name: &'a str) -> Self {
        BindingProbe {
            name,
            reads: 0,
            handler_depth: 0,
            in_handler: false,
            misused: false,
        }
    }

    fn disqualified(&self) -> bool {
        self.in_handler || self.misused
    }
}

impl NirVisitor for BindingProbe<'_> {
    fn visit_op(&mut self, op: &NirOp) {
        if binder(op).is_some_and(|(bound, _)| bound == self.name)
            || matches!(op, NirOp::Assign { name, .. } if name == self.name)
        {
            self.misused = true;
        }
        if let NirOp::Trap { .. } = op {
            self.handler_depth += 1;
            walk_op(self, op);
            self.handler_depth -= 1;
            return;
        }
        walk_op(self, op);
    }

    fn visit_value(&mut self, value: &NirValue) {
        match value {
            NirValue::Local(name) if name == self.name => {
                self.reads += 1;
                if self.handler_depth > 0 {
                    self.in_handler = true;
                }
            }
            NirValue::LocalRef { name, .. } if name == self.name => self.misused = true,
            NirValue::Closure { captures, .. }
                if captures.iter().any(
                    |capture| matches!(capture, NirValue::Local(name) if name == self.name),
                ) =>
            {
                self.misused = true;
            }
            _ => {}
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
