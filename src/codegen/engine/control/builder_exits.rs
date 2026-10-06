// --- codegen tier imports (migration) ---
use crate::arch::ops::CodeOp;
// plan-147-B (site S11): `RETURN f(x, …)` runs the same self-update seam as
// `x = f(x, …)` and then moves `x`'s block out.
use crate::codegen::collection::assign::inplace_dest::{InPlaceDest, WriteBack};
use crate::codegen::collection::assign::self_update::{
    returned_field_self_update_local, returned_self_update_local, FieldContainer, SelfUpdateSite,
};
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::value::builder_values::EscapingValue;
use crate::codegen::error::constants::*;
use crate::target::shared::abi;
use crate::target::shared::nir::*;
use crate::types::ParameterType;
impl CodeBuilder<'_> {
    pub(crate) fn emit_cleanup_sequence(&mut self) -> Result<(), String> {
        let cleanups = self.active_cleanups.clone();
        self.emit_cleanups(&cleanups)
    }

    /// The cleanups present at this function's top-level (trap-handler) scope.
    /// The trap body runs at the function's top-level scope, so an error routed
    /// to it stays inside the function and these locals remain in scope. Only
    /// cleanups belonging to inner blocks being exited by the jump to the trap
    /// are *fully* out of scope. `cleanup_scope_starts[0]` is the function body;
    /// `[1]`, when present, is the first nested block's entry depth — i.e. the
    /// function-level cleanup count. With no inner block open, every active
    /// cleanup is function-level.
    pub(crate) fn trap_cleanup_floor(&self) -> usize {
        self.cleanup_scope_starts
            .get(1)
            .copied()
            .unwrap_or(self.active_cleanups.len())
    }

    /// The cleanups to run when an error is routed to this function's trap
    /// handler. Inner-block locals being exited (`index >= floor`) are dropped
    /// like any other exit. A function-level (trap-shared) **owned arena value**
    /// is *not* dropped here: it stays live for the handler to read and is freed
    /// exactly once on the handler's own exit — dropping it here would
    /// double-free it. Trap-shared resources *are* still dropped here: their drop
    /// is idempotent (the handler's later drop is a harmless no-op) and
    /// propagating an error past them closes them as before. A thread view is one
    /// of them (plan-156-C): a handler that names it reads a closed view.
    pub(crate) fn trap_route_cleanups(&self) -> Vec<ActiveCleanup> {
        let floor = self.trap_cleanup_floor();
        self.active_cleanups
            .iter()
            .enumerate()
            .filter(|(index, cleanup)| {
                !(*index < floor && matches!(cleanup, ActiveCleanup::OwnedValue(_)))
            })
            .map(|(_, cleanup)| cleanup.clone())
            .collect()
    }

    /// Emit the scope-drop frees for `cleanups` (innermost/last first).
    pub(crate) fn emit_cleanups(&mut self, cleanups: &[ActiveCleanup]) -> Result<(), String> {
        for cleanup in cleanups.iter().rev() {
            match cleanup {
                ActiveCleanup::Resource(cleanup) => {
                    self.emit_resource_cleanup_call(cleanup)?;
                }
                ActiveCleanup::ResourceUnion(cleanup) => {
                    self.emit_resource_union_cleanup_call(cleanup)?;
                }
                ActiveCleanup::OwnedList(cleanup) => {
                    self.emit_owned_list_drain(cleanup)?;
                }
                ActiveCleanup::OwnedValue(cleanup) => {
                    self.emit_owned_value_drop(cleanup)?;
                }
            }
        }
        Ok(())
    }

    pub(crate) fn emit_cleanup_branch_to_depth(
        &mut self,
        target: &str,
        cleanup_depth: usize,
        temp_depth: usize,
    ) -> Result<(), String> {
        // bug-620: the branch skips the statement-end drop of every statement it leaves, so
        // their temps are freed here. In place: the fall-through path still frees them.
        self.emit_pending_temp_frees_in_place_from(temp_depth)?;
        let cleanups = self.active_cleanups[cleanup_depth..].to_vec();
        for cleanup in cleanups.iter().rev() {
            match cleanup {
                ActiveCleanup::Resource(cleanup) => self.emit_resource_cleanup_call(cleanup)?,
                ActiveCleanup::ResourceUnion(cleanup) => {
                    self.emit_resource_union_cleanup_call(cleanup)?
                }
                ActiveCleanup::OwnedList(cleanup) => self.emit_owned_list_drain(cleanup)?,
                ActiveCleanup::OwnedValue(cleanup) => self.emit_owned_value_drop(cleanup)?,
            }
        }
        self.emit(abi::branch(target));
        Ok(())
    }

    pub(crate) fn emit_program_exit_value(&mut self, code: &NirValue) -> Result<(), String> {
        let result = self.lower_value(code)?;
        self.emit(abi::move_register(abi::return_register(), &result.location));
        self.emit(abi::move_register(
            RESULT_VALUE_REGISTER,
            abi::return_register(),
        ));
        self.emit(abi::move_immediate(
            RESULT_TAG_REGISTER,
            "Integer",
            RESULT_PROGRAM_EXIT_TAG,
        ));
        self.emit(abi::move_immediate(
            RESULT_ERROR_MESSAGE_REGISTER,
            "Integer",
            "0",
        ));
        self.emit_current_result_exit(ExitDestination::Return)
    }

    /// The error exit of a call whose result is not trapped inline: free the statement's
    /// pending temporaries, then leave as [`Self::emit_current_result_exit`] does.
    ///
    /// A fresh block an argument or an earlier operand produced is owned by its statement
    /// and freed at the statement's end — which a failing call jumps past. `boom(n, [])`
    /// leaked the empty list (48 B) and `boomS(n, "a" & toString(n))` the concat (32 B) on
    /// every failure, and `json::parse` leaked 48 B per rejected document through
    /// `__json_parseArrayItems(bytes, nextIndex, [], depth)`. Only a call's exit may do
    /// this: a `FAIL`'s error adopts its message temp (`TransferTemps::AdoptedByTheCatcher`),
    /// and a callee's error is its own block.
    pub(crate) fn emit_call_error_exit(&mut self) -> Result<(), String> {
        let destination = self.error_exit_destination();
        if !self.pending_temp_frees.is_empty() {
            self.store_pending_current_result();
            self.emit_pending_temp_frees_in_place()?;
            self.load_pending_result_registers();
        }
        self.emit_current_result_exit(destination)
    }

    /// bug-841: an inline `TRAP` must not catch `EXIT PROGRAM`.
    ///
    /// Called by every inline-`TRAP` lowering right after it parks the trapped
    /// call's raw result registers in `tag_slot` / `value_slot`, before it splits
    /// on success vs. error. A program-exit result (tag `2`, the exit code in the
    /// value slot, no message) leaves this frame exactly as an untrapped failing
    /// call does ([`Self::emit_call_error_exit`]: the statement's temporaries, then
    /// the frame's cleanups, then a return — or, under a function-level `TRAP`,
    /// [`Self::route_current_result_to_trap`], which returns on this tag too).
    /// `mfb spec language control-flow`: "`EXIT PROGRAM` is not catchable by
    /// `TRAP`: at each returning frame it bypasses any handler". Before this, the
    /// tag reached the trapped-`Error` rebuild, which read the zeroed message
    /// register as a `String` pointer (SIGSEGV).
    pub(crate) fn emit_inline_trap_program_exit_passthrough(
        &mut self,
        tag_slot: usize,
        value_slot: usize,
    ) -> Result<(), String> {
        let not_exit = self.label("inline_trap_not_program_exit");
        let tag = self.temporary_vreg();
        self.emit(abi::load_u64(tag, abi::stack_pointer(), tag_slot));
        self.emit(abi::compare_immediate(tag, RESULT_PROGRAM_EXIT_TAG));
        self.emit(abi::branch_ne(&not_exit));
        self.emit(abi::move_immediate(
            RESULT_TAG_REGISTER,
            "Integer",
            RESULT_PROGRAM_EXIT_TAG,
        ));
        self.emit(abi::load_u64(
            RESULT_VALUE_REGISTER,
            abi::stack_pointer(),
            value_slot,
        ));
        self.emit(abi::move_immediate(
            RESULT_ERROR_MESSAGE_REGISTER,
            "Integer",
            "0",
        ));
        self.emit(abi::move_immediate(
            RESULT_ERROR_SOURCE_REGISTER,
            "Integer",
            "0",
        ));
        self.emit_call_error_exit()?;
        self.emit(abi::label(&not_exit));
        Ok(())
    }

    /// bug-695: the error exit of an inline raise (a failing conversion, a division by
    /// zero, an overflow, a bounds check — everything `emit_error_register_return`
    /// ends) that is not captured by an inline `TRAP`. It frees exactly what a failing
    /// call at the same point frees ([`Self::emit_call_error_exit`]): the statement's
    /// pending temporaries, then every active cleanup (§14.7 — an error edge is a scope
    /// edge). Before this, the return route was a bare `return_()` that leaked every
    /// owned local, and the trap route skipped the temporaries.
    ///
    /// Raise sites are dense (every checked `+`, every conversion, every bounds check),
    /// so the return route does not repeat the walk per site — that grew the byte-identity
    /// covers by 21% in total and `vector` by 44%. The site parks the `Result` in the
    /// pending slots, frees its own statement temps (they differ per statement), and
    /// branches to a landing pad shared by every site with the same cleanup stack. Pads
    /// chain by prefix (`emit_error_return_pad`), so a function has one drop per
    /// distinct cleanup-stack prefix. The trap route keeps its per-site walk.
    pub(crate) fn emit_inline_error_exit(&mut self) -> Result<(), String> {
        if matches!(self.error_exit_destination(), ExitDestination::Trap)
            || self.active_cleanups.is_empty()
        {
            return self.emit_call_error_exit();
        }
        self.store_pending_current_result();
        self.emit_pending_temp_frees_in_place()?;
        let cleanups = self.active_cleanups.clone();
        let previous_escaping = self.escaping_value_slot;
        // Same identity skip as `emit_current_result_exit`'s `Return` arm, so a pad
        // drops exactly what the per-site walk would have.
        self.escaping_value_slot = self.pending_result_slots.map(|slots| slots.value);
        let result = self.emit_error_return_pad(&cleanups);
        self.escaping_value_slot = previous_escaping;
        result
    }

    /// Branch to the error-return landing pad for `cleanups`, creating it (and any
    /// missing pad down its prefix chain) on first use. Entry contract: the error
    /// `Result` is in the pending result slots.
    fn emit_error_return_pad(&mut self, cleanups: &[ActiveCleanup]) -> Result<(), String> {
        let label = self.error_return_pad_label(cleanups)?;
        self.emit(abi::branch(&label));
        Ok(())
    }

    /// The label of the pad for `cleanups`. A new pad's code is emitted NOW — the
    /// drop reads this point's locals — into a side buffer, and held for
    /// [`Self::append_error_return_pads`] to place after the body. The pad for
    /// `[c0..ck]` drops `ck` and continues into the pad for `[c0..ck-1]`; the empty
    /// prefix's pad reloads the parked `Result` and returns.
    fn error_return_pad_label(&mut self, cleanups: &[ActiveCleanup]) -> Result<String, String> {
        let key = self.error_return_pad_key(cleanups);
        if let Some(label) = self.error_return_pads.get(&key) {
            return Ok(label.clone());
        }
        let successor = match cleanups.split_last() {
            Some((_, rest)) => Some(self.error_return_pad_label(rest)?),
            None => None,
        };
        let label = self.label("error_return_pad");
        self.error_return_pads.insert(key, label.clone());
        let body = std::mem::take(&mut self.instructions);
        self.emit(abi::label(&label));
        let emitted = if successor.is_some() {
            self.emit_cleanups(&cleanups[cleanups.len() - 1..])
        } else {
            self.load_pending_result_registers();
            self.emit(abi::return_());
            Ok(())
        };
        let code = std::mem::replace(&mut self.instructions, body);
        emitted?;
        self.error_return_pad_code.push(ErrorReturnPad {
            depth: cleanups.len(),
            label: label.clone(),
            successor,
            code,
        });
        Ok(label)
    }

    /// Append the held error-return pads after the function body, deepest prefix
    /// first, so a pad's successor (one shorter) always follows it: every branch
    /// into a pad (from the body) and between pads is forward. A pad falls through
    /// into its successor when that is the next pad placed, and branches otherwise.
    /// The body never falls into the first pad: it ends in its own return.
    pub(crate) fn append_error_return_pads(&mut self) {
        let mut pads = std::mem::take(&mut self.error_return_pad_code);
        // Stable: pads of equal depth keep their creation order (determinism).
        pads.sort_by(|a, b| b.depth.cmp(&a.depth));
        let next_labels: Vec<Option<String>> = pads
            .iter()
            .skip(1)
            .map(|pad| Some(pad.label.clone()))
            .chain(std::iter::once(None))
            .collect();
        for (pad, next) in pads.into_iter().zip(next_labels) {
            self.instructions.extend(pad.code);
            if let Some(successor) = pad.successor {
                if next.as_deref() != Some(successor.as_str()) {
                    self.emit(abi::branch(&successor));
                }
            }
        }
    }

    fn error_return_pad_key(&self, cleanups: &[ActiveCleanup]) -> Vec<ErrorReturnPadEntry> {
        cleanups
            .iter()
            .map(|cleanup| {
                let name = match cleanup {
                    ActiveCleanup::Resource(cleanup) => Some(&cleanup.name),
                    ActiveCleanup::ResourceUnion(cleanup) => Some(&cleanup.name),
                    ActiveCleanup::OwnedList(cleanup) => Some(&cleanup.name),
                    ActiveCleanup::OwnedValue(_) => None,
                };
                ErrorReturnPadEntry {
                    cleanup: cleanup.clone(),
                    local_slot: name
                        .and_then(|name| self.locals.get(name))
                        .map(|local| local.stack_offset),
                }
            })
            .collect()
    }

    pub(crate) fn emit_current_result_exit(
        &mut self,
        destination: ExitDestination,
    ) -> Result<(), String> {
        // A `Return` leaves the function and frees every live local. A `Trap`
        // jumps to the same-function handler, where function-level locals stay in
        // scope; `trap_route_cleanups` omits their owned arena values so the
        // handler can read them and free them once on its own exit (freeing them
        // here would double-free).
        let cleanups = match destination {
            ExitDestination::Return => self.active_cleanups.clone(),
            ExitDestination::Trap => self.trap_route_cleanups(),
        };
        if !cleanups.is_empty() {
            self.store_pending_current_result();
            // plan-59-D: only a `RETURN` carries a value out of this scope, so
            // only here is a cleanup allowed to skip on pointer identity. The
            // slot is the one `store_pending_current_result` just wrote, which is
            // live precisely across `emit_cleanups` (cleanups clobber every
            // caller-saved register, so the result must be parked anyway).
            //
            // Left `None` for `ExitDestination::Trap`: routing to a handler is
            // not an escape, and §15 requires the resource still be closed.
            let previous_escaping = self.escaping_value_slot;
            if matches!(destination, ExitDestination::Return) {
                self.escaping_value_slot = self.pending_result_slots.map(|slots| slots.value);
            }
            let result = self.emit_cleanups(&cleanups);
            self.escaping_value_slot = previous_escaping;
            result?;
            self.load_pending_result_registers();
        }
        match destination {
            ExitDestination::Return => self.emit(abi::return_()),
            ExitDestination::Trap => self.route_current_result_to_trap()?,
        }
        Ok(())
    }

    /// The exit of a `FAIL`. `enclosing_temps` is the pending-temp depth at the
    /// `FAIL` statement's start: the temps below it belong to the statements that
    /// enclose the `FAIL` (an `IF` condition's `longKey()` in
    /// `IF s = longKey() THEN FAIL …`), are freed at those statements' ends, and the
    /// `FAIL` jumps past every one of them — so it frees them here, as a failing
    /// call's exit does (`emit_call_error_exit`). The temps above it are the
    /// `FAIL`'s own and are the catcher's to adopt (`TransferTemps::AdoptedByTheCatcher`).
    /// bug-780: leaving them leaked the condition's block on every failure.
    pub(crate) fn emit_error_value_exit(
        &mut self,
        error: &NirValue,
        destination: ExitDestination,
        enclosing_temps: usize,
    ) -> Result<(), String> {
        // See `emit_current_result_exit`: a trap route keeps the function-level
        // locals' owned arena values live for the handler, freeing only the rest.
        let cleanups = match destination {
            ExitDestination::Return => self.active_cleanups.clone(),
            ExitDestination::Trap => self.trap_route_cleanups(),
        };
        if cleanups.is_empty() {
            return match destination {
                ExitDestination::Return => self.emit_direct_error_return(error, enclosing_temps),
                ExitDestination::Trap => {
                    self.emit_direct_error_route_to_trap(error, enclosing_temps)
                }
            };
        }
        self.store_pending_error_from_value(error)?;
        // The error is parked in the pending result slots, so the frees below may
        // clobber every result register; `load_pending_result_registers` restores them.
        self.emit_pending_temp_frees_in_place_below(enclosing_temps)?;
        self.emit_cleanups(&cleanups)?;
        self.load_pending_result_registers();
        match destination {
            ExitDestination::Return => self.emit(abi::return_()),
            ExitDestination::Trap => self.route_current_result_to_trap()?,
        }
        Ok(())
    }

    /// Lower a returned value as a caller-owned, standalone block. An aliasing
    /// source of a freeable flat type is deep-copied here (plan-02 Phase 8): the
    /// returned block must outlive this scope's frees and is owned/freed by the
    /// caller, so it cannot remain an alias into a local that is about to be
    /// freed. The bool is `already_standalone` — true when the result is a fresh
    /// standalone allocation (a copy made here) that must NOT be re-materialized;
    /// false for a fresh value or an alias of a non-flat type, which keep the
    /// existing inline-payload materialization. A returned thread/resource local
    /// is a move (never freeable-flat) and is handled by cleanup deactivation.
    pub(crate) fn lower_returned_value(
        &mut self,
        value: &NirValue,
        move_elided: bool,
    ) -> Result<(ValueResult, bool), String> {
        // Copy elision via ownership transfer (plan-25-C C1): `emit_return_exit`
        // has already removed this owned local's scope-drop free for this return
        // path, transferring its uniquely-owned block to the caller — so return the
        // existing block pointer directly (a standalone arena block) instead of
        // deep-copying it. Copy insertion (`lower_value_owned`) guarantees the block
        // has no live alias, so the move creates exactly one owner, never two.
        if move_elided {
            return Ok((self.lower_value(value)?, true));
        }
        // bug-623 B: a resource union whose box is already this function's alone
        // moves to the caller as-is. Reporting it standalone keeps the exit from
        // re-materializing a copy and orphaning the original 16 B box.
        if self.returned_resource_union_owns_box(value) {
            return Ok((self.lower_value(value)?, true));
        }
        if self.value_needs_owning_copy(value) {
            let lowered = self.lower_value(value)?;
            if self.is_freeable_flat_value(&lowered.type_) {
                // plan-86 K1: a pure parameter-passthrough function (`RETURN <param>`,
                // `current_returns_param_borrow`) returns the argument pointer uncopied.
                // The caller classifies our result as an aliasing source via the SAME
                // predicate (`call_returns_param_borrow`) and deep-copies it only at an
                // owning store, so copying here would be a redundant second copy. The
                // skip fires ONLY when the returned local owns no block (no `OwnedValue`
                // cleanup) — i.e. it is genuinely a borrow of the caller's argument, not
                // an owned local whose block scope-drop would free out from under the
                // returned pointer. That guard is both the soundness condition and a
                // fail-safe: any predicate false-positive falls through to the copy (at
                // worst a missed elision, never a use-after-free).
                if self.current_returns_param_borrow {
                    if let NirValue::Local(name) = value {
                        let owns_block = self.locals.get(name).is_some_and(|local| {
                            let stack_offset = local.stack_offset;
                            self.active_cleanups.iter().any(|cleanup| {
                                matches!(cleanup, ActiveCleanup::OwnedValue(c)
                                    if c.stack_offset == stack_offset)
                            })
                        });
                        if !owns_block {
                            return Ok((lowered, true));
                        }
                    }
                }
                let copied = self.copy_flat_block(&lowered.type_, &lowered.location)?;
                return Ok((
                    ValueResult {
                        origin: None,
                        type_: lowered.type_,
                        location: Operand::from(copied),
                        text: lowered.text,
                    },
                    true,
                ));
            }
            // plan-134-D: a recursive value returned from an aliasing source (a field, a
            // parameter, a global) gets an independent graph from the walker, so the
            // caller never owns a graph the callee's source still reaches — unless the
            // return is the source's last read (plan-134-C), which is a move.
            if self.needs_graph_copy(&lowered.type_) && !self.store_is_last_use(value) {
                // The same parameter-passthrough borrow as the flat branch above: the
                // caller copies at its own owning store, so copying here too would be a
                // second copy. plan-134-G: and with the flat branch's guard — a recursive
                // local now owns an `OwnedValue` cleanup, so only a local that owns no
                // block is the caller's argument.
                if self.current_returns_param_borrow {
                    if let NirValue::Local(name) = value {
                        let owns_block = self.locals.get(name).is_some_and(|local| {
                            let stack_offset = local.stack_offset;
                            self.active_cleanups.iter().any(|cleanup| {
                                matches!(cleanup, ActiveCleanup::OwnedValue(c)
                                    if c.stack_offset == stack_offset)
                            })
                        });
                        if !owns_block {
                            return Ok((lowered, true));
                        }
                    }
                }
                let copied = self.copy_value_to_current_arena(&lowered.type_, &lowered.location)?;
                return Ok((
                    ValueResult {
                        origin: None,
                        type_: lowered.type_,
                        location: Operand::from(copied),
                        text: lowered.text,
                    },
                    true,
                ));
            }
            // plan-134-G: a return that is the owning source's last read moves the graph out.
            // plan-134-H: the moved graph is already its own block — a local's, or a record's
            // pointer field's — so it is reported standalone. Reporting it `false` made the
            // exit re-materialize a data union's top block and orphan the moved one: 32 B per
            // `json::parse` (`return parsed.value`).
            if self.owns_graph(&lowered.type_) && self.store_is_last_use(value) {
                self.release_moved_source(value, &lowered, false)?;
                return Ok((lowered, true));
            }
            return Ok((lowered, false));
        }
        let lowered = self.lower_value(value)?;
        // bug-536 shape A: `RETURN <RecordConstructor>` / `RETURN <call returning
        // a record or union>` leaked the fresh block, 64 B per call.
        //
        // The value is not an aliasing source, so it fell straight through with
        // `already_standalone = false`. `store_pending_success_result` then read
        // that as "an alias or an inline payload" and promoted it into a SECOND
        // arena block with `materialize_inline_value_in_arena` — and the RETURN
        // statement's `clear_pending_temps_to` discarded the FIRST block's
        // statement-scope free, on the reasoning that "a returned temp is moved to
        // the caller". True only while the temp *is* what is returned; after the
        // re-materialisation it is not, and nothing else ever frees it. Measured on
        // `FUNC mkLit(i) AS Plain / RETURN Plain[i, i]`: 25 MB at 400 000 calls,
        // 50 MB at 800 000. `RETURN r` (a local) was immune because
        // `plan_returned_move` moves the block; `RETURN [list literal]` because a
        // collection has no inline payload size.
        //
        // A registered pending temp is by construction a **fresh standalone arena
        // block** — `register_pending_temp` requires exactly that
        // (`!value_needs_owning_copy`, freeable-flat, not runtime-managed, not a
        // borrowed `get`). So claim it, the way `lower_value_owned` claims one for
        // a binding, and report it standalone: one block, one owner, and the
        // re-materialisation copy disappears with the leak because it was always
        // redundant. This is the same shape as the plan-25-C C1 `move_elided` path
        // just above, reached for a fresh temp instead of an owned local.
        //
        // The tail-entry identity test is `claim_pending_temp`'s own: the outermost
        // node's temp is the most recently registered, so a match on its location
        // is precise.
        //
        // bug-783: a register-native vector has no block yet, so it registered no
        // temp. Its materialized block is fresh and registered as one, so claim it
        // the same way — otherwise the exit copies it a second time and frees the
        // first (two allocations for one returned vector).
        if Self::is_vector_native(&lowered) {
            let block = self.vector_value_as_block(lowered)?;
            self.claim_pending_temp(&block);
            return Ok((block, true));
        }
        if self
            .pending_temp_frees
            .last()
            .is_some_and(|temp| temp.location == lowered.location)
        {
            self.claim_pending_temp(&lowered);
            return Ok((lowered, true));
        }
        // bug-536 shape B-2: reaching here means lowering could NOT establish that
        // the value is a fresh, solely-owned block — it is not a move-elided owned
        // local, not an aliasing source or static string (both copied above), and no
        // pending temp was registered for it. For a bare `String` that is exactly the
        // set the caller cannot classify either: `RETURN toString(s)` hands back the
        // *argument* (`toString`'s `String` arm is the identity), and an unopted
        // native producer's block has no provenance mark.
        //
        // `function_returns_fresh_string` promised every caller a solely-owned block,
        // so deliver it here rather than weakening the promise — one copy on exactly
        // the return sites whose freshness is unprovable, and none on the ones the
        // three arms above already made fresh (an owned local moves, a literal or a
        // parameter is already copied, a marked native producer's temp is claimed).
        // The alternative — classifying per return site at the caller — would need a
        // seed set of "fresh" native targets, which is the audited call-target
        // allowlist bug-536 rejected as design 2.
        if self.current_returns_fresh_string && lowered.type_ == ParameterType::String {
            let copied = self.copy_flat_block(&lowered.type_, &lowered.location)?;
            return Ok((
                ValueResult {
                    origin: None,
                    type_: lowered.type_,
                    location: Operand::from(copied),
                    text: lowered.text,
                },
                true,
            ));
        }
        Ok((lowered, false))
    }

    /// Plan a return-value copy elision (plan-25-C C1). A `RETURN <owned-local>`
    /// of a freeable-flat binding that owns its block moves the block to the
    /// caller instead of deep-copying it: the function exits at the return, so the
    /// local is dead, and dropping its scope-drop free leaves the caller the sole
    /// owner (one free total). When eligible this removes the binding's
    /// `OwnedValue` cleanup from the live set and returns the pre-removal cleanup
    /// stack, which `emit_return_exit` restores after the return is emitted — a
    /// sibling return path or the enclosing block's normal exit must still free the
    /// binding. Returns `None` (cleanup untouched, copy kept) when the move would
    /// be unsound:
    ///
    /// - A **parameter** or `by_ref` local is a reference into the caller's block (it
    ///   has no `OwnedValue` free), so returning its pointer without copying would
    ///   let the caller's binding double-free the source — the copy is load-bearing.
    /// - A **`FOR EACH` iterable** whose iterator still reads the block, or an
    ///   **address-taken** local an escaping closure env may reference, could leave a
    ///   dangling reader if the block moved out.
    /// plan-147-B: the read-only twin of [`Self::plan_returned_move`]'s gates for a
    /// `RETURN <local>` — "would the move be admitted?", asked WITHOUT removing the
    /// binding's cleanup.
    ///
    /// Site S11 needs the answer *before* it runs an arm: firing an in-place update
    /// and then discovering the block may not move would leave the value correct but
    /// the return copying, which is the cost S11 exists to remove. The gates are
    /// `plan_returned_move`'s, one for one — keep the two in step.
    pub(crate) fn returned_move_admits(&self, name: &str) -> bool {
        // A constant-folded `String` local has no block to move (bug-536 shape B-2).
        if self
            .static_string_value(&NirValue::Local(name.to_string()))
            .is_some()
        {
            return false;
        }
        // A `String` grown in place carries capacity only in THIS frame's shadow
        // slot, so its block cannot be handed to a caller that frees by
        // `byteLength` alone (bug-560).
        if self.string_capacity_slots.contains_key(name) {
            return false;
        }
        let Some(local) = self.locals.get(name) else {
            return false;
        };
        if local.by_ref {
            return false;
        }
        if self.for_each_iterable_locals.iter().any(|n| n == name)
            || self.address_taken_locals.contains(name)
        {
            return false;
        }
        // The authoritative ownership gate: only a binding that owns its block has a
        // live `OwnedValue` free at its slot. Parameters and aliases have none.
        let stack_offset = local.stack_offset;
        self.active_cleanups.iter().any(|cleanup| {
            matches!(cleanup, ActiveCleanup::OwnedValue(c) if c.stack_offset == stack_offset)
        })
    }

    /// plan-147-F: run `RETURN WITH r { f := OP(r.f, …) }`'s arm against the field in
    /// `r`'s own block — the FIELD form of S11.
    ///
    /// Composes two landed mechanisms: plan-145's field seam builds the
    /// `InPlaceDest::Inlined` destination, and letter B's move then hands `r`'s block
    /// out. The error path is plan-145's: every arm raises before its first write, so
    /// a failure cannot leave a half-updated record behind.
    fn try_returned_field_self_update(
        &mut self,
        name: &str,
        value: &NirValue,
    ) -> Result<bool, String> {
        if !self.returned_move_admits(name) {
            return Ok(false);
        }
        let Some(local) = self.locals.get(name) else {
            return Ok(false);
        };
        let (stack_offset, by_ref) = (local.stack_offset, local.by_ref);
        if by_ref {
            return Ok(false);
        }
        let container = FieldContainer::Record { local: name };
        let Some((field, field_type, update)) = self.field_self_update_site(container, value)
        else {
            return Ok(false);
        };
        let site = SelfUpdateSite {
            name,
            type_: field_type,
            dest: InPlaceDest::Inlined {
                block_slot: stack_offset,
                field_index: field.field_index,
                path: field.path_indices(),
                write_back: WriteBack::None,
            },
            by_ref: false,
            field: Some(field),
        };
        self.try_inplace_self_update(&site, update)
    }

    /// bug-784: `RETURN WITH p { … }` in a destination variant (`<base>$into<j>`,
    /// `p` parameter `j`): `p`'s slot holds the caller's destination — bytes inside
    /// a list element, never an allocation of their own — so the update is the
    /// field store over a span owner (`try_inplace_scalar_fields` with `p` as the
    /// element owner): every value is computed first, then stored into `p`'s bytes
    /// when the result keeps their exact size, and `p`'s address is returned.
    /// Otherwise nothing is written and a fresh block holding the same values is
    /// returned, as the base lowering would. The caller compares the result with
    /// the destination to know which (`element_into_call`). `false` = nothing was
    /// emitted and the ordinary return stands.
    fn try_into_return(
        &mut self,
        value: &NirValue,
        interior_temp_watermark: Option<usize>,
    ) -> Result<bool, String> {
        let Some(param) = self.element_updates.into_param.clone() else {
            return Ok(false);
        };
        let NirValue::WithUpdate { target, .. } = value else {
            return Ok(false);
        };
        if !matches!(target.as_ref(), NirValue::Local(name) if *name == param) {
            return Ok(false);
        }
        let Some(type_) = self.locals.get(&param).map(|local| local.type_.clone()) else {
            return Ok(false);
        };
        let result_slot = self.allocate_stack_object("into_result", 8);
        let saved_owner = self.element_updates.field_owner.replace(param.clone());
        self.element_updates.into_result_slot = Some(result_slot);
        let handled =
            self.try_inplace_scalar_fields(FieldContainer::Record { local: &param }, value);
        self.element_updates.field_owner = saved_owner;
        self.element_updates.into_result_slot = None;
        if !handled? {
            return Ok(false);
        }
        let hidden = format!("$intoresult{}", self.element_updates.hidden);
        self.element_updates.hidden += 1;
        self.locals.insert(
            hidden.clone(),
            LocalValue {
                type_,
                stack_offset: result_slot,
                constant: None,
                by_ref: false,
            },
        );
        // The result is the destination (owned by the caller's element) or a
        // fresh block this statement claimed: either way it leaves as it is.
        let result = self.emit_return_exit_inner(
            Some(&NirValue::Local(hidden.clone())),
            true,
            interior_temp_watermark,
        );
        self.locals.remove(&hidden);
        result.map(|()| true)
    }

    /// bug-766 C: run `RETURN WITH r { … }`'s field stores against `r`'s own block.
    ///
    /// `r` must be a binding whose block the return may move out
    /// (`returned_move_admits`: it owns the block, is not by-ref, not a `FOR EACH`
    /// iterable, not address-taken) — so nothing else can observe the stores, and
    /// the function exits right after them. `try_inplace_scalar_fields` evaluates
    /// every new value before its first write, so a failing value leaves `r` as
    /// it was (and a function-level `TRAP` that reads it sees the old value).
    /// `false` = nothing was emitted and the copying `WITH` stands.
    fn try_returned_field_store(&mut self, name: &str, value: &NirValue) -> Result<bool, String> {
        if !self.returned_move_admits(name) {
            return Ok(false);
        }
        self.try_inplace_scalar_fields(FieldContainer::Record { local: name }, value)
    }

    /// plan-147-B (site S11): run `RETURN f(x, …)`'s arm against `x`'s own block.
    ///
    /// `true` = an arm fired and `x`'s slot now holds the result, so the caller
    /// re-emits the return as `RETURN x`. `false` = nothing was emitted and the
    /// ordinary copying path stands.
    fn try_returned_self_update(&mut self, name: &str, value: &NirValue) -> Result<bool, String> {
        if !self.returned_move_admits(name) {
            return Ok(false);
        }
        let Some(local) = self.locals.get(name) else {
            return Ok(false);
        };
        let (stack_offset, local_type) = (local.stack_offset, local.type_.clone());
        // `returned_move_admits` has already declined a by-ref local, so the
        // destination is always the local's own slot.
        let site = SelfUpdateSite {
            name,
            type_: local_type,
            dest: InPlaceDest::Direct { slot: stack_offset },
            by_ref: false,
            field: None,
        };
        self.try_inplace_self_update(&site, value)
    }

    pub(crate) fn plan_returned_move(
        &mut self,
        value: Option<&NirValue>,
    ) -> Option<Vec<ActiveCleanup>> {
        let value = value?;
        let NirValue::Local(name) = value else {
            return None;
        };
        // bug-536 shape B-2: a local whose value is a compile-time `String`
        // constant is constant-FOLDED at every read — `lower_value` re-materialises
        // the rodata pointer (`adrp _mfb_str_N`) instead of loading the local's
        // slot. So there is no block to move: the move would hand the caller a
        // READ-ONLY constant and orphan the arena copy the binding actually made.
        //
        // That was already wrong before B-2 (the orphaned copy leaks 64 B per call,
        // and any owner that frees the result frees rodata), but it was invisible
        // while no caller ever freed a bare `String`. It stops being invisible the
        // moment `function_returns_fresh_string` licenses that free: measured on
        // `FUNC mkEmpty(i) AS String / MUT out AS String = "" / RETURN out` as an
        // immediate **SIGBUS**. Declining the move sends the value through
        // `value_needs_owning_copy`'s `copy_flat_block` instead, which copies the
        // rodata bytes into a fresh block — the caller gets a real arena block and
        // the binding's own cleanup still frees its copy.
        if self.static_string_value(value).is_some() {
            return None;
        }
        // bug-560: a `String` local grown by an in-place self-append carries
        // capacity headroom recorded ONLY in this frame's shadow slot. Moving the
        // block to the caller moves it out of the shadow's reach: the caller sizes
        // its own free from the `byteLength` header alone and under-frees by
        // `spare` on every call (measured at 32 KB per call on a 9 KB string, the
        // shape `__csv_decodeRange` and `__encoding_utf32Decode` both have).
        // Declining sends the return through `copy_flat_block`, which copies
        // exactly `byteLength` bytes into a tight block the caller CAN free, and
        // leaves this binding's own capacity-aware drop to reclaim the whole
        // buffer. Same remedy, and same reason, as the `static_string_value`
        // decline above: an unmovable carrier is copied, never handed on.
        if self.string_capacity_slots.contains_key(name) {
            return None;
        }
        let local = self.locals.get(name)?;
        if local.by_ref {
            return None;
        }
        let stack_offset = local.stack_offset;
        if self.for_each_iterable_locals.iter().any(|n| n == name)
            || self.address_taken_locals.contains(name)
        {
            return None;
        }
        // Only a binding that owns its block (has a live `OwnedValue` free at this
        // slot) can be moved; parameters and aliases have none, so this is the
        // authoritative ownership gate.
        let index = self.active_cleanups.iter().rposition(|cleanup| {
            matches!(cleanup, ActiveCleanup::OwnedValue(c) if c.stack_offset == stack_offset)
        })?;
        let saved = self.active_cleanups.clone();
        self.active_cleanups.remove(index);
        Some(saved)
    }

    /// `interior_temp_watermark` is the pending-temp depth this `RETURN` STATEMENT
    /// started at (bug-567). Everything the return expression registered above it
    /// and no owner claimed is an interior temp, and this is the last reachable
    /// point at which it can be freed — `clear_pending_temps_to` runs after the
    /// branch. `None` for the synthesized fall-off-the-end return, which lowers no
    /// expression and can register nothing.
    pub(crate) fn emit_return_exit(
        &mut self,
        value: Option<&NirValue>,
        interior_temp_watermark: Option<usize>,
    ) -> Result<(), String> {
        // Plan a return-copy elision (plan-25-C C1) before emitting: a movable
        // `RETURN <owned-local>` removes the binding's scope-drop free for this
        // path so the block moves to the caller uncopied. Restore the live cleanup
        // set afterward so a sibling return path or the block's normal exit still
        // frees the binding.
        // Resource-union alias class: `RETURN c` where `c` wraps a live concrete binding
        // `u` hands `u`'s record to the caller inside the union, so this path must not
        // close it — retire `u`'s cleanup for this return exactly as a returned concrete
        // local's is retired. The runtime identity skip cannot catch it: the escaping
        // value is the union box, not `u`'s record.
        //
        // Every removal a RETURN makes is PATH-LOCAL. `emit_return_exit_inner` retires
        // the returned local's close (a concrete resource, a resource union, a thread
        // handle, an owned-list container) with a `deactivate_*` call, which removes the
        // cleanup from the builder's compile-time list. That list is shared by the code
        // lowered AFTER this statement — a sibling `RETURN b` after `IF give THEN RETURN a
        // END IF`, the rest of a loop body whose `RETURN s` only fires on one iteration,
        // a later TRAP route, the block's own scope exit — none of which returned the
        // local, so each must still close it. Left permanent, the removal leaked one
        // descriptor and one record per call on those paths (`udp::bind` failing with
        // 7-707-0003 under a 128-fd limit). So a `RETURN <local>` restores the cleanup
        // list it started with once its exit is emitted, the same save/restore
        // `plan_returned_move` does for an owned value.
        //
        // No double close follows: this path's emitted cleanup sequence ran without
        // the returned local's cleanup, and code after a RETURN on this path is
        // unreachable, so the restored entry is only ever emitted on paths that did not
        // return the local. The caller stays its one closer on the path that did.
        // plan-147-B (site S11): `RETURN f(x, …)` where `x` is an owned local at its
        // last use. Today the copying `f` builds a second block and the return then
        // treats that block as the result, while `x`'s own block is dropped at exit —
        // two blocks where one would do. Instead run the SAME arm `x = f(x, …)` runs,
        // updating `x`'s block in place, and then re-enter this function as
        // `RETURN x`, which takes the ordinary `plan_returned_move` path.
        //
        // Reducing S11 to "S1, then the existing move" is deliberate: that pair is
        // already correct, including on the error path. plan-142's failure atomicity
        // means an arm raises before its first write, so a later operand's failure
        // cannot observe a half-updated `x` (plan-147-A §2.3 row S3), and the
        // re-entry gets the cleanup save/restore that keeps a SIBLING return path
        // still freeing `x`.
        //
        // The re-entry cannot recurse: its value is a `Local`, which
        // `returned_self_update_local` never matches.
        if let Some(value) = value {
            // bug-784: in a destination variant, `RETURN WITH <destination> { … }`
            // writes into the caller's destination when it can.
            if self.try_into_return(value, interior_temp_watermark)? {
                return Ok(());
            }
            if let Some(name) = returned_self_update_local(value) {
                let name = name.to_string();
                if self.try_returned_self_update(&name, value)? {
                    let moved = NirValue::Local(name);
                    return self.emit_return_exit(Some(&moved), interior_temp_watermark);
                }
            }
            // plan-147-F: the same reduction for a record's field.
            if let Some(name) = returned_field_self_update_local(value) {
                let name = name.to_string();
                if self.try_returned_field_self_update(&name, value)? {
                    let moved = NirValue::Local(name);
                    return self.emit_return_exit(Some(&moved), interior_temp_watermark);
                }
            }
            // bug-766 C: `RETURN WITH r { … }` whose updates are fields stored in
            // `r`'s own block — scalars, pointers, fixed-size inlined records, and
            // (bug-766 D) the last variable-width inlined field. The same
            // reduction: store the new values into `r`'s block exactly as
            // `r = WITH r { … }` does, then return `r` by the ordinary move. Inside
            // an owned variant this is what makes `RETURN WITH p { … }` consume the
            // handed-over parameter (`handover::returned_field_store_local`).
            if let NirValue::WithUpdate { target, .. } = value {
                if let NirValue::Local(name) = target.as_ref() {
                    let name = name.to_string();
                    if self.try_returned_field_store(&name, value)? {
                        let moved = NirValue::Local(name);
                        return self.emit_return_exit(Some(&moved), interior_temp_watermark);
                    }
                }
            }
        }
        let return_snapshot = match value {
            Some(NirValue::Local(_)) => Some(self.active_cleanups.clone()),
            _ => None,
        };
        if let Some(root) = value.and_then(|value| self.returned_union_alias_root(value)) {
            if let Some(index) = self
                .active_cleanups
                .iter()
                .rposition(|c| matches!(c, ActiveCleanup::Resource(r) if r.name == root))
            {
                self.active_cleanups.remove(index);
            }
        }
        let restore_cleanups = self.plan_returned_move(value);
        let result =
            self.emit_return_exit_inner(value, restore_cleanups.is_some(), interior_temp_watermark);
        if let Some(saved) = restore_cleanups {
            self.active_cleanups = saved;
        }
        if let Some(saved) = return_snapshot {
            self.active_cleanups = saved;
        }
        result
    }

    pub(crate) fn emit_return_exit_inner(
        &mut self,
        value: Option<&NirValue>,
        move_elided: bool,
        interior_temp_watermark: Option<usize>,
    ) -> Result<(), String> {
        let interior_temp_watermark =
            interior_temp_watermark.unwrap_or(self.pending_temp_frees.len());
        let lowered = if let Some(value) = value {
            // bug-744: a returned `Nothing` travels in no register (the paths below
            // skip it and the caller synthesizes `NOTHING`), so a returned
            // `Nothing` call keeps its `void` location. Only a `Nothing` result is
            // affected by the flag, so arming it for every return is exact.
            self.value_discarded_armed = true;
            let lowered = self.lower_returned_value(value, move_elided);
            self.value_discarded_armed = false;
            Some(lowered?)
        } else {
            None
        };
        let already_standalone = lowered
            .as_ref()
            .map(|(_, standalone)| *standalone)
            .unwrap_or(true);
        let result = lowered.map(|(result, _)| result);
        // Observation boundary: a returned `Float` becomes the caller's value
        // and must be finite (plan-17).
        if let (Some(value), Some(result)) = (value, result.as_ref()) {
            self.observe_float(value, result)?;
        }
        // The return value travels in a GPR (`RESULT_VALUE_REGISTER`), so a
        // `d`-native float is materialized into one first (ABI option (b),
        // plan-01 float-dnative §4.3), and a register-native vector into its block
        // pointer. Identity for every GP-native value.
        let result = match result {
            Some(result) => Some(self.materialize_value(result)?),
            None => None,
        };
        // bug-755: a record this function's scopes own leaves unowned, so the caller's
        // binding takes it; one owned further out keeps its owner. The release runs
        // before the cleanups below, which then skip it on the owner test. Spilled
        // because the walk reads it from a slot; it makes no call, so the value's
        // register is still live afterwards.
        let resource_temps = self.pending_temp_frees.iter().any(|temp| temp.resource);
        let result = match result {
            Some(mut result) if self.value_may_carry_owned_resources(&result.type_) => {
                let slot = self.spill_to_slot("returned_resources", &result.location);
                // bug-842: the statement's resource temporaries close before the
                // release, with every resource the returned value carries claimed
                // first, so a temporary the value hands out (`RETURN ident(mk())`)
                // is not closed under the caller; the release then frees the claim.
                if resource_temps {
                    self.emit_claim_returned_value(slot, &result.type_)?;
                    self.close_resource_temps_before_return()?;
                }
                self.emit_release_returned_value(slot, &result.type_)?;
                // bug-843: a function value's release calls its environment routine,
                // which clobbers every caller-saved register; the value is reloaded
                // from the slot the walk read it from. bug-842: so do the closes.
                if resource_temps || self.type_reaches_closure_leaf(&result.type_) {
                    let reloaded = self.allocate_register();
                    self.emit(abi::load_u64(&reloaded, abi::stack_pointer(), slot));
                    result.location = Operand::from(reloaded);
                }
                Some(result)
            }
            other => other,
        };
        if self.active_cleanups.is_empty() {
            let mut escaping = None;
            if let Some(result) = &result {
                if result.type_ != ParameterType::Nothing {
                    escaping = Some(
                        if !already_standalone
                            && self.inline_collection_payload_size(&result.type_).is_some()
                        {
                            Operand::from(self.materialize_inline_value_in_arena(
                                &result.type_,
                                &result.location,
                            )?)
                        } else {
                            result.location.clone()
                        },
                    );
                }
            }
            // bug-567: the escaping value is standalone by here — claimed, moved,
            // or copied — so the statement's remaining temps are interior and this
            // is the last reachable instruction slot before the `ret`. Emits
            // nothing when there are none.
            let escaping = self.drop_interior_temps_before_branch(
                interior_temp_watermark,
                match escaping {
                    Some(location) => EscapingValue::InRegister(location),
                    None => EscapingValue::None,
                },
            )?;
            if let Some(location) = &escaping {
                self.emit(abi::move_register(RESULT_VALUE_REGISTER, location));
            }
            self.emit(abi::move_immediate(
                RESULT_TAG_REGISTER,
                "Integer",
                RESULT_OK_TAG,
            ));
            self.emit(abi::return_());
            return Ok(());
        }
        self.store_pending_success_result(result.as_ref(), already_standalone)?;
        // bug-567, the same free on the cleanup-bearing path. The escaping value is
        // already parked — `store_pending_success_result` wrote
        // `pending_result_slots.value` — so nothing needs holding across the frees,
        // but that slot is still handed over so the interior frees carry the same
        // pointer-identity guard the fast path emits.
        let escaping_slot = self
            .pending_result_slots
            .map(|slots| slots.value)
            .map_or(EscapingValue::None, EscapingValue::InSlot);
        self.drop_interior_temps_before_branch(interior_temp_watermark, escaping_slot)?;
        if let Some(value) = value {
            if let NirValue::Local(name) = value {
                if result.as_ref().is_some_and(|result| {
                    crate::codegen::builtins::resource_close_function(&result.type_).is_some()
                }) {
                    self.deactivate_resource_cleanup(name);
                }
                // A returned resource union transfers ownership to the caller;
                // deactivate its tag-dispatched drop so the callee does not close
                // the resource the caller now owns (bug-141). `resource_close_function`
                // is `None` for a union type, so the plain-resource branch above
                // misses it — key off the union cleanup shape instead.
                if result
                    .as_ref()
                    .is_some_and(|result| self.resource_union_cleanup(&result.type_).is_some())
                {
                    self.deactivate_resource_cleanup(name);
                }
                // Returning an owner container — a `List OF RES File`, a record
                // with a `RES` field, any aggregate reaching a resource — hands its
                // block to the caller, so this scope must not free it. Its
                // owned-lists stay registered: the release above made every record
                // the returned value reaches unowned, so their drains close none of
                // those (the caller adopts them) and only free their nodes — and
                // close a record the container no longer holds, which this scope
                // still owns (§15.6: removal does not undo the float).
                if result
                    .as_ref()
                    .is_some_and(|result| self.is_resource_owning_container(&result.type_))
                {
                    self.retire_returned_owner_block(name);
                }
            }
        }
        // plan-59-D: this is the real `RETURN <value>` path, so the identity skip
        // belongs here — around the cleanup sequence, with the escaping value in
        // the slot `store_pending_success_result` wrote above.
        //
        // It BACKSTOPS the static deactivations above rather than replacing them.
        // Those are keyed on the returned value being syntactically the
        // `NirValue::Local` that owns the cleanup; a returned resource that is not
        // that local is invisible to them, and the runtime compare catches it.
        // Where both apply the skip is simply never reached, because the cleanup
        // has already been removed from the list.
        let previous_escaping = self.escaping_value_slot;
        self.escaping_value_slot = self.pending_result_slots.map(|slots| slots.value);
        let cleanup_result = self.emit_cleanup_sequence();
        self.escaping_value_slot = previous_escaping;
        cleanup_result?;
        self.load_pending_result_registers();
        self.emit(abi::return_());
        Ok(())
    }

    pub(crate) fn route_current_result_to_trap(&mut self) -> Result<(), String> {
        self.emit(abi::compare_immediate(
            RESULT_TAG_REGISTER,
            RESULT_PROGRAM_EXIT_TAG,
        ));
        let trap_label = self.label("trap_route_error");
        self.emit(abi::branch_ne(&trap_label));
        self.emit(abi::return_());
        self.emit(abi::label(&trap_label));

        let code_slot = self.allocate_stack_object("trap_error_code", 8);
        let message_slot = self.allocate_stack_object("trap_error_message", 8);
        let source_slot = self.allocate_stack_object("trap_error_source", 8);
        // The function-level trap local's slot is pinned in `TrapState`, not read
        // from `self.locals[name]`: an inline `TRAP(e)` in the body rebinds the
        // shared name `e` to a different slot, so a `self.locals` lookup here would
        // store the built `Error` to whichever slot was last bound, desyncing it
        // from the handler's read of the pinned slot (bug-148).
        let (stack_offset, label) = self
            .trap
            .as_ref()
            .map(|trap| (trap.stack_offset, trap.label.clone()))
            .ok_or_else(|| "trap routing requires bound trap local".to_string())?;

        // Design "b": an `ERR_BLOCK` error parked its owned Error block base in the
        // current-error slot. ADOPT it as the trap local (freed once on the
        // handler's exit) and clear the slot, instead of rebuilding a fresh block
        // and orphaning the parked one (bug-152). A legacy `ERR` falls through to
        // the rebuild below.
        let rebuild_label = self.label("trap_route_rebuild");
        self.emit(abi::compare_immediate(
            RESULT_TAG_REGISTER,
            RESULT_ERR_BLOCK_TAG,
        ));
        self.emit(abi::branch_ne(&rebuild_label));
        let adopted = self.emit_adopt_current_error_block();
        self.emit(abi::store_u64(&adopted, abi::stack_pointer(), stack_offset));
        self.emit(abi::branch(&label));

        self.emit(abi::label(&rebuild_label));
        self.emit(abi::store_u64(
            RESULT_VALUE_REGISTER,
            abi::stack_pointer(),
            code_slot,
        ));
        self.emit(abi::store_u64(
            RESULT_ERROR_MESSAGE_REGISTER,
            abi::stack_pointer(),
            message_slot,
        ));
        self.emit(abi::store_u64(
            RESULT_ERROR_SOURCE_REGISTER,
            abi::stack_pointer(),
            source_slot,
        ));
        let error_register = self.emit_build_error_inline(code_slot, message_slot, source_slot)?;
        self.emit(abi::store_u64(
            &error_register,
            abi::stack_pointer(),
            stack_offset,
        ));
        self.emit(abi::branch(&label));
        Ok(())
    }

    pub(crate) fn current_block_returns(&self) -> bool {
        self.instructions
            .last()
            .is_some_and(|instruction| matches!(instruction.op, CodeOp::Ret | CodeOp::Branch))
    }
}
