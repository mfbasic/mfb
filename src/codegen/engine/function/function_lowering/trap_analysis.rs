//! plan-176-U: the `TRAP` analyses — discard conversions, unboxed trap results and trap
//! delivery moves. The function lowering itself (`lower_function`) lives in
//! `engine/function/function_lowering/mod.rs`.

use super::*;

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
pub(super) fn trap_discard_error_results(ops: &[NirOp]) -> HashSet<String> {
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
pub(super) fn unboxed_trap_result_candidates(ops: &[NirOp]) -> HashMap<String, UnboxedTrapUses> {
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
pub(super) fn trap_delivery_moves(ops: &[NirOp]) -> HashSet<String> {
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
