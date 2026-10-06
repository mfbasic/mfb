//! bug-755: the resource holds its cleanup scope.
//!
//! Every resource record carries one **owner** word (`RESOURCE_OFFSET_OWNER`): the
//! token of the one scope that closes it, or `0` while it is unowned (freshly
//! produced and not yet bound, or released by a `RETURN` and not yet taken). Every
//! `RES` binding, parameter, record field and collection slot is a reference to the
//! record; none of them is an owner. A scope exit closes a registered record only
//! when the owner word equals that scope's token, so a binding that merely refers to
//! a resource owned further out — `RES h = ident(a)` in an inner block — closes
//! nothing.
//!
//! ## Scope tokens
//!
//! A scope's token is `((sp + token_slot) << SCOPE_TOKEN_DEPTH_BITS) - depth`, where
//! `token_slot` is one 8-byte stack object per function and `depth` is the scope's
//! nesting depth inside the function (`cleanup_scope_starts.len()`, at least 1).
//!
//! * **Unique while live.** Two live scopes in one thread are either in different
//!   frames — different `token_slot` addresses — or at different depths of one
//!   frame. Sibling scopes share a token, but never live at the same time, and a
//!   scope that exits leaves no open record owned by it.
//! * **Outer is larger.** The stack grows down on every target, and a callee's whole
//!   frame lies below its caller's `sp`, so a callee's `token_slot` is at least 8
//!   bytes below its caller's. With `depth < 2^SCOPE_TOKEN_DEPTH_BITS`, every token
//!   of the callee is below every token of the caller; inside one frame a deeper
//!   scope subtracts more. Floating a resource outward is therefore "raise the owner
//!   to the larger token", and it never moves inward.
//! * **Per thread for free.** Each thread runs on its own stack. A record changes
//!   thread only through `thread::transfer`/`accept`, and the receiving copy is
//!   unowned until the accepting binding claims it.
//! * **Frame membership** is `(owner >> SCOPE_TOKEN_DEPTH_BITS) + 1 == sp +
//!   token_slot`: a `RETURN` releases exactly the records owned by its own frame.
//!
//! A token is never `0`: `sp + token_slot` is a stack address.
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::VirtualRegister;
use crate::codegen::error::constants::*;
use crate::target::shared::abi;
use crate::target::shared::abi::Reg;
use crate::types::ParameterType;

/// The low bits of a scope token that hold the (negated) scope depth. Statement
/// nesting is capped at 256 (`ast::stmt::MAX_STMT_DEPTH`); the desugared scopes a
/// statement adds stay far inside this bound.
pub(crate) const SCOPE_TOKEN_DEPTH_BITS: u8 = 12;

/// The largest scope depth a token can encode.
pub(crate) const SCOPE_TOKEN_MAX_DEPTH: usize = (1 << SCOPE_TOKEN_DEPTH_BITS) - 1;

impl CodeBuilder<'_> {
    /// The nesting depth of the scope a cleanup registered now belongs to: the
    /// function body is 1, each nested block one more.
    pub(crate) fn current_scope_depth(&self) -> usize {
        self.cleanup_scope_starts.len().max(1)
    }

    /// This function's token stack object, allocated on first use. Its address is
    /// the frame part of every scope token the function mints.
    fn scope_token_slot(&mut self) -> usize {
        if let Some(slot) = self.scope_token_slot {
            return slot;
        }
        let slot = self.allocate_stack_object("res_scope_token", 8);
        self.scope_token_slot = Some(slot);
        slot
    }

    /// `sp + token_slot` into a fresh register.
    pub(crate) fn emit_scope_token_base(&mut self) -> Reg {
        let slot = self.scope_token_slot();
        let base = self.allocate_register();
        self.emit(abi::add_immediate(&base, abi::stack_pointer(), slot));
        Reg::from(base)
    }

    /// The token of the scope at `depth` in this function, into a fresh register.
    pub(crate) fn emit_scope_token(&mut self, depth: usize) -> Result<Reg, String> {
        if depth == 0 || depth > SCOPE_TOKEN_MAX_DEPTH {
            return Err(format!(
                "native code resource scope depth {depth} is outside the scope-token range \
                 1..={SCOPE_TOKEN_MAX_DEPTH} (bug-755)"
            ));
        }
        let base = self.emit_scope_token_base();
        let token = self.allocate_register();
        self.emit(abi::shift_left_immediate(
            &token,
            &base,
            SCOPE_TOKEN_DEPTH_BITS,
        ));
        self.emit(abi::subtract_immediate(&token, &token, depth));
        Ok(Reg::from(token))
    }

    /// Claim the record in `record` for the scope at `depth` when it is unowned: a
    /// fresh producer result, or a record a callee's `RETURN` released. A record that
    /// already has an owner keeps it — this binding is a reference. A null record
    /// (a slot an error left unwritten) is skipped.
    pub(crate) fn emit_resource_claim(&mut self, record: &Reg, depth: usize) -> Result<(), String> {
        let done = self.label("resource_claim_done");
        self.emit(abi::compare_immediate(record, "0"));
        self.emit(abi::branch_eq(&done));
        let owner = self.allocate_register();
        self.emit(abi::load_u64(&owner, record, RESOURCE_OFFSET_OWNER));
        self.emit(abi::compare_immediate(&owner, "0"));
        self.emit(abi::branch_ne(&done));
        let token = self.emit_scope_token(depth)?;
        self.emit(abi::store_u64(&token, record, RESOURCE_OFFSET_OWNER));
        self.emit(abi::label(&done));
        Ok(())
    }

    /// [`Self::emit_resource_claim`] for the record a stack slot holds.
    pub(crate) fn emit_resource_claim_at(
        &mut self,
        slot: usize,
        depth: usize,
    ) -> Result<(), String> {
        let record = self.allocate_register();
        self.emit(abi::load_u64(&record, abi::stack_pointer(), slot));
        self.emit_resource_claim(&Reg::from(record), depth)
    }

    /// [`Self::emit_resource_claim`] for the active variant of the resource-union box a
    /// stack slot holds (`{tag@0, record@8}`). A null box is skipped.
    pub(crate) fn emit_resource_union_claim_at(
        &mut self,
        slot: usize,
        depth: usize,
    ) -> Result<(), String> {
        let done = self.label("resource_union_claim_done");
        let union_box = self.allocate_register();
        self.emit(abi::load_u64(&union_box, abi::stack_pointer(), slot));
        self.emit(abi::compare_immediate(&union_box, "0"));
        self.emit(abi::branch_eq(&done));
        let record = self.allocate_register();
        self.emit(abi::load_u64(&record, &union_box, 8));
        self.emit_resource_claim(&Reg::from(record), depth)?;
        self.emit(abi::label(&done));
        Ok(())
    }

    /// Branch to `skip` unless the scope at `depth` owns the (non-null) record in
    /// `record`. Every scope-exit close runs behind this test.
    pub(crate) fn emit_skip_unless_owned(
        &mut self,
        record: &Reg,
        depth: usize,
        skip: &str,
    ) -> Result<(), String> {
        let owner = self.allocate_register();
        self.emit(abi::load_u64(&owner, record, RESOURCE_OFFSET_OWNER));
        let token = self.emit_scope_token(depth)?;
        self.emit(abi::compare_registers(&owner, &token));
        self.emit(abi::branch_ne(skip));
        Ok(())
    }

    /// Float the record in `record` up to the scope at `depth` (an owner
    /// collection's scope): an unowned record, or one owned by a scope inside
    /// `depth`'s, takes `depth`'s token. A record owned by an enclosing scope or an
    /// enclosing frame keeps its owner — ownership only floats up.
    pub(crate) fn emit_resource_float(&mut self, record: &Reg, depth: usize) -> Result<(), String> {
        let done = self.label("resource_float_done");
        let take = self.label("resource_float_take");
        self.emit(abi::compare_immediate(record, "0"));
        self.emit(abi::branch_eq(&done));
        let owner = self.allocate_register();
        self.emit(abi::load_u64(&owner, record, RESOURCE_OFFSET_OWNER));
        let token = self.emit_scope_token(depth)?;
        // Unsigned: an unowned record (`0`) is below every token, so it is taken too.
        self.emit(abi::compare_registers(&owner, &token));
        self.emit(abi::branch_lo(&take));
        self.emit(abi::branch(&done));
        self.emit(abi::label(&take));
        self.emit(abi::store_u64(&token, record, RESOURCE_OFFSET_OWNER));
        self.emit(abi::label(&done));
        Ok(())
    }

    /// Release the (possibly null) record in `record` to the caller when a scope of
    /// this function owns it: a `RETURN` hands it over unowned and the caller's
    /// binding takes it. A record owned further out is left alone — returning a
    /// reference to it changes nothing.
    pub(crate) fn emit_resource_release(&mut self, record: &Reg) -> Result<(), String> {
        let done = self.label("resource_release_done");
        self.emit(abi::compare_immediate(record, "0"));
        self.emit(abi::branch_eq(&done));
        let owner = self.allocate_register();
        self.emit(abi::load_u64(&owner, record, RESOURCE_OFFSET_OWNER));
        self.emit(abi::shift_right_immediate(
            &owner,
            &owner,
            SCOPE_TOKEN_DEPTH_BITS,
        ));
        self.emit(abi::add_immediate(&owner, &owner, 1));
        let base = self.emit_scope_token_base();
        self.emit(abi::compare_registers(&owner, &base));
        self.emit(abi::branch_ne(&done));
        self.emit(abi::store_u64(abi::ZERO, record, RESOURCE_OFFSET_OWNER));
        self.emit(abi::label(&done));
        Ok(())
    }

    /// bug-843: [`Self::emit_resource_release`] measured against the frame whose
    /// token base (`sp + token_slot`) is in `base_slot` rather than this one's: an
    /// environment routine releasing, for the frame that is returning a closure, the
    /// records that frame owns. A resource-union leaf releases its active variant.
    pub(crate) fn emit_leaf_release_against(
        &mut self,
        leaf_slot: usize,
        leaf: &ParameterType,
        base_slot: usize,
    ) -> Result<(), String> {
        let done = self.label("resource_release_against_done");
        let record = self.allocate_register();
        self.emit(abi::load_u64(&record, abi::stack_pointer(), leaf_slot));
        self.emit(abi::compare_immediate(&record, "0"));
        self.emit(abi::branch_eq(&done));
        if self.resource_union_cleanup(leaf).is_some() {
            self.emit(abi::load_u64(&record, &record, 8));
            self.emit(abi::compare_immediate(&record, "0"));
            self.emit(abi::branch_eq(&done));
        }
        let owner = self.allocate_register();
        self.emit(abi::load_u64(&owner, &record, RESOURCE_OFFSET_OWNER));
        self.emit(abi::shift_right_immediate(
            &owner,
            &owner,
            SCOPE_TOKEN_DEPTH_BITS,
        ));
        self.emit(abi::add_immediate(&owner, &owner, 1));
        let base = self.allocate_register();
        self.emit(abi::load_u64(&base, abi::stack_pointer(), base_slot));
        self.emit(abi::compare_registers(&owner, &base));
        self.emit(abi::branch_ne(&done));
        self.emit(abi::store_u64(abi::ZERO, &record, RESOURCE_OFFSET_OWNER));
        self.emit(abi::label(&done));
        Ok(())
    }

    /// Whether a value of `type_` can carry a record a scope of this function owns:
    /// one that reaches a resource at any depth (bug-805) — a resource, a resource
    /// union, or a list, map, record or data union holding one (see
    /// [`Self::emit_release_returned_value`]).
    pub(crate) fn value_may_carry_owned_resources(&self, type_: &ParameterType) -> bool {
        self.type_carries_resources(type_)
    }

    /// Release, before a `RETURN`'s cleanups run, every record the returned value
    /// carries that a scope of this function owns — the record of a returned
    /// resource, the active variant of a returned resource union, and every
    /// resource a returned aggregate reaches at any depth (a `List OF RES` element,
    /// a record's `RES` field, a `List OF List OF Holder`'s inner field, bug-805).
    /// The caller's binding then claims them. A record owned further out stays
    /// with its owner.
    ///
    /// The aggregate walk is not optional for a record: a returned container
    /// declared before the resource it carries floats that resource to the
    /// container's scope (`TYPE_RESOURCE_RETURN_ORDER` only refuses the opposite
    /// order), so a returned record's field can be a resource this frame owns.
    pub(crate) fn emit_release_returned_value(
        &mut self,
        slot: usize,
        type_: &ParameterType,
    ) -> Result<(), String> {
        self.emit_for_each_resource(slot, type_, &mut |builder, leaf_slot, leaf| {
            if builder.is_closure_leaf(leaf) {
                // bug-843: a returned closure releases what it carries, through its
                // environment routine, against this frame's token base.
                let base = builder.emit_scope_token_base();
                let base_slot = builder.spill_to_slot("returned_closure_base", &base);
                return builder.emit_closure_resource_op(
                    leaf_slot,
                    CLOSURE_ENV_OP_RELEASE,
                    &[base_slot],
                );
            }
            let value = builder.allocate_register();
            builder.emit(abi::load_u64(&value, abi::stack_pointer(), leaf_slot));
            if builder.resource_union_cleanup(leaf).is_some() {
                builder.emit_union_variant_release(&value)
            } else {
                builder.emit_resource_release(&Reg::from(value))
            }
        })
    }

    /// [`Self::emit_resource_release`] for the active variant of the (possibly null)
    /// resource-union box in `union_box`.
    fn emit_union_variant_release(&mut self, union_box: &VirtualRegister) -> Result<(), String> {
        let done = self.label("resource_union_release_done");
        self.emit(abi::compare_immediate(union_box, "0"));
        self.emit(abi::branch_eq(&done));
        let record = self.allocate_register();
        self.emit(abi::load_u64(&record, union_box, 8));
        self.emit_resource_release(&Reg::from(record))?;
        self.emit(abi::label(&done));
        Ok(())
    }

    /// Close what a call statement's discarded result carries that nobody owns: a
    /// record the callee produced or released and the caller never bound, at any
    /// depth of the result (bug-805). A record with an owner — a handle the callee
    /// was passed and handed back — is left to that owner. The record itself stays
    /// as the tombstone; its buffers and `STATE` are reclaimed exactly as a
    /// binding's drop reclaims them.
    pub(crate) fn emit_close_discarded_value(
        &mut self,
        location: &crate::codegen::engine::operand::Operand,
        type_: &ParameterType,
    ) -> Result<(), String> {
        if !self.value_may_carry_owned_resources(type_) {
            return Ok(());
        }
        let slot = self.spill_to_slot("discarded_resources", location);
        self.emit_for_each_resource(slot, type_, &mut |builder, leaf_slot, leaf| {
            builder.emit_close_unowned_at(leaf_slot, leaf)
        })
    }

    /// bug-842: close what a resource temporary in `slot` (a producer call's result
    /// nobody took) reaches that is still unowned at the end of its statement. A
    /// resource already closed in that statement — `fs::close(mk())`, a transfer —
    /// is left alone: its close was the program's.
    pub(crate) fn emit_resource_temp_close(
        &mut self,
        slot: usize,
        type_: &ParameterType,
    ) -> Result<(), String> {
        self.emit_for_each_resource(slot, type_, &mut |builder, leaf_slot, leaf| {
            if builder.is_closure_leaf(leaf) {
                return builder.emit_close_unowned_at(leaf_slot, leaf);
            }
            let done = builder.label("resource_temp_close_done");
            let record = builder.allocate_register();
            builder.emit(abi::load_u64(&record, abi::stack_pointer(), leaf_slot));
            builder.emit(abi::compare_immediate(&record, "0"));
            builder.emit(abi::branch_eq(&done));
            if builder.resource_union_cleanup(leaf).is_some() {
                builder.emit(abi::load_u64(&record, &record, 8));
                builder.emit(abi::compare_immediate(&record, "0"));
                builder.emit(abi::branch_eq(&done));
            }
            let flag = builder.allocate_register();
            builder.emit(abi::load_u64(&flag, &record, RESOURCE_OFFSET_CLOSED));
            builder.emit(abi::compare_immediate(&flag, "0"));
            builder.emit(abi::branch_ne(&done));
            builder.emit_close_unowned_at(leaf_slot, leaf)?;
            builder.emit(abi::label(&done));
            Ok(())
        })
    }

    /// bug-842: claim for the scope at the current depth every unowned resource the
    /// returned value in `slot` reaches, before a `RETURN` closes its statement's
    /// resource temporaries: one the returned value carries out is then owned, so the
    /// close leaves it, and the release that follows hands it to the caller unowned
    /// again. A closure leaf claims through its environment routine.
    pub(crate) fn emit_claim_returned_value(
        &mut self,
        slot: usize,
        type_: &ParameterType,
    ) -> Result<(), String> {
        let depth = self.current_scope_depth();
        self.emit_for_each_resource(slot, type_, &mut |builder, leaf_slot, leaf| {
            if builder.is_closure_leaf(leaf) {
                let token = builder.emit_scope_token(depth)?;
                let token_slot = builder.spill_to_slot("returned_claim_token", &token);
                return builder.emit_closure_resource_op(
                    leaf_slot,
                    CLOSURE_ENV_OP_CLAIM,
                    &[token_slot],
                );
            }
            if builder.resource_union_cleanup(leaf).is_some() {
                builder.emit_resource_union_claim_at(leaf_slot, depth)
            } else {
                builder.emit_resource_claim_at(leaf_slot, depth)
            }
        })
    }

    /// bug-842: [`Self::emit_resource_claim`] against the token in `token_slot`
    /// rather than one of this frame's scopes — an environment routine claiming,
    /// for a returning frame, the resources its returned closure carries.
    pub(crate) fn emit_leaf_claim_against(
        &mut self,
        leaf_slot: usize,
        leaf: &ParameterType,
        token_slot: usize,
    ) -> Result<(), String> {
        let done = self.label("resource_claim_against_done");
        let record = self.allocate_register();
        self.emit(abi::load_u64(&record, abi::stack_pointer(), leaf_slot));
        self.emit(abi::compare_immediate(&record, "0"));
        self.emit(abi::branch_eq(&done));
        if self.resource_union_cleanup(leaf).is_some() {
            self.emit(abi::load_u64(&record, &record, 8));
            self.emit(abi::compare_immediate(&record, "0"));
            self.emit(abi::branch_eq(&done));
        }
        let owner = self.allocate_register();
        self.emit(abi::load_u64(&owner, &record, RESOURCE_OFFSET_OWNER));
        self.emit(abi::compare_immediate(&owner, "0"));
        self.emit(abi::branch_ne(&done));
        let token = self.allocate_register();
        self.emit(abi::load_u64(&token, abi::stack_pointer(), token_slot));
        self.emit(abi::store_u64(&token, &record, RESOURCE_OFFSET_OWNER));
        self.emit(abi::label(&done));
        Ok(())
    }

    /// Close the resource (or resource-union box) a slot holds when it is non-null
    /// and unowned. bug-843: a closure leaf closes what its closure carries that
    /// nobody owns, through its environment routine.
    pub(crate) fn emit_close_unowned_at(
        &mut self,
        slot: usize,
        type_: &ParameterType,
    ) -> Result<(), String> {
        if self.is_closure_leaf(type_) {
            return self.emit_closure_resource_op(slot, CLOSURE_ENV_OP_CLOSE_UNOWNED, &[]);
        }
        let done = self.label("discarded_close_done");
        let value = self.allocate_register();
        self.emit(abi::load_u64(&value, abi::stack_pointer(), slot));
        self.emit(abi::compare_immediate(&value, "0"));
        self.emit(abi::branch_eq(&done));
        if let Some(variants) = self.resource_union_cleanup(type_) {
            let record = self.allocate_register();
            self.emit(abi::load_u64(&record, &value, 8));
            self.emit(abi::compare_immediate(&record, "0"));
            self.emit(abi::branch_eq(&done));
            let owner = self.allocate_register();
            self.emit(abi::load_u64(&owner, &record, RESOURCE_OFFSET_OWNER));
            self.emit(abi::compare_immediate(&owner, "0"));
            self.emit(abi::branch_ne(&done));
            self.emit_union_tag_dispatch_drop(
                &value,
                &variants,
                type_.state().as_ref(),
                &[],
                &done,
            )?;
            self.emit(abi::label(&done));
            return Ok(());
        }
        let Some(close) = self.resource_cleanup_symbol(type_) else {
            self.emit(abi::label(&done));
            return Ok(());
        };
        let owner = self.allocate_register();
        self.emit(abi::load_u64(&owner, &value, RESOURCE_OFFSET_OWNER));
        self.emit(abi::compare_immediate(&owner, "0"));
        self.emit(abi::branch_ne(&done));
        let closed = self.label("discarded_close_ok");
        self.emit(abi::move_register(abi::return_register(), &value));
        self.emit_symbol_call(&close);
        self.emit(abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG));
        self.emit(abi::branch_eq(&closed));
        self.record_secondary_cleanup_failure();
        self.emit(abi::label(&closed));
        self.emit_resource_block_reclaim(
            slot,
            type_.state().as_ref(),
            Self::resource_uses_io_buffers(type_),
            false,
        )?;
        self.emit(abi::label(&done));
        Ok(())
    }
}
