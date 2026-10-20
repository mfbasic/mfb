//! plan-176-O: the aarch64 encoder's the integer ALU, shift, multiply/divide, immediate,
//! stack-pointer, compare and scalar floating-point emitters. `Encoder`, the `emit_instruction`
//! dispatch and `patch_labels` live in `encode/emitter/mod.rs`.

use super::*;

impl Encoder {
    pub(super) fn emit_word(&mut self, word: u32) -> Result<(), String> {
        self.text.extend_from_slice(&word.to_le_bytes());
        Ok(())
    }

    pub(super) fn emit_mov(&mut self, dst: &str, src: &str) -> Result<(), String> {
        let rd = reg(dst.to_string())?;
        let rn = reg(src.to_string())?;
        // bug-178 B: `mov` is encoded as `ORR rd, xzr, rn`, where register 31 is
        // XZR. Register 31 is *also* SP, so a mov to/from the stack pointer would
        // read/write zero instead of SP. When either operand is the stack pointer,
        // encode `ADD rd, rn, #0` — register 31 there is SP. A mov to/from `xzr`
        // keeps the ORR form (reads 0 / discards), which is correct.
        let is_sp = |name: &str| matches!(name, "sp" | "raw_sp");
        if is_sp(dst) || is_sp(src) {
            return self.emit_word(0x9100_0000 | ((rn as u32) << 5) | rd as u32);
        }
        self.emit_word(0xaa00_03e0 | ((rn as u32) << 16) | rd as u32)
    }

    pub(super) fn emit_mov_imm(&mut self, rd: u8, value: u64) -> Result<(), String> {
        // Index 0 always emits a `movz` for the low halfword (including the
        // `value == 0` case, which emits `movz rd, #0`), so the base is always
        // laid down; higher halfwords add `movk` only when nonzero.
        for (index, shift) in [0, 16, 32, 48].into_iter().enumerate() {
            let part = ((value >> shift) & 0xffff) as u32;
            if index == 0 {
                self.emit_word(0xd280_0000 | (part << 5) | rd as u32)?;
            } else if part != 0 {
                self.emit_word(
                    0xf280_0000 | (((shift / 16) as u32) << 21) | (part << 5) | rd as u32,
                )?;
            }
        }
        Ok(())
    }

    /// Emit a three-register data-processing instruction: `opcode | Rm<<16 |
    /// Rn<<5 | Rd`. The register-field placement is identical across the ~21
    /// AArch64 arithmetic/logical/shift/FP-2-source ops that differ only in the
    /// opcode base, so they all funnel through here rather than restating the bit
    /// packing (bug-341-B6). GPR and FP register numbers share the same 5-bit
    /// fields, so the same helper serves both.
    fn emit_rrr(&mut self, opcode: u32, rd: u8, rn: u8, rm: u8) -> Result<(), String> {
        self.emit_word(opcode | ((rm as u32) << 16) | ((rn as u32) << 5) | rd as u32)
    }

    /// Emit a two-register instruction: `opcode | Rn<<5 | Rd` (register-to-register
    /// moves, FP 1-source ops, and the GPR<->FP conversion group). Companion to
    /// [`Self::emit_rrr`] for the ops with no `Rm` field (bug-341-B6).
    pub(super) fn emit_rr(&mut self, opcode: u32, rd: u8, rn: u8) -> Result<(), String> {
        self.emit_word(opcode | ((rn as u32) << 5) | rd as u32)
    }

    pub(super) fn emit_add(&mut self, rd: u8, rn: u8, rm: u8) -> Result<(), String> {
        self.emit_rrr(0x8b00_0000, rd, rn, rm)
    }

    pub(super) fn emit_adds(&mut self, rd: u8, rn: u8, rm: u8) -> Result<(), String> {
        self.emit_rrr(0xab00_0000, rd, rn, rm)
    }

    pub(super) fn emit_sub(&mut self, rd: u8, rn: u8, rm: u8) -> Result<(), String> {
        self.emit_rrr(0xcb00_0000, rd, rn, rm)
    }

    pub(super) fn emit_subs(&mut self, rd: u8, rn: u8, rm: u8) -> Result<(), String> {
        self.emit_rrr(0xeb00_0000, rd, rn, rm)
    }

    pub(super) fn emit_and(&mut self, rd: u8, rn: u8, rm: u8) -> Result<(), String> {
        self.emit_rrr(0x8a00_0000, rd, rn, rm)
    }

    pub(super) fn emit_orr(&mut self, rd: u8, rn: u8, rm: u8) -> Result<(), String> {
        self.emit_rrr(0xaa00_0000, rd, rn, rm)
    }

    pub(super) fn emit_eor(&mut self, rd: u8, rn: u8, rm: u8) -> Result<(), String> {
        self.emit_rrr(0xca00_0000, rd, rn, rm)
    }

    pub(super) fn emit_mvn(&mut self, rd: u8, rm: u8) -> Result<(), String> {
        self.emit_word(0xaa20_03e0 | ((rm as u32) << 16) | rd as u32)
    }

    pub(super) fn emit_mul(&mut self, rd: u8, rn: u8, rm: u8) -> Result<(), String> {
        self.emit_rrr(0x9b00_7c00, rd, rn, rm)
    }

    pub(super) fn emit_smulh(&mut self, rd: u8, rn: u8, rm: u8) -> Result<(), String> {
        self.emit_rrr(0x9b40_7c00, rd, rn, rm)
    }

    pub(super) fn emit_umulh(&mut self, rd: u8, rn: u8, rm: u8) -> Result<(), String> {
        self.emit_rrr(0x9bc0_7c00, rd, rn, rm)
    }

    /// Explicit-carry add (plan-00-G §4): `dst = lhs + rhs + carry_in`,
    /// `carry_out` = unsigned carry as a value. Expanded atomically so the carry
    /// flag never escapes the op (it cannot be clobbered by register allocation):
    ///   no carry-in (`carry_in == xzr`): `adds dst, lhs, rhs`  (2 instructions)
    ///   carry-in register:  `cmp carry_in, #1; adcs dst, lhs, rhs`  (3)
    ///   then `cset carry_out, cs` — materialize the carry-out (0/1).
    /// The no-carry-in case must NOT use `cmp xzr, #1`: in the ADD/SUB *immediate*
    /// form register 31 in the `Rn` field is **SP**, not XZR, so `subs xzr, xzr,
    /// #1` would compute `SP - 1` (carry set) and wrongly add 1. The register-form
    /// `adds dst, lhs, rhs` reads its carry-in as 0 with no SP hazard.
    pub(super) fn emit_add_carry(
        &mut self,
        dst: u8,
        carry_out: u8,
        lhs: u8,
        rhs: u8,
        carry_in: u8,
    ) -> Result<(), String> {
        if carry_in == 31 {
            // No carry-in: adds dst, lhs, rhs (register form; sets C = carry-out).
            self.emit_word(0xab00_0000 | ((rhs as u32) << 16) | ((lhs as u32) << 5) | dst as u32)?;
        } else {
            // cmp carry_in, #1 (carry_in is a real register, never x31/SP):
            // C = carry_in for {0,1}.
            self.emit_word(0xf100_0400 | ((carry_in as u32) << 5) | 31)?;
            // adcs dst, lhs, rhs: dst = lhs + rhs + C, C = carry-out.
            self.emit_word(0xba00_0000 | ((rhs as u32) << 16) | ((lhs as u32) << 5) | dst as u32)?;
        }
        // cset carry_out, cs  (csinc carry_out, xzr, xzr, cc): carry_out = C.
        self.emit_word(0x9a80_0400 | (31 << 16) | (0x3 << 12) | (31 << 5) | carry_out as u32)
    }

    /// Explicit-borrow subtract (plan-00-G §4): `dst = lhs - rhs - borrow_in`,
    /// `borrow_out` = borrow as a value. Mirror of [`Self::emit_add_carry`]:
    ///   `subs xzr, xzr, borrow_in` — `C = !borrow_in` (`sbc` subtracts `1 - C`)
    ///   `sbcs dst, lhs, rhs`       — `dst = lhs - rhs - (1 - C) = … - borrow_in`
    ///   `cset borrow_out, cc`      — `C` clear after a subtract means a borrow
    pub(super) fn emit_sub_borrow(
        &mut self,
        dst: u8,
        borrow_out: u8,
        lhs: u8,
        rhs: u8,
        borrow_in: u8,
    ) -> Result<(), String> {
        // subs xzr, xzr, borrow_in: C = !borrow_in for {0,1}.
        self.emit_word(0xeb00_0000 | ((borrow_in as u32) << 16) | (31 << 5) | 31)?;
        // sbcs dst, lhs, rhs: dst = lhs - rhs - (1 - C), C = !borrow-out.
        self.emit_word(0xfa00_0000 | ((rhs as u32) << 16) | ((lhs as u32) << 5) | dst as u32)?;
        // cset borrow_out, cc  (csinc borrow_out, xzr, xzr, cs): borrow_out = !C.
        self.emit_word(0x9a80_0400 | (31 << 16) | (0x2 << 12) | (31 << 5) | borrow_out as u32)
    }

    pub(super) fn emit_rorv(&mut self, rd: u8, rn: u8, rm: u8) -> Result<(), String> {
        self.emit_rrr(0x9ac0_2c00, rd, rn, rm)
    }

    /// 32-bit `RORV Wd, Wn, Wm` — rotate right by `Wm mod 32`; zero-extends Wd.
    pub(super) fn emit_rorv_w(&mut self, rd: u8, rn: u8, rm: u8) -> Result<(), String> {
        self.emit_rrr(0x1ac0_2c00, rd, rn, rm)
    }

    /// 64-bit `LSLV Xd, Xn, Xm` — logical shift left by `Xm mod 64`.
    pub(super) fn emit_lslv(&mut self, rd: u8, rn: u8, rm: u8) -> Result<(), String> {
        self.emit_rrr(0x9ac0_2000, rd, rn, rm)
    }

    /// 64-bit `LSRV Xd, Xn, Xm` — logical shift right by `Xm mod 64`.
    pub(super) fn emit_lsrv(&mut self, rd: u8, rn: u8, rm: u8) -> Result<(), String> {
        self.emit_rrr(0x9ac0_2400, rd, rn, rm)
    }

    /// 64-bit `ASRV Xd, Xn, Xm` — arithmetic shift right by `Xm mod 64`.
    pub(super) fn emit_asrv(&mut self, rd: u8, rn: u8, rm: u8) -> Result<(), String> {
        self.emit_rrr(0x9ac0_2800, rd, rn, rm)
    }

    /// 64-bit `CLZ Xd, Xn` — count leading zeros.
    pub(super) fn emit_clz(&mut self, rd: u8, rn: u8) -> Result<(), String> {
        self.emit_word(0xdac0_1000 | ((rn as u32) << 5) | rd as u32)
    }

    /// `SXTW Xd, Wn` — alias of `SBFM Xd, Xn, #0, #31` (sf=1, N=1, immr=0,
    /// imms=31). Sign-extends the low 32 bits of `rn` into the 64-bit `rd`.
    pub(super) fn emit_sxtw(&mut self, rd: u8, rn: u8) -> Result<(), String> {
        self.emit_word(0x9340_7c00 | ((rn as u32) << 5) | rd as u32)
    }

    /// 64-bit `RBIT Xd, Xn` — reverse bit order.
    pub(super) fn emit_rbit(&mut self, rd: u8, rn: u8) -> Result<(), String> {
        self.emit_word(0xdac0_0000 | ((rn as u32) << 5) | rd as u32)
    }

    /// 32-bit `REV Wd, Wn` — reverse the four bytes of Wn; zero-extends Wd.
    pub(super) fn emit_rev_w(&mut self, rd: u8, rn: u8) -> Result<(), String> {
        self.emit_word(0x5ac0_0800 | ((rn as u32) << 5) | rd as u32)
    }

    /// 64-bit `REV Xd, Xn` — reverse all eight bytes of Xn.
    pub(super) fn emit_rev_x(&mut self, rd: u8, rn: u8) -> Result<(), String> {
        self.emit_word(0xdac0_0c00 | ((rn as u32) << 5) | rd as u32)
    }

    pub(super) fn emit_sdiv(&mut self, rd: u8, rn: u8, rm: u8) -> Result<(), String> {
        self.emit_word(0x9ac0_0c00 | ((rm as u32) << 16) | ((rn as u32) << 5) | rd as u32)
    }

    pub(super) fn emit_udiv(&mut self, rd: u8, rn: u8, rm: u8) -> Result<(), String> {
        self.emit_word(0x9ac0_0800 | ((rm as u32) << 16) | ((rn as u32) << 5) | rd as u32)
    }

    pub(super) fn emit_msub(&mut self, rd: u8, rn: u8, rm: u8, ra: u8) -> Result<(), String> {
        self.emit_word(
            0x9b00_8000
                | ((rm as u32) << 16)
                | ((ra as u32) << 10)
                | ((rn as u32) << 5)
                | rd as u32,
        )
    }

    pub(super) fn emit_lsl_imm(&mut self, rd: u8, rn: u8, shift: u8) -> Result<(), String> {
        if shift >= 64 {
            return Err(format!("AArch64 lsl shift {shift} is out of range"));
        }
        let immr = (64 - shift as u32) & 63;
        let imms = 63 - shift as u32;
        self.emit_word(0xd340_0000 | (immr << 16) | (imms << 10) | ((rn as u32) << 5) | rd as u32)
    }

    pub(super) fn emit_lsr_imm(&mut self, rd: u8, rn: u8, shift: u8) -> Result<(), String> {
        if shift >= 64 {
            return Err(format!("AArch64 lsr shift {shift} is out of range"));
        }
        self.emit_word(
            0xd340_0000 | ((shift as u32) << 16) | (63 << 10) | ((rn as u32) << 5) | rd as u32,
        )
    }

    pub(super) fn emit_asr_imm(&mut self, rd: u8, rn: u8, shift: u8) -> Result<(), String> {
        if shift >= 64 {
            return Err(format!("AArch64 asr shift {shift} is out of range"));
        }
        self.emit_word(
            0x9340_0000 | ((shift as u32) << 16) | (63 << 10) | ((rn as u32) << 5) | rd as u32,
        )
    }

    pub(super) fn emit_add_imm(&mut self, rd: u8, rn: u8, imm: u64) -> Result<(), String> {
        if let Some((imm12, shift12)) = encode_add_sub_imm(imm) {
            return self.emit_add_imm_chunk(rd, rn, imm12, shift12);
        }
        // bug-284 C2: refuse an immediate that would decompose into an absurd
        // number of chunks instead of grinding through ~10^12 iterations. Nothing
        // legitimate comes close; an immediate this wide means a lowering produced
        // a wrapped-negative offset, and a loud error beats a hung compiler.
        if add_sub_chunk_count(imm) > MAX_ADD_SUB_CHUNKS {
            return Err(format!(
                "AArch64 add immediate {imm} is too wide to decompose (more than \
                 {MAX_ADD_SUB_CHUNKS} chunks); this usually means a wrapped-negative offset"
            ));
        }
        let mut remaining = imm;
        let mut src = rn;
        while remaining > 0 {
            let (chunk, shift12) = next_add_sub_chunk(remaining);
            self.emit_add_imm_chunk(rd, src, chunk, shift12)?;
            remaining -= if shift12 {
                u64::from(chunk) << 12
            } else {
                u64::from(chunk)
            };
            src = rd;
        }
        Ok(())
    }

    fn emit_add_imm_chunk(
        &mut self,
        rd: u8,
        rn: u8,
        imm12: u32,
        shift12: bool,
    ) -> Result<(), String> {
        self.emit_word(
            0x9100_0000
                | ((u32::from(shift12)) << 22)
                | (imm12 << 10)
                | ((rn as u32) << 5)
                | rd as u32,
        )
    }

    pub(super) fn emit_sub_imm(&mut self, rd: u8, rn: u8, imm: u64) -> Result<(), String> {
        if let Some((imm12, shift12)) = encode_add_sub_imm(imm) {
            return self.emit_sub_imm_chunk(rd, rn, imm12, shift12);
        }
        // bug-284 C2: refuse an immediate that would decompose into an absurd
        // number of chunks instead of grinding through ~10^12 iterations. Nothing
        // legitimate comes close; an immediate this wide means a lowering produced
        // a wrapped-negative offset, and a loud error beats a hung compiler.
        if add_sub_chunk_count(imm) > MAX_ADD_SUB_CHUNKS {
            return Err(format!(
                "AArch64 sub immediate {imm} is too wide to decompose (more than \
                 {MAX_ADD_SUB_CHUNKS} chunks); this usually means a wrapped-negative offset"
            ));
        }
        let mut remaining = imm;
        let mut src = rn;
        while remaining > 0 {
            let (chunk, shift12) = next_add_sub_chunk(remaining);
            self.emit_sub_imm_chunk(rd, src, chunk, shift12)?;
            remaining -= if shift12 {
                u64::from(chunk) << 12
            } else {
                u64::from(chunk)
            };
            src = rd;
        }
        Ok(())
    }

    fn emit_sub_imm_chunk(
        &mut self,
        rd: u8,
        rn: u8,
        imm12: u32,
        shift12: bool,
    ) -> Result<(), String> {
        self.emit_word(
            0xd100_0000
                | ((u32::from(shift12)) << 22)
                | (imm12 << 10)
                | ((rn as u32) << 5)
                | rd as u32,
        )
    }

    pub(super) fn emit_sub_sp(&mut self, imm: u64) -> Result<(), String> {
        self.emit_sub_imm(31, 31, imm)
    }

    pub(super) fn emit_add_sp(&mut self, imm: u64) -> Result<(), String> {
        self.emit_add_imm(31, 31, imm)
    }

    /// `rn_is_sp` reports whether `rn` was spelled as the stack pointer, which
    /// changes what register 31 means depending on which form is chosen.
    ///
    /// bug-284 C1: the immediate form below reads register 31 as SP and is correct
    /// for an `sp` operand, but the register-form fallback taken for a wide
    /// immediate reads 31 as XZR -- so this one instruction silently changed
    /// meaning with the magnitude of its immediate. Rather than compare against
    /// zero, refuse the combination.
    pub(super) fn emit_cmp_imm(&mut self, rn: u8, imm: u64, rn_is_sp: bool) -> Result<(), String> {
        if let Ok(imm) = checked_imm12(imm) {
            return self.emit_word(0xf100_001f | (imm << 10) | ((rn as u32) << 5));
        }
        if rn_is_sp {
            return Err(format!(
                "cmp_imm against the stack pointer needs an immediate that fits 12                  bits (got {imm}): the wider form encodes register 31 as XZR, which                  would compare against zero instead of sp"
            ));
        }
        let scratch = scratch_excluding(rn, 31);
        self.emit_mov_imm(scratch, imm)?;
        self.emit_cmp(rn, scratch)
    }

    pub(super) fn emit_cmp(&mut self, rn: u8, rm: u8) -> Result<(), String> {
        self.emit_word(0xeb00_001f | ((rm as u32) << 16) | ((rn as u32) << 5))
    }

    pub(super) fn emit_fmov_x_from_d(&mut self, rd: u8, dn: u8) -> Result<(), String> {
        self.emit_rr(0x9e66_0000, rd, dn)
    }

    pub(super) fn emit_fmov_d_from_x(&mut self, dd: u8, rn: u8) -> Result<(), String> {
        self.emit_rr(0x9e67_0000, dd, rn)
    }

    /// `FMOV Dd, Dn` — scalar double register-to-register move.
    pub(super) fn emit_fmov_d_from_d(&mut self, dd: u8, dn: u8) -> Result<(), String> {
        self.emit_rr(0x1e60_4000, dd, dn)
    }

    pub(super) fn emit_fadd_d(&mut self, dd: u8, dn: u8, dm: u8) -> Result<(), String> {
        self.emit_rrr(0x1e60_2800, dd, dn, dm)
    }

    pub(super) fn emit_fsub_d(&mut self, dd: u8, dn: u8, dm: u8) -> Result<(), String> {
        self.emit_rrr(0x1e60_3800, dd, dn, dm)
    }

    pub(super) fn emit_fmul_d(&mut self, dd: u8, dn: u8, dm: u8) -> Result<(), String> {
        self.emit_rrr(0x1e60_0800, dd, dn, dm)
    }

    pub(super) fn emit_fdiv_d(&mut self, dd: u8, dn: u8, dm: u8) -> Result<(), String> {
        self.emit_rrr(0x1e60_1800, dd, dn, dm)
    }

    /// `FMINNM Dd, Dn, Dm` — FP data-processing (2-source) `.d`, opcode `0111`.
    pub(super) fn emit_fminnm_d(&mut self, dd: u8, dn: u8, dm: u8) -> Result<(), String> {
        self.emit_rrr(0x1e60_7800, dd, dn, dm)
    }

    /// `FMAXNM Dd, Dn, Dm` — FP data-processing (2-source) `.d`, opcode `0110`.
    pub(super) fn emit_fmaxnm_d(&mut self, dd: u8, dn: u8, dm: u8) -> Result<(), String> {
        self.emit_rrr(0x1e60_6800, dd, dn, dm)
    }

    pub(super) fn emit_fneg_d(&mut self, dd: u8, dn: u8) -> Result<(), String> {
        self.emit_rr(0x1e61_4000, dd, dn)
    }

    pub(super) fn emit_fabs_d(&mut self, dd: u8, dn: u8) -> Result<(), String> {
        // FABS (scalar, double): opcode field 000001 in the FP-1-source group.
        self.emit_rr(0x1e60_c000, dd, dn)
    }

    pub(super) fn emit_fsqrt_d(&mut self, dd: u8, dn: u8) -> Result<(), String> {
        self.emit_rr(0x1e61_c000, dd, dn)
    }

    pub(super) fn emit_fcmp_d(&mut self, dn: u8, dm: u8) -> Result<(), String> {
        self.emit_word(0x1e60_2000 | ((dm as u32) << 16) | ((dn as u32) << 5))
    }

    pub(super) fn emit_fcmp_zero_d(&mut self, dn: u8) -> Result<(), String> {
        self.emit_word(0x1e60_2000 | ((dn as u32) << 5) | 0x8)
    }

    pub(super) fn emit_scvtf_d_from_x(&mut self, dd: u8, rn: u8) -> Result<(), String> {
        self.emit_rr(0x9e62_0000, dd, rn)
    }

    pub(super) fn emit_fcvtzs_x_from_d(&mut self, rd: u8, dn: u8) -> Result<(), String> {
        self.emit_rr(0x9e78_0000, rd, dn)
    }

    pub(super) fn emit_fcvtms_x_from_d(&mut self, rd: u8, dn: u8) -> Result<(), String> {
        self.emit_rr(0x9e70_0000, rd, dn)
    }

    pub(super) fn emit_fcvtps_x_from_d(&mut self, rd: u8, dn: u8) -> Result<(), String> {
        self.emit_rr(0x9e68_0000, rd, dn)
    }

    pub(super) fn emit_fcvtas_x_from_d(&mut self, rd: u8, dn: u8) -> Result<(), String> {
        self.emit_rr(0x9e64_0000, rd, dn)
    }
}
