//! plan-176-T: the shared arithmetic primitives — lane select, error-free products and sums,
//! Horner evaluation, the math pool base and constant broadcasts. The consts, the math const
//! pool and the kernel enums live in `builtins/vector/builder_simd_float_math/mod.rs`.

use super::*;

impl CodeBuilder<'_> {
    /// `acc[lane] = mask ? val : acc[lane]` accumulator select. `BIT acc, val,
    /// mask` inserts `val`'s bits into `acc` where `mask` is set — one instruction,
    /// leaving `mask` and `val` untouched (both are reused across atan's segments),
    /// bit-identical to the old `orr;bsl;orr` triple.
    pub(super) fn emit_vsel(
        &mut self,
        acc: impl Into<Reg>,
        mask: impl Into<Reg>,
        val: impl Into<Reg>,
    ) {
        let (acc, mask, val): (Reg, Reg, Reg) = (acc.into(), mask.into(), val.into());
        self.emit(abi::vector_bit(acc, val, mask));
    }

    /// Double-double product `a*b → (p, e)` with `p+e == a*b` to ~2x precision:
    /// `p = a*b`, `e = fma(a, b, -p)`. `p`/`e` must be distinct from `a`/`b`.
    pub(super) fn emit_twoprod(
        &mut self,
        p: impl Into<Reg>,
        e: impl Into<Reg>,
        a: impl Into<Reg>,
        b: impl Into<Reg>,
    ) {
        let (p, e, a, b): (Reg, Reg, Reg, Reg) = (p.into(), e.into(), a.into(), b.into());
        self.emit(abi::vector_fmul(p, a, b));
        self.emit(abi::vector_fneg(e, p)); // e = -p
        self.emit(abi::vector_fmla(e, a, b)); // e = -p + a*b = fma(a,b,-p)
    }

    /// Knuth two-sum `a+b → (s, e)` with `s+e == a+b` exactly. `t1`/`t2` are
    /// caller-supplied scratch (must be caller-saved vector regs distinct from the
    /// operands/results — `v8`-`v15` are callee-saved and off-limits here).
    pub(super) fn emit_twosum(
        &mut self,
        s: impl Into<Reg>,
        e: impl Into<Reg>,
        a: impl Into<Reg>,
        b: impl Into<Reg>,
        t1: impl Into<Reg>,
        t2: impl Into<Reg>,
    ) {
        let (s, e, a, b, t1, t2): (Reg, Reg, Reg, Reg, Reg, Reg) =
            (s.into(), e.into(), a.into(), b.into(), t1.into(), t2.into());
        self.emit(abi::vector_fadd(s, a, b));
        self.emit(abi::vector_fsub(t1, s, a)); // t
        self.emit(abi::vector_fsub(t2, s, t1)); // s - t  (~a)
        self.emit(abi::vector_fsub(t2, a, t2)); // a - (s-t)
        self.emit(abi::vector_fsub(t1, b, t1)); // b - t
        self.emit(abi::vector_fadd(e, t2, t1));
    }

    /// Compensated (double-double) Horner of `coeffs` in `var`, leaving the result
    /// as `(hi, lo)`. Each step keeps the running accumulator to ~2x precision.
    /// Uses `VEC_SCRATCH[6]` (coeff broadcast), `VEC_SCRATCH[7]`/v28/v29/v30 and
    /// `VEC_SCRATCH[0]`/v31 (via two-sum) as scratch — distinct from
    /// `hi`/`lo`/`var`. (bug-175 H: the callee-saved v8/v9 named here previously
    /// were never touched; two-sum uses v0/v31, per the note at the twosum call.)
    pub(super) fn emit_compensated_horner(
        &mut self,
        hi: impl Into<Reg>,
        lo: impl Into<Reg>,
        var: impl Into<Reg>,
        coeffs: &[f64],
        k: &KernelRegs,
    ) {
        let (hi, lo, var): (Reg, Reg, Reg) = (hi.into(), lo.into(), var.into());
        self.broadcast_f64(hi, coeffs[coeffs.len() - 1]);
        self.emit(abi::vector_eor(lo, lo, lo));
        for i in (0..coeffs.len() - 1).rev() {
            // (ph, pe) = twoprod(hi, var); pe += lo*var.
            self.emit_twoprod(abi::VEC_SCRATCH[7], &k.v28, hi, var);
            self.emit(abi::vector_fmla(&k.v28, lo, var));
            // (sh, se) = twosum(c, ph). Scratch v0/v31 are free during the Horner.
            self.broadcast_f64(abi::VEC_SCRATCH[6], coeffs[i]);
            self.emit_twosum(
                &k.v29,
                &k.v30,
                abi::VEC_SCRATCH[6],
                abi::VEC_SCRATCH[7],
                abi::VEC_SCRATCH[0],
                &k.v31,
            );
            // hi = sh; lo = se + pe.
            self.emit(abi::vector_orr(hi, &k.v29, &k.v29));
            self.emit(abi::vector_fadd(lo, &k.v30, &k.v28));
        }
    }

    /// Horner evaluation of `coeffs` in the variable held by `var`, leaving the
    /// result in `acc`. `acc = c[n-1]; acc = c[i] + acc*var` for `i = n-2..=0`.
    /// Uses `v4` as the per-step coefficient broadcast (callers must not hold a
    /// live value there).
    pub(super) fn emit_horner(&mut self, acc: impl Into<Reg>, var: impl Into<Reg>, coeffs: &[f64]) {
        let (acc, var): (Reg, Reg) = (acc.into(), var.into());
        self.broadcast_f64(acc, coeffs[coeffs.len() - 1]);
        for i in (0..coeffs.len() - 1).rev() {
            self.broadcast_f64(abi::VEC_SCRATCH[4], coeffs[i]);
            self.emit(abi::vector_fmla(abi::VEC_SCRATCH[4], acc, var));
            self.emit(abi::vector_orr(
                acc,
                abi::VEC_SCRATCH[4],
                abi::VEC_SCRATCH[4],
            ));
        }
    }

    /// Load the constant pool base address into the kernel's pool-base register
    /// before any broadcast). adrp+add to the read-only pool data symbol.
    /// The register the SIMD math kernels address the constant pool through. If
    /// the backend pins a physical (AArch64 `x2`), return it; otherwise (x86) a
    /// per-kernel virtual register the allocator colors, minted once per function
    /// and reused for every coefficient load and the re-materialized base.
    /// GPR pinned to the pool base for a kernel's lifetime. The register is
    /// per-backend (`mir::Backend::math_pool_base`): AArch64 uses `x2` (caller-saved
    /// scratch the allocator never assigns, `INT_ALLOCATABLE` is `x8`+, so it holds
    /// the base across the whole body; the array paths load it *after* their one
    /// `bl`, and the bodies make no further call). x86 uses `r11` instead, because
    /// `x2` is a SysV ABI-role register there and would be remapped inconsistently
    /// across the quadrant branch. See `math_pool_base` for the full rationale.
    pub(super) fn math_pool_base_reg(&mut self) -> Reg {
        if let Some(pinned) = mir::active_backend().register_model().math_pool_base() {
            return pinned;
        }
        if let Some((symbol, reg)) = &self.math_pool_base_vreg {
            if *symbol == self.current_symbol {
                return Reg::from(reg);
            }
        }
        // `allocate_register` is infallible: it never overflows a pool — the
        // liveness-driven coloring spills under pressure.
        let reg = self.temporary_vreg();
        self.math_pool_base_vreg = Some((self.current_symbol.clone(), reg));
        Reg::from(reg)
    }

    pub(super) fn emit_load_math_pool_base(&mut self) {
        let pool_base = self.math_pool_base_reg();
        self.emit(abi::load_page_address(&pool_base, MATH_CONST_POOL_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: MATH_CONST_POOL_SYMBOL.to_string(),
            kind: RelocIntent::DataAddrHi,
            binding: "data".to_string(),
            library: None,
        });
        self.emit(abi::add_page_offset(
            &pool_base,
            &pool_base,
            MATH_CONST_POOL_SYMBOL,
        ));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: MATH_CONST_POOL_SYMBOL.to_string(),
            kind: RelocIntent::DataAddrLo,
            binding: "data".to_string(),
            library: None,
        });
    }

    /// Broadcast an `f64` constant's bit pattern into both `.2d` lanes of `vreg`.
    /// Pooled constants load with one `ldr q` from the pool base; anything else
    /// falls back to building the immediate inline (so the pool's coverage never
    /// affects correctness).
    pub(super) fn broadcast_f64(&mut self, vreg: impl Into<Reg>, value: f64) {
        let vreg: Reg = vreg.into();
        if let Some(offset) = math_const_pool_offset(value.to_bits()) {
            let pool_base = self.math_pool_base_reg();
            self.emit(abi::vector_load(vreg, &pool_base, offset));
        } else {
            let tmp = self.temporary_vreg();
            self.emit(abi::move_immediate(
                &tmp,
                "Integer",
                &value.to_bits().to_string(),
            ));
            self.emit(abi::vector_dup_from_x(vreg, &tmp));
        }
    }

    /// Broadcast a signed `i64` constant into both `.2d` lanes of `vreg`.
    pub(super) fn broadcast_i64(&mut self, vreg: impl Into<Reg>, value: i64) {
        let vreg: Reg = vreg.into();
        if let Some(offset) = math_const_pool_offset(value as u64) {
            let pool_base = self.math_pool_base_reg();
            self.emit(abi::vector_load(vreg, &pool_base, offset));
        } else {
            let tmp = self.temporary_vreg();
            self.emit(abi::move_immediate(
                &tmp,
                "Integer",
                &(value as u64).to_string(),
            ));
            self.emit(abi::vector_dup_from_x(vreg, &tmp));
        }
    }
}
