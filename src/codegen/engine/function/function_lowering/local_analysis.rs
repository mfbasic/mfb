//! plan-176-U: the per-function local analyses — union variants, address-taken and reassigned locals, and the
//! borrowed-param and fresh-string return checks. The function lowering itself (`lower_function`) lives in
//! `engine/function/function_lowering/mod.rs`.

use super::*;

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
