//! plan-176-U: `lower_closure_value` and `lower_union_wrap_value`, the capturing `Closure` and
//! `UnionWrap` arms of the value lowering. Called from `lower_value_inner` in
//! `engine/value/builder_values/lower_inner.rs`.

use super::*;

impl CodeBuilder<'_> {
    /// Lower a capturing `NirValue::Closure` (the capture-less one stays in `lower_value_inner`):
    /// allocate the closure and its environment and copy each capture in.
    pub(super) fn lower_closure_value(&mut self, value: &NirValue) -> Result<ValueResult, String> {
        let NirValue::Closure {
            name,
            type_,
            captures,
        } = value
        else {
            unreachable!("lower_closure_value is called from the NirValue::Closure arm only")
        };
        let symbol = self
            .function_symbols
            .get(name)
            .cloned()
            .unwrap_or_else(|| name.clone());
        let ops_symbol = closure_env_ops_symbol(&symbol);
        let function_register = self.allocate_register();
        self.emit(abi::load_page_address(&function_register, &symbol));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: symbol.clone(),
            kind: RelocIntent::DataAddrHi,
            binding: "data".to_string(),
            library: None,
        });
        self.emit(abi::add_page_offset(
            &function_register,
            &function_register,
            &symbol,
        ));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: symbol,
            kind: RelocIntent::DataAddrLo,
            binding: "data".to_string(),
            library: None,
        });
        let function_slot = self.allocate_stack_object("closure_code", 8);
        self.emit(abi::store_u64(
            &function_register,
            abi::stack_pointer(),
            function_slot,
        ));
        // plan-142-G: a lambda that borrows this frame's self-update
        // scratch finds its slot's address one word past the captures. The
        // prescan gave this frame a scratch of its own for it; a frame that
        // only borrows one cannot lend it on.
        let borrowed_scratch = match self.scratch_closure_captures(name) {
            None => None,
            Some(_) => match (self.self_update_scratch, self.self_update_scratch_env) {
                (Some(slot), None) => Some(slot),
                _ => {
                    return Err(format!(
                        "native closure `{name}` borrows a self-update scratch its \
                                 creator does not own"
                    ))
                }
            },
        };
        // plan-146-G: a lambda that self-updates a by-ref-captured `String`
        // works on the OWNER's block, so it shares the owner's capacity
        // shadow through one further environment word per such capture.
        let shared_shadows: Vec<(String, usize)> =
            crate::codegen::collection::assign::string_self_update::string_shadow_captures(
                self.functions,
                name,
            );
        let env_register = self.allocate_register();
        let env_slot = self.allocate_stack_object("closure_env", 8);
        let alloc_ok = self.label("closure_env_alloc_ok");
        let shares_resource_base =
            crate::codegen::collection::assign::string_self_update::lambda_shares_resource_base(
                self.functions,
                name,
            );
        let auxiliary_words = usize::from(borrowed_scratch.is_some())
            + shared_shadows.len()
            + usize::from(shares_resource_base);
        let env_size = (closure_env_word_offset(captures.len() + auxiliary_words)).to_string();
        self.emit(abi::move_immediate(
            abi::return_register(),
            "Integer",
            &env_size,
        ));
        self.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
        self.emit_arena_alloc_call();
        self.emit(abi::branch_eq(&alloc_ok));
        self.raise_error_bare("ErrOutOfMemory")?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::move_register(&env_register, abi::mfb_return(1)));
        self.emit(abi::store_u64(
            &env_register,
            abi::stack_pointer(),
            env_slot,
        ));
        // bug-781: word 0 names this `LAMBDA`'s environment routine, so a
        // holder that knows only the `FUNC` type can copy or drop it.
        let ops_register = self.allocate_register();
        self.emit(abi::load_page_address(&ops_register, &ops_symbol));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ops_symbol.clone(),
            kind: RelocIntent::DataAddrHi,
            binding: "data".to_string(),
            library: None,
        });
        self.emit(abi::add_page_offset(
            &ops_register,
            &ops_register,
            &ops_symbol,
        ));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ops_symbol.clone(),
            kind: RelocIntent::DataAddrLo,
            binding: "data".to_string(),
            library: None,
        });
        let env_register = self.allocate_register();
        self.emit(abi::load_u64(&env_register, abi::stack_pointer(), env_slot));
        self.emit(abi::store_u64(
            &ops_register,
            &env_register,
            CLOSURE_ENV_OFFSET_OPS,
        ));
        let mut words = Vec::with_capacity(captures.len());
        let mut resource_words = Vec::new();
        for (index, capture) in captures.iter().enumerate() {
            // The closure env outlives the capturing scope, so it must
            // own each captured value independently (plan-02 Phase 8).
            // `lower_value_owned` deep-copies an aliasing source; its
            // `arena_alloc` clobbers caller-saved scratch (incl.
            // `env_register`), so reload the env from its slot.
            let value = self.lower_value_owned(capture)?;
            let word = self.closure_env_word(capture, &value.type_);
            // bug-843: the environment owns its own copy of a captured
            // resource-union box (`ClosureEnvWord::ResourceUnionBox`).
            let value = if word == ClosureEnvWord::ResourceUnionBox {
                let box_slot = self.spill_to_slot("closure_capture_union_box", &value.location);
                let copied = self.emit_copy_resource_union_box(box_slot)?;
                ValueResult {
                    location: Operand::from(copied),
                    ..value
                }
            } else {
                value
            };
            words.push(word);
            // bug-843: a by-value capture that reaches a resource is a
            // reference the closure carries, walked by the routine's
            // resource operations (`closure_carry`).
            let by_ref = matches!(
                capture,
                NirValue::LocalRef { .. } | NirValue::Capture { by_ref: true, .. }
            );
            if !by_ref && self.type_carries_resources(&value.type_) {
                resource_words.push((index, value.type_.clone()));
            }
            // Observation boundary: a `Float` captured into the
            // closure env is read back when the closure runs, so it
            // must be finite (plan-17).
            self.observe_float(capture, &value)?;
            // Materialize a `d`-native float before storing it into
            // the closure env (plan-01 float-dnative).
            let value = self.materialize_float(value)?;
            let env_register = self.allocate_register();
            self.emit(abi::load_u64(&env_register, abi::stack_pointer(), env_slot));
            self.emit(abi::store_u64(
                &value.location,
                &env_register,
                closure_env_word_offset(index),
            ));
        }
        if let Some(scratch_slot) = borrowed_scratch {
            let holder = self.allocate_register();
            self.emit(abi::add_immediate(
                &holder,
                abi::stack_pointer(),
                scratch_slot,
            ));
            let env_register = self.allocate_register();
            self.emit(abi::load_u64(&env_register, abi::stack_pointer(), env_slot));
            self.emit(abi::store_u64(
                &holder,
                &env_register,
                closure_env_word_offset(captures.len()),
            ));
        }
        for (position, (local, _)) in shared_shadows.iter().enumerate() {
            // The owner's shadow slot, which its own prescan allocated
            // for exactly this capture (`prescan_string_self_appends`).
            let Some(shadow_slot) = self.string_capacity_slots.get(local).copied() else {
                return Err(format!(
                    "native closure `{name}` shares the capacity shadow of `{local}`, \
                             which its creator does not have"
                ));
            };
            let holder = self.allocate_register();
            self.emit(abi::add_immediate(
                &holder,
                abi::stack_pointer(),
                shadow_slot,
            ));
            let env_register = self.allocate_register();
            self.emit(abi::load_u64(&env_register, abi::stack_pointer(), env_slot));
            let index = captures.len() + usize::from(borrowed_scratch.is_some()) + position;
            self.emit(abi::store_u64(
                &holder,
                &env_register,
                closure_env_word_offset(index),
            ));
        }
        if shares_resource_base {
            // plan-146-G: the address of this frame's `os::appResourcePath`
            // base cache, which the lambda fills on its first call and
            // this frame owns and frees.
            let Some(base_slot) = self.string_app_resource_base else {
                return Err(format!(
                    "native closure `{name}` shares an os.appResourcePath base cache \
                             its creator does not have"
                ));
            };
            let holder = self.allocate_register();
            self.emit(abi::add_immediate(&holder, abi::stack_pointer(), base_slot));
            let env_register = self.allocate_register();
            self.emit(abi::load_u64(&env_register, abi::stack_pointer(), env_slot));
            let index =
                captures.len() + usize::from(borrowed_scratch.is_some()) + shared_shadows.len();
            self.emit(abi::store_u64(
                &holder,
                &env_register,
                closure_env_word_offset(index),
            ));
        }
        self.record_closure_env_layout(ClosureEnvLayout {
            symbol: ops_symbol,
            words,
            auxiliary_words,
            resource_words,
        })?;
        let closure_register = self.allocate_register();
        let alloc_ok = self.label("closure_alloc_ok");
        // plan-71-C Family-1a: alloc size is arg 0 → `%arg0`, not return_register().
        self.emit(abi::move_immediate(
            abi::c_arg(0),
            "Integer",
            &CLOSURE_OBJECT_SIZE.to_string(),
        ));
        self.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
        self.emit_arena_alloc_call();
        self.emit(abi::branch_eq(&alloc_ok));
        self.raise_error_bare("ErrOutOfMemory")?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::load_u64(
            &function_register,
            abi::stack_pointer(),
            function_slot,
        ));
        self.emit(abi::store_u64(
            &function_register,
            abi::mfb_return(1),
            CLOSURE_OFFSET_CODE,
        ));
        let env_register = self.allocate_register();
        self.emit(abi::load_u64(&env_register, abi::stack_pointer(), env_slot));
        self.emit(abi::store_u64(
            &env_register,
            abi::mfb_return(1),
            CLOSURE_OFFSET_ENV,
        ));
        self.emit(abi::move_register(&closure_register, abi::mfb_return(1)));
        // bug-781: the object is a fresh value its consumer owns — a binding,
        // a field, an element, a return claims it; an argument leaves it to
        // the statement-scope free (`register_pending_temp`, as for any
        // owned value).
        Ok(ValueResult {
            origin: None,
            type_: type_.clone(),
            location: Operand::from(closure_register),
            text: name.clone(),
        })
    }

    /// Lower a `NirValue::UnionWrap`: store the variant into a new union block. `scratch9` and
    /// `scratch10` are `lower_value_inner`'s two scratch vregs.
    pub(super) fn lower_union_wrap_value(
        &mut self,
        value: &NirValue,
        scratch9: &VirtualRegister,
        scratch10: &VirtualRegister,
    ) -> Result<ValueResult, String> {
        let NirValue::UnionWrap {
            union_type,
            member_type,
            value,
        } = value
        else {
            unreachable!("lower_union_wrap_value is called from the NirValue::UnionWrap arm only")
        };
        // plan-134-E: the variant is stored into the new union block.
        let wrapped = self.lower_value_stored_inline(value)?;
        let wrapped_slot = self.allocate_stack_object("union_wrap_source", 8);
        self.emit(abi::store_u64(
            &wrapped.location,
            abi::stack_pointer(),
            wrapped_slot,
        ));
        // A resource-union variant is a bare resource whose payload is
        // the resource pointer itself (one word at offset 8), not record
        // fields.
        // bug-851: a `RESOURCE`-declared variant is a bare handle as well.
        let is_resource_variant = self.is_resource_nominal(member_type);
        let fields = if is_resource_variant {
            Vec::new()
        } else {
            self.type_model
                .record_fields
                .get(member_type)
                .cloned()
                .ok_or_else(|| {
                    format!("native code union wrap member '{member_type}' is not a record")
                })?
        };
        let tag = self
            .type_model
            .union_variant_tags
            .get(member_type)
            .copied()
            .ok_or_else(|| format!("native code union variant '{member_type}' does not resolve"))?;
        // Data variant: build a flat `{tag, size, variant-record-block}`
        // union, inlining the wrapped variant record at +16 (plan-02
        // §4.3). Resource variants fall through to the fixed
        // `{tag, resource-ptr}` layout below.
        if !is_resource_variant {
            let _ = &fields;
            let register = self.emit_wrap_record_in_union(member_type, tag, wrapped_slot)?;
            return Ok(ValueResult {
                origin: None,
                type_: union_type.clone(),
                location: Operand::from(register),
                text: format!("wrap {member_type} as {union_type}"),
            });
        }
        // Payload words across all variants: a resource variant occupies
        // one word (the handle pointer); a record variant occupies its
        // field count.
        let max_payload = self
            .type_model
            .variants_for_union(union_type)
            .map(|variant| {
                if self.is_resource_nominal(variant) {
                    1
                } else {
                    self.type_model
                        .union_variant_fields
                        .get(variant)
                        .map(Vec::len)
                        .unwrap_or(0)
                }
            })
            .max()
            .unwrap_or(if is_resource_variant { 1 } else { fields.len() });
        let union_size = 8 * (1 + max_payload.max(1));
        let result_slot = self.allocate_stack_object("union_result", 8);
        let alloc_ok = self.label("union_construct_alloc_ok");
        self.emit(abi::move_immediate(
            abi::return_register(),
            "Integer",
            &union_size.to_string(),
        ));
        self.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
        self.emit_arena_alloc_call();
        self.emit(abi::branch_eq(&alloc_ok));
        self.raise_error_bare("ErrOutOfMemory")?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::store_u64(
            abi::mfb_return(1),
            abi::stack_pointer(),
            result_slot,
        ));
        let zero_register = self.allocate_register();
        self.emit(abi::move_immediate(&zero_register, "Integer", "0"));
        for offset in (0..union_size).step_by(8) {
            self.emit(abi::store_u64(&zero_register, abi::mfb_return(1), offset));
        }
        let tag_register = self.allocate_register();
        self.emit(abi::move_immediate(
            &tag_register,
            abi::IMMEDIATE_CLASS_UNION_TAG,
            &tag.to_string(),
        ));
        self.emit(abi::store_u64(&tag_register, abi::mfb_return(1), 0));
        // Only resource variants reach here — every data variant returned
        // early above via `emit_wrap_record_in_union`. A resource variant
        // stores its handle pointer as a single word at +8 (plan-02 §4.2).
        self.emit(abi::load_u64(scratch9, abi::stack_pointer(), wrapped_slot));
        self.emit(abi::load_u64(scratch10, abi::stack_pointer(), result_slot));
        self.emit(abi::store_u64(scratch9, scratch10, 8));
        let register = self.allocate_register();
        self.emit(abi::load_u64(&register, abi::stack_pointer(), result_slot));
        Ok(ValueResult {
            origin: None,
            type_: union_type.clone(),
            location: Operand::from(register),
            text: format!("wrap {member_type} as {union_type}"),
        })
    }
}
