//! Which `collections::get`/`getOr` bindings may hold an ALIAS into their
//! container's element instead of an owned copy (plan-86 E, widened by bug-689),
//! and which of those are *element-bound*: updated in place and written back to
//! the slot they came from.
//!
//! `mfb spec language memory-semantics` §14.6 makes every read an owned value;
//! whether a read has to COPY to honour that is the compiler's business. A copy is
//! unobservable to skip exactly when nothing can tell the alias from the copy:
//!
//! * the binding is only read — through fields, or as a `MATCH` scrutinee and
//!   that match's variant extracts — and never stored, returned, passed,
//!   captured or reassigned whole;
//! * nothing writes the container while the binding is live.
//!
//! A borrow that outlives a write to its container is a use-after-free into the
//! container's data region (bug-538/bug-601's class), so every borrow here comes
//! with a proven liveness bound, in one of two forms:
//!
//! * **pinned** (plan-86 E): the container is a local bound at most once, never
//!   reassigned and not address-taken, so no write to it exists at all. `last_use`
//!   additionally refuses to move or hand over anything the initializer reads.
//! * **windowed** (bug-689): the binding's every read lies in the statements
//!   between its `Bind` and the last statement of the same block that reads it,
//!   and none of those statements can write the container. For a local container
//!   that means no statement there rebinds or reassigns it, and every read of it
//!   there is a lookup (`get`/`getOr`/`len`'s first argument, or a `FOR EACH`
//!   iterable) — so no move, hand-over or in-place update can reach its block.
//!   For a module-level container it means no call there can reach a
//!   `StoreGlobal` of it (`global_written`).
//!
//! An **element-bound** binding (bug-689 Phase 3) is the `get → WITH → set`
//! element update:
//!
//! ```text
//! MUT p = collections::get(xs, i)          ' S0
//! …reads of p's fields…                    ' R
//! p = WITH p { … }                         ' S1 (optional)
//! xs = collections::set(xs, i, p)          ' W
//! ```
//!
//! `p` aliases `xs[i]`; S1 updates the element where it lies and W writes nothing.
//! That is only unobservable because S1 and W are ADJACENT (nothing can see `xs`
//! between them), `i` is a pure expression over locals none of which change
//! before W (so W names the same slot S0 read), `p` is dead after W, and a failed
//! S1 leaves both `p` and the element unchanged (the field routes are failure
//! atomic). The lowering refuses every route that would grow the element's block
//! (`CodeBuilder::element_field_owner`).
//!
//! bug-766 admits three more spellings of the same S1+W pair, each held to the
//! same rules:
//!
//! * **fused** — `xs = set(xs, i, WITH p { … })`: S1 and W are one statement (the
//!   reads of `p` in the `WITH` are S1's), so adjacency holds trivially, and `p`
//!   may be a `LET`. The statement is the update and has no separate write-back.
//! * **through a temporary** — `LET u = WITH p { … }` then `xs = set(xs, i, u)`,
//!   `u` bound once, never reassigned and read only there: the bind is S1 under a
//!   new name, and `u` is never materialised.
//! * **in every arm of an `IF`** — the statement after the window is an `IF` with
//!   an `ELSE` whose every arm ends in a write-back of the same slot (an arm may
//!   itself end in such an `IF`); each arm is checked with the per-block rule,
//!   and the condition like a window statement.
//!
//! A **scratch copy** (bug-766 B) is a `get` binding that is none of the above
//! but never escapes: every read is a field read or the target of a `WITH` that
//! is the item of a `set`/`append` — both of which copy what they read. It must
//! still be a copy (the container may be written while it is live: `bj` of an
//! n-body pair step must not see the write to slot `i` when `i = j`), but a copy
//! that nothing can observe outliving its binding. The builder places it in the
//! function's record-temp scratch instead of a fresh arena block when its type is
//! a fixed-size record (`CodeBuilder::install_borrow_gets`).

use crate::target::shared::nir::visit::{walk_op, walk_value, NirVisitor};
use crate::target::shared::nir::{NirMatchPattern, NirOp, NirValue};
use crate::types::ParameterType;
use std::collections::{HashMap, HashSet};

mod by_name;
mod predicates;

use by_name::collect_borrow_gets_by_name;
use predicates::*;
pub(crate) use predicates::{
    bind_single_expression_element, pure_index, same_index, single_expression_update,
};

/// The container a borrowed `get` reads: a local, or a module-level global.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ContainerRef {
    Local(String),
    Global(String),
}

impl ContainerRef {
    /// The container `value` names, if it is a plain local or global read.
    pub(crate) fn of(value: &NirValue) -> Option<ContainerRef> {
        match value {
            NirValue::Local(name) => Some(ContainerRef::Local(name.clone())),
            NirValue::Global { name, .. } => Some(ContainerRef::Global(name.clone())),
            _ => None,
        }
    }

    /// Whether `value` is a read of this container.
    pub(crate) fn is(&self, value: &NirValue) -> bool {
        matches!(
            (self, value),
            (ContainerRef::Local(a), NirValue::Local(b)) if a == b
        ) || matches!(
            (self, value),
            (ContainerRef::Global(a), NirValue::Global { name: b, .. }) if a == b
        )
    }
}

/// An element-bound binding: `p = get(container, index)` updated in place and
/// written back to the same slot.
#[derive(Clone)]
pub(crate) struct ElementBinding {
    /// The binding's declared type — the element type.
    pub(crate) type_: ParameterType,
    /// The write-back statement `container = set(container, index, …)` as the
    /// source wrote it, for the copying fallback.
    pub(crate) template: NirOp,
    /// The S1 statement (`op_key`): `p = WITH p { … }` or `LET u = WITH p { … }`
    /// immediately before the write-back, or a fused write-back itself; `None`
    /// when the element is written back unchanged.
    pub(crate) update: Option<usize>,
    /// `container = set(container, index, p)` or `(…, u)` (`op_key`); `None` for
    /// a single-expression or fused update, which is its own write-back.
    pub(crate) write_back: Option<usize>,
    /// bug-766: the sites in the other arms, when the write-back is in every arm
    /// of an `IF` (the fields above are the first arm's).
    pub(crate) other_sites: Vec<ElementSite>,
}

/// One update/write-back site of an [`ElementBinding`] — see its fields.
#[derive(Clone)]
pub(crate) struct ElementSite {
    pub(crate) template: NirOp,
    pub(crate) update: Option<usize>,
    pub(crate) write_back: Option<usize>,
}

impl ElementBinding {
    /// `container = set(container, index, item)` — the statement the update
    /// `update` (`op_key`; `None` for a single-expression update) stands for,
    /// lowered as written when no in-place route serves it.
    pub(crate) fn write_back_op(&self, update: Option<usize>, item: NirValue) -> NirOp {
        let template = self
            .other_sites
            .iter()
            .find(|site| update.is_some() && site.update == update)
            .map_or(&self.template, |site| &site.template);
        let mut op = template.clone();
        let value = match &mut op {
            NirOp::Assign { value, .. } => value,
            NirOp::StoreGlobal {
                value: Some(value), ..
            } => value,
            _ => unreachable!("an element write-back is an Assign or a StoreGlobal"),
        };
        if let NirValue::Call { args, .. } = value {
            args[2] = item;
        }
        op
    }

    /// Every site's `(update, write_back)`: the first, then [`Self::other_sites`].
    pub(crate) fn sites(&self) -> impl Iterator<Item = (Option<usize>, Option<usize>)> + '_ {
        std::iter::once((self.update, self.write_back)).chain(
            self.other_sites
                .iter()
                .map(|site| (site.update, site.write_back)),
        )
    }
}

/// The `WITH` an element update statement registered in
/// [`ElementBinding::update`] applies: the statement's own value for
/// `p = WITH p { … }` / `LET u = WITH p { … }`, the item of a fused
/// `container = set(container, i, WITH p { … })`.
pub(crate) fn element_update_with(value: &NirValue) -> &NirValue {
    match value {
        NirValue::Call { target, args, .. }
            if builtin(target) == Some("set")
                && args.len() == 3
                && matches!(args[2], NirValue::WithUpdate { .. }) =>
        {
            &args[2]
        }
        _ => value,
    }
}

/// The result of [`collect_borrow_gets`].
#[derive(Default)]
pub(crate) struct BorrowGets {
    /// Every binding whose initializer is lowered as an alias (and never freed).
    pub(crate) names: HashSet<String>,
    /// The plan-86 E subset whose container is immutable for the whole function:
    /// `last_use` refuses to move or hand over anything their initializer reads.
    pub(crate) pinned: HashSet<String>,
    /// The element-bound subset, by binding name.
    pub(crate) elements: HashMap<String, ElementBinding>,
    /// bug-766 B: the `get` bindings (not in `names`) that are still copies but
    /// never escape — read only through fields or as the target of a `WITH`
    /// that is a `set`/`append` item — with their declared type.
    pub(crate) scratch: HashMap<String, ParameterType>,
    /// bug-784: per `Bind` (`op_key`) of a name the function binds more than once
    /// (in disjoint scopes — a local is never redeclared while live), that one
    /// binding's classification. The name-keyed fields above say nothing about
    /// such a name except that it is in `names`/`pinned` when ANY of its bindings
    /// borrows (which is all `last_use` asks: a name to exclude); the lowering
    /// switches the name's role at each of these binds.
    pub(crate) rebound: HashMap<usize, Rebound>,
}

/// bug-784: one binding of a name bound more than once in the function.
#[derive(Clone)]
pub(crate) struct Rebound {
    /// The binding's (source) name.
    pub(crate) name: String,
    pub(crate) role: ReboundRole,
}

/// bug-784: how one binding of a re-bound name is lowered.
#[derive(Clone)]
pub(crate) enum ReboundRole {
    /// An owned copy (or the name's scratch copy, `BorrowGets::scratch`).
    Copy,
    /// A borrow (`BorrowGets::names`), pinned or windowed.
    Borrow,
    /// An element-bound binding (boxed: it carries write-back templates).
    Element(Box<ElementBinding>),
}

impl BorrowGets {
    /// Every element-bound binding — by name, and (bug-784) each one of a
    /// re-bound name — with its source name.
    pub(crate) fn element_bindings(&self) -> impl Iterator<Item = (&str, &ElementBinding)> {
        self.elements
            .iter()
            .map(|(name, binding)| (name.as_str(), binding))
            .chain(
                self.rebound
                    .values()
                    .filter_map(|rebound| match &rebound.role {
                        ReboundRole::Element(binding) => {
                            Some((rebound.name.as_str(), binding.as_ref()))
                        }
                        _ => None,
                    }),
            )
    }
}

/// bug-766 B: the item of a `set(c, k, item)` / `append(c, item)` call — the
/// operand those builtins copy into the container, or `None` for any other value.
pub(crate) fn copied_item(value: &NirValue) -> Option<&NirValue> {
    match value {
        NirValue::Call { target, args, .. } => match (builtin(target), args.len()) {
            (Some("set"), 3) => Some(&args[2]),
            (Some("append"), 2) => Some(&args[1]),
            _ => None,
        },
        _ => None,
    }
}

/// See the module doc. `global_written(ops, g)` answers whether running `ops` can
/// reach a `StoreGlobal` of `g`; a caller that cannot answer may say `false`
/// ONLY if it uses the result solely to exclude names (the answer then describes
/// a superset of the real borrows, which is the safe direction for exclusion).
///
/// bug-784: the rules below reason about THE binding of a name, so a name bound
/// twice — `LET h = get(…)` in one loop and again in a later one, or in both
/// arms of an `IF` — used to disqualify every binding of it. A local is never
/// redeclared while it is live (`mfb spec language bindings-and-scope`, "No
/// shadowing of an in-scope local"), so such bindings have disjoint scopes and
/// each is a binding of its own. The classification therefore runs on a copy of
/// the body in which every binding of a re-bound name — `Bind`, `FOR`/`FOR EACH`
/// variable, `TRAP` name — gets a unique name and every read resolves to the
/// binding in scope (`rename_ops`); the results are mapped back to the real
/// ops (`op_key`s, write-back templates) and, for a re-bound name, reported per
/// binding in [`BorrowGets::rebound`]. A function with no re-bound name is
/// classified exactly as before.
pub(crate) fn collect_borrow_gets(
    ops: &[NirOp],
    address_taken: &HashSet<String>,
    global_written: &dyn Fn(&[NirOp], &str) -> bool,
) -> BorrowGets {
    let rebound = rebound_names(ops);
    let by_name = collect_borrow_gets_by_name(ops, address_taken, global_written);
    if rebound.is_empty() {
        return by_name;
    }
    let mut renamed = ops.to_vec();
    let mut state = RenameState {
        rebound: &rebound,
        scopes: vec![HashMap::new()],
        next: 0,
        origin: HashMap::new(),
    };
    rename_ops(&mut renamed, &mut state);
    let origin = state.origin;
    // A renamed binding is address-taken when its name is.
    let mut renamed_address_taken = address_taken.clone();
    for (unique, name) in &origin {
        if address_taken.contains(name) {
            renamed_address_taken.insert(unique.clone());
        }
    }
    let scoped = collect_borrow_gets_by_name(&renamed, &renamed_address_taken, global_written);
    // The copy's ops, by `op_key`, to the real ones they were cloned from.
    let mut real: HashMap<usize, &NirOp> = HashMap::new();
    pair_ops(ops, &renamed, &mut real);
    let real_key = |key: usize| real.get(&key).map(|op| op_key(op));
    let real_binding = |binding: &ElementBinding| -> Option<ElementBinding> {
        let real_site = |template_key: usize, update: Option<usize>, write_back: Option<usize>| {
            Some((
                (*real.get(&template_key)?).clone(),
                match update {
                    Some(key) => Some(real_key(key)?),
                    None => None,
                },
                match write_back {
                    Some(key) => Some(real_key(key)?),
                    None => None,
                },
            ))
        };
        let (template, update, write_back) = real_site(
            binding.write_back.or(binding.update)?,
            binding.update,
            binding.write_back,
        )?;
        let mut other_sites = Vec::with_capacity(binding.other_sites.len());
        for site in &binding.other_sites {
            let (template, update, write_back) = real_site(
                site.write_back.or(site.update)?,
                site.update,
                site.write_back,
            )?;
            other_sites.push(ElementSite {
                template,
                update,
                write_back,
            });
        }
        Some(ElementBinding {
            type_: binding.type_.clone(),
            template,
            update,
            write_back,
            other_sites,
        })
    };
    let mut out = BorrowGets {
        scratch: by_name.scratch,
        ..BorrowGets::default()
    };
    // Bindings that kept their name: exactly as classified (a re-bound container
    // or index local may have made one more of them borrowable).
    for name in &scoped.names {
        if !origin.contains_key(name) {
            out.names.insert(name.clone());
            out.scratch.remove(name);
        }
    }
    for name in &scoped.pinned {
        if !origin.contains_key(name) {
            out.pinned.insert(name.clone());
        }
    }
    for (name, binding) in &scoped.elements {
        if origin.contains_key(name) {
            continue;
        }
        match real_binding(binding) {
            Some(binding) => {
                out.elements.insert(name.clone(), binding);
            }
            // Unmappable (cannot happen: the copy has the real ops' shape) —
            // the binding is then a copy.
            None => {
                out.names.remove(name);
                out.pinned.remove(name);
            }
        }
    }
    // Each binding of a re-bound name, by its real `Bind`.
    let mut roles: Vec<(usize, Rebound)> = Vec::new();
    for_each_bind(&renamed, &mut |op| {
        let NirOp::Bind { name: unique, .. } = op else {
            return;
        };
        let Some(name) = origin.get(unique) else {
            return;
        };
        let Some(key) = real_key(op_key(op)) else {
            return;
        };
        let role = match scoped.elements.get(unique).and_then(&real_binding) {
            Some(binding) => ReboundRole::Element(Box::new(binding)),
            None if scoped.names.contains(unique) && !scoped.elements.contains_key(unique) => {
                ReboundRole::Borrow
            }
            None => ReboundRole::Copy,
        };
        if !matches!(role, ReboundRole::Copy) {
            out.names.insert(name.clone());
            if scoped.pinned.contains(unique) {
                out.pinned.insert(name.clone());
            }
        }
        roles.push((
            key,
            Rebound {
                name: name.clone(),
                role,
            },
        ));
    });
    // A name none of whose bindings borrows needs no per-binding switch.
    let borrowing: HashSet<String> = roles
        .iter()
        .filter(|(_, rebound)| !matches!(rebound.role, ReboundRole::Copy))
        .map(|(_, rebound)| rebound.name.clone())
        .collect();
    out.rebound = roles
        .into_iter()
        .filter(|(_, rebound)| borrowing.contains(&rebound.name))
        .collect();
    out
}

/// bug-784: the names `ops` binds more than once — `Bind`, `FOR`/`FOR EACH`
/// variables, `TRAP` names.
fn rebound_names(ops: &[NirOp]) -> HashSet<String> {
    struct Count(HashMap<String, usize>);
    impl NirVisitor for Count {
        fn visit_op(&mut self, op: &NirOp) {
            match op {
                NirOp::Bind { name, .. }
                | NirOp::For { name, .. }
                | NirOp::ForEach { name, .. }
                | NirOp::Trap { name, .. } => *self.0.entry(name.clone()).or_insert(0) += 1,
                _ => {}
            }
            walk_op(self, op);
        }
    }
    let mut count = Count(HashMap::new());
    count.visit_ops(ops);
    count
        .0
        .into_iter()
        .filter(|(_, n)| *n > 1)
        .map(|(name, _)| name)
        .collect()
}

/// bug-784: the state of [`rename_ops`].
struct RenameState<'a> {
    rebound: &'a HashSet<String>,
    /// One map per open block: a re-bound name → the unique name of the binding
    /// of it in scope there.
    scopes: Vec<HashMap<String, String>>,
    next: usize,
    /// Unique name → source name.
    origin: HashMap<String, String>,
}

impl RenameState<'_> {
    /// Give a binding of `name` a unique name if `name` is re-bound. `@` never
    /// appears in a source or compiler-internal local name.
    fn bind(&mut self, name: &mut String) {
        if !self.rebound.contains(name.as_str()) {
            return;
        }
        let unique = format!("{name}@{}", self.next);
        self.next += 1;
        self.origin.insert(unique.clone(), name.clone());
        self.scopes
            .last_mut()
            .expect("a scope is open")
            .insert(name.clone(), unique.clone());
        *name = unique;
    }

    /// The binding a read of `name` sees: the innermost open one.
    fn resolve(&self, name: &mut String) {
        if !self.rebound.contains(name.as_str()) {
            return;
        }
        if let Some(unique) = self
            .scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name.as_str()))
        {
            *name = unique.clone();
        }
    }

    fn block(&mut self, ops: &mut [NirOp]) {
        self.scopes.push(HashMap::new());
        rename_ops(ops, self);
        self.scopes.pop();
    }
}

/// bug-784: rename every binding of a re-bound name in `ops` (in place, in the
/// current scope) and every read of one, following the resolver's scoping: a
/// block's bindings are visible to its later statements and its nested blocks;
/// `IF`/`ELSE` arms, loop bodies, `MATCH` cases and `TRAP` bodies are nested
/// blocks; a loop variable or `TRAP` name is bound in its body's scope.
fn rename_ops(ops: &mut [NirOp], state: &mut RenameState<'_>) {
    for op in ops {
        match op {
            NirOp::Bind { name, value, .. } => {
                if let Some(value) = value {
                    rename_value(value, state);
                }
                state.bind(name);
            }
            NirOp::StoreGlobal { value, .. } => {
                if let Some(value) = value {
                    rename_value(value, state);
                }
            }
            NirOp::Assign { name, value } => {
                rename_value(value, state);
                state.resolve(name);
            }
            NirOp::StateAssign { resource, value } => {
                rename_value(value, state);
                state.resolve(resource);
            }
            NirOp::Return { value } => {
                if let Some(value) = value {
                    rename_value(value, state);
                }
            }
            NirOp::ExitLoop { .. } | NirOp::ContinueLoop { .. } => {}
            NirOp::ExitProgram { code } => rename_value(code, state),
            NirOp::Fail { error } => rename_value(error, state),
            NirOp::Eval { value } => rename_value(value, state),
            NirOp::If {
                condition,
                then_body,
                else_body,
            } => {
                rename_value(condition, state);
                state.block(then_body);
                state.block(else_body);
            }
            NirOp::Match { value, cases } => {
                rename_value(value, state);
                for case in cases {
                    state.scopes.push(HashMap::new());
                    match &mut case.pattern {
                        NirMatchPattern::Else => {}
                        NirMatchPattern::Value(value) => rename_value(value, state),
                        NirMatchPattern::OneOf(values) => {
                            for value in values {
                                rename_value(value, state);
                            }
                        }
                    }
                    rename_ops(&mut case.body, state);
                    if let Some(guard) = &mut case.guard {
                        rename_value(guard, state);
                    }
                    state.scopes.pop();
                }
            }
            NirOp::While {
                condition, body, ..
            } => {
                rename_value(condition, state);
                state.block(body);
            }
            NirOp::For {
                name,
                start,
                end,
                step,
                body,
                ..
            } => {
                rename_value(start, state);
                rename_value(end, state);
                rename_value(step, state);
                state.scopes.push(HashMap::new());
                state.bind(name);
                rename_ops(body, state);
                state.scopes.pop();
            }
            NirOp::DoUntil { body, condition } => {
                state.block(body);
                rename_value(condition, state);
            }
            NirOp::ForEach {
                name,
                iterable,
                body,
                ..
            } => {
                rename_value(iterable, state);
                state.scopes.push(HashMap::new());
                state.bind(name);
                rename_ops(body, state);
                state.scopes.pop();
            }
            NirOp::Trap { name, body } => {
                state.scopes.push(HashMap::new());
                state.bind(name);
                rename_ops(body, state);
                state.scopes.pop();
            }
        }
    }
}

/// bug-784: [`rename_ops`] for a value: every read of a local resolves to the
/// binding in scope. Exhaustive, so a new variant cannot be skipped silently.
fn rename_value(value: &mut NirValue, state: &mut RenameState<'_>) {
    match value {
        NirValue::Local(name) | NirValue::LocalRef { name, .. } => state.resolve(name),
        NirValue::Const { .. }
        | NirValue::Global { .. }
        | NirValue::FunctionRef { .. }
        | NirValue::Capture { .. } => {}
        NirValue::Closure { captures, .. } => {
            for capture in captures {
                rename_value(capture, state);
            }
        }
        NirValue::Call { args, .. }
        | NirValue::CallResult { args, .. }
        | NirValue::RuntimeCall { args, .. }
        | NirValue::Constructor { args, .. } => {
            for arg in args {
                rename_value(arg, state);
            }
        }
        NirValue::Checked { value, .. }
        | NirValue::UnionWrap { value, .. }
        | NirValue::UnionExtract { value, .. }
        | NirValue::ResultIsOk { value }
        | NirValue::ResultValue { value }
        | NirValue::ResultError { value } => rename_value(value, state),
        NirValue::WithUpdate {
            target, updates, ..
        } => {
            rename_value(target, state);
            for update in updates {
                rename_value(&mut update.value, state);
            }
        }
        NirValue::ListLiteral { values, .. } | NirValue::SetLiteral { values, .. } => {
            for value in values {
                rename_value(value, state);
            }
        }
        NirValue::MapLiteral { entries, .. } => {
            for (key, value) in entries {
                rename_value(key, state);
                rename_value(value, state);
            }
        }
        NirValue::MemberAccess { target, .. } => rename_value(target, state),
        NirValue::Binary { left, right, .. } => {
            rename_value(left, state);
            rename_value(right, state);
        }
        NirValue::Unary { operand, .. } => rename_value(operand, state),
    }
}

/// bug-784: pair each op of `copy` (by `op_key`) with the op of `real` it was
/// cloned from — the two trees have the same shape.
fn pair_ops<'r>(real: &'r [NirOp], copy: &[NirOp], out: &mut HashMap<usize, &'r NirOp>) {
    for (real_op, copy_op) in real.iter().zip(copy) {
        out.insert(op_key(copy_op), real_op);
        let pairs: Vec<(&'r [NirOp], &[NirOp])> = match (real_op, copy_op) {
            (
                NirOp::If {
                    then_body: a,
                    else_body: b,
                    ..
                },
                NirOp::If {
                    then_body: c,
                    else_body: d,
                    ..
                },
            ) => vec![(a, c), (b, d)],
            (NirOp::Match { cases: a, .. }, NirOp::Match { cases: b, .. }) => a
                .iter()
                .zip(b)
                .map(|(a, b)| (a.body.as_slice(), b.body.as_slice()))
                .collect(),
            (NirOp::While { body: a, .. }, NirOp::While { body: b, .. })
            | (NirOp::For { body: a, .. }, NirOp::For { body: b, .. })
            | (NirOp::DoUntil { body: a, .. }, NirOp::DoUntil { body: b, .. })
            | (NirOp::ForEach { body: a, .. }, NirOp::ForEach { body: b, .. })
            | (NirOp::Trap { body: a, .. }, NirOp::Trap { body: b, .. }) => vec![(a, b)],
            _ => Vec::new(),
        };
        for (a, b) in pairs {
            pair_ops(a, b, out);
        }
    }
}

/// bug-784: every `Bind` in `ops`, at any depth.
fn for_each_bind(ops: &[NirOp], f: &mut dyn FnMut(&NirOp)) {
    struct Binds<'f>(&'f mut dyn FnMut(&NirOp));
    impl NirVisitor for Binds<'_> {
        fn visit_op(&mut self, op: &NirOp) {
            if matches!(op, NirOp::Bind { .. }) {
                (self.0)(op);
            }
            walk_op(self, op);
        }
    }
    Binds(f).visit_ops(ops);
}

#[cfg(test)]
mod tests;
