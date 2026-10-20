//! plan-176-U: the vector-promotion and loop-local analyses (`promotable_vector_locals`, `scan_loop_locals`). The function lowering itself (`lower_function`) lives in
//! `engine/function/function_lowering/mod.rs`.

use super::*;

/// Collect every local *read* (`Local`) in `value`, via the shared NIR value
/// seam (bug-328).
fn collect_value_local_reads(value: &NirValue, out: &mut HashSet<String>) {
    use nir::visit::{walk_value, NirVisitor};
    struct Collector<'a> {
        out: &'a mut HashSet<String>,
    }
    impl NirVisitor for Collector<'_> {
        fn visit_value(&mut self, value: &NirValue) {
            if let NirValue::Local(name) = value {
                self.out.insert(name.clone());
            }
            walk_value(self, value);
        }
    }
    Collector { out }.visit_value(value);
}

/// Small-vector locals safe to keep in registers (their lanes) for their whole
/// lifetime, with no arena block (plan-01-vector). A candidate is a binding of a
/// vector type (`Float2/3/4`, `Fixed*`, `Integer*`) whose initializer produces a
/// register-native value — a vector construction or an inlined vector op — and
/// whose every use is *non-materializing* (a member read, or a direct argument to
/// an inlined vector op). Such a binding never needs a heap record. Excludes
/// address-taken and reassigned locals. Correctness does not hinge on precision
/// (`vector_value_as_block` materializes on demand); the analysis exists to avoid
/// promoting an *escaping* local, which would copy its block per use.
pub(crate) fn promotable_vector_locals(
    ops: &[NirOp],
    address_taken: &HashSet<String>,
) -> HashSet<String> {
    let mut candidates = HashSet::new();
    collect_vector_native_bindings(ops, &mut candidates);
    let mut reassigned = HashSet::new();
    collect_assigned_locals(ops, &mut reassigned);
    let mut escaping = HashSet::new();
    mark_vector_escaping_ops(ops, &mut escaping);
    candidates
        .into_iter()
        .filter(|name| {
            !address_taken.contains(name) && !reassigned.contains(name) && !escaping.contains(name)
        })
        .collect()
}

/// Whether `value` lowers to a register-native small vector (a vector constructor
/// or an inlined vector op), so a binding of it starts life in lanes.
fn is_vector_native_producing(value: &NirValue) -> bool {
    match value {
        NirValue::Constructor { type_, .. } => vector_field_count(type_).is_some(),
        NirValue::Call { target, args, .. } => vector_call_is_inlined(target, args),
        _ => false,
    }
}

fn collect_vector_native_bindings(ops: &[NirOp], out: &mut HashSet<String>) {
    for op in ops {
        match op {
            NirOp::Bind {
                name,
                type_,
                value: Some(value),
                ..
            } if vector_field_count(type_).is_some() && is_vector_native_producing(value) => {
                out.insert(name.clone());
            }
            NirOp::Bind { .. }
            | NirOp::StoreGlobal { .. }
            | NirOp::Assign { .. }
            | NirOp::StateAssign { .. }
            | NirOp::Return { .. }
            | NirOp::Eval { .. }
            | NirOp::Fail { .. }
            | NirOp::ExitProgram { .. }
            | NirOp::ExitLoop { .. }
            | NirOp::ContinueLoop { .. } => {}
            NirOp::If {
                then_body,
                else_body,
                ..
            } => {
                collect_vector_native_bindings(then_body, out);
                collect_vector_native_bindings(else_body, out);
            }
            NirOp::Match { cases, .. } => {
                for case in cases {
                    collect_vector_native_bindings(&case.body, out);
                }
            }
            NirOp::While { body, .. }
            | NirOp::DoUntil { body, .. }
            | NirOp::For { body, .. }
            | NirOp::ForEach { body, .. }
            | NirOp::Trap { body, .. } => collect_vector_native_bindings(body, out),
        }
    }
}

fn collect_assigned_locals(ops: &[NirOp], out: &mut HashSet<String>) {
    // Collect the target name of every `Assign` op via the shared NIR seam
    // (bug-328), which closes the silent `_ => {}` gap the hand-written match had.
    use nir::visit::{walk_op, NirVisitor};
    struct Collector<'a> {
        out: &'a mut HashSet<String>,
    }
    impl NirVisitor for Collector<'_> {
        fn visit_op(&mut self, op: &NirOp) {
            if let NirOp::Assign { name, .. } = op {
                self.out.insert(name.clone());
            }
            walk_op(self, op);
        }
    }
    Collector { out }.visit_ops(ops);
}

/// plan-142-F: whether `ops` can write the local `name` — an `Assign` to it, or a
/// by-reference capture of it (`LocalRef`, a `MUT` captured by a lambda) through
/// which a callback may assign it. Anywhere in `ops`, nested blocks included.
pub(crate) fn ops_write_local(ops: &[NirOp], name: &str) -> bool {
    use nir::visit::{walk_op, walk_value, NirVisitor};
    struct Finder<'a> {
        name: &'a str,
        found: bool,
    }
    impl NirVisitor for Finder<'_> {
        fn visit_op(&mut self, op: &NirOp) {
            if matches!(op, NirOp::Assign { name, .. } if name == self.name) {
                self.found = true;
            }
            walk_op(self, op);
        }
        fn visit_value(&mut self, value: &NirValue) {
            if matches!(value, NirValue::LocalRef { name, .. } if name == self.name) {
                self.found = true;
            }
            walk_value(self, value);
        }
    }
    let mut finder = Finder { name, found: false };
    finder.visit_ops(ops);
    finder.found
}

/// plan-145-H: whether `ops` write the `STATE` of the handle `resource` — a
/// `StateAssign` on it (`h.state = …`, `h.state.f = …`), or a by-reference capture
/// of the handle, anywhere in them.
pub(crate) fn ops_write_state(ops: &[NirOp], resource: &str) -> bool {
    use nir::visit::{walk_op, walk_value, NirVisitor};
    struct Finder<'a> {
        resource: &'a str,
        found: bool,
    }
    impl NirVisitor for Finder<'_> {
        fn visit_op(&mut self, op: &NirOp) {
            if matches!(op, NirOp::StateAssign { resource, .. } if resource == self.resource) {
                self.found = true;
            }
            walk_op(self, op);
        }
        fn visit_value(&mut self, value: &NirValue) {
            if matches!(value, NirValue::LocalRef { name, .. } if name == self.resource) {
                self.found = true;
            }
            walk_value(self, value);
        }
    }
    let mut finder = Finder {
        resource,
        found: false,
    };
    finder.visit_ops(ops);
    finder.found
}

/// Mark every local read in a *materializing* position — anything other than a
/// member read of the local or a direct argument to an inlined vector op.
fn mark_vector_escaping_value(value: &NirValue, out: &mut HashSet<String>) {
    match value {
        NirValue::Local(name) => {
            out.insert(name.clone());
        }
        // `a.x` reads a lane and does not materialize `a`; a deeper target recurses.
        NirValue::MemberAccess { target, .. } => {
            if !matches!(target.as_ref(), NirValue::Local(_)) {
                mark_vector_escaping_value(target, out);
            }
        }
        NirValue::Call { target, args, .. } => {
            let inlined = vector_call_is_inlined(target, args);
            for arg in args {
                if inlined && matches!(arg, NirValue::Local(_)) {
                    continue; // a lane-read argument to an inlined op
                }
                mark_vector_escaping_value(arg, out);
            }
        }
        NirValue::CallResult { args, .. }
        | NirValue::RuntimeCall { args, .. }
        | NirValue::Constructor { args, .. }
        | NirValue::ListLiteral { values: args, .. }
        | NirValue::SetLiteral { values: args, .. } => {
            for arg in args {
                mark_vector_escaping_value(arg, out);
            }
        }
        NirValue::UnionWrap { value, .. }
        | NirValue::UnionExtract { value, .. }
        | NirValue::ResultIsOk { value }
        | NirValue::ResultValue { value }
        | NirValue::ResultError { value }
        | NirValue::Checked { value, .. }
        | NirValue::Unary { operand: value, .. } => mark_vector_escaping_value(value, out),
        NirValue::Binary { left, right, .. } => {
            mark_vector_escaping_value(left, out);
            mark_vector_escaping_value(right, out);
        }
        NirValue::WithUpdate {
            target, updates, ..
        } => {
            mark_vector_escaping_value(target, out);
            for update in updates {
                mark_vector_escaping_value(&update.value, out);
            }
        }
        NirValue::MapLiteral { entries, .. } => {
            for (key, val) in entries {
                mark_vector_escaping_value(key, out);
                mark_vector_escaping_value(val, out);
            }
        }
        NirValue::Closure { captures, .. } => {
            for capture in captures {
                mark_vector_escaping_value(capture, out);
            }
        }
        NirValue::Const { .. }
        | NirValue::Global { .. }
        | NirValue::FunctionRef { .. }
        | NirValue::Capture { .. }
        | NirValue::LocalRef { .. } => {}
    }
}

fn mark_vector_escaping_ops(ops: &[NirOp], out: &mut HashSet<String>) {
    for op in ops {
        match op {
            NirOp::Bind { value, .. }
            | NirOp::StoreGlobal { value, .. }
            | NirOp::Return { value } => {
                if let Some(value) = value {
                    mark_vector_escaping_value(value, out);
                }
            }
            NirOp::Assign { value, .. }
            | NirOp::StateAssign { value, .. }
            | NirOp::Eval { value }
            | NirOp::ExitProgram { code: value }
            | NirOp::Fail { error: value } => mark_vector_escaping_value(value, out),
            NirOp::ExitLoop { .. } | NirOp::ContinueLoop { .. } => {}
            NirOp::If {
                condition,
                then_body,
                else_body,
            } => {
                mark_vector_escaping_value(condition, out);
                mark_vector_escaping_ops(then_body, out);
                mark_vector_escaping_ops(else_body, out);
            }
            NirOp::Match { value, cases } => {
                mark_vector_escaping_value(value, out);
                for case in cases {
                    if let NirMatchPattern::Value(v) = &case.pattern {
                        mark_vector_escaping_value(v, out);
                    }
                    if let NirMatchPattern::OneOf(values) = &case.pattern {
                        for v in values {
                            mark_vector_escaping_value(v, out);
                        }
                    }
                    if let Some(guard) = &case.guard {
                        mark_vector_escaping_value(guard, out);
                    }
                    mark_vector_escaping_ops(&case.body, out);
                }
            }
            NirOp::While {
                condition, body, ..
            }
            | NirOp::DoUntil { body, condition } => {
                mark_vector_escaping_value(condition, out);
                mark_vector_escaping_ops(body, out);
            }
            NirOp::For {
                start,
                end,
                step,
                body,
                ..
            } => {
                mark_vector_escaping_value(start, out);
                mark_vector_escaping_value(end, out);
                mark_vector_escaping_value(step, out);
                mark_vector_escaping_ops(body, out);
            }
            NirOp::ForEach { iterable, body, .. } => {
                mark_vector_escaping_value(iterable, out);
                mark_vector_escaping_ops(body, out);
            }
            NirOp::Trap { body, .. } => mark_vector_escaping_ops(body, out),
        }
    }
}

/// Walk a loop body collecting, at `depth` 0 (this loop's own level), the locals
/// directly assigned (`top_assigns` — the loop-carried-accumulator candidates),
/// and into `excluded` every local that is bound inside the body, is a loop
/// induction variable, or is read/assigned inside a *nested* loop (depth ≥ 1).
/// A candidate that is excluded is never promoted, so a nested loop always sees
/// the authoritative stack slot (plan-03 Stage D part 2).
pub(crate) fn scan_loop_locals(
    ops: &[NirOp],
    depth: u32,
    top_assigns: &mut HashSet<String>,
    excluded: &mut HashSet<String>,
) {
    let reads = |v: &NirValue, excluded: &mut HashSet<String>| {
        if depth >= 1 {
            collect_value_local_reads(v, excluded);
        }
    };
    for op in ops {
        match op {
            NirOp::Bind { name, value, .. } => {
                excluded.insert(name.clone());
                if let Some(v) = value {
                    reads(v, excluded);
                }
            }
            NirOp::Assign { name, value } => {
                if depth == 0 {
                    top_assigns.insert(name.clone());
                } else {
                    excluded.insert(name.clone());
                }
                reads(value, excluded);
            }
            NirOp::StoreGlobal { value, .. } => {
                if let Some(v) = value {
                    reads(v, excluded);
                }
            }
            NirOp::StateAssign { value, .. }
            | NirOp::Eval { value }
            | NirOp::ExitProgram { code: value }
            | NirOp::Fail { error: value } => reads(value, excluded),
            NirOp::Return { value } => {
                if let Some(v) = value {
                    reads(v, excluded);
                }
            }
            NirOp::ExitLoop { .. } | NirOp::ContinueLoop { .. } => {}
            NirOp::If {
                condition,
                then_body,
                else_body,
            } => {
                reads(condition, excluded);
                scan_loop_locals(then_body, depth, top_assigns, excluded);
                scan_loop_locals(else_body, depth, top_assigns, excluded);
            }
            NirOp::Match { value, cases } => {
                reads(value, excluded);
                for case in cases {
                    if let Some(guard) = &case.guard {
                        reads(guard, excluded);
                    }
                    scan_loop_locals(&case.body, depth, top_assigns, excluded);
                }
            }
            NirOp::While {
                condition, body, ..
            }
            | NirOp::DoUntil { body, condition } => {
                // A nested loop's condition reads its own locals regardless of
                // depth (bug-70: the former if/else ran the same call in both
                // branches).
                collect_value_local_reads(condition, excluded);
                scan_loop_locals(body, depth + 1, top_assigns, excluded);
            }
            NirOp::For {
                name,
                start,
                end,
                step,
                body,
                ..
            } => {
                excluded.insert(name.clone());
                reads(start, excluded);
                reads(end, excluded);
                reads(step, excluded);
                scan_loop_locals(body, depth + 1, top_assigns, excluded);
            }
            NirOp::ForEach {
                name,
                iterable,
                body,
                ..
            } => {
                excluded.insert(name.clone());
                reads(iterable, excluded);
                scan_loop_locals(body, depth + 1, top_assigns, excluded);
            }
            NirOp::Trap { body, .. } => scan_loop_locals(body, depth, top_assigns, excluded),
        }
    }
}
