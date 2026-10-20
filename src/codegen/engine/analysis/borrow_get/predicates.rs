//! plan-176-U: the borrow-get window predicates — op and builtin keys, index purity, write-back and
//! `WITH` recognition, local reads/writes, and the single-expression element update. The
//! classification they feed lives in `engine/analysis/borrow_get/mod.rs`.

use super::*;

/// `op as *const NirOp as usize` — the same identity `last_use::op_key` uses.
pub(super) fn op_key(op: &NirOp) -> usize {
    op as *const NirOp as usize
}

/// The bare builtin a call target names (`collections.get` → `get`).
pub(super) fn builtin(target: &str) -> Option<&'static str> {
    crate::codegen::builtins::native_builtin_target(target)
}

/// A `get`/`getOr` call: `(bare name, args)`.
pub(super) fn get_call(value: &NirValue) -> Option<(&'static str, &[NirValue])> {
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
pub(super) fn index_locals(value: &NirValue, out: &mut HashSet<String>) {
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
pub(super) fn write_back_of<'o>(
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
pub(super) fn is_with_over(value: &NirValue, item: &str) -> bool {
    matches!(value, NirValue::WithUpdate { target, .. }
        if matches!(target.as_ref(), NirValue::Local(t) if t == item))
}

/// Every op list in `ops`, depth-first: the top level and each nested body.
pub(super) fn for_each_block<'o>(ops: &'o [NirOp], f: &mut dyn FnMut(&'o [NirOp])) {
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
pub(super) struct Reads {
    pub(super) all: usize,
    pub(super) member: usize,
    pub(super) with_target: usize,
}

pub(super) fn reads_in(ops: &[NirOp], names: &HashSet<String>) -> Reads {
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
pub(super) fn writes_any(ops: &[NirOp], names: &HashSet<String>) -> bool {
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
pub(super) fn container_untouched(
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
