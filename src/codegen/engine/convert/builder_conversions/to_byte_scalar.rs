//! plan-176-U: the `toByte` and `toScalar` conversions and the scalar/string helpers. The other conversions live in
//! `engine/convert/builder_conversions/mod.rs`.

use super::*;

impl CodeBuilder<'_> {
    pub(crate) fn lower_to_byte(&mut self, arg: &NirValue) -> Result<ValueResult, String> {
        let value = self.lower_value(arg)?;
        if !matches!(value.type_.name().as_ref(), "Integer" | "Money" | "Scalar") {
            return Err(format!(
                "native toByte does not accept argument type '{}'",
                value.type_
            ));
        }
        // `toByte(Money)` narrows the whole-unit part (`raw / 100000`), then
        // range-checks it exactly like an Integer (plan-29-G §4.3).
        let checked = if value.type_ == ParameterType::Money {
            let scale = self.allocate_register();
            let whole = self.allocate_register();
            self.emit(abi::move_immediate(&scale, "Integer", "100000"));
            self.emit(abi::signed_divide_registers(
                &whole,
                &value.location,
                &scale,
            ));
            Operand::from(whole)
        } else {
            value.location.clone()
        };
        let result = self.allocate_register();
        let overflow = self.label("to_byte_overflow");
        let ok = self.label("to_byte_ok");
        self.emit(abi::compare_immediate(&checked, "0"));
        self.emit(abi::branch_lt(&overflow));
        self.emit(abi::compare_immediate(&checked, "255"));
        self.emit(abi::branch_hi(&overflow));
        self.emit(abi::move_register(&result, &checked));
        self.emit(abi::branch(&ok));
        self.emit(abi::label(&overflow));
        self.raise_error("toByte", "ErrOverflow")?;
        self.emit(abi::label(&ok));
        Ok(ValueResult {
            origin: None,
            type_: ParameterType::Byte,
            location: Operand::from(result),
            text: format!("toByte({})", value.text),
        })
    }

    /// `toScalar(Integer|String|Byte) -> Scalar` (plan-41-D). `Byte` is an
    /// infallible widen (every byte is a valid scalar); `Integer` fails
    /// `ErrInvalidArgument` for a surrogate (U+D800..U+DFFF) or a value outside
    /// 0..U+10FFFF; `String` decodes the single scalar of a one-scalar string,
    /// failing for an empty or multi-scalar string.
    pub(crate) fn lower_to_scalar(&mut self, arg: &NirValue) -> Result<ValueResult, String> {
        let value = self.lower_value(arg)?;
        match &value.type_ {
            ParameterType::Byte => {
                let register = self.allocate_register();
                self.emit(abi::move_register(&register, &value.location));
                Ok(ValueResult {
                    origin: None,
                    type_: ParameterType::named("Scalar"),
                    location: Operand::from(register),
                    text: format!("toScalar({})", value.text),
                })
            }
            ParameterType::Integer => {
                let cp = value.location.clone();
                let ok = self.label("to_scalar_ok");
                let invalid = self.label("to_scalar_invalid");
                let not_surrogate = self.label("to_scalar_not_surrogate");
                // cp < 0 -> invalid.
                self.emit(abi::compare_immediate(&cp, "0"));
                self.emit(abi::branch_lt(&invalid));
                // cp > 0x10FFFF (1114111) -> invalid.
                self.emit(abi::compare_immediate(&cp, "1114111"));
                self.emit(abi::branch_hi(&invalid));
                // Surrogate band 0xD800..0xDFFF (55296..57343) -> invalid.
                self.emit(abi::compare_immediate(&cp, "55296"));
                self.emit(abi::branch_lo(&not_surrogate));
                self.emit(abi::compare_immediate(&cp, "57343"));
                self.emit(abi::branch_le(&invalid));
                self.emit(abi::label(&not_surrogate));
                self.emit(abi::branch(&ok));
                self.emit(abi::label(&invalid));
                self.raise_error("toScalar", "ErrInvalidArgument")?;
                self.emit(abi::label(&ok));
                let register = self.allocate_register();
                self.emit(abi::move_register(&register, &cp));
                Ok(ValueResult {
                    origin: None,
                    type_: ParameterType::named("Scalar"),
                    location: Operand::from(register),
                    text: format!("toScalar({})", value.text),
                })
            }
            ParameterType::String => {
                let result = self.emit_string_to_scalar_value(&value.location)?;
                Ok(ValueResult {
                    origin: None,
                    type_: ParameterType::named("Scalar"),
                    location: Operand::from(result),
                    text: format!("toScalar({})", value.text),
                })
            }
            other => Err(format!(
                "native toScalar does not accept argument type '{other}'"
            )),
        }
    }

    /// Decode the single Unicode scalar of a one-scalar `String` into a codepoint
    /// register. A `String` is guaranteed valid UTF-8, so the decoder trusts
    /// well-formedness and only enforces "exactly one scalar": it computes the
    /// lead byte's expected length, reassembles the codepoint, and traps
    /// `ErrInvalidArgument` when the string is empty or its byte length differs
    /// from that scalar's length (i.e. zero or more than one scalar).
    /// Reject a UTF-8 byte that is not a continuation byte, i.e. one whose top two
    /// bits are not `10` (bug-312 K2).
    ///
    /// `scratch` is clobbered; callers re-materialize the 0x3F payload mask after
    /// calling, since this reuses the same register for the 0xC0 test.
    fn emit_continuation_byte_check(
        &mut self,
        byte: impl Into<abi::Reg>,
        scratch: impl Into<abi::Reg>,
        invalid: &str,
    ) {
        let scratch: abi::Reg = scratch.into();
        self.emit(abi::move_immediate(scratch, "Integer", "192")); // 0xC0
        self.emit(abi::and_registers(scratch, byte, scratch));
        self.emit(abi::compare_immediate(scratch, "128")); // 0x80
        self.emit(abi::branch_ne(invalid));
    }

    pub(crate) fn emit_string_to_scalar_value(
        &mut self,
        source_register: impl Into<abi::Reg>,
    ) -> Result<VirtualRegister, String> {
        let string_v = self.temporary_vreg();
        let length_v = self.temporary_vreg();
        let b0_v = self.temporary_vreg();
        let cp_v = self.temporary_vreg();
        let cont_v = self.temporary_vreg();
        let mask_v = self.temporary_vreg();
        let nbytes_v = self.temporary_vreg();
        let string = &string_v;
        let length = &length_v;
        let b0 = &b0_v;
        let cp = &cp_v;
        let cont = &cont_v;
        let mask = &mask_v;
        let nbytes = &nbytes_v;

        let one_byte = self.label("str_scalar_one");
        let two_byte = self.label("str_scalar_two");
        let three_byte = self.label("str_scalar_three");
        let four_byte = self.label("str_scalar_four");
        let assembled = self.label("str_scalar_assembled");
        let invalid = self.label("str_scalar_invalid");
        let ok = self.label("str_scalar_ok");

        self.emit(abi::move_register(string, source_register));
        self.emit(abi::load_u64(length, string, 0));
        // Empty string -> invalid.
        self.emit(abi::compare_immediate(length, "0"));
        self.emit(abi::branch_eq(&invalid));
        self.emit(abi::load_u8(b0, string, 8));
        // Classify the lead byte by its high bits.
        self.emit(abi::compare_immediate(b0, "128")); // < 0x80 -> 1 byte
        self.emit(abi::branch_lo(&one_byte));
        self.emit(abi::compare_immediate(b0, "192")); // 0x80..0xBF lead -> invalid
        self.emit(abi::branch_lo(&invalid));
        self.emit(abi::compare_immediate(b0, "224")); // < 0xE0 -> 2 bytes
        self.emit(abi::branch_lo(&two_byte));
        self.emit(abi::compare_immediate(b0, "240")); // < 0xF0 -> 3 bytes
        self.emit(abi::branch_lo(&three_byte));
        self.emit(abi::compare_immediate(b0, "248")); // < 0xF8 -> 4 bytes
        self.emit(abi::branch_lo(&four_byte));
        self.emit(abi::branch(&invalid));

        // 1 byte: cp = b0.
        self.emit(abi::label(&one_byte));
        self.emit(abi::move_immediate(nbytes, "Integer", "1"));
        self.emit(abi::move_register(cp, b0));
        self.emit(abi::branch(&assembled));

        // 2 bytes: cp = (b0 & 0x1F) << 6 | (b1 & 0x3F).
        self.emit(abi::label(&two_byte));
        self.emit(abi::move_immediate(nbytes, "Integer", "2"));
        // bug-312 K2: check the length BEFORE the fixed-offset continuation reads
        // below. The "exactly one scalar" check at `assembled` runs after them, so
        // a 2-byte lead on a shorter buffer read past the allocation before
        // anything rejected it. Safe today only because a `String` is guaranteed
        // well-formed UTF-8; the sibling decoders (`emit_utf8_decode_next`, the
        // padChar check) were hardened for exactly this in audit-unicode and this
        // one was left trusting.
        self.emit(abi::compare_registers(length, nbytes));
        self.emit(abi::branch_lt(&invalid));
        self.emit(abi::move_immediate(mask, "Integer", "31"));
        self.emit(abi::and_registers(cp, b0, mask));
        self.emit(abi::shift_left_immediate(cp, cp, 6));
        self.emit(abi::load_u8(cont, string, 9));
        self.emit_continuation_byte_check(cont, mask, &invalid);
        self.emit(abi::move_immediate(mask, "Integer", "63"));
        self.emit(abi::and_registers(cont, cont, mask));
        self.emit(abi::or_registers(cp, cp, cont));
        self.emit(abi::branch(&assembled));

        // 3 bytes: cp = (b0 & 0x0F) << 12 | (b1 & 0x3F) << 6 | (b2 & 0x3F).
        self.emit(abi::label(&three_byte));
        self.emit(abi::move_immediate(nbytes, "Integer", "3"));
        // bug-312 K2: check the length BEFORE the fixed-offset continuation reads
        // below. The "exactly one scalar" check at `assembled` runs after them, so
        // a 3-byte lead on a shorter buffer read past the allocation before
        // anything rejected it. Safe today only because a `String` is guaranteed
        // well-formed UTF-8; the sibling decoders (`emit_utf8_decode_next`, the
        // padChar check) were hardened for exactly this in audit-unicode and this
        // one was left trusting.
        self.emit(abi::compare_registers(length, nbytes));
        self.emit(abi::branch_lt(&invalid));
        self.emit(abi::move_immediate(mask, "Integer", "15"));
        self.emit(abi::and_registers(cp, b0, mask));
        self.emit(abi::shift_left_immediate(cp, cp, 12));
        self.emit(abi::load_u8(cont, string, 9));
        self.emit_continuation_byte_check(cont, mask, &invalid);
        self.emit(abi::move_immediate(mask, "Integer", "63"));
        self.emit(abi::and_registers(cont, cont, mask));
        self.emit(abi::shift_left_immediate(cont, cont, 6));
        self.emit(abi::or_registers(cp, cp, cont));
        self.emit(abi::load_u8(cont, string, 10));
        self.emit_continuation_byte_check(cont, mask, &invalid);
        self.emit(abi::move_immediate(mask, "Integer", "63"));
        self.emit(abi::and_registers(cont, cont, mask));
        self.emit(abi::or_registers(cp, cp, cont));
        self.emit(abi::branch(&assembled));

        // 4 bytes: cp = (b0 & 0x07)<<18 | (b1&0x3F)<<12 | (b2&0x3F)<<6 | (b3&0x3F).
        self.emit(abi::label(&four_byte));
        self.emit(abi::move_immediate(nbytes, "Integer", "4"));
        // bug-312 K2: check the length BEFORE the fixed-offset continuation reads
        // below. The "exactly one scalar" check at `assembled` runs after them, so
        // a 4-byte lead on a shorter buffer read past the allocation before
        // anything rejected it. Safe today only because a `String` is guaranteed
        // well-formed UTF-8; the sibling decoders (`emit_utf8_decode_next`, the
        // padChar check) were hardened for exactly this in audit-unicode and this
        // one was left trusting.
        self.emit(abi::compare_registers(length, nbytes));
        self.emit(abi::branch_lt(&invalid));
        self.emit(abi::move_immediate(mask, "Integer", "7"));
        self.emit(abi::and_registers(cp, b0, mask));
        self.emit(abi::shift_left_immediate(cp, cp, 18));
        self.emit(abi::load_u8(cont, string, 9));
        self.emit_continuation_byte_check(cont, mask, &invalid);
        self.emit(abi::move_immediate(mask, "Integer", "63"));
        self.emit(abi::and_registers(cont, cont, mask));
        self.emit(abi::shift_left_immediate(cont, cont, 12));
        self.emit(abi::or_registers(cp, cp, cont));
        self.emit(abi::load_u8(cont, string, 10));
        self.emit_continuation_byte_check(cont, mask, &invalid);
        self.emit(abi::move_immediate(mask, "Integer", "63"));
        self.emit(abi::and_registers(cont, cont, mask));
        self.emit(abi::shift_left_immediate(cont, cont, 6));
        self.emit(abi::or_registers(cp, cp, cont));
        self.emit(abi::load_u8(cont, string, 11));
        self.emit_continuation_byte_check(cont, mask, &invalid);
        self.emit(abi::move_immediate(mask, "Integer", "63"));
        self.emit(abi::and_registers(cont, cont, mask));
        self.emit(abi::or_registers(cp, cp, cont));
        self.emit(abi::branch(&assembled));

        // Exactly-one-scalar check: the byte length must equal the lead byte's
        // expected length; anything else is zero or more than one scalar.
        self.emit(abi::label(&assembled));
        self.emit(abi::compare_registers(length, nbytes));
        self.emit(abi::branch_ne(&invalid));
        self.emit(abi::branch(&ok));
        self.emit(abi::label(&invalid));
        self.raise_error_bare("ErrInvalidArgument")?;
        self.emit(abi::label(&ok));
        let result = self.allocate_register();
        self.emit(abi::move_register(&result, cp));
        Ok(result)
    }

    /// `toString(Scalar) -> String` (plan-41-D): UTF-8-encode the one codepoint
    /// into a fresh 1–4 byte `String`. Infallible — every valid `Scalar` is a
    /// valid UTF-8 string. Writes the encoded bytes into a stack buffer, then
    /// materializes an owned arena `String` from them.
    pub(crate) fn emit_scalar_to_string_value(
        &mut self,
        source_register: impl Into<abi::Reg>,
    ) -> Result<ValueResult, String> {
        let cp_v = self.temporary_vreg();
        let buf_v = self.temporary_vreg();
        let len_v = self.temporary_vreg();
        let cp = &cp_v;
        let buf = &buf_v;
        let len = &len_v;
        let buf_slot = self.allocate_stack_object("scalar_utf8_buf", 8);

        // S3 (bug-333): route through the canonical UTF-8 codec in
        // `private/unicode.rs` instead of a second open-coded encoder. The width
        // helper sets `len`; the encode helper writes the bytes at `buf` (and
        // advances it, which is why the buffer address is re-derived below).
        self.emit(abi::move_register(cp, source_register));
        self.emit(abi::add_immediate(buf, abi::stack_pointer(), buf_slot));
        self.emit_utf8_encoded_width(cp, len);
        self.emit_utf8_encode_next(buf, cp);

        // Re-derive the buffer address (the encode helper advanced `buf`, and the
        // arena call inside materialize spills it) and build the owned String.
        let buf_addr = self.allocate_register();
        self.emit(abi::add_immediate(
            &buf_addr,
            abi::stack_pointer(),
            buf_slot,
        ));
        let result = self.emit_materialize_string_from_bytes(&buf_addr, len)?;
        Ok(ValueResult {
            origin: None,
            type_: ParameterType::String,
            location: Operand::from(result),
            text: "toString(Scalar)".to_string(),
        })
    }
}
