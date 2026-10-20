//! plan-176-U: the Integer/Fixed/Float binary emitters and the float operand helpers. The
//! module doc lives in `engine/operators/builder_numeric/mod.rs`.

use super::*;

impl CodeBuilder<'_> {
    pub(crate) fn emit_integer_binary(
        &mut self,
        op: BinaryOp,
        left: &ValueResult,
        right: &ValueResult,
        dst: impl Into<abi::Reg>,
        byte_result: bool,
    ) -> Result<(), String> {
        self.emit_integer_binary_checked(op, left, right, dst, byte_result, false)
    }

    /// As [`Self::emit_integer_binary`], but when `elide_overflow` is set the
    /// `Integer` subtraction is proven (plan-39 I1) not to overflow, so the
    /// `subs`+`b.vc` check is dropped in favour of a bare `sub`. The caller
    /// establishes the proof from a guard-derived lower bound; it is only ever
    /// `true` for the non-byte `-` path.
    pub(crate) fn emit_integer_binary_checked(
        &mut self,
        op: BinaryOp,
        left: &ValueResult,
        right: &ValueResult,
        dst: impl Into<abi::Reg>,
        byte_result: bool,
        elide_overflow: bool,
    ) -> Result<(), String> {
        let dst = dst.into();
        match op {
            BinaryOp::Add => {
                if elide_overflow && !byte_result {
                    // Proven non-overflowing (`local + 1` under a strict upper
                    // bound): bare add, no flags/check.
                    self.emit(abi::add_registers(dst, &left.location, &right.location));
                } else {
                    self.emit(abi::add_registers_set_flags(
                        dst,
                        &left.location,
                        &right.location,
                    ));
                    self.emit_overflow_if_flags_set()?;
                    if byte_result {
                        self.emit_byte_upper_bound_check(dst)?;
                    }
                }
            }
            BinaryOp::Subtract => {
                if byte_result {
                    let underflow_label = self.label("byte_underflow");
                    let ok_label = self.label("byte_ok");
                    self.emit(abi::compare_registers(&left.location, &right.location));
                    self.emit(abi::branch_lo(&underflow_label));
                    self.emit(abi::subtract_registers(
                        dst,
                        &left.location,
                        &right.location,
                    ));
                    self.emit(abi::branch(&ok_label));
                    self.emit(abi::label(&underflow_label));
                    self.raise_error_bare("ErrUnderflow")?;
                    self.emit(abi::label(&ok_label));
                } else if elide_overflow {
                    // Proven non-overflowing (guard-derived lower bound): bare sub.
                    self.emit(abi::subtract_registers(
                        dst,
                        &left.location,
                        &right.location,
                    ));
                } else {
                    self.emit(abi::subtract_registers_set_flags(
                        dst,
                        &left.location,
                        &right.location,
                    ));
                    self.emit_overflow_if_flags_set()?;
                }
            }
            BinaryOp::Multiply => {
                self.emit_checked_integer_multiply(dst, &left.location, &right.location)?;
                if byte_result {
                    self.emit_byte_upper_bound_check(dst)?;
                }
            }
            BinaryOp::Divide | BinaryOp::IntDiv => {
                self.emit_nonzero_or_invalid(&right.location)?;
                self.emit_integer_division_overflow_check(&left.location, &right.location)?;
                self.emit(abi::signed_divide_registers(
                    dst,
                    &left.location,
                    &right.location,
                ));
            }
            BinaryOp::Mod => {
                self.emit_nonzero_or_invalid(&right.location)?;
                self.emit_integer_division_overflow_check(&left.location, &right.location)?;
                let quotient = self.allocate_register();
                self.emit(abi::signed_divide_registers(
                    &quotient,
                    &left.location,
                    &right.location,
                ));
                self.emit(abi::multiply_subtract_registers(
                    dst,
                    &quotient,
                    &right.location,
                    &left.location,
                ));
            }
            BinaryOp::Power => {
                self.emit_integer_pow(dst, &left.location, &right.location, byte_result)?
            }
            // The comparison and logical operators never reach an arithmetic
            // emitter: `lower_binary` routes them to their own lowering, and
            // `&` is String-only.
            BinaryOp::Or
            | BinaryOp::Xor
            | BinaryOp::And
            | BinaryOp::Equal
            | BinaryOp::NotEqual
            | BinaryOp::Less
            | BinaryOp::LessEqual
            | BinaryOp::Greater
            | BinaryOp::GreaterEqual
            | BinaryOp::Concat => {
                return Err(format!(
                    "native code plan does not lower integer operator '{}'",
                    op.name()
                ));
            }
        }
        Ok(())
    }

    pub(crate) fn emit_fixed_binary(
        &mut self,
        op: BinaryOp,
        left: &ValueResult,
        right: &ValueResult,
        dst: impl Into<abi::Reg>,
    ) -> Result<(), String> {
        let dst = dst.into();
        match op {
            BinaryOp::Add => {
                self.emit(abi::add_registers_set_flags(
                    dst,
                    &left.location,
                    &right.location,
                ));
                self.emit_overflow_if_flags_set()?;
            }
            BinaryOp::Subtract => {
                self.emit(abi::subtract_registers_set_flags(
                    dst,
                    &left.location,
                    &right.location,
                ));
                self.emit_overflow_if_flags_set()?;
            }
            BinaryOp::Multiply => self.emit_fixed_multiply(dst, &left.location, &right.location)?,
            BinaryOp::Divide => self.emit_fixed_divide(dst, &left.location, &right.location)?,
            BinaryOp::Mod => {
                self.emit_nonzero_or_invalid(&right.location)?;
                self.emit_integer_division_overflow_check(&left.location, &right.location)?;
                let quotient = self.allocate_register();
                self.emit(abi::signed_divide_registers(
                    &quotient,
                    &left.location,
                    &right.location,
                ));
                self.emit(abi::multiply_subtract_registers(
                    dst,
                    &quotient,
                    &right.location,
                    &left.location,
                ));
            }
            BinaryOp::Power => self.emit_fixed_pow(dst, &left.location, &right.location)?,
            BinaryOp::Or
            | BinaryOp::Xor
            | BinaryOp::And
            | BinaryOp::Equal
            | BinaryOp::NotEqual
            | BinaryOp::Less
            | BinaryOp::LessEqual
            | BinaryOp::Greater
            | BinaryOp::GreaterEqual
            | BinaryOp::Concat
            | BinaryOp::IntDiv => {
                return Err(format!(
                    "native code plan does not lower Fixed operator '{}'",
                    op.name()
                ));
            }
        }
        Ok(())
    }

    /// Lower a `Float` arithmetic operator and return the **location of the
    /// result** (plan-01 float-dnative). The result stays in its FP virtual
    /// register (`%fN`) and that register is returned, so no `fmov`-to-GPR
    /// shuttle is emitted. No
    /// finiteness check is emitted here: an anonymous intermediate may be
    /// non-finite and a transient that recovers to finite must not trap — the
    /// check fires only where the value crosses an observation boundary
    /// (plan-17), where `observe_float` reads the returned location.
    pub(crate) fn emit_float_binary(
        &mut self,
        op: BinaryOp,
        left: &ValueResult,
        right: &ValueResult,
        dst: impl Into<abi::Reg>,
    ) -> Result<abi::Reg, String> {
        let dst: abi::Reg = dst.into();
        match op {
            // The pure-FP arithmetic ops run on FP virtual registers so a chained
            // operand stays resident in a `d`-register instead of round-tripping
            // through a GPR (plan-03 Stage C).
            BinaryOp::Add
            | BinaryOp::Subtract
            | BinaryOp::Multiply
            | BinaryOp::Divide
            | BinaryOp::IntDiv => {
                let d_left = self.operand_as_double(left)?;
                let d_right = self.operand_as_double(right)?;
                let d_res = self.allocate_fp_register();
                match op {
                    BinaryOp::Add => self.emit(abi::float_add_d(&d_res, &d_left, &d_right)),
                    BinaryOp::Subtract => {
                        self.emit(abi::float_subtract_d(&d_res, &d_left, &d_right))
                    }
                    BinaryOp::Multiply => {
                        self.emit(abi::float_multiply_d(&d_res, &d_left, &d_right))
                    }
                    // Division by zero is no longer pre-checked: `x/0` → `±Inf`
                    // and `0/0` → `NaN` flow out as ordinary non-finite results
                    // and trap at the boundary (`ErrFloatOverflow`/`ErrFloatNaN`),
                    // never `ErrFloatDomain` (plan-17 §4.3). `DIV` on Float is
                    // the same `fdiv`: the Float escape hatch, not a truncating
                    // divide.
                    BinaryOp::Or
                    | BinaryOp::Xor
                    | BinaryOp::And
                    | BinaryOp::Equal
                    | BinaryOp::NotEqual
                    | BinaryOp::Less
                    | BinaryOp::LessEqual
                    | BinaryOp::Greater
                    | BinaryOp::GreaterEqual
                    | BinaryOp::Concat
                    | BinaryOp::Divide
                    | BinaryOp::Mod
                    | BinaryOp::IntDiv
                    | BinaryOp::Power => self.emit(abi::float_divide_d(&d_res, &d_left, &d_right)),
                }
                // `d`-native model: the FP register *is* the value's home; return
                // it so consumers stay in the FP domain and the GP shuttle is
                // never created.
                return Ok(d_res.into());
            }
            BinaryOp::Mod => {
                self.load_numeric_as_double(abi::FP_SCRATCH[0], left)?;
                self.load_numeric_as_double(abi::FP_SCRATCH[1], right)?;
                // Domain pre-check: b == 0 raises ErrFloatDomain. This stays a
                // genuine pre-check (not a boundary finiteness check) because the
                // in-tree exact `fmod` kernel below requires a non-zero, finite
                // divisor — it does not itself produce the NaN that `a MOD 0`
                // would otherwise yield (plan-17 keeps ErrFloatDomain here).
                self.emit(abi::float_compare_zero_d(abi::FP_SCRATCH[1]));
                let nonzero = self.label("float_mod_divisor_nonzero");
                self.emit(abi::branch_ne(&nonzero));
                self.raise_error_bare("ErrFloatDomain")?;
                self.emit(abi::label(&nonzero));
                // Move the f64 bit patterns into GPRs and run the in-tree exact
                // fmod kernel (no libm). d0/d1 still hold a/b from above. The
                // result of a finite, non-zero-divisor fmod is always finite, so
                // no per-op finiteness check is needed (plan-17).
                let a_bits = self.allocate_register();
                let b_bits = self.allocate_register();
                self.emit(abi::float_move_x_from_d(&a_bits, abi::FP_SCRATCH[0]));
                self.emit(abi::float_move_x_from_d(&b_bits, abi::FP_SCRATCH[1]));
                let result = self.emit_float_fmod(&a_bits, &b_bits)?;
                self.emit(abi::float_move_d_from_x(abi::FP_SCRATCH[0], &result));
                self.emit(abi::float_move_x_from_d(dst, abi::FP_SCRATCH[0]));
            }
            BinaryOp::Power => {
                self.load_numeric_as_double(abi::FP_SCRATCH[0], left)?;
                self.load_numeric_as_double(abi::FP_SCRATCH[1], right)?;
                // `emit_float_pow` keeps its domain guards (whole, non-negative
                // exponent) but no longer checks the result: an overflow to `Inf`
                // is an anonymous intermediate that traps only at the boundary
                // (plan-17).
                self.emit_float_pow(abi::FP_SCRATCH[0], abi::FP_SCRATCH[1])?;
                self.emit(abi::float_move_x_from_d(dst, abi::FP_SCRATCH[0]));
            }
            BinaryOp::Or
            | BinaryOp::Xor
            | BinaryOp::And
            | BinaryOp::Equal
            | BinaryOp::NotEqual
            | BinaryOp::Less
            | BinaryOp::LessEqual
            | BinaryOp::Greater
            | BinaryOp::GreaterEqual
            | BinaryOp::Concat => {
                return Err(format!(
                    "native code plan does not lower Float operator '{}'",
                    op.name()
                ));
            }
        }
        // `MOD`/`^` leave their result in `dst` (a GPR); they are not on a hot
        // path and stay GP-native.
        Ok(dst)
    }

    /// Whether `value` is a `Float` whose canonical home is a `d`-register: its
    /// `location` is an FP virtual register (`%fN`) rather than a GPR/slot holding
    /// the bit pattern (plan-01 float-dnative). Such a value is consumed directly
    /// in the FP domain by float-aware sites (`operand_as_double`,
    /// `load_numeric_as_double`, `fcmp`, the FP-domain finiteness check, a `str d`
    /// store) and materialized into a GPR on demand by [`Self::float_value_as_gpr`]
    /// for every site that needs the raw bits.
    pub(crate) fn float_is_dnative(value: &ValueResult) -> bool {
        value.type_ == ParameterType::Float
            && regalloc::parse_fp_vreg(&value.location.render()).is_some()
    }

    /// The single choke point every consumer that needs a `Float`'s **bit
    /// pattern in a GPR** calls (plan-01 float-dnative §4.1). For a GP-native
    /// value it is the identity (returns the existing location); for a
    /// `d`-register-native value it materializes the bits with one `fmov x, d`
    /// into a fresh GPR. Routing all bit consumers through here is what makes the
    /// carrier flip safe — a value that lives only in a `d`-register reaches a GPR
    /// consumer correctly instead of leaking its FP virtual register into a GP
    /// instruction (which would fail to encode rather than silently miscompile).
    pub(crate) fn float_value_as_gpr(&mut self, value: &ValueResult) -> Result<abi::Reg, String> {
        if Self::float_is_dnative(value) {
            let gpr = self.allocate_register();
            self.emit(abi::float_move_x_from_d(&gpr, &value.location));
            return Ok(gpr.into());
        }
        Ok(abi::Reg::from(&value.location))
    }

    /// Return `value` with its bits in a GPR (plan-01 float-dnative): a `d`-native
    /// `Float` is `fmov`'d into a fresh GP register and rewrapped as a GP-native
    /// `ValueResult`; every other value (already GP-native) is returned unchanged.
    /// Consumers that hand a value's `location` to a GP instruction — comparisons,
    /// conversions, the math kernels, call/return marshalling — call this so a
    /// `d`-native float never leaks its FP register into a GP-context encoding.
    pub(crate) fn materialize_float(&mut self, value: ValueResult) -> Result<ValueResult, String> {
        if Self::float_is_dnative(&value) {
            let gpr = self.allocate_register();
            self.emit(abi::float_move_x_from_d(&gpr, &value.location));
            return Ok(ValueResult {
                origin: None,
                type_: value.type_,
                location: Operand::from(gpr),
                text: value.text,
            });
        }
        Ok(value)
    }

    /// Store `value` into `[base + offset]`. A `d`-native `Float` is stored
    /// straight from its FP register (`str d`); every other value stores its GPR
    /// (`str x`). The 8 bytes written are identical either way, so a slot written
    /// by `str d` and later read as `ldr x` (copy/transfer/marshalling) is
    /// unchanged — only the in-flight register class differs (plan-01
    /// float-dnative §1 non-goals).
    pub(crate) fn store_value_at(
        &mut self,
        value: &ValueResult,
        base: impl Into<abi::Reg>,
        offset: usize,
    ) {
        let base: abi::Reg = base.into();
        if Self::float_is_dnative(value) {
            self.emit(abi::store_double(&value.location, base, offset));
        } else {
            self.emit(abi::store_u64(&value.location, base, offset));
        }
    }

    /// Materialize `value` into a `d`-register and return it. A `Float` operand
    /// already resident in an FP virtual register (a prior float op's result, or a
    /// `d`-native load) is returned directly with no reload; otherwise a fresh FP
    /// virtual register is loaded (plan-03 Stage C / plan-01 float-dnative).
    pub(crate) fn operand_as_double(&mut self, value: &ValueResult) -> Result<abi::Reg, String> {
        if Self::float_is_dnative(value) {
            return Ok(abi::Reg::from(&value.location));
        }
        if value.type_ == ParameterType::Float {
            if let Some(resident) = self.float_residents.get(&value.location.render()).cloned() {
                return Ok(resident);
            }
        }
        let dst = self.allocate_fp_register();
        self.load_numeric_as_double(&dst, value)?;
        Ok(dst.into())
    }
}
