// --- codegen tier imports (migration) ---
use crate::arch::ops::CodeOp;
use crate::codegen::builtins::vector::vector_call_is_inlined;
use crate::codegen::builtins::vector::vector_field_count;
use crate::codegen::compiler::opt::fma_fusion;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::mir;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::optimizer::opt2::peephole;
use crate::target::shared::abi;
use crate::target::shared::nir;
use crate::target::shared::nir::*;
use crate::types::ParameterType;
use std::collections::HashMap;
use std::collections::HashSet;
pub(crate) fn expanded_nir_union_variants<'a>(
    module: &'a NirModule,
    union_name: &str,
) -> Vec<&'a crate::target::shared::nir::NirVariant> {
    let Some(type_) = module
        .types
        .iter()
        .find(|candidate| candidate.kind == "union" && candidate.name == union_name)
    else {
        return Vec::new();
    };
    let mut variants = Vec::new();
    for include in &type_.includes {
        variants.extend(expanded_nir_union_variants(module, include));
    }
    variants.extend(type_.variants.iter());
    variants
}

/// Collect the names of every local whose address is taken (`LocalRef`) anywhere
/// in `ops`. A loop-promoted local must have *no* such slot reference, since a callback
/// holding it could read or mutate the slot while the value lives only in
/// a register (plan-03 Stage D part 2). Descends through the shared NIR seam
/// (bug-328), whose `walk_*` recursion is exhaustive — missing a `LocalRef`
/// would be unsound, and a new value variant is a compile error in one place.
pub(crate) fn collect_address_taken_locals(ops: &[NirOp], out: &mut HashSet<String>) {
    use nir::visit::{walk_value, NirVisitor};
    struct Collector<'a> {
        out: &'a mut HashSet<String>,
    }
    impl NirVisitor for Collector<'_> {
        fn visit_value(&mut self, value: &NirValue) {
            if let NirValue::LocalRef { name, .. } = value {
                self.out.insert(name.clone());
            }
            walk_value(self, value);
        }
    }
    Collector { out }.visit_ops(ops);
}

/// plan-86 G1: names of every local REASSIGNED anywhere in `ops` — rebound
/// (`Bind`), assigned (`Assign`), or state-mutated (`StateAssign`). For the
/// bounds-check elision, the induction var `i` and the list `L` must both be
/// absent from this set over the loop body, so `i < len(L)` provably holds at
/// every iteration (a reassigned `L` changes its length; a reassigned `i` breaks
/// the `0..=len-k` range). Uses the exhaustive `NirVisitor` seam so a new
/// mutation op cannot silently escape the check — a missed reassignment would be
/// an UNSOUND unchecked access (silent OOB).
pub(crate) fn collect_reassigned_locals(ops: &[NirOp]) -> HashSet<String> {
    use nir::visit::{walk_op, NirVisitor};
    struct Collector {
        out: HashSet<String>,
    }
    impl NirVisitor for Collector {
        fn visit_op(&mut self, op: &NirOp) {
            match op {
                NirOp::Bind { name, .. } => {
                    self.out.insert(name.clone());
                }
                NirOp::Assign { name, .. } => {
                    self.out.insert(name.clone());
                }
                NirOp::StateAssign { resource, .. } => {
                    self.out.insert(resource.clone());
                }
                _ => {}
            }
            walk_op(self, op);
        }
    }
    let mut c = Collector {
        out: HashSet::new(),
    };
    c.visit_ops(ops);
    c.out
}

/// plan-86 K1: whether EVERY value-return in `f` returns a bare PARAMETER that is
/// never mutated or address-taken in the body — so `f`'s return value is always a
/// BORROW of one of the caller's argument blocks, NEVER a freshly allocated one
/// (the pure passthrough / identity shape, e.g. `FUNC copyStrs(xs) RETURN xs`).
///
/// For such a function the callee-side return deep-copy is elided (it returns the
/// parameter pointer directly), and every caller treats the call result as an
/// aliasing source (`value_needs_owning_copy`): `register_pending_temp` does not
/// free it, and `lower_value_owned` deep-copies it at any owning store. The net
/// effect is that the one deep-copy MOVES from the callee to the caller's ownership
/// boundary — a read-only-and-discarded result pays no copy at all, while a result
/// stored into an owned binding is copied exactly once, so observable value
/// semantics is byte-identical to the always-copy behavior it replaces.
///
/// Both the callee skip and the caller's aliasing classification key off THIS one
/// predicate, so they can never disagree (a disagreement would leak the callee's
/// copy or double-free the caller's argument). Conservative in the safe direction:
/// requires at least one value-return, EVERY value-return to be a bare parameter,
/// and no returned parameter to be reassigned (`Bind`/`Assign`/`StateAssign`) or
/// address-taken anywhere in the body — any doubt keeps the copy. Uses the
/// exhaustive `NirVisitor` seam so a new return/mutation route cannot silently slip
/// the gate (a missed mutation would return a modified block as if it were the
/// caller's untouched argument).
pub(crate) fn function_returns_param_borrow(
    f: &NirFunction,
    callback_referenced: &HashSet<String>,
) -> bool {
    use nir::visit::{walk_op, NirVisitor};
    // A function used as a `FunctionRef` (callback) is invoked through an ABI that
    // OWNS and frees its return value, so it must keep returning a fresh copy — never
    // a parameter borrow (see `collect_function_ref_names`).
    if callback_referenced.contains(&f.name) {
        return false;
    }
    let params: HashSet<String> = f.params.iter().map(|p| p.name.clone()).collect();
    if params.is_empty() {
        return false;
    }
    struct Returns<'a> {
        params: &'a HashSet<String>,
        any: bool,
        all_param: bool,
        returned: HashSet<String>,
    }
    impl NirVisitor for Returns<'_> {
        fn visit_op(&mut self, op: &NirOp) {
            if let NirOp::Return { value: Some(v) } = op {
                self.any = true;
                match v {
                    NirValue::Local(name) if self.params.contains(name) => {
                        self.returned.insert(name.clone());
                    }
                    _ => self.all_param = false,
                }
            }
            walk_op(self, op);
        }
    }
    let mut r = Returns {
        params: &params,
        any: false,
        all_param: true,
        returned: HashSet::new(),
    };
    r.visit_ops(&f.body);
    if !r.any || !r.all_param {
        return false;
    }
    // A returned parameter that is reassigned or address-taken is no longer
    // guaranteed to be the caller's untouched argument block, so the "borrow" could
    // alias a modified/freed local — bail (keep the copy).
    let reassigned = collect_reassigned_locals(&f.body);
    let mut address_taken = HashSet::new();
    collect_address_taken_locals(&f.body, &mut address_taken);
    !r.returned
        .iter()
        .any(|p| reassigned.contains(p) || address_taken.contains(p))
}

/// plan-77 M6: names of every local used as a VALUE — read (`Local`) or
/// address-taken (`LocalRef`) — anywhere in `ops`. A closure binding whose name
/// is NOT in this set never flows anywhere except as a direct call target (an
/// invoke lowers `Call { target: name }`, whose `target` is a String, not a
/// `NirValue`, so it is not visited): it is not returned, passed as an argument,
/// stored, captured, or aliased, so it is provably dead at scope end and safe to
/// free. Reuses the exhaustive `NirVisitor` seam (a new value variant is a
/// compile error in `walk_value`), so no escape route can be silently missed —
/// which for a would-be-freed closure is the difference between a reclaim and a
/// use-after-free. Conservative in the safe direction: any doubt keeps the name.
pub(crate) fn collect_value_used_locals(ops: &[NirOp], out: &mut HashSet<String>) {
    use nir::visit::{walk_value, NirVisitor};
    struct Collector<'a> {
        out: &'a mut HashSet<String>,
    }
    impl NirVisitor for Collector<'_> {
        fn visit_value(&mut self, value: &NirValue) {
            match value {
                NirValue::Local(name) | NirValue::LocalRef { name, .. } => {
                    self.out.insert(name.clone());
                }
                _ => {}
            }
            walk_value(self, value);
        }
    }
    Collector { out }.visit_ops(ops);
}

/// plan-86 E: names of every binding `e = collections::get(L, i)` (or `getOr`)
/// whose result is consumed READ-ONLY — `e` is used only (and at least once) as a
/// `MATCH` scrutinee (`NirOp::Match { value: Local(e) }`), never stored/returned/
/// mutated/address-taken — AND whose CONTAINER `L` is a plain Local that is
/// immutable through the scope (bound at most once, never `Assign`ed/`StateAssign`ed,
/// not address-taken). For such a binding, `get` may return an aliasing borrow into
/// `L`'s inline element instead of a fresh `copy_flat_block` (the ~4M-copy/rep cost,
/// `dispatch union`), and the binding registers no scope-drop free.
///
/// SOUNDNESS: a read-only borrow that outlives its container, or is freed, is a
/// dangling read / double-free into the container's data region. So `L` must be
/// immutable (a reassign frees `L`'s old block while `e` still points into it) and
/// `e` must be read-only and never freed. The freeable-flat-non-String element gate
/// is applied at codegen (a String `get` returns an OWNED fresh block, which keeps
/// its copy + free); the copy-skip and the free-skip are gated on this same set.
/// Conservative in the safe direction — any doubt drops the name (uses the
/// exhaustive `NirVisitor` seam so a new escape route is a compile error).
pub(crate) fn collect_borrow_get_locals(
    ops: &[NirOp],
    address_taken: &HashSet<String>,
) -> HashSet<String> {
    use nir::visit::{walk_op, walk_value, NirVisitor};
    use std::collections::HashMap;
    struct Collector {
        get_container: HashMap<String, String>, // e  -> L   (get-bindings)
        copy_src: HashMap<String, String>,      // m  -> src (Bind { value: Local(src) })
        bind_counts: HashMap<String, usize>,
        assigned: HashSet<String>,
        total_reads: HashMap<String, usize>,
        scrutinee_reads: HashMap<String, usize>,
        // Reads of a local as the value of a `UnionExtract` — the MATCH's own
        // read-only variant bindings (`n = UnionExtract($matchN)`), part of
        // consuming the scrutinee, not an escape.
        union_extract_reads: HashMap<String, usize>,
    }
    impl NirVisitor for Collector {
        fn visit_op(&mut self, op: &NirOp) {
            match op {
                NirOp::Bind { name, value, .. } => {
                    *self.bind_counts.entry(name.clone()).or_insert(0) += 1;
                    match value {
                        Some(NirValue::Call { target, args, .. })
                            if matches!(
                                crate::codegen::builtins::native_builtin_target(target),
                                Some("get") | Some("getOr")
                            ) =>
                        {
                            if let Some(NirValue::Local(container)) = args.first() {
                                self.get_container.insert(name.clone(), container.clone());
                            }
                        }
                        Some(NirValue::Local(src)) => {
                            self.copy_src.insert(name.clone(), src.clone());
                        }
                        _ => {}
                    }
                }
                NirOp::Assign { name, .. } => {
                    self.assigned.insert(name.clone());
                }
                NirOp::StateAssign { resource, .. } => {
                    self.assigned.insert(resource.clone());
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
        get_container: HashMap::new(),
        copy_src: HashMap::new(),
        bind_counts: HashMap::new(),
        assigned: HashSet::new(),
        total_reads: HashMap::new(),
        scrutinee_reads: HashMap::new(),
        union_extract_reads: HashMap::new(),
    };
    c.visit_ops(ops);
    let Collector {
        get_container,
        copy_src,
        bind_counts,
        assigned,
        total_reads,
        scrutinee_reads,
        union_extract_reads,
    } = c;
    let reads = |n: &str| total_reads.get(n).copied().unwrap_or(0);
    let scrut = |n: &str| scrutinee_reads.get(n).copied().unwrap_or(0);
    let extract = |n: &str| union_extract_reads.get(n).copied().unwrap_or(0);
    // `m` is a MATCH-scrutinee temp: used only by the MATCH — as its scrutinee (≥1)
    // and as the value of the case `UnionExtract`s (its read-only variant bindings),
    // nothing else. The IR desugars `MATCH e` into `$matchN = e; MATCH $matchN`, so
    // the scrutinee is this temp, not `e`.
    let is_match_temp =
        |m: &str| scrut(m) >= 1 && reads(m) == scrut(m) + extract(m) && !address_taken.contains(m);
    // Count the match-temp copy-binds `m = Local(src)` per source `src`.
    let mut match_temp_binds_of: HashMap<&str, usize> = HashMap::new();
    for (m, src) in &copy_src {
        if is_match_temp(m) {
            *match_temp_binds_of.entry(src.as_str()).or_insert(0) += 1;
        }
    }
    // Container `L` is immutable through the scope (bound ≤1, never reassigned, not
    // address-taken) — a reassign frees `L`'s old block while a borrow points into it.
    let l_immutable = |l: &str| {
        bind_counts.get(l).copied().unwrap_or(0) <= 1
            && !assigned.contains(l)
            && !address_taken.contains(l)
    };
    let mut result: HashSet<String> = HashSet::new();
    // A get-binding `e = get(L, i)` is a read-only borrow when EVERY read of `e` is
    // borrow-transparent — a direct MATCH scrutinee, or the value of a match-temp
    // copy-bind `$matchN = e` — and `L` is immutable and `e` not address-taken.
    for (e, l) in &get_container {
        let transparent = scrut(e) + match_temp_binds_of.get(e.as_str()).copied().unwrap_or(0);
        if reads(e) >= 1 && reads(e) == transparent && !address_taken.contains(e) && l_immutable(l)
        {
            result.insert(e.clone());
        }
    }
    // A match-temp `$matchN = Local(e)` that copies a get-borrow `e` also borrows
    // (aliases `e`), so the container element flows into `MATCH` with zero copies.
    for (m, src) in &copy_src {
        if is_match_temp(m) && result.contains(src) {
            result.insert(m.clone());
        }
    }
    result
}

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

/// Inline-conversion built-ins that produce a raw `Result` under an inline
/// `TRAP` (`lower_inline_conversion_raw`). Their error path is a single
/// `emit_error_register_return` seam, which is what plan-64-I elides.
fn is_trap_discard_conversion(target: &str) -> bool {
    matches!(
        target,
        "toInt" | "toFloat" | "toFixed" | "toByte" | "toMoney" | "toScalar"
    )
}

/// plan-64-I: names of inline-conversion `CallResult` Result-locals whose
/// trapped error is provably unused. `CallResult` is produced *only* by the
/// inline-`TRAP` desugar (`ir::lower::lower_inline_trap`), which binds the raw
/// `Result` to a temp consumed solely by
/// `ResultIsOk`/`ResultValue`/`ResultError`. So a conversion `CallResult` local
/// is error-discardable when the local its `ResultError` flows into is never
/// read (the `RECOVER` handler ignores `err`), or when no `ResultError` of it
/// exists at all. Such a local's `Error` object is never observed, so the error
/// path can skip building the ErrorLoc + flat `Error` block and keep only the
/// tag. Conservative: an unrelated read of an identically-named local only keeps
/// the (correct) full error build.
///
/// The desugar has **two** shapes and both must be recognised (bug-457):
///
/// * one check — `Bind err = ResultError(result)` in the `If`'s else arm;
/// * a check chain — `Assign $trap_errN = ResultError(result)` into the shared
///   error slot, because the chain reports through one slot and binds `err` from
///   it once, after the branches.
///
/// Matching only the `Bind` form silently mis-classified every chain as
/// error-discardable: codegen then emitted the error tag with no `Error` block
/// while the `Assign` went on to read one, killing the process (the acceptance
/// suite's `expectTrap(toInt(toFloat("1e20")), …)`). An unrecognised shape here
/// is not a missed optimisation, it is a miscompile — so this must be kept in
/// step with `lower_inline_trap`.
fn trap_discard_error_results(ops: &[NirOp]) -> HashSet<String> {
    use nir::visit::{walk_op, walk_value, NirVisitor};
    struct Collector {
        // Conversion `CallResult` Result-locals.
        candidates: HashSet<String>,
        // Result-local -> the `err` local bound from `ResultError(result)`.
        err_binding: HashMap<String, String>,
        // Every `Local` read anywhere in the function.
        reads: HashSet<String>,
    }
    impl NirVisitor for Collector {
        fn visit_op(&mut self, op: &NirOp) {
            // Both desugar shapes land the error in a named local: the one-check
            // form binds it, the check chain assigns it into the shared slot.
            let bound = match op {
                NirOp::Bind {
                    name,
                    value: Some(value),
                    ..
                } => Some((name, value)),
                NirOp::Assign { name, value } => Some((name, value)),
                _ => None,
            };
            if let Some((name, value)) = bound {
                match value {
                    NirValue::CallResult { target, .. } if is_trap_discard_conversion(target) => {
                        self.candidates.insert(name.clone());
                    }
                    NirValue::ResultError { value: inner } => {
                        if let NirValue::Local(result) = inner.as_ref() {
                            self.err_binding.insert(result.clone(), name.clone());
                        }
                    }
                    _ => {}
                }
            }
            walk_op(self, op);
        }
        fn visit_value(&mut self, value: &NirValue) {
            if let NirValue::Local(name) = value {
                self.reads.insert(name.clone());
            }
            walk_value(self, value);
        }
    }
    let mut collector = Collector {
        candidates: HashSet::new(),
        err_binding: HashMap::new(),
        reads: HashSet::new(),
    };
    collector.visit_ops(ops);
    let Collector {
        candidates,
        err_binding,
        reads,
    } = collector;
    candidates
        .into_iter()
        .filter(|name| match err_binding.get(name) {
            Some(err_local) => !reads.contains(err_local),
            None => true,
        })
        .collect()
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

#[allow(clippy::too_many_arguments)]
pub(crate) fn lower_function(
    function: &NirFunction,
    function_symbols: &HashMap<String, String>,
    functions: &HashMap<String, &NirFunction>,
    package_return_types: &HashMap<String, ParameterType>,
    platform_imports: &HashMap<String, String>,
    platform: &dyn crate::codegen::engine::types::CodegenPlatform,
    build_mode: crate::target::NativeBuildMode,
    globals: &HashMap<String, GlobalValue>,
    string_symbols: &HashMap<String, String>,
    callback_referenced_functions: &HashSet<String>,
    // plan-118-D: the record types with their own `construct.T` function.
    synthesized_constructors: &HashSet<ParameterType>,
    type_model: TypeModel,
) -> Result<CodeFunction, String> {
    let params = function
        .params
        .iter()
        .enumerate()
        .map(|(index, param)| {
            // Arguments 0..8 arrive in `x0`–`x7`; the rest arrive in the caller's
            // stack tail (bug-08). A stack parameter has no argument register, so
            // its `location` records the tail slot instead (never emitted as a
            // register — the prologue below loads it via `incoming_stack_arg_load`).
            let location = if index < abi::REGISTER_ARGUMENT_COUNT {
                // Typed convention token — a parameter's register location stays
                // Operand::Abi so it is realized by each backend's typed handler
                // (never a Raw convention string). plan-85-D Phase 3.
                abi::argument_register(index)?
            } else {
                // Stack-tail sentinel (bug-08); a Raw marker the prologue interprets.
                Operand::from(format!("stack{}", index - abi::REGISTER_ARGUMENT_COUNT))
            };
            Ok(CodeParam {
                name: param.name.clone(),
                type_: param.type_.clone().name().into_owned(),
                location,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;

    let mut builder = CodeBuilder {
        current_symbol: nir::function_symbol(&function.name),
        function_symbols,
        functions,
        package_return_types,
        platform_imports,
        platform,
        build_mode,
        globals,
        type_model,
        string_symbols,
        locals: HashMap::new(),
        instructions: vec![abi::label("entry")],
        relocations: Vec::new(),
        stack_slots: Vec::new(),
        used_callee_saved: Vec::new(),
        stack_size: 0,
        next_register: 8,
        next_vreg: 0,
        next_fp_vreg: 0,
        float_residents: HashMap::new(),
        promoted_float_locals: HashMap::new(),
        address_taken_locals: HashSet::new(),
        value_used_locals: HashSet::new(),
        borrow_get_locals: HashSet::new(),
        borrow_get_result: false,
        current_returns_param_borrow: false,
        callback_referenced_functions: HashSet::new(),
        // A helper body constructs nothing through the NIR arm.
        synthesized_constructors: synthesized_constructors.clone(),
        next_label: 0,
        trap: None,
        loop_stack: Vec::new(),
        active_cleanups: Vec::new(),
        cleanup_scope_starts: Vec::new(),
        pending_result_slots: None,
        escaping_value_slot: None,
        raw_result_capture: None,
        trap_discard_error_results: HashSet::new(),
        raw_result_discard_error: false,
        suppress_resource_source_flag: false,
        emitting_error_route: false,
        building_error_block: false,
        current_file: function.file.clone(),
        current_loc: NirSourceLoc::default(),
        owner_containers: function
            .resource_owners
            .values()
            .filter_map(|owner| match owner {
                crate::ir::resource_escape::ResOwner::Float(name) => Some(name.clone()),
                _ => None,
            })
            .collect(),
        resource_owners: function.resource_owners.clone(),
        owned_list_heads: HashMap::new(),
        owned_value_slots: Vec::new(),
        pending_temp_frees: Vec::new(),
        operand_snapshot_wanted: Vec::new(),
        for_each_iterable_locals: Vec::new(),
        for_each_iterable_state_fields: Vec::new(),
        for_each_iterable_record_fields: Vec::new(),
        string_capacity_slots: HashMap::new(),
        math_pool_base_vreg: None,
        vector_natives: HashMap::new(),
        next_vector_native: 0,
        promoted_vector_locals: HashMap::new(),
        promotable_vector_locals: HashSet::new(),
        integer_lower_bounds: HashMap::new(),
        integer_strict_upper: std::collections::HashSet::new(),
        for_bound_expr: HashMap::new(),
        len_of_local: HashMap::new(),
        provable_index_locals: HashMap::new(),
        enclosing_loop_reassigned: Vec::new(),
    };
    for (index, param) in params.iter().enumerate() {
        let stack_offset = builder.allocate_stack_object(&param.name, 8);
        builder.locals.insert(
            param.name.clone(),
            LocalValue {
                // The typed NIR param, not the rendered `CodeParam` string —
                // `params[i]` was built 1:1 from `function.params[i]` above.
                type_: function.params[index].type_.clone(),
                stack_offset,
                constant: None,
                by_ref: false,
            },
        );
        if index < abi::REGISTER_ARGUMENT_COUNT {
            builder.emit(abi::store_u64(
                &param.location,
                abi::stack_pointer(),
                stack_offset,
            ));
        } else {
            // A stack parameter is loaded from the incoming stack tail (resolved
            // to an `sp`-relative offset in `finalize_frame`) and spilled into its
            // local slot like a register parameter (bug-08).
            let scratch = builder.temporary_vreg();
            builder.emit(abi::incoming_stack_arg_load(
                &scratch,
                index - abi::REGISTER_ARGUMENT_COUNT,
            ));
            builder.emit(abi::store_u64(&scratch, abi::stack_pointer(), stack_offset));
            builder.reset_temporary_registers();
        }
        // The TYPED NIR param, not the rendered `CodeParam` string beside it.
        if CodeBuilder::is_thread_type(&function.params[index].type_) {
            builder
                .active_cleanups
                .push(ActiveCleanup::Thread(ThreadCleanup {
                    name: param.name.clone(),
                    symbol: CodeBuilder::thread_drop_symbol(),
                }));
        }
    }
    if let Some(name) = function.body.iter().find_map(|op| match op {
        NirOp::Trap { name, .. } => Some(name.clone()),
        _ => None,
    }) {
        let stack_offset = builder.allocate_stack_object(&name, 8);
        builder.locals.insert(
            name.clone(),
            LocalValue {
                type_: ParameterType::named("Error"),
                stack_offset,
                constant: None,
                by_ref: false,
            },
        );
        let label = builder.label("trap");
        builder.trap = Some(TrapState {
            name,
            label,
            in_trap_body: false,
            stack_offset,
        });
    }
    // Pre-allocate the capacity shadow slot for every in-place string self-append
    // target so bind/assign sites can reset it and the prologue can zero it.
    builder.prescan_string_self_appends(&function.body);
    // Locals whose address is taken anywhere — never loop-promoted (plan-03 D2).
    collect_address_taken_locals(&function.body, &mut builder.address_taken_locals);
    collect_value_used_locals(&function.body, &mut builder.value_used_locals);
    // plan-86 E: read-only `get`-borrow bindings (needs address_taken above).
    builder.borrow_get_locals =
        collect_borrow_get_locals(&function.body, &builder.address_taken_locals);
    // Small-vector locals that can live in registers for their whole lifetime with
    // no arena block (plan-01-vector).
    builder.promotable_vector_locals =
        promotable_vector_locals(&function.body, &builder.address_taken_locals);
    // plan-64-I: inline-conversion `CallResult` Result-locals whose trapped error
    // is provably unused, so the error path builds only a tag (no ErrorLoc/Error).
    builder.trap_discard_error_results = trap_discard_error_results(&function.body);
    // plan-86 K1: a pure parameter-passthrough function returns its argument uncopied;
    // the caller copies at its ownership boundary. Only the user-function path sets
    // this — builtin/thread-copy wrappers default false (never param-borrow, and no
    // caller looks them up in `functions`), keeping callee-skip and caller-classify
    // consistent.
    builder.callback_referenced_functions = callback_referenced_functions.clone();
    builder.current_returns_param_borrow =
        function_returns_param_borrow(function, callback_referenced_functions);
    {
        // `-vv` span (`crate::trace`): emission is one of the four big
        // per-function costs, and separating it from allocation is what says
        // whether a slow function is slow to *lower* or slow to *color*.
        let _span = crate::trace::span("emit ops");
        builder.lower_ops(&function.body)?;
        if !builder.current_block_returns() {
            builder.emit_return_exit(None)?;
        }
    }
    // Fuse single-use `a*b ± c` float chains into one single-rounded fused op
    // (plan-02 Phase 3) before allocation, so the fused op's operands are colored
    // as a unit. A no-op unless the `d`-native FP virtual registers are present.
    {
        let _span = crate::trace::span("fma fusion");
        fma_fusion::fuse_scalar_fma(&mut builder.instructions);
    }
    // Color virtual registers to physical registers (plan-03 Stage A) before the
    // body is moved out for the peephole pass and finalize_frame.
    builder.run_register_allocation()?;
    let mut instructions = builder.instructions;
    // Zero every string self-append capacity shadow at function entry: the buffer a
    // parameter or first assignment hands the local is tight (no spare). Stores are
    // sp-relative with pre-prologue offsets; `finalize_frame` shifts them like every
    // other stack access. The shadow is reset on every later non-self-append
    // bind/assign, so it always reflects the live buffer's spare bytes.
    if !builder.string_capacity_slots.is_empty() {
        // Store the zero token (`xzr`) directly — no scratch register, no `mov`
        // (plan-34-C: shared lowering names no physical scratch).
        let mut zeroing = Vec::new();
        let mut slots: Vec<usize> = builder.string_capacity_slots.values().copied().collect();
        slots.sort_unstable();
        for slot in slots {
            zeroing.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), slot));
        }
        let insert_at = if instructions
            .first()
            .is_some_and(|instruction| instruction.op == CodeOp::Label)
        {
            1
        } else {
            0
        };
        instructions.splice(insert_at..insert_at, zeroing);
    }
    // Zero every owned freeable-flat slot at entry so a scope-drop skips any
    // binding or temporary whose initializer never executed (its null guard sees
    // 0 instead of stack garbage). A trap handler can jump past a not-yet-run
    // `LET`, but the same hazard exists without a trap — a scope-drop over a
    // temporary that a given path leaves unwritten frees whatever the stack held
    // (benign on AArch64 where the slot happened to be zero, a wild free on x86).
    // The stores are sp-relative with pre-prologue offsets, so `finalize_frame`
    // shifts them by the callee-save area like every other stack access.
    if !builder.owned_value_slots.is_empty() {
        // Store the zero token (`xzr`) directly — no scratch register, no `mov`.
        let mut zeroing = Vec::new();
        let mut slots = builder.owned_value_slots.clone();
        slots.sort_unstable();
        slots.dedup();
        for slot in slots {
            zeroing.push(abi::store_u64(abi::ZERO, abi::stack_pointer(), slot));
        }
        let insert_at = if instructions
            .first()
            .is_some_and(|instruction| instruction.op == CodeOp::Label)
        {
            1
        } else {
            0
        };
        instructions.splice(insert_at..insert_at, zeroing);
    }
    // Store-to-load forwarding over the lowered stream (offsets are still
    // pre-prologue here, before finalize_frame shifts them).
    // bug-284 C8: x86-64's mul/div/msub expansions clobber rdx:rax beyond their
    // named dst, so the forwarder must flush across them. Read the ISA the same
    // way `remove_fp_shuttles` does -- from the active backend's arena base --
    // rather than sniffing operand spellings.
    let is_x86 = mir::active_backend().register_model().arena_base()
        == crate::arch::x86_64::regmodel::ARENA_BASE_REGISTER;
    {
        let _span = crate::trace::span("peephole: store-to-load");
        peephole::forward_stores_to_loads(&mut instructions, is_x86);
    }
    // Drop the GP shuttle a checked float value round-trips through (plan-16). The
    // FP-shuttle liveness derives its call-clobber mask from the active backend's
    // register model, not from operand spellings (bug-350).
    {
        let _span = crate::trace::span("peephole: fp shuttles");
        peephole::remove_fp_shuttles(&mut instructions, mir::active_backend().register_model());
    }
    let mut stack_slots = builder.stack_slots;
    let frame = {
        let _span = crate::trace::span("finalize frame");
        finalize_frame(
            &mut instructions,
            &mut stack_slots,
            builder.stack_size,
            builder.used_callee_saved,
        )
    };
    crate::trace::count("machine instructions", instructions.len() as u64);
    // plan-118-A: the size twin of the "slowest lower_function" leaderboard a
    // few lines of instrumentation up the stack. Lowering time and emitted size
    // are different axes — a function can be slow to lower because it is
    // quadratic in something small, or fast to lower and still be 6% of the
    // module — so "is the expansion one pathological function or all of them?"
    // is only answerable from this one.
    crate::trace::size_item(
        "lower_function",
        || function.name.clone(),
        instructions.len() as u64,
    );

    Ok(CodeFunction {
        name: function.name.clone(),
        symbol: nir::function_symbol(&function.name),
        params,
        returns: function.returns.name().into_owned(),
        frame,
        instructions,
        relocations: builder.relocations,
        stack_slots,
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn lower_builtin_function_wrapper(
    name: &str,
    type_: &ParameterType,
    symbol: &str,
    function_symbols: &HashMap<String, String>,
    functions: &HashMap<String, &NirFunction>,
    package_return_types: &HashMap<String, ParameterType>,
    platform_imports: &HashMap<String, String>,
    platform: &dyn crate::codegen::engine::types::CodegenPlatform,
    build_mode: crate::target::NativeBuildMode,
    globals: &HashMap<String, GlobalValue>,
    string_symbols: &HashMap<String, String>,
    type_model: TypeModel,
) -> Result<CodeFunction, String> {
    let (params, returns) = match type_ {
        ParameterType::Func(params, returns, false) => (params, returns),
        _ => {
            return Err(format!(
                "native built-in function wrapper has malformed function type '{type_}'"
            ))
        }
    };
    if params.len() != 1 || !matches!(returns.as_ref(), ParameterType::Boolean) {
        return Err(format!(
            "native built-in function wrapper expects a unary Boolean function, got '{type_}'"
        ));
    }

    let param = CodeParam {
        name: "value".to_string(),
        type_: params[0].name().into_owned(),
        location: abi::argument_register(0)?,
    };
    let mut builder = CodeBuilder::for_synthetic_function(
        symbol,
        function_symbols,
        functions,
        package_return_types,
        platform_imports,
        platform,
        build_mode,
        globals,
        string_symbols,
        type_model,
    );

    let stack_offset = builder.allocate_stack_object("value", 8);
    builder.locals.insert(
        "value".to_string(),
        LocalValue {
            type_: params[0].clone(),
            stack_offset,
            constant: None,
            by_ref: false,
        },
    );
    builder.emit(abi::store_u64(
        &param.location,
        abi::stack_pointer(),
        stack_offset,
    ));

    let result = builder.lower_value(&NirValue::Call {
        target: name.to_string(),
        args: vec![NirValue::Local("value".to_string())],
        loc: NirSourceLoc::default(),
    })?;
    builder.emit(abi::move_register(RESULT_VALUE_REGISTER, &result.location));
    builder.emit(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    builder.emit(abi::return_());

    builder.run_register_allocation()?;
    let mut instructions = builder.instructions;
    // bug-284 C8: x86-64's mul/div/msub expansions clobber rdx:rax beyond their
    // named dst, so the forwarder must flush across them. Read the ISA the same
    // way `remove_fp_shuttles` does -- from the active backend's arena base --
    // rather than sniffing operand spellings.
    let is_x86 = mir::active_backend().register_model().arena_base()
        == crate::arch::x86_64::regmodel::ARENA_BASE_REGISTER;
    peephole::forward_stores_to_loads(&mut instructions, is_x86);
    peephole::remove_fp_shuttles(&mut instructions, mir::active_backend().register_model());
    let mut stack_slots = builder.stack_slots;
    let frame = finalize_frame(
        &mut instructions,
        &mut stack_slots,
        builder.stack_size,
        builder.used_callee_saved,
    );

    Ok(CodeFunction {
        name: format!("builtin.{name}.{type_}"),
        symbol: symbol.to_string(),
        params: vec![param],
        returns: returns.name().into_owned(),
        frame,
        instructions,
        relocations: builder.relocations,
        stack_slots,
    })
}

/// The experimental `AbiFunction` wrapper — the unified successor to `OsLower`.
/// Builds a `CodeBuilder`, binds the incoming ABI argument registers as
/// **pre-lowered** `ValueResult`s (captured into vregs at entry, since the physical
/// arg registers are only live-in at the prologue), runs the registered
/// [`crate::codegen::registry::AbiFunction`] body, then wraps its returned
/// `ValueResult` in the shared runtime-helper fallible convention (value in
/// `RESULT_VALUE_REGISTER`, `RESULT_OK_TAG` in `RESULT_TAG_REGISTER`) and finalizes
/// into the `(frame, instructions, relocations, stack_slots)` tuple every runtime
/// helper produces. Reached from `lower_runtime_helper`'s `abi.*` arm. The demo
/// `abi::funcAddTwo` member routes here.
#[allow(clippy::too_many_arguments)]
pub(crate) fn lower_abi_function_helper(
    call: &str,
    symbol: &str,
    build_mode: crate::target::NativeBuildMode,
    module_name: &str,
    type_model: &TypeModel,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    string_symbols: &HashMap<String, String>,
    term_state_offset: Option<usize>,
    presentation_mode_offset: Option<usize>,
    arena_global_slots: usize,
    uses_rng: bool,
) -> Result<
    (
        CodeFrame,
        Vec<CodeInstruction>,
        Vec<CodeRelocation>,
        Vec<CodeStackSlot>,
    ),
    String,
> {
    let (lower, arity) = crate::codegen::registry::abi_function_lower(call)
        .ok_or_else(|| format!("native code plan has no AbiFunction lowering for '{call}'"))?;

    // A runtime helper is standalone: no user functions/globals/strings are in
    // scope. Empty tables suffice for a builder-driven body that only computes over
    // its argument registers (the demo). Kept as locals the builder borrows for the
    // duration of this call.
    let function_symbols: HashMap<String, String> = HashMap::new();
    let functions: HashMap<String, &NirFunction> = HashMap::new();
    let package_return_types: HashMap<String, ParameterType> = HashMap::new();
    let globals: HashMap<String, GlobalValue> = HashMap::new();

    let mut builder = CodeBuilder {
        current_symbol: symbol.to_string(),
        function_symbols: &function_symbols,
        functions: &functions,
        package_return_types: &package_return_types,
        platform_imports,
        platform,
        build_mode,
        globals: &globals,
        type_model: type_model.clone(),
        string_symbols,
        locals: HashMap::new(),
        instructions: vec![abi::label("entry")],
        relocations: Vec::new(),
        stack_slots: Vec::new(),
        used_callee_saved: Vec::new(),
        stack_size: 0,
        next_register: 8,
        next_vreg: 0,
        next_fp_vreg: 0,
        float_residents: HashMap::new(),
        promoted_float_locals: HashMap::new(),
        address_taken_locals: HashSet::new(),
        value_used_locals: HashSet::new(),
        borrow_get_locals: HashSet::new(),
        borrow_get_result: false,
        current_returns_param_borrow: false,
        callback_referenced_functions: HashSet::new(),
        // A helper body constructs nothing through the NIR arm.
        synthesized_constructors: HashSet::new(),
        next_label: 0,
        trap: None,
        loop_stack: Vec::new(),
        active_cleanups: Vec::new(),
        cleanup_scope_starts: Vec::new(),
        pending_result_slots: None,
        escaping_value_slot: None,
        raw_result_capture: None,
        trap_discard_error_results: HashSet::new(),
        raw_result_discard_error: false,
        suppress_resource_source_flag: false,
        emitting_error_route: false,
        building_error_block: false,
        current_file: String::new(),
        current_loc: NirSourceLoc::default(),
        resource_owners: HashMap::new(),
        owner_containers: HashSet::new(),
        owned_list_heads: HashMap::new(),
        owned_value_slots: Vec::new(),
        pending_temp_frees: Vec::new(),
        operand_snapshot_wanted: Vec::new(),
        for_each_iterable_locals: Vec::new(),
        for_each_iterable_state_fields: Vec::new(),
        for_each_iterable_record_fields: Vec::new(),
        string_capacity_slots: HashMap::new(),
        math_pool_base_vreg: None,
        vector_natives: HashMap::new(),
        next_vector_native: 0,
        promoted_vector_locals: HashMap::new(),
        promotable_vector_locals: HashSet::new(),
        integer_lower_bounds: HashMap::new(),
        integer_strict_upper: std::collections::HashSet::new(),
        for_bound_expr: HashMap::new(),
        len_of_local: HashMap::new(),
        provable_index_locals: HashMap::new(),
        enclosing_loop_reassigned: Vec::new(),
    };

    // Hand the body its incoming ABI argument registers directly as `ValueResult`s
    // (a runtime helper is finalized as a vreg body, so an argument register is
    // live-in at entry; the body reads it before any call clobbers it, exactly like
    // an OS-seam helper does).
    let mut args = Vec::with_capacity(arity);
    for index in 0..arity {
        args.push(ValueResult {
            origin: None,
            type_: ParameterType::Integer,
            location: abi::argument_register(index)?,
            text: format!("abiArg{index}"),
        });
    }

    let ctx = crate::codegen::registry::AbiCtx {
        platform_imports,
        platform,
        build_mode,
        module_name,
        term_state_offset,
        presentation_mode_offset,
        call,
        arena_global_slots,
        uses_rng,
    };
    let result = lower(&mut builder, &args, &ctx)?;

    // A body that returns a `void`-location `ValueResult` has emitted its OWN
    // fallible ABI — the success value in `RESULT_VALUE_REGISTER` + `RESULT_OK_TAG`,
    // each error path setting its error + jumping to its own `ret`. The wrapper then
    // adds nothing (a keygen needs distinct `ErrUnknown`/`ErrOutOfMemory` exits it
    // can't express through the auto-epilogue). A body that returns a real value
    // gets the convenient epilogue: move it into the result register, tag OK, return.
    if result.location.render() != "void" {
        builder.emit(abi::move_register(RESULT_VALUE_REGISTER, &result.location));
        builder.emit(abi::move_immediate(
            RESULT_TAG_REGISTER,
            "Integer",
            RESULT_OK_TAG,
        ));
        builder.emit(abi::return_());
    }

    // Finalize as a runtime-helper vreg body (like OsLower) — this maps the
    // hand-picked `%v9`..`%v15` vregs the general marshallers use, which full
    // register allocation would reject. `stack_size` is the scratch the body
    // reserved via `allocate_stack_object`.
    let mut instructions = builder.instructions;
    let (frame, stack_slots) =
        finalize_vreg_body_with_locals(&mut instructions, &[], builder.stack_size);
    Ok((frame, instructions, builder.relocations, stack_slots))
}

/// The per-type thread-transfer deep-copy function (bug-391). Takes a pointer to
/// a value of `type_` (a recursive type) in the first argument register and
/// returns a pointer to a fresh, independent copy in the current arena. Its body
/// is `emit_thread_copy_real`, whose recursive sub-edges call *these* functions,
/// so the deep copy recurses at run time over the finite data instead of at
/// compile time over the (infinite) type.
#[allow(clippy::too_many_arguments)]
pub(crate) fn lower_thread_copy_function(
    type_: &ParameterType,
    symbol: &str,
    function_symbols: &HashMap<String, String>,
    functions: &HashMap<String, &NirFunction>,
    package_return_types: &HashMap<String, ParameterType>,
    platform_imports: &HashMap<String, String>,
    platform: &dyn crate::codegen::engine::types::CodegenPlatform,
    build_mode: crate::target::NativeBuildMode,
    globals: &HashMap<String, GlobalValue>,
    string_symbols: &HashMap<String, String>,
    type_model: TypeModel,
) -> Result<CodeFunction, String> {
    let param = CodeParam {
        name: "source".to_string(),
        type_: type_.clone().name().into_owned(),
        location: abi::argument_register(0)?,
    };
    let mut builder = CodeBuilder {
        current_symbol: symbol.to_string(),
        function_symbols,
        functions,
        package_return_types,
        platform_imports,
        platform,
        build_mode,
        globals,
        type_model,
        string_symbols,
        locals: HashMap::new(),
        instructions: vec![abi::label("entry")],
        relocations: Vec::new(),
        stack_slots: Vec::new(),
        used_callee_saved: Vec::new(),
        stack_size: 0,
        next_register: 8,
        next_vreg: 0,
        next_fp_vreg: 0,
        float_residents: HashMap::new(),
        promoted_float_locals: HashMap::new(),
        address_taken_locals: HashSet::new(),
        value_used_locals: HashSet::new(),
        borrow_get_locals: HashSet::new(),
        borrow_get_result: false,
        current_returns_param_borrow: false,
        callback_referenced_functions: HashSet::new(),
        // A helper body constructs nothing through the NIR arm.
        synthesized_constructors: HashSet::new(),
        next_label: 0,
        trap: None,
        loop_stack: Vec::new(),
        active_cleanups: Vec::new(),
        cleanup_scope_starts: Vec::new(),
        pending_result_slots: None,
        escaping_value_slot: None,
        raw_result_capture: None,
        trap_discard_error_results: HashSet::new(),
        raw_result_discard_error: false,
        suppress_resource_source_flag: false,
        emitting_error_route: false,
        building_error_block: false,
        current_file: String::new(),
        current_loc: NirSourceLoc::default(),
        resource_owners: HashMap::new(),
        owner_containers: HashSet::new(),
        owned_list_heads: HashMap::new(),
        owned_value_slots: Vec::new(),
        pending_temp_frees: Vec::new(),
        operand_snapshot_wanted: Vec::new(),
        for_each_iterable_locals: Vec::new(),
        for_each_iterable_state_fields: Vec::new(),
        for_each_iterable_record_fields: Vec::new(),
        string_capacity_slots: HashMap::new(),
        math_pool_base_vreg: None,
        vector_natives: HashMap::new(),
        next_vector_native: 0,
        promoted_vector_locals: HashMap::new(),
        promotable_vector_locals: HashSet::new(),
        integer_lower_bounds: HashMap::new(),
        integer_strict_upper: std::collections::HashSet::new(),
        for_bound_expr: HashMap::new(),
        len_of_local: HashMap::new(),
        provable_index_locals: HashMap::new(),
        enclosing_loop_reassigned: Vec::new(),
    };

    // Capture the incoming source pointer in a vreg (spilled across the copy's
    // internal calls), deep-copy it, and return the fresh pointer.
    let source = builder.allocate_register();
    builder.emit(abi::move_register(&source, &param.location));
    let result = builder.emit_thread_copy_real(type_, &source)?;
    builder.emit(abi::move_register(abi::return_register(), &result));
    builder.emit(abi::return_());

    builder.run_register_allocation()?;
    let mut instructions = builder.instructions;
    let is_x86 = mir::active_backend().register_model().arena_base()
        == crate::arch::x86_64::regmodel::ARENA_BASE_REGISTER;
    peephole::forward_stores_to_loads(&mut instructions, is_x86);
    peephole::remove_fp_shuttles(&mut instructions, mir::active_backend().register_model());
    let mut stack_slots = builder.stack_slots;
    let frame = finalize_frame(
        &mut instructions,
        &mut stack_slots,
        builder.stack_size,
        builder.used_callee_saved,
    );

    Ok(CodeFunction {
        name: format!("thread_copy.{type_}"),
        symbol: symbol.to_string(),
        params: vec![param],
        returns: type_.to_string(),
        frame,
        instructions,
        relocations: builder.relocations,
        stack_slots,
    })
}

#[cfg(test)]
mod m6_escape_tests {
    use super::collect_value_used_locals;
    use crate::target::shared::nir::{NirOp, NirSourceLoc, NirValue};
    use crate::types::ParameterType;
    use std::collections::HashSet;

    fn closure() -> NirValue {
        NirValue::Closure {
            name: "lambda_impl".to_string(),
            type_: ParameterType::parse("FUNC(Integer) AS Integer"),
            captures: vec![],
        }
    }
    fn used(ops: &[NirOp]) -> HashSet<String> {
        let mut out = HashSet::new();
        collect_value_used_locals(ops, &mut out);
        out
    }

    // plan-77 M6: the escape analysis. A closure binding invoked ONLY as a call
    // target is not "value-used" (non-escaping, safe to free); any other reference
    // — argument, return, store, assign, LocalRef — marks it value-used (escaping,
    // NOT freed). A missed escape here would be a use-after-free, so pin it.
    #[test]
    fn invoke_only_closure_is_not_value_used() {
        let ops = vec![
            NirOp::Bind {
                mutable: false,
                name: "f".to_string(),
                type_: ParameterType::parse("FUNC(Integer) AS Integer"),
                value: Some(closure()),
            },
            // `f(x)` lowers to Call { target: "f" } — the target is a String, not a
            // NirValue, so "f" is not visited as a value.
            NirOp::Eval {
                value: NirValue::Call {
                    target: "f".to_string(),
                    args: vec![NirValue::Const {
                        type_: ParameterType::Integer,
                        value: "5".to_string(),
                    }],
                    loc: NirSourceLoc::default(),
                },
            },
        ];
        assert!(!used(&ops).contains("f"));
    }

    #[test]
    fn returned_closure_is_value_used() {
        let ops = vec![NirOp::Return {
            value: Some(NirValue::Local("g".to_string())),
        }];
        assert!(used(&ops).contains("g"));
    }

    #[test]
    fn passed_as_argument_closure_is_value_used() {
        let ops = vec![NirOp::Eval {
            value: NirValue::Call {
                target: "collections.forEach".to_string(),
                args: vec![
                    NirValue::Local("list".to_string()),
                    NirValue::Local("h".to_string()),
                ],
                loc: NirSourceLoc::default(),
            },
        }];
        let u = used(&ops);
        assert!(u.contains("h"));
        assert!(u.contains("list"));
    }

    #[test]
    fn address_taken_and_aliased_closures_are_value_used() {
        let ops = vec![
            // `LET k = f` aliases f — f escapes to k.
            NirOp::Bind {
                mutable: false,
                name: "k".to_string(),
                type_: ParameterType::parse("FUNC(Integer) AS Integer"),
                value: Some(NirValue::Local("f".to_string())),
            },
            // A LocalRef of `m` (address taken) is also an escape.
            NirOp::Eval {
                value: NirValue::LocalRef {
                    name: "m".to_string(),
                    type_: ParameterType::parse("FUNC(Integer) AS Integer"),
                },
            },
        ];
        let u = used(&ops);
        assert!(u.contains("f"));
        assert!(u.contains("m"));
    }
}
