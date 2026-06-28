use super::*;
use super::operand::{field, immediate, reg, scratch_excluding, shift, vreg};
use super::sizing::{
    branch_imm19, branch_imm26, checked_imm12, encode_add_sub_imm, next_add_sub_chunk,
};

pub(super) struct Encoder {
    pub(super) text: Vec<u8>,
    pub(super) data: Vec<u8>,
    pub(super) symbols: Vec<EncodedSymbol>,
    pub(super) relocations: Vec<EncodedRelocation>,
    pub(super) imports: HashMap<String, String>,
    pub(super) labels: HashMap<String, usize>,
    pub(super) patches: Vec<LabelPatch>,
}

pub(super) struct LabelPatch {
    offset: usize,
    target: String,
    kind: String,
}

impl Encoder {
    pub(super) fn emit_instruction(&mut self, instruction: &CodeInstruction) -> Result<(), String> {
        match instruction.op.mnemonic() {
            "label" => Ok(()),
            "mov" => self.emit_mov(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "src")?)?,
            ),
            "mov_imm" => self.emit_mov_imm(
                reg(field(instruction, "dst")?)?,
                immediate(field(instruction, "value")?)?,
            ),
            "add" => self.emit_add(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "lhs")?)?,
                reg(field(instruction, "rhs")?)?,
            ),
            "adds" => self.emit_adds(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "lhs")?)?,
                reg(field(instruction, "rhs")?)?,
            ),
            "sub" => self.emit_sub(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "lhs")?)?,
                reg(field(instruction, "rhs")?)?,
            ),
            "subs" => self.emit_subs(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "lhs")?)?,
                reg(field(instruction, "rhs")?)?,
            ),
            "and" => self.emit_and(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "lhs")?)?,
                reg(field(instruction, "rhs")?)?,
            ),
            "orr" => self.emit_orr(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "lhs")?)?,
                reg(field(instruction, "rhs")?)?,
            ),
            "eor" => self.emit_eor(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "lhs")?)?,
                reg(field(instruction, "rhs")?)?,
            ),
            "mvn" => self.emit_mvn(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "src")?)?,
            ),
            "mul" => self.emit_mul(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "lhs")?)?,
                reg(field(instruction, "rhs")?)?,
            ),
            "smulh" => self.emit_smulh(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "lhs")?)?,
                reg(field(instruction, "rhs")?)?,
            ),
            "umulh" => self.emit_umulh(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "lhs")?)?,
                reg(field(instruction, "rhs")?)?,
            ),
            "adc" => self.emit_adc(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "lhs")?)?,
                reg(field(instruction, "rhs")?)?,
            ),
            "rorv" => self.emit_rorv(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "lhs")?)?,
                reg(field(instruction, "rhs")?)?,
            ),
            "rorv_w" => self.emit_rorv_w(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "lhs")?)?,
                reg(field(instruction, "rhs")?)?,
            ),
            "lslv" => self.emit_lslv(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "lhs")?)?,
                reg(field(instruction, "rhs")?)?,
            ),
            "lsrv" => self.emit_lsrv(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "lhs")?)?,
                reg(field(instruction, "rhs")?)?,
            ),
            "asrv" => self.emit_asrv(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "lhs")?)?,
                reg(field(instruction, "rhs")?)?,
            ),
            "clz" => self.emit_clz(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "src")?)?,
            ),
            "rbit" => self.emit_rbit(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "src")?)?,
            ),
            "rev_w" => self.emit_rev_w(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "src")?)?,
            ),
            "rev_x" => self.emit_rev_x(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "src")?)?,
            ),
            "sdiv" => self.emit_sdiv(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "lhs")?)?,
                reg(field(instruction, "rhs")?)?,
            ),
            "udiv" => self.emit_udiv(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "lhs")?)?,
                reg(field(instruction, "rhs")?)?,
            ),
            "msub" => self.emit_msub(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "lhs")?)?,
                reg(field(instruction, "rhs")?)?,
                reg(field(instruction, "minuend")?)?,
            ),
            "lsl_imm" => self.emit_lsl_imm(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "src")?)?,
                shift(field(instruction, "shift")?)?,
            ),
            "lsr_imm" => self.emit_lsr_imm(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "src")?)?,
                shift(field(instruction, "shift")?)?,
            ),
            "asr_imm" => self.emit_asr_imm(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "src")?)?,
                shift(field(instruction, "shift")?)?,
            ),
            "add_imm" => self.emit_add_imm(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "src")?)?,
                immediate(field(instruction, "imm")?)?,
            ),
            "sub_imm" => self.emit_sub_imm(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "src")?)?,
                immediate(field(instruction, "imm")?)?,
            ),
            "sub_sp" => self.emit_sub_sp(immediate(field(instruction, "imm")?)?),
            "add_sp" => self.emit_add_sp(immediate(field(instruction, "imm")?)?),
            "cmp_imm" => self.emit_cmp_imm(
                reg(field(instruction, "lhs")?)?,
                immediate(field(instruction, "rhs")?)?,
            ),
            "cmp" => self.emit_cmp(
                reg(field(instruction, "lhs")?)?,
                reg(field(instruction, "rhs")?)?,
            ),
            "b.eq" => self.emit_label_branch("b.eq", field(instruction, "target")?),
            "b.ne" => self.emit_label_branch("b.ne", field(instruction, "target")?),
            "b.ge" => self.emit_label_branch("b.ge", field(instruction, "target")?),
            "b.lt" => self.emit_label_branch("b.lt", field(instruction, "target")?),
            "b.gt" => self.emit_label_branch("b.gt", field(instruction, "target")?),
            "b.le" => self.emit_label_branch("b.le", field(instruction, "target")?),
            "b.vc" => self.emit_label_branch("b.vc", field(instruction, "target")?),
            "b.hi" => self.emit_label_branch("b.hi", field(instruction, "target")?),
            "b.lo" => self.emit_label_branch("b.lo", field(instruction, "target")?),
            "b" => self.emit_label_branch("b", field(instruction, "target")?),
            "bl" => self.emit_bl(field(instruction, "target")?),
            "blr" => self.emit_blr(reg(field(instruction, "register")?)?),
            "svc" => self.emit_word(0xd400_0001),
            "branch_self" => self.emit_word(0x1400_0000),
            "ret" => self.emit_word(0xd65f_03c0),
            "ldr_u64" => self.emit_ldr_u64(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "base")?)?,
                immediate(field(instruction, "offset")?)?,
            ),
            "ldr_u32" => self.emit_ldr_u32(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "base")?)?,
                immediate(field(instruction, "offset")?)?,
            ),
            "ldr_u16" => self.emit_ldr_u16(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "base")?)?,
                immediate(field(instruction, "offset")?)?,
            ),
            "ldr_u8" => self.emit_ldr_u8(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "base")?)?,
                immediate(field(instruction, "offset")?)?,
            ),
            "str_u64" => self.emit_str_u64(
                reg(field(instruction, "src")?)?,
                reg(field(instruction, "base")?)?,
                immediate(field(instruction, "offset")?)?,
            ),
            "str_u32" => self.emit_str_u32(
                reg(field(instruction, "src")?)?,
                reg(field(instruction, "base")?)?,
                immediate(field(instruction, "offset")?)?,
            ),
            "str_u8" => self.emit_str_u8(
                reg(field(instruction, "src")?)?,
                reg(field(instruction, "base")?)?,
                immediate(field(instruction, "offset")?)?,
            ),
            "adrp" => self.emit_symbol_ref(
                "adrp",
                reg(field(instruction, "dst")?)?,
                field(instruction, "symbol")?,
            ),
            "add_pageoff" => self.emit_symbol_ref(
                "add_pageoff",
                reg(field(instruction, "dst")?)?,
                field(instruction, "symbol")?,
            ),
            "fmov_x_from_d" => self.emit_fmov_x_from_d(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "src")?)?,
            ),
            "fmov_d_from_x" => self.emit_fmov_d_from_x(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "src")?)?,
            ),
            "fadd_d" => self.emit_fadd_d(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "lhs")?)?,
                reg(field(instruction, "rhs")?)?,
            ),
            "fsub_d" => self.emit_fsub_d(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "lhs")?)?,
                reg(field(instruction, "rhs")?)?,
            ),
            "fmul_d" => self.emit_fmul_d(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "lhs")?)?,
                reg(field(instruction, "rhs")?)?,
            ),
            "fdiv_d" => self.emit_fdiv_d(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "lhs")?)?,
                reg(field(instruction, "rhs")?)?,
            ),
            "fneg_d" => self.emit_fneg_d(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "src")?)?,
            ),
            "fsqrt_d" => self.emit_fsqrt_d(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "src")?)?,
            ),
            "fcmp_d" => self.emit_fcmp_d(
                reg(field(instruction, "lhs")?)?,
                reg(field(instruction, "rhs")?)?,
            ),
            "fcmp_zero_d" => self.emit_fcmp_zero_d(reg(field(instruction, "src")?)?),
            "scvtf_d_from_x" => self.emit_scvtf_d_from_x(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "src")?)?,
            ),
            "fcvtzs_x_from_d" => self.emit_fcvtzs_x_from_d(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "src")?)?,
            ),
            "fcvtms_x_from_d" => self.emit_fcvtms_x_from_d(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "src")?)?,
            ),
            "fcvtps_x_from_d" => self.emit_fcvtps_x_from_d(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "src")?)?,
            ),
            "fcvtas_x_from_d" => self.emit_fcvtas_x_from_d(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "src")?)?,
            ),
            "ldr_q" => self.emit_ldr_q(
                vreg(field(instruction, "dst")?)?,
                reg(field(instruction, "base")?)?,
                immediate(field(instruction, "offset")?)?,
            ),
            "str_q" => self.emit_str_q(
                vreg(field(instruction, "src")?)?,
                reg(field(instruction, "base")?)?,
                immediate(field(instruction, "offset")?)?,
            ),
            "fadd_v" | "fsub_v" | "fmul_v" | "fdiv_v" | "fmla_v" | "fmls_v" | "fmin_v"
            | "fmax_v" | "fcmgt_v" | "fcmge_v" | "fcmeq_v" | "add_v" | "sub_v" | "cmgt_v"
            | "cmge_v" | "cmeq_v" | "sshl_v" | "ushl_v" | "and_v" | "orr_v" | "eor_v"
            | "bsl_v" | "bit_v" => self.emit_v_three_same(
                instruction.op,
                vreg(field(instruction, "dst")?)?,
                vreg(field(instruction, "lhs")?)?,
                vreg(field(instruction, "rhs")?)?,
            ),
            "fabs_v" | "fneg_v" | "fsqrt_v" | "frintp_v" | "frintm_v" | "frinta_v"
            | "frintn_v" | "frintz_v" | "fcvtzs_v" | "fcvtas_v" | "scvtf_v" | "neg_v"
            | "abs_v" | "fcmgt_zero_v" | "fcmge_zero_v" | "fcmeq_zero_v" | "fcmlt_zero_v"
            | "fcmle_zero_v" => self.emit_v_two_misc(
                instruction.op,
                vreg(field(instruction, "dst")?)?,
                vreg(field(instruction, "src")?)?,
            ),
            "shl_v" | "sshr_v" | "ushr_v" => self.emit_v_shift_imm(
                instruction.op,
                vreg(field(instruction, "dst")?)?,
                vreg(field(instruction, "src")?)?,
                shift(field(instruction, "shift")?)?,
            ),
            "dup_v_from_x" => self.emit_dup_v_from_x(
                vreg(field(instruction, "dst")?)?,
                reg(field(instruction, "src")?)?,
            ),
            "umov_x_from_v" => self.emit_umov_x_from_v(
                reg(field(instruction, "dst")?)?,
                vreg(field(instruction, "src")?)?,
                immediate(field(instruction, "index")?)?,
            ),
            "fmadd_d" => self.emit_fmadd_d(
                reg(field(instruction, "dst")?)?,
                reg(field(instruction, "addend")?)?,
                reg(field(instruction, "lhs")?)?,
                reg(field(instruction, "rhs")?)?,
            ),
            other => Err(format!(
                "AArch64 encoder does not support instruction '{other}'"
            )),
        }
    }

    /// 128-bit `LDR Qt, [Xn, #offset]` — offset must be a multiple of 16.
    fn emit_ldr_q(&mut self, vt: u8, rn: u8, offset: u64) -> Result<(), String> {
        if offset % 16 != 0 || offset / 16 > 4095 {
            return Err(format!("AArch64 ldr q offset {offset} is not encodable"));
        }
        let imm12 = (offset / 16) as u32;
        self.emit_word(0x3dc0_0000 | (imm12 << 10) | ((rn as u32) << 5) | vt as u32)
    }

    /// 128-bit `STR Qt, [Xn, #offset]` — offset must be a multiple of 16.
    fn emit_str_q(&mut self, vt: u8, rn: u8, offset: u64) -> Result<(), String> {
        if offset % 16 != 0 || offset / 16 > 4095 {
            return Err(format!("AArch64 str q offset {offset} is not encodable"));
        }
        let imm12 = (offset / 16) as u32;
        self.emit_word(0x3d80_0000 | (imm12 << 10) | ((rn as u32) << 5) | vt as u32)
    }

    /// Three-same NEON ops: `op Vd.<T>, Vn.<T>, Vm.<T>`. The arrangement (`.2d`
    /// for numeric lanes, `.16b` for the bitwise ops) is baked into the base word.
    fn emit_v_three_same(&mut self, op: CodeOp, vd: u8, vn: u8, vm: u8) -> Result<(), String> {
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
    fn emit_v_two_misc(&mut self, op: CodeOp, vd: u8, vn: u8) -> Result<(), String> {
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
            other => return Err(format!("{} is not a two-reg-misc NEON op", other.mnemonic())),
        };
        self.emit_word(base | ((vn as u32) << 5) | vd as u32)
    }

    /// Shifted-immediate NEON ops on `.2d` lanes (64-bit element). `shl` takes a
    /// left-shift 0..=63; `sshr`/`ushr` take a right-shift 1..=64.
    fn emit_v_shift_imm(&mut self, op: CodeOp, vd: u8, vn: u8, amount: u8) -> Result<(), String> {
        let (base, immhb) = match op {
            CodeOp::ShlV => {
                if amount > 63 {
                    return Err(format!("AArch64 shl.2d shift {amount} is out of range"));
                }
                (0x4f00_5400, 64 + amount as u32)
            }
            CodeOp::SshrV => {
                if amount == 0 || amount > 64 {
                    return Err(format!("AArch64 sshr.2d shift {amount} is out of range"));
                }
                (0x4f00_0400, 128 - amount as u32)
            }
            CodeOp::UshrV => {
                if amount == 0 || amount > 64 {
                    return Err(format!("AArch64 ushr.2d shift {amount} is out of range"));
                }
                (0x6f00_0400, 128 - amount as u32)
            }
            other => return Err(format!("{} is not a NEON shift-imm op", other.mnemonic())),
        };
        self.emit_word(base | (immhb << 16) | ((vn as u32) << 5) | vd as u32)
    }

    /// `DUP Vd.2d, Xn` — broadcast a 64-bit GPR into both lanes.
    fn emit_dup_v_from_x(&mut self, vd: u8, rn: u8) -> Result<(), String> {
        self.emit_word(0x4e08_0c00 | ((rn as u32) << 5) | vd as u32)
    }

    /// `FMADD Dd, Dn, Dm, Da` — `Dd = Da + Dn*Dm`, rounded once.
    fn emit_fmadd_d(&mut self, dd: u8, da: u8, dn: u8, dm: u8) -> Result<(), String> {
        self.emit_word(
            0x1f40_0000
                | ((dm as u32) << 16)
                | ((da as u32) << 10)
                | ((dn as u32) << 5)
                | dd as u32,
        )
    }

    /// `UMOV Xd, Vn.d[index]` — extract lane `index` (0 or 1) into a GPR.
    fn emit_umov_x_from_v(&mut self, rd: u8, vn: u8, index: u64) -> Result<(), String> {
        if index > 1 {
            return Err(format!("AArch64 umov .d lane index {index} is out of range"));
        }
        let imm5 = 0x8 | ((index as u32) << 4);
        self.emit_word(0x4e00_3c00 | (imm5 << 16) | ((vn as u32) << 5) | rd as u32)
    }

    fn emit_word(&mut self, word: u32) -> Result<(), String> {
        self.text.extend_from_slice(&word.to_le_bytes());
        Ok(())
    }

    fn emit_mov(&mut self, rd: u8, rn: u8) -> Result<(), String> {
        self.emit_word(0xaa00_03e0 | ((rn as u32) << 16) | rd as u32)
    }

    fn emit_mov_imm(&mut self, rd: u8, value: u64) -> Result<(), String> {
        let mut emitted = false;
        for (index, shift) in [0, 16, 32, 48].into_iter().enumerate() {
            let part = ((value >> shift) & 0xffff) as u32;
            if index == 0 {
                self.emit_word(0xd280_0000 | (part << 5) | rd as u32)?;
                emitted = true;
            } else if part != 0 {
                self.emit_word(
                    0xf280_0000 | (((shift / 16) as u32) << 21) | (part << 5) | rd as u32,
                )?;
            }
        }
        if !emitted {
            self.emit_word(0xd280_0000 | rd as u32)?;
        }
        Ok(())
    }

    fn emit_add(&mut self, rd: u8, rn: u8, rm: u8) -> Result<(), String> {
        self.emit_word(0x8b00_0000 | ((rm as u32) << 16) | ((rn as u32) << 5) | rd as u32)
    }

    fn emit_adds(&mut self, rd: u8, rn: u8, rm: u8) -> Result<(), String> {
        self.emit_word(0xab00_0000 | ((rm as u32) << 16) | ((rn as u32) << 5) | rd as u32)
    }

    fn emit_sub(&mut self, rd: u8, rn: u8, rm: u8) -> Result<(), String> {
        self.emit_word(0xcb00_0000 | ((rm as u32) << 16) | ((rn as u32) << 5) | rd as u32)
    }

    fn emit_subs(&mut self, rd: u8, rn: u8, rm: u8) -> Result<(), String> {
        self.emit_word(0xeb00_0000 | ((rm as u32) << 16) | ((rn as u32) << 5) | rd as u32)
    }

    fn emit_and(&mut self, rd: u8, rn: u8, rm: u8) -> Result<(), String> {
        self.emit_word(0x8a00_0000 | ((rm as u32) << 16) | ((rn as u32) << 5) | rd as u32)
    }

    fn emit_orr(&mut self, rd: u8, rn: u8, rm: u8) -> Result<(), String> {
        self.emit_word(0xaa00_0000 | ((rm as u32) << 16) | ((rn as u32) << 5) | rd as u32)
    }

    fn emit_eor(&mut self, rd: u8, rn: u8, rm: u8) -> Result<(), String> {
        self.emit_word(0xca00_0000 | ((rm as u32) << 16) | ((rn as u32) << 5) | rd as u32)
    }

    fn emit_mvn(&mut self, rd: u8, rm: u8) -> Result<(), String> {
        self.emit_word(0xaa20_03e0 | ((rm as u32) << 16) | rd as u32)
    }

    fn emit_mul(&mut self, rd: u8, rn: u8, rm: u8) -> Result<(), String> {
        self.emit_word(0x9b00_7c00 | ((rm as u32) << 16) | ((rn as u32) << 5) | rd as u32)
    }

    fn emit_smulh(&mut self, rd: u8, rn: u8, rm: u8) -> Result<(), String> {
        self.emit_word(0x9b40_7c00 | ((rm as u32) << 16) | ((rn as u32) << 5) | rd as u32)
    }

    fn emit_umulh(&mut self, rd: u8, rn: u8, rm: u8) -> Result<(), String> {
        self.emit_word(0x9bc0_7c00 | ((rm as u32) << 16) | ((rn as u32) << 5) | rd as u32)
    }

    fn emit_adc(&mut self, rd: u8, rn: u8, rm: u8) -> Result<(), String> {
        self.emit_word(0x9a00_0000 | ((rm as u32) << 16) | ((rn as u32) << 5) | rd as u32)
    }

    fn emit_rorv(&mut self, rd: u8, rn: u8, rm: u8) -> Result<(), String> {
        self.emit_word(0x9ac0_2c00 | ((rm as u32) << 16) | ((rn as u32) << 5) | rd as u32)
    }

    /// 32-bit `RORV Wd, Wn, Wm` — rotate right by `Wm mod 32`; zero-extends Wd.
    fn emit_rorv_w(&mut self, rd: u8, rn: u8, rm: u8) -> Result<(), String> {
        self.emit_word(0x1ac0_2c00 | ((rm as u32) << 16) | ((rn as u32) << 5) | rd as u32)
    }

    /// 64-bit `LSLV Xd, Xn, Xm` — logical shift left by `Xm mod 64`.
    fn emit_lslv(&mut self, rd: u8, rn: u8, rm: u8) -> Result<(), String> {
        self.emit_word(0x9ac0_2000 | ((rm as u32) << 16) | ((rn as u32) << 5) | rd as u32)
    }

    /// 64-bit `LSRV Xd, Xn, Xm` — logical shift right by `Xm mod 64`.
    fn emit_lsrv(&mut self, rd: u8, rn: u8, rm: u8) -> Result<(), String> {
        self.emit_word(0x9ac0_2400 | ((rm as u32) << 16) | ((rn as u32) << 5) | rd as u32)
    }

    /// 64-bit `ASRV Xd, Xn, Xm` — arithmetic shift right by `Xm mod 64`.
    fn emit_asrv(&mut self, rd: u8, rn: u8, rm: u8) -> Result<(), String> {
        self.emit_word(0x9ac0_2800 | ((rm as u32) << 16) | ((rn as u32) << 5) | rd as u32)
    }

    /// 64-bit `CLZ Xd, Xn` — count leading zeros.
    fn emit_clz(&mut self, rd: u8, rn: u8) -> Result<(), String> {
        self.emit_word(0xdac0_1000 | ((rn as u32) << 5) | rd as u32)
    }

    /// 64-bit `RBIT Xd, Xn` — reverse bit order.
    fn emit_rbit(&mut self, rd: u8, rn: u8) -> Result<(), String> {
        self.emit_word(0xdac0_0000 | ((rn as u32) << 5) | rd as u32)
    }

    /// 32-bit `REV Wd, Wn` — reverse the four bytes of Wn; zero-extends Wd.
    fn emit_rev_w(&mut self, rd: u8, rn: u8) -> Result<(), String> {
        self.emit_word(0x5ac0_0800 | ((rn as u32) << 5) | rd as u32)
    }

    /// 64-bit `REV Xd, Xn` — reverse all eight bytes of Xn.
    fn emit_rev_x(&mut self, rd: u8, rn: u8) -> Result<(), String> {
        self.emit_word(0xdac0_0c00 | ((rn as u32) << 5) | rd as u32)
    }

    fn emit_sdiv(&mut self, rd: u8, rn: u8, rm: u8) -> Result<(), String> {
        self.emit_word(0x9ac0_0c00 | ((rm as u32) << 16) | ((rn as u32) << 5) | rd as u32)
    }

    fn emit_udiv(&mut self, rd: u8, rn: u8, rm: u8) -> Result<(), String> {
        self.emit_word(0x9ac0_0800 | ((rm as u32) << 16) | ((rn as u32) << 5) | rd as u32)
    }

    fn emit_msub(&mut self, rd: u8, rn: u8, rm: u8, ra: u8) -> Result<(), String> {
        self.emit_word(
            0x9b00_8000
                | ((rm as u32) << 16)
                | ((ra as u32) << 10)
                | ((rn as u32) << 5)
                | rd as u32,
        )
    }

    fn emit_lsl_imm(&mut self, rd: u8, rn: u8, shift: u8) -> Result<(), String> {
        if shift >= 64 {
            return Err(format!("AArch64 lsl shift {shift} is out of range"));
        }
        let immr = (64 - shift as u32) & 63;
        let imms = 63 - shift as u32;
        self.emit_word(0xd340_0000 | (immr << 16) | (imms << 10) | ((rn as u32) << 5) | rd as u32)
    }

    fn emit_lsr_imm(&mut self, rd: u8, rn: u8, shift: u8) -> Result<(), String> {
        if shift >= 64 {
            return Err(format!("AArch64 lsr shift {shift} is out of range"));
        }
        self.emit_word(
            0xd340_0000 | ((shift as u32) << 16) | (63 << 10) | ((rn as u32) << 5) | rd as u32,
        )
    }

    fn emit_asr_imm(&mut self, rd: u8, rn: u8, shift: u8) -> Result<(), String> {
        if shift >= 64 {
            return Err(format!("AArch64 asr shift {shift} is out of range"));
        }
        self.emit_word(
            0x9340_0000 | ((shift as u32) << 16) | (63 << 10) | ((rn as u32) << 5) | rd as u32,
        )
    }

    fn emit_add_imm(&mut self, rd: u8, rn: u8, imm: u64) -> Result<(), String> {
        if let Some((imm12, shift12)) = encode_add_sub_imm(imm) {
            return self.emit_add_imm_chunk(rd, rn, imm12, shift12);
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

    fn emit_sub_imm(&mut self, rd: u8, rn: u8, imm: u64) -> Result<(), String> {
        if let Some((imm12, shift12)) = encode_add_sub_imm(imm) {
            return self.emit_sub_imm_chunk(rd, rn, imm12, shift12);
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

    fn emit_sub_sp(&mut self, imm: u64) -> Result<(), String> {
        self.emit_sub_imm(31, 31, imm)
    }

    fn emit_add_sp(&mut self, imm: u64) -> Result<(), String> {
        self.emit_add_imm(31, 31, imm)
    }

    fn emit_cmp_imm(&mut self, rn: u8, imm: u64) -> Result<(), String> {
        if let Ok(imm) = checked_imm12(imm) {
            return self.emit_word(0xf100_001f | (imm << 10) | ((rn as u32) << 5));
        }
        let scratch = scratch_excluding(rn, 31);
        self.emit_mov_imm(scratch, imm)?;
        self.emit_cmp(rn, scratch)
    }

    fn emit_cmp(&mut self, rn: u8, rm: u8) -> Result<(), String> {
        self.emit_word(0xeb00_001f | ((rm as u32) << 16) | ((rn as u32) << 5))
    }

    fn emit_fmov_x_from_d(&mut self, rd: u8, dn: u8) -> Result<(), String> {
        self.emit_word(0x9e66_0000 | ((dn as u32) << 5) | rd as u32)
    }

    fn emit_fmov_d_from_x(&mut self, dd: u8, rn: u8) -> Result<(), String> {
        self.emit_word(0x9e67_0000 | ((rn as u32) << 5) | dd as u32)
    }

    fn emit_fadd_d(&mut self, dd: u8, dn: u8, dm: u8) -> Result<(), String> {
        self.emit_word(0x1e60_2800 | ((dm as u32) << 16) | ((dn as u32) << 5) | dd as u32)
    }

    fn emit_fsub_d(&mut self, dd: u8, dn: u8, dm: u8) -> Result<(), String> {
        self.emit_word(0x1e60_3800 | ((dm as u32) << 16) | ((dn as u32) << 5) | dd as u32)
    }

    fn emit_fmul_d(&mut self, dd: u8, dn: u8, dm: u8) -> Result<(), String> {
        self.emit_word(0x1e60_0800 | ((dm as u32) << 16) | ((dn as u32) << 5) | dd as u32)
    }

    fn emit_fdiv_d(&mut self, dd: u8, dn: u8, dm: u8) -> Result<(), String> {
        self.emit_word(0x1e60_1800 | ((dm as u32) << 16) | ((dn as u32) << 5) | dd as u32)
    }

    fn emit_fneg_d(&mut self, dd: u8, dn: u8) -> Result<(), String> {
        self.emit_word(0x1e61_4000 | ((dn as u32) << 5) | dd as u32)
    }

    fn emit_fsqrt_d(&mut self, dd: u8, dn: u8) -> Result<(), String> {
        self.emit_word(0x1e61_c000 | ((dn as u32) << 5) | dd as u32)
    }

    fn emit_fcmp_d(&mut self, dn: u8, dm: u8) -> Result<(), String> {
        self.emit_word(0x1e60_2000 | ((dm as u32) << 16) | ((dn as u32) << 5))
    }

    fn emit_fcmp_zero_d(&mut self, dn: u8) -> Result<(), String> {
        self.emit_word(0x1e60_2000 | ((dn as u32) << 5) | 0x8)
    }

    fn emit_scvtf_d_from_x(&mut self, dd: u8, rn: u8) -> Result<(), String> {
        self.emit_word(0x9e62_0000 | ((rn as u32) << 5) | dd as u32)
    }

    fn emit_fcvtzs_x_from_d(&mut self, rd: u8, dn: u8) -> Result<(), String> {
        self.emit_word(0x9e78_0000 | ((dn as u32) << 5) | rd as u32)
    }

    fn emit_fcvtms_x_from_d(&mut self, rd: u8, dn: u8) -> Result<(), String> {
        self.emit_word(0x9e70_0000 | ((dn as u32) << 5) | rd as u32)
    }

    fn emit_fcvtps_x_from_d(&mut self, rd: u8, dn: u8) -> Result<(), String> {
        self.emit_word(0x9e68_0000 | ((dn as u32) << 5) | rd as u32)
    }

    fn emit_fcvtas_x_from_d(&mut self, rd: u8, dn: u8) -> Result<(), String> {
        self.emit_word(0x9e64_0000 | ((dn as u32) << 5) | rd as u32)
    }

    fn emit_ldr_u64(&mut self, rt: u8, rn: u8, offset: u64) -> Result<(), String> {
        if offset % 8 != 0 {
            return Err(format!("unaligned AArch64 ldr offset {offset}"));
        }
        if let Ok(imm) = checked_imm12(offset / 8) {
            return self.emit_word(0xf940_0000 | (imm << 10) | ((rn as u32) << 5) | rt as u32);
        }
        let scratch = scratch_excluding(rt, rn);
        self.emit_add_imm(scratch, rn, offset)?;
        self.emit_word(0xf940_0000 | ((scratch as u32) << 5) | rt as u32)
    }

    fn emit_ldr_u32(&mut self, rt: u8, rn: u8, offset: u64) -> Result<(), String> {
        if offset % 4 != 0 {
            return Err(format!("unaligned AArch64 ldr u32 offset {offset}"));
        }
        if let Ok(imm) = checked_imm12(offset / 4) {
            return self.emit_word(0xb940_0000 | (imm << 10) | ((rn as u32) << 5) | rt as u32);
        }
        let scratch = scratch_excluding(rt, rn);
        self.emit_add_imm(scratch, rn, offset)?;
        self.emit_word(0xb940_0000 | ((scratch as u32) << 5) | rt as u32)
    }

    fn emit_ldr_u16(&mut self, rt: u8, rn: u8, offset: u64) -> Result<(), String> {
        if offset % 2 != 0 {
            return Err(format!("unaligned AArch64 ldr u16 offset {offset}"));
        }
        if let Ok(imm) = checked_imm12(offset / 2) {
            return self.emit_word(0x7940_0000 | (imm << 10) | ((rn as u32) << 5) | rt as u32);
        }
        let scratch = scratch_excluding(rt, rn);
        self.emit_add_imm(scratch, rn, offset)?;
        self.emit_word(0x7940_0000 | ((scratch as u32) << 5) | rt as u32)
    }

    fn emit_str_u64(&mut self, rt: u8, rn: u8, offset: u64) -> Result<(), String> {
        if offset % 8 != 0 {
            return Err(format!("unaligned AArch64 str offset {offset}"));
        }
        if let Ok(imm) = checked_imm12(offset / 8) {
            return self.emit_word(0xf900_0000 | (imm << 10) | ((rn as u32) << 5) | rt as u32);
        }
        let scratch = scratch_excluding(rt, rn);
        self.emit_add_imm(scratch, rn, offset)?;
        self.emit_word(0xf900_0000 | ((scratch as u32) << 5) | rt as u32)
    }

    fn emit_str_u32(&mut self, rt: u8, rn: u8, offset: u64) -> Result<(), String> {
        if offset % 4 != 0 {
            return Err(format!("unaligned AArch64 str u32 offset {offset}"));
        }
        if let Ok(imm) = checked_imm12(offset / 4) {
            return self.emit_word(0xb900_0000 | (imm << 10) | ((rn as u32) << 5) | rt as u32);
        }
        let scratch = scratch_excluding(rt, rn);
        self.emit_add_imm(scratch, rn, offset)?;
        self.emit_word(0xb900_0000 | ((scratch as u32) << 5) | rt as u32)
    }

    fn emit_ldr_u8(&mut self, rt: u8, rn: u8, offset: u64) -> Result<(), String> {
        if let Ok(imm) = checked_imm12(offset) {
            return self.emit_word(0x3940_0000 | (imm << 10) | ((rn as u32) << 5) | rt as u32);
        }
        let scratch = scratch_excluding(rt, rn);
        self.emit_add_imm(scratch, rn, offset)?;
        self.emit_word(0x3940_0000 | ((scratch as u32) << 5) | rt as u32)
    }

    fn emit_str_u8(&mut self, rt: u8, rn: u8, offset: u64) -> Result<(), String> {
        if let Ok(imm) = checked_imm12(offset) {
            return self.emit_word(0x3900_0000 | (imm << 10) | ((rn as u32) << 5) | rt as u32);
        }
        let scratch = scratch_excluding(rt, rn);
        self.emit_add_imm(scratch, rn, offset)?;
        self.emit_word(0x3900_0000 | ((scratch as u32) << 5) | rt as u32)
    }

    fn emit_label_branch(&mut self, kind: &str, target: String) -> Result<(), String> {
        let offset = self.text.len();
        self.emit_word(0)?;
        self.patches.push(LabelPatch {
            offset,
            target,
            kind: kind.to_string(),
        });
        Ok(())
    }

    fn emit_bl(&mut self, target: String) -> Result<(), String> {
        let offset = self.text.len();
        self.emit_word(0x9400_0000)?;
        if self.symbols.iter().any(|symbol| symbol.name == target) {
            self.relocations.push(EncodedRelocation {
                offset,
                target,
                kind: "branch26".to_string(),
                binding: "internal".to_string(),
                library: None,
            });
        } else if let Some(library) = self.imports.get(&target) {
            self.relocations.push(EncodedRelocation {
                offset,
                target,
                kind: "branch26".to_string(),
                binding: "external".to_string(),
                library: Some(library.clone()),
            });
        } else {
            return Err(format!(
                "AArch64 branch target symbol '{target}' does not resolve"
            ));
        }
        Ok(())
    }

    fn emit_blr(&mut self, rn: u8) -> Result<(), String> {
        self.emit_word(0xd63f_0000 | ((rn as u32) << 5))
    }

    fn emit_symbol_ref(&mut self, kind: &str, rd: u8, symbol: String) -> Result<(), String> {
        let offset = self.text.len();
        match kind {
            "adrp" => self.emit_word(0x9000_0000 | rd as u32)?,
            "add_pageoff" => self.emit_word(0x9100_0000 | ((rd as u32) << 5) | rd as u32)?,
            _ => unreachable!(),
        }
        let relocation_kind = if kind == "adrp" {
            "page21"
        } else {
            "pageoff12"
        }
        .to_string();
        if let Some(library) = self.imports.get(&symbol) {
            self.relocations.push(EncodedRelocation {
                offset,
                target: symbol,
                kind: relocation_kind,
                binding: "external".to_string(),
                library: Some(library.clone()),
            });
        } else {
            self.relocations.push(EncodedRelocation {
                offset,
                target: symbol,
                kind: relocation_kind,
                binding: "data".to_string(),
                library: None,
            });
        }
        Ok(())
    }

    pub(super) fn patch_labels(&mut self) -> Result<(), String> {
        for patch in &self.patches {
            let Some(&target) = self.labels.get(&patch.target) else {
                return Err(format!(
                    "AArch64 branch target label '{}' does not resolve",
                    patch.target
                ));
            };
            let word = match patch.kind.as_str() {
                "b" => 0x1400_0000 | branch_imm26(patch.offset, target),
                "b.eq" => 0x5400_0000 | (branch_imm19(patch.offset, target) << 5),
                "b.ne" => 0x5400_0001 | (branch_imm19(patch.offset, target) << 5),
                "b.ge" => 0x5400_000a | (branch_imm19(patch.offset, target) << 5),
                "b.lt" => 0x5400_000b | (branch_imm19(patch.offset, target) << 5),
                "b.gt" => 0x5400_000c | (branch_imm19(patch.offset, target) << 5),
                "b.le" => 0x5400_000d | (branch_imm19(patch.offset, target) << 5),
                "b.vc" => 0x5400_0007 | (branch_imm19(patch.offset, target) << 5),
                "b.hi" => 0x5400_0008 | (branch_imm19(patch.offset, target) << 5),
                "b.lo" => 0x5400_0003 | (branch_imm19(patch.offset, target) << 5),
                other => return Err(format!("unknown AArch64 branch patch '{other}'")),
            };
            self.text[patch.offset..patch.offset + 4].copy_from_slice(&word.to_le_bytes());
        }
        Ok(())
    }
}
