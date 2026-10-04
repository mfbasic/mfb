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
    fn emit_scope_token_base(&mut self) -> Reg {
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

    /// Whether a value of `type_` can carry a record a scope of this function owns:
    /// a resource, a resource union, or a `List OF RES` (see
    /// [`Self::emit_release_returned_value`]).
    pub(crate) fn value_may_carry_owned_resources(&self, type_: &ParameterType) -> bool {
        let bare = match type_ {
            ParameterType::Res(inner) => inner.as_ref(),
            other => other,
        };
        self.resource_cleanup_symbol(bare).is_some()
            || self.resource_union_cleanup(bare).is_some()
            || (Self::is_res_marked_resource_collection(type_)
                && crate::codegen::engine::types::typed_list_element_type(type_).is_some())
    }

    /// Release, before a `RETURN`'s cleanups run, every record the returned value
    /// carries that a scope of this function owns: the record of a returned
    /// resource, the active variant of a returned resource union, and each element of
    /// a returned `List OF RES`. The caller's binding then claims them. A record
    /// owned further out stays with its owner.
    ///
    /// A record with a `RES` field needs no walk: `TYPE_RESOURCE_RETURN_ORDER` refuses
    /// returning one around a resource the function produced, so its fields always
    /// refer to resources owned further out.
    pub(crate) fn emit_release_returned_value(
        &mut self,
        slot: usize,
        type_: &ParameterType,
    ) -> Result<(), String> {
        let bare = match type_ {
            ParameterType::Res(inner) => inner.as_ref(),
            other => other,
        };
        if self.resource_cleanup_symbol(bare).is_some() {
            let record = self.allocate_register();
            self.emit(abi::load_u64(&record, abi::stack_pointer(), slot));
            return self.emit_resource_release(&Reg::from(record));
        }
        if self.resource_union_cleanup(bare).is_some() {
            let union_box = self.allocate_register();
            self.emit(abi::load_u64(&union_box, abi::stack_pointer(), slot));
            return self.emit_union_variant_release(&union_box);
        }
        if !Self::is_res_marked_resource_collection(type_) {
            return Ok(());
        }
        let Some(element_type) =
            crate::codegen::engine::types::typed_list_element_type(type_).cloned()
        else {
            return Ok(());
        };
        let element_is_union = {
            let bare = match &element_type {
                ParameterType::Res(inner) => inner.as_ref(),
                other => other,
            };
            self.resource_union_cleanup(bare).is_some()
        };
        let cursor_slot = self.allocate_stack_object("release_cursor", 8);
        let remaining_slot = self.allocate_stack_object("release_remaining", 8);
        let null_list = self.label("release_list_null");
        let list = self.allocate_register();
        self.emit(abi::load_u64(&list, abi::stack_pointer(), slot));
        self.emit(abi::compare_immediate(&list, "0"));
        self.emit(abi::branch_eq(&null_list));
        self.initialize_collection_loop_slots(slot, cursor_slot, remaining_slot, &element_type);
        let loop_label = self.label("release_list_loop");
        self.emit(abi::label(&loop_label));
        let remaining = self.allocate_register();
        self.emit(abi::load_u64(
            &remaining,
            abi::stack_pointer(),
            remaining_slot,
        ));
        self.emit(abi::compare_immediate(&remaining, "0"));
        self.emit(abi::branch_eq(&null_list));
        let item = self.load_collection_loop_item(slot, cursor_slot, &element_type)?;
        if element_is_union {
            self.emit_union_variant_release(&item)?;
        } else {
            self.emit_resource_release(&Reg::from(item))?;
        }
        self.advance_collection_loop(cursor_slot, remaining_slot, &loop_label, &element_type);
        self.emit(abi::label(&null_list));
        Ok(())
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
    /// record the callee produced or released and the caller never bound. A record
    /// with an owner — a handle the callee was passed and handed back — is left to
    /// that owner. The record itself stays as the tombstone; its buffers and `STATE`
    /// are reclaimed exactly as a binding's drop reclaims them.
    pub(crate) fn emit_close_discarded_value(
        &mut self,
        location: &crate::codegen::engine::operand::Operand,
        type_: &ParameterType,
    ) -> Result<(), String> {
        if !self.value_may_carry_owned_resources(type_) {
            return Ok(());
        }
        let slot = self.spill_to_slot("discarded_resources", location);
        let bare = match type_ {
            ParameterType::Res(inner) => inner.as_ref(),
            other => other,
        };
        if Self::is_res_marked_resource_collection(type_) {
            let Some(element_type) =
                crate::codegen::engine::types::typed_list_element_type(type_).cloned()
            else {
                return Ok(());
            };
            let element_slot = self.allocate_stack_object("discarded_element", 8);
            let cursor_slot = self.allocate_stack_object("discarded_cursor", 8);
            let remaining_slot = self.allocate_stack_object("discarded_remaining", 8);
            let done = self.label("discarded_list_done");
            let list = self.allocate_register();
            self.emit(abi::load_u64(&list, abi::stack_pointer(), slot));
            self.emit(abi::compare_immediate(&list, "0"));
            self.emit(abi::branch_eq(&done));
            self.initialize_collection_loop_slots(slot, cursor_slot, remaining_slot, &element_type);
            let loop_label = self.label("discarded_list_loop");
            self.emit(abi::label(&loop_label));
            let remaining = self.allocate_register();
            self.emit(abi::load_u64(
                &remaining,
                abi::stack_pointer(),
                remaining_slot,
            ));
            self.emit(abi::compare_immediate(&remaining, "0"));
            self.emit(abi::branch_eq(&done));
            let item = self.load_collection_loop_item(slot, cursor_slot, &element_type)?;
            self.emit(abi::store_u64(&item, abi::stack_pointer(), element_slot));
            let element_bare = match &element_type {
                ParameterType::Res(inner) => inner.as_ref().clone(),
                other => other.clone(),
            };
            self.emit_close_unowned_at(element_slot, &element_bare)?;
            self.advance_collection_loop(cursor_slot, remaining_slot, &loop_label, &element_type);
            self.emit(abi::label(&done));
            return Ok(());
        }
        let bare = bare.clone();
        self.emit_close_unowned_at(slot, &bare)
    }

    /// Close the resource (or resource-union box) a slot holds when it is non-null
    /// and unowned.
    fn emit_close_unowned_at(&mut self, slot: usize, type_: &ParameterType) -> Result<(), String> {
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
