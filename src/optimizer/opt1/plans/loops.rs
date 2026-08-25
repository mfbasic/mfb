//! Loop facts for the Opt1 loop rows (LICM, unswitching, fusion, fission,
//! peeling, rotation) — invariance, loop-control capture, and the pure
//! statement class fusion/fission's interleaving argument needs. Rides the
//! sanctioned read-only traversal seam (`nir::visit`) like the DCE census, so
//! the recursion cannot drift from the one authoritative walk.

use std::collections::HashSet;

use crate::ast::LoopKind;
use crate::target::shared::nir::visit::{walk_op, walk_value, NirVisitor};
use crate::target::shared::nir::{NirOp, NirValue};

use super::super::dce::{pure_non_trapping, scalar_type};

/// Every name a statement list can (re)define anywhere within it: `Bind` and
/// `Assign` targets, `STATE` assigns, loop variables, `TRAP` bindings.
/// Scope-blind like the DCE census, so shadowing is conservative.
pub(crate) fn defined_names(ops: &[NirOp]) -> HashSet<String> {
    #[derive(Default)]
    struct Defs {
        names: HashSet<String>,
    }
    impl NirVisitor for Defs {
        fn visit_op(&mut self, op: &NirOp) {
            match op {
                NirOp::Bind { name, .. }
                | NirOp::Assign { name, .. }
                | NirOp::For { name, .. }
                | NirOp::ForEach { name, .. }
                | NirOp::Trap { name, .. } => {
                    self.names.insert(name.clone());
                }
                NirOp::StateAssign { resource, .. } => {
                    self.names.insert(resource.clone());
                }
                _ => {}
            }
            walk_op(self, op);
        }
    }
    let mut defs = Defs::default();
    defs.visit_ops(ops);
    defs.names
}

/// The names a loop op's body can redefine — **including the loop's own
/// variable**: `FOR`/`FOR EACH` bind it per iteration even though it is not
/// a body statement, so an expression reading it is never invariant. `None`
/// for a non-loop op.
pub(crate) fn loop_body_defined(op: &NirOp) -> Option<HashSet<String>> {
    let (body, var) = match op {
        NirOp::While { body, .. } | NirOp::DoUntil { body, .. } => (body, None),
        NirOp::For { name, body, .. } | NirOp::ForEach { name, body, .. } => (body, Some(name)),
        _ => return None,
    };
    let mut defined = defined_names(body);
    if let Some(var) = var {
        defined.insert(var.clone());
    }
    Some(defined)
}

#[derive(Default)]
struct Reads {
    names: HashSet<String>,
    globals: bool,
}

impl NirVisitor for Reads {
    fn visit_value(&mut self, value: &NirValue) {
        match value {
            NirValue::Local(name) | NirValue::LocalRef { name, .. } => {
                self.names.insert(name.clone());
            }
            NirValue::Global { .. } => self.globals = true,
            _ => {}
        }
        walk_value(self, value);
    }
}

/// The local names `value` reads.
pub(crate) fn value_reads(value: &NirValue) -> HashSet<String> {
    let mut reads = Reads::default();
    reads.visit_value(value);
    reads.names
}

/// Whether `value` is **loop-invariant and freely movable** with respect to a
/// body that defines `defined`: provably pure and non-trapping (so evaluating
/// it once instead of per iteration — or once where zero times ran — is
/// unobservable), reading no name the body can redefine, and reading no
/// global (a call or `StoreGlobal` inside the body could change one between
/// iterations; rather than model that, globals are simply not invariant).
pub(crate) fn invariant(value: &NirValue, defined: &HashSet<String>) -> bool {
    if !pure_non_trapping(value) {
        return false;
    }
    let mut reads = Reads::default();
    reads.visit_value(value);
    !reads.globals && reads.names.is_disjoint(defined)
}

/// Whether `ops` contains an `EXIT`/`CONTINUE` of `kind` that would bind to a
/// *newly introduced* enclosing loop of that kind (i.e., one not already
/// shielded by an inner loop pushing `kind` onto the loop stack — `While`
/// pushes its own kind, `For`/`ForEach` push `For`, `DoUntil` pushes `Do`,
/// mirroring `builder_control`'s loop-stack discipline exactly).
pub(crate) fn captures_loop_control(ops: &[NirOp], kind: LoopKind) -> bool {
    ops.iter().any(|op| match op {
        NirOp::ExitLoop { kind: k } | NirOp::ContinueLoop { kind: k } => *k == kind,
        NirOp::While { kind: k, body, .. } => *k != kind && captures_loop_control(body, kind),
        NirOp::For { body, .. } | NirOp::ForEach { body, .. } => {
            kind != LoopKind::For && captures_loop_control(body, kind)
        }
        NirOp::DoUntil { body, .. } => kind != LoopKind::Do && captures_loop_control(body, kind),
        NirOp::If {
            then_body,
            else_body,
            ..
        } => captures_loop_control(then_body, kind) || captures_loop_control(else_body, kind),
        NirOp::Match { cases, .. } => cases
            .iter()
            .any(|case| captures_loop_control(&case.body, kind)),
        NirOp::Trap { body, .. } => captures_loop_control(body, kind),
        _ => false,
    })
}

/// Recursive statement count — the loop rows' code-growth caps.
pub(crate) fn op_count(ops: &[NirOp]) -> usize {
    ops.iter()
        .map(|op| {
            1 + match op {
                NirOp::If {
                    then_body,
                    else_body,
                    ..
                } => op_count(then_body) + op_count(else_body),
                NirOp::Match { cases, .. } => cases.iter().map(|case| op_count(&case.body)).sum(),
                NirOp::While { body, .. }
                | NirOp::For { body, .. }
                | NirOp::DoUntil { body, .. }
                | NirOp::ForEach { body, .. }
                | NirOp::Trap { body, .. } => op_count(body),
                _ => 0,
            }
        })
        .sum()
}

/// The statement class fusion/fission may interleave or separate: flat
/// scalar `Bind`s/`Assign`s with pure, non-trapping values (and pure `Eval`s,
/// which are DCE food anyway). No control flow, no calls, no collection or
/// resource machinery, no possible trap — so, combined with the callers'
/// read/write disjointness checks, reordering across the other body's
/// iterations is unobservable.
pub(crate) fn pure_statement(op: &NirOp) -> bool {
    match op {
        NirOp::Bind { type_, value, .. } => {
            scalar_type(type_) && value.as_ref().is_none_or(pure_non_trapping)
        }
        NirOp::Assign { value, .. } => pure_non_trapping(value),
        NirOp::Eval { value } => pure_non_trapping(value),
        _ => false,
    }
}

/// The names a `pure_statement` list writes (bind + assign targets).
pub(crate) fn statement_writes(ops: &[NirOp]) -> HashSet<String> {
    let mut writes = HashSet::new();
    for op in ops {
        match op {
            NirOp::Bind { name, .. } | NirOp::Assign { name, .. } => {
                writes.insert(name.clone());
            }
            _ => {}
        }
    }
    writes
}

/// The names a `pure_statement` list reads.
pub(crate) fn statement_reads(ops: &[NirOp]) -> HashSet<String> {
    let mut reads = Reads::default();
    reads.visit_ops(ops);
    reads.names
}

/// Whether a raise inside this function body could be *caught somewhere that
/// still sees the function's locals*: a `TRAP` handler in the body itself, or
/// a by-ref capture (the body is a lowered closure — writes to captured names
/// escape to the parent frame, which survives propagation). Fusion/fission
/// change how many iterations of the *other* half ran when the FOR increment
/// overflows mid-loop; with neither escape hatch, the halves' local writes
/// die unobserved with the frame and the divergence is invisible.
pub(crate) fn locals_survive_a_raise(ops: &[NirOp]) -> bool {
    #[derive(Default)]
    struct Escapes {
        found: bool,
    }
    impl NirVisitor for Escapes {
        fn visit_op(&mut self, op: &NirOp) {
            if matches!(op, NirOp::Trap { .. }) {
                self.found = true;
            }
            walk_op(self, op);
        }
        fn visit_value(&mut self, value: &NirValue) {
            if matches!(value, NirValue::Capture { .. }) {
                self.found = true;
            }
            walk_value(self, value);
        }
    }
    let mut escapes = Escapes::default();
    escapes.visit_ops(ops);
    escapes.found
}

/// Structural equality for the FOR-bound leaves fusion/fission may share or
/// duplicate: equal constants, or the same local name. Anything else — an
/// expression, a global — is not a stable leaf.
pub(crate) fn same_stable_leaf(a: &NirValue, b: &NirValue) -> bool {
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
        (NirValue::Local(na), NirValue::Local(nb)) => na == nb,
        _ => false,
    }
}

/// The local names among a set of bound leaves (for the "no half writes a
/// bound" check).
pub(crate) fn leaf_names(values: &[&NirValue]) -> HashSet<String> {
    let mut names = HashSet::new();
    for value in values {
        if let NirValue::Local(name) = value {
            names.insert(name.clone());
        }
    }
    names
}

/// Whether `value` is itself a stable leaf (Const or Local).
pub(crate) fn stable_leaf(value: &NirValue) -> bool {
    matches!(value, NirValue::Const { .. } | NirValue::Local(_))
}

/// The names a statement list declares **at its own scope level** (top-level
/// `Bind`s only — nested scopes keep their declarations). Splicing one scope's
/// statements into another (branch folding's arm splice, unswitching's arm
/// specialization) must keep these disjoint, or the flattened scope declares
/// a name twice and NIR validation rejects the function.
pub(crate) fn scope_bind_names(ops: &[NirOp]) -> HashSet<String> {
    ops.iter()
        .filter_map(|op| match op {
            NirOp::Bind { name, .. } => Some(name.clone()),
            _ => None,
        })
        .collect()
}

/// Every name a statement list *declares* at any depth: `Bind`s plus
/// `FOR`/`FOR EACH`/`TRAP` variables. NIR locals are **function-unique**
/// (`validate::body` keeps one flat map — nested scopes included), so any
/// pass that duplicates statements must rename exactly these in the copy.
pub(crate) fn declared_names(ops: &[NirOp]) -> HashSet<String> {
    #[derive(Default)]
    struct Decls {
        names: HashSet<String>,
    }
    impl NirVisitor for Decls {
        fn visit_op(&mut self, op: &NirOp) {
            match op {
                NirOp::Bind { name, .. }
                | NirOp::For { name, .. }
                | NirOp::ForEach { name, .. }
                | NirOp::Trap { name, .. } => {
                    self.names.insert(name.clone());
                }
                _ => {}
            }
            walk_op(self, op);
        }
    }
    let mut decls = Decls::default();
    decls.visit_ops(ops);
    decls.names
}

/// Duplicate `ops` with every declared name replaced by a fresh spelling —
/// the scope-safe clone the duplicating loop rows (peeling, unswitching,
/// fission's carried binds) need under NIR's function-unique locals rule.
///
/// The rename itself is a hand-rolled mutable walk (the `nir::visit` seam is
/// read-only), so it is **verified, not trusted**: after renaming, the
/// canonical read-only census must find zero occurrences of every old name
/// in the copy — a variant this walk missed shows up there, and the caller
/// gets `None` (skip the transform) instead of a miscompile. `census` is the
/// whole-function census and `salt` a per-function counter: fresh spellings
/// (`{name}$dup{n}`) are checked unused against it.
pub(crate) fn freshened_clone(
    ops: &[NirOp],
    census: &super::reads::NameUses,
    salt: &mut u64,
) -> Option<Vec<NirOp>> {
    let mut copy = ops.to_vec();
    let declared = declared_names(&copy);
    if declared.is_empty() {
        return Some(copy);
    }
    let mut renames: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for name in &declared {
        loop {
            *salt += 1;
            let fresh = format!("{name}$dup{salt}");
            if census.count(&fresh) == 0 && !declared.contains(&fresh) {
                renames.insert(name.clone(), fresh);
                break;
            }
        }
    }
    for op in &mut copy {
        rename_op(op, &renames);
    }
    // Verification through the canonical walk: any surviving old name means
    // the hand walk missed a variant — refuse the clone.
    let renamed_census = super::reads::NameUses::census(&copy);
    if declared.iter().any(|name| renamed_census.count(name) != 0) {
        return None;
    }
    Some(copy)
}

fn rename(name: &mut String, renames: &std::collections::HashMap<String, String>) {
    if let Some(fresh) = renames.get(name.as_str()) {
        *name = fresh.clone();
    }
}

fn rename_op(op: &mut NirOp, renames: &std::collections::HashMap<String, String>) {
    match op {
        NirOp::Bind { name, value, .. } => {
            rename(name, renames);
            if let Some(value) = value {
                rename_value(value, renames);
            }
        }
        NirOp::StoreGlobal { value, .. } => {
            if let Some(value) = value {
                rename_value(value, renames);
            }
        }
        NirOp::Assign { name, value } => {
            rename(name, renames);
            rename_value(value, renames);
        }
        NirOp::StateAssign { resource, value } => {
            rename(resource, renames);
            rename_value(value, renames);
        }
        NirOp::Return { value } => {
            if let Some(value) = value {
                rename_value(value, renames);
            }
        }
        NirOp::ExitLoop { .. } | NirOp::ContinueLoop { .. } => {}
        NirOp::ExitProgram { code } => rename_value(code, renames),
        NirOp::Fail { error } => rename_value(error, renames),
        NirOp::Eval { value } => rename_value(value, renames),
        NirOp::If {
            condition,
            then_body,
            else_body,
        } => {
            rename_value(condition, renames);
            for op in then_body.iter_mut().chain(else_body.iter_mut()) {
                rename_op(op, renames);
            }
        }
        NirOp::Match { value, cases } => {
            rename_value(value, renames);
            for case in cases {
                if let Some(guard) = &mut case.guard {
                    rename_value(guard, renames);
                }
                for op in &mut case.body {
                    rename_op(op, renames);
                }
            }
        }
        NirOp::While {
            condition, body, ..
        } => {
            rename_value(condition, renames);
            for op in body {
                rename_op(op, renames);
            }
        }
        NirOp::For {
            name,
            start,
            end,
            step,
            body,
            ..
        } => {
            rename(name, renames);
            rename_value(start, renames);
            rename_value(end, renames);
            rename_value(step, renames);
            for op in body {
                rename_op(op, renames);
            }
        }
        NirOp::DoUntil { body, condition } => {
            for op in body.iter_mut() {
                rename_op(op, renames);
            }
            rename_value(condition, renames);
        }
        NirOp::ForEach {
            name,
            iterable,
            body,
            ..
        } => {
            rename(name, renames);
            rename_value(iterable, renames);
            for op in body {
                rename_op(op, renames);
            }
        }
        NirOp::Trap { name, body } => {
            rename(name, renames);
            for op in body {
                rename_op(op, renames);
            }
        }
    }
}

fn rename_value(value: &mut NirValue, renames: &std::collections::HashMap<String, String>) {
    match value {
        NirValue::Const { .. }
        | NirValue::Global { .. }
        | NirValue::FunctionRef { .. }
        | NirValue::Capture { .. } => {}
        NirValue::Local(name) => rename(name, renames),
        NirValue::LocalRef { name, .. } => rename(name, renames),
        NirValue::Closure { captures, .. } => {
            for capture in captures {
                rename_value(capture, renames);
            }
        }
        NirValue::Call { target, args, .. } | NirValue::CallResult { target, args, .. } => {
            // A call through a function-typed local names the local in its
            // `target` string, not as a `Local` value: rename it too (the
            // map holds only declared locals, so global function targets
            // never match).
            rename(target, renames);
            for arg in args {
                rename_value(arg, renames);
            }
        }
        NirValue::RuntimeCall { args, .. } | NirValue::Constructor { args, .. } => {
            // A runtime helper's target is a runtime symbol, never a local.
            for arg in args {
                rename_value(arg, renames);
            }
        }
        NirValue::UnionWrap { value, .. }
        | NirValue::UnionExtract { value, .. }
        | NirValue::ResultIsOk { value }
        | NirValue::ResultValue { value }
        | NirValue::ResultError { value } => rename_value(value, renames),
        NirValue::WithUpdate {
            target, updates, ..
        } => {
            rename_value(target, renames);
            for update in updates {
                rename_value(&mut update.value, renames);
            }
        }
        NirValue::ListLiteral { values, .. } | NirValue::SetLiteral { values, .. } => {
            for value in values {
                rename_value(value, renames);
            }
        }
        NirValue::MapLiteral { entries, .. } => {
            for (key, value) in entries {
                rename_value(key, renames);
                rename_value(value, renames);
            }
        }
        NirValue::MemberAccess { target, .. } => rename_value(target, renames),
        NirValue::Binary { left, right, .. } => {
            rename_value(left, renames);
            rename_value(right, renames);
        }
        NirValue::Unary { operand, .. } => rename_value(operand, renames),
    }
}
