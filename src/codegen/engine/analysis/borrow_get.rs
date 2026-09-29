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
use crate::target::shared::nir::{NirOp, NirValue};
use crate::types::ParameterType;
use std::collections::{HashMap, HashSet};

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

/// `op as *const NirOp as usize` — the same identity `last_use::op_key` uses.
fn op_key(op: &NirOp) -> usize {
    op as *const NirOp as usize
}

/// The bare builtin a call target names (`collections.get` → `get`).
fn builtin(target: &str) -> Option<&'static str> {
    crate::codegen::builtins::native_builtin_target(target)
}

/// A `get`/`getOr` call: `(bare name, args)`.
fn get_call(value: &NirValue) -> Option<(&'static str, &[NirValue])> {
    match value {
        NirValue::Call { target, args, .. } => match builtin(target) {
            Some(name @ ("get" | "getOr")) => Some((name, args.as_slice())),
            _ => None,
        },
        _ => None,
    }
}

/// An index expression W may re-evaluate and get the slot S0 read: constants and
/// locals under arithmetic. Arithmetic can fail, but it already succeeded on the
/// same inputs at S0, and the inputs are proven unchanged.
pub(crate) fn pure_index(value: &NirValue) -> bool {
    match value {
        NirValue::Const { .. } | NirValue::Local(_) => true,
        NirValue::Binary { left, right, .. } => pure_index(left) && pure_index(right),
        NirValue::Unary { operand, .. } => pure_index(operand),
        _ => false,
    }
}

/// Structural equality over [`pure_index`] shapes (source locations ignored).
pub(crate) fn same_index(a: &NirValue, b: &NirValue) -> bool {
    match (a, b) {
        (
            NirValue::Const {
                type_: ta,
                value: va,
            },
            NirValue::Const {
                type_: tb,
                value: vb,
            },
        ) => ta == tb && va == vb,
        (NirValue::Local(a), NirValue::Local(b)) => a == b,
        (
            NirValue::Binary {
                op: oa,
                left: la,
                right: ra,
                ..
            },
            NirValue::Binary {
                op: ob,
                left: lb,
                right: rb,
                ..
            },
        ) => oa == ob && same_index(la, lb) && same_index(ra, rb),
        (
            NirValue::Unary {
                op: oa,
                operand: xa,
                ..
            },
            NirValue::Unary {
                op: ob,
                operand: xb,
                ..
            },
        ) => oa == ob && same_index(xa, xb),
        _ => false,
    }
}

/// The locals an index expression reads.
fn index_locals(value: &NirValue, out: &mut HashSet<String>) {
    match value {
        NirValue::Local(name) => {
            out.insert(name.clone());
        }
        NirValue::Binary { left, right, .. } => {
            index_locals(left, out);
            index_locals(right, out);
        }
        NirValue::Unary { operand, .. } => index_locals(operand, out),
        _ => {}
    }
}

/// `container = set(container, index, item)`: `Some((index, item))` when `op`
/// has that shape.
fn write_back_of<'o>(
    op: &'o NirOp,
    container: &ContainerRef,
) -> Option<(&'o NirValue, &'o NirValue)> {
    let value = match (op, container) {
        (NirOp::Assign { name, value }, ContainerRef::Local(c)) if name == c => value,
        (
            NirOp::StoreGlobal {
                name,
                value: Some(value),
                ..
            },
            ContainerRef::Global(c),
        ) if name == c => value,
        _ => return None,
    };
    let NirValue::Call { target, args, .. } = value else {
        return None;
    };
    if builtin(target) != Some("set") || args.len() != 3 || !container.is(&args[0]) {
        return None;
    }
    Some((&args[1], &args[2]))
}

/// `WITH item { … }` — a `WITH` over the local `item`.
fn is_with_over(value: &NirValue, item: &str) -> bool {
    matches!(value, NirValue::WithUpdate { target, .. }
        if matches!(target.as_ref(), NirValue::Local(t) if t == item))
}

/// Every op list in `ops`, depth-first: the top level and each nested body.
fn for_each_block<'o>(ops: &'o [NirOp], f: &mut dyn FnMut(&'o [NirOp])) {
    f(ops);
    for op in ops {
        match op {
            NirOp::If {
                then_body,
                else_body,
                ..
            } => {
                for_each_block(then_body, f);
                for_each_block(else_body, f);
            }
            NirOp::Match { cases, .. } => {
                for case in cases {
                    for_each_block(&case.body, f);
                }
            }
            NirOp::While { body, .. }
            | NirOp::For { body, .. }
            | NirOp::DoUntil { body, .. }
            | NirOp::ForEach { body, .. }
            | NirOp::Trap { body, .. } => for_each_block(body, f),
            NirOp::Bind { .. }
            | NirOp::StoreGlobal { .. }
            | NirOp::Assign { .. }
            | NirOp::StateAssign { .. }
            | NirOp::Return { .. }
            | NirOp::ExitLoop { .. }
            | NirOp::ContinueLoop { .. }
            | NirOp::ExitProgram { .. }
            | NirOp::Fail { .. }
            | NirOp::Eval { .. } => {}
        }
    }
}

/// How `ops` read the names in `names`: every read, the reads that are the
/// target of a field access, and the reads that are a `WITH`'s target.
#[derive(Default)]
struct Reads {
    all: usize,
    member: usize,
    with_target: usize,
}

fn reads_in(ops: &[NirOp], names: &HashSet<String>) -> Reads {
    struct Count<'n> {
        names: &'n HashSet<String>,
        reads: Reads,
    }
    impl NirVisitor for Count<'_> {
        fn visit_value(&mut self, value: &NirValue) {
            match value {
                NirValue::Local(name) if self.names.contains(name) => self.reads.all += 1,
                NirValue::MemberAccess { target, .. } => {
                    if matches!(target.as_ref(), NirValue::Local(name) if self.names.contains(name))
                    {
                        self.reads.member += 1;
                    }
                }
                NirValue::WithUpdate { target, .. } => {
                    if matches!(target.as_ref(), NirValue::Local(name) if self.names.contains(name))
                    {
                        self.reads.with_target += 1;
                    }
                }
                _ => {}
            }
            walk_value(self, value);
        }
    }
    let mut count = Count {
        names,
        reads: Reads::default(),
    };
    count.visit_ops(ops);
    count.reads
}

/// Whether any op in `ops` (re)binds or assigns one of `names` — a `Bind`,
/// `Assign`, `STATE` assignment, or a loop / `TRAP` variable.
fn writes_any(ops: &[NirOp], names: &HashSet<String>) -> bool {
    struct Find<'n> {
        names: &'n HashSet<String>,
        found: bool,
    }
    impl NirVisitor for Find<'_> {
        fn visit_op(&mut self, op: &NirOp) {
            match op {
                NirOp::Bind { name, .. }
                | NirOp::Assign { name, .. }
                | NirOp::For { name, .. }
                | NirOp::ForEach { name, .. }
                | NirOp::Trap { name, .. } => {
                    if self.names.contains(name) {
                        self.found = true;
                    }
                }
                NirOp::StateAssign { resource, .. } => {
                    if self.names.contains(resource) {
                        self.found = true;
                    }
                }
                _ => {}
            }
            walk_op(self, op);
        }
    }
    let mut find = Find {
        names,
        found: false,
    };
    find.visit_ops(ops);
    find.found
}

/// Whether every read of the local `container` in `ops` is a lookup: the first
/// argument of `get`/`getOr`/`len`, or a `FOR EACH` iterable. Any other read —
/// a store, an argument, a builtin that may update it in place at its last use —
/// could move or mutate the block a borrow points into.
fn local_reads_are_lookups(ops: &[NirOp], container: &str) -> bool {
    struct Check<'c> {
        container: &'c str,
        ok: bool,
    }
    impl Check<'_> {
        fn is_container(&self, value: &NirValue) -> bool {
            matches!(value, NirValue::Local(name) if name == self.container)
        }
    }
    impl NirVisitor for Check<'_> {
        fn visit_op(&mut self, op: &NirOp) {
            if let NirOp::ForEach { iterable, body, .. } = op {
                if !self.is_container(iterable) {
                    self.visit_value(iterable);
                }
                self.visit_ops(body);
                return;
            }
            walk_op(self, op);
        }
        fn visit_value(&mut self, value: &NirValue) {
            match value {
                // `len` is the unqualified general builtin, which the registry's
                // collections lookup does not name.
                NirValue::Call { target, args, .. }
                    if (target == "len" || matches!(builtin(target), Some("get" | "getOr")))
                        && args.first().is_some_and(|arg| self.is_container(arg)) =>
                {
                    for arg in &args[1..] {
                        self.visit_value(arg);
                    }
                }
                NirValue::Local(name) | NirValue::LocalRef { name, .. }
                    if name == self.container =>
                {
                    self.ok = false;
                }
                _ => walk_value(self, value),
            }
        }
    }
    let mut check = Check {
        container,
        ok: true,
    };
    check.visit_ops(ops);
    check.ok
}

/// Every read of `container` in `ops` leaves its block where it is (see the
/// module doc): a local is neither rebound nor read other than by a lookup; a
/// global is not stored to by anything `ops` can run.
fn container_untouched(
    ops: &[NirOp],
    container: &ContainerRef,
    global_written: &dyn Fn(&[NirOp], &str) -> bool,
) -> bool {
    match container {
        ContainerRef::Local(name) => {
            !writes_any(ops, &HashSet::from([name.clone()])) && local_reads_are_lookups(ops, name)
        }
        ContainerRef::Global(name) => !global_written(ops, name),
    }
}

/// `value` is `set(container, index, WITH get(container, index) { … })` — the
/// single-expression element update — with a pure `index` and a plain local or
/// global container: `(container, index, the WITH)`.
pub(crate) fn single_expression_update(
    value: &NirValue,
) -> Option<(ContainerRef, &NirValue, &NirValue)> {
    let NirValue::Call { target, args, .. } = value else {
        return None;
    };
    if builtin(target) != Some("set") || args.len() != 3 {
        return None;
    }
    let container = ContainerRef::of(&args[0])?;
    let with = &args[2];
    let NirValue::WithUpdate { target, .. } = with else {
        return None;
    };
    let (kind, get_args) = get_call(target)?;
    (kind == "get"
        && get_args.len() == 2
        && container.is(&get_args[0])
        && pure_index(&args[1])
        && same_index(&args[1], &get_args[1]))
    .then_some((container, &args[1], with))
}

/// `get(container, i)` for an `i` that is [`same_index`] as `index`.
fn is_element_get(value: &NirValue, container: &ContainerRef, index: &NirValue) -> bool {
    get_call(value).is_some_and(|(kind, args)| {
        kind == "get" && args.len() == 2 && container.is(&args[0]) && same_index(&args[1], index)
    })
}

/// Every child value of `value`, mutably. Exhaustive, so a new `NirValue`
/// variant is a build error here rather than a missed occurrence.
fn children_mut(value: &mut NirValue, f: &mut dyn FnMut(&mut NirValue)) {
    match value {
        NirValue::Const { .. }
        | NirValue::Local(_)
        | NirValue::LocalRef { .. }
        | NirValue::Global { .. }
        | NirValue::FunctionRef { .. }
        | NirValue::Capture { .. } => {}
        NirValue::Closure { captures: args, .. }
        | NirValue::Call { args, .. }
        | NirValue::CallResult { args, .. }
        | NirValue::RuntimeCall { args, .. }
        | NirValue::Constructor { args, .. }
        | NirValue::ListLiteral { values: args, .. }
        | NirValue::SetLiteral { values: args, .. } => args.iter_mut().for_each(f),
        NirValue::MapLiteral { entries, .. } => {
            for (key, value) in entries {
                f(key);
                f(value);
            }
        }
        NirValue::UnionWrap { value, .. }
        | NirValue::UnionExtract { value, .. }
        | NirValue::ResultIsOk { value }
        | NirValue::ResultValue { value }
        | NirValue::ResultError { value }
        | NirValue::Checked { value, .. }
        | NirValue::MemberAccess { target: value, .. } => f(value),
        NirValue::WithUpdate {
            target, updates, ..
        } => {
            f(target);
            for update in updates {
                f(&mut update.value);
            }
        }
        NirValue::Binary { left, right, .. } => {
            f(left);
            f(right);
        }
        NirValue::Unary { operand, .. } => f(operand),
    }
}

/// The single-expression update's `WITH` with every `get(container, index)` in it
/// replaced by `Local(element)` — the element-bound binding the lowering holds
/// the element's address in. `None` unless every such `get` other than the
/// `WITH`'s own target is the target of a field read (anything else would pass
/// the alias somewhere that may keep, free or hand it over), and every other
/// read of a local container is a lookup.
pub(crate) fn bind_single_expression_element(
    with: &NirValue,
    container: &ContainerRef,
    index: &NirValue,
    element: &str,
) -> Option<NirValue> {
    let NirValue::WithUpdate {
        type_,
        target: _,
        updates,
    } = with
    else {
        return None;
    };
    // Field reads of the element become reads of `element`.
    fn rewrite(
        value: &mut NirValue,
        container: &ContainerRef,
        index: &NirValue,
        element: &str,
        ok: &mut bool,
    ) {
        if let NirValue::MemberAccess { target, .. } = value {
            if is_element_get(target, container, index) {
                **target = NirValue::Local(element.to_string());
                return;
            }
        }
        if is_element_get(value, container, index) {
            *ok = false;
            return;
        }
        children_mut(value, &mut |child| {
            rewrite(child, container, index, element, ok)
        });
    }
    let mut ok = true;
    let mut rewritten = updates.clone();
    for update in &mut rewritten {
        rewrite(&mut update.value, container, index, element, &mut ok);
    }
    if !ok {
        return None;
    }
    if let ContainerRef::Local(name) = container {
        let evals: Vec<NirOp> = rewritten
            .iter()
            .map(|update| NirOp::Eval {
                value: update.value.clone(),
            })
            .collect();
        if !local_reads_are_lookups(&evals, name) {
            return None;
        }
    }
    Some(NirValue::WithUpdate {
        type_: type_.clone(),
        target: Box::new(NirValue::Local(element.to_string())),
        updates: rewritten,
    })
}

/// The element-bound rules for one `get` binding `e` (see the module doc).
struct SiteCheck<'a> {
    e: &'a str,
    container: &'a ContainerRef,
    index: &'a NirValue,
    /// The locals `index` reads.
    index_names: &'a HashSet<String>,
    /// Whether a local may stand for S1's result: see the through-a-temporary
    /// spelling.
    single_use: &'a dyn Fn(&str) -> bool,
    global_written: &'a dyn Fn(&[NirOp], &str) -> bool,
}

/// The sites [`SiteCheck::segment`] accepted, with the reads of `e` and the
/// `e = WITH e { … }` statements they account for.
#[derive(Default)]
struct SiteTally {
    sites: Vec<ElementSite>,
    reads: usize,
    reassignments: usize,
}

impl SiteCheck<'_> {
    /// `op` writes the container, or is an `IF` with an `ELSE` whose every arm
    /// ends in such a statement.
    fn is_terminal(&self, op: &NirOp) -> bool {
        match op {
            NirOp::If {
                then_body,
                else_body,
                ..
            } => [then_body, else_body]
                .iter()
                .all(|arm| arm.last().is_some_and(|last| self.is_terminal(last))),
            _ => write_back_of(op, self.container).is_some(),
        }
    }

    /// Nothing `ops` runs writes the container or a local of the index.
    fn holds_still(&self, ops: &[NirOp]) -> bool {
        !writes_any(ops, self.index_names)
            && container_untouched(ops, self.container, self.global_written)
    }

    /// `ops[start..w]` is a window, and `ops[w]` writes `e` back to its slot — or is
    /// an `IF` each of whose arms is such a segment. Records the accepted sites
    /// and the reads of `e` they account for into `tally`.
    fn segment(&self, ops: &[NirOp], start: usize, w: usize, tally: &mut SiteTally) -> bool {
        let this = HashSet::from([self.e.to_string()]);
        if let NirOp::If {
            condition,
            then_body,
            else_body,
        } = &ops[w]
        {
            // The window and the condition run before either arm: field reads of
            // `e` only, and nothing that writes the container or the index.
            let before = &ops[start..w];
            let condition = [NirOp::Eval {
                value: condition.clone(),
            }];
            let window = reads_in(before, &this);
            let tested = reads_in(&condition, &this);
            if window.all != window.member
                || tested.all != tested.member
                || !self.holds_still(before)
                || !self.holds_still(&condition)
            {
                return false;
            }
            tally.reads += window.all + tested.all;
            return [then_body, else_body]
                .iter()
                .all(|arm| !arm.is_empty() && self.segment(arm, 0, arm.len() - 1, tally));
        }
        let Some((index, item)) = write_back_of(&ops[w], self.container) else {
            return false;
        };
        if !same_index(index, self.index) {
            return false;
        }
        let key = |j: usize| op_key(&ops[j]);
        // S1 (the statement holding `WITH e { … }`, if any) and the site.
        let (s1, site) = match item {
            // `e = WITH e { … }` (optional) / `xs = set(xs, i, e)`.
            NirValue::Local(p) if p == self.e => {
                let s1 = (w > start).then(|| w - 1).filter(|&j| {
                    matches!(&ops[j], NirOp::Assign { name, value }
                        if name == self.e && is_with_over(value, self.e))
                });
                tally.reads += 1;
                tally.reassignments += usize::from(s1.is_some());
                (
                    s1,
                    ElementSite {
                        template: ops[w].clone(),
                        update: s1.map(key),
                        write_back: Some(key(w)),
                    },
                )
            }
            // `LET u = WITH e { … }` / `xs = set(xs, i, u)`.
            NirValue::Local(u) => {
                let bound = (w > start).then(|| w - 1).filter(|&j| {
                    matches!(&ops[j], NirOp::Bind { name, value: Some(value), .. }
                        if name == u && is_with_over(value, self.e))
                });
                let Some(s1) = bound.filter(|_| (self.single_use)(u)) else {
                    return false;
                };
                (
                    Some(s1),
                    ElementSite {
                        template: ops[w].clone(),
                        update: Some(key(s1)),
                        write_back: Some(key(w)),
                    },
                )
            }
            // Fused: `xs = set(xs, i, WITH e { … })`. The window below stops
            // before W, so W's own update values are checked here.
            with if is_with_over(with, self.e) => {
                if !self.holds_still(&[NirOp::Eval {
                    value: with.clone(),
                }]) {
                    return false;
                }
                (
                    Some(w),
                    ElementSite {
                        template: ops[w].clone(),
                        update: Some(key(w)),
                        write_back: None,
                    },
                )
            }
            _ => return false,
        };
        // In S1 `e` is read through fields and once as the WITH's target.
        if let Some(s1) = s1 {
            let update = reads_in(std::slice::from_ref(&ops[s1]), &this);
            if update.with_target != 1 || update.all != update.member + update.with_target {
                return false;
            }
            tally.reads += update.all;
        }
        // Before S1, only through fields; and nothing before W writes the
        // container or a local of the index.
        let window = reads_in(&ops[start..s1.unwrap_or(w)], &this);
        if window.all != window.member || !self.holds_still(&ops[start..w]) {
            return false;
        }
        tally.reads += window.all;
        tally.sites.push(site);
        true
    }
}

/// See the module doc. `global_written(ops, g)` answers whether running `ops` can
/// reach a `StoreGlobal` of `g`; a caller that cannot answer may say `false`
/// ONLY if it uses the result solely to exclude names (the answer then describes
/// a superset of the real borrows, which is the safe direction for exclusion).
pub(crate) fn collect_borrow_gets(
    ops: &[NirOp],
    address_taken: &HashSet<String>,
    global_written: &dyn Fn(&[NirOp], &str) -> bool,
) -> BorrowGets {
    /// A `get`/`getOr` binding's initializer.
    struct GetBind {
        type_: ParameterType,
        container: ContainerRef,
        get_or: bool,
        /// `getOr`'s default, when it is a plain local.
        default_local: Option<String>,
        index: NirValue,
    }
    struct Collector {
        gets: HashMap<String, GetBind>,
        copy_src: HashMap<String, String>, // m -> src (Bind { value: Local(src) })
        bind_counts: HashMap<String, usize>,
        assign_counts: HashMap<String, usize>,
        by_ref_captures: HashSet<String>,
        total_reads: HashMap<String, usize>,
        member_reads: HashMap<String, usize>,
        scrutinee_reads: HashMap<String, usize>,
        // Reads of a local as the value of a `UnionExtract` — the MATCH's own
        // read-only variant bindings (`n = UnionExtract($matchN)`), part of
        // consuming the scrutinee, not an escape.
        union_extract_reads: HashMap<String, usize>,
        // bug-776: the `UnionExtract`s that ARE a MATCH's own variant bindings —
        // the direct value of a `Bind` at the top of a case body of a `MATCH`
        // over the same local. Place forwarding (plan-149, and Level-3
        // `aggcopy`) deletes the `$matchN = e` temp, leaving `MATCH e` and
        // `UnionExtract(e)`; these are then `e`'s own borrow-transparent reads.
        case_extract_binds: HashMap<String, usize>,
        // bug-766 B: reads of a local as the target of a `WITH` that is a
        // `set`/`append` item (`copied_item`) — a read the item's copy consumes.
        item_with_targets: HashMap<String, usize>,
        // bug-766 B: the declared type of every `get` (not `getOr`) bind of a name.
        get_bind_types: HashMap<String, Vec<ParameterType>>,
    }
    impl NirVisitor for Collector {
        fn visit_op(&mut self, op: &NirOp) {
            match op {
                NirOp::Bind {
                    name, value, type_, ..
                } => {
                    *self.bind_counts.entry(name.clone()).or_insert(0) += 1;
                    match value {
                        Some(value @ NirValue::Call { .. }) => {
                            if let Some((kind, args)) = get_call(value) {
                                if let (Some(container), Some(index)) =
                                    (args.first().and_then(ContainerRef::of), args.get(1))
                                {
                                    let default_local = match args.get(2) {
                                        Some(NirValue::Local(d)) => Some(d.clone()),
                                        _ => None,
                                    };
                                    if kind == "get" {
                                        self.get_bind_types
                                            .entry(name.clone())
                                            .or_default()
                                            .push(type_.clone());
                                    }
                                    self.gets.insert(
                                        name.clone(),
                                        GetBind {
                                            type_: type_.clone(),
                                            container,
                                            get_or: kind == "getOr",
                                            default_local,
                                            index: index.clone(),
                                        },
                                    );
                                }
                            }
                        }
                        Some(NirValue::Local(src)) => {
                            self.copy_src.insert(name.clone(), src.clone());
                        }
                        Some(NirValue::Capture { by_ref: true, .. }) => {
                            self.by_ref_captures.insert(name.clone());
                        }
                        _ => {}
                    }
                }
                NirOp::Assign { name, .. } => {
                    *self.assign_counts.entry(name.clone()).or_insert(0) += 1;
                }
                NirOp::StateAssign { resource, .. } => {
                    *self.assign_counts.entry(resource.clone()).or_insert(0) += 1;
                }
                NirOp::Match {
                    value: NirValue::Local(scrut),
                    cases,
                } => {
                    *self.scrutinee_reads.entry(scrut.clone()).or_insert(0) += 1;
                    for case in cases {
                        for op in &case.body {
                            if let NirOp::Bind {
                                value: Some(NirValue::UnionExtract { value, .. }),
                                ..
                            } = op
                            {
                                if matches!(value.as_ref(), NirValue::Local(l) if l == scrut) {
                                    *self.case_extract_binds.entry(scrut.clone()).or_insert(0) += 1;
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
            walk_op(self, op);
        }
        fn visit_value(&mut self, value: &NirValue) {
            match value {
                NirValue::Local(name) => {
                    *self.total_reads.entry(name.clone()).or_insert(0) += 1;
                }
                NirValue::MemberAccess { target, .. } => {
                    if let NirValue::Local(name) = target.as_ref() {
                        *self.member_reads.entry(name.clone()).or_insert(0) += 1;
                    }
                }
                NirValue::UnionExtract { value: inner, .. } => {
                    if let NirValue::Local(name) = inner.as_ref() {
                        *self.union_extract_reads.entry(name.clone()).or_insert(0) += 1;
                    }
                }
                NirValue::Call { .. } => {
                    if let Some(NirValue::WithUpdate { target, .. }) = copied_item(value) {
                        if let NirValue::Local(name) = target.as_ref() {
                            *self.item_with_targets.entry(name.clone()).or_insert(0) += 1;
                        }
                    }
                }
                _ => {}
            }
            walk_value(self, value);
        }
    }
    let mut c = Collector {
        gets: HashMap::new(),
        copy_src: HashMap::new(),
        bind_counts: HashMap::new(),
        assign_counts: HashMap::new(),
        by_ref_captures: HashSet::new(),
        total_reads: HashMap::new(),
        member_reads: HashMap::new(),
        scrutinee_reads: HashMap::new(),
        union_extract_reads: HashMap::new(),
        case_extract_binds: HashMap::new(),
        item_with_targets: HashMap::new(),
        get_bind_types: HashMap::new(),
    };
    c.visit_ops(ops);
    let count = |map: &HashMap<String, usize>, n: &str| map.get(n).copied().unwrap_or(0);
    let reads = |n: &str| count(&c.total_reads, n);
    let scrut = |n: &str| count(&c.scrutinee_reads, n);
    let extract = |n: &str| count(&c.union_extract_reads, n);
    let members = |n: &str| count(&c.member_reads, n);
    let assigned = |n: &str| count(&c.assign_counts, n) > 0;
    // `m` is a MATCH-scrutinee temp: used only by the MATCH — as its scrutinee (≥1)
    // and as the value of the case `UnionExtract`s (its read-only variant bindings),
    // nothing else. The IR desugars `MATCH e` into `$matchN = e; MATCH $matchN`, so
    // the scrutinee is this temp, not `e`.
    let is_match_temp =
        |m: &str| scrut(m) >= 1 && reads(m) == scrut(m) + extract(m) && !address_taken.contains(m);
    // The match temps that copy each source.
    let mut match_temps_of: HashMap<&str, Vec<&str>> = HashMap::new();
    for (m, src) in &c.copy_src {
        if is_match_temp(m) {
            match_temps_of
                .entry(src.as_str())
                .or_default()
                .push(m.as_str());
        }
    }
    // A local is immutable through the scope (bound ≤1, never reassigned, not
    // address-taken) — a reassign frees its old block while a borrow points in.
    let immutable = |l: &str| {
        count(&c.bind_counts, l) <= 1
            && !assigned(l)
            && !address_taken.contains(l)
            && !c.by_ref_captures.contains(l)
    };
    let container_ok = |container: &ContainerRef| match container {
        ContainerRef::Local(l) => !address_taken.contains(l) && !c.by_ref_captures.contains(l),
        ContainerRef::Global(_) => true,
    };

    // Where each candidate is bound: (block, index in block).
    let mut sites: HashMap<&str, (&[NirOp], usize)> = HashMap::new();
    for_each_block(ops, &mut |block| {
        for (k, op) in block.iter().enumerate() {
            if let NirOp::Bind { name, .. } = op {
                if c.gets.contains_key(name) {
                    sites.insert(name.as_str(), (block, k));
                }
            }
        }
    });

    let mut out = BorrowGets::default();
    for (e, get) in &c.gets {
        // One binding of this name, never address-taken — every rule below reasons
        // about THE binding, and a second one would share its classification.
        if count(&c.bind_counts, e) != 1
            || address_taken.contains(e)
            || !container_ok(&get.container)
        {
            continue;
        }
        let temps = match_temps_of.get(e.as_str()).cloned().unwrap_or_default();
        // Every read is borrow-transparent: a field read, a MATCH scrutinee, the
        // value of a match temp's copy-bind, or — once forwarding has removed that
        // temp (bug-776) — one of `e`'s own MATCH's variant bindings. Only extracts
        // in that exact position count; any other `UnionExtract(e)` stays an
        // unexplained read, so `e` copies as before.
        let own_case_extracts = count(&c.case_extract_binds, e);
        let read_only = !assigned(e)
            && reads(e) >= 1
            && reads(e) == members(e) + scrut(e) + own_case_extracts + temps.len();
        if read_only {
            // plan-86 E: an immutable local container (and, for `getOr`, an
            // immutable local default — on a miss the binding IS the default, and a
            // default built by the statement is freed at its end).
            let pinned = matches!(&get.container, ContainerRef::Local(l) if immutable(l))
                && (!get.get_or || get.default_local.as_deref().is_some_and(&immutable));
            if pinned {
                out.names.insert(e.clone());
                out.pinned.insert(e.clone());
                continue;
            }
            // bug-689: the windowed borrow. `getOr` is left to the pinned form: its
            // default has a lifetime of its own this window does not bound.
            if get.get_or {
                continue;
            }
            let Some(&(block, k)) = sites.get(e.as_str()) else {
                continue;
            };
            let mut named: HashSet<String> = HashSet::from([e.clone()]);
            named.extend(temps.iter().map(|m| m.to_string()));
            let Some(last) = (k + 1..block.len())
                .rev()
                .find(|&j| reads_in(std::slice::from_ref(&block[j]), &named).all > 0)
            else {
                continue;
            };
            let window = &block[k + 1..=last];
            // Every read of `e` lies in the window (so none survives past its end).
            if reads_in(window, &HashSet::from([e.clone()])).all != reads(e) {
                continue;
            }
            // The bind itself counts: an index operand that stores to a global
            // container makes bug-496 snapshot it, and the borrow would then point
            // into that statement's temporary copy.
            if container_untouched(&block[k..=last], &get.container, global_written) {
                out.names.insert(e.clone());
            }
            continue;
        }
        // bug-689 Phase 3: the element-bound `get → WITH → set`.
        if get.get_or || !temps.is_empty() || scrut(e) != 0 || !pure_index(&get.index) {
            continue;
        }
        let Some(&(block, k)) = sites.get(e.as_str()) else {
            continue;
        };
        let mut index_names = HashSet::new();
        index_locals(&get.index, &mut index_names);
        if index_names.iter().any(|n| address_taken.contains(n)) {
            continue;
        }
        // bug-766: `u` of `LET u = WITH e { … }` / `set(xs, i, u)` — bound once,
        // never reassigned or captured by reference, read only by the `set`.
        let single_use = |u: &str| {
            count(&c.bind_counts, u) == 1
                && !assigned(u)
                && !address_taken.contains(u)
                && !c.by_ref_captures.contains(u)
                && reads(u) == 1
        };
        let check = SiteCheck {
            e,
            container: &get.container,
            index: &get.index,
            index_names: &index_names,
            single_use: &single_use,
            global_written,
        };
        // The first statement after the bind that writes the container (or is an
        // `IF` every arm of which ends by writing it). The window before it runs
        // from the bind itself (see the read-only window).
        let Some(w) = (k + 1..block.len()).find(|&j| check.is_terminal(&block[j])) else {
            continue;
        };
        let mut tally = SiteTally::default();
        // `e` is read only through fields outside its S1s and write-backs, is
        // reassigned only by its S1s, and nothing between the bind and each W
        // writes the container or a local of the index.
        if !check.segment(block, k, w, &mut tally)
            || count(&c.assign_counts, e) != tally.reassignments
            || reads(e) != tally.reads
        {
            continue;
        }
        let mut sites = tally.sites.into_iter();
        let Some(first) = sites.next() else {
            continue;
        };
        out.names.insert(e.clone());
        out.elements.insert(
            e.clone(),
            ElementBinding {
                type_: get.type_.clone(),
                template: first.template,
                update: first.update,
                write_back: first.write_back,
                other_sites: sites.collect(),
            },
        );
    }
    // bug-766 B: a copy nothing can see outlive its binding. Every bind of the
    // name is a `get` of one type (sibling scopes may each bind it — a local is
    // never redeclared in a nested one, so their lifetimes are disjoint), never
    // reassigned, address-taken or captured by reference, and every read a field
    // read or the target of a `WITH` whose result a `set`/`append` copies — so no
    // read hands the binding's block to anything that keeps it. `getOr` is left
    // alone: on a miss its value is the default, whose lifetime is its own.
    for e in c.gets.keys() {
        let with_items = count(&c.item_with_targets, e);
        let types = c.get_bind_types.get(e).map(Vec::as_slice).unwrap_or(&[]);
        let Some(type_) = types.first() else {
            continue;
        };
        if out.names.contains(e)
            || types.len() != count(&c.bind_counts, e)
            || types.iter().any(|other| other != type_)
            || assigned(e)
            || address_taken.contains(e)
            || c.by_ref_captures.contains(e)
            || reads(e) == 0
            || reads(e) != members(e) + with_items
        {
            continue;
        }
        out.scratch.insert(e.clone(), type_.clone());
    }
    // A match temp `$matchN = Local(e)` that copies a get-borrow `e` also borrows
    // (aliases `e`), so the container element flows into `MATCH` with zero copies.
    for (m, src) in &c.copy_src {
        if is_match_temp(m) && out.names.contains(src) && !out.elements.contains_key(src) {
            out.names.insert(m.clone());
            if out.pinned.contains(src) {
                out.pinned.insert(m.clone());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::target::shared::nir::{NirFunction, NirModule};
    use crate::testutil::{nir_for_src, CodeTarget};

    const TYPES: &str = "\
IMPORT collections
IMPORT io

TYPE P
  age AS Integer
  trail AS List OF Float
END TYPE

FUNC mk AS List OF P
  RETURN [P[age := 1, trail := [0.0]], P[age := 2, trail := [0.0]]]
END FUNC

FUNC f(p AS P) AS Integer
  RETURN p.age
END FUNC
";

    fn lower(body: &str) -> NirModule {
        nir_for_src(
            &format!("{TYPES}\n{body}"),
            CodeTarget::MacosAarch64,
            crate::target::NativeBuildMode::Console,
        )
        .unwrap_or_else(|error| panic!("the probe lowers to NIR: {error}"))
    }

    fn function<'m>(module: &'m NirModule, name: &str) -> &'m NirFunction {
        module
            .functions
            .iter()
            .find(|f| f.name == name || f.name.rsplit(['.', ':']).next() == Some(name))
            .unwrap_or_else(|| panic!("no function `{name}`"))
    }

    /// The classification of `main`'s bindings, with no global ever written.
    fn classify(body: &str) -> BorrowGets {
        let module = lower(body);
        let main = function(&module, "main");
        let mut address_taken = HashSet::new();
        crate::codegen::engine::function::function_lowering::collect_address_taken_locals(
            &main.body,
            &mut address_taken,
        );
        collect_borrow_gets(&main.body, &address_taken, &|_, _| false)
    }

    /// bug-776: plan-149's place forwarding deletes the `$match0 = e` temp the
    /// IR's `MATCH` desugar introduces, so the classifier sees `MATCH e` and
    /// `UnionExtract(e)` directly. `e` is still read only as the match's
    /// scrutinee and its variant bindings, so it must stay a pinned borrow.
    #[test]
    fn a_directly_matched_binding_after_place_forwarding_is_pinned() {
        use crate::optimizer::{with_opt_level, OptLevel};
        let body = "TYPE Num\n  value AS Integer\nEND TYPE\n\
             TYPE Add\n  left AS Integer\n  right AS Integer\nEND TYPE\n\
             UNION Expr\n  Num\n  Add\nEND UNION\n\
             FUNC main() AS Integer\n  LET xs AS List OF Expr = [Num[3], Add[1, 2]]\n  \
             LET e AS Expr = collections::get(xs, 1)\n  MATCH e\n    CASE Num(n)\n      \
             RETURN n.value\n    CASE Add(a)\n      RETURN a.left + a.right\n  END MATCH\n  \
             RETURN 0\nEND FUNC\n";
        let mut module = with_opt_level(OptLevel(0), || lower(body));
        with_opt_level(OptLevel(1), || {
            crate::optimizer::opt1::placefwd::forward(&mut module)
        });
        let main = function(&module, "main");
        struct MatchTemp(bool);
        impl NirVisitor for MatchTemp {
            fn visit_op(&mut self, op: &NirOp) {
                if let NirOp::Bind { name, .. } = op {
                    self.0 |= name.starts_with("$match");
                }
                walk_op(self, op);
            }
            fn visit_value(&mut self, value: &NirValue) {
                if let NirValue::Local(name) = value {
                    self.0 |= name.starts_with("$match");
                }
                walk_value(self, value);
            }
        }
        let mut temp = MatchTemp(false);
        temp.visit_ops(&main.body);
        assert!(
            !temp.0,
            "the probe needs the forwarded form: the `$match` temp must be gone"
        );
        let mut address_taken = HashSet::new();
        crate::codegen::engine::function::function_lowering::collect_address_taken_locals(
            &main.body,
            &mut address_taken,
        );
        let got = collect_borrow_gets(&main.body, &address_taken, &|_, _| false);
        assert!(
            got.names.contains("e") && got.pinned.contains("e"),
            "a get binding read only through MATCH and its variant bindings is a pinned \
             borrow after place forwarding; got names {:?} pinned {:?}",
            got.names,
            got.pinned
        );
    }

    #[test]
    fn a_field_read_binding_over_an_immutable_list_is_pinned() {
        let got = classify(
            "FUNC main() AS Integer\n  LET xs AS List OF P = mk()\n  \
             LET p AS P = collections::get(xs, 0)\n  RETURN p.age + len(p.trail)\nEND FUNC\n",
        );
        assert!(got.names.contains("p") && got.pinned.contains("p"));
        assert!(got.elements.is_empty());
    }

    #[test]
    fn a_field_read_binding_over_a_mut_list_is_windowed() {
        let got = classify(
            "FUNC main() AS Integer\n  MUT xs AS List OF P = mk()\n  \
             LET p AS P = collections::get(xs, 0)\n  LET n AS Integer = p.age + len(xs)\n  \
             xs = collections::append(xs, p)\n  RETURN n\nEND FUNC\n",
        );
        // `p` is passed whole to `append` — not a field read — so it is no borrow.
        assert!(!got.names.contains("p"));
        let got = classify(
            "FUNC main() AS Integer\n  MUT xs AS List OF P = mk()\n  \
             LET p AS P = collections::get(xs, 0)\n  LET n AS Integer = p.age + len(xs)\n  \
             xs = collections::append(xs, P[age := n, trail := []])\n  RETURN n\nEND FUNC\n",
        );
        assert!(got.names.contains("p") && !got.pinned.contains("p"));
    }

    #[test]
    fn a_write_to_the_container_inside_the_window_refuses_the_borrow() {
        let got = classify(
            "FUNC main() AS Integer\n  MUT xs AS List OF P = mk()\n  \
             LET p AS P = collections::get(xs, 0)\n  \
             xs = collections::append(xs, P[age := 9, trail := []])\n  RETURN p.age\nEND FUNC\n",
        );
        assert!(!got.names.contains("p"));
    }

    #[test]
    fn a_non_lookup_read_of_the_container_inside_the_window_refuses_the_borrow() {
        // `xs` handed to a user function could be moved or handed over there.
        let got = classify(
            "FUNC g(ys AS List OF P) AS Integer\n  RETURN len(ys)\nEND FUNC\n\
             FUNC main() AS Integer\n  MUT xs AS List OF P = mk()\n  xs = mk()\n  \
             LET p AS P = collections::get(xs, 0)\n  LET n AS Integer = g(xs)\n  \
             RETURN p.age + n\nEND FUNC\n",
        );
        assert!(!got.names.contains("p"));
    }

    #[test]
    fn a_get_or_with_a_built_default_is_no_borrow() {
        let got = classify(
            "FUNC main() AS Integer\n  LET xs AS List OF P = mk()\n  \
             LET p AS P = collections::getOr(xs, 5, P[age := 0, trail := []])\n  \
             RETURN p.age\nEND FUNC\n",
        );
        assert!(!got.names.contains("p"));
        let got = classify(
            "FUNC main() AS Integer\n  LET xs AS List OF P = mk()\n  \
             LET d AS P = P[age := 0, trail := []]\n  \
             LET p AS P = collections::getOr(xs, 5, d)\n  RETURN p.age\nEND FUNC\n",
        );
        assert!(got.pinned.contains("p"));
    }

    #[test]
    fn get_with_set_on_the_same_slot_is_element_bound() {
        let got = classify(
            "FUNC main() AS Integer\n  MUT xs AS List OF P = mk()\n  FOR i = 0 TO 1\n    \
             MUT p AS P = collections::get(xs, i)\n    LET n AS Integer = p.age\n    \
             p = WITH p { age := n + 1 }\n    xs = collections::set(xs, i, p)\n  NEXT\n  \
             RETURN len(xs)\nEND FUNC\n",
        );
        let binding = got.elements.get("p").expect("p is element-bound");
        assert!(binding.update.is_some() && binding.write_back.is_some());
        assert!(got.names.contains("p"));
    }

    /// bug-766: `set(xs, i, WITH p { … })` is the S1+W pair in one statement.
    #[test]
    fn a_with_over_the_binding_as_the_set_item_is_element_bound() {
        let got = classify(
            "FUNC main() AS Integer\n  MUT xs AS List OF P = mk()\n  FOR i = 0 TO 1\n    \
             LET p AS P = collections::get(xs, i)\n    LET n AS Integer = p.age\n    \
             xs = collections::set(xs, i, WITH p { age := n + p.age })\n  NEXT\n  \
             RETURN len(xs)\nEND FUNC\n",
        );
        let binding = got.elements.get("p").expect("p is element-bound");
        // The fused statement is the update and its own write-back.
        assert!(binding.update.is_some() && binding.write_back.is_none());
        assert!(binding.other_sites.is_empty() && got.names.contains("p"));
    }

    /// bug-766: `LET u = WITH p { … }` / `set(xs, i, u)` — `u` is S1's result.
    #[test]
    fn a_let_bound_with_before_its_set_is_element_bound() {
        let got = classify(
            "FUNC main() AS Integer\n  MUT xs AS List OF P = mk()\n  FOR i = 0 TO 1\n    \
             LET p AS P = collections::get(xs, i)\n    LET u AS P = WITH p { age := p.age + 1 }\n    \
             xs = collections::set(xs, i, u)\n  NEXT\n  RETURN len(xs)\nEND FUNC\n",
        );
        let binding = got.elements.get("p").expect("p is element-bound");
        assert!(binding.update.is_some() && binding.write_back.is_some());
        // `u` read again is a second use of S1's result: no element binding.
        let got = classify(
            "FUNC main() AS Integer\n  MUT xs AS List OF P = mk()\n  \
             LET p AS P = collections::get(xs, 0)\n  LET u AS P = WITH p { age := p.age + 1 }\n  \
             xs = collections::set(xs, 0, u)\n  RETURN u.age\nEND FUNC\n",
        );
        assert!(got.elements.is_empty());
    }

    /// bug-766: a write-back in every arm of an `IF`, each arm its own site.
    #[test]
    fn write_backs_in_both_if_arms_are_element_bound() {
        let got = classify(
            "FUNC main() AS Integer\n  MUT xs AS List OF P = mk()\n  FOR i = 0 TO 1\n    \
             LET p AS P = collections::get(xs, i)\n    IF p.age > 1 THEN\n      \
             xs = collections::set(xs, i, WITH p { age := p.age + 1 })\n    ELSE\n      \
             LET n AS Integer = p.age * 2\n      LET u AS P = WITH p { age := n }\n      \
             xs = collections::set(xs, i, u)\n    END IF\n  NEXT\n  RETURN len(xs)\nEND FUNC\n",
        );
        let binding = got.elements.get("p").expect("p is element-bound");
        assert_eq!(binding.sites().count(), 2);
        assert!(binding.sites().all(|(update, _)| update.is_some()));
    }

    #[test]
    fn a_write_back_in_one_if_arm_only_is_not_bound() {
        let got = classify(
            "FUNC main() AS Integer\n  MUT xs AS List OF P = mk()\n  FOR i = 0 TO 1\n    \
             LET p AS P = collections::get(xs, i)\n    IF p.age > 1 THEN\n      \
             xs = collections::set(xs, i, WITH p { age := p.age + 1 })\n    ELSE\n      \
             io::print(toString(p.age))\n    END IF\n  NEXT\n  RETURN len(xs)\nEND FUNC\n",
        );
        assert!(got.elements.is_empty() && !got.names.contains("p"));
        // No ELSE at all.
        let got = classify(
            "FUNC main() AS Integer\n  MUT xs AS List OF P = mk()\n  FOR i = 0 TO 1\n    \
             LET p AS P = collections::get(xs, i)\n    IF p.age > 1 THEN\n      \
             xs = collections::set(xs, i, WITH p { age := p.age + 1 })\n    END IF\n  NEXT\n  \
             RETURN len(xs)\nEND FUNC\n",
        );
        assert!(got.elements.is_empty() && !got.names.contains("p"));
    }

    #[test]
    fn a_read_of_the_binding_after_a_with_item_set_is_not_bound() {
        let got = classify(
            "FUNC main() AS Integer\n  MUT xs AS List OF P = mk()\n  \
             LET p AS P = collections::get(xs, 0)\n  \
             xs = collections::set(xs, 0, WITH p { age := 7 })\n  RETURN p.age\nEND FUNC\n",
        );
        assert!(got.elements.is_empty() && !got.names.contains("p"));
    }

    /// bug-766 B: the n-body pair step — two elements live across a write to the
    /// container, each read only through fields and as a `set` item's `WITH`
    /// target. Neither can alias its slot (`pi` is read after its write-back,
    /// `pj`'s slot is written first), so both stay copies, of the scratch kind.
    #[test]
    fn two_elements_live_across_a_write_are_scratch_copies() {
        let got = classify(
            "FUNC main() AS Integer\n  MUT xs AS List OF P = mk()\n  FOR i = 0 TO 1\n    \
             LET pi AS P = collections::get(xs, i)\n    LET pj AS P = collections::get(xs, 1 - i)\n    \
             xs = collections::set(xs, i, WITH pi { age := pi.age + pj.age })\n    \
             xs = collections::set(xs, 1 - i, WITH pj { age := pj.age - pi.age })\n  NEXT\n  \
             RETURN len(xs)\nEND FUNC\n",
        );
        assert!(!got.names.contains("pi") && !got.names.contains("pj"));
        assert!(got.scratch.contains_key("pi") && got.scratch.contains_key("pj"));
    }

    /// bug-766 B: any read that could keep the binding's block — passed whole,
    /// bound to another name, returned, a `WITH` that is not a copied item — or a
    /// `getOr` leaves it an ordinary owned copy.
    #[test]
    fn an_escaping_or_get_or_binding_is_no_scratch_copy() {
        for (tail, why) in [
            ("RETURN f(pi) + pj.age", "passed whole"),
            (
                "MUT q AS P = pi\n  q = WITH q { age := 1 }\n  RETURN q.age + pj.age",
                "bound to another name",
            ),
            (
                "LET q AS P = WITH pi { age := 1 }\n  RETURN q.age + pj.age",
                "a WITH bound to a name",
            ),
        ] {
            let got = classify(&format!(
                "FUNC main() AS Integer\n  MUT xs AS List OF P = mk()\n  \
                 LET pi AS P = collections::get(xs, 0)\n  LET pj AS P = collections::get(xs, 1)\n  \
                 xs = collections::set(xs, 1, WITH pj {{ age := pi.age }})\n  {tail}\nEND FUNC\n"
            ));
            assert!(!got.scratch.contains_key("pi"), "pi {why}");
        }
        let got = classify(
            "FUNC main() AS Integer\n  MUT xs AS List OF P = mk()\n  \
             LET d AS P = P[age := 0, trail := []]\n  \
             LET pi AS P = collections::getOr(xs, 0, d)\n  \
             xs = collections::set(xs, 0, WITH pi { age := 1 })\n  RETURN pi.age\nEND FUNC\n",
        );
        assert!(!got.scratch.contains_key("pi"), "a getOr binding");
    }

    #[test]
    fn a_write_to_the_container_between_the_bind_and_the_set_is_not_bound() {
        let got = classify(
            "FUNC main() AS Integer\n  MUT xs AS List OF P = mk()\n  \
             LET p AS P = collections::get(xs, 0)\n  \
             xs = collections::append(xs, P[age := 9, trail := []])\n  \
             xs = collections::set(xs, 0, WITH p { age := p.age + 1 })\n  RETURN 0\nEND FUNC\n",
        );
        assert!(got.elements.is_empty() && !got.names.contains("p"));
        // Nor inside an arm before its write-back.
        let got = classify(
            "FUNC main() AS Integer\n  MUT xs AS List OF P = mk()\n  \
             LET p AS P = collections::get(xs, 0)\n  IF p.age > 1 THEN\n    \
             xs = collections::set(xs, 0, WITH p { age := 1 })\n  ELSE\n    \
             xs = collections::append(xs, P[age := 9, trail := []])\n    \
             xs = collections::set(xs, 0, WITH p { age := 2 })\n  END IF\n  RETURN 0\nEND FUNC\n",
        );
        assert!(got.elements.is_empty() && !got.names.contains("p"));
        // Nor by the update's own values.
        let got = classify(
            "FUNC g(ys AS List OF P) AS Integer\n  RETURN len(ys)\nEND FUNC\n\
             FUNC main() AS Integer\n  MUT xs AS List OF P = mk()\n  \
             LET p AS P = collections::get(xs, 0)\n  \
             xs = collections::set(xs, 0, WITH p { age := g(xs) })\n  RETURN 0\nEND FUNC\n",
        );
        assert!(got.elements.is_empty());
    }

    #[test]
    fn an_element_update_that_is_not_adjacent_to_its_write_back_is_not_bound() {
        let got = classify(
            "FUNC main() AS Integer\n  MUT xs AS List OF P = mk()\n  \
             MUT p AS P = collections::get(xs, 0)\n  p = WITH p { age := 7 }\n  \
             io::print(toString(collections::get(xs, 0).age))\n  \
             xs = collections::set(xs, 0, p)\n  RETURN 0\nEND FUNC\n",
        );
        assert!(got.elements.is_empty() && !got.names.contains("p"));
    }

    #[test]
    fn an_element_read_after_its_write_back_is_not_bound() {
        let got = classify(
            "FUNC main() AS Integer\n  MUT xs AS List OF P = mk()\n  \
             MUT p AS P = collections::get(xs, 0)\n  p = WITH p { age := 7 }\n  \
             xs = collections::set(xs, 0, p)\n  RETURN p.age\nEND FUNC\n",
        );
        assert!(got.elements.is_empty() && !got.names.contains("p"));
    }

    #[test]
    fn a_write_back_to_another_slot_is_not_bound() {
        let got = classify(
            "FUNC main() AS Integer\n  MUT xs AS List OF P = mk()\n  \
             MUT p AS P = collections::get(xs, 0)\n  p = WITH p { age := 7 }\n  \
             xs = collections::set(xs, 1, p)\n  RETURN 0\nEND FUNC\n",
        );
        assert!(got.elements.is_empty());
        // The index names the same slot only while its locals hold still.
        let got = classify(
            "FUNC main() AS Integer\n  MUT xs AS List OF P = mk()\n  MUT i AS Integer = 0\n  \
             MUT p AS P = collections::get(xs, i)\n  i = 1\n  p = WITH p { age := 7 }\n  \
             xs = collections::set(xs, i, p)\n  RETURN 0\nEND FUNC\n",
        );
        assert!(got.elements.is_empty());
    }

    #[test]
    fn an_element_passed_whole_before_its_update_is_not_bound() {
        let got = classify(
            "FUNC main() AS Integer\n  MUT xs AS List OF P = mk()\n  \
             MUT p AS P = collections::get(xs, 0)\n  LET n AS Integer = f(p)\n  \
             p = WITH p { age := n }\n  xs = collections::set(xs, 0, p)\n  RETURN 0\nEND FUNC\n",
        );
        assert!(got.elements.is_empty() && !got.names.contains("p"));
    }

    #[test]
    fn a_global_container_needs_no_reachable_store() {
        let body = "MUT gs AS List OF P = []\n\
             FUNC main() AS Integer\n  gs = mk()\n  LET p AS P = collections::get(gs, 0)\n  \
             RETURN p.age\nEND FUNC\n";
        let module = lower(body);
        let main = function(&module, "main");
        let none = HashSet::new();
        assert!(collect_borrow_gets(&main.body, &none, &|_, _| false)
            .names
            .contains("p"));
        assert!(!collect_borrow_gets(&main.body, &none, &|_, _| true)
            .names
            .contains("p"));
    }

    #[test]
    fn the_single_expression_update_names_its_container_and_index() {
        let module = lower(
            "FUNC main() AS Integer\n  MUT xs AS List OF P = mk()\n  FOR i = 0 TO 1\n    \
             xs = collections::set(xs, i, WITH collections::get(xs, i) { age := collections::get(xs, i).age + 1 })\n  \
             NEXT\n  RETURN 0\nEND FUNC\n",
        );
        let main = function(&module, "main");
        let mut found = None;
        for_each_block(&main.body, &mut |block| {
            for op in block {
                if let NirOp::Assign { name, value } = op {
                    if name == "xs" {
                        if let Some((container, index, with)) = single_expression_update(value) {
                            found = Some((container, index.clone(), with.clone()));
                        }
                    }
                }
            }
        });
        let (container, index, with) = found.expect("the update is recognised");
        assert_eq!(container, ContainerRef::Local("xs".to_string()));
        let bound = bind_single_expression_element(&with, &container, &index, "$e")
            .expect("its element is read only through fields");
        struct Count(usize);
        impl NirVisitor for Count {
            fn visit_value(&mut self, value: &NirValue) {
                if matches!(value, NirValue::Local(name) if name == "$e") {
                    self.0 += 1;
                }
                walk_value(self, value);
            }
        }
        let mut count = Count(0);
        count.visit_value(&bound);
        // The WITH's target and the one field read.
        assert_eq!(count.0, 2);
    }
}
