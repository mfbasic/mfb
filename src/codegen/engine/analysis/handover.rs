//! plan-147-C: the hand-over analysis — which call arguments may be *given* to the
//! callee instead of lent, and which parameters an owned variant would use up.
//!
//! `mfb spec language functions` §6 ("Parameter passing") and `memory-semantics`
//! §14.1 already license this: an argument is an owned value, and "a copy may become
//! a move when the caller no longer needs the value". Today a collection argument is
//! always lent — `acc = helper(acc, i)` makes the callee copy `acc` to update it —
//! and this module computes what letter D needs to stop that.
//!
//! **Two questions, two answers.**
//!
//! * [`collect_handover_args`] — caller side. The `(op, call, argument)` triples
//!   where a local's value may be handed over.
//! * [`consumable_params`] — callee side. The parameters an owned variant would
//!   consume rather than merely read, which is what decides whether a variant is
//!   worth emitting at all.
//!
//! Letter D consumes both: `lower_function` hands a function's approved sites to
//! the builder (`Builder::handover`), and the variant demand closure in
//! `builder/lower_module.rs:variant_demand` asks for one `<base>$own<mask>` lowering per demanded mask.
//!
//! **Fail closed.** Refusing a hand-over costs a copy; licensing a wrong one hands a
//! callee a block the caller still reads. Every condition below is therefore a
//! refusal, and anything unrecognised refuses.

use std::collections::{HashMap, HashSet};

use crate::codegen::collection::assign::self_update::returned_self_update_local;
use crate::codegen::collection::layout::type_contains_resource;
use crate::codegen::engine::analysis::last_use::{
    kill, live_out_of, op_key, place_live, read_count, reads_of, Place,
};
use crate::codegen::engine::builder::TypeModel;
use crate::target::shared::nir::visit::{walk_value, NirVisitor};
use crate::target::shared::nir::{NirFunction, NirOp, NirValue};
use crate::types::ParameterType;

/// The identity of a call inside an op: its address, for the same reason
/// [`op_key`] uses one — an index into a value tree would drift as the tree is
/// rebuilt, an address cannot. Names a `Call` or a `CallResult` node alike.
pub(crate) fn call_key(value: &NirValue) -> usize {
    value as *const NirValue as usize
}

/// The approved hand-overs of one function, keyed by call site.
///
/// One entry per `(op, call)` that hands over at least one argument, carrying the
/// callee's NIR name and the owned-parameter mask that site needs. Keeping the
/// callee and the mask HERE, rather than re-deriving them at each consumer, is what
/// stops the caller (which must call the variant symbol) and the variant demand
/// (which must emit it) from ever disagreeing about a site.
#[derive(Clone, Debug, Default)]
pub(crate) struct HandOverArgs {
    sites: HashMap<(usize, usize), Site>,
    /// plan-147-E: the `op_key`s of `MUT y = p` / `LET y = p` binds whose source is
    /// an owned PARAMETER at its last use, so the bind may move the block instead of
    /// copying it. That is the `__json_parseArrayItems` shape — `MUT acc = items`
    /// followed by a loop of in-place appends — where the copy is the whole cost.
    param_binds: HashSet<usize>,
}

/// One approved call site: the callee, and which of its parameters this site hands
/// over.
#[derive(Clone, Debug)]
pub(crate) struct Site {
    /// The callee's NIR function name.
    pub(crate) target: String,
    /// Bit `i` set = argument `i` is handed over.
    pub(crate) mask: u64,
    /// plan-147-E: bit `i` set = argument `i` is a **fresh temporary**, not a local.
    /// A subset of `mask`. The two are handed over differently: a local's slot is
    /// nulled, while a temporary is CLAIMED off the pending-temp list, so neither
    /// the statement's post-call drop nor `emit_call_error_exit` frees it.
    pub(crate) temp_mask: u64,
    /// plan-147-E: bit `i` set = argument `i` is `OP(x, …)` on an owned local `x` at
    /// its last use, so the argument is built by updating `x`'s own block in place
    /// and then handing that block over. A subset of `temp_mask` — without the
    /// in-place update the argument would be an ordinary fresh temporary, which is
    /// what it falls back to if no arm fires.
    pub(crate) arg_update_mask: u64,
}

impl Site {
    /// Whether argument `index` is handed over as a fresh temporary.
    pub(crate) fn is_temp(&self, index: usize) -> bool {
        index < 64 && self.temp_mask & (1u64 << index) != 0
    }

    /// Whether argument `index` is an in-place self-update of an owned local.
    pub(crate) fn is_arg_update(&self, index: usize) -> bool {
        index < 64 && self.arg_update_mask & (1u64 << index) != 0
    }
}

impl HandOverArgs {
    /// The site at `(op, call)`, or `None` if nothing there is handed over.
    pub(crate) fn site(&self, op: usize, call: usize) -> Option<&Site> {
        self.sites.get(&(op, call))
    }

    /// The `(callee, mask)` pairs this function's sites ask an owned variant for.
    pub(crate) fn demanded_variants(&self) -> HashSet<(String, u64)> {
        self.sites
            .values()
            .map(|site| (site.target.clone(), site.mask))
            .collect()
    }

    /// Whether the bind at `op` moves an owned parameter instead of copying it.
    pub(crate) fn moves_owned_param(&self, op: usize) -> bool {
        self.param_binds.contains(&op)
    }

    /// How many arguments are approved in total. Test-only: the unit table below
    /// counts approvals through it; no lowering needs a total.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.sites
            .values()
            .map(|site| site.mask.count_ones() as usize)
            .sum()
    }
}

/// The parameter names an owned variant of a function would consume.
pub(crate) type ParamSet = HashSet<String>;

/// plan-147-E: whether `value` is a **fresh temporary** — a value this statement
/// just built, which therefore has no other reader and can be given away whole.
///
/// A direct or trapped call's result, a runtime helper's, a collection literal, or a
/// record construction. Everything else (a local, a global, a field read, a capture)
/// may be reachable from somewhere the caller still reads, so it is not a temporary
/// and this answers `false` — the fail-closed direction.
fn is_fresh_temporary(value: &NirValue) -> bool {
    matches!(
        value,
        NirValue::Call { .. }
            | NirValue::CallResult { .. }
            | NirValue::RuntimeCall { .. }
            | NirValue::ListLiteral { .. }
            | NirValue::SetLiteral { .. }
            | NirValue::MapLiteral { .. }
            | NirValue::Constructor { .. }
    )
}

/// H2/P1: a type whose block is worth handing over — a collection, or a record of
/// them — and never one that carries a resource (plan-147-A §2.3 row S8: resources
/// keep pointer semantics, close-once and drop order, so they are excluded by type),
/// and never a bare `String` (see [`handover_type_within`]).
fn handover_type(model: &TypeModel, type_: &ParameterType) -> bool {
    handover_type_within(model, type_, &mut HashSet::new())
}

/// [`handover_type`], carrying the record types already being examined so a
/// self-referential record cannot recurse forever.
fn handover_type_within(
    model: &TypeModel,
    type_: &ParameterType,
    seen: &mut HashSet<ParameterType>,
) -> bool {
    // Row S8: a resource keeps pointer semantics, close-once and drop order, so
    // anything carrying one is excluded by type. This also covers a record with a
    // `RES` field, and a `RES … STATE` payload.
    if type_contains_resource(model, type_) {
        return false;
    }
    // A bare `String` parameter is NOT handed over. A `String` local can hold a
    // pointer to a **static literal** (`_mfb_str_N`) rather than an arena block:
    // `MUT x AS String = "abc"` binds the symbol itself, and there is no runtime tag
    // that tells the two apart. Handing it over makes the callee's owned parameter
    // free it, and `arena_free` on a static symbol is a **bus error** — the same rule
    // bug-655 states at `pending_temp_would_be_claimed`. Measured: `LET y AS String =
    // strings::padRightToWidth(x, 8)` with `x = "abc"` exited **138 (SIGBUS)**, four
    // `rt_inplace_self_update` case/site pairs.
    //
    // A `String` FIELD of a record is a different thing and stays admissible: it
    // occupies its slot BY VALUE, not as a pointer to a separate allocation
    // (`record_field_is_pointer`: `ParameterType::String => false`), so it moves with
    // the record's own block and is never a loose static pointer. That is why
    // `is_scalar_field` accepts it.
    if matches!(
        type_,
        ParameterType::ListOf(_) | ParameterType::MapOf(_, _) | ParameterType::SetOf(_)
    ) {
        return true;
    }
    // plan-147-F: a RECORD is handed over when every field is, recursively. The
    // census's accumulators are records of collections — `json_schema`'s `Acc` holds
    // three `Map OF String TO String`, threaded as `state = walkSchema(…, state, …)` —
    // and the whole point is to stop copying the record to update one field.
    //
    // A scalar field is fine: it lives in the record's own block and moves with it.
    // What must be refused is anything the record does not solely own, which
    // `type_contains_resource` above already answers for resources and threads.
    let Some(fields) = model.record_fields.get(type_) else {
        return false;
    };
    if !seen.insert(type_.clone()) {
        // Already on the stack: a cycle, which is only reachable through a pointer
        // field the record does not inline. Refuse rather than reason about it.
        return false;
    }
    let ok = fields
        .iter()
        .all(|(_, field)| is_scalar_field(field) || handover_type_within(model, field, seen));
    seen.remove(type_);
    ok
}

/// A field that lives in the record's own block and therefore moves with it.
///
/// `String` belongs here: `record_field_is_pointer` classifies a `String` field as a
/// plain by-value slot, not a pointer to a separate allocation, so it travels inside
/// the record's block. That is exactly what a bare `String` parameter does NOT do,
/// which is why [`handover_type_within`] refuses that one.
fn is_scalar_field(type_: &ParameterType) -> bool {
    matches!(
        type_,
        ParameterType::Integer
            | ParameterType::Float
            | ParameterType::Fixed
            | ParameterType::Boolean
            | ParameterType::Byte
            | ParameterType::Nothing
            | ParameterType::String
    )
}

/// Call `visit` with every direct user call inside `value`, at any depth.
///
/// Two spellings count, and missing the second is a real hole: the inline-`TRAP`
/// desugar rewrites `x = f(x) TRAP(e) …` into a `Bind $trap_res = CallResult{…}`, so
/// a walk that matched only `NirValue::Call` would never even look at a call under a
/// handler — precisely the shape plan-147-A §2.3's S3 rows are about.
///
/// A callback rather than a returned `Vec<&NirValue>`: [`NirVisitor`]'s methods take
/// `&NirValue` with no lifetime of their own, so a visitor cannot hand back borrows
/// that outlive its traversal — but it can do the work during it. Going through the
/// shared `walk_value` is what keeps this from drifting as the NIR tree grows.
fn for_each_call(value: &NirValue, visit: &mut impl FnMut(&NirValue)) {
    struct Calls<'f, F> {
        visit: &'f mut F,
    }
    impl<F: FnMut(&NirValue)> NirVisitor for Calls<'_, F> {
        fn visit_value(&mut self, value: &NirValue) {
            if matches!(value, NirValue::Call { .. } | NirValue::CallResult { .. }) {
                (self.visit)(value);
            }
            walk_value(self, value);
        }
    }
    let mut calls = Calls { visit };
    calls.visit_value(value);
}

/// The types `function` binds its locals to, plus its parameters'.
fn local_types(function: &NirFunction) -> HashMap<String, ParameterType> {
    struct Types {
        types: HashMap<String, ParameterType>,
    }
    impl NirVisitor for Types {
        fn visit_op(&mut self, op: &NirOp) {
            if let NirOp::Bind { name, type_, .. } = op {
                self.types.insert(name.clone(), type_.clone());
            }
            crate::target::shared::nir::visit::walk_op(self, op);
        }
    }
    let mut types = Types {
        types: function
            .params
            .iter()
            .map(|p| (p.name.clone(), p.type_.clone()))
            .collect(),
    };
    types.visit_ops(&function.body);
    types.types
}

/// The `(op, call, argument)` triples of `function` whose argument may be handed to
/// the callee instead of lent (§3.1 of plan-147-C).
///
/// `callees` resolves a call target to the user function it names — the merged
/// module's functions, packages included. A target it does not know is a builtin, a
/// `LINK` function, or a call through a function value, and is refused (H5).
pub(crate) fn collect_handover_args(
    function: &NirFunction,
    model: &TypeModel,
    callees: &HashMap<String, &NirFunction>,
    // plan-147-E: the parameters THIS lowering owns. Empty for a base lowering, where
    // the caller keeps every argument's block; the variant's mask for an owned
    // variant, where a parameter is an ordinary owned local and may be handed on.
    owned_params: &ParamSet,
    // bug-1019: the functions reachable as a `FUNC` value — the set
    // `function_returns_param_borrow` is decided over, so H7 answers exactly as the
    // callee's own lowering and every caller's classification of its result do.
    callback_referenced: &HashSet<String>,
) -> HandOverArgs {
    let live = live_out_of(function, model);
    let types = local_types(function);
    // H6 asks the same question of a callee once per call target, not once per call.
    let mut consumable: HashMap<String, ParamSet> = HashMap::new();
    // H7 likewise.
    let mut borrowed: HashMap<String, ParamSet> = HashMap::new();

    let mut sites: HashMap<(usize, usize), Site> = HashMap::new();
    let mut param_binds: HashSet<usize> = HashSet::new();
    for (key, (op, out)) in &live.after {
        // Only a simple statement is a site, exactly as in `collect_last_use_moves`:
        // a compound op's value is a loop condition or a branch test, which the
        // lowering may evaluate more than once.
        let (values, killed): (Vec<&NirValue>, Option<&str>) = match op {
            NirOp::Bind { name, value, .. } => (value.iter().collect(), Some(name.as_str())),
            NirOp::Assign { name, value } => (vec![value], Some(name.as_str())),
            NirOp::StoreGlobal { value, .. } => (value.iter().collect(), None),
            NirOp::StateAssign { value, .. } => (vec![value], None),
            NirOp::Eval { value } => (vec![value], None),
            NirOp::Return { value } => (value.iter().collect(), None),
            NirOp::Fail { error } => (vec![error], None),
            NirOp::ExitProgram { code } => (vec![code], None),
            NirOp::ExitLoop { .. }
            | NirOp::ContinueLoop { .. }
            | NirOp::If { .. }
            | NirOp::Match { .. }
            | NirOp::While { .. }
            | NirOp::For { .. }
            | NirOp::DoUntil { .. }
            | NirOp::ForEach { .. }
            | NirOp::Trap { .. } => continue,
        };

        // Every place the op reads, so H3's "read exactly once" is asked of the WHOLE
        // op and not just of this call: `x = pair(x, x)` is two reads (row S6), and so
        // is `f(x) + len(x)`.
        let all_reads: Vec<Place> = values.iter().flat_map(|value| reads_of(value)).collect();

        // What can still observe a place after this op: what is live after it, minus
        // the op's own store target (that names a NEW value, so the old one is dead),
        // plus everything any handler reads (row S3).
        let mut after = match op {
            NirOp::Return { .. } | NirOp::Fail { .. } | NirOp::ExitProgram { .. } => HashSet::new(),
            _ => out.clone(),
        };
        if let Some(name) = killed {
            kill(&mut after, name);
        }
        after.extend(live.trap_live.iter().cloned());

        // plan-147-E: `MUT y = p` on an owned parameter at its last use. The same H1-H3
        // questions as a call argument, asked of a bind: the parameter must be one
        // this lowering owns, of a hand-over type, read exactly once here, and dead
        // afterwards (including in any handler).
        if let NirOp::Bind {
            value: Some(NirValue::Local(source)),
            ..
        } = op
        {
            let place = Place::Local(source.clone());
            if owned_params.contains(source.as_str())
                && types
                    .get(source.as_str())
                    .is_some_and(|type_| handover_type(model, type_))
                && read_count(&all_reads, &place) == 1
                && !place_live(&after, &place)
            {
                param_binds.insert(*key);
            }
        }

        for value in &values {
            for_each_call(value, &mut |call| {
                let (NirValue::Call {
                    target,
                    args: call_args,
                    ..
                }
                | NirValue::CallResult {
                    target,
                    args: call_args,
                    ..
                }) = call
                else {
                    return;
                };
                // H5: a direct call to a user function whose body is in the module.
                let Some(callee) = callees.get(target.as_str()) else {
                    return;
                };
                // S10: an `ISOLATED` entry point is not called directly.
                if callee.isolated {
                    return;
                }
                let wanted = consumable
                    .entry(target.clone())
                    .or_insert_with(|| consumable_params(callee, model));
                for (index, arg) in call_args.iter().enumerate() {
                    // H6: handing over a parameter the callee only reads would move a
                    // free from caller to callee and nothing else.
                    let Some(param) = callee.params.get(index) else {
                        continue;
                    };
                    if !wanted.contains(&param.name) {
                        continue;
                    }
                    // H7 (bug-1019): a parameter the callee returns as a BORROW stays
                    // the caller's. A pure passthrough (`RETURN x`, plan-86 K1) hands
                    // back the argument's own pointer and every caller reads that
                    // result as a borrow of the argument: it copies it at an owning
                    // store and frees nothing, because the argument's owner frees the
                    // block. Handing the argument over removes that owner — the owned
                    // variant moves the block straight back out as its result — so
                    // the block was freed by no one. `LET d = idOf(squares(20.0))`
                    // leaked the whole list on every call.
                    //
                    // H6 calls `RETURN x` a consuming use, and for a function that
                    // also builds its result (`RETURN x` on one path, `RETURN
                    // append(x, n)` on another) it is: that function's result is a
                    // fresh block on every path, the caller owns it, and the move
                    // saves a copy. It is the function whose EVERY return is a bare
                    // parameter that promises a borrow instead, and that promise is
                    // the one this refuses to break. A parameter such a function
                    // does not return is unaffected.
                    let lent = borrowed.entry(target.clone()).or_insert_with(|| {
                        crate::codegen::engine::function::function_lowering::param_borrow_returns(
                            callee,
                            callback_referenced,
                        )
                        .unwrap_or_default()
                    });
                    if lent.contains(&param.name) {
                        continue;
                    }
                    // plan-147-E: a FRESH TEMPORARY argument is handed over with no
                    // further question. It is a value this statement just built, so
                    // nothing else can read it — there is no liveness to check and no
                    // caller slot to null. Its type is the parameter's, which H6 has
                    // already established is consumable and therefore H2.
                    if is_fresh_temporary(arg) {
                        if !handover_type(model, &param.type_) {
                            continue;
                        }
                        // plan-147-E: the temporary is often `OP(x, …)` on a local
                        // the caller is done with — `fill(collections::append(xs, n),
                        // n - 1)` is the recursive shape this plan exists for. Handing
                        // the temporary over is not enough there: BUILDING it still
                        // copies `x`. When `x` is an owned local at its last use, the
                        // argument is instead built by updating `x`'s own block in
                        // place, and that block is what is handed over — the same
                        // reduction letter B made for `RETURN OP(x, …)`, one position
                        // over.
                        let arg_update = returned_self_update_local(arg)
                            .or_else(|| {
                                crate::codegen::collection::assign::self_update::
                                    returned_field_self_update_local(arg)
                            })
                            .is_some_and(|name| {
                                (!live.excluded.contains(name) || owned_params.contains(name))
                                    && types
                                        .get(name)
                                        .is_some_and(|type_| handover_type(model, type_))
                                    && read_count(&all_reads, &Place::Local(name.to_string())) == 1
                                    && !place_live(&after, &Place::Local(name.to_string()))
                            });
                        let entry = sites.entry((*key, call_key(call))).or_insert_with(|| Site {
                            target: target.clone(),
                            mask: 0,
                            temp_mask: 0,
                            arg_update_mask: 0,
                        });
                        if index < 64 {
                            entry.mask |= 1u64 << index;
                            entry.temp_mask |= 1u64 << index;
                            if arg_update {
                                entry.arg_update_mask |= 1u64 << index;
                            }
                        }
                        continue;
                    }
                    // H4 is structural: `Place` names no globals (row S7).
                    let NirValue::Local(name) = arg else {
                        continue;
                    };
                    // H1: an owned local of this function. `excluded_roots` excludes
                    // every parameter, because in the BASE lowering the caller owns
                    // the block; in an owned variant this lowering owns it, so the
                    // exclusion is lifted for exactly those names.
                    if live.excluded.contains(name.as_str())
                        && !owned_params.contains(name.as_str())
                    {
                        continue;
                    }
                    // H2: the type.
                    if !types
                        .get(name.as_str())
                        .is_some_and(|type_| handover_type(model, type_))
                    {
                        continue;
                    }
                    // H3: read exactly once in the op, and dead after it.
                    let place = Place::Local(name.clone());
                    if read_count(&all_reads, &place) != 1 || place_live(&after, &place) {
                        continue;
                    }
                    let entry = sites.entry((*key, call_key(call))).or_insert_with(|| Site {
                        target: target.clone(),
                        mask: 0,
                        temp_mask: 0,
                        arg_update_mask: 0,
                    });
                    if index < 64 {
                        entry.mask |= 1u64 << index;
                    }
                }
            });
        }
    }
    HandOverArgs { sites, param_binds }
}

/// The parameters of `function` an owned variant would consume — use up rather than
/// merely read (§3.2 of plan-147-C).
///
/// This decides only whether a variant is *worth* emitting. Correctness does not
/// depend on it: an owned variant frees any owned parameter it does not consume
/// (letter D).
pub(crate) fn consumable_params(function: &NirFunction, model: &TypeModel) -> ParamSet {
    let live = live_out_of(function, model);
    let mut consumable = ParamSet::new();

    for param in &function.params {
        // P1: the type.
        if !handover_type(model, &param.type_) {
            continue;
        }
        // P2: it would pass H1 if it were a local — not captured, not address-taken,
        // not a `FOR EACH` variable. (`excluded_roots` also excludes every parameter
        // as such, since the CALLER owns one today; that is the very rule an owned
        // variant changes, so a bare "is a parameter" exclusion is not consulted
        // here — the structural reasons are, through `consuming_use` refusing
        // anything but a direct consuming read.)
        if is_captured_or_address_taken(function, &param.name) {
            continue;
        }
        // P3: some path ends the parameter's life with a consuming use.
        if consuming_use(function, model, &live, &param.name) {
            consumable.insert(param.name.clone());
        }
    }
    consumable
}

/// P2: whether a closure captures `name`, or its address is taken.
fn is_captured_or_address_taken(function: &NirFunction, name: &str) -> bool {
    struct Scan<'n> {
        name: &'n str,
        found: bool,
    }
    impl NirVisitor for Scan<'_> {
        fn visit_value(&mut self, value: &NirValue) {
            match value {
                NirValue::Capture { .. } | NirValue::LocalRef { .. } => {
                    for place in reads_of(value) {
                        if place.root() == self.name {
                            self.found = true;
                        }
                    }
                }
                NirValue::Closure { captures, .. } => {
                    // A capture is a VALUE, so a capture of `x.f` names `x` too.
                    for capture in captures {
                        for place in reads_of(capture) {
                            if place.root() == self.name {
                                self.found = true;
                            }
                        }
                    }
                }
                _ => {}
            }
            walk_value(self, value);
        }
        fn visit_op(&mut self, op: &NirOp) {
            if let NirOp::ForEach { name, .. } | NirOp::For { name, .. } = op {
                if name == self.name {
                    self.found = true;
                }
            }
            crate::target::shared::nir::visit::walk_op(self, op);
        }
    }
    let mut scan = Scan { name, found: false };
    scan.visit_ops(&function.body);
    scan.found
}

/// P3: whether any op ends `name`'s life by consuming it — `RETURN OP(name, …)`
/// (letter B's site S11), `RETURN name` (`plan_returned_move`'s move), or
/// plan-147-E's `MUT y = name` / `LET y = name`, which moves the block into the new
/// binding instead of copying it, and bug-766 C's `RETURN WITH name { … }` over
/// fields stored in `name`'s own block ([`returned_field_store_local`]).
fn consuming_use(
    function: &NirFunction,
    model: &TypeModel,
    live: &crate::codegen::engine::analysis::last_use::LiveOut<'_>,
    name: &str,
) -> bool {
    let place = Place::Local(name.to_string());
    for op in ops_of(function) {
        // plan-147-E: a bind of the bare parameter consumes it.
        if let NirOp::Bind {
            value: Some(NirValue::Local(source)),
            ..
        } = op
        {
            if source == name && !place_live(&live.trap_live, &place) {
                return true;
            }
        }
        let NirOp::Return { value: Some(value) } = op else {
            continue;
        };
        // plan-147-F: the FIELD form reads its owner TWICE by construction — once as
        // the `WITH`'s base and once as the field's source (`WITH r { f := OP(r.f,
        // …) }`) — so the "read exactly once" question below is the wrong one to ask
        // of it. Both reads belong to the one statement that consumes `r`, which is
        // what the shape itself guarantees; the liveness check still applies.
        //
        // bug-766 C: `RETURN WITH r { <fields stored in r's own block> }` reads its
        // owner as often as its update values name it, for the same reason, and is
        // consumed the same way — the owned variant stores the new values into `r`'s
        // block (`try_inplace_scalar_fields`) and then moves that block out.
        let field_form =
            crate::codegen::collection::assign::self_update::returned_field_self_update_local(
                value,
            ) == Some(name)
                || returned_field_store_local(model, value) == Some(name);
        let consumes = field_form
            || match value {
                NirValue::Local(local) => local == name,
                _ => {
                    crate::codegen::collection::assign::self_update::returned_self_update_local(
                        value,
                    ) == Some(name)
                }
            };
        if !consumes {
            continue;
        }
        // The `RETURN` must be the parameter's last read on this path: a handler that
        // reads it afterwards would see a consumed value (row S3).
        let reads: Vec<Place> = reads_of(value);
        if !field_form && read_count(&reads, &place) != 1 {
            continue;
        }
        if place_live(&live.trap_live, &place) {
            continue;
        }
        // A `Return` has nothing live after it, so `live.after` adds no condition.
        let _ = live.after.get(&op_key(op));
        return true;
    }
    false
}

/// bug-766 C: the owner local of a `RETURN WITH r { … }` whose every update is a
/// field the in-place field store (`try_inplace_scalar_fields`) writes into `r`'s
/// own block — the non-collection twin of plan-147-F's
/// `returned_field_self_update_local`.
///
/// Admitted, per update: a scalar slot, a pointer slot, an inlined record of fixed
/// size (plan-145-F), and the record's LAST inlined sub-block when it is a `String`
/// or a record of value-dependent size (bug-766 D — replaced in place when its
/// 16-byte granule is unchanged, else the owner is rebuilt). Anything else — an
/// inlined collection, a variable-width field that is not the last — answers
/// `None`, since the lowering would rebuild the record anyway.
///
/// This mirrors the lowering's classification from the type model alone. It
/// decides only whether an owned variant is worth emitting; if the lowering
/// declines at a site the lowering asks more of (the owner's freeability, an
/// `Error` record), the variant builds a fresh block and frees the parameter —
/// the base lowering's cost, never a wrong answer.
fn returned_field_store_local<'v>(model: &TypeModel, value: &'v NirValue) -> Option<&'v str> {
    use crate::codegen::collection::layout::record_field_is_inlined;
    use crate::codegen::engine::types::typed_is_collection_type;
    let NirValue::WithUpdate {
        type_,
        target,
        updates,
    } = value
    else {
        return None;
    };
    let NirValue::Local(name) = target.as_ref() else {
        return None;
    };
    let fields = model.record_fields.get(type_)?;
    if updates.is_empty() {
        return None;
    }
    for update in updates {
        let index = fields
            .iter()
            .position(|(field, _)| *field == update.field)?;
        let field_type = &fields[index].1;
        if !record_field_is_inlined(model, field_type) {
            // A scalar or a pointer slot: stored (a pointer's old pointee dropped)
            // in the owner's block.
            continue;
        }
        if inlined_fixed(model, field_type) {
            continue;
        }
        let variable = *field_type == ParameterType::String
            || (model.record_fields.contains_key(field_type)
                && !typed_is_collection_type(field_type));
        let last = !fields[index + 1..]
            .iter()
            .any(|(_, later)| record_field_is_inlined(model, later));
        if !(variable && last) {
            return None;
        }
    }
    Some(name.as_str())
}

/// `record_field_is_inlined_fixed` over the type model: an inlined record whose
/// every field is a scalar slot or itself such a record, so any value of the type
/// fills exactly its sub-block.
fn inlined_fixed(model: &TypeModel, type_: &ParameterType) -> bool {
    use crate::codegen::collection::layout::{record_field_is_inlined, record_field_is_pointer};
    use crate::codegen::engine::types::typed_is_collection_type;
    let Some(fields) = model.record_fields.get(type_) else {
        return false;
    };
    fields.iter().all(|(_, field)| {
        if record_field_is_inlined(model, field) {
            !typed_is_collection_type(field) && inlined_fixed(model, field)
        } else {
            !record_field_is_pointer(model, field)
        }
    })
}

/// Every op of `function`, at any depth. Hand-written for the same reason as
/// [`calls_in`]: a [`NirVisitor`] cannot collect borrows that outlive its traversal.
fn ops_of(function: &NirFunction) -> Vec<&NirOp> {
    fn walk<'f>(ops: &'f [NirOp], out: &mut Vec<&'f NirOp>) {
        for op in ops {
            out.push(op);
            match op {
                NirOp::If {
                    then_body,
                    else_body,
                    ..
                } => {
                    walk(then_body, out);
                    walk(else_body, out);
                }
                NirOp::Match { cases, .. } => {
                    for case in cases {
                        walk(&case.body, out);
                    }
                }
                NirOp::While { body, .. }
                | NirOp::For { body, .. }
                | NirOp::DoUntil { body, .. }
                | NirOp::ForEach { body, .. }
                | NirOp::Trap { body, .. } => walk(body, out),
                // No wildcard: a new statement `NirOp` is a build error here, exactly
                // as in `last_use.rs`.
                NirOp::Bind { .. }
                | NirOp::Assign { .. }
                | NirOp::StoreGlobal { .. }
                | NirOp::StateAssign { .. }
                | NirOp::Eval { .. }
                | NirOp::Return { .. }
                | NirOp::Fail { .. }
                | NirOp::ExitProgram { .. }
                | NirOp::ExitLoop { .. }
                | NirOp::ContinueLoop { .. } => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(&function.body, &mut out);
    out
}

#[cfg(test)]
mod tests;
