// --- codegen tier imports (migration) ---
use crate::codegen::collection::layout::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::function::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::*;
use crate::codegen::error::constants::*;
use crate::operators::BinaryOp;
use crate::target::shared::abi;
use crate::target::shared::nir::*;
use crate::types::ParameterType;
impl CodeBuilder<'_> {
    /// Whether a statement unconditionally branches away (so the fall-through
    /// statement-scope temp free would be unreachable): `RETURN`, `EXIT`,
    /// `CONTINUE`, program exit, or `Fail`. A returned/failed fresh temp is moved
    /// to the target and must not be freed here.
    fn op_transfers_control(op: &NirOp) -> bool {
        matches!(
            op,
            NirOp::Return { .. }
                | NirOp::ExitLoop { .. }
                | NirOp::ContinueLoop { .. }
                | NirOp::ExitProgram { .. }
                | NirOp::Fail { .. }
        )
    }

    pub(crate) fn lower_ops(&mut self, ops: &[NirOp]) -> Result<(), String> {
        let cleanup_scope_start = self.active_cleanups.len();
        self.cleanup_scope_starts.push(cleanup_scope_start);
        let result = self.lower_ops_inner(ops, cleanup_scope_start);
        self.cleanup_scope_starts.pop();
        result
    }

    /// bug-424 Layer 1: recognize `s.state.field = <value>` where every updated
    /// field is a fixed-width scalar stored inline in its record slot, and store
    /// each new value in place at its field offset in the *existing* STATE block
    /// — no whole-record rebuild, so no re-copy of any other (possibly large,
    /// inlined `List`/`String`) field. `src/ast/stmt.rs` desugars the single-field
    /// form to a single-field `WITH` update over `s.state`, so this matches a
    /// `WithUpdate` whose target is exactly this resource's `state`.
    ///
    /// Only plain scalars are eligible: an inlined field (`String`, a flat
    /// collection, a nested record — the slot holds a block-relative offset into
    /// the trailing data region) or a pointer composite cannot be overwritten at a
    /// fixed slot without relaying the block out, so those fall through to the
    /// whole-record replace (`NirOp::StateAssign`) and Layer 2. The store goes
    /// through the resource record's shared STATE pointer, so it stays visible to
    /// the owner and every alias (§15). Returns `true` when handled in place.
    fn try_inplace_state_scalar_assign(
        &mut self,
        resource: &str,
        value: &NirValue,
    ) -> Result<bool, String> {
        let NirValue::WithUpdate {
            type_,
            target,
            updates,
        } = value
        else {
            return Ok(false);
        };
        // The update must rebuild THIS resource's current state (`resource.state`),
        // not install some other record as the new state.
        let NirValue::MemberAccess {
            target: inner,
            member,
        } = target.as_ref()
        else {
            return Ok(false);
        };
        if member != "state" || !matches!(inner.as_ref(), NirValue::Local(name) if name == resource)
        {
            return Ok(false);
        }
        let Some(fields) = self.type_model.record_fields.get(type_).cloned() else {
            return Ok(false);
        };
        // Every updated field must be a plain inline scalar. A `String`/collection/
        // nested field is inlined (a block-relative offset) or a pointer composite;
        // overwriting it at a fixed slot would corrupt the block or leak the old
        // allocation, so bail to the whole-record replace.
        let mut indices = Vec::with_capacity(updates.len());
        for update in updates {
            let Some((index, (_, field_type))) = fields
                .iter()
                .enumerate()
                .find(|(_, (name, _))| name == &update.field)
            else {
                return Ok(false);
            };
            if self.record_field_is_inlined(type_, field_type)
                || self.record_field_is_pointer(field_type)
            {
                return Ok(false);
            }
            indices.push(index);
        }
        if indices.is_empty() {
            return Ok(false);
        }
        // Eligible. Compute every new value first (source order, matching WITH so a
        // field that reads another field's old value sees it), spilling each to a
        // slot; then store them into the existing STATE block.
        let mut stores = Vec::with_capacity(updates.len());
        for (update, index) in updates.iter().zip(indices) {
            let value = self.lower_value(&update.value)?;
            // Observation boundary: a `Float` field must be finite (plan-17).
            self.observe_float(&update.value, &value)?;
            // Materialize a `d`-native float to its GP bit pattern before the spill
            // (plan-01), matching how `lower_with_update` gathers field values.
            let value = self.materialize_value(value)?;
            let slot = self.allocate_stack_object("state_field_inplace", 8);
            self.emit(abi::store_u64(&value.location, abi::stack_pointer(), slot));
            stores.push((index, slot));
        }
        // Load the shared STATE record pointer from the resource record. A scalar
        // store never moves the block, so one load serves every field store.
        let local = self
            .locals
            .get(resource)
            .ok_or_else(|| format!("native code state assignment unknown local '{resource}'"))?;
        let stack_offset = local.stack_offset;
        let resource_type = local.type_.clone();
        let block = self.allocate_register();
        self.emit(abi::load_u64(&block, abi::stack_pointer(), stack_offset));
        let record = self.emit_resource_record_ptr(&block, &resource_type)?;
        let state_ptr = self.allocate_register();
        self.emit(abi::load_u64(&state_ptr, &record, RESOURCE_OFFSET_STATE));
        for (index, slot) in stores {
            let value = self.allocate_register();
            self.emit(abi::load_u64(&value, abi::stack_pointer(), slot));
            self.emit(abi::store_u64(&value, &state_ptr, 8 * index));
        }
        Ok(true)
    }

    /// True when `value` reads exactly `<resource>.state.<field>` — the
    /// self-append source/alias check for bug-430.
    pub(crate) fn value_is_state_field(
        &self,
        value: &NirValue,
        resource: &str,
        field: &str,
    ) -> bool {
        let NirValue::MemberAccess { target, member } = value else {
            return false;
        };
        if member != field {
            return false;
        }
        let NirValue::MemberAccess {
            target: inner,
            member: inner_member,
        } = target.as_ref()
        else {
            return false;
        };
        inner_member == "state" && matches!(inner.as_ref(), NirValue::Local(n) if n == resource)
    }

    /// True when `value` reads exactly `<local>.<field>` — the self-append
    /// source/alias check for a MUT record field grow (bug-430).
    pub(crate) fn value_is_record_field(&self, value: &NirValue, local: &str, field: &str) -> bool {
        matches!(
            value,
            NirValue::MemberAccess { target, member }
                if member == field && matches!(target.as_ref(), NirValue::Local(n) if n == local)
        )
    }

    /// If `field` is a **collection** field of `record_type` that is inlined AND
    /// is the **last inlined field** (no later field is inlined, so growing its
    /// trailing sub-block extends the record block's tail without shifting any
    /// sibling), return `(field_index, field_type)`. This is the shape bug-430's
    /// in-place grow supports; every other shape falls back to the whole-record
    /// rebuild.
    ///
    /// plan-121-C: this is a *container* question — "can this field's sub-block be
    /// mutated where it lies" — and the answer does not depend on which operation
    /// is about to run. It used to reject anything that was not a `List`, because
    /// its only caller was `append`. That coupled the container to one operation
    /// and made `add`/`removeKey` on a record-held `Set`/`Map` unreachable even
    /// though their sub-blocks are inlined on exactly the same terms.
    ///
    /// Widening it is safe because **each arm still gates its own kind**: the
    /// append arms take `typed_list_element_type` right after this returns (`G9`),
    /// so a `Map`/`Set` field cannot reach a list lowering by this route.
    pub(crate) fn record_collection_last_inlined(
        &self,
        record_type: &ParameterType,
        field: &str,
    ) -> Option<(usize, ParameterType)> {
        let fields = self.type_model.record_fields.get(record_type)?;
        let index = fields.iter().position(|(name, _)| name == field)?;
        let field_type = fields[index].1.clone();
        // A `List`, `Map` or `Set` — the three block-backed collection kinds.
        // `record_field_is_inlined` below already asks `typed_is_collection_type`
        // among its composite cases; asking here too keeps the *reason* for the
        // refusal specific ("not a collection" rather than "not inlined").
        if !typed_is_collection_type(&field_type) {
            return None;
        }
        if !self.record_field_is_inlined(record_type, &field_type) {
            return None;
        }
        // No field after this one may be inlined, or growing this sub-block would
        // shift the later sub-blocks (and their stored offsets).
        if fields
            .iter()
            .skip(index + 1)
            .any(|(_, ft)| self.record_field_is_inlined(record_type, ft))
        {
            return None;
        }
        Some((index, field_type.clone()))
    }

    /// plan-121-D Phase 1: the **operation dispatch** for a collection held in a
    /// `RES … STATE` block, the STATE analogue of the `try_inplace_record_field_*`
    /// chain in `Assign` above.
    ///
    /// The split this introduces is between the two questions an in-place arm has
    /// to answer, which used to be tangled in one function:
    ///
    /// * **Which container is this?** — `resolve_inplace_state_field` (the
    ///   container matcher): the `WITH` shape, `G13` the target is exactly this
    ///   resource's `.state`, `G14` the single updated field, `G16` no live
    ///   `FOR EACH` over it, `G17` last-inlined, `G10` the layout. None of that
    ///   depends on which operation is running, which is exactly why it is shared.
    /// * **Which operation is this?** — this function, one arm per builtin.
    ///
    /// **All seven mutating operations now reach a collection held in a
    /// `RES … STATE` block in place.** Phase 1 dispatched `append` alone and
    /// changed no behaviour; Phase 2 added `removeKey`, `add` and `set`; Phase 3
    /// added `removeAt`, Set `remove`, `insert` and `prepend`.
    ///
    /// Order is irrelevant to correctness — each arm re-matches the operation name
    /// through `resolve_inplace_state_field`, so at most one can accept a given
    /// statement — but it is kept in phase order so the ledger reads against the
    /// code.
    ///
    /// Returning `false` is always correct: it falls through to the whole-record
    /// STATE replace below, which is the slow path, never a wrong one.
    fn try_inplace_state_collection_assign(
        &mut self,
        resource: &str,
        value: &NirValue,
    ) -> Result<bool, String> {
        Ok(self.try_inplace_state_collection_append(resource, value)?
            || self.try_inplace_state_remove_key_assign(resource, value)?
            || self.try_inplace_state_set_add_assign(resource, value)?
            || self.try_inplace_state_set_assign(resource, value)?
            || self.try_inplace_state_remove_at_assign(resource, value)?
            || self.try_inplace_state_set_remove_assign(resource, value)?
            || self.try_inplace_state_insert_assign(resource, value)?
            || self.try_inplace_state_prepend_assign(resource, value)?)
    }

    /// bug-430: recognize `s.state.coll = collections::append(s.state.coll, x)`
    /// where `coll` is a `List` field that is the last inlined field of the STATE
    /// record, and grow it IN PLACE inside the existing STATE block (amortized
    /// O(1) append with geometric headroom) instead of rebuilding the whole
    /// record and re-inlining the accumulated buffer (the O(n²) path). `x` may be
    /// a single element (`T`) or a whole list (`List OF T`, concatenation). On a
    /// realloc the new STATE pointer is written back through the resource's shared
    /// STATE slot, so the owner and every alias observe the grown block (§15).
    /// Anything else — a non-last-inlined collection, a whole-state replace, or an
    /// append whose source is not this same field — returns `false` and falls
    /// through to the whole-record replace.
    fn try_inplace_state_collection_append(
        &mut self,
        resource: &str,
        value: &NirValue,
    ) -> Result<bool, String> {
        // Container (plan-121-A's shared seam): the `RES … STATE` self-update
        // `res.state.field = append(res.state.field, …)`, which `src/ast/stmt.rs`
        // desugars to a single-field `WITH` over `res.state`. Discharges G2 the
        // shape, G13 the target is exactly this resource's `.state`, G14 the
        // single updated field, G16 the live `FOR EACH` over this state field
        // (bug-430; the alias analogue of the `for_each_iterable_locals` guard),
        // G17 last-inlined, G10 the layout, and G3/G4 the call target and arity.
        let Some(target) = self.resolve_inplace_state_field(resource, value, "append", 2) else {
            return Ok(false);
        };
        let field_type = target.field_type.clone();
        // G9 — `append` mutates a List. (Subsumed by G17; kept for the element
        // type the lowering needs.)
        let Some(element_type) = typed_list_element_type(&field_type).cloned() else {
            return Ok(false);
        };
        // G18 — the appended-to source must be exactly this same field
        // (self-append), the invariant that makes an in-place grow sound.
        if !self.value_is_state_field(&target.args[0], resource, target.field) {
            return Ok(false);
        }
        // G11 — single element (item type == element type) vs bulk concatenation
        // (item type == the whole list type).
        //
        // `static_item_type`, not `static_type_name`: the narrow helper's
        // `NirValue::Call` arm is a hand-written table of a few builtin names and
        // answers `None` for EVERY user function, so
        // `f.state.xs = append(f.state.xs, someFunc(x))` fell off this path
        // entirely — not even reaching the bulk grow — and rebuilt the whole STATE
        // block per element (O(n²)), while the identical record-field program was
        // fast. This was the one gate site the widening never reached; see
        // `planning/plan-121-gate-inventory.md` §"DEFECT FOUND". Reading a
        // callee's declared `returns` is exactly as static as reading a local's
        // declared type, and the `field_type` arm below still separates a whole
        // `List OF T` result from a `T` one, so the widening cannot reclassify a
        // concatenation as a single element.
        let bulk = match self.static_item_type(&target.args[1]) {
            Some(t) if t == element_type => false,
            Some(t) if t == field_type => true,
            _ => return Ok(false),
        };
        // G12 — exclude the self-alias `append(field, field)`: the grow frees the
        // old block out from under the RHS copy. Fall back to the value path.
        if self.value_is_state_field(&target.args[1], resource, target.field) {
            return Ok(false);
        }

        // Load the shared STATE record pointer into a slot the grow helper
        // repoints. Emits, so it runs after every gate (`O-order-1`) and BEFORE
        // the operand is lowered (`O-order-4`: the operand's own lowering must not
        // observe a stale STATE pointer).
        let dest = self.open_inplace_state_dest(resource, target.field_index)?;

        // Evaluate the appended value and spill it for the grow helper.
        let rhs = self.lower_value(&target.args[1])?;
        self.observe_float(&target.args[1], &rhs)?;
        let rhs = self.materialize_value(rhs)?;
        let rhs_slot = self.allocate_stack_object("inline_state_rhs", 8);
        self.emit(abi::store_u64(
            &rhs.location,
            abi::stack_pointer(),
            rhs_slot,
        ));

        self.lower_inplace_inlined_list_grow(&dest, bulk, &field_type, &element_type, rhs_slot)?;

        // O4 — publish the (possibly new) STATE pointer back through the
        // resource's shared STATE slot so the owner and every alias observe the
        // grown block (§15).
        self.close_inplace_dest(&dest)?;
        Ok(true)
    }

    fn lower_ops_inner(&mut self, ops: &[NirOp], cleanup_scope_start: usize) -> Result<(), String> {
        let zero_slot = self.temporary_vreg();
        for op in ops {
            // plan-39 I1: keep the guard-derived range maps sound. Any op that can
            // re-enter or transfer control non-linearly (loops / Match / Trap) drops
            // all bounds *before* it is lowered (a `While`'s own condition then
            // re-establishes the strict-upper bound for its body). Per-local
            // invalidation on `Bind`/`Assign` happens *after* the op is lowered
            // (below), so the RHS of `i = i + 1` can still use `i`'s pre-assignment
            // bound. Dropping a bound only ever keeps an overflow check, so this is
            // unconditionally safe.
            match op {
                NirOp::While { .. }
                | NirOp::For { .. }
                | NirOp::ForEach { .. }
                | NirOp::DoUntil { .. }
                | NirOp::Match { .. }
                | NirOp::Trap { .. } => {
                    self.integer_lower_bounds.clear();
                    self.integer_strict_upper.clear();
                }
                _ => {}
            }
            // Fresh interior heap temporaries produced while lowering this
            // statement are freed when it finishes (plan-25 temp-lifetime fix), so
            // a hot loop body does not accumulate them until the function returns.
            let temp_watermark = self.pending_temp_frees.len();
            // plan-118-A: attribute the instructions this statement emits to its
            // op kind. The closure below is what makes the pairing safe — no `?`
            // can jump over the `exit`.
            crate::codegen::engine::expansion::enter(
                || crate::codegen::engine::expansion::op_key(op).to_string(),
                self.instructions.len(),
            );
            let result = (|| -> Result<(), String> {
                match op {
                    NirOp::Bind {
                        name, type_, value, ..
                    } => {
                        let stack_offset = self.allocate_stack_object(name, 8);
                        // A non-escaping `MUT` by-ref capture: the env slot holds a
                        // pointer to the parent binding's slot, so this binding is a
                        // *reference* local — its slot stores that pointer and reads
                        // /writes deref through it. It is non-owning:
                        // never deep-copied and never freed here (the parent owns
                        // and frees the value).
                        let by_ref_capture_slot =
                            matches!(value, Some(NirValue::Capture { by_ref: true, .. }));
                        // A reference local must never carry a folded constant: its
                        // value lives in the parent slot and can change underneath
                        // it, so every read must deref.
                        let constant = if by_ref_capture_slot {
                            None
                        } else {
                            value
                                .as_ref()
                                .and_then(|value| self.local_constant_value(value))
                        };
                        self.locals.insert(
                            name.clone(),
                            LocalValue {
                                type_: type_.clone(),
                                stack_offset,
                                constant,
                                by_ref: by_ref_capture_slot,
                            },
                        );
                        // Clear any stale vector-promotion for a rebound name (e.g.
                        // the same name promoted in one branch and block-bound in
                        // another); it is re-established below only if this binding
                        // promotes (plan-01-vector).
                        self.promoted_vector_locals.remove(name);
                        // plan-86 G1: track `LET n = len(L)` (both locals) so a loop
                        // bound `n - k` can resolve to `len(L) - k`. A rebind of `n`
                        // drops its fact; a rebind of any list drops facts naming it
                        // as `L` (its length may have changed).
                        self.len_of_local.remove(name);
                        self.len_of_local.retain(|_, (l, _)| l != name);
                        if let Some(NirValue::Call { target, args, .. }) = value {
                            if target == "len" && args.len() == 1 {
                                if let NirValue::Local(l) = &args[0] {
                                    self.len_of_local.insert(
                                        name.clone(),
                                        (l.clone(), self.enclosing_loop_reassigned.len()),
                                    );
                                }
                            }
                        }
                        // Record the synthetic FOR-bound/step binds so the loop's
                        // `end`/`step` (which arrive as `Local($for_endN)`) resolve.
                        if name.starts_with("$for_end") || name.starts_with("$for_step") {
                            if let Some(v) = value {
                                self.for_bound_expr.insert(name.clone(), v.clone());
                            }
                        }
                        // plan-86 G1: a rebind clears any provable-index fact keyed on
                        // `name`, and any fact naming it as the indexed list `L`. An
                        // alias `LET i = $for_iterN` (the IR's user-var binding) then
                        // INHERITS the loop var's provable fact, so `get(L, i)` in the
                        // body can elide. `i` is immutable, so the alias stays valid.
                        self.provable_index_locals.remove(name);
                        self.provable_index_locals.retain(|_, (l, _)| l != name);
                        if let Some(NirValue::Local(src)) = value {
                            if let Some(fact) = self.provable_index_locals.get(src).cloned() {
                                self.provable_index_locals.insert(name.clone(), fact);
                            }
                        }
                        // A `MATCH` variant binding (`UnionExtract`) is an alias
                        // into the matched union's inlined variant block: the union
                        // owns the data and frees it as one block on its own drop,
                        // so the binding is neither deep-copied nor freed here.
                        let aliases_union_variant =
                            matches!(value, Some(NirValue::UnionExtract { .. }));
                        // A thread-boundary result (`thread::receive`/`waitFor`/…)
                        // is owned by the thread runtime / worker arena, not this
                        // scope, so it is neither zero-initialized nor freed here.
                        let runtime_managed =
                            value.as_ref().is_some_and(Self::value_is_runtime_managed);
                        // This binding owns a freeable flat block that scope-drop
                        // must free (plan-02 Phase 8). A by-ref capture slot is
                        // not owned here — the parent binding remains the freer.
                        // A small-vector binding promoted to its lanes owns no
                        // arena block (plan-01-vector), so it is neither zero-init'd
                        // nor freed at scope-drop.
                        let promote_vector = self.promotable_vector_locals.contains(name);
                        // plan-86 E: a read-only `get`-borrow binding (`e = get(L,i)`
                        // used only as a MATCH scrutinee over an immutable container
                        // `L`) aliases `L`'s inline element for a freeable-flat,
                        // non-String type — so it is neither copied nor freed here
                        // (the container owns the element). String `get` returns an
                        // OWNED fresh block, so it is excluded and keeps its copy+free.
                        let is_borrow_get = self.borrow_get_locals.contains(name)
                            && self.is_freeable_flat_value(type_)
                            && !matches!(type_, ParameterType::String);
                        let owns_freeable_value = !aliases_union_variant
                            && !by_ref_capture_slot
                            && !runtime_managed
                            && !promote_vector
                            && !is_borrow_get
                            && self.is_freeable_flat_value(type_);
                        // This binding will register a resource-close cleanup (a
                        // plain resource or a resource union) rather than a flat-value
                        // free. Its slot faces the same not-yet-initialized hazard as
                        // an owned flat value: a fallible initializer that traps (or
                        // an error jumping past the bind to the function `TRAP`
                        // handler) leaves the slot unwritten, and closing a garbage
                        // handle SIGSEGVs (bug-246). The conditions here mirror the
                        // cleanup-registration branches below exactly.
                        let floats_to_collection = matches!(
                            self.resource_owners.get(name),
                            Some(crate::ir::resource_escape::ResOwner::Float(_))
                        );
                        // bug-375: a bind that merely aliases an already-live
                        // resource registers no cleanup below, so it owns no
                        // slot to zero here either.
                        let aliases_live_resource = value
                            .as_ref()
                            .is_some_and(Self::value_aliases_live_resource);
                        let owns_resource_slot = !Self::is_thread_type(&type_)
                            && !aliases_union_variant
                            && !by_ref_capture_slot
                            && !floats_to_collection
                            && !aliases_live_resource
                            && (self.resource_cleanup_symbol(type_).is_some()
                                || self.resource_union_cleanup(type_).is_some());
                        // plan-77 M6: a closure binding is non-escaping when its
                        // name is never used as a value (only a direct invoke
                        // target) — see `collect_value_used_locals`. Such a closure
                        // is dead at scope end; its object/env/flat-captures are
                        // freed by the closure branch below. The env/object frees
                        // share the same not-yet-initialized-slot hazard as an owned
                        // flat value, so the slot must be zeroed and registered too.
                        let is_non_escaping_closure = !aliases_union_variant
                            && !by_ref_capture_slot
                            && !self.value_used_locals.contains(name)
                            && value
                                .as_ref()
                                .is_some_and(|v| matches!(v, NirValue::Closure { .. }));
                        // A `Thread` binding registers a `thread.drop` cleanup below
                        // (the first branch of the chain, taken unconditionally on the
                        // type), so its slot carries the identical hazard: a
                        // `thread::start` that raises on a bad queue limit -- or any
                        // error routed to the function `TRAP` handler past a bind that
                        // never ran -- leaves the slot unwritten, and `thread.drop`
                        // dereferences the handle's outbound-queue pointer (bug-469).
                        // Mirrors the cleanup-registration condition exactly.
                        let owns_thread_slot = Self::is_thread_type(&type_);
                        // Zero the slot before a (possibly fallible) initializer
                        // runs. If the initializer traps before storing, the slot
                        // stays null and the scope-drop free/close skips it instead of
                        // touching an uninitialized pointer.
                        if owns_freeable_value
                            || owns_resource_slot
                            || owns_thread_slot
                            || is_non_escaping_closure
                        {
                            self.emit(abi::move_immediate(&zero_slot, "Integer", "0"));
                            self.emit(abi::store_u64(
                                &zero_slot,
                                abi::stack_pointer(),
                                stack_offset,
                            ));
                        }
                        // Record the resource slot for prologue zero-init too: an
                        // error can jump to the function `TRAP` handler past a bind
                        // that never ran, and the handler's null-guarded close must
                        // see 0 rather than stack garbage (bug-246). `owned_value_slots`
                        // is consumed only as the entry-zeroing list. A `Thread` slot
                        // rides the same list for the same reason (bug-469).
                        if owns_resource_slot || owns_thread_slot {
                            self.owned_value_slots.push(stack_offset);
                        }
                        if let Some(value) = value {
                            // A promoted small-vector binding keeps its lanes in
                            // registers with no arena block: lower the (native)
                            // initializer and record its lanes; reads reconstruct a
                            // native view from them (plan-01-vector). Lowered
                            // *without* the owning-copy/materialize path so the
                            // native survives. If the value unexpectedly did not
                            // lower to a native (the escape analysis over-approved),
                            // fall back to storing it as an ordinary block.
                            if promote_vector {
                                let result = self.lower_value(value)?;
                                if let Some(lanes) = self.vector_native_lanes(&result) {
                                    self.promoted_vector_locals
                                        .insert(name.clone(), (type_.clone(), lanes));
                                } else {
                                    let block = self.vector_value_as_block(result)?;
                                    self.claim_pending_temp(&block);
                                    self.store_value_at(&block, abi::stack_pointer(), stack_offset);
                                    self.active_cleanups.push(ActiveCleanup::OwnedValue(
                                        OwnedValueCleanup {
                                            type_: type_.clone(),
                                            stack_offset,
                                            closure_captures: None,
                                        },
                                    ));
                                    self.owned_value_slots.push(stack_offset);
                                }
                                self.reset_string_capacity_shadow(name);
                                return Ok(());
                            }
                            // plan-64-I: if this binds an inline-conversion
                            // `CallResult` whose trapped error is provably unused,
                            // flag its lowering so the error path emits only a tag
                            // (no ErrorLoc / flat `Error` block). Reset immediately
                            // after — the flag is scoped to this one initializer.
                            let discard_error = matches!(value, NirValue::CallResult { .. })
                                && self.trap_discard_error_results.contains(name);
                            if discard_error {
                                self.raw_result_discard_error = true;
                            }
                            // Deep-copy aliasing sources so this binding owns an
                            // independent flat block (plan-02 Phase 8); an aliased
                            // variant binding or by-ref capture slot aliases its
                            // source deliberately and is stored without copying.
                            // plan-86 E: a borrow binding lowers its `get` WITHOUT the
                            // owning copy, and the `borrow_get_result` flag makes
                            // `materialize_owned_element` return the aliasing element
                            // pointer instead of copying it. Scoped to this one
                            // initializer.
                            let result = if aliases_union_variant || by_ref_capture_slot {
                                self.lower_value(value)?
                            } else if is_borrow_get {
                                self.borrow_get_result = true;
                                let r = self.lower_value(value);
                                self.borrow_get_result = false;
                                r?
                            } else {
                                self.lower_value_owned(value)?
                            };
                            self.raw_result_discard_error = false;
                            // Observation boundary: a `Float` becoming a named
                            // binding must be finite (plan-17).
                            self.observe_float(value, &result)?;
                            // A `d`-native `Float` stores via `str d` (plan-01
                            // float-dnative); every other value via `str x`.
                            self.store_value_at(&result, abi::stack_pointer(), stack_offset);
                        } else {
                            let result = self.lower_default_value(type_)?;
                            // The default empty `String` is static rodata; copy it
                            // into the arena so this binding owns an arena block its
                            // scope-drop free can reclaim (collections/records
                            // default to arena allocations already).
                            let location = if result.type_ == ParameterType::String {
                                Operand::from(
                                    self.copy_flat_block(&ParameterType::String, &result.location)?
                                        .render(),
                                )
                            } else {
                                result.location
                            };
                            self.emit(abi::store_u64(
                                &location,
                                abi::stack_pointer(),
                                stack_offset,
                            ));
                        }
                        // A (re)bind installs a fresh tight buffer; clear any stale
                        // self-append capacity headroom recorded for this name.
                        self.reset_string_capacity_shadow(name);
                        // A collection that owns resources floated up from inner
                        // blocks (§15.6) gets a runtime owned-list anchored at
                        // this scope; it is drained on every exit path.
                        if self.owner_containers.contains(name) {
                            self.setup_owned_list(name, type_)?;
                        } else if Self::is_res_marked_resource_collection(&type_)
                            && matches!(
                                value,
                                Some(NirValue::Call { .. } | NirValue::CallResult { .. })
                            )
                        {
                            // A `List OF RES File` bound from a call adopts the
                            // resources transferred out by the callee: this scope
                            // owns them and closes each once at exit (§15.6).
                            self.setup_owned_list(name, type_)?;
                            if let Some(element_type) =
                                crate::codegen::engine::types::typed_list_element_type(&type_)
                                    .cloned()
                            {
                                self.emit_owned_list_seed_from_collection(
                                    name,
                                    stack_offset,
                                    &element_type,
                                )?;
                            }
                        }
                        // Where this binding's close obligation lives (§15.6).
                        let resource_owner = self
                            .resource_owners
                            .get(name)
                            .cloned()
                            .unwrap_or(crate::ir::resource_escape::ResOwner::Local);
                        if Self::is_thread_type(&type_) {
                            self.active_cleanups
                                .push(ActiveCleanup::Thread(ThreadCleanup {
                                    name: name.clone(),
                                    symbol: Self::thread_drop_symbol(),
                                }));
                        } else if aliases_union_variant || by_ref_capture_slot {
                            // Non-owning — no cleanup (the parent binding frees it).
                        } else if let crate::ir::resource_escape::ResOwner::Float(collection) =
                            &resource_owner
                        {
                            // Ownership floated to an outer collection's scope:
                            // register the record in that owned-list. This binding
                            // is now an alias and registers no static cleanup.
                            let collection = collection.clone();
                            self.emit_owned_list_push(&collection, stack_offset)?;
                            // The floated record is aliased from a producer temp
                            // (`local $src`) when a fallible producer used an inline
                            // `TRAP` — the desugar's `$trap_valN` is not a `RES`
                            // name, so the escape analysis leaves it owning a close
                            // at *its* (inner loop) scope. Ownership has floated to
                            // `collection`; drop the source's close so the record is
                            // closed once by the owned-list drain, not prematurely at
                            // each loop iteration (a floated handle would otherwise be
                            // closed while the collection still held it).
                            if let Some(NirValue::Local(src)) = value {
                                self.deactivate_resource_cleanup(src);
                            }
                        } else if aliases_live_resource
                            && (self.resource_cleanup_symbol(type_).is_some()
                                || self.resource_union_cleanup(type_).is_some())
                        {
                            // Non-owning — this bind only copies a pointer to a
                            // resource the owning scope already closes exactly
                            // once (§15.6). Registering a cleanup here released
                            // the caller's handle at this scope's exit (bug-375).
                            // Gated on the resource-typed cleanups alone so a
                            // plain aliasing bind of a flat value still takes the
                            // `owns_freeable_value` branch below and is freed.
                        } else if let Some(symbol) = self.resource_cleanup_symbol(type_) {
                            self.active_cleanups
                                .push(ActiveCleanup::Resource(ResourceCleanup {
                                    name: name.clone(),
                                    symbol,
                                    state_type: type_.state(),
                                    has_io_buffers: Self::resource_uses_io_buffers(type_),
                                }));
                        } else if let Some(variants) = self.resource_union_cleanup(type_) {
                            // A resource union drops by dispatching on its tag to
                            // the active variant's registered close op, then frees
                            // the active variant record's uniform STATE (plan-74).
                            self.active_cleanups.push(ActiveCleanup::ResourceUnion(
                                ResourceUnionCleanup {
                                    name: name.clone(),
                                    variants,
                                    state_type: type_.state(),
                                },
                            ));
                        } else if owns_freeable_value {
                            // An owned, non-escaping flat value (plan-01 Phase 5 /
                            // plan-02 Phase 8): a single `arena_free` of its block
                            // reclaims everything at scope-drop. Copy-insertion
                            // (`lower_value_owned`) guarantees this block is
                            // unaliased, so the free is sound and once-only.
                            self.active_cleanups.push(ActiveCleanup::OwnedValue(
                                OwnedValueCleanup {
                                    type_: type_.clone(),
                                    stack_offset,
                                    closure_captures: None,
                                },
                            ));
                            self.owned_value_slots.push(stack_offset);
                        } else if is_non_escaping_closure {
                            // plan-77 M6: free the closure object, its env, and each
                            // freeable-flat capture (all deep-copied, so unaliased)
                            // at scope-drop. The captures' free types are resolved
                            // now (Local captures from `self.locals`); a capture that
                            // is a by-value scalar/float or of unknown type is left
                            // (a safe leak, never a wild free).
                            if let Some(NirValue::Closure { captures, .. }) = value {
                                let capture_types = captures
                                    .iter()
                                    .map(|capture| self.capture_free_type(capture))
                                    .collect();
                                self.active_cleanups.push(ActiveCleanup::OwnedValue(
                                    OwnedValueCleanup {
                                        type_: type_.clone(),
                                        stack_offset,
                                        closure_captures: Some(capture_types),
                                    },
                                ));
                                self.owned_value_slots.push(stack_offset);
                            }
                        }
                        // Default-initialize a `RES` binding's `STATE` payload.
                        // The owning binding allocates the state record on first
                        // bind; a moved/returned resource that already carries a
                        // state keeps it (the slot is non-null).
                        if let Some(state_type) = type_.state() {
                            self.emit_resource_state_init(stack_offset, &state_type, type_)?;
                        }
                    }
                    NirOp::StoreGlobal { name, type_, value } => {
                        let global = self.global_value(name)?;
                        let value_type = if crate::codegen::engine::types::is_unset_type(type_) {
                            global.type_.clone()
                        } else {
                            type_.clone()
                        };
                        // A global outlives every scope, so it must own its value
                        // independently: deep-copy an aliasing source so freeing a
                        // local never dangles the global (plan-02 Phase 8).
                        let result = if let Some(value) = value {
                            self.lower_value_owned(value)?
                        } else {
                            self.lower_default_value(&value_type)?
                        };
                        // Observation boundary: a `Float` global must be finite
                        // (plan-17).
                        if let Some(value) = value {
                            self.observe_float(value, &result)?;
                        }
                        // Free the global's previous freeable-flat block before
                        // overwriting the slot (bug-47, the bug-01 class). A global
                        // carries no `OwnedValue` scope-drop cleanup, and unlike the
                        // local `NirOp::Assign` path (:351-385) `StoreGlobal` had no
                        // old-block free, so a freeable-flat global reassigned in a
                        // loop (`g = collections::filter(g, cb)`) leaked one block per
                        // iteration. The old pointer is snapshotted from the global
                        // into a stack slot and freed via `emit_owned_value_drop`
                        // (null-guarded, so the first store over a zero-initialized
                        // global is a no-op; sized from the type incl. a map's bucket
                        // region). The freshly computed value is spilled across the
                        // `arena_free` (which trashes all caller-saved registers) and
                        // the global address is re-derived afterward (its base, the
                        // arena-state register x19, is callee-saved and survives the
                        // call). `lower_value_owned` deep-copied any aliasing source,
                        // so the new block never aliases the freed one — the free is
                        // sound and once-only.
                        if self.is_freeable_flat_value(&value_type) {
                            let new_slot = self.allocate_stack_object("store_global_new", 8);
                            self.emit(abi::store_u64(
                                &result.location,
                                abi::stack_pointer(),
                                new_slot,
                            ));
                            let old_slot = self.allocate_stack_object("store_global_old", 8);
                            let address = self.load_global_address(name)?;
                            let old_ptr = self.allocate_register();
                            self.emit(abi::load_u64(&old_ptr, &address, 0));
                            self.emit(abi::store_u64(&old_ptr, abi::stack_pointer(), old_slot));
                            self.emit_owned_value_drop(&OwnedValueCleanup {
                                type_: value_type.clone(),
                                stack_offset: old_slot,
                                closure_captures: None,
                            })?;
                            let new_ptr = self.allocate_register();
                            self.emit(abi::load_u64(&new_ptr, abi::stack_pointer(), new_slot));
                            let stored = ValueResult {
                                origin: None,
                                type_: result.type_.clone(),
                                location: Operand::from(new_ptr.render()),
                                text: String::new(),
                            };
                            let address = self.load_global_address(name)?;
                            self.store_value_at(&stored, &address, 0);
                        } else {
                            let address = self.load_global_address(name)?;
                            self.store_value_at(&result, &address, 0);
                        }
                    }
                    NirOp::Assign { name, value } => {
                        // plan-86 G1: reassigning `name` invalidates any `len_of_local`
                        // or provable-index fact keyed on it, and any fact naming it
                        // as the list `L`.
                        self.len_of_local.remove(name);
                        self.len_of_local.retain(|_, (l, _)| l != name);
                        self.provable_index_locals.remove(name);
                        self.provable_index_locals.retain(|_, (l, _)| l != name);
                        // A loop-promoted float local is updated in its FP
                        // register, not its slot (plan-03 Stage D part 2).
                        if let Some(d) = self.promoted_float_locals.get(name).cloned() {
                            let result = self.lower_value(value)?;
                            self.update_promoted_float(&d, &result);
                            // Observation boundary: the promoted accumulator's
                            // `d`-register holds the named local's value after
                            // the update, so check it there — the FP-domain
                            // variant keeps the store-to-slot peephole-foldable
                            // (plan-16/plan-17).
                            self.observe_promoted_float(value, &d)?;
                            return Ok(());
                        }
                        let (stack_offset, by_ref) = {
                            let local = self.locals.get(name).ok_or_else(|| {
                                format!("native code assignment unknown local '{name}'")
                            })?;
                            (local.stack_offset, local.by_ref)
                        };
                        // `name = collections::append(name, item)` on a uniquely
                        // owned `MUT` list mutates the live buffer in place
                        // (plan-01 §4.2): the helper updates the slot, so skip the
                        // general reassignment path entirely.
                        if !self.try_inplace_append_assign(name, value, stack_offset, by_ref)?
                            && !self.try_inplace_bulk_append_assign(
                                name,
                                value,
                                stack_offset,
                                by_ref,
                            )?
                            && !self.try_inplace_set_add_assign(
                                name,
                                value,
                                stack_offset,
                                by_ref,
                            )?
                            && !self.try_inplace_set_assign(name, value, stack_offset, by_ref)?
                            && !self.try_inplace_remove_key_assign(
                                name,
                                value,
                                stack_offset,
                                by_ref,
                            )?
                            && !self.try_inplace_prepend_assign(
                                name,
                                value,
                                stack_offset,
                                by_ref,
                            )?
                            // plan-121-B: `removeAt` and Set `remove` had no arm in
                            // any container, so every call allocated a fresh block
                            // and copied the whole collection. Order within this
                            // chain is immaterial — the arms match disjoint builtin
                            // names — so they are appended rather than interleaved.
                            && !self.try_inplace_remove_at_assign(
                                name,
                                value,
                                stack_offset,
                                by_ref,
                            )?
                            && !self.try_inplace_insert_assign(
                                name,
                                value,
                                stack_offset,
                                by_ref,
                            )?
                            && !self.try_inplace_set_remove_assign(
                                name,
                                value,
                                stack_offset,
                                by_ref,
                            )?
                            && !self.try_inplace_concat_assign(name, value, stack_offset, by_ref)?
                            && !self.try_inplace_record_field_append(
                                name,
                                value,
                                stack_offset,
                                by_ref,
                            )?
                            && !self.try_inplace_record_field_remove_key_assign(
                                name,
                                value,
                                stack_offset,
                                by_ref,
                            )?
                            && !self.try_inplace_record_field_remove_at_assign(
                                name,
                                value,
                                stack_offset,
                                by_ref,
                            )?
                            && !self.try_inplace_record_field_set_remove_assign(
                                name,
                                value,
                                stack_offset,
                                by_ref,
                            )?
                            && !self.try_inplace_record_field_set_add_assign(
                                name,
                                value,
                                stack_offset,
                                by_ref,
                            )?
                            && !self.try_inplace_record_field_set_assign(
                                name,
                                value,
                                stack_offset,
                                by_ref,
                            )?
                            && !self.try_inplace_record_field_insert_assign(
                                name,
                                value,
                                stack_offset,
                                by_ref,
                            )?
                            && !self.try_inplace_record_field_prepend_assign(
                                name,
                                value,
                                stack_offset,
                                by_ref,
                            )?
                        {
                            // Reassignment installs a fresh independent block; the old
                            // block remains owned by this binding's scope-drop free
                            // (the slot is overwritten with the new owner). Deep-copy
                            // an aliasing source so the binding stays unaliased.
                            let result = self.lower_value_owned(value)?;
                            // Observation boundary: a `Float` reassignment must
                            // be finite (plan-17).
                            self.observe_float(value, &result)?;
                            let assign_slot = if Self::is_thread_type(&result.type_) {
                                let slot = self.allocate_stack_object("thread_assign_value", 8);
                                self.emit(abi::store_u64(
                                    &result.location,
                                    abi::stack_pointer(),
                                    slot,
                                ));
                                self.emit_thread_cleanup_for_name(name)?;
                                Some(slot)
                            } else if let Some(symbol) = self.resource_cleanup_symbol(&result.type_)
                            {
                                let slot = self.allocate_stack_object("resource_assign_value", 8);
                                self.emit(abi::store_u64(
                                    &result.location,
                                    abi::stack_pointer(),
                                    slot,
                                ));
                                let cleanup = ResourceCleanup {
                                    name: name.clone(),
                                    symbol,
                                    state_type: result.type_.state(),
                                    has_io_buffers: Self::resource_uses_io_buffers(&result.type_),
                                };
                                self.emit_resource_cleanup_call(&cleanup)?;
                                Some(slot)
                            } else if !by_ref
                                && self.is_freeable_flat_value(&result.type_)
                                && !self.for_each_iterable_locals.iter().any(|n| n == name)
                                // bug-430: a live `FOR EACH x IN name.field` holds an
                                // alias into this record's block; freeing the block
                                // mid-loop is a use-after-free (this rebuild is the
                                // fallback the in-place record grow declines to for
                                // exactly this reason). Leak the old block instead,
                                // matching the `for_each_iterable_locals` case above.
                                && !self
                                    .for_each_iterable_record_fields
                                    .iter()
                                    .any(|(base, _)| base == name)
                            {
                                // Free the binding's previous block before
                                // overwriting the slot. A reassignment installs a
                                // fresh independent block; without this the old
                                // block leaks (bug-01) — e.g. the value-semantic
                                // `list = collections::append(list, complexItem)`
                                // fallback (when the item isn't a simple element
                                // and the in-place path declines) or a plain
                                // `list = otherList`. The new value is spilled
                                // across the `arena_free` (which trashes the
                                // caller-saved registers) and reloaded, mirroring
                                // the thread/resource paths. The slot still holds
                                // the old block here, and the freshly computed new
                                // value never aliases it (`lower_value_owned`
                                // deep-copies aliasing sources; a call result is a
                                // fresh block), so the free is sound and once-only.
                                // A live `FOR EACH` iterable is excluded: its
                                // iterator still reads the old block, so freeing it
                                // mid-loop would be a use-after-free (mirrors the
                                // `try_inplace_set_assign` guard). That old block
                                // leaks, but the pattern is rare.
                                let slot = self.allocate_stack_object("reassign_value", 8);
                                self.emit(abi::store_u64(
                                    &result.location,
                                    abi::stack_pointer(),
                                    slot,
                                ));
                                self.emit_owned_value_drop(&OwnedValueCleanup {
                                    type_: result.type_.clone(),
                                    stack_offset,
                                    closure_captures: None,
                                })?;
                                Some(slot)
                            } else {
                                None
                            };
                            // What to store: the value itself, or a GPR reloaded
                            // from the thread/resource cleanup slot (never a
                            // `Float`, so always GP-native there). A `d`-native
                            // `Float` is stored via `str d` (plan-01
                            // float-dnative).
                            let store_value = if let Some(slot) = assign_slot {
                                let register = self.allocate_register();
                                self.emit(abi::load_u64(&register, abi::stack_pointer(), slot));
                                ValueResult {
                                    origin: None,
                                    type_: result.type_.clone(),
                                    location: Operand::from(register.render()),
                                    text: String::new(),
                                }
                            } else {
                                result.clone()
                            };
                            if by_ref {
                                // A reference local (non-escaping `MUT` by-ref capture): write
                                // through the slot pointer so the live parent binding is
                                // updated, not a local copy.
                                let slot_pointer = self.allocate_register();
                                self.emit(abi::load_u64(
                                    &slot_pointer,
                                    abi::stack_pointer(),
                                    stack_offset,
                                ));
                                self.store_value_at(&store_value, &slot_pointer.render(), 0);
                            } else {
                                self.store_value_at(
                                    &store_value,
                                    abi::stack_pointer(),
                                    stack_offset,
                                );
                            }
                            // A reference local never folds to a constant (see Bind).
                            let constant = if by_ref {
                                None
                            } else {
                                self.local_constant_value(value)
                            };
                            if let Some(local) = self.locals.get_mut(name) {
                                local.constant = constant;
                            }
                            // A non-self-append reassignment installs a fresh tight
                            // buffer; clear any stale self-append capacity headroom.
                            if !by_ref {
                                self.reset_string_capacity_shadow(name);
                            }
                        }
                    }
                    NirOp::StateAssign { resource, value } => {
                        // bug-424 Layer 1: a scalar `s.state.field = v` mutates the
                        // existing STATE block in place; only a whole-record replace
                        // (or a not-yet-in-place inlined field) falls through here.
                        if self.try_inplace_state_scalar_assign(resource, value)? {
                            return Ok(());
                        }
                        // bug-430 Layer 2: a collection field that is the last
                        // inlined field is mutated in place inside the existing
                        // STATE block instead of rebuilding the record. plan-121-D
                        // Phase 1 put the operation dispatch behind one call so
                        // Phase 2's arms are additive; `append` (amortized O(1)
                        // grow) is currently the only operation dispatched.
                        if self.try_inplace_state_collection_assign(resource, value)? {
                            return Ok(());
                        }
                        // Replace the resource's `STATE` payload: store the new
                        // record pointer into the resource record's state slot.
                        // The resource value is itself a pointer, so the update
                        // is visible to the owner and every other pointer to it.
                        let local = self.locals.get(resource).ok_or_else(|| {
                            format!("native code state assignment unknown local '{resource}'")
                        })?;
                        let stack_offset = local.stack_offset;
                        // A resource union value is a `{tag, record-ptr}` block; the
                        // STATE lives in the active variant's record at `+8`
                        // (plan-74). Concrete resources address their record directly.
                        let resource_type = local.type_.clone();
                        let result = self.lower_value(value)?;
                        // A register-native vector STATE payload materializes to its
                        // block here (identity otherwise; plan-01-vector).
                        let result = self.vector_value_as_block(result)?;
                        // The raw block pointer is stored into the resource's STATE
                        // slot (below), so this store takes ownership — claim the
                        // temp so the statement-scope free never reclaims a block the
                        // resource still points at (plan-25).
                        self.claim_pending_temp(&result);
                        // Observation boundary: a `Float` resource STATE payload
                        // must be finite (plan-17).
                        self.observe_float(value, &result)?;
                        let value_slot = self.allocate_stack_object("state_assign_value", 8);
                        self.store_value_at(&result, abi::stack_pointer(), value_slot);
                        let block = self.allocate_register();
                        self.emit(abi::load_u64(&block, abi::stack_pointer(), stack_offset));
                        let ptr = self.emit_resource_record_ptr(&block, &resource_type)?;
                        let val = self.allocate_register();
                        self.emit(abi::load_u64(&val, abi::stack_pointer(), value_slot));
                        self.emit(abi::store_u64(&val, &ptr, RESOURCE_OFFSET_STATE));
                    }
                    NirOp::Eval { value } => {
                        self.lower_value(value)?;
                    }
                    NirOp::Return { value } => {
                        self.emit_return_exit(value.as_ref())?;
                    }
                    NirOp::ExitLoop { kind } => {
                        let target = self
                            .loop_stack
                            .iter()
                            .rev()
                            .find(|labels| labels.kind == *kind)
                            .cloned()
                            .ok_or_else(|| "native code EXIT has no matching loop".to_string())?;
                        self.emit_cleanup_branch_to_depth(
                            &target.exit_label,
                            target.cleanup_depth,
                        )?;
                    }
                    NirOp::ContinueLoop { kind } => {
                        let target = self
                            .loop_stack
                            .iter()
                            .rev()
                            .find(|labels| labels.kind == *kind)
                            .cloned()
                            .ok_or_else(|| {
                                "native code CONTINUE has no matching loop".to_string()
                            })?;
                        self.emit_cleanup_branch_to_depth(
                            &target.continue_label,
                            target.cleanup_depth,
                        )?;
                    }
                    NirOp::ExitProgram { code } => {
                        self.emit_program_exit_value(code)?;
                    }
                    NirOp::Fail { error } => {
                        self.emit_error_value_exit(error, self.error_exit_destination())?;
                    }
                    NirOp::If {
                        condition,
                        then_body,
                        else_body,
                    } => {
                        // plan-39 I1: derive the fall-through lower bound from the
                        // raw condition before it is lowered/shadowed.
                        let guard = self.guard_lower_bound(condition);
                        let bounds_before_if = self.integer_lower_bounds.clone();
                        let condition = self.lower_value(condition)?;
                        let else_label = self.label("if_else");
                        let end_label = self.label("if_end");
                        let constants_before_if = self.local_constants();
                        self.emit(abi::compare_immediate(&condition.location, "0"));
                        self.emit(abi::branch_eq(&else_label).field("reason", "ifFalse"));
                        self.lower_ops(then_body)?;
                        let then_terminal = self.current_block_returns();
                        if !then_terminal {
                            self.emit(abi::branch(&end_label));
                        }
                        self.emit(abi::label(&else_label));
                        self.restore_local_constants(&constants_before_if);
                        self.integer_lower_bounds = bounds_before_if;
                        self.lower_ops(else_body)?;
                        self.emit(abi::label(&end_label));
                        self.clear_local_constants();
                        // The merged path can inherit no bound (either branch may
                        // have reassigned), so clear conservatively — then, if the
                        // then-branch always exits and the else is empty, the
                        // guard condition is provably false here: record its bound.
                        self.integer_lower_bounds.clear();
                        if then_terminal && else_body.is_empty() {
                            if let Some((name, bound)) = guard {
                                self.integer_lower_bounds.insert(name, bound);
                            }
                        }
                    }
                    NirOp::Match { value, cases } => {
                        let matched = self.lower_value(value)?;
                        let matched_slot = self.allocate_stack_object("match_value", 8);
                        self.emit(abi::store_u64(
                            &matched.location,
                            abi::stack_pointer(),
                            matched_slot,
                        ));
                        let end_label = self.label("match_end");
                        for case in cases {
                            let matched_register = self.allocate_register();
                            self.emit(abi::load_u64(
                                &matched_register,
                                abi::stack_pointer(),
                                matched_slot,
                            ));
                            let case_matched = ValueResult {
                                origin: None,
                                type_: matched.type_.clone(),
                                location: Operand::from(matched_register.render()),
                                text: matched.text.clone(),
                            };
                            let next_label = self.label("match_next");
                            match &case.pattern {
                                NirMatchPattern::Else => {}
                                NirMatchPattern::Value(pattern) => {
                                    let case_label = self.label("match_case");
                                    self.lower_match_compare(&case_matched, pattern, &case_label)?;
                                    self.emit(abi::branch(&next_label));
                                    self.emit(abi::label(&case_label));
                                }
                                NirMatchPattern::OneOf(patterns) => {
                                    let case_label = self.label("match_case");
                                    for pattern in patterns {
                                        self.lower_match_compare(
                                            &case_matched,
                                            pattern,
                                            &case_label,
                                        )?;
                                    }
                                    self.emit(abi::branch(&next_label));
                                    self.emit(abi::label(&case_label));
                                }
                            }
                            let constants_before_case = self.local_constants();
                            let mut case_locals = self.locals.clone();
                            let mut body_index = 0;
                            while let Some(NirOp::Bind {
                                name,
                                type_,
                                value: Some(NirValue::UnionExtract { .. }),
                                ..
                            }) = case.body.get(body_index)
                            {
                                let bind = &case.body[body_index..body_index + 1];
                                self.lower_ops(bind)?;
                                if let Some(local) = self.locals.get(name).cloned() {
                                    case_locals.insert(
                                        name.clone(),
                                        LocalValue {
                                            type_: type_.clone(),
                                            stack_offset: local.stack_offset,
                                            constant: local.constant,
                                            by_ref: local.by_ref,
                                        },
                                    );
                                }
                                body_index += 1;
                            }
                            if let Some(guard) = &case.guard {
                                let saved_locals = self.locals.clone();
                                self.locals = case_locals.clone();
                                let guard_value = self.lower_value(guard)?;
                                self.emit(abi::compare_immediate(&guard_value.location, "0"));
                                self.emit(
                                    abi::branch_eq(&next_label).field("reason", "matchGuardFalse"),
                                );
                                self.locals = saved_locals;
                            }
                            self.lower_ops(&case.body[body_index..])?;
                            if !self.current_block_returns() {
                                self.emit(abi::branch(&end_label));
                            }
                            self.restore_local_constants(&constants_before_case);
                            self.emit(abi::label(&next_label));
                        }
                        self.emit(abi::label(&end_label));
                        self.clear_local_constants();
                    }
                    NirOp::While {
                        kind,
                        condition,
                        body,
                    } => {
                        // plan-39 I1 (upper side): a `WHILE local < S` body runs only
                        // when `local < S`, so `local` is strictly below an i64 there
                        // and `local + 1 <= S <= i64::MAX` cannot overflow. Capture
                        // the guarded local from the raw condition before it is
                        // lowered/shadowed, and mark it for the body below.
                        let strict_upper_name = match condition {
                            NirValue::Binary { op, left, .. } if *op == BinaryOp::Less => {
                                match left.as_ref() {
                                    NirValue::Local(n) => Some(n.clone()),
                                    _ => None,
                                }
                            }
                            _ => None,
                        };
                        let promoted = self.begin_loop_promotion(body, None)?;
                        let loop_label = self.label("while_loop");
                        let end_label = self.label("while_end");
                        self.emit(abi::label(&loop_label));
                        // The back-edge jumps to `loop_label` above the condition, so
                        // the condition (and body) is re-tested each iteration from
                        // this one emitted comparison. Constants known at loop entry
                        // (e.g. a `MUT` local's literal initializer) must not fold reads
                        // in the condition or body — they go stale once the body
                        // reassigns them. A string-producing condition like
                        // `WHILE toString(c) <> "3"` would otherwise freeze the fold to
                        // `c`'s entry value and loop forever (bug-57). Mirrors the
                        // `clear_local_constants()` the `DoUntil` path runs before its
                        // body+condition.
                        self.clear_local_constants();
                        let condition = self.lower_value(condition)?;
                        self.emit(abi::compare_immediate(&condition.location, "0"));
                        self.emit(abi::branch_eq(&end_label));
                        self.loop_stack.push(LoopLabels {
                            kind: *kind,
                            continue_label: loop_label.clone(),
                            exit_label: end_label.clone(),
                            cleanup_depth: self.active_cleanups.len(),
                        });
                        if let Some(ref name) = strict_upper_name {
                            self.integer_strict_upper.insert(name.clone());
                        }
                        self.lower_loop_body(body)?;
                        self.loop_stack.pop();
                        self.emit(abi::branch(&loop_label));
                        self.emit(abi::label(&end_label));
                        self.clear_local_constants();
                        self.end_loop_promotion(promoted)?;
                    }
                    NirOp::For {
                        name,
                        type_,
                        start,
                        end,
                        step,
                        body,
                        loc,
                    } => {
                        self.lower_numeric_for(name, type_, start, end, step, body, *loc)?;
                    }
                    NirOp::DoUntil { body, condition } => {
                        let promoted = self.begin_loop_promotion(body, None)?;
                        let loop_label = self.label("do_loop");
                        let condition_label = self.label("do_until");
                        let end_label = self.label("do_end");
                        self.emit(abi::label(&loop_label));
                        // The back-edge jumps to `loop_label` above the body, so
                        // constants known at loop entry (e.g. a `MUT` local's literal
                        // initializer) must not fold reads inside the body — they go
                        // stale once the body reassigns them on later iterations.
                        // Matches the `clear_local_constants()` the `While` path runs
                        // before its body.
                        self.clear_local_constants();
                        self.loop_stack.push(LoopLabels {
                            kind: crate::ast::LoopKind::Do,
                            continue_label: condition_label.clone(),
                            exit_label: end_label.clone(),
                            cleanup_depth: self.active_cleanups.len(),
                        });
                        self.lower_loop_body(body)?;
                        self.loop_stack.pop();
                        self.emit(abi::label(&condition_label));
                        let condition = self.lower_value(condition)?;
                        self.emit(abi::compare_immediate(&condition.location, "0"));
                        self.emit(abi::branch_eq(&loop_label));
                        self.emit(abi::label(&end_label));
                        self.clear_local_constants();
                        self.end_loop_promotion(promoted)?;
                    }
                    NirOp::ForEach {
                        name,
                        type_,
                        iterable,
                        body,
                    } => {
                        self.lower_for_each(name, type_, iterable, body)?;
                    }
                    NirOp::Trap { body, .. } => {
                        let (label, trap_name, trap_offset) = self
                            .trap
                            .as_ref()
                            .map(|trap| (trap.label.clone(), trap.name.clone(), trap.stack_offset))
                            .expect("trap op requires trap state");
                        // Re-pin the trap error local to its function-level slot: an
                        // inline `TRAP(e)` in the body reuses the shared name `e` and
                        // leaves `self.locals[e]` pointing at that inline handler's
                        // slot, so the function-level handler would resolve `e` (and
                        // read `e.message`) from the wrong slot — a null `Error`
                        // pointer → segfault (bug-148).
                        self.locals.insert(
                            trap_name,
                            LocalValue {
                                type_: ParameterType::named("Error"),
                                stack_offset: trap_offset,
                                constant: None,
                                by_ref: false,
                            },
                        );
                        self.emit(abi::label(&label));
                        if let Some(trap) = &mut self.trap {
                            trap.in_trap_body = true;
                        }
                        // Free the caught `Error` block on every exit from the
                        // handler (bug-151): the routed error is a fresh arena block
                        // stored into the trap slot, and it was never registered for
                        // scope-drop, so a `TRAP(e)` taken in a loop leaked one block
                        // per catch. Register `e` as the FIRST owned value of the
                        // handler body's own cleanup scope, mirroring `lower_ops`, so
                        // the body's scope-drop frees it exactly once on RETURN / FAIL
                        // / fall-through and NOT on the success path that branches over
                        // the handler (where the slot is never written). Escapes are
                        // already safe: `RETURN e` elides via `plan_returned_move`, and
                        // `FAIL e` deep-copies the error in `store_pending_error_from_value`
                        // before the cleanup frees the original.
                        let handler_scope_start = self.active_cleanups.len();
                        self.cleanup_scope_starts.push(handler_scope_start);
                        self.active_cleanups
                            .push(ActiveCleanup::OwnedValue(OwnedValueCleanup {
                                type_: ParameterType::named("Error"),
                                stack_offset: trap_offset,
                                closure_captures: None,
                            }));
                        self.owned_value_slots.push(trap_offset);
                        let handler_result = self.lower_ops_inner(body, handler_scope_start);
                        self.cleanup_scope_starts.pop();
                        handler_result?;
                        if let Some(trap) = &mut self.trap {
                            trap.in_trap_body = false;
                        }
                    }
                }
                Ok(())
            })();
            crate::codegen::engine::expansion::exit(self.instructions.len());
            result.map_err(|err| format!("{err} while lowering {}", nir_op_context(op)))?;
            // plan-39 I1: after lowering the op, invalidate range facts. A `Bind`/
            // `Assign` drops just the reassigned local's bounds (its RHS, already
            // lowered above, used the valid pre-assignment bound). A loop / Match /
            // Trap body may reassign a guarded local, so any bound it established
            // must not leak past it — drop all. Straight-line ops keep their bounds;
            // the `If` arm has already set its own post-condition bound.
            match op {
                NirOp::Bind { name, .. } | NirOp::Assign { name, .. } => {
                    self.integer_lower_bounds.remove(name);
                    self.integer_strict_upper.remove(name);
                }
                NirOp::While { .. }
                | NirOp::For { .. }
                | NirOp::ForEach { .. }
                | NirOp::DoUntil { .. }
                | NirOp::Match { .. }
                | NirOp::Trap { .. } => {
                    self.integer_lower_bounds.clear();
                    self.integer_strict_upper.clear();
                }
                _ => {}
            }
            // A control-transfer statement branches away, so any interior-temp free
            // would be unreachable and a returned/moved temp belongs to the target;
            // just forget them. Every other statement frees its interior temps here.
            if Self::op_transfers_control(op) {
                self.clear_pending_temps_to(temp_watermark);
            } else {
                self.drop_pending_temps_to(temp_watermark)?;
            }
            self.reset_temporary_registers();
        }
        let scope_returns = self.current_block_returns();
        while self.active_cleanups.len() > cleanup_scope_start {
            let cleanup = self
                .active_cleanups
                .pop()
                .expect("cleanup scope length already checked");
            if !scope_returns {
                match cleanup {
                    ActiveCleanup::Thread(cleanup) => self.emit_thread_cleanup_call(&cleanup)?,
                    ActiveCleanup::Resource(cleanup) => {
                        self.emit_resource_cleanup_call(&cleanup)?
                    }
                    ActiveCleanup::ResourceUnion(cleanup) => {
                        self.emit_resource_union_cleanup_call(&cleanup)?
                    }
                    ActiveCleanup::OwnedList(cleanup) => self.emit_owned_list_drain(&cleanup)?,
                    ActiveCleanup::OwnedValue(cleanup) => self.emit_owned_value_drop(&cleanup)?,
                }
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    /// plan-86 G1: resolve an expression to the list `L` whose `len(L)` it is —
    /// either a direct `len(L)` call (depth `None`: re-evaluated at every loop
    /// entry, so no back edge can stale it) or a local `n` bound as `LET n = len(L)`
    /// (depth `Some(d)`: the `enclosing_loop_reassigned` length when the fact was
    /// recorded — see that field for why it matters, bug-495).
    fn resolve_len_list(&self, expr: &NirValue) -> Option<(String, Option<usize>)> {
        match expr {
            NirValue::Call { target, args, .. } if target == "len" && args.len() == 1 => {
                match &args[0] {
                    NirValue::Local(l) => Some((l.clone(), None)),
                    _ => None,
                }
            }
            NirValue::Local(n) => self
                .len_of_local
                .get(n)
                .map(|(l, depth)| (l.clone(), Some(*depth))),
            _ => None,
        }
    }

    /// Lower a loop body with its reassigned-locals set pushed on
    /// `enclosing_loop_reassigned` (bug-495), so an inner `FOR`'s provable-index
    /// proof can see every reassignment this loop's back edge may run before
    /// re-entering it. Every loop kind (`WHILE`, `FOR`, `DO … UNTIL`, `FOR EACH`)
    /// lowers its body through here and nowhere else.
    fn lower_loop_body(&mut self, body: &[NirOp]) -> Result<(), String> {
        self.enclosing_loop_reassigned.push(
            crate::codegen::engine::function::collect_reassigned_locals(body),
        );
        let result = self.lower_ops(body);
        self.enclosing_loop_reassigned.pop();
        result
    }

    /// plan-86 G1: recognize `FOR i = 0 TO len(L) - k` (`k >= 1`, step 1) where `i`
    /// and `L` are provably unmodified across the whole loop body. Returns
    /// `(L, k)` — then `get/set(L, i)` is in-range (`i <= len-k < len`), and
    /// `get/set(L, i+1)` too when `k >= 2`. Conservative: any deviation returns
    /// `None` (the bounds check is kept). SOUNDNESS rests on the whole-body
    /// no-reassign proof — an unsound elision is a silent OOB.
    /// plan-86 G1: resolve a synthetic `$for_end*`/`$for_step*` local back to its
    /// bound expr; identity for anything else.
    fn resolve_for_local<'a>(&'a self, v: &'a NirValue) -> &'a NirValue {
        if let NirValue::Local(name) = v {
            if let Some(expr) = self.for_bound_expr.get(name) {
                return expr;
            }
        }
        v
    }

    fn recognize_provable_index(
        &self,
        i: &str,
        start: &NirValue,
        end: &NirValue,
        step: &NirValue,
        body: &[NirOp],
    ) -> Option<(String, i64)> {
        let is_int_const = |v: &NirValue, want: &str| matches!(v, NirValue::Const { type_, value } if matches!(type_, ParameterType::Integer) && value == want);
        // The IR desugars the bound/step into synthetic locals — resolve them.
        let start = self.resolve_for_local(start);
        let step = self.resolve_for_local(step);
        let end = self.resolve_for_local(end);
        if !is_int_const(start, "0") || !is_int_const(step, "1") {
            return None;
        }
        // end must be `<lenexpr> - k`, k >= 1.
        let NirValue::Binary {
            op, left, right, ..
        } = end
        else {
            return None;
        };
        if *op != BinaryOp::Subtract {
            return None;
        }
        let NirValue::Const { type_, value } = right.as_ref() else {
            return None;
        };
        if !matches!(type_, ParameterType::Integer) {
            return None;
        }
        let k: i64 = value.parse().ok()?;
        if k < 1 {
            return None;
        }
        let (list, fact_depth) = self.resolve_len_list(left)?;
        // Soundness: `i` and `L` (and the `n` in `n - k`, since `n = len(L)` must
        // still hold) must not be reassigned anywhere in the body.
        let reassigned = crate::codegen::engine::function::collect_reassigned_locals(body);
        if reassigned.contains(i) || reassigned.contains(&list) {
            return None;
        }
        if let NirValue::Local(n) = left.as_ref() {
            if reassigned.contains(n) {
                return None;
            }
        }
        // bug-495: a `LET n = len(L)` fact recorded BEFORE an enclosing loop was
        // entered is re-used on every back edge of that loop without `n` being
        // rebound — so if that loop's body reassigns `L` (or `n`) anywhere, even
        // AFTER this `FOR` in program order, the second entry sees a stale `n`
        // against a shorter `L` and an unchecked `get` reads out of bounds. The
        // body-only proof above cannot see that; the enclosing sets can. Loops
        // entered before the fact was recorded (index `< depth`) contain the `LET`
        // and re-run it each iteration, so they cannot stale it.
        if let Some(depth) = fact_depth {
            let n = match left.as_ref() {
                NirValue::Local(n) => Some(n.as_str()),
                _ => None,
            };
            for enclosing in self.enclosing_loop_reassigned.iter().skip(depth) {
                if enclosing.contains(&list) || n.is_some_and(|n| enclosing.contains(n)) {
                    return None;
                }
            }
        }
        Some((list, k))
    }

    /// plan-86 G1: is `index_arg` a provably-in-range index into list `list_arg`?
    /// True when `list_arg == Local(L)` and `index_arg` is `Local(i)` (needs
    /// headroom `k >= 1`) or `i + 1` (needs `k >= 2`), with
    /// `provable_index_locals[i] == (L, k)`.
    /// Whether pre-lowered `index` provably indexes pre-lowered `list` in range, so a
    /// `get`/`set` may elide the bounds check (plan-86 G1). Reads the args' source
    /// `NirValue` off `ValueResult::origin` — the collection must be a bare local `L`,
    /// and the index a bare local `i` (`k >= 1`) or `i + 1` (`k >= 2`) — against
    /// `provable_index_locals` (set by the `FOR i = 0 TO len(L)-k` recognizer). The
    /// pre-lowered `ValueResult` carries the source node the self-lowering body matched.
    pub(crate) fn is_provable_index_access(
        &self,
        list_arg: &ValueResult,
        index_arg: &ValueResult,
    ) -> bool {
        let Some(NirValue::Local(list)) = &list_arg.origin else {
            return false;
        };
        let (i, need_k) = match &index_arg.origin {
            Some(NirValue::Local(i)) => (i.as_str(), 1i64),
            Some(NirValue::Binary {
                op, left, right, ..
            }) if *op == BinaryOp::Add => match (left.as_ref(), right.as_ref()) {
                (NirValue::Local(i), NirValue::Const { type_, value })
                    if matches!(type_, ParameterType::Integer) && value == "1" =>
                {
                    (i.as_str(), 2i64)
                }
                _ => return false,
            },
            _ => return false,
        };
        matches!(self.provable_index_locals.get(i), Some((l, k)) if l == list && *k >= need_k)
    }

    pub(crate) fn lower_numeric_for(
        &mut self,
        name: &str,
        type_: &ParameterType,
        start: &NirValue,
        end: &NirValue,
        step: &NirValue,
        body: &[NirOp],
        loc: NirSourceLoc,
    ) -> Result<(), String> {
        let local_slot = self.allocate_stack_object(name, 8);
        let start_value = self.lower_value(start)?;
        // Observation boundary: a `Float` loop counter's initial value is a
        // named binding and must be finite (plan-17).
        self.observe_float(start, &start_value)?;
        self.store_value_at(&start_value, abi::stack_pointer(), local_slot);
        let previous = self.locals.insert(
            name.to_string(),
            LocalValue {
                type_: type_.clone(),
                stack_offset: local_slot,
                constant: None,
                by_ref: false,
            },
        );

        // plan-86 G1: if this is `FOR name = 0 TO len(L)-k` (step 1) with `name` and
        // `L` unmodified across the whole body, record that `name` provably indexes
        // `L` in-range for the body's `get`/`set(L, name[+1])` — cleared right after.
        let g1_provable = self.recognize_provable_index(name, start, end, step, body);
        if let Some((list, k)) = &g1_provable {
            self.provable_index_locals
                .insert(name.to_string(), (list.clone(), *k));
        }

        let promoted = self.begin_loop_promotion(body, Some(name))?;
        let loop_label = self.label("for_loop");
        let continue_label = self.label("for_continue");
        let end_label = self.label("for_end");
        self.emit(abi::label(&loop_label));
        let iter = NirValue::Local(name.to_string());
        let zero = NirValue::Const {
            type_: type_.clone(),
            value: "0".to_string(),
        };
        // The loop bound comparisons are infallible (comparisons never overflow),
        // so a default source location is correct here; only the increment below
        // can originate an overflow error and it carries the loop's location.
        let cmp = NirSourceLoc::default();
        let condition = NirValue::Binary {
            op: BinaryOp::Or,
            left: Box::new(NirValue::Binary {
                op: BinaryOp::And,
                left: Box::new(NirValue::Binary {
                    op: BinaryOp::GreaterEqual,
                    left: Box::new(step.clone()),
                    right: Box::new(zero.clone()),
                    loc: cmp,
                }),
                right: Box::new(NirValue::Binary {
                    op: BinaryOp::LessEqual,
                    left: Box::new(iter.clone()),
                    right: Box::new(end.clone()),
                    loc: cmp,
                }),
                loc: cmp,
            }),
            right: Box::new(NirValue::Binary {
                op: BinaryOp::And,
                left: Box::new(NirValue::Binary {
                    op: BinaryOp::Less,
                    left: Box::new(step.clone()),
                    right: Box::new(zero),
                    loc: cmp,
                }),
                right: Box::new(NirValue::Binary {
                    op: BinaryOp::GreaterEqual,
                    left: Box::new(iter.clone()),
                    right: Box::new(end.clone()),
                    loc: cmp,
                }),
                loc: cmp,
            }),
            loc: cmp,
        };
        let condition = self.lower_value(&condition)?;
        self.emit(abi::compare_immediate(&condition.location, "0"));
        self.emit(abi::branch_eq(&end_label));
        self.clear_local_constants();
        self.loop_stack.push(LoopLabels {
            kind: crate::ast::LoopKind::For,
            continue_label: continue_label.clone(),
            exit_label: end_label.clone(),
            cleanup_depth: self.active_cleanups.len(),
        });
        self.lower_loop_body(body)?;
        // plan-86 G1: the provable-index fact is scoped to this loop's body only.
        if g1_provable.is_some() {
            self.provable_index_locals.remove(name);
        }
        self.loop_stack.pop();
        self.emit(abi::label(&continue_label));
        let increment_node = NirValue::Binary {
            op: BinaryOp::Add,
            left: Box::new(iter),
            right: Box::new(step.clone()),
            loc,
        };
        let increment = self.lower_value(&increment_node)?;
        // Observation boundary: the incremented `Float` counter is written back
        // to its named slot, so a non-finite step must trap (plan-17).
        self.observe_float(&increment_node, &increment)?;
        self.store_value_at(&increment, abi::stack_pointer(), local_slot);
        self.emit(abi::branch(&loop_label));
        self.emit(abi::label(&end_label));
        self.end_loop_promotion(promoted)?;
        if let Some(previous) = previous {
            self.locals.insert(name.to_string(), previous);
        } else {
            self.locals.remove(name);
        }
        self.clear_local_constants();
        Ok(())
    }

    /// Reset a `String` local's capacity shadow to 0 ("tight, no spare") after any
    /// non-self-append bind/assign installs a fresh tight buffer. Keeps the shadow
    /// from claiming spare that the new buffer does not have (plan-02 §4.1).
    pub(crate) fn reset_string_capacity_shadow(&mut self, name: &str) {
        let zero = self.temporary_vreg();
        if let Some(&slot) = self.string_capacity_slots.get(name) {
            self.emit(abi::move_immediate(&zero, "Integer", "0"));
            self.emit(abi::store_u64(&zero, abi::stack_pointer(), slot));
        }
    }

    /// Pre-allocate a capacity shadow slot for every `String` local targeted by an
    /// in-place self-append (`name = name & …`) anywhere in `ops`, recursing into
    /// nested blocks. Done before lowering so bind/assign sites can reset the shadow
    /// and the prologue can zero it (plan-02 §4.1).
    pub(crate) fn prescan_string_self_appends(&mut self, ops: &[NirOp]) {
        for op in ops {
            match op {
                NirOp::Assign { name, value } => {
                    if string_self_append_operands(value, name).is_some()
                        && !self.string_capacity_slots.contains_key(name)
                    {
                        let slot = self.allocate_stack_object(&format!("strcap_{name}"), 8);
                        self.string_capacity_slots.insert(name.clone(), slot);
                    }
                }
                NirOp::If {
                    then_body,
                    else_body,
                    ..
                } => {
                    self.prescan_string_self_appends(then_body);
                    self.prescan_string_self_appends(else_body);
                }
                NirOp::Match { cases, .. } => {
                    for case in cases {
                        self.prescan_string_self_appends(&case.body);
                    }
                }
                NirOp::While { body, .. }
                | NirOp::For { body, .. }
                | NirOp::DoUntil { body, .. }
                | NirOp::ForEach { body, .. }
                | NirOp::Trap { body, .. } => {
                    self.prescan_string_self_appends(body);
                }
                _ => {}
            }
        }
    }

    /// Begin loop-carried promotion of safe float-accumulator locals for a loop
    /// with body `body` (plan-03 Stage D part 2). Each promotable local's slot
    /// value is loaded into a fresh FP virtual register held for the loop, and
    /// its folded constant is cleared so loop reads use the register. Returns the
    /// promoted names for `end_loop_promotion`. Emit this *before* the loop
    /// header so the load runs once on entry.
    pub(crate) fn begin_loop_promotion(
        &mut self,
        body: &[NirOp],
        exclude: Option<&str>,
    ) -> Result<Vec<String>, String> {
        let mut top_assigns = std::collections::HashSet::new();
        let mut excluded = std::collections::HashSet::new();
        scan_loop_locals(body, 0, &mut top_assigns, &mut excluded);
        // A numeric `FOR` induction variable is reassigned by the loop's own
        // increment (a slot store the lowering adds), so it is never promotable.
        if let Some(name) = exclude {
            excluded.insert(name.to_string());
        }
        let mut names: Vec<String> = top_assigns
            .into_iter()
            .filter(|name| !excluded.contains(name))
            .collect();
        names.sort();
        let mut promoted = Vec::new();
        for name in names {
            if self.address_taken_locals.contains(&name)
                || self.promoted_float_locals.contains_key(&name)
                || self.owner_containers.contains(&name)
            {
                continue;
            }
            let Some(local) = self.locals.get(&name) else {
                continue;
            };
            if !matches!(local.type_, ParameterType::Float) || local.by_ref {
                continue;
            }
            let stack_offset = local.stack_offset;
            let gpr = self.allocate_register();
            let d = self.allocate_fp_register();
            self.emit(abi::load_u64(&gpr, abi::stack_pointer(), stack_offset));
            self.emit(abi::float_move_d_from_x(&d, &gpr));
            if let Some(local) = self.locals.get_mut(&name) {
                local.constant = None;
            }
            self.promoted_float_locals.insert(name.clone(), d.render());
            promoted.push(name);
        }
        Ok(promoted)
    }

    /// Store each promoted local back to its slot and end its promotion. Emit
    /// this *after* the loop's exit label — every loop exit (normal, `EXIT`/
    /// break) reaches that label, and a `RETURN` inside the loop reads the
    /// register directly and leaves the function, so a single store-back covers
    /// all exits for a non-escaping local.
    pub(crate) fn end_loop_promotion(&mut self, promoted: Vec<String>) -> Result<(), String> {
        for name in promoted {
            let Some(d) = self.promoted_float_locals.remove(&name) else {
                continue;
            };
            let Some(stack_offset) = self.locals.get(&name).map(|local| local.stack_offset) else {
                continue;
            };
            let gpr = self.allocate_register();
            self.emit(abi::float_move_x_from_d(&gpr, &d));
            self.emit(abi::store_u64(&gpr, abi::stack_pointer(), stack_offset));
        }
        Ok(())
    }

    /// Update a promoted float local's resident register from an assignment
    /// result (an FP-resident result is a `d`-to-`d` move; a GPR result is moved
    /// in via `fmov d, x`).
    pub(crate) fn update_promoted_float(&mut self, d: &str, result: &ValueResult) {
        // A `d`-native result already lives in an FP register: move it `d`-to-`d`
        // (plan-01 float-dnative). Otherwise reuse a recorded FP resident, or
        // shuttle the GPR bits in (`fmov d, x`).
        if Self::float_is_dnative(result) {
            if result.location != d {
                self.emit(abi::float_move_d_from_d(d, &result.location));
            }
        } else if let Some(d_res) = self.float_residents.get(&result.location.render()).cloned() {
            if d_res != d {
                self.emit(abi::float_move_d_from_d(d, &d_res));
            }
        } else {
            self.emit(abi::float_move_d_from_x(d, &result.location));
        }
    }

    pub(crate) fn lower_for_each(
        &mut self,
        name: &str,
        type_: &ParameterType,
        iterable: &NirValue,
        body: &[NirOp],
    ) -> Result<(), String> {
        let collection = self.temporary_vreg();
        let remaining = self.temporary_vreg();
        let cursor = self.temporary_vreg();
        let payload_off = self.temporary_vreg();
        let payload_len = self.temporary_vreg();
        let iterable_value = self.lower_value(iterable)?;
        if !typed_is_collection_type(&iterable_value.type_) {
            return Err(format!(
                "native code FOR EACH target '{}' is not a collection",
                iterable_value.type_
            ));
        }
        // Structural destructure (plan-104-D). The render that used to happen
        // here "for the string-typed payload emitters below" is gone with
        // plan-111-E: those emitters take the type.
        let map_entry_types = match &iterable_value.type_ {
            ParameterType::MapOf(..) => {
                let (key, value) =
                    typed_map_type_parts(&iterable_value.type_).ok_or_else(|| {
                        format!(
                            "native code FOR EACH target '{}' is not a valid map type",
                            iterable_value.type_
                        )
                    })?;
                Some((key.clone(), value.clone()))
            }
            _ => None,
        };
        let list_element_type = typed_list_element_type(&iterable_value.type_).cloned();
        // A `Set OF T` iterates its Map-shaped entries yielding the element `T`
        // (the entry key), not a `MapEntry` (plan-63-B). Computed here so the loop
        // body below can read the key payload directly into the loop local.
        let set_element_type = typed_set_element_type(&iterable_value.type_).cloned();
        let item_value_type = list_element_type.as_ref();
        let collection_slot = self.allocate_stack_object("for_each_collection", 8);
        let cursor_slot = self.allocate_stack_object("for_each_cursor", 8);
        let remaining_slot = self.allocate_stack_object("for_each_remaining", 8);
        let local_slot = self.allocate_stack_object(name, 8);
        let entry_payload_slot = if map_entry_types.is_some() {
            Some(self.allocate_stack_object("for_each_map_entry", 16))
        } else {
            None
        };

        self.emit(abi::store_u64(
            &iterable_value.location,
            abi::stack_pointer(),
            collection_slot,
        ));
        // When the iterable is a plain local, its live buffer is the one this loop
        // snapshots and re-reads each step; record it so an in-place `set`/`prepend`
        // that overwrites an existing entry (observable to this iterator) is
        // excluded for the binding inside the loop body (plan-02 §4.1, D1).
        let pushed_iterable = if let NirValue::Local(local_name) = iterable {
            self.for_each_iterable_locals.push(local_name.clone());
            true
        } else {
            false
        };
        // bug-430: `FOR EACH x IN resource.state.field` snapshots an ALIAS into the
        // STATE collection's inlined buffer, so record it — an in-place
        // `resource.state.field = append(...)` in the body must NOT reallocate+free
        // that buffer out from under the live iterator (it falls back to the
        // non-freeing whole-record rebuild instead).
        let pushed_state_field = if let NirValue::MemberAccess { target, member } = iterable {
            if let NirValue::MemberAccess {
                target: inner,
                member: inner_member,
            } = target.as_ref()
            {
                if inner_member == "state" {
                    if let NirValue::Local(res) = inner.as_ref() {
                        self.for_each_iterable_state_fields
                            .push((res.clone(), member.clone()));
                        true
                    } else {
                        false
                    }
                } else {
                    false
                }
            } else {
                false
            }
        } else {
            false
        };
        // bug-430: `FOR EACH x IN local.field` over a MUT record local snapshots an
        // alias into the record's inlined collection buffer — record it so an
        // in-place record-field grow in the body falls back to the rebuild.
        let pushed_record_field = if let NirValue::MemberAccess { target, member } = iterable {
            if let NirValue::Local(base) = target.as_ref() {
                self.for_each_iterable_record_fields
                    .push((base.clone(), member.clone()));
                true
            } else {
                false
            }
        } else {
            false
        };
        self.emit(abi::load_u64(
            &collection,
            abi::stack_pointer(),
            collection_slot,
        ));
        self.emit(abi::load_u64(
            &remaining,
            &collection,
            COLLECTION_OFFSET_COUNT,
        ));
        // A kind-2 list has no entry table: the cursor carries a byte OFFSET from
        // the data base instead of an entry pointer, and strides by payloadSize
        // (plan-57-D). The Map arm below is unaffected — maps keep their entries.
        let list_payload = item_value_type.and_then(kind2_payload_size);
        if list_payload.is_some() {
            self.emit(abi::move_immediate(&cursor, "Integer", "0"));
        } else {
            self.emit(abi::add_immediate(
                &cursor,
                &collection,
                COLLECTION_HEADER_SIZE,
            ));
        }
        self.emit(abi::store_u64(&cursor, abi::stack_pointer(), cursor_slot));
        self.emit(abi::store_u64(
            &remaining,
            abi::stack_pointer(),
            remaining_slot,
        ));

        let loop_label = self.label("for_each_loop");
        let end_label = self.label("for_each_end");
        self.emit(abi::label(&loop_label));
        self.emit(abi::load_u64(
            &remaining,
            abi::stack_pointer(),
            remaining_slot,
        ));
        self.emit(abi::compare_immediate(&remaining, "0"));
        self.emit(abi::branch_eq(&end_label));
        self.emit(abi::load_u64(&cursor, abi::stack_pointer(), cursor_slot));
        if let (Some(entry_payload_slot), Some((key_type, value_type))) =
            (entry_payload_slot, map_entry_types.as_ref())
        {
            self.emit(abi::load_u64(
                &payload_off,
                &cursor,
                COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
            ));
            self.emit(abi::load_u64(
                &payload_len,
                &cursor,
                COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
            ));
            self.emit(abi::load_u64(
                &collection,
                abi::stack_pointer(),
                collection_slot,
            ));
            let key_value =
                self.emit_load_map_payload(key_type, &collection, &payload_off, &payload_len)?;
            self.emit(abi::store_u64(
                key_value,
                abi::stack_pointer(),
                entry_payload_slot,
            ));
            self.emit(abi::load_u64(&cursor, abi::stack_pointer(), cursor_slot));
            self.emit(abi::load_u64(
                &payload_off,
                &cursor,
                COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
            ));
            self.emit(abi::load_u64(
                &payload_len,
                &cursor,
                COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
            ));
            self.emit(abi::load_u64(
                &collection,
                abi::stack_pointer(),
                collection_slot,
            ));
            let item_value =
                self.emit_load_map_payload(value_type, &collection, &payload_off, &payload_len)?;
            self.emit(abi::store_u64(
                item_value,
                abi::stack_pointer(),
                entry_payload_slot + 8,
            ));
            self.emit(abi::add_immediate(
                &payload_off,
                abi::stack_pointer(),
                entry_payload_slot,
            ));
            self.emit(abi::store_u64(
                &payload_off,
                abi::stack_pointer(),
                local_slot,
            ));
        } else if let Some(set_element_type) = set_element_type.as_ref() {
            // `Set OF T`: read the entry's KEY payload (the element) into the loop
            // local as `T`. Entries stride by `COLLECTION_ENTRY_SIZE` (the common
            // advance below, since `list_payload` is `None` for a Set).
            self.emit(abi::load_u64(
                &payload_off,
                &cursor,
                COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
            ));
            self.emit(abi::load_u64(
                &payload_len,
                &cursor,
                COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
            ));
            self.emit(abi::load_u64(
                &collection,
                abi::stack_pointer(),
                collection_slot,
            ));
            let item_value = self.emit_load_map_payload(
                set_element_type,
                &collection,
                &payload_off,
                &payload_len,
            )?;
            self.emit(abi::store_u64(item_value, abi::stack_pointer(), local_slot));
        } else {
            let item_value_type = item_value_type.ok_or_else(|| {
                format!(
                    "native code FOR EACH target '{}' is not a list",
                    iterable_value.type_
                )
            })?;
            if let Some(payload) = list_payload {
                self.emit(abi::move_register(&payload_off, &cursor));
                self.emit(abi::move_immediate(
                    &payload_len,
                    "Integer",
                    &payload.to_string(),
                ));
            } else {
                self.emit(abi::load_u64(
                    &payload_off,
                    &cursor,
                    COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
                ));
                self.emit(abi::load_u64(
                    &payload_len,
                    &cursor,
                    COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
                ));
            }
            self.emit(abi::load_u64(
                &collection,
                abi::stack_pointer(),
                collection_slot,
            ));
            let item_value = self.emit_load_collection_payload(
                item_value_type,
                &collection,
                &payload_off,
                &payload_len,
            )?;
            self.emit(abi::store_u64(item_value, abi::stack_pointer(), local_slot));
        }
        self.emit(abi::load_u64(&cursor, abi::stack_pointer(), cursor_slot));
        self.emit(abi::add_immediate(
            &cursor,
            &cursor,
            list_payload.unwrap_or(COLLECTION_ENTRY_SIZE),
        ));
        self.emit(abi::store_u64(&cursor, abi::stack_pointer(), cursor_slot));
        self.emit(abi::load_u64(
            &remaining,
            abi::stack_pointer(),
            remaining_slot,
        ));
        self.emit(abi::subtract_immediate(&remaining, &remaining, 1));
        self.emit(abi::store_u64(
            &remaining,
            abi::stack_pointer(),
            remaining_slot,
        ));

        let previous = self.locals.insert(
            name.to_string(),
            LocalValue {
                type_: type_.clone(),
                stack_offset: local_slot,
                constant: None,
                by_ref: false,
            },
        );
        self.clear_local_constants();
        self.loop_stack.push(LoopLabels {
            kind: crate::ast::LoopKind::For,
            continue_label: loop_label.clone(),
            exit_label: end_label.clone(),
            cleanup_depth: self.active_cleanups.len(),
        });
        self.lower_loop_body(body)?;
        self.loop_stack.pop();
        if pushed_iterable {
            self.for_each_iterable_locals.pop();
        }
        if pushed_state_field {
            self.for_each_iterable_state_fields.pop();
        }
        if pushed_record_field {
            self.for_each_iterable_record_fields.pop();
        }
        if let Some(previous) = previous {
            self.locals.insert(name.to_string(), previous);
        } else {
            self.locals.remove(name);
        }
        self.emit(abi::branch(&loop_label));
        self.emit(abi::label(&end_label));
        self.clear_local_constants();
        Ok(())
    }
}

/// If `value` is a left-associated string-concat chain `name & a & b …` whose
/// leftmost leaf is `Local(name)`, return the operands to append in source order
/// (`[a, b, …]`); otherwise `None`. Used to recognize the in-place self-append
/// idiom `name = name & …` (plan-02 §4.1). `&` is string concatenation, so a
/// match guarantees `name` is a `String` local.
pub(crate) fn string_self_append_operands<'v>(
    value: &'v NirValue,
    name: &str,
) -> Option<Vec<&'v NirValue>> {
    let NirValue::Binary {
        op, left, right, ..
    } = value
    else {
        return None;
    };
    if *op != BinaryOp::Concat {
        return None;
    }
    let mut operands = vec![right.as_ref()];
    let mut cursor = left.as_ref();
    loop {
        match cursor {
            NirValue::Local(local) if local == name => {
                operands.reverse();
                return Some(operands);
            }
            NirValue::Binary {
                op, left, right, ..
            } if *op == BinaryOp::Concat => {
                operands.push(right.as_ref());
                cursor = left.as_ref();
            }
            _ => return None,
        }
    }
}

/// True when `name` is read anywhere within `value`. Used to reject the in-place
/// string self-append fast path when the target reappears as (or inside) a later
/// operand of the concat chain (`s = s & x & s`): the operands are lowered one at
/// a time *after* earlier ones have mutated the buffer, so a later read of `name`
/// would see the already-extended value (bug-143). A `LocalRef` reference to the
/// same slot is equally hazardous.
pub(crate) fn nir_value_reads_local(value: &NirValue, name: &str) -> bool {
    match value {
        NirValue::Local(local) | NirValue::LocalRef { name: local, .. } => local == name,
        NirValue::Const { .. }
        | NirValue::Global { .. }
        | NirValue::FunctionRef { .. }
        | NirValue::Capture { .. } => false,
        NirValue::Closure { captures, .. } => {
            captures.iter().any(|v| nir_value_reads_local(v, name))
        }
        NirValue::Call { args, .. }
        | NirValue::CallResult { args, .. }
        | NirValue::RuntimeCall { args, .. }
        | NirValue::Constructor { args, .. } => args.iter().any(|v| nir_value_reads_local(v, name)),
        NirValue::UnionWrap { value, .. }
        | NirValue::UnionExtract { value, .. }
        | NirValue::ResultIsOk { value }
        | NirValue::ResultValue { value }
        | NirValue::ResultError { value }
        | NirValue::Checked { value, .. }
        | NirValue::Unary { operand: value, .. } => nir_value_reads_local(value, name),
        NirValue::WithUpdate {
            target, updates, ..
        } => {
            nir_value_reads_local(target, name)
                || updates
                    .iter()
                    .any(|u| nir_value_reads_local(&u.value, name))
        }
        NirValue::ListLiteral { values, .. } | NirValue::SetLiteral { values, .. } => {
            values.iter().any(|v| nir_value_reads_local(v, name))
        }
        NirValue::MapLiteral { entries, .. } => entries
            .iter()
            .any(|(k, v)| nir_value_reads_local(k, name) || nir_value_reads_local(v, name)),
        NirValue::MemberAccess { target, .. } => nir_value_reads_local(target, name),
        NirValue::Binary { left, right, .. } => {
            nir_value_reads_local(left, name) || nir_value_reads_local(right, name)
        }
    }
}

fn nir_op_context(op: &NirOp) -> String {
    match op {
        NirOp::Bind { name, type_, .. } => format!("bind {name} AS {type_}"),
        NirOp::StoreGlobal { name, .. } => format!("store global {name}"),
        NirOp::Assign { name, .. } => format!("assign {name}"),
        NirOp::StateAssign { resource, .. } => format!("state assign {resource}"),
        NirOp::Return { .. } => "return".to_string(),
        NirOp::ExitLoop { .. } => "exit loop".to_string(),
        NirOp::ContinueLoop { .. } => "continue loop".to_string(),
        NirOp::ExitProgram { .. } => "exit program".to_string(),
        NirOp::Fail { .. } => "fail".to_string(),
        NirOp::Eval { value } => format!("eval {}", nir_value_context(value)),
        NirOp::If { .. } => "if".to_string(),
        NirOp::Match { .. } => "match".to_string(),
        NirOp::While { .. } => "while".to_string(),
        NirOp::For { name, .. } => format!("for {name}"),
        NirOp::DoUntil { .. } => "do until".to_string(),
        NirOp::ForEach { name, .. } => format!("for each {name}"),
        NirOp::Trap { .. } => "trap".to_string(),
    }
}

fn nir_value_context(value: &NirValue) -> String {
    match value {
        NirValue::Call { target, .. }
        | NirValue::CallResult { target, .. }
        | NirValue::RuntimeCall { target, .. } => format!("call {target}"),
        NirValue::Constructor { type_, .. } => format!("construct {type_}"),
        NirValue::MemberAccess { member, .. } => format!("member {member}"),
        NirValue::Local(name) => format!("local {name}"),
        NirValue::LocalRef { name, .. } => format!("local ref {name}"),
        NirValue::Global { name, .. } => format!("global {name}"),
        NirValue::FunctionRef { name, .. } => format!("function {name}"),
        NirValue::Closure { name, .. } => format!("closure {name}"),
        NirValue::Const { type_, .. } => format!("const {type_}"),
        NirValue::ListLiteral { type_, .. }
        | NirValue::SetLiteral { type_, .. }
        | NirValue::MapLiteral { type_, .. } => {
            format!("literal {type_}")
        }
        NirValue::Unary { op, .. } => format!("operator {}", op.name()),
        NirValue::Binary { op, .. } => format!("operator {}", op.name()),
        NirValue::UnionWrap {
            union_type,
            member_type,
            ..
        } => format!("wrap {member_type} AS {union_type}"),
        NirValue::UnionExtract { type_, .. } => format!("extract {type_}"),
        NirValue::ResultIsOk { .. } => "result is ok".to_string(),
        NirValue::ResultValue { .. } => "result value".to_string(),
        NirValue::ResultError { .. } => "result error".to_string(),
        NirValue::Checked { type_, .. } => format!("checked {type_}"),
        NirValue::WithUpdate { type_, .. } => format!("with update {type_}"),
        NirValue::Capture { index, .. } => format!("capture {index}"),
    }
}
