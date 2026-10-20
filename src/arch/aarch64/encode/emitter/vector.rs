//! plan-176-O: the aarch64 encoder's the AdvSIMD/FP-vector instruction emitters. `Encoder`, the
//! `emit_instruction` dispatch and `patch_labels` live in `encode/emitter/mod.rs`.

use super::*;

impl Encoder {
    /// 128-bit `LDR Qt, [Xn, #offset]` — scaled form when the offset is a
    /// 16-aligned multiple within range, otherwise the address is materialized
    /// through a GPR scratch (the same fallback the `ldr x` family uses for
    /// out-of-range offsets; a huge frame puts FP spill slots past 65520).
    pub(super) fn emit_ldr_q(&mut self, vt: u8, rn: u8, offset: u64) -> Result<(), String> {
        if offset.is_multiple_of(16) && offset / 16 <= 4095 {
            let imm12 = (offset / 16) as u32;
            return self.emit_word(0x3dc0_0000 | (imm12 << 10) | ((rn as u32) << 5) | vt as u32);
        }
        // `vt` is a vector register, so only `rn` needs excluding.
        let scratch = scratch_excluding(rn, 31);
        self.emit_add_imm(scratch, rn, offset)?;
        self.emit_word(0x3dc0_0000 | ((scratch as u32) << 5) | vt as u32)
    }

    /// 128-bit `STR Qt, [Xn, #offset]` — scaled form when in range, else via a
    /// GPR scratch address (see [`Self::emit_ldr_q`]).
    pub(super) fn emit_str_q(&mut self, vt: u8, rn: u8, offset: u64) -> Result<(), String> {
        if offset.is_multiple_of(16) && offset / 16 <= 4095 {
            let imm12 = (offset / 16) as u32;
            return self.emit_word(0x3d80_0000 | (imm12 << 10) | ((rn as u32) << 5) | vt as u32);
        }
        let scratch = scratch_excluding(rn, 31);
        self.emit_add_imm(scratch, rn, offset)?;
        self.emit_word(0x3d80_0000 | ((scratch as u32) << 5) | vt as u32)
    }

    /// Three-same NEON ops: `op Vd.<T>, Vn.<T>, Vm.<T>`. The arrangement (`.2d`
    /// for numeric lanes, `.16b` for the bitwise ops) is baked into the base word.
    pub(super) fn emit_v_three_same(
        &mut self,
        op: CodeOp,
        vd: u8,
        vn: u8,
        vm: u8,
    ) -> Result<(), String> {
        let base = match op {
            CodeOp::FAddV => 0x4e60_d400,
            CodeOp::FSubV => 0x4ee0_d400,
            CodeOp::FMulV => 0x6e60_dc00,
            CodeOp::FDivV => 0x6e60_fc00,
            CodeOp::FMlaV => 0x4e60_cc00,
            CodeOp::FMlsV => 0x4ee0_cc00,
            CodeOp::FMinV => 0x4ee0_f400,
            CodeOp::FMaxV => 0x4e60_f400,
            CodeOp::FCmGtV => 0x6ee0_e400,
            CodeOp::FCmGeV => 0x6e60_e400,
            CodeOp::FCmEqV => 0x4e60_e400,
            CodeOp::AddV => 0x4ee0_8400,
            CodeOp::SubV => 0x6ee0_8400,
            CodeOp::CmGtV => 0x4ee0_3400,
            CodeOp::CmGeV => 0x4ee0_3c00,
            CodeOp::CmEqV => 0x6ee0_8c00,
            CodeOp::SshlV => 0x4ee0_4400,
            CodeOp::UshlV => 0x6ee0_4400,
            CodeOp::AndV => 0x4e20_1c00,
            CodeOp::OrrV => 0x4ea0_1c00,
            CodeOp::EorV => 0x6e20_1c00,
            CodeOp::BslV => 0x6e60_1c00,
            CodeOp::BitV => 0x6ea0_1c00,
            other => return Err(format!("{} is not a three-same NEON op", other.mnemonic())),
        };
        self.emit_word(base | ((vm as u32) << 16) | ((vn as u32) << 5) | vd as u32)
    }

    /// Two-register-misc NEON ops: `op Vd.<T>, Vn.<T>` (the compare-zero forms
    /// compare each lane against 0.0/0 implicitly).
    pub(super) fn emit_v_two_misc(&mut self, op: CodeOp, vd: u8, vn: u8) -> Result<(), String> {
        let base = match op {
            CodeOp::FAbsV => 0x4ee0_f800,
            CodeOp::FNegV => 0x6ee0_f800,
            CodeOp::FSqrtV => 0x6ee1_f800,
            CodeOp::FRintpV => 0x4ee1_8800,
            CodeOp::FRintmV => 0x4e61_9800,
            CodeOp::FRintaV => 0x6e61_8800,
            CodeOp::FRintnV => 0x4e61_8800,
            CodeOp::FRintzV => 0x4ee1_9800,
            CodeOp::FCvtzsV => 0x4ee1_b800,
            CodeOp::FCvtasV => 0x4e61_c800,
            CodeOp::ScvtfV => 0x4e61_d800,
            CodeOp::NegV => 0x6ee0_b800,
            CodeOp::AbsV => 0x4ee0_b800,
            CodeOp::FCmGtZeroV => 0x4ee0_c800,
            CodeOp::FCmGeZeroV => 0x6ee0_c800,
            CodeOp::FCmEqZeroV => 0x4ee0_d800,
            CodeOp::FCmLtZeroV => 0x4ee0_e800,
            CodeOp::FCmLeZeroV => 0x6ee0_d800,
            // plan-39 K2: `CNT Vd.8B, Vn.8B` and `ADDV Bd, Vn.8B` — both `.8B`
            // arrangement (Q=0), so they read the low 8 bytes of Vn only.
            CodeOp::Cnt8bV => 0x0e20_5800,
            CodeOp::Addv8bV => 0x0e31_b800,
            other => {
                return Err(format!(
                    "{} is not a two-reg-misc NEON op",
                    other.mnemonic()
                ))
            }
        };
        self.emit_word(base | ((vn as u32) << 5) | vd as u32)
    }

    /// Shifted-immediate NEON ops on `.2d` lanes (64-bit element). `shl` takes a
    /// left-shift 0..=63; `sshr`/`ushr` take a right-shift 1..=63. (ARM encodes a
    /// `.2d` right-shift up to 64, but the shared `shift()` operand parser rejects
    /// any amount >= 64, so 64 never reaches here — the encoder matches that bound
    /// rather than carrying an unreachable case, bug-217.)
    pub(super) fn emit_v_shift_imm(
        &mut self,
        op: CodeOp,
        vd: u8,
        vn: u8,
        amount: u8,
    ) -> Result<(), String> {
        let (base, immhb) = match op {
            CodeOp::ShlV => {
                if amount > 63 {
                    return Err(format!("AArch64 shl.2d shift {amount} is out of range"));
                }
                (0x4f00_5400, 64 + amount as u32)
            }
            CodeOp::SshrV => {
                if amount == 0 || amount > 63 {
                    return Err(format!("AArch64 sshr.2d shift {amount} is out of range"));
                }
                (0x4f00_0400, 128 - amount as u32)
            }
            CodeOp::UshrV => {
                if amount == 0 || amount > 63 {
                    return Err(format!("AArch64 ushr.2d shift {amount} is out of range"));
                }
                (0x6f00_0400, 128 - amount as u32)
            }
            other => return Err(format!("{} is not a NEON shift-imm op", other.mnemonic())),
        };
        self.emit_word(base | (immhb << 16) | ((vn as u32) << 5) | vd as u32)
    }

    /// `DUP Vd.2d, Xn` — broadcast a 64-bit GPR into both lanes.
    pub(super) fn emit_dup_v_from_x(&mut self, vd: u8, rn: u8) -> Result<(), String> {
        self.emit_word(0x4e08_0c00 | ((rn as u32) << 5) | vd as u32)
    }

    /// Scalar FP three-source (fused multiply-add family), `.d`. All four share the
    /// `Dm<<16 | Da<<10 | Dn<<5 | Dd` field layout and differ only in the `o1`
    /// (bit 21) / `o0` (bit 15) base word. The neutral MIR mnemonic names the
    /// *result* (`dst = addend ± lhs*rhs`); the AArch64 instruction that computes it
    /// does not always share the name (e.g. MIR `fmsub_d` = `lhs*rhs - addend` is
    /// AArch64 `FNMSUB` = `-Da + Dn*Dm`), so the mapping is pinned by byte tests:
    ///   `fmadd_d`  → FMADD  `Da + Dn*Dm`   = addend + lhs*rhs
    ///   `fmsub_d`  → FNMSUB `-Da + Dn*Dm`  = lhs*rhs - addend
    ///   `fnmsub_d` → FMSUB  `Da - Dn*Dm`   = addend - lhs*rhs
    ///   `fnmadd_d` → FNMADD `-Da - Dn*Dm`  = -(lhs*rhs) - addend
    pub(super) fn emit_fma_d(
        &mut self,
        op: CodeOp,
        dd: u8,
        da: u8,
        dn: u8,
        dm: u8,
    ) -> Result<(), String> {
        let base = match op {
            CodeOp::FMaddD => 0x1f40_0000,  // FMADD
            CodeOp::FMsubD => 0x1f60_8000,  // FNMSUB
            CodeOp::FNmsubD => 0x1f40_8000, // FMSUB
            CodeOp::FNmaddD => 0x1f60_0000, // FNMADD
            other => return Err(format!("{} is not a scalar FMA op", other.mnemonic())),
        };
        self.emit_word(
            base | ((dm as u32) << 16) | ((da as u32) << 10) | ((dn as u32) << 5) | dd as u32,
        )
    }

    /// `UMOV Xd, Vn.d[index]` — extract lane `index` (0 or 1) into a GPR.
    pub(super) fn emit_umov_x_from_v(&mut self, rd: u8, vn: u8, index: u64) -> Result<(), String> {
        if index > 1 {
            return Err(format!(
                "AArch64 umov .d lane index {index} is out of range"
            ));
        }
        let imm5 = 0x8 | ((index as u32) << 4);
        self.emit_word(0x4e00_3c00 | (imm5 << 16) | ((vn as u32) << 5) | rd as u32)
    }
}
