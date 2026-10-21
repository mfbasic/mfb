//! plan-176-U: `lower_bind_op`, the `NirOp::Bind` arm of the statement dispatch. Called from `lower_ops_inner` in
//! `engine/control/builder_control/statement_dispatch.rs`.

use super::*;

impl CodeBuilder<'_> {
    /// Lower one `NirOp::Bind` (`LET`/`MUT`/`RES`). Runs inside `lower_ops_inner`'s per-op closure,
    /// so its `?` is attributed to the op by `nir_op_context`.
    pub(super) fn lower_bind_op(
        &mut self,
        op: &NirOp,
        mutable: bool,
        name: &str,
        type_: &ParameterType,
        value: &Option<NirValue>,
        zero_slot: VirtualRegister,
    ) -> Result<(), String> {
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
        let by_ref_capture_slot = matches!(value, Some(NirValue::Capture { by_ref: true, .. }));
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
        let borrowed_capture = !mutable
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
            name.to_string(),
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
            if let Some((uses, payload, block)) = self.unboxed_trap_shape(name, type_, value) {
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
                        name.to_string(),
                        (l.clone(), self.enclosing_loop_reassigned.len()),
                    );
                }
            }
        }
        // Record the synthetic FOR-bound/step binds so the loop's
        // `end`/`step` (which arrive as `Local($for_endN)`) resolve.
        if name.starts_with("$for_end") || name.starts_with("$for_step") {
            if let Some(v) = value {
                self.for_bound_expr.insert(name.to_string(), v.clone());
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
                self.provable_index_locals.insert(name.to_string(), fact);
            }
        }
        // A `MATCH` variant binding (`UnionExtract`) is an alias
        // into the matched union's inlined variant block: the union
        // owns the data and frees it as one block on its own drop,
        // so the binding is neither deep-copied nor freed here.
        let aliases_union_variant = matches!(value, Some(NirValue::UnionExtract { .. }));
        // A raw thread-boundary result (a `thread` helper's block that
        // reaches this frame uncopied) is owned by the thread runtime /
        // block arena, not this scope, so it is neither zero-initialized
        // nor freed here. A `thread::receive`/`waitFor`/… value is the
        // call site's copy into this arena and is owned like any fresh
        // value (bug-622 A).
        let runtime_managed = value.as_ref().is_some_and(Self::value_is_runtime_managed);
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
        let rebound_borrow =
            self.apply_rebound(crate::codegen::engine::analysis::last_use::op_key(op));
        let scratch_copy = if rebound_borrow {
            None
        } else {
            self.record_temps.bindings.get(name).copied()
        };
        let is_borrow_get = (self.borrow_get_locals.contains(name) || scratch_copy.is_some())
            && self.is_freeable_flat_value(type_)
            && !matches!(type_, ParameterType::String);
        // plan-134-G: a recursive value's owner frees its graph — except a
        // borrowed `MATCH` view (plan-134-C), whose bind skips the graph copy
        // and so holds its source's graph without owning it.
        let borrowed_graph_view = self.owns_graph(type_) && self.store_is_borrowed_view();
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
            (ParameterType::ResultOf(payload), Some(NirValue::CallResult { .. }))
                if !owns_freeable_value
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
                && value
                    .as_ref()
                    .is_some_and(|value| self.value_yields_fresh_union_box(value, type_)));
        // Zero the slot before a (possibly fallible) initializer
        // runs. If the initializer traps before storing, the slot
        // stays null and the scope-drop free/close skips it instead of
        // touching an uninitialized pointer.
        if owns_freeable_value || result_wrapper.is_some() || owns_resource_slot || owns_box_only {
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
            self.trap_owner_flags.insert(name.to_string(), flag);
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
                        .insert(name.to_string(), (type_.clone(), lanes));
                } else {
                    let block = self.vector_value_as_block(result)?;
                    self.claim_pending_temp(&block);
                    self.store_value_at(&block, abi::stack_pointer(), stack_offset);
                    self.active_cleanups
                        .push(ActiveCleanup::OwnedValue(OwnedValueCleanup {
                            type_: type_.clone(),
                            stack_offset,
                            // A promoted-vector fallback block is never a
                            // `String`, so it never carries self-append
                            // headroom.
                            capacity_slot: None,
                            loop_alias_slot: None,
                            result_wrapper: None,
                        }));
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
                    Some((offset, size)) => self.copy_into_record_temp(r, offset, size)?,
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
                let copied = self.materialize_inline_value_in_arena(type_, &result.location)?;
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
                Operand::from(self.copy_flat_block(&ParameterType::String, &result.location)?)
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
                self.emit_owned_list_seed_from_collection(name, stack_offset, type_)?;
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
                                self.live_resource_aliases.insert(name.to_string(), src.clone());
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
                                name: name.to_string(),
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
                                self.live_union_wraps.insert(name.to_string(), src.clone());
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
                                name: name.to_string(),
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
        Ok(())
    }
}
