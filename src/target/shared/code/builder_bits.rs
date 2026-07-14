use super::*;

// 64-bit population-count masks (SWAR Hamming weight), as decimal so they round
// trip through `move_immediate`'s arbitrary-constant path.
const POPCOUNT_MASK_5555: &str = "6148914691236517205"; // 0x5555555555555555
const POPCOUNT_MASK_3333: &str = "3689348814741910323"; // 0x3333333333333333
const POPCOUNT_MASK_0F0F: &str = "1085102592571150095"; // 0x0F0F0F0F0F0F0F0F
const POPCOUNT_MASK_0101: &str = "72340172838076673"; //  0x0101010101010101

impl CodeBuilder<'_> {
    pub(super) fn lower_bits_call(
        &mut self,
        function: &str,
        args: &[NirValue],
    ) -> Result<ValueResult, String> {
        match function {
            "band" | "bor" | "bxor" if args.len() == 2 => self.lower_bits_binary(function, args),
            "bnot" if args.len() == 1 => self.lower_bits_not(&args[0]),
            "sl" | "sr" | "sra" if args.len() == 2 => self.lower_bits_shift(function, args),
            "rl32" | "rr32" | "rl64" | "rr64" if args.len() == 2 => {
                self.lower_bits_rotate(function, args)
            }
            "clz" | "ctz" if args.len() == 1 => self.lower_bits_count_zeros(function, &args[0]),
            "popCount" if args.len() == 1 => self.lower_bits_popcount(&args[0]),
            "bswap16" | "bswap32" | "bswap64" if args.len() == 1 => {
                self.lower_bits_bswap(function, &args[0])
            }
            other => Err(format!(
                "native bits lowering does not support bits.{other}"
            )),
        }
    }

    /// Lower the two `Integer` operands of a binary `bits` op into fresh
    /// registers, spilling the first across the second lowering so a temporary
    /// reset cannot clobber it.
    fn lower_bits_two_integers(
        &mut self,
        function: &str,
        args: &[NirValue],
    ) -> Result<(String, String, String, String), String> {
        let left = self.lower_value(&args[0])?;
        if left.type_ != "Integer" {
            return Err(format!("bits.{function} does not accept {}", left.type_));
        }
        let left_slot = self.allocate_stack_object("bits_left", 8);
        self.emit(abi::store_u64(
            &left.location,
            abi::stack_pointer(),
            left_slot,
        ));
        let right = self.lower_value(&args[1])?;
        if right.type_ != "Integer" {
            return Err(format!("bits.{function} does not accept {}", right.type_));
        }
        let right_slot = self.allocate_stack_object("bits_right", 8);
        self.emit(abi::store_u64(
            &right.location,
            abi::stack_pointer(),
            right_slot,
        ));
        self.reset_temporary_registers();
        let left_reg = self.allocate_register()?;
        let right_reg = self.allocate_register()?;
        self.emit(abi::load_u64(&left_reg, abi::stack_pointer(), left_slot));
        self.emit(abi::load_u64(&right_reg, abi::stack_pointer(), right_slot));
        Ok((left_reg, right_reg, left.text, right.text))
    }

    fn lower_bits_binary(
        &mut self,
        function: &str,
        args: &[NirValue],
    ) -> Result<ValueResult, String> {
        let (left_reg, right_reg, left_text, right_text) =
            self.lower_bits_two_integers(function, args)?;
        let dst = self.allocate_register()?;
        match function {
            "band" => self.emit(abi::and_registers(&dst, &left_reg, &right_reg)),
            "bor" => self.emit(abi::or_registers(&dst, &left_reg, &right_reg)),
            "bxor" => self.emit(abi::exclusive_or_registers(&dst, &left_reg, &right_reg)),
            other => {
                return Err(format!(
                    "native bits lowering does not support bits.{other}"
                ))
            }
        }
        Ok(ValueResult {
            type_: "Integer".to_string(),
            location: dst,
            text: format!("bits.{function}({left_text}, {right_text})"),
        })
    }

    fn lower_bits_not(&mut self, arg: &NirValue) -> Result<ValueResult, String> {
        let value = self.lower_value(arg)?;
        if value.type_ != "Integer" {
            return Err(format!("bits.bnot does not accept {}", value.type_));
        }
        let dst = self.allocate_register()?;
        self.emit(abi::bitwise_not(&dst, &value.location));
        Ok(ValueResult {
            type_: "Integer".to_string(),
            location: dst,
            text: format!("bits.bnot({})", value.text),
        })
    }

    /// `sl`/`sr`/`sra` — variable shift after validating `count` is in `0 .. 63`.
    /// An out-of-range count fails `ErrInvalidArgument`.
    fn lower_bits_shift(
        &mut self,
        function: &str,
        args: &[NirValue],
    ) -> Result<ValueResult, String> {
        let (value_reg, count_reg, value_text, count_text) =
            self.lower_bits_two_integers(function, args)?;
        let valid = self.label("bits_shift_valid");
        let out_of_range = self.label("bits_shift_out_of_range");
        self.emit(abi::compare_immediate(&count_reg, "0"));
        self.emit(abi::branch_lt(&out_of_range));
        self.emit(abi::compare_immediate(&count_reg, "63"));
        self.emit(abi::branch_le(&valid));
        self.emit(abi::label(&out_of_range));
        self.emit_invalid_argument_return()?;
        self.emit(abi::label(&valid));
        let dst = self.allocate_register()?;
        match function {
            "sl" => self.emit(abi::shift_left_variable(&dst, &value_reg, &count_reg)),
            "sr" => self.emit(abi::shift_right_variable(&dst, &value_reg, &count_reg)),
            "sra" => self.emit(abi::arithmetic_shift_right_variable(
                &dst, &value_reg, &count_reg,
            )),
            other => {
                return Err(format!(
                    "native bits lowering does not support bits.{other}"
                ))
            }
        }
        Ok(ValueResult {
            type_: "Integer".to_string(),
            location: dst,
            text: format!("bits.{function}({value_text}, {count_text})"),
        })
    }

    /// `rl32`/`rr32`/`rl64`/`rr64` — total rotates. AArch64 has only rotate-right
    /// (`RORV`), so a left rotate by `count` becomes a right rotate by `-count`
    /// (the hardware reduces the amount modulo the width).
    fn lower_bits_rotate(
        &mut self,
        function: &str,
        args: &[NirValue],
    ) -> Result<ValueResult, String> {
        let (value_reg, count_reg, value_text, count_text) =
            self.lower_bits_two_integers(function, args)?;
        let dst = self.allocate_register()?;
        match function {
            "rr64" => self.emit(abi::rotate_right_registers(&dst, &value_reg, &count_reg)),
            "rl64" => {
                let neg = self.allocate_register()?;
                self.emit(abi::subtract_registers(&neg, abi::ZERO, &count_reg));
                self.emit(abi::rotate_right_registers(&dst, &value_reg, &neg));
            }
            "rr32" => self.emit(abi::rotate_right_word_registers(
                &dst, &value_reg, &count_reg,
            )),
            "rl32" => {
                let neg = self.allocate_register()?;
                self.emit(abi::subtract_registers(&neg, abi::ZERO, &count_reg));
                self.emit(abi::rotate_right_word_registers(&dst, &value_reg, &neg));
            }
            other => {
                return Err(format!(
                    "native bits lowering does not support bits.{other}"
                ))
            }
        }
        Ok(ValueResult {
            type_: "Integer".to_string(),
            location: dst,
            text: format!("bits.{function}({value_text}, {count_text})"),
        })
    }

    /// `clz`/`ctz`. `ctz` reverses the bits (`RBIT`) and then counts leading
    /// zeros; both return `64` for a zero input.
    fn lower_bits_count_zeros(
        &mut self,
        function: &str,
        arg: &NirValue,
    ) -> Result<ValueResult, String> {
        let value = self.lower_value(arg)?;
        if value.type_ != "Integer" {
            return Err(format!("bits.{function} does not accept {}", value.type_));
        }
        let dst = self.allocate_register()?;
        match function {
            "clz" => self.emit(abi::count_leading_zeros(&dst, &value.location)),
            "ctz" => {
                let reversed = self.allocate_register()?;
                self.emit(abi::reverse_bits(&reversed, &value.location));
                self.emit(abi::count_leading_zeros(&dst, &reversed));
            }
            other => {
                return Err(format!(
                    "native bits lowering does not support bits.{other}"
                ))
            }
        }
        Ok(ValueResult {
            type_: "Integer".to_string(),
            location: dst,
            text: format!("bits.{function}({})", value.text),
        })
    }

    /// `popCount` — 64-bit Hamming weight via the standard SWAR sequence (no SIMD,
    /// so it lowers entirely with the integer ALU ops the codegen already owns).
    fn lower_bits_popcount(&mut self, arg: &NirValue) -> Result<ValueResult, String> {
        let value = self.lower_value(arg)?;
        if value.type_ != "Integer" {
            return Err(format!("bits.popCount does not accept {}", value.type_));
        }
        let text = format!("bits.popCount({})", value.text);

        // plan-39 K2: on AArch64 the 64-bit Hamming weight is a short NEON
        // sequence — move the value into a `d` register, `CNT` per byte, `ADDV`
        // the 8 byte-counts into lane 0, and move the (0..=64) sum back — instead
        // of the 12-instruction SWAR. Other ISAs keep the portable SWAR below.
        if mir::active_backend().is_aarch64() {
            let dst = self.allocate_register()?;
            self.emit(abi::vector_dup_from_x(abi::VEC_SCRATCH[0], &value.location));
            self.emit(abi::vector_cnt8b(abi::VEC_SCRATCH[0], abi::VEC_SCRATCH[0]));
            self.emit(abi::vector_addv8b(abi::VEC_SCRATCH[0], abi::VEC_SCRATCH[0]));
            self.emit(abi::vector_extract_to_x(&dst, abi::VEC_SCRATCH[0], 0));
            return Ok(ValueResult {
                type_: "Integer".to_string(),
                location: dst,
                text,
            });
        }

        let acc = self.allocate_register()?;
        let temp = self.allocate_register()?;
        let mask = self.allocate_register()?;
        self.emit(abi::move_register(&acc, &value.location));

        // acc = acc - ((acc >> 1) & 0x5555...)
        self.emit(abi::shift_right_immediate(&temp, &acc, 1));
        self.emit(abi::move_immediate(&mask, "Integer", POPCOUNT_MASK_5555));
        self.emit(abi::and_registers(&temp, &temp, &mask));
        self.emit(abi::subtract_registers(&acc, &acc, &temp));

        // acc = (acc & 0x3333...) + ((acc >> 2) & 0x3333...)
        self.emit(abi::move_immediate(&mask, "Integer", POPCOUNT_MASK_3333));
        let low = self.allocate_register()?;
        self.emit(abi::and_registers(&low, &acc, &mask));
        self.emit(abi::shift_right_immediate(&temp, &acc, 2));
        self.emit(abi::and_registers(&temp, &temp, &mask));
        self.emit(abi::add_registers(&acc, &low, &temp));

        // acc = (acc + (acc >> 4)) & 0x0F0F...
        self.emit(abi::shift_right_immediate(&temp, &acc, 4));
        self.emit(abi::add_registers(&acc, &acc, &temp));
        self.emit(abi::move_immediate(&mask, "Integer", POPCOUNT_MASK_0F0F));
        self.emit(abi::and_registers(&acc, &acc, &mask));

        // acc = (acc * 0x0101...) >> 56
        self.emit(abi::move_immediate(&mask, "Integer", POPCOUNT_MASK_0101));
        self.emit(abi::multiply_registers(&acc, &acc, &mask));
        self.emit(abi::shift_right_immediate(&acc, &acc, 56));

        Ok(ValueResult {
            type_: "Integer".to_string(),
            location: acc,
            text,
        })
    }

    /// `bswap16`/`bswap32`/`bswap64` — byte reversal. The 16/32-bit forms clear
    /// the bits above their width: `REV` on the `W` register zero-extends, and the
    /// 16-bit form additionally shifts the reversed low half into place.
    fn lower_bits_bswap(&mut self, function: &str, arg: &NirValue) -> Result<ValueResult, String> {
        let value = self.lower_value(arg)?;
        if value.type_ != "Integer" {
            return Err(format!("bits.{function} does not accept {}", value.type_));
        }
        let dst = self.allocate_register()?;
        match function {
            "bswap16" => {
                // REV of the low word puts the two low bytes at bits [31:16];
                // a logical >>16 drops the other two bytes and clears bits 16..63.
                self.emit(abi::reverse_bytes_word(&dst, &value.location));
                self.emit(abi::shift_right_immediate(&dst, &dst, 16));
            }
            "bswap32" => self.emit(abi::reverse_bytes_word(&dst, &value.location)),
            "bswap64" => self.emit(abi::reverse_bytes(&dst, &value.location)),
            other => {
                return Err(format!(
                    "native bits lowering does not support bits.{other}"
                ))
            }
        }
        Ok(ValueResult {
            type_: "Integer".to_string(),
            location: dst,
            text: format!("bits.{function}({})", value.text),
        })
    }
}
