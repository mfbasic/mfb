//! plan-176-P: `toString` of enums, booleans, integers and byte lists. String comparison and
//! ordering live in `string/repr/builder_strings/mod.rs`.

use super::*;

impl CodeBuilder<'_> {
    /// plan-140-C: `toString(<enum>)` is the member's name as declared. The value
    /// is the member's 0-based ordinal, so a compare-and-branch chain over the
    /// members in declaration order selects the name's string data. The last
    /// member is the fall-through: an enum value is always one of its members,
    /// so there is no "no match" case. The pre-pass registers these exact names
    /// (`to_string_enum_members`), both reading `TypeModel::enum_names`.
    ///
    /// The selected name is then copied into a fresh arena block and marked
    /// fresh, like every other `toString` result, rather than handed back as a
    /// read-only pointer. A read-only result is only safe where every owner
    /// recognizes it as one: an owning store that did not would take it as its
    /// own and later free or grow it in place (a bound `toString(<Boolean>)`
    /// followed by `s = s & "!"` does exactly that, see plan-140-C C-C3).
    fn lower_enum_to_string(
        &mut self,
        value: ValueResult,
        members: &[String],
    ) -> Result<ValueResult, String> {
        let Some((last, earlier)) = members.split_last() else {
            return Err(format!(
                "native toString: enum {} has no members",
                value.type_
            ));
        };
        let result = self.allocate_register();
        let done = self.label("enum_to_string_done");
        for (ordinal, member) in earlier.iter().enumerate() {
            let next = self.label("enum_to_string_next");
            self.emit(abi::compare_immediate(
                &value.location,
                &ordinal.to_string(),
            ));
            self.emit(abi::branch_ne(&next));
            self.emit_load_string_constant(&result, member)?;
            self.emit(abi::branch(&done));
            self.emit(abi::label(&next));
        }
        self.emit_load_string_constant(&result, last)?;
        self.emit(abi::label(&done));
        let copied = self.copy_flat_block(&ParameterType::String, &result)?;
        self.mark_fresh_string(Operand::from(copied));
        Ok(ValueResult {
            origin: None,
            type_: ParameterType::String,
            location: Operand::from(copied),
            text: format!("toString({})", value.text),
        })
    }

    pub(crate) fn lower_to_string(&mut self, args: &[NirValue]) -> Result<ValueResult, String> {
        let scratch8 = self.temporary_vreg();
        let value = self.lower_value(&args[0])?;
        if args.len() == 1 {
            if let Some(members) = self.type_model.enum_member_names(&value.type_) {
                let members = members.to_vec();
                return self.lower_enum_to_string(value, &members);
            }
        }
        // Observation boundary: rendering a `Float` to text makes it
        // user-accessible, so a non-finite arithmetic result must trap here
        // rather than print as "inf"/"nan" (plan-17). `toString`/`toText` are
        // the only Float→String path (`print` formats through them), and a
        // non-arithmetic Float argument is already finite by the invariant.
        self.observe_float(&args[0], &value)?;
        // The value is spilled through an integer slot and reloaded into a GPR
        // for formatting, so a `d`-native float is materialized into a GPR first
        // (plan-01 float-dnative). Identity for every GP-native value.
        let value = self.materialize_float(value)?;
        let value_slot = self.allocate_stack_object("to_string_value", 8);
        self.emit(abi::store_u64(
            &value.location,
            abi::stack_pointer(),
            value_slot,
        ));

        let precision_slot = self.allocate_stack_object("to_string_precision", 8);
        if let Some(precision) = args.get(1) {
            let precision = self.lower_value(precision)?;
            if precision.type_ != ParameterType::Byte {
                return Err(format!(
                    "native toString precision must be Byte, got {}",
                    precision.type_
                ));
            }
            self.emit(abi::store_u64(
                &precision.location,
                abi::stack_pointer(),
                precision_slot,
            ));
        } else {
            self.emit(abi::move_immediate(&scratch8, "Byte", "2"));
            self.emit(abi::store_u64(
                &scratch8,
                abi::stack_pointer(),
                precision_slot,
            ));
        }

        self.reset_temporary_registers();
        let value_register = self.allocate_register();
        self.emit(abi::load_u64(
            value_register,
            abi::stack_pointer(),
            value_slot,
        ));

        // Every arm is mutually exclusive (a variant, or a specific nominal, or
        // exactly `List OF Byte`), so the reordering the variant patterns impose
        // is not observable — `Scalar` and `AttributedString` are `Named`, which
        // no scalar variant can also match.
        // plan-118-C: which arm each `toString` site takes, so "out-line the
        // expensive kinds" is chosen from the corpus rather than from the
        // static size of the emitters. Counted under `-vv` only.
        crate::trace::count(
            match &value.type_ {
                ParameterType::String => "toString arm: String",
                ParameterType::Boolean => "toString arm: Boolean",
                ParameterType::Byte => "toString arm: Byte",
                ParameterType::Integer => "toString arm: Integer",
                ParameterType::Fixed => "toString arm: Fixed",
                ParameterType::Float => "toString arm: Float",
                ParameterType::Money => "toString arm: Money",
                type_ if type_.is_named("Scalar") => "toString arm: Scalar",
                type_ if type_.is_named("AttributedString") => "toString arm: AttributedString",
                ParameterType::ListOf(element) if **element == ParameterType::Byte => {
                    "toString arm: List OF Byte"
                }
                _ => "toString arm: (unsupported)",
            },
            1,
        );
        match &value.type_ {
            // The IDENTITY arm: `toString` of a `String` IS its argument. The
            // block is the same one, but it leaves here under a different operand
            // (the argument was spilled and reloaded across
            // `reset_temporary_registers`), so the argument's pending-temp
            // registration must be re-identified onto this result or the
            // statement-scope free outlives the owner that claims it — a
            // use-after-free, not a leak. See `retarget_pending_temp`.
            //
            // bug-667: the identity is only sound for a block this statement owns
            // (its most recent pending temporary). Anything else — a binding's
            // block, a parameter, a literal — is copied: handed back as-is, an
            // owning store took it as its own, so `s = toString(s)` freed the
            // block it stored.
            ParameterType::String => {
                let location = Operand::from(value_register);
                if !self.pending_temp_would_be_claimed(&value) {
                    let copied = self.copy_flat_block(&ParameterType::String, &location)?;
                    self.mark_fresh_string(Operand::from(copied));
                    return Ok(ValueResult {
                        origin: None,
                        type_: ParameterType::String,
                        location: Operand::from(copied),
                        text: format!("toString({})", value.text),
                    });
                }
                self.retarget_pending_temp(&value.location, &location);
                Ok(ValueResult {
                    origin: None,
                    type_: ParameterType::String,
                    location,
                    text: format!("toString({})", value.text),
                })
            }
            ParameterType::Boolean => self.lower_boolean_to_string(&value_register),
            ParameterType::Byte => self.emit_integer_to_string_value(&value_register, false),
            type_ if type_.is_named("Scalar") => self.emit_to_string_helper_call(
                ToStringHelper::Scalar,
                &value_register,
                None,
                "toString(Scalar)",
            ),
            ParameterType::Integer => self.emit_integer_to_string_value(&value_register, true),
            // NOT out-of-lined, unlike the three arms around it. This one can fail
            // TWO ways — `ErrEncoding` for invalid UTF-8 as well as
            // `ErrOutOfMemory` — and the helper contract collapses any failure
            // into the single code the call site re-raises. Routing it through a
            // helper turned `toString(<invalid bytes>)` from "Text encoding or
            // decoding failed" into "Allocation failed", caught by
            // `rt-error/general/toString_invalid_encoding`. See plan-118-C
            // Corrections.
            ParameterType::ListOf(element) if **element == ParameterType::Byte => {
                self.emit_byte_list_to_string_value(&value_register)
            }
            ParameterType::Fixed => {
                let precision = self.allocate_register();
                self.emit(abi::load_u64(
                    &precision,
                    abi::stack_pointer(),
                    precision_slot,
                ));
                self.emit_to_string_helper_call(
                    ToStringHelper::Fixed,
                    &value_register,
                    Some(Operand::from(precision)),
                    "toString(Fixed)",
                )
            }
            ParameterType::Float => {
                let precision = self.allocate_register();
                self.emit(abi::load_u64(
                    &precision,
                    abi::stack_pointer(),
                    precision_slot,
                ));
                self.emit_float_to_string_value(&value_register, &precision)
            }
            ParameterType::Money => {
                let precision = self.allocate_register();
                self.emit(abi::load_u64(
                    &precision,
                    abi::stack_pointer(),
                    precision_slot,
                ));
                self.emit_to_string_helper_call(
                    ToStringHelper::Money,
                    &value_register,
                    Some(Operand::from(precision)),
                    "toString(Money)",
                )
            }
            type_ if type_.is_named("AttributedString") => {
                // The visible text is the inlined `text` String field (index 0):
                // its record slot holds a block-relative offset, so the String
                // block is `value_register + offset` (plan-02 §4.2). `toString`
                // yields an OWNED String, so deep-copy that block — returning the
                // alias would share the record's arena memory, freed at the
                // record's scope drop (plan-89-A).
                let text_ptr = self.allocate_register();
                self.emit(abi::load_u64(&text_ptr, &value_register, 0));
                self.emit(abi::add_registers(&text_ptr, &value_register, &text_ptr));
                let copied = self.copy_flat_block(&ParameterType::String, &text_ptr)?;
                // bug-536 shape B: `copied` is the `copy_flat_block` above —
                // a fresh arena block, deliberately not the record's inlined
                // text field.
                self.mark_fresh_string(Operand::from(copied));
                Ok(ValueResult {
                    origin: None,
                    type_: ParameterType::String,
                    location: Operand::from(copied),
                    text: format!("toString({})", value.text),
                })
            }
            other => Err(format!(
                "native toString does not accept argument type '{other}'"
            )),
        }
    }

    pub(crate) fn lower_boolean_to_string(
        &mut self,
        value_register: impl Into<abi::Reg>,
    ) -> Result<ValueResult, String> {
        let false_label = self.label("bool_string_false");
        let done = self.label("bool_string_done");
        let result = self.allocate_register();
        self.emit(abi::compare_immediate(value_register, "0"));
        self.emit(abi::branch_eq(&false_label));
        self.emit_load_string_constant(&result, "TRUE")?;
        self.emit(abi::branch(&done));
        self.emit(abi::label(&false_label));
        self.emit_load_string_constant(&result, "FALSE")?;
        self.emit(abi::label(&done));
        // bug-667: the constant is copied into a fresh block, as the enum arm does.
        // Handed back read-only, a binding took it as its own and freed it (or grew
        // it in place) — SIGBUS.
        let copied = self.copy_flat_block(&ParameterType::String, &result)?;
        self.mark_fresh_string(Operand::from(copied));
        Ok(ValueResult {
            origin: None,
            type_: ParameterType::String,
            location: Operand::from(copied),
            text: "toString(Boolean)".to_string(),
        })
    }

    /// `toString(Integer)` / `toString(Byte)` — marshal, call, check.
    ///
    /// plan-118-C: the digit loop, the sign, the arena allocation and the copy
    /// used to be emitted at every site (~100 instructions each, over 5,826
    /// `toString` sites). They are `_mfb_rt_int_to_string` now, exactly as the
    /// `Float` arm beside this one has always called `_mfb_rt_float_to_string`,
    /// and with the same Result ABI so both sites read the same.
    ///
    /// The `ErrOutOfMemory` raise stays here: its `ErrorLoc` names the
    /// `toString` the program wrote, and a shared helper has no line or column.
    pub(crate) fn emit_integer_to_string_value(
        &mut self,
        source_register: impl Into<abi::Reg>,
        signed: bool,
    ) -> Result<ValueResult, String> {
        let alloc_ok = self.label("int_string_alloc_ok");
        self.emit(abi::move_register(abi::c_arg(0), source_register));
        self.emit(abi::move_immediate(
            abi::c_arg(1),
            "Integer",
            if signed { "1" } else { "0" },
        ));
        self.emit(abi::branch_link(INT_TO_STRING_SYMBOL));
        self.push_internal_call_relocation(INT_TO_STRING_SYMBOL);
        self.emit(abi::compare_immediate(
            abi::return_register(),
            RESULT_OK_TAG,
        ));
        self.emit(abi::branch_eq(&alloc_ok));
        self.raise_error_bare("ErrOutOfMemory")?;
        self.emit(abi::label(&alloc_ok));
        let result = self.allocate_register();
        self.emit(abi::move_register(&result, abi::mfb_return(1)));
        // bug-536 shape B: `_mfb_rt_int_to_string` renders into its own stack
        // buffer, `_mfb_arena_alloc`s the String block and copies the digits in
        // (`lower_int_to_string_helper`); the argument is an Integer, so no
        // caller storage can be aliased. Fresh — an unbound `toString(i)` is
        // freed at statement end.
        self.mark_fresh_string(Operand::from(result));
        Ok(ValueResult {
            origin: None,
            type_: ParameterType::String,
            location: Operand::from(result),
            text: if signed {
                "toString(Integer)".to_string()
            } else {
                "toString(Byte)".to_string()
            },
        })
    }

    pub(crate) fn emit_byte_list_to_string_value(
        &mut self,
        source_register: impl Into<abi::Reg>,
    ) -> Result<ValueResult, String> {
        let scratch8 = self.temporary_vreg();
        let scratch9 = self.temporary_vreg();
        let scratch10 = self.temporary_vreg();
        let scratch11 = self.temporary_vreg();
        let scratch12 = self.temporary_vreg();
        let scratch13 = self.temporary_vreg();
        let scratch14 = self.temporary_vreg();
        let scratch15 = self.temporary_vreg();
        let scratch16 = self.temporary_vreg();
        let scratch17 = self.temporary_vreg();
        let list_slot = self.allocate_stack_object("to_string_byte_list", 8);
        let length_slot = self.allocate_stack_object("to_string_byte_list_length", 8);
        let data_slot = self.allocate_stack_object("to_string_byte_list_data", 8);
        let result_slot = self.allocate_stack_object("to_string_byte_list_result", 8);

        let list = &scratch8;
        let length = &scratch9;
        let index = &scratch10;
        let offset = &scratch11;
        let byte = &scratch12;
        let byte2 = &scratch13;
        let byte3 = &scratch14;
        let byte4 = &scratch15;
        let result = &scratch16;
        let dst = &scratch17;

        let validate_loop = self.label("byte_list_string_validate_loop");
        let validate_done = self.label("byte_list_string_validate_done");
        let invalid = self.label("byte_list_string_invalid");
        let ascii = self.label("byte_list_string_ascii");
        let two = self.label("byte_list_string_two");
        let three = self.label("byte_list_string_three");
        let three_e0 = self.label("byte_list_string_three_e0");
        let three_ed = self.label("byte_list_string_three_ed");
        let three_mid = self.label("byte_list_string_three_mid");
        let four = self.label("byte_list_string_four");
        let four_f0 = self.label("byte_list_string_four_f0");
        let four_f4 = self.label("byte_list_string_four_f4");
        let four_mid = self.label("byte_list_string_four_mid");
        let alloc_ok = self.label("byte_list_string_alloc_ok");
        let copy_loop = self.label("byte_list_string_copy_loop");
        let copy_done = self.label("byte_list_string_copy_done");

        self.emit(abi::move_register(list, source_register));
        self.emit(abi::store_u64(list, abi::stack_pointer(), list_slot));
        self.emit(abi::load_u64(length, list, COLLECTION_OFFSET_COUNT));
        self.emit(abi::store_u64(length, abi::stack_pointer(), length_slot));
        self.emit_collection_data_pointer_for(offset, list, &ParameterType::Byte);
        self.emit(abi::store_u64(offset, abi::stack_pointer(), data_slot));
        self.emit(abi::move_immediate(index, "Integer", "0"));

        self.emit(abi::label(&validate_loop));
        self.emit(abi::compare_registers(index, length));
        self.emit(abi::branch_ge(&validate_done));
        self.emit(abi::load_u64(offset, abi::stack_pointer(), data_slot));
        self.emit(abi::add_registers(offset, offset, index));
        self.emit(abi::load_u8(byte, offset, 0));

        self.emit(abi::compare_immediate(byte, "128"));
        self.emit(abi::branch_lo(&ascii));

        self.emit(abi::compare_immediate(byte, "194"));
        self.emit(abi::branch_lo(&invalid));
        self.emit(abi::compare_immediate(byte, "224"));
        self.emit(abi::branch_lo(&two));
        self.emit(abi::compare_immediate(byte, "240"));
        self.emit(abi::branch_lo(&three));
        self.emit(abi::compare_immediate(byte, "245"));
        self.emit(abi::branch_lo(&four));
        self.emit(abi::branch(&invalid));

        self.emit(abi::label(&ascii));
        self.emit(abi::add_immediate(index, index, 1));
        self.emit(abi::branch(&validate_loop));

        self.emit(abi::label(&two));
        self.emit(abi::add_immediate(index, index, 1));
        self.emit(abi::compare_registers(index, length));
        self.emit(abi::branch_ge(&invalid));
        self.emit(abi::load_u64(offset, abi::stack_pointer(), data_slot));
        self.emit(abi::add_registers(offset, offset, index));
        self.emit(abi::load_u8(byte2, offset, 0));
        self.emit(abi::compare_immediate(byte2, "128"));
        self.emit(abi::branch_lo(&invalid));
        self.emit(abi::compare_immediate(byte2, "192"));
        self.emit(abi::branch_ge(&invalid));
        self.emit(abi::add_immediate(index, index, 1));
        self.emit(abi::branch(&validate_loop));

        self.emit(abi::label(&three));
        self.emit(abi::add_immediate(index, index, 2));
        self.emit(abi::compare_registers(index, length));
        self.emit(abi::branch_ge(&invalid));
        self.emit(abi::subtract_immediate(index, index, 2));
        self.emit(abi::load_u64(offset, abi::stack_pointer(), data_slot));
        self.emit(abi::add_registers(offset, offset, index));
        self.emit(abi::add_immediate(offset, offset, 1));
        self.emit(abi::load_u8(byte2, offset, 0));
        self.emit(abi::load_u64(offset, abi::stack_pointer(), data_slot));
        self.emit(abi::add_registers(offset, offset, index));
        self.emit(abi::add_immediate(offset, offset, 2));
        self.emit(abi::load_u8(byte3, offset, 0));
        self.emit(abi::compare_immediate(byte3, "128"));
        self.emit(abi::branch_lo(&invalid));
        self.emit(abi::compare_immediate(byte3, "192"));
        self.emit(abi::branch_ge(&invalid));
        self.emit(abi::compare_immediate(byte, "224"));
        self.emit(abi::branch_eq(&three_e0));
        self.emit(abi::compare_immediate(byte, "237"));
        self.emit(abi::branch_eq(&three_ed));
        self.emit(abi::branch(&three_mid));

        self.emit(abi::label(&three_e0));
        self.emit(abi::compare_immediate(byte2, "160"));
        self.emit(abi::branch_lo(&invalid));
        self.emit(abi::compare_immediate(byte2, "192"));
        self.emit(abi::branch_ge(&invalid));
        self.emit(abi::add_immediate(index, index, 3));
        self.emit(abi::branch(&validate_loop));

        self.emit(abi::label(&three_ed));
        self.emit(abi::compare_immediate(byte2, "128"));
        self.emit(abi::branch_lo(&invalid));
        self.emit(abi::compare_immediate(byte2, "160"));
        self.emit(abi::branch_ge(&invalid));
        self.emit(abi::add_immediate(index, index, 3));
        self.emit(abi::branch(&validate_loop));

        self.emit(abi::label(&three_mid));
        self.emit(abi::compare_immediate(byte2, "128"));
        self.emit(abi::branch_lo(&invalid));
        self.emit(abi::compare_immediate(byte2, "192"));
        self.emit(abi::branch_ge(&invalid));
        self.emit(abi::add_immediate(index, index, 3));
        self.emit(abi::branch(&validate_loop));

        self.emit(abi::label(&four));
        self.emit(abi::add_immediate(index, index, 3));
        self.emit(abi::compare_registers(index, length));
        self.emit(abi::branch_ge(&invalid));
        self.emit(abi::subtract_immediate(index, index, 3));
        self.emit(abi::load_u64(offset, abi::stack_pointer(), data_slot));
        self.emit(abi::add_registers(offset, offset, index));
        self.emit(abi::add_immediate(offset, offset, 1));
        self.emit(abi::load_u8(byte2, offset, 0));
        self.emit(abi::load_u64(offset, abi::stack_pointer(), data_slot));
        self.emit(abi::add_registers(offset, offset, index));
        self.emit(abi::add_immediate(offset, offset, 2));
        self.emit(abi::load_u8(byte3, offset, 0));
        self.emit(abi::load_u64(offset, abi::stack_pointer(), data_slot));
        self.emit(abi::add_registers(offset, offset, index));
        self.emit(abi::add_immediate(offset, offset, 3));
        self.emit(abi::load_u8(byte4, offset, 0));
        for continuation in [byte3, byte4] {
            self.emit(abi::compare_immediate(continuation, "128"));
            self.emit(abi::branch_lo(&invalid));
            self.emit(abi::compare_immediate(continuation, "192"));
            self.emit(abi::branch_ge(&invalid));
        }
        self.emit(abi::compare_immediate(byte, "240"));
        self.emit(abi::branch_eq(&four_f0));
        self.emit(abi::compare_immediate(byte, "244"));
        self.emit(abi::branch_eq(&four_f4));
        self.emit(abi::branch(&four_mid));

        self.emit(abi::label(&four_f0));
        self.emit(abi::compare_immediate(byte2, "144"));
        self.emit(abi::branch_lo(&invalid));
        self.emit(abi::compare_immediate(byte2, "192"));
        self.emit(abi::branch_ge(&invalid));
        self.emit(abi::add_immediate(index, index, 4));
        self.emit(abi::branch(&validate_loop));

        self.emit(abi::label(&four_f4));
        self.emit(abi::compare_immediate(byte2, "128"));
        self.emit(abi::branch_lo(&invalid));
        self.emit(abi::compare_immediate(byte2, "144"));
        self.emit(abi::branch_ge(&invalid));
        self.emit(abi::add_immediate(index, index, 4));
        self.emit(abi::branch(&validate_loop));

        self.emit(abi::label(&four_mid));
        self.emit(abi::compare_immediate(byte2, "128"));
        self.emit(abi::branch_lo(&invalid));
        self.emit(abi::compare_immediate(byte2, "192"));
        self.emit(abi::branch_ge(&invalid));
        self.emit(abi::add_immediate(index, index, 4));
        self.emit(abi::branch(&validate_loop));

        self.emit(abi::label(&invalid));
        self.raise_error_bare("ErrEncoding")?;

        self.emit(abi::label(&validate_done));
        self.emit(abi::load_u64(length, abi::stack_pointer(), length_slot));
        // plan-71-C Family-1a: alloc size is arg 0 of the arena-alloc call → `%arg0`.
        self.emit(abi::add_immediate(abi::c_arg(0), length, 9));
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
        self.emit(abi::load_u64(length, abi::stack_pointer(), length_slot));
        self.emit(abi::store_u64(length, abi::mfb_return(1), 0));
        self.emit(abi::add_immediate(dst, abi::mfb_return(1), 8));
        self.emit(abi::move_immediate(index, "Integer", "0"));
        self.emit(abi::load_u64(list, abi::stack_pointer(), list_slot));

        self.emit(abi::label(&copy_loop));
        self.emit(abi::compare_registers(index, length));
        self.emit(abi::branch_ge(&copy_done));
        self.emit(abi::load_u64(offset, abi::stack_pointer(), data_slot));
        self.emit(abi::add_registers(offset, offset, index));
        self.emit(abi::load_u8(byte, offset, 0));
        self.emit(abi::store_u8(byte, dst, 0));
        self.emit(abi::add_immediate(dst, dst, 1));
        self.emit(abi::add_immediate(index, index, 1));
        self.emit(abi::branch(&copy_loop));

        self.emit(abi::label(&copy_done));
        self.emit(abi::move_immediate(byte, "Integer", "0"));
        self.emit(abi::store_u8(byte, dst, 0));
        self.emit(abi::load_u64(result, abi::stack_pointer(), result_slot));

        // bug-536 shape B: `result_slot` holds this lowering's own
        // `emit_arena_alloc_call` block, with the list's bytes copied in.
        self.mark_fresh_string(Operand::from(result));
        Ok(ValueResult {
            origin: None,
            type_: ParameterType::String,
            location: Operand::from(result),
            text: "toString(List OF Byte)".to_string(),
        })
    }
}
