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
    /// `p = WITH p { … }` immediately before the write-back, if any (`op_key`).
    pub(crate) update: Option<usize>,
    /// `container = set(container, index, p)` (`op_key`); `None` for a
    /// single-expression update, which is its own write-back.
    pub(crate) write_back: Option<usize>,
}

impl ElementBinding {
    /// `container = set(container, index, item)` — the statement an element
    /// update stands for, lowered as written when no in-place route serves it.
    pub(crate) fn write_back_op(&self, item: NirValue) -> NirOp {
        let mut op = self.template.clone();
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

/// `container = set(container, index, Local(item))` — the write-back of an
/// element-bound `item`: `Some(index)` when `op` has that shape.
fn write_back_index<'o>(
    op: &'o NirOp,
    container: &ContainerRef,
    item: &str,
) -> Option<&'o NirValue> {
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
    matches!(&args[2], NirValue::Local(p) if p == item).then_some(&args[1])
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
                    ..
                } => {
                    *self.scrutinee_reads.entry(scrut.clone()).or_insert(0) += 1;
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
        // Every read is borrow-transparent: a field read, a MATCH scrutinee, or the
        // value of a match temp's copy-bind.
        let read_only =
            !assigned(e) && reads(e) >= 1 && reads(e) == members(e) + scrut(e) + temps.len();
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
        let Some(w) = (k + 1..block.len())
            .find(|&j| write_back_index(&block[j], &get.container, e).is_some())
        else {
            continue;
        };
        if !write_back_index(&block[w], &get.container, e)
            .is_some_and(|index| same_index(index, &get.index))
        {
            continue;
        }
        // S1: `e = WITH e { … }` right before the write-back.
        let update = (w > k + 1)
            .then(|| &block[w - 1])
            .filter(|op| {
                matches!(op, NirOp::Assign { name, value: NirValue::WithUpdate { target, .. } }
                    if name == e && matches!(target.as_ref(), NirValue::Local(t) if t == e))
            })
            .map(|_| w - 1);
        let reads_end = update.unwrap_or(w);
        let this = HashSet::from([e.clone()]);
        let window = &block[k + 1..reads_end];
        let window_reads = reads_in(window, &this);
        let update_reads = match update {
            Some(s1) => reads_in(std::slice::from_ref(&block[s1]), &this),
            None => Reads::default(),
        };
        // `e` is read only through fields before S1; in S1 through fields and as the
        // WITH's target; at W once, as the item; and nowhere else.
        let reassignments = count(&c.assign_counts, e);
        if window_reads.all != window_reads.member
            || update_reads.all != update_reads.member + update_reads.with_target
            || update.is_some_and(|_| update_reads.with_target != 1)
            || reassignments != usize::from(update.is_some())
            || reads(e) != window_reads.all + update_reads.all + 1
        {
            continue;
        }
        // Nothing before W — the bind included (see the read-only window) — writes
        // the container, or any local of the index.
        let before_w = &block[k..w];
        let mut index_names = HashSet::new();
        index_locals(&get.index, &mut index_names);
        if index_names.iter().any(|n| address_taken.contains(n))
            || writes_any(before_w, &index_names)
            || !container_untouched(before_w, &get.container, global_written)
        {
            continue;
        }
        out.names.insert(e.clone());
        out.elements.insert(
            e.clone(),
            ElementBinding {
                type_: get.type_.clone(),
                template: block[w].clone(),
                update: update.map(|s1| op_key(&block[s1])),
                write_back: Some(op_key(&block[w])),
            },
        );
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
