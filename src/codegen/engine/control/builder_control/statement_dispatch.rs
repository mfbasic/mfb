//! plan-176-U: `lower_ops_inner`, the per-`NirOp` statement dispatch. The statement entry
//! (`lower_ops`) and shared types live in `engine/control/builder_control/mod.rs`.

use super::*;

impl CodeBuilder<'_> {
    pub(super) fn lower_ops_inner(
        &mut self,
        ops: &[NirOp],
        cleanup_scope_start: usize,
    ) -> Result<(), String> {
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
            // plan-134-D: a store lowered for this op asks plan-134-C's analysis about
            // THIS op; a nested body records its own and this op's key comes back after.
            let enclosing_op_key = self
                .current_op_key
                .replace(crate::codegen::engine::analysis::last_use::op_key(op));
            crate::codegen::engine::expansion::enter(
                || crate::codegen::engine::expansion::op_key(op).to_string(),
                self.instructions.len(),
            );
            // bug-842: a binding, an assignment and a module-level store take the
            // resources their own value carries, so that root is no temporary.
            let enclosing_temp_exempt = std::mem::replace(
                &mut self.resource_temp_exempt,
                match op {
                    NirOp::Bind {
                        value: Some(value), ..
                    }
                    | NirOp::Assign { value, .. }
                    | NirOp::StoreGlobal {
                        value: Some(value), ..
                    } => {
                        // A resource union's bind wraps its producer
                        // (`RES s AS Stream = tls::connect(…)`): the producer is
                        // the root the binding claims.
                        let mut root = value;
                        while let NirValue::UnionWrap { value: inner, .. } = root {
                            root = inner;
                        }
                        Some(root as *const NirValue as usize)
                    }
                    _ => None,
                },
            );
            let result = (|| -> Result<(), String> {
                match op {
                    NirOp::Bind {
                        mutable,
                        name,
                        type_,
                        value,
                    } => {
                        // bug-766: `LET u = WITH p { … }` standing for an
                        // element-bound `p`'s update — `u` is never bound: its one
                        // read is the write-back, lowered as nothing.
                        let key = crate::codegen::engine::analysis::last_use::op_key(op);
                        if let (Some(element), Some(value)) =
                            (self.element_updates.updates.get(&key).cloned(), value)
                        {
                            return self.lower_element_update(&element, Some(key), value);
                        }
                        // bug-1075: a binding a `LAMBDA` captures by reference
                        // and that reaches a resource is a three-word cell.
                        let ref_cell = self.statement_temps.ref_cells.contains(name)
                            && self.type_carries_resources(type_)
                            && !self.type_reaches_closure_leaf(type_)
                            && !matches!(value, Some(NirValue::Capture { by_ref: true, .. }));
                        let stack_offset = self.allocate_stack_object(
                            name,
                            if ref_cell {
                                self.ref_cell_size(type_)
                            } else {
                                8
                            },
                        );
                        if ref_cell {
                            self.setup_ref_cell(name, type_, stack_offset)?;
                        }
                        // A non-escaping `MUT` by-ref capture: the env slot holds a
                        // pointer to the parent binding's slot, so this binding is a
                        // *reference* local — its slot stores that pointer and reads
                        // /writes deref through it. It is non-owning:
                        // never deep-copied and never freed here (the parent owns
                        // and frees the value).
                        let by_ref_capture_slot =
                            matches!(value, Some(NirValue::Capture { by_ref: true, .. }));
                        // A by-value capture of a flat value is read straight out of
                        // the closure env: the env owns the copy made when the closure
                        // was created, it outlives this call, and the binding is
                        // immutable. Copying it here cost one block per call
                        // (`rt_lambda_capture_not_copied_per_call`). Like a by-ref
                        // capture, the binding owns nothing.
                        // bug-781: a captured function value (or a value holding one)
                        // is owned by the environment the same way.
                        //
                        // bug-843: so is a captured resource. The environment holds a
                        // reference to it, exactly as a `List` element does; the
                        // resource belongs to its own (possibly raised) scope, so the
                        // binding neither claims an unowned record nor closes one.
                        let borrowed_capture = !*mutable
                            && matches!(value, Some(NirValue::Capture { by_ref: false, .. }))
                            && (self.is_freeable_flat_value(type_)
                                || self.owns_graph(type_)
                                || self.resource_cleanup_symbol(type_).is_some()
                                || self.resource_union_cleanup(type_).is_some());
                        let aliases_capture = by_ref_capture_slot || borrowed_capture;
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
                        // bug-778: an inline-`TRAP` temporary read only by the three
                        // `Result` readers lives in frame slots, not a heap block.
                        self.unboxed_trap_locals.remove(name);
                        if let Some(value) = value {
                            if let Some((uses, payload, block)) =
                                self.unboxed_trap_shape(name, type_, value)
                            {
                                return self.lower_unboxed_trap_bind(
                                    name,
                                    value,
                                    stack_offset,
                                    uses,
                                    payload,
                                    block,
                                );
                            }
                        }
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
                        // A raw thread-boundary result (`t.result`) is owned by the
                        // thread runtime / block arena, not this scope, so it is
                        // neither zero-initialized nor freed here. A
                        // `thread::receive`/`waitFor`/… value is the call site's copy
                        // into this arena and is owned like any fresh value (bug-622 A).
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
                        // bug-766 B: a scratch copy lowers its `get` as the same
                        // borrow, copies the element into its record-temp region
                        // at once, and owns no block either.
                        // bug-784: a re-bound name takes this binding's role; a
                        // borrowing one skips the name's scratch copy.
                        let rebound_borrow = self
                            .apply_rebound(crate::codegen::engine::analysis::last_use::op_key(op));
                        let scratch_copy = if rebound_borrow {
                            None
                        } else {
                            self.record_temps.bindings.get(name).copied()
                        };
                        let is_borrow_get = (self.borrow_get_locals.contains(name)
                            || scratch_copy.is_some())
                            && self.is_freeable_flat_value(type_)
                            && !matches!(type_, ParameterType::String);
                        // plan-134-G: a recursive value's owner frees its graph — except a
                        // borrowed `MATCH` view (plan-134-C), whose bind skips the graph copy
                        // and so holds its source's graph without owning it.
                        let borrowed_graph_view =
                            self.owns_graph(type_) && self.store_is_borrowed_view();
                        let owns_freeable_value = !aliases_union_variant
                            && !aliases_capture
                            && !runtime_managed
                            && !promote_vector
                            && !is_borrow_get
                            && !borrowed_graph_view
                            && (self.is_freeable_flat_value(type_) || self.owns_graph(type_));
                        // bug-593: the `$trap_resN : Result OF T = CallResult(..)` an
                        // inline `TRAP` binds is a `{tag, size, payload}` block THIS
                        // frame allocated (`fresh_trapped_result_value` is its only
                        // constructor). A flat `T` is released by the branch above; a
                        // non-flat `T` — a resource, or a collection of `net::Address`
                        // — got no owner at all, and on the error path that wrapper
                        // carries the whole trapped `Error` inline. §14 preamble: every
                        // live value is owned by exactly one temporary; §14.7: it drops
                        // on every scope edge. This names the owner. It ADDS the one
                        // wrapper free and moves no lifetime: the Ok wrapper of an inlined
                        // block payload — which `ResultValue` hands the binding as an
                        // alias — is kept. A recursive payload is the exception: every
                        // owning store graph-copies it, so the wrapper is its only owner
                        // and drops its graph (plan-134, `OkGraphPayload`).
                        let result_wrapper = match (type_, value.as_ref()) {
                            (
                                ParameterType::ResultOf(payload),
                                Some(NirValue::CallResult { .. }),
                            ) if !owns_freeable_value
                                && !aliases_union_variant
                                && !aliases_capture
                                && !runtime_managed
                                && !promote_vector =>
                            {
                                Some(if self.owns_graph(payload) {
                                    ResultWrapperDrop::OkGraphPayload
                                } else if self.result_payload_is_block(payload) {
                                    ResultWrapperDrop::ErrorOnly
                                } else {
                                    ResultWrapperDrop::Always
                                })
                            }
                            _ => None,
                        };
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
                        // Rule 4: an alias read out of a value the statement owns
                        // (`RES r = collections::get(mkList(), 0)`) is taken by this
                        // binding's scope like any other, so it is no mere alias.
                        let aliases_live_resource = value.as_ref().is_some_and(|value| {
                            Self::value_aliases_live_resource(value)
                                && !crate::codegen::cleanup::owned::builder_owned_cleanup::value_may_carry_statement_resources(value)
                        });
                        let owns_resource_slot = !aliases_union_variant
                            && !by_ref_capture_slot
                            && !floats_to_collection
                            && !aliases_live_resource
                            && (self.resource_cleanup_symbol(type_).is_some()
                                || self.resource_union_cleanup(type_).is_some());
                        // bug-1012: a resource-union binding that owns its box and
                        // nothing else. Three shapes:
                        //
                        // * a read of a record field (`RES v AS Handle = b.h`). The
                        //   record holds the box inline, so the field read is an
                        //   address inside the record's block, which the record's
                        //   owner frees. The binding takes its own copy of the two
                        //   words, as a binding of any plain value read out of a
                        //   field copies it.
                        // * a borrowed-element read (bug-1026: `RES v AS Handle =
                        //   collections::get(hs, 0)`, `getOr`, over a `List` or a
                        //   `Map`). The collection holds the box inline too, so the
                        //   call yields an address inside the collection's block,
                        //   which its owner frees (at the end of the statement when
                        //   the collection is a temporary: `MATCH
                        //   collections::get(mkHandles(), 0)`); and a variant
                        //   written as `getOr`'s default is wrapped in a box the
                        //   statement frees at its end. The binding copies the two
                        //   words either way.
                        // * a binding whose resource floated to an outer container.
                        //   The container's owned-list keeps its own copy of the box
                        //   (`emit_owned_list_push_node`), so the box this bind's
                        //   initializer made is still the binding's to free.
                        //
                        // Either way the drop frees the 16 bytes and closes nothing:
                        // the record belongs to the field's record's owner, or to
                        // the container.
                        let is_union_binding = self.resource_union_cleanup(type_).is_some()
                            && !aliases_union_variant
                            && !aliases_capture;
                        let copies_borrowed_box = is_union_binding
                            && match value {
                                Some(NirValue::MemberAccess { .. }) => true,
                                Some(
                                    NirValue::Call { target, .. }
                                    | NirValue::CallResult { target, .. }
                                    | NirValue::RuntimeCall { target, .. },
                                ) => Self::target_returns_borrowed_resource(target),
                                _ => false,
                            };
                        let owns_box_only = copies_borrowed_box
                            || (is_union_binding
                                && floats_to_collection
                                && value.as_ref().is_some_and(|value| {
                                    self.value_yields_fresh_union_box(value, type_)
                                }));
                        // Zero the slot before a (possibly fallible) initializer
                        // runs. If the initializer traps before storing, the slot
                        // stays null and the scope-drop free/close skips it instead of
                        // touching an uninitialized pointer.
                        if owns_freeable_value
                            || result_wrapper.is_some()
                            || owns_resource_slot
                            || owns_box_only
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
                        // is consumed only as the entry-zeroing list. A `Thread` view's
                        // slot is one of them: a `thread::start` that raises on a bad
                        // queue limit leaves it unwritten too (bug-469).
                        if owns_resource_slot || owns_box_only {
                            self.owned_value_slots.push(stack_offset);
                        }
                        // bug-648: an inline-`TRAP` temp has no initializer to classify,
                        // and some of its stores are lent to it (a borrowed element, an
                        // aliasing `RECOVER`), so whether it owns what it holds is a run-
                        // time fact. The flag reads "the value in the slot is owned". It is
                        // SET here because the closed default this bind materializes is the
                        // temp's own; the prologue clears it, which is harmless because a
                        // drop reached before this bind finds a null slot first.
                        let owner_flag_slot = if owns_resource_slot
                            && value.is_none()
                            && self.trap_ownership.lent_temps.contains(name)
                        {
                            let flag = self.allocate_stack_object("trap_owner_flag", 8);
                            let owned = self.temporary_vreg();
                            self.emit(abi::move_immediate(&owned, "Integer", "1"));
                            self.emit(abi::store_u64(&owned, abi::stack_pointer(), flag));
                            self.owned_value_slots.push(flag);
                            self.trap_owner_flags.insert(name.clone(), flag);
                            Some(flag)
                        } else {
                            None
                        };
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
                                            // A promoted-vector fallback block is never a
                                            // `String`, so it never carries self-append
                                            // headroom.
                                            capacity_slot: None,
                                            loop_alias_slot: None,
                                            result_wrapper: None,
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
                            let result = if aliases_union_variant || aliases_capture {
                                self.lower_value(value)?
                            } else if is_borrow_get {
                                // bug-592: ARM, do not set — `lower_value` turns the
                                // flag on for this initializer's own node only, so
                                // its operands keep their ordinary frees.
                                self.borrow_get_armed = true;
                                let r = self.lower_value(value);
                                self.borrow_get_armed = false;
                                self.borrow_get_result = false;
                                let r = r?;
                                match scratch_copy {
                                    Some((offset, size)) => {
                                        self.copy_into_record_temp(r, offset, size)?
                                    }
                                    None => r,
                                }
                            } else if self.append_headroom_bind(op, name, type_) {
                                // bug-770 C: ARM, like the borrow above — only the
                                // `get`/`getOr` node's own frame copies with headroom.
                                self.headroom_copy_armed = true;
                                let r = self.lower_value_owned(value);
                                self.headroom_copy_armed = false;
                                self.headroom_copy_result = false;
                                r?
                            } else {
                                self.lower_value_owned(value)?
                            };
                            // bug-1012, bug-1026: the binding's own copy of the box
                            // a field read or a borrowed-element read named.
                            let result = if copies_borrowed_box {
                                let copied = self
                                    .materialize_inline_value_in_arena(type_, &result.location)?;
                                ValueResult {
                                    location: Operand::from(copied),
                                    ..result
                                }
                            } else {
                                result
                            };
                            self.raw_result_discard_error = false;
                            // Observation boundary: a `Float` becoming a named
                            // binding must be finite (plan-17).
                            self.observe_float(value, &result)?;
                            // A `d`-native `Float` stores via `str d` (plan-01
                            // float-dnative); every other value via `str x`.
                            self.store_value_at(&result, abi::stack_pointer(), stack_offset);
                        } else if owns_freeable_value
                            && !owns_resource_slot
                            && is_inline_trap_delivery_slot(name)
                            && self.result_payload_is_block(type_)
                        {
                            // bug-778: the inline-`TRAP` desugar's delivery slots —
                            // `$trap_valN` (the value) and `$trap_errN` (a check
                            // chain's error) — are assigned on every path before
                            // they are read: the `Ok` arm assigns the value, a
                            // failure assigns the error, and every handler path must
                            // `RECOVER` (which assigns) or diverge (`mfb spec language
                            // error-model` §8 rule 12). A default block here is built
                            // and freed unread on every evaluation, success included.
                            // The slot was zeroed above, and a null slot is what the
                            // assignment's free of the old value and the scope drop
                            // both skip.
                        } else {
                            let result = self.lower_default_value(type_)?;
                            // The default empty `String` is static rodata; copy it
                            // into the arena so this binding owns an arena block its
                            // scope-drop free can reclaim (collections/records
                            // default to arena allocations already).
                            let location = if result.type_ == ParameterType::String {
                                Operand::from(
                                    self.copy_flat_block(&ParameterType::String, &result.location)?,
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
                        //
                        // A binding of a call result that reaches resources adopts
                        // the ones the callee transferred out: this scope owns them
                        // and closes each once at exit (§15.6). bug-805: at any
                        // depth of the value — a `List OF List OF Holder`, a record
                        // with a `RES` field, a map of such records — not only a
                        // flat `List OF RES File`. Not for a `collections` helper
                        // over the caller's own handles, which transfers nothing
                        // (bug-643).
                        let adopts_call_result = self.is_resource_owning_container(type_)
                            && value.as_ref().is_some_and(|value| {
                                crate::codegen::cleanup::owned::builder_owned_cleanup::value_may_carry_statement_resources(value)
                                    // The inline-`TRAP` desugar delivers a trapped
                                    // call's success value through `resultValue`:
                                    // the binding is that call's taker.
                                    || self.adopts_from_trapped_call(value)
                            });
                        if self.owner_containers.contains(name) || adopts_call_result {
                            self.setup_owned_list(name, type_)?;
                            if adopts_call_result {
                                self.emit_owned_list_seed_from_collection(
                                    name,
                                    stack_offset,
                                    type_,
                                )?;
                            }
                        }
                        // Where this binding's close obligation lives (§15.6).
                        let resource_owner = self
                            .resource_owners
                            .get(name)
                            .cloned()
                            .unwrap_or(crate::ir::resource_escape::ResOwner::Local);
                        if aliases_union_variant || aliases_capture {
                            // Non-owning — no cleanup (the parent binding or the
                            // closure env frees it).
                        } else if let crate::ir::resource_escape::ResOwner::Float(collection) =
                            &resource_owner
                        {
                            // Ownership floated to an outer collection's scope:
                            // register the record in that owned-list. This binding
                            // is now an alias and registers no static cleanup.
                            let collection = collection.clone();
                            if self.is_module_global(&collection) {
                                // bug-806: the container is a module-level binding,
                                // the outermost scope: the module instance owns the
                                // record and closes it at instance teardown.
                                self.emit_module_float_value(stack_offset, type_)?;
                            } else {
                                self.emit_owned_list_push(&collection, stack_offset, type_)?;
                            }
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
                                self.release_floated_source_cleanup(src);
                            }
                            if owns_box_only {
                                self.register_union_box_cleanup(name, type_);
                            }
                        } else if copies_borrowed_box
                            && !value.as_ref().is_some_and(|value| {
                                crate::codegen::cleanup::owned::builder_owned_cleanup::value_may_carry_statement_resources(value)
                            })
                        {
                            // A box read out of a named place: the record has its
                            // owner there. One read out of a value the statement
                            // owns takes the full union arm below (rule 4): the
                            // binding raises the record to its scope and closes it.
                            self.register_union_box_cleanup(name, type_);
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
                            if let Some(NirValue::Local(src)) = value {
                                self.live_resource_aliases.insert(name.clone(), src.clone());
                                // bug-1050: a STATE this binding attaches rides the
                                // record its source owns, so the source's cleanup
                                // frees it after its close. The inline-`TRAP`
                                // desugar is the usual road here: `RES v AS fs::File
                                // STATE Cur = open() TRAP` binds `v` from a delivery
                                // temporary typed by the producer, with no STATE,
                                // and the temporary's cleanup is the one that runs.
                                // It knew of no state, so the block was kept on
                                // every evaluation. A root with no live cleanup (a
                                // parameter) keeps the STATE for the caller's owner.
                                if let Some(state) = type_.state() {
                                    let root = self.resource_alias_root(src);
                                    let owner_state =
                                        self.active_cleanups.iter_mut().rev().find_map(|cleanup| {
                                            match cleanup {
                                                ActiveCleanup::Resource(owner)
                                                    if owner.name == root =>
                                                {
                                                    Some(&mut owner.state_type)
                                                }
                                                ActiveCleanup::ResourceUnion(owner)
                                                    if owner.name == root =>
                                                {
                                                    Some(&mut owner.state_type)
                                                }
                                                _ => None,
                                            }
                                        });
                                    if let Some(owner_state) = owner_state {
                                        if owner_state.is_none() {
                                            *owner_state = Some(state);
                                        }
                                    }
                                }
                            }
                        } else if let Some(symbol) = self.resource_cleanup_symbol(type_) {
                            // bug-755: the binding claims the record only when it is
                            // unowned — a producer's fresh record, or one a callee's
                            // `RETURN` released. A record owned further out (a handle
                            // the callee was passed and hands back) keeps its owner, and
                            // the drop registered here closes nothing.
                            let scope_depth = self.current_scope_depth();
                            self.emit_resource_claim_at(stack_offset, scope_depth)?;
                            // The scope that closes a resource frees its record:
                            // the drop takes the record it closed and the exit
                            // frees it after its last cleanup, by the owner word.
                            let frees_record = Self::resource_record_freed_at_drop(type_);
                            let reclaim_slot = frees_record.then(|| {
                                let slot = self
                                    .allocate_stack_object(&format!("resource_reclaim_{name}"), 8);
                                self.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), slot));
                                self.owned_value_slots.push(slot);
                                slot
                            });
                            // bug-1051: a value-less bind is the inline-`TRAP`
                            // desugar's temp, and the closed default it has just
                            // stored is its own; a store over it frees it.
                            let default_record_slot = (value.is_none()
                                && Self::resource_record_freed_at_drop(type_))
                                .then(|| self.remember_trap_default_record(name, stack_offset, 0));
                            let cleanup = ResourceCleanup {
                                name: name.clone(),
                                symbol,
                                state_type: type_.state(),
                                has_io_buffers: Self::resource_uses_io_buffers(type_),
                                frees_record,
                                moved_record_only: false,
                                owner_flag_slot,
                                default_record_slot,
                                scope_depth,
                                reclaim_slot,
                            };
                            if reclaim_slot.is_some() {
                                self.reclaiming_resources
                                    .push(ActiveCleanup::Resource(cleanup.clone()));
                            }
                            self.active_cleanups.push(ActiveCleanup::Resource(cleanup));
                        } else if let Some(variants) = self.resource_union_cleanup(type_) {
                            // A resource union drops by dispatching on its tag to
                            // the active variant's registered close op, then frees
                            // the active variant record's uniform STATE (plan-74).
                            //
                            // Resource-union alias class: a union wrapping an aliasing
                            // source (`RES c AS Union = u`, a field, an extract, a
                            // borrowed element) does not own that record. Its drop
                            // frees only its own box; the record's owner closes it.
                            let wrapped = match value {
                                Some(NirValue::UnionWrap { value: inner, .. }) => {
                                    Some(inner.as_ref())
                                }
                                _ => None,
                            };
                            let alias_wrap = wrapped.is_some_and(|inner| {
                                matches!(inner, NirValue::UnionExtract { .. })
                                    || Self::value_aliases_live_resource(inner)
                            });
                            if let Some(NirValue::Local(src)) = wrapped {
                                self.live_union_wraps.insert(name.clone(), src.clone());
                                // A STATE the alias attaches rides the wrapped record, so
                                // its owner frees it after its close (the union's drop no
                                // longer does). A root with no live cleanup (a parameter)
                                // keeps the STATE for the caller's owner.
                                if let Some(state) = type_.state() {
                                    let root = self.resource_alias_root(src);
                                    if let Some(ActiveCleanup::Resource(owner)) = self
                                        .active_cleanups
                                        .iter_mut()
                                        .rev()
                                        .find(|c| matches!(c, ActiveCleanup::Resource(r) if r.name == root))
                                    {
                                        if owner.state_type.is_none() {
                                            owner.state_type = Some(state);
                                        }
                                    }
                                }
                            }
                            let scope_depth = self.current_scope_depth();
                            if !alias_wrap {
                                self.emit_resource_union_claim_at(stack_offset, scope_depth)?;
                            }
                            // The scope that closes the active variant frees its
                            // record, by the owner word, after the exit's last
                            // cleanup (as the concrete arm).
                            let reclaims = !alias_wrap;
                            let record_free_tags = if reclaims {
                                self.resource_union_record_free_tags(type_)
                            } else {
                                Vec::new()
                            };
                            let io_buffer_tags = self.resource_union_io_buffer_tags(type_);
                            let reclaim_slot = (reclaims
                                && (!record_free_tags.is_empty()
                                    || !io_buffer_tags.is_empty()
                                    || type_.state().is_some()))
                            .then(|| {
                                let slot = self.allocate_stack_object(
                                    &format!("resource_union_reclaim_{name}"),
                                    8,
                                );
                                self.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), slot));
                                self.owned_value_slots.push(slot);
                                slot
                            });
                            // bug-1051: as the concrete arm, for the closed default
                            // record the temp's default box names.
                            let default_record_slot = (value.is_none()
                                && !self.resource_union_record_free_tags(type_).is_empty())
                            .then(|| self.remember_trap_default_record(name, stack_offset, 8));
                            let cleanup = ResourceUnionCleanup {
                                name: name.clone(),
                                variants,
                                state_type: type_.state(),
                                io_buffer_tags,
                                record_free_tags,
                                closes_variant: !alias_wrap,
                                owner_flag_slot,
                                default_record_slot,
                                scope_depth,
                                reclaim_slot,
                            };
                            if reclaim_slot.is_some() {
                                self.reclaiming_resources
                                    .push(ActiveCleanup::ResourceUnion(cleanup.clone()));
                            }
                            self.active_cleanups
                                .push(ActiveCleanup::ResourceUnion(cleanup));
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
                                    // bug-560: if an in-place self-append targets this
                                    // name, its block carries capacity headroom the
                                    // `byteLength` header does not record, and the tight
                                    // drop would orphan it on every scope exit.
                                    capacity_slot: self.string_capacity_slot_for(name, type_),
                                    loop_alias_slot: None,
                                    result_wrapper: None,
                                },
                            ));
                            self.owned_value_slots.push(stack_offset);
                        } else if let Some(wrapper) = result_wrapper {
                            // bug-593: the inline-`TRAP` wrapper of a non-flat `T` —
                            // see `result_wrapper` above. The slot is zero-initialized
                            // with the owned values, so a bind that never ran reads 0.
                            self.active_cleanups.push(ActiveCleanup::OwnedValue(
                                OwnedValueCleanup {
                                    type_: type_.clone(),
                                    stack_offset,
                                    capacity_slot: None,
                                    loop_alias_slot: None,
                                    result_wrapper: Some(wrapper),
                                },
                            ));
                            self.owned_value_slots.push(stack_offset);
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
                        // bug-806: a module-level binding is the outermost scope
                        // there is. Every resource a store puts into one is raised
                        // to the module instance, which closes it at instance
                        // teardown — before the store when the stored operands are
                        // named values, after it (over the whole global) when an
                        // operand is a call whose resources have no name.
                        let walk_after =
                            self.emit_module_store_float(name, type_, value.as_ref())?;
                        self.lower_store_global(op, name, type_, value.as_ref())?;
                        if walk_after {
                            self.emit_module_float_global(name)?;
                        }
                        // bug-1030: a module-level `RES g AS T STATE S` owns its
                        // state as a local owner does, so the store
                        // default-initializes the payload the same way (a
                        // resource that arrives carrying one keeps it).
                        let declared = self.global_value(name)?.type_.clone();
                        if let Some(state_type) = declared.state() {
                            let address = self.load_global_address(name)?;
                            let stored = self.allocate_register();
                            self.emit(abi::load_u64(stored, address, 0));
                            let slot = self.spill_to_slot("module_state_init", stored);
                            self.emit_resource_state_init(slot, &state_type, &declared)?;
                        }
                    }
                    NirOp::Assign { name, value } => {
                        // Rule 4: whether this store can carry a resource owned
                        // inside the container's scope (`record_ownership::
                        // StoreFloats`); the store raises what it stores.
                        self.statement_temps.assign_target = Some(name.clone());
                        self.statement_temps.assign_floats = self
                            .store_floats
                            .stores
                            .contains(&(value as *const NirValue as usize));
                        self.statement_temps.item_floated = false;
                        // bug-689: an element-bound binding's update, its write-back
                        // (the element already holds the value it writes), and the
                        // single-expression update. None changes the list's length, so
                        // the facts below stay true.
                        let key = crate::codegen::engine::analysis::last_use::op_key(op);
                        if self.element_updates.write_backs.contains(&key) {
                            return Ok(());
                        }
                        if let Some(element) = self.element_updates.updates.get(&key).cloned() {
                            return self.lower_element_update(&element, Some(key), value);
                        }
                        if self.try_single_expression_element_update(op, value)? {
                            return Ok(());
                        }
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
                            self.update_promoted_float(d, &result);
                            // Observation boundary: the promoted accumulator's
                            // `d`-register holds the named local's value after
                            // the update, so check it there — the FP-domain
                            // variant keeps the store-to-slot peephole-foldable
                            // (plan-16/plan-17).
                            self.observe_promoted_float(value, d)?;
                            return Ok(());
                        }
                        let (stack_offset, by_ref, local_type) = {
                            let local = self.locals.get(name).ok_or_else(|| {
                                format!("native code assignment unknown local '{name}'")
                            })?;
                            (local.stack_offset, local.by_ref, local.type_.clone())
                        };
                        // `name = f(name, …)` on a uniquely owned `MUT` local
                        // mutates the live block in place when an arm of
                        // `SELF_UPDATE_ARMS` recognises `f` (plan-142-A, site S1):
                        // the arm updates the slot, so skip the general
                        // reassignment path entirely. A by-ref capture (S9,
                        // plan-142-G) mutates the parent's block through the
                        // reference.
                        let dest = if by_ref
                            && (is_self_update_call(value, name)
                                // plan-146-G: `s = s & t` is a `Binary`, not a
                                // call, and it is an arm at S9 now too.
                                || string_self_append_operands_of(value, &|root| {
                                    matches!(root, NirValue::Local(local) if local == name)
                                })
                                .is_some())
                        {
                            InPlaceDest::Ref {
                                ref_slot: stack_offset,
                                block_slot: self.allocate_stack_object("su_ref_block", 8),
                            }
                        } else {
                            InPlaceDest::Direct { slot: stack_offset }
                        };
                        let site = SelfUpdateSite {
                            name,
                            type_: local_type.clone(),
                            dest,
                            by_ref,
                            field: None,
                        };
                        // plan-145-B: `name = WITH name { f := op(name.f, …) }` — a
                        // self-update of one of the record local's fields — is a
                        // field site of the same seam.
                        let field_site = self
                            .field_self_update_site(FieldContainer::Record { local: name }, value)
                            .map(|(field, field_type, update)| {
                                (
                                    SelfUpdateSite {
                                        name,
                                        type_: field_type,
                                        // plan-145-H: a by-ref capture's slot
                                        // holds the parent slot's address.
                                        dest: if by_ref {
                                            InPlaceDest::RefField {
                                                ref_slot: stack_offset,
                                                field_index: field.field_index,
                                                path: field.path_indices(),
                                            }
                                        } else {
                                            InPlaceDest::Inlined {
                                                block_slot: stack_offset,
                                                field_index: field.field_index,
                                                path: field.path_indices(),
                                                write_back: WriteBack::None,
                                            }
                                        },
                                        by_ref,
                                        field: Some(field),
                                    },
                                    update,
                                )
                            });
                        // plan-145-C: a `WITH` whose every update is a scalar or a
                        // pointer field is stored field by field in the record's
                        // own block (the `STATE` Layer 1 routine, for a local).
                        let in_place = self.try_inplace_self_update(&site, value)?
                            || self.try_inplace_scalar_fields(
                                FieldContainer::Record { local: name },
                                value,
                            )?
                            || match &field_site {
                                Some((site, update)) => {
                                    self.try_inplace_self_update(site, update)?
                                }
                                None => false,
                            }
                            || self.try_inplace_mixed_with(
                                FieldContainer::Record { local: name },
                                value,
                                Some(stack_offset),
                                by_ref,
                            )?;
                        if !in_place {
                            // Reassignment installs a fresh independent block; the old
                            // block remains owned by this binding's scope-drop free
                            // (the slot is overwritten with the new owner). Deep-copy
                            // an aliasing source so the binding stays unaliased.
                            let result = self.lower_value_owned(value)?;
                            // bug-1026: an aliasing `RECOVER` into a resource-union
                            // inline-`TRAP` temp (`RECOVER other`, `RECOVER b.h`,
                            // `RECOVER collections::getOr(…)`) lends the temp a
                            // record and hands it a COPY of the box: the address the
                            // value names is another binding's box, or lies inside a
                            // record's or a collection's block, and none of those
                            // need outlive the temp. A borrowed element that arrives
                            // as the trapped call's own `resultValue` was copied when
                            // its `Result` was built. The temp's drop frees the copy
                            // and closes nothing (the flag written below is clear).
                            let result = if self.trap_owner_flags.contains_key(name)
                                && self.resource_union_cleanup(&local_type).is_some()
                                && !matches!(value, NirValue::ResultValue { .. })
                                && !self.trap_ownership.store_is_owned(value)
                            {
                                let copied = self.materialize_inline_value_in_arena(
                                    &local_type,
                                    &result.location,
                                )?;
                                ValueResult {
                                    location: Operand::from(copied),
                                    ..result
                                }
                            } else {
                                result
                            };
                            // Observation boundary: a `Float` reassignment must
                            // be finite (plan-17).
                            self.observe_float(value, &result)?;
                            // bug-755: the scope of the binding's registered close, when
                            // it has one. A binding with no close is a reference: it
                            // claims nothing, and the old record's owner check below can
                            // only match a record this very scope owns.
                            let registered_depth = self.active_cleanups.iter().rev().find_map(
                                |cleanup| match cleanup {
                                    ActiveCleanup::Resource(resource) if resource.name == *name => {
                                        Some(resource.scope_depth)
                                    }
                                    _ => None,
                                },
                            );
                            let assign_slot = if let Some(symbol) =
                                self.resource_cleanup_symbol(&result.type_)
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
                                    // The value a store replaces is closed here
                                    // and its record kept: the binding's scope has
                                    // not exited, and another cleanup of it may
                                    // still read the record.
                                    frees_record: false,
                                    moved_record_only: false,
                                    owner_flag_slot: self.trap_owner_flags.get(name).copied(),
                                    default_record_slot: self
                                        .trap_default_records
                                        .get(name)
                                        .copied(),
                                    scope_depth: registered_depth
                                        .unwrap_or_else(|| self.current_scope_depth()),
                                    reclaim_slot: None,
                                };
                                self.emit_resource_cleanup_call(&cleanup)?;
                                Some(slot)
                            } else if let Some(cleanup) = self
                                .resource_union_cleanup(&local_type)
                                .and_then(|_| self.resource_union_binding_cleanup(name))
                            {
                                // bug-1037: a resource-union binding that is assigned
                                // drops the value it held first, as the concrete arm
                                // above does. The only such binding is the inline-`TRAP`
                                // desugar's `$trap_valN` (a `RES` binding is not
                                // assignable), whose bind materializes a closed default
                                // `{tag, record}` box and record that nothing else
                                // names: storing the call's result over it kept 16 +
                                // 112 bytes per evaluation. The drop is the binding's
                                // own registered one, so it answers for the old value
                                // exactly as the scope exit would have: the owner flag
                                // still describes that value (it is rewritten below,
                                // after the store), a lent value loses only the temp's
                                // copy of its box, and the slot is nulled. The new value
                                // is spilled across it because the drop calls the close
                                // op and `arena_free`.
                                let slot =
                                    self.allocate_stack_object("resource_union_assign_value", 8);
                                self.emit(abi::store_u64(
                                    &result.location,
                                    abi::stack_pointer(),
                                    slot,
                                ));
                                // The binding's scope has not exited: the
                                // replaced value is closed and its record kept
                                // (the default record is freed by its own rule).
                                let cleanup = ResourceUnionCleanup {
                                    reclaim_slot: None,
                                    record_free_tags: Vec::new(),
                                    ..cleanup
                                };
                                self.emit_resource_union_cleanup_call(&cleanup)?;
                                Some(slot)
                            } else if by_ref && self.is_freeable_flat_value(&result.type_) {
                                // plan-142-G: a reference local's reassignment frees the
                                // parent binding's previous block, read through the
                                // reference, exactly as the owner's own reassignment
                                // below would; it leaked before. Nothing in the parent
                                // still walks that block: a `forEach` over it walks a
                                // snapshot, and so does a `FOR EACH` whose body passes
                                // the binding by reference (operand_snapshot.rs).
                                let old_slot = self.allocate_stack_object("reassign_ref_old", 8);
                                let parent = self.allocate_register();
                                self.emit(abi::load_u64(
                                    &parent,
                                    abi::stack_pointer(),
                                    stack_offset,
                                ));
                                let old = self.allocate_register();
                                self.emit(abi::load_u64(&old, &parent, 0));
                                self.emit(abi::store_u64(&old, abi::stack_pointer(), old_slot));
                                let slot = self.allocate_stack_object("reassign_value", 8);
                                self.emit(abi::store_u64(
                                    &result.location,
                                    abi::stack_pointer(),
                                    slot,
                                ));
                                // plan-146-G: the owner's block may carry spare
                                // bytes an arm in this lambda left there, so the
                                // free is sized by the SHARED shadow — and the
                                // shadow is reset to 0 below, because the block
                                // being installed is a fresh tight one.
                                let shared = self.string_shadow_working_copy(name);
                                let (shared_slot, shared_holder) = match shared {
                                    Some((slot, holder)) => (Some(slot), Some(holder)),
                                    None => (None, None),
                                };
                                self.emit_owned_value_drop(&OwnedValueCleanup {
                                    type_: result.type_.clone(),
                                    stack_offset: old_slot,
                                    capacity_slot: shared_slot,
                                    loop_alias_slot: None,
                                    result_wrapper: None,
                                })?;
                                self.reset_shared_string_shadow(shared_holder);
                                Some(slot)
                            } else if !by_ref
                                && (self.is_freeable_flat_value(&result.type_)
                                    || self.owns_graph(&result.type_))
                                // bug-430: a live `FOR EACH x IN name.field` holds an
                                // alias into this record's block; freeing the block
                                // mid-loop is a use-after-free (this rebuild is the
                                // fallback the in-place record grow declines to for
                                // exactly this reason). Leak the old block instead.
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
                                // bug-1036: a live `FOR EACH` that walks this
                                // binding's own block still reads it, so that one
                                // block is handed to the loop, which frees it when
                                // it ends (`emit_defer_walked_block`); any other
                                // block the binding held is freed here.
                                let slot = self.allocate_stack_object("reassign_value", 8);
                                self.emit(abi::store_u64(
                                    &result.location,
                                    abi::stack_pointer(),
                                    slot,
                                ));
                                let walked = self.emit_defer_walked_block(name, stack_offset);
                                self.emit_owned_value_drop(&OwnedValueCleanup {
                                    type_: result.type_.clone(),
                                    stack_offset,
                                    // bug-560: this drop runs BEFORE the new value is
                                    // stored and before `reset_string_capacity_shadow`,
                                    // so the shadow still describes the block being
                                    // freed — which is the one an earlier self-append
                                    // grew. Freeing it tight orphaned the headroom on
                                    // every reassignment.
                                    capacity_slot: self
                                        .string_capacity_slot_for(name, &result.type_),
                                    loop_alias_slot: None,
                                    result_wrapper: None,
                                })?;
                                if let Some(walked) = walked {
                                    self.emit(abi::label(&walked));
                                }
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
                                    location: Operand::from(register),
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
                                self.store_value_at(&store_value, &slot_pointer, 0);
                            } else {
                                self.store_value_at(
                                    &store_value,
                                    abi::stack_pointer(),
                                    stack_offset,
                                );
                                // bug-755: the binding's scope claims the new record when
                                // it is unowned, exactly as the bind does.
                                if let (Some(depth), Some(_)) = (registered_depth, assign_slot) {
                                    if self.resource_cleanup_symbol(&result.type_).is_some() {
                                        self.emit_resource_claim_at(stack_offset, depth)?;
                                    }
                                }
                                // A resource-union binding whose drop closes its active
                                // variant claims the new variant's record the same way:
                                // the drop is owner-checked, so an unclaimed record (the
                                // inline-`TRAP` desugar fills `$trap_valN` by assignment)
                                // would never be closed.
                                if let Some(depth) =
                                    self.active_cleanups.iter().rev().find_map(|cleanup| {
                                        match cleanup {
                                            ActiveCleanup::ResourceUnion(union)
                                                if union.name == *name && union.closes_variant =>
                                            {
                                                Some(union.scope_depth)
                                            }
                                            _ => None,
                                        }
                                    })
                                {
                                    self.emit_resource_union_claim_at(stack_offset, depth)?;
                                }
                                // bug-805: a container assigned a call result adopts the
                                // resources the callee handed out, exactly as a bind of
                                // one does (`call_assigned_owner_containers` gave it its
                                // owned-lists at its bind).
                                if self.adopts_from_trapped_call(value)
                                    && (self.owned_list_heads.keys().any(|(list, _)| list == name)
                                        || self.dynamic_owned_list(name).is_some())
                                {
                                    self.emit_owned_list_seed_from_collection(
                                        name,
                                        stack_offset,
                                        &local_type,
                                    )?;
                                }
                            }
                            // bug-648: record whether this store handed a `Mixed` temp
                            // an owned value — strictly after the old value's drop
                            // above, which must read the flag of the value it releases.
                            if let Some(&flag) = self.trap_owner_flags.get(name) {
                                if self.trap_ownership.store_is_owned(value) {
                                    let owned = self.temporary_vreg();
                                    self.emit(abi::move_immediate(&owned, "Integer", "1"));
                                    self.emit(abi::store_u64(&owned, abi::stack_pointer(), flag));
                                } else {
                                    self.emit(abi::store_u64(
                                        abi::ZERO,
                                        abi::stack_pointer(),
                                        flag,
                                    ));
                                }
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
                        if self
                            .try_inplace_scalar_fields(FieldContainer::State { resource }, value)?
                        {
                            return Ok(());
                        }
                        // bug-430 Layer 2, plan-145-B: `s.state.f = op(s.state.f, …)`
                        // on a collection field is a field site of the self-update
                        // seam; the arm that recognises `op` mutates the field
                        // inside the existing STATE block and republishes it.
                        if let Some((field, field_type, update)) =
                            self.field_self_update_site(FieldContainer::State { resource }, value)
                        {
                            let site = SelfUpdateSite {
                                name: resource,
                                type_: field_type,
                                dest: InPlaceDest::StateField {
                                    resource: resource.clone(),
                                    field_index: field.field_index,
                                    path: field.path_indices(),
                                },
                                by_ref: false,
                                field: Some(field),
                            };
                            if self.try_inplace_self_update(&site, update)? {
                                return Ok(());
                            }
                        }
                        // plan-145-C: a collection field and scalar fields in one
                        // `WITH` over the payload.
                        if self.try_inplace_mixed_with(
                            FieldContainer::State { resource },
                            value,
                            None,
                            false,
                        )? {
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
                        // bug-644: this arm REBUILDS the whole STATE record into a
                        // fresh block and republishes it through the record's
                        // `RESOURCE_OFFSET_STATE` slot. The block it displaces has no
                        // other owner — every alias of the handle reads the slot
                        // (§15), so once the slot names the new block nothing can
                        // reach the old one — and it used to leak, one STATE record
                        // per `s.state.field = v` that is not an in-place store
                        // (32 B an iteration for a `String` field, which never takes
                        // the inline-scalar fast path above).
                        //
                        // Freeing it is gated on two things. The size must be
                        // derivable exactly as the scope-exit drop derives it
                        // (`emit_free_resource_state_block` →
                        // `emit_inlined_block_size_from_ptr_slot`); a STATE type that
                        // sizer cannot answer for keeps the old, leaking behavior
                        // instead of turning a program that builds today into a
                        // codegen error. And no live `FOR EACH` may be walking this
                        // resource's STATE: such a loop snapshots an alias INTO the
                        // block, so freeing it mid-iteration is a use-after-free —
                        // the same reason the in-place grow declines for
                        // `for_each_iterable_state_fields` (bug-430). Leak it there,
                        // matching the reassign arm's `for_each` carve-outs.
                        let replaced_state_type = resource_type.state().filter(|state| {
                            (*state == ParameterType::String
                                || self.type_model.record_fields.contains_key(state)
                                || self.union_is_data(state)
                                || matches!(state, ParameterType::ResultOf(_))
                                || typed_is_collection_type(state))
                                && !self
                                    .for_each_iterable_state_fields
                                    .iter()
                                    .any(|(res, _)| res == resource)
                                && !self
                                    .for_each_iterable_record_fields
                                    .iter()
                                    .any(|(base, _)| base == resource)
                                && !self.for_each_iterable_locals.iter().any(|n| n == resource)
                        });
                        // plan-134-E: the replacement is stored into the resource.
                        // bug-678: the resource keeps the POINTER, so it must own the
                        // block — an aliasing source (`h.state = r`) is copied, exactly
                        // as a binding's reassignment copies one. The field store
                        // (`lower_value_stored_field`) returned `r`'s own block, which
                        // `r` and the resource then both freed, and which an in-place
                        // update of `r` changed under the resource.
                        let result = self.lower_value_owned(value)?;
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
                        // bug-644: capture the outgoing block BEFORE the publish
                        // overwrites the slot. It is read here rather than before the
                        // operands on purpose: an operand that itself reaches a STATE
                        // assignment (the `G25` shape) has already republished and
                        // freed by now, so the slot names the block this statement is
                        // actually displacing and neither block is freed twice.
                        let replaced_slot = replaced_state_type.as_ref().map(|_| {
                            let slot = self.allocate_stack_object("state_assign_replaced", 8);
                            let previous = self.allocate_register();
                            self.emit(abi::load_u64(&previous, &ptr, RESOURCE_OFFSET_STATE));
                            self.emit(abi::store_u64(&previous, abi::stack_pointer(), slot));
                            slot
                        });
                        let val = self.allocate_register();
                        self.emit(abi::load_u64(&val, abi::stack_pointer(), value_slot));
                        self.emit(abi::store_u64(&val, &ptr, RESOURCE_OFFSET_STATE));
                        // bug-644: reclaim the displaced block, AFTER the new one is
                        // published so no window exists where the slot names freed
                        // memory. Two run-time guards keep this to the one block that
                        // is definitively gone:
                        //   * a null STATE pointer is skipped — the first assignment
                        //     on a record whose `emit_resource_state_init` has not run
                        //     (or a resource handed in already drained) has nothing to
                        //     free, and `arena_free(0)` faults.
                        //     `emit_shallow_block_free` null-guards.
                        //   * old == published is skipped — an in-place store or an
                        //     `InlineGrow` realloc republishes THE SAME block, and
                        //     freeing it is a use-after-free of state the resource
                        //     still reads through `RESOURCE_OFFSET_STATE`. The two
                        //     in-place layers above return early, so this arm should
                        //     never see that, but the identity test costs two
                        //     instructions and is what makes the free unconditionally
                        //     safe rather than safe-by-reachability-argument.
                        if let (Some(state_type), Some(replaced_slot)) =
                            (replaced_state_type, replaced_slot)
                        {
                            let kept = self.label("state_assign_replaced_kept");
                            let previous = self.allocate_register();
                            self.emit(abi::load_u64(
                                &previous,
                                abi::stack_pointer(),
                                replaced_slot,
                            ));
                            let published = self.allocate_register();
                            self.emit(abi::load_u64(&published, abi::stack_pointer(), value_slot));
                            self.emit(abi::compare_registers(&previous, &published));
                            self.emit(abi::branch_eq(&kept));
                            self.emit_shallow_block_free(&state_type, replaced_slot)?;
                            self.emit(abi::label(&kept));
                        }
                    }
                    NirOp::Eval { value } => {
                        // bug-744: a statement discards its value, so a
                        // `Nothing` call here needs no `NOTHING` materialized.
                        self.value_discarded_armed = true;
                        let lowered = self.lower_value(value);
                        self.value_discarded_armed = false;
                        let lowered = lowered?;
                        // bug-755: a discarded call result closes what nobody owns —
                        // a record the callee produced or released — and leaves a
                        // handle the callee was passed and handed back to its owner.
                        if matches!(
                            value,
                            NirValue::Call { .. }
                                | NirValue::CallResult { .. }
                                | NirValue::RuntimeCall { .. }
                        ) {
                            self.emit_close_discarded_value(&lowered.location, &lowered.type_)?;
                        }
                    }
                    NirOp::Return { value } => {
                        // bug-567: the watermark travels with the statement so the
                        // return can free its own interior temps while the code is
                        // still reachable — before the `ret`, not after it.
                        self.emit_return_exit(value.as_ref(), Some(temp_watermark))?;
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
                            target.temp_depth,
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
                            target.temp_depth,
                        )?;
                    }
                    NirOp::ExitProgram { code } => {
                        self.emit_program_exit_value(code)?;
                    }
                    NirOp::Fail { error } => {
                        self.emit_error_value_exit(
                            error,
                            self.error_exit_destination(),
                            temp_watermark,
                        )?;
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
                        self.emit_branch_unless(&condition.location, &else_label, Some("ifFalse"));
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
                        // bug-845: a `d`-native `Float` scrutinee is stored from its
                        // FP register; a GP store of it wrote some other register's
                        // bits, so a `CASE 1.5` missed a scrutinee of `1.5`.
                        self.store_value_at(&matched, abi::stack_pointer(), matched_slot);
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
                                location: Operand::from(matched_register),
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
                        let condition = self.lower_loop_condition(condition)?;
                        self.emit_branch_unless(&condition, &end_label, None);
                        self.loop_stack.push(LoopLabels {
                            kind: *kind,
                            continue_label: loop_label.clone(),
                            exit_label: end_label.clone(),
                            cleanup_depth: self.active_cleanups.len(),
                            temp_depth: self.pending_temp_frees.len(),
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
                            temp_depth: self.pending_temp_frees.len(),
                        });
                        self.lower_loop_body(body)?;
                        self.loop_stack.pop();
                        self.emit(abi::label(&condition_label));
                        let condition = self.lower_loop_condition(condition)?;
                        self.emit(abi::compare_immediate(&condition, "0"));
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
                                capacity_slot: None,
                                loop_alias_slot: None,
                                result_wrapper: None,
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
            self.current_op_key = enclosing_op_key;
            self.resource_temp_exempt = enclosing_temp_exempt;
            result.map_err(|err| format!("{err} while lowering {}", nir_op_context(op)))?;
            // plan-134-F: the drop walker's symmetry probe — emits nothing unless the
            // build's environment names this local (`graph_drop.rs`).
            if let NirOp::Bind { name, .. } | NirOp::Assign { name, .. } = op {
                self.emit_test_graph_drop_hook(name)?;
            }
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
            // Every statement retires its own temps here. A fall-through statement
            // frees them; a control transfer has already dealt with them according
            // to its `TransferTemps` class, and `forget_pending_temps_to` checks
            // the two classes that claim to have nothing left (bug-567).
            // Rule 4: an assignment whose value can carry a resource the statement
            // owns stores it into a binding that outlives the statement, so every
            // resource the binding now reaches is raised to the binding's scope
            // before the statement's temporaries close what it still owns. Done
            // here, after the store, so it holds for every way an assignment is
            // lowered (a plain store, an in-place append or set), as the
            // module-level store walks its global after a call (bug-806).
            if let NirOp::Assign { name, value } = op {
                self.statement_temps.assign_target = None;
                self.emit_assigned_container_float(name, value)?;
            }
            match Self::transfer_temp_disposition(op) {
                TransferTemps::StatementScope => self.drop_pending_temps_to(temp_watermark)?,
                disposition => self.forget_pending_temps_to(temp_watermark, disposition)?,
            }
            self.reset_temporary_registers();
        }
        let scope_returns = self.current_block_returns();
        // bug-1053: the lists of this scope whose drain takes records, in the order
        // their drains ran; the records are freed after the scope's last cleanup.
        let mut reclaiming = Vec::new();
        while self.active_cleanups.len() > cleanup_scope_start {
            let cleanup = self
                .active_cleanups
                .pop()
                .expect("cleanup scope length already checked");
            if !scope_returns {
                match cleanup {
                    ActiveCleanup::Resource(cleanup) => {
                        self.emit_resource_cleanup_call(&cleanup)?;
                        if cleanup.reclaim_slot.is_some() {
                            reclaiming.push(ActiveCleanup::Resource(cleanup));
                        }
                    }
                    ActiveCleanup::ResourceUnion(cleanup) => {
                        self.emit_resource_union_cleanup_call(&cleanup)?;
                        if cleanup.reclaim_slot.is_some() {
                            reclaiming.push(ActiveCleanup::ResourceUnion(cleanup));
                        }
                    }
                    ActiveCleanup::OwnedList(cleanup) => {
                        self.emit_owned_list_drain(&cleanup)?;
                        if cleanup.reclaim_slot.is_some() {
                            reclaiming.push(ActiveCleanup::OwnedList(cleanup));
                        }
                    }
                    ActiveCleanup::OwnedValue(cleanup) => self.emit_owned_value_drop(&cleanup)?,
                }
            }
        }
        reclaiming.reverse();
        self.emit_owned_list_reclaims(&reclaiming)?;
        Ok(())
    }
}
