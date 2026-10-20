//! plan-176-U: the string capacity shadows and the self-append prescan. The statement entry (`lower_ops`) and shared types live in
//! `engine/control/builder_control/mod.rs`.

use super::*;

impl CodeBuilder<'_> {
    /// Reset a `String` local's capacity shadow to 0 ("tight, no spare") after any
    /// non-self-append bind/assign installs a fresh tight buffer. Keeps the shadow
    /// from claiming spare that the new buffer does not have (plan-02 §4.1).
    /// bug-560: the self-append capacity shadow that describes the block in
    /// `name`'s slot, when there is one.
    ///
    /// `prescan_string_self_appends` claims a shadow per NAME, and
    /// `SYMBOL_DUPLICATE_LOCAL` makes a local name unique within a function, so
    /// one shadow describes exactly one binding — there is no live outer binding
    /// of the same name whose (tight) block this could over-free. The shadow is
    /// reset to 0 by every non-self-append bind/assign to the slot and is only
    /// made non-zero by the regrow that allocated the block it describes, so
    /// `byteLength + shadow + 9` is the block's allocation size exactly.
    ///
    /// Answers `None` for anything but a `String` and for a name with no shadow —
    /// fail-closed: the drop then frees the historical tight size.
    pub(crate) fn string_capacity_slot_for(
        &self,
        name: &str,
        type_: &ParameterType,
    ) -> Option<usize> {
        if *type_ != ParameterType::String {
            return None;
        }
        self.string_capacity_slots.get(name).copied()
    }

    /// plan-146-G: a working copy of the OWNER's capacity shadow for the by-ref
    /// capture `name`, read through the closure environment — the size a free of
    /// its block must use. `None` when this lambda shares no shadow for it.
    pub(crate) fn string_shadow_working_copy(&mut self, name: &str) -> Option<(usize, usize)> {
        let index = self.string_shadow_env.get(name).copied()?;
        let slot = self.allocate_stack_object("ref_strcap_free", 8);
        let holder_slot = self.allocate_stack_object("ref_strcap_owner", 8);
        let holder = self.allocate_register();
        let spare = self.allocate_register();
        self.emit(abi::load_u64(
            &holder,
            CLOSURE_ENV_REGISTER,
            closure_env_word_offset(index),
        ));
        self.emit(abi::store_u64(&holder, abi::stack_pointer(), holder_slot));
        self.emit(abi::load_u64(&spare, &holder, 0));
        self.emit(abi::store_u64(&spare, abi::stack_pointer(), slot));
        Some((slot, holder_slot))
    }

    /// plan-146-G: store 0 into the OWNER's shadow through the closure
    /// environment — every store that installs a fresh (tight) block through the
    /// reference does this, exactly as a plain local's store resets its own.
    pub(crate) fn reset_shared_string_shadow(&mut self, holder_slot: Option<usize>) {
        let Some(holder_slot) = holder_slot else {
            return;
        };
        let holder = self.allocate_register();
        self.emit(abi::load_u64(&holder, abi::stack_pointer(), holder_slot));
        self.emit(abi::store_u64(abi::ZERO, &holder, 0));
    }

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
        // plan-146-G: a `String` this frame lends a lambda by reference needs a
        // shadow HERE, because the lambda's self-update leaves its spare bytes in
        // this frame's block; the lambda reads and publishes it through the closure
        // environment (`string_shadow_captures`).
        for lambda in self.closures_created_in(ops) {
            for (local, _) in
                crate::codegen::collection::assign::string_self_update::string_shadow_captures(
                    self.functions,
                    &lambda,
                )
            {
                if !self.string_capacity_slots.contains_key(&local) {
                    let slot = self.allocate_stack_object(&format!("strcap_{local}"), 8);
                    self.string_capacity_slots.insert(local, slot);
                }
            }
        }
        for op in ops {
            match op {
                NirOp::Assign { name, value } => {
                    // plan-146-G: a local captured by reference gets one too — and
                    // shares it with every lambda that self-updates it, through the
                    // closure environment. Before that it got none, because a
                    // callback could replace the buffer through the reference
                    // without seeing this frame's shadow, which would then claim
                    // spare bytes the new buffer does not have
                    // (`rt_byref_string_capture_capacity`); the shared word is what
                    // keeps the two in step.
                    if crate::codegen::collection::assign::string_self_update::is_string_self_update(
                        value,
                        &|root| matches!(root, NirValue::Local(local) if local == name),
                    ) && !self.string_capacity_slots.contains_key(name)
                        // plan-146-G observation O2: a by-ref capture that shares
                        // its owner's shadow needs no slot of its own — the one it
                        // used to get was never read.
                        && !self.string_shadow_env.contains_key(name)
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
}
