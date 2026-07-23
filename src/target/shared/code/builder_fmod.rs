//! Standalone in-tree port of musl's 64-bit `fmod` (bug-332 F1). Split out of
//! `builder_numeric.rs` to mirror `builder_pow.rs` (the fdlibm `pow` port) — the
//! other large standalone libm kernel that earns its own file. No platform math
//! library is linked; see `docs/spec/architecture/18_math-kernels.md`.

use super::*;

impl CodeBuilder<'_> {
    /// `fmod(a, b) = a - n*b`, `n = trunc(a/b)`, computed **exactly** over GPRs
    /// (the IEEE bitwise remainder; musl's 64-bit `fmod`). Returns a register
    /// holding the result's f64 bit pattern. The divisor is guaranteed finite and
    /// non-zero — `Float MOD Float` raises `ErrFloatDomain` for `b == 0` before
    /// calling — and MFBASIC has no inf/NaN Float values, so libm's exception
    /// prologue is omitted. The result is exactly representable (bit-identical to
    /// libm `fmod`), so no accuracy tolerance applies.
    pub(super) fn emit_float_fmod(&mut self, a_loc: &str, b_loc: &str) -> Result<String, String> {
        // Spill the operands and reset the register file: the kernel needs ~12
        // live registers and would otherwise exhaust the file in a busy
        // expression. The caller consumes `result` immediately after the return.
        let a_slot = self.allocate_stack_object("fmod_a", 8);
        let b_slot = self.allocate_stack_object("fmod_b", 8);
        self.emit(abi::store_u64(a_loc, abi::stack_pointer(), a_slot));
        self.emit(abi::store_u64(b_loc, abi::stack_pointer(), b_slot));
        self.reset_temporary_registers();
        let ux = self.allocate_register()?;
        let uy = self.allocate_register()?;
        self.emit(abi::load_u64(&ux, abi::stack_pointer(), a_slot));
        self.emit(abi::load_u64(&uy, abi::stack_pointer(), b_slot));
        let result = self.allocate_register()?;
        // Persistent constants.
        let signmask = self.allocate_register()?;
        let expmask = self.allocate_register()?;
        let mantmask = self.allocate_register()?;
        let implicit = self.allocate_register()?;
        self.emit(abi::move_immediate(&signmask, "Integer", F64_SIGN_BIT)); // 1<<63
        self.emit(abi::move_immediate(&expmask, "Integer", "2047")); // 0x7ff
        self.emit(abi::move_immediate(&mantmask, "Integer", F64_MANTISSA_MASK)); // (1<<52)-1
        self.emit(abi::move_immediate(
            &implicit,
            "Integer",
            "4503599627370496",
        )); // 1<<52
            // sign = ux & SIGN; ex = (ux>>52)&0x7ff; ey = (uy>>52)&0x7ff; uxi = ux.
        let sign = self.allocate_register()?;
        let ex = self.allocate_register()?;
        let ey = self.allocate_register()?;
        let uxi = self.allocate_register()?;
        let i = self.allocate_register()?;
        let shift = self.allocate_register()?;
        self.emit(abi::and_registers(&sign, &ux, &signmask));
        self.emit(abi::shift_right_immediate(&ex, &ux, 52));
        self.emit(abi::and_registers(&ex, &ex, &expmask));
        self.emit(abi::shift_right_immediate(&ey, &uy, 52));
        self.emit(abi::and_registers(&ey, &ey, &expmask));
        self.emit(abi::move_register(&uxi, &ux));

        let end = self.label("fmod_end");
        let return_x = self.label("fmod_return_x");
        let ret_zero = self.label("fmod_ret_zero");

        // |x| <= |y|: compare magnitudes via the sign-stripped (<<1) bit patterns.
        let not_le = self.label("fmod_not_le");
        self.emit(abi::shift_left_immediate(&i, &ux, 1)); // ax2 (reuse i)
        self.emit(abi::shift_left_immediate(&shift, &uy, 1)); // bx2 (reuse shift)
        self.emit(abi::compare_registers(&i, &shift));
        self.emit(abi::branch_hi(&not_le)); // |x| > |y| (unsigned) → reduce
        self.emit(abi::compare_registers(&i, &shift));
        self.emit(abi::branch_ne(&return_x)); // |x| < |y| → result is x
        self.emit(abi::move_register(&result, &sign)); // |x| == |y| → ±0
        self.emit(abi::branch(&end));
        self.emit(abi::label(&return_x));
        self.emit(abi::move_register(&result, &ux));
        self.emit(abi::branch(&end));
        self.emit(abi::label(&not_le));

        // Normalize x: implicit-bit mantissa for normals; shift subnormals up.
        let x_normal = self.label("fmod_x_normal");
        let x_done = self.label("fmod_x_done");
        let x_subloop = self.label("fmod_x_subloop");
        let x_subdone = self.label("fmod_x_subdone");
        self.emit(abi::compare_immediate(&ex, "0"));
        self.emit(abi::branch_ne(&x_normal));
        self.emit(abi::shift_left_immediate(&i, &uxi, 12));
        self.emit(abi::label(&x_subloop));
        self.emit(abi::compare_immediate(&i, "0"));
        self.emit(abi::branch_lt(&x_subdone)); // top bit set → normalized
        self.emit(abi::subtract_immediate(&ex, &ex, 1));
        self.emit(abi::add_registers(&i, &i, &i));
        self.emit(abi::branch(&x_subloop));
        self.emit(abi::label(&x_subdone));
        self.emit(abi::move_immediate(&shift, "Integer", "1"));
        self.emit(abi::subtract_registers(&shift, &shift, &ex)); // 1 - ex
        self.emit(abi::shift_left_variable(&uxi, &uxi, &shift));
        self.emit(abi::branch(&x_done));
        self.emit(abi::label(&x_normal));
        self.emit(abi::and_registers(&uxi, &uxi, &mantmask));
        self.emit(abi::or_registers(&uxi, &uxi, &implicit));
        self.emit(abi::label(&x_done));

        // Normalize y in place (uy becomes its 53-bit mantissa).
        let y_normal = self.label("fmod_y_normal");
        let y_done = self.label("fmod_y_done");
        let y_subloop = self.label("fmod_y_subloop");
        let y_subdone = self.label("fmod_y_subdone");
        self.emit(abi::compare_immediate(&ey, "0"));
        self.emit(abi::branch_ne(&y_normal));
        self.emit(abi::shift_left_immediate(&i, &uy, 12));
        self.emit(abi::label(&y_subloop));
        self.emit(abi::compare_immediate(&i, "0"));
        self.emit(abi::branch_lt(&y_subdone));
        self.emit(abi::subtract_immediate(&ey, &ey, 1));
        self.emit(abi::add_registers(&i, &i, &i));
        self.emit(abi::branch(&y_subloop));
        self.emit(abi::label(&y_subdone));
        self.emit(abi::move_immediate(&shift, "Integer", "1"));
        self.emit(abi::subtract_registers(&shift, &shift, &ey)); // 1 - ey
        self.emit(abi::shift_left_variable(&uy, &uy, &shift));
        self.emit(abi::branch(&y_done));
        self.emit(abi::label(&y_normal));
        self.emit(abi::and_registers(&uy, &uy, &mantmask));
        self.emit(abi::or_registers(&uy, &uy, &implicit));
        self.emit(abi::label(&y_done));

        // Fixed-point remainder: for (; ex>ey; ex--) { i=uxi-uy; if i>=0 { if i==0
        // → ±0; uxi=i } uxi<<=1 }.
        let modloop = self.label("fmod_modloop");
        let modloop_end = self.label("fmod_modloop_end");
        let mod_shift = self.label("fmod_mod_shift");
        self.emit(abi::label(&modloop));
        self.emit(abi::compare_registers(&ex, &ey));
        self.emit(abi::branch_le(&modloop_end));
        self.emit(abi::subtract_registers(&i, &uxi, &uy));
        self.emit(abi::compare_immediate(&i, "0"));
        self.emit(abi::branch_lt(&mod_shift)); // i<0 → keep uxi
        self.emit(abi::branch_eq(&ret_zero)); // i==0 → exact, ±0
        self.emit(abi::move_register(&uxi, &i));
        self.emit(abi::label(&mod_shift));
        self.emit(abi::add_registers(&uxi, &uxi, &uxi));
        self.emit(abi::subtract_immediate(&ex, &ex, 1));
        self.emit(abi::branch(&modloop));
        self.emit(abi::label(&modloop_end));

        // Final step: i=uxi-uy; if i>=0 { if i==0 → ±0; uxi=i }.
        let after_final = self.label("fmod_after_final");
        self.emit(abi::subtract_registers(&i, &uxi, &uy));
        self.emit(abi::compare_immediate(&i, "0"));
        self.emit(abi::branch_lt(&after_final));
        self.emit(abi::branch_eq(&ret_zero));
        self.emit(abi::move_register(&uxi, &i));
        self.emit(abi::label(&after_final));

        // Re-normalize the result mantissa: for (; uxi>>52==0; uxi<<=1, ex--).
        let normloop = self.label("fmod_normloop");
        let normloop_end = self.label("fmod_normloop_end");
        self.emit(abi::label(&normloop));
        self.emit(abi::shift_right_immediate(&i, &uxi, 52));
        self.emit(abi::compare_immediate(&i, "0"));
        self.emit(abi::branch_ne(&normloop_end));
        self.emit(abi::add_registers(&uxi, &uxi, &uxi));
        self.emit(abi::subtract_immediate(&ex, &ex, 1));
        self.emit(abi::branch(&normloop));
        self.emit(abi::label(&normloop_end));

        // Scale back: ex>0 → reattach exponent; ex<=0 → shift into a subnormal.
        let scale_sub = self.label("fmod_scale_sub");
        let scale_done = self.label("fmod_scale_done");
        self.emit(abi::compare_immediate(&ex, "0"));
        self.emit(abi::branch_le(&scale_sub));
        self.emit(abi::subtract_registers(&uxi, &uxi, &implicit)); // drop implicit bit
        self.emit(abi::shift_left_immediate(&shift, &ex, 52)); // ex<<52
        self.emit(abi::or_registers(&uxi, &uxi, &shift));
        self.emit(abi::branch(&scale_done));
        self.emit(abi::label(&scale_sub));
        self.emit(abi::move_immediate(&shift, "Integer", "1"));
        self.emit(abi::subtract_registers(&shift, &shift, &ex)); // 1 - ex
        self.emit(abi::shift_right_variable(&uxi, &uxi, &shift));
        self.emit(abi::label(&scale_done));
        self.emit(abi::or_registers(&uxi, &uxi, &sign)); // restore sign
        self.emit(abi::move_register(&result, &uxi));
        self.emit(abi::branch(&end));

        self.emit(abi::label(&ret_zero));
        self.emit(abi::move_register(&result, &sign)); // ±0
        self.emit(abi::label(&end));
        // Spill the result and reset, so the surrounding expression resumes with a
        // fresh (low-pressure) register file.
        let out_slot = self.allocate_stack_object("fmod_out", 8);
        self.emit(abi::store_u64(&result, abi::stack_pointer(), out_slot));
        self.reset_temporary_registers();
        let out = self.allocate_register()?;
        self.emit(abi::load_u64(&out, abi::stack_pointer(), out_slot));
        Ok(out)
    }
}
