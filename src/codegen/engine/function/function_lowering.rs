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
    param_borrow_returns(f, callback_referenced).is_some()
}

/// The parameters a [`function_returns_param_borrow`] function returns — `Some`
/// exactly when that predicate holds, and then never empty.
///
/// bug-1019: these are the parameters whose block the CALLER must keep owning.
/// The result of such a call is a borrow of one of them, so the hand-over
/// analysis (`analysis::handover`) never gives one away: a block handed to the
/// callee's owned variant comes straight back as the result, which the caller
/// still reads as a borrow of an argument it no longer owns, and nothing frees it.
pub(crate) fn param_borrow_returns(
    f: &NirFunction,
    callback_referenced: &HashSet<String>,
) -> Option<HashSet<String>> {
    use nir::visit::{walk_op, NirVisitor};
    // A function used as a `FunctionRef` (callback) is invoked through an ABI that
    // OWNS and frees its return value, so it must keep returning a fresh copy — never
    // a parameter borrow (see `collect_function_ref_names`).
    if callback_referenced.contains(&f.name) {
        return None;
    }
    let params: HashSet<String> = f.params.iter().map(|p| p.name.clone()).collect();
    if params.is_empty() {
        return None;
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
        return None;
    }
    // A returned parameter that is reassigned or address-taken is no longer
    // guaranteed to be the caller's untouched argument block, so the "borrow" could
    // alias a modified/freed local — bail (keep the copy).
    let reassigned = collect_reassigned_locals(&f.body);
    let mut address_taken = HashSet::new();
    collect_address_taken_locals(&f.body, &mut address_taken);
    if r.returned
        .iter()
        .any(|p| reassigned.contains(p) || address_taken.contains(p))
    {
        return None;
    }
    Some(r.returned)
}

/// bug-536 shape B-2: whether a caller may free the bare `String` this function
/// returns — i.e. whether `f` **guarantees** every value it returns is a fresh,
/// solely-owned arena block.
///
/// This is the callee half of the same contract `function_returns_param_borrow`
/// expresses for the opposite answer, and the two are deliberately disjoint: a
/// param-borrow function returns the caller's own argument block, so freeing its
/// result would free a live `String` the caller still owns. The order matters —
/// `function_returns_param_borrow` is checked FIRST and wins.
///
/// The guarantee is **delivered**, not merely observed. `lower_returned_value`
/// copies any returned `String` whose freshness lowering could not establish (no
/// move-elision, no owning copy, no claimed pending temp), so every return site of
/// a function this predicate admits hands the caller a block with exactly one
/// owner. That is why the predicate needs no fixpoint over calls: a
/// `RETURN <call to another such function>` is fresh *by this same guarantee*,
/// and any mutual recursion bottoms out at a return that is either provably fresh
/// or copied.
///
/// Gated in the safe direction. Excluded, each of which keeps bug-536 shape B's
/// leak rather than risking a wild free:
///
/// * a non-`String` return type — the bare-`String` exemption in
///   `register_pending_temp` is the only thing this lifts;
/// * a param-borrow function (above);
/// * a function with no value return at all, so there is no returned block to
///   own.
///
/// **bug-562: a callback-referenced function is NOT excluded, and must not be.**
/// It was, when B-2 landed, purely to keep callback lowering byte-identical for
/// one change — and that exclusion was the live half of a SIGSEGV.
///
/// The `FunctionRef` ABI hands a callback a per-iteration block it owns and frees
/// (`free_collection_loop_item`), on the stated assumption that "a callback that
/// *returns* something derived from it returns a separate allocation". Nothing
/// enforced that assumption for a return that is not a bare parameter.
/// `function_returns_param_borrow` excludes this same set to FORCE a copy, which
/// covers `RETURN <param>`; but `RETURN toString(s)` returns a `Call`, so it is
/// not a param-borrow function and that exclusion never applied — while
/// `toString`'s `String` arm is the identity, so it handed the HOF back the very
/// block the HOF was about to free. `collections::transform(xs, identish)` with
/// `FUNC identish(s AS String) AS String / RETURN toString(s)` was `[exit 139]`.
///
/// Admitting the set here is what closes it: the predicate is the callee's
/// obligation, so a `String`-returning callback now copies on exactly the return
/// sites whose freshness lowering could not prove — the same delivery
/// `function_returns_param_borrow`'s exclusion buys for the borrow shape, applied
/// to the shape that escaped it.
///
/// This can only ever ADD an owner-establishing copy; it never removes a free.
/// `lower_returned_value`'s arms are mutually exclusive early returns, so a return
/// site already made fresh (a move-elided owned local, a copied literal or
/// parameter, a claimed pending temp) is untouched — a callback that already
/// worked is byte-identical, and no value is copied twice.
pub(crate) fn function_returns_fresh_string(
    f: &NirFunction,
    callback_referenced: &HashSet<String>,
) -> bool {
    use nir::visit::{walk_op, NirVisitor};
    if f.returns != ParameterType::String {
        return false;
    }
    // A borrow and a fresh block are mutually exclusive answers to the same
    // question; the borrow contract is the older one and wins.
    if function_returns_param_borrow(f, callback_referenced) {
        return false;
    }
    struct AnyValueReturn {
        any: bool,
    }
    impl NirVisitor for AnyValueReturn {
        fn visit_op(&mut self, op: &NirOp) {
            if matches!(op, NirOp::Return { value: Some(_) }) {
                self.any = true;
            }
            walk_op(self, op);
        }
    }
    let mut r = AnyValueReturn { any: false };
    r.visit_ops(&f.body);
    r.any
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

/// bug-778: the `Result`-locals that may live in frame slots instead of a heap
/// `{tag, size, payload}` block ([`UnboxedTrapResult`]).
///
/// The inline-`TRAP` desugar (`ir::lower::lower_inline_trap`, both the one-check
/// and the check-chain shape) binds each fallible node to a temporary
/// `Bind $trap_resN = CallResult(..) | Checked(..)` and reads it only through
/// `ResultIsOk` / `ResultValue` / `ResultError`. That is the property the frame
/// form needs, and it is VERIFIED here on the final NIR rather than assumed from
/// the name, so an optimizer rewrite or a decoded package that breaks it falls
/// back to the heap form. A local qualifies when:
///
/// * it is bound exactly once, by a `Bind` whose value is a `CallResult` or a
///   `Checked`, and never assigned, `STATE`-assigned or address-taken;
/// * every read of it is the direct operand of one of the three readers;
/// * `ResultValue` and `ResultError` each occur at most once, and each as the
///   whole value of a `Bind` or an `Assign` — an owning store, which is what
///   lets the reader MOVE an owned block out instead of copying it.
fn unboxed_trap_result_candidates(ops: &[NirOp]) -> HashMap<String, UnboxedTrapUses> {
    use nir::visit::{walk_op, walk_value, NirVisitor};
    #[derive(Default)]
    struct Collector {
        bound: HashMap<String, usize>,
        candidates: HashSet<String>,
        disqualified: HashSet<String>,
        // (`ResultValue`, `ResultError`) reads per local in store position ...
        stored_reads: HashMap<String, (usize, usize)>,
        // ... and anywhere at all. A qualifying local has the two equal.
        all_reads: HashMap<String, (usize, usize)>,
    }
    impl NirVisitor for Collector {
        fn visit_op(&mut self, op: &NirOp) {
            match op {
                NirOp::Bind {
                    name,
                    value: Some(value),
                    ..
                } => {
                    *self.bound.entry(name.clone()).or_default() += 1;
                    if matches!(
                        value,
                        NirValue::CallResult { .. } | NirValue::Checked { .. }
                    ) {
                        self.candidates.insert(name.clone());
                    }
                }
                NirOp::Bind { name, .. } => {
                    *self.bound.entry(name.clone()).or_default() += 1;
                }
                NirOp::Assign { name, .. } => {
                    self.disqualified.insert(name.clone());
                }
                NirOp::StateAssign { resource, .. } => {
                    self.disqualified.insert(resource.clone());
                }
                _ => {}
            }
            // A value reader in store position: the one place it may move.
            let stored = match op {
                NirOp::Bind {
                    value: Some(value), ..
                } => Some(value),
                NirOp::Assign { value, .. } => Some(value),
                _ => None,
            };
            if let Some(
                NirValue::ResultValue { value: inner } | NirValue::ResultError { value: inner },
            ) = stored
            {
                if let NirValue::Local(name) = inner.as_ref() {
                    let counts = self.stored_reads.entry(name.clone()).or_default();
                    if matches!(stored, Some(NirValue::ResultValue { .. })) {
                        counts.0 += 1;
                    } else {
                        counts.1 += 1;
                    }
                }
            }
            walk_op(self, op);
        }
        fn visit_value(&mut self, value: &NirValue) {
            match value {
                NirValue::ResultIsOk { value: inner }
                | NirValue::ResultValue { value: inner }
                | NirValue::ResultError { value: inner } => {
                    if let NirValue::Local(name) = inner.as_ref() {
                        // Every value read; `visit_op` counts the store-position
                        // ones, so a read anywhere else makes the two differ.
                        if !matches!(value, NirValue::ResultIsOk { .. }) {
                            let counts = self.all_reads.entry(name.clone()).or_default();
                            if matches!(value, NirValue::ResultValue { .. }) {
                                counts.0 += 1;
                            } else {
                                counts.1 += 1;
                            }
                        }
                        return;
                    }
                    walk_value(self, value);
                }
                NirValue::Local(name) | NirValue::LocalRef { name, .. } => {
                    self.disqualified.insert(name.clone());
                }
                _ => walk_value(self, value),
            }
        }
    }
    let mut collector = Collector::default();
    collector.visit_ops(ops);
    let Collector {
        bound,
        candidates,
        disqualified,
        stored_reads,
        all_reads,
    } = collector;
    candidates
        .into_iter()
        .filter_map(|name| {
            let stored = stored_reads.get(&name).copied().unwrap_or_default();
            let all = all_reads.get(&name).copied().unwrap_or_default();
            let qualifies = bound.get(&name) == Some(&1)
                && !disqualified.contains(&name)
                && stored == all
                && stored.0 <= 1
                && stored.1 <= 1;
            qualifies.then_some((
                name,
                UnboxedTrapUses {
                    reads_value: stored.0 == 1,
                    reads_error: stored.1 == 1,
                },
            ))
        })
        .collect()
}

/// bug-778: the inline-`TRAP` delivery slots whose one read can MOVE.
///
/// `lower_inline_trap` stages the trapped value in `$trap_valN` (and a check
/// chain's error in `$trap_errN`) and delivers it with `Bind y = $trap_valN` (or
/// `Assign`). When that store is the slot's ONLY read in the function — checked
/// here, not assumed, so an optimizer rewrite that reads it again keeps the copy
/// — the read is its last use, and copying the block only to drop the original at
/// scope end is a wasted allocation on every evaluation. Address-taken slots are
/// excluded (`LocalRef`), and so is a slot read anywhere but a store's whole
/// value.
fn trap_delivery_moves(ops: &[NirOp]) -> HashSet<String> {
    use nir::visit::{walk_op, walk_value, NirVisitor};
    #[derive(Default)]
    struct Collector {
        stored: HashMap<String, usize>,
        reads: HashMap<String, usize>,
        address_taken: HashSet<String>,
    }
    impl NirVisitor for Collector {
        fn visit_op(&mut self, op: &NirOp) {
            let stored = match op {
                NirOp::Bind {
                    value: Some(value), ..
                } => Some(value),
                NirOp::Assign { value, .. } => Some(value),
                _ => None,
            };
            if let Some(NirValue::Local(name)) = stored {
                *self.stored.entry(name.clone()).or_default() += 1;
            }
            walk_op(self, op);
        }
        fn visit_value(&mut self, value: &NirValue) {
            match value {
                NirValue::Local(name) => *self.reads.entry(name.clone()).or_default() += 1,
                NirValue::LocalRef { name, .. } => {
                    self.address_taken.insert(name.clone());
                }
                _ => {}
            }
            walk_value(self, value);
        }
    }
    let mut collector = Collector::default();
    collector.visit_ops(ops);
    collector
        .stored
        .iter()
        .filter(|(name, stored)| {
            crate::codegen::engine::control::is_inline_trap_delivery_slot(name)
                && **stored == 1
                && collector.reads.get(*name) == Some(&1)
                && !collector.address_taken.contains(*name)
        })
        .map(|(name, _)| name.clone())
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

#[allow(clippy::too_many_arguments)]
/// plan-147-D: which parameters a lowering OWNS, and the symbol it emits under.
///
/// A function lowers once as its base symbol with every parameter lent (the caller
/// keeps the block, `None` here), and then once more per distinct owned-parameter
/// mask that an approved call site asks for. A variant is a second lowering of the
/// **same `NirFunction`**, so every span, `Error.source` stamp and line-table entry
/// is identical to the base's — which is what keeps plan-147-A §2.3 row S11 true.
///
/// Inside a variant an owned parameter is an ordinary owned local: it gets an
/// `ActiveCleanup::OwnedValue`, so it is freed on every exit unless moved out, and
/// letter B's S11 can update it in place and hand the block on.
#[derive(Clone, Debug)]
pub(crate) struct OwnedVariant {
    /// Bit `i` set = parameter `i` is owned by this lowering.
    pub(crate) mask: u64,
    /// bug-784: `Some(j)` for a destination variant (`<base>$into<j>`, mask 0):
    /// parameter `j` is the caller's destination, and `RETURN WITH <param j> { … }`
    /// writes its result there when the size is unchanged
    /// (`analysis::into_dest`).
    pub(crate) into: Option<usize>,
    /// The symbol this lowering emits under.
    pub(crate) symbol: String,
}

impl OwnedVariant {
    /// The symbol a variant of `base` with `mask` emits under.
    ///
    /// `$` is the only separator monomorphization uses (`mangle_name`), and a
    /// mangled segment is `[A-Za-z0-9_$]*` (`sanitize_type_name` maps everything
    /// else to `$`). A lowercase segment IS reachable — a package prefix is
    /// lowercase, so `own1f` is not structurally impossible as a type segment — so
    /// this name is NOT assumed unique. `variant_symbol_is_fresh` asserts it against
    /// the module's real symbol set instead, which fails loudly rather than
    /// silently aliasing two functions.
    pub(crate) fn symbol_for(base: &str, mask: u64) -> String {
        format!("{base}$own{mask:x}")
    }

    /// bug-784: the symbol a destination variant of `base` for parameter `param`
    /// emits under. Checked against the module's symbols like
    /// [`Self::symbol_for`]'s.
    pub(crate) fn into_symbol_for(base: &str, param: usize) -> String {
        format!("{base}$into{param}")
    }

    /// Whether parameter `index` is owned by this lowering.
    pub(crate) fn owns(&self, index: usize) -> bool {
        index < 64 && self.mask & (1u64 << index) != 0
    }
}

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
    module_name: &str,
    // plan-147-D: `None` is the base lowering (every parameter lent).
    variant: Option<&OwnedVariant>,
    // bug-781: receives the environment layout of every capturing `LAMBDA` the
    // function creates, for the module to emit each one's environment routine.
    closure_env_layouts: &mut Vec<ClosureEnvLayout>,
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
                abi::argument_register(index)?.into()
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

    let base_symbol = nir::function_symbol(&function.name);
    let emitted_symbol = match variant {
        Some(variant) => variant.symbol.clone(),
        None => base_symbol.clone(),
    };
    let mut builder = CodeBuilder {
        current_symbol: emitted_symbol.clone(),
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
        float_residents: HashMap::new(),
        promoted_float_locals: HashMap::new(),
        address_taken_locals: HashSet::new(),
        borrow_get_locals: HashSet::new(),
        element_updates: ElementUpdates::default(),
        record_temps: Default::default(),
        borrow_get_result: false,
        borrow_get_armed: false,
        append_headroom_binds: Default::default(),
        headroom_copy_result: false,
        headroom_copy_armed: false,
        map_settle_exempt_armed: false,
        record_member_read: false,
        value_discarded_armed: false,
        resource_temp_exempt: None,
        trap_call_results: HashSet::new(),
        resource_temps_closed_on_path: false,
        exclusive_temps: ExclusiveTemps::default(),
        current_returns_param_borrow: false,
        record_owning_locals: HashSet::new(),
        resource_alias_sources: HashMap::new(),
        owned_list_owning_collections: HashSet::new(),
        store_floats: Default::default(),
        reassign_free_collections: HashSet::new(),
        inner_owning_collections: HashSet::new(),
        inner_block_slots: HashSet::new(),
        trap_ownership: Default::default(),
        trap_owner_flags: HashMap::new(),
        live_resource_aliases: HashMap::new(),
        live_union_wraps: HashMap::new(),
        current_returns_fresh_string: false,
        callback_referenced_functions: HashSet::new(),
        // A helper body constructs nothing through the NIR arm.
        synthesized_constructors: synthesized_constructors.clone(),
        next_label: 0,
        trap: None,
        loop_stack: Vec::new(),
        active_cleanups: Vec::new(),
        cleanup_scope_starts: Vec::new(),
        scope_token_slot: None,
        pending_result_slots: None,
        escaping_value_slot: None,
        error_return_pads: HashMap::new(),
        error_return_pad_code: Vec::new(),
        raw_result_capture: None,
        trap_discard_error_results: HashSet::new(),
        raw_result_discard_error: false,
        unboxed_trap_candidates: HashMap::new(),
        trap_delivery_moves: HashSet::new(),
        unboxed_trap_locals: HashMap::new(),
        unboxed_trap_pending: None,
        unboxed_trap_active: None,
        unboxed_trap_filled: false,
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
        closure_env_layouts: Vec::new(),
        fresh_string_block: None,
        operand_snapshot_wanted: Vec::new(),
        for_each_iterable_locals: Vec::new(),
        for_each_iterable_state_fields: Vec::new(),
        for_each_iterable_record_fields: Vec::new(),
        string_capacity_slots: HashMap::new(),
        self_update_scratch: None,
        self_update_scratch_env: None,
        field_pre_emit: None,
        field_pre_emitted: None,
        simd_result_into: None,
        math_pool_base_vreg: None,
        vector_natives: HashMap::new(),
        next_vector_native: 0,
        promoted_vector_locals: HashMap::new(),
        promotable_vector_locals: HashSet::new(),
        resource_containment: HashMap::new(),
        integer_lower_bounds: HashMap::new(),
        integer_strict_upper: std::collections::HashSet::new(),
        for_bound_expr: HashMap::new(),
        len_of_local: HashMap::new(),
        provable_index_locals: HashMap::new(),
        cold_instructions: Vec::new(),
        cold_blocks_enabled: true,
        enclosing_loop_reassigned: Vec::new(),
        graph_copy_walker: None,
        move_sites: None,
        handover: None,
        current_call_key: None,
        args_updated_in_place: Vec::new(),
        current_op_key: None,
        string_app_resource_base: None,
        string_shadow_env: std::collections::HashMap::new(),
        string_app_resource_base_env: None,
        module_name: module_name.to_string(),
    };
    // bug-789: a function that can recurse checks the stack at entry, before its
    // parameters are spilled: the check's two temporaries then never share a
    // register with a parameter it would have to reload. The raise is a cold block
    // after the body.
    let stack_overflow =
        if crate::codegen::runtime::stack_limit::function_needs_stack_check(function) {
            let label = builder.label("stack_overflow");
            let start = builder.instructions.len();
            builder.emit_stack_check(&label);
            Some((label, start..builder.instructions.len()))
        } else {
            None
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
        // plan-147-D: an owned parameter is an ordinary owned local. Registering the
        // cleanup here — after the spill, so the slot genuinely holds the block —
        // frees it on every exit (normal, error, trap route) unless it is moved out,
        // and takes it out of `excluded_roots`'s "never bound" set so letter B's S11
        // and `plan_returned_move` treat it as owned.
        if variant.is_some_and(|variant| variant.owns(index)) {
            builder
                .active_cleanups
                .push(ActiveCleanup::OwnedValue(OwnedValueCleanup {
                    type_: function.params[index].type_.clone(),
                    stack_offset,
                    capacity_slot: None,
                    loop_alias_slot: None,
                    result_wrapper: None,
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
    // Locals whose address is taken anywhere — never loop-promoted (plan-03 D2),
    // and never given a string capacity shadow (below).
    collect_address_taken_locals(&function.body, &mut builder.address_taken_locals);
    // Pre-allocate the capacity shadow slot for every in-place string self-append
    // target so bind/assign sites can reset it and the prologue can zero it.
    // plan-146-G: which by-ref captures share their owner's capacity shadow —
    // before the self-append prescan, which skips those (they need no slot here).
    builder.prescan_string_shadow_env(&function.name);
    builder.prescan_string_app_resource_base_env(&function.name);
    builder.prescan_string_self_appends(&function.body);
    // plan-142-B: the scratch block the in-place shrink arms keep marks in.
    builder.prescan_self_update_scratch(&function.name, &function.body);
    builder.prescan_string_app_resource_base(&function.body);
    // plan-134-D: the owning stores that read their source for the last time
    // (plan-134-C), so a recursive value's store can move instead of copying.
    builder.move_sites = Some(
        crate::codegen::engine::analysis::last_use::collect_last_use_moves(
            function,
            &builder.type_model,
        ),
    );
    // plan-147-D: which call arguments this function may hand to the callee instead
    // of lending (plan-147-C). Computed once here, like `move_sites` above, so the
    // answer cannot depend on the order the builder reaches the ops in.
    let owned_param_names: std::collections::HashSet<String> = function
        .params
        .iter()
        .enumerate()
        .filter(|(index, _)| variant.is_some_and(|variant| variant.owns(*index)))
        .map(|(_, param)| param.name.clone())
        .collect();
    builder.handover = Some(
        crate::codegen::engine::analysis::handover::collect_handover_args(
            function,
            &builder.type_model,
            functions,
            &owned_param_names,
            callback_referenced_functions,
        ),
    );
    // Small-vector locals that can live in registers for their whole lifetime with
    // no arena block (plan-01-vector).
    builder.promotable_vector_locals =
        promotable_vector_locals(&function.body, &builder.address_taken_locals);
    // plan-86 E, bug-689: `get` bindings that hold an alias into their container
    // (needs address_taken above, and — for the element-bound ones — the promoted
    // vectors, which never hold a block to alias).
    let borrow_gets = {
        let global_written = |ops: &[NirOp], global: &str| {
            builder.ops_reach_store(
                ops,
                crate::codegen::engine::value::store_reach::StoreLeaf::Global(global),
            )
        };
        crate::codegen::engine::analysis::borrow_get::collect_borrow_gets(
            &function.body,
            &builder.address_taken_locals,
            &global_written,
        )
    };
    builder.install_borrow_gets(borrow_gets);
    // bug-784: the destination-passing call sites (after `install_borrow_gets`,
    // which resets the element-update state), and — in a destination variant —
    // the destination parameter.
    builder.element_updates.into_sites =
        crate::codegen::engine::analysis::into_dest::collect_into_sites(
            function,
            &builder.type_model,
            functions,
        );
    builder.element_updates.into_param = variant
        .and_then(|variant| variant.into)
        .and_then(|index| function.params.get(index))
        .map(|param| param.name.clone());
    // bug-770 C: `MUT x = get/getOr(…)` binds whose next statement appends to `x`
    // in place get their owned copy with growth headroom.
    builder.append_headroom_binds =
        crate::codegen::engine::analysis::append_headroom::collect_append_headroom_binds(
            &function.body,
        );
    // bug-766 B: the fixed-size record temporaries kept in the record-temp scratch.
    builder.install_record_temps(&function.body);
    // plan-116-J: what each local holds, for the consuming-parameter deactivation. A
    // whole-function pre-pass rather than an incremental map built during lowering, so
    // it does not depend on the order the builder happens to reach the ops in.
    builder.resource_containment = crate::codegen::resource_containment(&function.body);
    // bug-623 B: which resource bindings own their record, and the alias chains a
    // returned union follows to its owner. A whole-function pre-pass, like the above.
    let ownership = {
        let record_type = |type_: &ParameterType| builder.record_freeable_type(type_);
        // bug-805: a collection whose elements reach a resource at any depth (a
        // `List OF Holder`, a `List OF List OF Holder`) owns its adopted resources
        // exactly as a `List OF RES` does.
        // bug-1025: and so does a record that reaches one (a record with a `RES`
        // field, or one nesting such a record), which adopts the resources a call
        // hands back inside it.
        let res_collection_type = |type_: &ParameterType| {
            CodeBuilder::is_res_collection_block(type_)
                || builder.is_resource_owning_container(type_)
        };
        crate::codegen::resource::cleanup::record_ownership::record_ownership(
            function,
            functions,
            &record_type,
            &res_collection_type,
        )
    };
    builder.record_owning_locals = ownership.owning_locals;
    builder.resource_alias_sources = ownership.alias_sources;
    builder.owned_list_owning_collections = ownership.owning_collections;
    builder.reassign_free_collections = ownership.reassign_free_collections;
    builder.inner_owning_collections = ownership.inner_owning_collections;
    builder.exclusive_temps.calls = ownership.exclusive_temps;
    // bug-1045: a container that a store raises an inner scope's value into lists
    // what the store hands it, so it needs its owned-lists from its bind, like a
    // float target.
    builder
        .owner_containers
        .extend(ownership.store_floats.containers.iter().cloned());
    builder.store_floats = ownership.store_floats;
    // bug-805: a container local later ASSIGNED a call result adopts what the call
    // hands out, so it needs its owned-lists from its bind, like a float target.
    let call_assigned = builder.call_assigned_owner_containers(&function.body);
    builder.owner_containers.extend(call_assigned);
    // bug-842: and the inline-`TRAP` deliveries of such a call's success value.
    builder.trap_call_results =
        crate::codegen::cleanup::owned::builder_owned_cleanup::trap_call_result_locals(
            &function.body,
        );
    // bug-648: which inline-`TRAP` temps are handed borrowed values.
    builder.trap_ownership =
        crate::codegen::resource::cleanup::trap_ownership::collect_trap_ownership(&function.body);
    // plan-64-I: inline-conversion `CallResult` Result-locals whose trapped error
    // is provably unused, so the error path builds only a tag (no ErrorLoc/Error).
    builder.trap_discard_error_results = trap_discard_error_results(&function.body);
    // bug-778: the inline-`TRAP` temporaries that need no heap `Result` block.
    builder.unboxed_trap_candidates = unboxed_trap_result_candidates(&function.body);
    builder.trap_delivery_moves = trap_delivery_moves(&function.body);
    // plan-86 K1: a pure parameter-passthrough function returns its argument uncopied;
    // the caller copies at its ownership boundary. Only the user-function path sets
    // this — builtin/thread-copy wrappers default false (never param-borrow, and no
    // caller looks them up in `functions`), keeping callee-skip and caller-classify
    // consistent.
    builder.callback_referenced_functions = callback_referenced_functions.clone();
    builder.current_returns_param_borrow =
        function_returns_param_borrow(function, callback_referenced_functions);
    // bug-536 shape B-2: the callee half of "a caller may free this `String`". Set
    // from the SAME predicate every call site consults, so the obligation this
    // function takes on (return a solely-owned block on every path) and the licence
    // the caller takes (free the result at statement end) can never disagree.
    builder.current_returns_fresh_string =
        function_returns_fresh_string(function, callback_referenced_functions);
    {
        // `-vv` span (`crate::trace`): emission is one of the four big
        // per-function costs, and separating it from allocation is what says
        // whether a slow function is slow to *lower* or slow to *color*.
        let _span = crate::trace::span("emit ops");
        // plan-156-E: a parameter that declares a thread view's STATE default-builds it
        // when the view has none yet — the worker entry's own view arrives without one
        // (the runtime builds the view, not its STATE); a view that already carries
        // STATE is left alone. The same lazy init a `RES … STATE` binding runs.
        for param in &function.params {
            if let ParameterType::Stateful { base, state } = &param.type_ {
                if matches!(**base, ParameterType::ThreadHandle { .. }) {
                    if let Some(slot) = builder.locals.get(&param.name).map(|l| l.stack_offset) {
                        builder.emit_resource_state_init(slot, state, &param.type_)?;
                    }
                }
            }
        }
        builder.lower_ops(&function.body)?;
        if !builder.current_block_returns() {
            builder.emit_return_exit(None, None)?;
        }
        if let Some((label, check)) = stack_overflow {
            // A body that lowered to no call at all (its NIR calls were inlined
            // builtins) cannot recurse: drop the check, which would otherwise cost
            // a leaf its leaf frame through the cold block's call.
            let calls = builder.instructions[check.end..].iter().any(|instruction| {
                matches!(
                    instruction.op,
                    crate::arch::ops::CodeOp::BranchLink
                        | crate::arch::ops::CodeOp::BranchLinkRegister
                )
            });
            if calls {
                builder.emit_stack_overflow_block(&label)?;
            } else {
                builder.instructions.drain(check);
            }
        }
    }
    closure_env_layouts.append(&mut builder.closure_env_layouts);
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
        name: match variant {
            Some(OwnedVariant {
                into: Some(param), ..
            }) => format!("{}$into{param}", function.name),
            Some(variant) => format!("{}$own{:x}", function.name, variant.mask),
            None => function.name.clone(),
        },
        symbol: emitted_symbol,
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
        location: abi::argument_register(0)?.into(),
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
    mouse_state_offset: Option<usize>,
    input_state_offset: Option<usize>,
    arena_global_slots: usize,
    uses_rng: bool,
    debug_arena_registry: bool,
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
        float_residents: HashMap::new(),
        promoted_float_locals: HashMap::new(),
        address_taken_locals: HashSet::new(),
        borrow_get_locals: HashSet::new(),
        element_updates: ElementUpdates::default(),
        record_temps: Default::default(),
        borrow_get_result: false,
        borrow_get_armed: false,
        append_headroom_binds: Default::default(),
        headroom_copy_result: false,
        headroom_copy_armed: false,
        map_settle_exempt_armed: false,
        record_member_read: false,
        value_discarded_armed: false,
        resource_temp_exempt: None,
        trap_call_results: HashSet::new(),
        resource_temps_closed_on_path: false,
        exclusive_temps: ExclusiveTemps::default(),
        current_returns_param_borrow: false,
        record_owning_locals: HashSet::new(),
        resource_alias_sources: HashMap::new(),
        owned_list_owning_collections: HashSet::new(),
        store_floats: Default::default(),
        reassign_free_collections: HashSet::new(),
        inner_owning_collections: HashSet::new(),
        inner_block_slots: HashSet::new(),
        trap_ownership: Default::default(),
        trap_owner_flags: HashMap::new(),
        live_resource_aliases: HashMap::new(),
        live_union_wraps: HashMap::new(),
        current_returns_fresh_string: false,
        callback_referenced_functions: HashSet::new(),
        // A helper body constructs nothing through the NIR arm.
        synthesized_constructors: HashSet::new(),
        next_label: 0,
        trap: None,
        loop_stack: Vec::new(),
        active_cleanups: Vec::new(),
        cleanup_scope_starts: Vec::new(),
        scope_token_slot: None,
        pending_result_slots: None,
        escaping_value_slot: None,
        error_return_pads: HashMap::new(),
        error_return_pad_code: Vec::new(),
        raw_result_capture: None,
        trap_discard_error_results: HashSet::new(),
        raw_result_discard_error: false,
        unboxed_trap_candidates: HashMap::new(),
        trap_delivery_moves: HashSet::new(),
        unboxed_trap_locals: HashMap::new(),
        unboxed_trap_pending: None,
        unboxed_trap_active: None,
        unboxed_trap_filled: false,
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
        closure_env_layouts: Vec::new(),
        fresh_string_block: None,
        operand_snapshot_wanted: Vec::new(),
        for_each_iterable_locals: Vec::new(),
        for_each_iterable_state_fields: Vec::new(),
        for_each_iterable_record_fields: Vec::new(),
        string_capacity_slots: HashMap::new(),
        self_update_scratch: None,
        self_update_scratch_env: None,
        field_pre_emit: None,
        field_pre_emitted: None,
        simd_result_into: None,
        math_pool_base_vreg: None,
        vector_natives: HashMap::new(),
        next_vector_native: 0,
        promoted_vector_locals: HashMap::new(),
        promotable_vector_locals: HashSet::new(),
        resource_containment: HashMap::new(),
        integer_lower_bounds: HashMap::new(),
        integer_strict_upper: std::collections::HashSet::new(),
        for_bound_expr: HashMap::new(),
        len_of_local: HashMap::new(),
        provable_index_locals: HashMap::new(),
        cold_instructions: Vec::new(),
        cold_blocks_enabled: false,
        enclosing_loop_reassigned: Vec::new(),
        graph_copy_walker: None,
        move_sites: None,
        handover: None,
        current_call_key: None,
        args_updated_in_place: Vec::new(),
        current_op_key: None,
        string_app_resource_base: None,
        string_shadow_env: std::collections::HashMap::new(),
        string_app_resource_base_env: None,
        module_name: String::new(),
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
            location: abi::argument_register(index)?.into(),
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
        mouse_state_offset,
        input_state_offset,
        call,
        arena_global_slots,
        uses_rng,
        debug_arena_registry,
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
/// returns a pointer to a fresh, independent copy in the current arena.
///
/// plan-134-B: the copy itself is the module's one non-recursive walker,
/// `_mfb_rt_graph_copy` (`memory/arena/graph_copy.rs`); this entry point passes
/// `type_`'s `kind` index and returns what the walker returns. Its body used to be
/// `emit_thread_copy_real`, whose recursive sub-edges called these functions — a
/// native call per level of the data, which overflowed the stack on a chain about
/// 60 000 deep.
#[allow(clippy::too_many_arguments)]
pub(crate) fn lower_thread_copy_function(
    type_: &ParameterType,
    kind: usize,
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
        location: abi::argument_register(0)?.into(),
    };
    let mut builder = helper_code_builder(
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

    // Hand the source to the walker with this type's kind, and return its copy.
    let source = builder.allocate_register();
    builder.emit(abi::move_register(&source, &param.location));
    builder.emit(abi::move_register(abi::c_arg(1), &source));
    builder.emit(abi::move_immediate(
        abi::c_arg(0),
        "Integer",
        &kind.to_string(),
    ));
    builder.emit_symbol_call(crate::codegen::memory::arena::graph_copy::GRAPH_COPY_SYMBOL);
    let result = builder.allocate_register();
    builder.emit(abi::move_register(&result, abi::return_register()));
    builder.emit(abi::move_register(abi::return_register(), &result));
    builder.emit(abi::return_());
    finish_helper_function(
        builder,
        &format!("thread_copy.{type_}"),
        vec![param],
        &type_.to_string(),
    )
}

/// A [`CodeBuilder`] for a runtime helper the codegen synthesizes rather than
/// lowers from NIR (bug-391's per-type copy entry, bug-806's module teardown):
/// no locals, no cleanups, no trap — a body the caller emits instruction by
/// instruction, with every builder service (register allocation, stack objects,
/// calls, raises) available.
#[allow(clippy::too_many_arguments)]
pub(crate) fn helper_code_builder<'a>(
    symbol: &str,
    function_symbols: &'a HashMap<String, String>,
    functions: &'a HashMap<String, &'a NirFunction>,
    package_return_types: &'a HashMap<String, ParameterType>,
    platform_imports: &'a HashMap<String, String>,
    platform: &'a dyn crate::codegen::engine::types::CodegenPlatform,
    build_mode: crate::target::NativeBuildMode,
    globals: &'a HashMap<String, GlobalValue>,
    string_symbols: &'a HashMap<String, String>,
    type_model: TypeModel,
) -> CodeBuilder<'a> {
    CodeBuilder {
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
        float_residents: HashMap::new(),
        promoted_float_locals: HashMap::new(),
        address_taken_locals: HashSet::new(),
        borrow_get_locals: HashSet::new(),
        element_updates: ElementUpdates::default(),
        record_temps: Default::default(),
        borrow_get_result: false,
        borrow_get_armed: false,
        append_headroom_binds: Default::default(),
        headroom_copy_result: false,
        headroom_copy_armed: false,
        map_settle_exempt_armed: false,
        record_member_read: false,
        value_discarded_armed: false,
        resource_temp_exempt: None,
        trap_call_results: HashSet::new(),
        resource_temps_closed_on_path: false,
        exclusive_temps: ExclusiveTemps::default(),
        current_returns_param_borrow: false,
        record_owning_locals: HashSet::new(),
        resource_alias_sources: HashMap::new(),
        owned_list_owning_collections: HashSet::new(),
        store_floats: Default::default(),
        reassign_free_collections: HashSet::new(),
        inner_owning_collections: HashSet::new(),
        inner_block_slots: HashSet::new(),
        trap_ownership: Default::default(),
        trap_owner_flags: HashMap::new(),
        live_resource_aliases: HashMap::new(),
        live_union_wraps: HashMap::new(),
        current_returns_fresh_string: false,
        callback_referenced_functions: HashSet::new(),
        // A helper body constructs nothing through the NIR arm.
        synthesized_constructors: HashSet::new(),
        next_label: 0,
        trap: None,
        loop_stack: Vec::new(),
        active_cleanups: Vec::new(),
        cleanup_scope_starts: Vec::new(),
        scope_token_slot: None,
        pending_result_slots: None,
        escaping_value_slot: None,
        error_return_pads: HashMap::new(),
        error_return_pad_code: Vec::new(),
        raw_result_capture: None,
        trap_discard_error_results: HashSet::new(),
        raw_result_discard_error: false,
        unboxed_trap_candidates: HashMap::new(),
        trap_delivery_moves: HashSet::new(),
        unboxed_trap_locals: HashMap::new(),
        unboxed_trap_pending: None,
        unboxed_trap_active: None,
        unboxed_trap_filled: false,
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
        closure_env_layouts: Vec::new(),
        fresh_string_block: None,
        operand_snapshot_wanted: Vec::new(),
        for_each_iterable_locals: Vec::new(),
        for_each_iterable_state_fields: Vec::new(),
        for_each_iterable_record_fields: Vec::new(),
        string_capacity_slots: HashMap::new(),
        self_update_scratch: None,
        self_update_scratch_env: None,
        field_pre_emit: None,
        field_pre_emitted: None,
        simd_result_into: None,
        math_pool_base_vreg: None,
        vector_natives: HashMap::new(),
        next_vector_native: 0,
        promoted_vector_locals: HashMap::new(),
        promotable_vector_locals: HashSet::new(),
        resource_containment: HashMap::new(),
        integer_lower_bounds: HashMap::new(),
        integer_strict_upper: std::collections::HashSet::new(),
        for_bound_expr: HashMap::new(),
        len_of_local: HashMap::new(),
        provable_index_locals: HashMap::new(),
        cold_instructions: Vec::new(),
        cold_blocks_enabled: false,
        enclosing_loop_reassigned: Vec::new(),
        graph_copy_walker: None,
        move_sites: None,
        handover: None,
        current_call_key: None,
        args_updated_in_place: Vec::new(),
        current_op_key: None,
        string_app_resource_base: None,
        string_shadow_env: std::collections::HashMap::new(),
        string_app_resource_base_env: None,
        module_name: String::new(),
    }
}

/// Allocate registers, run the helper peepholes and lay out the frame of a
/// [`helper_code_builder`] body whose instructions end in a `return`.
pub(crate) fn finish_helper_function(
    mut builder: CodeBuilder<'_>,
    name: &str,
    params: Vec<CodeParam>,
    returns: &str,
) -> Result<CodeFunction, String> {
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
        name: name.to_string(),
        symbol: builder.current_symbol,
        params,
        returns: returns.to_string(),
        frame,
        instructions,
        relocations: builder.relocations,
        stack_slots,
    })
}
